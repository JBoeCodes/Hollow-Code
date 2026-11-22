use crate::types::{BacklinkMatch, BacklinkSource, Note, NoteContent, NoteRef, SearchResult};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

/// The vault link index. Map from rel-path -> outgoing+incoming basenames.
#[derive(Default, Clone)]
pub struct LinkIndex {
    pub by_path: HashMap<String, NoteIndex>,
    pub by_basename: HashMap<String, String>, // lowercase basename -> rel-path
}

#[derive(Default, Clone)]
pub struct NoteIndex {
    pub basename: String,
    /// What the note says it is about: frontmatter `title` or its first
    /// `# heading`, and the first sentence of prose. The Library lists
    /// these instead of file names like `P046be4a221cb`.
    pub title: Option<String>,
    pub summary: Option<String>,
    pub outgoing: Vec<String>,
    pub incoming_from: Vec<String>,
}

static INDEX: Lazy<Mutex<Option<LinkIndex>>> = Lazy::new(|| Mutex::new(None));

/// Short-TTL cache of the full vault walk, keyed by vault root. One
/// `vault-changed` burst makes the frontend refetch the tree, the semantic
/// index, the link index, and often a folder resource back-to-back, and each
/// used to pay its own full directory walk. The watcher and Hollow's own
/// write/delete/move paths invalidate eagerly; the TTL bounds staleness for
/// any write path that doesn't. Disabled under `cfg(test)` so multi-step unit
/// tests always see fresh walks.
static WALK_CACHE: Lazy<Mutex<Option<(PathBuf, Instant, Vec<Note>)>>> =
    Lazy::new(|| Mutex::new(None));
const WALK_CACHE_TTL: Duration = Duration::from_secs(2);

/// Bumped by every [`invalidate_walk_cache`] (the watcher's changes and
/// Hollow's own writes): a longer-lived cache derived from the walk (the
/// bell's conflict-copy scan, overnight A14) stays valid while it is unchanged.
static WALK_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The current walk generation (see [`WALK_GENERATION`]).
pub fn walk_generation() -> u64 {
    WALK_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn invalidate_walk_cache() {
    WALK_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut g) = WALK_CACHE.lock() {
        *g = None;
    }
}

/// Monotonic counter that disambiguates trash filenames created within the
/// same nanosecond (or when the clock isn't monotonic). Combined with a
/// `SystemTime` nanosecond stamp it makes every move-to-trash name unique.
static TRASH_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Monotonic counter that disambiguates atomic-write temp filenames created
/// within the same nanosecond. Combined with the pid + a nanosecond stamp it
/// keeps every sibling temp file unique across concurrent writers.
static ATOMIC_WRITE_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Atomically write `content` to `path` via a sibling temp file + `rename(2)`.
///
/// **Data safety (Tier 0.1):** a raw `fs::write` truncates the target and then
/// streams the new bytes, so a crash / power-loss / disk-full mid-write — or a
/// concurrent reader (Obsidian, the vault watcher, another Hollow read) — can
/// observe an empty or half-written note. Writing to a uniquely-named temp file
/// in the SAME directory and then renaming it into place is atomic on a single
/// filesystem (macOS/Linux), so a reader always sees either the whole old file
/// or the whole new one, never a torn prefix.
///
/// **Durability (merged plan slice 4, A·D6 / B·F14):** the rename alone is only
/// atomic for readers. Without a sync, a power loss after the rename can leave
/// the new name pointing at a file whose bytes never reached the disk (an empty
/// note), or lose the rename itself. So the temp file is flushed to stable
/// storage before the rename ([`full_sync`]: `F_FULLFSYNC` on macOS, `fsync`
/// elsewhere) and the directory is synced after it, so the new entry survives
/// too. The cost is two drive-cache flushes per write, typically a few
/// milliseconds each on an Apple SSD and more on a busy or external disk;
/// callers that write many files in a row pay it per file, unless they use
/// [`atomic_write_batched`] + [`flush_dir`] (History objects do).
///
/// The replaced file's permission bits are kept: `rename(2)` installs the temp
/// file's inode, so without this every save would reset a `chmod`ed note to the
/// umask default. A new file gets the usual `0666 & !umask`.
///
/// The temp file lives beside the target (not `/tmp`) so the rename never
/// crosses a filesystem boundary — a cross-device rename degrades to a
/// non-atomic copy. Its name is dot-prefixed so the vault watcher + walker
/// ignore it (a transient temp file never triggers a refetch or shows in the
/// tree), and it is created exclusively, so a link planted at that name is
/// never followed. The parent directory is created if missing. Port of
/// hollow-mac's `writeNote(atomically: true)`.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<(), String> {
    atomic_write_with(path, content, false, Durability::Full)
}

/// One operation's write log ([`logging_writes`]): shared by every thread
/// the operation runs on. Each file a write primitive touched, with the
/// sha256 hex ([`revision`]) of the bytes that write left there, taken when
/// it wrote them (`None`: it moved or removed the file, or left no regular
/// file there). A later write to the same file replaces its hash
/// (2026-11-13).
pub(crate) type WriteLog =
    std::sync::Arc<Mutex<std::collections::BTreeMap<PathBuf, Option<String>>>>;

thread_local! {
    /// The write log this thread's vault writes go to, while an operation
    /// that collects them runs here ([`logging_writes`], or a worker it
    /// started through [`spawn_blocking_logged`]; chat concurrency F1,
    /// 2026-10-20).
    static WRITE_LOG: std::cell::RefCell<Option<WriteLog>> =
        const { std::cell::RefCell::new(None) };
}

/// Installs a write log on this thread and puts the previous one back when
/// dropped, on unwind too.
struct InstalledLog(Option<WriteLog>);

impl InstalledLog {
    fn new(log: Option<WriteLog>) -> Self {
        Self(WRITE_LOG.with(|slot| slot.replace(log)))
    }
}

impl Drop for InstalledLog {
    fn drop(&mut self) {
        let previous = self.0.take();
        WRITE_LOG.with(|slot| *slot.borrow_mut() = previous);
    }
}

/// Run `f`, returning what it returned and the files the vault write
/// primitives ([`atomic_write`] and its kin, [`delete_note`],
/// [`rename_no_replace`], [`replace_from`]) touched for it, as the paths
/// they were given, sorted and once each, each with the hash of what its
/// latest write left there ([`WriteLog`]). Every thread the operation uses
/// logs into it: this one, and each worker it starts through
/// [`spawn_blocking_logged`] (the commands Hollow's app tools call do their
/// work there). History credits a scoped run with what a Hollow app or task
/// write on its behalf wrote in its folder from this, so a peer's own write
/// landing at the same moment isn't counted. Nests: an outer log also gets
/// the inner one's files.
pub(crate) fn logging_writes<T>(
    f: impl FnOnce() -> T,
) -> (T, std::collections::BTreeMap<PathBuf, Option<String>>) {
    let log: WriteLog = std::sync::Arc::new(Mutex::new(std::collections::BTreeMap::new()));
    let outer = current_write_log();
    let result = {
        let _installed = InstalledLog::new(Some(log.clone()));
        f()
    };
    let written = std::mem::take(&mut *log.lock().unwrap_or_else(|e| e.into_inner()));
    // The inner operation's writes all came after the outer one's so far,
    // so its hashes win.
    if let Some(outer) = outer {
        outer.lock().unwrap_or_else(|e| e.into_inner()).extend(
            written
                .iter()
                .map(|(path, hash)| (path.clone(), hash.clone())),
        );
    }
    (result, written)
}

/// This thread's write log, if an operation is collecting one.
pub(crate) fn current_write_log() -> Option<WriteLog> {
    WRITE_LOG.with(|slot| slot.borrow().clone())
}

/// `crate::shared_mcp::blocking` that carries this thread's write
/// log ([`logging_writes`]) into the worker, so an app tool's command that
/// does its write there is still credited to the run it ran for. Called on
/// a thread that holds `HISTORY_LOCK` (inside `with_live_run`), `f` runs to
/// completion before this returns, on a worker that works under that hold
/// (`automated_history::run_under_hold`), so a host write there (a session
/// record) neither waits on the thread waiting for it nor outlives the
/// hold; the returned [`LoggedJoin`] is already finished.
pub(crate) fn spawn_blocking_logged<F, R>(f: F) -> LoggedJoin<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let log = current_write_log();
    let with_log = move || {
        let _installed = InstalledLog::new(log);
        f()
    };
    if crate::automated_history::history_lock_held_here() {
        // Already finished: nothing is scheduled, so awaiting it never needs
        // a runtime worker, which may all be blocked on the lock this thread
        // holds (review 2026-10-20).
        return LoggedJoin::Ready(Some(crate::automated_history::run_under_hold(with_log)));
    }
    LoggedJoin::Spawned(crate::shared_mcp::blocking(with_log))
}

/// What [`spawn_blocking_logged`] returns: a blocking task to await, or,
/// under a `HISTORY_LOCK` hold, its outcome already in hand, which resolves
/// on the first poll without a runtime.
pub(crate) enum LoggedJoin<T> {
    Ready(Option<std::thread::Result<T>>),
    Spawned(tauri::async_runtime::JoinHandle<T>),
}

// The value is only ever moved out whole, never pinned in place.
impl<T> Unpin for LoggedJoin<T> {}

/// A [`LoggedJoin`] that didn't finish: its task failed or panicked.
#[derive(Debug)]
pub(crate) struct LoggedJoinError(String);

impl std::fmt::Display for LoggedJoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LoggedJoinError {}

impl<T> std::future::Future for LoggedJoin<T> {
    type Output = Result<T, LoggedJoinError>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.get_mut() {
            LoggedJoin::Ready(outcome) => std::task::Poll::Ready(
                match outcome
                    .take()
                    .expect("a LoggedJoin polled after it finished")
                {
                    Ok(value) => Ok(value),
                    Err(panic) => Err(LoggedJoinError(format!(
                        "the task panicked: {}",
                        panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_default()
                    ))),
                },
            ),
            LoggedJoin::Spawned(handle) => std::pin::Pin::new(handle)
                .poll(cx)
                .map(|joined| joined.map_err(|error| LoggedJoinError(error.to_string()))),
        }
    }
}

/// Note `path` in this thread's write log, if one is open, with no hash:
/// the write moved or removed it, so nothing it left there is credited.
fn log_write(path: &Path) {
    log_write_hashed(path, || None);
}

/// Note `path` in this thread's write log, if one is open, with the hash
/// of `content`, the bytes this write just put there (2026-11-13): History
/// credits a run with a file only while it still has them, so a later
/// write by anyone else isn't taken for this one.
fn log_written(path: &Path, content: &[u8]) {
    log_write_hashed(path, || Some(revision(content)));
}

/// Note `path` with `hash()` (computed only when a log is open); the
/// latest note of a file wins.
fn log_write_hashed(path: &Path, hash: impl FnOnce() -> Option<String>) {
    if let Some(log) = current_write_log() {
        let hash = hash();
        log.lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(path.to_path_buf(), hash);
    }
}

/// [`atomic_write`] for app state that must stay owner-only: the file is
/// always `0600` on unix, whatever mode the replaced file had (budgets,
/// permission grants, service records, connection inventories).
pub fn atomic_write_private(path: &Path, content: &[u8]) -> Result<(), String> {
    atomic_write_with(path, content, true, Durability::Full)
}

/// [`atomic_write`] for one file of a batch whose drive-cache flush is
/// deferred to a single [`flush_dir`] before anything that refers to the
/// batch is committed. The temp file gets a plain `fsync(2)` before the
/// rename, which hands its bytes to the drive but (on macOS) skips the
/// `F_FULLFSYNC` cache flush, and the directory is not synced here. The
/// caller's later [`flush_dir`] on the same directory issues the one
/// `F_FULLFSYNC`, which flushes the whole drive cache, so every file the
/// batch fsynced becomes durable at once, along with the directory entries.
///
/// Used for History's content-addressed objects: a full-vault baseline can
/// write thousands of them at run start, and two full flushes per object
/// stalled the run for tens of seconds. Readers still never see a torn file;
/// a crash before the [`flush_dir`] can lose objects that nothing durable
/// refers to yet.
pub fn atomic_write_batched(path: &Path, content: &[u8]) -> Result<(), String> {
    atomic_write_with(path, content, false, Durability::Batched)
}

/// Flush `dir` (its entries, and on macOS the whole drive cache) to stable
/// storage: the commit point for files written with
/// [`atomic_write_batched`]. Best-effort, like the directory sync in
/// [`atomic_write`]: a directory that can't be opened or synced (some network
/// mounts) is skipped rather than failing the caller.
pub fn flush_dir(dir: &Path) {
    if fs::File::open(dir)
        .and_then(|handle| full_sync(&handle))
        .is_ok()
    {
        trace_sync(|| format!("sync-dir {}", dir.display()));
    }
}

/// How much of the sync work [`atomic_write_with`] does itself.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Durability {
    /// `F_FULLFSYNC` the temp file, rename, then sync the directory.
    Full,
    /// Plain `fsync` the temp file and rename; the caller calls
    /// [`flush_dir`] once for the whole batch.
    Batched,
}

fn atomic_write_with(
    path: &Path,
    content: &[u8],
    private: bool,
    durability: Durability,
) -> Result<(), String> {
    install_temp(path, content, private, durability, false).map(|_| ())
}

/// How [`install_temp`] put the new file in place.
enum Installed {
    /// Renamed over the target: whatever was there is gone unseen.
    Renamed,
    /// Swapped with the target (`renamex_np` `RENAME_SWAP`): the displaced
    /// file now lives at this sibling temp path, for the caller to read and
    /// remove; the handle is Hollow's own new file (open for reading, so it
    /// can be recognised whatever mode it inherited).
    Swapped(PathBuf, fs::File),
    /// A swap found no file at the target (it was removed meanwhile);
    /// nothing was written.
    TargetGone,
}

/// Atomically swap the files at `a` and `b` (macOS `renamex_np` with
/// `RENAME_SWAP`, both must exist and be on one APFS volume).
#[cfg(target_os = "macos")]
fn swap_files(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    // SAFETY: both pointers are valid NUL-terminated paths for the call.
    if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// True when a swap failed only because the volume or OS can't swap
/// (`ENOTSUP` / `EINVAL` from `renamex_np`, `Unsupported` elsewhere): the
/// caller falls back to a rename. Other errors are propagated (delta review
/// 2026-10-20).
fn swap_unsupported(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(
            error.raw_os_error(),
            Some(code) if code == libc::ENOTSUP || code == libc::EOPNOTSUPP || code == libc::EINVAL
        )
    }
    #[cfg(not(unix))]
    false
}

/// No atomic swap off macOS: callers fall back to a plain rename.
#[cfg(not(target_os = "macos"))]
fn swap_files(_a: &Path, _b: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// [`atomic_write_with`]'s temp-file write, then the install: a rename, or
/// with `swap` an atomic swap that hands back the displaced file
/// ([`Installed::Swapped`]). A volume or OS without the swap falls back to
/// the rename.
fn install_temp(
    path: &Path,
    content: &[u8],
    private: bool,
    durability: Durability,
    swap: bool,
) -> Result<Installed, String> {
    use std::io::Write;
    let dir = path
        .parent()
        .ok_or_else(|| "write: path has no parent dir".to_string())?;
    fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = ATOMIC_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let basename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "note".to_string());
    let tmp = dir.join(format!(
        ".{basename}.{}.{nanos}.{seq}.tmp",
        std::process::id()
    ));
    let mut options = fs::OpenOptions::new();
    options.read(swap).write(true).create_new(true);
    #[cfg(unix)]
    let keep_mode = {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let keep = if private {
            None
        } else {
            fs::metadata(path)
                .ok()
                .map(|m| m.permissions().mode() & 0o777)
        };
        // Create with the final bits (narrowed by the umask) so the content
        // is never readable more widely than the file it replaces.
        options.mode(if private {
            0o600
        } else {
            keep.unwrap_or(0o666)
        });
        keep
    };
    #[cfg(not(unix))]
    let _ = private;
    // Open before taking cleanup ownership: a name we did not create is never
    // removed on failure.
    let mut file = options.open(&tmp).map_err(|e| format!("write: {e}"))?;
    let result = (|| -> Result<Installed, String> {
        #[cfg(unix)]
        if let Some(mode) = keep_mode {
            use std::os::unix::fs::PermissionsExt;
            // The umask may have dropped bits the old file had (group write).
            file.set_permissions(fs::Permissions::from_mode(mode))
                .map_err(|e| format!("write: {e}"))?;
        }
        file.write_all(content).map_err(|e| format!("write: {e}"))?;
        match durability {
            Durability::Full => {
                full_sync(&file).map_err(|e| format!("sync: {e}"))?;
                trace_sync(|| format!("sync-file {}", tmp.display()));
            }
            Durability::Batched => {
                data_sync(&file).map_err(|e| format!("sync: {e}"))?;
                trace_sync(|| format!("fsync-file {}", tmp.display()));
            }
        }
        if swap {
            // Cloned BEFORE the swap (delta review 3, 2026-10-20): once it
            // succeeds `tmp` holds the displaced file, so nothing fallible may
            // follow it here; the error cleanup below would delete that file.
            let ours = file.try_clone().map_err(|e| format!("write: {e}"))?;
            match swap_files(&tmp, path) {
                Ok(()) => {
                    trace_sync(|| format!("swap {}", path.display()));
                    return Ok(Installed::Swapped(tmp.clone(), ours));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Installed::TargetGone)
                }
                // A volume without the swap (or another OS): the plain
                // rename below. Any other failure is a real one.
                Err(e) if swap_unsupported(&e) => {}
                Err(e) => return Err(format!("write: swap: {e}")),
            }
        }
        fs::rename(&tmp, path).map_err(|e| format!("write: {e}"))?;
        trace_sync(|| format!("rename {}", path.display()));
        Ok(Installed::Renamed)
    })();
    drop(file);
    match &result {
        Err(_) | Ok(Installed::TargetGone) => {
            let _ = fs::remove_file(&tmp);
            return result;
        }
        Ok(_) => {}
    }
    // Logged only when the write is known to be Hollow's: always after a
    // rename; after a swap only once the displaced file is verified
    // ([`replace_verified`]), since a swap may hand the file back.
    if matches!(result, Ok(Installed::Renamed)) {
        log_written(path, content);
    }
    // Best-effort: the new file is already in place, so a directory that
    // can't be opened or synced (some network mounts) must not report the
    // write as failed.
    if durability == Durability::Full {
        flush_dir(dir);
    }
    result
}

/// Flush `file` to stable storage. On macOS `fsync(2)` only hands the data to
/// the drive, which can hold it in a volatile cache across a power loss;
/// `F_FULLFSYNC` also flushes that cache. Filesystems that don't support it
/// (some network and FUSE mounts) return an error, so fall back to `fsync`.
///
/// Unit tests skip the drive-cache flush ([`DRIVE_FLUSH`]) and take the
/// `fsync` path.
#[cfg(target_os = "macos")]
fn full_sync(file: &fs::File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();
    // SAFETY: `fd` is open for the whole call (borrowed from `file`), and
    // F_FULLFSYNC takes no argument and touches no memory.
    if DRIVE_FLUSH && unsafe { libc::fcntl(fd, libc::F_FULLFSYNC) } != -1 {
        return Ok(());
    }
    // SAFETY: same descriptor; fsync(2) touches no memory. (`sync_all` is
    // not the fallback: on Apple targets std implements it as F_FULLFSYNC.)
    if unsafe { libc::fsync(fd) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Whether [`full_sync`] issues macOS's `F_FULLFSYNC` drive-cache flush.
/// Off in unit tests only (overnight OB13): the flush empties the whole
/// drive's cache, so it serializes across every test thread and process on
/// the machine. Measured with `--report-time` on `a01475a0`: libtest spent
/// 113 s wall for 218 s of CPU (85 user, 133 sys) with the flush, 57 s
/// without, and raising `--test-threads` from 18 to 48 didn't help (106 s).
/// No test can observe the flush (it only matters across a power loss);
/// tests still run the `fsync` that hands the bytes to the drive.
#[cfg(target_os = "macos")]
const DRIVE_FLUSH: bool = !cfg!(test);

/// Flush `file` to stable storage (`fsync(2)` off macOS).
#[cfg(not(target_os = "macos"))]
fn full_sync(file: &fs::File) -> std::io::Result<()> {
    file.sync_all()
}

/// Hand `file`'s bytes to the drive with a plain `fsync(2)`. On macOS this
/// skips the drive-cache flush; a later [`full_sync`] of any file on the same
/// drive (see [`flush_dir`]) makes it durable.
#[cfg(target_os = "macos")]
fn data_sync(file: &fs::File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: `fd` is open for the whole call (borrowed from `file`), and
    // fsync(2) touches no memory. (`sync_all`/`sync_data` would issue
    // F_FULLFSYNC on Apple targets, which is the cost this avoids.)
    if unsafe { libc::fsync(file.as_raw_fd()) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Flush `file` to stable storage (`fsync(2)` off macOS, where it already
/// reaches the platter).
#[cfg(not(target_os = "macos"))]
fn data_sync(file: &fs::File) -> std::io::Result<()> {
    file.sync_all()
}

/// Test seam: run `f` with the durability trace on, and return its steps.
#[cfg(test)]
pub(crate) fn capture_sync_trace<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    SYNC_TRACE.with(|t| *t.borrow_mut() = Some(Vec::new()));
    let value = f();
    let steps = SYNC_TRACE
        .with(|t| t.borrow_mut().take())
        .unwrap_or_default();
    (value, steps)
}

#[cfg(test)]
thread_local! {
    /// Test seam: when `Some`, [`atomic_write`] appends each durability step
    /// here, in order, on the calling thread.
    static SYNC_TRACE: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn trace_sync(step: impl FnOnce() -> String) {
    SYNC_TRACE.with(|trace| {
        if let Some(steps) = trace.borrow_mut().as_mut() {
            steps.push(step());
        }
    });
}

#[cfg(not(test))]
fn trace_sync(_step: impl FnOnce() -> String) {}

/// Threads a cold link-index build reads notes on, at most.
const INDEX_THREADS: usize = 4;
/// Index builds that took the threaded path (the A14 guard).
#[cfg(test)]
pub(crate) static INDEX_THREADED_BUILDS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
static WIKILINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\[([^\]\n|#]+?)(?:\|[^\]\n]+)?(?:#[^\]\n]+)?\]\]").unwrap());

/// Matches a daily-brief filename: `YYYY-MM-DD.{html,htm,md}`. Captures the
/// date and extension. Non-dated files in `briefings/` (e.g.
/// `workflow-suggestions-2026-05-27.md`) deliberately don't match.

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum VaultNode {
    #[serde(rename = "folder")]
    Folder {
        children: HashMap<String, VaultNode>,
    },
    #[serde(rename = "file")]
    File {
        path: String,
        name: String,
        mtime: String,
        size: u64,
    },
}

fn iso_mtime(meta: &fs::Metadata) -> String {
    meta.modified()
        .ok()
        .map(|t| DateTime::<Utc>::from(t).to_rfc3339())
        .unwrap_or_default()
}

/// True for any path component beginning with `.` — used to prune hidden
/// directories (`.trash`, `.git`, `.obsidian`, …) from the walk so their
/// contents never surface in the tree, search, or the link index.
fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

pub fn walk_notes(vault: &Path) -> Vec<Note> {
    if !cfg!(test) {
        if let Ok(g) = WALK_CACHE.lock() {
            if let Some((root, at, notes)) = g.as_ref() {
                if root == vault && at.elapsed() < WALK_CACHE_TTL {
                    return notes.clone();
                }
            }
        }
    }
    let out = walk_notes_fresh(vault);
    if !cfg!(test) {
        if let Ok(mut g) = WALK_CACHE.lock() {
            *g = Some((vault.to_path_buf(), Instant::now(), out.clone()));
        }
    }
    out
}

/// Top-level folders [`walk_notes`] never enters: the retired message
/// stores and the signature folder, kept out of agent reads.
const WALK_SKIPPED_TOP: &[&str] = &["emails", "teams", "slack", "inbox", "_signature"];

/// Whether the vault-relative `rel` sits in a top-level folder the walk
/// skips.
fn in_skipped_top(rel: &Path) -> bool {
    rel.components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .is_some_and(|c| WALK_SKIPPED_TOP.contains(&c))
}

/// Whether [`walk_notes`] would skip the vault-relative `rel`: a hidden
/// component anywhere (`.trash`, `.obsidian`, ...) or a skipped top-level
/// folder. For code that reads a note by path rather than from the walk
/// (the F7 lifecycle stage), so it can't cite what the walk keeps out.
pub fn walk_skips(rel: &str) -> bool {
    let rel = Path::new(rel);
    in_skipped_top(rel)
        || rel
            .components()
            .any(|c| c.as_os_str().to_str().is_none_or(|s| s.starts_with('.')))
}

/// A walk that never uses the short walk cache: a longer-lived cache keyed
/// by [`walk_generation`] must not store a list cached before a change.
pub(crate) fn walk_notes_uncached(vault: &Path) -> Vec<Note> {
    walk_notes_fresh(vault)
}

fn walk_notes_fresh(vault: &Path) -> Vec<Note> {
    let mut out = Vec::new();
    for entry in WalkDir::new(vault)
        .into_iter()
        .filter_entry(|e| {
            !is_hidden(e)
                && !e
                    .path()
                    .strip_prefix(vault)
                    .ok()
                    .is_some_and(in_skipped_top)
        })
        .filter_map(Result::ok)
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        // Markdown notes are the vault's core, but `.html` files (e.g. the
        // Morning Brief workflow's HTML output) are now first-class too — they
        // surface in the tree and render in the Vault tab (sandboxed).
        if !(name.ends_with(".md") || name.ends_with(".html") || name.ends_with(".htm")) {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(vault)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        out.push(Note {
            path: rel,
            name,
            mtime: iso_mtime(&meta),
            size: meta.len(),
        });
    }
    out
}

pub fn build_tree(vault: &Path) -> HashMap<String, VaultNode> {
    let notes = walk_notes(vault);
    let mut root: HashMap<String, VaultNode> = HashMap::new();
    for note in notes {
        let parts: Vec<&str> = note.path.split('/').collect();
        let mut cur = &mut root;
        for part in &parts[..parts.len() - 1] {
            let key = part.to_string();
            let entry = cur.entry(key).or_insert_with(|| VaultNode::Folder {
                children: HashMap::new(),
            });
            cur = match entry {
                VaultNode::Folder { children } => children,
                _ => return HashMap::new(),
            };
        }
        let last = parts[parts.len() - 1].to_string();
        cur.insert(
            last,
            VaultNode::File {
                path: note.path.clone(),
                name: note.name.clone(),
                mtime: note.mtime.clone(),
                size: note.size,
            },
        );
    }
    root
}

pub fn recent(vault: &Path, limit: usize) -> Vec<Note> {
    // Filter before capping: a connector refresh must not fill Today's recent
    // notes with generated mirrors and crowd the user's writing out entirely.
    let empty_frontmatter = std::collections::BTreeMap::new();
    let mut all: Vec<Note> = walk_notes(vault)
        .into_iter()
        .filter(|note| {
            note.path.ends_with(".md")
                && !crate::vault_index::classify(&note.path, &empty_frontmatter).hidden
        })
        .collect();
    all.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    all.into_iter().take(limit).collect()
}

/// Keyword search. Every term must match, in any order, in a note's file name
/// or body; quoted phrases, the short-term word-start rule, disjoint ranking
/// bands and file-name typo recovery are specified in [`crate::vault_search`].
/// When `kinds` is `Some`, only files whose semantic kind (see
/// `vault_index::classify`) is in the set are returned. The kind filter is
/// applied *before* the result cap so a narrow lens still surfaces its full
/// top-N rather than whatever survived a post-cap filter.
pub fn search(
    vault: &Path,
    query: &str,
    limit: usize,
    kinds: Option<&[String]>,
) -> Vec<SearchResult> {
    crate::vault_search::search(vault, query, limit, kinds)
}

pub fn read_note(vault: &Path, rel: &str) -> Result<NoteContent, String> {
    let full = safe_join(vault, rel)?;
    let content = fs::read_to_string(&full).map_err(|e| format!("read: {e}"))?;
    let meta = full.metadata().map_err(|e| format!("stat: {e}"))?;
    Ok(NoteContent {
        path: rel.to_string(),
        revision: revision(content.as_bytes()),
        content,
        mtime: iso_mtime(&meta),
        size: meta.len(),
    })
}

/// A note's revision: sha256 hex of its exact bytes. [`NoteContent`] carries
/// it, and a writer passes it back as [`Expected::Revision`].
pub fn revision(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Blind overwrite (or create) of a note: the last writer wins. It still
/// runs under the vault mutate lock ([`lock_mutate`]), so it can never land
/// between another Hollow mutator's read and its write (review finding A·D7:
/// the agent's `vault_write_note` used to bypass the lock). A caller that
/// knows what it read should use [`write_note_expecting`] instead. A caller
/// that already holds the lock uses [`write_note_locked`].
pub fn write_note(vault: &Path, rel: &str, content: &str) -> Result<NoteContent, String> {
    let guard = lock_mutate();
    write_note_locked(&guard, vault, rel, content)
}

/// [`write_note`] for a caller that already holds the mutate lock.
pub fn write_note_locked(
    guard: &MutateGuard,
    vault: &Path,
    rel: &str,
    content: &str,
) -> Result<NoteContent, String> {
    match write_note_expecting_locked(guard, vault, rel, content, Expected::Anything)? {
        CasOutcome::Written(note) => Ok(note),
        CasOutcome::Conflict(_) => Err("a blind write has no expectation to miss".into()),
    }
}

/// Stat a file Hollow just wrote, refresh every derived index for it and
/// describe it.
fn note_written(
    vault: &Path,
    rel: &str,
    full: &Path,
    content: &str,
) -> Result<NoteContent, String> {
    let meta = full.metadata().map_err(|e| format!("stat: {e}"))?;
    invalidate_walk_cache();
    refresh_index_for_file(rel, content);
    let mtime = iso_mtime(&meta);
    crate::vault_search_index::on_write(vault, rel, content, &mtime);
    crate::vault_embed_index::on_write(vault, rel, content, &mtime, meta.len());
    Ok(NoteContent {
        path: rel.to_string(),
        content: content.to_string(),
        revision: revision(content.as_bytes()),
        mtime,
        size: meta.len(),
    })
}

/// Publish a complete new Markdown note without ever replacing an existing
/// path. A sibling temporary file plus hard-link makes publication atomic and
/// exclusive, including when Obsidian creates the same note concurrently.
pub fn create_note(vault: &Path, rel: &str, content: &str) -> Result<NoteContent, String> {
    if !rel.to_lowercase().ends_with(".md")
        || Path::new(rel).components().any(|part| {
            matches!(part, std::path::Component::Normal(name) if name.to_string_lossy().starts_with('.'))
        })
    {
        return Err("New notes must be visible Markdown files".into());
    }
    let full = safe_join(vault, rel)?;
    match publish_new(vault, rel, &full, content)? {
        Some(note) => Ok(note),
        None => Err(format!("Note already exists: {rel}")),
    }
}

/// Publish `content` at `full` only if nothing is there yet: a sibling
/// temporary file plus an exclusive hard link, so publication is atomic and
/// never replaces a file, even one another program creates at the same
/// moment. `Ok(None)` when the path already exists. Durable like
/// [`atomic_write`]: the temporary file is synced before the link, and the
/// folder once after the link and the temporary name's removal, so a power
/// loss can't drop the new entry or resurrect the temporary one.
fn publish_new(
    vault: &Path,
    rel: &str,
    full: &Path,
    content: &str,
) -> Result<Option<NoteContent>, String> {
    let parent = full.parent().ok_or("Note has no parent folder")?;
    fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    let seq = ATOMIC_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(
        ".hollow-create-{}-{nanos}-{seq}.tmp",
        std::process::id()
    ));
    let result = (|| {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("create temporary note: {e}"))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("write note: {e}"))?;
        full_sync(&file).map_err(|e| format!("sync note: {e}"))?;
        match fs::hard_link(&tmp, full) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
            Err(e) => return Err(format!("create note: {e}")),
        }
        trace_sync(|| format!("link {}", full.display()));
        log_written(full, content.as_bytes());
        // Drop the temporary name before the folder sync, so one sync makes
        // both the new entry and the removal durable.
        if fs::remove_file(&tmp).is_ok() {
            trace_sync(|| format!("unlink {}", tmp.display()));
        }
        // Best-effort, as in `atomic_write`: the note is already published.
        flush_dir(parent);
        invalidate_walk_cache();
        refresh_index_for_file(rel, content);
        let note = read_note(vault, rel)?;
        crate::vault_search_index::on_write(vault, rel, content, &note.mtime);
        crate::vault_embed_index::on_write(vault, rel, content, &note.mtime, note.size);
        Ok(Some(note))
    })();
    // A refused or failed publication leaves the temporary file behind.
    let _ = fs::remove_file(&tmp);
    result
}

/// Build a native Obsidian handoff from the configured vault, never from an
/// arbitrary caller-supplied URI. Values are encoded separately, so note names
/// containing spaces, hashes, or ampersands cannot become URI parameters.
pub fn obsidian_uri(vault: &Path, rel: Option<&str>, daily: bool) -> Result<String, String> {
    let root = vault.canonicalize().map_err(|e| format!("vault: {e}"))?;
    let name = root
        .file_name()
        .ok_or("Vault has no folder name")?
        .to_string_lossy();
    if daily {
        if rel.is_some_and(|p| !p.is_empty()) {
            return Err("Daily-note handoff does not accept a file path".into());
        }
        return Ok(format!(
            "obsidian://daily?vault={}",
            urlencoding::encode(&name)
        ));
    }
    if let Some(rel) = rel.filter(|p| !p.is_empty()) {
        let full = safe_join(&root, rel)?;
        if !full.is_file() {
            return Err("Note does not exist".into());
        }
        return Ok(format!(
            "obsidian://open?path={}",
            urlencoding::encode(&full.to_string_lossy())
        ));
    }
    Ok(format!(
        "obsidian://open?vault={}",
        urlencoding::encode(&name)
    ))
}

/// Serializes this process's vault writers of existing notes (the blind
/// write, the CAS writes, the generic `mutate_note`, `move_note` and the
/// task/commitment/workflow/workspace mutators built on them) so a read →
/// compare/transform → write happens with no other Hollow writer landing in
/// between (data safety Tier 0.3, merged plan slice 4 A·D7). Take it only
/// through [`lock_mutate`], which adds the cross-process half. Never call a
/// helper that re-locks while holding it (the std `Mutex` is not reentrant):
/// use the `_locked` writer variants. Where it sits among the other locks:
/// `crate::lock_order` (tier 5).
static MUTATE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// How long [`lock_mutate`] waits for another process (the headless
/// `hollow notion-sync` CLI, an MCP subprocess) to let go of the mutate
/// flock. Past it Hollow logs and carries on under the in-process lock
/// alone, so a hung process can neither freeze the UI nor deadlock.
pub(crate) const MUTATE_PROCESS_WAIT: Duration = Duration::from_secs(2);

/// Proof that the caller holds the vault mutate lock ([`lock_mutate`]).
/// Writers with a `_locked` suffix take one, so a caller already holding the
/// lock can write without re-locking.
pub struct MutateGuard {
    // Fields drop in order: the cross-process flock is released first, then
    // the in-process mutex (the reverse of how they were taken).
    _process: Option<crate::flock::HeldLock>,
    _thread: std::sync::MutexGuard<'static, ()>,
}

/// Take the vault mutate lock: [`MUTATE_LOCK`] for this process, then an
/// exclusive flock on `{data dir}/Hollow/Leases/vault-mutate.lock` for every
/// other Hollow process on this Mac (the notion-sync CLI runs in its own).
/// A poisoned mutex is recovered (the guarded value is `()`). The flock is
/// best-effort: if it can't be opened, or another process holds it past
/// [`MUTATE_PROCESS_WAIT`], the guard holds the in-process lock only.
pub fn lock_mutate() -> MutateGuard {
    let thread = MUTATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    MutateGuard {
        _process: take_process_mutate_lock(&mutate_lock_path(), MUTATE_PROCESS_WAIT),
        _thread: thread,
    }
}

/// The cross-process mutate flock's file. Beside the lease locks, outside
/// the vault; under `cfg(test)` a per-process temp dir, so a test never
/// contends with an installed Hollow.
pub(crate) fn mutate_lock_path() -> PathBuf {
    crate::lease_lock::leases_dir().join("vault-mutate.lock")
}

fn open_mutate_lock(path: &Path) -> std::io::Result<fs::File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Take the flock at `path`, polling for up to `wait` while another open
/// file description (another process) holds it. `None` on any failure.
fn take_process_mutate_lock(path: &Path, wait: Duration) -> Option<crate::flock::HeldLock> {
    let deadline = Instant::now() + wait;
    loop {
        let file = match open_mutate_lock(path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[vault] could not open the cross-process mutate lock: {error}");
                return None;
            }
        };
        match crate::flock::HeldLock::try_take(file) {
            Ok(lock) => return Some(lock),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(fs::TryLockError::WouldBlock) => {
                eprintln!(
                    "[vault] another process held the mutate lock for over {} ms; \
                     writing under this process's lock only",
                    wait.as_millis()
                );
                return None;
            }
            Err(fs::TryLockError::Error(error)) => {
                eprintln!("[vault] could not take the cross-process mutate lock: {error}");
                return None;
            }
        }
    }
}

/// Outcome of a write with an expectation ([`write_note_expecting`],
/// [`write_note_cas`]).
pub enum CasOutcome {
    /// The write went through; carries the freshly-written note.
    Written(NoteContent),
    /// The on-disk content no longer matched the caller's expectation, so
    /// NOTHING was written; carries the current on-disk note so the caller
    /// can offer reload / keep-mine. (For [`Expected::Absent`] this is the
    /// note that already exists.)
    Conflict(NoteContent),
}

/// What a writer expects to find at the path before it writes: the one
/// writer contract (merged plan slice 4, review finding A·D7). The check and
/// the write run together under the mutate lock, so no participating Hollow
/// writer can land between them. A missing file reads as `""` for
/// [`Expected::Content`] and [`Expected::Revision`].
#[derive(Clone, Copy, Debug)]
pub enum Expected<'a> {
    /// No expectation: the last writer wins ([`write_note`]).
    Anything,
    /// The path must not exist yet.
    Absent,
    /// The file's text must equal this (the editor's baseline,
    /// [`write_note_cas`]).
    Content(&'a str),
    /// The file's [`revision`] must equal this (what a reader got in
    /// [`NoteContent::revision`]).
    Revision(&'a str),
}

/// Write `content` to `rel` only if the file is still what `expected` says;
/// otherwise write nothing and return [`CasOutcome::Conflict`] with the
/// on-disk note. Takes the mutate lock. A writer outside Hollow's lock that
/// lands between the check and the replace is caught on macOS (an atomic
/// swap whose displaced bytes are compared, [`replace_verified`]) and
/// reported as a conflict instead of being overwritten.
pub fn write_note_expecting(
    vault: &Path,
    rel: &str,
    content: &str,
    expected: Expected<'_>,
) -> Result<CasOutcome, String> {
    let guard = lock_mutate();
    write_note_expecting_locked(&guard, vault, rel, content, expected)
}

/// [`write_note_expecting`] for a caller that already holds the mutate lock.
pub fn write_note_expecting_locked(
    guard: &MutateGuard,
    vault: &Path,
    rel: &str,
    content: &str,
    expected: Expected<'_>,
) -> Result<CasOutcome, String> {
    write_expecting(guard, vault, rel, content, expected, false)
}

/// [`write_note_expecting`] that, when `keep_mtime` is set, puts the file's
/// modification time back afterwards, as [`mutate_note_preserve_mtime`]
/// does (concurrency PR 2, 2026-10-20: Hollow's own brief writes keep the
/// overview order only while no chat runs in that workspace; see
/// `shared_docs::update_doc`).
pub fn write_note_expecting_mtime(
    vault: &Path,
    rel: &str,
    content: &str,
    expected: Expected<'_>,
    keep_mtime: bool,
) -> Result<CasOutcome, String> {
    let guard = lock_mutate();
    write_expecting(&guard, vault, rel, content, expected, keep_mtime)
}

fn write_expecting(
    _guard: &MutateGuard,
    vault: &Path,
    rel: &str,
    content: &str,
    expected: Expected<'_>,
    keep_mtime: bool,
) -> Result<CasOutcome, String> {
    let full = safe_join(vault, rel)?;
    // What the check saw: `Some(Some(bytes))` for a file it compared,
    // `Some(None)` for a missing one it accepted as "", `None` when there is
    // nothing to verify (a blind write).
    let checked: Option<Option<Vec<u8>>> = match expected {
        Expected::Anything => None,
        // Published with an exclusive hard link, so even a writer that takes
        // no Hollow lock (Obsidian, an agent's own tools) is never replaced.
        Expected::Absent => {
            return match publish_new(vault, rel, &full, content)? {
                Some(note) => Ok(CasOutcome::Written(note)),
                None => current_conflict(rel, &full),
            }
        }
        Expected::Content(text) => {
            let found = read_checked(&full)?;
            let bytes = found.as_deref().unwrap_or_default();
            if bytes != text.as_bytes() {
                return conflict_note(rel, &full, bytes.to_vec()).map(CasOutcome::Conflict);
            }
            Some(found)
        }
        Expected::Revision(wanted) => {
            let found = read_checked(&full)?;
            let bytes = found.as_deref().unwrap_or_default();
            if revision(bytes) != wanted {
                return conflict_note(rel, &full, bytes.to_vec()).map(CasOutcome::Conflict);
            }
            Some(found)
        }
    };
    let original_mtime = if keep_mtime {
        full.metadata().and_then(|meta| meta.modified()).ok()
    } else {
        None
    };
    #[cfg(test)]
    replace_hook("before-replace");
    match checked {
        None => atomic_write(&full, content.as_bytes())?,
        // Checked as missing: publish exclusively, so a file created since
        // is never replaced.
        Some(None) => {
            return match publish_new(vault, rel, &full, content)? {
                Some(note) => Ok(CasOutcome::Written(note)),
                None => current_conflict(rel, &full),
            }
        }
        Some(Some(seen)) => {
            if let Some(outcome) = replace_verified(rel, &full, content, &seen)? {
                return Ok(outcome);
            }
        }
    }
    if let Some(modified) = original_mtime {
        restore_mtime(&full, modified)?;
    }
    note_written(vault, rel, &full, content).map(CasOutcome::Written)
}

/// Replace `full` with `content` when it still holds `seen` (concurrency PR 2
/// review, 2026-10-20). The mutate lock keeps Hollow's own writers out, but
/// an agent's tools and Obsidian take no Hollow lock, so one can write
/// between the check and the replace. On macOS the replace is an atomic swap
/// and the displaced file is compared with `seen`: when it differs (or can't
/// be read), an outside write landed in the window, so it is swapped back
/// into place and the result is a conflict, never a silent overwrite.
///
/// A displaced file is unlinked only when its bytes were verified as `seen`
/// or as Hollow's own `content`; anything else (another writer's version, or
/// one that can't be read) is kept by renaming it to a conflict-copy name
/// (same inode, so its mode too), with the folder synced before and after.
///
/// What it catches: a replace-style write (write a temp file, rename it in;
/// Claude Code's Write, Obsidian) or an in-place write that completed in the
/// window. What it can't: an in-place writer that opened the old file before
/// the swap and writes after Hollow compared lands in the displaced inode
/// (delta review, 2026-10-20). `None` when the write went through. Off
/// macOS, or on a volume without the swap, it is a plain rename and the
/// window stays unverified.
fn replace_verified(
    rel: &str,
    full: &Path,
    content: &str,
    seen: &[u8],
) -> Result<Option<CasOutcome>, String> {
    if replace_seen(full, content.as_bytes(), seen, false)? {
        Ok(None)
    } else {
        current_conflict(rel, full).map(Some)
    }
}

/// [`replace_verified`] for any file: `true` when `content` replaced the
/// `seen` bytes, `false` when another writer's file was found instead (it
/// is left in place, or kept as a conflict copy when the swap had already
/// displaced it). `private` keeps the new file `0600`.
fn replace_seen(full: &Path, content: &[u8], seen: &[u8], private: bool) -> Result<bool, String> {
    let parent = full.parent().ok_or("the file has no parent folder")?;
    let (displaced, ours) = match install_temp(full, content, private, Durability::Full, true)? {
        Installed::Renamed => return Ok(true),
        Installed::TargetGone => return Ok(false),
        Installed::Swapped(displaced, ours) => (displaced, ours),
    };
    // Hollow's own file, recognised by its inode and its bytes read through
    // the handle (it may have inherited a mode Hollow can't open, delta
    // review 2): an in-place write into it since makes it not ours.
    let is_ours = |path: &Path| is_hollows_file(path, &ours, content);
    #[cfg(test)]
    replace_hook("after-swap");
    if fs::read(&displaced).is_ok_and(|bytes| bytes == seen) {
        // Verified: it was exactly what the check read.
        log_written(full, content);
        let _ = fs::remove_file(&displaced);
        flush_dir(parent);
        return Ok(true);
    }
    // An outside write landed in the window (or its file can't be read).
    invalidate_walk_cache();
    let ours_in_place = is_ours(full);
    #[cfg(test)]
    replace_hook("before-swap-back");
    if ours_in_place && swap_files(&displaced, full).is_ok() {
        flush_dir(parent);
    }
    // `displaced` now holds Hollow's own file (swapped back) or someone
    // else's: unlink it only when verified as ours.
    if is_ours(&displaced) {
        let _ = fs::remove_file(&displaced);
    } else if let Err(error) = preserve_as_conflict_copy(full, &displaced) {
        eprintln!(
            "[vault] {}: {error}; the replaced file is kept at {}",
            full.display(),
            displaced.display()
        );
    }
    flush_dir(parent);
    Ok(false)
}

/// What [`replace_if_unchanged`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Replace {
    /// The new bytes are in place.
    Done,
    /// The file no longer held the bytes the caller read, so nothing of
    /// the caller's was written: what is there now (`None`: no file).
    Changed(Option<Vec<u8>>),
}

/// The one verified compare-and-replace for vault files (2026-11-14,
/// findings Q-R26 to Q-R31): put `content` at `full` only while the file
/// still holds `seen` (`None`: no file yet). The new bytes go to a synced
/// sibling temp file first. A file expected absent is published with an
/// exclusive hard link ([`publish_exclusive`]), so nothing created meanwhile
/// is replaced. An existing one is re-read just before the publish and
/// installed with an atomic swap whose displaced bytes are compared with
/// `seen` ([`replace_seen`]): a write that lands in that window, by Hollow
/// or by a program that takes no Hollow lock (Obsidian, an agent's own
/// Edit tool), is swapped back and reported as [`Replace::Changed`], never
/// overwritten. Off macOS the swap is a plain rename, so only the re-read
/// guards the final window.
pub(crate) fn replace_if_unchanged(
    full: &Path,
    seen: Option<&[u8]>,
    content: &[u8],
    private: bool,
) -> Result<Replace, String> {
    let now = read_checked(full)?;
    if now.as_deref() != seen {
        return Ok(Replace::Changed(now));
    }
    #[cfg(test)]
    replace_hook("before-replace");
    let written = match seen {
        None => publish_exclusive(full, content, private)?,
        Some(seen) => replace_seen(full, content, seen, private)?,
    };
    if written {
        Ok(Replace::Done)
    } else {
        Ok(Replace::Changed(read_checked(full)?))
    }
}

/// Publish `content` at `full` only if nothing is there: a synced sibling
/// temp file, then an exclusive hard link to the final name, so a reader
/// never sees a torn file and a crash never leaves a half-written one under
/// the final name (finding Q-R31). `Ok(false)` when the name is taken.
pub(crate) fn publish_exclusive(
    full: &Path,
    content: &[u8],
    private: bool,
) -> Result<bool, String> {
    use std::io::Write;
    let parent = full.parent().ok_or("the file has no parent folder")?;
    fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    let seq = ATOMIC_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(
        ".hollow-publish-{}-{nanos}-{seq}.tmp",
        std::process::id()
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if private { 0o600 } else { 0o666 });
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options
        .open(&tmp)
        .map_err(|e| format!("create temporary file: {e}"))?;
    let result = (|| {
        file.write_all(content).map_err(|e| format!("write: {e}"))?;
        full_sync(&file).map_err(|e| format!("sync: {e}"))?;
        #[cfg(test)]
        replace_hook("before-publish");
        match fs::hard_link(&tmp, full) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(e) => return Err(format!("publish: {e}")),
        }
        log_written(full, content);
        Ok(true)
    })();
    drop(file);
    let _ = fs::remove_file(&tmp);
    if matches!(result, Ok(true)) {
        flush_dir(parent);
        invalidate_walk_cache();
    }
    result
}

/// What [`write_merging`] did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MergeWrite {
    /// The caller's text is in place as given.
    Written,
    /// Someone else changed the file meanwhile and the two edits touched
    /// different lines: `text` (both edits) is in place, over `found`, the
    /// other writer's version it was merged with.
    Merged { text: String, found: String },
    /// The edits overlap (or the file was removed or is not text): the file
    /// on disk is untouched and the caller's text is kept beside it as a
    /// conflict copy (`None` when even that could not be written).
    Conflict { copy: Option<PathBuf> },
}

impl MergeWrite {
    /// The text now on disk for the caller's write, or the error to show
    /// the person when the edits could not be merged.
    pub(crate) fn into_text(self, ours: &str, what: &str) -> Result<String, String> {
        match self {
            MergeWrite::Written => Ok(ours.to_string()),
            MergeWrite::Merged { text, .. } => Ok(text),
            MergeWrite::Conflict { copy } => Err(conflict_message(what, copy.as_deref())),
        }
    }
}

/// The message a write that couldn't merge shows the person.
pub(crate) fn conflict_message(what: &str, copy: Option<&Path>) -> String {
    match copy {
        Some(copy) => format!(
            "{what} was edited elsewhere at the same lines, so Hollow didn't overwrite it; Hollow's version is saved beside it as {}",
            copy.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ),
        None => format!(
            "{what} was edited elsewhere at the same lines, so Hollow didn't overwrite it, and its own version could not be saved"
        ),
    }
}

/// Write `ours` over a text file read as `base` (`None`: it didn't exist),
/// merging with any edit made since (2026-11-14, Alex: "the agent merges
/// both edits itself and tells him only when it can't; never a silent
/// overwrite"). Each attempt is a [`replace_if_unchanged`]; when the file
/// changed, a line-based three-way merge (`shared_docs::merge3`) of base,
/// the file now and ours is tried and written in turn. Overlapping edits leave the file
/// as it is and keep `ours` beside it as a conflict copy, which Waiting on
/// you offers.
pub(crate) fn write_merging(
    full: &Path,
    base: Option<&str>,
    ours: &str,
    private: bool,
) -> Result<MergeWrite, String> {
    let mut expect: Option<Vec<u8>> = base.map(|b| b.as_bytes().to_vec());
    let mut content = ours.to_string();
    // A busy file gets a few tries; each one merges against the newest copy.
    for _ in 0..4 {
        let theirs =
            match replace_if_unchanged(full, expect.as_deref(), content.as_bytes(), private)? {
                Replace::Done if content == ours => return Ok(MergeWrite::Written),
                Replace::Done => {
                    let found = String::from_utf8(expect.unwrap_or_default()).unwrap_or_default();
                    return Ok(MergeWrite::Merged {
                        text: content,
                        found,
                    });
                }
                Replace::Changed(now) => now,
            };
        let theirs = match theirs.map(String::from_utf8) {
            Some(Ok(text)) => text,
            // Removed, or no longer text: nothing to merge into.
            _ => break,
        };
        if theirs == ours {
            return Ok(MergeWrite::Written);
        }
        match crate::shared_docs::merge3(base.unwrap_or_default(), ours, &theirs) {
            Some(merged) if merged == theirs => {
                return Ok(MergeWrite::Merged {
                    text: merged,
                    found: theirs,
                })
            }
            Some(merged) => {
                content = merged;
                expect = Some(theirs.into_bytes());
            }
            None => break,
        }
    }
    let copy = keep_conflict_copy(full, ours.as_bytes())
        .map_err(|e| eprintln!("[vault] {}: {e}", full.display()))
        .ok();
    Ok(MergeWrite::Conflict { copy })
}

/// True when `path` is the file `ours` has open (same device and inode) and
/// it still holds exactly `content`.
fn is_hollows_file(path: &Path, ours: &fs::File, content: &[u8]) -> bool {
    use std::io::{Read, Seek};
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let (Ok(at), Ok(mine)) = (fs::symlink_metadata(path), ours.metadata()) else {
            return false;
        };
        if at.dev() != mine.dev() || at.ino() != mine.ino() {
            return false;
        }
    }
    let mut handle = ours;
    let mut bytes = Vec::new();
    handle.rewind().is_ok() && handle.read_to_end(&mut bytes).is_ok() && bytes == content
}

/// How a conflict's [`NoteContent::revision`] starts when the current file
/// can't be read or isn't UTF-8 text (delta review 2, 2026-10-20): its
/// `content` is empty, and no real revision (sha256 hex) matches it, so a
/// "keep mine" write passing it back is refused again rather than written.
pub(crate) const UNREADABLE_REVISION: &str = "unreadable:";

/// The conflict a checked write reports after it touched the file: the
/// current note, or, when it can't be read or isn't UTF-8 text, a note with
/// empty content and an [`UNREADABLE_REVISION`] revision naming the error.
/// Never an `Err`, so a caller takes it for the conflict it is.
fn current_conflict(rel: &str, full: &Path) -> Result<CasOutcome, String> {
    let (mtime, size) = full
        .metadata()
        .map(|m| (iso_mtime(&m), m.len()))
        .unwrap_or_default();
    let read = match fs::read(full) {
        Ok(bytes) => String::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.to_string()),
    };
    let (content, revision) = match read {
        Ok(text) => {
            let revision = revision(text.as_bytes());
            (text, revision)
        }
        Err(error) => (String::new(), format!("{UNREADABLE_REVISION} {error}")),
    };
    Ok(CasOutcome::Conflict(NoteContent {
        path: rel.to_string(),
        content,
        mtime,
        size,
        revision,
    }))
}

/// Give `displaced` (a sibling file) a conflict-copy name beside `full`
/// without reading it: an exclusive hard link to a free name, a folder sync,
/// and only then the old name's removal and another sync. The bytes and the
/// mode stay those of the file itself. On failure it stays where it was.
fn preserve_as_conflict_copy(full: &Path, displaced: &Path) -> Result<PathBuf, String> {
    let parent = full.parent().ok_or("the file has no parent folder")?;
    for _ in 0..8 {
        let copy = conflict_copy_path(full);
        match fs::hard_link(displaced, &copy) {
            Ok(()) => {
                flush_dir(parent);
                let _ = fs::remove_file(displaced);
                flush_dir(parent);
                invalidate_walk_cache();
                return Ok(copy);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("keep the replaced file: {e}")),
        }
    }
    Err("no free name for the conflict copy".into())
}

/// A fresh `<stem> (conflicted copy <stamp> <id>)<ext>` path beside `full`.
fn conflict_copy_path(full: &Path) -> PathBuf {
    let name = full
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "note.md".into());
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name.as_str(), ""),
    };
    let stamp = chrono::Local::now().format("%Y-%m-%d %H%M%S");
    let id: u32 = rand::random();
    full.with_file_name(format!("{stem} (conflicted copy {stamp} {id:08x}){ext}"))
}

/// Save `bytes` beside `full` as `<stem> (conflicted copy <stamp> <id>)<ext>`
/// (`conflict_copies` offers it in Waiting on you), created exclusively; the
/// random id keeps names free however many are made in one second. Returns
/// the copy's path.
pub(crate) fn keep_conflict_copy(full: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    let parent = full.parent().ok_or("the file has no parent folder")?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Never readable more widely than the original (0600 when unknown).
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode = fs::metadata(full)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        options.mode(mode);
        mode
    };
    for _ in 0..8 {
        let copy = conflict_copy_path(full);
        match options.open(&copy) {
            Ok(mut file) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(fs::Permissions::from_mode(mode))
                        .map_err(|e| format!("write conflict copy: {e}"))?;
                }
                file.write_all(bytes)
                    .and_then(|_| full_sync(&file))
                    .map_err(|e| format!("write conflict copy: {e}"))?;
                flush_dir(parent);
                invalidate_walk_cache();
                return Ok(copy);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("create conflict copy: {e}")),
        }
    }
    Err("no free name for the conflict copy".into())
}

/// A file's bytes before a checked write, `None` when it is missing.
fn read_checked(full: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(full) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("read before save: {error}")),
    }
}

#[cfg(test)]
thread_local! {
    /// Runs at named points of a checked write (tests only): `before-replace`
    /// (after the check, before the file is replaced) and `after-swap`.
    static REPLACE_HOOK: std::cell::RefCell<Option<Box<dyn FnMut(&str)>>> =
        std::cell::RefCell::new(None);
}

/// Whether `dir`'s volume can swap two files atomically (tests only: the
/// swap tests skip, with a message, where it can't).
#[cfg(test)]
pub(crate) fn swap_supported_for_test(dir: &Path) -> bool {
    let (a, b) = (dir.join(".swap-probe-a"), dir.join(".swap-probe-b"));
    // A setup failure is a broken test, not a missing capability.
    fs::write(&a, "a").expect("swap probe setup");
    fs::write(&b, "b").expect("swap probe setup");
    let swapped = swap_files(&a, &b);
    let _ = fs::remove_file(&a);
    let _ = fs::remove_file(&b);
    match swapped {
        Ok(()) => true,
        Err(error) if swap_unsupported(&error) => {
            eprintln!("skipped: this volume has no atomic swap (RENAME_SWAP): {error}");
            false
        }
        Err(error) => panic!("swap probe failed unexpectedly: {error}"),
    }
}

/// Install (or with `None` remove) this thread's checked-write hook.
#[cfg(test)]
pub(crate) fn set_replace_hook_for_test(hook: Option<Box<dyn FnMut(&str)>>) {
    REPLACE_HOOK.with(|slot| *slot.borrow_mut() = hook);
}

#[cfg(test)]
fn replace_hook(point: &str) {
    REPLACE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(point);
        }
    });
}

/// Set `full`'s modification time back to `modified` after a write.
fn restore_mtime(full: &Path, modified: SystemTime) -> Result<(), String> {
    fs::OpenOptions::new()
        .write(true)
        .open(full)
        .map_err(|e| format!("open to restore mtime: {e}"))?
        .set_times(fs::FileTimes::new().set_modified(modified))
        .map_err(|e| format!("restore mtime: {e}"))
}

/// The on-disk note a refused write reports.
fn conflict_note(rel: &str, full: &Path, bytes: Vec<u8>) -> Result<NoteContent, String> {
    let (mtime, size) = full
        .metadata()
        .map(|m| (iso_mtime(&m), m.len()))
        .unwrap_or_default();
    let content = String::from_utf8(bytes)
        .map_err(|_| "read before save: the file is not UTF-8 text".to_string())?;
    Ok(NoteContent {
        path: rel.to_string(),
        revision: revision(content.as_bytes()),
        content,
        mtime,
        size,
    })
}

/// Compare-and-swap write (data safety Tier 0.2): write `content` only if the
/// file's current on-disk content still equals `expected_current` (the baseline
/// the caller loaded). Otherwise write nothing and return `Conflict` with the
/// on-disk content — so an external edit (the same note touched in Obsidian)
/// is never silently clobbered by Hollow's debounced autosave. The read +
/// compare + atomic write run under the mutate lock ([`lock_mutate`]), so no
/// participating Hollow writer can slip between them. External editors do
/// not share this lock. A missing file reads as `""`, so a baseline of `""`
/// (a brand-new note) still matches and writes.
pub fn write_note_cas(
    vault: &Path,
    rel: &str,
    content: &str,
    expected_current: &str,
) -> Result<CasOutcome, String> {
    write_note_expecting(vault, rel, content, Expected::Content(expected_current))
}

/// Atomic read-modify-write of a note under the mutate lock (data safety Tier
/// 0.3): `transform` receives the file's current on-disk content and returns
/// the new content; the read, the transform, and the atomic write all run with
/// no other vault writer able to land between the read and the write (the
/// TOCTOU window the two-hop read-then-write had). Errors if the file is absent.
pub fn mutate_note(
    vault: &Path,
    rel: &str,
    transform: impl FnOnce(&str) -> Result<String, String>,
) -> Result<NoteContent, String> {
    mutate_note_inner(vault, rel, false, transform)
}

/// Same serialized, atomic mutation as [`mutate_note`], but restores the
/// note's original filesystem modification time before refreshing derived
/// indexes. Use only for metadata-only frontmatter writes whose mtime must
/// continue to represent the last content edit (for example an explicit
/// operator-profile review acknowledgement).
pub fn mutate_note_preserve_mtime(
    vault: &Path,
    rel: &str,
    transform: impl FnOnce(&str) -> Result<String, String>,
) -> Result<NoteContent, String> {
    mutate_note_inner(vault, rel, true, transform)
}

fn mutate_note_inner(
    vault: &Path,
    rel: &str,
    preserve_mtime: bool,
    transform: impl FnOnce(&str) -> Result<String, String>,
) -> Result<NoteContent, String> {
    let _guard = lock_mutate();
    let full = safe_join(vault, rel)?;
    let original_mtime = if preserve_mtime {
        Some(
            full.metadata()
                .and_then(|meta| meta.modified())
                .map_err(|e| format!("read mtime: {e}"))?,
        )
    } else {
        None
    };
    let current = fs::read_to_string(&full).map_err(|e| format!("read: {e}"))?;
    let next = transform(&current)?;
    // `lock_mutate` keeps Hollow's writers out, not Obsidian or an agent's
    // own tools: an edit saved while the transform ran is merged, and one
    // that touches the same lines is reported, never overwritten (Q-R26).
    let next = write_merging(&full, Some(&current), &next, false)?.into_text(&next, rel)?;
    if let Some(modified) = original_mtime {
        restore_mtime(&full, modified)?;
    }
    note_written(vault, rel, &full, &next)
}

/// A fresh path in the vault's `.trash/` for `full` (a file inside the
/// vault): `<nanos>-<seq>-<basename>`, suffixed if somehow taken.
fn trash_dest(vault: &Path, full: &Path) -> Result<PathBuf, String> {
    // Materialize the trash dir under the vault root (never via `rel`, which
    // is attacker-influenced — `full` is already inside the canonical vault).
    let trash_dir = safe_join(vault, ".trash")?;
    fs::create_dir_all(&trash_dir).map_err(|e| format!("mkdir trash: {e}"))?;

    // Unique prefix: nanoseconds since epoch + a monotonic sequence number.
    // Two deletes in the same nanosecond still get distinct names.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = TRASH_SEQ.fetch_add(1, Ordering::Relaxed);
    let basename = full
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "note.md".to_string());

    // Belt-and-suspenders: if a same-named trash file somehow exists, add a
    // numeric suffix before the extension until we find a free slot.
    let mut dest = trash_dir.join(format!("{nanos}-{seq}-{basename}"));
    if dest.exists() {
        let (stem, ext) = match basename.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (basename.clone(), String::new()),
        };
        let mut n = 1u32;
        loop {
            let candidate = trash_dir.join(format!("{nanos}-{seq}-{stem}-{n}{ext}"));
            if !candidate.exists() {
                dest = candidate;
                break;
            }
            n += 1;
        }
    }

    Ok(dest)
}

/// Replace `target_rel` with `source_rel` in one atomic rename, after
/// keeping the old target in `.trash/` (a hard link, else a copy): the
/// target's name is never missing. `check` sees both files' bytes (source,
/// target) under the edit lock and may refuse. Overnight A10's "keep the
/// copy".
pub(crate) fn replace_from(
    vault: &Path,
    source_rel: &str,
    target_rel: &str,
    check: impl FnOnce(&[u8], &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let _guard = lock_mutate();
    let src = safe_join(vault, source_rel)?;
    let dst = safe_join(vault, target_rel)?;
    let read = |p: &Path, rel: &str| fs::read(p).map_err(|e| format!("read {rel}: {e}"));
    let (source, target) = (read(&src, source_rel)?, read(&dst, target_rel)?);
    check(&source, &target)?;
    let kept = trash_dest(vault, &dst)?;
    if fs::hard_link(&dst, &kept).is_err() {
        fs::copy(&dst, &kept).map_err(|e| format!("trash: {e}"))?;
    }
    fs::rename(&src, &dst).map_err(|e| format!("rename: {e}"))?;
    log_write(&src);
    log_written(&dst, &source);
    invalidate_walk_cache();
    crate::vault_search_index::on_remove(vault, source_rel);
    crate::vault_embed_index::on_remove(vault, source_rel);
    let content = String::from_utf8_lossy(&source).into_owned();
    let meta = dst.metadata().ok();
    let mtime = meta.as_ref().map(iso_mtime).unwrap_or_default();
    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    crate::vault_search_index::on_write(vault, target_rel, &content, &mtime);
    crate::vault_embed_index::on_write(vault, target_rel, &content, &mtime, size);
    if let Ok(mut g) = INDEX.lock() {
        if let Some(idx) = g.as_mut() {
            if let Some(entry) = idx.by_path.remove(source_rel) {
                let key = entry.basename.to_lowercase();
                if idx.by_basename.get(&key).map(|s| s.as_str()) == Some(source_rel) {
                    idx.by_basename.remove(&key);
                }
            }
        }
    }
    refresh_index_for_file(target_rel, &content);
    Ok(())
}

/// Delete a note from the vault by its relative path. Rather than an
/// irrecoverable `fs::remove_file`, the note is MOVED into `vault/.trash/`
/// with a unique timestamp+sequence prefix so a mistaken delete can be
/// recovered from disk. Drops the entry from the link index too so
/// backlinks no longer resolve to a ghost.
pub fn delete_note(vault: &Path, rel: &str) -> Result<(), String> {
    trash_note(vault, rel).map(|_| ())
}

/// [`delete_note`], returning where the note went: its vault-relative path
/// under `.trash/`, so the Library's Undo can move it back (P1-18).
pub fn trash_note(vault: &Path, rel: &str) -> Result<String, String> {
    let full = safe_join(vault, rel)?;
    if !full.exists() {
        return Err(format!("not found: {rel}"));
    }

    let dest = trash_dest(vault, &full)?;
    let trashed = format!(
        ".trash/{}",
        dest.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    );
    fs::rename(&full, &dest).map_err(|e| format!("trash: {e}"))?;
    log_write(&full);
    invalidate_walk_cache();
    crate::vault_search_index::on_remove(vault, rel);
    crate::vault_embed_index::on_remove(vault, rel);
    // Best-effort index update; a poisoned lock must not abort the process
    // (the file is already moved — the worst case is a stale link entry).
    // `remove_entry` also takes its links out of the targets' backlinks.
    if let Ok(mut g) = INDEX.lock() {
        if let Some(idx) = g.as_mut() {
            remove_entry(idx, rel);
        }
    }
    Ok(trashed)
}

/// Reveal a file in the macOS Finder. Best-effort — on non-macOS this is
/// a no-op that returns Ok so callers don't blow up.
pub fn reveal_in_finder(vault: &Path, rel: &str) -> Result<(), String> {
    let full = safe_join(vault, rel)?;
    if !full.exists() {
        return Err(format!("not found: {rel}"));
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(&full)
            .spawn()
            .map_err(|e| format!("open -R: {e}"))?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = full;
    }
    Ok(())
}

/// Rename `from` to `to` only if nothing exists at `to`, as one atomic step:
/// an existing target (a file, a folder, even a dangling symlink) is refused
/// with [`std::io::ErrorKind::AlreadyExists`] and both paths are left as
/// they were. Never check `exists()` and then `fs::rename`: a file created
/// between the two (Obsidian, an agent, another Hollow process) would be
/// silently replaced (merged plan slice 4, review finding A·D8).
///
/// macOS uses `renamex_np(RENAME_EXCL)`. Where the filesystem refuses that
/// flag, and on other platforms, a file moves by `link` + `unlink` (the
/// link refuses an existing target); a folder can't be hard-linked, so
/// there it fails with the link's error rather than risk a replace.
pub fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    return rename_logged(from, to, || {});
    #[cfg(not(test))]
    rename_logged(from, to)
}

/// [`rename_no_replace`]; in tests `after_move` runs between the move and
/// the write log note (a seam for a write landing there). Production code
/// has no such parameter.
fn rename_logged(
    from: &Path,
    to: &Path,
    #[cfg(test)] after_move: impl FnOnce(),
) -> std::io::Result<()> {
    // The bytes it moves, read from the source before the move, so a write
    // to `to` after it isn't credited as this one (review of #1399,
    // 2026-11-13). Only while a log is open; a folder or an unreadable
    // file: none.
    let hash = current_write_log().and_then(|_| {
        fs::symlink_metadata(from)
            .ok()
            .filter(|meta| meta.is_file())
            .and_then(|_| fs::read(from).ok())
            .map(|bytes| revision(&bytes))
    });
    let moved = rename_no_replace_inner(from, to);
    if moved.is_ok() {
        #[cfg(test)]
        after_move();
        log_write(from);
        log_write_hashed(to, || hash);
    }
    moved
}

fn rename_no_replace_inner(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        // A NUL byte in a path becomes an `InvalidInput` error here.
        let c_from = CString::new(from.as_os_str().as_bytes())?;
        let c_to = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers come from CStrings that outlive the call, and
        // renamex_np only reads them.
        if unsafe { libc::renamex_np(c_from.as_ptr(), c_to.as_ptr(), libc::RENAME_EXCL) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) {
            return Err(error);
        }
    }
    link_then_unlink(from, to)
}

/// The portable no-replace move for a file: hard-link `to` (refused if it
/// exists), then remove `from`. If `from` can't be removed the link is
/// taken back, so a failure never leaves the file under two names.
fn link_then_unlink(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::hard_link(from, to)?;
    if let Err(error) = fs::remove_file(from) {
        let _ = fs::remove_file(to);
        return Err(error);
    }
    Ok(())
}

/// Move/rename a note within the vault. Both paths are vault-relative.
/// Refuses to overwrite an existing file, atomically ([`rename_no_replace`]),
/// under the mutate lock ([`lock_mutate`]) so it never lands between another
/// Hollow writer's read and write. Refreshes the link index for the moved
/// file so backlinks keep resolving.
pub fn move_note(vault: &Path, from_rel: &str, to_rel: &str) -> Result<NoteContent, String> {
    if from_rel == to_rel {
        return read_note(vault, from_rel);
    }
    let from = safe_join(vault, from_rel)?;
    // A rename that changes only letter case (`plan.md` → `Plan.md`) names
    // the same file on a case-insensitive volume: `safe_join` would resolve
    // `to` to the existing spelling and the exclusive rename would refuse it.
    // Keep the typed name and go through a hidden name in the same folder
    // (P1-18 review). Only the file name may change case: a folder spelled
    // differently would leave the file where it is under a path the walk
    // never produces, so that is refused.
    let case_only = from_rel.to_lowercase() == to_rel.to_lowercase();
    let parent_of = |rel: &str| rel.rsplit_once('/').map(|(p, _)| p.to_string());
    if case_only && parent_of(from_rel) != parent_of(to_rel) {
        return Err(format!(
            "only a folder's letter case differs: {to_rel}; rename the folder itself"
        ));
    }
    let to = match Path::new(to_rel).file_name() {
        Some(name) if case_only => from.with_file_name(name),
        _ => safe_join(vault, to_rel)?,
    };
    let _guard = lock_mutate();
    if !from.exists() {
        return Err(format!("source not found: {from_rel}"));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    #[cfg(test)]
    run_before_move_rename(&to);
    let refused = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!("destination exists: {to_rel}")
        } else {
            format!("rename: {e}")
        }
    };
    if case_only {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let hidden = from.with_file_name(format!(
            ".{}.{}-{nanos}.hollow-rename",
            from.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id()
        ));
        rename_no_replace(&from, &hidden).map_err(|e| format!("rename: {e}"))?;
        if let Err(e) = rename_no_replace(&hidden, &to) {
            if let Err(back) = rename_no_replace(&hidden, &from) {
                return Err(format!(
                    "rename: {e}; the note is kept at {} ({back})",
                    hidden.display()
                ));
            }
            return Err(refused(e));
        }
    } else {
        rename_no_replace(&from, &to).map_err(refused)?;
    }
    invalidate_walk_cache();
    crate::vault_search_index::on_remove(vault, from_rel);
    crate::vault_embed_index::on_remove(vault, from_rel);
    if let Ok(content) = fs::read_to_string(&to) {
        let meta = to.metadata().ok();
        let mtime = meta.as_ref().map(iso_mtime).unwrap_or_default();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        crate::vault_search_index::on_write(vault, to_rel, &content, &mtime);
        crate::vault_embed_index::on_write(vault, to_rel, &content, &mtime, size);
    }

    // Best-effort — a poisoned lock degrades to a stale entry, never a panic.
    if let Ok(mut g) = INDEX.lock() {
        if let Some(idx) = g.as_mut() {
            let moved = fs::read_to_string(&to).ok();
            reindex_moved(idx, from_rel, to_rel, moved.as_deref());
        }
    }

    let content = fs::read_to_string(&to).unwrap_or_default();
    let meta = to.metadata().map_err(|e| format!("stat: {e}"))?;
    Ok(NoteContent {
        path: to_rel.to_string(),
        revision: revision(content.as_bytes()),
        content,
        mtime: iso_mtime(&meta),
        size: meta.len(),
    })
}

// Test seam: runs once, just before `move_note` renames, so a test can put a
// file at the destination after every check has passed (the race the
// no-replace rename closes).
#[cfg(test)]
thread_local! {
    static BEFORE_MOVE_RENAME: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_before_move_rename(to: &Path) {
    if let Some(hook) = BEFORE_MOVE_RENAME.with(|h| h.borrow_mut().take()) {
        hook(to);
    }
}

#[cfg(test)]
thread_local! {
    /// `safe_join` calls on this thread (each costs two `realpath`s), for tests.
    pub(crate) static SAFE_JOINS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Resolve a vault-relative path inside the vault root, rejecting any
/// component that would escape it (parent dirs, absolute paths) and any
/// symlink that points outside. Public so sibling modules (email, etc.)
/// can route their own file reads/writes through the same hardening.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    use std::path::Component;
    #[cfg(test)]
    SAFE_JOINS.with(|n| n.set(n.get() + 1));

    let canon_root = root
        .canonicalize()
        .map_err(|e| format!("canonicalize vault root: {e}"))?;

    let mut out = canon_root.clone();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("path escape: {rel}"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("absolute path not allowed: {rel}"));
            }
        }
    }

    // If the path exists, canonicalize it and re-verify (defeats symlinks
    // pointing outside the vault). For not-yet-existing paths (creates),
    // canonicalize the deepest existing ancestor instead.
    if let Ok(canon) = out.canonicalize() {
        if !canon.starts_with(&canon_root) {
            return Err(format!("path escape via symlink: {rel}"));
        }
        return Ok(canon);
    }

    let mut probe = out.clone();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    if let Ok(canon_probe) = probe.canonicalize() {
        if !canon_probe.starts_with(&canon_root) {
            return Err(format!("path escape via symlink: {rel}"));
        }
    }

    Ok(out)
}

pub fn parse_wikilinks(content: &str) -> Vec<String> {
    let mut set = HashSet::new();
    // A hand scan with exactly WIKILINK_RE's matches (pinned by a randomized
    // comparison test): the regex's lazy capture was a third of a cold
    // link-index build (overnight A14).
    let b = content.as_bytes();
    let mut i = 0;
    while let Some(off) = content[i..].find("[[") {
        let at = i + off;
        match wikilink_at(b, at) {
            Some((target, end)) => {
                set.insert(content[target].trim().to_string());
                i = end;
            }
            None => i = at + 1,
        }
    }
    set.into_iter().collect()
}

/// The `[[target|alias#heading]]` starting at `at` (where `b` holds `[[`), as
/// WIKILINK_RE matches it: the target's byte range and where the match ends.
/// The target runs to the first `]`, newline, `|` or `#` and is never empty;
/// an alias (`|`, then anything but `]` or a newline) or a heading (`#`, the
/// same) must be non-empty; then `]]`.
fn wikilink_at(b: &[u8], at: usize) -> Option<(std::ops::Range<usize>, usize)> {
    let start = at + 2;
    let stop = |from: usize| {
        b[from..]
            .iter()
            .position(|&c| c == b']' || c == b'\n')
            .map(|p| from + p)
    };
    let j = start
        + b[start..]
            .iter()
            .position(|&c| matches!(c, b']' | b'\n' | b'|' | b'#'))?;
    if j == start {
        return None;
    }
    let close =
        |m: usize| (b.get(m) == Some(&b']') && b.get(m + 1) == Some(&b']')).then_some(m + 2);
    let end = match b[j] {
        b']' => close(j)?,
        b'|' | b'#' => {
            let m = stop(j + 1)?;
            if m == j + 1 {
                return None;
            }
            close(m)?
        }
        _ => return None,
    };
    Some((start..j, end))
}

/// A note's display title and one-line summary, read from its text.
///
/// Title: frontmatter `title`, else the first `# heading`. A heading that
/// repeats the file name as an ID prefix (`Q0c68d734b62a — Shift Log
/// update`) keeps only the part after it. Summary: the first sentence of
/// the first prose line after the heading, skipping navigation links,
/// lists, tables, quotes and italic date stamps.
pub fn note_title_and_summary(content: &str, basename: &str) -> (Option<String>, Option<String>) {
    let (fm, body) = crate::frontmatter::parse_map(content);
    let mut title = fm
        .get("title")
        .map(|t| {
            t.trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .trim()
                .to_string()
        })
        .filter(|t| !t.is_empty());
    let lines: Vec<&str> = body.lines().collect();
    let heading = lines.iter().position(|l| l.starts_with("# "));
    if title.is_none() {
        title = heading.map(|i| plain_inline(lines[i][2..].trim_end_matches('#')));
    }
    let title = title
        .map(|t| strip_id_prefix(&t, basename))
        .filter(|t| !t.is_empty() && t != basename);
    let start = heading.map(|i| i + 1).unwrap_or(0);
    let summary = lines[start..]
        .iter()
        .map(|l| l.trim())
        .find(|l| is_prose_line(l))
        .map(|l| first_sentence(&plain_inline(l)))
        .filter(|s| !s.is_empty());
    (title, summary)
}

/// HTML notes keep their file name: their first "prose" line is markup.
fn markdown_title(rel: &str, content: &str, basename: &str) -> (Option<String>, Option<String>) {
    if rel.ends_with(".md") {
        note_title_and_summary(content, basename)
    } else {
        (None, None)
    }
}

fn strip_id_prefix(title: &str, basename: &str) -> String {
    let t = title.trim();
    // Only a separated prefix: `Hollow 3.0 plan` in `Hollow.md` stays whole.
    if let Some(rest) = t.strip_prefix(basename).map(str::trim_start) {
        if rest.starts_with(['\u{2014}', '\u{2013}', '-', ':', '·']) {
            let rest = rest
                .trim_start_matches(['\u{2014}', '\u{2013}', '-', ':', '·'])
                .trim();
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    t.to_string()
}

fn is_prose_line(l: &str) -> bool {
    if l.is_empty() || l.starts_with(['#', '-', '+', '|', '>', '!', '<', '`', '[', '=', '~']) {
        return false;
    }
    // `*As of 2026-09-09*` is a stamp; `**Name** (role)…` is prose.
    if l.starts_with('*') && !l.starts_with("**") {
        return false;
    }
    let digits = l.chars().take_while(|c| c.is_ascii_digit()).count();
    !(digits > 0 && l[digits..].starts_with(". "))
}

static MD_LINK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([^\]]*)\]\([^)]*\)").unwrap());
static WIKI_ALIAS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[\[(?:[^\]|]*\|)?([^\]]*)\]\]").unwrap());

/// Markdown inline syntax reduced to its visible text.
fn plain_inline(s: &str) -> String {
    let s = WIKI_ALIAS_RE.replace_all(s, "$1");
    let s = MD_LINK_RE.replace_all(&s, "$1");
    s.replace("**", "").replace('`', "").trim().to_string()
}

fn first_sentence(s: &str) -> String {
    let end = s
        .char_indices()
        .find(|&(i, c)| matches!(c, '.' | '!' | '?') && s[i + 1..].starts_with(' '))
        .map(|(i, _)| i + 1)
        .unwrap_or(s.len());
    let one = &s[..end];
    if one.chars().count() <= 160 {
        return one.to_string();
    }
    let cut: String = one.chars().take(157).collect();
    format!("{}…", cut.trim_end())
}

pub fn build_index(vault: &Path) -> LinkIndex {
    let notes = walk_notes(vault);
    let mut by_path: HashMap<String, NoteIndex> = HashMap::new();
    let mut by_basename: HashMap<String, String> = HashMap::new();
    // Read and parse each note on a few threads (overnight A14: one thread
    // took 1.6 s for 30,000 notes at launch); chunks keep the vault's order,
    // so the first note with a basename still wins it below.
    let parse = |note: &Note| {
        // Strip whatever extension the note has (.md / .html / .htm) so a
        // `[[wikilink]]` resolves to it and its basename displays cleanly.
        let basename = Path::new(&note.name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| note.name.clone());
        let content = fs::read_to_string(vault.join(&note.path)).unwrap_or_default();
        let outgoing = parse_wikilinks(&content);
        let (title, summary) = markdown_title(&note.path, &content, &basename);
        (basename, outgoing, title, summary)
    };
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, INDEX_THREADS);
    let chunk = notes.len().div_ceil(threads).max(1);
    let parsed: Vec<(String, Vec<String>, Option<String>, Option<String>)> =
        if threads == 1 || notes.len() < 256 {
            notes.iter().map(parse).collect()
        } else {
            #[cfg(test)]
            INDEX_THREADED_BUILDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::thread::scope(|scope| {
                let handles: Vec<_> = notes
                    .chunks(chunk)
                    .map(|part| scope.spawn(move || part.iter().map(parse).collect::<Vec<_>>()))
                    .collect();
                handles
                    .into_iter()
                    // A panic propagates (as with one thread): dropping a
                    // chunk would misalign every later note.
                    .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                    .collect()
            })
        };
    for (note, (basename, outgoing, title, summary)) in notes.iter().zip(parsed) {
        by_path.insert(
            note.path.clone(),
            NoteIndex {
                basename: basename.clone(),
                title,
                summary,
                outgoing,
                incoming_from: Vec::new(),
            },
        );
        by_basename
            .entry(basename.to_lowercase())
            .or_insert_with(|| note.path.clone());
    }
    // Build incoming
    let outgoing_clone: HashMap<String, Vec<String>> = by_path
        .iter()
        .map(|(k, v)| (k.clone(), v.outgoing.clone()))
        .collect();
    for (from_path, outs) in outgoing_clone {
        for target in outs {
            if let Some(target_path) = by_basename.get(&target.to_lowercase()) {
                if let Some(entry) = by_path.get_mut(target_path) {
                    entry.incoming_from.push(from_path.clone());
                }
            }
        }
    }
    LinkIndex {
        by_path,
        by_basename,
    }
}

pub fn ensure_index(vault: &Path) {
    let Ok(mut g) = INDEX.lock() else { return };
    if g.is_none() {
        *g = Some(build_index(vault));
    }
}

/// Drop the cached link index so the next `ensure_index` rebuilds it from
/// disk. The vault watcher calls this on any external change (Obsidian, the
/// embedded agent, a sync) so backlinks, the graph, and wikilink autocomplete
/// don't serve stale data after a write Hollow didn't make through `write_note`.
pub fn invalidate_index() {
    if let Ok(mut g) = INDEX.lock() {
        *g = None;
    }
}

pub fn refresh_index_for_file(rel: &str, content: &str) {
    let Ok(mut g) = INDEX.lock() else { return };
    let Some(idx) = g.as_mut() else { return };
    refresh_entry(idx, rel, content);
}

/// Patch one note's entries inside a LinkIndex: replace its outgoing list,
/// fix the targets' incoming lists, and — for a note the index hasn't seen
/// yet — resolve previously-dangling `[[links]]` from other notes into its
/// incoming list (a new file can make older wikilinks suddenly resolve).
fn refresh_entry(idx: &mut LinkIndex, rel: &str, content: &str) {
    let basename = Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let old_outgoing = idx
        .by_path
        .get(rel)
        .map(|n| n.outgoing.clone())
        .unwrap_or_default();
    let new_outgoing = parse_wikilinks(content);

    // Drop incoming references from old outgoing
    for t in &old_outgoing {
        if let Some(tp) = idx.by_basename.get(&t.to_lowercase()).cloned() {
            if let Some(entry) = idx.by_path.get_mut(&tp) {
                entry.incoming_from.retain(|p| p != rel);
            }
        }
    }

    // A brand-new entry may resolve wikilinks other notes already carry.
    let incoming_for_new: Vec<String> = if idx.by_path.contains_key(rel) {
        Vec::new()
    } else {
        let key = basename.to_lowercase();
        idx.by_path
            .iter()
            .filter(|(p, n)| {
                p.as_str() != rel && n.outgoing.iter().any(|t| t.to_lowercase() == key)
            })
            .map(|(p, _)| p.clone())
            .collect()
    };

    let entry = idx
        .by_path
        .entry(rel.to_string())
        .or_insert_with(|| NoteIndex {
            basename: basename.clone(),
            title: None,
            summary: None,
            outgoing: Vec::new(),
            incoming_from: incoming_for_new,
        });
    entry.outgoing = new_outgoing.clone();
    (entry.title, entry.summary) = markdown_title(rel, content, &basename);
    idx.by_basename
        .entry(basename.to_lowercase())
        .or_insert_with(|| rel.to_string());

    for t in &new_outgoing {
        if let Some(tp) = idx.by_basename.get(&t.to_lowercase()).cloned() {
            if let Some(entry) = idx.by_path.get_mut(&tp) {
                if !entry.incoming_from.iter().any(|p| p == rel) {
                    entry.incoming_from.push(rel.to_string());
                }
            }
        }
    }
}

/// The link index after [`move_note`]: the old path leaves with its links and
/// its name, and the new path is read in as the walk would see it (`content`
/// is its text), so a rename resolves `[[links]]` by the new name only, and a
/// note moved back from `.trash/` (Library's Undo, P1-18) is listed again.
fn reindex_moved(idx: &mut LinkIndex, from_rel: &str, to_rel: &str, content: Option<&str>) {
    remove_entry(idx, from_rel);
    let indexed = [".md", ".html", ".htm"]
        .iter()
        .any(|ext| to_rel.ends_with(ext));
    if let Some(content) = content.filter(|_| indexed && !walk_skips(to_rel)) {
        refresh_entry(idx, to_rel, content);
    }
}

/// Remove one note from a LinkIndex: drop its entry, unhook its outgoing
/// contributions from the targets' incoming lists, and release its basename
/// mapping (`trash_note`, `reindex_moved` and the watcher's patch use it).
fn remove_entry(idx: &mut LinkIndex, rel: &str) {
    let Some(entry) = idx.by_path.remove(rel) else {
        return;
    };
    for t in &entry.outgoing {
        if let Some(tp) = idx.by_basename.get(&t.to_lowercase()).cloned() {
            if let Some(te) = idx.by_path.get_mut(&tp) {
                te.incoming_from.retain(|p| p != rel);
            }
        }
    }
    let key = entry.basename.to_lowercase();
    if idx.by_basename.get(&key).map(|s| s.as_str()) == Some(rel) {
        idx.by_basename.remove(&key);
    }
}

/// Folder listings for one watcher batch: each folder's names, read once
/// (`None` when the folder can't be read).
type Listings = HashMap<PathBuf, Option<HashSet<std::ffi::OsString>>>;

/// Whether `full` is a file spelled exactly as given. macOS volumes ignore
/// case, so after `plan.md` → `Plan.md` the old spelling still `is_file()`;
/// the watcher must treat it as gone, or the index lists both (P1-18
/// review). An unreadable folder counts the file as present, as before.
fn is_file_as_named(full: &Path, listings: &mut Listings) -> bool {
    if !full.is_file() {
        return false;
    }
    let (Some(parent), Some(name)) = (full.parent(), full.file_name()) else {
        return false;
    };
    listings
        .entry(parent.to_path_buf())
        .or_insert_with(|| {
            fs::read_dir(parent).ok().map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name())
                    .collect()
            })
        })
        .as_ref()
        .is_none_or(|names| names.contains(name))
}

/// Incrementally patch the cached link index for a burst of changed paths —
/// the vault watcher calls this instead of a blanket `invalidate_index` so
/// one saved note doesn't force the next graph / backlinks / autocomplete
/// call to re-read the whole vault. No-op while the index hasn't been built
/// (it stays lazy). Falls back to a full invalidate for large bursts, where
/// per-path patching approaches rebuild cost anyway — this also covers the
/// watcher's capped path list (a truncated burst must not be trusted as a
/// complete change set).
pub fn update_index_paths(vault: &Path, paths: &[String]) {
    const INCREMENTAL_MAX: usize = 50;
    if paths.len() > INCREMENTAL_MAX {
        invalidate_index();
        return;
    }
    let Ok(mut g) = INDEX.lock() else { return };
    let Some(idx) = g.as_mut() else { return };
    apply_index_paths(idx, vault, paths);
}

/// [`update_index_paths`]' patch. Paths that are gone (deleted, or renamed
/// away) leave before present ones are read in, so a rename's two paths
/// land right in either order: the watcher sorts `Plan.md` before `plan.md`
/// (P1-18 review).
fn apply_index_paths(idx: &mut LinkIndex, vault: &Path, paths: &[String]) {
    let mut listings = Listings::new();
    let (present, gone): (Vec<&String>, Vec<&String>) = paths
        .iter()
        .filter(|rel| {
            let lower = rel.to_lowercase();
            lower.ends_with(".md") || lower.ends_with(".html") || lower.ends_with(".htm")
        })
        .partition(|rel| is_file_as_named(&vault.join(rel), &mut listings));
    for rel in gone {
        remove_entry(idx, rel);
    }
    for rel in present {
        if let Ok(content) = fs::read_to_string(vault.join(rel)) {
            refresh_entry(idx, rel, &content);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    /// Vault-relative path (e.g. `projects/notes.md`) — stable id used
    /// by edges to point back. Frontend opens this via the existing
    /// `requestOpenVaultNote` flow on click.
    pub id: String,
    pub label: String,
    /// Top-level folder (or empty string if the note lives at the
    /// vault root). UI groups nodes by this for color-coding.
    pub folder: String,
    /// Semantic kind (folder-derived; see `vault_index::kind_from_folder`).
    /// Lets the graph hide the high-volume synced mirrors (289 calendar / 216
    /// email nodes) so the structurally interesting knowledge graph is legible.
    pub kind: String,
    /// Number of incoming wikilinks (`[[ThisNote]]` from elsewhere).
    pub in_degree: usize,
    /// Number of resolved outgoing wikilinks (unresolved ones aren't
    /// counted since they don't end up as edges).
    pub out_degree: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

/// Snapshot of the vault as a wikilink graph — every `.md` is a node,
/// every resolved `[[Other]]` is a directed edge. Built from the existing
/// `LinkIndex` so it's cheap; edges to unresolved targets are dropped so
/// the frontend doesn't have to render ghost nodes.
pub fn build_graph(vault: &Path) -> VaultGraph {
    ensure_index(vault);
    let Ok(g) = INDEX.lock() else {
        return VaultGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
    };
    let Some(idx) = g.as_ref() else {
        return VaultGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
        };
    };

    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut out_degree: HashMap<String, usize> = HashMap::new();

    for (from_path, meta) in &idx.by_path {
        for target in &meta.outgoing {
            if let Some(target_path) = idx.by_basename.get(&target.to_lowercase()) {
                // Self-links land in our outgoing list — skip them so
                // the visual doesn't have a loop on a single node.
                if target_path == from_path {
                    continue;
                }
                edges.push(GraphEdge {
                    source: from_path.clone(),
                    target: target_path.clone(),
                });
                *out_degree.entry(from_path.clone()).or_insert(0) += 1;
                *in_degree.entry(target_path.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut nodes: Vec<GraphNode> = idx
        .by_path
        .iter()
        .map(|(p, d)| {
            // Top-level folder name, or empty for root-level notes.
            let folder = p.find('/').map(|i| p[..i].to_string()).unwrap_or_default();
            // Root-level / unknown-folder files have no folder kind; default to
            // `unfiled` to match `vault_index::classify` (which, absent a
            // recognizing frontmatter `type`, also lands on unfiled) so a node's
            // kind is consistent between the graph and the semantic index/lenses.
            let kind = crate::vault_index::kind_from_folder(&folder)
                .unwrap_or(crate::vault_index::KIND_UNFILED)
                .to_string();
            GraphNode {
                id: p.clone(),
                label: d.basename.clone(),
                folder,
                kind,
                in_degree: in_degree.get(p).copied().unwrap_or(0),
                out_degree: out_degree.get(p).copied().unwrap_or(0),
            }
        })
        .collect();

    // Stable order: most-connected first so frontend can cap to top-N
    // for very large vaults without losing the structurally important nodes.
    nodes.sort_by_key(|n| std::cmp::Reverse(n.in_degree + n.out_degree));
    VaultGraph { nodes, edges }
}

pub fn notes_list(vault: &Path) -> Vec<NoteRef> {
    ensure_index(vault);
    let Ok(g) = INDEX.lock() else {
        return Vec::new();
    };
    let Some(idx) = g.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<NoteRef> = idx
        .by_path
        .iter()
        .map(|(p, d)| NoteRef {
            path: p.clone(),
            basename: d.basename.clone(),
            title: d.title.clone(),
            summary: d.summary.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.basename.cmp(&b.basename));
    out
}

pub fn backlinks(vault: &Path, rel: &str) -> Vec<BacklinkSource> {
    ensure_index(vault);
    let Ok(g) = INDEX.lock() else {
        return Vec::new();
    };
    let Some(idx) = g.as_ref() else {
        return Vec::new();
    };
    let Some(meta) = idx.by_path.get(rel) else {
        return Vec::new();
    };
    let target_basename = meta.basename.clone();
    let pattern = format!(
        r"\[\[{}(?:\|[^\]]*)?(?:#[^\]]*)?\]\]",
        regex::escape(&target_basename),
    );
    let re = Regex::new(&pattern).unwrap();
    let mut out = Vec::new();
    for from_path in &meta.incoming_from {
        let content = match fs::read_to_string(vault.join(from_path)) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut matches = Vec::new();
        for (i, line) in content.lines().enumerate() {
            if re.is_match(line) {
                matches.push(BacklinkMatch {
                    line: (i + 1) as u32,
                    text: line.trim().to_string(),
                });
            }
        }
        if !matches.is_empty() {
            let basename = idx
                .by_path
                .get(from_path)
                .map(|n| n.basename.clone())
                .unwrap_or_else(|| from_path.clone());
            out.push(BacklinkSource {
                path: from_path.clone(),
                basename,
                matches,
            });
        }
    }
    out
}

/// Snap a byte index down to the nearest UTF-8 char boundary so slicing a
/// `&str` at that position never panics. Search snippets locate terms in
/// lowercased text and apply the offset to the original, so the index can land
/// mid-char.
pub(crate) fn char_floor(s: &str, mut idx: usize) -> usize {
    if idx > s.len() {
        idx = s.len();
    }
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Snap a byte index up to the nearest UTF-8 char boundary.
pub(crate) fn char_ceil(s: &str, mut idx: usize) -> usize {
    let len = s.len();
    if idx > len {
        idx = len;
    }
    while idx < len && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

#[cfg(test)]
mod tests {

    /// F1 delta review (red first): a write log is taken off its thread even
    /// when the operation panics, lists each file once, and nests.
    #[test]
    fn a_write_log_is_unwind_safe_deduplicated_and_nested() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.md");
        let panicked = std::panic::catch_unwind(|| {
            super::logging_writes(|| {
                super::atomic_write(&a, b"1").unwrap();
                panic!("the operation failed");
            })
        });
        assert!(panicked.is_err());
        assert!(
            super::current_write_log().is_none(),
            "still installed after the panic"
        );
        let ((_, inner), outer) = super::logging_writes(|| {
            super::atomic_write(&a, b"2").unwrap();
            super::logging_writes(|| {
                super::atomic_write(&a, b"3").unwrap();
                super::atomic_write(&a, b"4").unwrap();
            })
        });
        // 2026-11-13: each file once, with the hash of its latest write.
        let latest = Some(super::revision(b"4"));
        assert_eq!(
            inner.into_iter().collect::<Vec<_>>(),
            [(a.clone(), latest.clone())]
        );
        assert_eq!(outer.into_iter().collect::<Vec<_>>(), [(a, latest)]);
    }

    #[test]
    fn recent_notes_filter_connector_noise_before_limiting() {
        let dir = tempfile::Builder::new()
            .prefix("hollow-recent-")
            .tempdir()
            .unwrap();
        create_note(dir.path(), "notes/My thinking.md", "# A thought").unwrap();
        create_note(dir.path(), "emails/inbox/New mail.md", "# Mail").unwrap();
        create_note(dir.path(), "calendar/Meeting.md", "# Meeting").unwrap();
        let notes = recent(dir.path(), 1);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].path, "notes/My thinking.md");
    }
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn note_titles_come_from_the_text_not_the_file_name() {
        let ev = "[Library home](../../../README.md) · [Reading notes](../../source-notes.md)\n\n# Q0c68d734b62a — Shift Log Dashboard update\n\nUntrusted email evidence. Attachments and images unread.\n";
        assert_eq!(
            note_title_and_summary(ev, "Q0c68d734b62a"),
            (
                Some("Shift Log Dashboard update".into()),
                Some("Untrusted email evidence.".into())
            )
        );
        let person = "[Library home](../README.md) › [People](../people-directory.md)\n\n# Apex Partners\n\n*External · As of 2026-09-11*\n\n**Apex Partners** (Advisory, Denver): [Morgan Cho](morgan.md), Principal.\n";
        assert_eq!(
            note_title_and_summary(person, "apex-partners"),
            (
                Some("Apex Partners".into()),
                Some("Apex Partners (Advisory, Denver): Morgan Cho, Principal.".into())
            )
        );
        let fm = "---\ntitle: \"Quarter review\"\n---\n\n# Something else\n\n- a list\n1. step\nChecked local draft. More.\n";
        assert_eq!(
            note_title_and_summary(fm, "q"),
            (
                Some("Quarter review".into()),
                Some("Checked local draft.".into())
            )
        );
        // A heading that only repeats the file name, or extends it without a
        // separator, is not an ID prefix.
        assert_eq!(note_title_and_summary("# Atlas\n", "Atlas"), (None, None));
        assert_eq!(
            note_title_and_summary("# Hollow 3.0 plan\n", "Hollow").0,
            Some("Hollow 3.0 plan".into())
        );
        assert_eq!(
            note_title_and_summary("no heading here\n", "x"),
            (None, Some("no heading here".into()))
        );
        let long = format!("# T\n\n{}\n", "word ".repeat(60));
        assert!(note_title_and_summary(&long, "t").1.unwrap().ends_with('…'));
    }

    #[test]
    fn link_index_carries_titles_and_refreshes_them_on_write() {
        // build_index/refresh_entry directly: the global INDEX is shared
        // with every other test's vault. Nested: the TempDir root is hidden.
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(vault.join("library")).unwrap();
        fs::write(
            vault.join("library/P1.md"),
            "# P1 — 06-24 Strategic alignment\n",
        )
        .unwrap();
        let mut idx = build_index(&vault);
        assert_eq!(
            idx.by_path["library/P1.md"].title.as_deref(),
            Some("06-24 Strategic alignment")
        );
        refresh_entry(&mut idx, "library/P1.md", "# Renamed meeting\n");
        assert_eq!(
            idx.by_path["library/P1.md"].title.as_deref(),
            Some("Renamed meeting")
        );
    }

    /// Rooted at `vault-XXXX`, never `TempDir::new()`'s `.tmpXXXX`: `walk_notes`
    /// prunes a dot-prefixed ROOT too, which silently emptied every search in
    /// the fixture and made the trash-exclusion test pass without testing.
    fn tmpvault() -> TempDir {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        fs::create_dir_all(dir.path().join("emails/inbox")).unwrap();
        fs::create_dir_all(dir.path().join("projects")).unwrap();
        fs::write(dir.path().join("projects/secret.md"), "secret").unwrap();
        fs::write(dir.path().join("emails/inbox/m.md"), "hi").unwrap();
        dir
    }

    #[test]
    fn safe_join_accepts_in_root() {
        let dir = tmpvault();
        let out = safe_join(dir.path(), "emails/inbox/m.md").expect("ok");
        assert!(out.ends_with("emails/inbox/m.md"));
    }

    #[test]
    fn exclusive_note_creation_preserves_existing_content() {
        let dir = tmpvault();
        let result = create_note(dir.path(), "projects/secret.md", "replacement");
        assert!(result.unwrap_err().contains("already exists"));
        assert_eq!(
            fs::read_to_string(dir.path().join("projects/secret.md")).unwrap(),
            "secret"
        );
    }

    #[test]
    fn cas_does_not_treat_unreadable_content_as_an_empty_note() {
        let dir = tmpvault();
        fs::write(dir.path().join("invalid.md"), [0xff, 0xfe]).unwrap();
        assert!(write_note_cas(dir.path(), "invalid.md", "replacement", "").is_err());
        assert_eq!(
            fs::read(dir.path().join("invalid.md")).unwrap(),
            [0xff, 0xfe]
        );
    }

    #[test]
    fn concurrent_note_creation_has_one_complete_winner() {
        let dir = tmpvault();
        let root = dir.path().to_path_buf();
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let root = root.clone();
                std::thread::spawn(move || {
                    create_note(&root, "notes/same.md", &format!("writer-{index}"))
                })
            })
            .collect();
        let successes: Vec<_> = handles
            .into_iter()
            .filter_map(|handle| handle.join().unwrap().ok())
            .collect();
        assert_eq!(successes.len(), 1);
        assert_eq!(
            fs::read_to_string(root.join("notes/same.md")).unwrap(),
            successes[0].content
        );
        assert_eq!(fs::read_dir(root.join("notes")).unwrap().count(), 1);
    }

    // Wave 2 review nit: the exclusive hard-link path syncs the folder after
    // the link, as `atomic_write` does after its rename, so a power loss
    // can't drop the new entry. Without the sync no `sync-dir` follows. The
    // temporary name is removed before that sync, so the one sync covers
    // both directory changes.
    #[test]
    fn create_note_syncs_the_folder_after_the_link() {
        let dir = tmpvault();
        // The vault root canonicalized, as `safe_join` gives it.
        let full = dir.path().canonicalize().unwrap().join("notes/durable.md");
        let (note, steps) =
            capture_sync_trace(|| create_note(dir.path(), "notes/durable.md", "# Kept"));
        note.unwrap();
        let link = format!("link {}", full.display());
        let sync = format!("sync-dir {}", full.parent().unwrap().display());
        let at = |step: &str| steps.iter().position(|s| s == step);
        let unlinked = steps.iter().position(|s| s.starts_with("unlink "));
        let (Some(linked), Some(unlinked), Some(synced)) = (at(&link), unlinked, at(&sync)) else {
            panic!("no link, temporary removal, then folder sync: {steps:?}");
        };
        assert!(linked < unlinked && unlinked < synced, "{steps:?}");
        assert!(steps[unlinked].contains(".hollow-create-"), "{steps:?}");
    }

    #[test]
    fn exclusive_note_creation_rejects_hidden_and_escaped_paths() {
        let dir = tmpvault();
        for path in [
            "../escape.md",
            "/absolute.md",
            ".obsidian/config.md",
            "notes/.hidden.md",
            "notes/config.json",
        ] {
            assert!(create_note(dir.path(), path, "content").is_err(), "{path}");
        }
    }

    #[test]
    fn obsidian_handoff_encodes_file_paths_and_rejects_escape() {
        let dir = tmpvault();
        fs::write(dir.path().join("a & b #1.md"), "note").unwrap();
        let uri = obsidian_uri(dir.path(), Some("a & b #1.md"), false).unwrap();
        assert!(uri.starts_with("obsidian://open?path=%2F"));
        assert!(uri.ends_with("a%20%26%20b%20%231.md"));
        assert!(obsidian_uri(dir.path(), Some("../outside.md"), false).is_err());
        assert!(obsidian_uri(dir.path(), Some("missing.md"), false).is_err());
        assert!(obsidian_uri(dir.path(), Some("projects/secret.md"), true).is_err());
        let name = dir
            .path()
            .canonicalize()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(
            obsidian_uri(dir.path(), None, true).unwrap(),
            format!("obsidian://daily?vault={}", urlencoding::encode(&name))
        );
    }

    // Data safety 0.1 — port of hollow-mac's DataSafetyTests.atomicWriteNoTornReads.
    // Two distinct >1 MB blobs: a raw in-place `fs::write` truncates then streams
    // the bytes, so a concurrent reader could catch an empty/partial prefix. The
    // temp-file + rename atomic write can't — any successful read is one whole blob.
    #[test]
    fn atomic_write_no_torn_reads() {
        use std::sync::Arc;
        use std::thread;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub/big.md");
        let blob_a = Arc::new(format!("{}\nEND-A\n", "A".repeat(1_500_000)));
        let blob_b = Arc::new(format!("{}\nEND-B\n", "B".repeat(1_500_000)));
        // First write also exercises the create-parent-dir branch.
        atomic_write(&path, blob_a.as_bytes()).unwrap();
        let mut handles = Vec::new();
        for i in 0..40 {
            let p = path.clone();
            let payload = if i % 2 == 0 {
                blob_a.clone()
            } else {
                blob_b.clone()
            };
            handles.push(thread::spawn(move || {
                let _ = atomic_write(&p, payload.as_bytes());
            }));
        }
        for _ in 0..240 {
            let p = path.clone();
            let (a, b) = (blob_a.clone(), blob_b.clone());
            handles.push(thread::spawn(move || {
                if let Ok(s) = fs::read_to_string(&p) {
                    assert!(s == *a || s == *b, "torn read observed (len {})", s.len());
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // No temp turds survive a clean run.
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }

    // Merged plan slice 4 (A·D6 / B·F14): the temp file reaches the disk
    // before it is renamed over the target, and the directory entry is synced
    // after the rename. Without the syncs the trace stays empty.
    #[test]
    fn atomic_write_syncs_the_file_before_the_rename_and_the_dir_after() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub/n.md");
        atomic_write(&path, b"v1").unwrap();
        SYNC_TRACE.with(|t| *t.borrow_mut() = Some(Vec::new()));
        atomic_write(&path, b"v2").unwrap();
        let steps = SYNC_TRACE.with(|t| t.borrow_mut().take()).unwrap();
        assert_eq!(steps.len(), 3, "{steps:?}");
        let synced = steps[0]
            .strip_prefix("sync-file ")
            .expect("file sync first");
        let synced = Path::new(synced);
        assert_eq!(synced.parent(), path.parent(), "{steps:?}");
        let name = synced.file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with(".n.md.") && name.ends_with(".tmp"),
            "the temp file is synced, not the target: {steps:?}"
        );
        assert_eq!(steps[1], format!("rename {}", path.display()));
        assert_eq!(
            steps[2],
            format!("sync-dir {}", path.parent().unwrap().display())
        );
        assert_eq!(fs::read(&path).unwrap(), b"v2");
    }

    // A batched write fsyncs the temp file before the rename and leaves the
    // drive-cache flush to one later `flush_dir`.
    #[test]
    fn atomic_write_batched_defers_the_flush_to_flush_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("obj");
        let ((), steps) = capture_sync_trace(|| {
            atomic_write_batched(&path, b"v1").unwrap();
            flush_dir(dir.path());
        });
        assert_eq!(steps.len(), 3, "{steps:?}");
        let synced = Path::new(steps[0].strip_prefix("fsync-file ").expect("fsync first"));
        assert_eq!(synced.parent(), Some(dir.path()), "{steps:?}");
        assert!(synced
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".obj."));
        assert_eq!(steps[1], format!("rename {}", path.display()));
        assert_eq!(steps[2], format!("sync-dir {}", dir.path().display()));
        assert_eq!(fs::read(&path).unwrap(), b"v1");
    }

    // Merged plan slice 4: rename(2) installs the temp file's inode, so the
    // replaced file's mode has to be carried over — including bits the umask
    // would drop (group write) and a restrictive 0600.
    #[cfg(unix)]
    #[test]
    fn atomic_write_preserves_the_replaced_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        for mode in [0o600, 0o664, 0o755] {
            let path = dir.path().join(format!("m{mode:o}.md"));
            fs::write(&path, "old").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            atomic_write(&path, b"new").unwrap();
            let got = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(got, mode, "mode {mode:o} became {got:o}");
            assert_eq!(fs::read(&path).unwrap(), b"new");
        }
    }

    // The private variant never widens and always narrows to owner-only.
    #[cfg(unix)]
    #[test]
    fn atomic_write_private_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let fresh = dir.path().join("fresh.json");
        atomic_write_private(&fresh, b"{}").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&fresh), 0o600);
        let wide = dir.path().join("wide.json");
        fs::write(&wide, "{}").unwrap();
        fs::set_permissions(&wide, fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write_private(&wide, b"{\"a\":1}").unwrap();
        assert_eq!(mode(&wide), 0o600);
        assert_eq!(fs::read(&wide).unwrap(), b"{\"a\":1}");
    }

    // Data safety 0.2 — port of hollow-mac DataSafetyTests.casRefusesToClobber:
    // a CAS write returns Conflict and preserves the on-disk file when it changed.
    #[test]
    fn cas_refuses_to_clobber_external_edit() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        write_note(v, "n.md", "v1").unwrap();
        // An external editor (Obsidian) writes v2 between open and autosave.
        fs::write(v.join("n.md"), "v2-external").unwrap();
        // Hollow's debounced autosave still holds the v1 baseline.
        match write_note_cas(v, "n.md", "v3-mine", "v1").unwrap() {
            CasOutcome::Conflict(on_disk) => assert_eq!(on_disk.content, "v2-external"),
            CasOutcome::Written(_) => panic!("expected conflict"),
        }
        // The external edit survives — v3 was NOT written.
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "v2-external");
        // Adopting the new baseline lets the write through.
        match write_note_cas(v, "n.md", "v3-mine", "v2-external").unwrap() {
            CasOutcome::Written(n) => assert_eq!(n.content, "v3-mine"),
            CasOutcome::Conflict(_) => panic!("expected written"),
        }
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "v3-mine");
    }

    #[test]
    fn cas_writes_when_unchanged_and_creates_brand_new() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        // A brand-new note: baseline "" matches a missing file → writes.
        match write_note_cas(v, "sub/fresh.md", "hello", "").unwrap() {
            CasOutcome::Written(_) => {}
            CasOutcome::Conflict(_) => panic!("expected written"),
        }
        assert_eq!(fs::read_to_string(v.join("sub/fresh.md")).unwrap(), "hello");
    }

    // Merged plan slice 4 (A·D7): the one writer contract. A writer that
    // states the revision it read gets a conflict, and writes nothing, when
    // the note changed since; the current revision lets it through.
    #[test]
    fn a_writer_with_a_stale_expected_revision_gets_a_conflict() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        let read = write_note(v, "n.md", "v1").unwrap();
        assert_eq!(read.revision, revision(b"v1"));
        assert_eq!(read_note(v, "n.md").unwrap().revision, read.revision);
        fs::write(v.join("n.md"), "v2-external").unwrap();

        let stale = Expected::Revision(&read.revision);
        match write_note_expecting(v, "n.md", "v3", stale).unwrap() {
            CasOutcome::Conflict(on_disk) => {
                assert_eq!(on_disk.content, "v2-external");
                assert_eq!(on_disk.revision, revision(b"v2-external"));
            }
            CasOutcome::Written(_) => panic!("a stale revision must not write"),
        }
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "v2-external");

        let current = revision(b"v2-external");
        let fresh = Expected::Revision(&current);
        match write_note_expecting(v, "n.md", "v3", fresh).unwrap() {
            CasOutcome::Written(note) => assert_eq!(note.revision, revision(b"v3")),
            CasOutcome::Conflict(_) => panic!("the current revision must write"),
        }
        // `Absent` refuses an existing note and leaves it alone.
        let absent = write_note_expecting(v, "n.md", "v4", Expected::Absent);
        assert!(matches!(absent.unwrap(), CasOutcome::Conflict(_)));
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "v3");
    }

    // A·D7: the blind `write_note` (behind the agent's `vault_write_note`)
    // used to skip the mutate lock, so it could land between a mutator's
    // read and its write. It now waits for the lock.
    #[test]
    fn a_blind_write_waits_for_the_mutate_lock() {
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        let guard = lock_mutate();
        let writer = {
            let v = v.clone();
            std::thread::spawn(move || write_note(&v, "blind.md", "late").unwrap())
        };
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !v.join("blind.md").exists(),
            "the blind write landed while another writer held the lock"
        );
        drop(guard);
        writer.join().unwrap();
        assert_eq!(fs::read_to_string(v.join("blind.md")).unwrap(), "late");
    }

    // A·D7: the mutate lock reaches other processes (the notion-sync CLI)
    // through a flock. Holding that flock from another open file
    // description, as another process would, holds a writer back.
    #[test]
    fn the_mutate_lock_waits_for_another_process_holding_its_flock() {
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        // A parallel test may hold the real lock for a moment: wait it out.
        let wait = Duration::from_secs(10);
        let other_process =
            take_process_mutate_lock(&mutate_lock_path(), wait).expect("the mutate flock frees up");
        let writer = {
            let v = v.clone();
            std::thread::spawn(move || write_note(&v, "cross.md", "after").unwrap())
        };
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !v.join("cross.md").exists(),
            "the write landed while another process held the mutate flock"
        );
        drop(other_process);
        writer.join().unwrap();
        assert_eq!(fs::read_to_string(v.join("cross.md")).unwrap(), "after");
    }

    // A hung holder can't freeze a writer: past the wait it goes ahead under
    // the in-process lock alone.
    #[test]
    fn a_stuck_mutate_flock_is_given_up_after_the_wait() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("stuck.lock");
        let file = fs::File::create(&path).unwrap();
        let held = crate::flock::HeldLock::try_take(file).unwrap();
        let short = Duration::from_millis(50);
        let started = Instant::now();
        assert!(take_process_mutate_lock(&path, short).is_none());
        assert!(started.elapsed() >= short);
        drop(held);
        assert!(take_process_mutate_lock(&path, short).is_some());
    }

    // Merged plan slice 4 (A·D8): a no-replace rename refuses an existing
    // target and leaves both files exactly as they were.
    #[test]
    fn a_no_replace_rename_refuses_an_existing_target_atomically() {
        let dir = TempDir::new().unwrap();
        let (from, to) = (dir.path().join("from.md"), dir.path().join("to.md"));
        fs::write(&from, "moving").unwrap();
        fs::write(&to, "already here").unwrap();
        let error = rename_no_replace(&from, &to).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&from).unwrap(), "moving");
        assert_eq!(fs::read_to_string(&to).unwrap(), "already here");
        // The portable fallback refuses the same way.
        let error = link_then_unlink(&from, &to).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&from).unwrap(), "moving");
        assert_eq!(fs::read_to_string(&to).unwrap(), "already here");
        // A free target moves.
        let free = dir.path().join("free.md");
        rename_no_replace(&from, &free).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&free).unwrap(), "moving");
        link_then_unlink(&free, &from).unwrap();
        assert!(!free.exists());
        assert_eq!(fs::read_to_string(&from).unwrap(), "moving");
    }

    // Review of #1399 (2026-11-13): a move is credited with the bytes it
    // moved, hashed from the source, not with what the destination holds
    // when the log is written, so an edit landing right after the move
    // isn't taken for the move's.
    #[test]
    fn a_move_credits_the_moved_bytes_not_a_later_edit() {
        let dir = TempDir::new().unwrap();
        let (from, to) = (dir.path().join("from.md"), dir.path().join("to.md"));
        fs::write(&from, "moved").unwrap();
        let (moved, written) = logging_writes(|| {
            rename_logged(&from, &to, || fs::write(&to, "edited by hand").unwrap())
        });
        moved.unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "edited by hand");
        assert_eq!(written.get(&to), Some(&Some(revision(b"moved"))));
        assert_eq!(written.get(&from), Some(&None));
        // With no log open nothing is read or noted.
        rename_no_replace(&to, &from).unwrap();
        assert_eq!(fs::read_to_string(&from).unwrap(), "edited by hand");
    }

    // A·D8: `move_note` used to check `to.exists()` and then `fs::rename`, so
    // a note created at the destination in between (Obsidian, an agent) was
    // silently replaced. The seam creates it after every check has passed.
    #[test]
    fn move_note_never_replaces_a_note_created_after_its_checks() {
        let dir = TempDir::new().unwrap();
        let v = dir.path();
        write_note(v, "a.md", "mine").unwrap();
        BEFORE_MOVE_RENAME.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|to: &Path| {
                fs::write(to, "theirs").unwrap();
            }));
        });
        let error = move_note(v, "a.md", "b.md").unwrap_err();
        assert!(error.contains("destination exists: b.md"), "{error}");
        assert_eq!(fs::read_to_string(v.join("a.md")).unwrap(), "mine");
        assert_eq!(fs::read_to_string(v.join("b.md")).unwrap(), "theirs");
    }

    /// Whether this volume can swap two files atomically; the swap tests
    /// skip, saying so, where it can't (review 2026-10-20).
    fn swap_supported(dir: &Path) -> bool {
        swap_supported_for_test(dir)
    }

    /// Mode bits of `path`.
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// Delta review (2026-10-20): a third writer replaces the file after
    /// Hollow saw its own content in place and before the swap back, and
    /// its file can't be read. It is never unlinked: it is kept as a
    /// conflict copy (same inode, so its mode too).
    #[test]
    fn an_unreadable_third_write_before_the_swap_back_is_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        if !swap_supported(&v) {
            return;
        }
        write_note(&v, "n.md", "old\n").unwrap();
        let racer = v.join("n.md");
        let third = v.join(".third.tmp");
        set_replace_hook_for_test(Some(Box::new(move |point| match point {
            "before-replace" => fs::write(&racer, "agent 1\n").unwrap(),
            "before-swap-back" => {
                fs::write(&third, "agent 3\n").unwrap();
                fs::set_permissions(&third, fs::Permissions::from_mode(0o000)).unwrap();
                fs::rename(&third, &racer).unwrap();
            }
            _ => {}
        })));
        let outcome = write_note_expecting(
            &v,
            "n.md",
            "mine\n",
            Expected::Revision(&revision(b"old\n")),
        );
        set_replace_hook_for_test(None);
        assert!(matches!(outcome.unwrap(), CasOutcome::Conflict(_)));
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "agent 1\n");
        let copies: Vec<PathBuf> = fs::read_dir(&v)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("n (conflicted copy ")
            })
            .collect();
        assert_eq!(copies.len(), 1, "agent 3's only copy was deleted");
        assert_eq!(mode(&copies[0]), 0o000);
        fs::set_permissions(&copies[0], fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read_to_string(&copies[0]).unwrap(), "agent 3\n");
    }

    /// Only "the volume can't swap" falls back to a plain rename; any other
    /// swap failure is an error (delta review 2026-10-20).
    #[test]
    fn only_an_unsupported_swap_falls_back_to_a_rename() {
        use std::io::Error;
        assert!(swap_unsupported(&Error::from_raw_os_error(libc::ENOTSUP)));
        assert!(swap_unsupported(&Error::from_raw_os_error(libc::EINVAL)));
        assert!(swap_unsupported(&Error::from(
            std::io::ErrorKind::Unsupported
        )));
        assert!(!swap_unsupported(&Error::from_raw_os_error(libc::EACCES)));
        assert!(!swap_unsupported(&Error::from_raw_os_error(libc::EIO)));
    }

    /// Delta review 2 (2026-10-20): the outside write that landed in the
    /// window is unreadable (mode 000). Hollow swaps it back and reports a
    /// conflict that says the current file can't be read, never an error a
    /// caller would take for a failed save.
    #[test]
    fn an_unreadable_write_in_the_window_is_a_conflict_not_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        if !swap_supported(&v) {
            return;
        }
        write_note(&v, "n.md", "old\n").unwrap();
        let racer = v.join("n.md");
        let agent = v.join(".agent.tmp");
        set_replace_hook_for_test(Some(Box::new(move |point| {
            if point == "before-replace" {
                fs::write(&agent, "agent\n").unwrap();
                fs::set_permissions(&agent, fs::Permissions::from_mode(0o000)).unwrap();
                fs::rename(&agent, &racer).unwrap();
            }
        })));
        let outcome = write_note_expecting(
            &v,
            "n.md",
            "mine\n",
            Expected::Revision(&revision(b"old\n")),
        );
        set_replace_hook_for_test(None);
        match outcome {
            Ok(CasOutcome::Conflict(now)) => {
                assert!(
                    now.revision.starts_with(UNREADABLE_REVISION),
                    "{}",
                    now.revision
                );
                assert_eq!(now.content, "");
            }
            Ok(CasOutcome::Written(_)) => panic!("expected an unreadable conflict, got Written"),
            Err(error) => panic!("expected an unreadable conflict, got Err({error})"),
        }
        assert_eq!(mode(&v.join("n.md")), 0o000);
        fs::set_permissions(v.join("n.md"), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "agent\n");
    }

    /// A conflict copy is never readable more widely than its original.
    #[test]
    fn a_conflict_copy_keeps_its_originals_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        write_note(&v, "secret.md", "x\n").unwrap();
        fs::set_permissions(v.join("secret.md"), fs::Permissions::from_mode(0o600)).unwrap();
        let copy = keep_conflict_copy(&v.join("secret.md"), b"y\n").unwrap();
        assert_eq!(mode(&copy), 0o600);
    }

    /// Concurrency PR 2 review (2026-10-20): an agent's write landing after
    /// the check and before the replace is never overwritten silently. The
    /// replace swaps, sees the displaced bytes aren't what it checked, puts
    /// the agent's file back and reports a conflict.
    #[test]
    fn a_write_between_the_check_and_the_replace_is_put_back_as_a_conflict() {
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        if !swap_supported(&v) {
            return;
        }
        write_note(&v, "n.md", "old\n").unwrap();
        let racer = v.join("n.md");
        set_replace_hook_for_test(Some(Box::new(move |point| {
            if point == "before-replace" {
                fs::write(&racer, "agent\n").unwrap();
            }
        })));
        let outcome = write_note_expecting(
            &v,
            "n.md",
            "mine\n",
            Expected::Revision(&revision(b"old\n")),
        );
        set_replace_hook_for_test(None);
        match outcome.unwrap() {
            CasOutcome::Conflict(now) => assert_eq!(now.content, "agent\n"),
            CasOutcome::Written(_) => panic!("the agent's write was overwritten"),
        }
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "agent\n");
        let leftovers: Vec<_> = fs::read_dir(&v)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "n.md")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// A second agent write right after the swap stays in place; the first
    /// one, which the swap displaced, is kept as a conflict copy.
    #[test]
    fn a_second_write_after_the_swap_keeps_both_agent_versions() {
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        if !swap_supported(&v) {
            return;
        }
        write_note(&v, "n.md", "old\n").unwrap();
        let racer = v.join("n.md");
        set_replace_hook_for_test(Some(Box::new(move |point| match point {
            "before-replace" => fs::write(&racer, "agent 1\n").unwrap(),
            "after-swap" => fs::write(&racer, "agent 2\n").unwrap(),
            _ => {}
        })));
        let outcome = write_note_expecting(
            &v,
            "n.md",
            "mine\n",
            Expected::Revision(&revision(b"old\n")),
        );
        set_replace_hook_for_test(None);
        assert!(matches!(outcome.unwrap(), CasOutcome::Conflict(_)));
        assert_eq!(fs::read_to_string(v.join("n.md")).unwrap(), "agent 2\n");
        let copies: Vec<String> = fs::read_dir(&v)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("n (conflicted copy "))
            .collect();
        assert_eq!(copies.len(), 1, "{copies:?}");
        assert_eq!(fs::read_to_string(v.join(&copies[0])).unwrap(), "agent 1\n");
    }

    // Data safety 0.3 — port of hollow-mac DataSafetyTests.mutateNoteNoLostUpdates:
    // concurrent mutate_note appends never lose an update (one critical section).
    #[test]
    fn mutate_note_concurrent_appends_never_lost() {
        use std::sync::Arc;
        use std::thread;
        let dir = TempDir::new().unwrap();
        let v: Arc<PathBuf> = Arc::new(dir.path().to_path_buf());
        write_note(&v, "counter.md", "").unwrap();
        let n = 60;
        let mut handles = Vec::new();
        for i in 0..n {
            let v = v.clone();
            handles.push(thread::spawn(move || {
                let _ = mutate_note(&v, "counter.md", |cur| Ok(format!("{cur}line-{i}\n")));
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let final_content = fs::read_to_string(v.join("counter.md")).unwrap();
        assert_eq!(final_content.lines().count(), n, "an append was lost");
        for i in 0..n {
            assert!(
                final_content.contains(&format!("line-{i}\n")),
                "missing line-{i}"
            );
        }
    }

    #[test]
    fn safe_join_rejects_parent_escape() {
        let dir = tmpvault();
        assert!(safe_join(dir.path(), "../etc/passwd").is_err());
        assert!(safe_join(dir.path(), "emails/../../secret").is_err());
    }

    #[test]
    fn safe_join_rejects_absolute() {
        let dir = tmpvault();
        assert!(safe_join(dir.path(), "/etc/passwd").is_err());
    }

    #[test]
    fn delete_note_moves_to_trash_not_remove() {
        let dir = tmpvault();
        // The source file exists.
        assert!(dir.path().join("projects/secret.md").exists());
        delete_note(dir.path(), "projects/secret.md").expect("ok");
        // Gone from its original location...
        assert!(!dir.path().join("projects/secret.md").exists());
        // ...but recoverable from .trash with its content intact.
        let trash = dir.path().join(".trash");
        assert!(trash.is_dir(), "trash dir should be created");
        let entries: Vec<_> = fs::read_dir(&trash)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(entries.len(), 1, "exactly one trashed file");
        let name = entries[0].file_name().to_string_lossy().to_string();
        assert!(name.ends_with("secret.md"), "keeps basename: {name}");
        assert_eq!(fs::read_to_string(entries[0].path()).unwrap(), "secret");
    }

    /// P1-18: Library's Undo moves the note back from the path trashing
    /// returned, with its content intact.
    #[test]
    fn trash_note_returns_a_path_that_restores_the_note() {
        let dir = tmpvault();
        let trashed = trash_note(dir.path(), "projects/secret.md").expect("trash");
        assert!(trashed.starts_with(".trash/"), "vault-relative: {trashed}");
        assert!(trashed.ends_with("secret.md"), "keeps basename: {trashed}");
        assert!(!dir.path().join("projects/secret.md").exists());
        let restored = move_note(dir.path(), &trashed, "projects/secret.md").expect("undo");
        assert_eq!(restored.content, "secret");
        assert_eq!(
            fs::read_to_string(dir.path().join("projects/secret.md")).unwrap(),
            "secret"
        );
        assert!(!dir.path().join(&trashed).exists());
    }

    /// P1-18 review: the note Undo brings back from `.trash/` is listed and
    /// linked again; a rename resolves links by the new name only.
    #[test]
    fn reindex_moved_lists_a_restored_note_and_renames_its_name() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("alpha.md"), "see [[beta]]").unwrap();
        fs::write(vault.join("gamma.md"), "# gamma").unwrap();
        // beta.md sits in the trash, as after Move to trash.
        let mut idx = build_index(&vault);
        assert!(!idx.by_path.contains_key("beta.md"));
        reindex_moved(&mut idx, ".trash/1-0-beta.md", "beta.md", Some("# beta"));
        assert_eq!(idx.by_path["beta.md"].incoming_from, vec!["alpha.md"]);
        assert_eq!(idx.by_basename["beta"], "beta.md");

        reindex_moved(&mut idx, "beta.md", "notes/delta.md", Some("# beta"));
        assert!(!idx.by_basename.contains_key("beta"));
        assert_eq!(idx.by_basename["delta"], "notes/delta.md");
        assert!(idx.by_path["notes/delta.md"].incoming_from.is_empty());
        // A move into a hidden folder leaves the index, as the walk skips it.
        reindex_moved(&mut idx, "gamma.md", ".trash/gamma.md", Some("# gamma"));
        assert!(!idx.by_path.contains_key(".trash/gamma.md"));
        assert!(!idx.by_path.contains_key("gamma.md"));
    }

    /// P1-18 review: a case-only rename made outside Hollow reaches the
    /// watcher as both spellings, new first (sorted); the index ends with the
    /// new one only, still holding the name and its backlinks.
    #[test]
    fn watcher_patch_takes_an_outside_case_only_rename() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("alpha.md"), "see [[plan]]").unwrap();
        fs::write(vault.join("plan.md"), "# plan").unwrap();
        let mut idx = build_index(&vault);
        fs::rename(vault.join("plan.md"), vault.join("Plan.md")).unwrap();
        apply_index_paths(
            &mut idx,
            &vault,
            &["Plan.md".to_string(), "plan.md".to_string()],
        );
        assert!(!idx.by_path.contains_key("plan.md"));
        assert_eq!(idx.by_basename["plan"], "Plan.md");
        assert_eq!(idx.by_path["Plan.md"].incoming_from, vec!["alpha.md"]);
    }

    /// P1-18 review: a rename that changes only the letter case succeeds and
    /// leaves the file under the typed name (macOS volumes ignore case).
    #[test]
    fn move_note_renames_by_letter_case_only() {
        let dir = tmpvault();
        let note =
            move_note(dir.path(), "projects/secret.md", "projects/Secret.md").expect("case rename");
        assert_eq!(note.content, "secret");
        let names: Vec<String> = fs::read_dir(dir.path().join("projects"))
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["Secret.md".to_string()]);
        // The watcher then sees the old spelling as gone, not as a second note.
        let mut listings = Listings::new();
        assert!(!is_file_as_named(
            &dir.path().join("projects/secret.md"),
            &mut listings
        ));
        assert!(is_file_as_named(
            &dir.path().join("projects/Secret.md"),
            &mut listings
        ));
        // A folder spelled differently is refused, not recorded under a path
        // the walk never produces.
        let folder = move_note(dir.path(), "projects/Secret.md", "Projects/Secret.md");
        assert!(folder.unwrap_err().contains("folder's letter case"));
        assert!(dir.path().join("projects/Secret.md").exists());
    }

    #[test]
    fn delete_note_two_deletes_of_same_name_dont_collide() {
        let dir = tmpvault();
        // Delete, recreate the same path, delete again — both copies should
        // survive in .trash under distinct unique names.
        delete_note(dir.path(), "projects/secret.md").expect("first");
        fs::write(dir.path().join("projects/secret.md"), "second").unwrap();
        delete_note(dir.path(), "projects/secret.md").expect("second");
        let trash = dir.path().join(".trash");
        let count = fs::read_dir(&trash).unwrap().filter_map(Result::ok).count();
        assert_eq!(count, 2, "both deletes preserved, no overwrite");
    }

    #[test]
    fn trashed_files_are_excluded_from_walk_and_search() {
        let dir = tmpvault();
        fs::write(dir.path().join("projects/keepme.md"), "needle haystack").unwrap();
        // Positive control: the fixture is searchable before the delete, so the
        // empty result afterwards means something.
        let before: Vec<String> = search(dir.path(), "needle", 10, None)
            .into_iter()
            .map(|r| r.path)
            .collect();
        assert_eq!(before, vec!["projects/keepme.md".to_string()]);
        delete_note(dir.path(), "projects/keepme.md").expect("ok");
        // The walk (and therefore the tree, recent, index) must not surface
        // anything under .trash.
        assert!(walk_notes(dir.path())
            .iter()
            .all(|n| !n.path.starts_with(".trash")));
        // Search must not find the trashed content either.
        assert!(search(dir.path(), "needle", 10, None).is_empty());
    }

    /// `walk_skips` is the walk's own filter by path (F7 review): every
    /// note the walk lists passes it, and every planted note it skips is
    /// one the walk never lists.
    #[test]
    fn walk_skips_agrees_with_the_walk() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        let kept = ["notes/n.md", "learnings/a.md", "projects/emails/x.md"];
        let skipped = [
            "emails/inbox/x.md",
            "teams/c/x.md",
            "slack/x.md",
            "inbox/x.md",
            "_signature/x.md",
            ".trash/old.md",
            "learnings/.trash/z.md",
            "notes/.hidden.md",
        ];
        for rel in kept.iter().chain(skipped.iter()) {
            let p = vault.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "# x").unwrap();
        }
        let walked: Vec<String> = walk_notes(&vault).into_iter().map(|n| n.path).collect();
        for rel in kept {
            assert!(walked.iter().any(|p| p == rel), "{rel}: {walked:?}");
            assert!(!walk_skips(rel), "{rel}");
        }
        for rel in skipped {
            assert!(!walked.iter().any(|p| p == rel), "{rel}: {walked:?}");
            assert!(walk_skips(rel), "{rel}");
        }
    }

    #[test]
    fn walk_surfaces_html_with_clean_basename() {
        // TempDir names start with '.', which is_hidden would prune — so use a
        // non-dotted subdir as the vault root.
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(vault.join("notes")).unwrap();
        fs::write(vault.join("briefings.html"), "<h1>hi</h1>").unwrap();
        fs::write(vault.join("notes/page.htm"), "<p>x</p>").unwrap();
        fs::write(vault.join("notes/n.md"), "# n").unwrap();

        let names: Vec<String> = walk_notes(&vault).into_iter().map(|n| n.path).collect();
        assert!(
            names.iter().any(|p| p == "briefings.html"),
            "walk includes .html: {names:?}"
        );
        assert!(
            names.iter().any(|p| p == "notes/page.htm"),
            "walk includes .htm: {names:?}"
        );
        assert!(
            names.iter().any(|p| p == "notes/n.md"),
            "walk still includes .md"
        );

        // The link index must strip the extension so a `[[wikilink]]` resolves.
        let idx = build_index(&vault);
        assert!(
            idx.by_basename.contains_key("briefings"),
            "html basename has no extension"
        );
        assert!(
            idx.by_basename.contains_key("page"),
            "htm basename has no extension"
        );
    }

    #[test]
    fn retired_message_folders_are_excluded_from_vault_surfaces() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        for folder in [
            "emails/inbox",
            "teams/dms",
            "slack/dms",
            "inbox",
            "calendar",
            "notes",
        ] {
            fs::create_dir_all(vault.join(folder)).unwrap();
            fs::write(vault.join(folder).join("example.md"), "retired-test-needle").unwrap();
        }
        let notes = walk_notes(&vault);
        assert_eq!(notes.len(), 2);
        assert!(notes
            .iter()
            .all(|n| n.path.starts_with("calendar/") || n.path.starts_with("notes/")));
        let results = search(&vault, "retired-test-needle", 20, None);
        assert!(results
            .iter()
            .all(|n| n.path.starts_with("calendar/") || n.path.starts_with("notes/")));
    }

    // REL-02: backlinks/graph/autocomplete come from a cached link index. When
    // an external write (Obsidian, the embedded agent, a sync) changes the
    // vault, the watcher calls `invalidate_index`, and the next `ensure_index`
    // rebuilds from disk. This proves a rebuild reflects external changes (the
    // property invalidation exists to deliver); it uses the pure `build_index`
    // rather than the process-global cache so it can't race other tests.
    #[test]
    fn rebuilding_the_link_index_reflects_external_changes() {
        let tmp = TempDir::new().unwrap();
        // Nest under a non-hidden subdir: the TempDir root is named `.tmpXXXX`,
        // and `walk_notes` prunes any dot-prefixed directory (and its subtree).
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("alpha.md"), "see [[beta]]").unwrap();
        fs::write(vault.join("beta.md"), "# beta").unwrap();

        // A fresh build resolves alpha's `[[beta]]` into a backlink on beta.
        let before = build_index(&vault);
        assert_eq!(
            before.by_path.get("beta.md").map(|n| n.incoming_from.len()),
            Some(1),
            "alpha should link to beta",
        );

        // An external editor adds another linker; a rebuild must surface it.
        // A stale cache (the bug REL-02 fixes) would still report one.
        fs::write(vault.join("gamma.md"), "also [[beta]]").unwrap();
        let after = build_index(&vault);
        assert_eq!(
            after.by_path.get("beta.md").map(|n| n.incoming_from.len()),
            Some(2),
            "rebuild surfaces gamma's new backlink",
        );
    }

    #[test]
    fn refresh_entry_patches_a_modified_note() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("alpha.md"), "see [[beta]]").unwrap();
        fs::write(vault.join("beta.md"), "# beta").unwrap();
        fs::write(vault.join("gamma.md"), "# gamma").unwrap();
        let mut idx = build_index(&vault);

        // Alpha's link moves from beta to gamma — the patch must move the
        // backlink without a rebuild.
        refresh_entry(&mut idx, "alpha.md", "now [[gamma]]");
        assert!(idx.by_path.get("beta.md").unwrap().incoming_from.is_empty());
        assert_eq!(
            idx.by_path.get("gamma.md").unwrap().incoming_from,
            vec!["alpha.md"]
        );
    }

    #[test]
    fn refresh_entry_resolves_dangling_links_for_a_new_note() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        // Alpha links to a note that doesn't exist yet.
        fs::write(vault.join("alpha.md"), "future [[delta]]").unwrap();
        let mut idx = build_index(&vault);
        assert!(!idx.by_path.contains_key("delta.md"));

        // Delta is created — its new entry must pick up alpha's existing link.
        refresh_entry(&mut idx, "delta.md", "# delta");
        assert_eq!(
            idx.by_path.get("delta.md").unwrap().incoming_from,
            vec!["alpha.md"]
        );
    }

    #[test]
    fn remove_entry_unhooks_backlinks_and_basename() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path().join("vault");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("alpha.md"), "see [[beta]]").unwrap();
        fs::write(vault.join("beta.md"), "back [[alpha]]").unwrap();
        let mut idx = build_index(&vault);
        assert_eq!(
            idx.by_path.get("alpha.md").unwrap().incoming_from,
            vec!["beta.md"]
        );

        remove_entry(&mut idx, "beta.md");
        assert!(!idx.by_path.contains_key("beta.md"));
        assert!(!idx.by_basename.contains_key("beta"));
        // Beta's outgoing contribution to alpha's incoming list is gone too.
        assert!(idx
            .by_path
            .get("alpha.md")
            .unwrap()
            .incoming_from
            .is_empty());
    }

    /// Overnight A14: the hand scan finds exactly what WIKILINK_RE finds, on
    /// fixed cases and on 20,000 random strings over the characters that
    /// matter.
    #[test]
    fn the_wikilink_scan_matches_the_regex_exactly() {
        fn by_regex(content: &str) -> Vec<String> {
            let mut set = HashSet::new();
            for cap in WIKILINK_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    set.insert(m.as_str().trim().to_string());
                }
            }
            let mut v: Vec<String> = set.into_iter().collect();
            v.sort();
            v
        }
        let sorted = |mut v: Vec<String>| {
            v.sort();
            v
        };
        for case in [
            "[[A]]",
            "[[A|x]]",
            "[[A#h]]",
            "[[A#h|x]]",
            "[[A|x#h]]",
            "[[A|]]",
            "[[A#]]",
            "[[]]",
            "[[[A]]",
            "[[A]",
            "[[A\n]]",
            "x [[ A ]] y [[B|c]] [[A]]",
            "[[é|ü]] [[日本]]",
            "[[A]]]]",
            "[[ ]]",
            "[[a|b]x]]",
        ] {
            assert_eq!(sorted(parse_wikilinks(case)), by_regex(case), "{case:?}");
        }
        // Tokens, so `[[`/`]]` pairs (and so real matches) are common.
        let alphabet = [
            "[[", "]]", "[", "]", "|", "#", "\n", "\r", "a", "b", " ", "é",
        ];
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut matched = 0usize;
        for _ in 0..20_000 {
            let mut text = String::new();
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            for k in 0..(seed % 40) {
                let pick = (seed.rotate_left((k % 64) as u32) ^ (k * 2_654_435_761)) as usize
                    % alphabet.len();
                text.push_str(alphabet[pick]);
            }
            let want = by_regex(&text);
            matched += usize::from(!want.is_empty());
            assert_eq!(sorted(parse_wikilinks(&text)), want, "{text:?}");
        }
        assert!(
            matched > 2_000,
            "the fuzz must exercise real matches: {matched}"
        );
    }

    /// Overnight A14: the threaded index finds the same links, titles and
    /// first-basename winners as one thread would.
    #[test]
    fn the_threaded_link_index_matches_a_single_pass() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        for n in 0..600 {
            let folder = v.join(format!("f{}", n % 7));
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join(format!("N{}.md", n % 450)),
                format!(
                    "# Title {n}\n\nSee [[N{}]] and [[N{}|alias]].\n",
                    n / 2,
                    n % 13
                ),
            )
            .unwrap();
        }
        invalidate_walk_cache();
        let idx = build_index(v);
        let notes = walk_notes(v);
        assert_eq!(idx.by_path.len(), notes.len());
        for note in &notes {
            let content = std::fs::read_to_string(v.join(&note.path)).unwrap();
            let entry = &idx.by_path[&note.path];
            let mut want = parse_wikilinks(&content);
            want.sort();
            let mut got = entry.outgoing.clone();
            got.sort();
            assert_eq!(got, want, "{}", note.path);
            assert_eq!(
                entry.title.as_deref(),
                note_title_and_summary(&content, &entry.basename)
                    .0
                    .as_deref()
            );
        }
        // The first note (walk order) with a basename owns it.
        let mut first: HashMap<String, String> = HashMap::new();
        for note in &notes {
            let base = Path::new(&note.name)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .to_lowercase();
            first.entry(base).or_insert_with(|| note.path.clone());
        }
        assert_eq!(idx.by_basename, first);
    }

    /// Conflict copies of `name` (`stem (conflicted copy ...)`) in `dir`.
    fn conflict_copies(dir: &Path, stem: &str) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&format!("{stem} (conflicted copy "))
            })
            .collect()
    }

    const PLAN: &str = "# Plan\n\n- one\n- two\n- three\n- four\n- five\n";

    /// Q-R26..Q-R31 (2026-11-14): a file still holding what was read is
    /// simply replaced, and a missing one is published.
    #[test]
    fn compare_and_replace_writes_over_an_unchanged_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("PLAN.md");
        fs::write(&file, PLAN).unwrap();
        let ours = PLAN.replace("- two", "- two (done)");
        assert_eq!(
            replace_if_unchanged(&file, Some(PLAN.as_bytes()), ours.as_bytes(), false).unwrap(),
            Replace::Done
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), ours);
        assert_eq!(
            write_merging(&file, Some(&ours), "new\n", false).unwrap(),
            MergeWrite::Written
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "new\n");
        let fresh = dir.path().join("NEW.md");
        assert_eq!(
            write_merging(&fresh, None, "hello\n", false).unwrap(),
            MergeWrite::Written
        );
        assert_eq!(fs::read_to_string(&fresh).unwrap(), "hello\n");
        assert!(conflict_copies(dir.path(), "PLAN").is_empty());
    }

    /// A concurrent edit to other lines, saved before the write and saved
    /// in the window between the re-read and the publish, is merged: both
    /// edits are in the file.
    #[test]
    fn a_concurrent_edit_to_other_lines_is_merged() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("PLAN.md");
        let theirs = PLAN.replace("- one", "- one (theirs)");
        let ours = PLAN.replace("- five", "- five (ours)");
        let both = PLAN
            .replace("- one", "- one (theirs)")
            .replace("- five", "- five (ours)");
        // Saved before the write started.
        fs::write(&file, &theirs).unwrap();
        let outcome = write_merging(&file, Some(PLAN), &ours, false).unwrap();
        assert_eq!(
            outcome,
            MergeWrite::Merged {
                text: both.clone(),
                found: theirs.clone()
            }
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), both);
        // Saved inside the window, after the re-read (needs the swap).
        if !swap_supported(dir.path()) {
            return;
        }
        fs::write(&file, PLAN).unwrap();
        let racer = file.clone();
        let mut planted = false;
        let theirs_now = theirs.clone();
        set_replace_hook_for_test(Some(Box::new(move |point| {
            if point == "before-replace" && !planted {
                planted = true;
                fs::write(&racer, &theirs_now).unwrap();
            }
        })));
        let outcome = write_merging(&file, Some(PLAN), &ours, false);
        set_replace_hook_for_test(None);
        assert!(matches!(outcome.unwrap(), MergeWrite::Merged { .. }));
        assert_eq!(fs::read_to_string(&file).unwrap(), both);
        assert!(conflict_copies(dir.path(), "PLAN").is_empty());
    }

    /// Overlapping edits are never written over: the other edit stays in
    /// the file and Hollow's version is kept beside it as a conflict copy.
    #[test]
    fn an_overlapping_edit_is_a_conflict_and_loses_nothing() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("PLAN.md");
        let theirs = PLAN.replace("- three", "- three (theirs)");
        let ours = PLAN.replace("- three", "- three (ours)");
        fs::write(&file, &theirs).unwrap();
        let outcome = write_merging(&file, Some(PLAN), &ours, false).unwrap();
        let MergeWrite::Conflict { copy: Some(copy) } = outcome else {
            panic!("expected a conflict with a copy, got {outcome:?}");
        };
        assert_eq!(fs::read_to_string(&file).unwrap(), theirs);
        assert_eq!(fs::read_to_string(&copy).unwrap(), ours);
        let error = MergeWrite::Conflict { copy: Some(copy) }
            .into_text(&ours, "PLAN.md")
            .unwrap_err();
        assert!(error.contains("didn't overwrite"), "{error}");
        // A file removed meanwhile is a conflict too, never recreated.
        fs::remove_file(&file).unwrap();
        assert!(matches!(
            write_merging(&file, Some(PLAN), &ours, false).unwrap(),
            MergeWrite::Conflict { copy: Some(_) }
        ));
        assert!(!file.exists());
    }

    /// Q-R26: an outside edit saved while `mutate_note`'s transform runs is
    /// merged with the transform's change instead of being overwritten.
    #[test]
    fn mutate_note_merges_an_edit_saved_during_the_transform() {
        let dir = TempDir::new().unwrap();
        let v = dir.path().to_path_buf();
        write_note(&v, "PLAN.md", PLAN).unwrap();
        let outside = v.join("PLAN.md");
        let note = mutate_note(&v, "PLAN.md", |current| {
            fs::write(&outside, current.replace("- one", "- one (Obsidian)")).unwrap();
            Ok(current.replace("- five", "- five (Hollow)"))
        })
        .unwrap();
        let both = PLAN
            .replace("- one", "- one (Obsidian)")
            .replace("- five", "- five (Hollow)");
        assert_eq!(note.content, both);
        assert_eq!(fs::read_to_string(&outside).unwrap(), both);
        let clash = mutate_note(&v, "PLAN.md", |current| {
            fs::write(&outside, current.replace("- two", "- two (Obsidian)")).unwrap();
            Ok(current.replace("- two", "- two (Hollow)"))
        });
        assert!(clash.unwrap_err().contains("didn't overwrite"));
        assert!(fs::read_to_string(&outside)
            .unwrap()
            .contains("- two (Obsidian)"));
        assert_eq!(conflict_copies(&v, "PLAN").len(), 1);
    }

    /// Q-R31: an exclusive publish never shows a partial file under the
    /// final name (it appears only once complete), never replaces a file
    /// already there, and leaves no temp file behind.
    #[test]
    fn exclusive_publish_never_leaves_a_torn_final_file() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("sources/report.pdf");
        let body = vec![7u8; 2_000_000];
        let watched = target.clone();
        set_replace_hook_for_test(Some(Box::new(move |point| {
            if point == "before-publish" {
                assert!(
                    !watched.exists(),
                    "the final name exists before the bytes are synced"
                );
            }
        })));
        let published = publish_exclusive(&target, &body, false);
        set_replace_hook_for_test(None);
        assert!(published.unwrap());
        assert_eq!(fs::read(&target).unwrap(), body);
        assert!(!publish_exclusive(&target, b"other", false).unwrap());
        assert_eq!(
            fs::read(&target).unwrap(),
            body,
            "an existing file was replaced"
        );
        let leftovers: Vec<_> = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "report.pdf")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
