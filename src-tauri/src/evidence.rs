//! Evidence receipts derived from observed tool events plus an authoritative
//! automated-history diff.
//!
//! Assistant prose is never evidence. A changed path comes from the vault
//! before/after snapshot; tool events only establish attributable ordering and
//! observed checks. Consequently a shell-driven or external edit is reported
//! as observed, but cannot be called verified unless every changed path has a
//! known successful write followed by a successful check.
//!
//! **Repository changes (Paperclip PR 16).** A run whose working folder is a
//! git repository outside the vault (or a linked project's repository) also
//! carries `repo_changes`, captured by [`crate::repo_diff`] before the lease
//! is released. A repo row is `confirmed` only when a successful structured
//! write named it exactly (under one of the repo's alias roots, or relative
//! to the run's folder inside the repo); a run with observed, unlisted or
//! unrecorded repo changes is never `noChanges`.

use crate::repo_diff::{RepoBaseline, RepoChanges};
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// The receipt store at the vault root. Trusted input since Paperclip PR 9b
/// (the quiet check reads a previous run's `confirmed` and, since
/// 2026-11-13, `hollowWrite` writes from it), so it
/// is a watched state file (`run_guard::WATCHED_STATE_FILES`) and every write
/// goes through `run_guard::write_host_state`.
pub(crate) const FILE_NAME: &str = ".evidence-receipts.json";
const SCHEMA_VERSION: u32 = 1;
pub(crate) const KEEP: usize = 500;
const EXCERPT_CHARS: usize = 600;
/// Repo rows a persisted receipt keeps per repository (Paperclip PR 16);
/// `totalFiles` keeps the real count and the sidecar keeps up to 200.
pub(crate) const RECEIPT_REPO_FILES: usize = 50;
/// Vault files per receipt that get line counts.
const LINE_STATS_FILES: usize = 50;

static FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// The parsed store, per path, as of the file identity it was parsed from
/// (five asks B4). The rail's refresh and every conversation open read all
/// ~1.5 MB of receipts; parsing it each time was most of both calls. Our
/// writes rename a new file into place (new inode); another process's or an
/// agent's in-place edit changes the length or the mtime (nanoseconds on
/// APFS; a filesystem with coarse mtimes could miss a same-length edit within
/// one tick until the next write). Writers never read through it.
type StoreStamp = (u64, std::time::SystemTime, u64);
type CachedStore = (StoreStamp, u64, std::sync::Arc<ReceiptFileStore>);
static STORE_CACHE: Lazy<Mutex<(u64, HashMap<PathBuf, CachedStore>)>> =
    Lazy::new(|| Mutex::new((0, HashMap::new())));
/// Vaults whose parsed store is kept (one in the app; the test suite uses
/// hundreds at once). The least recently parsed one makes room.
const STORE_CACHE_VAULTS: usize = if cfg!(test) { 256 } else { 4 };

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptFile {
    pub path: String,
    pub change_type: String,
    /// `confirmed` when a successful structured write event named this path;
    /// `observed` when only the filesystem diff proves the change; `shared`
    /// (chat concurrency F1, 2026-10-20) when no write of this run named it
    /// and History marks it another run's ([`ObservedChange::shared`]): the
    /// run shared its workspace folder, so the change is not credited to it.
    pub attribution: String,
    #[serde(default)]
    pub tool: Option<String>,
    /// Lines added and removed (`automated_history::line_stats`), for the
    /// chat's `+a −d` file chips. Absent on older receipts, binary or large
    /// files, and a history object that could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u32>,
    /// History holds no copy of one version of this file, or can't tell
    /// whether it existed, so it can't be restored (merged plan slice 4,
    /// decision 5). Absent when it can, and on older receipts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_recoverable: bool,
    /// Other runs in the same workspace folder whose changes list this file
    /// too (`HistoryChange::also_changed_by`, chat concurrency F1). Absent
    /// when none, and on older receipts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_changed_by: Vec<String>,
    /// A Hollow app or task write made on the run's behalf (the task
    /// broker) left this file as the run ended
    /// (`HistoryChange::hollow_write`, 2026-11-13). Its `attribution` stays
    /// as the tool events say; the quiet check credits it as the run's own
    /// write. Read from the receipt store, which run_guard watches, so a
    /// later run can't forge it unflagged. Absent when not, and on older
    /// receipts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hollow_write: bool,
}

impl ReceiptFile {
    /// Whether this file is credited to the receipt's run: anything but
    /// `shared` (Results' origin, task state, line totals and file counts
    /// skip a shared file).
    pub fn is_own(&self) -> bool {
        self.attribution != ATTRIBUTION_SHARED
    }
}

/// [`ReceiptFile::attribution`] for a file another run changed.
pub const ATTRIBUTION_SHARED: &str = "shared";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptCheck {
    pub command: String,
    pub kind: String,
    pub outcome: String,
    pub sequence: u64,
    #[serde(default)]
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptFailure {
    pub label: String,
    pub sequence: u64,
    pub excerpt: String,
}

/// `Default` (Paperclip PR 14a, cross-m34 M8) lets a literal name only the
/// fields it sets, so a new optional field touches no other literal.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReceipt {
    pub id: String,
    pub run_id: String,
    pub source: String,
    pub source_id: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub status: String,
    pub verification_state: String,
    pub files: Vec<ReceiptFile>,
    pub checks: Vec<ReceiptCheck>,
    pub failures: Vec<ReceiptFailure>,
    pub history_run_id: String,
    /// Hollow-owned session key (`agent_sessions`). Always present for a
    /// chat turn recorded after this field shipped, which is what stops a
    /// receipt being orphaned: `source_id` is the HARNESS session id and is
    /// still null when a first turn dies before its stream announces one.
    #[serde(default)]
    pub session_key: Option<String>,
    /// `"verified"` | `"unconfirmed"`: whether ending the run's processes
    /// (on stop, timeout or normal finish) left nothing Hollow could find.
    /// Absent when the run had no owned process group.
    #[serde(default)]
    pub process_stop: Option<String>,
    /// Paperclip PR 14a: how the run ended (`result_kind::ResultKind`, its
    /// camelCase wire value: `success`, `refusal`, `maxTurns`,
    /// `providerQuota`, `transient`, `auth`, `resumeFailed`,
    /// `permissionDenied`, `stalled`, `budgetStop`, `cancelled`, `timedOut`,
    /// `error`). Absent on receipts written before it, and on surfaces that
    /// do not classify (heartbeat, ingest).
    #[serde(default)]
    pub result_kind: Option<String>,
    /// Changes in the run's target git repositories (Paperclip PR 16),
    /// without patches (those live in the run's `.repo.json` sidecar), at
    /// most [`RECEIPT_REPO_FILES`] rows per repository. Absent when the run
    /// had no repository target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repo_changes: Vec<RepoChanges>,
    /// Paperclip PR 15 (15c): how long each phase of the run took. Absent on
    /// receipts written before it; always present since, because
    /// `evidenceMs` is measured by every `finish`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_timings: Option<PhaseTimings>,
    /// Merged plan Track U: the effort the turn asked for and what the agent
    /// was actually given. Absent when no effort was asked for, and on
    /// receipts written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReceiptEffort>,
    /// COO rebuild L7: what the CFO decided for the turn (the manifest's
    /// `cfo`), for the turn footer's "model · Effort · reason". Absent on
    /// runs the CFO doesn't decide for and on receipts written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cfo: Option<crate::cfo::Decision>,
    /// Services this turn started through `hollow_services` (merged plan
    /// decision 1): the command and folder of each, and whether it ends with
    /// the chat (bounded) or only on a stop (bypass). Absent when it started
    /// none, and on receipts written before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ReceiptService>,
    /// Merged plan decision 6: where the run moved to keep working at a
    /// ceiling, in order: another Claude account (R1's quota failover) or
    /// another agent (a budget ceiling at the claim). Absent when it never
    /// moved, and on receipts written before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub switches: Vec<ReceiptSwitch>,
    /// Actual account picks and quota/meter mismatches, in attempt order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub account_choices: Vec<String>,
    /// What the person said to the turn while it ran (mid-turn steering,
    /// 2026-10-21): when, the words (cut at [`RECEIPT_STEER_CHARS`]) and
    /// whether the agent took them. Absent when nobody steered, and on
    /// receipts written before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steers: Vec<ReceiptSteer>,
    /// Whether History can restore every vault change this receipt lists
    /// (merged plan slice 4, decision 5): `false` as soon as one file is not
    /// recoverable, `true` only when every change came from History and is.
    /// Absent when the receipt lists no vault change, when a change's
    /// coverage is not known, and on older receipts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_recoverable: Option<bool>,
    /// The `hollow-canvas` fences in the turn's reply (Canvas C3 wave 2
    /// C4): how many and their body bytes, so a reply's canvas cost shows
    /// on its receipt. Absent when the reply had none, on surfaces with no
    /// reply, and on receipts written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvases: Option<ReceiptCanvases>,
    /// The pull requests this turn merged or opened (overnight F8), from its
    /// own `gh pr merge <n>` / `gh pr create` calls whose results weren't
    /// errors (the parsing Home's Last night card uses). Absent when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pull_requests: Vec<ReceiptPullRequest>,
    /// The wake-up this turn asked for (overnight C3, `wake_me`), as one
    /// line: when and why. Absent when it asked for none, or that wake was
    /// cancelled or replaced before the receipt was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_wake: Option<String>,
    /// The run's context manifest (agent effectiveness F11): its schema,
    /// size and SHA-256. The manifest itself is the sidecar
    /// `.evidence-manifests/{runId}.json` (`run_manifest`), kept as long as
    /// this receipt. Absent when the run recorded none (heartbeat, ingest),
    /// when it could not be written, and on receipts written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_manifest: Option<crate::run_manifest::ManifestRef>,
    /// The run's cost with its descendants' as its turn ended (agent
    /// effectiveness F10, D2; `descendants::rollup`): its own, each bounded
    /// consultation, each durable child task (`null` while still running)
    /// and, when the turn was stopped, what the stop did to each child
    /// task's turn (`stop`). The manifest view reads the same roll-up live.
    /// Absent when the run had no descendants (and its own cost is the
    /// usage chip's), on surfaces that record none and on receipts written
    /// before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descendants: Option<crate::run_manifest::CostRollup>,
    /// The run's model calls and context (OB05; `usage::CallStats`, the
    /// same keys as its usage row's `usage_json`): `apiCalls`,
    /// `peakContextTokens`, `cacheWriteTokens`, `apiMs`, `numTurns`,
    /// `subagents` and (S31, estimated) `workflowAgents`, each only when
    /// the agent supplied it. Absent when it
    /// supplied none (Codex, Antigravity), on surfaces that don't set it
    /// (heartbeat, ingest) and on receipts written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_usage: Option<crate::usage::CallStats>,
    /// What the run did beyond its files (plan completion L2,
    /// `run_effects`): recorded outside effects (connector writes, Hollow
    /// changes, `git push`, `gh`, sending `curl`, …) each done / failed /
    /// uncertain, and the calls whose effects Hollow can't see. `{}` when it
    /// had neither; absent on receipts written before it (unknown, not
    /// none) and on receipts no observing run wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outside_effects: Option<crate::run_effects::OutsideEffects>,
    /// The tool calls the run's stream reported (OF18), errors included, so
    /// its failures can be read per call and agents compared. Absent
    /// (unknown, not none) on receipts written before it, on receipts no
    /// observing run wrote, and for an agent whose stream reports no tool
    /// calls at all (Antigravity, [`EvidenceCollector::mark_tool_calls_unreported`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u32>,
    /// The run's writes into Claude Code's own auto-memory folder (S13):
    /// its `Write`, `Edit` and `MultiEdit` calls that completed without an
    /// error (failed and never-answered ones left out), whose `file_path` is inside the manifest's `nativeMemory.autoMemory`
    /// folder. Present (0 included) when the manifest has `nativeMemory`
    /// (Claude runs); absent for other agents and on receipts written
    /// before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_writes: Option<u32>,
    /// How many times the agent compacted its session's context during the
    /// run (OB22): the `compact_boundary` events in a Claude-format stream
    /// (Claude, and Grok, which shares the parser; `ParseState::compactions`),
    /// a quota failover's attempts included. Counted for chat turns and
    /// automation runs only: heartbeat and delegate runs (`run_collect`)
    /// record none. Absent when it never compacted, for Codex and
    /// Antigravity, and on receipts written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compactions: Option<u32>,
    /// S27: whether the run's final reply claimed its checks pass, and
    /// whether the checks this receipt saw back that up
    /// ([`ReceiptVerification`]). Present on a chat turn's receipt; absent on
    /// surfaces with no reply (heartbeat, workflows, ingest) and on receipts
    /// written before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<ReceiptVerification>,
    /// Repo features R5: the `path:line` cites in the run's final reply (a
    /// chat turn's or a workflow's output), each checked when the run ended
    /// for whether the file exists, whether the line is in it and whether
    /// the run read it ([`crate::cite_check::ReceiptCites`]). A note only:
    /// it never changes `status` or `verificationState`. Absent when the
    /// reply cited none, on surfaces with no reply and on receipts written
    /// before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cites: Option<crate::cite_check::ReceiptCites>,
}

/// [`EvidenceReceipt::canvases`]: canvas fences in one reply and the UTF-8
/// bytes of their bodies (`canvas_fence::reply_canvases`), valid or not.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptCanvases {
    pub count: u32,
    pub bytes: u64,
}

/// What a Claude Code `task_notification` line says of a background task
/// that ended `stopped` or `killed`, rather than `completed` or `failed`
/// (it ran out): `(id, summary)`, the summary else the id. `None` for every
/// other line. On its own it doesn't mean the task was cut off: the agent
/// may have stopped it itself, and Hollow's own stop ends tasks too, so only
/// [`CutTasks`] decides. Overnight A3.
pub fn stopped_background_task(line: &str) -> Option<(String, String)> {
    if !line.contains("\"task_notification\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("system")
        || v.get("subtype").and_then(Value::as_str) != Some("task_notification")
    {
        return None;
    }
    let status = v.get("status").and_then(Value::as_str)?;
    if !matches!(status, "stopped" | "killed") {
        return None;
    }
    let id = v
        .get("task_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let summary = ["summary", "description", "task_id"]
        .iter()
        .find_map(|key| v.get(*key).and_then(Value::as_str))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "a background task".to_string());
    Some((id, summary))
}

/// Whether `line` is the agent's top-level `result` event (Claude Code's
/// end of the turn).
fn is_result_line(line: &str) -> bool {
    line.contains("\"result\"")
        && serde_json::from_str::<Value>(line)
            .ok()
            .is_some_and(|v| v.get("type").and_then(Value::as_str) == Some("result"))
}

/// The background tasks one attempt's CLI cut off as it exited (overnight
/// A3). Claude Code ends the turn with its `result` event, then kills the
/// background tasks still running and reports each `stopped` (seen in a real
/// `claude -p` probe, 2026-10-10). So only a stopped notice AFTER the
/// attempt's `result` counts (an agent stopping its own task does it
/// mid-turn), and only once the attempt is known to have exited on its own
/// ([`Self::commit`]): Hollow's own stop ends tasks too.
#[derive(Debug, Default)]
pub struct CutTasks {
    after_result: bool,
    tasks: Vec<(String, String)>,
}

impl CutTasks {
    /// Fold one stdout line in.
    pub fn observe(&mut self, line: &str) {
        if is_result_line(line) {
            self.after_result = true;
        } else if self.after_result {
            if let Some(task) = stopped_background_task(line) {
                self.tasks.push(task);
            }
        }
    }

    /// Record the tasks on `evidence` when the attempt exited on its own
    /// (`exited`: `Ended::Exited` with the leader's exit seen before
    /// settle); drop them otherwise.
    pub fn commit(self, exited: bool, evidence: &mut EvidenceCollector) {
        if exited {
            for task in self.tasks {
                evidence.record_background_stopped(task);
            }
        }
    }
}

/// Tools that schedule a later wakeup of the CLI's own session. None of them
/// resumes a Hollow run, which ends when the agent's process does.
pub const WAKEUP_TOOLS: [&str; 2] = ["ScheduleWakeup", "CronCreate"];

/// The receipt rows for a run that ended mid-work (overnight A3), as
/// `(label, excerpt)`: `background` when the CLI reported a background task
/// `stopped` at its exit, or (for a CLI that reports none) when it started
/// `run_in_background` work and `leftovers` of its processes were still
/// alive after the agent exited on its own, which Hollow then ended; and
/// `wakeup` when a call to one of [`WAKEUP_TOOLS`] succeeded.
fn ended_mid_work(
    observations: &[ToolObservation],
    leftovers: usize,
    stopped: &[(String, String)],
) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    let background = observations
        .iter()
        .filter(|o| o.input.get("run_in_background").and_then(Value::as_bool) == Some(true))
        .count();
    if let Some((_, first)) = stopped.first() {
        // Claude Code stops its own background tasks as it exits and says
        // so in its stream: the precise signal (seen in a real `claude -p`
        // probe, 2026-10-10).
        let more = match stopped.len() {
            1 => String::new(),
            n => format!(" and {} more", n - 1),
        };
        rows.push((
            "background",
            format!(
                "Ended with background work still running ({}{more}); it was stopped when the run ended. Background work never outlives a Hollow run.",
                cap(first, 120)
            ),
        ));
    } else if background > 0 && leftovers > 0 {
        let processes = if leftovers == 1 {
            "1 process".to_string()
        } else {
            format!("{leftovers} processes")
        };
        rows.push((
            "background",
            format!(
                "Ended with background work still running ({processes}); Hollow stopped it when the run ended. Background work never outlives a Hollow run."
            ),
        ));
    }
    let mut wakeups: Vec<&str> = Vec::new();
    for name in observations
        .iter()
        .filter(|o| o.completed_sequence.is_some() && !o.is_error)
        .map(|o| o.name.as_str())
    {
        if WAKEUP_TOOLS.contains(&name) && !wakeups.contains(&name) {
            wakeups.push(name);
        }
    }
    if !wakeups.is_empty() {
        rows.push((
            "wakeup",
            format!(
                "Scheduled a later wakeup ({}); it doesn't resume this Hollow run.",
                wakeups.join(", ")
            ),
        ));
    }
    rows
}

/// [`ReceiptSwitch::kind`] of R1's move to another Claude account.
pub const SWITCH_ACCOUNT: &str = "account";
/// [`ReceiptSwitch::kind`] of a budget ceiling's move to another agent.
pub const SWITCH_AGENT: &str = "agent";
/// [`ReceiptSwitch::kind`] of a run a claude.ai connector put on Claude
/// Code's own login instead of an added account (login override,
/// 2026-10-09). Not a move: it never left the login.
pub const SWITCH_LOGIN: &str = "login";
/// [`ReceiptSwitch::kind`] of a provider outage's one later resume of the
/// same session (overnight 9 A3, `chat::Next::Backoff`). Not a move either.
pub const SWITCH_RETRY: &str = "retry";
/// [`ReceiptSwitch::kind`] of a large Main-agent chat that started a fresh
/// session after more than an hour away instead of resuming (overnight 9
/// A9, `chat_resume::ResetReason::Rested`); `reason` is "away 3 h at 714K".
pub const SWITCH_RESTED: &str = "rested";

/// The login override's receipt row: `connectors` (names) kept this run on
/// Claude Code's own login while added accounts could run, and `remedy`,
/// the Hollow connector that would avoid it (`connectors::login_remedy`).
pub fn login_override(connectors: &[String], remedy: Option<&str>) -> ReceiptSwitch {
    let mut reason = format!("claude.ai connector: {}", connectors.join(", "));
    if let Some(remedy) = remedy {
        reason.push_str(". ");
        reason.push_str(remedy);
    }
    ReceiptSwitch {
        kind: SWITCH_LOGIN.into(),
        from: "added accounts".into(),
        to: "Claude Code login".into(),
        reason,
    }
}

/// One move a run made at a ceiling (decision 6): `kind`
/// ([`SWITCH_ACCOUNT`] or [`SWITCH_AGENT`]), the labels it moved between,
/// and why.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptSwitch {
    pub kind: String,
    pub from: String,
    pub to: String,
    pub reason: String,
}

/// One steer on a receipt (`crate::steering::SteerRecord`, shortened).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptSteer {
    pub at: String,
    pub text: String,
    /// The agent took it into the turn.
    pub delivered: bool,
}

/// A steer's words on a receipt are cut here (the transcript has them whole).
pub const RECEIPT_STEER_CHARS: usize = 280;

/// One service a turn started (`services::Registry::start`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptService {
    pub name: String,
    pub command: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// `dev-server` or `other`.
    pub kind: String,
    /// True for a bounded service (stops when its chat is archived).
    pub ends_with_chat: bool,
}

/// The effort a turn asked for (`requested`: a level or `ultracode`) and
/// the level its agent got (`sent`, `None` when the model takes no effort),
/// with why they differ when it matters (`note`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptEffort {
    pub requested: String,
    pub sent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Paperclip PR 15 (15c): where a run's time went, in milliseconds. Every
/// field is optional and omitted when that phase was not measured on this
/// surface; a missing value is never written as zero.
///
/// - `queueWaitMs`: the queue row's request → `running` (written by
///   `run_queue::mark_running`, copied here by [`EvidenceCollector::finish_mirrored`]).
/// - `baselineSnapshotMs`: the vault snapshot and manifest write inside
///   admission (`automated_history::RunLease::baseline_ms`).
/// - `spawnMs`: admitted → the agent spawned, i.e. preparation (context,
///   retrieval, the command, a repository snapshot).
/// - `firstOutputMs`: spawn → the first stdout or stderr line (kept as it
///   was; stderr is not the agent's output, so prefer the next two).
/// - `firstStdoutMs`: spawn → the first stdout line (agent effectiveness
///   F11): the agent's stream started.
/// - `firstModelMs`: spawn → the first model event (OB08a): the first
///   thinking, tool call or assistant text the stream carried
///   (`run_activity::RunActivity::note_event`). Absent for Antigravity: its
///   stream carries only text, so the figure would just repeat `firstTextMs`.
/// - `firstTextMs`: spawn → the first assistant text the stream carried
///   (F11): the first useful output. Absent when the reply had none.
/// - `toolsBeforeText`: a count, not a time (OB08a): the tool calls the
///   stream carried before its first text, a subagent's included. A run that
///   wrote no text has the count so far; a run with no model event, and
///   every Antigravity run (no tool calls in its stream), has none.
/// - `turnMs`: spawn → `process_group::drive` returned (the processes are
///   gone). Spans a resume retry (PR 14b): the spawn is the first attempt's.
/// - `repoCaptureMs`: the post-run repository capture (Paperclip PR 16).
/// - `finalizeDiffMs`: the history diff and lease release (`finish_run`).
/// - `evidenceMs`: classifying this receipt.
///
/// The heartbeat records spawn, first outputs and turn from its runner's
/// activity clock (`agent::Collected::activity`, OB08a); its Codex snapshot
/// pass (the tool-free transform) has no clock and keeps the whole runner
/// call as `turnMs`. Ingest has no receipt.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PhaseTimings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_wait_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_snapshot_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_output_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_stdout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_model_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_text_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_before_text: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_capture_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalize_diff_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_ms: Option<u64>,
    /// When admission completed, on the monotonic clock: the run's context
    /// manifest times its preparation steps from it (agent effectiveness
    /// F8, `prep_timing`). In memory only.
    #[serde(skip)]
    pub(crate) admitted: crate::prep_timing::InMemory<Option<Instant>>,
}

impl PhaseTimings {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Record the agent's phases once `drive` has returned, from the lease's
    /// admission clock and the run's activity clock (PR 15b), both read now:
    /// the spawn happened `since_spawn_ms` ago and admission
    /// `admitted_elapsed_ms` ago, so preparation took the difference. Nothing
    /// is recorded for a run that never spawned.
    pub(crate) fn record_drive(
        &mut self,
        admitted_elapsed_ms: u64,
        since_spawn_ms: Option<u64>,
        first: FirstOutputs,
    ) {
        if self.admitted.0.is_none() {
            self.admitted.0 =
                Instant::now().checked_sub(std::time::Duration::from_millis(admitted_elapsed_ms));
        }
        let Some(since_spawn) = since_spawn_ms else {
            return;
        };
        self.spawn_ms = Some(admitted_elapsed_ms.saturating_sub(since_spawn));
        self.turn_ms = Some(since_spawn);
        self.first_output_ms = first.any;
        self.first_stdout_ms = first.stdout;
        self.first_model_ms = first.model;
        self.first_text_ms = first.text;
        self.tools_before_text = first.tools_before_text;
    }

    /// Admission completed at `at` (`RunLease::admitted_at`): the clock the
    /// manifest's preparation timeline is read on (F8).
    pub(crate) fn set_admitted(&mut self, at: Instant) {
        self.admitted.0 = Some(at);
    }

    /// The clock the preparation timeline reads (`prep_timing::timeline`).
    fn run_clock(&self) -> crate::prep_timing::RunClock {
        crate::prep_timing::RunClock {
            admitted: self.admitted.0,
            baseline_ms: self.baseline_snapshot_ms,
            spawn_ms: self.spawn_ms,
            first_stdout_ms: self.first_stdout_ms,
            first_text_ms: self.first_text_ms,
        }
    }

    /// The camelCase keys that are set, for the queue row's mirror.
    pub(crate) fn to_map(&self) -> serde_json::Map<String, Value> {
        match serde_json::to_value(self) {
            Ok(Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    }
}

/// The run's first outputs after its spawn, in milliseconds
/// (`run_activity::RunActivity::first_outputs`): any line, the first stdout
/// line, the first model event, the first assistant text; and the tool calls
/// before that text (OB08a, see [`PhaseTimings`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FirstOutputs {
    pub any: Option<u64>,
    pub stdout: Option<u64>,
    pub model: Option<u64>,
    pub text: Option<u64>,
    pub tools_before_text: Option<u32>,
}

impl FirstOutputs {
    /// What `kind`'s stream can tell (OB08a): Antigravity's carries no
    /// thinking or tool calls Hollow parses, so its first model event would
    /// only repeat its first text and its count is unknown, never 0. Both
    /// stay absent.
    pub(crate) fn for_agent(mut self, kind: crate::agent::AgentKind) -> Self {
        if kind == crate::agent::AgentKind::Antigravity {
            self.model = None;
            self.tools_before_text = None;
        }
        self
    }
}

/// Milliseconds since `started`, saturating.
pub(crate) fn ms_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ToolObservation {
    id: String,
    name: String,
    input: Value,
    started_sequence: u64,
    completed_sequence: Option<u64>,
    result: Option<String>,
    is_error: bool,
    /// A pull request URL in the full result, read before the excerpt cap
    /// (`gh` prints it last, after any long output; overnight F8).
    pr_url: Option<(String, u64)>,
    /// What the full result's text says of each check the command holds,
    /// in [`shell_checks`]' order ([`read_output`]), read before the excerpt
    /// cap (a runner prints its summary last; OF16). Verdicts only, never
    /// text.
    #[serde(skip)]
    check_text: Vec<&'static str>,
    /// The `(path, line)` grep hits in the full result (repo features R5,
    /// `cite_check::grep_hits`), read before the excerpt cap: what the
    /// citation check counts as seen. Paths and lines only, never text.
    #[serde(skip)]
    hits: Vec<(String, u32)>,
}

#[derive(Debug, Clone)]
pub struct EvidenceCollector {
    source: String,
    source_id: Option<String>,
    run_id: String,
    started_at: String,
    sequence: u64,
    observations: Vec<ToolObservation>,
    session_key: Option<String>,
    process_stop: Option<String>,
    /// Processes the agent left running when it exited on its own
    /// ([`Self::set_leftovers`], overnight A3).
    leftovers: usize,
    /// Background tasks the agent's CLI itself stopped when it exited
    /// ([`stopped_background_task`]): what each was, as the CLI named it.
    stopped_background: Vec<(String, String)>,
    result_kind: Option<crate::result_kind::ResultKind>,
    /// The pre-run repository snapshot (Paperclip PR 16), set after
    /// admission and armed only once the agent spawned.
    repo_baseline: Option<RepoBaseline>,
    /// The watched repositories' roots, kept after the baseline is taken.
    repo_roots: Vec<std::path::PathBuf>,
    /// What the post-run capture found, set before `finish`.
    repo_changes: Vec<RepoChanges>,
    /// The phases measured so far (Paperclip PR 15c); `finish` adds
    /// `evidenceMs` and puts them on the receipt.
    timings: PhaseTimings,
    /// Requested → sent effort (Track U), for the receipt.
    effort: Option<ReceiptEffort>,
    /// The CFO's decision (COO rebuild L7), for the receipt.
    cfo: Option<crate::cfo::Decision>,
    /// Services the run started (merged plan decision 1).
    services: Vec<ReceiptService>,
    /// Moves the run made at a ceiling (merged plan decision 6).
    switches: Vec<ReceiptSwitch>,
    account_choices: Vec<String>,
    /// The person's mid-turn steers (2026-10-21).
    steers: Vec<ReceiptSteer>,
    /// The reply's canvas fences (Canvas C3 wave 2 C4).
    canvases: Option<ReceiptCanvases>,
    /// The run's context manifest (agent effectiveness F11), built as its
    /// preparation goes and stored with the receipt.
    context: Option<crate::run_manifest::RunManifest>,
    /// The run's cost with its descendants' (F10, D2), set before `finish`.
    descendants: Option<crate::run_manifest::CostRollup>,
    /// The run's calls and context from its usage row (OB05).
    api_usage: Option<crate::usage::CallStats>,
    /// A guarded retry's carried effects (plan completion L2): what the run
    /// it retries already did, kept on this run's receipt too.
    carried_effects: crate::run_effects::OutsideEffects,
    /// The agent reports no tool calls (Antigravity): outside effects are
    /// unknown, never "none".
    tool_calls_unreported: bool,
    /// The run's context compactions (OB22), set before `finish`.
    compactions: u32,
    /// Where the turn's final reply starts in its assistant text (S27): the
    /// text's length at its last tool call ([`Self::mark_reply_start`]).
    reply_start: usize,
    /// What that final reply claims ([`reply_claim_kind`]); `None` when no
    /// reply was recorded (S27).
    reply_claim: Option<ReplyClaim>,
    /// The `path:line` cites in the final reply (repo features R5,
    /// [`Self::set_reply_citations`]): paths and lines only.
    reply_cites: Vec<crate::cite_check::Cite>,
    /// The Read, `read_source` and shell calls of the conversation's earlier
    /// turns (R5, [`Self::set_earlier_reads`]), as `(tool, input)`.
    earlier_reads: Vec<(String, Value)>,
}

#[derive(Debug, Clone, Default)]
pub struct ObservedChange {
    pub path: String,
    pub change_type: String,
    /// The history objects of each side, for the receipt's line counts.
    /// `None` for an absent side (create/delete) or a test fixture.
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    /// Whether History can restore this change
    /// (`HistoryChange::recoverable`). `None` when it didn't come from
    /// History (a test fixture or another surface's own record).
    pub recoverable: Option<bool>,
    /// History marks it not this run's own (`HistoryChange::unattributed`).
    pub shared: bool,
    /// `HistoryChange::also_changed_by`.
    pub also_changed_by: Vec<String>,
    /// `HistoryChange::hollow_write`.
    pub hollow_write: bool,
}

impl ObservedChange {
    /// The receipt's view of one finalized history change.
    pub fn from_history(change: crate::automated_history::HistoryChange) -> Self {
        let recoverable = Some(change.recoverable());
        Self {
            path: change.vault_path,
            change_type: change.change_type,
            before_hash: change.before_hash,
            after_hash: change.after_hash,
            recoverable,
            shared: change.unattributed,
            also_changed_by: change.also_changed_by,
            hollow_write: change.hollow_write,
        }
    }
}

/// [`EvidenceReceipt::history_recoverable`] for these changes: false if any
/// one is not recoverable, true only if every one is known to be.
fn history_recoverable(changes: &[ObservedChange]) -> Option<bool> {
    if changes
        .iter()
        .any(|change| change.recoverable == Some(false))
    {
        return Some(false);
    }
    let all_known = changes.iter().all(|change| change.recoverable.is_some());
    (!changes.is_empty() && all_known).then_some(true)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReceiptFileStore {
    schema_version: u32,
    receipts: Vec<EvidenceReceipt>,
}

impl Default for ReceiptFileStore {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            receipts: Vec::new(),
        }
    }
}

impl EvidenceCollector {
    pub fn new(source: &str, source_id: Option<&str>, run_id: &str) -> Self {
        Self {
            source: source.trim().to_string(),
            source_id: source_id.map(str::to_string),
            run_id: run_id.to_string(),
            started_at: Utc::now().to_rfc3339(),
            sequence: 0,
            observations: Vec::new(),
            session_key: None,
            process_stop: None,
            leftovers: 0,
            stopped_background: Vec::new(),
            result_kind: None,
            repo_baseline: None,
            repo_roots: Vec::new(),
            repo_changes: Vec::new(),
            timings: PhaseTimings::default(),
            effort: None,
            cfo: None,
            services: Vec::new(),
            switches: Vec::new(),
            account_choices: Vec::new(),
            steers: Vec::new(),
            canvases: None,
            context: None,
            descendants: None,
            api_usage: None,
            carried_effects: crate::run_effects::OutsideEffects::default(),
            tool_calls_unreported: false,
            compactions: 0,
            reply_start: 0,
            reply_claim: None,
            reply_cites: Vec::new(),
            earlier_reads: Vec::new(),
        }
    }

    /// A guarded retry carries what the run it retries already did onto its
    /// own receipt (`run_effects::carried`), so a retry of this run still
    /// guards it.
    pub fn carry_effects(&mut self, effects: crate::run_effects::OutsideEffects) {
        self.carried_effects = effects;
    }

    /// The run's agent reports no tool calls, so what it did beyond its
    /// files is unknown (an unknown group on the receipt).
    pub fn mark_tool_calls_unreported(&mut self) {
        self.tool_calls_unreported = true;
    }

    /// The tool calls seen so far, every attempt together; `None` when the
    /// agent reports none at all (repo ideas 10-27 #8: only a turn that
    /// called no tool changed nothing).
    pub fn tool_calls_seen(&self) -> Option<usize> {
        (!self.tool_calls_unreported).then_some(self.observations.len())
    }

    /// The run's context manifest (agent effectiveness F11), started on
    /// first use. Each run has its own: nothing about one run's context is
    /// kept anywhere another run's preparation can overwrite it.
    pub fn context_mut(&mut self) -> &mut crate::run_manifest::RunManifest {
        let run_id = self.run_id.clone();
        self.context.get_or_insert_with(|| {
            let mut manifest = crate::run_manifest::RunManifest::new();
            manifest.run_id = run_id;
            manifest
        })
    }

    /// Record the canvas fences in the turn's reply (Canvas C3 wave 2 C4);
    /// a reply with none records nothing.
    pub fn set_reply_canvases(&mut self, reply: &str) {
        let found = crate::canvas_fence::reply_canvases(reply);
        self.canvases = (found.count > 0).then_some(ReceiptCanvases {
            count: found.count,
            bytes: found.bytes,
        });
    }

    /// The turn's assistant text is `offset` bytes long as the agent makes a
    /// tool call or gets its result (S27): its final reply is what comes
    /// after the last one, so a subagent's closing text before its
    /// delegation's result is not the reply.
    pub fn mark_reply_start(&mut self, offset: usize) {
        self.reply_start = offset;
    }

    /// Record whether the turn's final reply claims it is done, fixed, or
    /// that its checks pass (S27, overhaul E3): `reply` is the turn's whole
    /// assistant text, read from the last [`Self::mark_reply_start`] on
    /// (nothing, so no claim, when that offset doesn't fit this text: never
    /// the whole text, which holds earlier steps' prose).
    pub fn set_reply_claim(&mut self, reply: &str) {
        let last = reply.get(self.reply_start..).unwrap_or("");
        self.reply_claim = Some(reply_claim_kind(last));
    }

    /// Mark the kept retrieval passages the run's reply names (S26,
    /// `run_manifest::cites`): `reply` is the run's final answer (a
    /// workflow's output). Only the `cited` flags are kept, never the text.
    /// Also keeps the reply's `path:line` cites for the receipt's citation
    /// check (repo features R5, `cite_check::extract`): paths and lines only.
    pub fn set_reply_citations(&mut self, reply: &str) {
        if let Some(manifest) = self.context.as_mut() {
            manifest.mark_cited(reply);
        }
        self.reply_cites = crate::cite_check::extract(reply);
    }

    /// Whether the final reply cites any `path:line` (R5): only then does a
    /// chat turn read its earlier turns' reads.
    pub fn has_reply_cites(&self) -> bool {
        !self.reply_cites.is_empty()
    }

    /// The Read, `read_source` and shell calls of the conversation's earlier
    /// turns (R5, `step_spine::earlier_reads`): a cite of a file one of them
    /// read is `earlier`, not `notOpened`.
    pub fn set_earlier_reads(&mut self, reads: Vec<(String, Value)>) {
        self.earlier_reads = reads;
    }

    /// [`Self::set_reply_citations`] for a chat turn (S26): `assistant` is
    /// the turn's whole assistant text, read from the last
    /// [`Self::mark_reply_start`] on, as [`Self::set_reply_claim`] reads it,
    /// so earlier steps' narration never cites. Nothing is marked when that
    /// offset doesn't fit this text.
    pub fn set_final_reply_citations(&mut self, assistant: &str) {
        let last = assistant.get(self.reply_start..).unwrap_or("");
        self.set_reply_citations(last);
    }

    /// Record a move the run made at a ceiling (decision 6): R1's account
    /// failover, or a budget fallback to another agent.
    pub fn record_switch(&mut self, switch: ReceiptSwitch) {
        self.switches.push(switch);
    }

    pub fn record_account_choice(&mut self, reason: String) {
        // CFO capacity was tentative. Actual admission evidence owns the
        // account; never publish a contradictory old assignment.
        if let Some(cfo) = self.cfo.as_mut() {
            cfo.account = None;
        }
        if let Some(cfo) = self.context_mut().cfo.as_mut() {
            cfo.account = None;
        }
        self.account_choices.push(reason);
    }

    /// The turn's steers, as it finished (`steering::settle`).
    pub fn set_steers(&mut self, steers: &[crate::steering::SteerRecord]) {
        self.steers = steers
            .iter()
            .map(|steer| {
                let mut text: String = steer.text.chars().take(RECEIPT_STEER_CHARS).collect();
                if steer.text.chars().count() > RECEIPT_STEER_CHARS {
                    text.push('…');
                }
                ReceiptSteer {
                    at: steer.at.clone(),
                    text,
                    delivered: steer.state == crate::steering::SteerState::Delivered,
                }
            })
            .collect();
    }

    /// The moves recorded so far (tests read the login override's row).
    #[cfg(test)]
    pub fn switches(&self) -> &[ReceiptSwitch] {
        &self.switches
    }

    /// Record the run's cost with its descendants' (F10, D2); one with no
    /// descendant records nothing.
    pub fn set_descendants(&mut self, rollup: crate::run_manifest::CostRollup) {
        self.descendants = (!rollup.descendants.is_empty()).then_some(rollup);
    }

    /// Record the run's calls and context (OB05, `RunUsage::calls`); stats
    /// with no figure in them record nothing.
    pub fn set_api_usage(&mut self, stats: crate::usage::CallStats) {
        self.api_usage = (!stats.is_empty()).then_some(stats);
    }

    /// Record how many times the agent compacted its context in the run
    /// (OB22, `ParseState::compactions`): on the receipt and in the run
    /// manifest's capabilities when more than none.
    pub fn set_compactions(&mut self, count: u32) {
        self.compactions = count;
    }

    /// Record the services the run started (`services::take_started_by_run`).
    pub fn set_services(&mut self, services: Vec<ReceiptService>) {
        self.services = services;
    }

    /// Record the effort the turn asked for and what its agent got (Track U).
    pub fn set_effort(&mut self, effort: ReceiptEffort) {
        self.effort = Some(effort);
    }

    /// Record what the CFO decided (COO rebuild L7): on the receipt and in
    /// the run's manifest.
    pub fn set_cfo(&mut self, decision: crate::cfo::Decision) {
        self.context_mut().cfo = Some(decision.clone());
        self.cfo = Some(decision);
    }

    /// The CFO's decision recorded so far.
    #[cfg(test)]
    pub fn cfo(&self) -> Option<&crate::cfo::Decision> {
        self.cfo.as_ref()
    }

    /// The run's phase timings (Paperclip PR 15c), filled in by the launch
    /// point as each phase ends.
    pub fn timings_mut(&mut self) -> &mut PhaseTimings {
        &mut self.timings
    }

    /// Record how the run ended (Paperclip PR 14a). A failure kind other
    /// than a cancel or a timeout (which the `run` line already words) adds
    /// a `result` failure line to the receipt.
    pub fn set_result_kind(&mut self, kind: crate::result_kind::ResultKind) {
        self.result_kind = Some(kind);
    }

    /// Keep the pre-run repository snapshot. A baseline with no targets is
    /// dropped, so a vault-only run carries nothing.
    pub fn set_repo_baseline(&mut self, baseline: RepoBaseline) {
        // Kept apart: the post-run capture takes the baseline before the
        // receipt is built, and a merge without -R needs the roots (N8).
        self.repo_roots = baseline.roots();
        self.repo_baseline = (!baseline.is_empty()).then_some(baseline);
    }

    /// Mark the baseline usable: called right after the agent spawned, so a
    /// run that never started does no post-run git work.
    pub fn arm_repo_baseline(&mut self) {
        if let Some(baseline) = self.repo_baseline.as_mut() {
            baseline.arm();
        }
    }

    /// Take the baseline for the post-run capture; `None` unless armed.
    pub fn take_repo_baseline(&mut self) -> Option<RepoBaseline> {
        self.repo_baseline.take().filter(RepoBaseline::is_armed)
    }

    /// Record the post-run repository changes (without patches).
    pub fn set_repo_changes(&mut self, repo: Vec<RepoChanges>) {
        self.repo_changes = repo;
    }

    /// Record what ending the run's processes established. `None` when the
    /// run had no owned process group (nothing was checked).
    pub fn set_process_stop(&mut self, outcome: Option<crate::process_group::StopOutcome>) {
        self.process_stop = outcome.map(|outcome| outcome.as_str().to_string());
    }

    /// [`Self::set_process_stop`] combined with what is already recorded:
    /// one unconfirmed step makes the run unconfirmed (run 15 S2: an agent
    /// started early and ended unused is a step of the run).
    pub fn add_process_stop(&mut self, outcome: Option<crate::process_group::StopOutcome>) {
        use crate::process_group::StopOutcome;
        let recorded = self.process_stop.as_deref().and_then(StopOutcome::parse);
        self.set_process_stop(StopOutcome::combine(recorded, outcome));
    }

    /// Record how many processes the agent left running when it exited on
    /// its own (`process_group::Driven::leftovers`, only after a normal
    /// exit; 0 otherwise). With background work in the run, the receipt says
    /// it was cut off ([`ended_mid_work`]).
    /// The largest count of the run's attempts is kept (a failover's first
    /// attempt can leave work running too).
    pub fn set_leftovers(&mut self, leftovers: usize) {
        self.leftovers = self.leftovers.max(leftovers);
    }

    /// A background task `(id, summary)` the CLI reported stopped as it
    /// exited on its own (overnight A3; [`CutTasks`]). Once per id, at most
    /// 20 kept.
    pub fn record_background_stopped(&mut self, task: (String, String)) {
        if self.stopped_background.len() < 20
            && !self.stopped_background.iter().any(|(id, _)| *id == task.0)
        {
            self.stopped_background.push(task);
        }
    }

    /// The vault files this run's own successful structured writes named,
    /// vault-relative (chat concurrency F1, 2026-10-20): what a chat turn
    /// hands History before its finish (`automated_history::note_own_writes`),
    /// so a run that shared its workspace folder is credited only with them.
    /// The same resolution the receipt confirms by ([`own_writes`]).
    pub fn own_written_paths(&self, vault: &Path) -> BTreeSet<String> {
        let cwd = self
            .context
            .as_ref()
            .and_then(|manifest| manifest.run_cwd.as_deref());
        own_writes(vault, cwd, &self.observations)
            .into_iter()
            .map(|write| write.rel)
            .collect()
    }

    pub fn record_tool_use(&mut self, id: &str, name: &str, input: &Value) {
        self.sequence += 1;
        let id = if id.trim().is_empty() {
            format!("anonymous-{}", self.sequence)
        } else {
            id.to_string()
        };
        self.observations.push(ToolObservation {
            id,
            name: name.to_string(),
            input: input.clone(),
            started_sequence: self.sequence,
            completed_sequence: None,
            result: None,
            is_error: false,
            pr_url: None,
            check_text: Vec::new(),
            hits: Vec::new(),
        });
    }

    /// Stamped once, before the spawn, from the key Hollow minted. Unlike
    /// `set_source_id` this never has to wait for the agent's stream.
    pub fn set_session_key(&mut self, session_key: &str) {
        self.session_key = Some(session_key.to_string());
    }

    pub fn set_source_id(&mut self, source_id: Option<&str>) {
        self.source_id = source_id.map(str::to_string);
    }

    pub fn record_tool_result(&mut self, id: &str, content: &str, is_error: bool) {
        self.sequence += 1;
        if let Some(item) = self
            .observations
            .iter_mut()
            .rev()
            .find(|item| item.id == id)
        {
            item.completed_sequence = Some(self.sequence);
            item.pr_url = crate::overnight_summary::pr_url(content);
            item.check_text = command_from_observation(item)
                .map(|command| text_outcomes(&command, content))
                .unwrap_or_default();
            item.hits = crate::cite_check::grep_hits(&item.name, &item.input, content);
            item.result = Some(cap(content, EXCERPT_CHARS));
            item.is_error = is_error;
        }
    }

    /// The receipt's outcome of the checks tool call `id` holds, read from
    /// its full result (`failed` when any failed, `passed` when every one
    /// passed, else `unknown`); None for a call that holds no check. The
    /// step spine keeps it on the call's step (`checkOutcome`), which the
    /// task-state record reads (OF16 re-review).
    pub fn check_outcome(&self, id: &str) -> Option<&'static str> {
        let item = self.observations.iter().rev().find(|item| item.id == id)?;
        let command = command_from_observation(item)?;
        let checks = observed_checks(item, &command);
        if checks.is_empty() {
            return None;
        }
        Some(combined_outcome(checks.into_iter().map(|(_, o)| o)))
    }

    /// [`EvidenceReceipt::cites`] (repo features R5): the final reply's cites
    /// held against the run's successful reads and grep hits (a background
    /// call's left out: its result isn't what it read), the earlier turns'
    /// reads, the run's folder (`manifest.run_cwd`, as `notes_opened` reads
    /// it) and whether every read could be seen (a delegation ran, or the
    /// agent reports no tool calls). `None` when the reply cited nothing.
    fn reply_cites_check(&self, vault: &Path) -> Option<crate::cite_check::ReceiptCites> {
        use crate::cite_check::{check, hit_window, read_windows, Ctx, MAX_HITS};
        if self.reply_cites.is_empty() {
            return None;
        }
        let cwd = self
            .context
            .as_ref()
            .and_then(|manifest| manifest.run_cwd.as_deref());
        let mut reads = Vec::new();
        let mut hits = 0;
        for call in &self.observations {
            if call.completed_sequence.is_none() || call.is_error || runs_in_background(call) {
                continue;
            }
            reads.extend(read_windows(&call.name, &call.input));
            for (path, line) in call.hits.iter().take(MAX_HITS - hits) {
                reads.push(hit_window(path.clone(), *line));
                hits += 1;
            }
        }
        let earlier = self
            .earlier_reads
            .iter()
            .flat_map(|(name, input)| read_windows(name, input))
            .collect();
        // Codex's `spawn_agent` and Grok's `spawn_subagent` hand work off too.
        let delegated = self.observations.iter().any(|call| {
            is_delegation(&call.name)
                || matches!(call.name.as_str(), "spawn_agent" | "spawn_subagent")
        });
        let ctx = Ctx {
            cwd,
            vault,
            reads,
            earlier,
            delegated,
            unobserved: self.tool_calls_unreported,
        };
        // Counting lines reads up to 64 MiB of files: `finish` runs on an
        // async worker in a chat turn, so tell the runtime it blocks (a
        // current-thread runtime, in tests, can't be told), as `chat.rs`'
        // local-context fetch does.
        let run = || check(&self.reply_cites, &ctx);
        match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
            Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(run),
            _ => run(),
        }
    }

    pub fn finish(
        mut self,
        vault: &Path,
        run_status: &str,
        changes: Vec<ObservedChange>,
    ) -> Result<EvidenceReceipt, String> {
        let repo = dedupe_repo_rows(std::mem::take(&mut self.repo_changes), &changes);
        // `evidenceMs` covers building the receipt, not persisting it.
        let classify_started = Instant::now();
        let cwd = self
            .context
            .as_ref()
            .and_then(|manifest| manifest.run_cwd.clone());
        let mut receipt = classify(
            vault,
            cwd.as_deref(),
            &self.source,
            self.source_id.as_deref(),
            &self.run_id,
            &self.started_at,
            run_status,
            &self.observations,
            changes,
            repo,
        );
        // A merge without -R in a later turn (morning N8): its repository
        // from the command's `cd` folder or the run's own repository.
        fill_pr_repos(
            &mut receipt.pull_requests,
            &self.observations,
            &self.repo_roots,
        );
        receipt.session_key = self.session_key.clone();
        receipt.process_stop = self.process_stop.clone();
        receipt.result_kind = self.result_kind.map(|kind| kind.as_str().to_string());
        receipt.effort = self.effort.clone();
        receipt.cfo = self.cfo.clone();
        receipt.services = self.services.clone();
        receipt.descendants = self.descendants.clone();
        receipt.api_usage = self.api_usage.clone();
        receipt.switches = self.switches.clone();
        receipt.account_choices = self.account_choices.clone();
        receipt.steers = self.steers.clone();
        receipt.canvases = self.canvases;
        // OB22: the run's context compactions, absent when none.
        receipt.compactions = (self.compactions > 0).then_some(self.compactions);
        // Always present on a new receipt (`{}` when it had none), so a
        // reader can tell "recorded nothing" from an older receipt's
        // "unknown".
        let mut outside = crate::run_effects::with_carried(
            outside_effects_of(&self.observations),
            &self.carried_effects,
        )
        .unwrap_or_default();
        if self.tool_calls_unreported {
            outside.unknown.push(crate::run_effects::unreported_calls());
            // Its count would read 0: unknown, not none.
            receipt.tool_calls = None;
        }
        receipt.outside_effects = Some(outside);
        if let Some(kind) = self.result_kind {
            use crate::result_kind::ResultKind;
            if kind.is_failure() && !matches!(kind, ResultKind::Cancelled | ResultKind::TimedOut) {
                let label = kind.label();
                let mut excerpt: String =
                    label.chars().take(1).flat_map(char::to_uppercase).collect();
                excerpt.push_str(&label[label.chars().next().map_or(0, char::len_utf8)..]);
                excerpt.push('.');
                // Just ahead of the `run` line, after every observation.
                receipt.failures.push(ReceiptFailure {
                    label: "result".into(),
                    sequence: u64::MAX - 1,
                    excerpt,
                });
                receipt.failures.sort_by_key(|failure| failure.sequence);
            }
        }
        // Only what Hollow checked is claimed: "verified" means no process it
        // could identify from the run remains, never that none can exist.
        match receipt.process_stop.as_deref() {
            Some("unconfirmed") => {
                let excerpt = if run_status == "success" {
                    "Cleanup unconfirmed: a process started by this run may still be running."
                } else {
                    "Stop unconfirmed: a process started by this run may still be running."
                };
                receipt.failures.push(ReceiptFailure {
                    label: "stop".into(),
                    sequence: self.sequence + 1,
                    excerpt: excerpt.into(),
                });
                receipt.failures.sort_by_key(|failure| failure.sequence);
            }
            Some("verified") if run_status == "cancelled" => {
                if let Some(run) = receipt.failures.iter_mut().find(|f| f.label == "run") {
                    run.excerpt = "Run stopped by request. Stopped (verified): no process Hollow could find from this run remains.".into();
                }
            }
            _ => {}
        }
        // Overnight C3: the Hollow wake-up this turn asked for; a stopped
        // turn's is cancelled (nobody wants a stopped run continued later).
        if run_status == "cancelled" {
            crate::wake_me::cancel_run(vault, &receipt.run_id);
        }
        receipt.scheduled_wake = crate::wake_me::receipt_line(vault, &receipt.run_id);
        // Overnight A3: a turn that ended with its background work still
        // running, or that scheduled a wakeup no Hollow chat answers.
        for (label, excerpt) in
            ended_mid_work(&self.observations, self.leftovers, &self.stopped_background)
        {
            receipt.failures.push(ReceiptFailure {
                label: label.into(),
                sequence: self.sequence + 1,
                excerpt,
            });
        }
        receipt.failures.sort_by_key(|failure| failure.sequence);
        let mut timings = self.timings.clone();
        timings.evidence_ms = Some(ms_since(classify_started));
        receipt.phase_timings = (!timings.is_empty()).then_some(timings);
        // Agent rereads (F3): the run's own `hollow_vault` reads, counted
        // from its tool calls, when the server was attached or called.
        if let Some(manifest) = self.context.as_mut() {
            // The preparation timeline (F8), from the marks the run took and
            // the admission and spawn clocks above.
            manifest.preparation.timeline =
                crate::prep_timing::timeline(&manifest.timer.0, &self.timings.run_clock());
            // OB22: the compactions beside the window the command carried.
            manifest.capabilities.compactions = receipt.compactions;
            let mut reads = crate::run_manifest::AgentReads::default();
            for call in &self.observations {
                if call.name == crate::run_manifest::READ_SOURCE_TOOL {
                    reads.read_source += 1;
                    reads.read_source_failed += u32::from(call.is_error);
                    reads.read_source_queried += u32::from(
                        call.input
                            .get("query")
                            .and_then(Value::as_str)
                            .is_some_and(|q| !q.trim().is_empty()),
                    );
                } else if call.name == crate::run_manifest::SEARCH_VAULT_TOOL {
                    reads.search_vault += 1;
                }
            }
            // S26: the notes it opened outside its folder.
            reads.notes_opened =
                notes_opened(vault, manifest.run_cwd.as_deref(), &self.observations);
            let attached = manifest
                .capabilities
                .owned_servers
                .iter()
                .any(|server| server == "hollow_vault");
            if attached || reads != crate::run_manifest::AgentReads::default() {
                manifest.agent_reads = Some(reads);
            }
            // The skills the agent read (F9, wave 4 · V2).
            if let Some(skills) = manifest.skills.as_mut() {
                for call in &self.observations {
                    skills.observe(&call.name, &call.input, call.is_error);
                }
            }
        }
        // S13: the run's writes into Claude Code's auto-memory folder.
        receipt.memory_writes = self
            .context
            .as_ref()
            .and_then(|manifest| manifest.native_memory.as_ref())
            .map(|native| {
                let writes = self
                    .observations
                    .iter()
                    .filter(|call| {
                        call.completed_sequence.is_some()
                            && !call.is_error
                            && native.is_memory_write(&call.name, &call.input)
                    })
                    .count();
                u32::try_from(writes).unwrap_or(u32::MAX)
            });
        // S27: the final reply's completion claim against the checks seen.
        receipt.verification = self.reply_claim.map(|kind| {
            claim_verification(
                kind,
                self.tool_calls_unreported,
                &self.observations,
                &receipt.checks,
            )
        });
        // R5: the final reply's `path:line` cites against what it read.
        receipt.cites = self.reply_cites_check(vault);
        // The context manifest (F11): encoded now, written with the receipt
        // under the store's lock. A failure costs only the manifest.
        let manifest =
            self.context.as_ref().and_then(|manifest| {
                match crate::run_manifest::encode(manifest) {
                    Ok(encoded) => Some(encoded),
                    Err(error) => {
                        eprintln!("[evidence] run {}: {error}", self.run_id);
                        None
                    }
                }
            });
        persist_with(vault, &mut receipt, manifest)?;
        Ok(receipt)
    }

    /// [`finish`](Self::finish) for a run that may have come through the run
    /// queue (Paperclip PR 15c): the row's `queueWaitMs` is copied onto the
    /// receipt first, and the receipt's other timings are then merged into
    /// that row's `phase_timings_json` (`run_queue::merge_phase_timings`,
    /// which never overwrites `queueWaitMs` and touches only a `running`
    /// row; the pump writes the terminal status after the run returns). The
    /// queue steps are best effort: a failure is logged and never changes the
    /// receipt or the run. Call it after `finish_run`, holding no history,
    /// permission or queue lock (it takes the session store's leaf lock).
    pub fn finish_mirrored(
        mut self,
        vault: &Path,
        run_status: &str,
        changes: Vec<ObservedChange>,
    ) -> Result<EvidenceReceipt, String> {
        let history_run_id = self.run_id.clone();
        let mut queued = false;
        match crate::run_queue::queue_wait_for_run(vault, &history_run_id) {
            Ok(wait) => {
                if wait.is_some() {
                    queued = true;
                    self.timings.queue_wait_ms = wait;
                }
            }
            Err(error) => eprintln!("[evidence] queue wait for run {history_run_id}: {error}"),
        }
        let receipt = self.finish(vault, run_status, changes)?;
        if let Some(timings) = receipt.phase_timings.as_ref() {
            match crate::run_queue::merge_phase_timings(vault, &history_run_id, &timings.to_map()) {
                // A row ran this run but is no longer `running`: it was ended
                // before the receipt was written, so the mirror is lost. Say
                // so rather than drop it silently (the pump must end the row
                // only after the run's finalize returns).
                Ok(false) if queued => eprintln!(
                    "[evidence] run {history_run_id} came through the run queue, but its row \
                     had already ended; its phase timings were not mirrored"
                ),
                Ok(_) => {}
                Err(error) => {
                    eprintln!("[evidence] mirroring the timings of run {history_run_id}: {error}")
                }
            }
        }
        Ok(receipt)
    }
}

pub fn list(
    vault: &Path,
    source: Option<&str>,
    source_id: Option<&str>,
    limit: usize,
) -> Result<Vec<EvidenceReceipt>, String> {
    let _guard = FILE_LOCK
        .lock()
        .map_err(|_| "evidence receipt lock poisoned".to_string())?;
    let store = shared_store(vault)?;
    drop(_guard);
    let mut receipts: Vec<EvidenceReceipt> = store
        .receipts
        .iter()
        .filter(|receipt| {
            source.map(|v| receipt.source == v).unwrap_or(true)
                && source_id
                    .map(|v| receipt.source_id.as_deref() == Some(v))
                    .unwrap_or(true)
        })
        .cloned()
        .collect();
    receipts.sort_by(|a, b| b.finished_at.cmp(&a.finished_at));
    receipts.truncate(limit.min(KEEP));
    Ok(receipts)
}

pub fn get(vault: &Path, id: &str) -> Result<EvidenceReceipt, String> {
    list(vault, None, None, KEEP)?
        .into_iter()
        .find(|receipt| receipt.id == id)
        .ok_or_else(|| format!("run record not found: {id}"))
}

/// The receipt of history run `history_run_id` (Paperclip PR 9b's quiet
/// check reads a previous workflow run's `confirmed` writes from it).
/// Read-only, under `FILE_LOCK`. `Ok(None)` when no receipt carries that id,
/// and also when MORE than one does: the store sits in the agent-writable
/// vault, so a duplicate is treated as a forgery and trusted for neither.
/// The caller checks the receipt's source, source id and status.
pub fn find_by_history_run(
    vault: &Path,
    history_run_id: &str,
) -> Result<Option<EvidenceReceipt>, String> {
    if history_run_id.trim().is_empty() {
        return Ok(None);
    }
    let _guard = FILE_LOCK
        .lock()
        .map_err(|_| "evidence receipt lock poisoned".to_string())?;
    let store = shared_store(vault)?;
    let mut matches = store
        .receipts
        .iter()
        .filter(|receipt| receipt.history_run_id == history_run_id);
    let first = matches.next();
    if matches.next().is_some() {
        return Ok(None);
    }
    Ok(first.cloned())
}

/// Run `f` over every stored receipt (oldest first) without copying them:
/// the rail's line totals and a conversation's receipts only need a scan.
pub fn with_receipts<T>(
    vault: &Path,
    f: impl FnOnce(&[EvidenceReceipt]) -> T,
) -> Result<T, String> {
    let store = {
        let _guard = FILE_LOCK
            .lock()
            .map_err(|_| "evidence receipt lock poisoned".to_string())?;
        shared_store(vault)?
    };
    Ok(f(&store.receipts))
}

/// Drop a nested repository's rows that vault history already recorded (its
/// `vault_prefix/path` is in the vault change set), so nothing is counted
/// twice; `total_files` drops with them. Only a repo nested in the vault has
/// a prefix.
fn dedupe_repo_rows(mut repo: Vec<RepoChanges>, changes: &[ObservedChange]) -> Vec<RepoChanges> {
    let vault_paths: HashSet<&str> = changes.iter().map(|change| change.path.as_str()).collect();
    for entry in &mut repo {
        let Some(prefix) = entry.vault_prefix.clone() else {
            continue;
        };
        let before = entry.files.len();
        entry
            .files
            .retain(|file| !vault_paths.contains(format!("{prefix}/{}", file.path).as_str()));
        let dropped = before - entry.files.len();
        entry.total_files = entry.total_files.saturating_sub(dropped);
    }
    repo
}

#[allow(clippy::too_many_arguments)]
fn classify(
    vault: &Path,
    cwd: Option<&Path>,
    source: &str,
    source_id: Option<&str>,
    run_id: &str,
    started_at: &str,
    run_status: &str,
    observations: &[ToolObservation],
    mut changes: Vec<ObservedChange>,
    mut repo: Vec<RepoChanges>,
) -> EvidenceReceipt {
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    let recoverable = history_recoverable(&changes);
    let mut mutation_paths: BTreeMap<String, (&str, u64)> = BTreeMap::new();
    for (observation, paths) in own_write_calls(observations) {
        let sequence = observation
            .completed_sequence
            .unwrap_or(observation.started_sequence);
        for path in paths {
            mutation_paths.insert(path, (&observation.name, sequence));
        }
    }

    let mut confirmed_sequences = Vec::new();
    // Repository rows first: a mutation path matched to one never also
    // confirms a vault change (the old suffix match let an absolute repo
    // write `/…/app/README.md` confirm the vault's `README.md`).
    let mut repo_matched: BTreeSet<String> = BTreeSet::new();
    let mut repo_unknown = false;
    let mut repo_changed = false;
    for entry in &mut repo {
        if entry.status != "captured" {
            repo_unknown = true;
            repo_changed = true;
        }
        if entry.total_files > 0 {
            repo_changed = true;
        }
        if entry.total_files > entry.files.len() {
            repo_unknown = true;
        }
        let aliases = entry.alias_roots.clone();
        let cwd_in_repo = entry.cwd_in_repo.clone();
        for file in &mut entry.files {
            match find_repo_path_match(
                &file.path,
                &aliases,
                cwd_in_repo.as_deref(),
                &mutation_paths,
            ) {
                Some((key, tool, seq)) => {
                    repo_matched.insert(key);
                    confirmed_sequences.push(seq);
                    file.attribution = "confirmed".into();
                    file.tool = Some(tool.to_string());
                }
                None => {
                    repo_unknown = true;
                    file.attribution = "observed".into();
                    file.tool = None;
                }
            }
        }
    }
    // The writes as the vault files they reach (chat concurrency F1,
    // 2026-10-20): the only thing that confirms a vault change, and what
    // History's unattributed marks were measured against. A spelling matched
    // to a repository row never also confirms one. (The old suffix matcher
    // let a relative `workspaces/w1/report.md`, written from inside
    // `workspaces/w1`, confirm the folder's own `report.md`.)
    let own = own_write_map(vault, cwd, observations, &repo_matched);

    let mut files = Vec::new();
    let mut any_observed_only = false;
    // Files credited to this run: all but the shared ones.
    let mut own_files = 0usize;
    for (index, change) in changes.into_iter().enumerate() {
        let matched = own_entry(vault, &own, &change.path).map(|(tool, seq)| (tool.as_str(), *seq));
        // Line counts only for the first files a chat shows; each reads up
        // to two history objects.
        let stats = (index < LINE_STATS_FILES
            && (change.before_hash.is_some() || change.after_hash.is_some()))
        .then(|| {
            crate::automated_history::line_stats(
                vault,
                change.before_hash.as_deref(),
                change.after_hash.as_deref(),
            )
        })
        .flatten();
        let (additions, deletions) = stats.map_or((None, None), |(a, d)| (Some(a), Some(d)));
        let not_recoverable = change.recoverable == Some(false);
        if let Some((tool, seq)) = matched {
            confirmed_sequences.push(seq);
            own_files += 1;
            files.push(ReceiptFile {
                path: change.path,
                change_type: change.change_type,
                attribution: "confirmed".into(),
                tool: Some(tool.to_string()),
                additions,
                deletions,
                not_recoverable,
                also_changed_by: change.also_changed_by,
                hollow_write: change.hollow_write,
            });
        } else if change.shared {
            // Another run's change in the shared folder (F1): listed, never
            // credited, and no part of this run's verification state.
            files.push(ReceiptFile {
                path: change.path,
                change_type: change.change_type,
                attribution: ATTRIBUTION_SHARED.into(),
                tool: None,
                additions,
                deletions,
                not_recoverable,
                also_changed_by: change.also_changed_by,
                hollow_write: change.hollow_write,
            });
        } else {
            any_observed_only = true;
            own_files += 1;
            files.push(ReceiptFile {
                path: change.path,
                change_type: change.change_type,
                attribution: "observed".into(),
                tool: None,
                additions,
                deletions,
                not_recoverable,
                also_changed_by: change.also_changed_by,
                hollow_write: change.hollow_write,
            });
        }
    }

    let mut checks = Vec::new();
    let mut failures = Vec::new();
    for item in observations {
        let seq = item.completed_sequence.unwrap_or(item.started_sequence);
        if let Some(command) = command_from_observation(item) {
            // The result excerpt is stored only for a call the strict rule
            // recognised before OF16: a script's checks, and the newly strict
            // gate, `node --test` and Playwright, add verdicts to the store,
            // never more tool output. Each check stores its own words, not
            // the script (no `TOKEN=…`, no here-document body).
            let strict = check_kind(&command).is_some();
            for (check, outcome) in observed_checks(item, &command) {
                let newly_strict = check.kind == "gate"
                    || matches!(check.reader, Reader::Runner("node-test" | "playwright"));
                checks.push(ReceiptCheck {
                    command: cap(&check.words, 320),
                    kind: check.kind.into(),
                    outcome: outcome.into(),
                    sequence: seq,
                    excerpt: item.result.clone().filter(|_| strict && !newly_strict),
                });
            }
        }
        if item.is_error {
            failures.push(ReceiptFailure {
                label: item.name.clone(),
                sequence: seq,
                excerpt: item
                    .result
                    .clone()
                    .unwrap_or_else(|| "Tool reported an error.".into()),
            });
        }
    }
    let tail_sequence = observations
        .iter()
        .map(|item| item.completed_sequence.unwrap_or(item.started_sequence))
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for entry in repo.iter().filter(|entry| entry.status != "captured") {
        failures.push(ReceiptFailure {
            label: "repo".into(),
            sequence: tail_sequence,
            excerpt: format!(
                "Repository changes in {} could not be recorded: {}",
                entry.label,
                entry.error.as_deref().unwrap_or("unknown error")
            ),
        });
    }
    checks.sort_by_key(|check| check.sequence);
    failures.sort_by_key(|failure| failure.sequence);

    if run_status != "success" && !failures.iter().any(|f| f.label == "run") {
        failures.push(ReceiptFailure {
            label: "run".into(),
            sequence: u64::MAX,
            excerpt: format!("Run finished with status {run_status}."),
        });
    }

    // A repo-only run is never `noChanges`: that needs no vault file and every
    // repository captured with nothing changed.
    let verification_state = if own_files == 0 && !repo_changed {
        "noChanges"
    } else if any_observed_only || repo_unknown || confirmed_sequences.is_empty() {
        "unknownOrder"
    } else {
        let final_write = *confirmed_sequences.iter().max().unwrap_or(&0);
        let later_checks: Vec<_> = checks
            .iter()
            .filter(|check| check.sequence > final_write)
            .collect();
        if later_checks.iter().any(|check| check.outcome == "failed") {
            "failedAfterChange"
        } else if later_checks.iter().any(|check| check.outcome == "unknown") {
            "notVerified"
        } else if later_checks.iter().any(|check| check.outcome == "passed") {
            "verifiedAfterChange"
        } else if checks.is_empty() {
            "notVerified"
        } else {
            "staleAfterChange"
        }
    };

    // The receipt keeps a bounded list per repository; `total_files` keeps
    // the real count and the run's sidecar the longer list with patches.
    for entry in &mut repo {
        entry.files.truncate(RECEIPT_REPO_FILES);
        for file in &mut entry.files {
            file.patch = None;
        }
    }

    let finished_at = Utc::now().to_rfc3339();
    let id = receipt_id(source, run_id, &finished_at);
    EvidenceReceipt {
        id,
        run_id: run_id.to_string(),
        source: source.to_string(),
        source_id: source_id.map(str::to_string),
        started_at: started_at.to_string(),
        finished_at,
        status: run_status.to_string(),
        verification_state: verification_state.into(),
        files,
        checks,
        failures,
        history_run_id: run_id.to_string(),
        repo_changes: repo,
        history_recoverable: recoverable,
        pull_requests: pull_requests_of(observations),
        tool_calls: Some(u32::try_from(observations.len()).unwrap_or(u32::MAX)),
        ..EvidenceReceipt::default()
    }
}

/// [`EvidenceReceipt::outside_effects`] from the run's tool calls.
fn outside_effects_of(
    observations: &[ToolObservation],
) -> Option<crate::run_effects::OutsideEffects> {
    let calls: Vec<crate::run_effects::Call<'_>> = observations
        .iter()
        .map(|o| crate::run_effects::Call {
            name: &o.name,
            input: &o.input,
            result: o.result.as_deref(),
            is_error: o.is_error,
            sequence: o.completed_sequence.unwrap_or(o.started_sequence),
        })
        .collect();
    crate::run_effects::classify(&calls)
}

/// The vault notes the run's agent opened outside its folder `cwd` (S26,
/// `agentReads.notesOpened`): the `.md` files its successful `Read` and
/// `read_source` calls and its shell `cat`/`sed`/`head`/`tail` commands
/// named, vault-relative, once each in first-read order, at most
/// [`crate::run_manifest::MAX_NOTES_OPENED`]. A file outside the vault (or
/// gone by now), one under `cwd`, a workspace's own `sources/…` and anything
/// in a restricted folder, by the name read or by its real path (a symlinked
/// `library/shared -> ../restricted`), are left out. Nothing without a
/// `cwd`: with no folder, "outside it" means nothing.
fn notes_opened(vault: &Path, cwd: Option<&Path>, observations: &[ToolObservation]) -> Vec<String> {
    let Some(cwd) = cwd else {
        return Vec::new();
    };
    let Ok(real_vault) = fs::canonicalize(vault) else {
        return Vec::new();
    };
    // Any folder named `restricted`, in any case.
    let in_restricted = |rel: &Path| {
        rel.components()
            .any(|part| part.as_os_str().eq_ignore_ascii_case("restricted"))
    };
    // The paths as given and their real forms (`/var` is `/private/var`).
    let with_real = |p: &Path| {
        let mut forms = vec![p.to_path_buf()];
        forms.extend(fs::canonicalize(p).ok().filter(|real| real != p));
        forms
    };
    let vaults = with_real(vault);
    let cwds = with_real(cwd);
    let mut out: Vec<String> = Vec::new();
    for call in observations {
        let arg = |key: &str| call.input.get(key).and_then(Value::as_str);
        let named: Vec<PathBuf> = if call.name == "Read" {
            arg("file_path")
                .filter(|_| !call.is_error)
                .map(|p| resolve_read(cwd, p))
                .into_iter()
                .collect()
        } else if call.name == crate::run_manifest::READ_SOURCE_TOOL {
            // Vault-relative, or a connected project's absolute path;
            // `sources/…` is the workspace's own attached source.
            arg("path")
                .filter(|p| !call.is_error && !p.trim().starts_with("sources/"))
                .map(|p| resolve_read(vault, p))
                .into_iter()
                .collect()
        } else {
            crate::run_effects::shell_note_reads(&call.name, &call.input)
                .iter()
                .map(|p| resolve_read(cwd, p))
                .collect()
        };
        for path in named {
            if out.len() >= crate::run_manifest::MAX_NOTES_OPENED {
                return out;
            }
            let markdown = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
            if !markdown || cwds.iter().any(|dir| path.starts_with(dir)) {
                continue;
            }
            let Some(rel) = vaults.iter().find_map(|v| path.strip_prefix(v).ok()) else {
                continue;
            };
            let shown = rel.to_string_lossy().replace('\\', "/");
            if in_restricted(rel) || out.contains(&shown) {
                continue;
            }
            // The file it really is: still in the vault, still a file, and
            // not in a restricted folder there either.
            let Ok(real) = fs::canonicalize(vault.join(rel)) else {
                continue;
            };
            let allowed = real
                .strip_prefix(&real_vault)
                .is_ok_and(|r| !in_restricted(r));
            if !allowed || !real.is_file() {
                continue;
            }
            out.push(shown);
        }
    }
    out
}

/// `path` as a tool call named it, read from `base` when relative (`~/`
/// from the home folder), with `.` and `..` taken out lexically.
fn resolve_read(base: &Path, path: &str) -> PathBuf {
    let path = path.trim();
    let joined = match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => base.join(path),
    };
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// [`EvidenceReceipt::verification`] (S27): what the run's final reply
/// claimed about its checks, held against the checks the receipt saw. The
/// flags are true only when a claim was made and the receipt can tell; when
/// it can't (a subagent or other delegation may have run the check, or the
/// agent reports no tool calls), they stay false.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptVerification {
    /// `passed`, `done` or `fixed` when the final reply claims that
    /// ([`reply_claim_kind`]), else `none`.
    pub claim: String,
    /// A pass claim with no recognised check after the run's last
    /// successful file write ([`is_claim_write`]; or, with no write, none in
    /// the whole run).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub claim_without_check: bool,
    /// A pass claim whose last recognised check failed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub claim_after_fail: bool,
}

/// The phrases with which a final reply claims its checks pass (S27),
/// matched lowercased on word boundaries by [`pass_claim`]. Kept short and
/// conservative on purpose: a claim missed costs less than one invented.
/// Plural `tests`/`checks` only for the bare verb (`test pass rate` is no
/// claim); the last one is a pasted runner summary (`42 passed; 0 failed`).
const PASS_CLAIMS: &[&str] = &[
    r"tests (?:all )?pass(?:es|ed|ing)?",
    r"tests are (?:all )?(?:passing|green)",
    r"test (?:passes|passed)",
    r"checks (?:all )?pass(?:es|ed|ing)?",
    r"checks are (?:all )?(?:passing|green)",
    r"check (?:passes|passed)",
    r"(?:test )?suite (?:passes|passed)",
    r"all green",
    r"gate (?:passed|passes|is green)",
    r"build (?:passed|passes)",
    r"[1-9]\d* (?:tests? )?passed[,;]? 0 failed",
];

/// Words that make a [`PASS_CLAIMS`] phrase a negation, a condition or an
/// instruction rather than a claim (`not all tests pass`, `if the tests
/// pass`, `make sure the tests pass`) when one of them is among the
/// [`HEDGE_WINDOW`] words before it in its clause. Any word ending in `n't`
/// counts too.
const CLAIM_HEDGES: &[&str] = &[
    "not", "no", "never", "none", "nor", "without", "0", "zero", "if", "unless", "until", "once",
    "whether", "when", "sure", "ensure", "verify", "should", "would", "could", "might", "may",
    "can", "will", "expect", "hope", "assuming", "nothing", "dont", "doesnt", "didnt", "isnt",
    "arent", "wont", "cant",
];

/// How many words before a [`PASS_CLAIMS`] phrase [`CLAIM_HEDGES`] are
/// looked for in (S27 review: six, so `not all of the unit tests pass`
/// reads as a negation). Wider misses more claims, never invents one.
const HEDGE_WINDOW: usize = 6;

/// An inline code span (`` `all tests pass` ``) or a double-quoted string
/// (`"all tests pass"`) on one line: a reply quoting a phrase, not claiming
/// it (S27 review). A single backtick only, so a fenced block (a pasted
/// runner summary) is kept.
static QUOTED_SPAN: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r#"`[^`\n]+`|"[^"\n]*""#).expect("QUOTED_SPAN compiles"));

static PASS_CLAIM: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(&format!(r"\b(?:{})\b", PASS_CLAIMS.join("|"))).expect("PASS_CLAIMS compiles")
});

/// A reply that says the work is done, without claiming a check passed.
/// A bare `done` counts only at the start of the reply or of a sentence, so
/// "the done state" is not a claim. `it's done`, `all done` and `I'm done`
/// count anywhere a pass phrase would.
static DONE_CLAIM: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(
        r"(?:\b(?:it(?:'s| is)|that(?:'s| is)|this is|we(?:'re| are)|i(?:'m| am)|all) done\b|(?:^|[\n.])\s*done\b)",
    )
    .expect("DONE_CLAIM compiles")
});

/// A reply that says a bug or the work is fixed. Kept to the short forms
/// (`it's fixed`, `I fixed it`, `I fixed the …`, `fixed the bug`), so a
/// mention of a fix in passing is not a claim.
static FIXED_CLAIM: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(
        r"\b(?:(?:it(?:'s| is)|that(?:'s| is)|this is|all) fixed|i fixed (?:it|this|that|the)|fixed (?:it|the bug|this|that))\b",
    )
    .expect("FIXED_CLAIM compiles")
});

/// What a final reply claims. A pass phrase wins when the same reply also
/// says done or fixed (`Done. The gate is green.`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyClaim {
    None,
    Passed,
    Done,
    Fixed,
}

/// A non-zero failure count (`2 failed`), which undoes a claim in the same
/// sentence (`10 tests passed, 2 failed`).
static FAILED_COUNT: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"\b[1-9]\d* (?:tests? )?(?:failed|failing|failures?)\b")
        .expect("FAILED_COUNT compiles")
});

fn normalize_claim_text(reply: &str) -> String {
    let text = reply
        .replace('\u{2019}', "'")
        .replace(['\u{201C}', '\u{201D}'], "\"")
        .to_lowercase();
    QUOTED_SPAN.replace_all(&text, " ").into_owned()
}

/// Whether `text` (already [`normalize_claim_text`]) holds one phrase of
/// `re` that is a claim: outside a hedge, not a question, and with no
/// failure count or `except` in the same sentence.
fn phrase_is_claim(text: &str, re: &regex::Regex) -> bool {
    re.find_iter(text).any(|m| {
        // A done phrase may include the sentence boundary that introduces it
        // (`. Done.`). Hedges are read from the word, so the previous
        // sentence's `no` does not cancel the claim.
        let phrase = m.start()
            + text[m.start()..m.end()]
                .find(|c: char| c.is_alphanumeric())
                .unwrap_or(0);
        let before = &text[..phrase];
        let clause = before
            .rfind(['.', '!', '?', '\n', ';', ':'])
            .map_or(before, |i| &before[i + 1..]);
        let hedged = clause
            .split(|c: char| !(c.is_alphanumeric() || c == '\''))
            .filter(|w| !w.is_empty())
            .rev()
            .take(HEDGE_WINDOW)
            .any(|w| CLAIM_HEDGES.contains(&w) || w.ends_with("n't"));
        let after = &text[m.end()..];
        let end = after.find(['.', '!', '?', '\n']);
        let rest = end.map_or(after, |i| &after[..i]);
        let question = end.is_some_and(|i| after[i..].starts_with('?'));
        !hedged && !question && !rest.contains("except") && !FAILED_COUNT.is_match(rest)
    })
}

/// Whether `reply` claims its checks pass (S27): a [`PASS_CLAIMS`] phrase.
fn pass_claim(reply: &str) -> bool {
    phrase_is_claim(&normalize_claim_text(reply), &PASS_CLAIM)
}

/// A pass claim, else a done or fixed claim, else none. A pass phrase wins.
fn reply_claim_kind(reply: &str) -> ReplyClaim {
    if pass_claim(reply) {
        ReplyClaim::Passed
    } else {
        let text = normalize_claim_text(reply);
        if phrase_is_claim(&text, &DONE_CLAIM) {
            ReplyClaim::Done
        } else if phrase_is_claim(&text, &FIXED_CLAIM) {
            ReplyClaim::Fixed
        } else {
            ReplyClaim::None
        }
    }
}

/// A call that hands work to another agent: a Claude Code subagent (`Task`,
/// `Agent`), a Workflow-tool run, or Hollow's consultations and child tasks.
fn is_delegation(name: &str) -> bool {
    let tool = normalize_tool(name);
    matches!(tool.as_str(), "task" | "agent" | "workflow")
        || ["askagent", "startsession", "sendtosession"]
            .iter()
            .any(|suffix| tool.ends_with(suffix))
}

/// A file write for [`claim_verification`]'s ordering (S27 review): an
/// edit tool ([`is_mutation`]: Edit, Write, MultiEdit, NotebookEdit,
/// apply_patch) or Codex's `file_change` item, which `parse_codex` surfaces
/// under its item type (`file_change`, normalized `filechange`) unless its
/// item says it failed. Shell writes (`sed -i`, a redirect) are not counted:
/// a conservative miss.
fn is_claim_write(o: &ToolObservation) -> bool {
    is_mutation(&o.name)
        || (normalize_tool(&o.name) == "filechange"
            && o.input.get("status").and_then(Value::as_str) != Some("failed"))
}

/// A delegation the run never sees finish (S27 review): a Claude subagent
/// started with `run_in_background: true`, whose tool call returns at once
/// while its work (and its own calls) go on after it.
fn runs_in_background(o: &ToolObservation) -> bool {
    o.input.get("run_in_background").and_then(Value::as_bool) == Some(true)
}

/// [`EvidenceReceipt::verification`] for a final reply that does
/// (`claimed`) or doesn't claim a pass, from the run's tool calls and the
/// receipt's `checks` (each at its call's completion sequence).
/// `tool_calls_unreported` (Antigravity): the claim is recorded, but with no
/// calls to hold it against, neither flag is raised.
///
/// Checks another agent ran are unknown, never inferred. A Claude
/// subagent's own calls do reach this stream (as ordinary `assistant` /
/// `user` events), but `AgentEvent::ToolUse` / `ToolResult` drop their
/// `parent_tool_use_id`, so here they are indistinguishable from the main
/// thread's: a foreground subagent's land before its delegation call's
/// result, a background one's (`run_in_background`) after it, interleaved
/// with the main thread's or past the turn's end. Other delegations (a
/// Workflow run, Hollow's consultations and child tasks) report nothing
/// here. So a delegation that ended after the last write clears
/// `claimWithoutCheck`, one that ended after the last recognised check
/// clears `claimAfterFail`, and one that never ends here (no result, or
/// started in the background) clears both: the receipt can't tell, so
/// neither flag is raised.
fn claim_verification(
    kind: ReplyClaim,
    tool_calls_unreported: bool,
    observations: &[ToolObservation],
    checks: &[ReceiptCheck],
) -> ReceiptVerification {
    let claim = match kind {
        ReplyClaim::Passed => "passed",
        ReplyClaim::Done => "done",
        ReplyClaim::Fixed => "fixed",
        ReplyClaim::None => "none",
    };
    if kind == ReplyClaim::None || tool_calls_unreported {
        return ReceiptVerification {
            claim: claim.into(),
            ..ReceiptVerification::default()
        };
    }
    let last_write = observations
        .iter()
        .filter(|o| is_claim_write(o) && !o.is_error)
        .filter_map(|o| o.completed_sequence)
        .max()
        .unwrap_or(0);
    let delegated_after = |sequence: u64| {
        observations.iter().any(|o| {
            is_delegation(&o.name)
                && (runs_in_background(o) || o.completed_sequence.unwrap_or(u64::MAX) > sequence)
        })
    };
    let checked_after_write = checks.iter().any(|check| check.sequence > last_write);
    let last_check = checks.iter().map(|check| check.sequence).max();
    let claim_after_fail = last_check.is_some_and(|sequence| {
        checks
            .iter()
            .any(|check| check.sequence == sequence && check.outcome == "failed")
            && !delegated_after(sequence)
    });
    ReceiptVerification {
        claim: claim.into(),
        claim_without_check: !checked_after_write && !delegated_after(last_write),
        claim_after_fail,
    }
}

/// One pull request a turn merged or opened (overnight F8).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptPullRequest {
    pub number: u64,
    /// `owner/repo`, when the command or `gh`'s output named it.
    #[serde(default)]
    pub repo: Option<String>,
    /// `merged` | `opened`.
    pub action: String,
    /// The PR's GitHub page, when the repository is known.
    #[serde(default)]
    pub url: Option<String>,
}

/// The pull requests in a turn's shell calls: each `gh pr merge <n>` or
/// `gh pr create` that finished without an error (a create's number from the
/// URL `gh` printed); one row per PR, merged winning over opened, and a merge
/// with no repository taking its PR's from the open.
fn pull_requests_of(observations: &[ToolObservation]) -> Vec<ReceiptPullRequest> {
    use crate::overnight_summary::{pr_command, shell_command};
    let mut found: Vec<ReceiptPullRequest> = Vec::new();
    for o in observations {
        if o.is_error || o.result.is_none() {
            continue;
        }
        let Some((action, number, repo)) = shell_command(&o.name, &o.input)
            .as_deref()
            .and_then(pr_command)
        else {
            continue;
        };
        let url = o.pr_url.clone();
        let (repo, number) = match (action, number, url) {
            ("merged", Some(n), url) => (repo.or(url.map(|(r, _)| r)), n),
            ("opened", None, Some((r, n))) => (repo.or(Some(r)), n),
            _ => continue,
        };
        found.push(ReceiptPullRequest {
            number,
            repo,
            action: action.into(),
            url: None,
        });
    }
    let opened: Vec<(u64, Option<String>)> = found
        .iter()
        .filter(|p| p.action == "opened")
        .map(|p| (p.number, p.repo.clone()))
        .collect();
    for p in found
        .iter_mut()
        .filter(|p| p.action == "merged" && p.repo.is_none())
    {
        p.repo = opened
            .iter()
            .find(|(n, _)| *n == p.number)
            .and_then(|(_, r)| r.clone());
    }
    let mut out: Vec<ReceiptPullRequest> = Vec::new();
    for p in found {
        match out
            .iter_mut()
            .find(|q| q.number == p.number && q.repo == p.repo)
        {
            Some(q) if q.action == "opened" && p.action == "merged" => q.action = "merged".into(),
            Some(_) => {}
            None => out.push(p),
        }
    }
    out.truncate(20);
    for p in &mut out {
        // Only `owner/repo` is a github.com repository (`-R` also takes
        // `HOST/OWNER/REPO` for GitHub Enterprise).
        p.url = p
            .repo
            .as_ref()
            .filter(|r| r.matches('/').count() == 1)
            .map(|r| format!("https://github.com/{r}/pull/{}", p.number));
    }
    out
}

/// Where the shell stands, as far as the turn's commands say.
#[derive(Debug, Clone, PartialEq)]
enum Place {
    /// No `cd` yet: the run's own folder.
    Start,
    At(std::path::PathBuf),
    /// A `cd` that can't be followed (a variable, `..` from nowhere, `popd`).
    Unknown,
    /// After a `( … )` / `{ … }` group: nothing later in the command can be
    /// placed (a `cd` inside the group isn't told apart from one after it).
    Lost,
}

/// Follow one simple command's effect on where the shell stands.
fn step(place: &Place, words: &[String]) -> Place {
    if *place == Place::Lost {
        return Place::Lost;
    }
    match words.first().map(String::as_str) {
        Some("cd" | "pushd") => {
            let Some(target) = words.get(1).filter(|t| !t.starts_with('-')) else {
                return Place::Unknown;
            };
            let target = match target.strip_prefix("~/") {
                Some(rest) => match dirs::home_dir() {
                    Some(h) => h.join(rest),
                    None => return Place::Unknown,
                },
                None => std::path::PathBuf::from(target),
            };
            if target.to_string_lossy().contains('$') {
                Place::Unknown
            } else if target.is_absolute() {
                Place::At(target)
            } else {
                match place {
                    Place::At(base) => Place::At(base.join(target)),
                    _ => Place::Unknown,
                }
            }
        }
        Some("popd") => Place::Unknown,
        _ => place.clone(),
    }
}

/// Where the shell stood for each `gh pr merge <n>` (without `-R`) of the
/// turn: its number and [`Place`], following `cd`/`pushd` across calls (an
/// agent's shell keeps its folder between them), a Codex call's `workdir`,
/// and scripts inside `bash -c '…'`; a `cd` inside `( … )` doesn't last, so
/// it is left out.
fn merge_places(observations: &[ToolObservation]) -> Vec<(u64, Place)> {
    use crate::overnight_summary::{pr_words, shell_command, simple_commands};
    fn walk(
        script: &str,
        place: &mut Place,
        ok: bool,
        out: &mut Vec<(u64, Place)>,
        moved: &mut bool,
    ) {
        for words in simple_commands(script) {
            let mut w: &[String] = &words;
            while w
                .first()
                .is_some_and(|x| x.contains('=') && !x.starts_with('-'))
            {
                w = &w[1..];
            }
            if let [shell, flag, inner, ..] = w {
                if matches!(
                    shell.as_str(),
                    "bash" | "sh" | "zsh" | "/bin/bash" | "/bin/zsh" | "/bin/sh"
                ) && flag.starts_with('-')
                    && flag.contains('c')
                {
                    // A `bash -c` script runs in a child shell: its `cd`s end with it.
                    let mut inner_place = place.clone();
                    walk(inner, &mut inner_place, ok, out, moved);
                    continue;
                }
            }
            // A `( … )` or `{ … }` group: what its `cd`s leave behind can't be
            // told from words alone.
            if w.first()
                .is_some_and(|x| x.starts_with('(') || x.starts_with('{'))
            {
                *place = Place::Lost;
                *moved = true;
                continue;
            }
            if ok && w.first().map(String::as_str) == Some("gh") {
                if let Some(Some(("merged", Some(n), None))) = pr_words(w) {
                    out.push((n, place.clone()));
                }
            }
            if matches!(w.first().map(String::as_str), Some("cd" | "pushd" | "popd")) {
                *moved = true;
            }
            *place = step(place, w);
        }
    }
    // Each call starts over: a subagent's shell and a fresh Codex or Gemini
    // shell don't share the main shell's folder, and the recorded calls don't
    // say which thread made them. So only a merge command's own `cd` (or its
    // call's `workdir`) counts, and when any call of the turn moved, a merge
    // with none of its own can't be placed.
    let mut out = Vec::new();
    let mut moved = false;
    for o in observations {
        let Some(command) = shell_command(&o.name, &o.input) else {
            continue;
        };
        let mut place = match ["workdir", "cwd"]
            .iter()
            .find_map(|k| o.input.get(*k).and_then(Value::as_str))
            .map(std::path::PathBuf::from)
            .filter(|d| d.is_absolute())
        {
            Some(dir) => Place::At(dir),
            None => Place::Start,
        };
        walk(
            &command,
            &mut place,
            !o.is_error && o.result.is_some(),
            &mut out,
            &mut moved,
        );
        moved |= place != Place::Start;
    }
    if moved {
        for (_, p) in &mut out {
            if *p == Place::Start {
                *p = Place::Unknown;
            }
        }
    }
    out
}

/// `host/owner/name` as a pull request's repository: `owner/name` on
/// github.com, the whole of it on another host (GitHub Enterprise, unlinked).
fn repo_from_remote(remote: &str) -> String {
    remote
        .strip_prefix("github.com/")
        .map(str::to_string)
        .unwrap_or_else(|| remote.to_string())
}

/// Give a merged pull request with no repository (a `gh pr merge <n>` without
/// `-R`, often in a later turn than its create) the repository `gh` used:
/// `gh`'s base remote (`build_workspaces::gh_remote_of`) of the folder the
/// shell stood in for that merge, else (no `cd` in the turn) of the run's
/// one watched repository. Nothing is guessed when the turn set `GH_REPO`,
/// the folder can't be followed, or there are several watched repositories.
/// Then link it when that is on github.com.
fn fill_pr_repos(
    prs: &mut [ReceiptPullRequest],
    observations: &[ToolObservation],
    roots: &[std::path::PathBuf],
) {
    use crate::overnight_summary::shell_command;
    if observations
        .iter()
        .filter_map(|o| shell_command(&o.name, &o.input))
        .any(|c| c.contains("GH_REPO"))
    {
        return;
    }
    let places = merge_places(observations);
    for p in prs
        .iter_mut()
        .filter(|p| p.repo.is_none() && p.action == "merged")
    {
        let dir = match places
            .iter()
            .find(|(n, _)| *n == p.number)
            .map(|(_, pl)| pl)
        {
            Some(Place::At(d)) => Some(d.clone()),
            Some(Place::Start) => match roots {
                [only] => Some(only.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(remote) = dir
            .as_deref()
            .and_then(crate::build_workspaces::gh_remote_of)
        {
            let repo = repo_from_remote(&remote);
            p.url = (repo.matches('/').count() == 1)
                .then(|| format!("https://github.com/{repo}/pull/{}", p.number));
            p.repo = Some(repo);
        }
    }
}

fn is_mutation(name: &str) -> bool {
    matches!(
        normalize_tool(name).as_str(),
        "write" | "edit" | "multiedit" | "notebookedit" | "applypatch"
    )
}

fn normalize_tool(name: &str) -> String {
    name.rsplit([':', '.'])
        .next()
        .unwrap_or(name)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn paths_from_mutation(name: &str, input: &Value) -> Vec<String> {
    let normalized = normalize_tool(name);
    if normalized == "applypatch" {
        let body = input
            .as_str()
            .or_else(|| input.get("patch").and_then(Value::as_str));
        return body
            .into_iter()
            .flat_map(|body| body.lines())
            .filter_map(|line| {
                // A rename's destination too (`*** Move to:`, F1 review).
                [
                    "*** Add File: ",
                    "*** Update File: ",
                    "*** Delete File: ",
                    "*** Move to: ",
                ]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
                .map(str::trim)
                .map(str::to_string)
            })
            .collect();
    }
    [
        "file_path",
        "path",
        "notebook_path",
        "filePath",
        "notebookPath",
    ]
    .iter()
    .filter_map(|key| input.get(key).and_then(Value::as_str))
    .map(str::to_string)
    .collect()
}

/// The run's successful structured writes with the paths each names, in
/// call order: an edit tool ([`is_mutation`]) with a result and no error,
/// or (chat concurrency F1, 2026-10-20) Codex's `file_change` item unless
/// it says it failed, whose `changes[].path` are the files.
fn own_write_calls(observations: &[ToolObservation]) -> Vec<(&ToolObservation, Vec<String>)> {
    observations
        .iter()
        .filter(|item| !item.is_error && item.completed_sequence.is_some())
        .filter_map(|item| {
            if is_mutation(&item.name) {
                return item
                    .result
                    .is_some()
                    .then(|| (item, paths_from_mutation(&item.name, &item.input)));
            }
            if normalize_tool(&item.name) != "filechange"
                || item.input.get("status").and_then(Value::as_str) == Some("failed")
            {
                return None;
            }
            let paths = item
                .input
                .get("changes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|change| change.get("path").and_then(Value::as_str))
                .map(str::to_string)
                .collect();
            Some((item, paths))
        })
        .collect()
}

/// One path a successful structured write named: as the call spelled it,
/// and the vault file it reaches ([`own_writes`]).
struct OwnWrite {
    raw: String,
    rel: String,
    tool: String,
    sequence: u64,
}

/// Every path the run's own successful structured writes named
/// ([`own_write_calls`]) that reaches a vault file, with that file's
/// vault-relative path (chat concurrency F1, 2026-10-20). A relative path is
/// read from the run's folder `cwd` (the vault without one). The path is
/// resolved by the filesystem before anything is compared ([`real_path`]):
/// a symlink to the vault (`~/Hollow/Vault`) or to a folder inside it, `..`
/// after a symlink, and another letter case or Unicode spelling of an
/// existing file all land on the name History records. A path outside the
/// vault is left out.
fn own_writes(vault: &Path, cwd: Option<&Path>, observations: &[ToolObservation]) -> Vec<OwnWrite> {
    let Ok(real_vault) = fs::canonicalize(vault) else {
        return Vec::new();
    };
    let base = cwd.unwrap_or(vault);
    let mut out = Vec::new();
    for (call, paths) in own_write_calls(observations) {
        let sequence = call.completed_sequence.unwrap_or(call.started_sequence);
        for raw in paths {
            if let Some(rel) = vault_relative(&real_vault, base, &raw) {
                out.push(OwnWrite {
                    raw,
                    rel,
                    tool: call.name.clone(),
                    sequence,
                });
            }
        }
    }
    out
}

/// [`own_writes`] by vault path, each with its latest-completing call's
/// tool and sequence; a raw spelling in `skip` (matched to a repository row
/// already) is left out.
fn own_write_map(
    vault: &Path,
    cwd: Option<&Path>,
    observations: &[ToolObservation],
    skip: &BTreeSet<String>,
) -> BTreeMap<String, (String, u64)> {
    let mut out: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for write in own_writes(vault, cwd, observations) {
        if skip.contains(&write.raw) {
            continue;
        }
        let newer = out
            .get(&write.rel)
            .is_none_or(|(_, seen)| write.sequence >= *seen);
        if newer {
            out.insert(write.rel, (write.tool, write.sequence));
        }
    }
    out
}

/// The entry of `own` for vault path `path`: every name that is the same
/// file (`automated_history::same_file`: exact for an existing file, letter
/// case ignored for a gone one on a case-insensitive volume), and of those
/// the latest-completing write (F1 delta review: write `Notes.md`, check,
/// delete it as `notes.md` is a write after the check).
fn own_entry<'a>(
    vault: &Path,
    own: &'a BTreeMap<String, (String, u64)>,
    path: &str,
) -> Option<&'a (String, u64)> {
    own.iter()
        .filter(|(name, _)| crate::automated_history::same_file(vault, name, path))
        .map(|(_, entry)| entry)
        .max_by_key(|(_, sequence)| *sequence)
}

/// `raw` (a path a tool call named) as a vault-relative path, or `None`
/// outside the vault ([`own_writes`]).
fn vault_relative(real_vault: &Path, base: &Path, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let joined = match (raw.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => base.join(raw),
    };
    let rel = real_path(&joined)?
        .strip_prefix(real_vault)
        .ok()?
        .to_path_buf();
    let shown = rel.to_string_lossy().replace('\\', "/");
    (!shown.is_empty()).then_some(shown)
}

/// `path` as the filesystem resolves it: its longest existing prefix
/// canonicalized (symlinks, `..` after them, the stored case and Unicode
/// form), then the rest, which doesn't exist and so holds no symlink, taken
/// lexically (the file itself may be gone: a delete is a write too).
fn real_path(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let parts: Vec<Component<'_>> = path.components().collect();
    for cut in (1..=parts.len()).rev() {
        let head: PathBuf = parts[..cut].iter().collect();
        let Ok(mut out) = fs::canonicalize(&head) else {
            continue;
        };
        for part in &parts[cut..] {
            match part {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        return Some(out);
    }
    None
}

/// The newest successful mutation that names repository file `repo_path`
/// exactly: as an absolute path under one of the repository's alias roots
/// (its canonical root, the run's folder as given minus its in-repo prefix,
/// the linked project path as stored), or, when the run's folder is inside
/// the repository (`cwd_in_repo`, the folder's prefix such as `src/` or
/// ``), as a path relative to that folder. Returns the matched key too.
fn find_repo_path_match<'a>(
    repo_path: &str,
    alias_roots: &[String],
    cwd_in_repo: Option<&str>,
    paths: &'a BTreeMap<String, (&'a str, u64)>,
) -> Option<(String, &'a str, u64)> {
    let absolute: Vec<String> = alias_roots
        .iter()
        .map(|root| format!("{}/{repo_path}", root.trim_end_matches('/')))
        .collect();
    paths
        .iter()
        .filter(|(path, _)| {
            if path.starts_with('/') {
                absolute.iter().any(|candidate| candidate == path.as_str())
            } else {
                cwd_in_repo.is_some_and(|prefix| {
                    format!("{prefix}{}", path.trim_start_matches("./")) == repo_path
                })
            }
        })
        .map(|(path, value)| (path.clone(), value.0, value.1))
        .max_by_key(|(_, _, sequence)| *sequence)
}

/// Each check shell call `item` (running `command`) holds, with its
/// outcome on the receipt ([`ShellCheck::outcome`]).
fn observed_checks(item: &ToolObservation, command: &str) -> Vec<(ShellCheck, &'static str)> {
    // A check started with `run_in_background` returns at once ("running in
    // background"): its result is not the check's (overnight A3), so it
    // never reads as passed.
    let backgrounded = item.input.get("run_in_background").and_then(Value::as_bool) == Some(true);
    let finished = item.completed_sequence.is_some() && item.result.is_some();
    let mut out = Vec::new();
    for (index, check) in shell_checks(command).into_iter().enumerate() {
        let text = item.check_text.get(index).copied().unwrap_or("unknown");
        let outcome = check.outcome(finished, item.is_error, backgrounded, text);
        out.push((check, outcome));
    }
    out
}

/// One outcome for a call's checks: `failed` when any failed, `passed` when
/// every one passed, else `unknown`.
fn combined_outcome(outcomes: impl IntoIterator<Item = &'static str>) -> &'static str {
    let mut all_passed = true;
    for outcome in outcomes {
        if outcome == "failed" {
            return "failed";
        }
        all_passed &= outcome == "passed";
    }
    if all_passed {
        "passed"
    } else {
        "unknown"
    }
}

fn command_from_observation(item: &ToolObservation) -> Option<String> {
    let tool = normalize_tool(&item.name);
    if !matches!(
        tool.as_str(),
        "bash" | "execcommand" | "shell" | "terminal" | "command"
    ) {
        return None;
    }
    item.input
        .get("command")
        .or_else(|| item.input.get("cmd"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| item.input.as_str().map(str::to_string))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellToken {
    Word(String),
    AndIf,
}

/// Recognize one bounded check invocation, optionally preceded by one or more
/// `cd <path> &&` hops: the strict rule, whose exit status is the check's.
/// This intentionally is not a general shell parser: pipelines,
/// redirections, subshells, backgrounding, command substitution, and every
/// control operator other than the leading `cd` chain are rejected. The
/// task-state record and the outside-effects classifier use it to call a
/// command local check work. Receipts read every check of a script with
/// [`shell_checks`] instead (OF16), and the anti-laundering rule moved there:
/// a check whose exit status something can mask (`npm test || true`, a pipe
/// into `tail`, a background run) is never `passed` without its runner's
/// own positive pass text, from its own runner ([`read_output`]).
pub(crate) fn check_kind(command: &str) -> Option<&'static str> {
    let tokens = tokenize_bounded_shell(command)?;
    let words = strip_cd_prefixes(&tokens)?;
    let words = strip_wrappers(&words);
    if words.iter().any(|word| is_non_executing_flag(word)) {
        return None;
    }
    check_words(words)
}

/// `words` past what only sets up the command after it (overnight 9 Spare
/// 5): assignments, `env` (its `-i`, `-u NAME`), `time` (`-p`), `nohup`,
/// `nice` (`-n N`, `-N`) and `timeout` / `gtimeout` (their flags, then the
/// duration). The exit status is that command's (a timeout's own 124 fails
/// either way). A `timeout` without a duration, or with
/// `--preserve-status` (the program's own status after the timeout), and
/// any unknown flag stop the stripping, so it stays no check.
fn strip_wrappers(words: &[String]) -> &[String] {
    let mut rest = words;
    loop {
        let Some(first) = rest.first() else {
            return rest;
        };
        let at = |i: usize| rest.get(i).map_or("", String::as_str);
        let mut i = 1;
        match first.as_str() {
            word if is_assignment(word) => {}
            "nohup" => {}
            "env" => loop {
                match at(i) {
                    "-i" | "-" | "--ignore-environment" => i += 1,
                    "-u" | "--unset" => i += 2,
                    word if word.starts_with("--unset=") => i += 1,
                    _ => break,
                }
            },
            "time" => {
                if at(1) == "-p" {
                    i = 2;
                }
            }
            "nice" => {
                let flag = at(1);
                let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
                if flag == "-n" {
                    i = 3;
                } else if flag.strip_prefix("-n").is_some_and(digits)
                    || flag.strip_prefix('-').is_some_and(digits)
                    || flag.starts_with("--adjustment=")
                {
                    i = 2;
                }
            }
            "timeout" | "gtimeout" => {
                loop {
                    match at(i) {
                        "-s" | "--signal" | "-k" | "--kill-after" => i += 2,
                        // The program's own status after the timeout: a
                        // program that exits 0 on TERM would read as a pass.
                        "--preserve-status" => return rest,
                        "--foreground" | "-v" | "--verbose" => i += 1,
                        word if word.starts_with("--signal=")
                            || word.starts_with("--kill-after=") =>
                        {
                            i += 1
                        }
                        _ => break,
                    }
                }
                if !at(i).starts_with(|c: char| c.is_ascii_digit()) {
                    return rest;
                }
                i += 1;
            }
            _ => return rest,
        }
        rest = rest.get(i..).unwrap_or_default();
    }
}

/// The command a package runner runs (overnight 9 Spare 5): `npx` and
/// `bunx` (past `-y`, `--yes`, `--no-install`, `-q`, then one `--`),
/// `npm exec` / `npm x`, `pnpm exec` / `pnpm dlx`, `yarn dlx` / `yarn exec`
/// and `bun x`. `None` when `words` don't start with one.
fn runner_target(words: &[String]) -> Option<&[String]> {
    let at = |i: usize| words.get(i).map_or("", String::as_str);
    let skip = match (at(0), at(1)) {
        ("npx" | "bunx", _) => 1,
        ("npm", "exec" | "x") | ("pnpm", "exec" | "dlx") | ("yarn", "dlx" | "exec") => 2,
        ("bun", "x") => 2,
        _ => return None,
    };
    let mut rest = words.get(skip..).unwrap_or_default();
    while let Some(flag) = rest.first() {
        match flag.as_str() {
            "-y" | "--yes" | "--no-install" | "-q" | "--quiet" => rest = &rest[1..],
            "--" => {
                rest = &rest[1..];
                break;
            }
            _ => break,
        }
    }
    Some(rest)
}

fn tokenize_bounded_shell(command: &str) -> Option<Vec<ShellToken>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        Single,
        Double,
    }

    fn push_word(tokens: &mut Vec<ShellToken>, word: &mut String, started: &mut bool) {
        if *started {
            tokens.push(ShellToken::Word(std::mem::take(word)));
            *started = false;
        }
    }

    let mut chars = command.chars().peekable();
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut word_started = false;
    let mut quote: Option<Quote> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some(Quote::Single) => {
                if ch == '\'' {
                    quote = None;
                } else {
                    word.push(ch);
                }
                word_started = true;
            }
            Some(Quote::Double) => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' {
                    let next = chars.next()?;
                    word.push(next);
                } else if ch == '`' || (ch == '$' && chars.peek() == Some(&'(')) {
                    return None;
                } else {
                    word.push(ch);
                }
                word_started = true;
            }
            None => match ch {
                '\'' => {
                    quote = Some(Quote::Single);
                    word_started = true;
                }
                '"' => {
                    quote = Some(Quote::Double);
                    word_started = true;
                }
                '\\' => {
                    word.push(chars.next()?);
                    word_started = true;
                }
                ' ' | '\t' => push_word(&mut tokens, &mut word, &mut word_started),
                '\n' | '\r' | '|' | ';' | '<' | '>' | '(' | ')' | '`' => return None,
                '$' if chars.peek() == Some(&'(') => return None,
                '&' => {
                    push_word(&mut tokens, &mut word, &mut word_started);
                    if chars.next() != Some('&') {
                        return None;
                    }
                    tokens.push(ShellToken::AndIf);
                }
                // Reject comments rather than trying to emulate the shell's
                // context-sensitive comment grammar.
                '#' => return None,
                _ => {
                    word.push(ch);
                    word_started = true;
                }
            },
        }
    }
    if quote.is_some() {
        return None;
    }
    push_word(&mut tokens, &mut word, &mut word_started);
    (!tokens.is_empty()).then_some(tokens)
}

fn strip_cd_prefixes(tokens: &[ShellToken]) -> Option<Vec<String>> {
    let mut cursor = 0;
    loop {
        let Some(ShellToken::Word(command)) = tokens.get(cursor) else {
            return None;
        };
        if command != "cd" {
            break;
        }

        cursor += 1;
        if matches!(tokens.get(cursor), Some(ShellToken::Word(flag)) if flag == "--") {
            cursor += 1;
        }
        if !matches!(tokens.get(cursor), Some(ShellToken::Word(_))) {
            return None;
        }
        cursor += 1;
        if !matches!(tokens.get(cursor), Some(ShellToken::AndIf)) {
            return None;
        }
        cursor += 1;
    }

    let mut words = Vec::new();
    for token in &tokens[cursor..] {
        match token {
            ShellToken::Word(word) => words.push(word.clone()),
            ShellToken::AndIf => return None,
        }
    }
    (!words.is_empty()).then_some(words)
}

fn check_words(words: &[String]) -> Option<&'static str> {
    let first = words.first()?.as_str();
    let target = runner_target(words);
    if target.is_none() && matches!(first, "npm" | "pnpm" | "yarn" | "bun") {
        let script = if words.get(1).map(String::as_str) == Some("run") {
            words.get(2)?.as_str()
        } else {
            words.get(1)?.as_str()
        };
        return script_kind(script);
    }

    let direct: Vec<&str> = target.unwrap_or(words).iter().map(String::as_str).collect();
    match direct.as_slice() {
        ["vitest" | "jest" | "pytest", ..] => Some("test"),
        ["playwright", "test", ..] => Some("test"),
        ["node", rest @ ..] if node_runs_tests(rest) => Some("test"),
        ["python", "-m", "pytest", ..] => Some("test"),
        ["cargo" | "go" | "swift" | "dotnet", "test", ..] => Some("test"),
        ["tsc" | "pyright" | "mypy", ..] => Some("typecheck"),
        ["eslint" | "stylelint", ..] => Some("lint"),
        ["cargo", "clippy", ..] | ["ruff", "check", ..] => Some("lint"),
        ["cargo" | "go" | "swift" | "dotnet", "build", ..] => Some("build"),
        ["vite", "build", ..] | ["tauri", "build", ..] | ["make", "build", ..] => Some("build"),
        ["cargo", "check", ..] | ["make", "check", ..] => Some("check"),
        ["git", "diff", "--check", ..] => Some("check"),
        _ => None,
    }
}

/// Whether node's arguments run its test runner: `--test` among node's own
/// flags, before the script (`node x.mjs --test` passes it to the script).
fn node_runs_tests(args: &[&str]) -> bool {
    let mut flags = args.iter().take_while(|arg| arg.starts_with('-'));
    flags.any(|flag| *flag == "--test")
}

fn script_kind(script: &str) -> Option<&'static str> {
    [
        ("test", "test"),
        ("typecheck", "typecheck"),
        ("type-check", "typecheck"),
        ("lint", "lint"),
        ("build", "build"),
        ("check", "check"),
        // Hollow's own `npm run gate` (and its `gate:frontend` / `gate:rust`
        // halves): a check of its own kind, passed only by its
        // `[gate-record] Gate: … passed` line when its exit can't say.
        ("gate", "gate"),
    ]
    .into_iter()
    .find(|(prefix, _)| script == *prefix || script.starts_with(&format!("{prefix}:")))
    .map(|(_, kind)| kind)
}

fn is_non_executing_flag(word: &str) -> bool {
    matches!(
        word,
        "--help"
            | "-h"
            | "--version"
            | "-V"
            | "--list"
            | "--list-tests"
            | "--listTests"
            | "--collect-only"
            | "--no-run"
    )
}

/// How far a shell command's exit status speaks for one check in it
/// ([`shell_checks`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitSays {
    /// The script's last command, not after `||`, not backgrounded: the
    /// exit status is the check's (the strict rule's case).
    All,
    /// Only `&&` joins it to the end: exit 0 proves it passed, but a failure
    /// could be a later command's.
    Success,
    /// Something can mask it (a pipe, `;`, `||` after it, or it runs only
    /// after an `||`): only its output can say.
    Nothing,
    /// Started in the background (`&`): its output may not be in the result
    /// at all, so only a failure there counts.
    Background,
}

/// Whose summary lines judge a check's output ([`ShellCheck`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reader {
    /// A runner invoked directly, or Hollow's gate (its
    /// `runner-summaries.json` id, [`runner_of`]): only that runner's pass
    /// lines pass it (`cargo`'s `Finished` never passes a `tauri build`),
    /// and, but for the gate, which runs them all, only its failure lines
    /// fail it.
    Runner(&'static str),
    /// A package script other than the gate (`npm test`): any runner of its
    /// kind may be what ran.
    Script,
    /// A check no runner entry reads (`pytest`, `go test`, `make build`): its
    /// kind's failure lines fail it, and no text passes it.
    Unread,
}

/// One check a shell script runs ([`shell_checks`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellCheck {
    pub kind: &'static str,
    reader: Reader,
    exit: ExitSays,
    /// Whether its output may pass it: it is the script's only check of its
    /// kind (another one's summary would read as its own; a package script:
    /// its only check), and nothing that drops lines stands between it and
    /// the result ([`output_kept_whole`]).
    text_pass: bool,
    /// Its own words, past assignments and without redirections: what a
    /// receipt stores of the command, never the whole script.
    pub words: String,
}

impl ShellCheck {
    /// The check's outcome (`passed` | `failed` | `unknown`) from the call's
    /// result: `finished` (a result was recorded), its error flag, whether
    /// the whole call ran in the background (`run_in_background`), and what
    /// its output says ([`read_output`]). Failure text always wins; the exit
    /// status decides only where nothing can mask it; otherwise the text
    /// does, and `passed` needs the runner's own pass text. A call that
    /// errored (a Bash timeout, a later command's failure) never passes a
    /// check its exit status doesn't speak for, whatever its text says: a
    /// `cargo test | tee` cut off by the timeout after the first binary's
    /// `test result: ok.` is not a pass (OF16 re-review).
    pub(crate) fn outcome(
        &self,
        finished: bool,
        is_error: bool,
        backgrounded: bool,
        text: &'static str,
    ) -> &'static str {
        if !finished {
            return "unknown";
        }
        let exit = if backgrounded {
            ExitSays::Background
        } else {
            self.exit
        };
        match exit {
            _ if text == "failed" => "failed",
            ExitSays::All if is_error => "failed",
            ExitSays::All => "passed",
            ExitSays::Success if !is_error => "passed",
            ExitSays::Success | ExitSays::Nothing if is_error => "unknown",
            ExitSays::Success | ExitSays::Nothing => text,
            ExitSays::Background => "unknown",
        }
    }

    /// Whose failure lines fail it: its runner's id, or its kind (the
    /// gate, a package script, an unread check: every runner of the kind).
    /// Runner ids and kinds never share a name (a test holds that).
    pub(crate) fn fail_scope(&self) -> &'static str {
        match self.reader {
            Reader::Runner(id) if id != GATE_RUNNER => id,
            _ => self.kind,
        }
    }
}

/// The gate's runner entry: its `[gate-record] Gate: … passed` line.
const GATE_RUNNER: &str = "gate-record";

/// The runner a check's own words invoke directly (`runner-summaries.json`
/// ids), or the gate's. None for another package script and for a check no
/// runner entry reads.
fn runner_of(words: &[String]) -> Option<&'static str> {
    let direct: Vec<&str> = runner_target(words)
        .unwrap_or(words)
        .iter()
        .map(String::as_str)
        .collect();
    match direct.as_slice() {
        ["npm" | "pnpm" | "yarn" | "bun", "run", "gate", ..] => Some(GATE_RUNNER),
        ["vitest" | "jest", ..] => Some("vitest"),
        ["playwright", "test", ..] => Some("playwright"),
        ["node", ..] => Some("node-test"),
        ["cargo", "test", ..] => Some("cargo-test"),
        ["cargo", "build" | "check" | "clippy", ..] => Some("cargo-build"),
        ["tsc", ..] => Some("tsc"),
        ["eslint", ..] => Some("eslint"),
        ["vite", "build", ..] => Some("vite"),
        ["tauri", "build", ..] => Some("tauri"),
        _ => None,
    }
}

/// Whether a check's words run a package script (`npm test`).
fn is_package_script(words: &[String]) -> bool {
    let first = words.first().map_or("", String::as_str);
    matches!(first, "npm" | "pnpm" | "yarn" | "bun")
}

/// A simple command's program, without its folder.
fn program_of(words: &[String]) -> &str {
    let first = words.first().map_or("", String::as_str);
    first.rsplit('/').next().unwrap_or(first)
}

/// Where a simple command sends its stdout or its stderr ([`streams`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dest {
    /// Where it was going anyway: the result, or the pipe into the next
    /// command.
    Result,
    /// `/dev/null`, or closed (`2>&-`).
    Dropped,
    /// A file: its name as written, and whether it is appended to (`>>`).
    File(String, bool),
}

/// Where a simple command's raw words (redirections still in) send its
/// stdout and its stderr, one redirection after another as the shell does
/// (`>f 2>&1` sends both to `f`, `2>&1 >f` only stdout; `&>f` both). None
/// for a redirection it can't read (`cargo test>t.log` in one word, a
/// descriptor other than 1 or 2): whatever reads it then can't pass on
/// text.
fn streams(raw: &[String]) -> Option<(Dest, Dest)> {
    let mut out = Dest::Result;
    let mut err = Dest::Result;
    let mut words = raw.iter();
    while let Some(word) = words.next() {
        let digits = word.len() - word.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let (fd, rest) = word.split_at(digits);
        let (mut both, rest) = match rest.strip_prefix('&') {
            Some(r) if fd.is_empty() && r.starts_with('>') => (true, r),
            _ => (false, rest),
        };
        let Some(after) = rest.strip_prefix('>') else {
            if rest.contains('>') {
                return None;
            }
            continue;
        };
        let append = after.starts_with('>');
        let after = after.strip_prefix('>').unwrap_or(after);
        let after = after.strip_prefix('|').unwrap_or(after);
        let (copy, target) = match after.strip_prefix('&') {
            Some(target) => (true, target),
            None => (false, after),
        };
        let target = match target {
            "" => words.next()?.as_str(),
            target => target,
        };
        let file = || match target {
            "/dev/null" => Dest::Dropped,
            path => Dest::File(path.to_string(), append),
        };
        let dest = match (copy, target) {
            (true, "1") => out.clone(),
            (true, "2") => err.clone(),
            (true, "-") => Dest::Dropped,
            // `>&file`: both, like `&>file`.
            (true, _) if fd.is_empty() => {
                both = true;
                file()
            }
            (true, _) => return None,
            (false, _) => file(),
        };
        match fd {
            _ if both => {
                out = dest.clone();
                err = dest;
            }
            "" | "1" => out = dest,
            "2" => err = dest,
            _ => return None,
        }
    }
    Some((out, err))
}

/// The files a `tail` or `cat` reads (past its flags, and tail's `-n N` /
/// `-c N`); None for any other program.
fn tail_or_cat_files(words: &[String]) -> Option<Vec<&str>> {
    let program = program_of(words);
    if !matches!(program, "tail" | "cat") {
        return None;
    }
    let mut files = Vec::new();
    let mut args = words.iter().skip(1);
    while let Some(arg) = args.next() {
        if program == "tail" && matches!(arg.as_str(), "-n" | "-c") {
            args.next();
        } else if !(arg.starts_with('-') || arg.starts_with('+')) {
            files.push(arg.as_str());
        }
    }
    Some(files)
}

/// Whether segment `i`'s output can reach the result whole enough to pass
/// it (OF16 re-review: an allowlist, never a denylist of filters):
/// - its stderr reaches the result too (left alone or `2>&1`): never
///   `2>file` or `2>/dev/null`, which drop the failure lines a runner prints
///   there and keep the pass line on stdout;
/// - its stdout's pipeline goes on only through `tail` (a runner prints its
///   summary last), `tee` and `cat`: `head` keeps the first `test result:
///   ok.` and cuts a later `FAILED`; `grep Tests` keeps vitest's `Tests  10
///   passed` and drops `Test Files  1 failed`;
/// - or its stdout goes to a file with `>` (not `>>`, which keeps an older
///   run's text) and every later command is a `tail` or `cat` of that same
///   file (`cat f | tail -5` too): anything else (a `python3 -c` that prints
///   the first 40 lines) may have dropped the failure.
///
/// A failure line still fails it either way. Residual risk: `tail -N` can
/// cut an earlier failing cargo test binary when later ones pass, but cargo
/// stops at the first failing binary and prints `error: test failed` (or
/// `error: N targets failed` with `--no-fail-fast`) last, and that fails it.
fn output_kept_whole(
    i: usize,
    segments: &[crate::run_effects::Segment],
    raw: &[Vec<String>],
    words: &[Vec<String>],
) -> bool {
    use crate::run_effects::Joint;
    let Some((out, err)) = streams(&raw[i]) else {
        return false;
    };
    let err_kept = err == Dest::Result || (err == out && matches!(out, Dest::File(..)));
    if !err_kept {
        return false;
    }
    match out {
        Dest::Dropped => false,
        Dest::File(_, true) => false,
        Dest::File(path, false) => {
            i + 1 < segments.len()
                && (i + 1..segments.len()).all(|k| {
                    let piped = k > i + 1 && segments[k - 1].joint == Joint::Pipe;
                    match tail_or_cat_files(&words[k]) {
                        Some(files) => files == [path.as_str()] || (piped && files.is_empty()),
                        None => false,
                    }
                })
        }
        // A redirect the splitter saw and [`streams`] didn't: unreadable.
        Dest::Result if segments[i].redirect => false,
        Dest::Result => {
            let mut j = i;
            while segments[j].joint == Joint::Pipe && j + 1 < segments.len() {
                j += 1;
                if !matches!(program_of(&words[j]), "tail" | "tee" | "cat") {
                    return false;
                }
            }
            true
        }
    }
}

/// Programs that, run before a check, can end the script with their own
/// status (`test -f .skip && exit 0`) or change what its exit means: the
/// script's exit status then needn't be the check's.
const EXIT_TAKERS: &[&str] = &["exit", "return", "exec", "trap"];

/// Shell keywords `run_effects::split_joined` leaves on the front of a
/// simple command (`then exit 1`, `{ set -e`, `if ! npx vitest run`):
/// [`changes_script_exit`] reads the command past them.
const LEADING_KEYWORDS: &[&str] = &[
    "then", "else", "elif", "do", "{", "!", "if", "while", "until", "function",
];

/// Programs that run other shell text in the script's own shell, which can
/// `exit` or `set -e` there (`source env.sh`, `. ./env.sh`, `eval "$x"`).
const SCRIPT_RUNNERS: &[&str] = &["source", ".", "eval"];

/// Whether one simple command of a script (its text, as
/// `run_effects::split_joined` gives it) can make the script's exit status
/// another command's than its last one's (OF18 review,
/// `agent::codex_command_result`): an [`EXIT_TAKERS`] or [`SCRIPT_RUNNERS`]
/// program, read past [`LEADING_KEYWORDS`], assignments, `env` and `time`,
/// or a `set`, `setopt` or `shopt` that turns on errexit or pipefail (`set
/// -e`, `set -euo pipefail`, `set -o errexit`, `setopt pipefail`, `setopt
/// ERR_EXIT`). It reads wide on purpose, so a doubtful script keeps its exit
/// an error (OF18 re-review): an exit-taker, `source`, `eval` or such a
/// `set` *anywhere* in the command's words counts, not only as its program
/// (`function f { exit 1`, `grep exit x` too); any `set` flag word with an
/// `e` in it counts (`set -ex`, even `set +e`), and so does any word naming
/// errexit, errreturn or pipefail in a spelling zsh accepts (`err_exit`,
/// `NO_PIPE_FAIL`). Only `.` must be the program (`grep -rn foo .` is a
/// folder).
pub(crate) fn changes_script_exit(segment: &str) -> bool {
    let raw = crate::run_effects::shell_words(segment);
    let start = raw
        .iter()
        .position(|w| !LEADING_KEYWORDS.contains(&w.as_str()))
        .unwrap_or(raw.len());
    let words = command_words(raw[start..].to_vec());
    let program = program_of(&words);
    if EXIT_TAKERS.contains(&program) || SCRIPT_RUNNERS.contains(&program) {
        return true;
    }
    words
        .iter()
        .enumerate()
        .any(|(i, word)| match word.as_str() {
            "set" | "setopt" | "shopt" => turns_on_errexit(&words[i + 1..]),
            "source" | "eval" => true,
            w => EXIT_TAKERS.contains(&w),
        })
}

/// Whether a `set`, `setopt` or `shopt`'s arguments may turn on errexit or
/// pipefail ([`changes_script_exit`], read wide).
fn turns_on_errexit(args: &[String]) -> bool {
    args.iter().any(|word| {
        let flags = word.strip_prefix('-').or_else(|| word.strip_prefix('+'));
        let errexit_flag = flags.is_some_and(|f| !f.starts_with('-') && f.contains('e'));
        let name = word.replace('_', "").to_ascii_lowercase();
        errexit_flag
            || ["errexit", "errreturn", "pipefail"]
                .iter()
                .any(|o| name.contains(o))
    })
}

/// The check kinds a shell script runs, in order (OF16): [`shell_checks`]'
/// kinds. Unlike [`check_kind`] it reads every simple command of a real
/// script (pipes, redirects, `;`, newlines, `||`).
#[cfg(test)]
pub(crate) fn checks_in(command: &str) -> Vec<&'static str> {
    shell_checks(command).into_iter().map(|c| c.kind).collect()
}

/// Every check a shell script runs, each with how far the exit status
/// speaks for it. Takes a `<shell> -lc '…'` wrapper off, drops comments
/// and here-document bodies (data, not commands), splits the script into
/// simple commands (`run_effects::split_joined`), and reads each one past
/// its setup words ([`strip_wrappers`]: assignments, `env`, `time`,
/// `nohup`, `nice`, `timeout`), without its redirections (a
/// `cd` is a command of its own and never a check). A script the splitter
/// can't read (command substitution, a subshell, an unclosed quote or
/// here-document) holds none.
pub(crate) fn shell_checks(command: &str) -> Vec<ShellCheck> {
    use crate::run_effects::Joint;
    // A shell wrapper past setup words (`env CI=1 bash -c '…'`, overnight 9
    // Spare 5) runs its script like a bare `bash -c '…'`.
    let words = crate::run_effects::shell_words(command.trim());
    let script = match strip_wrappers(&words) {
        [shell, flag, script]
            if words.len() > 3
                && matches!(
                    shell.as_str(),
                    "bash" | "sh" | "zsh" | "/bin/bash" | "/bin/sh" | "/bin/zsh"
                )
                && matches!(flag.as_str(), "-lc" | "-c") =>
        {
            script.clone()
        }
        _ => crate::run_effects::unwrap_shell_wrapper(command.trim()),
    };
    let Some(script) = strip_heredocs(&script) else {
        return Vec::new();
    };
    let Some(segments) = crate::run_effects::split_joined(&script, true) else {
        return Vec::new();
    };
    let raw: Vec<Vec<String>> = segments
        .iter()
        .map(|s| crate::run_effects::shell_words(&s.text))
        .collect();
    let words: Vec<Vec<String>> = raw.iter().map(|w| command_words(w.clone())).collect();
    let last = segments.len().saturating_sub(1);
    let mut out: Vec<ShellCheck> = Vec::new();
    for (i, own) in words.iter().enumerate() {
        if own.iter().any(|word| is_non_executing_flag(word)) {
            continue;
        }
        let Some(kind) = check_words(own) else {
            continue;
        };
        // A `&` at the end of its list (`npm test && x &`) backgrounds all of it.
        let list_end = segments[i..]
            .iter()
            .find(|s| matches!(s.joint, Joint::Then | Joint::End | Joint::Background))
            .map(|s| s.joint);
        // It may not run at all after an `||`: before its own pipeline too
        // (`a || b | cargo test`).
        let mut start = i;
        while start > 0 && segments[start - 1].joint == Joint::Pipe {
            start -= 1;
        }
        let after_or = start > 0 && segments[start - 1].joint == Joint::Or;
        // An `exit`, `return`, `exec` or `trap` before it can end the script
        // with a status of its own (`test -f .skip && exit 0⏎cargo test`).
        let exit_taken = words[..i]
            .iter()
            .any(|w| EXIT_TAKERS.contains(&program_of(w)));
        let exit = if list_end == Some(Joint::Background) {
            ExitSays::Background
        } else if after_or || exit_taken {
            ExitSays::Nothing
        } else if i == last {
            ExitSays::All
        } else if segments[i..last].iter().all(|s| s.joint == Joint::And) {
            ExitSays::Success
        } else {
            ExitSays::Nothing
        };
        let reader = match runner_of(own) {
            Some(id) => Reader::Runner(id),
            // A package runner's command (`pnpm exec pytest`) is no script:
            // read like the bare command.
            None if runner_target(own).is_none() && is_package_script(own) => Reader::Script,
            None => Reader::Unread,
        };
        out.push(ShellCheck {
            kind,
            reader,
            exit,
            text_pass: reader != Reader::Unread && output_kept_whole(i, &segments, &raw, &words),
            words: own.join(" "),
        });
    }
    // Two checks of one kind: either one's summary would read as the other's.
    // A package script may be any runner, so with any other check in the
    // command (`pnpm build | tail -5; cargo check | tail -2`) the other's
    // pass line (cargo's `Finished`) would read as its own.
    let kinds: Vec<&'static str> = out.iter().map(|c| c.kind).collect();
    let several = out.len() > 1;
    for check in &mut out {
        let same_kind = kinds.iter().filter(|k| **k == check.kind).count() > 1;
        if same_kind || (several && check.reader == Reader::Script) {
            check.text_pass = false;
        }
    }
    out
}

/// A simple command's words past its setup words ([`strip_wrappers`]),
/// without redirections (`2>&1`, `> file`, `>$TMPDIR/x.log`).
fn command_words(words: Vec<String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut skip_target = false;
    for word in words {
        if std::mem::take(&mut skip_target) {
            continue;
        }
        let operator = word.trim_start_matches(|c: char| c.is_ascii_digit());
        let operator = operator.strip_prefix('&').unwrap_or(operator);
        if let Some(target) = operator
            .strip_prefix(">>")
            .or_else(|| operator.strip_prefix('>'))
            .or_else(|| operator.strip_prefix('<'))
        {
            // `>` alone: the file is the next word.
            skip_target = target.is_empty();
            continue;
        }
        kept.push(word);
    }
    strip_wrappers(&kept).to_vec()
}

/// `NAME=value`: an environment assignment before a command.
pub(crate) fn is_assignment(word: &str) -> bool {
    let name = word.split_once('=').map_or("", |(name, _)| name);
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The script without here-document bodies (`cat > f <<'EOF'` … `EOF`):
/// their lines are a file's text, never commands, and an apostrophe in one
/// would make the whole script unreadable. None when one is still open at
/// the end: what it was meant to hold can't be told from what follows.
fn strip_heredocs(script: &str) -> Option<String> {
    let mut out = String::new();
    let mut until: Option<String> = None;
    for line in script.lines() {
        if let Some(delimiter) = &until {
            if line.trim() == delimiter.as_str() {
                until = None;
            }
            continue;
        }
        until = heredoc_delimiter(line);
        out.push_str(line);
        out.push('\n');
    }
    until.is_none().then_some(out)
}

/// The delimiter of a here-document a line opens (`<<EOF`, `<<-'EOF'`,
/// `<< "EOF"`), not a here-string (`<<<`).
fn heredoc_delimiter(line: &str) -> Option<String> {
    let at = line.find("<<")?;
    let rest = &line[at + 2..];
    if rest.starts_with('<') {
        return None;
    }
    let rest = rest.strip_prefix('-').unwrap_or(rest).trim_start();
    let word: String = rest
        .chars()
        .take_while(|&c| !c.is_whitespace() && !matches!(c, ';' | '&' | '|' | ')' | '>'))
        .collect();
    let word = word.trim_start_matches('\\').trim_matches(['\'', '"']);
    (!word.is_empty()).then(|| word.to_string())
}

/// The runner summaries shared with the scorecard
/// (`scripts/agent-eval/runner-summaries.json`; its `about` says how to
/// read it).
const RUNNER_SUMMARIES: &str = include_str!("../../scripts/agent-eval/runner-summaries.json");

#[derive(Deserialize)]
struct RunnerSummaries {
    runners: Vec<RunnerSpec>,
    /// What cut or timed-out output holds: no text pass then.
    #[serde(default)]
    truncated: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerSpec {
    id: String,
    kinds: Vec<String>,
    #[serde(default)]
    pass_kinds: Option<Vec<String>>,
    #[serde(default)]
    fail: Vec<String>,
    #[serde(default)]
    pass: Vec<String>,
    #[serde(default)]
    not_after_pass: Vec<String>,
}

struct Runner {
    id: String,
    kinds: Vec<String>,
    pass_kinds: Vec<String>,
    fail: Vec<regex::Regex>,
    pass: Vec<regex::Regex>,
    /// Lines that, after the last pass line, mean the run went on past it
    /// (cargo's next test binary): the pass line then doesn't count.
    not_after_pass: Vec<regex::Regex>,
}

impl Runner {
    /// Whether prepared `text` holds this runner's pass text, with nothing
    /// of [`Runner::not_after_pass`] after the last pass line.
    fn passes(&self, text: &str) -> bool {
        let last_end = self
            .pass
            .iter()
            .filter_map(|re| re.find_iter(text).last().map(|m| m.end()))
            .max();
        match last_end {
            Some(end) => !hits(&self.not_after_pass, &text[end..]),
            None => false,
        }
    }
}

/// Runner `id`'s patterns compiled; the error names the runner and the
/// pattern that didn't.
fn compile_patterns(id: &str, patterns: &[String]) -> Result<Vec<regex::Regex>, String> {
    let mut out = Vec::new();
    for pattern in patterns {
        match regex::Regex::new(pattern) {
            Ok(re) => out.push(re),
            Err(e) => return Err(format!("{id} `{pattern}`: {e}")),
        }
    }
    Ok(out)
}

/// [`RUNNER_SUMMARIES`] parsed, each pattern compiled.
fn parse_runners(json: &str) -> Result<Vec<Runner>, String> {
    let summaries: RunnerSummaries = match serde_json::from_str(json) {
        Ok(summaries) => summaries,
        Err(e) => return Err(format!("runner summaries: {e}")),
    };
    let mut out = Vec::new();
    for spec in summaries.runners {
        let pass_kinds = spec.pass_kinds.unwrap_or_else(|| spec.kinds.clone());
        out.push(Runner {
            fail: compile_patterns(&spec.id, &spec.fail)?,
            pass: compile_patterns(&spec.id, &spec.pass)?,
            not_after_pass: compile_patterns(&spec.id, &spec.not_after_pass)?,
            pass_kinds,
            kinds: spec.kinds,
            id: spec.id,
        });
    }
    Ok(out)
}

/// [`RUNNER_SUMMARIES`]' `truncated` patterns compiled.
fn parse_truncated(json: &str) -> Result<Vec<regex::Regex>, String> {
    match serde_json::from_str::<RunnerSummaries>(json) {
        Ok(summaries) => compile_patterns("truncated", &summaries.truncated),
        Err(e) => Err(format!("runner summaries: {e}")),
    }
}

/// A test in this module proves the embedded file parses; a bad one here
/// only leaves every text outcome `unknown`.
static RUNNERS: Lazy<Vec<Runner>> =
    Lazy::new(|| parse_runners(RUNNER_SUMMARIES).unwrap_or_default());

/// What cut or timed-out output holds (`truncated`: Claude Code's `Command
/// timed out` and `[N lines truncated]`, Hollow's own `… (truncated)`).
/// Unlike [`RUNNERS`], a file that doesn't parse leaves one pattern that
/// matches everything, so no text passes rather than every text.
static TRUNCATED: Lazy<Vec<regex::Regex>> = Lazy::new(|| {
    parse_truncated(RUNNER_SUMMARIES)
        .unwrap_or_else(|_| vec![regex::Regex::new("").expect("empty pattern")])
});

/// Whether prepared `text` was cut or timed out ([`TRUNCATED`]).
fn truncated(text: &str) -> bool {
    hits(&TRUNCATED, text)
}

/// The text runner patterns read: ANSI colour codes stripped and a newline
/// added at both ends, so a pattern can take a summary as a whole line
/// without anchors (`runner-summaries.json`'s `about` tells a JS reader the
/// same).
fn prepared(text: &str) -> String {
    static ANSI: Lazy<regex::Regex> =
        Lazy::new(|| regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]").unwrap());
    format!("\n{}\n", ANSI.replace_all(text, ""))
}

fn hits(patterns: &[regex::Regex], text: &str) -> bool {
    patterns.iter().any(|re| re.is_match(text))
}

/// Whether prepared `text` holds the failure lines of `scope`
/// ([`ShellCheck::fail_scope`]): a runner id's own, or, for a kind, those
/// of every runner of that kind.
fn scope_fails(scope: &str, text: &str) -> bool {
    if let Some(runner) = RUNNERS.iter().find(|r| r.id == scope) {
        return hits(&runner.fail, text);
    }
    for runner in RUNNERS.iter() {
        if runner.kinds.iter().any(|k| k == scope) && hits(&runner.fail, text) {
            return true;
        }
    }
    false
}

/// Whether prepared `text` holds the pass lines of a runner whose pass
/// kinds hold `kind` (a package script's check: any runner may have run).
fn kind_passes(kind: &str, text: &str) -> bool {
    for runner in RUNNERS.iter() {
        if runner.pass_kinds.iter().any(|k| k == kind) && runner.passes(text) {
            return true;
        }
    }
    false
}

/// What a check's output says (OF16), from prepared `text`: `failed` on
/// its failure lines ([`ShellCheck::fail_scope`]), else `passed` on its own
/// runner's pass lines (any runner of its kind for a package script) when
/// its output may pass it at all and wasn't cut or timed out
/// ([`truncated`]), else `unknown`. Failure text always wins, and nothing
/// but positive pass text gives `passed`.
fn read_output(check: &ShellCheck, text: &str) -> &'static str {
    if scope_fails(check.fail_scope(), text) {
        return "failed";
    }
    if !check.text_pass || truncated(text) {
        return "unknown";
    }
    let passed = match check.reader {
        Reader::Runner(id) => RUNNERS.iter().any(|r| r.id == id && r.passes(text)),
        Reader::Script => kind_passes(check.kind, text),
        Reader::Unread => false,
    };
    if passed {
        "passed"
    } else {
        "unknown"
    }
}

/// What `output` says of each check `command` holds, in
/// [`shell_checks`]' order.
fn text_outcomes(command: &str, output: &str) -> Vec<&'static str> {
    let checks = shell_checks(command);
    if checks.is_empty() {
        return Vec::new();
    }
    let text = prepared(output);
    checks.iter().map(|c| read_output(c, &text)).collect()
}

/// The fail scopes of `command`'s checks ([`ShellCheck::fail_scope`]), once
/// each. The step spine keeps them on a shell step's checkpoint
/// (`checkScopes`), not the command.
pub(crate) fn check_scopes(command: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for check in shell_checks(command) {
        let scope = check.fail_scope();
        if !out.contains(&scope) {
            out.push(scope);
        }
    }
    out
}

/// Whether `output` holds the failure lines of fail scope `scope`.
pub(crate) fn failed_by_text(scope: &str, output: &str) -> bool {
    scope_fails(scope, &prepared(output))
}

/// The receipt's outcome of the checks a finished shell call holds, from
/// `output` and its error flag (`failed` when any failed, `passed` when
/// every one passed, else `unknown`; None for a command that holds no
/// check). The task-state record reads a turn recorded before the step
/// spine kept `checkOutcome` with it, on the transcript's stored result.
pub(crate) fn outcome_of_output(
    command: &str,
    output: &str,
    is_error: bool,
) -> Option<&'static str> {
    let checks = shell_checks(command);
    if checks.is_empty() {
        return None;
    }
    let text = prepared(output);
    let outcomes = checks
        .iter()
        .map(|c| c.outcome(true, is_error, false, read_output(c, &text)));
    Some(combined_outcome(outcomes.collect::<Vec<_>>()))
}

/// Store a receipt as a finished run would (tests elsewhere in the crate).
#[cfg(test)]
pub(crate) fn persist_for_test(vault: &Path, receipt: EvidenceReceipt) -> Result<(), String> {
    persist(vault, receipt)
}

/// Write `receipts` as the whole store at once (the newest [`KEEP`]), for
/// fixtures too large to persist one by one.
#[cfg(test)]
pub(crate) fn replace_all_for_test(
    vault: &Path,
    mut receipts: Vec<EvidenceReceipt>,
) -> Result<(), String> {
    receipts.sort_by(|a, b| a.finished_at.cmp(&b.finished_at));
    let drop = receipts.len().saturating_sub(KEEP);
    receipts.drain(0..drop);
    let store = ReceiptFileStore {
        schema_version: SCHEMA_VERSION,
        receipts,
    };
    let body = serde_json::to_vec_pretty(&store).map_err(|e| e.to_string())?;
    fs::write(store_path(vault), body).map_err(|e| e.to_string())
}

#[cfg(test)]
fn persist(vault: &Path, receipt: EvidenceReceipt) -> Result<(), String> {
    let mut receipt = receipt;
    persist_with(vault, &mut receipt, None)
}

/// Store `receipt` and, when given, its context manifest (the encoded
/// sidecar and the receipt's reference to it), under [`FILE_LOCK`]: the
/// sidecar first (a failure leaves the receipt without a reference), then
/// the store, then every sidecar whose receipt the store no longer keeps is
/// removed, so a manifest lives exactly as long as its receipt.
fn persist_with(
    vault: &Path,
    receipt: &mut EvidenceReceipt,
    manifest: Option<(Vec<u8>, crate::run_manifest::ManifestRef)>,
) -> Result<(), String> {
    let _guard = FILE_LOCK
        .lock()
        .map_err(|_| "evidence receipt lock poisoned".to_string())?;
    let mut store = read_store(vault)?;
    receipt.context_manifest = None;
    if let Some((bytes, reference)) = manifest {
        match crate::run_manifest::store(vault, &receipt.run_id, &bytes) {
            Ok(()) => receipt.context_manifest = Some(reference),
            Err(error) => eprintln!("[evidence] run {}: {error}", receipt.run_id),
        }
    }
    store.receipts.retain(|existing| existing.id != receipt.id);
    store.receipts.push(receipt.clone());
    store
        .receipts
        .sort_by(|a, b| a.finished_at.cmp(&b.finished_at));
    if store.receipts.len() > KEEP {
        let drop = store.receipts.len() - KEEP;
        store.receipts.drain(0..drop);
    }
    let body =
        serde_json::to_vec_pretty(&store).map_err(|e| format!("encode evidence receipts: {e}"))?;
    // Attributed host write (Paperclip PR 9b): receipts are persisted after
    // their run's `finish_run`, so another run may already be tracked; only
    // a write that bypasses this (an agent's) is flagged on it.
    crate::run_guard::write_host_state(vault, FILE_NAME, &body)
        .map_err(|e| format!("write evidence receipts: {e}"))?;
    let keep: HashSet<&str> = store
        .receipts
        .iter()
        .filter(|r| r.context_manifest.is_some())
        .map(|r| r.run_id.as_str())
        .collect();
    crate::run_manifest::prune(vault, &keep);
    Ok(())
}

/// A run's context manifest (agent effectiveness F11), for
/// `run_context_manifest`: with `run_id`, that run's and nothing else (no
/// fallback); else with `session_key`, the newest run of that conversation
/// that has one, and nothing else (never another conversation's); with
/// neither, the newest run that has one. `Ok(None)` when there is
/// none (a run id held by two receipts is
/// treated as none, as [`find_by_history_run`] does). The sidecar must
/// match the receipt's hash.
pub fn context_manifest(
    vault: &Path,
    run_id: Option<&str>,
    session_key: Option<&str>,
) -> Result<Option<crate::run_manifest::RunContextView>, String> {
    let run_id = run_id.map(str::trim).filter(|id| !id.is_empty());
    let session_key = session_key.map(str::trim).filter(|key| !key.is_empty());
    let receipt = match run_id {
        Some(run_id) => find_by_history_run(vault, run_id)?,
        // The newest request: a chat turn's or a workflow's (a heartbeat's
        // manifest, since O3, is read by its run id).
        None => list(vault, None, None, KEEP)?.into_iter().find(|r| {
            r.context_manifest.is_some()
                && matches!(r.source.as_str(), "chat" | "workflow")
                && session_key.is_none_or(|key| r.session_key.as_deref() == Some(key))
        }),
    };
    let Some(receipt) = receipt else {
        return Ok(None);
    };
    let Some(reference) = receipt.context_manifest.as_ref() else {
        return Ok(None);
    };
    let manifest = crate::run_manifest::load(vault, &receipt.run_id, reference)?;
    // Checks for this run that ran after its receipt (F11, O3), not those
    // already on the manifest.
    let after_run: Vec<crate::jev_decide::Report> = crate::jev_decide::for_run(&receipt.run_id)
        .into_iter()
        .filter(|r| !manifest.dedup.contains(r))
        .collect();
    let jev_calls = crate::run_manifest::JevCalls::of(&manifest, &after_run);
    let delegations = crate::delegation::for_run(vault, &receipt.run_id);
    // Read now, so a child task that ended after the receipt shows how it
    // ended and what it cost; the stop each child got is the receipt's.
    let cost_rollup = crate::descendants::with_stops(
        crate::descendants::rollup(vault, &receipt.run_id),
        receipt.descendants.as_ref(),
    );
    Ok(Some(crate::run_manifest::RunContextView {
        run_id: receipt.run_id.clone(),
        source: receipt.source.clone(),
        session_key: receipt.session_key.clone(),
        finished_at: receipt.finished_at.clone(),
        manifest,
        after_run,
        jev_calls,
        delegations,
        cost_rollup,
    }))
}

/// The connector decision on conversation `session_key`'s latest chat
/// manifest, which a turn that skips selection keeps (overnight OB18,
/// OB25); `None` when there is none or it can't be read.
pub(crate) fn last_connector_selection(
    vault: &Path,
    session_key: &str,
) -> Option<crate::context_rank::ConnectorSelection> {
    let receipt = list(vault, Some("chat"), None, KEEP)
        .ok()?
        .into_iter()
        .find(|r| r.context_manifest.is_some() && r.session_key.as_deref() == Some(session_key))?;
    let reference = receipt.context_manifest.as_ref()?;
    crate::run_manifest::load(vault, &receipt.run_id, reference)
        .ok()?
        .connectors
}

fn store_path(vault: &Path) -> PathBuf {
    vault.join(FILE_NAME)
}

/// The parsed store from [`STORE_CACHE`], parsing again when the file's
/// length, mtime or inode changed. Call under `FILE_LOCK`.
fn shared_store(vault: &Path) -> Result<std::sync::Arc<ReceiptFileStore>, String> {
    use std::os::unix::fs::MetadataExt;
    let path = store_path(vault);
    let meta = match fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(std::sync::Arc::new(ReceiptFileStore::default()));
        }
        Err(e) => return Err(format!("read evidence receipts: {e}")),
    };
    let stamp: StoreStamp = (
        meta.len(),
        meta.modified().unwrap_or(std::time::UNIX_EPOCH),
        meta.ino(),
    );
    let mut cache = STORE_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((seen, _, store)) = cache.1.get(&path) {
        if *seen == stamp {
            return Ok(store.clone());
        }
    }
    let store = std::sync::Arc::new(read_store(vault)?);
    if cache.1.len() >= STORE_CACHE_VAULTS && !cache.1.contains_key(&path) {
        let oldest = cache
            .1
            .iter()
            .min_by_key(|(_, (_, n, _))| *n)
            .map(|(p, _)| p.clone());
        if let Some(oldest) = oldest {
            cache.1.remove(&oldest);
        }
    }
    cache.0 += 1;
    let n = cache.0;
    cache.1.insert(path, (stamp, n, store.clone()));
    Ok(store)
}

/// How many times this process parsed `vault`'s store into the cache, for the
/// cache tests (parallel tests can't share a global count).
#[cfg(test)]
pub(crate) fn cached_parse_number(vault: &Path) -> Option<u64> {
    let cache = STORE_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    cache.1.get(&store_path(vault)).map(|(_, n, _)| *n)
}

fn read_store(vault: &Path) -> Result<ReceiptFileStore, String> {
    let path = store_path(vault);
    if !path.exists() {
        return Ok(ReceiptFileStore::default());
    }
    let raw = fs::read(&path).map_err(|e| format!("read evidence receipts: {e}"))?;
    let store: ReceiptFileStore =
        serde_json::from_slice(&raw).map_err(|e| format!("parse evidence receipts: {e}"))?;
    if store.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported evidence receipt schema {}",
            store.schema_version
        ));
    }
    Ok(store)
}

fn receipt_id(source: &str, run_id: &str, finished_at: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(source.as_bytes());
    hash.update([0]);
    hash.update(run_id.as_bytes());
    hash.update([0]);
    hash.update(finished_at.as_bytes());
    format!("receipt-{:x}", hash.finalize())[..28].to_string()
}

fn cap(value: &str, max: usize) -> String {
    let normalized = value.replace('\0', "�");
    if normalized.chars().count() <= max {
        normalized
    } else {
        format!("{}…", normalized.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn changes() -> Vec<ObservedChange> {
        vec![ObservedChange {
            path: "notes/a.md".into(),
            change_type: "modify".into(),
            ..Default::default()
        }]
    }

    /// A vault with workspace folder `workspaces/w1`, for F1's shared runs.
    fn shared_vault() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("workspaces/w1")).unwrap();
        dir
    }

    /// A chat collector for run `lease` working in `workspaces/w1`.
    fn turn_in_w1(v: &Path, lease: &crate::automated_history::RunLease) -> EvidenceCollector {
        let mut c = EvidenceCollector::new("chat", None, lease.id());
        c.context_mut().set_run_cwd(&v.join("workspaces/w1"));
        c
    }

    /// What a chat turn's finish does (`chat::finish_observed_run`): hand
    /// over its own writes, finish History, and write the receipt from it.
    fn finish_turn(
        v: &Path,
        lease: crate::automated_history::RunLease,
        c: EvidenceCollector,
    ) -> EvidenceReceipt {
        crate::automated_history::note_own_writes(v, lease.id(), c.own_written_paths(v));
        let run = lease.finish("success").unwrap();
        let changes = run
            .changes
            .into_iter()
            .map(ObservedChange::from_history)
            .collect();
        c.finish(v, "success", changes).unwrap()
    }

    fn file<'a>(r: &'a EvidenceReceipt, path: &str) -> &'a ReceiptFile {
        r.files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} not in {:?}", r.files))
    }

    /// Chat concurrency F1 (2026-10-20, red first): two turns in one
    /// workspace folder each write their own file with a recorded structured
    /// write (Claude's absolute `Write`, Codex's `file_change` relative to
    /// the folder) while both are live. Each receipt confirms only its own
    /// file; the other's is `shared`, names the other run and isn't
    /// credited.
    #[test]
    fn two_shared_turns_each_credit_only_their_own_file() {
        let dir = shared_vault();
        let v = dir.path();
        let w1 = "workspaces/w1";
        let one = crate::automated_history::try_begin_scoped_shared(v, w1, "chat", None).unwrap();
        let two = crate::automated_history::try_begin_scoped_shared(v, w1, "chat", None).unwrap();
        let (one_id, two_id) = (one.id().to_string(), two.id().to_string());
        let mut c1 = turn_in_w1(v, &one);
        let target = v.join("workspaces/w1/one.md");
        c1.record_tool_use("w", "Write", &json!({ "file_path": target }));
        fs::write(&target, "one's").unwrap();
        c1.record_tool_result("w", "ok", false);
        let mut c2 = turn_in_w1(v, &two);
        let item = json!({
            "id": "item_1",
            "type": "file_change",
            "changes": [{ "path": "two.md", "kind": "add" }],
            "status": "completed",
        });
        c2.record_tool_use("item_1", "file_change", &item);
        fs::write(v.join("workspaces/w1/two.md"), "two's").unwrap();
        c2.record_tool_result("item_1", "", false);

        let r1 = finish_turn(v, one, c1);
        let r2 = finish_turn(v, two, c2);
        for (r, mine, theirs, peer) in [
            (&r1, "workspaces/w1/one.md", "workspaces/w1/two.md", &two_id),
            (&r2, "workspaces/w1/two.md", "workspaces/w1/one.md", &one_id),
        ] {
            assert_eq!(file(r, mine).attribution, "confirmed", "{mine}");
            assert!(file(r, mine).is_own());
            assert_eq!(file(r, theirs).attribution, "shared", "{theirs}");
            assert!(!file(r, theirs).is_own());
            assert_eq!(file(r, theirs).also_changed_by, vec![peer.clone()]);
        }
        // Each run's own file is confirmed, so its order is known.
        assert_ne!(r1.verification_state, "unknownOrder");
    }

    /// F1 (red first): a read-only reviewer beside a writer is credited
    /// with nothing: the writer's file is on its receipt only as shared,
    /// and the receipt says it changed nothing.
    #[test]
    fn a_read_only_reviewer_beside_a_writer_is_credited_with_nothing() {
        let dir = shared_vault();
        let v = dir.path();
        let w1 = "workspaces/w1";
        let writer =
            crate::automated_history::try_begin_scoped_shared(v, w1, "chat", None).unwrap();
        let reviewer =
            crate::automated_history::try_begin_scoped_shared(v, w1, "chat", None).unwrap();
        let mut cw = turn_in_w1(v, &writer);
        let report = v.join("workspaces/w1/report.md");
        cw.record_tool_use("w", "Write", &json!({ "file_path": report }));
        fs::write(&report, "the writer's").unwrap();
        cw.record_tool_result("w", "ok", false);
        let mut cr = turn_in_w1(v, &reviewer);
        cr.record_tool_use("r", "Read", &json!({ "file_path": report }));
        cr.record_tool_result("r", "the writer's", false);
        cr.record_tool_use("b", "Bash", &json!({ "command": "git diff --stat" }));
        cr.record_tool_result("b", "1 file changed", false);

        let reviewed = finish_turn(v, reviewer, cr);
        assert!(!reviewed.files.is_empty(), "History saw the folder change");
        assert!(reviewed.files.iter().all(|f| f.attribution == "shared"));
        assert_eq!(reviewed.verification_state, "noChanges");
        let written = finish_turn(v, writer, cw);
        assert_eq!(
            file(&written, "workspaces/w1/report.md").attribution,
            "confirmed"
        );
    }

    /// F1: the paths a run's own writes named, vault-relative, however the
    /// tool call spelled them: relative to the run's folder, through a
    /// symlink to the vault (`~/Hollow/Vault`), a deleted file, apply_patch
    /// and Codex's file_change. A failed write and one outside the vault
    /// are left out.
    #[test]
    fn own_written_paths_resolve_every_spelling_of_a_vault_file() {
        let dir = shared_vault();
        let v = dir.path();
        let outside = tempdir().unwrap();
        let link = outside.path().join("Vault");
        std::os::unix::fs::symlink(v, &link).unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-f1-paths");
        c.context_mut().set_run_cwd(&link.join("workspaces/w1"));
        let calls = [
            (
                "Edit",
                json!({ "file_path": link.join("workspaces/w1/linked.md") }),
                false,
            ),
            (
                "Write",
                json!({ "file_path": link.join("workspaces/w1/gone/new.md") }),
                false,
            ),
            (
                "apply_patch",
                json!({ "patch": "*** Update File: ./patched.md\n" }),
                false,
            ),
            (
                "file_change",
                json!({ "changes": [{ "path": v.join("workspaces/w1/codex.md") }] }),
                false,
            ),
            (
                "Write",
                json!({ "file_path": v.join("workspaces/w1/failed.md") }),
                true,
            ),
            (
                "Write",
                json!({ "file_path": outside.path().join("elsewhere.md") }),
                false,
            ),
        ];
        for (i, (name, input, failed)) in calls.iter().enumerate() {
            let id = format!("c{i}");
            c.record_tool_use(&id, name, input);
            c.record_tool_result(&id, "ok", *failed);
        }
        let failed_change = json!({ "changes": [{ "path": "nope.md" }], "status": "failed" });
        c.record_tool_use("f", "file_change", &failed_change);
        c.record_tool_result("f", "", false);
        assert_eq!(
            c.own_written_paths(v).into_iter().collect::<Vec<_>>(),
            [
                "workspaces/w1/codex.md",
                "workspaces/w1/gone/new.md",
                "workspaces/w1/linked.md",
                "workspaces/w1/patched.md",
            ]
        );
    }

    /// A shared change the run's History marks unattributed.
    fn shared_change(path: &str) -> ObservedChange {
        ObservedChange {
            path: path.into(),
            change_type: "modify".into(),
            shared: true,
            ..Default::default()
        }
    }

    /// F1 review (blocking, red first): from the folder `workspaces/w1`, a
    /// write to the relative `workspaces/w1/report.md` writes the NESTED
    /// file. The suffix matcher used to confirm the peer's
    /// `workspaces/w1/report.md` with it; only the resolved path counts now.
    #[test]
    fn a_relative_write_never_confirms_a_peers_same_named_file() {
        let dir = shared_vault();
        let v = dir.path();
        let mut c = EvidenceCollector::new("chat", None, "run-f1-nested");
        c.context_mut().set_run_cwd(&v.join("workspaces/w1"));
        c.record_tool_use(
            "w",
            "Write",
            &json!({ "file_path": "workspaces/w1/report.md" }),
        );
        c.record_tool_result("w", "ok", false);
        let r = c
            .finish(
                v,
                "success",
                vec![
                    shared_change("workspaces/w1/report.md"),
                    shared_change("workspaces/w1/workspaces/w1/report.md"),
                ],
            )
            .unwrap();
        assert_eq!(file(&r, "workspaces/w1/report.md").attribution, "shared");
        assert_eq!(
            file(&r, "workspaces/w1/workspaces/w1/report.md").attribution,
            "confirmed"
        );
    }

    /// F1 review (red first): a write is owned by the file it really
    /// reaches: through a symlinked folder inside the workspace, through
    /// `..` after a symlink, in another letter case, and a deleted file in
    /// another case matches History's spelling; apply_patch's `Move to:`
    /// destination is owned too.
    #[test]
    fn a_write_is_owned_by_the_file_it_really_reaches() {
        let dir = shared_vault();
        let v = dir.path();
        let w1 = v.join("workspaces/w1");
        fs::create_dir_all(w1.join("real/sub")).unwrap();
        std::os::unix::fs::symlink(w1.join("real"), w1.join("link")).unwrap();
        std::os::unix::fs::symlink(w1.join("real/sub"), w1.join("deep")).unwrap();
        fs::write(w1.join("Notes.md"), "n").unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-f1-identity");
        c.context_mut().set_run_cwd(&w1);
        let calls = [
            ("Write", json!({ "file_path": w1.join("link/report.md") })),
            ("Write", json!({ "file_path": "deep/../up.md" })),
            ("Edit", json!({ "file_path": w1.join("notes.md") })),
            ("Write", json!({ "file_path": w1.join("gone.md") })),
            (
                "apply_patch",
                json!({ "patch": "*** Update File: old.md\n*** Move to: new.md\n" }),
            ),
        ];
        for (i, (name, input)) in calls.iter().enumerate() {
            let id = format!("c{i}");
            c.record_tool_use(&id, name, input);
            c.record_tool_result(&id, "ok", false);
        }
        let changes = [
            "workspaces/w1/real/report.md",
            "workspaces/w1/real/up.md",
            "workspaces/w1/Notes.md",
            "workspaces/w1/Gone.md",
            "workspaces/w1/old.md",
            "workspaces/w1/new.md",
        ];
        let r = c
            .finish(
                v,
                "success",
                changes.iter().map(|p| shared_change(p)).collect(),
            )
            .unwrap();
        for path in changes {
            assert_eq!(file(&r, path).attribution, "confirmed", "{path}");
        }
    }

    /// F1 delta review (red first): another letter case only names the same
    /// file for a deleted one, and only when its folder's volume reports it
    /// ignores case (`pathconf`, injected here); on a case-sensitive or
    /// unknown volume it is someone else's file. An existing file only ever
    /// matches its resolved name exactly.
    #[test]
    fn letter_case_matches_only_a_gone_file_on_a_case_insensitive_volume() {
        use crate::automated_history::{force_case_insensitive_for_test, same_file};
        let dir = shared_vault();
        let v = dir.path();
        let w1 = v.join("workspaces/w1");
        fs::write(w1.join("Here.md"), "x").unwrap();
        let receipt_for = |answer: Option<bool>| {
            force_case_insensitive_for_test(Some(answer));
            let mut c = EvidenceCollector::new("chat", None, "run-f1-case");
            c.context_mut().set_run_cwd(&w1);
            c.record_tool_use("w", "Write", &json!({ "file_path": w1.join("gone.md") }));
            c.record_tool_result("w", "ok", false);
            c.finish(v, "success", vec![shared_change("workspaces/w1/Gone.md")])
                .unwrap()
        };
        assert_eq!(receipt_for(Some(false)).files[0].attribution, "shared");
        assert_eq!(receipt_for(None).files[0].attribution, "shared");
        assert_eq!(receipt_for(Some(true)).files[0].attribution, "confirmed");
        force_case_insensitive_for_test(None);
        assert!(!same_file(
            v,
            "workspaces/w1/here.md",
            "workspaces/w1/Here.md"
        ));
        assert!(same_file(
            v,
            "workspaces/w1/Here.md",
            "workspaces/w1/Here.md"
        ));
    }

    /// F1 delta review (red first): spellings of one file in another case
    /// take the LATEST completion among them. Writing `Notes.md`, checking,
    /// then deleting it as `notes.md` leaves the check stale, never
    /// verified.
    #[test]
    fn equivalent_spellings_take_the_latest_write() {
        let dir = shared_vault();
        let v = dir.path();
        let w1 = v.join("workspaces/w1");
        let mut c = EvidenceCollector::new("chat", None, "run-f1-latest");
        c.context_mut().set_run_cwd(&w1);
        c.record_tool_use("w", "Write", &json!({ "file_path": w1.join("Notes.md") }));
        c.record_tool_result("w", "ok", false);
        c.record_tool_use("t", "Bash", &json!({ "command": "npm test" }));
        c.record_tool_result("t", "12 passed", false);
        c.record_tool_use(
            "d",
            "apply_patch",
            &json!({ "patch": "*** Delete File: notes.md\n" }),
        );
        c.record_tool_result("d", "ok", false);
        let deleted = ObservedChange {
            path: "workspaces/w1/Notes.md".into(),
            change_type: "delete".into(),
            ..Default::default()
        };
        let r = c.finish(v, "success", vec![deleted]).unwrap();
        assert_eq!(r.files[0].attribution, "confirmed");
        assert_ne!(r.verification_state, "verifiedAfterChange");
    }

    #[test]
    fn write_then_test_is_verified() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("notes/a.md");
        let mut c = EvidenceCollector::new("chat", Some("session"), "run-1");
        c.record_tool_use(
            "w",
            "Write",
            &json!({ "file_path": target.to_string_lossy() }),
        );
        c.record_tool_result("w", "ok", false);
        c.record_tool_use("t", "Bash", &json!({ "command": "npm test" }));
        c.record_tool_result("t", "12 passed", false);
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.verification_state, "verifiedAfterChange");
        assert_eq!(r.files[0].attribution, "confirmed");
        assert_eq!(r.checks[0].kind, "test");
    }

    /// Canvas C3 wave 2 C4: a reply's canvas fences land on its receipt as
    /// `canvases: {count, bytes}`; a reply with none leaves the key out, so
    /// older receipts and canvas-free turns read the same.
    #[test]
    fn the_receipt_records_the_reply_canvases() {
        let dir = tempdir().unwrap();
        let body = r#"<hc-canvas id="a" title="Café">é</hc-canvas>"#;
        let reply = format!(
            "Done.\n```hollow-canvas\n{body}\n```\n```html\n<p>x</p>\n```\n```hollow-canvas\nxy\n```"
        );
        let mut c = EvidenceCollector::new("chat", None, "run-canvases");
        c.set_reply_canvases(&reply);
        let r = c.finish(dir.path(), "success", Vec::new()).unwrap();
        let expected = ReceiptCanvases {
            count: 2,
            bytes: body.len() as u64 + 2,
        };
        assert_eq!(r.canvases, Some(expected));
        let wire = serde_json::to_value(&r).unwrap();
        assert_eq!(
            wire["canvases"],
            json!({ "count": 2, "bytes": expected.bytes })
        );
        let back: EvidenceReceipt = serde_json::from_value(wire).unwrap();
        assert_eq!(back.canvases, Some(expected));

        let mut none = EvidenceCollector::new("chat", None, "run-no-canvases");
        none.set_reply_canvases("Plain answer.\n```js\nlet a;\n```");
        let r = none.finish(dir.path(), "success", Vec::new()).unwrap();
        assert_eq!(r.canvases, None);
        let wire = serde_json::to_value(&r).unwrap();
        assert!(wire.get("canvases").is_none(), "absent, not null: {wire}");
        let old: EvidenceReceipt = serde_json::from_value(json!({
            "id": "x", "runId": "r", "source": "chat", "sourceId": null,
            "startedAt": "", "finishedAt": "", "status": "success",
            "verificationState": "noChanges", "files": [], "checks": [],
            "failures": [], "historyRunId": "r"
        }))
        .unwrap();
        assert_eq!(old.canvases, None, "an older receipt reads without it");
    }

    #[test]
    fn bounded_check_commands_are_recognized() {
        for (command, expected) in [
            ("npm test", "test"),
            ("npm run test:smoke -- --project desktop", "test"),
            ("cargo test --lib", "test"),
            ("python -m pytest tests/unit", "test"),
            ("npm run typecheck", "typecheck"),
            ("npx eslint src", "lint"),
            ("cargo build --release", "build"),
            ("npx tauri build --bundles app", "build"),
            ("git diff --check", "check"),
            ("cd src-tauri && cargo test --lib", "test"),
            ("cd 'a path' && cd src-tauri && cargo check", "check"),
            // OF16: Hollow's gate, node's test runner and Playwright.
            ("npm run gate", "gate"),
            ("node --test scripts/a.test.mjs", "test"),
            ("npx playwright test --workers=2", "test"),
            // Overnight 9 Spare 5: setup wrappers and package runners.
            ("timeout 600 cargo test --lib", "test"),
            ("gtimeout -k 5 -s TERM 10m npm test", "test"),
            ("timeout --signal=KILL 30s npx vitest run", "test"),
            ("npx -y vitest run", "test"),
            ("npx --yes tsc --noEmit", "typecheck"),
            ("nice -n 10 cargo build", "build"),
            ("nice -5 cargo check", "check"),
            ("nohup cargo test", "test"),
            ("time -p cargo test", "test"),
            ("pnpm exec vitest run", "test"),
            ("pnpm dlx eslint src", "lint"),
            ("yarn dlx tsc", "typecheck"),
            ("npm exec -- playwright test", "test"),
            ("bunx vitest", "test"),
            ("bun x tsc", "typecheck"),
            ("bun test", "test"),
            ("env CI=1 npm test", "test"),
            ("env -i -u FOO NODE_ENV=test npx vitest", "test"),
            ("CI=1 cargo test", "test"),
            (
                "cd src-tauri && timeout 900 cargo clippy -- -D warnings",
                "lint",
            ),
        ] {
            assert_eq!(check_kind(command), Some(expected), "{command}");
        }
        // A wrapper alone, or a timeout with no duration, is no check.
        for command in [
            "timeout cargo test",
            "timeout 10",
            "timeout --preserve-status 10 cargo test",
            "env -S 'cargo test'",
            "env -C dir cargo test",
            "nohup",
            "pnpm exec",
            "bunx --yes",
        ] {
            assert_eq!(check_kind(command), None, "{command}");
        }
    }

    /// OF16: the checks of real scripts, simple command by simple command:
    /// past `cd`, assignments, redirects, pipes, `||`, a `<shell> -lc`
    /// wrapper, comments and here-document bodies.
    #[test]
    fn real_scripts_hold_checks_segment_by_segment() {
        for (command, expected) in [
            ("cd /r && npx vitest run 2>&1 | tail -20", vec!["test"]),
            (
                "set -e\ncd /r\nnpm run gate > $TMPDIR/g.log 2>&1; tail -5 $TMPDIR/g.log",
                vec!["gate"],
            ),
            ("npm test || true", vec!["test"]),
            ("cargo test --lib 2>&1 | tail -3", vec!["test"]),
            ("cargo test --workspace 2>&1 \\\n  | head -40", vec!["test"]),
            ("CI=1 env TZ=UTC node --test a.mjs", vec!["test"]),
            ("zsh -lc 'npx tsc --noEmit | head'", vec!["typecheck"]),
            // Overnight 9 Spare 5: a shell wrapper past setup words, and
            // wrappers inside a script.
            ("env CI=1 bash -c 'cargo test 2>&1 | tail'", vec!["test"]),
            ("TZ=UTC zsh -lc 'npx -y vitest run'", vec!["test"]),
            ("set -e\ntimeout 600 npm run gate", vec!["gate"]),
            ("nice -n 5 pnpm exec tsc --noEmit | head", vec!["typecheck"]),
            ("# run the gate (it's slow)\nnpm run gate", vec!["gate"]),
            (
                "cat > f.md <<'EOF'\nnpm test\nIt's data\nEOF\ncargo check",
                vec!["check"],
            ),
            (
                "cargo fmt --check && cargo clippy -- -D warnings && cargo test",
                vec!["lint", "test"],
            ),
            ("echo npm test | tee x.txt", vec![]),
            ("rg 'cargo test' docs", vec![]),
            ("npm test -- --help | head", vec![]),
            ("$(printf npm) test", vec![]),
            // `--test` passed to the script, not node's own flag.
            ("node x.mjs --test", vec![]),
            // A here-document still open at the end: nothing can be told.
            ("cargo test <<EOF\ninput", vec![]),
        ] {
            assert_eq!(checks_in(command), expected, "{command:?}");
        }
    }

    /// Overnight 9 Spare 5: a check behind a wrapper or a package runner is
    /// read by its own runner's summary, and stores its own words.
    #[test]
    fn wrapped_checks_keep_their_runner() {
        for (command, runner, words) in [
            (
                "timeout 600 npx -y vitest run 2>&1 | tail",
                "vitest",
                "npx -y vitest run",
            ),
            (
                "pnpm exec vitest run | tail",
                "vitest",
                "pnpm exec vitest run",
            ),
            (
                "env CI=1 bash -c 'nice -n 5 cargo test --lib | tail'",
                "cargo-test",
                "cargo test --lib",
            ),
            ("bunx tsc --noEmit | head", "tsc", "bunx tsc --noEmit"),
        ] {
            let checks = shell_checks(command);
            assert_eq!(checks.len(), 1, "{command}");
            assert_eq!(checks[0].reader, Reader::Runner(runner), "{command}");
            assert_eq!(checks[0].words, words, "{command}");
        }
        // A runner's command no runner reads stays unread, as it is bare:
        // another runner's pass line never passes `pnpm exec pytest`.
        for command in ["pnpm exec pytest | tail", "bun x mypy . | head"] {
            let checks = shell_checks(command);
            assert_eq!(checks.len(), 1, "{command}");
            assert_eq!(checks[0].reader, Reader::Unread, "{command}");
        }
    }

    /// OF16: a check's outcome on a receipt. The exit status decides where
    /// nothing can mask it; otherwise its runner's summary in the output
    /// does (failure text first), and nothing else ever says `passed`.
    #[test]
    fn a_masked_checks_outcome_comes_from_its_runner_summary() {
        let dir = tempdir().unwrap();
        let outcome = |command: &str, output: &str, is_error: bool| {
            let mut c = EvidenceCollector::new("chat", None, "run-of16");
            c.record_tool_use("t", "Bash", &json!({ "command": command }));
            c.record_tool_result("t", output, is_error);
            let r = c.finish(dir.path(), "success", vec![]).unwrap();
            assert_eq!(r.checks.len(), 1, "{command:?}");
            r.checks[0].outcome.clone()
        };
        let vitest = "cd /r && npx vitest run 2>&1 | tail -20";
        let gate = "set -e\ncd /r\nnpm run gate > $TMPDIR/g.log 2>&1; tail -5 $TMPDIR/g.log";
        let or_true = "npm test || true";
        let cargo = "cargo test --lib 2>&1 | tail -3";
        let cargo_long = "cargo test 2>&1 | tail -200";
        let failing = " Test Files  1 failed | 3 passed (4)\n Tests  2 failed | 8 passed";
        let passing = " Test Files  4 passed (4)\n      Tests  10 passed (10)\n";
        let one_failed = "      Tests  1 failed (1)";
        let coloured = "\x1b[31m      Tests  \x1b[1m2 failed\x1b[22m\x1b[39m | 8 passed";
        let no_summary = "warning: unused import\n[gate-record] not recorded: dirty tree.";
        let recorded = "[gate-record] Gate: `npm run gate` passed on clean `0123abcd`.";
        let red = "test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured";
        let green = "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured";
        let broken = "error: could not compile `hollow` (lib test)";
        // The summary is read from the whole output, not the stored excerpt.
        let long = format!("{}{red}", "running 1 test\n".repeat(80));
        assert!(long.chars().count() > EXCERPT_CHARS);
        for (command, output, is_error, expected) in [
            (vitest, failing, false, "failed"),
            (vitest, passing, false, "passed"),
            (vitest, coloured, false, "failed"),
            // No summary: the exit status of `tail` says nothing.
            (vitest, "", false, "unknown"),
            (gate, no_summary, false, "unknown"),
            (gate, recorded, false, "passed"),
            // A runner's pass line inside the gate never passes the gate.
            (gate, passing, false, "unknown"),
            (or_true, passing, false, "passed"),
            (or_true, "> vitest run", false, "unknown"),
            (or_true, one_failed, false, "failed"),
            (cargo, red, false, "failed"),
            (cargo, green, false, "passed"),
            (cargo, broken, false, "failed"),
            (cargo_long, long.as_str(), false, "failed"),
            // Strict: the last command, unmasked, keeps the exit status.
            ("cargo test", "ok", false, "passed"),
            ("cargo test", red, true, "failed"),
            ("cd /r && cargo test > /tmp/t.log", "", false, "passed"),
            // `&&` to the end: exit 0 proves it; a failure may be a later one's.
            ("npm test && echo done", "done", false, "passed"),
            ("npm test && echo done", "done", true, "unknown"),
            // In the background only a failure in the output counts.
            ("npm test &", passing, false, "unknown"),
            ("npm test &", one_failed, false, "failed"),
            // After `||` it may not have run at all.
            ("false || cargo test", "", false, "unknown"),
        ] {
            let got = outcome(command, output, is_error);
            assert_eq!(got, expected, "{command:?} {output:?}");
        }
    }

    /// OF16 review: output never passes a check through another check's
    /// summary (same kind, or a runner it calls), through a filter that can
    /// drop the failure line, or on a line the runner prints even when it
    /// failed; and failure text wins over a clean exit.
    #[test]
    fn output_passes_a_check_only_on_its_own_whole_summary() {
        let dir = tempdir().unwrap();
        let outcomes = |command: &str, output: &str| {
            let mut c = EvidenceCollector::new("chat", None, "run-of16-review");
            c.record_tool_use("t", "Bash", &json!({ "command": command }));
            c.record_tool_result("t", output, false);
            let r = c.finish(dir.path(), "success", vec![]).unwrap();
            r.checks
                .iter()
                .map(|check| check.outcome.clone())
                .collect::<Vec<String>>()
        };
        let passing = " Test Files  4 passed (4)\n      Tests  10 passed (10)\n";
        let failing = " Test Files  1 failed (1)\n      Tests  2 failed (5)\n";
        let green = "test result: ok. 3 passed; 0 failed; 0 ignored";
        let red = "test result: FAILED. 1 passed; 2 failed; 0 ignored";
        // 1. Another check's pass line: same kind, or cargo inside tauri.
        let node_then_vitest = "node --test a.mjs >/dev/null || true; npx vitest run";
        let tauri = "npx tauri build 2>&1 | tail -30";
        let compiled = "    Finished `release` profile [optimized] target(s) in 1m 02s";
        let unbundled = format!("{compiled}\n       Error failed to bundle project: x");
        let bundled = "    Finished 2 bundles at:\n        /x/a.dmg";
        // 2. Playwright prints its pass count last, after the failures.
        let playwright = "npx playwright test | tail -1";
        let playwright_red = "  2 failed\n    [chromium] a.spec.ts:3:1\n  10 passed (30.2s)";
        // 3. `Tests  10 passed` is left when `Test Files  1 failed` was cut.
        let vitest_tail = "npx vitest run | tail -3";
        let cut = "      Tests  10 passed (10)\n   Start at  10:00:00\n   Duration  2.10s";
        // 4. `head`/`grep` keep a first `ok` and drop a later `FAILED`.
        let head = "cargo test --lib 2>&1 | head -20";
        let grep = "cargo test 2>&1 | grep 'test result'";
        let log_grep = "cargo test > t.log 2>&1; grep 'test result' t.log";
        // Two checks of one kind: neither can tell its own summary.
        let two = "npx vitest run 2>&1 | tail -5; npm test 2>&1 | tail -5";
        let both_redirects = "npm test &> /tmp/t.log; tail -5 /tmp/t.log";
        let prose = "retrying after 1 failed attempt";
        for (command, output, expected) in [
            (node_then_vitest, passing, vec!["unknown", "passed"]),
            (tauri, compiled, vec!["unknown"]),
            (tauri, unbundled.as_str(), vec!["failed"]),
            (tauri, bundled, vec!["passed"]),
            (playwright, "  10 passed (30.2s)", vec!["unknown"]),
            ("npx playwright test", playwright_red, vec!["failed"]),
            (vitest_tail, cut, vec!["unknown"]),
            (vitest_tail, failing, vec!["failed"]),
            (head, green, vec!["unknown"]),
            (head, red, vec!["failed"]),
            (grep, green, vec!["unknown"]),
            (log_grep, green, vec!["unknown"]),
            (two, passing, vec!["unknown", "unknown"]),
            (two, failing, vec!["failed", "failed"]),
            // `|&` is a pipe and `&>` a redirect, not a background run.
            ("cargo test |& tail -3", green, vec!["passed"]),
            ("cargo test |& tail -3", red, vec!["failed"]),
            (both_redirects, passing, vec!["passed"]),
            // Failure text wins over a clean exit (`npm test` = `vitest; eslint`).
            ("npm test", failing, vec!["failed"]),
            // Only Playwright's own summary line is a failure, not prose.
            ("npm test || true", prose, vec!["unknown"]),
        ] {
            let got = outcomes(command, output);
            assert_eq!(got, expected, "{command:?} {output:?}");
        }
    }

    /// OF16 re-review: a line continuation doesn't hide the `| head` or `&`
    /// after it; output cut short (a timeout, a truncation marker, a later
    /// cargo test binary that started after the `ok.`) never passes; nor
    /// does a check whose stderr was dropped, whose output was read back
    /// other than whole, a package script beside another check, or one an
    /// earlier `exit` or an `||` before its pipeline can mask. Failure text
    /// still fails each.
    #[test]
    fn output_never_passes_a_check_it_may_have_cut_or_dropped() {
        let dir = tempdir().unwrap();
        let outcomes = |command: &str, output: &str, is_error: bool| {
            let mut c = EvidenceCollector::new("chat", None, "run-of16-re-review");
            c.record_tool_use("t", "Bash", &json!({ "command": command }));
            c.record_tool_result("t", output, is_error);
            let r = c.finish(dir.path(), "success", vec![]).unwrap();
            r.checks
                .iter()
                .map(|check| check.outcome.clone())
                .collect::<Vec<String>>()
        };
        let green = "test result: ok. 3 passed; 0 failed; 0 ignored";
        let red = "test result: FAILED. 1 passed; 2 failed; 0 ignored";
        // B1: a `\`⏎ before the pipe or the `&`.
        let continued_head = "cargo test --workspace 2>&1 \\\n  | head -40";
        let continued_bg = "cargo test \\\n  &";
        // B2: one binary's `ok.`, then the next binary or a timeout.
        let tee = "cargo test 2>&1 | tee /tmp/t.log";
        let lib_ok = "test result: ok. 1890 passed; 0 failed; 0 ignored";
        let next_binary = format!("{lib_ok}\n\n     Running tests/smoke.rs\n\nrunning 12 tests");
        let doc_tests = format!("{lib_ok}\n\n   Doc-tests hollow\n\nrunning 4 tests");
        let whole = format!("{lib_ok}\n\n   Doc-tests hollow\n\nrunning 0 tests\n\n{green}");
        let timed_out = format!("{green}\nCommand timed out after 2m 0.0s");
        let cut = format!("running 3 tests\n... [1200 lines truncated] ...\n{green}");
        let doctest_failed = format!("{green}\nerror: doctest failed, to rerun pass `--doc`");
        // M1: stderr dropped, stdout's pass line kept.
        let built = "vite v5.4.0 building for production...\n✓ built in 2.31s";
        // M2: vitest passed, then the script's next runner failed.
        let pnpm = "pnpm test 2>&1 | tail -15";
        let vitest_green = " Test Files  12 passed (12)\n      Tests  80 passed (80)";
        let elifecycle = format!("{vitest_green}\nError: x\n ELIFECYCLE  Test failed. See above.");
        let yarn = format!("{vitest_green}\nerror Command failed with exit code 1.");
        let bun = format!("{vitest_green}\nerror: script \"test\" exited with code 1");
        let build_then_check = "pnpm build 2>&1 | tail -5; cargo check 2>&1 | tail -2";
        let finished = "    Finished `dev` profile [unoptimized] target(s) in 3.20s";
        // M3: a coverage threshold miss after the summary.
        let vitest_cov = "npx vitest run --coverage 2>&1 | tail -20";
        let miss = "ERROR: Coverage for lines (80%) does not meet global threshold (90%)";
        let below = format!("{vitest_green}\n{miss}");
        let jest_cov = "npx jest --coverage 2>&1 | tail -20";
        let jest_miss = "Jest: \"global\" coverage threshold for lines (90%) not met: 80%";
        let jest_below = format!("Test Suites: 3 passed, 3 total\n{jest_miss}");
        // M4: a redirect read back by anything but a whole tail or cat.
        let python = "cargo test > t.log 2>&1; python3 -c \"print(open('t.log').read()[:9])\"";
        for (command, output, is_error, expected) in [
            (continued_head, green, false, vec!["unknown"]),
            (continued_head, red, false, vec!["failed"]),
            (continued_bg, green, false, vec!["unknown"]),
            (tee, lib_ok, true, vec!["unknown"]),
            (tee, red, true, vec!["failed"]),
            (tee, next_binary.as_str(), false, vec!["unknown"]),
            (tee, doc_tests.as_str(), false, vec!["unknown"]),
            (tee, whole.as_str(), false, vec!["passed"]),
            (tee, timed_out.as_str(), false, vec!["unknown"]),
            (tee, cut.as_str(), false, vec!["unknown"]),
            (tee, doctest_failed.as_str(), false, vec!["failed"]),
            (
                "npm run build 2>/dev/null | tail -5",
                built,
                false,
                vec!["unknown"],
            ),
            (
                "npx vite build 2>err.log | tail -5",
                built,
                false,
                vec!["unknown"],
            ),
            (
                "npx vite build 2>&1 | tail -5",
                built,
                false,
                vec!["passed"],
            ),
            (
                "npx vite build 2>&1 >b.log | tail -5",
                built,
                false,
                vec!["unknown"],
            ),
            (pnpm, vitest_green, false, vec!["passed"]),
            (pnpm, elifecycle.as_str(), false, vec!["failed"]),
            (pnpm, yarn.as_str(), false, vec!["failed"]),
            (pnpm, bun.as_str(), false, vec!["failed"]),
            (build_then_check, finished, false, vec!["unknown", "passed"]),
            (vitest_cov, below.as_str(), false, vec!["failed"]),
            (jest_cov, jest_below.as_str(), false, vec!["failed"]),
            (python, green, false, vec!["unknown"]),
            (python, red, false, vec!["failed"]),
            (
                "cargo test >> t.log 2>&1; tail -5 t.log",
                green,
                false,
                vec!["unknown"],
            ),
            (
                "cargo test > t.log 2>&1; tail -5 old.log",
                green,
                false,
                vec!["unknown"],
            ),
            (
                "cargo test > t.log 2>&1; tail -n 5 t.log",
                green,
                false,
                vec!["passed"],
            ),
            (
                "cargo test > t.log 2>&1; cat t.log | tail -5",
                green,
                false,
                vec!["passed"],
            ),
            // L1: an earlier `exit` or `trap` can end the script on its own.
            (
                "test -f .skip && exit 0\ncargo test",
                "",
                false,
                vec!["unknown"],
            ),
            ("trap 'exit 0' EXIT; cargo test", "", false, vec!["unknown"]),
            (
                "test -f .skip && exit 0\ncargo test",
                red,
                false,
                vec!["failed"],
            ),
            // L2: an `||` before its pipeline: it may not have run.
            ("false || echo x | cargo test", "", false, vec!["unknown"]),
        ] {
            let got = outcomes(command, output, is_error);
            assert_eq!(got, expected, "{command:?} {output:?} {is_error}");
        }
    }

    /// OF16: such checks reach the receipt's verification state.
    #[test]
    fn piped_checks_after_a_write_verify_or_fail_the_receipt() {
        let dir = tempdir().unwrap();
        let receipt = |run: &str, command: &str, output: &str| {
            let mut c = EvidenceCollector::new("chat", None, run);
            c.record_tool_use("w", "Write", &json!({ "file_path": "notes/a.md" }));
            c.record_tool_result("w", "ok", false);
            c.record_tool_use("t", "Bash", &json!({ "command": command }));
            c.record_tool_result("t", output, false);
            c.finish(dir.path(), "success", changes()).unwrap()
        };
        let cargo = "cd src-tauri && cargo test --lib 2>&1 | tail -3";
        let red = "test result: FAILED. 1 passed; 2 failed;";
        let r = receipt("run-of16-a", cargo, red);
        assert_eq!(r.checks[0].kind, "test");
        assert_eq!(r.checks[0].outcome, "failed");
        assert_eq!(r.verification_state, "failedAfterChange");
        assert!(r.failures.is_empty(), "the call itself didn't error");
        assert_eq!(r.checks[0].excerpt, None, "no more tool output stored");

        let vitest = "npx vitest run 2>&1 | tail -5";
        let passing = " Test Files  4 passed (4)\n      Tests  10 passed (10)";
        let r = receipt("run-of16-b", vitest, passing);
        assert_eq!(r.verification_state, "verifiedAfterChange");

        let r = receipt("run-of16-c", vitest, "");
        assert_eq!(r.checks[0].outcome, "unknown");
        assert_eq!(r.verification_state, "notVerified");

        // A check stores its own words: no assignment, no rest of the script.
        let script = "TOKEN=abc npx vitest run 2>&1 | tail -5\ncat <<EOF\nsecret\nEOF";
        let r = receipt("run-of16-d", script, passing);
        assert_eq!(r.checks[0].command, "npx vitest run");
        // Newly strict checks keep no excerpt; one strict before OF16 does.
        let r = receipt("run-of16-e", "npm run gate", "ok");
        assert_eq!(r.checks[0].excerpt, None);
        let r = receipt("run-of16-f", "npm test", "ok");
        assert_eq!(r.checks[0].excerpt.as_deref(), Some("ok"));
    }

    /// S27 (red first): a final reply that claims a pass is held against
    /// the checks OF16 recognises in the run's stream: none after its last
    /// write, or a last check that failed, is flagged; no claim (or a
    /// negated one) flags nothing.
    #[test]
    fn a_pass_claim_without_a_check_or_after_a_failed_one_is_flagged() {
        let dir = tempdir().unwrap();
        let cargo = "cd src-tauri && cargo test --lib 2>&1 | tail -3";
        let red = "test result: FAILED. 1 passed; 2 failed;";
        let vitest = "npx vitest run 2>&1 | tail -5";
        let green = " Test Files  4 passed (4)\n      Tests  10 passed (10)";
        // Each step is a check `(command, output)`, or `None` for an edit.
        let receipt = |run: &str, steps: &[Option<(&str, &str)>], reply: &str| {
            let mut c = EvidenceCollector::new("chat", None, run);
            for (i, step) in steps.iter().enumerate() {
                let id = format!("call-{i}");
                match step {
                    None => {
                        c.record_tool_use(&id, "Edit", &json!({ "file_path": "notes/a.md" }));
                        c.record_tool_result(&id, "ok", false);
                    }
                    Some((command, output)) => {
                        c.record_tool_use(&id, "Bash", &json!({ "command": command }));
                        c.record_tool_result(&id, output, false);
                    }
                }
            }
            c.set_reply_claim(reply);
            c.finish(dir.path(), "success", changes()).unwrap()
        };
        let flags = |r: &EvidenceReceipt| {
            let v = r
                .verification
                .clone()
                .expect("a chat reply's claim is recorded");
            (v.claim, v.claim_without_check, v.claim_after_fail)
        };

        // An edit, a piped cargo test that failed, then "all tests pass".
        let r = receipt("run-s27-a", &[None, Some((cargo, red))], "all tests pass");
        assert_eq!(r.checks[0].outcome, "failed");
        assert_eq!(flags(&r), ("passed".to_string(), false, true));
        let wire = serde_json::to_value(&r).unwrap();
        assert_eq!(
            wire["verification"],
            json!({ "claim": "passed", "claimAfterFail": true })
        );

        // An edit, a passing check after it: the claim is backed.
        let r = receipt(
            "run-s27-b",
            &[None, Some((vitest, green))],
            "All tests pass.",
        );
        assert_eq!(flags(&r), ("passed".to_string(), false, false));

        // A check, then an edit, then a claim with no check after the edit.
        let r = receipt("run-s27-c", &[Some((vitest, green)), None], "tests pass");
        assert_eq!(flags(&r), ("passed".to_string(), true, false));

        // No completion claim, and a negated pass: claim "none", nothing
        // flagged, even after a failed check.
        for (run, reply) in [
            ("run-s27-d", "See the diff"),
            ("run-s27-e", "the tests don't pass yet"),
        ] {
            let r = receipt(run, &[None, Some((cargo, red))], reply);
            assert_eq!(flags(&r), ("none".to_string(), false, false), "{reply}");
            let wire = serde_json::to_value(&r).unwrap();
            assert_eq!(wire["verification"], json!({ "claim": "none" }));
        }

        // "Done" and "fixed" are claims too. A failed check after the edit
        // is the stronger flag; a fix with no check after the edit is
        // unconfirmed.
        let r = receipt(
            "run-s27-done",
            &[None, Some((cargo, red))],
            "Done, see diff",
        );
        assert_eq!(flags(&r), ("done".to_string(), false, true));
        let r = receipt("run-s27-fixed", &[None], "It's fixed.");
        assert_eq!(flags(&r), ("fixed".to_string(), true, false));
        // A check after the edit confirms it: the claim is recorded, no flag.
        let r = receipt(
            "run-s27-done-ok",
            &[None, Some((vitest, green))],
            "All done.",
        );
        assert_eq!(flags(&r), ("done".to_string(), false, false));

        // Only the final reply counts: a claim before the last tool call
        // doesn't.
        let mut c = EvidenceCollector::new("chat", None, "run-s27-f");
        let early = "Edited it; all tests pass.";
        c.mark_reply_start(early.len());
        c.set_reply_claim(&format!("{early}\nSee the diff."));
        let r = c.finish(dir.path(), "success", Vec::new()).unwrap();
        assert_eq!(flags(&r), ("none".to_string(), false, false));

        // A subagent ran after the last write: its checks are unknown, so a
        // claim with no visible check after the write isn't flagged.
        let mut c = EvidenceCollector::new("chat", None, "run-s27-g");
        c.record_tool_use("w", "Edit", &json!({ "file_path": "notes/a.md" }));
        c.record_tool_result("w", "ok", false);
        c.record_tool_use("sub", "Task", &json!({ "prompt": "run the tests" }));
        c.record_tool_result("sub", "All tests pass.", false);
        c.set_reply_claim("All tests pass.");
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(flags(&r), ("passed".to_string(), false, false));

        // No reply recorded (another surface), and an older receipt: absent.
        let c = EvidenceCollector::new("workflow", None, "run-s27-h");
        let r = c.finish(dir.path(), "success", Vec::new()).unwrap();
        assert_eq!(r.verification, None);
        assert!(serde_json::to_value(&r)
            .unwrap()
            .get("verification")
            .is_none());
        let old: EvidenceReceipt = serde_json::from_value(json!({
            "id": "x", "runId": "r", "source": "chat", "sourceId": null,
            "startedAt": "", "finishedAt": "", "status": "success",
            "verificationState": "noChanges", "files": [], "checks": [],
            "failures": [], "historyRunId": "r"
        }))
        .unwrap();
        assert_eq!(old.verification, None, "an older receipt reads without it");
    }

    /// S27: the claim patterns, with the negations, conditions and
    /// questions they must not read as a claim.
    #[test]
    fn pass_claims_skip_negations_conditions_and_questions() {
        for reply in [
            "All tests pass.",
            "Done. The gate is green.",
            "test result: ok. 42 passed; 0 failed",
            "The build passes and all checks pass.",
            "**All green**",
            "Tests are passing now.",
            "All tests pass \u{2014} committed as abc123.",
        ] {
            assert!(pass_claim(reply), "{reply}");
        }
        for reply in [
            "Not all tests pass.",
            "If the tests pass, merge it.",
            "Tests failed to pass.",
            "The tests don\u{2019}t pass yet.",
            "Do all tests pass?",
            "Make sure the tests pass.",
            "12 tests passed, 2 failed.",
            "The test pass rate dropped.",
            "No tests pass.",
            "0 passed; 0 failed",
            "Tests pass except the flaky one.",
            "Done, see diff",
        ] {
            assert!(!pass_claim(reply), "{reply}");
        }
    }

    /// Overhaul E3: done and fixed are claims, and a pass phrase still wins.
    /// A hedge, a question, a quote or a bare mention is not one.
    #[test]
    fn done_and_fixed_claims_skip_hedges_questions_and_quotes() {
        assert_eq!(
            reply_claim_kind("Done. The gate is green."),
            ReplyClaim::Passed
        );
        assert_eq!(reply_claim_kind("Done, see diff"), ReplyClaim::Done);
        assert_eq!(reply_claim_kind("All done."), ReplyClaim::Done);
        assert_eq!(reply_claim_kind("It's done."), ReplyClaim::Done);
        assert_eq!(reply_claim_kind("I'm done."), ReplyClaim::Done);
        assert_eq!(reply_claim_kind("This is fixed."), ReplyClaim::Fixed);
        assert_eq!(reply_claim_kind("I fixed the parser."), ReplyClaim::Fixed);
        assert_eq!(reply_claim_kind("Fixed the bug."), ReplyClaim::Fixed);
        assert_eq!(
            reply_claim_kind("No checks were run. Done."),
            ReplyClaim::Done
        );
        assert_eq!(reply_claim_kind("See the diff.\nDone."), ReplyClaim::Done);
        for reply in [
            "Not done.",
            "If it's done, ship it.",
            "Done?",
            "the done state",
            "See the diff",
            "not fixed yet",
            "Make sure it's done.",
            "I fixed `it's fixed` in the comment",
        ] {
            assert_eq!(reply_claim_kind(reply), ReplyClaim::None, "{reply}");
        }
    }

    /// S27 review: a receipt's `verification` as (claim, claimWithoutCheck,
    /// claimAfterFail).
    fn s27_flags(r: &EvidenceReceipt) -> (String, bool, bool) {
        let v = r
            .verification
            .clone()
            .expect("a chat reply's claim is recorded");
        (v.claim, v.claim_without_check, v.claim_after_fail)
    }

    /// S27 review (red first): an agent that reports no tool calls
    /// (Antigravity) has nothing to hold its claim against: the claim is
    /// recorded, neither flag is raised.
    #[test]
    fn unreported_tool_calls_record_a_pass_claim_without_flags() {
        let dir = tempdir().unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-s27r-a");
        c.mark_tool_calls_unreported();
        c.set_reply_claim("All tests pass.");
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(s27_flags(&r), ("passed".to_string(), false, false));
        let wire = serde_json::to_value(&r).unwrap();
        assert_eq!(wire["verification"], json!({ "claim": "passed" }));
    }

    /// S27 review (red first): a subagent started in the background never
    /// finishes in this stream, so no later check or write can be trusted
    /// to be the run's last: both flags stay false. The same subagent in the
    /// foreground, ended before the write, still lets the flag through.
    #[test]
    fn a_background_subagent_leaves_the_claim_flags_unknown() {
        let dir = tempdir().unwrap();
        let cargo = "cd src-tauri && cargo test --lib 2>&1 | tail -3";
        let red = "test result: FAILED. 1 passed; 2 failed;";
        let agent = |background: bool| {
            json!({
                "description": "run the tests",
                "prompt": "run the tests",
                "run_in_background": background,
            })
        };

        // An edit, a background Agent, a failing check, "all tests pass".
        let mut c = EvidenceCollector::new("chat", None, "run-s27r-b");
        c.record_tool_use("w", "Edit", &json!({ "file_path": "notes/a.md" }));
        c.record_tool_result("w", "ok", false);
        c.record_tool_use("sub", "Agent", &agent(true));
        c.record_tool_result("sub", "Async agent launched successfully.", false);
        c.record_tool_use("t", "Bash", &json!({ "command": cargo }));
        c.record_tool_result("t", red, false);
        c.set_reply_claim("all tests pass");
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.checks[0].outcome, "failed");
        assert_eq!(s27_flags(&r), ("passed".to_string(), false, false));

        // A background Agent, then an edit with no check after it.
        for (run, background, without_check) in
            [("run-s27r-c", true, false), ("run-s27r-d", false, true)]
        {
            let mut c = EvidenceCollector::new("chat", None, run);
            c.record_tool_use("sub", "Agent", &agent(background));
            c.record_tool_result("sub", "launched", false);
            c.record_tool_use("w", "Edit", &json!({ "file_path": "notes/a.md" }));
            c.record_tool_result("w", "ok", false);
            c.set_reply_claim("All tests pass.");
            let r = c.finish(dir.path(), "success", changes()).unwrap();
            assert_eq!(
                s27_flags(&r),
                ("passed".to_string(), without_check, false),
                "background {background}"
            );
        }
    }

    /// S27 review (red first): Codex's `file_change` item (surfaced by
    /// `parse_codex` under its item type) is a write: one after the last
    /// passing check leaves a pass claim without a check.
    #[test]
    fn a_codex_file_change_after_the_last_check_is_a_write() {
        let dir = tempdir().unwrap();
        let vitest = "npx vitest run 2>&1 | tail -5";
        let green = " Test Files  4 passed (4)\n      Tests  10 passed (10)";
        let mut c = EvidenceCollector::new("chat", None, "run-s27r-e");
        c.record_tool_use("item_0", "Bash", &json!({ "command": vitest }));
        c.record_tool_result("item_0", green, false);
        let item = json!({
            "id": "item_1",
            "type": "file_change",
            "changes": [{ "path": "notes/a.md", "kind": "update" }],
            "status": "completed",
        });
        c.record_tool_use("item_1", "file_change", &item);
        c.record_tool_result("item_1", "", false);
        c.set_reply_claim("All tests pass.");
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.checks[0].outcome, "passed");
        assert_eq!(s27_flags(&r), ("passed".to_string(), true, false));
    }

    /// S27 review (red first): a reply mark past the end of the text (the
    /// text it marked is gone) reads no reply, never the whole text.
    #[test]
    fn a_reply_mark_past_the_text_reads_no_claim() {
        let dir = tempdir().unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-s27r-f");
        c.mark_reply_start(400);
        c.set_reply_claim("Edited it; all tests pass.");
        let r = c.finish(dir.path(), "success", Vec::new()).unwrap();
        assert_eq!(s27_flags(&r), ("none".to_string(), false, false));
    }

    /// Overnight A16: a fresh retry's reply starts its own text, so the
    /// failed attempt's mark is reset; a shorter retry reply still records
    /// its claim and its cite.
    #[test]
    fn a_fresh_retry_reads_its_own_shorter_reply() {
        let dir = tempdir().unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-a16-fresh");
        c.mark_reply_start(400);
        // `chat::drive_turn_attempts`' Next::Fresh arm.
        c.mark_reply_start(0);
        let reply =
            "Fixed the parser in a.rs:3; ran the suite and all tests pass now, nothing else.";
        assert!(reply.len() < 400);
        c.set_reply_claim(reply);
        c.set_final_reply_citations(reply);
        assert!(c.has_reply_cites());
        assert_eq!(c.reply_cites.len(), 1);
        let r = c.finish(dir.path(), "success", Vec::new()).unwrap();
        assert_ne!(s27_flags(&r).0, "none");
    }

    /// S27 review (red first): a phrase quoted in inline code or double
    /// quotes is no claim, and a negation five words back still hedges; a
    /// runner summary pasted in a fenced block still claims.
    #[test]
    fn pass_claims_skip_quoted_phrases_and_wider_negations() {
        for reply in [
            "The matcher now flags `all tests pass`",
            "It prints \"all tests pass\" on success.",
            "It prints \u{201C}all tests pass\u{201D} on success.",
            "Not all of the unit tests pass.",
        ] {
            assert!(!pass_claim(reply), "{reply}");
        }
        for reply in [
            "```\ntest result: ok. 42 passed; 0 failed\n```",
            "Ran `cargo test`: all tests pass.",
        ] {
            assert!(pass_claim(reply), "{reply}");
        }
    }

    /// OF16: the runner summaries shared with the scorecard parse, every
    /// pattern compiles here, and none uses syntax a JS reader would read
    /// differently (inline flags, lookaround, anchors).
    #[test]
    fn the_runner_summaries_parse_and_every_pattern_compiles() {
        let runners = parse_runners(RUNNER_SUMMARIES).expect("runner-summaries.json");
        assert!(runners.len() >= 6);
        assert_eq!(RUNNERS.len(), runners.len());
        let known = ["test", "typecheck", "lint", "build", "check", "gate"];
        for runner in &runners {
            assert!(!runner.fail.is_empty() || !runner.pass.is_empty());
            for kind in runner.kinds.iter().chain(&runner.pass_kinds) {
                assert!(known.contains(&kind.as_str()), "{kind}");
            }
            // A fail scope is a runner id or a kind, never both.
            assert!(!known.contains(&runner.id.as_str()), "{}", runner.id);
        }
        // Every runner a direct invocation names has an entry.
        let direct = [
            "npx vitest run",
            "npx jest",
            "npx playwright test",
            "node --test a.mjs",
            "cargo test",
            "cargo clippy",
            "npx tsc --noEmit",
            "npx eslint src",
            "npx vite build",
            "npx tauri build",
            "npm run gate",
        ];
        for command in direct {
            let words: Vec<String> = command.split(' ').map(str::to_string).collect();
            let id = runner_of(&words).expect(command);
            assert!(runners.iter().any(|r| r.id == id), "{command}: {id}");
        }
        let raw: Value = serde_json::from_str(RUNNER_SUMMARIES).unwrap();
        let truncated = parse_truncated(RUNNER_SUMMARIES).expect("truncated patterns");
        assert!(!truncated.is_empty());
        assert_eq!(TRUNCATED.len(), truncated.len());
        let mut patterns: Vec<(&str, &Value)> = raw["truncated"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| ("truncated", p))
            .collect();
        for runner in raw["runners"].as_array().unwrap() {
            let id = runner["id"].as_str().unwrap();
            for key in ["fail", "pass", "notAfterPass"] {
                for pattern in runner[key].as_array().into_iter().flatten() {
                    patterns.push((id, pattern));
                }
            }
        }
        for (id, pattern) in patterns {
            let pattern = pattern.as_str().unwrap();
            for js_unsafe in ["(?", "^", "$"] {
                assert!(!pattern.contains(js_unsafe), "{id}: {pattern}");
            }
        }
        let lookbehind = r#"{"runners":[{"id":"x","kinds":["test"],"fail":["(?<=a)b"]}]}"#;
        let error = parse_runners(lookbehind).err().unwrap();
        assert!(error.contains("x `(?<=a)b`"), "{error}");
    }

    #[test]
    fn mentions_wrappers_and_masked_commands_are_not_checks() {
        for command in [
            "echo npm test",
            "rg 'npm test' docs",
            "printf 'cargo build'",
            "sh -c 'npm test'",
            "cd src-tauri && echo cargo test",
            "npm run pretest",
            "npm test || true",
            "npm test | cat",
            "npm test; true",
            "npm test && echo passed",
            "npm test > /tmp/test.log",
            "$(printf 'npm') test",
            "npm test -- --help",
            "cargo test --no-run",
        ] {
            assert_eq!(check_kind(command), None, "{command}");
        }
    }

    #[test]
    fn cd_then_test_is_verified() {
        let mut c = EvidenceCollector::new("chat", None, "run-cd-test");
        c.record_tool_use("w", "Write", &json!({ "file_path": "notes/a.md" }));
        c.record_tool_result("w", "ok", false);
        c.record_tool_use(
            "t",
            "Bash",
            &json!({ "command": "cd src-tauri && cargo test --lib" }),
        );
        c.record_tool_result("t", "527 passed", false);
        let dir = tempdir().unwrap();
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.verification_state, "verifiedAfterChange");
        assert_eq!(r.checks[0].outcome, "passed");
    }

    #[test]
    fn command_that_only_mentions_a_check_does_not_verify() {
        for (index, command) in ["echo npm test", "rg 'npm test' docs"]
            .into_iter()
            .enumerate()
        {
            let mut c = EvidenceCollector::new("chat", None, &format!("run-mention-{index}"));
            c.record_tool_use("w", "Write", &json!({ "file_path": "notes/a.md" }));
            c.record_tool_result("w", "ok", false);
            c.record_tool_use("t", "Bash", &json!({ "command": command }));
            c.record_tool_result("t", "npm test", false);
            let dir = tempdir().unwrap();
            let r = c.finish(dir.path(), "success", changes()).unwrap();
            assert!(r.checks.is_empty(), "{command}");
            assert_eq!(r.verification_state, "notVerified", "{command}");
        }
    }

    /// `npm test || true` exits 0 whatever the tests did: its check is read
    /// from the output (OF16), so a failure there fails the receipt and it
    /// never verifies on the exit status.
    #[test]
    fn masked_failure_command_does_not_verify() {
        let mut c = EvidenceCollector::new("chat", None, "run-masked");
        c.record_tool_use("w", "Write", &json!({ "file_path": "notes/a.md" }));
        c.record_tool_result("w", "ok", false);
        c.record_tool_use("t", "Bash", &json!({ "command": "npm test || true" }));
        c.record_tool_result("t", "1 failed", false);
        let dir = tempdir().unwrap();
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.checks[0].outcome, "failed");
        assert_eq!(r.verification_state, "failedAfterChange");
    }

    #[test]
    fn missing_or_unmatched_result_is_unknown_and_not_verified() {
        for (index, unmatched_first) in [false, true].into_iter().enumerate() {
            let mut c = EvidenceCollector::new("chat", None, &format!("run-missing-{index}"));
            c.record_tool_use("w", "Write", &json!({ "file_path": "notes/a.md" }));
            c.record_tool_result("w", "ok", false);
            if unmatched_first {
                c.record_tool_result("not-the-check", "12 passed", false);
            }
            c.record_tool_use("t", "Bash", &json!({ "command": "npm test" }));
            let dir = tempdir().unwrap();
            let r = c.finish(dir.path(), "success", changes()).unwrap();
            assert_eq!(r.checks[0].outcome, "unknown");
            assert_eq!(r.verification_state, "notVerified");
        }
    }

    #[test]
    fn test_then_write_is_stale() {
        let mut c = EvidenceCollector::new("chat", None, "run-2");
        c.record_tool_use("t", "Bash", &json!({ "command": "cargo test" }));
        c.record_tool_result("t", "ok", false);
        c.record_tool_use("w", "Edit", &json!({ "file_path": "notes/a.md" }));
        c.record_tool_result("w", "ok", false);
        let dir = tempdir().unwrap();
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.verification_state, "staleAfterChange");
    }

    #[test]
    fn failed_check_after_write_is_failed() {
        let mut c = EvidenceCollector::new("workflow", Some("daily"), "run-3");
        c.record_tool_use("w", "Write", &json!({ "file_path": "notes/a.md" }));
        c.record_tool_result("w", "ok", false);
        c.record_tool_use("t", "Bash", &json!({ "command": "npm run typecheck" }));
        c.record_tool_result("t", "TS2322", true);
        let dir = tempdir().unwrap();
        let r = c.finish(dir.path(), "error", changes()).unwrap();
        assert_eq!(r.verification_state, "failedAfterChange");
        assert!(!r.failures.is_empty());
    }

    #[test]
    fn opaque_filesystem_change_is_unknown_order() {
        let mut c = EvidenceCollector::new("chat", None, "run-4");
        c.record_tool_use("b", "Bash", &json!({ "command": "printf x > notes/a.md" }));
        c.record_tool_result("b", "", false);
        c.record_tool_use("t", "Bash", &json!({ "command": "git diff --check" }));
        c.record_tool_result("t", "", false);
        let dir = tempdir().unwrap();
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.verification_state, "unknownOrder");
        assert_eq!(r.files[0].attribution, "observed");
    }

    #[test]
    fn no_changes_is_honest_even_with_checks() {
        let mut c = EvidenceCollector::new("chat", None, "run-5");
        c.record_tool_use("t", "Bash", &json!({ "command": "cargo check" }));
        c.record_tool_result("t", "ok", false);
        let dir = tempdir().unwrap();
        let r = c.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(r.verification_state, "noChanges");
    }

    #[test]
    fn a_merge_without_repo_in_a_later_turn_is_linked_from_its_checkout() {
        let dir = tempdir().unwrap();
        // A main checkout and a linked worktree of it (the build workspace).
        let main = dir.path().join("Hollow");
        std::fs::create_dir_all(main.join(".git/worktrees/lane")).unwrap();
        std::fs::write(
            main.join(".git/config"),
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:example-org/hollow-code.git\n",
        )
        .unwrap();
        std::fs::write(main.join(".git/worktrees/lane/commondir"), "../..\n").unwrap();
        let lane = dir.path().join("lane");
        std::fs::create_dir_all(lane.join("src")).unwrap();
        std::fs::write(
            lane.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/lane").display()),
        )
        .unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-later");
        let bash = |cmd: &str| json!({ "command": cmd });
        c.record_tool_use(
            "t1",
            "Bash",
            &bash(&format!(
                "cd {} && gh pr merge 489 --merge",
                lane.join("src").display()
            )),
        );
        c.record_tool_result("t1", "", false);
        // A later bare merge after that `cd`: it may be another shell, so no guess.
        c.record_tool_use("t2", "Bash", &bash("gh pr merge 490 --merge"));
        c.record_tool_result("t2", "", false);
        // Another host's origin is named but never linked to github.com.
        let ghe = dir.path().join("ghe");
        std::fs::create_dir_all(ghe.join(".git")).unwrap();
        std::fs::write(
            ghe.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://ghe.corp/o/r.git\n",
        )
        .unwrap();
        c.record_tool_use(
            "t3",
            "Bash",
            &bash(&format!("cd \"{}\"; gh pr merge 7 --squash", ghe.display())),
        );
        c.record_tool_result("t3", "", false);
        let r = c.finish(dir.path(), "success", vec![]).unwrap();
        let rows: Vec<(u64, Option<&str>, Option<&str>)> = r
            .pull_requests
            .iter()
            .map(|p| (p.number, p.repo.as_deref(), p.url.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    489,
                    Some("example-org/hollow-code"),
                    Some("https://github.com/example-org/hollow-code/pull/489")
                ),
                (490, None, None),
                (7, Some("ghe.corp/o/r"), None),
            ]
        );
        // No `cd` in the turn: the run's one watched repository (kept after
        // the post-run capture took the baseline); several, or GH_REPO: none.
        let mut c = EvidenceCollector::new("chat", None, "run-root");
        c.repo_roots = vec![lane.clone()];
        c.record_tool_use("r1", "Bash", &bash("gh pr merge 491 --merge"));
        c.record_tool_result("r1", "", false);
        let r = c.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(
            r.pull_requests[0].repo.as_deref(),
            Some("example-org/hollow-code")
        );
        let mut c = EvidenceCollector::new("chat", None, "run-roots");
        c.repo_roots = vec![lane.clone(), main.clone()];
        c.record_tool_use("r1", "Bash", &bash("gh pr merge 492 --merge"));
        c.record_tool_result("r1", "", false);
        assert_eq!(
            c.finish(dir.path(), "success", vec![])
                .unwrap()
                .pull_requests[0]
                .repo,
            None
        );
        let mut c = EvidenceCollector::new("chat", None, "run-ghrepo");
        c.repo_roots = vec![lane.clone()];
        c.record_tool_use("r1", "Bash", &bash("export GH_REPO=o/x"));
        c.record_tool_result("r1", "", false);
        c.record_tool_use("r2", "Bash", &bash("gh pr merge 493 --merge"));
        c.record_tool_result("r2", "", false);
        assert_eq!(
            c.finish(dir.path(), "success", vec![])
                .unwrap()
                .pull_requests[0]
                .repo,
            None
        );
        // The shell's folder carries across calls and lines; a subshell's
        // `cd`, a quoted one, or one that can't be followed doesn't count.
        let obs = |cmds: &[&str]| {
            let mut c = EvidenceCollector::new("chat", None, "run-places");
            for (i, cmd) in cmds.iter().enumerate() {
                let id = format!("p{i}");
                c.record_tool_use(&id, "Bash", &json!({ "command": cmd }));
                c.record_tool_result(&id, "", false);
            }
            merge_places(&c.observations)
        };
        let at = |p: &str| Place::At(std::path::PathBuf::from(p));
        assert_eq!(
            obs(&["cd /a && npm test\ncd /b/c && gh pr merge 3"]),
            [(3, at("/b/c"))]
        );
        assert_eq!(
            obs(&["cd /b/c", "gh pr merge 3"]),
            [(3, Place::Unknown)],
            "an earlier call's cd may be another shell's"
        );
        assert_eq!(
            obs(&["(cd /a; cd /b; make); gh pr merge 4"]),
            [(4, Place::Lost)]
        );
        // A here-document's body is text (its `cd` doesn't move), and the
        // merge's own words are read, quotes and all (S06 delta review).
        assert_eq!(
            obs(&["cat > f <<EOF\ncd /x\nEOF\ncd /b/c && gh pr merge 3"]),
            [(3, at("/b/c"))]
        );
        assert_eq!(
            obs(&["cd /b/c && gh pr merge --squash --body \"# done\" 12"]),
            [(12, at("/b/c"))]
        );
        assert_eq!(
            obs(&["git commit -m \"x; cd /tmp/r\" && gh pr merge 5"]),
            [(5, Place::Start)]
        );
        assert_eq!(
            obs(&["cd /a && cd sub && gh pr merge 6"]),
            [(6, at("/a/sub"))]
        );
        assert_eq!(obs(&["cd $WT && gh pr merge 7"]), [(7, Place::Unknown)]);
        assert_eq!(
            obs(&["bash -lc 'cd /x && gh pr merge 8'", "gh pr merge 9"]),
            [(8, at("/x")), (9, Place::Unknown)]
        );
        assert_eq!(obs(&["gh pr merge 10 -R o/r"]), []);
    }

    #[test]
    fn a_turns_merged_and_opened_pull_requests_are_on_its_receipt() {
        let dir = tempdir().unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-prs");
        let bash = |cmd: &str| json!({ "command": cmd });
        c.record_tool_use(
            "t1",
            "Bash",
            &bash("gh pr create --title C8 --body-file /tmp/b"),
        );
        c.record_tool_result(
            "t1",
            "https://github.com/example-org/hollow-code/pull/453\n",
            false,
        );
        c.record_tool_use(
            "t2",
            "Bash",
            &bash("gh pr merge 453 --merge --subject \"Merge #453\""),
        );
        c.record_tool_result("t2", "", false);
        c.record_tool_use("t3", "Bash", &bash("gh pr merge 999 -R o/r --merge"));
        c.record_tool_result("t3", "GraphQL: not mergeable", true);
        c.record_tool_use("t4", "Bash", &bash("gh pr create -R o/r --title next"));
        c.record_tool_result("t4", "https://github.com/o/r/pull/12\n", false);
        c.record_tool_use("t5", "Bash", &bash("gh pr merge 13 -R o/r --merge"));
        // The URL after long output (the gate, a push): read before the excerpt cap.
        c.record_tool_use(
            "t6",
            "Bash",
            &bash("npm run gate && git push && gh pr create -R o/r --title big"),
        );
        c.record_tool_result(
            "t6",
            &format!("{}\nhttps://github.com/o/r/pull/14\n", "x".repeat(2_000)),
            false,
        );
        // A GitHub Enterprise `HOST/OWNER/REPO` never becomes a github.com link.
        c.record_tool_use(
            "t7",
            "Bash",
            &bash("gh pr merge 15 -R ghe.corp/o/r --merge"),
        );
        c.record_tool_result("t7", "", false);
        let r = c.finish(dir.path(), "success", vec![]).unwrap();
        let rows: Vec<(u64, &str, Option<&str>)> = r
            .pull_requests
            .iter()
            .map(|p| (p.number, p.action.as_str(), p.url.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    453,
                    "merged",
                    Some("https://github.com/example-org/hollow-code/pull/453")
                ),
                (12, "opened", Some("https://github.com/o/r/pull/12")),
                (14, "opened", Some("https://github.com/o/r/pull/14")),
                (15, "merged", None),
            ],
            "opened then merged is one merged row; an error or an unanswered call is none"
        );
        // Serialized only when there are some.
        let quiet = EvidenceCollector::new("chat", None, "run-quiet")
            .finish(dir.path(), "success", vec![])
            .unwrap();
        assert!(!serde_json::to_string(&quiet)
            .unwrap()
            .contains("pullRequests"));
    }

    /// Overnight A3: a turn that ended with its background work still
    /// running says so; one whose background work had finished (nothing
    /// left at the exit) or that started none doesn't; a wakeup tool is
    /// named once however often it was called.
    #[test]
    fn a_turn_that_ended_mid_work_says_so() {
        let dir = tempdir().unwrap();
        let background = |leftovers: usize| {
            let mut c = EvidenceCollector::new("chat", None, "run-bg");
            c.record_tool_use(
                "t1",
                "Bash",
                &json!({ "command": "npm test", "run_in_background": true }),
            );
            c.record_tool_result("t1", "Command running in background with ID: b1", false);
            c.set_leftovers(leftovers);
            c.finish(dir.path(), "success", vec![]).unwrap()
        };
        let cut = background(2);
        // The backgrounded `npm test` returned at once: never "passed".
        assert_eq!(cut.checks.len(), 1);
        assert_eq!(cut.checks[0].outcome, "unknown");
        let row = cut
            .failures
            .iter()
            .find(|f| f.label == "background")
            .expect("a background row");
        assert!(row.excerpt.contains("(2 processes)"), "{}", row.excerpt);
        assert!(!background(0)
            .failures
            .iter()
            .any(|f| f.label == "background"));

        // The real stream (a `claude -p` probe, 2026-10-10): the CLI stops
        // its own background task as it exits and says so.
        let line = r#"{"type":"system","subtype":"task_notification","task_id":"b8vhcq4xn","tool_use_id":"toolu_1","status":"stopped","output_file":"/tmp/x.output","summary":"/bin/sleep 97 ; echo probe-a3-done","uuid":"u","session_id":"s"}"#;
        let task = stopped_background_task(line).expect("a stopped task");
        assert_eq!(task.1, "/bin/sleep 97 ; echo probe-a3-done");
        assert_eq!(
            stopped_background_task(&line.replace("\"stopped\"", "\"completed\"")),
            None,
            "a task that ran out is not cut off"
        );
        assert_eq!(
            stopped_background_task(r#"{"type":"system","subtype":"init"}"#),
            None
        );
        // Only after the attempt's result, and only once it exited on its
        // own: a task the agent stopped mid-turn, or Hollow's own stop, is
        // not a cut-off.
        let result = r#"{"type":"result","subtype":"success","result":"started"}"#;
        let cut = |lines: &[&str], exited: bool| {
            let mut tasks = CutTasks::default();
            for l in lines {
                tasks.observe(l);
            }
            let mut c = EvidenceCollector::new("chat", None, "run-bg-cli");
            tasks.commit(exited, &mut c);
            c.finish(dir.path(), "success", vec![]).unwrap()
        };
        let r = cut(&[result, line, line], true);
        let rows: Vec<&ReceiptFailure> = r
            .failures
            .iter()
            .filter(|f| f.label == "background")
            .collect();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].excerpt.contains("/bin/sleep 97"),
            "{}",
            rows[0].excerpt
        );
        assert!(
            !rows[0].excerpt.contains("more"),
            "one task, reported twice: {}",
            rows[0].excerpt
        );
        assert!(
            cut(&[line, result], true).failures.is_empty(),
            "stopped mid-turn"
        );
        assert!(
            cut(&[result, line], false).failures.is_empty(),
            "Hollow's own stop"
        );

        let mut plain = EvidenceCollector::new("chat", None, "run-plain");
        plain.record_tool_use("t1", "Bash", &json!({ "command": "ls" }));
        plain.record_tool_result("t1", "ok", false);
        plain.set_leftovers(3);
        let r = plain.finish(dir.path(), "success", vec![]).unwrap();
        assert!(
            r.failures.is_empty(),
            "leftovers without background work are not this: {:?}",
            r.failures
        );

        let mut failed = EvidenceCollector::new("chat", None, "run-wake-failed");
        failed.record_tool_use("w0", "ScheduleWakeup", &json!({ "delaySeconds": 60 }));
        failed.record_tool_result("w0", "No such tool available: ScheduleWakeup", true);
        failed.record_tool_use("w1", "CronCreate", &json!({ "cron": "* * * * *" }));
        let r = failed.finish(dir.path(), "success", vec![]).unwrap();
        assert!(
            !r.failures.iter().any(|f| f.label == "wakeup"),
            "nothing was scheduled: one failed, one never answered"
        );

        let mut wake = EvidenceCollector::new("chat", None, "run-wake");
        for id in ["w1", "w2"] {
            wake.record_tool_use(id, "ScheduleWakeup", &json!({ "delaySeconds": 1500 }));
            wake.record_tool_result(id, "Next wakeup scheduled", false);
        }
        let r = wake.finish(dir.path(), "success", vec![]).unwrap();
        let rows: Vec<&ReceiptFailure> =
            r.failures.iter().filter(|f| f.label == "wakeup").collect();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].excerpt.contains("(ScheduleWakeup)"),
            "{}",
            rows[0].excerpt
        );
    }

    /// Run 15 S2 (review): an early-started agent ended unused is a step of
    /// the run, so its cleanup combines with the attempt's: one unconfirmed
    /// step makes the run unconfirmed, in either order.
    #[test]
    fn process_stops_combine_and_one_unconfirmed_step_wins() {
        use crate::process_group::StopOutcome;
        let dir = tempdir().unwrap();
        for (first, second, want) in [
            (None, Some(StopOutcome::Verified), Some("verified")),
            (Some(StopOutcome::Verified), None, Some("verified")),
            (
                Some(StopOutcome::Unconfirmed),
                Some(StopOutcome::Verified),
                Some("unconfirmed"),
            ),
            (
                Some(StopOutcome::Verified),
                Some(StopOutcome::Unconfirmed),
                Some("unconfirmed"),
            ),
            (None, None, None),
        ] {
            let mut run = EvidenceCollector::new("chat", None, "run-stop-steps");
            run.add_process_stop(first);
            run.add_process_stop(second);
            let r = run.finish(dir.path(), "cancelled", vec![]).unwrap();
            assert_eq!(r.process_stop.as_deref(), want, "{first:?} then {second:?}");
        }
        for outcome in [StopOutcome::Verified, StopOutcome::Unconfirmed] {
            assert_eq!(StopOutcome::parse(outcome.as_str()), Some(outcome));
        }
        let mut left = EvidenceCollector::new("chat", None, "leftovers");
        left.set_process_stop(Some(StopOutcome::Verified));
        left.set_leftovers(1);
        assert_eq!(StopOutcome::parse("other"), None);
    }

    #[test]
    fn receipt_records_process_stop_and_surfaces_unconfirmed() {
        use crate::process_group::StopOutcome;
        let dir = tempdir().unwrap();

        let mut stopped = EvidenceCollector::new("chat", None, "run-stop-verified");
        stopped.set_process_stop(Some(StopOutcome::Verified));
        let r = stopped.finish(dir.path(), "cancelled", vec![]).unwrap();
        assert_eq!(r.process_stop.as_deref(), Some("verified"));
        assert!(r
            .failures
            .iter()
            .any(|f| f.label == "run" && f.excerpt.contains("Stopped (verified)")));
        assert!(!r.failures.iter().any(|f| f.label == "stop"));

        let mut finished = EvidenceCollector::new("workflow", Some("daily"), "run-stop-cleanup");
        finished.record_tool_use("t", "Bash", &json!({ "command": "npm run dev &" }));
        finished.record_tool_result("t", "started", false);
        finished.set_process_stop(Some(StopOutcome::Unconfirmed));
        let r = finished.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(r.process_stop.as_deref(), Some("unconfirmed"));
        let stop = r
            .failures
            .iter()
            .find(|f| f.label == "stop")
            .expect("a stop failure");
        assert!(stop.excerpt.starts_with("Cleanup unconfirmed"));
        assert_eq!(stop.sequence, 3, "sequenced after the last observation");

        let mut failed = EvidenceCollector::new("heartbeat", None, "run-stop-error");
        failed.set_process_stop(Some(StopOutcome::Unconfirmed));
        let r = failed.finish(dir.path(), "error", vec![]).unwrap();
        assert!(r
            .failures
            .iter()
            .any(|f| f.label == "stop" && f.excerpt.starts_with("Stop unconfirmed")));
        assert_eq!(r.failures.last().map(|f| f.label.as_str()), Some("run"));

        let r = EvidenceCollector::new("chat", None, "run-stop-none")
            .finish(dir.path(), "success", vec![])
            .unwrap();
        assert!(r.process_stop.is_none());
        assert!(r.failures.is_empty());

        let legacy = r#"{"id":"receipt-x","runId":"x","source":"chat","sourceId":null,"startedAt":"a","finishedAt":"b","status":"success","verificationState":"noChanges","files":[],"checks":[],"failures":[],"historyRunId":"x"}"#;
        let parsed: EvidenceReceipt = serde_json::from_str(legacy).unwrap();
        assert!(parsed.process_stop.is_none());
    }

    /// Paperclip PR 14a: the run's result kind is recorded, and a failure
    /// kind other than a cancel or timeout adds one `result` line.
    #[test]
    fn receipt_records_result_kind_and_failure_line() {
        use crate::result_kind::ResultKind;
        let dir = tempdir().unwrap();

        let mut quota = EvidenceCollector::new("workflow", Some("brief"), "run-kind-quota");
        quota.set_result_kind(ResultKind::ProviderQuota);
        let r = quota.finish(dir.path(), "error", vec![]).unwrap();
        assert_eq!(r.result_kind.as_deref(), Some("providerQuota"));
        let line = r
            .failures
            .iter()
            .find(|f| f.label == "result")
            .expect("a result line");
        assert_eq!(line.excerpt, "Usage limit reached.");
        assert_eq!(
            r.failures.last().map(|f| f.label.as_str()),
            Some("run"),
            "the run line stays last"
        );

        let mut ok = EvidenceCollector::new("chat", None, "run-kind-ok");
        ok.set_result_kind(ResultKind::Success);
        let r = ok.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(r.result_kind.as_deref(), Some("success"));
        assert!(r.failures.is_empty());

        for kind in [ResultKind::Cancelled, ResultKind::TimedOut] {
            let mut own =
                EvidenceCollector::new("chat", None, &format!("run-kind-{}", kind.as_str()));
            own.set_result_kind(kind);
            let r = own.finish(dir.path(), "cancelled", vec![]).unwrap();
            assert!(!r.failures.iter().any(|f| f.label == "result"), "{kind:?}");
        }

        // A receipt written before the field parses, without a kind.
        let legacy = r#"{"id":"receipt-x","runId":"x","source":"chat","sourceId":null,"startedAt":"a","finishedAt":"b","status":"success","verificationState":"noChanges","files":[],"checks":[],"failures":[],"historyRunId":"x"}"#;
        let parsed: EvidenceReceipt = serde_json::from_str(legacy).unwrap();
        assert!(parsed.result_kind.is_none());
        // On the wire the field is always present (null when unset).
        let wire = serde_json::to_value(&parsed).unwrap();
        assert!(wire.get("resultKind").is_some());
    }

    /// OF18 (red first): a receipt counted only failures, so failures per
    /// call couldn't be read. It counts every observed call, a failed or
    /// unfinished one too. An older receipt, and an agent that reports no
    /// tool calls (Antigravity), leave it out: unknown, not 0 (OF18 review).
    #[test]
    fn receipt_counts_its_tool_calls() {
        let dir = tempdir().unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-tool-calls");
        c.record_tool_use("a", "Bash", &json!({ "command": "rg foo src" }));
        c.record_tool_result("a", "Exit code 1 (no match)", false);
        c.record_tool_use("b", "Bash", &json!({ "command": "cargo test" }));
        c.record_tool_result("b", "Exit code 101\nerror", true);
        c.record_tool_use("c", "Read", &json!({ "file_path": "/x/a.md" }));
        let r = c.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(r.tool_calls, Some(3));
        assert_eq!(r.failures.len(), 1);
        assert_eq!(serde_json::to_value(&r).unwrap()["toolCalls"], json!(3));

        let none = EvidenceCollector::new("chat", None, "run-no-tool-calls")
            .finish(dir.path(), "success", vec![])
            .unwrap();
        assert_eq!(none.tool_calls, Some(0));

        let mut c = EvidenceCollector::new("chat", None, "run-unreported-calls");
        c.mark_tool_calls_unreported();
        let unreported = c.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(unreported.tool_calls, None);
        assert!(serde_json::to_value(&unreported)
            .unwrap()
            .get("toolCalls")
            .is_none());

        let legacy = r#"{"id":"receipt-x","runId":"x","source":"chat","sourceId":null,"startedAt":"a","finishedAt":"b","status":"success","verificationState":"noChanges","files":[],"checks":[],"failures":[],"historyRunId":"x"}"#;
        let parsed: EvidenceReceipt = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.tool_calls, None);
    }

    /// OB05: what the run's usage row recorded (`RunUsage::calls`) lands on
    /// its receipt as `apiUsage`, survives the store, and is absent when the
    /// agent supplied none and on older receipts.
    #[test]
    fn receipt_carries_the_runs_api_usage() {
        use crate::agent::{parse_line, AgentKind, ParseState};
        use crate::usage_ledger::{Meter, Surface, UsageRun};
        let dir = tempdir().unwrap();
        let v = dir.path();
        let meter = Meter::for_vault(v, Surface::Chat, None, Some("run-api-usage".into()));
        let mut run = UsageRun::begin(meter, Some(AgentKind::Claude), None);
        let mut state = ParseState::default();
        for line in crate::usage::tests::CHAT_TOOL.lines() {
            for evt in parse_line(AgentKind::Claude, line, &mut state) {
                run.observe(&evt);
            }
        }
        let usage = run.finish(crate::usage::LedgerOutcome::Success);
        let mut c = EvidenceCollector::new("chat", None, "run-api-usage");
        c.set_api_usage(usage.calls);
        let r = c.finish(v, "success", vec![]).unwrap();
        let api = r.api_usage.clone().expect("the run's calls");
        assert_eq!(
            (api.api_calls, api.peak_context_tokens),
            (Some(2), Some(30_318))
        );
        assert_eq!(
            serde_json::to_value(&r).unwrap()["apiUsage"],
            json!({
                "apiCalls": 2,
                "peakContextTokens": 30_318,
                "cacheWriteTokens": 18_115,
                "apiMs": 4_739,
                "numTurns": 2
            })
        );
        let stored = list(v, Some("chat"), None, 10).unwrap();
        let stored = stored.iter().find(|s| s.run_id == "run-api-usage");
        assert_eq!(stored.unwrap().api_usage, r.api_usage, "kept by the store");

        // An agent that supplied none (Codex, Antigravity): absent.
        let mut c = EvidenceCollector::new("chat", None, "run-no-api-usage");
        c.set_api_usage(crate::usage::CallStats::default());
        let none = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(none.api_usage, None);
        assert!(serde_json::to_value(&none)
            .unwrap()
            .get("apiUsage")
            .is_none());
        let legacy = r#"{"id":"receipt-x","runId":"x","source":"chat","sourceId":null,"startedAt":"a","finishedAt":"b","status":"success","verificationState":"noChanges","files":[],"checks":[],"failures":[],"historyRunId":"x"}"#;
        let parsed: EvidenceReceipt = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.api_usage, None);
    }

    /// OB22: a stream with one `compact_boundary` puts `compactions: 1` on
    /// the receipt and in the run manifest's capabilities; a run that never
    /// compacted, and an older receipt, carry no key.
    #[test]
    fn receipt_and_manifest_count_the_runs_compactions() {
        use crate::agent::{parse_line, AgentKind, ParseState};
        let dir = tempdir().unwrap();
        let v = dir.path();
        let mut state = ParseState::default();
        for line in crate::usage::tests::COMPACTED.lines() {
            parse_line(AgentKind::Claude, line, &mut state);
        }
        assert_eq!(state.compactions, 1);
        let mut c = EvidenceCollector::new("chat", None, "run-compacted");
        c.set_session_key("s-compacted");
        c.context_mut().layer("request", "r");
        c.set_compactions(state.compactions);
        let r = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(r.compactions, Some(1));
        assert_eq!(serde_json::to_value(&r).unwrap()["compactions"], 1);
        let view = context_manifest(v, None, Some("s-compacted"))
            .unwrap()
            .expect("the run's manifest");
        assert_eq!(view.manifest.capabilities.compactions, Some(1));

        let mut quiet = EvidenceCollector::new("chat", None, "run-not-compacted");
        quiet.set_compactions(0);
        let none = quiet.finish(v, "success", vec![]).unwrap();
        assert_eq!(none.compactions, None);
        let wire = serde_json::to_value(&none).unwrap();
        assert!(wire.get("compactions").is_none(), "{wire}");
        let legacy = r#"{"id":"receipt-x","runId":"x","source":"chat","sourceId":null,"startedAt":"a","finishedAt":"b","status":"success","verificationState":"noChanges","files":[],"checks":[],"failures":[],"historyRunId":"x"}"#;
        let parsed: EvidenceReceipt = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.compactions, None);
    }

    /// OF18 review (red first): a Codex `npx vitest run > v.log 2>&1 && grep
    /// -q passed v.log` whose vitest failed exits 1 with no output (the
    /// `grep` never ran). Read as a no-match it wasn't an error, and the
    /// vitest check, whose exit only `&&` joins to the end, passed. Now it
    /// stays an error, and the check can't pass on it: it reads `unknown`
    /// (the 1 could as well be the `grep`'s, so it can't read `failed`).
    #[test]
    fn codex_failed_check_before_and_grep_never_passes() {
        let dir = tempdir().unwrap();
        let command =
            "/bin/zsh -lc 'npx vitest run > /tmp/v.log 2>&1 && grep -q passed /tmp/v.log'";
        let (content, is_error) = crate::agent::codex_command_result(command, "", Some(1));
        assert!(is_error, "{content}");
        let mut c = EvidenceCollector::new("chat", None, "run-of18-and-grep");
        c.record_tool_use("w", "Write", &json!({ "file_path": "notes/a.md" }));
        c.record_tool_result("w", "ok", false);
        c.record_tool_use("t", "Bash", &json!({ "command": command }));
        c.record_tool_result("t", &content, is_error);
        let r = c.finish(dir.path(), "success", changes()).unwrap();
        assert_eq!(r.checks.len(), 1);
        assert_eq!(r.checks[0].kind, "test");
        assert_ne!(r.checks[0].outcome, "passed");
        assert_ne!(r.verification_state, "verifiedAfterChange");
    }

    /// Paperclip PR 15c: a receipt carries the phases measured for its run
    /// (always `evidenceMs`), omits what was never measured, and a receipt
    /// written before the field still parses.
    #[test]
    fn receipts_carry_phase_timings_and_old_receipts_still_parse() {
        let dir = tempdir().unwrap();
        let mut c = EvidenceCollector::new("workflow", Some("brief"), "run-timed");
        c.timings_mut().baseline_snapshot_ms = Some(41);
        c.timings_mut().record_drive(
            5_000,
            Some(3_800),
            FirstOutputs {
                any: Some(900),
                stdout: Some(950),
                model: Some(1_000),
                text: Some(1_200),
                tools_before_text: Some(2),
            },
        );
        let r = c.finish(dir.path(), "success", vec![]).unwrap();
        let timings = r.phase_timings.clone().expect("timings on the receipt");
        assert_eq!(timings.baseline_snapshot_ms, Some(41));
        assert_eq!(
            timings.spawn_ms,
            Some(1_200),
            "admitted 5 s ago, spawned 3.8 s ago"
        );
        assert_eq!(timings.turn_ms, Some(3_800));
        assert_eq!(timings.first_output_ms, Some(900));
        // F11: the stream's first stdout line and first assistant text are
        // their own phases, apart from "any line" (stderr counts there).
        assert_eq!(timings.first_stdout_ms, Some(950));
        assert_eq!(timings.first_text_ms, Some(1_200));
        // OB08a: the first model event and the tool calls before the text.
        assert_eq!(timings.first_model_ms, Some(1_000));
        assert_eq!(timings.tools_before_text, Some(2));
        assert!(
            timings.evidence_ms.is_some(),
            "every finish measures itself"
        );
        assert_eq!(timings.queue_wait_ms, None);
        let wire = serde_json::to_value(&r).unwrap();
        let wire_timings = wire["phaseTimings"].as_object().expect("camelCase key");
        assert_eq!(wire_timings["spawnMs"], json!(1_200));
        assert_eq!(wire_timings["firstStdoutMs"], json!(950));
        assert_eq!(wire_timings["firstModelMs"], json!(1_000));
        assert_eq!(wire_timings["firstTextMs"], json!(1_200));
        assert_eq!(wire_timings["toolsBeforeText"], json!(2));
        assert!(wire_timings.contains_key("evidenceMs"));
        for unmeasured in ["queueWaitMs", "repoCaptureMs", "finalizeDiffMs"] {
            assert!(
                !wire_timings.contains_key(unmeasured),
                "{unmeasured} is omitted, never 0"
            );
        }
        // Persisted with the receipt.
        let stored = find_by_history_run(dir.path(), "run-timed")
            .unwrap()
            .unwrap();
        assert_eq!(stored.phase_timings, r.phase_timings);

        // A run that never spawned records no agent phases.
        let mut unstarted = PhaseTimings::default();
        unstarted.record_drive(5_000, None, FirstOutputs::default());
        assert!(unstarted.is_empty());
        assert!(unstarted.to_map().is_empty());

        // A legacy receipt parses without timings, and an empty set is omitted.
        let legacy = r#"{"id":"receipt-x","runId":"x","source":"chat","sourceId":null,"startedAt":"a","finishedAt":"b","status":"success","verificationState":"noChanges","files":[],"checks":[],"failures":[],"historyRunId":"x"}"#;
        let parsed: EvidenceReceipt = serde_json::from_str(legacy).unwrap();
        assert!(parsed.phase_timings.is_none());
        assert!(serde_json::to_value(&parsed)
            .unwrap()
            .get("phaseTimings")
            .is_none());
    }

    /// OB08a: what an agent can't supply stays absent, never 0: an
    /// Antigravity run has neither a first model event (it would only repeat
    /// its first text) nor a tool count, a run with no model event has
    /// neither new field, and timings written before them still parse.
    #[test]
    fn model_timings_an_agent_cannot_supply_are_omitted() {
        use crate::agent::AgentKind;
        let first = FirstOutputs {
            stdout: Some(400),
            model: Some(2_000),
            text: Some(2_000),
            tools_before_text: Some(0),
            ..FirstOutputs::default()
        };
        let agy = first.for_agent(AgentKind::Antigravity);
        assert_eq!((agy.model, agy.tools_before_text), (None, None));
        assert_eq!((agy.stdout, agy.text), (Some(400), Some(2_000)), "kept");
        let mut agy_timings = PhaseTimings::default();
        agy_timings.record_drive(5_000, Some(4_000), agy);
        let agy_map = agy_timings.to_map();
        assert!(agy_map.contains_key("firstTextMs"));
        for absent in ["firstModelMs", "toolsBeforeText"] {
            assert!(!agy_map.contains_key(absent), "Antigravity: no {absent}");
        }
        for kind in [AgentKind::Claude, AgentKind::Codex, AgentKind::Grok] {
            assert_eq!(first.for_agent(kind), first, "{kind:?}");
        }
        let mut timings = PhaseTimings::default();
        timings.record_drive(
            5_000,
            Some(4_000),
            FirstOutputs {
                stdout: Some(400),
                ..FirstOutputs::default()
            },
        );
        let map = timings.to_map();
        assert!(map.contains_key("firstStdoutMs"));
        for absent in ["firstModelMs", "firstTextMs", "toolsBeforeText"] {
            assert!(!map.contains_key(absent), "{absent} is omitted, never 0");
        }
        let old = r#"{"spawnMs":1,"firstTextMs":2}"#;
        let old: PhaseTimings = serde_json::from_str(old).unwrap();
        assert_eq!((old.first_model_ms, old.tools_before_text), (None, None));
    }

    /// Paperclip PR 15c: `finish_mirrored` copies the queue row's wait onto
    /// the receipt and merges the rest into the row, keeping its wait; a run
    /// no queue row ran still gets its receipt.
    #[test]
    fn finish_mirrored_copies_the_queue_wait_and_mirrors_the_rest() {
        use crate::run_queue::{self, NewRow, RunSource};
        let dir = tempdir().unwrap();
        let v = dir.path();
        let requested = Utc::now() - chrono::Duration::seconds(3);
        let id = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("brief"), "Brief", "run now"),
            requested,
        )
        .unwrap()
        .id();
        run_queue::claim(v, id, requested)
            .unwrap()
            .expect("claimed");
        assert!(run_queue::mark_running(v, id, "run-mirrored", Utc::now()).unwrap());
        let wait = run_queue::queue_wait_for_run(v, "run-mirrored")
            .unwrap()
            .expect("a wait");
        assert!(wait >= 2_500, "request → running: {wait}");

        let mut c = EvidenceCollector::new("workflow", Some("brief"), "run-mirrored");
        c.timings_mut().turn_ms = Some(700);
        c.timings_mut().finalize_diff_ms = Some(12);
        let r = c.finish_mirrored(v, "success", vec![]).unwrap();
        let timings = r.phase_timings.expect("timings");
        assert_eq!(timings.queue_wait_ms, Some(wait), "copied from the row");
        assert_eq!(timings.turn_ms, Some(700));

        let row = run_queue::get(v, id).unwrap().expect("the row");
        let mirrored = row.phase_timings.expect("mirrored");
        assert_eq!(
            mirrored["queueWaitMs"].as_u64(),
            Some(wait),
            "the row keeps its own wait"
        );
        assert_eq!(mirrored["turnMs"], json!(700));
        assert_eq!(mirrored["finalizeDiffMs"], json!(12));
        assert!(mirrored.get("evidenceMs").is_some());

        // Outside the queue (tests, the heartbeat's Run now): receipt only.
        let r = EvidenceCollector::new("heartbeat", None, "run-unqueued")
            .finish_mirrored(v, "success", vec![])
            .unwrap();
        let timings = r.phase_timings.expect("timings");
        assert_eq!(timings.queue_wait_ms, None);
        assert!(timings.evidence_ms.is_some());
    }

    #[test]
    fn persistence_is_newest_first_and_filterable() {
        let dir = tempdir().unwrap();
        for (run, source) in [("one", "chat"), ("two", "workflow")] {
            EvidenceCollector::new(source, Some("same"), run)
                .finish(dir.path(), "success", vec![])
                .unwrap();
        }
        assert_eq!(list(dir.path(), Some("chat"), None, 10).unwrap().len(), 1);
        assert_eq!(list(dir.path(), None, Some("same"), 10).unwrap().len(), 2);
    }

    fn write(c: &mut EvidenceCollector, id: &str, path: &str) {
        c.record_tool_use(id, "Write", &json!({ "file_path": path }));
        c.record_tool_result(id, "ok", false);
    }

    fn check(c: &mut EvidenceCollector, id: &str, command: &str) {
        c.record_tool_use(id, "Bash", &json!({ "command": command }));
        c.record_tool_result(id, "passed", false);
    }

    /// Paperclip PR 16 acceptance: a run that changed only repository files
    /// is never `noChanges`, and repo writes confirm only repo rows.
    #[test]
    fn repo_only_run_is_never_no_changes() {
        use crate::repo_diff::test_repo;
        let dir = tempdir().unwrap();
        let v = dir.path();

        // An observed repo file (no structured write named it).
        let mut c = EvidenceCollector::new("chat", None, "repo-observed");
        c.set_repo_changes(vec![test_repo("app", &["src/lib.rs"])]);
        let r = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(r.verification_state, "unknownOrder");
        assert_eq!(r.repo_changes[0].files[0].attribution, "observed");

        // An absolute Write under an alias root, then a passing check.
        let mut c = EvidenceCollector::new("chat", None, "repo-verified");
        write(&mut c, "w", "/work/app/src/lib.rs");
        check(&mut c, "t", "cargo test");
        c.set_repo_changes(vec![test_repo("app", &["src/lib.rs"])]);
        let r = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(r.verification_state, "verifiedAfterChange");
        assert_eq!(r.repo_changes[0].files[0].attribution, "confirmed");
        assert_eq!(r.repo_changes[0].files[0].tool.as_deref(), Some("Write"));

        // A path relative to the run's folder inside the repository.
        let mut c = EvidenceCollector::new("chat", None, "repo-relative");
        write(&mut c, "w", "./lib.rs");
        check(&mut c, "t", "cargo check");
        let mut repo = test_repo("app", &["src/lib.rs"]);
        repo.cwd_in_repo = Some("src/".into());
        c.set_repo_changes(vec![repo]);
        let r = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(r.repo_changes[0].files[0].attribution, "confirmed");
        assert_eq!(r.verification_state, "verifiedAfterChange");

        // An unavailable repository: unknown, plus a `repo` failure row.
        let mut c = EvidenceCollector::new("chat", None, "repo-unavailable");
        let mut repo = test_repo("app", &[]);
        repo.status = "unavailable".into();
        repo.error = Some("git timed out".into());
        c.set_repo_changes(vec![repo]);
        let r = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(r.verification_state, "unknownOrder");
        let failure = r
            .failures
            .iter()
            .find(|f| f.label == "repo")
            .expect("a repo failure");
        assert!(failure
            .excerpt
            .contains("Repository changes in app could not be recorded: git timed out"));

        // Rows past the listed ones.
        let mut c = EvidenceCollector::new("chat", None, "repo-unlisted");
        write(&mut c, "w", "/work/app/a.rs");
        check(&mut c, "t", "cargo test");
        let mut repo = test_repo("app", &["a.rs"]);
        repo.total_files = 5;
        c.set_repo_changes(vec![repo]);
        let r = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(r.verification_state, "unknownOrder");

        // A captured repository with nothing changed is still `noChanges`.
        let mut c = EvidenceCollector::new("chat", None, "repo-empty");
        c.set_repo_changes(vec![test_repo("app", &[])]);
        let r = c.finish(v, "success", vec![]).unwrap();
        assert_eq!(r.verification_state, "noChanges");

        // A repo write to `/work/app/README.md` never confirms the vault's
        // own `README.md`, and an absolute path outside the vault never
        // confirms a vault change either.
        let mut c = EvidenceCollector::new("chat", None, "repo-suffix");
        write(&mut c, "w", "/work/app/README.md");
        write(&mut c, "w2", "/elsewhere/notes/a.md");
        c.set_repo_changes(vec![test_repo("app", &["README.md"])]);
        let r = c
            .finish(
                v,
                "success",
                vec![
                    ObservedChange {
                        path: "README.md".into(),
                        change_type: "modify".into(),
                        ..Default::default()
                    },
                    ObservedChange {
                        path: "notes/a.md".into(),
                        change_type: "modify".into(),
                        ..Default::default()
                    },
                ],
            )
            .unwrap();
        assert_eq!(r.repo_changes[0].files[0].attribution, "confirmed");
        assert!(
            r.files.iter().all(|f| f.attribution == "observed"),
            "{:?}",
            r.files
        );
    }

    #[test]
    fn collector_repo_seam_survives_clone_and_finish() {
        use crate::repo_diff::{test_baseline, test_repo};
        use std::sync::{Arc, Mutex};
        let dir = tempdir().unwrap();
        let shared = Arc::new(Mutex::new(EvidenceCollector::new(
            "workflow",
            Some("w"),
            "repo-seam",
        )));

        // Not armed (the agent never spawned): nothing to capture.
        shared.lock().unwrap().set_repo_baseline(test_baseline());
        assert!(shared.lock().unwrap().take_repo_baseline().is_none());
        // An empty baseline is dropped outright.
        shared
            .lock()
            .unwrap()
            .set_repo_baseline(crate::repo_diff::RepoBaseline::default());
        shared.lock().unwrap().arm_repo_baseline();
        assert!(shared.lock().unwrap().take_repo_baseline().is_none());

        shared.lock().unwrap().set_repo_baseline(test_baseline());
        shared.lock().unwrap().arm_repo_baseline();
        assert!(shared.lock().unwrap().take_repo_baseline().is_some());
        assert!(
            shared.lock().unwrap().take_repo_baseline().is_none(),
            "taken once"
        );

        let paths: Vec<String> = (0..60).map(|n| format!("f{n:02}.rs")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut repo = test_repo("app", &refs);
        repo.total_files = 75;
        repo.files[0].patch = Some("diff --git a/f00.rs b/f00.rs".into());
        shared.lock().unwrap().set_repo_changes(vec![repo]);
        let collector = shared.lock().unwrap().clone();
        let receipt = collector.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(receipt.repo_changes.len(), 1);
        assert_eq!(receipt.repo_changes[0].files.len(), RECEIPT_REPO_FILES);
        assert_eq!(receipt.repo_changes[0].total_files, 75);
        assert!(receipt.repo_changes[0]
            .files
            .iter()
            .all(|f| f.patch.is_none()));

        let stored = list(dir.path(), None, None, 10).unwrap();
        assert_eq!(stored[0].repo_changes[0].total_files, 75);
        let raw = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(raw.contains("repoChanges"));
        assert!(!raw.contains("aliasRoots") && !raw.contains("vaultPrefix"));

        let legacy = r#"{"id":"receipt-x","runId":"x","source":"chat","sourceId":null,"startedAt":"a","finishedAt":"b","status":"success","verificationState":"noChanges","files":[],"checks":[],"failures":[],"historyRunId":"x"}"#;
        let parsed: EvidenceReceipt = serde_json::from_str(legacy).unwrap();
        assert!(parsed.repo_changes.is_empty());
        assert!(!serde_json::to_string(&parsed)
            .unwrap()
            .contains("repoChanges"));
    }

    /// Paperclip PR 9b: one receipt per history run, and a duplicate (a
    /// forged copy in the agent-writable store) is trusted for neither.
    #[test]
    fn find_by_history_run_returns_the_one_receipt_and_refuses_duplicates() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        assert_eq!(
            find_by_history_run(v, "run-a").unwrap(),
            None,
            "no store yet"
        );
        let mut collector = EvidenceCollector::new("workflow", Some("brief"), "run-a");
        collector.record_tool_use("t1", "Write", &json!({"file_path": "outputs/a.md"}));
        collector.record_tool_result("t1", "ok", false);
        collector
            .finish(
                v,
                "success",
                vec![ObservedChange {
                    path: "outputs/a.md".into(),
                    change_type: "create".into(),
                    ..Default::default()
                }],
            )
            .unwrap();
        EvidenceCollector::new("chat", None, "run-b")
            .finish(v, "success", vec![])
            .unwrap();

        let found = find_by_history_run(v, "run-a")
            .unwrap()
            .expect("run-a's receipt");
        assert_eq!(found.source_id.as_deref(), Some("brief"));
        assert_eq!(found.files[0].attribution, "confirmed");
        assert_eq!(find_by_history_run(v, "run-missing").unwrap(), None);
        assert_eq!(find_by_history_run(v, "").unwrap(), None);

        // A second receipt claiming the same history run.
        let mut store = read_store(v).unwrap();
        let mut forged = found.clone();
        forged.id = "receipt-forged".into();
        store.receipts.push(forged);
        fs::write(v.join(FILE_NAME), serde_json::to_vec(&store).unwrap()).unwrap();
        assert_eq!(find_by_history_run(v, "run-a").unwrap(), None);
    }

    /// Merged plan slice 4 (decision 5): a receipt never claims a run is
    /// fully recoverable when History holds no copy of one of its files.
    /// Rewriting an oversized file used to leave it off History and the
    /// receipt both.
    #[test]
    fn a_receipt_with_an_uncaptured_file_is_not_recoverable() {
        use crate::automated_history::{begin_run, finish_run};
        let dir = tempdir().unwrap();
        let v = dir.path();
        let big = 2 * 1024 * 1024 + 1;
        fs::create_dir_all(v.join("notes")).unwrap();
        fs::write(v.join("notes/a.md"), "one").unwrap();
        fs::write(v.join("notes/data.bin"), vec![b'x'; big]).unwrap();

        let receipt = |run: crate::automated_history::HistoryRun| {
            let changes = run
                .changes
                .into_iter()
                .map(ObservedChange::from_history)
                .collect();
            EvidenceCollector::new("chat", None, &run.id)
                .finish(v, "success", changes)
                .unwrap()
        };

        let handle = begin_run(v, "chat", None).unwrap();
        fs::write(v.join("notes/a.md"), "two").unwrap();
        let clean = receipt(finish_run(v, handle, "success").unwrap());
        assert_eq!(clean.history_recoverable, Some(true));
        assert!(clean.files.iter().all(|file| !file.not_recoverable));

        let handle = begin_run(v, "chat", None).unwrap();
        fs::write(v.join("notes/a.md"), "three").unwrap();
        fs::write(v.join("notes/data.bin"), vec![b'y'; big + 10]).unwrap();
        let mixed = receipt(finish_run(v, handle, "success").unwrap());
        assert_eq!(mixed.history_recoverable, Some(false));
        let flagged: Vec<&str> = mixed
            .files
            .iter()
            .filter(|file| file.not_recoverable)
            .map(|file| file.path.as_str())
            .collect();
        assert_eq!(flagged, ["notes/data.bin"]);

        // A change that didn't come from History claims nothing.
        let fixture = EvidenceCollector::new("chat", None, "run-x")
            .finish(v, "success", changes())
            .unwrap();
        assert_eq!(fixture.history_recoverable, None);
    }

    /// S2 follow-up (red first): F3's "agent rereads are measured". The
    /// manifest counts the run's on-demand reader and vault search calls
    /// (`hollow_vault`'s `read_source` and `search_vault`, as Claude names
    /// them and as Codex's are normalised), zero included when the run had
    /// the server; a run without it and without calls records nothing.
    #[test]
    fn reader_and_search_calls_are_counted_in_the_manifest() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let mut c = EvidenceCollector::new("chat", None, "run-reads");
        c.context_mut().capabilities.owned_servers = vec!["hollow_vault".into()];
        let read = "mcp__hollow_vault__read_source";
        c.record_tool_use("t1", read, &json!({ "path": "sources/a.md" }));
        c.record_tool_result("t1", "a.md · lines 1–40 of 400", false);
        c.record_tool_use("t2", read, &json!({ "path": "me/Identity.md" }));
        c.record_tool_result("t2", "Refused: …", true);
        c.record_tool_use(
            "t3",
            "mcp__hollow_vault__search_vault",
            &json!({ "query": "x" }),
        );
        c.record_tool_use("t4", "Read", &json!({ "file_path": "/x" }));
        // R3: a read that asked by intent is counted as queried too; an
        // empty query is a range read.
        c.record_tool_use(
            "t5",
            read,
            &json!({ "path": "/private/tmp/hollow-run/a/t.log", "query": "E0308" }),
        );
        c.record_tool_result("t5", "t.log · 900 lines · best matches", false);
        c.record_tool_use("t6", read, &json!({ "path": "sources/a.md", "query": " " }));
        c.record_tool_result("t6", "a.md · lines 1–40 of 400", false);
        c.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-reads"), None)
            .unwrap()
            .unwrap();
        let manifest = serde_json::to_value(&view.manifest).unwrap();
        assert_eq!(
            manifest["agentReads"],
            json!({ "readSource": 4, "readSourceFailed": 1, "readSourceQueried": 1, "searchVault": 1 }),
            "{manifest}"
        );
        // The server attached and never called: measured as zero.
        let mut quiet = EvidenceCollector::new("chat", None, "run-quiet");
        quiet.context_mut().capabilities.owned_servers = vec!["hollow_vault".into()];
        quiet.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-quiet"), None)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(&view.manifest).unwrap()["agentReads"],
            json!({ "readSource": 0, "readSourceFailed": 0, "readSourceQueried": 0, "searchVault": 0 })
        );
        // No reader and no calls: nothing to measure.
        let mut none = EvidenceCollector::new("workflow", Some("w"), "run-none");
        none.context_mut().layer("request", "x");
        none.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-none"), None)
            .unwrap()
            .unwrap();
        assert!(serde_json::to_value(&view.manifest).unwrap()["agentReads"].is_null());
    }

    /// S26 (red first): the notes the agent opened outside its folder are
    /// the manifest's `notesOpened`, vault-relative, once each, in order:
    /// two `Read`s outside the cwd are recorded and one inside it is not;
    /// so are a shell read and a `read_source`; a failed read, a file that
    /// isn't a note and one outside the vault are not.
    #[test]
    fn notes_opened_are_the_vault_notes_read_outside_the_cwd() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let cwd = v.join("workspaces/w1");
        for note in [
            "notes/a.md",
            "library/b.md",
            "library/c.md",
            "people/d.md",
            "notes/e.txt",
            "workspaces/w1/own.md",
        ] {
            fs::create_dir_all(v.join(note).parent().unwrap()).unwrap();
            fs::write(v.join(note), "x").unwrap();
        }
        let path = |rel: &str| json!({ "file_path": v.join(rel).to_string_lossy() });
        let mut c = EvidenceCollector::new("chat", None, "run-opened");
        c.context_mut().set_run_cwd(&cwd);
        c.record_tool_use("r1", "Read", &path("notes/a.md"));
        c.record_tool_result("r1", "x", false);
        c.record_tool_use("r2", "Read", &path("workspaces/w1/own.md"));
        c.record_tool_result("r2", "x", false);
        c.record_tool_use("r3", "Read", &path("library/b.md"));
        c.record_tool_result("r3", "x", false);
        c.record_tool_use("r4", "Read", &path("notes/a.md"));
        c.record_tool_use("r5", "Read", &path("notes/missing.md"));
        c.record_tool_result("r5", "File does not exist.", true);
        c.record_tool_use("r6", "Read", &path("notes/e.txt"));
        c.record_tool_use("r7", "Read", &json!({ "file_path": "/elsewhere/x.md" }));
        c.record_tool_use(
            "s1",
            "Bash",
            &json!({ "command": "sed -n 1,20p ../../library/c.md | head -5" }),
        );
        c.record_tool_use(
            "s2",
            crate::run_manifest::READ_SOURCE_TOOL,
            &json!({ "path": "people/d.md" }),
        );
        c.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-opened"), None)
            .unwrap()
            .unwrap();
        let manifest = serde_json::to_value(&view.manifest).unwrap();
        assert_eq!(
            manifest["agentReads"]["notesOpened"],
            json!(["notes/a.md", "library/b.md", "library/c.md", "people/d.md"]),
            "{manifest}"
        );
        // Without a known folder nothing is "outside" it: none recorded.
        let mut bare = EvidenceCollector::new("chat", None, "run-bare");
        bare.context_mut().layer("request", "x");
        bare.record_tool_use("r1", "Read", &path("notes/a.md"));
        bare.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-bare"), None)
            .unwrap()
            .unwrap();
        assert!(serde_json::to_value(&view.manifest).unwrap()["agentReads"].is_null());
    }

    /// S26: a note in a restricted folder is never recorded as opened,
    /// however it was read; and the list stops at 32.
    #[test]
    fn notes_opened_never_record_a_restricted_note_and_stop_at_32() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let cwd = v.join("workspaces/w1");
        for note in ["restricted/pay.md", "clients/Restricted/x.md"] {
            fs::create_dir_all(v.join(note).parent().unwrap()).unwrap();
            fs::write(v.join(note), "x").unwrap();
        }
        let mut c = EvidenceCollector::new("chat", None, "run-restricted");
        c.context_mut().set_run_cwd(&cwd);
        let pay = v.join("restricted/pay.md");
        c.record_tool_use("r1", "Read", &json!({ "file_path": pay.to_string_lossy() }));
        c.record_tool_use(
            "s1",
            "Bash",
            &json!({ "command": format!("cat {}", v.join("clients/Restricted/x.md").display()) }),
        );
        c.record_tool_use(
            "s2",
            crate::run_manifest::READ_SOURCE_TOOL,
            &json!({ "path": "restricted/pay.md" }),
        );
        c.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-restricted"), None)
            .unwrap()
            .unwrap();
        let manifest = serde_json::to_value(&view.manifest).unwrap();
        // The read_source call still counts as a read; no path is recorded.
        assert_eq!(manifest["agentReads"]["readSource"], 1, "{manifest}");
        assert!(
            manifest["agentReads"]["notesOpened"].is_null(),
            "{manifest}"
        );

        let mut many = EvidenceCollector::new("chat", None, "run-many");
        many.context_mut().set_run_cwd(&cwd);
        fs::create_dir_all(v.join("notes")).unwrap();
        for n in 0..40 {
            let note = v.join(format!("notes/n{n:02}.md"));
            fs::write(&note, "x").unwrap();
            let id = format!("r{n}");
            many.record_tool_use(&id, "Read", &json!({ "file_path": note.to_string_lossy() }));
        }
        many.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-many"), None)
            .unwrap()
            .unwrap();
        let opened = serde_json::to_value(&view.manifest).unwrap()["agentReads"]["notesOpened"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(opened, crate::run_manifest::MAX_NOTES_OPENED);
    }

    /// S26 review (red first): a symlinked folder into a restricted one
    /// (`library/shared -> ../restricted`) never shows its notes under the
    /// link's name, and neither does a link out of the vault; a link to an
    /// ordinary folder in the vault still records the note.
    #[cfg(unix)]
    #[test]
    fn notes_opened_never_record_a_restricted_note_through_a_symlink() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let v = dir.path();
        let cwd = v.join("workspaces/w1");
        for note in ["restricted/pay.md", "notes/ok.md"] {
            fs::create_dir_all(v.join(note).parent().unwrap()).unwrap();
            fs::write(v.join(note), "x").unwrap();
        }
        fs::write(outside.path().join("far.md"), "x").unwrap();
        fs::create_dir_all(v.join("library")).unwrap();
        std::os::unix::fs::symlink("../restricted", v.join("library/shared")).unwrap();
        std::os::unix::fs::symlink("../notes", v.join("library/linked")).unwrap();
        std::os::unix::fs::symlink(outside.path(), v.join("library/out")).unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-linked");
        c.context_mut().set_run_cwd(&cwd);
        for (id, rel) in [
            ("r1", "library/shared/pay.md"),
            ("r2", "library/out/far.md"),
            ("r3", "library/linked/ok.md"),
        ] {
            let path = v.join(rel);
            c.record_tool_use(id, "Read", &json!({ "file_path": path.to_string_lossy() }));
            c.record_tool_result(id, "x", false);
        }
        c.record_tool_use(
            "s1",
            "Bash",
            &json!({ "command": format!("cat {}", v.join("library/shared/pay.md").display()) }),
        );
        c.record_tool_use(
            "s2",
            crate::run_manifest::READ_SOURCE_TOOL,
            &json!({ "path": "library/shared/pay.md" }),
        );
        c.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-linked"), None)
            .unwrap()
            .unwrap();
        let manifest = serde_json::to_value(&view.manifest).unwrap();
        assert_eq!(
            manifest["agentReads"]["notesOpened"],
            json!(["library/linked/ok.md"]),
            "{manifest}"
        );
    }

    /// S26 (red first): a reply that names a kept passage's file, by its
    /// name without `.md` (any case) or by its path, marks that candidate
    /// cited; a name under 4 characters, a name inside a longer word and a
    /// dropped candidate are never cited.
    #[test]
    fn a_reply_naming_a_kept_file_marks_it_cited() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let candidate = |path: &str, kept: bool| crate::run_manifest::RetrievalCandidate {
            path: path.into(),
            kept,
            ..Default::default()
        };
        let mut c = EvidenceCollector::new("chat", None, "run-cited");
        c.context_mut().set_retrieval(
            crate::run_manifest::RetrievalRecord {
                engine: "jev".into(),
                candidates: vec![
                    candidate("library/atlas/go-live-decisions.md", true),
                    candidate("notes/faq.md", true),
                    candidate("projects/x/ops.md", true),
                    candidate("notes/review.md", true),
                    candidate("notes/dropped-note.md", false),
                ],
                ..Default::default()
            },
            None,
        );
        c.set_reply_citations(
            "Per **Go-Live-Decisions**, the FAQ holds (see projects/x/ops.md). \
             Reviewed; the dropped-note disagrees.",
        );
        c.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-cited"), None)
            .unwrap()
            .unwrap();
        let manifest = serde_json::to_value(&view.manifest).unwrap();
        let retrieval = &manifest["retrieval"];
        assert_eq!(retrieval["citedChecked"], true, "{manifest}");
        let cited: Vec<bool> = retrieval["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["cited"] == true)
            .collect();
        assert_eq!(cited, [true, false, true, false, false], "{manifest}");
        assert!(!manifest.to_string().contains("holds"), "no reply text");
    }

    /// S26 review (red first): a chat turn's citations come from its final
    /// reply, after its last tool call, never from the narration before it.
    #[test]
    fn citations_read_only_the_final_reply() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let candidate = |path: &str| crate::run_manifest::RetrievalCandidate {
            path: path.into(),
            kept: true,
            ..Default::default()
        };
        let mut c = EvidenceCollector::new("chat", None, "run-final-cited");
        c.context_mut().set_retrieval(
            crate::run_manifest::RetrievalRecord {
                engine: "jev".into(),
                candidates: vec![
                    candidate("notes/weekly-sync.md"),
                    candidate("notes/go-live-decisions.md"),
                ],
                ..Default::default()
            },
            None,
        );
        let early = "Let me open weekly-sync first. ";
        c.mark_reply_start(early.len());
        c.set_final_reply_citations(&format!("{early}Per go-live-decisions, ship Friday."));
        c.finish(v, "success", vec![]).unwrap();
        let view = context_manifest(v, Some("run-final-cited"), None)
            .unwrap()
            .unwrap();
        let manifest = serde_json::to_value(&view.manifest).unwrap();
        let cited: Vec<bool> = manifest["retrieval"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["cited"] == true)
            .collect();
        assert_eq!(cited, [false, true], "{manifest}");
    }

    /// Repo features R5: the receipt checks the final reply's `path:line`
    /// cites against the file and the run's reads: a line it read is seen,
    /// one past the end is reported, an unread file is not opened (unknown
    /// once a subagent ran), and the record never changes the run's state.
    #[test]
    fn the_receipt_checks_the_final_replys_cites() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let cwd = v.join("repo");
        fs::create_dir_all(cwd.join("src")).unwrap();
        let body: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        fs::write(cwd.join("src/a.rs"), &body).unwrap();
        fs::write(cwd.join("src/b.rs"), &body).unwrap();
        let run = |id: &str, delegate: bool| {
            let mut c = EvidenceCollector::new("chat", None, id);
            c.context_mut().set_run_cwd(&cwd);
            c.record_tool_use(
                "r1",
                "Read",
                &json!({ "file_path": cwd.join("src/a.rs").to_string_lossy() }),
            );
            c.record_tool_result("r1", &body, false);
            if delegate {
                c.record_tool_use("t1", "Task", &json!({ "prompt": "look around" }));
                c.record_tool_result("t1", "done", false);
            }
            let early = "Reading src/b.rs:1 first. ";
            c.mark_reply_start(early.len());
            c.set_final_reply_citations(&format!(
                "{early}See src/a.rs:12, src/a.rs:99 and src/b.rs:3.\n```\nsrc/a.rs:500\n```"
            ));
            c.finish(v, "success", vec![]).unwrap()
        };
        let receipt = run("run-cites", false);
        let cites = receipt.cites.clone().expect("the reply cited lines");
        assert_eq!(
            (cites.checked, cites.seen, cites.past_end, cites.not_opened),
            (3, 1, 1, 1),
            "{cites:?}"
        );
        let items: Vec<(String, u32, String)> = cites
            .items
            .iter()
            .map(|i| (i.path.clone(), i.line, i.status.clone()))
            .collect();
        assert_eq!(
            items,
            [
                ("src/a.rs".to_string(), 99, "pastEnd".to_string()),
                ("src/b.rs".to_string(), 3, "notOpened".to_string()),
            ]
        );
        assert_eq!(receipt.status, "success");
        // Stored with the receipt, camelCase.
        let stored = serde_json::to_value(get(v, &receipt.id).unwrap()).unwrap();
        assert_eq!(stored["cites"]["pastEnd"], 1, "{stored}");
        assert!(stored["cites"].get("unknown").is_none());

        let delegated = run("run-cites-task", true).cites.unwrap();
        assert_eq!((delegated.not_opened, delegated.unknown), (0, 1));

        // R5 review: Grok reads with `read_file` (`target_file`) and hands
        // off with `spawn_subagent`; it reports its tool calls.
        let mut grok = EvidenceCollector::new("chat", None, "run-cites-grok");
        grok.context_mut().set_run_cwd(&cwd);
        grok.record_tool_use(
            "g1",
            "read_file",
            &json!({ "target_file": cwd.join("src/a.rs").to_string_lossy() }),
        );
        grok.record_tool_result("g1", "{\"type\":\"ReadFile\"}", false);
        grok.set_final_reply_citations("See src/a.rs:12 and src/b.rs:3.");
        let solo = grok
            .clone()
            .finish(v, "success", vec![])
            .unwrap()
            .cites
            .unwrap();
        assert_eq!((solo.seen, solo.not_opened, solo.unknown), (1, 1, 0));
        grok.record_tool_use("g2", "spawn_subagent", &json!({ "prompt": "look" }));
        grok.record_tool_result("g2", "done", false);
        let handed = grok.finish(v, "success", vec![]).unwrap().cites.unwrap();
        assert_eq!((handed.seen, handed.not_opened, handed.unknown), (1, 0, 1));

        // A reply that cites nothing records nothing.
        let mut quiet = EvidenceCollector::new("chat", None, "run-no-cites");
        quiet.set_final_reply_citations("Done; nothing to cite.");
        assert!(quiet.finish(v, "success", vec![]).unwrap().cites.is_none());
    }

    /// R5 delta review: on a multi-thread runtime (a chat turn's) the check
    /// runs under `block_in_place`, and still checks the cites.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_cites_check_runs_on_a_multi_thread_runtime() {
        assert_eq!(
            tokio::runtime::Handle::current().runtime_flavor(),
            tokio::runtime::RuntimeFlavor::MultiThread
        );
        let dir = tempdir().unwrap();
        let v = dir.path();
        let cwd = v.join("repo");
        fs::create_dir_all(cwd.join("src")).unwrap();
        let body: String = (1..=30).map(|n| format!("line {n}\n")).collect();
        fs::write(cwd.join("src/a.rs"), &body).unwrap();
        let mut c = EvidenceCollector::new("chat", None, "run-cites-mt");
        c.context_mut().set_run_cwd(&cwd);
        c.record_tool_use(
            "r1",
            "Read",
            &json!({ "file_path": cwd.join("src/a.rs").to_string_lossy() }),
        );
        c.record_tool_result("r1", &body, false);
        c.set_final_reply_citations("See src/a.rs:12 and src/a.rs:99.");
        let cites = c.finish(v, "success", vec![]).unwrap().cites.unwrap();
        assert_eq!((cites.seen, cites.past_end), (1, 1), "{cites:?}");
    }

    /// Agent effectiveness F11 (O3, red first): every Jev call a run makes
    /// appears with that run. A chat turn's connector ranking (its
    /// preparation) and the learning duplicate check that ran after its
    /// receipt are reconciled in the view's `jevCalls` (requests sent, the
    /// fake's own count, latency, each outcome, a `null` cost); another
    /// run's check never appears.
    #[tokio::test]
    async fn a_runs_jev_calls_appear_with_it_including_after_run_checks() {
        use crate::jev::fake::{serve, Reply};
        let dir = tempdir().unwrap();
        let v = dir.path();
        let ranking = serve(Reply::ScoreAll(3.0));
        let dedup = serve(Reply::Status("500 Internal Server Error"));
        let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
        let target = crate::jev::Target {
            guard: &guard,
            generation: guard.generation(),
            allowed: None,
            key: Some("loopback".into()),
            endpoint: ranking.url.clone(),
        };
        let (ids, names) = (["a", "b", "c"], ["Alpha", "Beta", "Gamma"]);
        let candidates: Vec<crate::context_rank::ConnectorCandidate<'_>> = ids
            .iter()
            .zip(names.iter())
            .map(|(id, name)| crate::context_rank::ConnectorCandidate {
                id,
                name,
                description: "a service",
            })
            .collect();
        let deadline = crate::prep_deadline::PrepDeadline::start();
        let selection = crate::context_rank::select_connectors(
            &crate::context_rank::ConnectorQuery::plain("file the invoice"),
            &candidates,
            &deadline,
            &target,
        )
        .await;
        let run = "run-o3-jev-calls";
        let mut c = EvidenceCollector::new("chat", None, run);
        c.set_session_key("s-o3-jev");
        c.context_mut().set_connectors(selection);
        c.finish(v, "success", vec![]).unwrap();
        // The learning check after the receipt, for this run, and another
        // run's check against the same fake.
        let existing = [crate::jev_decide::Record::new("Pay the invoice", "")];
        let proposals = [crate::jev_decide::Record::new("Pay the invoice today", "")];
        for job in [
            crate::jev_decide::Job::learning(Some(run)),
            crate::jev_decide::Job::learning(Some("run-o3-jev-other")),
        ] {
            let _ = crate::jev_decide::check_at(
                "loopback",
                &dedup.url,
                crate::jev_decide::Subject::Learning,
                &proposals,
                &existing,
                &job,
            )
            .await;
        }
        let view = context_manifest(v, Some(run), None).unwrap().unwrap();
        let json = serde_json::to_value(&view).unwrap();
        let sent_here = ranking.seen.lock().unwrap().len() + 1;
        assert_eq!(
            dedup.seen.lock().unwrap().len(),
            2,
            "two checks reached the fake"
        );
        assert_eq!(json["jevCalls"]["sent"], sent_here, "{json}");
        assert_eq!(
            json["jevCalls"]["outcomes"],
            serde_json::json!({ "ok": 1, "error": 1 })
        );
        assert!(json["jevCalls"]["costUsd"].is_null(), "unknown, never 0");
        assert!(json["jevCalls"]["latencyMs"].as_u64().is_some());
        let after = json["afterRun"].as_array().expect("the learning check");
        assert_eq!(after.len(), 1, "only this run's check: {json}");
        assert_eq!(after[0]["runId"], run);
        assert_eq!(after[0]["request"]["use"], "proposals");
    }

    /// F11: a run's context manifest is stored with its receipt (a hashed
    /// sidecar), read back by run, by conversation or as the newest, and
    /// pruned with the receipt.
    #[test]
    fn a_context_manifest_is_stored_with_its_receipt_and_pruned_with_it() {
        let dir = tempdir().unwrap();
        let v = dir.path();
        let mut a = EvidenceCollector::new("chat", None, "run-a");
        a.set_session_key("s-a");
        a.context_mut().layer("request", "alpha words");
        a.context_mut().capabilities.owned_servers = vec!["hollow_vault".into()];
        let ra = a.finish(v, "success", vec![]).unwrap();
        let reference = ra.context_manifest.clone().expect("a reference");
        assert_eq!(
            reference.schema_version,
            crate::run_manifest::SCHEMA_VERSION
        );
        let mut b = EvidenceCollector::new("chat", None, "run-b");
        b.set_session_key("s-b");
        b.context_mut().layer("request", "beta words");
        b.finish(v, "success", vec![]).unwrap();
        // A run with no manifest stores no reference.
        let none = EvidenceCollector::new("heartbeat", None, "run-c")
            .finish(v, "success", vec![])
            .unwrap();
        assert!(none.context_manifest.is_none());
        assert!(!serde_json::to_value(&none)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("contextManifest"));

        let by_run = context_manifest(v, Some("run-a"), None).unwrap().unwrap();
        assert_eq!(by_run.run_id, "run-a");
        assert_eq!(by_run.session_key.as_deref(), Some("s-a"));
        assert_eq!(by_run.manifest.run_id, "run-a");
        assert_eq!(by_run.manifest.capabilities.owned_servers, ["hollow_vault"]);
        let by_session = context_manifest(v, None, Some("s-a")).unwrap().unwrap();
        assert_eq!(by_session.run_id, "run-a");
        let newest = context_manifest(v, None, None).unwrap().unwrap();
        assert_eq!(newest.run_id, "run-b", "the newest run that has one");
        // #386 review: "else the newest run's" applies only without a session
        // key. A conversation with no manifest answers none, never another
        // conversation's newest run.
        assert!(context_manifest(v, None, Some("s-none")).unwrap().is_none());
        assert!(context_manifest(v, Some("run-c"), None).unwrap().is_none());
        assert!(context_manifest(v, Some("run-x"), None).unwrap().is_none());

        // An edited sidecar no longer matches its receipt.
        let side = v.join(crate::run_manifest::DIR_NAME).join("run-a.json");
        let text = fs::read_to_string(&side)
            .unwrap()
            .replace("hollow_vault", "hollow_vaulx");
        fs::write(&side, text).unwrap();
        let err = context_manifest(v, Some("run-a"), None).unwrap_err();
        assert!(err.contains("does not match"), "{err}");

        // Retention follows the receipt: once the store drops run-b's
        // receipt, its sidecar goes on the next write.
        let mut kept: Vec<EvidenceReceipt> = list(v, None, None, KEEP).unwrap();
        kept.retain(|r| r.run_id != "run-b");
        replace_all_for_test(v, kept).unwrap();
        EvidenceCollector::new("chat", None, "run-d")
            .finish(v, "success", vec![])
            .unwrap();
        let dir_entries: Vec<String> = fs::read_dir(v.join(crate::run_manifest::DIR_NAME))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(dir_entries, ["run-a.json"]);
    }

    /// Overnight R1 review (S2): a turn that skips selection keeps the
    /// connector decision on its own conversation's newest chat manifest;
    /// another conversation's is never read, and an edited or missing
    /// manifest gives none.
    #[test]
    fn the_last_connector_selection_is_this_conversations_newest() {
        use crate::context_rank::{Basis, ConnectorDecision, ConnectorSelection};
        let dir = tempdir().unwrap();
        let v = dir.path();
        let chose = |run: &str, key: &str, id: &str| {
            let mut c = EvidenceCollector::new("chat", None, run);
            c.set_session_key(key);
            c.context_mut().set_connectors(ConnectorSelection {
                engine: "local".into(),
                decisions: vec![ConnectorDecision {
                    id: id.into(),
                    attach: true,
                    basis: Basis::Local,
                    score: None,
                    reason: None,
                }],
                ..ConnectorSelection::default()
            });
            c.finish(v, "success", vec![]).unwrap();
            // Receipts sort by their finish time.
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        chose("run-1", "s-mine", "notion");
        chose("run-2", "s-mine", "github");
        chose("run-3", "s-other", "slack");
        let ids = |key: &str| {
            last_connector_selection(v, key)
                .map(|s| s.decisions.into_iter().map(|d| d.id).collect::<Vec<_>>())
        };
        assert_eq!(ids("s-mine"), Some(vec!["github".to_string()]));
        assert_eq!(ids("s-other"), Some(vec!["slack".to_string()]));
        assert_eq!(ids("s-none"), None);
        // An edited sidecar no longer matches its receipt; a removed one is
        // gone. Neither falls back to an older run.
        let side = v.join(crate::run_manifest::DIR_NAME).join("run-2.json");
        let text = fs::read_to_string(&side)
            .unwrap()
            .replace("github", "gitbub");
        fs::write(&side, text).unwrap();
        assert_eq!(ids("s-mine"), None);
        fs::remove_file(&side).unwrap();
        assert_eq!(ids("s-mine"), None);
    }

    /// F11: runs finishing at once (threads) each keep their own manifest.
    #[test]
    fn concurrent_runs_keep_their_own_manifests() {
        let dir = tempdir().unwrap();
        let v = dir.path().to_path_buf();
        let handles: Vec<_> = (0..4)
            .map(|i| {
                let v = v.clone();
                std::thread::spawn(move || {
                    let mut c = EvidenceCollector::new("chat", None, &format!("run-{i}"));
                    c.set_session_key(&format!("s-{i}"));
                    c.context_mut().layer("request", &format!("request {i}"));
                    c.finish(&v, "success", vec![]).unwrap()
                })
            })
            .collect();
        for handle in handles {
            assert!(handle.join().unwrap().context_manifest.is_some());
        }
        for i in 0..4 {
            let view = context_manifest(&v, None, Some(&format!("s-{i}")))
                .unwrap()
                .unwrap();
            assert_eq!(view.run_id, format!("run-{i}"));
            assert_eq!(
                view.manifest.layers[0].sha256,
                crate::run_manifest::sha256_hex(format!("request {i}").as_bytes())
            );
        }
    }

    /// S13: one Write into the run's auto-memory folder counts; a Write
    /// elsewhere, a refused Edit into it and a Bash listing don't.
    #[test]
    fn memory_writes_count_writes_into_the_auto_memory_folder() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let native = crate::run_manifest::native_memory(&home, &cwd);
        let memory = std::path::PathBuf::from(&native.auto_memory.dir);
        let mut c = EvidenceCollector::new("chat", None, "run-memory");
        c.context_mut().native_memory = Some(native);
        let file = |p: std::path::PathBuf| json!({ "file_path": p.to_string_lossy() });
        c.record_tool_use("m1", "Write", &file(memory.join("lesson.md")));
        c.record_tool_result("m1", "ok", false);
        c.record_tool_use("m2", "Edit", &file(memory.join("MEMORY.md")));
        c.record_tool_result("m2", "refused", true);
        c.record_tool_use("w1", "Write", &file(cwd.join("notes.md")));
        c.record_tool_result("w1", "ok", false);
        let list = format!("ls {}", memory.display());
        c.record_tool_use("b1", "Bash", &json!({ "command": list }));
        c.record_tool_result("b1", "", false);
        let receipt = c.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(receipt.memory_writes, Some(1));
    }

    /// S13 review: a MultiEdit into the auto-memory folder counts; a Write
    /// into it that never completed (the run was interrupted before its
    /// result) doesn't.
    #[test]
    fn memory_writes_count_multi_edit_and_skip_interrupted_writes() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let native = crate::run_manifest::native_memory(&home, &cwd);
        let memory = std::path::PathBuf::from(&native.auto_memory.dir);
        let mut c = EvidenceCollector::new("chat", None, "run-memory-2");
        c.context_mut().native_memory = Some(native);
        let file = |p: std::path::PathBuf| json!({ "file_path": p.to_string_lossy() });
        c.record_tool_use("e1", "MultiEdit", &file(memory.join("MEMORY.md")));
        c.record_tool_result("e1", "ok", false);
        // Started, never answered.
        c.record_tool_use("w2", "Write", &file(memory.join("half.md")));
        let receipt = c.finish(dir.path(), "interrupted", vec![]).unwrap();
        assert_eq!(receipt.memory_writes, Some(1));
    }

    /// S13: a run with no `nativeMemory` (another agent) has no count.
    #[test]
    fn memory_writes_absent_without_native_memory() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("work");
        fs::create_dir_all(&cwd).unwrap();
        let native = crate::run_manifest::native_memory(&home, &cwd);
        let memory = std::path::PathBuf::from(&native.auto_memory.dir);
        let file = |p: std::path::PathBuf| json!({ "file_path": p.to_string_lossy() });
        let mut other = EvidenceCollector::new("chat", None, "run-other");
        other.context_mut().layer("request", "r");
        other.record_tool_use("w", "Write", &file(memory.join("x.md")));
        other.record_tool_result("w", "ok", false);
        let receipt = other.finish(dir.path(), "success", vec![]).unwrap();
        assert_eq!(receipt.memory_writes, None);
    }
}
