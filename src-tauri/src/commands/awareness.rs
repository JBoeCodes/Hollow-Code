//! Read-only awareness/diagnostic IPC commands.

use super::AppState;
use tauri::State;

/// Read-only backup posture. `managed_elsewhere` is the machine-local user
/// acknowledgement from Settings; Hollow never stores it in the vault.
#[tauri::command]
pub fn backup_health(
    state: State<'_, AppState>,
    managed_elsewhere: Option<bool>,
) -> crate::backup_health::BackupHealth {
    crate::backup_health::inspect(&state.config.vault_path, managed_elsewhere.unwrap_or(false))
}
