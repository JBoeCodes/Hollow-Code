//! The "needs you" query, the exact-key conversation lookup and the
//! palette's conversation search (merged plan slice 4, B·F15), with the
//! search's redacted twin for the agent action. Thin, like
//! the other adapters: the rules live in `needs_you` and `workspaces`. Each
//! reads SQLite and the vault, so each runs on a blocking worker.

use super::AppState;
use crate::session_store::MessageText;
use crate::{needs_you, workspaces};
use tauri::State;

/// One page of everything waiting on the person, each with its exact id.
/// `cursor` is a previous page's opaque `nextCursor` (a keyset, not an
/// offset). `kinds` and `categories` (a review item's kind, say) both filter
/// before paging.
#[tauri::command]
pub async fn list_needs_you(
    kinds: Option<Vec<String>>,
    categories: Option<Vec<String>>,
    cursor: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<needs_you::NeedsYouPage, String> {
    let root = state.config.vault_path.clone();
    let approvals = crate::approvals::pending_cards();
    crate::vault::spawn_blocking_logged(move || {
        // Ask-canvases outside every workspace live in app data; without it
        // they are left out and named unavailable.
        let app_data = crate::agent_settings::app_data_dir();
        let mut page = needs_you::list_matching(
            &root,
            app_data.as_ref().ok().map(|p| p.as_path()),
            approvals,
            kinds.as_deref(),
            categories.as_deref(),
            cursor.as_deref(),
            limit,
        )?;
        let canvases = kinds
            .as_ref()
            .is_none_or(|k| k.iter().any(|k| k == "canvas"));
        if app_data.is_err() && canvases {
            page.unavailable.push("canvases outside workspaces".into());
        }
        Ok(page)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One page of scrubbed full-text chat matches, with saved memories, nearby
/// context and workspace priority. The sidebar and palette share the index.
/// `cursor` is a previous page's opaque `nextCursor`.
#[tauri::command]
pub async fn search_sessions(
    query: String,
    cursor: Option<String>,
    limit: Option<usize>,
    workspace_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<workspaces::SessionSearchPage, String> {
    search_sessions_as(
        query,
        cursor,
        limit,
        workspace_id,
        state,
        MessageText::AsWritten,
    )
    .await
}

/// [`search_sessions`] for the agent action (`sessions.search_sessions`):
/// messages are matched and snipped only as `claude_runner::redact` leaves
/// them, as the session reads redact (review B·F11).
pub(crate) async fn search_sessions_redacted(
    query: String,
    cursor: Option<String>,
    limit: Option<usize>,
    workspace_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<workspaces::SessionSearchPage, String> {
    search_sessions_as(
        query,
        cursor,
        limit,
        workspace_id,
        state,
        MessageText::Redacted,
    )
    .await
}

async fn search_sessions_as(
    query: String,
    cursor: Option<String>,
    limit: Option<usize>,
    workspace_id: Option<String>,
    state: State<'_, AppState>,
    text: MessageText,
) -> Result<workspaces::SessionSearchPage, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        let page = workspaces::search_sessions_in(
            &root,
            &query,
            cursor.as_deref(),
            limit,
            text,
            workspace_id.as_deref(),
        );
        // What the search's budget left unindexed is finished off its path.
        crate::session_store::drain_search_index_soon(&root);
        page
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Where conversation `key` lives, found by its exact key (archived and
/// older-than-the-rail conversations included); `None` when none has it.
#[tauri::command]
pub async fn session_target(
    key: String,
    state: State<'_, AppState>,
) -> Result<Option<workspaces::SessionTarget>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::session_target(&root, &key))
        .await
        .map_err(|e| e.to_string())?
}

/// The vault's iCloud/Finder conflict copies whose original sits beside them
/// (overnight A10).
#[tauri::command]
pub async fn conflict_copies(
    state: State<'_, AppState>,
) -> Result<Vec<crate::conflict_copies::ConflictCopy>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::conflict_copies::scan(&vault))
        .await
        .map_err(|e| e.to_string())
}

/// Both texts and hashes of one copy and its original, for the person's
/// diff (and the COO's `vault.conflict_copy_pair`, L13): the resolve binds
/// these hashes.
#[tauri::command]
pub async fn conflict_copy_pair(
    copy: String,
    state: State<'_, AppState>,
) -> Result<crate::conflict_copies::ConflictPair, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::conflict_copies::pair(&vault, &copy))
        .await
        .map_err(|e| e.to_string())?
}

/// The person marks a copy "Not a copy" (morning M7; also the COO's
/// `vault.dismiss_conflict_copy`, L13): never offered again; nothing in the
/// vault changes. Returns its original.
#[tauri::command]
pub async fn dismiss_conflict_copy(
    copy: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::conflict_copies::dismiss(&vault, &copy))
        .await
        .map_err(|e| e.to_string())?
}

/// Undo of [`dismiss_conflict_copy`] (also `vault.undismiss_conflict_copy`).
#[tauri::command]
pub async fn undismiss_conflict_copy(
    copy: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::conflict_copies::undismiss(&vault, &copy))
        .await
        .map_err(|e| e.to_string())?
}

/// The person keeps one of the two: the other goes to the vault's
/// `.trash/`. Refused when either changed since `copyHash` /
/// `originalHash`. Returns the path kept. The COO's
/// `vault.resolve_conflict_copy` (L13) runs inside its run's History instead
/// (`conflict_copies::resolve_for_live_run`).
#[tauri::command]
pub async fn resolve_conflict_copy(
    copy: String,
    keep_copy: bool,
    copy_hash: String,
    original_hash: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::conflict_copies::resolve(&vault, &copy, keep_copy, &copy_hash, &original_hash)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The person's answer to "Delete the memory X?" (build run L2): Delete
/// (`delete`) removes only that memory file and its line in its folder's
/// `MEMORY.md`, a copy kept in the vault's `.trash/` and a History row
/// added; Keep records the choice. Either way it is not offered again.
/// Returns the memory's file name.
#[tauri::command]
pub async fn answer_lesson_memory(
    source_id: String,
    delete: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::lesson_memories::answer(&vault, &source_id, delete)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Undo of a Delete from [`answer_lesson_memory`]: the file and its
/// `MEMORY.md` line come back; the memory then counts as kept.
#[tauri::command]
pub async fn restore_lesson_memory(
    source_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::lesson_memories::restore(&vault, &source_id))
        .await
        .map_err(|e| e.to_string())?
}
