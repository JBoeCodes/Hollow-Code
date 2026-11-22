//! Settings, Agent tools, Task tools (Paperclip PR 12).
//!
//! Thin, like the other `commands/` adapters: the switch lives in
//! `agent-tools.json` outside the vault (`agent_settings`), loaded into the
//! process by `task_broker::load_settings` in setup before the run-queue
//! pump starts. The frontend reads the state from here rather than
//! localStorage, so the UI can never disagree with what runs attach.

/// Persist and apply the Task tools switch. New chat turns and automations
/// attach (or stop attaching) `hollow_tasks`; a run already holding the vault
/// keeps its server, and the broker refuses its writes while the switch is
/// off.
#[tauri::command]
pub fn set_tasks_mcp_enabled(enabled: bool) -> Result<(), String> {
    crate::task_broker::save_settings(enabled)
}

/// Whether Task tools are on (the persisted switch as loaded at startup, or
/// as last saved).
#[tauri::command]
pub fn tasks_mcp_enabled() -> bool {
    crate::tasks_mcp::enabled()
}
