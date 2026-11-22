//! The code-context index (CC2, design: the hollow workspace's
//! `outputs/code-context-2026-11/design.md`): one SQLite file per checkout
//! holding every definition and every name-scan edge of its Rust,
//! TypeScript (TSX) and Python files, with resolution annotations.
//!
//! The index is a cache outside any tracked tree (the OS cache folder, keyed
//! by a hash of the canonical root). A build writes a new file and swaps it
//! in by atomic rename, so an open reader keeps its old view. [`ensure`]
//! rebuilds whenever the listed files' paths, sizes or modified times
//! changed (a full rebuild); [`build_into`] is the reusable "index this
//! root into that file" step. CC3 ([`overlay`], [`open`]): a checkout with a
//! commit reads a base index shared by every worktree of its repository
//! plus its own small overlay instead, and falls back to the full index.
//!
//! Only files git lists (tracked, plus untracked and not ignored) are read;
//! never a hidden or secret-looking path (`directory_connections::
//! secret_name`), never a symlink or a path that resolves outside the root
//! (a symlinked parent folder: skipped and reported), never a file over
//! [`MAX_FILE_BYTES`] (skipped and reported). Nothing in the repo is
//! executed: the only subprocess is `git`, hardened as `repo_diff`'s is
//! ([`Git`]: `GIT_*` stripped, `--no-optional-locks`, fsmonitor, hooks,
//! filters and submodules off, git dir and work tree pinned, stdin closed).
//!
//! **Bounds** ([`Limits`]): a build refuses a checkout over a file count or
//! a total byte cap before reading anything, and stops at an overall
//! deadline or when the MCP request that asked for it is cancelled
//! (`shared_mcp::request_current`, checked between files). One build runs
//! per index file: an in-process mutex coalesces callers (a second waits
//! and reuses the first's index), a lock file keeps other processes out
//! (broken when older than [`STALE_LOCK`]), and each build writes its own
//! temp file.

pub(crate) mod overlay;
pub(crate) mod parse;
pub(crate) mod resolve;
pub(crate) mod store;

pub(crate) use overlay::open;

use once_cell::sync::Lazy;
use parse::Lang;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// A file over this many bytes is skipped and reported.
pub(crate) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// A build's bounds; [`LIMITS`] in use, smaller ones in tests.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    /// Most code files one index holds.
    pub max_files: usize,
    /// Most bytes of code files one index reads in total.
    pub max_bytes: u64,
    /// Longest one build (listing, waiting for another build, parsing and
    /// writing) may take.
    pub deadline: Duration,
}

pub(crate) const LIMITS: Limits = Limits {
    max_files: 40_000,
    max_bytes: 400 * 1024 * 1024,
    deadline: Duration::from_secs(180),
};

/// A build lock file older than this is a dead build's and is broken.
pub(crate) const STALE_LOCK: Duration = Duration::from_secs(600);
/// A temp `.building` file older than this is a dead build's and is pruned.
const STALE_BUILDING: Duration = Duration::from_secs(3600);
/// Longest one git call may take.
const GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// A refusal over a [`Limits`] bound starts with this (a leading
/// `mcp_envelope` code), so it reads as REFUSED, not unavailable.
pub(crate) const LIMIT_REFUSAL: &str = "REFUSED:";

/// One file the index may read.
#[derive(Debug, Clone)]
pub(crate) struct Listed {
    pub path: String,
    pub lang: Lang,
    pub bytes: u64,
    pub mtime_ns: u128,
}

/// What [`list_files`] found.
#[derive(Debug, Clone, Default)]
pub(crate) struct Listing {
    /// The files the index reads, sorted.
    pub files: Vec<Listed>,
    /// Paths skipped as over the byte cap.
    pub skipped: Vec<String>,
    /// Paths skipped because they resolve outside the root (a symlinked
    /// parent folder).
    pub outside: Vec<String>,
}

/// The files of `root` the index reads, and the paths it skipped.
#[cfg(test)]
pub(crate) fn list_files(root: &Path) -> Result<Listing, String> {
    let git = Git::open(root)?;
    list_with(&git)
}

fn list_with(git: &Git) -> Result<Listing, String> {
    let root = git.root.as_path();
    let out = git.run(&[
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ])?;
    let mut paths: Vec<&str> = out.split('\0').filter(|p| !p.is_empty()).collect();
    paths.sort_unstable();
    paths.dedup();
    let mut listing = Listing::default();
    for path in paths {
        let Some(lang) = Lang::of(path) else { continue };
        if !readable_path(path) {
            continue;
        }
        let full = root.join(path);
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue; // a symlink, a deleted file, a submodule
        }
        // A symlinked parent folder resolves somewhere else.
        if !std::fs::canonicalize(&full).is_ok_and(|c| c.starts_with(root)) {
            listing.outside.push(path.to_string());
            continue;
        }
        if meta.len() > MAX_FILE_BYTES {
            listing.skipped.push(path.to_string());
            continue;
        }
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        listing.files.push(Listed {
            path: path.to_string(),
            lang,
            bytes: meta.len(),
            mtime_ns,
        });
    }
    Ok(listing)
}

/// A relative path with no hidden or secret-looking part and no `..`.
pub(crate) fn readable_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && path.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !crate::directory_connections::secret_name(&part.to_lowercase())
        })
}

/// One checkout's git, hardened like `repo_diff`'s capture (the same
/// `repo_diff::hardened_git`): `GIT_*` stripped, `--no-optional-locks`,
/// fsmonitor, hooks, external diff and submodule recursion off through
/// `GIT_CONFIG_COUNT`, every filter driver the repository defines disabled,
/// the git dir and work tree pinned, stdin closed, each call under
/// [`GIT_TIMEOUT`]. The repository's own config never runs a program.
pub(crate) struct Git {
    /// The canonical top level.
    pub root: PathBuf,
    git_dir: PathBuf,
    filters: Vec<String>,
}

impl Git {
    /// Discover the checkout `dir` sits in (unpinned, nothing but
    /// `rev-parse`), then pin it.
    pub(crate) fn open(dir: &Path) -> Result<Git, String> {
        let out = run_git(
            crate::repo_diff::hardened_git(dir, None, &[]),
            &["rev-parse", "--show-toplevel", "--absolute-git-dir"],
            false,
        )?;
        let mut lines = out.lines();
        let (Some(top), Some(git_dir)) = (lines.next(), lines.next()) else {
            return Err("not a git checkout".into());
        };
        let canon =
            |p: &str| std::fs::canonicalize(p.trim()).map_err(|e| format!("git named `{p}`: {e}"));
        let mut git = Git {
            root: canon(top)?,
            git_dir: canon(git_dir)?,
            filters: Vec::new(),
        };
        // `git config --get-regexp` exits 1 when nothing matches.
        let names = git.run_with(
            &["config", "-z", "--name-only", "--get-regexp", r"^filter\."],
            true,
        )?;
        git.filters = crate::repo_diff::parse_filter_names(names.as_bytes());
        Ok(git)
    }

    /// Run a pinned, hardened `git <args>` in the root.
    pub(crate) fn run(&self, args: &[&str]) -> Result<String, String> {
        self.run_with(args, false)
    }

    fn run_with(&self, args: &[&str], exit_one_ok: bool) -> Result<String, String> {
        let cmd = crate::repo_diff::hardened_git(&self.root, Some(&self.git_dir), &self.filters);
        run_git(cmd, args, exit_one_ok)
    }

    /// [`Git::run`] with `input` on stdin (closed after it), its output as
    /// bytes (CC3: `cat-file --batch`).
    pub(crate) fn run_bytes(&self, args: &[&str], input: Vec<u8>) -> Result<Vec<u8>, String> {
        let cmd = crate::repo_diff::hardened_git(&self.root, Some(&self.git_dir), &self.filters);
        run_git_raw(cmd, args, false, Some(input))
    }
}

/// Run `cmd <args>` with stdin closed, its output read on threads, killed
/// after [`GIT_TIMEOUT`].
fn run_git(cmd: std::process::Command, args: &[&str], exit_one_ok: bool) -> Result<String, String> {
    run_git_raw(cmd, args, exit_one_ok, None).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// [`run_git`] with optional stdin `input` (written on a thread, then
/// closed), returning the raw output.
fn run_git_raw(
    mut cmd: std::process::Command,
    args: &[&str],
    exit_one_ok: bool,
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let what = args.first().copied().unwrap_or("");
    let mut child = cmd
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run git: {e}"))?;
    let feed = match (input, child.stdin.take()) {
        (Some(bytes), Some(mut pipe)) => Some(std::thread::spawn(move || {
            use std::io::Write;
            let _ = pipe.write_all(&bytes);
        })),
        _ => None,
    };
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let until = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "git {what} took longer than {}s",
                    GIT_TIMEOUT.as_secs()
                ));
            }
            Err(e) => return Err(format!("git {what} failed: {e}")),
        }
    };
    if let Some(feed) = feed {
        let _ = feed.join();
    }
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    let ok = status.success() || (exit_one_ok && status.code() == Some(1));
    if !ok {
        let err = String::from_utf8_lossy(&stderr);
        return Err(format!("git {what} failed: {}", err.trim()));
    }
    Ok(stdout)
}

/// The git work tree `dir` sits in (its top level), canonical.
pub(crate) fn checkout_root(dir: &Path) -> Option<PathBuf> {
    Git::open(dir).ok().map(|g| g.root)
}

/// The first 12 hex digits of SHA-256 of `bytes`.
pub(crate) fn short_sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))[..12].to_string()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The stamp [`ensure`] compares: paths, sizes and modified times.
fn stamp(listing: &Listing) -> String {
    let mut h = Sha256::new();
    for f in &listing.files {
        h.update(format!("{}\0{}\0{}\n", f.path, f.bytes, f.mtime_ns));
    }
    for s in &listing.skipped {
        h.update(format!("skip\0{s}\n"));
    }
    for s in &listing.outside {
        h.update(format!("outside\0{s}\n"));
    }
    h.update(parse::PARSERS);
    h.update(store::SCHEMA_VERSION.to_string());
    hex(&h.finalize())
}

/// The index folder: `cache`, else the OS cache folder's
/// `hollow/code-index`.
pub(crate) fn cache_dir(cache: Option<&Path>) -> Option<PathBuf> {
    match cache {
        Some(dir) => Some(dir.to_path_buf()),
        None => Some(dirs::cache_dir()?.join("hollow").join("code-index")),
    }
}

/// The index file of `root` under `cache` (the OS cache folder's
/// `hollow/code-index` when `None`).
pub(crate) fn index_path(root: &Path, cache: Option<&Path>) -> Option<PathBuf> {
    let key = short_sha(root.to_string_lossy().as_bytes());
    Some(cache_dir(cache)?.join(format!("{key}.sqlite")))
}

/// Index `root`'s work tree into the new file `out`. Returns the counts
/// the build recorded. (CC3's base index reads a commit's blobs instead:
/// `overlay::build_base`.)
#[cfg(test)]
pub(crate) fn build_into(root: &Path, out: &Path) -> Result<store::Built, String> {
    let git = Git::open(root)?;
    let listing = list_with(&git)?;
    within_limits(&listing, &LIMITS)?;
    build_listed(
        &git,
        out,
        &listing,
        Instant::now() + LIMITS.deadline,
        &LIMITS,
    )
}

/// Refuse a listing over a file-count or total-byte bound, naming it.
fn within_limits(listing: &Listing, limits: &Limits) -> Result<(), String> {
    let n = listing.files.len();
    if n > limits.max_files {
        return Err(format!(
            "{LIMIT_REFUSAL} this checkout has {n} code files, over the code index limit of {} files; nothing was indexed.",
            limits.max_files
        ));
    }
    let bytes: u64 = listing.files.iter().map(|f| f.bytes).sum();
    if bytes > limits.max_bytes {
        return Err(format!(
            "{LIMIT_REFUSAL} this checkout has {bytes} bytes of code files, over the code index limit of {} bytes; nothing was indexed.",
            limits.max_bytes
        ));
    }
    Ok(())
}

fn build_listed(
    git: &Git,
    out: &Path,
    listing: &Listing,
    deadline: Instant,
    limits: &Limits,
) -> Result<store::Built, String> {
    let root = git.root.as_path();
    let parsed = parse_with(&listing.files, deadline, &|f| read_inside(root, &f.path))
        .map_err(|why| abort_text(why, limits))?;
    let head = git
        .run(&["rev-parse", "--short=9", "HEAD"])
        .map(|h| h.trim().to_string())
        .unwrap_or_default();
    let resolved = resolve::resolve(&parsed);
    store::write(
        out,
        &parsed,
        &resolved,
        &store::Meta {
            head,
            stamp: stamp(listing),
            indexed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            skipped: listing.skipped.clone(),
            outside: listing.outside.clone(),
            ..Default::default()
        },
    )
}

/// Why a parse stopped, as the refusal or error a caller sees.
fn abort_text(why: Abort, limits: &Limits) -> String {
    match why {
        Abort::Deadline => format!(
            "{LIMIT_REFUSAL} the code index build ran past its {}s code index limit and stopped; nothing was indexed.",
            limits.deadline.as_secs()
        ),
        Abort::Cancelled => {
            "the request was cancelled, so the code index build stopped; nothing was indexed."
                .to_string()
        }
    }
}

/// One file read and parsed.
#[derive(Debug, Clone)]
pub(crate) struct FileParse {
    pub path: String,
    pub lang: Lang,
    pub sha: String,
    pub bytes: u64,
    pub test: bool,
    pub parsed: parse::Parsed,
}

/// Why a build stopped early.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Abort {
    Deadline,
    Cancelled,
}

/// The bytes of `path` under the canonical `root`: only a regular file that
/// still resolves inside the root, opened without following a final
/// symlink (`O_NOFOLLOW|O_NONBLOCK`), at most [`MAX_FILE_BYTES`].
fn read_inside(root: &Path, path: &str) -> Option<Vec<u8>> {
    let canon = std::fs::canonicalize(root.join(path)).ok()?;
    if !canon.starts_with(root) {
        return None;
    }
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = open.open(&canon).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut src = Vec::new();
    file.take(MAX_FILE_BYTES).read_to_end(&mut src).ok()?;
    Some(src)
}

/// CC3 review: the bytes of `path`, a path an index file names, under the
/// canonical checkout `root`: only a readable code path ([`readable_path`]:
/// relative, no `..` or hidden part, nothing secret-looking; [`Lang::of`]),
/// then [`read_inside`]. A cache file (an imported base) can never steer a
/// read outside the checkout, at a FIFO or a device, or past the byte cap.
pub(crate) fn read_index_path(root: &Path, path: &str) -> Option<Vec<u8>> {
    if !readable_path(path) || Lang::of(path).is_none() {
        return None;
    }
    read_inside(root, path)
}

/// Parse `files` on up to four threads, in their order, each file's bytes
/// from `read` (the work tree, or CC3's commit blobs; `None` skips it).
/// Each thread checks the deadline and the asking request (carried into it)
/// between files; the first to see either stops them all.
fn parse_with(
    files: &[Listed],
    deadline: Instant,
    read: &(dyn Fn(&Listed) -> Option<Vec<u8>> + Sync),
) -> Result<Vec<FileParse>, Abort> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 4);
    let chunk = files.len().div_ceil(threads).max(1);
    let stop = AtomicBool::new(false);
    let why = Mutex::new(None::<Abort>);
    let parsed = std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                let (stop, why) = (&stop, &why);
                scope.spawn(crate::shared_mcp::carry(move || {
                    let mut out = Vec::with_capacity(part.len());
                    for f in part {
                        let abort = if stop.load(Ordering::Relaxed) {
                            break;
                        } else if Instant::now() >= deadline {
                            Some(Abort::Deadline)
                        } else if !crate::shared_mcp::request_current() {
                            Some(Abort::Cancelled)
                        } else {
                            None
                        };
                        if let Some(a) = abort {
                            stop.store(true, Ordering::Relaxed);
                            why.lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get_or_insert(a);
                            break;
                        }
                        let Some(src) = read(f) else {
                            continue;
                        };
                        out.push(FileParse {
                            path: f.path.clone(),
                            lang: f.lang,
                            sha: short_sha(&src),
                            bytes: src.len() as u64,
                            test: parse::test_path(&f.path),
                            parsed: parse::parse_file(&f.path, f.lang, &src),
                        });
                    }
                    out
                }))
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect::<Vec<_>>()
    });
    let why = *why.lock().unwrap_or_else(|e| e.into_inner());
    match why {
        Some(a) => Err(a),
        None => Ok(parsed),
    }
}

/// One in-process mutex per index file: concurrent callers coalesce.
static BUILDS: Lazy<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Unique temp names within this process.
static BUILD_SEQ: AtomicU64 = AtomicU64::new(0);

/// The cross-process build lock (`.<index>.lock`, created exclusively),
/// removed on drop.
struct BuildLock(PathBuf);

impl BuildLock {
    /// Take the lock, waiting for another process's build until `deadline`;
    /// a lock older than [`STALE_LOCK`] is broken.
    fn acquire(path: PathBuf, deadline: Instant) -> Result<BuildLock, String> {
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(BuildLock(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok());
                    if age.is_some_and(|a| a > STALE_LOCK) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err("another Hollow process is building this checkout's code index; try again in a minute.".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("could not lock the code index: {e}")),
            }
        }
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The index of `root`, rebuilt first when its files changed since the
/// last build. Returns the index file. (Calls read [`open`]: base plus
/// overlay, falling back to this.)
#[cfg(test)]
pub(crate) fn ensure(root: &Path, cache: Option<&Path>) -> Result<PathBuf, String> {
    ensure_with(root, cache, &LIMITS)
}

/// [`ensure`] under `limits`. One build per index file: callers in this
/// process coalesce on its mutex, other processes wait on its lock file,
/// and the stamp is checked again after each wait, so a waiter reuses the
/// build it waited for.
pub(crate) fn ensure_with(
    root: &Path,
    cache: Option<&Path>,
    limits: &Limits,
) -> Result<PathBuf, String> {
    let deadline = Instant::now() + limits.deadline;
    let git = Git::open(root)?;
    let path = index_path(&git.root, cache).ok_or("no cache folder for the code index")?;
    let listing = list_with(&git)?;
    let want = stamp(&listing);
    let current = || store::read_meta(&path, "stamp").as_deref() == Some(want.as_str());
    if current() {
        return Ok(path);
    }
    within_limits(&listing, limits)?;
    build_locked(&path, deadline, &current, |tmp| {
        build_listed(&git, tmp, &listing, deadline, limits).map(|_| ())
    })?;
    Ok(path)
}

/// Build the index file `path` once: callers in this process coalesce on
/// its mutex, other processes wait on its lock file (until `deadline`),
/// `current` is asked again after each wait (true: nothing to build), and
/// `build` writes a temp file that is swapped in by atomic rename. Dead
/// builds' temp files are pruned after.
pub(crate) fn build_locked(
    path: &Path,
    deadline: Instant,
    current: &dyn Fn() -> bool,
    build: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let slot = BUILDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(path.to_path_buf())
        .or_default()
        .clone();
    let _held = slot.lock().unwrap_or_else(|e| e.into_inner());
    if current() {
        return Ok(());
    }
    let dir = path.parent().ok_or("bad index path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create the index folder: {e}"))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("index")
        .to_string();
    let _lock = BuildLock::acquire(dir.join(format!(".{name}.lock")), deadline)?;
    if current() {
        return Ok(());
    }
    let tmp = dir.join(format!(
        ".{name}.{}.{}.building",
        std::process::id(),
        BUILD_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&tmp);
    if let Err(e) = build(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("could not swap in the new index: {e}"));
    }
    prune_building(dir, &name, STALE_BUILDING);
    Ok(())
}

/// Remove `name`'s temp `.building` files older than `age` (dead builds').
fn prune_building(dir: &Path, name: &str, age: Duration) {
    let prefix = format!(".{name}.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if !file.starts_with(&prefix) || !file.ends_with(".building") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|a| a > age);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Files with uncommitted changes (`git status --porcelain`, hardened: no
/// optional locks, so an agent's concurrent commit never meets our
/// `index.lock`; submodules ignored), counted.
pub(crate) fn dirty_files(root: &Path) -> usize {
    Git::open(root)
        .and_then(|g| g.run(&["status", "--porcelain", "--ignore-submodules=all"]))
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
