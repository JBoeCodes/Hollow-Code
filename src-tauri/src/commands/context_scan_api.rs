//! Context scan commands (CX19): look for instruction and memory files in the
//! agents' folders and propose them for the review card (CX20). Read only.
//! `crate::context_scan` owns the scan; this file resolves the default
//! folders (the only place that reads `$HOME` and `CODEX_HOME`), the staging
//! root and the vault guard, and runs the pass on its own thread.
//!
//! Reading the person's agent folders waits for their click
//! (`start_context_scan` is user-only in the command map); the other four are
//! reads of the staging folder.

use super::AppState;
use crate::context_scan::{self as scan, ScanManifest, ScanRoot, ScanStatus, Scope};
use crate::session_import::staging::{self, Staging};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::State;

fn staging() -> Result<Staging, String> {
    staging::default_root()
        .map(Staging::new)
        .ok_or_else(|| "Hollow has no app support folder for staging a scan.".to_string())
}

fn default_roots() -> Result<Vec<ScanRoot>, String> {
    let home = dirs::home_dir().ok_or("Hollow can't find your home folder.")?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    Ok(scan::default_roots(&home, codex_home.as_deref()))
}

/// One folder the scan offers, shown before anything is read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSource {
    pub label: String,
    pub path: String,
    pub scope: Scope,
    /// Whether the folder exists (a stat, nothing inside is read).
    pub exists: bool,
}

/// The default folders and whether each exists. Reads nothing inside them.
#[tauri::command]
pub async fn context_scan_sources() -> Result<Vec<ScanSource>, String> {
    Ok(default_roots()?
        .into_iter()
        .map(|r| ScanSource {
            exists: r.path.is_dir(),
            label: r.label,
            path: r.path.to_string_lossy().into_owned(),
            scope: r.scope,
        })
        .collect())
}

/// A folder the person picked.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedRoot {
    pub path: String,
    /// `user` or `project` (default `project`).
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub label: Option<String>,
}

/// A scan just started.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStarted {
    pub scan_id: String,
    pub roots: Vec<String>,
}

/// Start scanning `roots` (else the default agent folders) on one reader
/// thread; answers at once with the scan's id. Poll `context_scan_status`,
/// then `list_context_scan`. The person's click. A folder inside the vault is
/// refused.
#[tauri::command]
pub async fn start_context_scan(
    state: State<'_, AppState>,
    roots: Option<Vec<PickedRoot>>,
) -> Result<ScanStarted, String> {
    let vault = state.config.vault_path.clone();
    let roots: Vec<ScanRoot> = match roots.filter(|r| !r.is_empty()) {
        Some(picked) => picked
            .into_iter()
            .map(|p| {
                let path = PathBuf::from(p.path.trim());
                if !path.is_absolute() {
                    return Err("Pick a folder by its full path.".to_string());
                }
                let label = p.label.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Folder".into())
                });
                Ok(ScanRoot {
                    path,
                    scope: p.scope.unwrap_or(Scope::Project),
                    label,
                    agent: None,
                })
            })
            .collect::<Result<_, _>>()?,
        None => default_roots()?,
    };
    let (roots, avoid) = scan::resolve_roots(roots, &vault)?;
    let staging = staging()?;
    let id = staging::mint_id();
    let cancel = scan::begin_job(&id);
    let job = id.clone();
    let shown = roots
        .iter()
        .map(|r| r.path.to_string_lossy().into_owned())
        .collect();
    std::thread::Builder::new()
        .name("context-scan".into())
        .spawn(move || {
            let result = scan::scan(
                &staging,
                &job,
                &roots,
                &avoid,
                &scan::ScanBudget::default(),
                &cancel,
            );
            scan::end_job(&job, &result);
        })
        .map_err(|e| format!("could not start the scan: {e}"))?;
    Ok(ScanStarted {
        scan_id: id,
        roots: shown,
    })
}

/// A scan's progress (this run's, else a staged scan's).
#[tauri::command]
pub async fn context_scan_status(scan_id: String) -> Result<Option<ScanStatus>, String> {
    crate::vault::spawn_blocking_logged(move || scan::status(&staging()?, &scan_id))
        .await
        .map_err(|e| e.to_string())?
}

/// A staged scan's proposals: per file its path, kind, size, hash, excerpt
/// (never for a secret-like file), flags and scope.
#[tauri::command]
pub async fn list_context_scan(scan_id: String) -> Result<ScanManifest, String> {
    crate::vault::spawn_blocking_logged(move || scan::manifest(&staging()?, &scan_id))
        .await
        .map_err(|e| e.to_string())?
}

/// Stop a running scan or delete a staged one.
#[tauri::command]
pub async fn cancel_context_scan(scan_id: String) -> Result<bool, String> {
    crate::vault::spawn_blocking_logged(move || scan::cancel(&staging()?, &scan_id))
        .await
        .map_err(|e| e.to_string())?
}
