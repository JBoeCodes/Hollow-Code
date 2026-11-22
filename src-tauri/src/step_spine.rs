//! The step spine (master plan F6, Phase 4): each chat turn as an ordered
//! list of steps, with a resumable checkpoint at every step boundary, and
//! "retry from step n".
//!
//! **Steps.** The `steps` table (session store, [`SCHEMA`]) holds one row per
//! step of a turn: `seq` from 1, `kind` (`model`, `tool`, `delegate`,
//! `write`), the tool's name and id, start and end times, `status`
//! (`running`, `ok`, `error`, or the turn's end for one cut short),
//! `parent_seq` (a step inside an open delegate, such as a subagent's tool
//! call), `files` for a write step, and a `checkpoint`. Rows are written as
//! the stream arrives (`chat.rs`'s tool events call [`tool_started`] and
//! [`tool_finished`]; the turn's end calls [`finish_turn`]), each its own
//! small transaction, so a crash leaves every step written so far.
//! [`steps_for_turn`] reads a `running` step of a turn that is no longer live
//! as `interrupted`: the spine is consistent after a crash without a repair
//! pass. A model step is the model's work before each tool call and after
//! the last one (it closes when the next tool starts or the turn ends).
//!
//! **Files.** When a write step (`Write`, `Edit`, `MultiEdit`,
//! `NotebookEdit`, Codex's `file_change`) ends, each file it names is stored
//! in History's object store, pinned by the turn's History run
//! (`automated_history::store_step_object`), and its
//! hash recorded on the step (`absent` when the file is gone; `outside` for a
//! path outside the vault; `uncaptured` over 2 MiB or unreadable). Shell
//! commands and teammates don't name their files, so their steps are
//! "unrecorded writers".
//!
//! **Checkpoint.** Each step's checkpoint is its own compact record (tool,
//! input and result, cut short, and status); the task state up to step n is
//! the conversation's task-state record before the turn
//! (`task_state::refresh`) plus checkpoints 1..n ([`seed_prompt`]).
//!
//! **Retry from step n** ([`plan_retry`]) is exact or refused: every file the
//! turn's History run changed goes back to its version after step n (the
//! newest version a write step at or before n recorded, else the run's
//! before-version). It refuses, changing nothing, when an unrecorded writer
//! (a shell command, a teammate) ran at or before step n while the run
//! changed files, when a write went outside the vault, or when History's
//! own checks fail (a file changed since the run, another conversation
//! changed it too, a version it can't vouch for: `restore_to_versions`).
//! The restore is a History run of its own, so it can be undone. The retry
//! is a **new conversation** in the same folder, same agent and model, at the
//! original permission mode or weaker, seeded with the task state up to step
//! n and told plainly that it is a retry, never a replay.

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS steps( \
   session_key TEXT NOT NULL, \
   turn_ord INTEGER NOT NULL, \
   seq INTEGER NOT NULL, \
   run_id TEXT, \
   kind TEXT NOT NULL, \
   name TEXT, \
   tool_id TEXT, \
   parent_seq INTEGER, \
   started_at TEXT NOT NULL, \
   ended_at TEXT, \
   status TEXT NOT NULL, \
   files TEXT, \
   checkpoint TEXT, \
   PRIMARY KEY(session_key, turn_ord, seq) \
 ); \
 CREATE INDEX IF NOT EXISTS idx_steps_run ON steps(run_id);";

/// A checkpoint's texts are cut to this many characters.
const CHECKPOINT_CHARS: usize = 300;
/// The most files one write step records.
const MAX_PATHS: usize = 50;
/// History stores files up to this size (`automated_history`'s limit).
const FILE_MAX: u64 = 2 * 1024 * 1024;
/// The most steps one turn records (a runaway turn stops adding rows).
pub(crate) const MAX_STEPS: i64 = 2_000;

/// The kind of a tool step, by the tool's name (provider-neutral: Claude's
/// and Codex's names alike).
pub(crate) fn classify(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if matches!(name, "Write" | "Edit" | "MultiEdit" | "NotebookEdit") || lower == "file_change" {
        return "write";
    }
    if matches!(name, "Task" | "Agent")
        || lower.contains("ask_agent")
        || lower.contains("start_session")
        || lower.contains("send_to_session")
        || lower.contains("hollow_team")
    {
        return "delegate";
    }
    "tool"
}

/// Whether a step may have written files it doesn't name: anything that
/// isn't a recorded write, a model step or a read-only tool.
pub(crate) fn unrecorded_writer(kind: &str, name: Option<&str>) -> bool {
    match kind {
        "model" | "write" => false,
        "delegate" => true,
        _ => !name.is_some_and(crate::task_state::is_read_only),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileVersion {
    /// Vault-relative, or the path as given when `outside`.
    pub path: String,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// `captured`, `absent`, `outside` or `uncaptured`.
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub session_key: String,
    pub turn_ord: i64,
    pub seq: i64,
    pub run_id: Option<String>,
    pub kind: String,
    pub name: Option<String>,
    pub tool_id: Option<String>,
    pub parent_seq: Option<i64>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
    pub files: Vec<FileVersion>,
    pub checkpoint: Option<Value>,
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn with_write<T>(
    vault: &Path,
    f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    crate::session_store::with_write(vault, "step", f)
}

fn next_seq(tx: &rusqlite::Transaction<'_>, key: &str, ord: i64) -> Result<i64, String> {
    tx.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM steps WHERE session_key = ?1 AND turn_ord = ?2",
        params![key, ord],
        |r| r.get(0),
    )
    .map_err(|e| format!("step seq: {e}"))
}

/// The open delegate a new step nests under (the newest still running).
fn open_delegate(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    ord: i64,
) -> Result<Option<i64>, String> {
    tx.query_row(
        "SELECT seq FROM steps WHERE session_key = ?1 AND turn_ord = ?2 AND kind = 'delegate' AND status = 'running' ORDER BY seq DESC LIMIT 1",
        params![key, ord],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| format!("step parent: {e}"))
}

/// Close the turn's open model step (if any) at `at` with `status`.
fn close_model(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    ord: i64,
    at: &str,
    status: &str,
) -> Result<(), String> {
    tx.execute(
        "UPDATE steps SET ended_at = ?3, status = ?4 WHERE session_key = ?1 AND turn_ord = ?2 AND kind = 'model' AND status = 'running'",
        params![key, ord, at, status],
    )
    .map_err(|e| format!("step close: {e}"))?;
    Ok(())
}

/// Open a model step (the model's work until the next tool or the end).
fn open_model(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    ord: i64,
    run_id: &str,
    at: &str,
) -> Result<(), String> {
    let seq = next_seq(tx, key, ord)?;
    if seq > MAX_STEPS {
        return Ok(());
    }
    let parent = open_delegate(tx, key, ord)?;
    tx.execute(
        "INSERT INTO steps(session_key, turn_ord, seq, run_id, kind, parent_seq, started_at, status) VALUES(?1, ?2, ?3, ?4, 'model', ?5, ?6, 'running')",
        params![key, ord, seq, run_id, parent, at],
    )
    .map_err(|e| format!("step insert: {e}"))?;
    Ok(())
}

/// A turn's first event: open its first model step. Idempotent.
pub(crate) fn turn_started(vault: &Path, key: &str, ord: i64, run_id: &str) -> Result<(), String> {
    if ord <= 0 || !crate::agent_sessions::is_safe_session_key(key) {
        return Ok(());
    }
    with_write(vault, |tx| {
        let any: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM steps WHERE session_key = ?1 AND turn_ord = ?2",
                params![key, ord],
                |r| r.get(0),
            )
            .map_err(|e| format!("step count: {e}"))?;
        if any == 0 {
            open_model(tx, key, ord, run_id, &now())?;
        }
        Ok(())
    })
}

/// A tool call started: the model step before it ends, the tool's step
/// opens (nested under an open delegate).
pub(crate) fn tool_started(
    vault: &Path,
    key: &str,
    ord: i64,
    run_id: &str,
    tool_id: &str,
    name: &str,
    input: &Value,
) -> Result<(), String> {
    if ord <= 0 || !crate::agent_sessions::is_safe_session_key(key) {
        return Ok(());
    }
    let at = now();
    let kind = classify(name);
    let mut checkpoint = json!({
        "tool": name,
        "input": cut(&input.to_string(), CHECKPOINT_CHARS),
    });
    // Keep a display target separately: truncating the JSON input can make
    // even its first file or command unreadable in the Subagents view.
    for field in [
        "file_path",
        "path",
        "command",
        "cmd",
        "query",
        "description",
    ] {
        if let Some(text) = input.get(field).and_then(Value::as_str) {
            // Redact before the cut (Q-C3): cut first, a key starting near
            // the end is left shorter than the key patterns' minimum length
            // and the read side's redaction can no longer see it.
            let redacted = crate::claude_runner::redact_prefix(text, 180);
            checkpoint["displayTarget"] = json!(cut(&redacted, 180));
            break;
        }
    }
    // A write step's files, from its full input now: the stored event is
    // cut at 8,000 characters, so a large write's input can't be re-read.
    if kind == "write" {
        let paths: Vec<String> = named_paths(input)
            .into_iter()
            .filter(|p| p.len() <= 1_024)
            .take(MAX_PATHS)
            .collect();
        checkpoint["paths"] = json!(paths);
    }
    // OF16: whose failure lines would fail a shell call's checks (a runner
    // id or a kind), from the full input now; `tool_finished` reads the
    // full result against them (`checkFailed`, which friction counts: a
    // pipe into `tail` or `|| true` hides the failure from the exit
    // status). Scopes only: no more of the command is kept.
    if let Some(command) = crate::run_effects::shell_command(name, input) {
        let scopes = crate::evidence::check_scopes(&command);
        if !scopes.is_empty() {
            checkpoint["checkScopes"] = json!(scopes);
        }
    }
    with_write(vault, |tx| {
        close_model(tx, key, ord, &at, "ok")?;
        let seq = next_seq(tx, key, ord)?;
        if seq > MAX_STEPS {
            return Ok(());
        }
        let parent = open_delegate(tx, key, ord)?;
        tx.execute(
            "INSERT INTO steps(session_key, turn_ord, seq, run_id, kind, name, tool_id, parent_seq, started_at, status, checkpoint) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'running', ?10)",
            params![key, ord, seq, run_id, kind, name, tool_id, parent, at, checkpoint.to_string()],
        )
        .map_err(|e| format!("step insert: {e}"))?;
        Ok(())
    })
}

/// The paths a write tool's input names.
pub(crate) fn named_paths(input: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["file_path", "path", "notebook_path"] {
        if let Some(p) = input.get(key).and_then(Value::as_str) {
            out.push(p.to_string());
        }
    }
    if let Some(changes) = input.get("changes").and_then(Value::as_array) {
        for c in changes {
            if let Some(p) = c.get("path").and_then(Value::as_str) {
                out.push(p.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// A vault-relative path for `raw` (absolute, or relative to `cwd`), or
/// `None` when it is outside the vault.
pub(crate) fn vault_relative(vault: &Path, cwd: &Path, raw: &str) -> Option<String> {
    let joined = if Path::new(raw).is_absolute() {
        Path::new(raw).to_path_buf()
    } else {
        cwd.join(raw)
    };
    let canon_vault = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    // The file may be gone: canonicalize its folder.
    let canon = match (joined.parent(), joined.file_name()) {
        (Some(dir), Some(name)) => dir
            .canonicalize()
            .map(|d| d.join(name))
            .unwrap_or_else(|_| joined.clone()),
        _ => joined.clone(),
    };
    let rel = canon.strip_prefix(&canon_vault).ok()?;
    let text = rel.to_string_lossy().replace('\\', "/");
    (!text.is_empty() && !text.split('/').any(|part| part == "..")).then_some(text)
}

/// Each named file's version now, stored in History's object store and
/// pinned by the turn's History run `run_id`.
#[cfg(test)]
pub(crate) fn capture(vault: &Path, cwd: &Path, run_id: &str, input: &Value) -> Vec<FileVersion> {
    capture_paths(vault, cwd, run_id, named_paths(input))
}

/// [`capture`] for paths already taken from the input.
pub(crate) fn capture_paths(
    vault: &Path,
    cwd: &Path,
    run_id: &str,
    paths: Vec<String>,
) -> Vec<FileVersion> {
    paths
        .into_iter()
        .map(|raw| {
            let Some(rel) = vault_relative(vault, cwd, &raw) else {
                return FileVersion {
                    path: raw,
                    hash: None,
                    size: None,
                    state: "outside".into(),
                };
            };
            let full = vault.join(&rel);
            match std::fs::symlink_metadata(&full) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileVersion {
                    path: rel,
                    hash: None,
                    size: None,
                    state: "absent".into(),
                },
                Ok(m) if m.is_file() && m.len() <= FILE_MAX => {
                    match std::fs::read(&full)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| {
                            crate::automated_history::store_step_object(vault, run_id, &bytes)
                                .map(|h| (h, bytes.len() as u64))
                        }) {
                        Ok((hash, size)) => FileVersion {
                            path: rel,
                            hash: Some(hash),
                            size: Some(size),
                            state: "captured".into(),
                        },
                        Err(_) => FileVersion {
                            path: rel,
                            hash: None,
                            size: None,
                            state: "uncaptured".into(),
                        },
                    }
                }
                _ => FileVersion {
                    path: rel,
                    hash: None,
                    size: None,
                    state: "uncaptured".into(),
                },
            }
        })
        .collect()
}

/// A tool call ended: its step closes with its result in the checkpoint
/// (and, for a write, the files' versions now); the next model step opens.
/// `check_outcome`: the receipt's outcome of the checks a shell call holds,
/// read from its full result (`EvidenceCollector::check_outcome`, OF16
/// re-review), kept as `checkOutcome` for the task-state record.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tool_finished(
    vault: &Path,
    cwd: &Path,
    key: &str,
    ord: i64,
    run_id: &str,
    tool_id: &str,
    content: &str,
    is_error: bool,
    check_outcome: Option<&str>,
) -> Result<(), String> {
    if ord <= 0 || !crate::agent_sessions::is_safe_session_key(key) {
        return Ok(());
    }
    let at = now();
    let open: Option<(i64, String, Option<String>)> = crate::session_store::with_read(
        vault,
        |conn| {
            conn.query_row(
            "SELECT seq, kind, checkpoint FROM steps WHERE session_key = ?1 AND turn_ord = ?2 AND tool_id = ?3 AND status = 'running' ORDER BY seq DESC LIMIT 1",
            params![key, ord, tool_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("step lookup: {e}"))
        },
    )?;
    let Some((seq, kind, checkpoint)) = open else {
        return Ok(());
    };
    let mut checkpoint: Value = checkpoint
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| json!({}));
    let status = if is_error { "error" } else { "ok" };
    // R6: the full result's size, before the cut, for the usage ring's
    // "What's using it" (`context_use`). A size only, never more text.
    checkpoint["resultBytes"] = json!(content.len());
    checkpoint["result"] = json!(cut(content, CHECKPOINT_CHARS));
    checkpoint["status"] = json!(status);
    if !is_error && masked_check_failed(&checkpoint, content) {
        checkpoint["checkFailed"] = json!(true);
    }
    if let Some(outcome) = check_outcome {
        checkpoint["checkOutcome"] = json!(outcome);
    }
    // Capture outside the database transaction: reading files and writing
    // History objects must not hold the session store's write lock.
    let files = if kind == "write" {
        let paths: Vec<String> = checkpoint
            .get("paths")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        capture_paths(vault, cwd, run_id, paths)
    } else {
        Vec::new()
    };
    let files_json = (!files.is_empty()).then(|| serde_json::to_string(&files).unwrap_or_default());
    with_write(vault, |tx| {
        tx.execute(
            "UPDATE steps SET ended_at = ?4, status = ?5, files = ?6, checkpoint = ?7 WHERE session_key = ?1 AND turn_ord = ?2 AND seq = ?3",
            params![key, ord, seq, at, status, files_json, checkpoint.to_string()],
        )
        .map_err(|e| format!("step finish: {e}"))?;
        open_model(tx, key, ord, run_id, &at)
    })
}

/// Whether a clean exit's full `content` holds the failure lines of one of
/// its checks (the checkpoint's `checkScopes`, OF16).
fn masked_check_failed(checkpoint: &Value, content: &str) -> bool {
    let Some(scopes) = checkpoint.get("checkScopes").and_then(Value::as_array) else {
        return false;
    };
    for scope in scopes.iter().filter_map(Value::as_str) {
        if crate::evidence::failed_by_text(scope, content) {
            return true;
        }
    }
    false
}

/// The turn ended with `status`: every step still running ends with it
/// (an `ok` model step when the turn succeeded).
pub(crate) fn finish_turn(vault: &Path, key: &str, ord: i64, status: &str) -> Result<(), String> {
    if ord <= 0 || !crate::agent_sessions::is_safe_session_key(key) {
        return Ok(());
    }
    let at = now();
    let end = if status == "success" { "ok" } else { status };
    with_write(vault, |tx| {
        close_model(tx, key, ord, &at, end)?;
        tx.execute(
            "UPDATE steps SET ended_at = ?3, status = ?4 WHERE session_key = ?1 AND turn_ord = ?2 AND status = 'running'",
            params![key, ord, at, if status == "success" { "unconfirmed" } else { status }],
        )
        .map_err(|e| format!("step finish turn: {e}"))?;
        Ok(())
    })
}

fn row_to_step(r: &rusqlite::Row<'_>) -> rusqlite::Result<Step> {
    let files: Option<String> = r.get(11)?;
    let checkpoint: Option<String> = r.get(12)?;
    Ok(Step {
        session_key: r.get(0)?,
        turn_ord: r.get(1)?,
        seq: r.get(2)?,
        run_id: r.get(3)?,
        kind: r.get(4)?,
        name: r.get(5)?,
        tool_id: r.get(6)?,
        parent_seq: r.get(7)?,
        started_at: r.get(8)?,
        ended_at: r.get(9)?,
        status: r.get(10)?,
        files: files
            .and_then(|f| serde_json::from_str(&f).ok())
            .unwrap_or_default(),
        checkpoint: checkpoint.and_then(|c| serde_json::from_str(&c).ok()),
    })
}

const COLUMNS: &str = "session_key, turn_ord, seq, run_id, kind, name, tool_id, parent_seq, started_at, ended_at, status, files, checkpoint";

/// A turn's steps in order. A step still `running` in a turn that is no
/// longer live reads as `interrupted` (`live`: whether the turn's run is).
pub(crate) fn steps_for_turn(
    vault: &Path,
    key: &str,
    ord: i64,
    live: bool,
) -> Result<Vec<Step>, String> {
    let mut steps = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM steps WHERE session_key = ?1 AND turn_ord = ?2 ORDER BY seq ASC"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![key, ord], row_to_step)
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect::<Vec<_>>())
    })?;
    if !live {
        for s in steps.iter_mut().filter(|s| s.status == "running") {
            s.status = "interrupted".into();
        }
    }
    Ok(steps)
}

/// The earlier turns [`earlier_reads`] looks back over.
const EARLIER_TURNS: i64 = 20;
/// The most steps [`earlier_reads`] reads.
const EARLIER_STEPS: i64 = 2_000;

/// The reads (`cite_check::is_reader`: Read, Grok's `read_file`,
/// `read_source` and shell calls) that succeeded in the newest
/// [`EARLIER_TURNS`] turns of conversation `key` before turn `ord`, as
/// `(tool, input)` from their checkpoints (repo features R5: the receipt's
/// citation check calls a file one of them read `earlier`). An input cut at
/// [`CHECKPOINT_CHARS`] that no longer parses is left out. Best effort:
/// empty on any error.
pub(crate) fn earlier_reads(vault: &Path, key: &str, ord: i64) -> Vec<(String, Value)> {
    if ord <= 1 || !crate::agent_sessions::is_safe_session_key(key) {
        return Vec::new();
    }
    let rows: Vec<(String, Option<String>)> = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT name, checkpoint FROM steps WHERE session_key = ?1 AND kind = 'tool' \
                 AND status = 'ok' AND name IS NOT NULL AND turn_ord < ?2 AND turn_ord >= ?2 - ?3 \
                 ORDER BY turn_ord DESC, seq ASC LIMIT ?4",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![key, ord, EARLIER_TURNS, EARLIER_STEPS], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    })
    .unwrap_or_default();
    rows.into_iter()
        .filter_map(|(name, checkpoint)| {
            if !crate::cite_check::is_reader(&name) {
                return None;
            }
            let checkpoint: Value = serde_json::from_str(&checkpoint?).ok()?;
            let input: Value = serde_json::from_str(checkpoint.get("input")?.as_str()?).ok()?;
            Some((name, input))
        })
        .collect()
}

/// The conversation and turn a run id ran as.
pub(crate) fn turn_of_run(vault: &Path, run_id: &str) -> Result<Option<(String, i64)>, String> {
    crate::session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT session_key, ord FROM turns WHERE run_id = ?1 ORDER BY id DESC LIMIT 1",
            params![run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("turn of run: {e}"))
    })
}

// ── Retry from step n ───────────────────────────────────────────────────

/// What a retry from step `n` would put back, or why it can't be exact.
/// `changed`: the History run's changes. Pure: the caller reads the steps
/// and the History record.
pub(crate) fn wanted_versions(
    steps: &[Step],
    n: i64,
    changed: &[Changed],
) -> Result<Vec<crate::automated_history::WantedVersion>, String> {
    if n < 0 || (n > 0 && !steps.iter().any(|s| s.seq == n)) {
        return Err(format!("This turn has no step {n}."));
    }
    if let Some(s) = steps
        .iter()
        .find(|s| s.files.iter().any(|f| f.state == "outside"))
    {
        return Err(format!(
            "Step {} wrote outside the vault, which History doesn't keep, so files can't be put back exactly. Nothing changed.",
            s.seq
        ));
    }
    if !changed.is_empty() {
        if let Some(s) = steps
            .iter()
            .filter(|s| s.seq <= n)
            .find(|s| unrecorded_writer(&s.kind, s.name.as_deref()))
        {
            return Err(format!(
                "Step {} ({}) may have changed files without naming them, so the files after step {n} aren't known exactly. Retry from an earlier step, or from the start. Nothing changed.",
                s.seq,
                s.name.as_deref().unwrap_or(&s.kind)
            ));
        }
    }
    // A write step at or before n that recorded no files (its result never
    // arrived, or it named none): what it wrote is unknown.
    if let Some(s) = steps
        .iter()
        .filter(|s| s.seq <= n && s.kind == "write")
        .find(|s| s.files.is_empty())
    {
        return Err(format!(
            "Step {} ({}) recorded no files (its result never arrived, or it named none), so the files after step {n} aren't known exactly. Retry from an earlier step. Nothing changed.",
            s.seq,
            s.name.as_deref().unwrap_or("write")
        ));
    }
    // A file written by step n that the run's record doesn't list ended
    // where it began, so History holds no version to check it against.
    if let Some(f) = steps
        .iter()
        .filter(|s| s.seq <= n)
        .flat_map(|s| s.files.iter())
        .find(|f| !changed.iter().any(|c| c.path == f.path))
    {
        return Err(format!(
            "{} was written by step {n} but the turn's History record doesn't list it (it ended as it began), so it can't be put back to step {n} exactly. Nothing changed.",
            f.path
        ));
    }
    let mut out = Vec::new();
    for Changed {
        path,
        before_hash,
        before_size,
        before_restorable,
        ..
    } in changed
    {
        let recorded = steps
            .iter()
            .filter(|s| s.seq <= n)
            .flat_map(|s| s.files.iter())
            .rfind(|f| &f.path == path);
        let (hash, size) = match recorded {
            Some(f) if f.state == "captured" => (f.hash.clone(), f.size),
            Some(f) if f.state == "absent" => (None, None),
            Some(_) => {
                return Err(format!(
                    "History holds no copy of {path} as it was after step {n} (too large or unreadable). Nothing changed."
                ))
            }
            // The run's own before-version, only when History has it (a
            // file over 2 MiB or unreadable is not "absent").
            None if *before_restorable => (before_hash.clone(), *before_size),
            None => {
                return Err(format!(
                    "History holds no copy of {path} as it was before the turn (too large or unreadable). Nothing changed."
                ))
            }
        };
        out.push(crate::automated_history::WantedVersion {
            vault_path: path.clone(),
            hash,
            size,
        });
    }
    Ok(out)
}

/// One change of the turn's History run, as a retry needs it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Changed {
    pub path: String,
    pub before_hash: Option<String>,
    pub before_size: Option<u64>,
    /// History holds the before-version (or it was absent).
    pub before_restorable: bool,
    pub after_hash: Option<String>,
}

/// The retry's first message: a fresh turn, seeded with the task state
/// before the turn and the turn's steps 1..n. Never a replay.
/// How a retry's seed opens: a `main_agent::WAKE_PREFIX` message that is
/// the new conversation's task, not a check-in (`main_agent::is_wake_message`).
pub(crate) const RETRY_OPEN: &str = "⟲ Hollow: this is a retry from step";

pub(crate) fn seed_prompt(
    session_key: &str,
    ord: i64,
    asked: &str,
    task_state: &str,
    steps: &[Step],
    n: i64,
    restored: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    for s in steps.iter().filter(|s| s.seq <= n && s.kind != "model") {
        let c = s.checkpoint.clone().unwrap_or(Value::Null);
        lines.push(format!(
            "{}. {} {}: {} → {}{}",
            s.seq,
            s.kind,
            s.name.as_deref().unwrap_or(""),
            c.get("input").and_then(Value::as_str).unwrap_or(""),
            c.get("status").and_then(Value::as_str).unwrap_or(&s.status),
            c.get("result")
                .and_then(Value::as_str)
                .map(|r| format!(" ({})", cut(r, 160)))
                .unwrap_or_default()
        ));
    }
    let files = match restored {
        Some(run) => format!("The files that turn changed are back as they were after step {n} (History run {run}, which can be undone)."),
        None => format!("The files that turn changed needed no change to be as they were after step {n}."),
    };
    format!(
        "{RETRY_OPEN} {n} of conversation {session_key}, turn {ord}. It is a fresh conversation, not a replay: the earlier attempt's later steps did not carry over. {files}\n\nThe original request:\n{asked}\n\n{task_state}\n\nSteps 1 to {n} of that turn, already done (don't repeat them):\n{}\n\nContinue the work from after step {n}.",
        if lines.is_empty() { "(none)".to_string() } else { lines.join("\n") }
    )
}

/// Everything a retry needs, checked; nothing changed yet.
pub(crate) struct RetryPlan {
    pub session_key: String,
    pub turn_ord: i64,
    pub run_id: String,
    pub cwd: String,
    pub agent: String,
    pub model: Option<String>,
    pub mode: String,
    pub wanted: Vec<crate::automated_history::WantedVersion>,
    pub asked: String,
    pub steps: Vec<Step>,
    /// How many of `wanted` differ from how the run left them.
    pub differs: usize,
}

pub(crate) fn plan_retry(
    vault: &Path,
    run_id: &str,
    n: i64,
    mode: Option<&str>,
    ceiling: Option<&str>,
) -> Result<RetryPlan, String> {
    let (key, ord) =
        turn_of_run(vault, run_id)?.ok_or_else(|| format!("No chat turn ran as {run_id}."))?;
    if crate::automated_history::run_is_live(vault, run_id) {
        return Err("That turn is still running. Nothing changed.".into());
    }
    let steps = steps_for_turn(vault, &key, ord, false)?;
    let history = crate::automated_history::get_run(vault, run_id)?;
    let changed: Vec<Changed> = history
        .changes
        .iter()
        .map(|c| Changed {
            path: c.vault_path.clone(),
            before_hash: c.before_hash.clone(),
            before_size: c.before_size,
            before_restorable: c.coverage_before().restorable(),
            after_hash: c.after_hash.clone(),
        })
        .collect();
    let wanted = wanted_versions(&steps, n, &changed)?;
    // The files that must change (the rest stay as the run left them).
    let differs = wanted
        .iter()
        .filter(|w| {
            changed
                .iter()
                .find(|c| c.path == w.vault_path)
                .is_none_or(|c| c.after_hash != w.hash)
        })
        .count();
    let session = crate::session_store::read_session(vault, &key)?;
    let original = crate::session_store::turn_permission_for_run(vault, &key, run_id)?
        .unwrap_or_else(|| "plan".into());
    let mode = match mode {
        Some(m) if crate::app_mcp::perm_rank(m) > crate::app_mcp::perm_rank(&original) => {
            return Err(format!(
                "A retry runs at the original mode ({original}) or weaker, not {m}. Nothing changed."
            ))
        }
        Some(m) => m.to_string(),
        None => original,
    };
    // Never stronger than the asking agent's own mode.
    let mode = match ceiling {
        Some(c) if crate::app_mcp::perm_rank(&mode) > crate::app_mcp::perm_rank(c) => c.to_string(),
        _ => mode,
    };
    let asked = session
        .turns
        .iter()
        .find(|t| t.ord as i64 == ord)
        .and_then(|t| t.asked.clone())
        .unwrap_or_default();
    Ok(RetryPlan {
        session_key: key,
        turn_ord: ord,
        run_id: run_id.to_string(),
        cwd: session.summary.cwd.clone(),
        agent: session.summary.agent.clone(),
        model: run_model(vault, run_id),
        mode,
        wanted,
        asked,
        steps,
        differs,
    })
}

/// The model the run asked for (its own ledger row), so the retry runs the
/// same one; `None` keeps the agent's default.
fn run_model(vault: &Path, run_id: &str) -> Option<String> {
    crate::session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT COALESCE(requested_model, model) FROM usage_ledger WHERE run_id = ?1 AND surface != 'delegate' ORDER BY id ASC LIMIT 1",
            params![run_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .filter(|m| !m.trim().is_empty())
}

/// Retry from step n: restore (only when files must change), then start a
/// new conversation seeded with the task state. `from_agent` (the asking
/// agent's permission mode, the retry's ceiling): an agent's own turn holds
/// the vault, so a retry that must restore files refuses and asks the
/// person (Timeline → Retry from this step).
pub(crate) async fn retry_from_step(
    app: Option<&tauri::AppHandle>,
    config: &crate::config::Config,
    run_id: &str,
    n: i64,
    mode: Option<&str>,
    from_agent: Option<&str>,
    depth: u32,
) -> Result<Value, String> {
    let vault = config.vault_path.clone();
    let plan = {
        let vault = vault.clone();
        let run_id = run_id.to_string();
        let mode = mode.map(str::to_string);
        let ceiling = from_agent.map(str::to_string);
        tauri::async_runtime::spawn_blocking(move || {
            plan_retry(&vault, &run_id, n, mode.as_deref(), ceiling.as_deref())
        })
        .await
        .map_err(|e| e.to_string())??
    };
    if plan.differs > 0 && from_agent.is_some() {
        return Err(format!(
            "Retrying from step {n} puts {} file(s) back first, which can't run while a conversation holds the vault. Ask the person to use Retry from this step in the turn's Timeline. Nothing changed.",
            plan.differs
        ));
    }
    // The person's path always goes through History, which also checks the
    // files nobody needs to change are still as the run left them.
    let restored = if from_agent.is_none() && !plan.wanted.is_empty() {
        let vault = vault.clone();
        let run_id = plan.run_id.clone();
        let wanted = plan.wanted.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::automated_history::restore_to_versions(&vault, &run_id, &wanted)
        })
        .await
        .map_err(|e| e.to_string())??
    } else {
        None
    };
    let put_back = restored.as_ref().map_or(0, |r| r.changes.len());
    let restored = restored.map(|r| r.id);
    let state = crate::task_state::refresh(&vault, &plan.session_key, plan.turn_ord)
        .ok()
        .map(|s| {
            crate::task_state::render(
                &s.record,
                &plan.session_key,
                plan.turn_ord,
                crate::task_state::Reader::AppTools,
            )
            .text
        })
        .unwrap_or_default();
    let prompt = seed_prompt(
        &plan.session_key,
        plan.turn_ord,
        &plan.asked,
        &state,
        &plan.steps,
        n,
        restored.as_deref(),
    );
    let input = crate::commands::ChatSendInput {
        prompt,
        session_id: None,
        session_key: None,
        cwd: Some(plan.cwd.clone()),
        model: plan.model.clone(),
        permission_mode: Some(plan.mode.clone()),
        agent: Some(plan.agent.clone()),
        client_turn_id: None,
        app_depth: depth,
        ultracode: false,
        canvases: None,
        continued_from: Vec::new(),
        unattended: false,
        typed: false,
        host_asker: None,
        canned: None,
        worker: false,
    };
    let accepted = match crate::commands::send_chat(app, config, input).await {
        Ok(accepted) => accepted,
        Err(e) => {
            return Err(match &restored {
                Some(run) => format!("{put_back} file(s) were put back to step {n} (History run {run}, which can be undone in History), but the new conversation didn't start: {e}"),
                None => e,
            })
        }
    };
    Ok(json!({
        "sessionKey": accepted.session_key,
        "queueId": accepted.queue_id,
        "restoredBy": restored,
        "filesPutBack": put_back,
        "mode": plan.mode,
        "note": "A new conversation seeded with the task state up to that step; not a replay.",
    }))
}

/// What fed step `seq` (the flight recorder's "what led here", master plan
/// #15): the model step that chose it (the newest model step before it at
/// the same nesting), the delegate it runs inside, and every earlier write
/// step that recorded a file its input names. Deterministic; no model.
pub(crate) fn fed_by(steps: &[Step], seq: i64) -> Vec<i64> {
    let Some(target) = steps.iter().find(|s| s.seq == seq) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(m) = steps
        .iter()
        .filter(|s| s.seq < seq && s.kind == "model" && s.parent_seq == target.parent_seq)
        .max_by_key(|s| s.seq)
    {
        out.push(m.seq);
    }
    if let Some(p) = target.parent_seq {
        out.push(p);
    }
    let input = target
        .checkpoint
        .as_ref()
        .and_then(|c| c.get("input"))
        .and_then(Value::as_str)
        .unwrap_or("");
    for w in steps.iter().filter(|s| s.seq < seq && s.kind == "write") {
        let named = w.files.iter().any(|f| {
            let base = f.path.rsplit('/').next().unwrap_or(&f.path);
            !base.is_empty() && (input.contains(&f.path) || input.contains(base))
        });
        if named {
            out.push(w.seq);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The hashed account labels a run's own ledger rows carry (more than one
/// when it failed over), never a token.
fn run_accounts(vault: &Path, run_id: &str) -> Vec<String> {
    crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT DISTINCT account_ref FROM usage_ledger WHERE run_id = ?1 AND account_ref IS NOT NULL AND surface != 'delegate' ORDER BY id ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![run_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    })
    .unwrap_or_default()
}

/// `runs.steps`: one run's steps, or a conversation turn's.
pub(crate) fn steps_view(
    vault: &Path,
    run_id: Option<&str>,
    session: Option<&str>,
    turn: Option<i64>,
) -> Result<Value, String> {
    let (key, ord, run) = match (run_id, session, turn) {
        (Some(run), _, _) => {
            let (k, o) =
                turn_of_run(vault, run)?.ok_or_else(|| format!("No chat turn ran as {run}."))?;
            (k, o, Some(run.to_string()))
        }
        (None, Some(k), Some(o)) => (k.to_string(), o, None),
        _ => return Err("Give runId, or session and turn.".into()),
    };
    let run = run.or_else(|| {
        crate::session_store::read_session(vault, &key)
            .ok()
            .and_then(|d| d.turns.into_iter().find(|t| t.ord as i64 == ord))
            .and_then(|t| t.run_id)
    });
    let live = run
        .as_deref()
        .is_some_and(|r| crate::automated_history::run_is_live(vault, r));
    let steps = steps_for_turn(vault, &key, ord, live)?;
    let first_error = steps
        .iter()
        .find(|s| matches!(s.status.as_str(), "error" | "interrupted" | "cancelled"))
        .map(|s| s.seq);
    let fed = first_error
        .map(|seq| fed_by(&steps, seq))
        .unwrap_or_default();
    let accounts = run
        .as_deref()
        .map(|r| run_accounts(vault, r))
        .unwrap_or_default();
    Ok(json!({
        "sessionKey": key,
        "turn": ord,
        "runId": run,
        "live": live,
        "steps": steps,
        "firstError": first_error,
        "fedBy": fed,
        "accounts": accounts,
        "note": "Steps are recorded as the turn streams; per-step cost isn't known yet (the run's cost is on its receipt). A retry is a new conversation seeded with the task state up to the step, never a replay.",
    }))
}

#[tauri::command]
pub async fn run_steps(
    run_id: Option<String>,
    session: Option<String>,
    turn: Option<i64>,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Value, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        steps_view(&vault, run_id.as_deref(), session.as_deref(), turn)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The person's Retry from this step (Timeline). It may put files back.
#[tauri::command]
pub async fn retry_run_from_step(
    app: tauri::AppHandle,
    run_id: String,
    step: i64,
    mode: Option<String>,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Value, String> {
    let config = state.config.clone();
    retry_from_step(Some(&app), &config, &run_id, step, mode.as_deref(), None, 0).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automated_history::WantedVersion;

    fn step(seq: i64, kind: &str, name: Option<&str>, files: Vec<FileVersion>) -> Step {
        Step {
            session_key: "s-1".into(),
            turn_ord: 1,
            seq,
            run_id: Some("run-1".into()),
            kind: kind.into(),
            name: name.map(String::from),
            tool_id: None,
            parent_seq: None,
            started_at: "2026-10-09T05:00:00Z".into(),
            ended_at: None,
            status: "ok".into(),
            files,
            checkpoint: None,
        }
    }

    fn ch(path: &str, before: Option<&str>, size: Option<u64>) -> Changed {
        Changed {
            path: path.into(),
            before_hash: before.map(String::from),
            before_size: size,
            before_restorable: true,
            after_hash: Some("after".into()),
        }
    }

    #[test]
    fn a_write_with_no_recorded_files_or_an_uncopied_before_refuses() {
        // Review finding 1: a write whose result never arrived records no
        // files; retrying past it would trash or revert what it wrote.
        let mut lost = step(1, "write", Some("Write"), vec![]);
        lost.status = "interrupted".into();
        let changed = vec![ch("w/big.md", None, None)];
        let err = wanted_versions(&[lost], 1, &changed).unwrap_err();
        assert!(err.contains("recorded no files"), "{err}");
        // Review finding 2: a before-version History couldn't copy is not
        // "absent": no trashing a 3 MB file.
        let mut big = ch("w/big.md", None, None);
        big.before_restorable = false;
        let err = wanted_versions(&[], 0, &[big]).unwrap_err();
        assert!(err.contains("before the turn"), "{err}");
    }

    /// Repo features R5: the citation check's earlier reads are the Read,
    /// `read_source` and shell calls that succeeded in earlier turns only.
    /// Delta review: Grok's `read_file` and `run_terminal_command` too.
    #[test]
    fn earlier_reads_are_the_earlier_turns_successful_reads() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-spine-earlier".to_string();
        crate::session_store::start_turn(v, &key, "claude", "/vault", "run-e1", "Look").unwrap();
        let read = json!({ "file_path": "/p/src/a.rs", "offset": 10, "limit": 5 });
        let shell = json!({ "command": "sed -n '1,9p' src/b.rs" });
        let grok_read = json!({ "target_file": "/p/src/c.rs" });
        let grok_shell = json!({ "command": "cat src/d.rs" });
        let calls = [
            (1, "t1", "Read", read.clone(), false),
            (1, "t2", "Read", json!({ "file_path": "/p/gone.rs" }), true),
            (1, "t3", "Grep", json!({ "pattern": "x" }), false),
            (2, "t4", "Bash", shell.clone(), false),
            (2, "t6", "read_file", grok_read.clone(), false),
            (2, "t7", "run_terminal_command", grok_shell.clone(), false),
            (3, "t5", "Read", json!({ "file_path": "/p/now.rs" }), false),
        ];
        for (ord, id, name, input, failed) in &calls {
            tool_started(v, &key, *ord, "run-e1", id, name, input).unwrap();
            tool_finished(v, v, &key, *ord, "run-e1", id, "x", *failed, None).unwrap();
        }
        let got = earlier_reads(v, &key, 3);
        assert_eq!(
            got,
            [
                ("Bash".to_string(), shell),
                ("read_file".to_string(), grok_read),
                ("run_terminal_command".to_string(), grok_shell),
                ("Read".to_string(), read)
            ],
            "{got:?}"
        );
        assert!(earlier_reads(v, &key, 1).is_empty());
    }

    #[test]
    fn a_write_step_takes_its_paths_from_the_full_input() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-spine-3".to_string();
        let history = crate::automated_history::begin_run(v, "chat", Some(&key)).unwrap();
        let run = history.id.clone();
        crate::session_store::start_turn(v, &key, "claude", "/vault", &run, "Write big").unwrap();
        std::fs::create_dir_all(v.join("w")).unwrap();
        let big = "x".repeat(20_000);
        std::fs::write(v.join("w/big.md"), &big).unwrap();
        let input = json!({"file_path": v.join("w/big.md").to_string_lossy(), "content": big});
        // The stored event is cut (over 8,000 characters): the spine must
        // not need it.
        crate::session_store::append_event(
            v,
            &key,
            1,
            "toolUse",
            &json!({"id": "t1", "name": "Write", "input": input}).to_string(),
        )
        .unwrap();
        tool_started(v, &key, 1, &run, "t1", "Write", &input).unwrap();
        tool_finished(v, v, &key, 1, &run, "t1", "ok", false, None).unwrap();
        let w = steps_for_turn(v, &key, 1, true)
            .unwrap()
            .into_iter()
            .find(|s| s.kind == "write")
            .unwrap();
        assert_eq!(w.files.len(), 1, "{w:?}");
        assert_eq!(w.files[0].state, "captured");
    }

    fn captured(path: &str, hash: &str, size: u64) -> FileVersion {
        FileVersion {
            path: path.into(),
            hash: Some(hash.into()),
            size: Some(size),
            state: "captured".into(),
        }
    }

    #[test]
    fn what_fed_a_failure_is_its_model_step_its_delegate_and_the_writes_it_names() {
        let mut task = step(2, "delegate", Some("Task"), vec![]);
        task.parent_seq = None;
        let mut inner_model = step(3, "model", None, vec![]);
        inner_model.parent_seq = Some(2);
        let mut write = step(
            4,
            "write",
            Some("Write"),
            vec![captured("w/out/report.md", "h", 1)],
        );
        write.parent_seq = Some(2);
        let mut inner_model2 = step(5, "model", None, vec![]);
        inner_model2.parent_seq = Some(2);
        let mut fail = step(6, "tool", Some("Bash"), vec![]);
        fail.parent_seq = Some(2);
        fail.status = "error".into();
        fail.checkpoint = Some(json!({"input": "{\"command\":\"wc -l report.md\"}"}));
        let steps = vec![
            step(1, "model", None, vec![]),
            task,
            inner_model,
            write,
            inner_model2,
            fail,
        ];
        assert_eq!(fed_by(&steps, 6), vec![2, 4, 5]);
        // At the top level: just the model step before it.
        assert_eq!(fed_by(&steps, 2), vec![1]);
        assert!(fed_by(&steps, 99).is_empty());
    }

    #[test]
    fn tools_classify_alike_across_providers() {
        assert_eq!(classify("Write"), "write");
        assert_eq!(classify("file_change"), "write");
        assert_eq!(classify("Task"), "delegate");
        assert_eq!(classify("mcp__hollow_team__ask_agent"), "delegate");
        assert_eq!(classify("hollow_app.start_session"), "delegate");
        assert_eq!(classify("Bash"), "tool");
        assert_eq!(classify("command_execution"), "tool");
        assert!(unrecorded_writer("tool", Some("Bash")));
        assert!(!unrecorded_writer("tool", Some("Read")));
        assert!(unrecorded_writer("delegate", Some("Task")));
        assert!(!unrecorded_writer("write", Some("Edit")));
    }

    #[test]
    fn named_paths_cover_claude_and_codex_inputs() {
        assert_eq!(
            named_paths(&json!({"file_path": "/v/a.md"})),
            vec!["/v/a.md"]
        );
        assert_eq!(
            named_paths(&json!({"changes": [{"path": "b.md", "kind": "add"}, {"path": "a.md"}]})),
            vec!["a.md", "b.md"]
        );
        assert!(named_paths(&json!({"command": "ls"})).is_empty());
    }

    #[test]
    fn wanted_versions_take_the_newest_recorded_at_or_before_n_else_the_before() {
        let steps = vec![
            step(1, "model", None, vec![]),
            step(
                2,
                "write",
                Some("Write"),
                vec![captured("w/a.md", "h-a2", 2)],
            ),
            step(3, "model", None, vec![]),
            step(
                4,
                "write",
                Some("Edit"),
                vec![captured("w/a.md", "h-a4", 4), captured("w/b.md", "h-b4", 4)],
            ),
            step(5, "model", None, vec![]),
        ];
        let changed = vec![
            ch("w/a.md", Some("h-a0"), Some(1)),
            ch("w/b.md", None, None),
        ];
        let at = |n| wanted_versions(&steps, n, &changed).unwrap();
        assert_eq!(
            at(2),
            vec![
                WantedVersion {
                    vault_path: "w/a.md".into(),
                    hash: Some("h-a2".into()),
                    size: Some(2)
                },
                WantedVersion {
                    vault_path: "w/b.md".into(),
                    hash: None,
                    size: None
                },
            ]
        );
        assert_eq!(at(5)[0].hash.as_deref(), Some("h-a4"));
        assert_eq!(at(5)[1].hash.as_deref(), Some("h-b4"));
        assert_eq!(
            at(0)[0].hash.as_deref(),
            Some("h-a0"),
            "step 0: the run's before"
        );
        assert!(wanted_versions(&steps, 9, &changed)
            .unwrap_err()
            .contains("no step 9"));
    }

    #[test]
    fn an_unrecorded_writer_or_an_outside_write_refuses() {
        let changed = vec![ch("w/a.md", Some("h0"), Some(1))];
        let shell = vec![
            step(1, "tool", Some("Bash"), vec![]),
            step(2, "write", Some("Write"), vec![captured("w/a.md", "h2", 2)]),
        ];
        let err = wanted_versions(&shell, 2, &changed).unwrap_err();
        assert!(
            err.contains("Bash") && err.contains("Nothing changed"),
            "{err}"
        );
        // Before the shell step it is exact; with no files written or changed, any step.
        assert!(wanted_versions(&shell, 0, &changed).is_ok());
        assert!(wanted_versions(&shell[..1], 1, &[]).is_ok());
        // A read-only tool is not a writer.
        let read = vec![
            step(1, "tool", Some("Read"), vec![]),
            step(2, "write", Some("Write"), vec![captured("w/a.md", "h2", 2)]),
        ];
        assert!(wanted_versions(&read, 2, &changed).is_ok());
        let outside = vec![step(
            1,
            "write",
            Some("Write"),
            vec![FileVersion {
                path: "/tmp/x".into(),
                hash: None,
                size: None,
                state: "outside".into(),
            }],
        )];
        assert!(wanted_versions(&outside, 1, &changed)
            .unwrap_err()
            .contains("outside the vault"));
        let gone = vec![step(
            1,
            "write",
            Some("Write"),
            vec![captured("w/tmp.md", "h1", 1)],
        )];
        assert!(wanted_versions(&gone, 1, &changed)
            .unwrap_err()
            .contains("w/tmp.md"));
        assert!(
            wanted_versions(&gone, 0, &changed).is_ok(),
            "before the write it is exact"
        );
        let big = vec![step(
            1,
            "write",
            Some("Write"),
            vec![FileVersion {
                path: "w/a.md".into(),
                hash: None,
                size: None,
                state: "uncaptured".into(),
            }],
        )];
        assert!(wanted_versions(&big, 1, &changed)
            .unwrap_err()
            .contains("no copy"));
    }

    #[test]
    fn the_seed_prompt_says_retry_not_replay_and_lists_steps_up_to_n() {
        let mut s2 = step(2, "write", Some("Write"), vec![]);
        s2.checkpoint =
            Some(json!({"input": "{\"file_path\":\"a.md\"}", "status": "ok", "result": "written"}));
        let mut s4 = step(4, "tool", Some("Bash"), vec![]);
        s4.checkpoint = Some(json!({"input": "npm test", "status": "error"}));
        let p = seed_prompt(
            "s-1",
            3,
            "Fix the report",
            "<task-state>…</task-state>",
            &[step(1, "model", None, vec![]), s2, s4],
            2,
            Some("run-r"),
        );
        assert!(p.contains("not a replay"));
        assert!(p.contains("2. write Write"));
        assert!(!p.contains("npm test"), "step 4 is after n");
        assert!(p.contains("History run run-r"));
        assert!(p.contains("Fix the report"));
    }

    #[test]
    fn the_spine_survives_a_crash_and_nests_delegates() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-spine-1".to_string();
        let ord = crate::session_store::start_turn(v, &key, "claude", "/vault", "run-1", "Do it")
            .unwrap();
        turn_started(v, &key, ord, "run-1").unwrap();
        tool_started(
            v,
            &key,
            ord,
            "run-1",
            "t1",
            "Task",
            &json!({"prompt": "look"}),
        )
        .unwrap();
        tool_started(
            v,
            &key,
            ord,
            "run-1",
            "t2",
            "Read",
            &json!({"file_path": "a"}),
        )
        .unwrap();
        tool_finished(v, v, &key, ord, "run-1", "t2", "ok", false, None).unwrap();
        // Crash: the delegate and the model step after Read never end.
        let live = steps_for_turn(v, &key, ord, true).unwrap();
        assert_eq!(
            live.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(),
            vec!["model", "delegate", "tool", "model"]
        );
        assert_eq!(live[2].parent_seq, Some(2), "the Read nests in the Task");
        assert_eq!(live[3].parent_seq, Some(2));
        assert_eq!(live[1].status, "running");
        let after = steps_for_turn(v, &key, ord, false).unwrap();
        assert_eq!(after[1].status, "interrupted");
        assert_eq!(after[3].status, "interrupted");
        assert_eq!(after[2].status, "ok");
        let seqs: Vec<i64> = after.iter().map(|s| s.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4], "no gaps");
        // The turn's end settles every open step.
        finish_turn(v, &key, ord, "error").unwrap();
        assert!(steps_for_turn(v, &key, ord, true)
            .unwrap()
            .iter()
            .all(|s| s.status != "running"));
    }

    /// Q-C3 (2026-11-13): the display target is redacted before it is cut,
    /// so a key starting near the 180-character cut can't leave a stub too
    /// short for the key patterns to catch.
    #[test]
    fn a_display_target_is_redacted_before_the_cut() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-spine-redact".to_string();
        let ord = crate::session_store::start_turn(v, &key, "claude", "/vault", "run-r", "Do it")
            .unwrap();
        turn_started(v, &key, ord, "run-r").unwrap();
        let secret = "sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let command = format!("{}{secret} tail", "word ".repeat(34));
        tool_started(
            v,
            &key,
            ord,
            "run-r",
            "t1",
            "Bash",
            &json!({"command": command}),
        )
        .unwrap();
        let steps = steps_for_turn(v, &key, ord, true).unwrap();
        let target = steps
            .iter()
            .find_map(|s| s.checkpoint.as_ref()?.get("displayTarget")?.as_str())
            .expect("a display target");
        assert!(!target.contains("sk-ant"), "{target}");
        assert!(target.contains("[api-key]"), "{target}");
    }

    #[test]
    fn a_write_step_records_the_files_version_in_history() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-spine-2".to_string();
        let history = crate::automated_history::begin_run(v, "chat", Some(&key)).unwrap();
        let run = history.id.clone();
        crate::session_store::start_turn(v, &key, "claude", "/vault", &run, "Write it").unwrap();
        std::fs::create_dir_all(v.join("w")).unwrap();
        std::fs::write(v.join("w/a.md"), "after step").unwrap();
        let input =
            json!({"file_path": v.join("w/a.md").to_string_lossy(), "content": "after step"});
        crate::session_store::append_event(
            v,
            &key,
            1,
            "toolUse",
            &json!({"id": "t1", "name": "Write", "input": input}).to_string(),
        )
        .unwrap();
        turn_started(v, &key, 1, &run).unwrap();
        tool_started(v, &key, 1, &run, "t1", "Write", &input).unwrap();
        tool_finished(v, v, &key, 1, &run, "t1", "File written", false, None).unwrap();
        let steps = steps_for_turn(v, &key, 1, true).unwrap();
        let w = steps.iter().find(|s| s.kind == "write").unwrap();
        assert_eq!(w.files.len(), 1);
        assert_eq!(w.files[0].path, "w/a.md");
        assert_eq!(w.files[0].state, "captured");
        // Without a History run to pin it, nothing is stored: uncaptured.
        let loose = capture(v, v, "run-unknown", &input);
        assert_eq!(loose[0].state, "uncaptured");
        assert_eq!(w.checkpoint.as_ref().unwrap()["result"], "File written");
        // Outside the vault: recorded as outside, nothing stored.
        let out = capture(v, v, "run-1", &json!({"file_path": "/etc/hosts"}));
        assert_eq!(out[0].state, "outside");
    }

    #[test]
    fn tool_finished_records_result_bytes() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-spine-3".to_string();
        let ord =
            crate::session_store::start_turn(v, &key, "codex", "/vault", "run-3", "Read").unwrap();
        turn_started(v, &key, ord, "run-3").unwrap();
        tool_started(
            v,
            &key,
            ord,
            "run-3",
            "t1",
            "Read",
            &json!({"file_path": "a"}),
        )
        .unwrap();
        // 1,000 two-byte characters: the size is the full result's UTF-8
        // bytes, though the checkpoint keeps 300 characters of it.
        let content = "é".repeat(1_000);
        tool_finished(v, v, &key, ord, "run-3", "t1", &content, false, None).unwrap();
        let steps = steps_for_turn(v, &key, ord, true).unwrap();
        let read = steps.iter().find(|s| s.kind == "tool").unwrap();
        let checkpoint = read.checkpoint.as_ref().unwrap();
        assert_eq!(checkpoint["resultBytes"], 2_000);
        assert_eq!(checkpoint["result"].as_str().unwrap().chars().count(), 301);
    }
}
