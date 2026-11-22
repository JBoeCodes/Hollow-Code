//! Usage-budget command adapters (Paperclip PR 8a). Thin: the store, the
//! evaluation, pauses and their review items live in `budgets`. Every write
//! emits `budget-changed` so Settings, Usage and the Automations pause panel
//! refresh.

use super::AppState;
use crate::budgets::{self, BudgetDef, BudgetInput, BudgetsView, Store};
use tauri::State;

/// Settings, Usage: every budget with its live state, pauses, holds and
/// whether suggestions are available. Never fails on a corrupt store: the
/// view carries `storeError` instead.
#[tauri::command]
pub async fn get_budgets(state: State<'_, AppState>) -> Result<BudgetsView, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        budgets::view(
            &Store::for_app(&vault),
            &vault,
            &budgets::production_scope_agent(&vault),
            chrono::Utc::now(),
        )
    })
    .await
    .map_err(|e| e.to_string())
}

/// Create (no `id`) or update a budget. Refuses budgets that could never
/// trip and duplicates of (scope, metric, window).
#[tauri::command]
pub async fn save_budget(
    input: BudgetInput,
    state: State<'_, AppState>,
) -> Result<BudgetDef, String> {
    let vault = state.config.vault_path.clone();
    let saved = crate::vault::spawn_blocking_logged(move || {
        budgets::save(
            &Store::for_app(&vault),
            &vault,
            input,
            &budgets::production_scope_agent(&vault),
            chrono::Utc::now(),
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    budgets::emit_changed();
    saved
}

/// Remove a budget, its pause and the pause's review item.
#[tauri::command]
pub async fn delete_budget(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    let deleted = crate::vault::spawn_blocking_logged(move || {
        budgets::delete(&Store::for_app(&vault), &vault, &id)
    })
    .await
    .map_err(|e| e.to_string())?;
    budgets::emit_changed();
    deleted
}

/// Settings, Usage "Resume…": raise a paused budget's limit above this
/// window's usage and clear the pause (accepting its review item when it is
/// still pending). Never starts a run: the pump claims the next one.
#[tauri::command]
pub async fn resume_budget(
    id: String,
    new_limit: f64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    let resumed = crate::vault::spawn_blocking_logged(move || {
        budgets::resume(
            &Store::for_app(&vault),
            &vault,
            &id,
            new_limit,
            chrono::Utc::now(),
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    budgets::emit_changed();
    resumed
}
