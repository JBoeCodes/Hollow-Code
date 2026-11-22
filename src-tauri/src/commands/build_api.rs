//! Build workspace adapters (Hollow build programme): the Changes pane (PR
//! B2). The domain logic lives in `build_changes.rs`.

use super::AppState;
use crate::build_changes::{self, BuildDiff, BuildStatus};
use tauri::State;

/// The worktree's branch against its base: files, commits, ahead/behind, PR.
#[tauri::command]
pub async fn build_status(id: String, state: State<'_, AppState>) -> Result<BuildStatus, String> {
    build_changes::status(&state.config.vault_path, &id).await
}

/// The rail's branch and PR state for these build workspaces (cached 2 min
/// while the worktree's refs hold; `fresh` ones are always read again).
#[tauri::command]
pub async fn build_rail_states(
    ids: Vec<String>,
    fresh: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<Vec<build_changes::RailGitState>, String> {
    Ok(build_changes::rail_states(
        &state.config.vault_path,
        &ids,
        fresh.as_deref().unwrap_or_default(),
    )
    .await)
}

/// Unified diff of the worktree against its merge-base, redacted and capped.
#[tauri::command]
pub async fn build_diff(
    id: String,
    path: Option<String>,
    state: State<'_, AppState>,
) -> Result<BuildDiff, String> {
    build_changes::diff(&state.config.vault_path, &id, path.as_deref()).await
}

/// `git add -A` + `git commit -m message`. Refused while an agent run holds the vault.
#[tauri::command]
pub async fn build_commit(
    id: String,
    message: String,
    state: State<'_, AppState>,
) -> Result<BuildStatus, String> {
    build_changes::commit(&state.config.vault_path, &id, &message).await
}

/// `git push -u origin <branch>`. Refused while an agent run holds the vault.
#[tauri::command]
pub async fn build_push(id: String, state: State<'_, AppState>) -> Result<BuildStatus, String> {
    build_changes::push(&state.config.vault_path, &id).await
}

/// Update from <base> (P1-46): fetch the base and rebase the branch onto it;
/// a conflict aborts. Refused while an agent works in the worktree.
#[tauri::command]
pub async fn build_update_from_base(
    id: String,
    state: State<'_, AppState>,
) -> Result<BuildStatus, String> {
    build_changes::update_from_base(&state.config.vault_path, &id).await
}

/// Runs on (the remote runner, 2026-11-01): set or clear (`null`,
/// `default`) the build workspace's own choice, `this-mac` or `runner`
/// (`mac-mini`, its old name, is accepted and saved as `runner`).
#[tauri::command]
pub async fn build_set_runs_on(
    id: String,
    runs_on: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::build_workspaces::BuildInfo, String> {
    crate::build_workspaces::set_runs_on(&state.config.vault_path, &id, runs_on.as_deref())
}

/// The remote runner set up for this Mac (`~/.config/hollow/slot-remote`,
/// the one `hollow-slot --remote` uses), or `null`: what Runs on's runner
/// choice runs on.
#[tauri::command]
pub async fn remote_runner() -> Result<Option<String>, String> {
    Ok(crate::remote_session::configured_runner())
}

/// The remote runner's state (overnight N1): whether it answers, its
/// preflight line, the sessions and gate running there and its last gate,
/// with `hollow-slot`'s latest routing decisions and the runner's tools
/// (smart routing 3).
/// `fresh` checks again over ssh (up to 30 s); otherwise the last check's
/// answer (`unchecked` before the first).
#[tauri::command]
pub async fn runner_status(
    fresh: Option<bool>,
) -> Result<crate::remote_status::RunnerStatus, String> {
    let fresh = fresh.unwrap_or(false);
    tokio::task::spawn_blocking(move || {
        let mut status = crate::remote_status::status(fresh);
        status.routing = Some(crate::build_slots::route_status());
        status
    })
    .await
    .map_err(|e| e.to_string())
}

/// `gh pr create` for the worktree's branch (pushing first when needed); the PR URL.
#[tauri::command]
pub async fn build_open_pr(
    id: String,
    title: String,
    body: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    build_changes::open_pr(&state.config.vault_path, &id, &title, &body).await
}

// ------------------------------------------------ Install from Hollow (B5)

use crate::self_install::{self, AppBuildStatus, InstallResult};
use tauri::{AppHandle, Emitter};

/// The app build of workspace `id` (idle when none), and whether its
/// worktree is Hollow's repository.
#[tauri::command]
pub async fn app_build_status(
    id: String,
    state: State<'_, AppState>,
) -> Result<AppBuildStatus, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || self_install::status(&vault, &id))
        .await
        .map_err(|e| e.to_string())
}

/// Compare the finished build's exact receipt; shared with build.install_brings.
#[tauri::command]
pub async fn app_build_brings(
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::install_brings::Report, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || crate::install_brings::for_build(&vault, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Start `npm run tauri:bundle` in the worktree as a background job; progress
/// arrives as `self-install-event`.
#[tauri::command]
pub async fn start_app_build(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppBuildStatus, String> {
    let emitter = app.clone();
    let emit: self_install::Emit = std::sync::Arc::new(move |status| {
        let _ = emitter.emit(self_install::EVENT, status);
    });
    self_install::start_build(&state.config.vault_path, &id, emit).await
}

#[tauri::command]
pub async fn cancel_app_build(id: String) -> Result<(), String> {
    self_install::cancel_build(&id)
}

/// The last self-install's result, until acknowledged.
#[tauri::command]
pub async fn self_install_result() -> Result<Option<InstallResult>, String> {
    Ok(self_install::pending_result())
}

/// The last install's persistent receipt, acknowledged or not, compared with
/// the running app's embedded provenance (next-level P00/P20). A receipt
/// that exists but can't be read is an error, not `null` (P1-46).
#[tauri::command]
pub async fn self_install_receipt() -> Result<Option<crate::self_install::InstallReceipt>, String> {
    crate::self_install::read_last_receipt()
}

#[tauri::command]
pub async fn ack_self_install_result() -> Result<(), String> {
    self_install::acknowledge_result()
}

/// Worktrees, local-only branches and literal `~` folders of this vault's
/// build repositories, and what a cleanup would remove (overnight A9).
#[tauri::command]
pub async fn worktree_hygiene(
    repo: Option<String>,
    state: State<'_, AppState>,
) -> Result<crate::worktree_hygiene::Report, String> {
    Ok(crate::worktree_hygiene::report(&state.config.vault_path, repo.as_deref()).await)
}

/// Remove what `worktree_hygiene` marks. `expected_hash` is the list's hash
/// from that report: a list that changed since refuses (a staleness check;
/// the person's approval is the UI's own click). Branches are never touched.
#[tauri::command]
pub async fn clean_worktrees(
    repo: Option<String>,
    expected_hash: String,
    state: State<'_, AppState>,
) -> Result<crate::worktree_hygiene::Cleaned, String> {
    crate::worktree_hygiene::clean(
        &state.config.vault_path,
        repo.as_deref(),
        &expected_hash,
        None,
    )
    .await
}

/// The native install build folders in ~/Library/Caches and which stay
/// (overnight A5): see `install_targets`.
#[tauri::command]
pub async fn install_targets(keep: Option<usize>) -> Result<crate::install_targets::Plan, String> {
    let keep = keep.unwrap_or(crate::install_targets::DEFAULT_KEEP);
    crate::vault::spawn_blocking_logged(move || crate::install_targets::plan(keep))
        .await
        .map_err(|e| e.to_string())?
}

/// Delete the install build folders `install_targets` lists for removal,
/// bound to `expected_hash` (the list the person saw): a changed list refuses.
#[tauri::command]
pub async fn prune_install_targets(
    keep: Option<usize>,
    expected_hash: String,
) -> Result<crate::install_targets::Pruned, String> {
    let keep = keep.unwrap_or(crate::install_targets::DEFAULT_KEEP);
    crate::vault::spawn_blocking_logged(move || {
        crate::install_targets::prune(keep, Some(&expected_hash))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ------------------------------------------ Services that outlive a turn (B6)

/// Services, all or those of one worktree (`cwd`).
#[tauri::command]
pub async fn list_services(
    cwd: Option<String>,
) -> Result<Vec<crate::services::ServiceView>, String> {
    let scope = cwd
        .map(|c| crate::services::scope_of(std::path::Path::new(&c)))
        .transpose()?;
    Ok(crate::services::registry().list(scope.as_deref()))
}

/// Stop a service (SIGTERM its group, 3 s, SIGKILL).
#[tauri::command]
pub async fn stop_service(cwd: String, name: String) -> Result<(), String> {
    let scope = crate::services::scope_of(std::path::Path::new(&cwd))?;
    crate::services::registry().stop(&scope, &name).await
}

/// The newest lines of a service's log, redacted.
#[tauri::command]
pub async fn service_log(
    cwd: String,
    name: String,
    lines: Option<usize>,
) -> Result<String, String> {
    let scope = crate::services::scope_of(std::path::Path::new(&cwd))?;
    crate::services::registry().logs(&scope, &name, lines)
}

// ------------------------------------------- COO parity (L13): the cards

use crate::app_actions::{Args, Card, Cx};
use serde_json::Value;

/// A string argument (validated already), or "".
fn arg(a: &Args, key: &str) -> String {
    a.0.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// `build.set_runs_on`: old → new, refused for what isn't a build
/// workspace, naming the runner a `runner` (or old `mac-mini`) choice runs on.
pub(crate) fn set_runs_on_card(cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let id = arg(a, "id");
    let info = crate::build_workspaces::binding_for(&cx.vault(), &id)
        .ok_or_else(|| format!("{id} is not a build workspace. Nothing changed."))?;
    let old = info.runs_on.unwrap_or_else(|| "default".into());
    let new = arg(a, "runsOn");
    let mut card = Card::new("Hollow app tools: where a build workspace runs")
        .line(format!(
            "Runs on for build workspace {id}: {old} → {new} (default follows Settings, Devices, Runs on)."
        ))
        .line("It applies from the workspace's next agent turn.");
    if crate::agent_settings::is_runner(&new) {
        card = card.line(match crate::remote_session::configured_runner() {
            Some(host) => format!("runner runs its turns on the remote runner {host} over ssh, with the worktree's checkout sent there."),
            None => "No remote runner is set up on this Mac, so its turns stay here until one is.".into(),
        });
    }
    Ok(Some(card))
}

/// `build.update_from_base` (external: it fetches from the repository's
/// origin): the branch, its base, whether it rebases or merges, and where
/// it fetches from; refused for a detached worktree.
pub(crate) fn update_from_base_card(cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (vault, id) = (cx.vault(), arg(a, "id"));
    let status = tauri::async_runtime::block_on(build_changes::status(&vault, &id))?;
    let origin = tauri::async_runtime::block_on(build_changes::origin_for_card(&vault, &id));
    if status.branch.is_empty() {
        return Err("The worktree is not on a branch (detached HEAD). Nothing changed.".into());
    }
    let base = &status.base;
    let how = if status.upstream.is_some() {
        format!("merge {base} into it (the branch is pushed, so its commits aren't rewritten)")
    } else {
        format!("rebase it onto {base} (it merges instead if the branch is on origin)")
    };
    Ok(Some(
        Card::new("Hollow app tools: update a branch from its base")
            .line(format!(
                "Update branch {} in {}: fetch {base} from origin, then {how}.",
                status.branch, status.worktree
            ))
            .line(format!(
                "It is {} commit(s) behind {base} and {} ahead.",
                status.behind, status.ahead
            ))
            .line("Uncommitted changes refuse it; a conflict or failure aborts and leaves the branch as it was. No agent turn starts in this workspace until it is done.")
            .to(match (base.strip_prefix("origin/"), origin) {
                (Some(branch), Some(origin)) => format!("{origin} (origin): fetches {branch}"),
                (Some(branch), None) => format!("the repository's origin: fetches {branch}"),
                (None, _) => format!("nowhere: {base} is a local branch, so nothing is fetched"),
            }),
    ))
}
