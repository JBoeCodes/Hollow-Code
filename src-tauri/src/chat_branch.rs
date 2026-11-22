//! Branch a chat from a turn (CH1, R1 #30, 2026-11-18).
//!
//! **Branch from here** on a reply makes a new chat B of chat K: a copy of
//! K's turns up to and including that reply's turn (their transcripts, the
//! CP1 restore events that apply to them, the effort), `parentKey` K,
//! `forkKind` [`FORK_KIND`], and a row in `chat_branches` naming where it
//! branched. K is never written. B's stint has no native session, so its
//! next turn starts a fresh one and receives B's own turns as its history
//! (`context_compiler::compile_turn_within` reads
//! `session_store::conversation_entries`): the branch continues from the
//! chosen turn on every agent. The copies carry no run id (a run id names
//! one turn): the branch's copied replies show their text, and votes,
//! receipts and Branch from here stay on the original's. AC1's `fork` stays false (no adapter forks a
//! native session here; Claude `--fork-session` and Codex `thread/fork`
//! fork at the session's end, not at a turn).
//!
//! **Files.** B works in K's folder: a project lists its chats by folder, so
//! a branch in a folder of its own would leave the project's sidebar. When
//! K owns an isolated worktree and CP1 recorded the chosen turn's after
//! checkpoint, the branch keeps that checkpoint reachable as
//! `refs/hollow/branches/<B>` and lists the paths that differ from it now,
//! so the files as they were at the branch point are one `git diff` away.
//! Two chats in one worktree are not isolated, so CP1 records no more
//! checkpoints for either (the confirm says so first).
//!
//! **Leaving.** "Back to the original" on B opens K with a short written
//! handoff of B's own turns (OR1's field names: objective, state, evidence,
//! next action) added to K's draft; nothing is sent or stored until the
//! person sends it.
//!
//! Refused: an unknown reply, a turn still running, a turn a restore undid,
//! a chat that lives on another Mac (moved away or moving).

use crate::session_store::{db_time, with_read, with_write};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::Path;

/// `sessions.fork_kind` of a branch.
pub(crate) const FORK_KIND: &str = "branch";

/// The longest run id a branch request carries.
const RUN_ID_MAX: usize = 200;

/// Characters of one handoff line, at most.
const LINE_CHARS: usize = 600;
/// Characters of the handoff's state (the branch's last reply), at most.
const STATE_CHARS: usize = 1_500;
/// The newest own turns the handoff lists, at most.
const HANDOFF_TURNS: usize = 6;

/// Where each branch came from. Applied with the session schema.
pub(crate) const SCHEMA: &str = "\
 CREATE TABLE IF NOT EXISTS chat_branches( \
   branch_key TEXT PRIMARY KEY, \
   from_key TEXT NOT NULL, \
   through_ord INTEGER NOT NULL, \
   run_id TEXT, \
   checkpoint TEXT, \
   at TEXT NOT NULL \
 ); \
 CREATE INDEX IF NOT EXISTS idx_chat_branches_from ON chat_branches(from_key);";

/// The columns of `turns` a branch copies, ords kept. Never `run_id`: a
/// run id names one turn (votes, `session_for_run`, the step spine's
/// `turn_of_run`), so the copies have none and the run stays the original's.
const TURN_COLUMNS: &str = "ord, started_at, ended_at, status, asked, permission_mode, effort, \
     asked_by, capture_scope, packet_constraints, attachments";
/// The columns of `steps` a branch copies (its check outcomes join the
/// task-state record a fresh session gets), without `run_id` likewise.
const STEP_COLUMNS: &str = "turn_ord, seq, kind, name, tool_id, parent_seq, started_at, \
     ended_at, status, files, checkpoint";

/// A branch just made.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Branched {
    pub key: String,
    pub title: Option<String>,
    /// The original's last turn the branch holds.
    pub through: i64,
    /// Turns copied.
    pub turns: i64,
    /// The CP1 checkpoint of the files at the branch point (a commit), kept
    /// as `ref_name`; none outside an isolated worktree.
    pub checkpoint: Option<String>,
    pub ref_name: Option<String>,
    /// Paths that differ between that checkpoint and the files now.
    pub changed_since: Vec<String>,
}

/// A branch, for its notice.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BranchInfo {
    pub key: String,
    pub from_key: String,
    pub from_title: Option<String>,
    pub through: i64,
    pub checkpoint: Option<String>,
    /// Turns the branch has of its own (after the branch point).
    pub own_turns: i64,
}

fn refuse(message: &str) -> String {
    message.to_string()
}

/// Turn `run_id`'s ord in chat `key`, refused when it can't be branched.
fn branch_point(conn: &Connection, key: &str, run_id: &str) -> Result<i64, String> {
    if let Some(home) = crate::chat_move::home_in(conn, key)? {
        if home.state == crate::chat_move::AWAY || home.state == crate::chat_move::LEAVING {
            return Err(refuse(
                "This chat lives on another Mac now, so it can't branch here.",
            ));
        }
    }
    // An offline copy folds back into its chat (and its row goes), so a
    // branch of it would point at nothing.
    let fork: Option<String> = conn
        .query_row(
            "SELECT fork_kind FROM sessions WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("branch read: {e}"))?
        .flatten();
    if fork.as_deref() == Some(crate::chat_offline::FORK_KIND) {
        return Err(refuse(
            "This chat is an offline copy that syncs back to its Mac; branch it once it has synced.",
        ));
    }
    let row: Option<(i64, Option<String>)> = conn
        .query_row(
            "SELECT ord, status FROM turns WHERE session_key = ?1 AND run_id = ?2 \
             ORDER BY ord DESC LIMIT 1",
            params![key, run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("branch read: {e}"))?;
    let Some((ord, status)) = row else {
        return Err(refuse("That reply isn't in this chat's record."));
    };
    if status.as_deref() == Some("running") {
        return Err(refuse(
            "That turn is still running; branch from it once it ends.",
        ));
    }
    if undone(conn, key)?
        .iter()
        .any(|(from, to)| (*from..=*to).contains(&ord))
    {
        return Err(refuse(
            "A restore undid that turn; branch from a turn before it.",
        ));
    }
    Ok(ord)
}

/// The `(before, through)` ranges CP1 restores undid in chat `key`.
fn undone(conn: &Connection, key: &str) -> Result<Vec<(i64, i64)>, String> {
    let mut stmt = conn
        .prepare("SELECT payload FROM events WHERE session_key = ?1 AND kind = ?2")
        .map_err(|e| format!("branch undone: {e}"))?;
    let rows = stmt
        .query_map(
            params![key, crate::turn_checkpoints::RESTORE_EVENT],
            |row| row.get::<_, Option<String>>(0),
        )
        .map_err(|e| format!("branch undone: {e}"))?;
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

/// A session key no chat here has.
fn fresh_key(conn: &Connection) -> Result<String, String> {
    for _ in 0..8 {
        let key = crate::agent_sessions::mint_session_key();
        let used: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .map_err(|e| format!("branch key: {e}"))?;
        if used == 0 {
            return Ok(key);
        }
    }
    Err("no free chat key".into())
}

/// Branch chat `key` after the turn whose run is `run_id`: a new chat with
/// its turns up to there (module docs). The original is not written.
pub(crate) fn branch(
    vault: &Path,
    key: &str,
    run_id: &str,
    now: DateTime<Utc>,
) -> Result<Branched, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(refuse("That isn't a chat's key."));
    }
    let run_id = run_id.trim();
    if run_id.is_empty() || run_id.len() > RUN_ID_MAX {
        return Err(refuse("That reply has no run to branch from."));
    }
    // The files first (git, outside the database's write lock); the branch
    // point is checked again in the write.
    let through = with_read(vault, |conn| branch_point(conn, key, run_id))?;
    let files = crate::turn_checkpoints::files_since(vault, key, through);
    let at = db_time(now);
    let (branched, title) = with_write(vault, "chat branch", |tx| {
        let again = branch_point(tx, key, run_id)?;
        if again != through {
            return Err(refuse("This chat changed while branching; try again."));
        }
        let (title, agent, cwd): (Option<String>, String, String) = tx
            .query_row(
                "SELECT title, agent, cwd FROM sessions WHERE key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| format!("branch read: {e}"))?
            .ok_or_else(|| refuse("That chat isn't in Hollow."))?;
        let branch_key = fresh_key(tx)?;
        let copied = tx
            .execute(
                &format!(
                    "INSERT INTO turns(session_key, {TURN_COLUMNS}) SELECT ?2, {TURN_COLUMNS} \
                     FROM turns WHERE session_key = ?1 AND ord <= ?3 ORDER BY ord"
                ),
                params![key, branch_key, through],
            )
            .map_err(|e| format!("branch turns: {e}"))? as i64;
        tx.execute(
            &format!(
                "INSERT INTO steps(session_key, {STEP_COLUMNS}) SELECT ?2, {STEP_COLUMNS} \
                 FROM steps WHERE session_key = ?1 AND turn_ord <= ?3"
            ),
            params![key, branch_key, through],
        )
        .map_err(|e| format!("branch steps: {e}"))?;
        tx.execute(
            "INSERT INTO transcripts(session_key, turn_ord, payload) \
             SELECT ?2, turn_ord, payload FROM transcripts \
             WHERE session_key = ?1 AND turn_ord <= ?3",
            params![key, branch_key, through],
        )
        .map_err(|e| format!("branch transcripts: {e}"))?;
        // A restore's undone turns stay out of the branch's history too.
        tx.execute(
            "INSERT INTO events(session_key, turn_ord, at, kind, payload) \
             SELECT ?2, turn_ord, at, kind, payload FROM events \
             WHERE session_key = ?1 AND kind = ?3 AND turn_ord <= ?4",
            params![
                key,
                branch_key,
                crate::turn_checkpoints::RESTORE_EVENT,
                through
            ],
        )
        .map_err(|e| format!("branch restores: {e}"))?;
        let title = title.map(|t| format!("{} (branch)", t.trim()));
        tx.execute(
            "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, \
             turn_count, parent_key, fork_kind) VALUES(?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
            params![branch_key, title, agent, cwd, at, copied, key, FORK_KIND],
        )
        .map_err(|e| format!("branch session: {e}"))?;
        tx.execute(
            "INSERT INTO stints(session_key, agent, started_at) VALUES(?1, ?2, ?3)",
            params![branch_key, agent, at],
        )
        .map_err(|e| format!("branch stint: {e}"))?;
        tx.execute(
            "INSERT INTO chat_efforts(key, effort, updated_at) \
             SELECT ?2, effort, ?3 FROM chat_efforts WHERE key = ?1",
            params![key, branch_key, at],
        )
        .map_err(|e| format!("branch effort: {e}"))?;
        tx.execute(
            "INSERT INTO chat_branches(branch_key, from_key, through_ord, run_id, checkpoint, at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                branch_key,
                key,
                through,
                run_id,
                files.as_ref().map(|f| f.checkpoint.clone()),
                at
            ],
        )
        .map_err(|e| format!("branch record: {e}"))?;
        Ok((
            Branched {
                key: branch_key,
                title: None,
                through,
                turns: copied,
                checkpoint: None,
                ref_name: None,
                changed_since: Vec::new(),
            },
            title,
        ))
    })?;
    let mut out = Branched { title, ..branched };
    if let Some(files) = files {
        // Keep the branch point's files reachable for the branch's life. On
        // a failure (logged) the branch stands without a checkpoint: its
        // record forgets the sha, so nothing names a ref that isn't there.
        match crate::turn_checkpoints::keep_for_branch(&files.top, &out.key, &files.checkpoint) {
            Ok(name) => {
                out.ref_name = Some(name);
                out.checkpoint = Some(files.checkpoint);
                out.changed_since = files.changed;
            }
            Err(e) => {
                eprintln!("[branch] {}: keeping the checkpoint failed: {e}", out.key);
                with_write(vault, "chat branch checkpoint", |tx| {
                    tx.execute(
                        "UPDATE chat_branches SET checkpoint = NULL WHERE branch_key = ?1",
                        params![out.key],
                    )
                    .map(|_| ())
                    .map_err(|e| format!("branch record: {e}"))
                })?;
            }
        }
    }
    Ok(out)
}

/// Chat `key`'s branch record, when it is a branch.
pub(crate) fn info(vault: &Path, key: &str) -> Result<Option<BranchInfo>, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Ok(None);
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT b.from_key, s.title, b.through_ord, b.checkpoint, \
             (SELECT COUNT(*) FROM turns t WHERE t.session_key = b.branch_key \
              AND t.ord > b.through_ord) \
             FROM chat_branches b LEFT JOIN sessions s ON s.key = b.from_key \
             WHERE b.branch_key = ?1",
            params![key],
            |row| {
                Ok(BranchInfo {
                    key: key.to_string(),
                    from_key: row.get(0)?,
                    from_title: row.get(1)?,
                    through: row.get(2)?,
                    checkpoint: row.get(3)?,
                    own_turns: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("branch info: {e}"))
    })
}

fn clip(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

/// The short written handoff "Back to the original" adds to the original's
/// draft: branch `key`'s own turns (after the branch point), in OR1's
/// field names. Refused for a chat that isn't a branch or has no turns of
/// its own yet.
pub(crate) fn handoff(vault: &Path, key: &str) -> Result<String, String> {
    let info = info(vault, key)?.ok_or_else(|| refuse("This chat isn't a branch."))?;
    let (title, turns) = with_read(vault, |conn| {
        let title: Option<String> = conn
            .query_row(
                "SELECT title FROM sessions WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("branch handoff: {e}"))?
            .flatten();
        let mut stmt = conn
            .prepare(
                "SELECT ord, asked FROM turns WHERE session_key = ?1 AND ord > ?2 ORDER BY ord",
            )
            .map_err(|e| format!("branch handoff: {e}"))?;
        let rows = stmt
            .query_map(params![key, info.through], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|e| format!("branch handoff: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("branch handoff: {e}"))?;
        let mut turns = Vec::with_capacity(rows.len());
        for (ord, asked) in rows {
            let reply = crate::session_store::transcript_text(conn, key, ord)?;
            turns.push((asked.unwrap_or_default(), reply));
        }
        Ok((title, turns))
    })?;
    if turns.is_empty() {
        return Err(refuse("This branch has no turns of its own yet."));
    }
    let name = title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("a branch");
    let count = if turns.len() == 1 {
        "1 turn".to_string()
    } else {
        format!("{} turns", turns.len())
    };
    let mut lines = vec![
        format!(
            "Handoff from the branch \u{201c}{name}\u{201d}, which branched from this chat after turn {} ({count} of its own; a written summary, not a resumed session).",
            info.through
        ),
        String::new(),
        format!("Objective: {}", clip(&turns[0].0, LINE_CHARS)),
        format!(
            "State: {}",
            clip(&turns[turns.len() - 1].1, STATE_CHARS)
        ),
        "Turns:".to_string(),
    ];
    for (asked, reply) in turns.iter().skip(turns.len().saturating_sub(HANDOFF_TURNS)) {
        lines.push(format!(
            "- Asked: {} Answered: {}",
            clip(asked, LINE_CHARS / 2),
            clip(reply, LINE_CHARS / 2)
        ));
    }
    let mut evidence = format!("Evidence: branch chat {key}");
    if info.checkpoint.is_some() {
        evidence.push_str(&format!(
            "; the files at the branch point: git ref refs/hollow/branches/{key}"
        ));
    }
    lines.push(evidence);
    lines.push("Next action: ".to_string());
    Ok(lines.join("\n"))
}

// ── commands ──────────────────────────────────────────────────────────

/// Branch from here (CH1): the person's click on a reply. The new chat.
#[tauri::command]
pub async fn branch_chat(
    state: tauri::State<'_, crate::commands::AppState>,
    key: String,
    run_id: String,
) -> Result<Branched, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || branch(&vault, &key, &run_id, Utc::now()))
        .await
        .map_err(|e| e.to_string())?
}

/// A branch's record for its notice (none for any other chat). Reads only.
#[tauri::command]
pub async fn chat_branch_info(
    state: tauri::State<'_, crate::commands::AppState>,
    key: String,
) -> Result<Option<BranchInfo>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || info(&vault, &key))
        .await
        .map_err(|e| e.to_string())?
}

/// The written handoff "Back to the original" adds to the original's
/// draft. Reads only.
#[tauri::command]
pub async fn chat_branch_handoff(
    state: tauri::State<'_, crate::commands::AppState>,
    key: String,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || handoff(&vault, &key))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "chat_branch_tests.rs"]
mod tests;
