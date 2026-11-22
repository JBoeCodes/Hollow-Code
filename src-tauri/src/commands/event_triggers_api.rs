//! Event triggers' Tauri commands (COO rebuild F2; the engine and its rules
//! are `event_triggers`). The automation editor's When: Event, the project
//! menu's "Wake the lead for new issues" and the agent actions
//! `automations.event_triggers` and `automations.set_event_trigger` call
//! these. The state lives in app support, outside the vault.

use crate::commands::AppState;
use crate::event_triggers::{self, EventTriggerStatus, SetTrigger};
use tauri::State;

/// Each kind (on or off, since when, the hourly cap and this hour's fires),
/// the projects turned on, the leads that wake for new issues, the tracked
/// and connected projects, and the newest decided events.
#[tauri::command]
pub fn event_triggers_status(state: State<'_, AppState>) -> EventTriggerStatus {
    event_triggers::status(&state.config, chrono::Utc::now())
}

/// Turn a kind, a project's trigger or a lead wake on or off, or set a
/// kind's hourly cap. Turning one on never fires events from before.
#[tauri::command]
pub fn set_event_trigger(
    state: State<'_, AppState>,
    input: SetTrigger,
) -> Result<EventTriggerStatus, String> {
    event_triggers::set(&state.config, &input, chrono::Utc::now())
}
