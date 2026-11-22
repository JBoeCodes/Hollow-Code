//! Agent-session command adapters.
//!
//! Thin: identity and phase live in `agent_sessions`, durable rows live in
//! `session_store`. These only bridge IPC, matching the existing
//! `commands/{awareness,system,m365_auth}.rs` pattern.

use super::AppState;
use crate::types::{AgentSessionDetail, AgentSessionSummary, LiveAgentSession, SessionStoreStatus};
use crate::{agent_sessions, session_store};
use tauri::State;

/// Default page size for the session rail. Generous: rows are small and the
/// list is the primary way to find a past conversation.
const DEFAULT_LIMIT: usize = 200;

/// The chats Hollow's last quit interrupted mid-turn that this launch offers
/// to resume (overnight A1, `restart_resume`), newest first, then the chats
/// an agent started that it stopped (`agentStarted`, to open only; P1-40).
/// The person's card, and since L13 the COO's
/// `sessions.list_interrupted_chats`.
#[tauri::command]
pub async fn list_interrupted_chats(
    state: State<'_, AppState>,
) -> Result<Vec<crate::restart_resume::InterruptedChat>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::restart_resume::listed(&vault, chrono::Utc::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The person's **Resume ticked** on the interrupted-chats card: each ticked chat still
/// offered gets the manual Continue's text as a queued turn, with its own
/// Canvases choice and its own Ultracode switch (both live in the webview,
/// per conversation since P1-10). The COO's
/// `sessions.resume_interrupted_chats` (L13) resumes the same way, after its
/// own card ([`resume_interrupted_card`]).
#[tauri::command]
pub async fn resume_interrupted_chats(
    app: tauri::AppHandle,
    chats: Vec<crate::restart_resume::ResumeRequest>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::restart_resume::ResumeOutcome>, String> {
    crate::restart_resume::resume(Some(&app), &state.config, chats).await
}

/// How far back `sessions.steer_session` looks for the chat-start record
/// of the conversation it steers (as `list_sessions` `mine` does).
const STEER_CHILD_DAYS: i64 = 90;

/// `sessions.steer_session`'s authorization (COO parity L13 review): the
/// queue row of `key`'s running turn, when `caller` created that
/// conversation (a `start_session` child task of its run or, in a chat, of
/// its conversation's earlier runs) and that running turn is one the
/// caller asked for. A conversation the caller only sent to, a handover
/// successor, a turn that ended or is still queued, and a turn someone else
/// asked are refused: nothing is sent.
pub(crate) fn steerable_child(
    vault: &std::path::Path,
    caller: &crate::app_actions::Caller,
    key: &str,
) -> Result<i64, String> {
    let mut records = crate::delegation::for_run(vault, &caller.run_id);
    if caller.surface == "chat" {
        let since = chrono::Utc::now() - chrono::Duration::days(STEER_CHILD_DAYS);
        records.extend(
            session_store::child_delegations_for_session(
                vault,
                &caller.scope,
                since,
                crate::descendants::CHAIN_SESSIONS,
            )
            .unwrap_or_default()
            .iter()
            .filter_map(|json| {
                serde_json::from_str::<crate::run_manifest::DelegationRecord>(json).ok()
            }),
        );
    }
    let child_task = crate::delegation::Contract::ChildTask.as_str();
    let theirs: Vec<_> = records
        .into_iter()
        .filter(|r| r.kind == child_task && r.child_session.as_deref() == Some(key))
        .collect();
    if !theirs.iter().any(|r| r.tool == "start_session") {
        return Err(format!("steer_session steers only a conversation you started (sessions.start_session; list_sessions mine); {key} isn't one. Nothing was sent."));
    }
    theirs
        .iter()
        .filter_map(|r| crate::descendants::child_row(vault, r))
        .find(|row| {
            matches!(
                row.status,
                crate::run_queue::RowStatus::Claimed | crate::run_queue::RowStatus::Running
            )
        })
        .map(|row| row.id)
        .ok_or_else(|| format!("{key} has no running turn you asked for (its turn ended, is still queued, or someone else's turn runs now). Send it with sessions.send instead; nothing was sent."))
}

/// The session keys `sessions.resume_interrupted_chats` was given, trimmed,
/// each once, as the resume requests the person's card would send (Canvases
/// following the setting, Ultracode off, and no effort of its own named: the
/// chat's own level in the session store applies when the turn runs, else the
/// fixed level or Automatic).
pub(crate) fn resume_requests(keys: &[String]) -> Vec<crate::restart_resume::ResumeRequest> {
    let mut seen = std::collections::HashSet::new();
    keys.iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty() && seen.insert(key.clone()))
        .map(|session_key| crate::restart_resume::ResumeRequest {
            session_key,
            canvases: None,
            ultracode: false,
            effort: None,
        })
        .collect()
}

/// `sessions.resume_interrupted_chats`'s card (L13): each chat asked for
/// that is still offered, with its agent and the permission mode its
/// Continue turn runs in (the chat's own last one, at the top level, as the
/// person's Resume ticked would), flagged when that is stronger than the
/// asking chat's. Refused when none of them is waiting to resume.
pub(crate) fn resume_interrupted_card(
    cx: &crate::app_actions::Cx,
    a: &crate::app_actions::Args,
) -> Result<Option<crate::app_actions::Card>, String> {
    let asked = a.0.get("sessions").and_then(serde_json::Value::as_array);
    let keys: Vec<String> = asked
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect();
    let offered = crate::restart_resume::listed(&cx.vault(), chrono::Utc::now())?;
    let first = "Each chat below gets Continue as a new turn, as Resume ticked would.";
    let mut card =
        crate::app_actions::Card::new("Hollow app tools: resume interrupted chats").line(first);
    let mut any = false;
    for request in resume_requests(&keys) {
        let Some(chat) = offered
            .iter()
            .find(|c| c.session_key == request.session_key && !c.agent_started)
        else {
            continue;
        };
        any = true;
        let mode = chat.permission_mode.as_deref();
        let mut line = format!(
            "{} ({}, {}): {}",
            chat.title,
            chat.agent,
            mode.unwrap_or("its default mode"),
            chat.session_key
        );
        if let Some(stronger) = mode.and_then(|m| crate::app_sessions::escalation(&cx.caller, m)) {
            line.push_str(&format!(". {stronger}"));
        }
        card = card.line(line);
    }
    if !any {
        return Err("None of these chats is waiting to resume (sessions.list_interrupted_chats lists them). Nothing started.".into());
    }
    Ok(Some(card))
}

#[tauri::command(async)]
pub fn list_artifact_sessions(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::artifact_sessions::ArtifactSessionLink>, String> {
    crate::artifact_sessions::list(&state.config.vault_path, &path)
}

#[tauri::command(async)]
pub fn read_artifact_session_context(
    key: String,
    state: State<'_, AppState>,
) -> Result<crate::artifact_sessions::ArtifactSessionContext, String> {
    crate::artifact_sessions::context(&state.config.vault_path, &key)
}

#[tauri::command(async)]
pub fn list_agent_sessions(
    limit: Option<usize>,
    archived: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<AgentSessionSummary>, String> {
    session_store::list_sessions_by_archive(
        &state.config.vault_path,
        limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 1000),
        archived.unwrap_or(false),
    )
}

/// The window's cold open of a chat (WD1, 2026-11-18): the whole
/// conversation with every long tool result cut to a preview
/// (`session_store::lean_for_window`); [`read_tool_result`] brings one back
/// whole when its row expands. Agents' `sessions.read_agent_conversation`
/// reads [`read_agent_conversation_full`].
#[tauri::command(async)]
pub fn read_agent_conversation(
    key: String,
    state: State<'_, AppState>,
) -> Result<session_store::AgentConversation, String> {
    let mut conversation = read_agent_conversation_full(key, state)?;
    session_store::lean_for_window(&mut conversation);
    Ok(conversation)
}

/// A chat's whole conversation, every tool result whole (the agents' read).
pub fn read_agent_conversation_full(
    key: String,
    state: State<'_, AppState>,
) -> Result<session_store::AgentConversation, String> {
    let mut conversation = session_store::read_conversation(&state.config.vault_path, &key)?;
    // The files each message was sent with, so a reloaded chat keeps its
    // chips (B1a, ENG-104).
    crate::phone_access::attachments::fill_conversation(
        &state.config.vault_path,
        &mut conversation,
    );
    Ok(conversation)
}

/// WD1: one stored tool call's full result, for a row the lean cold open
/// sent as a preview. Window only, reads only.
#[tauri::command(async)]
pub fn read_tool_result(
    key: String,
    ord: i64,
    tool_id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    session_store::read_tool_result(&state.config.vault_path, &key, ord, &tool_id)
}

#[tauri::command]
pub fn archive_agent_session(
    key: String,
    archived: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Refused while a turn is queued or running (P1-39), so a background
    // turn's services are never stopped under it.
    session_store::archive_session(&state.config.vault_path, &key, archived)?;
    // Archiving ends the chat, and with it the bounded services it started
    // (merged plan decision 1); a bypass service is not tied to its chat.
    if archived {
        crate::services::stop_chat(&key);
        crate::terminal::stop_chat(&key);
    }
    Ok(())
}

#[tauri::command]
pub fn rename_agent_session(
    key: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    session_store::rename_session(&state.config.vault_path, &key, &title)
}

/// Regenerate title (t3code parity 4.7): a fresh model title from the
/// conversation's first request, on its own agent, replacing the current one.
#[tauri::command]
pub async fn regenerate_chat_title(
    key: String,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let core = crate::core_host::Core::from(&app);
    crate::chat_title::regenerate_now(Some(&core), &state.config.vault_path, &key).await
}

#[tauri::command(async)]
pub fn list_conversation_organization(
    state: State<'_, AppState>,
) -> Result<Vec<session_store::organization::Organization>, String> {
    session_store::organization::list(&state.config.vault_path)
}

#[tauri::command]
pub fn organize_conversation(
    change: session_store::organization::Change,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<session_store::organization::Organization, String> {
    let result = session_store::organization::change(&state.config.vault_path, &change)?;
    if result.settled {
        crate::services::stop_chat(&change.key);
        crate::terminal::stop_chat(&change.key);
    }
    use tauri::Emitter;
    let _ = app.emit(
        crate::app_broker::CHANGED_EVENT,
        serde_json::json!({ "action": "sessions.organize", "area": "sessions" }),
    );
    Ok(result)
}

#[tauri::command]
pub async fn configure_conversation_settlement(
    change: session_store::organization::Change,
    state: State<'_, AppState>,
) -> Result<session_store::organization::Organization, String> {
    session_store::organization::settlement::configure(&state.config.vault_path, &change).await
}

#[tauri::command]
pub async fn sweep_conversation_settlement(
    protected: Vec<String>,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<Vec<session_store::organization::Organization>, String> {
    let rows = session_store::organization::settlement::sweep(&state.config.vault_path, &protected)
        .await?;
    for row in &rows {
        crate::services::stop_chat(&row.key);
        crate::terminal::stop_chat(&row.key);
    }
    if !rows.is_empty() {
        use tauri::Emitter;
        let _ = app.emit(
            crate::app_broker::CHANGED_EVENT,
            serde_json::json!({ "action": "sessions.auto_settle", "area": "sessions" }),
        );
    }
    Ok(rows)
}

/// Move a conversation to another workspace (drag onto a rail header, or
/// Move to…), inside the workspace edit lease so no full-vault run captures
/// the record move. Refused while the conversation has a turn in flight.
/// Emits `app-changed` (area `sessions`) so every window's rail refreshes.
#[tauri::command]
pub async fn move_agent_session(
    app: tauri::AppHandle,
    key: String,
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<session_store::SessionMove, String> {
    let vault = state.config.vault_path.clone();
    let moved = crate::vault::spawn_blocking_logged(move || {
        session_store::move_agent_session_with(&vault, &key, &workspace_id, || {
            crate::automated_history::workspace_edit_lease(&vault).map_err(String::from)
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    if moved.moved {
        use tauri::Emitter;
        let _ = app.emit(
            crate::app_broker::CHANGED_EVENT,
            serde_json::json!({ "action": "sessions.move_agent_session", "area": "sessions" }),
        );
    }
    Ok(moved)
}

#[tauri::command(async)]
pub fn read_agent_session(
    key: String,
    state: State<'_, AppState>,
) -> Result<AgentSessionDetail, String> {
    session_store::read_session(&state.config.vault_path, &key)
}

/// The in-flight CHAT turn's phase, or `None`. Chat-only: it does not see
/// workflow, heartbeat, ingest, tracker sync, scheduler or history-clear holders.
/// Whether the vault is held is answered by the lease alone
/// (`automated_history::lease_holder`); treat `None` here as "no chat turn",
/// never as "the vault is free". Since Paperclip PR 7b a chat turn still
/// waiting in the run queue is reported too, as `phase: 'queued'` with an
/// empty run id, so Resume session refuses while a turn is queued. Reads the
/// queue (SQLite) on a blocking worker.
#[tauri::command]
pub async fn agent_session_live(
    key: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<LiveAgentSession>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || live_session(&vault, key.as_deref()))
        .await
        .map_err(|e| e.to_string())
}

/// The live record of `key`'s running turn (any conversation's without a
/// key), else its live chat queue row, else (with a key, P1a) the chat's
/// paused record as phase `paused`. Several conversations can be live at
/// once since build programme PR B4.
fn live_session(vault: &std::path::Path, key: Option<&str>) -> Option<LiveAgentSession> {
    let live = match key {
        Some(key) => agent_sessions::live_for_key(key),
        None => agent_sessions::live(),
    };
    live.or_else(|| crate::chat::queued_live_session(vault, key))
        .or_else(|| {
            let paused = crate::chat_pause::get(vault, key?).ok().flatten()?;
            Some(crate::chat_pause::live_session(&paused))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_queue::{self, NewRow, RunSource};
    use chrono::Utc;

    /// 2026-11-12: a COO-triggered resume names no effort, and the chat's own
    /// level (kept in the session store) is what its Continue turn runs at.
    #[test]
    fn a_coo_resume_runs_at_the_chats_own_effort() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let key = "s-20261112-resume01".to_string();
        crate::session_store::set_chat_effort(v, &key, Some("max")).unwrap();
        let requests = resume_requests(&[key.clone(), format!(" {key} ")]);
        assert_eq!(requests.len(), 1, "each chat once");
        assert_eq!(requests[0].effort, None, "the request names none");
        assert_eq!(
            crate::chat::resolve_requested_effort(v, &key, requests[0].effort.clone()).as_deref(),
            Some("max")
        );
    }

    /// Paperclip PR 7b: with no in-memory record, a queued chat row is
    /// reported as `queued`, with its key and agent and no run id.
    #[test]
    fn agent_session_live_reports_a_queued_chat_turn() {
        let _live = agent_sessions::LIVE_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        agent_sessions::clear();
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        assert!(
            live_session(v, None).is_none(),
            "nothing live, nothing queued"
        );

        let key = "s-20260925-0000q001";
        run_queue::enqueue(
            v,
            NewRow::new(RunSource::Chat, Some(key), "Chat", "chat")
                .with_payload(serde_json::json!({ "agent": "codex", "clientTurnId": "ct-1" })),
            Utc::now(),
        )
        .unwrap();
        let live = live_session(v, None).expect("the queued turn is live");
        assert_eq!(live.key, key);
        assert_eq!(live.agent, "codex");
        // Working, not Queued (2026-11-16): nothing holds this row back, so
        // it is being prepared; only a real wait reads `queued`.
        assert_eq!(live.phase, "preparing");
        assert_eq!(live.run_id, "");
        // A vault-root turn behind another run's vault lane truly waits.
        let behind = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-20260925-0000q002"), "Chat", "chat")
                .with_payload(serde_json::json!({ "agent": "codex" })),
            Utc::now(),
        )
        .unwrap()
        .id();
        let blocker = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Manual, Some("brief"), "Morning brief", "run now"),
            Utc::now(),
        )
        .unwrap()
        .id();
        run_queue::claim(v, blocker, Utc::now()).unwrap();
        run_queue::mark_running(v, blocker, "run-brief", Utc::now()).unwrap();
        let held = live_session(v, Some("s-20260925-0000q002")).expect("live");
        assert_eq!(held.phase, "queued");
        run_queue::finish(v, blocker, run_queue::RowStatus::Done, None, Utc::now()).unwrap();
        run_queue::cancel(v, behind, Utc::now()).unwrap();

        // The running turn's own record wins over its queue row.
        agent_sessions::begin(
            key,
            crate::agent::AgentKind::Codex,
            "run-live",
            agent_sessions::SessionPhase::Running,
        );
        assert_eq!(
            live_session(v, None).map(|l| l.phase).as_deref(),
            Some("running")
        );
        agent_sessions::clear();
    }
}

#[tauri::command(async)]
pub fn session_store_status(state: State<'_, AppState>) -> Result<SessionStoreStatus, String> {
    session_store::status(&state.config.vault_path)
}
