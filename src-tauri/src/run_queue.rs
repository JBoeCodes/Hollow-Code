//! The durable automation queue (Paperclip PR 7): one `run_queue` table in the
//! session-store SQLite database, OUTSIDE the vault, keyed by canonical vault
//! path (the store's `#[cfg(test)]` root swap applies).
//!
//! Every automation that wants the full-vault lease becomes a row here first:
//! scheduled cron occurrences, trigger fires, Run now (`manual`), the
//! heartbeat, PR 9's retries and, since PR 7b, chat turns. One pump
//! (`run_pump`) claims the head row, decides it (`claim_verdict`: cheap facts,
//! then PR 5's permission verify), and admits it through
//! `automated_history::try_begin_owned`. This module is the storage layer
//! only: nothing here calls the permission check, takes the lease, or writes
//! the vault (the one exception, [`retire_legacy_deferrals`], deletes PR 2's
//! retired episode file at pump start).
//!
//! The enqueuers are `workflows::enqueue_due` (cron occurrences and trigger
//! fires, every 60s), `heartbeat::enqueue_if_due` (every 10 min) and Run now
//! (`workflows::run`, through [`enqueue_and_wait`]); chat turns
//! (`chat::accept_turn`, through [`enqueue_chat`], one live chat turn at a
//! time) since PR 7b, and PR 9's retries later. A person's send may queue
//! beside a non-person turn of the same conversation (a wake held for a card,
//! including just after that card was answered). The pump still runs one of
//! that conversation at a time.
//!
//! **Lifecycle.** `queued` → `claimed` (a compare-and-swap [`claim`]) →
//! `running` ([`mark_running`], after the lease is held) → one terminal
//! status: `done`, `failed`, `cancelled`, `skipped`, `coalesced` (absorbed into
//! a run that consumed it) or `interrupted` (Hollow stopped). A claim that
//! loses the lease race goes back to `queued` at the same id, so the same FIFO
//! place, without counting an attempt ([`requeue_waiting`]); a held or retried
//! row goes back with a `not_before` ([`defer`]). Every transition to a
//! terminal status goes through the single `terminal_update`, which also
//! clears a chat row's composed payload, keeping only an unattended turn's
//! `{"unattended":true}` (plus an automation turn's `auto-` `clientTurnId`)
//! or an agent-started turn's `{"appDepth":n}`
//! (overnight A1 and S5: a restart never resumes either as the person's)
//! (and its asked text once the
//! transcript holds it or the user withdrew the turn), records a completed
//! cron occurrence, and runs retention.
//!
//! **Coalescing and keys.** [`enqueue`] runs in one IMMEDIATE transaction: a
//! live row with the same idempotency key is a [`Enqueued::Duplicate`];
//! otherwise, for every source but chat and ingest, a `queued` row for the
//! same `(source, source_id)` and compatible host origin absorbs the request ([`Enqueued::Coalesced`]:
//! reasons merged, trigger fire facts unioned, a cron row moved to the newer
//! occurrence); otherwise a row is inserted. Keys: `cron:{slug}:{next_run_at}`,
//! `trigger_evaluator::fired_key`, `hb:{local date}:{ordinal}`; manual and
//! chat rows have none.
//!
//! **Waiting.** A queued row's wait for another writer's lease is its
//! [`WaitEpisode`], kept in the `waiting_*` columns (it replaced PR 2's
//! `.workflow-deferrals.json`). An episode never starts for a held row, for
//! the scheduler's own bookkeeping lease, or for the row's own workflow run.
//!
//! **Lock order.** `WAITERS` → `session_store::WRITE_LOCK`, never the reverse:
//! [`enqueue_and_wait`] holds `WAITERS` across the enqueue and the waiter
//! registration so a fast outcome cannot be missed, and [`resolve_waiters`]
//! takes `WAITERS` only after the terminal write has committed. Nothing in a
//! `with_write` closure calls out (lease, history, permission check, waiter),
//! and a caller must never drop or finish a `RunLease` while holding either.

use crate::automated_history::LeaseHolder;
use crate::session_store::{db_time, with_read, with_write};
use crate::trigger_evaluator::{merge_fired, FiredToken};
use crate::types::{RunLogEntry, RunQueueEvent, RunQueueItem, Workflow, WorkflowRunResult};
use crate::vault_workflows;
use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::{oneshot, Notify};

/// The Tauri event carrying a [`RunQueueEvent`].
pub(crate) const EVENT: &str = "run-queue-event";
/// Rows at or above this priority (triggers, retries, cron, the heartbeat)
/// wait for the pump's gap between automation runs; chat and manual rows do
/// not.
pub(crate) const GAP_PRIORITY: i64 = 2;
/// A wait episode whose last touch is older than this was not being watched,
/// so Hollow was closed or asleep rather than busy (PR 2's rule, unchanged).
pub(crate) const ABANDON_AFTER_MIN: i64 = 10;
/// Longest merged reason kept on a row.
pub(crate) const REASON_MAX_CHARS: usize = 500;
/// Longest asked text kept in a chat row's `detail`.
pub(crate) const DETAIL_MAX_CHARS: usize = 8_000;
const LABEL_MAX_CHARS: usize = 200;
/// Terminal rows older than this are pruned; live rows never are.
const KEEP_TERMINAL_DAYS: i64 = 30;
/// Only the newest this-many terminal rows are kept.
const KEEP_TERMINAL_ROWS: i64 = 500;
/// A retained `detail` (an interrupted chat turn's text) is dropped after this.
const KEEP_DETAIL_DAYS: i64 = 7;
/// An automation's chat turn (an `auto-` `clientTurnId`) requested within
/// this many hours outlives the count cap: a pending message checks for it
/// that long (`automation_in_chat::CHECK_CEILING_HOURS`).
const KEEP_AUTO_TURN_HOURS: i64 = crate::automation_in_chat::CHECK_CEILING_HOURS;
/// A row's `clientTurnId`, or NULL (a payload that isn't JSON reads as none).
const CLIENT_TURN_ID: &str =
    "CASE WHEN json_valid(payload_json) THEN json_extract(payload_json, '$.clientTurnId') END";

/// The row error for a claimed or running row found at startup.
pub(crate) const INTERRUPTED_RUNNING: &str = "Hollow stopped before this run finished";
/// The row error for a previous boot's queued Run now or chat turn.
pub(crate) const INTERRUPTED_QUEUED: &str = "Hollow stopped before this queued run started";
/// The row error, and the waiter's outcome, for a row cancelled while queued.
pub(crate) const CANCELLED_BEFORE_START: &str = "cancelled before it started";
/// PR 2's retired episode file at the vault root, deleted at pump start.
pub(crate) const LEGACY_DEFERRALS_FILE: &str = ".workflow-deferrals.json";

const LIVE: &str = "('queued', 'claimed', 'running')";
const TERMINAL: &str =
    "('done', 'failed', 'cancelled', 'skipped', 'coalesced', 'interrupted', 'paused')";
const COLUMNS: &str = "id, source, source_id, label, reason, payload_json, status, \
    coalesced_count, coalesced_into, idempotency_key, priority, attempt, retry_of, \
    requested_at, not_before, claimed_at, finished_at, history_run_id, error, \
    phase_timings_json, cancel_requested, held_reason, waiting_since, waiting_holder, \
    wait_count, last_waited_at, detail";

// ── Types ───────────────────────────────────────────────────────────────────

/// What asked for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
// Tests only: its derived union lists every variant, so the TS mirror check
// in `ts_bindings` can't miss one.
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum RunSource {
    Chat,
    Cron,
    Manual,
    Trigger,
    Retry,
    RestrictedResume,
    Heartbeat,
    Ingest,
}

impl RunSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RunSource::Chat => "chat",
            RunSource::Cron => "cron",
            RunSource::Manual => "manual",
            RunSource::Trigger => "trigger",
            RunSource::Retry => "retry",
            RunSource::RestrictedResume => "restrictedResume",
            RunSource::Heartbeat => "heartbeat",
            RunSource::Ingest => "ingest",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<RunSource> {
        Some(match value {
            "chat" => RunSource::Chat,
            "cron" => RunSource::Cron,
            "manual" => RunSource::Manual,
            "trigger" => RunSource::Trigger,
            "retry" => RunSource::Retry,
            "restrictedResume" => RunSource::RestrictedResume,
            "heartbeat" => RunSource::Heartbeat,
            "ingest" => RunSource::Ingest,
            _ => return None,
        })
    }

    /// Claim order, lowest first: chat 0, manual and ingest 1, trigger and
    /// retry 2, cron 3, heartbeat 4. PR 9 may pass a retry's explicitly.
    pub(crate) fn default_priority(self) -> i64 {
        match self {
            RunSource::Chat => 0,
            RunSource::Manual | RunSource::Ingest => 1,
            RunSource::Trigger | RunSource::Retry | RunSource::RestrictedResume => 2,
            RunSource::Cron => 3,
            RunSource::Heartbeat => 4,
        }
    }

    /// Rows whose `source_id` is a workflow slug.
    pub(crate) fn is_workflow(self) -> bool {
        matches!(
            self,
            RunSource::Cron
                | RunSource::Manual
                | RunSource::Trigger
                | RunSource::Retry
                | RunSource::RestrictedResume
        )
    }

    /// Whether a second request merges into a queued row for the same
    /// `(source, source_id)`. Chat turns and imports are distinct requests,
    /// and so are retries (Paperclip PR 9b): each stands for one failed run,
    /// keyed `retry:{slug}:{history_run_id}`, and the claim ends the older one
    /// silently once a later run has started ("ran since").
    pub(crate) fn coalesces(self) -> bool {
        !matches!(
            self,
            RunSource::Chat | RunSource::Ingest | RunSource::Retry | RunSource::RestrictedResume
        )
    }

    /// Whether the source's default priority skips the automation gap. The
    /// pump compares each row's own `priority`; this pins the defaults.
    #[cfg(test)]
    pub(crate) fn gap_exempt(self) -> bool {
        self.default_priority() < GAP_PRIORITY
    }
}

/// A row's place in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
// Tests only: its derived union lists every variant, so the TS mirror check
// in `ts_bindings` can't miss one.
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) enum RowStatus {
    Queued,
    Claimed,
    Running,
    Done,
    Failed,
    Cancelled,
    Skipped,
    Coalesced,
    Interrupted,
    Paused,
}

impl RowStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RowStatus::Queued => "queued",
            RowStatus::Claimed => "claimed",
            RowStatus::Running => "running",
            RowStatus::Done => "done",
            RowStatus::Failed => "failed",
            RowStatus::Cancelled => "cancelled",
            RowStatus::Skipped => "skipped",
            RowStatus::Coalesced => "coalesced",
            RowStatus::Interrupted => "interrupted",
            RowStatus::Paused => "paused",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<RowStatus> {
        Some(match value {
            "queued" => RowStatus::Queued,
            "claimed" => RowStatus::Claimed,
            "running" => RowStatus::Running,
            "done" => RowStatus::Done,
            "failed" => RowStatus::Failed,
            "cancelled" => RowStatus::Cancelled,
            "skipped" => RowStatus::Skipped,
            "coalesced" => RowStatus::Coalesced,
            "interrupted" => RowStatus::Interrupted,
            "paused" => RowStatus::Paused,
            _ => return None,
        })
    }

    pub(crate) fn is_live(self) -> bool {
        matches!(
            self,
            RowStatus::Queued | RowStatus::Claimed | RowStatus::Running
        )
    }

    pub(crate) fn is_terminal(self) -> bool {
        !self.is_live()
    }

    /// The row status for a finished run's history status (`success`,
    /// `error`, `cancelled`, `skipped`); anything else is a failure.
    pub(crate) fn from_run_status(status: &str) -> RowStatus {
        match status {
            "success" => RowStatus::Done,
            "cancelled" => RowStatus::Cancelled,
            "paused" => RowStatus::Paused,
            "skipped" => RowStatus::Skipped,
            _ => RowStatus::Failed,
        }
    }
}

/// One queued row's wait for another writer's lease. Built from the
/// `waiting_*` columns and the payload's `dueAt`; its endings are PR 2's
/// deferral-episode rows, unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WaitEpisode {
    /// The cron occurrence (`next_run_at`) being waited on, when cron-driven.
    pub due_at: Option<String>,
    /// True for a trigger row (its watermark was committed at enqueue).
    pub trigger: bool,
    pub first_waited_at: String,
    pub last_waited_at: String,
    /// Waits counted in this episode: once per scheduler tick and once per
    /// lost admission race.
    pub waits: u32,
    /// `LeaseHolder::describe` of the most recent real holder.
    pub holder: String,
}

fn parse_stamp(stamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(stamp.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

impl WaitEpisode {
    /// True when the workflow ran after this episode began (a manual run, or
    /// an admission that consumed the occurrence), so the episode ends
    /// without a row. `mark_status` stamps `last_run_at` at run start.
    pub(crate) fn ran_since(&self, workflow: &Workflow) -> bool {
        match (
            workflow.last_run_at.as_deref().and_then(parse_stamp),
            parse_stamp(&self.first_waited_at),
        ) {
            (Some(ran), Some(first)) => ran >= first,
            _ => false,
        }
    }

    /// True when nothing has touched the episode for more than
    /// [`ABANDON_AFTER_MIN`] minutes. While Hollow is open every scheduler
    /// tick that finds the row still blocked refreshes it, so only a closed
    /// or sleeping app lets an episode go quiet. An unreadable stamp counts
    /// as abandoned: nothing proves Hollow was watching.
    pub(crate) fn abandoned(&self, now: DateTime<Utc>) -> bool {
        match parse_stamp(&self.last_waited_at) {
            Some(last) => now - last > Duration::minutes(ABANDON_AFTER_MIN),
            None => true,
        }
    }

    /// Whole minutes since the episode began (0 when unreadable or future).
    pub(crate) fn waited_minutes(&self, now: DateTime<Utc>) -> i64 {
        parse_stamp(&self.first_waited_at)
            .map(|first| (now - first).num_minutes().max(0))
            .unwrap_or(0)
    }

    /// The `deferred` row a finally admitted run writes inside its own lease.
    /// When the episode went quiet first, only the wait that was actually
    /// observed is claimed, never the time Hollow spent closed.
    pub(crate) fn started_entry(&self, now: DateTime<Utc>) -> RunLogEntry {
        let until = if self.abandoned(now) {
            parse_stamp(&self.last_waited_at).unwrap_or(now)
        } else {
            now
        };
        vault_workflows::deferred_start_entry(
            &self.holder,
            self.waited_minutes(until),
            self.waits,
            now,
        )
    }

    /// The `skipped` row for an occurrence the holder kept past its grace.
    pub(crate) fn busy_skip_entry(&self, now: DateTime<Utc>, grace_min: u64) -> RunLogEntry {
        vault_workflows::busy_skip_entry(&self.holder, grace_min, self.waits, now)
    }

    /// The `skipped` row for an occurrence or fire that stopped being due.
    pub(crate) fn cleared_entry(&self, now: DateTime<Utc>) -> RunLogEntry {
        vault_workflows::deferral_cleared_entry(&self.holder, now)
    }

    /// The `skipped` row for an episode abandoned while Hollow was closed.
    pub(crate) fn abandoned_entry(&self, now: DateTime<Utc>) -> RunLogEntry {
        vault_workflows::abandoned_deferral_entry(&self.holder, now)
    }
}

/// One `run_queue` row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueueRow {
    pub id: i64,
    pub source: RunSource,
    pub source_id: Option<String>,
    pub label: String,
    pub reason: String,
    /// The due occurrence (`dueAt`), trigger fire facts (`fired`), or a chat
    /// turn's composed request. Never exposed to the UI.
    pub payload: Value,
    pub status: RowStatus,
    pub coalesced_count: u32,
    pub coalesced_into: Option<i64>,
    pub idempotency_key: Option<String>,
    pub priority: i64,
    /// Admissions that held the lease (counted by [`mark_running`] only).
    pub attempt: u32,
    pub retry_of: Option<i64>,
    pub requested_at: String,
    pub not_before: Option<String>,
    pub claimed_at: Option<String>,
    pub finished_at: Option<String>,
    pub history_run_id: Option<String>,
    pub error: Option<String>,
    pub phase_timings: Option<Value>,
    pub cancel_requested: bool,
    /// A permission hold (the run-log note), which is not a lease wait.
    pub held_reason: Option<String>,
    pub wait: Option<WaitEpisode>,
    /// Chat only: the user's asked text.
    pub detail: Option<String>,
}

/// A request to enqueue.
#[derive(Debug, Clone)]
pub(crate) struct NewRow {
    pub source: RunSource,
    pub source_id: Option<String>,
    pub label: String,
    pub reason: String,
    pub payload: Value,
    pub idempotency_key: Option<String>,
    pub priority: i64,
    pub retry_of: Option<i64>,
    /// Not claimable before this instant (PR 9's delayed retry).
    pub not_before: Option<DateTime<Utc>>,
    pub detail: Option<String>,
    /// Host-only provenance. Queue payloads and priority never authorize it.
    pub origin: crate::work_origin::Request,
}

impl NewRow {
    /// A row at the source's default priority, with an empty payload.
    pub(crate) fn new(
        source: RunSource,
        source_id: Option<&str>,
        label: &str,
        reason: &str,
    ) -> NewRow {
        NewRow {
            source,
            source_id: source_id.map(str::to_string),
            label: label.to_string(),
            reason: reason.to_string(),
            payload: Value::Object(Map::new()),
            idempotency_key: None,
            priority: source.default_priority(),
            retry_of: None,
            not_before: None,
            detail: None,
            origin: crate::work_origin::Request::default(),
        }
    }

    pub(crate) fn with_payload(mut self, payload: Value) -> NewRow {
        self.payload = payload;
        self
    }

    pub(crate) fn with_origin(mut self, origin: crate::work_origin::Request) -> NewRow {
        self.origin = origin;
        self
    }

    pub(crate) fn with_key(mut self, key: impl Into<String>) -> NewRow {
        self.idempotency_key = Some(key.into());
        self
    }
}

/// What [`enqueue`] did, with the row id that now carries the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Enqueued {
    Inserted(i64),
    Coalesced(i64),
    Duplicate(i64),
}

impl Enqueued {
    pub(crate) fn id(self) -> i64 {
        match self {
            Enqueued::Inserted(id) | Enqueued::Coalesced(id) | Enqueued::Duplicate(id) => id,
        }
    }

    /// The `run-queue-event` kind for this enqueue, if it changed anything.
    pub(crate) fn event_kind(self) -> Option<&'static str> {
        match self {
            Enqueued::Inserted(_) => Some("enqueued"),
            Enqueued::Coalesced(_) => Some("coalesced"),
            Enqueued::Duplicate(_) => None,
        }
    }
}

/// The next claimable row, or when one becomes claimable.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NextRow {
    Ready(QueueRow),
    /// Rows exist but none is claimable before this instant (a `not_before`,
    /// or the automation gap).
    NotBefore(DateTime<Utc>),
    Empty,
}

/// What [`cancel`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CancelOutcome {
    /// A queued row ended as `cancelled`; its waiters were resolved.
    Cancelled,
    /// A queued cron row: flagged, so the claim skips and bookkeeps the
    /// occurrence (advancing `next_run_at`) instead of silently dropping it.
    SkipRequested,
    /// A claimed row: flagged, so the pump stops the run right after
    /// `mark_running` (or skips it before admission).
    StopRequested,
    /// Already running: the caller stops it through its own stop path.
    Running {
        source: RunSource,
        source_id: Option<String>,
    },
    AlreadyFinished,
}

/// What a waiter (Run now's `run_workflow`) receives.
#[derive(Debug, Clone)]
pub(crate) enum RowOutcome {
    /// The executed workflow's result, receipt included.
    Workflow(Result<WorkflowRunResult, String>),
    /// The row ended without an executed result.
    Finished {
        status: RowStatus,
        error: Option<String>,
    },
}

// ── Waiters and wake ────────────────────────────────────────────────────────

type WaiterMap = HashMap<(PathBuf, i64), Vec<oneshot::Sender<RowOutcome>>>;

/// Keyed by (canonical vault, row id): ids restart at 1 per database, so two
/// vaults (or two tests) never resolve each other's waiters.
static WAITERS: Lazy<Mutex<WaiterMap>> = Lazy::new(|| Mutex::new(HashMap::new()));
static WAKE: Lazy<Notify> = Lazy::new(Notify::new);

fn canonical(vault: &Path) -> PathBuf {
    std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf())
}

/// Wake the pump: something was enqueued, cancelled or released.
pub(crate) fn wake() {
    WAKE.notify_one();
}

/// The pump waits on this between steps.
pub(crate) fn woken() -> &'static Notify {
    &WAKE
}

/// Enqueue and register a waiter for the resulting row in one step. `WAITERS`
/// is held across the enqueue transaction and the registration, and
/// [`resolve_waiters`] needs it too, so an outcome that is written the moment
/// the row commits still finds this waiter. Lock order: `WAITERS` →
/// `WRITE_LOCK`.
pub(crate) fn enqueue_and_wait(
    vault: &Path,
    row: NewRow,
    now: DateTime<Utc>,
) -> Result<(Enqueued, oneshot::Receiver<RowOutcome>), String> {
    let mut waiters = WAITERS.lock().unwrap_or_else(|e| e.into_inner());
    let enqueued = enqueue(vault, row, now)?;
    let (tx, rx) = oneshot::channel();
    waiters
        .entry((canonical(vault), enqueued.id()))
        .or_default()
        .push(tx);
    Ok((enqueued, rx))
}

/// Deliver `outcome` to every waiter on row `id`. Call only after the row's
/// terminal write has committed (never inside `with_write`). Returns how many
/// waiters were still listening.
pub(crate) fn resolve_waiters(vault: &Path, id: i64, outcome: RowOutcome) -> usize {
    let senders = WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(canonical(vault), id))
        .unwrap_or_default();
    senders
        .into_iter()
        .map(|sender| sender.send(outcome.clone()).is_ok())
        .filter(|delivered| *delivered)
        .count()
}

/// True when row `id` has a registered waiter.
#[cfg(test)]
pub(crate) fn has_waiters(vault: &Path, id: i64) -> bool {
    WAITERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(canonical(vault), id))
        .is_some_and(|senders| !senders.is_empty())
}

// ── Payload helpers ─────────────────────────────────────────────────────────

/// A cron row's payload: the occurrence it stands for.
pub(crate) fn cron_payload(due_at: &str) -> Value {
    let mut payload = Map::new();
    payload.insert("dueAt".into(), Value::String(due_at.to_string()));
    Value::Object(payload)
}

/// A trigger row's payload: the tokens that fired and their facts.
pub(crate) fn trigger_payload(fired: &[FiredToken]) -> Value {
    let mut payload = Map::new();
    payload.insert(
        "fired".into(),
        serde_json::to_value(fired).unwrap_or_else(|_| Value::Array(Vec::new())),
    );
    Value::Object(payload)
}

/// The occurrence a cron row stands for.
pub(crate) fn due_at(payload: &Value) -> Option<String> {
    payload
        .get("dueAt")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// A trigger row's fire facts (empty when absent or unreadable).
pub(crate) fn fired_tokens(payload: &Value) -> Vec<FiredToken> {
    payload
        .get("fired")
        .and_then(|fired| serde_json::from_value(fired.clone()).ok())
        .unwrap_or_default()
}

/// Merge a coalescing request's payload into a queued row's: the newer keys
/// win (a cron row moves to the newer `dueAt`), except that a trigger row's
/// fire facts are unioned.
pub(crate) fn merge_payload(source: RunSource, existing: &Value, incoming: &Value) -> Value {
    let mut merged = existing.as_object().cloned().unwrap_or_default();
    if let Some(newer) = incoming.as_object() {
        for (key, value) in newer {
            merged.insert(key.clone(), value.clone());
        }
    }
    if source == RunSource::Trigger {
        let fired = merge_fired(&fired_tokens(existing), &fired_tokens(incoming));
        merged.insert(
            "fired".into(),
            serde_json::to_value(fired).unwrap_or_else(|_| Value::Array(Vec::new())),
        );
    }
    Value::Object(merged)
}

fn cap_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// Reasons are `; `-separated fragments: merge without repeats, capped.
pub(crate) fn merge_reasons(existing: &str, incoming: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in existing.split("; ").chain(incoming.split("; ")) {
        let part = part.trim();
        if !part.is_empty() && !parts.contains(&part) {
            parts.push(part);
        }
    }
    cap_chars(&parts.join("; "), REASON_MAX_CHARS)
}

/// Whether `row` waits for the vault lease `holder` holds at all. A
/// workspace's chat turn never does (it holds only its own folder), so a wait
/// episode naming the holder would be false: on 2026-10-07 a chat queued
/// behind its own workspace's long turn read "Waiting for Notion sync" for
/// 82 minutes. Known tracker mirror writers use atomic per-file commits;
/// only an unknown tracker writer still holds scoped admission.
fn waits_for(vault: &Path, row: &QueueRow, holder: &LeaseHolder) -> bool {
    match crate::run_pump::chat_scope(vault, row) {
        None => true,
        Some(scope) => {
            matches!(holder, LeaseHolder::TrackerSync)
                && crate::automated_history::tracker_blocks_scope(vault, &scope)
        }
    }
}

/// A holder this row must never be recorded as waiting on: the scheduler's
/// own bookkeeping lease (dropped before any admission, so it only blocks by
/// accident of timing) and the row's own run (a workflow's, or the
/// heartbeat's for a heartbeat row).
fn names_no_one(row: &QueueRow, holder: &LeaseHolder) -> bool {
    if holder.is_scheduler() {
        return true;
    }
    match row.source {
        source if source.is_workflow() => row
            .source_id
            .as_deref()
            .is_some_and(|slug| holder.is_workflow(slug)),
        RunSource::Heartbeat => matches!(
            holder,
            LeaseHolder::Run { source: Some(source), .. } if source == "heartbeat"
        ),
        _ => false,
    }
}

// ── Reading rows ────────────────────────────────────────────────────────────

fn sql_error(what: &str) -> impl Fn(rusqlite::Error) -> String + '_ {
    move |error| format!("run queue {what}: {error}")
}

fn clamp_u32(value: i64) -> u32 {
    value.clamp(0, u32::MAX as i64) as u32
}

/// Map a row; `None` for a source or status this build does not know (a row
/// written by a newer Hollow), which is skipped rather than failing the queue.
fn read_row(row: &Row<'_>) -> rusqlite::Result<Option<QueueRow>> {
    let source: String = row.get(1)?;
    let status: String = row.get(6)?;
    let (Some(source), Some(status)) = (RunSource::parse(&source), RowStatus::parse(&status))
    else {
        return Ok(None);
    };
    let payload_text: String = row.get(5)?;
    let payload: Value =
        serde_json::from_str(&payload_text).unwrap_or_else(|_| Value::Object(Map::new()));
    let phase_timings: Option<String> = row.get(19)?;
    let waiting_since: Option<String> = row.get(22)?;
    let waiting_holder: Option<String> = row.get(23)?;
    let wait_count: i64 = row.get(24)?;
    let last_waited_at: Option<String> = row.get(25)?;
    let wait = waiting_since.map(|since| WaitEpisode {
        due_at: due_at(&payload),
        trigger: source == RunSource::Trigger,
        last_waited_at: last_waited_at.unwrap_or_else(|| since.clone()),
        first_waited_at: since,
        waits: clamp_u32(wait_count),
        holder: waiting_holder.unwrap_or_default(),
    });
    Ok(Some(QueueRow {
        id: row.get(0)?,
        source,
        source_id: row.get(2)?,
        label: row.get(3)?,
        reason: row.get(4)?,
        payload,
        status,
        coalesced_count: clamp_u32(row.get(7)?),
        coalesced_into: row.get(8)?,
        idempotency_key: row.get(9)?,
        priority: row.get(10)?,
        attempt: clamp_u32(row.get(11)?),
        retry_of: row.get(12)?,
        requested_at: row.get(13)?,
        not_before: row.get(14)?,
        claimed_at: row.get(15)?,
        finished_at: row.get(16)?,
        history_run_id: row.get(17)?,
        error: row.get(18)?,
        phase_timings: phase_timings.and_then(|text| serde_json::from_str(&text).ok()),
        cancel_requested: row.get::<_, i64>(20)? != 0,
        held_reason: row.get(21)?,
        wait,
        detail: row.get(26)?,
    }))
}

fn query_rows(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<QueueRow>, String> {
    let mut statement = conn.prepare(sql).map_err(sql_error("prepare"))?;
    let rows = statement
        .query_map(params, read_row)
        .map_err(sql_error("query"))?;
    let mut out = Vec::new();
    for row in rows {
        if let Some(row) = row.map_err(sql_error("row"))? {
            out.push(row);
        }
    }
    Ok(out)
}

/// Scheduling order is advisory: never rewrite persisted priority (which also
/// controls the automation gap). Only validated Direct workflow roots move into
/// the Run-now band, ahead of other rows there. Chat's earlier band is unchanged.
fn scheduling_key(row: &QueueRow, direct: &HashSet<i64>) -> (i64, bool, i64) {
    let promoted = direct.contains(&row.id);
    let priority = if promoted {
        row.priority.min(1)
    } else {
        row.priority
    };
    (priority, !(promoted && priority == 1), row.id)
}

/// Rows and retained origin are read in one SQLite snapshot. No History or
/// caller eligibility callback runs until the snapshot has been released.
fn ordered_rows_in(conn: &Connection, queued_only: bool) -> Result<Vec<QueueRow>, String> {
    let snapshot = if conn.is_autocommit() {
        Some(
            conn.unchecked_transaction()
                .map_err(sql_error("order snapshot"))?,
        )
    } else {
        None
    };
    let read = snapshot.as_ref().map_or(conn, |tx| &**tx);
    let statuses = if queued_only { "('queued')" } else { LIVE };
    let mut rows = query_rows(
        read,
        &format!("SELECT {COLUMNS} FROM run_queue WHERE status IN {statuses} ORDER BY priority,id"),
        params![],
    )?;
    let direct = crate::work_origin::direct_workflows_in(read, &rows);
    rows.sort_by_key(|row| {
        let state = match row.status {
            RowStatus::Running => 0,
            RowStatus::Claimed => 1,
            _ => 2,
        };
        (state, scheduling_key(row, &direct))
    });
    if let Some(snapshot) = snapshot {
        snapshot.commit().map_err(sql_error("order snapshot"))?;
    }
    Ok(rows)
}

pub(crate) fn row_by_id(conn: &Connection, id: i64) -> Result<Option<QueueRow>, String> {
    Ok(query_rows(
        conn,
        &format!("SELECT {COLUMNS} FROM run_queue WHERE id = ?1"),
        params![id],
    )?
    .into_iter()
    .next())
}

/// The newest row a history run ran as (main agent M4: which queued turn a
/// calling run is).
pub(crate) fn id_for_run(vault: &Path, run_id: &str) -> Result<Option<i64>, String> {
    if run_id.trim().is_empty() {
        return Ok(None);
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT id FROM run_queue WHERE history_run_id = ?1 ORDER BY id DESC LIMIT 1",
            params![run_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(sql_error("row for a run"))
    })
}

/// The history runs rows `ids` ran as (rows not run yet have none), in one
/// query (morning M1: the woken turns the overnight scope counts).
pub(crate) fn history_runs(vault: &Path, ids: &[i64]) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let marks = vec!["?"; ids.len()].join(",");
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT history_run_id FROM run_queue \
                 WHERE id IN ({marks}) AND history_run_id IS NOT NULL"
            ))
            .map_err(sql_error("runs of rows"))?;
        let runs = stmt
            .query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                r.get::<_, String>(0)
            })
            .map_err(sql_error("runs of rows"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error("runs of rows"));
        runs
    })
}

/// One row by id, whatever its status.
pub(crate) fn get(vault: &Path, id: i64) -> Result<Option<QueueRow>, String> {
    with_read(vault, |conn| row_by_id(conn, id))
}

/// Chat rows [`recover_at_startup`] ended mid-run ([`INTERRUPTED_RUNNING`],
/// with or without the leftover note) at or after `since`, newest first, each
/// with whether a later chat row exists for the same conversation (a turn
/// queued or run since, so the interrupted one is no longer its last).
/// A turn recovered as never started ([`INTERRUPTED_QUEUED`]) is not one:
/// its text goes back to the composer instead (overnight A1, resume after
/// restart, `restart_resume`).
pub(crate) fn chats_interrupted_mid_run(
    vault: &Path,
    since: DateTime<Utc>,
) -> Result<Vec<(QueueRow, bool)>, String> {
    with_read(vault, |conn| {
        let rows = query_rows(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE source = 'chat' AND status = 'interrupted' \
                 AND finished_at >= ?1 ORDER BY id DESC"
            ),
            params![db_time(since)],
        )?;
        let mut out = Vec::new();
        for row in rows {
            let mid_run = row
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with(INTERRUPTED_RUNNING));
            if !mid_run {
                continue;
            }
            let later: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM run_queue WHERE source = 'chat' \
                     AND source_id = ?1 AND id > ?2)",
                    params![row.source_id, row.id],
                    |r| r.get(0),
                )
                .map_err(sql_error("later chat row"))?;
            out.push((row, later));
        }
        Ok(out)
    })
}

// ── Enqueue ─────────────────────────────────────────────────────────────────

/// Dedupe by key, else coalesce into a queued row, else insert (see the
/// module doc). One IMMEDIATE transaction.
pub(crate) fn enqueue(vault: &Path, row: NewRow, now: DateTime<Utc>) -> Result<Enqueued, String> {
    with_write(vault, "run queue", |tx| enqueue_in(tx, &row, now))
}

pub(crate) fn enqueue_in(
    tx: &Transaction<'_>,
    row: &NewRow,
    now: DateTime<Utc>,
) -> Result<Enqueued, String> {
    let origin = crate::work_origin::prepare_in(tx, row, now)?;
    if let Some(key) = row.idempotency_key.as_deref() {
        let live: Option<i64> = tx
            .query_row(
                &format!(
                    "SELECT id FROM run_queue WHERE idempotency_key = ?1 AND status IN {LIVE} \
                     ORDER BY id LIMIT 1"
                ),
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql_error("dedupe"))?;
        if let Some(id) = live {
            if !crate::work_origin::compatible_in(tx, id, &origin)? {
                return Err(
                    "WORK_ORIGIN: duplicate queue key has a different request origin".into(),
                );
            }
            crate::work_origin::enqueued_in(tx, row, id, &origin)?;
            return Ok(Enqueued::Duplicate(id));
        }
    }
    let label = cap_chars(&row.label, LABEL_MAX_CHARS);
    if row.source.coalesces() {
        let candidates = query_rows(
            tx,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE status = 'queued' \
                 AND cancel_requested = 0 AND source = ?1 AND source_id IS ?2 \
                 ORDER BY id"
            ),
            params![row.source.as_str(), row.source_id],
        )?;
        let mut target = None;
        // An older incompatible request must not hide a later compatible one.
        for existing in candidates {
            if crate::work_origin::compatible_in(tx, existing.id, &origin)? {
                target = Some(existing);
                break;
            }
        }
        if let Some(existing) = target {
            let reason = merge_reasons(&existing.reason, &row.reason);
            let payload = merge_payload(row.source, &existing.payload, &row.payload);
            // A cron row stands for its newest occurrence; other rows keep
            // the key they were inserted under.
            let key = if row.source == RunSource::Cron && row.idempotency_key.is_some() {
                row.idempotency_key.clone()
            } else {
                existing.idempotency_key.clone()
            };
            tx.execute(
                "UPDATE run_queue SET reason = ?2, payload_json = ?3, \
                 coalesced_count = coalesced_count + 1, idempotency_key = ?4, label = ?5 \
                 WHERE id = ?1",
                params![existing.id, reason, payload.to_string(), key, label],
            )
            .map_err(sql_error("coalesce"))?;
            crate::work_origin::enqueued_in(tx, row, existing.id, &origin)?;
            return Ok(Enqueued::Coalesced(existing.id));
        }
    }
    tx.execute(
        "INSERT INTO run_queue(source, source_id, label, reason, payload_json, status, \
         idempotency_key, priority, retry_of, requested_at, not_before, detail) \
         VALUES(?1, ?2, ?3, ?4, ?5, 'queued', ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            row.source.as_str(),
            row.source_id,
            label,
            cap_chars(&row.reason, REASON_MAX_CHARS),
            row.payload.to_string(),
            row.idempotency_key,
            row.priority,
            row.retry_of,
            db_time(now),
            row.not_before.map(db_time),
            row.detail
                .as_deref()
                .map(|d| cap_chars(d, DETAIL_MAX_CHARS)),
        ],
    )
    .map_err(sql_error("insert"))?;
    let id = tx.last_insert_rowid();
    crate::work_origin::enqueued_in(tx, row, id, &origin)?;
    Ok(Enqueued::Inserted(id))
}

/// The row error of a claimed or running chat row that no in-memory turn
/// holds (Paperclip PR 7b): its terminal write failed, or its turn was lost.
pub(crate) const CHAT_ROW_LOST: &str = "Hollow lost track of this chat turn";

/// Whether a chat payload is the person's own send. Must match
/// `ChatTurnPayload::person_sent`: depth 0, not unattended, and no host
/// asker. A payload that is not an object, or that only carries `prompt`
/// (older rows and test rows), reads as the person's send. An empty or
/// absent `hostAsker` does too.
pub(crate) fn payload_closes_cards(payload: &Value) -> bool {
    let Some(map) = payload.as_object() else {
        return true;
    };
    let depth = map.get("appDepth").and_then(Value::as_u64).unwrap_or(0);
    let unattended = map
        .get("unattended")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let host = map
        .get("hostAsker")
        .and_then(Value::as_str)
        .filter(|asker| !asker.is_empty());
    depth == 0 && !unattended && host.is_none()
}

/// What [`enqueue_chat`] did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ChatEnqueue {
    /// Inserted as row `id`, at 1-based `position` among queued rows.
    Accepted { id: i64, position: u32 },
    /// Another chat turn is queued, or is the pump's active row: nothing was
    /// inserted.
    Busy,
}

/// Enqueue a chat turn (Paperclip PR 7b), in ONE IMMEDIATE transaction: one
/// chat turn at a time. `active` is the chat row the pump holds in memory
/// (`chat_rows::CHAT_ROW_ACTIVE`, which the caller keeps locked across this call:
/// lock order `CHAT_ROW_ACTIVE` → `WRITE_LOCK`). Any other `claimed` or
/// `running` chat row has no turn behind it and is ended `interrupted`
/// ([`CHAT_ROW_LOST`]), its turn and usage rows with it
/// ([`interrupt_chat_run`]), even when the send is then refused; a queued chat
/// row, or the active one, refuses the send, except a person's send beside a
/// non-person turn of the same conversation. Chat rows never coalesce, so an accepted send is always a new row.
pub(crate) fn enqueue_chat(
    vault: &Path,
    row: NewRow,
    active: &std::collections::HashSet<i64>,
    now: DateTime<Utc>,
) -> Result<ChatEnqueue, String> {
    if row.source != RunSource::Chat {
        return Err(format!("{} is not a chat row", row.source.as_str()));
    }
    with_write(vault, "run queue", |tx| {
        let live = query_rows(
            tx,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE source = 'chat' AND status IN {LIVE} \
                 ORDER BY id"
            ),
            params![],
        )?;
        let mut repaired = false;
        let mut busy = false;
        for existing in live {
            if existing.status == RowStatus::Queued || active.contains(&existing.id) {
                // One turn at a time PER CONVERSATION (build programme PR B4):
                // another conversation's live turn never refuses this one.
                if existing.source_id == row.source_id {
                    // A person's send may queue beside a non-person turn of
                    // this conversation (queued, claimed or running). The
                    // card may already be answered: Send resolves it before
                    // the answer is queued. A person turn still refuses it.
                    // The pump runs one of the conversation at a time.
                    let person_beside_hollow = payload_closes_cards(&row.payload)
                        && !payload_closes_cards(&existing.payload);
                    if !person_beside_hollow {
                        busy = true;
                    }
                }
                continue;
            }
            if terminal_update(
                tx,
                existing.id,
                RowStatus::Interrupted,
                Some(CHAT_ROW_LOST),
                now,
                Terminal::default(),
            )? {
                interrupt_chat_run(tx, &existing, now)?;
                repaired = true;
            }
        }
        if repaired {
            prune(tx, now)?;
        }
        if busy {
            return Ok(ChatEnqueue::Busy);
        }
        let id = enqueue_in(tx, &row, now)?.id();
        // Direct workflows only move within/into band 1. Ordinary chat's
        // earlier band is unchanged, so typing keeps the cheap count even
        // when many workflow payloads are queued. Nonstandard chat priorities
        // need the shared order only if a queued workflow can affect it.
        let unchanged_band = row.priority < 1 || !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_queue WHERE status='queued' AND source IN ('manual','cron','trigger','retry'))",
            [], |r| r.get::<_, bool>(0),
        ).map_err(sql_error("chat order"))?;
        let position = if unchanged_band {
            tx.query_row(
                "SELECT COUNT(*) FROM run_queue WHERE status='queued' AND (priority < ?1 OR (priority=?1 AND id<=?2))",
                params![row.priority,id], |r| r.get::<_,i64>(0),
            ).map_err(sql_error("chat position"))?
        } else {
            (ordered_rows_in(tx, true)?
                .iter()
                .position(|row| row.id == id)
                .ok_or_else(|| {
                    "run queue: inserted chat is missing from claim order".to_string()
                })?
                + 1) as i64
        };
        Ok(ChatEnqueue::Accepted {
            id,
            position: clamp_u32(position),
        })
    })
}

// ── Claim, contention, running ──────────────────────────────────────────────

/// The head claimable row: `queued`, in trusted scheduling order, skipping rows
/// with a future `not_before` and, while the gap is closed
/// (`gap_open_at > now`), every row at or above [`GAP_PRIORITY`].
#[cfg(test)]
pub(crate) fn next_ready(
    vault: &Path,
    now: DateTime<Utc>,
    gap_open_at: Option<DateTime<Utc>>,
) -> Result<NextRow, String> {
    next_ready_where(vault, now, gap_open_at, |_| true)
}

/// [`next_ready`] over only the queued rows `startable` accepts (build
/// programme PR B4: the pump skips a build workspace's turn whose workspace
/// already has a turn running, and when the vault is held, every row that
/// needs it). A skipped row sets no wake-up time: the pump is woken when the
/// run blocking it ends.
pub(crate) fn next_ready_where(
    vault: &Path,
    now: DateTime<Utc>,
    gap_open_at: Option<DateTime<Utc>>,
    startable: impl Fn(&QueueRow) -> bool,
) -> Result<NextRow, String> {
    let rows = with_read(vault, |conn| ordered_rows_in(conn, true))?;
    let gap = gap_open_at.filter(|open| *open > now);
    let mut wake_at: Option<DateTime<Utc>> = None;
    for row in rows {
        if !startable(&row) {
            continue;
        }
        // A chat row Stopped while claimed only ends at its next claim
        // (C-S3): no not-before or gap delays that.
        if row.source == RunSource::Chat && row.cancel_requested {
            return Ok(NextRow::Ready(row));
        }
        let mut ready_at = row
            .not_before
            .as_deref()
            .and_then(parse_stamp)
            .filter(|at| *at > now);
        if row.priority >= GAP_PRIORITY {
            if let Some(open) = gap {
                ready_at = Some(ready_at.map_or(open, |at| at.max(open)));
            }
        }
        match ready_at {
            None => return Ok(NextRow::Ready(row)),
            Some(at) => wake_at = Some(wake_at.map_or(at, |w| w.min(at))),
        }
    }
    Ok(wake_at.map(NextRow::NotBefore).unwrap_or(NextRow::Empty))
}

/// Compare-and-swap `queued` → `claimed`. `None` when the row is no longer
/// queued (cancelled, or claimed by someone else).
pub(crate) fn claim(vault: &Path, id: i64, now: DateTime<Utc>) -> Result<Option<QueueRow>, String> {
    with_write(vault, "run queue", |tx| {
        let changed = tx
            .execute(
                "UPDATE run_queue SET status = 'claimed', claimed_at = ?2 \
                 WHERE id = ?1 AND status = 'queued'",
                params![id, db_time(now)],
            )
            .map_err(sql_error("claim"))?;
        if changed == 0 {
            return Ok(None);
        }
        row_by_id(tx, id)
    })
}

/// A claimed row lost the admission race (another writer took the lease after
/// the pump's advisory check): back to `queued` at the same id, so the same
/// FIFO place, with no attempt counted. The row passed its checks to get this
/// far, so any earlier permission hold is cleared. Counts one wait and names
/// `holder`, starting an episode if none exists, unless the holder names no
/// one for this row. Returns true when this started the episode.
pub(crate) fn requeue_waiting(
    vault: &Path,
    id: i64,
    holder: &LeaseHolder,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    with_write(vault, "run queue", |tx| {
        let Some(row) = row_by_id(tx, id)? else {
            return Ok(false);
        };
        if row.status != RowStatus::Claimed {
            return Ok(false);
        }
        tx.execute(
            "UPDATE run_queue SET status = 'queued', claimed_at = NULL, held_reason = NULL \
             WHERE id = ?1",
            params![id],
        )
        .map_err(sql_error("requeue"))?;
        if names_no_one(&row, holder) {
            return Ok(false);
        }
        tx.execute(
            "UPDATE run_queue SET waiting_since = COALESCE(waiting_since, ?2), \
             last_waited_at = ?2, waiting_holder = ?3, wait_count = wait_count + 1 \
             WHERE id = ?1",
            params![id, db_time(now), holder.describe()],
        )
        .map_err(sql_error("requeue wait"))?;
        Ok(row.wait.is_none())
    })
}

/// Put a claimed row back as `queued`, not claimable before `not_before`.
/// `held` sets (or, when `None`, clears) the permission hold; `clear_wait`
/// ends the row's wait episode (a held row is paused, not waiting); `note`
/// records why in `error` (a bookkeeping failure, say).
pub(crate) fn defer(
    vault: &Path,
    id: i64,
    not_before: DateTime<Utc>,
    held: Option<&str>,
    clear_wait: bool,
    note: Option<&str>,
) -> Result<bool, String> {
    with_write(vault, "run queue", |tx| {
        let changed = tx
            .execute(
                "UPDATE run_queue SET status = 'queued', claimed_at = NULL, not_before = ?2, \
                 held_reason = ?3, error = ?4, \
                 waiting_since = CASE WHEN ?5 = 1 THEN NULL ELSE waiting_since END, \
                 waiting_holder = CASE WHEN ?5 = 1 THEN NULL ELSE waiting_holder END, \
                 last_waited_at = CASE WHEN ?5 = 1 THEN NULL ELSE last_waited_at END, \
                 wait_count = CASE WHEN ?5 = 1 THEN 0 ELSE wait_count END \
                 WHERE id = ?1 AND status IN ('claimed', 'queued')",
                params![id, db_time(not_before), held, note, clear_wait as i64],
            )
            .map_err(sql_error("defer"))?;
        Ok(changed == 1)
    })
}

/// Put every `claimed` row except `keep` back to `queued` (same id, so the
/// same FIFO place; its `cancel_requested` flag survives for the next claim to
/// honour). The pump calls this at the start of every step: it claims one row
/// at a time and awaits each admitted run before stepping again, so between
/// steps no row it owns can legitimately be `claimed`. One is left behind only
/// when the write that should have moved it on after a claim failed (a `defer`
/// or `requeue_waiting` that hit the session database's busy timeout), and
/// nothing else ever re-claims a claimed row: its Run now waiter would hang and
/// its workflow's cron and trigger rows would never be enqueued again. `keep`
/// holds the rows with a pending terminal write, which must not run again,
/// the live vault-lane row, and the active chat rows (A·R4: an active claim
/// is never queued again).
/// Reads first, so the common case (nothing stray) takes no write lock.
/// Returns the ids it released.
pub(crate) fn release_stray_claims(vault: &Path, keep: &[i64]) -> Result<Vec<i64>, String> {
    let stray: Vec<i64> = with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT id FROM run_queue WHERE status = 'claimed' ORDER BY id")
            .map_err(sql_error("find stray claims"))?;
        let ids = stmt
            .query_map(params![], |r| r.get::<_, i64>(0))
            .map_err(sql_error("find stray claims"))?
            .collect::<rusqlite::Result<Vec<i64>>>()
            .map_err(sql_error("find stray claims"))?;
        Ok(ids)
    })?
    .into_iter()
    .filter(|id| !keep.contains(id))
    .collect();
    if stray.is_empty() {
        return Ok(stray);
    }
    with_write(vault, "run queue", |tx| {
        let mut released = Vec::with_capacity(stray.len());
        for id in &stray {
            let changed = tx
                .execute(
                    "UPDATE run_queue SET status = 'queued', claimed_at = NULL \
                     WHERE id = ?1 AND status = 'claimed'",
                    params![id],
                )
                .map_err(sql_error("release stray claim"))?;
            if changed == 1 {
                released.push(*id);
            }
        }
        Ok(released)
    })
}

/// A claimed row whose run now holds the lease: `running`, one attempt
/// counted, the history run recorded, and `queueWaitMs` (requested → now)
/// written to the phase timings. Clears any hold or `not_before`. Returns
/// false when the row was not claimed; the pump runs the admitted run anyway.
/// An `Err` (a busy database) is different: the pump finalizes the admitted
/// run unexecuted and queues the row again (merged plan slice 3, A·R4).
pub(crate) fn mark_running(
    vault: &Path,
    id: i64,
    history_run_id: &str,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    with_write(vault, "run queue", |tx| {
        let Some(row) = row_by_id(tx, id)? else {
            return Ok(false);
        };
        if row.status != RowStatus::Claimed {
            return Ok(false);
        }
        crate::work_origin::bind_running_in(tx, &row, history_run_id)?;
        let wait_ms = parse_stamp(&row.requested_at)
            .map(|requested| (now - requested).num_milliseconds().max(0))
            .unwrap_or(0);
        let mut timings = row
            .phase_timings
            .as_ref()
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        timings.insert("queueWaitMs".into(), Value::from(wait_ms));
        // SE0: how long the recorded hold kept the row out, so the next
        // investigation reads a cause and a duration, not an inference.
        if let Some(held_since) = timings
            .get("holdSince")
            .and_then(Value::as_str)
            .and_then(parse_stamp)
        {
            let held_ms = (now - held_since).num_milliseconds().max(0);
            timings.insert("holdMs".into(), Value::from(held_ms));
            // The clock ends here: a retried row's next hold starts its own.
            timings.remove("holdSince");
            if held_ms > HOLD_TRACE_MS {
                eprintln!(
                    "[run queue] row {id} waited {held_ms} ms on {}",
                    timings
                        .get("holdKind")
                        .and_then(Value::as_str)
                        .unwrap_or("a hold")
                );
            }
        }
        tx.execute(
            "UPDATE run_queue SET status = 'running', attempt = attempt + 1, \
             history_run_id = ?2, phase_timings_json = ?3, held_reason = NULL, \
             not_before = NULL WHERE id = ?1",
            params![id, history_run_id, Value::Object(timings).to_string()],
        )
        .map_err(sql_error("mark running"))?;
        Ok(true)
    })
}

/// Paperclip PR 15c: the `queueWaitMs` [`mark_running`] recorded on the
/// newest row that ran history run `history_run_id`. `Ok(None)` when no row
/// ran it (a run admitted outside the queue: tests, the heartbeat's Run now)
/// or the row has no wait. A plain read: no lock, no transaction.
pub(crate) fn queue_wait_for_run(
    vault: &Path,
    history_run_id: &str,
) -> Result<Option<u64>, String> {
    if history_run_id.trim().is_empty() {
        return Ok(None);
    }
    with_read(vault, |conn| {
        let text: Option<Option<String>> = conn
            .query_row(
                "SELECT phase_timings_json FROM run_queue WHERE history_run_id = ?1 \
                 ORDER BY id DESC LIMIT 1",
                params![history_run_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql_error("queue wait"))?;
        Ok(text
            .flatten()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|timings| timings.get("queueWaitMs").and_then(Value::as_u64)))
    })
}

/// Paperclip PR 15c: merge a finished run's phase timings (the receipt's
/// camelCase keys) into the `phase_timings_json` of the `running` row that
/// ran history run `history_run_id`. Keys are merged, and `queueWaitMs` is
/// never overwritten (the row's own measurement is the authority). Returns
/// `Ok(false)` when no running row matches: a run admitted outside the queue,
/// or a row that already ended. One IMMEDIATE transaction; nothing in it calls
/// out.
pub(crate) fn merge_phase_timings(
    vault: &Path,
    history_run_id: &str,
    timings: &Map<String, Value>,
) -> Result<bool, String> {
    if history_run_id.trim().is_empty() {
        return Ok(false);
    }
    with_write(vault, "run queue", |tx| {
        let found: Option<(i64, Option<String>)> = tx
            .query_row(
                "SELECT id, phase_timings_json FROM run_queue \
                 WHERE history_run_id = ?1 AND status = ?2 ORDER BY id DESC LIMIT 1",
                params![history_run_id, RowStatus::Running.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(sql_error("phase timings"))?;
        let Some((id, text)) = found else {
            return Ok(false);
        };
        let mut merged = text
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        for (key, value) in timings {
            if key == "queueWaitMs" {
                continue;
            }
            merged.insert(key.clone(), value.clone());
        }
        tx.execute(
            "UPDATE run_queue SET phase_timings_json = ?2 WHERE id = ?1 AND status = ?3",
            params![
                id,
                Value::Object(merged).to_string(),
                RowStatus::Running.as_str()
            ],
        )
        .map_err(sql_error("merge phase timings"))?;
        Ok(true)
    })
}

/// A hold longer than this leaves a one-line pump trace when the row starts.
const HOLD_TRACE_MS: i64 = 2000;

/// SE0: record why queued row `id` is held (`kind`, a [`crate::run_pump::Hold`]
/// kind) and since when, in its phase timings, so [`mark_running`] can write
/// `holdMs`. The first sight starts the clock; a changed kind updates only
/// `holdKind`. The same kind causes no rewrite on the next poll.
pub(crate) fn record_hold(
    vault: &Path,
    id: i64,
    kind: &str,
    now: DateTime<Utc>,
) -> Result<(), String> {
    with_write(vault, "run queue", |tx| {
        let Some(row) = row_by_id(tx, id)?.filter(|row| row.status == RowStatus::Queued) else {
            return Ok(());
        };
        let mut timings = row
            .phase_timings
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        if timings.get("holdKind").and_then(Value::as_str) == Some(kind)
            && timings.contains_key("holdSince")
        {
            return Ok(());
        }
        timings.insert("holdKind".into(), Value::from(kind));
        timings
            .entry("holdSince".to_string())
            .or_insert_with(|| Value::from(db_time(now)));
        tx.execute(
            "UPDATE run_queue SET phase_timings_json = ?2 WHERE id = ?1 AND status = 'queued'",
            params![id, Value::Object(timings).to_string()],
        )
        .map_err(sql_error("record hold"))?;
        Ok(())
    })
}

/// Retain the latest pressure level and first hold time without making a
/// permission hold. The same sample causes no rewrite on the next poll.
pub(crate) fn record_memory_hold(
    vault: &Path,
    id: i64,
    level: i32,
    now: DateTime<Utc>,
) -> Result<(), String> {
    with_write(vault, "run queue", |tx| {
        let Some(row) = row_by_id(tx, id)?.filter(|row| row.status == RowStatus::Queued) else {
            return Ok(());
        };
        let mut timings = row
            .phase_timings
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        if timings.get("memoryPressureLevel") == Some(&Value::from(level)) {
            return Ok(());
        }
        timings.insert("memoryPressureLevel".into(), Value::from(level));
        timings
            .entry("memoryHeldSince".to_string())
            .or_insert_with(|| Value::from(db_time(now)));
        timings.insert("holdKind".into(), Value::from("memory"));
        timings
            .entry("holdSince".to_string())
            .or_insert_with(|| Value::from(db_time(now)));
        tx.execute(
            "UPDATE run_queue SET phase_timings_json = ?2 WHERE id = ?1 AND status = 'queued'",
            params![id, Value::Object(timings).to_string()],
        )
        .map_err(sql_error("memory hold"))?;
        Ok(())
    })
}

// ── Terminal transitions ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
struct Terminal {
    coalesced_into: Option<i64>,
}

/// SQL, evaluated on a `run_queue` row: true once the chat turn's user
/// message is in its conversation's transcript, i.e. a session-store turn
/// exists for the row's history run (`session_store::start_turn`, which
/// `chat::execute_turn` calls before it announces the turn). The `turns`
/// table lives in the same database.
const INPUT_COMMITTED: &str =
    "EXISTS (SELECT 1 FROM turns WHERE turns.run_id = run_queue.history_run_id)";

/// THE terminal transition. Live rows only, so it is idempotent. Clears a
/// chat row's composed payload (operator and page context must not linger
/// for 30 days); an automation's chat turn keeps its `auto-` `clientTurnId`
/// (`automation_in_chat::turn_id`), so a pending message whose send landed
/// is recognised after its turn ends and never sent twice. Its asked text is the turn's recovery payload (merged plan
/// slice 3, B·F10): it is cleared only once the transcript holds the message
/// ([`INPUT_COMMITTED`]) or the user withdrew the turn (`cancelled`: the
/// composer already has the text). A turn that ended before its transcript
/// commit (a crash between claim and turn start, a transcript that could not
/// be opened) keeps it, so the composer can restore it; retention drops it
/// after [`KEEP_DETAIL_DAYS`]. A completed cron run also records its
/// occurrence ([`record_occurrence`]).
fn terminal_update(
    tx: &Transaction<'_>,
    id: i64,
    status: RowStatus,
    error: Option<&str>,
    now: DateTime<Utc>,
    opts: Terminal,
) -> Result<bool, String> {
    if !status.is_terminal() {
        return Err(format!(
            "{} is not a terminal run-queue status",
            status.as_str()
        ));
    }
    let changed: Option<(String, Option<String>)> = tx
        .query_row(
            &format!(
                "UPDATE run_queue SET status = ?2, finished_at = ?3, error = ?4, \
                 coalesced_into = COALESCE(?5, coalesced_into), \
                 payload_json = CASE WHEN source != 'chat' THEN payload_json \
                 WHEN json_extract(payload_json, '$.unattended') = 1 \
                 AND substr(json_extract(payload_json, '$.clientTurnId'), 1, 5) = 'auto-' \
                 THEN json_object('unattended', json('true'), \
                 'clientTurnId', json_extract(payload_json, '$.clientTurnId')) \
                 WHEN json_extract(payload_json, '$.unattended') = 1 THEN '{{\"unattended\":true}}' \
                 WHEN COALESCE(json_extract(payload_json, '$.appDepth'), 0) > 0 \
                 THEN json_object('appDepth', json_extract(payload_json, '$.appDepth')) \
                 ELSE '{{}}' END, \
                 detail = CASE WHEN source = 'chat' \
                 AND (?2 = 'cancelled' OR {INPUT_COMMITTED}) THEN NULL ELSE detail END \
                 WHERE id = ?1 AND status IN {LIVE} RETURNING source, source_id"
            ),
            params![
                id,
                status.as_str(),
                db_time(now),
                error,
                opts.coalesced_into
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql_error("finish"))?;
    if changed.is_some() {
        record_occurrence(tx, id, status, now)?;
        crate::work_origin::terminal_in(tx, id)?;
    }
    Ok(changed.is_some())
}

/// Crash recovery's terminal funnel for a chat row that just ended
/// `interrupted` with no live turn behind it (merged plan slice 3, B·F10):
/// closes its history run's still-running session-store turn and usage rows
/// in the same transaction (`session_store::interrupt_chat_run_in`). Any
/// other row, or one with no history run yet, is left alone.
fn interrupt_chat_run(
    tx: &Transaction<'_>,
    row: &QueueRow,
    now: DateTime<Utc>,
) -> Result<(), String> {
    if row.source != RunSource::Chat {
        return Ok(());
    }
    if let Some(run) = row.history_run_id.as_deref() {
        crate::session_store::interrupt_chat_run_in(tx, run, now)?;
    }
    Ok(())
}

/// Whether a chat row's user message reached its transcript
/// ([`INPUT_COMMITTED`]).
fn input_committed(tx: &Transaction<'_>, id: i64) -> Result<bool, String> {
    tx.query_row(
        &format!("SELECT {INPUT_COMMITTED} FROM run_queue WHERE id = ?1"),
        params![id],
        |r| r.get::<_, bool>(0),
    )
    .optional()
    .map(|committed| committed.unwrap_or(false))
    .map_err(sql_error("input committed"))
}

// ── Completed occurrences ───────────────────────────────────────────────────

/// Completed cron occurrences are kept this long.
const KEEP_OCCURRENCE_DAYS: i64 = 90;

/// A cron row whose run started (it has a history run) and ended `done`,
/// `failed` or `cancelled` consumed its occurrence: record `(slug, dueAt)` in
/// `automation_occurrences`, in the same transaction as the terminal write
/// (merged plan slice 3, B·F10). The workflow file's `next_run_at` is what
/// normally moves the schedule on, but its write (`finalize_run`, beside the
/// run log) can fail, and the slot would then be due again on the next pass;
/// [`occurrence_completed`] lets `workflows::enqueue_due` refuse it. Nothing
/// is recorded for a row that never ran or was interrupted.
fn record_occurrence(
    tx: &Transaction<'_>,
    id: i64,
    status: RowStatus,
    now: DateTime<Utc>,
) -> Result<(), String> {
    if !matches!(
        status,
        RowStatus::Done | RowStatus::Failed | RowStatus::Cancelled
    ) {
        return Ok(());
    }
    let Some(row) = row_by_id(tx, id)? else {
        return Ok(());
    };
    if row.source != RunSource::Cron || row.history_run_id.is_none() {
        return Ok(());
    }
    let (Some(slug), Some(due)) = (row.source_id.as_deref(), due_at(&row.payload)) else {
        return Ok(());
    };
    tx.execute(
        "INSERT OR IGNORE INTO automation_occurrences(slug, due_at, status, finished_at) \
         VALUES(?1, ?2, ?3, ?4)",
        params![slug, due, status.as_str(), db_time(now)],
    )
    .map_err(sql_error("record occurrence"))?;
    tx.execute(
        "DELETE FROM automation_occurrences WHERE finished_at < ?1",
        params![db_time(now - Duration::days(KEEP_OCCURRENCE_DAYS))],
    )
    .map_err(sql_error("prune occurrences"))?;
    Ok(())
}

/// True when a run of `slug`'s cron occurrence `due` (the workflow's
/// `next_run_at` string, as queued) already completed.
pub(crate) fn occurrence_completed(vault: &Path, slug: &str, due: &str) -> Result<bool, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT 1 FROM automation_occurrences WHERE slug = ?1 AND due_at = ?2",
            params![slug, due],
            |_| Ok(()),
        )
        .optional()
        .map(|found| found.is_some())
        .map_err(sql_error("occurrence completed"))
    })
}

/// Retention: terminal rows past 30 days or beyond the newest 500 go, and a
/// kept `detail` goes after 7 days. Live rows are never pruned. An
/// automation's chat turn requested within [`KEEP_AUTO_TURN_HOURS`] is
/// exempt from the count cap, so a pending message whose send landed still
/// finds it (`chat_row_with_turn_id`) however busy the queue was.
fn prune(tx: &Transaction<'_>, now: DateTime<Utc>) -> Result<(), String> {
    tx.execute(
        &format!("DELETE FROM run_queue WHERE status IN {TERMINAL} AND finished_at < ?1"),
        params![db_time(now - Duration::days(KEEP_TERMINAL_DAYS))],
    )
    .map_err(sql_error("prune by age"))?;
    tx.execute(
        &format!(
            "DELETE FROM run_queue WHERE status IN {TERMINAL} AND id NOT IN ( \
             SELECT id FROM run_queue WHERE status IN {TERMINAL} ORDER BY id DESC LIMIT ?1) \
             AND NOT (source = 'chat' AND requested_at >= ?2 \
             AND COALESCE(substr({CLIENT_TURN_ID}, 1, 5) = 'auto-', 0))"
        ),
        params![
            KEEP_TERMINAL_ROWS,
            db_time(now - Duration::hours(KEEP_AUTO_TURN_HOURS))
        ],
    )
    .map_err(sql_error("prune by count"))?;
    tx.execute(
        &format!(
            "UPDATE run_queue SET detail = NULL WHERE status IN {TERMINAL} \
             AND detail IS NOT NULL AND finished_at < ?1"
        ),
        params![db_time(now - Duration::days(KEEP_DETAIL_DAYS))],
    )
    .map_err(sql_error("prune detail"))?;
    Ok(())
}

/// End a live row with a terminal `status`. Returns false when it had already
/// ended. Resolving its waiters is the caller's next step, after this returns.
pub(crate) fn finish(
    vault: &Path,
    id: i64,
    status: RowStatus,
    error: Option<&str>,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    let changed = with_write(vault, "run queue", |tx| {
        let changed = terminal_update(tx, id, status, error, now, Terminal::default())?;
        prune(tx, now)?;
        Ok(changed)
    })?;
    if changed {
        wake();
    }
    Ok(changed)
}

/// One row [`absorb_siblings`] ended into the admitted run: its id, source
/// and reason (Paperclip PR 9, 9a-2: the run's `{{trigger_reason}}` gains an
/// absorbed trigger fire's reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Absorbed {
    pub id: i64,
    pub source: RunSource,
    pub reason: String,
}

/// At admission: every other origin-compatible queued cron and trigger row
/// for `slug` is consumed by the admitted run (PR 2's rule that one run takes both a due
/// occurrence and a waiting fire). They end as `coalesced` into `admitted_id`,
/// which gains their reasons. Manual and retry rows are never absorbed.
/// Returns the rows it ended.
pub(crate) fn absorb_siblings(
    vault: &Path,
    admitted_id: i64,
    slug: &str,
    now: DateTime<Utc>,
) -> Result<Vec<Absorbed>, String> {
    with_write(vault, "run queue", |tx| {
        let siblings = query_rows(
            tx,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE status = 'queued' AND id != ?1 \
                 AND source IN ('cron', 'trigger') AND source_id = ?2 ORDER BY id"
            ),
            params![admitted_id, slug],
        )?;
        if siblings.is_empty() {
            return Ok(Vec::new());
        }
        let Some(admitted) = row_by_id(tx, admitted_id)? else {
            return Ok(Vec::new());
        };
        let mut reason = admitted.reason.clone();
        let mut absorbed = Vec::with_capacity(siblings.len());
        for sibling in &siblings {
            if !crate::work_origin::absorb_in(tx, &admitted, sibling)? {
                continue;
            }
            reason = merge_reasons(&reason, &sibling.reason);
            let opts = Terminal {
                coalesced_into: Some(admitted_id),
            };
            if terminal_update(tx, sibling.id, RowStatus::Coalesced, None, now, opts)? {
                absorbed.push(Absorbed {
                    id: sibling.id,
                    source: sibling.source,
                    reason: sibling.reason.clone(),
                });
            }
        }
        tx.execute(
            "UPDATE run_queue SET reason = ?2, coalesced_count = coalesced_count + ?3 WHERE id = ?1",
            params![admitted_id, reason, absorbed.len() as i64],
        )
        .map_err(sql_error("absorb"))?;
        prune(tx, now)?;
        Ok(absorbed)
    })
}

// ── Waiting episodes ────────────────────────────────────────────────────────

/// The pump found `holder` on the lease: start an episode on every queued,
/// unheld row that has none, so a `waiting` event can go out promptly. Never
/// counts a wait (that is the tick's job) and never renames an episode.
/// Returns the rows whose episode this started.
pub(crate) fn start_waiting_episodes(
    vault: &Path,
    holder: &LeaseHolder,
    now: DateTime<Utc>,
) -> Result<Vec<i64>, String> {
    if holder.is_scheduler() {
        return Ok(Vec::new());
    }
    with_write(vault, "run queue", |tx| {
        let rows = query_rows(
            tx,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE status = 'queued' \
                 AND waiting_since IS NULL AND held_reason IS NULL ORDER BY priority, id"
            ),
            params![],
        )?;
        let stamp = db_time(now);
        let name = holder.describe();
        let mut started = Vec::new();
        for row in rows
            .iter()
            .filter(|row| !names_no_one(row, holder) && waits_for(vault, row, holder))
        {
            tx.execute(
                "UPDATE run_queue SET waiting_since = ?2, last_waited_at = ?2, \
                 waiting_holder = ?3 WHERE id = ?1",
                params![row.id, stamp, name],
            )
            .map_err(sql_error("start waiting"))?;
            started.push(row.id);
        }
        Ok(started)
    })
}

/// Once per scheduler tick while `holder` has the lease: every queued, unheld
/// row it blocks counts one wait, refreshes its clock (so it never looks
/// abandoned while Hollow is open) and names the holder, starting an episode
/// if it had none. The same exclusions as [`start_waiting_episodes`]. Returns
/// how many rows it touched.
pub(crate) fn touch_waiting(
    vault: &Path,
    holder: &LeaseHolder,
    now: DateTime<Utc>,
) -> Result<usize, String> {
    if holder.is_scheduler() {
        return Ok(0);
    }
    with_write(vault, "run queue", |tx| {
        let rows = query_rows(
            tx,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE status = 'queued' \
                 AND held_reason IS NULL ORDER BY priority, id"
            ),
            params![],
        )?;
        let stamp = db_time(now);
        let name = holder.describe();
        let mut touched = 0;
        for row in rows
            .iter()
            .filter(|row| !names_no_one(row, holder) && waits_for(vault, row, holder))
        {
            tx.execute(
                "UPDATE run_queue SET waiting_since = COALESCE(waiting_since, ?2), \
                 last_waited_at = ?2, waiting_holder = ?3, wait_count = wait_count + 1 \
                 WHERE id = ?1",
                params![row.id, stamp, name],
            )
            .map_err(sql_error("touch waiting"))?;
            touched += 1;
        }
        Ok(touched)
    })
}

// ── Cancel ──────────────────────────────────────────────────────────────────

fn cancelled_outcome() -> RowOutcome {
    RowOutcome::Finished {
        status: RowStatus::Cancelled,
        error: Some(CANCELLED_BEFORE_START.to_string()),
    }
}

/// Cancel one row by status (see [`CancelOutcome`]). A cancelled row's
/// waiters are resolved after the write commits.
pub(crate) fn cancel(vault: &Path, id: i64, now: DateTime<Utc>) -> Result<CancelOutcome, String> {
    let outcome = with_write(vault, "run queue", |tx| {
        let Some(row) = row_by_id(tx, id)? else {
            return Err(format!("There is no queued run {id}."));
        };
        match row.status {
            RowStatus::Queued if row.source == RunSource::Cron => {
                tx.execute(
                    "UPDATE run_queue SET cancel_requested = 1 WHERE id = ?1",
                    params![id],
                )
                .map_err(sql_error("request skip"))?;
                Ok(CancelOutcome::SkipRequested)
            }
            RowStatus::Queued => {
                terminal_update(
                    tx,
                    id,
                    RowStatus::Cancelled,
                    Some(CANCELLED_BEFORE_START),
                    now,
                    Terminal::default(),
                )?;
                prune(tx, now)?;
                Ok(CancelOutcome::Cancelled)
            }
            RowStatus::Claimed => {
                tx.execute(
                    "UPDATE run_queue SET cancel_requested = 1 WHERE id = ?1",
                    params![id],
                )
                .map_err(sql_error("request stop"))?;
                Ok(CancelOutcome::StopRequested)
            }
            RowStatus::Running => Ok(CancelOutcome::Running {
                source: row.source,
                source_id: row.source_id.clone(),
            }),
            _ => Ok(CancelOutcome::AlreadyFinished),
        }
    })?;
    if outcome == CancelOutcome::Cancelled {
        resolve_waiters(vault, id, cancelled_outcome());
    }
    Ok(outcome)
}

/// True when a cancel was requested for row `id`.
pub(crate) fn cancel_requested(vault: &Path, id: i64) -> Result<bool, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT cancel_requested FROM run_queue WHERE id = ?1",
            params![id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map(|flag| flag.is_some_and(|flag| flag != 0))
        .map_err(sql_error("cancel requested"))
    })
}

// ── Listing ─────────────────────────────────────────────────────────────────

/// Cheap fallback-poll check, including rows held by limits or review cards.
pub(crate) fn has_queued_rows(vault: &Path) -> Result<bool, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_queue WHERE status = 'queued')",
            [],
            |row| row.get(0),
        )
        .map_err(sql_error("queued rows"))
    })
}

/// Every live row: running, then claimed, then queued in claim order.
pub(crate) fn live_rows(vault: &Path) -> Result<Vec<QueueRow>, String> {
    with_read(vault, |conn| ordered_rows_in(conn, false))
}

/// Conversation `key`'s newest chat row (its live turn, when it has one:
/// a conversation has at most one), if the queue still holds any (F10, D2:
/// `wait_for_session`).
pub(crate) fn newest_chat_row(vault: &Path, key: &str) -> Result<Option<QueueRow>, String> {
    with_read(vault, |conn| {
        query_rows(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE source = 'chat' AND source_id = ?1 \
                 ORDER BY id DESC LIMIT 1"
            ),
            params![key],
        )
        .map(|rows| rows.into_iter().next())
    })
}

/// Whether conversation `key` has any live chat row (queued, claimed or
/// running), not only its newest: a person's queued turn may sit beside a
/// running agent turn, and once that newer one is cancelled the older one
/// still runs (fresh follow-up re-review, 2026-11-12). One query on
/// `idx_run_queue_source`.
pub(crate) fn has_live_chat_row(vault: &Path, key: &str) -> Result<bool, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_queue WHERE source = 'chat' AND source_id = ?1 \
             AND status IN ('queued', 'claimed', 'running'))",
            params![key],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|e| e.to_string())
    })
}

/// Conversation `key`'s chat rows requested at or after `since` (a
/// [`db_time`] string), oldest first (morning M2: what became of a wake
/// whose send never settled).
pub(crate) fn chat_rows_since(
    vault: &Path,
    key: &str,
    since: &str,
) -> Result<Vec<QueueRow>, String> {
    with_read(vault, |conn| {
        query_rows(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE source = 'chat' AND source_id = ?1 \
                 AND requested_at >= ?2 ORDER BY id"
            ),
            params![key, since],
        )
    })
}

/// The first chat row in any conversation requested at or after `since` (a
/// [`db_time`] string) whose payload's `clientTurnId` is `turn_id`. An
/// automation's send is found wherever its chat's chain has moved since
/// (`automation_in_chat::already_sent`): a handover, or the Main agent's new
/// thread, makes another conversation current after the send landed.
pub(crate) fn chat_row_with_turn_id(
    vault: &Path,
    turn_id: &str,
    since: &str,
) -> Result<Option<i64>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            &format!(
                "SELECT id FROM run_queue WHERE source = 'chat' AND requested_at >= ?1 \
                 AND {CLIENT_TURN_ID} = ?2 ORDER BY id LIMIT 1"
            ),
            params![since, turn_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error("chat row by turn id"))
    })
}

/// The chat row queued under run-queue key `key`, in any status (a held
/// send's, `steering::queue_key`): kept while retention keeps the row.
pub(crate) fn row_with_key(vault: &Path, key: &str) -> Result<Option<i64>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT id FROM run_queue WHERE source = 'chat' AND idempotency_key = ?1 \
             ORDER BY id LIMIT 1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error("chat row by key"))
    })
}

/// Live rows (with each queued row's 1-based position in claim order), then
/// the newest `recent` terminal rows.
pub(crate) fn list(vault: &Path, recent: usize) -> Result<Vec<(QueueRow, Option<u32>)>, String> {
    let live = live_rows(vault)?;
    let finished = with_read(vault, |conn| {
        query_rows(
            conn,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE status IN {TERMINAL} \
                 ORDER BY finished_at DESC, id DESC LIMIT ?1"
            ),
            params![recent as i64],
        )
    })?;
    let mut position = 0u32;
    let mut out: Vec<(QueueRow, Option<u32>)> = live
        .into_iter()
        .map(|row| {
            let place = (row.status == RowStatus::Queued).then(|| {
                position += 1;
                position
            });
            (row, place)
        })
        .collect();
    out.extend(finished.into_iter().map(|row| (row, None)));
    Ok(out)
}

/// The UI's view of a row. The payload never crosses IPC.
pub(crate) fn to_item(row: &QueueRow, position: Option<u32>) -> RunQueueItem {
    RunQueueItem {
        id: row.id,
        source: row.source.as_str().to_string(),
        source_id: row.source_id.clone(),
        label: row.label.clone(),
        detail: row.detail.clone(),
        reason: row.reason.clone(),
        status: row.status.as_str().to_string(),
        priority: row.priority,
        position,
        attempt: row.attempt,
        coalesced_count: row.coalesced_count,
        requested_at: row.requested_at.clone(),
        claimed_at: row.claimed_at.clone(),
        finished_at: row.finished_at.clone(),
        history_run_id: row.history_run_id.clone(),
        error: row.error.clone(),
        waiting_since: row.wait.as_ref().map(|w| w.first_waited_at.clone()),
        // A chat row's wait on the app's own bookkeeping (a tracker sync's
        // write phase) is not shown as a hold: the row reads Working.
        waiting_holder: row
            .wait
            .as_ref()
            .filter(|w| {
                row.source != RunSource::Chat
                    || !crate::run_pump::is_internal_holder_text(&w.holder)
            })
            .map(|w| w.holder.clone()),
        held_reason: row.held_reason.clone(),
        cancel_requested: row.cancel_requested,
        waiting_reason: None,
        waiting_session: None,
        // Claimed and running read as Working; `run_pump::item_with_reason`
        // also says it for a queued chat row nothing truly holds back.
        working: (row.source == RunSource::Chat
            && matches!(row.status, RowStatus::Claimed | RowStatus::Running))
        .then_some(true),
    }
}

/// Every `run-queue-event` kind. The frontend's `RunQueueEventKind` is
/// generated from `ts_bindings::RunQueueEventKind`, which a test holds to this
/// list.
pub(crate) const EVENT_KINDS: [&str; 8] = [
    "enqueued",
    "coalesced",
    "waiting",
    "running",
    "finished",
    "cancelled",
    "requeued",
    "held",
];

/// A `run-queue-event` payload.
pub(crate) fn event(kind: &str, item: RunQueueItem) -> RunQueueEvent {
    debug_assert!(
        EVENT_KINDS.contains(&kind),
        "unknown run-queue-event kind {kind}"
    );
    RunQueueEvent {
        kind: kind.to_string(),
        item,
    }
}

// ── Startup ─────────────────────────────────────────────────────────────────

/// Run once by the queue owner before any claim. Claimed and running rows
/// belonged to a process that stopped: `interrupted`. Queued Run now and chat
/// rows requested before `boot_at` are interrupted too, never run unattended
/// (a `--hidden` login launch shows no window, and the request's permission
/// mode and page context are stale); a chat row keeps its asked text so the
/// composer can restore it. A claimed or running chat row whose message never
/// reached its transcript (Hollow stopped between the claim and the turn's
/// start) is recovered the same way, as the queued turn it still was: it ends
/// with [`INTERRUPTED_QUEUED`] and keeps its asked text (merged plan slice 3,
/// B·F10; it used to be cleared, losing the message). Queued cron, trigger,
/// heartbeat and retry rows stay, to be claimed (and labelled) normally. The
/// `boot_at` filter keeps a delayed recovery from touching this boot's own
/// rows. A claimed or running row whose history run is in `stopped_leftovers`
/// (the boot sweep ended processes that run left behind,
/// `process_group::orphans`) says so after [`INTERRUPTED_RUNNING`]. Returns
/// the rows it interrupted. A chat row's turn and usage rows are closed in
/// the same transaction ([`interrupt_chat_run`]), so a failure leaves all of
/// it to the pump's next recovery attempt.
pub(crate) fn recover_at_startup(
    vault: &Path,
    boot_at: DateTime<Utc>,
    now: DateTime<Utc>,
    stopped_leftovers: &HashSet<String>,
) -> Result<Vec<QueueRow>, String> {
    let stopped_error = interrupted_with_leftovers();
    with_write(vault, "run queue", |tx| {
        let mut ids = Vec::new();
        let unfinished = query_rows(
            tx,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE status IN ('claimed', 'running') ORDER BY id"
            ),
            params![],
        )?;
        for row in &unfinished {
            let stopped = row
                .history_run_id
                .as_ref()
                .is_some_and(|run| stopped_leftovers.contains(run));
            let unstarted_chat = row.source == RunSource::Chat && !input_committed(tx, row.id)?;
            let error = if unstarted_chat {
                INTERRUPTED_QUEUED
            } else if stopped {
                stopped_error.as_str()
            } else {
                INTERRUPTED_RUNNING
            };
            if terminal_update(
                tx,
                row.id,
                RowStatus::Interrupted,
                Some(error),
                now,
                Terminal::default(),
            )? {
                interrupt_chat_run(tx, row, now)?;
                ids.push(row.id);
            }
        }
        let stale_requests = query_rows(
            tx,
            &format!(
                "SELECT {COLUMNS} FROM run_queue WHERE status = 'queued' \
                 AND source IN ('manual', 'chat') AND requested_at < ?1 ORDER BY id"
            ),
            params![db_time(boot_at)],
        )?;
        for row in &stale_requests {
            if terminal_update(
                tx,
                row.id,
                RowStatus::Interrupted,
                Some(INTERRUPTED_QUEUED),
                now,
                Terminal::default(),
            )? {
                ids.push(row.id);
            }
        }
        let mut interrupted = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(row) = row_by_id(tx, id)? {
                interrupted.push(row);
            }
        }
        prune(tx, now)?;
        Ok(interrupted)
    })
}

/// The row error for an interrupted run whose leftover processes the boot
/// sweep stopped.
pub(crate) fn interrupted_with_leftovers() -> String {
    format!(
        "{INTERRUPTED_RUNNING}. {}",
        crate::process_group::orphans::LEFTOVER_STOPPED_NOTE
    )
}

/// Delete PR 2's `.workflow-deferrals.json` (the queue's waiting columns
/// replaced it) and return how many episodes it held; 0 when missing or
/// unreadable. In-flight episodes are not migrated: installing requires
/// quitting Hollow, so the first claim applies the honest misfire label.
/// Only safe because the file left `run_guard::WATCHED_STATE_FILES` in the
/// same change that made the pump's startup recovery its caller: otherwise a
/// tracked run live at that moment (an IPC-started chat turn) would be
/// flagged for the delete.
pub(crate) fn retire_legacy_deferrals(vault: &Path) -> Result<usize, String> {
    let path = vault.join(LEGACY_DEFERRALS_FILE);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(format!("stat legacy workflow deferrals: {error}")),
    };
    if meta.is_dir() {
        return Err(format!(
            "{LEGACY_DEFERRALS_FILE} is a directory; move it aside"
        ));
    }
    let episodes = if meta.file_type().is_symlink() {
        0
    } else {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|file| {
                file.get("episodes")
                    .and_then(Value::as_object)
                    .map(Map::len)
            })
            .unwrap_or(0)
    };
    std::fs::remove_file(&path).map_err(|e| format!("remove legacy workflow deferrals: {e}"))?;
    Ok(episodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_workflows::{seed_workflow_with, MISSED_WHILE_CLOSED_NOTE};
    use tempfile::TempDir;

    fn chat_holder() -> LeaseHolder {
        LeaseHolder::Run {
            id: "run-chat".into(),
            source: Some("chat".into()),
            source_id: None,
            started_at: None,
        }
    }

    fn workflow_holder(slug: &str) -> LeaseHolder {
        LeaseHolder::Run {
            id: format!("run-{slug}"),
            source: Some("workflow".into()),
            source_id: Some(slug.into()),
            started_at: None,
        }
    }

    fn row_for(vault: &Path, id: i64) -> QueueRow {
        get(vault, id).unwrap().expect("the row exists")
    }

    fn raw(vault: &Path, id: i64) -> (String, Option<String>) {
        with_read(vault, |conn| {
            conn.query_row(
                "SELECT payload_json, detail FROM run_queue WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap()
    }

    fn cron(slug: &str, due: &str) -> NewRow {
        NewRow::new(RunSource::Cron, Some(slug), slug, "scheduled")
            .with_payload(cron_payload(due))
            .with_key(format!("cron:{slug}:{due}"))
    }

    fn chat_row(key: &str, asked: &str) -> NewRow {
        let mut row = NewRow::new(RunSource::Chat, Some(key), "Chat turn", "chat")
            .with_payload(serde_json::json!({"prompt": "<page-context>secret</page-context>"}));
        row.detail = Some(asked.to_string());
        row
    }

    /// 2026-11-13: a held send's chat row is found by its key after a quit
    /// recovered it as never started, so the drain settles the send instead
    /// of sending it twice; another source's row with the key is not it.
    #[test]
    fn a_held_sends_chat_row_is_found_by_its_key_after_recovery() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let before = Utc::now() - Duration::minutes(5);
        let key = crate::steering::queue_key("held-00000000000000aa");
        assert_eq!(row_with_key(v, &key).unwrap(), None);
        let id = enqueue(
            v,
            chat_row("s-held-key", "later").with_key(key.clone()),
            before,
        )
        .unwrap()
        .id();
        assert_eq!(row_with_key(v, &key).unwrap(), Some(id));
        recover_at_startup(
            v,
            Utc::now() - Duration::minutes(1),
            Utc::now(),
            &HashSet::new(),
        )
        .unwrap();
        let row = row_for(v, id);
        assert_eq!(row.status, RowStatus::Interrupted);
        assert_eq!(
            row_with_key(v, &key).unwrap(),
            Some(id),
            "kept once it ended"
        );
        let other = crate::steering::queue_key("held-00000000000000bb");
        enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("f"), "F", "run").with_key(other.clone()),
            before,
        )
        .unwrap();
        assert_eq!(row_with_key(v, &other).unwrap(), None, "not a chat row");
    }

    /// What `chat::execute_turn` does before it announces a turn: the
    /// user's message goes into the conversation's transcript.
    fn commit_turn(vault: &Path, key: &str, run: &str, asked: &str) {
        crate::session_store::start_turn(vault, key, "claude", "/tmp", run, asked).unwrap();
    }

    /// Review of #1343: an automation's chat turn outlives the count cap
    /// while a pending message may still look for it
    /// (`automation_in_chat::already_sent`); older ones, and other chat rows,
    /// go as before.
    #[test]
    fn the_count_cap_keeps_a_recent_automation_chat_turn() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let recent = db_time(now - Duration::hours(KEEP_AUTO_TURN_HOURS - 1));
        let old = db_time(now - Duration::hours(KEEP_AUTO_TURN_HOURS + 1));
        with_write(v, "test", |tx| {
            // Oldest ids, so the count cap reaches them first.
            for (label, at, payload) in [
                ("auto-recent", &recent, r#"{"unattended":true,"clientTurnId":"auto-r1"}"#),
                ("auto-old", &old, r#"{"unattended":true,"clientTurnId":"auto-r2"}"#),
                ("person", &recent, r#"{"clientTurnId":"t-person"}"#),
                ("not-json", &recent, "auto-"),
            ] {
                tx.execute(
                    "INSERT INTO run_queue(source, source_id, label, status, priority, \
                     requested_at, finished_at, payload_json) \
                     VALUES('chat', 's-x', ?1, 'done', 0, ?2, ?2, ?3)",
                    params![label, at, payload],
                )
                .map_err(|e| e.to_string())?;
            }
            for _ in 0..KEEP_TERMINAL_ROWS {
                tx.execute(
                    "INSERT INTO run_queue(source, label, status, priority, requested_at, finished_at) \
                     VALUES('cron', 'bulk', 'done', 3, ?1, ?1)",
                    params![db_time(now)],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .unwrap();
        let pruner = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("p"), "P", "run now"),
            now,
        )
        .unwrap()
        .id();
        finish(v, pruner, RowStatus::Done, None, now).unwrap();
        let labels: Vec<String> = with_read(v, |conn| {
            let mut stmt = conn
                .prepare("SELECT label FROM run_queue WHERE source = 'chat' ORDER BY id")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(labels, vec!["auto-recent".to_string()]);
        let since = db_time(now - Duration::days(1));
        let kept = chat_row_with_turn_id(v, "auto-r1", &since).unwrap();
        assert!(kept.is_some(), "still found");
        assert_eq!(chat_row_with_turn_id(v, "auto-r2", &since).unwrap(), None);
    }

    /// Merged plan slice 3 (B·F10): Hollow stopped after the pump claimed a
    /// chat turn (or marked it running) but before the turn wrote the user's
    /// message to its transcript. Recovery used to clear the row's asked
    /// text with everything else, so the message was gone. Now the row is
    /// recovered as the queued turn it still was: interrupted with
    /// `INTERRUPTED_QUEUED`, text kept for the composer. A turn whose
    /// message did reach the transcript keeps no copy, as before.
    #[test]
    fn a_crash_between_claim_and_turn_start_loses_no_input() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let before_boot = Utc::now() - Duration::minutes(10);
        let boot_at = Utc::now() - Duration::minutes(1);
        let claimed = enqueue(
            v,
            chat_row("s-20260925-cccc0001", "draft the memo"),
            before_boot,
        )
        .unwrap()
        .id();
        claim(v, claimed, before_boot).unwrap().expect("claimed");
        let admitted = enqueue(
            v,
            chat_row("s-20260925-cccc0002", "add the budget"),
            before_boot,
        )
        .unwrap()
        .id();
        claim(v, admitted, before_boot).unwrap().expect("claimed");
        mark_running(v, admitted, "run-admitted", before_boot).unwrap();
        let started = enqueue(
            v,
            chat_row("s-20260925-cccc0003", "already in the transcript"),
            before_boot,
        )
        .unwrap()
        .id();
        claim(v, started, before_boot).unwrap().expect("claimed");
        mark_running(v, started, "run-started", before_boot).unwrap();
        commit_turn(
            v,
            "s-20260925-cccc0003",
            "run-started",
            "already in the transcript",
        );

        let interrupted = recover_at_startup(v, boot_at, Utc::now(), &HashSet::new()).unwrap();
        assert_eq!(interrupted.len(), 3);
        for (id, asked) in [(claimed, "draft the memo"), (admitted, "add the budget")] {
            let row = row_for(v, id);
            assert_eq!(row.status, RowStatus::Interrupted);
            assert_eq!(
                row.error.as_deref(),
                Some(INTERRUPTED_QUEUED),
                "row {id} never started"
            );
            assert_eq!(
                row.detail.as_deref(),
                Some(asked),
                "row {id}'s message is kept for the composer"
            );
            assert_eq!(raw(v, id).0, "{}", "the composed prompt still goes");
        }
        let row = row_for(v, started);
        assert_eq!(row.error.as_deref(), Some(INTERRUPTED_RUNNING));
        assert_eq!(row.detail, None, "the transcript has it");
    }

    fn ready_id(next: NextRow) -> i64 {
        match next {
            NextRow::Ready(row) => row.id,
            other => panic!("expected a ready row, got {other:?}"),
        }
    }

    /// R2: the boot sweep stopped processes left by run `run-left`, so its
    /// interrupted row says so; another interrupted row does not.
    #[test]
    fn an_interrupted_row_notes_the_leftovers_the_boot_sweep_stopped() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let before_boot = Utc::now() - Duration::minutes(10);
        let boot_at = Utc::now() - Duration::minutes(1);
        let mut rows = Vec::new();
        for (key, run) in [("a", "run-left"), ("b", "run-clean")] {
            let id = enqueue(
                v,
                NewRow::new(RunSource::Manual, Some(key), "A", "run now"),
                before_boot,
            )
            .unwrap()
            .id();
            claim(v, id, before_boot).unwrap().expect("claimed");
            mark_running(v, id, run, before_boot).unwrap();
            rows.push(id);
        }
        let stopped = HashSet::from(["run-left".to_string(), "run-not-in-this-vault".to_string()]);
        let interrupted = recover_at_startup(v, boot_at, Utc::now(), &stopped).unwrap();
        assert_eq!(interrupted.len(), 2);
        assert_eq!(
            row_for(v, rows[0]).error.as_deref(),
            Some(
                "Hollow stopped before this run finished. An earlier turn was still running \
                 after Hollow restarted; it was stopped."
            )
        );
        assert_eq!(row_for(v, rows[0]).status, RowStatus::Interrupted);
        assert_eq!(
            row_for(v, rows[1]).error.as_deref(),
            Some(INTERRUPTED_RUNNING)
        );
    }

    #[test]
    fn the_queue_survives_a_store_reopen_and_startup_interrupts_only_prior_boot_work() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let before_boot = Utc::now() - Duration::minutes(10);
        let boot_at = Utc::now() - Duration::minutes(1);

        let running = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("a"), "A", "run now"),
            before_boot,
        )
        .unwrap()
        .id();
        claim(v, running, before_boot).unwrap().expect("claimed");
        mark_running(v, running, "run-x", before_boot).unwrap();
        let claimed = enqueue(v, cron("b", "2026-09-25T07:00:00+00:00"), before_boot)
            .unwrap()
            .id();
        claim(v, claimed, before_boot).unwrap().expect("claimed");
        let old_manual = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("c"), "C", "run now"),
            before_boot,
        )
        .unwrap()
        .id();
        let old_chat = enqueue(
            v,
            chat_row("s-20260925-aaaa0001", "summarize my week"),
            before_boot,
        )
        .unwrap()
        .id();
        let old_cron = enqueue(v, cron("d", "2026-09-25T08:00:00+00:00"), before_boot)
            .unwrap()
            .id();
        let old_trigger = enqueue(
            v,
            NewRow::new(RunSource::Trigger, Some("e"), "E", "file changed"),
            before_boot,
        )
        .unwrap()
        .id();
        let old_heartbeat = enqueue(
            v,
            NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "tasks changed")
                .with_key("hb:2026-09-25:1"),
            before_boot,
        )
        .unwrap()
        .id();
        let this_boot_manual = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("f"), "F", "run now"),
            Utc::now(),
        )
        .unwrap()
        .id();

        // Every row survives fresh opens (each call opens its own connection).
        assert_eq!(list(v, 50).unwrap().len(), 8);

        let interrupted = recover_at_startup(v, boot_at, Utc::now(), &HashSet::new()).unwrap();
        let mut ids: Vec<i64> = interrupted.iter().map(|row| row.id).collect();
        ids.sort();
        assert_eq!(ids, vec![running, claimed, old_manual, old_chat]);
        for id in [running, claimed] {
            let row = row_for(v, id);
            assert_eq!(row.status, RowStatus::Interrupted);
            assert_eq!(row.error.as_deref(), Some(INTERRUPTED_RUNNING));
        }
        for id in [old_manual, old_chat] {
            let row = row_for(v, id);
            assert_eq!(row.status, RowStatus::Interrupted);
            assert_eq!(row.error.as_deref(), Some(INTERRUPTED_QUEUED));
        }
        assert_eq!(
            row_for(v, old_chat).detail.as_deref(),
            Some("summarize my week"),
            "a chat turn interrupted before it started keeps its text for the composer"
        );
        for id in [old_cron, old_trigger, old_heartbeat, this_boot_manual] {
            assert_eq!(
                row_for(v, id).status,
                RowStatus::Queued,
                "row {id} is untouched"
            );
        }
        assert_eq!(
            ready_id(next_ready(v, Utc::now(), None).unwrap()),
            this_boot_manual
        );
        assert!(
            recover_at_startup(v, boot_at, Utc::now(), &HashSet::new())
                .unwrap()
                .is_empty(),
            "a second recovery finds nothing left to interrupt"
        );
    }

    #[test]
    fn coalescing_merges_reasons_counts_and_trigger_facts_while_queued() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let changed = now - Duration::minutes(20);
        let first = NewRow::new(
            RunSource::Trigger,
            Some("prep"),
            "Prep",
            "file changed: notes/a.md",
        )
        .with_payload(trigger_payload(&[FiredToken {
            token: "file-changed:notes/*.md".into(),
            max_mtime: Some(changed),
            ..FiredToken::default()
        }]))
        .with_key("trigger:prep:aaaa");
        let second = NewRow::new(RunSource::Trigger, Some("prep"), "Prep", "task overdue: t1")
            .with_payload(trigger_payload(&[FiredToken {
                token: "task-overdue".into(),
                overdue_ids: vec!["t1".into()],
                ..FiredToken::default()
            }]))
            .with_key("trigger:prep:bbbb");

        let id = enqueue(v, first, now).unwrap();
        assert!(matches!(id, Enqueued::Inserted(_)));
        let merged = enqueue(v, second, now).unwrap();
        assert_eq!(merged, Enqueued::Coalesced(id.id()));
        let row = row_for(v, id.id());
        assert_eq!(row.reason, "file changed: notes/a.md; task overdue: t1");
        assert_eq!(row.coalesced_count, 1);
        let fired = fired_tokens(&row.payload);
        let tokens: Vec<&str> = fired.iter().map(|t| t.token.as_str()).collect();
        assert_eq!(tokens, vec!["file-changed:notes/*.md", "task-overdue"]);
        assert_eq!(fired[0].max_mtime, Some(changed));
        assert_eq!(fired[1].overdue_ids, vec!["t1".to_string()]);
        assert_eq!(
            row.idempotency_key.as_deref(),
            Some("trigger:prep:aaaa"),
            "a trigger row keeps the key it was inserted under"
        );

        // Once claimed, a new fire is a new row.
        claim(v, id.id(), now).unwrap().expect("claimed");
        let third = NewRow::new(RunSource::Trigger, Some("prep"), "Prep", "task overdue: t2")
            .with_key("trigger:prep:cccc");
        assert!(
            matches!(enqueue(v, third, now).unwrap(), Enqueued::Inserted(other) if other != id.id())
        );

        // A repeated reason is not repeated.
        assert_eq!(merge_reasons("a; b", "b; c"), "a; b; c");
        assert_eq!(merge_reasons("", "x"), "x");
        assert!(
            merge_reasons(&"y".repeat(400), &"z".repeat(400))
                .chars()
                .count()
                <= REASON_MAX_CHARS
        );

        // Chat rows never coalesce.
        let a = enqueue(v, chat_row("s-20260925-aaaa0002", "one"), now).unwrap();
        let b = enqueue(v, chat_row("s-20260925-aaaa0002", "two"), now).unwrap();
        assert!(matches!(a, Enqueued::Inserted(_)) && matches!(b, Enqueued::Inserted(_)));
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn an_idempotency_key_is_unique_only_while_the_row_is_live() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let first = enqueue(v, cron("brief", "2026-09-25T13:00:00+00:00"), now).unwrap();
        assert!(matches!(first, Enqueued::Inserted(_)));
        assert_eq!(
            enqueue(v, cron("brief", "2026-09-25T13:00:00+00:00"), now).unwrap(),
            Enqueued::Duplicate(first.id())
        );

        // A newer occurrence coalesces: the row moves to its dueAt and key.
        assert_eq!(
            enqueue(v, cron("brief", "2026-09-26T13:00:00+00:00"), now).unwrap(),
            Enqueued::Coalesced(first.id())
        );
        let row = row_for(v, first.id());
        assert_eq!(
            due_at(&row.payload).as_deref(),
            Some("2026-09-26T13:00:00+00:00")
        );
        assert_eq!(
            row.idempotency_key.as_deref(),
            Some("cron:brief:2026-09-26T13:00:00+00:00")
        );
        assert_eq!(
            enqueue(v, cron("brief", "2026-09-26T13:00:00+00:00"), now).unwrap(),
            Enqueued::Duplicate(first.id()),
            "the moved key is live"
        );

        // After the row ends the same key inserts again.
        claim(v, first.id(), now).unwrap();
        mark_running(v, first.id(), "run-1", now).unwrap();
        assert!(finish(v, first.id(), RowStatus::Done, None, now).unwrap());
        assert!(
            !finish(v, first.id(), RowStatus::Failed, None, now).unwrap(),
            "finish is idempotent"
        );
        let again = enqueue(v, cron("brief", "2026-09-26T13:00:00+00:00"), now).unwrap();
        assert!(matches!(again, Enqueued::Inserted(id) if id != first.id()));
    }

    #[test]
    fn claims_follow_priority_then_fifo_and_honor_not_before_and_the_gap() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let heartbeat = enqueue(
            v,
            NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "due").with_key("hb:x:1"),
            now,
        )
        .unwrap()
        .id();
        let cron_a = enqueue(v, cron("a", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let cron_b = enqueue(v, cron("b", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let trigger = enqueue(
            v,
            NewRow::new(RunSource::Trigger, Some("t"), "T", "fired"),
            now,
        )
        .unwrap()
        .id();
        let manual = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("m"), "M", "run now"),
            now,
        )
        .unwrap()
        .id();
        let chat = enqueue(v, chat_row("s-20260925-aaaa0003", "hi"), now)
            .unwrap()
            .id();
        let mut later = NewRow::new(RunSource::Manual, Some("later"), "Later", "retry soon");
        later.not_before = Some(now + Duration::minutes(5));
        let delayed = enqueue(v, later, now).unwrap().id();

        let mut order = Vec::new();
        loop {
            match next_ready(v, now, None).unwrap() {
                NextRow::Ready(row) => {
                    order.push(row.id);
                    claim(v, row.id, now).unwrap().expect("claim the head");
                }
                NextRow::NotBefore(at) => {
                    assert_eq!(
                        at,
                        parse_stamp(&db_time(now + Duration::minutes(5))).unwrap()
                    );
                    break;
                }
                NextRow::Empty => panic!("the delayed row is still queued"),
            }
        }
        assert_eq!(
            order,
            vec![chat, manual, trigger, cron_a, cron_b, heartbeat]
        );
        assert_eq!(
            row_for(v, delayed).status,
            RowStatus::Queued,
            "a future not_before is skipped"
        );
        assert_eq!(
            ready_id(next_ready(v, now + Duration::minutes(6), None).unwrap()),
            delayed
        );

        // With the gap closed, automation rows wait while chat and manual rows do not.
        let gap_dir = TempDir::new().unwrap();
        let g = gap_dir.path();
        let gap = now + Duration::seconds(30);
        let c = enqueue(g, cron("a", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        assert_eq!(
            next_ready(g, now, Some(gap)).unwrap(),
            NextRow::NotBefore(gap)
        );
        assert_eq!(
            ready_id(next_ready(g, now, Some(now - Duration::seconds(1))).unwrap()),
            c,
            "an open gap is ignored"
        );
        let m = enqueue(
            g,
            NewRow::new(RunSource::Manual, Some("m"), "M", "run now"),
            now,
        )
        .unwrap()
        .id();
        assert_eq!(ready_id(next_ready(g, now, Some(gap)).unwrap()), m);
        assert!(RunSource::Manual.gap_exempt() && RunSource::Chat.gap_exempt());
        assert!(!RunSource::Cron.gap_exempt() && !RunSource::Trigger.gap_exempt());
        assert_eq!(
            next_ready(TempDir::new().unwrap().path(), now, None).unwrap(),
            NextRow::Empty
        );
    }

    /// SE0: a hold's kind and first sight are recorded once, a changed kind
    /// keeps the clock, and `mark_running` writes `holdMs` beside `holdKind`.
    #[test]
    fn record_hold_writes_kind_and_duration() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let t0 = Utc::now() - Duration::seconds(5);
        let id = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("a"), "A", "run now"),
            t0,
        )
        .unwrap()
        .id();
        record_hold(v, id, "appLimit", t0).unwrap();
        record_hold(v, id, "appLimit", t0 + Duration::seconds(2)).unwrap();
        record_hold(v, id, "folder", t0 + Duration::seconds(3)).unwrap();
        let timings = row_for(v, id).phase_timings.expect("timings");
        assert_eq!(timings["holdKind"], "folder");
        let since = timings["holdSince"].as_str().unwrap().to_string();
        record_hold(v, id, "folder", t0 + Duration::seconds(4)).unwrap();
        assert_eq!(
            row_for(v, id).phase_timings.unwrap()["holdSince"].as_str(),
            Some(since.as_str()),
            "the clock starts at the first sight"
        );
        claim(v, id, t0).unwrap().expect("claimed");
        assert!(mark_running(v, id, "run-h", t0 + Duration::seconds(4)).unwrap());
        let done = row_for(v, id).phase_timings.expect("timings");
        assert_eq!(done["holdKind"], "folder");
        let held = done["holdMs"].as_i64().unwrap();
        assert!((3_500..=4_500).contains(&held), "holdMs {held}");
        assert!(
            done.get("holdSince").is_none(),
            "the clock ends at the start"
        );
    }

    /// Paperclip PR 15c: a run's timings are merged into the running row
    /// that ran it, keeping the row's own `queueWaitMs`; nothing matches a
    /// run no row ran, or a row that already ended.
    #[test]
    fn phase_timings_merge_by_history_run_and_keep_queue_wait() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let requested = Utc::now() - Duration::seconds(4);
        let id = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("a"), "A", "run now"),
            requested,
        )
        .unwrap()
        .id();
        assert_eq!(
            queue_wait_for_run(v, "run-t").unwrap(),
            None,
            "not running yet"
        );
        claim(v, id, requested).unwrap().expect("claimed");
        assert!(mark_running(v, id, "run-t", Utc::now()).unwrap());
        let wait = queue_wait_for_run(v, "run-t")
            .unwrap()
            .expect("the wait is read back");
        assert!(wait >= 3_500, "request → running: {wait}");
        assert_eq!(queue_wait_for_run(v, "run-unknown").unwrap(), None);
        assert_eq!(queue_wait_for_run(v, "").unwrap(), None);

        let mut timings = Map::new();
        timings.insert("queueWaitMs".into(), Value::from(1));
        timings.insert("turnMs".into(), Value::from(2_500));
        assert!(merge_phase_timings(v, "run-t", &timings).unwrap());
        let merged = row_for(v, id).phase_timings.expect("timings");
        assert_eq!(
            merged["queueWaitMs"].as_u64(),
            Some(wait),
            "the row's own wait is kept"
        );
        assert_eq!(merged["turnMs"].as_u64(), Some(2_500));

        // A second merge adds keys and keeps the earlier ones.
        let mut more = Map::new();
        more.insert("evidenceMs".into(), Value::from(3));
        assert!(merge_phase_timings(v, "run-t", &more).unwrap());
        let merged = row_for(v, id).phase_timings.expect("timings");
        assert_eq!(merged["turnMs"].as_u64(), Some(2_500));
        assert_eq!(merged["evidenceMs"].as_u64(), Some(3));

        assert!(
            !merge_phase_timings(v, "run-unknown", &timings).unwrap(),
            "an unknown run"
        );
        assert!(finish(v, id, RowStatus::Done, None, Utc::now()).unwrap());
        assert!(
            !merge_phase_timings(v, "run-t", &timings).unwrap(),
            "after the terminal update"
        );
        assert_eq!(
            queue_wait_for_run(v, "run-t").unwrap(),
            Some(wait),
            "still readable once ended"
        );
    }

    #[test]
    fn lease_contention_requeues_at_the_same_position_without_an_attempt() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let first = enqueue(v, cron("a", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let second = enqueue(v, cron("b", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();

        claim(v, first, now).unwrap().expect("claimed");
        assert!(
            requeue_waiting(v, first, &chat_holder(), now).unwrap(),
            "the episode starts"
        );
        let row = row_for(v, first);
        assert_eq!(row.status, RowStatus::Queued);
        assert_eq!(row.attempt, 0, "losing the race is not an attempt");
        let wait = row.wait.expect("an episode");
        assert_eq!((wait.waits, wait.holder.as_str()), (1, "a chat turn"));
        assert_eq!(
            ready_id(next_ready(v, now, None).unwrap()),
            first,
            "same id, same FIFO place"
        );

        claim(v, first, now).unwrap().expect("claimed again");
        let later = now + Duration::seconds(5);
        assert!(!requeue_waiting(v, first, &workflow_holder("other"), later).unwrap());
        let wait = row_for(v, first).wait.unwrap();
        assert_eq!(wait.waits, 2);
        assert_eq!(wait.holder, "workflow `other`");
        assert_eq!(
            wait.first_waited_at,
            db_time(now),
            "the episode keeps its start"
        );
        assert!(
            !requeue_waiting(v, second, &chat_holder(), now).unwrap(),
            "only a claimed row requeues"
        );

        claim(v, first, later).unwrap().expect("claimed");
        let running_at = later + Duration::milliseconds(1_500);
        assert!(mark_running(v, first, "run-a", running_at).unwrap());
        let row = row_for(v, first);
        assert_eq!(row.status, RowStatus::Running);
        assert_eq!(row.attempt, 1);
        assert_eq!(row.history_run_id.as_deref(), Some("run-a"));
        let waited = row.phase_timings.unwrap()["queueWaitMs"].as_i64().unwrap();
        assert!(
            waited >= 6_500,
            "queueWaitMs covers requested → running: {waited}"
        );
        assert!(
            !mark_running(v, first, "run-a", running_at).unwrap(),
            "only a claimed row starts"
        );

        // A deferral puts a claimed row back with a hold and no episode.
        claim(v, second, now).unwrap().expect("claimed");
        requeue_waiting(v, second, &chat_holder(), now).unwrap();
        claim(v, second, now).unwrap().expect("claimed");
        let not_before = now + Duration::seconds(60);
        assert!(defer(v, second, not_before, Some("held for review"), true, None).unwrap());
        let row = row_for(v, second);
        assert_eq!(row.status, RowStatus::Queued);
        assert_eq!(row.held_reason.as_deref(), Some("held for review"));
        assert!(row.wait.is_none(), "a held row is paused, not waiting");
        assert_eq!(
            row.not_before.as_deref(),
            Some(db_time(not_before).as_str())
        );
    }

    #[test]
    fn waiting_episodes_skip_self_scheduler_and_held_rows_and_count_once_per_tick() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let a = enqueue(v, cron("a", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let b = enqueue(v, cron("b", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let held = enqueue(v, cron("held", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        claim(v, held, now).unwrap();
        defer(v, held, now, Some("paused"), true, None).unwrap();
        let heartbeat = enqueue(
            v,
            NewRow::new(RunSource::Heartbeat, None, "Heartbeat", "due").with_key("hb:x:1"),
            now,
        )
        .unwrap()
        .id();

        assert!(start_waiting_episodes(v, &LeaseHolder::Scheduler, now)
            .unwrap()
            .is_empty());
        assert_eq!(touch_waiting(v, &LeaseHolder::Scheduler, now).unwrap(), 0);

        // Workflow `a`'s own run starts episodes for everyone but `a` and the held row.
        let started = start_waiting_episodes(v, &workflow_holder("a"), now).unwrap();
        assert_eq!(started, vec![b, heartbeat]);
        assert!(
            row_for(v, a).wait.is_none(),
            "a workflow never waits on its own run"
        );
        assert!(row_for(v, held).wait.is_none(), "a held row never waits");
        assert_eq!(
            row_for(v, b).wait.unwrap().waits,
            0,
            "starting an episode counts no wait"
        );
        assert!(
            start_waiting_episodes(v, &chat_holder(), now).unwrap() == vec![a],
            "only rows without an episode start one"
        );
        assert_eq!(
            row_for(v, b).wait.unwrap().holder,
            "workflow `a`",
            "start never renames"
        );

        // The heartbeat's own run never counts against the heartbeat row.
        let heartbeat_run = LeaseHolder::Run {
            id: "run-hb".into(),
            source: Some("heartbeat".into()),
            source_id: None,
            started_at: None,
        };
        let later = now + Duration::minutes(1);
        assert_eq!(
            touch_waiting(v, &heartbeat_run, later).unwrap(),
            2,
            "a and b, not the heartbeat"
        );
        assert_eq!(row_for(v, heartbeat).wait.unwrap().waits, 0);
        assert_eq!(touch_waiting(v, &chat_holder(), later).unwrap(), 3);
        let b_wait = row_for(v, b).wait.unwrap();
        assert_eq!(b_wait.waits, 2, "one wait per touch");
        assert_eq!(
            b_wait.holder, "a chat turn",
            "the touch names the current holder"
        );
        assert_eq!(b_wait.last_waited_at, db_time(later));
        assert!(row_for(v, held).wait.is_none());
    }

    #[test]
    fn cancelling_rows_by_status() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let queued_cron = enqueue(v, cron("a", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let queued_trigger = enqueue(
            v,
            NewRow::new(RunSource::Trigger, Some("t"), "T", "fired"),
            now,
        )
        .unwrap()
        .id();
        let (manual, rx) = enqueue_and_wait(
            v,
            NewRow::new(RunSource::Manual, Some("m"), "M", "run now"),
            now,
        )
        .unwrap();
        let claimed = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("c"), "C", "run now"),
            now,
        )
        .unwrap()
        .id();
        claim(v, claimed, now).unwrap();
        let running = enqueue(
            v,
            NewRow::new(RunSource::Trigger, Some("r"), "R", "fired"),
            now,
        )
        .unwrap()
        .id();
        claim(v, running, now).unwrap();
        mark_running(v, running, "run-r", now).unwrap();

        assert_eq!(
            cancel(v, queued_cron, now).unwrap(),
            CancelOutcome::SkipRequested
        );
        let row = row_for(v, queued_cron);
        assert_eq!(
            row.status,
            RowStatus::Queued,
            "the claim bookkeeps the skipped occurrence"
        );
        assert!(row.cancel_requested && cancel_requested(v, queued_cron).unwrap());

        assert_eq!(
            cancel(v, queued_trigger, now).unwrap(),
            CancelOutcome::Cancelled
        );
        assert_eq!(row_for(v, queued_trigger).status, RowStatus::Cancelled);
        assert_eq!(
            cancel(v, manual.id(), now).unwrap(),
            CancelOutcome::Cancelled
        );
        match rx.blocking_recv() {
            Ok(RowOutcome::Finished {
                status: RowStatus::Cancelled,
                error,
            }) => {
                assert_eq!(error.as_deref(), Some(CANCELLED_BEFORE_START))
            }
            other => panic!("the waiter hears the cancel: {other:?}"),
        }

        assert_eq!(
            cancel(v, claimed, now).unwrap(),
            CancelOutcome::StopRequested
        );
        assert!(cancel_requested(v, claimed).unwrap());
        assert_eq!(row_for(v, claimed).status, RowStatus::Claimed);
        assert_eq!(
            cancel(v, running, now).unwrap(),
            CancelOutcome::Running {
                source: RunSource::Trigger,
                source_id: Some("r".into())
            }
        );
        assert_eq!(
            cancel(v, queued_trigger, now).unwrap(),
            CancelOutcome::AlreadyFinished
        );
        assert!(cancel(v, 9_999, now).is_err());
        assert!(!cancel_requested(v, 9_999).unwrap());
    }

    #[test]
    fn a_waiter_registered_with_its_enqueue_cannot_miss_a_fast_outcome() {
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        let now = Utc::now();
        let (first, first_rx) = enqueue_and_wait(
            &v,
            NewRow::new(RunSource::Manual, Some("a"), "A", "run now"),
            now,
        )
        .unwrap();
        let (second, second_rx) = enqueue_and_wait(
            &v,
            NewRow::new(RunSource::Manual, Some("a"), "A", "again"),
            now,
        )
        .unwrap();
        assert_eq!(
            second,
            Enqueued::Coalesced(first.id()),
            "two Run nows share one row"
        );
        assert!(has_waiters(&v, first.id()));

        // Another thread ends the row and resolves it before anyone awaits.
        let vault = v.clone();
        let id = first.id();
        std::thread::spawn(move || {
            finish(&vault, id, RowStatus::Done, None, Utc::now()).unwrap();
            resolve_waiters(
                &vault,
                id,
                RowOutcome::Finished {
                    status: RowStatus::Done,
                    error: None,
                },
            )
        })
        .join()
        .map(|delivered| assert_eq!(delivered, 2))
        .unwrap();
        for rx in [first_rx, second_rx] {
            assert!(matches!(
                rx.blocking_recv(),
                Ok(RowOutcome::Finished {
                    status: RowStatus::Done,
                    ..
                })
            ));
        }
        assert!(!has_waiters(&v, first.id()));
        assert_eq!(resolve_waiters(&v, first.id(), cancelled_outcome()), 0);
    }

    #[test]
    fn waiters_are_scoped_by_vault() {
        let a_dir = TempDir::new().unwrap();
        let b_dir = TempDir::new().unwrap();
        let now = Utc::now();
        let (a, a_rx) = enqueue_and_wait(
            a_dir.path(),
            NewRow::new(RunSource::Manual, Some("x"), "X", "run now"),
            now,
        )
        .unwrap();
        let (b, mut b_rx) = enqueue_and_wait(
            b_dir.path(),
            NewRow::new(RunSource::Manual, Some("x"), "X", "run now"),
            now,
        )
        .unwrap();
        assert_eq!(a.id(), b.id(), "both databases start at id 1");
        assert_eq!(
            resolve_waiters(
                a_dir.path(),
                a.id(),
                RowOutcome::Finished {
                    status: RowStatus::Done,
                    error: None
                }
            ),
            1
        );
        assert!(a_rx.blocking_recv().is_ok());
        assert!(
            b_rx.try_recv().is_err(),
            "vault B's waiter is still pending"
        );
        assert!(has_waiters(b_dir.path(), b.id()));
    }

    #[test]
    fn a_live_chat_row_counts_even_behind_a_newer_cancelled_one() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let key = "s-20261112-live0001";
        assert!(!has_live_chat_row(v, key).unwrap());
        let older = enqueue(v, chat_row(key, "one"), now).unwrap().id();
        assert!(has_live_chat_row(v, key).unwrap(), "queued");
        claim(v, older, now).unwrap();
        mark_running(v, older, "run-live-1", now).unwrap();
        commit_turn(v, key, "run-live-1", "one");
        let newer = enqueue(v, chat_row(key, "two"), now).unwrap().id();
        cancel(v, newer, now).unwrap();
        assert!(!newest_chat_row(v, key).unwrap().unwrap().status.is_live());
        assert!(
            has_live_chat_row(v, key).unwrap(),
            "the older turn still runs"
        );
        assert!(!has_live_chat_row(v, "s-20261112-live0002").unwrap());
        finish(v, older, RowStatus::Done, None, now).unwrap();
        assert!(!has_live_chat_row(v, key).unwrap());
    }

    #[test]
    fn every_terminal_transition_clears_a_chat_payload_and_retention_prunes_only_terminal_rows() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let finished = enqueue(v, chat_row("s-20260925-bbbb0001", "one"), now)
            .unwrap()
            .id();
        claim(v, finished, now).unwrap();
        mark_running(v, finished, "run-1", now).unwrap();
        commit_turn(v, "s-20260925-bbbb0001", "run-1", "one");
        finish(v, finished, RowStatus::Done, None, now).unwrap();
        let cancelled = enqueue(v, chat_row("s-20260925-bbbb0002", "two"), now)
            .unwrap()
            .id();
        cancel(v, cancelled, now).unwrap();
        let prior = enqueue(
            v,
            chat_row("s-20260925-bbbb0003", "three"),
            now - Duration::hours(1),
        )
        .unwrap()
        .id();
        let running = enqueue(
            v,
            chat_row("s-20260925-bbbb0004", "four"),
            now - Duration::hours(1),
        )
        .unwrap()
        .id();
        claim(v, running, now).unwrap();
        mark_running(v, running, "run-4", now).unwrap();
        commit_turn(v, "s-20260925-bbbb0004", "run-4", "four");
        recover_at_startup(v, now - Duration::minutes(1), now, &HashSet::new()).unwrap();

        for id in [finished, cancelled, prior, running] {
            assert_eq!(raw(v, id).0, "{}", "row {id}'s composed prompt is gone");
        }
        assert_eq!(raw(v, finished).1, None);
        assert_eq!(raw(v, cancelled).1, None);
        assert_eq!(
            raw(v, running).1,
            None,
            "a turn that had started keeps no text"
        );
        assert_eq!(
            raw(v, prior).1.as_deref(),
            Some("three"),
            "only a never-started turn keeps it"
        );

        // Absorbed rows end through the same helper.
        let admitted = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("w"), "W", "run now"),
            now,
        )
        .unwrap()
        .id();
        let sibling_cron = enqueue(v, cron("w", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let sibling_trigger = enqueue(
            v,
            NewRow::new(RunSource::Trigger, Some("w"), "W", "file changed: x"),
            now,
        )
        .unwrap()
        .id();
        let absorbed = absorb_siblings(v, admitted, "w", now).unwrap();
        assert_eq!(
            absorbed.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec![sibling_cron, sibling_trigger]
        );
        assert_eq!(absorbed[1].source, RunSource::Trigger);
        assert_eq!(absorbed[1].reason, "file changed: x");
        let admitted_row = row_for(v, admitted);
        assert_eq!(admitted_row.reason, "run now; scheduled; file changed: x");
        assert_eq!(admitted_row.coalesced_count, 2);
        for id in [sibling_cron, sibling_trigger] {
            let row = row_for(v, id);
            assert_eq!(row.status, RowStatus::Coalesced);
            assert_eq!(row.coalesced_into, Some(admitted));
        }

        // Retention: age, count, and a kept detail's own age. Live rows stay.
        let old_live = enqueue(
            v,
            cron("keep", "2026-01-01T00:00:00+00:00"),
            now - Duration::days(90),
        )
        .unwrap()
        .id();
        with_write(v, "test", |tx| {
            for _ in 0..(KEEP_TERMINAL_ROWS + 10) {
                tx.execute(
                    "INSERT INTO run_queue(source, label, status, priority, requested_at, finished_at) \
                     VALUES('cron', 'bulk', 'done', 3, ?1, ?1)",
                    params![db_time(now)],
                )
                .map_err(|e| e.to_string())?;
            }
            // Newer ids than the bulk rows, so only their age decides them.
            let old = db_time(now - Duration::days(40));
            let week_old = db_time(now - Duration::days(8));
            tx.execute(
                "INSERT INTO run_queue(source, label, status, priority, requested_at, finished_at) \
                 VALUES('cron', 'ancient', 'done', 3, ?1, ?1)",
                params![old],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO run_queue(source, source_id, label, status, priority, requested_at, \
                 finished_at, detail) VALUES('chat', 's-x', 'kept', 'interrupted', 0, ?1, ?1, 'old text')",
                params![week_old],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
        let pruner = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("p"), "P", "run now"),
            now,
        )
        .unwrap()
        .id();
        finish(v, pruner, RowStatus::Done, None, now).unwrap();
        let (terminal, ancient, kept, kept_detail): (i64, i64, i64, i64) = with_read(v, |conn| {
            let count = |sql: &str| {
                conn.query_row(sql, [], |r| r.get::<_, i64>(0))
                    .map_err(|e| e.to_string())
            };
            Ok((
                count(&format!(
                    "SELECT COUNT(*) FROM run_queue WHERE status IN {TERMINAL}"
                ))?,
                count("SELECT COUNT(*) FROM run_queue WHERE label = 'ancient'")?,
                count("SELECT COUNT(*) FROM run_queue WHERE label = 'kept'")?,
                count(
                    "SELECT COUNT(*) FROM run_queue WHERE label = 'kept' AND detail IS NOT NULL",
                )?,
            ))
        })
        .unwrap();
        assert_eq!(
            terminal, KEEP_TERMINAL_ROWS,
            "only the newest terminal rows are kept"
        );
        assert_eq!(ancient, 0, "terminal rows past 30 days are pruned");
        assert_eq!(
            kept, 1,
            "an 8-day-old terminal row is inside the 30-day window"
        );
        assert_eq!(kept_detail, 0, "but its kept asked text goes after 7 days");
        assert_eq!(
            row_for(v, old_live).status,
            RowStatus::Queued,
            "a live row is never pruned"
        );
        assert_eq!(row_for(v, pruner).status, RowStatus::Done);
    }

    #[test]
    fn the_legacy_deferral_file_is_removed_at_startup() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("notes")).unwrap();
        std::fs::write(v.join("notes/keep.md"), "unchanged").unwrap();
        std::fs::write(
            v.join(LEGACY_DEFERRALS_FILE),
            r#"{"version":1,"episodes":{"a":{"firstDeferredAt":"x","lastDeferredAt":"x","attempts":1,"holder":"a chat turn"},"b":{"firstDeferredAt":"x","lastDeferredAt":"x","attempts":2,"holder":"Notion sync"}}}"#,
        )
        .unwrap();
        assert_eq!(retire_legacy_deferrals(v).unwrap(), 2);
        assert!(!v.join(LEGACY_DEFERRALS_FILE).exists());
        assert_eq!(
            std::fs::read_to_string(v.join("notes/keep.md")).unwrap(),
            "unchanged"
        );
        assert_eq!(retire_legacy_deferrals(v).unwrap(), 0, "missing is fine");

        std::fs::write(v.join(LEGACY_DEFERRALS_FILE), "not json").unwrap();
        assert_eq!(
            retire_legacy_deferrals(v).unwrap(),
            0,
            "an unreadable file is still removed"
        );
        assert!(!v.join(LEGACY_DEFERRALS_FILE).exists());
        let entries: Vec<String> = std::fs::read_dir(v)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            entries,
            vec!["notes".to_string()],
            "no other vault file changes"
        );

        // The pump's recovery can overlap an IPC-started chat turn: the file
        // is no longer watched dot-state, so removing it flags nothing.
        std::fs::write(v.join(LEGACY_DEFERRALS_FILE), "{}").unwrap();
        let chat = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
        assert!(crate::run_guard::is_tracking(v));
        assert_eq!(retire_legacy_deferrals(v).unwrap(), 0);
        let run = crate::automated_history::finish_run(v, chat, "success").unwrap();
        assert!(run.flags.is_empty(), "{:?}", run.flags);
    }

    fn episode(first: DateTime<Utc>, last: DateTime<Utc>, waits: u32) -> WaitEpisode {
        WaitEpisode {
            due_at: None,
            trigger: false,
            first_waited_at: first.to_rfc3339(),
            last_waited_at: last.to_rfc3339(),
            waits,
            holder: "a chat turn".into(),
        }
    }

    #[test]
    fn wait_episode_clocks_decide_ran_since_and_abandoned() {
        let dir = TempDir::new().unwrap();
        let now = Utc::now();
        let first = now - Duration::minutes(30);
        let ep = episode(first, now - Duration::minutes(ABANDON_AFTER_MIN), 2);

        assert!(!ep.abandoned(now), "exactly the window is not abandoned");
        assert!(ep.abandoned(now + Duration::seconds(1)));
        assert!(WaitEpisode {
            last_waited_at: "garbage".into(),
            ..ep.clone()
        }
        .abandoned(now));
        assert_eq!(ep.waited_minutes(now), 30);
        assert_eq!(ep.waited_minutes(first - Duration::minutes(5)), 0);

        let ran_before = seed_workflow_with(dir.path(), "Before", "manual", "BODY", |fm| {
            fm.last_run_at = Some((first - Duration::minutes(1)).to_rfc3339());
        });
        let ran_after = seed_workflow_with(dir.path(), "After", "manual", "BODY", |fm| {
            fm.last_run_at = Some((first + Duration::seconds(30)).to_rfc3339());
        });
        let never_ran = seed_workflow_with(dir.path(), "Never", "manual", "BODY", |_| {});
        assert!(!ep.ran_since(&ran_before));
        assert!(ep.ran_since(&ran_after));
        assert!(!ep.ran_since(&never_ran));

        // The stored form (db_time) reads back the same way.
        let stored = WaitEpisode {
            first_waited_at: db_time(first),
            last_waited_at: db_time(now),
            ..ep.clone()
        };
        assert_eq!(stored.waited_minutes(now), 30);
        assert!(!stored.abandoned(now));
    }

    #[test]
    fn every_wait_ending_names_the_holder_and_only_abandonment_says_closed() {
        let now = Utc::now();
        let ep = episode(now - Duration::minutes(12), now - Duration::minutes(1), 3);

        let started = ep.started_entry(now);
        assert_eq!(started.status, vault_workflows::DEFERRED_STATUS);
        assert_eq!(
            started.note,
            "started after waiting 12 min for a chat turn (3 deferrals)"
        );
        let quiet = episode(now - Duration::hours(5), now - Duration::hours(4), 2);
        assert_eq!(
            quiet.started_entry(now).note,
            "started after waiting 60 min for a chat turn (2 deferrals)"
        );
        let busy = ep.busy_skip_entry(now, 120);
        assert_eq!(busy.status, vault_workflows::SKIPPED_STATUS);
        assert!(busy.note.contains("a chat turn") && busy.note.contains("120-minute grace"));
        let cleared = ep.cleared_entry(now);
        assert!(cleared
            .note
            .contains("no longer due after waiting for a chat turn"));
        let abandoned = ep.abandoned_entry(now);
        assert!(abandoned.note.starts_with(MISSED_WHILE_CLOSED_NOTE));
        assert!(abandoned.note.contains("a chat turn"));

        for entry in [&started, &busy, &cleared, &abandoned] {
            assert_ne!(entry.note, MISSED_WHILE_CLOSED_NOTE);
            assert_eq!(entry.run_id, None);
            assert_eq!(entry.at, now.to_rfc3339());
        }
        for entry in [&started, &busy, &cleared] {
            assert!(!entry.note.contains(MISSED_WHILE_CLOSED_NOTE));
        }
    }

    /// `src/runQueue.ts` mirrors these strings to word a paused or stale row.
    #[test]
    fn the_frontend_mirrors_the_hold_and_stale_holder_strings() {
        assert_eq!(
            LeaseHolder::Stale { id: "run-x".into() }.describe(),
            "a stale lease left by an earlier run"
        );
        assert_eq!(
            vault_workflows::PERMISSION_REFUSED_REASON,
            "automation settings changed; review them in Automations → Permission changes"
        );
        assert_eq!(
            vault_workflows::PERMISSION_RECORDS_UNAVAILABLE_REASON,
            "automation permission records could not be read; repair or move them, then restart Hollow"
        );
    }

    #[test]
    fn the_ui_item_never_carries_the_payload_and_positions_follow_claim_order() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let c = enqueue(v, cron("a", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        let m = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("m"), "M", "run now"),
            now,
        )
        .unwrap()
        .id();
        let r = enqueue(
            v,
            NewRow::new(RunSource::Trigger, Some("r"), "R", "fired"),
            now,
        )
        .unwrap()
        .id();
        claim(v, r, now).unwrap();
        mark_running(v, r, "run-r", now).unwrap();
        let done = enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("d"), "D", "run now"),
            now,
        )
        .unwrap()
        .id();
        cancel(v, done, now).unwrap();

        let rows = list(v, 20).unwrap();
        let summary: Vec<(i64, Option<u32>)> =
            rows.iter().map(|(row, pos)| (row.id, *pos)).collect();
        assert_eq!(
            summary,
            vec![(r, None), (m, Some(1)), (c, Some(2)), (done, None)]
        );
        assert!(list(v, 0)
            .unwrap()
            .iter()
            .all(|(row, _)| row.status.is_live()));

        let item = to_item(&rows[2].0, rows[2].1);
        let json = serde_json::to_value(&item).unwrap();
        assert!(json.get("payload").is_none() && json.get("payloadJson").is_none());
        assert_eq!(json["source"], "cron");
        assert_eq!(json["position"], 2);
        assert_eq!(json["sourceId"], "a");
        let wire = serde_json::to_value(event("enqueued", item)).unwrap();
        assert_eq!(wire["kind"], "enqueued");
        assert_eq!(wire["item"]["id"], c);
        assert_eq!(EVENT, "run-queue-event");
    }

    #[test]
    fn sources_and_statuses_round_trip_and_unknown_rows_are_skipped() {
        for source in [
            RunSource::Chat,
            RunSource::Cron,
            RunSource::Manual,
            RunSource::Trigger,
            RunSource::Retry,
            RunSource::Heartbeat,
            RunSource::Ingest,
        ] {
            assert_eq!(RunSource::parse(source.as_str()), Some(source));
        }
        assert_eq!(RowStatus::from_run_status("success"), RowStatus::Done);
        assert_eq!(RowStatus::from_run_status("error"), RowStatus::Failed);
        assert_eq!(
            RowStatus::from_run_status("cancelled"),
            RowStatus::Cancelled
        );
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        with_write(v, "test", |tx| {
            tx.execute(
                "INSERT INTO run_queue(source, label, status, priority, requested_at) \
                 VALUES('future-source', 'x', 'queued', 0, ?1)",
                params![db_time(now)],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap();
        let known = enqueue(v, cron("a", "2026-09-25T07:00:00+00:00"), now)
            .unwrap()
            .id();
        assert_eq!(
            ready_id(next_ready(v, now, None).unwrap()),
            known,
            "an unknown row never blocks the queue"
        );
        assert_eq!(list(v, 10).unwrap().len(), 1);
    }

    #[test]
    fn e1_card_payload_closes_cards_matches_a_person_send() {
        let person = serde_json::json!({ "prompt": "hello" });
        assert!(payload_closes_cards(&person));
        assert!(payload_closes_cards(&serde_json::json!({})));
        assert!(payload_closes_cards(&Value::Null));
        assert!(!payload_closes_cards(&serde_json::json!({
            "prompt": "wake",
            "hostAsker": "hollow:wake"
        })));
        assert!(!payload_closes_cards(
            &serde_json::json!({ "prompt": "pulse", "unattended": true })
        ));
        assert!(!payload_closes_cards(
            &serde_json::json!({ "prompt": "child", "appDepth": 1 })
        ));
        assert!(payload_closes_cards(
            &serde_json::json!({ "prompt": "typed", "hostAsker": "" })
        ));
    }

    /// Lane T's re-review of #1313: an automation's chat turn keeps its
    /// `auto-` `clientTurnId` when it ends, so a pending message whose send
    /// landed is still recognised; every other unattended row is unchanged.
    #[test]
    fn an_automation_turn_keeps_its_turn_id_when_it_ends() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let row = |key: &str, turn: &str| {
            NewRow::new(RunSource::Chat, Some(key), "Chat turn", "chat").with_payload(
                serde_json::json!({
                    "prompt": "<page-context>secret</page-context>",
                    "unattended": true,
                    "clientTurnId": turn,
                    "hostAsker": "hollow:automation"
                }),
            )
        };
        let auto = enqueue(v, row("s-20261112-autoid01", "auto-x"), now)
            .unwrap()
            .id();
        let other = enqueue(v, row("s-20261112-autoid02", "ct-person"), now)
            .unwrap()
            .id();
        assert!(finish(v, auto, RowStatus::Done, None, now).unwrap());
        assert!(finish(v, other, RowStatus::Done, None, now).unwrap());
        let kept: Value = serde_json::from_str(&raw(v, auto).0).unwrap();
        assert_eq!(
            kept,
            serde_json::json!({"unattended": true, "clientTurnId": "auto-x"})
        );
        assert_eq!(raw(v, other).0, r#"{"unattended":true}"#);
    }
}
