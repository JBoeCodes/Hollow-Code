//! Workspace icons: the project's own app artwork (a favicon or app icon in
//! the repository) and the GitHub owner's avatar, so a workspace row shows
//! at a glance whether it is an app, an organization repo or a personal one.
//!
//! Read-only and local: the remote is parsed from `.git/config` directly (no
//! git process runs), artwork is read only from a fixed list of paths inside
//! the project, never through a symlink that leaves it, and is returned as a
//! `data:` URL. The avatar is a `https://github.com/<owner>.png` URL the
//! webview loads itself (and falls back from when offline).
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

/// Artwork paths, most specific first. Small Tauri icons come before the
/// large `icon.png`, which often exceeds the size cap.
const ARTWORK: &[&str] = &[
    "public/apple-touch-icon.png",
    "apple-touch-icon.png",
    "public/favicon.svg",
    "favicon.svg",
    "public/favicon.png",
    "public/icon.png",
    "public/logo.png",
    "favicon.png",
    "app/icon.png",
    "src/app/icon.png",
    "src/app/icon.svg",
    "app/icon.svg",
    "public/favicon.ico",
    "favicon.ico",
    "app/favicon.ico",
    "src/app/favicon.ico",
    "static/favicon.png",
    "static/favicon.ico",
    "src-tauri/icons/128x128.png",
    "src-tauri/icons/icon.png",
    "assets/icon.png",
    "src/assets/icon.png",
    "resources/icon.png",
    "build/icon.png",
];
const MAX_ARTWORK: u64 = 512 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIcon {
    pub workspace_id: String,
    /// The project's own artwork as a `data:` URL.
    pub image_url: Option<String>,
    /// The GitHub owner (user or organization) of the `origin` remote.
    pub owner: Option<String>,
    /// That owner's avatar.
    pub avatar_url: Option<String>,
}

/// Owner of a GitHub remote URL (`git@github.com:o/r.git`,
/// `https://github.com/o/r`, `ssh://git@github.com/o/r.git`). Anything else,
/// or a name GitHub would not allow, is None.
pub fn github_owner(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let path = if let Some(rest) = remote.strip_prefix("git@github.com:") {
        rest.to_string()
    } else {
        let url = reqwest::Url::parse(remote).ok()?;
        if url.host_str()? != "github.com"
            || !matches!(url.scheme(), "https" | "http" | "ssh" | "git")
        {
            return None;
        }
        url.path().trim_start_matches('/').to_string()
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    if parts.next().is_some() || owner.is_empty() || repo.is_empty() || owner.len() > 39 {
        return None;
    }
    let owner_ok = owner
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'-');
    let repo_ok = repo
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c));
    (owner_ok && repo_ok).then(|| owner.to_string())
}

/// `remote "origin"`'s url from a git config file's text.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            let header: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            in_origin = header.eq_ignore_ascii_case("[remote\"origin\"]");
            continue;
        }
        if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim().eq_ignore_ascii_case("url") {
                    return Some(value.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    None
}

fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let file = fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > cap {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= cap).then_some(bytes)
}

pub(crate) fn owner_of(root: &Path) -> Option<String> {
    let git = root.join(".git");
    // A plain `.git` folder only: linked worktrees are not connected projects.
    if !fs::symlink_metadata(&git).ok()?.is_dir() {
        return None;
    }
    let text = String::from_utf8(read_capped(&git.join("config"), 256 * 1024)?).ok()?;
    github_owner(&origin_url(&text)?)
}

fn artwork(root: &Path) -> Option<String> {
    for name in ARTWORK {
        let Ok(path) = root.join(name).canonicalize() else {
            continue;
        };
        if !path.starts_with(root) {
            continue;
        }
        let Some(bytes) = read_capped(&path, MAX_ARTWORK) else {
            continue;
        };
        let mime = if name.ends_with(".png") && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if name.ends_with(".ico") && bytes.starts_with(&[0, 0, 1, 0]) {
            "image/x-icon"
        } else if name.ends_with(".svg")
            && std::str::from_utf8(&bytes).is_ok_and(|s| s.contains("<svg"))
        {
            // Shown only through <img>, where SVG scripts never run.
            "image/svg+xml"
        } else {
            continue;
        };
        return Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
    }
    None
}

// Keep stamps independent: a future-dated config must not hide a changed
// nested icon. Candidate paths also detect a newly added higher-priority icon.
type IconStamp = Vec<Option<(PathBuf, Option<SystemTime>, u64)>>;
type IconValue = (Option<String>, Option<String>);

/// Artwork and owner cached by each artwork/config modification. Artwork must
/// stay contained; a linked config follows the existing owner_of read policy.
fn project_icon(root: &Path) -> (Option<String>, Option<String>) {
    static CACHE: Mutex<Option<HashMap<PathBuf, (IconStamp, IconValue)>>> = Mutex::new(None);
    let Ok(root) = root.canonicalize() else {
        return (None, None);
    };
    let stamp: IconStamp = std::iter::once(".git/config")
        .chain(ARTWORK.iter().copied())
        .map(|name| {
            let path = root.join(name).canonicalize().ok()?;
            if name != ".git/config" && !path.starts_with(&root) {
                return None;
            }
            let meta = fs::metadata(&path).ok()?;
            Some((path, meta.modified().ok(), meta.len()))
        })
        .collect();
    if let Ok(mut guard) = CACHE.lock() {
        let cache = guard.get_or_insert_with(HashMap::new);
        if let Some((cached, value)) = cache.get(&root) {
            if *cached == stamp {
                return value.clone();
            }
        }
        let value = (artwork(&root), owner_of(&root));
        cache.insert(root, (stamp, value.clone()));
        return value;
    }
    (artwork(&root), owner_of(&root))
}

fn name_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Icons for every workspace that maps to a connected project: `local-`
/// workspaces by id, and any other workspace whose title matches exactly one
/// connected project's name (a Notion project named like its repository).
pub fn list(vault: &Path) -> Vec<WorkspaceIcon> {
    let Ok(workspaces) = crate::workspaces::list(vault) else {
        return Vec::new();
    };
    let projects = crate::directory_connections::projects(vault);
    let mut by_name: HashMap<String, Vec<&crate::directory_connections::Project>> = HashMap::new();
    for project in &projects {
        by_name
            .entry(name_key(&project.title))
            .or_default()
            .push(project);
    }
    let mut out = Vec::new();
    for workspace in workspaces {
        let root = projects
            .iter()
            .find(|p| p.id == workspace.id)
            .or_else(
                || match by_name.get(&name_key(&workspace.title)).map(Vec::as_slice) {
                    Some([only]) => Some(*only),
                    _ => None,
                },
            )
            .map(|p| PathBuf::from(&p.path));
        let Some(root) = root else { continue };
        let (image_url, owner) = project_icon(&root);
        if image_url.is_none() && owner.is_none() {
            continue;
        }
        out.push(WorkspaceIcon {
            workspace_id: workspace.id,
            avatar_url: owner
                .as_ref()
                .map(|o| format!("https://github.com/{o}.png?size=64")),
            image_url,
            owner,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_owner_accepts_github_remotes_only() {
        assert_eq!(
            github_owner("git@github.com:example-org/hollow-code.git").as_deref(),
            Some("example-org")
        );
        assert_eq!(
            github_owner("https://github.com/example-org/app-hub").as_deref(),
            Some("example-org")
        );
        assert_eq!(
            github_owner("ssh://git@github.com/o/r.git").as_deref(),
            Some("o")
        );
        assert_eq!(github_owner("https://gitlab.com/o/r"), None);
        assert_eq!(github_owner("https://github.com/o"), None);
        assert_eq!(github_owner("https://github.com/o/r/extra"), None);
        assert_eq!(github_owner("https://github.com/bad_owner/r"), None);
        assert_eq!(github_owner("file:///tmp/r"), None);
    }

    #[test]
    fn origin_url_reads_only_the_origin_section() {
        let config = "[core]\n\turl = nope\n[remote \"upstream\"]\n\turl = https://github.com/a/b\n[remote \"origin\"]\n\turl = git@github.com:me/app.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n";
        assert_eq!(
            origin_url(config).as_deref(),
            Some("git@github.com:me/app.git")
        );
        assert_eq!(origin_url("[remote \"upstream\"]\nurl = x\n"), None);
    }

    #[test]
    fn artwork_prefers_listed_order_and_stays_inside_the_project() {
        let dir = tempfile::Builder::new().prefix("proj-").tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("public")).unwrap();
        fs::write(
            root.join("public/favicon.svg"),
            "<svg xmlns='http://www.w3.org/2000/svg'/>",
        )
        .unwrap();
        fs::write(root.join("favicon.ico"), [0u8, 0, 1, 0, 1]).unwrap();
        assert!(artwork(&root)
            .unwrap()
            .starts_with("data:image/svg+xml;base64,"));
        // A file with the wrong magic bytes is skipped.
        fs::write(root.join("public/favicon.svg"), "not an svg").unwrap();
        assert!(artwork(&root)
            .unwrap()
            .starts_with("data:image/x-icon;base64,"));
        // A symlink out of the project is never read.
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("logo.png"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        fs::remove_file(root.join("favicon.ico")).unwrap();
        fs::remove_file(root.join("public/favicon.svg")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("logo.png"), root.join("favicon.png"))
            .unwrap();
        assert_eq!(artwork(&root), None);
    }

    #[test]
    fn replacing_resolved_artwork_invalidates_cache_independently_of_config_time() {
        let dir = tempfile::Builder::new().prefix("proj-").tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("src-tauri/icons")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        let artwork = root.join("src-tauri/icons/128x128.png");
        let config = root.join(".git/config");
        fs::write(
            &config,
            "[remote \"origin\"]\nurl = https://github.com/example/app.git\n",
        )
        .unwrap();
        fs::write(&artwork, b"\x89PNG\r\n\x1a\nfirst").unwrap();
        let stamp =
            |path: &Path, seconds| {
                fs::File::open(path)
                    .unwrap()
                    .set_times(fs::FileTimes::new().set_modified(
                        std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
                    ))
                    .unwrap();
            };
        stamp(&root, 100);
        stamp(&config, 300);
        stamp(&artwork, 200);
        let first = project_icon(&root);
        assert!(first.0.is_some());
        fs::write(&artwork, b"\x89PNG\r\n\x1a\nother").unwrap();
        stamp(&artwork, 201);
        let second = project_icon(&root);
        assert_ne!(first.0, second.0);
        assert_eq!(first.1, second.1);
    }

    #[test]
    fn linked_config_changes_refresh_the_owner() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir_all(root.join(".git")).unwrap();
        let config = dir.path().join("shared-config");
        fs::write(
            &config,
            "[remote \"origin\"]\nurl = https://github.com/first/app.git\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&config, root.join(".git/config")).unwrap();
        assert_eq!(project_icon(&root).1.as_deref(), Some("first"));
        fs::write(
            &config,
            "[remote \"origin\"]\nurl = https://github.com/second-owner/app.git\n",
        )
        .unwrap();
        assert_eq!(project_icon(&root).1.as_deref(), Some("second-owner"));
    }

    #[test]
    fn owner_comes_from_the_git_config_of_a_real_repository() {
        let dir = tempfile::Builder::new().prefix("proj-").tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://github.com/example-user/portal.git\n",
        )
        .unwrap();
        assert_eq!(owner_of(root).as_deref(), Some("example-user"));
        // A gitfile (linked worktree) has no owner here.
        let wt = tempfile::Builder::new().prefix("wt-").tempdir().unwrap();
        fs::write(wt.path().join(".git"), "gitdir: /elsewhere").unwrap();
        assert_eq!(owner_of(wt.path()), None);
    }
}
