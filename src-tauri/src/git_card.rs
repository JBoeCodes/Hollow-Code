//! Git for the chat card, for any folder (t3code parity 5.2: D1, D4, D5).
//!
//! The Changes pane (`build_changes.rs`) is keyed to a build workspace. The
//! card is keyed to a chat's folder, so a plain git checkout gets the same
//! branch picker, worktree mode and Commit and push without a build binding.
//! Everything runs the person's own git through `build_workspaces::git_raw`
//! (`GIT_*` stripped, no prompt, process group, capped output). Reads pass
//! `--no-optional-locks`, so refreshing never takes `index.lock` from under an
//! agent using git in the same folder. Mutations (switch, new worktree, commit,
//! push) refuse while any Hollow agent run holds the vault, as the build
//! pane's do. These are the person's own clicks: plumbing, never agent actions.
//!
//! The generated commit message is deterministic and local (file names and
//! counts), so it works with any agent or none and sends nothing anywhere; the
//! person edits it in the confirm dialog before it is used.

use crate::build_workspaces::{self, git_error, git_raw, GitOutput, QUICK};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SLOW: Duration = Duration::from_secs(120);
/// Most changed files the card lists (the rest are counted in `more_files`).
const FILES_MAX: usize = 200;
/// Most branches the picker lists.
const BRANCHES_MAX: usize = 100;
/// Subject line cap for a generated message.
const SUBJECT_MAX: usize = 72;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CardFile {
    pub path: String,
    /// added, modified, deleted, renamed or untracked.
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitCardStatus {
    /// False when the folder is not inside a git work tree (the rest is empty).
    pub repo: bool,
    pub root: String,
    /// Current branch; empty on a detached HEAD.
    pub branch: String,
    /// HEAD's short sha (empty before the first commit).
    pub head: String,
    pub upstream: Option<String>,
    /// Commits not on the upstream yet (without one: commits on no remote).
    pub ahead: u64,
    /// Commits the upstream has that HEAD lacks (0 without an upstream).
    pub behind: u64,
    /// This folder is a linked worktree, not the main checkout.
    pub linked_worktree: bool,
    pub files: Vec<CardFile>,
    pub more_files: usize,
    pub additions: u64,
    pub deletions: u64,
    /// Why a git change can't run now (an agent holds the vault); None when free.
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitCardBranches {
    pub current: String,
    pub branches: Vec<String>,
}

fn dir_of(cwd: &str) -> Result<PathBuf, String> {
    let p = Path::new(cwd);
    if !p.is_absolute() {
        return Err("The folder must be an absolute path".into());
    }
    if !p.is_dir() {
        return Err(format!("{cwd} is not a folder"));
    }
    Ok(p.to_path_buf())
}

async fn read_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["--no-optional-locks"];
    full.extend_from_slice(args);
    let out = git_raw(dir, &full, QUICK, 16 * 1024 * 1024).await?;
    if out.code != Some(0) {
        return Err(git_error(args, &out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn failure(what: &str, args: &[&str], out: &GitOutput) -> String {
    format!(
        "{what}: {}",
        crate::build_changes::redacted_error(args, out)
    )
}

/// `git status --porcelain=v1 -z` records: `(path, status word)`. A rename has
/// its old path after the new one, which is skipped.
pub(crate) fn parse_porcelain_z(raw: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0');
    while let Some(rec) = parts.next() {
        if rec.len() < 4 {
            continue;
        }
        let (x, y) = (rec.as_bytes()[0] as char, rec.as_bytes()[1] as char);
        let path = rec[3..].to_string();
        if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
            let _old = parts.next();
        }
        let word = if x == '?' {
            "untracked"
        } else {
            match if y != ' ' { y } else { x } {
                'A' => "added",
                'D' => "deleted",
                'R' | 'C' => "renamed",
                _ => "modified",
            }
        };
        out.push((path, word));
    }
    out
}

/// The card's git facts for `cwd`; `repo: false` (not an error) outside git.
pub async fn status(cwd: &str, vault: &Path) -> Result<GitCardStatus, String> {
    let dir = dir_of(cwd)?;
    let Ok(root) = read_git(&dir, &["rev-parse", "--show-toplevel"]).await else {
        return Ok(GitCardStatus::default());
    };
    let root = root.trim().to_string();
    let branch = read_git(&dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let head = read_git(&dir, &["rev-parse", "--short", "HEAD"])
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let upstream = read_git(
        &dir,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
    .await
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());
    let (mut ahead, mut behind) = (0u64, 0u64);
    if upstream.is_some() {
        if let Ok(counts) = read_git(
            &dir,
            &["rev-list", "--left-right", "--count", "@{u}...HEAD"],
        )
        .await
        {
            let mut it = counts
                .split_whitespace()
                .map(|n| n.parse::<u64>().unwrap_or(0));
            behind = it.next().unwrap_or(0);
            ahead = it.next().unwrap_or(0);
        }
    } else if !head.is_empty() {
        if let Ok(n) = read_git(&dir, &["rev-list", "--count", "HEAD", "--not", "--remotes"]).await
        {
            ahead = n.trim().parse().unwrap_or(0);
        }
    }
    let linked_worktree = match (
        read_git(&dir, &["rev-parse", "--git-dir"]).await,
        read_git(&dir, &["rev-parse", "--git-common-dir"]).await,
    ) {
        (Ok(a), Ok(b)) => {
            let canon = |p: &str| {
                let p = Path::new(p.trim());
                let p = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    dir.join(p)
                };
                p.canonicalize().unwrap_or(p)
            };
            canon(&a) != canon(&b)
        }
        _ => false,
    };
    let porcelain = read_git(&dir, &["status", "--porcelain=v1", "-z", "-uall"]).await?;
    let records = parse_porcelain_z(&porcelain);
    let more_files = records.len().saturating_sub(FILES_MAX);
    // Counts for tracked changes against HEAD (staged and unstaged together);
    // an unborn HEAD has none, so everything then counts as added lines below.
    let mut counts = std::collections::HashMap::new();
    if !head.is_empty() {
        if let Ok(raw) = read_git(&dir, &["diff", "HEAD", "--numstat", "-z"]).await {
            for (path, _old, a, d) in crate::build_changes::parse_numstat_z(&raw) {
                counts.insert(path, (a, d));
            }
        }
    }
    let top = PathBuf::from(&root);
    let mut files = Vec::new();
    let (mut additions, mut deletions) = (0u64, 0u64);
    // The totals cover every changed file; only the first FILES_MAX are listed.
    // Past that, an untracked file adds no count (reading each would be slow).
    for (i, (path, word)) in records.into_iter().enumerate() {
        let (a, d) = match counts.get(&path) {
            Some(&(a, d)) => (a, d),
            None if (word == "untracked" || word == "added") && i < FILES_MAX => {
                (crate::build_changes::count_lines(&top.join(&path)), Some(0))
            }
            None => (None, None),
        };
        additions += a.unwrap_or(0);
        deletions += d.unwrap_or(0);
        if i >= FILES_MAX {
            continue;
        }
        files.push(CardFile {
            path,
            status: word.to_string(),
            additions: a,
            deletions: d,
        });
    }
    let blocked = crate::agent_sessions::busy_reason(vault)
        .map(|holder| format!("Wait for {holder} to finish before you change git here"));
    Ok(GitCardStatus {
        repo: true,
        root,
        branch,
        head,
        upstream,
        ahead,
        behind,
        linked_worktree,
        files,
        more_files,
        additions,
        deletions,
        blocked,
    })
}

/// Local branches, newest commit first (at most 100), with the current one.
pub async fn branches(cwd: &str) -> Result<GitCardBranches, String> {
    let dir = dir_of(cwd)?;
    let current = read_git(&dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let raw = read_git(
        &dir,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )
    .await?;
    let branches = raw
        .lines()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .take(BRANCHES_MAX)
        .map(str::to_string)
        .collect();
    Ok(GitCardBranches { current, branches })
}

/// A branch name git accepts that cannot be read as an option.
async fn valid_branch(dir: &Path, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('-') || name.len() > 200 {
        return Err("Enter a branch name".into());
    }
    let out = git_raw(dir, &["check-ref-format", "--branch", name], QUICK, 4096).await?;
    if out.code != Some(0) {
        return Err(format!("'{name}' is not a valid branch name"));
    }
    Ok(name.to_string())
}

fn refuse_while_busy(vault: &Path, doing: &str) -> Result<(), String> {
    match crate::agent_sessions::busy_reason(vault) {
        Some(holder) => Err(format!("Wait for {holder} to finish before you {doing}")),
        None => Ok(()),
    }
}

/// `git switch [-c] <branch>`; git itself refuses when local changes would be lost.
pub async fn switch(
    vault: &Path,
    cwd: &str,
    branch: &str,
    create: bool,
) -> Result<GitCardStatus, String> {
    let dir = dir_of(cwd)?;
    refuse_while_busy(vault, "switch branches")?;
    let name = valid_branch(&dir, branch).await?;
    let args: Vec<&str> = if create {
        vec!["switch", "-c", name.as_str()]
    } else {
        vec!["switch", name.as_str()]
    };
    let out = git_raw(&dir, &args, SLOW, 64 * 1024).await?;
    if out.code != Some(0) {
        return Err(failure("Couldn't switch branch", &args, &out));
    }
    status(cwd, vault).await
}

/// The folder a new worktree for `branch` of the repo at `root` gets:
/// `<worktrees root>/<repo name>/<branch slug>`, with a counter when taken.
pub(crate) fn worktree_path(worktrees: &Path, root: &Path, branch: &str) -> PathBuf {
    let repo = root
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| build_workspaces::slug(n, 60))
        .unwrap_or_else(|| "repo".into());
    let slug = build_workspaces::slug(branch, 60);
    let base = worktrees.join(repo);
    let mut path = base.join(&slug);
    let mut n = 2;
    while path.exists() {
        path = base.join(format!("{slug}-{n}"));
        n += 1;
    }
    path
}

/// Worktree mode: `git worktree add -b <branch> <path>` from HEAD. Returns the
/// new folder; the window then points the chat at it.
pub async fn new_worktree(
    vault: &Path,
    worktrees: Option<PathBuf>,
    cwd: &str,
    branch: &str,
) -> Result<String, String> {
    let dir = dir_of(cwd)?;
    refuse_while_busy(vault, "create a worktree")?;
    let worktrees = worktrees.ok_or("Hollow can't find its worktrees folder")?;
    let root = read_git(&dir, &["rev-parse", "--show-toplevel"])
        .await
        .map_err(|_| "This folder is not a git repository".to_string())?;
    let mut root = PathBuf::from(root.trim());
    // From a linked worktree the toplevel is that worktree, not the repository:
    // name the new one after the repository (the common dir's parent).
    if let Ok(common) = read_git(
        &dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .await
    {
        let common = PathBuf::from(common.trim());
        if common.file_name().is_some_and(|n| n == ".git") {
            if let Some(parent) = common.parent() {
                root = parent.to_path_buf();
            }
        }
    }
    let name = valid_branch(&dir, branch).await?;
    let path = worktree_path(&worktrees, &root, &name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Couldn't make {}: {e}", parent.display()))?;
    }
    let target = path.to_string_lossy().into_owned();
    let args = ["worktree", "add", "-b", name.as_str(), target.as_str()];
    let out = git_raw(&dir, &args, SLOW, 64 * 1024).await?;
    if out.code != Some(0) {
        return Err(failure("Couldn't create the worktree", &args, &out));
    }
    Ok(target)
}

fn file_word(f: &CardFile) -> &'static str {
    match f.status.as_str() {
        "added" | "untracked" => "Add",
        "deleted" => "Remove",
        "renamed" => "Rename",
        _ => "Update",
    }
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A commit message from the changed files alone: a subject naming what moved
/// (one verb when they all agree, else "Update"), then one line per file.
pub(crate) fn generated_message(files: &[CardFile], more: usize) -> String {
    if files.is_empty() {
        return String::new();
    }
    let verb = {
        let first = file_word(&files[0]);
        if files.iter().all(|f| file_word(f) == first) {
            first
        } else {
            "Update"
        }
    };
    let names: Vec<&str> = files.iter().map(|f| base_name(&f.path)).collect();
    let total = files.len() + more;
    let mut subject = match names.as_slice() {
        [one] if more == 0 => format!("{verb} {one}"),
        [a, b] if more == 0 => format!("{verb} {a} and {b}"),
        _ => {
            let shown = &names[..names.len().min(2)];
            format!(
                "{verb} {} and {} more files",
                shown.join(", "),
                total - shown.len()
            )
        }
    };
    if subject.chars().count() > SUBJECT_MAX {
        subject = format!("{verb} {total} files");
    }
    let mut body = String::new();
    for f in files.iter().take(12) {
        let counts = match (f.additions, f.deletions) {
            (Some(a), Some(d)) => format!(" (+{a} -{d})"),
            _ => String::new(),
        };
        body.push_str(&format!(
            "- {} {}{counts}\n",
            file_word(f).to_lowercase(),
            f.path
        ));
    }
    if total > 12 {
        body.push_str(&format!("- and {} more files\n", total - 12));
    }
    format!("{subject}\n\n{}", body.trim_end())
}

/// The suggested message for what is changed in `cwd` now.
pub async fn suggest_message(vault: &Path, cwd: &str) -> Result<String, String> {
    let s = status(cwd, vault).await?;
    Ok(generated_message(&s.files, s.more_files))
}

/// Why a commit would be wrong now: a merge, rebase or cherry-pick is
/// unfinished, or a path still has conflict markers. `git add -A` would mark
/// them resolved and the commit would finish the operation with a generated
/// message, so the card refuses and leaves it to the person or the agent.
async fn operation_in_progress(dir: &Path, status: &GitCardStatus) -> Option<String> {
    for (marker, what) in [
        ("MERGE_HEAD", "a merge"),
        ("REBASE_HEAD", "a rebase"),
        ("CHERRY_PICK_HEAD", "a cherry-pick"),
    ] {
        if read_git(dir, &["rev-parse", "-q", "--verify", marker])
            .await
            .is_ok()
        {
            return Some(format!(
                "Finish {what} first: it is still in progress in this folder."
            ));
        }
    }
    let unmerged = read_git(dir, &["diff", "--name-only", "--diff-filter=U", "-z"])
        .await
        .ok()?;
    if !unmerged.is_empty() && !status.files.is_empty() {
        return Some(
            "Resolve the conflicted files first: some still have conflict markers.".into(),
        );
    }
    None
}

/// `git add -A`, commit with `message`, and (with `push`) `git push -u origin
/// <branch>`. With nothing to commit and `push` on, only the push runs.
pub async fn commit_push(
    vault: &Path,
    cwd: &str,
    message: &str,
    push: bool,
) -> Result<GitCardStatus, String> {
    let dir = dir_of(cwd)?;
    refuse_while_busy(vault, if push { "commit and push" } else { "commit" })?;
    let before = status(cwd, vault).await?;
    if !before.repo {
        return Err("This folder is not a git repository".into());
    }
    if let Some(why) = operation_in_progress(&dir, &before).await {
        return Err(why);
    }
    if !before.files.is_empty() || before.more_files > 0 {
        let message = message.trim();
        if message.is_empty() {
            return Err("Write a commit message first".into());
        }
        let add = git_raw(&dir, &["add", "-A"], SLOW, 64 * 1024).await?;
        if add.code != Some(0) {
            return Err(failure("Couldn't stage", &["add", "-A"], &add));
        }
        let args = ["commit", "-q", "-m", message];
        let out = git_raw(&dir, &args, SLOW, 64 * 1024).await?;
        if out.code != Some(0) {
            return Err(failure("Commit failed", &["commit"], &out));
        }
    } else if !push {
        return Err("Nothing to commit: this folder has no changes".into());
    }
    if push {
        let after = status(cwd, vault).await?;
        if after.branch.is_empty() {
            return Err(
                "This folder is not on a branch (detached HEAD); switch to a branch first".into(),
            );
        }
        // A branch that already tracks something pushes there (the card's ahead
        // count is measured against it); a new one is published to origin.
        let args: Vec<&str> = if after.upstream.is_some() {
            vec!["push"]
        } else {
            vec!["push", "-u", "origin", after.branch.as_str()]
        };
        let out = git_raw(&dir, &args, SLOW, 256 * 1024).await?;
        if out.code != Some(0) {
            return Err(failure("Push failed", &args, &out));
        }
    }
    status(cwd, vault).await
}

/// A commit id as `git` prints one: 7 to 64 hex digits, so it can never be
/// read as an option or a revision expression.
pub(crate) fn valid_sha(sha: &str) -> bool {
    (7..=64).contains(&sha.len()) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What the Diff tab compares against when there is no commit yet.
async fn empty_tree(dir: &Path) -> Result<String, String> {
    read_git(dir, &["hash-object", "-t", "tree", "/dev/null"])
        .await
        .map(|s| s.trim().to_string())
}

/// The diff of one file (or all) for the Diff tab: the folder against HEAD
/// (tracked changes and untracked files), or, with `from` and `to` commits (a
/// turn's checkpoints), what changed between them. Redacted and capped.
pub async fn diff(
    cwd: &str,
    path: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<crate::build_changes::BuildDiff, String> {
    let dir = dir_of(cwd)?;
    let path = path.map(crate::build_changes::safe_relative).transpose()?;
    match (from, to) {
        (Some(from), Some(to)) => {
            if !valid_sha(from) || !valid_sha(to) {
                return Err("A turn's diff needs two commit ids".into());
            }
            let mut args = vec![
                "--no-optional-locks",
                "diff",
                "-M",
                "--no-color",
                "--no-ext-diff",
                from,
                to,
            ];
            if let Some(p) = &path {
                args.extend_from_slice(&["--", p.as_str()]);
            }
            let out = git_raw(&dir, &args, QUICK, 1024 * 1024 + 1).await?;
            if !out.truncated && out.code != Some(0) {
                return Err(git_error(&args, &out.stderr));
            }
            let mut result =
                crate::build_changes::assemble(vec![
                    String::from_utf8_lossy(&out.stdout).into_owned()
                ]);
            result.truncated |= out.truncated;
            Ok(result)
        }
        (None, None) => {
            let base = match read_git(&dir, &["rev-parse", "--verify", "-q", "HEAD"]).await {
                Ok(sha) => sha.trim().to_string(),
                Err(_) => empty_tree(&dir).await?,
            };
            crate::build_changes::worktree_diff(&dir, &base, path.as_deref()).await
        }
        _ => Err("A turn's diff needs both of its commits".into()),
    }
}

/// The files between two commits with their counts (a turn's changes).
pub async fn range_files(cwd: &str, from: &str, to: &str) -> Result<Vec<CardFile>, String> {
    let dir = dir_of(cwd)?;
    if !valid_sha(from) || !valid_sha(to) {
        return Err("A turn's files need two commit ids".into());
    }
    let stat = read_git(&dir, &["diff", "--no-renames", "--numstat", "-z", from, to]).await?;
    let names = read_git(
        &dir,
        &["diff", "--no-renames", "--name-status", "-z", from, to],
    )
    .await?;
    let mut counts = std::collections::HashMap::new();
    for (path, _old, a, d) in crate::build_changes::parse_numstat_z(&stat) {
        counts.insert(path, (a, d));
    }
    let mut files = Vec::new();
    for (path, letter, _old) in crate::build_changes::parse_name_status_z(&names)
        .into_iter()
        .take(FILES_MAX)
    {
        let (a, d) = counts.get(&path).copied().unwrap_or((None, None));
        let status = match letter {
            'A' => "added",
            'D' => "deleted",
            'R' | 'C' => "renamed",
            _ => "modified",
        };
        files.push(CardFile {
            path,
            status: status.into(),
            additions: a,
            deletions: d,
        });
    }
    Ok(files)
}

// ---- Tauri commands (the card's clicks; plumbing in the command map) ----

use crate::commands::AppState;
use tauri::State;

#[tauri::command]
pub async fn git_card_status(
    cwd: String,
    state: State<'_, AppState>,
) -> Result<GitCardStatus, String> {
    status(&cwd, &state.config.vault_path).await
}

#[tauri::command]
pub async fn git_card_branches(cwd: String) -> Result<GitCardBranches, String> {
    branches(&cwd).await
}

#[tauri::command]
pub async fn git_card_switch(
    cwd: String,
    branch: String,
    create: Option<bool>,
    state: State<'_, AppState>,
) -> Result<GitCardStatus, String> {
    switch(
        &state.config.vault_path,
        &cwd,
        &branch,
        create.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn git_card_new_worktree(
    cwd: String,
    branch: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let worktrees = crate::disk_guard::worktrees_root(&state.config_path);
    new_worktree(&state.config.vault_path, worktrees, &cwd, &branch).await
}

#[tauri::command]
pub async fn git_card_suggest_message(
    cwd: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    suggest_message(&state.config.vault_path, &cwd).await
}

#[tauri::command]
pub async fn git_card_commit_push(
    cwd: String,
    message: String,
    push: bool,
    state: State<'_, AppState>,
) -> Result<GitCardStatus, String> {
    commit_push(&state.config.vault_path, &cwd, &message, push).await
}

#[tauri::command]
pub async fn git_card_diff(
    cwd: String,
    path: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Result<crate::build_changes::BuildDiff, String> {
    diff(&cwd, path.as_deref(), from.as_deref(), to.as_deref()).await
}

#[tauri::command]
pub async fn git_card_range_files(
    cwd: String,
    from: String,
    to: String,
) -> Result<Vec<CardFile>, String> {
    range_files(&cwd, &from, &to).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_workspaces::tests::sh;

    fn file(path: &str, status: &str, a: u64, d: u64) -> CardFile {
        CardFile {
            path: path.into(),
            status: status.into(),
            additions: Some(a),
            deletions: Some(d),
        }
    }

    #[test]
    fn porcelain_records_map_to_status_words_and_skip_rename_sources() {
        let raw = " M src/a.rs\0A  new.txt\0?? notes.md\0 D gone.txt\0R  b.rs\0a.rs\0";
        assert_eq!(
            parse_porcelain_z(raw),
            vec![
                ("src/a.rs".to_string(), "modified"),
                ("new.txt".to_string(), "added"),
                ("notes.md".to_string(), "untracked"),
                ("gone.txt".to_string(), "deleted"),
                ("b.rs".to_string(), "renamed"),
            ]
        );
    }

    #[test]
    fn generated_message_names_files_and_stays_short() {
        assert_eq!(generated_message(&[], 0), "");
        let one = generated_message(&[file("docs/a.md", "added", 3, 0)], 0);
        assert_eq!(one, "Add a.md\n\n- add docs/a.md (+3 -0)");
        let two = generated_message(
            &[
                file("x/a.rs", "modified", 1, 1),
                file("x/b.rs", "modified", 2, 0),
            ],
            0,
        );
        assert!(two.starts_with("Update a.rs and b.rs\n\n"));
        let mixed = generated_message(
            &[
                file("a", "added", 1, 0),
                file("b", "deleted", 0, 4),
                file("c", "modified", 1, 1),
            ],
            0,
        );
        assert!(
            mixed.starts_with("Update a, b and 1 more files\n\n"),
            "{mixed}"
        );
        let many: Vec<CardFile> = (0..14)
            .map(|i| file(&format!("f{i}.txt"), "untracked", 1, 0))
            .collect();
        let text = generated_message(&many, 3);
        assert!(text
            .lines()
            .next()
            .unwrap()
            .starts_with("Add f0.txt, f1.txt and 15 more files"));
        assert!(text.ends_with("- and 5 more files"), "{text}");
        let long = generated_message(
            &[
                file(&format!("{}.rs", "x".repeat(50)), "modified", 1, 1),
                file(&format!("{}.rs", "y".repeat(50)), "modified", 1, 1),
            ],
            0,
        );
        assert!(long.starts_with("Update 2 files\n"), "{long}");
        assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
    }

    #[test]
    fn worktree_paths_are_slugged_and_never_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let root = Path::new("/somewhere/My Repo");
        let first = worktree_path(tmp.path(), root, "Feature/Add Thing");
        assert!(first.ends_with("my-repo/feature-add-thing"), "{first:?}");
        std::fs::create_dir_all(&first).unwrap();
        let second = worktree_path(tmp.path(), root, "Feature/Add Thing");
        assert!(
            second.ends_with("my-repo/feature-add-thing-2"),
            "{second:?}"
        );
    }

    #[tokio::test]
    async fn a_plain_folder_is_not_a_repo_and_a_repo_reports_changes() {
        let vault = tempfile::tempdir().unwrap();
        let plain = tempfile::tempdir().unwrap();
        let s = status(plain.path().to_str().unwrap(), vault.path())
            .await
            .unwrap();
        assert!(!s.repo && s.files.is_empty());
        let repo = tempfile::tempdir().unwrap();
        let p = repo.path();
        sh(p, &["init", "-q"]);
        std::fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
        sh(p, &["add", "-A"]);
        sh(p, &["commit", "-q", "-m", "first"]);
        std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(p.join("b.txt"), "x\ny\n").unwrap();
        let s = status(p.to_str().unwrap(), vault.path()).await.unwrap();
        assert!(s.repo && !s.linked_worktree);
        assert_eq!(s.branch, "main");
        assert_eq!((s.additions, s.deletions), (3, 0), "{s:?}");
        let a = s.files.iter().find(|f| f.path == "a.txt").unwrap();
        assert_eq!(
            (a.status.as_str(), a.additions, a.deletions),
            ("modified", Some(1), Some(0))
        );
        let msg = suggest_message(vault.path(), p.to_str().unwrap())
            .await
            .unwrap();
        assert!(msg.starts_with("Update a.txt and b.txt"), "{msg}");
    }

    #[tokio::test]
    async fn a_conflicted_or_unfinished_merge_is_not_committed_over() {
        let vault = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let p = repo.path();
        let cwd = p.to_str().unwrap();
        sh(p, &["init", "-q"]);
        sh(p, &["config", "user.name", "t"]);
        sh(p, &["config", "user.email", "t@t"]);
        std::fs::write(p.join("a.txt"), "base\n").unwrap();
        sh(p, &["add", "-A"]);
        sh(p, &["commit", "-q", "-m", "base"]);
        sh(p, &["switch", "-q", "-c", "other"]);
        std::fs::write(p.join("a.txt"), "other\n").unwrap();
        sh(p, &["commit", "-q", "-am", "other"]);
        sh(p, &["switch", "-q", "main"]);
        std::fs::write(p.join("a.txt"), "main\n").unwrap();
        sh(p, &["commit", "-q", "-am", "main"]);
        // The merge conflicts on purpose (the helper asserts success, so run it by hand).
        let merged = std::process::Command::new("git")
            .args([
                "-C",
                cwd,
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "merge",
                "other",
            ])
            .output()
            .unwrap();
        assert!(!merged.status.success());
        let err = commit_push(vault.path(), cwd, "Merge it", false)
            .await
            .unwrap_err();
        assert!(err.starts_with("Finish a merge first"), "{err}");
    }

    #[tokio::test]
    async fn switch_commit_and_worktree_mode_work_on_a_plain_checkout() {
        let vault = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let p = repo.path();
        let cwd = p.to_str().unwrap();
        sh(p, &["init", "-q"]);
        sh(p, &["config", "user.name", "t"]);
        sh(p, &["config", "user.email", "t@t"]);
        std::fs::write(p.join("a.txt"), "one\n").unwrap();
        sh(p, &["add", "-A"]);
        sh(p, &["commit", "-q", "-m", "first"]);
        // A branch name that could be read as an option is refused before git runs.
        assert!(switch(vault.path(), cwd, "--detach", true).await.is_err());
        assert!(switch(vault.path(), cwd, "bad name", true).await.is_err());
        let s = switch(vault.path(), cwd, "topic", true).await.unwrap();
        assert_eq!(s.branch, "topic");
        let b = branches(cwd).await.unwrap();
        assert_eq!(b.current, "topic");
        assert!(b.branches.contains(&"main".to_string()));
        std::fs::write(p.join("a.txt"), "one\nmore\n").unwrap();
        assert!(commit_push(vault.path(), cwd, "  ", false).await.is_err());
        let done = commit_push(vault.path(), cwd, "Add more", false)
            .await
            .unwrap();
        assert!(done.files.is_empty());
        assert!(commit_push(vault.path(), cwd, "again", false)
            .await
            .is_err());
        let worktrees = tempfile::tempdir().unwrap();
        let wt = new_worktree(
            vault.path(),
            Some(worktrees.path().to_path_buf()),
            cwd,
            "side",
        )
        .await
        .unwrap();
        let ws = status(&wt, vault.path()).await.unwrap();
        assert!(ws.repo && ws.linked_worktree);
        assert_eq!(ws.branch, "side");
    }
    #[tokio::test]
    async fn the_diff_tab_reads_uncommitted_and_per_turn_changes() {
        let repo = tempfile::tempdir().unwrap();
        let p = repo.path();
        let cwd = p.to_str().unwrap();
        sh(p, &["init", "-q"]);
        // Before the first commit everything is an addition against the empty tree.
        std::fs::write(p.join("a.txt"), "one\n").unwrap();
        let first = diff(cwd, None, None, None).await.unwrap();
        assert!(first.diff.contains("+one"), "{}", first.diff);
        sh(p, &["add", "-A"]);
        sh(p, &["commit", "-q", "-m", "first"]);
        let a = sha(p);
        std::fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(p.join("new.txt"), "x\n").unwrap();
        let now = diff(cwd, None, None, None).await.unwrap();
        assert!(
            now.diff.contains("+two") && now.diff.contains("+x"),
            "{}",
            now.diff
        );
        let only = diff(cwd, Some("new.txt"), None, None).await.unwrap();
        assert!(only.diff.contains("+x") && !only.diff.contains("+two"));
        sh(p, &["add", "-A"]);
        sh(p, &["commit", "-q", "-m", "second"]);
        let b = sha(p);
        let turn = diff(cwd, None, Some(&a), Some(&b)).await.unwrap();
        assert!(turn.diff.contains("+two"));
        let files = range_files(cwd, &a, &b).await.unwrap();
        let by: std::collections::HashMap<_, _> =
            files.iter().map(|f| (f.path.as_str(), f)).collect();
        assert_eq!(by["new.txt"].status, "added");
        assert_eq!(
            (by["a.txt"].additions, by["a.txt"].deletions),
            (Some(1), Some(0))
        );
        // Odd inputs never reach git as options or paths outside the folder.
        assert!(diff(cwd, None, Some("--output=x"), Some(&b)).await.is_err());
        assert!(diff(cwd, None, Some(&a), None).await.is_err());
        assert!(diff(cwd, Some("../x"), None, None).await.is_err());
        assert!(range_files(cwd, "HEAD", &b).await.is_err());
    }

    fn sha(dir: &Path) -> String {
        let out = std::process::Command::new("git")
            .args(["-C", dir.to_str().unwrap(), "rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
}
