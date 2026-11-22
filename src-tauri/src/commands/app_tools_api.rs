//! Settings, Connectors, Built into Hollow, "Hollow app" (PR A1).
//!
//! Thin, like `tasks_tools_api`: the switch lives in `agent-tools.json`
//! (`agent_settings`), loaded by `app_broker::load_settings` in setup before
//! the run-queue pump starts, so the UI reads it from here and never from
//! localStorage.

/// Persist and apply the Hollow app tools switch. New chat turns and
/// automations attach (or stop attaching) `hollow_app`; a run that already
/// has the server keeps it, and the broker refuses its calls while the
/// switch is off.
#[tauri::command]
pub fn set_app_mcp_enabled(enabled: bool) -> Result<(), String> {
    crate::app_broker::save_settings(enabled)
}

/// Whether the Hollow app tools are on.
#[tauri::command]
pub fn app_mcp_enabled() -> bool {
    crate::app_mcp::enabled()
}
