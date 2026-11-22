//! How a project starts (t3code parity 4.4, A7 and A9).
//!
//! A7: every quick chat has a scratch folder, `workspaces/quick-chats/scratch/<chat key>/`,
//! inside the folder its turns already run in. "Turn into project" carries what is in it
//! into the new project's `sources/chat-<key>/` and removes the empty scratch folder.
//! A9: clone a repository from a URL into the connected Projects folder, so the new-project
//! form can pick it. The URL is checked before git runs: `https://`, `ssh://` and the
//! `git@host:owner/repo` form only (no `file:`, `ext::`, plain `http://`, options,
//! credentials in the URL), and git runs with `GIT_ALLOW_PROTOCOL=https:ssh`.
use crate::{build_workspaces, process_group, workspaces};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// Folder name of the scratch area inside the Quick chats folder.
const SCRATCH: &str = "scratch";
/// A scratch folder moves at most this many files, each at most [`MAX_FILE`] bytes.
const MAX_FILES: usize = 500;
const MAX_FILE: u64 = 50 * 1024 * 1024;
const MAX_DEPTH: usize = 8;
/// A clone that takes longer than this is stopped and removed.
const CLONE_LIMIT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScratchInfo {
    pub path: String,
    /// The same folder relative to the vault (what Reveal in Finder takes).
    pub relative: String,
    pub files: u32,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptReport {
    /// Files now in the project.
    pub moved: u32,
    /// Files left in the scratch folder because the project already had one by that name.
    pub kept: u32,
    /// Where they went, relative to the project's folder (`sources/chat-<key>`).
    pub into: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClonedRepo {
    pub name: String,
    pub path: String,
}

fn key_ok(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 80
        && key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// The chat's scratch folder (not created).
fn scratch_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    if !key_ok(key) {
        return Err("That conversation key can't name a scratch folder".into());
    }
    let quick = workspaces::ensure_quick_chats(root)?;
    let dir = quick.join(SCRATCH).join(key);
    // A quick chat can write in its own folder, so a link there must not lead the move elsewhere.
    no_links(&[&quick.join(SCRATCH), &dir])?;
    Ok(dir)
}

/// Refuses a path that is a symbolic link (a path that doesn't exist yet is fine).
fn no_links(paths: &[&Path]) -> Result<(), String> {
    for p in paths {
        if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "{} is a symbolic link, so nothing moves through it",
                p.display()
            ));
        }
    }
    Ok(())
}

/// Removes empty folders under `dir` (and `dir` itself once empty). Never a file, never a
/// folder that still holds one.
fn remove_empty_dirs(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        if fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir()) {
            remove_empty_dirs(&e.path());
        }
    }
    let _ = fs::remove_dir(dir);
}

/// Files directly or deeply inside `dir`, without following symbolic links.
fn walk(dir: &Path, depth: usize, out: &mut Vec<(PathBuf, u64)>) {
    if depth > MAX_DEPTH || out.len() > MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let Ok(meta) = fs::symlink_metadata(e.path()) else {
            continue;
        };
        if meta.is_dir() {
            walk(&e.path(), depth + 1, out);
        } else if meta.is_file() {
            out.push((e.path(), meta.len()));
        }
    }
}

/// The chat's scratch folder and what is in it. `create` makes it first (the chat menu's
/// "Scratch folder"); without it a chat that never used one answers zero files and no
/// folder is made.
pub fn scratch_info(root: &Path, key: &str, create: bool) -> Result<ScratchInfo, String> {
    let dir = scratch_path(root, key)?;
    if create {
        fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make the scratch folder: {e}"))?;
    }
    let mut files = Vec::new();
    walk(&dir, 0, &mut files);
    Ok(ScratchInfo {
        path: dir.to_string_lossy().into_owned(),
        relative: format!("workspaces/{}/{SCRATCH}/{key}", workspaces::QUICK_CHATS_ID),
        files: files.len().min(MAX_FILES) as u32,
        bytes: files.iter().map(|f| f.1).sum(),
    })
}

/// Turn the chat's scratch files into project files: copy each into
/// `workspaces/<id>/sources/chat-<key>/` (never over an existing file), then remove what was
/// copied. A chat with no scratch folder moves nothing and is not an error.
pub fn adopt_scratch(root: &Path, key: &str, workspace_id: &str) -> Result<AdoptReport, String> {
    if workspaces::is_builtin(workspace_id) {
        return Err("Scratch files move into a project, not into a built-in workspace".into());
    }
    let from = scratch_path(root, key)?;
    let into_rel = format!("sources/chat-{key}");
    let project = workspaces::folder(root, workspace_id)?;
    if !project.join("WORKSPACE.md").is_file() {
        return Err("That project doesn't exist".into());
    }
    let mut files = Vec::new();
    walk(&from, 0, &mut files);
    if files.is_empty() {
        remove_empty_dirs(&from);
        return Ok(AdoptReport {
            moved: 0,
            kept: 0,
            into: into_rel,
        });
    }
    if files.len() > MAX_FILES {
        return Err(format!(
            "The scratch folder holds more than {MAX_FILES} files. Move the ones that matter into the project by hand."
        ));
    }
    let to = project.join(&into_rel);
    no_links(&[&project.join("sources"), &to])?;
    let (mut moved, mut kept) = (0u32, 0u32);
    let result = (|| -> Result<(), String> {
        for (path, size) in files {
            let rel = path.strip_prefix(&from).map_err(|e| e.to_string())?;
            let target = to.join(rel);
            if size > MAX_FILE || fs::symlink_metadata(&target).is_ok() {
                kept += 1;
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Couldn't make {}: {e}", parent.display()))?;
                no_links(&[parent])?;
            }
            // Copy to a private name, then link it to its final one: the link fails if that name
            // appeared meanwhile (nothing is overwritten), and a failed copy leaves no half file.
            let temp =
                target.with_file_name(format!(".hollow-copy-{}-{moved}", std::process::id()));
            if let Err(e) = fs::copy(&path, &temp) {
                let _ = fs::remove_file(&temp);
                return Err(format!("Couldn't copy {}: {e}", rel.display()));
            }
            let linked = fs::hard_link(&temp, &target);
            let _ = fs::remove_file(&temp);
            match linked {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    kept += 1;
                    continue;
                }
                Err(e) => return Err(format!("Couldn't place {}: {e}", rel.display())),
            }
            fs::remove_file(&path).map_err(|e| format!("Couldn't clear {}: {e}", rel.display()))?;
            moved += 1;
        }
        Ok(())
    })();
    crate::vault::invalidate_walk_cache();
    result?;
    // Only emptied folders go; anything the walk didn't see (deeper, written late) stays.
    remove_empty_dirs(&from);
    Ok(AdoptReport {
        moved,
        kept,
        into: into_rel,
    })
}

fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && !name.starts_with('.')
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

fn host_ok(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')
}

/// The folder name a URL implies: its last path segment without `.git`.
fn name_from(path: &str) -> String {
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    last.strip_suffix(".git").unwrap_or(last).to_string()
}

/// Whether `url` is a clone URL Hollow accepts, and the folder name it implies.
pub fn check_clone_url(url: &str) -> Result<String, String> {
    let bad = "Use an https:// or ssh:// address, or git@host:owner/repo";
    if url != url.trim() || url.is_empty() || url.len() > 300 {
        return Err(bad.into());
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) || url.starts_with('-') {
        return Err(bad.into());
    }
    let name = if let Some(rest) = url.strip_prefix("https://") {
        let (authority, path) = rest.split_once('/').ok_or(bad)?;
        // Credentials in a URL would be stored in the clone's git config.
        let host = authority.split_once(':').map_or(authority, |(h, port)| {
            if port.bytes().all(|c| c.is_ascii_digit()) && !port.is_empty() {
                h
            } else {
                ""
            }
        });
        if !host_ok(host) {
            return Err(bad.into());
        }
        name_from(path)
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/').ok_or(bad)?;
        let authority = authority.rsplit_once('@').map_or(authority, |(user, h)| {
            if user.is_empty() || !name_ok(user) {
                ""
            } else {
                h
            }
        });
        let host = authority.split_once(':').map_or(authority, |(h, port)| {
            if port.bytes().all(|c| c.is_ascii_digit()) && !port.is_empty() {
                h
            } else {
                ""
            }
        });
        if !host_ok(host) {
            return Err(bad.into());
        }
        name_from(path)
    } else if let Some(rest) = url.strip_prefix("git@") {
        let (host, path) = rest.split_once(':').ok_or(bad)?;
        if !host_ok(host) || path.is_empty() || path.starts_with('/') || path.starts_with('-') {
            return Err(bad.into());
        }
        name_from(path)
    } else {
        return Err(bad.into());
    };
    if !name_ok(&name) {
        return Err("That address doesn't name a repository".into());
    }
    Ok(name)
}

/// Clone `url` into `projects/<name>`. `name` overrides the folder the URL implies. The
/// folder must not exist yet; a failed or timed out clone leaves nothing behind.
pub async fn clone_repo(
    projects: &Path,
    url: &str,
    name: Option<&str>,
) -> Result<ClonedRepo, String> {
    let implied = check_clone_url(url)?;
    let name = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) if name_ok(n) => n.to_string(),
        Some(_) => {
            return Err("A folder name uses letters, digits, dots, dashes and underscores".into())
        }
        None => implied,
    };
    if !projects.is_dir() {
        return Err(format!("Couldn't read {}", projects.display()));
    }
    let dest = projects.join(&name);
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(format!("{name} already exists in your Projects folder"));
    }
    // Clone into a hidden folder of our own, then rename it into place: a failure removes only
    // that folder, never one somebody else made at `dest` in the meantime, and a quit mid-clone
    // leaves a dot folder the repository list skips.
    let temp_name = format!(
        ".hollow-clone-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    );
    let temp = projects.join(&temp_name);
    let mut cmd = build_workspaces::git_command(projects);
    cmd.env("GIT_ALLOW_PROTOCOL", "https:ssh")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .args(["clone", "--quiet", "--", url, &temp_name]);
    let out = process_group::run_to_completion_capped(&mut cmd, None, CLONE_LIMIT, None, 64 * 1024)
        .await
        .map_err(|e| format!("could not run git: {e}"))?;
    let ok = !out.timed_out && out.status.is_some_and(|s| s.success());
    if !ok {
        let _ = fs::remove_dir_all(&temp);
        if out.timed_out {
            return Err(format!(
                "The clone took longer than {} minutes and was stopped",
                CLONE_LIMIT.as_secs() / 60
            ));
        }
        return Err(build_workspaces::git_error(&["clone"], &out.stderr));
    }
    if fs::symlink_metadata(&dest).is_ok() {
        let _ = fs::remove_dir_all(&temp);
        return Err(format!("{name} already exists in your Projects folder"));
    }
    if let Err(e) = fs::rename(&temp, &dest) {
        let _ = fs::remove_dir_all(&temp);
        return Err(format!("Couldn't place the clone as {name}: {e}"));
    }
    Ok(ClonedRepo {
        name,
        path: dest.to_string_lossy().into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("intake-")
            .tempdir()
            .unwrap()
    }
    fn project(v: &Path) -> (String, PathBuf) {
        let w = workspaces::create(v, "Atlas", "Ship it").unwrap();
        let dir = v.join("workspaces").join(&w.id);
        (w.id, dir)
    }

    #[test]
    fn clone_urls_accept_the_three_safe_forms_and_name_the_folder() {
        for (url, name) in [
            ("https://github.com/example-user/hollow.git", "hollow"),
            ("https://github.com/example-user/hollow", "hollow"),
            ("https://git.example.com:8443/team/tool.git", "tool"),
            ("ssh://git@github.com/owner/repo.git", "repo"),
            ("ssh://git.example.com:2222/owner/repo", "repo"),
            ("git@github.com:owner/my-repo.git", "my-repo"),
        ] {
            assert_eq!(check_clone_url(url).as_deref(), Ok(name), "{url}");
        }
    }

    #[test]
    fn clone_urls_refuse_options_local_paths_helpers_credentials_and_plain_http() {
        for url in [
            "",
            " https://github.com/a/b",
            "--upload-pack=touch /tmp/x",
            "-u https://github.com/a/b",
            "http://github.com/a/b",
            "file:///etc/passwd",
            "/Users/me/secret",
            "ext::sh -c touch% /tmp/x",
            "https://user:token@github.com/a/b.git",
            "https://github.com",
            "https://github.com/a/b c",
            "https://-evil.com/a/b",
            "ssh://:pw@host/a/b",
            "git@host:",
            "git@host:/abs/path",
            "git@-host:a/b",
            "https://github.com/a/..",
            "https://github.com/a/.git",
            "https://github.com/a/%2e%2e",
        ] {
            assert!(check_clone_url(url).is_err(), "{url:?} should be refused");
        }
    }

    #[test]
    fn scratch_folders_are_per_chat_inside_quick_chats_and_made_only_on_request() {
        let v = vault();
        let none = scratch_info(v.path(), "s-1", false).unwrap();
        assert_eq!((none.files, none.bytes), (0, 0));
        assert!(!Path::new(&none.path).exists());
        let made = scratch_info(v.path(), "s-1", true).unwrap();
        assert!(Path::new(&made.path).is_dir());
        assert!(made.path.contains("quick-chats/scratch/s-1"));
        assert_eq!(made.relative, "workspaces/quick-chats/scratch/s-1");
        assert_ne!(
            made.path,
            scratch_info(v.path(), "s-2", false).unwrap().path
        );
        for bad in ["", "../x", "a/b", "a b"] {
            assert!(scratch_info(v.path(), bad, true).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn turn_into_project_moves_files_never_overwrites_and_clears_the_folder() {
        let v = vault();
        let (atlas, atlas_dir) = project(v.path());
        let dir = PathBuf::from(scratch_info(v.path(), "s-9", true).unwrap().path);
        fs::create_dir_all(dir.join("notes")).unwrap();
        fs::write(dir.join("draft.md"), "hello").unwrap();
        fs::write(dir.join("notes/a.txt"), "a").unwrap();
        let report = adopt_scratch(v.path(), "s-9", &atlas).unwrap();
        assert_eq!((report.moved, report.kept), (2, 0));
        assert_eq!(report.into, "sources/chat-s-9");
        let to = atlas_dir.join("sources/chat-s-9");
        assert_eq!(fs::read_to_string(to.join("draft.md")).unwrap(), "hello");
        assert!(to.join("notes/a.txt").is_file());
        assert!(!dir.exists(), "an emptied scratch folder is removed");
        // A second chat with the same file name keeps the project's copy.
        let dir2 = PathBuf::from(scratch_info(v.path(), "s-9", true).unwrap().path);
        fs::write(dir2.join("draft.md"), "different").unwrap();
        let again = adopt_scratch(v.path(), "s-9", &atlas).unwrap();
        assert_eq!((again.moved, again.kept), (0, 1));
        assert_eq!(fs::read_to_string(to.join("draft.md")).unwrap(), "hello");
        assert!(
            dir2.join("draft.md").is_file(),
            "a kept file stays where it was"
        );
    }

    #[test]
    fn turn_into_project_refuses_built_in_and_missing_projects_and_skips_symlinks() {
        let v = vault();
        assert!(adopt_scratch(v.path(), "s-1", workspaces::QUICK_CHATS_ID).is_err());
        assert!(adopt_scratch(v.path(), "s-1", "nope").is_err());
        let (atlas, _) = project(v.path());
        let dir = PathBuf::from(scratch_info(v.path(), "s-3", true).unwrap().path);
        let outside = v.path().join("outside.txt");
        fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link.txt")).unwrap();
        let report = adopt_scratch(v.path(), "s-3", &atlas).unwrap();
        assert_eq!(report.moved, 0);
        assert!(outside.is_file(), "a symlink never moves its target");
    }

    #[test]
    fn turn_into_project_never_follows_a_link_in_the_scratch_or_project_folders() {
        let v = vault();
        let (atlas, atlas_dir) = project(v.path());
        let victim = v.path().join("victim");
        fs::create_dir_all(&victim).unwrap();
        fs::write(victim.join("keep.txt"), "mine").unwrap();
        // scratch/<key> itself a link to a user folder
        let info = scratch_info(v.path(), "s-5", false).unwrap();
        fs::create_dir_all(Path::new(&info.path).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&victim, &info.path).unwrap();
        assert!(adopt_scratch(v.path(), "s-5", &atlas).is_err());
        assert!(scratch_info(v.path(), "s-5", true).is_err());
        assert!(
            victim.join("keep.txt").is_file(),
            "the linked folder's files are untouched"
        );
        // the project's sources folder a link to somewhere else
        let dir = PathBuf::from(scratch_info(v.path(), "s-6", true).unwrap().path);
        fs::write(dir.join("a.txt"), "a").unwrap();
        let _ = fs::remove_dir_all(atlas_dir.join("sources"));
        std::os::unix::fs::symlink(&victim, atlas_dir.join("sources")).unwrap();
        assert!(adopt_scratch(v.path(), "s-6", &atlas).is_err());
        assert!(dir.join("a.txt").is_file() && !victim.join("chat-s-6").exists());
    }

    #[test]
    fn turn_into_project_keeps_files_it_did_not_move() {
        let v = vault();
        let (atlas, _) = project(v.path());
        let dir = PathBuf::from(scratch_info(v.path(), "s-7", true).unwrap().path);
        let mut deep = dir.clone();
        for i in 0..=MAX_DEPTH + 1 {
            deep = deep.join(format!("d{i}"));
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("far.txt"), "far").unwrap();
        fs::write(dir.join("near.txt"), "near").unwrap();
        let report = adopt_scratch(v.path(), "s-7", &atlas).unwrap();
        assert_eq!(report.moved, 1);
        assert!(
            deep.join("far.txt").is_file(),
            "a file the walk never reached is not deleted"
        );
        assert!(!dir.join("near.txt").exists());
    }

    #[test]
    fn ssh_addresses_check_the_user_part_too() {
        assert!(check_clone_url("ssh://-oProxyCommand=x@host/a/b").is_err());
        assert!(check_clone_url("ssh://git@host.example/a/b").is_ok());
    }

    #[tokio::test]
    async fn a_failed_clone_leaves_no_folder_and_an_existing_name_is_refused() {
        let projects = tempfile::Builder::new().prefix("clone-").tempdir().unwrap();
        let err = clone_repo(projects.path(), "https://127.0.0.1:1/owner/repo.git", None)
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        assert!(!projects.path().join("repo").exists());
        fs::create_dir(projects.path().join("taken")).unwrap();
        let err = clone_repo(
            projects.path(),
            "https://127.0.0.1:1/owner/repo.git",
            Some("taken"),
        )
        .await
        .unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(
            clone_repo(projects.path(), "https://127.0.0.1:1/a/b", Some("../x"))
                .await
                .is_err()
        );
    }
}
