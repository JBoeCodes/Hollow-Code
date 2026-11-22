//! Turn checkpoints with a safe restore (CP1, R1 #4, 2026-11-18).
//!
//! **Where.** Only a chat that owns an isolated git worktree
//! ([`isolation`]): its folder is the root of a linked worktree (never a
//! repository's main checkout) and no other chat has that folder or one
//! inside it. Anywhere
//! else nothing is recorded and a restore is refused before any file changes.
//!
//! **Checkpoints.** Before a turn's agent starts and after the turn ends, the
//! worktree's files (tracked and untracked; ignored files are left out) are
//! written as a commit through a temporary index (seeded from the real one,
//! so only changed files are hashed) and kept under the hidden ref
//! `refs/hollow/checkpoints/<key>/<ord>/before|after`. The real index, the
//! branch and the files are never touched by a checkpoint. A failed
//! checkpoint is logged and never fails a turn.
//!
//! **Per-turn diff** ([`checkpoints`]): the paths that differ between a turn's
//! before and after, without the paths a merge commit made during the turn
//! brought in (listed apart as `mergedIn`).
//!
//! **Restore** ([`restore`]): "Restore to before this turn". Refused while a
//! turn runs, outside an isolated worktree, or with no before checkpoint.
//! It first saves the current state as `.../restore-<unix seconds>` (so the
//! restore itself can be undone with git), moves the branch back when the
//! turn or a later one committed (the commits stay reachable from that safety
//! ref), puts every changed file back and removes files added since. Then
//! Hollow rewinds the conversation itself: the native session is closed
//! (`session_store::restart_stint`), so the next turn starts a fresh one,
//! and a `checkpoint_restore` event leaves the undone turns out of the
//! history that fresh session receives (`session_store::conversation_entries`).
//! The transcript keeps every turn; the event and the chat's note say what was
//! undone. AC1's `rollback` flag stays false: no adapter rewinds natively.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Where the hidden refs live (shared by every worktree of a repository).
pub(crate) const REF_ROOT: &str = "refs/hollow/checkpoints";
/// The `session_store` event kind a restore writes.
pub(crate) const RESTORE_EVENT: &str = "checkpoint_restore";

// ── git ───────────────────────────────────────────────────────────────

/// A git command in `dir` with the caller's `GIT_*` environment dropped,
/// hooks, fsmonitor, auto gc and signing off, and literal pathspecs.
fn git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    cmd.current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .env("LC_ALL", "C")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "gc.auto=0",
            "-c",
            "commit.gpgSign=false",
        ])
        .stdin(Stdio::null());
    #[cfg(test)]
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("git did not start: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("git stopped ({})", out.status)
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim_end_matches('\n')
        .to_string())
}

fn run_z(cmd: &mut Command) -> Result<Vec<String>, String> {
    Ok(run(cmd)?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn rev(top: &Path, spec: &str) -> Option<String> {
    run(git(top).args(["rev-parse", "--verify", "-q", spec])).ok()
}

fn ref_name(key: &str, ord: i64, side: &str) -> String {
    format!("{REF_ROOT}/{key}/{ord}/{side}")
}

// ── isolation ─────────────────────────────────────────────────────────

/// The chat's worktree root when the chat owns an isolated worktree, else
/// why not (the refusal text).
pub(crate) fn isolation(vault: &Path, key: &str) -> Result<PathBuf, String> {
    let cwd = crate::session_store::session_cwd(vault, key)
        .filter(|c| !c.trim().is_empty())
        .ok_or("This chat has no working folder.")?;
    let dir = Path::new(&cwd);
    let top = run(git(dir).args(["rev-parse", "--show-toplevel"]))
        .map_err(|_| "This chat's folder is not a git worktree.".to_string())?;
    let git_dir = run(git(dir).args(["rev-parse", "--absolute-git-dir"]))?;
    let common = run(git(dir).args(["rev-parse", "--path-format=absolute", "--git-common-dir"]))?;
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    if canon(&git_dir) == canon(&common) {
        return Err(
            "This chat works in a repository's main checkout, not an isolated worktree.".into(),
        );
    }
    if canon(&top) != canon(&cwd) {
        return Err("This chat's folder is inside a worktree, not its root.".into());
    }
    let mine = canon(&top);
    for (other, folder) in crate::session_store::session_cwds(vault)? {
        if other == key || folder.trim().is_empty() {
            continue;
        }
        let theirs = canon(folder.trim_end_matches('/'));
        // The same folder or one inside it. A chat in a parent folder (the
        // home folder, say) is not counted, or nothing would be isolated.
        if theirs.starts_with(&mine) {
            return Err(format!(
                "Another chat ({other}) works in this worktree, so it is shared, not isolated."
            ));
        }
    }
    Ok(PathBuf::from(top))
}

// ── checkpoints ───────────────────────────────────────────────────────

/// The worktree's files now as a commit (parent: HEAD when there is one),
/// written through a temporary index; nothing visible changes.
pub(crate) fn snapshot(top: &Path, message: &str) -> Result<String, String> {
    let head = rev(top, "HEAD^{commit}");
    let real = run(git(top).args(["rev-parse", "--path-format=absolute", "--git-path", "index"]))?;
    let tmp = tempfile::Builder::new()
        .prefix("hollow-checkpoint-")
        .tempfile()
        .map_err(|e| e.to_string())?;
    let index = tmp.path().to_path_buf();
    if Path::new(&real).is_file() {
        std::fs::copy(&real, &index).map_err(|e| format!("index copy: {e}"))?;
    } else {
        // An empty file is not a valid index; git makes one when missing.
        std::fs::remove_file(&index).map_err(|e| e.to_string())?;
    }
    let with_index = |args: &[&str]| run(git(top).env("GIT_INDEX_FILE", &index).args(args));
    with_index(&["add", "-A", "--", "."])?;
    let tree = with_index(&["write-tree"])?;
    let mut commit = git(top);
    commit
        .env("GIT_AUTHOR_NAME", "Hollow checkpoint")
        .env("GIT_AUTHOR_EMAIL", "checkpoint@hollow.invalid")
        .env("GIT_COMMITTER_NAME", "Hollow checkpoint")
        .env("GIT_COMMITTER_EMAIL", "checkpoint@hollow.invalid")
        .args(["commit-tree", &tree, "-m", message]);
    if let Some(h) = &head {
        commit.args(["-p", h]);
    }
    let sha = run(&mut commit)?;
    let _ = std::fs::remove_file(&index);
    Ok(sha)
}

fn record(
    vault: &Path,
    key: &str,
    ord: i64,
    side: &str,
    abandoned: &AtomicBool,
) -> Result<(), String> {
    if side == "after" && !has_before(vault, key, ord) {
        return Ok(());
    }
    let top = match isolation(vault, key) {
        Ok(top) => top,
        Err(_) => return Ok(()),
    };
    let branch = branch_of(&top);
    let message = format!("Hollow checkpoint {key} turn {ord} {side}\n\n{BRANCH_TAG}{branch}");
    let sha = snapshot(&top, &message)?;
    // A before checkpoint that outran its wait is never written: the agent
    // may already be editing.
    if abandoned.load(Ordering::SeqCst) {
        return Err("took too long; not recorded".into());
    }
    run(git(&top).args(["update-ref", &ref_name(key, ord, side), &sha]))?;
    Ok(())
}

/// The before checkpoint's message names the branch HEAD was on.
const BRANCH_TAG: &str = "branch=";

/// HEAD's branch (`refs/heads/...`), or empty when detached.
fn branch_of(top: &Path) -> String {
    run(git(top).args(["symbolic-ref", "-q", "HEAD"])).unwrap_or_default()
}

fn has_before(vault: &Path, key: &str, ord: i64) -> bool {
    crate::session_store::session_cwd(vault, key)
        .is_some_and(|cwd| rev(Path::new(&cwd), &ref_name(key, ord, "before")).is_some())
}

/// Record turn `ord`'s before checkpoint (before its agent starts). Quiet:
/// only in an isolated worktree; a failure is logged.
pub(crate) async fn before_turn(vault: PathBuf, key: String, ord: i64) {
    hook(vault, key, ord, "before").await
}

/// Record turn `ord`'s after checkpoint (when it ends), only when its before
/// one exists.
pub(crate) async fn after_turn(vault: PathBuf, key: String, ord: i64) {
    hook(vault, key, ord, "after").await
}

async fn hook(vault: PathBuf, key: String, ord: i64, side: &'static str) {
    if !crate::agent_sessions::is_safe_session_key(&key) {
        return;
    }
    let abandoned = Arc::new(AtomicBool::new(false));
    let flag = abandoned.clone();
    let job = crate::vault::spawn_blocking_logged(move || record(&vault, &key, ord, side, &flag));
    match tokio::time::timeout(HOOK_WAIT, job).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => eprintln!("[checkpoint] turn {ord} {side}: {e}"),
        Ok(Err(e)) => eprintln!("[checkpoint] turn {ord} {side}: {e}"),
        Err(_) => {
            abandoned.store(true, Ordering::SeqCst);
            eprintln!(
                "[checkpoint] turn {ord} {side}: over {} s, skipped",
                HOOK_WAIT.as_secs()
            );
        }
    }
}

/// How long a turn waits for its checkpoint; past it the turn goes on and
/// that checkpoint is not recorded (so no restore to before that turn).
const HOOK_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

/// What a turn sent while its chat's restore runs is told.
pub(crate) const RESTORE_BUSY: &str =
    "a restore of this chat's files is running; send again when it ends";

/// Chats whose restore is running: a new turn there does not start.
static RESTORING: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

/// Whether a restore of `key`'s worktree is running (chat.rs refuses a turn).
pub(crate) fn restoring(key: &str) -> bool {
    RESTORING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .contains(key)
}

struct Restoring(String);

impl Drop for Restoring {
    fn drop(&mut self) {
        RESTORING
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.0);
    }
}

/// One turn's checkpoint pair and what it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnCheckpoint {
    pub ord: i64,
    /// The turn's run id (how the chat finds the turn it shows).
    pub run_id: Option<String>,
    pub before: String,
    pub after: Option<String>,
    /// Paths that differ between before and after, merged-in ones left out.
    pub files: Vec<String>,
    /// Paths a merge commit made during the turn brought in.
    pub merged_in: Vec<String>,
}

/// What a chat's checkpoints are; empty outside an isolated worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Checkpoints {
    /// Whether restore can run here (an isolated worktree).
    pub isolated: bool,
    /// Why not, when it cannot.
    pub reason: Option<String>,
    pub turns: Vec<TurnCheckpoint>,
}

pub(crate) fn checkpoints(vault: &Path, key: &str) -> Result<Checkpoints, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let top = match isolation(vault, key) {
        Ok(top) => top,
        Err(reason) => {
            return Ok(Checkpoints {
                isolated: false,
                reason: Some(reason),
                turns: Vec::new(),
            })
        }
    };
    let runs = crate::session_store::turn_run_ids(vault, key).unwrap_or_default();
    let undone = crate::session_store::undone_turns(vault, key).unwrap_or_default();
    let turns = pairs(&top, key)?
        .into_iter()
        .filter(|(ord, _)| !undone.iter().any(|(a, b)| (*a..=*b).contains(ord)))
        .map(|(ord, (before, after))| {
            let (files, merged_in) = match &after {
                Some(after) => turn_files(&top, &before, after).unwrap_or_default(),
                None => (Vec::new(), Vec::new()),
            };
            TurnCheckpoint {
                ord,
                run_id: runs.get(&ord).cloned(),
                before,
                after,
                files,
                merged_in,
            }
        })
        .collect();
    Ok(Checkpoints {
        isolated: true,
        reason: None,
        turns,
    })
}

type Pairs = BTreeMap<i64, (String, Option<String>)>;

fn pairs(top: &Path, key: &str) -> Result<Pairs, String> {
    let prefix = format!("{REF_ROOT}/{key}/");
    let out = run(git(top).args(["for-each-ref", "--format=%(refname) %(objectname)", &prefix]))?;
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    for line in out.lines() {
        let Some((name, sha)) = line.split_once(' ') else {
            continue;
        };
        let rest = name.strip_prefix(&prefix).unwrap_or("");
        let Some((ord, side)) = rest.split_once('/') else {
            continue;
        };
        let Ok(ord) = ord.parse::<i64>() else {
            continue;
        };
        match side {
            "before" => before.insert(ord, sha.to_string()),
            "after" => after.insert(ord, sha.to_string()),
            _ => None,
        };
    }
    Ok(before
        .into_iter()
        .map(|(ord, b)| (ord, (b, after.remove(&ord))))
        .collect())
}

fn diff_names(top: &Path, a: &str, b: &str) -> Result<Vec<String>, String> {
    run_z(git(top).args(["diff", "--no-renames", "--name-only", "-z", a, b, "--"]))
}

/// (changed paths without merged-in ones, merged-in paths).
fn turn_files(top: &Path, before: &str, after: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let all = diff_names(top, before, after)?;
    let mut merged = BTreeSet::new();
    if let (Some(from), Some(to)) = (
        rev(top, &format!("{before}^1")),
        rev(top, &format!("{after}^1")),
    ) {
        let range = format!("{from}..{to}");
        let merges = run(git(top).args(["rev-list", "--merges", &range]))?;
        for m in merges.lines().filter(|l| !l.is_empty()) {
            merged.extend(diff_names(top, &format!("{m}^1"), m)?);
        }
    }
    let (merged_in, files) = all.into_iter().partition(|p| merged.contains(p));
    Ok((files, merged_in))
}

// ── restore ───────────────────────────────────────────────────────────

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Restored {
    /// The turn restored to before.
    pub ord: i64,
    /// The last turn undone.
    pub through: i64,
    pub restored: Vec<String>,
    pub removed: Vec<String>,
    /// The branch moved back to (when a turn had committed).
    pub branch_reset_to: Option<String>,
    /// The ref holding the state before this restore.
    pub safety_ref: String,
    pub text: String,
}

/// Restore the chat's worktree to before turn `ord` (see the module notes).
/// `running` says whether a turn of this chat runs now (asked under the lock).
pub(crate) fn restore(
    vault: &Path,
    key: &str,
    ord: i64,
    running: &dyn Fn() -> bool,
) -> Result<Restored, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    if !RESTORING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key.to_string())
    {
        return Err("A restore of this chat is already running.".into());
    }
    let _restoring = Restoring(key.to_string());
    // Asked only once the lock is held: a turn that starts later is refused
    // by `restoring` (chat.rs).
    if running() {
        return Err("A turn is running in this chat; restore once it ends.".into());
    }
    let top = isolation(vault, key)?;
    let pairs = pairs(&top, key)?;
    let (before, _) = pairs
        .get(&ord)
        .ok_or_else(|| format!("No checkpoint was recorded before turn {ord}."))?;
    let before = before.clone();
    let through = crate::session_store::turn_run_ids(vault, key)?
        .keys()
        .chain(pairs.keys())
        .copied()
        .max()
        .unwrap_or(ord)
        .max(ord);
    let agent = crate::session_store::session_agent(vault, key)?;
    let message = run(git(&top).args(["log", "-1", "--format=%B", &before]))?;
    let branch_then = message
        .lines()
        .find_map(|l| l.strip_prefix(BRANCH_TAG))
        .unwrap_or("")
        .to_string();
    let branch_now = branch_of(&top);
    if branch_then != branch_now {
        let name = |b: &str| {
            if b.is_empty() {
                "a detached HEAD".to_string()
            } else {
                b.trim_start_matches("refs/heads/").to_string()
            }
        };
        return Err(format!(
            "This worktree was on {} before turn {ord} and is on {} now; switch back first.",
            name(&branch_then),
            name(&branch_now)
        ));
    }

    let now = snapshot(
        &top,
        &format!("Hollow checkpoint {key} before restore to {ord}"),
    )?;
    let changes = run_z(git(&top).args([
        "diff",
        "--no-renames",
        "--name-status",
        "-z",
        &before,
        &now,
        "--",
    ]))?;
    let mut restored = Vec::new();
    let mut removed = Vec::new();
    let mut it = changes.into_iter();
    while let (Some(status), Some(path)) = (it.next(), it.next()) {
        if status == "A" {
            removed.push(path);
        } else {
            restored.push(path);
        }
    }
    // Nothing about to be overwritten may be outside the safety snapshot: a
    // file git ignores now, or a folder where a file goes back.
    let present: Vec<&String> = restored
        .iter()
        .filter(|p| std::fs::symlink_metadata(top.join(p)).is_ok())
        .collect();
    for path in &restored {
        let mut parent = Path::new(path.as_str()).parent();
        while let Some(dir) = parent.filter(|d| !d.as_os_str().is_empty()) {
            if std::fs::symlink_metadata(top.join(dir)).is_ok_and(|m| !m.is_dir()) {
                return Err(format!(
                    "{} is not a folder now; restoring {path} would replace it. Nothing was changed.",
                    dir.display()
                ));
            }
            parent = dir.parent();
        }
    }
    if let Some(dir) = present.iter().find(|p| top.join(p).is_dir()) {
        return Err(format!(
            "{dir} is a folder now; restoring would replace it. Nothing was changed."
        ));
    }
    if let Some(ignored) = ignored_among(&top, &present)? {
        return Err(format!(
            "{ignored} is ignored by git now, so it is not in the safety snapshot; restoring would overwrite it. Nothing was changed."
        ));
    }

    let stamp = chrono::Utc::now().timestamp_millis();
    let safety_ref = format!("{REF_ROOT}/{key}/restore-{stamp}");
    // The real index as a tree too, so staged hunks that differ from the
    // files stay recoverable (unresolved conflicts refuse here).
    let staged = run(git(&top).args(["write-tree"])).map_err(|e| {
        format!("the index can't be saved ({e}); resolve or unstage first. Nothing was changed.")
    })?;
    // Create-only: an existing ref is never overwritten.
    run(git(&top).args(["update-ref", &safety_ref, &now, ""]))?;
    run(git(&top).args(["update-ref", &format!("{safety_ref}-index"), &staged, ""]))?;

    let head_then = rev(&top, &format!("{before}^1"));
    let head_now = rev(&top, "HEAD^{commit}");
    let mut branch_reset_to = None;
    if head_then.is_none() {
        // No commit yet before the turn: the index goes back to empty.
        run(git(&top).args(["read-tree", "--empty"]))?;
    }
    if let Some(then) = &head_then {
        // Always: the index goes back too, so nothing undone stays staged.
        run(git(&top).args(["reset", "-q", "--mixed", then]))?;
        if head_now.as_deref() != Some(then.as_str()) {
            branch_reset_to = Some(then.clone());
        }
    }

    for path in &removed {
        let full = top.join(path);
        match std::fs::symlink_metadata(&full) {
            Ok(m) if !m.is_dir() => {
                std::fs::remove_file(&full).map_err(|e| format!("{path}: {e}"))?;
            }
            _ => {}
        }
    }
    if !restored.is_empty() {
        let mut child = git(&top)
            .args([
                "restore",
                "--source",
                &before,
                "--worktree",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git did not start: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            for p in &restored {
                stdin
                    .write_all(p.as_bytes())
                    .and_then(|_| stdin.write_all(b"\0"))
                    .map_err(|e| e.to_string())?;
            }
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "restoring files stopped: {} (the state before the restore is in {safety_ref})",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }

    crate::session_store::restart_stint(vault, key, &agent, "restored to a checkpoint")?;
    let text = format!(
        "Restored the files to before turn {}; turns {} to {} are undone and left out of the agent's context (they stay in this transcript). The state before the restore is kept in {safety_ref}.",
        ord,
        ord,
        through
    );
    let payload = serde_json::json!({
        "before": ord,
        "through": through,
        "safetyRef": safety_ref,
        "restored": restored.len(),
        "removed": removed.len(),
        "text": text,
    });
    crate::session_store::append_event(vault, key, through, RESTORE_EVENT, &payload.to_string())?;
    Ok(Restored {
        ord,
        through,
        restored,
        removed,
        branch_reset_to,
        safety_ref,
        text,
    })
}

/// The first of `paths` git ignores, if any.
fn ignored_among(top: &Path, paths: &[&String]) -> Result<Option<String>, String> {
    if paths.is_empty() {
        return Ok(None);
    }
    let mut child = git(top)
        // check-ignore refuses literal pathspec magic.
        .env_remove("GIT_LITERAL_PATHSPECS")
        .args(["check-ignore", "-z", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("git did not start: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        for p in paths {
            stdin
                .write_all(p.as_bytes())
                .and_then(|_| stdin.write_all(b"\0"))
                .map_err(|e| e.to_string())?;
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    // check-ignore exits 1 when nothing is ignored; anything else is a
    // failure, which refuses the restore rather than assume nothing is.
    if !matches!(out.status.code(), Some(0) | Some(1)) {
        return Err(format!(
            "could not check which files git ignores ({}); nothing was changed",
            out.status
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .find(|s| !s.is_empty())
        .map(str::to_string))
}

// ── branches (CH1) ────────────────────────────────────────────────────

/// Where a branch's files stood (CH1, `chat_branch`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BranchFiles {
    pub top: PathBuf,
    /// Turn `ord`'s after checkpoint (a commit).
    pub checkpoint: String,
    /// Paths that differ between it and the files now.
    pub changed: Vec<String>,
}

/// Turn `ord`'s after checkpoint in chat `key`'s isolated worktree and what
/// changed since, else `None` (not isolated, no checkpoint, a git failure,
/// logged). Reads only: the snapshot of now is a commit no ref keeps.
pub(crate) fn files_since(vault: &Path, key: &str, ord: i64) -> Option<BranchFiles> {
    let top = isolation(vault, key).ok()?;
    let checkpoint = rev(&top, &format!("{}^{{commit}}", ref_name(key, ord, "after")))?;
    let changed = snapshot(&top, "Hollow branch point check")
        .and_then(|now| diff_names(&top, &checkpoint, &now));
    match changed {
        Ok(changed) => Some(BranchFiles {
            top,
            checkpoint,
            changed,
        }),
        Err(e) => {
            eprintln!("[branch] {key} turn {ord}: files since: {e}");
            None
        }
    }
}

/// Where a branch's checkpoint is kept.
pub(crate) const BRANCH_REF_ROOT: &str = "refs/hollow/branches";

/// Keep `sha` reachable as `refs/hollow/branches/<branch>` (create-only).
pub(crate) fn keep_for_branch(top: &Path, branch: &str, sha: &str) -> Result<String, String> {
    if !crate::agent_sessions::is_safe_session_key(branch) {
        return Err("unsafe branch key".into());
    }
    let name = format!("{BRANCH_REF_ROOT}/{branch}");
    // The empty old value makes update-ref refuse a ref that exists.
    run(git(top).args(["update-ref", &name, sha, ""]))?;
    Ok(name)
}

// ── commands ──────────────────────────────────────────────────────────

/// A chat's turn checkpoints and what each turn changed (agents:
/// `sessions.turn_checkpoints`). Reads only.
#[tauri::command]
pub async fn turn_checkpoints(
    state: tauri::State<'_, crate::commands::AppState>,
    key: String,
) -> Result<Checkpoints, String> {
    turn_checkpoints_at(state.config.vault_path.clone(), key).await
}

pub(crate) async fn turn_checkpoints_at(
    vault: PathBuf,
    key: String,
) -> Result<Checkpoints, String> {
    crate::vault::spawn_blocking_logged(move || checkpoints(&vault, &key))
        .await
        .map_err(|e| e.to_string())?
}

/// Restore a chat's worktree to before turn `ord` (the person's click on
/// the turn; user-only).
#[tauri::command]
pub async fn restore_turn_checkpoint(
    state: tauri::State<'_, crate::commands::AppState>,
    key: String,
    ord: i64,
) -> Result<Restored, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        restore(&vault, &key, ord, &|| crate::chat::turn_live(&vault, &key))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
#[path = "turn_checkpoints_tests.rs"]
mod tests;
