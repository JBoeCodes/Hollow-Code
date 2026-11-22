//! Settings' Computer control sheet (Cua CU7): what it reads and its one
//! action. The two target switches ride `set_computer_control_enabled`
//! (the launch call), so this adds no launch IPC.

use crate::computer_broker::{self, EngineConfig};
use serde::Serialize;

/// What the sheet shows about Computer control, read fresh each time it opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerStatus {
    /// The Basic switch (Settings, Connectors, Computer control).
    pub enabled: bool,
    pub sandbox_enabled: bool,
    pub this_mac_enabled: bool,
    /// A valid engine pin exists (`computer-engine.json`).
    pub pinned: bool,
    /// `sandboxVm` or `thisMac`: where the pinned engine runs. `None` when
    /// nothing is pinned.
    pub target: Option<String>,
    pub version: Option<String>,
    /// The broker is listening (no socket: see-and-act tools never list).
    pub broker_listening: bool,
    /// Live desktop sessions right now.
    pub live_sessions: usize,
}

pub(crate) fn status_from(
    engine: Option<&EngineConfig>,
    enabled: bool,
    targets: (bool, bool),
    listening: bool,
    live: usize,
) -> ComputerStatus {
    ComputerStatus {
        enabled,
        sandbox_enabled: targets.0,
        this_mac_enabled: targets.1,
        pinned: engine.is_some(),
        target: engine.map(|e| {
            match e.target.kind() {
                computer_broker::TargetKind::SandboxVm => "sandboxVm",
                computer_broker::TargetKind::ThisMac => "thisMac",
            }
            .to_string()
        }),
        version: engine.map(|e| e.version.clone()),
        broker_listening: listening,
        live_sessions: live,
    }
}

#[tauri::command]
pub fn computer_status() -> ComputerStatus {
    status_from(
        EngineConfig::load().as_ref(),
        crate::computer_mcp::computer_control_enabled(),
        computer_broker::targets(),
        computer_broker::socket_path().is_some(),
        computer_broker::sessions().count(),
    )
}

/// The Doctor's checks (CU10), each with a state and a copyable fix. Reads
/// and installs nothing. It hashes the Driver and tries the sandbox host
/// (bounded to a second), so it runs off the main thread.
#[tauri::command]
pub async fn computer_doctor() -> Result<Vec<crate::computer_doctor::Check>, String> {
    tauri::async_runtime::spawn_blocking(doctor_checks)
        .await
        .map_err(|e| format!("the Doctor stopped: {e}"))
}

fn doctor_checks() -> Vec<crate::computer_doctor::Check> {
    let dir = crate::instance::support_dir();
    let free = dir.as_deref().and_then(crate::disk_guard::free_bytes);
    crate::computer_doctor::checks(&crate::computer_doctor::Inputs {
        enabled: crate::computer_mcp::computer_control_enabled(),
        listening: computer_broker::socket_path().is_some(),
        support_dir: dir.as_deref(),
        free_bytes: free,
        reach: &crate::computer_doctor::ssh_reachable,
        connector_findings: &crate::connectors::library()
            .map(|c| crate::connector_hygiene::library_findings(&c))
            .unwrap_or_default(),
    })
}

/// One default-denied app group on This Mac, for Settings' list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeniedApp {
    pub label: String,
    /// The person has allowed it (agents may use it on This Mac).
    pub allowed: bool,
}

fn denied_apps(policy: &crate::computer_thismac::Policy) -> Vec<DeniedApp> {
    policy
        .groups()
        .into_iter()
        .map(|(label, allowed)| DeniedApp {
            label: label.to_string(),
            allowed,
        })
        .collect()
}

/// The apps agents can't use on This Mac until the person allows them.
#[tauri::command]
pub fn computer_denied_apps() -> Vec<DeniedApp> {
    denied_apps(&crate::computer_thismac::Policy::load())
}

/// Allow or block one default-denied app on This Mac (the person's own
/// click in Settings; no agent tool reaches this). Takes effect on the next
/// call: the broker reads the file per request.
#[tauri::command]
pub fn computer_set_app_allowed(label: String, allowed: bool) -> Result<Vec<DeniedApp>, String> {
    let dir = crate::instance::support_dir().ok_or("Hollow's support folder isn't available")?;
    set_app_allowed_in(&dir, &label, allowed)
}

pub(crate) fn set_app_allowed_in(
    dir: &std::path::Path,
    label: &str,
    allowed: bool,
) -> Result<Vec<DeniedApp>, String> {
    let mut policy = crate::computer_thismac::Policy::load_from(dir);
    if !policy.set_group(label, allowed) {
        return Err(format!("{label} isn't on the list"));
    }
    policy.save_to(dir)?;
    Ok(denied_apps(&policy))
}

/// The hosts agents added to their own deny list (ND1), newest last, for
/// Settings, Computer control. An unreadable file is an error the sheet shows.
#[tauri::command]
pub fn host_scope_agent_denied() -> Result<Vec<crate::host_scope::AgentDenyEntry>, String> {
    let dir = crate::instance::support_dir().ok_or("Hollow's support folder isn't available")?;
    crate::host_scope::read_agent_deny(&dir)
}

/// Remove one host from the agent deny list: the person's own click in
/// Settings (no agent tool reaches this; an agent can only add).
#[tauri::command]
pub fn host_scope_remove_agent_deny(
    pattern: String,
) -> Result<Vec<crate::host_scope::AgentDenyEntry>, String> {
    let dir = crate::instance::support_dir().ok_or("Hollow's support folder isn't available")?;
    crate::host_scope::remove_agent_deny_in(&dir, &pattern)
}

/// One chat's desktop state for its card and chip (CU8): paused or taken
/// over, the newest actions and which frame is current.
#[tauri::command]
pub fn computer_chat_state(scope: String) -> crate::computer_activity::ChatView {
    crate::computer_activity::view(scope.trim())
}

/// A frame as a `data:` URL: the newest when `id` is absent. Read by the
/// chat's own card; frames are never stored on disk.
#[tauri::command]
pub fn computer_chat_frame(scope: String, id: Option<u64>) -> Option<String> {
    crate::computer_activity::frame(scope.trim(), id)
}

/// The person's controls on one chat: `pause`, `resume`, `takeover`,
/// `handback` or `stop`. Each leaves an audit row.
#[tauri::command]
pub fn computer_chat_control(
    scope: String,
    action: String,
) -> Result<crate::computer_activity::ChatView, String> {
    chat_control(scope.trim(), action.trim())
}

pub(crate) fn chat_control(
    scope: &str,
    action: &str,
) -> Result<crate::computer_activity::ChatView, String> {
    use crate::computer_activity as activity;
    let scope = scope.trim();
    if scope.is_empty() {
        return Err("no chat named".into());
    }
    match action {
        "pause" => activity::set_paused(scope, true, false),
        "resume" => activity::set_paused(scope, false, false),
        "takeover" => activity::set_paused(scope, true, true),
        "handback" => activity::set_paused(scope, false, false),
        "stop" => {
            computer_broker::sessions().end_scope(scope);
            activity::forget(scope);
        }
        other => return Err(format!("unknown control: {other}")),
    }
    computer_broker::audit_control(scope, action);
    Ok(activity::view(scope))
}

/// Stop all computer control: end every desktop session now and forget the
/// per-chat grants (the next call asks again). Works with no chat open. The
/// Sandbox VM's own stop (`lume stop`) and This Mac's daemon join this when
/// CU5 and CU6 land. Returns how many sessions ended.
#[tauri::command]
pub fn computer_stop_all() -> usize {
    computer_broker::end_all_sessions()
}

#[cfg(test)]
#[path = "computer_commands_tests.rs"]
mod tests;
