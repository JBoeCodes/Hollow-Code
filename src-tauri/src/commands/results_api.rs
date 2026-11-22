//! Results within a workspace and Your work digests (next-level P02/P05).
//! Thin adapters over `workspace_results`; the store and its rules live there.
use super::AppState;
use crate::workspace_results::{self, WorkResult, WorkspaceDigest};
use tauri::State;

/// Every deliverable in a workspace's `outputs/`, with origin, checks and review state.
#[tauri::command]
pub async fn workspace_results(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorkResult>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspace_results::list(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// [`workspace_results`] for the agent action: a damaged review store is an
/// error but is never moved aside, so acceptance records stay in place for
/// the user's Results view, which moves it and says where (B·F08).
pub(crate) async fn workspace_results_unattended(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<WorkResult>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspace_results::list_unattended(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Record the user's review (draft, ready or accepted) of the version they
/// saw. Only Hollow's own window invokes Tauri commands, so this caller is
/// the person: the one path that can accept (owner decision 3).
#[tauri::command]
pub async fn set_result_review(
    id: String,
    path: String,
    review: String,
    expected_sha256: Option<String>,
    note: Option<String>,
    state: State<'_, AppState>,
) -> Result<WorkResult, String> {
    let root = state.config.vault_path.clone();
    record_review(
        root,
        id,
        path,
        review,
        expected_sha256,
        note,
        workspace_results::Reviewer::Person,
    )
    .await
}

/// An agent or automation run's review (`hollow_app`'s
/// `workspaces.set_result_review`): draft or ready for review only; asking
/// for `accepted` is refused, because only a person accepts (decision 3).
pub async fn agent_set_result_review(
    id: String,
    path: String,
    review: String,
    expected_sha256: Option<String>,
    note: Option<String>,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<WorkResult, String> {
    let root = state.config.vault_path.clone();
    record_review(
        root,
        id,
        path,
        review,
        expected_sha256,
        note,
        workspace_results::Reviewer::Agent { run_id },
    )
    .await
}

async fn record_review(
    root: std::path::PathBuf,
    id: String,
    path: String,
    review: String,
    expected_sha256: Option<String>,
    note: Option<String>,
    reviewer: workspace_results::Reviewer,
) -> Result<WorkResult, String> {
    crate::vault::spawn_blocking_logged(move || {
        workspace_results::set_review(
            &root,
            &id,
            &path,
            &review,
            expected_sha256.as_deref(),
            note.as_deref(),
            &reviewer,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The accepted text of a result (null when no snapshot was kept).
#[tauri::command]
pub async fn result_snapshot(
    id: String,
    path: String,
    sha256: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        workspace_results::snapshot(&root, &id, &path, &sha256)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Next step, latest result and review backlog for a few workspaces (Your work).
#[tauri::command]
pub async fn workspace_digests(
    ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<WorkspaceDigest>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspace_results::digests(&root, &ids))
        .await
        .map_err(|e| e.to_string())?
}

/// A result's preview and absolute path (outputs only).
#[tauri::command]
pub async fn result_preview(
    id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<workspace_results::ResultPreview, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspace_results::preview(&root, &id, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// Results across workspaces for Library (one kind optional), newest first.
#[tauri::command]
pub async fn list_all_results(
    kind: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<workspace_results::LibraryResult>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        workspace_results::list_all(&root, kind.as_deref(), limit.unwrap_or(500))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// [`list_all_results`] for Library's All: a view the user did not open to
/// see the review store, so a damaged one is an error and stays put, never
/// moved aside (see [`workspace_results_unattended`]). Library's Results
/// grid keeps the moving [`list_all_results`].
#[tauri::command]
pub async fn list_all_results_peek(
    kind: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<workspace_results::LibraryResult>, String> {
    list_all_results_unattended(kind, limit, state).await
}

/// [`list_all_results`] for the agent action (see
/// [`workspace_results_unattended`]).
pub(crate) async fn list_all_results_unattended(
    kind: Option<String>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<workspace_results::LibraryResult>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        list_all_peek(&root, kind.as_deref(), limit.unwrap_or(500))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The never-moving listing behind [`list_all_results_peek`] and the agent action.
fn list_all_peek(
    root: &std::path::Path,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<workspace_results::LibraryResult>, String> {
    workspace_results::list_all_unattended(root, kind, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_peek_listing_reports_a_damaged_store_and_leaves_it_in_place() {
        let root = tempfile::tempdir().unwrap();
        let w = crate::workspaces::create(root.path(), "Review", "Make a brief").unwrap();
        let dir = std::path::Path::new(&w.cwd).to_path_buf();
        std::fs::write(dir.join("outputs/a.md"), "# A").unwrap();
        std::fs::create_dir_all(dir.join(".hollow")).unwrap();
        let store = dir.join(".hollow/results.json");
        let damaged = b"{\"results\": \xff".to_vec();
        std::fs::write(&store, &damaged).unwrap();
        let err = list_all_peek(root.path(), None, 100).unwrap_err();
        assert!(
            err.contains("could not be read") && err.contains("left as is"),
            "{err}"
        );
        assert_eq!(std::fs::read(&store).unwrap(), damaged, "still in place");
        assert!(crate::state_file::corrupt_copies(&store).is_empty());
    }
}
