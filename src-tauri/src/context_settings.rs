//! Settings, Context (CX17): the reads and the few person-only writes behind
//! one row per context layer (home, personal, org, project folders, vault).
//!
//! Nothing here syncs. The personal row drives `sync_engine` through its own
//! commands (`context_sync_status`, `context_sync_now`,
//! `set_context_sync_enabled`, `configure_personal_sync`); this module adds
//! what those don't cover:
//!
//! - [`context_layers_status`]: every row's status in one read (also the
//!   agents' `settings.context_layers_status`). Reads only: the saved roots
//!   and slug, the sync settings, the org clone's origin URL (redacted) and
//!   when it last fetched, and which project folders carry a `CLAUDE.md` or
//!   `AGENTS.md`.
//! - [`seed_report`] (PX1): before the personal sync's first cycle, a dry run of
//!   what that cycle would do, from the vault's and the personal clone's
//!   synced folders side by side (new on either side, clashes). It rides
//!   [`context_layers_status`] as `seed`, so there is no command to add.
//! - [`context_org_access_check`]: `git ls-remote` on the org clone's origin,
//!   no prompt of any kind, 30 s at most; says whether this Mac can fetch.
//! - [`context_personal_local_only`]: drops the personal remote and turns
//!   sync off; every file and the git directory stay.
//! - [`set_context_root`] and [`set_context_machine_slug`]: the person's
//!   choice of clone folder and machine name (`context_roots`' rules).
//!
//! Create is guided steps in the window (a private repo the person makes on
//! GitHub, then Connect); creating a repo for them is CX18's card.

use crate::context_roots::{self as roots, Layer, RootStatus};
use crate::sync_engine::{self, RootKind, SyncStatus};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Project folders listed at most.
const PROJECTS_MAX: usize = 60;
/// The files that make a project folder carry context.
const PROJECT_FILES: [&str; 2] = ["CLAUDE.md", "AGENTS.md"];
/// The access check's limit.
const ACCESS_TIMEOUT: Duration = Duration::from_secs(30);

/// The home row: this Mac's slug and its file in the personal clone.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeLayer {
    pub slug: Option<String>,
    /// Offered from the Mac's name while no slug is saved.
    pub suggested_slug: Option<String>,
    /// `home/<slug>.md`, relative to the personal clone.
    pub file: Option<String>,
    pub file_exists: bool,
    /// The personal clone folder the home file is read from.
    pub root: RootStatus,
}

/// The org row: the read-only clone.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrgLayer {
    pub root: RootStatus,
    /// The clone's `origin`, any user part shown as `***`.
    pub remote: Option<String>,
    /// When the clone last fetched (`.git/FETCH_HEAD`'s time, ISO 8601).
    pub last_pull_at: Option<String>,
}

/// One project folder that carries context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectContext {
    pub title: String,
    pub path: String,
    /// Which of `CLAUDE.md` and `AGENTS.md` it has.
    pub files: Vec<String>,
    /// It is a git work tree (a `.git` at its top).
    pub git: bool,
}

/// The project folders row (read only).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsLayer {
    pub with_context: Vec<ProjectContext>,
    /// Project folders Hollow knows of.
    pub total: usize,
    /// Only the first [`PROJECTS_MAX`] were looked at.
    pub capped: bool,
    pub error: Option<String>,
}

/// The vault row: whether a vault root is saved in the sync settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultLayer {
    pub set_up: bool,
    pub enabled: bool,
    pub remote: Option<String>,
    pub text: String,
}

/// One synced folder's two sides (PX1): the vault's and the personal
/// clone's, file by file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedFolder {
    /// `me`, `memory`, `learnings` or `home`.
    pub folder: String,
    /// Files only the vault has: the first sync would send them to the repo.
    pub only_vault: Vec<String>,
    /// Files only the clone has: the first sync would write them into the vault.
    pub only_clone: Vec<String>,
    /// Files both have with different text: a clash, the vault's kept and the
    /// other's saved beside it.
    pub differ: Vec<String>,
    /// Files both have, identical.
    pub same: usize,
    /// More than [`SEED_FILES`] files on a side: the rest are not listed.
    pub capped: bool,
}

/// What the personal sync's first cycle would do (PX1), as of now. A guide,
/// not the cycle's own plan: the clone stands for the remote, which is the
/// state the clone last pulled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedReport {
    /// The personal clone the vault was compared with.
    pub clone: String,
    pub folders: Vec<SeedFolder>,
    /// The one line the row shows.
    pub text: String,
}

/// Files read per folder side at most.
const SEED_FILES: usize = 500;
/// Folders deep a synced folder is read.
const SEED_DEPTH: usize = 6;
/// Largest file compared byte for byte: a bigger one is compared by size.
const SEED_READ_MAX: u64 = 1024 * 1024;

/// Every file under `base/folder` (plain files only, no hidden names, no
/// symlinks), keyed by its path inside the folder with `/` parts. `true` when
/// [`SEED_FILES`] cut the walk short.
fn seed_files(base: &Path, folder: &str) -> (BTreeMap<String, PathBuf>, bool) {
    let mut out = BTreeMap::new();
    let root = base.join(folder);
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() && depth + 1 < SEED_DEPTH {
                stack.push((path, depth + 1));
            } else if kind.is_file() {
                if out.len() >= SEED_FILES {
                    return (out, true);
                }
                if let Ok(rel) = path.strip_prefix(&root) {
                    if let Some(rel) = rel.to_str() {
                        out.insert(rel.replace('\\', "/"), path);
                    }
                }
            }
        }
    }
    (out, false)
}

/// Whether two files hold the same bytes (a file over [`SEED_READ_MAX`] is
/// compared by size alone; one that can't be read counts as different).
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    if ma.len() > SEED_READ_MAX {
        return true;
    }
    matches!((fs::read(a), fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

/// The dry run of the personal sync's first cycle: for each synced folder
/// ([`sync_engine::PERSONAL_ALLOW`]), which files only the vault has, which
/// only the clone has and which both have with different text. Reads only.
pub(crate) fn seed_report(vault: &Path, clone: &Path) -> SeedReport {
    let mut folders = Vec::new();
    for folder in sync_engine::PERSONAL_ALLOW {
        let (mine, cut_mine) = seed_files(vault, folder);
        let (theirs, cut_theirs) = seed_files(clone, folder);
        let mut f = SeedFolder {
            folder: folder.to_string(),
            capped: cut_mine || cut_theirs,
            ..SeedFolder::default()
        };
        for (name, path) in &mine {
            match theirs.get(name) {
                None => f.only_vault.push(name.clone()),
                Some(other) if same_file(path, other) => f.same += 1,
                Some(_) => f.differ.push(name.clone()),
            }
        }
        f.only_clone = theirs
            .keys()
            .filter(|n| !mine.contains_key(*n))
            .cloned()
            .collect();
        folders.push(f);
    }
    let sum = |pick: fn(&SeedFolder) -> usize| folders.iter().map(pick).sum::<usize>();
    let (send, write, clash, same) = (
        sum(|f| f.only_vault.len()),
        sum(|f| f.only_clone.len()),
        sum(|f| f.differ.len()),
        sum(|f| f.same),
    );
    let text = if send + write + clash == 0 {
        format!("Nothing to merge: the vault and the clone hold the same {same} files.")
    } else {
        let mut parts = Vec::new();
        if send > 0 {
            parts.push(format!("send {send} file(s) from the vault to the repo"));
        }
        if write > 0 {
            parts.push(format!(
                "write {write} file(s) from the repo into the vault"
            ));
        }
        if clash > 0 {
            parts.push(format!(
                "keep both versions of {clash} file(s) that differ (a clash card each)"
            ));
        }
        format!("The first sync would {}.", parts.join(", "))
    };
    SeedReport {
        clone: clone.to_string_lossy().into_owned(),
        folders,
        text,
    }
}

/// Every row of Settings, Context.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayersStatus {
    pub home: HomeLayer,
    pub personal: SyncStatus,
    pub org: OrgLayer,
    pub projects: ProjectsLayer,
    pub vault: VaultLayer,
    /// What the personal sync's first cycle would do (PX1): only before any
    /// sync has run, and only with a personal clone on disk.
    pub seed: Option<SeedReport>,
}

/// What the org access check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessCheck {
    /// `ok`, `denied`, `offline`, `missing` (no clone to check) or `error`.
    pub state: String,
    pub text: String,
}

fn access(state: &str, text: impl Into<String>) -> AccessCheck {
    AccessCheck {
        state: state.into(),
        text: text.into(),
    }
}

/// The vault row from the sync settings: a root of the vault kind is
/// "set up" (CX6b builds it; until then its first push is held).
pub(crate) fn vault_layer(config: &sync_engine::SyncConfig) -> VaultLayer {
    match config.roots.iter().find(|r| r.kind == RootKind::Vault) {
        None => VaultLayer {
            set_up: false,
            enabled: false,
            remote: None,
            text: "Not set up".into(),
        },
        Some(r) => VaultLayer {
            set_up: true,
            enabled: r.enabled,
            remote: r.remote.as_deref().map(sync_engine::redact_urls),
            text: "Remote saved; the first push is held".into(),
        },
    }
}

/// Which context files `dir` holds (plain files only, no symlink followed).
pub(crate) fn project_files(dir: &Path) -> Vec<String> {
    PROJECT_FILES
        .iter()
        .filter(|f| fs::symlink_metadata(dir.join(f)).is_ok_and(|m| m.is_file()))
        .map(|f| f.to_string())
        .collect()
}

fn projects_layer(vault: &Path) -> ProjectsLayer {
    let projects = match crate::directory_connections::status(vault) {
        Ok(c) => c.projects,
        Err(e) => {
            return ProjectsLayer {
                error: Some(e),
                ..Default::default()
            }
        }
    };
    let total = projects.len();
    let with_context = projects
        .iter()
        .take(PROJECTS_MAX)
        .filter_map(|p| {
            let dir = Path::new(&p.path);
            let files = project_files(dir);
            (!files.is_empty()).then(|| ProjectContext {
                title: p.title.clone(),
                path: p.path.clone(),
                files,
                git: fs::symlink_metadata(dir.join(".git")).is_ok(),
            })
        })
        .collect();
    ProjectsLayer {
        with_context,
        total,
        capped: total > PROJECTS_MAX,
        error: None,
    }
}

fn iso(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()
}

async fn org_layer() -> OrgLayer {
    let root = roots::status(Layer::Org).await;
    let mut org = OrgLayer {
        root,
        remote: None,
        last_pull_at: None,
    };
    if !org.root.git_clone {
        return org;
    }
    let path = Path::new(&org.root.path).to_path_buf();
    org.last_pull_at = roots::resolve_in_root(&path, ".git/FETCH_HEAD")
        .ok()
        .and_then(|p| fs::symlink_metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .map(iso);
    use crate::build_workspaces::{git, QUICK};
    org.remote = git(&path, &["remote", "get-url", "origin"], QUICK)
        .await
        .ok()
        .filter(|u| !u.is_empty())
        .map(|u| sync_engine::redact_urls(&u));
    org
}

fn home_layer_blocking() -> (Option<String>, Option<String>) {
    let slug = roots::machine_slug();
    let suggested = if slug.is_none() {
        roots::suggested_slug()
    } else {
        None
    };
    (slug, suggested)
}

async fn home_layer() -> HomeLayer {
    let root = roots::status(Layer::Personal).await;
    let (slug, suggested_slug) = tauri::async_runtime::spawn_blocking(home_layer_blocking)
        .await
        .unwrap_or_default();
    let file = slug.as_deref().and_then(|s| roots::home_file(s).ok());
    let file_exists = match (&file, root.configured && root.exists) {
        (Some(rel), true) => roots::resolve_in_root(Path::new(&root.path), rel)
            .ok()
            .and_then(|p| fs::symlink_metadata(p).ok())
            .is_some_and(|m| m.is_file()),
        _ => false,
    };
    HomeLayer {
        slug,
        suggested_slug,
        file,
        file_exists,
        root,
    }
}

/// Every row's status. Reads only.
pub(crate) async fn layers_status(vault: std::path::PathBuf) -> Result<LayersStatus, String> {
    let personal = sync_engine::personal_status(vault.clone()).await?;
    let home = home_layer().await;
    let org = org_layer().await;
    let first_sync = personal.last_sync_at.is_none();
    let (projects, vault_row, seed) = tauri::async_runtime::spawn_blocking(move || {
        let config = sync_engine::settings_path()
            .map(|p| sync_engine::config_at(&p))
            .unwrap_or_default();
        let seed = if first_sync {
            roots::root(Layer::Personal)
                .filter(|clone| clone.is_dir())
                .map(|clone| seed_report(&vault, &clone))
        } else {
            None
        };
        (projects_layer(&vault), vault_layer(&config), seed)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(LayersStatus {
        home,
        personal,
        org,
        projects,
        vault: vault_row,
        seed,
    })
}

/// What `git ls-remote` said, in the row's words. `code` is its exit code
/// (`None`: it didn't finish).
pub(crate) fn classify_access(code: Option<i32>, stderr: &str) -> AccessCheck {
    // `--exit-code` exits 2 when the remote answers but has no HEAD yet
    // (an empty repository): the fetch itself works.
    if matches!(code, Some(0) | Some(2)) {
        return access("ok", "This Mac can fetch the org repo.");
    }
    let lower = stderr.to_ascii_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if any(&[
        "repository not found",
        "permission denied",
        "authentication failed",
        "could not read username",
        "terminal prompts disabled",
        "error: 403",
        "access denied",
        "not authorized",
    ]) {
        return access(
            "denied",
            "No access to the org repo from this Mac. Ask its owner to add you.",
        );
    }
    if any(&[
        "could not resolve host",
        "network is unreachable",
        "timed out",
        "connection refused",
        "operation timed out",
    ]) {
        return access(
            "offline",
            "Couldn't reach the org repo's host. Try again online.",
        );
    }
    let last = stderr.trim().lines().last().unwrap_or("").trim();
    let detail = if last.is_empty() {
        "git ls-remote failed".to_string()
    } else {
        sync_engine::redact_urls(last)
    };
    access("error", format!("The check failed: {detail}"))
}

/// `root/.git` is a real folder: not a symlink and not a gitfile (which
/// could point git anywhere).
pub(crate) fn is_plain_git_dir(root: &Path) -> bool {
    fs::symlink_metadata(root.join(".git")).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// Local only is refused while a cycle runs, so the cycle's own push and
/// the cleared remote never cross.
pub(crate) fn local_only_refusal(now: &SyncStatus) -> Option<String> {
    (now.state == sync_engine::SyncState::Syncing)
        .then(|| "A sync is running. Try Local only again when it finishes.".to_string())
}

/// Can this Mac fetch the org clone's origin? `git ls-remote`, no prompt
/// (ssh in batch mode, no terminal prompt), hooks and fsmonitor off.
pub(crate) async fn org_access_check() -> AccessCheck {
    let Some(root) = roots::root(Layer::Org) else {
        return access(
            "missing",
            "No org clone on this Mac yet. Choose its folder first.",
        );
    };
    if !is_plain_git_dir(&root) {
        return access("missing", "The org folder isn't a git clone.");
    }
    let args = [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.sshCommand=ssh -o BatchMode=yes",
        "ls-remote",
        "--exit-code",
        "--quiet",
        "origin",
        "HEAD",
    ];
    match crate::build_workspaces::git_raw(&root, &args, ACCESS_TIMEOUT, 64 * 1024).await {
        Ok(out) => classify_access(out.code, &out.stderr),
        Err(e) if e.contains("took longer") => access(
            "offline",
            "The org repo didn't answer in 30 seconds. Try again online.",
        ),
        Err(e) => access("error", format!("The check didn't run: {e}")),
    }
}

/// Drop the personal remote and turn sync off. Every file, the git
/// directory and the remote repository stay as they are.
pub(crate) fn personal_local_only() -> Result<(), String> {
    let _held = sync_engine::SETTINGS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    sync_engine::update_root_at(&sync_engine::settings_path()?, sync_engine::PERSONAL, |r| {
        r.remote = None;
        r.enabled = false;
        Ok(())
    })
    .map(drop)
}

fn layer_of(text: &str) -> Result<Layer, String> {
    match text {
        "personal" => Ok(Layer::Personal),
        "org" => Ok(Layer::Org),
        other => Err(format!("No context layer named {other:?}.")),
    }
}

// ── Commands ────────────────────────────────────────────────────────────

/// Settings, Context: every row's status. Reads only.
#[tauri::command]
pub async fn context_layers_status(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<LayersStatus, String> {
    layers_status(state.config.vault_path.clone()).await
}

/// The org row's Check access (the person's button).
#[tauri::command]
pub async fn context_org_access_check() -> Result<AccessCheck, String> {
    Ok(org_access_check().await)
}

/// The personal row's Local only: no remote, sync off, files kept.
#[tauri::command]
pub async fn context_personal_local_only(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<SyncStatus, String> {
    let now = sync_engine::personal_status(state.config.vault_path.clone()).await?;
    if let Some(why) = local_only_refusal(&now) {
        return Err(why);
    }
    tauri::async_runtime::spawn_blocking(personal_local_only)
        .await
        .map_err(|e| e.to_string())??;
    sync_engine::personal_status(state.config.vault_path.clone()).await
}

/// Save (or, with no path, clear) the personal or org clone's folder.
#[tauri::command]
pub async fn set_context_root(layer: String, path: Option<String>) -> Result<RootStatus, String> {
    let layer = layer_of(&layer)?;
    tauri::async_runtime::spawn_blocking(move || roots::set_root(layer, path.as_deref()))
        .await
        .map_err(|e| e.to_string())??;
    Ok(roots::status(layer).await)
}

/// Save (or, with none, clear) this Mac's slug. Automations whose owner
/// (`runs_on`) named the old slug move to the new one, or to this Mac's id
/// when it is cleared (M4, `runs_on::after_slug_change`).
#[tauri::command]
pub async fn set_context_machine_slug(
    state: tauri::State<'_, crate::commands::AppState>,
    slug: Option<String>,
) -> Result<Option<String>, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || save_machine_slug(&vault, slug.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

/// Save (or, with none, clear) this Mac's slug and move the automations
/// that named the old one ([`set_context_machine_slug`]; F3's Connect
/// names this Mac through it too).
pub(crate) fn save_machine_slug(
    vault: &Path,
    slug: Option<&str>,
) -> Result<Option<String>, String> {
    let slug = slug.map(str::trim).filter(|s| !s.is_empty());
    let old = roots::machine_slug();
    let saved = roots::set_machine_slug(slug)?;
    let id = roots::ensure_machine_id().ok();
    let moved =
        crate::runs_on::after_slug_change(vault, old.as_deref(), saved.as_deref(), id.as_deref());
    if !moved.is_empty() {
        eprintln!(
            "[runs_on] moved to this Mac's new name: {}",
            moved.join(", ")
        );
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_engine::{RootConfig, SyncConfig};

    #[test]
    fn the_access_check_reads_git_in_plain_words() {
        assert_eq!(classify_access(Some(0), "").state, "ok");
        assert_eq!(classify_access(Some(2), "").state, "ok");
        for denied in [
            "ERROR: Repository not found.",
            "git@github.com: Permission denied (publickey).",
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            "remote: Write access to repository not granted. The requested URL returned error: 403",
        ] {
            assert_eq!(
                classify_access(Some(128), denied).state,
                "denied",
                "{denied}"
            );
        }
        assert_eq!(
            classify_access(Some(128), "ssh: Could not resolve hostname github.com: nodename nor servname provided\nfatal: Could not resolve host: github.com").state,
            "offline"
        );
        let other = classify_access(Some(128), "fatal: https://user:tok@example.com/x.git broke");
        assert_eq!(other.state, "error");
        assert!(!other.text.contains("tok"), "{}", other.text);
        assert!(classify_access(None, "").text.contains("ls-remote"));
        assert_eq!(
            classify_access(Some(128), "fatal: read 4031 bytes, then hung up").state,
            "error"
        );
    }

    #[test]
    fn the_vault_row_says_not_set_up_until_a_vault_root_is_saved() {
        let none = vault_layer(&SyncConfig::default());
        assert!(!none.set_up);
        assert_eq!(none.text, "Not set up");
        let mut config = SyncConfig::default();
        config.roots.push(RootConfig {
            id: "vault".into(),
            kind: RootKind::Vault,
            remote: Some("https://tok@example.com/v.git".into()),
            branch: "main".into(),
            enabled: true,
        });
        let row = vault_layer(&config);
        assert!(row.set_up && row.enabled);
        assert_eq!(row.remote.as_deref(), Some("https://***@example.com/v.git"));
        assert!(row.text.contains("held"));
    }

    #[test]
    fn a_project_carries_context_by_its_plain_files_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(project_files(dir.path()).is_empty());
        fs::write(dir.path().join("AGENTS.md"), "x").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", dir.path().join("CLAUDE.md")).unwrap();
        assert_eq!(project_files(dir.path()), vec!["AGENTS.md".to_string()]);
        fs::remove_file(dir.path().join("CLAUDE.md")).unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "x").unwrap();
        assert_eq!(
            project_files(dir.path()),
            vec!["CLAUDE.md".to_string(), "AGENTS.md".to_string()]
        );
    }

    #[test]
    fn only_a_real_git_folder_counts_as_a_clone() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_plain_git_dir(dir.path()));
        fs::write(dir.path().join(".git"), "gitdir: /elsewhere").unwrap();
        assert!(!is_plain_git_dir(dir.path()), "a gitfile");
        fs::remove_file(dir.path().join(".git")).unwrap();
        let real = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(real.path(), dir.path().join(".git")).unwrap();
        assert!(!is_plain_git_dir(dir.path()), "a symlink");
        fs::remove_file(dir.path().join(".git")).unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(is_plain_git_dir(dir.path()));
    }

    #[test]
    fn urls_never_show_a_secret() {
        use crate::sync_engine::redact_urls;
        assert_eq!(
            redact_urls("https://tok@github.com/a.git"),
            "https://***@github.com/a.git"
        );
        assert_eq!(
            redact_urls("ssh://me:pw@host/a.git"),
            "ssh://***@host/a.git"
        );
        assert_eq!(
            redact_urls("git+ssh://me:pw@host/a"),
            "git+ssh://***@host/a"
        );
        assert_eq!(
            redact_urls("ssh://git@github.com/a.git"),
            "ssh://git@github.com/a.git"
        );
        assert_eq!(
            redact_urls("fatal: https://h/a.git?token=abc123&x=1 failed"),
            "fatal: https://h/a.git?token=***&x=1 failed"
        );
        assert_eq!(
            redact_urls("https://h/a?ACCESS_TOKEN=s3cret"),
            "https://h/a?ACCESS_TOKEN=***"
        );
        assert_eq!(
            redact_urls("https://h/a?page=2&key=k"),
            "https://h/a?page=2&key=***"
        );
        assert_eq!(
            redact_urls("git@github.com:me/a.git"),
            "git@github.com:me/a.git"
        );
        assert_eq!(redact_urls("no url here"), "no url here");
    }

    #[test]
    fn local_only_waits_for_a_running_sync() {
        let mut now = SyncStatus {
            root: "personal".into(),
            state: sync_engine::SyncState::Syncing,
            enabled: true,
            remote: Some("git@h:a.git".into()),
            pending_commits: 0,
            unsynced_files: 0,
            text: "Syncing".into(),
            last_sync_at: None,
            last_error: None,
            blocked_by: None,
            clash_copies: Vec::new(),
            clashes: 0,
        };
        assert!(local_only_refusal(&now).is_some());
        now.state = sync_engine::SyncState::Idle;
        assert!(local_only_refusal(&now).is_none());
    }

    #[test]
    fn a_layer_name_is_personal_or_org() {
        assert_eq!(layer_of("org").unwrap(), Layer::Org);
        assert_eq!(layer_of("personal").unwrap(), Layer::Personal);
        assert!(layer_of("vault").is_err());
    }

    /// A vault and a personal clone with the same `me/` and `learnings/` files.
    fn seed_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let (vault, clone) = (base.join("vault"), base.join("clone"));
        for side in [&vault, &clone] {
            fs::create_dir_all(side.join("me")).unwrap();
            fs::create_dir_all(side.join("learnings")).unwrap();
            fs::write(side.join("me/Identity.md"), "same\n").unwrap();
            fs::write(side.join("learnings/shared.md"), "same\n").unwrap();
        }
        (dir, vault, clone)
    }

    fn folder<'a>(r: &'a SeedReport, name: &str) -> &'a SeedFolder {
        r.folders.iter().find(|f| f.folder == name).unwrap()
    }

    #[test]
    fn matching_sides_have_nothing_to_merge() {
        let (_dir, vault, clone) = seed_fixture();
        let r = seed_report(&vault, &clone);
        assert_eq!(
            r.folders
                .iter()
                .map(|f| f.folder.as_str())
                .collect::<Vec<_>>(),
            ["me", "memory", "learnings", "home"]
        );
        assert!(
            r.text
                .starts_with("Nothing to merge: the vault and the clone hold the same 2 files"),
            "{}",
            r.text
        );
        assert_eq!(folder(&r, "me").same, 1);
        assert!(r
            .folders
            .iter()
            .all(|f| f.only_vault.is_empty() && f.only_clone.is_empty() && f.differ.is_empty()));
    }

    #[test]
    fn each_side_new_and_clashing_files_are_named_by_folder() {
        let (_dir, vault, clone) = seed_fixture();
        // The vault has learnings the repo lacks; the repo has memory and a
        // home file the vault lacks; one file differs; a nested one counts.
        for n in ["a.md", "b.md"] {
            fs::write(vault.join("learnings").join(n), "x").unwrap();
        }
        fs::create_dir_all(clone.join("memory/-Users-x")).unwrap();
        fs::write(clone.join("memory/rule.md"), "r").unwrap();
        fs::write(clone.join("memory/-Users-x/deep.md"), "d").unwrap();
        fs::create_dir_all(clone.join("home")).unwrap();
        fs::write(clone.join("home/mac.md"), "h").unwrap();
        fs::write(clone.join("me/Identity.md"), "changed\n").unwrap();
        // Hidden files and symlinks are not read.
        fs::write(vault.join("learnings/.DS_Store"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(vault.join("me/Identity.md"), vault.join("me/link.md")).unwrap();
        let r = seed_report(&vault, &clone);
        assert_eq!(folder(&r, "learnings").only_vault, ["a.md", "b.md"]);
        assert_eq!(folder(&r, "learnings").same, 1);
        assert_eq!(
            folder(&r, "memory").only_clone,
            ["-Users-x/deep.md", "rule.md"]
        );
        assert_eq!(folder(&r, "home").only_clone, ["mac.md"]);
        assert_eq!(folder(&r, "me").differ, ["Identity.md"]);
        assert!(
            folder(&r, "me").only_vault.is_empty(),
            "the symlink is skipped"
        );
        assert_eq!(
            r.text,
            "The first sync would send 2 file(s) from the vault to the repo, write 3 file(s) from the repo into the vault, keep both versions of 1 file(s) that differ (a clash card each)."
        );
        // Reads only: nothing was written on either side.
        assert!(!vault.join("memory").exists());
        assert!(!clone.join("learnings/a.md").exists());
    }
}
