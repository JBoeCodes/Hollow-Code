//! The run-queue pump (Paperclip PR 7): the ONE task that claims rows from
//! the durable run queue ([`crate::run_queue`]) and admits them, one run at a
//! time, through the full-vault lease. It is the only automation caller of
//! `automated_history::try_begin_owned`: workflows via
//! `workflows::admit_runnable`, the heartbeat via
//! `heartbeat::gate_scheduled_beat`, and since PR 7b chat turns via
//! `chat::admit_turn` (chat is no longer a direct lease holder). The
//! justified direct lease holders that stay outside it are interactive
//! ingest (with its own Cancel), the heartbeat's Run now diagnostic, and
//! history restore; tracker sync, the scheduler's bookkeeping and history
//! clears take the lease file without a run.
//!
//! **Chat rows (PR 7b).** A chat row is claimed first (priority 0, exempt
//! from the gap) but never preempts a running automation: it waits for the
//! lease like any row. Before claiming one the step marks it in
//! `chat_rows::CHAT_ROW_ACTIVE` ([`chat::ChatRowGuard`]), so a second send is
//! refused and never repairs a row the pump is admitting. A chat row that
//! ends without its turn starting (cancelled, failed behind a stale lease,
//! recovered because its folder vanished) returns its `chat-event`s in the
//! step result; [`drain`] emits them. An executed turn's row is ended
//! BEFORE its `done` is emitted, and the gate is released between the two.
//!
//! **One step.** [`step`] is sync and AppHandle-free, so tests drive the real
//! order: retry any terminal write that failed earlier; read the head row
//! (`next_ready`, honoring `not_before` and the automation gap); if someone
//! holds the lease, start the queued rows' wait episodes and return
//! [`PumpStep::Busy`] (a stale or unknown lease fails queued Run now rows at
//! once, with the restart hint; a lease whose owner process EXITED reads as
//! free since Paperclip PR 18, so the head row is claimed and its admission
//! reclaims the lease); otherwise compare-and-swap claim the head,
//! decide it ([`claim_verdict`]) and act: skip (its run-log row written only
//! under the scheduler's bookkeeping lease), defer (a permission hold, or a
//! retry), fail, cancel, or admit, returning a [`Dispatch`] that holds the
//! lease. [`drain`] runs [`execute`] on that dispatch on its own task: a
//! scoped chat turn beside everything, and every other dispatch as the one
//! vault-lane run ([`Lane`], merged plan slice 3), supervised and joined by
//! the loop, which opens the automation gap when it ends. While it runs the
//! loop keeps stepping ([`step_beside`]) and admits scoped chat rows only.
//!
//! **One budget (2026-10-20).** Every agent turn counts against the
//! Settings limits ([`crate::chat_limits`], default 10 at once and 5 at once
//! in one workspace's folder; the person may raise them to 60 and 25): scoped chat turns, shared or exclusive, swarm
//! parts among them, and the vault lane's one run. A swarm part also stays
//! within its swarm's own setting. While macOS reports memory pressure at
//! warning or worse, starts beyond the first three wait; running turns are untouched. The
//! rule is [`QueueFacts::hold`], which a queued row's waiting reason reads
//! too.
//!
//! **The claim function** runs in two phases. Phase 1 reads cheap facts with
//! no side effects (the workflow file, the row's wait episode and cancel
//! flag, the live overdue tasks for a task-overdue fire, whether the
//! heartbeat is on) and the pure [`decide_claim`] settles every row it can:
//! a cancel, a missing workflow, a cron occurrence no longer due, a lapsed
//! trigger fire, a stale heartbeat. Only a row still standing reaches phase
//! 2: `workflows::precheck`, PR 5's side-effecting permission verify (it may
//! open one review and a rate-limited notice). Its verified workflow then
//! replaces phase 1's read and `decide_claim` runs again, so the run executes
//! exactly what was verified. Phase 3 (Paperclip PR 8a) is the usage-budget
//! preflight, run only for a row phase 2 would admit ([`budget_verdict`]): a
//! hard-stop budget consumes a cron occurrence, fails Run now and skips a
//! trigger or retry row, unless the workflow's own agent is at an `agent:`
//! ceiling and another agent it can run on has room (merged plan decision 6,
//! `budgets::preflight_with_fallback` over [`fallback_agents`]): then it runs
//! on that agent, and the switch goes on its receipt. PR 9b's
//! inactive-workspace and quiet gates
//! (`workflow_hygiene::cron_gate`, cron rows only) go between the verify and
//! the budget, which has side effects and so runs last: a quiet or inactive
//! occurrence never opens a budget pause. Its skip ([`ClaimVerdict::IdleSkip`])
//! is written under the scheduler's bookkeeping lease like every skip.
//!
//! **Retry rows (PR 9b).** A retry row is checked in phase 1 (the workflow is
//! missing, it ran since the failure, or the retry is stale) before the
//! permission check, so a retry that no longer matters opens no review. A
//! refusal, unreadable permission records or a budget block skip it too.
//! Every such end ([`ClaimVerdict::EndRetry`]) except "ran since" reports
//! the deferred failure (`workflow_retry::resolve_skipped_retry`).
//!
//! **Gap and grace.** Rows at or above `run_queue::GAP_PRIORITY` (triggers,
//! retries, cron, the heartbeat) wait [`MIN_AUTOMATION_GAP_SECS`] after any
//! run the pump executed, and [`CHAT_FOLLOWUP_GRACE_SECS`] after a chat turn
//! released the lease ([`note_chat_finished`]); Run now is exempt.
//!
//! **Ownership.** Exactly one Hollow process owns a vault's queue: the one
//! holding the flock beside its session database ([`claim_ownership`]).
//! Only the owner recovers the queue at startup, drains it, and runs the
//! scheduler and heartbeat enqueuers; another process (a dev build beside the
//! installed app) refuses Run now instead of queueing work it would never
//! run. Recovery ([`run_queue::recover_at_startup`], then deleting PR 2's
//! `.workflow-deferrals.json`) finishes before the first claim.
//!
//! **Locks.** The claim (including phase 2's verify) and every admission run
//! holding none of the run queue's `WAITERS`, the session store's
//! `WRITE_LOCK` or `HISTORY_LOCK`; nothing here drops or finishes a lease
//! inside a queue transaction.

use crate::agent::AgentKind;
use crate::automated_history::{
    self, BackgroundWriteLease, BeginRunError, LeaseHolder, RunLease, ScopeMode,
};
use crate::budgets;
use crate::chat::{self, ChatAdmission, ChatRowGuard, QueuedEnd};
use crate::core_host::Core;
use crate::flock::HeldLock;
use crate::heartbeat::{self, BeatGate, HeartbeatConfig, PreparedBeat};
use crate::run_queue::{self, NextRow, QueueRow, RowOutcome, RowStatus, RunSource, WaitEpisode};
use crate::trigger_evaluator::{FiredToken, TriggerSpec};
use crate::types::{RunLogEntry, RunQueueItem, Workflow, WorkflowRunResult};
use crate::vault_workflows;
use crate::workflow_hygiene::{self, CronGate, NotifyPolicy, RunContext};
use crate::workflow_retry::{self, RetrySkipReason};
use crate::workflows::{self, Admission, AdmitContext, AdmittedRun, PrecheckFailure, Runnable};
use chrono::{DateTime, Duration, Local, Timelike, Utc};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Pause after every run the pump executed before the next gap-bound row.
pub(crate) const MIN_AUTOMATION_GAP_SECS: i64 = 30;
/// Pause after a chat turn released the lease, so a queued routine never
/// grabs the vault the moment a turn ends and refuses the user's follow-up.
pub(crate) const CHAT_FOLLOWUP_GRACE_SECS: i64 = 60;
/// A held or failed row is retried this long after its claim.
pub(crate) const RETRY_DEFER_SECS: i64 = 60;
/// How soon a row whose precheck is still running is asked again (OC13).
pub(crate) const PRECHECK_POLL_SECS: i64 = 1;
/// Company work was removed 11-10 (COO rebuild L11). A row it queued before
/// (a priority continuation, an assignment's chat turn) ends once, failed
/// with this, and is never retried.
pub(crate) const COMPANY_REMOVED: &str = "company work was removed (11-10)";
/// Pressure is sampled each second while held, and sooner on a queue wake.
const MEMORY_RECHECK_SECS: i64 = 1;
/// Keep the first three turns usable even under pressure (SE3, approved).
const MEMORY_BOOTSTRAP_TURNS: usize = 3;
/// Leave physical headroom for the OS and work outside agent turns.
const MEMORY_FLOOR_BYTES: u64 = 2_000_000_000;
#[cfg(test)]
thread_local! {
    static TEST_HEADROOM: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

fn turn_memory_bytes(row: &QueueRow) -> u64 {
    let kind = row
        .payload
        .get("agent")
        .cloned()
        .and_then(|value| serde_json::from_value::<crate::agent::AgentKind>(value).ok());
    crate::agent::turn_memory_bytes(kind)
}
/// A vault-lane row ready this long (a Run now at once) gets the next free
/// slot before any scoped chat, so a steady backlog of workspace chats never
/// starves an automation of the app-wide limit (2026-10-20 review).
pub(crate) const LANE_FAIRNESS_SECS: i64 = 60;
/// A heartbeat row older than this is skipped as stale at claim.
pub(crate) const HEARTBEAT_ROW_TTL_MIN: i64 = 60;
/// A trigger row older than this has lapsed (the scanner's change window).
pub(crate) const TRIGGER_ROW_TTL_DAYS: i64 = 7;
/// How often a pump waiting on another writer re-checks (holders in other
/// processes release without waking us).
const BUSY_POLL: std::time::Duration = std::time::Duration::from_secs(5);
/// After a busy step, lease releases are ignored this long (an enqueue, a
/// stop and the vault lane's end are not): a holder that releases and
/// retakes the lease per item (Notion sync, per page, before 2026-11-10) woke the pump ~100
/// times a second, each step recording another wait on the queued row (S14).
const LEASE_WAKE_MIN: std::time::Duration = std::time::Duration::from_millis(250);
/// How long an idle pump sleeps between checks when nothing wakes it.
const IDLE_POLL: std::time::Duration = std::time::Duration::from_secs(60);
/// A missed notification must not leave an existing row asleep for a minute.
const QUEUED_POLL: std::time::Duration = std::time::Duration::from_secs(5);
/// Startup recovery is retried this often until it succeeds.
const RECOVERY_RETRY: std::time::Duration = std::time::Duration::from_secs(30);
/// Ceiling of the backoff after a failed step.
const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(60);

// ── Ownership and state ─────────────────────────────────────────────────────

/// Whether this process runs the vault's queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueState {
    /// Startup skipped the queue (the vault folder was missing at launch).
    NotStarted,
    /// Another Hollow process holds the owner lock for this vault.
    OwnedElsewhere,
    /// This process owns the queue and is recovering it; rows may be
    /// enqueued, none is claimed yet.
    Recovering,
    /// This process owns and drains the queue.
    Owned,
}

impl QueueState {
    /// True when this process owns the queue (recovering or running).
    pub fn owned(self) -> bool {
        matches!(self, QueueState::Recovering | QueueState::Owned)
    }
}

static STATE: Mutex<QueueState> = Mutex::new(QueueState::NotStarted);
/// The owner lock, held for the life of the process (the kernel releases the
/// flock when the process exits).
static OWNER_LOCK: Mutex<Option<HeldLock>> = Mutex::new(None);
/// When the last chat turn released the lease.
static CHAT_FINISHED_AT: Mutex<Option<DateTime<Utc>>> = Mutex::new(None);

fn set_state(state: QueueState) -> QueueState {
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = state;
    state
}

/// This process's queue state.
pub fn queue_state() -> QueueState {
    *STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Take the exclusive owner lock at `lock_path` (a `flock`, released when the
/// [`HeldLock`] drops or the process dies). `Err`
/// names why when another holder has it.
pub(crate) fn take_ownership(lock_path: &Path) -> Result<HeldLock, String> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("run queue lock dir: {e}"))?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| format!("open the run queue owner lock: {e}"))?;
    match HeldLock::try_take(file) {
        Ok(lock) => Ok(lock),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err("another Hollow process owns this vault's run queue".into())
        }
        Err(std::fs::TryLockError::Error(e)) => Err(format!("lock the run queue owner lock: {e}")),
    }
}

/// Claim this vault's queue for this process, once, early in startup (before
/// startup reconciliation that another live process's rows must survive).
/// Returns [`QueueState::Recovering`] when owned.
pub fn claim_ownership(vault: &Path) -> QueueState {
    let Some(path) = crate::session_store::pump_lock_path(vault) else {
        return set_state(QueueState::NotStarted);
    };
    match take_ownership(&path) {
        Ok(file) => {
            *OWNER_LOCK.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
            set_state(QueueState::Recovering)
        }
        Err(error) => {
            eprintln!("[run queue] not owned here: {error}");
            set_state(QueueState::OwnedElsewhere)
        }
    }
}

/// Let go of the vault's queue owner lock (`hollow serve`'s stop, after its
/// live runs settled), so the next owner claims it at once instead of when
/// this process's descriptors close.
pub(crate) fn release_ownership() {
    drop(OWNER_LOCK.lock().unwrap_or_else(|e| e.into_inner()).take());
}

/// `Err` with the reason when this process cannot accept queued work.
pub(crate) fn accepting() -> Result<(), String> {
    #[cfg(test)]
    if TEST_ACCEPTS.with(std::cell::Cell::get) {
        return Ok(());
    }
    accepting_in(queue_state())
}

#[cfg(test)]
thread_local! {
    /// Tests only: this test thread queues turns as an owning process would,
    /// without changing the process-wide queue state other tests read.
    pub(crate) static TEST_ACCEPTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Tests only: this thread's queue state as [`queue_state_here`] reads
    /// it; `None` reads as owned, the state every other test assumes.
    pub(crate) static TEST_QUEUE_STATE: std::cell::Cell<Option<QueueState>> =
        const { std::cell::Cell::new(None) };
}

/// This process's queue state for showing a row: whether its own pump will
/// claim a queued row ([`waiting`]).
#[cfg(not(test))]
fn queue_state_here() -> QueueState {
    queue_state()
}

/// Tests: this thread's [`TEST_QUEUE_STATE`], else owned.
#[cfg(test)]
fn queue_state_here() -> QueueState {
    TEST_QUEUE_STATE
        .with(std::cell::Cell::get)
        .unwrap_or(QueueState::Owned)
}

/// Why a queued row waits in a process that doesn't run the vault's queue
/// (2026-11-17, a #1499 leftover): another Hollow process owns it and will
/// claim the row, so here it reads Queued, never Working.
pub(crate) const OWNED_ELSEWHERE_WAIT: &str = "the Hollow process that runs this vault's queue";
/// Why a queued row waits when this process never started the queue.
pub(crate) const QUEUE_NOT_STARTED_WAIT: &str = "the run queue to start (restart Hollow)";

fn accepting_in(state: QueueState) -> Result<(), String> {
    match state {
        QueueState::Recovering | QueueState::Owned => Ok(()),
        QueueState::OwnedElsewhere => Err(
            "Another Hollow process owns this vault's run queue; run it from there, or quit the other copy and restart Hollow."
                .into(),
        ),
        QueueState::NotStarted => Err(
            "The run queue is not running (the vault folder was missing at launch); restart Hollow."
                .into(),
        ),
    }
}

/// The refusal for interactive work (Run now; chat in PR 7b) behind a lease
/// only a restart can clear: a stale lease, or one this build cannot
/// classify. `None` for any live holder, which interactive work waits for.
pub(crate) fn refuse_interactive(holder: Option<&LeaseHolder>) -> Option<String> {
    match holder {
        Some(holder @ (LeaseHolder::Stale { .. } | LeaseHolder::Unknown)) => {
            Some(crate::agent_sessions::busy_message(holder))
        }
        _ => None,
    }
}

/// [`refuse_interactive`], unless this process is running a tracked run on
/// `vault` (Paperclip PR 7b): a lease owner's manifest is agent-writable, so a
/// live run whose agent wrote `finishedAt` into it (or garbled `active-run`)
/// would otherwise read as stale and fail every chat send and queued chat or
/// Run now row with a restart hint while the run is still alive and will
/// release the lease normally. `run_guard` tracks from memory, so it cannot
/// be forged that way.
///
/// `holder` is a lock-free read, and a healthy lease reads as `Unknown`
/// while it is being written and as `Stale` between a run's finalized
/// manifest and its release. So a dead-looking `holder` is re-read through
/// [`automated_history::dead_lease_holder`] (under `HISTORY_LOCK`, twice)
/// and refused only if it stays dead. Blocking: call it on a blocking worker.
pub(crate) fn refuse_interactive_in(vault: &Path, holder: Option<&LeaseHolder>) -> Option<String> {
    if crate::run_guard::is_tracking(vault) {
        return None;
    }
    refuse_interactive(holder)?;
    let settled = automated_history::dead_lease_holder(vault)?;
    if crate::run_guard::is_tracking(vault) {
        return None;
    }
    refuse_interactive(Some(&settled))
}

/// A chat turn released the lease at `at`: gap-bound rows wait the follow-up
/// grace after it (chat's `finish_observed_run` calls this).
pub(crate) fn note_chat_finished(at: DateTime<Utc>) {
    let mut last = CHAT_FINISHED_AT.lock().unwrap_or_else(|e| e.into_inner());
    *last = (*last).max(Some(at));
}

/// When gap-bound rows may be claimed again: the later of the pump's own gap
/// and the chat follow-up grace. Pure.
pub(crate) fn gap_open_at(
    own: Option<DateTime<Utc>>,
    chat_finished: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    let grace = chat_finished.map(|at| at + Duration::seconds(CHAT_FOLLOWUP_GRACE_SECS));
    own.max(grace)
}

fn current_gap(own: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    let chat = *CHAT_FINISHED_AT.lock().unwrap_or_else(|e| e.into_inner());
    gap_open_at(own, chat)
}

/// Emit one `run-queue-event` (`enqueued` | `coalesced` | `waiting` |
/// `running` | `finished` | `cancelled` | `requeued` | `held`). The frontend
/// refetches the list on any event, so a lost emit costs only freshness.
pub(crate) fn emit(app: &impl crate::core_host::CoreHost, kind: &str, item: RunQueueItem) {
    let _ = app.emit(run_queue::EVENT, run_queue::event(kind, item));
}

fn emit_row(app: &Core, vault: &Path, kind: &str, id: i64) {
    if let Ok(Some(row)) = run_queue::get(vault, id) {
        emit(app, kind, run_queue::to_item(&row, None));
        // Any run that ended (a workflow or heartbeat too) may have held a
        // tracker sync back (N4); asking again costs an atomic.
        if kind == "finished" || kind == "cancelled" {
            crate::tracker_sync::after_run();
        }
        // A chat that ended may be one the Main agent started (main agent M2).
        if kind == "finished" && row.source == RunSource::Chat {
            crate::main_agent::after_chat_finished(app);
        }
    }
}

/// Emit the `chat-event`s a step returned for chat turns that ended without
/// starting (Paperclip PR 7b), after the rows are terminal.
fn emit_chat(app: &Core, events: Vec<Value>) {
    for event in events {
        chat::emit_event(app, event);
    }
}

/// Start the pump for `vault` when this process owns its queue (see
/// [`claim_ownership`]). `boot_at` is when this process started: queued Run
/// now rows requested before it belong to a previous process and are
/// interrupted, never run unattended. `stopped_leftovers` names the history
/// runs whose leftover processes the boot sweep ended
/// (`process_group::orphans::start_at_boot`); their interrupted rows say so.
pub fn start(
    app: impl Into<Core>,
    vault: PathBuf,
    boot_at: DateTime<Utc>,
    stopped_leftovers: HashSet<String>,
) {
    let app = app.into();
    if queue_state() != QueueState::Recovering {
        eprintln!("[run queue] not started here: {:?}", queue_state());
        return;
    }
    tauri::async_runtime::spawn(pump_loop(app, vault, boot_at, stopped_leftovers));
}

// ── The claim function ──────────────────────────────────────────────────────

/// The workflow file behind a workflow row, as phase 1 found it.
pub(crate) enum WorkflowFact {
    /// No file (deleted or renamed). Never passed to the permission check,
    /// which would tombstone the stamp and retire its reviews.
    Missing,
    /// The file exists but does not parse; the permission check reports why.
    Unreadable,
    Present(Workflow),
}

/// The facts [`decide_claim`] decides from.
pub(crate) struct ClaimFacts {
    pub(crate) workflow: WorkflowFact,
    /// `None` until phase 2 has run the permission check.
    pub(crate) precheck: Option<Result<(), PrecheckFailure>>,
    /// The live overdue task ids, read only for a row whose task-overdue or
    /// task-unblocked token fired (one task-list pass reads both sets).
    pub(crate) live_overdue_ids: Option<HashSet<String>>,
    /// The live unblocked task ids (Paperclip PR 11b), read with the overdue
    /// set.
    pub(crate) live_unblocked_ids: Option<HashSet<String>>,
    pub(crate) heartbeat_enabled: bool,
}

/// A workflow run-log write a skip carries out under the bookkeeping lease.
#[derive(Debug, Clone)]
pub(crate) struct Bookkeeping {
    pub(crate) slug: String,
    pub(crate) entry: RunLogEntry,
    /// Also move `next_run_at` past now (`skip_scheduled_run`).
    pub(crate) advance: bool,
}

/// Which aggregated notification a skip joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    /// Missed while Hollow was closed (or asleep).
    Missed,
    /// Another writer held the vault.
    Busy,
    /// The CFO held it past its grace: every account was past the reserve
    /// for the person's own prompts (COO rebuild L7).
    Reserve,
}

/// A skipped run to announce. The skip's run-log row is written either way;
/// `notify` (the workflow's `notify:` policy, Paperclip PR 9) decides at the
/// flush whether its title joins the banner ([`announced_titles`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkipNotice {
    pub(crate) title: String,
    pub(crate) kind: NoticeKind,
    pub(crate) notify: NotifyPolicy,
}

impl SkipNotice {
    fn for_workflow(workflow: &Workflow, kind: NoticeKind) -> Self {
        SkipNotice {
            title: workflow.title.clone(),
            kind,
            notify: NotifyPolicy::parse(workflow.notify.as_deref()),
        }
    }
}

/// A CFO-held occurrence skipped past its grace, in its run log.
pub(crate) const CFO_SKIP_NOTE: &str =
    "every account was past the reserve for your own prompts (Settings, Usage)";

/// What to do with a claimed row.
pub(crate) enum ClaimVerdict {
    RunWorkflow {
        runnable: Runnable,
        waited: Option<WaitEpisode>,
        manual: bool,
        /// The claim's usage budget (Paperclip PR 8a's preflight), carried to
        /// the run for PR 8b's mid-run enforcement.
        budget: Option<budgets::RunBudget>,
    },
    RunHeartbeat,
    /// Admit a chat turn (Paperclip PR 7b).
    RunChat,
    /// End the row `skipped` (`why` is its error), after the bookkeeping.
    Skip {
        why: String,
        bookkeeping: Option<Bookkeeping>,
        notice: Option<SkipNotice>,
    },
    /// Back to `queued`, not claimable before `not_before`. `held` is a
    /// permission hold (the run-log note), which is not a lease wait.
    Defer {
        not_before: DateTime<Utc>,
        held: Option<String>,
        clear_wait: bool,
    },
    Fail {
        error: String,
    },
    /// End the row `cancelled` (a cancel landed while it was queued or
    /// claimed), after the bookkeeping.
    Cancel {
        bookkeeping: Option<Bookkeeping>,
    },
    /// A cron occurrence with nothing to do (Paperclip PR 9b: its workspace
    /// is not current, or nothing it watches changed): `skip_idle_run` under
    /// the bookkeeping lease, then the row ends `skipped`. `due` is the
    /// occurrence, re-checked under the lease.
    IdleSkip {
        slug: String,
        title: String,
        skip: vault_workflows::IdleSkip,
        due: Option<String>,
        notify: NotifyPolicy,
    },
    /// A retry row that will not run (Paperclip PR 9b): end it with `status`
    /// and no run-log row, then report the failure it was deferring.
    EndRetry {
        status: RowStatus,
        reason: RetrySkipReason,
    },
}

/// The pure half of the claim.
pub(crate) enum Decision {
    Verdict(ClaimVerdict),
    /// Only the permission check can decide it.
    NeedPrecheck,
    /// Admit it (phase 2's verified workflow is the one to run).
    Run {
        waited: Option<WaitEpisode>,
        manual: bool,
    },
}

fn skip(why: impl Into<String>) -> Decision {
    Decision::Verdict(ClaimVerdict::Skip {
        why: why.into(),
        bookkeeping: None,
        notice: None,
    })
}

fn defer_in(now: DateTime<Utc>, held: Option<String>, clear_wait: bool) -> Decision {
    Decision::Verdict(ClaimVerdict::Defer {
        not_before: now + Duration::seconds(RETRY_DEFER_SECS),
        held,
        clear_wait,
    })
}

fn parse_stamp(stamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(stamp.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn older_than(row: &QueueRow, now: DateTime<Utc>, limit: Duration) -> bool {
    parse_stamp(&row.requested_at).is_none_or(|requested| now - requested > limit)
}

/// A wait that ended without its run (the occurrence or fire stopped being
/// due): silent when the workflow ran since it began (or it never waited),
/// the combined misfire note when it went quiet (Hollow closed while it
/// waited), otherwise one "no longer due" row. Append only.
fn wait_ended(row: &QueueRow, workflow: &Workflow, now: DateTime<Utc>, why: &str) -> Decision {
    let (entry, kind) = match &row.wait {
        Some(episode) if !episode.ran_since(workflow) && episode.abandoned(now) => {
            (episode.abandoned_entry(now), NoticeKind::Missed)
        }
        Some(episode) if !episode.ran_since(workflow) => {
            (episode.cleared_entry(now), NoticeKind::Busy)
        }
        _ => return skip(why),
    };
    Decision::Verdict(ClaimVerdict::Skip {
        why: why.to_string(),
        bookkeeping: Some(Bookkeeping {
            slug: workflow.slug.clone(),
            entry,
            advance: false,
        }),
        notice: Some(SkipNotice::for_workflow(workflow, kind)),
    })
}

/// The grace verdict for `due` (a stored `next_run_at`): `Some(grace_min)`
/// when the occurrence is past its misfire grace, `None` when it may still
/// run. An unreadable stamp counts as past grace.
fn past_grace(workflow: &Workflow, due: &str, now: DateTime<Utc>) -> Option<u64> {
    let grace_min = workflow
        .misfire_grace_min
        .unwrap_or(vault_workflows::DEFAULT_MISFIRE_GRACE_MIN);
    let Some(due_at) = parse_stamp(due) else {
        return Some(grace_min);
    };
    match workflows::decide_misfire(now, due_at, workflows::grace_window(grace_min)) {
        workflows::MisfireDecision::Skip => Some(grace_min),
        _ => None,
    }
}

/// Scheduler v2: whether a missed occurrence of `workflow` runs late instead
/// of being skipped past its grace (a window schedule, run-once-on-wake by
/// default, or an explicit run-once/run-all catch-up). Only for an
/// occurrence that may run: a refused, invalid or budget-held one keeps its
/// grace, so a hold never stretches into a run at the wrong hour.
fn catches_up(workflow: &Workflow) -> bool {
    use crate::schedule_windows::{effective_catch_up, CatchUp};
    matches!(
        effective_catch_up(&workflow.schedule),
        Some(CatchUp::RunOnceOnWake | CatchUp::RunAll)
    )
}

fn decide_cron(row: &QueueRow, facts: &ClaimFacts, now: DateTime<Utc>) -> Decision {
    let Some(due) = run_queue::due_at(&row.payload) else {
        return skip("the scheduled row has no occurrence");
    };
    let workflow = match &facts.workflow {
        WorkflowFact::Missing => return skip("the workflow no longer exists"),
        WorkflowFact::Unreadable => None,
        WorkflowFact::Present(workflow) => Some(workflow),
    };
    if row.cancel_requested {
        // Skipped from the queue: record the occurrence and advance past it
        // when it is still the next run, so the scheduler does not queue it
        // again; otherwise nothing is left to record.
        let bookkeeping = workflow
            .filter(|w| w.next_run_at.as_deref() == Some(due.as_str()))
            .map(|w| Bookkeeping {
                slug: w.slug.clone(),
                entry: vault_workflows::queue_cancelled_entry(now),
                advance: true,
            });
        return Decision::Verdict(ClaimVerdict::Cancel { bookkeeping });
    }
    let Some(w) = workflow else {
        // An unparseable file: only the permission check can say why. Within
        // the default grace it is retried; past it the row goes, without a
        // write into a file Hollow cannot parse.
        return match &facts.precheck {
            None => Decision::NeedPrecheck,
            Some(Err(PrecheckFailure::Refused { note, .. }))
                if !older_than(
                    row,
                    now,
                    workflows::grace_window(vault_workflows::DEFAULT_MISFIRE_GRACE_MIN),
                ) =>
            {
                defer_in(now, Some(note.clone()), true)
            }
            Some(Err(PrecheckFailure::Invalid(_)))
                if !older_than(
                    row,
                    now,
                    workflows::grace_window(vault_workflows::DEFAULT_MISFIRE_GRACE_MIN),
                ) =>
            {
                defer_in(now, None, false)
            }
            Some(Err(failure)) => skip(match failure {
                PrecheckFailure::Refused { note, .. } => note.clone(),
                PrecheckFailure::Invalid(error) => error.clone(),
            }),
            Some(Ok(())) => skip("the workflow could not be read"),
        };
    };
    if w.schedule == "manual" {
        return wait_ended(row, w, now, "the workflow no longer runs on a schedule");
    }
    // An unparseable schedule is never skipped or rewritten.
    if vault_workflows::validate_schedule(&w.schedule).is_err() {
        return skip("the schedule does not parse");
    }
    if w.next_run_at.as_deref() != Some(due.as_str()) {
        return wait_ended(row, w, now, "the occurrence was consumed or moved");
    }
    let stale = past_grace(w, &due, now);
    // A missed occurrence that may run catches up instead of being skipped.
    let missed = if catches_up(w) { None } else { stale };
    match &facts.precheck {
        None => Decision::NeedPrecheck,
        Some(Err(PrecheckFailure::Refused { note, .. })) => match stale {
            // Held: a hold is not a wait, so any episode is cleared.
            None => defer_in(now, Some(note.clone()), true),
            Some(_) => Decision::Verdict(ClaimVerdict::Skip {
                why: note.clone(),
                bookkeeping: Some(Bookkeeping {
                    slug: w.slug.clone(),
                    entry: vault_workflows::refused_skip_entry(note, now),
                    advance: true,
                }),
                notice: None,
            }),
        },
        Some(Err(PrecheckFailure::Invalid(error))) => match stale {
            None => defer_in(now, None, false),
            Some(_) => Decision::Verdict(ClaimVerdict::Skip {
                why: error.clone(),
                bookkeeping: Some(Bookkeeping {
                    slug: w.slug.clone(),
                    entry: vault_workflows::invalid_skip_entry(error, now),
                    advance: true,
                }),
                notice: None,
            }),
        },
        Some(Ok(()))
            if stale.is_some()
                && missed.is_none()
                && row.held_reason.as_deref() != Some(budgets::UNAVAILABLE_HELD_NOTE) =>
        {
            Decision::Run {
                waited: row.wait.clone(),
                manual: false,
            }
        }
        Some(Ok(())) => match stale {
            None => Decision::Run {
                waited: row.wait.clone(),
                manual: false,
            },
            // Held past its grace because usage could not be read (Paperclip
            // PR 8a, [`budget_blocked`]): say so, never "missed".
            // Held past its grace by the CFO (COO rebuild L7, review 6): the
            // occurrence is skipped with its own run-log row and one
            // notification naming the reserve, never a silent misfire.
            Some(_) if cfo_held(row) => Decision::Verdict(ClaimVerdict::Skip {
                why: CFO_SKIP_NOTE.into(),
                bookkeeping: Some(Bookkeeping {
                    slug: w.slug.clone(),
                    entry: vault_workflows::refused_skip_entry(CFO_SKIP_NOTE, now),
                    advance: true,
                }),
                notice: Some(SkipNotice::for_workflow(w, NoticeKind::Reserve)),
            }),
            Some(_) if row.held_reason.as_deref() == Some(budgets::UNAVAILABLE_HELD_NOTE) => {
                Decision::Verdict(ClaimVerdict::Skip {
                    why: budgets::UNAVAILABLE_HELD_NOTE.into(),
                    bookkeeping: Some(Bookkeeping {
                        slug: w.slug.clone(),
                        entry: vault_workflows::budget_skip_entry(
                            "usage could not be read",
                            None,
                            now,
                        ),
                        advance: true,
                    }),
                    notice: None,
                })
            }
            Some(grace_min) => {
                // Only this row's own wait explains the skip: live means the
                // lease was held past grace, quiet means Hollow closed while
                // it waited; with none it is a plain misfire.
                let (entry, kind) = match &row.wait {
                    Some(episode) if !episode.ran_since(w) && !episode.abandoned(now) => {
                        (episode.busy_skip_entry(now, grace_min), NoticeKind::Busy)
                    }
                    Some(episode) if !episode.ran_since(w) => {
                        (episode.abandoned_entry(now), NoticeKind::Missed)
                    }
                    _ => (
                        vault_workflows::missed_while_closed_entry(now),
                        NoticeKind::Missed,
                    ),
                };
                Decision::Verdict(ClaimVerdict::Skip {
                    why: format!("past its {grace_min}-minute grace"),
                    bookkeeping: Some(Bookkeeping {
                        slug: w.slug.clone(),
                        entry,
                        advance: true,
                    }),
                    notice: Some(SkipNotice::for_workflow(w, kind)),
                })
            }
        },
    }
}

/// True when a fired token's facts no longer hold: none of its freshly
/// overdue tasks is still overdue, or none of
/// its unblocked tasks is still unblocked (re-blocked, finished or
/// cancelled since). Task ids are `vault:tasks/…` URLs. A file-changed fire
/// (a level trigger) never lapses on its facts.
fn token_lapsed(token: &FiredToken, facts: &ClaimFacts) -> bool {
    // `meeting-in` was retired with Outlook (2026-09-26): a fire left on a
    // queued row from before lapses.
    if token.token.starts_with("meeting-in") {
        return true;
    }
    if token.token == "task-overdue" {
        return match &facts.live_overdue_ids {
            Some(live) => !token.overdue_ids.iter().any(|id| live.contains(id)),
            None => false,
        };
    }
    if token.token == "task-unblocked" {
        return match &facts.live_unblocked_ids {
            Some(live) => !token.unblocked_ids.iter().any(|id| live.contains(id)),
            None => false,
        };
    }
    false
}

fn decide_trigger(row: &QueueRow, facts: &ClaimFacts, now: DateTime<Utc>) -> Decision {
    if row.cancel_requested {
        return Decision::Verdict(ClaimVerdict::Cancel { bookkeeping: None });
    }
    let w = match &facts.workflow {
        WorkflowFact::Missing => return skip("the workflow no longer exists"),
        WorkflowFact::Unreadable => {
            return match &facts.precheck {
                None => Decision::NeedPrecheck,
                Some(Err(PrecheckFailure::Refused { note, .. })) => {
                    defer_in(now, Some(note.clone()), true)
                }
                Some(Err(PrecheckFailure::Invalid(error))) => skip(error.clone()),
                Some(Ok(())) => skip("the workflow could not be read"),
            }
        }
        WorkflowFact::Present(workflow) => workflow,
    };
    let listed: HashSet<String> = TriggerSpec::parse_all(&w.triggers)
        .iter()
        .map(TriggerSpec::token)
        .collect();
    let standing = run_queue::fired_tokens(&row.payload)
        .into_iter()
        .filter(|token| listed.contains(&token.token))
        .filter(|token| !token_lapsed(token, facts))
        .count();
    if standing == 0 || older_than(row, now, Duration::days(TRIGGER_ROW_TTL_DAYS)) {
        return wait_ended(row, w, now, "the trigger fire no longer holds");
    }
    match &facts.precheck {
        None => Decision::NeedPrecheck,
        // Held: it keeps the fire (its watermark was committed at enqueue).
        Some(Err(PrecheckFailure::Refused { note, .. })) => defer_in(now, Some(note.clone()), true),
        // A configuration error spends the fire, so it does not re-fire.
        Some(Err(PrecheckFailure::Invalid(error))) => skip(error.clone()),
        Some(Ok(())) => Decision::Run {
            waited: row.wait.clone(),
            manual: false,
        },
    }
}

fn decide_manual(row: &QueueRow, facts: &ClaimFacts) -> Decision {
    if row.cancel_requested {
        return Decision::Verdict(ClaimVerdict::Cancel { bookkeeping: None });
    }
    if matches!(facts.workflow, WorkflowFact::Missing) {
        let slug = row.source_id.as_deref().unwrap_or("?");
        return Decision::Verdict(ClaimVerdict::Fail {
            error: format!("workflow {slug} no longer exists"),
        });
    }
    match &facts.precheck {
        None => Decision::NeedPrecheck,
        Some(Err(PrecheckFailure::Refused { reason, .. })) => skip(reason.clone()),
        Some(Err(PrecheckFailure::Invalid(error))) => Decision::Verdict(ClaimVerdict::Fail {
            error: error.clone(),
        }),
        Some(Ok(())) => Decision::Run {
            waited: row.wait.clone(),
            manual: true,
        },
    }
}

fn end_retry(status: RowStatus, reason: RetrySkipReason) -> Decision {
    Decision::Verdict(ClaimVerdict::EndRetry { status, reason })
}

/// A retry row (Paperclip PR 9b). Precedence: a cancel, then missing > ran
/// since > stale, all decided before the permission check, then refused or
/// unreadable records (the budget block is phase 3). A configuration error
/// is reported as missing: the retry cannot run the workflow as written.
fn decide_retry(row: &QueueRow, facts: &ClaimFacts, now: DateTime<Utc>) -> Decision {
    if row.cancel_requested {
        return end_retry(RowStatus::Cancelled, RetrySkipReason::UserCancel);
    }
    let workflow = match &facts.workflow {
        WorkflowFact::Missing => return end_retry(RowStatus::Skipped, RetrySkipReason::Missing),
        WorkflowFact::Unreadable => None,
        WorkflowFact::Present(workflow) => Some(workflow),
    };
    if let Some(w) = workflow {
        let payload = workflow_retry::payload_of(&w.slug, &row.payload);
        let not_before = row
            .not_before
            .as_deref()
            .and_then(parse_stamp)
            .or_else(|| parse_stamp(&row.requested_at))
            .unwrap_or(now);
        if let Err(skip) = workflow_retry::retry_row_valid(
            w.last_started_at.as_deref(),
            &payload.failed_started_at,
            not_before,
            now,
        ) {
            return end_retry(RowStatus::Skipped, skip.into());
        }
    }
    match &facts.precheck {
        None => Decision::NeedPrecheck,
        Some(Err(PrecheckFailure::Refused { note, .. })) => end_retry(
            RowStatus::Skipped,
            if note == vault_workflows::PERMISSION_RECORDS_UNAVAILABLE_REASON {
                RetrySkipReason::Unavailable
            } else {
                RetrySkipReason::Refused
            },
        ),
        // The file is there but cannot run as written: not "missing".
        Some(Err(PrecheckFailure::Invalid(_))) => {
            end_retry(RowStatus::Skipped, RetrySkipReason::Invalid)
        }
        Some(Ok(())) if workflow.is_none() => {
            end_retry(RowStatus::Skipped, RetrySkipReason::Invalid)
        }
        Some(Ok(())) => Decision::Run {
            waited: row.wait.clone(),
            manual: false,
        },
    }
}

/// The pure claim rules, per source. There is no disabled state: a manual or
/// retry row is never checked against a schedule, grace or lapse, and a
/// manual workflow with triggers is valid.
pub(crate) fn decide_claim(row: &QueueRow, facts: &ClaimFacts, now: DateTime<Utc>) -> Decision {
    match row.source {
        // A priority continuation of the removed company (L11).
        RunSource::RestrictedResume => Decision::Verdict(removed_company_verdict(row)),
        RunSource::Cron => decide_cron(row, facts, now),
        RunSource::Trigger => decide_trigger(row, facts, now),
        RunSource::Manual => decide_manual(row, facts),
        RunSource::Retry => decide_retry(row, facts, now),
        RunSource::Heartbeat => {
            if row.cancel_requested {
                Decision::Verdict(ClaimVerdict::Cancel { bookkeeping: None })
            } else if !facts.heartbeat_enabled {
                skip("the heartbeat is off")
            } else if older_than(row, now, Duration::minutes(HEARTBEAT_ROW_TTL_MIN)) {
                skip("stale: the beat waited more than an hour")
            } else {
                Decision::Verdict(ClaimVerdict::RunHeartbeat)
            }
        }
        // A chat turn has no claim-time facts: a cancel that landed while it
        // was queued or claimed ends it, otherwise it runs (it never reaches
        // the permission check).
        RunSource::Chat if row.cancel_requested => {
            Decision::Verdict(ClaimVerdict::Cancel { bookkeeping: None })
        }
        RunSource::Chat if removed_company_row(row) => {
            Decision::Verdict(removed_company_verdict(row))
        }
        RunSource::Chat => Decision::Verdict(ClaimVerdict::RunChat),
        RunSource::Ingest => Decision::Verdict(ClaimVerdict::Fail {
            error: format!("{} rows are not run by this build", row.source.as_str()),
        }),
    }
}

/// Phase 1: cheap facts, no side effects.
fn claim_facts(vault: &Path, row: &QueueRow, deps: &StepDeps<'_>) -> ClaimFacts {
    let workflow = match row.source_id.as_deref() {
        Some(slug) if row.source.is_workflow() => {
            let present = crate::vault::safe_join(
                vault,
                &format!("{}/{slug}.md", vault_workflows::WORKFLOWS_DIR),
            )
            .is_ok_and(|path| path.is_file());
            if !present {
                WorkflowFact::Missing
            } else {
                match vault_workflows::read_full(vault, slug) {
                    Ok((workflow, _, _)) => WorkflowFact::Present(workflow),
                    Err(_) => WorkflowFact::Unreadable,
                }
            }
        }
        _ => WorkflowFact::Missing,
    };
    let task_ids = (row.source == RunSource::Trigger
        && run_queue::fired_tokens(&row.payload)
            .iter()
            .any(|token| token.token == "task-overdue" || token.token == "task-unblocked"))
    .then(|| crate::trigger_scanner::task_trigger_ids(vault));
    let (live_overdue_ids, live_unblocked_ids) = match task_ids {
        Some(ids) => (Some(ids.overdue), Some(ids.unblocked)),
        None => (None, None),
    };
    ClaimFacts {
        workflow,
        precheck: None,
        live_overdue_ids,
        live_unblocked_ids,
        heartbeat_enabled: deps.heartbeat_enabled,
    }
}

/// THE claim function: phase 1's cheap facts and [`decide_claim`], then, only
/// for a row still standing, phase 2's permission check (`deps.precheck`,
/// `workflows::precheck` in production) and `decide_claim` again on the
/// verified workflow. Holds no lock while it runs.
pub(crate) fn claim_verdict(
    vault: &Path,
    row: &QueueRow,
    now: DateTime<Utc>,
    deps: &StepDeps<'_>,
) -> ClaimVerdict {
    // A row the removed company queued ends here, before any hold (L11).
    if removed_company_row(row) {
        return removed_company_verdict(row);
    }
    // COO rebuild L7: an unattended chat turn (a subagent, a night item)
    // waits while no account of its agent has room past the reserve.
    if row.source == RunSource::Chat {
        if let Some(verdict) = cfo_hold(vault, row, None, now, deps) {
            return verdict;
        }
        // L9: a subagent past its own budget fails with the pause note; a
        // turn in a project past its budget waits until usage rolls out.
        if let Some(verdict) = cfo_budget(vault, row, now) {
            return verdict;
        }
    }
    // OC13: a Stop on a scheduled row ends its precheck child by PID.
    if row.cancel_requested && row.source == RunSource::Cron {
        if let Some(slug) = row.source_id.as_deref() {
            crate::workflow_precheck::cancel(vault, slug);
        }
    }
    let mut facts = claim_facts(vault, row, deps);
    match decide_claim(row, &facts, now) {
        Decision::Verdict(verdict) => return verdict,
        Decision::Run { .. } => {
            return ClaimVerdict::Fail {
                error: "internal: a row was admitted without its permission check".into(),
            }
        }
        Decision::NeedPrecheck => {}
    }
    let Some(slug) = row.source_id.as_deref() else {
        return ClaimVerdict::Fail {
            error: "the row names no workflow".into(),
        };
    };
    match (deps.precheck)(vault, slug) {
        Ok(runnable) => {
            // M4: automatic work (cron, trigger, retry) for an automation
            // that no longer fires here by the scheduler's rule (a synced
            // edit since it was queued) ends skipped, writing nothing to its
            // file. Run now is the person's click on this Mac and still runs.
            if matches!(
                row.source,
                RunSource::Cron | RunSource::Trigger | RunSource::Retry
            ) {
                if let Some(why) = crate::runs_on::claim_skip(vault, runnable.workflow()) {
                    return ClaimVerdict::Skip {
                        why,
                        bookkeeping: None,
                        notice: None,
                    };
                }
            }
            facts.workflow = WorkflowFact::Present(runnable.workflow().clone());
            facts.precheck = Some(Ok(()));
            match decide_claim(row, &facts, now) {
                // Phase 3 (Paperclip PR 8a): the usage-budget preflight, LAST
                // among the claim gates because it has side effects (it may
                // open a pause, a review item and a notification). A refused
                // or unsettled row never reaches it.
                // PR 9b's cron gates first, on the verified workflow: a
                // quiet or inactive occurrence never reaches the budget.
                Decision::Run { waited, manual } => {
                    if row.source == RunSource::Cron {
                        if let Some(verdict) = idle_verdict(vault, row, runnable.workflow(), now) {
                            return verdict;
                        }
                    }
                    budget_verdict(vault, row, runnable, waited, manual, now, deps)
                }
                Decision::Verdict(verdict) => verdict,
                Decision::NeedPrecheck => ClaimVerdict::Fail {
                    error: "internal: the permission check did not settle the row".into(),
                },
            }
        }
        Err(failure) => {
            facts.precheck = Some(Err(failure));
            match decide_claim(row, &facts, now) {
                Decision::Verdict(verdict) => verdict,
                _ => ClaimVerdict::Fail {
                    error: "internal: a refused row was not settled".into(),
                },
            }
        }
    }
}

/// Whether `row` was queued by the company code removed 11-10 (L11): a
/// priority continuation (`restrictedResume`), or a chat turn in an
/// assignment's conversation (`s-company-…`) or on its queued agent marker
/// (`hollow-company-…`). Such a row is settled once, never run.
pub(crate) fn removed_company_row(row: &QueueRow) -> bool {
    match row.source {
        RunSource::RestrictedResume => true,
        RunSource::Chat => {
            row.source_id
                .as_deref()
                .is_some_and(|key| key.starts_with("s-company-"))
                || row
                    .payload
                    .get("agent")
                    .and_then(Value::as_str)
                    .is_some_and(|agent| agent.starts_with("hollow-company-"))
        }
        _ => false,
    }
}

/// The one terminal verdict of a [`removed_company_row`]: cancelled when a
/// cancel already landed, else failed with [`COMPANY_REMOVED`].
fn removed_company_verdict(row: &QueueRow) -> ClaimVerdict {
    if row.cancel_requested {
        ClaimVerdict::Cancel { bookkeeping: None }
    } else {
        ClaimVerdict::Fail {
            error: COMPANY_REMOVED.into(),
        }
    }
}

/// Paperclip PR 9b: the inactive-workspace and quiet gates for a verified
/// cron occurrence, or None to go on to the budget.
fn idle_verdict(
    vault: &Path,
    row: &QueueRow,
    workflow: &Workflow,
    now: DateTime<Utc>,
) -> Option<ClaimVerdict> {
    let skipped = |skip| {
        Some(ClaimVerdict::IdleSkip {
            slug: workflow.slug.clone(),
            title: workflow.title.clone(),
            skip,
            due: run_queue::due_at(&row.payload),
            notify: NotifyPolicy::parse(workflow.notify.as_deref()),
        })
    };
    match workflow_hygiene::cron_gate(vault, workflow, now) {
        CronGate::Skip(skip) => return skipped(skip),
        CronGate::Run => {}
    }
    // OC13: the precheck command runs on its own thread; the claim never
    // waits on it. Pending: the row goes back to queued for a second and is
    // asked again, while other rows are claimed meanwhile.
    let due = run_queue::due_at(&row.payload).unwrap_or_default();
    match workflow_hygiene::precheck_gate(vault, workflow, &due) {
        workflow_hygiene::PrecheckGate::Run => None,
        workflow_hygiene::PrecheckGate::Pending => Some(ClaimVerdict::Defer {
            not_before: now + Duration::seconds(PRECHECK_POLL_SECS),
            held: None,
            clear_wait: false,
        }),
        workflow_hygiene::PrecheckGate::Skip(skip) => skipped(skip),
    }
}

/// Phase 3 of the claim (Paperclip PR 8a): run the workflow's usage-budget
/// preflight on the VERIFIED workflow (its stamped `agent`, so an agent
/// cannot dodge an `agent:` budget by editing its own file: `verify` refuses
/// that first). Clear or a warning (notified by the preflight) admits it.
/// Merged plan decision 6: when the stamped agent is at its own `agent:`
/// ceiling, the first of `deps.budget_fallbacks` with room takes the run
/// (the admitted budget names it, `RunBudget::fallback`); with none the
/// block stands and says so.
fn budget_verdict(
    vault: &Path,
    row: &QueueRow,
    runnable: Runnable,
    waited: Option<WaitEpisode>,
    manual: bool,
    now: DateTime<Utc>,
    deps: &StepDeps<'_>,
) -> ClaimVerdict {
    let workflow = runnable.workflow();
    let intent = budgets::RunIntent::workflow(
        &workflow.slug,
        budgets::intent_agent(workflow.agent.as_deref()),
    );
    let store = budgets::Store::for_app(vault);
    let candidates = || (deps.budget_fallbacks)(workflow);
    match budgets::preflight_with_fallback(&store, vault, &intent, &candidates, now) {
        budgets::Preflight::Blocked(notice) => budget_blocked(row, workflow, &notice, now),
        budgets::Preflight::Clear(budget) | budgets::Preflight::Warn(budget, _) => {
            // COO rebuild L7: a scheduled, triggered or retried automation
            // waits while no account its agent may start on has room past
            // the reserve. The agent is the one it will really run on: a
            // budget fallback's when the preflight moved it (review 3).
            let agent = budget
                .fallback
                .as_ref()
                .map(|fallback| fallback.to.as_str().to_string())
                .or_else(|| workflow.agent.clone().filter(|a| !a.trim().is_empty()))
                .or_else(|| {
                    crate::agent_settings::resolve(None, None)
                        .ok()
                        .map(|(kind, _)| kind.as_str().to_string())
                });
            if let Some(verdict) = cfo_hold(vault, row, agent.as_deref(), now, deps) {
                return verdict;
            }
            ClaimVerdict::RunWorkflow {
                runnable,
                waited,
                manual,
                budget: Some(budget),
            }
        }
    }
}

/// Whether `row` is unattended work for the CFO (COO rebuild L7): a chat
/// turn queued unattended, by another agent (`appDepth`), as a subagent
/// (`parent`, whatever its `unattended` flag) or by Hollow itself for the
/// COO or a lead (`hostAsker`: check-ins, wakes, child-completion reports);
/// a scheduled or triggered automation, and a retry of one. Never: the
/// person's own sends, Run now, a retry of a Run now, and the heartbeat.
pub(crate) fn cfo_unattended(vault: &Path, row: &QueueRow) -> bool {
    match row.source {
        RunSource::Chat => {
            let p = &row.payload;
            let named = |key: &str| {
                p.get(key)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
            };
            p.get("unattended") == Some(&Value::Bool(true))
                || named("parent")
                || named("hostAsker")
                || p.get("appDepth")
                    .and_then(Value::as_u64)
                    .is_some_and(|d| d > 0)
        }
        RunSource::Cron | RunSource::Trigger => true,
        // A retry is what its failed run was (review R3).
        RunSource::Retry => !row
            .retry_of
            .and_then(|id| run_queue::get(vault, id).ok().flatten())
            .is_some_and(|original| matches!(original.source, RunSource::Manual | RunSource::Chat)),
        _ => false,
    }
}

/// Whether a held row was held by the CFO (its note carries the mark).
pub(crate) fn cfo_held(row: &QueueRow) -> bool {
    row.held_reason
        .as_deref()
        .is_some_and(|note| note.starts_with(crate::cfo::accounts::HOLD_MARK))
}

/// The CFO's hold for unattended `row` on its agent (the chat payload's, or
/// `workflow_agent`): back to queued with the reason, retried shortly.
fn cfo_hold(
    vault: &Path,
    row: &QueueRow,
    workflow_agent: Option<&str>,
    now: DateTime<Utc>,
    deps: &StepDeps<'_>,
) -> Option<ClaimVerdict> {
    if row.cancel_requested || !cfo_unattended(vault, row) {
        return None;
    }
    let agent = match row.source {
        RunSource::Chat => row.payload.get("agent").and_then(Value::as_str)?,
        _ => workflow_agent?,
    };
    let held = (deps.cfo_hold)(vault, agent)?;
    Some(ClaimVerdict::Defer {
        not_before: now + Duration::seconds(RETRY_DEFER_SECS),
        held: Some(held),
        clear_wait: true,
    })
}

/// The CFO's budget check for an unattended chat turn (COO rebuild L9,
/// `cfo::budgets::claim`); the person's own sends are never checked.
fn cfo_budget(vault: &Path, row: &QueueRow, now: DateTime<Utc>) -> Option<ClaimVerdict> {
    if row.cancel_requested {
        return None;
    }
    let key = row.source_id.as_deref()?;
    if !cfo_unattended(vault, row) {
        // The person's own prompt to a paused subagent runs it again.
        crate::cfo::budgets::clear_pause(vault, key);
        return None;
    }
    let cwd = row
        .payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(Path::new);
    match crate::cfo::budgets::claim(vault, key, cwd, now)? {
        crate::cfo::budgets::ClaimBudget::Fail(note) => Some(ClaimVerdict::Fail { error: note }),
        crate::cfo::budgets::ClaimBudget::Hold(note) => Some(ClaimVerdict::Defer {
            not_before: now + Duration::seconds(RETRY_DEFER_SECS),
            held: Some(note),
            clear_wait: true,
        }),
    }
}

/// The notice title's reason for automations the CFO held past their grace
/// (review 6): names the reserve and where to change it.
pub(crate) fn reserve_skip_why(reserve: u8) -> String {
    format!(
        "because every account was past your {reserve}% reserve for your own prompts (Settings, Usage)"
    )
}

/// What a budget-blocked workflow row does (a hard stop means the occurrence
/// does not run, never that it runs late):
/// - cron: the occurrence is consumed (`next_run_at` advanced) under the
///   bookkeeping lease, with one `not run: budget paused since … (…)` row per
///   pause episode (`vault_workflows::skip_scheduled_run` coalesces them
///   there, under the lease);
/// - Run now: fails with the notice, which its waiter returns as the error;
/// - trigger: skipped with no row (its watermark was committed at enqueue,
///   so the fire is spent);
/// - retry: skipped with no row, reporting the failure it was deferring
///   (Paperclip PR 9b).
///
/// A block because usage or budget settings could not be READ (level
/// `Unavailable`) is not a pause: nothing says the scope reached its limit.
/// Like PR 5's unavailable permission records, a cron or trigger row is then
/// HELD (deferred, [`budgets::UNAVAILABLE_HELD_NOTE`]) and keeps its
/// occurrence or fire; a cron row still held past its grace is skipped by
/// [`decide_cron`] with a "usage could not be read" row.
fn budget_blocked(
    row: &QueueRow,
    workflow: &Workflow,
    notice: &budgets::BudgetNotice,
    now: DateTime<Utc>,
) -> ClaimVerdict {
    let unavailable = notice.level == budgets::NoticeLevel::Unavailable;
    match row.source {
        RunSource::Cron | RunSource::Trigger if unavailable => ClaimVerdict::Defer {
            not_before: now + Duration::seconds(RETRY_DEFER_SECS),
            held: Some(budgets::UNAVAILABLE_HELD_NOTE.into()),
            clear_wait: true,
        },
        RunSource::Cron => ClaimVerdict::Skip {
            why: notice.message.clone(),
            bookkeeping: Some(Bookkeeping {
                slug: workflow.slug.clone(),
                entry: vault_workflows::budget_skip_entry(
                    &budgets::skip_detail(notice),
                    notice.episode.as_deref(),
                    now,
                ),
                advance: true,
            }),
            notice: None,
        },
        RunSource::Manual => ClaimVerdict::Fail {
            error: notice.message.clone(),
        },
        RunSource::Retry => ClaimVerdict::EndRetry {
            status: RowStatus::Skipped,
            reason: RetrySkipReason::BudgetPaused,
        },
        _ => ClaimVerdict::Skip {
            why: notice.message.clone(),
            bookkeeping: None,
            notice: None,
        },
    }
}

// ── One step ────────────────────────────────────────────────────────────────

fn no_hook(_: &Path) {}

/// What a step reads beyond the queue, injectable for tests.
pub(crate) struct StepDeps<'a> {
    pub(crate) precheck: &'a dyn Fn(&Path, &str) -> Result<Runnable, PrecheckFailure>,
    pub(crate) prepare_beat: &'a dyn Fn(&Path) -> Result<PreparedBeat, String>,
    pub(crate) heartbeat_cfg: HeartbeatConfig,
    pub(crate) heartbeat_enabled: bool,
    pub(crate) local_hour: u32,
    pub(crate) today: String,
    pub(crate) bookkeeping_lease: &'a dyn Fn(&Path) -> Result<BackgroundWriteLease, BeginRunError>,
    /// Runs right before a skip's bookkeeping (a race hook for tests).
    pub(crate) before_bookkeeping: &'a dyn Fn(&Path),
    /// Runs right before an admission takes the lease (a race hook).
    pub(crate) before_admit: &'a dyn Fn(&Path),
    /// Runs right after a row is claimed, before its verdict (a race hook).
    pub(crate) after_claim: &'a dyn Fn(&Path),
    /// Tests: pretend `mark_running`'s queue write failed.
    pub(crate) fail_mark_running: bool,
    /// Merged plan decision 6: the agents a workflow blocked at its own
    /// agent's budget ceiling may move to ([`fallback_agents`] in
    /// production; tests pass a fixed list, or none).
    pub(crate) budget_fallbacks: &'a dyn Fn(&Workflow) -> Vec<AgentKind>,
    /// COO rebuild L7: why unattended work on an agent may not start now
    /// (`cfo::unattended_hold` in production; tests pass fixed readings).
    pub(crate) cfo_hold: &'a dyn Fn(&Path, &str) -> Option<String>,
}

/// Merged plan decision 6: the agents that may take over `workflow` when its
/// own agent is at a budget ceiling, in Hollow's agent order: installed, and
/// able to run it as written (its permission mode, and its tool
/// restrictions, which only an agent that enforces them may take: the same
/// rules `agent::validate_run_spec` applies at the spawn). The budget
/// preflight then keeps only one with room whose usage every hard-stop
/// budget over it can measure.
pub(crate) fn fallback_agents(workflow: &Workflow) -> Vec<AgentKind> {
    crate::agent::available()
        .into_iter()
        .filter(|kind| can_take_over(*kind, workflow))
        .collect()
}

/// Whether `kind` could run `workflow` as written (see [`fallback_agents`]).
fn can_take_over(kind: AgentKind, workflow: &Workflow) -> bool {
    let mode = match workflow.permission_mode.trim() {
        "" => "acceptEdits",
        mode => mode,
    };
    let forbidden = workflow.disallowed_tools.iter().any(|tool| {
        !matches!(
            tool.as_str(),
            "SendUserFile" | "AskUserQuestion" | "SendUserMessage"
        ) && !crate::agent::holds_deny(kind, tool)
    });
    let restricted = !workflow.allowed_tools.is_empty() || forbidden;
    crate::agent::unavailable_mode(kind, mode).is_none()
        && (!restricted || crate::agent::capabilities(kind).tool_restrictions)
}

impl StepDeps<'static> {
    /// The real dependencies, for a step at `now_local`.
    pub(crate) fn production(now_local: DateTime<Local>) -> StepDeps<'static> {
        StepDeps {
            precheck: &workflows::precheck,
            prepare_beat: &heartbeat::prepare_scheduled_beat,
            heartbeat_cfg: heartbeat::current_config(),
            heartbeat_enabled: heartbeat::enabled(),
            local_hour: now_local.hour(),
            today: now_local.format("%Y-%m-%d").to_string(),
            bookkeeping_lease: &automated_history::scheduler_write_lease,
            before_bookkeeping: &no_hook,
            before_admit: &no_hook,
            after_claim: &no_hook,
            fail_mark_running: false,
            budget_fallbacks: &fallback_agents,
            cfo_hold: &crate::cfo::unattended_hold,
        }
    }
}

/// A terminal write that failed, retried at the start of every step. Its
/// waiters were resolved from memory already.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingFinish {
    pub(crate) id: i64,
    pub(crate) status: RowStatus,
    pub(crate) error: Option<String>,
}

/// An admitted row. It holds the lease; dropping it finalizes the run as
/// cancelled (the `RunLease` drop), so a dispatch is never leaked.
pub(crate) enum Dispatch {
    Workflow {
        row: QueueRow,
        run: AdmittedRun,
    },
    Heartbeat {
        row: QueueRow,
        reason: String,
        prepared: PreparedBeat,
        lease: RunLease,
    },
    /// A chat turn (Paperclip PR 7b). `guard` holds the chat-row gate until
    /// the row is terminal.
    Chat {
        row: QueueRow,
        turn: chat::AdmittedTurn,
        guard: ChatRowGuard,
    },
}

impl Dispatch {
    pub(crate) fn row(&self) -> &QueueRow {
        match self {
            Dispatch::Workflow { row, .. }
            | Dispatch::Heartbeat { row, .. }
            | Dispatch::Chat { row, .. } => row,
        }
    }

    /// A build workspace's chat turn on its own folder (PR B3): the loop runs
    /// it on its own task and keeps admitting vault-lane rows meanwhile.
    pub(crate) fn is_scoped(&self) -> bool {
        matches!(self, Dispatch::Chat { turn, .. } if turn.is_scoped())
    }
}

/// What one step did.
pub(crate) enum PumpStep {
    /// Nothing is queued.
    Idle,
    /// Rows are queued but none is claimable before this instant.
    Sleep(DateTime<Utc>),
    /// Another writer holds the lease; nothing was claimed. `episodes_started`
    /// are rows whose wait began now; `failed` are queued Run now rows ended
    /// behind a stale or unknown lease.
    Busy {
        #[cfg_attr(not(test), allow(dead_code))]
        holder: LeaseHolder,
        episodes_started: Vec<i64>,
        failed: Vec<i64>,
        /// The `chat-event`s of queued chat turns among `failed` (PR 7b).
        chat: Vec<Value>,
    },
    /// A row was decided without a run (skipped, held, deferred, requeued,
    /// failed, cancelled), or lost its claim to a concurrent change. `chat`
    /// holds the `chat-event`s of a chat row that ended without starting.
    Handled {
        row_id: i64,
        event: Option<&'static str>,
        notice: Option<SkipNotice>,
        chat: Vec<Value>,
    },
    Admitted(Dispatch),
    Error(String),
}

fn retry_pending(vault: &Path, now: DateTime<Utc>, pending: &mut Vec<PendingFinish>) {
    pending.retain(|finish| {
        run_queue::finish(
            vault,
            finish.id,
            finish.status,
            finish.error.as_deref(),
            now,
        )
        .is_err()
    });
}

/// End a row with a terminal status and resolve its waiters. A failed write
/// is queued for retry; the waiters are resolved either way.
fn end_row(
    vault: &Path,
    id: i64,
    status: RowStatus,
    error: Option<&str>,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
) {
    // P1a: a resume's chat row that ends here (refused at admission,
    // skipped, failed or cancelled before start, or after its turn) gives
    // back the claim it holds on its paused chat, read before the terminal
    // write clears its payload. After a spawn the record is gone, so this
    // releases nothing; only the row's own token can release (`chat_pause`).
    if let Ok(Some(row)) = run_queue::get(vault, id) {
        // Delta review: a cancel is the person's; only a failure warns.
        crate::chat_pause::release_for_row(vault, &row, status != RowStatus::Cancelled);
    }
    if let Err(write_error) = run_queue::finish(vault, id, status, error, now) {
        eprintln!("[run queue] could not finish row {id}: {write_error}; retrying");
        // One owed write per row: a second would finish it twice.
        if !pending.iter().any(|finish| finish.id == id) {
            pending.push(PendingFinish {
                id,
                status,
                error: error.map(str::to_string),
            });
        }
    }
    run_queue::resolve_waiters(
        vault,
        id,
        RowOutcome::Finished {
            status,
            error: error.map(str::to_string),
        },
    );
}

/// Behind a stale or unknown lease, queued Run now rows and chat turns fail
/// at once (Run now's waiters get the restart hint, a chat turn its error and
/// `done`, returned for the loop to emit); automation rows keep waiting,
/// bounded by their grace and TTL. A row whose terminal write is already
/// owed (it stays `queued` until the retry lands) has ended: it is skipped,
/// so its chat sees one error and `done` and it is finished once.
fn fail_interactive(
    vault: &Path,
    message: &str,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
) -> (Vec<i64>, Vec<Value>) {
    let Ok(rows) = run_queue::live_rows(vault) else {
        return (Vec::new(), Vec::new());
    };
    let mut failed = Vec::new();
    let mut chat = Vec::new();
    for row in rows {
        if row.status == RowStatus::Queued
            && matches!(row.source, RunSource::Manual | RunSource::Chat)
            && !pending.iter().any(|finish| finish.id == row.id)
            // A build workspace's turn never waits for the vault (PR B3).
            && !scoped_chat(vault, &row)
        {
            chat.extend(chat_end(&row, QueuedEnd::Failed(message.to_string())));
            end_row(
                vault,
                row.id,
                RowStatus::Failed,
                Some(message),
                now,
                pending,
            );
            failed.push(row.id);
        }
    }
    (failed, chat)
}

fn requeue(vault: &Path, row: &QueueRow, holder: &LeaseHolder, now: DateTime<Utc>) -> PumpStep {
    match run_queue::requeue_waiting(vault, row.id, holder, now) {
        Ok(started) => PumpStep::Handled {
            row_id: row.id,
            event: Some(if started { "waiting" } else { "requeued" }),
            notice: None,
            chat: Vec::new(),
        },
        Err(error) => PumpStep::Error(error),
    }
}

/// A failed bookkeeping lease or write: retry the row a minute later with
/// the error noted, never at once (the lease is free, so an immediate retry
/// would spin).
fn defer_after_failure(vault: &Path, row: &QueueRow, now: DateTime<Utc>, error: &str) -> PumpStep {
    let not_before = now + Duration::seconds(RETRY_DEFER_SECS);
    match run_queue::defer(
        vault,
        row.id,
        not_before,
        row.held_reason.as_deref(),
        false,
        Some(error),
    ) {
        Ok(_) => PumpStep::Handled {
            row_id: row.id,
            event: Some("requeued"),
            notice: None,
            chat: Vec::new(),
        },
        Err(write_error) => PumpStep::Error(write_error),
    }
}

/// A row admitted this step whose `mark_running` write failed (merged plan
/// slice 3, A·R4): it is not executed this step. The caller has already
/// finalized its run unexecuted (cancelled). The row goes back to `queued`
/// a minute later with the error noted or, if that write fails too, stays
/// `claimed` for the next step's stray-claim release. Either way it runs
/// once, on a later step, and never beside a copy of itself.
fn unmarked(vault: &Path, row: &QueueRow, now: DateTime<Utc>, error: &str) -> PumpStep {
    eprintln!(
        "[run queue] could not mark row {} running: {error}; it runs on a later step",
        row.id
    );
    let note = format!("could not mark it running: {error}");
    defer_after_failure(vault, row, now, &note)
}

/// Carry out a skip or cancel's run-log write under the bookkeeping lease,
/// then end the row. `Busy` re-queues it with nothing written. `chat` is the
/// ended chat turn's `chat-event`s (a chat row has no bookkeeping).
#[allow(clippy::too_many_arguments)]
fn end_with_bookkeeping(
    vault: &Path,
    row: &QueueRow,
    bookkeeping: Option<Bookkeeping>,
    status: RowStatus,
    error: &str,
    notice: Option<SkipNotice>,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
    chat: Vec<Value>,
) -> PumpStep {
    if let Some(write) = bookkeeping {
        (deps.before_bookkeeping)(vault);
        match workflows::skip_bookkeeping(
            vault,
            &write.slug,
            write.entry,
            write.advance,
            now,
            deps.bookkeeping_lease,
        ) {
            Ok(()) => {}
            Err(BeginRunError::Busy(holder)) => return requeue(vault, row, &holder, now),
            Err(BeginRunError::Failed(failure)) => {
                return defer_after_failure(vault, row, now, &failure)
            }
        }
    }
    end_row(vault, row.id, status, Some(error), now, pending);
    PumpStep::Handled {
        row_id: row.id,
        event: Some(if status == RowStatus::Cancelled {
            "cancelled"
        } else {
            "finished"
        }),
        notice,
        chat,
    }
}

// Test record of the notices `idle_skip` sent on this thread (there is no
// registered app handle in tests, so `create_registered` writes nothing).
#[cfg(test)]
thread_local! {
    static IDLE_NOTICES: std::cell::RefCell<Vec<crate::notifications::NotificationInput>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The notice an idle skip sends: only the first skip of an inactive episode
/// that was actually written (not coalesced into an earlier row, not moved),
/// and only when the workflow's `notify` allows skip notices. A quiet skip
/// sends none.
fn idle_skip_notice(
    title: &str,
    slug: &str,
    skip: &vault_workflows::IdleSkip,
    wrote: vault_workflows::IdleSkipWrite,
    notify: NotifyPolicy,
) -> Option<crate::notifications::NotificationInput> {
    match skip {
        vault_workflows::IdleSkip::Inactive { workspace, status }
            if wrote.written && !wrote.coalesced && notify.allows_skip_notice() =>
        {
            Some(workflow_hygiene::inactive_notice(
                title, slug, workspace, status,
            ))
        }
        _ => None,
    }
}

/// Write an idle skip (Paperclip PR 9b) under the bookkeeping lease, then end
/// the row `skipped`. `Busy` re-queues it with nothing written (the gate runs
/// again at the next claim); a failed write retries it a minute later. The
/// first skip of an inactive episode (not coalesced into an earlier row) also
/// records one in-app notice, unless the workflow's `notify` keeps it out of
/// skip notices.
#[allow(clippy::too_many_arguments)]
fn idle_skip(
    vault: &Path,
    row: &QueueRow,
    slug: &str,
    title: &str,
    skip: &vault_workflows::IdleSkip,
    due: Option<&str>,
    notify: NotifyPolicy,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
) -> PumpStep {
    (deps.before_bookkeeping)(vault);
    let wrote =
        match workflows::idle_skip_bookkeeping(vault, slug, skip, due, now, deps.bookkeeping_lease)
        {
            Ok(wrote) => wrote,
            Err(BeginRunError::Busy(holder)) => return requeue(vault, row, &holder, now),
            Err(BeginRunError::Failed(failure)) => {
                return defer_after_failure(vault, row, now, &failure)
            }
        };
    if let Some(notice) = idle_skip_notice(title, slug, skip, wrote, notify) {
        #[cfg(test)]
        IDLE_NOTICES.with(|sent| sent.borrow_mut().push(notice.clone()));
        if let Some(Err(error)) = crate::notifications::create_registered(vault, notice) {
            eprintln!("[workflow {slug}] could not record the inactive-workspace notice: {error}");
        }
    }
    let why = if wrote.written {
        skip.note()
    } else {
        "the occurrence was consumed or moved".to_string()
    };
    // A broken precheck (OC13) is a failure, not a skip: the queue row fails,
    // the failure streak counts it (the Workflows warning), and the person
    // gets one in-app notice per failure unless `notify: never`.
    let failed = skip.is_failure() && wrote.written;
    if failed {
        let streak = workflow_hygiene::record_outcome(
            vault,
            slug,
            title,
            "error",
            &format!("precheck-{}", row.id),
        );
        let notice = match &streak {
            Some(streak) => Some(workflow_hygiene::streak_notification(title, slug, streak)),
            None if notify.allows_run("error") => Some(crate::notifications::NotificationInput {
                title: format!("{title}: its precheck is broken"),
                body: format!("{why}. Scheduled runs are not starting until it is fixed."),
                kind: "error".to_string(),
                source: Some(slug.to_string()),
                action_label: Some("Open workflow".to_string()),
                action_url: Some(format!("workflow:{slug}")),
                notify_native: false,
            }),
            None => None,
        };
        if let Some(notice) = notice {
            #[cfg(test)]
            IDLE_NOTICES.with(|sent| sent.borrow_mut().push(notice.clone()));
            if let Some(Err(error)) = crate::notifications::create_registered(vault, notice) {
                eprintln!(
                    "[workflow {slug}] could not record the precheck failure notice: {error}"
                );
            }
        }
    }
    let end_status = if failed {
        RowStatus::Failed
    } else {
        RowStatus::Skipped
    };
    end_row(vault, row.id, end_status, Some(&why), now, pending);
    PumpStep::Handled {
        row_id: row.id,
        event: Some("finished"),
        notice: None,
        chat: Vec::new(),
    }
}

/// The `chat-event`s a chat row ending without its turn starting emits;
/// empty for every other source. Built from the claimed row in memory,
/// before the terminal write clears its payload.
fn chat_end(row: &QueueRow, end: QueuedEnd) -> Vec<Value> {
    if row.source == RunSource::Chat {
        chat::queued_end_events(row, &end)
    } else {
        Vec::new()
    }
}

#[allow(clippy::too_many_arguments)]
fn admit_workflow(
    vault: &Path,
    row: QueueRow,
    runnable: Runnable,
    waited: Option<WaitEpisode>,
    manual: bool,
    budget: Option<budgets::RunBudget>,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
) -> PumpStep {
    let slug = row.source_id.clone().unwrap_or_default();
    (deps.before_admit)(vault);
    let ctx = AdmitContext {
        waited: waited.as_ref(),
        manual,
        run: RunContext::from_row(&row),
        budget,
    };
    match workflows::admit_runnable(vault, &slug, runnable, ctx) {
        Ok(Admission::Admitted(mut run)) => {
            // A row that is not marked running is not executed (A·R4): the
            // run is dropped (finalized as cancelled, lease released) before
            // any sibling is absorbed, and the row runs on a later step.
            let marked = if deps.fail_mark_running {
                Err("injected mark_running failure".to_string())
            } else {
                run_queue::mark_running(vault, row.id, run.run_id(), now)
            };
            if let Err(error) = marked {
                drop(run);
                return unmarked(vault, &row, now, &error);
            }
            // From here on the run holds the lease and is never dropped: a
            // queue write that fails is logged and reconciled at finish.
            match run_queue::absorb_siblings(vault, row.id, &slug, now) {
                // An absorbed trigger fire's reason reaches the prompt's
                // `{{trigger_reason}}` (the context was built from the row
                // as claimed, before the merge).
                Ok(absorbed) => run.absorb_context(&absorbed),
                Err(error) => {
                    eprintln!("[run queue] could not absorb {slug}'s other queued rows: {error}");
                }
            }
            // Read only after `mark_running`: a cancel either saw `running`
            // and stopped the run's slot, or set the flag that is read here.
            if run_queue::cancel_requested(vault, row.id).unwrap_or(false) {
                run.request_stop();
            }
            PumpStep::Admitted(Dispatch::Workflow { row, run })
        }
        Ok(Admission::Deferred { holder }) => requeue(vault, &row, &holder, now),
        Err(error) => {
            end_row(vault, row.id, RowStatus::Failed, Some(&error), now, pending);
            // A retry that never starts still reports the failure it was
            // deferring (Paperclip PR 9b), or the failed run's banner, which
            // `finalize` replaced with the quiet "retrying once" notice, and
            // its streak outcome would both be lost.
            if row.source == RunSource::Retry {
                workflow_retry::resolve_skipped_retry(
                    vault,
                    &slug,
                    &row.payload,
                    RetrySkipReason::NotStarted,
                );
            }
            PumpStep::Handled {
                row_id: row.id,
                event: Some("finished"),
                notice: None,
                chat: Vec::new(),
            }
        }
    }
}

fn end_cancelled_heartbeat(
    vault: &Path,
    row: &QueueRow,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
) -> PumpStep {
    end_row(
        vault,
        row.id,
        RowStatus::Cancelled,
        Some(run_queue::CANCELLED_BEFORE_START),
        now,
        pending,
    );
    PumpStep::Handled {
        row_id: row.id,
        event: Some("cancelled"),
        notice: None,
        chat: Vec::new(),
    }
}

fn admit_heartbeat(
    vault: &Path,
    row: QueueRow,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
) -> PumpStep {
    // A cancel that landed after the claim read the row (while its facts were
    // gathered) is honoured before the beat is prepared or charged.
    if run_queue::cancel_requested(vault, row.id).unwrap_or(false) {
        return end_cancelled_heartbeat(vault, &row, now, pending);
    }
    let gate = heartbeat::gate_scheduled_beat(
        vault,
        deps.heartbeat_cfg,
        deps.local_hour,
        &deps.today,
        now,
        |v| (deps.prepare_beat)(v),
    );
    match gate {
        BeatGate::Skip => {
            end_row(
                vault,
                row.id,
                RowStatus::Skipped,
                Some("not due now (a heartbeat guardrail)"),
                now,
                pending,
            );
            PumpStep::Handled {
                row_id: row.id,
                event: Some("finished"),
                notice: None,
                chat: Vec::new(),
            }
        }
        BeatGate::Busy(holder) => requeue(vault, &row, &holder, now),
        // A usage budget holds the heartbeat (Paperclip PR 8a): skipped
        // silently (the pause itself notified), nothing charged.
        BeatGate::BudgetPaused(message) => {
            end_row(
                vault,
                row.id,
                RowStatus::Skipped,
                Some(&message),
                now,
                pending,
            );
            PumpStep::Handled {
                row_id: row.id,
                event: Some("finished"),
                notice: None,
                chat: Vec::new(),
            }
        }
        BeatGate::Admitted {
            reason,
            prepared,
            lease,
        } => {
            let marked = if deps.fail_mark_running {
                Err("injected mark_running failure".to_string())
            } else {
                run_queue::mark_running(vault, row.id, lease.id(), now)
            };
            // Not marked, not executed (A·R4): the beat's run ends
            // cancelled and the row is queued again. Its budget slot was
            // charged at admission, so the later step's gate may skip it as
            // not due; it never runs twice.
            if let Err(error) = marked {
                if let Err(finish) = lease.finish("cancelled") {
                    eprintln!("[heartbeat] finishing an unmarked beat's run failed: {finish}");
                }
                return unmarked(vault, &row, now, &error);
            }
            // Read only after `mark_running`, like a workflow: a cancel either
            // saw `running` (and was refused) or set the flag read here while
            // the beat was being prepared. The beat never starts; its budget
            // slot is already spent (charged once it held the lease).
            if run_queue::cancel_requested(vault, row.id).unwrap_or(false) {
                if let Err(error) = lease.finish("cancelled") {
                    eprintln!("[heartbeat] finishing a cancelled beat's run failed: {error}");
                }
                return end_cancelled_heartbeat(vault, &row, now, pending);
            }
            PumpStep::Admitted(Dispatch::Heartbeat {
                row,
                reason,
                prepared,
                lease,
            })
        }
    }
}

/// One pump step (see the module doc). Sync and AppHandle-free.
#[cfg(test)]
pub(crate) fn step(
    vault: &Path,
    now: DateTime<Utc>,
    gap_open_at: Option<DateTime<Utc>>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
) -> PumpStep {
    step_beside(vault, now, gap_open_at, pending, deps, None)
}

/// [`step`], told which row the loop's vault lane is running, if any
/// (merged plan slice 3, F09). While a vault-lane run is live only a scoped
/// chat row may start: no other row is claimed, so there is never a second
/// vault writer, and the lane's own lease is not "another writer" (no wait
/// episode starts behind it and no Run now row fails). The lane's row is
/// never released as a stray claim while it runs.
pub(crate) fn step_beside(
    vault: &Path,
    now: DateTime<Utc>,
    gap_open_at: Option<DateTime<Utc>>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
    lane: Option<i64>,
) -> PumpStep {
    retry_pending(vault, now, pending);
    // A claim whose follow-up write failed (a defer or requeue behind the
    // database's busy timeout) left its row `claimed`, and nothing else ever
    // re-claims one: put it back before choosing the head. Rows with a
    // pending terminal write stay put; they have already run, and so does
    // the live vault-lane row, and an active chat row (A·R4: an active claim
    // is never queued again, so its turn can never be admitted twice).
    let mut keep: Vec<i64> = pending.iter().map(|finish| finish.id).collect();
    keep.extend(lane);
    let active_chat = chat::active_chat_rows(vault);
    keep.extend(&active_chat);
    match run_queue::release_stray_claims(vault, &keep) {
        Ok(released) => {
            for id in released {
                eprintln!(
                    "[run queue] row {id} was left claimed by a failed write; queued it again"
                );
            }
        }
        Err(error) => return PumpStep::Error(error),
    }
    // A build workspace's turn waits only for its own workspace and the
    // limits (build programme PRs B3/B4), never for the vault. One budget
    // (2026-10-20): every agent turn live now, the vault lane's included,
    // counts against the Settings limits ([`QueueFacts::hold`]).
    let live = run_queue::live_rows(vault).unwrap_or_default();
    let finished: Vec<i64> = pending.iter().map(|finish| finish.id).collect();
    let facts = QueueFacts::read_beside(vault, &live, lane, &finished, now, gap_open_at);
    // A row whose turn is already active is never started again (A·R4).
    // Several of one workspace's conversations may run at once (2026-10-07);
    // a build workspace's worktree still takes one turn at a time. While the
    // loop's vault lane runs, no other vault-lane row starts.
    // Card holds are recorded while `startable` skips a non-person chat row.
    // They are not written onto the row, so an answer before the cap lets the
    // next step start the wake. The cache is per step: one session is asked
    // once. `startable` runs after the queue read lock is released.
    let card_holds: RefCell<Vec<DateTime<Utc>>> = RefCell::new(Vec::new());
    let hold_cache: RefCell<HashMap<String, Option<DateTime<Utc>>>> = RefCell::new(HashMap::new());
    // SE0: each row skipped for a hold, with its cause, recorded after the
    // queue read below (never inside `startable`, which runs under it).
    let skipped_holds: RefCell<Vec<(i64, &'static str)>> = RefCell::new(Vec::new());
    let startable = |row: &QueueRow| {
        if active_chat.contains(&row.id) {
            return false;
        }
        // A row that owes a terminal write has ended (it stays `queued` until
        // the retry lands): it never starts again.
        if finished.contains(&row.id) {
            return false;
        }
        // A chat turn Stopped while it was claimed (C-S3, 11-14): the next
        // claim only ends it, so it waits for no hold, card or other turn.
        if ends_on_claim(row) {
            return true;
        }
        if conversation_busy(&live, &finished, row) {
            return false;
        }
        if let Some(hold) = facts.hold_beyond_memory(vault, row, lane.is_some()) {
            // Only a row whose recorded cause differs needs a write, so a
            // long hold costs no write per pump step.
            let recorded = row.phase_timings.as_ref().is_some_and(|t| {
                t.get("holdKind").and_then(|k| k.as_str()) == Some(hold.kind())
                    && t.get("holdSince").is_some()
            });
            if !recorded {
                skipped_holds.borrow_mut().push((row.id, hold.kind()));
            }
            return false;
        }
        if row.source == RunSource::Chat && !run_queue::payload_closes_cards(&row.payload) {
            if let Some(key) = row.source_id.as_deref() {
                if let Some(until) =
                    cached_open_hold(vault, key, now, &hold_cache).filter(|at| *at > now)
                {
                    card_holds.borrow_mut().push(until);
                    return false;
                }
            }
        }
        true
    };
    let next = match run_queue::next_ready_where(vault, now, gap_open_at, startable) {
        Ok(next) => next,
        Err(error) => return PumpStep::Error(error),
    };
    for (id, kind) in skipped_holds.take() {
        if let Err(error) = run_queue::record_hold(vault, id, kind, now) {
            eprintln!("[run queue] could not record hold: {error}");
        }
    }
    if next == NextRow::Empty {
        if let Some(at) = soonest_hold(&card_holds) {
            return PumpStep::Sleep(at);
        }
        return PumpStep::Idle;
    }
    // SE3: the first three turns may start under pressure; subsequent turns
    // retry within one second. Running turns are never interrupted.
    let cancelled_head = matches!(&next, NextRow::Ready(row) if ends_on_claim(row));
    if facts.memory_held() && !cancelled_head {
        if let NextRow::Ready(row) = &next {
            if let Err(error) =
                run_queue::record_memory_hold(vault, row.id, facts.pressure.unwrap(), now)
            {
                eprintln!("[run queue] could not record memory hold: {error}");
            }
        }
        return match next {
            NextRow::NotBefore(at) => PumpStep::Sleep(at),
            _ => PumpStep::Sleep(now + Duration::seconds(MEMORY_RECHECK_SECS)),
        };
    }
    let scoped_head = matches!(&next, NextRow::Ready(row) if scoped_chat(vault, row));
    // While the vault is held, a startable build turn further back still goes.
    let lease = match lane {
        // The loop's own vault lane holds it: that is not a wait, and only
        // scoped rows were startable above.
        Some(_) => None,
        None => automated_history::lease_holder(vault).filter(|_| !scoped_head && !cancelled_head),
    };
    let next = match &lease {
        Some(_) => match run_queue::next_ready_where(vault, now, gap_open_at, |row| {
            chat_scope(vault, row).is_some() && startable(row)
        }) {
            Ok(NextRow::Ready(row)) => NextRow::Ready(row),
            Ok(_) => next,
            Err(error) => return PumpStep::Error(error),
        },
        None => next,
    };
    let lease = lease.filter(
        |_| !matches!(&next, NextRow::Ready(row) if scoped_chat(vault, row) || ends_on_claim(row)),
    );
    // The lease fallback may have selected a different adapter. Check the
    // actual candidate, not the original head's footprint.
    if let NextRow::Ready(row) = &next {
        if !ends_on_claim(row) && facts.headroom_hold(row).is_some() {
            return PumpStep::Sleep(now + Duration::seconds(MEMORY_RECHECK_SECS));
        }
    }
    // Advisory: used only to SKIP claiming, never to start anything.
    if let Some(holder) = lease {
        let episodes_started = run_queue::start_waiting_episodes(vault, &holder, now)
            .unwrap_or_else(|error| {
                eprintln!("[run queue] could not start wait episodes: {error}");
                Vec::new()
            });
        let (failed, chat) = match refuse_interactive_in(vault, Some(&holder)) {
            Some(message) => fail_interactive(vault, &message, now, pending),
            None => (Vec::new(), Vec::new()),
        };
        return PumpStep::Busy {
            holder,
            episodes_started,
            failed,
            chat,
        };
    }
    let head = match next {
        NextRow::Ready(row) => row,
        NextRow::NotBefore(at) => {
            let at = soonest_hold(&card_holds)
                .map(|hold| hold.min(at))
                .unwrap_or(at);
            return PumpStep::Sleep(at);
        }
        NextRow::Empty => {
            if let Some(at) = soonest_hold(&card_holds) {
                return PumpStep::Sleep(at);
            }
            return PumpStep::Idle;
        }
    };
    // A chat row is marked in the chat-row gate BEFORE it is claimed, so
    // `chat::accept_turn` never repairs a row this step is admitting (and
    // refuses a second turn meanwhile). Dropped when this step returns, unless
    // the turn is admitted and carries it until its row is terminal. An id
    // that is already active is a live turn (A·R4): it is never claimed again.
    let chat_guard = (head.source == RunSource::Chat).then(|| ChatRowGuard::hold(vault, head.id));
    if matches!(chat_guard, Some(None)) {
        return PumpStep::Error(format!("chat row {} is already active", head.id));
    }
    let chat_guard = chat_guard.flatten();
    let row = match run_queue::claim(vault, head.id, now) {
        Ok(Some(row)) => row,
        // Cancelled or claimed meanwhile: look again.
        Ok(None) => {
            return PumpStep::Handled {
                row_id: head.id,
                event: None,
                notice: None,
                chat: Vec::new(),
            }
        }
        Err(error) => return PumpStep::Error(error),
    };
    (deps.after_claim)(vault);
    let verdict = claim_verdict(vault, &row, now, deps);
    settle(vault, row, verdict, now, pending, deps, chat_guard)
}

/// A queued chat row that carries a Stop (C-S3, 11-14): it was claimed when
/// the Stop landed and then went back to `queued` (a lost admission race, a
/// failed write). Its next claim ends it as `cancelled` (`decide_claim`), with
/// no lease or bookkeeping, so it never waits behind the run that held it.
fn ends_on_claim(row: &QueueRow) -> bool {
    row.source == RunSource::Chat && row.cancel_requested
}

/// Another claimed or running chat row for the same conversation. Enqueue
/// allows a person's send beside a card-held wake; this keeps the pump from
/// starting that wake while the person's turn is claimed or running.
fn conversation_busy(live: &[QueueRow], finished: &[i64], row: &QueueRow) -> bool {
    if row.source != RunSource::Chat {
        return false;
    }
    let Some(key) = row.source_id.as_deref() else {
        return false;
    };
    live.iter().any(|other| {
        other.id != row.id
            && other.source == RunSource::Chat
            && other.source_id.as_deref() == Some(key)
            && other.status != RowStatus::Queued
            && !finished.contains(&other.id)
    })
}

fn cached_open_hold(
    vault: &Path,
    key: &str,
    now: DateTime<Utc>,
    cache: &RefCell<HashMap<String, Option<DateTime<Utc>>>>,
) -> Option<DateTime<Utc>> {
    if let Some(found) = cache.borrow().get(key) {
        return *found;
    }
    let found = crate::interactions::open_hold_until(vault, key, now);
    cache.borrow_mut().insert(key.to_string(), found);
    found
}

fn soonest_hold(holds: &RefCell<Vec<DateTime<Utc>>>) -> Option<DateTime<Utc>> {
    holds.borrow().iter().copied().min()
}

/// Whether `row` is a chat turn a workspace's scoped run will admit (PR B3):
/// its working folder is a live build worktree or a workspace's own vault
/// folder (see `chat::run_scope`).
pub(crate) fn scoped_chat(vault: &Path, row: &QueueRow) -> bool {
    chat_scope(vault, row).is_some()
}

/// Live swarm parts in an ordinary workspace, which the swarm's own
/// setting caps (2026-10-07; since 2026-10-20 within the one budget too).
fn swarm_live_of(vault: &Path, live: &[QueueRow]) -> usize {
    live.iter()
        .filter(|r| r.status != RowStatus::Queued)
        .filter(|r| crate::main_agent::is_swarm_part(vault, r.source_id.as_deref()))
        .filter(|r| chat_scope_mode(vault, r).is_some_and(|(_, mode)| !mode.alone()))
        .count()
}

/// L1: the conversation queued chat row `row` starts a subagent of
/// (`chat::ChatTurnPayload::parent`), if any.
pub(crate) fn subagent_parent_of(row: &QueueRow) -> Option<&str> {
    (row.source == RunSource::Chat)
        .then(|| {
            row.payload
                .get("parent")
                .and_then(serde_json::Value::as_str)
        })
        .flatten()
        .filter(|p| !p.is_empty())
}

/// Per parent conversation, how many of its subagents' turns are claimed or
/// running ([`Hold::ParentLimit`]).
fn parent_live_of(live: &[QueueRow]) -> std::collections::HashMap<String, usize> {
    let mut out = std::collections::HashMap::new();
    for row in live.iter().filter(|r| r.status != RowStatus::Queued) {
        if let Some(parent) = subagent_parent_of(row) {
            *out.entry(parent.to_string()).or_insert(0) += 1;
        }
    }
    out
}

/// Why the pump starts no queued row now ([`QueueFacts::hold`]): one rule
/// for the pump ([`step_beside`]) and the row's waiting reason
/// ([`waiting_reason_from`]), checked in this order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Hold {
    /// Memory pressure holds starts beyond the three bootstrap turns.
    Memory,
    /// Free plus inactive memory cannot cover the floor and reservations.
    Headroom,
    /// A live run keeps it out of its folder, by the blocker's own kind:
    /// `true` a turn that needs the folder to itself (a build worktree's),
    /// `false` a shared one (a chat in the workspace's own folder, which
    /// keeps a worktree turn out). `run` is the blocking run's id, which
    /// [`waiting_reason`] turns into its conversation ([`FolderHolder`]).
    Folder {
        exclusive: bool,
        run: Option<String>,
    },
    /// This many chats already share its workspace's folder (Settings).
    WorkspaceLimit(usize),
    /// This many of a swarm's parts already run (the swarm's own setting).
    SwarmLimit(usize),
    /// L1: this many of the same conversation's subagents already run
    /// (Settings, Chat, Chats at once, Subagents per chat,
    /// `chat_limits::ChatLimits::subagents_per_chat`, 6 until the person
    /// changes it); a further start waits for one to end.
    ParentLimit(usize),
    /// This many agent turns already run across Hollow (Settings).
    AppLimit(usize),
    /// A vault-lane row, while another runs there (its label).
    VaultLane(String),
    /// A scoped row would take the app-wide limit's last free slot while a
    /// vault-lane row (its label) has waited [`LANE_FAIRNESS_SECS`] for one
    /// (or is a Run now): that row goes first.
    LaneFirst(String),
}

impl Hold {
    /// The short cause recorded on a held row's phase timings (SE0).
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Hold::Memory => "memory",
            Hold::Headroom => "headroom",
            Hold::Folder { .. } => "folder",
            Hold::WorkspaceLimit(_) => "workspaceLimit",
            Hold::SwarmLimit(_) => "swarmLimit",
            Hold::ParentLimit(_) => "parentLimit",
            Hold::AppLimit(_) => "appLimit",
            Hold::VaultLane(_) => "vaultLane",
            Hold::LaneFirst(_) => "laneFirst",
        }
    }
}

// ── Why a queued row waits (overnight A2) ───────────────────────────────────

/// What the pump's admission rules know about one queued row, for
/// [`waiting_reason_from`]. A lease wait and a permission hold are already
/// on the row (`wait`, `held_reason`) and shown as such. The automation gap
/// (rows at or above `run_queue::GAP_PRIORITY`) is not named: missing, never
/// wrong.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct WaitFacts {
    /// A retry's or deferral's `not_before`, when still ahead.
    pub not_before: Option<DateTime<Utc>>,
    /// What keeps it from starting now ([`QueueFacts::hold`]).
    pub hold: Option<Hold>,
    /// The conversation whose turn holds a build worktree ([`Hold::Folder`]).
    pub holder: Option<FolderHolder>,
}

/// The conversation whose live turn holds a build worktree, for the queued
/// badge's "<title> … · Open it" (P1-46).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FolderHolder {
    pub key: String,
    pub title: String,
    /// Whole minutes since its turn started.
    pub minutes: i64,
    /// It is the queued row's own conversation (`asking`): its previous
    /// turn, which the badge doesn't offer to open.
    pub own: bool,
}

/// [`FolderHolder`] for live run `run_id`, from its turn in the session
/// store; `None` for a run that isn't a recorded chat turn. `asking` is the
/// queued row's own conversation key.
pub(crate) fn folder_holder(
    vault: &Path,
    run_id: &str,
    asking: Option<&str>,
    now: DateTime<Utc>,
) -> Option<FolderHolder> {
    let session = crate::session_store::session_for_run(vault, run_id).ok()??;
    let started = crate::session_store::turn_started_at(vault, &session.key, run_id).ok()??;
    let started = DateTime::parse_from_rfc3339(&started)
        .ok()?
        .with_timezone(&Utc);
    let title = session
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "A chat".to_string());
    Some(FolderHolder {
        own: asking == Some(session.key.as_str()),
        key: session.key,
        title: crate::app_actions::cut(title.trim(), 60),
        minutes: (now - started).num_minutes().max(0),
    })
}

/// What a queued turn is waiting for, phrased to follow "waiting for" (the
/// composer's badge, `queuedBadgeText`), or `None` when nothing holds it
/// back (it is next, or only behind other queued rows). A retry time first,
/// then the pump's own order ([`Hold`]).
pub(crate) fn waiting_reason_from(facts: &WaitFacts) -> Option<String> {
    if let Some(at) = facts.not_before {
        return Some(format!(
            "its retry time ({})",
            at.with_timezone(&Local).format("%H:%M")
        ));
    }
    Some(match facts.hold.as_ref()? {
        Hold::Memory => "memory".to_string(),
        Hold::Headroom => "memory headroom".to_string(),
        Hold::Folder {
            exclusive: true, ..
        } => match &facts.holder {
            Some(h) if h.own => "this conversation's previous turn to finish".to_string(),
            Some(h) => format!(
                "\u{201c}{}\u{201d} to finish (it has used this worktree for {} min)",
                h.title, h.minutes
            ),
            None => "the turn running in this build worktree".to_string(),
        },
        Hold::Folder {
            exclusive: false, ..
        } => "the chats in this workspace's folder to finish".to_string(),
        // Each limit names its number and the setting that holds it, after
        // the "Waiting for" every surface adds (`waitingText`, the queued
        // badge, the phone, `start_session`'s `waitingFor`).
        Hold::WorkspaceLimit(n) => format!(
            "a free chat slot: this workspace is at its limit of {n} chats at once ({})",
            crate::chat_limits::WHERE
        ),
        Hold::SwarmLimit(n) => format!(
            "a free swarm slot: the swarm is at its limit of {n} parts at once (Settings → Background work → Swarm parts at once)"
        ),
        Hold::ParentLimit(n) => format!(
            "a free subagent slot: this chat is at its limit of {n} subagents at once ({})",
            crate::chat_limits::WHERE
        ),
        Hold::AppLimit(n) => format!(
            "a free chat slot: Hollow is at its limit of {n} chats at once ({})",
            crate::chat_limits::WHERE
        ),
        Hold::VaultLane(label) => format!("the vault ({label} is running)"),
        Hold::LaneFirst(label) => format!("{label} to start first"),
    })
}

/// What one read of the queue knows for every row's admission
/// ([`QueueFacts::hold`]): the pump reads it once per step, a list of the
/// queue once per list ([`QueueFacts::read`]).
#[derive(Debug, Default, Clone)]
pub(crate) struct QueueFacts {
    /// Agent turns live across Hollow: scoped chat turns (a workspace
    /// archive's lease is not one) and the vault lane's run.
    live_turns: usize,
    /// Per workspace folder, the live chat turns sharing it.
    live_shared: std::collections::HashMap<String, usize>,
    swarm_live: usize,
    /// L1: per parent conversation, its subagents' live turns.
    parent_live: std::collections::HashMap<String, usize>,
    /// The claimed or running vault-lane row's label, if any.
    lane: Option<String>,
    limits: crate::chat_limits::ChatLimits,
    /// macOS memory pressure, when at warning or worse.
    pressure: Option<i32>,
    /// Unknown on unsupported systems; there the saved count limits apply.
    available_memory: Option<u64>,
    /// Reserve each live turn's estimate as well as its measured footprint:
    /// startup may not have allocated its memory yet when the next row starts.
    reserved_memory: u64,
    /// The label of the queued vault-lane row owed the next free slot
    /// ([`Hold::LaneFirst`]), if one is.
    lane_first: Option<String>,
}

impl QueueFacts {
    fn overall_ceiling(&self) -> usize {
        if self.available_memory.is_some() {
            crate::chat_limits::MAX_AT_ONCE as usize
        } else {
            self.limits.chats_at_once as usize
        }
    }

    fn last_headroom_slot(&self, row: &QueueRow) -> bool {
        self.available_memory.is_some_and(|available| {
            available
                < MEMORY_FLOOR_BYTES
                    .saturating_add(self.reserved_memory)
                    .saturating_add(turn_memory_bytes(row))
                    .saturating_add(crate::agent::turn_memory_bytes(None))
        })
    }

    fn headroom_hold(&self, row: &QueueRow) -> Option<Hold> {
        let available = self.available_memory?;
        let required = MEMORY_FLOOR_BYTES
            .saturating_add(self.reserved_memory)
            .saturating_add(turn_memory_bytes(row));
        (available < required).then_some(Hold::Headroom)
    }

    fn memory_held(&self) -> bool {
        self.pressure.is_some() && self.live_turns >= MEMORY_BOOTSTRAP_TURNS
    }

    /// For a list of the queue: `live` is `run_queue::live_rows`.
    pub(crate) fn read(vault: &Path, live: &[QueueRow]) -> Self {
        Self::read_beside(vault, live, None, &[], Utc::now(), None)
    }

    /// For a pump step: `lane` is the loop's live vault-lane row, and the
    /// rows in `finished` have run already (a terminal write still owed),
    /// so neither counts as a running vault-lane row. `gap_open_at` is when
    /// gap-bound rows may start (a list doesn't know it: `None`).
    pub(crate) fn read_beside(
        vault: &Path,
        live: &[QueueRow],
        lane: Option<i64>,
        finished: &[i64],
        now: DateTime<Utc>,
        gap_open_at: Option<DateTime<Utc>>,
    ) -> Self {
        let active_claims = chat::active_chat_rows(vault);
        let lane_row = live
            .iter()
            .filter(|r| r.status != RowStatus::Queued && !finished.contains(&r.id))
            .filter(|r| {
                !(r.source.is_workflow()
                    && r.status == RowStatus::Claimed
                    && r.history_run_id.is_none()
                    && active_claims.contains(&r.id))
            })
            .find(|r| chat_scope_mode(vault, r).is_none());
        let (scoped, live_shared) = automated_history::scoped_turn_counts(vault);
        let lane_live = lane.is_some() || lane_row.is_some();
        let active: Vec<_> = live
            .iter()
            .filter(|row| row.status != RowStatus::Queued && !finished.contains(&row.id))
            .collect();
        let live_turns = scoped + usize::from(lane_live);
        let reserved_memory = active
            .iter()
            .fold(0u64, |sum, row| sum.saturating_add(turn_memory_bytes(row)))
            .saturating_add(
                (live_turns.saturating_sub(active.len()) as u64)
                    .saturating_mul(crate::agent::turn_memory_bytes(None)),
            );
        #[cfg(not(test))]
        let available_memory = crate::proc_info::available_memory_bytes();
        #[cfg(test)]
        let available_memory = TEST_HEADROOM.with(std::cell::Cell::get);
        Self {
            live_turns,
            available_memory,
            reserved_memory,
            live_shared,
            swarm_live: swarm_live_of(vault, live),
            parent_live: parent_live_of(live),
            lane: lane_row.map(|r| match r.source {
                RunSource::Chat => "a chat in the vault root".to_string(),
                _ => r.label.clone(),
            }),
            limits: crate::chat_limits::current(),
            pressure: crate::run_permit::memory_pressure_level(),
            lane_first: lane_first(vault, live, now, gap_open_at),
        }
    }

    /// Why `row` can't start now, or `None` when it may: the run pump's one
    /// rule. Memory pressure holds rows beyond the bootstrap turns. A scoped turn
    /// waits for a live run that keeps it out of its folder, then (sharing
    /// a workspace's folder) for that workspace's limit, then (a swarm part
    /// in an ordinary workspace) for its swarm's own; a vault-lane row waits
    /// while `lane_busy`. Last, every row waits while the app-wide limit's
    /// worth of agent turns run.
    pub(crate) fn hold(&self, vault: &Path, row: &QueueRow, lane_busy: bool) -> Option<Hold> {
        if self.memory_held() {
            return Some(Hold::Memory);
        }
        if let Some(hold) = self.headroom_hold(row) {
            return Some(hold);
        }
        self.hold_beyond_memory(vault, row, lane_busy)
    }

    /// [`Self::hold`] but for memory pressure (the pump checks that once,
    /// for the whole step).
    fn hold_beyond_memory(&self, vault: &Path, row: &QueueRow, lane_busy: bool) -> Option<Hold> {
        // L1: one conversation runs at most this many subagents at once,
        // wherever they run; the rest wait their turn here.
        if let Some(parent) = subagent_parent_of(row) {
            let cap = self.limits.subagents_per_chat as usize;
            if self.parent_live.get(parent).copied().unwrap_or(0) >= cap {
                return Some(Hold::ParentLimit(cap));
            }
        }
        match chat_scope_mode(vault, row) {
            Some((scope, mode)) => {
                let exclusive = mode.alone();
                if let Some((run, kind)) =
                    automated_history::scope_blocker_kind(vault, &scope, &mode)
                {
                    return Some(Hold::Folder {
                        exclusive: kind,
                        run: Some(run),
                    });
                }
                let per_workspace = self.limits.chats_per_workspace as usize;
                if !exclusive && self.live_shared.get(&scope).copied().unwrap_or(0) >= per_workspace
                {
                    return Some(Hold::WorkspaceLimit(per_workspace));
                }
                let swarm =
                    crate::main_agent::swarm_part_limit(vault, row.source_id.as_deref(), exclusive);
                if let Some(parts) = swarm.filter(|parts| self.swarm_live >= *parts) {
                    return Some(Hold::SwarmLimit(parts));
                }
                // Bounded fairness: the last free slot goes to a vault-lane
                // row that has waited for one, unless the lane is busy (it
                // couldn't start anyway). Only a slot that is free now: at
                // the limit the row simply waits for it (delta review).
                let last_slot =
                    self.live_turns + 1 == self.overall_ceiling() || self.last_headroom_slot(row);
                if let Some(label) = self.lane_first.as_ref() {
                    if last_slot && !lane_busy && self.lane.is_none() {
                        return Some(Hold::LaneFirst(label.clone()));
                    }
                }
            }
            None if lane_busy => {
                return Some(Hold::VaultLane(
                    self.lane.clone().unwrap_or_else(|| "another run".into()),
                ))
            }
            None => {}
        }
        let at_once = self.overall_ceiling();
        if self.live_turns >= at_once {
            return Some(Hold::AppLimit(at_once));
        }
        None
    }
}

/// True when queued `row` can start only after live turn `run_id` ends
/// because of the chat limits alone (2026-10-20 review): it waits for its
/// workspace's limit and `run_id` is the only turn sharing that folder, or
/// for the app-wide limit and `run_id` is the only turn running. A wait on
/// a child the caller's own turn holds out this way answers at once
/// (`descendants::behind_note`) instead of running to its deadline.
pub(crate) fn waits_only_for_slot_of(vault: &Path, row: &QueueRow, run_id: &str) -> bool {
    if row.status != RowStatus::Queued || run_id.is_empty() {
        return false;
    }
    // Occupancy, not the displayed reason (which may name memory, a folder
    // or fairness first): a limit the caller's turn alone fills keeps the
    // row out until that turn ends, whatever else also holds it.
    let live = run_queue::live_rows(vault).unwrap_or_default();
    let facts = QueueFacts::read(vault, &live);
    let own = automated_history::scoped_turn_of(vault, run_id);
    let mine = own.is_some() || automated_history::lease_is_run(vault, run_id);
    let app_full_of_mine = mine && facts.live_turns == 1 && facts.overall_ceiling() <= 1;
    let workspace_full_of_mine = match chat_scope_mode(vault, row) {
        Some((scope, mode)) if !mode.alone() => {
            own.as_ref() == Some(&(scope.clone(), false))
                && facts.live_shared.get(&scope).copied() == Some(1)
                && facts.limits.chats_per_workspace as usize <= 1
        }
        _ => false,
    };
    app_full_of_mine || workspace_full_of_mine
}

/// The queued vault-lane row owed the next free slot, by its label: the
/// first in claim order that is ready now (its retry time and the
/// automation gap passed), held by no permission or skip, and a Run now or
/// waiting [`LANE_FAIRNESS_SECS`] since it became ready; none while any
/// lease is held now. A row's past lease wait episode doesn't matter
/// ([`Hold::LaneFirst`]).
fn lane_first(
    vault: &Path,
    live: &[QueueRow],
    now: DateTime<Utc>,
    gap_open_at: Option<DateTime<Utc>>,
) -> Option<String> {
    let stamp = |at: &str| {
        DateTime::parse_from_rfc3339(at.trim())
            .ok()
            .map(|at| at.with_timezone(&Utc))
    };
    // A lease held now (tracker sync, an ingest, another process) keeps any
    // vault-lane row out whatever its wait says; a row's wait episode is
    // only the record of an earlier wait, never a reason on its own.
    if automated_history::lease_holder(vault).is_some() {
        return None;
    }
    let gap = gap_open_at.filter(|open| *open > now);
    live.iter()
        .filter(|r| r.status == RowStatus::Queued)
        .filter(|r| r.held_reason.is_none() && !r.cancel_requested)
        .filter(|r| r.priority < run_queue::GAP_PRIORITY || gap.is_none())
        .filter(|r| {
            let not_before = r.not_before.as_deref().and_then(stamp);
            if not_before.is_some_and(|at| at > now) {
                return false;
            }
            // Ready since the later of its request, its retry time and (a
            // gap-bound row) the automation gap opening.
            let gap_opened = gap_open_at.filter(|_| r.priority >= run_queue::GAP_PRIORITY);
            let ready_since = stamp(&r.requested_at).max(not_before).max(gap_opened);
            r.source == RunSource::Manual
                || ready_since.is_some_and(|at| now - at >= Duration::seconds(LANE_FAIRNESS_SECS))
        })
        // `live_rows` already uses the shared trusted-origin order. Re-sorting
        // by stored priority here would owe the slot to a different row.
        .find(|r| chat_scope_mode(vault, r).is_none())
        .map(|r| match r.source {
            RunSource::Chat => "a chat in the vault root".to_string(),
            _ => r.label.clone(),
        })
}

/// Why a queued non-person chat row is waiting on an open question card.
/// The webview matches this exact string and does not adopt the row.
pub(crate) const CARD_HOLD_WAIT: &str = "an open question card";

/// What one live row's wait says for every surface (2026-11-16, Working,
/// not Queued): the reason a queued row truly can't start, the conversation
/// it names (P1-46), and whether the row reads Working instead.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Waiting {
    /// Why a queued row can't start now ([`waiting_reason_from`]).
    pub(crate) reason: Option<String>,
    /// The conversation holding the build worktree, when `reason` names it.
    pub(crate) session: Option<String>,
    /// The chat row reads Working ("Preparing"), not Queued: nothing truly
    /// holds it back. See [`RunQueueItem::working`].
    pub(crate) working: bool,
}

/// True for a lease holder that is the app's own short bookkeeping, not a
/// run: the tracker sync's write phase, the scheduler, a workspace edit, a
/// companion device's change, a history clear. A chat row waiting for one
/// reads Working ("Preparing"), never Queued (2026-11-16); a real run, or
/// a stale lease only a restart clears, is still named. `text` is
/// [`LeaseHolder::describe`], which is what a wait episode stores.
pub(crate) fn is_internal_holder_text(text: &str) -> bool {
    use crate::automated_history::LeaseHolder as H;
    [
        H::TrackerSync,
        H::Scheduler,
        H::WorkspaceEdit,
        H::Companion,
        H::HistoryClear,
    ]
    .iter()
    .any(|holder| holder.describe() == text)
}

/// A chat row that is claimed or running reads Working.
fn chat_working_status(row: &QueueRow) -> bool {
    row.source == RunSource::Chat && matches!(row.status, RowStatus::Claimed | RowStatus::Running)
}

/// [`Waiting`] for `row` now. A queued chat row has a `reason` only when it
/// truly can't be claimed (a chat-count limit, memory, the folder or vault
/// lane another run holds, a retry time, an open card, or another process
/// running the queue: [`OWNED_ELSEWHERE_WAIT`]); an internal lease
/// wait ([`is_internal_holder_text`]) or no hold at all means it is
/// `working`. Workflow rows keep their lease wait and never read `working`
/// while queued.
fn waiting(vault: &Path, row: &QueueRow, facts: &QueueFacts, now: DateTime<Utc>) -> Waiting {
    if row.status != RowStatus::Queued {
        return Waiting {
            working: chat_working_status(row),
            ..Waiting::default()
        };
    }
    let chat = row.source == RunSource::Chat;
    // A wake held for an open card is not the person's queued message. The
    // webview matches this string and leaves the composer free so the card
    // can be answered. Lease bookkeeping can set `row.wait`, and a
    // permission hold can set `held_reason`, while the card still holds the
    // wake, so this comes before those early returns. A person row (payload
    // closes cards) keeps its other reason.
    if chat && !crate::run_queue::payload_closes_cards(&row.payload) {
        if let Some(key) = row.source_id.as_deref() {
            if crate::interactions::open_hold_until(vault, key, now).is_some() {
                return Waiting {
                    reason: Some(CARD_HOLD_WAIT.to_string()),
                    ..Waiting::default()
                };
            }
        }
    }
    // A lease wait names its holder and a permission hold its note, and the
    // row shows those. Only a chat row's wait on the app's own bookkeeping
    // is set aside: it reads Working, or its limit below.
    let own_wait = row
        .wait
        .as_ref()
        .is_some_and(|w| !chat || !is_internal_holder_text(&w.holder));
    if own_wait || row.held_reason.is_some() {
        return Waiting::default();
    }
    // Only the process that runs the queue claims a row: anywhere else a
    // queued row waits for that process, never reads Working (2026-11-17).
    let elsewhere = match queue_state_here() {
        QueueState::OwnedElsewhere => Some(OWNED_ELSEWHERE_WAIT),
        QueueState::NotStarted => Some(QUEUE_NOT_STARTED_WAIT),
        QueueState::Recovering | QueueState::Owned => None,
    };
    if let Some(wait) = elsewhere {
        return Waiting {
            reason: Some(wait.to_string()),
            ..Waiting::default()
        };
    }
    let not_before = row
        .not_before
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc))
        .filter(|at| *at > now);
    let hold = facts.hold(vault, row, facts.lane.is_some());
    let holder = match &hold {
        Some(Hold::Folder {
            exclusive: true,
            run: Some(run),
        }) => folder_holder(vault, run, row.source_id.as_deref(), now),
        _ => None,
    };
    let session = holder
        .as_ref()
        .filter(|h| not_before.is_none() && !h.own)
        .map(|h| h.key.clone());
    let reason = waiting_reason_from(&WaitFacts {
        not_before,
        hold,
        holder,
    });
    Waiting {
        working: chat && reason.is_none(),
        reason,
        session,
    }
}

/// [`waiting_reason_from`] for `row` now. `None` for a row that isn't
/// queued, or whose lease wait or permission hold already says why. An
/// open card still names itself when that wait or hold is also set.
#[cfg(test)]
pub(crate) fn waiting_reason(
    vault: &Path,
    row: &QueueRow,
    facts: &QueueFacts,
    now: DateTime<Utc>,
) -> Option<String> {
    waiting(vault, row, facts, now).reason
}

/// The status word an agent reads for a live chat row (`wait_for_session`,
/// `sessions.overview` `mine`): the queue's own word, except that a queued row
/// that reads Working (nothing truly holds it back) is `claimed`, the word
/// for a turn Hollow is preparing, never `queued` (2026-11-16).
pub(crate) fn agent_status(vault: &Path, row: &QueueRow) -> &'static str {
    if row.status == RowStatus::Queued && row.source == RunSource::Chat {
        // An unreadable queue keeps the row's own word: never claim it works.
        if let Ok(live) = run_queue::live_rows(vault) {
            let facts = QueueFacts::read(vault, &live);
            if item_with_reason(vault, row, None, &facts, Utc::now()).reads_working() {
                return RowStatus::Claimed.as_str();
            }
        }
    }
    row.status.as_str()
}

/// [`run_queue::to_item`] with [`waiting_reason`] filled in.
pub(crate) fn item_with_reason(
    vault: &Path,
    row: &QueueRow,
    position: Option<u32>,
    facts: &QueueFacts,
    now: DateTime<Utc>,
) -> RunQueueItem {
    let mut item = run_queue::to_item(row, position);
    let waiting = waiting(vault, row, facts, now);
    if waiting.working {
        item.working = Some(true);
    }
    (item.waiting_reason, item.waiting_session) = (waiting.reason, waiting.session);
    item
}

#[cfg(test)]
mod waiting_reason_tests {
    use super::*;
    use crate::run_queue::{claim, enqueue, mark_running, NewRow};

    /// L1: one conversation runs at most six subagents at once; the seventh
    /// start waits for one to end, then may start. Another parent's and a
    /// message's rows are never held for it.
    #[test]
    fn the_seventh_subagent_of_one_parent_waits_for_one_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let past = Utc::now() - Duration::minutes(5);
        let row = |key: &str, parent: Option<&str>| {
            let mut payload = serde_json::json!({"prompt": "go", "agent": "claude"});
            if let Some(p) = parent {
                payload["parent"] = serde_json::json!(p);
            }
            NewRow::new(RunSource::Chat, Some(key), "chat", "test").with_payload(payload)
        };
        let mut running = Vec::new();
        for i in 0..6 {
            let id = enqueue(
                v,
                row(
                    &format!("s-20261110-sub0000{i}"),
                    Some("s-20261110-coo00001"),
                ),
                past,
            )
            .unwrap()
            .id();
            claim(v, id, past).unwrap();
            mark_running(v, id, &format!("run-sub-{i}"), past).unwrap();
            running.push(id);
        }
        let seventh = enqueue(
            v,
            row("s-20261110-sub00007", Some("s-20261110-coo00001")),
            past,
        )
        .unwrap()
        .id();
        let other = enqueue(
            v,
            row("s-20261110-oth00001", Some("s-20261110-lead0001")),
            past,
        )
        .unwrap()
        .id();
        let message = enqueue(v, row("s-20261110-msg00001", None), past)
            .unwrap()
            .id();
        let live = run_queue::live_rows(v).unwrap();
        let facts = QueueFacts::read(v, &live);
        let get = |id| live.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!(
            facts.hold_beyond_memory(v, &get(seventh), false),
            Some(Hold::ParentLimit(6))
        );
        assert_ne!(
            facts.hold_beyond_memory(v, &get(other), false),
            Some(Hold::ParentLimit(6))
        );
        assert_ne!(
            facts.hold_beyond_memory(v, &get(message), false),
            Some(Hold::ParentLimit(6))
        );
        // One ends: the seventh may start.
        run_queue::finish(v, running[0], RowStatus::Done, None, Utc::now()).unwrap();
        let live = run_queue::live_rows(v).unwrap();
        let facts = QueueFacts::read(v, &live);
        let seventh_row = live.iter().find(|r| r.id == seventh).unwrap();
        assert_ne!(
            facts.hold_beyond_memory(v, seventh_row, false),
            Some(Hold::ParentLimit(6))
        );
    }

    /// 2026-11-12: the per-chat subagent cap is Settings' "Subagents per
    /// chat", not a fixed six. At 2 the third start waits, and its reason
    /// names the number and the setting; at 8 the seventh starts.
    #[test]
    fn the_subagent_cap_follows_the_setting_and_the_reason_names_it() {
        use crate::chat_limits::{set_test_limits, ChatLimits};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let past = Utc::now() - Duration::minutes(5);
        let row = |key: &str| {
            let payload =
                serde_json::json!({"prompt": "go", "agent": "claude", "parent": "s-parent"});
            NewRow::new(RunSource::Chat, Some(key), "chat", "test").with_payload(payload)
        };
        for i in 0..2 {
            let id = enqueue(v, row(&format!("s-20261112-sub0000{i}")), past)
                .unwrap()
                .id();
            claim(v, id, past).unwrap();
            mark_running(v, id, &format!("run-cap-{i}"), past).unwrap();
        }
        let third = enqueue(v, row("s-20261112-sub00003"), past).unwrap().id();
        let hold_of_third = || {
            let live = run_queue::live_rows(v).unwrap();
            let facts = QueueFacts::read(v, &live);
            let third_row = live.iter().find(|r| r.id == third).unwrap();
            facts.hold_beyond_memory(v, third_row, false)
        };
        set_test_limits(ChatLimits {
            subagents_per_chat: 2,
            ..ChatLimits::default()
        });
        let hold = hold_of_third();
        assert_eq!(hold, Some(Hold::ParentLimit(2)));
        assert_eq!(
            waiting_reason_from(&WaitFacts {
                hold,
                ..WaitFacts::default()
            })
            .as_deref(),
            Some(
                "a free subagent slot: this chat is at its limit of 2 subagents at once (Settings → Chat → Chats at once)"
            )
        );
        set_test_limits(ChatLimits {
            subagents_per_chat: 3,
            ..ChatLimits::default()
        });
        assert_eq!(hold_of_third(), None, "one more allowed: it may start");
        set_test_limits(ChatLimits::default());
        assert_eq!(hold_of_third(), None, "the default is still 6");
    }

    #[test]
    fn each_admission_rule_names_what_the_turn_waits_for() {
        let none = WaitFacts::default();
        assert_eq!(
            waiting_reason_from(&none),
            None,
            "next in line: nothing holds it"
        );
        let held = |hold| WaitFacts {
            hold: Some(hold),
            ..WaitFacts::default()
        };
        let cases = [
            (Hold::Memory, "memory"),
            (
                Hold::Folder {
                    exclusive: true,
                    run: None,
                },
                "the turn running in this build worktree",
            ),
            (
                Hold::Folder {
                    exclusive: false,
                    run: None,
                },
                "the chats in this workspace's folder to finish",
            ),
            (
                Hold::WorkspaceLimit(5),
                "a free chat slot: this workspace is at its limit of 5 chats at once (Settings → Chat → Chats at once)",
            ),
            (
                Hold::SwarmLimit(6),
                "a free swarm slot: the swarm is at its limit of 6 parts at once (Settings → Background work → Swarm parts at once)",
            ),
            (
                Hold::ParentLimit(6),
                "a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)",
            ),
            (
                Hold::AppLimit(10),
                "a free chat slot: Hollow is at its limit of 10 chats at once (Settings → Chat → Chats at once)",
            ),
            (
                Hold::LaneFirst("Morning brief".into()),
                "Morning brief to start first",
            ),
            (
                Hold::VaultLane("Morning brief".into()),
                "the vault (Morning brief is running)",
            ),
        ];
        for (hold, text) in cases {
            assert_eq!(waiting_reason_from(&held(hold)).as_deref(), Some(text));
        }
        let retry = WaitFacts {
            not_before: Some(Utc::now() + Duration::minutes(5)),
            ..held(Hold::Memory)
        };
        assert!(waiting_reason_from(&retry)
            .unwrap()
            .starts_with("its retry time ("));
    }

    /// P1-46 (BUILD-10): a held build worktree names the conversation whose
    /// turn holds it and for how long, and keeps its key for "Open it".
    #[test]
    fn a_held_worktree_names_its_holder_and_how_long() {
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        crate::session_store::start_turn(
            v,
            "s-20261025-hold0001",
            "claude",
            "/w",
            "run-holder",
            "Fix the build panel",
        )
        .unwrap();
        let holder = folder_holder(
            v,
            "run-holder",
            Some("s-20261025-other001"),
            Utc::now() + Duration::minutes(12),
        )
        .unwrap();
        assert!(!holder.own);
        assert_eq!(holder.key, "s-20261025-hold0001");
        assert_eq!(holder.minutes, 12);
        assert!(holder.title.contains("Fix the build panel"), "{holder:?}");
        let facts = WaitFacts {
            hold: Some(Hold::Folder {
                exclusive: true,
                run: Some("run-holder".into()),
            }),
            holder: Some(holder.clone()),
            ..WaitFacts::default()
        };
        assert_eq!(
            waiting_reason_from(&facts),
            Some(format!(
                "\u{201c}{}\u{201d} to finish (it has used this worktree for 12 min)",
                holder.title
            ))
        );
        assert!(folder_holder(v, "run-unknown", None, Utc::now()).is_none());
        // Review: a follow-up queued behind its own conversation's turn is
        // not told to open the conversation it is already in.
        let own = folder_holder(v, "run-holder", Some("s-20261025-hold0001"), Utc::now()).unwrap();
        assert!(own.own);
        let facts = WaitFacts {
            holder: Some(own),
            ..facts
        };
        assert_eq!(
            waiting_reason_from(&facts).as_deref(),
            Some("this conversation's previous turn to finish")
        );
    }

    /// From the queue itself: a vault-root chat queued while an automation
    /// runs on the vault lane names it; the running row and a row with a
    /// lease wait get no extra reason.
    #[test]
    fn a_vault_root_chat_names_the_automation_it_waits_for() {
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let running = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("brief"), "Morning brief", "run now"),
            now,
        )
        .unwrap()
        .id();
        claim(v, running, now).unwrap().expect("claimed");
        mark_running(v, running, "run-brief", now).unwrap();
        let chat = enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20261010-kkkk0001"), "Chat", "chat")
                .with_payload(serde_json::json!({"cwd": v.to_string_lossy()})),
            now,
        )
        .unwrap()
        .id();
        let live = run_queue::live_rows(v).unwrap();
        let facts = QueueFacts::read(v, &live);
        let row = |id| live.iter().find(|r| r.id == id).unwrap().clone();
        assert_eq!(
            waiting_reason(v, &row(chat), &facts, now).as_deref(),
            Some("the vault (Morning brief is running)")
        );
        assert_eq!(waiting_reason(v, &row(running), &facts, now), None);
        let item = item_with_reason(v, &row(chat), Some(1), &facts, now);
        assert_eq!(
            item.waiting_reason.as_deref(),
            Some("the vault (Morning brief is running)")
        );
    }

    // ── Working, not Queued (2026-11-16) ────────────────────────────────────

    /// A scoped chat row in workspace `w1`'s folder, queued now.
    fn chat_in_workspace(v: &Path, key: &str, now: DateTime<Utc>) -> i64 {
        let folder = v.join("workspaces").join("w1");
        std::fs::create_dir_all(&folder).unwrap();
        enqueue(
            v,
            NewRow::new(RunSource::Chat, Some(key), "Chat", "chat")
                .with_payload(serde_json::json!({"cwd": folder.to_string_lossy()})),
            now,
        )
        .unwrap()
        .id()
    }

    fn item_of(v: &Path, id: i64, facts: &QueueFacts, now: DateTime<Utc>) -> RunQueueItem {
        let row = run_queue::get(v, id).unwrap().unwrap();
        item_with_reason(v, &row, Some(1), facts, now)
    }

    /// The pump's facts for the open queue (tests read no memory pressure).
    fn open_facts(v: &Path) -> QueueFacts {
        QueueFacts::read(v, &run_queue::live_rows(v).unwrap())
    }

    /// A claimed row is Working the moment the pump claims it, and a queued
    /// chat row nothing holds back (the pump is about to claim it) reads
    /// Working too: no waiting reason, no badge.
    #[test]
    fn a_claimed_or_claimable_chat_row_reads_working() {
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let id = chat_in_workspace(v, "s-20261116-work0001", now);
        let item = item_of(v, id, &open_facts(v), now);
        assert_eq!(item.status, "queued");
        assert!(item.reads_working(), "claimable: nothing truly holds it");
        assert_eq!(item.waiting_reason, None);
        claim(v, id, now).unwrap().expect("claimed");
        let item = item_of(v, id, &open_facts(v), now);
        assert_eq!(item.status, "claimed");
        assert!(item.reads_working(), "claimed reads Working");
        assert_eq!(item.waiting_reason, None);
    }

    /// #1499 leftover (2026-11-17): a queued chat row in a process that
    /// doesn't run this vault's queue is never claimed here, so it reads
    /// Queued, waiting for the process that does, not Working. A claimed
    /// row is being prepared by its owner and stays Working.
    #[test]
    fn a_queued_row_this_process_does_not_own_reads_queued() {
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let id = chat_in_workspace(v, "s-20261117-else0001", now);
        let read_in = |state: QueueState| {
            TEST_QUEUE_STATE.with(|s| s.set(Some(state)));
            let item = item_of(v, id, &open_facts(v), now);
            TEST_QUEUE_STATE.with(|s| s.set(None));
            item
        };
        let item = read_in(QueueState::OwnedElsewhere);
        assert!(!item.reads_working(), "another process owns it: Queued");
        assert_eq!(item.waiting_reason.as_deref(), Some(OWNED_ELSEWHERE_WAIT));
        let item = read_in(QueueState::NotStarted);
        assert!(!item.reads_working());
        assert_eq!(item.waiting_reason.as_deref(), Some(QUEUE_NOT_STARTED_WAIT));
        // This process owns it (recovering or running): #1499's Working.
        assert!(read_in(QueueState::Recovering).reads_working());
        assert!(read_in(QueueState::Owned).reads_working());
        claim(v, id, now).unwrap().expect("claimed");
        assert!(read_in(QueueState::OwnedElsewhere).reads_working());
    }

    /// A chat row the tracker sync's write phase holds is claimed, bounced
    /// back and shown as Working ("Preparing"): its holder is not named in
    /// the item, which the chat badge, sidebar and lists read. A real run's
    /// lease is still named and the row stays Queued.
    #[test]
    fn a_tracker_sync_hold_reads_working_but_a_real_run_still_queues() {
        use crate::automated_history::LeaseHolder;
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let id = enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20261116-sync0001"), "Chat", "chat")
                .with_payload(serde_json::json!({"cwd": v.to_string_lossy()})),
            now,
        )
        .unwrap()
        .id();
        claim(v, id, now).unwrap().expect("claimed");
        run_queue::requeue_waiting(v, id, &LeaseHolder::TrackerSync, now).unwrap();
        let row = run_queue::get(v, id).unwrap().unwrap();
        assert!(row.wait.is_some(), "the wait episode is still recorded");
        let item = item_of(v, id, &open_facts(v), now);
        assert_eq!(item.status, "queued");
        assert_eq!(item.waiting_holder, None, "not shown as a hold");
        assert_eq!(item.waiting_reason, None);
        assert!(item.reads_working(), "Working, Preparing");
        // The event path (no facts) hides it the same way.
        assert_eq!(run_queue::to_item(&row, None).waiting_holder, None);

        // Another run's lease on a vault-root chat is a real wait.
        let id2 = enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20261116-sync0002"), "Chat", "chat")
                .with_payload(serde_json::json!({"cwd": v.to_string_lossy()})),
            now,
        )
        .unwrap()
        .id();
        claim(v, id2, now).unwrap().expect("claimed");
        let run = LeaseHolder::Run {
            id: "run-x".into(),
            source: Some("workflow".into()),
            source_id: Some("morning-brief".into()),
            started_at: None,
        };
        run_queue::requeue_waiting(v, id2, &run, now).unwrap();
        let item = item_of(v, id2, &open_facts(v), now);
        assert!(!item.reads_working(), "a real run holds it: Queued");
        assert_eq!(
            item.waiting_holder.as_deref(),
            Some("workflow `morning-brief`")
        );
    }

    /// The memory gate, the app-wide limit and the workspace's limit are real
    /// waits: the row is Queued with that reason, never working. A tracker
    /// sync wait on top of a limit still names the limit, not the sync.
    #[test]
    fn each_chat_count_limit_and_the_memory_gate_stay_queued_with_their_reason() {
        use crate::automated_history::LeaseHolder;
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let id = chat_in_workspace(v, "s-20261116-lim00001", now);
        let mut facts = open_facts(v);

        facts.pressure = Some(2);
        facts.live_turns = MEMORY_BOOTSTRAP_TURNS;
        let item = item_of(v, id, &facts, now);
        assert!(!item.reads_working());
        assert_eq!(item.waiting_reason.as_deref(), Some("memory"));
        facts.pressure = None;
        facts.live_turns = 0;

        facts.live_shared.insert(
            "workspaces/w1".to_string(),
            facts.limits.chats_per_workspace as usize,
        );
        let item = item_of(v, id, &facts, now);
        assert!(!item.reads_working());
        assert!(
            item.waiting_reason
                .as_deref()
                .is_some_and(|r| r.starts_with("a free chat slot: this workspace is at its limit")),
            "{:?}",
            item.waiting_reason
        );
        facts.live_shared.clear();

        facts.live_turns = facts.limits.chats_at_once as usize;
        let item = item_of(v, id, &facts, now);
        assert!(!item.reads_working());
        assert!(
            item.waiting_reason
                .as_deref()
                .is_some_and(|r| r.starts_with("a free chat slot: Hollow is at its limit")),
            "{:?}",
            item.waiting_reason
        );

        // The same limit with a tracker sync wait recorded: the limit shows.
        claim(v, id, now).unwrap().expect("claimed");
        run_queue::requeue_waiting(v, id, &LeaseHolder::TrackerSync, now).unwrap();
        let item = item_of(v, id, &facts, now);
        assert!(!item.reads_working());
        assert_eq!(item.waiting_holder, None);
        assert!(item
            .waiting_reason
            .as_deref()
            .is_some_and(|r| r.contains("Hollow is at its limit")));
    }

    /// One conversation's subagent cap and a swarm's part cap are real waits.
    #[test]
    fn the_subagent_and_swarm_limits_stay_queued() {
        use crate::chat_limits::{set_test_limits, ChatLimits};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let past = Utc::now() - Duration::minutes(5);
        let payload = serde_json::json!({"prompt": "go", "agent": "claude", "parent": "s-parent"});
        let running = enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20261116-sub00000"), "chat", "t")
                .with_payload(payload.clone()),
            past,
        )
        .unwrap()
        .id();
        claim(v, running, past).unwrap();
        mark_running(v, running, "run-sub-0", past).unwrap();
        let second = enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20261116-sub00001"), "chat", "t")
                .with_payload(payload),
            past,
        )
        .unwrap()
        .id();
        set_test_limits(ChatLimits {
            subagents_per_chat: 1,
            ..ChatLimits::default()
        });
        let item = item_of(v, second, &open_facts(v), Utc::now());
        set_test_limits(ChatLimits::default());
        assert!(!item.reads_working());
        assert!(item
            .waiting_reason
            .as_deref()
            .is_some_and(|r| r.starts_with("a free subagent slot")));
        // A swarm's cap reads through the same mapping (`Hold::SwarmLimit`).
        let swarm = WaitFacts {
            hold: Some(Hold::SwarmLimit(3)),
            ..WaitFacts::default()
        };
        assert!(waiting_reason_from(&swarm).is_some());
    }

    /// The word agents read (`wait_for_session`, `sessions.overview` `mine`):
    /// a queued row nothing holds back is `claimed`; a row behind another
    /// run's vault lane stays `queued`.
    #[test]
    fn the_agent_status_word_says_claimed_for_a_working_row_and_queued_for_a_real_wait() {
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let id = chat_in_workspace(v, "s-20261116-word0001", now);
        let row = run_queue::get(v, id).unwrap().unwrap();
        assert_eq!(agent_status(v, &row), "claimed");
        claim(v, id, now).unwrap().expect("claimed");
        let row = run_queue::get(v, id).unwrap().unwrap();
        assert_eq!(agent_status(v, &row), "claimed");
        // A vault-root chat behind a running automation truly waits.
        let blocker = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("brief"), "Morning brief", "run now"),
            now,
        )
        .unwrap()
        .id();
        claim(v, blocker, now).unwrap().expect("claimed");
        mark_running(v, blocker, "run-brief", now).unwrap();
        let behind = enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20261116-word0002"), "Chat", "chat")
                .with_payload(serde_json::json!({"cwd": v.to_string_lossy()})),
            now,
        )
        .unwrap()
        .id();
        let row = run_queue::get(v, behind).unwrap().unwrap();
        assert_eq!(agent_status(v, &row), "queued");
    }

    /// Only the app's own bookkeeping holders are hidden; a run, a stale
    /// lease and anything unknown are still named.
    #[test]
    fn only_internal_lease_holders_are_set_aside() {
        use crate::automated_history::LeaseHolder as H;
        for holder in [
            H::TrackerSync,
            H::Scheduler,
            H::WorkspaceEdit,
            H::Companion,
            H::HistoryClear,
        ] {
            assert!(is_internal_holder_text(&holder.describe()), "{holder:?}");
        }
        for holder in [
            H::Unknown,
            H::Stale { id: "run-1".into() },
            H::Run {
                id: "run-2".into(),
                source: Some("workflow".into()),
                source_id: None,
                started_at: None,
            },
        ] {
            assert!(!is_internal_holder_text(&holder.describe()), "{holder:?}");
        }
    }
}

/// The folder a scoped chat row will hold, if it is one.
pub(crate) fn chat_scope(vault: &Path, row: &QueueRow) -> Option<String> {
    chat_scope_mode(vault, row).map(|(scope, _)| scope)
}

/// [`chat_scope`], and how the row's turn is admitted there (see
/// `chat::run_scope_mode`).
pub(crate) fn chat_scope_mode(vault: &Path, row: &QueueRow) -> Option<(String, ScopeMode)> {
    if row.source != RunSource::Chat {
        return None;
    }
    let cwd = row.payload.get("cwd").and_then(serde_json::Value::as_str)?;
    chat::run_scope_mode(vault, Path::new(cwd))
}

/// A past-grace cron row queued beside `row` for the same workflow, when
/// `row` (a trigger, manual or retry row) is about to be admitted. Admitting
/// `row` first would absorb that occurrence as `coalesced` and `finalize` would
/// advance `next_run_at` past it, so its misfire or contention note and its
/// skip notification would be lost; PR 2 always kept that note even with a
/// trigger fire waiting alongside.
fn stale_cron_sibling(
    vault: &Path,
    row: &QueueRow,
    workflow: &Workflow,
    now: DateTime<Utc>,
) -> Option<i64> {
    if row.source == RunSource::Cron {
        return None;
    }
    let due = workflow.next_run_at.as_deref()?;
    let rows = run_queue::live_rows(vault).ok()?;
    rows.into_iter()
        .find(|sibling| {
            sibling.id != row.id
                && sibling.status == RowStatus::Queued
                && sibling.source == RunSource::Cron
                // A sibling deferred after a failed bookkeeping write is left
                // to be absorbed; claiming it again at once would spin.
                && sibling
                    .not_before
                    .as_deref()
                    .and_then(parse_stamp)
                    .is_none_or(|at| at <= now)
                && sibling.source_id.as_deref() == Some(workflow.slug.as_str())
                && run_queue::due_at(&sibling.payload).as_deref() == Some(due)
                && past_grace(workflow, due, now).is_some()
        })
        .map(|sibling| sibling.id)
}

/// Carry out a claimed row's verdict. `chat_guard` is the chat-row gate the
/// step took for a chat row; only an admitted turn keeps it.
#[allow(clippy::too_many_arguments)]
fn settle(
    vault: &Path,
    row: QueueRow,
    verdict: ClaimVerdict,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
    chat_guard: Option<ChatRowGuard>,
) -> PumpStep {
    match verdict {
        ClaimVerdict::Skip {
            why,
            bookkeeping,
            notice,
        } => {
            let chat = chat_end(&row, QueuedEnd::Failed(why.clone()));
            end_with_bookkeeping(
                vault,
                &row,
                bookkeeping,
                RowStatus::Skipped,
                &why,
                notice,
                now,
                pending,
                deps,
                chat,
            )
        }
        ClaimVerdict::Cancel { bookkeeping } => {
            let chat = chat_end(&row, QueuedEnd::Cancelled);
            end_with_bookkeeping(
                vault,
                &row,
                bookkeeping,
                RowStatus::Cancelled,
                run_queue::CANCELLED_BEFORE_START,
                None,
                now,
                pending,
                deps,
                chat,
            )
        }
        ClaimVerdict::Defer {
            not_before,
            held,
            clear_wait,
        } => match run_queue::defer(vault, row.id, not_before, held.as_deref(), clear_wait, None) {
            Ok(_) => PumpStep::Handled {
                row_id: row.id,
                event: Some(if held.is_some() { "held" } else { "requeued" }),
                notice: None,
                chat: Vec::new(),
            },
            Err(error) => PumpStep::Error(error),
        },
        ClaimVerdict::Fail { error } => {
            let chat = chat_end(&row, QueuedEnd::Failed(error.clone()));
            end_row(vault, row.id, RowStatus::Failed, Some(&error), now, pending);
            PumpStep::Handled {
                row_id: row.id,
                event: Some("finished"),
                notice: None,
                chat,
            }
        }
        ClaimVerdict::RunWorkflow {
            runnable,
            waited,
            manual,
            budget,
        } => {
            if let Some(sibling) = stale_cron_sibling(vault, &row, runnable.workflow(), now) {
                // Let the stale occurrence write its own skip row first: put
                // this row back (same id, so it is next), claim the sibling
                // and settle it. The next step admits this row.
                if let Err(error) = run_queue::defer(vault, row.id, now, None, false, None) {
                    return PumpStep::Error(error);
                }
                return match run_queue::claim(vault, sibling, now) {
                    Ok(Some(cron)) => {
                        let verdict = claim_verdict(vault, &cron, now, deps);
                        settle(vault, cron, verdict, now, pending, deps, None)
                    }
                    Ok(None) => PumpStep::Handled {
                        row_id: row.id,
                        event: None,
                        notice: None,
                        chat: Vec::new(),
                    },
                    Err(error) => PumpStep::Error(error),
                };
            }
            admit_workflow(
                vault, row, runnable, waited, manual, budget, now, pending, deps,
            )
        }
        ClaimVerdict::IdleSkip {
            slug,
            title,
            skip,
            due,
            notify,
        } => idle_skip(
            vault,
            &row,
            &slug,
            &title,
            &skip,
            due.as_deref(),
            notify,
            now,
            pending,
            deps,
        ),
        ClaimVerdict::EndRetry { status, reason } => {
            let error = if status == RowStatus::Cancelled {
                run_queue::CANCELLED_BEFORE_START.to_string()
            } else {
                reason.row_error()
            };
            end_row(vault, row.id, status, Some(&error), now, pending);
            if let Some(slug) = row.source_id.as_deref() {
                workflow_retry::resolve_skipped_retry(vault, slug, &row.payload, reason);
            }
            PumpStep::Handled {
                row_id: row.id,
                event: Some(if status == RowStatus::Cancelled {
                    "cancelled"
                } else {
                    "finished"
                }),
                notice: None,
                chat: Vec::new(),
            }
        }
        ClaimVerdict::RunHeartbeat => admit_heartbeat(vault, row, now, pending, deps),
        ClaimVerdict::RunChat => match chat_guard.or_else(|| ChatRowGuard::hold(vault, row.id)) {
            Some(guard) => admit_chat(vault, row, guard, now, pending, deps),
            // Its turn is live elsewhere (A·R4): never admitted twice. The
            // claim stays put; an active row is never released as stray.
            None => PumpStep::Error(format!("chat row {} is already active", row.id)),
        },
    }
}

/// End a chat row whose turn never started, with its `chat-event`s. The
/// gate (held by the caller) is released after this returns.
fn end_unstarted_chat(
    vault: &Path,
    row: &QueueRow,
    status: RowStatus,
    end: QueuedEnd,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
) -> PumpStep {
    let (error, event) = match &end {
        QueuedEnd::Cancelled => (run_queue::CANCELLED_BEFORE_START.to_string(), "cancelled"),
        QueuedEnd::Failed(message) | QueuedEnd::Recovered(message) => (message.clone(), "finished"),
    };
    let chat = chat_end(row, end);
    end_row(vault, row.id, status, Some(&error), now, pending);
    PumpStep::Handled {
        row_id: row.id,
        event: Some(event),
        notice: None,
        chat,
    }
}

/// Admit a claimed chat row (Paperclip PR 7b) through `chat::admit_turn`,
/// the chat path's only lease call. A failed `mark_running` (a busy
/// database) ends the admitted turn unstarted and queues the row again
/// (merged plan slice 3, A·R4): the turn runs on a later step, never twice.
/// The gate is held until that requeue is written. The cancel flag is read
/// only after `mark_running`, as for a workflow: a cancel either saw
/// `running` and stopped the turn's (already registered) slot, or set the
/// flag read here, and then the turn ends before it starts (no
/// `runStarted`, no turn row) with `cancelled{queued}` and `done`.
fn admit_chat(
    vault: &Path,
    row: QueueRow,
    guard: ChatRowGuard,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
    deps: &StepDeps<'_>,
) -> PumpStep {
    (deps.before_admit)(vault);
    match chat::admit_turn(vault, &row) {
        ChatAdmission::Admitted(turn) => {
            let marked = if deps.fail_mark_running {
                Err("injected mark_running failure".to_string())
            } else {
                run_queue::mark_running(vault, row.id, turn.run_id(), now)
            };
            if let Err(error) = marked {
                if let Err(finish) = turn.cancel_unstarted() {
                    eprintln!("[chat] finishing an unmarked turn's run failed: {finish}");
                }
                return unmarked(vault, &row, now, &error);
            }
            if run_queue::cancel_requested(vault, row.id).unwrap_or(false) {
                if let Err(error) = turn.cancel_unstarted() {
                    eprintln!("[chat] finishing a cancelled turn's run failed: {error}");
                }
                return end_unstarted_chat(
                    vault,
                    &row,
                    RowStatus::Cancelled,
                    QueuedEnd::Cancelled,
                    now,
                    pending,
                );
            }
            PumpStep::Admitted(Dispatch::Chat { row, turn, guard })
        }
        // A Stop that landed while the row was claimed ends it here, rather
        // than queueing it behind the holder with the flag set (C-S3).
        ChatAdmission::Deferred(_)
            if run_queue::cancel_requested(vault, row.id).unwrap_or(false) =>
        {
            end_unstarted_chat(
                vault,
                &row,
                RowStatus::Cancelled,
                QueuedEnd::Cancelled,
                now,
                pending,
            )
        }
        ChatAdmission::Deferred(holder) => requeue(vault, &row, &holder, now),
        ChatAdmission::Recovered(message) => end_unstarted_chat(
            vault,
            &row,
            RowStatus::Failed,
            QueuedEnd::Recovered(message),
            now,
            pending,
        ),
        // P1a: a resume whose pause another turn already ended: a no-op,
        // ended like a cancel from the queue (no turn, `cancelled{queued}`
        // and `done`).
        ChatAdmission::Superseded => end_unstarted_chat(
            vault,
            &row,
            RowStatus::Cancelled,
            QueuedEnd::Cancelled,
            now,
            pending,
        ),
        ChatAdmission::Invalid(message) => end_unstarted_chat(
            vault,
            &row,
            RowStatus::Failed,
            QueuedEnd::Failed(message),
            now,
            pending,
        ),
    }
}

// ── Execute and complete ────────────────────────────────────────────────────

/// End an executed workflow row from its result and hand that exact result
/// (receipt included) to Run now's waiter. Returns the row's status.
pub(crate) fn complete_workflow(
    vault: &Path,
    row: &QueueRow,
    result: Result<WorkflowRunResult, String>,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
) -> RowStatus {
    let (status, error) = match &result {
        Ok(run) => (
            if run.status == "interrupted" {
                RowStatus::Interrupted
            } else if run.status == "paused" {
                RowStatus::Paused
            } else {
                RowStatus::from_run_status(&run.status)
            },
            run.error.clone(),
        ),
        Err(error) => (RowStatus::Failed, Some(error.clone())),
    };
    if let Err(write_error) = run_queue::finish(vault, row.id, status, error.as_deref(), now) {
        eprintln!(
            "[run queue] could not finish row {}: {write_error}; retrying",
            row.id
        );
        pending.push(PendingFinish {
            id: row.id,
            status,
            error,
        });
    }
    run_queue::resolve_waiters(vault, row.id, RowOutcome::Workflow(result));
    status
}

/// End an executed heartbeat row.
fn complete_heartbeat(
    vault: &Path,
    row: &QueueRow,
    result: &Result<usize, String>,
    now: DateTime<Utc>,
    pending: &mut Vec<PendingFinish>,
) {
    match result {
        Ok(_) => end_row(vault, row.id, RowStatus::Done, None, now, pending),
        Err(error) => end_row(vault, row.id, RowStatus::Failed, Some(error), now, pending),
    }
}

/// Run an admitted dispatch to its end and finish its row. Returns any
/// terminal write to retry.
pub(crate) async fn execute(app: &Core, vault: &Path, dispatch: Dispatch) -> Vec<PendingFinish> {
    let finish_vault = vault.to_path_buf();
    let (id, pending) = match dispatch {
        Dispatch::Workflow { row, run } => {
            let result = workflows::execute(app.clone(), vault.to_path_buf(), run).await;
            let id = row.id;
            let pending = tauri::async_runtime::spawn_blocking(move || {
                let mut pending = Vec::new();
                complete_workflow(&finish_vault, &row, result, Utc::now(), &mut pending);
                pending
            })
            .await
            .unwrap_or_default();
            (id, pending)
        }
        Dispatch::Heartbeat {
            row,
            reason,
            prepared,
            lease,
        } => {
            let result = heartbeat::run_beat(app, vault, &reason, prepared, lease).await;
            if let Err(error) = &result {
                eprintln!("[heartbeat] beat failed: {error}");
            }
            let id = row.id;
            let pending = tauri::async_runtime::spawn_blocking(move || {
                let mut pending = Vec::new();
                complete_heartbeat(&finish_vault, &row, &result, Utc::now(), &mut pending);
                pending
            })
            .await
            .unwrap_or_default();
            (id, pending)
        }
        Dispatch::Chat { row, turn, guard } => {
            let scoped = turn.is_scoped();
            let conversation = row.source_id.clone();
            // Repo ideas 2026-10-27 #5: a Preview tab shared with this chat
            // belongs to this turn and ends with it, however it ends.
            let shared_tab = conversation
                .as_deref()
                .map(|key| crate::tab_grants::TurnScope::start(key, turn.run_id()));
            let capture_turn = (turn.session_key().to_string(), turn.run_id().to_string());
            let end = chat::execute_turn(app.clone(), vault.to_path_buf(), turn).await;
            drop(shared_tab);
            let id = row.id;
            let status = RowStatus::from_run_status(&end.status);
            let error = end.error.clone();
            // The row goes terminal FIRST, then the chat-row gate is
            // released, then `done` is emitted: a follow-up sent the moment
            // `done` arrives is never refused as "already queued".
            let pending = tauri::async_runtime::spawn_blocking(move || {
                let mut pending = Vec::new();
                end_row(
                    &finish_vault,
                    id,
                    status,
                    error.as_deref(),
                    Utc::now(),
                    &mut pending,
                );
                pending
            })
            .await
            .unwrap_or_default();
            drop(guard);
            chat::emit_event(app, end.done);
            // Mid-turn steering: a send held for after this turn (Queue for
            // after, Stop and send) goes now, after `done`, as its own
            // `heldSent` turn. Repo ideas 2026-10-27 #8: before it, a turn
            // that ran out and changed nothing continues on the agent the
            // person chose, unless they already queued a message of their own.
            if let Some(key) = conversation.as_deref() {
                crate::commands::continue_after(app, key).await;
                crate::commands::drain_held(app, key).await;
            }
            if !scoped {
                note_chat_finished(Utc::now());
            }
            chat::spawn_learning(app, vault, &capture_turn.0, &capture_turn.1, end.learning);
            (id, pending)
        }
    };
    emit_row(app, vault, "finished", id);
    pending
}

// ── The loop ────────────────────────────────────────────────────────────────

/// The titles one banner of `kind` names: only workflows whose `notify`
/// policy allows skip banners (`always`, the default; Paperclip PR 9). A
/// `failures` or `never` workflow's skip is still logged in its run log.
pub(crate) fn announced_titles(notices: &[SkipNotice], kind: NoticeKind) -> Vec<String> {
    notices
        .iter()
        .filter(|notice| notice.kind == kind && notice.notify.allows_skip_notice())
        .map(|notice| notice.title.clone())
        .collect()
}

/// Flush skip notices collected over a burst of claims: one notification
/// per reason, like PR 2's per-tick aggregation, naming only the workflows
/// whose `notify` policy allows it.
fn flush_notices(app: &Core, vault: &Path, notices: &mut Vec<SkipNotice>) {
    let reserve = || reserve_skip_why(crate::cfo::peek_settings(Some(vault)).reserve_percent);
    for (kind, why) in [
        (
            NoticeKind::Missed,
            "missed while Hollow was closed".to_string(),
        ),
        (NoticeKind::Busy, "while Hollow was busy".to_string()),
        (NoticeKind::Reserve, String::new()),
    ] {
        let titles = announced_titles(notices, kind);
        if !titles.is_empty() {
            let why = if kind == NoticeKind::Reserve {
                reserve()
            } else {
                why
            };
            workflows::notify_skipped(app, vault, &titles, &why);
        }
    }
    notices.clear();
}

/// What, besides the deadline, a stop request and the vault lane's end
/// (which always do), wakes a waiting loop.
#[derive(Clone, Copy)]
enum Wake {
    /// Nothing else: the backoff after a failed step.
    Timer,
    /// An enqueue (`run_queue::wake`).
    Queue,
    /// An enqueue or a lease release (another writer is waited for).
    QueueOrLease,
}

/// One executed dispatch: the terminal writes it could not make.
pub(crate) type RunFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Vec<PendingFinish>> + Send>>;

/// What the drain loop needs beyond the queue. [`AppPump`] is the app's;
/// tests drive the real loop ([`drain`]) with a fake executor.
pub(crate) trait PumpHost: Send + Sync + 'static {
    fn vault(&self) -> &Path;
    /// One step (sync: the loop runs it on a blocking worker). `lane` is the
    /// live vault-lane row, if any (see [`step_beside`]).
    fn step(
        &self,
        gap: Option<DateTime<Utc>>,
        pending: &mut Vec<PendingFinish>,
        lane: Option<i64>,
    ) -> PumpStep;
    /// Run an admitted dispatch to its end and finish its row ([`execute`]).
    fn execute(&self, dispatch: Dispatch) -> RunFuture;
    fn emit_row(&self, kind: &str, id: i64);
    fn emit_chat(&self, events: Vec<Value>);
    fn flush_notices(&self, notices: &mut Vec<SkipNotice>);
    /// When gap-bound rows may be claimed again, given the loop's own gap.
    fn gap(&self, own: Option<DateTime<Utc>>) -> Option<DateTime<Utc>>;
    /// What an enqueue notifies ([`run_queue::woken`]); a test's own keeps
    /// other tests' enqueues from waking its loop.
    fn queue_woken(&self) -> &tokio::sync::Notify {
        run_queue::woken()
    }
    /// What a lease release notifies ([`automated_history::lease_released`]).
    fn lease_released(&self) -> &tokio::sync::Notify {
        automated_history::lease_released()
    }
}

/// The app's [`PumpHost`].
struct AppPump {
    app: Core,
    vault: PathBuf,
}

impl PumpHost for AppPump {
    fn vault(&self) -> &Path {
        &self.vault
    }

    fn step(
        &self,
        gap: Option<DateTime<Utc>>,
        pending: &mut Vec<PendingFinish>,
        lane: Option<i64>,
    ) -> PumpStep {
        let deps = StepDeps::production(Local::now());
        step_beside(&self.vault, Utc::now(), gap, pending, &deps, lane)
    }

    fn execute(&self, dispatch: Dispatch) -> RunFuture {
        let (app, vault) = (self.app.clone(), self.vault.clone());
        Box::pin(async move { execute(&app, &vault, dispatch).await })
    }

    fn emit_row(&self, kind: &str, id: i64) {
        emit_row(&self.app, &self.vault, kind, id);
    }

    fn emit_chat(&self, events: Vec<Value>) {
        emit_chat(&self.app, events);
    }

    fn flush_notices(&self, notices: &mut Vec<SkipNotice>) {
        flush_notices(&self.app, &self.vault, notices);
    }

    fn gap(&self, own: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
        current_gap(own)
    }
}

/// Asks [`drain`] to stop: it admits nothing more and returns once its
/// vault-lane run has ended. The app never asks (see [`pump_loop`]).
#[derive(Default)]
pub(crate) struct PumpStop {
    requested: AtomicBool,
    wake: tokio::sync::Notify,
}

impl PumpStop {
    #[cfg(test)]
    pub(crate) fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        // `notify_one` keeps a permit, so a stop asked between the loop's
        // check and its wait is not lost.
        self.wake.notify_one();
    }

    fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }
}

/// The vault lane's live run (merged plan slice 3, F09): one admitted
/// non-scoped dispatch (a workflow, the heartbeat, a vault-root chat turn)
/// on its own task. The loop keeps stepping meanwhile, admitting scoped
/// chat rows only, and joins the task when it ends, so a run whose task
/// panicked or was cancelled is still observed and the lane released.
struct Lane {
    unknown_status: RowStatus,
    row_id: i64,
    task: tauri::async_runtime::JoinHandle<Vec<PendingFinish>>,
}

/// How a supervised task (the vault lane or a scoped chat turn) ended: its
/// retries, or the join error of a task that panicked or was cancelled.
struct LaneEnd {
    unknown_status: RowStatus,
    row_id: i64,
    joined: Result<Vec<PendingFinish>, String>,
}

/// Scoped tasks notify even when execution unwinds or cancels.
struct WakeOnTurnEnd;

impl Drop for WakeOnTurnEnd {
    fn drop(&mut self) {
        run_queue::wake();
    }
}

/// Supervised scoped chat turns running beside the vault lane: every live
/// task is tracked by id so a panic or abort marks its row `failed` via
/// [`end_row`] and releases capacity, and shutdown awaits every live task.
struct ScopedTasks {
    tasks: tokio::task::JoinSet<(i64, Vec<PendingFinish>)>,
    rows: std::collections::HashMap<tokio::task::Id, i64>,
}

impl ScopedTasks {
    fn new() -> Self {
        Self {
            tasks: tokio::task::JoinSet::new(),
            rows: std::collections::HashMap::new(),
        }
    }

    fn spawn(&mut self, row_id: i64, fut: RunFuture) {
        let handle = self.tasks.spawn(async move {
            let _wake = WakeOnTurnEnd;
            (row_id, fut.await)
        });
        self.rows.insert(handle.id(), row_id);
    }

    fn finish(
        &mut self,
        result: Result<(i64, Vec<PendingFinish>), tokio::task::JoinError>,
    ) -> Option<LaneEnd> {
        match result {
            Ok((row_id, retry)) => {
                self.rows.retain(|_, id| *id != row_id);
                Some(LaneEnd {
                    row_id,
                    unknown_status: RowStatus::Failed,
                    joined: Ok(retry),
                })
            }
            Err(error) => {
                let row_id = self.rows.remove(&error.id())?;
                Some(LaneEnd {
                    row_id,
                    unknown_status: RowStatus::Failed,
                    joined: Err(error.to_string()),
                })
            }
        }
    }

    fn try_next(&mut self) -> Option<LaneEnd> {
        while let Some(result) = self.tasks.try_join_next() {
            if let Some(end) = self.finish(result) {
                return Some(end);
            }
        }
        None
    }

    async fn next(&mut self) -> Option<LaneEnd> {
        while let Some(result) = self.tasks.join_next().await {
            if let Some(end) = self.finish(result) {
                return Some(end);
            }
        }
        None
    }
}

/// Which supervised task ended during a wait.
enum TaskEnd {
    Lane(LaneEnd),
    Scoped(LaneEnd),
}

impl Lane {
    fn finished(&self) -> bool {
        self.task.inner().is_finished()
    }

    async fn join(self) -> LaneEnd {
        LaneEnd {
            row_id: self.row_id,
            unknown_status: self.unknown_status,
            joined: self.task.await.map_err(|e| e.to_string()),
        }
    }
}

/// The live lane task's end, or never when no run is live.
async fn lane_task(lane: Option<&mut Lane>) -> Result<Vec<PendingFinish>, String> {
    match lane {
        Some(lane) => (&mut lane.task).await.map_err(|e| e.to_string()),
        None => std::future::pending().await,
    }
}

/// What ended a [`wait_for`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Woke {
    Lane,
    Scoped,
    Queue,
    Lease,
    Stop,
    Timer,
}

/// Wait for `wake`, the deadline, a stop request, the vault lane's end or a
/// scoped turn's end, whichever comes first.
async fn wait_beside<H: PumpHost>(
    host: &H,
    deadline: std::time::Duration,
    wake: Wake,
    lane: &mut Option<Lane>,
    scoped: &mut ScopedTasks,
    stop: &PumpStop,
) -> Option<TaskEnd> {
    wait_for(host, deadline, wake, lane, scoped, stop).await.0
}

/// [`wait_beside`], also saying which of them came.
async fn wait_for<H: PumpHost>(
    host: &H,
    deadline: std::time::Duration,
    wake: Wake,
    lane: &mut Option<Lane>,
    scoped: &mut ScopedTasks,
    stop: &PumpStop,
) -> (Option<TaskEnd>, Woke) {
    let woken = host.queue_woken().notified();
    let released = host.lease_released().notified();
    enum Selected {
        Lane(Result<Vec<PendingFinish>, String>),
        Scoped(Result<(i64, Vec<PendingFinish>), tokio::task::JoinError>),
        Other,
    }
    let (selected, why) = {
        let live = lane.as_mut();
        tokio::select! {
            result = lane_task(live) => (Selected::Lane(result), Woke::Lane),
            Some(result) = scoped.tasks.join_next(), if !scoped.tasks.is_empty() => {
                (Selected::Scoped(result), Woke::Scoped)
            }
            _ = woken, if !matches!(wake, Wake::Timer) => (Selected::Other, Woke::Queue),
            _ = released, if matches!(wake, Wake::QueueOrLease) => (Selected::Other, Woke::Lease),
            _ = stop.wake.notified() => (Selected::Other, Woke::Stop),
            _ = tokio::time::sleep(deadline) => (Selected::Other, Woke::Timer),
        }
    };
    let ended = match selected {
        Selected::Lane(joined) => lane.take().map(|ended| {
            TaskEnd::Lane(LaneEnd {
                row_id: ended.row_id,
                unknown_status: ended.unknown_status,
                joined,
            })
        }),
        Selected::Scoped(result) => scoped.finish(result).map(TaskEnd::Scoped),
        Selected::Other => None,
    };
    (ended, why)
}

/// The wait after a busy step (S14): for [`LEASE_WAKE_MIN`] only an
/// enqueue, a stop, the vault lane's end or a scoped turn's end returns
/// early, then a lease release too (one released meanwhile kept
/// `notify_one`'s permit, so it returns at once), all within [`BUSY_POLL`].
/// So lease releases step the pump at most once per [`LEASE_WAKE_MIN`].
async fn busy_wait<H: PumpHost>(
    host: &H,
    lane: &mut Option<Lane>,
    scoped: &mut ScopedTasks,
    stop: &PumpStop,
) -> Option<TaskEnd> {
    let started = std::time::Instant::now();
    let (ended, why) = wait_for(host, LEASE_WAKE_MIN, Wake::Queue, lane, scoped, stop).await;
    // Anything but the timer (an enqueue, a stop, a task's end): step now.
    if why != Woke::Timer {
        return ended;
    }
    let rest = BUSY_POLL.saturating_sub(started.elapsed());
    wait_beside(host, rest, Wake::QueueOrLease, lane, scoped, stop).await
}

/// Settle a vault-lane run's end: queue its retries or, when its task
/// panicked or was cancelled (its dropped dispatch finalized its history run
/// and released the lease), fail its row so it is not left `running` until
/// the next launch. Returns when the automation gap opens.
async fn lane_ended<H: PumpHost>(
    host: &Arc<H>,
    end: LaneEnd,
    pending: &mut Vec<PendingFinish>,
) -> DateTime<Utc> {
    let unknown_status = end.unknown_status;
    task_ended(host, end, pending, unknown_status).await;
    Utc::now() + Duration::seconds(MIN_AUTOMATION_GAP_SECS)
}

async fn task_ended<H: PumpHost>(
    host: &Arc<H>,
    end: LaneEnd,
    pending: &mut Vec<PendingFinish>,
    unknown_status: RowStatus,
) {
    match end.joined {
        Ok(retry) => pending.extend(retry),
        Err(error) => {
            let id = end.row_id;
            eprintln!("[run queue] the supervised run of row {id} ended abnormally: {error}");
            let vault = host.vault().to_path_buf();
            let message = format!("The run stopped unexpectedly: {error}");
            let retry = tauri::async_runtime::spawn_blocking(move || {
                let mut retry = Vec::new();
                end_row(
                    &vault,
                    id,
                    unknown_status,
                    Some(&message),
                    Utc::now(),
                    &mut retry,
                );
                retry
            })
            .await
            .unwrap_or_else(|error| {
                vec![PendingFinish {
                    id,
                    status: unknown_status,
                    error: Some(format!(
                        "The supervised completion write stopped unexpectedly: {error}"
                    )),
                }]
            });
            pending.extend(retry);
            host.emit_row("finished", id);
        }
    }
}

/// Startup recovery, sync (the loop runs it on a blocking worker, and tests
/// drive it). [`run_queue::recover_at_startup`] interrupts the previous
/// process's claimed and running rows and its queued Run now and chat rows
/// (a chat row keeps its asked text, which the composer restores once; it is
/// never run). In the same transaction it closes the session-store turn and
/// the usage rows of each chat row it interrupted mid-run as `interrupted`
/// (merged plan slice 3, B·F10, `session_store::interrupt_chat_run_in`), so
/// recovery never closes only some of them and a failure is retried with
/// the rest of recovery; history's own startup reconciliation closes those
/// runs' manifests. Then PR 2's retired
/// deferral file is removed, best effort. Returns the interrupted rows and
/// how many legacy episodes were retired.
pub(crate) fn recover_queue(
    vault: &Path,
    boot_at: DateTime<Utc>,
    now: DateTime<Utc>,
    stopped_leftovers: &HashSet<String>,
) -> Result<(Vec<QueueRow>, usize), String> {
    let interrupted = run_queue::recover_at_startup(vault, boot_at, now, stopped_leftovers)?;
    // Best effort, before anything is admitted: a handover record whose
    // successor never got a row is removed (reliability 11-13).
    // Under the chat gate, so never between a handover's record and its row.
    let reconciled = crate::chat_rows::with_active_rows(vault, |_| {
        crate::app_sessions::reconcile_handovers(vault)
    });
    if let Err(error) = reconciled {
        eprintln!("[run queue] could not reconcile handover records: {error}");
    }
    // A retry row the previous process claimed or ran never reached
    // `finalize`'s outcome (Paperclip PR 9b): report the failure it was
    // deferring and count it toward the streak now.
    for row in interrupted
        .iter()
        .filter(|row| row.source == RunSource::Retry)
    {
        if let Some(slug) = row.source_id.as_deref() {
            workflow_retry::resolve_skipped_retry(
                vault,
                slug,
                &row.payload,
                RetrySkipReason::Interrupted,
            );
        }
    }
    // Best effort: an obsolete file that cannot be removed must never
    // keep the queue from reaching Owned (every claim, and every Run
    // now enqueued meanwhile, would wait forever).
    let retired = run_queue::retire_legacy_deferrals(vault).unwrap_or_else(|error| {
        eprintln!("[run queue] could not retire the legacy deferral file: {error}");
        crate::health::record(
            vault,
            "workflows",
            crate::health::Outcome::Partial,
            format!(
                "Could not remove the retired {}: {error}",
                run_queue::LEGACY_DEFERRALS_FILE
            ),
        );
        0
    });
    Ok((interrupted, retired))
}

/// Recover the queue (before any claim, retried until it succeeds), then
/// [`drain`] it for the life of the process. Quitting does not ask the loop
/// to stop (unchanged by merged plan slice 3):
/// `process_group::orphans::settle_at_quit` ends the live runs' processes,
/// and a row the quit cuts short is interrupted by the next launch's
/// recovery.
async fn pump_loop(
    app: Core,
    vault: PathBuf,
    boot_at: DateTime<Utc>,
    stopped_leftovers: HashSet<String>,
) {
    let stopped_leftovers = Arc::new(stopped_leftovers);
    loop {
        let recover_vault = vault.clone();
        let stopped = Arc::clone(&stopped_leftovers);
        let recovered = tauri::async_runtime::spawn_blocking(move || {
            recover_queue(&recover_vault, boot_at, Utc::now(), &stopped)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|result| result);
        match recovered {
            Ok((interrupted, retired)) => {
                // This launch now offers to resume the chats it interrupted
                // mid-run (overnight A1); only a person's click resumes them.
                crate::restart_resume::recovered(&vault, boot_at);
                for row in interrupted {
                    emit(&app, "finished", run_queue::to_item(&row, None));
                }
                if retired > 0 {
                    eprintln!("[run queue] retired {retired} legacy deferral episode(s)");
                }
                break;
            }
            Err(error) => {
                eprintln!("[run queue] startup recovery failed: {error}; retrying");
                tokio::time::sleep(RECOVERY_RETRY).await;
            }
        }
    }
    set_state(QueueState::Owned);
    // Sends held for after a turn when Hollow last quit go once their
    // conversation is idle (mid-turn steering).
    if !crate::steering::held_conversations().is_empty() {
        crate::commands::start_held_sweeper(app.clone());
    }
    drain(
        Arc::new(AppPump { app, vault }),
        Arc::new(PumpStop::default()),
    )
    .await;
}

/// Drain the queue one step at a time until `stop` is asked. A scoped chat
/// turn runs on its own task (PR B3); since merged plan slice 3 (F09) the
/// vault lane's run does too, on a supervised [`Lane`], so scoped turns are
/// admitted while a workflow, heartbeat or vault-root turn runs and no
/// second vault-lane row is ([`step_beside`]). The automation gap opens when
/// the lane's run ends. On stop, the lane's run is awaited before returning.
pub(crate) async fn drain<H: PumpHost>(host: Arc<H>, stop: Arc<PumpStop>) {
    // OC13: a stopping pump kills every running precheck by PID and starts no
    // new one; a restarted one starts afresh.
    struct PrecheckStop(PathBuf);
    impl Drop for PrecheckStop {
        fn drop(&mut self) {
            crate::workflow_precheck::stop_all(&self.0);
        }
    }
    crate::workflow_precheck::resume(host.vault());
    let _precheck_stop = PrecheckStop(host.vault().to_path_buf());
    let mut own_gap: Option<DateTime<Utc>> = None;
    let mut pending: Vec<PendingFinish> = Vec::new();
    let mut notices: Vec<SkipNotice> = Vec::new();
    let mut backoff = BUSY_POLL;
    let mut lane: Option<Lane> = None;
    let mut scoped = ScopedTasks::new();
    while !stop.requested() {
        if lane.as_ref().is_some_and(Lane::finished) {
            if let Some(done) = lane.take() {
                own_gap = Some(lane_ended(&host, done.join().await, &mut pending).await);
            }
        }
        while let Some(done) = scoped.try_next() {
            task_ended(&host, done, &mut pending, RowStatus::Failed).await;
        }
        let gap = host.gap(own_gap);
        let carried = std::mem::take(&mut pending);
        let lane_row = lane.as_ref().map(|live| live.row_id);
        let step_host = Arc::clone(&host);
        let joined = tauri::async_runtime::spawn_blocking(move || {
            let mut carried = carried;
            let outcome = step_host.step(gap, &mut carried, lane_row);
            let poll = if matches!(outcome, PumpStep::Idle | PumpStep::Sleep(_))
                && run_queue::has_queued_rows(step_host.vault()).unwrap_or(true)
            {
                eprintln!("[run queue] queued rows remain; sleep capped at 5 seconds");
                QUEUED_POLL
            } else {
                IDLE_POLL
            };
            (outcome, carried, poll)
        })
        .await;
        let (outcome, poll) = match joined {
            Ok((outcome, carried, poll)) => {
                pending = carried;
                (outcome, poll)
            }
            Err(error) => (PumpStep::Error(error.to_string()), QUEUED_POLL),
        };
        if !matches!(outcome, PumpStep::Handled { .. }) {
            host.flush_notices(&mut notices);
        }
        if !matches!(outcome, PumpStep::Error(_)) {
            backoff = BUSY_POLL;
        }
        let ended = match outcome {
            PumpStep::Idle => {
                wait_beside(&*host, poll, Wake::Queue, &mut lane, &mut scoped, &stop).await
            }
            PumpStep::Sleep(at) => {
                let wait = (at - Utc::now())
                    .to_std()
                    .unwrap_or(std::time::Duration::ZERO)
                    .min(poll);
                wait_beside(&*host, wait, Wake::Queue, &mut lane, &mut scoped, &stop).await
            }
            PumpStep::Busy {
                episodes_started,
                failed,
                chat,
                ..
            } => {
                for id in episodes_started {
                    host.emit_row("waiting", id);
                }
                for id in failed {
                    host.emit_row("finished", id);
                }
                host.emit_chat(chat);
                busy_wait(&*host, &mut lane, &mut scoped, &stop).await
            }
            PumpStep::Handled {
                row_id,
                event,
                notice,
                chat,
            } => {
                if let Some(kind) = event {
                    host.emit_row(kind, row_id);
                }
                host.emit_chat(chat);
                notices.extend(notice);
                None
            }
            PumpStep::Admitted(dispatch) if dispatch.is_scoped() => {
                // A build workspace's turn (PR B3) holds no vault lease: it
                // runs on its own supervised task beside the vault lane, and
                // it opens no automation gap.
                let row_id = dispatch.row().id;
                host.emit_row("running", row_id);
                scoped.spawn(row_id, host.execute(dispatch));
                None
            }
            PumpStep::Admitted(dispatch) => {
                // `step_beside` admits no vault-lane row while one runs (and
                // the lease would refuse it); never hold two lane tasks.
                let earlier = match lane.take() {
                    Some(live) => Some(TaskEnd::Lane(live.join().await)),
                    None => None,
                };
                let row_id = dispatch.row().id;
                host.emit_row("running", row_id);
                let task = tauri::async_runtime::spawn(host.execute(dispatch));
                lane = Some(Lane {
                    row_id,
                    task,
                    unknown_status: RowStatus::Failed,
                });
                earlier
            }
            PumpStep::Error(error) => {
                eprintln!("[run queue] step failed: {error}");
                let ended =
                    wait_beside(&*host, backoff, Wake::Timer, &mut lane, &mut scoped, &stop).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                ended
            }
        };
        match ended {
            Some(TaskEnd::Lane(end)) => {
                own_gap = Some(lane_ended(&host, end, &mut pending).await);
            }
            Some(TaskEnd::Scoped(end)) => {
                task_ended(&host, end, &mut pending, RowStatus::Failed).await;
            }
            None => {}
        }
    }
    // Stopping: the vault lane's run and every live scoped turn are awaited,
    // never abandoned, and the terminal writes still owed get one last try.
    if let Some(live) = lane.take() {
        lane_ended(&host, live.join().await, &mut pending).await;
    }
    while let Some(done) = scoped.next().await {
        task_ended(&host, done, &mut pending, RowStatus::Failed).await;
    }
    if !pending.is_empty() {
        let vault = host.vault().to_path_buf();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            retry_pending(&vault, Utc::now(), &mut pending);
        })
        .await;
    }
}

#[cfg(test)]
#[path = "run_pump_tests.rs"]
mod tests;
