//! Repository-changes command adapter (Paperclip PR 16). Thin and read-only:
//! the sidecar and its checks live in `repo_diff` and `automated_history`.

use super::AppState;
use crate::repo_diff::{self, RepoSidecar};
use tauri::State;

/// The repository changes (with bounded, redacted patches) recorded for
/// history run `run_id`, or `null` when the run has none. Display-only: the
/// receipt's Show patches button reads it; restore never does.
#[tauri::command]
pub async fn read_history_repo_changes(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<Option<RepoSidecar>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || repo_diff::read_sidecar(&vault, &run_id))
        .await
        .map_err(|e| e.to_string())?
}
