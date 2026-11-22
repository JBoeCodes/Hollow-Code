//! Agenda (next-level P10): read-only calendar events and local focus blocks.
use super::AppState;
use crate::agenda::{self, FocusBlock};
use tauri::State;

#[tauri::command]
pub async fn calendar_source_status() -> Result<serde_json::Value, String> {
    crate::vault::spawn_blocking_logged(crate::calendar_source::status)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_calendar_source(
    settings: crate::calendar_source::Settings,
    request_access: Option<bool>,
) -> Result<serde_json::Value, String> {
    crate::vault::spawn_blocking_logged(move || {
        crate::calendar_source::save(settings, request_access == Some(true))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_calendar_settings(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars",
            None::<String>,
        )
        .map_err(|e| e.to_string())
}

/// Read-only events in `[from, to)` from the chosen Agenda source, with freshness
/// and coverage. Dates mean local midnight; inaccessible sources are a state.
#[tauri::command]
/// A complete read of the same window in the last minute is reused unless
/// `fresh` (the Agenda's Refresh and Retry).
pub async fn agenda_events(
    from: String,
    to: String,
    fresh: Option<bool>,
) -> Result<serde_json::Value, String> {
    let reuse = if fresh == Some(true) {
        agenda::Reuse::Fresh
    } else {
        agenda::Reuse::Kept
    };
    Ok(agenda::events_with(&from, &to, reuse).await)
}

#[tauri::command]
pub async fn list_focus_blocks(
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<Vec<FocusBlock>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || agenda::list(&root, &from, &to))
        .await
        .map_err(|e| e.to_string())?
}

/// [`list_focus_blocks`] for the agent action: a damaged store is an error
/// but stays for the Agenda to move aside and report (B·F08).
pub(crate) async fn list_focus_blocks_unattended(
    from: String,
    to: String,
    state: State<'_, AppState>,
) -> Result<Vec<FocusBlock>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || agenda::list_unattended(&root, &from, &to))
        .await
        .map_err(|e| e.to_string())?
}

/// Create or update (compare-and-swap on `updatedAt`) a local focus block.
#[tauri::command]
pub async fn save_focus_block(
    block: FocusBlock,
    state: State<'_, AppState>,
) -> Result<FocusBlock, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || agenda::save(&root, block))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn delete_focus_block(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || agenda::delete(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}
