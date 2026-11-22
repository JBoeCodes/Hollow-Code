//! Session import commands (H5a): Claude Code and Codex conversations from
//! their folders. `crate::session_import` owns the flow; this file resolves
//! the default folders (the only place that reads `$HOME` and `CODEX_HOME`)
//! and the staging root, and runs the inspect pass on its own thread.
//!
//! Reading a person's agent folders and writing what they hold into Hollow
//! is the person's click (`inspect_session_import`, `commit_session_import`,
//! `continue_imported_session`, `set_session_import_search` are user-only in
//! the command map); the agent's read is `sessions.session_import_status`.

use super::AppState;
use crate::session_import::native_resume::{self, ContinuedNative, NativeOffer, NativeRequest};
use crate::session_import::{
    self as import, staging::Staging, CommitRequest, CommitResult, ContinueRequest, Continued,
    JobStatus, Manifest, Preview, Source,
};
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

fn staging() -> Result<Staging, String> {
    import::staging::default_root()
        .map(Staging::new)
        .ok_or_else(|| "Hollow has no app support folder for staging an import.".to_string())
}

fn source(value: &str) -> Result<Source, String> {
    Source::parse(value).ok_or_else(|| format!("Unknown import source {value:?}."))
}

/// `source`'s default folder on this Mac.
fn default_root(source: Source) -> Result<Option<PathBuf>, String> {
    let home = dirs::home_dir().ok_or("Hollow can't find your home folder.")?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    Ok(import::default_root(source, &home, codex_home.as_deref()))
}

/// One source the import offers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    pub source: Source,
    pub label: &'static str,
    /// The folder Hollow would read, shown before it reads anything.
    pub default_root: Option<String>,
    /// An export file (ChatGPT): the person picks the zip or its folder;
    /// there is no default folder to look in.
    pub export: bool,
    /// Whether that folder exists (a stat, nothing inside is read).
    pub exists: bool,
}

/// The sources and their default folders, for the import sheet's first
/// step. Reads nothing inside them.
#[tauri::command]
pub async fn session_import_sources() -> Result<Vec<ImportSource>, String> {
    Ok([
        Source::ClaudeCode,
        Source::Codex,
        Source::Pi,
        Source::ChatGpt,
        Source::ClaudeAi,
        Source::Gemini,
    ]
    .into_iter()
    .map(|source| {
        let root = default_root(source).ok().flatten();
        ImportSource {
            source,
            label: source.label(),
            export: source.is_export(),
            exists: root.as_ref().is_some_and(|r| r.is_dir()),
            default_root: root.map(|r| r.to_string_lossy().into_owned()),
        }
    })
    .collect())
}

/// An inspect pass just started.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectStarted {
    pub import_id: String,
    pub root: String,
}

/// Start reading `source`'s folder (`root`, else its default) into a staged
/// import, on one reader thread. Answers at once with the import's id; poll
/// `session_import_status`, then `list_session_import`. The person's click.
#[tauri::command]
pub async fn inspect_session_import(
    state: State<'_, AppState>,
    source: String,
    root: Option<String>,
) -> Result<InspectStarted, String> {
    let source = self::source(&source)?;
    let root = match root.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => {
            let path = PathBuf::from(r);
            if !path.is_absolute() {
                return Err("Pick a folder by its full path.".into());
            }
            path
        }
        None => default_root(source)?.ok_or_else(|| {
            format!(
                "Pick the {} export (the .zip, or its unzipped folder).",
                source.label()
            )
        })?,
    };
    let staging = staging()?;
    let vault = state.config.vault_path.clone();
    let id = import::staging::mint_id();
    let cancel = import::begin_job(&id, source);
    let (job, folder) = (id.clone(), root.clone());
    std::thread::Builder::new()
        .name("session-import".into())
        .spawn(move || {
            let projects = import::connected_projects(&vault);
            let cx = import::Context {
                vault: &vault,
                staging: &staging,
                projects: &projects,
            };
            let budget = import::walker::Budget::default();
            let result = import::inspect(&cx, &job, source, &folder, &budget, &cancel);
            import::end_job(&job, &result);
        })
        .map_err(|e| format!("could not start the import: {e}"))?;
    Ok(InspectStarted {
        import_id: id,
        root: root.to_string_lossy().into_owned(),
    })
}

/// Imports in progress or staged, and the committed batches.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStatus {
    /// `import_id`'s progress when one was asked for, else every staged one.
    pub jobs: Vec<JobStatus>,
    pub batches: Vec<import::Batch>,
}

/// An import's progress (or every staged import's), and the committed
/// batches with their Keep out of agent search switch. Reads only.
#[tauri::command]
pub async fn session_import_status(
    state: State<'_, AppState>,
    import_id: Option<String>,
) -> Result<ImportStatus, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        let staging = staging()?;
        let ids = match import_id {
            Some(id) => vec![id],
            None => staging.ids(),
        };
        let mut jobs = Vec::new();
        for id in ids {
            if let Some(job) = import::job_status(&staging, &id)? {
                jobs.push(job);
            }
        }
        Ok(ImportStatus {
            jobs,
            batches: import::batches(&vault)?,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A staged import's list: per conversation its title, dates, counts,
/// project folder, destination and duplicate status.
#[tauri::command]
pub async fn list_session_import(import_id: String) -> Result<Manifest, String> {
    crate::vault::spawn_blocking_logged(move || import::manifest(&staging()?, &import_id))
        .await
        .map_err(|e| e.to_string())?
}

/// A bounded preview of one staged conversation (its first and last turns).
#[tauri::command]
pub async fn preview_session_import(import_id: String, entry: String) -> Result<Preview, String> {
    crate::vault::spawn_blocking_logged(move || import::preview(&staging()?, &import_id, &entry))
        .await
        .map_err(|e| e.to_string())?
}

/// Import the picked conversations (one transaction each). The person's
/// Import click.
#[tauri::command]
pub async fn commit_session_import(
    state: State<'_, AppState>,
    import_id: String,
    request: Option<CommitRequest>,
) -> Result<CommitResult, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        let staging = staging()?;
        let projects = import::connected_projects(&vault);
        let cx = import::Context {
            vault: &vault,
            staging: &staging,
            projects: &projects,
        };
        import::commit(
            &cx,
            &import_id,
            &request.unwrap_or_default(),
            chrono::Utc::now(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Cancel a running or staged import and discard what it staged.
#[tauri::command]
pub async fn cancel_session_import(import_id: String) -> Result<bool, String> {
    crate::vault::spawn_blocking_logged(move || import::cancel(&staging()?, &import_id))
        .await
        .map_err(|e| e.to_string())?
}

/// Continue in Hollow (N26): a new conversation linked to imported one
/// `key`, with the agent, model, workspace and mode the person picked. The
/// person's click; the imported record stays read-only.
#[tauri::command]
pub async fn continue_imported_session(
    state: State<'_, AppState>,
    key: String,
    request: ContinueRequest,
) -> Result<Continued, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        import::continue_imported(&vault, &key, &request, chrono::Utc::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// H8: whether imported chat `key` can be continued by forking its Claude
/// Code session (flags probed on the installed CLI, source file still there
/// with the same id and folder), and why not when it can't. Starts no model
/// run; the CLI is asked only for its `--version` and `--help` text.
#[tauri::command]
pub async fn native_resume_offer(
    state: State<'_, AppState>,
    key: String,
) -> Result<NativeOffer, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || native_resume::native_offer(&vault, &key))
        .await
        .map_err(|e| e.to_string())?
}

/// H8: Continue in Hollow by native fork-resume: a new chat in the source
/// folder whose first turn runs `claude --resume <id> --fork-session`. The
/// person's click; refuses with the reason when `native_resume_offer` says
/// no, and the caller then uses `continue_imported_session` (the handoff).
#[tauri::command]
pub async fn continue_imported_session_native(
    state: State<'_, AppState>,
    key: String,
    request: Option<NativeRequest>,
) -> Result<ContinuedNative, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        native_resume::continue_native(
            &vault,
            &key,
            &request.unwrap_or_default(),
            chrono::Utc::now(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// N25: keep import `import_id`'s conversations out of agent search (or let
/// them back in). The number of conversations changed.
#[tauri::command]
pub async fn set_session_import_search(
    state: State<'_, AppState>,
    import_id: String,
    keep_out: bool,
) -> Result<usize, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        import::set_keep_out_of_search(&vault, &import_id, keep_out)
    })
    .await
    .map_err(|e| e.to_string())?
}
