//! Operator-context and local system-inspection command adapters.

use super::AppState;
use crate::types::Diagnostics;
use crate::{chat, operator, vault_people};
use tauri::State;

#[tauri::command]
pub fn operator_context(state: State<'_, AppState>) -> operator::OperatorContext {
    operator::compile(&state.config.vault_path)
}

#[tauri::command]
pub fn mark_operator_profile_reviewed(
    path: String,
    state: State<'_, AppState>,
) -> Result<operator::OperatorFreshness, String> {
    operator::mark_reviewed(&state.config.vault_path, &path)
}

/// The persisted About you switch, which the window adopts at launch
/// (the backend's value is the source of truth; see `operator::Switch`).
#[tauri::command]
pub fn operator_context_switch() -> operator::SwitchState {
    operator::switch_state()
}

/// An explicit change of the About you switch, persisted for later launches.
#[tauri::command]
pub fn set_operator_context_enabled(enabled: bool) {
    operator::set_enabled(enabled);
}

/// The notification scope the user is watching (Paperclip PR 22):
/// `workflow:<slug>` while that workflow is open in Automations, null
/// otherwise. A notification opening exactly that scope skips its native
/// banner while the main window is focused (see
/// `notifications::foreground_suppressed_decision`).
#[tauri::command]
pub fn set_foreground_notification_scope(scope: Option<String>) {
    crate::notifications::set_foreground_scope(scope);
}

#[tauri::command]
pub fn list_people(state: State<'_, AppState>) -> Vec<vault_people::Person> {
    vault_people::list(&state.config.vault_path)
}

#[tauri::command]
pub fn list_activity_ledger(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Vec<crate::activity_ledger::LedgerEntry> {
    crate::activity_ledger::read(&state.config.vault_path, limit.unwrap_or(200))
}

#[tauri::command]
pub fn diagnostics(state: State<'_, AppState>) -> Diagnostics {
    let vault_path = state.config.vault_path.clone();
    let vault_exists = vault_path.exists();
    let config_path = state.config_path.clone();
    let claude_binary = chat::find_claude_binary().map(|p| path_str(&p));
    let debug_log_dir = dirs::home_dir().map(|h| crate::instance::dot_cache_dir(&h));
    let debug_log_files = debug_log_dir
        .as_ref()
        .map(|d| {
            if !d.exists() {
                return 0usize;
            }
            std::fs::read_dir(d)
                .ok()
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("log"))
                        .count()
                })
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let debug_log_enabled = matches!(
        std::env::var("HOLLOW_DEBUG_LOG").ok().as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on"),
    );
    let context_health = crate::context_layers::health(&vault_path);
    Diagnostics {
        vault_path: path_str(&vault_path),
        vault_exists,
        config_path: path_str(&config_path),
        claude_binary,
        debug_log_dir: debug_log_dir
            .as_ref()
            .map(|d| path_str(d))
            .unwrap_or_default(),
        debug_log_enabled,
        debug_log_files,
        context_health,
    }
}

/// "Save diagnostics…" (overnight C6): a redacted zip of versions, health,
/// receipt metadata and install-log tails into `dir`, a folder outside the
/// vault the person chose. Never overwrites, never sends.
#[tauri::command]
pub async fn save_diagnostics(
    dir: String,
    state: State<'_, AppState>,
) -> Result<crate::diagnostics_bundle::Saved, String> {
    write_diagnostics(state.config.vault_path.clone(), dir).await
}

/// The body of `save_diagnostics`, also the COO's `system.save_diagnostics`
/// (it gives the folder in words; there is no picker). Only the redacted zip
/// `diagnostics_bundle` builds, saved under a new name in `dir`; the result
/// is its path, size and entry count.
pub(crate) async fn write_diagnostics(
    vault: std::path::PathBuf,
    dir: String,
) -> Result<crate::diagnostics_bundle::Saved, String> {
    crate::vault::spawn_blocking_logged(move || {
        let sources = crate::diagnostics_bundle::Sources::for_this_mac(&vault)?;
        crate::diagnostics_bundle::save(&sources, std::path::Path::new(&dir), chrono::Local::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Disk space and Hollow's largest folders (overnight D1). Read-only; sizing
/// runs `du` for up to 20 s.
#[tauri::command]
pub async fn disk_guard(
    threshold_gb: Option<u64>,
    state: State<'_, AppState>,
) -> Result<crate::disk_guard::Report, String> {
    let home = dirs::home_dir().ok_or("Hollow can't find your home folder.")?;
    let worktrees = crate::disk_guard::worktrees_root(&state.config_path);
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::disk_guard::report(
            &home,
            Some(&vault),
            worktrees.as_deref(),
            threshold_gb.unwrap_or_else(crate::disk_guard::threshold_gb),
            true,
        )
    })
    .await
    .map_err(|e| e.to_string())
}

/// Settings, Advanced, "Folders agents can run tools from" (A12): the PATH
/// every agent CLI gets, folder by folder with where each came from and
/// whether it exists, and whether the login shell's PATH has been read.
/// Read-only; checks each folder on disk off the main thread.
#[tauri::command]
pub async fn agent_tool_path() -> Result<crate::shell_path::AgentToolPath, String> {
    crate::vault::spawn_blocking_logged(crate::shell_path::agent_tool_path)
        .await
        .map_err(|e| e.to_string())
}

/// Settings, Advanced, Disk space, "Warn under" (morning N6): the
/// threshold the daily bell and the report use, 1–10,000 GB.
#[tauri::command]
pub fn set_disk_threshold(threshold_gb: u64) -> Result<u64, String> {
    crate::disk_guard::set_threshold_gb(threshold_gb)
}

/// The saved "Warn under", read without measuring, so the field shows before
/// the first check (P1-22). `disk_guard` reports the same value.
#[tauri::command]
pub fn disk_threshold() -> u64 {
    crate::disk_guard::threshold_gb()
}

fn databases_support() -> Result<std::path::PathBuf, String> {
    crate::hollow_databases::support_root()
        .ok_or_else(|| "Hollow can't find its app support folder.".to_string())
}

/// Settings, Advanced, Hollow's databases (repo features R7b): every SQLite
/// file in the app support folder; with `check`, each one's quick_check and
/// table row counts. Read-only connections; changes nothing.
#[tauri::command]
pub async fn hollow_databases(
    check: Option<bool>,
    state: State<'_, AppState>,
) -> Result<crate::hollow_databases::Report, String> {
    let support = databases_support()?;
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::hollow_databases::report(&support, &vault, check.unwrap_or(false))
    })
    .await
    .map_err(|e| e.to_string())
}

/// Disk space → Remove old indexes…'s preview: other vaults' search and
/// meaning-search indexes unused for an hour. Read-only.
#[tauri::command]
pub async fn stale_indexes(
    state: State<'_, AppState>,
) -> Result<crate::hollow_databases::Plan, String> {
    let support = databases_support()?;
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::hollow_databases::stale_indexes(&support, &vault)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Remove the indexes `stale_indexes` listed, bound to `expected_hash` (the
/// list the person confirmed): a changed list refuses and removes nothing.
#[tauri::command]
pub async fn remove_stale_indexes(
    expected_hash: String,
    state: State<'_, AppState>,
) -> Result<crate::hollow_databases::Removed, String> {
    let support = databases_support()?;
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::hollow_databases::remove_stale_indexes(&support, &vault, &expected_hash)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Home's overnight card (overnight C8): last night's runs, failures, pull
/// requests, commits and what waits on the person. Read-only.
#[tauri::command]
pub async fn overnight_summary(
    state: State<'_, AppState>,
) -> Result<crate::overnight_summary::Summary, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::overnight_summary::summary(&vault, chrono::Local::now())
    })
    .await
    .map_err(|e| e.to_string())
}

fn path_str(path: &std::path::Path) -> String {
    path.to_string_lossy().to_string()
}
