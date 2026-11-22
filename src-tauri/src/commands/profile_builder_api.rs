//! About you builder command adapter. The work lives in `profile_builder`;
//! every command returns the refreshed view. A finished scan also emits
//! `profile-builder-changed` so an open Settings page refreshes.

use super::AppState;
use crate::profile_builder::{self, ProfileBuilderView};
use tauri::{AppHandle, Emitter, State};

pub const EVENT: &str = "profile-builder-changed";

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    crate::vault::spawn_blocking_logged(f)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn profile_builder_view(
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    view_in(state.config.vault_path.clone()).await
}

/// [`profile_builder_view`] for the agent action: a damaged store is an
/// error but stays for About you to move aside and report (B·F08).
pub(crate) async fn profile_builder_view_unattended(
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || profile_builder::view_unattended(&vault)).await
}

#[tauri::command]
pub async fn profile_builder_scan(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    if !state.configured {
        return Err("Set up your Hollow folder first.".into());
    }
    let vault = state.config.vault_path.clone();
    let _ = app.emit(EVENT, ());
    let result = profile_builder::scan(&vault).await;
    let _ = app.emit(EVENT, ());
    result
}

#[tauri::command]
pub async fn profile_builder_accept(
    id: String,
    text: Option<String>,
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || profile_builder::accept(&vault, &id, text.as_deref())).await
}

#[tauri::command]
pub async fn profile_builder_accept_all(
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || profile_builder::accept_all(&vault)).await
}

#[tauri::command]
pub async fn profile_builder_dismiss(
    id: String,
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || profile_builder::dismiss(&vault, &id)).await
}

#[tauri::command]
pub async fn profile_builder_answer(
    id: String,
    answer: String,
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || profile_builder::answer(&vault, &id, &answer)).await
}

#[tauri::command]
pub async fn profile_builder_skip_question(
    id: String,
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || profile_builder::skip_question(&vault, &id)).await
}

#[tauri::command]
pub async fn profile_builder_set_auto_learn(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<ProfileBuilderView, String> {
    set_auto_learn_in(state.config.vault_path.clone(), enabled).await
}

/// [`profile_builder_set_auto_learn`]'s body, also the `profile.autoLearn`
/// setting's setter (`settings.change`, a settings import), so neither
/// calls the command itself (COO parity L13 review).
pub(crate) async fn set_auto_learn_in(
    vault: std::path::PathBuf,
    enabled: bool,
) -> Result<ProfileBuilderView, String> {
    blocking(move || profile_builder::set_auto_learn(&vault, enabled)).await
}

/// [`profile_builder_view`]'s body, also the `profile.autoLearn` setting's
/// getter.
pub(crate) async fn view_in(vault: std::path::PathBuf) -> Result<ProfileBuilderView, String> {
    blocking(move || profile_builder::view(&vault)).await
}

/// "Keep learning": check every hour whether a background scan is due
/// (`profile_builder::background_due`: learning on, 12 h since the last scan,
/// sources changed). The first check waits 3 minutes so launch stays quiet.
pub(crate) fn start_background(app: impl Into<crate::core_host::Core>, vault: std::path::PathBuf) {
    let app = app.into();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(180)).await;
        loop {
            let v = vault.clone();
            let due = crate::vault::spawn_blocking_logged(move || {
                profile_builder::background_due(&v, chrono::Utc::now())
            })
            .await
            .unwrap_or(false);
            if due {
                let _ = app.emit(EVENT, ());
                if let Err(e) =
                    profile_builder::scan_as(&vault, crate::self_hosted_jobs::Asked::Auto).await
                {
                    eprintln!("[profile-builder] background scan: {e}");
                }
                let _ = app.emit(EVENT, ());
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}
