//! A workspace's Activity (Canvas C5b · Q3, plan §6): one read-only
//! timeline of what happened in it, newest first, for the side panel's
//! Activity section (`workspace_activity`).
//!
//! Every event comes from a record Hollow already keeps. Nothing new is
//! recorded, and reading writes nothing (no file in the vault, no review
//! store moved aside):
//!
//! | Kind | From |
//! | --- | --- |
//! | `session` | the conversations that ran in the workspace (`workspaces::sessions_in`, as `WorkspaceDetail.sessions`) |
//! | `run` | each conversation's turns (the newest [`TURNS_PER_SESSION`]), joined by run id with the run's evidence receipt (status, files, checks, verification, canvases) and its usage row (agent, model, cost, tokens); and receipts of other runs (a workflow, the heartbeat, an older turn) that changed a file in the workspace |
//! | `result` | a receipt's change under `outputs/` (created, modified, deleted by that run); a saved canvas's kept revisions (its sidecar); the review store's history (draft, ready, accepted, and by whom); and, for a file no record explains, the file system (added or changed with no kept run record naming it, from its birth and modified times; `pending` while one of the workspace's turns is running, since a turn's receipt is written when it ends) |
//! | `install` | the app build this app knows for the workspace (a finished build is kept across a restart) and the last install's receipt when it names the workspace |
//!
//! **Retention is the records' own**, and a gap is counted rather than
//! hidden ([`ActivityGaps`]): the receipt store keeps the vault's newest 500
//! receipts, usage rows are kept 400 days, the review store keeps 50 events
//! per result, a canvas 30 revisions, and Hollow keeps only the last install
//! (`last-result.json`) and only the latest finished build. A run whose
//! receipt or usage row aged out still lists, without them, and once the
//! receipt store is full a result change older than its oldest kept receipt
//! is counted too (`results_without_receipt`): a run may have made it. Turn
//! and conversation rows are kept forever.
//!
//! **Bounds and paging:** a page holds at most `limit` events (default
//! [`DEFAULT_LIMIT`], at most [`MAX_LIMIT`]); `total` counts the whole
//! timeline and `next_cursor` continues it with the next, older page (a
//! keyset on the order, so events arriving at the top never shift a later
//! page). The newest [`TURNS_PER_SESSION`] turns per conversation, the
//! newest [`MAX_RESULT_FILES`] result files.
//!
//! **Incremental (Canvas C6 P3):** a read keeps what it read ([`Kept`]) and
//! the next read re-reads only what changed: a conversation's turns when its
//! turn count or last activity moved (or a turn was running), the receipt
//! store when its file changed, usage rows for new runs and for the ledger's
//! changes since its stamp; the timeline is rebuilt only when an input
//! changed, and a turn is drawn only when a page shows it.

use crate::evidence::EvidenceReceipt;
use crate::self_install::{AppBuildStatus, BuildState, InstallReceipt, InstallState};
use crate::session_store::{self, UsageRow};
use crate::types::{AgentSessionSummary, SessionRecordTurn};
use crate::workspace_results::{self, ResultActivity};
use crate::workspaces;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const DEFAULT_LIMIT: usize = 500;
pub const MAX_LIMIT: usize = 2_000;
/// Turns read per conversation, newest first.
pub const TURNS_PER_SESSION: usize = 500;
/// Result files read, newest first.
pub const MAX_RESULT_FILES: usize = 2_000;
/// A file modified more than this after the record that explains its last
/// write was changed again by something no record names.
const SLACK_MS: i64 = 2_000;
/// The first line of a turn's message shown as its title.
const TITLE_CHARS: usize = 160;

/// `workspace_activity`'s answer.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceActivity {
    pub workspace_id: String,
    pub as_of: String,
    /// Newest first, at most the limit asked for.
    pub events: Vec<ActivityEvent>,
    /// How many events the whole timeline has.
    pub total: usize,
    pub gaps: ActivityGaps,
    /// Pass back as `cursor` for the next, older page; `None` on the last
    /// page (Canvas C6 P3).
    pub next_cursor: Option<String>,
}

/// What the timeline could not see, so the view can say so.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivityGaps {
    /// Finished turns whose receipt is no longer kept (the store keeps the
    /// vault's newest 500): listed without files, checks or canvases.
    pub runs_without_receipt: usize,
    /// Finished turns with no usage row (kept 400 days): no model or cost.
    /// Zero when the usage store couldn't be read ([`Self::usage_error`]).
    pub runs_without_usage: usize,
    /// Result files whose change no kept record names and that changed
    /// before the oldest receipt the full store still keeps: a run whose
    /// receipt aged out may have made it.
    pub results_without_receipt: usize,
    /// A conversation has more turns than the timeline reads.
    pub turns_truncated: bool,
    /// `outputs/` has more files than the timeline reads.
    pub results_truncated: bool,
    /// The receipt store couldn't be read: runs list without receipts.
    pub receipts_error: Option<String>,
    /// The Results review store couldn't be read (it is left as it is): no
    /// review events.
    pub reviews_error: Option<String>,
    /// The usage store couldn't be read: every run lists without model,
    /// cost and tokens, and none is counted in `runs_without_usage`.
    pub usage_error: Option<String>,
}

/// One row of the timeline.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    /// Unique in the list and stable across reads.
    pub id: String,
    #[cfg_attr(test, ts(as = "crate::ts_bindings::ActivityKind"))]
    pub kind: String,
    /// RFC 3339.
    pub at: String,
    #[cfg_attr(test, ts(as = "crate::ts_bindings::ActivityStatus"))]
    pub status: String,
    pub title: String,
    /// The conversation to open (a session, a turn, a result a run or a
    /// canvas revision came from).
    pub session_key: Option<String>,
    pub run_id: Option<String>,
    /// The evidence receipt to open, when it is still kept.
    pub receipt_id: Option<String>,
    /// A result's vault-relative path.
    pub path: Option<String>,
    pub session: Option<ActivitySession>,
    pub run: Option<ActivityRun>,
    pub result: Option<ActivityResult>,
    pub install: Option<ActivityInstall>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivitySession {
    pub agent: String,
    pub turns: u32,
    pub last_activity_at: String,
}

/// A run: a conversation's turn, or another run's receipt.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivityRun {
    /// `chat`, `workflow`, `heartbeat`, `ingest`…: the receipt's source.
    pub source: String,
    pub source_id: Option<String>,
    /// The turn's number in its conversation.
    pub turn: Option<u32>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    /// From the usage row, else the conversation's agent at the time.
    pub agent: Option<String>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
    /// The usage ledger's cost status (`reported`, `estimated`, `unpriced`,
    /// `partial`, `unknown`).
    pub cost_status: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    /// The rest come from the receipt; all `None` without one.
    pub files: Option<usize>,
    /// Of `files`, those under this workspace's `outputs/`.
    pub outputs: Option<usize>,
    pub checks: Option<usize>,
    pub failed_checks: Option<usize>,
    pub verification: Option<String>,
    pub result_kind: Option<String>,
    pub canvases: Option<ActivityCanvases>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivityCanvases {
    pub count: u32,
    pub bytes: u64,
}

/// A result event; its `status` says what happened.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivityResult {
    /// Relative to the workspace folder (`outputs/x.md`).
    pub name: String,
    /// `workspace_results`' kind (`document`, `canvas`, `data`…).
    pub kind: String,
    /// The file is in `outputs/` now.
    pub exists: bool,
    /// Who: `run` (a receipt), `chat` (a canvas revision), `person` or
    /// `agent` (a review); `None` when no record names anyone.
    #[cfg_attr(test, ts(as = "Option<crate::ts_bindings::ActivityActor>"))]
    pub actor: Option<String>,
    /// A canvas revision's number.
    pub revision: Option<u64>,
    /// A review's note.
    pub note: Option<String>,
}

/// An app build or install of this workspace's worktree.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivityInstall {
    /// `build` | `install`.
    #[cfg_attr(test, ts(type = "\"build\" | \"install\""))]
    pub step: String,
    pub commit: Option<String>,
    pub branch: Option<String>,
    /// The install receipt's own sentence, or the build's error.
    pub summary: Option<String>,
    /// The running app reports the installed commit.
    pub matches_running: Option<bool>,
}

/// Everything the timeline is built from, read by [`list`].
#[derive(Debug)]
pub(crate) struct Inputs {
    pub workspace_id: String,
    /// Vault-relative folder (`workspaces/<id>`).
    pub workspace_path: String,
    pub sessions: Vec<AgentSessionSummary>,
    pub turns: HashMap<String, Vec<SessionRecordTurn>>,
    /// `Err` when the usage store couldn't be read (said, not swallowed).
    pub usage: Result<HashMap<String, Vec<UsageRow>>, String>,
    pub receipts: Result<Vec<EvidenceReceipt>, String>,
    pub results: ResultActivity,
    pub build: Option<AppBuildStatus>,
    pub install: Option<InstallReceipt>,
    pub as_of: String,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            workspace_path: String::new(),
            sessions: Vec::new(),
            turns: HashMap::new(),
            usage: Ok(HashMap::new()),
            receipts: Ok(Vec::new()),
            results: ResultActivity::default(),
            build: None,
            install: None,
            as_of: String::new(),
        }
    }
}

/// Workspace `id`'s timeline, newest first: at most `limit` events after
/// `cursor` (a previous page's `next_cursor`; `None` for the newest page).
/// Reads only what changed since the last read (see [`Kept`]).
pub fn list(
    root: &Path,
    id: &str,
    limit: Option<usize>,
    cursor: Option<&str>,
) -> Result<WorkspaceActivity, String> {
    static KEPT: std::sync::Mutex<Option<Kept>> = std::sync::Mutex::new(None);
    let mut kept = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    list_with(root, id, limit, cursor, &mut kept)
}

/// [`list`] with the kept reads in `kept`: the last workspace read, replaced
/// when another is asked for.
pub(crate) fn list_with(
    root: &Path,
    id: &str,
    limit: Option<usize>,
    cursor: Option<&str>,
    kept: &mut Option<Kept>,
) -> Result<WorkspaceActivity, String> {
    let after = cursor.map(decode_cursor).transpose()?;
    let prev = kept
        .take()
        .filter(|k| k.root == root && k.workspace_id == id);
    let k = kept.insert(refresh(root, id, prev)?);
    let timeline = k.timeline.as_ref().expect("refresh builds the timeline");
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let (events, next) = timeline.page(&k.inputs, after.as_ref(), limit);
    Ok(WorkspaceActivity {
        workspace_id: k.inputs.workspace_id.clone(),
        as_of: chrono::Utc::now().to_rfc3339(),
        events,
        total: timeline.entries.len(),
        gaps: timeline.gaps.clone(),
        next_cursor: next.map(|key| encode_cursor(&key)),
    })
}

/// What a read of a workspace's timeline keeps for the next one (Canvas C6
/// P3): Activity re-reads every minute while open and on every change in
/// the workspace, and a big vault holds up to 1,000 conversations of 500
/// turns. A conversation's turns are read again only when its turn count or
/// last activity moved, or a turn of it was still running; the receipt
/// store only when its file's size, modified time or inode changed; usage
/// rows only for runs not read before, plus the rows the ledger inserted or
/// updated since its last stamp (all of them again when rows were pruned);
/// and the timeline is rebuilt only when something it is built from
/// changed. Results (at most 2,000 files), the build and the install are
/// read every time and compared. One workspace is kept at a time.
#[derive(Default)]
pub(crate) struct Kept {
    root: std::path::PathBuf,
    workspace_id: String,
    inputs: Inputs,
    /// Per conversation: `(turn count, last activity)` when its turns were read.
    turn_stamps: HashMap<String, (u32, String)>,
    receipts_stamp: Option<Option<crate::workspace_canvases::FileStamp>>,
    usage_stamp: Option<session_store::UsageStamp>,
    /// Runs whose usage rows are kept (a run with none is still listed).
    usage_runs: HashSet<String>,
    timeline: Option<Timeline>,
    /// What each read had to read, for tests and the measurement.
    pub(crate) reads: Reads,
}

/// Counts of what [`refresh`] read (cumulative).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    /// Conversations whose turns were read.
    pub conversations: usize,
    /// Runs whose usage rows were looked up one by one.
    pub usage_runs: usize,
    /// Conversations whose runs' usage rows were read together.
    pub usage_conversations: usize,
    /// Usage rows taken from the ledger's changes since its last stamp.
    pub usage_changes: usize,
    pub receipt_store: usize,
    pub timelines: usize,
}

/// The turns kept for a conversation: only the first non-blank line of
/// what was asked is used (the title), so only it is kept.
fn slim_turns(turns: &mut [SessionRecordTurn]) {
    for t in turns {
        if let Some(asked) = &t.asked {
            let line = asked.lines().map(str::trim).find(|l| !l.is_empty());
            t.asked = line.map(str::to_string);
        }
    }
}

/// A usage row as the timeline keeps it: without its raw usage JSON.
fn slim_usage(mut row: UsageRow) -> UsageRow {
    row.usage_json = None;
    row.native_session = None;
    row
}

/// The fields of a build the timeline shows.
fn build_shown(
    b: &Option<AppBuildStatus>,
) -> Option<(
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
    b.as_ref().map(|b| {
        (
            format!("{:?}", b.state),
            b.started_at.clone(),
            b.finished_at.clone(),
            b.error.clone(),
            b.commit.clone(),
            b.branch.clone(),
        )
    })
}

/// Bring `prev` (the last read of this workspace, if any) up to date,
/// reading only what changed, and rebuild the timeline when anything did.
fn refresh(root: &Path, id: &str, prev: Option<Kept>) -> Result<Kept, String> {
    let workspace = workspaces::summary(root, id)?;
    let dir = workspaces::folder(root, id)?;
    let sessions = workspaces::sessions_in(root, &dir, &workspace)?;
    let mut k = prev.unwrap_or_else(|| Kept {
        root: root.to_path_buf(),
        workspace_id: id.to_string(),
        ..Default::default()
    });
    let mut changed = k.timeline.is_none()
        || k.inputs.workspace_id != workspace.id
        || k.inputs.workspace_path != workspace.path;
    // The set of runs on the timeline moved beyond the conversations re-read
    // below (whose runs are read with them): a first read, a conversation
    // gone, or the receipt store changed.
    let mut runs_moved = changed;
    k.inputs.workspace_id = workspace.id.clone();
    k.inputs.workspace_path = workspace.path.clone();

    // Conversations and their turns.
    let stale: Vec<String> = sessions
        .iter()
        .filter(|s| {
            let stamp = (s.turn_count, s.last_activity_at.clone());
            let running =
                k.inputs.turns.get(&s.key).is_some_and(|turns| {
                    turns.iter().any(|t| t.status.as_deref() == Some("running"))
                });
            running || k.turn_stamps.get(&s.key) != Some(&stamp)
        })
        .map(|s| s.key.clone())
        .collect();
    let live: HashSet<&str> = sessions.iter().map(|s| s.key.as_str()).collect();
    let before = k.inputs.turns.len();
    k.inputs.turns.retain(|key, _| live.contains(key.as_str()));
    k.turn_stamps.retain(|key, _| live.contains(key.as_str()));
    if k.inputs.turns.len() != before {
        changed = true;
        runs_moved = true;
    }
    if !stale.is_empty() {
        let read = session_store::recent_turns_for_sessions(root, &stale, TURNS_PER_SESSION)?;
        k.reads.conversations += stale.len();
        for (key, mut turns) in read {
            slim_turns(&mut turns);
            if k.inputs.turns.get(&key) != Some(&turns) {
                changed = true;
                k.inputs.turns.insert(key, turns);
            }
        }
        for s in sessions.iter().filter(|s| stale.contains(&s.key)) {
            k.turn_stamps
                .insert(s.key.clone(), (s.turn_count, s.last_activity_at.clone()));
        }
    }
    if k.inputs.sessions != sessions {
        changed = true;
        k.inputs.sessions = sessions;
    }

    // The receipt store, when its file changed (or the last read failed).
    let stamp = std::fs::symlink_metadata(root.join(crate::evidence::FILE_NAME))
        .ok()
        .map(|m| crate::workspace_canvases::FileStamp::of(&m));
    if k.receipts_stamp != Some(stamp) || k.inputs.receipts.is_err() {
        let receipts = crate::evidence::list(root, None, None, usize::MAX);
        k.reads.receipt_store += 1;
        k.receipts_stamp = receipts.is_ok().then_some(stamp);
        if k.inputs.receipts != receipts {
            changed = true;
            runs_moved = true;
            k.inputs.receipts = receipts;
        }
    }

    // Usage: the runs of these turns and of receipts that touched the
    // workspace. Usage is an enrichment: a store that can't answer leaves
    // runs without model and cost, never the timeline empty, and the view
    // says why.
    match refresh_usage(root, &mut k, &stale, runs_moved) {
        Ok(moved) => changed |= moved,
        Err(e) => {
            changed |= k.inputs.usage.as_ref().err() != Some(&e);
            k.inputs.usage = Err(e);
            k.usage_stamp = None;
            k.usage_runs.clear();
        }
    }

    // Results, the build and the install: bounded, read every time.
    let results = workspace_results::activity(root, id, MAX_RESULT_FILES)?;
    if k.inputs.results != results {
        changed = true;
        k.inputs.results = results;
    }
    let build = crate::self_install::build_job_for_workspace(id);
    if build_shown(&k.inputs.build) != build_shown(&build) {
        changed = true;
    }
    k.inputs.build = build;
    let install = crate::self_install::last_receipt_for_workspace(id);
    if k.inputs.install != install {
        changed = true;
        k.inputs.install = install;
    }

    if changed {
        k.inputs.as_of = chrono::Utc::now().to_rfc3339();
        k.timeline = Some(build_timeline(&k.inputs));
        k.reads.timelines += 1;
    }
    Ok(k)
}

/// The runs whose usage the timeline shows: every kept turn's, and those
/// of receipts that touched the workspace.
fn timeline_runs(k: &Kept) -> HashSet<String> {
    let prefix = format!("{}/", k.inputs.workspace_path);
    let keys: HashSet<&str> = k.inputs.sessions.iter().map(|s| s.key.as_str()).collect();
    let mut runs: HashSet<String> = k
        .inputs
        .turns
        .values()
        .flatten()
        .filter_map(|t| t.run_id.clone())
        .collect();
    if let Ok(receipts) = &k.inputs.receipts {
        runs.extend(
            receipts
                .iter()
                .filter(|r| touches(r, &prefix, &keys))
                .map(|r| r.run_id.clone()),
        );
    }
    runs
}

/// Bring the kept usage rows up to date; `Ok(true)` when any row the
/// timeline shows changed. `reread` are the conversations whose turns were
/// just read (their runs' rows are read with them, in one query);
/// `runs_moved` says the set of runs may have changed.
fn refresh_usage(
    root: &Path,
    k: &mut Kept,
    reread: &[String],
    runs_moved: bool,
) -> Result<bool, String> {
    if k.inputs.usage.is_err() {
        k.usage_stamp = None;
        k.usage_runs.clear();
    }
    let (stamp, fresh) = session_store::usage_changes_since(root, k.usage_stamp.as_ref())?;
    let mut moved = false;
    let mut usage = match std::mem::replace(&mut k.inputs.usage, Ok(HashMap::new())) {
        Ok(u) => u,
        Err(_) => {
            moved = true;
            HashMap::new()
        }
    };
    let mut runs_moved = runs_moved || moved;
    // Conversations whose runs' rows are read together below.
    let mut together: Vec<String> = reread.to_vec();
    if let Some(old) = k.usage_stamp.as_ref().filter(|old| **old != stamp) {
        k.reads.usage_changes += fresh.len();
        let inserted = fresh.iter().filter(|(id, _)| *id > old.max_id).count() as i64;
        if stamp.count != old.count + inserted {
            // Rows were pruned: read every conversation's runs again.
            usage.clear();
            k.usage_runs.clear();
            together = k.inputs.sessions.iter().map(|s| s.key.clone()).collect();
            moved = true;
            runs_moved = true;
        } else {
            for (_, row) in fresh {
                let Some(run) = row.run_id.clone().filter(|r| k.usage_runs.contains(r)) else {
                    continue;
                };
                let rows = usage.entry(run).or_default();
                let row = slim_usage(row);
                match rows.iter_mut().find(|r| r.run_key == row.run_key) {
                    Some(same) if *same == row => {}
                    Some(same) => {
                        *same = row;
                        moved = true;
                    }
                    None => {
                        rows.push(row);
                        moved = true;
                    }
                }
                rows.sort_by(|a, b| a.at.cmp(&b.at));
            }
        }
    }
    // The conversations just read: their runs' rows in one query.
    if !together.is_empty() {
        let mut read = session_store::usage_rows_for_sessions(root, &together)?;
        k.reads.usage_conversations += together.len();
        for key in &together {
            for run in k
                .inputs
                .turns
                .get(key)
                .into_iter()
                .flatten()
                .filter_map(|t| t.run_id.as_ref())
            {
                match read.remove(run) {
                    Some(rows) => {
                        usage.insert(run.clone(), rows.into_iter().map(slim_usage).collect());
                    }
                    None => {
                        usage.remove(run);
                    }
                }
                k.usage_runs.insert(run.clone());
            }
        }
        moved = true;
    }
    // Runs no longer on the timeline are forgotten; any other new one (a
    // receipt's) is read on its own.
    if runs_moved {
        let runs = timeline_runs(k);
        let before = usage.len();
        usage.retain(|run, _| runs.contains(run));
        k.usage_runs.retain(|run| runs.contains(run));
        moved |= usage.len() != before;
        let mut new: Vec<String> = runs
            .iter()
            .filter(|r| !k.usage_runs.contains(*r))
            .cloned()
            .collect();
        new.sort();
        if !new.is_empty() {
            let read = session_store::usage_rows_for_runs(root, &new)?;
            k.reads.usage_runs += new.len();
            moved |= !read.is_empty();
            for (run, rows) in read {
                usage.insert(run, rows.into_iter().map(slim_usage).collect());
            }
            k.usage_runs.extend(new);
        }
    }
    k.usage_stamp = Some(stamp);
    k.inputs.usage = Ok(usage);
    Ok(moved)
}

/// The order of the timeline: newest first (an unreadable time last); at
/// the same instant a run before the results it wrote, then installs, then
/// conversations; then by id, so the order is the same on every read.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct OrderKey {
    /// 0 with a time, 1 without (last).
    untimed: u8,
    newest: std::cmp::Reverse<i64>,
    rank: u8,
    id: String,
}

impl OrderKey {
    fn new(at: &str, kind: &str, id: String) -> Self {
        let ms = ms(at);
        Self {
            untimed: u8::from(ms.is_none()),
            newest: std::cmp::Reverse(ms.unwrap_or(0)),
            rank: match kind {
                "run" => 0,
                "result" => 1,
                "install" => 2,
                _ => 3,
            },
            id,
        }
    }
}

/// A page cursor: the last event's [`OrderKey`], opaque to callers
/// (URL-safe base64 of `[ms or null, rank, id]`).
fn encode_cursor(key: &OrderKey) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let ms = (key.untimed == 0).then_some(key.newest.0);
    URL_SAFE_NO_PAD.encode(serde_json::json!([ms, key.rank, key.id]).to_string())
}

/// A cursor back to its key; an error for anything [`list`] didn't give
/// out, so a bad cursor never silently restarts the timeline.
fn decode_cursor(cursor: &str) -> Result<OrderKey, String> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let bad = || "That page cursor is not one this timeline gave out.".to_string();
    let bytes = URL_SAFE_NO_PAD.decode(cursor.trim()).map_err(|_| bad())?;
    let (ms, rank, id): (Option<i64>, u8, String) =
        serde_json::from_slice(&bytes).map_err(|_| bad())?;
    if rank > 3 || id.is_empty() {
        return Err(bad());
    }
    Ok(OrderKey {
        untimed: u8::from(ms.is_none()),
        newest: std::cmp::Reverse(ms.unwrap_or(0)),
        rank,
        id,
    })
}

/// One row of a built timeline. A turn (the bulk of a big workspace) is
/// kept as where it is in the inputs and drawn only when a page shows it;
/// every other event is drawn when the timeline is built.
#[derive(Clone, Debug)]
enum Item {
    Event(Box<ActivityEvent>),
    Turn { session: usize, turn: usize },
}

#[derive(Clone, Debug)]
struct Entry {
    key: OrderKey,
    item: Item,
}

/// Every event of a workspace in order, and what it couldn't see.
#[derive(Clone, Debug, Default)]
pub(crate) struct Timeline {
    entries: Vec<Entry>,
    gaps: ActivityGaps,
}

impl Timeline {
    /// At most `limit` events strictly after `after` in the order, and the
    /// key to continue from when there are more.
    fn page(
        &self,
        input: &Inputs,
        after: Option<&OrderKey>,
        limit: usize,
    ) -> (Vec<ActivityEvent>, Option<OrderKey>) {
        let start = after.map_or(0, |a| self.entries.partition_point(|e| e.key <= *a));
        let end = (start + limit).min(self.entries.len());
        let cx = TurnContext::new(input);
        let events = self.entries[start..end]
            .iter()
            .map(|e| match &e.item {
                Item::Event(event) => (**event).clone(),
                Item::Turn { session, turn } => {
                    let s = &input.sessions[*session];
                    let t = &input.turns[&s.key][*turn];
                    cx.turn_event(s, t)
                }
            })
            .collect();
        let next =
            (end < self.entries.len() && end > start).then(|| self.entries[end - 1].key.clone());
        (events, next)
    }
}

/// What drawing a turn needs from the inputs besides the turn itself.
struct TurnContext<'a> {
    by_run: HashMap<&'a str, &'a EvidenceReceipt>,
    usage: Option<&'a HashMap<String, Vec<UsageRow>>>,
    outputs_prefix: String,
}

impl<'a> TurnContext<'a> {
    fn new(input: &'a Inputs) -> Self {
        Self {
            by_run: newest_receipt_by_run(input.receipts.as_deref().unwrap_or(&[])),
            usage: input.usage.as_ref().ok(),
            outputs_prefix: format!("{}/outputs/", input.workspace_path),
        }
    }

    fn receipt(&self, t: &SessionRecordTurn) -> Option<&'a EvidenceReceipt> {
        t.run_id
            .as_deref()
            .and_then(|r| self.by_run.get(r))
            .copied()
    }

    /// A conversation's turn as a run event, joined with its receipt and
    /// usage row.
    fn turn_event(&self, s: &AgentSessionSummary, t: &SessionRecordTurn) -> ActivityEvent {
        let receipt = self.receipt(t);
        let usage = t
            .run_id
            .as_deref()
            .and_then(|r| self.usage.and_then(|u| u.get(r)));
        let mut run = run_facts(
            receipt,
            usage,
            Some(agent_at(s, &t.started_at)),
            &self.outputs_prefix,
        );
        run.turn = Some(t.ord);
        run.started_at = Some(t.started_at.clone());
        run.ended_at = t.ended_at.clone();
        ActivityEvent {
            id: turn_id(s, t),
            kind: "run".into(),
            at: turn_at(receipt, t),
            status: run_status(t.status.as_deref().or(receipt.map(|r| r.status.as_str()))).into(),
            title: t
                .asked
                .as_deref()
                .and_then(first_line)
                .unwrap_or_else(|| format!("Turn {}", t.ord)),
            session_key: Some(s.key.clone()),
            run_id: t.run_id.clone(),
            receipt_id: receipt.map(|r| r.id.clone()),
            run: Some(run),
            ..Default::default()
        }
    }
}

/// A turn's event id: its run, or its place in its conversation.
fn turn_id(s: &AgentSessionSummary, t: &SessionRecordTurn) -> String {
    match &t.run_id {
        Some(r) => format!("run:{r}"),
        None => format!("turn:{}:{}", s.key, t.ord),
    }
}

/// When a turn happened: its receipt's end, else its own end, else its start.
fn turn_at(receipt: Option<&EvidenceReceipt>, t: &SessionRecordTurn) -> String {
    receipt
        .map(|r| r.finished_at.clone())
        .or_else(|| t.ended_at.clone())
        .unwrap_or_else(|| t.started_at.clone())
}

/// The newest receipt of each run.
fn newest_receipt_by_run(receipts: &[EvidenceReceipt]) -> HashMap<&str, &EvidenceReceipt> {
    let mut by_run: HashMap<&str, &EvidenceReceipt> = HashMap::new();
    for r in receipts {
        let newer = by_run
            .get(r.run_id.as_str())
            .is_none_or(|old| ms(&r.finished_at) > ms(&old.finished_at));
        if newer {
            by_run.insert(r.run_id.as_str(), r);
        }
    }
    by_run
}

/// Whether receipt `r` belongs on this workspace's timeline: it changed a
/// file in the workspace folder, or it is one of its conversations' turns.
fn touches(r: &EvidenceReceipt, prefix: &str, keys: &HashSet<&str>) -> bool {
    r.session_key.as_deref().is_some_and(|k| keys.contains(k))
        || r.files.iter().any(|f| f.path.starts_with(prefix))
}

/// Milliseconds since the epoch, for ordering; `None` when unparsable.
fn ms(at: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(at.trim())
        .ok()
        .map(|t| t.timestamp_millis())
}

/// The first non-blank line of `text`, trimmed and cut to [`TITLE_CHARS`].
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out: String = line.chars().take(TITLE_CHARS).collect();
    if line.chars().count() > TITLE_CHARS {
        out.push('…');
    }
    Some(out)
}

/// A run's status from its turn (or, without one, its receipt).
fn run_status(status: Option<&str>) -> &'static str {
    match status {
        Some("running") => "running",
        Some("success") => "succeeded",
        Some("error" | "failed") => "failed",
        Some("cancelled" | "stopped") => "stopped",
        Some("interrupted" | "abandoned") => "interrupted",
        _ => "unknown",
    }
}

/// A receipt's change under `outputs/` as a result status.
fn change_status(change_type: &str) -> &'static str {
    match change_type {
        "create" => "created",
        "delete" => "deleted",
        _ => "modified",
    }
}

/// Store bookkeeping, never a Result of its own: anything in a dot folder
/// (`.revisions/`), a saved canvas's `.canvas.json` record, a retired curated
/// section view's files (`_plan.canvas.*`, Canvas C6 · K1), and a page
/// review's capture images (beside its `<slug>.annotations.json`).
fn is_bookkeeping(rel: &str, annotation_sets: &HashSet<String>) -> bool {
    rel.split('/').any(|seg| seg.starts_with('.'))
        // Not a Result (canvas sidecars, retired curated views, a compacted
        // brief's archive, a process capture's frames): not activity either.
        || !crate::workspace_results::is_listed(rel)
        || rel
            .rsplit_once('/')
            .is_some_and(|(parent, _)| annotation_sets.contains(parent))
}

/// A path's file name without its extension, as a title of last resort.
fn stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.split('.').next().unwrap_or(name).to_string()
}

/// The agent a conversation had when its turn started: its stint then, else
/// its current agent.
fn agent_at(session: &AgentSessionSummary, at: &str) -> String {
    let when = ms(at);
    session
        .stints
        .iter()
        .filter(|s| matches!((ms(&s.started_at), when), (Some(a), Some(b)) if a <= b))
        .max_by_key(|s| ms(&s.started_at))
        .map(|s| s.agent.clone())
        .unwrap_or_else(|| session.agent.clone())
}

/// A run's facts from its receipt and usage rows.
fn run_facts(
    receipt: Option<&EvidenceReceipt>,
    usage: Option<&Vec<UsageRow>>,
    agent: Option<String>,
    outputs_prefix: &str,
) -> ActivityRun {
    // The run's own agent; a delegation shares the run id but is not it.
    let own = usage.and_then(|rows| {
        rows.iter()
            .find(|r| r.surface != "delegate")
            .or_else(|| rows.first())
    });
    ActivityRun {
        source: receipt
            .map(|r| r.source.clone())
            .unwrap_or_else(|| "chat".into()),
        source_id: receipt.and_then(|r| r.source_id.clone()),
        agent: own.and_then(|u| u.agent.clone()).or(agent),
        model: own.and_then(|u| u.model.clone().or_else(|| u.requested_model.clone())),
        cost_usd: own.and_then(|u| u.cost_usd),
        cost_status: own.map(|u| u.cost_status.clone()),
        input_tokens: own.and_then(|u| u.input_tokens),
        output_tokens: own.and_then(|u| u.output_tokens),
        // A shared file is another chat's (chat concurrency F1).
        files: receipt.map(|r| r.files.iter().filter(|f| f.is_own()).count()),
        outputs: receipt.map(|r| {
            r.files
                .iter()
                .filter(|f| f.is_own() && f.path.starts_with(outputs_prefix))
                .count()
        }),
        checks: receipt.map(|r| r.checks.len()),
        failed_checks: receipt.map(|r| r.checks.iter().filter(|c| c.outcome == "failed").count()),
        verification: receipt.map(|r| r.verification_state.clone()),
        result_kind: receipt.and_then(|r| r.result_kind.clone()),
        canvases: receipt.and_then(|r| r.canvases).map(|c| ActivityCanvases {
            count: c.count,
            bytes: c.bytes,
        }),
        ..Default::default()
    }
}

/// The first page of the timeline from `input`, at most `limit` events.
/// Pure.
#[cfg(test)]
pub(crate) fn assemble(input: Inputs, limit: usize) -> WorkspaceActivity {
    let timeline = build_timeline(&input);
    let (events, next) = timeline.page(&input, None, limit);
    WorkspaceActivity {
        workspace_id: input.workspace_id.clone(),
        as_of: input.as_of.clone(),
        events,
        total: timeline.entries.len(),
        gaps: timeline.gaps,
        next_cursor: next.map(|key| encode_cursor(&key)),
    }
}

/// Every event from `input`, in order. Pure.
fn build_timeline(input: &Inputs) -> Timeline {
    let mut gaps = ActivityGaps {
        results_truncated: input.results.truncated,
        reviews_error: input.results.store_error.clone(),
        ..Default::default()
    };
    let receipts: &[EvidenceReceipt] = match &input.receipts {
        Ok(r) => r,
        Err(e) => {
            gaps.receipts_error = Some(e.clone());
            &[]
        }
    };
    let receipts_known = gaps.receipts_error.is_none();
    let no_usage = HashMap::new();
    let usage: &HashMap<String, Vec<UsageRow>> = match &input.usage {
        Ok(u) => u,
        Err(e) => {
            gaps.usage_error = Some(e.clone());
            &no_usage
        }
    };
    let usage_known = gaps.usage_error.is_none();
    // A full store has dropped its oldest receipts: a change older than the
    // oldest kept one may be a run's whose receipt is gone.
    let store_floor = (receipts.len() >= crate::evidence::KEEP)
        .then(|| receipts.iter().filter_map(|r| ms(&r.finished_at)).min())
        .flatten();
    // A turn's receipt is written when it ends: while one of this
    // workspace's turns runs, a change since it started has no record yet.
    let running_since = input
        .sessions
        .iter()
        .filter_map(|s| input.turns.get(&s.key))
        .flatten()
        .filter(|t| t.status.as_deref() == Some("running"))
        .filter_map(|t| ms(&t.started_at))
        .min();
    let mut aged: HashSet<&str> = HashSet::new();
    let prefix = format!("{}/", input.workspace_path);
    let outputs_prefix = format!("{}/outputs/", input.workspace_path);
    let keys: HashSet<&str> = input.sessions.iter().map(|s| s.key.as_str()).collect();
    let by_run = newest_receipt_by_run(receipts);
    let files: HashMap<&str, &workspace_results::ResultFile> = input
        .results
        .files
        .iter()
        .map(|f| (f.rel.as_str(), f))
        .collect();
    let annotation_sets: HashSet<String> = input
        .results
        .files
        .iter()
        .filter_map(|f| f.rel.strip_suffix(".annotations.json").map(String::from))
        .collect();
    let title_of = |rel: &str| {
        files
            .get(rel)
            .map(|f| f.title.clone())
            .unwrap_or_else(|| stem(rel))
    };
    let mut events = Entries::default();
    let push = |events: &mut Entries, e: ActivityEvent| {
        let key = OrderKey::new(&e.at, &e.kind, e.id.clone());
        events.add(key, Item::Event(Box::new(e)));
    };
    let mut joined: HashSet<&str> = HashSet::new();

    // Conversations and their turns.
    for (si, s) in input.sessions.iter().enumerate() {
        let turns = input.turns.get(&s.key).map(Vec::as_slice).unwrap_or(&[]);
        if (s.turn_count as usize) > turns.len() && turns.len() >= TURNS_PER_SESSION {
            gaps.turns_truncated = true;
        }
        let running = turns.iter().any(|t| t.status.as_deref() == Some("running"));
        push(
            &mut events,
            ActivityEvent {
                id: format!("session:{}", s.key),
                kind: "session".into(),
                at: s.created_at.clone(),
                status: if running { "running" } else { "started" }.into(),
                title: s
                    .title
                    .as_deref()
                    .and_then(first_line)
                    .unwrap_or_else(|| "Conversation".into()),
                session_key: Some(s.key.clone()),
                session: Some(ActivitySession {
                    agent: s.agent.clone(),
                    turns: s.turn_count,
                    last_activity_at: s.last_activity_at.clone(),
                }),
                ..Default::default()
            },
        );
        for (ti, t) in turns.iter().enumerate() {
            let receipt = t.run_id.as_deref().and_then(|r| by_run.get(r)).copied();
            if let Some(r) = receipt {
                joined.insert(r.id.as_str());
            }
            let finished = t.status.as_deref() != Some("running");
            if finished && t.run_id.is_some() {
                if receipt.is_none() && receipts_known {
                    gaps.runs_without_receipt += 1;
                }
                let has_usage = t.run_id.as_deref().is_some_and(|r| usage.contains_key(r));
                if !has_usage && usage_known {
                    gaps.runs_without_usage += 1;
                }
            }
            // Drawn when a page shows it ([`TurnContext::turn_event`]).
            let at = match receipt {
                Some(r) => &r.finished_at,
                None => t.ended_at.as_ref().unwrap_or(&t.started_at),
            };
            let key = OrderKey::new(at, "run", turn_id(s, t));
            events.add(
                key,
                Item::Turn {
                    session: si,
                    turn: ti,
                },
            );
        }
    }

    // Other runs that changed the workspace (a workflow, the heartbeat, a
    // turn older than the ones read).
    for r in receipts {
        if joined.contains(r.id.as_str()) || !touches(r, &prefix, &keys) {
            continue;
        }
        let title = match (r.source.as_str(), r.source_id.as_deref()) {
            ("workflow", Some(slug)) => format!("Workflow {slug}"),
            ("workflow", None) => "Workflow run".into(),
            ("heartbeat", _) => "Heartbeat".into(),
            ("ingest", _) => "Ingest".into(),
            ("chat", _) => "Chat run".into(),
            (other, _) => format!("{other} run"),
        };
        push(
            &mut events,
            ActivityEvent {
                id: format!("receipt:{}", r.id),
                kind: "run".into(),
                at: r.finished_at.clone(),
                status: run_status(Some(r.status.as_str())).into(),
                title,
                session_key: r.session_key.clone().filter(|k| keys.contains(k.as_str())),
                run_id: Some(r.run_id.clone()),
                receipt_id: Some(r.id.clone()),
                run: Some(ActivityRun {
                    started_at: Some(r.started_at.clone()),
                    ended_at: Some(r.finished_at.clone()),
                    ..run_facts(Some(r), usage.get(&r.run_id), None, &outputs_prefix)
                }),
                ..Default::default()
            },
        );
    }

    // Results: what runs did to outputs/.
    let mut explained: HashMap<String, i64> = HashMap::new();
    for r in receipts {
        // Another chat's file in a shared folder (`shared`, chat concurrency
        // F1) is not this run's result, and explains nothing.
        for f in r.files.iter().filter(|f| f.is_own()) {
            let Some(rel) = f.path.strip_prefix(&prefix) else {
                continue;
            };
            if !rel.starts_with("outputs/") || is_bookkeeping(rel, &annotation_sets) {
                continue;
            }
            if let Some(at) = ms(&r.finished_at) {
                let e = explained.entry(rel.to_string()).or_insert(at);
                *e = (*e).max(at);
            }
            push(
                &mut events,
                ActivityEvent {
                    id: format!("result:{}:{rel}", r.id),
                    kind: "result".into(),
                    at: r.finished_at.clone(),
                    status: change_status(&f.change_type).into(),
                    title: title_of(rel),
                    session_key: r.session_key.clone().filter(|k| keys.contains(k.as_str())),
                    run_id: Some(r.run_id.clone()),
                    receipt_id: Some(r.id.clone()),
                    path: Some(f.path.clone()),
                    result: Some(ActivityResult {
                        name: rel.to_string(),
                        kind: workspace_results::kind_of(rel).into(),
                        exists: files.contains_key(rel),
                        actor: Some("run".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            );
        }
    }
    // A saved canvas's revisions, and what no record explains.
    for f in &input.results.files {
        for (revision, saved_at, session) in &f.canvas_revisions {
            if let Some(at) = ms(saved_at) {
                let e = explained.entry(f.rel.clone()).or_insert(at);
                *e = (*e).max(at);
            }
            push(
                &mut events,
                ActivityEvent {
                    id: format!("canvas:{}:v{revision}", f.rel),
                    kind: "result".into(),
                    at: saved_at.clone(),
                    status: if *revision <= 1 {
                        "created"
                    } else {
                        "modified"
                    }
                    .into(),
                    title: f.title.clone(),
                    session_key: session.clone().filter(|k| keys.contains(k.as_str())),
                    path: Some(f.path.clone()),
                    result: Some(ActivityResult {
                        name: f.rel.clone(),
                        kind: f.kind.into(),
                        exists: true,
                        actor: Some("chat".into()),
                        revision: Some(*revision),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            );
        }
        let outside = |status: &str, at: &str| ActivityEvent {
            id: format!("file:{}:{status}", f.rel),
            kind: "result".into(),
            at: at.to_string(),
            status: status.into(),
            title: f.title.clone(),
            path: Some(f.path.clone()),
            result: Some(ActivityResult {
                name: f.rel.clone(),
                kind: f.kind.into(),
                exists: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        let created = f.created.as_deref().and_then(ms);
        let modified = f.modified.as_deref().and_then(ms);
        // What no kept record explains: (status, when, as written).
        let mut unexplained: Vec<(&str, i64, &str)> = Vec::new();
        match explained.get(&f.rel) {
            None => {
                if let (Some(c), Some(at)) = (created, f.created.as_deref()) {
                    unexplained.push(("added", c, at));
                }
                if let (Some(m), Some(at)) = (modified, f.modified.as_deref()) {
                    if created.is_none_or(|c| m > c + SLACK_MS) {
                        unexplained.push(("edited", m, at));
                    }
                }
            }
            Some(&last) => {
                if let (Some(m), Some(at)) = (modified, f.modified.as_deref()) {
                    if m > last + SLACK_MS {
                        unexplained.push(("edited", m, at));
                    }
                }
            }
        }
        // Changes since a running turn started are one `pending` event, at
        // the latest of them; the rest say no kept record names them.
        let mut pending: Option<(i64, &str)> = None;
        for (status, t, at) in unexplained {
            if running_since.is_some_and(|since| t >= since) {
                if pending.is_none_or(|(p, _)| t > p) {
                    pending = Some((t, at));
                }
                continue;
            }
            if store_floor.is_some_and(|floor| t < floor) {
                aged.insert(f.rel.as_str());
            }
            push(&mut events, outside(status, at));
        }
        if let Some((_, at)) = pending {
            push(&mut events, outside("pending", at));
        }
    }
    gaps.results_without_receipt = aged.len();
    // Reviews: draft, ready, accepted, and who.
    for (rel, title, history) in &input.results.reviews {
        let path = format!("{}{rel}", prefix);
        for (n, e) in history.iter().enumerate() {
            let status = match e.state.as_str() {
                "accepted" => "accepted",
                "ready" => "ready",
                _ => "draft",
            };
            push(
                &mut events,
                ActivityEvent {
                    id: format!("review:{rel}:{n}"),
                    kind: "result".into(),
                    at: e.at.clone(),
                    status: status.into(),
                    title: files
                        .get(rel.as_str())
                        .map(|f| f.title.clone())
                        .or_else(|| title.clone())
                        .unwrap_or_else(|| stem(rel)),
                    run_id: e.run_id.clone(),
                    path: Some(path.clone()),
                    result: Some(ActivityResult {
                        name: rel.clone(),
                        kind: workspace_results::kind_of(rel).into(),
                        exists: files.contains_key(rel.as_str()),
                        actor: e.actor.clone().filter(|a| a == "person" || a == "agent"),
                        note: e.note.clone(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            );
        }
    }

    // Builds and installs.
    if let Some(b) = input.build.as_ref().filter(|b| b.state != BuildState::Idle) {
        let status = match b.state {
            BuildState::Building => "building",
            BuildState::Built => "built",
            BuildState::Failed | BuildState::Idle => "failed",
        };
        let at = b
            .finished_at
            .clone()
            .or_else(|| b.started_at.clone())
            .unwrap_or_else(|| input.as_of.clone());
        push(
            &mut events,
            ActivityEvent {
                id: format!("build:{}", b.started_at.as_deref().unwrap_or("")),
                kind: "install".into(),
                at,
                status: status.into(),
                title: "App build".into(),
                install: Some(ActivityInstall {
                    step: "build".into(),
                    commit: b.commit.clone(),
                    branch: b.branch.clone(),
                    summary: b.error.clone(),
                    matches_running: None,
                }),
                ..Default::default()
            },
        );
    }
    if let Some(i) = &input.install {
        push(
            &mut events,
            ActivityEvent {
                id: format!("install:{}", i.result.id),
                kind: "install".into(),
                at: i.result.finished_at.clone(),
                status: match i.state {
                    InstallState::Installed => "installed",
                    InstallState::Failed => "failed",
                    InstallState::Unknown => "unknown",
                }
                .into(),
                title: "Install and restart".into(),
                install: Some(ActivityInstall {
                    step: "install".into(),
                    commit: Some(i.result.commit.clone()).filter(|c| !c.is_empty()),
                    branch: Some(i.result.branch.clone()).filter(|b| !b.is_empty()),
                    summary: Some(i.summary.clone()),
                    matches_running: Some(i.matches_running),
                }),
                ..Default::default()
            },
        );
    }

    let mut entries = events.entries;
    entries.sort_unstable_by(|a, b| a.key.cmp(&b.key));
    Timeline { entries, gaps }
}

/// The timeline's rows while it is built: the first event with an id wins,
/// a later one with the same id is dropped.
#[derive(Default)]
struct Entries {
    entries: Vec<Entry>,
    ids: HashSet<String>,
}

impl Entries {
    fn add(&mut self, key: OrderKey, item: Item) {
        if self.ids.insert(key.id.clone()) {
            self.entries.push(Entry { key, item });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{ReceiptCanvases, ReceiptCheck, ReceiptFile};
    use crate::types::HarnessStint;
    use crate::workspace_results::{ResultFile, ReviewEvent};

    /// A retired curated section view's files (Canvas C6 · K1), left in an
    /// older workspace, are bookkeeping like a sidecar, not result events.
    #[test]
    fn curated_section_canvases_are_bookkeeping() {
        let none = HashSet::new();
        assert!(is_bookkeeping("outputs/canvases/_plan.canvas.html", &none));
        assert!(is_bookkeeping("outputs/canvases/_brief.canvas.json", &none));
        assert!(!is_bookkeeping("outputs/canvases/plan.canvas.html", &none));
        assert!(!is_bookkeeping(
            "outputs/canvases/_results.canvas.html",
            &none
        ));
    }

    const W: &str = "workspaces/w";

    fn session(key: &str, created: &str, turns: u32) -> AgentSessionSummary {
        AgentSessionSummary {
            key: key.into(),
            title: Some(format!("Conversation {key}")),
            agent: "codex".into(),
            harness_session_id: None,
            cwd: "/v/workspaces/w".into(),
            created_at: created.into(),
            last_activity_at: created.into(),
            ended_at: None,
            turn_count: turns,
            parent_key: None,
            fork_kind: None,
            adopted: false,
            stints: vec![],
            resumable: "native".into(),
            imported: None,
        }
    }

    fn turn(ord: u32, run: Option<&str>, at: &str, status: &str) -> SessionRecordTurn {
        SessionRecordTurn {
            ord,
            run_id: run.map(String::from),
            started_at: at.into(),
            ended_at: (status != "running").then(|| at.to_string()),
            status: Some(status.into()),
            asked: Some(format!("\n  Question {ord}\nmore text")),
        }
    }

    fn receipt(
        id: &str,
        run: &str,
        key: Option<&str>,
        at: &str,
        files: &[(&str, &str)],
    ) -> EvidenceReceipt {
        EvidenceReceipt {
            id: id.into(),
            run_id: run.into(),
            source: "chat".into(),
            started_at: at.into(),
            finished_at: at.into(),
            status: "success".into(),
            verification_state: "verified".into(),
            session_key: key.map(String::from),
            files: files
                .iter()
                .map(|(path, change)| ReceiptFile {
                    path: (*path).into(),
                    change_type: (*change).into(),
                    attribution: "confirmed".into(),
                    tool: None,
                    additions: None,
                    deletions: None,
                    not_recoverable: false,
                    also_changed_by: Vec::new(),
                    hollow_write: false,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn usage(run: &str, agent: &str, model: &str, cost: f64) -> UsageRow {
        UsageRow {
            run_id: Some(run.into()),
            agent: Some(agent.into()),
            model: Some(model.into()),
            cost_usd: Some(cost),
            cost_status: "reported".into(),
            ..crate::session_store::tests::usage_row_fixture(run, "2026-10-05T10:00:00Z", "success")
        }
    }

    fn file(rel: &str, created: Option<&str>, modified: &str) -> ResultFile {
        ResultFile {
            rel: rel.into(),
            path: format!("{W}/{rel}"),
            title: format!("Title of {rel}"),
            kind: workspace_results::kind_of(rel),
            created: created.map(String::from),
            modified: Some(modified.into()),
            canvas_revisions: vec![],
        }
    }

    fn inputs() -> Inputs {
        Inputs {
            workspace_id: "w".into(),
            workspace_path: W.into(),
            sessions: vec![],
            turns: HashMap::new(),
            usage: Ok(HashMap::new()),
            receipts: Ok(vec![]),
            results: ResultActivity::default(),
            build: None,
            install: None,
            as_of: "2026-10-05T12:00:00Z".into(),
        }
    }

    fn ids(a: &WorkspaceActivity) -> Vec<&str> {
        a.events.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn every_kind_newest_first_with_a_stable_order_at_the_same_instant() {
        let mut i = inputs();
        i.sessions = vec![session("s-1", "2026-10-05T09:00:00.000Z", 2)];
        i.turns.insert(
            "s-1".into(),
            vec![
                turn(2, Some("run-2"), "2026-10-05T11:00:00Z", "error"),
                turn(1, Some("run-1"), "2026-10-05T10:00:00+00:00", "success"),
            ],
        );
        // run-1's receipt finished at 10:05 (a different offset spelling)
        // and created a result; the receipt's time wins over the turn's.
        let mut r1 = receipt(
            "rc-1",
            "run-1",
            Some("s-1"),
            "2026-10-05T06:05:00-04:00",
            &[(&format!("{W}/outputs/a.md"), "create")],
        );
        r1.canvases = Some(ReceiptCanvases {
            count: 2,
            bytes: 14_000,
        });
        r1.checks = vec![ReceiptCheck {
            command: "npm test".into(),
            kind: "test".into(),
            outcome: "failed".into(),
            sequence: 1,
            excerpt: None,
        }];
        i.receipts = Ok(vec![r1]);
        i.usage
            .as_mut()
            .unwrap()
            .insert("run-1".into(), vec![usage("run-1", "claude", "opus", 0.42)]);
        i.results.files = vec![file(
            "outputs/a.md",
            Some("2026-10-05T10:04:00Z"),
            "2026-10-05T10:04:30Z",
        )];
        i.results.reviews = vec![(
            "outputs/a.md".into(),
            None,
            vec![ReviewEvent {
                state: "accepted".into(),
                sha256: Some("x".into()),
                at: "2026-10-05T11:30:00Z".into(),
                note: None,
                actor: Some("person".into()),
                run_id: None,
            }],
        )];
        let a = assemble(i, DEFAULT_LIMIT);
        assert_eq!(
            ids(&a),
            [
                "review:outputs/a.md:0",
                "run:run-2",
                "run:run-1",
                "result:rc-1:outputs/a.md",
                "session:s-1",
            ]
        );
        assert_eq!(a.total, 5);
        let run1 = a.events.iter().find(|e| e.id == "run:run-1").unwrap();
        assert_eq!(run1.at, "2026-10-05T06:05:00-04:00");
        assert_eq!(run1.status, "succeeded");
        assert_eq!(run1.title, "Question 1");
        assert_eq!(run1.receipt_id.as_deref(), Some("rc-1"));
        let run = run1.run.as_ref().unwrap();
        assert_eq!(
            (run.agent.as_deref(), run.model.as_deref(), run.cost_usd),
            (Some("claude"), Some("opus"), Some(0.42))
        );
        assert_eq!(
            (run.files, run.outputs, run.checks, run.failed_checks),
            (Some(1), Some(1), Some(1), Some(1))
        );
        assert_eq!(
            run.canvases,
            Some(ActivityCanvases {
                count: 2,
                bytes: 14_000
            })
        );
        assert_eq!(run.turn, Some(1));
        let run2 = a.events.iter().find(|e| e.id == "run:run-2").unwrap();
        assert_eq!(run2.status, "failed");
        let created = a
            .events
            .iter()
            .find(|e| e.kind == "result" && e.status == "created")
            .unwrap();
        assert_eq!(created.title, "Title of outputs/a.md");
        assert_eq!(created.session_key.as_deref(), Some("s-1"));
        assert_eq!(
            created.result.as_ref().unwrap().actor.as_deref(),
            Some("run")
        );
        let accepted = &a.events[0];
        assert_eq!(
            (
                accepted.status.as_str(),
                accepted.result.as_ref().unwrap().actor.as_deref()
            ),
            ("accepted", Some("person"))
        );
        // The file written inside the run's window has no outside event.
        assert!(!a.events.iter().any(|e| e.id.starts_with("file:")));
        // run-2 finished with neither a receipt nor a usage row.
        assert_eq!(
            a.gaps,
            ActivityGaps {
                runs_without_receipt: 1,
                runs_without_usage: 1,
                ..Default::default()
            }
        );
        // The same inputs give the same order.
        let mut again = inputs();
        again.sessions = vec![session("s-1", "2026-10-05T09:00:00Z", 0)];
        assert_eq!(ids(&assemble(again, 10)), ["session:s-1"]);
    }

    #[test]
    fn a_run_comes_before_what_it_wrote_and_ties_break_by_id() {
        let mut i = inputs();
        i.sessions = vec![session("s-1", "2026-10-05T10:00:00Z", 1)];
        i.turns.insert(
            "s-1".into(),
            vec![turn(1, Some("run-1"), "2026-10-05T10:00:00Z", "success")],
        );
        let b = format!("{W}/outputs/b.md");
        let a = format!("{W}/outputs/a.md");
        i.receipts = Ok(vec![receipt(
            "rc-1",
            "run-1",
            Some("s-1"),
            "2026-10-05T10:00:00Z",
            &[(&b, "modify"), (&a, "delete")],
        )]);
        let out = assemble(i, DEFAULT_LIMIT);
        assert_eq!(
            ids(&out),
            [
                "run:run-1",
                "result:rc-1:outputs/a.md",
                "result:rc-1:outputs/b.md",
                "session:s-1"
            ]
        );
        assert_eq!(out.events[1].status, "deleted");
        assert!(!out.events[1].result.as_ref().unwrap().exists);
        assert_eq!(out.events[2].status, "modified");
    }

    #[test]
    fn the_limit_bounds_the_list_and_total_counts_everything() {
        let mut i = inputs();
        i.sessions = (0..30)
            .map(|n| {
                session(
                    &format!("s-{n:02}"),
                    &format!("2026-10-05T10:{n:02}:00Z"),
                    0,
                )
            })
            .collect();
        let a = assemble(i, 7);
        assert_eq!(a.events.len(), 7);
        assert_eq!(a.total, 30);
        assert_eq!(a.events[0].id, "session:s-29");
        assert_eq!(a.events[6].id, "session:s-23");
    }

    #[test]
    fn an_unreadable_time_sorts_last_and_nothing_is_dropped() {
        let mut i = inputs();
        i.sessions = vec![
            session("s-bad", "not a time", 0),
            session("s-ok", "2026-01-01T00:00:00Z", 0),
        ];
        let a = assemble(i, DEFAULT_LIMIT);
        assert_eq!(ids(&a), ["session:s-ok", "session:s-bad"]);
    }

    #[test]
    fn missing_records_list_the_run_without_them_and_count_the_gap() {
        let mut i = inputs();
        let mut s = session("s-1", "2026-10-01T09:00:00Z", 3);
        // The conversation was on Codex, then moved to Claude at 10:30.
        s.stints = vec![
            HarnessStint {
                agent: "codex".into(),
                harness_session_id: None,
                started_at: "2026-10-01T09:00:00Z".into(),
                ended_at: Some("2026-10-01T10:30:00Z".into()),
                reason: None,
            },
            HarnessStint {
                agent: "claude".into(),
                harness_session_id: None,
                started_at: "2026-10-01T10:30:00Z".into(),
                ended_at: None,
                reason: None,
            },
        ];
        s.agent = "claude".into();
        i.sessions = vec![s];
        i.turns.insert(
            "s-1".into(),
            vec![
                turn(3, Some("run-3"), "2026-10-01T11:00:00Z", "running"),
                turn(2, None, "2026-10-01T10:45:00Z", "cancelled"),
                turn(1, Some("run-1"), "2026-10-01T10:00:00Z", "success"),
            ],
        );
        let a = assemble(i, DEFAULT_LIMIT);
        let run1 = a.events.iter().find(|e| e.id == "run:run-1").unwrap();
        let r = run1.run.as_ref().unwrap();
        // No receipt (aged out of the store's 500) and no usage row (400 days).
        assert_eq!(
            (r.files, r.checks, r.canvases, r.verification.as_deref()),
            (None, None, None, None)
        );
        assert_eq!(run1.receipt_id, None);
        assert_eq!((r.model.as_deref(), r.cost_usd), (None, None));
        // The agent the conversation had then, not now.
        assert_eq!(r.agent.as_deref(), Some("codex"));
        assert_eq!(run1.at, "2026-10-01T10:00:00Z");
        // A turn stopped before it had a run id still lists, by its number.
        let stopped = a.events.iter().find(|e| e.id == "turn:s-1:2").unwrap();
        assert_eq!(
            (
                stopped.status.as_str(),
                stopped.run.as_ref().unwrap().agent.as_deref()
            ),
            ("stopped", Some("claude"))
        );
        // A running turn: the conversation says so; no gap is counted for it.
        assert_eq!(
            a.events
                .iter()
                .find(|e| e.id == "run:run-3")
                .unwrap()
                .status,
            "running"
        );
        assert_eq!(
            a.events
                .iter()
                .find(|e| e.id == "session:s-1")
                .unwrap()
                .status,
            "running"
        );
        assert_eq!(a.gaps.runs_without_receipt, 1);
        assert_eq!(a.gaps.runs_without_usage, 1);
        assert!(!a.gaps.turns_truncated);
    }

    #[test]
    fn unreadable_stores_are_reported_and_the_rest_still_lists() {
        let mut i = inputs();
        i.sessions = vec![session("s-1", "2026-10-01T09:00:00Z", 1)];
        i.turns.insert(
            "s-1".into(),
            vec![turn(1, Some("run-1"), "2026-10-01T10:00:00Z", "success")],
        );
        i.receipts = Err("parse evidence receipts: expected value".into());
        i.results.store_error = Some("Results review state could not be read".into());
        i.results.files = vec![file("outputs/a.md", None, "2026-10-01T12:00:00Z")];
        let a = assemble(i, DEFAULT_LIMIT);
        assert_eq!(
            a.gaps.receipts_error.as_deref(),
            Some("parse evidence receipts: expected value")
        );
        assert_eq!(
            a.gaps.reviews_error.as_deref(),
            Some("Results review state could not be read")
        );
        // Not a run without a receipt: the store is unreadable, which is said once.
        assert_eq!(a.gaps.runs_without_receipt, 0);
        assert_eq!(
            ids(&a),
            ["file:outputs/a.md:edited", "run:run-1", "session:s-1"]
        );
    }

    #[test]
    fn an_unreadable_usage_store_is_an_error_not_runs_without_usage() {
        let mut i = inputs();
        i.sessions = vec![session("s-1", "2026-10-01T09:00:00Z", 2)];
        i.turns.insert(
            "s-1".into(),
            vec![
                turn(2, Some("run-2"), "2026-10-01T11:00:00Z", "success"),
                turn(1, Some("run-1"), "2026-10-01T10:00:00Z", "success"),
            ],
        );
        i.usage = Err("session store open: database is locked".into());
        let a = assemble(i, DEFAULT_LIMIT);
        assert_eq!(
            a.gaps.usage_error.as_deref(),
            Some("session store open: database is locked")
        );
        // Not two runs that lost their usage row: the store didn't answer.
        assert_eq!(a.gaps.runs_without_usage, 0);
        // The runs still list, without model or cost.
        assert_eq!(ids(&a), ["run:run-2", "run:run-1", "session:s-1"]);
        assert!(a.events[0].run.as_ref().unwrap().model.is_none());
        let wire = serde_json::to_value(&a.gaps).unwrap();
        assert_eq!(wire["usageError"], "session store open: database is locked");
    }

    #[test]
    fn retention_caps_are_said_not_hidden() {
        let mut i = inputs();
        // A conversation with more turns than the timeline reads.
        i.sessions = vec![session(
            "s-1",
            "2026-10-01T09:00:00Z",
            (TURNS_PER_SESSION + 1) as u32,
        )];
        i.turns.insert(
            "s-1".into(),
            (1..=TURNS_PER_SESSION as u32)
                .rev()
                .map(|n| turn(n, None, "2026-10-01T10:00:00Z", "success"))
                .collect(),
        );
        i.results.truncated = true;
        let a = assemble(i, DEFAULT_LIMIT);
        assert!(a.gaps.turns_truncated && a.gaps.results_truncated);
        assert_eq!(a.total, TURNS_PER_SESSION + 1);
        // Exactly the cap with no more turns is not truncated.
        let mut j = inputs();
        j.sessions = vec![session("s-1", "2026-10-01T09:00:00Z", 2)];
        j.turns.insert(
            "s-1".into(),
            vec![
                turn(2, None, "2026-10-01T10:00:00Z", "success"),
                turn(1, None, "2026-10-01T09:30:00Z", "success"),
            ],
        );
        assert!(!assemble(j, DEFAULT_LIMIT).gaps.turns_truncated);
    }

    #[test]
    fn files_no_record_explains_are_added_or_edited_with_no_one_named() {
        let mut i = inputs();
        let rel_new = "outputs/by-hand.md";
        let rel_edit = "outputs/run-then-edit.md";
        let rel_nobirth = "outputs/no-birth.csv";
        i.receipts = Ok(vec![receipt(
            "rc-1",
            "run-1",
            None,
            "2026-10-01T10:00:00Z",
            &[(&format!("{W}/{rel_edit}"), "create")],
        )]);
        i.results.files = vec![
            // Added and later changed by hand.
            file(
                rel_new,
                Some("2026-10-01T08:00:00Z"),
                "2026-10-01T09:00:00Z",
            ),
            // Written by run-1, changed an hour later.
            file(
                rel_edit,
                Some("2026-10-01T09:59:59Z"),
                "2026-10-01T11:00:00Z",
            ),
            // No birth time on this file system: only "changed".
            file(rel_nobirth, None, "2026-10-01T07:00:00Z"),
            // Added once, never touched again (within the slack).
            file(
                "outputs/once.md",
                Some("2026-10-01T06:00:00Z"),
                "2026-10-01T06:00:01Z",
            ),
        ];
        let a = assemble(i, DEFAULT_LIMIT);
        assert_eq!(
            ids(&a),
            [
                "file:outputs/run-then-edit.md:edited",
                // The receipt names no conversation here, but it changed a
                // workspace file: a run of its own, before what it wrote.
                "receipt:rc-1",
                "result:rc-1:outputs/run-then-edit.md",
                "file:outputs/by-hand.md:edited",
                "file:outputs/by-hand.md:added",
                "file:outputs/no-birth.csv:edited",
                "file:outputs/once.md:added",
            ]
        );
        // Nobody is named for what no record explains.
        for e in a.events.iter().filter(|e| e.id.starts_with("file:")) {
            assert_eq!(e.result.as_ref().unwrap().actor, None, "{}", e.id);
            assert_eq!((e.session_key.as_ref(), e.run_id.as_ref()), (None, None));
        }
    }

    /// Chat concurrency F1 review (red first): a reviewer's receipt that
    /// lists the writer's output as `shared` adds no result event of its
    /// own and explains nothing, so a later edit no record names still
    /// shows.
    #[test]
    fn a_shared_file_is_neither_a_result_nor_an_explanation() {
        let mut i = inputs();
        let rel = "outputs/report.md";
        let path = format!("{W}/{rel}");
        let mut reviewer = receipt(
            "rc-r",
            "run-r",
            None,
            "2026-10-01T10:30:00Z",
            &[(&path, "modify")],
        );
        reviewer.files[0].attribution = crate::evidence::ATTRIBUTION_SHARED.into();
        i.receipts = Ok(vec![
            receipt(
                "rc-w",
                "run-w",
                None,
                "2026-10-01T10:00:00Z",
                &[(&path, "create")],
            ),
            reviewer,
        ]);
        i.results.files = vec![file(
            rel,
            Some("2026-10-01T09:59:59Z"),
            "2026-10-01T10:30:01Z",
        )];
        let a = assemble(i, DEFAULT_LIMIT);
        let results: Vec<&str> = ids(&a)
            .into_iter()
            .filter(|id| id.starts_with("result:") || id.starts_with("file:"))
            .collect();
        assert_eq!(
            results,
            [
                "file:outputs/report.md:edited",
                "result:rc-w:outputs/report.md"
            ]
        );
    }

    #[test]
    fn a_file_changed_while_a_turn_runs_is_pending_not_unexplained() {
        // The turn's receipt is written when it ends, so a file it writes
        // meanwhile has no record yet; it must not read as unexplained.
        let mut i = inputs();
        i.sessions = vec![session("s-1", "2026-10-01T09:00:00Z", 2)];
        i.turns.insert(
            "s-1".into(),
            vec![
                turn(2, Some("run-2"), "2026-10-01T11:00:00Z", "running"),
                turn(1, Some("run-1"), "2026-10-01T10:00:00Z", "success"),
            ],
        );
        i.receipts = Ok(vec![receipt(
            "rc-1",
            "run-1",
            Some("s-1"),
            "2026-10-01T10:00:00Z",
            &[],
        )]);
        i.results.files = vec![
            // Born and written during the running turn: one pending event.
            file(
                "outputs/new.md",
                Some("2026-10-01T11:01:00Z"),
                "2026-10-01T11:02:00Z",
            ),
            // Born before the turn, changed during it.
            file(
                "outputs/older.md",
                Some("2026-10-01T08:00:00Z"),
                "2026-10-01T11:03:00Z",
            ),
            // Nothing during the turn: unexplained as before.
            file(
                "outputs/quiet.md",
                Some("2026-10-01T07:00:00Z"),
                "2026-10-01T07:00:00Z",
            ),
        ];
        let a = assemble(i, DEFAULT_LIMIT);
        let files: Vec<(&str, &str, &str)> = a
            .events
            .iter()
            .filter(|e| e.id.starts_with("file:"))
            .map(|e| (e.id.as_str(), e.status.as_str(), e.at.as_str()))
            .collect();
        assert_eq!(
            files,
            [
                (
                    "file:outputs/older.md:pending",
                    "pending",
                    "2026-10-01T11:03:00Z"
                ),
                (
                    "file:outputs/new.md:pending",
                    "pending",
                    "2026-10-01T11:02:00Z"
                ),
                (
                    "file:outputs/older.md:added",
                    "added",
                    "2026-10-01T08:00:00Z"
                ),
                (
                    "file:outputs/quiet.md:added",
                    "added",
                    "2026-10-01T07:00:00Z"
                ),
            ]
        );
        // Pending names no one either: the turn's record isn't written yet.
        for e in a.events.iter().filter(|e| e.status == "pending") {
            assert_eq!(e.result.as_ref().unwrap().actor, None);
            assert_eq!((e.session_key.as_ref(), e.run_id.as_ref()), (None, None));
        }
        // Once the turn has ended, the same file is unexplained again unless
        // its receipt names it.
        let mut j = inputs();
        j.sessions = vec![session("s-1", "2026-10-01T09:00:00Z", 1)];
        j.turns.insert(
            "s-1".into(),
            vec![turn(1, Some("run-2"), "2026-10-01T11:00:00Z", "success")],
        );
        j.results.files = vec![file(
            "outputs/new.md",
            Some("2026-10-01T11:01:00Z"),
            "2026-10-01T11:02:00Z",
        )];
        let b = assemble(j, DEFAULT_LIMIT);
        assert!(b.events.iter().all(|e| e.status != "pending"));
        assert!(b.events.iter().any(|e| e.id == "file:outputs/new.md:added"));
    }

    #[test]
    fn results_older_than_the_oldest_kept_receipt_are_a_counted_gap() {
        // The receipt store keeps the vault's newest receipts only: once it
        // is full, a file older than its oldest kept receipt may have been
        // written by a run whose receipt is gone, and the gap says so.
        let full: Vec<EvidenceReceipt> = (0..crate::evidence::KEEP)
            .map(|n| {
                receipt(
                    &format!("rc-{n}"),
                    &format!("run-{n}"),
                    None,
                    &format!("2026-10-02T{:02}:{:02}:00Z", n / 60 % 24, n % 60),
                    &[("elsewhere/x.md", "modify")],
                )
            })
            .collect();
        let files = vec![
            // Before the oldest kept receipt (2026-10-02T00:00): counted.
            file(
                "outputs/old.md",
                Some("2026-09-01T08:00:00Z"),
                "2026-09-02T08:00:00Z",
            ),
            file("outputs/old.csv", None, "2026-09-03T08:00:00Z"),
            // After it: the kept receipts would name a run that wrote it.
            file(
                "outputs/new.md",
                Some("2026-10-03T08:00:00Z"),
                "2026-10-03T08:00:00Z",
            ),
        ];
        let mut i = inputs();
        i.receipts = Ok(full.clone());
        i.results.files = files.clone();
        let a = assemble(i, DEFAULT_LIMIT);
        let gap = |a: &WorkspaceActivity| a.gaps.results_without_receipt;
        assert_eq!(gap(&a), 2);
        // The events still list, in record-neutral words.
        assert!(a.events.iter().any(|e| e.id == "file:outputs/old.md:added"));
        // A store that isn't full has dropped nothing: no gap.
        let mut j = inputs();
        j.receipts = Ok(full[1..].to_vec());
        j.results.files = files.clone();
        assert_eq!(gap(&assemble(j, DEFAULT_LIMIT)), 0);
        // An unreadable store is its own gap, said once.
        let mut k = inputs();
        k.receipts = Err("parse".into());
        k.results.files = files;
        assert_eq!(gap(&assemble(k, DEFAULT_LIMIT)), 0);
    }

    #[test]
    fn other_runs_that_changed_the_workspace_list_and_bookkeeping_never_does() {
        let mut i = inputs();
        let mut wf = receipt(
            "rc-wf",
            "run-wf",
            None,
            "2026-10-02T07:00:00Z",
            &[
                (&format!("{W}/outputs/brief.md"), "create"),
                (&format!("{W}/outputs/canvases/c1.canvas.json"), "modify"),
                (
                    &format!("{W}/outputs/canvases/.revisions/c1/abc.html"),
                    "create",
                ),
                (&format!("{W}/outputs/reviews/page/shot-1.png"), "create"),
                (&format!("{W}/PLAN.md"), "modify"),
            ],
        );
        wf.source = "workflow".into();
        wf.source_id = Some("morning-brief".into());
        // A run in another workspace never shows here.
        let other = receipt(
            "rc-x",
            "run-x",
            Some("s-elsewhere"),
            "2026-10-02T08:00:00Z",
            &[("workspaces/other/outputs/x.md", "create")],
        );
        i.receipts = Ok(vec![wf, other]);
        i.results.files = vec![
            file(
                "outputs/brief.md",
                Some("2026-10-02T06:59:00Z"),
                "2026-10-02T06:59:00Z",
            ),
            file(
                "outputs/reviews/page.annotations.json",
                Some("2026-10-01T00:00:00Z"),
                "2026-10-01T00:00:00Z",
            ),
        ];
        let a = assemble(i, DEFAULT_LIMIT);
        let run = a.events.iter().find(|e| e.id == "receipt:rc-wf").unwrap();
        assert_eq!(
            (run.kind.as_str(), run.title.as_str(), run.status.as_str()),
            ("run", "Workflow morning-brief", "succeeded")
        );
        assert_eq!(run.run.as_ref().unwrap().source, "workflow");
        assert_eq!(run.run.as_ref().unwrap().files, Some(5));
        let results: Vec<_> = a
            .events
            .iter()
            .filter(|e| e.id.starts_with("result:"))
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(results, ["result:rc-wf:outputs/brief.md"]);
        assert!(!a.events.iter().any(|e| e.id.contains("rc-x")));
    }

    #[test]
    fn canvas_revisions_are_result_events_from_their_conversation() {
        let mut i = inputs();
        i.sessions = vec![session("s-1", "2026-10-03T09:00:00Z", 2)];
        let mut c = file(
            "outputs/canvases/c1.canvas.html",
            Some("2026-10-03T10:00:00Z"),
            "2026-10-03T10:20:00Z",
        );
        c.canvas_revisions = vec![
            (1, "2026-10-03T10:00:00Z".into(), Some("s-1".into())),
            (2, "2026-10-03T10:20:00Z".into(), Some("s-gone".into())),
        ];
        i.results.files = vec![c];
        let a = assemble(i, DEFAULT_LIMIT);
        let v1 = a
            .events
            .iter()
            .find(|e| e.id == "canvas:outputs/canvases/c1.canvas.html:v1")
            .unwrap();
        let v2 = a
            .events
            .iter()
            .find(|e| e.id == "canvas:outputs/canvases/c1.canvas.html:v2")
            .unwrap();
        assert_eq!(
            (v1.status.as_str(), v1.session_key.as_deref()),
            ("created", Some("s-1"))
        );
        // A conversation not in this workspace is not a link.
        assert_eq!(
            (v2.status.as_str(), v2.session_key.as_deref()),
            ("modified", None)
        );
        assert_eq!(v2.result.as_ref().unwrap().revision, Some(2));
        assert_eq!(v2.result.as_ref().unwrap().kind, "canvas");
        // The sidecar explains the file: no outside event.
        assert!(!a.events.iter().any(|e| e.id.starts_with("file:")));
    }

    #[test]
    fn reviews_list_with_who_and_a_gone_file_says_so() {
        let mut i = inputs();
        i.results.reviews = vec![(
            "outputs/gone.md".into(),
            Some("The person's title".into()),
            vec![
                ReviewEvent {
                    state: "ready".into(),
                    sha256: None,
                    at: "2026-10-01T10:00:00Z".into(),
                    note: Some("Check figures".into()),
                    actor: Some("agent".into()),
                    run_id: Some("run-9".into()),
                },
                ReviewEvent {
                    state: "accepted".into(),
                    sha256: None,
                    at: "2026-10-01T11:00:00Z".into(),
                    note: None,
                    actor: None,
                    run_id: None,
                },
            ],
        )];
        let a = assemble(i, DEFAULT_LIMIT);
        assert_eq!(
            ids(&a),
            ["review:outputs/gone.md:1", "review:outputs/gone.md:0"]
        );
        let ready = &a.events[1];
        let r = ready.result.as_ref().unwrap();
        assert_eq!(
            (ready.status.as_str(), ready.title.as_str()),
            ("ready", "The person's title")
        );
        assert_eq!(
            (r.actor.as_deref(), r.note.as_deref(), r.exists),
            (Some("agent"), Some("Check figures"), false)
        );
        assert_eq!(ready.run_id.as_deref(), Some("run-9"));
        assert_eq!(ready.path.as_deref(), Some("workspaces/w/outputs/gone.md"));
        // An event recorded before `actor` existed names nobody.
        assert_eq!(a.events[0].result.as_ref().unwrap().actor, None);
    }

    #[test]
    fn builds_and_the_last_install_are_install_events() {
        let mut i = inputs();
        i.build = Some(AppBuildStatus {
            blocked: None,
            workspace_id: "w".into(),
            is_hollow: true,
            state: BuildState::Failed,
            other_job: None,
            log: None,
            log_tail: String::new(),
            started_at: Some("2026-10-04T09:00:00Z".into()),
            finished_at: Some("2026-10-04T09:20:00Z".into()),
            error: Some("cargo build failed".into()),
            commit: Some("abc1234".into()),
            branch: Some("hollow/x".into()),
            receipt: None,
        });
        let running = crate::app_menu::parse_identity(
            r#"{"commit":"63c79a889a214c368b1f52b057d20c4e2f1ad519"}"#,
            "0.0.0",
        );
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("last-result.json"),
            r#"{"id":"inst-1","ok":true,"startedAt":"s","finishedAt":"2026-10-04T10:00:00Z","commit":"63c79a889a21","branch":"hollow/x","build":{"buildId":"b1","workspaceId":"w"}}"#,
        )
        .unwrap();
        i.install = crate::self_install::receipt_for_workspace_in(dir.path(), &running, "w");
        let a = assemble(i, DEFAULT_LIMIT);
        assert_eq!(ids(&a), ["install:inst-1", "build:2026-10-04T09:00:00Z"]);
        let install = a.events[0].install.as_ref().unwrap();
        assert_eq!(
            (
                a.events[0].status.as_str(),
                install.step.as_str(),
                install.matches_running
            ),
            ("installed", "install", Some(true))
        );
        assert!(install
            .summary
            .as_deref()
            .unwrap()
            .starts_with("Installed 63c79a88"));
        let build = &a.events[1];
        assert_eq!(
            (
                build.status.as_str(),
                build.install.as_ref().unwrap().summary.as_deref()
            ),
            ("failed", Some("cargo build failed"))
        );
        // An idle build slot is no event.
        let mut j = inputs();
        j.build = Some(idle_build());
        assert!(assemble(j, DEFAULT_LIMIT).events.is_empty());
    }

    fn idle_build() -> AppBuildStatus {
        AppBuildStatus {
            blocked: None,
            workspace_id: "w".into(),
            is_hollow: true,
            state: BuildState::Idle,
            other_job: None,
            log: None,
            log_tail: String::new(),
            started_at: None,
            finished_at: None,
            error: None,
            commit: None,
            branch: None,
            receipt: None,
        }
    }

    #[test]
    fn an_install_names_its_workspace_or_is_left_out() {
        let running = crate::app_menu::parse_identity(
            r#"{"commit":"63c79a889a214c368b1f52b057d20c4e2f1ad519"}"#,
            "0.0.0",
        );
        let dir = tempfile::tempdir().unwrap();
        assert!(crate::self_install::receipt_for_workspace_in(dir.path(), &running, "w").is_none());
        // From a helper older than build receipts: no workspace, left out.
        std::fs::write(
            dir.path().join("last-result.json"),
            r#"{"id":"old","ok":true,"finishedAt":"t","commit":"63c79a88"}"#,
        )
        .unwrap();
        assert!(crate::self_install::receipt_for_workspace_in(dir.path(), &running, "w").is_none());
        // Another workspace's install is not this one's.
        std::fs::write(dir.path().join("last-result.json"), r#"{"id":"x","ok":false,"finishedAt":"t","commit":"63c79a88","build":{"workspaceId":"other"}}"#).unwrap();
        assert!(crate::self_install::receipt_for_workspace_in(dir.path(), &running, "w").is_none());
        assert!(
            crate::self_install::receipt_for_workspace_in(dir.path(), &running, "other").is_some()
        );
    }

    /// Every file under `dir`, hidden ones too, with its bytes.
    fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<_> = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| {
                (
                    e.path().display().to_string(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn list_reads_the_real_stores_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let v = root.path();
        let w = workspaces::create(v, "Activity", "Show what happened").unwrap();
        let dir = Path::new(&w.cwd);
        let key = "s-20261005-act00001";
        let first = session_store::start_turn(
            v,
            key,
            "claude",
            &w.cwd,
            "run-a1",
            "Draft the summary\nwith numbers",
        )
        .unwrap();
        session_store::finish_turn(v, key, first, "success").unwrap();
        let second =
            session_store::start_turn(v, key, "claude", &w.cwd, "run-a2", "Fix it").unwrap();
        session_store::finish_turn(v, key, second, "error").unwrap();
        std::fs::write(dir.join("outputs/summary.md"), "# Summary\n\nNumbers.").unwrap();
        let summary = format!("{}/outputs/summary.md", w.path);
        crate::evidence::persist_for_test(
            v,
            receipt(
                "rc-a1",
                "run-a1",
                Some(key),
                &chrono::Utc::now().to_rfc3339(),
                &[(&summary, "create")],
            ),
        )
        .unwrap();
        session_store::upsert_usage(v, &usage("run-a1", "claude", "claude-opus", 0.12)).unwrap();
        crate::workspace_results::set_review(
            v,
            &w.id,
            &summary,
            "ready",
            None,
            Some("Looks right"),
            &crate::workspace_results::Reviewer::Person,
        )
        .unwrap();
        // Another workspace's conversation stays out.
        let other = workspaces::create(v, "Other", "Elsewhere").unwrap();
        session_store::start_turn(v, "s-20261005-oth00001", "codex", &other.cwd, "run-o", "Hi")
            .unwrap();

        let before = (
            tree(dir),
            std::fs::read(v.join(".evidence-receipts.json")).unwrap(),
        );
        let a = list(v, &w.id, None, None).unwrap();
        let after = (
            tree(dir),
            std::fs::read(v.join(".evidence-receipts.json")).unwrap(),
        );
        assert_eq!(before, after, "viewing wrote a file");
        assert_eq!(a.workspace_id, w.id);
        let kinds: Vec<_> = a
            .events
            .iter()
            .map(|e| (e.kind.as_str(), e.status.as_str()))
            .collect();
        assert!(kinds.contains(&("session", "started")), "{kinds:?}");
        assert!(
            kinds.contains(&("run", "succeeded")) && kinds.contains(&("run", "failed")),
            "{kinds:?}"
        );
        assert!(
            kinds.contains(&("result", "created")) && kinds.contains(&("result", "ready")),
            "{kinds:?}"
        );
        assert!(!a
            .events
            .iter()
            .any(|e| e.session_key.as_deref() == Some("s-20261005-oth00001")));
        let run = a.events.iter().find(|e| e.id == "run:run-a1").unwrap();
        assert_eq!(run.title, "Draft the summary");
        assert_eq!(
            run.run.as_ref().unwrap().model.as_deref(),
            Some("claude-opus")
        );
        assert_eq!(run.receipt_id.as_deref(), Some("rc-a1"));
        let created = a.events.iter().find(|e| e.status == "created").unwrap();
        assert_eq!(created.title, "Summary");
        // run-a2 has neither a receipt nor a usage row.
        assert_eq!(
            (a.gaps.runs_without_receipt, a.gaps.runs_without_usage),
            (1, 1)
        );
        assert_eq!(a.total, a.events.len());
        // The limit.
        assert_eq!(list(v, &w.id, Some(2), None).unwrap().events.len(), 2);
        assert_eq!(list(v, &w.id, Some(0), None).unwrap().events.len(), 1);

        // A damaged review store is reported and left exactly where it is.
        let store = dir.join(".hollow/results.json");
        std::fs::write(&store, "{ not json").unwrap();
        let before = tree(dir);
        let damaged = list(v, &w.id, None, None).unwrap();
        assert!(damaged.gaps.reviews_error.is_some());
        assert!(!damaged.events.iter().any(|e| e.status == "ready"));
        assert!(damaged.events.iter().any(|e| e.status == "created"));
        assert_eq!(
            tree(dir),
            before,
            "the damaged store was moved or rewritten"
        );
        assert!(list(v, "no-such-workspace", None, None).is_err());
    }

    #[test]
    fn recent_turns_are_newest_first_and_capped_per_conversation() {
        let root = tempfile::tempdir().unwrap();
        let v = root.path();
        let w = workspaces::create(v, "Turns", "Count").unwrap();
        let key = "s-20261005-trn00001";
        for n in 0..4 {
            let ord = session_store::start_turn(
                v,
                key,
                "codex",
                &w.cwd,
                &format!("run-t{n}"),
                &"x".repeat(1000),
            )
            .unwrap();
            session_store::finish_turn(v, key, ord, "success").unwrap();
        }
        let turns =
            session_store::recent_turns_for_sessions(v, &[key.to_string(), "s-none".into()], 3)
                .unwrap();
        let got: Vec<u32> = turns[key].iter().map(|t| t.ord).collect();
        assert_eq!(got, [4, 3, 2]);
        assert_eq!(
            turns[key][0].asked.as_deref().map(|a| a.chars().count()),
            Some(240)
        );
        assert!(turns["s-none"].is_empty());
        let usage = session_store::usage_rows_for_runs(v, &["run-t0".into()]).unwrap();
        assert!(usage.is_empty());
    }

    #[test]
    fn a_reread_reads_only_what_changed() {
        let root = tempfile::tempdir().unwrap();
        let v = root.path();
        let w = workspaces::create(v, "Incremental", "Read less").unwrap();
        let keys = [
            "s-20261005-inc00001",
            "s-20261005-inc00002",
            "s-20261005-inc00003",
        ];
        for (n, key) in keys.iter().enumerate() {
            for t in 0..2 {
                let run = format!("run-inc-{n}-{t}");
                let ord =
                    session_store::start_turn(v, key, "codex", &w.cwd, &run, "Do it").unwrap();
                session_store::finish_turn(v, key, ord, "success").unwrap();
            }
        }
        session_store::upsert_usage(v, &usage("run-inc-0-0", "codex", "gpt-5", 0.1)).unwrap();
        let mut kept = None;
        let first = list_with(v, &w.id, None, None, &mut kept).unwrap();
        let reads = kept.as_ref().unwrap().reads;
        // Each conversation's turns, and its runs' usage in one query.
        assert_eq!(
            (
                reads.conversations,
                reads.usage_conversations,
                reads.usage_runs
            ),
            (3, 3, 0)
        );
        assert_eq!((reads.receipt_store, reads.timelines), (1, 1));
        assert_eq!(first.total, 3 + 6);

        // Nothing changed: nothing is read again and the answer is the same.
        let again = list_with(v, &w.id, None, None, &mut kept).unwrap();
        assert_eq!(
            kept.as_ref().unwrap().reads,
            reads,
            "an unchanged read re-read something"
        );
        assert_eq!(
            (again.events, again.total),
            (first.events.clone(), first.total)
        );

        // A new turn in one conversation: only it and its run are read.
        let ord =
            session_store::start_turn(v, keys[1], "codex", &w.cwd, "run-inc-new", "Again").unwrap();
        let running = list_with(v, &w.id, None, None, &mut kept).unwrap();
        let r = kept.as_ref().unwrap().reads;
        assert_eq!(
            (r.conversations, r.usage_conversations, r.timelines),
            (4, 4, 2)
        );
        assert_eq!(running.events[0].id, "run:run-inc-new");
        assert_eq!(running.events[0].status, "running");
        // A running turn is read again until it ends, even when nothing moved.
        session_store::finish_turn(v, keys[1], ord, "error").unwrap();
        let ended = list_with(v, &w.id, None, None, &mut kept).unwrap();
        let r = kept.as_ref().unwrap().reads;
        assert_eq!((r.conversations, r.usage_conversations), (5, 5));
        assert_eq!(ended.events[0].status, "failed");

        // A usage row that lands later for an old run comes from the
        // ledger's changes; no conversation is read again.
        session_store::upsert_usage(v, &usage("run-inc-2-1", "codex", "gpt-5-mini", 0.03)).unwrap();
        let priced = list_with(v, &w.id, None, None, &mut kept).unwrap();
        let r = kept.as_ref().unwrap().reads;
        assert_eq!(
            (r.conversations, r.usage_conversations, r.usage_runs),
            (5, 5, 0)
        );
        assert!(r.usage_changes >= 1);
        let run = priced
            .events
            .iter()
            .find(|e| e.id == "run:run-inc-2-1")
            .unwrap();
        assert_eq!(
            run.run.as_ref().unwrap().model.as_deref(),
            Some("gpt-5-mini")
        );

        // The receipt store is read again only when its file changes.
        let before = kept.as_ref().unwrap().reads.receipt_store;
        crate::evidence::persist_for_test(
            v,
            receipt(
                "rc-inc",
                "run-inc-0-1",
                Some(keys[0]),
                &chrono::Utc::now().to_rfc3339(),
                &[],
            ),
        )
        .unwrap();
        let receipted = list_with(v, &w.id, None, None, &mut kept).unwrap();
        assert_eq!(kept.as_ref().unwrap().reads.receipt_store, before + 1);
        let run = receipted
            .events
            .iter()
            .find(|e| e.id == "run:run-inc-0-1")
            .unwrap();
        assert_eq!(run.receipt_id.as_deref(), Some("rc-inc"));
        let r = kept.as_ref().unwrap().reads;
        assert_eq!((r.conversations, r.usage_runs), (5, 0));

        // Rows pruned from the ledger: every conversation's runs are read
        // again, together, and the gone row's model is gone.
        session_store::with_write(v, "prune", |tx| {
            tx.execute("DELETE FROM usage_ledger WHERE run_id = 'run-inc-0-0'", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .unwrap();
        let pruned = list_with(v, &w.id, None, None, &mut kept).unwrap();
        let r = kept.as_ref().unwrap().reads;
        assert_eq!(
            (r.conversations, r.usage_conversations, r.usage_runs),
            (5, 8, 0)
        );
        let run = pruned
            .events
            .iter()
            .find(|e| e.id == "run:run-inc-0-0")
            .unwrap();
        assert_eq!(run.run.as_ref().unwrap().model, None);

        // Another workspace replaces what was kept.
        let other = workspaces::create(v, "Other", "Elsewhere").unwrap();
        list_with(v, &other.id, None, None, &mut kept).unwrap();
        assert_eq!(kept.as_ref().unwrap().workspace_id, other.id);
        assert_eq!(kept.as_ref().unwrap().reads.timelines, 1);
    }

    #[test]
    fn pages_continue_after_the_last_event_shown_even_as_new_ones_arrive() {
        let root = tempfile::tempdir().unwrap();
        let v = root.path();
        let w = workspaces::create(v, "Paged", "Older").unwrap();
        let key = "s-20261005-pag00001";
        for t in 0..7 {
            let ord =
                session_store::start_turn(v, key, "codex", &w.cwd, &format!("run-p{t}"), "Step")
                    .unwrap();
            session_store::finish_turn(v, key, ord, "success").unwrap();
        }
        let mut kept = None;
        let whole = list_with(v, &w.id, Some(100), None, &mut kept).unwrap();
        assert_eq!((whole.total, whole.next_cursor.as_deref()), (8, None));
        let first = list_with(v, &w.id, Some(3), None, &mut kept).unwrap();
        assert_eq!(first.events, whole.events[..3]);
        let cursor = first.next_cursor.clone().expect("more pages");

        // A new turn arrives at the top between the two reads.
        let ord = session_store::start_turn(v, key, "codex", &w.cwd, "run-late", "Late").unwrap();
        session_store::finish_turn(v, key, ord, "success").unwrap();
        let mut seen: Vec<ActivityEvent> = first.events.clone();
        let mut next = Some(cursor);
        while let Some(c) = next {
            let page = list_with(v, &w.id, Some(3), Some(&c), &mut kept).unwrap();
            assert_eq!(page.total, 9);
            seen.extend(page.events);
            next = page.next_cursor;
        }
        let ids = |e: &[ActivityEvent]| e.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(
            ids(&seen),
            ids(&whole.events),
            "a page skipped or repeated an event"
        );

        // A cursor this timeline didn't give out is refused.
        for bad in ["", "not a cursor", "WyJ4Il0"] {
            assert!(
                list_with(v, &w.id, None, Some(bad), &mut kept).is_err(),
                "{bad}"
            );
        }
    }

    /// A large vault for the C6 P3 measurement: `sessions` conversations in
    /// workspace `cwd` with `turns` finished turns each (one usage row per
    /// run), receipts for the newest 500 runs (each writing one file under
    /// `outputs/`), and `files` result files. Seeded in one transaction.
    pub(crate) fn seed_large(
        v: &Path,
        w: &workspaces::Workspace,
        sessions: usize,
        turns: usize,
        files: usize,
    ) {
        let base = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let at = |s: usize, t: usize| {
            session_store::db_time(base + chrono::Duration::seconds((t * sessions + s) as i64 * 37))
        };
        session_store::with_write(v, "seed", |tx| {
            let mut sess = tx
                .prepare(
                    "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, turn_count) \
                     VALUES(?1, ?2, 'claude', ?3, ?4, ?5, ?6)",
                )
                .unwrap();
            let mut turn = tx
                .prepare(
                    "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked) \
                     VALUES(?1, ?2, ?3, ?4, ?4, 'success', ?5)",
                )
                .unwrap();
            let mut use_row = tx
                .prepare(
                    "INSERT INTO usage_ledger(run_key, at, started_at, surface, run_id, agent, model, \
                     billing, outcome, spawned, basis, input_tokens, output_tokens, cost_usd, cost_status) \
                     VALUES(?1, ?2, ?2, 'chat', ?1, 'claude', 'claude-opus', 'api', 'success', 0, \
                     'per_run', 1200, 300, 0.02, 'reported')",
                )
                .unwrap();
            for s in 0..sessions {
                let key = format!("s-20261005-big{s:05}");
                sess.execute(rusqlite::params![
                    key,
                    format!("Conversation {s}"),
                    w.cwd,
                    at(s, 0),
                    at(s, turns - 1),
                    turns as i64
                ])
                .unwrap();
                for t in 0..turns {
                    let run = format!("run-big-{s}-{t}");
                    turn.execute(rusqlite::params![
                        key,
                        (t + 1) as i64,
                        run,
                        at(s, t),
                        format!("Question {t} in conversation {s}\nwith more text")
                    ])
                    .unwrap();
                    use_row.execute(rusqlite::params![run, at(s, t)]).unwrap();
                }
            }
            Ok(())
        })
        .unwrap();
        let receipts: Vec<EvidenceReceipt> = (0..crate::evidence::KEEP)
            .map(|n| {
                let (s, t) = (n % sessions, turns - 1 - n / sessions);
                let path = format!("{}/outputs/file-{:04}.md", w.path, n % files.max(1));
                receipt(
                    &format!("rc-big-{n}"),
                    &format!("run-big-{s}-{t}"),
                    Some(&format!("s-20261005-big{s:05}")),
                    &at(s, t),
                    &[(&path, "modify")],
                )
            })
            .collect();
        crate::evidence::replace_all_for_test(v, receipts).unwrap();
        let outputs = Path::new(&w.cwd).join("outputs");
        for f in 0..files {
            std::fs::write(
                outputs.join(format!("file-{f:04}.md")),
                format!("# File {f}\n\nBody.\n"),
            )
            .unwrap();
        }
    }

    #[test]
    #[ignore = "measurement: cargo test --lib measure_activity -- --ignored --nocapture"]
    fn measure_activity_on_a_large_vault() {
        let sessions: usize = std::env::var("P3_SESSIONS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000);
        let turns: usize = std::env::var("P3_TURNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(200);
        let root = tempfile::tempdir().unwrap();
        let v = root.path();
        let w = workspaces::create(v, "Big", "Scale").unwrap();
        let seeded = std::time::Instant::now();
        seed_large(v, &w, sessions, turns, 200);
        println!(
            "seeded {sessions} conversations x {turns} turns in {:?}",
            seeded.elapsed()
        );
        let time = |what: &str, f: &mut dyn FnMut() -> usize| {
            let mut runs: Vec<(std::time::Duration, usize)> = (0..3)
                .map(|_| {
                    let start = std::time::Instant::now();
                    let n = f();
                    (start.elapsed(), n)
                })
                .collect();
            runs.sort();
            println!(
                "  {what}: median {:?} (min {:?}, max {:?}), {} items",
                runs[1].0, runs[0].0, runs[2].0, runs[1].1
            );
        };
        // The phases of `list`, each alone.
        let summary = workspaces::summary(v, &w.id).unwrap();
        let dir = workspaces::folder(v, &w.id).unwrap();
        time("sessions_in", &mut || {
            workspaces::sessions_in(v, &dir, &summary).unwrap().len()
        });
        let keys: Vec<String> = workspaces::sessions_in(v, &dir, &summary)
            .unwrap()
            .into_iter()
            .map(|s| s.key)
            .collect();
        time("recent turns", &mut || {
            session_store::recent_turns_for_sessions(v, &keys, TURNS_PER_SESSION)
                .unwrap()
                .values()
                .map(Vec::len)
                .sum()
        });
        time("receipts", &mut || {
            crate::evidence::list(v, None, None, usize::MAX)
                .unwrap()
                .len()
        });
        let runs: Vec<String> = (0..sessions)
            .flat_map(|s| (0..turns).map(move |t| format!("run-big-{s}-{t}")))
            .collect();
        time("usage rows", &mut || {
            session_store::usage_rows_for_runs(v, &runs).unwrap().len()
        });
        time("results", &mut || {
            workspace_results::activity(v, &w.id, MAX_RESULT_FILES)
                .unwrap()
                .files
                .len()
        });
        // The whole read, as the panel asks for it: a first read (nothing
        // kept), a re-read with nothing changed (the minute tick), a re-read
        // after one new turn, and the next older page.
        time("workspace_activity, first read", &mut || {
            let a = list_with(v, &w.id, None, None, &mut None).unwrap();
            assert_eq!(a.events.len(), DEFAULT_LIMIT);
            a.total
        });
        let mut kept = None;
        let first = list_with(v, &w.id, None, None, &mut kept).unwrap();
        time(
            "workspace_activity, re-read with nothing changed",
            &mut || list_with(v, &w.id, None, None, &mut kept).unwrap().total,
        );
        let key = "s-20261005-big00007";
        // A first finished turn runs retention, which writes the seeded
        // turns' transcripts once: keep that out of the timing.
        let ord =
            session_store::start_turn(v, key, "claude", &w.cwd, "run-warm", "Warm up").unwrap();
        session_store::finish_turn(v, key, ord, "success").unwrap();
        list_with(v, &w.id, None, None, &mut kept).unwrap();
        // The write is not timed, only the read after it.
        let mut after_turn: Vec<std::time::Duration> = (0..3)
            .map(|n| {
                let ord = session_store::start_turn(
                    v,
                    key,
                    "claude",
                    &w.cwd,
                    &format!("run-new-{n}"),
                    "One more",
                )
                .unwrap();
                session_store::finish_turn(v, key, ord, "success").unwrap();
                let start = std::time::Instant::now();
                list_with(v, &w.id, None, None, &mut kept).unwrap();
                start.elapsed()
            })
            .collect();
        after_turn.sort();
        println!(
            "  workspace_activity, re-read after one new turn: median {:?} (min {:?}, max {:?})",
            after_turn[1], after_turn[0], after_turn[2]
        );
        // Where a first read goes now.
        time(
            "  usage rows for every conversation, one query each 500",
            &mut || {
                session_store::usage_rows_for_sessions(v, &keys)
                    .unwrap()
                    .len()
            },
        );
        let inputs = &kept.as_ref().unwrap().inputs;
        time("  building the timeline from what was read", &mut || {
            build_timeline(inputs).entries.len()
        });
        let cursor = first.next_cursor.clone().unwrap();
        time(
            "workspace_activity, next older page (Show older)",
            &mut || {
                list_with(v, &w.id, None, Some(&cursor), &mut kept)
                    .unwrap()
                    .events
                    .len()
            },
        );
        println!("  reads kept: {:?}", kept.as_ref().unwrap().reads);
    }
}
