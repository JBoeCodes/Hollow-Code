//! The durable agent-session event log, OUTSIDE the vault, plus the usage
//! ledger (Paperclip PR 6).
//!
//! Sibling of the (Phase 3) vault-native Session Record, not its parent. This
//! store owns the app's provider-neutral conversation transcript and native
//! resume metadata. Raw trace events are a retention-limited diagnostic cache;
//! the bounded transcript is durable and must not be pruned with those events.
//!
//! The `usage_ledger` table is NOT derived and cannot be rebuilt: it is the
//! only record of what each agent invocation reported (one row per run,
//! written by `usage_ledger::UsageRun`). Its rows are pruned only after
//! [`KEEP_USAGE_DAYS`], and never while `running`.
//!
//! **More than one process writes this database.** `hollow team-mcp`
//! subprocesses record their delegations' usage rows here too. [`open`] sets a
//! busy timeout before anything else, and every read-then-write transaction
//! goes through [`with_write`] (`BEGIN IMMEDIATE`): in WAL mode a DEFERRED
//! transaction that reads and then writes fails with `SQLITE_BUSY_SNAPSHOT`
//! when another connection committed in between, and the busy handler is
//! never consulted for that. Single-statement writes are safe as autocommit.
//! Every compared TEXT timestamp in the usage table uses [`db_time`].
//!
//! It lives under the app data dir, keyed by canonical vault path, following
//! the shape already proven by `vault_search_index` and `vault_embed_index` —
//! including the `#[cfg(test)]` root swap. That swap is not decoration: 246
//! orphan databases once accumulated in the live app data dir because a test
//! wrote there, so `db_path_stays_out_of_the_real_app_data_dir` pins it.
//!
//! Bounded by construction. Assistant prose is stored as ONE capped event at
//! turn end rather than one row per streamed delta, because deltas arrive in
//! fragments and would otherwise cost thousands of rows per turn.
//!
//! The same database also holds the durable automation queue (`run_queue`,
//! Paperclip PR 7, with its completed cron occurrences,
//! `automation_occurrences`), owned by [`crate::run_queue`]. Unlike the trace, queue
//! rows are control state: [`with_write`] gives them IMMEDIATE transactions,
//! and `open` sets `busy_timeout` before anything else so a concurrent writer
//! is waited for rather than failing the call.
//!
//! It also holds the chat's structured question cards (`interactions`,
//! Paperclip PR 13), owned by [`crate::interactions`]: control state too, not
//! derived. Terminal cards are pruned after 90 days by [`retain`]; open ones
//! never are. [`archive_session`] expires a conversation's open cards in the
//! same transaction.
//!
//! And it holds saved turn feedback (`turn_feedback`, Paperclip PR 17), owned
//! by [`crate::turn_feedback`]: the user's Useful / Not useful votes with a
//! redacted snapshot of the turn. Primary, not derived: [`retain`] never
//! prunes it and [`archive_session`] leaves it alone; only
//! `turn_feedback::clear` (Settings, Response feedback, Clear feedback)
//! removes rows. Each transcript also keeps its turn's approved recall paths
//! (a `recall` event, [`record_recall`]) for those snapshots.
//!
//! Last, `derived_state` (agent effectiveness F4, wave 2 · K2): per
//! conversation, state DERIVED from the rows above and rebuilt from them
//! when missing or from an older schema: the task-state record
//! (`task_state.rs`) and the profile a native session was last given
//! (`context_compiler`'s revision delta). Never a source of truth: the
//! transcript, the receipts and History are.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
#[path = "session_search.rs"]
mod fulltext;
#[path = "session_organization.rs"]
pub mod organization;
pub use fulltext::SearchContext;
use once_cell::sync::Lazy;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use crate::agent_sessions::is_safe_session_key;
use crate::types::{
    AgentSessionDetail, AgentSessionSummary, HarnessStint, SessionRecordTurn, SessionStoreStatus,
};

/// Serializes this process's writers (one user, low volume: a single lock is
/// simpler than a connection pool and cannot deadlock against itself). It does
/// NOT cover other processes: `hollow team-mcp` writes usage rows from its own
/// process, so cross-process safety comes from the busy timeout in [`open`]
/// plus IMMEDIATE transactions ([`with_write`]). Never hold it while taking
/// the history lease, the permission write guard or the review queue lock; it
/// is a leaf. The run queue's `WAITERS` lock is the one lock taken before
/// it (`WAITERS` → `WRITE_LOCK`).
static WRITE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Take [`WRITE_LOCK`], recovering it if a writer panicked while holding it.
/// The guarded value is `()`, so there is no state a panic could leave half
/// written (each write is its own SQLite transaction), and refusing the lock
/// would stop every queue and usage write until restart (review finding A·R7).
fn write_lock() -> MutexGuard<'static, ()> {
    WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How long a connection waits for another process's write lock.
const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(2_000);

/// Usage rows are kept this long (a year of monthly history for PR 8's
/// budgets), then pruned unless still `running`.
pub const KEEP_USAGE_DAYS: i64 = 400;

/// The one timestamp format for every compared TEXT column in the usage table
/// (and PR 7's queue): fixed-width UTC with milliseconds and `Z`, so SQL range
/// filters can compare strings. Never mix `to_rfc3339()` (variable precision,
/// `+00:00`) into those columns.
pub fn db_time(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Per-event payload cap. Tool inputs and results can be large; the receipt
/// and the vault record are the durable summaries, this is the trace.
const EVENT_PAYLOAD_CHARS: usize = 8_000;
/// One assistant-prose event per turn, capped.
const ASSISTANT_CHARS: usize = 64_000;
const MAX_TRANSCRIPT_TOOLS: usize = 200;

/// Chat history belongs to Hollow. CLI-native ids are resume metadata, never
/// the identity or source of the transcript shown in the app.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTool {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub result: Option<String>,
    pub is_error: bool,
    pub status: String,
    /// AG7: the spawning call's id when a Claude subagent made this call
    /// (`parent_tool_use_id`); absent on the wire for a main-thread call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
    /// WD1: the full result's length in characters when the window's cold
    /// open sent only a preview ([`lean_for_window`]); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_chars: Option<u64>,
    /// WD1: a trimmed Read result's numbered lines (its row says "Read N
    /// lines"); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_lines: Option<u64>,
    /// WD1: the stored turn (its ord) a trimmed result is read back from
    /// ([`read_tool_result`]); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_turn: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    pub id: String,
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning: String,
    pub tools: Vec<ConversationTool>,
    pub agent: String,
    pub run_id: Option<String>,
    pub started_at: String,
    /// When the turn's run ended (assistant turns only, overnight 11-12 C8):
    /// the reply row's time when no receipt came back. `None` while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub status: Option<String>,
    pub cost_usd: Option<f64>,
    pub error: Option<String>,
    pub receipt: Option<crate::evidence::EvidenceReceipt>,
    /// Older trace data may already have expired before durable transcripts
    /// were introduced. Never imply the reconstructed history is complete.
    pub transcript_incomplete: bool,
    /// The `resultKind` of the turn's result envelope (Paperclip PR 14),
    /// assistant turns only. `None` for a turn with no result (stopped,
    /// failed to start) and for turns recorded before the field existed.
    #[serde(default)]
    pub result_kind: Option<String>,
    /// The turn was stopped before its agent was spawned (Paperclip PR 14b),
    /// so its native session never saw the request and Continue is not
    /// offered. Assistant turns only.
    #[serde(default)]
    pub stopped_before_spawn: bool,
    /// What the person said to the running turn (mid-turn steering,
    /// 2026-10-21), oldest first, each with when, where among the turn's
    /// tool calls (`afterTools`) and whether the agent took it. Assistant
    /// turns only; the turn is still one prompt and one answer. Absent when
    /// nobody steered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steers: Vec<crate::steering::SteerRecord>,
    /// The files (not images) a user turn was sent with, as the chat
    /// bubble's chips (B1a, ENG-104): from `turns.attachments`, filled by
    /// `phone_access::attachments::fill_conversation` in the
    /// `read_agent_conversation` command only. Absent on replies, on turns
    /// with no files and on turns from before attachments were recorded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<ConversationFile>,
}

/// A file a saved user turn carried: its readable name, the path of its
/// saved copy in the chat folder's `.hollow-attachments/`, and its size
/// when the turn started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFile {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// One entry of the bounded conversation bundle a fresh native session
/// receives (`context_compiler::conversation_bundle`): a
/// turn's user text or assistant text, with the agent that answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEntry {
    pub role: String,
    pub agent: String,
    pub text: String,
    /// The conversation the entry was read from and its turn number (what
    /// `read_session`'s `fromTurn` counts), so a bundle that leaves it out
    /// can point at it (PS1). Empty and 0 when unknown: no pointer then.
    pub key: String,
    pub turn: i64,
}

/// The open stint of a conversation, as the resume plan reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenStint {
    pub agent: String,
    pub harness_session_id: Option<String>,
    /// NULL (`None`) on a legacy stint: unknown, never "changed".
    pub config_fingerprint: Option<String>,
    /// The fingerprint's per-part hashes (OB03,
    /// `chat_resume::config_parts`); NULL on a stint stamped before them,
    /// whose reset names `unknown`.
    pub config_parts: Option<String>,
}

/// The most recently closed stint: why it closed, which native session it
/// held, and its fingerprint's parts (OB03: what a pending reset names).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClosedStint {
    pub harness_session_id: Option<String>,
    pub reason: Option<String>,
    pub config_parts: Option<String>,
}

/// What `chat_resume::plan_resume` decides a turn's resume from (Paperclip
/// PR 14b).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeState {
    pub open: Option<OpenStint>,
    pub last_closed: Option<ClosedStint>,
}

/// The turn before the running one, as the fingerprint version restamp
/// (OB04, `chat_resume::may_restamp`) compares it ([`previous_turn`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviousTurn {
    /// `turns.permission_mode`; `None` when that turn recorded none.
    pub permission_mode: Option<String>,
    /// Its chat usage row's requested model (`None`: the agent's default)
    /// and agent, as [`last_requested_model`] reads them. [`previous_turn`]
    /// reads only turns with such a row, so it always fills this; `None`
    /// fails the restamp check.
    pub requested: Option<(Option<String>, Option<String>)>,
    /// The conversation's `sessions.cwd`.
    pub session_cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConversation {
    pub summary: AgentSessionSummary,
    pub turns: Vec<ConversationTurn>,
    pub events_retained: bool,
    /// Latest recorded execution mode, independent of another chat's UI state.
    #[serde(default)]
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredTranscript {
    #[serde(default)]
    reasoning: String,
    agent: String,
    text: String,
    tools: Vec<ConversationTool>,
    cost_usd: Option<f64>,
    error: Option<String>,
    incomplete: bool,
    /// The result envelope's `resultKind` (Paperclip PR 14).
    result_kind: Option<String>,
    /// The approved learning paths recalled into this turn (Paperclip PR 17),
    /// at most [`MAX_RECALL_PATHS`]. Older transcripts decode to empty.
    recall: Vec<String>,
    /// A `cancelled` trace event with payload `before-spawn` (Paperclip PR 14b).
    stopped_before_spawn: bool,
    /// The person's mid-turn messages (`steer` events, their state moved by
    /// `steerState`; `crate::steering`), oldest first.
    steers: Vec<crate::steering::SteerRecord>,
}

/// Recall paths kept per turn (the recall itself selects at most five).
const MAX_RECALL_PATHS: usize = 10;
/// A recalled path longer than this is dropped rather than truncated.
const RECALL_PATH_CHARS: usize = 512;

/// Retention, mirroring `automated_history`: keep every session ROW forever,
/// but drop the raw events of anything past these bounds.
const KEEP_EVENT_SESSIONS: usize = 200;
const KEEP_EVENT_DAYS: i64 = 90;

/// Where the derived session databases live. Test builds get a per-process
/// temp dir so a test can never write into the user's app data dir.
#[cfg(not(test))]
fn store_root() -> Option<PathBuf> {
    Some(crate::instance::support_dir()?.join("Sessions"))
}

#[cfg(test)]
fn store_root() -> Option<PathBuf> {
    Some(crate::test_temp::process_dir("hollow-test-sessions-"))
}

/// `{vault folder name}-{sha256(canonical vault path)[..16]}`: the per-vault
/// file stem every out-of-vault store uses (this database, and the budgets
/// store's per-vault test swap). Non-alphanumeric characters in the folder
/// name become `-`. `workflow_permissions::store_path` still carries its own
/// byte-identical copy.
pub(crate) fn vault_file_stem(vault: &Path) -> String {
    let canon = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canon.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let name: String = canon
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "vault".to_string())
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    format!("{name}-{}", &hash[..16])
}

pub(crate) fn db_path_for(vault: &Path) -> Option<PathBuf> {
    Some(store_root()?.join(format!("{}.db", vault_file_stem(vault))))
}

fn open(vault: &Path) -> Result<Connection, String> {
    let path = db_path_for(vault).ok_or_else(|| "no session store path".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("session store dir: {e}"))?;
    }
    let conn = Connection::open(&path).map_err(|e| format!("session store open: {e}"))?;
    // First, before any pragma or DDL can meet another process's write lock.
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| format!("session store busy timeout: {e}"))?;
    let _ = conn.pragma_update(None, "foreign_keys", "ON");
    // The schema below (about 60 statements and 15 column checks) runs once
    // per database file per process (five asks B4): every store read used to
    // re-run all of it. The file's identity is part of the key, so a deleted
    // and recreated store gets its schema again.
    let identity = schema_identity(&path);
    if identity.as_ref().is_some_and(|id| {
        SCHEMA_READY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(id)
    }) {
        return Ok(conn);
    }
    // Serialize cold-boot WAL switching and schema DDL across concurrent
    // startup readers/writers so they cannot collide with SQLITE_BUSY.
    let _init = SCHEMA_INIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let identity = schema_identity(&path);
    if identity.as_ref().is_some_and(|id| {
        SCHEMA_READY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(id)
    }) {
        return Ok(conn);
    }
    // Remember the file only once WAL took: a first open that lost the race
    // for it (another connection) tries again next time, as every open did.
    let wal = conn
        .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .is_ok_and(|mode| mode.eq_ignore_ascii_case("wal"));
    apply_schema(&conn)?;
    if let Some(id) = identity.filter(|_| wal) {
        SCHEMA_READY
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id);
    }
    Ok(conn)
}

/// Serializes cold-boot `journal_mode = WAL` and [`apply_schema`] per process
/// so concurrent startup opens never race on ~60 DDL statements. Taken only on
/// a [`SCHEMA_READY`] miss, always after `WRITE_LOCK` (when held) and before
/// `SCHEMA_READY`.
static SCHEMA_INIT: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Database files whose schema this process already applied: (path, device,
/// inode).
static SCHEMA_READY: Lazy<Mutex<std::collections::HashSet<(PathBuf, u64, u64)>>> =
    Lazy::new(|| Mutex::new(std::collections::HashSet::new()));

fn schema_identity(path: &Path) -> Option<(PathBuf, u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some((path.to_path_buf(), meta.dev(), meta.ino()))
}

/// The runbook tables (`open` runs them). Runbooks were removed 11-10 (S6);
/// the tables stay inert so existing databases open the same way.
const RUNBOOK_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS runbook_runs( \
   id INTEGER PRIMARY KEY AUTOINCREMENT, \
   source TEXT NOT NULL, \
   name TEXT NOT NULL, \
   workspace TEXT NOT NULL, \
   day TEXT NOT NULL, \
   steps_hash TEXT NOT NULL, \
   status TEXT NOT NULL, \
   mode TEXT NOT NULL, \
   unattended INTEGER NOT NULL DEFAULT 0, \
   session_key TEXT, \
   copy_path TEXT NOT NULL, \
   started_at TEXT NOT NULL, \
   finished_at TEXT, \
   note TEXT \
 ); \
 CREATE UNIQUE INDEX IF NOT EXISTS idx_runbook_runs_version \
   ON runbook_runs(source, workspace, day, steps_hash); \
 CREATE TABLE IF NOT EXISTS runbook_steps( \
   run_id INTEGER NOT NULL, \
   seq INTEGER NOT NULL, \
   status TEXT NOT NULL, \
   evidence TEXT NOT NULL, \
   queue_id INTEGER, \
   confirmed_at TEXT, \
   ran INTEGER NOT NULL DEFAULT 0, \
   at TEXT NOT NULL, \
   PRIMARY KEY(run_id, seq) \
 ); \
 CREATE TABLE IF NOT EXISTS runbook_approvals( \
   source TEXT NOT NULL, \
   steps_hash TEXT NOT NULL, \
   workspace TEXT NOT NULL, \
   approved_at TEXT NOT NULL, \
   PRIMARY KEY(source, steps_hash, workspace) \
 );";

/// The Sessions schema version, stamped in `PRAGMA user_version` by
/// [`apply_schema`] (raised, never lowered). Every change so far is
/// additive, which every older build reads, so it is 0. Raise it in the
/// same change as a migration an older build would misread or damage (a
/// dropped or renamed column, a new NOT NULL column without a default, a
/// changed meaning): Go back then sees the newer store and offers the data
/// restore (`update_client::goback::known_schema`, row U6).
pub(crate) const SCHEMA_VERSION: i64 = 0;

pub(crate) fn apply_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions( \
           key TEXT PRIMARY KEY, \
           title TEXT, \
           agent TEXT NOT NULL, \
           cwd TEXT NOT NULL, \
           created_at TEXT NOT NULL, \
           last_activity_at TEXT NOT NULL, \
           ended_at TEXT, \
           turn_count INTEGER NOT NULL DEFAULT 0, \
           parent_key TEXT, \
           fork_kind TEXT, \
           adopted INTEGER NOT NULL DEFAULT 0 \
         ); \
         CREATE TABLE IF NOT EXISTS stints( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           session_key TEXT NOT NULL, \
           agent TEXT NOT NULL, \
           harness_session_id TEXT, \
           started_at TEXT NOT NULL, \
           ended_at TEXT, \
           reason TEXT \
         ); \
         CREATE TABLE IF NOT EXISTS turns( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           session_key TEXT NOT NULL, \
           ord INTEGER NOT NULL, \
           run_id TEXT, \
           started_at TEXT NOT NULL, \
           ended_at TEXT, \
           status TEXT, \
           asked TEXT, \
           UNIQUE(session_key, ord) \
         ); \
         CREATE TABLE IF NOT EXISTS events( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           session_key TEXT NOT NULL, \
           turn_ord INTEGER, \
           at TEXT NOT NULL, \
           kind TEXT NOT NULL, \
           payload TEXT \
         ); \
         CREATE TABLE IF NOT EXISTS transcripts( \
           session_key TEXT NOT NULL, \
           turn_ord INTEGER NOT NULL, \
           payload TEXT NOT NULL, \
           PRIMARY KEY(session_key, turn_ord) \
         ); \
         CREATE TABLE IF NOT EXISTS usage_ledger( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           run_key TEXT NOT NULL UNIQUE, \
           at TEXT NOT NULL, \
           started_at TEXT NOT NULL, \
           surface TEXT NOT NULL, \
           scope TEXT, \
           parent_surface TEXT, \
           run_id TEXT, \
           agent TEXT, \
           requested_model TEXT, \
           model TEXT, \
           native_session TEXT, \
           billing TEXT NOT NULL, \
           outcome TEXT NOT NULL, \
           spawned INTEGER NOT NULL, \
           basis TEXT NOT NULL, \
           input_tokens INTEGER, \
           cached_input_tokens INTEGER, \
           output_tokens INTEGER, \
           cost_usd REAL, \
           cost_status TEXT NOT NULL, \
           usage_json TEXT \
         ); \
         CREATE INDEX IF NOT EXISTS idx_stints_session ON stints(session_key); \
         CREATE INDEX IF NOT EXISTS idx_turns_session ON turns(session_key); \
         CREATE INDEX IF NOT EXISTS idx_turns_run ON turns(run_id); \
         CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_key, turn_ord); \
         CREATE INDEX IF NOT EXISTS idx_events_at ON events(at); \
         CREATE INDEX IF NOT EXISTS idx_usage_at ON usage_ledger(at); \
         CREATE INDEX IF NOT EXISTS idx_usage_run ON usage_ledger(run_id, outcome); \
         CREATE INDEX IF NOT EXISTS idx_usage_native ON usage_ledger(agent, native_session, at); \
         CREATE TABLE IF NOT EXISTS run_queue( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           source TEXT NOT NULL, \
           source_id TEXT, \
           label TEXT NOT NULL, \
           reason TEXT NOT NULL DEFAULT '', \
           payload_json TEXT NOT NULL DEFAULT '{}', \
           status TEXT NOT NULL, \
           coalesced_count INTEGER NOT NULL DEFAULT 0, \
           coalesced_into INTEGER, \
           idempotency_key TEXT, \
           priority INTEGER NOT NULL, \
           attempt INTEGER NOT NULL DEFAULT 0, \
           retry_of INTEGER, \
           requested_at TEXT NOT NULL, \
           not_before TEXT, \
           claimed_at TEXT, \
           finished_at TEXT, \
           history_run_id TEXT, \
           error TEXT, \
           phase_timings_json TEXT, \
           cancel_requested INTEGER NOT NULL DEFAULT 0, \
           held_reason TEXT, \
           waiting_since TEXT, \
           waiting_holder TEXT, \
           wait_count INTEGER NOT NULL DEFAULT 0, \
           last_waited_at TEXT, \
           detail TEXT \
         ); \
         CREATE UNIQUE INDEX IF NOT EXISTS idx_run_queue_key ON run_queue(idempotency_key) \
           WHERE idempotency_key IS NOT NULL AND status IN ('queued', 'claimed', 'running'); \
         CREATE INDEX IF NOT EXISTS idx_run_queue_ready ON run_queue(status, priority, id); \
         CREATE INDEX IF NOT EXISTS idx_run_queue_source ON run_queue(source, source_id, status); \
         CREATE INDEX IF NOT EXISTS idx_run_queue_run ON run_queue(history_run_id); \
         CREATE TABLE IF NOT EXISTS automation_occurrences( \
           slug TEXT NOT NULL, \
           due_at TEXT NOT NULL, \
           status TEXT NOT NULL, \
           finished_at TEXT NOT NULL, \
           PRIMARY KEY(slug, due_at) \
         ); \
         CREATE TABLE IF NOT EXISTS interactions( \
           id TEXT PRIMARY KEY, \
           session_key TEXT NOT NULL, \
           turn_ord INTEGER NOT NULL, \
           run_id TEXT NOT NULL, \
           agent TEXT NOT NULL, \
           native_session_id TEXT, \
           tool_use_id TEXT NOT NULL, \
           source TEXT NOT NULL, \
           mode TEXT NOT NULL, \
           request_json TEXT NOT NULL, \
           target_path TEXT, \
           target_abs TEXT, \
           target_hash TEXT, \
           supersede_on_user_message INTEGER NOT NULL DEFAULT 1, \
           status TEXT NOT NULL, \
           status_reason TEXT, \
           superseded_by TEXT, \
           answer_json TEXT, \
           answer_prompt TEXT, \
           answered_after_ord INTEGER, \
           created_at TEXT NOT NULL, \
           resolved_at TEXT \
         ); \
         CREATE INDEX IF NOT EXISTS idx_interactions_session ON interactions(session_key, status); \
         CREATE UNIQUE INDEX IF NOT EXISTS idx_interactions_run_tool \
           ON interactions(session_key, run_id, tool_use_id); \
         CREATE TABLE IF NOT EXISTS turn_feedback( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           session_key TEXT NOT NULL, \
           turn_ord INTEGER NOT NULL, \
           run_id TEXT NOT NULL, \
           vote TEXT NOT NULL, \
           reason TEXT, \
           agent TEXT, \
           model TEXT, \
           snapshot_json TEXT NOT NULL, \
           created_at TEXT NOT NULL, \
           updated_at TEXT NOT NULL, \
           UNIQUE(session_key, turn_ord) \
         ); \
         CREATE INDEX IF NOT EXISTS idx_feedback_updated ON turn_feedback(updated_at); \
         CREATE INDEX IF NOT EXISTS idx_feedback_run ON turn_feedback(run_id); \
         CREATE TABLE IF NOT EXISTS derived_state( \
           session_key TEXT NOT NULL, \
           kind TEXT NOT NULL, \
           revision INTEGER NOT NULL, \
           through_ord INTEGER NOT NULL, \
           payload TEXT NOT NULL, \
           updated_at TEXT NOT NULL, \
           PRIMARY KEY(session_key, kind) \
         ); \
         CREATE TABLE IF NOT EXISTS delegations( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           run_id TEXT NOT NULL, \
           at TEXT NOT NULL, \
           record_json TEXT NOT NULL \
         ); \
         CREATE INDEX IF NOT EXISTS idx_delegations_run ON delegations(run_id, id); \
         CREATE TABLE IF NOT EXISTS child_deliveries( \
           queue_id INTEGER PRIMARY KEY, \
           parent_key TEXT NOT NULL, \
           child_session TEXT NOT NULL, \
           via TEXT NOT NULL, \
           turn_ord INTEGER, \
           at TEXT NOT NULL \
         ); \
         CREATE TABLE IF NOT EXISTS root_deliveries( \
           queue_id INTEGER NOT NULL, \
           root_key TEXT NOT NULL, \
           asker TEXT NOT NULL DEFAULT '', \
           turn_ord INTEGER, \
           at TEXT NOT NULL, \
           PRIMARY KEY(root_key, queue_id) \
         ); \
         CREATE TABLE IF NOT EXISTS work_links( \
           session_key TEXT PRIMARY KEY, \
           issue_id TEXT, \
           issue_key TEXT, \
           workspace_id TEXT, \
           linked_by TEXT NOT NULL, \
           created_issue INTEGER NOT NULL DEFAULT 0, \
           at TEXT NOT NULL, \
           depth INTEGER NOT NULL DEFAULT 1, \
           tracked INTEGER NOT NULL DEFAULT 0, \
           outcome_at TEXT \
         ); \
         CREATE INDEX IF NOT EXISTS idx_work_links_issue ON work_links(issue_id); \
         CREATE TABLE IF NOT EXISTS work_events( \
           id INTEGER PRIMARY KEY AUTOINCREMENT, \
           session_key TEXT NOT NULL, \
           kind TEXT NOT NULL, \
           payload TEXT NOT NULL, \
           at TEXT NOT NULL, \
           journaled INTEGER NOT NULL DEFAULT 0 \
         ); \
         CREATE INDEX IF NOT EXISTS idx_work_events_open ON work_events(journaled, id);",
    )
    .map_err(|e| format!("session store schema: {e}"))?;
    ensure_column(
        conn,
        "sessions",
        "archived",
        "archived INTEGER NOT NULL DEFAULT 0",
    )?;
    conn.execute_batch(organization::SCHEMA)
        .map_err(|e| format!("conversation organization schema: {e}"))?;
    // T3g (2026-11-15): a run's title as it started, for its build log
    // comments.
    ensure_column(conn, "work_links", "title", "title TEXT")?;
    // 2026-11-04: the chat search (`search_sessions`) reads unarchived
    // conversations newest first from this index, so a page stops at its
    // last row instead of sorting the store. After `archived` exists, and
    // through `execute`, not a batch: the scripts' fixtures replay every
    // `execute_batch` here before the added columns.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_recent \
         ON sessions(archived, last_activity_at DESC, key)",
        [],
    )
    .map_err(|e| format!("sessions recent index: {e}"))?;
    // 2026-11-08: the ranked search lists the folders of unarchived chats by
    // stepping through this index (`search_cwds`), one seek per folder.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_cwd ON sessions(archived, cwd)",
        [],
    )
    .map_err(|e| format!("sessions folder index: {e}"))?;
    // 2026-11-12: a chat's own reasoning effort (the composer's Effort
    // list), kept here so every run of the chat resolves it, not only the
    // composer's sends. Keyed by session key, with no sessions row needed
    // (a level can be picked before the first turn mints the row).
    conn.execute(
        "CREATE TABLE IF NOT EXISTS chat_efforts( \
           key TEXT PRIMARY KEY, \
           effort TEXT NOT NULL, \
           updated_at TEXT NOT NULL \
         )",
        [],
    )
    .map_err(|e| format!("chat efforts table: {e}"))?;
    // 2026-11-15 (P1a): paused chats (`chat_pause`), keyed by session key,
    // so a paused chat survives a restart.
    conn.execute(crate::chat_pause::SCHEMA, [])
        .map_err(|e| format!("chat pauses table: {e}"))?;
    // M5 (2026-11-15): which Mac owns a chat that moved (`chat_move`).
    conn.execute_batch(crate::chat_move::SCHEMA)
        .map_err(|e| format!("chat moves schema: {e}"))?;
    // CH1 (2026-11-18): where each branched chat came from (`chat_branch`).
    conn.execute_batch(crate::chat_branch::SCHEMA)
        .map_err(|e| format!("chat branches schema: {e}"))?;
    // H5a (2026-11-18): imported conversations (`session_import`).
    conn.execute_batch(crate::session_import::SCHEMA)
        .map_err(|e| format!("imports schema: {e}"))?;
    // No ensure_column here: the table is new in this release and its
    // SCHEMA already holds every column (the agent-eval scripts replay this
    // function's literal batches and added columns, so an ALTER on a table
    // they cannot create would break them).
    // Paperclip PR 14b: what each native session was started with. NULL on
    // a legacy stint means unknown, never "changed" (no forced reset).
    ensure_column(
        conn,
        "stints",
        "config_fingerprint",
        "config_fingerprint TEXT",
    )?;
    // OB03 (2026-10-17): the fingerprint's per-part hashes, a JSON object
    // (`chat_resume::config_parts`), so a reset names what changed. NULL on
    // older stints, whose reset names `unknown`.
    ensure_column(conn, "stints", "config_parts", "config_parts TEXT")?;
    // 2026-09-28: the permission mode each chat turn ran with, so a turn a
    // `hollow_app` agent sends to another conversation never runs stronger
    // than that conversation last did. NULL on older turns (unknown).
    ensure_column(conn, "turns", "permission_mode", "permission_mode TEXT")?;
    // L1 (2026-11-10): the reasoning effort each chat turn ran with (a level,
    // `ultracode`, or NULL for the agent's own default and older turns), so
    // a follow-up keeps it and the Subagents list shows it.
    ensure_column(conn, "turns", "effort", "effort TEXT")?;
    // 2026-10-07: who asked a chat turn: `agent` when another agent sent it
    // through `hollow_app` (`start_session` / `send_to_session`), NULL when
    // the user did (and on older turns). The task-state record says who
    // settled a constraint (agent effectiveness F4, K2 follow-up).
    ensure_column(conn, "turns", "asked_by", "asked_by TEXT")?;
    ensure_column(conn, "turns", "capture_scope", "capture_scope TEXT")?;
    // 2026-10-20 (RF-797-F3): the constraints a durable child turn's task
    // packet carried, a JSON array of `{text, from}`, so they join the
    // child's task-state record and reach its own children. NULL on turns
    // with no packet (and on older turns).
    ensure_column(
        conn,
        "turns",
        "packet_constraints",
        "packet_constraints TEXT",
    )?;
    // 2026-11-07 (phone API v1.2): the files a turn's prompt carried in its
    // trailing `<attached-images>` / `<attached-files>` blocks, a JSON array
    // `phone_access::attachments` writes and reads (file names inside the
    // chat folder's `.hollow-attachments/`, never paths). NULL on turns with
    // none and on older turns.
    ensure_column(conn, "turns", "attachments", "attachments TEXT")?;
    // 2026-10-08, run ledger v2 (master plan F2): who paid for a run and
    // what it was for (`run_attribution`), and what became of its work
    // (`outcome_label`, derived by `usage_ledger::refresh_outcome_labels`).
    // NULL on older rows, which read as `unknown`.
    for (column, ddl) in [
        ("account_ref", "account_ref TEXT"),
        ("workspace_id", "workspace_id TEXT"),
        ("department", "department TEXT"),
        ("work_kind", "work_kind TEXT"),
        ("outcome_label", "outcome_label TEXT"),
    ] {
        ensure_column(conn, "usage_ledger", column, ddl)?;
    }
    // OB07 (2026-10-17): a priced lower bound on what a run with tokens but
    // no cost spent (`usage_prices::floor_usd`). Never a cost: known totals
    // and budgets read `cost_usd` only. NULL on older rows.
    ensure_column(
        conn,
        "usage_ledger",
        "cost_floor_usd",
        "cost_floor_usd REAL",
    )?;
    // 2026-09-30 to run 15 P4: the session importers' `external_*` tables
    // (From other apps). Removed with the feature; databases that have them
    // keep them, unread, and nothing creates them any more.
    // 2026-10-09: Scheduler v2's night queue (`night_queue`); inert since 11-10
    // (S6), kept so existing databases open the same way.
    conn.execute_batch(crate::night_queue::SCHEMA)
        .map_err(|e| format!("night queue schema: {e}"))?;
    // 2026-10-09: the overnight shift's links (master plan #1).
    for (column, ddl) in crate::night_queue::ADDED_COLUMNS {
        ensure_column(conn, "night_queue", column, ddl)?;
    }
    // 2026-10-09: executable runbooks (master plan #11). The feature was
    // removed 11-10 (S6); the tables stay, inert, so databases open as before.
    conn.execute_batch(RUNBOOK_SCHEMA)
        .map_err(|e| format!("runbooks schema: {e}"))?;
    // 2026-11-10: the audit record (COO rebuild X2, `audit`), append-only.
    conn.execute_batch(crate::audit::SCHEMA)
        .map_err(|e| format!("audit schema: {e}"))?;
    // 2026-11-13: old-to-new workspace ids after Move Hollow home
    // (`workspace_aliases`), append-only; audit, usage and history readers
    // resolve through it.
    conn.execute_batch(crate::workspace_aliases::SCHEMA)
        .map_err(|e| format!("workspace aliases schema: {e}"))?;
    // 2026-10-09: the step spine (master plan F6, `step_spine`).
    conn.execute_batch(crate::step_spine::SCHEMA)
        .map_err(|e| format!("step spine schema: {e}"))?;
    // 2026-10-09: the friction loop's patterns (master plan #13, `friction`).
    conn.execute_batch(crate::friction::SCHEMA)
        .map_err(|e| format!("friction schema: {e}"))?;
    conn.execute_batch(crate::work_origin::SCHEMA)
        .map_err(|e| format!("work origin schema: {e}"))?;
    // The paused AI company (11-09) and its restricted workflows and priority
    // pause: removed 11-10 (L11). Their 46 tables stay, inert, in databases
    // that have them; nothing creates or reads them any more except the CFO's
    // one-time `company_settings` migration below.
    // COO rebuild L7: the CFO's policy (migrated once from company_settings).
    conn.execute_batch(crate::cfo::settings::SCHEMA)
        .map_err(|e| format!("cfo settings schema: {e}"))?;
    fulltext::schema(conn)?;
    // L1 (2026-11-10): subagents started before their parent was recorded.
    backfill_subagent_parents(conn)?;
    crate::hollow_databases::stamp_schema_version(conn, SCHEMA_VERSION)
}

/// `sessions.fork_kind` of a conversation another conversation started as
/// its subagent (L1). A handover's successor keeps its own (none): it is the
/// same chat continued, not a subagent.
pub const FORK_SUBAGENT: &str = "subagent";

/// `sessions.fork_kind` of a side chat (t3code parity 7.6, `side_chat.rs`):
/// the person's own chat beside another one, kept off every chat list
/// (the SQL below names it as `'side'`).
pub const FORK_SIDE: &str = "side";

/// The delegation tools that start a subagent: `start_session`, a worker
/// batch's entries and a swarm's parts (a handover continues the same chat,
/// and `send_to_session` messages one that exists).
pub(crate) const SUBAGENT_START_TOOLS: [&str; 3] =
    ["start_session", "start_workers", "start_swarm"];

/// Record that conversation `key` is conversation `parent`'s subagent: once,
/// on a conversation with no parent yet, and never as its own parent. `Ok(false)`
/// when nothing changed (already parented, or no such conversation yet).
pub fn set_parent(vault: &Path, key: &str, parent: &str, kind: &str) -> Result<bool, String> {
    if !is_safe_session_key(key) || !is_safe_session_key(parent) || key == parent {
        return Ok(false);
    }
    with_write(vault, "subagent parent", |conn| {
        conn.execute(
            "UPDATE sessions SET parent_key = ?2, fork_kind = ?3 \
             WHERE key = ?1 AND parent_key IS NULL AND fork_kind IS NULL",
            params![key, parent, kind],
        )
        .map(|n| n > 0)
        .map_err(|e| format!("subagent parent: {e}"))
    })
}

/// [`backfill_subagent_parents`] on demand (tests: the schema is applied
/// once per process, before a test writes the records it links).
#[cfg(test)]
pub(crate) fn backfill_subagents(vault: &Path) -> Result<(), String> {
    with_write(vault, "subagent parents backfill", |tx| {
        backfill_subagent_parents(tx)
    })
}

/// Link conversations started before L1 to the conversation that started
/// them, from the child-task delegation records (`session_starters`' join,
/// newest record first), on every conversation with no parent yet. Runs
/// when the schema is applied; a conversation already linked, or one whose
/// starter's turn is no longer recorded, is left alone.
fn backfill_subagent_parents(conn: &Connection) -> Result<(), String> {
    let tools = SUBAGENT_START_TOOLS
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(",");
    conn.execute(
        &format!(
            "UPDATE sessions SET parent_key = p.parent, fork_kind = '{FORK_SUBAGENT}' FROM ( \
               SELECT child, parent, MAX(id) FROM ( \
                 SELECT json_extract(d.record_json, '$.childSession') AS child, \
                        t.session_key AS parent, d.id AS id \
                 FROM delegations d JOIN turns t ON t.run_id = d.run_id \
                 WHERE json_extract(d.record_json, '$.kind') = 'childTask' \
                 AND json_extract(d.record_json, '$.tool') IN ({tools})) \
               WHERE child IS NOT NULL GROUP BY child) p \
             WHERE sessions.key = p.child AND sessions.parent_key IS NULL \
             AND sessions.fork_kind IS NULL \
             AND p.parent != p.child"
        ),
        [],
    )
    .map(|_| ())
    .map_err(|e| format!("subagent parents backfill: {e}"))
}

/// Five asks B4's timing fixture: `sessions` conversations in `cwds` (round
/// robin), `turns` turns each, every turn a stored transcript of
/// `transcript_bytes` text. Written in one transaction straight to the
/// tables, so a realistic store seeds in well under a second. Since
/// 2026-11-04 the messages are what the chat search reads: about 2 KB each,
/// and a pasted 64 KB one opening every tenth conversation, none holding
/// the timed searches' words ("plan", "conversation").
#[cfg(test)]
pub(crate) fn seed_for_perf(
    vault: &Path,
    cwds: &[String],
    sessions: usize,
    turns: usize,
    transcript_bytes: usize,
) -> Result<(), String> {
    let text =
        "Reviewed the layout and changed the spacing tokens. ".repeat(transcript_bytes / 52 + 1);
    let asked = "Review the next part of the layout and note what changed. ".repeat(36);
    let pasted = "Pasted log line: the layout pass finished without errors. ".repeat(1_100);
    with_write(vault, "perf seed", |tx| {
        for i in 0..sessions {
            let key = format!("s-20261010-{i:08x}");
            let at = format!(
                "2026-10-{:02}T{:02}:{:02}:00+00:00",
                1 + i % 9,
                i % 24,
                i % 60
            );
            tx.execute(
                "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, turn_count) \
                 VALUES(?1, ?2, 'claude', ?3, ?4, ?4, ?5)",
                params![key, format!("Conversation {i}"), cwds[i % cwds.len()], at, turns as i64],
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO stints(session_key, agent, harness_session_id, started_at) VALUES(?1, 'claude', ?2, ?3)",
                params![key, format!("h-{i}"), at],
            )
            .map_err(|e| e.to_string())?;
            for ord in 0..turns as i64 {
                let message = if i % 10 == 0 && ord == 0 {
                    &pasted
                } else {
                    &asked
                };
                tx.execute(
                    "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked) \
                     VALUES(?1, ?2, ?3, ?4, ?4, 'done', ?5)",
                    params![key, ord, format!("run-{i}-{ord}"), at, message],
                )
                .map_err(|e| e.to_string())?;
                let transcript = StoredTranscript {
                    reasoning: String::new(),
                    agent: "claude".into(),
                    text: text[..transcript_bytes.min(text.len())].to_string(),
                    tools: Vec::new(),
                    cost_usd: Some(0.2),
                    error: None,
                    incomplete: false,
                    result_kind: Some("success".into()),
                    recall: Vec::new(),
                    stopped_before_spawn: false,
                    steers: Vec::new(),
                };
                write_transcript(tx, &key, ord, &transcript)?;
            }
        }
        Ok(())
    })
}

/// Add `column` to `table` when it is missing (`CREATE TABLE IF NOT EXISTS`
/// never migrates an existing table). `ddl` is the column definition. Another
/// caller may have opened the database at the same time; SQLite serializes
/// DDL, so only an already-added column is tolerated.
fn ensure_column(conn: &Connection, table: &str, column: &str, ddl: &str) -> Result<(), String> {
    let present = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("{table} schema inspect: {e}"))?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("{table} schema columns: {e}"))?
        .filter_map(Result::ok)
        .any(|name| name == column);
    if !present {
        if let Err(error) = conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {ddl}"), []) {
            if !error.to_string().contains("duplicate column name") {
                return Err(format!("{table}.{column} migration: {error}"));
            }
        }
    }
    Ok(())
}

/// Run `f` inside one `BEGIN IMMEDIATE` transaction under [`WRITE_LOCK`], and
/// commit. IMMEDIATE takes the database write lock up front, so the busy
/// timeout applies instead of a mid-transaction `SQLITE_BUSY_SNAPSHOT` when
/// another process (a team-mcp delegate's usage row) writes in between. Use it
/// for every read-then-write; PR 7's queue reuses it.
pub(crate) fn with_write<T>(
    vault: &Path,
    what: &str,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = write_lock();
    let mut conn = open(vault)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| format!("{what} begin: {e}"))?;
    let out = f(&tx)?;
    tx.commit().map_err(|e| format!("{what} commit: {e}"))?;
    Ok(out)
}

/// Run `f` on a fresh connection, with no transaction and no lock (reads).
pub(crate) fn with_read<T>(
    vault: &Path,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let conn = open(vault)?;
    f(&conn)
}

/// One conversation's newest `limit` trace events of `kind`, oldest first,
/// as raw payloads. Overnight 11-02's morning report read its `remoteRun`
/// notices this way; since the Q7 follow-up it reads [`remote_run_trail`],
/// and this plain read stays for the test that shows what the trail fixes.
#[cfg(test)]
pub(crate) fn event_payloads(
    vault: &Path,
    key: &str,
    kind: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT COALESCE(payload, '') FROM events WHERE session_key = ?1 AND kind = ?2 \
                 ORDER BY id DESC LIMIT ?3",
            )
            .map_err(|e| format!("events prepare: {e}"))?;
        let mut rows = stmt
            .query_map(params![key, kind, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("events query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("events rows: {e}"))?;
        rows.reverse();
        Ok(rows)
    })
}

/// Conversation `key`'s newest `limit` `remoteRun` notices of phase `start`
/// or `back`, oldest first, as raw payloads. Self-hosted models Q7 review:
/// what `night_queue::left_on_runner` reads for a self-hosted turn's note
/// and (Q7 follow-up) for the morning report, so the chat's own `stayed`
/// notes (one a turn) never push its last `start` out of the window. A
/// payload that isn't JSON is skipped.
pub(crate) fn remote_run_trail(
    vault: &Path,
    key: &str,
    limit: usize,
) -> Result<Vec<String>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT payload FROM events WHERE session_key = ?1 AND kind = 'remoteRun' \
                 AND CASE WHEN json_valid(payload) THEN json_extract(payload, '$.phase') END \
                 IN ('start', 'back') ORDER BY id DESC LIMIT ?2",
            )
            .map_err(|e| format!("remote trail prepare: {e}"))?;
        let mut rows = stmt
            .query_map(params![key, limit as i64], |row| row.get::<_, String>(0))
            .map_err(|e| format!("remote trail query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("remote trail rows: {e}"))?;
        rows.reverse();
        Ok(rows)
    })
}

/// Every conversation's key and folder, archived ones included, with no
/// limit and no per-row reads (five asks A2: which workspace started which).
pub fn session_cwds(vault: &Path) -> Result<Vec<(String, String)>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT key, cwd FROM sessions")
            .map_err(|e| format!("session folders: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("session folders: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("session folders: {e}"));
        rows
    })
}

/// The run-queue owner lock (Paperclip PR 7): beside this vault's session
/// database, so exactly one Hollow process owns (recovers and drains) a
/// vault's queue. Outside the vault, never watched or captured.
pub(crate) fn pump_lock_path(vault: &Path) -> Option<PathBuf> {
    db_path_for(vault).map(|db| db.with_extension("pump.lock"))
}

/// [`pump_lock_path`] under another support folder: the app's own, for a
/// `hollow serve --data-dir` whose stores live elsewhere (`serve.rs`).
pub(crate) fn pump_lock_path_under(support: &Path, vault: &Path) -> PathBuf {
    support
        .join("Sessions")
        .join(format!("{}.pump.lock", vault_file_stem(vault)))
}

fn cap(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}… (truncated)")
}

/// Cap structured tool data without ever slicing the JSON document itself.
/// The previous raw string cap broke large tool events on history reload.
fn bounded_event(kind: &str, payload: &str) -> String {
    let limit = if kind == "reasoning" {
        16_001
    } else if kind == "assistant" {
        ASSISTANT_CHARS
    } else {
        EVENT_PAYLOAD_CHARS
    };
    if payload.chars().count() <= limit {
        return payload.to_string();
    }
    if !matches!(kind, "toolUse" | "toolResult" | "result") {
        return cap(payload, limit);
    }
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        return cap(payload, limit);
    };
    let field = if kind == "toolUse" {
        "input"
    } else if kind == "toolResult" {
        "content"
    } else {
        "text"
    };
    let source = value
        .get(field)
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string())
        })
        .unwrap_or_default();
    let mut bounded = json!({"truncated": true});
    for key in [
        "id",
        "name",
        "isError",
        "costUsd",
        "subtype",
        "resultKind",
        "parentToolUseId",
    ] {
        if let Some(v) = value.get(key) {
            bounded[key] = v
                .as_str()
                .map(|s| Value::String(cap(s, 256)))
                .unwrap_or_else(|| v.clone());
        }
    }
    // Binary search the preview's character count because escaping can make
    // one character consume six JSON characters, and UTF-8 slicing is unsafe.
    let chars: Vec<_> = source.chars().collect();
    let mut low = 0;
    let mut high = chars.len().min(limit);
    let render = |n: usize| {
        let preview = format!("{}… (truncated)", chars.iter().take(n).collect::<String>());
        let mut next = bounded.clone();
        next[field] = if field == "input" {
            json!({"truncated": true, "preview": preview})
        } else {
            json!(preview)
        };
        next.to_string()
    };
    while low < high {
        let mid = (low + high).div_ceil(2);
        if render(mid).chars().count() <= limit {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    render(low)
}

fn apply_event(transcript: &mut StoredTranscript, kind: &str, payload: &str) {
    if kind == "reasoning" {
        transcript.reasoning = cap(payload, 16_001);
        return;
    }
    if kind == "assistant" {
        transcript.text = cap(payload, ASSISTANT_CHARS);
        transcript.incomplete |= payload.contains("… (truncated)");
        return;
    }
    if kind == "error" {
        transcript.error = Some(cap(payload, EVENT_PAYLOAD_CHARS));
        return;
    }
    if kind == "cancelled" {
        transcript.stopped_before_spawn = payload == "before-spawn";
        return;
    }
    if kind == "recall" {
        // `{"paths": [...]}` from [`record_recall`]. Malformed input keeps
        // whatever was recorded before; it never marks the prose incomplete.
        if let Ok(value) = serde_json::from_str::<Value>(payload) {
            if let Some(paths) = value.get("paths").and_then(Value::as_array) {
                transcript.recall = paths
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|path| !path.is_empty() && path.chars().count() <= RECALL_PATH_CHARS)
                    .take(MAX_RECALL_PATHS)
                    .map(str::to_owned)
                    .collect();
            }
        }
        return;
    }
    if !matches!(
        kind,
        "toolUse" | "toolResult" | "result" | "steer" | "steerState"
    ) {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(payload) else {
        transcript.incomplete = true;
        return;
    };
    transcript.incomplete |= value
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let string = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    match kind {
        "toolUse" => {
            if transcript.tools.len() >= MAX_TRANSCRIPT_TOOLS {
                transcript.incomplete = true;
                return;
            }
            transcript.tools.push(ConversationTool {
                id: string("id"),
                name: string("name"),
                input: value.get("input").cloned().unwrap_or(Value::Null),
                result: None,
                is_error: false,
                status: "running".to_string(),
                parent_tool_use_id: value
                    .get("parentToolUseId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned),
                result_chars: None,
                result_lines: None,
                result_turn: None,
            });
        }
        "toolResult" => {
            let id = string("id");
            let tool = transcript.tools.iter_mut().rev().find(|tool| {
                if id.is_empty() {
                    tool.status == "running"
                } else {
                    tool.id == id
                }
            });
            if let Some(tool) = tool {
                tool.result = Some(
                    value
                        .get("content")
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string())
                        })
                        .unwrap_or_default(),
                );
                tool.is_error = value
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                tool.status = if tool.is_error { "error" } else { "done" }.to_string();
            } else {
                transcript.incomplete = true;
            }
        }
        "steer" => {
            if transcript.steers.len() >= crate::steering::MAX_STEERS_PER_TURN {
                transcript.incomplete = true;
                return;
            }
            if let Ok(record) =
                serde_json::from_value::<crate::steering::SteerRecord>(value.clone())
            {
                transcript.steers.push(record);
            }
        }
        "steerState" => {
            let id = string("id");
            let state = match value.get("state").and_then(Value::as_str) {
                Some("delivered") => crate::steering::SteerState::Delivered,
                Some("notDelivered") => crate::steering::SteerState::NotDelivered,
                _ => return,
            };
            if let Some(steer) = transcript.steers.iter_mut().find(|s| s.id == id) {
                steer.state = state;
                // A refusal's reason (P1-30), already cut by `steering::refuse`.
                steer.reason = value
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(|r| r.chars().take(crate::steering::MAX_REFUSAL_CHARS).collect());
            }
        }
        "result" => {
            if let Some(cost) = value.get("costUsd").and_then(Value::as_f64) {
                transcript.cost_usd = Some(cost);
            }
            if let Some(kind) = value.get("resultKind").and_then(Value::as_str) {
                transcript.result_kind = Some(kind.to_string());
            }
            if transcript.text.is_empty() {
                if let Some(text) = value
                    .get("text")
                    .or_else(|| value.get("result"))
                    .and_then(Value::as_str)
                {
                    transcript.text = cap(text, ASSISTANT_CHARS);
                }
            }
            if value.get("isError").and_then(Value::as_bool) == Some(true) {
                transcript.error = Some(
                    value
                        .get("subtype")
                        .and_then(Value::as_str)
                        .unwrap_or("The agent reported an error.")
                        .to_string(),
                );
            }
        }
        _ => {}
    }
}

fn write_transcript(
    conn: &Connection,
    key: &str,
    ord: i64,
    transcript: &StoredTranscript,
) -> Result<(), String> {
    let payload =
        serde_json::to_string(transcript).map_err(|e| format!("transcript encode: {e}"))?;
    conn.execute(
        "INSERT INTO transcripts(session_key, turn_ord, payload) VALUES(?1, ?2, ?3) \
         ON CONFLICT(session_key, turn_ord) DO UPDATE SET payload = ?3",
        params![key, ord, payload],
    )
    .map_err(|e| format!("transcript write: {e}"))?;
    Ok(())
}

/// One imported turn's fields (H5a, `session_import`).
pub(crate) struct ImportedTurn<'a> {
    pub agent: &'a str,
    pub started_at: &'a str,
    pub ended_at: &'a str,
    pub asked: &'a str,
    pub reply: &'a str,
}

/// Write imported turn `ord` of conversation `key` (H5a): a finished
/// `turns` row with no run (a run id names a Hollow run) and its transcript
/// in the normalized payload every chat uses. Inside the import's own
/// transaction.
pub(crate) fn write_imported_turn(
    conn: &Connection,
    key: &str,
    ord: i64,
    turn: &ImportedTurn<'_>,
    tools: Vec<ConversationTool>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked) \
         VALUES(?1, ?2, NULL, ?3, ?4, 'success', ?5)",
        params![key, ord, turn.started_at, turn.ended_at, turn.asked],
    )
    .map_err(|e| format!("imported turn: {e}"))?;
    write_transcript(
        conn,
        key,
        ord,
        &StoredTranscript {
            agent: turn.agent.to_string(),
            text: turn.reply.to_string(),
            tools,
            ..StoredTranscript::default()
        },
    )
}

/// Turn `ord`'s reply text (CH1's branch handoff).
pub(crate) fn transcript_text(conn: &Connection, key: &str, ord: i64) -> Result<String, String> {
    load_transcript(conn, key, ord).map(|t| t.text)
}

fn load_transcript(conn: &Connection, key: &str, ord: i64) -> Result<StoredTranscript, String> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT payload FROM transcripts WHERE session_key = ?1 AND turn_ord = ?2",
            params![key, ord],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("transcript read: {e}"))?;
    if let Some(payload) = stored {
        return serde_json::from_str(&payload).map_err(|e| format!("transcript decode: {e}"));
    }
    let agent: String = conn
        .query_row(
            "SELECT COALESCE((SELECT s.agent FROM stints s WHERE s.session_key = t.session_key \
           AND s.started_at <= t.started_at ORDER BY s.id DESC LIMIT 1), \
           (SELECT agent FROM sessions WHERE key = t.session_key)) \
         FROM turns t WHERE t.session_key = ?1 AND t.ord = ?2",
            params![key, ord],
            |row| row.get(0),
        )
        .map_err(|e| format!("transcript provider: {e}"))?;
    let mut transcript = StoredTranscript {
        agent,
        ..StoredTranscript::default()
    };
    let mut stmt = conn.prepare(
        "SELECT kind, payload FROM events WHERE session_key = ?1 AND turn_ord = ?2 ORDER BY id ASC"
    ).map_err(|e| format!("transcript events prepare: {e}"))?;
    let rows = stmt
        .query_map(params![key, ord], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("transcript events query: {e}"))?;
    let mut any = false;
    for row in rows {
        let (kind, payload) = row.map_err(|e| format!("transcript event: {e}"))?;
        any = true;
        apply_event(&mut transcript, &kind, &payload);
    }
    transcript.incomplete |= !any;
    Ok(transcript)
}

/// Open a turn. Upserts the session row, opens a stint if the agent changed,
/// and returns the turn's ordinal. Refuses an unsafe key rather than letting
/// it reach a SQL parameter or, later, a filename.
pub fn start_turn(
    vault: &Path,
    key: &str,
    agent: &str,
    cwd: &str,
    run_id: &str,
    asked: &str,
) -> Result<i64, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_write(vault, "session", |conn| {
        start_turn_in(conn, key, agent, cwd, run_id, asked)
    })
}

fn start_turn_in(
    conn: &Connection,
    key: &str,
    agent: &str,
    cwd: &str,
    run_id: &str,
    asked: &str,
) -> Result<i64, String> {
    // H5a: an imported record is read-only; Continue in Hollow carries it on.
    if crate::session_import::is_imported(conn, key) {
        return Err(crate::session_import::READ_ONLY.to_string());
    }
    let now = Utc::now().to_rfc3339();
    // A short title from the first request (overnight F1); set once, so a
    // rename sticks.
    let title = crate::chat_title::short_title(asked);

    conn.execute(
        "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, turn_count) \
         VALUES(?1, ?2, ?3, ?4, ?5, ?5, 0) \
         ON CONFLICT(key) DO UPDATE SET last_activity_at = ?5, agent = ?3, ended_at = NULL, archived = 0",
        params![key, title, agent, cwd, now],
    )
    .map_err(|e| format!("session upsert: {e}"))?;

    // A stint is open when its ended_at is null. Open one only when there is
    // no open stint for this agent, so a multi-turn conversation on one
    // harness stays a single stint.
    let open_stint: Option<String> = conn
        .query_row(
            "SELECT agent FROM stints WHERE session_key = ?1 AND ended_at IS NULL \
             ORDER BY id DESC LIMIT 1",
            params![key],
            |row| row.get(0),
        )
        .ok();
    if open_stint.as_deref() != Some(agent) {
        if open_stint.is_some() {
            conn.execute(
                "UPDATE stints SET ended_at = ?2, reason = 'agent changed' \
                 WHERE session_key = ?1 AND ended_at IS NULL",
                params![key, now],
            )
            .map_err(|e| format!("stint close: {e}"))?;
        }
        conn.execute(
            "INSERT INTO stints(session_key, agent, started_at) VALUES(?1, ?2, ?3)",
            params![key, agent, now],
        )
        .map_err(|e| format!("stint open: {e}"))?;
    }

    let ord: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(ord), 0) + 1 FROM turns WHERE session_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .map_err(|e| format!("turn ord: {e}"))?;
    conn.execute(
        "INSERT INTO turns(session_key, ord, run_id, started_at, status, asked) \
         VALUES(?1, ?2, ?3, ?4, 'running', ?5)",
        params![key, ord, run_id, now, cap(asked, ASSISTANT_CHARS)],
    )
    .map_err(|e| format!("turn insert: {e}"))?;
    conn.execute(
        "UPDATE sessions SET turn_count = (SELECT COUNT(*) FROM turns WHERE session_key = ?1) \
         WHERE key = ?1",
        params![key],
    )
    .map_err(|e| format!("turn count: {e}"))?;
    write_transcript(
        conn,
        key,
        ord,
        &StoredTranscript {
            agent: agent.to_string(),
            incomplete: asked.chars().count() > ASSISTANT_CHARS,
            ..StoredTranscript::default()
        },
    )?;
    Ok(ord)
}

/// Record the harness's own session/thread id for the open stint. Providers
/// that do not announce an explicit id leave it absent instead of a sentinel.
/// Leaves the stint's config fingerprint untouched (see
/// [`record_harness_stamp`], which the chat turn uses). Kept, with its
/// signature, for the tests and other callers that record an id alone.
#[cfg(test)]
pub fn record_harness_id(vault: &Path, key: &str, harness_session_id: &str) -> Result<(), String> {
    record_harness(vault, key, harness_session_id, None)
}

/// [`record_harness_id`], also writing the configuration fingerprint the
/// native session was started with (Paperclip PR 14b): `Some` stores it,
/// `None` stores NULL (unknown). Its parts are cleared (OB03): a
/// fingerprint alone says nothing per part. The chat turn records both
/// ([`record_harness_stamp`]); kept for the tests.
#[cfg(test)]
pub fn record_harness_id_with(
    vault: &Path,
    key: &str,
    harness_session_id: &str,
    fingerprint: Option<&str>,
) -> Result<(), String> {
    record_harness(vault, key, harness_session_id, Some((fingerprint, None)))
}

/// Record a newly announced native id with the configuration fingerprint
/// the native session was started with (Paperclip PR 14b) and its per-part
/// hashes (OB03, `chat_resume::config_parts`), as the chat turn does.
pub fn record_harness_stamp(
    vault: &Path,
    key: &str,
    harness_session_id: &str,
    fingerprint: &str,
    parts: &str,
) -> Result<(), String> {
    let stamp = (Some(fingerprint), Some(parts));
    record_harness(vault, key, harness_session_id, Some(stamp))
}

/// `stamp`: `None` leaves the fingerprint and its parts alone;
/// `Some((fingerprint, parts))` writes both.
fn record_harness(
    vault: &Path,
    key: &str,
    harness_session_id: &str,
    stamp: Option<(Option<&str>, Option<&str>)>,
) -> Result<(), String> {
    if !is_safe_session_key(key) || harness_session_id.trim().is_empty() {
        return Ok(());
    }
    let _guard = write_lock();
    let conn = open(vault)?;
    let written = match stamp {
        None => conn.execute(
            "UPDATE stints SET harness_session_id = ?2 \
             WHERE session_key = ?1 AND ended_at IS NULL",
            params![key, harness_session_id],
        ),
        Some((fingerprint, parts)) => conn.execute(
            "UPDATE stints SET harness_session_id = ?2, config_fingerprint = ?3, \
             config_parts = ?4 WHERE session_key = ?1 AND ended_at IS NULL",
            params![key, harness_session_id, fingerprint, parts],
        ),
    };
    written.map_err(|e| format!("harness id: {e}"))?;
    Ok(())
}

/// The open stint and the most recently closed one (Paperclip PR 14b's
/// resume plan). An unsafe key reads as an empty state.
pub fn resume_state(vault: &Path, key: &str) -> Result<ResumeState, String> {
    if !is_safe_session_key(key) {
        return Ok(ResumeState::default());
    }
    with_read(vault, |conn| {
        let open = conn
            .query_row(
                "SELECT agent, harness_session_id, config_fingerprint, config_parts FROM stints \
                 WHERE session_key = ?1 AND ended_at IS NULL ORDER BY id DESC LIMIT 1",
                params![key],
                |row| {
                    Ok(OpenStint {
                        agent: row.get(0)?,
                        harness_session_id: row.get(1)?,
                        config_fingerprint: row.get(2)?,
                        config_parts: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|e| format!("open stint: {e}"))?;
        let last_closed = conn
            .query_row(
                "SELECT harness_session_id, reason, config_parts FROM stints \
                 WHERE session_key = ?1 AND ended_at IS NOT NULL ORDER BY id DESC LIMIT 1",
                params![key],
                |row| {
                    Ok(ClosedStint {
                        harness_session_id: row.get(0)?,
                        reason: row.get(1)?,
                        config_parts: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|e| format!("closed stint: {e}"))?;
        Ok(ResumeState { open, last_closed })
    })
}

/// Close the conversation's open stint with `reason` and open a fresh one for
/// `agent` (Paperclip PR 14b: a fresh native session, because the config
/// changed or the old session could not be resumed). In the same IMMEDIATE
/// transaction, the conversation's open question cards are expired
/// (`session_reset`) and returned, so the caller can emit them: a card
/// belongs to the old native session, which is gone (cross-m34 H4).
pub fn restart_stint(
    vault: &Path,
    key: &str,
    agent: &str,
    reason: &str,
) -> Result<Vec<crate::types::ChatInteraction>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_write(vault, "stint restart", |tx| {
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE stints SET ended_at = ?2, reason = ?3 \
             WHERE session_key = ?1 AND ended_at IS NULL",
            params![key, now, reason],
        )
        .map_err(|e| format!("stint close: {e}"))?;
        tx.execute(
            "INSERT INTO stints(session_key, agent, started_at) VALUES(?1, ?2, ?3)",
            params![key, agent, now],
        )
        .map_err(|e| format!("stint open: {e}"))?;
        crate::interactions::expire_pending_rows_in(tx, key, &db_time(Utc::now()), "session_reset")
    })
}

/// Fill the open stint's fingerprint when it is still NULL and the stint
/// holds `harness_session_id` (Paperclip PR 14b). A resumed native session
/// re-announces the same id, so [`record_harness_stamp`] never runs for
/// it; this backfills a legacy stint after its first successful resume.
/// Returns whether a row changed.
pub fn stamp_config_fingerprint(
    vault: &Path,
    key: &str,
    harness_session_id: &str,
    fingerprint: &str,
) -> Result<bool, String> {
    if !is_safe_session_key(key) || harness_session_id.trim().is_empty() {
        return Ok(false);
    }
    let _guard = write_lock();
    let conn = open(vault)?;
    let changed = conn
        .execute(
            "UPDATE stints SET config_fingerprint = ?3 \
             WHERE session_key = ?1 AND ended_at IS NULL AND harness_session_id = ?2 \
             AND config_fingerprint IS NULL",
            params![key, harness_session_id, fingerprint],
        )
        .map_err(|e| format!("stint fingerprint: {e}"))?;
    Ok(changed > 0)
}

/// Overwrite the open stint's fingerprint when an older version (`v1:` or
/// `v2:`) stamped it and the stint holds `harness_session_id` (OB04's
/// version restamp, after a resume `chat_resume::may_restamp` allowed
/// succeeded). Never touches a NULL or a current-version stamp; returns
/// whether a row changed.
pub fn restamp_config_fingerprint(
    vault: &Path,
    key: &str,
    harness_session_id: &str,
    fingerprint: &str,
) -> Result<bool, String> {
    if !is_safe_session_key(key) || harness_session_id.trim().is_empty() {
        return Ok(false);
    }
    let _guard = write_lock();
    let conn = open(vault)?;
    let changed = conn
        .execute(
            "UPDATE stints SET config_fingerprint = ?3 \
             WHERE session_key = ?1 AND ended_at IS NULL AND harness_session_id = ?2 \
             AND (substr(config_fingerprint, 1, 3) = 'v1:' \
                  OR substr(config_fingerprint, 1, 3) = 'v2:')",
            params![key, harness_session_id, fingerprint],
        )
        .map_err(|e| format!("stint restamp: {e}"))?;
    Ok(changed > 0)
}

/// Fill the open stint's fingerprint parts (OB03) when they are still NULL,
/// the stint holds `harness_session_id` and its fingerprint is
/// `fingerprint`, after a successful resume: a stint stamped before parts
/// existed, or one [`stamp_config_fingerprint`] or
/// [`restamp_config_fingerprint`] just stamped, learns them. Never
/// overwrites; returns whether a row changed.
pub fn stamp_config_parts(
    vault: &Path,
    key: &str,
    harness_session_id: &str,
    fingerprint: &str,
    parts: &str,
) -> Result<bool, String> {
    if !is_safe_session_key(key) || harness_session_id.trim().is_empty() {
        return Ok(false);
    }
    let _guard = write_lock();
    let conn = open(vault)?;
    let changed = conn
        .execute(
            "UPDATE stints SET config_parts = ?4 \
             WHERE session_key = ?1 AND ended_at IS NULL AND harness_session_id = ?2 \
             AND config_fingerprint = ?3 AND config_parts IS NULL",
            params![key, harness_session_id, fingerprint, parts],
        )
        .map_err(|e| format!("stint parts: {e}"))?;
    Ok(changed > 0)
}

/// Turn ranges a checkpoint restore undid (CP1, `turn_checkpoints`): each
/// `checkpoint_restore` event's `before..=through`. A fresh native session's
/// history leaves them out; the transcript keeps them.
fn undone_ords(conn: &Connection, key: &str) -> Result<Vec<(i64, i64)>, String> {
    let mut stmt = conn
        .prepare("SELECT payload FROM events WHERE session_key = ?1 AND kind = ?2")
        .map_err(|e| format!("undone prepare: {e}"))?;
    let rows = stmt
        .query_map(
            params![key, crate::turn_checkpoints::RESTORE_EVENT],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|e| format!("undone query: {e}"))?;
    let mut out = Vec::new();
    for payload in rows.flatten().flatten() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        if let (Some(from), Some(to)) = (v["before"].as_i64(), v["through"].as_i64()) {
            out.push((from, to));
        }
    }
    Ok(out)
}

/// The turn ranges checkpoint restores undid (CP1), for the checkpoint list.
pub fn undone_turns(vault: &Path, key: &str) -> Result<Vec<(i64, i64)>, String> {
    with_read(vault, |conn| undone_ords(conn, key))
}

/// Each turn's run id by ordinal (CP1 maps checkpoints to the turns shown).
pub fn turn_run_ids(
    vault: &Path,
    key: &str,
) -> Result<std::collections::BTreeMap<i64, String>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT ord, run_id FROM turns WHERE session_key = ?1")
            .map_err(|e| format!("run ids prepare: {e}"))?;
        let rows = stmt
            .query_map(params![key], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| format!("run ids query: {e}"))?;
        Ok(rows
            .flatten()
            .filter_map(|(ord, run)| run.map(|r| (ord, r)))
            .collect())
    })
}

/// The agent a conversation runs on.
pub fn session_agent(vault: &Path, key: &str) -> Result<String, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT agent FROM sessions WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| format!("no conversation {key}: {e}"))
    })
}

/// The conversation before turn `before_ord`, oldest first, for the bounded
/// bundle a fresh native session receives (Paperclip PR 14b): per turn its
/// user text (`turns.asked`) then its assistant text (the transcript), both
/// with the transcript's agent. The same data `read_conversation` returns,
/// without its evidence receipts.
pub fn conversation_entries(
    vault: &Path,
    key: &str,
    before_ord: i64,
) -> Result<Vec<ContextEntry>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT ord, asked FROM turns WHERE session_key = ?1 AND ord < ?2 ORDER BY ord ASC",
            )
            .map_err(|e| format!("conversation prepare: {e}"))?;
        let turns = stmt
            .query_map(params![key, before_ord], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| format!("conversation query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("conversation rows: {e}"))?;
        let undone = undone_ords(conn, key)?;
        let mut out = Vec::with_capacity(turns.len() * 2);
        for (ord, asked) in turns {
            if undone.iter().any(|(from, to)| (*from..=*to).contains(&ord)) {
                continue;
            }
            let transcript = load_transcript(conn, key, ord)?;
            out.push(ContextEntry {
                role: "user".to_string(),
                agent: transcript.agent.clone(),
                text: asked.unwrap_or_default(),
                key: key.to_string(),
                turn: ord,
            });
            out.push(ContextEntry {
                role: "assistant".to_string(),
                agent: transcript.agent,
                text: transcript.text,
                key: key.to_string(),
                turn: ord,
            });
        }
        Ok(out)
    })
}

/// One turn as the task-state record folds it (`task_state.rs`): the
/// request, how the turn ended, its run, the reply and its tool calls. A
/// turn still marked `running` reads as it stands (an earlier turn of the
/// same conversation is over by the time a later one compiles, unless a
/// crash left it, and recovery closes those).
#[derive(Debug, Clone, Default)]
pub struct TaskTurn {
    pub ord: i64,
    /// Present only for a turn admitted to automatic capture: cloud provider
    /// or private model entry. The CLI agent alone cannot prove this scope.
    pub capture_scope: Option<String>,
    pub asked: String,
    /// `agent` when another agent sent this turn through `hollow_app`;
    /// `None` when the user did (or the turn predates the column).
    pub asked_by: Option<String>,
    /// The constraints the turn's leading task packet carried (RF-797-F3),
    /// each text with its `from` (`user`, `agent` or `parent`), as
    /// [`record_turn_packet`] stored them; empty for a turn with none.
    pub packet_constraints: Vec<(String, String)>,
    pub status: Option<String>,
    pub run_id: Option<String>,
    pub reply: String,
    pub tools: Vec<ConversationTool>,
    /// The step spine's `checkOutcome` of each shell call that held a check,
    /// by tool-use id: the receipt's outcome, read from the call's full
    /// result (the transcript keeps only its first 8,000 characters, and a
    /// runner prints its summary last; OF16 re-review).
    pub check_outcomes: std::collections::HashMap<String, String>,
    /// What the person said to the turn while it ran (mid-turn steering,
    /// 2026-10-21), oldest first: always the person's own words, even on a
    /// turn another agent asked (they steered it from its own composer).
    pub steers: Vec<String>,
    /// What the agent that started this chat said to the turn while it ran
    /// (the COO's `sessions.steer_session`, L13), oldest first: never the
    /// person's direction.
    pub agent_steers: Vec<String>,
}

/// The step spine's `checkOutcome` of each tool call of turn `ord` that has
/// one, by tool-use id.
fn check_outcomes(
    conn: &Connection,
    key: &str,
    ord: i64,
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT tool_id, checkpoint FROM steps WHERE session_key = ?1 AND turn_ord = ?2 \
             AND tool_id IS NOT NULL AND checkpoint LIKE '%\"checkOutcome\"%'",
        )
        .map_err(|e| format!("check outcomes prepare: {e}"))?;
    let rows = stmt
        .query_map(params![key, ord], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("check outcomes query: {e}"))?;
    let mut out = std::collections::HashMap::new();
    for row in rows {
        let (tool_id, checkpoint) = row.map_err(|e| format!("check outcome row: {e}"))?;
        let outcome = serde_json::from_str::<Value>(&checkpoint)
            .ok()
            .and_then(|c| {
                c.get("checkOutcome")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        if let Some(outcome) = outcome {
            out.insert(tool_id, outcome);
        }
    }
    Ok(out)
}

/// Turns `after_ord < ord < before_ord` of conversation `key`, oldest first.
pub fn task_turns(
    vault: &Path,
    key: &str,
    after_ord: i64,
    before_ord: i64,
) -> Result<Vec<TaskTurn>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT ord, asked, status, run_id, asked_by, packet_constraints, capture_scope FROM turns \
                 WHERE session_key = ?1 AND ord > ?2 AND ord < ?3 ORDER BY ord ASC",
            )
            .map_err(|e| format!("task turns prepare: {e}"))?;
        let rows = stmt
            .query_map(params![key, after_ord, before_ord], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(|e| format!("task turns query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("task turns rows: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for (ord, asked, status, run_id, asked_by, packet, capture_scope) in rows {
            let transcript = load_transcript(conn, key, ord)?;
            // An unreadable value reads as no packet.
            let packet_constraints = packet
                .and_then(|json| serde_json::from_str::<Vec<PacketConstraint>>(&json).ok())
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.text, c.from))
                .collect();
            out.push(TaskTurn {
                ord,
                capture_scope,
                asked: asked.unwrap_or_default(),
                asked_by,
                packet_constraints,
                status,
                run_id,
                reply: transcript.text,
                tools: transcript.tools,
                check_outcomes: check_outcomes(conn, key, ord)?,
                steers: transcript
                    .steers
                    .iter()
                    .filter(|s| s.by.as_deref() != Some("agent"))
                    .map(|s| s.text.clone())
                    .collect(),
                agent_steers: transcript
                    .steers
                    .into_iter()
                    .filter(|s| s.by.as_deref() == Some("agent"))
                    .map(|s| s.text)
                    .collect(),
            });
        }
        Ok(out)
    })
}

/// The answer (transcript text) of conversation `key`'s turn that ran as
/// `run_id`, the newest such turn; `None` when no turn of it ran as that
/// run. Read by `descendants::child_summary` (overnight OB17).
pub fn turn_reply_for_run(vault: &Path, key: &str, run_id: &str) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        let ord: Option<i64> = conn
            .query_row(
                "SELECT ord FROM turns WHERE session_key = ?1 AND run_id = ?2 \
                 ORDER BY ord DESC LIMIT 1",
                params![key, run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("turn for run: {e}"))?;
        match ord {
            Some(ord) => Ok(Some(load_transcript(conn, key, ord)?.text)),
            None => Ok(None),
        }
    })
}

/// The newest turn of each of `keys`: its ord, the person's words and the
/// stored answer (`None` while it runs, or for a turn stored before
/// transcripts were), in one read. The phone API's list preview
/// (`phone_access`).
pub fn newest_exchanges(
    vault: &Path,
    keys: &[String],
) -> Result<std::collections::HashMap<String, (u32, Option<String>, Option<String>)>, String> {
    let mut out = std::collections::HashMap::new();
    if keys.is_empty() {
        return Ok(out);
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT t.ord, t.asked, (SELECT CASE WHEN json_valid(x.payload) \
                 THEN json_extract(x.payload, '$.text') END FROM transcripts x \
                 WHERE x.session_key = t.session_key AND x.turn_ord = t.ord) \
                 FROM turns t WHERE t.session_key = ?1 ORDER BY t.ord DESC LIMIT 1",
            )
            .map_err(|e| format!("newest exchange: {e}"))?;
        for key in keys {
            let row = stmt
                .query_row(params![key], |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u32,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .optional()
                .map_err(|e| format!("newest exchange: {e}"))?;
            if let Some(row) = row {
                out.insert(key.clone(), row);
            }
        }
        Ok(())
    })?;
    Ok(out)
}

/// One conversation as [`list_digest`] reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct DigestRow {
    pub key: String,
    pub title: Option<String>,
    pub cwd: String,
    pub last_activity_at: String,
    pub turn_count: u32,
}

/// The newest `limit` unarchived conversations, newest first, with only
/// their title, folder, last activity and turn count, in one cheap read (no
/// stints): what the phone API compares to tell a phone a list row changed,
/// and how it orders workspaces by their newest conversation
/// (`phone_access`).
pub fn list_digest(vault: &Path, limit: usize) -> Result<Vec<DigestRow>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT key, title, cwd, last_activity_at, turn_count FROM sessions \
                 WHERE archived = 0 ORDER BY last_activity_at DESC, key ASC LIMIT ?1",
            )
            .map_err(|e| format!("session digest: {e}"))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(DigestRow {
                    key: row.get(0)?,
                    title: row.get(1)?,
                    cwd: row.get(2)?,
                    last_activity_at: row.get(3)?,
                    turn_count: row.get::<_, i64>(4)? as u32,
                })
            })
            .map_err(|e| format!("session digest: {e}"))?;
        Ok(rows.filter_map(Result::ok).collect())
    })
}

/// The run id conversation `key`'s latest stored turn ran as; `None` with
/// no turn (or one recorded without a run). Read by
/// `descendants::is_ended_child` (overnight OB21): the newest run-queue row
/// is the latest turn only when their runs match.
pub fn latest_turn_run(vault: &Path, key: &str) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT run_id FROM turns WHERE session_key = ?1 ORDER BY ord DESC LIMIT 1",
            params![key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|e| format!("latest turn run: {e}"))
    })
}

/// The run id of conversation `key`'s latest turn (by `ord`) that ran as some
/// run other than `except`; `None` when there is none. NV7: an agent's eval
/// candidate defaults to the turn before its running one.
pub(crate) fn latest_turn_run_except(
    vault: &Path,
    key: &str,
    except: &str,
) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT run_id FROM turns WHERE session_key = ?1 AND run_id IS NOT NULL \
             AND run_id != ?2 ORDER BY ord DESC LIMIT 1",
            params![key, except],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("latest turn run: {e}"))
    })
}

/// One `derived_state` row: what kind of state, its revision, the last turn
/// it has folded, and its JSON payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedRow {
    pub revision: i64,
    pub through_ord: i64,
    pub payload: String,
}

/// Conversation `key`'s derived state of `kind`, if any. An unsafe key reads
/// as none.
pub fn derived_state(vault: &Path, key: &str, kind: &str) -> Result<Option<DerivedRow>, String> {
    if !is_safe_session_key(key) {
        return Ok(None);
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT revision, through_ord, payload FROM derived_state \
             WHERE session_key = ?1 AND kind = ?2",
            params![key, kind],
            |row| {
                Ok(DerivedRow {
                    revision: row.get(0)?,
                    through_ord: row.get(1)?,
                    payload: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("derived state read: {e}"))
    })
}

/// Days a delegation row is kept (agent effectiveness F10, D1): as long
/// as anything reads a run's manifest view, far longer than a receipt.
const DELEGATIONS_KEPT_DAYS: i64 = 90;

/// Record one delegation (`delegation::store`) the run `run_id` made: its
/// manifest record as JSON. At most `per_run` rows per run (`Ok(false)`
/// past that, nothing written); rows older than 90 days go at each write.
/// Written by the GUI (a child task) and by `hollow team-mcp` (a
/// consultation), never into the vault. Tests only since OF04: production
/// writes go through [`insert_delegation`].
#[cfg(test)]
pub fn record_delegation(
    vault: &Path,
    run_id: &str,
    at: &str,
    record_json: &str,
    per_run: usize,
) -> Result<bool, String> {
    insert_delegation(vault, run_id, at, record_json, per_run).map(|row| row.is_some())
}

/// Record one delegation, returning the row written (`None`: past the
/// per-run cap, or no run), so a consultation can finish its own record
/// ([`finish_delegation`]).
pub fn insert_delegation(
    vault: &Path,
    run_id: &str,
    at: &str,
    record_json: &str,
    per_run: usize,
) -> Result<Option<i64>, String> {
    if run_id.trim().is_empty() {
        return Ok(None);
    }
    let cutoff = db_time(Utc::now() - Duration::days(DELEGATIONS_KEPT_DAYS));
    with_write(vault, "delegation record", |tx| {
        tx.execute("DELETE FROM delegations WHERE at < ?1", params![cutoff])
            .map_err(|e| format!("delegation prune: {e}"))?;
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM delegations WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("delegation count: {e}"))?;
        if count >= per_run as i64 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO delegations(run_id, at, record_json) VALUES(?1, ?2, ?3)",
            params![run_id, at, record_json],
        )
        .map_err(|e| format!("delegation insert: {e}"))?;
        Ok(Some(tx.last_insert_rowid()))
    })
}

/// Replace delegation row `row`'s record (OF04: a consultation's outcome,
/// written when it ends). A row pruned meanwhile is left gone.
pub fn finish_delegation(vault: &Path, row: i64, record_json: &str) -> Result<(), String> {
    with_write(vault, "delegation finish", |tx| {
        tx.execute(
            "UPDATE delegations SET record_json = ?2 WHERE id = ?1",
            params![row, record_json],
        )
        .map_err(|e| format!("delegation finish: {e}"))?;
        Ok(())
    })
}

/// The delegation records (JSON) run `run_id` made, oldest first.
pub fn delegations_for_run(vault: &Path, run_id: &str) -> Result<Vec<String>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT record_json FROM delegations WHERE run_id = ?1 ORDER BY id ASC")
            .map_err(|e| format!("delegations prepare: {e}"))?;
        let rows = stmt
            .query_map(params![run_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("delegations query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("delegations rows: {e}"))?;
        Ok(rows)
    })
}

/// The delegation records (JSON) the runs of conversation `key` made in
/// its turns before `before_ord`, newest first, at most `limit` (F10, D2:
/// the durable child tasks a conversation started, for its completion
/// notes).
pub fn delegations_for_session(
    vault: &Path,
    key: &str,
    before_ord: i64,
    limit: usize,
) -> Result<Vec<String>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT d.record_json FROM delegations d \
                 JOIN turns t ON t.run_id = d.run_id \
                 WHERE t.session_key = ?1 AND t.ord < ?2 \
                 ORDER BY d.id DESC LIMIT ?3",
            )
            .map_err(|e| format!("delegations for a conversation: {e}"))?;
        let rows = stmt
            .query_map(params![key, before_ord, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("delegations for a conversation: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("delegations for a conversation: {e}"))?;
        Ok(rows)
    })
}

/// The child-task delegation whose turn was run-queue row `queue_id`, with
/// the delegating run's id (morning M1: a woken turn joins the delegation
/// of the turn that asked for the wake). The newest, if several name it.
pub fn delegation_for_child_turn(
    vault: &Path,
    queue_id: i64,
) -> Result<Option<(String, String)>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT run_id, record_json FROM delegations \
             WHERE json_extract(record_json, '$.kind') = 'childTask' \
             AND json_extract(record_json, '$.childQueueId') = ?1 \
             ORDER BY id DESC LIMIT 1",
            params![queue_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| format!("delegation for a child turn: {e}"))
    })
}

/// Which conversation started each of `keys` (overnight F2, the rail's
/// chains): the newest child-task delegation that *started* it
/// (`start_session` or `handover`; a `send_to_session` only messages a chat
/// that exists, so it starts nothing), by the conversation whose turn made
/// it. A key no conversation started, or only itself, is absent. Delegation
/// records are kept [`DELEGATIONS_KEPT_DAYS`] days.
pub fn session_starters(
    vault: &Path,
    keys: &[String],
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut out = std::collections::HashMap::new();
    if keys.is_empty() {
        return Ok(out);
    }
    let marks = vec!["?"; keys.len()].join(",");
    let sql = format!(
        "SELECT t.session_key, json_extract(d.record_json, '$.childSession') FROM delegations d \
         JOIN turns t ON t.run_id = d.run_id \
         WHERE json_extract(d.record_json, '$.kind') = 'childTask' \
         AND json_extract(d.record_json, '$.tool') IN ('start_session', 'handover') \
         AND json_extract(d.record_json, '$.childSession') IN ({marks}) \
         ORDER BY d.id DESC"
    );
    let rows = with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("session starters: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(keys.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("session starters: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("session starters: {e}"))?;
        Ok(rows)
    })?;
    for (parent, child) in rows {
        if child != parent {
            out.entry(child).or_insert(parent);
        }
    }
    Ok(out)
}

/// The delegations made by turns of conversations running in folder `cwd`
/// since `since`, newest first, each with the asking conversation's key
/// (main agent M2: the chats the Main agent started).
pub fn delegations_for_cwd(
    vault: &Path,
    cwd: &str,
    since: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<(String, String)>, String> {
    Ok(delegation_runs_for_cwd(vault, cwd, since, limit)?
        .into_iter()
        .map(|(key, _, json)| (key, json))
        .collect())
}

/// [`delegations_for_cwd`] with the asking turn's run id too (overnight
/// 10-11 A1: whether the person or Hollow asked for the chat).
pub fn delegation_runs_for_cwd(
    vault: &Path,
    cwd: &str,
    since: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<(String, String, String)>, String> {
    let since = db_time(since);
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT t.session_key, d.run_id, d.record_json FROM delegations d \
                 JOIN turns t ON t.run_id = d.run_id \
                 JOIN sessions s ON s.key = t.session_key \
                 WHERE s.cwd = ?1 AND d.at >= ?2 \
                 AND d.record_json LIKE '%\"kind\":\"childTask\"%' \
                 ORDER BY d.id DESC LIMIT ?3",
            )
            .map_err(|e| format!("delegations for a folder: {e}"))?;
        let rows = stmt
            .query_map(params![cwd, since, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("delegations for a folder: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("delegations for a folder: {e}"))?;
        Ok(rows)
    })
}

/// What the turns of conversations in folder `cwd` that started at or after
/// `since` cost, with their consultations and helpers, split by who asked
/// (overnight 10-11 A1): `(hollow, person)`, where Hollow's turns are those
/// whose message starts with `hollow_prefix` (the Main agent's wakes and
/// check-ins). Unknown costs add nothing. Runs already charged to a durable
/// child chain are excluded, retaining their root attribution.
pub fn cost_for_cwd_since_split(
    vault: &Path,
    cwd: &str,
    since: DateTime<Utc>,
    hollow_prefix: &str,
    excluded_runs: &std::collections::HashSet<String>,
) -> Result<(f64, f64), String> {
    let since = db_time(since);
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT r.run_id, r.hollow, COALESCE(SUM(u.cost_usd), 0) \
             FROM usage_ledger u JOIN ( \
                 SELECT t.run_id AS run_id, \
                        MAX(substr(COALESCE(t.asked, ''), 1, length(?3)) = ?3) AS hollow \
                 FROM turns t JOIN sessions s ON s.key = t.session_key \
                 WHERE s.cwd = ?1 AND t.started_at >= ?2 AND t.run_id IS NOT NULL \
                 GROUP BY t.run_id) r ON r.run_id = u.run_id GROUP BY r.run_id, r.hollow",
            )
            .map_err(|e| format!("cost for a folder: {e}"))?;
        let rows = stmt
            .query_map(params![cwd, since, hollow_prefix], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, bool>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })
            .map_err(|e| format!("cost for a folder: {e}"))?;
        let mut totals = (0.0, 0.0);
        for row in rows {
            let (run, hollow, cost) = row.map_err(|e| format!("cost for a folder: {e}"))?;
            if !excluded_runs.contains(&run) {
                if hollow {
                    totals.0 += cost;
                } else {
                    totals.1 += cost;
                }
            }
        }
        Ok(totals)
    })
}

/// The asked text of the turn run `run_id` ran as, with its conversation's
/// folder (main agent M4: is this a wake or check-in turn).
pub fn asked_for_run(vault: &Path, run_id: &str) -> Result<Option<(String, String)>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT COALESCE(t.asked, ''), s.cwd FROM turns t JOIN sessions s ON s.key = t.session_key \
             WHERE t.run_id = ?1 ORDER BY t.id DESC LIMIT 1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("turn for a run: {e}"))
    })
}

/// The newest unarchived conversation in folder `cwd`: its key, when it was
/// started and its agent (main agent M2: today's Main agent thread).
pub fn newest_session_in(
    vault: &Path,
    cwd: &str,
) -> Result<Option<(String, String, String)>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT key, created_at, agent FROM sessions \
             WHERE cwd = ?1 AND archived = 0 ORDER BY created_at DESC LIMIT 1",
            params![cwd],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("newest conversation in a folder: {e}"))
    })
}

/// The unarchived conversations in folder `cwd`, newest first, at most
/// `limit` (one Main agent conversation, 2026-11-07: its earlier chats).
pub fn chain_sessions_in(
    vault: &Path,
    cwd: &str,
    limit: usize,
) -> Result<Vec<crate::chat_chain::ChainLink>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT key, title, created_at FROM sessions \
                 WHERE cwd = ?1 AND archived = 0 ORDER BY created_at DESC LIMIT ?2",
            )
            .map_err(|e| format!("conversations in a folder: {e}"))?;
        let rows = stmt
            .query_map(params![cwd, limit as i64], |row| {
                Ok(crate::chat_chain::ChainLink {
                    key: row.get(0)?,
                    title: row
                        .get::<_, Option<String>>(1)?
                        .filter(|t| !t.trim().is_empty()),
                    created_at: row.get(2)?,
                })
            })
            .map_err(|e| format!("conversations in a folder: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("conversations in a folder: {e}"))?;
        Ok(rows)
    })
}

/// The child-task delegations (JSON) made since `since` by the runs
/// `run_ids` (each with its run id), the newest `limit`, oldest first
/// (overnight C1: one query per level of a chain walk, following only the
/// turns the chain itself caused).
pub fn child_delegations_for_runs(
    vault: &Path,
    run_ids: &[String],
    since: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<(String, String)>, String> {
    if run_ids.is_empty() {
        return Ok(Vec::new());
    }
    let since = db_time(since);
    let marks = vec!["?"; run_ids.len()].join(",");
    let sql = format!(
        "SELECT run_id, record_json, at FROM delegations \
         WHERE run_id IN ({marks}) AND at >= ? \
         AND record_json LIKE '%\"kind\":\"childTask\"%' \
         ORDER BY id DESC LIMIT ?"
    );
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("delegations for runs: {e}"))?;
        let mut args: Vec<rusqlite::types::Value> = run_ids
            .iter()
            .map(|k| rusqlite::types::Value::Text(k.clone()))
            .collect();
        args.push(rusqlite::types::Value::Text(since));
        args.push(rusqlite::types::Value::Integer(limit as i64));
        let mut rows = stmt
            .query_map(rusqlite::params_from_iter(args), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("delegations for runs: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("delegations for runs: {e}"))?;
        rows.reverse();
        Ok(rows)
    })
}

/// The child-task delegations (JSON) the runs of conversation `key` made
/// since `since`, oldest first (overnight C1: the walk's seeds, without the
/// note's newest-32 window that consultations also fill).
pub fn child_delegations_for_session(
    vault: &Path,
    key: &str,
    since: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<String>, String> {
    let since = db_time(since);
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT d.record_json FROM delegations d \
                 JOIN turns t ON t.run_id = d.run_id \
                 WHERE t.session_key = ?1 AND d.at >= ?2 \
                 AND d.record_json LIKE '%\"kind\":\"childTask\"%' \
                 ORDER BY d.id DESC LIMIT ?3",
            )
            .map_err(|e| format!("child delegations for a conversation: {e}"))?;
        let mut rows = stmt
            .query_map(params![key, since, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("child delegations for a conversation: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("child delegations for a conversation: {e}"))?;
        rows.reverse();
        Ok(rows)
    })
}

/// A conversation's working folder, if it is recorded (one column; the
/// chain walk asks it every turn).
pub fn session_cwd(vault: &Path, key: &str) -> Option<String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT cwd FROM sessions WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
}

/// Which of the finished turns `queue_ids` were already reported to the
/// chain's root `root_key` (overnight C1), each with the conversation and
/// turn that reported it. Read only.
pub fn root_deliveries(
    vault: &Path,
    root_key: &str,
    queue_ids: &[i64],
) -> Result<std::collections::HashMap<i64, (String, i64)>, String> {
    let mut out = std::collections::HashMap::new();
    if queue_ids.is_empty() {
        return Ok(out);
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT queue_id, asker, turn_ord FROM root_deliveries WHERE root_key = ?1")
            .map_err(|e| format!("root deliveries: {e}"))?;
        let wanted: std::collections::HashSet<i64> = queue_ids.iter().copied().collect();
        let rows = stmt
            .query_map(params![root_key], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })
            .map_err(|e| format!("root deliveries: {e}"))?;
        for (id, asker, ord) in rows.flatten() {
            if wanted.contains(&id) {
                out.insert(id, (asker, ord.unwrap_or(-1)));
            }
        }
        Ok(out)
    })
}

/// Record that the finished turns `queue_ids` further down a chain were
/// reported to its root `root_key` by conversation `asker`'s turn
/// `turn_ord`, in one write (and prune old records once). Ids already
/// recorded keep their first report.
pub fn deliver_to_root(
    vault: &Path,
    root_key: &str,
    asker: &str,
    turn_ord: i64,
    queue_ids: &[i64],
) -> Result<(), String> {
    if queue_ids.is_empty() {
        return Ok(());
    }
    let now = Utc::now();
    // Nothing older than the chain window can be reported again.
    let cutoff = db_time(now - Duration::days(8));
    with_write(vault, "root delivery", |tx| {
        tx.execute("DELETE FROM root_deliveries WHERE at < ?1", params![cutoff])
            .map_err(|e| format!("root delivery prune: {e}"))?;
        for id in queue_ids {
            tx.execute(
                "INSERT OR IGNORE INTO root_deliveries(queue_id, root_key, asker, turn_ord, at) VALUES(?1, ?2, ?3, ?4, ?5)",
                params![id, root_key, asker, turn_ord, db_time(now)],
            )
            .map_err(|e| format!("root delivery insert: {e}"))?;
        }
        Ok(())
    })
}

/// How a child task's finished turn (its run-queue row `queue_id`) was
/// first reported to the conversation that started it (F10, D2): `via`
/// `wait` (`wait_for_session` returned it) or `note` (a
/// `<hollow-child-finished>` note in that conversation's turn `turn_ord`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildDelivery {
    pub via: String,
    pub turn_ord: Option<i64>,
    pub at: String,
}

/// Record that the finished child turn `queue_id` was reported, once:
/// `Ok(None)` when this is the first report, else the earlier one
/// (unchanged). Rows older than the delegations they belong to go with
/// them (90 days).
pub fn deliver_child(
    vault: &Path,
    queue_id: i64,
    parent_key: &str,
    child_session: &str,
    via: &str,
    turn_ord: Option<i64>,
) -> Result<Option<ChildDelivery>, String> {
    let now = Utc::now();
    let cutoff = db_time(now - Duration::days(DELEGATIONS_KEPT_DAYS));
    with_write(vault, "child delivery", |tx| {
        tx.execute(
            "DELETE FROM child_deliveries WHERE at < ?1",
            params![cutoff],
        )
        .map_err(|e| format!("child delivery prune: {e}"))?;
        let earlier = read_child_delivery(tx, queue_id)?;
        if earlier.is_some() {
            return Ok(earlier);
        }
        tx.execute(
            "INSERT INTO child_deliveries(queue_id, parent_key, child_session, via, turn_ord, at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                queue_id,
                parent_key,
                child_session,
                via,
                turn_ord,
                db_time(now)
            ],
        )
        .map_err(|e| format!("child delivery insert: {e}"))?;
        Ok(None)
    })
}

/// How the finished child turn `queue_id` was first reported, if it was.
pub fn child_delivery(vault: &Path, queue_id: i64) -> Result<Option<ChildDelivery>, String> {
    with_read(vault, |conn| read_child_delivery(conn, queue_id))
}

/// Forget the reports `via` recorded for conversation `parent_key` since
/// `since` (overnight OB10b: a woken turn whose send failed or never
/// settled): its child turns of `child_sessions`, and the chain turns
/// recorded with it as their root, so a later note or retry reports them.
/// Earlier reports, and other ways of reporting, stay.
pub fn undeliver(
    vault: &Path,
    parent_key: &str,
    child_sessions: &[String],
    via: &str,
    since: &str,
) -> Result<(), String> {
    with_write(vault, "child undelivery", |tx| {
        for child in child_sessions {
            tx.execute(
                "DELETE FROM child_deliveries \
                 WHERE parent_key = ?1 AND child_session = ?2 AND via = ?3 AND at >= ?4",
                params![parent_key, child, via, since],
            )
            .map_err(|e| format!("child undelivery: {e}"))?;
        }
        tx.execute(
            "DELETE FROM root_deliveries WHERE root_key = ?1 AND asker = ?2 AND at >= ?3",
            params![parent_key, via, since],
        )
        .map_err(|e| format!("root undelivery: {e}"))?;
        Ok(())
    })
}

fn read_child_delivery(conn: &Connection, queue_id: i64) -> Result<Option<ChildDelivery>, String> {
    conn.query_row(
        "SELECT via, turn_ord, at FROM child_deliveries WHERE queue_id = ?1",
        params![queue_id],
        |row| {
            Ok(ChildDelivery {
                via: row.get(0)?,
                turn_ord: row.get(1)?,
                at: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("child delivery read: {e}"))
}

/// Write conversation `key`'s derived state of `kind` (replacing it).
pub fn put_derived_state(
    vault: &Path,
    key: &str,
    kind: &str,
    row: &DerivedRow,
) -> Result<(), String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let _guard = write_lock();
    let conn = open(vault)?;
    conn.execute(
        "INSERT INTO derived_state(session_key, kind, revision, through_ord, payload, updated_at) \
         VALUES(?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(session_key, kind) DO UPDATE SET revision = ?3, through_ord = ?4, \
         payload = ?5, updated_at = ?6",
        params![
            key,
            kind,
            row.revision,
            row.through_ord,
            row.payload,
            Utc::now().to_rfc3339()
        ],
    )
    .map_err(|e| format!("derived state write: {e}"))?;
    Ok(())
}

/// Append one bounded trace event. Failures are the caller's to ignore: a
/// trace that cannot be written must never fail the user's chat turn.
pub fn append_event(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    kind: &str,
    payload: &str,
) -> Result<(), String> {
    if !is_safe_session_key(key) {
        return Ok(());
    }
    if turn_ord <= 0 {
        return Ok(());
    }
    let bounded = bounded_event(kind, payload);
    // Reads the transcript, then writes it: IMMEDIATE, so a team-mcp usage
    // row committed in between waits instead of failing this event.
    with_write(vault, "event", |conn| {
        let mut transcript = load_transcript(conn, key, turn_ord)?;
        apply_event(&mut transcript, kind, &bounded);
        transcript.incomplete |= bounded != payload;
        write_transcript(conn, key, turn_ord, &transcript)?;
        conn.execute(
            "INSERT INTO events(session_key, turn_ord, at, kind, payload) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![key, turn_ord, Utc::now().to_rfc3339(), kind, bounded],
        )
        .map_err(|e| format!("event append: {e}"))?;
        Ok(())
    })
}

/// Record the approved learning paths recalled into a turn (Paperclip PR 17)
/// as one `recall` event, which copies them into the turn's durable
/// transcript (so they survive event retention). A no-op for an empty list or
/// no turn row (`turn_ord <= 0`); at most [`MAX_RECALL_PATHS`] paths are kept
/// and over-long ones are dropped. Failures are the caller's to ignore.
pub fn record_recall(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    paths: &[String],
) -> Result<(), String> {
    let kept: Vec<&str> = paths
        .iter()
        .map(String::as_str)
        .filter(|path| !path.is_empty() && path.chars().count() <= RECALL_PATH_CHARS)
        .take(MAX_RECALL_PATHS)
        .collect();
    if kept.is_empty() || turn_ord <= 0 {
        return Ok(());
    }
    append_event(
        vault,
        key,
        turn_ord,
        "recall",
        &json!({ "paths": kept }).to_string(),
    )
}

/// Every run id recorded on the turns of these conversations, per key.
pub fn run_ids_for_sessions(
    vault: &Path,
    keys: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>, String> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    if keys.is_empty() {
        return Ok(out);
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT run_id FROM turns WHERE session_key = ?1 AND run_id IS NOT NULL")
            .map_err(|e| format!("session runs: {e}"))?;
        for key in keys {
            let rows = stmt
                .query_map(params![key], |row| row.get::<_, String>(0))
                .map_err(|e| format!("session runs: {e}"))?;
            out.insert(key.clone(), rows.filter_map(Result::ok).collect());
        }
        Ok(())
    })?;
    Ok(out)
}

/// The newest `per_session` turns of each of `keys`, newest first (a
/// workspace's Activity timeline, Canvas C5b). `asked` is cut to 240
/// characters: the timeline shows a line of it, never the message. Turn rows
/// are kept forever, so the cap is what bounds this read.
pub fn recent_turns_for_sessions(
    vault: &Path,
    keys: &[String],
    per_session: usize,
) -> Result<std::collections::HashMap<String, Vec<SessionRecordTurn>>, String> {
    let mut out = std::collections::HashMap::new();
    if keys.is_empty() || per_session == 0 {
        return Ok(out);
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT ord, run_id, started_at, ended_at, status, substr(asked, 1, 240) \
                 FROM turns WHERE session_key = ?1 ORDER BY ord DESC LIMIT ?2",
            )
            .map_err(|e| format!("recent turns: {e}"))?;
        for key in keys {
            let rows = stmt
                .query_map(params![key, per_session as i64], |row| {
                    Ok(SessionRecordTurn {
                        ord: row.get::<_, i64>(0)? as u32,
                        run_id: row.get(1)?,
                        started_at: row.get(2)?,
                        ended_at: row.get(3)?,
                        status: row.get(4)?,
                        asked: row.get(5)?,
                    })
                })
                .map_err(|e| format!("recent turns: {e}"))?;
            out.insert(key.clone(), rows.filter_map(Result::ok).collect());
        }
        Ok(())
    })?;
    Ok(out)
}

/// The usage rows of each of `run_ids` (a run's own agent, plus any
/// delegations that share its run id), oldest first. Rows older than
/// [`KEEP_USAGE_DAYS`] are pruned, so an old run may have none.
pub fn usage_rows_for_runs(
    vault: &Path,
    run_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<UsageRow>>, String> {
    let mut out: std::collections::HashMap<String, Vec<UsageRow>> =
        std::collections::HashMap::new();
    if run_ids.is_empty() {
        return Ok(out);
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {USAGE_COLUMNS} FROM usage_ledger WHERE run_id = ?1 ORDER BY at ASC, id ASC"
            ))
            .map_err(|e| format!("usage for runs: {e}"))?;
        for run in run_ids {
            let rows = stmt
                .query_map(params![run], usage_row)
                .map_err(|e| format!("usage for runs: {e}"))?
                .filter_map(Result::ok)
                .collect::<Vec<_>>();
            if !rows.is_empty() {
                out.insert(run.clone(), rows);
            }
        }
        Ok(())
    })?;
    Ok(out)
}

/// The usage rows of every run of conversations `keys`, by run id, oldest
/// first: one query where [`usage_rows_for_runs`] asks once per run (Canvas
/// C6 P3: a workspace can hold 1,000 conversations of 500 turns).
pub fn usage_rows_for_sessions(
    vault: &Path,
    keys: &[String],
) -> Result<std::collections::HashMap<String, Vec<UsageRow>>, String> {
    let mut out: std::collections::HashMap<String, Vec<UsageRow>> =
        std::collections::HashMap::new();
    // SQLite's bound-parameter limit is far above this; stay well under it.
    for chunk in keys.chunks(500) {
        let marks = vec!["?"; chunk.len()].join(", ");
        with_read(vault, |conn| {
            // Turn by conversation, then its run's rows by run id (both
            // indexed). The raw usage JSON and native session id are left
            // out: the timeline never shows them.
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT DISTINCT {SLIM_USAGE_COLUMNS}, u.id FROM turns t \
                     JOIN usage_ledger u ON u.run_id = t.run_id \
                     WHERE t.session_key IN ({marks}) ORDER BY u.at ASC, u.id ASC"
                ))
                .map_err(|e| format!("usage for conversations: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(chunk.iter()), usage_row)
                .map_err(|e| format!("usage for conversations: {e}"))?;
            for row in rows.filter_map(Result::ok) {
                if let Some(run) = row.run_id.clone() {
                    out.entry(run).or_default().push(row);
                }
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// How the usage ledger stands, to tell whether it changed since a reader
/// last looked (Canvas C6 P3): an insert raises `max_id`, an update (an
/// upsert, an interrupt) sets `at` to its own time, and a prune lowers
/// `count`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageStamp {
    pub count: i64,
    pub max_id: i64,
    pub max_at: Option<String>,
}

fn read_usage_stamp(conn: &Connection) -> Result<UsageStamp, String> {
    conn.query_row(
        "SELECT COUNT(*), COALESCE(MAX(id), 0), MAX(at) FROM usage_ledger",
        [],
        |row| {
            Ok(UsageStamp {
                count: row.get(0)?,
                max_id: row.get(1)?,
                max_at: row.get(2)?,
            })
        },
    )
    .map_err(|e| format!("usage stamp: {e}"))
}

/// The ledger's stamp now and, when `since` is given and differs, the rows
/// inserted or updated since it was taken (an id above its `max_id`, or an
/// `at` at or after its `max_at`), each with its row id, oldest first. One
/// read transaction, so the stamp and the rows are the same snapshot.
pub fn usage_changes_since(
    vault: &Path,
    since: Option<&UsageStamp>,
) -> Result<(UsageStamp, Vec<(i64, UsageRow)>), String> {
    with_read(vault, |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("usage since: {e}"))?;
        let stamp = read_usage_stamp(&tx)?;
        let Some(since) = since.filter(|s| **s != stamp) else {
            return Ok((stamp, Vec::new()));
        };
        let mut stmt = tx
            .prepare(&format!(
                "SELECT id, {USAGE_COLUMNS} FROM usage_ledger WHERE id > ?1 \
                 UNION SELECT id, {USAGE_COLUMNS} FROM usage_ledger WHERE at >= ?2 \
                 ORDER BY 3 ASC, 1 ASC"
            ))
            .map_err(|e| format!("usage since: {e}"))?;
        let rows = stmt
            .query_map(
                params![since.max_id, since.max_at.as_deref().unwrap_or("")],
                |row| {
                    let id: i64 = row.get(0)?;
                    // The row's own columns start after the id.
                    let shifted = UsageRow {
                        run_key: row.get(1)?,
                        at: row.get(2)?,
                        started_at: row.get(3)?,
                        surface: row.get(4)?,
                        scope: row.get(5)?,
                        parent_surface: row.get(6)?,
                        run_id: row.get(7)?,
                        agent: row.get(8)?,
                        requested_model: row.get(9)?,
                        model: row.get(10)?,
                        native_session: row.get(11)?,
                        billing: row.get(12)?,
                        outcome: row.get(13)?,
                        spawned: row.get::<_, i64>(14)? != 0,
                        basis: row.get(15)?,
                        input_tokens: row.get(16)?,
                        cached_input_tokens: row.get(17)?,
                        output_tokens: row.get(18)?,
                        cost_usd: row.get(19)?,
                        cost_status: row.get(20)?,
                        usage_json: row.get(21)?,
                        cost_floor_usd: row.get(22)?,
                        workspace_id: row.get(23)?,
                        account_ref: row.get(24)?,
                    };
                    Ok((id, shifted))
                },
            )
            .map_err(|e| format!("usage since: {e}"))?;
        let changes = rows.filter_map(Result::ok).collect();
        Ok((stamp, changes))
    })
}

/// Mark an admitted turn's capture privacy domain. Unmarked turns never
/// enter capture proof, including older turns whose model cannot be proved.
pub(crate) fn record_turn_capture_scope(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    scope: &str,
) -> Result<(), String> {
    if !is_safe_session_key(key)
        || turn_ord <= 0
        || scope.len() > 256
        || !(scope.starts_with("cloud:") || scope.starts_with("private:"))
    {
        return Err("Invalid learning capture scope.".into());
    }
    with_write(vault, "capture scope", |conn| {
        conn.execute("UPDATE turns SET capture_scope=?3 WHERE session_key=?1 AND ord=?2 AND capture_scope IS NULL",params![key,turn_ord,scope])
            .map(|_|()).map_err(|e|format!("capture scope: {e}"))
    })
}

/// Record the permission mode a turn runs with (best effort, like every
/// trace write).
pub fn record_turn_permission(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    mode: &str,
) -> Result<(), String> {
    if !is_safe_session_key(key) || turn_ord <= 0 {
        return Ok(());
    }
    with_write(vault, "turn permission", |conn| {
        conn.execute(
            "UPDATE turns SET permission_mode = ?3 WHERE session_key = ?1 AND ord = ?2",
            params![key, turn_ord, mode.chars().take(40).collect::<String>()],
        )
        .map(|_| ())
        .map_err(|e| format!("turn permission: {e}"))
    })
}

/// Record the reasoning effort turn `turn_ord` runs with (L1). Best
/// effort, like every trace write.
pub fn record_turn_effort(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    effort: &str,
) -> Result<(), String> {
    if !is_safe_session_key(key) || turn_ord <= 0 || effort.trim().is_empty() {
        return Ok(());
    }
    with_write(vault, "turn effort", |conn| {
        conn.execute(
            "UPDATE turns SET effort = ?3 WHERE session_key = ?1 AND ord = ?2",
            params![key, turn_ord, effort.chars().take(20).collect::<String>()],
        )
        .map(|_| ())
        .map_err(|e| format!("turn effort: {e}"))
    })
}

/// The effort conversation `key`'s newest turn that recorded one ran with.
pub fn last_effort(vault: &Path, key: &str) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT effort FROM turns WHERE session_key = ?1 AND effort IS NOT NULL \
             ORDER BY ord DESC LIMIT 1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("last effort: {e}"))
    })
}

/// Conversation `key`'s own reasoning effort (the composer's Effort list),
/// when the person chose one: a level of `effort::LEVELS`, else `None` (the
/// app default). A stored value that is no longer a level reads as none.
pub fn chat_effort(vault: &Path, key: &str) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Ok(None);
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT effort FROM chat_efforts WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|level| level.filter(|l| crate::effort::LEVELS.contains(&l.as_str())))
        .map_err(|e| format!("chat effort: {e}"))
    })
}

/// Every chat's own effort, for the window's cache (one small read at
/// start): `(session key, level)`.
pub fn chat_efforts(vault: &Path) -> Result<Vec<(String, String)>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT key, effort FROM chat_efforts ORDER BY key")
            .map_err(|e| format!("chat efforts: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("chat efforts: {e}"))?;
        let all = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("chat efforts: {e}"))?;
        Ok(all
            .into_iter()
            .filter(|(_, level)| crate::effort::LEVELS.contains(&level.as_str()))
            .collect())
    })
}

/// Set (`Some(level)`) or clear (`None`: Default, or a reset) conversation
/// `key`'s own effort. The level is checked; an unknown one is refused.
pub fn set_chat_effort(vault: &Path, key: &str, effort: Option<&str>) -> Result<(), String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let level = match effort.map(str::trim) {
        None | Some("") => None,
        Some(l) if crate::effort::LEVELS.contains(&l) => Some(l),
        Some(_) => return Err("Unknown reasoning effort.".into()),
    };
    with_write(vault, "chat effort", |conn| match level {
        Some(level) => conn
            .execute(
                "INSERT INTO chat_efforts(key, effort, updated_at) VALUES(?1, ?2, ?3) \
                 ON CONFLICT(key) DO UPDATE SET effort = excluded.effort, \
                 updated_at = excluded.updated_at",
                params![key, level, db_time(Utc::now())],
            )
            .map(|_| ())
            .map_err(|e| format!("chat effort: {e}")),
        None => conn
            .execute("DELETE FROM chat_efforts WHERE key = ?1", params![key])
            .map(|_| ())
            .map_err(|e| format!("chat effort: {e}")),
    })
}

/// Fill in chats' own efforts the store doesn't have yet (the one-time move
/// of the webview's old localStorage values): an entry for a key that already
/// has one, an unsafe key or an unknown level is skipped, never overwritten.
/// Returns how many were kept.
pub fn import_chat_efforts(vault: &Path, entries: &[(String, String)]) -> Result<usize, String> {
    with_write(vault, "chat effort import", |conn| {
        let mut kept = 0;
        for (key, level) in entries {
            if !is_safe_session_key(key) || !crate::effort::LEVELS.contains(&level.as_str()) {
                continue;
            }
            kept += conn
                .execute(
                    "INSERT OR IGNORE INTO chat_efforts(key, effort, updated_at) VALUES(?1, ?2, ?3)",
                    params![key, level, db_time(Utc::now())],
                )
                .map_err(|e| format!("chat effort import: {e}"))?;
        }
        Ok(kept)
    })
}

/// A handover successor starts with its predecessor's effort: copy `from`'s
/// level to `to` when `to` has none of its own. Best effort.
pub fn inherit_chat_effort(vault: &Path, from: &str, to: &str) -> Result<(), String> {
    if !is_safe_session_key(from) || !is_safe_session_key(to) {
        return Ok(());
    }
    with_write(vault, "chat effort inherit", |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO chat_efforts(key, effort, updated_at) \
             SELECT ?2, effort, ?3 FROM chat_efforts WHERE key = ?1",
            params![from, to, db_time(Utc::now())],
        )
        .map(|_| ())
        .map_err(|e| format!("chat effort inherit: {e}"))
    })
}

/// Record that an agent, not the user, asked turn `turn_ord` (a turn
/// another agent sent through `hollow_app`). Best effort, like every trace
/// write.
pub fn record_turn_asker(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    asker: &str,
) -> Result<(), String> {
    if !is_safe_session_key(key) || turn_ord <= 0 {
        return Ok(());
    }
    with_write(vault, "turn asker", |conn| {
        conn.execute(
            "UPDATE turns SET asked_by = ?3 WHERE session_key = ?1 AND ord = ?2",
            params![key, turn_ord, asker.chars().take(40).collect::<String>()],
        )
        .map(|_| ())
        .map_err(|e| format!("turn asker: {e}"))
    })
}

/// One constraint of a turn's task packet, as `turns.packet_constraints`
/// stores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PacketConstraint {
    text: String,
    from: String,
}

/// Record the constraints turn `turn_ord`'s task packet carried (RF-797-F3:
/// `delegation::record_child_packet`), each text with its `from`. Nothing
/// is written for none. Best effort, like every trace write.
pub fn record_turn_packet(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    constraints: &[(String, String)],
) -> Result<(), String> {
    if !is_safe_session_key(key) || turn_ord <= 0 || constraints.is_empty() {
        return Ok(());
    }
    let rows: Vec<PacketConstraint> = constraints
        .iter()
        .map(|(text, from)| PacketConstraint {
            text: text.clone(),
            from: from.clone(),
        })
        .collect();
    let json = serde_json::to_string(&rows).map_err(|e| format!("turn packet encode: {e}"))?;
    with_write(vault, "turn packet", |conn| {
        conn.execute(
            "UPDATE turns SET packet_constraints = ?3 WHERE session_key = ?1 AND ord = ?2",
            params![key, turn_ord, json],
        )
        .map(|_| ())
        .map_err(|e| format!("turn packet: {e}"))
    })
}

/// Record the attachments of turn `turn_ord` as the JSON text
/// `phone_access::attachments` built (best effort, like every trace write).
pub fn record_turn_attachments(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    json: &str,
) -> Result<(), String> {
    if !is_safe_session_key(key) || turn_ord <= 0 || json.trim().is_empty() {
        return Ok(());
    }
    with_write(vault, "turn attachments", |conn| {
        conn.execute(
            "UPDATE turns SET attachments = ?3 WHERE session_key = ?1 AND ord = ?2",
            params![key, turn_ord, json],
        )
        .map(|_| ())
        .map_err(|e| format!("turn attachments: {e}"))
    })
}

/// Every turn of conversation `key` that recorded attachments: its ordinal
/// and the JSON text [`record_turn_attachments`] stored, oldest first.
pub fn turn_attachments(vault: &Path, key: &str) -> Result<Vec<(i64, String)>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT ord, attachments FROM turns \
                 WHERE session_key = ?1 AND attachments IS NOT NULL ORDER BY ord ASC",
            )
            .map_err(|e| format!("turn attachments prepare: {e}"))?;
        let rows = stmt
            .query_map(params![key], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("turn attachments query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("turn attachments rows: {e}"))?;
        Ok(rows)
    })
}

/// The permission mode of a conversation's latest turn that recorded one,
/// or `None` (no turns yet, or only turns from before it was recorded).
pub fn last_permission(vault: &Path, key: &str) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT permission_mode FROM turns WHERE session_key = ?1 AND permission_mode IS NOT NULL ORDER BY ord DESC LIMIT 1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("last permission: {e}"))
    })
}

/// The model conversation `key`'s newest chat turn asked for, with that
/// turn's agent: `Some((None, agent))` when it asked for the agent's default,
/// `None` with no priced turn yet (morning M2: a woken turn keeps the
/// conversation's current choice, and so its native session).
pub fn last_requested_model(
    vault: &Path,
    key: &str,
) -> Result<Option<(Option<String>, Option<String>)>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT u.requested_model, u.agent FROM usage_ledger u \
             JOIN turns t ON t.run_id = u.run_id \
             WHERE t.session_key = ?1 AND u.surface = 'chat' \
             ORDER BY t.ord DESC, u.id DESC LIMIT 1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("last requested model: {e}"))
    })
}

/// The model conversation `key`'s FIRST chat turn asked for, `None` when it
/// asked for the agent's default or has no priced turn. Regenerate title reads
/// it: the title is made from that turn's words, so a self-hosted first turn
/// keeps them on its own entry.
pub fn first_requested_model(vault: &Path, key: &str) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT u.requested_model FROM usage_ledger u \
             JOIN turns t ON t.run_id = u.run_id \
             WHERE t.session_key = ?1 AND u.surface = 'chat' \
             ORDER BY t.ord ASC, u.id ASC LIMIT 1",
            params![key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|e| format!("first requested model: {e}"))
    })
}

/// Conversation `key`'s latest turn before `before_ord` that ran on native
/// session `harness_session_id` (OB04's version restamp): its recorded
/// permission mode and its chat usage row's model and agent, with the
/// conversation's folder. `None` with no such turn. Only a turn whose usage
/// row names that native session (`usage_ledger.native_session`: the id it
/// resumed or announced) counts, so a turn that failed before its resume
/// plan ran (its mode and model recorded, no reset made) never stands for
/// the stint. The running turn has already recorded its own mode and begun
/// its usage row, so [`last_permission`] and [`last_requested_model`] would
/// read it.
pub fn previous_turn(
    vault: &Path,
    key: &str,
    before_ord: i64,
    harness_session_id: &str,
) -> Result<Option<PreviousTurn>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT t.permission_mode, u.requested_model, u.agent, s.cwd FROM turns t \
             JOIN usage_ledger u ON u.run_id = t.run_id AND u.surface = 'chat' \
             AND u.native_session = ?3 \
             LEFT JOIN sessions s ON s.key = t.session_key \
             WHERE t.session_key = ?1 AND t.ord < ?2 \
             ORDER BY t.ord DESC, u.id DESC LIMIT 1",
            params![key, before_ord, harness_session_id],
            |row| {
                Ok(PreviousTurn {
                    permission_mode: row.get(0)?,
                    requested: Some((row.get(1)?, row.get(2)?)),
                    session_cwd: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("previous turn: {e}"))
    })
}

/// Overnight 9 A9: when conversation `key`'s newest ended turn before
/// `before_ord` ended (`turns.ended_at`, RFC 3339), or `None` with none.
pub fn last_turn_ended(vault: &Path, key: &str, before_ord: i64) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT ended_at FROM turns WHERE session_key = ?1 AND ord < ?2 \
             AND ended_at IS NOT NULL ORDER BY ord DESC LIMIT 1",
            params![key, before_ord],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("last turn ended: {e}"))
    })
}

/// The permission mode recorded for the turn run `run_id` in conversation
/// `key`, or `None` when that turn recorded none (merged plan slice 4: a
/// Result's acceptance names the mode of the run that wrote it).
pub fn turn_permission_for_run(
    vault: &Path,
    key: &str,
    run_id: &str,
) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT permission_mode FROM turns WHERE session_key = ?1 AND run_id = ?2 AND permission_mode IS NOT NULL ORDER BY ord DESC LIMIT 1",
            params![key, run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("turn permission for run: {e}"))
    })
}

/// Close a turn and run retention. `status` mirrors the run status the
/// evidence receipt records (`success` / `error` / `cancelled`).
pub fn finish_turn(vault: &Path, key: &str, turn_ord: i64, status: &str) -> Result<(), String> {
    if !is_safe_session_key(key) {
        return Ok(());
    }
    let now = Utc::now().to_rfc3339();
    {
        let _guard = write_lock();
        let conn = open(vault)?;
        conn.execute(
            "UPDATE turns SET ended_at = ?3, status = ?4 WHERE session_key = ?1 AND ord = ?2",
            params![key, turn_ord, now, status],
        )
        .map_err(|e| format!("turn finish: {e}"))?;
        conn.execute(
            "UPDATE sessions SET last_activity_at = ?2 WHERE key = ?1",
            params![key, now],
        )
        .map_err(|e| format!("session touch: {e}"))?;
    }
    retain(vault)
}

/// What [`interrupt_chat_run_in`] closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InterruptedRun {
    /// Session-store turns closed as `interrupted`.
    pub turns: usize,
    /// Usage rows closed as `interrupted` (the run's own and its delegates').
    pub usage: usize,
}

/// The terminal funnel's counterpart for a chat run no live turn will ever
/// finish (merged plan slice 3, B·F10): the run queue calls it, inside the
/// transaction that ends the row `interrupted`, for a claimed or running chat
/// row a previous process left behind (startup recovery) or whose in-memory
/// turn was lost (`run_queue::enqueue_chat`'s repair). In that one
/// transaction it closes the run's still-`running` session-store turn and its
/// still-`running` usage rows (its own and its delegates', which carry its
/// run id) as `interrupted`, so the queue row, the transcript and the usage
/// ledger end together or not at all, and a failure is retried with the
/// recovery that called it. Turns and rows already closed (the live funnel
/// ran, but the row's terminal write did not) are left alone. History's
/// manifest is a file, not a row: `automated_history::reconcile_interrupted`
/// still closes it at startup.
pub(crate) fn interrupt_chat_run_in(
    tx: &Transaction<'_>,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<InterruptedRun, String> {
    if run_id.trim().is_empty() {
        return Ok(InterruptedRun::default());
    }
    let turns = tx
        .execute(
            "UPDATE turns SET status = 'interrupted', ended_at = ?2 \
             WHERE run_id = ?1 AND status = 'running'",
            params![run_id, now.to_rfc3339()],
        )
        .map_err(|e| format!("interrupt turn: {e}"))?;
    let usage = tx
        .execute(
            &format!("{INTERRUPT_RUNNING} AND run_id = ?2"),
            params![db_time(now), run_id],
        )
        .map_err(|e| format!("usage interrupt: {e}"))?;
    Ok(InterruptedRun { turns, usage })
}

/// Drop raw events beyond the retention bounds. Session, stint, turn, and
/// bounded transcript rows survive so app history remains readable.
pub fn retain(vault: &Path) -> Result<(), String> {
    with_write(vault, "retention", retain_in)
}

fn retain_in(conn: &Transaction<'_>) -> Result<(), String> {
    // Preserve readable history before pruning the raw diagnostic trace.
    // This also migrates older sessions using whichever events still exist.
    let missing = {
        let mut statement = conn
            .prepare(
                "SELECT t.session_key, t.ord FROM turns t LEFT JOIN transcripts p \
             ON p.session_key = t.session_key AND p.turn_ord = t.ord WHERE p.session_key IS NULL",
            )
            .map_err(|e| format!("transcript migration prepare: {e}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|e| format!("transcript migration query: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("transcript migration rows: {e}"))?
    };
    for (key, ord) in missing {
        let transcript = load_transcript(conn, &key, ord)?;
        write_transcript(conn, &key, ord, &transcript)?;
    }
    let cutoff = (Utc::now() - Duration::days(KEEP_EVENT_DAYS)).to_rfc3339();
    conn.execute("DELETE FROM events WHERE at < ?1", params![cutoff])
        .map_err(|e| format!("event retention by age: {e}"))?;
    conn.execute(
        "DELETE FROM events WHERE session_key NOT IN ( \
           SELECT key FROM sessions ORDER BY last_activity_at DESC LIMIT ?1)",
        params![KEEP_EVENT_SESSIONS as i64],
    )
    .map_err(|e| format!("event retention by count: {e}"))?;
    // Event retention never touches usage rows; they age out on their own
    // clock, and never while still running.
    prune_usage_in(conn, &db_time(Utc::now() - Duration::days(KEEP_USAGE_DAYS)))?;
    // Question cards (Paperclip PR 13): terminal rows age out; open ones never.
    crate::interactions::prune_in(
        conn,
        &db_time(Utc::now() - Duration::days(crate::interactions::KEEP_DAYS)),
    )?;
    Ok(())
}

// ── Usage ledger ────────────────────────────────────────────────────────────

/// One `usage_ledger` row (see `usage_ledger` for what each value means).
/// `at` and `started_at` are [`db_time`] strings.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub run_key: String,
    pub at: String,
    pub started_at: String,
    pub surface: String,
    pub scope: Option<String>,
    pub parent_surface: Option<String>,
    pub run_id: Option<String>,
    pub agent: Option<String>,
    pub requested_model: Option<String>,
    pub model: Option<String>,
    pub native_session: Option<String>,
    pub billing: String,
    pub outcome: String,
    pub spawned: bool,
    pub basis: String,
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub cost_status: String,
    pub usage_json: Option<String>,
    /// OB07: when `cost_usd` is NULL on a run with tokens (`partial`), the
    /// price table applied to what streamed (`usage_prices::floor_usd`): a
    /// lower bound, never added to a known total or a budget.
    pub cost_floor_usd: Option<f64>,
    /// Run ledger v2: the workspace the run belonged to, written after the
    /// row by `set_usage_attribution`. [`upsert_usage`] never writes it.
    pub workspace_id: Option<String>,
    /// Run ledger v2: the hashed Claude account label (`account_ref`), or
    /// `None` for a run on an agent's own login. Not written by
    /// [`upsert_usage`] either.
    pub account_ref: Option<String>,
}

/// The columns [`upsert_usage`] writes.
const USAGE_WRITE_COLUMNS: &str =
    "run_key, at, started_at, surface, scope, parent_surface, run_id, \
     agent, requested_model, model, native_session, billing, outcome, spawned, basis, \
     input_tokens, cached_input_tokens, output_tokens, cost_usd, cost_status, usage_json, \
     cost_floor_usd";

const USAGE_COLUMNS: &str = "run_key, at, started_at, surface, scope, parent_surface, run_id, \
     agent, requested_model, model, native_session, billing, outcome, spawned, basis, \
     input_tokens, cached_input_tokens, output_tokens, cost_usd, cost_status, usage_json, \
     cost_floor_usd, workspace_id, account_ref";

/// [`USAGE_COLUMNS`] of `usage_ledger u`, with `native_session` and
/// `usage_json` read as NULL.
const SLIM_USAGE_COLUMNS: &str = "u.run_key, u.at, u.started_at, u.surface, u.scope, \
     u.parent_surface, u.run_id, u.agent, u.requested_model, u.model, NULL, u.billing, \
     u.outcome, u.spawned, u.basis, u.input_tokens, u.cached_input_tokens, u.output_tokens, \
     u.cost_usd, u.cost_status, NULL, u.cost_floor_usd, u.workspace_id, u.account_ref";

fn usage_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRow> {
    Ok(UsageRow {
        run_key: row.get(0)?,
        at: row.get(1)?,
        started_at: row.get(2)?,
        surface: row.get(3)?,
        scope: row.get(4)?,
        parent_surface: row.get(5)?,
        run_id: row.get(6)?,
        agent: row.get(7)?,
        requested_model: row.get(8)?,
        model: row.get(9)?,
        native_session: row.get(10)?,
        billing: row.get(11)?,
        outcome: row.get(12)?,
        spawned: row.get::<_, i64>(13)? != 0,
        basis: row.get(14)?,
        input_tokens: row.get(15)?,
        cached_input_tokens: row.get(16)?,
        output_tokens: row.get(17)?,
        cost_usd: row.get(18)?,
        cost_status: row.get(19)?,
        usage_json: row.get(20)?,
        cost_floor_usd: row.get(21)?,
        workspace_id: row.get(22)?,
        account_ref: row.get(23)?,
    })
}

/// Insert or update one run's row, keyed by `run_key`, as a single
/// autocommit statement (safe against a second writer process without a
/// transaction). The identity columns (`started_at`, `surface`, `scope`,
/// `parent_surface`, `run_id`) keep their first values.
pub fn upsert_usage(vault: &Path, row: &UsageRow) -> Result<(), String> {
    let _guard = write_lock();
    let conn = open(vault)?;
    conn.execute(
        &format!(
            "INSERT INTO usage_ledger({USAGE_WRITE_COLUMNS}) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, \
             ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22) \
             ON CONFLICT(run_key) DO UPDATE SET at = excluded.at, agent = excluded.agent, \
             requested_model = excluded.requested_model, model = excluded.model, \
             native_session = excluded.native_session, billing = excluded.billing, \
             outcome = excluded.outcome, spawned = excluded.spawned, basis = excluded.basis, \
             input_tokens = excluded.input_tokens, \
             cached_input_tokens = excluded.cached_input_tokens, \
             output_tokens = excluded.output_tokens, cost_usd = excluded.cost_usd, \
             cost_status = excluded.cost_status, usage_json = excluded.usage_json, \
             cost_floor_usd = excluded.cost_floor_usd"
        ),
        params![
            row.run_key,
            row.at,
            row.started_at,
            row.surface,
            row.scope,
            row.parent_surface,
            row.run_id,
            row.agent,
            row.requested_model,
            row.model,
            row.native_session,
            row.billing,
            row.outcome,
            row.spawned as i64,
            row.basis,
            row.input_tokens,
            row.cached_input_tokens,
            row.output_tokens,
            row.cost_usd,
            row.cost_status,
            row.usage_json,
            row.cost_floor_usd,
        ],
    )
    .map_err(|e| format!("usage write: {e}"))?;
    Ok(())
}

/// Write a run's attribution (run ledger v2, `run_attribution`) onto its
/// row. A `None` keeps what the row has, so a later write never erases an
/// earlier one. A missing row is not an error (nothing to attribute).
pub fn set_usage_attribution(
    vault: &Path,
    run_key: &str,
    attribution: &crate::run_attribution::Attribution,
) -> Result<(), String> {
    let _guard = write_lock();
    let conn = open(vault)?;
    conn.execute(
        "UPDATE usage_ledger SET account_ref = COALESCE(?2, account_ref), \
         workspace_id = COALESCE(?3, workspace_id), department = COALESCE(?4, department), \
         work_kind = COALESCE(?5, work_kind) WHERE run_key = ?1",
        params![
            run_key,
            attribution.account_ref,
            attribution.workspace_id,
            attribution.department,
            attribution.work_kind,
        ],
    )
    .map_err(|e| format!("usage attribution write: {e}"))?;
    Ok(())
}

/// The `SET … WHERE outcome = 'running'` every interrupt below shares.
const INTERRUPT_RUNNING: &str =
    "UPDATE usage_ledger SET outcome = 'interrupted', at = ?1, cost_status = \
     CASE WHEN input_tokens IS NULL THEN 'unknown' ELSE 'partial' END \
     WHERE outcome = 'running'";

/// Close stray `running` rows as `interrupted`: partial when a checkpoint
/// recorded tokens, unknown otherwise. `delegate_run_id` narrows it to one
/// parent run's delegate rows. Without it every running row is closed except
/// those of `except_run` (Paperclip PR 18: the run another live Hollow
/// process holds the lease for, and its delegates, which carry its run id).
pub fn interrupt_running_usage(
    vault: &Path,
    delegate_run_id: Option<&str>,
    except_run: Option<&str>,
    at: &str,
) -> Result<usize, String> {
    let _guard = write_lock();
    let conn = open(vault)?;
    let changed = match (delegate_run_id, except_run) {
        (Some(run_id), _) => conn.execute(
            &format!("{INTERRUPT_RUNNING} AND surface = 'delegate' AND run_id = ?2"),
            params![at, run_id],
        ),
        (None, Some(spared)) => conn.execute(
            &format!("{INTERRUPT_RUNNING} AND (run_id IS NULL OR run_id != ?2)"),
            params![at, spared],
        ),
        (None, None) => conn.execute(INTERRUPT_RUNNING, params![at]),
    }
    .map_err(|e| format!("usage interrupt: {e}"))?;
    Ok(changed)
}

/// Close one run's `running` usage rows, its own and its delegates' (which
/// carry the parent's run id), as `interrupted` (Paperclip PR 18: a run whose
/// lease was reclaimed from a process that exited).
pub fn interrupt_running_usage_for_run(
    vault: &Path,
    run_id: &str,
    at: &str,
) -> Result<usize, String> {
    let _guard = write_lock();
    let conn = open(vault)?;
    conn.execute(
        &format!("{INTERRUPT_RUNNING} AND run_id = ?2"),
        params![at, run_id],
    )
    .map_err(|e| format!("usage interrupt: {e}"))
}

fn prune_usage_in(conn: &Connection, cutoff: &str) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM usage_ledger WHERE at < ?1 AND outcome != 'running'",
        params![cutoff],
    )
    .map_err(|e| format!("usage retention: {e}"))
}

/// Delete finished usage rows older than `cutoff` (a [`db_time`] string).
/// Running rows are never deleted.
pub fn prune_usage(vault: &Path, cutoff: &str) -> Result<usize, String> {
    let _guard = write_lock();
    let conn = open(vault)?;
    prune_usage_in(&conn, cutoff)
}

/// Every usage row with `at >= since`, oldest first.
pub fn usage_rows_since(vault: &Path, since: &str) -> Result<Vec<UsageRow>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {USAGE_COLUMNS} FROM usage_ledger WHERE at >= ?1 ORDER BY at ASC, id ASC"
            ))
            .map_err(|e| format!("usage query prepare: {e}"))?;
        let rows = stmt
            .query_map(params![since], usage_row)
            .map_err(|e| format!("usage query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("usage row: {e}"));
        rows
    })
}

/// Every usage row with `since <= at < until` (both [`db_time`] strings),
/// oldest first: one budget window (Paperclip PR 8a). A read on a fresh
/// connection with no lock, relying on WAL and the busy timeout.
pub fn usage_rows_between(vault: &Path, since: &str, until: &str) -> Result<Vec<UsageRow>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {USAGE_COLUMNS} FROM usage_ledger WHERE at >= ?1 AND at < ?2 \
                 ORDER BY at ASC, id ASC"
            ))
            .map_err(|e| format!("usage query prepare: {e}"))?;
        let rows = stmt
            .query_map(params![since, until], usage_row)
            .map_err(|e| format!("usage query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("usage row: {e}"));
        rows
    })
}

/// When recording began: the oldest row still kept.
pub fn usage_ledger_since(vault: &Path) -> Result<Option<String>, String> {
    with_read(vault, |conn| {
        conn.query_row("SELECT MIN(at) FROM usage_ledger", [], |row| {
            row.get::<_, Option<String>>(0)
        })
        .map_err(|e| format!("usage since: {e}"))
    })
}

/// A resumed Claude turn's baseline ([`previous_terminal`]).
#[derive(Debug, Clone, PartialEq)]
pub struct UsageBaseline {
    /// The baseline row's `usage_json`; its `terminal` is non-null.
    pub usage_json: String,
    /// The finished rows of the session after the baseline (a cancelled,
    /// killed or failed-to-spawn turn stores no terminal), oldest first: the
    /// runs a `session_delta_span` row's cost covers (OB24).
    pub skipped_runs: Vec<String>,
    /// Whether any of those rows already has a `cost_usd` (a turn guard's
    /// estimate): a span would count that spend a second time, so none is
    /// recorded.
    pub skipped_priced: bool,
}

/// The newest finished row for this agent's native session before `before`,
/// other than `exclude_key` (the row asking), that stored a terminal: the
/// baseline a resumed Claude turn subtracts. Until OB24 any row with a
/// `usage_json` qualified, so a cancelled turn (terminal null) left the next
/// resume with no baseline at all.
pub fn previous_terminal(
    vault: &Path,
    agent: &str,
    native_session: &str,
    before: &str,
    exclude_key: &str,
) -> Result<Option<UsageBaseline>, String> {
    with_read(vault, |conn| {
        // `json_valid` first: `json_extract` raises on malformed text, and
        // only CASE guarantees the order.
        let found: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT usage_json, at, id FROM usage_ledger WHERE agent = ?1 \
                 AND native_session = ?2 AND outcome != 'running' AND at < ?3 \
                 AND run_key != ?4 AND CASE WHEN json_valid(usage_json) \
                 THEN json_extract(usage_json, '$.terminal') END IS NOT NULL \
                 ORDER BY at DESC, id DESC LIMIT 1",
                params![agent, native_session, before, exclude_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| format!("usage baseline: {e}"))?;
        let Some((usage_json, at, id)) = found else {
            return Ok(None);
        };
        let mut stmt = conn
            .prepare(
                "SELECT run_key, COALESCE(cost_usd > 0, 0) FROM usage_ledger WHERE agent = ?1 \
                 AND native_session = ?2 AND outcome != 'running' AND at < ?3 \
                 AND run_key != ?4 AND (at > ?5 OR (at = ?5 AND id > ?6)) \
                 ORDER BY at ASC, id ASC",
            )
            .map_err(|e| format!("usage baseline: {e}"))?;
        let skipped = stmt
            .query_map(
                params![agent, native_session, before, exclude_key, at, id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .map_err(|e| format!("usage baseline: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("usage baseline: {e}"))?;
        let skipped_priced = skipped.iter().any(|(_, priced)| *priced);
        Ok(Some(UsageBaseline {
            usage_json,
            skipped_runs: skipped.into_iter().map(|(key, _)| key).collect(),
            skipped_priced,
        }))
    })
}

fn stints_for(conn: &Connection, key: &str) -> Vec<HarnessStint> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT agent, harness_session_id, started_at, ended_at, reason FROM stints \
         WHERE session_key = ?1 ORDER BY id ASC",
    ) {
        if let Ok(rows) = stmt.query_map(params![key], |row| {
            Ok(HarnessStint {
                agent: row.get(0)?,
                harness_session_id: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                reason: row.get(4)?,
            })
        }) {
            out = rows.filter_map(Result::ok).collect();
        }
    }
    out
}

fn summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSessionSummary> {
    Ok(AgentSessionSummary {
        key: row.get(0)?,
        title: row.get(1)?,
        agent: row.get(2)?,
        cwd: row.get(3)?,
        created_at: row.get(4)?,
        last_activity_at: row.get(5)?,
        ended_at: row.get(6)?,
        turn_count: row.get::<_, i64>(7)? as u32,
        parent_key: row.get(8)?,
        fork_kind: row.get(9)?,
        adopted: row.get::<_, i64>(10)? != 0,
        stints: Vec::new(),
        harness_session_id: None,
        resumable: "none".to_string(),
        imported: None,
    })
}

fn hydrate_summary(conn: &Connection, summary: &mut AgentSessionSummary) {
    summary.stints = stints_for(conn, &summary.key);
    summary.harness_session_id = summary
        .stints
        .last()
        .and_then(|s| s.harness_session_id.clone());
    summary.resumable = resumability(&summary.agent, summary.harness_session_id.as_deref());
    // H5a: an imported record says where it came from and is never resumed.
    if summary.adopted {
        summary.imported = crate::session_import::provenance(conn, &summary.key);
        if summary.imported.is_some() {
            summary.resumable = "none".to_string();
        }
    }
}

fn summary_for(conn: &Connection, key: &str) -> Result<AgentSessionSummary, String> {
    let mut summary = conn
        .query_row(
            "SELECT key, title, agent, cwd, created_at, last_activity_at, ended_at, \
         turn_count, parent_key, fork_kind, adopted FROM sessions WHERE key = ?1",
            params![key],
            summary_row,
        )
        .map_err(|e| format!("read session {key}: {e}"))?;
    hydrate_summary(conn, &mut summary);
    Ok(summary)
}

/// Exact run identity, never a title/path/recency guess. Duplicate identities
/// are rejected instead of attributing a run to an arbitrary conversation.
pub fn session_for_run(vault: &Path, run_id: &str) -> Result<Option<AgentSessionSummary>, String> {
    let conn = open(vault)?;
    let mut stmt = conn
        .prepare("SELECT DISTINCT session_key FROM turns WHERE run_id = ?1")
        .map_err(|e| e.to_string())?;
    let keys = stmt
        .query_map(params![run_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    match keys.as_slice() {
        [] => Ok(None),
        [key] => summary_for(&conn, key).map(Some),
        _ => Err(format!("ambiguous session identity for run {run_id}")),
    }
}

/// When run `run_id`'s turn in conversation `key` started (the queued
/// badge's holder, P1-46); `None` when that turn isn't recorded.
pub(crate) fn turn_started_at(
    vault: &Path,
    key: &str,
    run_id: &str,
) -> Result<Option<String>, String> {
    let conn = open(vault)?;
    conn.query_row(
        "SELECT started_at FROM turns WHERE session_key = ?1 AND run_id = ?2 ORDER BY ord DESC LIMIT 1",
        params![key, run_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("turn start for {run_id}: {e}"))
}

/// One conversation by its exact key, archived or not and however old: the
/// rail and the workspace page list only the newest. `None` when no
/// conversation has this key (merged plan slice 4, B·F15).
pub fn session_by_key(vault: &Path, key: &str) -> Result<Option<AgentSessionSummary>, String> {
    if !is_safe_session_key(key) {
        return Ok(None);
    }
    let conn = open(vault)?;
    let found = conn
        .query_row(
            "SELECT 1 FROM sessions WHERE key = ?1",
            params![key],
            |_| Ok(()),
        )
        .optional()
        .map_err(|e| format!("find session {key}: {e}"))?;
    match found {
        Some(()) => summary_for(&conn, key).map(Some),
        None => Ok(None),
    }
}

/// Whether conversation `key` is archived; `None` when no conversation has
/// this key (overnight A1: an archived chat is never offered a resume).
pub fn is_archived(vault: &Path, key: &str) -> Result<Option<bool>, String> {
    if !is_safe_session_key(key) {
        return Ok(None);
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT archived FROM sessions WHERE key = ?1",
            params![key],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(|e| format!("archived {key}: {e}"))
    })
}

/// The model chat run `run_id` asked for (else the one it reported), from
/// its first usage row; `None` when unknown. A resume after a restart asks
/// for the same one, so the native session's configuration is unchanged
/// (overnight A1).
pub fn chat_run_model(vault: &Path, run_id: &str) -> Result<Option<String>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT COALESCE(requested_model, model) FROM usage_ledger \
             WHERE run_id = ?1 AND surface = 'chat' ORDER BY id LIMIT 1",
            params![run_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|e| format!("chat run model: {e}"))
    })
}

/// When conversation `key` last recorded a stream event (RFC 3339), or
/// `None` when it has none (or retention dropped them). Recovery rewrites a
/// turn's and its usage row's end times; this is when it was last seen
/// working (overnight A1's staleness).
pub fn last_event_at(vault: &Path, key: &str) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Ok(None);
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT MAX(at) FROM events WHERE session_key = ?1",
            params![key],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|e| format!("last event {key}: {e}"))
    })
}

/// One conversation as the "needs you" query asks about its ask-canvases
/// (Canvas C4 A3): its working folder, whether it is archived, and the user
/// messages (`asked`, oldest first) that contain `needle`, the answer
/// block's fence language. `None` when no conversation has `key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAsks {
    pub cwd: String,
    pub archived: bool,
    pub asked: Vec<String>,
}

pub fn session_asks(vault: &Path, key: &str, needle: &str) -> Result<Option<SessionAsks>, String> {
    if !is_safe_session_key(key) {
        return Ok(None);
    }
    let conn = open(vault)?;
    let head: Option<(String, bool)> = conn
        .query_row(
            "SELECT cwd, archived FROM sessions WHERE key = ?1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("find session {key}: {e}"))?;
    let Some((cwd, archived)) = head else {
        return Ok(None);
    };
    let mut stmt = conn
        .prepare(
            "SELECT asked FROM turns WHERE session_key = ?1 AND asked IS NOT NULL \
             AND instr(asked, ?2) > 0 ORDER BY ord",
        )
        .map_err(|e| format!("asks of {key}: {e}"))?;
    let asked = stmt
        .query_map(params![key, needle], |row| row.get::<_, String>(0))
        .map_err(|e| format!("asks of {key}: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("asks of {key}: {e}"))?;
    Ok(Some(SessionAsks {
        cwd,
        archived,
        asked,
    }))
}

/// How [`search_sessions`] reads the person's messages (`turns.asked`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageText {
    /// Legacy caller name for the person's own search. The shared derived
    /// index now scrubs secrets for both person and agent callers.
    AsWritten,
    /// Through [`crate::claude_runner::redact`]: an agent's search
    /// (`sessions.search_sessions`). As `sessions.conversation`'s search
    /// (R4), a word only a secret holds is no match, a placeholder such as
    /// `[api-key]` holds no word, and a snippet never carries a secret, so
    /// this action is redacted like the session reads (review B·F11).
    Redacted,
}

/// One [`search_sessions`] hit.
#[derive(Debug, Clone)]
pub struct SessionHit {
    pub summary: AgentSessionSummary,
    pub context: SearchContext,
    pub rank: u8,
    /// The earliest indexed turn holding the first word the title lacks,
    /// cut around that match ([`search_snippet`]).
    pub snippet: Option<String>,
}

/// Most words of a query [`search_sessions`] reads; any after them are
/// ignored (each word past [`SEARCH_SQL_WORDS`] costs a query per row).
const SEARCH_MAX_WORDS: usize = 32;
/// Most of them tested in SQL, so the statement stays bounded. Every word
/// read is checked again on the rows it returns.
const SEARCH_SQL_WORDS: usize = 8;
/// A snippet's length in characters, before its `…` marks.
const SEARCH_SNIPPET_CHARS: usize = 140;
/// Characters a snippet keeps before its match when the message has them.
const SEARCH_SNIPPET_LEAD: usize = 40;
/// How far a snippet's cut moves to fall between two words.
const SEARCH_SNIPPET_SNAP: usize = 16;

/// The turns `t` whose `asked` the person wrote: not another agent's
/// (`asked_by` `agent`), not a request Hollow wrote for them (`hollow:*`),
/// and not one of Hollow's own messages (a wake, check-in, night-queue item
/// or runbook run: they open with "⟲ Hollow", as `chat_title` reads them).
/// Turns from before `asked_by` (2026-10-07) have none and count as the
/// person's.
const SEARCH_PERSON_TURN: &str = "t.asked_by IS NULL AND t.asked IS NOT NULL \
     AND ltrim(substr(t.asked, 1, 64), ' ' || char(9, 10, 13)) NOT LIKE '⟲ Hollow%'";

/// Titles that Rust's folding could match where SQLite's `lower()` can't,
/// for a word with a character outside ASCII: any title with one.
const TITLE_GLOB_WIDE: &str = "*[^ -~]*";
/// The same for an ASCII word: a title with `İ` or the Kelvin sign, which
/// fold to `i̇` and `k`, the only letters that fold into ASCII.
const TITLE_GLOB_ASCII: &str = "*[\u{130}\u{212A}]*";

/// One word of a search query.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchWord {
    /// Fully lowercased: what a title (folded in Rust) must hold.
    full: String,
    /// What a message may hold as SQLite's `lower()` leaves it, since it
    /// folds ASCII letters only: `full`; the word as typed with its ASCII
    /// letters lowercased; and, when `full` starts with a letter outside
    /// ASCII, `full` with that letter capitalized ("москва" finds "Москва",
    /// "über" finds "Über"). So a capital outside ASCII past a word's first
    /// letter is found only when typed as one: "GRÖSSE" by "GRÖSSE", not by
    /// "grösse".
    forms: Vec<String>,
}

impl SearchWord {
    fn new(typed: &str) -> Self {
        let full = typed.to_lowercase();
        let mut forms = vec![full.clone()];
        let mut add = |form: String| {
            if !forms.contains(&form) {
                forms.push(form);
            }
        };
        add(typed.to_ascii_lowercase());
        let mut rest = full.chars();
        if let Some(first) = rest.next().filter(|c| !c.is_ascii()) {
            let mut upper = first.to_uppercase();
            if let (Some(capital), None) = (upper.next(), upper.next()) {
                if capital != first && !capital.is_ascii() {
                    add(format!("{capital}{}", rest.as_str()));
                }
            }
        }
        Self { full, forms }
    }

    /// Whether the full-text index can find this word: the trigram
    /// tokenizer has no entries for one or two characters.
    fn indexed(&self) -> bool {
        self.full.chars().count() >= 3
    }
}

/// `(instr(lower(expr), ?first) > 0 OR …)`: one test per form, parameters
/// numbered from `first`.
fn any_form(expr: &str, first: usize, forms: usize) -> String {
    let tests: Vec<String> = (first..first + forms)
        .map(|n| format!("instr(lower({expr}), ?{n}) > 0"))
        .collect();
    format!("({})", tests.join(" OR "))
}

/// The search's SQL and its parameters: unarchived conversations after
/// `after` (`(last activity, key)`, newest first) that hold each of the
/// first [`SEARCH_SQL_WORDS`] `words` in the title or in one of the person's
/// messages. The order is `idx_sessions_recent`'s, so rows stream from the
/// index and a page stops at its last row instead of sorting the store; the
/// keyset's first test is a range on it.
fn search_sql(words: &[SearchWord], after: Option<(&str, &str)>) -> (String, Vec<String>) {
    let mut sql = String::from(
        "SELECT s.key, s.title, s.agent, s.cwd, s.created_at, s.last_activity_at, \
                s.ended_at, s.turn_count, s.parent_key, s.fork_kind, s.adopted \
         FROM sessions s WHERE s.archived = 0 \
         AND (s.fork_kind IS NULL OR s.fork_kind <> 'side')",
    );
    let mut args: Vec<String> = Vec::new();
    if let Some((at, key)) = after {
        args.extend([at.to_string(), key.to_string()]);
        sql.push_str(" AND s.last_activity_at <= ?1 AND (s.last_activity_at < ?1 OR s.key > ?2)");
    }
    for word in words.iter().take(SEARCH_SQL_WORDS) {
        let first = args.len() + 1;
        args.extend(word.forms.iter().cloned());
        // A title Rust's folding may match lets the row through; Rust decides.
        let glob = if word.full.is_ascii() {
            TITLE_GLOB_ASCII
        } else {
            TITLE_GLOB_WIDE
        };
        sql.push_str(&format!(
            " AND (instr(lower(coalesce(s.title, '')), ?{first}) > 0 OR s.title GLOB '{glob}' \
             OR EXISTS (SELECT 1 FROM turns t WHERE t.session_key = s.key \
             AND {SEARCH_PERSON_TURN} AND {}))",
            any_form("t.asked", first, word.forms.len())
        ));
    }
    sql.push_str(" ORDER BY s.last_activity_at DESC, s.key ASC");
    (sql, args)
}

/// One priority's candidate rows for the ranked search: unarchived
/// conversations whose working folder is one of `cwds`, strictly after
/// `after` (`(last activity, key)`), newest first, at most `limit`, streamed
/// from `idx_sessions_recent` (the folder test is kept off every index, `+`,
/// so it never displaces it). Each of the first [`SEARCH_SQL_WORDS`] words of
/// three characters or more must be in one of the chat's index documents (an
/// FTS5 phrase, evaluated once). When every word is shorter, the search reads
/// titles only: each word must be in the title as SQLite's `lower()` folds
/// it, or the title has a character Rust's folding may match. Rust checks
/// every word again on the rows returned.
fn search_hits_sql(
    words: &[SearchWord],
    cwds: &[String],
    after: Option<(&str, &str)>,
    limit: usize,
) -> (String, Vec<String>) {
    let mut sql = String::from(
        "SELECT s.key, s.title, s.agent, s.cwd, s.created_at, s.last_activity_at, \
                s.ended_at, s.turn_count, s.parent_key, s.fork_kind, s.adopted \
         FROM sessions s WHERE s.archived = 0 \
         AND (s.fork_kind IS NULL OR s.fork_kind <> 'side') \
         AND +s.cwd IN (SELECT value FROM json_each(?1))",
    );
    let mut args: Vec<String> =
        vec![serde_json::to_string(cwds).unwrap_or_else(|_| "[]".to_string())];
    if let Some((at, key)) = after {
        args.extend([at.to_string(), key.to_string()]);
        sql.push_str(" AND s.last_activity_at <= ?2 AND (s.last_activity_at < ?2 OR s.key > ?3)");
    }
    let indexed: Vec<&SearchWord> = words.iter().filter(|w| w.indexed()).collect();
    if indexed.is_empty() {
        for word in words.iter().take(SEARCH_SQL_WORDS) {
            args.push(word.full.clone());
            let glob = if word.full.is_ascii() {
                TITLE_GLOB_ASCII
            } else {
                TITLE_GLOB_WIDE
            };
            sql.push_str(&format!(
                " AND (instr(lower(coalesce(s.title, '')), ?{}) > 0 OR s.title GLOB '{glob}')",
                args.len()
            ));
        }
    } else {
        for word in indexed.into_iter().take(SEARCH_SQL_WORDS) {
            args.push(fulltext::phrase(word));
            sql.push_str(&format!(
                " AND s.key IN (SELECT session_key FROM chat_search_fts WHERE body MATCH ?{})",
                args.len()
            ));
        }
    }
    sql.push_str(&format!(
        " ORDER BY s.last_activity_at DESC, s.key ASC LIMIT {limit}"
    ));
    (sql, args)
}

/// Every working folder an unarchived conversation has, each once: a short
/// list (a workspace folder or worktree each), read by stepping through
/// `idx_sessions_cwd` one folder at a time rather than reading every row.
fn search_cwds(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "WITH RECURSIVE folders(cwd) AS ( \
               SELECT (SELECT min(cwd) FROM sessions WHERE archived = 0) \
               UNION ALL \
               SELECT (SELECT min(cwd) FROM sessions WHERE archived = 0 AND cwd > folders.cwd) \
               FROM folders WHERE folders.cwd IS NOT NULL \
             ) SELECT cwd FROM folders WHERE cwd IS NOT NULL",
        )
        .map_err(|e| format!("session search folders prepare: {e}"))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| format!("session search folders: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("session search folder: {e}"))
}

#[cfg(test)]
thread_local! {
    /// Conversations the ranked search read from SQL on this thread, so a
    /// test can prove a page reads about a page (2026-11-08 review).
    static SEARCH_ROWS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Conversations [`search_sessions_ranked`] has read on this thread so far.
#[cfg(test)]
pub(crate) fn search_rows_read() -> usize {
    SEARCH_ROWS_READ.with(std::cell::Cell::get)
}

/// Unarchived conversations newest first (ties by key), strictly after
/// `after` (`(last activity, key)`), that `keep` accepts: at most `limit`,
/// and whether more follow. The sidebar's Show older for one project
/// (overnight 11-04 S4b); rows stream from `idx_sessions_recent` as the
/// search's do ([`search_sql`] with no words).
pub fn list_sessions_after(
    vault: &Path,
    keep: impl Fn(&AgentSessionSummary) -> bool,
    after: Option<(&str, &str)>,
    limit: usize,
) -> Result<(Vec<AgentSessionSummary>, bool), String> {
    if limit == 0 {
        return Ok((Vec::new(), false));
    }
    let (sql, args) = search_sql(&[], after);
    let conn = open(vault)?;
    let mut page = Vec::new();
    let mut more = false;
    {
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("session page prepare: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), summary_row)
            .map_err(|e| format!("session page query: {e}"))?;
        for row in rows {
            let summary = row.map_err(|e| format!("session page row: {e}"))?;
            if !keep(&summary) {
                continue;
            }
            if page.len() == limit {
                more = true;
                break;
            }
            page.push(summary);
        }
    }
    for summary in page.iter_mut() {
        hydrate_summary(&conn, summary);
    }
    Ok((page, more))
}

/// Unarchived conversations holding every query word across titles, personal
/// requests, replies, errors and short tool summaries. The shared derived
/// index redacts before indexing; `text` remains a compatibility argument.
/// This entry point keeps recency order. The workspace-facing ranked entry
/// point below also carries context and preference-aware keyset pagination.
#[cfg(test)]
pub fn search_sessions(
    vault: &Path,
    query: &str,
    keep: impl Fn(&AgentSessionSummary) -> bool,
    after: Option<&crate::page_cursor::Keyset>,
    limit: usize,
    text: MessageText,
) -> Result<(Vec<SessionHit>, bool), String> {
    search_sessions_ranked(vault, query, keep, after, limit, text, |_| Some(0))
}

/// The ranked chat search: at most `limit` hits and whether more follow.
/// `rank` gives a working folder's priority (lower first), or `None` to
/// leave its conversations out; it is asked once per folder. Within a
/// priority, newest first, ties by key. `after` is the last hit of the page
/// before, its keyset's time `"<priority>|<last activity>"` (a time with no
/// priority is priority 0). `keep` can still drop a row SQL returned.
///
/// Each priority is its own query, keyset and `LIMIT` in SQL, from the
/// cursor's priority on, and the search stops once a page plus one hit is
/// held (2026-11-08 review: one stream newest first can't stop before the
/// end while a later row may rank ahead, so a broad query with no preferred
/// workspace read every matching chat). A page reads about one page of rows
/// per priority it reaches; a row Rust rejects costs one more.
pub(crate) fn search_sessions_ranked(
    vault: &Path,
    query: &str,
    keep: impl Fn(&AgentSessionSummary) -> bool,
    after: Option<&crate::page_cursor::Keyset>,
    limit: usize,
    text: MessageText,
    rank: impl Fn(&str) -> Option<u8>,
) -> Result<(Vec<SessionHit>, bool), String> {
    let mut words: Vec<SearchWord> = Vec::new();
    for typed in query.split_whitespace() {
        let word = SearchWord::new(typed);
        if words.len() < SEARCH_MAX_WORDS && !words.iter().any(|w| w.full == word.full) {
            words.push(word);
        }
    }
    if words.is_empty() || limit == 0 {
        return Ok((Vec::new(), false));
    }
    let after = match after {
        // Every conversation has a last activity, so a cursor without one
        // sorts after them all (`page_cursor::newest_first`).
        Some(cursor) => match cursor.at.as_deref() {
            Some(at) => {
                let (priority, at) = match at.split_once('|') {
                    Some((priority, at)) => (
                        priority
                            .parse::<u8>()
                            .map_err(|_| "Invalid search cursor.".to_string())?,
                        at,
                    ),
                    None => (0, at),
                };
                Some((priority, at, cursor.id.as_str()))
            }
            None => return Ok((Vec::new(), false)),
        },
        None => None,
    };
    let conn = open(vault)?;
    // Reindex only when something is marked: a refresh with nothing to do
    // still opened three connections of its own (2026-11-08).
    if fulltext::pending(&conn)? {
        fulltext::refresh(vault, fulltext::REFRESH_UNITS)?;
    }
    let title_only = words.iter().all(|w| !w.indexed());
    // Each priority's folders, from the cursor's priority on.
    let floor = after.map_or(0, |(priority, _, _)| priority);
    let mut tiers: std::collections::BTreeMap<u8, Vec<String>> = std::collections::BTreeMap::new();
    for cwd in search_cwds(&conn)? {
        if let Some(priority) = rank(&cwd).filter(|p| *p >= floor) {
            tiers.entry(priority).or_default().push(cwd);
        }
    }
    let mut found: Vec<(u8, AgentSessionSummary, Option<String>)> = Vec::new();
    let mut more = false;
    'tiers: for (priority, cwds) in &tiers {
        // The cursor's keyset applies within its own priority; a later one
        // starts at its newest chat.
        let mut seek: Option<(String, String)> = after
            .filter(|(p, _, _)| p == priority)
            .map(|(_, at, key)| (at.to_string(), key.to_string()));
        loop {
            let want = limit + 1 - found.len();
            let (sql, args) = search_hits_sql(
                &words,
                cwds,
                seek.as_ref().map(|(at, key)| (at.as_str(), key.as_str())),
                want,
            );
            let rows: Vec<AgentSessionSummary> = {
                let mut stmt = conn
                    .prepare(&sql)
                    .map_err(|e| format!("session search prepare: {e}"))?;
                let read = stmt
                    .query_map(rusqlite::params_from_iter(args.iter()), summary_row)
                    .map_err(|e| format!("session search query: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("session search row: {e}"));
                read?
            };
            #[cfg(test)]
            SEARCH_ROWS_READ.with(|n| n.set(n.get() + rows.len()));
            let exhausted = rows.len() < want;
            for mut summary in rows {
                seek = Some((summary.last_activity_at.clone(), summary.key.clone()));
                if !keep(&summary) {
                    continue;
                }
                summary.title = summary.title.map(|s| crate::claude_runner::redact(&s));
                // Each word the title lacks must be in one of the chat's
                // indexed turns (short words alone read titles only); the
                // first one's turn makes the snippet.
                let title = fulltext::searchable(summary.title.as_deref().unwrap_or_default())
                    .to_lowercase();
                let mut snippet = None;
                let mut every = true;
                for word in words.iter().filter(|w| !title.contains(w.full.as_str())) {
                    let holding = if title_only {
                        None
                    } else {
                        message_with(&conn, &summary.key, word, text)?
                    };
                    let Some(holding) = holding else {
                        every = false;
                        break;
                    };
                    if snippet.is_none() {
                        snippet = Some(search_snippet(
                            &holding.text,
                            holding.at,
                            word.full.chars().count(),
                        ));
                    }
                }
                if !every {
                    continue;
                }
                if found.len() == limit {
                    more = true;
                    break 'tiers;
                }
                found.push((*priority, summary, snippet));
            }
            if exhausted {
                break;
            }
        }
    }
    let mut page = Vec::with_capacity(found.len());
    for (priority, mut summary, snippet) in found {
        let context = fulltext::context(&conn, &summary.key, &words)?;
        hydrate_summary(&conn, &mut summary);
        page.push(SessionHit {
            summary,
            snippet,
            context,
            rank: priority,
        });
    }
    Ok((page, more))
}

/// Reindex everything the chat search index has marked, in bounded passes
/// ([`fulltext::refresh`]), pausing `pause` between them so other writers
/// get the store. A search reindexes only a budget first; this finishes.
pub fn drain_search_index(vault: &Path, pause: std::time::Duration) -> Result<(), String> {
    while fulltext::refresh(vault, fulltext::REFRESH_UNITS)? {
        std::thread::sleep(pause);
    }
    Ok(())
}

/// The startup integrity check of the chat search index (PER-23): a damaged
/// full-text index is rebuilt from the stored chats and refilled, which are
/// never touched. `true` when it was rebuilt. A clean index costs a bounded
/// read-only probe and no write lock; [`check_search_index_soon`] runs it off
/// the startup path.
pub fn check_search_index(vault: &Path) -> Result<bool, String> {
    fulltext::verify(vault)
}

/// Once per process per vault, on a background thread: [`check_search_index`],
/// then a drain of whatever the rebuild marked.
pub fn check_search_index_soon(vault: &Path) {
    static CHECKED: Lazy<Mutex<std::collections::HashSet<PathBuf>>> =
        Lazy::new(|| Mutex::new(std::collections::HashSet::new()));
    if !CHECKED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(vault.to_path_buf())
    {
        return;
    }
    let vault = vault.to_path_buf();
    let _ = std::thread::Builder::new()
        .name("chat-search-check".into())
        .spawn(move || match check_search_index(&vault) {
            Ok(true) => drain_search_index_soon(&vault),
            Ok(false) => {}
            Err(e) => eprintln!("[session_store] chat search check: {e}"),
        });
}

/// Vaults whose chat search index a background thread is draining now.
static SEARCH_DRAINS: Lazy<Mutex<std::collections::HashSet<PathBuf>>> =
    Lazy::new(|| Mutex::new(std::collections::HashSet::new()));

/// After a search: when marks remain past its budget, finish them on one
/// background thread per vault ([`drain_search_index`]), so the next search
/// reads a current index without indexing on its own path.
pub fn drain_search_index_soon(vault: &Path) {
    if !with_read(vault, fulltext::pending).unwrap_or(false) {
        return;
    }
    let vault = vault.to_path_buf();
    if !SEARCH_DRAINS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(vault.clone())
    {
        return;
    }
    let done = vault.clone();
    let spawned = std::thread::Builder::new()
        .name("chat-search-index".into())
        .spawn(move || {
            if let Err(e) = drain_search_index(&vault, std::time::Duration::from_millis(20)) {
                eprintln!("[session_store] chat search index: {e}");
            }
            SEARCH_DRAINS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&vault);
        });
    if spawned.is_err() {
        SEARCH_DRAINS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&done);
    }
}

/// A message that holds a search word, as the reader sees it, and the
/// character where the word starts in it.
struct Holding {
    text: String,
    at: usize,
}

/// The first of the person's messages in conversation `key` (lowest `ord`)
/// that holds `word` as `text` reads it; `None` when none does.
fn message_with(
    conn: &Connection,
    key: &str,
    word: &SearchWord,
    _text: MessageText,
) -> Result<Option<Holding>, String> {
    fulltext::message(conn, key, word)
}

/// The character of `text` where `word` (fully lowercased) first starts,
/// with Rust's full case folding. Lowercasing can change a character's
/// length (`İ`), so the offset is counted per character, as
/// `app_actions::snippet` counts it.
fn find_folded(text: &str, word: &str) -> Option<usize> {
    let byte = text.to_lowercase().find(word)?;
    let mut lower = 0;
    text.chars().position(|c| {
        let here = lower >= byte;
        lower += c.to_lowercase().map(char::len_utf8).sum::<usize>();
        here
    })
}

/// About [`SEARCH_SNIPPET_CHARS`] characters of `text` around the match
/// that starts at its character `at` and is about `width` characters long:
/// whitespace collapsed to single spaces, cut on character boundaries
/// (between words when a space is within [`SEARCH_SNIPPET_SNAP`]
/// characters, never inside the match), with `…` where it is cut.
fn search_snippet(text: &str, at: usize, width: usize) -> String {
    let mut chars: Vec<char> = Vec::new();
    let mut hit = 0;
    let mut gap = false;
    for (i, c) in text.chars().enumerate() {
        if c.is_whitespace() {
            gap = !chars.is_empty();
            continue;
        }
        if gap {
            chars.push(' ');
            gap = false;
        }
        if i == at {
            hit = chars.len();
        }
        chars.push(c);
    }
    let total = chars.len();
    let mut start = hit
        .saturating_sub(SEARCH_SNIPPET_LEAD)
        .min(total.saturating_sub(SEARCH_SNIPPET_CHARS));
    let mut end = (start + SEARCH_SNIPPET_CHARS).min(total);
    if start > 0 && chars[start - 1] != ' ' {
        let reach = (start + SEARCH_SNIPPET_SNAP).min(hit);
        if let Some(space) = chars[start..reach].iter().position(|&c| c == ' ') {
            start += space + 1;
        }
    }
    if end < total && chars[end] != ' ' {
        let match_end = (hit + width).min(end);
        let reach = end.saturating_sub(SEARCH_SNIPPET_SNAP).max(match_end);
        if let Some(space) = chars[reach..end].iter().rposition(|&c| c == ' ') {
            end = reach + space;
        }
    }
    let mut out: String = chars[start..end].iter().collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if end < total {
        out.push('…');
    }
    out
}

/// What a saved feedback vote (Paperclip PR 17) snapshots about one turn, read
/// from the durable stores only: the turn row, its transcript, and the chat
/// usage row for the same run. Nothing here is capped or redacted; the
/// caller (`turn_feedback::record`) does both.
#[derive(Debug, Clone)]
pub(crate) struct FeedbackTurnSource {
    pub session_key: String,
    pub ord: i64,
    pub status: Option<String>,
    pub asked: String,
    pub agent: String,
    pub answer: String,
    pub tool_names: Vec<String>,
    pub recall: Vec<String>,
    pub model: Option<String>,
}

/// The exact-identity lookup behind a feedback vote, with the same rule as
/// [`session_for_run`]: a run id names one turn or the vote is refused.
#[derive(Debug, Clone)]
pub(crate) enum FeedbackTurnLookup {
    Missing,
    /// This many turn rows carry the run id.
    Ambiguous(usize),
    Found(FeedbackTurnSource),
}

pub(crate) fn feedback_turn_source(
    vault: &Path,
    run_id: &str,
) -> Result<FeedbackTurnLookup, String> {
    with_read(vault, |conn| {
        let rows = {
            let mut stmt = conn
                .prepare("SELECT session_key, ord, status, asked FROM turns WHERE run_id = ?1")
                .map_err(|e| format!("feedback turn prepare: {e}"))?;
            let rows = stmt
                .query_map(params![run_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })
                .map_err(|e| format!("feedback turn query: {e}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("feedback turn rows: {e}"))?
        };
        let (session_key, ord, status, asked) = match rows.len() {
            0 => return Ok(FeedbackTurnLookup::Missing),
            1 => rows.into_iter().next().expect("one row"),
            n => return Ok(FeedbackTurnLookup::Ambiguous(n)),
        };
        let transcript = load_transcript(conn, &session_key, ord)?;
        let model: Option<String> = conn
            .query_row(
                "SELECT COALESCE(model, requested_model) FROM usage_ledger \
                 WHERE run_id = ?1 AND surface = 'chat' ORDER BY id DESC LIMIT 1",
                params![run_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| format!("feedback model: {e}"))?
            .flatten();
        Ok(FeedbackTurnLookup::Found(FeedbackTurnSource {
            session_key,
            ord,
            status,
            asked: asked.unwrap_or_default(),
            agent: transcript.agent,
            answer: transcript.text,
            tool_names: transcript.tools.into_iter().map(|tool| tool.name).collect(),
            recall: transcript.recall,
            model,
        }))
    })
}

/// Capped assistant summaries already retained by the recorder; no raw tool
/// inputs/results are interpreted as prose or re-executed.
pub fn assistant_summaries(
    vault: &Path,
    key: &str,
) -> Result<std::collections::BTreeMap<u32, String>, String> {
    if !is_safe_session_key(key) {
        return Err("unsafe session key".into());
    }
    let conn = open(vault)?;
    let mut stmt = conn.prepare("SELECT turn_ord, payload FROM events WHERE session_key = ?1 AND kind = 'assistant' ORDER BY id ASC")
        .map_err(|e| e.to_string())?;
    let summaries = stmt
        .query_map(params![key], |row| {
            Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string());
    summaries
}

/// The user's own recent requests, newest first (the stripped `turns.asked`
/// text, without an ask-canvas's hidden answer block: `next_step::strip`),
/// for the About you builder. Read-only; empty asks are skipped.
pub fn recent_asks(vault: &Path, limit: usize) -> Result<Vec<String>, String> {
    let conn = open(vault)?;
    let mut stmt = conn
        .prepare(
            "SELECT asked FROM turns WHERE asked IS NOT NULL AND trim(asked) != '' \
             ORDER BY started_at DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit as i64], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|asked| crate::next_step::strip(asked))
        .filter(|asked| !asked.trim().is_empty())
        .collect())
}

/// One conversation's latest user requests, newest first: `(ord, asked)`,
/// at most `limit`, each the stored `turns.asked` text without an
/// ask-canvas's hidden answer block. Read-only. The antecedent resolver
/// (`turn_request.rs`) reads only these, never the composed prompts.
pub fn latest_asks(vault: &Path, key: &str, limit: usize) -> Result<Vec<(i64, String)>, String> {
    if !is_safe_session_key(key) || limit == 0 {
        return Ok(Vec::new());
    }
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT ord, asked FROM turns WHERE session_key = ?1 AND asked IS NOT NULL \
                 AND trim(asked) != '' ORDER BY ord DESC LIMIT ?2",
            )
            .map_err(|e| format!("asks of {key}: {e}"))?;
        let rows = stmt
            .query_map(params![key, limit as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("asks of {key}: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("asks of {key}: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|(ord, asked)| (ord, crate::next_step::strip(&asked)))
            .collect())
    })
}

/// Conversation `key`'s latest `limit` requests that a person or an agent
/// asked, newest first: never Hollow's own (`asked_by` `hollow:*`, or a
/// wake or check-in, `main_agent::is_wake_message`). The first 4,000
/// characters of each, which hold the 16 terms retrieval reads. What a
/// short follow-up borrows its topic from (overnight OB25); modelled on
/// [`recent_turns_for_sessions`].
pub fn person_asks(vault: &Path, key: &str, limit: usize) -> Result<Vec<String>, String> {
    if !is_safe_session_key(key) || limit == 0 {
        return Ok(Vec::new());
    }
    // Hollow's messages are left out in SQL too (a Main agent thread is
    // mostly wakes), except the two that open a conversation as its task.
    let wake = format!("{}%", crate::main_agent::WAKE_PREFIX);
    let retry = format!("{}%", crate::step_spine::RETRY_OPEN);
    let fix = format!("{}%", crate::friction::FIX_OPEN);
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT substr(asked, 1, 4000) FROM turns WHERE session_key = ?1 \
                 AND asked IS NOT NULL AND trim(asked) != '' \
                 AND (asked_by IS NULL OR asked_by NOT LIKE 'hollow:%') \
                 AND (asked NOT LIKE ?2 OR asked LIKE ?3 OR asked LIKE ?4) ORDER BY ord DESC",
            )
            .map_err(|e| format!("person asks of {key}: {e}"))?;
        let rows = stmt
            .query_map(params![key, wake, retry, fix], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("person asks of {key}: {e}"))?;
        // Newest first, stopping once `limit` are found; the same rule in
        // Rust (`is_wake_message` also reads past leading whitespace).
        let asks: Vec<String> = rows
            .filter_map(Result::ok)
            .filter(|asked| !crate::main_agent::is_wake_message(asked))
            .take(limit)
            .map(|asked| crate::next_step::strip(&asked))
            .collect();
        Ok(asks)
    })
}

/// The turn numbers (`ord`) of conversation `key`'s latest `limit` requests
/// a person or an agent asked, newest first: the turns [`person_asks`]
/// reads. What a task packet takes its settled constraints from (overnight
/// 9 A1): a rule said 20 messages ago, or once in a request Hollow wrote,
/// doesn't bind the work an agent hands on today.
pub fn person_ask_ords(vault: &Path, key: &str, limit: usize) -> Result<Vec<i64>, String> {
    if !is_safe_session_key(key) || limit == 0 {
        return Ok(Vec::new());
    }
    let wake = format!("{}%", crate::main_agent::WAKE_PREFIX);
    let retry = format!("{}%", crate::step_spine::RETRY_OPEN);
    let fix = format!("{}%", crate::friction::FIX_OPEN);
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT ord, substr(asked, 1, 400) FROM turns WHERE session_key = ?1 \
                 AND asked IS NOT NULL AND trim(asked) != '' \
                 AND (asked_by IS NULL OR asked_by NOT LIKE 'hollow:%') \
                 AND (asked NOT LIKE ?2 OR asked LIKE ?3 OR asked LIKE ?4) ORDER BY ord DESC",
            )
            .map_err(|e| format!("person ask turns of {key}: {e}"))?;
        let rows = stmt
            .query_map(params![key, wake, retry, fix], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("person ask turns of {key}: {e}"))?;
        Ok(rows
            .filter_map(Result::ok)
            .filter(|(_, asked)| !crate::main_agent::is_wake_message(asked))
            .take(limit)
            .map(|(ord, _)| ord)
            .collect())
    })
}

/// Give an accepted worker a sidebar identity before admission. No turn,
/// transcript or harness stint is fabricated; admission records those later.
pub(crate) fn remember_queued_worker(vault: &Path, key: &str, title: &str) -> Result<(), String> {
    if session_by_key(vault, key)?.is_some() {
        return Ok(());
    }
    let row =
        crate::run_queue::newest_chat_row(vault, key)?.ok_or("Worker queue row is missing")?;
    let agent = row.payload["agent"]
        .as_str()
        .ok_or("Worker agent is missing")?;
    let cwd = row.payload["cwd"]
        .as_str()
        .ok_or("Worker folder is missing")?;
    let now = Utc::now().to_rfc3339();
    with_write(vault, "queued worker", |tx| {
        tx.execute(
            "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, turn_count) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?5, 0) ON CONFLICT(key) DO NOTHING",
            params![key, title, agent, cwd, now],
        ).map_err(|e| format!("queued worker: {e}"))?;
        Ok(())
    })
}

/// Newest-first session list for the rail.
pub fn list_sessions(vault: &Path, limit: usize) -> Result<Vec<AgentSessionSummary>, String> {
    list_sessions_by_archive(vault, limit, false)
}

/// The unarchived conversations among `keys`, newest first: the sidebar's
/// pinned chats that [`list_sessions`]'s window didn't reach (overnight
/// 11-04 S3b). Unknown and archived keys are left out.
pub fn unarchived_sessions_by_key(
    vault: &Path,
    keys: &[&str],
) -> Result<Vec<AgentSessionSummary>, String> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let conn = open(vault)?;
    let marks = (1..=keys.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT key, title, agent, cwd, created_at, last_activity_at, ended_at, \
                    turn_count, parent_key, fork_kind, adopted FROM sessions \
             WHERE archived = 0 AND key IN ({marks}) ORDER BY last_activity_at DESC, key ASC"
        ))
        .map_err(|e| format!("pinned sessions prepare: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(keys.iter()), summary_row)
        .map_err(|e| format!("pinned sessions query: {e}"))?;
    let mut out: Vec<AgentSessionSummary> = rows.filter_map(Result::ok).collect();
    for summary in out.iter_mut() {
        hydrate_summary(&conn, summary);
    }
    Ok(out)
}

/// Each of `keys` that has a conversation (archived too), with its
/// `last_activity_at`, in one read (`project_agents::LeadKeys`).
pub(crate) fn last_activity_by_key(
    vault: &Path,
    keys: &[String],
) -> Result<std::collections::HashMap<String, String>, String> {
    let mut out = std::collections::HashMap::new();
    if keys.is_empty() {
        return Ok(out);
    }
    let conn = open(vault)?;
    for chunk in keys.chunks(500) {
        let marks = (1..=chunk.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut stmt = conn
            .prepare(&format!(
                "SELECT key, last_activity_at FROM sessions WHERE key IN ({marks})"
            ))
            .map_err(|e| format!("lead keys prepare: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(chunk.iter()), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| format!("lead keys query: {e}"))?;
        out.extend(rows.filter_map(Result::ok));
    }
    Ok(out)
}

pub fn list_sessions_by_archive(
    vault: &Path,
    limit: usize,
    archived: bool,
) -> Result<Vec<AgentSessionSummary>, String> {
    let conn = open(vault)?;
    // Ties by key, the order Show older's keyset pages in
    // ([`list_sessions_after`], overnight 11-04 S4b).
    let mut stmt = conn
        .prepare(
            "SELECT key, title, agent, cwd, created_at, last_activity_at, ended_at, \
                    turn_count, parent_key, fork_kind, adopted FROM sessions \
             WHERE archived = ?2 AND (fork_kind IS NULL OR fork_kind <> 'side') \
             ORDER BY last_activity_at DESC, key ASC LIMIT ?1",
        )
        .map_err(|e| format!("session list prepare: {e}"))?;
    let rows = stmt
        .query_map(params![limit as i64, archived], summary_row)
        .map_err(|e| format!("session list query: {e}"))?;
    let mut out: Vec<AgentSessionSummary> = rows.filter_map(Result::ok).collect();
    for summary in out.iter_mut() {
        hydrate_summary(&conn, summary);
    }
    Ok(out)
}

/// `native` = an explicit harness id we can resume; `briefOnly` = a session
/// whose id was never recorded, so continuation has to go through a brief.
fn resumability(agent: &str, harness_session_id: Option<&str>) -> String {
    match (agent, harness_session_id) {
        ("claude" | "codex", Some(id)) if !id.trim().is_empty() => "native".to_string(),
        ("antigravity", Some(id)) if !id.trim().is_empty() && id != "antigravity" => {
            "native".to_string()
        }
        _ => "briefOnly".to_string(),
    }
}

/// Rename a conversation. Only the display title changes; the title is set
/// once from the first request and never regenerated, so a rename sticks.
pub fn rename_session(vault: &Path, key: &str, title: &str) -> Result<(), String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 120 || title.contains(['\n', '\r']) {
        return Err("Use a conversation name of 1–120 characters on one line".into());
    }
    with_write(vault, "session rename", |tx| {
        let count = tx
            .execute(
                "UPDATE sessions SET title = ?2 WHERE key = ?1",
                params![key, title],
            )
            .map_err(|e| format!("session rename: {e}"))?;
        if count == 0 {
            return Err(format!("no session {key}"));
        }
        Ok(())
    })
}

/// What the optional model retitle (morning N5) reads of a conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct FirstRequest {
    pub title: String,
    /// The agent the conversation runs on (its provider gets the request).
    pub agent: String,
    pub ord: i64,
    pub asked: String,
}

/// A conversation's title, agent and its first turn's ordinal and request.
/// `None` when it has no turn.
pub fn first_request(vault: &Path, key: &str) -> Result<Option<FirstRequest>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT s.title, COALESCE(s.agent, ''), t.ord, COALESCE(t.asked, '') FROM sessions s \
             JOIN turns t ON t.session_key = s.key WHERE s.key = ?1 ORDER BY t.ord LIMIT 1",
            params![key],
            |r| {
                Ok(FirstRequest {
                    title: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    agent: r.get(1)?,
                    ord: r.get(2)?,
                    asked: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("first request: {e}"))
    })
}

/// Set a conversation's title only while it is still `expected` (the title
/// Hollow gave it): a person's rename in between wins. Whether it changed.
pub fn retitle_if(vault: &Path, key: &str, expected: &str, title: &str) -> Result<bool, String> {
    with_write(vault, "session retitle", |tx| {
        tx.execute(
            "UPDATE sessions SET title = ?3 WHERE key = ?1 AND title = ?2",
            params![key, expected, title],
        )
        .map(|n| n == 1)
        .map_err(|e| format!("session retitle: {e}"))
    })
}

/// Archive is reversible app metadata. It never edits/deletes CLI history,
/// vault files, transcripts, tool evidence, or native resume ids.
///
/// Archiving also expires the conversation's open question cards (Paperclip
/// PR 13), in the same IMMEDIATE transaction; unarchiving revives none.
/// Archiving is refused while a turn is queued or running (P1-39), checked as
/// [`move_agent_session`] does; restoring never is.
pub fn archive_session(vault: &Path, key: &str, archived: bool) -> Result<(), String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    if archived && crate::agent_sessions::live_for_key(key).is_some() {
        return Err(ARCHIVE_BUSY.into());
    }
    with_write(vault, "session archive", |tx| {
        if archived && chat_turn_queued(tx, key)? {
            return Err(ARCHIVE_BUSY.to_string());
        }
        let pinned: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_organization WHERE session_key = ?1 AND pinned = 1)",
            [key], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if archived && pinned {
            return Err("Unpin this conversation first".into());
        }
        let count = tx
            .execute(
                "UPDATE sessions SET archived = ?2 WHERE key = ?1",
                params![key, archived],
            )
            .map_err(|e| format!("session archive: {e}"))?;
        if count == 0 {
            return Err(format!("no session {key}"));
        }
        tx.execute("UPDATE session_organization SET snoozed_until = NULL, reason = NULL, revision = revision + 1 WHERE session_key = ?1", [key])
            .map_err(|e| e.to_string())?;
        if archived {
            crate::interactions::expire_pending_in(tx, key, &db_time(Utc::now()), "archived")?;
        }
        Ok(())
    })
}

/// The stint close reason a move writes (`chat_resume::ResetReason::Moved`).
pub const MOVED_REASON: &str = "moved";

/// Why a move is refused while the conversation has a turn in flight.
pub const MOVE_BUSY: &str =
    "This conversation has a turn queued or running. Move it when the turn finishes.";
/// Why archiving is refused while the conversation has a turn in flight.
pub const ARCHIVE_BUSY: &str =
    "This conversation has a turn queued or running. Archive it when the turn finishes.";

/// Whether chat `key` has a run-queue row queued, claimed or running.
fn chat_turn_queued(conn: &Connection, key: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM run_queue WHERE source = 'chat' AND source_id = ?1 \
         AND status IN ('queued', 'claimed', 'running')",
        params![key],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| format!("session queue check: {e}"))
}

/// What [`move_agent_session`] did.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMove {
    pub key: String,
    /// The workspace the conversation was in; `None` for a vault-root chat
    /// (Conversation history) or a folder that is no workspace.
    pub from: Option<String>,
    /// The workspace it is in now.
    pub to: String,
    /// Its cwd now (`workspaces::session_dir_for(to)`).
    pub cwd: String,
    /// False when it was already there (a no-op).
    pub moved: bool,
    /// The session record written in the new workspace, if any.
    pub record: Option<String>,
    /// Open question cards the move expired (they belonged to the old
    /// native session).
    pub expired_cards: usize,
    /// Record-file problems that did not stop the move.
    pub warnings: Vec<String>,
}

/// Move conversation `key` to workspace `workspace_id` (2026-09-30): its
/// `cwd` becomes the target's session folder, so the rail groups it there and
/// its next turn runs there with that workspace's context. The open native
/// stint closes with reason [`MOVED_REASON`] and a fresh one opens, so the
/// next turn never resumes the old native session in the wrong folder: it
/// starts fresh and receives the conversation so far through the usual
/// handoff (`chat_resume::plan_resume` → `ResumePlan::Reset(Moved)` when the
/// composer still holds the old id, the frontend's own bundle when it holds
/// none). Open question cards expire with the old session. The vault record
/// moves too (`workspaces::move_session_record`).
///
/// Refused for an unsafe or unknown key and while the conversation has a
/// turn queued, claimed or running (checked against the live record and,
/// inside the write transaction, the run queue, so a send can't slip in
/// between). Moving into the workspace it is already in is a no-op success.
///
/// Takes no lease itself: the rail's command wraps it in the workspace edit
/// lease ([`move_agent_session_with`]); a Hollow app tool runs it inside the
/// calling run's own lease, so the record move lands in that run's History.
pub fn move_agent_session(
    vault: &Path,
    key: &str,
    workspace_id: &str,
) -> Result<SessionMove, String> {
    move_agent_session_with(vault, key, workspace_id, || Ok(()))
}

/// [`move_agent_session`], taking `guard` (held to the end) only once the
/// move is known to change something: a no-op or a refusal never waits on it.
pub fn move_agent_session_with<G>(
    vault: &Path,
    key: &str,
    workspace_id: &str,
    guard: impl FnOnce() -> Result<G, String>,
) -> Result<SessionMove, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let workspace_id = workspace_id.trim();
    // One Main agent conversation (2026-11-07): a chat moved into its folder
    // would become its newest, the one the button, wakes and sends use.
    if crate::workspaces::is_main_agent(workspace_id) {
        return Err(crate::app_sessions::MAIN_MOVE_REFUSED.into());
    }
    if crate::agent_sessions::live_for_key(key).is_some() {
        return Err(MOVE_BUSY.into());
    }
    let cwd = with_read(vault, |conn| {
        conn.query_row(
            "SELECT cwd FROM sessions WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("session move read: {e}"))
    })?
    .ok_or_else(|| format!("no session {key}"))?;
    let from = crate::workspaces::workspace_for_cwd(vault, Path::new(&cwd))
        .ok()
        .flatten();
    let target = crate::workspaces::session_dir_for(vault, workspace_id)?;
    let target_cwd = target.to_string_lossy().to_string();
    let canonical = |p: &str| {
        Path::new(p)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(p))
    };
    let mut out = SessionMove {
        key: key.to_string(),
        from: from.as_ref().map(|w| w.id.clone()),
        to: workspace_id.to_string(),
        cwd: target_cwd.clone(),
        ..SessionMove::default()
    };
    if from.as_ref().is_some_and(|w| w.id == workspace_id)
        || canonical(&cwd) == canonical(&target_cwd)
    {
        return Ok(out);
    }
    let _guard = guard()?;
    let expired = with_write(vault, "session move", |tx| {
        if chat_turn_queued(tx, key)? {
            return Err(MOVE_BUSY.to_string());
        }
        let agent: String = tx
            .query_row(
                "SELECT agent FROM sessions WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .map_err(|e| format!("session move: {e}"))?;
        tx.execute(
            "UPDATE sessions SET cwd = ?2 WHERE key = ?1",
            params![key, target_cwd],
        )
        .map_err(|e| format!("session move: {e}"))?;
        let now = Utc::now().to_rfc3339();
        let open_agent: Option<String> = tx
            .query_row(
                "SELECT agent FROM stints WHERE session_key = ?1 AND ended_at IS NULL ORDER BY id DESC LIMIT 1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("session move stint: {e}"))?;
        match &open_agent {
            Some(_) => {
                tx.execute(
                    "UPDATE stints SET ended_at = ?2, reason = ?3 WHERE session_key = ?1 AND ended_at IS NULL",
                    params![key, now, MOVED_REASON],
                )
                .map_err(|e| format!("session move stint close: {e}"))?;
            }
            // No open stint (a legacy row): record the move as a closed one
            // holding the latest native id, so the resume plan still sees it.
            None => {
                let last: Option<String> = tx
                    .query_row(
                        "SELECT harness_session_id FROM stints WHERE session_key = ?1 ORDER BY id DESC LIMIT 1",
                        params![key],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|e| format!("session move stint: {e}"))?
                    .flatten();
                tx.execute(
                    "INSERT INTO stints(session_key, agent, harness_session_id, started_at, ended_at, reason) \
                     VALUES(?1, ?2, ?3, ?4, ?4, ?5)",
                    params![key, agent, last, now, MOVED_REASON],
                )
                .map_err(|e| format!("session move stint: {e}"))?;
            }
        }
        tx.execute(
            "INSERT INTO stints(session_key, agent, started_at) VALUES(?1, ?2, ?3)",
            params![key, open_agent.as_deref().unwrap_or(&agent), now],
        )
        .map_err(|e| format!("session move stint open: {e}"))?;
        crate::interactions::expire_pending_rows_in(tx, key, &db_time(Utc::now()), "session_reset")
    })?;
    out.moved = true;
    out.expired_cards = expired.len();
    // Quick chats (2026-11-14): a chat moved into a project leaves its lead
    // a one-line note for its next turn.
    let title = session_by_key(vault, key)
        .ok()
        .flatten()
        .and_then(|s| s.title);
    crate::chat_chain::record_arrival(
        vault,
        key,
        title.as_deref(),
        workspace_id,
        from.as_ref().map(|w| w.title.as_str()),
    );
    let (record, warnings) = crate::workspaces::move_session_record(vault, key, from.as_ref());
    out.record = record;
    out.warnings = warnings;
    Ok(out)
}

/// Bounded runtime evidence for workspace re-entry; never loads a transcript.
pub fn latest_turn(vault: &Path, key: &str) -> Result<Option<SessionRecordTurn>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let conn = open(vault)?;
    conn.query_row(
        "SELECT ord, run_id, started_at, ended_at, status FROM turns WHERE session_key = ?1 ORDER BY ord DESC LIMIT 1",
        params![key],
        |row| Ok(SessionRecordTurn { ord: row.get::<_, i64>(0)? as u32, run_id: row.get(1)?, started_at: row.get(2)?, ended_at: row.get(3)?, status: row.get(4)?, asked: None }),
    ).optional().map_err(|e| format!("latest turn: {e}"))
}

pub fn read_session(vault: &Path, key: &str) -> Result<AgentSessionDetail, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let conn = open(vault)?;
    // Direct lookup is essential: an archived or older-than-1000 session is
    // still readable and resumable even when absent from the default rail.
    let summary = summary_for(&conn, key)?;
    let mut stmt = conn
        .prepare(
            "SELECT ord, run_id, started_at, ended_at, status, asked FROM turns \
             WHERE session_key = ?1 ORDER BY ord ASC",
        )
        .map_err(|e| format!("turn prepare: {e}"))?;
    let turns = stmt
        .query_map(params![key], |row| {
            Ok(SessionRecordTurn {
                ord: row.get::<_, i64>(0)? as u32,
                run_id: row.get(1)?,
                started_at: row.get(2)?,
                ended_at: row.get(3)?,
                status: row.get(4)?,
                asked: row.get(5)?,
            })
        })
        .map_err(|e| format!("turn query: {e}"))?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    let events_retained: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE session_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(AgentSessionDetail {
        summary,
        turns,
        events_retained: events_retained > 0,
    })
}

/// A turn's raw trace events, oldest first, as `(kind, payload)` (tests only).
#[cfg(test)]
pub(crate) fn turn_events(vault: &Path, key: &str, turn_ord: i64) -> Vec<(String, String)> {
    let conn = open(vault).expect("session store");
    let mut stmt = conn
        .prepare("SELECT kind, COALESCE(payload, '') FROM events WHERE session_key = ?1 AND turn_ord = ?2 ORDER BY id ASC")
        .expect("events prepare");
    let rows = stmt
        .query_map(params![key, turn_ord], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .expect("events query")
        .filter_map(Result::ok)
        .collect();
    rows
}

// ── WD1: the window's wire diet ────────────────────────────────────────

/// WD1 (2026-11-18): a stored tool result longer than this many characters
/// reaches the window's cold open as a preview ([`lean_for_window`]); the
/// full text comes when its row expands ([`read_tool_result`]).
pub(crate) const WIRE_RESULT_CHARS: usize = 1_200;
/// The preview's characters.
pub(crate) const WIRE_PREVIEW_CHARS: usize = 400;

/// Results a collapsed row reads whole, kept whole: a call with no id (it
/// could not be read back), Hollow's subagent and route calls (their JSON
/// gives the subagent row its key and the route chip its Mac), question
/// cards ("Posted ..."), and `gh pr create` output (the PR link), as the
/// window finds them (`subagentsModel.ts`, `routeReceipt.ts`, `runFold.ts`,
/// `shipActions.ts`).
fn keeps_whole(tool: &ConversationTool) -> bool {
    if tool.id.is_empty() {
        return true;
    }
    let name = tool.name.as_str();
    if let Some(verb) = name.strip_prefix("mcp__hollow_app__") {
        let action = tool.input.get("id").and_then(Value::as_str);
        return matches!(verb, "start_session" | "send_to_session")
            || (verb == "run_action"
                && matches!(action, Some("sessions.start" | "sessions.route_to_runner")));
    }
    if name.contains("ask_user_questions") || name.contains("hollow_questions") {
        return true;
    }
    let command = ["command", "cmd"]
        .iter()
        .filter_map(|k| tool.input.get(*k))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .contains("gh pr create")
}

/// Numbered lines in a Read result (`toolDescribe.ts` `readLineCount`).
fn numbered_lines(text: &str) -> u64 {
    text.lines()
        .filter(|line| {
            let t = line.trim_start();
            let rest = t.trim_start_matches(|c: char| c.is_ascii_digit());
            rest.len() < t.len() && (rest.starts_with('\u{2192}') || rest.starts_with('\t'))
        })
        .count() as u64
}

/// Cut `tool`'s result (of stored turn `ord`) to a preview when it is long
/// (WD1).
fn lean_tool(tool: &mut ConversationTool, ord: i64) {
    let Some(result) = tool.result.as_ref() else {
        return;
    };
    if keeps_whole(tool) {
        return;
    }
    let chars = result.chars().count();
    if chars <= WIRE_RESULT_CHARS {
        return;
    }
    let lines = if tool.name == "Read" {
        numbered_lines(result)
    } else {
        0
    };
    let preview: String = result.chars().take(WIRE_PREVIEW_CHARS).collect();
    tool.result = Some(preview);
    tool.result_chars = Some(chars as u64);
    tool.result_lines = (lines > 0).then_some(lines);
    tool.result_turn = Some(ord);
}

/// The window's cold open (WD1): every long tool result cut to a preview.
/// Agents' reads keep [`read_conversation`] whole.
pub fn lean_for_window(conversation: &mut AgentConversation) {
    let prefix = format!("{}-", conversation.summary.key);
    for turn in &mut conversation.turns {
        // `read_conversation`'s ids are `<key>-<ord>-assistant|user`; a turn
        // whose ord can't be read keeps its results whole.
        let Some(ord) = turn
            .id
            .strip_prefix(&prefix)
            .and_then(|rest| {
                rest.strip_suffix("-assistant")
                    .or_else(|| rest.strip_suffix("-user"))
            })
            .and_then(|ord| ord.parse::<i64>().ok())
        else {
            continue;
        };
        for tool in &mut turn.tools {
            lean_tool(tool, ord);
        }
    }
}

/// One stored tool call's full result (WD1: an expanded row whose cold
/// open sent a preview), from stored turn `ord` (the preview's
/// `resultTurn`). By turn, not run: a branch's copied turns have no run id,
/// and Codex tool ids repeat in every run.
pub fn read_tool_result(
    vault: &Path,
    key: &str,
    ord: i64,
    tool_id: &str,
) -> Result<Option<String>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    if tool_id.is_empty() {
        return Err("That tool call has no id.".into());
    }
    with_read(vault, |conn| {
        load_transcript(conn, key, ord)?
            .tools
            .into_iter()
            .find(|t| t.id == tool_id)
            .map(|t| t.result)
            .ok_or_else(|| "That tool call isn't in this chat's record.".to_string())
    })
}

#[cfg(test)]
#[path = "session_store_wire_tests.rs"]
mod wire_tests;

pub fn read_conversation(vault: &Path, key: &str) -> Result<AgentConversation, String> {
    let detail = read_session(vault, key)?;
    let conn = open(vault)?;
    // This conversation's chat receipts by run id, the newest per run, from
    // one scan of the cached store (five asks B4: no copy of the others).
    let wanted: std::collections::HashSet<&str> = detail
        .turns
        .iter()
        .filter_map(|t| t.run_id.as_deref())
        .collect();
    let mut receipts: std::collections::HashMap<String, crate::evidence::EvidenceReceipt> =
        std::collections::HashMap::new();
    let _ = crate::evidence::with_receipts(vault, |all| {
        for r in all
            .iter()
            .filter(|r| r.source == "chat" && wanted.contains(r.run_id.as_str()))
        {
            let newer = receipts
                .get(&r.run_id)
                .is_none_or(|seen| r.finished_at > seen.finished_at);
            if newer {
                receipts.insert(r.run_id.clone(), r.clone());
            }
        }
    });
    let mut turns = Vec::with_capacity(detail.turns.len() * 2);
    for turn in detail.turns {
        let transcript = load_transcript(&conn, key, i64::from(turn.ord))?;
        turns.push(ConversationTurn {
            id: format!("{key}-{}-user", turn.ord),
            reasoning: String::new(),
            role: "user".to_string(),
            text: turn.asked.unwrap_or_default(),
            tools: Vec::new(),
            agent: transcript.agent.clone(),
            run_id: turn.run_id.clone(),
            started_at: turn.started_at.clone(),
            ended_at: None,
            status: turn.status.clone(),
            cost_usd: None,
            error: None,
            receipt: None,
            transcript_incomplete: transcript.incomplete,
            result_kind: None,
            stopped_before_spawn: false,
            steers: Vec::new(),
            files: Vec::new(),
        });
        let receipt = turn
            .run_id
            .as_deref()
            .and_then(|id| receipts.get(id))
            .cloned();
        let mut tools = transcript.tools;
        if turn.status.as_deref() != Some("running") {
            // An interrupted call has no successful result. Keep that absence
            // visible instead of showing an eternal spinner or inventing success.
            for tool in &mut tools {
                if tool.status == "running" {
                    tool.status = "error".to_string();
                    tool.is_error = true;
                    tool.result =
                        Some("No tool result was recorded before the run ended.".to_string());
                }
            }
        }
        turns.push(ConversationTurn {
            id: format!("{key}-{}-assistant", turn.ord),
            reasoning: transcript.reasoning,
            role: "assistant".to_string(),
            text: transcript.text,
            tools,
            agent: transcript.agent,
            run_id: turn.run_id,
            started_at: turn.started_at,
            ended_at: turn.ended_at,
            status: turn.status,
            cost_usd: transcript.cost_usd,
            error: transcript.error,
            receipt,
            transcript_incomplete: transcript.incomplete,
            result_kind: transcript.result_kind,
            stopped_before_spawn: transcript.stopped_before_spawn,
            steers: transcript.steers,
            files: Vec::new(),
        });
    }
    Ok(AgentConversation {
        permission_mode: last_permission(vault, key)?,
        summary: detail.summary,
        turns,
        events_retained: detail.events_retained,
    })
}

pub fn status(vault: &Path) -> Result<SessionStoreStatus, String> {
    let path = db_path_for(vault).ok_or_else(|| "no session store path".to_string())?;
    let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let conn = open(vault)?;
    let sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .unwrap_or(0);
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap_or(0);
    let oldest: Option<String> = conn
        .query_row("SELECT MIN(at) FROM events", [], |row| row.get(0))
        .ok()
        .flatten();
    Ok(SessionStoreStatus {
        sessions: sessions as u32,
        events_retained: events as u32,
        bytes,
        oldest_retained_iso: oldest,
        path: path.to_string_lossy().to_string(),
    })
}

/// Parse helper kept public so a later phase's reconciler can reuse it.
pub fn parse_iso(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 2026-11-12: a chat's own effort is kept per session key: set, read,
    /// cleared by Default, refused when unknown, filled in by the one-time
    /// import without overwriting, and inherited by a handover successor.
    #[test]
    fn a_chats_own_effort_is_kept_cleared_imported_and_inherited() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (a, b, c) = (
            "s-20261112-ef000001",
            "s-20261112-ef000002",
            "s-20261112-ef000003",
        );
        assert_eq!(chat_effort(v, a).unwrap(), None);
        // No sessions row is needed (a level picked before the first turn).
        set_chat_effort(v, a, Some("high")).unwrap();
        assert_eq!(chat_effort(v, a).unwrap().as_deref(), Some("high"));
        set_chat_effort(v, a, Some(" max ")).unwrap();
        assert_eq!(chat_effort(v, a).unwrap().as_deref(), Some("max"));
        assert!(
            set_chat_effort(v, a, Some("ultracode")).is_err(),
            "not a level"
        );
        assert!(
            set_chat_effort(v, "../x", Some("low")).is_err(),
            "unsafe key"
        );
        assert_eq!(
            chat_effort(v, a).unwrap().as_deref(),
            Some("max"),
            "a refusal keeps it"
        );
        // The import fills only chats without a level, skips bad rows.
        let kept = import_chat_efforts(
            v,
            &[
                (a.into(), "low".into()),
                (b.into(), "medium".into()),
                (c.into(), "turbo".into()),
                ("../x".into(), "low".into()),
            ],
        )
        .unwrap();
        assert_eq!(kept, 1);
        assert_eq!(
            chat_effort(v, a).unwrap().as_deref(),
            Some("max"),
            "never overwritten"
        );
        assert_eq!(chat_effort(v, b).unwrap().as_deref(), Some("medium"));
        assert_eq!(chat_effort(v, c).unwrap(), None);
        assert_eq!(
            chat_efforts(v).unwrap(),
            vec![
                (a.to_string(), "max".to_string()),
                (b.to_string(), "medium".to_string())
            ]
        );
        // A handover successor inherits when it has none, never replaces its own.
        inherit_chat_effort(v, a, c).unwrap();
        assert_eq!(chat_effort(v, c).unwrap().as_deref(), Some("max"));
        inherit_chat_effort(v, b, c).unwrap();
        assert_eq!(chat_effort(v, c).unwrap().as_deref(), Some("max"));
        inherit_chat_effort(v, "s-20261112-ef000099", "s-20261112-ef000098").unwrap();
        assert_eq!(
            chat_effort(v, "s-20261112-ef000098").unwrap(),
            None,
            "nothing to inherit"
        );
        // Default clears it.
        set_chat_effort(v, a, None).unwrap();
        assert_eq!(chat_effort(v, a).unwrap(), None);
    }

    /// Overnight 11-12 C8: a reply carries when its turn ended, so the
    /// chat's reply row has a time when no receipt came back; the request
    /// and a running reply have none.
    #[test]
    fn a_finished_reply_carries_when_its_turn_ended() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let key = "s-c8-ended";
        let ord = start_turn(v, key, "claude", "/vault", "run-c8-ended", "go").unwrap();
        append_event(v, key, ord, "assistant", "Done.").unwrap();
        let running = read_conversation(v, key).unwrap();
        assert!(running.turns.iter().all(|t| t.ended_at.is_none()));
        finish_turn(v, key, ord, "success").unwrap();
        let done = read_conversation(v, key).unwrap();
        assert_eq!(done.turns[0].role, "user");
        assert_eq!(done.turns[0].ended_at, None);
        let ended = done.turns[1].ended_at.as_deref().expect("the reply's end");
        assert!(
            chrono::DateTime::parse_from_rfc3339(ended).is_ok(),
            "{ended}"
        );
        let wire = serde_json::to_value(&done.turns[1]).unwrap();
        assert_eq!(wire["endedAt"], ended);
        assert!(serde_json::to_value(&done.turns[0])
            .unwrap()
            .get("endedAt")
            .is_none());
    }

    /// Self-hosted models Q7 review: the trail a self-hosted turn's note
    /// reads keeps the last `start` and its `back` however many `stayed`
    /// notes (one a self-hosted turn) and other notices came since, oldest
    /// first, and skips a payload that isn't JSON.
    #[test]
    fn the_remote_trail_reads_starts_and_returns_past_any_number_of_notes() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let key = "s-q7-trail";
        let ord = start_turn(v, key, "claude", "/vault", "run-q7-trail", "go").unwrap();
        let note =
            |phase: &str| json!({ "type": "remoteRun", "phase": phase, "host": "runner-host" });
        append_event(v, key, ord, "remoteRun", &note("start").to_string()).unwrap();
        append_event(v, key, ord, "remoteRun", &note("back").to_string()).unwrap();
        append_event(v, key, ord, "remoteRun", "not json").unwrap();
        for _ in 0..250 {
            append_event(v, key, ord, "remoteRun", &note("stayed").to_string()).unwrap();
        }
        append_event(v, key, ord, "accountSwitched", &note("start").to_string()).unwrap();
        let trail = remote_run_trail(v, key, 40).unwrap();
        assert_eq!(
            trail,
            vec![note("start").to_string(), note("back").to_string()]
        );
        // The plain read of the newest 200 has lost them.
        let newest = event_payloads(v, key, "remoteRun", 200).unwrap();
        assert!(!newest.contains(&note("start").to_string()));
        assert!(remote_run_trail(v, "s-q7-none", 40).unwrap().is_empty());
    }

    /// P1-30 review: a refused steer's reason survives a reload; a state
    /// change without one keeps none.
    #[test]
    fn a_stored_steer_state_keeps_the_agents_refusal_reason() {
        let mut transcript = StoredTranscript::default();
        for (id, text) in [("st-a", "and lint"), ("st-b", "and test")] {
            let record = json!({ "id": id, "text": text, "at": "2026-10-25T07:00:00Z", "afterTools": 0, "state": "sent" });
            apply_event(&mut transcript, "steer", &record.to_string());
        }
        let refused = crate::steering::SteerRecord {
            id: "st-a".into(),
            text: "and lint".into(),
            at: "2026-10-25T07:00:00Z".into(),
            after_tools: 0,
            state: crate::steering::SteerState::NotDelivered,
            reason: Some("no active turn to steer".into()),
            by: None,
            by_run: None,
        };
        apply_event(
            &mut transcript,
            "steerState",
            &crate::steering::state_payload(&refused),
        );
        apply_event(
            &mut transcript,
            "steerState",
            r#"{"id":"st-b","state":"notDelivered"}"#,
        );
        assert_eq!(
            transcript.steers[0].state,
            crate::steering::SteerState::NotDelivered
        );
        assert_eq!(
            transcript.steers[0].reason.as_deref(),
            Some("no active turn to steer")
        );
        assert_eq!(transcript.steers[1].reason, None);
    }

    /// Overnight F2: the rail's chains. A child task's conversation is
    /// started by the conversation whose turn recorded it; one started by
    /// none, a self-reference and a key nobody asked about are absent, and a
    /// re-delegated child keeps its newest starter.
    #[test]
    fn session_starters_name_the_conversation_whose_turn_started_each_child() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let now = db_time(Utc::now());
        start_turn(v, "s-f2-lane", "claude", "/tmp", "run-f2-lane", "Lane").unwrap();
        start_turn(v, "s-f2-next", "claude", "/tmp", "run-f2-next", "Successor").unwrap();
        start_turn(v, "s-f2-other", "claude", "/tmp", "run-f2-other", "Other").unwrap();
        start_turn(v, "s-f2-last", "claude", "/tmp", "run-f2-last-run", "Last").unwrap();
        let child = |key: &str| {
            format!(
                "{{\"kind\":\"childTask\",\"tool\":\"start_session\",\"childSession\":\"{key}\"}}"
            )
        };
        record_delegation(v, "run-f2-lane", &now, &child("s-f2-next"), 10).unwrap();
        record_delegation(v, "run-f2-next", &now, &child("s-f2-last"), 10).unwrap();
        record_delegation(v, "run-f2-lane", &now, &child("s-f2-lane"), 10).unwrap();
        record_delegation(
            v,
            "run-f2-lane",
            &now,
            "{\"kind\":\"consultation\",\"childSession\":\"s-f2-other\"}",
            10,
        )
        .unwrap();
        record_delegation(v, "run-f2-other", &now, &child("s-f2-moved"), 10).unwrap();
        record_delegation(v, "run-f2-next", &now, &child("s-f2-moved"), 10).unwrap();
        // A message to a chat that exists starts nothing.
        let sent =
            "{\"kind\":\"childTask\",\"tool\":\"send_to_session\",\"childSession\":\"s-f2-next\"}";
        record_delegation(v, "run-f2-other", &now, sent, 10).unwrap();
        let handed =
            "{\"kind\":\"childTask\",\"tool\":\"handover\",\"childSession\":\"s-f2-heir\"}";
        record_delegation(v, "run-f2-last-run", &now, handed, 10).unwrap();
        let keys: Vec<String> = [
            "s-f2-lane",
            "s-f2-next",
            "s-f2-last",
            "s-f2-other",
            "s-f2-moved",
            "s-f2-heir",
        ]
        .iter()
        .map(|k| k.to_string())
        .collect();
        let got = session_starters(v, &keys).unwrap();
        assert_eq!(got.get("s-f2-next").map(String::as_str), Some("s-f2-lane"));
        assert_eq!(got.get("s-f2-last").map(String::as_str), Some("s-f2-next"));
        assert_eq!(
            got.get("s-f2-moved").map(String::as_str),
            Some("s-f2-next"),
            "the newest record wins"
        );
        assert!(
            !got.contains_key("s-f2-lane"),
            "a self-reference is no chain"
        );
        assert!(
            !got.contains_key("s-f2-other"),
            "a consultation starts no conversation"
        );
        assert_eq!(
            got.get("s-f2-heir").map(String::as_str),
            Some("s-f2-last"),
            "a handover starts its successor"
        );
        assert_eq!(got.len(), 4, "a send_to_session reparents nothing: {got:?}");
        assert!(session_starters(v, &[]).unwrap().is_empty());
        assert!(session_starters(v, &["s-f2-nobody".to_string()])
            .unwrap()
            .is_empty());
    }

    /// Canvas C6 P1 (C4 review minor): the About-you builder reads the
    /// person's asks without an ask-canvas's hidden answer block; the stored
    /// turn keeps it (the composer is restored from it).
    #[test]
    fn recent_asks_drop_an_ask_canvas_answer_block() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let key = "s-20261005-asks";
        let asked = "Answered canvas \"a\":\n- Pick: A\n\n```hollow-canvas-answer\n{\"canvasId\":\"a\",\"revision\":1,\"answers\":{\"pick\":\"A\"}}\n```";
        start_turn(v, key, "claude", "/tmp", "run-a1", asked).unwrap();
        assert_eq!(
            recent_asks(v, 5).unwrap(),
            vec!["Answered canvas \"a\":\n- Pick: A".to_string()]
        );
        let conn = open(v).unwrap();
        let stored: String = conn
            .query_row("SELECT asked FROM turns WHERE run_id = 'run-a1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored, asked, "the turn keeps the block");
    }

    #[test]
    fn a_turns_permission_mode_is_recorded_and_the_latest_read_back() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let key = "s-20260928-permmode";
        let first = start_turn(v, key, "claude", "/tmp", "run-p1", "One").unwrap();
        assert_eq!(
            last_permission(v, key).unwrap(),
            None,
            "nothing recorded yet"
        );
        record_turn_permission(v, key, first, "plan").unwrap();
        let second = start_turn(v, key, "claude", "/tmp", "run-p2", "Two").unwrap();
        assert_eq!(
            last_permission(v, key).unwrap().as_deref(),
            Some("plan"),
            "an unrecorded turn is skipped"
        );
        record_turn_permission(v, key, second, "auto").unwrap();
        assert_eq!(last_permission(v, key).unwrap().as_deref(), Some("auto"));
        assert_eq!(
            read_conversation(v, key)
                .unwrap()
                .permission_mode
                .as_deref(),
            Some("auto")
        );
        assert!(last_permission(v, "../x").is_err());
    }

    /// A chat usage row for `run_id` asking for `model`, on native session
    /// `native` (`None`: the turn ended before its resume plan ran).
    fn chat_usage(v: &Path, run_id: &str, model: &str, native: Option<&str>) {
        let mut row = usage_row_fixture(&format!("u-{run_id}"), &db_time(Utc::now()), "success");
        row.run_id = Some(run_id.into());
        row.requested_model = Some(model.into());
        row.native_session = native.map(str::to_string);
        upsert_usage(v, &row).unwrap();
    }

    /// OB04's version restamp reads the turn before the running one that ran
    /// on the stint's native session: its mode, its chat usage row's model
    /// and agent, and the conversation's folder, never the running turn's
    /// own (already recorded) values.
    #[test]
    fn the_previous_turn_is_read_before_the_running_one() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let key = "s-20261016-previous";
        let first = start_turn(v, key, "claude", "/tmp/ws", "run-q1", "One").unwrap();
        assert_eq!(previous_turn(v, key, first, "sess-1").unwrap(), None);
        record_turn_permission(v, key, first, "bypassPermissions").unwrap();
        let second = start_turn(v, key, "claude", "/tmp/ws", "run-q2", "Two").unwrap();
        assert_eq!(
            previous_turn(v, key, second, "sess-1").unwrap(),
            None,
            "a turn without a usage row on that session"
        );
        chat_usage(v, "run-q1", "claude-opus-5-5", Some("sess-1"));
        // The running turn's own mode and usage row are never read.
        record_turn_permission(v, key, second, "plan").unwrap();
        chat_usage(v, "run-q2", "claude-sonnet-5", Some("sess-1"));
        let previous = previous_turn(v, key, second, "sess-1").unwrap();
        assert_eq!(
            previous,
            Some(PreviousTurn {
                permission_mode: Some("bypassPermissions".into()),
                requested: Some((Some("claude-opus-5-5".into()), Some("claude".into()))),
                session_cwd: Some("/tmp/ws".into()),
            })
        );
        assert_eq!(previous_turn(v, key, second, "sess-2").unwrap(), None);
        assert!(previous_turn(v, "../x", second, "sess-1").is_err());
    }

    /// OB04 review: a turn that failed before its resume plan ran (its mode
    /// and model recorded, its usage row naming no native session, no reset
    /// made) never stands for the stint. A v2 stint stamped under opus at
    /// plan, then a failed sonnet turn at bypass, then a sonnet turn at
    /// bypass: the restamp compares the opus plan turn, so it resets.
    #[test]
    fn an_early_failed_turn_never_lets_an_older_stamp_resume() {
        use crate::agent::AgentKind;
        use crate::chat_resume::{may_restamp, plan_resume_with, ResetReason, ResumePlan};
        let vault = temp_vault("early-failed-restamp");
        let v = vault.as_path();
        let key = "s-20261016-earlyfail";
        let cwd = Path::new("/tmp/ws");
        let stamped = start_turn(v, key, "claude", "/tmp/ws", "run-e1", "One").unwrap();
        record_turn_permission(v, key, stamped, "plan").unwrap();
        record_harness_id_with(v, key, "sess-1", Some("v2:aaa")).unwrap();
        chat_usage(v, "run-e1", "claude-opus-5-5", Some("sess-1"));
        let failed = start_turn(v, key, "claude", "/tmp/ws", "run-e2", "Two").unwrap();
        record_turn_permission(v, key, failed, "bypassPermissions").unwrap();
        chat_usage(v, "run-e2", "claude-sonnet-5", None);
        let now = start_turn(v, key, "claude", "/tmp/ws", "run-e3", "Three").unwrap();
        let state = resume_state(v, key).unwrap();
        let open = state.open.clone().expect("the v2 stint");
        let previous = previous_turn(v, key, now, "sess-1").unwrap();
        let prior = previous.as_ref();
        let opus = Some("claude-opus-5-5");
        let sonnet = Some("claude-sonnet-5");
        let reset = ResumePlan::Reset(ResetReason::ConfigChanged);
        for (perm, model) in [
            ("bypassPermissions", sonnet),
            ("bypassPermissions", opus),
            ("plan", sonnet),
        ] {
            let ok = may_restamp(&open, AgentKind::Claude, perm, model, cwd, prior);
            assert!(!ok, "{perm} {model:?}");
            let plan = plan_resume_with(Some("sess-1"), &state, "v3:now", ok);
            assert_eq!(plan, reset, "{perm} {model:?}");
        }
        // The turn that ran on the stint, unchanged, still restamps.
        let ok = may_restamp(&open, AgentKind::Claude, "plan", opus, cwd, prior);
        assert!(ok);
        let plan = plan_resume_with(Some("sess-1"), &state, "v3:now", ok);
        assert_eq!(plan, ResumePlan::Resume("sess-1".into()));
    }

    /// A finished (or running) usage row with the given key, `at` and outcome.
    pub(crate) fn usage_row_fixture(key: &str, at: &str, outcome: &str) -> UsageRow {
        UsageRow {
            run_key: key.to_string(),
            at: at.to_string(),
            started_at: at.to_string(),
            surface: "chat".into(),
            scope: None,
            parent_surface: None,
            run_id: None,
            agent: Some("claude".into()),
            requested_model: None,
            model: None,
            native_session: None,
            billing: "unknown".into(),
            outcome: outcome.to_string(),
            spawned: true,
            basis: "per_run".into(),
            input_tokens: None,
            cached_input_tokens: None,
            output_tokens: None,
            cost_usd: None,
            cost_status: "unknown".into(),
            usage_json: None,
            cost_floor_usd: None,
            workspace_id: None,
            account_ref: None,
        }
    }

    fn keys_since(vault: &Path, since: &str) -> Vec<String> {
        usage_rows_since(vault, since)
            .unwrap()
            .into_iter()
            .map(|row| row.run_key)
            .collect()
    }

    #[test]
    fn open_sets_a_busy_timeout_and_the_usage_table() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let conn = open(vault).unwrap();
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, 2000);
        drop(conn);
        let at = db_time(Utc::now());
        upsert_usage(vault, &usage_row_fixture("u-1", &at, "running")).unwrap();
        let mut finished = usage_row_fixture("u-1", &at, "success");
        finished.surface = "workflow".into(); // identity columns keep their first value
        finished.input_tokens = Some(7);
        upsert_usage(vault, &finished).unwrap();
        let rows = usage_rows_since(vault, "").unwrap();
        assert_eq!(rows.len(), 1, "a second upsert with the same key updates");
        assert_eq!(rows[0].outcome, "success");
        assert_eq!(rows[0].input_tokens, Some(7));
        assert_eq!(rows[0].surface, "chat");
        assert_eq!(
            at.len(),
            "2026-09-25T00:00:00.000Z".len(),
            "fixed-width db_time"
        );
        assert!(at.ends_with('Z'));
    }

    /// OB07: a ledger written before `usage_ledger.cost_floor_usd` gains the
    /// column on open (once), its rows read NULL, and an upsert writes the
    /// floor and replaces it (a run that learns its cost clears it).
    #[test]
    fn a_legacy_usage_ledger_gains_a_null_cost_floor_column() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let db = db_path_for(vault).unwrap();
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_ledger(id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 run_key TEXT NOT NULL UNIQUE, at TEXT NOT NULL, started_at TEXT NOT NULL, \
                 surface TEXT NOT NULL, scope TEXT, parent_surface TEXT, run_id TEXT, \
                 agent TEXT, requested_model TEXT, model TEXT, native_session TEXT, \
                 billing TEXT NOT NULL, outcome TEXT NOT NULL, spawned INTEGER NOT NULL, \
                 basis TEXT NOT NULL, input_tokens INTEGER, cached_input_tokens INTEGER, \
                 output_tokens INTEGER, cost_usd REAL, cost_status TEXT NOT NULL, \
                 usage_json TEXT); \
                 INSERT INTO usage_ledger(run_key, at, started_at, surface, agent, model, \
                 billing, outcome, spawned, basis, cached_input_tokens, output_tokens, \
                 cost_status) VALUES('old', '2026-10-16T12:00:00.000Z', \
                 '2026-10-16T12:00:00.000Z', 'chat', 'claude', 'claude-opus-5-5', \
                 'subscription', 'cancelled', 1, 'per_run', 1000000, 10000, 'partial');",
            )
            .unwrap();
        }
        let rows = usage_rows_since(vault, "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].cost_usd, rows[0].cost_floor_usd),
            (None, None),
            "an older row has no floor"
        );
        let conn = open(vault).unwrap();
        let floors = conn
            .prepare("PRAGMA table_info(usage_ledger)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|name| name == "cost_floor_usd")
            .count();
        assert_eq!(floors, 1, "added once, and a second open is a no-op");
        drop(conn);
        let at = db_time(Utc::now());
        let mut killed = usage_row_fixture("u-new", &at, "cancelled");
        killed.cost_status = "partial".into();
        killed.cost_floor_usd = Some(0.40);
        upsert_usage(vault, &killed).unwrap();
        let new_row = || {
            usage_rows_since(vault, "")
                .unwrap()
                .into_iter()
                .find(|r| r.run_key == "u-new")
                .unwrap()
        };
        let row = new_row();
        assert_eq!((row.cost_usd, row.cost_floor_usd), (None, Some(0.40)));
        killed.cost_usd = Some(0.55);
        killed.cost_status = "estimated".into();
        killed.cost_floor_usd = None;
        upsert_usage(vault, &killed).unwrap();
        let row = new_row();
        assert_eq!((row.cost_usd, row.cost_floor_usd), (Some(0.55), None));
    }

    /// Paperclip PR 17: a turn's recall paths live in its durable transcript,
    /// so they outlive the raw events; an older transcript without the field
    /// decodes to an empty list.
    #[test]
    fn recall_paths_survive_event_retention() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20260926-feed0001";
        let ord = start_turn(vault, key, "claude", "/vault", "run-recall", "Ask").unwrap();
        let long = "x".repeat(RECALL_PATH_CHARS + 1);
        let mut paths = vec![String::new(), long, "learnings/a.md".to_string()];
        paths.extend((0..20).map(|n| format!("learnings/n{n}.md")));
        record_recall(vault, key, ord, &paths).unwrap();
        record_recall(vault, key, ord, &[]).unwrap(); // no-op, keeps the list
        record_recall(vault, key, 0, &paths).unwrap(); // no turn row: no-op
        open(vault)
            .unwrap()
            .execute("DELETE FROM events", [])
            .unwrap();
        let FeedbackTurnLookup::Found(source) = feedback_turn_source(vault, "run-recall").unwrap()
        else {
            panic!("the turn exists");
        };
        assert_eq!(source.recall.len(), MAX_RECALL_PATHS);
        assert_eq!(source.recall[0], "learnings/a.md");
        assert_eq!(source.asked, "Ask");
        assert_eq!(source.agent, "claude");

        let old: StoredTranscript =
            serde_json::from_str(r#"{"agent":"codex","text":"hi","tools":[]}"#).unwrap();
        assert!(old.recall.is_empty());
        assert!(matches!(
            feedback_turn_source(vault, "no-such-run").unwrap(),
            FeedbackTurnLookup::Missing
        ));
    }

    /// Merged plan slice 3 (B·F10): a chat run nothing will finish has its
    /// still-running turn and usage rows (its own and a delegate's) closed
    /// together; closed turns and rows, and other runs, keep their status.
    #[test]
    fn interrupt_chat_run_in_closes_only_that_runs_running_turn_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20260925-cccc0002";
        let live = start_turn(vault, key, "claude", "/vault", "run-live", "One").unwrap();
        let other = start_turn(vault, key, "claude", "/vault", "run-other", "Two").unwrap();
        let closed = start_turn(vault, key, "claude", "/vault", "run-closed", "Three").unwrap();
        finish_turn(vault, key, closed, "success").unwrap();
        let at = db_time(Utc::now());
        for (row_key, run, outcome, surface) in [
            ("u-live", "run-live", "running", "chat"),
            ("u-live-delegate", "run-live", "running", "delegate"),
            ("u-other", "run-other", "running", "chat"),
            ("u-closed", "run-closed", "success", "chat"),
        ] {
            let mut row = usage_row_fixture(row_key, &at, outcome);
            row.run_id = Some(run.to_string());
            row.surface = surface.to_string();
            upsert_usage(vault, &row).unwrap();
        }
        let interrupt = |run: &str| {
            with_write(vault, "test", |tx| {
                interrupt_chat_run_in(tx, run, Utc::now())
            })
            .unwrap()
        };
        assert_eq!(interrupt("run-live"), InterruptedRun { turns: 1, usage: 2 });
        assert_eq!(interrupt("run-closed"), InterruptedRun::default());
        assert_eq!(interrupt(" "), InterruptedRun::default());
        assert_eq!(
            interrupt("run-live"),
            InterruptedRun::default(),
            "idempotent"
        );
        let status = |ord: i64| -> String {
            open(vault)
                .unwrap()
                .query_row(
                    "SELECT status FROM turns WHERE session_key = ?1 AND ord = ?2",
                    params![key, ord],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(status(live), "interrupted");
        assert_eq!(status(other), "running");
        assert_eq!(status(closed), "success");
        let outcome = |row_key: &str| -> String {
            open(vault)
                .unwrap()
                .query_row(
                    "SELECT outcome FROM usage_ledger WHERE run_key = ?1",
                    params![row_key],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(outcome("u-live"), "interrupted");
        assert_eq!(outcome("u-live-delegate"), "interrupted");
        assert_eq!(outcome("u-other"), "running");
        assert_eq!(outcome("u-closed"), "success");
    }

    /// A second connection holds the write lock the way a `hollow team-mcp`
    /// delegate's usage write does. Every read-then-write waits for it
    /// instead of failing.
    #[test]
    fn append_event_waits_for_another_connections_writer() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().to_path_buf();
        let key = "s-20260925-cccc0001";
        let ord = start_turn(&vault, key, "claude", "/vault", "run-lock", "Hold on").unwrap();
        for (name, op) in [("append_event", 0), ("start_turn", 1), ("retain", 2)] {
            let path = db_path_for(&vault).unwrap();
            // Current, so the retention pass in the `retain` case keeps it.
            let at = db_time(Utc::now());
            let (locked_tx, locked_rx) = std::sync::mpsc::channel();
            let holder = std::thread::spawn(move || {
                let other = Connection::open(&path).unwrap();
                other.execute_batch("BEGIN IMMEDIATE").unwrap();
                other
                    .execute(
                        "INSERT INTO usage_ledger(run_key, at, started_at, surface, billing, \
                         outcome, spawned, basis, cost_status) VALUES(?1, ?2, ?2, 'delegate', \
                         'unknown', 'running', 1, 'per_run', 'unknown')",
                        params![format!("other-{name}"), at],
                    )
                    .unwrap();
                locked_tx.send(()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(300));
                other.execute_batch("COMMIT").unwrap();
            });
            locked_rx.recv().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
            let result = match op {
                0 => append_event(&vault, key, ord, "toolUse", r#"{"id":"t","name":"Read"}"#),
                1 => start_turn(&vault, key, "claude", "/vault", "run-lock-2", "Again").map(|_| ()),
                _ => retain(&vault),
            };
            holder.join().unwrap();
            assert!(result.is_ok(), "{name} failed under contention: {result:?}");
        }
        assert!(read_session(&vault, key).unwrap().events_retained);
        assert_eq!(usage_rows_since(&vault, "").unwrap().len(), 3);
    }

    #[test]
    fn usage_rows_prune_after_400_days_but_never_while_running() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let now = Utc::now();
        let old = db_time(now - Duration::days(KEEP_USAGE_DAYS + 1));
        let recent = db_time(now - Duration::days(KEEP_USAGE_DAYS - 1));
        upsert_usage(vault, &usage_row_fixture("old-done", &old, "success")).unwrap();
        upsert_usage(vault, &usage_row_fixture("old-running", &old, "running")).unwrap();
        upsert_usage(vault, &usage_row_fixture("recent", &recent, "error")).unwrap();
        // Event retention alone (a chat turn's finish) runs the usage prune too.
        let key = "s-20260925-cccc0002";
        let ord = start_turn(vault, key, "claude", "/vault", "run-p", "x").unwrap();
        finish_turn(vault, key, ord, "success").unwrap();
        let mut keys = keys_since(vault, "");
        keys.sort();
        assert_eq!(keys, vec!["old-running".to_string(), "recent".to_string()]);
        // Event retention never deletes usage rows.
        open(vault)
            .unwrap()
            .execute("UPDATE events SET at = '2000-01-01T00:00:00Z'", [])
            .unwrap();
        retain(vault).unwrap();
        assert_eq!(keys_since(vault, "").len(), 2);
        assert_eq!(
            prune_usage(vault, &db_time(now)).unwrap(),
            1,
            "running rows survive"
        );
    }

    /// A writer that panics while holding the write lock poisons it. Before
    /// the recovery every later write failed with "session store lock
    /// poisoned" until restart: usage rows, turns and the run queue all
    /// stopped (review finding A·R7).
    #[test]
    fn a_poisoned_write_lock_still_writes() {
        let _ = std::thread::spawn(|| {
            let _guard = WRITE_LOCK.lock();
            panic!("a writer panics while holding the session store lock");
        })
        .join();
        assert!(WRITE_LOCK.is_poisoned(), "the lock should now be poisoned");
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let at = db_time(Utc::now());
        upsert_usage(vault, &usage_row_fixture("after-poison", &at, "success"))
            .expect("a usage write after the panic");
        let key = "s-20261002-dddd0001";
        let ord = start_turn(vault, key, "claude", "/vault", "run-poison", "x")
            .expect("a transactional write after the panic");
        finish_turn(vault, key, ord, "success").expect("a turn finish after the panic");
        assert_eq!(keys_since(vault, ""), vec!["after-poison".to_string()]);
    }

    /// OB24: only a row that stored a terminal is a baseline; the finished
    /// rows after it (a cancelled turn's `terminal: null`, an interrupted
    /// row with no `usage_json`) are listed, oldest first, for the span.
    #[test]
    fn previous_terminal_finds_the_newest_earlier_terminal_and_lists_the_rows_it_skips() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let now = Utc::now();
        // The terminal's JSON text; t0's `{` leaves its document malformed.
        for (key, minutes, outcome, terminal) in [
            ("t0", 50, "success", Some("{")),
            ("t1", 40, "success", Some("{}")),
            ("t2", 30, "error", Some("{}")),
            ("t3", 20, "cancelled", Some("null")),
            ("t3b", 15, "interrupted", None),
            ("t4", 10, "running", Some("{}")),
        ] {
            let mut row =
                usage_row_fixture(key, &db_time(now - Duration::minutes(minutes)), outcome);
            row.native_session = Some("sess-1".into());
            row.usage_json = terminal.map(|t| format!("{{\"key\":\"{key}\",\"terminal\":{t}}}"));
            upsert_usage(vault, &row).unwrap();
        }
        let found = |before: &str, exclude: &str| {
            previous_terminal(vault, "claude", "sess-1", before, exclude)
                .unwrap()
                .map(|b| (b.usage_json, b.skipped_runs))
        };
        let t2 = "{\"key\":\"t2\",\"terminal\":{}}".to_string();
        let before = db_time(now);
        assert_eq!(
            found(&before, "u-self"),
            Some((t2.clone(), vec!["t3".to_string(), "t3b".to_string()])),
            "the newest finished terminal, not running, skipping rows without one"
        );
        assert_eq!(
            found(&before, "t3"),
            Some((t2, vec!["t3b".to_string()])),
            "the asking row is excluded"
        );
        let early = db_time(now - Duration::minutes(35));
        assert_eq!(
            found(&early, "u"),
            Some(("{\"key\":\"t1\",\"terminal\":{}}".to_string(), vec![])),
            "malformed JSON is never a baseline, and never an error"
        );
        let earliest = db_time(now - Duration::minutes(45));
        assert_eq!(found(&earliest, "u"), None);
        let priced = |before: &str| {
            previous_terminal(vault, "claude", "sess-1", before, "u")
                .unwrap()
                .map(|b| b.skipped_priced)
        };
        assert_eq!(priced(&before), Some(false), "no skipped row has a cost");
        // A cancelled turn a turn guard priced: its estimate is its cost.
        let mut guarded =
            usage_row_fixture("t3c", &db_time(now - Duration::minutes(12)), "cancelled");
        guarded.native_session = Some("sess-1".into());
        guarded.usage_json = Some("{\"terminal\":null}".into());
        guarded.cost_usd = Some(0.5);
        guarded.cost_status = "partial".into();
        upsert_usage(vault, &guarded).unwrap();
        assert_eq!(priced(&before), Some(true), "a skipped row has a cost");
        assert_eq!(priced(&early), Some(false), "t3c is after `early`");
        assert_eq!(
            previous_terminal(vault, "codex", "sess-1", &before, "u").unwrap(),
            None
        );
        assert_eq!(
            previous_terminal(vault, "claude", "sess-2", &before, "u").unwrap(),
            None
        );
    }

    fn temp_vault(name: &str) -> PathBuf {
        let dir = crate::test_temp::process_dir("hollow-sessionstore-vault-").join(name);
        std::fs::create_dir_all(&dir).expect("temp vault");
        dir
    }

    #[test]
    fn db_path_stays_out_of_the_real_app_data_dir() {
        let db = db_path_for(Path::new("/tmp/some-vault")).expect("a db path");
        assert!(
            db.starts_with(std::env::temp_dir()),
            "test db path should live under the temp dir, got {db:?}"
        );
        if let Some(real) = dirs::data_dir() {
            assert!(
                !db.starts_with(real.join("Hollow")),
                "test db path must not sit under the live app data dir, got {db:?}"
            );
        }
    }

    /// Every open runs the schema batch. Before the busy timeout, an open that
    /// needed a write while another connection held one failed at once with
    /// "database is locked" instead of waiting.
    #[test]
    fn open_waits_for_a_concurrent_writer_instead_of_failing_the_schema() {
        let vault = temp_vault("busy-open");
        let path = db_path_for(&vault).expect("a db path");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("store dir");
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let holder_path = path.clone();
        let holder = std::thread::spawn(move || {
            // A fresh file with no schema: the next open must CREATE, which
            // needs the write lock this transaction holds.
            let mut conn = Connection::open(&holder_path).expect("holder open");
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .expect("holder begins");
            ready_tx.send(()).expect("signal");
            std::thread::sleep(std::time::Duration::from_millis(300));
            tx.commit().expect("holder commits");
        });
        ready_rx.recv().expect("holder ready");
        let started = std::time::Instant::now();
        let conn = open(&vault).expect("open waits for the writer instead of failing the schema");
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(200),
            "open returned before the writer committed: it did not wait"
        );
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'run_queue'",
                [],
                |row| row.get(0),
            )
            .expect("schema readable");
        assert_eq!(tables, 1);
        holder.join().expect("holder thread");
    }

    #[test]
    fn with_write_commits_only_on_ok_and_db_time_is_fixed_width() {
        let vault = temp_vault("with-write");
        let failed: Result<(), String> = with_write(&vault, "test", |tx| {
            tx.execute(
                "INSERT INTO run_queue(source, label, status, priority, requested_at) \
                 VALUES('cron', 'x', 'queued', 3, '2026-01-01T00:00:00.000Z')",
                [],
            )
            .map_err(|e| e.to_string())?;
            Err("roll back".into())
        });
        assert_eq!(failed, Err("roll back".to_string()));
        let rows: i64 = with_read(&vault, |conn| {
            conn.query_row("SELECT COUNT(*) FROM run_queue", [], |row| row.get(0))
                .map_err(|e| e.to_string())
        })
        .expect("count");
        assert_eq!(
            rows, 0,
            "an Err from the closure rolls the transaction back"
        );

        let early = DateTime::parse_from_rfc3339("2026-09-25T09:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let later = early + Duration::milliseconds(1);
        assert_eq!(db_time(early), "2026-09-25T09:05:00.000Z");
        assert_eq!(db_time(early).len(), db_time(later).len());
        assert!(db_time(early) < db_time(later));
        let lock = pump_lock_path(&vault).expect("a lock path");
        assert!(lock.to_string_lossy().ends_with(".pump.lock"));
        assert!(
            !lock.starts_with(&vault),
            "the owner lock never sits in the vault"
        );
    }

    #[test]
    fn the_db_file_is_named_by_the_shared_vault_stem() {
        let vault = temp_vault("stem");
        let db = db_path_for(&vault).expect("a db path");
        let stem = vault_file_stem(&vault);
        assert_eq!(
            db.file_name().and_then(|n| n.to_str()),
            Some(format!("{stem}.db").as_str())
        );
        // folder name, then 16 hex digits of the canonical path's sha256
        let (name, hash) = stem.rsplit_once('-').expect("name-hash");
        assert!(name.ends_with("stem"), "{stem}");
        assert_eq!(hash.len(), 16);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{stem}");
    }

    #[test]
    fn the_store_is_never_inside_the_vault() {
        let vault = temp_vault("outside");
        let db = db_path_for(&vault).expect("a db path");
        assert!(
            !db.starts_with(&vault),
            "the derived store must not live in the vault, got {db:?}"
        );
    }

    #[test]
    fn exact_lookup_reaches_old_sessions_and_rejects_ambiguous_runs() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20260923-old00001";
        start_turn(vault, key, "claude", "/vault", "old-run", "Old session").unwrap();
        let conn = open(vault).unwrap();
        conn.execute("UPDATE sessions SET last_activity_at = '2000-01-01'", [])
            .unwrap();
        conn.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 1001) INSERT INTO sessions(key, agent, cwd, created_at, last_activity_at) SELECT 's-20260923-new' || x, 'claude', '/vault', '2026-09-23', '2026-09-23' FROM n").unwrap();
        assert!(!list_sessions(vault, 1000)
            .unwrap()
            .iter()
            .any(|s| s.key == key));
        assert_eq!(read_session(vault, key).unwrap().summary.key, key);
        assert_eq!(session_for_run(vault, "old-run").unwrap().unwrap().key, key);
        assert!(assistant_summaries(vault, key).unwrap().is_empty());
        start_turn(
            vault,
            "s-20260923-other001",
            "codex",
            "/vault",
            "old-run",
            "Other",
        )
        .unwrap();
        assert!(session_for_run(vault, "old-run")
            .unwrap_err()
            .contains("ambiguous"));
    }

    #[test]
    fn turn_round_trip_lists_and_reads() {
        let vault = temp_vault("roundtrip");
        let key = "s-20260811-aaaa0001";
        let ord = start_turn(
            &vault,
            key,
            "claude",
            "/vault",
            "run-1",
            "Fix the rail\nmore",
        )
        .expect("start turn");
        assert_eq!(ord, 1);
        record_harness_id(&vault, key, "claude-sess-1").expect("harness id");
        append_event(&vault, key, ord, "toolUse", "{\"name\":\"Read\"}").expect("event");
        finish_turn(&vault, key, ord, "success").expect("finish");

        let second =
            start_turn(&vault, key, "claude", "/vault", "run-2", "And again").expect("second turn");
        assert_eq!(second, 2, "ordinals increment per session");
        finish_turn(&vault, key, second, "success").expect("finish 2");

        let listed = list_sessions(&vault, 50).expect("list");
        let found = listed
            .iter()
            .find(|summary| summary.key == key)
            .expect("session listed");
        assert_eq!(found.turn_count, 2);
        assert_eq!(found.title.as_deref(), Some("Fix the rail"));
        assert_eq!(found.harness_session_id.as_deref(), Some("claude-sess-1"));
        assert_eq!(found.resumable, "native");
        assert_eq!(found.stints.len(), 1, "same agent stays one stint");

        let detail = read_session(&vault, key).expect("detail");
        assert_eq!(detail.turns.len(), 2);
        assert_eq!(detail.turns[0].status.as_deref(), Some("success"));
        assert!(detail.events_retained);
    }

    #[test]
    fn changing_agent_opens_a_new_stint_under_one_key() {
        let vault = temp_vault("stints");
        let key = "s-20260811-aaaa0002";
        let a = start_turn(&vault, key, "claude", "/vault", "run-a", "one").expect("turn a");
        record_harness_id(&vault, key, "claude-1").expect("id a");
        finish_turn(&vault, key, a, "success").expect("finish a");

        let b = start_turn(&vault, key, "codex", "/vault", "run-b", "two").expect("turn b");
        finish_turn(&vault, key, b, "success").expect("finish b");

        let listed = list_sessions(&vault, 50).expect("list");
        let found = listed
            .iter()
            .find(|summary| summary.key == key)
            .expect("session");
        assert_eq!(found.stints.len(), 2, "a harness change opens a stint");
        assert_eq!(found.stints[0].agent, "claude");
        assert_eq!(
            found.stints[0].harness_session_id.as_deref(),
            Some("claude-1")
        );
        assert!(
            found.stints[0].ended_at.is_some(),
            "the prior stint is closed"
        );
        assert_eq!(found.stints[1].agent, "codex");
        assert_eq!(found.agent, "codex", "the session's agent follows the move");
    }

    #[test]
    fn antigravity_legacy_sentinel_is_brief_only() {
        let vault = temp_vault("antigravity");
        let key = "s-20260811-aaaa0003";
        let ord = start_turn(&vault, key, "antigravity", "/vault", "run-x", "hi").expect("turn");
        // Older Hollow builds stamped the provider label as an id; it never
        // identifies a native conversation and must remain brief-only.
        record_harness_id(&vault, key, "antigravity").expect("id");
        finish_turn(&vault, key, ord, "success").expect("finish");
        let listed = list_sessions(&vault, 50).expect("list");
        let found = listed.iter().find(|s| s.key == key).expect("session");
        assert_eq!(found.resumable, "briefOnly");
    }

    #[test]
    fn unsafe_keys_never_reach_sql() {
        let vault = temp_vault("unsafe");
        let error = start_turn(&vault, "../etc/passwd", "claude", "/vault", "r", "x")
            .expect_err("must refuse");
        assert!(error.contains("unsafe session key"), "got {error}");
        assert!(read_session(&vault, "s-2026/08").is_err());
        // The write helpers swallow rather than error, but must not insert.
        append_event(&vault, "bad key", 1, "toolUse", "{}").expect("no-op");
        let listed = list_sessions(&vault, 50).expect("list");
        assert!(listed.iter().all(|s| is_safe_session_key(&s.key)));
    }

    #[test]
    fn events_are_capped_and_status_reports_the_store() {
        let vault = temp_vault("caps");
        let key = "s-20260811-aaaa0004";
        let ord = start_turn(&vault, key, "claude", "/vault", "run-c", "hi").expect("turn");
        let huge = "x".repeat(EVENT_PAYLOAD_CHARS * 3);
        append_event(&vault, key, ord, "toolResult", &huge).expect("event");
        finish_turn(&vault, key, ord, "success").expect("finish");
        let reported = status(&vault).expect("status");
        assert!(reported.sessions >= 1);
        assert!(reported.events_retained >= 1);
        assert!(reported.bytes > 0);
        assert!(
            !reported.path.is_empty() && !reported.path.starts_with(&*vault.to_string_lossy()),
            "status must report a path outside the vault, got {}",
            reported.path
        );
    }

    #[test]
    fn durable_conversation_survives_event_retention_and_keeps_provider_identity() {
        let vault = temp_vault("durable-transcript");
        let key = "s-20260907-bbbb0001";
        for (agent, text, ordinal) in [
            ("claude", "First answer", 1),
            ("codex", "Second answer", 2),
            ("antigravity", "Third answer", 3),
        ] {
            let ord = start_turn(
                &vault,
                key,
                agent,
                "/vault",
                &format!("run-{ordinal}"),
                "Question",
            )
            .unwrap();
            if agent != "antigravity" {
                record_harness_id(&vault, key, &format!("{agent}-native-id")).unwrap();
            }
            append_event(
                &vault,
                key,
                ord,
                "toolUse",
                r#"{"id":"read-1","name":"Read","input":{"path":"note.md"}}"#,
            )
            .unwrap();
            append_event(
                &vault,
                key,
                ord,
                "toolResult",
                r#"{"id":"read-1","content":"A note","isError":false}"#,
            )
            .unwrap();
            append_event(&vault, key, ord, "assistant", text).unwrap();
            append_event(
                &vault,
                key,
                ord,
                "result",
                r#"{"costUsd":0.125,"isError":false}"#,
            )
            .unwrap();
            finish_turn(&vault, key, ord, "success").unwrap();
        }
        // Simulate a later restart/retention sweep, with no in-memory state.
        open(&vault)
            .unwrap()
            .execute("UPDATE events SET at = '2000-01-01T00:00:00Z'", [])
            .unwrap();
        retain(&vault).unwrap();
        let restored = read_conversation(&vault, key).unwrap();
        assert!(!restored.events_retained);
        assert_eq!(restored.turns.len(), 6);
        for (index, (agent, text)) in [
            ("claude", "First answer"),
            ("codex", "Second answer"),
            ("antigravity", "Third answer"),
        ]
        .iter()
        .enumerate()
        {
            let answer = &restored.turns[index * 2 + 1];
            assert_eq!(answer.agent, *agent);
            assert_eq!(answer.text, *text);
            assert_eq!(answer.tools[0].result.as_deref(), Some("A note"));
            assert_eq!(answer.tools[0].status, "done");
            assert_eq!(answer.cost_usd, Some(0.125));
            assert_eq!(
                answer.run_id.as_deref(),
                Some(format!("run-{}", index + 1).as_str())
            );
            assert!(!answer.transcript_incomplete);
        }
        assert_eq!(restored.summary.resumable, "briefOnly");
        assert_eq!(
            restored.summary.stints[1].harness_session_id.as_deref(),
            Some("codex-native-id")
        );
    }

    #[test]
    fn structured_event_cap_preserves_json_tool_identity_and_results() {
        let vault = temp_vault("json-cap");
        let key = "s-20260907-bbbb0002";
        let ord = start_turn(&vault, key, "codex", "/vault", "run-json", "Read it").unwrap();
        let large = "\"\\\n🦉".repeat(5000);
        for (kind, payload) in [
            (
                "toolUse",
                json!({"id":"tool-id","name":"Read","input":{"body":large}}).to_string(),
            ),
            (
                "toolResult",
                json!({"id":"tool-id","content":large,"isError":true}).to_string(),
            ),
        ] {
            let bounded = bounded_event(kind, &payload);
            let json: Value = serde_json::from_str(&bounded).expect("cap must preserve JSON");
            assert!(bounded.chars().count() <= EVENT_PAYLOAD_CHARS);
            assert_eq!(json["id"], "tool-id");
            append_event(&vault, key, ord, kind, &payload).unwrap();
        }
        finish_turn(&vault, key, ord, "error").unwrap();
        let restored = read_conversation(&vault, key).unwrap();
        let answer = &restored.turns[1];
        assert_eq!(answer.tools[0].name, "Read");
        assert_eq!(answer.tools[0].status, "error");
        assert!(answer.tools[0]
            .result
            .as_ref()
            .unwrap()
            .contains("truncated"));
        assert!(answer.transcript_incomplete);
    }

    #[test]
    fn legacy_events_migrate_before_pruning_and_missing_history_is_explicit() {
        let vault = temp_vault("legacy-migration");
        let key = "s-20260907-bbbb0003";
        let ord = start_turn(&vault, key, "codex", "/vault", "legacy-run", "Old question").unwrap();
        append_event(&vault, key, ord, "assistant", "Retained legacy answer").unwrap();
        finish_turn(&vault, key, ord, "success").unwrap();
        let missing = start_turn(
            &vault,
            key,
            "codex",
            "/vault",
            "missing-run",
            "Missing answer",
        )
        .unwrap();
        finish_turn(&vault, key, missing, "cancelled").unwrap();
        let conn = open(&vault).unwrap();
        conn.execute("DELETE FROM transcripts", []).unwrap();
        conn.execute("UPDATE events SET at = '2000-01-01T00:00:00Z'", [])
            .unwrap();
        drop(conn);
        retain(&vault).unwrap();
        let restored = read_conversation(&vault, key).unwrap();
        assert!(!restored.events_retained);
        assert_eq!(restored.turns[1].text, "Retained legacy answer");
        assert_eq!(restored.turns[1].agent, "codex");
        assert!(!restored.turns[1].transcript_incomplete);
        assert!(restored.turns[3].transcript_incomplete);
        assert_eq!(restored.turns[3].status.as_deref(), Some("cancelled"));
    }

    #[test]
    fn archiving_is_reversible_and_keeps_transcript_and_native_resume_id() {
        let vault = temp_vault("archive");
        let key = "s-20260907-bbbb0004";
        let ord = start_turn(&vault, key, "codex", "/vault", "archive-run", "Keep this").unwrap();
        record_harness_id(&vault, key, "native-thread").unwrap();
        append_event(&vault, key, ord, "assistant", "Answer to keep").unwrap();
        finish_turn(&vault, key, ord, "success").unwrap();
        archive_session(&vault, key, true).unwrap();
        assert!(list_sessions(&vault, 200).unwrap().is_empty());
        assert_eq!(
            list_sessions_by_archive(&vault, 200, true).unwrap()[0].key,
            key
        );
        let restored = read_conversation(&vault, key).unwrap();
        assert_eq!(
            restored.summary.harness_session_id.as_deref(),
            Some("native-thread")
        );
        assert_eq!(restored.turns[1].text, "Answer to keep");
        archive_session(&vault, key, false).unwrap();
        assert_eq!(list_sessions(&vault, 200).unwrap()[0].key, key);
        assert!(list_sessions_by_archive(&vault, 200, true)
            .unwrap()
            .is_empty());
        assert!(archive_session(&vault, "../bad", true).is_err());
    }

    /// P1-39: archiving a conversation with a turn queued or running is
    /// refused (it used to archive and stop its services anyway); restoring
    /// one never is.
    #[test]
    fn archiving_refuses_a_live_or_queued_turn() {
        let vault = temp_vault("archive-live");
        let key = "s-20261025-cccc0039";
        let ord = start_turn(&vault, key, "claude", "/vault", "archive-live-run", "Busy").unwrap();
        finish_turn(&vault, key, ord, "success").unwrap();
        let queued = crate::run_queue::enqueue(
            &vault,
            crate::run_queue::NewRow::new(
                crate::run_queue::RunSource::Chat,
                Some(key),
                "Chat",
                "chat",
            )
            .with_payload(serde_json::json!({ "agent": "claude", "clientTurnId": "ct-39" })),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(
            archive_session(&vault, key, true).unwrap_err(),
            ARCHIVE_BUSY
        );
        crate::run_queue::cancel(&vault, queued.id(), Utc::now()).unwrap();
        {
            let _live = crate::agent_sessions::LIVE_TEST_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            crate::agent_sessions::begin(
                key,
                crate::agent::AgentKind::Claude,
                "run-live-ar39",
                crate::agent_sessions::SessionPhase::Running,
            );
            assert_eq!(
                archive_session(&vault, key, true).unwrap_err(),
                ARCHIVE_BUSY
            );
            // Restoring is never refused.
            archive_session(&vault, key, false).unwrap();
            crate::agent_sessions::clear_run("run-live-ar39");
        }
        assert!(list_sessions_by_archive(&vault, 200, true)
            .unwrap()
            .is_empty());
        archive_session(&vault, key, true).unwrap();
        assert_eq!(
            list_sessions_by_archive(&vault, 200, true).unwrap()[0].key,
            key
        );
    }

    #[test]
    fn unknown_providers_never_claim_native_resume_capability() {
        assert_eq!(resumability("unknown", Some("some-id")), "briefOnly");
        assert_eq!(resumability("codex", Some("thread-id")), "native");
        assert_eq!(resumability("claude", Some("session-id")), "native");
        assert_eq!(resumability("codex", None), "briefOnly");
        assert_eq!(
            resumability("antigravity", Some("native-agy-session")),
            "native"
        );
        assert_eq!(
            resumability("antigravity", Some("antigravity")),
            "briefOnly"
        );
    }

    /// Paperclip PR 14b: a database created before `stints.config_fingerprint`
    /// (and before `sessions.archived`) gains both columns on open, the
    /// legacy stint reads as fingerprint unknown, and opening again is a
    /// no-op. OB03: `stints.config_parts` migrates the same way, NULL.
    #[test]
    fn legacy_stints_gain_fingerprint_column_and_stay_null() {
        let vault = temp_vault("legacy-fingerprint");
        let path = db_path_for(&vault).expect("db path");
        let _ = std::fs::remove_file(&path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions( \
                   key TEXT PRIMARY KEY, title TEXT, agent TEXT NOT NULL, cwd TEXT NOT NULL, \
                   created_at TEXT NOT NULL, last_activity_at TEXT NOT NULL, ended_at TEXT, \
                   turn_count INTEGER NOT NULL DEFAULT 0, parent_key TEXT, fork_kind TEXT, \
                   adopted INTEGER NOT NULL DEFAULT 0); \
                 CREATE TABLE stints( \
                   id INTEGER PRIMARY KEY AUTOINCREMENT, session_key TEXT NOT NULL, agent TEXT NOT NULL, \
                   harness_session_id TEXT, started_at TEXT NOT NULL, ended_at TEXT, reason TEXT); \
                 INSERT INTO sessions(key, agent, cwd, created_at, last_activity_at) \
                   VALUES('s-20260926-14b0aaaa', 'claude', '/vault', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z'); \
                 INSERT INTO stints(session_key, agent, harness_session_id, started_at) \
                   VALUES('s-20260926-14b0aaaa', 'claude', 'legacy-sess', '2026-09-01T00:00:00Z');",
            )
            .unwrap();
        }
        let key = "s-20260926-14b0aaaa";
        let state = resume_state(&vault, key).expect("migrated read");
        let legacy = state.open.expect("the legacy stint is open");
        assert_eq!(legacy.harness_session_id.as_deref(), Some("legacy-sess"));
        assert_eq!(legacy.config_fingerprint, None, "NULL means unknown");
        assert_eq!(legacy.config_parts, None, "no parts: a reset names unknown");
        assert_eq!(state.last_closed, None);
        // A second open is idempotent, and sessions.archived migrated too.
        let conn = open(&vault).expect("reopen");
        let columns = |table: &str| -> Vec<String> {
            conn.prepare(&format!("PRAGMA table_info({table})"))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .filter_map(Result::ok)
                .collect()
        };
        for column in ["config_fingerprint", "config_parts"] {
            let stints = columns("stints");
            assert_eq!(
                stints.iter().filter(|c| *c == column).count(),
                1,
                "{column}"
            );
        }
        assert_eq!(
            columns("sessions")
                .iter()
                .filter(|c| *c == "archived")
                .count(),
            1
        );
        archive_session(&vault, key, true).expect("archived works on the migrated table");
    }

    /// Paperclip PR 14b: the fingerprint is written with a new native id,
    /// a restart closes the stint with its reason (and opens a fresh one), and
    /// the backfill fills only a NULL fingerprint on the matching id.
    #[test]
    fn record_restart_stamp_and_resume_state() {
        let vault = temp_vault("restart-stamp");
        let key = "s-20260926-14b0bbbb";
        let a = start_turn(&vault, key, "claude", "/vault", "run-a", "one").expect("turn");
        record_harness_id_with(&vault, key, "sess-1", Some("v1:aaa")).expect("id with fp");
        let open = resume_state(&vault, key).unwrap().open.unwrap();
        assert_eq!(open.harness_session_id.as_deref(), Some("sess-1"));
        assert_eq!(open.config_fingerprint.as_deref(), Some("v1:aaa"));
        // The plain recorder keeps the fingerprint; the explicit None clears it.
        record_harness_id(&vault, key, "sess-1").unwrap();
        assert_eq!(
            resume_state(&vault, key)
                .unwrap()
                .open
                .unwrap()
                .config_fingerprint
                .as_deref(),
            Some("v1:aaa")
        );
        record_harness_id_with(&vault, key, "sess-1", None).unwrap();
        assert_eq!(
            resume_state(&vault, key)
                .unwrap()
                .open
                .unwrap()
                .config_fingerprint,
            None
        );

        // Backfill: only the matching id, only NULL.
        assert!(!stamp_config_fingerprint(&vault, key, "other", "v1:bbb").unwrap());
        assert!(stamp_config_fingerprint(&vault, key, "sess-1", "v1:bbb").unwrap());
        assert!(
            !stamp_config_fingerprint(&vault, key, "sess-1", "v1:ccc").unwrap(),
            "never overwrites"
        );
        assert_eq!(
            resume_state(&vault, key)
                .unwrap()
                .open
                .unwrap()
                .config_fingerprint
                .as_deref(),
            Some("v1:bbb")
        );

        // OB04's restamp: only the matching id, only a v1 or v2 stamp.
        assert!(!restamp_config_fingerprint(&vault, key, "other", "v3:ddd").unwrap());
        assert!(restamp_config_fingerprint(&vault, key, "sess-1", "v3:ddd").unwrap());
        assert!(
            !restamp_config_fingerprint(&vault, key, "sess-1", "v3:eee").unwrap(),
            "never overwrites a v3 stamp"
        );
        record_harness_id_with(&vault, key, "sess-1", Some("v2:fff")).unwrap();
        assert!(restamp_config_fingerprint(&vault, key, "sess-1", "v3:ddd").unwrap());
        record_harness_id_with(&vault, key, "sess-1", None).unwrap();
        assert!(
            !restamp_config_fingerprint(&vault, key, "sess-1", "v3:ddd").unwrap(),
            "never fills a NULL (the backfill does)"
        );
        assert!(stamp_config_fingerprint(&vault, key, "sess-1", "v1:bbb").unwrap());
        assert!(!restamp_config_fingerprint(&vault, "../x", "sess-1", "v3:ddd").unwrap());

        let expired = restart_stint(&vault, key, "claude", "config changed").expect("restart");
        assert!(expired.is_empty(), "no open cards");
        let state = resume_state(&vault, key).unwrap();
        let open = state.open.expect("a fresh stint");
        assert_eq!(
            (open.harness_session_id, open.config_fingerprint),
            (None, None)
        );
        let closed = state.last_closed.expect("the closed stint");
        assert_eq!(closed.harness_session_id.as_deref(), Some("sess-1"));
        assert_eq!(closed.reason.as_deref(), Some("config changed"));
        finish_turn(&vault, key, a, "success").unwrap();
        let summary = read_session(&vault, key).unwrap().summary;
        assert_eq!(summary.stints.len(), 2);
        assert_eq!(summary.stints[0].reason.as_deref(), Some("config changed"));
        assert!(restart_stint(&vault, "../x", "claude", "config changed").is_err());
    }

    /// OB03: a new native id is stored with its fingerprint's parts; a
    /// fingerprint alone clears them; a restart leaves them on the closed
    /// stint (what a pending reset names) and none on the fresh one; and
    /// the backfill fills NULL parts only on the matching id and
    /// fingerprint, never overwriting.
    #[test]
    fn parts_are_stored_with_a_new_native_id() {
        let vault = temp_vault("config-parts");
        let key = "s-20261017-0b030001";
        start_turn(&vault, key, "claude", "/vault", "run-a", "one").expect("turn");
        let parts = r#"{"agent":"aaaa"}"#;
        record_harness_stamp(&vault, key, "sess-1", "v3:aaa", parts).expect("stamp");
        let open_stint = || resume_state(&vault, key).unwrap().open.unwrap();
        let stored = open_stint();
        assert_eq!(stored.config_fingerprint.as_deref(), Some("v3:aaa"));
        assert_eq!(stored.config_parts.as_deref(), Some(parts));
        // The plain recorder leaves both alone.
        record_harness_id(&vault, key, "sess-1").unwrap();
        assert_eq!(open_stint().config_parts.as_deref(), Some(parts));
        // A fingerprint without parts clears them.
        record_harness_id_with(&vault, key, "sess-1", Some("v3:aaa")).unwrap();
        assert_eq!(open_stint().config_parts, None);

        // Backfill: only the matching id and fingerprint, only NULL.
        let later = r#"{"agent":"bbbb"}"#;
        assert!(!stamp_config_parts(&vault, key, "other", "v3:aaa", parts).unwrap());
        assert!(!stamp_config_parts(&vault, key, "sess-1", "v3:zzz", parts).unwrap());
        assert!(stamp_config_parts(&vault, key, "sess-1", "v3:aaa", parts).unwrap());
        assert!(
            !stamp_config_parts(&vault, key, "sess-1", "v3:aaa", later).unwrap(),
            "never overwrites"
        );
        assert_eq!(open_stint().config_parts.as_deref(), Some(parts));
        assert!(!stamp_config_parts(&vault, "../x", "sess-1", "v3:aaa", parts).unwrap());

        restart_stint(&vault, key, "claude", "config changed").expect("restart");
        let state = resume_state(&vault, key).unwrap();
        assert_eq!(state.open.unwrap().config_parts, None, "a fresh stint");
        let closed = state.last_closed.expect("the closed stint");
        assert_eq!(closed.config_parts.as_deref(), Some(parts));
    }

    /// OB03 (review): a stint stamped v2, restamped v3 after an allowed
    /// resume, learns its parts in the same successful-resume branch.
    #[test]
    fn a_restamped_stint_learns_its_parts() {
        let vault = temp_vault("restamp-parts");
        let key = "s-20261017-0b030003";
        start_turn(&vault, key, "claude", "/vault", "run-a", "one").expect("turn");
        record_harness_id_with(&vault, key, "sess-1", Some("v2:x")).unwrap();
        assert!(restamp_config_fingerprint(&vault, key, "sess-1", "v3:y").unwrap());
        let parts = r#"{"agent":"cccc"}"#;
        assert!(stamp_config_parts(&vault, key, "sess-1", "v3:y", parts).unwrap());
        let open = resume_state(&vault, key).unwrap().open.unwrap();
        assert_eq!(open.config_fingerprint.as_deref(), Some("v3:y"));
        assert_eq!(open.config_parts.as_deref(), Some(parts));
    }

    /// Paperclip PR 14b (cross-m34 H4): restarting a stint expires the
    /// conversation's open question cards in the same transaction and
    /// returns them.
    #[test]
    fn restart_stint_expires_open_question_cards() {
        let vault = temp_vault("restart-cards");
        let key = "s-20260926-14b0cccc";
        let ord = start_turn(&vault, key, "claude", "/vault", "run-q", "ask me").unwrap();
        let arguments = Ok(crate::interaction_schema::tests::layout_request());
        let recorded = crate::interactions::record(
            &vault,
            &crate::interactions::NewInteraction {
                session_key: key,
                turn_ord: ord,
                run_id: "run-q",
                agent: crate::agent::AgentKind::Claude,
                native_session_id: Some("sess-q"),
                tool_use_id: "toolu_q",
                source: crate::interactions::InteractionSource::Mcp,
                arguments: &arguments,
                cwd: &vault,
                vault_root: &vault,
            },
        )
        .expect("a pending card");
        assert_eq!(
            crate::interactions::list_pending(&vault, key)
                .unwrap()
                .len(),
            1
        );
        let expired = restart_stint(&vault, key, "claude", "resume failed").unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, recorded.interaction.id);
        assert_eq!(expired[0].status, "expired");
        assert_eq!(expired[0].status_reason.as_deref(), Some("session_reset"));
        assert!(crate::interactions::list_pending(&vault, key)
            .unwrap()
            .is_empty());
    }

    /// Paperclip PR 14b: only a stop before the agent was spawned marks the
    /// restored turn, so Continue is never offered for a request its native
    /// session never saw.
    #[test]
    fn a_before_spawn_stop_marks_the_restored_turn() {
        let vault = temp_vault("before-spawn");
        let key = "s-20260926-14b0eeee";
        for (run, payload) in [
            ("run-1", "before-spawn"),
            ("run-2", "before-retry"),
            ("run-3", "verified"),
        ] {
            let ord = start_turn(&vault, key, "claude", "/vault", run, "Ask").unwrap();
            append_event(&vault, key, ord, "cancelled", payload).unwrap();
            finish_turn(&vault, key, ord, "cancelled").unwrap();
        }
        let conversation = read_conversation(&vault, key).unwrap();
        let marks: Vec<(&str, bool)> = conversation
            .turns
            .iter()
            .map(|t| (t.role.as_str(), t.stopped_before_spawn))
            .collect();
        assert_eq!(
            marks,
            vec![
                ("user", false),
                ("assistant", true),
                ("user", false),
                ("assistant", false),
                ("user", false),
                ("assistant", false)
            ]
        );
        let wire = serde_json::to_value(&conversation.turns[1]).unwrap();
        assert_eq!(wire["stoppedBeforeSpawn"], true);
    }

    /// Paperclip PR 14b: the bundle's source rows. The current turn is left
    /// out, and a result's `resultKind` reaches the conversation turn.
    #[test]
    fn conversation_entries_exclude_current_turn_and_keep_result_kind() {
        let vault = temp_vault("conversation-entries");
        let key = "s-20260926-14b0dddd";
        let first = start_turn(&vault, key, "claude", "/vault", "run-1", "First question").unwrap();
        append_event(&vault, key, first, "assistant", "First answer").unwrap();
        append_event(&vault, key, first, "result", &json!({"costUsd": null, "isError": true, "subtype": "success", "resultKind": "providerQuota"}).to_string()).unwrap();
        finish_turn(&vault, key, first, "error").unwrap();
        let second =
            start_turn(&vault, key, "codex", "/vault", "run-2", "Second question").unwrap();
        append_event(&vault, key, second, "assistant", "Second answer").unwrap();
        finish_turn(&vault, key, second, "success").unwrap();
        let current =
            start_turn(&vault, key, "codex", "/vault", "run-3", "Current question").unwrap();

        let entries = conversation_entries(&vault, key, current).unwrap();
        let summary: Vec<(&str, &str, &str)> = entries
            .iter()
            .map(|e| (e.role.as_str(), e.agent.as_str(), e.text.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("user", "claude", "First question"),
                ("assistant", "claude", "First answer"),
                ("user", "codex", "Second question"),
                ("assistant", "codex", "Second answer"),
            ]
        );
        assert!(conversation_entries(&vault, "../x", current).is_err());

        let conversation = read_conversation(&vault, key).unwrap();
        let kinds: Vec<Option<&str>> = conversation
            .turns
            .iter()
            .filter(|t| t.role == "assistant")
            .map(|t| t.result_kind.as_deref())
            .collect();
        assert_eq!(kinds, vec![Some("providerQuota"), None, None]);
    }

    /// A conversation for the chat search tests: its messages (one turn
    /// each), its title (`None`: untitled) and its last activity.
    fn seed_search(vault: &Path, key: &str, title: Option<&str>, asked: &[&str], at: &str) {
        for (i, text) in asked.iter().enumerate() {
            start_turn(
                vault,
                key,
                "claude",
                "/vault",
                &format!("run-{key}-{i}"),
                text,
            )
            .unwrap();
        }
        let conn = open(vault).unwrap();
        match title {
            Some(title) => rename_session(vault, key, title).unwrap(),
            None => {
                conn.execute(
                    "UPDATE sessions SET title = NULL WHERE key = ?1",
                    params![key],
                )
                .unwrap();
            }
        }
        conn.execute(
            "UPDATE sessions SET last_activity_at = ?2 WHERE key = ?1",
            params![key, at],
        )
        .unwrap();
    }

    /// Each hit of a one-page chat search: its key and its snippet.
    fn found(vault: &Path, query: &str, text: MessageText) -> Vec<(String, Option<String>)> {
        search_sessions(vault, query, |_| true, None, 20, text)
            .unwrap()
            .0
            .into_iter()
            .map(|hit| (hit.summary.key, hit.snippet))
            .collect()
    }

    fn hit(key: &str, snippet: Option<&str>) -> (String, Option<String>) {
        (key.to_string(), snippet.map(str::to_string))
    }

    /// 2026-11-04: the chat search finds a conversation by the person's own
    /// messages too. Each word may sit in the title or in any message; a hit
    /// whose title lacks a word carries the first message that holds the
    /// first such word; an untitled conversation can match; archived ones
    /// never do.
    #[test]
    fn search_matches_titles_or_messages_and_snips_the_message() {
        let vault = temp_vault("search-messages");
        let v = vault.as_path();
        let (a, b, c, d) = (
            "s-20261104-srch0001",
            "s-20261104-srch0002",
            "s-20261104-srch0003",
            "s-20261104-srch0004",
        );
        seed_search(
            v,
            a,
            Some("Quarterly budget review"),
            &["Go"],
            "2026-11-04T10:00:00+00:00",
        );
        seed_search(
            v,
            b,
            Some("Weekly sync"),
            &["Plan the garage lease renewal", "Send the lease to Dana"],
            "2026-11-04T09:00:00+00:00",
        );
        seed_search(
            v,
            c,
            None,
            &["Budget numbers for the east garage"],
            "2026-11-04T08:00:00+00:00",
        );
        seed_search(
            v,
            d,
            Some("Archived budget"),
            &["Budget for the archive"],
            "2026-11-04T07:00:00+00:00",
        );
        archive_session(v, d, true).unwrap();
        let find = |query: &str| found(v, query, MessageText::AsWritten);

        // A title hit has no snippet; the untitled one matches by its text.
        assert_eq!(
            find("BUDGET"),
            [
                hit(a, None),
                hit(c, Some("Budget numbers for the east garage"))
            ]
        );
        // Both words only in messages, in different ones: the snippet is the
        // first message holding the first word.
        assert_eq!(
            find("lease dana"),
            [hit(b, Some("Plan the garage lease renewal"))]
        );
        // One word in the title, the other in a message.
        assert_eq!(
            find("weekly DANA"),
            [hit(b, Some("Send the lease to Dana"))]
        );
        assert_eq!(
            find("garage"),
            [
                hit(b, Some("Plan the garage lease renewal")),
                hit(c, Some("Budget numbers for the east garage"))
            ]
        );
        // A word in no title and no message leaves a conversation out.
        assert!(find("budget zebra").is_empty());
        assert!(find("   ").is_empty());
        // The archived one: not by its title, not by its message.
        assert!(find("archive").is_empty());
    }

    /// 2026-11-04: text hits page as title hits did: newest first, ties by
    /// key, each once, `keep` applied before a page is counted, with the
    /// cursor's keyset in the SQL (a tie at 18:00 spans two pages here).
    #[test]
    fn search_pages_text_hits_by_keyset() {
        let vault = temp_vault("search-pages");
        let v = vault.as_path();
        let skip = "s-20261104-page0005";
        let mut expected = Vec::new();
        for i in 0..12 {
            let key = format!("s-20261104-page{i:04}");
            let asked = if i % 4 == 3 {
                "Nothing to see"
            } else {
                "Count the pallets in bay 4"
            };
            let at = format!("2026-11-04T{:02}:00:00+00:00", 20 - i / 4);
            let title = format!("Note {i}");
            seed_search(v, &key, Some(title.as_str()), &[asked], &at);
            if i % 4 != 3 && key != skip {
                expected.push(key);
            }
        }
        let mut seen = Vec::new();
        let mut after: Option<crate::page_cursor::Keyset> = None;
        let mut pages = 0;
        loop {
            let (page, more) = search_sessions(
                v,
                "PALLETS",
                |s| s.key != skip,
                after.as_ref(),
                3,
                MessageText::AsWritten,
            )
            .unwrap();
            pages += 1;
            assert!(page.len() <= 3);
            assert!(page
                .iter()
                .all(|h| h.snippet.as_deref() == Some("Count the pallets in bay 4")));
            seen.extend(page.iter().map(|h| h.summary.key.clone()));
            if !more {
                break;
            }
            let last = &page.last().unwrap().summary;
            after = Some(crate::page_cursor::Keyset::new(
                Some(&last.last_activity_at),
                &last.key,
            ));
        }
        assert_eq!(seen, expected);
        assert_eq!(pages, 3);
        // A cursor with no time sorts after every conversation.
        let timeless = crate::page_cursor::Keyset::new(None, "s-20261104-page0000");
        let (rest, more) = search_sessions(
            v,
            "pallets",
            |_| true,
            Some(&timeless),
            3,
            MessageText::AsWritten,
        )
        .unwrap();
        assert!(rest.is_empty() && !more);
    }

    /// 2026-11-04: a long message's snippet is about 140 characters around
    /// the match: whitespace collapsed, cut on character boundaries (in text
    /// full of multi-byte characters on both sides) and between words, with
    /// `…` marks.
    #[test]
    fn a_long_messages_snippet_is_bounded_and_char_safe() {
        let vault = temp_vault("search-snippet");
        let v = vault.as_path();
        let key = "s-20261104-snip0001";
        let before = "Grüße aus Köln über Düsseldorf nach Zürich, ".repeat(12);
        let after = " 日本語のメモ と 🚗 の置き場所".repeat(12);
        let asked = format!("{before}\n\n   der Schlüssel liegt bei Ünal{after}");
        seed_search(
            v,
            key,
            Some("Garage"),
            &[asked.as_str()],
            "2026-11-04T10:00:00+00:00",
        );
        let hits = found(v, "SCHLÜSSEL", MessageText::AsWritten);
        let [(hit_key, Some(snippet))] = hits.as_slice() else {
            panic!("one hit with a snippet: {hits:?}");
        };
        assert_eq!(hit_key, key);
        assert!(
            snippet.starts_with('…') && snippet.ends_with('…'),
            "{snippet}"
        );
        assert!(
            snippet.chars().count() <= SEARCH_SNIPPET_CHARS + 2,
            "{snippet}"
        );
        assert!(
            snippet.contains("der Schlüssel liegt bei Ünal"),
            "{snippet}"
        );
        // Between words at both cuts, as the message reads once collapsed.
        let flat = asked.split_whitespace().collect::<Vec<_>>().join(" ");
        let inner = snippet.trim_start_matches('…').trim_end_matches('…');
        assert!(flat.contains(&format!(" {inner} ")), "{snippet}");
        assert!(!inner.contains("  ") && !inner.contains('\n'), "{snippet}");
    }

    /// 2026-11-04: titles keep Rust's full case folding (the SQL lets a row
    /// through on a title Rust may match, the Kelvin sign included, and Rust
    /// decides). Messages are matched as SQLite's `lower()` leaves them, in
    /// each `SearchWord` form: a capital outside ASCII is found typed
    /// exactly, typed in lowercase when it opens the word, and otherwise
    /// only typed as a capital (review of `b36e5aed`).
    #[test]
    fn search_folds_titles_fully_and_messages_by_form() {
        let vault = temp_vault("search-fold");
        let v = vault.as_path();
        let [a, b, c, d] = [
            "s-20261104-fold0001",
            "s-20261104-fold0002",
            "s-20261104-fold0003",
            "s-20261104-fold0004",
        ];
        seed_search(
            v,
            a,
            Some("Über den Zaun"),
            &["Go"],
            "2026-11-04T10:00:00+00:00",
        );
        seed_search(
            v,
            b,
            Some("Fence"),
            &["Über alles, mit ärger"],
            "2026-11-04T09:00:00+00:00",
        );
        seed_search(
            v,
            c,
            Some("Trip"),
            &["Die Öffnungszeiten in Москва", "GRÖSSE M"],
            "2026-11-04T08:00:00+00:00",
        );
        seed_search(
            v,
            d,
            Some("\u{212A}ARAOKE night"),
            &["Go"],
            "2026-11-04T07:00:00+00:00",
        );
        let find = |query: &str| found(v, query, MessageText::AsWritten);
        let keys_of = |query: &str| find(query).into_iter().map(|(k, _)| k).collect::<Vec<_>>();
        assert_eq!(find("über ZAUN"), [hit(a, None)]);
        // a by its title; b's "Über" by the capitalized form.
        assert_eq!(
            find("über"),
            [hit(a, None), hit(b, Some("Über alles, mit ärger"))]
        );
        assert_eq!(find("ÄRGER"), [hit(b, Some("Über alles, mit ärger"))]);
        for query in [
            "Öffnungszeiten",
            "öffnungszeiten",
            "ÖFFNUNGSZEITEN",
            "Москва",
            "москва",
            "МОСКВА",
        ] {
            assert_eq!(keys_of(query), [c], "{query}");
        }
        // The full-text index folds capitals throughout a word.
        assert_eq!(keys_of("GRÖSSE"), [c]);
        assert_eq!(keys_of("grösse"), [c]);
        // The Kelvin sign folds to "k" in Rust only.
        assert_eq!(find("karaoke"), [hit(d, None)]);
        // The distinct literal spellings with ss and sharp-s stay distinct.
        assert!(find("größe").is_empty());
    }

    /// 2026-11-04: SQL tests the first `SEARCH_SQL_WORDS` words; every word
    /// read is still checked on the rows it returns. Words past
    /// `SEARCH_MAX_WORDS` are ignored (review of `b36e5aed`).
    #[test]
    fn search_checks_every_word_past_the_sql_cap() {
        let vault = temp_vault("search-cap");
        let v = vault.as_path();
        let (a, b, c) = (
            "s-20261104-capw0001",
            "s-20261104-capw0002",
            "s-20261104-capw0003",
        );
        let title = "one two three four five six seven eight";
        seed_search(
            v,
            a,
            Some(title),
            &["Go", "and nine more"],
            "2026-11-04T10:00:00+00:00",
        );
        seed_search(v, b, Some(title), &["Go"], "2026-11-04T09:00:00+00:00");
        let query = format!("{title} nine");
        assert!(query.split_whitespace().count() > SEARCH_SQL_WORDS);
        assert_eq!(
            found(v, &query, MessageText::AsWritten),
            [hit(a, Some("and nine more"))]
        );
        let many = (0..SEARCH_MAX_WORDS)
            .map(|i| format!("{}{}", char::from(b'a' + (i / 10) as u8), i % 10))
            .collect::<Vec<_>>()
            .join(" ");
        seed_search(
            v,
            c,
            Some(many.as_str()),
            &["Go"],
            "2026-11-04T08:00:00+00:00",
        );
        let read = |query: String| found(v, &query, MessageText::AsWritten);
        assert_eq!(read(format!("{many} absent")), [hit(c, None)]);
        assert!(read(format!("absent {many}")).is_empty());
    }

    /// 2026-11-04: an agent's search reads messages through `redact`: a word
    /// only a secret holds is no match, a placeholder holds no word (blanked
    /// as R4 does; review of `b36e5aed`), and the snippet carries the
    /// placeholder, never the secret. The person's own search now uses the
    /// same scrubbed index.
    #[test]
    fn an_agents_search_reads_messages_redacted() {
        let vault = temp_vault("search-redacted");
        let v = vault.as_path();
        let key = "s-20261104-rdct0001";
        let secret = "sk-ant-api03-keyabcdefghijklmnopqrstuvw";
        let asked = format!("Use {secret} for the garage bot");
        seed_search(
            v,
            key,
            Some("Bot setup"),
            &[asked.as_str()],
            "2026-11-04T10:00:00+00:00",
        );
        assert!(found(v, "garage sk-ant", MessageText::AsWritten).is_empty());
        assert!(found(v, "key", MessageText::AsWritten).is_empty());
        for query in ["sk-ant", "key", "api"] {
            assert!(found(v, query, MessageText::Redacted).is_empty(), "{query}");
        }
        assert_eq!(
            found(v, "garage", MessageText::Redacted),
            [hit(key, Some("Use [api-key] for the garage bot"))]
        );
        assert_eq!(
            found(v, "bot SETUP", MessageText::Redacted),
            [hit(key, None)]
        );
    }

    /// Review of `b36e5aed`: only the person's own messages are searched,
    /// not a request another agent sent, one Hollow wrote for them, or
    /// Hollow's own wake, check-in or night-queue message.
    #[test]
    fn search_reads_only_the_persons_own_messages() {
        let vault = temp_vault("search-person");
        let v = vault.as_path();
        let key = "s-20261104-pers0001";
        seed_search(
            v,
            key,
            Some("Weekly sync"),
            &[
                "Plan the garage lease renewal",
                "Check the budget forecast",
                "Make a canvas view of the budget",
                "⟲ Hollow: check-in at 10:00. The budget review is due.",
                "  ⟲ Hollow (night queue): item 3, the budget sweep",
            ],
            "2026-11-04T10:00:00+00:00",
        );
        record_turn_asker(v, key, 2, "agent").unwrap();
        record_turn_asker(v, key, 3, "hollow:canvas").unwrap();
        let find = |query: &str| found(v, query, MessageText::AsWritten);
        assert_eq!(
            find("garage"),
            [hit(key, Some("Plan the garage lease renewal"))]
        );
        for query in ["budget", "forecast", "canvas", "check-in", "sweep"] {
            assert!(find(query).is_empty(), "{query}");
        }
    }

    /// Review of `b36e5aed`: `more` says a further row `keep` accepts
    /// follows, so a page whose only next match `keep` rejects is the last.
    #[test]
    fn a_page_is_the_last_when_keep_rejects_the_rows_after_it() {
        let vault = temp_vault("search-keep");
        let v = vault.as_path();
        for (i, hour) in ["10", "09", "08"].into_iter().enumerate() {
            let key = format!("s-20261104-keep000{i}");
            let title = format!("Budget {i}");
            let at = format!("2026-11-04T{hour}:00:00+00:00");
            seed_search(v, &key, Some(title.as_str()), &["Go"], &at);
        }
        let keys =
            |hits: Vec<SessionHit>| hits.into_iter().map(|h| h.summary.key).collect::<Vec<_>>();
        let first_two = ["s-20261104-keep0000", "s-20261104-keep0001"];
        let (all, more) =
            search_sessions(v, "budget", |_| true, None, 2, MessageText::AsWritten).unwrap();
        assert_eq!(keys(all), first_two);
        assert!(more);
        let (kept, more) = search_sessions(
            v,
            "budget",
            |s| s.key != "s-20261104-keep0002",
            None,
            2,
            MessageText::AsWritten,
        )
        .unwrap();
        assert_eq!(keys(kept), first_two);
        assert!(!more, "the only row after the page is one keep rejects");
    }

    /// Review of `b36e5aed`: a word is literal text, `%`, `_` and `'`
    /// included (an FTS5 phrase or `instr` with bound parameters), never a
    /// pattern or SQL. Since 2026-11-08 short words alone read titles only,
    /// so the short ones here come with a longer word the chat holds.
    #[test]
    fn search_words_are_literal_text() {
        let vault = temp_vault("search-literal");
        let v = vault.as_path();
        let (a, b) = ("s-20261104-litr0001", "s-20261104-litr0002");
        seed_search(
            v,
            a,
            Some("Rates"),
            &["Raise the rate 5% for night_shift, says O'Brien"],
            "2026-11-04T10:00:00+00:00",
        );
        seed_search(
            v,
            b,
            Some("Plain"),
            &["Nothing special here"],
            "2026-11-04T09:00:00+00:00",
        );
        let keys_of = |query: &str| {
            found(v, query, MessageText::AsWritten)
                .into_iter()
                .map(|(k, _)| k)
                .collect::<Vec<_>>()
        };
        for query in [
            "raise 5%",
            "raise %",
            "raise _",
            "night_shift",
            "o'brien",
            "raise '",
        ] {
            assert_eq!(keys_of(query), [a], "{query}");
        }
        for query in ["raise 5_", "night%shift", "%%", "5%", "x' OR '1'='1"] {
            assert!(keys_of(query).is_empty(), "{query}");
        }
    }

    /// Review of `b36e5aed`: the search reads `idx_sessions_recent` in its
    /// order, on the first page and after a cursor, so it never sorts the
    /// store and a page stops at its last row. Since 2026-11-08 the ranked
    /// search's rows (full-text words, or short words against titles) stream
    /// from it too, with each phrase's chats evaluated once, and since the
    /// second 2026-11-08 review each priority is its own query (its folders,
    /// the cursor and a `LIMIT`), still streamed from it; the folder list
    /// steps through `idx_sessions_cwd`.
    #[test]
    fn the_search_streams_from_the_recent_index() {
        let vault = temp_vault("search-plan");
        let conn = open(&vault).unwrap();
        let words = [SearchWord::new("budget"), SearchWord::new("Öl")];
        let short = [SearchWord::new("q3"), SearchWord::new("Öl")];
        let cwds = ["/a".to_string(), "/b".to_string()];
        let cursor = Some(("2026-11-04T10:00:00+00:00", "s-20261104-plan0001"));
        for (sql, args) in [
            search_sql(&words, None),
            search_sql(&words, cursor),
            search_hits_sql(&words, &cwds, None, 21),
            search_hits_sql(&words, &cwds, cursor, 21),
            search_hits_sql(&short, &cwds, None, 21),
            search_hits_sql(&short, &cwds, cursor, 3),
        ] {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan = stmt
                .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                plan.iter().any(|d| d.contains("idx_sessions_recent")),
                "{plan:?}"
            );
            assert!(!plan.iter().any(|d| d.contains("TEMP B-TREE")), "{plan:?}");
        }
        let plan: Vec<String> = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT min(cwd) FROM sessions WHERE archived = 0 AND cwd > '/a'",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            plan.iter().any(|d| d.contains("idx_sessions_cwd")),
            "{plan:?}"
        );
    }

    #[test]
    fn concurrent_cold_boot_opens_serialize_schema_initialization_without_busy_errors() {
        use std::sync::{Arc, Barrier};

        let dir = Arc::new(
            tempfile::Builder::new()
                .prefix("vault-cold-boot-")
                .tempdir()
                .unwrap(),
        );
        let threads = 16usize;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);
        for i in 0..threads {
            let dir = Arc::clone(&dir);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || -> Result<(), String> {
                barrier.wait();
                if i % 2 == 0 {
                    with_read(dir.path(), |conn| {
                        conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    })
                } else {
                    start_turn(
                        dir.path(),
                        &format!("s-20261120-boot{i:04}"),
                        "claude",
                        "/vault",
                        &format!("run-boot-{i}"),
                        "hello",
                    )
                    .map(|_| ())
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap().expect("cold-boot open succeeded");
        }
        let (mode, count) = with_read(dir.path(), |conn| {
            let mode: String = conn
                .pragma_query_value(None, "journal_mode", |r| r.get(0))
                .map_err(|e| e.to_string())?;
            let count: i64 = conn
                .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            Ok((mode, count))
        })
        .unwrap();
        assert!(mode.eq_ignore_ascii_case("wal"), "journal_mode={mode}");
        assert_eq!(count, (threads / 2) as i64);
    }
}
