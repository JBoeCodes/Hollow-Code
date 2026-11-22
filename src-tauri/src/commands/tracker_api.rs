//! The task tracker's Tauri commands (overhaul A2; shapes in
//! `tracker::ipc`, contract `docs/TRACKER_SYNC.md`).
//!
//! Reads use config, the saved key's state and the sync engine's local
//! mirror. User-only issue edits, creates, archives and Sync now are wired;
//! project creation shares its durable journal with the agent action.
//! Agents get their own tracker actions with tiers and
//! cards when the seams are repointed (T4). Settings' key and settings
//! commands are real: they save the key in Hollow's Keychain bundle and the
//! `tracker` block in config.json (applied after a restart). Nothing here
//! returns or logs the key.

use crate::commands::AppState;
use crate::config::Config;
use crate::tracker::ipc::{
    TrackerEdit, TrackerIssueDetail, TrackerNewProject, TrackerNewTask, TrackerProviderOption,
    TrackerRunState, TrackerStatus, TrackerSyncOutcome, TrackerTeamMeta, TrackerView,
    TrackerViewQuery, TrackerWriteOutcome,
};
use crate::tracker::settings::{self, KeyState, TrackerConfig, TrackerSettingsUpdate};
use tauri::State;

/// Whether this build has the sync engine (T3 turns it on).
pub(crate) const ENGINE: bool = true;

/// The tracker block saved in config.json now (what Settings shows), when it
/// can be read; the running config otherwise.
fn saved_tracker(state: &AppState) -> Option<TrackerConfig> {
    let path = if state.config_path.as_os_str().is_empty() {
        crate::setup::config_write_path().ok()?
    } else {
        state.config_path.clone()
    };
    let map = crate::setup::load_config_map(&path).ok()?;
    Some(TrackerConfig::from_value(
        map.get("tracker").unwrap_or(&serde_json::Value::Null),
    ))
}

/// The provider whose key Settings manages: the saved choice, else the running one.
fn key_provider(state: &AppState) -> Result<String, String> {
    saved_tracker(state)
        .and_then(|saved| saved.provider)
        .or_else(|| state.config.tracker.provider.clone())
        .ok_or_else(|| "Choose a task tracker first.".to_string())
}

/// The status for `config` (running), the saved block and the key's state.
pub(crate) fn status_for(
    config: &Config,
    saved: Option<&TrackerConfig>,
    key: KeyState,
) -> TrackerStatus {
    let tracker = &config.tracker;
    let info = tracker.provider_info();
    let missing = tracker.missing(key);
    let state = if info.is_none() {
        TrackerRunState::Off
    } else if missing.is_some() {
        TrackerRunState::NeedsSetup
    } else {
        TrackerRunState::Ready
    };
    TrackerStatus {
        state,
        provider: tracker.provider.clone(),
        display_name: info.map(|p| p.display_name.to_string()),
        providers: settings::PROVIDERS
            .iter()
            .map(|p| TrackerProviderOption {
                id: p.id.to_string(),
                display_name: p.display_name.to_string(),
                keychain_item: p.keychain_item.is_some(),
                transports: p.transports.iter().map(|t| t.to_string()).collect(),
            })
            .collect(),
        team_id: tracker.team_id.clone(),
        team_key: tracker.team_key.clone(),
        personal_team_id: tracker.personal_team_id.clone(),
        personal_team_key: tracker.personal_team_key.clone(),
        transport: tracker.transport.clone(),
        poll_seconds: tracker.poll_seconds,
        track_agent_work: tracker.track_agent_work,
        friday_status: saved.map_or(tracker.friday_status, |s| s.friday_status),
        post_status_updates: saved.map_or(tracker.post_status_updates, |s| s.post_status_updates),
        key,
        missing,
        blocked_by: None,
        last_sync: None,
        next_sync: None,
        last_error: None,
        rate: None,
        outbox: 0,
        problems: tracker.problems.clone(),
        restart_needed: saved.is_some_and(|saved| saved != tracker),
        engine: ENGINE,
    }
}

fn current_status(state: &AppState) -> TrackerStatus {
    let saved = saved_tracker(state);
    // Touch the Keychain only once a provider is chosen.
    let key = match saved
        .as_ref()
        .and_then(|s| s.provider.clone())
        .or_else(|| state.config.tracker.provider.clone())
    {
        Some(provider) => settings::key_state(&provider),
        None => KeyState::Missing,
    };
    let mut status = status_for(&state.config, saved.as_ref(), key);
    crate::tracker_sync::apply_status(&state.config, &mut status);
    status
}

#[tauri::command(async)]
pub fn tracker_status(state: State<'_, AppState>) -> TrackerStatus {
    current_status(&state)
}

#[tauri::command(async)]
pub fn tracker_view(
    state: State<'_, AppState>,
    query: Option<TrackerViewQuery>,
) -> Result<TrackerView, String> {
    let mut view = crate::tracker_sync::view(&state.config, &query.unwrap_or_default())?;
    // H4b: each row's "Tracked in Linear" (or "Linear unreachable since ...").
    crate::commands::sources_api::label_rows_for(&state, view.provider.as_deref(), &mut view.rows);
    Ok(view)
}

#[tauri::command(async)]
pub fn tracker_issue(state: State<'_, AppState>, id: String) -> Result<TrackerIssueDetail, String> {
    crate::tracker_sync::issue(&state.config, &id)
}

/// The issue detail's activity: one mirrored issue's change history, read
/// from the tracker on demand (state, assignee, priority, labels, project,
/// title, description), oldest first, with `more` when older entries were
/// left out. Read-only and never stored.
#[tauri::command(async)]
pub async fn tracker_issue_history(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::tracker::TrackerHistory, String> {
    crate::tracker_sync::history(&state.config, &id).await
}

/// X1: the agent runs tied to an issue (id or key) and its Hollow audit
/// rows (X2, newest first). Local reads only.
#[tauri::command(async)]
pub fn tracker_issue_work(
    state: State<'_, AppState>,
    issue_id: String,
) -> crate::tracker::work::TrackerIssueWork {
    crate::tracker::work::issue_work(&state.config.vault_path, &issue_id, chrono::Utc::now())
}

/// The Custom link value to paste into Linear's coding-tool setting
/// (`hollow://issue?prompt={{prompt}}`), and where it goes. A constant:
/// reads nothing, writes nothing.
#[tauri::command]
pub fn tracker_link_setup() -> crate::linear_link::LinkSetup {
    crate::linear_link::setup()
}

/// Webview drain for `hollow://` links the OS opened (Linear's Custom
/// link): each queued link's draft and where it opens, oldest first, then
/// the queue is empty. A draft only; the window never sends it.
#[tauri::command]
pub fn take_linear_links() -> Vec<crate::linear_link::LinkOpen> {
    crate::linear_link::take_pending()
}

#[tauri::command(async)]
pub fn tracker_team_meta(state: State<'_, AppState>) -> Result<TrackerTeamMeta, String> {
    crate::tracker_sync::team_meta(&state.config)
}

#[tauri::command(async)]
pub async fn tracker_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<TrackerSyncOutcome, String> {
    use tauri::Emitter;
    let result = crate::tracker_sync::sync(&state.config).await;
    let changed = result.as_ref().map(|r| r.pulled + r.pushed).unwrap_or(0);
    let _ = app.emit(
        crate::tracker_sync::SYNC_EVENT,
        serde_json::json!({"ok":result.as_ref().is_ok_and(|r| r.errors.is_empty()),"changed":changed}),
    );
    result
}

#[tauri::command(async)]
pub async fn edit_tracker_item(
    state: State<'_, AppState>,
    edit: TrackerEdit,
) -> Result<TrackerWriteOutcome, String> {
    crate::tracker_sync::edit(&state.config, edit).await
}

#[tauri::command(async)]
pub async fn create_tracker_task(
    state: State<'_, AppState>,
    task: TrackerNewTask,
) -> Result<TrackerWriteOutcome, String> {
    if task.title.trim().is_empty() {
        return Err("A task needs a title.".into());
    }
    crate::tracker_sync::create(&state.config, task).await
}

#[tauri::command(async)]
pub async fn create_tracker_project(
    state: State<'_, AppState>,
    project: TrackerNewProject,
) -> Result<TrackerWriteOutcome, String> {
    if project.name.trim().is_empty() {
        return Err("A project needs a name.".into());
    }
    crate::tracker_sync::create_project(&state.config, project, None).await
}

/// The issue detail's comment box (overhaul B5): one comment on a mirrored
/// issue of the scoped team, journaled under `requestKey` (a retry with the
/// same key never posts twice; an unconfirmed one is never resent).
#[tauri::command(async)]
pub async fn comment_tracker_issue(
    state: State<'_, AppState>,
    id: String,
    body: String,
    request_key: String,
) -> Result<TrackerWriteOutcome, String> {
    crate::tracker_sync::comment(&state.config, &id, &body, &request_key).await
}

/// The project detail's "Write status now" (A18): the Friday status of one
/// tracked workspace, or of every one (with the COO's digest) when
/// `workspaceId` is empty. Notes first, then the notification, then the
/// tracker copies when Settings' "Also post it to the tracker" is on.
#[tauri::command(async)]
pub async fn write_friday_status(
    state: State<'_, AppState>,
    workspace_id: Option<String>,
) -> Result<crate::tracker::ipc::TrackerFridayStatus, String> {
    crate::tracker::status::run(
        &state.config,
        workspace_id.as_deref(),
        chrono::Local::now(),
        None,
    )
    .await
}

/// A saved canvas's "Attach to issue..." (A19): a copy of the canvas (its
/// current revision, or `revision`) on an issue of team ENG. The canvas
/// stays in Hollow.
#[tauri::command(async)]
pub async fn attach_canvas_to_issue(
    state: State<'_, AppState>,
    workspace_id: String,
    canvas_id: String,
    revision: Option<u64>,
    issue_id: String,
) -> Result<TrackerWriteOutcome, String> {
    crate::tracker_sync::attach_canvas(
        &state.config,
        &workspace_id,
        &canvas_id,
        revision,
        &issue_id,
    )
    .await
}

#[tauri::command(async)]
pub async fn archive_tracker_task(
    state: State<'_, AppState>,
    id: String,
) -> Result<TrackerWriteOutcome, String> {
    crate::tracker_sync::archive(&state.config, &id).await
}

/// Settings, Task tracker, API key: save a pasted key in Hollow's bundle.
#[tauri::command(async)]
pub fn set_tracker_key(state: State<'_, AppState>, key: String) -> Result<TrackerStatus, String> {
    save_tracker_key(&state, &key)
}

/// [`set_tracker_key`]'s body, also the COO's `tracker.set_key` (L13): the
/// key goes into Hollow's Keychain bundle and only the status comes back
/// (its `key` says saved or missing, never the value).
pub(crate) fn save_tracker_key(state: &AppState, key: &str) -> Result<TrackerStatus, String> {
    let provider = key_provider(state)?;
    settings::save_key(&provider, key)?;
    Ok(current_status(state))
}

#[tauri::command(async)]
pub fn clear_tracker_key(state: State<'_, AppState>) -> Result<TrackerStatus, String> {
    remove_tracker_key(&state)
}

/// [`clear_tracker_key`]'s body, also the COO's `tracker.clear_key` (L13).
pub(crate) fn remove_tracker_key(state: &AppState) -> Result<TrackerStatus, String> {
    let provider = key_provider(state)?;
    settings::clear_key(&provider)?;
    Ok(current_status(state))
}

/// "Use the key saved in Keychain": copy the provider's own Keychain item
/// into Hollow's bundle (one macOS prompt the first time).
#[tauri::command(async)]
pub fn use_keychain_tracker_key(state: State<'_, AppState>) -> Result<TrackerStatus, String> {
    copy_keychain_tracker_key(&state)
}

/// [`use_keychain_tracker_key`]'s body, also the COO's
/// `tracker.use_keychain_key` (L13): the item's value goes straight into the
/// bundle and only the status comes back.
pub(crate) fn copy_keychain_tracker_key(state: &AppState) -> Result<TrackerStatus, String> {
    let provider = key_provider(state)?;
    let Some(info) = settings::provider(&provider) else {
        return Err("Choose a task tracker first.".into());
    };
    let Some((service, _)) = info.keychain_item else {
        return Err(format!(
            "{} has no Keychain item to copy; paste the key instead.",
            info.display_name
        ));
    };
    if !settings::copy_keychain_item(&provider)? {
        return Err(format!(
            "No Keychain item named \"{service}\" was found. Create it, or paste the key instead."
        ));
    }
    Ok(current_status(state))
}

/// Settings, Task tracker: provider, team, transport and interval. Saved to
/// config.json; applied after a restart (`restartNeeded` says so).
#[tauri::command(async)]
pub fn set_tracker_settings(
    state: State<'_, AppState>,
    update: TrackerSettingsUpdate,
) -> Result<TrackerStatus, String> {
    save_tracker_settings(&state, &update)
}

/// [`set_tracker_settings`]'s body, also the COO's `tracker.set_settings`
/// (L13).
pub(crate) fn save_tracker_settings(
    state: &AppState,
    update: &TrackerSettingsUpdate,
) -> Result<TrackerStatus, String> {
    let path = if state.config_path.as_os_str().is_empty() {
        crate::setup::config_write_path()?
    } else {
        state.config_path.clone()
    };
    let mut map = crate::setup::load_config_map(&path)?;
    let (before, _) = crate::sources::SourcesConfig::from_map(&map);
    settings::apply_update(&mut map, update)?;
    // H4b: while tasks follow the tracker block (no `sources` choice yet),
    // turning the tracker off would move them to the vault with no dry run.
    // That is a switch (Settings, Sources), never a Save.
    let (after, _) = crate::sources::SourcesConfig::from_map(&map);
    if !before.tasks_set
        && before.tasks == crate::sources::OwnerKind::Tracker
        && after.tasks != crate::sources::OwnerKind::Tracker
    {
        return Err(TRACKER_OFF_IS_A_SWITCH.into());
    }
    crate::setup::publish_config(&path, &map)?;
    Ok(current_status(state))
}

/// Why the tracker can't be turned off from its own row while tasks live in it.
pub(crate) const TRACKER_OFF_IS_A_SWITCH: &str = "Tasks and projects live in the tracker, so turning it off would move them: switch to Vault only in Settings, Sources first (a dry run shows what changes). Nothing was changed.";

/// Settings key `tracker.trackAgentWork` (T3a): the saved mode (the
/// running one when nothing is saved).
pub(crate) fn agent_work(state: &AppState) -> settings::AgentWork {
    saved_tracker(state)
        .map(|saved| saved.agent_work)
        .unwrap_or(state.config.tracker.agent_work)
}

/// Save `trackAgentWork` (`off`, `dryRun` or `on`) to config.json; applies
/// after a restart, like every tracker setting. Returns the saved mode.
pub(crate) fn save_agent_work(state: &AppState, value: &str) -> Result<&'static str, String> {
    let mode = settings::AgentWork::from_value(&serde_json::json!(value))
        .ok_or("Track agent work is off, dryRun or on.")?;
    let path = if state.config_path.as_os_str().is_empty() {
        crate::setup::config_write_path()?
    } else {
        state.config_path.clone()
    };
    let mut map = crate::setup::load_config_map(&path)?;
    settings::set_agent_work(&mut map, mode);
    crate::setup::publish_config(&path, &map)?;
    Ok(mode.as_str())
}

/// `tracker.configure` (T6b): the tracker block through
/// [`save_tracker_settings`] (Settings' own path), then, when asked, the
/// provider's Keychain item copied as "Use the key saved in Keychain" does.
/// Refuses any team but Engineering before writing. Returns the status,
/// which never carries the key.
pub(crate) fn configure_tracker(
    state: &AppState,
    request: &settings::TrackerConfigure,
) -> Result<TrackerStatus, String> {
    let update = request.update()?;
    save_tracker_settings(state, &update)?;
    if request.copy_keychain_key {
        copy_keychain_tracker_key(state)?;
    }
    Ok(current_status(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `two_way`: an old config's `notion.twoWaySync`, which no longer
    /// blocks anything (Notion two-way sync was removed on 2026-11-10).
    fn config(tracker: serde_json::Value, two_way: bool) -> Config {
        serde_json::from_value(json!({
            "vaultPath": "/vault",
            "notion": {"twoWaySync": two_way},
            "tracker": tracker
        }))
        .unwrap()
    }

    fn full() -> serde_json::Value {
        json!({"provider": "linear", "linear": {"teamId": "11111111-1111-4111-8111-111111111111", "teamKey": "ENG"}})
    }

    #[test]
    fn status_walks_off_needs_setup_blocked_and_ready() {
        let off = status_for(
            &config(serde_json::Value::Null, false),
            None,
            KeyState::Missing,
        );
        assert_eq!(off.state, TrackerRunState::Off);
        assert_eq!(off.provider, None);
        assert_eq!(off.blocked_by, None);
        assert_eq!(off.providers.len(), settings::PROVIDERS.len());
        assert!(off.providers[0].keychain_item);

        let no_key = status_for(&config(full(), false), None, KeyState::Missing);
        assert_eq!(no_key.state, TrackerRunState::NeedsSetup);
        assert_eq!(no_key.missing.as_deref(), Some("Save an API key."));
        assert_eq!(no_key.display_name.as_deref(), Some("Linear"));

        // An old `notion.twoWaySync: true` is ignored.
        let legacy = status_for(&config(full(), true), None, KeyState::Saved);
        assert_eq!(legacy.state, TrackerRunState::Ready);
        assert_eq!(legacy.blocked_by, None);

        let ready = status_for(&config(full(), false), None, KeyState::Saved);
        assert_eq!(ready.state, TrackerRunState::Ready);
        assert_eq!(ready.team_key.as_deref(), Some("ENG"));
        assert_eq!(ready.poll_seconds, settings::DEFAULT_POLL_SECONDS);
        assert!(ready.engine);
        assert!(!ready.restart_needed);
        assert!(!ready.track_agent_work, "off until the cutover");
    }

    #[test]
    fn a_saved_change_says_a_restart_is_needed() {
        let running = config(full(), false);
        let mut saved = running.tracker.clone();
        assert!(!status_for(&running, Some(&saved), KeyState::Saved).restart_needed);
        saved.poll_seconds = 600;
        assert!(status_for(&running, Some(&saved), KeyState::Saved).restart_needed);
    }

    #[test]
    fn the_status_never_carries_a_key() {
        let status = status_for(&config(full(), false), None, KeyState::Saved);
        let wire = serde_json::to_string(&status).unwrap();
        assert!(wire.contains("\"key\":\"saved\""), "{wire}");
        assert!(!wire.contains("lin_api"), "{wire}");
    }
}
