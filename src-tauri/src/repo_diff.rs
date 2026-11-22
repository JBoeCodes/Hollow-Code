//! Repo-aware diff receipts (Paperclip PR 16).
//!
//! A chat turn or workflow run whose working folder is a git repository
//! outside the vault, or a linked project's workspace
//! (`vault/workspaces/local-<24 hex>`), also records what changed in that
//! repository, next to the vault diff automated history already takes.
//!
//! **Targets** ([`select_targets`]). The vault and the cwd are canonicalized.
//! (a) Unless the cwd IS the vault root (no git spawn at all then: its
//! toplevel could only be the vault, an ancestor or nothing), the cwd's git
//! toplevel, unless it is the vault or an ancestor of it. A repository nested
//! inside the vault is a target: vault history skips dot-paths and files over
//! 2 MiB, so its `.gitignore` or `.github/**` edits would be invisible
//! otherwise; its rows that vault history did record are dropped at receipt
//! time (`evidence`'s dedupe, through `vault_prefix`). (b) For a linked
//! project's workspace cwd, the project path from the project index (`directory_connections::manifest_path`)
//! when its canonical path IS its own git toplevel (a plain folder under a
//! repo-backed Projects root never diffs the enclosing repository). Projects
//! and Vault never overlap (`directory_connections::connect`), so (b) is
//! never nested. Deduplicated by canonical root, at most two.
//!
//! **Timing.** [`begin`] runs after admission (the lease is held) and before
//! the spawn, never under `HISTORY_LOCK`; the baseline is ARMED only after a
//! successful spawn (`EvidenceCollector::arm_repo_baseline`), so a run that
//! never started does no post-run git work. [`finish_for_run`] runs after the
//! run's processes have ended, the usage row is finished and orphaned
//! delegate rows are closed, and BEFORE the lease is released, so the next
//! admitted run's edits can never land in this receipt. The sidecar is written
//! through `automated_history::store_repo_sidecar`, which takes
//! `HISTORY_LOCK` only for the atomic write and checks the lease still names
//! this run. The post capture never uses the run's stop signal (a stopped run
//! still records what it changed).
//!
//! **Read-only, hardened git** ([`git_command`]). Every `GIT_*` variable is
//! stripped; `GIT_OPTIONAL_LOCKS=0` plus `--no-optional-locks` keep status
//! from taking `index.lock` or rewriting the index, and
//! `diff.autoRefreshIndex=false` does the same for `git diff <commit>`
//! (whose refresh ignores optional-locks); `--no-lazy-fetch`,
//! `GIT_NO_LAZY_FETCH=1` and `protocol.allow=never` stop a promisor fetch;
//! the git dir and work tree are pinned from the pre-run discovery against a
//! later `core.worktree` edit or `.git` swap. Command-scope config goes only
//! through `GIT_CONFIG_COUNT/KEY_n/VALUE_n`, because `-c` splits at the first
//! `=` and config subsection names (filter drivers) may contain `=`. That
//! config disables fsmonitor, the untracked cache, hooks, external diff,
//! submodule recursion, and every filter driver the repository defines,
//! re-read at each snapshot; diffs pass `--no-ext-diff --no-textconv` and
//! ignore submodules, so no program the agent planted in `.git/config` or
//! `.gitattributes` ever runs under Hollow after the run's process sweep.
//! Nothing is ever staged: untracked files are read directly (bounded,
//! `O_NOFOLLOW|O_NONBLOCK`, regular files only), never added to the index.
//!
//! **Bounds.** [`LIMITS`]: 200 files per repository (`total_files` keeps the
//! real count), 512 KiB per file read, 256 KiB per patch, 1 MiB of patches
//! in total, a 10 s limit per git call, 15 s for the pre-run capture and 30 s
//! for the post-run one. At most two git processes run at once (`GIT_SLOTS`).
//! Each capture costs about five to seven git spawns per repository
//! (numstat and patches are batched), each with process_group's settle scan.
//! A partial capture stays honest: truncated status or raw output makes the
//! repository `unavailable`; a budget that runs out after status keeps every
//! row with `truncated` set.
//!
//! **Content.** A path any component of which is a secret name
//! (`directory_connections::secret_name`) is `withheld`: listed, never read
//! and never patched. Every stored patch passes through
//! `claude_runner::redact`. Patches live in the run's
//! `.hollow-history/runs/{id}.repo.json` sidecar (display-only, pruned with
//! its run); the receipt carries summaries only.
//!
//! GUI process only (the agent.rs MCP source lint bans this module from
//! every Hollow MCP subprocess). `git_activity` does not use it.

use crate::process_group::{self, StopSignal};
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::Semaphore;

/// git's well-known empty tree: the diff base for an unborn HEAD.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const SIDECAR_SCHEMA_VERSION: u32 = 1;
const MAX_ERROR_CHARS: usize = 300;
const MAX_TARGETS: usize = 2;
const BINARY_SNIFF_BYTES: usize = 8000;
const DISCOVERY_CAP: usize = 64 * 1024;
const HEAD_CAP: usize = 4 * 1024;
const FILTER_LIST_CAP: usize = 256 * 1024;
const NUMSTAT_CAP: usize = 4 * 1024 * 1024;
const BUDGET_ERROR: &str = "the capture ran out of time";

/// How much one capture may read and spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_patch_bytes: usize,
    pub max_total_patch_bytes: usize,
    /// Extra bytes the batched patch stream may read past the total, so the
    /// last kept section is rarely cut mid-file.
    pub stream_slack: usize,
    pub git_timeout: Duration,
    pub pre_budget: Duration,
    pub post_budget: Duration,
    pub max_status_bytes: usize,
    pub max_status_entries: usize,
    /// Bytes the pre-run capture may hash; past it a file's signature falls
    /// back to (length, mtime).
    pub hash_budget: u64,
}

pub const LIMITS: Limits = Limits {
    max_files: 200,
    max_file_bytes: 512 * 1024,
    max_patch_bytes: 256 * 1024,
    max_total_patch_bytes: 1024 * 1024,
    stream_slack: 64 * 1024,
    git_timeout: Duration::from_secs(10),
    pre_budget: Duration::from_secs(15),
    post_budget: Duration::from_secs(30),
    max_status_bytes: 8 * 1024 * 1024,
    max_status_entries: 20_000,
    hash_budget: 64 * 1024 * 1024,
};

impl Default for Limits {
    fn default() -> Self {
        LIMITS
    }
}

/// One repository's changes during a run. In the receipt without patches
/// (at most `evidence::RECEIPT_REPO_FILES` rows); in the sidecar with them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoChanges {
    /// Canonical repository root.
    pub root: String,
    pub label: String,
    /// `cwd` (rule a) | `linkedProject` (rule b).
    pub target: String,
    /// `captured` | `unavailable`.
    pub status: String,
    /// Why the repository is `unavailable` (at most 300 characters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub head_before: Option<String>,
    #[serde(default)]
    pub head_after: Option<String>,
    #[serde(default)]
    pub files: Vec<RepoFileChange>,
    /// Every changed path, including rows past the listed ones.
    #[serde(default)]
    pub total_files: usize,
    /// Some rows or patches were left out (caps or the time budget).
    #[serde(default)]
    pub truncated: bool,
    /// The run's folder relative to the root (`src/`, or `` at the root),
    /// for a cwd target: relative tool paths resolve against it.
    #[serde(skip)]
    pub cwd_in_repo: Option<String>,
    /// Every spelling of the root an agent may write absolute paths under.
    #[serde(skip)]
    pub alias_roots: Vec<String>,
    /// The root relative to the vault, for a repository nested in it.
    #[serde(skip)]
    pub vault_prefix: Option<String>,
}

fn observed() -> String {
    "observed".into()
}

/// One changed path in a repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoFileChange {
    /// Relative to the repository root.
    pub path: String,
    /// `create` | `modify` | `delete`.
    pub change_type: String,
    /// The porcelain XY status, a committed row's raw letter, `clean` (a
    /// file dirty before the run is back to its committed content) or
    /// `special` (a FIFO, socket or device: never read).
    pub status: String,
    #[serde(default)]
    pub untracked: bool,
    /// A submodule pointer: listed, never diffed.
    #[serde(default)]
    pub gitlink: bool,
    #[serde(default)]
    pub additions: Option<u64>,
    #[serde(default)]
    pub deletions: Option<u64>,
    #[serde(default)]
    pub binary: bool,
    /// Larger than the per-file read cap: no patch.
    #[serde(default)]
    pub oversized: bool,
    #[serde(default)]
    pub patch_truncated: bool,
    /// A secret-named path: its content was never read or stored.
    #[serde(default)]
    pub withheld: bool,
    /// The path had uncommitted edits before the run; its patch includes them.
    #[serde(default)]
    pub dirty_before: bool,
    /// `confirmed` | `observed`, set by `evidence`.
    #[serde(default = "observed")]
    pub attribution: String,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

/// `.hollow-history/runs/{runId}.repo.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoSidecar {
    pub schema_version: u32,
    pub run_id: String,
    pub captured_at: String,
    pub repos: Vec<RepoChanges>,
}

/// The pre-run snapshot of a run's target repositories. Not serialized.
#[derive(Debug, Clone, Default)]
pub struct RepoBaseline {
    targets: Vec<Target>,
    limits: Limits,
    armed: bool,
}

impl RepoBaseline {
    /// No target repository: nothing to capture.
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// The agent spawned: the post-run capture may run.
    pub fn arm(&mut self) {
        self.armed = true;
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// The repositories this run watches (their worktree roots).
    pub fn roots(&self) -> Vec<std::path::PathBuf> {
        self.targets.iter().map(|t| t.root.clone()).collect()
    }
}

#[derive(Debug, Clone)]
struct Target {
    root: PathBuf,
    git_dir: PathBuf,
    label: String,
    kind: &'static str,
    cwd_in_repo: Option<String>,
    alias_roots: Vec<String>,
    vault_prefix: Option<String>,
    pre: Result<Snapshot, String>,
}

#[derive(Debug, Clone, Default)]
struct Snapshot {
    head: Option<String>,
    /// Porcelain XY per path.
    entries: BTreeMap<String, String>,
    sigs: BTreeMap<String, FileSig>,
}

/// What a path looked like, without keeping its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileSig {
    Hash { len: u64, sha: String },
    Meta { len: u64, mtime_ns: u128 },
    Symlink(String),
    Special,
    Missing,
}

impl FileSig {
    fn len(&self) -> Option<u64> {
        match self {
            FileSig::Hash { len, .. } | FileSig::Meta { len, .. } => Some(*len),
            _ => None,
        }
    }
}

static GIT_SLOTS: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(2));

static GIT_BINARY: Lazy<PathBuf> = Lazy::new(|| {
    ["/opt/homebrew/bin/git", "/usr/local/bin/git"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("git"))
});

/// Command-scope config every capture command gets (after the filter
/// drivers are disabled). Passed only through `GIT_CONFIG_COUNT`.
const OVERRIDES: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    ("core.untrackedCache", "false"),
    // `git diff <commit> -- paths` refreshes the index (taking index.lock)
    // when two or more listed paths are stat-dirty but content-clean; that
    // refresh ignores optional-locks and obeys only this key.
    ("diff.autoRefreshIndex", "false"),
    ("core.hooksPath", "/dev/null"),
    ("core.pager", "cat"),
    ("core.quotePath", "true"),
    ("diff.external", ""),
    ("diff.noprefix", "false"),
    ("diff.mnemonicPrefix", "false"),
    ("diff.submodule", "short"),
    ("submodule.recurse", "false"),
    ("status.submoduleSummary", "false"),
    ("color.ui", "false"),
    ("protocol.allow", "never"),
];

/// Flags every worktree diff gets: deterministic headers, no external
/// programs, no submodule recursion.
const WORKTREE_DIFF_FLAGS: &[&str] = &[
    "--ignore-submodules=all",
    "--no-ext-diff",
    "--no-textconv",
    "--no-color",
    "--no-renames",
    "--no-relative",
    "-O/dev/null",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

#[cfg(test)]
thread_local! {
    static GIT_SPAWNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static EXPIRE_BEFORE_PATCHES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Snapshot the run's target repositories before its agent spawns. Never
/// fails: a target whose snapshot failed is recorded as unavailable at
/// finish. Empty (and spawn-free) for a vault-root cwd.
pub async fn begin(vault: &Path, cwd: &Path, stop: Option<&StopSignal>) -> RepoBaseline {
    begin_with(vault, cwd, stop, LIMITS).await
}

pub(crate) async fn begin_with(
    vault: &Path,
    cwd: &Path,
    stop: Option<&StopSignal>,
    limits: Limits,
) -> RepoBaseline {
    let deadline = Instant::now() + limits.pre_budget;
    let mut targets = select_targets(vault, cwd, deadline, stop, &limits).await;
    let mut hash_budget = limits.hash_budget;
    for target in &mut targets {
        target.pre = pre_snapshot(target, deadline, stop, &limits, &mut hash_budget).await;
    }
    RepoBaseline {
        targets,
        limits,
        armed: false,
    }
}

/// Capture the run's repository changes (with patches). Needs an armed
/// baseline; the caller ([`finish_for_run`]) checks that.
pub(crate) async fn capture_after(baseline: RepoBaseline) -> Vec<RepoChanges> {
    let limits = baseline.limits;
    let deadline = Instant::now() + limits.post_budget;
    let mut hash_budget = limits.hash_budget;
    let mut out = Vec::with_capacity(baseline.targets.len());
    for target in &baseline.targets {
        out.push(capture_one(target, &limits, deadline, &mut hash_budget).await);
    }
    out
}

/// The post-run step of a leased run: capture, store the sidecar (logging
/// any error; it never fails the run) and return the summaries without
/// patches. With no armed baseline or no target it captures nothing and
/// removes any sidecar the agent planted for this run.
pub async fn finish_for_run(
    vault: &Path,
    run_id: &str,
    baseline: Option<RepoBaseline>,
) -> Vec<RepoChanges> {
    let Some(baseline) = baseline.filter(|b| b.is_armed() && !b.is_empty()) else {
        store_sidecar(vault, run_id, None).await;
        return Vec::new();
    };
    let repos = capture_after(baseline).await;
    let sidecar = RepoSidecar {
        schema_version: SIDECAR_SCHEMA_VERSION,
        run_id: run_id.to_string(),
        captured_at: Utc::now().to_rfc3339(),
        repos: repos.clone(),
    };
    match serde_json::to_vec_pretty(&sidecar) {
        Ok(bytes) => store_sidecar(vault, run_id, Some(bytes)).await,
        Err(error) => {
            eprintln!("[repo_diff] encoding repository changes for {run_id} failed: {error}")
        }
    }
    repos
        .into_iter()
        .map(|mut repo| {
            for file in &mut repo.files {
                file.patch = None;
            }
            repo
        })
        .collect()
}

async fn store_sidecar(vault: &Path, run_id: &str, bytes: Option<Vec<u8>>) {
    let (vault, id) = (vault.to_path_buf(), run_id.to_string());
    let result = tokio::task::spawn_blocking(move || {
        crate::automated_history::store_repo_sidecar(&vault, &id, bytes.as_deref())
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("[repo_diff] {error}"),
        Err(error) => {
            eprintln!("[repo_diff] storing repository changes for {run_id} failed: {error}")
        }
    }
}

/// The sidecar of history run `run_id`, or `None` when it has none.
pub fn read_sidecar(vault: &Path, run_id: &str) -> Result<Option<RepoSidecar>, String> {
    let Some(bytes) = crate::automated_history::read_repo_sidecar_bytes(vault, run_id)? else {
        return Ok(None);
    };
    let sidecar: RepoSidecar = serde_json::from_slice(&bytes)
        .map_err(|error| format!("repository changes record is unreadable: {error}"))?;
    if sidecar.schema_version != SIDECAR_SCHEMA_VERSION || sidecar.run_id != run_id {
        return Err("repository changes record does not belong to this run".into());
    }
    Ok(Some(sidecar))
}

// ---------------------------------------------------------------- targets

struct Discovered {
    toplevel: PathBuf,
    git_dir: PathBuf,
    prefix: String,
}

async fn select_targets(
    vault: &Path,
    cwd: &Path,
    deadline: Instant,
    stop: Option<&StopSignal>,
    limits: &Limits,
) -> Vec<Target> {
    let (Ok(vault_c), Ok(cwd_c)) = (std::fs::canonicalize(vault), std::fs::canonicalize(cwd))
    else {
        return Vec::new();
    };
    let mut out: Vec<Target> = Vec::new();

    // Rule (a): the cwd's own repository. Skipped with no spawn for the vault
    // root, whose toplevel can only be the vault, an ancestor, or none.
    if cwd_c != vault_c {
        if let Some(found) = discover(&cwd_c, deadline, stop, limits).await {
            if allowed_root(&found.toplevel, &vault_c) {
                let mut alias_roots = vec![lossy(&found.toplevel)];
                if let Some(given) = strip_components(cwd, &found.prefix) {
                    push_unique(&mut alias_roots, lossy(&given));
                }
                out.push(Target {
                    label: label_of(&found.toplevel),
                    kind: "cwd",
                    cwd_in_repo: Some(found.prefix.clone()),
                    alias_roots,
                    vault_prefix: nested_prefix(&found.toplevel, &vault_c),
                    root: found.toplevel,
                    git_dir: found.git_dir,
                    pre: Err("not captured".into()),
                });
            }
        }
    }

    // Rule (b): a linked project's workspace, from the canonical or the given
    // path (a workspace folder symlinked into its project dedupes here).
    let id = workspace_project_id(&cwd_c, &vault_c).or_else(|| workspace_project_id(cwd, vault));
    if let Some(stored) = id.and_then(|id| crate::directory_connections::project_root(vault, &id)) {
        if let Ok(project_c) = std::fs::canonicalize(&stored) {
            if !out.iter().any(|target| target.root == project_c) {
                if let Some(found) = discover(&project_c, deadline, stop, limits).await {
                    if found.toplevel == project_c && allowed_root(&project_c, &vault_c) {
                        let mut alias_roots = vec![lossy(&project_c)];
                        push_unique(&mut alias_roots, lossy(&stored));
                        out.push(Target {
                            label: label_of(&project_c),
                            kind: "linkedProject",
                            cwd_in_repo: None,
                            alias_roots,
                            vault_prefix: nested_prefix(&project_c, &vault_c),
                            root: project_c,
                            git_dir: found.git_dir,
                            pre: Err("not captured".into()),
                        });
                    }
                }
            }
        }
    }
    out.truncate(MAX_TARGETS);
    out
}

/// Never the vault's own repository, and never one that contains the vault.
fn allowed_root(root: &Path, vault_c: &Path) -> bool {
    root != vault_c && !vault_c.starts_with(root)
}

fn nested_prefix(root: &Path, vault_c: &Path) -> Option<String> {
    let rel = root.strip_prefix(vault_c).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// `local-<24 hex>` when `cwd` is inside `vault/workspaces/<id>`.
fn workspace_project_id(cwd: &Path, vault: &Path) -> Option<String> {
    let rel = cwd.strip_prefix(vault).ok()?;
    let mut parts = rel.components();
    if parts.next()?.as_os_str() != "workspaces" {
        return None;
    }
    let id = parts.next()?.as_os_str().to_str()?.to_string();
    let hex = id.strip_prefix("local-")?;
    (hex.len() == 24
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(id)
}

/// `path` minus as many trailing components as `prefix` (`src/app/`) has:
/// the repository root spelled the way the cwd was given.
fn strip_components(path: &Path, prefix: &str) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let depth = prefix.split('/').filter(|part| !part.is_empty()).count();
    let mut root = path;
    for _ in 0..depth {
        root = root.parent()?;
    }
    Some(root.to_path_buf())
}

fn label_of(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| lossy(root))
}

fn lossy(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches('/').to_string()
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !value.is_empty() && !list.contains(&value) {
        list.push(value);
    }
}

/// `git rev-parse --show-toplevel --absolute-git-dir --show-prefix` in
/// `dir`, canonicalized. `None` for anything else: not a repository, a bare
/// or `.git` folder, a safe.directory refusal, no git.
async fn discover(
    dir: &Path,
    deadline: Instant,
    stop: Option<&StopSignal>,
    limits: &Limits,
) -> Option<Discovered> {
    let mut cmd = git_command(dir, None, &[]);
    cmd.args([
        "rev-parse",
        "--show-toplevel",
        "--absolute-git-dir",
        "--show-prefix",
    ]);
    let out = run_git(cmd, DISCOVERY_CAP, deadline, limits, stop)
        .await
        .ok()?;
    if out.truncated || !out.success {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let mut lines = text.split('\n');
    let toplevel = lines.next()?.trim_end_matches('\r');
    let git_dir = lines.next()?.trim_end_matches('\r');
    let prefix = lines.next().unwrap_or("").trim_end_matches('\r');
    if toplevel.is_empty() || git_dir.is_empty() {
        return None;
    }
    Some(Discovered {
        toplevel: std::fs::canonicalize(toplevel).ok()?,
        git_dir: std::fs::canonicalize(git_dir).ok()?,
        prefix: prefix.to_string(),
    })
}

// -------------------------------------------------------------------- git

/// A hardened git invocation (see the module docs). With `git_dir` the
/// command is pinned to that git dir and to `root` as its work tree, and gets
/// `--no-lazy-fetch` (git 2.44+; an older git fails the command, so the
/// repository reads as unavailable instead of running unprotected).
fn git_command(root: &Path, git_dir: Option<&Path>, filters: &[String]) -> Command {
    Command::from(hardened_git(root, git_dir, filters))
}

/// [`git_command`] as a blocking `std` command, for a synchronous reader
/// (the code index, CC2) that needs the same hardening.
pub(crate) fn hardened_git(
    root: &Path,
    git_dir: Option<&Path>,
    filters: &[String],
) -> std::process::Command {
    let mut cmd = std::process::Command::new(&*GIT_BINARY);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    cmd.env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C");
    // Tests never read the developer's own git config.
    #[cfg(test)]
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    let mut overrides: Vec<(String, String)> = OVERRIDES
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    for name in filters {
        for key in ["process", "clean", "smudge"] {
            overrides.push((format!("filter.{name}.{key}"), String::new()));
        }
        overrides.push((format!("filter.{name}.required"), "false".into()));
    }
    cmd.env("GIT_CONFIG_COUNT", overrides.len().to_string());
    for (index, (key, value)) in overrides.iter().enumerate() {
        cmd.env(format!("GIT_CONFIG_KEY_{index}"), key)
            .env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    // A worktree a Codex auto run could write: its `commondir` is put back
    // and the common folder pinned, never read from that file.
    if let Some((_, common)) = crate::agent::restore_codex_worktree_git(root) {
        cmd.env("GIT_COMMON_DIR", common);
    }
    cmd.arg("--no-optional-locks").arg("--no-pager");
    if let Some(git_dir) = git_dir {
        cmd.arg("--no-lazy-fetch");
        let mut arg = OsString::from("--git-dir=");
        arg.push(git_dir);
        cmd.arg(arg);
        let mut arg = OsString::from("--work-tree=");
        arg.push(root);
        cmd.arg(arg);
    }
    cmd.current_dir(root).stdin(std::process::Stdio::null());
    cmd
}

struct GitOut {
    stdout: Vec<u8>,
    /// Stdout hit the cap (the child usually died of SIGPIPE; not a failure).
    truncated: bool,
    success: bool,
    /// The exit code (None when truncated, signalled or unavailable).
    code: Option<i32>,
    detail: String,
}

impl GitOut {
    /// The complete, successful output, or why not.
    fn complete(self, what: &str) -> Result<Vec<u8>, String> {
        if self.truncated {
            return Err(format!("{what} output exceeded its cap"));
        }
        if !self.success {
            return Err(self.detail);
        }
        Ok(self.stdout)
    }
}

/// Run one git command under a `GIT_SLOTS` permit and the time budget. The
/// truncation check comes BEFORE the exit status: a capped read closes the
/// pipe, and git then dies of SIGPIPE, which is expected.
async fn run_git(
    mut cmd: Command,
    cap: usize,
    deadline: Instant,
    limits: &Limits,
    stop: Option<&StopSignal>,
) -> Result<GitOut, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(BUDGET_ERROR.into());
    }
    let permit = match tokio::time::timeout(remaining, GIT_SLOTS.acquire()).await {
        Ok(Ok(permit)) => permit,
        Ok(Err(_)) => return Err("the git scheduler is closed".into()),
        Err(_) => return Err(BUDGET_ERROR.into()),
    };
    let limit = limits
        .git_timeout
        .min(deadline.saturating_duration_since(Instant::now()));
    if limit.is_zero() {
        return Err(BUDGET_ERROR.into());
    }
    #[cfg(test)]
    GIT_SPAWNS.with(|count| count.set(count.get() + 1));
    let out = process_group::run_to_completion_capped(&mut cmd, None, limit, stop, cap)
        .await
        .map_err(|error| cap_error(&format!("could not run git: {error}")))?;
    drop(permit);
    if out.stopped {
        return Err("the run was stopped".into());
    }
    if out.stdout_truncated {
        return Ok(GitOut {
            stdout: out.stdout,
            truncated: true,
            success: true,
            code: None,
            detail: String::new(),
        });
    }
    if out.timed_out {
        return Err(format!("git did not finish within {}s", limit.as_secs()));
    }
    let success = out.status.is_some_and(|status| status.success());
    let detail = if success {
        String::new()
    } else {
        let status = out
            .status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "no exit status".into());
        cap_error(&format!("git failed ({status}): {}", out.stderr.trim()))
    };
    Ok(GitOut {
        stdout: out.stdout,
        truncated: false,
        success,
        code: out.status.and_then(|status| status.code()),
        detail,
    })
}

/// Filter driver names the repository's config defines (subsection between
/// the first and last `.`), each disabled for every later command.
async fn filter_drivers(
    target: &Target,
    deadline: Instant,
    stop: Option<&StopSignal>,
    limits: &Limits,
) -> Result<Vec<String>, String> {
    let mut cmd = git_command(&target.root, Some(&target.git_dir), &[]);
    cmd.args(["config", "-z", "--name-only", "--get-regexp", r"^filter\."]);
    let out = run_git(cmd, FILTER_LIST_CAP, deadline, limits, stop).await?;
    if out.truncated {
        return Err("the repository defines too many filter settings".into());
    }
    // `git config --get-regexp` exits 1 when nothing matches.
    if !out.success {
        return if out.stdout.is_empty() && out.code == Some(1) {
            Ok(Vec::new())
        } else if out.stdout.is_empty() {
            Err(out.detail)
        } else {
            Ok(parse_filter_names(&out.stdout))
        };
    }
    Ok(parse_filter_names(&out.stdout))
}

pub(crate) fn parse_filter_names(bytes: &[u8]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for key in bytes.split(|b| *b == 0) {
        let key = String::from_utf8_lossy(key);
        let key = key.trim();
        let (Some(first), Some(last)) = (key.find('.'), key.rfind('.')) else {
            continue;
        };
        if last > first + 1 {
            names.insert(key[first + 1..last].to_string());
        }
    }
    names.into_iter().collect()
}

async fn read_head(
    target: &Target,
    filters: &[String],
    deadline: Instant,
    stop: Option<&StopSignal>,
    limits: &Limits,
) -> Result<Option<String>, String> {
    let mut cmd = git_command(&target.root, Some(&target.git_dir), filters);
    cmd.args(["rev-parse", "--verify", "-q", "HEAD"]);
    let out = run_git(cmd, HEAD_CAP, deadline, limits, stop).await?;
    if out.truncated {
        return Err("HEAD output exceeded its cap".into());
    }
    // An unborn HEAD exits 1 with nothing printed.
    if !out.success {
        return if out.stdout.is_empty() && out.code == Some(1) {
            Ok(None)
        } else {
            Err(out.detail)
        };
    }
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok((!head.is_empty()).then_some(head))
}

async fn read_status(
    target: &Target,
    filters: &[String],
    deadline: Instant,
    stop: Option<&StopSignal>,
    limits: &Limits,
) -> Result<BTreeMap<String, String>, String> {
    let mut cmd = git_command(&target.root, Some(&target.git_dir), filters);
    cmd.args([
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--ignore-submodules=all",
        "--no-renames",
    ]);
    let bytes = run_git(cmd, limits.max_status_bytes, deadline, limits, stop)
        .await?
        .complete("status")?;
    let entries = parse_status_z(&bytes);
    if entries.len() > limits.max_status_entries {
        return Err(format!(
            "the repository has more than {} changed paths",
            limits.max_status_entries
        ));
    }
    Ok(entries.into_iter().map(|(xy, path)| (path, xy)).collect())
}

/// `XY path\0` records (a rename or copy, never produced with `--no-renames`,
/// carries its source path in the next record, which is skipped).
fn parse_status_z(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut records = bytes.split(|b| *b == 0);
    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        let xy = String::from_utf8_lossy(&record[..2]).into_owned();
        let path = String::from_utf8_lossy(&record[3..]).into_owned();
        if xy.contains('R') || xy.contains('C') {
            let _ = records.next();
        }
        out.push((xy, path));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawRow {
    old_mode: String,
    new_mode: String,
    status: String,
}

impl RawRow {
    fn gitlink(&self) -> bool {
        self.old_mode == "160000" || self.new_mode == "160000"
    }
}

/// `:old new oldsha newsha S\0path\0` records.
fn parse_raw_z(bytes: &[u8]) -> BTreeMap<String, RawRow> {
    let mut out = BTreeMap::new();
    let mut tokens = bytes.split(|b| *b == 0);
    while let Some(header) = tokens.next() {
        let header = String::from_utf8_lossy(header);
        let Some(header) = header.strip_prefix(':') else {
            continue;
        };
        let fields: Vec<&str> = header.split(' ').collect();
        if fields.len() < 5 {
            continue;
        }
        let status: String = fields[4].chars().take(1).collect();
        let Some(mut path) = tokens.next() else { break };
        if status == "R" || status == "C" {
            match tokens.next() {
                Some(next) => path = next,
                None => break,
            }
        }
        out.insert(
            String::from_utf8_lossy(path).into_owned(),
            RawRow {
                old_mode: fields[0].to_string(),
                new_mode: fields[1].to_string(),
                status,
            },
        );
    }
    out
}

/// `added\tdeleted\tpath\0` records; `-\t-` marks a binary file.
fn parse_numstat_z(bytes: &[u8]) -> BTreeMap<String, (Option<u64>, Option<u64>, bool)> {
    let mut out = BTreeMap::new();
    let mut tokens = bytes.split(|b| *b == 0);
    while let Some(token) = tokens.next() {
        let token = String::from_utf8_lossy(token).into_owned();
        let mut parts = token.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            // A rename's `\0old\0new` form (never with `--no-renames`).
            let _ = tokens.next();
            match tokens.next() {
                Some(next) => String::from_utf8_lossy(next).into_owned(),
                None => break,
            }
        } else {
            path.to_string()
        };
        let binary = added == "-" && deleted == "-";
        out.insert(path, (added.parse().ok(), deleted.parse().ok(), binary));
    }
    out
}

/// Split a patch stream into `(path, section, capped)` at each `diff --git `
/// header, keeping at most `per_file_cap` bytes of each section (cut at a
/// character boundary). Paths come from the header's `b/` side, C-unquoted
/// when git quoted them (`core.quotePath=true`).
fn split_patch_stream(stream: &[u8], per_file_cap: usize) -> Vec<(String, String, bool)> {
    const HEADER: &[u8] = b"diff --git ";
    let mut starts = Vec::new();
    let mut at = 0usize;
    while at < stream.len() {
        if stream[at..].starts_with(HEADER) && (at == 0 || stream[at - 1] == b'\n') {
            starts.push(at);
        }
        match stream[at..].iter().position(|b| *b == b'\n') {
            Some(offset) => at += offset + 1,
            None => break,
        }
    }
    let mut out = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(stream.len());
        let section = &stream[*start..end];
        let header_end = section
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or(section.len());
        let header = String::from_utf8_lossy(&section[..header_end]);
        let Some(path) = parse_diff_header(&header) else {
            continue;
        };
        let text = String::from_utf8_lossy(section).into_owned();
        let (text, capped) = cap_patch(text, per_file_cap);
        out.push((path, text, capped));
    }
    out
}

/// The `b/` path of a `diff --git a/P b/P` header (renames are disabled, so
/// both sides name the same path).
fn parse_diff_header(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    if rest.starts_with('"') {
        let (_, after) = c_unquote(rest)?;
        let after = after.strip_prefix(' ')?;
        let b = if after.starts_with('"') {
            c_unquote(after)?.0
        } else {
            after.to_string()
        };
        return b.strip_prefix("b/").map(str::to_string);
    }
    // Unquoted: `a/P b/P`, where P may itself contain ` b/`.
    let len = rest.len();
    if len < 5 || (len - 5) % 2 != 0 {
        return None;
    }
    let p = (len - 5) / 2;
    let a = rest.get(2..2 + p)?;
    let middle = rest.get(2 + p..5 + p)?;
    let b = rest.get(5 + p..)?;
    (rest.starts_with("a/") && middle == " b/" && a == b).then(|| b.to_string())
}

/// Decode one C-quoted string at the start of `s` (`"…"` with `\t`, `\n`,
/// `\"`, `\\`, `\ooo` octal bytes). Returns the text and what follows it.
fn c_unquote(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), s.get(i + 1..)?)),
            b'\\' => {
                let next = *bytes.get(i + 1)?;
                match next {
                    b'a' => out.push(0x07),
                    b'b' => out.push(0x08),
                    b't' => out.push(b'\t'),
                    b'n' => out.push(b'\n'),
                    b'v' => out.push(0x0b),
                    b'f' => out.push(0x0c),
                    b'r' => out.push(b'\r'),
                    b'0'..=b'7' => {
                        let digits = bytes.get(i + 1..i + 4)?;
                        if !digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
                            return None;
                        }
                        let value = digits
                            .iter()
                            .fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
                        out.push(u8::try_from(value).ok()?);
                        i += 4;
                        continue;
                    }
                    other => out.push(other),
                }
                i += 2;
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    None
}

// -------------------------------------------------------------- snapshots

async fn pre_snapshot(
    target: &Target,
    deadline: Instant,
    stop: Option<&StopSignal>,
    limits: &Limits,
    hash_budget: &mut u64,
) -> Result<Snapshot, String> {
    let filters = filter_drivers(target, deadline, stop, limits).await?;
    let head = read_head(target, &filters, deadline, stop, limits).await?;
    let entries = read_status(target, &filters, deadline, stop, limits).await?;
    let wanted: Vec<(String, bool)> = entries
        .keys()
        .map(|path| (path.clone(), !is_secret_path(path)))
        .collect();
    let (sigs, left) = compute_sigs(&target.root, wanted, *hash_budget, deadline).await;
    *hash_budget = left;
    Ok(Snapshot {
        head,
        entries,
        sigs,
    })
}

/// Signatures of `paths` (`(path, hash?)`), in one blocking task under the
/// time budget. A path left out (the budget ran out) has no signature, which
/// the post-run comparison treats as changed.
async fn compute_sigs(
    root: &Path,
    paths: Vec<(String, bool)>,
    hash_budget: u64,
    deadline: Instant,
) -> (BTreeMap<String, FileSig>, u64) {
    if paths.is_empty() {
        return (BTreeMap::new(), hash_budget);
    }
    let root = root.to_path_buf();
    let remaining = deadline.saturating_duration_since(Instant::now());
    let task = tokio::task::spawn_blocking(move || {
        let mut budget = hash_budget;
        let mut out = BTreeMap::new();
        for (path, hash) in paths {
            if Instant::now() >= deadline {
                break;
            }
            let sig = file_sig(&root, &path, hash, &mut budget);
            out.insert(path, sig);
        }
        (out, budget)
    });
    match tokio::time::timeout(remaining, task).await {
        Ok(Ok(result)) => result,
        _ => (BTreeMap::new(), 0),
    }
}

/// Open `path` for reading only if it is a regular file: never following a
/// final symlink, never blocking on a FIFO, and checked again on the handle.
fn open_regular(path: &Path) -> Option<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}

/// A regular file under `root` (its canonical parent is inside the root, so
/// a directory swapped for a symlink is never followed out).
fn inside_root(root: &Path, path: &Path) -> bool {
    path.parent()
        .and_then(|parent| std::fs::canonicalize(parent).ok())
        .is_some_and(|parent| parent.starts_with(root))
}

fn file_sig(root: &Path, rel: &str, hash: bool, budget: &mut u64) -> FileSig {
    let path = root.join(rel);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(_) => return FileSig::Missing,
    };
    if meta.file_type().is_symlink() {
        return FileSig::Symlink(
            std::fs::read_link(&path)
                .map(|target| target.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }
    if !meta.is_file() {
        return if meta.is_dir() {
            FileSig::Missing
        } else {
            FileSig::Special
        };
    }
    let len = meta.len();
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_nanos())
        .unwrap_or(0);
    let meta_sig = FileSig::Meta { len, mtime_ns };
    if !hash || len > *budget || !inside_root(root, &path) {
        return meta_sig;
    }
    let Some(mut file) = open_regular(&path) else {
        return FileSig::Special;
    };
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut read_total = 0u64;
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                read_total += n as u64;
                if read_total > len {
                    return meta_sig;
                }
                hasher.update(&buf[..n]);
            }
            Err(_) => return meta_sig,
        }
    }
    *budget = budget.saturating_sub(read_total);
    FileSig::Hash {
        len: read_total,
        sha: format!("{:x}", hasher.finalize()),
    }
}

/// Any component of `path` is a secret name.
fn is_secret_path(path: &str) -> bool {
    path.split('/')
        .any(|part| crate::directory_connections::secret_name(&part.to_lowercase()))
}

// ---------------------------------------------------------------- capture

fn unavailable(mut repo: RepoChanges, error: &str) -> RepoChanges {
    repo.status = "unavailable".into();
    repo.error = Some(cap_error(error));
    repo
}

fn empty_changes(target: &Target) -> RepoChanges {
    RepoChanges {
        root: lossy(&target.root),
        label: target.label.clone(),
        target: target.kind.to_string(),
        status: "captured".into(),
        error: None,
        head_before: None,
        head_after: None,
        files: Vec::new(),
        total_files: 0,
        truncated: false,
        cwd_in_repo: target.cwd_in_repo.clone(),
        alias_roots: target.alias_roots.clone(),
        vault_prefix: target.vault_prefix.clone(),
    }
}

fn file_row(path: &str, change_type: &str, status: &str) -> RepoFileChange {
    RepoFileChange {
        path: path.to_string(),
        change_type: change_type.to_string(),
        status: status.to_string(),
        untracked: false,
        gitlink: false,
        additions: None,
        deletions: None,
        binary: false,
        oversized: false,
        patch_truncated: false,
        withheld: false,
        dirty_before: false,
        attribution: observed(),
        tool: None,
        patch: None,
    }
}

fn change_from_raw(raw: &RawRow) -> &'static str {
    match raw.status.as_str() {
        "A" => "create",
        "D" => "delete",
        _ => "modify",
    }
}

async fn capture_one(
    target: &Target,
    limits: &Limits,
    deadline: Instant,
    hash_budget: &mut u64,
) -> RepoChanges {
    let mut repo = empty_changes(target);
    let pre = match &target.pre {
        Ok(pre) => pre,
        Err(error) => return unavailable(repo, error),
    };
    repo.head_before = pre.head.clone();
    let filters = match filter_drivers(target, deadline, None, limits).await {
        Ok(filters) => filters,
        Err(error) => return unavailable(repo, &error),
    };
    let head_after = match read_head(target, &filters, deadline, None, limits).await {
        Ok(head) => head,
        Err(error) => return unavailable(repo, &error),
    };
    repo.head_after = head_after.clone();
    let post = match read_status(target, &filters, deadline, None, limits).await {
        Ok(post) => post,
        Err(error) => return unavailable(repo, &error),
    };
    let committed = match (&head_after, head_after != pre.head) {
        (Some(after), true) => {
            let base = pre.head.as_deref().unwrap_or(EMPTY_TREE);
            let mut cmd = git_command(&target.root, Some(&target.git_dir), &filters);
            cmd.args([
                "diff",
                "--raw",
                "-z",
                "--no-abbrev",
                "--no-renames",
                "--submodule=short",
                "--no-ext-diff",
                "--no-textconv",
                base,
                after.as_str(),
            ]);
            match run_git(cmd, limits.max_status_bytes, deadline, limits, None).await {
                Ok(out) => match out.complete("committed diff") {
                    Ok(bytes) => parse_raw_z(&bytes),
                    Err(error) => return unavailable(repo, &error),
                },
                Err(error) => return unavailable(repo, &error),
            }
        }
        _ => BTreeMap::new(),
    };

    // Signatures now: post paths dirty before (to compare) and pre paths now
    // clean (to tell a removed untracked file from a reverted one).
    let wanted: Vec<(String, bool)> = post
        .keys()
        .filter(|path| pre.entries.contains_key(*path))
        .map(|path| {
            let hash = matches!(pre.sigs.get(path), Some(FileSig::Hash { .. }));
            (path.clone(), hash)
        })
        .chain(
            pre.entries
                .keys()
                .filter(|path| !post.contains_key(*path))
                .map(|path| (path.clone(), false)),
        )
        .collect();
    let (now_sigs, left) = compute_sigs(&target.root, wanted, *hash_budget, deadline).await;
    *hash_budget = left;

    let mut rows: BTreeMap<String, RepoFileChange> = BTreeMap::new();
    for (path, xy) in &post {
        let raw = committed.get(path);
        let dirty_before = pre.entries.contains_key(path);
        if dirty_before && raw.is_none() {
            let same_xy = pre.entries.get(path) == Some(xy);
            let same_sig = match (pre.sigs.get(path), now_sigs.get(path)) {
                (Some(before), Some(now)) => before == now,
                _ => false,
            };
            if same_xy && same_sig {
                continue;
            }
        }
        let change_type = if xy.contains('D') {
            "delete"
        } else if xy == "??" {
            "create"
        } else if let Some(raw) = raw {
            change_from_raw(raw)
        } else if xy.contains('A') {
            "create"
        } else {
            "modify"
        };
        let mut row = file_row(path, change_type, xy);
        row.untracked = xy == "??";
        row.dirty_before = dirty_before;
        row.gitlink = raw.is_some_and(RawRow::gitlink);
        rows.insert(path.clone(), row);
    }
    for (path, pre_xy) in &pre.entries {
        if post.contains_key(path) {
            continue;
        }
        let mut row = match committed.get(path) {
            Some(raw) => {
                let mut row = file_row(path, change_from_raw(raw), &raw.status);
                row.gitlink = raw.gitlink();
                row
            }
            None if pre_xy == "??" => {
                let gone = matches!(now_sigs.get(path), Some(FileSig::Missing) | None);
                let mut row = file_row(path, if gone { "delete" } else { "modify" }, "clean");
                row.untracked = true;
                row
            }
            None => file_row(path, "modify", "clean"),
        };
        row.dirty_before = true;
        rows.insert(path.clone(), row);
    }
    for (path, raw) in &committed {
        if rows.contains_key(path) {
            continue;
        }
        let mut row = file_row(path, change_from_raw(raw), &raw.status);
        row.gitlink = raw.gitlink();
        rows.insert(path.clone(), row);
    }

    repo.total_files = rows.len();
    repo.files = rows.into_values().take(limits.max_files).collect();
    if repo.total_files > repo.files.len() {
        repo.truncated = true;
    }
    let base = pre.head.clone().unwrap_or_else(|| EMPTY_TREE.to_string());
    add_patches(&mut repo, target, &filters, &base, pre, deadline, limits).await;
    repo
}

/// What a row's path is now, from one `symlink_metadata`.
#[derive(Debug, Clone, Copy)]
enum PathKind {
    Regular(u64),
    Symlink,
    Special,
    Missing,
}

async fn path_kinds(
    root: &Path,
    paths: Vec<String>,
    deadline: Instant,
) -> BTreeMap<String, PathKind> {
    if paths.is_empty() {
        return BTreeMap::new();
    }
    let root = root.to_path_buf();
    let remaining = deadline.saturating_duration_since(Instant::now());
    let task = tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| {
                let kind = match std::fs::symlink_metadata(root.join(&path)) {
                    Ok(meta) if meta.file_type().is_symlink() => PathKind::Symlink,
                    Ok(meta) if meta.is_file() => PathKind::Regular(meta.len()),
                    Ok(meta) if meta.is_dir() => PathKind::Missing,
                    Ok(_) => PathKind::Special,
                    Err(_) => PathKind::Missing,
                };
                (path, kind)
            })
            .collect::<BTreeMap<String, PathKind>>()
    });
    match tokio::time::timeout(remaining, task).await {
        Ok(Ok(kinds)) => kinds,
        _ => BTreeMap::new(),
    }
}

/// Bounded patches for the listed rows: one numstat and one capped patch
/// stream for the tracked rows, direct bounded reads for untracked files.
/// Running out of time keeps every row (without a patch) and sets
/// `truncated`; it never makes the repository unavailable.
async fn add_patches(
    repo: &mut RepoChanges,
    target: &Target,
    filters: &[String],
    base: &str,
    pre: &Snapshot,
    deadline: Instant,
    limits: &Limits,
) {
    let kinds = path_kinds(
        &target.root,
        repo.files.iter().map(|row| row.path.clone()).collect(),
        deadline,
    )
    .await;
    let mut tracked: Vec<usize> = Vec::new();
    let mut untracked: Vec<usize> = Vec::new();
    for (index, row) in repo.files.iter_mut().enumerate() {
        if is_secret_path(&row.path) {
            row.withheld = true;
            continue;
        }
        if row.gitlink {
            continue;
        }
        let kind = kinds.get(&row.path).copied();
        if matches!(kind, Some(PathKind::Special)) {
            row.status = "special".into();
            continue;
        }
        let size = match kind {
            Some(PathKind::Regular(len)) => Some(len),
            _ => pre.sigs.get(&row.path).and_then(FileSig::len),
        };
        if size.is_some_and(|len| len > limits.max_file_bytes) {
            row.oversized = true;
            continue;
        }
        if kind.is_none() && !matches!(row.change_type.as_str(), "delete") {
            // No kind: the budget ran out while looking.
            repo.truncated = true;
            continue;
        }
        if row.status == "??" {
            untracked.push(index);
        } else if !(row.untracked && row.change_type == "delete") {
            tracked.push(index);
        }
    }

    #[cfg(test)]
    let expired = EXPIRE_BEFORE_PATCHES.with(|flag| flag.get());
    #[cfg(not(test))]
    let expired = false;
    if expired || Instant::now() >= deadline {
        if !tracked.is_empty() || !untracked.is_empty() {
            repo.truncated = true;
        }
        return;
    }

    let mut used = 0usize;
    if !tracked.is_empty() {
        let paths: Vec<String> = tracked
            .iter()
            .map(|&i| repo.files[i].path.clone())
            .collect();

        let mut cmd = git_command(&target.root, Some(&target.git_dir), filters);
        cmd.args(["diff", "--numstat", "-z"])
            .args(WORKTREE_DIFF_FLAGS)
            .arg(base)
            .arg("--")
            .args(&paths);
        if let Ok(out) = run_git(cmd, NUMSTAT_CAP, deadline, limits, None).await {
            if let Ok(bytes) = out.complete("numstat") {
                let counts = parse_numstat_z(&bytes);
                for &index in &tracked {
                    if let Some((added, deleted, binary)) = counts.get(&repo.files[index].path) {
                        let row = &mut repo.files[index];
                        row.additions = *added;
                        row.deletions = *deleted;
                        row.binary = *binary;
                    }
                }
            }
        }

        let cap = limits
            .max_total_patch_bytes
            .saturating_add(limits.stream_slack);
        let mut cmd = git_command(&target.root, Some(&target.git_dir), filters);
        cmd.arg("diff")
            .args(WORKTREE_DIFF_FLAGS)
            .arg(base)
            .arg("--")
            .args(&paths);
        match run_git(cmd, cap, deadline, limits, None).await {
            Ok(out) if out.truncated || out.success => {
                let stream_truncated = out.truncated;
                let sections = split_patch_stream(&out.stdout, limits.max_patch_bytes);
                let last = sections.len().saturating_sub(1);
                let mut stopped = false;
                for (n, (path, text, capped)) in sections.into_iter().enumerate() {
                    let Some(index) = tracked
                        .iter()
                        .copied()
                        .find(|&i| repo.files[i].path == path)
                    else {
                        continue;
                    };
                    let (text, recapped) =
                        cap_patch(crate::claude_runner::redact(&text), limits.max_patch_bytes);
                    if stopped || used.saturating_add(text.len()) > limits.max_total_patch_bytes {
                        stopped = true;
                        repo.truncated = true;
                        continue;
                    }
                    used += text.len();
                    let row = &mut repo.files[index];
                    row.patch_truncated = capped || recapped || (stream_truncated && n == last);
                    row.patch = Some(text);
                }
                // The stream was cut: its last section is partial and every
                // file after it has no patch.
                if stream_truncated {
                    repo.truncated = true;
                }
            }
            // A failed patch read (a missing object with lazy fetch refused,
            // the budget): rows stay listed without patches.
            _ => repo.truncated = true,
        }
    }

    if untracked.is_empty() {
        return;
    }
    let root = target.root.clone();
    let items: Vec<(usize, String)> = untracked
        .iter()
        .map(|&i| (i, repo.files[i].path.clone()))
        .collect();
    let limits_copy = *limits;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let task = tokio::task::spawn_blocking(move || {
        items
            .into_iter()
            .map(|(index, path)| {
                if Instant::now() >= deadline {
                    return (index, None);
                }
                (
                    index,
                    Some(read_untracked(&root, &path, limits_copy.max_file_bytes)),
                )
            })
            .collect::<Vec<_>>()
    });
    let read = match tokio::time::timeout(remaining, task).await {
        Ok(Ok(read)) => read,
        _ => {
            repo.truncated = true;
            return;
        }
    };
    for (index, content) in read {
        let Some(content) = content else {
            repo.truncated = true;
            continue;
        };
        let path = repo.files[index].path.clone();
        let row = &mut repo.files[index];
        let text = match content {
            Untracked::Text { bytes, executable } => {
                row.additions = Some(line_count(&bytes));
                row.deletions = Some(0);
                untracked_patch(&path, &bytes, if executable { "100755" } else { "100644" })
            }
            Untracked::Binary => {
                row.binary = true;
                format!("diff --git a/{path} b/{path}\nnew file mode 100644\nBinary files /dev/null and b/{path} differ\n")
            }
            Untracked::Symlink(link) => {
                row.additions = Some(1);
                row.deletions = Some(0);
                untracked_patch(&path, link.as_bytes(), "120000")
            }
            Untracked::Oversized => {
                row.oversized = true;
                continue;
            }
            Untracked::Special => {
                row.status = "special".into();
                continue;
            }
            Untracked::Missing => continue,
        };
        let (text, capped) = cap_patch(crate::claude_runner::redact(&text), limits.max_patch_bytes);
        if used.saturating_add(text.len()) > limits.max_total_patch_bytes {
            repo.truncated = true;
            continue;
        }
        used += text.len();
        let row = &mut repo.files[index];
        row.patch_truncated = capped;
        row.patch = Some(text);
    }
}

enum Untracked {
    Text { bytes: Vec<u8>, executable: bool },
    Binary,
    Symlink(String),
    Oversized,
    Special,
    Missing,
}

/// Read an untracked file for its patch: a symlink records its target (never
/// followed), anything but a regular file inside the root is `Special`, more
/// than `cap` bytes is `Oversized`, a NUL early on is `Binary`.
fn read_untracked(root: &Path, rel: &str, cap: u64) -> Untracked {
    let path = root.join(rel);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(_) => return Untracked::Missing,
    };
    if meta.file_type().is_symlink() {
        return Untracked::Symlink(
            std::fs::read_link(&path)
                .map(|target| target.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
    }
    if !meta.is_file() {
        return Untracked::Special;
    }
    if meta.len() > cap {
        return Untracked::Oversized;
    }
    if !inside_root(root, &path) {
        return Untracked::Special;
    }
    let Some(file) = open_regular(&path) else {
        return Untracked::Special;
    };
    let executable = executable(&file);
    let mut bytes = Vec::new();
    if file.take(cap + 1).read_to_end(&mut bytes).is_err() {
        return Untracked::Special;
    }
    if bytes.len() as u64 > cap {
        return Untracked::Oversized;
    }
    if bytes.iter().take(BINARY_SNIFF_BYTES).any(|b| *b == 0) {
        return Untracked::Binary;
    }
    Untracked::Text { bytes, executable }
}

fn executable(file: &std::fs::File) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        false
    }
}

fn line_count(bytes: &[u8]) -> u64 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    if bytes.ends_with(b"\n") {
        newlines
    } else {
        newlines + 1
    }
}

/// A `new file` patch for an untracked file, in git's own shape.
fn untracked_patch(path: &str, bytes: &[u8], mode: &str) -> String {
    let mut out = format!("diff --git a/{path} b/{path}\nnew file mode {mode}\n");
    if bytes.is_empty() {
        return out;
    }
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    out.push_str(&format!(
        "--- /dev/null\n+++ b/{path}\n@@ -0,0 +1{} @@\n",
        if lines.len() == 1 {
            String::new()
        } else {
            format!(",{}", lines.len())
        }
    ));
    for line in &lines {
        out.push('+');
        out.push_str(line);
    }
    if !text.ends_with('\n') {
        out.push_str("\n\\ No newline at end of file\n");
    }
    out
}

// ---------------------------------------------------------------- strings

/// `s` cut to at most `max` bytes at a character boundary.
fn truncate_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn cap_patch(text: String, max: usize) -> (String, bool) {
    if text.len() <= max {
        (text, false)
    } else {
        (truncate_char_boundary(&text, max).to_string(), true)
    }
}

/// An error line for the receipt: one line, at most [`MAX_ERROR_CHARS`]
/// characters (never cut inside one).
fn cap_error(error: &str) -> String {
    let one_line = error.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_ERROR_CHARS {
        one_line
    } else {
        format!(
            "{}…",
            one_line
                .chars()
                .take(MAX_ERROR_CHARS - 1)
                .collect::<String>()
        )
    }
}

// ------------------------------------------------------------ test helpers

/// A captured repository with `paths` as observed modifications, rooted at
/// the alias `/work/app` (evidence and change-feed tests).
#[cfg(test)]
pub(crate) fn test_repo(label: &str, paths: &[&str]) -> RepoChanges {
    RepoChanges {
        root: "/work/app".into(),
        label: label.into(),
        target: "cwd".into(),
        status: "captured".into(),
        error: None,
        head_before: Some("a".repeat(40)),
        head_after: Some("a".repeat(40)),
        files: paths
            .iter()
            .map(|path| file_row(path, "modify", " M"))
            .collect(),
        total_files: paths.len(),
        truncated: false,
        cwd_in_repo: None,
        alias_roots: vec!["/work/app".into()],
        vault_prefix: None,
    }
}

/// A non-empty, unarmed baseline (collector seam tests).
#[cfg(test)]
pub(crate) fn test_baseline() -> RepoBaseline {
    RepoBaseline {
        targets: vec![Target {
            root: PathBuf::from("/work/app"),
            git_dir: PathBuf::from("/work/app/.git"),
            label: "app".into(),
            kind: "cwd",
            cwd_in_repo: Some(String::new()),
            alias_roots: vec!["/work/app".into()],
            vault_prefix: None,
            pre: Ok(Snapshot::default()),
        }],
        limits: LIMITS,
        armed: false,
    }
}

#[cfg(test)]
#[path = "repo_diff_tests.rs"]
mod tests;
