//! The tracker copies of things that live in Hollow (overhaul A18 and A19):
//! a project's Friday status as a project status update, and a saved canvas
//! as a file on an issue. Hollow keeps the original either way (the status
//! note in the workspace's `outputs/status/`, the canvas in
//! `outputs/canvases/`); the tracker gets a copy for the team.
//!
//! Both are team AI only, journaled in the sync ledger before any request
//! (`statusPosts`, `canvasAttachments`), take the same locks as the issue
//! comment box (B5) and leave one audit row. A status copy is one a project
//! a week (`status-<project>-<ISO week>`) and an unconfirmed one is never
//! resent. A canvas copy keys on the canvas, its revision and the issue: the
//! upload is kept once it lands, and the provider's attach is idempotent per
//! issue and file, so an uncertain attach may be tried again without a
//! second copy.

use super::api::{audit_write, cached};
use super::*;
use crate::tracker::work;

/// The longest status body sent.
pub(crate) const STATUS_MAX_BYTES: usize = 60_000;
/// The largest canvas copied (the adapter's own cap is the same).
pub(crate) const CANVAS_MAX_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const TEAM_AI_ONLY: &str =
    "Hollow sends status updates and canvas copies to team AI only; nothing was sent.";

/// A18's journal key: one status a project an ISO week.
pub(crate) fn status_key(project_id: &str, week: &str) -> String {
    format!("status-{project_id}-{week}")
}

/// A19's journal key: the canvas (workspace, id), its revision and the issue.
pub(crate) fn attach_key(
    workspace_id: &str,
    canvas_id: &str,
    revision: u64,
    issue_id: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!(
        "{workspace_id}\0{canvas_id}\0{revision}\0{issue_id}"
    ));
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("canvas-{hex}")
}

fn audited(
    result: &Result<TrackerWriteOutcome, String>,
    confirmed: bool,
) -> Result<Option<String>, String> {
    match result {
        Ok(outcome) if confirmed => Ok(Some(outcome.note.clone())),
        Ok(outcome) => Err(format!("Unconfirmed: {}", outcome.note)),
        Err(why) => Err(why.clone()),
    }
}

// ── A18: the Friday status as a project status update ────────────────────

/// Post `body` as `project_id`'s status update for `week`, once. Refused
/// before any request unless Settings' "Also post it to the tracker" is on,
/// the team is team AI and the project is a mirrored project of it.
pub(crate) async fn post_status_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    project_id: &str,
    body: &str,
    health: ProjectHealth,
    week: &str,
) -> Result<TrackerWriteOutcome, String> {
    let key = status_key(project_id, week);
    let result = post_status_unaudited(config, remote, project_id, body, health, &key, week).await;
    let confirmed = result.as_ref().is_ok_and(|o| o.id.is_some());
    audit_write(
        &config.vault_path,
        "status_update",
        None,
        (None, None),
        None,
        Some(
            json!({"projectId": project_id, "week": week, "health": health.as_str(), "bytes": body.trim().len()}),
        ),
        Some(&key),
        audited(&result, confirmed),
    );
    result
}

async fn post_status_unaudited(
    config: &Config,
    remote: &dyn TrackerAdapter,
    project_id: &str,
    body: &str,
    health: ProjectHealth,
    key: &str,
    week: &str,
) -> Result<TrackerWriteOutcome, String> {
    if !config.tracker.post_status_updates {
        return Err("Posting status updates to the tracker is off (Settings, Task tracker). The status stays in Hollow.".into());
    }
    enabled(config)?;
    if !work::ai_scope(config) {
        return Err(TEAM_AI_ONLY.into());
    }
    let body = body.trim();
    if body.is_empty() || body.len() > STATUS_MAX_BYTES {
        return Err("A status update needs a body of at most 60,000 bytes.".into());
    }
    if !valid_id(key) {
        return Err("A status update needs a stable request key.".into());
    }
    let root = &config.vault_path;
    let _guard = root_lock(root).lock_owned().await;
    let _file = file_lock(root)?;
    let mut state = cached(config)?;
    if remote.provider() != state.provider
        || remote.scope().await.map_err(|e| e.to_string())?.id != state.scope
    {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let record = state
        .records
        .get(project_id)
        .cloned()
        .ok_or("Sync the tracker before posting that project's status.")?;
    let project = record
        .project
        .as_ref()
        .filter(|p| record.kind == "project" && p.team_ids.contains(&state.scope))
        .ok_or("Only a project of the team Hollow syncs can take a status update.")?;
    if record.state != "active" || record.remote_missing {
        return Err("Only an available tracker project can take a status update.".into());
    }
    let name = project.name.clone();
    let _lease = crate::automated_history::tracker_mirror_write_lease(root, &folders(&state, []))?;
    use sha2::{Digest, Sha256};
    let body_sha256: String = Sha256::digest(body.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    match state.status_posts.get(key) {
        Some(entry) if entry.project_id != project_id => {
            return Err("That status key already names another project.".into())
        }
        Some(_) => {}
        None => {
            state.status_posts.insert(
                key.into(),
                StatusPost {
                    project_id: project_id.into(),
                    week: week.into(),
                    body_sha256,
                    health,
                    attempted: false,
                    update_id: None,
                    error: None,
                },
            );
            save(root, &state)?;
        }
    }
    if !state.status_posts[key].attempted {
        state.status_posts.get_mut(key).unwrap().attempted = true;
        save(root, &state)?;
        let authorized = std::sync::atomic::AtomicBool::new(false);
        let authorize = || -> Result<(), TrackerError> {
            authorized.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        };
        match remote
            .post_project_update(project_id, body, health, &authorize)
            .await
        {
            Ok(update) => state.status_posts.get_mut(key).unwrap().update_id = Some(update.id),
            // Nothing went out before the team check passed: it may be tried again.
            Err(error) if !authorized.load(std::sync::atomic::Ordering::SeqCst) => {
                state.status_posts.remove(key);
                save(root, &state)?;
                return Err(error.to_string());
            }
            Err(error) => {
                state.status_posts.get_mut(key).unwrap().error = Some(format!(
                    "The status update is unconfirmed: {error}. It will not be resent this week; check the tracker."
                ));
            }
        }
        save(root, &state)?;
    }
    let entry = &state.status_posts[key];
    Ok(TrackerWriteOutcome {
        id: entry.update_id.clone(),
        key: None,
        path: Some(record.path.clone()),
        queued: false,
        note: entry.error.clone().unwrap_or_else(|| {
            if entry.update_id.is_some() {
                format!("Status for {name} posted to the tracker for {}.", entry.week)
            } else {
                "The status update is unconfirmed: Hollow stopped while sending it. It will not be resent this week; check the tracker.".into()
            }
        }),
    })
}

// ── A19: a saved canvas as a file on an issue ────────────────────────────

/// The canvas a copy is made from.
#[derive(Clone, Debug)]
pub(crate) struct CanvasCopy {
    pub workspace_id: String,
    pub canvas_id: String,
    pub revision: u64,
    pub fence_sha256: String,
    pub title: String,
    pub html: String,
}

/// Read a saved canvas from its workspace's store (its current revision,
/// or `revision`) and copy it onto `issue_id` through the configured adapter.
pub(crate) async fn attach_canvas(
    config: &Config,
    workspace_id: &str,
    canvas_id: &str,
    revision: Option<u64>,
    issue_id: &str,
) -> Result<TrackerWriteOutcome, String> {
    let copy = read_canvas(&config.vault_path, workspace_id, canvas_id, revision)?;
    let adapter = tracker::adapter(&config.tracker).map_err(|e| e.to_string())?;
    attach_canvas_with(config, adapter.as_ref(), &copy, issue_id).await
}

fn read_canvas(
    vault: &Path,
    workspace_id: &str,
    canvas_id: &str,
    revision: Option<u64>,
) -> Result<CanvasCopy, String> {
    use crate::workspace_canvases::{self as canvases, Scope};
    let app_data = crate::agent_settings::app_data_dir()?;
    let scope = Scope {
        workspace_id: Some(workspace_id),
        session_key: None,
    };
    let canvas = canvases::read(vault, &app_data, scope, canvas_id)?
        .ok_or("That canvas isn't saved in this workspace.")?;
    let (revision, fence_sha256, title, html) = match revision.filter(|r| *r != canvas.revision) {
        None => (
            canvas.revision,
            canvas.fence_sha256,
            canvas.title,
            canvas.html,
        ),
        Some(n) => {
            let old = canvases::read_revision(vault, &app_data, scope, canvas_id, n)?;
            (old.revision, old.fence_sha256, old.title, old.html)
        }
    };
    Ok(CanvasCopy {
        workspace_id: workspace_id.into(),
        canvas_id: canvas_id.into(),
        revision,
        fence_sha256,
        title,
        html,
    })
}

/// The file name a canvas copy gets: its title's letters and digits.
pub(crate) fn canvas_filename(title: &str, canvas_id: &str) -> String {
    let mut name = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c.to_ascii_lowercase());
        } else if !name.ends_with('-') && !name.is_empty() {
            name.push('-');
        }
        if name.len() >= 60 {
            break;
        }
    }
    let name = name.trim_matches('-');
    let stem = if name.is_empty() { canvas_id } else { name };
    format!("{stem}.canvas.html")
}

/// Copy `copy` onto `issue_id`: upload, then attach, once per canvas
/// revision and issue. Refused before any request outside team AI, for an
/// issue the mirror doesn't hold, or an empty or oversized canvas.
pub(crate) async fn attach_canvas_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    copy: &CanvasCopy,
    issue_id: &str,
) -> Result<TrackerWriteOutcome, String> {
    let key = attach_key(&copy.workspace_id, &copy.canvas_id, copy.revision, issue_id);
    let result = attach_unaudited(config, remote, copy, issue_id, &key).await;
    let confirmed = result.as_ref().is_ok_and(|o| o.id.is_some());
    let issue_key = result.as_ref().ok().and_then(|o| o.key.clone());
    audit_write(
        &config.vault_path,
        "attach",
        None,
        (Some(issue_id), issue_key.as_deref()),
        None,
        Some(json!({
            "workspaceId": copy.workspace_id,
            "canvasId": copy.canvas_id,
            "revision": copy.revision,
            "bytes": copy.html.len(),
            "attachmentId": result.as_ref().ok().and_then(|o| o.id.clone()),
        })),
        Some(&key),
        audited(&result, confirmed),
    );
    result
}

async fn attach_unaudited(
    config: &Config,
    remote: &dyn TrackerAdapter,
    copy: &CanvasCopy,
    issue_id: &str,
    key: &str,
) -> Result<TrackerWriteOutcome, String> {
    enabled(config)?;
    if !work::ai_scope(config) {
        return Err(TEAM_AI_ONLY.into());
    }
    if copy.html.trim().is_empty() || copy.html.len() > CANVAS_MAX_BYTES {
        return Err("A canvas copy needs a saved canvas of at most 10 MB.".into());
    }
    let root = &config.vault_path;
    let _guard = root_lock(root).lock_owned().await;
    let _file = file_lock(root)?;
    let mut state = cached(config)?;
    if remote.provider() != state.provider
        || remote.scope().await.map_err(|e| e.to_string())?.id != state.scope
    {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let record = state
        .records
        .get(issue_id)
        .cloned()
        .ok_or("Sync the tracker before attaching to that issue.")?;
    let issue_key = record
        .issue
        .as_ref()
        .filter(|i| i.team_id == state.scope)
        .map(|i| i.key.clone())
        .ok_or("Only an issue of the team Hollow syncs can take a canvas.")?;
    if record.kind != "task" || record.state != "active" || record.remote_missing {
        return Err("Only an available tracker issue can take a canvas.".into());
    }
    let _lease = crate::automated_history::tracker_mirror_write_lease(root, &folders(&state, []))?;
    let outcome = |entry: &CanvasAttach, note: String| TrackerWriteOutcome {
        id: entry.attachment.as_ref().map(|a| a.id.clone()),
        key: Some(issue_key.clone()),
        path: Some(record.path.clone()),
        queued: false,
        note,
    };
    match state.canvas_attachments.get(key) {
        Some(entry)
            if entry.issue_id != issue_id
                || entry.canvas_id != copy.canvas_id
                || entry.revision != copy.revision =>
        {
            return Err("That canvas copy key already names another canvas or issue.".into())
        }
        Some(entry) if entry.attachment.is_some() => {
            return Ok(outcome(
                entry,
                format!(
                    "v{} of this canvas is already on {issue_key}.",
                    copy.revision
                ),
            ))
        }
        Some(_) => {}
        None => {
            state.canvas_attachments.insert(
                key.into(),
                CanvasAttach {
                    issue_id: issue_id.into(),
                    workspace_id: copy.workspace_id.clone(),
                    canvas_id: copy.canvas_id.clone(),
                    revision: copy.revision,
                    fence_sha256: copy.fence_sha256.clone(),
                    upload: None,
                    attachment: None,
                    error: None,
                },
            );
            save(root, &state)?;
        }
    }
    let upload = match state.canvas_attachments[key].upload.clone() {
        Some(upload) => upload,
        None => {
            let name = canvas_filename(&copy.title, &copy.canvas_id);
            match remote
                .upload_file(&name, "text/html; charset=utf-8", copy.html.as_bytes())
                .await
            {
                Ok(upload) => {
                    state.canvas_attachments.get_mut(key).unwrap().upload = Some(upload.clone());
                    save(root, &state)?;
                    upload
                }
                // The file never reached an issue: the copy can start over.
                Err(error) => {
                    state.canvas_attachments.remove(key);
                    save(root, &state)?;
                    return Err(error.to_string());
                }
            }
        }
    };
    let title = format!("Canvas: {} (v{})", copy.title.trim(), copy.revision);
    let authorized = std::sync::atomic::AtomicBool::new(false);
    let authorize = || -> Result<(), TrackerError> {
        authorized.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    };
    let receipt = remote
        .attach_upload(issue_id, &upload, &title, &authorize)
        .await;
    let entry = state.canvas_attachments.get_mut(key).unwrap();
    let note = match receipt {
        Ok(link) => {
            entry.attachment = Some(link.clone());
            entry.error = None;
            if let Some(mirrored) = state
                .records
                .get_mut(issue_id)
                .and_then(|r| r.issue.as_mut())
            {
                if !mirrored.attachments.iter().any(|a| a.id == link.id) {
                    mirrored.attachments.push(link);
                }
            }
            format!(
                "v{} of the canvas is on {issue_key} as a copy; the canvas stays in Hollow.",
                copy.revision
            )
        }
        Err(error) if !authorized.load(std::sync::atomic::Ordering::SeqCst) => {
            entry.error = Some(error.to_string());
            save(root, &state)?;
            return Err(error.to_string());
        }
        Err(error) => {
            let why = format!(
                "The canvas copy is unconfirmed: {error}. Attach it again to finish; it won't add a second copy."
            );
            entry.error = Some(why.clone());
            why
        }
    };
    save(root, &state)?;
    request_soon();
    Ok(outcome(&state.canvas_attachments[key], note))
}

// ── T1: the labels Hollow keeps in each team ───────────────────────────────

/// What `tracker.ensure_labels` found and made in one team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LabelsEnsured {
    /// The team's key (`AI` or `HOL`).
    pub team: String,
    /// Labels it created now.
    pub created: Vec<String>,
    /// Labels it already had (any case).
    pub present: Vec<String>,
}

/// Make sure each of the config's teams (AI, and HOL when set up) has the
/// labels Hollow keeps there (`linear::labels_for`, D8): the team's labels
/// are read first and only the missing ones are created, so a second run
/// creates nothing. Linear only.
pub(crate) async fn ensure_labels(config: &Config) -> Result<Vec<LabelsEnsured>, String> {
    enabled(config)?;
    if config.tracker.provider.as_deref() != Some(tracker::linear::PROVIDER) {
        return Err("Hollow keeps its labels in Linear only; nothing was sent.".into());
    }
    let mut done = Vec::new();
    for team in config.tracker.team_ids() {
        let adapter = tracker::adapter_for(&config.tracker, &team).map_err(|e| e.to_string())?;
        done.push(ensure_labels_with(config, adapter.as_ref(), &team).await?);
    }
    Ok(done)
}

/// [`ensure_labels`] for one team through `remote`, which must be scoped to
/// it. One audit row (`label`) per label created or refused by the tracker.
pub(crate) async fn ensure_labels_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    team: &str,
) -> Result<LabelsEnsured, String> {
    let wanted = tracker::linear::labels_for(team);
    let key = tracker::linear::team_key_of(team).ok_or(tracker::linear::TEAM_ONLY)?;
    if wanted.is_empty() || remote.scope().await.map_err(|e| e.to_string())?.id != team {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let _guard = root_lock(&config.vault_path).lock_owned().await;
    let have = remote.labels().await.map_err(|e| e.to_string())?;
    let mut done = LabelsEnsured {
        team: key.to_string(),
        created: Vec::new(),
        present: Vec::new(),
    };
    for name in wanted {
        if have
            .iter()
            .any(|l| l.name.trim().eq_ignore_ascii_case(name))
        {
            done.present.push(name.to_string());
            continue;
        }
        let made = remote.create_label(name).await.map_err(|e| e.to_string());
        audit_write(
            &config.vault_path,
            "label",
            None,
            (made.as_ref().ok().map(|l| l.id.as_str()), None),
            None,
            Some(json!({"team": key, "label": name})),
            None,
            made.as_ref().map(|_| None).map_err(Clone::clone),
        );
        made?;
        done.created.push(name.to_string());
    }
    Ok(done)
}
