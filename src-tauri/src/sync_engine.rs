//! Context sync engine (context layers CX6a): one engine, a list of roots,
//! the personal root first, the vault root (CX6b) beside it.
//!
//! **Roots.** A [`Root`] is a git directory in Hollow's app data (never in
//! the vault) with a work tree, an allow-list of top-level folders and a
//! remote. The personal root's work tree is the vault and its allow-list is
//! `me/`, `memory/`, `learnings/` and `home/`; nothing else in the vault is
//! ever staged, written or removed. The vault root (CX6b, `sync_vault.rs`)
//! tracks the rest of the vault, text only, under a generated ignore, and
//! commits locally only until V6 gives it a remote; the org read-only root
//! is a later [`RootKind`] of the same engine.
//!
//! **Settings.** `context-sync.json` in the app data folder:
//! `{"version": 1, "roots": [{"id": "personal", "kind": "personal",
//! "remote": "<url>", "branch": "main", "enabled": true}]}`. No secret goes
//! in it: a URL carrying a password is refused. A file that doesn't parse
//! reads as off and is refused, never overwritten, by a save. The git
//! directory is `ContextSync/<id>.git`, the cycle lock `ContextSync/<id>.lock`
//! and the last cycle's record `ContextSync/<id>.state.json`, all in app data.
//!
//! **One cycle** ([`cycle`]), under an exclusive `flock` on the root's lock
//! file (one cycle at a time across every Hollow process; the kernel lets it
//! go when a process dies, so a crash never leaves it held):
//! 0. a tracked synced folder missing from the vault, or a commit removing
//!    more than 20 files or more than half of those tracked, blocks the
//!    cycle: an evicted or unmounted vault is never pushed as a deletion;
//! 1. stage the allow-listed folders into the index (`git add -A -f` with
//!    those folders as the pathspec, so the vault's own `.gitignore` can't
//!    hide them and nothing outside them is staged);
//! 2. check the staged changes: a symlink, nested repository, `.git` part or
//!    oversized file is left out and named while the rest syncs; a blocking
//!    finding of the shared scan rules
//!    (`scan_rules`, read from the staged blob, so what is scanned is exactly
//!    what would be committed) puts the index back to `HEAD` and stops the
//!    cycle with nothing committed: the root is **blocked** until the file is
//!    fixed, and a Waiting on you item names the file and rule;
//! 3. commit the index with plumbing (`write-tree`, `commit-tree`,
//!    `update-ref`), no hook runs; the message names the trigger and the
//!    machine slug (`Hollow-Machine:` trailer);
//! 4. `fetch`, then merge the remote branch without a rebase: `merge-tree
//!    --write-tree` computes the merge in the object store, so git never
//!    writes conflict markers anywhere. A true clash keeps the local version
//!    at its name and the remote's beside it as `Name (from <machine>).md`
//!    and saves a clash record: one clash card per file in Waiting on you
//!    (CX6c, `sync_clash.rs`);
//! 5. scan every incoming blob (a hit blocks the merge and the push), then
//!    write the merge into the work tree ourselves, only inside the
//!    allow-list, only where the file still holds what `HEAD` holds (else the
//!    cycle stops and the next one retries), files first and `HEAD` after, so
//!    a crash between the two re-commits the new files as a local change
//!    instead of reverting them;
//! 6. `push` (never forced). A push or fetch that fails leaves the commits
//!    queued in the git directory: the status says "N changes not synced"
//!    and the next cycle pushes them.
//!
//! **Quiet timer** ([`spawn_timer`]): its own thread, never on a turn's path.
//! It watches the allow-listed folders' sizes and times every 15 seconds and
//! runs a cycle once they have been still for a minute, and every ten
//! minutes regardless (to take in the other Mac's changes).
//!
//! **Off switch.** `enabled: false` stops every cycle before it reads or
//! writes anything (a running cycle re-reads it before each file it would
//! write): the git directory, the vault and the remote stay exactly
//! as they are.

#![cfg_attr(test, allow(dead_code))]

use crate::context_roots as roots;
use crate::flock::HeldLock;
use crate::scan_rules::{self, ScanOptions, Severity};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The settings file in the app data folder.
const SETTINGS_FILE: &str = "context-sync.json";
/// The folder in app data holding each root's git directory, lock and state.
const SYNC_DIR: &str = "ContextSync";
/// The personal root's id.
pub(crate) const PERSONAL: &str = "personal";
/// What the personal root tracks, relative to the vault.
pub(crate) const PERSONAL_ALLOW: [&str; 4] = ["me", "memory", "learnings", "home"];
/// A local git command's limit.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(60);
/// `git add`'s limit: it hashes every changed file.
const ADD_TIMEOUT: Duration = Duration::from_secs(300);
/// A commit that removes more files than this, or more than half of those
/// tracked (two or more), is refused as a likely accident.
const MAX_DELETIONS: usize = 20;
/// A fetch or push's limit.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);
/// A git lock file older than this, found while the cycle lock is held, was
/// left by a git that died (every git this engine runs is stopped well
/// before it).
const STALE_GIT_LOCK: Duration = Duration::from_secs(10 * 60);
/// Most bytes kept of a git command's output.
const OUT_CAP: usize = 64 * 1024 * 1024;
/// Files a commit message lists by name.
const LISTED: usize = 20;
/// The commit trailer naming the machine that wrote a commit.
const MACHINE_TRAILER: &str = "Hollow-Machine";

// ── Roots ───────────────────────────────────────────────────────────────

/// What a root syncs: `Personal` (CX6a) and `Vault` (CX6b, local only) are
/// wired; the org read-only clone (V6) comes later on the same engine.
#[allow(dead_code)] // Org: read from settings, built by V6
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RootKind {
    Personal,
    Vault,
    Org,
}

/// One root as saved in `context-sync.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RootConfig {
    pub id: String,
    pub kind: RootKind,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default)]
    pub enabled: bool,
}

fn default_branch() -> String {
    "main".to_string()
}

/// `context-sync.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncConfig {
    #[serde(default = "config_version")]
    pub version: u32,
    #[serde(default)]
    pub roots: Vec<RootConfig>,
}

fn config_version() -> u32 {
    1
}

impl Default for SyncConfig {
    fn default() -> Self {
        SyncConfig {
            version: config_version(),
            roots: Vec::new(),
        }
    }
}

impl SyncConfig {
    pub(crate) fn root(&self, id: &str) -> Option<&RootConfig> {
        self.roots.iter().find(|r| r.id == id)
    }
}

/// A root ready for a cycle: where its git directory and work tree are,
/// what it may track and how its files are scanned.
#[derive(Debug, Clone)]
pub(crate) struct Root {
    pub id: String,
    pub kind: RootKind,
    pub remote: Option<String>,
    pub branch: String,
    pub enabled: bool,
    pub git_dir: PathBuf,
    pub work_tree: PathBuf,
    /// Top-level folders of the work tree it tracks, plain names.
    pub allow: Vec<String>,
    /// Where the lock and state files live.
    pub sync_dir: PathBuf,
    /// The settings file re-read before each write into the work tree, so
    /// the off switch stops a cycle already running (`None`: tests).
    pub settings_file: Option<PathBuf>,
}

impl Root {
    /// `config` placed in `sync_dir` (`<app data>/ContextSync`) with `vault`
    /// as the personal root's work tree. A kind not built yet is refused.
    pub(crate) fn from_config(
        config: &RootConfig,
        sync_dir: &Path,
        vault: &Path,
    ) -> Result<Root, String> {
        valid_id(&config.id)?;
        valid_branch(&config.branch)?;
        if let Some(url) = &config.remote {
            valid_remote(url)?;
        }
        let (work_tree, allow) = match config.kind {
            RootKind::Personal => (
                vault.to_path_buf(),
                PERSONAL_ALLOW.iter().map(|s| s.to_string()).collect(),
            ),
            RootKind::Vault => {
                if config.remote.is_some() {
                    return Err(
                        "The vault root commits on this Mac only; its remote comes with V6.".into(),
                    );
                }
                (vault.to_path_buf(), Vec::new())
            }
            RootKind::Org => return Err("The org root isn't built yet (V6).".into()),
        };
        Ok(Root {
            id: config.id.clone(),
            kind: config.kind,
            remote: config.remote.clone(),
            branch: config.branch.clone(),
            enabled: config.enabled,
            git_dir: sync_dir.join(format!("{}.git", config.id)),
            work_tree,
            allow,
            sync_dir: sync_dir.to_path_buf(),
            settings_file: None,
        })
    }

    /// Whether the root is still on in its settings file (re-read now).
    fn still_enabled(&self) -> bool {
        match &self.settings_file {
            None => self.enabled,
            Some(path) => config_at(path).root(&self.id).is_some_and(|r| r.enabled),
        }
    }

    fn lock_path(&self) -> PathBuf {
        self.sync_dir.join(format!("{}.lock", self.id))
    }

    fn state_path(&self) -> PathBuf {
        self.sync_dir.join(format!("{}.state.json", self.id))
    }

    /// The scan layer for a path: `home/` files are the home layer, the rest
    /// of the personal root the personal layer.
    fn scan_layer(&self, rel: &str) -> &'static str {
        match self.kind {
            RootKind::Personal if rel.starts_with("home/") => "home",
            RootKind::Personal => "personal",
            RootKind::Vault => "vault",
            RootKind::Org => "org",
        }
    }

    fn allowed(&self, rel: &str) -> bool {
        if self.kind == RootKind::Vault {
            return vault::path_allowed(rel);
        }
        self.allow.iter().any(|a| {
            rel.strip_prefix(a.as_str())
                .is_some_and(|r| r.starts_with('/'))
        })
    }

    fn remote_ref(&self) -> String {
        format!("refs/remotes/origin/{}", self.branch)
    }

    fn local_ref(&self) -> String {
        format!("refs/heads/{}", self.branch)
    }
}

fn valid_id(id: &str) -> Result<(), String> {
    roots::valid_slug(id).map_err(|_| format!("{id:?} is not a root id."))
}

fn valid_branch(branch: &str) -> Result<(), String> {
    let ok = !branch.is_empty()
        && branch.len() <= 100
        && !branch.starts_with(['-', '.', '/'])
        && !branch.ends_with(['/', '.'])
        && !branch.contains("..")
        && !branch.contains("//")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!("{branch:?} is not a branch name this sync uses."))
    }
}

/// A remote URL this engine pushes to: `https://`, `ssh://`, scp form
/// (`git@host:owner/repo.git`), `file://` or an absolute local path. Never
/// one carrying credentials: an `https://` URL with any user part
/// (`https://TOKEN@host` is a token too) or an `ssh://` one with a password.
/// No secret is saved in the settings or passed in argv. No whitespace, control character or
/// leading dash, and no `ext::`-style transport.
pub(crate) fn valid_remote(url: &str) -> Result<(), String> {
    let refuse = |why: &str| Err(format!("That remote can't be used: {why}."));
    if url.is_empty() || url.len() > 1000 {
        return refuse("it is empty or too long");
    }
    if url.starts_with('-') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return refuse("it has a space, a control character or a leading dash");
    }
    if url.contains("::") {
        return refuse("git transports like ext:: aren't allowed");
    }
    let user_part = |rest: &str| {
        let authority = rest.split('/').next().unwrap_or("");
        authority.rsplit_once('@').map(|(user, _)| user.to_string())
    };
    let https = url.strip_prefix("https://");
    if let Some(rest) = https.or_else(|| url.strip_prefix("ssh://")) {
        let user = user_part(rest);
        if (https.is_some() && user.is_some()) || user.is_some_and(|u| u.contains(':')) {
            return refuse("it carries credentials; use a credential helper or an SSH key");
        }
        if rest.split('/').next().unwrap_or("").is_empty() {
            return refuse("it names no host");
        }
        return Ok(());
    }
    if url.starts_with("file:///") || url.starts_with('/') {
        return Ok(());
    }
    // scp form: user@host:path, with no `/` before the colon.
    if let Some((before, path)) = url.split_once(':') {
        if !before.contains('/')
            && before.contains('@')
            && !path.is_empty()
            && !path.starts_with('/')
        {
            return Ok(());
        }
    }
    refuse("use https://, ssh://, git@host:owner/repo.git or a local path")
}

/// `text` with the user part of every `https://` or `http://` URL in it
/// replaced by `***`, so a status or error never shows a token (a URL saved
/// before the stricter check, or one git prints back). Any other scheme's
/// `user:secret@` is replaced too (a bare `ssh://git@` user stays), and so is
/// the value of a `token=`-style query parameter (CX17 review).
pub(crate) fn redact_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '/' || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(i)
                if head.ends_with("http://")
                    || head.ends_with("https://")
                    || authority[..i].contains(':') =>
            {
                out.push_str("***");
                out.push_str(&authority[i..]);
            }
            _ => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    // `?token=…` and `&access_token=…`: the value up to the next `&`, space or quote.
    const SECRET_KEYS: [&str; 13] = [
        "token=",
        "api_key=",
        "apikey=",
        "secret=",
        "client_secret=",
        "refresh_token=",
        "id_token=",
        "access_token=",
        "private_token=",
        "auth=",
        "key=",
        "password=",
        "sig=",
    ];
    let mut redacted = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(at) = rest.find(['?', '&']) {
        let (head, tail) = rest.split_at(at + 1);
        redacted.push_str(head);
        let lower = tail.to_ascii_lowercase();
        match SECRET_KEYS.iter().find(|k| lower.starts_with(*k)) {
            Some(key) => {
                redacted.push_str(&tail[..key.len()]);
                redacted.push_str("***");
                let value = &tail[key.len()..];
                let end = value
                    .find(|c: char| c.is_whitespace() || c == '&' || c == '\'' || c == '"')
                    .unwrap_or(value.len());
                rest = &value[end..];
            }
            None => rest = tail,
        }
    }
    redacted.push_str(rest);
    redacted
}

// ── Settings ────────────────────────────────────────────────────────────

/// The app data folder's sync folder.
pub(crate) fn sync_dir() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(SYNC_DIR))
}

pub(crate) fn settings_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(SETTINGS_FILE))
}

/// The settings in `path`: missing is none; a file that doesn't parse reads
/// as none (every root off) with a log line.
pub(crate) fn config_at(path: &Path) -> SyncConfig {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("[context-sync] {}: {e}; sync is off", path.display());
            SyncConfig::default()
        }),
        Err(_) => SyncConfig::default(),
    }
}

fn config_for_write(path: &Path) -> Result<SyncConfig, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} isn't valid ({e}); fix or remove it first.",
                path.display()
            )
        }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(SyncConfig::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Change the root `id` in `path` with `edit` (a missing root starts as the
/// personal kind, off, no remote) and save; returns the saved root.
pub(crate) fn update_root_at(
    path: &Path,
    id: &str,
    edit: impl FnOnce(&mut RootConfig) -> Result<(), String>,
) -> Result<RootConfig, String> {
    let mut config = config_for_write(path)?;
    let at = match config.roots.iter().position(|r| r.id == id) {
        Some(at) => at,
        None => {
            config.roots.push(RootConfig {
                id: id.to_string(),
                kind: RootKind::Personal,
                remote: None,
                branch: default_branch(),
                enabled: false,
            });
            config.roots.len() - 1
        }
    };
    edit(&mut config.roots[at])?;
    let saved = config.roots[at].clone();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(path, format!("{text}\n").as_bytes())?;
    Ok(saved)
}

pub(crate) static SETTINGS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Save the personal root's remote and turn it on.
pub(crate) fn configure_personal(remote: &str) -> Result<RootConfig, String> {
    let remote = remote.trim().to_string();
    valid_remote(&remote)?;
    let _held = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    update_root_at(&settings_path()?, PERSONAL, |r| {
        r.remote = Some(remote);
        r.enabled = true;
        Ok(())
    })
}

/// Turn a configured root on or off. Off leaves every file as it is.
pub(crate) fn set_enabled(id: &str, enabled: bool) -> Result<RootConfig, String> {
    let _held = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = settings_path()?;
    if config_at(&path)
        .root(id)
        .and_then(|r| r.remote.as_ref())
        .is_none()
    {
        return Err("Connect a remote first.".into());
    }
    update_root_at(&path, id, |r| {
        r.enabled = enabled;
        Ok(())
    })
}

// ── Git ─────────────────────────────────────────────────────────────────

/// What one git command printed.
struct GitOut {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
}

impl GitOut {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_string()
    }
}

/// Settings every git run gets, whatever the user's or the repo's config
/// says: no hook or fsmonitor runs, no user ignore file applies, no `ext::`
/// transport, paths printed unquoted.
const GIT_CONFIG: [&str; 9] = [
    "core.hooksPath=/dev/null",
    "core.fsmonitor=false",
    "core.excludesFile=/dev/null",
    "core.quotePath=false",
    "protocol.ext.allow=never",
    "core.autocrlf=false",
    "gc.auto=0",
    // Every object fetched is checked (bad paths such as `.git`, `..`,
    // malformed trees) before anything reads it.
    "transfer.fsckObjects=true",
    "fetch.fsckObjects=true",
];

/// Run `git` on `root`'s git directory (and work tree when `work_tree`),
/// with `stdin`, stopped at `timeout` (its whole process group). Never
/// prompts; inherited `GIT_*` locations are dropped.
fn git_run(
    root: &Root,
    args: &[&str],
    work_tree: bool,
    stdin: Option<Vec<u8>>,
    env: &[(&str, String)],
    timeout: Duration,
) -> Result<GitOut, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new("git");
    cmd.arg("--git-dir").arg(&root.git_dir);
    if work_tree {
        cmd.arg("--work-tree").arg(&root.work_tree);
        cmd.current_dir(&root.work_tree);
    } else {
        cmd.current_dir(&root.sync_dir);
    }
    for c in GIT_CONFIG {
        cmd.arg("-c").arg(c);
    }
    cmd.args(args);
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .env("LC_ALL", "C");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .process_group(0);
    let mut child = cmd.spawn().map_err(|e| format!("git did not start: {e}"))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        });
    }
    let reader = |pipe: Option<Box<dyn Read + Send>>, cap: usize| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(mut pipe) = pipe {
                let mut buf = [0u8; 64 * 1024];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let room = cap.saturating_sub(out.len());
                            out.extend_from_slice(&buf[..n.min(room)]);
                        }
                    }
                }
            }
            out
        })
    };
    let out_t = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        OUT_CAP,
    );
    let err_t = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        64 * 1024,
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                if let Ok(pid) = i32::try_from(child.id()) {
                    // SAFETY: killpg only delivers a signal, to the group
                    // this git leads (spawned with process_group(0)).
                    unsafe {
                        libc::killpg(pid, libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git {} stopped after {timeout:?}", args[0]));
            }
        }
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default())
        .trim()
        .to_string();
    Ok(GitOut {
        code: status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
}

/// [`git_run`] that must succeed.
fn git_ok(root: &Root, args: &[&str], work_tree: bool) -> Result<GitOut, String> {
    git_with(root, args, work_tree, None, &[])
}

fn git_with(
    root: &Root,
    args: &[&str],
    work_tree: bool,
    stdin: Option<Vec<u8>>,
    env: &[(&str, String)],
) -> Result<GitOut, String> {
    let out = git_run(root, args, work_tree, stdin, env, LOCAL_TIMEOUT)?;
    if out.code == 0 {
        Ok(out)
    } else {
        Err(format!("git {} failed: {}", args[0], out.stderr))
    }
}

/// The commit `rev` names, `None` when it names nothing (an unborn branch).
fn rev(root: &Root, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    git_run(
        root,
        &["rev-parse", "--verify", "--quiet", &spec],
        false,
        None,
        &[],
        LOCAL_TIMEOUT,
    )
    .ok()
    .filter(|o| o.code == 0)
    .map(|o| o.text())
}

/// `ancestor` is `of` or an ancestor of it.
fn is_ancestor(root: &Root, ancestor: &str, of: &str) -> Result<bool, String> {
    let out = git_run(
        root,
        &["merge-base", "--is-ancestor", ancestor, of],
        false,
        None,
        &[],
        LOCAL_TIMEOUT,
    )?;
    match out.code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(format!("git merge-base failed: {}", out.stderr)),
    }
}

/// The empty tree's id in this repository's hash.
fn empty_tree(root: &Root) -> Result<String, String> {
    Ok(git_with(root, &["mktree", "-z"], false, Some(Vec::new()), &[])?.text())
}

/// One changed path between two trees (or a tree and the index).
#[derive(Debug, Clone)]
struct Change {
    old_mode: String,
    new_mode: String,
    old_oid: String,
    new_oid: String,
    path: String,
}

const ABSENT_MODE: &str = "000000";

/// Parse `diff-tree`/`diff-index` `-z --raw` output.
fn parse_raw(out: &[u8]) -> Result<Vec<Change>, String> {
    let mut parts = out.split(|b| *b == 0);
    let mut changes = Vec::new();
    while let Some(head) = parts.next() {
        if head.is_empty() {
            continue;
        }
        let head = std::str::from_utf8(head).map_err(|_| "git printed a non-UTF-8 line")?;
        let fields: Vec<&str> = head.trim_start_matches(':').split(' ').collect();
        if fields.len() < 5 {
            return Err(format!("unexpected git diff line {head:?}"));
        }
        let path = parts.next().ok_or("git diff line without a path")?;
        let path = String::from_utf8(path.to_vec())
            .map_err(|_| "a file name that isn't UTF-8 can't be synced")?;
        changes.push(Change {
            old_mode: fields[0].to_string(),
            new_mode: fields[1].to_string(),
            old_oid: fields[2].to_string(),
            new_oid: fields[3].to_string(),
            path,
        });
    }
    Ok(changes)
}

/// The allow-list as literal pathspecs; for the vault root, the whole tree
/// but the personal root's folders.
fn pathspecs(root: &Root) -> Vec<String> {
    if root.kind == RootKind::Vault {
        let mut specs = vec![":/".to_string()];
        specs.extend(
            PERSONAL_ALLOW
                .iter()
                .map(|a| format!(":(top,exclude,literal){a}")),
        );
        return specs;
    }
    root.allow
        .iter()
        .map(|a| format!(":(literal){a}"))
        .collect()
}

/// [`pathspecs`] for `ls-tree`, which takes no `exclude` magic: the vault
/// root's tree never holds the personal folders, so it is read whole.
fn tree_pathspecs(root: &Root) -> Vec<String> {
    if root.kind == RootKind::Vault {
        Vec::new()
    } else {
        pathspecs(root)
    }
}

/// A tree entry: (mode, oid), when `path` is a blob or link in `rev`.
fn entry(root: &Root, rev: &str, path: &str) -> Result<Option<(String, String)>, String> {
    let spec = format!(":(literal){path}");
    let out = git_ok(root, &["ls-tree", "-z", rev, "--", &spec], false)?;
    for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let Some((meta, name)) = rec.split_once('\t') else {
            continue;
        };
        let fields: Vec<&str> = meta.split(' ').collect();
        if name == path && fields.len() == 3 && fields[1] == "blob" {
            return Ok(Some((fields[0].to_string(), fields[2].to_string())));
        }
    }
    Ok(None)
}

/// `path` is a folder in `rev`.
fn is_tree(root: &Root, rev: &str, path: &str) -> bool {
    let spec = format!(":(literal){path}");
    git_ok(root, &["ls-tree", "-z", rev, "--", &spec], false)
        .map(|o| {
            o.stdout
                .split(|b| *b == 0)
                .any(|r| String::from_utf8_lossy(r).split(' ').nth(1) == Some("tree"))
        })
        .unwrap_or(false)
}

/// A blob's size in bytes.
fn blob_size(root: &Root, oid: &str) -> Result<u64, String> {
    git_ok(root, &["cat-file", "-s", oid], false)?
        .text()
        .parse()
        .map_err(|_| format!("git gave no size for {oid}"))
}

/// A blob's bytes. One larger than [`OUT_CAP`] is refused, never cut.
fn blob(root: &Root, oid: &str) -> Result<Vec<u8>, String> {
    let size = blob_size(root, oid)?;
    if size > OUT_CAP as u64 {
        return Err(format!("a file of {size} bytes is too large to sync"));
    }
    let bytes = git_ok(root, &["cat-file", "blob", oid], false)?.stdout;
    if bytes.len() as u64 != size {
        return Err(format!("git returned part of {oid}"));
    }
    Ok(bytes)
}

// ── Status and state ────────────────────────────────────────────────────

/// A root's sync state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncState {
    Off,
    Idle,
    Syncing,
    Blocked,
    Offline,
    Clash,
    /// Committed on this Mac; the root has no remote (the vault root until
    /// V6).
    #[serde(rename = "localOnly")]
    LocalOnly,
}

/// What the last cycle left, in `<id>.state.json` (app data).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct SavedState {
    pub state: Option<SyncState>,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    /// The file and rules that block the root, while it is blocked.
    pub blocked_by: Option<String>,
    /// The copies the last clash wrote (work-tree relative).
    pub clash_copies: Vec<String>,
    /// The last Waiting on you item raised, so a cycle that finds the same
    /// problem again doesn't raise it twice.
    pub last_alert: Option<String>,
    /// The remote branch's tip after the last fetch, to notice a rewrite.
    pub remote_tip: Option<String>,
    /// Pushes the remote rejected in a row.
    pub rejected_pushes: u32,
    /// Synced paths whose remote version holds a secret: never written into
    /// the vault and left out of every `add`, until the remote changes them.
    pub withheld: Vec<String>,
    /// Open clashes, one card each (CX6c), until the person resolves one or
    /// its copy is gone.
    pub clashes: Vec<clash::ClashRecord>,
    /// Copies the person kept on purpose (Keep both, Done): the conflict
    /// copies item never offers them again while they are there.
    pub resolved_copies: Vec<String>,
}

fn load_state(root: &Root) -> SavedState {
    fs::read_to_string(root.state_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_state(root: &Root, state: &SavedState) {
    let text = match serde_json::to_string_pretty(state) {
        Ok(t) => t,
        Err(_) => return,
    };
    if let Err(e) = fs::create_dir_all(&root.sync_dir)
        .map_err(|e| e.to_string())
        .and_then(|_| crate::vault::atomic_write_private(&root.state_path(), text.as_bytes()))
    {
        eprintln!("[context-sync] {}: state not saved: {e}", root.id);
    }
}

/// What Settings and the status pill show for one root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub root: String,
    pub state: SyncState,
    pub enabled: bool,
    pub remote: Option<String>,
    /// Local commits the remote doesn't have yet.
    pub pending_commits: u32,
    /// Files in the synced folders changed since the last commit (the quiet
    /// timer hasn't committed them yet).
    pub unsynced_files: u32,
    /// The one line the pill shows ("2 changes not synced").
    pub text: String,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    pub blocked_by: Option<String>,
    pub clash_copies: Vec<String>,
    /// Open clash cards (CX6c).
    pub clashes: u32,
}

/// Local commits not on the remote: `HEAD` past the remote branch, or
/// every commit while the remote has none.
fn pending_commits(root: &Root) -> u32 {
    if !root.git_dir.is_dir() || rev(root, "HEAD").is_none() {
        return 0;
    }
    let range = match rev(root, &root.remote_ref()) {
        Some(_) => format!("{}..HEAD", root.remote_ref()),
        None => "HEAD".to_string(),
    };
    git_ok(root, &["rev-list", "--count", &range], false)
        .ok()
        .and_then(|o| o.text().parse().ok())
        .unwrap_or(0)
}

/// Files in the synced folders that differ from `HEAD` (changed, removed
/// or new), leaving out entries the engine never writes.
fn unsynced_files(root: &Root) -> u32 {
    if !root.git_dir.join("HEAD").is_file() {
        return 0;
    }
    let mut specs = pathspecs(root);
    specs.extend(exclusions(root));
    specs.push(":(exclude,glob)**/.DS_Store".into());
    let mut files = BTreeSet::new();
    if let Some(head) = rev(root, "HEAD") {
        let mut args = vec![
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            head.as_str(),
            "--",
        ];
        args.extend(specs.iter().map(String::as_str));
        if let Ok(o) = git_ok(root, &args, true) {
            files.extend(
                o.stdout
                    .split(|b| *b == 0)
                    .filter(|f| !f.is_empty())
                    .map(<[u8]>::to_vec),
            );
        }
    }
    if root.kind == RootKind::Vault {
        if let Ok(new) = vault::untracked(root) {
            files.extend(new.into_iter().map(String::into_bytes));
        }
        return files.len() as u32;
    }
    let mut args = vec!["ls-files", "--others", "-z", "--"];
    args.extend(specs.iter().map(String::as_str));
    if let Ok(o) = git_ok(root, &args, true) {
        files.extend(
            o.stdout
                .split(|b| *b == 0)
                .filter(|f| !f.is_empty())
                .map(<[u8]>::to_vec),
        );
    }
    files.len() as u32
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `root`'s status now. Reads only: the saved state, the lock (held means
/// a cycle is running) and one `rev-list`.
pub(crate) fn status_of(root: &Root) -> SyncStatus {
    let saved = load_state(root);
    // With no remote, no commit is waiting to be pushed anywhere.
    let pending = if root.remote.is_some() {
        pending_commits(root)
    } else {
        0
    };
    let unsynced = if root.enabled {
        unsynced_files(root)
    } else {
        0
    };
    let mut state = saved.state.unwrap_or(SyncState::Idle);
    if !root.enabled {
        state = SyncState::Off;
    } else if lock_held(root) {
        state = SyncState::Syncing;
    }
    let live_copies: Vec<String> = saved
        .clash_copies
        .iter()
        .filter(|c| fs::symlink_metadata(root.work_tree.join(c)).is_ok())
        .cloned()
        .collect();
    if state == SyncState::Clash && live_copies.is_empty() {
        state = SyncState::Idle;
    }
    let text = match state {
        SyncState::Off => "Sync is off".to_string(),
        SyncState::Syncing => "Syncing".to_string(),
        SyncState::Blocked => format!(
            "Not synced: {}",
            saved.blocked_by.as_deref().unwrap_or("a file needs a look")
        ),
        SyncState::LocalOnly if unsynced > 0 => format!(
            "Local only: {} not committed yet",
            plural(unsynced, "change", "changes")
        ),
        SyncState::LocalOnly => "Local only: committed on this Mac, no remote yet".to_string(),
        _ if pending + unsynced > 0 => format!(
            "{} not synced",
            plural(pending + unsynced, "change", "changes")
        ),
        SyncState::Clash => format!(
            "Kept both versions of {}",
            plural(live_copies.len() as u32, "file", "files")
        ),
        SyncState::Offline => "Offline".to_string(),
        SyncState::Idle => "Up to date".to_string(),
    };
    SyncStatus {
        root: root.id.clone(),
        state,
        enabled: root.enabled,
        remote: root.remote.as_deref().map(redact_urls),
        pending_commits: pending,
        unsynced_files: unsynced,
        text,
        last_sync_at: saved.last_sync_at,
        last_error: saved.last_error.as_deref().map(redact_urls),
        blocked_by: saved.blocked_by,
        clash_copies: live_copies,
        clashes: saved
            .clashes
            .iter()
            .filter(|c| fs::symlink_metadata(root.work_tree.join(&c.copy)).is_ok())
            .count() as u32,
    }
}

// ── The lock ────────────────────────────────────────────────────────────

fn open_lock(root: &Root) -> std::io::Result<fs::File> {
    fs::create_dir_all(&root.sync_dir)?;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.lock_path())
}

/// The root's cycle lock, `None` when another cycle (in any process) holds
/// it or it can't be opened.
fn take_lock(root: &Root) -> Option<HeldLock> {
    let file = open_lock(root).ok()?;
    HeldLock::try_take(file).ok()
}

/// A cycle holds the lock now.
fn lock_held(root: &Root) -> bool {
    if !root.lock_path().exists() {
        return false;
    }
    match open_lock(root) {
        Ok(file) => matches!(
            HeldLock::try_take(file),
            Err(std::fs::TryLockError::WouldBlock)
        ),
        Err(_) => false,
    }
}

/// Remove git's own lock files left in the git directory by a git that died
/// (older than `older_than`). Only called with the cycle lock held, so no
/// git of this engine is running on it.
fn clear_stale_git_locks(git_dir: &Path, older_than: Duration) {
    let mut candidates = vec![
        git_dir.join("index.lock"),
        git_dir.join("HEAD.lock"),
        git_dir.join("config.lock"),
        git_dir.join("packed-refs.lock"),
        git_dir.join("sync-merge.index.lock"),
    ];
    for sub in ["refs/heads", "refs/remotes/origin"] {
        if let Ok(read) = fs::read_dir(git_dir.join(sub)) {
            candidates.extend(
                read.filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "lock")),
            );
        }
    }
    for path in candidates {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|m| m.elapsed().ok())
            .is_some_and(|age| age >= older_than);
        if meta.is_file() && old {
            eprintln!("[context-sync] removing a stale {}", path.display());
            let _ = fs::remove_file(&path);
        }
    }
}

/// Some process's command line names `git_dir` (a git of this engine or one
/// left running by a Hollow that died). When none does, a git lock file in
/// it is stale whatever its age.
fn git_running_on(git_dir: &Path) -> bool {
    let needle = git_dir.to_string_lossy().into_owned();
    match Command::new("ps")
        .args(["-axww", "-o", "command="])
        .output()
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.contains(&needle)),
        // Can't tell: assume one runs, so only old locks go.
        Err(_) => true,
    }
}

// ── The cycle ───────────────────────────────────────────────────────────

/// Who runs a cycle and on which machine.
#[derive(Debug, Clone)]
pub(crate) struct CycleCtx {
    /// This machine's slug (commit trailers and copy names).
    pub machine: String,
    /// What started the cycle ("quiet timer", "Sync now").
    pub actor: String,
    /// The person allowed removals past the mass-removal guard, for this
    /// cycle only (Sync now with removals allowed).
    pub allow_deletions: bool,
}

/// A Waiting on you item a cycle raises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Alert {
    pub key: String,
    pub title: String,
    pub body: String,
    /// The work-tree relative file it is about.
    pub path: Option<String>,
}

/// How a cycle ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The root is off: nothing was read or written.
    Off,
    /// Another cycle holds the lock.
    Busy,
    /// Committed (when there was anything) and in step with the remote.
    Synced {
        committed: bool,
    },
    /// No remote: committed locally only.
    LocalOnly {
        committed: bool,
    },
    /// A staged file broke a rule: nothing committed.
    Blocked {
        file: String,
        rules: Vec<String>,
    },
    /// The remote couldn't be reached: commits wait.
    Offline {
        error: String,
    },
    /// Both versions of these files were kept (copies' paths).
    Clash {
        copies: Vec<String>,
    },
    /// A file changed under the merge, or the push lost a race: the next
    /// cycle retries.
    Retry {
        reason: String,
    },
    Failed {
        error: String,
    },
}

/// Why an entry (`mode`, `path`, size when known) is never synced: outside
/// the allow-list, a `.git`, `.` or `..` part (any case), a symlink, a nested
/// repository, an odd mode, or larger than [`OUT_CAP`].
fn refuse(root: &Root, path: &str, mode: &str, size: Option<u64>) -> Option<String> {
    if !root.allowed(path) {
        return Some(if root.kind == RootKind::Vault {
            "left out by the vault's ignore rules".into()
        } else {
            "outside the synced folders".into()
        });
    }
    if path
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == ".." || p.eq_ignore_ascii_case(".git"))
    {
        return Some("a path with a .git, . or .. part".into());
    }
    match mode {
        "120000" => return Some("a symlink".into()),
        "160000" => return Some("a nested git repository".into()),
        "100644" | "100755" | ABSENT_MODE => {}
        other => return Some(format!("an unsupported file mode {other}")),
    }
    match size {
        Some(n) if root.kind == RootKind::Vault && n > vault::MAX_BYTES => {
            Some(format!("larger than {} MB", vault::MAX_BYTES >> 20))
        }
        Some(n) if n > OUT_CAP as u64 => Some(format!("larger than {} MB", OUT_CAP >> 20)),
        _ => None,
    }
}

/// Why a change's new side is refused ([`refuse`], its size read). A size
/// git can't give is an error for the cycle, never a guess.
fn refuse_entry(root: &Root, c: &Change) -> Result<Option<String>, String> {
    let size = if c.new_mode == ABSENT_MODE || c.new_mode == "160000" {
        None
    } else {
        Some(blob_size(root, &c.new_oid).map_err(|e| format!("{}: {e}", c.path))?)
    };
    if let Some(why) = refuse(root, &c.path, &c.new_mode, size) {
        return Ok(Some(why));
    }
    // The vault root keeps text only: a tracked file that turned binary
    // keeps its last version.
    if root.kind == RootKind::Vault
        && matches!(c.new_mode.as_str(), "100644" | "100755")
        && vault::looks_binary(&blob(root, &c.new_oid).map_err(|e| format!("{}: {e}", c.path))?)
    {
        return Ok(Some("not a text file".into()));
    }
    Ok(None)
}

/// The synced files in `rev` (none on an unborn branch).
fn tracked_files(root: &Root, rev: Option<&str>) -> Result<Vec<String>, String> {
    let Some(rev) = rev else {
        return Ok(Vec::new());
    };
    let mut args = vec!["ls-tree", "-r", "-z", "--name-only", rev, "--"];
    let specs = tree_pathspecs(root);
    args.extend(specs.iter().map(String::as_str));
    Ok(git_ok(root, &args, false)?
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

/// Removing `removed` of `tracked` synced files at once is a likely
/// accident (an evicted folder, a bad remote) unless the person says so.
fn too_many_removed(removed: usize, tracked: usize) -> bool {
    removed > MAX_DELETIONS || (removed >= 2 && removed * 2 > tracked)
}

/// What the person does to let a refused removal through.
const ALLOW_REMOVALS: &str = "if it was meant, use Sync now and allow removals (one sync only)";

/// Index entries in the synced folders the engine never writes (a symlink
/// or oversized file the remote holds): left out of every `add`, so their
/// absence from the vault is never staged as a removal.
fn refused_in_index(root: &Root) -> Vec<(String, String)> {
    let mut args = vec![
        "ls-files".to_string(),
        "-s".into(),
        "-z".into(),
        "--".into(),
    ];
    args.extend(pathspecs(root));
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let Ok(out) = git_ok(root, &argv, false) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for rec in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let Some((meta, path)) = rec.split_once('\t') else {
            continue;
        };
        let fields: Vec<&str> = meta.split(' ').collect();
        if fields.len() == 3 {
            entries.push((
                fields[0].to_string(),
                fields[1].to_string(),
                path.to_string(),
            ));
        }
    }
    let input: String = entries
        .iter()
        .map(|(_, oid, _)| format!("{oid}\n"))
        .collect();
    let sizes: BTreeMap<String, u64> = git_with(
        root,
        &["cat-file", "--batch-check=%(objectname) %(objectsize)"],
        false,
        Some(input.into_bytes()),
        &[],
    )
    .map(|o| {
        o.text()
            .lines()
            .filter_map(|l| {
                let (oid, n) = l.split_once(' ')?;
                Some((oid.to_string(), n.parse().ok()?))
            })
            .collect()
    })
    .unwrap_or_default();
    entries
        .into_iter()
        .filter_map(|(mode, oid, path)| {
            refuse(root, &path, &mode, sizes.get(&oid).copied()).map(|why| (path, why))
        })
        .collect()
}

/// [`refused_in_index`] as exclude pathspecs.
/// Also the files withheld from the vault (a remote version holding a
/// secret), which were never written here.
fn exclusions(root: &Root) -> Vec<String> {
    excluded_paths(root)
        .into_iter()
        .map(|p| format!(":(exclude,literal){p}"))
        .collect()
}

/// The paths [`exclusions`] names.
fn excluded_paths(root: &Root) -> BTreeSet<String> {
    let mut paths: BTreeSet<String> = refused_in_index(root).into_iter().map(|(p, _)| p).collect();
    paths.extend(load_state(root).withheld);
    paths
}

/// Put the index back to `HEAD` (or empty on an unborn branch). The work
/// tree isn't touched.
fn unstage(root: &Root) {
    let result = match rev(root, "HEAD") {
        Some(head) => git_ok(root, &["read-tree", &head], false),
        None => git_ok(root, &["read-tree", "--empty"], false),
    };
    if let Err(e) = result {
        eprintln!("[context-sync] {}: index not reset: {e}", root.id);
    }
}

/// Create the git directory on first use (a bare repository: git never
/// guesses a work tree for it) and keep the remote URL current.
fn ensure_repo(root: &Root) -> Result<(), String> {
    fs::create_dir_all(&root.sync_dir).map_err(|e| e.to_string())?;
    if !root.git_dir.join("HEAD").is_file() {
        let branch = format!("--initial-branch={}", root.branch);
        let out = Command::new("git")
            .args(["init", "--quiet", "--bare", &branch])
            .arg(&root.git_dir)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("git init did not start: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git init failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        if root.kind != RootKind::Vault {
            let mut exclude =
                String::from("# Hollow context sync: only the synced folders are tracked.\n/*\n");
            for a in &root.allow {
                exclude.push_str(&format!("!/{a}/\n"));
            }
            let info = root.git_dir.join("info");
            fs::create_dir_all(&info).map_err(|e| e.to_string())?;
            fs::write(info.join("exclude"), exclude).map_err(|e| e.to_string())?;
        }
    }
    // The vault root's ignore is generated from its manifest and rewritten
    // whenever that changes (only the git directory is written).
    if root.kind == RootKind::Vault {
        vault::ensure_exclude(root, &vault::MANIFEST)?;
    }
    let current = git_run(
        root,
        &["config", "--get", "remote.origin.url"],
        false,
        None,
        &[],
        LOCAL_TIMEOUT,
    )?;
    match &root.remote {
        Some(url) if current.code != 0 => {
            git_ok(root, &["remote", "add", "--", "origin", url], false)?;
        }
        Some(url) if current.text() != *url => {
            git_ok(root, &["remote", "set-url", "--", "origin", url], false)?;
        }
        _ => {}
    }
    Ok(())
}

/// The commit identity: the user's git identity when set, else Hollow on
/// this machine.
fn identity_env(root: &Root, machine: &str) -> Vec<(&'static str, String)> {
    let set = |key: &str| {
        git_run(
            root,
            &["config", "--get", key],
            false,
            None,
            &[],
            LOCAL_TIMEOUT,
        )
        .ok()
        .is_some_and(|o| o.code == 0 && !o.text().is_empty())
    };
    if set("user.name") && set("user.email") {
        return Vec::new();
    }
    let name = format!("Hollow ({machine})");
    let email = format!("hollow@{machine}.local");
    vec![
        ("GIT_AUTHOR_NAME", name.clone()),
        ("GIT_AUTHOR_EMAIL", email.clone()),
        ("GIT_COMMITTER_NAME", name),
        ("GIT_COMMITTER_EMAIL", email),
    ]
}

/// Stage the allow-list: every add, change and removal inside it, nothing
/// outside it. The vault's own ignore files can't hide these folders
/// (`-f`); `.DS_Store` stays out. Returns the files the vault root left at
/// their last version (`path (why)`), to be named.
fn stage(root: &Root) -> Result<Vec<String>, String> {
    if root.kind == RootKind::Vault {
        return vault::stage(root);
    }
    let mut specs = Vec::new();
    for a in &root.allow {
        let on_disk = fs::symlink_metadata(root.work_tree.join(a)).is_ok();
        let tracked = !git_ok(
            root,
            &["ls-files", "-z", "--", &format!(":(literal){a}")],
            true,
        )?
        .stdout
        .is_empty();
        if on_disk || tracked {
            specs.push(format!(":(literal){a}"));
        }
    }
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    specs.extend(exclusions(root));
    let mut args = vec!["add", "-A", "-f", "--"];
    args.extend(specs.iter().map(String::as_str));
    args.push(":(exclude,glob)**/.DS_Store");
    let out = git_run(root, &args, true, None, &[], ADD_TIMEOUT)?;
    if out.code != 0 {
        return Err(format!("git add failed: {}", out.stderr));
    }
    Ok(Vec::new())
}

/// Put one staged path back to `HEAD`'s entry (or out of the index when
/// `HEAD` has none), leaving the file alone.
fn unstage_path(root: &Root, c: &Change) -> Result<(), String> {
    let line = if c.old_mode == ABSENT_MODE {
        format!("0 {}\t{}", "0".repeat(c.old_oid.len().max(40)), c.path)
    } else {
        format!("{} {}\t{}", c.old_mode, c.old_oid, c.path)
    };
    let mut input = line.into_bytes();
    input.push(0);
    git_with(
        root,
        &["update-index", "-z", "--replace", "--index-info"],
        false,
        Some(input),
        &[],
    )
    .map(drop)
}

/// The staged changes against `HEAD` (or everything staged on an unborn
/// branch).
fn staged_changes(root: &Root) -> Result<Vec<Change>, String> {
    let base = match rev(root, "HEAD") {
        Some(h) => h,
        None => empty_tree(root)?,
    };
    let out = git_ok(
        root,
        &[
            "diff-index",
            "--cached",
            "-z",
            "--no-renames",
            "--raw",
            &base,
        ],
        false,
    )?;
    parse_raw(&out.stdout)
}

/// Scan one staged blob with the shared rules; the blocking findings' rule
/// ids.
fn scan_blob(root: &Root, c: &Change) -> Result<Vec<String>, String> {
    Ok(scan_for(root, &c.path, &blob(root, &c.new_oid)?))
}

/// The blocking rule ids the shared scan finds in `bytes` at `path`.
fn scan_for(root: &Root, path: &str, bytes: &[u8]) -> Vec<String> {
    let opts = ScanOptions {
        layer: root.scan_layer(path),
        groups: None,
        // Agents write these files: an allow line in one must not wave its
        // own secret through.
        honor_allows: false,
    };
    let mut rules: Vec<String> = scan_rules::scan_bytes(bytes, path, opts)
        .into_iter()
        .filter(|f| f.severity == Severity::Block)
        .map(|f| f.rule.to_string())
        .collect();
    rules.sort();
    rules.dedup();
    rules
}

/// The commit message: trigger, files, machine trailer.
fn message(title: &str, files: &[String], ctx: &CycleCtx) -> String {
    let mut m = format!("{title}\n\n");
    for f in files.iter().take(LISTED) {
        m.push_str(f);
        m.push('\n');
    }
    if files.len() > LISTED {
        m.push_str(&format!("and {} more\n", files.len() - LISTED));
    }
    m.push_str(&format!(
        "\nHollow-Actor: {}\n{MACHINE_TRAILER}: {}\n",
        ctx.actor, ctx.machine
    ));
    m
}

/// Commit the index when it differs from `HEAD`; whether it did.
fn commit_index(root: &Root, changes: &[Change], ctx: &CycleCtx) -> Result<bool, String> {
    let tree = git_ok(root, &["write-tree"], false)?.text();
    let head = rev(root, "HEAD");
    let base_tree = match &head {
        Some(h) => git_ok(root, &["rev-parse", &format!("{h}^{{tree}}")], false)?.text(),
        None => empty_tree(root)?,
    };
    if base_tree == tree {
        return Ok(false);
    }
    let files: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
    let title = format!(
        "Hollow sync from {}: {}",
        ctx.machine,
        plural(files.len() as u32, "file", "files")
    );
    let msg = message(&title, &files, ctx);
    let mut args = vec!["commit-tree", tree.as_str()];
    if let Some(h) = &head {
        args.extend(["-p", h.as_str()]);
    }
    let env = identity_env(root, &ctx.machine);
    let commit = git_with(root, &args, false, Some(msg.into_bytes()), &env)?.text();
    let old = head.unwrap_or_default();
    let local = root.local_ref();
    git_ok(
        root,
        &["update-ref", "-m", "context sync", &local, &commit, &old],
        false,
    )?;
    Ok(true)
}

/// The machine that last wrote `path` on `rev` (its trailer), else
/// `"remote"`.
fn writer_of(root: &Root, rev: &str, path: &str) -> String {
    let fmt = format!("--format=%(trailers:key={MACHINE_TRAILER},valueonly,separator=%x20)");
    let spec = format!(":(literal){path}");
    git_ok(root, &["log", "-1", &fmt, rev, "--", &spec], false)
        .ok()
        .map(|o| o.text())
        .and_then(|t| t.split_whitespace().next().map(str::to_string))
        .filter(|s| roots::valid_slug(s).is_ok())
        .unwrap_or_else(|| "remote".to_string())
}

/// `dir/Name (from <machine>).ext`, or `(from <machine> N)` when that is
/// taken (`taken` says so).
pub(crate) fn copy_name(path: &str, machine: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => (&file[..i], &file[i..]),
        _ => (file, ""),
    };
    let mut n = 1;
    loop {
        let tag = if n == 1 {
            format!("from {machine}")
        } else {
            format!("from {machine} {n}")
        };
        let candidate = format!("{dir}{stem} ({tag}){ext}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// The merge's result: the commit to move to and the copies it made.
struct Merged {
    commit: String,
    copies: Vec<String>,
    /// One record per copy (CX6c).
    clashes: Vec<clash::ClashRecord>,
}

/// Merge `theirs` into `head` in the object store (`merge-tree`): no file is
/// written. A clean merge is one commit with both parents; a clash keeps
/// `head`'s version at each conflicted name and `theirs`' beside it.
fn merge(root: &Root, head: &str, theirs: &str, ctx: &CycleCtx) -> Result<Merged, String> {
    let out = git_run(
        root,
        &[
            "merge-tree",
            "--write-tree",
            "-z",
            "--name-only",
            "--allow-unrelated-histories",
            head,
            theirs,
        ],
        false,
        None,
        &[],
        LOCAL_TIMEOUT,
    )?;
    if out.code != 0 && out.code != 1 {
        return Err(format!("git merge-tree failed: {}", out.stderr));
    }
    let mut fields = out.stdout.split(|b| *b == 0);
    let tree = String::from_utf8_lossy(fields.next().unwrap_or_default())
        .trim()
        .to_string();
    let mut conflicted = BTreeSet::new();
    if out.code == 1 {
        for f in fields {
            if f.is_empty() {
                break;
            }
            conflicted.insert(
                String::from_utf8(f.to_vec()).map_err(|_| "a conflicted file name isn't UTF-8")?,
            );
        }
        if conflicted.is_empty() {
            return Err("git merge-tree reported a conflict without a file".into());
        }
    }
    let env = identity_env(root, &ctx.machine);
    let mut copies = Vec::new();
    let mut clashes = Vec::new();
    let now = chrono::Utc::now().to_rfc3339();
    let record =
        |path: &str, copy: &str, mine: &str, theirs: &str, hashes: Option<(String, String)>| {
            let (mine_hash, theirs_hash) = hashes.clone().unwrap_or_default();
            clash::ClashRecord {
                path: path.to_string(),
                copy: copy.to_string(),
                mine_machine: mine.to_string(),
                theirs_machine: theirs.to_string(),
                at: now.clone(),
                mine_hash,
                theirs_hash,
                replaceable: hashes.is_some(),
                editing: false,
            }
        };
    let tree = if conflicted.is_empty() {
        tree
    } else {
        // Resolve in a private index, never the real one.
        let index = root.git_dir.join("sync-merge.index");
        let _ = fs::remove_file(&index);
        let ienv: Vec<(&str, String)> =
            vec![("GIT_INDEX_FILE", index.to_string_lossy().into_owned())];
        git_with(root, &["read-tree", &tree], false, None, &ienv)?;
        let in_result = |p: &str| -> bool {
            entry(root, &tree, p).ok().flatten().is_some()
                || entry(root, head, p).ok().flatten().is_some()
                || entry(root, theirs, p).ok().flatten().is_some()
        };
        let mut info = Vec::new();
        let mut taken_names: BTreeSet<String> = BTreeSet::new();
        for path in &conflicted {
            if !root.allowed(path) {
                return Err(format!("{path} clashed outside the synced folders"));
            }
            let ours = entry(root, head, path)?;
            let theirs_e = entry(root, theirs, path)?;
            let mut set = |mode: &str, oid: &str, p: &str| {
                info.extend_from_slice(format!("{mode} {oid}\t{p}").as_bytes());
                info.push(0);
            };
            match (&ours, &theirs_e) {
                (Some((m, o)), Some((_, t))) if o == t => set(m, o, path),
                (Some((m, o)), Some((tm, t))) => {
                    set(m, o, path);
                    let machine = writer_of(root, theirs, path);
                    let copy = copy_name(path, &machine, &|c| {
                        taken_names.contains(c)
                            || in_result(c)
                            || fs::symlink_metadata(root.work_tree.join(c)).is_ok()
                    });
                    taken_names.insert(copy.clone());
                    set(tm, t, &copy);
                    let hashes = (clash::hash(&blob(root, o)?), clash::hash(&blob(root, t)?));
                    clashes.push(record(path, &copy, &ctx.machine, &machine, Some(hashes)));
                    copies.push(copy);
                }
                // One side's blob where the other has a folder: the blob
                // goes beside it, never over the folder's files.
                (Some((m, o)), None) if is_tree(root, theirs, path) => {
                    let copy = copy_name(path, &ctx.machine, &|c| {
                        taken_names.contains(c) || in_result(c)
                    });
                    taken_names.insert(copy.clone());
                    set(m, o, &copy);
                    let machine = writer_of(root, theirs, path);
                    clashes.push(record(path, &copy, &ctx.machine, &machine, None));
                    copies.push(copy);
                }
                (None, Some((m, t))) if is_tree(root, head, path) => {
                    let machine = writer_of(root, theirs, path);
                    let copy =
                        copy_name(path, &machine, &|c| taken_names.contains(c) || in_result(c));
                    taken_names.insert(copy.clone());
                    set(m, t, &copy);
                    clashes.push(record(path, &copy, &ctx.machine, &machine, None));
                    copies.push(copy);
                }
                (Some((m, o)), None) => set(m, o, path),
                (None, Some((m, t))) => set(m, t, path),
                // A name only the merge made (a file moved aside by a
                // file/directory clash, `name~branch`): keep the merge's
                // entry, which the marker check below vets, and name it.
                (None, None) => {
                    if entry(root, &tree, path)?.is_some() {
                        let machine = writer_of(root, theirs, path);
                        clashes.push(record(path, path, &ctx.machine, &machine, None));
                        copies.push(path.clone());
                    }
                }
            }
        }
        git_with(
            root,
            &["update-index", "-z", "--replace", "--index-info"],
            false,
            Some(info),
            &ienv,
        )?;
        let resolved = git_with(root, &["write-tree"], false, None, &ienv)?.text();
        let _ = fs::remove_file(&index);
        // No conflict marker reaches the result: a conflicted name whose
        // blob neither side wrote (the merge made it) is checked; a file
        // either Mac saved as it is passes, whatever it says.
        let written = side_blobs(root, &[head, theirs])?;
        for path in conflicted.iter().chain(copies.iter()) {
            if let Some((_, oid)) = entry(root, &resolved, path)? {
                if !written.contains(&oid) && has_markers(&blob(root, &oid)?) {
                    return Err(format!(
                        "{path} would hold conflict markers; nothing was merged"
                    ));
                }
            }
        }
        resolved
    };
    let title = if copies.is_empty() {
        format!("Hollow sync merge on {}", ctx.machine)
    } else {
        format!(
            "Hollow sync merge on {}: kept both versions of {}",
            ctx.machine,
            plural(copies.len() as u32, "file", "files")
        )
    };
    let msg = message(&title, &copies, ctx);
    let commit = git_with(
        root,
        &["commit-tree", &tree, "-p", head, "-p", theirs],
        false,
        Some(msg.into_bytes()),
        &env,
    )?
    .text();
    Ok(Merged {
        commit,
        copies,
        clashes,
    })
}

/// Every blob id in the synced folders of `revs`.
fn side_blobs(root: &Root, revs: &[&str]) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    for r in revs {
        let mut args = vec![
            "ls-tree".to_string(),
            "-r".into(),
            "-z".into(),
            r.to_string(),
            "--".into(),
        ];
        args.extend(tree_pathspecs(root));
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let o = git_ok(root, &argv, false)?;
        for rec in o.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let rec = String::from_utf8_lossy(rec);
            if let Some(oid) = rec.split('\t').next().and_then(|m| m.split(' ').nth(2)) {
                out.insert(oid.to_string());
            }
        }
    }
    Ok(out)
}

/// A line that starts a conflict marker (`<<<<<<< `, `>>>>>>> `).
fn has_markers(bytes: &[u8]) -> bool {
    bytes
        .split(|b| *b == b'\n')
        .any(|l| l.starts_with(b"<<<<<<< ") || l.starts_with(b">>>>>>> "))
}

/// Whether the work tree still holds `old` (`None`: no file) at `path`:
/// `Err` names why not (changed, appeared, removed, a symlink on the way).
#[cfg(test)]
fn holds(root: &Root, path: &str, old: Option<&[u8]>) -> Result<(), String> {
    holds_with(root, path, old, &BTreeSet::new())
}

/// The files under folder `full` (work-tree relative, recursive), leaving
/// out `.DS_Store`; `None` when it can't be read whole.
fn files_under(root: &Root, full: &Path) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut stack = vec![full.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).ok()? {
            let e = e.ok()?;
            let kind = e.file_type().ok()?;
            if kind.is_dir() {
                stack.push(e.path());
            } else if e.file_name() != ".DS_Store" {
                out.push(
                    e.path()
                        .strip_prefix(&root.work_tree)
                        .ok()?
                        .to_str()?
                        .to_string(),
                );
            }
        }
    }
    Some(out)
}

/// [`holds`], knowing which paths this same apply removes: a new file may
/// sit below a file being removed (a file that became a folder) or where a
/// folder holds nothing but files being removed (a folder that became a
/// file). An empty folder counts as no file.
fn holds_with(
    root: &Root,
    path: &str,
    old: Option<&[u8]>,
    removing: &BTreeSet<String>,
) -> Result<(), String> {
    let full = match roots::resolve_in_root(&root.work_tree, path) {
        Ok(full) => full,
        Err(roots::PathError::NotFolder(at))
            if old.is_none()
                && at
                    .strip_prefix(&root.work_tree)
                    .ok()
                    .and_then(Path::to_str)
                    .is_some_and(|rel| removing.contains(rel)) =>
        {
            return Ok(())
        }
        Err(e) => return Err(format!("{path}: {e}")),
    };
    let meta = match fs::symlink_metadata(&full) {
        Ok(m) => Some(m),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{path}: {e}")),
    };
    match (old, meta) {
        (None, None) => Ok(()),
        (None, Some(m))
            if m.is_dir()
                && files_under(root, &full)
                    .is_some_and(|files| files.iter().all(|f| removing.contains(f))) =>
        {
            Ok(())
        }
        (None, Some(_)) => Err(format!("{path} appeared since the sync started")),
        (Some(_), None) => Err(format!("{path} was removed since the sync started")),
        (Some(_), Some(m)) if !m.is_file() => Err(format!("{path} is no longer a plain file")),
        (Some(want), Some(_)) => {
            let now = fs::read(&full).map_err(|e| format!("{path}: {e}"))?;
            if now == want {
                Ok(())
            } else {
                Err(format!("{path} changed since the sync started"))
            }
        }
    }
}

/// Remove folder `full` when it holds nothing but `.DS_Store` files and
/// empty folders; anything else stays and is an error.
fn clear_empty_dir(full: &Path) -> Result<(), String> {
    for e in fs::read_dir(full).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let kind = e.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            clear_empty_dir(&e.path())?;
        } else if e.file_name() == ".DS_Store" {
            fs::remove_file(e.path()).map_err(|e| e.to_string())?;
        } else {
            return Err(format!("{} is not empty", full.display()));
        }
    }
    fs::remove_dir(full).map_err(|e| e.to_string())
}

/// After removing `rel`, remove the folders above it that are now empty
/// (a `.DS_Store` doesn't count), up to but not including its synced folder.
fn prune_parents(root: &Root, rel: &str) {
    let mut parts: Vec<&str> = rel.split('/').collect();
    parts.pop();
    while parts.len() > 1 {
        let dir = root.work_tree.join(parts.join("/"));
        let only_finder = fs::read_dir(&dir)
            .map(|rd| rd.flatten().all(|e| e.file_name() == ".DS_Store"))
            .unwrap_or(false);
        if !only_finder
            || fs::symlink_metadata(&dir)
                .map(|m| !m.is_dir())
                .unwrap_or(true)
        {
            return;
        }
        if clear_empty_dir(&dir).is_err() {
            return;
        }
        parts.pop();
    }
}

/// One change [`apply`] writes, with both sides' bytes read up front.
struct PlannedWrite {
    change: Change,
    old: Option<Vec<u8>>,
    new: Option<Vec<u8>>,
}

/// What [`apply`] left out.
#[derive(Debug, Default)]
struct Applied {
    /// `path (why)` for each entry not written.
    skipped: Vec<String>,
    /// Paths withheld after this apply (a remote version holding a secret).
    withheld: BTreeSet<String>,
}

#[cfg(test)]
thread_local! {
    /// Called with each path right before [`apply`]'s own check and write.
    pub(crate) static BEFORE_WRITE: std::cell::RefCell<Option<Box<dyn FnMut(&str)>>> =
        std::cell::RefCell::new(None);
}

/// Write `from..to`'s changes inside the allow-list into the work tree.
/// Refused entries (a symlink, a `.git` part, an oversized file) are left
/// out and named. A version holding a secret is withheld: not written, left
/// out of every `add`, and named, while the rest syncs (the fix belongs on
/// the remote or the Mac that pushed it). More removals than the guard
/// allows block the apply unless the person allowed them. Each file is
/// checked to still hold `from`'s bytes before the pass starts and again
/// right before its own write or removal; the settings are re-read before
/// each, so the off switch stops the writes.
fn apply(
    root: &Root,
    from: Option<&str>,
    to: &str,
    ctx: &CycleCtx,
    tracked: usize,
    withheld: &BTreeSet<String>,
) -> Result<Applied, Outcome> {
    let fail = |error: String| Outcome::Failed { error };
    let base = match from {
        Some(f) => f.to_string(),
        None => empty_tree(root).map_err(fail)?,
    };
    let mut args = vec![
        "diff-tree".to_string(),
        "-r".into(),
        "-z".into(),
        "--no-renames".into(),
        "--raw".into(),
        base,
        to.to_string(),
        "--".into(),
    ];
    args.extend(pathspecs(root));
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = git_ok(root, &argv, false).map_err(fail)?;
    let changes = parse_raw(&out.stdout).map_err(fail)?;
    let mut done = Applied {
        skipped: Vec::new(),
        withheld: withheld.clone(),
    };
    let mut writes = Vec::new();
    for c in changes {
        // Never written: a refused new side, or the removal of an entry the
        // engine never wrote (a symlink, a nested repository).
        let why = refuse_entry(root, &c).map_err(fail)?.or_else(|| {
            matches!(c.old_mode.as_str(), "120000" | "160000")
                .then(|| "a symlink or nested repository".to_string())
        });
        if let Some(why) = why {
            done.skipped.push(format!("{} ({why})", c.path));
            continue;
        }
        // A withheld file was never written here: its old side is no file.
        let was_withheld = done.withheld.remove(&c.path);
        let old = if c.old_mode == ABSENT_MODE || was_withheld {
            None
        } else {
            Some(blob(root, &c.old_oid).map_err(fail)?)
        };
        let new = if c.new_mode == ABSENT_MODE {
            None
        } else {
            Some(blob(root, &c.new_oid).map_err(fail)?)
        };
        if let Some(bytes) = &new {
            let rules = scan_for(root, &c.path, bytes);
            if !rules.is_empty() {
                done.withheld.insert(c.path.clone());
                done.skipped.push(format!(
                    "{} (the remote's version holds {}; not written here: remove it on the remote or on the Mac that pushed it)",
                    c.path,
                    rules.join(", ")
                ));
                continue;
            }
        }
        if was_withheld && new.is_none() {
            continue;
        }
        writes.push(PlannedWrite {
            change: c,
            old,
            new,
        });
    }
    let removing: BTreeSet<String> = writes
        .iter()
        .filter(|w| w.new.is_none())
        .map(|w| w.change.path.clone())
        .collect();
    if !ctx.allow_deletions && too_many_removed(removing.len(), tracked) {
        return Err(Outcome::Blocked {
            file: plural(removing.len() as u32, "file", "files"),
            rules: vec![format!(
                "the remote removes {} of {tracked} synced files, so none were removed here and nothing was pushed; {ALLOW_REMOVALS}",
                removing.len()
            )],
        });
    }
    for w in &writes {
        holds_with(root, &w.change.path, w.old.as_deref(), &removing)
            .map_err(|reason| Outcome::Retry { reason })?;
    }
    // Writes first, removals after, so a crash part way never leaves a
    // file's content only in git. A write that needs a removal first (below
    // a file being removed, or where a folder is being emptied) waits.
    let needs_removal = |w: &PlannedWrite| {
        let p = &w.change.path;
        removing
            .iter()
            .any(|r| p.starts_with(&format!("{r}/")) || r.starts_with(&format!("{p}/")))
    };
    let mut order: Vec<&PlannedWrite> = writes
        .iter()
        .filter(|w| w.new.is_some() && !needs_removal(w))
        .collect();
    order.extend(writes.iter().filter(|w| w.new.is_none()));
    order.extend(
        writes
            .iter()
            .filter(|w| w.new.is_some() && needs_removal(w)),
    );
    for w in order {
        let c = &w.change;
        #[cfg(test)]
        BEFORE_WRITE.with(|h| {
            if let Some(hook) = h.borrow_mut().as_mut() {
                hook(&c.path);
            }
        });
        if !root.still_enabled() {
            return Err(Outcome::Off);
        }
        // Again, right before this file's own write.
        holds_with(root, &c.path, w.old.as_deref(), &removing)
            .map_err(|reason| Outcome::Retry { reason })?;
        let path = root.work_tree.join(&c.path);
        match &w.new {
            None => {
                if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
                    fs::remove_file(&path).map_err(|e| fail(format!("{}: {e}", c.path)))?;
                }
                prune_parents(root, &c.path);
            }
            Some(bytes) => {
                if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
                    clear_empty_dir(&path).map_err(|e| Outcome::Retry { reason: e })?;
                }
                crate::vault::atomic_write(&path, bytes).map_err(fail)?;
                if c.new_mode == "100755" {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
                }
            }
        }
    }
    Ok(done)
}

/// Move `HEAD` (and the index) from `head` to `to`, after [`apply`] wrote
/// the files.
fn advance(root: &Root, head: Option<&str>, to: &str) -> Result<(), String> {
    let old = head.unwrap_or("").to_string();
    let local = root.local_ref();
    git_ok(
        root,
        &["update-ref", "-m", "context sync", &local, to, &old],
        false,
    )?;
    git_ok(root, &["read-tree", to], false)?;
    Ok(())
}

/// A fetch or push failure that means "the remote moved", not "offline".
fn rejected(stderr: &str) -> bool {
    stderr.contains("[rejected]")
        || stderr.contains("non-fast-forward")
        || stderr.contains("fetch first")
        || stderr.contains("stale info")
}

/// What a cycle noticed besides its outcome.
#[derive(Debug, Default)]
pub(crate) struct Notes {
    /// Paths left out of this cycle, with why (`me/x.md (a symlink)`).
    pub skipped: Vec<String>,
    /// The remote branch no longer contains the tip seen last time.
    pub rewritten: bool,
    /// The remote branch's tip after the fetch.
    pub remote_tip: Option<String>,
    /// The push was rejected (the remote moved or refuses it).
    pub push_rejected: bool,
    /// The withheld paths after this cycle's apply, when it ran.
    pub withheld: Option<Vec<String>>,
    /// The clashes this cycle's merge left in the work tree (set once the
    /// merge was written and `HEAD` moved, whatever the push did).
    pub clashes: Vec<clash::ClashRecord>,
    /// The copies those clashes wrote.
    pub copies: Vec<String>,
}

/// One sync cycle of `root` (see the module notes). `alert` is called for a
/// new Waiting on you item; the outcome is also saved as the root's state.
pub(crate) fn cycle(root: &Root, ctx: &CycleCtx, alert: &mut dyn FnMut(Alert)) -> Outcome {
    if !root.enabled {
        return Outcome::Off;
    }
    let Some(_lock) = take_lock(root) else {
        return Outcome::Busy;
    };
    let mut notes = Notes::default();
    let outcome = cycle_locked(root, ctx, &mut notes);
    record(root, &outcome, &notes, alert);
    outcome
}

fn cycle_locked(root: &Root, ctx: &CycleCtx, notes: &mut Notes) -> Outcome {
    let fail = |error: String| Outcome::Failed { error };
    if let Err(e) = roots::check_root(&root.work_tree) {
        return fail(format!("The vault can't be synced: {e}"));
    }
    for a in &root.allow {
        match roots::resolve_in_root(&root.work_tree, a) {
            Ok(_) => {}
            Err(roots::PathError::Symlink(_)) => {
                return Outcome::Blocked {
                    file: format!("{a}/"),
                    rules: vec!["a symlink".into()],
                }
            }
            Err(e) => return fail(format!("{a}/: {e}")),
        }
    }
    if let Err(e) = ensure_repo(root) {
        return fail(e);
    }
    // A git lock is stale when no git runs on this directory now (the cycle
    // lock is held, so none of this engine's), else only when old.
    let stale_after = if git_running_on(&root.git_dir) {
        STALE_GIT_LOCK
    } else {
        Duration::ZERO
    };
    clear_stale_git_locks(&root.git_dir, stale_after);

    // A tracked folder gone from the vault (an evicted or unmounted vault, a
    // move half done) is never read as "delete all of it".
    let head = rev(root, "HEAD");
    let tracked = match tracked_files(root, head.as_deref()) {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    // The vault root's folders come and go (a workspace renamed or
    // deleted): the person's allow_deletions lets one cycle past, and a
    // folder the manifest now leaves out is never guarded.
    let folders = match root.kind {
        RootKind::Vault if ctx.allow_deletions => Vec::new(),
        RootKind::Vault => vault::tracked_folders(&tracked),
        _ => root.allow.clone(),
    };
    for a in &folders {
        let prefix = format!("{a}/");
        let in_head = tracked.iter().any(|p| p.starts_with(&prefix));
        if in_head && fs::symlink_metadata(root.work_tree.join(a)).is_err() {
            return Outcome::Blocked {
                file: prefix,
                rules: vec!["missing from the vault (nothing was deleted)".into()],
            };
        }
    }

    // 1-3: stage, check, commit. A refused entry is left out and named; a
    // scan hit or a mass removal puts the index back and commits nothing.
    match stage(root) {
        Ok(skipped) => notes.skipped.extend(skipped),
        Err(e) => {
            unstage(root);
            return fail(e);
        }
    }
    let mut changes = match staged_changes(root) {
        Ok(c) => c,
        Err(e) => {
            unstage(root);
            return fail(e);
        }
    };
    let mut kept = Vec::new();
    for c in changes.drain(..) {
        let refused = match refuse_entry(root, &c) {
            Ok(r) => r,
            Err(e) => {
                unstage(root);
                return fail(e);
            }
        };
        match refused {
            Some(why) => {
                if let Err(e) = unstage_path(root, &c) {
                    unstage(root);
                    return fail(e);
                }
                notes.skipped.push(format!("{} ({why})", c.path));
            }
            None => kept.push(c),
        }
    }
    let changes = kept;
    let removed = changes.iter().filter(|c| c.new_mode == ABSENT_MODE).count();
    if !ctx.allow_deletions && too_many_removed(removed, tracked.len()) {
        unstage(root);
        return Outcome::Blocked {
            file: plural(removed as u32, "file", "files"),
            rules: vec![format!(
                "{removed} of {} synced files are gone from the vault, so nothing was committed or removed from the remote; {ALLOW_REMOVALS}",
                tracked.len()
            )],
        };
    }
    // A scan hit: the personal root commits nothing; the vault root leaves
    // that file at its last version, names it and commits the rest, so one
    // false positive never holds back the whole vault.
    let mut clean = Vec::with_capacity(changes.len());
    for c in changes {
        if c.new_mode == ABSENT_MODE {
            clean.push(c);
            continue;
        }
        match scan_blob(root, &c) {
            Ok(rules) if rules.is_empty() => clean.push(c),
            Ok(rules) if root.kind == RootKind::Vault => {
                if let Err(e) = unstage_path(root, &c) {
                    unstage(root);
                    return fail(e);
                }
                notes
                    .skipped
                    .push(format!("{} (holds {})", c.path, rules.join(", ")));
            }
            Ok(rules) => {
                unstage(root);
                return Outcome::Blocked {
                    file: c.path.clone(),
                    rules,
                };
            }
            Err(e) => {
                unstage(root);
                return fail(e);
            }
        }
    }
    let changes = clean;
    let committed = match commit_index(root, &changes, ctx) {
        Ok(c) => c,
        Err(e) => {
            unstage(root);
            return fail(e);
        }
    };
    // No remote (the vault root until V6): nothing is fetched or pushed.
    if root.remote.is_none() {
        return Outcome::LocalOnly { committed };
    }

    // 4: fetch.
    let fetch = git_run(
        root,
        &["fetch", "--quiet", "--no-tags", "--prune", "origin"],
        false,
        None,
        &[],
        NETWORK_TIMEOUT,
    );
    match fetch {
        Ok(o) if o.code == 0 => {}
        Ok(o) => return Outcome::Offline { error: o.stderr },
        Err(e) => return Outcome::Offline { error: e },
    }

    // 5: merge the remote branch, if it has one.
    let mut copies = Vec::new();
    let mut clashes = Vec::new();
    if let Some(theirs) = rev(root, &root.remote_ref()) {
        notes.remote_tip = Some(theirs.clone());
        if let Some(seen) = load_state(root).remote_tip {
            let known = rev(root, &seen).is_some();
            if !known || !is_ancestor(root, &seen, &theirs).unwrap_or(true) {
                notes.rewritten = true;
            }
        }
        let head = rev(root, "HEAD");
        let target = match &head {
            None => Some(theirs.clone()),
            Some(h) => match is_ancestor(root, &theirs, h) {
                Ok(true) => None,
                Ok(false) => match is_ancestor(root, h, &theirs) {
                    Ok(true) => Some(theirs.clone()),
                    Ok(false) => match merge(root, h, &theirs, ctx) {
                        Ok(m) => {
                            copies = m.copies;
                            clashes = m.clashes;
                            Some(m.commit)
                        }
                        Err(e) => return fail(e),
                    },
                    Err(e) => return fail(e),
                },
                Err(e) => return fail(e),
            },
        };
        if let Some(target) = target {
            let withheld: BTreeSet<String> = load_state(root).withheld.into_iter().collect();
            let count = match tracked_files(root, head.as_deref()) {
                Ok(t) => t.len(),
                Err(e) => return fail(e),
            };
            match apply(root, head.as_deref(), &target, ctx, count, &withheld) {
                Ok(done) => {
                    notes.skipped.extend(done.skipped);
                    notes.withheld = Some(done.withheld.into_iter().collect());
                }
                Err(outcome) => return outcome,
            }
            if let Err(e) = advance(root, head.as_deref(), &target) {
                return fail(e);
            }
            // Written and committed: the clash cards stand whatever the
            // push does next.
            notes.clashes = std::mem::take(&mut clashes);
            notes.copies = copies.clone();
        }
    }

    // 6: push what the remote lacks. Never forced.
    if pending_commits(root) > 0 {
        let spec = format!("{}:{}", root.local_ref(), root.local_ref());
        let push = git_run(
            root,
            &["push", "--quiet", "--porcelain", "origin", &spec],
            false,
            None,
            &[],
            NETWORK_TIMEOUT,
        );
        match push {
            Ok(o) if o.code == 0 => {}
            Ok(o) if rejected(&o.stderr) || rejected(&o.text()) => {
                notes.push_rejected = true;
                return Outcome::Retry {
                    reason: "the remote refused the push (it moved, or its history was rewritten)"
                        .into(),
                };
            }
            Ok(o) => return Outcome::Offline { error: o.stderr },
            Err(e) => return Outcome::Offline { error: e },
        }
    }
    if copies.is_empty() {
        Outcome::Synced { committed }
    } else {
        Outcome::Clash { copies }
    }
}

/// Pushes rejected in a row before the person is told.
const REJECTED_ALERT: u32 = 3;

/// Save `outcome` as the root's state and raise a Waiting on you item for a
/// new block, clash, skipped file or rewritten remote.
fn record(root: &Root, outcome: &Outcome, notes: &Notes, alert: &mut dyn FnMut(Alert)) {
    let _held = clash::STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut saved = load_state(root);
    let now = chrono::Utc::now().to_rfc3339();
    let mut raised: Vec<String> = Vec::new();
    let mut raise = |saved: &SavedState, raised: &mut Vec<String>, a: Alert| {
        let new = saved
            .last_alert
            .as_deref()
            .is_none_or(|keys| !keys.split('\u{1f}').any(|k| k == a.key));
        raised.push(a.key.clone());
        if new {
            alert(a);
        }
    };
    match outcome {
        Outcome::Off | Outcome::Busy => return,
        Outcome::Synced { .. } | Outcome::LocalOnly { .. } => {
            let clash_open = saved
                .clash_copies
                .iter()
                .any(|c| fs::symlink_metadata(root.work_tree.join(c)).is_ok());
            saved.state = Some(if clash_open {
                SyncState::Clash
            } else if matches!(outcome, Outcome::LocalOnly { .. }) {
                SyncState::LocalOnly
            } else {
                SyncState::Idle
            });
            saved.last_sync_at = Some(now);
            saved.last_error = None;
            saved.blocked_by = None;
            if !clash_open {
                saved.clash_copies.clear();
            }
        }
        Outcome::Blocked { file, rules } => {
            let why = rules.join(", ");
            saved.state = Some(SyncState::Blocked);
            saved.blocked_by = Some(format!("{file} ({why})"));
            saved.last_error = None;
            let a = Alert {
                key: format!("blocked:{file}:{why}"),
                title: format!("Context sync stopped: {file} needs a look"),
                body: format!(
                    "{file} wasn't synced: {why}. No file in the vault changed and nothing was pushed."
                ),
                path: Some(file.trim_end_matches('/').to_string()).filter(|p| root.allowed(&format!("{p}/x")) || root.allowed(p)),
            };
            raise(&saved, &mut raised, a);
        }
        Outcome::Offline { error } => {
            saved.state = Some(SyncState::Offline);
            saved.last_error = Some(redact_urls(error));
            saved.blocked_by = None;
        }
        // Each clash is a card of its own (CX6c, below), not an alert.
        Outcome::Clash { .. } => {
            saved.state = Some(SyncState::Clash);
            saved.last_sync_at = Some(now);
            saved.last_error = None;
            saved.blocked_by = None;
        }
        Outcome::Retry { reason } => {
            saved.last_error = Some(redact_urls(reason));
        }
        Outcome::Failed { error } => {
            saved.last_error = Some(redact_urls(error));
        }
    }
    if !notes.skipped.is_empty() {
        let list = notes.skipped.join(", ");
        if matches!(
            saved.state,
            Some(SyncState::Idle) | Some(SyncState::LocalOnly)
        ) {
            saved.state = Some(SyncState::Blocked);
            saved.blocked_by = Some(list.clone());
        }
        let a = Alert {
            key: format!("skipped:{list}"),
            title: format!(
                "Context sync left out {}",
                plural(notes.skipped.len() as u32, "file", "files")
            ),
            body: format!(
                "These weren't synced and the vault's copy wasn't touched: {list}. Everything else synced."
            ),
            path: None,
        };
        raise(&saved, &mut raised, a);
    }
    if notes.rewritten {
        let a = Alert {
            key: format!("rewritten:{}", notes.remote_tip.clone().unwrap_or_default()),
            title: "Context sync: the remote's history was rewritten".into(),
            body: "The remote branch no longer holds commits this Mac saw before (a force push). Nothing was lost here; this Mac merged what the remote has now. Check the remote if that wasn't you.".into(),
            path: None,
        };
        raise(&saved, &mut raised, a);
    }
    saved.rejected_pushes = if notes.push_rejected {
        saved.rejected_pushes + 1
    } else if matches!(outcome, Outcome::Synced { .. } | Outcome::Clash { .. }) {
        0
    } else {
        saved.rejected_pushes
    };
    if saved.rejected_pushes >= REJECTED_ALERT {
        let a = Alert {
            key: "rejected-pushes".into(),
            title: "Context sync: the remote keeps refusing pushes".into(),
            body: format!(
                "The last {} pushes were refused. The remote's history may have been rewritten, or the branch is protected. Local commits are kept and wait.",
                saved.rejected_pushes
            ),
            path: None,
        };
        raise(&saved, &mut raised, a);
    }
    if notes.remote_tip.is_some() {
        saved.remote_tip = notes.remote_tip.clone();
    }
    if let Some(w) = &notes.withheld {
        saved.withheld = w.clone();
    }
    for c in &notes.copies {
        if !saved.clash_copies.contains(c) {
            saved.clash_copies.push(c.clone());
        }
    }
    clash::merge_records(root, &mut saved, &notes.clashes);
    saved.last_alert = (!raised.is_empty()).then(|| raised.join("\u{1f}"));
    save_state(root, &saved);
}

// ── Quiet timer ─────────────────────────────────────────────────────────

/// How often the timer looks.
const TICK: Duration = Duration::from_secs(15);
/// How long the folders stay still before a cycle.
const QUIET: Duration = Duration::from_secs(60);
/// A cycle at least this often, to take in the other Mac's changes.
const PERIOD: Duration = Duration::from_secs(10 * 60);
/// The first look after launch.
const FIRST: Duration = Duration::from_secs(45);
/// Entries a fingerprint walks at most.
const WALK_MAX: usize = 50_000;

/// A cheap fingerprint of the allow-listed folders: each entry's path, size
/// and modification time. Symlinks are not followed.
pub(crate) fn fingerprint(root: &Root) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut stack: Vec<PathBuf> = root.allow.iter().map(|a| root.work_tree.join(a)).collect();
    let mut seen = 0usize;
    while let Some(path) = stack.pop() {
        seen += 1;
        if seen > WALK_MAX {
            break;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        path.hash(&mut hasher);
        meta.len().hash(&mut hasher);
        if let Ok(m) = meta.modified() {
            m.hash(&mut hasher);
        }
        if meta.is_dir() {
            if let Ok(read) = fs::read_dir(&path) {
                let mut kids: Vec<PathBuf> =
                    read.filter_map(Result::ok).map(|e| e.path()).collect();
                kids.sort();
                stack.extend(kids);
            }
        }
    }
    hasher.finish()
}

/// This machine's slug for commits and copy names: the saved one, else one
/// suggested from the Mac's name, else `this-mac`.
fn machine() -> String {
    roots::machine_slug()
        .or_else(roots::suggested_slug)
        .unwrap_or_else(|| "this-mac".to_string())
}

/// The personal root as configured now, for `vault`.
pub(crate) fn personal_root(vault: &Path) -> Result<Option<Root>, String> {
    configured_root(vault, PERSONAL)
}

/// Root `id` as configured now, for `vault`.
fn configured_root(vault: &Path, id: &str) -> Result<Option<Root>, String> {
    let path = settings_path()?;
    let config = config_at(&path);
    match config.root(id) {
        None => Ok(None),
        Some(c) => {
            let mut root = Root::from_config(c, &sync_dir()?, vault)?;
            root.settings_file = Some(path);
            Ok(Some(root))
        }
    }
}

/// A Waiting on you item through the notification center (an unread error
/// is listed by `needs_you` until it is read). Logged when no window is
/// registered.
fn notify(vault: &Path, a: Alert) {
    let input = crate::notifications::NotificationInput {
        title: a.title.clone(),
        body: a.body,
        kind: "error".into(),
        source: Some("context-sync".into()),
        action_label: a.path.as_ref().map(|_| "Open".into()),
        action_url: a.path.map(|p| format!("vault:{p}")),
        notify_native: false,
    };
    match crate::notifications::create_registered(vault, input) {
        Some(Ok(_)) => {}
        Some(Err(e)) => eprintln!("[context-sync] {}: not raised: {e}", a.title),
        None => eprintln!("[context-sync] {} (no window to tell)", a.title),
    }
}

/// Run one cycle of the personal root now (Sync now, the timer).
pub(crate) fn run_personal(
    vault: &Path,
    actor: &str,
    allow_deletions: bool,
) -> Result<(Outcome, SyncStatus), String> {
    let root = personal_root(vault)?.ok_or("Context sync isn't set up.")?;
    let ctx = CycleCtx {
        machine: machine(),
        actor: actor.to_string(),
        allow_deletions,
    };
    let outcome = cycle(&root, &ctx, &mut |a| notify(vault, a));
    Ok((outcome, status_of(&root)))
}

/// Starts the quiet timer (one thread for the app's life). Each tick
/// re-reads the settings, so turning sync on later works and turning it off
/// stops the next cycle.
pub(crate) fn spawn_timer(vault: PathBuf) {
    std::thread::Builder::new()
        .name("context-sync".into())
        .spawn(move || {
            std::thread::sleep(FIRST);
            let mut last_print: Option<u64> = None;
            let mut changed_at = Instant::now();
            let mut last_cycle: Option<Instant> = None;
            let mut dirty = true;
            loop {
                if let Ok(Some(root)) = personal_root(&vault) {
                    if root.enabled && root.remote.is_some() {
                        let print = fingerprint(&root);
                        if last_print != Some(print) {
                            last_print = Some(print);
                            changed_at = Instant::now();
                            dirty = true;
                        }
                        let quiet = changed_at.elapsed() >= QUIET;
                        let due = last_cycle.is_none_or(|t| t.elapsed() >= PERIOD);
                        if (dirty && quiet) || due {
                            match run_personal(&vault, "quiet timer", false) {
                                Ok((Outcome::Busy, _)) => {}
                                Ok((Outcome::Retry { .. }, _)) => {
                                    last_cycle = Some(Instant::now());
                                }
                                Ok(_) => {
                                    dirty = false;
                                    last_cycle = Some(Instant::now());
                                    last_print = Some(fingerprint(&root));
                                }
                                Err(e) => {
                                    eprintln!("[context-sync] {e}");
                                    last_cycle = Some(Instant::now());
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(TICK);
            }
        })
        .map(drop)
        .unwrap_or_else(|e| eprintln!("[context-sync] no timer: {e}"));
}

// ── Commands ────────────────────────────────────────────────────────────

/// The personal root's status (Settings, the status pill). Reads only.
pub(crate) async fn personal_status(vault: PathBuf) -> Result<SyncStatus, String> {
    crate::vault::spawn_blocking_logged(move || {
        Ok(match personal_root(&vault)? {
            Some(root) => status_of(&root),
            None => SyncStatus {
                root: PERSONAL.into(),
                state: SyncState::Off,
                enabled: false,
                remote: None,
                pending_commits: 0,
                unsynced_files: 0,
                text: "Not set up".into(),
                last_sync_at: None,
                last_error: None,
                blocked_by: None,
                clash_copies: Vec::new(),
                clashes: 0,
            },
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn context_sync_status(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<SyncStatus, String> {
    personal_status(state.config.vault_path.clone()).await
}

/// Sync now: one cycle off the UI thread; the status after it.
#[tauri::command]
/// `allow_deletions`: the person confirms the removals the mass-removal
/// guard stopped, for this one cycle (person-only, like the command).
pub async fn context_sync_now(
    state: tauri::State<'_, crate::commands::AppState>,
    allow_deletions: Option<bool>,
) -> Result<SyncStatus, String> {
    let vault = state.config.vault_path.clone();
    let allow = allow_deletions == Some(true);
    let actor = if allow {
        "Sync now, removals allowed"
    } else {
        "Sync now"
    };
    crate::vault::spawn_blocking_logged(move || run_personal(&vault, actor, allow).map(|(_, s)| s))
        .await
        .map_err(|e| e.to_string())?
}

/// The off switch (and back on). Off leaves every file as it is.
#[tauri::command]
pub async fn set_context_sync_enabled(
    state: tauri::State<'_, crate::commands::AppState>,
    enabled: bool,
) -> Result<SyncStatus, String> {
    set_enabled(PERSONAL, enabled)?;
    personal_status(state.config.vault_path.clone()).await
}

/// Connect the personal root to a remote (an empty repository or one this
/// sync wrote) and turn it on; the quiet timer runs the first cycle.
#[tauri::command]
pub async fn configure_personal_sync(
    state: tauri::State<'_, crate::commands::AppState>,
    remote: String,
) -> Result<SyncStatus, String> {
    configure_personal(&remote)?;
    personal_status(state.config.vault_path.clone()).await
}

// ── The vault root (CX6b) ───────────────────────────────────────────────

/// Turn the vault root on (saved as kind `vault`, no remote, then one cycle:
/// the first commit, local only) or off (nothing read or written). Explicit
/// only: the vault root is never on by default and has no timer until V6.
pub(crate) fn configure_vault_at(settings: &Path, enabled: bool) -> Result<RootConfig, String> {
    let _held = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    update_root_at(settings, VAULT, |r| {
        r.kind = RootKind::Vault;
        if r.remote.is_some() {
            return Err("The vault root has a remote saved; V6 handles that.".into());
        }
        r.enabled = enabled;
        Ok(())
    })
}

/// The vault root's status, or "Not set up".
pub(crate) fn vault_status(vault: &Path) -> Result<SyncStatus, String> {
    Ok(match configured_root(vault, VAULT)? {
        Some(root) => status_of(&root),
        None => SyncStatus {
            root: VAULT.into(),
            state: SyncState::Off,
            enabled: false,
            remote: None,
            pending_commits: 0,
            unsynced_files: 0,
            text: "Not set up".into(),
            last_sync_at: None,
            last_error: None,
            blocked_by: None,
            clash_copies: Vec::new(),
            clashes: 0,
        },
    })
}

/// Set up (or switch off) the vault root's local git: `enabled` saves it on
/// and runs one cycle now (scan of the would-be tree, then the first commit
/// on this Mac; nothing is pushed: it has no remote until V6); off leaves
/// every file as it is. The person's choice, like the personal remote.
#[tauri::command]
/// `allow_deletions`: the person confirms, for this one cycle, the removals
/// or the missing top-level folder the guards stopped.
pub async fn configure_vault_sync(
    state: tauri::State<'_, crate::commands::AppState>,
    enabled: bool,
    allow_deletions: Option<bool>,
) -> Result<SyncStatus, String> {
    let vault = state.config.vault_path.clone();
    let allow = allow_deletions == Some(true);
    crate::vault::spawn_blocking_logged(move || {
        configure_vault_at(&settings_path()?, enabled)?;
        if enabled {
            let root = configured_root(&vault, VAULT)?.ok_or("The vault root isn't saved.")?;
            let ctx = CycleCtx {
                machine: machine(),
                actor: if allow {
                    "Vault sync, removals allowed".into()
                } else {
                    "Vault sync".into()
                },
                allow_deletions: allow,
            };
            cycle(&root, &ctx, &mut |a| notify(&vault, a));
        }
        vault_status(&vault)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Clash cards (CX6c) ──────────────────────────────────────────────────

/// The roots that can hold clash records: every configured root this build
/// knows (on or off: a clash left before the switch went off still waits).
fn clash_roots(vault: &Path) -> Result<Vec<Root>, String> {
    let mut out = Vec::new();
    for id in [PERSONAL, VAULT] {
        if let Some(root) = configured_root(vault, id)? {
            out.push(root);
        }
    }
    Ok(out)
}

/// Every open clash across the roots, oldest first. `contents` false (the
/// agents' read) leaves out every line of text.
pub(crate) fn clash_list(vault: &Path, contents: bool) -> Result<Vec<clash::SyncClash>, String> {
    Ok(clash_list_of(&clash_roots(vault)?, contents))
}

/// [`clash_list`] over `roots`.
pub(crate) fn clash_list_of(roots: &[Root], contents: bool) -> Vec<clash::SyncClash> {
    let mut out = Vec::new();
    for root in roots {
        out.extend(
            clash::open(root)
                .iter()
                .map(|r| clash::card(root, r, contents)),
        );
    }
    out.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.path.cmp(&b.path)));
    out
}

/// The copies the conflict copies item leaves out (open clashes and copies
/// the person kept), across the roots.
pub(crate) fn clash_skip_copies(vault: &Path) -> Result<BTreeSet<String>, String> {
    Ok(clash_roots(vault)?
        .iter()
        .flat_map(clash::skip_copies)
        .collect())
}

/// The person's choice on one card: `root` a root id, `path` the file as the
/// card names it (anything else is refused before a file is touched).
pub(crate) fn resolve_clash_in(
    vault: &Path,
    root_id: &str,
    path: &str,
    copy: &str,
    choice: &str,
) -> Result<clash::ClashResolved, String> {
    valid_id(root_id)?;
    let choice = clash::ClashChoice::parse(choice)?;
    let root = configured_root(vault, root_id)?
        .ok_or_else(|| format!("{root_id} isn't a context sync root here."))?;
    clash::resolve(&root, path, copy, choice)
}

/// The open clash cards, for the person's window (with both versions'
/// first lines).
#[tauri::command]
pub async fn sync_clashes(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Vec<clash::SyncClash>, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || clash_list(&vault, true))
        .await
        .map_err(|e| e.to_string())?
}

/// Keep mine, Keep theirs, Keep both, Edit or Done on one clash card. The
/// person's; the next cycle commits what it changed like any edit.
#[tauri::command]
pub async fn resolve_sync_clash(
    state: tauri::State<'_, crate::commands::AppState>,
    root: String,
    path: String,
    copy: String,
    choice: String,
) -> Result<clash::ClashResolved, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        resolve_clash_in(&vault, &root, &path, &copy, &choice)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[path = "sync_vault.rs"]
pub(crate) mod vault;
use vault::VAULT;

#[path = "sync_clash.rs"]
pub(crate) mod clash;

#[cfg(test)]
#[path = "sync_engine_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sync_vault_tests.rs"]
mod vault_tests;

#[cfg(test)]
#[path = "sync_clash_tests.rs"]
mod clash_tests;
