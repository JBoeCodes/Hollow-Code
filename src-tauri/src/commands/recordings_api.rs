//! Recordings (next-level P11): on-demand reads through Hollow's Plaud sign-in,
//! explicit links, opt-in transcript retention.
use super::AppState;
use crate::recordings::{self, RecordingDetail, RecordingLink, RecordingList};
use crate::state_file::OnDamage;
use tauri::State;

#[tauri::command]
pub async fn list_recordings(
    days: Option<u32>,
    fresh: Option<bool>,
    state: State<'_, AppState>,
) -> Result<RecordingList, String> {
    recordings::list(
        &state.config.vault_path,
        days.unwrap_or(14),
        OnDamage::SetAside,
        fresh.unwrap_or(false),
    )
    .await
}

/// [`list_recordings`] for the agent action: a damaged links file is an
/// error but stays for the Recordings view to move aside and report (B·F08).
pub(crate) async fn list_recordings_unattended(
    days: Option<u32>,
    state: State<'_, AppState>,
) -> Result<RecordingList, String> {
    recordings::list(
        &state.config.vault_path,
        days.unwrap_or(14),
        OnDamage::Leave,
        false,
    )
    .await
}

#[tauri::command]
pub async fn read_recording(
    id: String,
    state: State<'_, AppState>,
) -> Result<RecordingDetail, String> {
    recordings::read(&state.config.vault_path, &id, OnDamage::SetAside).await
}

/// [`read_recording`] for the agent action (see [`list_recordings_unattended`]).
pub(crate) async fn read_recording_unattended(
    id: String,
    state: State<'_, AppState>,
) -> Result<RecordingDetail, String> {
    recordings::read(&state.config.vault_path, &id, OnDamage::Leave).await
}

/// The audio URL, on request (Plaud's URL expires after about a day).
#[tauri::command]
pub async fn recording_audio_url(id: String) -> Result<String, String> {
    recordings::audio_url(&id).await
}

#[tauri::command]
pub async fn set_recording_link(
    id: String,
    workspace_id: Option<String>,
    event_id: Option<String>,
    event_title: Option<String>,
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<RecordingLink>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        recordings::set_link(&root, &id, workspace_id, event_id, event_title, name)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opt-in: keep this recording's transcript in one workspace's sources/.
/// The person's click, or the COO's `recordings.keep_transcript` (L13).
#[tauri::command]
pub async fn keep_recording_transcript(
    app: tauri::AppHandle,
    id: String,
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    keep_transcript_and_extract(app, vault, &id, &workspace_id, None).await
}

/// Keep the transcript, then extract it in the background. Needs no webview:
/// the command and the COO's action both run it. `run_id` is the COO's run
/// (L13 review): the extraction's spend is metered to it and it is an
/// agent's ask, not the person's; `None` for the person's click.
pub(crate) async fn keep_transcript_and_extract(
    app: tauri::AppHandle,
    vault: std::path::PathBuf,
    id: &str,
    workspace_id: &str,
    run_id: Option<String>,
) -> Result<String, String> {
    let path = recordings::keep_transcript(&vault, id, workspace_id).await?;
    // Master plan F3: a kept transcript is extracted in the background, onto
    // the review queue only (recordings first; chats wait). A failure never
    // fails the keep.
    let source = crate::extract::Source::Recording { path: path.clone() };
    let kept = path.clone();
    tauri::async_runtime::spawn(async move {
        let me = crate::extract::me(&vault);
        // Self-hosted models Q10: it can wait for a self-hosted model that
        // is asleep (Settings, Advanced, Background jobs).
        let model = crate::extract::ConfiguredModel::new(
            run_id,
            Some(crate::self_hosted_jobs::Job::Extraction {
                vault: vault.clone(),
                source: source.clone(),
            }),
        );
        match crate::extract::run(&vault, &source, &model, &me, chrono::Utc::now()).await {
            // "4 promises found in Ops sync" (master plan #2); the items
            // themselves wait in Waiting on you.
            Ok(report) => crate::extract::notify_found(&app, &vault, &report, &kept),
            Err(e) => eprintln!("[extract] {e}"),
        }
    });
    Ok(path)
}
