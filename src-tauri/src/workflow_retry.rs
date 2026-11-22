//! Paperclip PR 9 (9b): a workflow's bounded retry. It classifies a failed
//! run ([`classify_failure`]), decides whether it earns its one retry
//! ([`retry_plan`]), checks a queued retry row when it is claimed
//! ([`retry_row_valid`]), and reports the original failure when the retry
//! never runs ([`resolve_skipped_retry`]).
//!
//! Wiring: `workflows::execute` builds [`FailureFacts`] from the exit branch
//! and `workflows::finalize` enqueues the plan as one PR 7 `retry` row
//! (`not_before` = finish + 60 s) after `finish_run`, deferring the run's
//! failure-streak outcome and replacing its failure banner with one quiet
//! "retrying once" notice. The run-queue pump's claim (`run_pump`) checks a
//! retry row in phase 1 (missing, ran since, stale) before PR 5's permission
//! check, so a retry that no longer matters opens no review; a refusal, an
//! unreadable permission store or a PR 8 budget block skips it too. Every
//! skip except "ran since" (that later run already reported and counted)
//! calls [`resolve_skipped_retry`], as do a user's cancel of the queued
//! row, an admission that fails after the claim and startup recovery of a
//! retry row the previous process claimed or ran.
//!
//! It is a separate module from `workflow_hygiene.rs` (created by 9a-2) so
//! the two items could be drafted side by side; the quiet and inactive gates
//! live in `workflow_hygiene.rs` as planned.
//!
//! Rules (the contract's "classified from the EXIT BRANCH"):
//! - Hollow's own ends (cancel, timeout, budget) never retry, whatever the
//!   CLI printed.
//! - A signal retries only when the agent's leader died of an outside
//!   SIGKILL/SIGTERM/SIGHUP/SIGINT before Hollow's stop sequence began
//!   (`process_group::Driven.leader_exited_first`). A signal Hollow's own
//!   settle delivered, a crash signal or any other signal is permanent.
//! - Otherwise only the CLI's error envelope decides: sign-in, then usage
//!   limit, are permanent; rate limit and network errors are transient.
//!   Numeric codes count only after an `api`, `http` or `status` prefix, so a
//!   stray "line 429" never retries. Anything unrecognised is permanent, so a
//!   CLI wording change means fewer retries, never more.
//! - One retry at most: only cron and trigger runs, only a first admission,
//!   and never for a run whose history record raised protected-change flags
//!   or could not be recorded (PR 5).

use crate::notifications::NotificationInput;
use crate::process_group::Ended;
use crate::result_kind::{AUTH_RE, NETWORK_RE, QUOTA_RE, RATE_RE};
use crate::run_queue::RunSource;
use crate::workflow_hygiene::{self, FailureStreak, NotificationPlanInput, NotifyPolicy};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Seconds between a failed run's finish and its retry's `not_before`.
pub(crate) const RETRY_DELAY_SECS: i64 = 60;
/// A retry claimed more than this long after its `not_before` (the machine
/// slept, Hollow was closed, the vault stayed busy) is skipped as stale.
pub(crate) const RETRY_STALE_AFTER_MIN: i64 = 30;
/// Bound on the payload's error excerpt, in characters.
const ERROR_EXCERPT_CHARS: usize = 300;

/// Claude's result subtype when Hollow's own `--max-budget-usd` stopped the
/// run (captured live: `is_error: true`, no `result` text, exit 1). Only PR 8
/// passes that flag, so this is always Hollow's own stop.
pub(crate) const SUBTYPE_MAX_BUDGET: &str = crate::result_kind::SUBTYPE_MAX_BUDGET;
/// Claude's result subtype when the run hit its turn limit.
pub(crate) const SUBTYPE_MAX_TURNS: &str = crate::result_kind::SUBTYPE_MAX_TURNS;

/// Where a run failed. Everything before `Run` is Hollow's own preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureStage {
    /// Agent resolution, retrieval, context or command building failed.
    Prepare,
    /// The CLI could not be spawned.
    Spawn,
    /// The CLI started without a stdout pipe.
    MissingStdout,
    /// The CLI ran and `process_group::drive` returned this ending. The
    /// match on it stays exhaustive (no `_` arm), so PR 8's
    /// `Ended::LimitReached` fails to compile here until it is classified.
    Run(Ended),
}

/// Everything [`classify_failure`] looks at, gathered from one exit path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FailureFacts<'a> {
    pub stage: FailureStage,
    /// PR 8's budget guard ended the run (it may arrive as `Stopped`).
    pub budget_stop: bool,
    /// The leader's terminating signal (`exit_signal`), if it had one.
    pub signal: Option<i32>,
    /// The leader's exit status was observed before Hollow's stop sequence
    /// began, so a signal in it came from outside Hollow.
    pub leader_exited_first: bool,
    /// The CLI's error-result subtype (`Result { is_error: true, subtype }`).
    pub result_subtype: Option<&'a str>,
    /// The CLI's error-result text, when it sent one. Claude's budget result
    /// has none; its message is only in `errors`, which `agent::parse_line`
    /// does not keep.
    pub result_error: Option<&'a str>,
    /// The bounded stderr tail.
    pub stderr_tail: &'a str,
    /// The error string the run is finalized with.
    pub error: &'a str,
}

impl FailureFacts<'_> {
    /// Facts for `stage` with nothing else known yet.
    pub(crate) fn at(stage: FailureStage) -> Self {
        Self {
            stage,
            budget_stop: false,
            signal: None,
            leader_exited_first: false,
            result_subtype: None,
            result_error: None,
            stderr_tail: "",
            error: "",
        }
    }
}

/// Why a transient failure is worth one retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransientKind {
    ExternalSignal,
    RateLimit,
    Network,
}

/// Why a failure is not retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermanentKind {
    Config,
    Auth,
    Quota,
    TurnLimit,
    Crash,
    /// The model declined the request (Paperclip PR 14a; decided by the
    /// result envelope in `workflows::run_failure`, never by this module).
    Refusal,
    /// The permission mode denied the agent's actions (Paperclip PR 14a,
    /// Antigravity's headless denials; decided like `Refusal`).
    Denied,
    Unknown,
}

/// Which of Hollow's own stops ended the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnStopKind {
    Cancelled,
    TimedOut,
    Budget,
    /// The no-output watchdog ended the run (Paperclip PR 15b). Never
    /// retried: the contract forbids auto-continuing a stalled run.
    Stalled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureClass {
    Transient(TransientKind),
    Permanent(PermanentKind),
    OwnStop(OwnStopKind),
}

impl FailureClass {
    pub(crate) fn is_transient(self) -> bool {
        matches!(self, FailureClass::Transient(_))
    }

    /// A short, note-safe label (no em dash, no `<`/`>`).
    pub(crate) fn label(self) -> &'static str {
        match self {
            FailureClass::Transient(TransientKind::ExternalSignal) => {
                "stopped by an outside signal"
            }
            FailureClass::Transient(TransientKind::RateLimit) => "rate limited or overloaded",
            FailureClass::Transient(TransientKind::Network) => "network or server error",
            FailureClass::Permanent(PermanentKind::Config) => "could not start",
            FailureClass::Permanent(PermanentKind::Auth) => "sign-in problem",
            FailureClass::Permanent(PermanentKind::Quota) => "usage limit reached",
            FailureClass::Permanent(PermanentKind::TurnLimit) => "turn limit reached",
            FailureClass::Permanent(PermanentKind::Crash) => "agent crashed",
            FailureClass::Permanent(PermanentKind::Refusal) => "the model declined the request",
            FailureClass::Permanent(PermanentKind::Denied) => {
                "actions were denied by the permission mode"
            }
            FailureClass::Permanent(PermanentKind::Unknown) => "failed",
            FailureClass::OwnStop(OwnStopKind::Cancelled) => "cancelled",
            FailureClass::OwnStop(OwnStopKind::TimedOut) => "timed out",
            FailureClass::OwnStop(OwnStopKind::Budget) => "budget reached",
            FailureClass::OwnStop(OwnStopKind::Stalled) => "stalled: no output",
        }
    }
}

// The envelope regexes (sign-in, usage limit, rate limit, network) live in
// `crate::result_kind` since Paperclip PR 14a, shared with chat and receipt
// classification so "quota" means one thing everywhere. This module keeps
// its own order below: AUTH, QUOTA, RATE, NETWORK.

/// Classify a failed run. The order is the rule: preparation failures, then
/// Hollow's own stops, then signals, then the CLI's error envelope.
pub(crate) fn classify_failure(facts: &FailureFacts<'_>) -> FailureClass {
    use FailureClass::{OwnStop, Permanent, Transient};

    let ended = match facts.stage {
        FailureStage::Prepare | FailureStage::Spawn | FailureStage::MissingStdout => {
            return Permanent(PermanentKind::Config);
        }
        FailureStage::Run(ended) => ended,
    };

    // Hollow's own stops. The budget comes first: PR 8's guard may end the
    // run through the stop signal, and a CLI-enforced budget exits normally.
    if facts.budget_stop || facts.result_subtype == Some(SUBTYPE_MAX_BUDGET) {
        return OwnStop(OwnStopKind::Budget);
    }
    match ended {
        Ended::LimitReached => return OwnStop(OwnStopKind::Budget),
        Ended::Stopped => return OwnStop(OwnStopKind::Cancelled),
        Ended::TimedOut => return OwnStop(OwnStopKind::TimedOut),
        Ended::Stalled => return OwnStop(OwnStopKind::Stalled),
        Ended::Exited => {}
    }

    if let Some(signal) = facts.signal {
        if is_crash_signal(signal) {
            return Permanent(PermanentKind::Crash);
        }
        if facts.leader_exited_first && is_external_signal(signal) {
            return Transient(TransientKind::ExternalSignal);
        }
        return Permanent(PermanentKind::Unknown);
    }

    if facts.result_subtype == Some(SUBTYPE_MAX_TURNS) {
        return Permanent(PermanentKind::TurnLimit);
    }
    let texts = [
        facts.result_error.unwrap_or_default(),
        facts.stderr_tail,
        facts.error,
    ];
    if any_match(&AUTH_RE, &texts) {
        Permanent(PermanentKind::Auth)
    } else if any_match(&QUOTA_RE, &texts) {
        Permanent(PermanentKind::Quota)
    } else if any_match(&RATE_RE, &texts) {
        Transient(TransientKind::RateLimit)
    } else if any_match(&NETWORK_RE, &texts) {
        Transient(TransientKind::Network)
    } else {
        Permanent(PermanentKind::Unknown)
    }
}

/// Each source is matched on its own, so a pattern never spans two of them.
fn any_match(re: &Regex, texts: &[&str]) -> bool {
    texts.iter().any(|text| re.is_match(text))
}

#[cfg(unix)]
fn is_external_signal(signal: i32) -> bool {
    matches!(
        signal,
        libc::SIGKILL | libc::SIGTERM | libc::SIGHUP | libc::SIGINT
    )
}

#[cfg(not(unix))]
fn is_external_signal(_signal: i32) -> bool {
    false
}

#[cfg(unix)]
fn is_crash_signal(signal: i32) -> bool {
    matches!(
        signal,
        libc::SIGSEGV | libc::SIGABRT | libc::SIGBUS | libc::SIGILL
    )
}

#[cfg(not(unix))]
fn is_crash_signal(_signal: i32) -> bool {
    false
}

/// The signal that terminated a process, if any.
#[cfg(unix)]
pub(crate) fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
pub(crate) fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

// ── The retry plan ───────────────────────────────────────────────────────────

/// What [`retry_plan`] needs from a finished run. A struct rather than
/// positional parameters, so later PRs add fields (cross-m2 M1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryInput<'a> {
    pub slug: &'a str,
    pub title: &'a str,
    /// The failed run's automated-history id (its run-log run id).
    pub history_run_id: &'a str,
    /// The run's source (`RunContext.source`, PR 7's vocabulary). A `retry`
    /// run is never retried again.
    pub source: RunSource,
    /// Earlier admissions of the run's queue row (`RunContext.attempt`, the
    /// row's `attempt` as claimed): 0 for a first admission. A retry row is a
    /// new row, so its own attempt is 0 too; its source is what bounds it.
    pub attempt: u32,
    /// The run's queue row, if it had one (the retry's `retry_of`).
    pub queue_id: Option<i64>,
    /// The run's queue-row priority (a retry keeps it).
    pub priority: i64,
    pub class: FailureClass,
    /// `automated_history::finish_run` returned `Ok` for this run.
    pub history_finished: bool,
    /// The finished record carries PR 5 protected-change flags.
    pub history_flagged: bool,
    /// The run may have acted outside its files, or ran calls whose effects
    /// Hollow can't see (`run_effects::OutsideEffects::allows_automatic_retry`
    /// is false, or its receipt could not be written). Running it again
    /// unattended could repeat a completed external action, so it never
    /// retries on its own; the run review offers a guarded retry instead.
    pub outside_effects_unsafe: bool,
    /// Exactly the `at` passed to `vault_workflows::mark_started`.
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    /// The error the run was finalized with.
    pub error: &'a str,
    /// The failed run's `{{trigger_reason}}` (`RunContext::trigger_reason`),
    /// carried to the retry, which is the only run left to act on the fire.
    pub trigger_reason: Option<&'a str>,
    /// The failed run's admitted snapshot `last_run_at` (the run before it),
    /// the retry's `{{last_run_at}}`.
    pub prior_last_run_at: Option<&'a str>,
}

/// The payload stored on the retry row, read back by the claim and by the
/// skipped-retry report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetryPayload {
    pub failed_run_id: String,
    /// RFC 3339 with full sub-second precision, so it parses back to exactly
    /// the `mark_started` stamp. A stored `last_started_at` is never more
    /// precise, so the failed run itself can never look like a later start.
    pub failed_started_at: String,
    pub title: String,
    pub error_excerpt: String,
    pub class_label: String,
    /// The failed run's `{{trigger_reason}}`; absent when it had none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_reason: Option<String>,
    /// The failed run's `{{last_run_at}}` (None: `never`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_last_run_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetryPlan {
    /// `NewRow.not_before` (7a's column; no `_ms` variant).
    pub not_before: DateTime<Utc>,
    /// `NewRow.retry_of`: the failed run's queue row.
    pub retry_of: Option<i64>,
    /// The failed run's row priority.
    pub priority: i64,
    /// The queue row's reason text.
    pub reason: String,
    /// `retry:{slug}:{history_run_id}`: one retry per failed run, even if
    /// finalize is reached twice.
    pub idempotency_key: String,
    pub payload: RetryPayload,
}

/// The one retry a failed run earns, or `None`. `Some` only for a transient
/// failure of an unattended (cron or trigger) first attempt whose history
/// record finished cleanly: a flagged run changed protected settings (the user
/// already got run_guard's warning, and the next run would be refused anyway),
/// and a record that could not be finished leaves nothing trustworthy to
/// retry from.
pub(crate) fn retry_plan(input: &RetryInput<'_>) -> Option<RetryPlan> {
    if !input.class.is_transient()
        || !matches!(input.source, RunSource::Cron | RunSource::Trigger)
        || input.attempt != 0
        || !input.history_finished
        || input.history_flagged
        || input.outside_effects_unsafe
        || input.slug.trim().is_empty()
        || input.history_run_id.trim().is_empty()
    {
        return None;
    }
    let label = input.class.label();
    Some(RetryPlan {
        not_before: input.finished_at + Duration::seconds(RETRY_DELAY_SECS),
        retry_of: input.queue_id,
        priority: input.priority,
        reason: format!("retry after a failed run ({label})"),
        idempotency_key: format!("retry:{}:{}", input.slug, input.history_run_id),
        payload: RetryPayload {
            failed_run_id: input.history_run_id.to_string(),
            failed_started_at: input
                .started_at
                .to_rfc3339_opts(SecondsFormat::AutoSi, true),
            title: input.title.to_string(),
            error_excerpt: error_excerpt(input.error),
            class_label: label.to_string(),
            trigger_reason: input.trigger_reason.map(str::to_string),
            prior_last_run_at: input.prior_last_run_at.map(str::to_string),
        },
    })
}

/// Why a claimed retry row is skipped instead of run. Only the two reasons
/// [`retry_row_valid`] decides; missing, refused, unavailable and paused come
/// from the claim's other gates (precedence: missing > ran_since > stale >
/// refused/unavailable/invalid > paused).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryRowSkip {
    /// The workflow started again after the failed run; that run already
    /// reported and counted, so the retry ends silently.
    RanSince,
    /// Claimed more than [`RETRY_STALE_AFTER_MIN`] after its `not_before`.
    Stale,
}

impl RetryRowSkip {
    #[cfg(test)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RetryRowSkip::RanSince => "ran_since",
            RetryRowSkip::Stale => "stale",
        }
    }
}

/// Is a queued retry still worth running at `now`? `last_started_at` is the
/// workflow's current stamp, `failed_started_at` the payload's. Both are
/// compared as parsed instants, never as strings. An unparseable value on
/// either side cannot prove a later start, so it never makes the row
/// `RanSince` (running the retry is the safe side).
pub(crate) fn retry_row_valid(
    last_started_at: Option<&str>,
    failed_started_at: &str,
    not_before: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), RetryRowSkip> {
    let last = last_started_at.and_then(crate::trigger_evaluator::parse_rfc3339);
    let failed = crate::trigger_evaluator::parse_rfc3339(failed_started_at);
    if let (Some(last), Some(failed)) = (last, failed) {
        if last > failed {
            return Err(RetryRowSkip::RanSince);
        }
    }
    if now > not_before + Duration::minutes(RETRY_STALE_AFTER_MIN) {
        return Err(RetryRowSkip::Stale);
    }
    Ok(())
}

// ── A retry that never runs ──────────────────────────────────────────────────

/// Why a planned retry did not run. [`RetryRowSkip`]'s two, plus the claim's
/// other gates, an enqueue failure and the user's cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetrySkipReason {
    /// The workflow started again after the failed run: that run already
    /// notified and evaluated the streak, so nothing more happens.
    RanSince,
    Stale,
    /// The workflow file is gone (deleted, or renamed: queued rows are not
    /// renamed with it).
    Missing,
    /// The workflow file exists but cannot run as written (an empty prompt,
    /// frontmatter that does not parse).
    Invalid,
    /// The claim admitted the row but the run could not be started (the
    /// file went away after the lease was taken, the run registry failed).
    NotStarted,
    /// Hollow stopped while the retry was claimed or running (startup
    /// recovery marked its row interrupted), so it never finished.
    Interrupted,
    /// PR 5 refused the workflow as written (its settings changed outside
    /// the editor). PR 5 already raised its own rate-limited warning.
    Refused,
    /// PR 5's permission records could not be read (it already warned).
    Unavailable,
    /// A PR 8 usage budget holds the automation.
    BudgetPaused,
    /// `finalize` could not queue the retry row.
    EnqueueFailed,
    /// The user cancelled the queued retry.
    UserCancel,
}

impl RetrySkipReason {
    fn phrase(self) -> &'static str {
        match self {
            RetrySkipReason::RanSince => "the automation ran again",
            RetrySkipReason::Stale => "it could not start within 30 minutes",
            RetrySkipReason::Missing => "the automation no longer exists",
            RetrySkipReason::Invalid => "the automation cannot run as written",
            RetrySkipReason::NotStarted => "it could not be started",
            RetrySkipReason::Interrupted => "Hollow stopped before it finished",
            RetrySkipReason::Refused => "its settings changed and await approval",
            RetrySkipReason::Unavailable => "its permission records could not be read",
            RetrySkipReason::BudgetPaused => "a usage budget paused it",
            RetrySkipReason::EnqueueFailed => "it could not be queued",
            RetrySkipReason::UserCancel => "it was cancelled",
        }
    }

    /// The claim's row error for this skip.
    pub(crate) fn row_error(self) -> String {
        format!("retry skipped: {}", self.phrase())
    }
}

impl From<RetryRowSkip> for RetrySkipReason {
    fn from(skip: RetryRowSkip) -> Self {
        match skip {
            RetryRowSkip::RanSince => RetrySkipReason::RanSince,
            RetryRowSkip::Stale => RetrySkipReason::Stale,
        }
    }
}

/// The payload of a retry row, or a stand-in naming the workflow when it is
/// unreadable (a hand-edited queue database), so the outcome still counts.
pub(crate) fn payload_of(slug: &str, payload: &Value) -> RetryPayload {
    serde_json::from_value(payload.clone()).unwrap_or_else(|_| RetryPayload {
        failed_run_id: String::new(),
        failed_started_at: String::new(),
        title: slug.to_string(),
        error_excerpt: "no error details".to_string(),
        class_label: String::new(),
        trigger_reason: None,
        prior_last_run_at: None,
    })
}

/// The notification a skipped retry records, pure. `streak_opened` is what
/// `workflow_hygiene::record_outcome` returned for the deferred failure.
/// - for [`RetrySkipReason::RanSince`]: none.
/// - a streak opened: the one native streak notification (it ignores
///   `notify`, as in `finalize`);
/// - [`RetrySkipReason::UserCancel`]: none otherwise (the user's own doing);
/// - otherwise the original failure, gated by `notify`, saying why the retry
///   was skipped. Native, except after a PR 5 refusal or unreadable
///   permission records, which already raised PR 5's own warning, so one
///   banner is shown, not two.
pub(crate) fn skipped_retry_notification(
    payload: &RetryPayload,
    slug: &str,
    reason: RetrySkipReason,
    policy: NotifyPolicy,
    streak_opened: Option<&FailureStreak>,
) -> Option<NotificationInput> {
    if reason == RetrySkipReason::RanSince {
        return None;
    }
    if reason == RetrySkipReason::UserCancel && streak_opened.is_none() {
        return None;
    }
    let error = format!(
        "{} The retry was skipped because {}.",
        payload.error_excerpt.trim(),
        reason.phrase()
    );
    let title = if payload.title.trim().is_empty() {
        slug
    } else {
        payload.title.as_str()
    };
    let mut input = workflow_hygiene::finalize_notification(&NotificationPlanInput {
        status: "error",
        produced: false,
        policy,
        streak_opened,
        title,
        slug,
        note: "",
        error: Some(error.trim()),
    })?;
    if streak_opened.is_none()
        && matches!(
            reason,
            RetrySkipReason::Refused | RetrySkipReason::Unavailable
        )
    {
        input.notify_native = false;
    }
    Some(input)
}

/// Resolve the failure a retry was deferring, when the retry does not run
/// ([`RetrySkipReason`]), and return the notification to deliver: record the
/// failure's streak outcome now (`workflow_hygiene::record_outcome`, which
/// `finalize` deferred while the retry was planned), then plan the
/// notification. Nothing for [`RetrySkipReason::RanSince`]. The workflow's
/// current `notify` applies (a missing workflow: the default). Errors are
/// logged by `record_outcome`, never propagated.
pub(crate) fn plan_skipped_retry(
    vault: &Path,
    slug: &str,
    payload: &Value,
    reason: RetrySkipReason,
) -> Option<NotificationInput> {
    if reason == RetrySkipReason::RanSince {
        return None;
    }
    let payload = payload_of(slug, payload);
    let policy = crate::vault_workflows::read_full(vault, slug)
        .map(|(workflow, _, _)| NotifyPolicy::parse(workflow.notify.as_deref()))
        .unwrap_or_default();
    let streak = workflow_hygiene::record_outcome(
        vault,
        slug,
        &payload.title,
        "error",
        &payload.failed_run_id,
    );
    skipped_retry_notification(&payload, slug, reason, policy, streak.as_ref())
}

/// [`plan_skipped_retry`], delivered through the registered app handle
/// (`notifications::create_registered`, a no-op in tests and subprocesses).
pub(crate) fn resolve_skipped_retry(
    vault: &Path,
    slug: &str,
    payload: &Value,
    reason: RetrySkipReason,
) {
    if let Some(input) = plan_skipped_retry(vault, slug, payload, reason) {
        if let Some(Err(error)) = crate::notifications::create_registered(vault, input) {
            eprintln!("[workflow {slug}] could not record the skipped retry's notice: {error}");
        }
    }
}

/// The quiet notice `finalize` records in place of a failed run's banner
/// while its retry is queued (gated by `notify` like the banner it replaces;
/// never native: the retry's own outcome is the one that interrupts).
pub(crate) fn retrying_notification(
    title: &str,
    slug: &str,
    plan: &RetryPlan,
    policy: NotifyPolicy,
) -> Option<NotificationInput> {
    if !policy.allows_run("error") {
        return None;
    }
    Some(NotificationInput {
        title: format!("{title} failed; retrying once in 1 minute"),
        body: format!(
            "{} ({}).",
            plan.payload.error_excerpt, plan.payload.class_label
        ),
        kind: "warning".to_string(),
        source: Some(slug.to_string()),
        action_label: Some("Open workflow".to_string()),
        action_url: Some(format!("workflow:{slug}")),
        notify_native: false,
    })
}

/// One line, redacted, note-safe (no control characters, em dash, `<`/`>`),
/// at most [`ERROR_EXCERPT_CHARS`] characters.
fn error_excerpt(error: &str) -> String {
    let redacted = crate::claude_runner::redact(error);
    let cleaned: String = redacted
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '—' | '<' | '>') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return "no error details".to_string();
    }
    if line.chars().count() <= ERROR_EXCERPT_CHARS {
        return line;
    }
    let mut out: String = line.chars().take(ERROR_EXCERPT_CHARS - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::FailureClass::{OwnStop, Permanent, Transient};
    use super::*;
    use chrono::TimeZone;

    fn exited() -> FailureFacts<'static> {
        FailureFacts::at(FailureStage::Run(Ended::Exited))
    }

    fn envelope(error_text: &'static str) -> FailureClass {
        classify_failure(&FailureFacts {
            result_error: Some(error_text),
            error: "claude exited exit status: 1",
            ..exited()
        })
    }

    fn stderr(tail: &'static str) -> FailureClass {
        classify_failure(&FailureFacts {
            stderr_tail: tail,
            error: "codex exited exit status: 1",
            ..exited()
        })
    }

    #[test]
    fn classify_failure_from_the_exit_branch() {
        // Preparation failures are configuration, whatever the text says.
        for stage in [
            FailureStage::Prepare,
            FailureStage::Spawn,
            FailureStage::MissingStdout,
        ] {
            let facts = FailureFacts {
                error: "fetch failed: ECONNRESET",
                ..FailureFacts::at(stage)
            };
            assert_eq!(classify_failure(&facts), Permanent(PermanentKind::Config));
        }

        // Hollow's own stops never retry, even over transient-looking text.
        let stopped = FailureFacts {
            stderr_tail: "API Error: 529 overloaded",
            ..FailureFacts::at(FailureStage::Run(Ended::Stopped))
        };
        assert_eq!(classify_failure(&stopped), OwnStop(OwnStopKind::Cancelled));
        let timed_out = FailureFacts {
            stderr_tail: "ECONNRESET",
            ..FailureFacts::at(FailureStage::Run(Ended::TimedOut))
        };
        assert_eq!(classify_failure(&timed_out), OwnStop(OwnStopKind::TimedOut));
        let budget_kill = FailureFacts {
            budget_stop: true,
            ..FailureFacts::at(FailureStage::Run(Ended::Stopped))
        };
        assert_eq!(classify_failure(&budget_kill), OwnStop(OwnStopKind::Budget));
        let limit_reached = FailureFacts {
            stderr_tail: "API Error: 529 overloaded",
            ..FailureFacts::at(FailureStage::Run(Ended::LimitReached))
        };
        assert_eq!(
            classify_failure(&limit_reached),
            OwnStop(OwnStopKind::Budget)
        );
        // Paperclip PR 15b: a watchdog stall is Hollow's own stop, never
        // transient, even when the SIGTERM it sent shows in the status.
        let stalled = FailureFacts {
            stderr_tail: "fetch failed: ECONNRESET",
            signal: Some(15),
            ..FailureFacts::at(FailureStage::Run(Ended::Stalled))
        };
        assert_eq!(classify_failure(&stalled), OwnStop(OwnStopKind::Stalled));
        assert!(!classify_failure(&stalled).is_transient());
        let cli_budget = FailureFacts {
            result_subtype: Some(SUBTYPE_MAX_BUDGET),
            ..exited()
        };
        assert_eq!(classify_failure(&cli_budget), OwnStop(OwnStopKind::Budget));

        // The envelope: rate limit and overload, then network, are transient.
        assert_eq!(
            envelope("Selected model is at capacity."),
            Transient(TransientKind::RateLimit)
        );
        assert_eq!(
            envelope(
                r#"API Error: 529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
            ),
            Transient(TransientKind::RateLimit)
        );
        assert_eq!(
            envelope(r#"API Error: 429 {"type":"error","error":{"type":"rate_limit_error"}}"#),
            Transient(TransientKind::RateLimit)
        );
        assert_eq!(
            stderr("ERROR: stream error: exceeded retry limit, last status: 429 Too Many Requests"),
            Transient(TransientKind::RateLimit)
        );
        assert_eq!(
            stderr("Error: read ECONNRESET"),
            Transient(TransientKind::Network)
        );
        assert_eq!(
            envelope("API Error: Connection error."),
            Transient(TransientKind::Network)
        );
        assert_eq!(
            envelope("API Error: 503 service unavailable"),
            Transient(TransientKind::Network)
        );
        assert_eq!(
            stderr("stream disconnected before completion: error sending request"),
            Transient(TransientKind::Network)
        );

        // Sign-in is checked first, so an expired login that also mentions a
        // network error never retries.
        assert_eq!(
            envelope("Failed to authenticate. API Error: 401 Invalid authentication credentials (after ECONNRESET)"),
            Permanent(PermanentKind::Auth)
        );
        assert_eq!(
            envelope("Invalid API key · Please run /login"),
            Permanent(PermanentKind::Auth)
        );
        assert_eq!(
            stderr("HTTP/1.1 401 Unauthorized"),
            Permanent(PermanentKind::Auth)
        );

        // A usage limit is permanent, and beats the 429 that often comes
        // with it.
        assert_eq!(
            envelope("Claude AI usage limit reached|1790374200"),
            Permanent(PermanentKind::Quota)
        );
        assert_eq!(
            stderr("You've hit your usage limit. Try again later. (last status: 429)"),
            Permanent(PermanentKind::Quota)
        );

        // Numbers need a prefix, and unknown text is permanent.
        assert_eq!(
            envelope("syntax error on line 429 of file"),
            Permanent(PermanentKind::Unknown)
        );
        assert_eq!(
            stderr("processed 503 notes, 500 skipped"),
            Permanent(PermanentKind::Unknown)
        );
        assert_eq!(
            envelope("something odd happened"),
            Permanent(PermanentKind::Unknown)
        );
        assert_eq!(
            classify_failure(&exited()),
            Permanent(PermanentKind::Unknown)
        );
        // The budget message alone is neither a rate limit nor a quota.
        assert_eq!(
            envelope("Reached maximum budget ($0.01)"),
            Permanent(PermanentKind::Unknown)
        );
        assert_eq!(
            classify_failure(&FailureFacts {
                result_subtype: Some(SUBTYPE_MAX_TURNS),
                ..exited()
            }),
            Permanent(PermanentKind::TurnLimit)
        );
        // "moderate limit" is not a rate limit.
        assert_eq!(
            envelope("hit a moderate limit"),
            Permanent(PermanentKind::Unknown)
        );

        // Paperclip PR 14a: the regexes moved to `result_kind` (shared with
        // chat). A RAW network error still retries; the classifier is never
        // handed a labelled string, whose "rate limited" would read as a
        // rate limit.
        assert_eq!(
            classify_failure(&FailureFacts {
                error: "ECONNRESET",
                ..exited()
            }),
            Transient(TransientKind::Network)
        );
        // Paperclip's added phrases keep 9b's order: a usage limit first.
        assert_eq!(
            envelope("You’ve hit your weekly limit"),
            Permanent(PermanentKind::Quota)
        );
        assert_eq!(
            stderr("We're currently experiencing high demand"),
            Transient(TransientKind::RateLimit)
        );
    }

    #[cfg(unix)]
    #[test]
    fn classify_failure_by_signal() {
        let signalled = |signal: i32, leader_first: bool| {
            classify_failure(&FailureFacts {
                signal: Some(signal),
                leader_exited_first: leader_first,
                // A signal decides before any envelope text.
                stderr_tail: "ECONNRESET",
                ..exited()
            })
        };
        for signal in [libc::SIGKILL, libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
            assert_eq!(
                signalled(signal, true),
                Transient(TransientKind::ExternalSignal)
            );
            // Without leader-first, Hollow's own settle may have sent it.
            assert_eq!(signalled(signal, false), Permanent(PermanentKind::Unknown));
        }
        for signal in [libc::SIGSEGV, libc::SIGABRT, libc::SIGBUS, libc::SIGILL] {
            assert_eq!(signalled(signal, true), Permanent(PermanentKind::Crash));
            assert_eq!(signalled(signal, false), Permanent(PermanentKind::Crash));
        }
        assert_eq!(
            signalled(libc::SIGPIPE, true),
            Permanent(PermanentKind::Unknown)
        );
    }

    #[cfg(unix)]
    #[test]
    fn exit_signal_reads_only_signal_terminations() {
        use std::os::unix::process::ExitStatusExt;
        let killed = std::process::ExitStatus::from_raw(libc::SIGKILL);
        assert_eq!(exit_signal(&killed), Some(libc::SIGKILL));
        let exited_one = std::process::ExitStatus::from_raw(1 << 8);
        assert_eq!(exited_one.code(), Some(1));
        assert_eq!(exit_signal(&exited_one), None);
    }

    /// The live `--max-budget-usd 0.01` capture (FINDINGS.md fact 3): its
    /// error result carries a subtype and no `result` text.
    #[test]
    fn the_captured_budget_result_is_hollows_own_stop_and_never_retries() {
        let fixture = include_str!("../fixtures/usage/claude-max-budget.jsonl");
        let mut state = crate::agent::ParseState::default();
        let mut result = None;
        for line in fixture.lines() {
            for event in crate::agent::parse_line(crate::agent::AgentKind::Claude, line, &mut state)
            {
                if let crate::agent::AgentEvent::Result {
                    text,
                    is_error,
                    subtype,
                    ..
                } = event
                {
                    result = Some((text, is_error, subtype));
                }
            }
        }
        let (text, is_error, subtype) = result.expect("the capture has a result record");
        assert!(is_error);
        assert_eq!(text, None, "the error result has no text");
        assert_eq!(subtype, SUBTYPE_MAX_BUDGET);

        let facts = FailureFacts {
            result_subtype: Some(subtype.as_str()),
            result_error: text.as_deref(),
            error: "claude exited exit status: 1",
            ..exited()
        };
        let class = classify_failure(&facts);
        assert_eq!(class, OwnStop(OwnStopKind::Budget));
        let at = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
        assert_eq!(retry_plan(&RetryInput { class, ..input(at) }), None);
    }

    fn input(started_at: DateTime<Utc>) -> RetryInput<'static> {
        RetryInput {
            slug: "morning-brief",
            title: "Morning brief",
            history_run_id: "run-1",
            source: RunSource::Cron,
            attempt: 0,
            queue_id: Some(41),
            priority: 3,
            class: Transient(TransientKind::Network),
            history_finished: true,
            history_flagged: false,
            outside_effects_unsafe: false,
            started_at,
            finished_at: started_at + Duration::seconds(95),
            error: "API Error: 503 service unavailable",
            trigger_reason: None,
            prior_last_run_at: None,
        }
    }

    /// Plan completion L2: a run that may have acted outside its files, or
    /// ran calls Hollow can't see into, never retries on its own, however
    /// transient its failure.
    #[test]
    fn a_run_with_outside_effects_never_retries_on_its_own() {
        let at = Utc.with_ymd_and_hms(2026, 10, 13, 9, 0, 0).unwrap();
        assert!(retry_plan(&input(at)).is_some());
        assert_eq!(
            retry_plan(&RetryInput {
                outside_effects_unsafe: true,
                ..input(at)
            }),
            None
        );
    }

    #[test]
    fn retry_plan_is_bounded_and_carries_the_exact_start_stamp() {
        let started = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap()
            + Duration::nanoseconds(123_456_789);
        let base = input(started);

        let plan = retry_plan(&base).expect("a cron network failure retries");
        assert_eq!(plan.not_before, base.finished_at + Duration::seconds(60));
        assert_eq!(plan.retry_of, Some(41));
        assert_eq!(
            plan.priority, 3,
            "the retry keeps the failed row's priority"
        );
        assert_eq!(plan.idempotency_key, "retry:morning-brief:run-1");
        assert_eq!(plan.payload.failed_run_id, "run-1");
        assert_eq!(plan.payload.title, "Morning brief");
        assert_eq!(plan.payload.class_label, "network or server error");
        assert_eq!(
            crate::trigger_evaluator::parse_rfc3339(&plan.payload.failed_started_at),
            Some(started),
            "the payload parses back to exactly the mark_started stamp"
        );
        let json = serde_json::to_value(&plan.payload).unwrap();
        for key in [
            "failedRunId",
            "failedStartedAt",
            "title",
            "errorExcerpt",
            "classLabel",
        ] {
            assert!(json.get(key).is_some(), "payload key {key}");
        }
        let trigger = RetryInput {
            source: RunSource::Trigger,
            class: Transient(TransientKind::ExternalSignal),
            ..base
        };
        assert!(retry_plan(&trigger).is_some());

        // Never more than one, never for an attended run, never for a
        // permanent failure or one of Hollow's own stops.
        let none = [
            RetryInput {
                source: RunSource::Manual,
                ..base
            },
            RetryInput {
                source: RunSource::Retry,
                ..base
            },
            RetryInput {
                source: RunSource::Chat,
                ..base
            },
            RetryInput {
                source: RunSource::Heartbeat,
                ..base
            },
            RetryInput { attempt: 1, ..base },
            RetryInput {
                class: Permanent(PermanentKind::Auth),
                ..base
            },
            RetryInput {
                class: Permanent(PermanentKind::Unknown),
                ..base
            },
            RetryInput {
                class: OwnStop(OwnStopKind::Budget),
                ..base
            },
            RetryInput {
                class: OwnStop(OwnStopKind::TimedOut),
                ..base
            },
            RetryInput {
                class: OwnStop(OwnStopKind::Cancelled),
                ..base
            },
            // PR 5: a flagged run, or one whose record could not be finished.
            RetryInput {
                history_flagged: true,
                ..base
            },
            RetryInput {
                history_finished: false,
                ..base
            },
            RetryInput {
                history_run_id: "",
                ..base
            },
        ];
        for case in none {
            assert_eq!(retry_plan(&case), None, "{case:?}");
        }
    }

    #[test]
    fn retry_row_validity_compares_parsed_instants() {
        let started = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap()
            + Duration::nanoseconds(123_456_789);
        let plan = retry_plan(&input(started)).unwrap();
        let failed = plan.payload.failed_started_at.as_str();
        let due = plan.not_before;

        // The failed run's own stamp, in any stored form, is not a later start.
        assert_eq!(retry_row_valid(Some(failed), failed, due, due), Ok(()));
        let stored_local = started
            .with_timezone(&chrono::FixedOffset::west_opt(4 * 3600).unwrap())
            .to_rfc3339_opts(SecondsFormat::Secs, false);
        assert_eq!(
            retry_row_valid(Some(stored_local.as_str()), failed, due, due),
            Ok(())
        );

        // A later start ends the retry silently.
        let later = (started + Duration::seconds(30)).to_rfc3339();
        assert_eq!(
            retry_row_valid(Some(later.as_str()), failed, due, due),
            Err(RetryRowSkip::RanSince)
        );
        // ...and takes precedence over staleness.
        let long_after = due + Duration::hours(2);
        assert_eq!(
            retry_row_valid(Some(later.as_str()), failed, due, long_after),
            Err(RetryRowSkip::RanSince)
        );

        // Nothing provable about a later start: the retry stays valid.
        assert_eq!(retry_row_valid(None, failed, due, due), Ok(()));
        assert_eq!(
            retry_row_valid(Some("not a date"), failed, due, due),
            Ok(())
        );
        assert_eq!(
            retry_row_valid(Some(later.as_str()), "garbled", due, due),
            Ok(())
        );

        // Stale only strictly past the 30-minute window.
        let edge = due + Duration::minutes(RETRY_STALE_AFTER_MIN);
        assert_eq!(retry_row_valid(Some(failed), failed, due, edge), Ok(()));
        assert_eq!(
            retry_row_valid(Some(failed), failed, due, edge + Duration::seconds(1)),
            Err(RetryRowSkip::Stale)
        );
        assert_eq!(RetryRowSkip::RanSince.as_str(), "ran_since");
        assert_eq!(RetryRowSkip::Stale.as_str(), "stale");
    }

    #[test]
    fn the_error_excerpt_is_one_redacted_bounded_line() {
        let long = format!(
            "line one\nAuthorization: Bearer sk-secret-value\n<b>{}</b> — end",
            "x".repeat(400)
        );
        let excerpt = error_excerpt(&long);
        assert!(!excerpt.contains('\n'));
        assert!(!excerpt.contains("sk-secret-value"));
        assert!(!excerpt.contains(['<', '>', '—']));
        assert_eq!(excerpt.chars().count(), ERROR_EXCERPT_CHARS);
        assert!(excerpt.ends_with('…'));
        assert_eq!(error_excerpt(" \n\t "), "no error details");
        assert_eq!(error_excerpt("short"), "short");
    }

    // --- A retry that never runs, against a real vault ---------------------

    /// A failed run's row, written the way `finalize` writes it.
    fn failed_row(vault: &Path, slug: &str, n: usize) {
        let error = format!("API Error: 503 service unavailable (run {n})");
        crate::vault_workflows::finalize_run(
            vault,
            slug,
            "error",
            Some(&error),
            1,
            None,
            crate::types::RunLogEntry {
                at: Utc::now().to_rfc3339(),
                status: "error".into(),
                duration_sec: 1,
                note: error.clone(),
                run_id: Some(format!("run-{n}")),
                error_tail: None,
                exit_kind: None,
            },
        )
        .unwrap();
    }

    fn streak_items(vault: &Path) -> Vec<crate::review_queue::ReviewItem> {
        crate::review_queue::list(vault, Some(workflow_hygiene::STREAK_KIND)).unwrap()
    }

    fn retry_payload(n: usize) -> Value {
        let at = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
        let plan = retry_plan(&RetryInput {
            history_run_id: "run-x",
            ..input(at)
        })
        .unwrap();
        let mut payload = serde_json::to_value(&plan.payload).unwrap();
        payload["failedRunId"] = Value::String(format!("run-{n}"));
        payload
    }

    #[test]
    fn a_skipped_retry_reports_the_original_failure() {
        let reasons = [
            RetrySkipReason::Stale,
            RetrySkipReason::BudgetPaused,
            RetrySkipReason::Refused,
            RetrySkipReason::Unavailable,
            RetrySkipReason::Missing,
            RetrySkipReason::Invalid,
            RetrySkipReason::NotStarted,
            RetrySkipReason::Interrupted,
            RetrySkipReason::EnqueueFailed,
        ];
        for reason in reasons {
            let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
            let v = dir.path();
            let w = crate::vault_workflows::seed_workflow(v, "Morning brief", "manual", "Brief.");
            failed_row(v, &w.slug, 0);
            let notice = plan_skipped_retry(v, &w.slug, &retry_payload(0), reason)
                .unwrap_or_else(|| panic!("{reason:?} reports the failure"));
            assert_eq!(notice.kind, "error", "{reason:?}");
            assert_eq!(notice.title, "Morning brief failed", "{reason:?}");
            assert!(
                notice.body.contains("The retry was skipped because"),
                "{}",
                notice.body
            );
            assert!(
                notice.body.contains("503"),
                "the original error: {}",
                notice.body
            );
            // PR 5 already warned about a refusal or unreadable records.
            let quiet = matches!(
                reason,
                RetrySkipReason::Refused | RetrySkipReason::Unavailable
            );
            assert_eq!(notice.notify_native, !quiet, "{reason:?}");
            assert!(streak_items(v).is_empty(), "one failure opens no streak");
        }

        // With two prior errors, the deferred outcome opens the streak, and
        // the streak's notification is the only one.
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::vault_workflows::seed_workflow(v, "Morning brief", "manual", "Brief.");
        for n in 0..3 {
            failed_row(v, &w.slug, n);
        }
        let notice = plan_skipped_retry(v, &w.slug, &retry_payload(2), RetrySkipReason::Stale)
            .expect("the streak notification");
        assert!(
            notice.title.contains("failed 3 times in a row"),
            "{}",
            notice.title
        );
        assert!(notice.notify_native);
        assert_eq!(streak_items(v).len(), 1);

        // Ran since: that later run already reported and counted.
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::vault_workflows::seed_workflow(v, "Morning brief", "manual", "Brief.");
        for n in 0..3 {
            failed_row(v, &w.slug, n);
        }
        assert!(
            plan_skipped_retry(v, &w.slug, &retry_payload(2), RetrySkipReason::RanSince).is_none()
        );
        assert!(streak_items(v).is_empty(), "nothing recorded");

        // A user cancel records the outcome with no banner.
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::vault_workflows::seed_workflow(v, "Morning brief", "manual", "Brief.");
        failed_row(v, &w.slug, 0);
        assert!(
            plan_skipped_retry(v, &w.slug, &retry_payload(0), RetrySkipReason::UserCancel)
                .is_none()
        );

        // notify: never silences the failure (never the streak).
        let payload = payload_of("brief", &retry_payload(0));
        assert!(skipped_retry_notification(
            &payload,
            "brief",
            RetrySkipReason::Stale,
            NotifyPolicy::Never,
            None
        )
        .is_none());
        // An unreadable payload still names the workflow.
        let stand_in = payload_of("brief", &Value::Null);
        assert_eq!(stand_in.title, "brief");
    }

    /// The run-log arithmetic behind the deferral: a deferred third failure
    /// plus a failed retry is one streak of four. `finalize`'s actual
    /// decision is driven in `workflows::tests::
    /// a_transient_failure_defers_its_streak_to_the_queued_retry`.
    #[test]
    fn a_retry_pending_failure_waits_to_count_toward_a_streak() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::vault_workflows::seed_workflow(v, "Morning brief", "manual", "Brief.");
        // Two earlier failures, counted as they happened.
        for n in 0..2 {
            failed_row(v, &w.slug, n);
            assert!(workflow_hygiene::record_outcome(
                v,
                &w.slug,
                "Morning brief",
                "error",
                &format!("run-{n}")
            )
            .is_none());
        }
        // The third failure plans a retry, so `finalize` defers its outcome:
        // no streak item yet.
        failed_row(v, &w.slug, 2);
        assert!(streak_items(v).is_empty());
        // The retry fails too: one item, counting all four.
        failed_row(v, &w.slug, 3);
        let streak =
            workflow_hygiene::record_outcome(v, &w.slug, "Morning brief", "error", "run-3")
                .expect("the retry's failure opens the streak");
        assert_eq!(streak.count, 4);
        assert_eq!(streak_items(v).len(), 1);

        // Had the retry succeeded instead, no item would open.
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::vault_workflows::seed_workflow(v, "Morning brief", "manual", "Brief.");
        for n in 0..3 {
            failed_row(v, &w.slug, n);
        }
        crate::vault_workflows::append_run_log(
            v,
            &w.slug,
            crate::types::RunLogEntry {
                at: Utc::now().to_rfc3339(),
                status: "success".into(),
                duration_sec: 2,
                note: "retry · wrote the brief".into(),
                run_id: Some("run-3".into()),
                error_tail: None,
                exit_kind: None,
            },
        )
        .unwrap();
        assert!(
            workflow_hygiene::record_outcome(v, &w.slug, "Morning brief", "success", "run-3")
                .is_none()
        );
        assert!(streak_items(v).is_empty());
    }

    #[test]
    fn the_retrying_notice_is_quiet_and_follows_notify() {
        let at = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
        let plan = retry_plan(&input(at)).unwrap();
        let notice = retrying_notification(
            "Morning brief",
            "morning-brief",
            &plan,
            NotifyPolicy::Always,
        )
        .unwrap();
        assert_eq!(
            notice.title,
            "Morning brief failed; retrying once in 1 minute"
        );
        assert!(!notice.notify_native);
        assert_eq!(notice.kind, "warning");
        assert!(retrying_notification(
            "Morning brief",
            "morning-brief",
            &plan,
            NotifyPolicy::Failures
        )
        .is_some());
        assert!(retrying_notification(
            "Morning brief",
            "morning-brief",
            &plan,
            NotifyPolicy::Never
        )
        .is_none());
    }
}
