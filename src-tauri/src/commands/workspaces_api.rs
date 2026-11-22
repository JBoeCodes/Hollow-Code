use super::AppState;
use crate::workspaces::{self, Workspace, WorkspaceDetail};
use tauri::State;

#[tauri::command]
pub async fn list_workspaces(state: State<'_, AppState>) -> Result<Vec<Workspace>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::list(&root))
        .await
        .map_err(|e| e.to_string())?
}
/// App artwork and GitHub owner avatars for the workspace rows.
#[tauri::command]
pub async fn workspace_icons(
    state: State<'_, AppState>,
) -> Result<Vec<crate::project_icons::WorkspaceIcon>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::project_icons::list(&root))
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn create_workspace(
    title: String,
    goal: String,
    state: State<'_, AppState>,
) -> Result<Workspace, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::create(&root, &title, &goal))
        .await
        .map_err(|e| e.to_string())?
}
/// The built-in Quick chats workspace, created on first use.
#[tauri::command]
pub async fn ensure_quick_chats_workspace(state: State<'_, AppState>) -> Result<Workspace, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::quick_chats_workspace(&root))
        .await
        .map_err(|e| e.to_string())?
}
/// The built-in Main agent workspace, created on first use (main agent M1).
#[tauri::command]
pub async fn ensure_main_agent_workspace(state: State<'_, AppState>) -> Result<Workspace, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::main_agent_workspace(&root))
        .await
        .map_err(|e| e.to_string())?
}
/// The person's explicit restore-defaults click, or the COO's
/// `workspaces.create_main_standing_orders` after a confirm card.
#[tauri::command]
pub async fn create_main_standing_orders(state: State<'_, AppState>) -> Result<bool, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || create_standing_orders_noted(&root))
        .await
        .map_err(|e| e.to_string())?
}

/// Create the Main agent's default standing orders when there are none (never
/// replaces a file), and tell the run guard the host wrote them. The
/// protected-orders write both callers share.
pub(crate) fn create_standing_orders_noted(root: &std::path::Path) -> Result<bool, String> {
    let created = workspaces::create_standing_orders(root)?;
    if created {
        crate::run_guard::note_host_vault_write(
            root,
            &format!(
                "workspaces/{}/{}",
                workspaces::MAIN_AGENT_ID,
                workspaces::STANDING_ORDERS
            ),
            None,
            Some(workspaces::STANDING_ORDERS_SEED.as_bytes()),
        );
    }
    Ok(created)
}
#[tauri::command]
pub async fn read_workspace(
    id: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceDetail, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::read(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn attach_workspace_source(
    id: String,
    source: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceDetail, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::attach(&root, &id, &source))
        .await
        .map_err(|e| e.to_string())?
}

/// Compare a snapshot (`sources/<name>`) with its original by content
/// before a refresh (plan L3 X5, 2026-10-14; review U library). Reads only.
/// The COO's `workspaces.preview_source_refresh` is a read too (L13): it
/// returns the original's text, and the original can be any file on this Mac
/// (up to 1 MB), which an agent could already read.
#[tauri::command]
pub async fn preview_source_refresh(
    id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<workspaces::SourceRefreshPreview, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::preview_refresh(&root, &id, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// Replace a snapshot with its original's current bytes, the ones the
/// person reviewed in the preview: refused with `SOURCE_CHANGED:` when the
/// original or the snapshot no longer hashes to `expectedOriginSha` /
/// `expectedSnapshotSha`. The person's click, or the COO's
/// `workspaces.refresh_source` (a confirm card; the earlier copy is kept).
#[tauri::command]
pub async fn apply_source_refresh(
    id: String,
    path: String,
    expected_origin_sha: String,
    expected_snapshot_sha: String,
    state: State<'_, AppState>,
) -> Result<WorkspaceDetail, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        workspaces::refresh_source(
            &root,
            &id,
            &path,
            &expected_origin_sha,
            &expected_snapshot_sha,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The unchecked items of a workspace's `PLAN.md`, each with its idempotency
/// key and the task an earlier create with that key made (Paperclip PR 11).
#[tauri::command]
pub async fn plan_task_candidates(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<workspaces::PlanTaskCandidate>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::plan_task_candidates(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Tick or untick one PLAN.md row from the panel's Plan view (Canvas C5a):
/// the row at `line` whose text hashes to `lineSha256`, in a PLAN.md whose
/// revision is still `expectedRevision`. Refused with `PLAN_CHANGED:` (and
/// nothing written) when either moved on. The person's own click only:
/// user-only in the command map, never an agent action.
#[tauri::command]
pub async fn toggle_plan_row(
    workspace_id: String,
    line: u32,
    line_sha256: String,
    expected_revision: String,
    done: bool,
    state: State<'_, AppState>,
) -> Result<crate::plan_rows::PlanRowToggled, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::plan_rows::toggle(
            &root,
            &workspace_id,
            line,
            &line_sha256,
            &expected_revision,
            done,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Workspace `id`'s Activity (Canvas C5b · Q3): conversations, runs with
/// their receipt and usage, result events and installs, newest first, one
/// page of at most `limit` (default 500, at most 2,000) after `cursor` (the
/// previous page's `nextCursor`; Canvas C6 P3). Read-only over records
/// Hollow already keeps; reading writes nothing, and a re-read reads only
/// what changed.
#[tauri::command]
pub async fn workspace_activity(
    id: String,
    limit: Option<usize>,
    cursor: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::workspace_activity::WorkspaceActivity, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::workspace_activity::list(&root, &id, limit, cursor.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn read_work_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<crate::types::NoteContent, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::read_file(&root, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// Unarchived conversations grouped by workspace, for the rail tree. Those
/// in `pinned` (the sidebar's chat pins) are listed past each workspace's
/// newest 25.
#[tauri::command]
pub async fn workspace_sessions(
    state: State<'_, AppState>,
    pinned: Option<Vec<String>>,
) -> Result<Vec<workspaces::WorkspaceSessions>, String> {
    let root = state.config.vault_path.clone();
    let pinned = pinned.unwrap_or_default();
    crate::vault::spawn_blocking_logged(move || {
        workspaces::sessions_by_workspace_with_pins(&root, &pinned)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Rename a workspace, change its status (`archived` archives it, which
/// unpins it) and/or pin, unpin or move its pin (`crate::workspace_pins`).
#[tauri::command]
pub async fn update_workspace(
    id: String,
    title: Option<String>,
    status: Option<String>,
    pinned: Option<bool>,
    pin_position: Option<u32>,
    state: State<'_, AppState>,
) -> Result<Workspace, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::workspace_pins::update(
            &root,
            &id,
            title.as_deref(),
            status.as_deref(),
            pinned,
            pin_position,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Set the workspace's local canvas palette; None restores the app theme.
#[tauri::command]
pub async fn set_workspace_canvas_theme(
    id: String,
    theme: Option<String>,
    state: State<'_, AppState>,
) -> Result<Workspace, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        workspaces::set_canvas_theme(&root, &id, theme.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Set a workspace's department and default work kind (run ledger v2).
#[tauri::command]
pub async fn set_department(
    id: String,
    department: Option<String>,
    work_kind: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::run_attribution::set_workspace(
            &root,
            &id,
            department.as_deref(),
            work_kind.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Move a workspace folder to the vault's `.trash/`.
#[tauri::command]
pub async fn trash_workspace(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::trash(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Move the newest trashed copy of workspace `id` back (P1-41: Delete's
/// Undo, Home's Recently deleted). Refused while the id is taken.
#[tauri::command]
pub async fn restore_workspace(
    id: String,
    state: State<'_, AppState>,
) -> Result<workspaces::Workspace, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::restore(&root, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Workspaces in the vault's `.trash/`, newest first (Home's Recently deleted).
#[tauri::command]
pub async fn trashed_workspaces(
    state: State<'_, AppState>,
) -> Result<Vec<workspaces::TrashedWorkspace>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || workspaces::trashed(&root))
        .await
        .map_err(|e| e.to_string())?
}

/// What merging workspace `from` into `into` would move (master plan #18):
/// every file and conversation, the hash that binds the confirm, and why it
/// can't run right now (`blocked`, P1-38).
#[tauri::command]
pub async fn preview_workspace_merge(
    from: String,
    into: String,
    state: State<'_, AppState>,
) -> Result<crate::workspace_merge::MergePlan, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::workspace_merge::preview(&root, &from, &into, None)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Merge workspace `from` into `into` from the rail's confirm, inside the
/// workspace edit lease like the rail's other workspace edits (refused while
/// a run holds the vault). `hash` is the previewed plan's: anything that
/// changed since refuses. Undo merge (`undo_workspace_merge`) puts it back.
/// Emits `app-changed` (area `workspaces`) so every window regroups.
#[tauri::command]
pub async fn merge_workspace(
    app: tauri::AppHandle,
    from: String,
    into: String,
    hash: String,
    state: State<'_, AppState>,
) -> Result<crate::workspace_merge::MergeOutcome, String> {
    let root = state.config.vault_path.clone();
    let out = crate::vault::spawn_blocking_logged(move || {
        let _lease = crate::automated_history::workspace_edit_lease(&root).map_err(String::from)?;
        crate::workspace_merge::merge(&root, &from, &into, &hash, None)
    })
    .await
    .map_err(|e| e.to_string())?;
    // Also after a merge that stopped part-way: something may have moved.
    emit_workspaces_changed(&app, "workspaces.merge_workspace");
    out
}

/// Undo the merge recorded in workspace `from` (its menu's Undo merge),
/// inside the workspace edit lease.
#[tauri::command]
pub async fn undo_workspace_merge(
    app: tauri::AppHandle,
    from: String,
    state: State<'_, AppState>,
) -> Result<crate::workspace_merge::UnmergeOutcome, String> {
    let root = state.config.vault_path.clone();
    let out = crate::vault::spawn_blocking_logged(move || {
        let _lease = crate::automated_history::workspace_edit_lease(&root).map_err(String::from)?;
        crate::workspace_merge::undo(&root, &from, None)
    })
    .await
    .map_err(|e| e.to_string())?;
    emit_workspaces_changed(&app, "workspaces.undo_workspace_merge");
    out
}

/// `workspaces.archive_shared_docs` from the workspace menu: one shared doc
/// (`WORKSPACE.md` or `PLAN.md`) of `workspace`. `mode` `dry_run` (the
/// default) returns sizes before and after, what would move and the doc's
/// hash, and writes nothing. `apply` needs that hash as `expected_hash` and
/// the dry run's `today` (so it moves exactly what was shown), and moves
/// nothing when the doc changed; it runs as its own History run
/// ("Archive older entries"), refused while a run is live in the workspace.
/// Emits `app-changed` (area `workspaces`) after an apply.
#[tauri::command]
pub async fn archive_shared_docs(
    app: tauri::AppHandle,
    workspace: String,
    doc: String,
    mode: Option<String>,
    expected_hash: Option<String>,
    today: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::context_compact::SharedDocArchive, String> {
    let root = state.config.vault_path.clone();
    let doc = crate::context_compact::SharedDoc::parse(&doc)?;
    let today = crate::context_compact::parse_today(today.as_deref())?;
    let apply = match mode.as_deref().unwrap_or("dry_run") {
        "dry_run" => false,
        "apply" => true,
        other => {
            return Err(format!(
                "mode must be dry_run or apply, not {other:?}; nothing moved."
            ))
        }
    };
    let expected = expected_hash.unwrap_or_default();
    if apply && expected.trim().is_empty() {
        return Err("apply needs expectedHash from a dry run; nothing moved.".into());
    }
    let out = crate::vault::spawn_blocking_logged(move || {
        if apply {
            crate::context_compact::apply_shared_doc(&root, &workspace, doc, &expected, today)
        } else {
            crate::context_compact::dry_run_shared_doc(&root, &workspace, doc, today, None)
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    if apply {
        emit_workspaces_changed(&app, "workspaces.archive_shared_docs");
    }
    out
}

fn emit_workspaces_changed(app: &tauri::AppHandle, action: &str) {
    use tauri::Emitter;
    let _ = app.emit(
        crate::app_broker::CHANGED_EVENT,
        serde_json::json!({ "action": action, "area": "workspaces" }),
    );
}

/// `hollowHome` and `projectsPath` from the loaded config file (either may
/// be missing on older installs).
pub(crate) fn config_paths(state: &AppState) -> (Option<String>, Option<String>) {
    if state.config_path.as_os_str().is_empty() {
        return (None, None);
    }
    let Ok(map) = crate::setup::load_config_map(&state.config_path) else {
        return (None, None);
    };
    let get = |key: &str| map.get(key).and_then(|v| v.as_str()).map(str::to_string);
    (get("hollowHome"), get("projectsPath"))
}

/// [`create_build_workspace`]'s input: the workspace fields plus the stale
/// base override (OC5). Same JSON shape as before; both new keys optional.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBuildWorkspaceInput {
    #[serde(flatten)]
    pub input: crate::build_workspaces::CreateInput,
    /// Branch from a base too far behind main anyway.
    #[serde(default)]
    pub allow_stale_base: Option<bool>,
    /// Commits behind main at which a base is refused (default
    /// [`crate::build_workspaces::STALE_BASE_LIMIT`]; 0 turns the check off).
    #[serde(default)]
    pub stale_base_limit: Option<u32>,
}

/// Create a build workspace: a new git worktree and branch in `input.repo`,
/// bound to a new workspace whose conversations run in it. A base too far
/// behind main is refused (`STALE_BASE: …`) unless `allowStaleBase`.
#[tauri::command]
pub async fn create_build_workspace(
    input: CreateBuildWorkspaceInput,
    state: State<'_, AppState>,
) -> Result<crate::build_workspaces::Created, String> {
    let defaults = crate::build_workspaces::BasePolicy::default();
    let policy = crate::build_workspaces::BasePolicy {
        limit: input.stale_base_limit.unwrap_or(defaults.limit),
        allow_stale: input.allow_stale_base.unwrap_or(false),
    };
    create_build_workspace_policy(input.input, policy, &state).await
}

/// [`create_build_workspace`] for a caller that holds the state directly
/// (the friction loop's fix draft, master plan #13). Never refused for a
/// stale base: the draft's caller can't send `allowStaleBase`, so a refusal
/// naming it would be a dead end.
pub(crate) async fn create_build_workspace_with(
    input: crate::build_workspaces::CreateInput,
    state: &AppState,
) -> Result<crate::build_workspaces::Created, String> {
    let policy = crate::build_workspaces::BasePolicy {
        allow_stale: true,
        ..Default::default()
    };
    create_build_workspace_policy(input, policy, state).await
}

async fn create_build_workspace_policy(
    input: crate::build_workspaces::CreateInput,
    policy: crate::build_workspaces::BasePolicy,
    state: &AppState,
) -> Result<crate::build_workspaces::Created, String> {
    let root = state.config.vault_path.clone();
    let (home, _) = config_paths(state);
    let worktrees = crate::build_workspaces::worktrees_root(home.as_deref())?;
    crate::build_workspaces::create_with(&root, &worktrees, input, policy).await
}

/// Add another worktree to build workspace `id` (F6 part B), under the
/// same worktrees folder its own went to (`workspaces.add_build_worktree`).
pub(crate) async fn add_build_worktree_for(
    id: &str,
    input: crate::build_workspaces::LaneInput,
    state: &AppState,
) -> Result<crate::build_workspaces::LaneView, String> {
    let root = state.config.vault_path.clone();
    let (home, _) = config_paths(state);
    let worktrees = crate::build_workspaces::worktrees_root(home.as_deref())?;
    crate::build_workspaces::bind_lane(&root, &worktrees, id, input).await
}

/// Remove a build workspace's worktree (the branch is kept). Without
/// `force`, uncommitted or unpushed work refuses with `WORKTREE_UNSAVED:`.
#[tauri::command]
pub async fn remove_build_worktree(
    id: String,
    force: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let root = state.config.vault_path.clone();
    crate::build_workspaces::remove(&root, &id, force).await
}

/// Git repositories in the connected Projects folder, for the new-workspace
/// form.
#[tauri::command]
pub async fn build_repo_candidates(
    state: State<'_, AppState>,
) -> Result<Vec<crate::build_workspaces::RepoCandidate>, String> {
    let (_, projects) = config_paths(&state);
    build_repo_candidates_for(projects).await
}

async fn build_repo_candidates_for(
    projects: Option<String>,
) -> Result<Vec<crate::build_workspaces::RepoCandidate>, String> {
    let projects = projects
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "No Projects folder is set".to_string())?;
    crate::vault::spawn_blocking_logged(move || {
        crate::build_workspaces::repo_candidates(std::path::Path::new(&projects))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A quick chat's scratch folder and its file count (A7). `create` makes it first.
#[tauri::command]
pub async fn quick_chat_scratch(
    key: String,
    create: bool,
    state: State<'_, AppState>,
) -> Result<crate::project_intake::ScratchInfo, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::project_intake::scratch_info(&root, &key, create)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Turn a quick chat's scratch files into project files (A7): they move into the
/// project's `sources/chat-<key>/`.
#[tauri::command]
pub async fn adopt_chat_scratch(
    key: String,
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<crate::project_intake::AdoptReport, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::project_intake::adopt_scratch(&root, &key, &workspace_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Clone a repository from an https:// or ssh:// URL into the Projects folder (A9).
#[tauri::command]
pub async fn clone_repository(
    url: String,
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::project_intake::ClonedRepo, String> {
    let (_, projects) = config_paths(&state);
    let projects = projects
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "No Projects folder is set".to_string())?;
    crate::project_intake::clone_repo(std::path::Path::new(&projects), &url, name.as_deref()).await
}

/// A project's scripts (A10), validated on read.
#[tauri::command]
pub async fn list_project_scripts(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::project_scripts::ScriptView>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::project_scripts::read_scripts(&root, &workspace_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Replace a project's scripts (A10). Every field is validated; the reason comes back as the error.
#[tauri::command]
pub async fn save_project_scripts(
    workspace_id: String,
    scripts: Vec<crate::project_scripts::ProjectScript>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::project_scripts::ScriptView>, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::project_scripts::write_scripts(&root, &workspace_id, scripts)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Run one script because the person clicked Run or pressed its chord (A10): it starts as a
/// service in the project's folder, but only if its command is still `expected_command`, the one the
/// window showed. Never callable by an agent.
#[tauri::command]
pub async fn run_project_script(
    workspace_id: String,
    script_id: String,
    expected_command: String,
    state: State<'_, AppState>,
) -> Result<crate::services::ServiceView, String> {
    let root = state.config.vault_path.clone();
    crate::project_scripts::run_script(&root, &workspace_id, &script_id, &expected_command).await
}

#[cfg(test)]
mod repo_candidate_read_tests {
    use super::build_repo_candidates_for;

    #[tokio::test]
    async fn missing_projects_configuration_explains_why_discovery_is_unavailable() {
        assert_eq!(
            build_repo_candidates_for(None).await.unwrap_err(),
            "No Projects folder is set"
        );
    }

    #[tokio::test]
    async fn unreadable_projects_path_names_the_path_instead_of_an_empty_result() {
        let root = tempfile::Builder::new()
            .prefix("repo-read-")
            .tempdir()
            .unwrap();
        let missing = root.path().join("missing");
        let reason = build_repo_candidates_for(Some(missing.to_string_lossy().into_owned()))
            .await
            .unwrap_err();
        assert!(reason.starts_with("Couldn't read "), "{reason}");
        assert!(
            reason.contains(&missing.to_string_lossy().to_string()),
            "{reason}"
        );
    }

    #[tokio::test]
    async fn a_readable_empty_projects_folder_remains_a_successful_empty_result() {
        let root = tempfile::Builder::new()
            .prefix("repo-read-")
            .tempdir()
            .unwrap();
        assert!(
            build_repo_candidates_for(Some(root.path().to_string_lossy().into_owned()))
                .await
                .unwrap()
                .is_empty()
        );
    }
}

/// Runs on for a vault workspace's own chats (the Mac mini runner, Phase 3,
/// 2026-11-02): `this-mac`, `runner` (a file from before M6 saying
/// `mac-mini` reads as `runner`), or `null` (follows Settings,
/// Devices → Runs on). Kept in app data, never the vault.
#[tauri::command]
pub async fn workspace_runs_on(
    id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    Ok(crate::remote_folder::runs_on_for(
        &state.config.vault_path,
        &id,
    ))
}

/// Set or clear (`null`, `default`) a vault workspace's own Runs on.
#[tauri::command]
pub async fn workspace_set_runs_on(
    id: String,
    runs_on: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    crate::remote_folder::set_runs_on(&state.config.vault_path, &id, runs_on.as_deref())
}

/// A workspace's project record (overhaul B3): the tracker's fields when it
/// is tracked, and the vault's portfolio fields from its PROJECT.md.
#[tauri::command]
pub async fn project_portfolio(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<crate::project_portfolio::ProjectPortfolio, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        crate::project_portfolio::read(&root, &workspace_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Save portfolio fields in a workspace's PROJECT.md (a compare-and-swap on
/// the revision read; refused with `PORTFOLIO_CHANGED:` when it moved on).
/// Never the tracker's fields, and nothing is sent to the tracker.
#[tauri::command]
pub async fn set_project_portfolio(
    update: crate::project_portfolio::PortfolioUpdate,
    state: State<'_, AppState>,
) -> Result<crate::project_portfolio::ProjectPortfolio, String> {
    let root = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::project_portfolio::write(&root, &update))
        .await
        .map_err(|e| e.to_string())?
}
