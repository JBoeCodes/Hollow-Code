//! Which Hollow checkout answers for the installed app (fix chat 10-20):
//! the repository `build.install_ready` and `build.install_brings` (no id)
//! read `origin/main` from.
//!
//! 1. **The install receipt.** The last install's build receipt names the
//!    build workspace the installed app came from: that binding's repository
//!    (removed bindings too, while the folder is a Hollow checkout), else the
//!    receipt's own worktree.
//! 2. **Build workspaces.** Otherwise the repositories bindings were cut from:
//!    live bindings before removed ones, newest first, never path order (an
//!    old checkout's removed bindings sorted first by path and answered for
//!    the app). When the installed commit is known, the first whose
//!    `origin/main` contains it wins.
//! 3. **Said, never silent.** When the chosen checkout's `origin/main`
//!    doesn't contain the installed commit, `warning` names the checkout, its
//!    `origin/main` and the installed commit. When the installed commit is
//!    unknown, `warning` says it couldn't be checked and `installed_unknown`
//!    is set (install readiness lists it without holding `ready` false).
//!
//! [`hollow_repo_in`] takes the receipt and the installed commit as
//! arguments, so it reads no app state but the vault's bindings;
//! [`for_this_mac`] passes the last install receipt only when that install
//! succeeded (a failed or refused one says nothing about the installed app).

use crate::self_install::{is_hollow_repo, BuildReceipt, InstallResult};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Where the checkout was found.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RepoSource {
    /// The last install receipt's build workspace (or its worktree).
    InstallReceipt,
    /// A build workspace binding, live before removed, newest first.
    BuildWorkspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HollowRepo {
    pub path: PathBuf,
    pub source: RepoSource,
    /// Set when this checkout's `origin/main` doesn't contain the installed
    /// commit (it may not be the repository the app was built from), or when
    /// the installed commit is unknown (see `installed_unknown`).
    pub warning: Option<String>,
    /// The installed commit was unknown, so nothing was checked against it:
    /// `warning` says so, but it is no evidence of a wrong checkout.
    pub installed_unknown: bool,
}

/// The warning when the installed commit is unknown.
pub(crate) const UNKNOWN_INSTALLED: &str =
    "The installed commit is unknown, so this checkout's origin/main can't be checked against it.";

/// The resolved git with `GIT_*` stripped (`build_workspaces::git_command`),
/// run synchronously: this module is called from blocking report code.
fn git_ok(repo: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = crate::build_workspaces::git_command(repo);
    let out = cmd
        .as_std_mut()
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `repo`'s `origin/main` contains `commit` (false when either is missing).
fn main_contains(repo: &Path, commit: &str) -> bool {
    git_ok(
        repo,
        &[
            "merge-base",
            "--is-ancestor",
            "--end-of-options",
            commit,
            "refs/remotes/origin/main",
        ],
    )
    .is_some()
}

fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

fn warning_for(repo: &Path, installed: &str) -> Option<String> {
    if main_contains(repo, installed) {
        return None;
    }
    let main = git_ok(
        repo,
        &["rev-parse", "--verify", "-q", "refs/remotes/origin/main"],
    )
    .map(|s| format!("origin/main ({})", short(&s)))
    .unwrap_or_else(|| "origin/main (missing)".into());
    Some(format!(
        "{}'s {main} doesn't contain the installed commit {}: it may not be the repo this app was built from (or it hasn't fetched since, or the install was a branch build).",
        repo.display(),
        short(installed)
    ))
}

/// The checkout for `vault`, from the last install's build receipt and the
/// installed commit (both passed in, so this reads no app state of its own).
pub(crate) fn hollow_repo_in(
    vault: &Path,
    receipt: Option<&BuildReceipt>,
    installed: Option<&str>,
) -> Option<HollowRepo> {
    let installed = installed.map(str::trim).filter(|c| !c.is_empty());
    // `checked`: `main_contains` already passed for this path.
    let found = |path: PathBuf, source: RepoSource, checked: bool| HollowRepo {
        warning: match installed {
            None => Some(UNKNOWN_INSTALLED.into()),
            Some(_) if checked => None,
            Some(c) => warning_for(&path, c),
        },
        installed_unknown: installed.is_none(),
        path,
        source,
    };
    if let Some(r) = receipt {
        let from_binding = (!r.workspace_id.is_empty())
            .then(|| crate::build_workspaces::binding_for(vault, &r.workspace_id))
            .flatten()
            .map(|b| PathBuf::from(b.repo));
        let from_worktree = (!r.worktree.is_empty()).then(|| PathBuf::from(&r.worktree));
        if let Some(path) = from_binding
            .into_iter()
            .chain(from_worktree)
            .find(|p| p.is_dir() && is_hollow_repo(p))
        {
            return Some(found(path, RepoSource::InstallReceipt, false));
        }
    }
    let candidates: Vec<PathBuf> = crate::build_workspaces::repos_newest_first(vault)
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.is_dir() && is_hollow_repo(p))
        .collect();
    if let Some(path) = installed.and_then(|c| candidates.iter().find(|p| main_contains(p, c))) {
        return Some(found(path.clone(), RepoSource::BuildWorkspace, true));
    }
    let path = candidates.into_iter().next()?;
    Some(found(path, RepoSource::BuildWorkspace, false))
}

/// The build an install result names, only when that install succeeded: a
/// failed or refused install's receipt is the build that didn't go in.
fn installed_build(result: InstallResult) -> Option<BuildReceipt> {
    result.ok.then_some(result.build).flatten()
}

/// [`hollow_repo_in`] with this Mac's last install receipt, when that install succeeded.
pub(crate) fn for_this_mac(vault: &Path, installed: Option<&str>) -> Option<HollowRepo> {
    let receipt = crate::self_install::last_receipt().and_then(|r| installed_build(r.result));
    hollow_repo_in(vault, receipt.as_ref(), installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_workspaces::{bind_repo_for_test, mark_removed_for_test};
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A Hollow checkout named `name` under `root` whose origin/main is its
    /// one commit (distinct per name). Returns (canonical path, commit).
    fn hollow(root: &Path, name: &str) -> (PathBuf, String) {
        let d = root.join(name);
        std::fs::create_dir_all(d.join("src-tauri")).unwrap();
        std::fs::write(
            d.join("src-tauri/tauri.conf.json"),
            r#"{"identifier":"com.hollow.app"}"#,
        )
        .unwrap();
        std::fs::write(d.join("NAME"), name).unwrap();
        sh(&d, &["init", "-q", "-b", "main"]);
        sh(&d, &["add", "."]);
        sh(&d, &["commit", "-qm", name]);
        let head = sh(&d, &["rev-parse", "HEAD"]);
        sh(&d, &["update-ref", "refs/remotes/origin/main", &head]);
        (d.canonicalize().unwrap(), head)
    }

    fn receipt(workspace_id: &str, worktree: &Path, commit: &str) -> BuildReceipt {
        BuildReceipt {
            build_id: "b1".into(),
            workspace_id: workspace_id.into(),
            worktree: worktree.to_string_lossy().into(),
            commit: commit.into(),
            ..Default::default()
        }
    }

    // The 10-20 bug: two Hollow checkouts bound, the old one sorting first by
    // path; the install receipt names the new one's workspace.
    #[test]
    fn the_repo_comes_from_the_install_receipt_not_path_order() {
        let root = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let (old, _) = hollow(root.path(), "aaa-old");
        let (new, new_head) = hollow(root.path(), "zzz-new");
        bind_repo_for_test(vault.path(), "old-ws", &old, &old.join("gone"));
        mark_removed_for_test(vault.path(), "old-ws");
        bind_repo_for_test(vault.path(), "new-ws", &new, &new.join("gone"));
        let r = receipt("new-ws", &new.join("gone"), &new_head);
        let found = hollow_repo_in(vault.path(), Some(&r), Some(&new_head)).unwrap();
        assert_eq!(found.path, new, "{found:?}");
        assert_eq!(found.source, RepoSource::InstallReceipt);
        assert_eq!(found.warning, None);
        // A receipt whose workspace is no longer bound: its worktree, when
        // that is a Hollow checkout.
        let r = receipt("forgotten-ws", &new, &new_head);
        let found = hollow_repo_in(vault.path(), Some(&r), Some(&new_head)).unwrap();
        assert_eq!(
            (found.path, found.source),
            (new.clone(), RepoSource::InstallReceipt)
        );
        // The receipt's repository lacks the installed commit (installed some
        // other way since): kept, but said.
        let old_head = sh(&old, &["rev-parse", "HEAD"]);
        let found = hollow_repo_in(vault.path(), Some(&r), Some(&old_head)).unwrap();
        assert_eq!(found.path, new);
        let w = found.warning.expect("warned");
        assert!(
            w.contains(&*new.to_string_lossy())
                && w.contains(&new_head[..8])
                && w.contains(&old_head[..8])
                && w.contains("may not be the repo this app was built from"),
            "{w}"
        );
    }

    #[test]
    fn without_a_receipt_live_newest_bindings_holding_the_installed_commit_win() {
        let root = tempfile::tempdir().unwrap();
        let (old, old_head) = hollow(root.path(), "aaa-old");
        let (new, new_head) = hollow(root.path(), "zzz-new");
        // Newest live binding first, the installed commit unknown.
        let vault = tempfile::tempdir().unwrap();
        bind_repo_for_test(vault.path(), "old-ws", &old, &old.join("w"));
        bind_repo_for_test(vault.path(), "new-ws", &new, &new.join("w"));
        std::fs::create_dir_all(old.join("w")).unwrap();
        std::fs::create_dir_all(new.join("w")).unwrap();
        let found = hollow_repo_in(vault.path(), None, None).unwrap();
        assert_eq!(
            (found.path.clone(), found.source),
            (new.clone(), RepoSource::BuildWorkspace)
        );
        // The installed commit unknown: picked by order alone, and said.
        assert_eq!(found.warning.as_deref(), Some(UNKNOWN_INSTALLED));
        assert!(found.installed_unknown);
        // A removed binding loses to a live one even when it is newer
        // (ordering alone: no installed commit to check ancestry with).
        let vault = tempfile::tempdir().unwrap();
        bind_repo_for_test(vault.path(), "new-ws", &new, &new.join("w"));
        bind_repo_for_test(vault.path(), "old-ws", &old, &old.join("w"));
        mark_removed_for_test(vault.path(), "old-ws");
        let found = hollow_repo_in(vault.path(), None, None).unwrap();
        assert_eq!(
            (found.path, found.warning.as_deref()),
            (new.clone(), Some(UNKNOWN_INSTALLED))
        );
        // Only removed bindings: the newest of them first.
        let vault = tempfile::tempdir().unwrap();
        bind_repo_for_test(vault.path(), "old-ws", &old, &old.join("w"));
        bind_repo_for_test(vault.path(), "new-ws", &new, &new.join("w"));
        mark_removed_for_test(vault.path(), "old-ws");
        mark_removed_for_test(vault.path(), "new-ws");
        assert_eq!(hollow_repo_in(vault.path(), None, None).unwrap().path, new);
        let vault = tempfile::tempdir().unwrap();
        bind_repo_for_test(vault.path(), "new-ws", &new, &new.join("w"));
        bind_repo_for_test(vault.path(), "old-ws", &old, &old.join("w"));
        mark_removed_for_test(vault.path(), "new-ws");
        mark_removed_for_test(vault.path(), "old-ws");
        assert_eq!(hollow_repo_in(vault.path(), None, None).unwrap().path, old);
        // Both live: the one whose origin/main holds the installed commit wins
        // over a newer one that lacks it.
        let vault = tempfile::tempdir().unwrap();
        bind_repo_for_test(vault.path(), "new-ws", &new, &new.join("w"));
        bind_repo_for_test(vault.path(), "old-ws", &old, &old.join("w"));
        let found = hollow_repo_in(vault.path(), None, Some(&new_head)).unwrap();
        assert_eq!((found.path, found.warning), (new.clone(), None));
        assert!(!found.installed_unknown);
        // Only an old checkout is known and it lacks the installed commit:
        // never picked silently.
        let vault = tempfile::tempdir().unwrap();
        bind_repo_for_test(vault.path(), "old-ws", &old, &old.join("w"));
        mark_removed_for_test(vault.path(), "old-ws");
        let found = hollow_repo_in(vault.path(), None, Some(&new_head)).unwrap();
        assert_eq!(found.path, old);
        let w = found.warning.expect("warned");
        assert!(
            w.contains(&*old.to_string_lossy())
                && w.contains(&old_head[..8])
                && w.contains(&new_head[..8]),
            "{w}"
        );
        // Nothing bound, no receipt: none.
        assert_eq!(
            hollow_repo_in(tempfile::tempdir().unwrap().path(), None, None),
            None
        );
    }

    #[test]
    fn a_receipt_with_an_unknown_installed_commit_is_used_and_said() {
        let root = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let (new, new_head) = hollow(root.path(), "zzz-new");
        let r = receipt("forgotten-ws", &new, &new_head);
        let found = hollow_repo_in(vault.path(), Some(&r), Some("  ")).unwrap();
        assert_eq!(
            (found.path, found.source, found.warning.as_deref()),
            (new, RepoSource::InstallReceipt, Some(UNKNOWN_INSTALLED))
        );
        assert!(found.installed_unknown);
    }

    #[test]
    fn only_a_successful_install_names_the_build() {
        let result = |ok: bool| -> InstallResult {
            serde_json::from_value(serde_json::json!({
                "id": "i1",
                "ok": ok,
                "finishedAt": "2026-10-20T10:00:00Z",
                "build": { "buildId": "b1", "workspaceId": "ws" },
            }))
            .unwrap()
        };
        assert_eq!(
            installed_build(result(true)).map(|b| b.workspace_id),
            Some("ws".into())
        );
        // A failed or refused install: its build isn't what is installed.
        assert_eq!(installed_build(result(false)), None);
        let mut none = result(true);
        none.build = None;
        assert_eq!(installed_build(none), None);
    }
}
