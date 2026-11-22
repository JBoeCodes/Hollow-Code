//! Question-card command adapters (Paperclip PR 13).
//!
//! Thin, like the other `commands/` adapters: the rules live in
//! `interactions`. Every command touches SQLite (and `resolve_interaction`
//! re-hashes a card's target file), so each runs on a blocking worker.
//! Refusals are plain strings with the `INTERACTION_*` prefixes the composer
//! keys on (Tauri rejects with the string itself).

use super::AppState;
use crate::interactions;
use crate::types::{ChatInteraction, ResolvedInteraction};
use serde_json::Value;
use tauri::State;

/// A conversation's open question cards (pending and stale), oldest first.
/// With `chain` (the chat on screen, one Main conversation 2026-11-07), a
/// current conversation's list also holds the open cards of the chats
/// before it in its chain, first: a card asked in a chat that has since
/// continued here is answered here (`chat_chain::chain_keys`).
#[tauri::command]
pub async fn list_pending_interactions(
    session_key: String,
    chain: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<ChatInteraction>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        let own = interactions::list_pending(&vault, &session_key)?;
        if chain != Some(true) {
            return Ok(own);
        }
        let earlier = crate::chat_chain::chain_keys(&vault, &session_key);
        if earlier.is_empty() {
            return Ok(own);
        }
        // Newest first from the store: oldest first here, before its own.
        let mut out: Vec<ChatInteraction> = interactions::list_open_every(&vault)?
            .into_iter()
            .filter(|card| earlier.contains(&card.session_key))
            .collect();
        out.reverse();
        out.extend(own);
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Every conversation's open cards, newest first (next-level P19 activity).
#[tauri::command]
pub async fn list_open_interactions(
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<ChatInteraction>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        interactions::list_open_all(&vault, limit.unwrap_or(50))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Answer a pending card. Returns the card and the exact prompt to send as the
/// next turn, through the normal send path with the same native session.
#[tauri::command]
pub async fn resolve_interaction(
    id: String,
    answer: Value,
    native_session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ResolvedInteraction, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        interactions::resolve(&vault, &id, &answer, native_session_id.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Close an open card without sending anything.
#[tauri::command]
pub async fn dismiss_interaction(
    id: String,
    state: State<'_, AppState>,
) -> Result<ChatInteraction, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || interactions::dismiss(&vault, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Put an answered card back to pending when its answer turn never started.
#[tauri::command]
pub async fn reopen_interaction(
    id: String,
    state: State<'_, AppState>,
) -> Result<ChatInteraction, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || interactions::reopen(&vault, &id))
        .await
        .map_err(|e| e.to_string())?
}
