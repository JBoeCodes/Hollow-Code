//! Paperclip PR 15 (15a): per-run scratch space.
//!
//! Every agent run Hollow spawns with tools gets a private temporary folder
//! outside the vault, so downloads, intermediate files, test output and
//! throwaway scripts stop landing in the working directory (where they show
//! up as the run's changes). The folder is `<base>/<folder>`:
//!
//! - `<base>` is `<canonical temp dir>/hollow-run` only when that path is
//!   short (at most [`MAX_BASE_BYTES`]) and has no whitespace; otherwise
//!   `/private/tmp/hollow-run-<uid>` (`/tmp/…` off macOS). On macOS the
//!   canonical per-user temp dir alone is about 57 bytes, so the fallback is
//!   the norm there: the folder becomes the child's `TMPDIR`, and a Unix
//!   socket path (tsx's IPC pipe, Python multiprocessing, ssh ControlPath,
//!   Chromium's SingletonSocket) must fit in 104 bytes including the NUL.
//! - `<folder>` is [`folder_name`] of the logical name: the first 16 hex
//!   digits of its SHA-256, so the path stays short whatever the name. The
//!   logical name is the history run id for automations (workflows, the
//!   heartbeat's tool pass, ingest) and the Hollow session key for a chat
//!   turn; the marker records the run id. Claude Code lists `--add-dir`
//!   paths in its own environment block and announces changes on resume, so
//!   a per-run path in chat would change the prompt every turn and defeat
//!   caching. A chat turn therefore creates the session key's folder at
//!   admission and sets it aside at its finish or Stop; the next turn
//!   recreates the same path. Since PER-9 (11-14) the set-aside folder is
//!   kept as `.kept-<pid>-<nanos>-<folder>` beside it for six hours (at most
//!   three per conversation; an empty one is removed at once), so a Stop no
//!   longer deletes uncommitted work in scratch within seconds.
//! - The base and each run folder are 0700; each run folder carries a 0600
//!   owner marker (`.hollow-scratch`: pid, the pid's start stamp, the run id,
//!   the creation time). A run folder is created under a staging name
//!   (`.new-<pid>-<nanos>-<name>`), marked, then renamed into place, so a
//!   folder at its final name always has its marker.
//! - Removal ([`ScratchDir`]'s `Drop`) first renames the folder to a unique
//!   tombstone (`.dead-<pid>-<nanos>-<name>`), synchronously, and only then
//!   deletes the tombstone on a detached thread. A same-name folder created
//!   right after (the next chat turn, which the run queue may claim as soon as
//!   this one returns) is therefore never touched by the deletion.
//! - [`sweep_orphans`] runs once at startup: marked folders whose owner is
//!   gone (or that are older than 7 days) and staging or tombstone folders
//!   whose pid is gone (or older than a day) are removed. Unmarked folders and
//!   symlinks are never touched. `std::fs::remove_dir_all` does not follow
//!   symlinks, so nothing outside a scratch folder is ever deleted through one.
//!
//! Launch points set `RunExtras.scratch` (see `agent::apply_scratch`): a
//! [`ScratchSpec::Dir`] sets `TMPDIR`/`TEMP`/`TMP` (with a trailing slash) and
//! `HOLLOW_SCRATCH_DIR` on the child and gives Claude and Antigravity
//! `--add-dir`; Codex needs only `TMPDIR` (its `workspace-write` sandbox treats
//! `$TMPDIR` as a writable root, confirmed by a live probe on codex-cli
//! 0.153.4). When the folder cannot be created the spec is
//! [`ScratchSpec::Unavailable`]: only `HOLLOW_SCRATCH_DIR` is set, to a path
//! under `/dev/null` that can never exist, so the always-present guidance
//! never expands to an empty string (`rm -rf "$HOLLOW_SCRATCH_DIR"/*` matches
//! nothing, and a write fails with ENOTDIR instead of landing at `/`).
//!
//! The prompt only ever names the variable ([`SCRATCH_GUIDANCE`]), never a
//! path, so it stays byte-stable across turns (Claude caches the appended
//! prompt; the stint's configuration fingerprint never includes the argv or
//! the MCP bodies).
//!
//! A `hollow team-mcp` delegate never creates or removes anything: it reuses
//! its parent's folder when [`inherited`] validates it. Its `TMPDIR` is
//! already the parent's scratch folder (the server inherits the CLI's
//! environment), so the check is structural and never consults `TMPDIR`.
//! This module runs inside that MCP subprocess, so it must never create or
//! signal a process group; `proc_info` only probes with signal 0.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The child environment variable naming the run's scratch folder.
pub const SCRATCH_ENV: &str = "HOLLOW_SCRATCH_DIR";

/// Where `HOLLOW_SCRATCH_DIR` points when no folder could be created: under
/// `/dev/null`, so it can never be created and nothing ever resolves to `/`.
pub const UNAVAILABLE_PATH: &str = "/dev/null/hollow-scratch-unavailable";

/// The owner marker inside every run folder.
pub const MARKER: &str = ".hollow-scratch";

/// The byte-stable scratch guidance every tool-running prompt carries (the
/// chat's appended prompt, the workflow boundary, the heartbeat's tool pass,
/// ingest and validated delegates). It names only the variables, never a
/// path: a per-run path would change the chat prompt every turn.
pub const SCRATCH_GUIDANCE: &str = "Scratch space: this run has a private temporary folder at $HOLLOW_SCRATCH_DIR ($TMPDIR points there too). Put scratch and intermediate files there (downloads, intermediate drafts, test files, logs, throwaway scripts), never in the working directory or the vault, where they would show up as changes. Only deliverables belong in the working directory. The folder is deleted when the run ends.";

const BASE_NAME: &str = "hollow-run";
/// Longest base [`choose_base`] keeps under the temp dir. With a 16-digit
/// folder name the child's `TMPDIR` is then at most 58 bytes, which leaves
/// room for a socket name under it (see [`SOCKET_PATH_MAX`]).
pub const MAX_BASE_BYTES: usize = 40;
/// Hex digits of a folder name ([`folder_name`]).
const FOLDER_HEX: usize = 16;
/// The longest Unix socket path macOS accepts (`sun_path` is 104 bytes,
/// NUL included).
#[cfg(test)]
const SOCKET_PATH_MAX: usize = 103;
const TOMBSTONE_PREFIX: &str = ".dead-";
const STAGING_PREFIX: &str = ".new-";
/// A chat turn's folder set aside when the turn ends or is stopped (PER-9).
const KEPT_PREFIX: &str = ".kept-";
/// How long a chat turn's set-aside folder is kept (PER-9, 11-14): a Stop
/// no longer deletes uncommitted work in scratch within seconds.
const KEPT_GRACE_HOURS: i64 = 6;
/// At most this many set-aside folders per conversation, newest first.
const KEPT_PER_CHAT: usize = 3;
/// Longest accepted folder name (session keys and run ids are far shorter).
const MAX_NAME: usize = 128;
/// A marked folder older than this is removed by the sweep even when its
/// owner still runs (no run lasts a week).
const DIR_MAX_AGE_DAYS: i64 = 7;
/// A staging or tombstone folder older than this is removed regardless.
const TRANSIENT_MAX_AGE_DAYS: i64 = 1;
/// Markers are tiny; anything larger is not ours.
const MARKER_MAX_BYTES: u64 = 4096;

/// What a launch point hands the agent builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScratchSpec {
    /// A created, marked folder.
    Dir(PathBuf),
    /// Creation failed: only `HOLLOW_SCRATCH_DIR` is set, to [`UNAVAILABLE_PATH`].
    Unavailable,
}

impl ScratchSpec {
    /// The folder, when there is one.
    pub fn dir(&self) -> Option<&Path> {
        match self {
            ScratchSpec::Dir(dir) => Some(dir),
            ScratchSpec::Unavailable => None,
        }
    }
}

/// The owner marker's JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    pid: i32,
    /// The owner's process start stamp (`proc_info`), so a reused pid is not
    /// mistaken for the owner. `None` when it could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pid_start: Option<u64>,
    run_id: String,
    created_at: DateTime<Utc>,
}

/// A created scratch folder. Dropping it removes the folder: a synchronous
/// rename to a tombstone, then an off-thread delete of the tombstone. Keep it
/// in a NAMED local (never `_`, which drops at once) until the run's
/// processes are gone.
#[derive(Debug)]
pub struct ScratchDir {
    path: PathBuf,
    /// A chat turn's folder (named for its conversation, not its run): set
    /// aside for [`KEPT_GRACE_HOURS`] instead of deleted ([`keep`], PER-9).
    keep: bool,
}

impl ScratchDir {
    /// The folder (canonical).
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The spec a launch point puts on its `RunExtras`.
    pub fn spec(&self) -> ScratchSpec {
        ScratchSpec::Dir(self.path.clone())
    }

    /// Remove the folder now, on this thread (tests).
    #[cfg(test)]
    pub fn remove_now(self) {
        let path = self.path.clone();
        std::mem::forget(self);
        if let Some(tomb) = rename_to_tombstone(&path) {
            let _ = fs::remove_dir_all(tomb);
        }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if self.keep {
            keep(&self.path);
        } else {
            retire(&self.path);
        }
    }
}

/// The real base folder: `canonicalize(temp_dir())/hollow-run`, or the
/// `/private/tmp/hollow-run-<uid>` fallback (see [`choose_base`]).
#[cfg(not(test))]
pub fn base() -> PathBuf {
    choose_base(&std::env::temp_dir(), current_uid())
}

/// Test builds use one FIXED base (no pid, so leftovers are bounded to one
/// folder), whose dead-pid entries are swept on first use in each process.
/// Each [`ScratchDir`] removes itself on drop.
#[cfg(test)]
pub fn base() -> PathBuf {
    static SWEPT: std::sync::Once = std::sync::Once::new();
    let dir = std::env::temp_dir().join("hollow-run-tests");
    SWEPT.call_once(|| {
        let _ = sweep_in(
            &dir,
            Utc::now(),
            DateTime::<Utc>::MIN_UTC,
            own_pid(),
            &owner_alive,
        );
    });
    dir
}

/// Pick the base under `tmp`: `<canonical tmp>/hollow-run`, unless that path
/// holds whitespace, is not UTF-8 (the contract wants no spaces in the path)
/// or is longer than [`MAX_BASE_BYTES`] (the child's `TMPDIR` must leave room
/// for a Unix socket name), in which case `/private/tmp/hollow-run-<uid>`
/// (`/tmp/…` off macOS). The fallback is at most 34 bytes (a 10-digit uid).
pub fn choose_base(tmp: &Path, uid: u32) -> PathBuf {
    let canonical = fs::canonicalize(tmp).unwrap_or_else(|_| tmp.to_path_buf());
    let candidate = canonical.join(BASE_NAME);
    match candidate.to_str() {
        Some(text) if !text.chars().any(char::is_whitespace) && text.len() <= MAX_BASE_BYTES => {
            candidate
        }
        _ => fallback_base(uid),
    }
}

/// The on-disk folder for a logical name (a run id or a session key): the
/// first 16 hex digits of its SHA-256. Stable for the same name (a chat
/// session keeps one path across turns) and always short.
pub fn folder_name(name: &str) -> String {
    use sha2::{Digest, Sha256};
    let hex = format!("{:x}", Sha256::digest(name.as_bytes()));
    hex[..FOLDER_HEX].to_string()
}

fn fallback_base(uid: u32) -> PathBuf {
    #[cfg(target_os = "macos")]
    let root = "/private/tmp";
    #[cfg(not(target_os = "macos"))]
    let root = "/tmp";
    PathBuf::from(root).join(format!("{BASE_NAME}-{uid}"))
}

/// Create `<base>/<name>`: base and folder 0700, marker 0600 (O_EXCL). An
/// existing folder of that name is retired first when it is ours or its
/// owner is gone; a live foreign owner (or an unmarked folder) is an error.
pub fn create_in(base: &Path, name: &str, run_id: &str, own: i32) -> Result<ScratchDir, String> {
    if !valid_name(name) {
        return Err(format!("unsafe scratch folder name `{name}`"));
    }
    let base = ensure_base(base)?;
    let dir = base.join(name);
    match fs::symlink_metadata(&dir) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!(
                    "{} exists and is not a scratch folder",
                    dir.display()
                ));
            }
            match read_marker(&dir) {
                Some(marker) if marker.pid == own || !owner_alive(marker.pid, marker.pid_start) => {
                    retire(&dir);
                }
                Some(marker) => {
                    return Err(format!(
                        "scratch folder {} is in use by process {}",
                        dir.display(),
                        marker.pid
                    ))
                }
                None => return Err(format!("{} exists without a scratch marker", dir.display())),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect {}: {error}", dir.display())),
    }
    let staging = base.join(format!("{STAGING_PREFIX}{own}-{}-{name}", unique_nanos()));
    if let Err(error) = make_private_dir(&staging) {
        return Err(format!(
            "create scratch folder under {}: {error}",
            base.display()
        ));
    }
    let marker = Marker {
        pid: own,
        pid_start: crate::proc_info::own_start_micros(),
        run_id: run_id.to_string(),
        created_at: Utc::now(),
    };
    if let Err(error) = write_marker(&staging, &marker) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("write scratch marker: {error}"));
    }
    if let Err(error) = fs::rename(&staging, &dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("place scratch folder {}: {error}", dir.display()));
    }
    Ok(ScratchDir {
        path: dir,
        keep: false,
    })
}

/// A launch point's scratch: `<base()>/<folder_name(name)>`; for a chat
/// session key (a name that is not the run id) one retry under the folder of
/// a unique `<name>-<nanos>` (that
/// turn's Claude environment changes, only for that turn); otherwise
/// [`ScratchSpec::Unavailable`], logged, and recorded in the vault's health
/// journal once per process. Never fails the run: scratch is hygiene.
pub fn create_for_turn(
    vault: Option<&Path>,
    name: &str,
    run_id: &str,
) -> (Option<ScratchDir>, ScratchSpec) {
    create_for_turn_in(&base(), vault, name, run_id)
}

/// [`create_for_turn`] under an explicit base (tests).
pub fn create_for_turn_in(
    base: &Path,
    vault: Option<&Path>,
    name: &str,
    run_id: &str,
) -> (Option<ScratchDir>, ScratchSpec) {
    let own = own_pid();
    // The logical name is validated too, although only its hash reaches disk.
    let attempt = if valid_name(name) {
        create_in(base, &folder_name(name), run_id, own)
    } else {
        Err(format!("unsafe scratch folder name `{name}`"))
    };
    // A chat turn's folder is named for its conversation (PER-9): kept a
    // while after the turn ends; an automation's (named for its run) is not.
    let chat = name != run_id;
    let first = match attempt {
        Ok(mut dir) => {
            dir.keep = chat;
            let spec = dir.spec();
            return (Some(dir), spec);
        }
        Err(error) => error,
    };
    let second = if name != run_id && valid_name(name) {
        match create_in(
            base,
            &folder_name(&format!("{name}-{}", unique_nanos())),
            run_id,
            own,
        ) {
            Ok(mut dir) => {
                dir.keep = true;
                eprintln!("[scratch] {first}; using a one-off folder for this turn");
                let spec = dir.spec();
                return (Some(dir), spec);
            }
            Err(error) => Some(error),
        }
    } else {
        None
    };
    let detail = match second {
        Some(second) => format!("{first}; retry: {second}"),
        None => first,
    };
    eprintln!("[scratch] no scratch folder for run {run_id}: {detail}");
    static REPORTED: AtomicBool = AtomicBool::new(false);
    if let Some(vault) = vault {
        if !REPORTED.swap(true, Ordering::SeqCst) {
            crate::health::record(
                vault,
                "runs",
                crate::health::Outcome::Partial,
                format!("Agent runs have no scratch folder: {detail}"),
            );
        }
    }
    (None, ScratchSpec::Unavailable)
}

/// The parent's scratch folder a delegate may reuse, read through `get`
/// (the process environment in production). Accepted only when ALL hold:
/// `HOLLOW_SCRATCH_DIR` is absolute; it is a directory, not a symlink; it is
/// canonical; its parent is named `hollow-run` or `hollow-run-*`; its
/// `.hollow-scratch` marker is a regular file owned by this user with mode
/// 0600; and the marker's run id equals `HOLLOW_RUN_ID`. `TMPDIR` is never
/// consulted: inside team-mcp it is already the parent's scratch folder.
pub fn inherited(get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let raw = get(SCRATCH_ENV).filter(|value| !value.is_empty())?;
    let run_id = get(crate::usage_ledger::ENV_RUN_ID).filter(|value| !value.is_empty())?;
    let path = PathBuf::from(raw);
    if !path.is_absolute() {
        return None;
    }
    let meta = fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    if fs::canonicalize(&path).ok()? != path {
        return None;
    }
    let parent = path.parent()?.file_name()?.to_str()?;
    if parent != BASE_NAME && !parent.starts_with(&format!("{BASE_NAME}-")) {
        return None;
    }
    let marker_path = path.join(MARKER);
    let marker_meta = fs::symlink_metadata(&marker_path).ok()?;
    if !marker_meta.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if marker_meta.uid() != current_uid() || marker_meta.mode() & 0o777 != 0o600 {
            return None;
        }
    }
    let marker = read_marker(&path)?;
    (marker.run_id == run_id).then_some(path)
}

/// Remove scratch left by processes that are gone (startup, off the main
/// thread; not vault-gated). Returns how many entries were removed.
pub fn sweep_orphans(boot_at: DateTime<Utc>) -> usize {
    let base = base();
    let removed = sweep_in(&base, Utc::now(), boot_at, own_pid(), &owner_alive);
    if removed > 0 {
        eprintln!(
            "[scratch] removed {removed} orphaned scratch folder(s) under {}",
            base.display()
        );
    }
    removed
}

/// The sweep over `base`, with the clock, this process's boot time and pid,
/// and the liveness probe injected. Removes:
/// - a marked folder whose owner is gone (`is_alive` false for a foreign
///   pid; created before `boot_at` for our own pid) or older than 7 days;
/// - a staging or tombstone folder whose pid is gone (the same rules, by the
///   stamp in its name) or older than a day.
///
/// Unmarked folders, plain files and symlinks are skipped.
pub fn sweep_in(
    base: &Path,
    now: DateTime<Utc>,
    boot_at: DateTime<Utc>,
    own: i32,
    is_alive: &dyn Fn(i32, Option<u64>) -> bool,
) -> usize {
    // Kept chat folders go by age alone, whoever kept them (PER-9).
    let mut removed = prune_kept(base, now);
    let Ok(entries) = fs::read_dir(base) else {
        return removed;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        let Some(name) = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if let Some((pid, stamp)) = parse_transient(&name) {
            let gone = if pid == own {
                stamp < boot_at
            } else {
                !is_alive(pid, None)
            };
            if (gone || now - stamp > ChronoDuration::days(TRANSIENT_MAX_AGE_DAYS))
                && fs::remove_dir_all(&path).is_ok()
            {
                removed += 1;
            }
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        let Some(marker) = read_marker(&path) else {
            continue;
        };
        let gone = if marker.pid == own {
            marker.created_at < boot_at
        } else {
            !is_alive(marker.pid, marker.pid_start)
        };
        if gone || now - marker.created_at > ChronoDuration::days(DIR_MAX_AGE_DAYS) {
            let target = rename_to_tombstone(&path).unwrap_or(path);
            if fs::remove_dir_all(&target).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Whether the process that wrote a marker still runs: `false` only when
/// `kill(pid, 0)` says there is no such process, or when both start stamps
/// are known and differ (the pid was reused). Unknown answers count as alive,
/// so nothing a live run owns is ever removed on a guess.
fn owner_alive(pid: i32, pid_start: Option<u64>) -> bool {
    if crate::proc_info::pid_alive(pid) == Some(false) {
        return false;
    }
    match (pid_start, crate::proc_info::process_start_micros(pid)) {
        (Some(recorded), Some(current)) => recorded == current,
        _ => true,
    }
}

fn own_pid() -> i32 {
    std::process::id() as i32
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: getuid has no preconditions and cannot fail.
        unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Nanoseconds since the epoch, strictly increasing within this process, so
/// two tombstones or staging folders never share a name.
fn unique_nanos() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut prev = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(prev.saturating_add(1));
        match LAST.compare_exchange_weak(prev, next, Ordering::SeqCst, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(actual) => prev = actual,
        }
    }
}

/// `(pid, stamp)` of a staging or tombstone name (`.new-`/`.dead-` +
/// `<pid>-<nanos>-<name>`).
fn parse_transient(name: &str) -> Option<(i32, DateTime<Utc>)> {
    let rest = name
        .strip_prefix(TOMBSTONE_PREFIX)
        .or_else(|| name.strip_prefix(STAGING_PREFIX))?;
    stamped_parts(rest).map(|(pid, stamp, _)| (pid, stamp))
}

/// `(pid, stamp, folder)` of `<pid>-<nanos>-<folder>`.
fn stamped_parts(rest: &str) -> Option<(i32, DateTime<Utc>, &str)> {
    let mut parts = rest.splitn(3, '-');
    let pid: i32 = parts.next()?.parse().ok()?;
    let nanos: u64 = parts.next()?.parse().ok()?;
    let folder = parts.next()?;
    let secs = i64::try_from(nanos / 1_000_000_000).ok()?;
    DateTime::<Utc>::from_timestamp(secs, (nanos % 1_000_000_000) as u32)
        .map(|stamp| (pid, stamp, folder))
}

/// Set a chat turn's folder aside when the turn ends or is stopped (PER-9,
/// 11-14): a synchronous rename to `.kept-<pid>-<nanos>-<folder>`, so the
/// next turn still gets the same path, empty, and nothing the stopped turn
/// left (an uncommitted clone, a test database's files) is gone at once.
/// [`prune_kept`] removes it after [`KEPT_GRACE_HOURS`], or once the same
/// conversation has [`KEPT_PER_CHAT`] newer ones. A folder holding only its
/// marker is retired as before (nothing to keep).
fn keep(path: &Path) {
    if only_marker(path) {
        retire(path);
        return;
    }
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let kept = parent.join(format!(
        "{KEPT_PREFIX}{}-{}-{}",
        own_pid(),
        unique_nanos(),
        name.to_string_lossy()
    ));
    match fs::rename(path, &kept) {
        Ok(()) => {
            eprintln!(
                "[scratch] kept {} for {KEPT_GRACE_HOURS} hours",
                kept.display()
            );
            let base = parent.to_path_buf();
            let _ = std::thread::Builder::new()
                .name("hollow-scratch-kept".into())
                .spawn(move || prune_kept(&base, Utc::now()));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => retire(path),
    }
}

/// True when `path` holds nothing but its marker (or can't be read).
fn only_marker(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(entries) => entries
            .flatten()
            .all(|entry| entry.file_name() == std::ffi::OsStr::new(MARKER)),
        Err(_) => true,
    }
}

/// Remove kept chat folders under `base` older than [`KEPT_GRACE_HOURS`],
/// and per conversation folder all but the newest [`KEPT_PER_CHAT`].
/// Symlinks and plain files are skipped. Returns how many were removed.
fn prune_kept(base: &Path, now: DateTime<Utc>) -> usize {
    let Ok(entries) = fs::read_dir(base) else {
        return 0;
    };
    let mut kept: Vec<(String, DateTime<Utc>, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((_, stamp, folder)) = file.strip_prefix(KEPT_PREFIX).and_then(stamped_parts)
        else {
            continue;
        };
        kept.push((folder.to_string(), stamp, path.clone()));
    }
    // Newest first within each folder name.
    kept.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut removed = 0;
    let mut seen: Option<(String, usize)> = None;
    for (folder, stamp, path) in kept {
        let rank = match &mut seen {
            Some((name, count)) if *name == folder => {
                *count += 1;
                *count
            }
            _ => {
                seen = Some((folder, 1));
                1
            }
        };
        let stale = now - stamp > ChronoDuration::hours(KEPT_GRACE_HOURS);
        if (stale || rank > KEPT_PER_CHAT) && fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Make sure `base` exists as a directory owned by this user, without group
/// or other access, and return it canonicalized. Existing permissions are
/// only ever tightened (never widened), so an unwritable base stays so.
fn ensure_base(base: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(base) {
        Ok(meta) => {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!(
                    "scratch base {} is not a directory",
                    base.display()
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt, PermissionsExt};
                if meta.uid() != current_uid() {
                    return Err(format!(
                        "scratch base {} belongs to another user",
                        base.display()
                    ));
                }
                let mode = meta.mode() & 0o777;
                if mode & 0o077 != 0 {
                    let _ = fs::set_permissions(base, fs::Permissions::from_mode(mode & 0o700));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(base)
                .map_err(|e| format!("create scratch base {}: {e}", base.display()))?;
        }
        Err(error) => return Err(format!("inspect scratch base {}: {error}", base.display())),
    }
    fs::canonicalize(base).map_err(|e| format!("resolve scratch base {}: {e}", base.display()))
}

fn make_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_marker(dir: &Path, marker: &Marker) -> std::io::Result<()> {
    let body = serde_json::to_vec(marker).map_err(std::io::Error::other)?;
    let path = dir.join(MARKER);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(&body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn read_marker(dir: &Path) -> Option<Marker> {
    let path = dir.join(MARKER);
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MARKER_MAX_BYTES {
        return None;
    }
    serde_json::from_slice(&fs::read(&path).ok()?).ok()
}

/// Rename `path` to a fresh tombstone beside it; `None` when it is already
/// gone or the rename failed (the caller then removes inline).
fn rename_to_tombstone(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let tomb = parent.join(format!(
        "{TOMBSTONE_PREFIX}{}-{}-{name}",
        own_pid(),
        unique_nanos()
    ));
    fs::rename(path, &tomb).ok().map(|_| tomb)
}

/// Remove a scratch folder without ever walking a path a new run may
/// recreate: a synchronous rename to a tombstone, then a detached delete of
/// the tombstone. A missing folder is fine; a failed rename (other than
/// NotFound) falls back to an inline delete.
fn retire(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    let name = match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => return,
    };
    let tomb = parent.join(format!(
        "{TOMBSTONE_PREFIX}{}-{}-{name}",
        own_pid(),
        unique_nanos()
    ));
    match fs::rename(path, &tomb) {
        Ok(()) => {
            let doomed = tomb.clone();
            let spawned = std::thread::Builder::new()
                .name("hollow-scratch-rm".into())
                .spawn(move || {
                    let _ = fs::remove_dir_all(&doomed);
                });
            if spawned.is_err() {
                let _ = fs::remove_dir_all(&tomb);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            eprintln!(
                "[scratch] could not retire {}: {error}; removing in place",
                path.display()
            );
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A pid no process can have (above macOS's and Linux's default limits).
    const DEAD_PID: i32 = 99_999_999;

    fn temp_base() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix("scratch-")
            .tempdir()
            .unwrap();
        let base = fs::canonicalize(dir.path()).unwrap().join(BASE_NAME);
        (dir, base)
    }

    fn tombstones(base: &Path) -> Vec<String> {
        fs::read_dir(base)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| e.file_name().to_str().map(str::to_string))
                    .filter(|n| n.starts_with(TOMBSTONE_PREFIX))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "timed out waiting for {what}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn plant(base: &Path, name: &str, marker: &Marker) -> PathBuf {
        fs::create_dir_all(base).unwrap();
        let dir = base.join(name);
        fs::create_dir(&dir).unwrap();
        write_marker(&dir, marker).unwrap();
        dir
    }

    fn marker(pid: i32, run_id: &str, created_at: DateTime<Utc>) -> Marker {
        Marker {
            pid,
            pid_start: None,
            run_id: run_id.into(),
            created_at,
        }
    }

    #[cfg(unix)]
    #[test]
    fn create_makes_a_private_dir_with_an_owner_marker() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, base) = temp_base();
        let dir = create_in(&base, "run-1-2-3", "run-1-2-3", own_pid()).unwrap();
        let path = dir.path().to_path_buf();
        assert_eq!(
            fs::metadata(&base).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let marker_mode = fs::metadata(path.join(MARKER))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(marker_mode, 0o600);
        let written = read_marker(&path).expect("marker");
        assert_eq!(written.pid, own_pid());
        assert_eq!(written.run_id, "run-1-2-3");
        assert!(!path.to_string_lossy().chars().any(char::is_whitespace));
        assert_eq!(
            fs::canonicalize(&path).unwrap(),
            path,
            "the path is canonical"
        );
        assert!(
            fs::read_dir(&base)
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().starts_with(STAGING_PREFIX)),
            "no staging folder is left behind"
        );
        assert_eq!(dir.spec(), ScratchSpec::Dir(path.clone()));
        dir.remove_now();
        assert!(!path.exists());
    }

    #[test]
    fn create_rejects_unsafe_names_and_live_foreign_dirs() {
        let (_tmp, base) = temp_base();
        let long = "x".repeat(MAX_NAME + 1);
        for bad in ["", "../x", "a/b", "a b", ".hidden", "é", long.as_str()] {
            assert!(create_in(&base, bad, "r", own_pid()).is_err(), "{bad:?}");
        }
        // pid 1 always exists (kill(1, 0) is EPERM for a user, which counts
        // as alive): a live foreign owner is an error.
        plant(&base, "busy", &marker(1, "r-other", Utc::now()));
        let error = create_in(&base, "busy", "r", own_pid()).unwrap_err();
        assert!(error.contains("in use"), "{error}");
        assert!(
            base.join("busy").join(MARKER).exists(),
            "the live folder is untouched"
        );
        // An unmarked folder is never adopted.
        fs::create_dir(base.join("bare")).unwrap();
        assert!(create_in(&base, "bare", "r", own_pid()).is_err());
        // A dead owner's and our own leftover folder are replaced.
        for (name, pid) in [("dead", DEAD_PID), ("mine", own_pid())] {
            let old = plant(&base, name, &marker(pid, "r-old", Utc::now()));
            fs::write(old.join("stale.txt"), "x").unwrap();
            let fresh = create_in(&base, name, "r-new", own_pid()).unwrap();
            assert_eq!(read_marker(fresh.path()).unwrap().run_id, "r-new");
            assert!(!fresh.path().join("stale.txt").exists(), "{name}");
            fresh.remove_now();
        }
        wait_until("tombstones to go", || tombstones(&base).is_empty());
    }

    #[test]
    fn a_reused_name_survives_the_previous_removal() {
        let (_tmp, base) = temp_base();
        let first = create_in(&base, "s-20260926-abcdef01", "run-a", own_pid()).unwrap();
        let path = first.path().to_path_buf();
        fs::write(path.join("a.txt"), "a").unwrap();
        drop(first);
        let second = create_in(&base, "s-20260926-abcdef01", "run-b", own_pid()).unwrap();
        assert_eq!(second.path(), path);
        fs::write(path.join("b.txt"), "b").unwrap();
        wait_until("the tombstone to go", || tombstones(&base).is_empty());
        assert!(path.join("b.txt").exists(), "the new turn's file survives");
        assert!(!path.join("a.txt").exists());
        assert_eq!(read_marker(&path).unwrap().run_id, "run-b");
        second.remove_now();
    }

    #[test]
    fn dropping_a_scratch_dir_removes_it() {
        let (_tmp, base) = temp_base();
        let dir = create_in(&base, "run-drop", "run-drop", own_pid()).unwrap();
        let path = dir.path().to_path_buf();
        fs::create_dir_all(path.join("nested/deeper")).unwrap();
        fs::write(path.join("nested/deeper/file.bin"), vec![0u8; 4096]).unwrap();
        drop(dir);
        assert!(!path.exists(), "renamed away at once");
        wait_until("the tombstone to go", || tombstones(&base).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn removal_never_follows_symlinks() {
        let (tmp, base) = temp_base();
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let dir = create_in(&base, "run-link", "run-link", own_pid()).unwrap();
        std::os::unix::fs::symlink(&outside, dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("keep.txt"), dir.path().join("file-link")).unwrap();
        drop(dir);
        wait_until("the tombstone to go", || tombstones(&base).is_empty());
        assert!(
            outside.join("keep.txt").exists(),
            "the symlink target survives"
        );
    }

    #[test]
    fn choose_base_falls_back_when_tmpdir_has_spaces() {
        #[cfg(target_os = "macos")]
        let fallback = "/private/tmp/hollow-run-501";
        #[cfg(not(target_os = "macos"))]
        let fallback = "/tmp/hollow-run-501";
        assert_eq!(
            choose_base(Path::new("/tmp/a b"), 501),
            PathBuf::from(fallback)
        );
        // A short temp dir is kept.
        let short = fs::canonicalize("/tmp").unwrap();
        assert_eq!(choose_base(Path::new("/tmp"), 501), short.join(BASE_NAME));
        // A long one (macOS's per-user /private/var/folders/…/T/ is ~57
        // bytes) falls back, so the child's TMPDIR stays short.
        let (tmp, _) = temp_base();
        let long = tmp
            .path()
            .join("a-temp-folder-name-long-enough-to-overflow");
        fs::create_dir(&long).unwrap();
        assert!(
            fs::canonicalize(&long)
                .unwrap()
                .join(BASE_NAME)
                .as_os_str()
                .len()
                > MAX_BASE_BYTES
        );
        assert_eq!(choose_base(&long, 501), PathBuf::from(fallback));
    }

    /// Break-it review (15a): the scratch folder becomes the child's TMPDIR,
    /// and tools put Unix sockets under it (tsx's `tsx-<uid>/<pid>.pipe`,
    /// Python's `pymp-*/listener-*`, Chromium's
    /// `org.chromium.Chromium.XXXXXX/SingletonSocket`). A run-id-named folder
    /// under macOS's canonical temp dir left 3 bytes for them.
    #[test]
    fn tmpdir_leaves_room_for_a_unix_socket_name() {
        // The longest suffix a common tool appends (Chromium's, 46 bytes).
        let suffix = "org.chromium.Chromium.a1b2c3/SingletonSocket";
        let names = [
            "run-1727350000123456789-12345-17",
            "s-20260926-abcdef01",
            &"x".repeat(MAX_NAME),
        ];
        let real_tmp = std::env::temp_dir();
        for tmp in [real_tmp.as_path(), Path::new("/tmp")] {
            for uid in [current_uid(), 501, u32::MAX] {
                let base = choose_base(tmp, uid);
                assert!(
                    base.as_os_str().len() <= MAX_BASE_BYTES,
                    "{}",
                    base.display()
                );
                for name in names {
                    let folder = folder_name(name);
                    assert_eq!(folder.len(), FOLDER_HEX);
                    assert!(valid_name(&folder));
                    let tmpdir = format!("{}/", base.join(&folder).display());
                    let socket = format!("{tmpdir}{suffix}");
                    assert!(
                        socket.len() <= SOCKET_PATH_MAX,
                        "{} bytes: {socket}",
                        socket.len()
                    );
                }
            }
        }
        assert_eq!(folder_name("s-1"), folder_name("s-1"), "stable per session");
        assert_ne!(folder_name("s-1"), folder_name("s-2"));
    }

    #[test]
    fn sweep_removes_only_dead_or_stale_dirs_and_tombstones() {
        let (_tmp, base) = temp_base();
        let now = Utc::now();
        let boot = now - ChronoDuration::hours(1);
        let own = own_pid();
        const LIVE: i32 = 4242;
        let alive = |pid: i32, _: Option<u64>| pid == LIVE;
        let nanos = |at: DateTime<Utc>| at.timestamp_nanos_opt().unwrap();

        // Removed.
        plant(&base, "dead-owner", &marker(DEAD_PID, "r1", now));
        plant(
            &base,
            "own-before-boot",
            &marker(own, "r2", boot - ChronoDuration::minutes(5)),
        );
        plant(
            &base,
            "week-old",
            &marker(LIVE, "r3", now - ChronoDuration::days(8)),
        );
        fs::create_dir(base.join(format!(".dead-{DEAD_PID}-{}-x", nanos(now)))).unwrap();
        fs::create_dir(base.join(format!(".new-{DEAD_PID}-{}-y", nanos(now)))).unwrap();
        fs::create_dir(base.join(format!(
            ".dead-{LIVE}-{}-old",
            nanos(now - ChronoDuration::days(2))
        )))
        .unwrap();
        // Kept.
        plant(&base, "live-owner", &marker(LIVE, "r4", now));
        plant(&base, "own-this-boot", &marker(own, "r5", now));
        fs::create_dir(base.join("unmarked")).unwrap();
        fs::write(base.join("stray-file"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("live-owner"), base.join("link")).unwrap();
        let live_tomb = format!(".dead-{LIVE}-{}-fresh", nanos(now));
        fs::create_dir(base.join(&live_tomb)).unwrap();

        assert_eq!(sweep_in(&base, now, boot, own, &alive), 6);
        let mut left: Vec<String> = fs::read_dir(&base)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut expected = vec![
            "live-owner".to_string(),
            "own-this-boot".into(),
            "unmarked".into(),
            "stray-file".into(),
            live_tomb,
        ];
        #[cfg(unix)]
        expected.push("link".into());
        expected.sort();
        assert_eq!(left, expected);
        assert!(
            base.join("live-owner").join(MARKER).exists(),
            "a symlinked folder is never followed"
        );
        assert_eq!(sweep_in(&base.join("missing"), now, boot, own, &alive), 0);
    }

    #[test]
    fn inherited_validates_structure_not_tmpdir() {
        let (tmp, base) = temp_base();
        let dir = create_in(&base, "run-parent", "R-parent", own_pid()).unwrap();
        let path = dir.path().to_path_buf();
        let path_text = path.to_string_lossy().into_owned();
        // The real team-mcp situation: TMPDIR is the scratch folder itself.
        let env = |scratch: &str, run: &str| {
            let map: std::collections::HashMap<String, String> = [
                ("TMPDIR".to_string(), format!("{path_text}/")),
                (SCRATCH_ENV.to_string(), scratch.to_string()),
                ("HOLLOW_RUN_ID".to_string(), run.to_string()),
            ]
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .collect();
            move |key: &str| map.get(key).cloned()
        };
        assert_eq!(inherited(env(&path_text, "R-parent")), Some(path.clone()));
        assert_eq!(
            inherited(env(&path_text, "R-other")),
            None,
            "run id mismatch"
        );
        assert_eq!(inherited(env("", "R-parent")), None, "unset");
        assert_eq!(
            inherited(env("relative/run-parent", "R-parent")),
            None,
            "relative"
        );
        let dotted = format!("{}/../{BASE_NAME}/run-parent", base.to_string_lossy());
        assert_eq!(inherited(env(&dotted, "R-parent")), None, "not canonical");

        // A folder whose parent is not hollow-run*, even with a good marker.
        let elsewhere = fs::canonicalize(tmp.path()).unwrap().join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        let foreign = plant(
            &elsewhere,
            "run-parent",
            &marker(own_pid(), "R-parent", Utc::now()),
        );
        assert_eq!(
            inherited(env(&foreign.to_string_lossy(), "R-parent")),
            None,
            "wrong parent"
        );

        // No marker.
        let bare = base.join("run-bare");
        fs::create_dir(&bare).unwrap();
        assert_eq!(
            inherited(env(&bare.to_string_lossy(), "R-parent")),
            None,
            "no marker"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // A symlink to a valid folder.
            let link = base.join("run-link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert_eq!(
                inherited(env(&link.to_string_lossy(), "R-parent")),
                None,
                "symlink"
            );
            // A marker readable by others.
            fs::set_permissions(path.join(MARKER), fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(inherited(env(&path_text, "R-parent")), None, "marker mode");
            fs::set_permissions(path.join(MARKER), fs::Permissions::from_mode(0o600)).unwrap();
        }
        dir.remove_now();
    }

    #[cfg(unix)]
    #[test]
    fn create_for_turn_never_leaves_the_variable_unset() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, base) = temp_base();
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o500)).unwrap();
        let (dir, spec) = create_for_turn_in(&base, None, "s-20260926-abcdef01", "run-x");
        assert!(dir.is_none());
        assert_eq!(spec, ScratchSpec::Unavailable);
        assert_eq!(spec.dir(), None);
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();

        // A writable base gives a folder; a live foreign same-name folder
        // sends a chat key to a one-off name, never to Unavailable.
        let (dir, spec) = create_for_turn_in(&base, None, "s-free", "run-w");
        let dir = dir.expect("a folder");
        assert_eq!(
            dir.path(),
            fs::canonicalize(&base).unwrap().join(folder_name("s-free"))
        );
        assert_eq!(read_marker(dir.path()).unwrap().run_id, "run-w");
        assert_eq!(spec, dir.spec());
        dir.remove_now();
        plant(
            &base,
            &folder_name("s-key"),
            &marker(1, "r-other", Utc::now()),
        );
        let (dir, spec) = create_for_turn_in(&base, None, "s-key", "run-y");
        let dir = dir.expect("a one-off folder");
        assert_ne!(dir.path(), base.join(folder_name("s-key")));
        assert_eq!(dir.path().file_name().unwrap().len(), FOLDER_HEX);
        assert_eq!(spec, dir.spec());
        dir.remove_now();
        // A run id gets no retry.
        plant(
            &base,
            &folder_name("run-z"),
            &marker(1, "r-other", Utc::now()),
        );
        assert_eq!(
            create_for_turn_in(&base, None, "run-z", "run-z").1,
            ScratchSpec::Unavailable
        );
        // An unsafe logical name never gets a folder, even hashed.
        assert_eq!(
            create_for_turn_in(&base, None, "../x", "run-q").1,
            ScratchSpec::Unavailable
        );
    }

    #[test]
    fn scratch_guidance_names_only_the_variable() {
        assert!(SCRATCH_GUIDANCE.contains("$HOLLOW_SCRATCH_DIR"));
        assert!(SCRATCH_GUIDANCE.contains("$TMPDIR"));
        for banned in ["/", "hollow-run", "var/folders", "\n"] {
            assert!(!SCRATCH_GUIDANCE.contains(banned), "{banned:?}");
        }
        assert!(UNAVAILABLE_PATH.starts_with("/dev/null/"));
    }

    #[test]
    fn transient_names_parse_and_are_unique() {
        let a = unique_nanos();
        let b = unique_nanos();
        assert!(b > a);
        let (pid, stamp) = parse_transient(&format!(".dead-42-{b}-s-2026-x")).unwrap();
        assert_eq!(pid, 42);
        assert_eq!(stamp.timestamp_nanos_opt().unwrap() as u64, b);
        assert!(parse_transient(&format!(".new-7-{a}-run-1")).is_some());
        assert!(parse_transient(".dead-x-1-y").is_none());
        assert!(parse_transient("run-1").is_none());
    }

    /// The startup sweep is wired into core startup (`core_boot`, which the
    /// GUI's setup and `hollow serve` both call), off the main thread and
    /// not behind the vault check.
    #[test]
    fn the_startup_sweep_is_wired() {
        let lib = include_str!("core_boot.rs");
        let sweep = lib
            .find(concat!("scratch::", "sweep_orphans(boot_at)"))
            .expect("startup sweeps");
        let setup = lib
            .find(concat!("pub(crate) fn ", "account_services("))
            .expect("the phase");
        let vault_gate = lib
            .find(concat!("if vault_path", ".exists() {"))
            .expect("the vault gate");
        assert!(
            setup < sweep && sweep < vault_gate,
            "in setup, before (not inside) the vault-gated block"
        );
        assert!(
            lib[..sweep]
                .rfind(concat!("std::thread::", "spawn("))
                .unwrap()
                > setup,
            "on its own thread"
        );
    }

    /// PER-9 (11-14): a chat turn's folder is set aside when the turn ends
    /// or is stopped, not deleted, and the next turn gets the same path,
    /// empty. An empty folder and an automation's folder go as before; kept
    /// ones go past the per-chat cap and after the grace period.
    #[test]
    fn a_chat_turns_scratch_is_kept_for_a_grace_period() {
        let (_tmp, base) = temp_base();
        let kept = |base: &Path| -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(base)
                .unwrap()
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| n.starts_with(KEPT_PREFIX))
                .collect();
            names.sort();
            names
        };
        let key = "s-20261114-hol9";
        let turn = |run: &str, work: Option<&str>| {
            let (dir, _) = create_for_turn_in(&base, None, key, run);
            let dir = dir.expect("a folder");
            let path = dir.path().to_path_buf();
            if let Some(work) = work {
                fs::write(path.join("work.txt"), work).unwrap();
            }
            drop(dir);
            assert!(!path.exists(), "the path is free for the next turn");
        };
        turn("run-1", Some("uncommitted"));
        let first = kept(&base);
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(
            fs::read_to_string(base.join(&first[0]).join("work.txt")).unwrap(),
            "uncommitted"
        );
        turn("run-2", None);
        assert_eq!(kept(&base), first, "an empty folder is not kept");
        let (auto, _) = create_for_turn_in(&base, None, "run-auto", "run-auto");
        let auto = auto.expect("a folder");
        fs::write(auto.path().join("x.txt"), "x").unwrap();
        drop(auto);
        assert_eq!(kept(&base), first, "an automation's folder is not kept");
        for i in 0..4 {
            turn(&format!("run-more-{i}"), Some("more"));
        }
        prune_kept(&base, Utc::now());
        wait_until("the cap", || kept(&base).len() == KEPT_PER_CHAT);
        assert!(
            !kept(&base).contains(&first[0]),
            "the oldest goes past the cap"
        );
        let later = Utc::now() + ChronoDuration::hours(KEPT_GRACE_HOURS + 1);
        prune_kept(&base, later);
        wait_until("the grace period", || kept(&base).is_empty());
    }
}
