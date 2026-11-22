//! Saved turn feedback (Paperclip PR 17): the user's Useful / Not useful vote
//! on a chat response, with a redacted snapshot of what was voted on.
//!
//! One row per turn in the `turn_feedback` table of the session store, which
//! lives OUTSIDE the vault (no history, receipt, watcher or `run_guard`
//! involvement). The rows are primary data, not a derived cache: session-store
//! retention never prunes them and archiving a conversation leaves them alone.
//! Only [`clear`] (Settings, Response feedback, Clear feedback) deletes them.
//! Hollow has no session delete, so nothing else can.
//!
//! **Identity is the exact run id.** A vote names `turns.run_id`, the id the
//! evidence receipt, the history run and the usage row share. It resolves to
//! `(session_key, turn_ord)` only when exactly one turn row carries it (the
//! [`session_store::session_for_run`] rule); otherwise nothing is written. The
//! frontend sends a run id only from exact sources (the live `runStarted` id,
//! the live receipt, a Hollow-store reload, an artifact summary); legacy
//! Claude-history turns, whose receipts are joined by timing, never carry one.
//!
//! **The snapshot comes from the stores, never from UI prose** (standing rule
//! 7): the ask from `turns.asked`, the answer, tool names, agent and recall
//! paths from the durable transcript, the receipt id / status / verification
//! from `.evidence-receipts.json` (an unreadable receipt store degrades to no
//! receipt, since the file is agent-writable), and the model from the chat
//! usage row. Every string is passed through `claude_runner::redact` and THEN
//! capped, so a truncated secret can't slip past a pattern. [`export`] redacts
//! every string field again (rows written before a pattern existed, and the
//! raw session title).
//!
//! A turn is refused as still running only when its row says `running` AND
//! the full-vault lease names this run: `finish_turn` is the only writer that
//! closes a turn and its caller ignores failures, so a crash can leave a turn
//! `running` forever. Such an orphan is accepted and its snapshot records the
//! stored status. `lease_holder` is advisory; here it only chooses between
//! saving and refusing one vote, never an admission.
//!
//! Lock order: [`crate::evidence::list`] (its `FILE_LOCK`) is read BEFORE any
//! `session_store::with_write`; the two leaf locks are never nested.

use crate::agent_sessions::is_safe_session_key;
use crate::automated_history;
use crate::claude_runner::redact;
use crate::session_store::{self, db_time, FeedbackTurnLookup};
use crate::types::{FeedbackExport, TurnFeedback, TurnFeedbackInput, TurnFeedbackSnapshot};
use chrono::{DateTime, Local, Utc};
use rusqlite::params;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const NO_TURN: &str = "FEEDBACK_NO_TURN";
pub const AMBIGUOUS_RUN: &str = "FEEDBACK_AMBIGUOUS_RUN";
pub const TURN_RUNNING: &str = "FEEDBACK_TURN_RUNNING";
pub const INVALID: &str = "FEEDBACK_INVALID";
pub const EXPORT_IN_VAULT: &str = "FEEDBACK_EXPORT_IN_VAULT";
pub const EXPORT_INVALID: &str = "FEEDBACK_EXPORT_INVALID";

const ASK_CHARS: usize = 8_000;
const ANSWER_CHARS: usize = 8_000;
const REASON_CHARS: usize = 2_000;
const RECEIPT_FIELD_CHARS: usize = 128;
const NAME_CHARS: usize = 128;
const RECALL_CHARS: usize = 512;
const MAX_TOOLS: usize = 50;
pub const LIST_DEFAULT: usize = 200;
pub const LIST_MAX: usize = 1_000;
const RUN_ID_MAX: usize = 200;
const RUN_IDS_MAX: usize = 200;
/// Receipts scanned for the run's receipt (the store keeps at most 500).
const RECEIPT_SCAN: usize = 1_000;
const EXPORT_SCHEMA_VERSION: u32 = 1;

pub const VOTES: [&str; 2] = ["useful", "notUseful"];

/// `text` cut to at most `limit` characters (never mid-character), and
/// whether anything was cut.
fn cap_chars(text: &str, limit: usize) -> (String, bool) {
    if text.chars().count() <= limit {
        return (text.to_string(), false);
    }
    (text.chars().take(limit).collect(), true)
}

/// Redact, then cap. The order matters: capping first could cut a secret
/// in half so that no pattern recognizes the half that is kept.
fn clean(text: &str, limit: usize) -> String {
    cap_chars(&redact(text), limit).0
}

fn clean_opt(text: Option<&str>, limit: usize) -> Option<String> {
    text.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| clean(s, limit))
}

fn validate_run_id(run_id: &str) -> Result<(), String> {
    if run_id.trim().is_empty() {
        return Err(format!("{INVALID}: a vote needs the response's run id"));
    }
    if run_id.chars().count() > RUN_ID_MAX || run_id.chars().any(char::is_control) {
        return Err(format!("{INVALID}: that run id is not valid"));
    }
    Ok(())
}

fn validate_key(session_key: Option<&str>) -> Result<(), String> {
    match session_key {
        Some(key) if !is_safe_session_key(key) => {
            Err(format!("{INVALID}: unsafe session key: {key}"))
        }
        _ => Ok(()),
    }
}

/// Save (or replace) the vote on the turn `input.run_id` names. Returns the
/// stored row. Nothing is written on any refusal.
///
/// A vote with no reason on a turn that already has the SAME vote keeps the
/// saved reason (clicking an active "Not useful" again must not erase what
/// the user typed); a changed vote replaces it. Save order is the caller's:
/// the store chains saves per run id, because a timestamp guard here would
/// silently drop votes after the wall clock stepped back.
pub fn record(
    vault: &Path,
    input: &TurnFeedbackInput,
    now: DateTime<Utc>,
) -> Result<TurnFeedback, String> {
    let run_id = input.run_id.as_str();
    if !VOTES.contains(&input.vote.as_str()) {
        return Err(format!(
            "{INVALID}: unknown vote `{}`",
            clean(&input.vote, 32)
        ));
    }
    validate_run_id(run_id)?;
    let reason = clean_opt(input.reason.as_deref(), REASON_CHARS);

    let source = match session_store::feedback_turn_source(vault, run_id)? {
        FeedbackTurnLookup::Missing => {
            return Err(format!(
                "{NO_TURN}: this response has no saved turn record, so the vote was not saved"
            ))
        }
        FeedbackTurnLookup::Ambiguous(count) => {
            return Err(format!(
                "{AMBIGUOUS_RUN}: {count} turns share this response's run id, so the vote was not saved"
            ))
        }
        FeedbackTurnLookup::Found(source) => source,
    };
    // Live means the run still holds the vault lease, or (PR B3) its scoped
    // folder.
    if source.status.as_deref() == Some("running") && automated_history::run_is_live(vault, run_id)
    {
        return Err(format!(
            "{TURN_RUNNING}: this response is still being written; vote once it finishes"
        ));
    }

    // Read before `with_write`: never nest evidence's FILE_LOCK inside the
    // session store's WRITE_LOCK. The receipts file is agent-writable, so an
    // unreadable one means "no receipt", never a refused vote.
    let receipt = crate::evidence::list(vault, Some("chat"), None, RECEIPT_SCAN)
        .unwrap_or_default()
        .into_iter()
        .find(|receipt| receipt.run_id == run_id);

    let (answer, answer_truncated) = cap_chars(&redact(&source.answer), ANSWER_CHARS);
    let snapshot = TurnFeedbackSnapshot {
        asked: clean(&source.asked, ASK_CHARS),
        answer,
        answer_truncated,
        tools: source
            .tool_names
            .iter()
            .take(MAX_TOOLS)
            .map(|name| clean(name, NAME_CHARS))
            .collect(),
        receipt_id: receipt.as_ref().map(|r| clean(&r.id, RECEIPT_FIELD_CHARS)),
        receipt_status: receipt
            .as_ref()
            .map(|r| clean(&r.status, RECEIPT_FIELD_CHARS)),
        verification_state: receipt
            .as_ref()
            .map(|r| clean(&r.verification_state, RECEIPT_FIELD_CHARS)),
        recall: source
            .recall
            .iter()
            .map(|path| clean(path, RECALL_CHARS))
            .collect(),
        turn_status: clean_opt(source.status.as_deref(), RECEIPT_FIELD_CHARS),
    };
    let snapshot_json =
        serde_json::to_string(&snapshot).map_err(|e| format!("feedback snapshot encode: {e}"))?;
    let agent = clean_opt(Some(source.agent.as_str()), NAME_CHARS);
    let model = clean_opt(source.model.as_deref(), NAME_CHARS);
    let stamp = db_time(now);

    session_store::with_write(vault, "turn feedback", |tx| {
        tx.execute(
            "INSERT INTO turn_feedback(session_key, turn_ord, run_id, vote, reason, agent, model, \
             snapshot_json, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9) \
             ON CONFLICT(session_key, turn_ord) DO UPDATE SET run_id = excluded.run_id, \
             vote = excluded.vote, \
             reason = CASE WHEN excluded.reason IS NULL AND excluded.vote = turn_feedback.vote \
             THEN turn_feedback.reason ELSE excluded.reason END, agent = excluded.agent, \
             model = excluded.model, snapshot_json = excluded.snapshot_json, \
             updated_at = excluded.updated_at",
            params![
                source.session_key,
                source.ord,
                run_id,
                input.vote,
                reason,
                agent,
                model,
                snapshot_json,
                stamp
            ],
        )
        .map_err(|e| format!("turn feedback write: {e}"))?;
        Ok(())
    })?;

    let mut rows = query(
        vault,
        Some(source.session_key.as_str()),
        &[],
        Some(LIST_MAX),
    )?;
    rows.retain(|row| i64::from(row.turn_ord) == source.ord);
    rows.into_iter()
        .next()
        .ok_or_else(|| "turn feedback: the saved vote could not be read back".to_string())
}

const SELECT: &str = "SELECT f.id, f.session_key, s.title, f.turn_ord, f.run_id, f.vote, \
     f.reason, f.agent, f.model, f.snapshot_json, f.created_at, f.updated_at \
     FROM turn_feedback f LEFT JOIN sessions s ON s.key = f.session_key";

fn feedback_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnFeedback> {
    let title: Option<String> = row.get(2)?;
    let snapshot: String = row.get(9)?;
    Ok(TurnFeedback {
        id: row.get(0)?,
        session_key: row.get(1)?,
        // The title is the raw first line of the conversation's first ask.
        session_title: title.map(|t| redact(&t)),
        turn_ord: row.get::<_, i64>(3)?.max(0) as u32,
        run_id: row.get(4)?,
        vote: row.get(5)?,
        reason: row.get(6)?,
        agent: row.get(7)?,
        model: row.get(8)?,
        // A hand-edited or future-schema row must not break the list.
        snapshot: serde_json::from_str(&snapshot).unwrap_or_default(),
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

/// Rows filtered by conversation and/or run ids, newest update first. `None`
/// for `limit` means every row (export only).
fn query(
    vault: &Path,
    session_key: Option<&str>,
    run_ids: &[String],
    limit: Option<usize>,
) -> Result<Vec<TurnFeedback>, String> {
    let mut clauses: Vec<String> = Vec::new();
    let mut values: Vec<String> = Vec::new();
    if let Some(key) = session_key {
        values.push(key.to_string());
        clauses.push(format!("f.session_key = ?{}", values.len()));
    }
    let ids: Vec<&String> = run_ids.iter().filter(|id| !id.is_empty()).collect();
    if !ids.is_empty() {
        let mut marks = Vec::with_capacity(ids.len());
        for id in ids {
            values.push(id.clone());
            marks.push(format!("?{}", values.len()));
        }
        clauses.push(format!("f.run_id IN ({})", marks.join(", ")));
    }
    let mut sql = SELECT.to_string();
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY f.updated_at DESC, f.id DESC");
    if let Some(limit) = limit {
        sql.push_str(&format!(" LIMIT {}", limit.clamp(1, LIST_MAX)));
    }
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("turn feedback prepare: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(values.iter()), feedback_row)
            .map_err(|e| format!("turn feedback query: {e}"))?;
        let collected = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("turn feedback rows: {e}"));
        collected
    })
}

/// Saved votes, newest update first: all of them, one conversation's, or the
/// ones on these runs (how a view rehydrates the turns it shows, by the same
/// identity it votes with). `limit` is clamped to 1..=[`LIST_MAX`].
pub fn list(
    vault: &Path,
    session_key: Option<&str>,
    run_ids: &[String],
    limit: usize,
) -> Result<Vec<TurnFeedback>, String> {
    validate_key(session_key)?;
    if run_ids.len() > RUN_IDS_MAX {
        return Err(format!(
            "{INVALID}: at most {RUN_IDS_MAX} run ids per request"
        ));
    }
    if run_ids.iter().any(|id| id.chars().count() > RUN_ID_MAX) {
        return Err(format!("{INVALID}: that run id is not valid"));
    }
    query(vault, session_key, run_ids, Some(limit))
}

/// Delete saved votes: one conversation's, or every one. Conversations,
/// transcripts and receipts are untouched. Returns how many were removed.
pub fn clear(vault: &Path, session_key: Option<&str>) -> Result<usize, String> {
    validate_key(session_key)?;
    session_store::with_write(vault, "clear turn feedback", |tx| {
        let removed = match session_key {
            Some(key) => tx.execute(
                "DELETE FROM turn_feedback WHERE session_key = ?1",
                params![key],
            ),
            None => tx.execute("DELETE FROM turn_feedback", []),
        };
        removed.map_err(|e| format!("clear turn feedback: {e}"))
    })
}

/// Every string a person or an agent wrote, redacted again: rows saved before
/// a pattern existed, and the session title (the raw first line of the first
/// ask), which the export would otherwise copy verbatim. Idempotent.
fn redact_row(row: &mut TurnFeedback) {
    let fix = |s: &mut String| *s = redact(s);
    let fix_opt = |s: &mut Option<String>| {
        if let Some(value) = s.as_mut() {
            *value = redact(value);
        }
    };
    fix_opt(&mut row.session_title);
    fix_opt(&mut row.reason);
    fix_opt(&mut row.agent);
    fix_opt(&mut row.model);
    let snapshot = &mut row.snapshot;
    fix(&mut snapshot.asked);
    fix(&mut snapshot.answer);
    snapshot.tools.iter_mut().for_each(fix);
    snapshot.recall.iter_mut().for_each(fix);
    fix_opt(&mut snapshot.receipt_id);
    fix_opt(&mut snapshot.receipt_status);
    fix_opt(&mut snapshot.verification_state);
    fix_opt(&mut snapshot.turn_status);
}

/// A code fence longer than any run of backticks inside `text` (at least 3),
/// so an answer containing a fence can't close ours.
fn md_fence(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

/// One line: newlines in a heading or list item would let the content start
/// its own Markdown section.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn vote_label(vote: &str) -> &'static str {
    if vote == "useful" {
        "Useful"
    } else {
        "Not useful"
    }
}

fn render_markdown(rows: &[TurnFeedback], exported_at: &str) -> String {
    let mut out = String::new();
    out.push_str("# Hollow response feedback\n\n");
    out.push_str(&format!(
        "Exported {exported_at}. {} saved vote{}. Redacted with claude_runner::redact. Local only.\n",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    ));
    for row in rows {
        let title = row
            .session_title
            .as_deref()
            .map(one_line)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| row.session_key.clone());
        out.push_str(&format!(
            "\n## {} · {} · turn {}\n\n",
            vote_label(&row.vote),
            title,
            row.turn_ord
        ));
        out.push_str(&format!("- Conversation: `{}`\n", row.session_key));
        out.push_str(&format!("- Run: `{}`\n", row.run_id));
        out.push_str(&format!(
            "- Agent: {} · Model: {}\n",
            row.agent
                .as_deref()
                .map(one_line)
                .unwrap_or_else(|| "unknown".into()),
            row.model
                .as_deref()
                .map(one_line)
                .unwrap_or_else(|| "unknown".into())
        ));
        out.push_str(&format!("- Saved: {}\n", row.updated_at));
        if let Some(reason) = row.reason.as_deref() {
            out.push_str(&format!("- Reason: {}\n", one_line(reason)));
        }
        let snap = &row.snapshot;
        if !snap.tools.is_empty() {
            out.push_str(&format!("- Tools: {}\n", one_line(&snap.tools.join(", "))));
        }
        if let Some(id) = snap.receipt_id.as_deref() {
            out.push_str(&format!(
                "- Receipt: {} ({}, {})\n",
                one_line(id),
                snap.receipt_status
                    .as_deref()
                    .map(one_line)
                    .unwrap_or_default(),
                snap.verification_state
                    .as_deref()
                    .map(one_line)
                    .unwrap_or_default()
            ));
        }
        if !snap.recall.is_empty() {
            out.push_str(&format!(
                "- Recall: {}\n",
                one_line(&snap.recall.join(", "))
            ));
        }
        if let Some(status) = snap.turn_status.as_deref() {
            out.push_str(&format!("- Turn status: {}\n", one_line(status)));
        }
        for (label, text) in [("Asked", &snap.asked), ("Answer", &snap.answer)] {
            let fence = md_fence(text);
            let note = if label == "Answer" && snap.answer_truncated {
                " (truncated)"
            } else {
                ""
            };
            out.push_str(&format!(
                "\n**{label}**{note}\n\n{fence}\n{text}\n{fence}\n"
            ));
        }
    }
    out
}

/// Create `path` exclusively (never replacing a file), owner-only on Unix.
fn create_new(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Write both files under the first free `{stem}[-n]` (n = 2..=99), each
/// created exclusively. A name taken by either file moves on to the next.
fn write_pair(
    dir: &Path,
    stem: &str,
    json: &[u8],
    markdown: &[u8],
) -> Result<(PathBuf, PathBuf), String> {
    for n in 1..=99u32 {
        let base = if n == 1 {
            stem.to_string()
        } else {
            format!("{stem}-{n}")
        };
        let json_path = dir.join(format!("{base}.json"));
        let md_path = dir.join(format!("{base}.md"));
        let mut json_file = match create_new(&json_path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{EXPORT_INVALID}: could not write the export: {e}")),
        };
        let mut md_file = match create_new(&md_path) {
            Ok(file) => file,
            Err(e) => {
                drop(json_file);
                let _ = std::fs::remove_file(&json_path);
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    continue;
                }
                return Err(format!("{EXPORT_INVALID}: could not write the export: {e}"));
            }
        };
        let written = json_file
            .write_all(json)
            .and_then(|_| json_file.sync_all())
            .and_then(|_| md_file.write_all(markdown))
            .and_then(|_| md_file.sync_all());
        if let Err(e) = written {
            let _ = std::fs::remove_file(&json_path);
            let _ = std::fs::remove_file(&md_path);
            return Err(format!("{EXPORT_INVALID}: could not write the export: {e}"));
        }
        return Ok((json_path, md_path));
    }
    Err(format!(
        "{EXPORT_INVALID}: too many exports named {stem} in that folder"
    ))
}

/// Write every saved vote, redacted, as `hollow-feedback-{stamp}.json` and
/// `.md` into `dir`: an absolute, existing folder OUTSIDE the vault (files in
/// the vault would be picked up by search, history and sync). Never
/// overwrites a file.
pub fn export(vault: &Path, dir: &Path, now: DateTime<Local>) -> Result<FeedbackExport, String> {
    if !dir.is_absolute() {
        return Err(format!("{EXPORT_INVALID}: choose an absolute folder"));
    }
    let target = std::fs::canonicalize(dir)
        .map_err(|_| format!("{EXPORT_INVALID}: that folder does not exist"))?;
    if !target.is_dir() {
        return Err(format!("{EXPORT_INVALID}: choose a folder, not a file"));
    }
    let canonical_vault = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    if target.starts_with(&canonical_vault) {
        return Err(format!(
            "{EXPORT_IN_VAULT}: choose a folder outside the vault"
        ));
    }

    let mut rows = query(vault, None, &[], None)?;
    rows.iter_mut().for_each(redact_row);
    let exported_at = now.to_rfc3339();
    let json = serde_json::to_vec_pretty(&json!({
        "schemaVersion": EXPORT_SCHEMA_VERSION,
        "exportedAt": exported_at,
        "redaction": "claude_runner::redact",
        "count": rows.len(),
        "feedback": rows,
    }))
    .map_err(|e| format!("feedback export encode: {e}"))?;
    let markdown = render_markdown(&rows, &exported_at);
    let stem = format!("hollow-feedback-{}", now.format("%Y%m%d-%H%M%S"));
    let (json_path, md_path) = write_pair(&target, &stem, &json, markdown.as_bytes())?;
    Ok(FeedbackExport {
        json_path: json_path.to_string_lossy().to_string(),
        markdown_path: md_path.to_string_lossy().to_string(),
        count: rows.len() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::{
        append_event, finish_turn, record_recall, start_turn, upsert_usage,
    };
    use chrono::TimeZone;

    /// Named `vault-…`, not `.tmp…`: the lease snapshot walks the vault, and a
    /// dot-prefixed root is pruned whole.
    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn input(run: &str, vote: &str, reason: Option<&str>) -> TurnFeedbackInput {
        TurnFeedbackInput {
            run_id: run.to_string(),
            vote: vote.to_string(),
            reason: reason.map(str::to_string),
        }
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_790_000_000 + secs, 0).unwrap()
    }

    /// A finished turn with an answer.
    fn turn(v: &Path, key: &str, run: &str, asked: &str, answer: &str) -> i64 {
        let ord = start_turn(v, key, "claude", "/vault", run, asked).unwrap();
        append_event(v, key, ord, "assistant", answer).unwrap();
        finish_turn(v, key, ord, "success").unwrap();
        ord
    }

    fn all(v: &Path) -> Vec<TurnFeedback> {
        list(v, None, &[], LIST_MAX).unwrap()
    }

    const KEY: &str = "s-20260926-feed00a1";
    const KEY2: &str = "s-20260926-feed00a2";

    #[test]
    fn a_vote_maps_its_run_to_the_turn_ordinal() {
        let dir = vault();
        let v = dir.path();
        for (run, ask) in [("r1", "one"), ("r2", "two"), ("r3", "three")] {
            turn(v, KEY, run, ask, "answer");
        }
        let saved = record(v, &input("r2", "useful", None), at(0)).unwrap();
        assert_eq!(saved.session_key, KEY);
        assert_eq!(saved.turn_ord, 2);
        assert_eq!(saved.run_id, "r2");
        assert_eq!(saved.vote, "useful");
        assert_eq!(saved.snapshot.asked, "two");
        let by_key = list(v, Some(KEY), &[], 10).unwrap();
        let by_run = list(v, None, &["r2".to_string()], 10).unwrap();
        assert_eq!(by_key, vec![saved.clone()]);
        assert_eq!(by_run, vec![saved]);
    }

    #[test]
    fn an_unknown_run_records_nothing() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "answer");
        let error = record(v, &input("nope", "useful", None), at(0)).unwrap_err();
        assert!(error.starts_with(NO_TURN), "{error}");
        assert!(all(v).is_empty());
    }

    #[test]
    fn a_run_shared_by_two_sessions_is_refused() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "shared", "one", "a");
        turn(v, KEY2, "shared", "two", "b");
        let error = record(v, &input("shared", "useful", None), at(0)).unwrap_err();
        assert!(error.starts_with(AMBIGUOUS_RUN), "{error}");
        assert!(all(v).is_empty());
    }

    #[test]
    fn a_turn_whose_run_holds_the_lease_is_refused() {
        let dir = vault();
        let v = dir.path();
        std::fs::write(v.join("seed.md"), "seed").unwrap();
        let lease = automated_history::try_begin_run(v, "chat", Some(KEY)).unwrap();
        let run = lease.id.clone();
        let ord = start_turn(v, KEY, "claude", "/vault", &run, "still going").unwrap();
        let error = record(v, &input(&run, "useful", None), at(0)).unwrap_err();
        assert!(error.starts_with(TURN_RUNNING), "{error}");
        assert!(all(v).is_empty());

        automated_history::finish_run(v, lease, "success").unwrap();
        finish_turn(v, KEY, ord, "success").unwrap();
        let saved = record(v, &input(&run, "useful", None), at(1)).unwrap();
        assert_eq!(saved.snapshot.turn_status.as_deref(), Some("success"));
    }

    #[test]
    fn an_orphaned_running_turn_can_be_voted_on() {
        let dir = vault();
        let v = dir.path();
        // No lease at all.
        start_turn(v, KEY, "claude", "/vault", "r-orphan", "crashed").unwrap();
        let saved = record(v, &input("r-orphan", "notUseful", None), at(0)).unwrap();
        assert_eq!(saved.snapshot.turn_status.as_deref(), Some("running"));
        // A live lease naming ANOTHER run does not make this one live.
        std::fs::write(v.join("seed.md"), "seed").unwrap();
        let other = automated_history::try_begin_run(v, "chat", Some(KEY2)).unwrap();
        start_turn(v, KEY2, "claude", "/vault", "r-orphan-2", "also crashed").unwrap();
        let saved = record(v, &input("r-orphan-2", "useful", None), at(1)).unwrap();
        assert_eq!(saved.snapshot.turn_status.as_deref(), Some("running"));
        automated_history::finish_run(v, other, "success").unwrap();
    }

    #[test]
    fn the_snapshot_comes_from_the_stores() {
        let dir = vault();
        let v = dir.path();
        let ord = start_turn(v, KEY, "claude", "/vault", "r1", "Fix the bug").unwrap();
        record_recall(v, KEY, ord, &["learnings/a.md".to_string()]).unwrap();
        append_event(
            v,
            KEY,
            ord,
            "toolUse",
            r#"{"id":"t1","name":"Read","input":{}}"#,
        )
        .unwrap();
        append_event(v, KEY, ord, "toolResult", r#"{"id":"t1","content":"ok"}"#).unwrap();
        append_event(
            v,
            KEY,
            ord,
            "toolUse",
            r#"{"id":"t2","name":"Edit","input":{}}"#,
        )
        .unwrap();
        append_event(v, KEY, ord, "assistant", "Fixed it.").unwrap();
        finish_turn(v, KEY, ord, "success").unwrap();
        let receipt = crate::evidence::EvidenceCollector::new("chat", None, "r1")
            .finish(v, "success", vec![])
            .unwrap();
        let now = db_time(Utc::now());
        let mut chat_row = session_store::tests::usage_row_fixture("u-chat", &now, "success");
        chat_row.run_id = Some("r1".into());
        chat_row.model = Some("claude-test".into());
        upsert_usage(v, &chat_row).unwrap();
        // A later delegate row for the same run is ignored.
        let mut delegate = session_store::tests::usage_row_fixture("u-del", &now, "success");
        delegate.run_id = Some("r1".into());
        delegate.surface = "delegate".into();
        delegate.model = Some("gpt-delegate".into());
        upsert_usage(v, &delegate).unwrap();

        let saved = record(v, &input("r1", "useful", None), at(0)).unwrap();
        let snap = &saved.snapshot;
        assert_eq!(snap.asked, "Fix the bug");
        assert_eq!(snap.answer, "Fixed it.");
        assert!(!snap.answer_truncated);
        assert_eq!(snap.tools, vec!["Read".to_string(), "Edit".to_string()]);
        assert_eq!(snap.receipt_id.as_deref(), Some(receipt.id.as_str()));
        assert_eq!(snap.receipt_status.as_deref(), Some("success"));
        assert_eq!(
            snap.verification_state.as_deref(),
            Some(receipt.verification_state.as_str())
        );
        assert_eq!(snap.recall, vec!["learnings/a.md".to_string()]);
        assert_eq!(saved.agent.as_deref(), Some("claude"));
        assert_eq!(saved.model.as_deref(), Some("claude-test"));
    }

    #[test]
    fn a_corrupt_receipt_store_does_not_block_a_vote() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "answer");
        std::fs::write(v.join(crate::evidence::FILE_NAME), "{ not json").unwrap();
        let saved = record(v, &input("r1", "useful", None), at(0)).unwrap();
        assert_eq!(saved.snapshot.receipt_id, None);
        assert_eq!(saved.snapshot.receipt_status, None);

        std::fs::remove_file(v.join(crate::evidence::FILE_NAME)).unwrap();
        turn(v, KEY, "r-long", "two", "answer");
        crate::evidence::EvidenceCollector::new("chat", None, "r-long")
            .finish(v, &"s".repeat(10_000), vec![])
            .unwrap();
        let saved = record(v, &input("r-long", "useful", None), at(1)).unwrap();
        let status = saved.snapshot.receipt_status.unwrap();
        assert_eq!(status.chars().count(), RECEIPT_FIELD_CHARS);
    }

    const PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAsecretpembody\n-----END RSA PRIVATE KEY-----";

    fn ghp() -> String {
        format!("ghp_{}", "Q".repeat(36))
    }

    #[test]
    fn the_snapshot_is_redacted_before_it_is_capped() {
        let dir = vault();
        let v = dir.path();
        let ask = format!(
            "Please help\n{PEM}\ntoken {} db postgres://admin:hunter2@db.example.com/app key sk-ant-api03-abcdefghijklmnopqrstuvwx",
            ghp()
        );
        // A secret straddling character 8,000 of the answer.
        let answer = format!("{} {} {}", "a".repeat(7_990), ghp(), "b".repeat(12_000));
        let pat = format!("github_pat_{}", "R".repeat(40));
        turn(v, KEY, "r1", &ask, &answer);
        let reason = format!("leaked {pat}");
        record(v, &input("r1", "notUseful", Some(&reason)), at(0)).unwrap();

        let rows = all(v);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(row.snapshot.answer_truncated);
        assert_eq!(row.snapshot.answer.chars().count(), ANSWER_CHARS);
        let listed = serde_json::to_string(row).unwrap();
        let stored: String = session_store::with_read(v, |conn| {
            conn.query_row(
                "SELECT snapshot_json || COALESCE(reason, '') FROM turn_feedback",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        for secret in [
            "secretpembody",
            "ghp_",
            "QQQQQQQQ",
            "hunter2",
            "abcdefghijklmnop",
            "github_pat_",
            "RRRRRRRR",
        ] {
            assert!(
                !listed.contains(secret),
                "{secret} survived in the listed row"
            );
            assert!(
                !stored.contains(secret),
                "{secret} survived in the stored row"
            );
        }
    }

    #[test]
    fn a_second_vote_replaces_the_first() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "answer");
        let first = record(v, &input("r1", "useful", None), at(0)).unwrap();
        let second = record(v, &input("r1", "notUseful", Some("wrong file")), at(5)).unwrap();
        assert_eq!(all(v).len(), 1);
        assert_eq!(second.id, first.id);
        assert_eq!(second.created_at, first.created_at);
        assert!(second.updated_at >= second.created_at);
        assert_ne!(second.updated_at, first.updated_at);
        assert_eq!(second.vote, "notUseful");
        assert_eq!(second.reason.as_deref(), Some("wrong file"));
        let third = record(v, &input("r1", "useful", None), at(9)).unwrap();
        assert_eq!(third.reason, None);
        assert_eq!(third.vote, "useful");
    }

    #[test]
    fn repeating_a_vote_without_a_reason_keeps_the_saved_reason() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "answer");
        record(v, &input("r1", "notUseful", Some("wrong file")), at(0)).unwrap();
        let again = record(v, &input("r1", "notUseful", None), at(3)).unwrap();
        assert_eq!(again.reason.as_deref(), Some("wrong file"));
        let replaced = record(v, &input("r1", "notUseful", Some("missed tests")), at(6)).unwrap();
        assert_eq!(replaced.reason.as_deref(), Some("missed tests"));
        let switched = record(v, &input("r1", "useful", None), at(9)).unwrap();
        assert_eq!(switched.reason, None);
        let back = record(v, &input("r1", "notUseful", None), at(12)).unwrap();
        assert_eq!(back.reason, None);
    }

    #[test]
    fn an_invalid_vote_or_run_id_is_refused() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "answer");
        let long = "r".repeat(RUN_ID_MAX + 1);
        for bad in [
            input("r1", "meh", None),
            input("", "useful", None),
            input(&long, "useful", None),
        ] {
            let error = record(v, &bad, at(0)).unwrap_err();
            assert!(error.starts_with(INVALID), "{error}");
        }
        assert!(all(v).is_empty());
        let many: Vec<String> = (0..=RUN_IDS_MAX).map(|n| format!("r{n}")).collect();
        assert!(list(v, None, &many, 10).unwrap_err().starts_with(INVALID));
    }

    #[test]
    fn list_filters_and_orders_newest_first() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "First conversation", "a");
        turn(v, KEY, "r2", "follow-up", "b");
        turn(v, KEY2, "r3", "Second conversation", "c");
        record(v, &input("r2", "useful", None), at(0)).unwrap();
        record(v, &input("r3", "useful", None), at(1)).unwrap();
        record(v, &input("r1", "notUseful", None), at(2)).unwrap();
        let order: Vec<String> = all(v).into_iter().map(|row| row.run_id).collect();
        assert_eq!(order, vec!["r1", "r3", "r2"]);
        let first = list(v, Some(KEY), &[], 10).unwrap();
        assert_eq!(first.len(), 2);
        assert!(first
            .iter()
            .all(|row| row.session_title.as_deref() == Some("First conversation")));
        let second = list(v, Some(KEY2), &[], 10).unwrap();
        assert_eq!(
            second[0].session_title.as_deref(),
            Some("Second conversation")
        );
        let by_runs = list(v, None, &["r3".to_string(), "r1".to_string()], 10).unwrap();
        assert_eq!(by_runs.len(), 2);
        assert_eq!(list(v, None, &[], 1).unwrap().len(), 1);
        assert!(list(v, Some("../etc"), &[], 10)
            .unwrap_err()
            .starts_with(INVALID));
    }

    fn local(secs: i64) -> DateTime<Local> {
        Local.timestamp_opt(1_790_000_000 + secs, 0).unwrap()
    }

    #[test]
    fn export_writes_json_and_markdown_outside_the_vault() {
        let dir = vault();
        let v = dir.path();
        let out = tempfile::tempdir().unwrap();
        turn(
            v,
            KEY,
            "r1",
            "Show code",
            "Here:\n```rust\nfn main() {}\n```\ndone",
        );
        turn(v, KEY, "r2", "Thanks", "You're welcome");
        record(v, &input("r1", "useful", None), at(0)).unwrap();
        record(v, &input("r2", "notUseful", Some("too short")), at(1)).unwrap();

        let first = export(v, out.path(), local(0)).unwrap();
        assert_eq!(first.count, 2);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&first.json_path).unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["count"], 2);
        assert_eq!(json["feedback"].as_array().unwrap().len(), 2);
        let md = std::fs::read_to_string(&first.markdown_path).unwrap();
        assert_eq!(md.matches("\n## ").count(), 2, "{md}");
        assert!(
            md.contains("````\nHere:\n```rust"),
            "the answer's fence can't close ours: {md}"
        );
        assert!(md.contains("- Reason: too short"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&first.json_path, &first.markdown_path] {
                let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{path}");
            }
        }

        let before = std::fs::read(&first.json_path).unwrap();
        let second = export(v, out.path(), local(0)).unwrap();
        assert!(
            second.json_path.ends_with("-2.json"),
            "{}",
            second.json_path
        );
        assert!(
            second.markdown_path.ends_with("-2.md"),
            "{}",
            second.markdown_path
        );
        assert_eq!(std::fs::read(&first.json_path).unwrap(), before);
    }

    #[test]
    fn export_refuses_the_vault_and_missing_folders() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "a");
        record(v, &input("r1", "useful", None), at(0)).unwrap();
        let inside = v.join("sub");
        std::fs::create_dir(&inside).unwrap();
        let error = export(v, &inside, local(0)).unwrap_err();
        assert!(error.starts_with(EXPORT_IN_VAULT), "{error}");
        let error = export(v, v, local(0)).unwrap_err();
        assert!(error.starts_with(EXPORT_IN_VAULT), "{error}");
        let missing = v.parent().unwrap().join("hollow-no-such-export-dir-pr17");
        assert!(export(v, &missing, local(0))
            .unwrap_err()
            .starts_with(EXPORT_INVALID));
        assert!(export(v, Path::new("relative/dir"), local(0))
            .unwrap_err()
            .starts_with(EXPORT_INVALID));
        assert_eq!(
            std::fs::read_dir(&inside).unwrap().count(),
            0,
            "nothing written"
        );
    }

    #[test]
    fn export_redacts_every_field_including_the_session_title() {
        let dir = vault();
        let v = dir.path();
        let out = tempfile::tempdir().unwrap();
        // The session title is the first line of the FIRST ask, stored raw.
        let first_ask = format!(
            "connect postgres://admin:hunter2@db.example.com/app with {}",
            ghp()
        );
        turn(v, KEY, "r1", &first_ask, "ok");
        record(v, &input("r1", "useful", None), at(0)).unwrap();
        // A row written before a pattern existed: raw secrets in the snapshot.
        let raw = TurnFeedbackSnapshot {
            asked: PEM.to_string(),
            answer: format!("see {PEM}"),
            tools: vec![ghp()],
            ..TurnFeedbackSnapshot::default()
        };
        let raw_json = serde_json::to_string(&raw).unwrap();
        session_store::with_write(v, "seed", |tx| {
            tx.execute(
                "INSERT INTO turn_feedback(session_key, turn_ord, run_id, vote, reason, agent, \
                 model, snapshot_json, created_at, updated_at) VALUES(?1, 9, 'r-old', 'notUseful', \
                 ?2, 'claude', NULL, ?3, ?4, ?4)",
                params![
                    KEY,
                    "pw postgres://u:hunter2@h/db",
                    raw_json,
                    db_time(at(1))
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();

        let written = export(v, out.path(), local(0)).unwrap();
        assert_eq!(written.count, 2);
        let json = std::fs::read_to_string(&written.json_path).unwrap();
        let md = std::fs::read_to_string(&written.markdown_path).unwrap();
        for text in [&json, &md] {
            for secret in ["hunter2", "ghp_", "QQQQQQQQ", "secretpembody", "BEGIN RSA"] {
                assert!(!text.contains(secret), "{secret} survived the export");
            }
        }
        assert!(md.contains("postgres://[redacted]"), "{md}");
    }

    #[test]
    fn clear_removes_only_what_it_names() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "a");
        turn(v, KEY, "r2", "two", "b");
        turn(v, KEY2, "r3", "three", "c");
        for run in ["r1", "r2", "r3"] {
            record(v, &input(run, "useful", None), at(0)).unwrap();
        }
        assert_eq!(clear(v, Some(KEY)).unwrap(), 2);
        assert_eq!(all(v).len(), 1);
        assert_eq!(clear(v, None).unwrap(), 1);
        assert!(all(v).is_empty());
        assert!(clear(v, Some("../x")).unwrap_err().starts_with(INVALID));
        // Conversations are untouched.
        let conversation = session_store::read_conversation(v, KEY).unwrap();
        assert_eq!(conversation.turns.len(), 4);
        assert_eq!(conversation.turns[1].text, "a");
    }

    #[test]
    fn retention_and_archive_keep_feedback() {
        let dir = vault();
        let v = dir.path();
        turn(v, KEY, "r1", "one", "a");
        record(v, &input("r1", "useful", None), at(0)).unwrap();
        session_store::retain(v).unwrap();
        session_store::archive_session(v, KEY, true).unwrap();
        assert_eq!(list(v, None, &["r1".to_string()], 10).unwrap().len(), 1);
        assert_eq!(list(v, Some(KEY), &[], 10).unwrap().len(), 1);
    }

    #[test]
    fn md_fence_outgrows_every_backtick_run() {
        assert_eq!(md_fence("plain"), "```");
        assert_eq!(md_fence("a ``` b"), "````");
        assert_eq!(md_fence("`````"), "``````");
    }
}
