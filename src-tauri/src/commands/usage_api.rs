//! Usage-ledger command adapter (Paperclip PR 6). Thin: the ledger and its
//! windows live in `usage_ledger`; rows live in `session_store`.

use super::AppState;
use crate::types::UsageSummary;
use crate::usage_ledger::{self, UsagePeriod};
use tauri::State;

/// Settings, Usage for `window` (`day` | `week` | `month` | `last30`,
/// local time), every agent or one (`agent`, Settings PR 8).
#[tauri::command]
pub async fn usage_summary(
    window: String,
    agent: Option<String>,
    state: State<'_, AppState>,
) -> Result<UsageSummary, String> {
    let period =
        UsagePeriod::parse(&window).ok_or_else(|| format!("unknown usage window: {window}"))?;
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        usage_ledger::summary_for(&vault, period, agent.as_deref(), chrono::Local::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// How long a finished chat of this folder's work kind usually takes
/// (`run_ledger::job_timing_for`). Nulls when the folder has no kind, or
/// fewer than three of that kind have finished. Not the hours that kind
/// is said to save.
#[tauri::command]
pub async fn job_timing(
    cwd: String,
    state: State<'_, AppState>,
) -> Result<crate::run_ledger::JobTiming, String> {
    let cwd = cwd.trim().to_string();
    if cwd.is_empty() || cwd.len() > 4_096 {
        return Ok(crate::run_ledger::JobTiming {
            work_kind: None,
            typical_ms: None,
        });
    }
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::run_ledger::job_timing_for(&vault, std::path::Path::new(&cwd))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Run ledger v2 (master plan F2): the ledger grouped and filtered by
/// account, workspace, department, work kind, outcome label, agent, model,
/// surface, outcome, day or month (`run_ledger::query`).
#[tauri::command]
pub async fn ledger_query(
    from: Option<String>,
    to: Option<String>,
    group_by: Option<Vec<String>>,
    filters: Option<std::collections::BTreeMap<String, String>>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let vault = state.config.vault_path.clone();
    let query = crate::run_ledger::Query {
        from,
        to,
        group_by: group_by.unwrap_or_default(),
        filters: filters.unwrap_or_default(),
    };
    crate::vault::spawn_blocking_logged(move || {
        crate::run_ledger::query(&vault, &query, chrono::Utc::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What one conversation's context is spent on (repo features R6,
/// `context_use::for_session`): tool results by size since its agent
/// session began, the cached share of its last turns and its compactions.
/// Sizes only, never result text, so an agent may read its own.
#[tauri::command]
pub async fn conversation_context_use(
    key: String,
    state: State<'_, AppState>,
) -> Result<crate::context_use::ContextUse, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::context_use::for_session(&vault, key.trim()))
        .await
        .map_err(|e| e.to_string())?
}

/// One run's cost with its descendants' (`descendants::rollup`), for the
/// automation run review (plan completion L2): `ownUsd` / `totalUsd` are
/// null when a cost is unknown (Codex, an unpriced model), never 0.
#[tauri::command]
pub async fn run_cost(
    run_id: String,
    state: State<'_, AppState>,
) -> Result<crate::run_manifest::CostRollup, String> {
    let run_id = run_id.trim().to_string();
    if run_id.is_empty() || run_id.len() > 128 {
        return Err("a run id is required".into());
    }
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::descendants::rollup(&vault, &run_id))
        .await
        .map_err(|e| e.to_string())
}

/// The price table as it applies (`usage_prices`), each row marked saved
/// (the person's own) or built in (overnight 9 A13, Settings, Usage,
/// Prices).
#[tauri::command]
pub fn price_table() -> Vec<crate::usage_prices::PriceRow> {
    crate::usage_prices::rows()
}

/// Save (or remove) the person's price for one model (`usage_prices::set`),
/// and return the table as it now applies.
#[tauri::command]
pub async fn set_price(
    price: crate::usage_prices::Price,
    remove: Option<bool>,
) -> Result<Vec<crate::usage_prices::PriceRow>, String> {
    crate::vault::spawn_blocking_logged(move || {
        crate::usage_prices::set(price, remove.unwrap_or(false))
            .map(|_| crate::usage_prices::rows())
    })
    .await
    .map_err(|e| e.to_string())?
}
