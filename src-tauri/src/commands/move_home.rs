//! Settings, Files & storage, Hollow folder: Move Hollow home
//! (`move_home.rs`). The dry run and status read only; start and rollback
//! stage the work for a detached helper and then quit Hollow 1.5 s later,
//! the same exit path as an install (`lifecycle::expect_exit`, then exit),
//! so the reply reaches the window first.

use super::AppState;
use crate::move_home::{self, Options, Plan, RollbackStaged, Roots, Started, Status};
use tauri::{AppHandle, State};

fn roots(state: &AppState) -> Result<Roots, String> {
    if state.config_path.as_os_str().is_empty() {
        return Err("Hollow has no config file yet, so there is nothing to move.".into());
    }
    Roots::from_config(&state.config_path)
}

/// Quit Hollow in 1.5 s, approved: the helper waits for this process.
pub(crate) fn quit_soon(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        crate::lifecycle::expect_exit();
        app.exit(0);
    });
}

/// The dry run: what moving Hollow home would do now. Changes nothing.
#[tauri::command]
pub async fn move_home_plan(
    options: Option<Options>,
    state: State<'_, AppState>,
) -> Result<Plan, String> {
    let roots = roots(&state)?;
    let options = options.unwrap_or_default();
    crate::vault::spawn_blocking_logged(move || move_home::plan(&roots, &options, None))
        .await
        .map_err(|e| e.to_string())?
}

/// Stage the move confirmed as `hash`, start the helper and quit Hollow.
#[tauri::command]
pub async fn move_home_start(
    hash: String,
    options: Option<Options>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Started, String> {
    let roots = roots(&state)?;
    let options = options.unwrap_or_default();
    let started = crate::vault::spawn_blocking_logged(move || {
        move_home::start(&roots, &options, &hash, None)
    })
    .await
    .map_err(|e| e.to_string())??;
    quit_soon(app);
    Ok(started)
}

/// Where the last move stands: its journal, checks and whether it can be
/// rolled back.
#[tauri::command]
pub async fn move_home_status(state: State<'_, AppState>) -> Result<Status, String> {
    let roots = roots(&state)?;
    crate::vault::spawn_blocking_logged(move || move_home::status(&roots))
        .await
        .map_err(|e| e.to_string())
}

/// Stage a rollback of the last move (or `backup_dir`) and quit Hollow; the
/// helper undoes it and opens Hollow again.
#[tauri::command]
pub async fn move_home_rollback(
    backup_dir: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RollbackStaged, String> {
    let roots = roots(&state)?;
    let staged = crate::vault::spawn_blocking_logged(move || {
        move_home::stage_rollback(&roots, backup_dir.map(std::path::PathBuf::from))
    })
    .await
    .map_err(|e| e.to_string())??;
    quit_soon(app);
    Ok(staged)
}
