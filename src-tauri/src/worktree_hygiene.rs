//! Worktree and branch hygiene (overnight A9). For every repository a build
//! workspace came from (one entry per repository, however many worktrees
//! the bindings name), [`report`] lists:
//! - its linked worktrees: merged into the remote's default branch, clean,
//!   with no commits on no remote, the build workspace bound to it, what it
//!   holds that git ignores (deleted with it), and whether a process runs in
//!   it;
//! - local-only branches (commits no remote has), listed and never deleted;
//! - literal `~` folders (`<worktree>/~`, `<worktree>/src-tauri/~`), left by
//!   an unexpanded tilde in `CARGO_TARGET_DIR` (A4), with their sizes.
//!
//! [`clean`] removes only what the report marks removable: merged, clean,
//! fully pushed, unlocked worktrees nothing runs in, holding no ignored file
//! beyond rebuildable output (a `.env` or a local database keeps it, as it
//! keeps the gardener's `worktree:` line), not sitting exactly on
//! the default branch's tip (a lane that has only just started looks the
//! same as a finished one there), and not at the commit the installed app or
//! its rollback copy was built from (their `build-provenance.json`,
//! `install_targets::app_commits`; overnight H20: the release checkout that
//! built the installed app is otherwise merged, clean and pushed, and would
//! go), and not the folder either app was built in (the provenance's
//! `checkout`, `install_targets::app_checkouts`) or a build running, waiting
//! to install or installing still needs (`self_install::held_worktrees`;
//! overnight S01a: a checkout that has moved past both apps' commits matches
//! neither; read again right before each removal, overnight R2, so a Build
//! or Install pressed after the list was made keeps its worktree, and for a
//! bound one again right before `git worktree remove`, overnight R4). A
//! bound one goes through
//! `build_workspaces::remove_for`, so its workspace falls back to its vault
//! folder; an unbound one through `git worktree remove` without `--force`,
//! which itself refuses tracked or untracked changes (ignored rebuildable
//! output goes with the folder; the card names it). Plus the `~` folders of worktrees
//! nothing runs in. Only through the confirm-tier `build.clean_worktrees`,
//! bound to the exact list (and how each goes) its card showed, or the
//! person's Settings, Advanced, Disk space, Remove finished worktrees…
//! (overnight H20; `clean_worktrees` with its confirm dialog's hash). Never
//! automatic, never `--force`, never a branch.
//!
//! **Reclaiming generated output (OC6, 2026-11-16).** The report also lists,
//! per worktree [`clean`] keeps (unmerged, unpushed or changed ones), the
//! ignored build folders ([`RECLAIM_DIRS`]: `node_modules`, `target`, `dist`,
//! `src-tauri/target`) that could go without touching the worktree, its
//! branch or any work in it, and a hash of that list (`reclaim_hash`). Only
//! an idle worktree is listed: no process in it, not locked, no build
//! workspace bound to it, not what an app or build came from, HEAD and the
//! git index untouched for [`RECLAIM_IDLE_DAYS`], every changed path
//! untouched for [`RECLAIM_CHANGE_DAYS`], and each folder ignored by git,
//! holding no tracked file, a real folder inside the worktree and untouched
//! for [`RECLAIM_IDLE_DAYS`] (newest change in its top two levels). Anything
//! that can't be dated (an unreadable folder or entry, more than the walk's
//! entry cap) counts as touched, and a folder other than `node_modules` that
//! holds a `.env*` file or a local database (`gardener::rebuildable`, the
//! rule `clean` uses) is never offered. **This is a dry report:** the
//! apply step, [`reclaim_output`], is bound to the report's hash, rechecks
//! each worktree right before removing and deletes only those folders; it
//! is not wired to an action yet, so nothing deletes today.

use crate::build_workspaces::{self, git, git_raw, QUICK};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// One linked worktree.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    /// Its branch, or `None` when detached.
    pub branch: Option<String>,
    /// The build workspace bound to it.
    pub workspace: Option<String>,
    /// Its HEAD is in the remote's default branch.
    pub merged: bool,
    /// No tracked change and no untracked file (`-unormal`).
    pub clean: bool,
    /// Commits at HEAD that no remote has.
    pub unpushed: u32,
    /// Top-level entries git ignores (rebuildable output), at most 8,
    /// and how many there are in all: removing the worktree deletes them.
    pub ignored: Vec<String>,
    pub ignored_count: u32,
    /// A process's working folder is inside it.
    pub in_use: bool,
    /// [`clean`] would remove it.
    pub removable: bool,
    /// Why it stays, when it does.
    pub reason: Option<String>,
    /// Generated folders it could shed while it stays (OC6): empty unless
    /// the worktree is idle (see the module docs).
    pub reclaim: Vec<ReclaimDir>,
}

/// A generated folder a worktree could shed without losing anything (OC6).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReclaimDir {
    /// Relative to the worktree (`node_modules`, `src-tauri/target`).
    pub path: String,
    /// Bytes on disk, when sizes were read and `du` answered.
    pub bytes: Option<u64>,
    /// Whole days since anything in its top two levels changed.
    pub idle_days: u64,
}

/// A branch with commits no remote has.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalBranch {
    pub name: String,
    pub commits: u32,
    /// Its newest commit's date (ISO 8601).
    pub last_commit: String,
}

/// A literal `~` folder.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TildeFolder {
    pub path: String,
    /// Bytes on disk, when sizes were read and `du` answered.
    pub bytes: Option<u64>,
}

/// One repository's findings.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    /// Its main worktree.
    pub repo: String,
    /// `origin/main` or similar; `None` when the remote has no default.
    pub default_branch: Option<String>,
    pub worktrees: Vec<Worktree>,
    pub local_only_branches: Vec<LocalBranch>,
    pub tilde_folders: Vec<TildeFolder>,
    /// A git error that cut this repository's report short.
    pub error: Option<String>,
}

/// The whole report and what [`clean`] would remove.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub repos: Vec<Repo>,
    /// Worktrees [`clean`] removes, then `~` folders.
    pub remove: Vec<String>,
    /// Binds a confirm card to exactly `remove`, and to how each worktree
    /// goes (through its workspace or with git).
    pub hash: String,
    /// The `~` folders' total, where known.
    pub tilde_bytes: u64,
    /// Running processes couldn't be read, so nothing is removable.
    pub in_use_unknown: bool,
    /// Binds [`reclaim_output`] to exactly the folders each worktree's
    /// `reclaim` lists (sizes left out: they change as a build runs).
    pub reclaim_hash: String,
    /// The listed folders' total, where sizes were read.
    pub reclaim_bytes: u64,
}

/// What [`clean`] did.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Cleaned {
    pub removed: Vec<String>,
    pub failed: Vec<(String, String)>,
    /// Worktrees and `~` folders that were due to go but weren't reached
    /// before the time budget ran out; a second clean (a new card) takes them.
    #[serde(default)]
    pub remaining: Vec<String>,
    /// Every one due was reached (an old payload without it was).
    #[serde(default = "yes")]
    pub complete: bool,
    /// Free space read from each volume the worktrees sit on, before the
    /// first removal and after the last (statvfs, not `du`: APFS clones share
    /// blocks, so adding up sizes overstates what deleting them frees).
    #[serde(default)]
    pub volumes: Vec<VolumeFree>,
    /// Summed over `volumes`: free bytes before, after, and the measured
    /// gain (never negative; other writers can eat into it).
    #[serde(default)]
    pub free_before: u64,
    #[serde(default)]
    pub free_after: u64,
    #[serde(default)]
    pub freed: u64,
    /// How long the clean ran.
    #[serde(default)]
    pub elapsed_ms: u64,
}

fn yes() -> bool {
    true
}

impl Default for Cleaned {
    fn default() -> Self {
        Cleaned {
            removed: Vec::new(),
            failed: Vec::new(),
            remaining: Vec::new(),
            complete: true,
            volumes: Vec::new(),
            free_before: 0,
            free_after: 0,
            freed: 0,
            elapsed_ms: 0,
        }
    }
}

/// One volume's free space around a clean.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VolumeFree {
    /// A folder on it (the repository).
    pub path: String,
    pub free_before: u64,
    pub free_after: u64,
    /// `free_after - free_before`, floored at 0.
    pub freed: u64,
}

/// How long after [`clean_for`] is entered (its own report read included)
/// it starts new removals: past it, what is left is listed in `remaining`
/// instead of the action timing out with nothing said. The broker gives a
/// change 450 s in all, of which up to 120 s may go on the card's Allow
/// (`app_mcp::BROKER_CHANGE_TIMEOUT_SECS`), so 330 s are left once it runs.
/// The last removal may start at 60 s; before its box opens it reads the
/// holds again (`held_now`, up to 20 s) and keeps the build record; the box
/// is [`ONE_REMOVAL`] (90 s) for a bound workspace's service stop and checks;
/// then git's own 120 s ([`GIT_REMOVE`]) for the delete, then a few seconds
/// for the binding's `put` and the final free-space read: about 60 + 20 + 90
/// + 120 + 10 = 300 s, inside the 330 s.
const CLEAN_BUDGET: Duration = Duration::from_secs(60);
/// The longest a bound workspace's service stop and checks may take before
/// the delete starts (a delete already started is never cut off).
const ONE_REMOVAL: Duration = Duration::from_secs(90);
/// `git worktree remove`'s own limit.
const GIT_REMOVE: Duration = Duration::from_secs(120);
const IDLE: u8 = 0;
const GOING: u8 = 1;
const CUT: u8 = 2;
use std::sync::atomic::Ordering::SeqCst;
const CUT_MESSAGE: &str =
    "stopping its services and checking it took too long; nothing was deleted, so run the clean again";

/// A [`build_workspaces::git`] error from its own time limit.
fn timed_out(e: &str) -> bool {
    e.contains("took longer than")
}

/// `~` folders whose delete is running (one that outran its limit keeps
/// going), so a second clean doesn't start a second delete of the same one.
static DELETING: std::sync::Mutex<BTreeSet<PathBuf>> = std::sync::Mutex::new(BTreeSet::new());

/// What a cut-off delete leaves behind.
const PARTLY: &str = "it may be partly deleted; finish with `git worktree remove --force <path>` or remove the workspace";
/// Worktrees read at once by the report (each is a handful of git calls).
const SCAN_PARALLEL: usize = 12;

/// Days HEAD, the git index and a generated folder must sit untouched
/// before the folder is offered for reclaiming (OC6).
pub(crate) const RECLAIM_IDLE_DAYS: u64 = 30;
/// Days every changed path of a worktree must sit untouched (OC6).
pub(crate) const RECLAIM_CHANGE_DAYS: u64 = 7;
/// The generated folders a worktree may shed, relative to it. Each is
/// checked to be ignored by git and to hold no tracked file.
pub(crate) const RECLAIM_DIRS: [&str; 4] = ["node_modules", "target", "dist", "src-tauri/target"];
/// Changed paths read at most before a worktree counts as not idle.
const RECLAIM_STATUS_CAP: usize = 2_000;
/// Directory entries read at most to date one folder: past it the folder
/// counts as touched (unknown is not idle).
const RECLAIM_WALK_CAP: usize = 300_000;

/// How much [`report_for`] reads.
#[derive(Debug, Clone, Default)]
pub(crate) struct Opts {
    /// `du` each `~` folder (seconds for gigabytes).
    pub sizes: bool,
    /// List local-only branches (a `rev-list` per branch).
    pub branches: bool,
    /// The processes' working folders.
    pub cwds: Cwds,
    /// The commits and folders whose worktrees stay.
    pub held: Held,
    /// How long [`clean_for`] starts removals ([`CLEAN_BUDGET`] when `None`).
    pub budget: Option<Duration>,
    /// Read each kept worktree's generated folders (OC6): a `git status`
    /// and a folder walk per worktree, which [`clean_for`] never needs.
    pub reclaim: bool,
}

/// Where [`report_for`] gets the processes' working folders.
#[derive(Debug, Clone, Default)]
pub(crate) enum Cwds {
    /// From `lsof`.
    #[default]
    Read,
    /// These (tests).
    #[cfg(test)]
    Given(Vec<PathBuf>),
    /// As if `lsof` had failed (tests).
    #[cfg(test)]
    Unknown,
}

/// Where [`report_for`] gets the commits and folders whose worktrees stay:
/// the installed app's and its rollback copy's, and a build's that still
/// needs its worktree, each with why.
#[derive(Debug, Clone, Default)]
pub(crate) enum Held {
    /// From the apps' `build-provenance.json` (`install_targets::app_commits`
    /// and `app_checkouts`) and the build job (`self_install::held_worktrees`).
    #[default]
    Read,
    /// These (tests).
    #[cfg(test)]
    Given(Holds),
    /// `scan` for the report, then `removal` when [`clean_for`] reads them
    /// again right before each removal (tests: a Build pressed in between).
    #[cfg(test)]
    Later { scan: Holds, removal: Holds },
    /// None until a bound worktree's removal reads them after its services
    /// stopped, then these (tests: a Build pressed during those steps).
    #[cfg(test)]
    During(Holds),
}

impl Held {
    /// What the report reads, folders canonical.
    fn scan(&self) -> Holds {
        match self {
            Held::Read => Holds::read(),
            #[cfg(test)]
            Held::Given(h) | Held::Later { scan: h, .. } => h.clone(),
            #[cfg(test)]
            Held::During(_) => Holds::default(),
        }
        .canonical()
    }

    /// What [`clean_for`] reads again right before each removal (overnight
    /// R2), folders canonical.
    fn removal(&self) -> Holds {
        match self {
            Held::Read => Holds::read(),
            #[cfg(test)]
            Held::Given(h) | Held::Later { removal: h, .. } => h.clone(),
            #[cfg(test)]
            Held::During(_) => Holds::default(),
        }
        .canonical()
    }

    /// What a bound worktree's removal reads again after its services
    /// stopped, right before `git worktree remove` (overnight R4), folders
    /// canonical.
    fn in_removal(&self) -> Holds {
        #[cfg(test)]
        if let Held::During(h) = self {
            return h.clone().canonical();
        }
        self.removal()
    }
}

/// What keeps a worktree because an app or a build came from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Holds {
    /// (commit, why): the installed app's and its rollback copy's.
    pub commits: Vec<(String, String)>,
    /// (folder, why): where those apps were built (their provenance's
    /// `checkout`), and the worktree of a build running, waiting to install
    /// or installing (overnight S01a: such a checkout may have moved past
    /// both apps' commits and look finished).
    pub paths: Vec<(PathBuf, String)>,
}

impl Holds {
    /// This Mac's: the apps in /Applications and the build job (the
    /// gardener reads the same, overnight S01e).
    pub(crate) fn read() -> Holds {
        let mut paths = crate::install_targets::app_checkouts();
        paths.extend(crate::self_install::held_worktrees());
        Holds {
            commits: crate::install_targets::app_commits(),
            paths,
        }
    }

    /// The same with every folder canonical, so a link or `/tmp` for
    /// `/private/tmp` still matches the worktree's own canonical path.
    pub(crate) fn canonical(self) -> Holds {
        Holds {
            commits: self.commits,
            paths: self
                .paths
                .into_iter()
                .map(|(p, why)| (p.canonicalize().unwrap_or(p), why))
                .collect(),
        }
    }
}

impl Opts {
    /// Everything, for the read action and the card.
    pub(crate) fn full() -> Opts {
        Opts {
            sizes: true,
            branches: true,
            cwds: Cwds::Read,
            held: Held::Read,
            budget: None,
            reclaim: true,
        }
    }
}

/// Each of `held`'s reasons whose commit is `head`: an app was built from it
/// (both when the installed app and its rollback copy share the commit).
/// Abbreviated commits match (`install_targets::same_commit`).
fn commit_reasons<'a>(head: Option<&str>, held: &'a [(String, String)]) -> Vec<&'a str> {
    let Some(head) = head else {
        return Vec::new();
    };
    held.iter()
        .filter(|(commit, _)| {
            commit == crate::install_targets::ANY_COMMIT
                || crate::install_targets::same_commit(commit, head)
        })
        .map(|(_, why)| why.as_str())
        .collect()
}

/// Why the worktree at `head` whose canonical folder is `real` stays because
/// an app or a build came from it: each held commit's reason that is `head`
/// ([`commit_reasons`]), then each held folder's that is `real`, each reason
/// once; or `None`.
pub(crate) fn held_for(head: Option<&str>, real: &Path, held: &Holds) -> Option<String> {
    let by_path = held
        .paths
        .iter()
        .filter(|(p, _)| p == real)
        .map(|(_, why)| why.as_str());
    let mut why: Vec<&str> = Vec::new();
    for w in commit_reasons(head, &held.commits)
        .into_iter()
        .chain(by_path)
    {
        if !why.contains(&w) {
            why.push(w);
        }
    }
    (!why.is_empty()).then(|| why.join("; "))
}

/// Why the worktree at `dir` stays after all, from `held` read again right
/// before it is removed (overnight R2: a Build or Install pressed between
/// the list and the removal needs it): "Kept: …", or `None`. The callers
/// hold no lock here (`self_install::held_worktrees` takes the build job's).
pub(crate) async fn held_now(dir: &Path, held: &Holds) -> Option<String> {
    let real = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let head = git(dir, &["rev-parse", "HEAD"], QUICK).await.ok();
    held_for(head.as_deref(), &real, held)
        .map(|why| format!("Kept: {why}, since the list was shown"))
}

/// One `git worktree list --porcelain` entry after the main worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listed {
    pub path: String,
    pub branch: Option<String>,
    pub locked: bool,
}

/// `git worktree list --porcelain` → the main worktree's path and each
/// linked, non-bare worktree.
pub(crate) fn parse_worktrees(porcelain: &str) -> (Option<String>, Vec<Listed>) {
    let mut main = None;
    let mut out = Vec::new();
    for (i, block) in porcelain.split("\n\n").enumerate() {
        let mut path = None;
        let mut branch = None;
        let mut bare = false;
        let mut locked = false;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(p.to_string());
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.trim_start_matches("refs/heads/").to_string());
            } else if line == "bare" {
                bare = true;
            } else if line == "locked" || line.starts_with("locked ") {
                locked = true;
            }
        }
        if i == 0 {
            main = path;
            continue;
        }
        if let (Some(path), false) = (path, bare) {
            out.push(Listed {
                path,
                branch,
                locked,
            });
        }
    }
    (main, out)
}

/// The literal `~` folders Hollow looks for under `dir`: each reached only
/// through real folders, so a link never leads outside it.
fn tilde_candidates(dir: &Path) -> Vec<PathBuf> {
    let real_dir = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
    let mut out = Vec::new();
    if real_dir(dir) {
        out.push(dir.join("~"));
        if real_dir(&dir.join("src-tauri")) {
            out.push(dir.join("src-tauri").join("~"));
        }
    }
    out.into_iter().filter(|p| is_tilde_folder(p)).collect()
}

/// A real folder (not a link) named exactly `~`.
pub(crate) fn is_tilde_folder(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == "~")
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

async fn run_capped(
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
    limit: Duration,
) -> Option<String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = crate::process_group::run_to_completion_capped(&mut cmd, None, limit, None, 8 << 20)
        .await
        .ok()?;
    if out.timed_out || out.status.and_then(|s| s.code()) != Some(0) {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// lsof's escapes undone (`\xNN`, `\n`, `\t`, `\\`): without a UTF-8 locale
/// it writes a path's non-ASCII bytes that way, and "Alex’s" would never
/// match its worktree.
pub(crate) fn lsof_unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            let two = match b[i + 1] {
                b'x' if i + 3 < b.len() => std::str::from_utf8(&b[i + 2..i + 4])
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                    .map(|v| (v, 4)),
                b'n' => Some((b'\n', 2)),
                b't' => Some((b'\t', 2)),
                b'\\' => Some((b'\\', 2)),
                _ => None,
            };
            if let Some((byte, len)) = two {
                out.push(byte);
                i += len;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn du_bytes(path: &Path) -> Option<u64> {
    let text = run_capped(
        "du",
        &["-sk", &path.to_string_lossy()],
        &[],
        Duration::from_secs(120),
    )
    .await?;
    let kb: u64 = text.split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}

/// Every process's working folder (`lsof`, this user's processes), canonical.
async fn process_cwds() -> Option<Vec<PathBuf>> {
    // A UTF-8 locale so lsof prints paths as they are; unescape anyway.
    let text = run_capped(
        "lsof",
        &["-a", "-d", "cwd", "-Fn", "-w"],
        &[("LC_ALL", "en_US.UTF-8")],
        Duration::from_secs(20),
    )
    .await?;
    Some(
        text.lines()
            .filter_map(|l| l.strip_prefix('n'))
            .map(lsof_unescape)
            .map(|p| {
                Path::new(&p)
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from(&p))
            })
            .collect(),
    )
}

async fn count(dir: &Path, args: &[&str]) -> u32 {
    git(dir, args, QUICK)
        .await
        .ok()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0)
}

/// Top-level ignored entries of a worktree and their number, and whether it
/// is clean (the same read lists tracked and untracked changes too: no line
/// that isn't an ignored `!!` one, which saves a `git status` per worktree).
async fn ignored(dir: &Path) -> (Vec<String>, u32, Option<bool>) {
    let text = git(
        dir,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--ignored=traditional",
            "-unormal",
        ],
        QUICK,
    )
    .await
    .ok();
    let clean = text.as_ref().map(|t| {
        t.lines()
            .all(|l| l.starts_with("!! ") || l.trim().is_empty())
    });
    let text = text.unwrap_or_default();
    // At most two levels (`src-tauri/gen/`, not a bare `src-tauri/`, which
    // would read as the whole folder).
    let mut top = BTreeSet::new();
    for line in text.lines().filter_map(|l| l.strip_prefix("!! ")) {
        let line = line.trim_matches('"');
        let parts: Vec<&str> = line.trim_end_matches('/').split('/').collect();
        let shown = parts[..parts.len().min(2)].join("/");
        let dir = parts.len() > 2 || line.ends_with('/');
        top.insert(if dir { format!("{shown}/") } else { shown });
    }
    let n = top.len() as u32;
    (top.into_iter().take(8).collect(), n, clean)
}

/// Ignored files in a worktree that aren't rebuildable output (a `.env`, a
/// local database), at most 3, and how many: they keep the worktree, as they
/// do for the gardener's `worktree:` line (`gardener::rebuildable`). The
/// literal `~` folders are rebuildable cargo output, listed on their own.
async fn kept_ignored(dir: &Path) -> (Vec<String>, usize) {
    let text = git(
        dir,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "--ignored=matching",
            "-unormal",
        ],
        QUICK,
    )
    .await
    .unwrap_or_default();
    let kept: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("!! "))
        .map(|l| l.trim_matches('"').to_string())
        .filter(|p| !crate::gardener::rebuildable(p) && !p.split('/').any(|c| c == "~"))
        .collect();
    let n = kept.len();
    (kept.into_iter().take(3).collect(), n)
}

/// One worktree's state and verdict.
#[allow(clippy::too_many_arguments)]
async fn worktree(
    listed: Listed,
    workspace: Option<String>,
    default_branch: Option<&str>,
    default_tip: Option<&str>,
    cwds: Option<&[PathBuf]>,
    held: &Holds,
    sizes: bool,
    reclaim: bool,
    now: SystemTime,
) -> Worktree {
    let dir = Path::new(&listed.path);
    let mut w = Worktree {
        path: listed.path.clone(),
        branch: listed.branch.clone(),
        workspace,
        merged: false,
        clean: false,
        unpushed: 0,
        ignored: Vec::new(),
        ignored_count: 0,
        in_use: false,
        removable: false,
        reason: None,
        reclaim: Vec::new(),
    };
    if !dir.is_dir() {
        w.reason = Some("its folder is gone (`git worktree prune` forgets it)".into());
        return w;
    }
    let real = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    w.in_use = cwds.is_some_and(|c| c.iter().any(|p| p.starts_with(&real)));
    // The independent reads run together (a handful of git calls each).
    // --no-optional-locks: a report never rewrites a worktree's index.
    let (unpushed, ign, kept_read, head, merged) = tokio::join!(
        count(dir, &["rev-list", "--count", "HEAD", "--not", "--remotes"]),
        ignored(dir),
        kept_ignored(dir),
        async { git(dir, &["rev-parse", "HEAD"], QUICK).await.ok() },
        async {
            match default_branch {
                Some(base) => git_raw(
                    dir,
                    &["merge-base", "--is-ancestor", "HEAD", base],
                    QUICK,
                    0,
                )
                .await
                .is_ok_and(|o| o.code == Some(0)),
                None => false,
            }
        },
    );
    w.unpushed = unpushed;
    (w.ignored, w.ignored_count) = (ign.0, ign.1);
    // An unreadable status is not clean.
    w.clean = ign.2.unwrap_or(false);
    let (kept, kept_count) = kept_read;
    w.merged = merged;
    // The installed app's or its rollback copy's build came from here, or a
    // build still needs it: kept, however finished it looks (the release
    // checkout is merged and clean, and may have moved past both apps).
    let built = held_for(head.as_deref(), &real, held);
    let held_by_app = built.is_some();
    w.reason = if default_branch.is_none() {
        Some("the remote has no default branch to compare with".into())
    } else if listed.locked {
        Some("locked (`git worktree unlock` first)".into())
    } else if cwds.is_none() {
        Some("couldn't read what is running, so nothing is removed".into())
    } else if w.in_use {
        Some("a process is working in it".into())
    } else if built.is_some() {
        built
    } else if !w.clean {
        Some("uncommitted or untracked changes".into())
    } else if kept_count > 0 {
        let more = kept_count.saturating_sub(kept.len());
        let more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        Some(format!(
            "holds ignored files that aren't rebuildable output: {}{more}",
            kept.join(", ")
        ))
    } else if w.unpushed > 0 {
        Some(format!("{} commit(s) on no remote", w.unpushed))
    } else if !w.merged {
        Some("not merged into the default branch".into())
    } else if head.is_some() && head.as_deref() == default_tip {
        Some("exactly at the default branch's tip (it may have only just started)".into())
    } else {
        None
    };
    w.removable = w.reason.is_none();
    // OC6: a worktree that stays may still shed generated folders, when it
    // is idle. One a process works in, a lock, a bound workspace or an app's
    // or build's checkout never is; a removable one goes whole.
    if reclaim
        && !w.removable
        && !listed.locked
        && cwds.is_some()
        && !w.in_use
        && !held_by_app
        && w.workspace.is_none()
    {
        if let Ok(found) = reclaimable(dir, now).await {
            for (path, idle_days) in found {
                let bytes = if sizes {
                    du_bytes(&dir.join(&path)).await
                } else {
                    None
                };
                w.reclaim.push(ReclaimDir {
                    path,
                    bytes,
                    idle_days,
                });
            }
        }
    }
    w
}

/// The newest modification time of `root` and everything in its top `depth`
/// levels (symlinks not followed), or `None` when it can't be read or holds
/// more than `cap` entries: unknown counts as touched.
fn newest_mtime(root: &Path, depth: usize, cap: usize) -> Option<SystemTime> {
    let root_meta = std::fs::symlink_metadata(root).ok()?;
    let mut newest = root_meta.modified().ok()?;
    // A file (or a link) is dated by itself: there is nothing inside to read.
    if !root_meta.is_dir() {
        return Some(newest);
    }
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((folder, level)) = stack.pop() {
        // A folder or entry that can't be read can't be dated: unknown.
        let entries = std::fs::read_dir(&folder).ok()?;
        for entry in entries {
            let entry = entry.ok()?;
            seen += 1;
            if seen > cap {
                return None;
            }
            let meta = entry.metadata().ok()?;
            newest = newest.max(meta.modified().ok()?);
            if meta.is_dir() && level + 1 < depth {
                stack.push((entry.path(), level + 1));
            }
        }
    }
    Some(newest)
}

/// Whether the candidate `folder` (named `rel` in its worktree) holds a file
/// the gardener counts as the person's own, not output: a `.env*` file or a
/// local database (`gardener::rebuildable`, the one rule `clean` uses too),
/// at any depth. `Some(true)` when one is found, `Some(false)` when none is,
/// `None` when the folder can't be read or holds more than `cap` entries:
/// unknown is not clear. Symlinks are not followed.
fn holds_kept_file(folder: &Path, rel: &str, cap: usize) -> Option<bool> {
    let top = rel.rsplit('/').next().unwrap_or(rel);
    let mut stack = vec![(folder.to_path_buf(), top.to_string())];
    let mut seen = 0usize;
    while let Some((dir, shown)) = stack.pop() {
        for entry in std::fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            seen += 1;
            if seen > cap {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = format!("{shown}/{name}");
            if !crate::gardener::rebuildable(&path) {
                return Some(true);
            }
            if entry.metadata().ok()?.is_dir() {
                stack.push((entry.path(), path));
            }
        }
    }
    Some(false)
}

/// Why the worktree at `dir` is not idle as of `now`, or the generated
/// folders it could shed with each one's idle days (OC6; see the module
/// docs for every condition). Reads git and the file system, writes nothing.
async fn reclaimable(dir: &Path, now: SystemTime) -> Result<Vec<(String, u64)>, String> {
    let age_of = move |t: SystemTime| now.duration_since(t).unwrap_or_default();
    let idle = Duration::from_secs(RECLAIM_IDLE_DAYS * 86_400);
    let changed = Duration::from_secs(RECLAIM_CHANGE_DAYS * 86_400);
    let gitdir = git(dir, &["rev-parse", "--absolute-git-dir"], QUICK).await?;
    let commit_secs: u64 = git(dir, &["log", "-1", "--format=%ct", "HEAD"], QUICK)
        .await?
        .trim()
        .parse()
        .map_err(|_| "HEAD has no readable commit date".to_string())?;
    let mut moved = SystemTime::UNIX_EPOCH + Duration::from_secs(commit_secs);
    for name in ["HEAD", "logs/HEAD", "index"] {
        if let Ok(modified) =
            std::fs::metadata(Path::new(&gitdir).join(name)).and_then(|m| m.modified())
        {
            moved = moved.max(modified);
        }
    }
    if age_of(moved) < idle {
        return Err(format!(
            "HEAD or the index moved in the last {RECLAIM_IDLE_DAYS} days"
        ));
    }
    let status = git_raw(
        dir,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain",
            "-z",
            "-unormal",
        ],
        Duration::from_secs(60),
        16 << 20,
    )
    .await?;
    if status.code != Some(0) || status.truncated {
        return Err("git status could not be read in full".into());
    }
    let text = String::from_utf8_lossy(&status.stdout).into_owned();
    let mut records = text.split('\0').filter(|r| !r.is_empty());
    let mut changed_paths: Vec<String> = Vec::new();
    while let Some(record) = records.next() {
        let (Some(code), Some(path)) = (record.get(..3), record.get(3..)) else {
            continue;
        };
        if code == "!! " {
            continue;
        }
        // A rename or copy names its old path in the next record.
        if code.as_bytes()[..2]
            .iter()
            .any(|b| matches!(b, b'R' | b'C'))
        {
            records.next();
        }
        changed_paths.push(path.to_string());
        if changed_paths.len() > RECLAIM_STATUS_CAP {
            return Err(format!(
                "more than {RECLAIM_STATUS_CAP} changed paths to check"
            ));
        }
    }
    let real = dir
        .canonicalize()
        .map_err(|e| format!("couldn't resolve the folder: {e}"))?;
    let mut candidates: Vec<String> = Vec::new();
    for rel in RECLAIM_DIRS {
        let path = real.join(rel);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        // Resolved, it must still be a folder inside the worktree.
        if !path
            .canonicalize()
            .is_ok_and(|c| c.starts_with(&real) && c != real)
        {
            continue;
        }
        let ignored = git_raw(dir, &["check-ignore", "-q", "--", rel], QUICK, 0)
            .await
            .is_ok_and(|o| o.code == Some(0));
        let tracked = git(dir, &["ls-files", "--", rel], QUICK)
            .await
            .map(|t| !t.trim().is_empty())
            .unwrap_or(true);
        if ignored && !tracked {
            candidates.push(rel.to_string());
        }
    }
    let root = real.clone();
    tokio::task::spawn_blocking(move || {
        for path in &changed_paths {
            // An untracked folder is one entry in `git status`: look deep inside it.
            let depth = if path.ends_with('/') { 8 } else { 2 };
            match newest_mtime(&root.join(path), depth, RECLAIM_WALK_CAP) {
                Some(t) if age_of(t) >= changed => {}
                Some(_) => {
                    return Err(format!(
                        "a changed path was touched in the last {RECLAIM_CHANGE_DAYS} days"
                    ))
                }
                None => return Err("a changed path can't be dated".to_string()),
            }
        }
        let mut found = Vec::new();
        for rel in candidates {
            // `node_modules` is installed from the lockfile, so nothing of the
            // person's lives in it (packages ship `.env.example` files); every
            // other folder is checked for a `.env*` file or a local database.
            if rel != "node_modules"
                && holds_kept_file(&root.join(&rel), &rel, RECLAIM_WALK_CAP) != Some(false)
            {
                continue;
            }
            if let Some(t) = newest_mtime(&root.join(&rel), 2, RECLAIM_WALK_CAP) {
                let age = age_of(t);
                if age >= idle {
                    found.push((rel, age.as_secs() / 86_400));
                }
            }
        }
        Ok(found)
    })
    .await
    .map_err(|e| format!("the idle check stopped: {e}"))?
}

/// Branches with commits no remote has, newest first.
async fn local_only(repo: &Path) -> Vec<LocalBranch> {
    let refs = git(
        repo,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)\t%(committerdate:iso-strict)",
            "refs/heads",
        ],
        QUICK,
    )
    .await
    .unwrap_or_default();
    let mut out = Vec::new();
    for line in refs.lines() {
        let Some((name, date)) = line.split_once('\t') else {
            continue;
        };
        let commits = count(repo, &["rev-list", "--count", name, "--not", "--remotes"]).await;
        if commits > 0 {
            out.push(LocalBranch {
                name: name.to_string(),
                commits,
                last_commit: date.to_string(),
            });
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
async fn repo_report(
    repo: &str,
    bound: &[(String, build_workspaces::BuildInfo)],
    protected: &[PathBuf],
    lanes: &[PathBuf],
    opts: &Opts,
    cwds: Option<&[PathBuf]>,
    held: &Holds,
    now: SystemTime,
) -> Repo {
    let dir = Path::new(repo);
    let mut r = Repo {
        repo: repo.to_string(),
        default_branch: None,
        worktrees: Vec::new(),
        local_only_branches: Vec::new(),
        tilde_folders: Vec::new(),
        error: None,
    };
    let listing = match git(dir, &["worktree", "list", "--porcelain"], QUICK).await {
        Ok(text) => text,
        Err(e) => {
            r.error = Some(e);
            return r;
        }
    };
    let (main, linked) = parse_worktrees(&listing);
    if let Some(main) = main {
        r.repo = main;
    }
    let main_dir = PathBuf::from(&r.repo);
    r.default_branch = git(
        &main_dir,
        &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"],
        QUICK,
    )
    .await
    .ok()
    .and_then(|s| build_workspaces::remote_head_to_base(&s));
    let tip = match &r.default_branch {
        Some(b) => git(&main_dir, &["rev-parse", b], QUICK).await.ok(),
        None => None,
    };
    let real = |p: &str| Path::new(p).canonicalize().ok();
    let mut folders: Vec<(PathBuf, bool)> = vec![(main_dir.clone(), false)];
    // Hundreds of worktrees read one after another timed the action out
    // (PER-3): read SCAN_PARALLEL at once, results back in listed order.
    let shared = std::sync::Arc::new((
        r.default_branch.clone(),
        tip.clone(),
        cwds.map(|c| c.to_vec()),
        held.clone(),
        opts.sizes,
        opts.reclaim,
        now,
    ));
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(SCAN_PARALLEL));
    let mut tasks = tokio::task::JoinSet::new();
    let mut homes: Vec<Option<PathBuf>> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for (i, listed) in linked.into_iter().enumerate() {
        let here = real(&listed.path);
        let workspace = bound
            .iter()
            .find(|(_, b)| b.exists && here.is_some() && real(&b.worktree) == here)
            .map(|(id, _)| id.clone());
        homes.push(here);
        paths.push(listed.path.clone());
        let (shared, gate) = (shared.clone(), gate.clone());
        tasks.spawn(async move {
            let _slot = gate.acquire_owned().await.ok();
            #[cfg(test)]
            let _flight = tests::Flight::enter();
            let (default_branch, tip, cwds, held, sizes, reclaim, now) = &*shared;
            let w = worktree(
                listed,
                workspace,
                default_branch.as_deref(),
                tip.as_deref(),
                cwds.as_deref(),
                held,
                *sizes,
                *reclaim,
                *now,
            )
            .await;
            (i, w)
        });
    }
    let mut done: Vec<Option<Worktree>> = (0..homes.len()).map(|_| None).collect();
    while let Some(joined) = tasks.join_next().await {
        if let Ok((i, w)) = joined {
            done[i] = Some(w);
        }
    }
    for ((here, path), w) in homes.into_iter().zip(paths).zip(done) {
        // A task that panicked leaves a worktree nothing is known about:
        // listed, never removable.
        let mut w = w.unwrap_or_else(|| Worktree {
            path,
            branch: None,
            workspace: None,
            merged: false,
            clean: false,
            unpushed: 0,
            ignored: Vec::new(),
            ignored_count: 0,
            in_use: true,
            removable: false,
            reason: Some("its state couldn't be read".into()),
            reclaim: Vec::new(),
        });
        // A path some binding uses as its repository is never removed.
        if here.as_ref().is_some_and(|h| protected.contains(h)) {
            w.reclaim.clear();
            if w.removable {
                w.removable = false;
                w.reason = Some("a build workspace uses it as its repository".into());
            }
        }
        // A build workspace's extra worktree (F6 part B) goes only through
        // its workspace's Remove worktree, which keeps the binding right.
        if here.as_ref().is_some_and(|h| lanes.contains(h)) && w.removable {
            w.removable = false;
            w.reason =
                Some("a build workspace's extra worktree; remove it from its workspace".into());
        }
        // Unknown processes keep its "~" folders too.
        folders.push((PathBuf::from(&w.path), w.in_use || cwds.is_none()));
        r.worktrees.push(w);
    }
    if opts.branches {
        r.local_only_branches = local_only(&main_dir).await;
    }
    // The main checkout's own `~` folders, and those of worktrees nothing
    // runs in. A cargo writing into `~` runs from the checkout (or its
    // src-tauri), not from inside `~`, so anything working in the checkout
    // keeps them.
    let main_busy = cwds.is_none_or(|c| {
        let m = main_dir.canonicalize().unwrap_or_else(|_| main_dir.clone());
        c.iter().any(|p| p.starts_with(&m))
    });
    folders[0].1 = main_busy;
    for (folder, busy) in folders {
        if busy {
            continue;
        }
        for t in tilde_candidates(&folder) {
            r.tilde_folders.push(TildeFolder {
                bytes: if opts.sizes { du_bytes(&t).await } else { None },
                path: t.to_string_lossy().into_owned(),
            });
        }
    }
    r
}

fn hash_of(entries: &[String]) -> String {
    let mut hash = Sha256::new();
    for p in entries {
        hash.update(p.as_bytes());
        hash.update(b"\n");
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The report for `repos` (any folder of each repository; one entry per
/// repository, by its git common dir).
pub(crate) async fn report_for(vault: &Path, repos: &BTreeSet<String>, opts: &Opts) -> Report {
    report_at(vault, repos, opts, SystemTime::now()).await
}

/// Every process's working folder, from where `opts` says.
async fn read_cwds(opts: &Opts) -> Option<Vec<PathBuf>> {
    match &opts.cwds {
        Cwds::Read => process_cwds().await,
        #[cfg(test)]
        Cwds::Given(c) => Some(c.clone()),
        #[cfg(test)]
        Cwds::Unknown => None,
    }
}

/// [`report_for`] with the clock given (idle days are counted from `now`).
async fn report_at(vault: &Path, repos: &BTreeSet<String>, opts: &Opts, now: SystemTime) -> Report {
    let bound = build_workspaces::all_bindings(vault);
    // Lane worktrees count as bound (F6 part B), never offered for removal.
    let lanes: Vec<PathBuf> = build_workspaces::all_worktrees(vault)
        .into_iter()
        .filter(|(_, lane, info)| lane != build_workspaces::MAIN_LANE && info.exists)
        .filter_map(|(_, _, info)| Path::new(&info.worktree).canonicalize().ok())
        .collect();
    // Every binding's repository (not only the requested ones: `repo` may
    // narrow the set) and the requested folders are never removed.
    let protected: Vec<PathBuf> = repos
        .iter()
        .cloned()
        .chain(bound.iter().map(|(_, b)| b.repo.clone()))
        .filter_map(|r| Path::new(&r).canonicalize().ok())
        .collect();
    let cwds = read_cwds(opts).await;
    let held = opts.held.scan();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for repo in repos {
        let common = git(
            Path::new(repo),
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            QUICK,
        )
        .await
        .unwrap_or_else(|_| repo.clone());
        if !seen.insert(common) {
            continue;
        }
        out.push(
            repo_report(
                repo,
                &bound,
                &protected,
                &lanes,
                opts,
                cwds.as_deref(),
                &held,
                now,
            )
            .await,
        );
    }
    let mut remove = Vec::new();
    let mut hashed = Vec::new();
    for r in &out {
        for w in r.worktrees.iter().filter(|w| w.removable) {
            remove.push(w.path.clone());
            // How it goes, and what ignored files go with it: a new ignored
            // file since the card changes the list.
            let how = match &w.workspace {
                Some(id) => format!("workspace {id}"),
                None => "git".to_string(),
            };
            hashed.push(format!(
                "{how}: {} ignoring {} [{}]",
                w.path,
                w.ignored_count,
                w.ignored.join(", ")
            ));
        }
    }
    for t in out.iter().flat_map(|r| r.tilde_folders.iter()) {
        remove.push(t.path.clone());
        hashed.push(format!("tilde: {}", t.path));
    }
    let tilde_bytes = out
        .iter()
        .flat_map(|r| r.tilde_folders.iter().filter_map(|t| t.bytes))
        .sum();
    let mut reclaim_lines = Vec::new();
    let mut reclaim_bytes = 0u64;
    for w in out
        .iter()
        .flat_map(|r| r.worktrees.iter())
        .filter(|w| !w.reclaim.is_empty())
    {
        let names: Vec<&str> = w.reclaim.iter().map(|d| d.path.as_str()).collect();
        reclaim_lines.push(format!("reclaim: {}: {}", w.path, names.join(", ")));
        reclaim_bytes += w.reclaim.iter().filter_map(|d| d.bytes).sum::<u64>();
    }
    Report {
        hash: hash_of(&hashed),
        repos: out,
        remove,
        tilde_bytes,
        in_use_unknown: cwds.is_none(),
        reclaim_hash: hash_of(&reclaim_lines),
        reclaim_bytes,
    }
}

/// The repositories this vault's build workspaces came from, or only
/// `only` (a repository folder) when given.
pub(crate) fn known_repos(vault: &Path, only: Option<&str>) -> BTreeSet<String> {
    build_workspaces::all_bindings(vault)
        .into_iter()
        .map(|(_, b)| b.repo)
        .filter(|r| Path::new(r).is_dir())
        .filter(|r| {
            only.is_none_or(|o| {
                Path::new(o).canonicalize().ok() == Path::new(r).canonicalize().ok()
            })
        })
        .collect()
}

/// The full report for this vault's repositories (or one of them).
pub(crate) async fn report(vault: &Path, only: Option<&str>) -> Report {
    report_for(vault, &known_repos(vault, only), &Opts::full()).await
}

fn human(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

/// Card lines: everything that goes (with the ignored files each worktree
/// takes with it), then what stays and why. The agent's card; the person's
/// confirm dialog says the same in the panel's words (`src/diskGuard.ts`,
/// `cleanConfirmBody`): change both.
pub(crate) fn describe(r: &Report) -> Vec<String> {
    let mut lines = Vec::new();
    let worktrees: Vec<&Worktree> = r
        .repos
        .iter()
        .flat_map(|x| x.worktrees.iter().filter(|w| w.removable))
        .collect();
    let tildes: Vec<&TildeFolder> = r
        .repos
        .iter()
        .flat_map(|x| x.tilde_folders.iter())
        .collect();
    if r.in_use_unknown {
        lines.push(
            "Couldn't read which processes are running, so no worktree is removed.".to_string(),
        );
    }
    if worktrees.is_empty() && tildes.is_empty() {
        lines.push("Nothing to remove.".to_string());
    }
    if !worktrees.is_empty() {
        lines.push(format!(
            "Remove {} merged, clean, fully pushed worktree(s) nothing runs in (branches are kept; a bound build workspace falls back to its vault folder; files git ignores in them are deleted with them):",
            worktrees.len()
        ));
        for w in worktrees {
            let ws = w
                .workspace
                .as_deref()
                .map(|id| format!(" (workspace {id})"))
                .unwrap_or_default();
            let branch = w
                .branch
                .as_deref()
                .map(|b| format!(" [{b}]"))
                .unwrap_or_default();
            let ign = if w.ignored_count == 0 {
                String::new()
            } else {
                let more = w.ignored_count.saturating_sub(w.ignored.len() as u32);
                let more = if more > 0 {
                    format!(" +{more} more")
                } else {
                    String::new()
                };
                format!(" — also deletes ignored: {}{more}", w.ignored.join(", "))
            };
            lines.push(format!("- {}{branch}{ws}{ign}", w.path));
        }
    }
    if !tildes.is_empty() {
        lines.push(format!(
            "Delete {} literal \"~\" folder(s) of stray cargo output ({}):",
            tildes.len(),
            human(r.tilde_bytes)
        ));
        for t in tildes {
            lines.push(format!(
                "- {}{}",
                t.path,
                t.bytes
                    .map(|b| format!(" ({})", human(b)))
                    .unwrap_or_default()
            ));
        }
    }
    let kept = r
        .repos
        .iter()
        .flat_map(|x| x.worktrees.iter().filter(|w| !w.removable))
        .count();
    let branches: usize = r.repos.iter().map(|x| x.local_only_branches.len()).sum();
    if kept > 0 {
        lines.push(format!("Kept: {kept} worktree(s) (in use, changed, unpushed, unmerged, locked, just started, built the installed or rollback app, or needed by a build waiting to install)."));
    }
    if !r.remove.is_empty() {
        lines.push("\"Nothing runs in it\" means no process's working folder is inside it; a tool working there from elsewhere (an editor, `cargo --manifest-path`) isn't seen.".to_string());
    }
    if branches > 0 {
        lines.push(format!(
            "Not touched: {branches} local-only branch(es) (commits no remote has)."
        ));
    }
    lines
}

/// Remove what [`report_for`] marks now (no sizes or branch scan: the hash
/// covers paths and how each goes); a list that changed since refuses and
/// removes nothing. Right before each worktree goes, the held commits and
/// folders are read again ([`held_now`], overnight R2): one a Build or
/// Install has needed since the report stays, in `failed` with why, and the
/// rest go. `opts` says where the running processes and the held commits
/// and folders come from; its `sizes` and `branches` are ignored.
pub(crate) async fn clean_for(
    vault: &Path,
    repos: &BTreeSet<String>,
    expected_hash: &str,
    caller: Option<&str>,
    opts: Opts,
) -> Result<Cleaned, String> {
    let opts = Opts {
        sizes: false,
        branches: false,
        reclaim: false,
        ..opts
    };
    let started = std::time::Instant::now();
    let r = report_for(vault, repos, &opts).await;
    if r.hash != expected_hash {
        return Err("The worktrees and folders to remove changed since the card was shown. Nothing was removed; ask again to see the new list.".into());
    }
    let budget = opts.budget.unwrap_or(CLEAN_BUDGET);
    let volumes_at = volumes_of(&r);
    let before: Vec<Option<u64>> = volumes_at
        .iter()
        .map(|p| crate::disk_guard::free_bytes(p))
        .collect();
    let mut removed = Vec::new();
    let mut failed = Vec::new();
    let mut remaining = Vec::new();
    for repo in &r.repos {
        // Where a worktree's clean build is kept before it goes (S12).
        let records = git(
            Path::new(&repo.repo),
            &["rev-parse", "--git-common-dir"],
            Duration::from_secs(10),
        )
        .await
        .ok()
        .map(|common| {
            Path::new(&repo.repo)
                .join(common.trim())
                .join(crate::install_ready::RECORDS_DIR)
        });
        for w in repo.worktrees.iter().filter(|w| w.removable) {
            // Out of time: say what is left rather than time out (PER-3).
            if started.elapsed() >= budget {
                remaining.push(w.path.clone());
                continue;
            }
            // Held since the report (overnight R2): a Build or Install
            // pressed a moment ago needs it. This one stays; the rest go.
            let held = opts.held.removal();
            if let Some(why) = held_now(Path::new(&w.path), &held).await {
                failed.push((w.path.clone(), why));
                continue;
            }
            if let Some(dir) = &records {
                if let Err(e) = crate::install_ready::save_build_record(Path::new(&w.path), dir) {
                    eprintln!(
                        "[worktree-hygiene] couldn't keep {}'s build record: {e}",
                        w.path
                    );
                }
            }
            let done = match &w.workspace {
                // And again after its services stopped, right before `git
                // worktree remove` (overnight R4: that can take a while).
                // Only the service stop and the checks are time-boxed: once
                // `kept` has said go, the delete is git's to finish (a cut
                // one leaves a half-deleted worktree).
                Some(id) => {
                    // One state decides who wins: `kept` (the delete is about
                    // to start) or the timer (cut it before it does).
                    let state = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(IDLE));
                    let task = {
                        let (vault, id, caller) =
                            (vault.to_path_buf(), id.clone(), caller.map(str::to_string));
                        let (dir, from, state) =
                            (PathBuf::from(&w.path), opts.held.clone(), state.clone());
                        tokio::spawn(async move {
                            let kept = move || async move {
                                if let Some(why) = held_now(&dir, &from.in_removal()).await {
                                    return Some(why);
                                }
                                match state.compare_exchange(IDLE, GOING, SeqCst, SeqCst) {
                                    Ok(_) => None,
                                    Err(_) => Some(CUT_MESSAGE.to_string()),
                                }
                            };
                            build_workspaces::remove_for_unless(
                                &vault,
                                &id,
                                false,
                                caller.as_deref(),
                                kept,
                            )
                            .await
                        })
                    };
                    // Dropped with the clean (the action cancelled): a delete
                    // not yet started is stopped, one under way finishes and
                    // says so in the log.
                    struct Guard(
                        tokio::task::AbortHandle,
                        std::sync::Arc<std::sync::atomic::AtomicU8>,
                        String,
                        bool,
                    );
                    impl Drop for Guard {
                        fn drop(&mut self) {
                            if self.3 {
                                return;
                            }
                            if self.1.compare_exchange(IDLE, CUT, SeqCst, SeqCst).is_ok() {
                                self.0.abort();
                            } else {
                                eprintln!(
                                    "[worktree-hygiene] the clean was dropped while {} was being removed; the removal continues",
                                    self.2
                                );
                            }
                        }
                    }
                    let mut guard =
                        Guard(task.abort_handle(), state.clone(), w.path.clone(), false);
                    tokio::pin!(task);
                    let timer = tokio::time::sleep(ONE_REMOVAL);
                    tokio::pin!(timer);
                    let mut armed = true;
                    let joined = loop {
                        tokio::select! {
                            r = &mut task => break r,
                            _ = &mut timer, if armed => {
                                armed = false;
                                if state.compare_exchange(IDLE, CUT, SeqCst, SeqCst).is_ok() {
                                    guard.0.abort();
                                }
                                // Lost: the delete started; git finishes it.
                            }
                        }
                    };
                    guard.3 = true;
                    match joined {
                        Ok(Err(e)) if state.load(SeqCst) == GOING && timed_out(&e) => {
                            Err(format!("{e} ({PARTLY})"))
                        }
                        Ok(r) => r,
                        Err(_) if state.load(SeqCst) == GOING => {
                            Err(format!("its removal stopped part-way: {PARTLY}"))
                        }
                        Err(_) => Err(CUT_MESSAGE.to_string()),
                    }
                }
                None => {
                    git(
                        Path::new(&repo.repo),
                        &["worktree", "remove", &w.path],
                        GIT_REMOVE,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| {
                        // Only git's own timeout can leave it half-deleted;
                        // a refusal never suggests --force.
                        if timed_out(&e) {
                            format!("{e} ({PARTLY})")
                        } else {
                            e
                        }
                    })
                }
            };
            match done {
                Ok(()) => removed.push(w.path.clone()),
                Err(e) => failed.push((w.path.clone(), e)),
            }
        }
    }
    for repo in &r.repos {
        for t in &repo.tilde_folders {
            let p = Path::new(&t.path);
            // Gone already: it went with its worktree (an ignored folder, as
            // `*~` in a global ignore makes it, is removed with the tree).
            if std::fs::symlink_metadata(p).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                removed.push(t.path.clone());
                continue;
            }
            if started.elapsed() >= budget {
                remaining.push(t.path.clone());
                continue;
            }
            if !is_tilde_folder(p) {
                failed.push((t.path.clone(), "no longer a \"~\" folder".into()));
                continue;
            }
            // Gigabytes of cargo output can take minutes: off the async
            // workers and under a limit. A blocking delete can't be stopped,
            // so a late one is reported as running on.
            let target = p.to_path_buf();
            if !DELETING.lock().unwrap().insert(target.clone()) {
                failed.push((
                    t.path.clone(),
                    "still being deleted by an earlier clean; run it again later".into(),
                ));
                continue;
            }
            let del = tokio::task::spawn_blocking(move || {
                let r = std::fs::remove_dir_all(&target);
                DELETING.lock().unwrap().remove(&target);
                r
            });
            match tokio::time::timeout(ONE_REMOVAL, del).await {
                Ok(Ok(Ok(()))) => removed.push(t.path.clone()),
                Ok(Ok(Err(e))) => failed.push((t.path.clone(), e.to_string())),
                Ok(Err(e)) => failed.push((t.path.clone(), e.to_string())),
                Err(_) => failed.push((
                    t.path.clone(),
                    format!(
                        "deleting it took longer than {} s and is still running in the background; it may be partly deleted, so run the clean again",
                        ONE_REMOVAL.as_secs()
                    ),
                )),
            }
        }
    }
    let mut volumes = Vec::new();
    for (path, before) in volumes_at.iter().zip(before) {
        let after = crate::disk_guard::free_bytes(path);
        if let (Some(b), Some(a)) = (before, after) {
            volumes.push(VolumeFree {
                path: path.to_string_lossy().into_owned(),
                free_before: b,
                free_after: a,
                freed: a.saturating_sub(b),
            });
        }
    }
    Ok(Cleaned {
        removed,
        failed,
        complete: remaining.is_empty(),
        remaining,
        free_before: volumes.iter().map(|v| v.free_before).sum(),
        free_after: volumes.iter().map(|v| v.free_after).sum(),
        freed: volumes.iter().map(|v| v.freed).sum(),
        volumes,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// One folder per distinct volume the report's repositories sit on (their
/// worktrees are read where they are: a worktree on another volume is
/// measured through its own folder).
fn volumes_of(r: &Report) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut probes = BTreeSet::new();
    let mut out = Vec::new();
    for repo in &r.repos {
        let folders = std::iter::once(repo.repo.as_str())
            .chain(repo.worktrees.iter().map(|w| w.path.as_str()));
        for f in folders {
            let parent = Path::new(f).parent().unwrap_or(Path::new(f));
            let probe = if parent.exists() {
                parent
            } else {
                Path::new(f)
            };
            // Volumes of one APFS container share their free space and report
            // the same blocks, so key on that, not on the device alone.
            // One read per distinct folder, before any comparing, and a key
            // of stable data only (total blocks; never the live free count).
            if !probes.insert(probe.to_path_buf()) {
                continue;
            }
            if let Some(key) = crate::disk_guard::volume_key(probe) {
                if seen.insert(key) {
                    out.push(probe.to_path_buf());
                }
            }
        }
    }
    out
}

/// [`clean_for`] for this vault's repositories (or one of them).
pub(crate) async fn clean(
    vault: &Path,
    only: Option<&str>,
    expected_hash: &str,
    caller: Option<&str>,
) -> Result<Cleaned, String> {
    clean_for(
        vault,
        &known_repos(vault, only),
        expected_hash,
        caller,
        Opts::full(),
    )
    .await
}

/// What [`reclaim_output`] did.
#[cfg_attr(not(test), allow(dead_code))] // Wired to a confirm action in a later step.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reclaimed {
    /// Folders removed (absolute paths).
    pub removed: Vec<String>,
    /// Folders kept, each with why (rechecked at apply time).
    pub failed: Vec<(String, String)>,
}

/// Remove the generated folders `report_at` lists now, bound to
/// `expected_hash` (the report's `reclaim_hash`): a list that changed since
/// refuses and removes nothing. Right before each worktree's folders go the
/// held commits and folders, the running processes and the idle conditions
/// are read again; a worktree that stopped qualifying keeps its folders, in
/// `failed` with why. Only the listed folders are deleted, one
/// `remove_dir_all` each: never a worktree, a branch, a tracked file or
/// anything else in a worktree. `opts` says where the processes and held
/// folders come from; its `sizes` and `branches` are ignored.
#[cfg_attr(not(test), allow(dead_code))] // Wired to a confirm action in a later step.
pub(crate) async fn reclaim_output(
    vault: &Path,
    repos: &BTreeSet<String>,
    expected_hash: &str,
    opts: Opts,
    now: SystemTime,
) -> Result<Reclaimed, String> {
    let opts = Opts {
        sizes: false,
        branches: false,
        reclaim: true,
        ..opts
    };
    let report = report_at(vault, repos, &opts, now).await;
    if report.in_use_unknown {
        return Err("Couldn't read which processes are running. Nothing was removed.".into());
    }
    if report.reclaim_hash != expected_hash {
        return Err("The list changed since the report was made. Nothing was removed.".into());
    }
    let held = opts.held.removal();
    let cwds = read_cwds(&opts).await;
    let mut done = Reclaimed::default();
    for w in report
        .repos
        .iter()
        .flat_map(|r| r.worktrees.iter())
        .filter(|w| !w.reclaim.is_empty())
    {
        let dir = Path::new(&w.path);
        let real = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let stop = if let Some(why) = held_now(dir, &held).await {
            Some(why)
        } else if cwds
            .as_ref()
            .is_none_or(|c| c.iter().any(|p| p.starts_with(&real)))
        {
            Some("a process is working in it".to_string())
        } else {
            match reclaimable(dir, now).await {
                Ok(again) => {
                    let still: Vec<String> = again.into_iter().map(|(rel, _)| rel).collect();
                    if w.reclaim.iter().all(|d| still.contains(&d.path)) {
                        None
                    } else {
                        Some("no longer idle".to_string())
                    }
                }
                Err(why) => Some(why),
            }
        };
        for d in &w.reclaim {
            let path = real.join(&d.path);
            let shown = dir.join(&d.path).to_string_lossy().into_owned();
            if let Some(why) = &stop {
                done.failed.push((shown, why.clone()));
                continue;
            }
            // A build or install may have claimed it while an earlier folder
            // was going: read the holds again before each one.
            if let Some(why) = held_now(dir, &held).await {
                done.failed.push((shown, why));
                continue;
            }
            // Right before the delete: a real folder (not a link) that still
            // resolves inside this worktree.
            let inside = std::fs::symlink_metadata(&path)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                && path
                    .canonicalize()
                    .is_ok_and(|c| c.starts_with(&real) && c != real);
            if !inside {
                done.failed
                    .push((shown, "no longer a plain folder inside the worktree".into()));
                continue;
            }
            let target = path.clone();
            match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(target)).await {
                Ok(Ok(())) => done.removed.push(shown),
                Ok(Err(e)) => done.failed.push((shown, e.to_string())),
                Err(e) => done.failed.push((shown, e.to_string())),
            }
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_workspaces::tests::sh;
    use std::fs;

    thread_local! {
        static NOW: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        static PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Counts worktree reads in flight on this thread (a `#[tokio::test]`
    /// runtime is one thread, so a test sees only its own).
    pub(super) struct Flight;
    impl Flight {
        pub(super) fn enter() -> Flight {
            NOW.with(|n| n.set(n.get() + 1));
            PEAK.with(|p| p.set(p.get().max(NOW.with(|n| n.get()))));
            Flight
        }
    }
    impl Drop for Flight {
        fn drop(&mut self) {
            NOW.with(|n| n.set(n.get().saturating_sub(1)));
        }
    }

    /// A repository with an `origin` (bare, its HEAD set), and worktrees:
    /// `merged` (its own commit, merged into origin/main, which then moved
    /// on), `ahead` (a commit no remote has), `dirty` (an untracked file),
    /// `fresh` (exactly at origin/main's tip). `merged` holds ignored
    /// rebuildable output (`node_modules/`) and a `~` folder in `src-tauri`.
    struct Bed {
        _tmp: tempfile::TempDir,
        repo: PathBuf,
        vault: PathBuf,
        merged: PathBuf,
        ahead: PathBuf,
        dirty: PathBuf,
        fresh: PathBuf,
    }

    fn bed() -> Bed {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let origin = root.join("origin.git");
        let repo = root.join("app");
        fs::create_dir_all(&repo).unwrap();
        sh(&root, &["init", "-q", "--bare", origin.to_str().unwrap()]);
        sh(&repo, &["init", "-q"]);
        fs::write(repo.join("README.md"), "hi\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "first"]);
        sh(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh(&repo, &["push", "-q", "origin", "main"]);
        sh(&repo, &["remote", "set-head", "origin", "main"]);
        // Ignored, as a global `*~` makes `~` on this Mac (the test never
        // reads a global ignore), and a secrets file.
        fs::write(
            repo.join(".git/info/exclude"),
            "~\n.env\n*.log\nnode_modules\n",
        )
        .unwrap();
        let wt = |name: &str| root.join("Worktrees").join(name);
        for name in ["merged", "ahead", "dirty"] {
            sh(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &format!("b/{name}"),
                    wt(name).to_str().unwrap(),
                    "main",
                ],
            );
        }
        // merged: its own commit lands on origin/main, then main moves on.
        fs::write(wt("merged").join("feature.txt"), "f").unwrap();
        sh(&wt("merged"), &["add", "."]);
        sh(&wt("merged"), &["commit", "-q", "-m", "feature"]);
        sh(&wt("merged"), &["push", "-q", "origin", "HEAD:main"]);
        sh(&repo, &["fetch", "-q", "origin"]);
        sh(&repo, &["merge", "-q", "--ff-only", "origin/main"]);
        fs::write(repo.join("later.txt"), "l").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "later"]);
        sh(&repo, &["push", "-q", "origin", "main"]);
        sh(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "b/fresh",
                wt("fresh").to_str().unwrap(),
                "origin/main",
            ],
        );
        fs::write(wt("ahead").join("new.txt"), "x").unwrap();
        sh(&wt("ahead"), &["add", "."]);
        sh(&wt("ahead"), &["commit", "-q", "-m", "local only"]);
        fs::write(wt("dirty").join("scratch.txt"), "x").unwrap();
        fs::create_dir_all(wt("merged").join("node_modules/pkg")).unwrap();
        fs::write(wt("merged").join("node_modules/pkg/index.js"), "x").unwrap();
        fs::create_dir_all(wt("merged").join("src-tauri/~/Library/Caches/t")).unwrap();
        fs::write(
            wt("merged").join("src-tauri/~/Library/Caches/t/big"),
            vec![0u8; 64 * 1024],
        )
        .unwrap();
        let vault = root.join("vault");
        fs::create_dir_all(&vault).unwrap();
        Bed {
            merged: wt("merged"),
            ahead: wt("ahead"),
            dirty: wt("dirty"),
            fresh: wt("fresh"),
            _tmp: tmp,
            repo,
            vault,
        }
    }

    fn repos(b: &Bed) -> BTreeSet<String> {
        [b.repo.to_string_lossy().into_owned()]
            .into_iter()
            .collect()
    }

    /// Everything, with nothing running anywhere and no app built from any
    /// of these worktrees (never this Mac's /Applications).
    fn idle() -> Opts {
        Opts {
            sizes: true,
            branches: true,
            cwds: Cwds::Given(vec![]),
            held: Held::Given(Holds::default()),
            budget: None,
            reclaim: true,
        }
    }

    /// These commits held, each with why.
    fn held(list: &[(&str, &str)]) -> Held {
        Held::Given(Holds {
            commits: list
                .iter()
                .map(|(c, w)| (c.to_string(), w.to_string()))
                .collect(),
            paths: vec![],
        })
    }

    /// [`held_for`] with commits only, at a folder nothing holds.
    fn held_by(head: Option<&str>, held: &[(String, String)]) -> Option<String> {
        let holds = Holds {
            commits: held.to_vec(),
            paths: vec![],
        };
        held_for(head, Path::new("/nowhere"), &holds)
    }

    fn find<'a>(r: &'a Report, p: &Path) -> &'a Worktree {
        r.repos[0]
            .worktrees
            .iter()
            .find(|w| Path::new(&w.path) == p)
            .unwrap()
    }

    /// Prints the report for `HOLLOW_HYGIENE_REPO`; reads only (git reads,
    /// lsof and du). `HOLLOW_HYGIENE_REPO=<repo> cargo test --lib live_report -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "reads a real repository; run by hand"]
    async fn live_report_reads_only() {
        let repo = std::env::var("HOLLOW_HYGIENE_REPO").expect("HOLLOW_HYGIENE_REPO");
        let vault = tempfile::tempdir().unwrap();
        let r = report_for(vault.path(), &[repo].into_iter().collect(), &Opts::full()).await;
        for x in &r.repos {
            println!(
                "{}: default {:?}, {} worktrees ({} removable, {} in use), {} local-only branches, {} ~ folders, error {:?}",
                x.repo,
                x.default_branch,
                x.worktrees.len(),
                x.worktrees.iter().filter(|w| w.removable).count(),
                x.worktrees.iter().filter(|w| w.in_use).count(),
                x.local_only_branches.len(),
                x.tilde_folders.len(),
                x.error
            );
        }
        println!("{}", describe(&r).join("\n"));
    }

    #[test]
    fn lsof_escapes_are_undone() {
        assert_eq!(
            lsof_unescape(r"/Users/dev/Documents - Alex\xe2\x80\x99s MacBook Pro/x"),
            "/Users/dev/Documents - Alex’s MacBook Pro/x"
        );
        assert_eq!(lsof_unescape(r"/a\nb\tc\\d"), "/a\nb\tc\\d");
        assert_eq!(
            lsof_unescape(r"/plain/\xZZ/end\"),
            r"/plain/\xZZ/end\",
            "not an escape: kept"
        );
    }

    #[test]
    fn porcelain_gives_the_main_worktree_and_linked_ones_with_branch_and_lock() {
        let text = "worktree /r\nHEAD a\nbranch refs/heads/main\n\nworktree /w/one\nHEAD b\nbranch refs/heads/feat/x\n\nworktree /w/two\nHEAD c\ndetached\nlocked moving it\n\nworktree /w/bare\nbare\n";
        let (main, linked) = parse_worktrees(text);
        assert_eq!(main.as_deref(), Some("/r"));
        assert_eq!(
            linked,
            [
                Listed {
                    path: "/w/one".into(),
                    branch: Some("feat/x".into()),
                    locked: false
                },
                Listed {
                    path: "/w/two".into(),
                    branch: None,
                    locked: true
                },
            ]
        );
    }

    #[tokio::test]
    async fn the_report_marks_only_merged_clean_pushed_idle_worktrees_and_lists_the_rest() {
        let b = bed();
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        let repo = &r.repos[0];
        assert_eq!(repo.error, None);
        assert_eq!(repo.default_branch.as_deref(), Some("origin/main"));
        let (m, a, d, f) = (
            find(&r, &b.merged),
            find(&r, &b.ahead),
            find(&r, &b.dirty),
            find(&r, &b.fresh),
        );
        assert!(
            m.removable && m.merged && m.clean && m.unpushed == 0,
            "{m:?}"
        );
        assert_eq!(m.branch.as_deref(), Some("b/merged"));
        assert_eq!(
            m.ignored,
            ["node_modules/", "src-tauri/"],
            "what goes with it (git names the folder: all it holds is ignored)"
        );
        assert_eq!(a.reason.as_deref(), Some("1 commit(s) on no remote"));
        assert_eq!(
            d.reason.as_deref(),
            Some("uncommitted or untracked changes")
        );
        assert_eq!(
            f.reason.as_deref(),
            Some("exactly at the default branch's tip (it may have only just started)")
        );
        assert_eq!(
            repo.local_only_branches
                .iter()
                .map(|x| x.name.as_str())
                .collect::<Vec<_>>(),
            ["b/ahead"]
        );
        let tilde = b.merged.join("src-tauri/~");
        assert_eq!(repo.tilde_folders.len(), 1);
        assert!(
            repo.tilde_folders[0].bytes.is_some_and(|n| n >= 64 * 1024),
            "{:?}",
            repo.tilde_folders
        );
        assert_eq!(
            r.remove,
            [
                b.merged.to_string_lossy().into_owned(),
                tilde.to_string_lossy().into_owned()
            ]
        );
        let text = describe(&r).join("\n");
        assert!(
            text.contains("Remove 1 merged, clean, fully pushed worktree(s) nothing runs in"),
            "{text}"
        );
        assert!(
            text.contains("[b/merged] — also deletes ignored: node_modules/, src-tauri/"),
            "{text}"
        );
        assert!(text.contains("Delete 1 literal \"~\" folder(s)"), "{text}");
        assert!(text.contains("Kept: 3 worktree(s)"), "{text}");
        assert!(
            text.contains("Not touched: 1 local-only branch(es)"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn a_worktree_something_runs_in_stays_with_its_tilde_folder_and_unknown_processes_keep_all(
    ) {
        let b = bed();
        let busy = Opts {
            cwds: Cwds::Given(vec![b.merged.join("src-tauri")]),
            ..idle()
        };
        let r = report_for(&b.vault, &repos(&b), &busy).await;
        let m = find(&r, &b.merged);
        assert!(m.in_use && !m.removable, "{m:?}");
        assert_eq!(m.reason.as_deref(), Some("a process is working in it"));
        assert!(
            r.remove.is_empty(),
            "its ~ folder stays too: {:?}",
            r.remove
        );
        // Something working in the main checkout keeps the checkout's own ~.
        fs::create_dir_all(b.repo.join("src-tauri/~")).unwrap();
        let main_busy = Opts {
            cwds: Cwds::Given(vec![b.repo.join("src-tauri")]),
            ..idle()
        };
        let r = report_for(&b.vault, &repos(&b), &main_busy).await;
        assert!(
            !r.remove
                .iter()
                .any(|p| Path::new(p) == b.repo.join("src-tauri/~")),
            "{:?}",
            r.remove
        );
        // lsof failed: no worktree and no "~" folder anywhere.
        let r = report_for(
            &b.vault,
            &repos(&b),
            &Opts {
                cwds: Cwds::Unknown,
                ..idle()
            },
        )
        .await;
        assert!(r.in_use_unknown && r.remove.is_empty(), "{:?}", r.remove);
        assert_eq!(
            find(&r, &b.merged).reason.as_deref(),
            Some("couldn't read what is running, so nothing is removed")
        );
        assert!(describe(&r)[0].starts_with("Couldn't read which processes"));
    }

    /// A secrets file or local data git ignores keeps the worktree, as it
    /// keeps the gardener's `worktree:` line (overnight S5).
    #[tokio::test]
    async fn an_ignored_file_that_isnt_rebuildable_output_keeps_the_worktree() {
        let b = bed();
        fs::write(b.merged.join(".env"), "SECRET=1").unwrap();
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        let m = find(&r, &b.merged);
        assert!(!m.removable, "{m:?}");
        assert_eq!(
            m.reason.as_deref(),
            Some("holds ignored files that aren't rebuildable output: .env")
        );
        assert!(!r.remove.contains(&b.merged.to_string_lossy().into_owned()));
    }

    #[tokio::test]
    async fn a_new_ignored_file_since_the_card_changes_the_list() {
        let b = bed();
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        fs::write(b.merged.join("debug.log"), "x").unwrap();
        let err = clean_for(&b.vault, &repos(&b), &r.hash, None, idle())
            .await
            .unwrap_err();
        assert!(err.contains("Nothing was removed"), "{err}");
        assert!(b.merged.exists());
    }

    #[tokio::test]
    async fn a_locked_worktree_and_a_bindings_own_repository_are_never_removed() {
        let b = bed();
        sh(&b.repo, &["worktree", "lock", b.merged.to_str().unwrap()]);
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        assert_eq!(
            find(&r, &b.merged).reason.as_deref(),
            Some("locked (`git worktree unlock` first)")
        );
        sh(&b.repo, &["worktree", "unlock", b.merged.to_str().unwrap()]);
        // A binding whose "repository" is the merged worktree itself: one
        // entry for the repository (by its common dir), main as the repo,
        // and that worktree protected.
        let both: BTreeSet<String> = [
            b.repo.to_string_lossy().into_owned(),
            b.merged.to_string_lossy().into_owned(),
        ]
        .into_iter()
        .collect();
        let r = report_for(&b.vault, &both, &idle()).await;
        assert_eq!(
            r.repos.len(),
            1,
            "one repository, however many folders name it"
        );
        assert_eq!(Path::new(&r.repos[0].repo), b.repo);
        assert_eq!(
            find(&r, &b.merged).reason.as_deref(),
            Some("a build workspace uses it as its repository")
        );
        // Narrowed to the main repository: a binding elsewhere naming the
        // merged worktree as its repository still protects it.
        crate::build_workspaces::bind_repo_for_test(&b.vault, "ws-odd", &b.merged, &b.dirty);
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        assert_eq!(
            find(&r, &b.merged).reason.as_deref(),
            Some("a build workspace uses it as its repository")
        );
    }

    #[test]
    fn an_apps_commit_holds_its_worktree_abbreviated_or_not() {
        let held: Vec<(String, String)> = vec![
            ("abcdef1234567890".into(), "installed".into()),
            ("abcdef1".into(), "rollback".into()),
        ];
        assert_eq!(
            held_by(Some("abcdef1234567890"), &held).as_deref(),
            Some("installed; rollback"),
            "one side abbreviated; both apps from one commit"
        );
        assert_eq!(held_by(Some("1234567890abcdef"), &held), None);
        assert_eq!(held_by(Some("abcdef"), &held), None, "too short to tell");
        assert_eq!(held_by(None, &held), None);
        assert_eq!(held_by(Some("abcdef1234567890"), &[]), None);
        // An app there whose provenance can't be read holds every worktree.
        let unknown = [(
            crate::install_targets::ANY_COMMIT.to_string(),
            "couldn't read which commit built the installed app".to_string(),
        )];
        assert_eq!(
            held_by(Some("1234567890abcdef"), &unknown).as_deref(),
            Some("couldn't read which commit built the installed app")
        );
    }

    /// The release checkout that built the installed app is merged, clean
    /// and pushed, and was listed for removal (overnight H20). Now the
    /// worktree at the installed or rollback app's commit stays, out of the
    /// list and the hash the confirm binds; the others still go.
    #[tokio::test]
    async fn the_worktree_that_built_the_installed_or_rollback_app_is_kept() {
        let b = bed();
        let unheld = report_for(&b.vault, &repos(&b), &idle()).await;
        let merged_head = git(&b.merged, &["rev-parse", "HEAD"], QUICK).await.unwrap();
        let fresh_head = git(&b.fresh, &["rev-parse", "HEAD"], QUICK).await.unwrap();
        // The installed app from `merged` (its provenance abbreviated), the
        // rollback copy from `fresh`.
        let installed = Opts {
            held: held(&[
                (&merged_head[..12], "built the installed app"),
                (fresh_head.as_str(), "built the rollback copy"),
            ]),
            ..idle()
        };
        let r = report_for(&b.vault, &repos(&b), &installed).await;
        let m = find(&r, &b.merged);
        assert!(
            m.merged && m.clean && m.unpushed == 0 && !m.removable,
            "{m:?}"
        );
        assert_eq!(m.reason.as_deref(), Some("built the installed app"));
        assert_eq!(
            find(&r, &b.fresh).reason.as_deref(),
            Some("built the rollback copy")
        );
        let tilde = b.merged.join("src-tauri/~").to_string_lossy().into_owned();
        assert_eq!(
            r.remove,
            std::slice::from_ref(&tilde),
            "only its stray ~ folder"
        );
        assert_eq!(
            r.hash,
            hash_of(&[format!("tilde: {tilde}")]),
            "the kept worktree isn't part of what the confirm binds"
        );
        let text = describe(&r).join("\n");
        assert!(!text.contains("[b/merged]"), "{text}");

        // Another commit installed: nothing changes, the merged one goes.
        let other = Opts {
            held: held(&[("0123456789abcdef", "built the installed app")]),
            ..idle()
        };
        let r_other = report_for(&b.vault, &repos(&b), &other).await;
        assert!(find(&r_other, &b.merged).removable);
        assert_eq!(r_other.remove, unheld.remove);
        assert_eq!(r_other.hash, unheld.hash);

        // A list shown before that app was installed no longer matches.
        let err = clean_for(&b.vault, &repos(&b), &unheld.hash, None, installed.clone())
            .await
            .unwrap_err();
        assert!(err.contains("Nothing was removed"), "{err}");
        assert!(b.merged.exists() && b.merged.join("src-tauri/~").exists());
        // Bound to the held list: the "~" folder goes, the worktrees stay.
        let done = clean_for(&b.vault, &repos(&b), &r.hash, None, installed)
            .await
            .unwrap();
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.removed, [tilde]);
        assert!(b.merged.exists() && b.fresh.exists());
    }

    /// A finished build waiting to install needs its worktree (the install
    /// step runs `npm run tauri:install` there), and the checkout an app was
    /// built in stays after it moves past that app's commit (overnight S01a:
    /// the release checkout, merged, clean, pushed and at neither app's
    /// commit, was listed, and Install and restart then said "The worktree
    /// is gone"). Matched by folder, through a link too (`/tmp` is one).
    #[tokio::test]
    async fn a_pending_builds_worktree_and_an_apps_checkout_are_kept_by_folder() {
        let b = bed();
        let merged = b.merged.to_string_lossy().into_owned();
        let tilde = b.merged.join("src-tauri/~").to_string_lossy().into_owned();
        let unheld = report_for(&b.vault, &repos(&b), &idle()).await;
        assert!(find(&unheld, &b.merged).removable);
        assert!(unheld.remove.contains(&merged), "{:?}", unheld.remove);
        let link = b.repo.with_file_name("link-to-merged");
        std::os::unix::fs::symlink(&b.merged, &link).unwrap();
        let holding = |at: &PathBuf, why: &str| Opts {
            held: Held::Given(Holds {
                // An app built from some other commit.
                commits: vec![(
                    "0123456789abcdef".to_string(),
                    "built the rollback copy".to_string(),
                )],
                paths: vec![(at.clone(), why.to_string())],
            }),
            ..idle()
        };
        for why in [
            "a finished build waiting to install",
            "built the installed app",
        ] {
            for at in [&b.merged, &link] {
                let r = report_for(&b.vault, &repos(&b), &holding(at, why)).await;
                let m = find(&r, &b.merged);
                assert!(
                    m.merged && m.clean && m.unpushed == 0 && !m.removable,
                    "{at:?}: {m:?}"
                );
                assert_eq!(m.reason.as_deref(), Some(why), "{at:?}");
                assert_eq!(
                    r.remove,
                    std::slice::from_ref(&tilde),
                    "only its stray ~ folder"
                );
                assert_eq!(
                    r.hash,
                    hash_of(&[format!("tilde: {tilde}")]),
                    "the kept worktree isn't part of what the confirm binds"
                );
                assert!(!describe(&r).join("\n").contains("[b/merged]"));
            }
        }

        // An app's commit and folders naming one worktree: each reason once.
        let head = git(&b.merged, &["rev-parse", "HEAD"], QUICK).await.unwrap();
        let both = Opts {
            held: Held::Given(Holds {
                commits: vec![(head, "built the installed app".to_string())],
                paths: vec![
                    (link.clone(), "built the installed app".to_string()),
                    (
                        b.merged.clone(),
                        "a finished build waiting to install".to_string(),
                    ),
                ],
            }),
            ..idle()
        };
        let r = report_for(&b.vault, &repos(&b), &both).await;
        assert_eq!(
            find(&r, &b.merged).reason.as_deref(),
            Some("built the installed app; a finished build waiting to install")
        );

        // A list shown before the build finished no longer matches; bound to
        // the held list, the "~" folder goes and the worktree stays.
        let pending = holding(&b.merged, "a finished build waiting to install");
        let err = clean_for(&b.vault, &repos(&b), &unheld.hash, None, pending.clone())
            .await
            .unwrap_err();
        assert!(err.contains("Nothing was removed"), "{err}");
        let r = report_for(&b.vault, &repos(&b), &pending).await;
        let done = clean_for(&b.vault, &repos(&b), &r.hash, None, pending)
            .await
            .unwrap();
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.removed, [tilde]);
        assert!(b.merged.exists());
    }

    /// Overnight R2: a Build or Install pressed after the list was made
    /// holds its worktree by the time the removal comes. Holds are read again
    /// right before each removal, so that one stays (through its workspace
    /// or not) with why, and the rest of the list still goes.
    #[tokio::test]
    async fn a_worktree_held_after_the_list_was_made_stays_and_the_rest_go() {
        let why = "a build running now";
        for bound in [false, true] {
            let b = bed();
            // A second removable worktree: merged, clean, pushed, not at the tip.
            let older = b.merged.with_file_name("older");
            sh(
                &b.repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "b/older",
                    older.to_str().unwrap(),
                    "origin/main~1",
                ],
            );
            if bound {
                crate::build_workspaces::bind_repo_for_test(
                    &b.vault,
                    "ws-merged",
                    &b.repo,
                    &b.merged,
                );
            }
            let opts = Opts {
                held: Held::Later {
                    scan: Holds::default(),
                    removal: Holds {
                        commits: vec![],
                        paths: vec![(b.merged.clone(), why.to_string())],
                    },
                },
                ..idle()
            };
            let merged = b.merged.to_string_lossy().into_owned();
            let older_path = older.to_string_lossy().into_owned();
            let r = report_for(&b.vault, &repos(&b), &opts).await;
            assert!(
                r.remove.contains(&merged) && r.remove.contains(&older_path),
                "both on the list: {:?}",
                r.remove
            );
            let done = clean_for(&b.vault, &repos(&b), &r.hash, None, opts)
                .await
                .unwrap();
            assert_eq!(
                done.failed,
                [(
                    merged.clone(),
                    format!("Kept: {why}, since the list was shown")
                )],
                "bound: {bound}"
            );
            assert!(b.merged.exists(), "bound: {bound}");
            assert!(done.removed.contains(&older_path), "{:?}", done.removed);
            assert!(!done.removed.contains(&merged), "{:?}", done.removed);
            assert!(!older.exists(), "bound: {bound}");
            if bound {
                assert!(
                    crate::build_workspaces::binding_for(&b.vault, "ws-merged")
                        .unwrap()
                        .exists,
                    "its workspace keeps its worktree"
                );
            }
        }
    }

    /// Overnight R4: a Build pressed while a bound worktree's removal is
    /// stopping its services and checking for unsaved work holds it by the
    /// time `git worktree remove` comes. Read again right then, it stays with
    /// its workspace's binding, with why; the rest of the list still goes.
    #[tokio::test]
    async fn a_bound_worktree_held_during_its_removal_stays_with_its_binding() {
        let why = "a build running now";
        let b = bed();
        let older = b.merged.with_file_name("older");
        sh(
            &b.repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "b/older",
                older.to_str().unwrap(),
                "origin/main~1",
            ],
        );
        crate::build_workspaces::bind_repo_for_test(&b.vault, "ws-merged", &b.repo, &b.merged);
        let opts = Opts {
            held: Held::During(Holds {
                commits: vec![],
                paths: vec![(b.merged.clone(), why.to_string())],
            }),
            ..idle()
        };
        let merged = b.merged.to_string_lossy().into_owned();
        let older_path = older.to_string_lossy().into_owned();
        let r = report_for(&b.vault, &repos(&b), &opts).await;
        assert!(
            r.remove.contains(&merged) && r.remove.contains(&older_path),
            "both on the list: {:?}",
            r.remove
        );
        let done = clean_for(&b.vault, &repos(&b), &r.hash, None, opts)
            .await
            .unwrap();
        assert_eq!(
            done.failed,
            [(
                merged.clone(),
                format!("Kept: {why}, since the list was shown")
            )]
        );
        assert!(b.merged.exists());
        assert!(done.removed.contains(&older_path), "{:?}", done.removed);
        assert!(!older.exists());
        assert!(
            crate::build_workspaces::binding_for(&b.vault, "ws-merged")
                .unwrap()
                .exists,
            "its workspace keeps its worktree"
        );
    }

    #[tokio::test]
    async fn clean_removes_exactly_the_bound_list_and_keeps_branches() {
        let b = bed();
        // Its clean build's provenance outlives it (S12). Ignored, as in Hollow.
        let common = git(&b.repo, &["rev-parse", "--git-common-dir"], QUICK)
            .await
            .unwrap();
        let exclude = b.repo.join(common.trim()).join("info/exclude");
        let rules = fs::read_to_string(&exclude).unwrap();
        fs::write(&exclude, format!("{rules}build-provenance.json\n")).unwrap();
        let fp = "f".repeat(64);
        fs::write(
            b.merged.join("src-tauri/build-provenance.json"),
            format!(r#"{{"sourceFingerprint":"{fp}","commit":"c","dirty":false}}"#),
        )
        .unwrap();
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        // Something changed since the card (a new "~" folder): refused,
        // nothing removed.
        fs::create_dir_all(b.ahead.join("~")).unwrap();
        let err = clean_for(&b.vault, &repos(&b), &r.hash, None, idle())
            .await
            .unwrap_err();
        assert!(err.contains("Nothing was removed"), "{err}");
        assert!(b.merged.exists() && b.merged.join("src-tauri/~").exists());
        fs::remove_dir(b.ahead.join("~")).unwrap();

        let done = clean_for(&b.vault, &repos(&b), &r.hash, None, idle())
            .await
            .unwrap();
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert_eq!(done.removed, r.remove);
        assert!(!b.merged.exists(), "the merged worktree is gone");
        let kept = b
            .repo
            .join(common.trim())
            .join(crate::install_ready::RECORDS_DIR)
            .join(format!("{fp}.json"));
        assert!(fs::read_to_string(&kept)
            .unwrap()
            .contains(r#""gate": "build""#));
        assert!(
            b.ahead.exists() && b.dirty.exists() && b.fresh.exists(),
            "the others stay"
        );
        let branches = git(&b.repo, &["branch", "--list", "b/*"], QUICK)
            .await
            .unwrap();
        assert!(
            branches.contains("b/merged") && branches.contains("b/ahead"),
            "{branches}"
        );
        let list = git(&b.repo, &["worktree", "list", "--porcelain"], QUICK)
            .await
            .unwrap();
        assert!(!list.contains(b.merged.to_str().unwrap()), "{list}");
    }

    #[tokio::test]
    async fn a_bound_worktree_goes_through_its_workspace_and_links_are_never_followed() {
        let b = bed();
        let unbound = report_for(&b.vault, &repos(&b), &idle()).await;
        crate::build_workspaces::bind_repo_for_test(&b.vault, "ws-merged", &b.repo, &b.merged);
        // A link named "~", and a `src-tauri` that is a link holding a real
        // "~": neither is a "~" folder of this worktree.
        let elsewhere = tempfile::tempdir().unwrap();
        fs::create_dir_all(elsewhere.path().join("~")).unwrap();
        fs::write(elsewhere.path().join("~/keep"), "x").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), b.ahead.join("~")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), b.dirty.join("src-tauri")).unwrap();
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        assert_eq!(find(&r, &b.merged).workspace.as_deref(), Some("ws-merged"));
        assert_ne!(
            r.hash, unbound.hash,
            "how a worktree goes is part of what was approved"
        );
        assert!(
            !r.remove
                .iter()
                .any(|p| p.contains("/ahead/") || p.contains("/dirty/")),
            "{:?}",
            r.remove
        );
        let done = clean_for(&b.vault, &repos(&b), &r.hash, None, idle())
            .await
            .unwrap();
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert!(!b.merged.exists());
        assert!(
            !crate::build_workspaces::binding_for(&b.vault, "ws-merged")
                .unwrap()
                .exists,
            "the workspace falls back"
        );
        assert!(elsewhere.path().join("~/keep").exists());
    }

    /// F6 part B: a build workspace's extra worktree counts as bound and is
    /// never offered for removal, merged or not (its own Remove worktree
    /// keeps the binding right; this clean would leave it pointing nowhere).
    #[tokio::test]
    async fn a_lane_worktree_is_never_offered_for_removal() {
        let b = bed();
        crate::build_workspaces::bind_repo_for_test(&b.vault, "ws-lanes", &b.repo, &b.fresh);
        crate::build_workspaces::bind_lane_for_test(&b.vault, "ws-lanes", "merged", &b.merged);
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        let lane = find(&r, &b.merged);
        assert!(!lane.removable, "{lane:?}");
        assert!(
            lane.reason
                .as_deref()
                .is_some_and(|w| w.contains("extra worktree")),
            "{lane:?}"
        );
        assert!(
            !r.remove.iter().any(|p| Path::new(p) == b.merged),
            "{:?}",
            r.remove
        );
    }

    #[tokio::test]
    async fn a_repo_with_no_remote_default_keeps_every_worktree() {
        let b = bed();
        sh(&b.repo, &["remote", "set-head", "origin", "-d"]);
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        assert_eq!(r.repos[0].default_branch, None);
        assert!(r.repos[0].worktrees.iter().all(|w| !w.removable));
        assert_eq!(
            r.remove,
            [b.merged.join("src-tauri/~").to_string_lossy().into_owned()],
            "only the ~ folder"
        );
    }

    /// A repository with `n` merged, clean linked worktrees (older than
    /// origin/main's tip), each holding a fake build output folder.
    fn many(n: usize) -> Bed {
        let b = bed();
        let older = |i: usize| {
            b.repo
                .parent()
                .unwrap()
                .join("Many")
                .join(format!("w{i:03}"))
        };
        for i in 0..n {
            sh(
                &b.repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &format!("m/{i:03}"),
                    older(i).to_str().unwrap(),
                    "origin/main~1",
                ],
            );
            let out = older(i).join("node_modules/pkg");
            fs::create_dir_all(&out).unwrap();
            fs::write(out.join("index.js"), vec![1u8; 16 * 1024]).unwrap();
        }
        b
    }

    #[tokio::test]
    async fn three_hundred_worktrees_finish_with_a_measured_report() {
        let b = many(300);
        // The read action's own limit (`app_mcp::BROKER_TIMEOUT_SECS`, 60 s)
        // covers the report alone, with sizes and branches as the card reads.
        let t = std::time::Instant::now();
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        let scan = t.elapsed();
        assert!(
            scan < Duration::from_secs(60),
            "the report alone took {scan:?}; the read action's limit is 60 s (BROKER_TIMEOUT_SECS)"
        );
        let peak = PEAK.with(|p| p.get());
        assert!(
            peak > 1 && peak <= SCAN_PARALLEL,
            "worktree reads in flight peaked at {peak}, bound {SCAN_PARALLEL}"
        );
        let t = std::time::Instant::now();
        let listed: Vec<_> = r.repos[0].worktrees.iter().collect();
        assert_eq!(listed.len(), 304, "300 + the bed's four");
        // Same order as `git worktree list`, and the four of the bed keep
        // the verdicts they have alone.
        assert!(find(&r, &b.ahead).reason.is_some() && find(&r, &b.dirty).reason.is_some());
        let go = r.remove.iter().filter(|p| p.contains("/Many/")).count();
        assert_eq!(go, 300, "all of the fixture's are removable");
        // The default budget (60 s) can stop a loaded machine part-way and
        // say so (the next test); this one checks the whole 300 land.
        let all = Opts {
            budget: Some(Duration::from_secs(400)),
            ..idle()
        };
        let done = clean_for(&b.vault, &repos(&b), &r.hash, None, all)
            .await
            .unwrap();
        let took = t.elapsed();
        assert!(
            took < Duration::from_secs(120),
            "300 worktrees took {took:?}"
        );
        assert!(done.complete && done.remaining.is_empty(), "{done:?}");
        assert_eq!(
            done.removed.iter().filter(|p| p.contains("/Many/")).count(),
            300
        );
        assert!(done.failed.is_empty(), "{:?}", done.failed);
        assert!(b.ahead.exists() && b.dirty.exists(), "unmerged work stays");
        // Measured, not estimated: a volume was read twice, and the totals
        // are what those reads say.
        assert!(!done.volumes.is_empty());
        assert_eq!(
            done.free_before,
            done.volumes.iter().map(|v| v.free_before).sum::<u64>()
        );
        assert_eq!(
            done.freed,
            done.volumes
                .iter()
                .map(|v| v.free_after.saturating_sub(v.free_before))
                .sum::<u64>()
        );
        assert!(done.free_before > 0 && done.free_after > 0);
        assert!(done.elapsed_ms > 0);
    }

    #[tokio::test]
    async fn a_spent_budget_stops_removing_and_names_what_is_left() {
        let b = many(6);
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        let opts = Opts {
            budget: Some(Duration::ZERO),
            ..idle()
        };
        let done = clean_for(&b.vault, &repos(&b), &r.hash, None, opts)
            .await
            .unwrap();
        assert!(done.removed.is_empty() && !done.complete);
        assert!(done.remaining.len() >= 6, "{:?}", done.remaining);
        for p in &done.remaining {
            assert!(Path::new(p).exists(), "{p} was not touched");
        }
        // A second clean (a new card) finishes it.
        let r = report_for(&b.vault, &repos(&b), &idle()).await;
        let again = clean_for(&b.vault, &repos(&b), &r.hash, None, idle())
            .await
            .unwrap();
        assert!(again.complete && again.remaining.is_empty());
    }

    #[test]
    fn the_cleaned_report_serializes_with_both_free_space_readings() {
        let c = Cleaned {
            removed: vec!["/W/a".into()],
            volumes: vec![VolumeFree {
                path: "/V".into(),
                free_before: 100,
                free_after: 160,
                freed: 60,
            }],
            free_before: 100,
            free_after: 160,
            freed: 60,
            complete: true,
            ..Cleaned::default()
        };
        let v = serde_json::to_value(&c).unwrap();
        for k in [
            "removed",
            "failed",
            "remaining",
            "complete",
            "volumes",
            "freeBefore",
            "freeAfter",
            "freed",
            "elapsedMs",
        ] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        assert_eq!(v["volumes"][0]["freeAfter"], 160);
        assert_eq!(v["freed"], 60);
        // An old payload without the new fields still reads.
        let old: Cleaned = serde_json::from_str(r#"{"removed":[],"failed":[]}"#).unwrap();
        assert_eq!(old.freed, 0);
    }

    // ── OC6: reclaiming generated output ─────────────────────────────────

    /// The clock `days` ahead: everything a test made is that old then.
    fn later(days: u64) -> SystemTime {
        SystemTime::now() + Duration::from_secs(days * 86_400)
    }

    /// Ignored generated output in `dir`: `node_modules` (the bed's exclude
    /// file already ignores it), `target` and `dist` (added to it here).
    fn grow(b: &Bed, dir: &Path) {
        let exclude = b.repo.join(".git/info/exclude");
        let text = fs::read_to_string(&exclude).unwrap();
        if !text.contains("target") {
            fs::write(&exclude, format!("{text}target\ndist\n")).unwrap();
        }
        for rel in ["node_modules/pkg", "target/debug", "dist"] {
            fs::create_dir_all(dir.join(rel)).unwrap();
            fs::write(dir.join(rel).join("f"), "x").unwrap();
        }
    }

    fn touch(path: &Path, at: SystemTime) {
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    fn names(dirs: &[(String, u64)]) -> Vec<&str> {
        dirs.iter().map(|(p, _)| p.as_str()).collect()
    }

    #[tokio::test]
    async fn an_idle_worktree_offers_only_ignored_untracked_generated_folders() {
        let b = bed();
        grow(&b, &b.ahead);
        // Just committed: not idle yet, whatever it holds.
        let err = reclaimable(&b.ahead, SystemTime::now()).await.unwrap_err();
        assert!(err.contains("HEAD or the index moved"), "{err}");
        let found = reclaimable(&b.ahead, later(40)).await.unwrap();
        assert_eq!(names(&found), ["node_modules", "target", "dist"]);
        assert!(found.iter().all(|(_, days)| *days >= 39), "{found:?}");
        // A tracked `dist` is never offered, nor is one git doesn't ignore.
        fs::create_dir_all(b.dirty.join("dist")).unwrap();
        fs::write(b.dirty.join("dist/keep.txt"), "x").unwrap();
        sh(&b.dirty, &["add", "-f", "dist/keep.txt"]);
        sh(&b.dirty, &["commit", "-q", "-m", "track dist"]);
        fs::create_dir_all(b.dirty.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(b.dirty.join("target")).unwrap();
        let exclude = b.repo.join(".git/info/exclude");
        let text = fs::read_to_string(&exclude).unwrap();
        fs::write(&exclude, text.replace("target\n", "")).unwrap();
        let found = reclaimable(&b.dirty, later(40)).await.unwrap();
        assert_eq!(
            names(&found),
            ["node_modules"],
            "tracked dist and un-ignored target stay"
        );
        // A symlink named like a generated folder is left alone.
        #[cfg(unix)]
        {
            fs::remove_dir_all(b.ahead.join("dist")).unwrap();
            std::os::unix::fs::symlink(&b.repo, b.ahead.join("dist")).unwrap();
            let found = reclaimable(&b.ahead, later(40)).await.unwrap();
            assert!(!names(&found).contains(&"dist"), "{found:?}");
        }
    }

    #[tokio::test]
    async fn a_recent_change_or_a_recently_touched_folder_keeps_the_output() {
        let b = bed();
        grow(&b, &b.ahead);
        // An untracked file touched a day before "now" keeps everything.
        let f = b.ahead.join("scratch.txt");
        fs::write(&f, "x").unwrap();
        touch(&f, later(39));
        let err = reclaimable(&b.ahead, later(40)).await.unwrap_err();
        assert!(err.contains("changed path was touched"), "{err}");
        fs::remove_file(&f).unwrap();
        // A deleted tracked file can't be dated: also not idle.
        fs::remove_file(b.ahead.join("new.txt")).unwrap();
        let err = reclaimable(&b.ahead, later(40)).await.unwrap_err();
        assert!(err.contains("can't be dated"), "{err}");
        sh(&b.ahead, &["checkout", "--", "new.txt"]);
        // One folder touched inside its top two levels is kept, the rest offered.
        touch(&b.ahead.join("node_modules/pkg/f"), later(39));
        let found = reclaimable(&b.ahead, later(40)).await.unwrap();
        assert_eq!(names(&found), ["target", "dist"]);
    }

    #[tokio::test]
    async fn the_report_lists_reclaimable_folders_only_for_idle_worktrees_that_stay() {
        let b = bed();
        for dir in [&b.ahead, &b.dirty, &b.merged] {
            grow(&b, dir);
        }
        let r = report_at(&b.vault, &repos(&b), &idle(), SystemTime::now()).await;
        assert!(r.repos[0].worktrees.iter().all(|w| w.reclaim.is_empty()));
        assert_eq!(r.reclaim_bytes, 0);
        let now = later(40);
        let r = report_at(&b.vault, &repos(&b), &idle(), now).await;
        let a = find(&r, &b.ahead);
        assert_eq!(
            a.reclaim
                .iter()
                .map(|d| d.path.as_str())
                .collect::<Vec<_>>(),
            ["node_modules", "target", "dist"]
        );
        assert!(a.reclaim.iter().all(|d| d.bytes.is_some()));
        assert!(r.reclaim_bytes > 0);
        assert!(
            !find(&r, &b.dirty).reclaim.is_empty(),
            "old changes are idle"
        );
        let m = find(&r, &b.merged);
        assert!(
            m.removable && m.reclaim.is_empty(),
            "a removable one goes whole"
        );
        // The hash names folders, not sizes: stable between two reads.
        let again = report_at(&b.vault, &repos(&b), &idle(), now).await;
        assert_eq!(r.reclaim_hash, again.reclaim_hash);
        // A process working in it keeps it out of the list.
        let busy = Opts {
            cwds: Cwds::Given(vec![b.ahead.clone()]),
            ..idle()
        };
        let r2 = report_at(&b.vault, &repos(&b), &busy, now).await;
        assert!(find(&r2, &b.ahead).reclaim.is_empty());
        assert_ne!(r.reclaim_hash, r2.reclaim_hash);
        // So does an unreadable process list, an app built from it, a lock.
        let unknown = Opts {
            cwds: Cwds::Unknown,
            ..idle()
        };
        let r3 = report_at(&b.vault, &repos(&b), &unknown, now).await;
        assert!(r3.repos[0].worktrees.iter().all(|w| w.reclaim.is_empty()));
        let app = Opts {
            held: Held::Given(Holds {
                commits: vec![(crate::install_targets::ANY_COMMIT.to_string(), "app".into())],
                paths: vec![],
            }),
            ..idle()
        };
        let r4 = report_at(&b.vault, &repos(&b), &app, now).await;
        assert!(r4.repos[0].worktrees.iter().all(|w| w.reclaim.is_empty()));
        sh(&b.repo, &["worktree", "lock", b.ahead.to_str().unwrap()]);
        let r5 = report_at(&b.vault, &repos(&b), &idle(), now).await;
        assert!(find(&r5, &b.ahead).reclaim.is_empty());
        sh(&b.repo, &["worktree", "unlock", b.ahead.to_str().unwrap()]);
    }

    #[tokio::test]
    async fn reclaim_output_removes_only_the_listed_folders_and_keeps_the_worktree() {
        let b = bed();
        for dir in [&b.ahead, &b.merged] {
            grow(&b, dir);
        }
        let now = later(40);
        let r = report_at(&b.vault, &repos(&b), &idle(), now).await;
        // A stale hash refuses and removes nothing.
        let err = reclaim_output(&b.vault, &repos(&b), "stale", idle(), now)
            .await
            .unwrap_err();
        assert!(err.contains("changed since"), "{err}");
        assert!(b.ahead.join("node_modules").exists());
        // A process that appeared since the report changes the list: refused.
        let busy = Opts {
            cwds: Cwds::Given(vec![b.ahead.clone()]),
            ..idle()
        };
        assert!(
            reclaim_output(&b.vault, &repos(&b), &r.reclaim_hash, busy, now)
                .await
                .is_err()
        );
        assert!(b.ahead.join("node_modules").exists());
        // A build pressed after the list: kept, with why.
        let later_held = Opts {
            held: Held::Later {
                scan: Holds::default(),
                removal: Holds {
                    commits: vec![(crate::install_targets::ANY_COMMIT.to_string(), "app".into())],
                    paths: vec![],
                },
            },
            ..idle()
        };
        let kept = reclaim_output(&b.vault, &repos(&b), &r.reclaim_hash, later_held, now)
            .await
            .unwrap();
        assert!(
            kept.removed.is_empty() && kept.failed.len() == 3,
            "{kept:?}"
        );
        assert!(
            kept.failed.iter().all(|(_, why)| why.starts_with("Kept:")),
            "{kept:?}"
        );
        assert!(b.ahead.join("target").exists());
        // The real thing.
        let done = reclaim_output(&b.vault, &repos(&b), &r.reclaim_hash, idle(), now)
            .await
            .unwrap();
        assert!(
            done.failed.is_empty() && done.removed.len() == 3,
            "{done:?}"
        );
        for rel in ["node_modules", "target", "dist"] {
            assert!(!b.ahead.join(rel).exists(), "{rel}");
        }
        // The worktree, its commit's file and its branch stay; a removable
        // worktree's own output is left for `clean`.
        assert!(b.ahead.join("new.txt").is_file());
        sh(&b.repo, &["rev-parse", "--verify", "b/ahead"]);
        assert!(b.merged.join("node_modules").exists());
    }

    #[tokio::test]
    async fn a_folder_holding_a_secrets_file_or_a_database_is_not_offered() {
        let b = bed();
        grow(&b, &b.ahead);
        fs::write(b.ahead.join("dist/.env.production"), "K=v").unwrap();
        fs::write(b.ahead.join("target/debug/cache.db"), "x").unwrap();
        // Packages ship `.env.example` files: node_modules is installed from the lockfile.
        fs::write(b.ahead.join("node_modules/pkg/.env.example"), "A=").unwrap();
        let found = reclaimable(&b.ahead, later(40)).await.unwrap();
        assert_eq!(names(&found), ["node_modules"], "{found:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_generated_folder_reached_through_a_link_is_not_offered() {
        let b = bed();
        grow(&b, &b.ahead);
        // node_modules is a link to another folder of the worktree.
        fs::remove_dir_all(b.ahead.join("node_modules")).unwrap();
        std::os::unix::fs::symlink(b.ahead.join("dist"), b.ahead.join("node_modules")).unwrap();
        // src-tauri is a link: its target folder is reached through it.
        fs::create_dir_all(b.ahead.join("tools/target/debug")).unwrap();
        fs::write(b.ahead.join("tools/target/debug/f"), "x").unwrap();
        std::os::unix::fs::symlink(b.ahead.join("tools"), b.ahead.join("src-tauri")).unwrap();
        let found = reclaimable(&b.ahead, later(40)).await.unwrap();
        assert_eq!(names(&found), ["target", "dist"], "{found:?}");
    }

    #[tokio::test]
    async fn a_file_edited_deep_inside_an_untracked_folder_keeps_the_output() {
        let b = bed();
        grow(&b, &b.ahead);
        fs::create_dir_all(b.ahead.join("scratch/a/b")).unwrap();
        let f = b.ahead.join("scratch/a/b/c.txt");
        fs::write(&f, "x").unwrap();
        touch(&f, later(39));
        let err = reclaimable(&b.ahead, later(40)).await.unwrap_err();
        assert!(err.contains("changed path was touched"), "{err}");
    }

    #[tokio::test]
    async fn clean_reads_no_generated_folders() {
        let b = bed();
        grow(&b, &b.ahead);
        let off = Opts {
            reclaim: false,
            ..idle()
        };
        let r = report_at(&b.vault, &repos(&b), &off, later(40)).await;
        assert!(r.repos[0].worktrees.iter().all(|w| w.reclaim.is_empty()));
        assert_eq!(r.reclaim_bytes, 0);
        let on = report_at(&b.vault, &repos(&b), &idle(), later(40)).await;
        assert!(!find(&on, &b.ahead).reclaim.is_empty());
    }
}
