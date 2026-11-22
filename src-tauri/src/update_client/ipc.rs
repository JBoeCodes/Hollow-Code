//! The updater's Tauri commands (the top bar's Update button and Settings,
//! Updates, row U5 PR 2). Each runs on its own thread: checks, codesign and
//! the plugin's blocking calls never hold the event loop. The token goes in
//! once (`update_connect`) and never comes back out.
//!
//! None is a `hollow_app` action (`docs/HOLLOW_APP_COMMAND_MAP.tsv`: the
//! reads are plumbing, the rest the person's own clicks): no agent installs
//! a channel update (design section 7). An agent proposes a Local build only
//! through `build.install_built_app`, which always asks (row U10).

use super::client::Client;
use super::install::UpdateInstallOutcome;
use super::last::UpdateLastResult;
use super::settings::UpdateSettings;
use super::state::{Channel, UpdateState};
use std::sync::Arc;

const NOT_RUNNING: &str = "Hollow's updater isn't running in this window.";

/// Run `f` with the client on a plain thread and wait for it.
pub(super) async fn on_client<T: Send + 'static>(
    f: impl FnOnce(&Arc<Client>) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let client = super::client().ok_or(NOT_RUNNING)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("hollow-update-call".into())
        .spawn(move || {
            let _ = tx.send(f(&client));
        })
        .map_err(|e| format!("Could not ask the updater: {e}"))?;
    rx.await.map_err(|_| NOT_RUNNING.to_string())?
}

/// The client's state (the `update-state` event carries the same).
#[tauri::command]
pub async fn update_get_state() -> Result<UpdateState, String> {
    on_client(|c| Ok(c.state())).await
}

#[tauri::command]
pub async fn update_get_settings() -> Result<UpdateSettings, String> {
    on_client(|c| Ok(c.settings())).await
}

/// Switch channel, then check it.
#[tauri::command]
pub async fn update_set_channel(channel: Channel) -> Result<UpdateState, String> {
    on_client(move |c| {
        let before = c.settings().channel;
        let state = c.set_channel(channel)?;
        if before != channel {
            super::kick(c);
        }
        Ok(state)
    })
    .await
}

/// "Install updates overnight" (row U7, `overnight.rs`): takes effect at once.
#[tauri::command]
pub async fn update_set_overnight(enabled: bool) -> Result<UpdateSettings, String> {
    on_client(move |c| c.set_overnight(enabled)).await
}

/// Check now; the result follows as `update-state` events.
#[tauri::command]
pub async fn update_check_now() -> Result<UpdateState, String> {
    on_client(|c| {
        super::kick(c);
        Ok(c.state())
    })
    .await
}

/// Store the person's token, then check. Answers only whether a token is
/// stored.
#[tauri::command]
pub async fn update_connect(token: String) -> Result<UpdateConnected, String> {
    on_client(move |c| {
        let connected = c.connect(&token)?;
        super::kick(c);
        Ok(UpdateConnected { connected })
    })
    .await
}

/// "Use the new token": store the token a Connect Hollow link brought while
/// another was stored (`pendingConnect`), then check.
#[tauri::command]
pub async fn update_confirm_connect() -> Result<UpdateState, String> {
    on_client(|c| {
        let state = c.confirm_connect()?;
        super::kick(c);
        Ok(state)
    })
    .await
}

/// "Keep the current token": forget the link's token.
#[tauri::command]
pub async fn update_discard_connect() -> Result<UpdateState, String> {
    on_client(|c| Ok(c.discard_connect())).await
}

/// Remove the token from the Keychain.
#[tauri::command]
pub async fn update_disconnect() -> Result<UpdateConnected, String> {
    on_client(|c| {
        Ok(UpdateConnected {
            connected: c.disconnect()?,
        })
    })
    .await
}

/// "Not now": hide this version's button until a newer one is ready.
#[tauri::command]
pub async fn update_dismiss(version: String) -> Result<UpdateState, String> {
    on_client(move |c| c.dismiss(&version)).await
}

/// "Restart to update". `confirmed_keys`: the running work the restart
/// dialog listed (`exit_running_work`), as `setup_restart` takes them.
#[tauri::command]
pub async fn update_install_now(
    confirmed_keys: Option<Vec<String>>,
) -> Result<UpdateInstallOutcome, String> {
    on_client(move |c| c.install_now(confirmed_keys.as_deref())).await
}

/// "Update when chats finish": waits with no expiry, in memory only.
#[tauri::command]
pub async fn update_install_when_idle() -> Result<UpdateState, String> {
    on_client(|c| c.install_when_idle(super::install::POLL)).await
}

#[tauri::command]
pub async fn update_cancel_wait() -> Result<UpdateState, String> {
    on_client(|c| Ok(c.cancel_wait())).await
}

/// The notice after an update, until acknowledged.
#[tauri::command]
pub async fn update_last_result() -> Result<Option<UpdateLastResult>, String> {
    on_client(|c| Ok(c.last_result())).await
}

#[tauri::command]
pub async fn update_ack_last_result() -> Result<(), String> {
    on_client(|c| c.ack_last_result()).await
}

/// "Show in Update button" in Changes, Hollow app (row U10): offer the
/// build workspace's finished build as "Local build of <branch>" again
/// (after Not now, or when it wasn't offered). The person's click; the
/// install itself is still the Update button's Restart to update.
#[tauri::command]
pub async fn update_offer_local_build(
    id: String,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<UpdateState, String> {
    let vault = state.config.vault_path.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("hollow-update-call".into())
        .spawn(move || {
            let _ = tx.send(super::offer_local_build(&vault, &id, true));
        })
        .map_err(|e| format!("Could not ask the updater: {e}"))?;
    rx.await.map_err(|_| NOT_RUNNING.to_string())?
}

/// `update_connect` and `update_disconnect`: whether a token is stored now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct UpdateConnected {
    pub connected: bool,
}
