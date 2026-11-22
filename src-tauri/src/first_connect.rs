//! Describe-only first connect (BI7, 2026-11-18; idea from beancount-io's
//! `bea init --existing`, which writes an agent file from an existing ledger,
//! lists what needs attention and changes no file).
//!
//! When a build workspace is created from an existing repository
//! (`build_workspaces::create_with`), Hollow writes a card draft,
//! [`DRAFT_FILE`], into the new workspace's `outputs/` folder in the vault,
//! never into the repository. The draft holds what Hollow can learn by
//! reading only: manifests, the README's title, agent files, the top-level
//! layout, how-to-run hints, CI workflow names, and a "Needs attention" list
//! where every item names a next step. The host calls no model: the
//! project card check (`project_card.rs`) points the agent at the draft while
//! the card is missing or not fresh, and the agent writes the card with the
//! card prompt.
//!
//! **The proof.** A [`Fingerprint`] of the repository is taken just before
//! `git worktree add` (after `git fetch`) and again after the facts are
//! read; the draft records both digests and says "Nothing changed" only when
//! they match. When they differ it says so at the top, names up to ten
//! paths, and the create returns a warning. The fingerprint covers every
//! entry under the repository folder outside `.git` folders (tracked,
//! untracked and ignored files alike), plus the repository's own
//! [`GIT_KEPT`] git files (`index`, `HEAD`, `config`, `hooks/`): relative
//! path, type, size, modification time in nanoseconds, permission bits, the
//! status-change time and inode, and a symlink's target. The rest of `.git`
//! is left out because the connect writes it by design (the worktree's
//! folder, the new branch's ref and log, fetched objects). Contents are not
//! hashed (a repository with `node_modules` and `target` would take
//! minutes); a write moves the change time even when it restores the
//! modification time. More than [`ENTRY_MAX`] entries: the proof says it
//! was not taken.
//!
//! **What ran.** The describe step reads files directly, and its one git
//! process is `git --no-optional-locks -c core.fsmonitor=false status
//! --porcelain`, through `build_workspaces::git_raw`: `--no-optional-locks`
//! keeps status from refreshing `.git/index`, and the fsmonitor setting
//! keeps it from starting a daemon. The connect around it is not read-only
//! in the same way: it runs `git fetch` (with an origin), `rev-parse` and
//! `git worktree add`, which run the repository's own hooks and LFS filters,
//! and it may clone `node_modules` from the repository into the new
//! worktree. The draft says so, and a change inside the window is reported
//! as happening during the connect, possibly from those hooks.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The draft's path inside its workspace folder.
pub(crate) const DRAFT_FILE: &str = "outputs/PROJECT_CARD_DRAFT.md";
/// The draft's file name, beside the card.
pub(crate) const DRAFT_NAME: &str = "PROJECT_CARD_DRAFT.md";
/// Entries a fingerprint walks at most before it gives up.
pub(crate) const ENTRY_MAX: usize = 400_000;
/// Changed paths the draft names at most.
const CHANGED_SHOWN: usize = 10;
/// Bytes read from one manifest or README at most.
const READ_MAX: u64 = 256 * 1024;
/// Top-level entries the layout lists at most.
const LAYOUT_SHOWN: usize = 40;

/// A tree fingerprint of a repository folder (see the module doc).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    /// Relative path (`""` for the folder itself) to its metadata line.
    entries: BTreeMap<String, String>,
    /// `sha256:<hex>` over every entry, in path order.
    pub(crate) digest: String,
    /// The walk stopped at [`ENTRY_MAX`]: the digest proves nothing.
    pub(crate) truncated: bool,
}

impl Fingerprint {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn mode_of(meta: &fs::Metadata) -> u32 {
    u32::from(meta.permissions().readonly())
}

/// One entry's metadata line: type, size, modification time, mode, and a
/// symlink's target. Never follows a link.
fn entry_line(path: &Path, meta: &fs::Metadata) -> String {
    let kind = if meta.is_dir() {
        "d"
    } else if meta.is_file() {
        "f"
    } else if meta.file_type().is_symlink() {
        "l"
    } else {
        "o"
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let target = if kind == "l" {
        fs::read_link(path)
            .map(|t| t.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!(
        "{kind} {} {mtime} {:o} {} {target}",
        meta.len(),
        mode_of(meta),
        change_of(meta)
    )
}

/// The status-change time (ns) and inode: a rename over the file, a
/// `chmod` or a write that restores the old mtime still moves these.
#[cfg(unix)]
fn change_of(meta: &fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!("{}.{:09} {}", meta.ctime(), meta.ctime_nsec(), meta.ino())
}

#[cfg(not(unix))]
fn change_of(_meta: &fs::Metadata) -> String {
    String::new()
}

/// The git files the proof covers ([`fingerprint`]): what a read-only
/// describe must never write (`index` is what a plain `git status`
/// refreshes) and what runs code (`config`, `hooks/`). Left out on purpose,
/// because the connect writes them by design: `worktrees/` and the new
/// branch's `refs/` and `logs/` (`git worktree add`), `objects/`,
/// `FETCH_HEAD`, `packed-refs` and remote refs (`git fetch`).
pub(crate) const GIT_KEPT: [&str; 4] = ["index", "HEAD", "config", "hooks"];

/// The repository's git folder and common folder: `.git` itself, or for a
/// linked worktree the folder its `.git` file names and that folder's
/// `commondir`. `None` when there is no readable `.git`.
fn git_dirs(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let dot = root.join(".git");
    let meta = fs::symlink_metadata(&dot).ok()?;
    let git_dir = if meta.is_dir() {
        dot
    } else if meta.is_file() {
        let text = read_small(&dot)?;
        root.join(text.trim().strip_prefix("gitdir:")?.trim())
    } else {
        return None;
    };
    let common = read_small(&git_dir.join("commondir"))
        .map(|c| git_dir.join(c.trim()))
        .unwrap_or_else(|| git_dir.clone());
    Some((git_dir, common))
}

/// `path`'s line (or `- absent`) under `key`, never following a link.
fn record(entries: &mut BTreeMap<String, String>, key: String, path: &Path) {
    let line = match fs::symlink_metadata(path) {
        Ok(meta) => entry_line(path, &meta),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "- absent".to_string(),
        Err(_) => "? unreadable".to_string(),
    };
    entries.insert(key, line);
}

/// The fingerprint of `root` now, by metadata only (no file is opened):
/// every entry outside `.git` folders (a linked worktree's `.git` file
/// included), plus the repository's own [`GIT_KEPT`] git files under
/// `(git) ` keys. Unreadable entries are recorded as such, so the digest
/// stays deterministic.
pub(crate) fn fingerprint(root: &Path) -> Fingerprint {
    let mut entries = BTreeMap::new();
    let mut truncated = false;
    if let Some((git_dir, common)) = git_dirs(root) {
        for name in GIT_KEPT {
            // A linked worktree keeps its own index and HEAD; config and
            // hooks are shared in the common folder.
            let base = if matches!(name, "index" | "HEAD") {
                &git_dir
            } else {
                &common
            };
            let top = base.join(name);
            for item in walkdir::WalkDir::new(&top).follow_links(false) {
                let Ok(e) = item else { continue };
                let rel = e.path().strip_prefix(base).unwrap_or(e.path());
                record(
                    &mut entries,
                    format!("(git) {}", rel.to_string_lossy()),
                    e.path(),
                );
            }
            // Absent or unwalkable: still one line, so it can't vanish.
            entries
                .entry(format!("(git) {name}"))
                .or_insert_with(|| "- absent".to_string());
        }
    }
    let walk = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0 || e.file_name() != ".git" || (e.depth() == 1 && !e.file_type().is_dir())
        });
    for item in walk {
        if entries.len() >= ENTRY_MAX {
            truncated = true;
            break;
        }
        let (path, line) = match item {
            Ok(e) => {
                let line = match e.metadata() {
                    Ok(meta) => entry_line(e.path(), &meta),
                    Err(_) => "? unreadable".to_string(),
                };
                (e.into_path(), line)
            }
            Err(err) => match err.path() {
                Some(p) => (p.to_path_buf(), "? unreadable".to_string()),
                None => continue,
            },
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        entries.insert(rel, line);
    }
    let mut hasher = Sha256::new();
    for (path, line) in &entries {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    Fingerprint {
        entries,
        digest: format!("sha256:{:x}", hasher.finalize()),
        truncated,
    }
}

/// Paths whose entry differs between `a` and `b` (added, removed or
/// changed), in path order, at most `max`, plus how many differ in all.
fn changed_paths(a: &Fingerprint, b: &Fingerprint, max: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut total = 0;
    let keys: std::collections::BTreeSet<&String> =
        a.entries.keys().chain(b.entries.keys()).collect();
    for key in keys {
        if a.entries.get(key) != b.entries.get(key) {
            total += 1;
            if out.len() < max {
                out.push(if key.is_empty() {
                    "(the repository folder itself)".to_string()
                } else {
                    key.clone()
                });
            }
        }
    }
    (out, total)
}

/// What the before and after fingerprints say.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Proof {
    /// Both digests match.
    Unchanged,
    /// They differ: up to [`CHANGED_SHOWN`] paths and the count.
    Changed { paths: Vec<String>, total: usize },
    /// A walk hit [`ENTRY_MAX`]: nothing proven either way.
    TooLarge,
}

pub(crate) fn compare(before: &Fingerprint, after: &Fingerprint) -> Proof {
    if before.truncated || after.truncated {
        return Proof::TooLarge;
    }
    if before.digest == after.digest {
        return Proof::Unchanged;
    }
    let (paths, total) = changed_paths(before, after, CHANGED_SHOWN);
    Proof::Changed { paths, total }
}

/// One "Needs attention" item: what Hollow saw and the next step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Attention {
    pub(crate) what: String,
    pub(crate) next: String,
}

/// What reading the repository found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Description {
    pub(crate) facts: Vec<String>,
    pub(crate) run: Vec<String>,
    pub(crate) attention: Vec<Attention>,
}

/// A regular file of at most [`READ_MAX`] bytes, never through a symlink.
fn read_small(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > READ_MAX {
        return None;
    }
    fs::read_to_string(path).ok()
}

fn is_file(dir: &Path, name: &str) -> bool {
    fs::symlink_metadata(dir.join(name)).is_ok_and(|m| m.is_file())
}

fn is_dir(dir: &Path, name: &str) -> bool {
    fs::symlink_metadata(dir.join(name)).is_ok_and(|m| m.is_dir())
}

/// Top-level names, folders with a trailing `/`, hidden ones left out
/// except `.github`.
fn top_level(repo: &Path) -> Vec<String> {
    let Ok(read) = fs::read_dir(repo) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name != ".github" {
                return None;
            }
            let dir = e.file_type().is_ok_and(|t| t.is_dir());
            Some(if dir { format!("{name}/") } else { name })
        })
        .collect();
    names.sort();
    names
}

/// The README's file name and first `# ` heading, if there is one.
fn readme(repo: &Path, names: &[String]) -> Option<(String, Option<String>)> {
    let name = names
        .iter()
        .find(|n| {
            let lower = n.to_ascii_lowercase();
            !n.ends_with('/') && (lower == "readme" || lower.starts_with("readme."))
        })?
        .clone();
    let title = read_small(&repo.join(&name)).and_then(|text| {
        text.lines()
            .map(str::trim)
            .find(|l| l.starts_with("# "))
            .map(|l| {
                l.trim_start_matches("# ")
                    .trim()
                    .chars()
                    .take(120)
                    .collect()
            })
    });
    Some((name, title))
}

/// The package manager a lockfile names, for the run hints.
fn node_runner(repo: &Path) -> (&'static str, Option<&'static str>) {
    for (lock, runner) in [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
        ("package-lock.json", "npm"),
        ("npm-shrinkwrap.json", "npm"),
    ] {
        if is_file(repo, lock) {
            return (runner, Some(lock));
        }
    }
    ("npm", None)
}

/// What the repository says by reading its files only. `uncommitted` is the
/// count of paths `git status` listed (or why it could not run).
pub(crate) fn describe(repo: &Path, uncommitted: Result<usize, String>) -> Description {
    let mut d = Description::default();
    let names = top_level(repo);
    let mut manifests = Vec::new();
    let mut has_test_script = false;

    // package.json: name and scripts.
    let mut node_lock_missing = false;
    if let Some(text) = read_small(&repo.join("package.json")) {
        let (runner, lock) = node_runner(repo);
        node_lock_missing = lock.is_none();
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(json) => {
                let name = json["name"].as_str().unwrap_or("(no name)");
                let scripts: Vec<String> = json["scripts"]
                    .as_object()
                    .map(|m| m.keys().cloned().collect())
                    .unwrap_or_default();
                has_test_script = scripts.iter().any(|s| s == "test");
                manifests.push(format!(
                    "`package.json`: package `{name}`{}",
                    if scripts.is_empty() {
                        ", no scripts".to_string()
                    } else {
                        let shown: Vec<&str> =
                            scripts.iter().take(12).map(String::as_str).collect();
                        format!(
                            ", scripts {}{}",
                            shown.join(", "),
                            if scripts.len() > 12 { " and more" } else { "" }
                        )
                    }
                ));
                if lock.is_some() || !scripts.is_empty() {
                    d.run.push(format!("`{runner} install`"));
                }
                for s in ["dev", "start", "build", "test", "lint"] {
                    if scripts.iter().any(|k| k == s) {
                        d.run.push(format!("`{runner} run {s}`"));
                    }
                }
            }
            Err(e) => manifests.push(format!("`package.json` (not valid JSON: {e})")),
        }
    }
    // Cargo.toml: package name or workspace members.
    let mut cargo_lock_missing = false;
    if let Some(text) = read_small(&repo.join("Cargo.toml")) {
        let value = text.parse::<toml::Table>().ok();
        let name = value
            .as_ref()
            .and_then(|t| t.get("package")?.get("name")?.as_str().map(String::from));
        let members = value
            .as_ref()
            .and_then(|t| t.get("workspace")?.get("members")?.as_array().map(Vec::len));
        manifests.push(match (name, members) {
            (Some(n), _) => format!("`Cargo.toml`: crate `{n}`"),
            (None, Some(m)) => format!("`Cargo.toml`: a workspace of {m} members"),
            (None, None) => "`Cargo.toml`".to_string(),
        });
        d.run.push("`cargo build`".into());
        d.run.push("`cargo test`".into());
        cargo_lock_missing = !is_file(repo, "Cargo.lock") && is_file(repo, "src/main.rs");
    }
    if let Some(text) = read_small(&repo.join("pyproject.toml")) {
        let name = text.parse::<toml::Table>().ok().and_then(|t| {
            t.get("project")
                .and_then(|p| p.get("name"))
                .or_else(|| t.get("tool")?.get("poetry")?.get("name"))
                .and_then(|n| n.as_str().map(String::from))
        });
        manifests.push(name.map_or_else(
            || "`pyproject.toml`".to_string(),
            |n| format!("`pyproject.toml`: project `{n}`"),
        ));
    }
    if let Some(text) = read_small(&repo.join("go.mod")) {
        let module = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("module "))
            .map(str::trim)
            .unwrap_or("(no module line)");
        manifests.push(format!("`go.mod`: module `{module}`"));
        d.run.push("`go build ./...`".into());
        d.run.push("`go test ./...`".into());
    }
    for name in ["Makefile", "makefile", "GNUmakefile"] {
        if let Some(text) = read_small(&repo.join(name)) {
            let targets: Vec<&str> = text
                .lines()
                .filter_map(|l| {
                    let (t, _) = l.split_once(':')?;
                    (!t.is_empty()
                        && !t.starts_with('.')
                        && !l.contains(":=")
                        && t.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)))
                    .then_some(t)
                })
                .take(10)
                .collect();
            manifests.push(format!(
                "`{name}`{}",
                if targets.is_empty() {
                    String::new()
                } else {
                    format!(": targets {}", targets.join(", "))
                }
            ));
            has_test_script |= targets.contains(&"test");
            for t in targets
                .iter()
                .filter(|t| ["build", "test", "run"].contains(t))
            {
                d.run.push(format!("`make {t}`"));
            }
            break;
        }
    }
    for other in [
        "Gemfile",
        "composer.json",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "requirements.txt",
        "Dockerfile",
    ] {
        if is_file(repo, other) {
            manifests.push(format!("`{other}`"));
        }
    }
    if manifests.is_empty() {
        d.facts
            .push("Manifests: none recognized at the top level.".into());
    } else {
        d.facts
            .push(format!("Manifests: {}.", manifests.join("; ")));
    }

    let readme = readme(repo, &names);
    match &readme {
        Some((name, Some(title))) => d
            .facts
            .push(format!("README: `{name}`, titled \"{title}\".")),
        Some((name, None)) => d.facts.push(format!("README: `{name}` (no `# ` title).")),
        None => {}
    }
    let agent_files: Vec<&str> = ["CLAUDE.md", "AGENTS.md"]
        .into_iter()
        .filter(|f| is_file(repo, f))
        .collect();
    if !agent_files.is_empty() {
        d.facts.push(format!(
            "Agent files: {}.",
            agent_files
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let lockfiles: Vec<&str> = [
        "package-lock.json",
        "npm-shrinkwrap.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lockb",
        "bun.lock",
        "Cargo.lock",
        "poetry.lock",
        "uv.lock",
        "go.sum",
        "Gemfile.lock",
        "composer.lock",
    ]
    .into_iter()
    .filter(|f| is_file(repo, f))
    .collect();
    if !lockfiles.is_empty() {
        d.facts
            .push(format!("Lockfiles: {}.", lockfiles.join(", ")));
    }
    if is_dir(repo, ".github") && is_dir(repo, ".github/workflows") {
        let mut flows: Vec<String> = fs::read_dir(repo.join(".github/workflows"))
            .map(|r| {
                r.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
                    .collect()
            })
            .unwrap_or_default();
        flows.sort();
        if !flows.is_empty() {
            let more = flows.len().saturating_sub(8);
            flows.truncate(8);
            d.facts.push(format!(
                "CI workflows (`.github/workflows/`): {}{}.",
                flows.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ));
        }
    }
    if !names.is_empty() {
        let more = names.len().saturating_sub(LAYOUT_SHOWN);
        let shown: Vec<&str> = names
            .iter()
            .take(LAYOUT_SHOWN)
            .map(String::as_str)
            .collect();
        d.facts.push(format!(
            "Top level: {}{}.",
            shown.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ));
    }

    // Needs attention: a small, honest set, each with a next step.
    if readme.is_none() {
        d.attention.push(Attention {
            what: "No README at the top level.".into(),
            next: "write the card's Purpose from the code alone, and suggest a README to the person (do not add one unasked).".into(),
        });
    }
    if agent_files.is_empty() {
        d.attention.push(Attention {
            what: "No `CLAUDE.md` or `AGENTS.md`: agents get no project rules.".into(),
            next: "once the card is fresh, offer to draft `AGENTS.md` from it for the person to review.".into(),
        });
    }
    let tests_folder = ["tests", "test", "__tests__", "spec", "e2e"]
        .into_iter()
        .find(|t| is_dir(repo, t));
    let rust = is_file(repo, "Cargo.toml");
    if tests_folder.is_none() && !has_test_script && !rust {
        d.attention.push(Attention {
            what: "No tests folder or `test` script at the top level.".into(),
            next: "find where tests live (or confirm there are none) before relying on a change, and say so in the card's Risks.".into(),
        });
    }
    if manifests.is_empty() {
        d.attention.push(Attention {
            what: "No manifest recognized, so Hollow has no run hints.".into(),
            next: "take How to run from the README or scripts, and ask the person if neither says."
                .into(),
        });
    }
    if node_lock_missing {
        d.attention.push(Attention {
            what: "`package.json` without a lockfile: installs are not reproducible.".into(),
            next: "say so in the card's Risks; committing a lockfile is the person's call.".into(),
        });
    }
    if cargo_lock_missing {
        d.attention.push(Attention {
            what: "A Rust binary (`src/main.rs`) without `Cargo.lock`.".into(),
            next: "say so in the card's Risks; a binary normally commits its lockfile.".into(),
        });
    }
    match uncommitted {
        Ok(0) => {}
        Ok(n) => d.attention.push(Attention {
            what: format!(
                "{n} uncommitted or untracked path{} in the repository's own checkout.",
                if n == 1 { "" } else { "s" }
            ),
            next: "the workspace's worktree starts from the base branch, so this work is not in it; ask the person whether it should be committed first.".into(),
        }),
        Err(e) => d.attention.push(Attention {
            what: format!("Could not read `git status` ({e})."),
            next: "check the repository in a terminal before trusting its state.".into(),
        }),
    }
    d
}

/// The draft's Markdown.
pub(crate) fn render(
    repo: &Path,
    commit: Option<&str>,
    written: &str,
    d: &Description,
    before: &Fingerprint,
    after: &Fingerprint,
) -> String {
    let proof = compare(before, after);
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string());
    let mut out = format!(
        "---\nkind: project-card-draft\nrepo: {}\ncommit: {}\nwritten: {written}\nfingerprint_before: {}\nfingerprint_after: {}\nunchanged: {}\n---\n\n# First-connect draft: {name}\n\n",
        repo.display(),
        commit.unwrap_or("unknown"),
        before.digest,
        after.digest,
        proof == Proof::Unchanged,
    );
    let span = format!(
        "before {} ({} entries), after {} ({} entries)",
        before.digest,
        before.len(),
        after.digest,
        after.len()
    );
    match &proof {
        Proof::Unchanged => out.push_str(&format!(
            "Nothing changed in `{}`: fingerprint {span}.\n\n",
            repo.display()
        )),
        Proof::Changed { paths, total } => out.push_str(&format!(
            "**CHANGED: {total} path{} in `{}` differ between the fingerprints** ({span}). The change happened during the connect, between just before `git worktree add` and the end of the description: possibly from the repository's own hooks or filters that `git worktree add` ran, or from another program writing to it (an editor, a build, another agent). Check these before trusting this draft:\n{}\n\n",
            if *total == 1 { "" } else { "s" },
            repo.display(),
            paths
                .iter()
                .map(|p| format!("- `{p}`"))
                .collect::<Vec<_>>()
                .join("\n")
        )),
        Proof::TooLarge => out.push_str(&format!(
            "Not proven: the repository has over {ENTRY_MAX} entries, so the fingerprint stopped ({span}).\n\n"
        )),
    }
    out.push_str("Hollow wrote this when the repository was connected to this workspace. The description step read files only and ran git once, as `git --no-optional-locks status`; no build, install or package script ran. The connect around it also ran `git fetch` (when there is an origin) and `git worktree add`, which run the repository's own hooks and LFS filters, and may have cloned `node_modules` from it into the new worktree. The fingerprints were taken just before `git worktree add` and after the description; they cover every file and folder outside `.git` (tracked, untracked and ignored) and `.git`'s `index`, `HEAD`, `config` and `hooks/`, by path, type, size, modification and change times, inode and permissions; contents are not hashed. This is a lead, not the card: write `outputs/PROJECT_CARD.md` with the card prompt (Hollow's `workspaces.project_card` action), checking each fact below in the code.\n\n## Facts read\n\n");
    for f in &d.facts {
        out.push_str(&format!("- {f}\n"));
    }
    out.push_str("\n## How to run (hints, not run)\n\n");
    if d.run.is_empty() {
        out.push_str("- None found.\n");
    } else {
        for r in &d.run {
            out.push_str(&format!("- {r}\n"));
        }
    }
    out.push_str("\n## Needs attention\n\n");
    if d.attention.is_empty() {
        out.push_str("- Nothing found.\n");
    } else {
        for a in &d.attention {
            out.push_str(&format!("- {} Next step: {}\n", a.what, a.next));
        }
    }
    out
}

/// The draft's absolute path for workspace `id` in vault `root`.
pub(crate) fn draft_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    crate::vault::safe_join(root, &format!("workspaces/{id}/{DRAFT_FILE}"))
}

/// [`fingerprint`] off the async runtime.
pub(crate) async fn fingerprint_async(repo: PathBuf) -> Fingerprint {
    tokio::task::spawn_blocking(move || fingerprint(&repo))
        .await
        .unwrap_or_else(|_| Fingerprint {
            entries: BTreeMap::new(),
            digest: "sha256:unavailable".into(),
            truncated: true,
        })
}

/// How many paths `git status` lists for `repo`, without writing its index.
async fn uncommitted(repo: &Path) -> Result<usize, String> {
    let args = [
        "--no-optional-locks",
        "-c",
        "core.fsmonitor=false",
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=normal",
    ];
    let out =
        crate::build_workspaces::git_raw(repo, &args, crate::build_workspaces::QUICK, 1024 * 1024)
            .await?;
    if out.code != Some(0) {
        return Err(crate::build_workspaces::git_error(&args, &out.stderr));
    }
    // `-z`: entries end in NUL; a rename's second path is its own field.
    let mut count = 0;
    let mut skip = false;
    for field in out.stdout.split(|b| *b == 0) {
        if skip {
            skip = false;
            continue;
        }
        if field.len() < 3 {
            continue;
        }
        count += 1;
        skip = matches!(field[0], b'R' | b'C');
    }
    Ok(count)
}

/// What [`write_draft`] did.
#[derive(Debug)]
pub(crate) struct Written {
    pub(crate) path: PathBuf,
    pub(crate) proof: Proof,
    pub(crate) attention: usize,
}

/// Describe `repo` for workspace `id` and write the draft into the vault,
/// taking the after fingerprint once the facts are read. `before` was taken
/// before the connect touched anything. Refuses a draft path inside the
/// repository (a vault kept inside it), so the draft never lands there.
pub(crate) async fn write_draft(
    vault: &Path,
    id: &str,
    repo: &Path,
    before: Fingerprint,
) -> Result<Written, String> {
    let path = draft_path(vault, id)?;
    let canon_repo = repo.canonicalize().unwrap_or_else(|_| repo.to_path_buf());
    if path.starts_with(&canon_repo) || path.starts_with(repo) {
        return Err(format!(
            "the workspace folder is inside {}, so a draft there would change the repository",
            repo.display()
        ));
    }
    let status = uncommitted(repo).await;
    let repo_owned = repo.to_path_buf();
    let (description, after, commit) = tokio::task::spawn_blocking(move || {
        let description = describe(&repo_owned, status);
        let commit = crate::project_card::head_commit(&repo_owned);
        (description, fingerprint(&repo_owned), commit)
    })
    .await
    .map_err(|e| e.to_string())?;
    let text = render(
        repo,
        commit.as_deref(),
        &chrono::Utc::now().to_rfc3339(),
        &description,
        &before,
        &after,
    );
    crate::vault::atomic_write(&path, text.as_bytes())?;
    Ok(Written {
        path,
        proof: compare(&before, &after),
        attention: description.attention.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn fingerprint_is_stable_skips_git_and_sees_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        write(repo, "src/a.rs", "fn a() {}\n");
        write(repo, ".git/HEAD", "ref: refs/heads/main\n");
        let one = fingerprint(repo);
        let two = fingerprint(repo);
        assert_eq!(one.digest, two.digest);
        assert!(!one.truncated);
        assert!(one.entries.keys().all(|k| !k.starts_with(".git")));
        assert!(one.entries.contains_key("(git) HEAD"));
        // What the connect writes by design under .git is not in the proof.
        write(repo, ".git/objects/ab/cdef", "x");
        write(repo, ".git/refs/heads/hollow/x", "x");
        write(repo, ".git/worktrees/x/HEAD", "x");
        write(repo, ".git/FETCH_HEAD", "x");
        assert_eq!(compare(&one, &fingerprint(repo)), Proof::Unchanged);
        // The index, config and hooks are.
        for (file, key) in [
            (".git/index", "(git) index"),
            (".git/config", "(git) config"),
            (".git/hooks/post-checkout", "(git) hooks/post-checkout"),
        ] {
            let before = fingerprint(repo);
            write(repo, file, "changed");
            let Proof::Changed { paths, .. } = compare(&before, &fingerprint(repo)) else {
                panic!("{file} not in the proof");
            };
            assert!(paths.contains(&key.to_string()), "{paths:?}");
        }
        let one = fingerprint(repo);
        // An untracked file and a changed size both show.
        write(repo, "notes.txt", "new");
        write(repo, "src/a.rs", "fn a() { /* longer */ }\n");
        let Proof::Changed { paths, total } = compare(&one, &fingerprint(repo)) else {
            panic!("expected a change");
        };
        assert!(paths.contains(&"notes.txt".to_string()));
        assert!(paths.iter().any(|p| p.ends_with("a.rs")));
        assert!(total >= 2);
    }

    #[test]
    fn describe_lists_facts_and_attention_with_next_steps() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        write(
            repo,
            "package.json",
            r#"{"name":"shop","scripts":{"dev":"vite","build":"vite build"}}"#,
        );
        write(repo, "src/main.ts", "");
        let d = describe(repo, Ok(3));
        let facts = d.facts.join("\n");
        assert!(facts.contains("package `shop`"), "{facts}");
        assert!(facts.contains("Top level: package.json, src/."), "{facts}");
        assert!(d.run.contains(&"`npm run build`".to_string()));
        let what: Vec<&str> = d.attention.iter().map(|a| a.what.as_str()).collect();
        assert!(what.iter().any(|w| w.starts_with("No README")), "{what:?}");
        assert!(what.iter().any(|w| w.contains("AGENTS.md")));
        assert!(what.iter().any(|w| w.starts_with("No tests folder")));
        assert!(what.iter().any(|w| w.contains("without a lockfile")));
        assert!(what.iter().any(|w| w.starts_with("3 uncommitted")));
        assert!(d.attention.iter().all(|a| !a.next.is_empty()));

        // A tidy repository has nothing to flag.
        write(repo, "README.md", "# Shop\n");
        write(repo, "AGENTS.md", "rules\n");
        write(repo, "package-lock.json", "{}");
        fs::create_dir_all(repo.join("tests")).unwrap();
        let tidy = describe(repo, Ok(0));
        assert!(tidy.attention.is_empty(), "{:?}", tidy.attention);
        assert!(tidy.facts.join("\n").contains("titled \"Shop\""));
    }

    #[test]
    fn render_says_nothing_changed_or_changed_loudly() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        write(repo, "README.md", "# R\n");
        let before = fingerprint(repo);
        let d = describe(repo, Ok(0));
        let same = render(repo, Some("abc"), "t", &d, &before, &before.clone());
        assert!(same.contains("unchanged: true"));
        assert!(same.contains("Nothing changed in"));
        assert!(same.contains(&format!("before {}", before.digest)));
        assert!(same.contains("## Needs attention"));
        write(repo, "README.md", "# R, rewritten\n");
        let after = fingerprint(repo);
        let changed = render(repo, Some("abc"), "t", &d, &before, &after);
        assert!(changed.contains("unchanged: false"));
        assert!(changed.contains("**CHANGED: "), "{changed}");
        assert!(changed.contains("- `README.md`"));
        assert!(!changed.contains("Nothing changed"));
    }

    /// The real connect: a build workspace created from an existing repo
    /// gets the draft in the vault, the repository's files are untouched
    /// (fingerprint and `git status` both), and no draft lands in it.
    #[tokio::test]
    async fn first_connect_writes_a_draft_and_changes_nothing() {
        let (_tmp, repo, vault, roots) = crate::build_workspaces::tests::fixture();
        write(&repo, "scratch.txt", "untracked work\n");
        // README.md rewritten with the same bytes and an old mtime: stat
        // dirty, content clean, so a plain `git status` would rewrite
        // .git/index. A fixed time, not the clock.
        write(&repo, "README.md", "hi\n");
        fs::OpenOptions::new()
            .write(true)
            .open(repo.join("README.md"))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000))
            .unwrap();
        let before = fingerprint(&repo);
        let made = crate::build_workspaces::create(
            &vault,
            &roots,
            crate::build_workspaces::CreateInput {
                title: "Describe".into(),
                goal: "G".into(),
                repo: repo.to_string_lossy().into(),
                branch: None,
                base: None,
            },
        )
        .await
        .unwrap();
        let after = fingerprint(&repo);
        assert_eq!(compare(&before, &after), Proof::Unchanged);
        for key in ["(git) index", "(git) HEAD", "(git) config"] {
            assert!(before.entries.contains_key(key), "{key}");
            assert_eq!(before.entries.get(key), after.entries.get(key), "{key}");
        }
        let draft = draft_path(&vault, &made.workspace.id).unwrap();
        let text = fs::read_to_string(&draft).unwrap();
        assert!(text.contains("unchanged: true"), "{text}");
        assert!(text.contains("Nothing changed in"));
        assert!(text.contains("1 uncommitted or untracked path"), "{text}");
        assert!(text.contains("Next step:"));
        assert!(!repo.join("outputs").exists());
        assert!(
            made.warnings.iter().all(|w| !w.contains("draft")),
            "{:?}",
            made.warnings
        );
        // The control: a plain `git status` does rewrite the index here, so
        // the assertions above are not vacuous.
        crate::build_workspaces::tests::sh(&repo, &["status", "--porcelain"]);
        let Proof::Changed { paths, .. } = compare(&after, &fingerprint(&repo)) else {
            panic!("a plain git status left the index alone; the fixture is not stat dirty");
        };
        assert!(paths.contains(&"(git) index".to_string()), "{paths:?}");
    }
}
