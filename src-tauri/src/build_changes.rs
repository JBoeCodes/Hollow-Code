//! The Changes pane of a build workspace (Hollow build programme, PR B2; see
//! docs/history/HOLLOW_BUILD_PLAN.md): the worktree's branch against its base, the
//! diff, and Commit / Push / Open PR.
//!
//! Everything here runs the user's own git in the workspace's worktree
//! (`build_workspaces::git_raw`: `GIT_*` stripped, no prompt, through
//! `process_group`). Reads pass `--no-optional-locks`, so refreshing the pane
//! never takes `index.lock` from under an agent that is using git in the same
//! worktree. Mutations (commit, push, open PR) refuse while any Hollow agent
//! run holds the vault, for the same reason.
//!
//! "The diff" is the worktree against the merge-base of its base and HEAD:
//! committed work on the branch, uncommitted tracked changes, and untracked
//! files (as additions). Diff text shown to the user goes through
//! `claude_runner::redact`, and is capped (1 MiB total, 256 KiB per file).
//!
//! The PR lookup and `gh pr create` use the GitHub CLI when it is installed
//! (`/opt/homebrew/bin/gh`, `/usr/local/bin/gh`, else PATH). A missing `gh`
//! or a branch with no PR is `pr: None`, never an error.

use crate::build_workspaces::{self, git_error, git_raw, BuildInfo, QUICK};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Most commits the pane lists.
const COMMITS_MAX: usize = 50;
/// Whole diff text returned by [`diff`].
const DIFF_TOTAL_MAX: usize = 1024 * 1024;
/// One file's share of it.
const DIFF_FILE_MAX: usize = 256 * 1024;
/// Untracked files larger than this are listed with no line count.
const UNTRACKED_COUNT_MAX: u64 = 2 * 1024 * 1024;
/// Most files [`status`] lists (the rest are counted in `more_files`).
const FILES_MAX: usize = 500;
const SLOW: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    /// The old path of a rename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: FileStatus,
    /// Lines; `None` for a binary file (or an untracked one too big to count).
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    /// Has changes in the index that are not yet committed.
    pub staged: bool,
    /// Has changes not yet committed (staged, unstaged or untracked).
    pub uncommitted: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchCommit {
    pub sha: String,
    pub subject: String,
    /// Committer date, RFC 3339.
    pub when: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub url: String,
    /// `open`, `merged` or `closed`.
    pub state: String,
    pub number: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStatus {
    /// The same refusal used by mutations, without the agent-caller exemption.
    pub blocked: Option<String>,
    pub worktree: String,
    /// The worktree's current branch (empty on a detached HEAD).
    pub branch: String,
    pub base: String,
    /// HEAD's short sha.
    pub head: String,
    /// Commits on HEAD that the base doesn't have.
    pub ahead: u64,
    /// Commits on the base that HEAD doesn't have.
    pub behind: u64,
    /// The upstream branch (`origin/<branch>`), once pushed.
    pub upstream: Option<String>,
    /// Commits the upstream doesn't have yet (0 without an upstream).
    pub unpushed: u64,
    pub files: Vec<ChangedFile>,
    /// Files past [`FILES_MAX`], not listed.
    pub more_files: usize,
    /// Commits in HEAD that the base doesn't have, newest first (≤ 50).
    pub commits: Vec<BranchCommit>,
    pub pr: Option<PullRequest>,
    /// The workspace's outcome (its `## Outcome` section), for a PR body.
    pub goal: String,
    /// The last successful install's commit contains HEAD (`git merge-base
    /// --is-ancestor`), so a merged branch counts as installed (P1-46);
    /// `None` when no install is recorded or git can't tell.
    pub installed_contains_head: Option<bool>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BuildDiff {
    /// Unified diff text, redacted.
    pub diff: String,
    /// Stopped at the total cap: some files are missing.
    pub truncated: bool,
    /// Files whose diff was cut at the per-file cap.
    pub truncated_files: Vec<String>,
}

/// The live binding of workspace `id`, or a readable refusal.
fn live_binding(vault: &Path, id: &str) -> Result<BuildInfo, String> {
    let info = build_workspaces::binding_for(vault, id).ok_or("This workspace has no worktree")?;
    if !info.exists {
        return Err("This workspace's worktree was removed".into());
    }
    Ok(info)
}

/// Refuse while an agent could be running git in this worktree: its own
/// turn (build programme PR B3, which holds no vault lease; [`primary_holder`]:
/// once the workspace has lanes, only the primary's own turn, since its
/// folder's conversations are then given no worktree) or any run holding
/// the vault.
fn refuse_while_busy(
    vault: &Path,
    id: &str,
    doing: &str,
    caller: Option<&str>,
) -> Result<(), String> {
    if primary_holder(vault, id).is_some() {
        return Err(format!(
            "Wait for this workspace's agent turn to finish before you {doing}"
        ));
    }
    refuse_while_vault_busy(vault, doing, caller)
}

/// The live run holding build workspace `id`'s primary worktree, in the
/// mode its turns take (`build_workspaces::turn_mode`): with no lanes any
/// run in the workspace's folder (its chats are told to work there), with
/// lanes its own turn (the worktree's lock) or a run holding the folder
/// alone.
fn primary_holder(vault: &Path, id: &str) -> Option<String> {
    let info = build_workspaces::binding_for(vault, id)?;
    let mode = build_workspaces::turn_mode(vault, id, &info.worktree);
    crate::automated_history::mode_holder(vault, &format!("workspaces/{id}"), &mode)
}

/// Refuse a per-workspace build action (commit, push, pull request, update;
/// the app build and the setup hook, in `app_actions`) from a turn in another worktree of the same workspace (F6 part B review):
/// these act on the workspace's own worktree, and a lane's agent calling
/// one would commit or push the primary's work in progress.
pub(crate) fn refuse_from_another_worktree(
    vault: &Path,
    id: &str,
    caller: Option<&str>,
) -> Result<(), String> {
    let Some(key) = caller.and_then(|run| crate::automated_history::lock_of(vault, run)) else {
        return Ok(());
    };
    let lane = build_workspaces::lanes_for(vault, id)
        .into_iter()
        .find(|l| {
            l.name != build_workspaces::MAIN_LANE
                && build_workspaces::lock_key(Path::new(&l.worktree)) == key
        });
    match lane {
        Some(lane) => Err(format!(
            "This conversation works in worktree {} of this workspace; Hollow's commit, push, pull request and update act on the workspace's own worktree. Use git in your worktree ({}) instead.",
            lane.name, lane.worktree
        )),
        None => Ok(()),
    }
}

/// [`refuse_while_busy`]'s vault-wide half: any run holding the vault but
/// the caller's own.
fn refuse_while_vault_busy(vault: &Path, doing: &str, caller: Option<&str>) -> Result<(), String> {
    // A Hollow app call from the run that holds the vault (PR A3) is that
    // run asking, after the user allowed it on a card: not another writer.
    if caller.is_some_and(|run| crate::automated_history::lease_is_run(vault, run)) {
        return Ok(());
    }
    match crate::agent_sessions::busy_reason(vault) {
        Some(holder) => Err(format!("Wait for {holder} to finish before you {doing}")),
        None => Ok(()),
    }
}

/// Git that never takes optional locks (reads).
async fn read_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["--no-optional-locks"];
    full.extend_from_slice(args);
    let out = git_raw(dir, &full, QUICK, 16 * 1024 * 1024).await?;
    if out.code != Some(0) {
        return Err(git_error(args, &out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn merge_base(tree: &Path, base: &str) -> Result<String, String> {
    read_git(tree, &["merge-base", base, "HEAD"])
        .await
        .map(|s| s.trim().to_string())
        .map_err(|_| format!("{base} and this branch share no history (was the base deleted?)"))
}

async fn current_branch(tree: &Path) -> String {
    read_git(tree, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// The branch's upstream once it was pushed. An upstream equal to the base
/// is not "pushed": B1 worktrees created before `--no-track` track the
/// `origin/main` they started from.
async fn upstream_of(tree: &Path, base: &str) -> Option<String> {
    read_git(
        tree,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .await
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty() && s != base)
}

/// `git diff --numstat -z` records: `(path, old path, additions, deletions)`;
/// binary files have `None` counts.
pub(crate) fn parse_numstat_z(
    raw: &str,
) -> Vec<(String, Option<String>, Option<u64>, Option<u64>)> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0');
    while let Some(head) = parts.next() {
        if head.is_empty() {
            continue;
        }
        let mut cols = head.splitn(3, '\t');
        let (Some(a), Some(d), Some(rest)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let (path, old) = if rest.is_empty() {
            // A rename: `a\td\t\0old\0new\0`.
            let old = parts.next().unwrap_or_default().to_string();
            let new = parts.next().unwrap_or_default().to_string();
            (new, Some(old))
        } else {
            (rest.to_string(), None)
        };
        out.push((path, old, a.parse().ok(), d.parse().ok()));
    }
    out
}

/// `git diff --name-status -z` records: `(path, status letter, old path)`.
pub(crate) fn parse_name_status_z(raw: &str) -> Vec<(String, char, Option<String>)> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0');
    while let Some(code) = parts.next() {
        let Some(letter) = code.chars().next() else {
            continue;
        };
        if matches!(letter, 'R' | 'C') {
            let old = parts.next().unwrap_or_default().to_string();
            let new = parts.next().unwrap_or_default().to_string();
            out.push((new, letter, Some(old)));
        } else if let Some(path) = parts.next() {
            out.push((path.to_string(), letter, None));
        }
    }
    out
}

fn split_z(raw: &str) -> impl Iterator<Item = &str> {
    raw.split('\0').filter(|s| !s.is_empty())
}

/// Line count of an untracked file; `None` for a binary or oversized one.
pub(crate) fn count_lines(path: &Path) -> Option<u64> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return Some(1);
    }
    if !meta.is_file() || meta.len() > UNTRACKED_COUNT_MAX {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes[..bytes.len().min(8000)].contains(&0) {
        return None;
    }
    if bytes.is_empty() {
        return Some(0);
    }
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    Some(if bytes.ends_with(b"\n") {
        newlines
    } else {
        newlines + 1
    })
}

/// The text of the brief's `## Outcome` section.
fn workspace_goal(vault: &Path, id: &str) -> String {
    if id.contains('/') || id.contains("..") {
        return String::new();
    }
    let Ok(text) = std::fs::read_to_string(vault.join("workspaces").join(id).join("WORKSPACE.md"))
    else {
        return String::new();
    };
    let mut inside = false;
    let mut goal = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            if inside {
                break;
            }
            inside = line.trim() == "## Outcome";
            continue;
        }
        if inside {
            goal.push(line);
        }
    }
    let goal = goal.join("\n").trim().to_string();
    goal.chars().take(4000).collect()
}

/// The branch, its base, ahead/behind, changed files and commits.
pub async fn status(vault: &Path, id: &str) -> Result<BuildStatus, String> {
    status_with(vault, id, gh_binary().as_deref()).await
}

pub(crate) async fn status_with(
    vault: &Path,
    id: &str,
    gh: Option<&Path>,
) -> Result<BuildStatus, String> {
    let info = live_binding(vault, id)?;
    let tree = PathBuf::from(&info.worktree);
    let base = info.base.clone();
    let head = read_git(&tree, &["rev-parse", "--short=12", "HEAD"])
        .await?
        .trim()
        .to_string();
    let branch = current_branch(&tree).await;
    let mb = merge_base(&tree, &base).await?;
    let counts = read_git(
        &tree,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base}...HEAD"),
        ],
    )
    .await?;
    let mut nums = counts
        .split_whitespace()
        .map(|n| n.parse::<u64>().unwrap_or(0));
    let behind = nums.next().unwrap_or(0);
    let ahead = nums.next().unwrap_or(0);
    let upstream = upstream_of(&tree, &base).await;
    let unpushed = match &upstream {
        Some(up) => read_git(&tree, &["rev-list", "--count", &format!("{up}..HEAD")])
            .await
            .ok()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0),
        None => 0,
    };

    // Changed files: the worktree against the merge-base (committed and
    // uncommitted tracked changes), then untracked files.
    let names = read_git(&tree, &["diff", "-M", "--name-status", "-z", &mb]).await?;
    let stats = read_git(&tree, &["diff", "-M", "--numstat", "-z", &mb]).await?;
    let staged: BTreeSet<String> =
        split_z(&read_git(&tree, &["diff", "--cached", "--name-only", "-z"]).await?)
            .map(str::to_string)
            .collect();
    let dirty: BTreeSet<String> =
        split_z(&read_git(&tree, &["diff", "--name-only", "-z", "HEAD"]).await?)
            .map(str::to_string)
            .collect();
    let untracked: Vec<String> =
        split_z(&read_git(&tree, &["ls-files", "--others", "--exclude-standard", "-z"]).await?)
            .map(str::to_string)
            .collect();
    let counts: BTreeMap<String, (Option<u64>, Option<u64>)> = parse_numstat_z(&stats)
        .into_iter()
        .map(|(p, _, a, d)| (p, (a, d)))
        .collect();
    let mut files: Vec<ChangedFile> = parse_name_status_z(&names)
        .into_iter()
        .map(|(path, letter, old_path)| {
            let (additions, deletions) = counts.get(&path).cloned().unwrap_or((None, None));
            let status = match letter {
                'A' => FileStatus::Added,
                'D' => FileStatus::Deleted,
                'R' | 'C' => FileStatus::Renamed,
                _ => FileStatus::Modified,
            };
            let uncommitted =
                dirty.contains(&path) || old_path.as_ref().is_some_and(|o| dirty.contains(o));
            ChangedFile {
                staged: staged.contains(&path),
                uncommitted,
                path,
                old_path,
                status,
                additions,
                deletions,
            }
        })
        .collect();
    for path in untracked {
        let lines = count_lines(&tree.join(&path));
        files.push(ChangedFile {
            path,
            old_path: None,
            status: FileStatus::Untracked,
            additions: lines,
            deletions: lines.map(|_| 0),
            staged: false,
            uncommitted: true,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let more_files = files.len().saturating_sub(FILES_MAX);
    files.truncate(FILES_MAX);

    let log = read_git(
        &tree,
        &[
            "log",
            &format!("-n{COMMITS_MAX}"),
            "--format=%h%x1f%s%x1f%cI%x1e",
            &format!("{base}..HEAD"),
        ],
    )
    .await?;
    let commits = log
        .split('\u{1e}')
        .filter_map(|record| {
            let mut f = record.trim_start_matches('\n').splitn(3, '\u{1f}');
            let sha = f.next()?.trim();
            (!sha.is_empty()).then(|| BranchCommit {
                sha: sha.to_string(),
                subject: f.next().unwrap_or_default().to_string(),
                when: f.next().unwrap_or_default().trim().to_string(),
            })
        })
        .collect();

    let pr = match (gh, branch.is_empty()) {
        (Some(gh), false) => pr_for(gh, &tree, &branch).await,
        _ => None,
    };
    let vault_owned = vault.to_path_buf();
    let id_owned = id.to_string();
    let goal = tokio::task::spawn_blocking(move || workspace_goal(&vault_owned, &id_owned))
        .await
        .unwrap_or_default();
    let installed_contains_head = match crate::self_install::last_receipt() {
        Some(r) if r.result.ok => contains_commit(&tree, &r.result.commit).await,
        _ => None,
    };
    Ok(BuildStatus {
        blocked: refuse_while_busy(vault, id, "commit or publish", None).err(),
        worktree: info.worktree,
        branch,
        base,
        head,
        ahead,
        behind,
        upstream,
        unpushed,
        files,
        more_files,
        commits,
        pr,
        goal,
        installed_contains_head,
    })
}

/// Whether `commit` (an installed app's) contains the worktree's HEAD:
/// `None` for a commit that isn't a hex id or that this repository doesn't
/// have.
async fn contains_commit(tree: &Path, commit: &str) -> Option<bool> {
    let commit = commit.trim();
    if commit.len() < 7 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let args = [
        "--no-optional-locks",
        "merge-base",
        "--is-ancestor",
        "HEAD",
        commit,
    ];
    match git_raw(tree, &args, QUICK, 4096).await.ok()?.code {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// What the rail shows for a build workspace (Conductor-style): its branch,
/// how far it is ahead of its base, and its pull request's state.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RailGitState {
    pub workspace_id: String,
    pub branch: String,
    pub ahead: u64,
    /// `open`, `merged`, `closed` (gh's state, lower-cased), or null with no PR.
    pub pr_state: Option<String>,
    pub pr_number: Option<u64>,
}

/// How long a rail state is reused: the rail asks on every refresh, and a
/// PR lookup is a `gh` call over the network.
const RAIL_TTL: Duration = Duration::from_secs(120);

/// A cached rail state, with the worktree's [`git_stamp`] when it was read.
type RailEntry = (std::time::Instant, GitStamp, RailGitState);
static RAIL_CACHE: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<(PathBuf, String), RailEntry>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Bumped by [`forget_rail_state`] per workspace, so a read that started
/// before the forget (its `gh pr view` saw no PR yet) can't put its stale
/// answer back: `gh pr create` moves no ref the stamp watches.
static RAIL_GENERATION: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<(PathBuf, String), u64>>,
> = once_cell::sync::Lazy::new(Default::default);

fn rail_generation(key: &(PathBuf, String)) -> u64 {
    RAIL_GENERATION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .copied()
        .unwrap_or(0)
}

/// Drop a workspace's cached rail state: Hollow just committed, pushed or
/// opened a pull request there (overnight Y7).
pub(crate) fn forget_rail_state(vault: &Path, id: &str) {
    let key = (vault.to_path_buf(), id.to_string());
    // Bumped under the cache lock, so an insert either sees the new
    // generation or is removed here.
    let mut cache = RAIL_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *RAIL_GENERATION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key.clone())
        .or_default() += 1;
    cache.remove(&key);
}

/// Size and modification time (ns) of each file that moves when the rail's
/// branch or ahead count can: the worktree's `HEAD` and its reflog (every
/// commit, checkout, reset or rebase there appends to it), the shared
/// `packed-refs`, and the base's loose ref (a fetch). Missing files count
/// as `None`. Read from disk without spawning git, so checking it on every
/// rail refresh is cheap (overnight Y7).
type GitStamp = Vec<Option<(u64, i128)>>;
fn git_stamp(tree: &Path, base: &str) -> GitStamp {
    let stat = |p: &Path| {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(p).ok().map(|m| {
            (
                m.len(),
                m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128,
            )
        })
    };
    let dot = tree.join(".git");
    // A linked worktree's `.git` is a file naming its private git dir.
    let git_dir = match std::fs::read_to_string(&dot) {
        Ok(text) => match text.trim().strip_prefix("gitdir:") {
            Some(dir) => tree.join(dir.trim()),
            None => return vec![stat(&dot)],
        },
        Err(_) => dot,
    };
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .map(|c| git_dir.join(c.trim()))
        .unwrap_or_else(|_| git_dir.clone());
    let mut stamp = vec![
        stat(&git_dir.join("HEAD")),
        stat(&git_dir.join("logs/HEAD")),
        stat(&common.join("packed-refs")),
        // A reftable repository keeps every ref in its tables.
        stat(&common.join("reftable/tables.list")),
    ];
    // The checked-out branch's own ref (a commit moves it even with the
    // reflog turned off), then the base's.
    let head = std::fs::read_to_string(git_dir.join("HEAD")).unwrap_or_default();
    let branch = head
        .trim()
        .strip_prefix("ref:")
        .map(|r| r.trim().to_string());
    let bases = match base {
        b if b.starts_with("refs/") => vec![b.to_string()],
        b if !b.is_empty() => vec![format!("refs/remotes/{b}"), format!("refs/heads/{b}")],
        _ => vec![],
    };
    for r in branch.into_iter().chain(bases) {
        if r.starts_with("refs/") && !r.split('/').any(|c| c == ".." || c.is_empty()) {
            stamp.push(stat(&common.join(&r)));
        }
    }
    stamp
}

/// The rail states of these build workspaces. Each is reused for
/// [`RAIL_TTL`] while its worktree's [`git_stamp`] holds, so a commit, a
/// checkout or a fetch shows at once; the ones in `fresh` (a turn there just
/// ended, and may have opened a pull request) are always read again.
/// A workspace without a live worktree, or whose git read fails, is left out;
/// a missing `gh` just means no PR state.
pub async fn rail_states(vault: &Path, ids: &[String], fresh: &[String]) -> Vec<RailGitState> {
    let gh = gh_binary();
    let mut bindings: std::collections::HashMap<String, BuildInfo> =
        build_workspaces::all_bindings(vault).into_iter().collect();
    let mut tasks = tokio::task::JoinSet::new();
    let mut out = Vec::new();
    for id in ids.iter().take(40) {
        let Some(info) = bindings.remove(id).filter(|b| b.exists) else {
            continue;
        };
        let key = (vault.to_path_buf(), id.clone());
        let stamp = git_stamp(Path::new(&info.worktree), &info.base);
        let cached = if fresh.contains(id) {
            None
        } else {
            RAIL_CACHE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
                .filter(|(at, seen, _)| at.elapsed() < RAIL_TTL && *seen == stamp)
                .map(|(_, _, s)| s.clone())
        };
        if let Some(state) = cached {
            out.push(state);
            continue;
        }
        let generation = rail_generation(&key);
        let gh = gh.clone();
        tasks.spawn(async move {
            let state = rail_state(&key.1, &info, gh.as_deref()).await;
            (key, state, stamp, generation)
        });
    }
    while let Some(Ok((key, state, stamp, generation))) = tasks.join_next().await {
        if let Some(state) = state {
            let mut cache = RAIL_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            // Forgotten while this read ran: answer with it, but don't keep it.
            if rail_generation(&key) == generation {
                cache.insert(key, (std::time::Instant::now(), stamp, state.clone()));
            }
            out.push(state);
        }
    }
    out.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));
    out
}

async fn rail_state(id: &str, info: &BuildInfo, gh: Option<&Path>) -> Option<RailGitState> {
    let tree = PathBuf::from(&info.worktree);
    let branch = current_branch(&tree).await;
    let ahead = read_git(
        &tree,
        &["rev-list", "--count", &format!("{}..HEAD", info.base)],
    )
    .await
    .ok()
    .and_then(|n| n.trim().parse().ok())
    .unwrap_or(0);
    let pr = match (gh, branch.is_empty()) {
        (Some(gh), false) => pr_for(gh, &tree, &branch).await,
        _ => None,
    };
    Some(RailGitState {
        workspace_id: id.to_string(),
        branch,
        ahead,
        pr_state: pr.as_ref().map(|p| p.state.clone()),
        pr_number: pr.map(|p| p.number),
    })
}

/// A repo-relative path inside the worktree: no `..`, not absolute.
pub(crate) fn safe_relative(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("No file given".into());
    }
    let p = Path::new(trimmed);
    if p.is_absolute()
        || p.components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!("{trimmed} is not a path inside the worktree"));
    }
    Ok(trimmed.to_string())
}

/// Cut `chunk` (one file's diff) at a line boundary within `max` bytes.
fn cap_chunk(chunk: &str, max: usize) -> (&str, bool) {
    if chunk.len() <= max {
        return (chunk, false);
    }
    let mut end = max;
    while !chunk.is_char_boundary(end) {
        end -= 1;
    }
    let end = chunk[..end].rfind('\n').map(|i| i + 1).unwrap_or(end);
    (&chunk[..end], true)
}

/// Split a unified diff at its `diff --git` headers.
fn file_chunks(text: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = text
        .match_indices("\ndiff --git ")
        .map(|(i, _)| i + 1)
        .collect();
    if text.starts_with("diff --git ") {
        starts.insert(0, 0);
    }
    if starts.is_empty() {
        return if text.is_empty() { vec![] } else { vec![text] };
    }
    let mut out = Vec::new();
    for (n, start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(text.len());
        out.push(&text[*start..end]);
    }
    out
}

/// The name a `diff --git a/x b/y` chunk is about (its `+++`/`---` path).
fn chunk_path(chunk: &str) -> String {
    for line in chunk.lines().take(8) {
        if let Some(p) = line
            .strip_prefix("+++ b/")
            .or_else(|| line.strip_prefix("--- a/"))
        {
            return p.trim_end_matches('\t').to_string();
        }
    }
    chunk
        .lines()
        .next()
        .and_then(|l| l.rsplit(" b/").next())
        .unwrap_or_default()
        .to_string()
}

/// Assemble capped, redacted diff text from per-file chunks.
pub(crate) fn assemble(chunks: Vec<String>) -> BuildDiff {
    let mut diff = String::new();
    let mut truncated = false;
    let mut truncated_files = Vec::new();
    for chunk in &chunks {
        for piece in file_chunks(chunk) {
            let (kept, cut) = cap_chunk(piece, DIFF_FILE_MAX);
            if diff.len() + kept.len() > DIFF_TOTAL_MAX {
                truncated = true;
                break;
            }
            if cut {
                truncated_files.push(chunk_path(piece));
            }
            diff.push_str(kept);
            if !diff.ends_with('\n') {
                diff.push('\n');
            }
        }
        if truncated {
            break;
        }
    }
    BuildDiff {
        diff: crate::claude_runner::redact(&diff),
        truncated,
        truncated_files,
    }
}

/// The worktree's diff against the merge-base, for one file or all of them.
pub async fn diff(vault: &Path, id: &str, path: Option<&str>) -> Result<BuildDiff, String> {
    let info = live_binding(vault, id)?;
    let tree = PathBuf::from(&info.worktree);
    let path = path.map(safe_relative).transpose()?;
    let mb = merge_base(&tree, &info.base).await?;
    worktree_diff(&tree, &mb, path.as_deref()).await
}

/// The worktree against `base` (a commit or tree): tracked changes plus
/// untracked files as additions, redacted and capped. `path` must already be
/// a safe relative path. Shared by the Changes pane and the card's Diff tab.
pub(crate) async fn worktree_diff(
    tree: &Path,
    base: &str,
    path: Option<&str>,
) -> Result<BuildDiff, String> {
    let path = path.map(str::to_string);
    let mb = base.to_string();
    let tree = tree.to_path_buf();
    let cap = DIFF_TOTAL_MAX + 1;
    let mut args = vec![
        "--no-optional-locks",
        "diff",
        "-M",
        "--no-color",
        "--no-ext-diff",
        mb.as_str(),
    ];
    if let Some(p) = &path {
        args.extend_from_slice(&["--", p.as_str()]);
    }
    let out = git_raw(&tree, &args, QUICK, cap).await?;
    if !out.truncated && out.code != Some(0) {
        return Err(git_error(&args, &out.stderr));
    }
    let mut chunks = vec![String::from_utf8_lossy(&out.stdout).into_owned()];
    let mut truncated = out.truncated;
    // Untracked files: `git diff --no-index /dev/null <file>` (exit 1 = differs).
    let mut untracked_args = vec![
        "--no-optional-locks",
        "ls-files",
        "--others",
        "--exclude-standard",
        "-z",
    ];
    if let Some(p) = &path {
        untracked_args.extend_from_slice(&["--", p.as_str()]);
    }
    let listed = git_raw(&tree, &untracked_args, QUICK, 16 * 1024 * 1024).await?;
    let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
    let mut used: usize = chunks[0].len();
    for file in split_z(&listed) {
        if used > DIFF_TOTAL_MAX {
            truncated = true;
            break;
        }
        let args = [
            "--no-optional-locks",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-index",
            "--",
            "/dev/null",
            file,
        ];
        let out = git_raw(&tree, &args, QUICK, DIFF_FILE_MAX + 1).await?;
        if !out.truncated && !matches!(out.code, Some(0) | Some(1)) {
            continue;
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        used += text.len();
        chunks.push(text);
    }
    let mut result = assemble(chunks);
    result.truncated |= truncated;
    Ok(result)
}

/// `git add -A` and commit everything with `message`.
pub async fn commit(vault: &Path, id: &str, message: &str) -> Result<BuildStatus, String> {
    commit_for(vault, id, message, None).await
}

/// [`commit`], also allowed while `caller` (a Hollow app caller's run id)
/// is the run holding the vault.
pub(crate) async fn commit_for(
    vault: &Path,
    id: &str,
    message: &str,
    caller: Option<&str>,
) -> Result<BuildStatus, String> {
    let info = live_binding(vault, id)?;
    let message = message.trim();
    if message.is_empty() {
        return Err("Write a commit message first".into());
    }
    refuse_from_another_worktree(vault, id, caller)?;
    refuse_while_busy(vault, id, "commit", caller)?;
    let tree = PathBuf::from(&info.worktree);
    let add = git_raw(&tree, &["add", "-A"], SLOW, 64 * 1024).await?;
    if add.code != Some(0) {
        return Err(git_error(&["add", "-A"], &add.stderr));
    }
    // `--ignore-submodules=none`: a staged submodule commit still counts
    // (`git_command` ignores submodules; a cached diff reads no submodule).
    let staged = git_raw(
        &tree,
        &["diff", "--cached", "--quiet", "--ignore-submodules=none"],
        QUICK,
        1024,
    )
    .await?;
    if staged.code == Some(0) {
        return Err("Nothing to commit: the worktree has no changes".into());
    }
    let out = git_raw(&tree, &["commit", "-q", "-m", message], SLOW, 64 * 1024).await?;
    if out.code != Some(0) {
        return Err(format!(
            "Commit failed: {}",
            redacted_error(&["commit"], &out)
        ));
    }
    changed(vault, id);
    status(vault, id).await
}

pub(crate) fn redacted_error(args: &[&str], out: &build_workspaces::GitOutput) -> String {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let text = if out.stderr.trim().is_empty() {
        stdout.as_ref()
    } else {
        out.stderr.as_str()
    };
    crate::claude_runner::redact(&git_error(args, text))
}

async fn push_branch(tree: &Path) -> Result<(), String> {
    let branch = current_branch(tree).await;
    if branch.is_empty() {
        return Err(
            "The worktree is not on a branch (detached HEAD); check out a branch first".into(),
        );
    }
    let args = ["push", "-u", "origin", branch.as_str()];
    let out = git_raw(tree, &args, SLOW, 256 * 1024).await?;
    if out.code != Some(0) {
        return Err(format!("Push failed: {}", redacted_error(&args, &out)));
    }
    Ok(())
}

/// `git push -u origin <branch>`.
pub async fn push(vault: &Path, id: &str) -> Result<BuildStatus, String> {
    push_for(vault, id, None).await
}

/// [`push`], also allowed while `caller` is the run holding the vault.
pub(crate) async fn push_for(
    vault: &Path,
    id: &str,
    caller: Option<&str>,
) -> Result<BuildStatus, String> {
    let info = live_binding(vault, id)?;
    refuse_from_another_worktree(vault, id, caller)?;
    refuse_while_busy(vault, id, "push", caller)?;
    push_branch(Path::new(&info.worktree)).await?;
    changed(vault, id);
    status(vault, id).await
}

/// Update from <base> (P1-46): fetch the base from `origin` (a remote base)
/// and rebase the branch onto it, or merge it in once the branch is pushed.
/// Refused while an agent works here or with uncommitted changes; a failure
/// (a conflict, a timeout) aborts, so the branch is left exactly as it was.
pub async fn update_from_base(vault: &Path, id: &str) -> Result<BuildStatus, String> {
    let info = live_binding(vault, id)?;
    refuse_while_busy(vault, id, "update this branch", None)?;
    update_branch(vault, id, &info).await?;
    status(vault, id).await
}

/// The History source of the reservation [`update_from_base_for`] holds.
pub(crate) const UPDATE_RESERVATION_SOURCE: &str = "buildUpdate";

/// [`update_from_base`] for a Hollow app caller (`build.update_from_base`,
/// COO parity L13), which runs outside any History lease: the workspace's
/// folder is reserved first (an exclusive scoped run, released when the
/// update ends, however it ends), so no agent turn starts there while git
/// fetches and rebases or merges, and one already there refuses it. The
/// vault-wide check exempts only `caller`'s own run, when it is the one
/// holding the vault, never another run.
pub(crate) async fn update_from_base_for(
    vault: &Path,
    id: &str,
    caller: &str,
) -> Result<BuildStatus, String> {
    const DOING: &str = "update this branch";
    let info = live_binding(vault, id)?;
    refuse_from_another_worktree(vault, id, Some(caller).filter(|run| !run.is_empty()))?;
    // In the mode the primary's turns take (F6 part B): with no lanes the
    // whole folder, as before; with lanes its lock, so the folder's
    // conversations and other worktrees don't keep the update out.
    let reservation = crate::automated_history::try_begin_scoped_mode(
        vault,
        &format!("workspaces/{id}"),
        build_workspaces::turn_mode(vault, id, &info.worktree),
        UPDATE_RESERVATION_SOURCE,
        None,
    )
    .map_err(|e| match e {
        crate::automated_history::BeginRunError::Busy(_) => {
            format!("Wait for this workspace's agent turn to finish before you {DOING}")
        }
        crate::automated_history::BeginRunError::Failed(why) => {
            format!("Hollow couldn't reserve this workspace to {DOING}: {why}")
        }
    })?;
    let caller = Some(caller).filter(|run| !run.is_empty());
    let result = match refuse_while_vault_busy(vault, DOING, caller) {
        Ok(()) => update_branch(vault, id, &info).await,
        Err(why) => Err(why),
    };
    let _ = reservation.finish(if result.is_ok() { "success" } else { "error" });
    result?;
    status(vault, id).await
}

/// Update from base's git work, once the caller checked (and for an app
/// caller reserved) the workspace: fetch, then rebase or merge, undone on
/// a failure.
async fn update_branch(vault: &Path, id: &str, info: &BuildInfo) -> Result<(), String> {
    let tree = PathBuf::from(&info.worktree);
    let base = info.base.as_str();
    if current_branch(&tree).await.is_empty() {
        return Err(
            "The worktree is not on a branch (detached HEAD); check out a branch first".into(),
        );
    }
    let dirty = read_git(&tree, &["status", "--porcelain", "--untracked-files=no"]).await?;
    if !dirty.trim().is_empty() {
        return Err(format!(
            "Commit or discard the uncommitted changes first: Update from {base} moves commits only"
        ));
    }
    if let Some(branch) = base.strip_prefix("origin/") {
        let args = ["fetch", "--quiet", "origin", branch];
        let out = git_raw(&tree, &args, SLOW, 64 * 1024).await?;
        if out.code != Some(0) {
            return Err(format!("Fetch failed: {}", redacted_error(&args, &out)));
        }
    }
    // A pushed branch merges the base in, so its commits aren't rewritten
    // and Push stays a fast-forward (review); an unpushed one rebases.
    // "Pushed" includes `origin/<branch>` without an upstream (an agent's
    // own `git push` without -u), and a stale one whose remote branch was
    // deleted unpruned: merging is the safe side either way.
    let branch = current_branch(&tree).await;
    let pushed = upstream_of(&tree, base).await.is_some()
        || read_git(
            &tree,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/remotes/origin/{branch}"),
            ],
        )
        .await
        .is_ok();
    let before = read_git(&tree, &["rev-parse", "HEAD"]).await?;
    let (args, how) = if pushed {
        (["merge", "--no-edit", base], "merge")
    } else {
        (["rebase", "--quiet", base], "rebase")
    };
    let out = git_raw(&tree, &args, SLOW, 256 * 1024).await;
    let failed = match &out {
        Ok(out) if out.code == Some(0) => None,
        Ok(out) => Some((
            redacted_error(&args, out),
            format!("{}{}", String::from_utf8_lossy(&out.stdout), out.stderr),
        )),
        Err(e) => Some((e.clone(), String::new())),
    };
    if let Some((why, said)) = failed {
        let aborted = git_raw(&tree, &[how, "--abort"], SLOW, 64 * 1024)
            .await
            .is_ok_and(|o| o.code == Some(0));
        // "Nothing changed" only when it holds: no rebase or merge left, HEAD
        // where it was and no tracked change (delta review: a timeout can
        // stop git before its own markers exist).
        let undone = (aborted || !in_progress(&tree, how).await)
            && read_git(&tree, &["rev-parse", "HEAD"]).await.ok() == Some(before.clone())
            && read_git(&tree, &["status", "--porcelain", "--untracked-files=no"])
                .await
                .is_ok_and(|s| s.trim().is_empty());
        changed(vault, id);
        if !undone {
            return Err(format!("Update from {base} failed ({why}), and the {how} couldn't be undone cleanly: check the worktree with git status (git {how} --abort, or ask the agent)."));
        }
        return Err(
            if said.contains("CONFLICT") || said.contains("could not apply") {
                format!("Update from {base} stopped on a conflict, so nothing changed ({why}). Ask the agent to {how} and resolve it.")
            } else {
                format!("Update from {base} failed, so nothing changed: {why}")
            },
        );
    }
    changed(vault, id);
    Ok(())
}

/// Where Update from base fetches from, for its card: workspace `id`'s
/// `origin` as `host/owner/name`, with any credential, port, query and
/// `.git` dropped; `None` when it has none (a local base fetches nothing).
pub(crate) async fn origin_for_card(vault: &Path, id: &str) -> Option<String> {
    let info = live_binding(vault, id).ok()?;
    let url = read_git(Path::new(&info.worktree), &["remote", "get-url", "origin"])
        .await
        .ok()?;
    origin_shown(&url)
}

/// A remote URL as `host/owner/name` (see [`origin_for_card`]).
pub(crate) fn origin_shown(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = if let Some((_, r)) = url.split_once("://") {
        r.rsplit_once('@').map(|(_, h)| h).unwrap_or(r).to_string()
    } else if let Some((user_host, path)) = url.split_once(':') {
        let host = user_host
            .rsplit_once('@')
            .map(|(_, h)| h)
            .unwrap_or(user_host);
        format!("{host}/{path}")
    } else {
        return None;
    };
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (host, path) = rest.split_once('/')?;
    let host = host.split(':').next().unwrap_or(host);
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    (!host.is_empty() && !path.is_empty()).then(|| format!("{host}/{path}"))
}

/// Whether a `how` (`rebase` or `merge`) is still in progress in `tree`.
async fn in_progress(tree: &Path, how: &str) -> bool {
    let marks: &[&str] = if how == "merge" {
        &["MERGE_HEAD"]
    } else {
        &["rebase-merge", "rebase-apply"]
    };
    for mark in marks {
        if let Ok(path) = read_git(tree, &["rev-parse", "--git-path", mark]).await {
            if tree.join(path.trim()).exists() {
                return true;
            }
        }
    }
    false
}

/// `origin/main` → `main`; a local base stays as it is.
pub(crate) fn pr_base(base: &str) -> String {
    base.strip_prefix("origin/").unwrap_or(base).to_string()
}

/// Open a pull request for the worktree's branch (pushing it first when it
/// has no upstream); returns its URL.
pub async fn open_pr(vault: &Path, id: &str, title: &str, body: &str) -> Result<String, String> {
    let gh = gh_binary().ok_or("The GitHub CLI (gh) is not installed; install it with `brew install gh`, then `gh auth login`")?;
    open_pr_with(vault, id, title, body, &gh).await
}

/// [`open_pr`] for a Hollow app caller's run (PR A5): the same gh, and the
/// busy check exempts the calling run.
pub(crate) async fn open_pr_as(
    vault: &Path,
    id: &str,
    title: &str,
    body: &str,
    caller: &str,
) -> Result<String, String> {
    let gh = gh_binary().ok_or("The GitHub CLI (gh) is not installed; install it with `brew install gh`, then `gh auth login`")?;
    open_pr_for(vault, id, title, body, &gh, Some(caller)).await
}

pub(crate) async fn open_pr_with(
    vault: &Path,
    id: &str,
    title: &str,
    body: &str,
    gh: &Path,
) -> Result<String, String> {
    open_pr_for(vault, id, title, body, gh, None).await
}

/// [`open_pr_with`], also allowed while `caller` is the run holding the vault.
pub(crate) async fn open_pr_for(
    vault: &Path,
    id: &str,
    title: &str,
    body: &str,
    gh: &Path,
    caller: Option<&str>,
) -> Result<String, String> {
    let info = live_binding(vault, id)?;
    let title = title.trim();
    if title.is_empty() {
        return Err("Give the pull request a title".into());
    }
    refuse_from_another_worktree(vault, id, caller)?;
    refuse_while_busy(vault, id, "open a pull request", caller)?;
    let tree = PathBuf::from(&info.worktree);
    let branch = current_branch(&tree).await;
    if branch.is_empty() {
        return Err(
            "The worktree is not on a branch (detached HEAD); check out a branch first".into(),
        );
    }
    if upstream_of(&tree, &info.base).await.is_none() {
        push_branch(&tree).await?;
    }
    let base = pr_base(&info.base);
    let mut cmd = gh_command(gh, &tree);
    cmd.args([
        "pr", "create", "--base", &base, "--head", &branch, "--title", title, "--body", body,
    ]);
    let out = crate::process_group::run_to_completion(&mut cmd, None, SLOW, None).await;
    // Pushed, opened, or neither: the rail reads this branch again.
    changed(vault, id);
    let out = out.map_err(|e| format!("could not run gh: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.timed_out || !out.status.is_some_and(|s| s.success()) {
        let detail = if out.stderr.trim().is_empty() {
            stdout.trim()
        } else {
            out.stderr.trim()
        };
        let last = detail.lines().last().unwrap_or("gh pr create failed");
        return Err(crate::claude_runner::redact(&format!(
            "Could not open the pull request: {last}"
        )));
    }
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("https://") || l.starts_with("http://"))
        .map(str::to_string)
        .ok_or_else(|| "gh created no pull request URL".into())
}

// ------------------------------------------------------------------ gh

/// Domain-level notification also covers mutations requested by agents.
fn changed(vault: &Path, id: &str) {
    forget_rail_state(vault, id);
    if let Some(app) = crate::notifications::registered_app() {
        use tauri::Emitter;
        let _ = app.emit("build-changed", id);
    }
}

/// The GitHub CLI, if installed.
pub(crate) fn gh_binary() -> Option<PathBuf> {
    // Tests never reach the developer's GitHub account; they pass a fake gh.
    if cfg!(test) {
        return None;
    }
    for fixed in ["/opt/homebrew/bin/gh", "/usr/local/bin/gh"] {
        let p = PathBuf::from(fixed);
        if p.is_file() {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("gh"))
        .find(|p| p.is_file())
}

pub(crate) fn gh_command(gh: &Path, dir: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(gh);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    cmd.current_dir(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("NO_COLOR", "1")
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// The PR for `branch`, or `None` (no PR, gh missing, not signed in, offline).
async fn pr_for(gh: &Path, tree: &Path, branch: &str) -> Option<PullRequest> {
    let mut cmd = gh_command(gh, tree);
    cmd.args(["pr", "view", branch, "--json", "url,state,number"]);
    let out =
        crate::process_group::run_to_completion(&mut cmd, None, Duration::from_secs(15), None)
            .await
            .ok()?;
    if out.timed_out || !out.status.is_some_and(|s| s.success()) {
        return None;
    }
    parse_pr(&String::from_utf8_lossy(&out.stdout))
}

pub(crate) fn parse_pr(json: &str) -> Option<PullRequest> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    Some(PullRequest {
        url: v.get("url")?.as_str()?.to_string(),
        state: v
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("open")
            .to_lowercase(),
        number: v.get("number")?.as_u64()?,
    })
}

/// A verified PR binding chosen for one conversation. Never guessed from its text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettlementPr {
    pub workspace_id: String,
    pub repo: String,
    pub worktree: String,
    pub remote: String,
    pub head: String,
    pub branch: String,
    pub url: String,
    pub number: u64,
    pub state: String,
}

fn settlement_repo(remote: &str) -> Option<String> {
    let value = remote.trim();
    let (host, path) = if let Some(scp) = value.strip_prefix("git@") {
        scp.split_once(':')?
    } else {
        value
            .strip_prefix("https://")
            .or_else(|| value.strip_prefix("ssh://git@"))?
            .split_once('/')?
    };
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<_> = path.split('/').collect();
    let safe = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    // Reject credentials, query strings, ports and ambiguous paths. Persist
    // only repository identity, never a credential-bearing remote URL.
    if !safe(host) || parts.len() != 2 || !parts.iter().all(|s| safe(s)) {
        return None;
    }
    Some(format!("{}/{path}", host.to_ascii_lowercase()))
}

pub async fn settlement_pr(
    vault: &Path,
    id: &str,
    expected: Option<&SettlementPr>,
) -> Option<SettlementPr> {
    let info = live_binding(vault, id).ok()?;
    let tree = Path::new(&info.worktree);
    let branch = current_branch(tree).await;
    if branch.is_empty() {
        return None;
    }
    let remote = settlement_repo(
        &read_git(tree, &["remote", "get-url", "origin"])
            .await
            .ok()?,
    )?;
    let head = read_git(tree, &["rev-parse", "HEAD"])
        .await
        .ok()?
        .trim()
        .to_owned();
    if let Some(old) = expected {
        if old.repo != info.repo
            || old.worktree != info.worktree
            || old.branch != branch
            || old.remote != remote
            || old.head != head
        {
            return None;
        }
    }
    let gh = gh_binary()?;
    let mut cmd = gh_command(&gh, tree);
    let target = expected.map(|p| p.url.as_str()).unwrap_or(&branch);
    cmd.args([
        "pr",
        "view",
        target,
        "--repo",
        &remote,
        "--json",
        "url,state,number,headRefName,headRefOid,isCrossRepository",
    ]);
    let out =
        crate::process_group::run_to_completion(&mut cmd, None, Duration::from_secs(15), None)
            .await
            .ok()?;
    if out.timed_out || !out.status.is_some_and(|s| s.success()) {
        return None;
    }
    let json = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    let pr = parse_pr(&json)?;
    if !pr
        .url
        .eq_ignore_ascii_case(&format!("https://{remote}/pull/{}", pr.number))
    {
        return None;
    }
    // Do not infer a same-named fork branch or an unpushed local branch.
    if value.get("isCrossRepository")?.as_bool()? || value.get("headRefOid")?.as_str()? != head {
        return None;
    }
    if value.get("headRefName")?.as_str()? != branch
        || expected.is_some_and(|old| old.url != pr.url || old.number != pr.number)
    {
        return None;
    }
    Some(SettlementPr {
        workspace_id: id.into(),
        repo: info.repo,
        worktree: info.worktree,
        remote,
        head,
        branch,
        url: pr.url,
        number: pr.number,
        state: pr.state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_workspaces::tests::{fixture, sh};
    use crate::build_workspaces::{create, CreateInput};

    #[test]
    fn settlement_repo_is_explicit_and_never_persists_credentials() {
        for remote in [
            "git@github.com:owner/repo.git",
            "https://github.com/owner/repo.git",
            "ssh://git@github.com/owner/repo",
        ] {
            assert_eq!(
                settlement_repo(remote).as_deref(),
                Some("github.com/owner/repo")
            );
        }
        for remote in [
            "https://token@github.com/owner/repo",
            "https://github.com/owner/repo?token=x",
            "file:///repo",
            "https://github.com/owner/../repo",
            "git@github.com:owner/repo/extra",
        ] {
            assert!(settlement_repo(remote).is_none(), "{remote}");
        }
    }

    /// Integration of B2 with B3: a build workspace's own turn holds no vault
    /// lease, so commit, push and Open PR check its scoped run too.
    #[tokio::test]
    async fn git_actions_wait_for_the_workspaces_own_turn() {
        let vault = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let tree = tempfile::Builder::new()
            .prefix("worktree-")
            .tempdir()
            .unwrap();
        let workspace = crate::workspaces::create(vault.path(), "Scoped git", "Ship it").unwrap();
        crate::build_workspaces::bind_for_test(vault.path(), &workspace.id, tree.path());
        let lease =
            crate::automated_history::try_begin_scoped(vault.path(), &workspace.path, "chat", None)
                .unwrap();
        let refused = commit(vault.path(), &workspace.id, "wip")
            .await
            .unwrap_err();
        assert!(refused.contains("agent turn to finish"), "{refused}");
        let refused = push(vault.path(), &workspace.id).await.unwrap_err();
        assert!(refused.contains("agent turn to finish"), "{refused}");
        lease.finish("success").unwrap();
    }

    #[test]
    fn numstat_and_name_status_parse_renames_and_binaries() {
        let raw = "3\t1\tsrc/a.rs\0-\t-\timg.png\x000\t0\t\0old name.txt\0new name.txt\0";
        let rows = parse_numstat_z(raw);
        assert_eq!(rows[0], ("src/a.rs".into(), None, Some(3), Some(1)));
        assert_eq!(rows[1], ("img.png".into(), None, None, None));
        assert_eq!(
            rows[2],
            (
                "new name.txt".into(),
                Some("old name.txt".into()),
                Some(0),
                Some(0)
            )
        );
        let names = parse_name_status_z(
            "M\0src/a.rs\0R087\0old name.txt\0new name.txt\0D\0gone\0A\0added\0",
        );
        assert_eq!(names[0], ("src/a.rs".into(), 'M', None));
        assert_eq!(
            names[1],
            ("new name.txt".into(), 'R', Some("old name.txt".into()))
        );
        assert_eq!(names[2], ("gone".into(), 'D', None));
        assert_eq!(names[3], ("added".into(), 'A', None));
    }

    #[test]
    fn paths_must_stay_inside_the_worktree() {
        assert_eq!(safe_relative("src/a.rs").unwrap(), "src/a.rs");
        assert_eq!(safe_relative("./a b.txt").unwrap(), "./a b.txt");
        for bad in ["", "/etc/passwd", "../x", "src/../../x", "a\0b"] {
            assert!(safe_relative(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn diff_text_is_capped_per_file_and_redacted() {
        let big = format!(
            "diff --git a/big b/big\n--- a/big\n+++ b/big\n@@ -0,0 +1 @@\n{}",
            "+x\n".repeat(200_000)
        );
        let small = "diff --git a/s b/s\n--- a/s\n+++ b/s\n@@ -1 +1 @@\n-a\n+token sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789\n".to_string();
        let out = assemble(vec![format!("{big}{small}")]);
        assert_eq!(out.truncated_files, vec!["big".to_string()]);
        assert!(!out.truncated);
        assert!(out.diff.len() <= DIFF_FILE_MAX + small.len() + 1);
        assert!(out.diff.contains("diff --git a/s b/s"));
        assert!(!out.diff.contains("sk-ant-api03-abcdef"), "redacted");
        // Past the total cap, whole files are dropped.
        let many: Vec<String> = (0..6)
            .map(|n| format!("diff --git a/f{n} b/f{n}\n{}", "+y\n".repeat(80_000)))
            .collect();
        let out = assemble(many);
        assert!(out.truncated);
        assert!(out.diff.len() <= DIFF_TOTAL_MAX);
    }

    #[test]
    fn pr_json_and_base_names() {
        let pr = parse_pr(r#"{"number":12,"state":"OPEN","url":"https://github.com/o/r/pull/12"}"#)
            .unwrap();
        assert_eq!(
            pr,
            PullRequest {
                url: "https://github.com/o/r/pull/12".into(),
                state: "open".into(),
                number: 12
            }
        );
        assert!(parse_pr("no pull requests found").is_none());
        assert_eq!(pr_base("origin/main"), "main");
        assert_eq!(pr_base("release/1.0"), "release/1.0");
    }

    /// A build workspace on a fixture repo with a bare `origin`.
    async fn build_workspace() -> (tempfile::TempDir, PathBuf, PathBuf, String, PathBuf) {
        let (tmp, repo, vault, roots) = fixture();
        let origin = tmp.path().join("origin.git");
        sh(
            tmp.path(),
            &["init", "-q", "--bare", origin.to_str().unwrap()],
        );
        sh(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh(&repo, &["push", "-q", "origin", "main"]);
        sh(&repo, &["config", "user.name", "t"]);
        sh(&repo, &["config", "user.email", "t@example.com"]);
        let made = create(
            &vault,
            &roots,
            CreateInput {
                title: "Change things".into(),
                goal: "Make the README better.".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap();
        let id = made.workspace.id.clone();
        let tree = PathBuf::from(build_workspaces::binding_for(&vault, &id).unwrap().worktree);
        (tmp, vault, tree, id, origin)
    }

    /// Overnight Y7: the rail's cached state follows the worktree's refs, so
    /// an agent's own commit shows on the next refresh, not 2 min later;
    /// `fresh` ids and Hollow's own commits skip the cache.
    #[tokio::test]
    async fn rail_states_follow_commits_and_fresh_asks() {
        let (_tmp, vault, tree, id, _origin) = build_workspace().await;
        let ids = [id.clone()];
        let first = rail_states(&vault, &ids, &[]).await;
        assert_eq!((first.len(), first[0].ahead), (1, 0));
        assert_eq!(first[0].branch, "hollow/change-things");
        // A commit made outside Hollow (an agent's shell) moves the stamp.
        std::fs::write(tree.join("README.md"), "hi\nthere\n").unwrap();
        sh(&tree, &["commit", "-q", "-am", "Say more"]);
        assert_eq!(rail_states(&vault, &ids, &[]).await[0].ahead, 1);
        // While the stamp holds the cache answers; `fresh` reads git again.
        let key = (vault.clone(), id.clone());
        let mut stale = first[0].clone();
        stale.pr_number = Some(99);
        RAIL_CACHE.lock().unwrap().get_mut(&key).unwrap().2 = stale;
        assert_eq!(rail_states(&vault, &ids, &[]).await[0].pr_number, Some(99));
        let fresh = rail_states(&vault, &ids, &ids).await;
        assert_eq!((fresh[0].pr_number, fresh[0].ahead), (None, 1));
        // Hollow's own commit drops the entry outright.
        RAIL_CACHE
            .lock()
            .unwrap()
            .get_mut(&key)
            .unwrap()
            .2
            .pr_number = Some(99);
        std::fs::write(tree.join("README.md"), "hi\nthere\nagain\n").unwrap();
        let generation = rail_generation(&key);
        commit(&vault, &id, "Again").await.unwrap();
        assert!(!RAIL_CACHE.lock().unwrap().contains_key(&key));
        assert_eq!(
            rail_generation(&key),
            generation + 1,
            "a read begun before the commit can't be kept"
        );
        let after = rail_states(&vault, &ids, &[]).await;
        assert_eq!((after[0].pr_number, after[0].ahead), (None, 2));
    }

    #[test]
    fn git_stamp_moves_with_head_and_the_base_ref_only() {
        let (_tmp, repo, _vault, _roots) = fixture();
        let tree = repo.parent().unwrap().join("linked");
        sh(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                tree.to_str().unwrap(),
            ],
        );
        let base = "main";
        let s0 = git_stamp(&tree, base);
        assert!(s0.iter().take(2).all(Option::is_some), "{s0:?}");
        assert_eq!(git_stamp(&tree, base), s0, "nothing changed");
        std::fs::write(tree.join("notes.txt"), "x").unwrap();
        assert_eq!(
            git_stamp(&tree, base),
            s0,
            "an uncommitted file is not a ref"
        );
        sh(&tree, &["add", "."]);
        sh(&tree, &["commit", "-q", "-m", "side work"]);
        let s1 = git_stamp(&tree, base);
        assert_ne!(s1, s0, "a commit in the worktree");
        std::fs::write(repo.join("README.md"), "base moved\n").unwrap();
        sh(&repo, &["commit", "-q", "-am", "base"]);
        assert_ne!(git_stamp(&tree, base), s1, "the base branch moved");
        let none = git_stamp(&tree, "").len();
        assert_eq!(
            git_stamp(&tree, "../escape").len(),
            none,
            "no path outside refs"
        );
        assert_eq!(
            git_stamp(&tree, "refs/heads/main").len(),
            none + 1,
            "a full ref"
        );
    }

    /// P1-46 (BUILD-07): an installed commit "contains this branch" when the
    /// branch's head is its ancestor; an unknown commit is unknown.
    #[tokio::test]
    async fn an_install_contains_the_branch_only_when_head_is_its_ancestor() {
        let (_tmp, _vault, tree, _id, _origin) = build_workspace().await;
        std::fs::write(tree.join("README.md"), "hi\nthere\n").unwrap();
        sh(&tree, &["commit", "-q", "-am", "Say more"]);
        let head = read_git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
        let head = head.trim();
        std::fs::write(tree.join("README.md"), "hi\nthere\nlater\n").unwrap();
        sh(&tree, &["commit", "-q", "-am", "Later"]);
        let later = read_git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
        sh(&tree, &["reset", "-q", "--hard", head]);
        assert_eq!(contains_commit(&tree, later.trim()).await, Some(true));
        assert_eq!(contains_commit(&tree, &head[..12]).await, Some(true));
        let base = read_git(&tree, &["rev-parse", "origin/main"])
            .await
            .unwrap();
        assert_eq!(contains_commit(&tree, base.trim()).await, Some(false));
        assert_eq!(contains_commit(&tree, "0123456789ab").await, None);
        assert_eq!(contains_commit(&tree, "--all").await, None);
    }

    /// P1-46 (BUILD-11): Update from <base> fetches and rebases; a conflict
    /// aborts and leaves the branch as it was; an agent turn there refuses.
    #[tokio::test]
    async fn update_from_base_rebases_and_aborts_on_conflict() {
        let (tmp, vault, tree, id, origin) = build_workspace().await;
        let other = tmp.path().join("other");
        sh(
            tmp.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        std::fs::write(other.join("NEWS.md"), "news\n").unwrap();
        sh(&other, &["add", "."]);
        sh(&other, &["commit", "-q", "-m", "News"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        std::fs::write(tree.join("README.md"), "hi\nmine\n").unwrap();
        sh(&tree, &["commit", "-q", "-am", "Mine"]);
        let run = crate::automated_history::try_begin_scoped(
            &vault,
            &format!("workspaces/{id}"),
            "chat",
            None,
        )
        .unwrap();
        let refused = update_from_base(&vault, &id).await.unwrap_err();
        assert!(refused.contains("agent turn to finish"), "{refused}");
        run.finish("success").unwrap();
        let updated = update_from_base(&vault, &id).await.unwrap();
        assert_eq!((updated.behind, updated.ahead), (0, 1));
        assert!(tree.join("NEWS.md").is_file());
        // The base changes the same line: the rebase stops and is undone.
        std::fs::write(other.join("README.md"), "hi\ntheirs\n").unwrap();
        sh(&other, &["commit", "-q", "-am", "Theirs"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        let before = read_git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
        let why = update_from_base(&vault, &id).await.unwrap_err();
        assert!(why.contains("conflict"), "{why}");
        assert_eq!(
            read_git(&tree, &["rev-parse", "HEAD"]).await.unwrap(),
            before
        );
        assert_eq!(
            read_git(&tree, &["status", "--porcelain"]).await.unwrap(),
            ""
        );
        let rebasing = read_git(&tree, &["rev-parse", "--git-path", "rebase-merge"])
            .await
            .unwrap();
        assert!(!tree.join(rebasing.trim()).exists());
        // Uncommitted changes are refused before anything is fetched.
        std::fs::write(tree.join("README.md"), "dirty\n").unwrap();
        let dirty = update_from_base(&vault, &id).await.unwrap_err();
        assert!(dirty.contains("uncommitted"), "{dirty}");
    }

    /// L13 review: the COO's Update from base reserves the workspace's
    /// folder through completion (an agent turn there refuses it, and the
    /// reservation is gone afterwards, however it ended), and its vault-wide
    /// check exempts only the caller's own lease, never another run's.
    #[tokio::test]
    async fn an_app_callers_update_reserves_the_workspace_and_exempts_only_its_own_lease() {
        let (tmp, vault, tree, id, origin) = build_workspace().await;
        let scope = format!("workspaces/{id}");
        let other = tmp.path().join("other");
        sh(
            tmp.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        std::fs::write(other.join("NEWS.md"), "news\n").unwrap();
        sh(&other, &["add", "."]);
        sh(&other, &["commit", "-q", "-m", "News"]);
        sh(&other, &["push", "-q", "origin", "main"]);

        // An agent turn in the workspace refuses it.
        let turn =
            crate::automated_history::try_begin_scoped(&vault, &scope, "chat", None).unwrap();
        let refused = update_from_base_for(&vault, &id, "run-x")
            .await
            .unwrap_err();
        assert!(refused.contains("agent turn to finish"), "{refused}");
        turn.finish("success").unwrap();

        // Another run holding the vault refuses it; the reservation it took
        // first is released at once.
        let holder = crate::automated_history::try_begin_owned(&vault, "chat", None).unwrap();
        let refused = update_from_base_for(&vault, &id, "run-x")
            .await
            .unwrap_err();
        assert!(refused.contains("Wait for"), "{refused}");
        assert!(crate::automated_history::scope_holder(&vault, &scope).is_none());
        // ...but not when the caller is that run.
        let updated = update_from_base_for(&vault, &id, holder.id())
            .await
            .unwrap();
        assert_eq!(updated.behind, 0);
        assert!(tree.join("NEWS.md").is_file());
        holder.finish("success").unwrap();

        // Released after a success too, so the next turn can start.
        assert!(crate::automated_history::scope_holder(&vault, &scope).is_none());
        let after = status(&vault, &id).await.unwrap();
        assert!(after.blocked.is_none(), "{:?}", after.blocked);
    }

    #[test]
    fn the_update_card_shows_origin_without_its_credentials() {
        assert_eq!(
            origin_shown("https://x-access-token:SECRET-PAT@github.com/o/r.git").as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(
            origin_shown("git@github.com:o/r.git").as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(
            origin_shown("https://github.com:443/o/r?token=SECRET-PAT").as_deref(),
            Some("github.com/o/r")
        );
        assert_eq!(origin_shown("/local/path/repo"), None);
    }

    /// Review of P1-46: a pushed branch merges the base in rather than
    /// rebasing, so the next Push is a fast-forward.
    #[tokio::test]
    async fn update_from_base_merges_a_pushed_branch_so_push_still_works() {
        let (tmp, vault, tree, id, origin) = build_workspace().await;
        std::fs::write(tree.join("README.md"), "hi\nmine\n").unwrap();
        sh(&tree, &["commit", "-q", "-am", "Mine"]);
        push(&vault, &id).await.unwrap();
        let other = tmp.path().join("other");
        sh(
            tmp.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                other.to_str().unwrap(),
            ],
        );
        std::fs::write(other.join("NEWS.md"), "news\n").unwrap();
        sh(&other, &["add", "."]);
        sh(&other, &["commit", "-q", "-m", "News"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        let mine = read_git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
        let updated = update_from_base(&vault, &id).await.unwrap();
        assert_eq!(updated.behind, 0);
        assert!(tree.join("NEWS.md").is_file());
        let parents = read_git(&tree, &["rev-list", "--parents", "-n1", "HEAD"])
            .await
            .unwrap();
        assert!(
            parents.contains(mine.trim()),
            "a merge keeps {mine}: {parents}"
        );
        assert_eq!(
            updated.unpushed, 2,
            "Mine is already pushed: the merge and News"
        );
        let pushed = push(&vault, &id).await.unwrap();
        assert_eq!(pushed.unpushed, 0);
        // An agent's own `git push` without -u leaves no upstream; the branch
        // on origin still counts as pushed, so it merges too (delta review).
        sh(&tree, &["branch", "--unset-upstream"]);
        std::fs::write(other.join("MORE.md"), "more\n").unwrap();
        sh(&other, &["add", "."]);
        sh(&other, &["commit", "-q", "-m", "More"]);
        sh(&other, &["push", "-q", "origin", "main"]);
        let merged = read_git(&tree, &["rev-parse", "HEAD"]).await.unwrap();
        assert_eq!(update_from_base(&vault, &id).await.unwrap().behind, 0);
        let parents = read_git(&tree, &["rev-list", "--parents", "-n1", "HEAD"])
            .await
            .unwrap();
        assert!(parents.contains(merged.trim()), "merged again: {parents}");
        push(&vault, &id).await.unwrap();
    }

    #[tokio::test]
    async fn status_exposes_background_scope_guard_and_clears_after_finish() {
        let (_tmp, vault, _tree, id, _origin) = build_workspace().await;
        let run = crate::automated_history::try_begin_scoped(
            &vault,
            &format!("workspaces/{id}"),
            "chat",
            None,
        )
        .unwrap();
        let status = status_with(&vault, &id, None).await.unwrap();
        assert_eq!(
            status.blocked,
            refuse_while_busy(&vault, &id, "commit or publish", None).err()
        );
        assert!(status.blocked.unwrap().contains("Wait for"));
        assert!(crate::self_install::status(&vault, &id).blocked.is_some());
        run.finish("success").unwrap();
        assert!(status_with(&vault, &id, None)
            .await
            .unwrap()
            .blocked
            .is_none());
        assert!(crate::self_install::status(&vault, &id).blocked.is_none());
    }

    #[tokio::test]
    async fn status_lists_committed_uncommitted_and_untracked_changes() {
        let (_tmp, vault, tree, id, _origin) = build_workspace().await;
        let clean = status_with(&vault, &id, None).await.unwrap();
        assert_eq!(clean.base, "origin/main");
        assert_eq!(clean.branch, "hollow/change-things");
        assert_eq!((clean.ahead, clean.behind), (0, 0));
        assert!(clean.files.is_empty() && clean.commits.is_empty());
        assert!(clean.upstream.is_none() && clean.pr.is_none());
        assert_eq!(clean.goal, "Make the README better.");

        std::fs::write(tree.join("README.md"), "hi\nthere\n").unwrap();
        sh(&tree, &["commit", "-q", "-am", "Say more"]);
        std::fs::write(tree.join("README.md"), "hello\nthere\n").unwrap();
        std::fs::write(tree.join("new file.txt"), "one\ntwo\nthree").unwrap();
        std::fs::write(tree.join("blob.bin"), [0u8, 1, 2]).unwrap();
        let s = status_with(&vault, &id, None).await.unwrap();
        assert_eq!(s.ahead, 1);
        assert_eq!(s.commits.len(), 1);
        assert_eq!(s.commits[0].subject, "Say more");
        let readme = s.files.iter().find(|f| f.path == "README.md").unwrap();
        assert_eq!(readme.status, FileStatus::Modified);
        assert_eq!((readme.additions, readme.deletions), (Some(2), Some(1)));
        assert!(readme.uncommitted && !readme.staged);
        let new = s.files.iter().find(|f| f.path == "new file.txt").unwrap();
        assert_eq!(new.status, FileStatus::Untracked);
        assert_eq!(new.additions, Some(3));
        let blob = s.files.iter().find(|f| f.path == "blob.bin").unwrap();
        assert_eq!(blob.additions, None);

        let d = diff(&vault, &id, None).await.unwrap();
        assert!(
            d.diff.contains("diff --git a/README.md b/README.md"),
            "{}",
            d.diff
        );
        assert!(d.diff.contains("+hello"));
        assert!(d.diff.contains("-hi"));
        assert!(d.diff.contains("+++ b/new file.txt"));
        assert!(d.diff.contains("\\ No newline at end of file"));
        let one = diff(&vault, &id, Some("new file.txt")).await.unwrap();
        assert!(one.diff.contains("+three") && !one.diff.contains("README"));
        assert!(diff(&vault, &id, Some("../x")).await.is_err());
    }

    #[tokio::test]
    async fn commit_push_and_open_pr() {
        let (tmp, vault, tree, id, origin) = build_workspace().await;
        assert!(commit(&vault, &id, "  ")
            .await
            .unwrap_err()
            .contains("commit message"));
        assert!(commit(&vault, &id, "Nothing")
            .await
            .unwrap_err()
            .contains("Nothing to commit"));
        std::fs::write(tree.join("added.txt"), "x\n").unwrap();
        let s = commit(&vault, &id, "Add a file").await.unwrap();
        assert!(s.files.iter().all(|f| !f.uncommitted), "{:?}", s.files);
        assert_eq!(s.ahead, 1);
        assert!(s.upstream.is_none());

        // A fake gh that records its arguments and prints a PR URL.
        let log = tmp.path().join("gh-args.txt");
        let gh = tmp.path().join("gh");
        std::fs::write(&gh, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho 'Creating pull request...'\necho https://github.com/o/r/pull/7\n", log.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(open_pr_with(&vault, &id, " ", "b", &gh)
            .await
            .unwrap_err()
            .contains("title"));
        let url = open_pr_with(&vault, &id, "Add a file", "Body text", &gh)
            .await
            .unwrap();
        assert_eq!(url, "https://github.com/o/r/pull/7");
        let args = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            [
                "pr",
                "create",
                "--base",
                "main",
                "--head",
                "hollow/change-things",
                "--title",
                "Add a file",
                "--body",
                "Body text"
            ]
        );
        // Opening the PR pushed the branch first.
        sh(
            &origin,
            &["rev-parse", "--verify", "refs/heads/hollow/change-things"],
        );
        let s = status_with(&vault, &id, None).await.unwrap();
        assert_eq!(s.upstream.as_deref(), Some("origin/hollow/change-things"));
        assert_eq!(s.unpushed, 0);

        std::fs::write(tree.join("added.txt"), "y\n").unwrap();
        commit(&vault, &id, "Change it").await.unwrap();
        let s = status_with(&vault, &id, None).await.unwrap();
        assert_eq!(s.unpushed, 1);
        let s = push(&vault, &id).await.unwrap();
        assert_eq!(s.unpushed, 0);
    }

    #[tokio::test]
    async fn a_removed_or_unknown_worktree_is_refused() {
        let (_tmp, vault, _tree, id, _origin) = build_workspace().await;
        assert!(status_with(&vault, "nope", None)
            .await
            .unwrap_err()
            .contains("no worktree"));
        build_workspaces::remove(&vault, &id, true).await.unwrap();
        assert!(status_with(&vault, &id, None)
            .await
            .unwrap_err()
            .contains("removed"));
        assert!(commit(&vault, &id, "x")
            .await
            .unwrap_err()
            .contains("removed"));
    }
}
