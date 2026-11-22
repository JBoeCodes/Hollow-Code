//! Workflow runner + the run queue's workflow enqueuer.
//!
//! A workflow is an agent-driven automation stored in
//! `vault/workflows/{slug}.md` (CRUD lives in [`crate::vault_workflows`]).
//! This module spawns the selected agent for a workflow, streams events
//! ([`EVENT`]) to the frontend, persists run status to the workflow's
//! frontmatter, and appends to its `## Run log` section.
//!
//! **One run at a time, through one pump (Paperclip PR 7).** Every workflow
//! run holds the single full-vault lease (`automated_history`), shared with
//! chat, the heartbeat, ingest, Notion sync and history clears, so runs never
//! overlap. Every request to run a workflow is a row on the durable run queue
//! ([`crate::run_queue`]): a due cron occurrence, a trigger fire, Run now, and
//! later PR 9's retries. The single pump ([`crate::run_pump`]) claims the head
//! row, decides it (`run_pump::claim_verdict`: cheap facts, then [`precheck`],
//! PR 5's permission verify) and admits it through [`admit_runnable`], which
//! accepts only a [`Runnable`] that `precheck` built and takes the lease with
//! `automated_history::try_begin_owned`. The pump is the ONLY automation
//! caller of the lease; nothing checks the lease and then spawns. [`execute`]
//! then prepares the agent inside that lease and runs it, keeping the
//! [`RunLease`] until [`finalize`] releases it, so no fallible step can leak
//! it.
//!
//! **The scheduler only enqueues.** [`start_scheduler`] wakes every 60s and
//! calls [`enqueue_due`]: one cron row per due `next_run_at` (even past its
//! grace: the claim labels it), and one trigger row per fire, whose watermark
//! is committed once the row is enqueued (a re-fire while it is still queued
//! coalesces into it). Its only vault writes are the startup repairs and
//! `next_run_at` heals, under the short bookkeeping lease. A workflow with a
//! claimed or running row is not evaluated that tick.
//!
//! **Owner Mac (M4).** A pass sees only the automations whose `runs_on`
//! names this Mac, or that have no owner while this Mac holds the vault
//! alone ([`crate::runs_on`]), so a `workflows/` folder synced to two Macs
//! fires each automation on one of them and only that one writes its file.
//!
//! **Skips and misfires** are decided at claim, per source (see `run_pump`).
//! A cron occurrence due past its **misfire grace** window (default 120 min,
//! frontmatter `misfire_grace_min`) is skipped and rescheduled rather than
//! fired late, so schedules missed while Hollow was closed never fire in one
//! catch-up burst. Its run-log row says why: `not run: <reason>` when the
//! workflow cannot run as written (an empty prompt, or a permission refusal),
//! `missed while Hollow was closed` only for a genuine misfire, and a
//! contention note naming the holder when that occurrence waited on the lease
//! (its row's wait episode). Every such write happens only under the
//! scheduler's bookkeeping lease ([`skip_bookkeeping`]), so none is ever
//! attributed to another run's capture; while another writer holds the vault
//! the row simply stays queued. An unparseable schedule is never enqueued,
//! skipped or rewritten.
//!
//! **Run now** ([`run`]) checks the workflow up front ([`precheck`], so a
//! refusal returns at once), enqueues a manual row and waits for its outcome:
//! behind another writer it waits in the queue instead of refusing (a stale
//! or unknown lease still refuses at once, with the restart hint). A manual
//! run absorbs only origin-compatible queued occurrences. Proven requests do
//! not consume independent pending trigger fires or advance their schedules.
//! Legacy Unknown admissions retain pending-watermark behavior, but actual
//! queued runs advance only an owned or compatible absorbed Cron occurrence;
//! neither that compatibility behavior nor Manual priority proves direct origin.
//!
//! **Permission stamps (Paperclip PR 5).** [`precheck`] reads the workflow
//! through `workflow_permissions::verify`, which compares it with the
//! settings Hollow last approved and returns the exact parsed workflow the
//! run executes (there is no second read). A workflow changed outside
//! Hollow's automation editor is refused before the lease is ever tried: Run
//! now at once, and every queued row at claim (held within its window, then
//! skipped with its own note). `verify` releases its write guard before
//! returning, so the lease is never taken under it.

use crate::agent::{self, AgentEvent, AgentKind};
use crate::automated_history::{self, BackgroundWriteLease, BeginRunError, LeaseHolder, RunLease};
use crate::core_host::Core;
use crate::notifications;
use crate::prep_timing::{Mark, Step};
use crate::process_group::{
    self, Ended, GroupGuard, StderrPolicy, StopSignal, Supervision, Timings,
};
use crate::run_queue::{self, Enqueued, NewRow, RowOutcome, RowStatus, RunSource, WaitEpisode};
use crate::trigger_evaluator::TriggerWatermark;
use crate::types::{RunLogEntry, RunQueueItem, Workflow, WorkflowRunResult};
use crate::usage::LedgerOutcome;
use crate::usage_ledger::{self, DelegateParent, Meter, Surface, UsageRun};
use crate::vault_workflows;
use crate::workflow_hygiene::{self, NotificationPlanInput, NotifyPolicy, RunContext};
use crate::workflow_retry::{
    self, FailureClass, FailureFacts, FailureStage, RetryInput, RetryPlan, RetrySkipReason,
};
use chrono::{DateTime, Local, Utc};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;

pub const EVENT: &str = "workflow-event";

/// Wall-clock cap on a single workflow run. Long enough for a multi-step
/// agent task; short enough that a misbehaving model can't hold a slot
/// forever.
const RUN_TIMEOUT_SECS: u64 = 60 * 60;
/// [`RUN_TIMEOUT_SECS`] as a duration.
const RUN_TIMEOUT: Duration = Duration::from_secs(RUN_TIMEOUT_SECS);

/// Cap on the assistant-text payload we ship back to the frontend.
/// Truncation happens before the `done` event is emitted so a runaway
/// model can't blow out the IPC bus.
const OUTPUT_TRUNCATE_CHARS: usize = 8_000;

/// How many trailing stderr lines a failed run keeps in its run-log entry.
const STDERR_TAIL_LINES: usize = 30;

/// Char cap on the persisted stderr tail (the run log lives inside the
/// workflow markdown file — keep the hidden marker bounded).
const STDERR_TAIL_MAX_CHARS: usize = 4_000;

struct CancelHandle {
    stop: Arc<StopSignal>,
}

/// A workflow's entry in [`RUNNING`], registered by [`admit_runnable`] right after the
/// run takes the vault lease and owned by its [`AdmittedRun`], so a stop
/// pressed during preparation is honored before spawn. Dropping it removes
/// the entry, on every exit path (an admission dropped unexecuted included),
/// unless a cancel already took it or a newer run replaced it.
struct RunningSlot {
    slug: String,
    stop: Arc<StopSignal>,
}

impl RunningSlot {
    fn register(slug: &str) -> Option<Self> {
        let stop = Arc::new(StopSignal::new());
        let mut running = RUNNING.lock().ok()?;
        running.insert(
            slug.to_string(),
            CancelHandle {
                stop: Arc::clone(&stop),
            },
        );
        Some(Self {
            slug: slug.to_string(),
            stop,
        })
    }
}

impl Drop for RunningSlot {
    fn drop(&mut self) {
        if let Ok(mut running) = RUNNING.lock() {
            if running
                .get(&self.slug)
                .is_some_and(|handle| Arc::ptr_eq(&handle.stop, &self.stop))
            {
                running.remove(&self.slug);
            }
        }
    }
}

struct FinalizeOutcome {
    receipt: Option<crate::evidence::EvidenceReceipt>,
    evidence_error: Option<String>,
}

/// How one run ended, handed to [`finalize`]. A struct so later PRs (budget
/// exits, failure streaks) add fields instead of positional parameters.
struct RunFinish<'a> {
    /// The run-log status: `success` | `error` | `cancelled`.
    status: &'a str,
    error: Option<&'a str>,
    error_tail: Option<String>,
    duration_sec: u64,
    output: &'a str,
    /// The run's usage row, finished first thing in `finalize`.
    ledger: UsageRun,
    outcome: LedgerOutcome,
    /// `Some(EXIT_KIND_BUDGET)` when a usage budget ended the run (Paperclip
    /// PR 8b): persisted as the run-log row's hidden exit marker, kept out of
    /// the failure streak, and notified quietly (the budget pause is the
    /// actionable notice).
    exit_kind: Option<&'static str>,
    /// The claim's usage budget (Paperclip PR 8b), so `after_run` holds a run
    /// that changed `budgets.json` mid-run to the claim's limits. None when
    /// the run never reached its spawn plan.
    claim: Option<&'a crate::budgets::RunBudget>,
    /// Why the run is happening (Paperclip PR 9b: the retry plan reads its
    /// source, attempt, row and priority, and a retry's row note says so).
    ctx: &'a RunContext,
    /// Exactly the `at` passed to `vault_workflows::mark_started`; None when
    /// the run failed or stopped before it (the retry payload's
    /// `failedStartedAt`).
    started_at: Option<DateTime<Utc>>,
    /// How a failed run is classified (`workflow_retry::classify_failure`);
    /// None for a success or a cancel.
    failure: Option<FailureClass>,
}

static RUNNING: Lazy<Mutex<HashMap<String, CancelHandle>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static SCHEDULER_STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// True when a workflow with the given slug currently has a running
/// subprocess.
pub fn is_running(slug: &str) -> bool {
    RUNNING
        .lock()
        .map(|g| g.contains_key(slug))
        .unwrap_or(false)
}

/// Cancel a running workflow. No-op if the slug isn't running. The run ends
/// every process it started before it finalizes and releases the lease.
pub fn cancel(slug: &str) {
    let handle = RUNNING.lock().ok().and_then(|mut g| g.remove(slug));
    if let Some(h) = handle {
        h.stop.request();
    }
}

/// A workflow that passed [`precheck`]: the exact parsed file
/// `workflow_permissions::verify` returned and its non-empty prompt body. The
/// fields are private and only `precheck` builds one, so the compiler, not
/// just a source pin, enforces PR 5's "every run path reaches verify, with no
/// second read": [`admit_runnable`] accepts nothing else.
pub(crate) struct Runnable {
    workflow: Workflow,
    prompt: String,
}

impl Runnable {
    /// The verified workflow this run will execute.
    pub(crate) fn workflow(&self) -> &Workflow {
        &self.workflow
    }
}

/// Why a workflow cannot even try for the lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PrecheckFailure {
    /// The workflow may not run as written yet: its automation settings
    /// changed outside Hollow's editor and await (or were denied) approval,
    /// or the permission records cannot be read (Paperclip PR 5's
    /// `workflow_permissions::verify`, which releases its own guard before
    /// returning, so the lease is never taken under it). `reason` is the full
    /// refusal shown to whoever asked (Run now's error, starting with
    /// `workflow_permissions::REFUSAL_PREFIX` for a permission change);
    /// `note` is the short reason a past-grace run-log row records. The run
    /// queue (Paperclip PR 7) holds such a row rather than spending it: a
    /// cron or trigger row is deferred with `held_reason` = `note` (a hold,
    /// never a lease wait, so no wait episode), keeping the occurrence and
    /// the fire (a trigger's watermark was already committed at enqueue);
    /// past grace a cron row logs [`refused_skip_entry`] (outranking both
    /// contention and the misfire note) and is reported in neither skip
    /// notification. A queued Run now is skipped and its waiter gets
    /// `reason`.
    ///
    /// [`refused_skip_entry`]: vault_workflows::refused_skip_entry
    Refused { reason: String, note: String },
    /// Missing or unreadable file, no title, or an empty prompt: a
    /// configuration error. Run now shows it; at claim a trigger row is
    /// spent (so a broken workflow does not re-fire), a cron row is retried
    /// silently within grace and past grace skipped with
    /// [`invalid_skip_entry`] (never the misfire note, since Hollow was
    /// open), reported in neither notification.
    ///
    /// [`invalid_skip_entry`]: vault_workflows::invalid_skip_entry
    Invalid(String),
}

impl PrecheckFailure {
    /// Map a permission verification failure. A refusal, or unreadable
    /// permission records, keeps the fire and the occurrence
    /// ([`VerifyError::holds_fire`]); a missing or unparseable file is a
    /// configuration error, exactly as the plain read reported it before.
    ///
    /// [`VerifyError::holds_fire`]: crate::workflow_permissions::VerifyError::holds_fire
    fn from_permissions(error: crate::workflow_permissions::VerifyError) -> Self {
        if !error.holds_fire() {
            return PrecheckFailure::Invalid(error.into());
        }
        let note = if error.is_refusal() {
            vault_workflows::PERMISSION_REFUSED_REASON
        } else {
            vault_workflows::PERMISSION_RECORDS_UNAVAILABLE_REASON
        };
        PrecheckFailure::Refused {
            reason: error.into(),
            note: note.to_string(),
        }
    }
}

/// THE validity hook every workflow admission passes: its permission stamp
/// (`workflow_permissions::verify`, which returns the exact parsed workflow
/// the run then executes), then a non-empty prompt body. The run queue's
/// claim (`run_pump::claim_verdict`, phase 2) calls it for every cron,
/// trigger, manual and retry row, and Run now calls it up front before
/// enqueueing. Lock-free with respect to history and the run queue. Its only
/// writes are hidden review-queue state (the one live permission review) and,
/// when a review opens, a rate-limited notification: never `workflows/*.md`.
/// Never call it for a workflow already known to be missing: `verify` on a
/// missing file tombstones its stamp and retires its reviews.
pub(crate) fn precheck(vault: &Path, slug: &str) -> Result<Runnable, PrecheckFailure> {
    let crate::workflow_permissions::VerifiedWorkflow { workflow, prompt } =
        crate::workflow_permissions::verify(vault, slug)
            .map_err(PrecheckFailure::from_permissions)?;
    if prompt.trim().is_empty() {
        return Err(PrecheckFailure::Invalid(
            "prompt is empty; add a prompt body before running".into(),
        ));
    }
    // Paperclip PR 19: a configuration error like the empty prompt, so Run
    // now shows it at once, a cron row is retried within grace and then
    // skipped with `not run: working folder …`, a trigger fire is spent, and
    // a retry row (9b) is skipped as invalid. Never retried as transient.
    if let Some(problem) = workflow_cwd_problem(vault, workflow.cwd.as_deref()) {
        return Err(PrecheckFailure::Invalid(problem));
    }
    Ok(Runnable { workflow, prompt })
}

/// What is wrong with a workflow's `cwd`, or `None` when the run may use it
/// (Paperclip PR 19). Empty means the vault. Otherwise it must be an
/// absolute path to an existing folder that is not a system or shared root
/// ([`agent::unsafe_working_dir`]; the vault itself always passes). One line,
/// no em dash, no angle brackets: it becomes the `not run: …` run-log note.
pub(crate) fn workflow_cwd_problem(vault: &Path, cwd: Option<&str>) -> Option<String> {
    let cwd = cwd.map(str::trim).filter(|cwd| !cwd.is_empty())?;
    let path = Path::new(cwd);
    if !path.is_absolute() {
        return Some(format!("working folder {cwd} must be an absolute path"));
    }
    if !path.is_dir() {
        return Some(format!(
            "working folder {cwd} does not exist or is not a folder"
        ));
    }
    agent::unsafe_working_dir(path, Some(vault)).map(|reason| {
        format!(
            "working folder {reason}; choose a project folder, or leave it empty to use the vault"
        )
    })
}

/// A workflow run that holds the vault lease (a [`RunLease`], finalized as
/// cancelled if dropped) and its stop slot. Pass it to [`execute`].
pub(crate) struct AdmittedRun {
    slug: String,
    workflow: Workflow,
    prompt: String,
    /// Why this run is happening (Paperclip PR 9, 9a-2): the claimed row's
    /// source, reason, attempt and id. Feeds `{{trigger_reason}}` and the
    /// `started` event.
    ctx: RunContext,
    /// The claim-time usage budget (Paperclip PR 8a's preflight, carried by
    /// PR 8b): the run's budget guard, partial-message streaming and
    /// spending backstop come from it. None when admitted without a claim
    /// (tests); `execute` then runs with no budget.
    budget: Option<crate::budgets::RunBudget>,
    lease: RunLease,
    slot: RunningSlot,
}

impl AdmittedRun {
    #[cfg(test)]
    pub(crate) fn slug(&self) -> &str {
        &self.slug
    }

    /// The history run this admission started.
    pub(crate) fn run_id(&self) -> &str {
        self.lease.id()
    }

    /// Why this run is happening (tests: the pump builds it from the row).
    #[cfg(test)]
    pub(crate) fn context(&self) -> &RunContext {
        &self.ctx
    }

    /// The claim's usage budget this run carries (tests: Paperclip PR 8b).
    #[cfg(test)]
    pub(crate) fn budget(&self) -> Option<&crate::budgets::RunBudget> {
        self.budget.as_ref()
    }

    /// Note the queued rows admission absorbed into this run (their trigger
    /// reasons join `{{trigger_reason}}`).
    pub(crate) fn absorb_context(&mut self, rows: &[crate::run_queue::Absorbed]) {
        self.ctx.absorb(rows);
    }

    /// Ask the run to stop (a cancel that landed while its queue row was
    /// claimed). `execute` checks the signal before it spawns and finalizes a
    /// stopped run as cancelled.
    pub(crate) fn request_stop(&self) {
        self.slot.stop.request();
    }

    #[cfg(test)]
    pub(crate) fn record_completion_for_test(
        &self,
        vault: &Path,
        status: &str,
    ) -> Result<String, String> {
        finalize_record(
            vault,
            &self.workflow,
            &self.ctx,
            self.run_id(),
            None,
            RunLogEntry {
                at: Utc::now().to_rfc3339(),
                status: status.into(),
                duration_sec: 1,
                note: "synthetic completion".into(),
                run_id: Some(self.run_id().into()),
                error_tail: None,
                exit_kind: None,
            },
        )
    }

    /// Finalize without executing, the way a run that wrote its own changes
    /// would be finalized.
    #[cfg(test)]
    pub(crate) fn finish_for_test(self, status: &str) -> automated_history::HistoryRun {
        let AdmittedRun { lease, slot, .. } = self;
        drop(slot);
        lease.finish(status).expect("finish the admitted run")
    }
}

/// How the pump is admitting a row.
pub(crate) struct AdmitContext<'a> {
    /// The row's wait episode: a run that waited on another writer's lease
    /// writes its `deferred` row inside its own lease, unless the workflow
    /// ran since the episode began (then the wait was not this run's).
    pub(crate) waited: Option<&'a WaitEpisode>,
    /// Run now. Only a legacy Unknown request may consume an unevaluated
    /// trigger fire; a proven root leaves independent scheduler events intact.
    pub(crate) manual: bool,
    /// What the run itself knows about its origin (Paperclip PR 9's
    /// [`RunContext`], built from the claimed row). Its queue identity resolves
    /// durable provenance for admission and schedule-consumption checks.
    pub(crate) run: RunContext,
    /// The claim's usage budget (`budgets::preflight`, Paperclip PR 8a),
    /// carried to [`execute`] for PR 8b's mid-run enforcement.
    pub(crate) budget: Option<crate::budgets::RunBudget>,
}

/// The outcome of trying to start a workflow.
pub(crate) enum Admission {
    /// It holds the lease; [`execute`] it.
    Admitted(AdmittedRun),
    /// Another writer holds the lease. Nothing was written.
    Deferred { holder: LeaseHolder },
}

/// Take the full-vault lease for a checked workflow, atomically. Called only
/// by the run-queue pump (`run_pump::step`) and by tests; `runnable` can only
/// come from [`precheck`]. Sync; it snapshots the vault baseline on success,
/// so async callers run it on a blocking worker.
///
/// On success the run's stop slot is registered at once (a stop pressed
/// during preparation is honored), a waited row writes its `deferred` row
/// inside the new lease. Only legacy Unknown manual rows consume pending
/// trigger fires (see [`AdmitContext`]).
pub(crate) fn admit_runnable(
    vault: &Path,
    slug: &str,
    runnable: Runnable,
    ctx: AdmitContext<'_>,
) -> Result<Admission, String> {
    if is_running(slug) {
        return Err(format!("workflow {slug} is already running"));
    }
    let Runnable { workflow, prompt } = runnable;
    let AdmitContext {
        waited,
        manual,
        run: run_ctx,
        budget,
    } = ctx;
    let consume_pending = if manual && !workflow.triggers.is_empty() {
        match run_ctx.queue_id {
            Some(queue_id) => crate::session_store::with_read(vault, |conn| {
                crate::work_origin::consumes_pending_trigger_in(conn, queue_id)
            })?,
            // The queue-free admission seam is tests only; production pump
            // contexts always name their exact claimed row.
            None => true,
        }
    } else {
        false
    };
    let lease = match automated_history::try_begin_owned(vault, "workflow", Some(slug)) {
        Ok(lease) => lease,
        Err(BeginRunError::Busy(holder)) => return Ok(Admission::Deferred { holder }),
        Err(BeginRunError::Failed(error)) => return Err(error),
    };
    // Register the stop slot and confirm the file still exists in ONE step
    // under the permission write guard, which the editor's save holds for its
    // whole rename (Paperclip PR 9, 9a-1). A rename that finished before this
    // point fails the check; one that starts later sees the slot (or this
    // run's lease) and is refused. The guard is a tight inner scope: the
    // lease must never be finished or dropped under it, because
    // `finish_run` takes HISTORY_LOCK and then this same (non-reentrant)
    // guard for its workflow flag check.
    let (slot, present) = {
        let _guard = crate::workflow_permissions::write_guard();
        let slot = RunningSlot::register(slug);
        let present = crate::vault::safe_join(
            vault,
            &format!("{}/{slug}.md", vault_workflows::WORKFLOWS_DIR),
        )
        .is_ok_and(|path| path.is_file());
        (slot, present)
    };
    let Some(slot) = slot else {
        let _ = lease.finish("error");
        return Err("workflow registry poisoned".into());
    };
    if !present {
        drop(slot);
        let _ = lease.finish("cancelled");
        return Err(format!("workflow {slug} was renamed or deleted"));
    }
    let now = Utc::now();
    // The episode ends with its `deferred` row inside this run's own lease,
    // so the row is attributed to the run it describes. One older than the
    // workflow's last start is stale, not this run's wait, and ends silently.
    if let Some(episode) = waited {
        if !episode.ran_since(&workflow) {
            let _ = vault_workflows::append_run_log(vault, slug, episode.started_entry(now));
        }
    }
    if consume_pending {
        consume_pending_trigger_fire(vault, &workflow, now);
    }
    Ok(Admission::Admitted(AdmittedRun {
        slug: slug.to_string(),
        workflow,
        prompt,
        ctx: run_ctx,
        budget,
        lease,
        slot,
    }))
}

/// Tests only: [`precheck`] then [`admit_runnable`] as a manual run, the way
/// PR 2 and PR 5's admission tests drive a single workflow without the queue.
#[cfg(test)]
pub(crate) enum TestAdmission {
    Admitted(AdmittedRun),
    Deferred(LeaseHolder),
    Refused(String),
}

#[cfg(test)]
pub(crate) fn admit(vault: &Path, slug: &str) -> Result<TestAdmission, String> {
    admit_with(vault, slug, precheck)
}

#[cfg(test)]
fn admit_with(
    vault: &Path,
    slug: &str,
    check: impl FnOnce(&Path, &str) -> Result<Runnable, PrecheckFailure>,
) -> Result<TestAdmission, String> {
    if is_running(slug) {
        return Err(format!("workflow {slug} is already running"));
    }
    let runnable = match check(vault, slug) {
        Ok(runnable) => runnable,
        Err(PrecheckFailure::Refused { reason, .. }) => return Ok(TestAdmission::Refused(reason)),
        Err(PrecheckFailure::Invalid(error)) => return Err(error),
    };
    let ctx = AdmitContext {
        waited: None,
        manual: true,
        run: RunContext::manual_for_test(),
        budget: None,
    };
    Ok(match admit_runnable(vault, slug, runnable, ctx)? {
        Admission::Admitted(run) => TestAdmission::Admitted(run),
        Admission::Deferred { holder } => TestAdmission::Deferred(holder),
    })
}

/// Serializes every read-merge-write of `.workflow-triggers.json` in this
/// process: the enqueuer's per-tick commit and legacy manual consumption.
static TRIGGER_STATE_LOCK: Mutex<()> = Mutex::new(());

/// Merge per-workflow watermarks into `.workflow-triggers.json`, replacing
/// only the given workflows' entries, so a concurrent writer's entry for
/// another workflow survives. Writes only when something changed, through
/// `trigger_scanner::save_state` (an attributed host write, so a run live
/// meanwhile is never flagged for it). Never call it while holding the
/// session store's `WRITE_LOCK`, and never enqueue from inside it.
fn commit_trigger_updates(vault: &Path, updates: WatermarkState) {
    if updates.is_empty() {
        return;
    }
    let _guard = TRIGGER_STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut state = crate::trigger_scanner::load_state(vault);
    let before = state.clone();
    state.extend(updates);
    if state != before {
        crate::trigger_scanner::save_state(vault, &state);
    }
}

/// Legacy Unknown manual runs retain their historical trigger consumption.
/// Proven roots never use this path: unevaluated events have no durable origin
/// with which to establish compatibility. Called inside the legacy run's lease.
fn consume_pending_trigger_fire(vault: &Path, workflow: &Workflow, now: DateTime<Utc>) {
    use crate::trigger_evaluator::{TriggerEvaluator, TriggerSpec};
    let specs = TriggerSpec::parse_polled(&workflow.triggers);
    if specs.is_empty() {
        return;
    }
    let task_ids = crate::trigger_scanner::task_trigger_ids(vault);
    let facts = crate::trigger_scanner::live_facts(vault, now, task_ids);
    let prior = crate::trigger_scanner::load_state(vault)
        .remove(&workflow.slug)
        .unwrap_or_default();
    let (_, _, next) = TriggerEvaluator::evaluate_all(&specs, &facts, &prior);
    commit_trigger_updates(vault, HashMap::from([(workflow.slug.clone(), next)]));
}

/// Run one workflow now (the Run button), through the run queue: it checks
/// the workflow up front (a refusal or configuration error returns at once,
/// even while the vault is busy), enqueues a manual row (priority 1, exempt
/// from the automation gap), and waits for that row's outcome, returning the
/// executed run's result with its receipt. Behind another writer it waits in
/// the queue instead of refusing; a stale or unknown lease still refuses at
/// once with the restart hint.
pub(crate) async fn run(
    app: AppHandle,
    vault: PathBuf,
    slug: String,
    retry_of: Option<String>,
    origin: crate::work_origin::Request,
) -> Result<WorkflowRunResult, String> {
    let (receiver, _) = start_run_now(Some(&app), vault, slug.clone(), retry_of, origin).await?;
    manual_outcome(&slug, receiver.await)
}

/// Run now's enqueue, without waiting for the run: [`run`] awaits the
/// receiver; Hollow app tools' `run_automation` (PR A3) returns at once,
/// because the new run waits for the lease the calling run holds. `app` is
/// only for the queue event (`None` in tests).
pub(crate) async fn start_run_now(
    app: Option<&AppHandle>,
    vault: PathBuf,
    slug: String,
    retry_of: Option<String>,
    origin: crate::work_origin::Request,
) -> Result<
    (
        tokio::sync::oneshot::Receiver<RowOutcome>,
        Option<RunQueueItem>,
    ),
    String,
> {
    if is_running(&slug) {
        return Err(format!("workflow {slug} is already running"));
    }
    crate::run_pump::accepting()?;
    // The permission check, the enqueue and the row read touch files and
    // SQLite, so keep them off the async worker.
    let enqueue_vault = vault.clone();
    let enqueue_slug = slug.clone();
    let (enqueued, receiver, item) = tauri::async_runtime::spawn_blocking(move || {
        enqueue_run_now(
            &enqueue_vault,
            &enqueue_slug,
            Utc::now(),
            precheck,
            retry_of.as_deref(),
            origin,
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    if let (Some(app), Some(kind), Some(item)) = (app, enqueued.event_kind(), item.clone()) {
        crate::run_pump::emit(app, kind, item);
    }
    run_queue::wake();
    Ok((receiver, item))
}

/// Run now's synchronous half, AppHandle-free so tests drive it: refuse
/// behind a stale or unknown lease, check the workflow (`check` is
/// [`precheck`] in production), then enqueue the manual row and register the
/// waiter in one step. Nothing is enqueued when it refuses.
pub(crate) fn enqueue_run_now(
    vault: &Path,
    slug: &str,
    now: DateTime<Utc>,
    check: impl FnOnce(&Path, &str) -> Result<Runnable, PrecheckFailure>,
    retry_of: Option<&str>,
    origin: crate::work_origin::Request,
) -> Result<
    (
        Enqueued,
        tokio::sync::oneshot::Receiver<RowOutcome>,
        Option<RunQueueItem>,
    ),
    String,
> {
    if let Some(message) =
        crate::run_pump::refuse_interactive(automated_history::lease_holder(vault).as_ref())
    {
        return Err(message);
    }
    let runnable = match check(vault, slug) {
        Ok(runnable) => runnable,
        Err(PrecheckFailure::Refused { reason, .. }) => return Err(reason),
        Err(PrecheckFailure::Invalid(error)) => return Err(error),
    };
    // A usage budget that holds this automation refuses Run now at once
    // (Paperclip PR 8a). The read-only check runs first so a clear budget
    // has no side effects; when it blocks, the full preflight (safe here,
    // after the permission check) runs too, so a limit crossing no claim has
    // seen yet (usage from chat, ingest or one-shots) opens its pause and its
    // Automations card now, and the refusal points at a card that exists.
    let intent = crate::budgets::RunIntent::workflow(
        slug,
        crate::budgets::intent_agent(runnable.workflow.agent.as_deref()),
    );
    // Merged plan decision 6: not when another agent has room to take it
    // over (the claim moves it there).
    let store = crate::budgets::Store::for_app(vault);
    if crate::budgets::is_blocked(&store, vault, &intent, now).is_some() {
        let fallbacks = || crate::run_pump::fallback_agents(&runnable.workflow);
        if let crate::budgets::Preflight::Blocked(notice) =
            crate::budgets::preflight_with_fallback(&store, vault, &intent, &fallbacks, now)
        {
            return Err(notice.message);
        }
    }
    let title = runnable.workflow.title;
    let row = match retry_of.map(str::trim).filter(|id| !id.is_empty()) {
        // Plan completion L2: "Run again without repeating" carries the
        // failed run on its row; the claim reads it into
        // `RunContext.guarded_retry_of`. Checked here so a bad id refuses
        // before anything is queued.
        Some(failed) => {
            guarded_retry_effects(vault, slug, failed)?;
            // A retry that joined a queued or running row of this automation
            // would run unguarded (the queue coalesces manual rows).
            let live = run_queue::live_rows(vault)?.into_iter().any(|row| {
                row.source_id.as_deref() == Some(slug)
                    && matches!(row.source, RunSource::Manual | RunSource::Retry)
            });
            if live {
                return Err("A run of this automation is already queued or running, and a retry that joined it wouldn't be guarded. Wait for it to finish, or stop it, then retry.".into());
            }
            NewRow::new(
                RunSource::Manual,
                Some(slug),
                &title,
                "run again without repeating",
            )
            .with_payload(json!({ "retryOf": failed }))
        }
        None => NewRow::new(RunSource::Manual, Some(slug), &title, "run now"),
    };
    let (enqueued, receiver) = run_queue::enqueue_and_wait(vault, row.with_origin(origin), now)?;
    let item = run_queue::get(vault, enqueued.id())
        .ok()
        .flatten()
        .map(|row| run_queue::to_item(&row, None));
    Ok((enqueued, receiver, item))
}

/// What Run now returns for its row's outcome.
pub(crate) fn manual_outcome(
    slug: &str,
    outcome: Result<RowOutcome, tokio::sync::oneshot::error::RecvError>,
) -> Result<WorkflowRunResult, String> {
    match outcome {
        Ok(RowOutcome::Workflow(result)) => result,
        Ok(RowOutcome::Finished {
            status: RowStatus::Cancelled,
            error,
        }) => Ok(WorkflowRunResult {
            slug: slug.to_string(),
            status: "cancelled".to_string(),
            duration_sec: 0,
            output: String::new(),
            error: Some(error.unwrap_or_else(|| run_queue::CANCELLED_BEFORE_START.to_string())),
            receipt: None,
            evidence_error: None,
        }),
        Ok(RowOutcome::Finished { status, error }) => {
            Err(error
                .unwrap_or_else(|| format!("workflow {slug} did not run ({})", status.as_str())))
        }
        // The pump dropped the waiter without an outcome: Hollow is stopping.
        Err(_) => Err(run_queue::INTERRUPTED_RUNNING.to_string()),
    }
}

/// Stop `slug`'s queued work from the Stop button: its queued Run now and
/// retry rows end `cancelled` (their waiters resolve as cancelled), and a
/// claimed row of any source is flagged so the pump stops it before or right
/// after admission. Queued cron and trigger rows stay: Stop is about this
/// run, not the schedule. Returns the rows it acted on.
pub(crate) fn cancel_queued(vault: &Path, slug: &str) -> Result<Vec<RunQueueItem>, String> {
    let now = Utc::now();
    let mut acted = Vec::new();
    for row in run_queue::live_rows(vault)? {
        if !row.source.is_workflow() || row.source_id.as_deref() != Some(slug) {
            continue;
        }
        let wanted = match row.status {
            RowStatus::Claimed => true,
            RowStatus::Queued => matches!(
                row.source,
                RunSource::Manual | RunSource::Retry | RunSource::RestrictedResume
            ),
            _ => false,
        };
        if !wanted {
            continue;
        }
        let outcome = run_queue::cancel(vault, row.id, now)?;
        // Paperclip PR 9b: a cancelled retry ends the failure it deferred.
        if outcome == run_queue::CancelOutcome::Cancelled && row.source == RunSource::Retry {
            workflow_retry::resolve_skipped_retry(
                vault,
                slug,
                &row.payload,
                RetrySkipReason::UserCancel,
            );
        }
        match outcome {
            run_queue::CancelOutcome::Cancelled | run_queue::CancelOutcome::StopRequested => {
                if let Some(row) = run_queue::get(vault, row.id)? {
                    acted.push(run_queue::to_item(&row, None));
                }
            }
            _ => {}
        }
    }
    Ok(acted)
}

/// Drive an admitted run to completion. Streams events over [`EVENT`] keyed
/// by slug; persists status + run-log on every exit path.
///
/// Preparation (agent resolution, retrieval, context, the command) happens
/// here, inside the run's lease, so a configuration failure becomes a visible
/// failed run (run-log row, `last_status`, advanced `next_run_at`,
/// notification) instead of a silent pre-lease retry.
pub(crate) async fn execute(
    app: Core,
    vault: PathBuf,
    admitted: AdmittedRun,
) -> Result<WorkflowRunResult, String> {
    let AdmittedRun {
        slug,
        workflow,
        prompt,
        ctx,
        budget,
        lease,
        slot,
    } = admitted;
    // The claim's usage budget (Paperclip PR 8a/8b). Admitted without a
    // claim (tests) the run has none: no guard, no backstop.
    let budget = budget.unwrap_or_else(|| {
        crate::budgets::RunBudget::none(crate::budgets::RunIntent::workflow(&slug, None))
    });
    // The lease stays an owned `RunLease` until `finalize` releases it
    // (Paperclip PR 7): if this future is dropped midway, the lease still
    // finalizes its run as cancelled instead of holding the vault until
    // restart.
    let run_id = lease.id().to_string();
    // The run's usage row (Paperclip PR 6), begun only once the run is
    // admitted (a refused or deferred workflow never gets one) and finished
    // first in `finalize` on every exit. `begin` and everything else on the
    // ledger is infallible: nothing between the admission and `finalize` may
    // return early, or PR 5's flags would only come from the drop path.
    let mut ledger = UsageRun::begin(
        Meter::for_vault(
            &vault,
            Surface::Workflow,
            Some(slug.clone()),
            Some(run_id.clone()),
        ),
        None,
        None,
    );
    let mut collector = crate::evidence::EvidenceCollector::new("workflow", Some(&slug), &run_id);
    // Phase timings (Paperclip PR 15c): admission's baseline now, the
    // agent's phases once `drive` returns, the diff in `finalize`.
    collector.timings_mut().baseline_snapshot_ms = lease.baseline_ms();
    // The preparation timeline's clock (agent effectiveness F8).
    collector.timings_mut().set_admitted(lease.admitted_at());
    // Merged plan decision 6: a claim that moved this run off its agent's
    // budget ceiling puts the switch on the receipt (`prepare_spawn` runs
    // the agent it names).
    if let Some(fallback) = budget.fallback.as_ref() {
        collector.record_switch(fallback.receipt_switch());
    }
    // Plan completion L2: a guarded retry's receipt carries what the run it
    // retries already did, so a retry of this one still guards it.
    if let Some(failed) = ctx.guarded_retry_of.as_deref() {
        if let Ok(effects) = guarded_retry_effects(&vault, &slug, failed) {
            collector.carry_effects(crate::run_effects::carried(failed, &effects));
        }
    }
    let evidence = Arc::new(Mutex::new(collector));
    let stop = Arc::clone(&slot.stop);
    // Run inside this chat (overhaul F1): a bound automation queues its
    // turn in that conversation and ends here; the chat's turn does the
    // work. A busy chat or a session store it can't read now parks the
    // message (a success row noting "Waiting:"; the minute pass sends it). A
    // chat that is gone, archived or in a build workspace fails the run with
    // the reason (`automation_in_chat`).
    if workflow.in_chat.is_some() {
        drop(slot);
        let delivered = {
            let (v, w, r, c) = (vault.clone(), workflow.clone(), run_id.clone(), ctx.clone());
            let p = prompt.clone();
            tauri::async_runtime::spawn_blocking(move || {
                // Prompt variables expand as in a normal run (`prepare_spawn`).
                let p = crate::automation_in_chat::expand_prompt(&v, &w, &p, &c, Local::now());
                crate::automation_in_chat::deliver(&v, &w, &p, &r, Utc::now())
            })
            .await
            .unwrap_or_else(|e| Err(format!("queue the chat turn: {e}")))
        };
        let note = delivered.as_ref().map(|d| d.note()).unwrap_or_default();
        let error = delivered.as_ref().err().cloned();
        let finalization = finalize(
            &app,
            &vault,
            &workflow,
            &slug,
            &run_id,
            lease,
            evidence,
            RunFinish {
                status: if error.is_some() { "error" } else { "success" },
                error: error.as_deref(),
                error_tail: None,
                duration_sec: 0,
                output: &note,
                ledger,
                outcome: if error.is_some() {
                    LedgerOutcome::SpawnFailed
                } else {
                    LedgerOutcome::Success
                },
                exit_kind: None,
                claim: Some(&budget),
                ctx: &ctx,
                started_at: None,
                failure: error.as_ref().map(|_| {
                    workflow_retry::classify_failure(&FailureFacts::at(FailureStage::Prepare))
                }),
            },
        )
        .await;
        if let Some(error) = error {
            return Err(error);
        }
        return Ok(WorkflowRunResult {
            slug,
            status: "success".to_string(),
            duration_sec: 0,
            output: note,
            error: None,
            receipt: finalization.receipt,
            evidence_error: finalization.evidence_error,
        });
    }
    // The run's scratch folder (Paperclip PR 15a), made now that the lease is
    // held and keyed by the run id. Bound to a named local so it lives until
    // this function returns (after `finalize`, with the processes gone);
    // its drop removes it through a tombstone rename.
    let (_scratch_dir, scratch_spec) =
        crate::scratch::create_for_turn(Some(vault.as_path()), &run_id, &run_id);

    let plan = SpawnPlan {
        workflow: &workflow,
        prompt,
        slug: &slug,
        run_id: &run_id,
        ctx: &ctx,
        budget: &budget,
        seams: SpawnSeams::installed(),
        scratch: Some(scratch_spec),
    };
    let PreparedSpawn {
        agent,
        model,
        cmd,
        recall_items,
        manifest,
        parse,
        cwd: spawn_cwd,
        spec,
    } = match prepare_spawn(&vault, plan).await {
        Ok(prepared) => prepared,
        Err(err) => {
            drop(slot);
            // A stop pressed during preparation is the user's doing, not an error.
            if stop.requested() {
                return Ok(cancel_before_spawn(
                    &app, &vault, &workflow, &slug, &run_id, lease, evidence, ledger, &ctx,
                )
                .await);
            }
            finalize(
                &app,
                &vault,
                &workflow,
                &slug,
                &run_id,
                lease,
                evidence,
                RunFinish {
                    status: "error",
                    error: Some(&err),
                    error_tail: None,
                    duration_sec: 0,
                    output: "",
                    ledger,
                    outcome: LedgerOutcome::SpawnFailed,
                    exit_kind: None,
                    claim: Some(&budget),
                    ctx: &ctx,
                    started_at: None,
                    failure: Some(workflow_retry::classify_failure(&FailureFacts::at(
                        FailureStage::Prepare,
                    ))),
                },
            )
            .await;
            return Err(err);
        }
    };
    // Login override: an automation a claude.ai connector keeps on Claude
    // Code's login says so on its receipt, and, when the login is low while
    // an added account has room, warns in the bell once per window. Read
    // before the receipt's lock is taken.
    let login_names = agent::login_connectors(agent, &spec);
    let on_login = !login_names.is_empty()
        && crate::claude_accounts::account_of(&cmd).as_deref()
            == Some(crate::claude_accounts::LOGIN_ID)
        && crate::claude_accounts::has_added();
    if on_login {
        if let Some(low) = crate::claude_accounts::login_low_last_seen()
            .filter(crate::claude_accounts::first_login_warning)
        {
            crate::claude_accounts::notify_login_low(&vault, &low);
        }
    }
    // The run's own context manifest (agent effectiveness F11).
    if let Ok(mut collector) = evidence.lock() {
        // Plan completion L2: Antigravity's stream carries no tool calls.
        if matches!(agent, AgentKind::Antigravity) {
            collector.mark_tool_calls_unreported();
        }
        adopt_manifest(&mut collector, &run_id, manifest);
        if on_login {
            let remedy = agent::login_remedy(agent, &spec);
            collector.record_switch(crate::evidence::login_override(
                &login_names,
                remedy.as_deref(),
            ));
        }
    }
    ledger.set_agent(agent, model.clone());
    ledger.set_billing(crate::usage::classify(agent, &cmd, true));
    // Paperclip PR 16: snapshot the run's target git repositories inside the
    // lease, before the start is stamped and the agent spawns, never under
    // HISTORY_LOCK (empty, with no git run, for a vault-root cwd). A stop
    // pressed meanwhile ends the git reads and is honored just below. Armed
    // only once the agent spawned.
    let snapshot_started = Instant::now();
    let repo_baseline = crate::repo_diff::begin(&vault, &spawn_cwd, Some(&*stop)).await;
    if let Ok(mut collector) = evidence.lock() {
        collector.set_repo_baseline(repo_baseline);
        collector
            .context_mut()
            .timer
            .0
            .record(Step::RepoSnapshot, Mark::since(snapshot_started));
    }

    let started = Instant::now();
    if stop.requested() {
        drop(slot);
        return Ok(cancel_before_spawn(
            &app, &vault, &workflow, &slug, &run_id, lease, evidence, ledger, &ctx,
        )
        .await);
    }
    // Stamp the start right before the spawn (Paperclip PR 9, 9a-2): the
    // agent sees the vault from here on, so this is the baseline a later
    // quiet check compares against, and a run that failed in preparation or
    // was stopped never advances it. `running` also shows while the process
    // is alive. Unlike the old ignored status write, a failure here (the file
    // renamed or deleted past admission) finalizes the run as an error.
    let started_at = Utc::now();
    if let Err(error) = vault_workflows::mark_started(&vault, &slug, started_at) {
        drop(slot);
        let err = format!("could not record the run start: {error}");
        finalize(
            &app,
            &vault,
            &workflow,
            &slug,
            &run_id,
            lease,
            evidence,
            RunFinish {
                status: "error",
                error: Some(&err),
                error_tail: None,
                duration_sec: 0,
                output: "",
                ledger,
                outcome: LedgerOutcome::SpawnFailed,
                exit_kind: None,
                claim: Some(&budget),
                ctx: &ctx,
                started_at: None,
                failure: Some(workflow_retry::classify_failure(&FailureFacts::at(
                    FailureStage::Prepare,
                ))),
            },
        )
        .await;
        return Err(err);
    }
    emit(
        &app,
        json!({
            "type": "started",
            "slug": slug,
            "at": started_at.to_rfc3339(),
            "runId": &run_id,
            "recall": recall_items,
            "source": ctx.source.as_str(),
            "reason": &ctx.reason,
            "attempt": ctx.attempt,
            "queueId": ctx.queue_id,
            // Merged plan decision 6: a run moved off its agent's budget
            // ceiling says so in the live run log, not only on the receipt
            // at the end (`null` when it never moved).
            "switched": budget.fallback.as_ref().map(|fallback| fallback.receipt_switch()),
        }),
    );
    if let Ok(mut collector) = evidence.lock() {
        collector.context_mut().timer.0.spawn_called(Instant::now());
    }
    let limits = crate::agent_settings::run_limits(agent);
    let stall_after = limits.stall_after;
    let deadline = run_deadline(limits.deadline);
    let mut cmd = cmd;
    let mut first_parse = Some(parse);
    let mut attempt_spec = spec.clone();
    let mut switches: u32 = 0;
    // OB22: context compactions over every attempt (an account restart
    // starts a fresh session, so each attempt counts its own).
    let mut compactions: u32 = 0;
    let (driven, budget_stop, result_subtype, result_error, result_record, text) = loop {
        if let Some(reason) = crate::claude_accounts::pick_receipt(&cmd) {
            if let Ok(mut collector) = evidence.lock() {
                collector.record_account_choice(reason);
            }
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err = format!("spawn {}: {e}", agent.as_str());
                finalize(
                    &app,
                    &vault,
                    &workflow,
                    &slug,
                    &run_id,
                    lease,
                    evidence,
                    RunFinish {
                        status: "error",
                        error: Some(&err),
                        error_tail: None,
                        duration_sec: 0,
                        output: "",
                        ledger,
                        outcome: LedgerOutcome::SpawnFailed,
                        exit_kind: None,
                        claim: Some(&budget),
                        ctx: &ctx,
                        started_at: Some(started_at),
                        failure: Some(workflow_retry::classify_failure(&FailureFacts::at(
                            FailureStage::Spawn,
                        ))),
                    },
                )
                .await;
                return Err(err);
            }
        };
        ledger.launched(&cmd);
        if switches == 0 {
            if let Ok(mut collector) = evidence.lock() {
                collector.arm_repo_baseline();
            }
        }

        let guard = GroupGuard::adopt_run(&cmd, &child, Some(&run_id));

        // drive() is the only waiter: it streams events, enforces the deadline,
        // ends every process the run started on cancel, timeout, a budget stop
        // or normal exit, and returns only after that, before finalize releases
        // the lease.
        // The silent-run indicator and the no-output watchdog (Paperclip PR
        // 15b): one activity clock for the run, stamped by `drive` on every
        // stdout and stderr line, reported every 15 s as a `workflow-event` of
        // type `activity` (keyed by slug and run id) by a ticker dropped as soon
        // as the agent has finished. The watchdog and the optional deadline are
        // the agent's settings, the same path for every agent (merged plan
        // slice 3, A·R2); the deadline never extends the run's fixed hour.
        let activity = Arc::new(crate::run_activity::RunActivity::new());
        let ticker = {
            let app = app.clone();
            let slug = slug.clone();
            let run_id = run_id.clone();
            let queue_id = ctx.queue_id;
            crate::run_activity::spawn_ticker(
                Arc::clone(&activity),
                crate::run_activity::TICK,
                move |snapshot| {
                    emit(
                        &app,
                        workflow_activity_event(&slug, &run_id, queue_id, &snapshot),
                    );
                },
            )
        };
        // The prepared parse mode, then the restarted attempt's own (#16).
        let parse = first_parse
            .take()
            .unwrap_or_else(|| agent::ParseState::for_spec(agent, &attempt_spec));
        let mut core = WorkflowStreamCore::new(slug.clone(), agent, evidence.clone())
            .budgeted(
                parse,
                crate::budgets::BudgetGuard::for_agent(&budget, agent),
            )
            .with_activity(Arc::clone(&activity), stall_after);
        let driven = drive_workflow_stream(
            child,
            guard,
            &stop,
            deadline,
            StderrPolicy::Echo(format!("[workflow {slug} stderr]")),
            Timings::default(),
            &mut core,
            &mut ledger,
            &mut |payload: Value| emit(&app, payload),
        )
        .await;
        // Spawn, first output and turn (Paperclip PR 15c), read from the
        // lease's admission clock and the run's activity clock together, now
        // that the processes are gone. `drive` stamped the spawn.
        if let Ok(mut collector) = evidence.lock() {
            collector.timings_mut().record_drive(
                lease.admitted_elapsed_ms(),
                activity.since_spawn_ms(),
                activity.first_outputs().for_agent(agent),
            );
        }
        drop(ticker);
        let budget_stop = core.budget_stop().cloned();
        let result_subtype = core.result_subtype().map(str::to_string);
        let result_error = core.result_error();
        let result_record = core.result_record();
        let last_quota = core.last_quota().cloned();
        let tool_calls = core.tool_calls();
        compactions = compactions.saturating_add(core.compactions());
        let (text, flush) = core.finish();
        if let Some(payload) = flush {
            emit(&app, payload);
        }
        // #16: an automation whose Claude account ran out restarts once on
        // another eligible account (`account_pool`), in the same run.
        if agent == AgentKind::Claude && !driven.missing_stdout && driven.ended != Ended::Stopped {
            let account = crate::claude_accounts::account_of(&cmd);
            let backstop = result_subtype.as_deref() == Some(crate::budgets::BACKSTOP_SUBTYPE);
            let kind = crate::result_kind::classify(&crate::result_kind::ResultFacts {
                agent,
                ended: driven.ended,
                exit_success: matches!(&driven.status, Some(Ok(s)) if s.success()),
                result: result_record.as_ref(),
                stderr_tail: &driven.stderr_tail,
                budget_stop: budget_stop.is_some() || backstop,
                stalled_minutes: None,
            });
            let facts = crate::account_pool::AttemptFacts {
                account: account.as_deref(),
                kind,
                outage: crate::result_kind::is_outage(result_record.as_ref(), &driven.stderr_tail),
                last_quota: last_quota.as_ref(),
                // A person's stop ends as `Stopped`, excluded above.
                stopped: budget_stop.is_some() || backstop,
                spending_backstop: attempt_spec.max_budget_usd.is_some(),
                switches,
            };
            if let Some(switch) =
                crate::account_pool::judge(&crate::account_pool::ClaudePool, &facts).await
            {
                switches += 1;
                let mut next = spec.clone();
                next.extras.account = Some(switch.to.clone());
                next.prompt = format!(
                    "{}\n\n{}",
                    spec.prompt,
                    crate::account_pool::restart_note(&switch.from_label, tool_calls)
                );
                cmd = crate::prep_timing::timed_command(agent, &next).0;
                attempt_spec = next;
                // The exhausted attempt keeps its own row (and account); the
                // restart gets a new one under the same run id.
                let restarted = ledger.sibling();
                let _ = std::mem::replace(&mut ledger, restarted).finish(LedgerOutcome::Error);
                ledger.set_agent(agent, model.clone());
                ledger.set_billing(crate::usage::classify(agent, &cmd, true));
                if let Ok(mut collector) = evidence.lock() {
                    collector.record_switch(crate::evidence::ReceiptSwitch {
                        kind: crate::evidence::SWITCH_ACCOUNT.into(),
                        reason: crate::account_pool::switch_reason(&switch.from_label, tool_calls),
                        from: switch.from_label.clone(),
                        to: switch.to_label.clone(),
                    });
                }
                emit(
                    &app,
                    json!({
                        "type": "accountSwitched",
                        "slug": slug,
                        "runId": &run_id,
                        "from": switch.from_label,
                        "to": switch.to_label,
                        "toolCalls": tool_calls,
                    }),
                );
                continue;
            }
        }
        break (
            driven,
            budget_stop,
            result_subtype,
            result_error,
            result_record,
            text,
        );
    };
    // Covers natural completion, error and timeout; cancel already removed it.
    drop(slot);
    if let Ok(mut collector) = evidence.lock() {
        collector.set_process_stop(driven.outcome);
        collector.set_leftovers(driven.left_running());
        collector.set_compactions(compactions);
    }

    if driven.missing_stdout {
        let err = format!("{} produced no stdout", agent.as_str());
        finalize(
            &app,
            &vault,
            &workflow,
            &slug,
            &run_id,
            lease,
            evidence,
            RunFinish {
                status: "error",
                error: Some(&err),
                error_tail: None,
                duration_sec: 0,
                output: "",
                ledger,
                outcome: LedgerOutcome::Error,
                exit_kind: None,
                claim: Some(&budget),
                ctx: &ctx,
                started_at: Some(started_at),
                failure: Some(workflow_retry::classify_failure(&FailureFacts::at(
                    FailureStage::MissingStdout,
                ))),
            },
        )
        .await;
        return Err(err);
    }

    // How the run ended (Paperclip PR 8b's `classify_exit`): a budget stop
    // (the guard's kill, or Claude's backstop exit, checked before the exit
    // status) is an error with exit kind `budget`, never a cancel.
    // A watchdog stall (Paperclip PR 15b) is worded with the limit that
    // fired; `stalled_minutes` makes it the `stalled` result kind.
    // A deadline shorter than the fixed hour (merged plan slice 3) is
    // worded with its own minutes.
    let stalled_minutes =
        (driven.ended == Ended::Stalled).then(|| crate::run_activity::stall_minutes(stall_after));
    let exit = match stalled_minutes {
        Some(minutes) => stalled_exit(minutes),
        None if driven.ended == Ended::TimedOut && deadline < RUN_TIMEOUT => {
            deadline_exit(deadline)
        }
        None => classify_exit(
            driven.ended,
            driven.status.as_ref(),
            text,
            budget_stop.as_ref().map(|notice| notice.message.as_str()),
            result_subtype.as_deref(),
            agent,
        ),
    };
    let exit_kind = exit.exit_kind;
    // Paperclip PR 14a: the run's result kind, beside (never instead of)
    // 8b's `classify_exit`. It goes on the receipt; a refusal or denied
    // actions turn a would-be success into an error; and the displayed
    // error gets the kind's label, while PR 9b's classifier (below) and
    // 9a-2's streak see the raw error.
    let result_kind = crate::result_kind::classify(&crate::result_kind::ResultFacts {
        agent,
        ended: driven.ended,
        exit_success: matches!(&driven.status, Some(Ok(s)) if s.success()),
        result: result_record.as_ref(),
        stderr_tail: &driven.stderr_tail,
        budget_stop: exit_kind.is_some(),
        stalled_minutes,
    });
    if let Ok(mut collector) = evidence.lock() {
        collector.set_result_kind(result_kind);
    }
    let output = exit.output;
    let applied = apply_result_kind(result_kind, result_record.as_ref(), exit.status, exit.error);
    let run_status = applied.status;
    // The usage row follows the recorded status, as chat's does: a refusal
    // or denied actions that exited 0 is an `error` row (Paperclip PR 14a).
    let ledger_outcome = if exit_kind.is_some() {
        LedgerOutcome::BudgetStopped
    } else {
        usage_ledger::outcome_for(driven.ended, run_status == "success")
    };

    // A failed run's log entry carries the actual error: the last lines of
    // the (bounded) stderr tail, not just the exit status.
    let error_tail = if run_status == "error" {
        let lines: Vec<&str> = driven.stderr_tail.lines().collect();
        let start = lines.len().saturating_sub(STDERR_TAIL_LINES);
        Some(lines[start..].join("\n"))
            .filter(|s| !s.trim().is_empty())
            .map(|s| tail_truncate(&s, STDERR_TAIL_MAX_CHARS))
    } else {
        None
    };

    // Paperclip PR 9b: classify a failure from the exit branch (Hollow's own
    // stops first, then an outside signal only when the leader died before
    // the stop sequence, then the CLI's error envelope). Only a transient
    // failure of an unattended run earns its one retry, decided in finalize.
    let failure = run_failure(
        &applied,
        &RunEnd {
            ended: driven.ended,
            budget_stop: exit_kind.is_some(),
            signal: match &driven.status {
                Some(Ok(status)) => workflow_retry::exit_signal(status),
                _ => None,
            },
            leader_exited_first: driven.leader_exited_first,
            result_subtype: result_subtype.as_deref(),
            result_error: result_error.as_deref(),
            stderr_tail: &driven.stderr_tail,
        },
    );
    let error = applied.error;

    let duration_sec = started.elapsed().as_secs();
    let trimmed_output = truncate(&output, OUTPUT_TRUNCATE_CHARS);

    let finalization = finalize(
        &app,
        &vault,
        &workflow,
        &slug,
        &run_id,
        lease,
        evidence,
        RunFinish {
            status: run_status,
            error: error.as_deref(),
            error_tail,
            duration_sec,
            output: &trimmed_output,
            ledger,
            outcome: ledger_outcome,
            exit_kind,
            claim: Some(&budget),
            ctx: &ctx,
            started_at: Some(started_at),
            failure,
        },
    )
    .await;

    Ok(WorkflowRunResult {
        slug,
        status: run_status.to_string(),
        duration_sec,
        output: trimmed_output,
        error,
        receipt: finalization.receipt,
        evidence_error: finalization.evidence_error,
    })
}

/// Finalize a run stopped before its agent spawned (during preparation or
/// just before the spawn) as a neutral cancelled run, never an error. The
/// caller has already dropped the stop slot. The usage row records the run as
/// cancelled (never spawned, so its figures are unknown).
#[allow(clippy::too_many_arguments)]
async fn cancel_before_spawn(
    app: &Core,
    vault: &Path,
    workflow: &Workflow,
    slug: &str,
    run_id: &str,
    lease: RunLease,
    evidence: Arc<Mutex<crate::evidence::EvidenceCollector>>,
    ledger: UsageRun,
    ctx: &RunContext,
) -> WorkflowRunResult {
    let error = "cancelled by user".to_string();
    let finalization = finalize(
        app,
        vault,
        workflow,
        slug,
        run_id,
        lease,
        evidence,
        RunFinish {
            status: "cancelled",
            error: Some(&error),
            error_tail: None,
            duration_sec: 0,
            output: "",
            ledger,
            outcome: LedgerOutcome::Cancelled,
            exit_kind: None,
            claim: None,
            ctx,
            started_at: None,
            failure: None,
        },
    )
    .await;
    WorkflowRunResult {
        slug: slug.to_string(),
        status: "cancelled".to_string(),
        duration_sec: 0,
        output: String::new(),
        error: Some(error),
        receipt: finalization.receipt,
        evidence_error: finalization.evidence_error,
    }
}

/// What [`prepare_spawn`] needs for one admitted run. A struct rather than
/// positional parameters so later PRs (budgets, prompt variables) add fields
/// instead of touching every call.
struct SpawnPlan<'a> {
    workflow: &'a Workflow,
    prompt: String,
    slug: &'a str,
    /// The run's history id: team-mcp attributes delegations to it, and the
    /// run's finish closes any delegation still recorded as running.
    run_id: &'a str,
    /// Why the run is happening: `{{trigger_reason}}` reads it.
    ctx: &'a RunContext,
    /// The claim's usage budget (Paperclip PR 8b): a Claude run streams
    /// partial messages when a token budget watches it, and gets the
    /// `--max-budget-usd` backstop under an enforced spending budget.
    budget: &'a crate::budgets::RunBudget,
    /// How the agent is resolved and the backstop probed ([`SpawnSeams::installed`]
    /// in production; tests inject both so no installed CLI is needed).
    seams: SpawnSeams,
    /// The run's scratch folder (Paperclip PR 15a), made by `execute` once
    /// the lease is held: it goes on the spec and to the team server.
    /// `execute` always sets it; tests may leave it `None`.
    scratch: Option<crate::scratch::ScratchSpec>,
}

/// A boxed probe future ([`agent::claude_max_budget_supported`]).
type ProbeFuture = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;

/// The two things [`prepare_spawn`] asks of the machine rather than the
/// vault: which agent and model run (it needs the CLI installed), and
/// whether the installed Claude CLI supports `--max-budget-usd`.
#[derive(Clone, Copy)]
struct SpawnSeams {
    resolve: fn(Option<&str>, Option<&str>) -> Result<(AgentKind, Option<String>), String>,
    probe: fn() -> ProbeFuture,
}

impl SpawnSeams {
    fn installed() -> Self {
        fn probe() -> ProbeFuture {
            Box::pin(agent::claude_max_budget_supported())
        }
        SpawnSeams {
            resolve: crate::agent_settings::resolve,
            probe,
        }
    }
}

/// A workflow's resolved agent and model, built command and cited recall,
/// ready to spawn.
struct PreparedSpawn {
    agent: AgentKind,
    model: Option<String>,
    cmd: tokio::process::Command,
    recall_items: Vec<crate::vault_learning::RecallItem>,
    /// What prepared the run's context (agent effectiveness F11), for its
    /// receipt.
    manifest: crate::run_manifest::RunManifest,
    /// The stream's parse mode (usage-only partial messages, PR 8b).
    parse: agent::ParseState,
    /// The folder the agent runs in (Paperclip PR 16's repository targets
    /// are chosen from it).
    cwd: PathBuf,
    /// The spec the command was built from: a Claude account switch (#16)
    /// rebuilds the restarted attempt from it.
    spec: agent::RunSpec,
}

/// A workflow's allowlist as the spawn passes it. An empty list (no
/// restriction) stays empty. A tool-restricted workflow must explicitly
/// permit each attached Hollow server, or its allowlist would block their
/// tools, so the whole-server token of each server in `servers` is added
/// once; an entry the author already wrote is not repeated, and a server that
/// isn't attached adds nothing.
pub(crate) fn restricted_allowlist(
    allowed: &[String],
    servers: &serde_json::Map<String, serde_json::Value>,
) -> Vec<String> {
    let mut tools = allowed.to_vec();
    if tools.is_empty() {
        return tools;
    }
    for (server, token) in [
        ("hollow_team", crate::team_mcp::TEAM_TOOL_ALLOW),
        (
            crate::tasks_mcp::SERVER_NAME,
            crate::tasks_mcp::TASKS_TOOL_ALLOW,
        ),
    ] {
        if servers.contains_key(server) && !tools.iter().any(|t| t.trim() == token) {
            tools.push(token.to_string());
        }
    }
    tools
}

/// A workflow run's prompt with its context around it, and what its recall
/// and connector selection read.
pub(crate) struct PromptContext {
    pub prompt: String,
    pub recall_items: Vec<crate::vault_learning::RecallItem>,
    /// What `connectors::attach` selects "when relevant" connectors on.
    #[cfg_attr(not(test), allow(dead_code))]
    pub connector_query: crate::context_rank::ConnectorQuery,
    /// The operator-profile block as wrapped into `prompt` (empty when
    /// none), measured into the run's context manifest (F11).
    pub operator_block: String,
    /// The approved-recall block (empty when none), measured likewise.
    pub recall_block: String,
    /// Recall's history cue and lifecycle decisions (F7), for the manifest.
    pub recall_history: Option<&'static str>,
    pub recall_lifecycle: Vec<crate::run_manifest::LifecycleDecision>,
    /// Whether recall ranked by meaning (OF26 review), for the manifest.
    pub recall_meaning: crate::vault_learning::RecallMeaning,
    /// How long recall took (agent effectiveness F8).
    pub recall_mark: Mark,
}

/// Wrap the run's request in its context, innermost first: retrieved
/// evidence, the operator profile, approved recall, the workspace envelope.
/// Recall and the connector query read `request` (its words and the notes it
/// names), so none of that context can choose memories or connectors (agent
/// effectiveness F2: each used to read the prompt expanded so far).
pub(crate) fn compose_run_context(
    vault: &Path,
    request: &crate::turn_request::TurnRequest,
    retrieved: &str,
    operator_body: &str,
    workspace_context: Option<&str>,
) -> PromptContext {
    let prompt = format!("{retrieved}{}", request.request);
    let operator_block = if operator_body.is_empty() {
        String::new()
    } else {
        format!(
            "<operator-context>\n{operator_body}\n\n(Durable context about how the user works and wants you to respond. Apply it to this run's output: match their communication style, honor their settled decisions and anti-patterns. Do not echo it back.)\n</operator-context>\n\n"
        )
    };
    let prompt = format!("{operator_block}{prompt}");
    let recall_started = Instant::now();
    // After retrieval, which embedded the same request: its vector is in
    // `embed_model`'s query cache, or (retrieval's semantic half cut at the
    // deadline and still running) recall's bounded wait queues behind it
    // and reads the cache once it holds the model (OF26 review).
    let recall = request.recall(vault);
    let recall_mark = Mark::since(recall_started);
    let prompt = if recall.prompt_block.is_empty() {
        prompt
    } else {
        format!("{}\n\n{}", recall.prompt_block, prompt)
    };
    let prompt = match workspace_context {
        Some(context) => format!("{context}{prompt}"),
        None => prompt,
    };
    PromptContext {
        prompt,
        recall_items: recall.items,
        connector_query: request.connector_query(),
        operator_block,
        recall_block: recall.prompt_block,
        recall_history: recall.history,
        recall_lifecycle: recall.lifecycle,
        recall_meaning: recall.meaning,
        recall_mark,
    }
}

/// Put the manifest `prepare_spawn` built on the run's evidence collector,
/// stamped with the run's id; the receipt stores it as its sidecar
/// (`evidence::EvidenceCollector::finish`).
fn adopt_manifest(
    collector: &mut crate::evidence::EvidenceCollector,
    run_id: &str,
    manifest: crate::run_manifest::RunManifest,
) {
    *collector.context_mut() = crate::run_manifest::RunManifest {
        run_id: run_id.to_string(),
        ..manifest
    };
}

/// The outside effects of `failed`, the run a guarded retry of `slug`
/// retries: its receipt must be this automation's. A run whose receipt
/// recorded none has empty effects (nothing to guard, no note).
pub(crate) fn guarded_retry_effects(
    vault: &Path,
    slug: &str,
    failed: &str,
) -> Result<crate::run_effects::OutsideEffects, String> {
    let receipt = crate::evidence::find_by_history_run(vault, failed)
        .map_err(|e| format!("couldn't read the failed run's record, so the retry didn't start: {e}"))?
        .ok_or_else(|| format!("Hollow has no record of run {failed}, so it can't tell what that run already did; the retry didn't start. Use Run now instead."))?;
    if receipt.source != "workflow" || receipt.source_id.as_deref() != Some(slug) {
        return Err(format!(
            "run {failed} isn't a run of this automation; the retry didn't start."
        ));
    }
    Ok(receipt.outside_effects.unwrap_or_default())
}

/// Resolve the agent and build the command for an admitted run. Read-only
/// (retrieval, workspace, operator and recall context do no vault writes),
/// so it is safe inside the run's own lease.
///
/// Lease-hold cost: this runs AFTER the lease is taken, so its time is time
/// the vault is held before the agent even starts. Retrieval is the bulk of
/// it (Paperclip PR 4's multi-term search scans up to 16 terms over up to
/// 32 MB of notes), and a chat send in that window is refused with "Hollow is
/// busy: workflow `…` holds the vault". A stop pressed meanwhile is honored
/// before spawn, including during Codex's connector probe in
/// `build_command`.
async fn prepare_spawn(vault: &Path, plan: SpawnPlan<'_>) -> Result<PreparedSpawn, String> {
    let SpawnPlan {
        workflow,
        prompt,
        slug,
        run_id,
        ctx,
        budget,
        seams,
        scratch,
    } = plan;
    // Plan completion L2: "Run again without repeating". The guard is in
    // place before anything spawns, or the run doesn't start.
    let prompt = match ctx.guarded_retry_of.as_deref() {
        Some(failed) => {
            let effects = guarded_retry_effects(vault, slug, failed)?;
            if let Some(guard) = crate::run_effects::guard_for(run_id, failed, &effects) {
                crate::run_effects::write_guard(&guard)?;
            }
            match crate::run_effects::retry_note(failed, &effects) {
                Some(note) => format!("{prompt}\n\n{note}"),
                None => prompt,
            }
        }
        None => prompt,
    };
    // Merged plan decision 6: a claim that found the workflow's agent at its
    // own budget ceiling names the agent with room; it runs on that agent's
    // own model setting (the workflow's model belongs to its agent).
    let (agent, model) = match budget.fallback.as_ref() {
        Some(fallback) => (seams.resolve)(Some(fallback.to.as_str()), None)?,
        None => (seams.resolve)(workflow.agent.as_deref(), workflow.model.as_deref())?,
    };
    // The saved reasoning effort (Paperclip PR 14a): app-wide per agent, not
    // a workflow field (so not PR 5-stamped). Unvalidated here: an
    // unsupported level fails `validate_run_spec` below, visibly. COO
    // rebuild L7: in Automatic mode the CFO picks the level from the
    // prompt (an automation is a report unless it says otherwise).
    let effort = crate::cfo::run_effort(
        &crate::cfo::settings_for(Some(vault)),
        &prompt,
        crate::cfo::Origin::Automation,
        crate::agent_settings::effort(agent)?,
    );

    // Every provider works in the explicit directory or the connected vault.
    let spawn_cwd = workflow
        .cwd
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| vault.to_path_buf());
    // Paperclip PR 19: `precheck` already refused a bad working folder; this
    // re-check covers one removed or swapped since the claim, so the run
    // fails visibly (a Prepare-stage configuration error, never retried)
    // instead of spawning into ENOENT or a system folder.
    if let Some(problem) = workflow_cwd_problem(vault, workflow.cwd.as_deref()) {
        return Err(problem);
    }

    // Prompt variables (Paperclip PR 9, 9a-2), before retrieval so retrieval
    // and recall see the same text the agent gets. Substitution is on this
    // spawn-time copy only and is never written back, so PR 5's permission
    // stamp (`prompt_sha256` over the file's prompt) is unaffected. The
    // workspace lookup swallows its errors: `workspaces::context` below still
    // reports them exactly as before.
    let vars = workflow_hygiene::template_vars(vault, workflow, &spawn_cwd, ctx, Local::now());
    let (prompt, external_values) = workflow_hygiene::compose_prompt(&prompt, &vars);
    let var_note = if external_values {
        format!("\n{}", workflow_hygiene::HOLLOW_VAR_NOTE)
    } else {
        String::new()
    };

    let retrieval_vault = vault.to_path_buf();
    let retrieval_cwd = spawn_cwd.clone();
    let retrieval_query = prompt.clone();
    // What prepared this run's context (agent effectiveness F11): sizes,
    // hashes and decisions only, stored with its receipt.
    let mut manifest = crate::run_manifest::RunManifest::new();
    manifest.layer("request", &prompt);
    // The run's request is its prompt after variable substitution (agent
    // effectiveness F2): recall, connector selection and the attached
    // sources (F3 part 2) read it and the notes it names, never the prompt
    // expanded with evidence, profile, recall and workspace context.
    let request = crate::turn_request::TurnRequest::from_text(vault, &prompt);
    let connector_query = request.connector_query();
    let permission_mode = if workflow.permission_mode.is_empty() {
        "acceptEdits".to_string()
    } else {
        workflow.permission_mode.clone()
    };
    // Retrieval and the library connectors switched on for automations are
    // prepared at once, under one deadline for their optional parts (agent
    // effectiveness F8, O2), as a chat turn's are.
    let deadline = crate::prep_deadline::PrepDeadline::start();
    let jev = crate::jev::Target::app().await;
    let ((retrieved, retrieval_mark), (attached_library, connectors_mark)) =
        crate::prep_deadline::concurrently(
            crate::directory_connections::prioritized_context(
                retrieval_vault,
                retrieval_cwd,
                retrieval_query,
                &deadline,
                &jev,
            ),
            // An automation always selects afresh: no conversation (S28).
            crate::connectors::attach(
                agent,
                &permission_mode,
                true,
                &connector_query,
                &deadline,
                &jev,
                None,
            ),
        )
        .await;
    let retrieved = retrieved?;
    manifest.preparation.retrieval_ms = Some(retrieval_mark.ms);
    manifest.timer.0.record(Step::Retrieval, retrieval_mark);
    manifest.layer("retrievalEvidence", &retrieved.text);
    manifest.set_retrieval(retrieved.retrieval, retrieved.jev);
    manifest.add_lifecycle(retrieved.history, retrieved.lifecycle);
    let retrieved = retrieved.text;
    // An automation has no hollow_vault (no reader): its notes point at its
    // own file tools.
    let sources_started = Instant::now();
    let workspace_context = crate::workspaces::context_for(
        vault,
        &spawn_cwd,
        &crate::workspaces::SourceRequest {
            query: &request.source_query(),
            reader: crate::workspaces::SourceReader::OwnTools,
            child_packet: None,
        },
    )?;
    let (workspace_context, attached_sources) = match workspace_context {
        Some(c) => {
            manifest
                .timer
                .0
                .record(Step::AttachedSources, Mark::since(sources_started));
            (Some(c.text), c.sources)
        }
        None => (None, None),
    };
    manifest.attached_sources = attached_sources;
    // The user's profile and approved memory apply to every run, inside a
    // workspace too (2026-09-26); the workspace envelope still leads.
    let operator = if crate::operator::is_enabled() {
        let compile_started = Instant::now();
        let operator = crate::operator::compile(vault);
        manifest
            .timer
            .0
            .record(Step::ContextCompile, Mark::since(compile_started));
        operator
    } else {
        Default::default()
    };
    let PromptContext {
        prompt,
        recall_items,
        connector_query: _,
        operator_block,
        recall_block,
        recall_history,
        recall_lifecycle,
        recall_meaning,
        recall_mark,
    } = compose_run_context(
        vault,
        &request,
        &retrieved,
        &operator.body,
        workspace_context.as_deref(),
    );
    manifest.layer("operatorProfile", &operator_block);
    manifest.layer("recall", &recall_block);
    manifest.add_lifecycle(recall_history, recall_lifecycle);
    manifest.recall_meaning = recall_meaning.recorded();
    manifest.timer.0.record(Step::Recall, recall_mark);
    if let Some(context) = workspace_context.as_deref() {
        manifest.layer("workspace", context);
    }

    // The Hollow servers an automation gets (`mcp_attach`, only for an
    // adapter with owned MCP): teamwork (never for a `plan`
    // workflow: a teammate would be a way to make changes, Paperclip PR 13)
    // and the task tools. Both get the
    // vault root separately from the run's cwd, so delegation usage rows and
    // task writes land in this vault, credited to this workflow and run. The
    // task server reads only when the automation is in plan mode or its
    // (PR 5-stamped) allowed tools grant no write (Paperclip PR 12).
    let parent = DelegateParent {
        surface: Surface::Workflow,
        scope: slug.to_string(),
        run_id: run_id.to_string(),
        scratch: scratch
            .as_ref()
            .and_then(|s| s.dir())
            .map(Path::to_path_buf),
        // Teammates keep the workflow's reviewed tool lists
        // (`team_mcp::delegate_grant`).
        tool_limits: crate::team_mcp::ToolLimits::new(
            &workflow.allowed_tools,
            &workflow.disallowed_tools,
        ),
    };
    let tasks_access =
        crate::tasks_mcp::TasksAccess::for_workflow(&permission_mode, &workflow.allowed_tools);
    // Hollow app tools (PR A1): unrestricted automations read and write; a
    // tool-restricted one gets the server only when its (stamped) allowed
    // tools name it, so a reviewed list is never widened to all of Hollow.
    let app_access =
        crate::app_mcp::AppAccess::for_workflow(&permission_mode, &workflow.allowed_tools);
    let app_grant =
        app_access.map(|access| crate::app_mcp::AppGrant::new(access, 0, &permission_mode));
    crate::app_broker::register_workflow_run(&parent.run_id, &workflow.allowed_tools, tasks_access);
    // The agent it really runs on (a budget fallback's included), so a
    // session it starts inherits its mode from that agent (Q-R3 round 3).
    crate::app_broker::set_workflow_run_agent(&parent.run_id, agent);
    let (servers, attached) = crate::mcp_attach::server_map_for(
        agent,
        &permission_mode,
        &spawn_cwd,
        vault,
        &parent,
        tasks_access,
        app_grant,
    );
    let allowed_tools = restricted_allowlist(&workflow.allowed_tools, &servers);
    // Library connectors switched on for automations (selected above, beside
    // retrieval). A tool-restricted automation (a stamped permission) gets
    // only those its allowed tools name as `mcp__<id>`, so the library never
    // widens a reviewed list.
    // Everything from here to the command is prompt assembly (F8).
    let assembly_started = Instant::now();
    manifest.preparation.connectors_ms = Some(connectors_mark.ms);
    manifest.timer.0.record(Step::Connectors, connectors_mark);
    manifest
        .timer
        .0
        .record_some(Step::MicrosoftAuth, attached_library.auth_warm);
    manifest.set_connectors(attached_library.selection);
    manifest.set_deadline(&deadline, attached_library.auth_cut);
    let shared: Vec<crate::connectors::Attachment> = attached_library
        .connectors
        .into_iter()
        .filter(|a| {
            workflow.allowed_tools.is_empty()
                || workflow
                    .allowed_tools
                    .iter()
                    .any(|t| t == &format!("mcp__{}", a.id))
        })
        .collect();
    let shared_connectors: Vec<agent::SharedConnector> = shared
        .iter()
        .map(|a| agent::SharedConnector {
            id: a.id.clone(),
            auto_approve: a.auto_approve,
            claude_login: a.claude_login,
            name: a.name.clone(),
            remedy: a.remedy.clone(),
        })
        .collect();
    let mut servers = servers;
    for attachment in &shared {
        servers.insert(attachment.id.clone(), attachment.entry.clone());
    }
    let connectors_note = crate::connectors::prompt_note(&shared, &permission_mode, true);
    manifest.layer("connectorsNote", &connectors_note);
    manifest.capabilities = crate::run_manifest::Capabilities {
        owned_servers: crate::chat_resume::mcp_server_names(&servers)
            .into_iter()
            .filter(|name| !shared.iter().any(|a| &a.id == name))
            .collect(),
        connectors: shared.iter().map(|a| a.id.clone()).collect(),
        // Set from the finished spec, where its command is built (OB23).
        bash_output_max_chars: None,
        // The same (OB22); the count is set when the receipt is written.
        compact_window: None,
        compactions: None,
    };
    let prompt = format!("{connectors_note}{prompt}");
    let mcp_config = crate::mcp_attach::merge(servers);
    // The task guidance rides after the boundary block, outside prompt
    // variable substitution, byte-stable (one const per access).
    let tasks_block = attached.tasks.then(|| {
        let guidance = if attached.tasks_read_only {
            crate::tasks_mcp::PROMPT_GUIDANCE_READ_ONLY
        } else {
            crate::tasks_mcp::PROMPT_GUIDANCE
        };
        format!("<hollow-tasks>\n{guidance}\n</hollow-tasks>")
    });
    let app_block = attached.app.then(|| {
        format!(
            "<hollow-app>\n{}\n</hollow-app>",
            crate::app_mcp::prompt_guidance(&permission_mode, true, attached.app_read_only)
        )
    });
    let after: Vec<&str> = tasks_block
        .iter()
        .chain(app_block.iter())
        .map(String::as_str)
        .collect();
    // The scratch guidance (Paperclip PR 15a) sits inside the boundary,
    // after 9a-2's variable note; it names only `$HOLLOW_SCRATCH_DIR`.
    let mut spec = agent::RunSpec {
        prompt: workflow_boundary_prompt(
            &var_note,
            &[crate::scratch::SCRATCH_GUIDANCE],
            &after,
            &prompt,
        ),
        resume: None,
        model: model.clone(),
        perm: permission_mode,
        cwd: spawn_cwd,
        allowed_tools,
        disallowed_tools: workflow.disallowed_tools.clone(),
        interactive: false,
        mcp_config,
        // Workflows decide their own output format from the prompt body (HTML
        // briefs included) — no blanket HTML steer; that's a chat-only default.
        append_system_prompt: None,
        process_scope: agent::ProcessScope::GuiOwned,
        usage_stream: false,
        max_budget_usd: None,
        extras: agent::RunExtras {
            effort,
            scratch,
            shared_connectors: shared_connectors.clone(),
            // #16: start away from an account past the CFO's line when
            // another has room. COO rebuild L7 (review H1): on the CFO's pick
            // among the accounts the run pump judged the hold on.
            long_work: true,
            account: None,
            // OB22: the compaction window from Settings, when set.
            env: crate::agent_settings::compaction_env(agent),
            ..Default::default()
        },
    };
    // Usage budgets (Paperclip PR 8b), Claude only: partial messages when a
    // token budget watches the run, so its guard reads final per-message
    // usage; the spending backstop under an enforced spending budget, when
    // the installed CLI supports it (probed once per binary, outside the
    // vault). Neither is a stamped workflow field.
    agent::apply_run_budget(agent, &mut spec, budget, seams.probe).await;
    agent::validate_run_spec(agent, &spec)?;
    manifest
        .timer
        .0
        .record(Step::PromptAssembly, Mark::since(assembly_started));
    agent::refresh_accounts(agent, &spec).await;
    let (cmd, built, inventory) = crate::prep_timing::timed_command(agent, &spec);
    manifest.timer.0.record(Step::Command, built);
    manifest.set_codex_inventory(inventory);
    manifest.capabilities.bash_output_max_chars = agent::bash_output_cap(agent, &spec);
    // OB22: the compaction window its command carries (env or `-c`).
    manifest.capabilities.compact_window = agent::compact_window(agent, &spec);
    // S13: what Claude Code loads by itself (CLAUDE.md, auto memory, skills).
    manifest.set_native_memory(agent, &spec.cwd, spec.extras.remote.is_some());
    // S26: notes opened outside it are the receipt's `notesOpened`.
    manifest.set_run_cwd(&spec.cwd);
    // The boundary block and the tool guidance after it: the run's policy.
    if let Some(policy) = spec.prompt.strip_suffix(prompt.as_str()) {
        manifest.layer("policy", policy);
    }
    Ok(PreparedSpawn {
        agent,
        model,
        cmd,
        recall_items,
        manifest,
        parse: agent::ParseState::for_spec(agent, &spec),
        cwd: spec.cwd.clone(),
        spec,
    })
}

/// The messaging boundary every automation prompt starts with.
const WORKFLOW_BOUNDARY: &str = "Hollow has no built-in email, Slack or Teams features. Do not read or sync retired message folders or rebuild those features. Use local vault notes, tasks, projects, Calendar and recordings, and only the connectors this run was given in a <hollow-connectors> note. An automation never sends, publishes or deletes anything through a connector unless its prompt explicitly asks for exactly that.";

/// An automation's spawn prompt: the `<hollow-workspace-boundary>` block
/// (with 9a-2's prompt-variable note, then each `inside` line on its own
/// line), then each `after` block followed by a blank line, then the
/// prompt (cross-m34 §4; PR 12 puts `<hollow-tasks>` after the boundary,
/// PR 15a's scratch note goes inside). With no extras the text is exactly
/// what it was before PR 12.
fn workflow_boundary_prompt(
    var_note: &str,
    inside: &[&str],
    after: &[&str],
    prompt: &str,
) -> String {
    let mut out = format!("<hollow-workspace-boundary>\n{WORKFLOW_BOUNDARY}{var_note}");
    for line in inside {
        out.push('\n');
        out.push_str(line);
    }
    out.push_str("\n</hollow-workspace-boundary>\n\n");
    for block in after {
        out.push_str(block);
        out.push_str("\n\n");
    }
    out.push_str(prompt);
    out
}

/// Translates the agent's stdout via [`agent::parse_line`], fans tool-use /
/// text events out over [`EVENT`] keyed by slug (through the `sink` of
/// [`drive_workflow_stream`]), and yields the assistant output (preferring a
/// `result` envelope when the agent emits one).
///
/// Antigravity streams plain-text deltas rather than discrete text blocks;
/// we accumulate those and flush a single `assistantText` at the end so the
/// run log doesn't get one entry per token.
///
/// It needs no AppHandle: it parses each line, feeds the usage ledger and the
/// evidence collector, keeps the run's text, and returns the events to show
/// instead of emitting them, so it is testable without a running app.
///
/// **Budget guard (Paperclip PR 8b).** Built with [`Self::budgeted`], it
/// checks the ledger's running total against the run's claim-time token
/// budgets after every change; an enforced limit crossed mid-run records the
/// notice ([`Self::budget_stop`]) and requests the core's limit signal, which
/// [`drive_workflow_stream`] passes to `drive` as `Supervision::limit`, so
/// the run ends as `Ended::LimitReached` with the same termination as a
/// stop. Warnings crossed mid-run are left to the finished run's
/// `budgets::after_run`, which notifies them once per window. It also keeps
/// the result subtype, so Claude's backstop exit is recognized. PR 9b adds
/// its result error here.
struct WorkflowStreamCore {
    slug: String,
    agent: AgentKind,
    evidence: Arc<Mutex<crate::evidence::EvidenceCollector>>,
    state: agent::ParseState,
    last_text: String,
    delta_buf: String,
    final_result: Option<String>,
    budget_guard: Option<crate::budgets::BudgetGuard>,
    limit: Arc<StopSignal>,
    budget_stop: Option<crate::budgets::BudgetNotice>,
    result_subtype: Option<String>,
    /// The run's result envelope with API-error messages merged in
    /// (Paperclip PR 14a). `result_kind::classify` reads it, and PR 9b's
    /// failure classifier reads its error text ([`Self::result_error`]).
    results: crate::result_kind::ResultTracker,
    /// The run's activity clock and watchdog limit (Paperclip PR 15b,
    /// [`Self::with_activity`]): [`drive_workflow_stream`] hands them to
    /// `drive` as its `Watch`, and each tool call sets the progress phase.
    activity: Option<Arc<crate::run_activity::RunActivity>>,
    stall_after: Option<Duration>,
    /// The run's last Claude rate-limit event and how many tools it called:
    /// what an account switch (#16) is decided and described with.
    last_quota: Option<crate::claude_quota::StreamQuota>,
    tool_calls: u32,
    /// Background tasks the CLI cut off as it exited (overnight A3),
    /// committed by [`drive_workflow_stream`] after a normal exit.
    cut_tasks: crate::evidence::CutTasks,
}

impl WorkflowStreamCore {
    fn new(
        slug: String,
        agent: AgentKind,
        evidence: Arc<Mutex<crate::evidence::EvidenceCollector>>,
    ) -> Self {
        Self {
            slug,
            agent,
            evidence,
            state: agent::ParseState::default(),
            last_text: String::new(),
            delta_buf: String::new(),
            final_result: None,
            budget_guard: None,
            limit: Arc::new(StopSignal::new()),
            budget_stop: None,
            result_subtype: None,
            results: crate::result_kind::ResultTracker::default(),
            activity: None,
            stall_after: None,
            last_quota: None,
            tool_calls: 0,
            cut_tasks: crate::evidence::CutTasks::default(),
        }
    }

    /// The attempt's last rate-limit event (#16).
    fn last_quota(&self) -> Option<&crate::claude_quota::StreamQuota> {
        self.last_quota.as_ref()
    }

    /// How many tools the attempt called (#16's receipt wording).
    fn tool_calls(&self) -> u32 {
        self.tool_calls
    }

    /// How many times the attempt's agent compacted its context (OB22).
    fn compactions(&self) -> u32 {
        self.state.compactions
    }

    /// The run's activity clock and, for a Codex run, its no-output watchdog
    /// limit (Paperclip PR 15b; `None` switches the watchdog off).
    fn with_activity(
        mut self,
        activity: Arc<crate::run_activity::RunActivity>,
        stall_after: Option<Duration>,
    ) -> Self {
        self.activity = Some(activity);
        self.stall_after = stall_after;
        self
    }

    /// The `Watch` `drive` stamps and watches, when the core has a clock.
    fn watch(&self) -> Option<process_group::Watch> {
        self.activity.as_ref().map(|activity| process_group::Watch {
            activity: Arc::clone(activity),
            stall_after: self.stall_after,
        })
    }

    /// The run's parse mode (`ParseState::for_spec`: usage-only partial
    /// messages when a token budget watches a Claude run) and its budget
    /// guard (None without token budgets).
    fn budgeted(
        mut self,
        state: agent::ParseState,
        guard: Option<crate::budgets::BudgetGuard>,
    ) -> Self {
        self.state = state;
        self.budget_guard = guard;
        self
    }

    /// Fold one stdout line in. Every event reaches the ledger first, then
    /// the budget guard when the running total grew; the returned payloads
    /// are the `workflow-event`s to emit, in order.
    fn on_line(&mut self, line: &str, ledger: &mut UsageRun) -> Vec<Value> {
        let mut shown = Vec::new();
        // Background tasks the CLI cuts off as it exits (overnight A3).
        self.cut_tasks.observe(line);
        for evt in agent::parse_line(self.agent, line, &mut self.state) {
            let grew = ledger.observe(&evt);
            self.results.observe(&evt);
            if let Some(budget_guard) = self.budget_guard.as_mut() {
                let running = if grew { ledger.running_total() } else { None };
                budget_guard.bind_run(ledger.key());
                if let Some(notice) = budget_guard.on_event(&evt, grew, running).stop {
                    self.budget_stop = Some(notice);
                    self.limit.request();
                }
            }
            // The first model event, the first assistant text (agent
            // effectiveness F11; the run's first useful output) and the tool
            // calls before it (OB08a), apart from launch and stderr.
            if let Some(activity) = &self.activity {
                activity.note_event(&evt);
            }
            match evt {
                // An API-error wrapper reads exactly as the text it replaced
                // (Paperclip PR 14a); `results` also counts it as part of the
                // error envelope.
                AgentEvent::AssistantText(t) | AgentEvent::ApiError { text: t, .. } => {
                    if !t.trim().is_empty() {
                        self.last_text = t.clone();
                        shown
                            .push(json!({ "type": "assistantText", "slug": self.slug, "text": t }));
                    }
                }
                AgentEvent::AssistantDelta(t) => self.delta_buf.push_str(&t),
                AgentEvent::ToolUse { id, name, input } => {
                    self.tool_calls = self.tool_calls.saturating_add(1);
                    // The progress phase reads the tool NAME only (PR 15b).
                    if let Some(activity) = &self.activity {
                        activity.set_progress(crate::run_activity::progress_for_tool(&name));
                    }
                    if let Ok(mut collector) = self.evidence.lock() {
                        collector.record_tool_use(&id, &name, &input);
                    }
                    shown.push(
                        json!({ "type": "toolUse", "slug": self.slug, "id": id, "name": name, "input": input }),
                    );
                }
                AgentEvent::ToolResult {
                    id,
                    content,
                    is_error,
                } => {
                    if let Ok(mut collector) = self.evidence.lock() {
                        collector.record_tool_result(&id, &content, is_error);
                    }
                    shown.push(json!({
                        "type": "toolResult",
                        "slug": self.slug,
                        "id": id,
                        "content": content,
                        "isError": is_error,
                    }));
                }
                AgentEvent::Result { text, subtype, .. } => {
                    self.result_subtype = Some(subtype);
                    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
                        self.final_result = Some(t);
                    }
                }
                // Kept for an account switch (#16), not surfaced.
                AgentEvent::RateLimit(quota) => self.last_quota = Some(quota),
                // Session / TurnBoundary / MessageUsage aren't surfaced in
                // the workflow stream.
                _ => {}
            }
        }
        shown
    }

    /// The notice of the budget stop the guard requested, if any.
    fn budget_stop(&self) -> Option<&crate::budgets::BudgetNotice> {
        self.budget_stop.as_ref()
    }

    /// The last result envelope's subtype (`error_max_budget_usd` when
    /// Claude's spending backstop ended the run).
    fn result_subtype(&self) -> Option<&str> {
        self.result_subtype.as_deref()
    }

    /// The error text of an error result envelope, for PR 9b's classifier:
    /// its text, falling back to its structured errors (Claude's budget
    /// result has no text; an API-error message's text is merged into them).
    /// `None` for a non-error envelope.
    fn result_error(&self) -> Option<String> {
        self.results
            .record()
            .filter(|record| record.is_error)
            .and_then(|record| record.error_text())
    }

    /// The run's result envelope (Paperclip PR 14a).
    fn result_record(&self) -> Option<crate::result_kind::ResultRecord> {
        self.results.record()
    }

    /// The run's text (a result envelope wins, then accumulated plain-text
    /// deltas, then the last text block), plus the one `assistantText` that
    /// flushes Antigravity's deltas, if any.
    fn finish(self) -> (String, Option<Value>) {
        let mut last_text = self.last_text;
        let mut flush = None;
        if !self.delta_buf.trim().is_empty() {
            last_text = self.delta_buf.clone();
            flush =
                Some(json!({ "type": "assistantText", "slug": self.slug, "text": self.delta_buf }));
        }
        (self.final_result.unwrap_or(last_text), flush)
    }
}

/// Drive one workflow run's agent to its end through `process_group::drive`
/// (the only waiter), feeding each stdout line to `core` and each payload it
/// returns to `sink` (`execute` emits them as `workflow-event`s; tests
/// collect them). Besides the user's `stop` and the deadline, the core's
/// budget guard may end the run (Paperclip PR 8b): its limit signal is
/// `Supervision::limit`, so the run ends as `Ended::LimitReached` after the
/// same process-tree termination as a stop, and returns only after that.
#[allow(clippy::too_many_arguments)]
async fn drive_workflow_stream(
    child: tokio::process::Child,
    guard: GroupGuard,
    stop: &StopSignal,
    deadline: Duration,
    stderr: StderrPolicy,
    timings: Timings,
    core: &mut WorkflowStreamCore,
    ledger: &mut UsageRun,
    sink: &mut (dyn FnMut(Value) + Send),
) -> process_group::Driven {
    let limit = Arc::clone(&core.limit);
    let supervision = Supervision {
        stop: Some(stop),
        limit: Some(&*limit),
        deadline: Some(deadline),
        stderr,
        timings,
        // The activity clock and watchdog (Paperclip PR 15b), when the core
        // has them; a stall ends the run as `Ended::Stalled`.
        watch: core.watch(),
    };
    let driven = process_group::drive(child, guard, supervision, |line| {
        for payload in core.on_line(line, ledger) {
            sink(payload);
        }
    })
    .await;
    if let Ok(mut collector) = core.evidence.lock() {
        std::mem::take(&mut core.cut_tasks).commit(
            driven.ended == process_group::Ended::Exited && driven.leader_exited_first,
            &mut collector,
        );
    }
    driven
}

/// Paperclip PR 8b: a run a usage budget ended keeps `last_status: error` and
/// carries this exit kind (the run-log marker `<!-- hollow-exit: budget -->`).
const EXIT_KIND_BUDGET: &str = vault_workflows::EXIT_KIND_BUDGET;

/// How an ended workflow run is recorded.
#[derive(Debug, PartialEq, Eq)]
struct ExitClass {
    output: String,
    status: &'static str,
    error: Option<String>,
    /// `Some(EXIT_KIND_BUDGET)` for a budget stop, else `None`.
    exit_kind: Option<&'static str>,
}

/// A budget-stop message: the guard's notice (which already carries
/// `budgets::BUDGET_STOP_PREFIX`), or a generic one when there is none.
fn budget_stop_message(notice: Option<&str>) -> String {
    let prefix = crate::budgets::BUDGET_STOP_PREFIX;
    match notice.map(str::trim).filter(|s| !s.is_empty()) {
        Some(notice) if notice.starts_with(prefix) => notice.to_string(),
        Some(notice) => format!("{prefix} {notice}"),
        None => format!("{prefix} the run passed its token limit."),
    }
}

/// Classify how a workflow run ended. Pure.
///
/// * `LimitReached`: the budget guard ended the run. An error carrying the
///   guard's notice, exit kind budget. The output is dropped, as for any run
///   Hollow ended.
/// * `Stopped`: the user's cancel. `drive` checks the stop before the limit,
///   so a Stop pressed as the limit trips wins.
/// * `TimedOut`: unchanged.
/// * `Exited` with Claude's `error_max_budget_usd` result subtype: Claude's
///   own spending backstop ended the run. The recorded shape exits with
///   status 1 and no result text, so this is checked before the exit status,
///   which would otherwise label it "claude exited 1".
/// * `Exited` otherwise: success on a zero status, else an error naming it.
fn classify_exit(
    ended: Ended,
    status: Option<&std::io::Result<std::process::ExitStatus>>,
    text: String,
    budget_stop: Option<&str>,
    result_subtype: Option<&str>,
    agent: AgentKind,
) -> ExitClass {
    let class = |output: String,
                 run_status: &'static str,
                 error: Option<String>,
                 exit_kind: Option<&'static str>| ExitClass {
        output,
        status: run_status,
        error,
        exit_kind,
    };
    match ended {
        Ended::LimitReached => class(
            String::new(),
            "error",
            Some(budget_stop_message(budget_stop)),
            Some(EXIT_KIND_BUDGET),
        ),
        Ended::Stopped => class(
            String::new(),
            "cancelled",
            Some("cancelled by user".to_string()),
            None,
        ),
        Ended::TimedOut => class(
            String::new(),
            "error",
            Some(format!("timed out after {RUN_TIMEOUT_SECS}s")),
            None,
        ),
        // The watchdog (Paperclip PR 15b): an error, output dropped as
        // for any run Hollow ended. `execute` words it with the limit's
        // minutes ([`stalled_exit`]); this is the minute-less fallback.
        Ended::Stalled => class(
            String::new(),
            "error",
            Some(crate::result_kind::ResultKind::Stalled.label().to_string()),
            None,
        ),
        Ended::Exited if result_subtype == Some(crate::budgets::BACKSTOP_SUBTYPE) => class(
            text,
            "error",
            Some(crate::budgets::backstop_notice(agent).message),
            Some(EXIT_KIND_BUDGET),
        ),
        Ended::Exited => match status {
            Some(Ok(s)) if s.success() => class(text, "success", None, None),
            Some(Ok(s)) => class(
                text,
                "error",
                Some(format!("{} exited {}", agent.as_str(), s)),
                None,
            ),
            Some(Err(e)) => class(text, "error", Some(format!("wait: {e}")), None),
            None => class(
                text,
                "error",
                Some("wait: exit status unavailable".to_string()),
                None,
            ),
        },
    }
}

/// The deadline counterpart of [`stalled_exit`] (merged plan slice 3): a run
/// ended by the agent's own deadline, below the fixed hour, is worded with
/// that deadline's minutes (`result_kind::deadline_message`).
fn deadline_exit(deadline: Duration) -> ExitClass {
    let minutes = crate::run_activity::stall_minutes(Some(deadline));
    ExitClass {
        output: String::new(),
        status: "error",
        error: Some(crate::result_kind::deadline_message(minutes)),
        exit_kind: None,
    }
}

/// A workflow run's deadline: the agent's optional deadline (merged plan
/// slice 3) when it is earlier than the fixed hour, else the hour.
fn run_deadline(agent_deadline: Option<Duration>) -> Duration {
    agent_deadline.unwrap_or(RUN_TIMEOUT).min(RUN_TIMEOUT)
}

/// How a run the no-output watchdog ended is recorded (Paperclip PR 15b;
/// every agent since merged plan slice 3): an error carrying
/// `result_kind::stalled_message` for the limit that fired, no output, no
/// exit kind. Pure.
fn stalled_exit(minutes: u64) -> ExitClass {
    ExitClass {
        output: String::new(),
        status: "error",
        error: Some(crate::result_kind::stalled_message(minutes)),
        exit_kind: None,
    }
}

/// What [`apply_result_kind`] decided for a run (Paperclip PR 14a).
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppliedResult {
    kind: crate::result_kind::ResultKind,
    /// The run's status: `success`, `error` or `cancelled`.
    status: &'static str,
    /// The error as the run produced it: what PR 9b's classifier reads.
    raw_error: Option<String>,
    /// The error the run log and `last_error` show (labelled once).
    error: Option<String>,
}

/// Apply a run's result kind (Paperclip PR 14a) to what [`classify_exit`]
/// decided:
/// - a refusal or denied actions (`result_kind::override_error`) make the
///   run an `error` with that message, whatever the exit said;
/// - any other failure kind on a run `classify_exit` called a success (an
///   error envelope, or API-error messages with no result line, at exit 0)
///   makes it an `error` too, with the envelope's text, so the status and
///   the receipt's result kind never disagree;
/// - otherwise the status stands, the raw error is `classify_exit`'s, and
///   the display error (the run log's note and `last_error`) gets the
///   kind's label once (`ResultKind::label_error`: turn limit, usage limit,
///   rate limit or network, sign-in, resume failure).
///
/// The raw error is what PR 9b's `classify_failure` must see ([`run_failure`]
/// reads only it): the Transient label says "rate limited", which its
/// `RATE_RE` checks before `NETWORK_RE`. Labels are note-safe (no em dash,
/// no angle brackets).
fn apply_result_kind(
    kind: crate::result_kind::ResultKind,
    record: Option<&crate::result_kind::ResultRecord>,
    status: &'static str,
    error: Option<String>,
) -> AppliedResult {
    if let Some(message) = crate::result_kind::override_error(kind, record) {
        return AppliedResult {
            kind,
            status: "error",
            raw_error: Some(message.clone()),
            error: Some(message),
        };
    }
    if status == "success" && kind.is_failure() {
        let raw = record
            .and_then(crate::result_kind::ResultRecord::error_text)
            .unwrap_or_else(|| "the agent reported an error".to_string());
        return AppliedResult {
            kind,
            status: "error",
            error: Some(kind.label_error(&raw)),
            raw_error: Some(raw),
        };
    }
    let display = error.as_deref().map(|raw| kind.label_error(raw));
    AppliedResult {
        kind,
        status,
        raw_error: error,
        error: display,
    }
}

/// How an exited workflow run ended, beyond its result kind: the facts PR
/// 9b's classifier reads.
struct RunEnd<'a> {
    ended: Ended,
    budget_stop: bool,
    signal: Option<i32>,
    leader_exited_first: bool,
    result_subtype: Option<&'a str>,
    result_error: Option<&'a str>,
    stderr_tail: &'a str,
}

/// Classify a failed run for PR 9b's retry (`None` unless it is an error).
/// A refusal or denied actions are decided by the result envelope's
/// structured fields, so they are permanent without reading any text: a
/// stderr line about an internally retried 529 must not schedule a retry of
/// a turn the model finished. Every other failure goes to `classify_failure`
/// with the RAW error, never the labelled display one.
fn run_failure(applied: &AppliedResult, end: &RunEnd<'_>) -> Option<FailureClass> {
    use crate::result_kind::ResultKind;
    if applied.status != "error" {
        return None;
    }
    Some(match applied.kind {
        ResultKind::Refusal => FailureClass::Permanent(workflow_retry::PermanentKind::Refusal),
        ResultKind::PermissionDenied => {
            FailureClass::Permanent(workflow_retry::PermanentKind::Denied)
        }
        _ => workflow_retry::classify_failure(&FailureFacts {
            stage: FailureStage::Run(end.ended),
            budget_stop: end.budget_stop,
            signal: end.signal,
            leader_exited_first: end.leader_exited_first,
            result_subtype: end.result_subtype,
            result_error: end.result_error,
            stderr_tail: end.stderr_tail,
            error: applied.raw_error.as_deref().unwrap_or(""),
        }),
    })
}

/// Record completion, preserving independent occurrences when provenance is
/// unavailable or does not prove this run consumed the schedule.
fn finalize_record(
    vault: &Path,
    workflow: &Workflow,
    ctx: &RunContext,
    run_id: &str,
    error: Option<&str>,
    entry: RunLogEntry,
) -> Result<String, String> {
    use crate::work_origin::ScheduleConsumption;
    let consume = match ctx.queue_id {
        Some(queue_id) => crate::session_store::with_read(vault, |conn| {
            crate::work_origin::consumes_schedule_in(conn, queue_id, run_id, &workflow.slug)
        }),
        None => Ok(ScheduleConsumption::Legacy), // legacy queue-free test admission
    };
    let consume = consume.unwrap_or_else(|error| {
        eprintln!("[workflow {}] schedule preserved: {error}", workflow.slug);
        ScheduleConsumption::Preserve
    });
    let status = entry.status.clone();
    let duration = entry.duration_sec;
    match consume {
        ScheduleConsumption::Legacy => vault_workflows::finalize_run(
            vault,
            &workflow.slug,
            &status,
            error,
            duration,
            vault_workflows::next_after_run(workflow, Utc::now()),
            entry,
        ),
        ScheduleConsumption::Occurrence(due) => vault_workflows::finalize_run_for_occurrence(
            vault,
            &workflow.slug,
            &status,
            error,
            duration,
            (&due, vault_workflows::next_after_run(workflow, Utc::now())),
            entry,
        ),
        ScheduleConsumption::Preserve => vault_workflows::finalize_run_preserving_schedule(
            vault,
            &workflow.slug,
            &status,
            error,
            duration,
            entry,
        ),
    }
}

/// Persist the run outcome to the workflow file, append to its run log,
/// and emit the `done` event. Used by every exit path — natural
/// completion, error, cancellation, timeout — so the side effects stay
/// in one place.
///
/// The per-run notification is planned by
/// `workflow_hygiene::finalize_notification` (Paperclip PR 9, 9a-2). With
/// the default `notify: always` every non-cancelled run records one (cancel
/// is the user's own doing, so a "Workflow cancelled" pop would be noise),
/// but only the runs that need a person interrupt them: a run that wrote a
/// file lands in the notification center unread and silent, while a run
/// that wrote nothing or failed also raises the native banner (see
/// [`produced_artifact`]). `notify: failures` drops the success rows,
/// `never` drops them all, and the third consecutive failure sends one
/// failure-streak notification in place of that run's own, whatever the
/// policy. `notify` comes from the verified snapshot the run executed, so an
/// edit made while it ran applies from the next run. A run PR 5's run guard
/// flagged also gets that separate flag notification; a flagged failing run
/// therefore sends two.
///
/// Order (Paperclip PR 6, shared with chat's `finish_observed_run`): the
/// usage row is finished first, then any teammate delegation still recorded
/// as running under this run is closed, then the usage budgets are brought up
/// to date (`budgets::after_run`, PR 8a), then the run log, then
/// `lease.finish` (the public `finish_run`: PR 5's flags and notification)
/// releases the lease. The ledger steps need no lease and take only the
/// session store's leaf lock, never the history lock or the permission
/// guard. Then, with the lease released, the failure streak
/// (`workflow_hygiene::record_outcome`, a review-queue write attributed
/// through `with_file`) and the notification.
#[allow(clippy::too_many_arguments)]
async fn finalize(
    app: &Core,
    vault: &std::path::Path,
    workflow: &Workflow,
    slug: &str,
    run_id: &str,
    lease: RunLease,
    evidence: Arc<Mutex<crate::evidence::EvidenceCollector>>,
    finish: RunFinish<'_>,
) -> FinalizeOutcome {
    let RunFinish {
        status,
        error,
        error_tail,
        duration_sec,
        output,
        ledger,
        outcome,
        exit_kind,
        claim,
        ctx,
        started_at,
        failure,
    } = finish;
    let agent = ledger.agent();
    let usage = ledger.finish(outcome);
    // OB05: the run's calls and peak context, for its receipt.
    if let Ok(mut collector) = evidence.lock() {
        collector.set_api_usage(usage.calls);
        // S26: the kept passages its output names.
        collector.set_reply_citations(output);
    }
    if let Err(close_error) = usage_ledger::close_orphaned_delegates(vault, run_id) {
        eprintln!("[workflow {slug}] closing this run's teammate usage rows failed: {close_error}");
    }
    // Paperclip PR 16: the run's repository changes, captured after its
    // processes ended and its delegate rows closed, BEFORE `lease.finish`
    // (so the next admitted run's edits can't land here) and outside
    // HISTORY_LOCK. The collector lock is never held across the await; the
    // changes go onto the SHARED collector before it is cloned below. No git
    // runs for a run that never spawned (its baseline was never armed).
    let repo_baseline = evidence
        .lock()
        .ok()
        .and_then(|mut collector| collector.take_repo_baseline());
    // Timed as repoCaptureMs (Paperclip PR 15c) only when there was an armed
    // baseline to capture against.
    let repo_timed = repo_baseline.is_some();
    let repo_started = Instant::now();
    let repo_changes = crate::repo_diff::finish_for_run(vault, run_id, repo_baseline).await;
    let repo_capture_ms = repo_timed.then(|| crate::evidence::ms_since(repo_started));
    let repo_produced = repo_produced(&repo_changes);
    if let Ok(mut collector) = evidence.lock() {
        collector.set_repo_changes(repo_changes);
        collector.timings_mut().repo_capture_ms = repo_capture_ms;
    }
    // Usage budgets (Paperclip PR 8a): after this run's row, before the lease
    // is released, so the next claim already sees any pause it caused, and
    // while the run is still tracked, so its queue writes are attributed.
    // Infallible: it logs and never propagates.
    // With the claim's budget (PR 8b), a `budgets.json` edited during the
    // run is judged at the claim's limits, never a raised one.
    crate::budgets::after_run_claimed(
        &crate::budgets::Store::for_app(vault),
        vault,
        &crate::budgets::RunIntent::workflow(slug, agent),
        claim,
        Utc::now(),
    );
    let completed_at = Utc::now().to_rfc3339();
    // Status + its matching run-log row are one vault-file mutation. The
    // history manifest and evidence store finalize separately below; failures
    // at those boundaries are surfaced through `evidenceError`.
    let _ = finalize_record(
        vault,
        workflow,
        ctx,
        run_id,
        error,
        RunLogEntry {
            at: completed_at,
            status: status.to_string(),
            duration_sec,
            note: if ctx.source == RunSource::Retry {
                format!("retry · {}", short_note(output, error))
            } else {
                short_note(output, error)
            },
            run_id: Some(run_id.to_string()),
            error_tail,
            exit_kind: exit_kind.map(str::to_string),
        },
    );
    // `history_flagged`: PR 5's flags, read before the change list moves. A
    // flagged run, or one whose record could not be finished, never retries.
    let mut history_finished = false;
    let mut history_flagged = false;
    // The history diff and lease release, timed (Paperclip PR 15c).
    let diff_started = Instant::now();
    let finished = lease.finish(status);
    let finalize_diff_ms = crate::evidence::ms_since(diff_started);
    let (receipt, evidence_error, produced) = match finished {
        Ok(history) => {
            history_finished = true;
            history_flagged = !history.flags.is_empty();
            let changes: Vec<crate::evidence::ObservedChange> = history
                .changes
                .into_iter()
                .map(crate::evidence::ObservedChange::from_history)
                .collect();
            let produced = produced_artifact(slug, &changes) || repo_produced;
            let collector = evidence
                .lock()
                .map(|collector| collector.clone())
                .map_err(|_| "workflow evidence lock poisoned".to_string());
            // The receipt copies the queue row's wait and mirrors its
            // timings back into that row, still `running` here (the pump
            // ends it after `execute` returns).
            match collector.and_then(|mut collector| {
                collector.timings_mut().finalize_diff_ms = Some(finalize_diff_ms);
                collector.finish_mirrored(vault, status, changes)
            }) {
                Ok(receipt) => (Some(receipt), None, produced),
                Err(error) => {
                    let message = format!("evidence receipt failed: {error}");
                    eprintln!("[workflow {slug}] {message}");
                    (None, Some(message), produced)
                }
            }
        }
        Err(error) => {
            let message = format!("history finalization failed: {error}");
            eprintln!("[workflow {slug}] {message}");
            // Without the diff there's no proof anything landed, so the
            // run falls on the side that asks for a look.
            (None, Some(message), false)
        }
    };
    // A run that may have acted outside its files (or whose receipt, and so
    // its record of that, is missing) never retries on its own (plan
    // completion L2): an unattended second run could repeat a completed
    // external action. The run review offers a guarded retry instead.
    let outside_effects_unsafe = match &receipt {
        Some(receipt) => receipt
            .outside_effects
            .as_ref()
            .is_some_and(|effects| !effects.allows_automatic_retry()),
        None => true,
    };
    // A guarded retry's guard ends with its run.
    crate::run_effects::remove_guard(run_id);
    // The lease is released: the retry, the streak and the notification are
    // decided by `settle_outcome` (AppHandle-free, so tests drive it). The
    // streak is written before the `done` event, so a panel refreshing on it
    // sees the streak card.
    let settled = settle_outcome(
        vault,
        &SettleInput {
            workflow,
            slug,
            run_id,
            ctx,
            status,
            error,
            output,
            exit_kind,
            failure,
            started_at,
            history_finished,
            history_flagged,
            outside_effects_unsafe,
            produced,
            finished_at: Utc::now(),
        },
    );
    if let Some(enqueued) = settled.enqueued {
        if let (Some(kind), Ok(Some(row))) =
            (enqueued.event_kind(), run_queue::get(vault, enqueued.id()))
        {
            crate::run_pump::emit(app, kind, run_queue::to_item(&row, None));
        }
    }
    // Report what is actually saved, including a preserved/newer occurrence.
    // A failed read is unknown (null), never a newly computed schedule.
    let next = match vault_workflows::read_full(vault, slug) {
        Ok((saved, _, _)) => saved.next_run_at,
        Err(error) => {
            eprintln!("[workflow {slug}] saved next run time is unknown: {error}");
            None
        }
    };
    emit(
        app,
        json!({
            "type": "done",
            "slug": slug,
            // The run and queue row this `done` ends (Paperclip PR 15b), so
            // the dock's activity map drops exactly this run.
            "runId": run_id,
            "queueId": ctx.queue_id,
            "status": status,
            "output": output,
            "durationSec": duration_sec,
            "error": error,
            "nextRunAt": next,
            "receipt": &receipt,
            "evidenceError": &evidence_error,
        }),
    );
    if let Some(input) = settled.notification {
        let _ = notifications::create(app, vault, input);
    }
    FinalizeOutcome {
        receipt,
        evidence_error,
    }
}

/// What [`settle_outcome`] needs from a finished run, after `finish_run`.
pub(crate) struct SettleInput<'a> {
    pub(crate) workflow: &'a Workflow,
    pub(crate) slug: &'a str,
    /// The run's automated-history id (its run-log run id).
    pub(crate) run_id: &'a str,
    pub(crate) ctx: &'a RunContext,
    /// The run-log status: `success` | `error` | `cancelled`.
    pub(crate) status: &'a str,
    pub(crate) error: Option<&'a str>,
    pub(crate) output: &'a str,
    /// `Some(EXIT_KIND_BUDGET)` when a usage budget ended the run (PR 8b).
    pub(crate) exit_kind: Option<&'static str>,
    pub(crate) failure: Option<FailureClass>,
    /// Exactly the `at` passed to `mark_started`; None before it.
    pub(crate) started_at: Option<DateTime<Utc>>,
    /// `finish_run` returned `Ok` for this run.
    pub(crate) history_finished: bool,
    /// The finished record carries PR 5 protected-change flags.
    pub(crate) history_flagged: bool,
    /// The run may have acted outside its files or ran calls Hollow can't
    /// see into (`RetryInput::outside_effects_unsafe`).
    pub(crate) outside_effects_unsafe: bool,
    /// The run's diff shows something a person can open.
    pub(crate) produced: bool,
    pub(crate) finished_at: DateTime<Utc>,
}

/// What a finished run settled to.
#[derive(Debug)]
pub(crate) struct Settled {
    /// The retry that was queued, if any (read by tests; `finalize` needs
    /// only the queue answer and the notification).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) retry: Option<RetryPlan>,
    /// The queue's answer for that retry row (for the `run-queue-event`).
    pub(crate) enqueued: Option<Enqueued>,
    /// The failure streak this run opened, if any.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) streak: Option<workflow_hygiene::FailureStreak>,
    /// The one notification to record for the run (None: none).
    pub(crate) notification: Option<notifications::NotificationInput>,
}

/// Settle a finished run once its lease is released: queue its retry, record
/// its streak outcome and plan its notification. AppHandle-free; `finalize`
/// emits the queue event and records the notification.
///
/// - Paperclip PR 9b: a transient failure of an unattended run queues ONE
///   retry a minute out (after `finish_run`, so a flagged or broken record
///   is known). While it is queued this run's streak outcome is deferred:
///   the retry's own outcome counts it (the run log keeps this row), or
///   `workflow_retry::resolve_skipped_retry` does if the retry never runs;
///   and one quiet "retrying once" notice replaces the failure banner. A
///   retry that was itself stopped counts the failure it was deferring now.
///   When the retry row cannot be queued, `resolve_skipped_retry` reports
///   the failure at once and nothing else is recorded.
/// - A run a budget ended (PR 8b) is not a failure of the workflow: it
///   neither counts toward nor closes a streak, and gets one quiet warning
///   instead of the "failed" banner (the budget pause is the actionable
///   notice).
/// - Otherwise the streak opens (and is returned) only on the failure that
///   crosses the threshold; a success closes the subject; a cancel does
///   nothing. The streak's queue write is attributed through `with_file`,
///   so a run that started meanwhile is never flagged for it.
pub(crate) fn settle_outcome(vault: &Path, input: &SettleInput<'_>) -> Settled {
    let SettleInput {
        workflow,
        slug,
        run_id,
        ctx,
        status,
        error,
        output,
        exit_kind,
        failure,
        started_at,
        history_finished,
        history_flagged,
        outside_effects_unsafe,
        produced,
        finished_at,
    } = *input;
    let trigger_reason = ctx.trigger_reason();
    let plan = match (failure, started_at) {
        (Some(class), Some(started_at)) if status == "error" && exit_kind.is_none() => {
            workflow_retry::retry_plan(&RetryInput {
                slug,
                title: &workflow.title,
                history_run_id: run_id,
                source: ctx.source,
                attempt: ctx.attempt,
                queue_id: ctx.queue_id,
                priority: ctx.priority,
                class,
                history_finished,
                history_flagged,
                outside_effects_unsafe,
                started_at,
                finished_at,
                error: error.unwrap_or(""),
                trigger_reason: trigger_reason.as_deref(),
                prior_last_run_at: workflow.last_run_at.as_deref(),
            })
        }
        _ => None,
    };
    let mut retry_unqueued = false;
    let mut enqueued = None;
    let retry = match plan {
        Some(plan) => match enqueue_retry(vault, slug, &workflow.title, &plan) {
            Ok(row) => {
                enqueued = Some(row);
                run_queue::wake();
                Some(plan)
            }
            Err(enqueue_error) => {
                eprintln!("[workflow {slug}] could not queue its retry: {enqueue_error}");
                retry_unqueued = true;
                let payload = serde_json::to_value(&plan.payload).unwrap_or(Value::Null);
                workflow_retry::resolve_skipped_retry(
                    vault,
                    slug,
                    &payload,
                    RetrySkipReason::EnqueueFailed,
                );
                None
            }
        },
        None => None,
    };
    let streak = if exit_kind.is_some() || retry.is_some() || retry_unqueued {
        None
    } else if ctx.source == RunSource::Retry && status == "cancelled" {
        workflow_hygiene::record_outcome(vault, slug, &workflow.title, "error", run_id)
    } else {
        workflow_hygiene::record_outcome(vault, slug, &workflow.title, status, run_id)
    };

    let note = short_note(output, None);
    let policy = NotifyPolicy::parse(workflow.notify.as_deref());
    // The persistent row is written whenever the policy allows one, so the
    // ledger stays complete; only `notify_native` separates "here when you
    // want it" from "look at this now".
    let notification = if exit_kind.is_some() {
        budget_stop_notification(workflow, slug, error)
    } else if let Some(plan) = &retry {
        // The retry's own outcome is the one that interrupts.
        workflow_retry::retrying_notification(&workflow.title, slug, plan, policy)
    } else if retry_unqueued {
        // `resolve_skipped_retry` already reported the failure.
        None
    } else {
        workflow_hygiene::finalize_notification(&NotificationPlanInput {
            // A stopped retry whose deferred failure opened a streak still
            // gets the streak's notification.
            status: if streak.is_some() { "error" } else { status },
            produced,
            policy,
            streak_opened: streak.as_ref(),
            title: &workflow.title,
            slug,
            note: &note,
            error,
        })
    };
    Settled {
        retry,
        enqueued,
        streak,
        notification,
    }
}

/// Queue a failed run's one retry (Paperclip PR 9b): a `retry` row for
/// `slug`, not claimable before the plan's `not_before`, keyed
/// `retry:{slug}:{history_run_id}` (a second finalize adds nothing), keeping
/// the failed row's priority, with the plan's payload for the claim.
pub(crate) fn enqueue_retry(
    vault: &Path,
    slug: &str,
    title: &str,
    plan: &RetryPlan,
) -> Result<run_queue::Enqueued, String> {
    let payload = serde_json::to_value(&plan.payload).map_err(|e| e.to_string())?;
    let mut row = NewRow::new(RunSource::Retry, Some(slug), title, &plan.reason)
        .with_payload(payload)
        .with_origin(crate::work_origin::Request::retry(
            &plan.payload.failed_run_id,
            plan.retry_of,
        ))
        .with_key(plan.idempotency_key.clone());
    row.priority = plan.priority;
    row.retry_of = plan.retry_of;
    row.not_before = Some(plan.not_before);
    run_queue::enqueue(vault, row, Utc::now())
}

/// The notification of a run a usage budget ended (Paperclip PR 8b): a
/// warning, in-app only, whose body is the stop notice. Not native: the
/// budget pause raised its own native warning. It follows the workflow's
/// `notify` policy like any failed run (`never` sends none; the pause's own
/// notification stays the actionable notice).
fn budget_stop_notification(
    workflow: &Workflow,
    slug: &str,
    error: Option<&str>,
) -> Option<notifications::NotificationInput> {
    if !workflow_hygiene::NotifyPolicy::parse(workflow.notify.as_deref()).allows_run("error") {
        return None;
    }
    Some(notifications::NotificationInput {
        title: format!("{} stopped at a budget limit", workflow.title),
        body: error
            .unwrap_or(crate::budgets::BUDGET_STOP_PREFIX)
            .to_string(),
        kind: "warning".into(),
        source: Some(slug.to_string()),
        action_label: Some("Review".into()),
        action_url: Some(crate::budgets::PAUSES_ACTION.into()),
        notify_native: false,
    })
}

/// Whether a run's repository changes (Paperclip PR 16) give a person
/// something to open: a create or modify that isn't back to its committed
/// content, or rows past the listed ones. Deletes don't count, as in
/// [`produced_artifact`].
fn repo_produced(repo: &[crate::repo_diff::RepoChanges]) -> bool {
    repo.iter().any(|repo| {
        repo.total_files > repo.files.len()
            || repo
                .files
                .iter()
                .any(|file| file.change_type != "delete" && file.status != "clean")
    })
}

/// Whether a run's filesystem diff shows it wrote something a person can
/// open. Deletes don't count, and neither does the workflow's own
/// `workflows/{slug}.md` — [`finalize`] stamps the status and run log into
/// that file before the after-snapshot, so it changes on every single run.
/// A workflow that authors a *different* workflow file did produce
/// something, so the exclusion is that one path, not the folder.
fn produced_artifact(slug: &str, changes: &[crate::evidence::ObservedChange]) -> bool {
    let own = format!("workflows/{slug}.md");
    changes
        .iter()
        .any(|change| change.change_type != "delete" && change.path != own)
}

fn short_note(output: &str, error: Option<&str>) -> String {
    if let Some(err) = error {
        return truncate(err.trim(), 240);
    }
    let line = output
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    truncate(line, 240)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

/// Truncate from the FRONT, keeping the last `n` chars — for stderr tails,
/// where the useful part (the actual error) prints last.
fn tail_truncate(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        let tail: String = s.chars().skip(count - n).collect();
        format!("…{tail}")
    }
}

/// What the run queue's claim should do with a cron occurrence whose due time
/// has been compared against now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MisfireDecision {
    /// `next_run_at` is still in the future — leave it alone.
    NotDue,
    /// Due, and within the grace window — fire the run.
    Run,
    /// Due, but missed by more than the grace window — skip it and advance
    /// `next_run_at`, so a 6pm "close my day" can't fire at noon the next
    /// launch. This says nothing about WHY it was missed: the caller decides
    /// whether Hollow was closed (the misfire note) or the run waited on the
    /// vault lease past its grace (a contention note naming the holder).
    Skip,
}

/// Pure stale-vs-fresh decision for a due schedule. `grace` is how far past
/// `next_run_at` a run may still fire as a late catch-up (boundary inclusive).
pub fn decide_misfire(
    now: DateTime<Utc>,
    next_run_at: DateTime<Utc>,
    grace: chrono::Duration,
) -> MisfireDecision {
    if next_run_at > now {
        return MisfireDecision::NotDue;
    }
    if now - next_run_at <= grace {
        MisfireDecision::Run
    } else {
        MisfireDecision::Skip
    }
}

fn emit(app: &impl crate::core_host::CoreHost, payload: Value) {
    let _ = app.emit(EVENT, payload);
}

/// The `activity` workflow-event a running run's ticker emits every 15 s
/// (Paperclip PR 15b), keyed by slug and run id (the `started` event's), with
/// its queue row: when the agent last printed anything (its spawn before the
/// first line) and the progress phase of its latest tool call. Pure.
fn workflow_activity_event(
    slug: &str,
    run_id: &str,
    queue_id: Option<i64>,
    snapshot: &crate::run_activity::ActivitySnapshot,
) -> Value {
    json!({
        "type": "activity",
        "slug": slug,
        "runId": run_id,
        "queueId": queue_id,
        "lastEventAt": snapshot.last_event_at,
        "progress": snapshot.progress,
    })
}

/// Start the background enqueuer. Every 60s it calls [`enqueue_due`], which
/// puts due cron occurrences and trigger fires on the run queue; the pump
/// (`run_pump`) runs them. Started only when this process owns the vault's
/// run queue. Idempotent: calling this twice is a no-op after the first call
/// wins.
pub fn start_scheduler(app: impl Into<Core>, vault: PathBuf) {
    let app = app.into();
    {
        let mut started = match SCHEDULER_STARTED.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if *started {
            return;
        }
        *started = true;
    }

    // `tauri::async_runtime::spawn` instead of bare `tokio::spawn`: this is
    // kicked off from inside Tauri's `.setup` callback, which runs before the
    // tokio runtime is current on the main thread.
    tauri::async_runtime::spawn(async move {
        // The startup repairs write workflow files, under the bookkeeping
        // lease like every other scheduler write, so they run on a blocking
        // worker.
        let startup_vault = vault.clone();
        let _ =
            tauri::async_runtime::spawn_blocking(move || startup_bookkeeping(&startup_vault)).await;

        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            // The pass reads the vault, heals under the bookkeeping lease and
            // writes the queue, so it runs on a blocking worker.
            let tick_vault = vault.clone();
            let outcome = match tauri::async_runtime::spawn_blocking(move || {
                enqueue_due(&tick_vault, Utc::now())
            })
            .await
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    eprintln!("[scheduler] enqueue pass failed: {error}");
                    continue;
                }
            };
            let enqueued = !outcome.changed.is_empty();
            for (kind, item) in outcome.changed {
                crate::run_pump::emit(&app, kind, item);
            }
            if enqueued {
                run_queue::wake();
            }
        }
    });
}

/// The scheduler's startup repair, under its bookkeeping lease: heal any
/// nextRunAt that's missing on a scheduled workflow, as happens on the first
/// launch after a workflow file is added by hand in Obsidian. An unparseable
/// schedule has nothing to heal to, so it is left untouched rather than
/// rewritten. Returns false, writing nothing, when another writer (a headless
/// `hollow notion-sync`, say) holds the vault; the per-tick heal in
/// [`enqueue_due`] still fixes missing next runs once the vault is free.
fn startup_bookkeeping(vault: &Path) -> bool {
    let guard = match automated_history::scheduler_write_lease(vault) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("[scheduler] startup repairs wait for the vault: {error}");
            return false;
        }
    };
    for w in crate::runs_on::owned_here(vault, vault_workflows::list(vault)) {
        if w.schedule == "manual" {
            continue;
        }
        if w.next_run_at.is_none()
            && vault_workflows::next_run_after(&w.schedule, Utc::now()).is_some()
        {
            let _ = vault_workflows::refresh_next_run(vault, &w.slug);
        }
    }
    drop(guard);
    true
}

/// The misfire grace window for `grace_min` minutes. An absurd hand-edited
/// value saturates to "never stale" rather than panicking: chrono's
/// `Duration::minutes` panics out of range, and the release profile aborts
/// on panic, so one bad `misfire_grace_min` would crash Hollow every tick.
pub(crate) fn grace_window(grace_min: u64) -> chrono::Duration {
    i64::try_from(grace_min)
        .ok()
        .and_then(chrono::Duration::try_minutes)
        .unwrap_or(chrono::Duration::MAX)
}

/// One warning notification for every run skipped for the same reason in a
/// burst of claims (the pump aggregates consecutive skips).
pub(crate) fn notify_skipped(
    app: &impl crate::core_host::CoreHost,
    vault: &Path,
    titles: &[String],
    why: &str,
) {
    let n = titles.len();
    let _ = notifications::create(
        app,
        vault,
        notifications::NotificationInput {
            title: format!(
                "Skipped {n} scheduled run{} {why}",
                if n == 1 { "" } else { "s" }
            ),
            body: titles.join(", "),
            kind: "warning".to_string(),
            source: Some("workflow".to_string()),
            action_label: None,
            action_url: None,
            notify_native: true,
        },
    );
}

/// Per-workflow trigger watermarks, as `trigger_scanner` persists them.
type WatermarkState = HashMap<String, HashMap<String, TriggerWatermark>>;

/// Unparseable schedules already reported, keyed by slug and schedule, so a
/// broken cron is logged once per process (and again only if it changes).
static INVALID_SCHEDULES_LOGGED: Lazy<Mutex<HashSet<String>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));

fn log_invalid_schedule_once(slug: &str, schedule: &str) {
    let first = INVALID_SCHEDULES_LOGGED
        .lock()
        .map(|mut seen| seen.insert(format!("{slug}\n{schedule}")))
        .unwrap_or(true);
    if first {
        eprintln!(
            "[scheduler] workflow {slug} has an unparseable schedule {schedule:?}; it will not run until the schedule is fixed"
        );
    }
}

/// What one enqueue pass did, for the scheduler loop to announce.
pub(crate) struct EnqueueOutcome {
    /// Rows this pass inserted or coalesced into, with their event kind.
    pub(crate) changed: Vec<(&'static str, RunQueueItem)>,
    /// Queued rows the current lease holder blocks, touched once this pass.
    pub(crate) touched: usize,
}

fn note_enqueued(vault: &Path, outcome: &mut EnqueueOutcome, enqueued: Enqueued) {
    if let Some(kind) = enqueued.event_kind() {
        if let Ok(Some(row)) = run_queue::get(vault, enqueued.id()) {
            outcome.changed.push((kind, run_queue::to_item(&row, None)));
        }
    }
}

/// Move a completed occurrence's stale `next_run_at` on (see [`enqueue_due`])
/// under the scheduler's bookkeeping lease, like the heal; while another
/// writer holds the vault it waits for a later pass.
fn advance_completed(vault: &Path, slug: &str) {
    match automated_history::scheduler_write_lease(vault) {
        Ok(guard) => {
            let _ = vault_workflows::refresh_next_run(vault, slug);
            drop(guard);
        }
        Err(BeginRunError::Busy(_)) => {}
        Err(BeginRunError::Failed(error)) => {
            eprintln!("[scheduler] bookkeeping lease: {error}");
        }
    }
}

/// One enqueue pass at `now`. Sync and AppHandle-free; it never admits,
/// executes or skips anything (the pump does, one row at a time).
///
/// 1. **Heal** a missing `next_run_at` on a valid scheduled workflow, under
///    the scheduler's bookkeeping lease; while another writer holds the vault
///    the heal waits for a later pass.
/// 2. **Cron**: one row per due `next_run_at` (key `cron:{slug}:{next_run_at}`,
///    payload `dueAt`), even past its grace: the claim labels a stale one. An
///    unparseable schedule is logged once and never enqueued. An occurrence
///    a finished run already consumed (`run_queue::occurrence_completed`)
///    is never queued again; its stale `next_run_at` is moved on.
/// 3. **Triggers**: each trigger workflow is evaluated; a fire becomes a row
///    carrying its fire facts (key `trigger_evaluator::fired_key`), and its
///    advanced watermark is committed once that enqueue succeeded (at least
///    once: a failed enqueue keeps the prior watermark and re-fires next
///    pass). A non-firing workflow's watermark advances as before. A merely
///    QUEUED row does not stop evaluation, so a re-fire coalesces into it.
///    The watermarks are committed after every enqueue, never while holding
///    the queue's lock.
/// 4. **Touch**: when someone holds the lease, every queued row it blocks
///    counts one wait (`run_queue::touch_waiting`), so a wait episode never
///    looks abandoned while Hollow is open.
///
/// A workflow that is running here, or has a claimed or running row, is
/// skipped for cron and triggers alike: evaluation resumes once it ends, and
/// its own writes are not evaluated mid-run. Only owned/compatible occurrences
/// are consumed by a proven run; legacy Unknown retains its previous behavior.
pub(crate) fn enqueue_due(vault: &Path, now: DateTime<Utc>) -> EnqueueOutcome {
    use crate::trigger_evaluator::{fired_key, parse_rfc3339, TriggerEvaluator, TriggerSpec};

    let mut outcome = EnqueueOutcome {
        changed: Vec::new(),
        touched: 0,
    };
    // M4: only the automations whose owner Mac is this one (`runs_on`), so
    // a synced `workflows/` folder is never queued, healed or ended by two
    // Macs.
    let workflows = crate::runs_on::owned_here(vault, vault_workflows::list(vault));

    // Stopped for good, or past its end and waiting for the pass below to
    // write it: a polled trigger neither fires nor enqueues it (its watermark
    // still advances, so reviving it does not replay old changes). A cron
    // occurrence is judged in step 2.
    let ended = |w: &Workflow| w.ended_note.is_some() || vault_workflows::end_passed(w, now);

    // (1) Heal.
    let heal: Vec<&Workflow> = workflows
        .iter()
        .filter(|w| {
            w.schedule != "manual"
                && vault_workflows::validate_schedule(&w.schedule).is_ok()
                && w.next_run_at.as_deref().and_then(parse_rfc3339).is_none()
                && vault_workflows::next_run_after(&w.schedule, now).is_some()
        })
        .collect();
    if !heal.is_empty() {
        match automated_history::scheduler_write_lease(vault) {
            Ok(guard) => {
                for w in &heal {
                    let _ = vault_workflows::refresh_next_run(vault, &w.slug);
                }
                drop(guard);
            }
            Err(BeginRunError::Busy(_)) => {}
            Err(BeginRunError::Failed(error)) => {
                eprintln!("[scheduler] bookkeeping lease: {error}");
            }
        }
    }

    let active: HashSet<String> = match run_queue::live_rows(vault) {
        Ok(rows) => rows
            .into_iter()
            .filter(|row| row.status != RowStatus::Queued && row.source.is_workflow())
            .filter_map(|row| row.source_id)
            .collect(),
        Err(error) => {
            eprintln!("[scheduler] the run queue is unavailable: {error}");
            return outcome;
        }
    };
    let busy = |w: &Workflow| is_running(&w.slug) || active.contains(&w.slug);

    // (2) Cron.
    for w in &workflows {
        if w.schedule == "manual" || busy(w) || w.ended_note.is_some() {
            continue;
        }
        // Checked before anything reads `next_run_at`: a schedule edited into
        // something the cron crate rejects must not keep firing its last
        // stored occurrence.
        if vault_workflows::validate_schedule(&w.schedule).is_err() {
            log_invalid_schedule_once(&w.slug, &w.schedule);
            continue;
        }
        let Some(due) = w.next_run_at.as_deref() else {
            continue;
        };
        if parse_rfc3339(due).is_none_or(|due_at| due_at > now) {
            continue;
        }
        // Past its end: only an occurrence that came due before the end still
        // runs (queued before step 2b ends it); a later one never does.
        if vault_workflows::end_passed(w, now)
            && w.ends
                .as_deref()
                .and_then(|raw| vault_workflows::parse_end(raw).ok())
                .zip(parse_rfc3339(due))
                .is_some_and(|(end, due_at)| due_at > end)
        {
            continue;
        }
        // A run of this occurrence already finished, but its workflow-file
        // write (`finalize_run`: the run log and `next_run_at`) failed, so the
        // slot still reads as due (merged plan slice 3, B·F10). It never runs
        // again: `next_run_at` is moved on instead, as that write would have.
        // A queue that cannot answer queues nothing this pass.
        match run_queue::occurrence_completed(vault, &w.slug, due) {
            Ok(false) => {}
            Ok(true) => {
                advance_completed(vault, &w.slug);
                continue;
            }
            Err(error) => {
                eprintln!("[scheduler] occurrence check for {}: {error}", w.slug);
                continue;
            }
        }
        let origin = crate::work_origin::Request::scheduled_workflow();
        let row = NewRow::new(RunSource::Cron, Some(&w.slug), &w.title, "scheduled")
            .with_origin(origin)
            .with_payload(run_queue::cron_payload(due))
            .with_key(format!("cron:{}:{due}", w.slug));
        match run_queue::enqueue(vault, row, now) {
            Ok(enqueued) => note_enqueued(vault, &mut outcome, enqueued),
            Err(error) => eprintln!("[scheduler] could not queue workflow {}: {error}", w.slug),
        }
    }

    // (2b) End (G2): an automation whose `ends` has passed is set to `manual`
    // with a note, after step 2 so an occurrence that came due before the end
    // (inside its misfire grace) is still queued first. Under the bookkeeping lease like the heal;
    // while another writer holds the vault the automation is simply not
    // enqueued this pass (`end_passed` below) and ends on a later one.
    let due_to_end: Vec<String> = workflows
        .iter()
        .filter(|w| vault_workflows::end_passed(w, now))
        .map(|w| w.slug.clone())
        .collect();
    if !due_to_end.is_empty() {
        match automated_history::scheduler_write_lease(vault) {
            Ok(guard) => {
                for slug in &due_to_end {
                    if let Err(error) = vault_workflows::end_if_due(vault, slug, now) {
                        eprintln!("[scheduler] could not end {slug}: {error}");
                    }
                }
                drop(guard);
            }
            Err(BeginRunError::Busy(_)) => {}
            Err(BeginRunError::Failed(error)) => {
                eprintln!("[scheduler] bookkeeping lease: {error}");
            }
        }
    }

    // (3) Triggers. Independent of `schedule`: a workflow may be `manual` and
    // purely trigger-fired.
    // Event triggers (F2) are `event_triggers`' business, not this pass's.
    let polled = |w: &Workflow| !TriggerSpec::parse_polled(&w.triggers).is_empty();
    if workflows.iter().any(polled) {
        let task_ids = crate::trigger_scanner::task_trigger_ids(vault);
        let facts = crate::trigger_scanner::live_facts(vault, now, task_ids);
        let priors = crate::trigger_scanner::load_state(vault);
        let mut updates = WatermarkState::new();
        for w in workflows.iter().filter(|w| polled(w)) {
            if busy(w) {
                continue;
            }
            let specs = TriggerSpec::parse_polled(&w.triggers);
            if specs.is_empty() {
                continue;
            }
            let prior = priors.get(&w.slug).cloned().unwrap_or_default();
            let evaluated = TriggerEvaluator::evaluate_all_detailed(&specs, &facts, &prior);
            if !evaluated.fire || ended(w) {
                updates.insert(w.slug.clone(), evaluated.next);
                continue;
            }
            let origin = crate::work_origin::Request::scheduled_workflow();
            let key = fired_key(&w.slug, &evaluated);
            let row = NewRow::new(
                RunSource::Trigger,
                Some(&w.slug),
                &w.title,
                &evaluated.reason,
            )
            .with_origin(origin)
            .with_payload(run_queue::trigger_payload(&evaluated.fired))
            .with_key(key);
            match run_queue::enqueue(vault, row, now) {
                Ok(enqueued) => {
                    note_enqueued(vault, &mut outcome, enqueued);
                    updates.insert(w.slug.clone(), evaluated.next);
                }
                Err(error) => eprintln!(
                    "[scheduler] could not queue a trigger fire for workflow {}: {error}; it fires again next pass",
                    w.slug
                ),
            }
        }
        commit_trigger_updates(vault, updates);
    }

    // (4) Touch the rows the current holder blocks.
    if let Some(holder) = automated_history::lease_holder(vault) {
        match run_queue::touch_waiting(vault, &holder, now) {
            Ok(touched) => outcome.touched = touched,
            Err(error) => eprintln!("[scheduler] could not refresh waiting rows: {error}"),
        }
    }
    outcome
}

/// Write one skipped row's run-log entry (and, when `advance`, move
/// `next_run_at` past `now` via `skip_scheduled_run`) under the scheduler's
/// bookkeeping lease (`lease` is `automated_history::scheduler_write_lease`
/// in production), then drop it. Contract PR 7's "never write
/// `workflows/*.md` while another holder has the lease": `Busy` means nothing
/// was written and the caller re-queues the row; `Failed` covers the lease
/// itself or the write. The status and run-log writers never re-stamp the
/// workflow's permission record, so no permission write guard is needed.
pub(crate) fn skip_bookkeeping(
    vault: &Path,
    slug: &str,
    entry: RunLogEntry,
    advance: bool,
    now: DateTime<Utc>,
    lease: &dyn Fn(&Path) -> Result<BackgroundWriteLease, BeginRunError>,
) -> Result<(), BeginRunError> {
    let guard = lease(vault)?;
    let written = if advance {
        vault_workflows::skip_scheduled_run(vault, slug, entry, now)
    } else {
        vault_workflows::append_run_log(vault, slug, entry)
    };
    drop(guard);
    written.map(|_| ()).map_err(BeginRunError::Failed)
}

/// [`skip_bookkeeping`] for an idle skip (Paperclip PR 9b): under the same
/// manifest-free bookkeeping lease (never a history run, so a streak of quiet
/// skips never prunes the manifest the quiet check reads),
/// `vault_workflows::skip_idle_run` advances `next_run_at` and writes or
/// coalesces the row, provided `due` is still the workflow's next run.
pub(crate) fn idle_skip_bookkeeping(
    vault: &Path,
    slug: &str,
    skip: &vault_workflows::IdleSkip,
    due: Option<&str>,
    now: DateTime<Utc>,
    lease: &dyn Fn(&Path) -> Result<BackgroundWriteLease, BeginRunError>,
) -> Result<vault_workflows::IdleSkipWrite, BeginRunError> {
    let guard = lease(vault)?;
    let written = vault_workflows::skip_idle_run(vault, slug, skip, due, now);
    drop(guard);
    written.map_err(BeginRunError::Failed)
}

#[cfg(test)]
#[path = "workflows_tests.rs"]
mod tests;

/// Agent effectiveness F2 (wave 1 · lane C): a workflow run's recall and
/// connector selection read its request (the prompt after variable
/// substitution) and the notes it names, not the prompt expanded with
/// evidence, profile, recall and workspace context.
#[cfg(test)]
mod f2_request_tests {
    use super::*;
    use crate::turn_request::fixtures::{self, ATLAS, BOREALIS, OUTLOOK};
    use crate::turn_request::TurnRequest;

    const ASK: &str = "Draft the Atlas weekly report";

    fn context(vault: &Path, ask: &str, retrieved: &str, profile: &str) -> PromptContext {
        compose_run_context(
            vault,
            &TurnRequest::from_text(vault, ask),
            retrieved,
            profile,
            Some("<workspace-context>\nWorkspace brief.\n</workspace-context>\n\n"),
        )
    }

    fn statements(items: &[crate::vault_learning::RecallItem]) -> Vec<String> {
        let mut out: Vec<String> = items.iter().map(|i| i.statement.clone()).collect();
        out.sort();
        out
    }

    #[test]
    fn unrelated_profile_prose_changes_neither_recall_nor_connectors() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let plain = context(v, ASK, "", fixtures::PLAIN_PROFILE);
        let noisy = context(v, ASK, "", fixtures::NOISY_PROFILE);
        assert_eq!(
            statements(&plain.recall_items),
            vec![OUTLOOK.to_string(), ATLAS.to_string()]
        );
        assert_eq!(
            statements(&noisy.recall_items),
            statements(&plain.recall_items)
        );
        assert_eq!(
            fixtures::local_selection(&noisy.connector_query.local),
            fixtures::local_selection(&plain.connector_query.local),
            "profile prose chose connectors"
        );
        assert!(fixtures::local_selection(&noisy.connector_query.local).is_empty());
        // The context still reaches the agent, in its order.
        assert!(noisy.prompt.starts_with("<workspace-context>"));
        assert!(noisy.prompt.contains(fixtures::NOISY_PROFILE));
        assert!(noisy.prompt.ends_with(ASK));
    }

    #[test]
    fn recall_reads_the_request_not_the_retrieved_evidence() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let evidence = "<retrieved-evidence>\nBorealis budget reviews and finance notes.\n</retrieved-evidence>\n\n";
        let run = context(v, ASK, evidence, "");
        assert!(!statements(&run.recall_items).contains(&BOREALIS.to_string()));
        assert!(statements(&run.recall_items).contains(&ATLAS.to_string()));
        assert_eq!(
            run.connector_query,
            crate::context_rank::ConnectorQuery::plain(ASK)
        );
        // A note the request names is a subject path: recalled by path even
        // when no word of the learning is in the request.
        let named = context(v, "Tidy projects/active/Borealis.md", "", "");
        assert!(statements(&named.recall_items).contains(&BOREALIS.to_string()));
    }

    #[test]
    fn a_long_profile_cannot_hide_an_explicit_connector_request() {
        let dir = fixtures::learned_vault();
        let v = dir.path();
        let profile = "I like tidy notes and calm mornings. ".repeat(200);
        let run = context(v, "Summarize yesterday's Plaud call", "", &profile);
        assert_eq!(
            fixtures::local_selection(&run.connector_query.local),
            vec!["plaud"]
        );
        // Jev scores the first 2,000 characters of the query (`jev.rs`).
        let head: String = run.connector_query.request.chars().take(2000).collect();
        assert!(head.contains("Plaud"), "{head}");
    }

    #[test]
    fn prepare_spawn_selects_connectors_on_the_request() {
        let whole = include_str!("workflows.rs");
        let start = whole.find(concat!("async fn ", "prepare_spawn(")).unwrap();
        let body = &whole[start..start + whole[start..].find("\n}\n").unwrap()];
        assert!(body.contains(concat!("compose_run", "_context(")));
        assert!(!body.contains(concat!("vault_learning::", "recall(")));
        // Both routes' lifecycle decisions reach the run's manifest (F7).
        assert!(body.contains(concat!(
            "manifest.add_lifecycle(retrieved.history, ",
            "retrieved.lifecycle);"
        )));
        assert!(body.contains(concat!(
            "manifest.add_lifecycle(recall_history, ",
            "recall_lifecycle);"
        )));
        // So does whether recall ranked by meaning (OF26 review).
        assert!(body.contains(concat!(
            "manifest.recall_meaning = ",
            "recall_meaning.recorded();"
        )));
        assert!(crate::source_pins::contains(
            body,
            concat!(
                "connectors::attach(agent, &permission_mode, true, ",
                "&connector_query, &deadline, &jev, None)"
            )
        ));
        // F8 (O2): beside retrieval, under one deadline, recorded after both.
        let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(
            at(concat!("PrepDeadline::", "start()"))
                < at(concat!("prep_deadline::", "concurrently("))
        );
        assert!(
            at(concat!("prep_deadline::", "concurrently("))
                < at(concat!("connectors::", "attach("))
        );
        assert!(
            at(concat!("set_connectors(", "attached_library.selection)"))
                < at(concat!(
                    "set_deadline(",
                    "&deadline, attached_library.auth_cut)"
                ))
        );
        let heartbeat = include_str!("heartbeat.rs");
        // The beat's recall reads its own request, always current-state (F7).
        assert!(heartbeat.contains(concat!(
            "vault_learning::recall_with(vault, ",
            "&core, &[], None)"
        )));
    }
}
