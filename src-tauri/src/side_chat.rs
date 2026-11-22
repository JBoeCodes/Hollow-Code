//! Side chat (t3code parity 7.6, 2026-11-20): a second chat beside the one
//! on screen, for a quick question about it that must not disturb its run.
//!
//! A side chat is an ordinary chat of its own: its own session key, turns,
//! run ledger rows, budgets and usage, so it counts like any chat. What makes
//! it a side chat is `sessions.fork_kind` [`FORK_KIND`] with `parent_key` the
//! chat it sits beside, and four rules this module holds:
//!
//! 1. **Seeded, not shared.** Its first turn continues from the parent
//!    (`ChatSendRequest::continued_from`), the same path Continue from
//!    summary uses: the context compiler bundles the parent's recorded turns
//!    and its task-state record into the fresh native session, and the
//!    parent is only read. The side chat never resumes the parent's native
//!    session and never writes one of its turns into it. A result goes back
//!    only when the person presses Add to main chat, which puts words in the
//!    parent's composer as a draft (window only, nothing is sent).
//! 2. **Never stronger than the parent.** A side turn's permission mode is at
//!    most the parent's last recorded mode ([`ceiling`]) and defaults to
//!    `plan` (read only), so a question can't change files the parent's run
//!    is working on. [`clamp_admitted`] applies this at queue admission to
//!    every send into a side chat, whoever sent it, so a caller naming a
//!    higher mode is lowered, never obeyed. The agent is the parent's own, so
//!    the ranking and `agent::mode_capability` agree.
//! 3. **Off the lists.** Side chats are not rows of the sidebar, Conversation
//!    history, search or Jump; the pane lists the parent's own ([`list`]).
//!    They are not subagents (`fork_kind` is not `subagent`), so no subagent
//!    machinery (lean prompt, per-parent limit, spend rolled into the
//!    parent) applies: a side chat is the person's own conversation.
//! 4. **Not for the Main agent.** The Main agent's (COO's) conversations keep
//!    their own page, and a side chat has no side chat. A project lead's chat
//!    is an ordinary chat here and may have one. A chat that lives on another
//!    Mac, or is an offline copy, is refused (it can't be read here as a
//!    whole).
//!
//! Refused with a sentence: an unknown parent (nothing sent yet), the above.

use crate::agent::AgentKind;
use crate::commands::{AppState, ChatSendInput};
use crate::session_store::{self, db_time, with_read, with_write};
use crate::types::{AgentSessionSummary, SendAccepted};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

/// `sessions.fork_kind` of a side chat.
pub(crate) const FORK_KIND: &str = session_store::FORK_SIDE;

/// The mode a side turn runs in unless the person asks for the parent's own:
/// read only on every agent (`agent::mode_capability`).
pub(crate) const DEFAULT_MODE: &str = "plan";

/// A new side chat's title until its first question names it.
const NEW_TITLE: &str = "Side chat";

/// The parent's side chats the pane lists, newest first, at most.
const LIST_MAX: usize = 20;

fn refuse(message: &str) -> String {
    message.to_string()
}

/// The highest mode a side chat of `parent` may run in: the parent's last
/// recorded mode, or Hollow's default when it recorded none.
pub(crate) fn ceiling(vault: &Path, parent: &str) -> &'static str {
    crate::app_mcp::own_mode(
        session_store::last_permission(vault, parent)
            .ok()
            .flatten()
            .as_deref(),
    )
}

/// The mode a side turn runs in: `requested` (none: [`DEFAULT_MODE`]) lowered
/// to `ceiling`, and read only when the agent's own capability for it would
/// still exceed the ceiling's. Never above `ceiling`.
pub(crate) fn side_mode(
    requested: Option<&str>,
    ceiling: &str,
    agent: Option<AgentKind>,
) -> &'static str {
    let wanted = requested
        .map(crate::app_mcp::normalize_perm)
        .unwrap_or(DEFAULT_MODE);
    let mode = crate::app_mcp::lowered(wanted, Some(ceiling));
    if let Some(kind) = agent {
        if !crate::agent::mode_capability(kind, mode)
            .within(crate::agent::mode_capability(kind, ceiling))
        {
            return DEFAULT_MODE;
        }
    }
    mode
}

/// `(parent key, agent)` when `key` is a side chat.
fn side_of(vault: &Path, key: &str) -> Option<(Option<String>, String)> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return None;
    }
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT parent_key, agent FROM sessions WHERE key = ?1 AND fork_kind = ?2",
            params![key, FORK_KIND],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("side chat read: {e}"))
    })
    .ok()
    .flatten()
}

/// Queue admission's mode for a send into conversation `key` (`mode`: what
/// the caller asked for): unchanged for any chat that isn't a side chat, and
/// for a side chat [`side_mode`] against its parent's ceiling. A side chat
/// with no parent runs read only.
pub(crate) fn clamp_admitted(vault: &Path, key: &str, mode: Option<String>) -> Option<String> {
    let Some((parent, agent)) = side_of(vault, key) else {
        return mode;
    };
    let kind = AgentKind::from_opt(&agent);
    let ceiling = match parent.as_deref() {
        Some(parent) => ceiling(vault, parent),
        None => DEFAULT_MODE,
    };
    Some(side_mode(mode.as_deref(), ceiling, kind).to_string())
}

/// The parent's agent and folder, or why it can't have a side chat.
fn check_parent(conn: &Connection, parent: &str) -> Result<(String, String), String> {
    if !crate::agent_sessions::is_safe_session_key(parent) {
        return Err(refuse("That isn't a chat's key."));
    }
    let row: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT agent, cwd, fork_kind FROM sessions WHERE key = ?1",
            params![parent],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("side chat read: {e}"))?;
    let Some((agent, cwd, fork)) = row else {
        return Err(refuse(
            "Send a first message in this chat first: a side chat reads what the chat has said.",
        ));
    };
    match fork.as_deref() {
        Some(FORK_KIND) => return Err(refuse("A side chat can't have a side chat.")),
        Some(kind) if kind == crate::chat_offline::FORK_KIND => {
            return Err(refuse(
                "This chat is an offline copy that syncs back to its Mac; open a side chat once it has synced.",
            ))
        }
        _ => {}
    }
    if let Some(home) = crate::chat_move::home_in(conn, parent)? {
        if home.state == crate::chat_move::AWAY || home.state == crate::chat_move::LEAVING {
            return Err(refuse(
                "This chat lives on another Mac now, so it can't have a side chat here.",
            ));
        }
    }
    Ok((agent, cwd))
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
            .map_err(|e| format!("side chat key: {e}"))?;
        if used == 0 {
            return Ok(key);
        }
    }
    Err("no free chat key".into())
}

/// Parent `parent`'s side chat to show: its newest one (what the pane opens
/// on), or a new one when it has none, when `fresh` asks for one, or never
/// while the newest has no turn yet (an empty one is reused, so the list
/// holds no empty rows). The parent is only read.
pub(crate) fn open(
    vault: &Path,
    parent: &str,
    fresh: bool,
    now: DateTime<Utc>,
) -> Result<AgentSessionSummary, String> {
    if crate::chat_chain::is_main_conversation(vault, parent) {
        return Err(refuse(
            "The Main agent keeps its own page. Open a side chat from a project chat or a chat in Chats.",
        ));
    }
    let key = with_write(vault, "side chat", |tx| {
        let (agent, cwd) = check_parent(tx, parent)?;
        let newest: Option<(String, i64)> = tx
            .query_row(
                "SELECT key, turn_count FROM sessions \
                 WHERE parent_key = ?1 AND fork_kind = ?2 AND archived = 0 \
                 ORDER BY last_activity_at DESC, key ASC LIMIT 1",
                params![parent, FORK_KIND],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| format!("side chat read: {e}"))?;
        if let Some((key, turns)) = newest {
            if !fresh || turns == 0 {
                return Ok(key);
            }
        }
        let key = fresh_key(tx)?;
        let at = db_time(now);
        tx.execute(
            "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, \
             turn_count, parent_key, fork_kind) VALUES(?1, ?2, ?3, ?4, ?5, ?5, 0, ?6, ?7)",
            params![key, NEW_TITLE, agent, cwd, at, parent, FORK_KIND],
        )
        .map_err(|e| format!("side chat insert: {e}"))?;
        Ok(key)
    })?;
    session_store::session_by_key(vault, &key)?
        .ok_or_else(|| refuse("The side chat could not be opened."))
}

/// `parent`'s side chats that have a turn, newest first.
pub(crate) fn list(vault: &Path, parent: &str) -> Result<Vec<AgentSessionSummary>, String> {
    if !crate::agent_sessions::is_safe_session_key(parent) {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT key FROM sessions WHERE parent_key = ?1 AND fork_kind = ?2 \
                 AND archived = 0 AND turn_count > 0 \
                 ORDER BY last_activity_at DESC, key ASC LIMIT {LIST_MAX}"
            ))
            .map_err(|e| format!("side chat list: {e}"))?;
        let rows = stmt
            .query_map(params![parent, FORK_KIND], |row| row.get::<_, String>(0))
            .map_err(|e| format!("side chat list: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("side chat list: {e}"));
        rows
    })?;
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        if let Some(summary) = session_store::session_by_key(vault, &key)? {
            out.push(summary);
        }
    }
    Ok(out)
}

/// What a side turn is sent with, from the stored side chat `key`: its
/// parent's folder-mate agent and model, the parent's effort, the mode
/// ([`side_mode`]: `same_access` asks for the parent's own, else read only),
/// the native session to resume (none on the first turn) and the parent as
/// the conversation it continues from. Names the chat after its first
/// question. Returns the input and the effort to queue with it.
pub(crate) fn prepare(
    vault: &Path,
    key: &str,
    prompt: &str,
    client_turn_id: Option<String>,
    same_access: bool,
) -> Result<(ChatSendInput, Option<String>), String> {
    let side = session_store::session_by_key(vault, key)?
        .filter(|s| s.fork_kind.as_deref() == Some(FORK_KIND))
        .ok_or_else(|| refuse("That isn't a side chat."))?;
    let parent = side
        .parent_key
        .clone()
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| refuse("This side chat has lost the chat it sat beside."))?;
    let parent_row = session_store::session_by_key(vault, &parent)?
        .ok_or_else(|| refuse("The chat this side chat sat beside is gone."))?;
    let ceiling = ceiling(vault, &parent);
    let kind = AgentKind::from_opt(&side.agent);
    let mode = side_mode(same_access.then_some(ceiling), ceiling, kind);
    // The parent's model for the same agent; another agent's model id would
    // be wrong here.
    let model = session_store::last_requested_model(vault, &parent)
        .ok()
        .flatten()
        .filter(|(_, agent)| agent.as_deref() == Some(side.agent.as_str()))
        .and_then(|(model, _)| model);
    let effort = session_store::chat_effort(vault, &parent).ok().flatten();
    if side.turn_count == 0 {
        let title = crate::chat_title::short_title(prompt);
        with_write(vault, "side chat title", |tx| {
            tx.execute(
                "UPDATE sessions SET title = ?2 WHERE key = ?1 AND turn_count = 0",
                params![key, title],
            )
            .map(|_| ())
            .map_err(|e| format!("side chat title: {e}"))
        })?;
    }
    let input = ChatSendInput {
        prompt: prompt.to_string(),
        session_id: side.harness_session_id.clone(),
        session_key: Some(key.to_string()),
        cwd: Some(parent_row.cwd),
        model,
        permission_mode: Some(mode.to_string()),
        agent: Some(side.agent),
        client_turn_id,
        // The person typed it, so it teaches like any chat of theirs. It
        // continues from the parent on a fresh native session only (the
        // compiler ignores it on a resumed one).
        typed: true,
        continued_from: vec![parent],
        ..ChatSendInput::default()
    };
    Ok((input, effort))
}

// ── commands ──────────────────────────────────────────────────────────

/// The chat's side chat for the pane: its newest, or a new one (`fresh`
/// asks for a new one). Writes only a new empty side chat's own row.
#[tauri::command]
pub async fn side_chat_open(
    state: tauri::State<'_, AppState>,
    parent_key: String,
    fresh: Option<bool>,
) -> Result<AgentSessionSummary, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        open(&vault, &parent_key, fresh.unwrap_or(false), Utc::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The chat's earlier side chats, newest first. Reads only.
#[tauri::command]
pub async fn side_chat_list(
    state: tauri::State<'_, AppState>,
    parent_key: String,
) -> Result<Vec<AgentSessionSummary>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || list(&vault, &parent_key))
        .await
        .map_err(|e| e.to_string())?
}

/// A question in a side chat: queued like any chat turn (its run, budget and
/// usage rows are its own), in a mode no stronger than the parent's, seeded
/// from the parent on its first turn. `same_access` asks for the parent's own
/// mode instead of read only.
#[tauri::command]
pub async fn side_chat_send(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    key: String,
    prompt: String,
    client_turn_id: Option<String>,
    same_access: Option<bool>,
) -> Result<SendAccepted, String> {
    let vault = state.config.vault_path.clone();
    let (input, effort) = crate::vault::spawn_blocking_logged(move || {
        prepare(
            &vault,
            &key,
            &prompt,
            client_turn_id,
            same_access.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::commands::send_chat_with_effort(Some(&app), &state.config, input, effort, None).await
}

#[cfg(test)]
#[path = "side_chat_tests.rs"]
mod tests;
