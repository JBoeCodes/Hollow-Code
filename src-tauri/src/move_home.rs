//! Move Hollow home: make `<hollowHome>` (default `~/Hollow`) one real
//! folder holding `Vault`, `Projects` (with `Projects/Personal`),
//! `Repositories`, `Worktrees`, `Archive` and `Backups`, and re-key every
//! store that names the old places.
//!
//! Three parts, each with its own file:
//! - the dry run ([`plan`], `move_home_plan.rs`): read-only; what moves,
//!   what is re-keyed, collisions, iCloud-evicted (dataless) files, running
//!   chats, processes, git worktree repairs, backup status;
//! - the run (`move_home_run.rs`): two processes. The app stages the plan
//!   ([`start`]) under `<hollowHome>/Backups/move-home-<ts>/`, spawns this
//!   binary detached (`move-home --run <plan.json> --wait-pid <pid>
//!   --relaunch <app> --yes`) and quits; the helper waits for Hollow to
//!   exit and its Sessions database to close, executes exactly the plan
//!   (renames on one volume with compat symlinks at the old paths, re-keys on
//!   copies then swaps, git worktree repair, never prune), verifies and
//!   relaunches Hollow;
//! - rollback ([`stage_rollback`]), from the journal, also after a crash.
//!
//! The pure path, stem and id functions are in `move_home_rekey.rs`.
//! Every path derives from config and `$HOME` ([`Roots`]); tests build
//! their own roots under a temp folder and never fall back to real ones.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use crate::move_home_plan::plan_with;
pub use crate::move_home_run::{rollback_with, run_with, unfinished, RunHooks, Unfinished};

/// Plan file format version.
pub const PLAN_VERSION: u32 = 2;
/// The macOS flag of an iCloud-evicted file (`SF_DATALESS`).
pub const SF_DATALESS: u32 = 0x4000_0000;

// ── IPC shapes ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    /// Folders beside the vault join the plan (each to `Projects/<name>`).
    pub include_vault_siblings: bool,
    /// Beside-the-vault folders that go to `Projects/Personal/<name>`.
    pub personal: Vec<String>,
    /// LaunchAgent labels whose plists are rewritten (bootout, bootstrap).
    pub launch_agents: Vec<String>,
    /// Scripts (full path, or a name in `~/.local/bin`) that are rewritten.
    pub scripts: Vec<String>,
    /// Allow plan paths outside `$HOME`.
    pub allow_outside_home: bool,
    /// The dry run's time budget for walking the moved folders (seconds).
    pub budget_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MoveKind {
    Vault,
    ProjectsRoot,
    Project,
    Personal,
    Archive,
}

pub const METHOD_RENAME: &str = "rename";
pub const METHOD_COPY: &str = "copy";

fn default_method() -> String {
    METHOD_RENAME.into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlannedMove {
    /// Where it is now (original terms, even when an earlier move carries
    /// its parent).
    pub from: String,
    pub to: String,
    pub kind: MoveKind,
    pub bytes: u64,
    pub files: u64,
    pub dataless: u64,
    /// Bytes of the evicted files (their size once downloaded).
    #[serde(default)]
    pub dataless_bytes: u64,
    /// `rename` (same-volume rename) or `copy` (a folder in iCloud: clone,
    /// verify, rename into place, remove the original last).
    #[serde(default = "default_method")]
    pub method: String,
    /// The symlink at `to` this move replaces (its target as read).
    pub replaces_symlink: Option<String>,
    /// A symlink is left at `from` pointing to `to`.
    pub compat_symlink: bool,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoreRekey {
    pub from: String,
    pub to: String,
    pub what: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnRewrite {
    pub table: String,
    pub column: String,
    pub rows: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileRewrite {
    /// The file where it is now (original terms).
    pub path: String,
    pub changes: u64,
    pub what: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdChange {
    pub old: String,
    pub new: String,
    pub project_from: String,
    pub project_to: String,
    pub has_workspace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Collision {
    pub kind: String,
    pub path: String,
    pub detail: String,
    pub winner: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatalessRoot {
    pub root: String,
    pub count: u64,
    #[serde(default)]
    pub bytes: u64,
    pub top: Vec<(String, u64)>,
    pub sample: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatalessReport {
    pub total: u64,
    /// Bytes the evicted files take once downloaded.
    #[serde(default)]
    pub bytes: u64,
    /// Evicted files in folders that move by copy: they must be downloaded
    /// first (`Hollow move-home --download`), and what that takes.
    #[serde(default)]
    pub dirs: u64,
    #[serde(default)]
    pub to_download: u64,
    #[serde(default)]
    pub to_download_bytes: u64,
    #[serde(default)]
    pub download_secs: u64,
    /// Folders that move by copy and roughly how long the copy takes.
    #[serde(default)]
    pub copy_folders: Vec<String>,
    #[serde(default)]
    pub copy_secs: u64,
    pub roots: Vec<DatalessRoot>,
    pub file_provider_domain: Option<String>,
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Blocker {
    pub kind: String,
    pub detail: String,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoRepair {
    pub repo_from: String,
    pub repo_to: String,
    pub worktrees: u64,
    pub broken: u64,
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentStoreChange {
    pub agent: String,
    pub path: String,
    pub changes: u64,
    pub dead: u64,
    pub merges: u64,
    pub ambiguous: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchItem {
    pub label: String,
    pub path: String,
    pub mentions: Vec<String>,
    pub on_plan: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupStatus {
    pub covered: Option<bool>,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub version: u32,
    pub created: String,
    pub hash: String,
    pub ready: bool,
    pub partial: bool,
    pub hollow_home: String,
    pub vault_from: String,
    pub vault_to: String,
    pub old_stem: String,
    pub new_stem: String,
    pub moves: Vec<PlannedMove>,
    pub mkdirs: Vec<String>,
    pub mappings: Vec<(String, String)>,
    pub beside_vault: Vec<String>,
    pub stores: Vec<StoreRekey>,
    pub columns: Vec<ColumnRewrite>,
    pub files: Vec<FileRewrite>,
    pub ids: Vec<IdChange>,
    pub orphans: Vec<String>,
    pub collisions: Vec<Collision>,
    pub dataless: DatalessReport,
    pub same_volume: bool,
    pub free_bytes: u64,
    pub total_bytes: u64,
    pub blockers: Vec<Blocker>,
    pub processes: Vec<String>,
    pub launch_agents: Vec<LaunchItem>,
    pub scripts: Vec<LaunchItem>,
    pub repos: Vec<RepoRepair>,
    pub agents: Vec<AgentStoreChange>,
    pub backup: BackupStatus,
    pub notes: Vec<String>,
    pub text: Vec<String>,
}

impl Plan {
    /// Nothing to move or re-key: already home.
    pub fn is_noop(&self) -> bool {
        self.moves.is_empty()
            && self.stores.is_empty()
            && self.ids.is_empty()
            && self.files.iter().all(|f| f.changes == 0)
            && self.columns.iter().all(|c| c.rows == 0)
            && !self.launch_agents.iter().any(|l| l.on_plan)
            && !self.scripts.iter().any(|l| l.on_plan)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub step: String,
    /// `started`, `done`, `failed` or `undone`.
    pub state: String,
    pub detail: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `none`, `staged`, `running`, `done`, `failed` or `rolledBack`.
    pub state: String,
    pub backup_dir: Option<String>,
    pub steps: Vec<Step>,
    pub checks: Vec<Check>,
    pub last_result: Option<String>,
    pub can_roll_back: bool,
    pub rollback_refusal: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Started {
    pub backup_dir: String,
    pub plan_file: String,
    pub helper_pid: u32,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RollbackStaged {
    pub staged: bool,
    pub backup_dir: String,
    pub note: String,
}

// ── Roots ─────────────────────────────────────────────────────────────────

/// Every folder the move reads or writes, from config and `$HOME` only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Roots {
    /// `$HOME`.
    pub home: PathBuf,
    /// Hollow's config.json.
    pub config_path: PathBuf,
    /// The app support folder (`Sessions/`, `MainAgent/`, ...).
    pub support: PathBuf,
    /// The app data folder (`build-workspaces.json`, ...).
    pub app_data: PathBuf,
    /// The default Hollow folder name under `$HOME` (`Hollow`).
    pub home_name: String,
    /// `vaultPath` as configured.
    pub vault: PathBuf,
    /// `projectsPath` as configured.
    pub projects: Option<PathBuf>,
    /// `hollowHome` as configured.
    pub hollow_home: Option<PathBuf>,
}

impl Roots {
    /// Roots from explicit folders and the config file at `config_path`.
    pub fn with(
        home: &Path,
        config_path: &Path,
        support: &Path,
        app_data: &Path,
        home_name: &str,
    ) -> Result<Roots, String> {
        let map = crate::setup::load_config_map(config_path)?;
        let get = |key: &str| {
            map.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| expand(s, home))
        };
        let vault = get("vaultPath")
            .ok_or("Hollow's config has no vaultPath, so there is no vault to move.")?;
        Ok(Roots {
            home: home.to_path_buf(),
            config_path: config_path.to_path_buf(),
            support: support.to_path_buf(),
            app_data: app_data.to_path_buf(),
            home_name: home_name.to_string(),
            vault,
            projects: get("projectsPath"),
            hollow_home: get("hollowHome"),
        })
    }

    /// The app's roots from its loaded config file.
    #[cfg(not(test))]
    pub fn from_config(config_path: &Path) -> Result<Roots, String> {
        let home = dirs::home_dir().ok_or("Hollow can't find your home folder.")?;
        let support =
            crate::instance::support_dir().ok_or("Hollow can't find its app support folder.")?;
        let app_data = crate::agent_settings::app_data_dir()?;
        Roots::with(
            &home,
            config_path,
            &support,
            &app_data,
            &crate::instance::current().home_name(),
        )
    }

    /// Test builds never resolve the real folders.
    #[cfg(test)]
    pub fn from_config(_config_path: &Path) -> Result<Roots, String> {
        Err("Roots::from_config is not available in tests".into())
    }

    /// The CLI's roots: the config file Hollow itself would load.
    #[cfg(not(test))]
    pub fn locate() -> Result<Roots, String> {
        let (_, path) = crate::config::locate_and_load()?;
        Roots::from_config(&path)
    }

    #[cfg(test)]
    pub fn locate() -> Result<Roots, String> {
        Err("Roots::locate is not available in tests".into())
    }

    /// `<hollowHome>`, else `$HOME/<home name>`.
    pub fn hollow_home_path(&self) -> PathBuf {
        self.hollow_home
            .clone()
            .unwrap_or_else(|| self.home.join(&self.home_name))
    }
    pub fn backups(&self) -> PathBuf {
        self.hollow_home_path().join("Backups")
    }
    pub fn claude_projects(&self) -> PathBuf {
        self.home.join(".claude").join("projects")
    }
    pub fn claude_json(&self) -> PathBuf {
        self.home.join(".claude.json")
    }
    pub fn codex_config(&self) -> PathBuf {
        self.home.join(".codex").join("config.toml")
    }
    pub fn gemini_projects(&self) -> PathBuf {
        self.home.join(".gemini").join("projects.json")
    }
    pub fn claude_desktop(&self) -> PathBuf {
        self.home
            .join("Library/Application Support/Claude/claude_desktop_config.json")
    }
    pub fn obsidian(&self) -> PathBuf {
        self.home
            .join("Library/Application Support/obsidian/obsidian.json")
    }
    pub fn launch_agents(&self) -> PathBuf {
        self.home.join("Library").join("LaunchAgents")
    }
    pub fn local_bin(&self) -> PathBuf {
        self.home.join(".local").join("bin")
    }
}

fn expand(s: &str, home: &Path) -> PathBuf {
    if s == "~" {
        home.to_path_buf()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(s)
    }
}

// ── Probes ────────────────────────────────────────────────────────────────

/// Everything the move asks the system that isn't a plain file read; tests
/// stub it.
pub trait Probes: Send + Sync {
    /// `st_flags` of an entry (lstat).
    fn flags(&self, path: &Path, meta: &std::fs::Metadata) -> u32;
    /// Processes that may hold the roots (name, pid and why).
    fn processes(&self, roots: &[PathBuf]) -> Vec<String>;
    /// The File Provider domain `path` (or a parent) belongs to.
    fn file_provider_domain(&self, path: &Path) -> Option<String>;
    /// Whether Time Machine covers `path`, read-only.
    fn time_machine(&self, path: &Path) -> BackupStatus;
    /// A launchd job lists `dir`: Ok(listing head) or why not. `Ok(None)`
    /// when launchctl isn't available here.
    fn launchd_probe(&self, dir: &Path, scratch: &Path) -> Result<Option<String>, String>;
    fn bootout(&self, label: &str) -> Result<(), String>;
    fn bootstrap(&self, plist: &Path) -> Result<(), String>;
    /// Claude Code and Codex CLI processes running now ("claude (pid N)"),
    /// leaving out this process and those descended from `not_under` (the
    /// Hollow app, whose chats end when it quits).
    fn agent_clis(&self, not_under: Option<u32>) -> Vec<String>;
}

/// The executable of `pid` (macOS `proc_pidpath`).
fn exe_of(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; 4096];
        // SAFETY: a buffer of the length passed; proc_pidpath writes at most
        // that many bytes and returns how many.
        let n = unsafe {
            libc::proc_pidpath(
                pid as i32,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len() as u32,
            )
        };
        (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}

/// `claude` or `codex` when this process is one of their CLIs (the native
/// binaries, which retitle themselves, or the npm packages under node).
pub(crate) fn agent_cli_name(exe: &str, args: &str) -> Option<&'static str> {
    let base = Path::new(exe)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if base == "claude" || exe.contains("/claude/versions/") {
        return Some("claude");
    }
    if base == "codex" || base.starts_with("codex-") {
        return Some("codex");
    }
    if matches!(base.as_str(), "node" | "bun") {
        if args.contains("@anthropic-ai/claude-code") || args.contains("/bin/claude") {
            return Some("claude");
        }
        if args.contains("@openai/codex") || args.contains("/bin/codex") {
            return Some("codex");
        }
    }
    None
}

/// The real system.
pub struct SystemProbes;

fn run_capped(cmd: &mut std::process::Command, secs: u64) -> Option<(i32, String, String)> {
    use std::io::Read;
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let mut err = child.stderr.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let ereader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&ereader.join().unwrap_or_default()).into_owned();
    Some((code, out, err))
}

const WATCHED: [&str; 5] = ["claude", "codex", "Hollow", "Obsidian", "node"];

impl Probes for SystemProbes {
    fn agent_clis(&self, not_under: Option<u32>) -> Vec<String> {
        let Some((_, text, _)) = run_capped(
            std::process::Command::new("ps").args(["-axo", "pid=,ppid=,args="]),
            10,
        ) else {
            return Vec::new();
        };
        let mut parent = std::collections::HashMap::new();
        let mut rows = Vec::new();
        for line in text.lines() {
            let mut it = line.split_whitespace();
            let (Some(pid), Some(ppid)) = (it.next(), it.next()) else {
                continue;
            };
            let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
                continue;
            };
            parent.insert(pid, ppid);
            rows.push((pid, it.collect::<Vec<_>>().join(" ")));
        }
        let me = std::process::id();
        let under = |mut pid: u32, root: u32| {
            for _ in 0..64 {
                if pid == root {
                    return true;
                }
                match parent.get(&pid) {
                    Some(&p) if p > 1 && p != pid => pid = p,
                    _ => return false,
                }
            }
            false
        };
        rows.into_iter()
            .filter(|(pid, _)| *pid != me && !not_under.is_some_and(|root| under(*pid, root)))
            .filter_map(|(pid, args)| {
                let exe =
                    exe_of(pid).unwrap_or_else(|| args.split(' ').next().unwrap_or("").to_string());
                agent_cli_name(&exe, &args).map(|name| format!("{name} (pid {pid})"))
            })
            .collect()
    }

    fn flags(&self, _path: &Path, meta: &std::fs::Metadata) -> u32 {
        #[cfg(target_os = "macos")]
        {
            use std::os::macos::fs::MetadataExt;
            meta.st_flags()
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = meta;
            0
        }
    }

    fn processes(&self, roots: &[PathBuf]) -> Vec<String> {
        let me = std::process::id().to_string();
        let mut out = Vec::new();
        // Working folders under a root (lsof's cwd list).
        if let Some((_, text, _)) = run_capped(
            std::process::Command::new("lsof").args(["-nP", "-a", "-d", "cwd", "-Fpcn"]),
            20,
        ) {
            let (mut pid, mut cmd) = (String::new(), String::new());
            for line in text.lines() {
                if let Some(p) = line.strip_prefix('p') {
                    pid = p.to_string();
                } else if let Some(c) = line.strip_prefix('c') {
                    cmd = c.to_string();
                } else if let Some(n) = line.strip_prefix('n') {
                    let path = Path::new(n);
                    if pid != me && roots.iter().any(|r| path.starts_with(r)) {
                        out.push(format!("{cmd} (pid {pid}) works in {n}"));
                    }
                }
            }
        }
        if let Some((_, text, _)) = run_capped(
            std::process::Command::new("ps").args(["-axo", "pid=,comm="]),
            10,
        ) {
            for line in text.lines() {
                let mut parts = line.trim().splitn(2, ' ');
                let (Some(pid), Some(comm)) = (parts.next(), parts.next()) else {
                    continue;
                };
                let name = Path::new(comm.trim())
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if pid != me
                    && WATCHED.contains(&name.as_str())
                    && name != "node"
                    && !out.iter().any(|o| o.contains(&format!("(pid {pid})")))
                {
                    out.push(format!("{name} (pid {pid}) is running"));
                }
            }
        }
        out
    }

    fn file_provider_domain(&self, path: &Path) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;
            let name = CString::new("com.apple.file-provider-domain-id").ok()?;
            let mut cur = Some(path);
            while let Some(p) = cur {
                let c = CString::new(p.as_os_str().as_bytes()).ok()?;
                let mut buf = vec![0u8; 1024];
                // SAFETY: valid NUL-terminated strings and a buffer of the
                // length passed; getxattr writes at most that many bytes.
                let n = unsafe {
                    libc::getxattr(
                        c.as_ptr(),
                        name.as_ptr(),
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                        0,
                        libc::XATTR_NOFOLLOW,
                    )
                };
                if n > 0 {
                    buf.truncate(n as usize);
                    return Some(
                        String::from_utf8_lossy(&buf)
                            .trim_end_matches('\0')
                            .to_string(),
                    );
                }
                cur = p.parent();
            }
            None
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            None
        }
    }

    fn time_machine(&self, path: &Path) -> BackupStatus {
        let Some((_, dest, _)) = run_capped(
            std::process::Command::new("tmutil").arg("destinationinfo"),
            15,
        ) else {
            return BackupStatus {
                covered: None,
                detail: "Time Machine status is unknown (tmutil didn't answer).".into(),
            };
        };
        if dest.contains("No destinations configured") || !dest.contains("Name") {
            return BackupStatus {
                covered: Some(false),
                detail: "No Time Machine destination is set up.".into(),
            };
        }
        match run_capped(
            std::process::Command::new("tmutil")
                .arg("isexcluded")
                .arg(path),
            15,
        ) {
            Some((_, text, _)) if text.contains("[Excluded]") => BackupStatus {
                covered: Some(false),
                detail: format!("Time Machine excludes {}.", path.display()),
            },
            Some((_, text, _)) if text.contains("[Included]") => BackupStatus {
                covered: Some(true),
                detail: format!("Time Machine has a destination and includes {}.", path.display()),
            },
            _ => BackupStatus {
                covered: None,
                detail: "Time Machine has a destination; whether it includes the Hollow folder is unknown.".into(),
            },
        }
    }

    fn launchd_probe(&self, dir: &Path, scratch: &Path) -> Result<Option<String>, String> {
        if !Path::new("/bin/launchctl").exists() {
            return Ok(None);
        }
        let label = format!("com.hollow.move-home-probe.{}", std::process::id());
        let out = scratch.join("launchd-probe.out");
        let err = scratch.join("launchd-probe.err");
        let _ = std::fs::remove_file(&out);
        let _ = std::fs::remove_file(&err);
        let submitted = run_capped(
            std::process::Command::new("/bin/launchctl")
                .args(["submit", "-l", &label, "-o"])
                .arg(&out)
                .arg("-e")
                .arg(&err)
                .arg("--")
                .arg("/bin/ls")
                .arg(dir),
            10,
        );
        let result = match submitted {
            Some((0, _, _)) => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
                loop {
                    let o = std::fs::read_to_string(&out).unwrap_or_default();
                    let e = std::fs::read_to_string(&err).unwrap_or_default();
                    if !e.trim().is_empty() {
                        break Err(format!("the launchd job couldn't list it: {}", e.trim()));
                    }
                    if !o.trim().is_empty() {
                        break Ok(Some(o.lines().take(3).collect::<Vec<_>>().join(", ")));
                    }
                    if std::time::Instant::now() > deadline {
                        break Err("the launchd job wrote nothing within 8 s".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
            Some((code, _, e)) => Err(format!("launchctl submit failed ({code}): {}", e.trim())),
            None => Err("launchctl submit didn't answer".into()),
        };
        let _ = run_capped(
            std::process::Command::new("/bin/launchctl").args(["remove", &label]),
            10,
        );
        result
    }

    fn bootout(&self, label: &str) -> Result<(), String> {
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        match run_capped(
            std::process::Command::new("/bin/launchctl")
                .arg("bootout")
                .arg(format!("gui/{uid}/{label}")),
            20,
        ) {
            // 3 / 113: not loaded, which is the state we want.
            Some((0 | 3 | 113, _, _)) => Ok(()),
            Some((code, _, e)) => Err(format!(
                "launchctl bootout {label} failed ({code}): {}",
                e.trim()
            )),
            None => Err(format!("launchctl bootout {label} didn't answer")),
        }
    }

    fn bootstrap(&self, plist: &Path) -> Result<(), String> {
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        match run_capped(
            std::process::Command::new("/bin/launchctl")
                .arg("bootstrap")
                .arg(format!("gui/{uid}"))
                .arg(plist),
            20,
        ) {
            Some((0, _, _)) => Ok(()),
            Some((code, _, e)) => Err(format!(
                "launchctl bootstrap {} failed ({code}): {}",
                plist.display(),
                e.trim()
            )),
            None => Err("launchctl bootstrap didn't answer".into()),
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────

/// The dry run: what a move would do now. Reads only.
pub fn plan(
    roots: &Roots,
    options: &Options,
    caller_session: Option<&str>,
) -> Result<Plan, String> {
    plan_with(roots, options, caller_session, &SystemProbes)
}

/// The readable report.
pub fn describe(plan: &Plan) -> Vec<String> {
    crate::move_home_plan::describe(plan)
}

/// Re-plan, refuse unless `hash` matches and the plan is ready, stage it
/// under `<hollowHome>/Backups/move-home-<ts>/` and spawn the helper. The
/// caller quits Hollow afterwards.
pub fn start(
    roots: &Roots,
    options: &Options,
    hash: &str,
    caller: Option<&str>,
) -> Result<Started, String> {
    let staged = crate::move_home_run::stage(roots, options, hash, caller, &SystemProbes)?;
    let pid = spawn_helper(
        &[
            "--run".into(),
            staged.plan_file.to_string_lossy().into_owned(),
        ],
        &staged.backup_dir,
    )?;
    Ok(Started {
        backup_dir: staged.backup_dir.to_string_lossy().into_owned(),
        plan_file: staged.plan_file.to_string_lossy().into_owned(),
        helper_pid: pid,
        note: "Hollow quits in a moment; the move runs while it is closed and Hollow opens again on the new folders. This turn ends with the quit.".into(),
    })
}

/// Where the move stands: the newest `move-home-*` folder's journal.
pub fn status(roots: &Roots) -> Status {
    crate::move_home_run::status(roots)
}

/// Stage a rollback of `backup_dir` (default: the newest move) and spawn
/// the helper; refused with the reason once new work landed after the move.
pub fn stage_rollback(
    roots: &Roots,
    backup_dir: Option<PathBuf>,
) -> Result<RollbackStaged, String> {
    let dir = crate::move_home_run::check_rollback(roots, backup_dir)?;
    spawn_helper(
        &["--rollback".into(), dir.to_string_lossy().into_owned()],
        &dir,
    )?;
    Ok(RollbackStaged {
        staged: true,
        backup_dir: dir.to_string_lossy().into_owned(),
        note: "Hollow quits in a moment; the move is undone while it is closed and Hollow opens again on the old folders.".into(),
    })
}

/// The `.app` this binary runs from, if any.
fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
}

/// This binary, detached (`setsid`), as `move-home <args> --wait-pid <me>
/// --relaunch <app> --yes`, logging to the backup folder.
#[cfg(not(test))]
fn spawn_helper(args: &[String], backup_dir: &Path) -> Result<u32, String> {
    use std::os::unix::process::CommandExt;
    let exe =
        std::env::current_exe().map_err(|e| format!("Hollow can't find its own program: {e}"))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(backup_dir.join("helper.log"))
        .map_err(|e| format!("couldn't open the helper log: {e}"))?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("move-home")
        .args(args)
        .arg("--wait-pid")
        .arg(std::process::id().to_string());
    if let Some(app) = app_bundle() {
        cmd.arg("--relaunch").arg(app);
    }
    cmd.arg("--yes")
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log2);
    // SAFETY: setsid is async-signal-safe and touches no memory of ours.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start the move helper: {e}"))?;
    Ok(child.id())
}

#[cfg(test)]
fn spawn_helper(_args: &[String], _backup_dir: &Path) -> Result<u32, String> {
    let _ = app_bundle;
    Err("the move helper is never spawned in tests".into())
}

// ── CLI ───────────────────────────────────────────────────────────────────

/// The folders `plan` moves by copy that are not inside another move.
pub fn copy_roots(plan: &Plan) -> Vec<PathBuf> {
    plan.moves
        .iter()
        .enumerate()
        .filter(|(i, m)| {
            m.method == METHOD_COPY
                && !plan.moves[..*i]
                    .iter()
                    .any(|o| m.from.starts_with(&format!("{}/", o.from)))
        })
        .map(|(_, m)| PathBuf::from(&m.from))
        .collect()
}

/// `Hollow move-home --download`: plan, then download every evicted file of
/// the copy folders with progress lines. Exit 0 when none are left.
fn download_cli(roots: &Roots, options: &Options) -> i32 {
    let plan = match plan(roots, options, None) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let folders = copy_roots(&plan);
    let d = &plan.dataless;
    if folders.is_empty() {
        println!("Nothing to download: no folder moves by copy.");
        return 0;
    }
    // The plan's walk may have stopped early (plan.partial): its counts are a
    // floor, so the download always walks the folders itself.
    println!(
        "Downloading the iCloud-only files of {} folder(s): the check found {} ({}){}, about {} at the measured speeds (small files {} a second, large ones {} MB/s). Stop with ctrl+c and run it again to continue.",
        folders.len(),
        d.to_download,
        crate::move_home_plan::size(d.to_download_bytes),
        if plan.partial { ", a partial count" } else { "" },
        crate::move_home_icloud::duration_text(d.download_secs),
        crate::move_home_icloud::FILES_PER_SEC,
        (crate::move_home_icloud::BYTES_PER_SEC / 1048576.0).round()
    );
    let stop = std::sync::atomic::AtomicBool::new(false);
    let total = d.to_download;
    let total_bytes = d.to_download_bytes;
    let r = crate::move_home_icloud::download_evicted(
        &folders,
        &SystemProbes,
        crate::move_home_icloud::READ_THREADS,
        &stop,
        &|p| {
            let secs = p.elapsed.as_secs_f64().max(1.0);
            let left = total.saturating_sub(p.files_done);
            let eta = (left as f64 / (p.files_done.max(1) as f64 / secs)) as u64;
            println!(
                "  {} of about {} files, {} of about {}, {} failed, {} elapsed, about {} left",
                p.files_done,
                total,
                crate::move_home_plan::size(p.bytes_done),
                crate::move_home_plan::size(total_bytes),
                p.failed,
                crate::move_home_icloud::duration_text(p.elapsed.as_secs()),
                if p.files_done == 0 || left == 0 {
                    "?".to_string()
                } else {
                    crate::move_home_icloud::duration_text(eta)
                }
            );
        },
    );
    println!(
        "Downloaded {} files ({}); {} failed.",
        r.files,
        crate::move_home_plan::size(r.bytes),
        r.failed.len()
    );
    for f in r.failed.iter().take(20) {
        eprintln!("  {f}");
    }
    if r.failed.is_empty() {
        println!(
            "Run the dry run again (Settings → Check the move): the iCloud blocker should be gone."
        );
        0
    } else {
        1
    }
}

const USAGE: &str = "usage:
  Hollow move-home [--dry-run] [--json] [--with-vault-siblings] [--personal NAME]...
                   [--launch-agent LABEL]... [--script PATH]... [--allow-outside-home]
                   [--budget SECS] [--write-plan FILE]
  Hollow move-home --run PLAN [--wait-pid PID] [--relaunch APP] --yes
  Hollow move-home --rollback [BACKUP_DIR] [--wait-pid PID] [--relaunch APP] --yes
  Hollow move-home --download [--with-vault-siblings] [--personal NAME]...
  Hollow move-home --status
The dry run is the default and changes nothing. --download reads the iCloud-only
(evicted) files of every folder that moves by copy, 128 at a time, so the move
finds them local; it can run while Hollow is open, can be stopped (ctrl+c) and
run again, and prints its progress.";

/// `Hollow move-home ...`; returns the exit code.
pub fn cli_main(args: Vec<String>) -> i32 {
    let mut options = Options::default();
    let (mut json, mut yes, mut status_only, mut download) = (false, false, false, false);
    let (mut run, mut rollback, mut write_plan): (
        Option<String>,
        Option<Option<String>>,
        Option<String>,
    ) = (None, None, None);
    let (mut wait_pid, mut relaunch): (Option<i32>, Option<String>) = (None, None);
    let mut it = args.into_iter().peekable();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| -> Result<String, String> {
            it.next().ok_or(format!("{name} needs a value"))
        };
        let r: Result<(), String> = (|| {
            match arg.as_str() {
                "--dry-run" => {}
                "--json" => json = true,
                "--yes" => yes = true,
                "--status" => status_only = true,
                "--download" => download = true,
                "--with-vault-siblings" => options.include_vault_siblings = true,
                "--allow-outside-home" => options.allow_outside_home = true,
                "--personal" => options.personal.push(value("--personal")?),
                "--launch-agent" => options.launch_agents.push(value("--launch-agent")?),
                "--script" => options.scripts.push(value("--script")?),
                "--budget" => {
                    options.budget_secs = Some(
                        value("--budget")?
                            .parse()
                            .map_err(|_| "--budget takes seconds".to_string())?,
                    )
                }
                "--write-plan" => write_plan = Some(value("--write-plan")?),
                "--run" => run = Some(value("--run")?),
                "--rollback" => rollback = Some(None),
                "--wait-pid" => {
                    wait_pid = Some(
                        value("--wait-pid")?
                            .parse()
                            .map_err(|_| "--wait-pid takes a pid".to_string())?,
                    )
                }
                "--relaunch" => relaunch = Some(value("--relaunch")?),
                "--hydrate" => {
                    return Err("Hydrating iCloud-evicted files first is not supported in this version: the move renames on one volume and never downloads them.".into())
                }
                "-h" | "--help" => return Err(String::new()),
                other if matches!(rollback, Some(None)) && !other.starts_with('-') => {
                    rollback = Some(Some(other.to_string()))
                }
                other => return Err(format!("unknown option {other}")),
            }
            Ok(())
        })();
        if let Err(e) = r {
            if e.is_empty() {
                println!("{USAGE}");
                return 0;
            }
            eprintln!("{e}\n{USAGE}");
            return 2;
        }
    }
    if let Some(plan_file) = run {
        if !yes {
            eprintln!("--run changes your folders; add --yes to confirm.");
            return 2;
        }
        let plan_file = PathBuf::from(plan_file);
        let result = run_with(&plan_file, &SystemProbes, wait_pid, &RunHooks::default());
        let dir = plan_file
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let code = match &result {
            Ok(line) => {
                println!("{line}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
        relaunch_if_whole(&dir, relaunch.as_deref());
        return code;
    }
    if let Some(dir) = rollback {
        if !yes {
            eprintln!("--rollback changes your folders; add --yes to confirm.");
            return 2;
        }
        let dir = match dir {
            Some(d) => PathBuf::from(d),
            None => match Roots::locate().and_then(|r| {
                crate::move_home_run::latest_backup(&r).ok_or("No move to roll back.".to_string())
            }) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("{e}");
                    return 1;
                }
            },
        };
        let code = match rollback_with(&dir, &SystemProbes, wait_pid) {
            Ok(line) => {
                println!("{line}");
                0
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        };
        relaunch_if_whole(&dir, relaunch.as_deref());
        return code;
    }
    let roots = match Roots::locate() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    if status_only {
        let s = status(&roots);
        println!("{}", serde_json::to_string_pretty(&s).unwrap_or_default());
        return 0;
    }
    if download {
        return download_cli(&roots, &options);
    }
    match plan(&roots, &options, None) {
        Ok(p) => {
            if let Some(path) = write_plan {
                if let Err(e) =
                    std::fs::write(&path, serde_json::to_string_pretty(&p).unwrap_or_default())
                {
                    eprintln!("couldn't write {path}: {e}");
                    return 1;
                }
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&p).unwrap_or_default());
            } else {
                for line in &p.text {
                    println!("{line}");
                }
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// A plain GUI launch while a move is unfinished (a helper that crashed or
/// was killed mid-run). While a helper still holds the backup folder's lock
/// this launch steps aside: the helper opens Hollow when it ends. Otherwise
/// the half-done move is rolled back here, before anything opens a store, so
/// Hollow never starts on a half-moved layout and makes a fresh, empty store
/// beside the real one. A rollback that fails stops the launch with an alert
/// naming the backup folder and its log.
pub fn launch_guard() -> Option<i32> {
    let roots = Roots::locate().ok()?;
    let alert = |dir: &Path, why: &str| {
        let text = format!(
            "Hollow found an unfinished move of its folders and could not undo it: {why}. Nothing else was changed. Its backup and log are in {}. Run Hollow move-home --status for details, then Hollow move-home --rollback --yes once the cause is fixed.",
            dir.display()
        );
        let _ = std::process::Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg("on run argv\ndisplay alert \"Hollow can't open yet\" message (item 1 of argv)\nend run")
            .arg(text)
            .status();
        Some(1)
    };
    match unfinished(&roots)? {
        crate::move_home_run::Unfinished::Interrupted { locked: true, .. } => Some(0),
        crate::move_home_run::Unfinished::Failed { dir, reason } => {
            eprintln!("[move home] a failed rollback left the folders half moved: {reason}");
            alert(&dir, &reason)
        }
        crate::move_home_run::Unfinished::Interrupted { dir, locked: false } => {
            match rollback_with(&dir, &SystemProbes, None) {
                Ok(line) => {
                    eprintln!("[move home] an interrupted move was rolled back: {line}");
                    None
                }
                Err(e) => {
                    eprintln!("[move home] an interrupted move could not be rolled back: {e}");
                    alert(&dir, &e)
                }
            }
        }
    }
}

/// Open Hollow again after the helper ends, unless the layout is half moved
/// (a stopped run or rollback, or a failed undo): then the next launch's
/// guard explains it instead of opening on it.
fn relaunch_if_whole(dir: &Path, app: Option<&str>) {
    if crate::move_home_run::should_relaunch(dir) {
        relaunch_app(app);
    } else {
        eprintln!(
            "[move home] not opening Hollow: the folders are half moved; see {}",
            dir.display()
        );
    }
}

/// `open <app>`.
fn relaunch_app(app: Option<&str>) {
    let Some(app) = app else {
        return;
    };
    let _ = std::process::Command::new("/usr/bin/open")
        .arg(app)
        .status();
}
