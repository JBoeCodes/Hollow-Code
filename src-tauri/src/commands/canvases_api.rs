//! Saved chat canvases (Canvas C3 · S2). Thin adapters over
//! `workspace_canvases`; the store, its paths and its rules live there.
//!
//! Every command names its store the same way: `workspaceId` for a
//! workspace's `outputs/canvases/` (then `sessionKey` is only recorded as the
//! conversation a revision came from), or `sessionKey` alone for the app-data
//! store of a conversation outside any workspace.
use super::AppState;
use crate::workspace_canvases::{
    self, Canvas, CanvasRecorded, CanvasRevisionHtml, CanvasStateSaved, CanvasSummary, Scope,
};
use serde_json::Value;
use std::path::PathBuf;
use tauri::State;

fn scope<'a>(workspace_id: &'a Option<String>, session_key: &'a Option<String>) -> Scope<'a> {
    Scope {
        workspace_id: workspace_id.as_deref(),
        session_key: session_key.as_deref(),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce(PathBuf, PathBuf) -> Result<T, String> + Send + 'static,
    state: &State<'_, AppState>,
) -> Result<T, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        let app_data = crate::agent_settings::app_data_dir()?;
        f(vault, app_data)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Log one refused bridge message (Canvas C6 P1) in the app data
/// directory's bounded refusal log (`canvas_bridge_log`): its kind, size,
/// reason, canvas id and conversation, never its payload. Rate-limited.
#[tauri::command]
pub async fn record_canvas_bridge_refusal(
    refusal: crate::canvas_bridge_log::BridgeRefusal,
) -> Result<(), String> {
    crate::vault::spawn_blocking_logged(move || {
        let app_data = crate::agent_settings::app_data_dir()?;
        crate::canvas_bridge_log::log(&app_data, &refusal, chrono::Utc::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The saved canvases in a workspace, or in a conversation's app-data store.
#[tauri::command]
pub async fn list_canvases(
    workspace_id: Option<String>,
    session_key: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<CanvasSummary>, String> {
    blocking(
        move |vault, app| {
            workspace_canvases::list(&vault, &app, scope(&workspace_id, &session_key))
        },
        &state,
    )
    .await
}

/// One saved canvas (current HTML, state, revisions), or null if never saved.
#[tauri::command]
pub async fn read_canvas(
    workspace_id: Option<String>,
    session_key: Option<String>,
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<Canvas>, String> {
    blocking(
        move |vault, app| {
            workspace_canvases::read(&vault, &app, scope(&workspace_id, &session_key), &id)
        },
        &state,
    )
    .await
}

/// One kept revision's fence body.
#[tauri::command]
pub async fn canvas_item_source(
    webview: tauri::Webview,
    workspace_id: String,
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::canvas_items::CanvasItemSource, String> {
    if webview.label() != "main" {
        return Err("Open Add items from Hollow's main window.".into());
    }
    blocking(
        move |vault, app| crate::canvas_items::source(&vault, &app, &workspace_id, &id),
        &state,
    )
    .await
}

#[tauri::command]
pub async fn canvas_plan_items(
    webview: tauri::Webview,
    request: crate::canvas_items::CanvasPlanItems,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if webview.label() != "main" {
        return Err("Confirm canvas items in Hollow's main window.".into());
    }
    blocking(
        move |vault, app| crate::canvas_items::plan(&vault, &app, &request),
        &state,
    )
    .await
}

/// One kept revision's fence body.
#[tauri::command]
pub async fn read_canvas_revision(
    workspace_id: Option<String>,
    session_key: Option<String>,
    id: String,
    revision: u64,
    state: State<'_, AppState>,
) -> Result<CanvasRevisionHtml, String> {
    blocking(
        move |vault, app| {
            workspace_canvases::read_revision(
                &vault,
                &app,
                scope(&workspace_id, &session_key),
                &id,
                revision,
            )
        },
        &state,
    )
    .await
}

/// Save a finished `hollow-canvas` fence the chat rendered: a new canvas, a
/// new revision, or nothing when the body is already kept. Refused with
/// `CANVAS_CHANGED:` when the sidecar is no longer `expectedSha256`.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn record_canvas(
    workspace_id: Option<String>,
    session_key: Option<String>,
    body: String,
    turn_id: Option<String>,
    kit: Option<String>,
    expected_sha256: Option<String>,
    state: State<'_, AppState>,
) -> Result<CanvasRecorded, String> {
    let now = chrono::Utc::now().to_rfc3339();
    blocking(
        move |vault, app| {
            workspace_canvases::record(
                &vault,
                &app,
                scope(&workspace_id, &session_key),
                &body,
                turn_id.as_deref(),
                kit.as_deref(),
                expected_sha256.as_deref(),
                &now,
            )
        },
        &state,
    )
    .await
}

/// Save the canvas's UI state (bridge `state`, ≤16 KB); refused with
/// `CANVAS_CHANGED:` when the sidecar is no longer `expectedSha256`.
#[tauri::command]
pub async fn save_canvas_state(
    workspace_id: Option<String>,
    session_key: Option<String>,
    id: String,
    canvas_state: Value,
    expected_sha256: String,
    state: State<'_, AppState>,
) -> Result<CanvasStateSaved, String> {
    let now = chrono::Utc::now().to_rfc3339();
    blocking(
        move |vault, app| {
            workspace_canvases::save_state(
                &vault,
                &app,
                scope(&workspace_id, &session_key),
                &id,
                canvas_state,
                &expected_sha256,
                &now,
            )
        },
        &state,
    )
    .await
}
