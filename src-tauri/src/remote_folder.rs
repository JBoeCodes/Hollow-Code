//! Remote sessions, Phase 3 (2026-11-02, the Mac mini runner): a vault
//! workspace's folder on the runner. A workspace chat's turn (its folder is
//! the agent's working folder) and a build workspace's turn (its vault
//! folder is an extra folder beside the worktree) get their own copy of that
//! folder there, and the agent's changes come back when the turn ends. The
//! Main agent and chats outside a workspace stay on this Mac. Design:
//! docs/REMOTE_SESSIONS.md ("Workspace folders").
//!
//! The vault isn't a git repository, so the folder travels as git objects
//! from a **private repository of Hollow's** (app data, `RemoteFolders/`,
//! one per vault; the folder is its work tree only through `GIT_WORK_TREE`
//! and an index of its own, so nothing is written into the vault):
//!
//! 1. [`snapshot`]: the folder's files as one commit (junk, secret-looking
//!    names, repositories inside it and files over [`MAX_FILE_BYTES`] left
//!    out, and nothing else: the folder's own `.gitignore` files don't
//!    decide what goes), pushed to the vault's bare repository on the
//!    runner, which checks it out fresh in `~/hollow-remote/folders/<id>`
//!    (one copy per folder and conversation, so two chats in one workspace
//!    never share one).
//! 2. When the agent ends, the runner commits the copy (parent: the snapshot)
//!    to a result ref, the copy's `.gitignore` files not deciding either.
//! 3. [`bring_back`] fetches it and applies the difference from the snapshot
//!    file by file, three ways: a file unchanged here takes the runner's
//!    version (or goes, if the agent removed it); one changed here and not
//!    there stays; one changed on both sides is merged line by line when
//!    both are text and the edits don't overlap (`shared_docs::merge3`), and
//!    otherwise keeps this Mac's version with the runner's beside it as a
//!    conflicted copy (`vault::keep_conflict_copy`), which Waiting on you
//!    offers like any other. Symlinks, submodules and paths that would leave
//!    the folder are never written.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;

/// What isn't sent and doesn't come back (both sides use the same list).
/// `/sessions/` is Hollow's own conversation records (re-grounding): never
/// taken from the runner.
pub(crate) const EXCLUDE: &str = ".DS_Store\nnode_modules/\n.trash/\n*.hollow-tmp-*\n/sessions/\n";

/// A file over this stays on this Mac (the agent there doesn't see it), and
/// one the runner made stays there (overnight R3 review: both ways).
pub(crate) const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// What only a `.gitignore` names travels in bounds (overnight R3 review): a
/// snapshot holds at most this many such files, those sent before
/// included, and a finish there adds at most this many new ones; the rest
/// stay where they are and are named.
pub(crate) const GITIGNORED_MAX_FILES: usize = 2_000;
/// ... and at most this many bytes of them.
pub(crate) const GITIGNORED_MAX_BYTES: u64 = 200 * 1024 * 1024;

/// A text file over this isn't merged line by line (both versions are kept).
const MAX_MERGE_BYTES: usize = 1024 * 1024;

/// A folder sent to the runner for one turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FolderSync {
    /// The workspace folder here.
    pub local: PathBuf,
    /// Its copy on the runner.
    pub dir: String,
    /// The vault's bare repository's id there.
    pub repo_id: String,
    /// This copy's id (folder and conversation): `<slug>-<hash>`.
    pub fid: String,
    /// The snapshot commit that was sent.
    pub snapshot: String,
    /// Hollow's private repository for this vault here.
    pub git_dir: PathBuf,
    /// What stayed on this Mac (the turn's first line names it; not kept in
    /// a remote turn's record, overnight R1).
    #[serde(skip)]
    pub left: LeftHere,
}

/// Paths of one kind: how many there are, and the first [`LEFT_NAMED`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Named {
    pub count: usize,
    pub first: Vec<String>,
}

impl Named {
    pub(crate) fn push(&mut self, path: String) {
        if self.first.len() < LEFT_NAMED {
            self.first.push(path);
        }
        self.count = self.count.saturating_add(1);
    }
    pub(crate) fn add(&mut self, other: &Named) {
        for path in &other.first {
            if self.first.len() < LEFT_NAMED {
                self.first.push(path.clone());
            }
        }
        self.count = self.count.saturating_add(other.count);
    }
    pub(crate) fn of(paths: &[String]) -> Named {
        let mut named = Named::default();
        for path in paths {
            named.push(path.clone());
        }
        named
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.count == 0
    }
    /// `{count, first}`, as the chat's lines read it.
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({ "count": self.count, "first": self.first })
    }
}

/// What stays on this Mac when a folder is sent, by why: paths relative to
/// the folder, `/`-separated, a folder's ending in `/`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeftHere {
    /// Secret-looking names ([`secret_looking`]): never sent, whatever a
    /// `.gitignore` says.
    pub secret: Vec<String>,
    /// Files over [`MAX_FILE_BYTES`].
    pub big: Vec<String>,
    /// Repositories inside the folder (git can't snapshot one).
    pub repos: Vec<String>,
    /// Folders that ignore themselves ([`self_ignoring`]: a tool's cache or
    /// environment): junk, so not named.
    pub caches: Vec<String>,
    /// Files only a `.gitignore` names, past [`GITIGNORED_MAX_FILES`] or
    /// [`GITIGNORED_MAX_BYTES`].
    pub budget: Named,
}

impl LeftHere {
    /// What the exclude list and the index leave out by path.
    fn all(&self) -> impl Iterator<Item = &String> {
        self.secret
            .iter()
            .chain(&self.big)
            .chain(&self.repos)
            .chain(&self.caches)
    }

    /// The start line's `leftHere`, None when nothing named stayed: each
    /// kind's first [`LEFT_NAMED`] paths and how many there are.
    pub(crate) fn notice(&self) -> Option<serde_json::Value> {
        if self.secret.is_empty()
            && self.big.is_empty()
            && self.repos.is_empty()
            && self.budget.is_empty()
        {
            return None;
        }
        Some(serde_json::json!({
            "secret": Named::of(&self.secret).json(),
            "big": Named::of(&self.big).json(),
            "repos": Named::of(&self.repos).json(),
            "budget": self.budget.json(),
        }))
    }
}

/// Paths of one kind the chat's lines name; the rest are counted.
pub(crate) const LEFT_NAMED: usize = 5;

impl FolderSync {
    /// The hash part of the id, the refs' prefix.
    pub(crate) fn hash(&self) -> &str {
        self.fid.rsplit('-').next().unwrap_or(&self.fid)
    }
    /// The result ref the runner writes.
    pub(crate) fn result_ref(&self) -> String {
        format!("refs/hollow-session/{}-result", self.hash())
    }
}

/// The ids for `folder` in `vault`, for conversation `session_key`: the
/// vault's bare repository there and this copy's folder.
pub(crate) fn ids(vault: &Path, folder: &Path, session_key: &str) -> (String, String) {
    let vault_s = vault.to_string_lossy();
    let folder_s = folder.to_string_lossy();
    (
        format!("vault-{}-{}", slug(&vault_s), hash_of(&vault_s, 12)),
        format!(
            "{}-{}",
            slug(&folder_s),
            hash_of(&format!("{folder_s}\n{session_key}"), 12)
        ),
    )
}

fn hash_of(text: &str, n: usize) -> String {
    use sha2::{Digest, Sha256};
    let hex = format!("{:x}", Sha256::digest(text.as_bytes()));
    hex[..n.min(hex.len())].to_string()
}

fn slug(path: &str) -> String {
    let base = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    base.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect()
}

/// The vault workspace folder a turn in `cwd` takes to the runner, and
/// whether it is the agent's working folder (a workspace chat) rather than
/// an extra folder (a build workspace's vault folder). None for the Main
/// agent, a chat outside a workspace, or a subfolder.
pub(crate) fn folder_for(vault: &Path, cwd: &Path) -> Option<(PathBuf, bool)> {
    let workspace = crate::workspaces::workspace_for_cwd(vault, cwd).ok()??;
    if never_moves(&workspace.id) {
        return None;
    }
    let folder = vault.join(&workspace.path);
    let is_cwd = match (folder.canonicalize(), cwd.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !is_cwd && workspace.build.as_ref().is_none_or(|b| !b.exists) {
        return None;
    }
    folder.is_dir().then_some((folder, is_cwd))
}

/// The Main agent and Quick chats always run on this Mac.
pub(crate) fn never_moves(id: &str) -> bool {
    crate::workspaces::is_main_agent(id) || crate::workspaces::is_quick_chats(id)
}

/// The id of the vault workspace whose own folder `cwd` is (not the Main
/// agent's, not a build workspace's worktree).
pub(crate) fn workspace_of(vault: &Path, cwd: &Path) -> Option<String> {
    match folder_for(vault, cwd) {
        Some((_, true)) => crate::workspaces::workspace_for_cwd(vault, cwd)
            .ok()
            .flatten()
            .map(|w| w.id),
        _ => None,
    }
}

// ---- Runs on for vault workspaces -----------------------------------------

/// Where each vault workspace's own Runs on choice lives: app data, never the
/// vault (an agent that writes the vault must not move where runs execute),
/// `{ "<vault>": { "<workspace id>": "this-mac" | "runner" } }`. Files from
/// before M6 (2026-11-15) say `mac-mini` for the runner: still read as
/// `runner`, and left as they are until the person changes the choice.
fn store_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join("workspace-runs-on.json"))
}

type Store = std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>;

/// One writer at a time: the store is read, changed and written whole.
static STORE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn load() -> Result<Store, String> {
    let path = store_path()?;
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Vault workspace `id`'s own Runs on (`this-mac` or `runner`, an old
/// `mac-mini` read as `runner`), or None (it follows Settings, Devices,
/// Runs on). Reads only: an old file is never rewritten here.
pub(crate) fn runs_on_for(vault: &Path, id: &str) -> Option<String> {
    load()
        .ok()?
        .get(&crate::session_store::vault_file_stem(vault))?
        .get(id)
        .map(|v| crate::agent_settings::canonical_place(v))
}

/// Vault workspace `id`'s own Runs on exactly as stored (an old `mac-mini`
/// kept), for [`restore_runs_on`].
pub(crate) fn stored_runs_on(vault: &Path, id: &str) -> Option<String> {
    load()
        .ok()?
        .get(&crate::session_store::vault_file_stem(vault))?
        .get(id)
        .cloned()
}

/// Set or clear (`None`, `default`) vault workspace `id`'s own Runs on;
/// returns what it is now. The Main agent always runs here.
pub(crate) fn set_runs_on(
    vault: &Path,
    id: &str,
    runs_on: Option<&str>,
) -> Result<Option<String>, String> {
    store_runs_on(vault, id, runs_on, false)
}

/// Put back a value [`stored_runs_on`] read (the phone's failed-send undo):
/// an old `mac-mini` goes back as it was, so an undo never rewrites a
/// choice the person didn't change (M6 review).
pub(crate) fn restore_runs_on(
    vault: &Path,
    id: &str,
    stored: Option<&str>,
) -> Result<Option<String>, String> {
    store_runs_on(vault, id, stored, true)
}

fn store_runs_on(
    vault: &Path,
    id: &str,
    runs_on: Option<&str>,
    keep_old_name: bool,
) -> Result<Option<String>, String> {
    if never_moves(id) {
        return Err("The COO and Chats always run on this Mac.".into());
    }
    let _held = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let folder = vault.join("workspaces").join(id);
    if id.is_empty() || id.contains('/') || id.starts_with('.') || !folder.is_dir() {
        return Err(format!("{id} is not a workspace"));
    }
    let value = match runs_on.map(str::trim) {
        None | Some("") | Some("default") => None,
        Some(v) => match crate::agent_settings::place_choice(v) {
            Some(place) => Some(place.to_string()),
            None => return Err(format!("Runs on is this-mac or runner, not {v}")),
        },
    };
    let mut store = load()?;
    let key = crate::session_store::vault_file_stem(vault);
    let entry = store.entry(key.clone()).or_default();
    match &value {
        Some(v) => {
            let old = crate::agent_settings::LEGACY_RUNNER;
            let kept = keep_old_name && runs_on.map(str::trim) == Some(old);
            entry.insert(id.to_string(), if kept { old.into() } else { v.clone() });
        }
        None => {
            entry.remove(id);
        }
    }
    if entry.is_empty() {
        store.remove(&key);
    }
    let path = store_path()?;
    let text = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&path, text.as_bytes())?;
    Ok(value)
}

// ---- git with the private repository --------------------------------------

fn git_raw(cwd: &Path, args: &[&str], env: &[(&str, &OsStr)]) -> Result<Vec<u8>, String> {
    git_run(cwd, args, env, None)
}

/// [`git_raw`] with `input` on its stdin (written from a thread, so a full
/// pipe either way can't stall it).
fn git_input(
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &OsStr)],
    input: &[u8],
) -> Result<Vec<u8>, String> {
    git_run(cwd, args, env, Some(input))
}

fn git_run(
    cwd: &Path,
    args: &[&str],
    env: &[(&str, &OsStr)],
    input: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut cmd = std::process::Command::new("git");
    // English messages (some are matched below), and no line-ending or
    // attribute conversion either way: bytes go and come back as they are.
    cmd.current_dir(cwd)
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.attributesFile=/dev/null",
        ])
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("LC_ALL", "C");
    for (key, value) in env {
        cmd.env(key, value);
    }
    let started = |e: std::io::Error| format!("git {} couldn't start ({e})", args.join(" "));
    let mut child = cmd.spawn().map_err(started)?;
    let writer = match (input, child.stdin.take()) {
        (Some(bytes), Some(mut stdin)) => {
            let bytes = bytes.to_vec();
            Some(std::thread::spawn(move || {
                use std::io::Write;
                let _ = stdin.write_all(&bytes);
            }))
        }
        _ => None,
    };
    let out = child.wait_with_output().map_err(started)?;
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!(
            "git {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn git_text(cwd: &Path, args: &[&str], env: &[(&str, &OsStr)]) -> Result<String, String> {
    git_raw(cwd, args, env).map(|out| String::from_utf8_lossy(&out).trim().to_string())
}

/// Hollow's private repository for `vault`'s folders (made when missing).
pub(crate) fn private_repo(vault: &Path) -> Result<PathBuf, String> {
    let dir = crate::agent_settings::app_data_dir()?
        .join("RemoteFolders")
        .join(format!(
            "{}.git",
            crate::session_store::vault_file_stem(vault)
        ));
    if !dir.join("HEAD").is_file() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        git_text(
            &dir,
            &["init", "-q", "--bare", "."],
            &[("GIT_DIR", dir.as_os_str())],
        )?;
    }
    Ok(dir)
}

const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "hollow-folder"),
    ("GIT_AUTHOR_EMAIL", "hollow-folder@localhost"),
    ("GIT_COMMITTER_NAME", "hollow-folder"),
    ("GIT_COMMITTER_EMAIL", "hollow-folder@localhost"),
];

/// The folder's files as one commit in `git_dir` (kept at
/// `refs/hollow-folder/<hash>/snap`, so its objects stay and the next push
/// sends only what changed), and what stayed here. Junk ([`EXCLUDE`],
/// folders that ignore themselves), secret-looking names, repositories
/// inside the folder and files over [`MAX_FILE_BYTES`] are left out. What
/// the folder's own `.gitignore` files name goes like any other file
/// (overnight R3), within [`GITIGNORED_MAX_FILES`] and
/// [`GITIGNORED_MAX_BYTES`]: the vault isn't a git repository, so they are
/// the person's own, for some other tool. What stays is Hollow's call
/// alone: a `.gitignore` negation (`!node_modules/`) brings nothing back
/// in either, since the index is checked after the adds.
pub(crate) fn snapshot(
    git_dir: &Path,
    folder: &Path,
    hash: &str,
) -> Result<(String, LeftHere), String> {
    let exclude = git_dir.join(format!("hollow-exclude-{hash}"));
    let mut patterns = EXCLUDE.to_string();
    let mut left = left_here(folder);
    for stays in left.all() {
        // A name with a line break can't be a pattern (it would be two,
        // `!*` among them); the check after the adds takes it out instead.
        if stays.contains(['\n', '\r']) {
            continue;
        }
        patterns.push('/');
        for c in stays.chars() {
            if matches!(c, '*' | '?' | '[' | '\\' | ' ' | '!' | '#') {
                patterns.push('\\');
            }
            patterns.push(c);
        }
        patterns.push('\n');
    }
    std::fs::write(&exclude, patterns).map_err(|e| format!("{}: {e}", exclude.display()))?;
    let index = git_dir.join(format!("hollow-index-{hash}"));
    let mut env: Vec<(&str, &OsStr)> = vec![
        ("GIT_DIR", git_dir.as_os_str()),
        ("GIT_WORK_TREE", folder.as_os_str()),
        ("GIT_INDEX_FILE", index.as_os_str()),
    ];
    for (k, v) in IDENTITY {
        env.push((k, OsStr::new(v)));
    }
    // What an earlier snapshot took and now stays here (grown past the cap,
    // become a repository or a cache, a secret-looking name sent before
    // overnight R3) leaves the index first, so `add -A` doesn't hash it
    // again: excludes cover only new files.
    let stays: Vec<&[u8]> = left
        .all()
        .map(|p| p.trim_end_matches('/').as_bytes())
        .collect();
    let _ = rm_cached(git_dir, folder, hash, &stays, &env);
    let excludes = format!("core.excludesFile={}", exclude.display());
    unreadable_is_fine(git_text(
        folder,
        &[
            "-c",
            &excludes,
            "-c",
            "core.precomposeUnicode=true",
            "add",
            "-A",
            "--ignore-errors",
            ".",
        ],
        &env,
    ))
    .and_then(|_| add_gitignored(folder, &exclude, &env))
    .and_then(|budget| {
        left.budget = budget;
        drop_what_stays(git_dir, folder, hash, &left, &env)
    })
    .map_err(|e| format!("the workspace folder couldn't be gathered ({e})"))?;
    let tree = git_text(folder, &["write-tree"], &env)?;
    // Every call makes its own commit. A commit with no parent, this message and
    // the clock's whole second is the same commit for the same tree, so two
    // snapshots of an unchanged folder within one second shared an id, and the
    // kept result of the turn sent first (its parent: that snapshot) looked like
    // one made on this turn's own, which `bring_kept_folder` refuses to apply
    // three ways (overnight 11-07, lane D3: the flake in
    // `a_workspace_folder_turn_runs_there_and_its_changes_come_back`).
    let nonce = format!("{:016x}", rand::random::<u64>());
    let commit = git_text(
        folder,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &tree,
            "-m",
            "hollow-folder: snapshot",
            "-m",
            &nonce,
        ],
        &env,
    )?;
    let dir_env = [("GIT_DIR", git_dir.as_os_str())];
    let snap = format!("refs/hollow-folder/{hash}/snap");
    let sent_prefix = format!("refs/hollow-folder/{hash}/sent/");
    // Before sent snapshots were remembered, only the last one was (`snap`):
    // count it as sent, so a result kept on it then still comes back.
    let none_yet = git_text(
        git_dir,
        &[
            "for-each-ref",
            "--count=1",
            "--format=%(refname)",
            &sent_prefix,
        ],
        &dir_env,
    )
    .is_ok_and(|listed| listed.is_empty());
    if none_yet {
        if let Ok(earlier) = git_text(git_dir, &["rev-parse", "-q", "--verify", &snap], &dir_env) {
            let _ = git_text(
                git_dir,
                &["update-ref", &sent_ref(hash, &earlier), &earlier],
                &dir_env,
            );
        }
    }
    git_text(git_dir, &["update-ref", &snap, &commit], &dir_env)?;
    // Every snapshot sent is remembered (Phase 3 leftover, 2026-11-02): a
    // result kept there is applied only on top of one of these
    // (`was_sent`), never a base the runner names. The newest
    // [`SENT_KEPT`] stay; one stays after its result came back too, so a
    // result kept on it again (a drop that failed there, a later attempt
    // of the same turn kept after it died) still applies, three ways.
    git_text(
        git_dir,
        &["update-ref", &sent_ref(hash, &commit), &commit],
        &dir_env,
    )?;
    if let Ok(listed) = git_text(
        git_dir,
        &[
            "for-each-ref",
            "--sort=-creatordate",
            "--format=%(refname)",
            &format!("refs/hollow-folder/{hash}/sent/"),
        ],
        &dir_env,
    ) {
        for old in listed.lines().skip(SENT_KEPT) {
            let _ = git_text(git_dir, &["update-ref", "-d", old], &dir_env);
        }
    }
    Ok((commit, left))
}

/// `add --ignore-errors` still exits 1 when a file couldn't be read; the
/// rest are in the index.
fn unreadable_is_fine(added: Result<String, String>) -> Result<String, String> {
    added.or_else(|e| {
        if e.contains("unable to") || e.contains("Permission denied") {
            Ok(String::new())
        } else {
            Err(e)
        }
    })
}

/// NUL-separated paths as a git argument list's file.
fn nul_list(paths: &[&[u8]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(path);
        bytes.push(0);
    }
    bytes
}

/// Take `paths` (and what is under them) out of the folder's index, by a NUL
/// pathspec file (no argument limit, any bytes), forced: the private
/// repository has no HEAD, so git otherwise refuses a file changed since it
/// was added (which is how a file that grew past the cap stayed in the
/// index before overnight R3's review).
fn rm_cached(
    git_dir: &Path,
    folder: &Path,
    hash: &str,
    paths: &[&[u8]],
    env: &[(&str, &OsStr)],
) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let list = git_dir.join(format!("hollow-rm-{hash}"));
    std::fs::write(&list, nul_list(paths)).map_err(|e| format!("{}: {e}", list.display()))?;
    let from_file = format!("--pathspec-from-file={}", list.display());
    let removed = git_text(
        folder,
        &[
            "--literal-pathspecs",
            "rm",
            "--cached",
            "-r",
            "-q",
            "-f",
            "--ignore-unmatch",
            "--pathspec-file-nul",
            &from_file,
        ],
        env,
    );
    let _ = std::fs::remove_file(&list);
    removed.map(|_| ())
}

/// The files of `listed` (NUL-separated, from `ls-files -z`).
fn nul_paths(listed: &[u8]) -> impl Iterator<Item = &[u8]> {
    listed.split(|b| *b == 0).filter(|p| !p.is_empty())
}

/// What only the folder's own `.gitignore` files hid from `add -A`
/// (overnight R3), added by name: the files still untracked after it that
/// Hollow's own list (`exclude`) doesn't name, listed with that list and no
/// `.gitignore` (`ls-files --others --exclude-from`), less what may not
/// travel ([`may_travel`]), within [`GITIGNORED_MAX_FILES`] and
/// [`GITIGNORED_MAX_BYTES`] counting those an earlier snapshot sent (in
/// path order, so the same ones go each turn). Never `add -f` on a folder:
/// it would take the junk too, since it ignores `core.excludesFile` as
/// well. `update-index --add --remove --stdin` takes the list in linear
/// time (a long `add -f` pathspec list is quadratic) and passes over a file
/// that went meanwhile; it stops at one it can't read, so those are left
/// out first. What is past the budget is returned.
fn add_gitignored(folder: &Path, exclude: &Path, env: &[(&str, &OsStr)]) -> Result<Named, String> {
    let size = |rel: &[u8]| {
        folder
            .join(String::from_utf8_lossy(rel).as_ref())
            .symlink_metadata()
            .map_or(0, |m| m.len())
    };
    // What an earlier snapshot sent of these counts first.
    let sent = git_raw(
        folder,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--ignored",
            "--exclude-per-directory=.gitignore",
        ],
        env,
    )?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    for rel in nul_paths(&sent) {
        files += 1;
        bytes += size(rel);
    }
    let from = format!("--exclude-from={}", exclude.display());
    let listed = git_raw(
        folder,
        &[
            "-c",
            "core.precomposeUnicode=true",
            "ls-files",
            "-z",
            "--others",
            &from,
        ],
        env,
    )?;
    let mut names: Vec<&[u8]> = Vec::new();
    let mut past = Named::default();
    let mut memo = Memo::new();
    for raw in nul_paths(&listed) {
        let rel = String::from_utf8_lossy(raw);
        // A repository inside the folder is listed as its folder (`app/`).
        if rel.ends_with('/') || !may_travel(folder, &rel, &mut memo) {
            continue;
        }
        let full = folder.join(rel.as_ref());
        let Ok(meta) = full.symlink_metadata() else {
            continue;
        };
        // The budget first: past it, no file is opened.
        if files >= GITIGNORED_MAX_FILES || bytes + meta.len() > GITIGNORED_MAX_BYTES {
            past.push(rel.into_owned());
            continue;
        }
        if meta.is_file() && std::fs::File::open(&full).is_err() {
            continue;
        }
        files += 1;
        bytes += meta.len();
        names.push(raw);
    }
    if !names.is_empty() {
        git_input(
            folder,
            &[
                "-c",
                "core.precomposeUnicode=true",
                "update-index",
                "--add",
                "--remove",
                "-z",
                "--stdin",
            ],
            env,
            &nul_list(&names),
        )?;
    }
    Ok(past)
}

/// Which folders [`self_ignoring`] said yes or no to, so each is looked at
/// once per pass.
type Memo = std::collections::HashMap<PathBuf, bool>;

/// Whether a file git lists in `folder` may go to the runner: nothing that
/// stays here ([`stays_here`], [`secret_looking`], inside a cache: round 2
/// review, since [`left_here`]'s names can differ from git's in Unicode
/// form, so its patterns can miss) and nothing over [`MAX_FILE_BYTES`].
fn may_travel(folder: &Path, rel: &str, memo: &mut Memo) -> bool {
    let path = Path::new(rel);
    !stays_here(folder, path)
        && !rel.split('/').any(secret_looking)
        && !inside_cache_here(folder, path, memo)
        && folder
            .join(path)
            .symlink_metadata()
            .is_ok_and(|m| !(m.is_file() && m.len() > MAX_FILE_BYTES))
}

/// What stays here never is in the snapshot, whatever got it into the index:
/// a `.gitignore` negation outranks `core.excludesFile` (`!node_modules/`,
/// `!/sessions/` or `!render.mov` let `add -A` take them), a name with a
/// line break can't be an exclude pattern, and a file made after
/// [`left_here`] looked isn't in its lists. So every path in the index that
/// may not travel, or is under what stays, leaves it.
fn drop_what_stays(
    git_dir: &Path,
    folder: &Path,
    hash: &str,
    left: &LeftHere,
    env: &[(&str, &OsStr)],
) -> Result<(), String> {
    let listed = git_raw(folder, &["ls-files", "-z"], env)?;
    let under: Vec<String> = left
        .all()
        .map(|p| format!("{}/", p.trim_end_matches('/')))
        .collect();
    let mut memo = Memo::new();
    let found: Vec<&[u8]> = nul_paths(&listed)
        .filter(|raw| {
            let rel = String::from_utf8_lossy(raw);
            let path = format!("{rel}/");
            under.iter().any(|u| path.starts_with(u.as_str()))
                || !may_travel(folder, &rel, &mut memo)
        })
        .collect();
    rm_cached(git_dir, folder, hash, &found, env)
}

/// A secret-looking name (keys, credentials, tokens, env files): Hollow's
/// rule for these, `directory_connections::secret_name` (what `read_source`
/// refuses and repo receipts withhold), plus key files and token names it
/// doesn't list (overnight R3 review), compared as APFS compares names.
/// Such a file or folder never goes to the runner, whatever a `.gitignore`
/// says. What comes back is held to the narrower [`never_taken_back`].
pub(crate) fn secret_looking(name: &str) -> bool {
    let name = apfs_fold(&name.to_lowercase());
    crate::directory_connections::secret_name(&name)
        || name.starts_with("id_ecdsa")
        || name.starts_with("id_dsa")
        || name.contains("serviceaccount")
        || [".p8", ".jks", ".ppk", ".kdbx"]
            .iter()
            .any(|ext| name.ends_with(ext))
        || ["-token", "_token", ".token"]
            .iter()
            .any(|t| name.ends_with(t) || name.contains(&format!("{t}.")))
}

/// A name never written into the vault from the runner (overnight R3
/// review): files tools read for keys and settings on their own (`.env*`,
/// `.npmrc`, `.netrc`, `.pypirc`, their folders `.ssh/`, `.aws/`, `.kube/`,
/// `.docker/`, `.gnupg/`) and private keys and certificates. Narrower than
/// [`secret_looking`], so a deliverable named like `token-samples.json`
/// comes back the usual way: it can't overwrite a file this Mac didn't
/// send, and a clash becomes a conflicted copy.
fn never_taken_back(name: &str) -> bool {
    let name = apfs_fold(&name.to_lowercase());
    name.starts_with(".env")
        || matches!(
            name.as_str(),
            ".npmrc" | ".netrc" | ".pypirc" | ".ssh" | ".aws" | ".kube" | ".docker" | ".gnupg"
        )
        || ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"]
            .iter()
            .any(|key| name.starts_with(key))
        || [".pem", ".key", ".p12", ".pfx", ".p8"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

/// A folder that ignores itself (overnight R3 review): a tool's cache or
/// environment, junk on both sides like `node_modules/`. `python3 -m venv`
/// (3.13 on), `uv venv`, pytest, mypy and ruff write a `.gitignore` of just
/// `*` into what they make; caches carry `CACHEDIR.TAG`; a venv has
/// `pyvenv.cfg`. The runner has the same test (`self_ignoring` in
/// `REMOTE_RUN_PL`).
pub(crate) fn self_ignoring(dir: &Path) -> bool {
    if dir.join("CACHEDIR.TAG").symlink_metadata().is_ok()
        || dir.join("pyvenv.cfg").symlink_metadata().is_ok()
    {
        return true;
    }
    let ignore = dir.join(".gitignore");
    match ignore.symlink_metadata() {
        Ok(meta) if meta.is_file() && meta.len() <= 4096 => {}
        _ => return false,
    }
    let Ok(text) = std::fs::read_to_string(&ignore) else {
        return false;
    };
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    lines.next() == Some("*") && lines.next().is_none()
}

/// Snapshots of one folder remembered as sent.
const SENT_KEPT: usize = 16;

/// Where snapshot `commit` of folder `hash` is remembered as sent.
pub(crate) fn sent_ref(hash: &str, commit: &str) -> String {
    format!("refs/hollow-folder/{hash}/sent/{commit}")
}

/// Whether `commit` is one of the snapshots of folder `hash` this Mac sent
/// (the newest [`SENT_KEPT`]).
pub(crate) fn was_sent(git_dir: &Path, hash: &str, commit: &str) -> bool {
    commit.len() >= 40
        && commit.chars().all(|c| c.is_ascii_hexdigit())
        && git_text(
            git_dir,
            &["rev-parse", "-q", "--verify", &sent_ref(hash, commit)],
            &[("GIT_DIR", git_dir.as_os_str())],
        )
        .is_ok_and(|got| got == commit)
}

/// What under `folder` stays here ([`LeftHere`]), each list sorted:
/// symlinks not followed, junk and `sessions/` not entered, and nothing
/// inside what stays listed.
fn left_here(folder: &Path) -> LeftHere {
    let mut out = LeftHere::default();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = path.symlink_metadata() else {
                continue;
            };
            let Ok(rel) = path.strip_prefix(folder) else {
                continue;
            };
            let rel = rel.to_string_lossy().into_owned();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == ".DS_Store" || name.contains(".hollow-tmp-") {
                continue;
            }
            if meta.is_dir() {
                if matches!(&*name, "node_modules" | ".trash" | ".git")
                    || (dir == folder && name == "sessions")
                {
                    continue;
                }
                if secret_looking(&name) {
                    out.secret.push(format!("{rel}/"));
                } else if path.join(".git").symlink_metadata().is_ok() {
                    // A repository inside the folder stays here whole (git
                    // can't snapshot one, and one with no commit fails the
                    // add).
                    out.repos.push(format!("{rel}/"));
                } else if self_ignoring(&path) {
                    out.caches.push(format!("{rel}/"));
                } else {
                    stack.push(path);
                }
            } else if secret_looking(&name) {
                out.secret.push(rel);
            } else if meta.is_file() && meta.len() > MAX_FILE_BYTES {
                out.big.push(rel);
            }
        }
    }
    out.secret.sort();
    out.big.sort();
    out.repos.sort();
    out.caches.sort();
    out
}

/// How a folder's changes came back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderBack {
    /// Files written or removed here as the runner left them.
    pub changed: Vec<String>,
    /// Files both sides changed, merged line by line.
    pub merged: Vec<String>,
    /// Conflicted copies made beside files both sides changed (their names).
    pub copies: Vec<String>,
    /// Files changed here and removed there: kept as they are here.
    pub kept_here: Vec<String>,
    /// What wasn't brought (a symlink, a submodule, an unsafe path, or what
    /// the snapshot leaves out: `sessions/`, junk, a cache, inside a
    /// repository that stays here).
    pub skipped: Vec<String>,
    /// Key, credential and tool-settings files ([`never_taken_back`]),
    /// never written here (overnight R3 review).
    pub refused: Vec<String>,
    /// Files over [`MAX_FILE_BYTES`] there: in the result (checked here) or
    /// left out of it by the runner (named in its message).
    pub too_big: Named,
    /// Files only a `.gitignore` names that the runner left out past
    /// [`GITIGNORED_MAX_FILES`] or [`GITIGNORED_MAX_BYTES`].
    pub past_budget: Named,
    /// Files that couldn't be applied, with why (the rest were).
    pub failed: Vec<String>,
    /// Copies of an earlier result on a base this Mac never sent
    /// ([`apply_copies`]): kept beside the files here, nothing written over.
    pub unsure: Vec<String>,
}

impl FolderBack {
    pub(crate) fn is_empty(&self) -> bool {
        self.changed.is_empty()
            && self.merged.is_empty()
            && self.copies.is_empty()
            && self.unsure.is_empty()
            && self.kept_here.is_empty()
            && self.skipped.is_empty()
            && self.refused.is_empty()
            && self.too_big.is_empty()
            && self.past_budget.is_empty()
            && self.failed.is_empty()
    }

    /// Another result's outcome added to this one (kept results applied one
    /// after another).
    pub(crate) fn extend(&mut self, other: FolderBack) {
        self.changed.extend(other.changed);
        self.merged.extend(other.merged);
        self.copies.extend(other.copies);
        self.unsure.extend(other.unsure);
        self.kept_here.extend(other.kept_here);
        self.skipped.extend(other.skipped);
        self.refused.extend(other.refused);
        self.too_big.add(&other.too_big);
        self.past_budget.add(&other.past_budget);
        self.failed.extend(other.failed);
    }

    /// The lists the chat's lines show of what didn't come back, each the
    /// first [`LEFT_NAMED`] and how many (overnight R3 review: a result
    /// with thousands of them stays one readable line).
    pub(crate) fn not_brought_json(&self) -> serde_json::Value {
        serde_json::json!({
            "skipped": Named::of(&self.skipped).json(),
            "refused": Named::of(&self.refused).json(),
            "tooBig": self.too_big.json(),
            "pastBudget": self.past_budget.json(),
        })
    }
}

/// A relative path from the runner that is safe to write under the folder:
/// plain components only, none of them `.git` (any case).
fn safe_rel(rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.contains('\0') || rel.starts_with('/') {
        return None;
    }
    let path = PathBuf::from(rel);
    for part in path.components() {
        match part {
            Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case(".git") => {}
            _ => return None,
        }
    }
    Some(path)
}

/// What the snapshot leaves out is never taken from the runner either (a
/// tampered runner could otherwise add it): Hollow's `sessions/` records,
/// junk, and anything inside a repository that stays here. Names compare
/// without case: this Mac's volume doesn't tell `Sessions/` from
/// `sessions/`. Secret-looking names are the caller's: everything of them
/// stays when sending ([`may_travel`]), only [`never_taken_back`] ones when
/// bringing back.
fn stays_here(folder: &Path, rel: &Path) -> bool {
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let lower: Vec<String> = parts.iter().map(|p| apfs_fold(p)).collect();
    if lower.first().map(String::as_str) == Some("sessions")
        || lower.iter().any(|p| {
            matches!(p.as_str(), "node_modules" | ".trash" | ".ds_store")
                || p.contains(".hollow-tmp-")
        })
    {
        return true;
    }
    let mut at = folder.to_path_buf();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        at.push(part);
        if at.join(".git").symlink_metadata().is_ok() {
            return true;
        }
    }
    false
}

/// A name as APFS compares it, as far as the names [`stays_here`] checks
/// go: ASCII lowercase plus every full case fold that lands on ASCII
/// letters (the long s, the Kelvin sign, ß and the Latin ligatures), so
/// `ſessions/`, `seßions/` or `.ds_ﬆore` can't reach what stays here
/// (reviews of #1083 and #1084).
fn apfs_fold(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '\u{17F}' => out.push('s'),
            '\u{212A}' => out.push('k'),
            '\u{DF}' | '\u{1E9E}' => out.push_str("ss"),
            '\u{FB00}' => out.push_str("ff"),
            '\u{FB01}' => out.push_str("fi"),
            '\u{FB02}' => out.push_str("fl"),
            '\u{FB03}' => out.push_str("ffi"),
            '\u{FB04}' => out.push_str("ffl"),
            '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
            c => out.push(c.to_ascii_lowercase()),
        }
    }
    out
}

/// Whether every folder between `folder` and `full` (exclusive) is a real
/// folder or doesn't exist yet: never write through a symlink.
fn no_symlink_between(folder: &Path, full: &Path) -> bool {
    let Ok(rel) = full.strip_prefix(folder) else {
        return false;
    };
    let mut at = folder.to_path_buf();
    let parts: Vec<_> = rel.components().collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        at.push(part);
        match at.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => return false,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    true
}

/// What is at `full` here now: `Some(Some(blob id))` for a file,
/// `Some(None)` for nothing, `None` for something else (a folder, a
/// symlink): never touched.
fn here_now(git_dir: &Path, full: &Path) -> Option<Option<String>> {
    match full.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(None),
        Err(_) => None,
        Ok(meta) if meta.is_file() => {
            let path = full.to_string_lossy();
            git_text(
                git_dir,
                &["hash-object", "--no-filters", "--", &path],
                &[("GIT_DIR", git_dir.as_os_str())],
            )
            .ok()
            .map(Some)
        }
        Ok(_) => None,
    }
}

fn blob(git_dir: &Path, sha: &str) -> Result<Vec<u8>, String> {
    git_raw(
        git_dir,
        &["cat-file", "blob", sha],
        &[("GIT_DIR", git_dir.as_os_str())],
    )
}

/// Write `bytes` to `full` atomically (`vault::atomic_write`: a hidden temp
/// file the vault watcher ignores, then a rename; folders made as needed).
fn write_file(full: &Path, bytes: &[u8], executable: bool) -> Result<(), String> {
    crate::vault::atomic_write(full, bytes)?;
    if executable {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = full.metadata() {
            let mode = meta.permissions().mode() | 0o111;
            let _ = std::fs::set_permissions(full, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

const ZERO: &str = "0000000000000000000000000000000000000000";

/// Apply what changed from `base` to `result` (both commits in `git_dir`)
/// to `folder`, three ways against what is here now (see the module doc).
/// A file that can't be applied is listed in `failed` and the rest go on.
pub(crate) fn apply(
    git_dir: &Path,
    folder: &Path,
    base: &str,
    result: &str,
) -> Result<FolderBack, String> {
    apply_with(git_dir, folder, base, result, false)
}

/// An earlier result whose base this Mac never sent (overnight P3L-4): what
/// it changed from that base comes here only as conflicted copies beside the
/// files (`unsure`), never written over or removed, since a base the runner
/// names can't say what "unchanged here" means. A file it removed stays
/// here (not mentioned); one already the same here is left alone.
pub(crate) fn apply_copies(
    git_dir: &Path,
    folder: &Path,
    base: &str,
    result: &str,
) -> Result<FolderBack, String> {
    apply_with(git_dir, folder, base, result, true)
}

fn apply_with(
    git_dir: &Path,
    folder: &Path,
    base: &str,
    result: &str,
    copies_only: bool,
) -> Result<FolderBack, String> {
    let env = [("GIT_DIR", git_dir.as_os_str())];
    let raw = git_raw(
        git_dir,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--no-commit-id",
            base,
            result,
        ],
        &env,
    )?;
    let mut back = FolderBack::default();
    let fields: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    let mut i = 0;
    while i + 1 < fields.len() {
        let meta = String::from_utf8_lossy(fields[i]).into_owned();
        let rel = String::from_utf8_lossy(fields[i + 1]).into_owned();
        i += 2;
        let parts: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        if parts.len() < 5 {
            continue;
        }
        match apply_one(
            git_dir,
            folder,
            &rel,
            [parts[0], parts[1], parts[2], parts[3]],
            copies_only,
        ) {
            Ok(Some(outcome)) => match outcome {
                One::Changed => back.changed.push(rel),
                One::Merged => back.merged.push(rel),
                One::Copy(copy) if copies_only => back.unsure.push(copy),
                One::Copy(copy) => back.copies.push(copy),
                One::KeptHere => back.kept_here.push(rel),
                One::Skipped => back.skipped.push(rel),
                One::Refused => back.refused.push(rel),
                One::TooBig => back.too_big.push(rel),
            },
            Ok(None) => {}
            Err(e) => back.failed.push(format!("{rel} ({e})")),
        }
    }
    left_there(git_dir, result, &mut back);
    Ok(back)
}

/// Lines of a result's message read, the most a count there may say, and
/// the characters of a path it names that are kept.
const LEFT_LINES: usize = 64;
const LEFT_MAX_COUNT: usize = 10_000_000;
const LEFT_PATH_CHARS: usize = 300;

/// What the runner left out of its result and says so in its message
/// (`left: big|budget <count>`, then `left-path: <kind> <path>` for the
/// first few; `REMOTE_RUN_PL`'s `left_lines`): named, never trusted for
/// anything else.
fn left_there(git_dir: &Path, result: &str, back: &mut FolderBack) {
    let Ok(message) = git_text(
        git_dir,
        &["log", "-1", "--format=%B", result],
        &[("GIT_DIR", git_dir.as_os_str())],
    ) else {
        return;
    };
    let mut big = Named::default();
    let mut budget = Named::default();
    // A runner's message is only read for names: its lines, counts and
    // paths are bounded (round 2 review).
    for line in message.lines().take(LEFT_LINES) {
        if let Some(rest) = line.strip_prefix("left: ") {
            let (kind, count) = rest.split_once(' ').unwrap_or((rest, ""));
            let count = count
                .trim()
                .parse::<usize>()
                .unwrap_or(0)
                .min(LEFT_MAX_COUNT);
            match kind {
                "big" => big.count = count,
                "budget" => budget.count = count,
                _ => {}
            }
        } else if let Some(rest) = line.strip_prefix("left-path: ") {
            let (kind, path) = rest.split_once(' ').unwrap_or((rest, ""));
            let named = match kind {
                "big" => &mut big,
                "budget" => &mut budget,
                _ => continue,
            };
            if !path.is_empty() && named.first.len() < LEFT_NAMED {
                named
                    .first
                    .push(path.chars().take(LEFT_PATH_CHARS).collect());
            }
        }
    }
    for named in [&mut big, &mut budget] {
        named.count = named.count.max(named.first.len());
    }
    back.too_big.add(&big);
    back.past_budget.add(&budget);
}

enum One {
    Changed,
    Merged,
    Copy(String),
    KeptHere,
    Skipped,
    Refused,
    TooBig,
}

/// Whether a folder `rel` is in is a cache or environment here
/// ([`self_ignoring`]): junk, never written into.
fn inside_cache_here(folder: &Path, rel: &Path, memo: &mut Memo) -> bool {
    let mut at = folder.to_path_buf();
    let parts: Vec<_> = rel.components().collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        at.push(part);
        let cache = match memo.get(&at) {
            Some(known) => *known,
            None => {
                let known = self_ignoring(&at);
                memo.insert(at.clone(), known);
                known
            }
        };
        if cache {
            return true;
        }
    }
    false
}

/// Text Hollow merges line by line when both sides changed it; anything else
/// (JSON, code, binaries) is kept twice instead, since a line merge can
/// produce a file that no longer parses.
fn mergeable(rel: &str) -> bool {
    let lower = rel.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".txt")
}

/// One file of [`apply`] (`modes_shas`: old mode, new mode, old blob, new
/// blob from `diff-tree`): None when nothing was to be done. `copies_only`:
/// [`apply_copies`].
fn apply_one(
    git_dir: &Path,
    folder: &Path,
    rel: &str,
    modes_shas: [&str; 4],
    copies_only: bool,
) -> Result<Option<One>, String> {
    let [old_mode, new_mode, old_sha, new_sha] = modes_shas;
    let unsafe_mode = |m: &str| m == "120000" || m == "160000";
    let Some(rel_path) = safe_rel(rel) else {
        return Ok(Some(One::Skipped));
    };
    let full = folder.join(&rel_path);
    if unsafe_mode(old_mode)
        || unsafe_mode(new_mode)
        || !no_symlink_between(folder, &full)
        || stays_here(folder, &rel_path)
        || inside_cache_here(folder, &rel_path, &mut Memo::new())
    {
        return Ok(Some(One::Skipped));
    }
    if rel.split('/').any(never_taken_back) {
        return Ok(Some(One::Refused));
    }
    let base_sha = (old_sha != ZERO).then(|| old_sha.to_string());
    let new_sha = (new_sha != ZERO).then(|| new_sha.to_string());
    let Some(now) = here_now(git_dir, &full) else {
        return Ok(Some(One::Skipped));
    };
    if now == new_sha {
        return Ok(None);
    }
    // Over the cap there stays there, as here it stays here (overnight R3
    // review), and is never read into memory to find out.
    if let Some(sha) = &new_sha {
        let size = git_text(
            git_dir,
            &["cat-file", "-s", sha],
            &[("GIT_DIR", git_dir.as_os_str())],
        )?;
        if size.parse::<u64>().map_or(true, |n| n > MAX_FILE_BYTES) {
            return Ok(Some(One::TooBig));
        }
    }
    if copies_only {
        // A removal there says nothing about this Mac's file: it stays.
        return match &new_sha {
            Some(sha) => keep_copy(folder, &full, &blob(git_dir, sha)?).map(Some),
            None => Ok(None),
        };
    }
    let executable = new_mode == "100755";
    if now == base_sha {
        match &new_sha {
            Some(sha) => {
                let bytes = blob(git_dir, sha)?;
                // Still as it was read: another chat's write meanwhile wins.
                if here_now(git_dir, &full) != Some(now.clone()) {
                    return keep_copy(folder, &full, &bytes).map(Some);
                }
                write_file(&full, &bytes, executable)?;
            }
            None => std::fs::remove_file(&full).map_err(|e| e.to_string())?,
        }
        return Ok(Some(One::Changed));
    }
    // Both sides changed it.
    let Some(theirs_sha) = new_sha else {
        return Ok(Some(One::KeptHere));
    };
    let theirs = blob(git_dir, &theirs_sha)?;
    // Merged only when there is a common version, both sides have the file,
    // and it is text a line merge can't break.
    if let (Some(ours_sha), Some(base_sha), true) = (&now, &base_sha, mergeable(rel)) {
        let ours = std::fs::read(&full).map_err(|e| e.to_string())?;
        let base_text = blob(git_dir, base_sha)?;
        let small = ours.len().max(theirs.len()).max(base_text.len()) <= MAX_MERGE_BYTES;
        if let (true, Ok(b), Ok(o), Ok(t)) = (
            small,
            std::str::from_utf8(&base_text),
            std::str::from_utf8(&ours),
            std::str::from_utf8(&theirs),
        ) {
            if let Some(merged) = crate::shared_docs::merge3(b, o, t) {
                if here_now(git_dir, &full) == Some(Some(ours_sha.clone())) {
                    write_file(&full, merged.as_bytes(), executable)?;
                    return Ok(Some(One::Merged));
                }
            }
        }
    }
    keep_copy(folder, &full, &theirs).map(Some)
}

/// The runner's version beside `full` as a conflicted copy (its folder made
/// when this Mac removed it); the copy's path in the folder.
fn keep_copy(folder: &Path, full: &Path, bytes: &[u8]) -> Result<One, String> {
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let copy = crate::vault::keep_conflict_copy(full, bytes)?;
    Ok(One::Copy(
        copy.strip_prefix(folder)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| copy.to_string_lossy().into_owned()),
    ))
}

#[cfg(test)]
#[path = "remote_folder_tests.rs"]
mod tests;
