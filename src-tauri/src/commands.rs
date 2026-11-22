use crate::config::Config;
use crate::core_host::Core;
use crate::types::*;
use crate::{
    activation, automated_history, chat, evidence, git_activity, ingest, notifications, plaud,
    project_next_move, quick_add, review_queue, sessions, setup, skills, themes, vault,
    vault_commitments, vault_goals, vault_index, vault_learning, vault_projects, vault_tasks,
    vault_workflows, workflows,
};
use chrono::{Local, NaiveDate};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

pub(crate) mod agenda_api;
pub(crate) mod app_bridge_api;
pub(crate) mod app_tools_api;
pub(crate) mod approvals_api;
pub(crate) mod awareness;
pub(crate) mod budgets_api;
pub(crate) mod build_api;
pub(crate) mod canvases_api;
pub(crate) mod chat_move_api;
pub(crate) mod claude_quota_api;
pub(crate) mod context_review_api;
pub(crate) mod context_scan_api;
pub(crate) mod custom_icons_api;
pub(crate) mod event_triggers_api;
pub(crate) mod feedback_api;
pub(crate) mod interactions_api;
pub(crate) mod move_home;
pub(crate) mod needs_you_api;
pub(crate) mod onepassword_api;
pub(crate) mod peer_api;
pub(crate) mod profile_builder_api;
pub(crate) mod recordings_api;
pub(crate) mod repo_receipts_api;
pub(crate) mod results_api;
pub(crate) mod run_queue_api;
pub(crate) mod runner_api;
pub(crate) mod self_hosted_api;
pub(crate) mod session_import_api;
pub(crate) mod sessions_api;
pub(crate) mod sources_api;
pub(crate) mod system;
pub(crate) mod tasks_tools_api;
pub(crate) mod tracker_api;
pub(crate) mod usage_api;
pub(crate) mod workspaces_api;

/// App-wide shared state passed to every command.
pub struct AppState {
    pub config: Arc<Config>,
    pub config_path: PathBuf,
    /// False when no config.json could be loaded at boot — the app is in
    /// setup mode (placeholder config, Onboarding wizard instead of the
    /// shell). Flips back to true only via restart after `connect_directories`.
    pub configured: bool,
    /// Why the config failed to load, when it did. Surfaced by the wizard.
    pub config_error: Option<String>,
}

impl AppState {
    pub fn new(config: Config, config_path: PathBuf) -> Self {
        Self {
            config: Arc::new(config),
            config_path,
            configured: true,
            config_error: None,
        }
    }

    /// Setup-mode state: empty placeholder paths, so anything gated on
    /// `vault_path.exists()` stays inert until the wizard finishes.
    pub fn unconfigured(error: String) -> Self {
        Self {
            config: Arc::new(Config::placeholder()),
            config_path: PathBuf::new(),
            configured: false,
            config_error: Some(error),
        }
    }
}

fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().to_string()
}

// ── Setup / onboarding ───────────────────────────────────────────────────

/// Whether the app booted with a working config. `main.tsx` calls this before
/// importing the shell: `configured: false` routes to the storeless
/// Onboarding wizard instead.
#[tauri::command]
pub fn setup_status(state: State<'_, AppState>) -> setup::SetupStatus {
    let vault = &state.config.vault_path;
    let has_vault = !vault.as_os_str().is_empty();
    setup::SetupStatus {
        configured: state.configured,
        config_path: if state.config_path.as_os_str().is_empty() {
            None
        } else {
            Some(path_str(&state.config_path))
        },
        config_error: state.config_error.clone(),
        vault_path: has_vault.then(|| path_str(vault)),
        vault_exists: has_vault && vault.exists(),
    }
}

/// Settings, Personalization, Your profile, Create profile notes: the five `me/` starters,
/// strictly additive (an existing note is never overwritten).
#[tauri::command]
pub fn create_profile_notes(
    state: State<'_, AppState>,
) -> Result<setup::ProfileNotesResult, String> {
    setup::create_profile_notes(&state.config.vault_path)
}

/// Relaunch the app so a freshly-written config.json takes effect (config is
/// `Arc<Config>` for the process lifetime). Called from the wizard's done
/// screen — never automatically.
#[tauri::command]
pub async fn setup_restart(
    app: AppHandle,
    confirmed_keys: Option<Vec<String>>,
) -> Result<(), String> {
    // The restart dialog approves these exact runs, not whatever happens to
    // be running after the config write. Read off the event-loop thread.
    if let Some(keys) = confirmed_keys {
        let vault = app
            .try_state::<AppState>()
            .ok_or("Hollow state is unavailable; restart was not requested")?
            .config
            .vault_path
            .clone();
        crate::vault::spawn_blocking_logged(move || {
            crate::lifecycle::check_restart_work(&vault, &keys)
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    crate::lifecycle::expect_exit();
    app.restart();
}

/// Save the canonical Obsidian workspace source. The caller restarts after
/// success because Config is immutable for the process lifetime. Any other
/// source is rejected; a stored `notion` block is left untouched.
#[tauri::command]
pub fn set_workspace_source(
    state: State<'_, AppState>,
    update: setup::WorkspaceSourceUpdate,
) -> Result<(), String> {
    let target = if state.config_path.as_os_str().is_empty() {
        setup::config_write_path()?
    } else {
        state.config_path.clone()
    };
    setup::set_config_workspace_source(&update, &target)
}

// ── Sessions ─────────────────────────────────────────────────────────────

#[tauri::command(async)]
pub fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionMeta>, String> {
    Ok(sessions::list(&state.config.claude_projects_path))
}

#[tauri::command(async)]
pub fn read_session(
    state: State<'_, AppState>,
    project_dir: String,
    session_id: String,
) -> Result<SessionDetail, String> {
    sessions::read_detail(
        &state.config.claude_projects_path,
        &project_dir,
        &session_id,
    )
}

/// Resolve (or generate + cache) a short AI title for one session. Idempotent
/// and cheap on a cache hit; the frontend backfills untitled sessions a few at
/// a time and patches the list as each resolves.
#[tauri::command]
pub async fn generate_session_title(
    state: State<'_, AppState>,
    project_dir: String,
    session_id: String,
) -> Result<String, String> {
    let projects = state.config.claude_projects_path.clone();
    sessions::generate(&projects, &project_dir, &session_id).await
}

// ── Vault ────────────────────────────────────────────────────────────────

/// Whole-vault walks run on the blocking pool, not the main thread or an
/// async worker that chat streaming shares (five asks B4).
#[tauri::command]
pub async fn vault_tree(
    state: State<'_, AppState>,
) -> Result<HashMap<String, vault::VaultNode>, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || vault::build_tree(&vault)).await
}

/// `f` on Tauri's blocking pool.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("background read failed: {e}"))
}

/// [`vault_tree`] for the `hollow_app` broker, which runs off the main thread.
pub fn vault_tree_now(state: State<'_, AppState>) -> HashMap<String, vault::VaultNode> {
    vault::build_tree(&state.config.vault_path)
}

/// Multi-term keyword vault search (grammar in `vault_search`). `kinds`
/// (optional) restricts results to the given semantic kinds (task/meeting/…) so
/// a Vault lens can search within its slice; omit it to search everything.
/// Runs on a blocking worker, off the main thread: every prefilter candidate
/// is scored (there is no early exit), and a query of only 1-2 character terms
/// has no FTS5 set, so it reads every note.
#[tauri::command]
pub async fn vault_search(
    state: State<'_, AppState>,
    query: String,
    kinds: Option<Vec<String>>,
) -> Result<Vec<SearchResult>, String> {
    let vault_path = state.config.vault_path.clone();
    Ok(tauri::async_runtime::spawn_blocking(move || {
        crate::retrieval_broker::search_notes(&vault_path, &query, 50, kinds.as_deref())
    })
    .await
    .unwrap_or_default())
}

/// Local semantic search over the embedded vault (candle, fully on-device).
/// Embeds the query and cosine-ranks the chunk vectors, deduped to the
/// best-scoring chunk per note, returning the same `SearchResult` shape as
/// `vault_search` so the Vault UI renders + routes hits identically. Empty when
/// the feature is off or not yet indexed — the keyword path stays the fallback.
#[tauri::command]
pub async fn semantic_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
    kinds: Option<Vec<String>>,
) -> Result<Vec<SearchResult>, String> {
    if !crate::vault_embed_index::is_semantic_enabled() {
        return Ok(Vec::new());
    }
    let vault_path = state.config.vault_path.clone();
    let request_epoch = crate::embed_model::request_epoch();
    // Model reload after idle eviction must not block the window's command
    // handling. The model mutex still serializes loading and inference.
    let results = tauri::async_runtime::spawn_blocking(move || {
        crate::retrieval_broker::semantic_notes(
            &vault_path,
            &query,
            limit.unwrap_or(50),
            kinds.as_deref(),
            request_epoch,
        )
    })
    .await
    .unwrap_or_default();
    Ok(results)
}

/// Status for the Settings "Semantic search" row: enabled + backfill progress
/// (embedded chunks / total chunks).
#[tauri::command]
pub fn semantic_search_status(state: State<'_, AppState>) -> SemanticStatus {
    let (enabled, indexed, total) =
        crate::vault_embed_index::semantic_status(&state.config.vault_path);
    SemanticStatus {
        enabled,
        indexed,
        total,
    }
}

/// Sync the Settings "Semantic search (local)" toggle into the backend. OFF by
/// default: enabling starts the on-device backfill worker (within ~30s);
/// disabling stops it and makes `semantic_search` a no-op. Keyword search is
/// unaffected either way.
#[tauri::command]
pub fn set_semantic_search_enabled(enabled: bool) {
    crate::vault_embed_index::set_semantic_enabled(enabled);
}

/// The semantic vault index — every file classified into a `VaultItem`
/// (kind / canonical surface / editable / hidden / trust / drift). Backs the
/// Vault tab's lenses, the cleanup queue, and richer agent context. Newest
/// first; tool-internal `_*` files are skipped.
#[tauri::command]
pub async fn vault_items(
    state: State<'_, AppState>,
) -> Result<Vec<vault_index::VaultItem>, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || vault_index::list_items(&vault)).await
}

/// [`vault_items`] for the `hollow_app` broker.
pub fn vault_items_now(state: State<'_, AppState>) -> Vec<vault_index::VaultItem> {
    vault_index::list_items(&state.config.vault_path)
}

#[tauri::command(async)]
pub fn vault_note(state: State<'_, AppState>, path: String) -> Result<NoteContent, String> {
    vault::read_note(&state.config.vault_path, &path)
}

/// Current bytes of a protected note (`me/**`, an agent instruction or config
/// file) before a Library write, for `run_guard` attribution; None for any
/// other path (not read) or an absent file.
fn protected_before(vault: &std::path::Path, rel: &str) -> Option<Vec<u8>> {
    if !crate::run_guard::is_protected_path(rel) {
        return None;
    }
    vault::safe_join(vault, rel)
        .ok()
        .and_then(|full| std::fs::read(full).ok())
}

// The Library editor's note IPCs are user actions (an agent can't invoke
// Tauri IPC), so their writes of protected files are attributed to Hollow:
// editing `me/Guardrails.md` in the Library while a run is active does not
// flag that run. `note_host_vault_write` ignores unprotected paths.

/// Error prefix when `vault_write_note`'s `expected_revision` no longer
/// matches the file: nothing was written.
pub const WRITE_CONFLICT_ERROR: &str = "WRITE_CONFLICT";

/// Overwrite (or create) a note. With `expected_revision` (the `revision` a
/// read returned) the write happens only if the file is still that revision,
/// else it fails with `WRITE_CONFLICT:` naming the current revision and
/// carrying both versions (R7b, `vault_clash`) and writes nothing (merged
/// plan slice 4, A·D7). Without it the write is
/// last-writer-wins, still under the vault mutate lock.
#[tauri::command]
pub fn vault_write_note(
    state: State<'_, AppState>,
    path: String,
    content: String,
    expected_revision: Option<String>,
) -> Result<NoteContent, String> {
    vault_write_note_as(state, "", path, content, expected_revision)
}

/// [`vault_write_note`] for the calling chat `scope` (its session key; the
/// refusal count is per chat).
pub fn vault_write_note_as(
    state: State<'_, AppState>,
    scope: &str,
    path: String,
    content: String,
    expected_revision: Option<String>,
) -> Result<NoteContent, String> {
    let vault = &state.config.vault_path;
    let before = protected_before(vault, &path);
    let expected = match expected_revision.as_deref() {
        Some(revision) => vault::Expected::Revision(revision),
        None => vault::Expected::Anything,
    };
    let note = match vault::write_note_expecting(vault, &path, &content, expected)? {
        vault::CasOutcome::Written(note) => note,
        vault::CasOutcome::Conflict(current) => {
            // R7b: the agent gets both versions to merge itself; one item
            // only when it can't (`vault_clash`).
            return Err(crate::vault_clash::refused(
                vault,
                scope,
                &path,
                expected_revision.as_deref().unwrap_or_default(),
                &content,
                &current,
            ));
        }
    };
    crate::vault_clash::settled(vault, scope, &path);
    crate::run_guard::note_host_vault_write(
        vault,
        &path,
        before.as_deref(),
        Some(content.as_bytes()),
    );
    Ok(note)
}

/// An agent that can't merge a `WRITE_CONFLICT` hands its text over: it is
/// kept beside the file as a conflicted copy, the one item in Waiting on
/// you for that clash (R7b). Returns the copy's name.
#[tauri::command]
pub fn vault_give_up_write(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<String, String> {
    vault_give_up_write_as(state, "", path, content)
}

pub fn vault_give_up_write_as(
    state: State<'_, AppState>,
    scope: &str,
    path: String,
    content: String,
) -> Result<String, String> {
    let vault = &state.config.vault_path;
    crate::app_actions::writable_path(vault, &path)?;
    crate::vault_clash::give_up(vault, scope, &path, &content)
}

#[tauri::command]
pub fn vault_create_note(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<NoteContent, String> {
    let vault = &state.config.vault_path;
    let note = vault::create_note(vault, &path, &content)?;
    crate::run_guard::note_host_vault_write(vault, &path, None, Some(content.as_bytes()));
    Ok(note)
}

#[tauri::command]
pub fn vault_open_in_obsidian(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: Option<String>,
    daily: Option<bool>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let uri = vault::obsidian_uri(
        &state.config.vault_path,
        path.as_deref(),
        daily.unwrap_or(false),
    )?;
    app.opener()
        .open_url(uri, None::<String>)
        .map_err(|e| format!("Could not open Obsidian: {e}"))
}

/// Whether an app on this Mac opens `obsidian://` links (P1-41): the
/// workspace menu's Open brief in Obsidian is off, saying why, when none
/// does. Launch Services' answer, read once per menu; true off macOS.
#[tauri::command]
pub fn obsidian_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let Some(url) = NSURL::URLWithString(&NSString::from_str("obsidian://open")) else {
            return false;
        };
        NSWorkspace::sharedWorkspace()
            .URLForApplicationToOpenURL(&url)
            .is_some()
    }
    #[cfg(not(target_os = "macos"))]
    true
}

/// Compare-and-swap note save for the editor autosave (data safety Tier 0.2).
/// Writes `content` only if the file still equals `baseline` (what the editor
/// loaded); otherwise it returns `{ conflict: true, note: <on-disk> }` so the
/// editor can offer reload / keep-mine instead of silently clobbering an
/// external (Obsidian) edit. On success `{ conflict: false, note: <written> }`.
#[tauri::command]
pub fn vault_save_note_cas(
    state: State<'_, AppState>,
    path: String,
    content: String,
    baseline: String,
) -> Result<SaveNoteResult, String> {
    let vault = &state.config.vault_path;
    match vault::write_note_cas(vault, &path, &content, &baseline)? {
        vault::CasOutcome::Written(note) => {
            // CAS proved the replaced bytes were `baseline` ("" = absent).
            crate::run_guard::note_host_vault_write(
                vault,
                &path,
                Some(baseline.as_bytes()),
                Some(content.as_bytes()),
            );
            Ok(SaveNoteResult {
                conflict: false,
                note,
            })
        }
        vault::CasOutcome::Conflict(note) => Ok(SaveNoteResult {
            conflict: true,
            note,
        }),
    }
}

#[tauri::command]
pub fn vault_move_note(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<NoteContent, String> {
    let vault = &state.config.vault_path;
    let before = protected_before(vault, &from);
    let note = vault::move_note(vault, &from, &to)?;
    crate::run_guard::note_host_vault_write(vault, &from, before.as_deref(), None);
    crate::run_guard::note_host_vault_write(vault, &to, None, Some(note.content.as_bytes()));
    Ok(note)
}

/// Moves the note into the vault's `.trash/` and returns its path there, so
/// the Library's Undo can move it back with [`vault_move_note`] (P1-18).
#[tauri::command]
pub fn vault_delete_note(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let vault = &state.config.vault_path;
    let before = protected_before(vault, &path);
    let trashed = vault::trash_note(vault, &path)?;
    crate::run_guard::note_host_vault_write(vault, &path, before.as_deref(), None);
    Ok(trashed)
}

#[tauri::command]
pub fn vault_reveal_in_finder(state: State<'_, AppState>, path: String) -> Result<(), String> {
    vault::reveal_in_finder(&state.config.vault_path, &path)
}

#[tauri::command]
pub async fn vault_notes_list(state: State<'_, AppState>) -> Result<Vec<NoteRef>, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || vault::notes_list(&vault)).await
}

/// [`vault_notes_list`] for the `hollow_app` broker.
pub fn vault_notes_list_now(state: State<'_, AppState>) -> Vec<NoteRef> {
    vault::notes_list(&state.config.vault_path)
}

/// List the daily briefs (`briefings/{date}.{html,md}`) the Morning Brief
/// workflow produces, newest first. Surfaces `.html` briefs that the
/// markdown-only vault index intentionally skips.
/// Snapshot of the vault as a wikilink graph. One node per `.md`, one
/// directed edge per resolved `[[Other]]`. The Graph page renders this
/// with a force-directed layout. Cheap — reads the existing LinkIndex.
#[tauri::command]
pub fn vault_graph(state: State<'_, AppState>) -> vault::VaultGraph {
    vault::build_graph(&state.config.vault_path)
}

#[tauri::command(async)]
pub fn vault_backlinks(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<BacklinkSource>, String> {
    Ok(vault::backlinks(&state.config.vault_path, &path))
}

// ── Automated change history + evidence ──────────────────────────────────

#[tauri::command(async)]
pub fn list_history_runs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<automated_history::HistoryRun>, String> {
    automated_history::list_runs(&state.config.vault_path, limit.unwrap_or(100))
}

#[tauri::command]
pub fn list_note_history(
    state: State<'_, AppState>,
    path: String,
    limit: Option<usize>,
) -> Result<Vec<automated_history::HistoryRun>, String> {
    automated_history::list_note_history(&state.config.vault_path, &path, limit.unwrap_or(100))
}

#[tauri::command]
pub fn get_history_run(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<automated_history::HistoryRun, String> {
    automated_history::get_run(&state.config.vault_path, &run_id)
}

#[tauri::command]
pub fn read_history_version(
    state: State<'_, AppState>,
    run_id: String,
    path: String,
    version: String,
) -> Result<Option<String>, String> {
    automated_history::read_version(&state.config.vault_path, &run_id, &path, &version)
}

#[tauri::command]
pub fn restore_history_change(
    state: State<'_, AppState>,
    run_id: String,
    path: String,
    force: Option<bool>,
) -> Result<automated_history::HistoryRun, String> {
    automated_history::restore_change(
        &state.config.vault_path,
        &run_id,
        &path,
        force.unwrap_or(false),
    )
}

#[tauri::command]
pub fn restore_history_run(
    state: State<'_, AppState>,
    run_id: String,
    force: Option<bool>,
) -> Result<automated_history::HistoryRun, String> {
    automated_history::restore_run(&state.config.vault_path, &run_id, force.unwrap_or(false))
}

#[tauri::command]
pub fn history_storage_status(
    state: State<'_, AppState>,
) -> Result<automated_history::HistoryStorageStatus, String> {
    automated_history::storage_status(&state.config.vault_path)
}

#[tauri::command]
pub fn clear_automated_history(state: State<'_, AppState>) -> Result<(), String> {
    automated_history::clear_history(&state.config.vault_path)
}

#[tauri::command]
pub fn list_evidence_receipts(
    state: State<'_, AppState>,
    source: Option<String>,
    source_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<evidence::EvidenceReceipt>, String> {
    evidence::list(
        &state.config.vault_path,
        source.as_deref(),
        source_id.as_deref(),
        limit.unwrap_or(100),
    )
}

/// A run's context manifest (agent effectiveness F11): with `run_id`, that
/// run's or none; else with `session_key`, the newest run of that
/// conversation with one, or none; with neither, the newest run with one.
/// Sizes, hashes and decisions; never prompt text.
#[tauri::command]
pub fn run_context_manifest(
    state: State<'_, AppState>,
    run_id: Option<String>,
    session_key: Option<String>,
) -> Result<Option<crate::run_manifest::RunContextView>, String> {
    evidence::context_manifest(
        &state.config.vault_path,
        run_id.as_deref(),
        session_key.as_deref(),
    )
}

#[tauri::command]
pub fn get_evidence_receipt(
    state: State<'_, AppState>,
    id: String,
) -> Result<evidence::EvidenceReceipt, String> {
    evidence::get(&state.config.vault_path, &id)
}

// ── Goals, commitments, and approved learning context ───────────────────

#[tauri::command]
pub fn list_goals(state: State<'_, AppState>) -> Result<Vec<vault_goals::Goal>, String> {
    vault_goals::list(&state.config.vault_path)
}

#[tauri::command]
pub fn create_goal(
    state: State<'_, AppState>,
    input: vault_goals::GoalCreateInput,
) -> Result<vault_goals::Goal, String> {
    vault_goals::create(&state.config.vault_path, input)
}

#[tauri::command]
pub fn update_goal(
    state: State<'_, AppState>,
    url: String,
    input: vault_goals::GoalUpdateInput,
) -> Result<vault_goals::Goal, String> {
    vault_goals::update(&state.config.vault_path, &url, input)
}

#[tauri::command]
pub fn trash_goal(state: State<'_, AppState>, url: String) -> Result<(), String> {
    vault_goals::trash(&state.config.vault_path, &url)
}

#[tauri::command]
pub fn restore_goal(
    state: State<'_, AppState>,
    trash_id: String,
) -> Result<vault_goals::Goal, String> {
    vault_goals::restore(&state.config.vault_path, &trash_id)
}

#[tauri::command]
pub fn goal_alignment(state: State<'_, AppState>) -> Result<vault_goals::GoalAlignment, String> {
    let goals = vault_goals::list(&state.config.vault_path)?;
    let projects = vault_projects::list(&state.config.vault_path);
    Ok(vault_goals::alignment(&goals, &projects))
}

#[tauri::command]
pub async fn suggest_project_next_move(
    state: State<'_, AppState>,
    project_url: String,
) -> Result<project_next_move::ProjectNextMove, String> {
    project_next_move::suggest(&state.config.vault_path, &project_url).await
}

// ── Shared approval inbox ───────────────────────────────────────────────

#[tauri::command]
pub fn list_review_inbox(
    state: State<'_, AppState>,
    status: Option<String>,
    kind: Option<String>,
) -> Result<Vec<review_queue::VersionedItem>, String> {
    let status = status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if status.is_some_and(|value| {
        !matches!(
            value,
            "pending" | "applying" | "accepted" | "rejected" | "snoozed"
        )
    }) {
        return Err(format!(
            "unsupported review inbox status: {}",
            status.unwrap_or_default()
        ));
    }
    let kind = kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    Ok(review_queue::list(&state.config.vault_path, kind)?
        .into_iter()
        .filter(|item| status.map(|value| item.status == value).unwrap_or(true))
        .map(review_queue::VersionedItem::from)
        .collect())
}

#[tauri::command]
pub fn snooze_review_item(
    state: State<'_, AppState>,
    id: String,
    until: String,
) -> Result<review_queue::ReviewItem, String> {
    let vault = &state.config.vault_path;
    // A budget pause is never snoozed (Paperclip PR 8a).
    crate::budgets::refuse_pause_snooze(vault, &id)?;
    review_queue::snooze(vault, &id, &until)
}

/// Wake now (P1-13): the Snoozed list's button and a snooze's Undo put the
/// item back pending. The COO's `review.wake_review_item` does the same.
#[tauri::command]
pub fn wake_review_item(
    state: State<'_, AppState>,
    id: String,
) -> Result<review_queue::ReviewItem, String> {
    review_queue::wake(&state.config.vault_path, &id)
}

#[tauri::command]
pub fn activation_inventory(
    state: State<'_, AppState>,
) -> Result<activation::ActivationInventory, String> {
    activation::inventory(&state.config.vault_path)
}

#[tauri::command]
pub async fn run_activation_scan(
    state: State<'_, AppState>,
) -> Result<activation::ActivationScanResult, String> {
    activation::scan(&state.config.vault_path).await
}

/// Resolve through the kind-specific validated writer. The queue owns state
/// transitions, while the target module remains the authority for what a
/// successful approval is allowed to persist.
///
/// Only a person resolves a review item (owner decision 2026-10-02): this is
/// the review inbox's own click, never a `hollow_app` action
/// (`app_actions::PERSON_RESOLVES`).
#[tauri::command]
pub async fn resolve_review_item(
    state: State<'_, AppState>,
    id: String,
    action: String,
    edited_payload: Option<serde_json::Value>,
    // The fingerprint of the card the user acted on. Required to accept a
    // `workflow_permissions` review (the approval is bound to it); checked
    // for any other kind when supplied.
    expected_fingerprint: Option<String>,
    // The card's reading of the item (`review_queue::version_of`, ME1): a
    // learning's Save is refused when the item moved past it, and an edited
    // one must carry it.
    expected_version: Option<String>,
) -> Result<review_queue::ReviewItem, String> {
    let vault = state.config.vault_path.clone();
    let item = review_queue::list(&vault, None)?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("review item not found: {id}"))?;
    let action = action.trim().to_lowercase();
    // A budget pause (Paperclip PR 8): "Keep paused" holds the pause in
    // budgets.json (outside the vault) and then rejects; both arms re-derive
    // the pause from budgets.json, never from the agent-writable payload.
    if item.kind == crate::budgets::REVIEW_KIND {
        return crate::budgets::resolve_pause(
            &crate::budgets::Store::for_app(&vault),
            &vault,
            &item,
            &action,
            expected_fingerprint.as_deref(),
            edited_payload.as_ref(),
            chrono::Utc::now(),
        );
    }
    // Discard it on an old companion op: answered `refused` to the device
    // (in the ledger, outside the vault; a backend op's receipt is owed to
    // the daemon and sent now), then the plain rejection.
    if item.kind == crate::companion_inbox::OLD_OP_KIND && action == "reject" {
        let store = crate::companion_inbox::Store::for_app()
            .ok_or("Hollow's support folder is unavailable.")?;
        let (vault, item) = (vault.clone(), item.clone());
        let discarded = tauri::async_runtime::spawn_blocking(move || {
            crate::companion_inbox::discard_old_op(&vault, &store, &item, chrono::Local::now())
        })
        .await
        .map_err(|e| format!("companion change: {e}"))?;
        crate::companion_bridge::send_owed_now();
        return discarded;
    }
    match action.as_str() {
        "reject" => return review_queue::reject(&vault, &id),
        "accept" => {}
        other => return Err(format!("unsupported review resolution: {other}")),
    }
    if let Some(expected) = expected_fingerprint.as_deref() {
        if item.kind != crate::workflow_permissions::REVIEW_KIND
            && !item.fingerprint.eq_ignore_ascii_case(expected.trim())
        {
            return Err("This suggestion changed after you opened it. Review it again.".into());
        }
    }

    match item.kind.as_str() {
        "learning" => {
            let edited_statement = edited_payload
                .map(serde_json::from_value::<vault_learning::LearningProposalPayload>)
                .transpose()
                .map_err(|e| format!("invalid learning review payload: {e}"))?
                .map(|payload| payload.statement);
            vault_learning::resolve_proposal_at(
                &vault,
                &id,
                "accept",
                edited_statement.as_deref(),
                expected_version.as_deref(),
            )?;
        }
        "commitment" => {
            let edited = edited_payload
                .map(serde_json::from_value::<vault_commitments::CommitmentProposalPayload>)
                .transpose()
                .map_err(|e| format!("invalid commitment review payload: {e}"))?;
            vault_commitments::resolve_proposal(&vault, &id, "accept", edited)?;
        }
        "project_activation" | "goal" => {
            return activation::resolve(&vault, &id, "accept", edited_payload);
        }
        "heartbeat_send" | "reply_draft" | "meeting_followup" | "external_send" => {
            return Err("Messaging has been removed from Hollow.".into());
        }
        // Meeting follow-up suggestions came from Outlook meeting notes,
        // removed with Outlook on 2026-09-26. Reject leaves the card sticky.
        // An extracted task (master plan F3) carries its anchor and is
        // accepted like an agent's task suggestion. The old Outlook ones
        // were retired on 2026-09-26: reject leaves the card sticky.
        "meeting_task" if item.payload.get("anchor").is_some() => {
            crate::task_broker::accept_extracted_task(&vault, &item, edited_payload)?
        }
        "meeting_task" => {
            return Err("Meeting suggestions were retired with Outlook; reject this suggestion or add the task yourself.".into());
        }
        // An extracted decision or person fact (F3): accepted through F4's
        // records writer.
        "record" => {
            crate::records::accept_proposal(
                &vault,
                &item,
                edited_payload.as_ref(),
                chrono::Utc::now(),
            )?;
        }
        // Master plan #3: the person's answer to "who is this?".
        "person_alias" => {
            crate::people_dossier::resolve(&vault, &item, edited_payload.as_ref())?;
        }
        // Use the device's value (companion apps): re-checked against the
        // task as it is now, then written as the person's edit. Keep the
        // Mac's is the plain rejection above.
        "companion_conflict" => {
            // Blocking (it reads and writes vault files): off the async runtime.
            let store = crate::companion_inbox::Store::for_app()
                .ok_or("Hollow's support folder is unavailable.")?;
            let (vault, item) = (vault.clone(), item.clone());
            tauri::async_runtime::spawn_blocking(move || {
                crate::companion_inbox::accept_conflict(&vault, &store, &item, chrono::Local::now())
            })
            .await
            .map_err(|e| format!("companion conflict: {e}"))??;
        }
        // Apply it on a companion op sent more than 14 days ago: the held,
        // signed op applied once through the inbox's path and ledger. A
        // backend op's receipt is owed to the daemon and sent now.
        "companion_old_op" => {
            let store = crate::companion_inbox::Store::for_app()
                .ok_or("Hollow's support folder is unavailable.")?;
            let (vault, item) = (vault.clone(), item.clone());
            let applied = tauri::async_runtime::spawn_blocking(move || {
                crate::companion_inbox::accept_old_op(&vault, &store, &item, chrono::Local::now())
            })
            .await
            .map_err(|e| format!("companion change: {e}"))?;
            crate::companion_bridge::send_owed_now();
            applied?;
        }
        "heartbeat_task" => {
            resolve_heartbeat_task(state, id.clone(), true).await?;
        }
        // A hollow_tasks suggestion (Paperclip PR 12): the shared idempotent
        // create keyed by the item's fingerprint, so a double accept (or a
        // re-accept after a crash) creates exactly one task.
        "agent_task" => crate::task_broker::accept_agent_task(&vault, &item, edited_payload)?,
        // Re-derives the change from disk and re-stamps the automation outside
        // the vault; never edits the workflow file. Reject stays the plain,
        // sticky queue rejection above.
        "workflow_permissions" => {
            crate::workflow_permissions::approve(
                &vault,
                &item,
                edited_payload.as_ref(),
                expected_fingerprint.as_deref(),
            )?;
        }
        // Pause schedule (Paperclip PR 9, 9a-2): the owned schedule write to
        // `manual`, then accepted, which keeps the automation's streak closed
        // to new cards until it succeeds again. Reject (Dismiss) stays the
        // plain queue rejection above.
        "workflow_failure_streak" => {
            crate::workflow_hygiene::pause_from_review(&vault, &item)?;
        }
        // Do it on the gardener's card (master plan #19): only the ticked
        // lines a fresh scan still finds, then accepted. Keep all is the plain
        // queue rejection above.
        "gardener" => {
            crate::gardener::resolve_review(&vault, &item, edited_payload.as_ref()).await?;
        }
        // Rechecked on a stale memory's card (ME2): closes it; the file is
        // the person's to re-date. Dismiss is the plain rejection above.
        "memory_recheck" => crate::context_stale::mark_rechecked(&vault, &item)?,
        // NV7: the reviewer's expected skill (never the chat's output),
        // checked against the card's version (ME1), then a proposed fixture
        // in the Main agent's outputs; the repo's eval set is never edited.
        "eval_candidate" => {
            let (vault, item) = (vault.clone(), item.clone());
            let version = expected_version.clone();
            tauri::async_runtime::spawn_blocking(move || {
                crate::eval_candidates::accept(
                    &vault,
                    &item,
                    edited_payload.as_ref(),
                    version.as_deref(),
                    chrono::Utc::now(),
                )
            })
            .await
            .map_err(|e| format!("eval candidate: {e}"))??;
        }
        other => {
            return Err(format!(
                "review item kind {other} does not have an approval handler yet"
            ));
        }
    }

    review_queue::list(&vault, None)?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("resolved review item disappeared: {id}"))
}

/// Commitments, optionally only one direction (`owed_by_me`, `owed_to_me`)
/// or one person (master plan #2).
#[tauri::command]
pub fn list_commitments(
    state: State<'_, AppState>,
    direction: Option<String>,
    person: Option<String>,
) -> Vec<vault_commitments::Commitment> {
    vault_commitments::filter(
        vault_commitments::list(&state.config.vault_path),
        direction.as_deref(),
        person.as_deref(),
    )
}

#[tauri::command]
pub fn create_commitment(
    state: State<'_, AppState>,
    input: vault_commitments::CreateCommitmentInput,
) -> Result<vault_commitments::Commitment, String> {
    vault_commitments::create(&state.config.vault_path, input)
}

#[tauri::command]
pub fn update_commitment(
    state: State<'_, AppState>,
    input: vault_commitments::UpdateCommitmentInput,
) -> Result<vault_commitments::Commitment, String> {
    vault_commitments::update(&state.config.vault_path, input)
}

#[tauri::command]
pub fn set_commitment_status(
    state: State<'_, AppState>,
    url: String,
    status: String,
) -> Result<vault_commitments::Commitment, String> {
    vault_commitments::set_status(&state.config.vault_path, &url, &status)
}

#[tauri::command]
pub fn trash_commitment(
    state: State<'_, AppState>,
    url: String,
) -> Result<vault_commitments::TrashedCommitment, String> {
    vault_commitments::trash(&state.config.vault_path, &url)
}

#[tauri::command]
pub fn restore_commitment(
    state: State<'_, AppState>,
    trashed: vault_commitments::TrashedCommitment,
) -> Result<vault_commitments::Commitment, String> {
    vault_commitments::restore(&state.config.vault_path, trashed)
}

#[tauri::command]
pub fn list_commitment_proposals(
    state: State<'_, AppState>,
) -> Result<Vec<vault_commitments::CommitmentProposal>, String> {
    vault_commitments::list_proposals(&state.config.vault_path)
}

#[tauri::command]
pub fn submit_commitment_proposal(
    state: State<'_, AppState>,
    payload: vault_commitments::CommitmentProposalPayload,
) -> Result<vault_commitments::CommitmentProposal, String> {
    vault_commitments::submit_proposal(&state.config.vault_path, payload)
}

#[tauri::command]
pub fn resolve_commitment_proposal(
    state: State<'_, AppState>,
    id: String,
    action: String,
    edited_payload: Option<vault_commitments::CommitmentProposalPayload>,
) -> Result<vault_commitments::CommitmentProposal, String> {
    vault_commitments::resolve_proposal(&state.config.vault_path, &id, &action, edited_payload)
}

#[tauri::command]
pub fn list_learning_proposals(
    state: State<'_, AppState>,
) -> Result<Vec<vault_learning::LearningProposal>, String> {
    vault_learning::list_proposals(&state.config.vault_path)
}

#[tauri::command]
pub fn list_skill_lessons(
    state: State<'_, AppState>,
) -> Result<Vec<crate::review_queue::ReviewItem>, String> {
    crate::skill_learning::list(&state.config.vault_path)
}

#[tauri::command]
pub fn resolve_skill_lesson(
    state: State<'_, AppState>,
    id: String,
    action: String,
) -> Result<crate::review_queue::ReviewItem, String> {
    crate::skill_learning::resolve(&state.config.vault_path, &id, &action)
}

#[tauri::command]
pub fn undo_skill_lesson(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::review_queue::ReviewItem, String> {
    crate::skill_learning::undo(&state.config.vault_path, &id)
}

#[tauri::command]
pub fn list_private_memories(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<crate::private_memory::Entry>, String> {
    crate::private_memory::list(&state.config.vault_path, &workspace_id)
}

#[tauri::command]
pub fn promote_private_memory(
    state: State<'_, AppState>,
    workspace_id: String,
    id: String,
    expected_hash: String,
    scope: String,
    statement: String,
) -> Result<vault_learning::LearningProposal, String> {
    crate::private_memory::promote(
        &state.config.vault_path,
        &workspace_id,
        &id,
        &expected_hash,
        &scope,
        &statement,
    )
}

#[tauri::command]
pub fn submit_learning_proposal(
    state: State<'_, AppState>,
    payload: vault_learning::LearningProposalPayload,
) -> Result<vault_learning::LearningProposal, String> {
    vault_learning::submit_proposal(&state.config.vault_path, payload)
}

#[tauri::command]
pub fn resolve_learning_proposal(
    state: State<'_, AppState>,
    id: String,
    action: String,
    edited_statement: Option<String>,
    expected_version: Option<String>,
) -> Result<vault_learning::LearningProposal, String> {
    vault_learning::resolve_proposal_at(
        &state.config.vault_path,
        &id,
        &action,
        edited_statement.as_deref(),
        expected_version.as_deref(),
    )
}

/// Settings, Memory, Learning suggestions (fewer learnings, 10-11).
#[tauri::command]
pub fn get_learning_settings() -> vault_learning::LearningSettings {
    vault_learning::settings()
}

#[tauri::command]
pub fn set_learning_suggestions(mode: String) -> Result<vault_learning::LearningSettings, String> {
    let mode = vault_learning::Suggestions::parse(&mode)
        .ok_or_else(|| format!("unknown learning suggestions `{mode}` (off, important or all)"))?;
    vault_learning::set_suggestions(mode)
}

/// Bulk review (the person's click only; never an app action): accept or
/// reject several learning proposals at once, each reported on its own,
/// with one batch token for Undo. A row's Save is a one-item batch, with
/// `edited_statement` when the person changed the words, so it has the same
/// Undo. Off the main thread: a batch rewrites the review queue once per
/// proposal.
#[tauri::command]
pub async fn resolve_learning_proposals_batch(
    state: State<'_, AppState>,
    ids: Vec<String>,
    action: String,
    reason: Option<String>,
    edited_statement: Option<String>,
    // ME1: a one-item Save's reading of its proposal; edited words need it.
    expected_version: Option<String>,
) -> Result<vault_learning::LearningBatchOutcome, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || {
        vault_learning::resolve_batch_versioned(
            &vault,
            &ids,
            &action,
            reason.as_deref(),
            edited_statement.as_deref(),
            expected_version.as_deref(),
        )
    })
    .await?
}

/// Undo a bulk review: its proposals back to pending, an accepted one's
/// note out of `learnings/`. Fails when the batch was already undone.
#[tauri::command]
pub async fn undo_learning_batch(
    state: State<'_, AppState>,
    batch_id: String,
) -> Result<vault_learning::LearningBatchOutcome, String> {
    let vault = state.config.vault_path.clone();
    blocking(move || vault_learning::undo_batch(&vault, &batch_id)).await?
}

/// Undo of one Reject (P0-10): the snackbar's Undo after the person rejected
/// a review card puts the same item back pending. The person's click only,
/// never an action (`app_actions::PERSON_RESOLVES`).
#[tauri::command]
pub fn undo_review_reject(
    state: State<'_, AppState>,
    id: String,
    // The rejected item's `updatedAt` as the Reject returned it: an Undo
    // never takes back a later rejection it didn't see.
    expected_updated_at: String,
) -> Result<review_queue::ReviewItem, String> {
    review_queue::undo_reject(&state.config.vault_path, &id, &expected_updated_at)
}

#[tauri::command]
pub fn list_learning_context(
    state: State<'_, AppState>,
    include_superseded: Option<bool>,
) -> Vec<vault_learning::LearningContext> {
    vault_learning::list_context(
        &state.config.vault_path,
        include_superseded.unwrap_or(false),
    )
}

#[tauri::command]
pub fn memory_maintenance(
    state: State<'_, AppState>,
) -> Result<crate::memory_maintenance::View, String> {
    crate::memory_maintenance::refresh(&state.config.vault_path)
}

#[tauri::command]
pub fn edit_memory_maintenance(
    state: State<'_, AppState>,
    key: String,
    expected_hash: String,
    action: String,
    date: Option<String>,
) -> Result<crate::memory_maintenance::View, String> {
    crate::memory_maintenance::edit(
        &state.config.vault_path,
        &key,
        &expected_hash,
        &action,
        date.as_deref(),
    )
}

#[tauri::command]
pub fn supersede_learning_context(
    state: State<'_, AppState>,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    vault_learning::supersede_context(&state.config.vault_path, &old_path, &new_path)
}

#[tauri::command]
pub fn recall_learning_context(
    state: State<'_, AppState>,
    query: String,
    subject_paths: Vec<String>,
) -> vault_learning::RecallResult {
    vault_learning::recall(&state.config.vault_path, &query, &subject_paths)
}

// ── Tasks & Projects (vault mirrors) ─────────────────────────────────────

fn days_until_due(due: &Option<String>) -> Option<i64> {
    let due = due.as_ref()?;
    let d = NaiveDate::parse_from_str(due, "%Y-%m-%d").ok()?;
    let today = Local::now().date_naive();
    Some(d.signed_duration_since(today).num_days())
}

fn bucket_from_days(d: Option<i64>) -> String {
    match d {
        None => "undated".into(),
        Some(x) if x < 0 => "overdue".into(),
        Some(0) => "today".into(),
        Some(x) if x <= 7 => "thisweek".into(),
        _ => "later".into(),
    }
}

/// Enrich raw vault tasks with computed buckets, due-date deltas, and the
/// vault-relative path. Project names come straight off the `project:`
/// frontmatter field during parse. The frontend renders these directly.
fn enrich_tasks(state: &AppState, mut items: Vec<Task>) -> TasksResponse {
    // Which fields a tracker issue owns, so TaskModal locks them.
    vault_tasks::mark_task_locks(&state.config.vault_path, &mut items);
    // Where each task lives, in words (H4b): "Tracked in Linear", "Stored in Hollow".
    sources_api::label_tasks_for(state, &mut items);
    for t in items.iter_mut() {
        let d = days_until_due(&t.due);
        // A completed task is never overdue/today/etc. — it drops into the
        // terminal "done" bucket so it leaves the active date groups and the
        // dashboard overdue/today counts, regardless of its due date. Done via
        // the shared `is_terminal_status` so `Delivered`, `Done`, `Complete`
        // (any casing the app or the embedded chat writes) all count.
        t.bucket = if vault_tasks::is_terminal_status(&t.status) {
            "done".into()
        } else {
            bucket_from_days(d)
        };
        t.days_until_due = d;
        // Vault-relative path to the task file, so the UI can open it in the
        // Vault tab. Every task is keyed by its `vault:` url now.
        if let Some(rel) = t.url.strip_prefix("vault:") {
            t.vault_path = Some(rel.to_string());
        }
    }

    // Sort by due date so the UI gets stable ordering.
    items.sort_by(|a, b| {
        a.due
            .clone()
            .unwrap_or_else(|| "9999".into())
            .cmp(&b.due.clone().unwrap_or_else(|| "9999".into()))
    });

    let (last_synced_at, sync_error) = (None, None);
    TasksResponse {
        items,
        fetched: None,
        source: "vault".into(),
        todo_path: None,
        last_synced_at,
        sync_error,
    }
}

/// Tasks always render from canonical Obsidian vault markdown.
fn tasks_inner(state: &AppState) -> TasksResponse {
    let items = vault_tasks::list(&state.config.vault_path);
    enrich_tasks(state, items)
}

/// Projects always render from canonical vault markdown and status folders.
fn projects_inner(state: &AppState) -> ProjectsResponse {
    let items = vault_projects::list(&state.config.vault_path);
    let (last_synced_at, sync_error) = (None, None);
    ProjectsResponse {
        items,
        fetched: None,
        source: "vault".into(),
        last_synced_at,
        sync_error,
    }
}

#[tauri::command]
pub async fn tasks(state: State<'_, AppState>) -> Result<TasksResponse, String> {
    Ok(tasks_inner(&state))
}

#[tauri::command]
pub async fn projects(state: State<'_, AppState>) -> Result<ProjectsResponse, String> {
    Ok(projects_inner(&state))
}

// ── Dashboard (combined for the Today page) ──────────────────────────────

#[tauri::command]
pub fn dashboard(state: State<'_, AppState>) -> Dashboard {
    let s = state.inner();
    let sessions = sessions::list(&s.config.claude_projects_path);
    let recent_notes = vault::recent(&s.config.vault_path, 10);
    let tasks_resp = tasks_inner(s);
    let projects_resp = projects_inner(s);
    let overdue = tasks_resp
        .items
        .iter()
        .filter(|t| t.bucket == "overdue")
        .count();
    let today = tasks_resp
        .items
        .iter()
        .filter(|t| t.bucket == "today")
        .count();
    let active_projects: Vec<Project> = projects_resp
        .items
        .iter()
        .filter(|p| p.status == "Active")
        .cloned()
        .collect();
    let mut sorted_tasks = tasks_resp.items.clone();
    let order = |b: &str| match b {
        "overdue" => 0,
        "today" => 1,
        "thisweek" => 2,
        "later" => 3,
        "done" => 5,
        _ => 4,
    };
    sorted_tasks.sort_by(|a, b| {
        order(&a.bucket).cmp(&order(&b.bucket)).then(
            a.due
                .clone()
                .unwrap_or_else(|| "9999".into())
                .cmp(&b.due.clone().unwrap_or_else(|| "9999".into())),
        )
    });
    Dashboard {
        sessions_total: sessions.len(),
        sessions: sessions.into_iter().take(6).collect(),
        recent_notes,
        tasks_total: tasks_resp.items.len(),
        tasks: sorted_tasks.into_iter().take(8).collect(),
        overdue_count: overdue,
        today_count: today,
        projects_total: projects_resp.items.len(),
        active_projects,
        tasks_fetched: tasks_resp.fetched,
        projects_fetched: projects_resp.fetched,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

/// Git contribution heatmap for the Today dashboard. Scans the configured
/// `gitReposPath` (default: the vault's parent directory) one level deep for
/// git repositories and aggregates commit counts per day over the trailing
/// ~53 weeks. Pure read — runs `git log` per repo concurrently, never writes.
#[tauri::command]
pub async fn git_activity(state: State<'_, AppState>) -> Result<GitActivity, String> {
    let root = state
        .config
        .git_repos_path
        .clone()
        .or_else(|| state.config.vault_path.parent().map(|p| p.to_path_buf()))
        .ok_or_else(|| "no gitReposPath configured and vault path has no parent".to_string())?;
    Ok(git_activity::collect(&root).await)
}

// ── Task & project mutations (always vault-local) ──────────────

/// Set a task's status (rolling a recurring task forward instead of
/// finishing it). Returns the written file's content hash, so the UI can
/// refresh the CAS baseline it holds without waiting for a refetch.
#[tauri::command]
pub async fn set_task_status(
    state: State<'_, AppState>,
    url: String,
    status: String,
    expected: Option<String>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    let rel = vault_tasks::find_by_url(&vault, &url)
        .ok_or_else(|| format!("no vault task for url: {url}"))?;
    // `expected` (review F01): bulk apply sends the previewed contentHash and
    // its Undo the hash the apply returned, so a newer edit is refused
    // (`TASK_CONFLICT: …`) rather than overwritten. Omitted means no check.
    // Recurrence: completing a recurring task rolls its due date to the next
    // occurrence (anchored on today) and keeps it active. The read, decision
    // and write are one critical section in the shared writer (Paperclip
    // PR 12 extracted it; the MCP set_status uses the same function).
    if crate::tracker_sync::seams::tracked(&vault, &rel) {
        return crate::tracker_sync::seams::user_edit(
            &state.config,
            rel,
            expected,
            serde_json::Map::from_iter([("status".into(), serde_json::json!(status))]),
        )
        .await;
    }
    vault_tasks::set_status_with_recurrence(
        &vault,
        &rel,
        &status,
        Local::now().date_naive(),
        expected.as_deref(),
    )
    .map(|outcome| outcome.content_hash)
}

/// Set a task's priority. Returns the written file's content hash.
#[tauri::command]
pub async fn set_task_priority(
    state: State<'_, AppState>,
    url: String,
    priority: Option<String>,
    expected: Option<String>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    let rel = vault_tasks::find_by_url(&vault, &url)
        .ok_or_else(|| format!("no vault task for url: {url}"))?;
    if crate::tracker_sync::seams::tracked(&vault, &rel) {
        return crate::tracker_sync::seams::user_edit(
            &state.config,
            rel,
            expected,
            serde_json::Map::from_iter([("priority".into(), serde_json::json!(priority))]),
        )
        .await;
    }
    vault_tasks::set_priority_checked(&vault, &rel, priority.as_deref(), expected.as_deref())
}

#[tauri::command]
pub async fn set_project_status(
    state: State<'_, AppState>,
    url: String,
    status: String,
) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    let rel = vault_projects::find_by_url(&vault, &url)
        .ok_or_else(|| format!("no vault project for url: {url}"))?;
    if crate::tracker_sync::seams::tracked(&vault, &rel) {
        let current = vault::read_note(&vault, &rel)?;
        return crate::tracker_sync::seams::user_edit(
            &state.config,
            rel,
            Some(vault_tasks::content_hash(&current.content)),
            serde_json::Map::from_iter([("status".into(), serde_json::json!(status))]),
        )
        .await
        .map(|_| ());
    }
    vault_projects::set_status(&vault, &rel, &status).map(|_| ())
}

/// Apply an in-app edit directly to the canonical project markdown.
#[tauri::command]
pub async fn update_project(
    state: State<'_, AppState>,
    input: UpdateProjectInput,
) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    let rel = vault_projects::find_by_url(&vault, &input.url)
        .ok_or_else(|| format!("no vault project for url: {}", input.url))?;
    if crate::tracker_sync::seams::tracked(&vault, &rel) {
        let current = vault::read_note(&vault, &rel)?;
        let mut fields = serde_json::Map::new();
        for (key, value) in [
            ("title", input.title),
            ("status", input.status),
            ("priority", input.priority),
            ("description", input.description),
            ("targetDate", input.target_date),
            ("currentState", input.current_state),
            ("desiredState", input.desired_state),
            ("nonGoals", input.non_goals),
            ("successEvidence", input.success_evidence),
        ] {
            if let Some(value) = value {
                fields.insert(key.into(), serde_json::json!(value));
            }
        }
        if let Some(urls) = input.goal_urls {
            fields.insert("goalUrls".into(), serde_json::json!(urls));
        }
        return crate::tracker_sync::seams::user_edit(
            &state.config,
            rel,
            Some(vault_tasks::content_hash(&current.content)),
            fields,
        )
        .await
        .map(|_| ());
    }
    vault_projects::update(&vault, &rel, &input).map(|_| ())
}

/// Create a task as canonical markdown under `vault/tasks/` through the
/// shared [`vault_tasks::create_from_input`] path. With an `idempotencyKey`
/// whose fingerprint an existing task carries, that task is returned
/// (`existing: true`) and nothing is written. `blockedBy` entries must name
/// existing tasks (`TASK_BLOCKER: …`) and must not close a loop
/// (`TASK_CYCLE: …`). The scan and create run off the async executor.
#[tauri::command]
pub async fn create_task(
    state: State<'_, AppState>,
    input: CreateTaskInput,
) -> Result<CreatedTask, String> {
    let vault = state.config.vault_path.clone();
    let created = tauri::async_runtime::spawn_blocking(move || {
        vault_tasks::create_from_input(&vault, &input)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(CreatedTask {
        url: format!("vault:{}", created.rel),
        id: created.rel,
        existing: created.existing,
    })
}

// ── Quick-add NLP, full task edit, sections, migration ───────────────────

/// Parse a natural-language quick-add line into structured fields. Pure +
/// synchronous — drives the live preview and the create flow.
#[tauri::command]
pub fn parse_quick_add(input: String) -> ParsedQuickAdd {
    quick_add::parse(&input, Local::now().date_naive())
}

/// Apply a full edit (priority, labels, section, due/time, duration,
/// recurrence, description, sub-tasks) to a task file by line-patching the
/// changed keys. With `input.expected` the write is compare-and-swap: a
/// stale baseline rejects with `TASK_CONFLICT: …` and writes nothing. A
/// project change on a Notion-linked task rejects with `TASK_LINKED: …`. A
/// `blockedBy` list whose new entries name no task rejects with
/// `TASK_BLOCKER: …`, and one that would close a loop (or names the task
/// itself) with `TASK_CYCLE: …`; neither writes anything.
#[tauri::command]
pub async fn update_task(
    state: State<'_, AppState>,
    input: UpdateTaskInput,
) -> Result<UpdatedTask, String> {
    let vault = state.config.vault_path.clone();
    let rel = vault_tasks::find_by_url(&vault, &input.url)
        .ok_or_else(|| format!("no vault task for url: {}", input.url))?;
    if crate::tracker_sync::seams::tracked(&vault, &rel) {
        if input.subtasks.is_some() || input.blocked_by.is_some() {
            return Err("TASK_LINKED: Tracker issue relations and comment checklists are read-only here; edit the issue's fields in Tasks.".into());
        }
        let mut fields = serde_json::Map::new();
        for (key, value) in [
            ("title", input.title.clone()),
            ("status", input.status.clone()),
            ("priority", input.priority.clone()),
            ("due", input.due.clone()),
            ("dueTime", input.due_time.clone()),
            ("recurrence", input.recurrence.clone()),
            ("section", input.section.clone()),
            ("project", input.project.clone()),
            ("description", input.description.clone()),
        ] {
            if let Some(value) = value {
                fields.insert(key.into(), serde_json::json!(value));
            }
        }
        if let Some(value) = &input.labels {
            fields.insert("labels".into(), serde_json::json!(value));
        }
        if let Some(value) = input.duration_min {
            fields.insert("durationMin".into(), serde_json::json!(value));
        }
        let content_hash = crate::tracker_sync::seams::user_edit(
            &state.config,
            rel.clone(),
            input.expected,
            fields,
        )
        .await?;
        return Ok(UpdatedTask {
            url: format!("vault:{rel}"),
            content_hash,
        });
    }
    let content_hash = vault_tasks::apply_update(&vault, &rel, &input)?;
    Ok(UpdatedTask {
        url: format!("vault:{rel}"),
        content_hash,
    })
}

/// Move a task to the vault trash and return its raw markdown for Undo.
/// Provenance from a Notion import never causes a remote deletion.
#[tauri::command]
pub async fn delete_task(state: State<'_, AppState>, url: String) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    let rel = vault_tasks::find_by_url(&vault, &url)
        .ok_or_else(|| format!("no vault task for url: {url}"))?;
    let full = vault::safe_join(&vault, &rel)?;
    let content = std::fs::read_to_string(&full).map_err(|e| format!("read: {e}"))?;
    vault_tasks::delete(&vault, &rel)?;
    Ok(content)
}

/// Undo `delete_task` by restoring its original local markdown.
#[tauri::command]
pub async fn restore_task(
    state: State<'_, AppState>,
    url: String,
    content: String,
) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    let rel = url
        .strip_prefix("vault:")
        .ok_or_else(|| format!("not a vault url: {url}"))?;
    vault_tasks::restore(&vault, rel, &content)
}

/// One-shot migration of every task's `priority` to the canonical `P1`..`P4`
/// form. Returns the number of files rewritten. Idempotent.
#[tauri::command]
pub async fn migrate_task_priorities(state: State<'_, AppState>) -> Result<usize, String> {
    vault_tasks::migrate_priorities(&state.config.vault_path)
}

/// Create a canonical project note at `projects/{status}/{title}.md`.
#[tauri::command]
pub async fn create_project(
    state: State<'_, AppState>,
    input: CreateProjectInput,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    vault_projects::create(&vault, vault_projects::frontmatter_for_create(&input)?)
}

// ── AI assist (break down / next steps / rephrase) via one-shot claude ────

/// Strip list markers (`- [ ]`, `-`, `*`, `1.`) from a model's bullet output
/// into clean suggestion lines.
fn parse_assist_lines(s: &str) -> Vec<String> {
    s.lines()
        .filter_map(|l| {
            let mut t = l.trim();
            for p in ["- [ ]", "- [x]", "- [X]", "- ", "* ", "• ", "+ "] {
                if let Some(r) = t.strip_prefix(p) {
                    t = r.trim();
                    break;
                }
            }
            let t = t
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim_start_matches(['.', ')'])
                .trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
        .collect()
}

/// The assist prompt. `alignment` (a `<project-alignment>` block) is appended
/// only to `breakdown` and `next_steps`; rephrasing never needs the goals.
fn assist_prompt(input: &AssistInput, alignment: Option<&str>) -> Result<String, String> {
    let alignment = alignment.unwrap_or("");
    Ok(match input.action.as_str() {
        "breakdown" => format!(
            "Break this task into 3-7 concrete, ordered sub-tasks. Output ONLY a markdown \
             checklist, one `- [ ] step` per line. No preamble, no commentary.\n\nTask: {}\n{}{}",
            input.title, input.description, alignment
        ),
        "next_steps" => format!(
            "Suggest 3-5 concrete next actions to move this work forward. Output ONLY a markdown \
             checklist, one `- [ ] action` per line. No preamble.\n\nTask: {}\n{}{}",
            input.title, input.description, alignment
        ),
        "rephrase" => format!(
            "Rewrite this as a single clear, action-oriented imperative starting with a verb. \
             Output ONLY the rewritten line — no quotes, no trailing punctuation.\n\nText: {}",
            input.title
        ),
        other => return Err(format!("unknown assist action: {other}")),
    })
}

/// The goal-alignment block for a planning assist on a linked task: only
/// `breakdown` and `next_steps`, only with a non-blank stored `project_url`,
/// and only when that project has at least one parsed goal (Paperclip PR 10).
fn assist_alignment(vault: &Path, input: &AssistInput) -> Option<String> {
    if !matches!(input.action.as_str(), "breakdown" | "next_steps") {
        return None;
    }
    let url = input
        .project_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())?;
    crate::goal_context::task_assist_block(vault, url)
}

async fn run_assist(input: AssistInput, alignment: Option<String>) -> Result<AssistResult, String> {
    let prompt = assist_prompt(&input, alignment.as_deref())?;
    // Pure text transform: deny all tools and run from a throwaway cwd so a
    // prompt-injected title/description can't reach the vault (or inherit
    // vault-scoped MCP servers) even though one-shots run under bypassPermissions.
    let cwd = std::env::temp_dir();
    let out = crate::claude_runner::run_oneshot(
        &prompt,
        crate::claude_runner::RunOpts {
            cwd: &cwd,
            timeout: std::time::Duration::from_secs(60),
            allowed_tools: &[],
            disallowed_tools: crate::claude_runner::NO_TOOLS,
            model: None,
            debug_log: None,
            meter: crate::usage_ledger::Meter::oneshot("assist"),
            self_hosted: None,
        },
    )
    .await?;
    if input.action == "rephrase" {
        Ok(AssistResult {
            suggestions: vec![],
            text: Some(out.trim().trim_matches('"').to_string()),
        })
    } else {
        Ok(AssistResult {
            suggestions: parse_assist_lines(&out),
            text: None,
        })
    }
}

/// AI assist for a task — break it down, suggest next steps, or rephrase.
/// A linked task's planning assists also see its project's goals.
#[tauri::command]
pub async fn task_assist(
    state: State<'_, AppState>,
    input: AssistInput,
) -> Result<AssistResult, String> {
    let vault = state.config.vault_path.clone();
    let probe = input.clone();
    // Bounded vault reads (the project note, its linked goals and, for a
    // stale path, one bounded projects/ walk), kept off the async runtime.
    let alignment = tauri::async_runtime::spawn_blocking(move || assist_alignment(&vault, &probe))
        .await
        .unwrap_or(None);
    run_assist(input, alignment).await
}

/// AI assist for a project — same actions, project framing supplied by the UI.
/// A project being created has no goals yet, so no alignment block.
#[tauri::command]
pub async fn project_assist(input: AssistInput) -> Result<AssistResult, String> {
    run_assist(input, None).await
}

// ── Saved task filters (persisted to vault/.task-filters.json) ────────────

fn task_filters_path(vault: &std::path::Path) -> PathBuf {
    vault.join(".task-filters.json")
}

/// The saved filters. Only a missing file is empty: an unreadable file is an
/// error and stays put, and a damaged one is moved aside
/// (`.task-filters.json.corrupt-<time>`) before the error, so a save can
/// never replace every saved filter with just the new one (B·F08).
fn read_task_filters(vault: &std::path::Path) -> Result<Vec<TaskFilter>, String> {
    read_task_filters_or(vault, crate::state_file::OnDamage::SetAside)
}

fn read_task_filters_or(
    vault: &std::path::Path,
    on_damage: crate::state_file::OnDamage,
) -> Result<Vec<TaskFilter>, String> {
    let filters = crate::state_file::read_json_or(
        &task_filters_path(vault),
        "Saved task filters",
        on_damage,
    )?;
    Ok(filters.unwrap_or_default())
}

fn write_task_filters(vault: &std::path::Path, filters: &[TaskFilter]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(filters).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&task_filters_path(vault), json.as_bytes())
        .map_err(|e| format!("write filters: {e}"))
}

fn save_task_filter_in(vault: &std::path::Path, filter: TaskFilter) -> Result<(), String> {
    let mut filters = read_task_filters(vault)?;
    if let Some(existing) = filters.iter_mut().find(|f| f.id == filter.id) {
        *existing = filter;
    } else {
        filters.push(filter);
    }
    write_task_filters(vault, &filters)
}

fn delete_task_filter_in(vault: &std::path::Path, id: &str) -> Result<(), String> {
    let mut filters = read_task_filters(vault)?;
    filters.retain(|f| f.id != id);
    write_task_filters(vault, &filters)
}

#[tauri::command]
pub fn list_task_filters(state: State<'_, AppState>) -> Result<Vec<TaskFilter>, String> {
    read_task_filters(&state.config.vault_path)
}

/// [`list_task_filters`] for the agent action: a damaged file is an error but
/// stays for Tasks to move aside and report (B·F08).
pub(crate) fn list_task_filters_unattended(
    state: State<'_, AppState>,
) -> Result<Vec<TaskFilter>, String> {
    read_task_filters_or(&state.config.vault_path, crate::state_file::OnDamage::Leave)
}

#[tauri::command]
pub fn save_task_filter(state: State<'_, AppState>, filter: TaskFilter) -> Result<(), String> {
    save_task_filter_in(&state.config.vault_path, filter)
}

#[tauri::command]
pub fn delete_task_filter(state: State<'_, AppState>, id: String) -> Result<(), String> {
    delete_task_filter_in(&state.config.vault_path, &id)
}

// ── Chat (spawn `claude` headless and stream events back) ────────────────

/// Validate the conversation cwd before the frontend consumes the composer
/// text or writes an attachment. A returned path means that cwd disappeared;
/// the frontend resets to the vault and requires an explicit resend.
#[tauri::command]
pub fn chat_preflight(
    state: State<'_, AppState>,
    cwd: Option<String>,
) -> Result<Option<String>, String> {
    let working_dir: PathBuf = match cwd {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => state.config.vault_path.clone(),
    };
    chat::preflight_chat_context(working_dir, &state.config.vault_path)
        .map(|path| path.map(|value| value.to_string_lossy().into_owned()))
}

/// Queue a chat turn (Paperclip PR 7b, D3) and return at once with where it
/// stands. The run-queue pump admits it once the vault lease is free and its
/// events (`runStarted`, …, `done`) echo `client_turn_id`. `Err` means
/// nothing was queued: `CHAT_CONTEXT_RECOVERED: {path}` when the working
/// folder vanished after the frontend preflight (the composer keeps the text,
/// resets to the vault and asks for an explicit resend), a queue this process
/// does not own, a lease only a restart can clear, or a turn already queued.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn chat_send(
    app: AppHandle,
    state: State<'_, AppState>,
    prompt: String,
    session_id: Option<String>,
    // Hollow's own session key from the previous turn's `runStarted` event.
    // Absent on a fresh conversation; `chat::accept_turn` mints one.
    session_key: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    permission_mode: Option<String>,
    agent: Option<String>,
    client_turn_id: Option<String>,
    // Track U: this chat's Ultracode switch, set beside its permission mode.
    // Absent (older frontends) is off.
    ultracode: Option<bool>,
    // Canvas C3 wave 2 C4: this chat's own Canvases choice (`off`, `helps`,
    // `most`). Absent follows Settings, Chat, Canvases.
    canvases: Option<String>,
    // Agent effectiveness F4 (K1 review): the earlier conversations this one
    // continues from (Continue from summary, a folder-vanished reset),
    // oldest first. Absent (older frontends) is none.
    continued_from: Option<Vec<String>>,
    // Overnight 9 A1: a request Hollow wrote that the person sent unchanged
    // (`canvas`, `canvasRefresh`, `explain`). It only lowers what the turn
    // settles and teaches; an unknown value is the person's own words.
    canned: Option<String>,
    // Mid-turn steering (2026-10-21): what to do when this conversation's
    // turn is still running: `queue` sends it when that turn ends, `stop`
    // stops it first (Stop and send). Absent: today's send, refused while
    // a turn runs.
    after: Option<String>,
    // Settings PR 7: this chat's own reasoning effort (one of
    // `effort::LEVELS`), queued with the turn and never saved to Settings.
    // Absent (older frontends, a chat on the default) is the app default.
    effort: Option<String>,
) -> Result<crate::types::SendAccepted, String> {
    let effort = chat_effort(effort)?;
    let after = crate::steering::After::parse(after.as_deref())?;
    let canned = crate::task_state::canned_asker(canned.as_deref());
    let input = ChatSendInput {
        prompt,
        session_id,
        session_key,
        cwd,
        model,
        permission_mode,
        agent,
        client_turn_id,
        app_depth: 0,
        ultracode: ultracode.unwrap_or(false),
        canvases,
        continued_from: continued_from.unwrap_or_default(),
        unattended: false,
        // The person typed it: the only send that teaches (fewer learnings).
        // Hollow's own words, sent unchanged, don't.
        typed: canned.is_none(),
        host_asker: None,
        canned,
        worker: false,
    };
    // One conversation from the person's side (2026-11-07): a message for a
    // conversation that continued elsewhere goes to the chain's current one
    // (`chat_chain::route_send`), and is held behind its running turn rather
    // than refused, since the person can't see that turn from here.
    let mut input = input;
    let routed = {
        let vault = state.config.vault_path.clone();
        let key = input.session_key.clone();
        let cwd = PathBuf::from(
            input
                .cwd
                .clone()
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| vault.to_string_lossy().into_owned()),
        );
        let continues = !input.continued_from.is_empty();
        tauri::async_runtime::spawn_blocking(move || {
            crate::chat_chain::route_send(&vault, key.as_deref(), &cwd, 0, continues)
        })
        .await
        .unwrap_or(None)
    };
    // Pointed at the current conversation as admission would point it
    // (`chat_queue::apply_route`), so a hold waits under its key. Queue for
    // after, never Stop and send: the running turn is the current
    // conversation's (a successor's first turn carries its handover task),
    // not one the person was looking at.
    let after = match &routed {
        Some(routed) => {
            let vault = state.config.vault_path.clone();
            let (to, mode) = (routed.clone(), input.permission_mode.clone());
            let settings = tauri::async_runtime::spawn_blocking(move || {
                crate::chat_chain::routed_settings(&vault, &to, mode.as_deref())
            })
            .await
            .map_err(|e| e.to_string())?;
            route_input(&mut input, routed, settings);
            Some(crate::steering::After::Queue)
        }
        None => after,
    };
    let prompt = input.prompt.clone();
    let sent = match after {
        Some(after) => send_or_hold(&app, &state.config, input, after, effort).await,
        None => send_chat_with_effort(Some(&app), &state.config, input, effort, None).await,
    };
    match (sent, routed) {
        (Ok(mut accepted), Some(routed)) => {
            let vault = state.config.vault_path.clone();
            let note = routed.clone();
            let at = chrono::Utc::now().to_rfc3339();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                crate::chat_chain::record_forward(&vault, &note, &prompt, &at)
            })
            .await;
            accepted.forwarded_from = Some(routed.from.unwrap_or_default());
            accepted.forwarded_title = routed.to_title;
            Ok(accepted)
        }
        (sent, _) => sent,
    }
}

/// Queue for after / Stop and send (mid-turn steering): send now when the
/// conversation has no turn queued or running and holds nothing, else hold
/// the send behind what it already holds; held sends go oldest first as
/// each turn ends ([`drain_held`]). Stop and send stops the running turn
/// first. Only the person's own composer sends this (`chat_send` with
/// `after`); agents' sends to a busy chat stay refused.
pub(crate) async fn send_or_hold(
    app: &AppHandle,
    config: &crate::config::Config,
    input: ChatSendInput,
    after: crate::steering::After,
    effort: Option<String>,
) -> Result<crate::types::SendAccepted, String> {
    let Some(key) = input
        .session_key
        .clone()
        .filter(|key| crate::agent_sessions::is_safe_session_key(key))
    else {
        // A new conversation has no turn to wait for.
        return send_chat_with_effort(Some(app), config, input, effort, None).await;
    };
    // AC1: an agent without the queue capability is refused before anything
    // else, Stop and send's cancel included.
    refuse_unqueueable(input.agent.as_deref())?;
    let _order = HELD_ORDER.lock().await;
    if after == crate::steering::After::Stop {
        crate::chat::cancel(app.clone(), config.vault_path.clone(), Some(key.clone())).await?;
    }
    // Nothing held: try it now. Something held: it waits behind that, in
    // the order the person sent them.
    if crate::steering::held_count(&key) == 0 {
        match send_chat_with_effort(Some(app), config, input.clone(), effort.clone(), None).await {
            Err(error) if error.contains(crate::chat::CHAT_ALREADY_QUEUED) => {}
            sent => return sent,
        }
    }
    let id = format!("held-{:016x}", rand::random::<u64>());
    crate::steering::hold(
        &key,
        crate::steering::Held {
            id: id.clone(),
            after,
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            input: crate::steering::HeldInput {
                effort,
                ..held_input(&input)
            },
        },
    )?;
    start_held_sweeper(app.clone());
    Ok(crate::types::SendAccepted {
        session_key: key,
        queue_id: 0,
        queue_position: 0,
        phase: crate::agent_sessions::SessionPhase::Queued
            .as_str()
            .to_string(),
        waiting_for: Some("the running turn to end".to_string()),
        held_id: Some(id),
        forwarded_from: None,
        forwarded_title: None,
    })
}

/// A send that is never held (a paired phone's, `phone_access`): sent now
/// when the conversation has no turn queued or running and holds nothing
/// for after one, else refused as already queued. A held send waits where
/// only the window that sent it shows it, so a phone's would start unseen
/// when the person stops the turn at the Mac. In [`HELD_ORDER`], so it never
/// overtakes a held send the window is sending.
pub(crate) async fn send_unless_busy(
    app: &Core,
    config: &crate::config::Config,
    input: ChatSendInput,
    gate: impl FnOnce() -> Result<(), String>,
) -> Result<crate::types::SendAccepted, String> {
    let _order = held_order(gate).await?;
    if holds_a_send(&input) {
        return Err(crate::chat::CHAT_ALREADY_QUEUED.to_string());
    }
    send_chat_on(Some(app), config, input, None, None).await
}

/// Whether `input`'s conversation holds a send for after its turn, which a
/// send that is never held must not overtake ([`send_unless_busy`]).
pub(crate) fn holds_a_send(input: &ChatSendInput) -> bool {
    input
        .session_key
        .as_deref()
        .is_some_and(|key| crate::steering::held_count(key) > 0)
}

/// A person's send pointed at the chain's current conversation, as
/// `chat_queue::apply_route` points one at admission (which then finds it
/// current): its key, native session and folder, its own agent, model and
/// mode (`settings`, `chat_chain::routed_settings`), with the note of where
/// it was typed.
fn route_input(
    input: &mut ChatSendInput,
    routed: &crate::chat_chain::Routed,
    settings: (Option<String>, Option<String>, String),
) {
    let (agent, model, mode) = settings;
    input.session_key = Some(routed.to.clone());
    input.session_id = routed.session_id.clone();
    if !routed.cwd.is_empty() {
        input.cwd = Some(routed.cwd.clone());
    }
    if let Some(agent) = agent {
        input.agent = Some(agent);
        input.model = model;
    }
    input.permission_mode = Some(mode);
    input.continued_from.clear();
    input.prompt = crate::chat_chain::with_forward_block(
        &input.prompt,
        routed,
        crate::chat_chain::Sender::Person,
    );
}

/// One order for holding and sending held sends, process-wide: a new hold
/// never overtakes an older one being sent, and two drains never race.
static HELD_ORDER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Take [`HELD_ORDER`] for a paired phone's send, then run its commit-point
/// `gate` (still paired?): the wait for the order can be long, so a phone
/// revoked during it must not queue anything (R1). The gate's refusal is
/// returned as is and the order is released.
pub(crate) async fn held_order(
    gate: impl FnOnce() -> Result<(), String>,
) -> Result<tokio::sync::MutexGuard<'static, ()>, String> {
    let order = HELD_ORDER.lock().await;
    gate()?;
    Ok(order)
}

/// Tests: the order itself, so a test can hold it while a phone's send waits.
#[cfg(test)]
pub(crate) fn held_order_for_test() -> &'static tokio::sync::Mutex<()> {
    &HELD_ORDER
}

/// AC1: `Err` with the capability's reason when the agent a send names (the
/// default agent when it names none) has no `queue` capability. It has no
/// side effect and runs first, so nothing is stopped, stored or saved for a
/// send that can't be held. An agent that can't be resolved is left to the send's own error.
fn refuse_unqueueable(agent: Option<&str>) -> Result<(), String> {
    let kind = agent
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(crate::agent::AgentKind::from_opt)
        .or_else(|| {
            crate::agent_settings::resolve(agent, None)
                .ok()
                .map(|(kind, _)| kind)
        });
    match crate::steering::queue_refusal(kind) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// The person's send as it is held.
fn held_input(input: &ChatSendInput) -> crate::steering::HeldInput {
    crate::steering::HeldInput {
        prompt: input.prompt.clone(),
        session_id: input.session_id.clone(),
        session_key: input.session_key.clone(),
        cwd: input.cwd.clone(),
        model: input.model.clone(),
        permission_mode: input.permission_mode.clone(),
        agent: input.agent.clone(),
        client_turn_id: input.client_turn_id.clone(),
        ultracode: input.ultracode,
        canvases: input.canvases.clone(),
        continued_from: input.continued_from.clone(),
        canned: input.canned.map(str::to_string),
        typed: input.typed,
        app_depth: input.app_depth,
        unattended: input.unattended,
        ..Default::default()
    }
}

/// L1: hold an agent's message (`send_to_session`) for conversation `key`,
/// whose turn is queued or running, behind what it already holds, as the
/// person's Queue for after is held ([`send_or_hold`]): it is sent when that
/// turn ends ([`drain_held`]), at its own depth and `effort`, and the asking
/// run's `delegation` record (run id, record) is stored then, so its end is
/// reported to the asker. Returns the held id and its place (1 = next).
/// `mode_cap` is the sender's cap on the conversation's own mode
/// (`app_sessions::SendMode::cap`), applied again when it goes.
/// `app` is `None` only in tests, which drain by hand.
pub(crate) async fn hold_agent_send(
    app: Option<&AppHandle>,
    key: &str,
    input: &ChatSendInput,
    effort: Option<String>,
    delegation: Option<(String, crate::run_manifest::DelegationRecord)>,
    mode_cap: Option<String>,
    mode_lift: Option<String>,
) -> Result<(String, usize), String> {
    refuse_unqueueable(input.agent.as_deref())?;
    let _order = HELD_ORDER.lock().await;
    let mut held = held_input(input);
    held.effort = effort;
    held.mode_cap = mode_cap;
    held.mode_lift = mode_lift;
    if let Some((run, record)) = delegation {
        held.delegation_run = Some(run);
        held.delegation_record = serde_json::to_string(&record).ok();
    }
    let id = format!("held-{:016x}", rand::random::<u64>());
    crate::steering::hold(
        key,
        crate::steering::Held {
            id: id.clone(),
            after: crate::steering::After::Queue,
            at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            input: held,
        },
    )?;
    let place = crate::steering::held_count(key);
    if let Some(app) = app {
        start_held_sweeper(app.clone());
    }
    Ok((id, place))
}

/// An agent's held send fails to queue at most this many times (L1 review).
const MAX_AGENT_HELD_ATTEMPTS: u32 = 3;

/// An agent's held send as it goes (L1 review): the conversation's native
/// session as it is now (the turn it waited for may have started a new one),
/// and the conversation's own mode as it is now, lowered only by the
/// sender's cap (`HeldInput::mode_cap`: the lower mode it asked for, or plan
/// for a sender in plan). Until 2026-11-12 it was the weaker of the mode it
/// was held with and the conversation's last, so a send held while the
/// person raised the mode ran in the old one.
fn refreshed_agent_input(
    vault: &std::path::Path,
    held: &crate::steering::HeldInput,
) -> ChatSendInput {
    let mut input = send_input(held);
    let Some(key) = held.session_key.as_deref() else {
        return input;
    };
    if let Ok(Some(summary)) = crate::session_store::session_by_key(vault, key) {
        input.session_id = summary.harness_session_id;
    }
    if let Ok(Some(last)) = crate::session_store::last_permission(vault, key) {
        let own = crate::app_mcp::own_mode(Some(last.as_str()));
        // A send lifted to its sender's mode keeps it while that is stronger
        // than the conversation's own now; the cap still lowers it.
        let base = match held
            .mode_lift
            .as_deref()
            .map(crate::app_mcp::normalize_perm)
        {
            Some(lift) if crate::app_mcp::perm_rank(lift) > crate::app_mcp::perm_rank(own) => lift,
            _ => own,
        };
        let mode = crate::app_mcp::lowered(base, held.mode_cap.as_deref());
        input.permission_mode = Some(mode.to_string());
    }
    input
}

/// After an agent's held send was queued as row `queue_id` of conversation
/// `session_key`: store the asking run's delegation record with the row, so
/// the asker hears when it ends, and keep an unattended send's limits.
fn settle_agent_held(
    vault: &std::path::Path,
    held: &crate::steering::HeldInput,
    session_key: &str,
    queue_id: i64,
) {
    if let (Some(run), Some(json)) = (&held.delegation_run, &held.delegation_record) {
        match serde_json::from_str::<crate::run_manifest::DelegationRecord>(json) {
            Ok(mut record) => {
                record.child_session = Some(session_key.to_string());
                record.child_queue_id = Some(queue_id);
                crate::delegation::store(vault, run, &record);
            }
            Err(error) => eprintln!("[held] a held send's delegation record didn't parse: {error}"),
        }
    }
    if held.unattended {
        crate::main_agent::unattended::register(queue_id);
    }
}

/// A held send as the person's own send again (as [`chat_send`] builds it).
fn send_input(held: &crate::steering::HeldInput) -> ChatSendInput {
    let canned = crate::task_state::canned_asker(held.canned.as_deref());
    ChatSendInput {
        prompt: held.prompt.clone(),
        session_id: held.session_id.clone(),
        session_key: held.session_key.clone(),
        cwd: held.cwd.clone(),
        model: held.model.clone(),
        permission_mode: held.permission_mode.clone(),
        agent: held.agent.clone(),
        client_turn_id: held.client_turn_id.clone(),
        // The person's own (0), or an agent's held message's (L1).
        app_depth: held.app_depth,
        ultracode: held.ultracode,
        canvases: held.canvases.clone(),
        continued_from: held.continued_from.clone(),
        unattended: held.unattended,
        // As `chat_send` set it when the person sent it.
        typed: held.typed,
        host_asker: None,
        canned,
        worker: false,
    }
}

/// Whether `key` has a chat turn waiting or running.
fn conversation_busy(vault: &std::path::Path, key: &str) -> bool {
    let active = crate::chat_rows::active_chat_rows(vault);
    crate::run_queue::live_rows(vault)
        .unwrap_or_default()
        .iter()
        .any(|row| {
            row.source_id.as_deref() == Some(key)
                && (row.status == crate::run_queue::RowStatus::Queued || active.contains(&row.id))
        })
}

/// Send `key`'s oldest held send when the conversation is idle: as its turn
/// ends (the pump, after `done`) and from the sweeper. One goes per turn;
/// the next waits for that turn. A send that can't go (its folder vanished)
/// is reported and the next one tries. The send stays saved until its turn
/// is queued (2026-11-13: `steering::take_held` saves nothing,
/// `settle_held` removes it after the commit), and its row carries
/// `steering::queue_key`, so one found already queued after a quit between
/// the two is settled, not sent again. Each outcome is a `chat-event`:
/// `heldSent {heldId, after, queueId, clientTurnId}` or `heldFailed
/// {heldId, error, prompt}` (the composer puts the words back), routed to
/// the conversation by `conversationKey`.
pub(crate) async fn drain_held(app: &Core, key: &str) {
    let _order = HELD_ORDER.lock().await;
    let config = app.state::<AppState>().config.clone();
    // R8b: failed sends the window never acknowledged (a quit after the
    // event) are reported again, once per launch.
    for failed in crate::steering::unreported_failures(key) {
        crate::chat::emit_event(
            app,
            crate::chat_event::build::held_failed(
                &failed.id,
                &failed.input.failed.clone().unwrap_or_default(),
                &failed.input.prompt,
                key,
                key,
            ),
        );
    }
    loop {
        if crate::steering::held_count(key) == 0 || conversation_busy(&config.vault_path, key) {
            return;
        }
        let Some(held) = crate::steering::take_held(key) else {
            return;
        };
        let row_key = crate::steering::queue_key(&held.id);
        match held_row_lookup(&config.vault_path, &row_key) {
            // Queued before a quit that came before its removal was saved:
            // that row is the send (a restart recovers it like any queued
            // turn).
            HeldLookup::Queued(queue_id) => {
                eprintln!(
                    "[held] {} to {key} was already queued as row {queue_id}",
                    held.id
                );
                crate::steering::settle_held(key, &held.id);
                continue;
            }
            HeldLookup::Send => {}
            // Unknown: it may have gone already. Held for the sweeper's next
            // try, never sent blind (review of ae6e2ac84).
            HeldLookup::Retry(error) => {
                eprintln!("[held] couldn't check {} to {key}: {error}", held.id);
                crate::steering::put_back(key, held);
                return;
            }
        }
        // An agent's held message (L1) goes at its own depth and effort, and
        // is the window's business only as an ordinary queued turn: no
        // held-send event (the composer would treat it as the person's).
        if held.input.app_depth > 0 {
            let input = refreshed_agent_input(&config.vault_path, &held.input);
            let effort = held.input.effort.clone();
            match send_chat_keyed(Some(app), &config, input, effort, None, Some(row_key)).await {
                Ok(accepted) => {
                    crate::steering::settle_held(key, &held.id);
                    settle_agent_held(
                        &config.vault_path,
                        &held.input,
                        &accepted.session_key,
                        accepted.queue_id,
                    );
                    return;
                }
                Err(error) if error.contains(crate::chat::CHAT_ALREADY_QUEUED) => {
                    crate::steering::put_back(key, held);
                    return;
                }
                Err(error) => {
                    // Tried again on the next drain, dropped after the third
                    // failure (a vanished folder or conversation never heals).
                    let mut held = held;
                    held.input.attempts += 1;
                    if held.input.attempts < MAX_AGENT_HELD_ATTEMPTS {
                        eprintln!(
                            "[held] an agent's held message to {key} failed (try {}): {error}",
                            held.input.attempts
                        );
                        crate::steering::put_back(key, held);
                        return;
                    }
                    eprintln!("[held] an agent's held message to {key} was dropped after {MAX_AGENT_HELD_ATTEMPTS} failures: {error}");
                    crate::steering::settle_held(key, &held.id);
                    continue;
                }
            }
        }
        let event = match send_chat_keyed(
            Some(app),
            &config,
            send_input(&held.input),
            held.input.effort.clone(),
            None,
            Some(row_key),
        )
        .await
        {
            Ok(accepted) => crate::chat_event::build::held_sent(
                &held.id,
                held.after,
                &accepted.session_key,
                accepted.queue_id,
                held.input.client_turn_id.clone(),
                key,
            ),
            Err(error) if error.contains(crate::chat::CHAT_ALREADY_QUEUED) => {
                crate::steering::put_back(key, held);
                return;
            }
            Err(error) => crate::chat_event::build::held_failed(
                &held.id,
                &error,
                &held.input.prompt,
                key,
                key,
            ),
        };
        let sent = event["type"] == "heldSent";
        report_then_settle(key, &held.id, event, |event| {
            crate::chat::emit_event(app, event)
        });
        if sent {
            return;
        }
    }
}

/// What the drain knows of a held send's turn before sending it.
#[derive(Debug, PartialEq, Eq)]
enum HeldLookup {
    /// Its row exists: it went before a quit; settle it, never send it.
    Queued(i64),
    /// No row under its key: send it.
    Send,
    /// The run queue couldn't be read: put it back and try later.
    Retry(String),
}

/// Whether held send `row_key`'s turn is already queued
/// (`run_queue::row_with_key`). A failed read is never "no row": the enqueue
/// dedupes only live rows, so sending then could deliver a finished one twice.
fn held_row_lookup(vault: &std::path::Path, row_key: &str) -> HeldLookup {
    match crate::run_queue::row_with_key(vault, row_key) {
        Ok(Some(queue_id)) => HeldLookup::Queued(queue_id),
        Ok(None) => HeldLookup::Send,
        Err(error) => HeldLookup::Retry(error),
    }
}

/// Report a drained held send's outcome (`heldSent`, or `heldFailed`, which
/// gives the person's words back to the composer), and only then settle it
/// in the held store (review of ae6e2ac84): a quit before the report
/// leaves it held, and the next launch drains it again (at least once,
/// never lost). A sent one is removed. A failed one is kept, marked failed
/// and never sent again, until the window acknowledges it holds the words
/// (`chat_ack_held_failed`, R8b), and is reported again at the next launch if
/// that never came.
fn report_then_settle(
    key: &str,
    id: &str,
    event: serde_json::Value,
    emit: impl FnOnce(serde_json::Value),
) {
    emit(event.clone());
    if event["type"] == "heldFailed" {
        let error = event["error"].as_str().unwrap_or_default();
        crate::steering::mark_failed(key, id, error);
    } else {
        crate::steering::settle_held(key, id);
    }
}

/// Repo ideas 2026-10-27 #8: after conversation `key`'s turn ended, send
/// the continuation its end planned (`agent_out::continuation`), if any, as
/// the conversation's next message on the agent the person chose for when
/// one runs out, and tell the window (`agentContinued`), which shows it like
/// a sent message. In [`HELD_ORDER`], like a hold (review of #1065): a Queue
/// for after or Stop and send the person started first is held (or sent)
/// before this looks, and wins. Never after a Stop of that conversation
/// since its turn started; one landing while it sends withdraws it. A
/// failure sends nothing more: the turn that ran out already says when it
/// can go on.
pub(crate) async fn continue_after(app: &Core, key: &str) {
    let _order = HELD_ORDER.lock().await;
    let Some(next) = crate::agent_out::take(key) else {
        return;
    };
    let stopped = || crate::agent_out::stops(key) != next.stops;
    if crate::steering::held_count(key) > 0 || stopped() {
        return;
    }
    let config = app.state::<AppState>().config.clone();
    let client_turn_id = format!("out-{:016x}", rand::random::<u64>());
    let prompt = crate::agent_out::continue_prompt(&next);
    crate::agent_out::mark_sent(&client_turn_id);
    let input = ChatSendInput {
        prompt: prompt.clone(),
        session_id: None,
        session_key: Some(next.session_key.clone()),
        cwd: Some(next.cwd.clone()),
        model: None,
        permission_mode: next.permission_mode.clone(),
        agent: Some(next.to.as_str().to_string()),
        client_turn_id: Some(client_turn_id.clone()),
        app_depth: 0,
        ultracode: false,
        canvases: next.canvases.clone(),
        continued_from: Vec::new(),
        unattended: false,
        typed: false,
        host_asker: None,
        canned: None,
        worker: false,
    };
    match send_chat_on(Some(app), &config, input, None, None).await {
        Ok(accepted) if stopped() => {
            // A Stop landed while it was being queued: take back this
            // continuation alone, by its own queue row, never a newer turn
            // of the conversation (R2 of #1065).
            withdraw_continuation(app, &config.vault_path, accepted.queue_id).await;
        }
        Ok(accepted) => {
            crate::chat::emit_event(
                app,
                crate::chat_event::build::agent_continued(
                    next.from.label(),
                    next.to.label(),
                    &prompt,
                    &accepted.session_key,
                    accepted.queue_id,
                    &client_turn_id,
                    &next.session_key,
                ),
            );
        }
        Err(error) => {
            crate::agent_out::was_continuation(&client_turn_id);
            eprintln!(
                "[agent out] continuing on {} failed: {error}",
                next.to.label()
            );
        }
    }
}

/// Take back one continuation by its queue row (`run_queue_api::cancel_row`):
/// queued, it leaves the queue with its `cancelled{queued}` and `done`;
/// claimed, the pump ends it before it starts; running, its own turn is
/// stopped, checked by its row under the stop slot's lock, so a later turn
/// that took the slot meanwhile is never touched; already ended, nothing.
async fn withdraw_continuation(app: &Core, vault: &std::path::Path, id: i64) {
    let vault = vault.to_path_buf();
    let cancelled = tauri::async_runtime::spawn_blocking(move || {
        run_queue_api::cancel_row(
            &vault,
            id,
            chrono::Utc::now(),
            &|_: &str| {},
            &|key: &str| crate::chat::cancel_running_row(key, id),
        )
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|done| done);
    match cancelled {
        Ok((outcome, item, events)) => {
            for event in events {
                crate::chat::emit_event(app, event);
            }
            if outcome != crate::run_queue::CancelOutcome::AlreadyFinished {
                run_queue_api::emit_queue_event(app, "cancelled", item);
            }
            crate::run_queue::wake();
        }
        Err(error) => eprintln!("[agent out] withdrawing a stopped continuation failed: {error}"),
    }
}

/// While any send is held, look every few seconds for a conversation that is
/// idle with sends still held: a turn that ended on a path the pump's
/// end-of-turn send doesn't cover (withdrawn or never started), a send put
/// back after a lost race, or sends held before a restart. One sweeper at a
/// time; it ends when nothing is held.
pub(crate) fn start_held_sweeper(app: impl Into<Core>) {
    let app = app.into();
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            crate::steering::retry_save();
            let keys = crate::steering::held_conversations();
            if keys.is_empty() {
                RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
                // A hold that landed after the check starts a new sweeper.
                if crate::steering::held_conversations().is_empty()
                    || RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    return;
                }
                continue;
            }
            for key in keys {
                drain_held(&app, &key).await;
            }
        }
    });
}

/// Steer now (mid-turn steering, 2026-10-21): give the running turn of
/// `session_key` the person's `text`, taken at its next step. Fails with a
/// `STEER_*` code (`crate::steering`): no turn running, an agent with no
/// true steer, or a turn whose answer already ended (the composer then
/// sends it as Queue for after). The steer's event goes to the chat.
#[tauri::command]
pub fn chat_steer(
    app: AppHandle,
    session_key: String,
    text: String,
    client_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let vault = tauri::Manager::state::<AppState>(&app)
        .config
        .vault_path
        .clone();
    steer_turn(
        Some(&app),
        &vault,
        &session_key,
        &text,
        client_id,
        crate::steering::SteerBy::Person,
    )
}

/// [`chat_steer`]'s body, also the COO's `sessions.steer_session` (L13),
/// which first checks the conversation is one the calling agent started
/// and passes `by` as itself (stored `by: agent` with its run, never the
/// person's). `app` emits the steer to the chat; `None` only in tests.
pub(crate) fn steer_turn(
    app: Option<&AppHandle>,
    vault: &Path,
    session_key: &str,
    text: &str,
    client_id: Option<String>,
    by: crate::steering::SteerBy<'_>,
) -> Result<serde_json::Value, String> {
    // A conversation that continued elsewhere takes no steer: its turn is
    // ending (a handover's last step). The composer then sends the words as
    // the next message, which goes to the chain's current conversation.
    if crate::chat_chain::current_of(vault, session_key).is_some() {
        return Err(crate::steering::STEER_ENDING.to_string());
    }
    let id = client_id
        .filter(|id| crate::steering::is_safe_id(id))
        .unwrap_or_else(|| format!("steer-{:016x}", rand::random::<u64>()));
    let event = crate::chat::accept_steer_by(session_key, text, &id, chrono::Utc::now(), by)
        .map_err(str::to_string)?;
    if let Some(app) = app {
        crate::chat::emit_event(app, event.clone());
    }
    Ok(event)
}

/// What Steer now does for each agent (`steering::support_table`).
#[tauri::command]
pub async fn chat_steer_support() -> Vec<crate::steering::SteerSupportInfo> {
    crate::steering::support_table().await
}

/// Cancel a held send (its chip's Cancel). True when it was still held.
#[tauri::command]
pub fn chat_cancel_held(session_key: String, held_id: String) -> Result<bool, String> {
    crate::steering::cancel_held(&session_key, &held_id)
}

/// The window now holds a failed held send's words (its `heldFailed` event):
/// remove Rust's saved copy (R8b). True when one was removed.
#[tauri::command]
pub fn chat_ack_held_failed(session_key: String, held_id: String) -> Result<bool, String> {
    crate::steering::ack_failed(&session_key, &held_id)
}

/// One chat send: the `chat_send` arguments, plus how many `hollow_app`
/// session starts deep the turn runs (0 for the user's own send; see
/// `app_sessions`).
#[derive(Debug, Clone, Default)]
pub(crate) struct ChatSendInput {
    pub prompt: String,
    pub session_id: Option<String>,
    pub session_key: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub agent: Option<String>,
    pub client_turn_id: Option<String>,
    pub app_depth: u32,
    /// Track U: run this turn at ultracode (`effort::ULTRACODE`).
    pub ultracode: bool,
    /// Canvas C3 wave 2 C4: the chat's own Canvases choice, or `None` to
    /// follow the setting.
    pub canvases: Option<String>,
    /// The earlier conversations this one continues from
    /// (`chat::ChatSendRequest::continued_from`); the caller's key for a
    /// handover's successor, empty for an agent's start or send.
    pub continued_from: Vec<String>,
    /// [`chat::ChatSendRequest::unattended`]: only the night queue sets it.
    pub unattended: bool,
    /// [`chat::ChatSendRequest::typed`]: only [`chat_send`] sets it.
    pub typed: bool,
    /// [`chat::ChatSendRequest::host_asker`]: only a turn Hollow queues
    /// itself sets it (`main_agent::queue_wake_as`, `wake_me::run_due`).
    pub host_asker: Option<&'static str>,
    /// [`chat::ChatSendRequest::canned`]: only [`chat_send`] sets it.
    pub canned: Option<&'static str>,
    /// [`chat::ChatSendRequest::worker`]: only a worker batch
    /// (`app_sessions::queue_turn` for `project_workers`) sets it.
    pub worker: bool,
}

/// The chat's own effort from the composer: a level of `effort::LEVELS`,
/// else `None` (the app default). Anything else is refused, like a
/// started session's.
pub(crate) fn chat_effort(effort: Option<String>) -> Result<Option<String>, String> {
    match effort.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(level) if crate::effort::LEVELS.contains(&level) => Ok(Some(level.to_string())),
        Some(_) => Err("Unknown reasoning effort.".into()),
    }
}

/// Every chat's own reasoning effort, `[sessionKey, level]`: the window reads
/// it once at start into its cache (the store is Rust's, so runs Hollow
/// starts itself resolve it too).
#[tauri::command]
pub async fn chat_efforts_list(
    state: State<'_, AppState>,
) -> Result<Vec<(String, String)>, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || crate::session_store::chat_efforts(&vault))
        .await
        .map_err(|e| e.to_string())?
}

/// Set (a level of `effort::LEVELS`) or clear (`None`: Default, or a reset)
/// one conversation's own effort. Never touches the app default.
#[tauri::command]
pub async fn chat_effort_set(
    state: State<'_, AppState>,
    session_key: String,
    effort: Option<String>,
) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::session_store::set_chat_effort(&vault, &session_key, effort.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The one-time move of the webview's old per-chat levels (localStorage
/// `hollow.effort.<key>`) into the store: only chats without one yet. Returns
/// how many were kept.
#[tauri::command]
pub async fn chat_efforts_import(
    state: State<'_, AppState>,
    entries: Vec<(String, String)>,
) -> Result<usize, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::session_store::import_chat_efforts(&vault, &entries)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The body of [`chat_send`], shared with the Hollow app tools' session
/// starters (PR A2). `app` is only for the `enqueued` event; `None` (tests)
/// skips it.
pub(crate) async fn send_chat(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
    input: ChatSendInput,
) -> Result<crate::types::SendAccepted, String> {
    send_chat_with_effort(app, config, input, None, None).await
}

/// [`send_chat`] for an agent's start or send (L1): `effort` is the turn's
/// own reasoning effort (`None`: the agent's saved one), queued with the
/// turn, and `parent` is the conversation whose subagent this turn's new
/// conversation is (`chat::ChatTurnPayload::parent`; `None` for a message
/// to a conversation that exists).
pub(crate) async fn send_agent_chat(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
    input: ChatSendInput,
    effort: Option<String>,
    parent: Option<String>,
) -> Result<crate::types::SendAccepted, String> {
    send_chat_with_effort(app, config, input, effort, parent).await
}

/// The phone's explicit settings belong to this turn. The held-send guard
/// remains the composer's own guard; no phone send is held.
pub(crate) async fn send_phone_chat(
    app: &Core,
    config: &crate::config::Config,
    input: ChatSendInput,
    effort: Option<String>,
    existing: bool,
    gate: impl FnOnce() -> Result<(), String>,
) -> Result<crate::types::SendAccepted, String> {
    let _order = held_order(gate).await?;
    if existing && holds_a_send(&input) {
        return Err(crate::chat::CHAT_ALREADY_QUEUED.to_string());
    }
    send_chat_on(Some(app), config, input, effort, None).await
}

pub(crate) async fn send_chat_with_effort(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
    input: ChatSendInput,
    effort: Option<String>,
    parent: Option<String>,
) -> Result<crate::types::SendAccepted, String> {
    let app = app.map(Core::from);
    send_chat_keyed(app.as_ref(), config, input, effort, parent, None).await
}

/// [`send_chat_with_effort`] for a core service that holds a [`Core`] (M1
/// PR 1: the COO, wake-ups, pressure tests, the phone API, held sends).
pub(crate) async fn send_chat_on(
    app: Option<&Core>,
    config: &crate::config::Config,
    input: ChatSendInput,
    effort: Option<String>,
    parent: Option<String>,
) -> Result<crate::types::SendAccepted, String> {
    send_chat_keyed(app, config, input, effort, parent, None).await
}

/// [`send_chat_with_effort`] with the row's run-queue key (a held send's,
/// `steering::queue_key`; see [`drain_held`]).
async fn send_chat_keyed(
    app: Option<&Core>,
    config: &crate::config::Config,
    input: ChatSendInput,
    effort: Option<String>,
    parent: Option<String>,
    row_key: Option<String>,
) -> Result<crate::types::SendAccepted, String> {
    let ChatSendInput {
        prompt,
        session_id,
        session_key,
        cwd,
        model,
        permission_mode,
        agent,
        client_turn_id,
        app_depth,
        ultracode,
        canvases,
        continued_from,
        unattended,
        typed,
        host_asker,
        canned,
        worker,
    } = input;
    // A send from the person's own composer: depth 0, attended, and not a
    // host turn (a Main-agent wake must not cancel the thread's wake_me).
    let person_sent = app_depth == 0 && !unattended && host_asker.is_none();
    let working_dir: PathBuf = match cwd {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => config.vault_path.clone(),
    };
    // Resolve the app default and provider-specific model in one place.
    let (agent, model) = crate::agent_settings::resolve(agent.as_deref(), model.as_deref())?;
    // Only the process that owns this vault's run queue can run the turn.
    crate::run_pump::accepting()?;
    // The vault path rides along (independent of cwd) so the environment
    // brief can always name the vault root, even for a /cwd'd session.
    let vault = config.vault_path.clone();
    let request = chat::ChatSendRequest {
        prompt,
        session_id,
        session_key,
        cwd: working_dir,
        model,
        permission_mode,
        agent,
        client_turn_id,
        app_depth,
        ultracode,
        canvases,
        continued_from,
        unattended,
        typed,
        host_asker,
        canned,
        worker,
    };
    let (accepted, item) = tauri::async_runtime::spawn_blocking(move || {
        // No named effort is the ordinary accept. A phone send that names one
        // queues that effort with the turn.
        let mut accepted = if effort.is_none() && parent.is_none() && row_key.is_none() {
            chat::accept_turn(&vault, request, chrono::Utc::now())?
        } else {
            let now = chrono::Utc::now();
            chat::accept_keyed_turn(&vault, request, now, effort, parent, row_key)?
        };
        let live = crate::run_queue::live_rows(&vault).unwrap_or_default();
        let facts = crate::run_pump::QueueFacts::read(&vault, &live);
        let item = crate::run_queue::get(&vault, accepted.queue_id)
            .ok()
            .flatten()
            .map(|row| {
                crate::run_pump::item_with_reason(
                    &vault,
                    &row,
                    Some(accepted.position),
                    &facts,
                    chrono::Utc::now(),
                )
            });
        // No lease holder named: say what else holds the turn back
        // (overnight A2), so a queued turn never just reads "Queued".
        if accepted.waiting_for.is_none() {
            accepted.waiting_for = item.as_ref().and_then(|i| i.waiting_reason.clone());
        }
        Ok::<_, String>((accepted, item))
    })
    .await
    .map_err(|e| e.to_string())??;
    // Emitted and woken only once the row is committed, with its position
    // computed first, so the reply and a fast `runStarted` agree.
    if let (Some(app), Some(item)) = (app, item) {
        crate::run_pump::emit(app, "enqueued", item);
    }
    // The person's own message supersedes a wake-up an agent asked for in
    // this conversation (overnight C3): it would otherwise continue an old
    // direction later. One already due waits for the turn instead (PER-12).
    if person_sent {
        crate::wake_me::cancel_session_on_send(
            &config.vault_path,
            &accepted.session_key,
            chrono::Utc::now(),
        );
    }
    crate::run_queue::wake();
    Ok(accepted.into_wire())
}

/// Which agent CLIs (claude / codex / antigravity) are installed on this
/// machine. Drives the chat agent picker — only installed agents are offered.
#[tauri::command]
pub fn agent_available() -> Vec<String> {
    crate::agent::available()
        .into_iter()
        .map(|k| k.as_str().to_string())
        .collect()
}

#[tauri::command]
pub fn agent_descriptors() -> Vec<crate::agent::AgentDescriptor> {
    crate::agent::descriptors()
}

/// Whether the installed Codex runs Hollow's tool-free helpers (agent
/// effectiveness F9, wave 4 · V1): its version, asked once per binary, and
/// whether Hollow verified that version (`codex_probe`). Settings, Models
/// shows the sentence.
#[tauri::command]
pub async fn codex_helper_status() -> crate::codex_probe::CodexHelperStatus {
    crate::codex_probe::installed_status().await
}

/// Model choices for an agent's picker (suggestions; custom IDs stay valid).
/// Cached for a few minutes; `refresh` asks the agent again.
#[tauri::command]
pub async fn agent_models(
    agent: String,
    refresh: Option<bool>,
) -> crate::agent_models::ModelCatalog {
    crate::agent_models::catalog(&agent, refresh.unwrap_or(false)).await
}

/// Settings, Accounts, Agent updates: each agent CLI's installed
/// and latest version, install method and Updates choice. `refresh` checks
/// for newer versions first.
#[tauri::command]
pub async fn agent_cli_status(refresh: Option<bool>) -> Vec<crate::agent_cli::AgentCliStatus> {
    crate::agent_cli::status(refresh.unwrap_or(false)).await
}

/// Update one agent's CLI with its install method's own updater. While a
/// run uses it the update waits for it; `now` updates beside running chats
/// where that is safe (Claude Code's native installer).
#[tauri::command]
pub async fn update_agent_cli(
    agent: String,
    now: Option<bool>,
) -> Result<crate::agent_cli::AgentCliStatus, String> {
    crate::agent_cli::update(&agent, now.unwrap_or(false)).await
}

/// Reinstall the version before Hollow's last update, where the install
/// method can (Claude Code's installer, npm, Grok).
#[tauri::command]
pub async fn restore_agent_cli(agent: String) -> Result<crate::agent_cli::AgentCliStatus, String> {
    crate::agent_cli::restore(&agent).await
}

/// Cancel an update that is waiting for runs to finish.
#[tauri::command]
pub async fn cancel_agent_cli_update(
    agent: String,
) -> Result<crate::agent_cli::AgentCliStatus, String> {
    crate::agent_cli::cancel_pending(&agent)?;
    let kind = crate::agent::AgentKind::from_opt(&agent).ok_or("Unknown agent.")?;
    Ok(crate::agent_cli::status_one(kind).await)
}

/// An agent's Updates choice: automatic, ask or off.
#[tauri::command]
pub fn set_agent_cli_policy(agent: String, policy: String) -> Result<String, String> {
    crate::agent_cli::set_policy(&agent, &policy)
}

/// Settings, Advanced, Extraction model (overnight A11): the agent
/// extraction runs on now, its chosen model and what it will ask.
#[tauri::command]
pub fn get_extraction_settings() -> Result<crate::extract::ExtractionView, String> {
    crate::extract::extraction_view()
}

/// Turn the model retitle of new conversations on or off (morning N5).
#[tauri::command]
pub fn set_chat_retitle(enabled: bool) -> Result<crate::extract::ExtractionView, String> {
    let saved = crate::extract::set_retitle_chats(enabled)?;
    // Saved: report it even if the current agent can't be resolved now.
    crate::extract::extraction_view().or_else(|_| {
        Ok(crate::extract::ExtractionView {
            agent: String::new(),
            chosen: String::new(),
            asks: None,
            retitle_chats: saved.retitle_chats,
        })
    })
}

/// Set one agent's extraction model (`""` clears it: Haiku 5.5 on Claude,
/// the agent's Settings, Models model elsewhere). `agent` defaults to the
/// one extraction runs on now.
#[tauri::command]
pub fn set_extraction_settings(
    agent: Option<String>,
    model: String,
) -> Result<crate::extract::ExtractionView, String> {
    let agent = match agent {
        Some(a) if !a.trim().is_empty() => a,
        _ => crate::extract::extraction_view()?.agent,
    };
    let saved = crate::extract::set_extraction_model(&agent, &model)?;
    // Saved: report it even if the current agent can't be resolved now.
    crate::extract::extraction_view().or_else(|_| {
        let kind = crate::agent::AgentKind::from_opt(&agent).ok_or("unknown agent")?;
        Ok(crate::extract::ExtractionView {
            agent: kind.as_str().to_string(),
            chosen: saved.models.get(kind.as_str()).cloned().unwrap_or_default(),
            asks: crate::extract::model_for(kind, &saved),
            retitle_chats: saved.retitle_chats,
        })
    })
}

#[tauri::command]
pub fn get_agent_settings() -> Result<crate::agent_settings::AgentSettings, String> {
    crate::agent_settings::get_settings()
}

#[tauri::command]
pub fn set_agent_settings(
    settings: crate::agent_settings::AgentSettings,
) -> Result<crate::agent_settings::AgentSettings, String> {
    crate::agent_settings::set_settings(settings)
}

/// Stop a chat turn: `session_key` names the conversation (Paperclip PR 7b);
/// its queued turn is taken out of the queue, its running turn is stopped.
/// A missing or invalid key stops nothing and is an error (review A·R9).
#[tauri::command]
pub async fn chat_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    session_key: Option<String>,
) -> Result<(), String> {
    chat::cancel(app, state.config.vault_path.clone(), session_key)
        .await
        .map(|_| ())
}

/// P1a: pause a running chat at its next safe point (`chat_pause`): the turn
/// ends keeping its native session, and the chat is recorded as paused. True
/// when the request reached a running turn; false when the chat has no
/// running turn or it is already stopping. A Stop still wins.
#[tauri::command]
pub fn chat_pause(session_key: String) -> Result<bool, String> {
    let key = session_key.trim();
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err("Pause needs the chat's session key. Nothing was paused.".into());
    }
    Ok(chat::request_pause_for(key))
}

/// P1a: resume a paused chat: a normal queued turn on the same conversation
/// with `chat_pause::RESUME_PROMPT`, continuing its stored native session.
/// Refused when the chat isn't paused or already has a turn running or
/// queued. The paused record clears when that turn is admitted.
#[tauri::command]
pub async fn chat_resume_paused(
    app: AppHandle,
    state: State<'_, AppState>,
    session_key: String,
) -> Result<crate::types::SendAccepted, String> {
    resume_paused(Some(&app), &state.config, session_key.trim()).await
}

/// The body of [`chat_resume_paused`], shared with `sessions.resume`.
pub(crate) async fn resume_paused(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
    key: &str,
) -> Result<crate::types::SendAccepted, String> {
    // The claim's owner, sent as the turn's `clientTurnId`.
    let token = crate::chat_pause::new_resume_token();
    // The claim is this resume's; a send that fails gives it back, so the
    // chat stays paused and can be resumed again.
    let sent = resume_paused_as(app, config, key, &token).await;
    if sent.is_err() {
        crate::chat_pause::release_claim(&config.vault_path, key, Some(&token));
    }
    sent
}

/// [`resume_paused`] under `token`'s claim, leaving a failed send's claim
/// for the caller (review fix: the scheduler retries a transient failure,
/// `chat_pause::release_for_retry`).
pub(crate) async fn resume_paused_as(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
    key: &str,
    token: &str,
) -> Result<crate::types::SendAccepted, String> {
    let vault = config.vault_path.clone();
    let lookup = key.to_string();
    let owner = token.to_string();
    let input = tauri::async_runtime::spawn_blocking(move || {
        let busy = chat::turn_live(&vault, &lookup);
        let now = chrono::Utc::now().timestamp();
        crate::chat_pause::resume_input(&vault, &lookup, busy, now, &owner)
    })
    .await
    .map_err(|e| e.to_string())??;
    send_chat(app, config, input).await
}

/// P1b: Pause all. Every chat with a running turn is asked to pause at its
/// next safe point (reason `all`); queued rows are left alone. Returns the
/// keys asked; when the last of them settles, one notification lists what
/// each was doing (or that it finished first).
#[tauri::command]
pub fn chat_pause_all(state: State<'_, AppState>) -> Vec<String> {
    chat::request_pause_all(&state.config.vault_path, None)
}

/// What Resume all did: the chats it queued, those it couldn't (with why)
/// and those it left for their own time (review fix: a resume time still
/// ahead, such as a limit pause waiting for its reset), each by session key.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeAllOutcome {
    pub resumed: Vec<String>,
    pub refused: Vec<(String, String)>,
    pub skipped: Vec<String>,
}

/// P1b: Resume all. Resumes every paused chat (whatever paused it) through
/// [`resume_paused`], so each is claimed once, as by its own Resume.
#[tauri::command]
pub async fn chat_resume_all(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ResumeAllOutcome, String> {
    resume_all_paused(Some(&app), &state.config, None).await
}

/// The body of [`chat_resume_all`], shared with `sessions.resume_all`
/// (`except`: the calling agent's own chat).
pub(crate) async fn resume_all_paused(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
    except: Option<&str>,
) -> Result<ResumeAllOutcome, String> {
    let paused = crate::chat_pause::list(&config.vault_path)?;
    let now = chrono::Utc::now().timestamp();
    let mut outcome = ResumeAllOutcome::default();
    for chat in paused {
        if except == Some(chat.session_key.as_str()) {
            continue;
        }
        if chat.resume_at.is_some_and(|at| at > now) {
            outcome.skipped.push(chat.session_key);
            continue;
        }
        match resume_paused(app, config, &chat.session_key).await {
            Ok(_) => outcome.resumed.push(chat.session_key),
            Err(error) => outcome.refused.push((chat.session_key, error)),
        }
    }
    Ok(outcome)
}

/// P1b, P1c: every paused chat with its note, reason and resume time, newest
/// first, for the window's paused state.
#[tauri::command]
pub fn chat_paused_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::chat_pause::PausedChat>, String> {
    crate::chat_pause::list(&state.config.vault_path)
}

/// The latest resume time accepted: a year ahead.
const RESUME_AT_MAX_AHEAD_SECS: i64 = 366 * 24 * 3600;

/// P1c: set (`at`, unix seconds) or clear (`None`) when paused chat
/// `session_key` resumes on its own. A time already past resumes it at once,
/// through the same claim as Resume (`Ok(None)` then). Refused when the
/// chat isn't paused or the time is more than a year ahead.
#[tauri::command]
pub async fn chat_set_resume_at(
    app: AppHandle,
    state: State<'_, AppState>,
    session_key: String,
    at: Option<i64>,
) -> Result<Option<crate::chat_pause::PausedChat>, String> {
    set_resume_at(Some(&app), &state.config, session_key.trim(), at).await
}

/// The body of [`chat_set_resume_at`], shared with `sessions.resume_at`.
pub(crate) async fn set_resume_at(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
    key: &str,
    at: Option<i64>,
) -> Result<Option<crate::chat_pause::PausedChat>, String> {
    let now = chrono::Utc::now().timestamp();
    if at.is_some_and(|at| at - now > RESUME_AT_MAX_AHEAD_SECS) {
        return Err("Pick a time within a year. Nothing changed.".into());
    }
    if crate::chat_pause::get(&config.vault_path, key)?.is_none() {
        return Err("This chat isn't paused. Nothing changed.".into());
    }
    if at.is_some_and(|at| at <= now) {
        resume_paused(app, config, key).await?;
        return Ok(None);
    }
    crate::chat_pause::set_resume_at(&config.vault_path, key, at)?
        .map(Some)
        .ok_or_else(|| "This chat isn't paused. Nothing changed.".to_string())
}

/// P1c: the scheduler tick's resume of every paused chat whose time has come
/// (`chat_pause::due`), each through the Resume claim under an automatic
/// token naming its attempt, so a chat the person is resuming, or that an
/// earlier tick queued, is never resumed twice. A time missed while Hollow
/// was closed comes due at the first tick after launch. Review fixes: a
/// definitive refusal (not paused, gone) drops the time; a transient failure
/// retries on the next tick up to `chat_pause::MAX_AUTO_ATTEMPTS`, then
/// gives up with a notification; a resume that starts posts one. Returns
/// the keys queued.
pub(crate) async fn resume_due_paused(
    app: Option<&AppHandle>,
    config: &crate::config::Config,
) -> Vec<String> {
    use crate::chat_pause;
    let vault = &config.vault_path;
    let now = chrono::Utc::now().timestamp();
    let due = match chat_pause::due(vault, now) {
        Ok(due) => due,
        Err(error) => {
            eprintln!("[chat pause] due resumes: {error}");
            return Vec::new();
        }
    };
    let mut resumed = Vec::new();
    for chat in due {
        let key = chat.session_key.clone();
        let attempt = chat.auto_attempts + 1;
        if attempt > chat_pause::MAX_AUTO_ATTEMPTS {
            let _ = chat_pause::set_resume_at(vault, &key, None);
            chat_pause::notify_gave_up(vault, &key);
            continue;
        }
        let token = chat_pause::new_auto_token(attempt);
        match resume_paused_as(app, config, &key, &token).await {
            Ok(_) => {
                chat_pause::notify_resumed(vault, &chat);
                resumed.push(key);
            }
            Err(error) => {
                eprintln!("[chat pause] scheduled resume {key}: {error}");
                if chat_pause::definitive_refusal(&error) {
                    if chat_pause::set_resume_at(vault, &key, None)
                        .ok()
                        .flatten()
                        .is_some()
                    {
                        chat_pause::notify_stays_paused(vault, &key, &error);
                    }
                } else {
                    // The claim was taken only if the send itself failed;
                    // a busy refusal took none and simply waits a tick.
                    chat_pause::release_for_retry(vault, &key, &token);
                }
            }
        }
    }
    resumed
}

/// Save a pasted/dropped chat image or file (base64) into the chat working
/// dir; returns the absolute path the frontend embeds in the next prompt to
/// Read. `name` is a file's original name (C5, 2026-11-04: any file
/// attaches); the saved name keeps only a cleaned stem and extension of it.
#[tauri::command]
pub async fn chat_save_attachment(
    state: State<'_, AppState>,
    data: String,
    mime: String,
    cwd: Option<String>,
    name: Option<String>,
) -> Result<String, String> {
    let base: PathBuf = match cwd {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => state.config.vault_path.clone(),
    };
    chat::save_attachment(
        &data,
        &mime,
        name.as_deref(),
        &base,
        &state.config.vault_path,
    )
}

/// The first of `paths` that is a file (chat::locate_file, 2026-11-04): a
/// reply's file link tries each place it may be before the Preview pane opens.
/// Async on the blocking pool, like the preview read.
#[tauri::command]
pub async fn chat_locate_file(paths: Vec<String>) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || chat::locate_file(&paths))
        .await
        .map_err(|e| e.to_string())
}

/// Files whose path ends with `tail` (chat::find_files_by_tail, 2026-11-10):
/// the last try for a reply's file link no place of its chat holds, under
/// every project's folder and repository (`roots`), then across the vault.
#[tauri::command]
pub async fn chat_find_file(
    state: State<'_, AppState>,
    tail: String,
    roots: Vec<String>,
) -> Result<Vec<String>, String> {
    let vault = state.config.vault_path.clone();
    tokio::task::spawn_blocking(move || {
        chat::find_files_by_tail(&tail, &roots, Some(vault.as_path()))
    })
    .await
    .map_err(|e| e.to_string())
}

/// Read a file the chat agent created/edited for the Chat preview pane.
/// Main webview only, scoped to `media::roots` like PDF/Office/media previews.
/// Absolute paths only; text capped, images as data URLs, videos as size
/// only (see chat::preview_file_scoped). Async on the blocking pool: a sync
/// command would read on the main thread.
#[tauri::command]
pub async fn chat_preview_file(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    path: String,
    session_key: Option<String>,
) -> Result<ChatPreviewFile, String> {
    if webview.label() != "main" {
        return Err("File preview is only available in Hollow's main window.".into());
    }
    let vault = state.config.vault_path.clone();
    tokio::task::spawn_blocking(move || {
        let roots = crate::media::roots(&vault, session_key.as_deref());
        chat::preview_file_scoped(&path, &roots)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Read-only PDF pages, main webview only, scoped like media previews.
#[tauri::command]
pub async fn pdf_preview_page(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    path: String,
    page: u32,
    session_key: Option<String>,
) -> Result<crate::pdf_preview::PdfPage, String> {
    if webview.label() != "main" {
        return Err("PDF preview is only available in Hollow's main window.".into());
    }
    let vault = state.config.vault_path.clone();
    static SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));
    let permit = SLOTS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "PDF preview is unavailable.")?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::pdf_preview::page(
            &path,
            page,
            &crate::media::roots(&vault, session_key.as_deref()),
        )
    })
    .await
    .map_err(|_| "This PDF page could not be rendered. Retry.".to_string())?
}

/// A read-only text preview of a Word, Excel or PowerPoint file (B1b,
/// ENG-104): main webview only, the same roots and path checks as
/// [`pdf_preview_page`] (`pdf_preview::read_scoped`), two at a time.
#[tauri::command]
pub async fn office_preview(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    path: String,
    session_key: Option<String>,
) -> Result<crate::office_preview::OfficePreview, String> {
    if webview.label() != "main" {
        return Err("Office preview is only available in Hollow's main window.".into());
    }
    let vault = state.config.vault_path.clone();
    static SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)));
    let permit = SLOTS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "Office preview is unavailable.")?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        crate::office_preview::preview(&path, &crate::media::roots(&vault, session_key.as_deref()))
    })
    .await
    .map_err(|_| "This file could not be previewed. Retry.".to_string())?
}

/// A token for playing one video through the `hollow-media` scheme (see
/// media.rs). Only the main webview may ask: `tauri::Webview` is the calling
/// webview, so a child webview inside the main window is not mistaken for
/// it. `session_key` adds that conversation's folder and its receipts'
/// repositories to the allowed roots. Errors are the stable codes
/// `OUTSIDE_ROOTS`, `NOT_VIDEO`, `UNSUPPORTED_CONTAINER`, `MISSING`,
/// `HIDDEN`; never the token, never logged.
#[tauri::command]
pub async fn media_grant(
    webview: tauri::Webview,
    state: State<'_, AppState>,
    path: String,
    session_key: Option<String>,
) -> Result<crate::media::MediaGrant, String> {
    if webview.label() != "main" {
        return Err("OUTSIDE_ROOTS".into());
    }
    let vault = state.config.vault_path.clone();
    tokio::task::spawn_blocking(move || {
        let roots = crate::media::roots(&vault, session_key.as_deref());
        crate::media::admit(&path, &roots)
            .map(crate::media::grant)
            .map_err(|r| r.code().to_string())
    })
    .await
    .map_err(|_| "MISSING".to_string())?
}

/// A poster frame for one video as a `data:image/jpeg` URL (inline video
/// PR 3): Results cards and the reply strip's cards before a player mounts.
/// The same path rules as [`media_grant`]; no grant is kept (a Results page
/// of 24 cards would crowd out the players' grants). Cached in the app's
/// cache folder under the file's identity; see `media::poster`.
#[tauri::command]
pub async fn media_poster(
    app: AppHandle,
    webview: tauri::Webview,
    state: State<'_, AppState>,
    path: String,
    session_key: Option<String>,
) -> Result<String, String> {
    use tauri::Manager;
    if webview.label() != "main" {
        return Err("OUTSIDE_ROOTS".into());
    }
    let cache = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("posters");
    let vault = state.config.vault_path.clone();
    let roots =
        tokio::task::spawn_blocking(move || crate::media::roots(&vault, session_key.as_deref()))
            .await
            .map_err(|e| e.to_string())?;
    crate::media::poster_for(path, roots, cache).await
}

/// Forget a video token (the player unmounted or went off screen).
#[tauri::command]
pub fn media_release(token: String) {
    crate::media::release(&token);
}

/// Whether an open of `target` goes to QuickTime Player: only when asked
/// (`app == "quicktime"`), only for something the guard would open anyway,
/// and only for the formats QuickTime plays.
pub(crate) fn quicktime_target<'a>(
    action: &'a crate::open_guard::OpenAction,
    app: Option<&str>,
) -> Option<&'a Path> {
    match (action, app) {
        (crate::open_guard::OpenAction::Open(target), Some("quicktime")) => {
            let ext = target
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            crate::media::is_video_ext(&ext).then_some(target.as_path())
        }
        _ => None,
    }
}

/// Open a previewed artifact outside Hollow: in the OS default app, or
/// revealed in Finder/file manager when `reveal` is true. An open is decided
/// on the path's canonical target (`open_guard`, overnight H4 and R1): a
/// folder or bundle, a Finder alias, a runnable extension, or an executable
/// whose extension isn't a known document type is revealed instead, even
/// behind a symlink with a document's name. A check, not a sandbox: `open`
/// resolves the canonical path again by name, so a file swapped in at that
/// name just before is what opens. Says which it did, `"opened"` or
/// `"revealed"`. `with: "quicktime"` opens an mp4/m4v/mov in QuickTime
/// Player (see [`quicktime_target`]); anything else ignores it.
#[tauri::command]
pub fn chat_preview_open(
    app: AppHandle,
    path: String,
    reveal: bool,
    with: Option<String>,
) -> Result<&'static str, String> {
    use crate::open_guard::{open_or_reveal, OpenAction};
    use tauri_plugin_opener::OpenerExt;
    if !Path::new(&path).is_absolute() {
        return Err("path must be absolute".into());
    }
    let opener = app.opener();
    if reveal {
        return opener
            .reveal_item_in_dir(&path)
            .map(|()| "revealed")
            .map_err(|e| e.to_string());
    }
    let action = open_or_reveal(Path::new(&path)).map_err(|e| format!("can't open {path}: {e}"))?;
    if let Some(target) = quicktime_target(&action, with.as_deref()) {
        return opener
            .open_path(target.to_string_lossy(), Some("QuickTime Player"))
            .map(|()| "opened")
            .map_err(|e| e.to_string());
    }
    match action {
        OpenAction::Open(target) => {
            let target = target
                .into_os_string()
                .into_string()
                .map_err(|_| format!("can't open {path}: its target isn't a UTF-8 path"))?;
            opener.open_path(target, None::<String>).map(|()| "opened")
        }
        OpenAction::Reveal(target) => opener.reveal_item_in_dir(target).map(|()| "revealed"),
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn claude_admin(
    state: State<'_, AppState>,
    args: Vec<String>,
    cwd: Option<String>,
) -> Result<String, String> {
    let working_dir: PathBuf = match cwd {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => state.config.vault_path.clone(),
    };
    chat::run_admin(args, Some(working_dir)).await
}

#[tauri::command]
pub async fn chat_discover(
    state: State<'_, AppState>,
    cwd: Option<String>,
) -> Result<Vec<String>, String> {
    let working_dir: PathBuf = match cwd {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => state.config.vault_path.clone(),
    };
    chat::discover_slash_commands(working_dir).await
}

#[tauri::command]
pub fn delete_session(
    state: State<'_, AppState>,
    project_dir: String,
    session_id: String,
) -> Result<(), String> {
    sessions::delete(
        &state.config.claude_projects_path,
        &project_dir,
        &session_id,
    )
}

// ── Skills ───────────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_skills() -> Vec<skills::Skill> {
    skills::list_all()
}

/// Whether each agent loads a skill, loads it degraded or doesn't (NV3): the
/// per-agent profile table with the CLI version each rule was checked on.
/// `path` is a listed `SKILL.md`; none returns the rules alone. Read-only.
#[tauri::command]
pub async fn skill_portability_table(
    path: Option<String>,
) -> Result<crate::skill_portability::PortabilityTable, String> {
    // The skill's file is read on a blocking thread; the versions are read
    // after, outside any lock (the per-binary cache is shared).
    let facts = match path.filter(|p| !p.trim().is_empty()) {
        None => None,
        Some(path) => Some(
            tauri::async_runtime::spawn_blocking(move || {
                let skill = skills::list_all()
                    .into_iter()
                    .find(|s| s.path == path)
                    .ok_or_else(|| format!("path not registered as a skill: {path}"))?;
                let text = skills::read_skill(&skill.path)?;
                let facts = crate::skill_portability::SkillFacts::from_text(
                    &text,
                    crate::skill_portability::Origin::of(&skill.source),
                );
                Ok::<_, String>((skill.name, facts))
            })
            .await
            .map_err(|e| e.to_string())??,
        ),
    };
    Ok(crate::skill_portability::resolve(facts.as_ref().map(|(n, f)| (n.as_str(), f))).await)
}

#[tauri::command]
pub fn read_skill(path: String) -> Result<String, String> {
    skills::read_skill(&path)
}

#[tauri::command]
pub fn write_user_skill(
    name: String,
    description: String,
    body: String,
    create_only: Option<bool>,
) -> Result<skills::Skill, String> {
    let skill = skills::write_user_skill(&name, &description, &body, create_only.unwrap_or(true))?;
    // Hollow's own edit: not "changed outside Hollow" (repo ideas 10-27 #2).
    if let Some(home) = dirs::home_dir() {
        let _ = crate::skill_provenance::refresh_after_edit(&home, &name);
    }
    Ok(skill)
}

#[tauri::command]
pub fn delete_user_skill(name: String) -> Result<(), String> {
    skills::delete_user_skill(&name)?;
    let _ = crate::skill_provenance::forget(&name);
    Ok(())
}

/// Repo ideas 2026-10-27 #2: a shared skill's import record (or none: "Not
/// imported by Hollow"), what changed outside Hollow, and its local note.
#[tauri::command]
pub async fn skill_provenance(name: String) -> Result<crate::skill_provenance::View, String> {
    tauri::async_runtime::spawn_blocking(move || crate::skill_provenance::view(&name))
        .await
        .map_err(|e| e.to_string())?
}

/// The person's Check for update…: stage the recorded source again (a
/// shallow clone of an https origin, only on this click) for Import's card.
/// Nothing is replaced until that card's Install.
#[tauri::command]
pub async fn stage_skill_update(
    name: String,
) -> Result<crate::skill_import::ImportPreview, String> {
    tauri::async_runtime::spawn_blocking(move || crate::skill_provenance::stage_update(&name))
        .await
        .map_err(|e| e.to_string())?
}

/// The COO's `skills.stage_skill_update` card (L13): where Check for update
/// reads from, the recorded `https://` origin it shallow-clones or the
/// recorded path on this Mac. Refused when there is nothing to check.
pub(crate) fn stage_skill_update_card(
    _cx: &crate::app_actions::Cx,
    a: &crate::app_actions::Args,
) -> Result<Option<crate::app_actions::Card>, String> {
    let name =
        a.0.get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
    let view = crate::skill_provenance::view(&name)?;
    let record = view.record.ok_or_else(|| {
        format!("Hollow has no import record for {name}; import the new version with skills.import_skill. Nothing was fetched.")
    })?;
    let (line, to) = match (&record.origin, &record.from) {
        (Some(origin), _) => (
            format!("Shallow-clones {} into Import's staging and lists what differs from the installed {name}.", origin.url),
            origin.url.clone(),
        ),
        (None, crate::skill_provenance::ImportedFrom::Path { path }) => (
            format!("Reads {path} again into Import's staging and lists what differs from the installed {name}."),
            format!("this Mac: {path}"),
        ),
        (None, crate::skill_provenance::ImportedFrom::Dropped { .. }) => {
            return Err(format!("{name} came from a dropped file, so there is nowhere to check. Nothing was fetched."))
        }
    };
    Ok(Some(
        crate::app_actions::Card::new(format!("Hollow app tools: check {name} for an update"))
            .line(line)
            .line("Nothing is installed: installing stays skills.import_skill's own card.")
            .to(to),
    ))
}

/// The person's local note on a shared skill (null or blank removes it),
/// written into its SKILL.md and kept through updates.
#[tauri::command]
pub async fn set_skill_note(
    name: String,
    note: Option<String>,
) -> Result<crate::skill_provenance::View, String> {
    tauri::async_runtime::spawn_blocking(move || crate::skill_provenance::set_note(&name, note))
        .await
        .map_err(|e| e.to_string())?
}

/// Import step 1: read a `.skill`/`.zip` file, a skill folder or a SKILL.md
/// (a picked path, or a drop's bytes), check it and stage it outside the
/// skills folder. The preview lists exactly the staged files. `rename_to`
/// (SD2) is the legal name to install a skill under when its SKILL.md name is
/// illegal; the refusal names a suggestion, and nothing is renamed unless this
/// is passed.
#[tauri::command]
pub async fn preview_skill_import(
    source: crate::skill_import::ImportSource,
    rename_to: Option<String>,
) -> Result<crate::skill_import::ImportPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::skill_import::stage_renamed(source, rename_to.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Import step 2: the person's Install. Moves the staged folder into
/// `~/.agents/skills`; `onClash` is `cancel`, `replace` or `keep-both`.
#[tauri::command]
pub async fn commit_skill_import(
    token: String,
    on_clash: Option<String>,
) -> Result<crate::skill_import::ImportOutcome, String> {
    let on_clash = crate::skill_import::OnClash::parse(on_clash.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || crate::skill_import::commit(&token, on_clash))
        .await
        .map_err(|e| e.to_string())?
}

/// Cancel: drop a staged import.
#[tauri::command]
pub fn discard_skill_import(token: String) -> Result<(), String> {
    crate::skill_import::discard(&token)
}

// ── Debug logs ───────────────────────────────────────────────────────────

/// Wipe `~/.cache/hollow/*.log`. Returns how many files were removed.
/// Surface from a settings/diagnostics panel so users can clear cached
/// email/calendar/Teams model output without using the shell.
#[tauri::command]
pub async fn clear_debug_logs() -> Result<usize, String> {
    crate::claude_runner::clear_debug_logs().await
}

// ── Plaud connector ───────────────────────────────────────────────────
//
// The Plaud connector: sign-in and a health probe (see [`plaud`]).
// Recordings stay in Plaud; nothing is pulled into the vault.

/// True when the `plaud` CLI has stored OAuth tokens (no network call).
#[tauri::command]
pub fn plaud_connected() -> bool {
    plaud::is_connected()
}

/// Run `plaud login` (opens the browser, blocks on the OAuth callback).
#[tauri::command]
pub async fn plaud_login() -> Result<(), String> {
    plaud::login().await
}

/// Run `plaud logout` (revokes the CLI's stored OAuth tokens).
#[tauri::command]
pub async fn plaud_logout() -> Result<(), String> {
    plaud::logout().await
}

/// Health probe: `plaud me` — confirms the stored token is still accepted.
#[tauri::command]
pub async fn plaud_ping() -> Result<bool, String> {
    plaud::ping().await
}

// ── Ingestion (PDF / image / YouTube / URL / text → vault) ───────────────

/// Drive a single ingestion run end-to-end. Spawns Claude with a
/// structured-notes prompt, streams `ingest-event` events to the
/// frontend for each Write + the final summary, and resolves with
/// the count of files created plus a one-paragraph rollup.
#[tauri::command]
pub async fn ingest_run(
    app: AppHandle,
    state: State<'_, AppState>,
    source: ingest::IngestSource,
    opts: ingest::IngestOpts,
) -> Result<ingest::IngestResult, String> {
    ingest::ingest(app, state.config.vault_path.clone(), source, opts).await
}

/// Kill the currently-running ingestion subprocess, if any. Idempotent.
#[tauri::command]
pub fn ingest_cancel() {
    ingest::cancel()
}

// ── Custom themes ───────────────────────────────────────────────────
// `*.theme` palette files under `dirs::config_dir() / Hollow / themes /`.
// The format is kitty-style key/value (see [`themes::parse_theme`]); the
// frontend applies them by deriving the full set of Hollow design tokens
// from `accent` / `foreground` / `background` plus the ANSI palette.

#[tauri::command]
pub fn list_custom_themes() -> Result<Vec<themes::CustomThemeMeta>, String> {
    themes::list_themes()
}

#[tauri::command]
pub fn read_custom_theme(name: String) -> Result<themes::CustomTheme, String> {
    themes::read_theme(&name)
}

#[tauri::command]
pub fn write_custom_theme(name: String, content: String) -> Result<themes::CustomTheme, String> {
    themes::write_theme(&name, &content)
}

#[tauri::command]
pub fn delete_custom_theme(name: String) -> Result<(), String> {
    themes::delete_theme(&name)
}

#[tauri::command]
pub fn reveal_themes_folder() -> Result<(), String> {
    themes::reveal_themes_dir()
}

#[tauri::command]
pub fn themes_dir_path() -> Result<String, String> {
    themes::ensure_themes_dir().map(|p| p.to_string_lossy().into_owned())
}

// ── Workflows ───────────────────────────────────────────────────────────
//
// Claude-driven automations stored in `vault/workflows/{slug}.md`. Each
// workflow has a schedule (cron expression or `manual`) and a markdown
// prompt body. Runs stream `workflow-event` to the frontend keyed by
// slug; the scheduler tokio task started in `lib.rs::setup` fires due
// workflows every minute.

#[tauri::command]
pub fn list_workflows(state: State<'_, AppState>) -> Vec<Workflow> {
    let vault = &state.config.vault_path;
    let mut workflows = vault_workflows::list(vault);
    let holds = crate::workflow_permissions::permission_holds(vault);
    // Computed separately from the permission holds: budgets never hold
    // BUDGETS_LOCK across `permission_holds`, which takes the write guard.
    let budget_holds = crate::budgets::workflow_holds(vault, &workflows, chrono::Utc::now());
    for workflow in &mut workflows {
        workflow.permission_hold = holds.get(&workflow.slug).map(|hold| hold.to_string());
        workflow.budget_hold = budget_holds.get(&workflow.slug).cloned();
    }
    workflows
}

#[tauri::command]
pub fn read_workflow(state: State<'_, AppState>, slug: String) -> Result<WorkflowDetail, String> {
    let vault = &state.config.vault_path;
    let (mut workflow, prompt, run_log) = vault_workflows::read_full(vault, &slug)?;
    // The editor's baseline for `write_workflow` (Paperclip PR 9): the save
    // is refused if the editor-owned fields change on disk before it.
    let edit_base = vault_workflows::edit_digest(&workflow, &prompt);
    workflow.permission_hold = crate::workflow_permissions::permission_holds(vault)
        .get(&workflow.slug)
        .map(|hold| hold.to_string());
    workflow.budget_hold =
        crate::budgets::workflow_holds(vault, std::slice::from_ref(&workflow), chrono::Utc::now())
            .remove(&workflow.slug);
    Ok(WorkflowDetail {
        workflow,
        prompt,
        run_log,
        edit_base,
    })
}

#[tauri::command]
pub fn list_workflow_outputs(state: State<'_, AppState>, slug: String) -> Vec<WorkflowOutput> {
    vault_workflows::list_outputs(&state.config.vault_path, &slug)
}

/// The automation editor's save. Updates an existing file as a verified line
/// patch of the editor-owned fields (every other key and the run log
/// survive), compare-and-swap against `input.edit_base`: a file changed
/// outside the editor since it was loaded rejects with
/// `WORKFLOW_EDIT_CONFLICT: …` and writes nothing. A rename writes the new
/// file before trashing the old one and is refused while the workflow runs
/// (Paperclip PR 9; `vault_workflows::upsert_input`).
#[tauri::command]
pub fn write_workflow(
    state: State<'_, AppState>,
    input: WorkflowInput,
    original_slug: Option<String>,
) -> Result<Workflow, String> {
    vault_workflows::upsert_input(&state.config.vault_path, original_slug.as_deref(), &input)
}

#[tauri::command]
pub fn delete_workflow(state: State<'_, AppState>, slug: String) -> Result<(), String> {
    vault_workflows::delete(&state.config.vault_path, &slug)
}

/// Kick off a single run of the workflow with its selected agent. Streams
/// progress via the `workflow-event` channel; resolves with the final status
/// payload once the run completes (or errors / is cancelled). Rejects with
/// "Hollow is busy: <holder> holds the vault; …" while any writer holds the
/// full-vault lease; it never waits or queues.
#[tauri::command]
pub async fn run_workflow(
    app: AppHandle,
    state: State<'_, AppState>,
    slug: String,
    retry_of: Option<String>,
) -> Result<WorkflowRunResult, String> {
    workflows::run(
        app,
        state.config.vault_path.clone(),
        slug,
        retry_of,
        crate::work_origin::Request::person_workflow(),
    )
    .await
}

/// Stop a workflow: its running run (through the normal stop path), and its
/// queued Run now and retry rows (Paperclip PR 7; a claimed row is flagged so
/// the pump stops it before or right after admission). Idempotent.
#[tauri::command]
pub async fn cancel_workflow(
    app: AppHandle,
    state: State<'_, AppState>,
    slug: String,
) -> Result<(), String> {
    workflows::cancel(&slug);
    // Only the queue owner may end rows: a Run now waiter lives in the
    // owner's memory, so a row cancelled from another process would leave
    // that `run_workflow` waiting until restart.
    crate::run_pump::accepting()?;
    let vault = state.config.vault_path.clone();
    let ended =
        tauri::async_runtime::spawn_blocking(move || workflows::cancel_queued(&vault, &slug))
            .await
            .map_err(|e| e.to_string())??;
    for item in ended {
        run_queue_api::emit_queue_event(&app, "cancelled", item);
    }
    crate::run_queue::wake();
    Ok(())
}

// ── Notifications ──────────────────────────────────────────────────────
//
// In-app notification center, backed by `vault/.notifications.jsonl`.
// Each entry shows up in the bell dropdown + (optionally) the OS
// notification center via the tauri-plugin-notification bridge. See
// [`notifications`] for the file layout and [`notifications::EVENT`]
// for the emit channel the frontend listens on.

#[tauri::command]
pub fn list_notifications(state: State<'_, AppState>) -> notifications::NotificationPage {
    notifications::list(&state.config.vault_path)
}

#[tauri::command]
pub fn unread_notification_count(state: State<'_, AppState>) -> usize {
    notifications::unread_count(&state.config.vault_path)
}

#[tauri::command]
pub fn mark_notification_read(state: State<'_, AppState>, id: String) -> Result<(), String> {
    notifications::mark_read(&state.config.vault_path, &id)
}

#[tauri::command]
pub fn mark_all_notifications_read(state: State<'_, AppState>) -> Result<usize, String> {
    notifications::mark_all_read(&state.config.vault_path)
}

/// Recent activity's Mark these read (P0-08): only the listed ids, never a
/// failure still waiting in Review.
#[tauri::command]
pub fn mark_notifications_read(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<usize, String> {
    notifications::mark_read_many(&state.config.vault_path, &ids)
}

/// Recent activity's Clear these (P0-08): only the listed ids, never a
/// failure still waiting in Review.
#[tauri::command]
pub fn delete_notifications(state: State<'_, AppState>, ids: Vec<String>) -> Result<usize, String> {
    notifications::delete_many(&state.config.vault_path, &ids)
}

#[tauri::command]
pub fn delete_notification(state: State<'_, AppState>, id: String) -> Result<(), String> {
    notifications::delete(&state.config.vault_path, &id)
}

#[tauri::command]
pub fn clear_notifications(state: State<'_, AppState>) -> Result<usize, String> {
    notifications::clear(&state.config.vault_path)
}

/// Surface a notification from the frontend. Mostly useful for testing
/// the toast + center round-trip; backend code (workflows, etc.) goes
/// through `notifications::create` directly so callers don't have to
/// hop through the IPC bus to notify themselves.
#[tauri::command]
pub fn create_notification(
    app: AppHandle,
    state: State<'_, AppState>,
    input: notifications::NotificationInput,
) -> Result<notifications::Notification, String> {
    notifications::create(&app, &state.config.vault_path, input)
}

/// Sync the Settings "macOS system notifications" toggle into the
/// backend so workflows-fired notifications honor the user's choice.
#[tauri::command]
pub fn set_notify_native_enabled(enabled: bool) {
    notifications::set_native_enabled(enabled);
}

/// 0.4: a chat finished or needs you. Shows one native banner (no notification record) and
/// returns whether it was shown; the macOS system notifications switch still silences it.
/// Called by the webview's announcer, which applies the quiet rules.
#[tauri::command]
pub fn announce_chat_native(app: AppHandle, title: String, body: String) -> Result<bool, String> {
    if !notifications::native_enabled() {
        return Ok(false);
    }
    let (title, body) = notifications::announce_text(&title, &body);
    crate::core_host::CoreHost::show_native(&app, &title, &body)?;
    Ok(true)
}

/// 0.4: the dock badge count (0 clears it). The badge belongs to the main window's app.
#[tauri::command]
pub fn set_dock_badge(app: AppHandle, count: u32) -> Result<(), String> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    window
        .set_badge_count(notifications::badge_value(count))
        .map_err(|e| e.to_string())
}

/// Sync the Settings "Keep running when I close the window" toggle into the
/// backend so the `main` window close handler in `lib.rs` either hides to the
/// tray (true) or quits the app (false).
#[tauri::command]
pub fn set_minimize_to_tray(enabled: bool) {
    crate::lifecycle::set_minimize_to_tray(enabled);
}

/// Apply or clear the native macOS window vibrancy (NSVisualEffectView)
/// behind the `main` window — the real "glass over the desktop" layer the
/// Liquid Glass theme rides on. Called at runtime when the user switches
/// into / out of a glass theme (and once on boot from the frontend), so it
/// uses `apply_vibrancy` / `clear_vibrancy`, both of which work after the
/// window already exists — unlike the window's `transparent` flag, which is
/// a static creation-time setting and stays on permanently (stock
/// Alabaster/Obsidian just paint an opaque canvas over it). `dark` selects
/// the window appearance; both polarities use the same vibrancy material.
/// No-op on non-macOS. Best-effort: a vibrancy error is surfaced but never
/// fatal — the CSS glass layer still renders, just without the desktop blur.
#[tauri::command]
pub fn set_window_vibrancy(app: AppHandle, enabled: bool, dark: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        use window_vibrancy::{
            apply_vibrancy, clear_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
        };
        if let Some(w) = app.get_window("main") {
            // tao applies this appearance to NSApplication. Leaving glass must
            // release it, or WKWebView's media query stops following macOS.
            w.set_theme(if enabled {
                Some(if dark {
                    tauri::Theme::Dark
                } else {
                    tauri::Theme::Light
                })
            } else {
                None
            })
            .map_err(|e| e.to_string())?;
            if enabled {
                // UnderWindowBackground is the documented "glass over desktop"
                // material. The explicit window appearance above supplies its
                // polarity even when macOS itself uses the opposite theme.
                let material = NSVisualEffectMaterial::UnderWindowBackground;
                apply_vibrancy(&w, material, Some(NSVisualEffectState::Active), Some(12.0))
                    .map_err(|e| e.to_string())?;
            } else {
                clear_vibrancy(&w).map_err(|e| e.to_string())?;
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, enabled, dark);
    }
    Ok(())
}

/// Stamp the `AssociatedBundleIdentifiers` launchd key onto the per-user
/// LaunchAgent plist the autostart plugin writes. Without it, macOS Login Items
/// attributes the agent to its bare program — our self-signed binary — and shows
/// the lowercase executable name ("hollow") with a generic icon. The key points
/// the OS at our app bundle so the row renders as "Hollow" with the app icon
/// instead. (It does NOT change the "unidentified developer" grouping — that's
/// inherent to the self-signed cert with no Team ID, fixable only with a real
/// Developer ID + notarization.) Best-effort + idempotent: presentation only, so
/// a failure here must never fail the enable.
///
/// It also repairs a login item whose program no longer exists. The plugin
/// records whichever binary called `enable()`, so enabling from a dev build
/// left `ProgramArguments[0]` pointing at a `target/debug/hollow` that was
/// later deleted: launchd refused it with exit 78 (EX_CONFIG) at every login
/// while Settings still showed "Run at login" as on (found 2026-09-25). See
/// [`repair_login_item`] for the rules.
pub(crate) fn brand_login_item(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        // The autostart plugin labels the plist with the package name (product
        // name), so the file is `~/Library/LaunchAgents/{name}.plist`.
        let label = app.package_info().name.clone();
        let identifier = app.config().identifier.clone();
        let Some(home) = dirs::home_dir() else { return };
        let path = home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist"));
        let Ok(mut value) = plist::Value::from_file(&path) else {
            return;
        };
        let Some(dict) = value.as_dictionary_mut() else {
            return;
        };
        let current_exe = std::env::current_exe().ok();
        if repair_login_item(dict, &identifier, current_exe.as_deref()) {
            let _ = value.to_file_xml(&path);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

/// Pure half of [`brand_login_item`]: adds the bundle association when it is
/// missing, and repoints `ProgramArguments[0]` at `current_exe` when the
/// recorded program does not exist AND `current_exe` is an installed app
/// bundle's executable (`….app/Contents/MacOS/…`). A dev or test binary never
/// becomes the login item this way, since that is how it broke. An existing
/// program is left alone, whichever copy it is. Launch arguments after the
/// program (`--hidden`) are kept. Returns whether anything changed. launchd
/// reads the file at the next login; the job already loaded is not reloaded
/// here (a `bootout` + `bootstrap` with `RunAtLoad` would start a second copy).
fn repair_login_item(
    dict: &mut plist::Dictionary,
    identifier: &str,
    current_exe: Option<&std::path::Path>,
) -> bool {
    let mut changed = false;
    if !dict.contains_key("AssociatedBundleIdentifiers") {
        dict.insert(
            "AssociatedBundleIdentifiers".into(),
            plist::Value::Array(vec![plist::Value::String(identifier.to_string())]),
        );
        changed = true;
    }
    let Some(exe) = current_exe else {
        return changed;
    };
    if !exe.to_string_lossy().contains(".app/Contents/MacOS/") || !exe.exists() {
        return changed;
    }
    let Some(args) = dict
        .get_mut("ProgramArguments")
        .and_then(plist::Value::as_array_mut)
    else {
        return changed;
    };
    let recorded_missing = match args.first().and_then(plist::Value::as_string) {
        Some(program) => !std::path::Path::new(program).exists(),
        None => false,
    };
    if recorded_missing {
        args[0] = plist::Value::String(exe.to_string_lossy().into_owned());
        changed = true;
    }
    changed
}

/// Enable / disable launch-at-login via the autostart plugin's per-user macOS
/// LaunchAgent. Returns the resulting state (re-read after the change) so the
/// UI never optimistically flips when the OS refuses (e.g. MDM-restricted
/// login items). The login item is the source of truth — there's no AtomicBool
/// to sync.
#[tauri::command]
pub fn set_run_on_startup(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    // Either way: a review launch of the everyday bundle shares its login item.
    if !crate::instance::may_change_shared_setup() {
        return Err(crate::instance::REVIEW_LOGIN_ITEM_REFUSAL.into());
    }
    if enabled {
        manager.enable().map_err(|e| e.to_string())?;
        // Re-attribute the freshly-written plist to our bundle so Login Items
        // shows "Hollow" + icon rather than the bare "hollow" binary.
        brand_login_item(&app);
    } else {
        manager.disable().map_err(|e| e.to_string())?;
    }
    manager.is_enabled().map_err(|e| e.to_string())
}

/// Whether Hollow is currently registered to launch at login.
#[tauri::command]
pub fn run_on_startup_status(app: tauri::AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Stash a fully-scaffolded artifact document for the `artifact://` custom
/// protocol to serve. The caller names the sandbox mode (`offline`, `images`,
/// `https`, `interactive`, `canvas`); `artifacts.rs` owns the CSP each mode is
/// served with, and an unknown mode is refused (the frontend then falls back
/// to srcdoc). The document comes as host-built `parts`: `artifacts::put`
/// draws the document's script nonce and puts it between consecutive parts
/// only, refusing a part count the mode doesn't have. Returns the id the
/// iframe src embeds.
#[tauri::command]
pub fn artifact_put(parts: Vec<String>, mode: String) -> Result<String, String> {
    let mode = crate::artifacts::ArtifactMode::parse(&mode)
        .ok_or_else(|| format!("unknown artifact mode {mode:?}"))?;
    crate::artifacts::put(parts, mode)
}

/// Sync the Settings "Proactive nudges" toggle into the backend so the
/// background nudge scan only fires when the user wants the initiative.
#[tauri::command]
pub fn set_nudges_enabled(enabled: bool) {
    crate::nudges::set_enabled(enabled);
}

/// Run one nudge scan immediately (Settings → "Scan now" + verification).
/// Returns how many nudges fired.
#[tauri::command]
pub async fn run_nudge_scan(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || crate::nudges::scan(&app, &vault))
        .await
        .map_err(|e| format!("join: {e}"))?
}

/// Sync the Settings, Background work "Silent heartbeat" kill switch into the
/// backend so the heartbeat loop only spends a beat when the user wants it.
/// `switched`: the person turned it (not the window's start-up sync), so
/// turning it from on to off also drops the beats waiting for a self-hosted
/// model, a Run now included (self-hosted models Q10 follow-up;
/// `heartbeat::switched_off`).
#[tauri::command]
pub fn set_heartbeat_enabled(enabled: bool, switched: Option<bool>) {
    let was = crate::heartbeat::set_enabled(enabled);
    if crate::heartbeat::switched_off(was, enabled, switched) {
        crate::self_hosted_jobs::drop_heartbeats();
    }
}

/// Sub-toggle: when off, the heartbeat runs notify-only (drops the Write tool).
#[tauri::command]
pub fn set_heartbeat_drafts_enabled(enabled: bool) {
    crate::heartbeat::set_drafts_enabled(enabled);
}

/// Sync the heartbeat tunables (work-hours window, min interval, daily budget).
#[tauri::command]
pub fn set_heartbeat_config(
    work_hours_start: u32,
    work_hours_end: u32,
    min_interval_min: i64,
    daily_budget: u32,
) {
    crate::heartbeat::set_config(crate::heartbeat::HeartbeatConfig {
        work_hours_start,
        work_hours_end,
        min_interval_min,
        daily_budget,
    });
}

/// Force one heartbeat beat now (Settings "Run now" + verification), bypassing
/// the work-hours / interval / budget gates. Returns surfaced-notification count.
#[tauri::command]
pub async fn run_heartbeat_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    crate::heartbeat::run_now(app, state.config.vault_path.clone()).await
}

/// Queued tasks the heartbeat extracted from the vault/inbox, pending approval.
#[tauri::command]
pub fn list_heartbeat_tasks(state: State<'_, AppState>) -> Vec<crate::heartbeat::PendingTask> {
    crate::heartbeat::list_task_actions(&state.config.vault_path)
}

/// Approve (create) or discard a heartbeat-extracted task. Approving routes
/// through `heartbeat::create_approved_task` → `vault_tasks::create_idempotent`
/// (fingerprint, then title slug), so the auto-extract output is an ordinary
/// vault task and a re-approved proposal never makes a twin. This is the
/// human gate: no task is created without this call.
#[tauri::command]
pub async fn resolve_heartbeat_task(
    state: State<'_, AppState>,
    id: String,
    approve: bool,
) -> Result<(), String> {
    let vault = state.config.vault_path.clone();
    let pending = crate::heartbeat::list_task_actions(&vault)
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("task {id} not found"))?;
    if pending.status != "pending" {
        return Err(format!("task already {}", pending.status));
    }
    if !approve {
        crate::heartbeat::set_task_action_status(&vault, &id, "discarded")?;
        return Ok(());
    }

    // Fingerprint first (a task this proposal already created, even after a
    // rename), then the title slug (the ~770 tasks written before
    // fingerprints existed), under one create lock: an equivalent task is
    // returned instead of minting a `slug-2.md` twin.
    crate::heartbeat::create_approved_task(&vault, &pending.task)?;
    crate::heartbeat::set_task_action_status(&vault, &id, "created")?;
    Ok(())
}

/// Sync the Settings "Agent teamwork" toggle into the backend so chat +
/// workflow Claude runs only attach the teammate-delegation MCP server when
/// the user has it enabled.
#[tauri::command]
pub fn set_teamwork_enabled(enabled: bool) {
    crate::team_mcp::set_teamwork_enabled(enabled);
}

/// Store a teammate role preset ("Codex = Engineer" + standing brief). The
/// next chat/workflow spawn carries it to the team MCP server via env, where
/// it colors the delegation tool description and prefixes every task.
#[tauri::command]
pub fn set_agent_role(agent: String, role: String, brief: String) {
    crate::team_mcp::set_role(&agent, role, brief);
}

/// Sync the Settings "Computer control" toggle into the backend so chat
/// Claude runs only attach the desktop-verbs MCP server (and the matching
/// Bash osascript/open deny rules) when the user has it enabled.
#[tauri::command]
pub fn set_computer_control_enabled(enabled: bool, sandbox: Option<bool>, this_mac: Option<bool>) {
    crate::computer_mcp::set_computer_control_enabled(enabled);
    // The same launch call carries the two target switches (CU7), so the
    // frontend's launch IPC budget doesn't grow.
    let (was_sandbox, was_this_mac) = crate::computer_broker::targets();
    crate::computer_broker::set_targets(
        sandbox.unwrap_or(was_sandbox),
        this_mac.unwrap_or(was_this_mac),
    );
}

/// Sync the Settings "Ask your vault" toggle into the backend so chat Claude
/// runs only attach the read-only `hollow_vault` search server when enabled.
#[tauri::command]
pub fn set_vault_mcp_enabled(enabled: bool) {
    crate::vault_mcp::set_vault_mcp_enabled(enabled);
}

/// Sync the Settings "Look up code" toggle (CC2): runs attach the read-only
/// `hollow_code_context` server only while it is on.
#[tauri::command]
pub fn set_code_context_mcp_enabled(enabled: bool) {
    crate::code_context_mcp::set_code_context_mcp_enabled(enabled);
}

#[tauri::command]
pub async fn directories_status(
    state: State<'_, AppState>,
) -> Result<crate::directory_connections::Connection, String> {
    let active = state.config.vault_path.clone();
    let config_path = state.config_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let map = if config_path.as_os_str().is_empty() {
            Default::default()
        } else {
            crate::setup::load_config_map(&config_path)?
        };
        let selected = map
            .get("vaultPath")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| active.clone());
        let mut c = crate::directory_connections::status(&selected)?;
        c.home_path = map
            .get("hollowHome")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        c.pending_restart = selected.canonicalize().ok() != active.canonicalize().ok();
        Ok(c)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn connect_hollow_home(
    home_path: String,
    existing_projects: String,
    existing_vault: String,
    state: State<'_, AppState>,
) -> Result<crate::directory_connections::Connection, String> {
    let config = if state.config_path.as_os_str().is_empty() {
        setup::config_write_path()?
    } else {
        state.config_path.clone()
    };
    let active = state.config.vault_path.canonicalize().ok();
    let configured = state.configured;
    tauri::async_runtime::spawn_blocking(move || {
        let mut c = crate::directory_connections::connect_home(
            &home_path,
            &existing_projects,
            &existing_vault,
            &config,
        )?;
        c.pending_restart = !configured || active.as_ref() != Some(&PathBuf::from(&c.vault_path));
        Ok(c)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn refresh_directories(
    state: State<'_, AppState>,
) -> Result<crate::directory_connections::Connection, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || crate::directory_connections::refresh(&vault))
        .await
        .map_err(|e| e.to_string())?
}
/// The workspace for a connected project, made on first open (overnight
/// 10-11 Y1); a Notion project's workspace of the same name wins.
#[tauri::command]
pub async fn open_project_workspace(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::directory_connections::open_project_workspace(&vault, &id)
    })
    .await
    .map_err(|e| e.to_string())?
}
/// The build this binary was compiled from (audit H14): version, commit,
/// uncommitted-changes flag, build time and source fingerprint.
#[tauri::command]
pub fn build_identity() -> crate::app_menu::BuildIdentity {
    crate::app_menu::build_identity()
}
/// Leave a folder in the Projects folder out of Hollow's index (or bring it
/// back), then rescan. Hollow-side only: nothing is written into the folder.
#[tauri::command]
pub async fn set_directory_exclusion(
    relative: String,
    excluded: bool,
    state: State<'_, AppState>,
) -> Result<crate::directory_connections::Connection, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::directory_connections::set_exclusion(&vault, &relative, excluded)
    })
    .await
    .map_err(|e| e.to_string())?
}
/// One page of the connected-project inventory (next-level P07): filter by a
/// path query, project and file state, paginated with offset/limit. The
/// first page of a query also carries direct lookups outside the listed
/// inventory (`direct`). `state` is `text`, `extracted` (a PDF or Office
/// file Hollow reads), `tooLarge` or `needsReader`.
#[tauri::command]
pub async fn connected_files(
    query: Option<String>,
    project_id: Option<String>,
    file_state: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<crate::directory_connections::FilePage, String> {
    let vault = state.config.vault_path.clone();
    let wanted = match file_state.as_deref() {
        None | Some("") => None,
        Some(s) => Some(
            serde_json::from_value::<crate::directory_connections::FileState>(serde_json::json!(s))
                .map_err(|_| format!("Unknown file state: {s}"))?,
        ),
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::directory_connections::list_files(
            &vault,
            query.as_deref().unwrap_or(""),
            project_id.as_deref().filter(|s| !s.is_empty()),
            wanted,
            offset.unwrap_or(0),
            limit.unwrap_or(100),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
/// Settings, Advanced, Search & indexes's "Search inside project files (local index)"
/// line (repo features R14b): on or off, and files indexed of those listed,
/// image-only PDFs, failures and files left out for privacy.
#[tauri::command]
pub async fn connected_index_status(
    state: State<'_, AppState>,
) -> Result<crate::project_text_index::Status, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || crate::project_text_index::status(&vault))
        .await
        .map_err(|e| e.to_string())
}
/// Turn the project content index on or off (the default). Off deletes its
/// database with its -wal and -shm; on starts catching up at once, on a
/// blocking thread, then with every refresh.
#[tauri::command]
pub async fn set_connected_index_enabled(
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<crate::project_text_index::Status, String> {
    let vault = state.config.vault_path.clone();
    let status = {
        let vault = vault.clone();
        tauri::async_runtime::spawn_blocking(move || {
            crate::project_text_index::set_enabled(&vault, enabled)
        })
        .await
        .map_err(|e| e.to_string())??
    };
    if enabled {
        tauri::async_runtime::spawn_blocking(move || {
            crate::project_text_index::reconcile(&vault, &crate::project_text_index::BUDGET)
        });
    }
    Ok(status)
}
#[tauri::command]
pub async fn read_connected_file(
    path: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || crate::directory_connections::read(&vault, &path))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod command_boundary_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Review of ae6e2ac84 (1): a failed held send is reported while it is
    /// still saved, and removed only after; a send reported as sent likewise.
    #[test]
    fn a_drained_held_send_is_reported_before_it_is_removed() {
        let key = "s-20261113-report01";
        let item = crate::steering::Held {
            id: "held-report-1".into(),
            after: crate::steering::After::Queue,
            at: "2026-11-13T02:00:00Z".into(),
            input: crate::steering::HeldInput {
                prompt: "keep these words".into(),
                ..Default::default()
            },
        };
        crate::steering::hold(key, item).unwrap();
        let taken = crate::steering::take_held(key).unwrap();
        let saved = || {
            crate::steering::saved_view()
                .get(key)
                .map_or(0, |queue| queue.len())
        };
        let mut seen = None;
        let event = serde_json::json!({"type": "heldFailed", "heldId": taken.id});
        report_then_settle(key, &taken.id, event, |event| {
            seen = Some((event["type"].clone(), saved()));
        });
        assert_eq!(
            seen,
            Some((serde_json::json!("heldFailed"), 1)),
            "held while reported"
        );
        // R8b: a failure stays saved, marked, until the window acknowledges.
        assert_eq!(saved(), 1, "a failed one is kept for the window's ack");
        assert_eq!(crate::steering::held_count(key), 0, "never sent again");
        assert!(crate::steering::take_held(key).is_none());
        assert!(crate::steering::unreported_failures(key).is_empty());
        // Stop's and a chip's Cancel (`cancel_held`) see it as already
        // settled, so they never return its words a second time; only the
        // acknowledgement removes it, and only a failed one.
        assert_eq!(crate::steering::cancel_held(key, &taken.id), Ok(false));
        assert_eq!(saved(), 1, "Cancel/Stop left it for the acknowledgement");
        assert_eq!(crate::steering::ack_failed(key, &taken.id), Ok(true));
        assert_eq!(saved(), 0, "removed once acknowledged");
    }

    /// R8b: a failure no window acknowledged before a quit is reported again
    /// at the next launch, and not sent.
    #[test]
    fn a_failed_held_send_nobody_acknowledged_is_reported_again_after_a_restart() {
        let key = "s-20261114-report02";
        let item = crate::steering::Held {
            id: "held-report-2".into(),
            after: crate::steering::After::Queue,
            at: "2026-11-14T02:00:00Z".into(),
            input: crate::steering::HeldInput {
                prompt: "still mine".into(),
                failed: Some("The folder is gone.".into()),
                ..Default::default()
            },
        };
        crate::steering::hold(key, item).unwrap();
        // As loaded from the file: failed, not yet reported in this process.
        assert!(crate::steering::held_conversations().contains(&key.to_string()));
        assert_eq!(crate::steering::held_count(key), 0);
        assert!(crate::steering::take_held(key).is_none());
        let again = crate::steering::unreported_failures(key);
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].input.prompt, "still mine");
        // Reported once per process, and no longer keeps the sweeper awake.
        assert!(crate::steering::unreported_failures(key).is_empty());
        assert!(!crate::steering::held_conversations().contains(&key.to_string()));
        assert_eq!(crate::steering::ack_failed(key, "held-report-2"), Ok(true));
    }

    /// R8b review: the acknowledgement never cancels a send still held.
    #[test]
    fn the_acknowledgement_leaves_a_send_still_held_to_stop_and_cancel() {
        let key = "s-20261114-report03";
        let item = crate::steering::Held {
            id: "held-report-3".into(),
            after: crate::steering::After::Queue,
            at: "2026-11-14T02:00:00Z".into(),
            input: crate::steering::HeldInput {
                prompt: "still waiting".into(),
                ..Default::default()
            },
        };
        crate::steering::hold(key, item).unwrap();
        assert_eq!(crate::steering::ack_failed(key, "held-report-3"), Ok(false));
        assert_eq!(crate::steering::held_count(key), 1);
        assert_eq!(crate::steering::cancel_held(key, "held-report-3"), Ok(true));
    }

    /// Review of ae6e2ac84 (2): a run queue that can't be read is never
    /// "no row", so a held send that may have gone is not sent again.
    #[test]
    fn a_held_send_whose_row_cant_be_checked_is_not_sent() {
        let dir = TempDir::new().unwrap();
        let key = crate::steering::queue_key("held-lookup-1");
        assert_eq!(held_row_lookup(dir.path(), &key), HeldLookup::Send);
        // A store that can't be opened: a folder where its database goes.
        let broken = dir.path().join("broken-vault");
        let db = crate::session_store::db_path_for(&broken).unwrap();
        fs::create_dir_all(&db).unwrap();
        assert!(matches!(
            held_row_lookup(&broken, &key),
            HeldLookup::Retry(_)
        ));
        fs::remove_dir_all(&db).unwrap();
        // The drain puts it back on Retry rather than sending.
        let src = include_str!("commands.rs");
        let at = src
            .find(concat!("pub(crate) async fn ", "drain_held("))
            .unwrap();
        let body = &src[at..at + src[at..].find("\n}\n").unwrap()];
        let retry = &body[body.find("HeldLookup::Retry(error) => {").unwrap()..];
        let arm = &retry[..retry.find("return;").unwrap()];
        assert!(
            arm.contains(concat!("steering::", "put_back(key, held)")),
            "{arm}"
        );
    }

    /// Settings PR 7: the composer's per-chat effort is a level or nothing.
    #[test]
    fn a_chats_own_effort_is_a_level_or_the_default() {
        assert_eq!(chat_effort(None), Ok(None));
        assert_eq!(chat_effort(Some(" ".into())), Ok(None));
        assert_eq!(chat_effort(Some("high".into())), Ok(Some("high".into())));
        assert!(chat_effort(Some("ultracode".into())).is_err());
        assert!(chat_effort(Some("turbo".into())).is_err());
    }

    /// L1: an agent's message to a busy conversation is held at its own
    /// depth, effort and unattended state with the asker's delegation
    /// record, survives a restart's round trip, and goes out as the agent's
    /// (never the person's) when the turn ends.
    #[test]
    fn an_agents_held_message_keeps_its_depth_effort_and_record() {
        let input = ChatSendInput {
            prompt: "Next step".into(),
            session_key: Some("s-20261110-busy0001".into()),
            app_depth: 1,
            unattended: true,
            ..Default::default()
        };
        let mut held = held_input(&input);
        held.effort = Some("medium".into());
        held.delegation_run = Some("run-asker".into());
        let record = crate::run_manifest::DelegationRecord {
            kind: "childTask".into(),
            tool: "send_to_session".into(),
            ..Default::default()
        };
        held.delegation_record = serde_json::to_string(&record).ok();
        let wire = serde_json::to_string(&held).unwrap();
        let back: crate::steering::HeldInput = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, held);
        let out = send_input(&back);
        assert_eq!(out.app_depth, 1);
        assert!(out.unattended);
        assert!(!out.typed);
        // The person's own held send is unchanged on the wire.
        let own = held_input(&ChatSendInput {
            prompt: "Mine".into(),
            ..Default::default()
        });
        let wire = serde_json::to_value(&own).unwrap();
        for absent in [
            "appDepth",
            "effort",
            "unattended",
            "delegationRun",
            "delegationRecord",
        ] {
            assert!(wire.get(absent).is_none(), "{absent}");
        }
        assert_eq!(send_input(&own).app_depth, 0);
        // At drain time it takes the conversation's current native session
        // and never a stronger mode than its last (L1 review).
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let key = "s-20261110-busy0001";
        let ord =
            crate::session_store::start_turn(v, key, "claude", "/vault", "run-busy", "Go").unwrap();
        crate::session_store::record_harness_id(v, key, "native-new").unwrap();
        crate::session_store::record_turn_permission(v, key, ord, "default").unwrap();
        let mut stale = held_input(&ChatSendInput {
            permission_mode: Some("bypassPermissions".into()),
            session_id: Some("native-old".into()),
            ..input.clone()
        });
        stale.app_depth = 1;
        let fresh = refreshed_agent_input(v, &stale);
        assert_eq!(fresh.session_id.as_deref(), Some("native-new"));
        assert_eq!(fresh.permission_mode.as_deref(), Some("default"));
        // 2026-11-12: it follows the conversation's own mode as it is now
        // (raised to bypass while it waited: bypass, not the held mode), and
        // only the sender's cap lowers it.
        crate::session_store::record_turn_permission(v, key, ord, "bypassPermissions").unwrap();
        stale.permission_mode = Some("acceptEdits".into());
        let raised = refreshed_agent_input(v, &stale);
        assert_eq!(raised.permission_mode.as_deref(), Some("bypassPermissions"));
        for (cap, want) in [
            ("plan", "plan"),
            ("default", "default"),
            ("auto", "auto"),
            ("bypassPermissions", "bypassPermissions"),
        ] {
            let capped = crate::steering::HeldInput {
                mode_cap: Some(cap.into()),
                ..stale.clone()
            };
            let out = refreshed_agent_input(v, &capped);
            assert_eq!(out.permission_mode.as_deref(), Some(want), "{cap}");
        }
        // 2026-11-20: a send lifted to its sender's mode keeps it while that
        // is stronger than the conversation's own now (bypass here, so a
        // lower lift changes nothing); the cap still lowers it.
        crate::session_store::record_turn_permission(v, key, ord, "acceptEdits").unwrap();
        let lifted = crate::steering::HeldInput {
            mode_lift: Some("bypassPermissions".into()),
            ..stale.clone()
        };
        assert_eq!(
            refreshed_agent_input(v, &lifted).permission_mode.as_deref(),
            Some("bypassPermissions")
        );
        let lifted_capped = crate::steering::HeldInput {
            mode_cap: Some("default".into()),
            ..lifted.clone()
        };
        assert_eq!(
            refreshed_agent_input(v, &lifted_capped)
                .permission_mode
                .as_deref(),
            Some("default")
        );
        let weak_lift = crate::steering::HeldInput {
            mode_lift: Some("plan".into()),
            ..stale.clone()
        };
        assert_eq!(
            refreshed_agent_input(v, &weak_lift)
                .permission_mode
                .as_deref(),
            Some("acceptEdits")
        );
        // The cap survives the held store's round trip; none is not written.
        let capped = crate::steering::HeldInput {
            mode_cap: Some("plan".into()),
            ..stale.clone()
        };
        let back: crate::steering::HeldInput =
            serde_json::from_value(serde_json::to_value(&capped).unwrap()).unwrap();
        assert_eq!(back.mode_cap.as_deref(), Some("plan"));
        assert!(serde_json::to_value(&stale)
            .unwrap()
            .get("modeCap")
            .is_none());
        assert_eq!(MAX_AGENT_HELD_ATTEMPTS, 3);
        // The drain sends an agent's at its effort and stores its record.
        let src = include_str!("commands.rs");
        let at = src
            .find(concat!("pub(crate) async fn ", "drain_held("))
            .unwrap();
        let body = &src[at..at + src[at..].find("\n}\n").unwrap()];
        assert!(body.contains("if held.input.app_depth > 0 {"));
        assert!(body.contains(concat!("settle_agent_", "held(")));
    }

    /// "Open in QuickTime Player" only reaches QuickTime for a video the
    /// guard would open: a script is still revealed, a PDF opens as usual.
    #[test]
    fn quicktime_opens_only_videos_the_guard_would_open() {
        use crate::open_guard::{open_or_reveal, OpenAction};
        let dir = TempDir::new().unwrap();
        let video = dir.path().join("clip.MOV");
        let script = dir.path().join("run.sh");
        let pdf = dir.path().join("doc.pdf");
        fs::write(&video, b"x").unwrap();
        fs::write(&script, b"echo hi").unwrap();
        fs::write(&pdf, b"%PDF").unwrap();
        let qt = |p: &Path| {
            let action = open_or_reveal(p).unwrap();
            quicktime_target(&action, Some("quicktime")).is_some()
        };
        assert!(qt(&video));
        assert!(!qt(&pdf), "a pdf opens in its default app");
        assert!(matches!(
            open_or_reveal(&script).unwrap(),
            OpenAction::Reveal(_)
        ));
        assert!(!qt(&script), "a script is still revealed");
        let action = open_or_reveal(&video).unwrap();
        assert!(quicktime_target(&action, None).is_none());
        assert!(quicktime_target(&action, Some("vlc")).is_none());
    }

    fn filter(id: &str) -> TaskFilter {
        TaskFilter {
            id: id.into(),
            name: id.into(),
            query: "p1".into(),
        }
    }

    #[test]
    fn a_damaged_task_filter_file_is_kept_never_overwritten_by_a_save() {
        let vault = TempDir::new().unwrap();
        save_task_filter_in(vault.path(), filter("a")).unwrap();
        save_task_filter_in(vault.path(), filter("b")).unwrap();
        assert_eq!(read_task_filters(vault.path()).unwrap().len(), 2);
        let path = task_filters_path(vault.path());
        let damaged = b"[{\"id\": \xff".to_vec();
        fs::write(&path, &damaged).unwrap();
        // Before B·F08 this read as no filters and the save kept only "c".
        let err = save_task_filter_in(vault.path(), filter("c")).unwrap_err();
        assert!(err.contains("Saved task filters was damaged"), "{err}");
        let kept = crate::state_file::corrupt_copies(&path);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(fs::read(&kept[0]).unwrap(), damaged, "bytes kept exactly");
        assert!(!path.exists(), "nothing was written over it");
        // With the damage set aside, saving starts a fresh list.
        save_task_filter_in(vault.path(), filter("c")).unwrap();
        assert_eq!(read_task_filters(vault.path()).unwrap().len(), 1);
    }

    #[test]
    fn an_agent_read_leaves_a_damaged_task_filter_file_in_place() {
        let vault = TempDir::new().unwrap();
        let path = task_filters_path(vault.path());
        fs::write(&path, b"nope").unwrap();
        let err =
            read_task_filters_or(vault.path(), crate::state_file::OnDamage::Leave).unwrap_err();
        assert!(err.contains("Saved task filters is damaged"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), b"nope");
        assert!(crate::state_file::corrupt_copies(&path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_task_filter_file_is_surfaced_and_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let vault = TempDir::new().unwrap();
        save_task_filter_in(vault.path(), filter("a")).unwrap();
        let path = task_filters_path(vault.path());
        let before = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            return; // running as root: permissions don't stop the read
        }
        let err = delete_task_filter_in(vault.path(), "a").unwrap_err();
        assert!(err.contains("could not be read"), "{err}");
        assert!(read_task_filters(vault.path()).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before, "never replaced");
        assert!(crate::state_file::corrupt_copies(&path).is_empty());
    }

    fn login_dict(program: &str, branded: bool) -> plist::Dictionary {
        let mut dict = plist::Dictionary::new();
        dict.insert("Label".into(), plist::Value::String("Hollow".into()));
        dict.insert(
            "ProgramArguments".into(),
            plist::Value::Array(vec![
                plist::Value::String(program.into()),
                plist::Value::String("--hidden".into()),
            ]),
        );
        if branded {
            dict.insert(
                "AssociatedBundleIdentifiers".into(),
                plist::Value::Array(vec![plist::Value::String("com.hollow.app".into())]),
            );
        }
        dict
    }

    fn program_args(dict: &plist::Dictionary) -> Vec<String> {
        dict.get("ProgramArguments")
            .and_then(plist::Value::as_array)
            .unwrap()
            .iter()
            .map(|v| v.as_string().unwrap().to_string())
            .collect()
    }

    fn fake_bundle_exe(temp: &TempDir) -> std::path::PathBuf {
        let exe = temp.path().join("Hollow.app/Contents/MacOS/hollow");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "").unwrap();
        exe
    }

    #[test]
    fn login_item_with_a_missing_program_is_repointed_at_the_installed_bundle() {
        let temp = TempDir::new().unwrap();
        let exe = fake_bundle_exe(&temp);
        let missing = temp.path().join("hollow/src-tauri/target/debug/hollow");
        let mut dict = login_dict(missing.to_str().unwrap(), true);
        assert!(repair_login_item(&mut dict, "com.hollow.app", Some(&exe)));
        assert_eq!(
            program_args(&dict),
            vec![exe.to_string_lossy().into_owned(), "--hidden".to_string()]
        );
        // Idempotent: a second pass finds nothing to do.
        assert!(!repair_login_item(&mut dict, "com.hollow.app", Some(&exe)));
    }

    #[test]
    fn login_item_repair_never_installs_a_dev_binary_or_replaces_a_live_program() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("gone/hollow");
        let dev = temp.path().join("target/debug/hollow");
        fs::create_dir_all(dev.parent().unwrap()).unwrap();
        fs::write(&dev, "").unwrap();
        let mut dict = login_dict(missing.to_str().unwrap(), true);
        assert!(!repair_login_item(&mut dict, "com.hollow.app", Some(&dev)));
        assert_eq!(program_args(&dict)[0], missing.to_string_lossy());

        // A program that exists is kept even when another bundle is running.
        let exe = fake_bundle_exe(&temp);
        let mut live = login_dict(dev.to_str().unwrap(), true);
        assert!(!repair_login_item(&mut live, "com.hollow.app", Some(&exe)));
        assert_eq!(program_args(&live)[0], dev.to_string_lossy());
    }

    #[test]
    fn login_item_branding_is_added_once() {
        let temp = TempDir::new().unwrap();
        let dev = temp.path().join("hollow");
        fs::write(&dev, "").unwrap();
        let mut dict = login_dict(dev.to_str().unwrap(), false);
        assert!(repair_login_item(&mut dict, "com.hollow.app", None));
        assert!(dict.contains_key("AssociatedBundleIdentifiers"));
        assert!(!repair_login_item(&mut dict, "com.hollow.app", None));
    }

    fn assist_input(action: &str, project_url: Option<&str>) -> AssistInput {
        AssistInput {
            action: action.into(),
            title: "Write the export check".into(),
            description: "Covers CSV and JSON".into(),
            project_url: project_url.map(str::to_string),
        }
    }

    #[test]
    fn assist_prompt_includes_alignment_only_for_breakdown_and_next_steps() {
        let block = "\n\n<project-alignment>\nREF\n</project-alignment>";
        for action in ["breakdown", "next_steps"] {
            let prompt = assist_prompt(&assist_input(action, None), Some(block)).unwrap();
            assert!(prompt.ends_with(block), "{action}");
        }
        let rephrase = assist_prompt(&assist_input("rephrase", None), Some(block)).unwrap();
        assert!(!rephrase.contains("project-alignment"));
        // Without a block, breakdown is byte-for-byte the pre-PR prompt.
        assert_eq!(
            assist_prompt(&assist_input("breakdown", None), None).unwrap(),
            "Break this task into 3-7 concrete, ordered sub-tasks. Output ONLY a markdown \
             checklist, one `- [ ] step` per line. No preamble, no commentary.\n\nTask: \
             Write the export check\nCovers CSV and JSON"
        );
        assert!(assist_prompt(&assist_input("summon", None), None)
            .unwrap_err()
            .contains("unknown assist action"));
    }

    #[test]
    fn assist_alignment_wires_linked_projects_for_planning_actions_only() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let vault = dir.path();
        let goal = vault_goals::create(
            vault,
            vault_goals::GoalCreateInput {
                title: "Exports never lose data".into(),
                status: Some(vault_goals::GoalStatus::Active),
                why: Some("Customers rely on them".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let linked = vault_projects::create(
            vault,
            vault_projects::ProjectFrontmatter {
                title: "Exports".into(),
                status: "active".into(),
                goal_urls: vec![goal.url],
                ..Default::default()
            },
        )
        .unwrap();
        let bare = vault_projects::create(
            vault,
            vault_projects::ProjectFrontmatter {
                title: "Bare".into(),
                status: "active".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let url = format!("vault:{linked}");
        let breakdown = assist_alignment(vault, &assist_input("breakdown", Some(&url)))
            .expect("breakdown gets the goals");
        assert!(breakdown.contains("Exports never lose data"));
        assert!(assist_alignment(vault, &assist_input("next_steps", Some(&url))).is_some());
        assert!(assist_alignment(vault, &assist_input("rephrase", Some(&url))).is_none());
        assert!(assist_alignment(vault, &assist_input("breakdown", Some(""))).is_none());
        assert!(assist_alignment(vault, &assist_input("breakdown", Some("   "))).is_none());
        assert!(assist_alignment(vault, &assist_input("breakdown", None)).is_none());
        let bare = format!("vault:{bare}");
        assert!(assist_alignment(vault, &assist_input("breakdown", Some(&bare))).is_none());
    }

    #[test]
    fn assist_input_reads_an_optional_camel_case_project_url() {
        let input: AssistInput = serde_json::from_value(serde_json::json!({
            "action": "breakdown",
            "title": "T",
            "projectUrl": "vault:projects/active/Exports.md",
        }))
        .unwrap();
        assert_eq!(
            input.project_url.as_deref(),
            Some("vault:projects/active/Exports.md")
        );
        let input: AssistInput =
            serde_json::from_value(serde_json::json!({"action": "rephrase", "title": "T"}))
                .unwrap();
        assert!(input.project_url.is_none());
        assert_eq!(input.description, "");
    }
}
