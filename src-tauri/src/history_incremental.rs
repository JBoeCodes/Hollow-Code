//! Incremental History (overhaul K1).
//!
//! The first full-vault snapshot in a process walks the vault and stores a
//! sealed tip. Later snapshots in that process ask FSEvents what changed,
//! flush that stream, and rescan only those paths. The result is still a
//! complete coverage: every unchanged file keeps the hash from the tip, and
//! every changed file is read again. Recovery reads those objects, so the
//! bytes match the vault. A dropped event, a root change, or a tip that does
//! not open falls back to the full walk. A watcher that fails to start
//! releases everything its setup made and registers nothing, so the next
//! snapshot walks and tries again.

use super::*;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
use std::sync::Mutex;

const TIP_FILE: &str = "snapshot-tip.nosync";

/// Highest generation this process has assigned for a vault, and which of
/// those generations is the tip on disk. The counter never goes backwards
/// and is not cleared when a publish fails, so an older sealed tip cannot
/// match again. `published` is `None` when no tip is current.
#[derive(Default)]
struct TipGeneration {
    high: u64,
    published: Option<u64>,
}

static TIP_GENERATION: Lazy<Mutex<HashMap<PathBuf, TipGeneration>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn tip_generations() -> std::sync::MutexGuard<'static, HashMap<PathBuf, TipGeneration>> {
    TIP_GENERATION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[derive(Serialize, Deserialize)]
struct Tip {
    vault: String,
    generation: u64,
    captured: Vec<SnapshotEntry>,
    uncaptured: Vec<UncapturedEntry>,
    unlisted: Vec<String>,
}

#[cfg(all(not(test), target_os = "macos"))]
impl Tip {
    fn coverage(self) -> SnapshotCoverage {
        SnapshotCoverage {
            captured: self.captured,
            uncaptured: self.uncaptured,
            unlisted: self.unlisted,
            links: false,
        }
    }
}

fn tip_path(vault: &Path) -> PathBuf {
    history_root(vault).join(TIP_FILE)
}

fn forget_tip(vault: &Path) {
    let _ = fs::remove_file(tip_path(vault));
    let Ok(root) = fs::canonicalize(vault) else {
        return;
    };
    // Drop the published generation only. The counter stays, so the next
    // save cannot hand the same number out again.
    let mut generations = tip_generations();
    if let Some(state) = generations.get_mut(&root) {
        state.published = None;
    }
}

/// `false` removes any tip already on disk. Nothing is published, and the
/// generation counter still advances, so the removed tip's bytes do not
/// open again. A published tip's objects stay reachable for retention
/// ([`tip_hashes`]).
pub(super) fn save_tip(vault: &Path, coverage: &SnapshotCoverage) -> bool {
    let Ok(root) = fs::canonicalize(vault) else {
        forget_tip(vault);
        return false;
    };
    let generation = {
        let mut generations = tip_generations();
        let state = generations.entry(root.clone()).or_default();
        state.high = state.high.saturating_add(1);
        // The previous publish stays acceptable until this write succeeds
        // or forget_tip runs. The counter still moves, so a later save
        // cannot reuse this number.
        state.high
    };
    let tip = Tip {
        vault: root.to_string_lossy().into_owned(),
        generation,
        captured: coverage.captured.clone(),
        uncaptured: coverage.uncaptured.clone(),
        unlisted: coverage.unlisted.clone(),
    };
    let Ok(body) = serde_json::to_vec(&tip) else {
        forget_tip(vault);
        return false;
    };
    let sealed = seal_stat_cache(&body);
    if sealed.len() as u64 > stat_cache_cap()
        || crate::vault::atomic_write_batched(&tip_path(vault), &sealed).is_err()
    {
        forget_tip(vault);
        return false;
    }
    let mut generations = tip_generations();
    if let Some(state) = generations.get_mut(&root) {
        if state.high == generation {
            state.published = Some(generation);
        }
    }
    true
}

fn load_tip(vault: &Path) -> Option<Tip> {
    let root = fs::canonicalize(vault).ok()?;
    let published = {
        let generations = tip_generations();
        generations.get(&root).and_then(|state| state.published)?
    };
    let bytes = read_stat_cache(&tip_path(vault))?;
    let body = open_sealed(&bytes)?;
    let tip: Tip = serde_json::from_slice(&body).ok()?;
    if tip.vault != root.to_string_lossy() {
        return None;
    }
    (tip.generation == published).then_some(tip)
}

/// Object hashes the current tip still names. Empty when the tip will not
/// open, which is also when the next snapshot walks and writes new objects.
pub(super) fn tip_hashes(vault: &Path) -> HashSet<String> {
    load_tip(vault)
        .map(|tip| tip.captured.into_iter().map(|entry| entry.hash).collect())
        .unwrap_or_default()
}

fn open_sealed(bytes: &[u8]) -> Option<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let split = bytes.iter().position(|&b| b == b'\n')?;
    let (tag, body) = (&bytes[..split], &bytes[split + 1..]);
    let tag = BASE64.decode(tag).ok()?;
    ring::hmac::verify(&STAT_CACHE_KEY, body, &tag).ok()?;
    Some(body.to_vec())
}

/// True when a directory between the vault root and `rel` is a symlink.
#[cfg(any(test, target_os = "macos"))]
fn ancestor_symlink(vault: &Path, rel: &str) -> bool {
    let mut acc = vault.to_path_buf();
    let mut parts = rel.split('/');
    let _leaf = parts.next_back();
    for part in parts {
        acc.push(part);
        match fs::symlink_metadata(&acc) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

#[cfg(any(test, target_os = "macos"))]
fn under(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

#[cfg(any(test, target_os = "macos"))]
fn hidden_rel(path: &str) -> bool {
    path.split('/')
        .any(|part| part.starts_with('.') || part.is_empty())
}

/// Replace `changes` in `tip` by reading those paths, and leave every other
/// file on the hash the tip already stored while its object is still in the
/// store. One whose object is gone is read again as if it had changed: the
/// tip is this process's alone, so another process's object cleanup does
/// not keep what only it names (Q-R9, 2026-11-13), and a run record must
/// never name an object the store lacks. The caller holds
/// `objects_in_flight`, so nothing removes one between this check and the
/// record.
#[cfg(any(test, target_os = "macos"))]
pub(super) fn apply_changes(
    vault: &Path,
    tip: SnapshotCoverage,
    changes: &[String],
) -> Result<SnapshotCoverage, String> {
    let mut captured: BTreeMap<String, SnapshotEntry> = tip
        .captured
        .into_iter()
        .map(|entry| (entry.vault_path.clone(), entry))
        .collect();
    let mut uncaptured: BTreeMap<String, UncapturedEntry> = tip
        .uncaptured
        .into_iter()
        .map(|entry| (entry.vault_path.clone(), entry))
        .collect();
    let mut unlisted: BTreeSet<String> = tip.unlisted.into_iter().collect();
    // One `lstat` per kept file, the stat cache's own check (`object_stat`).
    let missing: Vec<String> = captured
        .values()
        .filter(|entry| object_stat(vault, &entry.hash).is_none())
        .map(|entry| entry.vault_path.clone())
        .collect();
    let mut roots: Vec<&String> = changes
        .iter()
        .chain(&missing)
        .filter(|rel| validate_vault_path(rel).is_ok() && !hidden_rel(rel))
        .collect();
    roots.sort();
    roots.dedup();
    let mut kept: Vec<&String> = Vec::new();
    for rel in roots {
        if kept.iter().any(|root| under(rel, root)) {
            continue;
        }
        // A symlink ancestor would let a later child path read outside the
        // vault. The caller walks the whole vault instead.
        if ancestor_symlink(vault, rel) {
            return Err(format!("symlink above {rel}"));
        }
        kept.push(rel);
    }
    for rel in kept {
        captured.retain(|path, _| !under(path, rel));
        uncaptured.retain(|path, _| !under(path, rel));
        unlisted.retain(|path| !under(path, rel));
        let abs = vault.join(rel);
        match fs::symlink_metadata(&abs) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            // A directory we cannot stat must not look like a deletion of
            // everything under it. The caller walks the vault.
            Err(_) => return Err(format!("could not stat {rel}")),
        }
        // Broker walk: a link is recorded as excluded and is not followed.
        let part = walk_coverage(vault, &[abs], None, true)?;
        for entry in part.captured {
            captured.insert(entry.vault_path.clone(), entry);
        }
        for entry in part.uncaptured {
            uncaptured.insert(entry.vault_path.clone(), entry);
        }
        unlisted.extend(part.unlisted);
    }
    let mut out = SnapshotCoverage {
        captured: captured.into_values().collect(),
        uncaptured: uncaptured.into_values().collect(),
        unlisted: unlisted.into_iter().collect(),
        links: false,
    };
    out.captured.sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
    out.uncaptured
        .sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
    out.unlisted.sort();
    out.unlisted.dedup();
    Ok(out)
}

/// Test-only failure injection for watcher setup. In a release build the
/// macro is the call itself, so no step can be made to fail there.
#[cfg(all(test, target_os = "macos"))]
macro_rules! unless_injected {
    ($step:ident, $failed:expr, $call:expr) => {
        if seam::fails(seam::Step::$step) {
            $failed
        } else {
            $call
        }
    };
}

#[cfg(all(not(test), target_os = "macos"))]
macro_rules! unless_injected {
    ($step:ident, $failed:expr, $call:expr) => {
        $call
    };
}

#[cfg(target_os = "macos")]
struct ChangeBuf {
    paths: BTreeSet<String>,
    /// The stream dropped events or the vault root itself moved. The next
    /// snapshot walks the whole vault.
    rescan: bool,
}

#[cfg(target_os = "macos")]
struct WatchState {
    root: PathBuf,
    buf: Mutex<ChangeBuf>,
}

#[cfg(target_os = "macos")]
impl WatchState {
    fn new(root: &Path) -> Self {
        #[cfg(test)]
        seam::add(seam::Kind::State, 1);
        WatchState {
            root: root.to_path_buf(),
            buf: Mutex::new(ChangeBuf {
                paths: BTreeSet::new(),
                rescan: false,
            }),
        }
    }

    /// The paths reported since the last call, or `None` when the stream
    /// asked for a full scan. Either way the buffer is empty afterwards.
    fn take(&self) -> Option<BTreeSet<String>> {
        let mut buf = self.buf.lock().unwrap_or_else(|error| error.into_inner());
        if buf.rescan {
            buf.rescan = false;
            buf.paths.clear();
            return None;
        }
        Some(std::mem::take(&mut buf.paths))
    }
}

#[cfg(all(test, target_os = "macos"))]
impl Drop for WatchState {
    fn drop(&mut self) {
        seam::add(seam::Kind::State, -1);
    }
}

/// One vault's event stream and what it holds. Setup fills it in one step
/// at a time, and dropping it undoes whatever was made, in reverse. A failed
/// step therefore leaves nothing behind, and a torn-down stream frees its
/// callback state.
#[cfg(target_os = "macos")]
struct Armed {
    /// Null until `FSEventStreamCreate` succeeds.
    stream: *mut c_void,
    /// Null until `dispatch_queue_create` succeeds. Released after the
    /// stream.
    queue: *mut c_void,
    /// The callback's context, from `Box::into_raw`. Owned here, and freed
    /// only after the stream is stopped, invalidated, drained and released,
    /// so no callback can still hold it.
    state: *mut WatchState,
    /// `FSEventStreamSetDispatchQueue` ran, so the stream must be
    /// invalidated before it is released.
    scheduled: bool,
    /// `FSEventStreamStart` succeeded, so the stream must be stopped and its
    /// queue drained.
    started: bool,
}

#[cfg(target_os = "macos")]
unsafe impl Send for Armed {}

#[cfg(target_os = "macos")]
impl Drop for Armed {
    fn drop(&mut self) {
        unsafe {
            if !self.stream.is_null() {
                if self.started {
                    FSEventStreamStop(self.stream);
                }
                if self.scheduled {
                    FSEventStreamInvalidate(self.stream);
                }
                if self.started {
                    // A callback already running on the serial queue finishes
                    // before the state it reads is freed.
                    dispatch_sync_f(self.queue, std::ptr::null_mut(), drained);
                }
                FSEventStreamRelease(self.stream);
                #[cfg(test)]
                seam::add(seam::Kind::Stream, -1);
            }
            if !self.queue.is_null() {
                dispatch_release(self.queue);
                #[cfg(test)]
                seam::add(seam::Kind::Queue, -1);
            }
            drop(Box::from_raw(self.state));
        }
    }
}

#[cfg(target_os = "macos")]
extern "C" fn drained(_context: *mut c_void) {}

/// A CoreFoundation object this setup created and releases when dropped.
#[cfg(target_os = "macos")]
struct CfOwned(*const c_void);

#[cfg(target_os = "macos")]
impl CfOwned {
    fn new(object: *const c_void) -> Option<Self> {
        if object.is_null() {
            return None;
        }
        #[cfg(test)]
        seam::add(seam::Kind::Cf, 1);
        Some(CfOwned(object))
    }
}

#[cfg(target_os = "macos")]
impl Drop for CfOwned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) };
        #[cfg(test)]
        seam::add(seam::Kind::Cf, -1);
    }
}

#[cfg(target_os = "macos")]
static WATCHES: Lazy<Mutex<HashMap<PathBuf, Armed>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(not(test))]
pub(super) fn armed(vault: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Ok(root) = fs::canonicalize(vault) else {
            return false;
        };
        WATCHES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&root)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = vault;
        false
    }
}

/// Start the event stream if this process does not already have one for
/// `vault`. Events that arrive after this call are kept for the next
/// snapshot. A failure leaves the vault unarmed, so the caller walks.
#[cfg(not(test))]
pub(super) fn arm(vault: &Path) {
    #[cfg(target_os = "macos")]
    {
        let Ok(root) = fs::canonicalize(vault) else {
            return;
        };
        arm_root(root);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = vault;
    }
}

/// Register a stream for `root` unless one is already there. A failed setup
/// has released everything it made and registers nothing, so the next full
/// snapshot walks and tries again without leaving anything behind.
#[cfg(target_os = "macos")]
fn arm_root(root: PathBuf) {
    let mut watches = WATCHES.lock().unwrap_or_else(|error| error.into_inner());
    if watches.contains_key(&root) {
        return;
    }
    if let Some(armed) = start_stream(&root) {
        watches.insert(root, armed);
    }
}

/// Flush the stream and apply the paths it reported to the saved tip.
/// `None` means the caller should walk the vault (nothing armed, no tip,
/// or the stream asked for a full scan).
#[cfg(not(test))]
pub(super) fn try_since_tip(vault: &Path) -> Option<SnapshotCoverage> {
    #[cfg(target_os = "macos")]
    {
        let root = fs::canonicalize(vault).ok()?;
        let stream = {
            let watches = WATCHES.lock().unwrap_or_else(|error| error.into_inner());
            let armed = watches.get(&root)?;
            armed.stream
        };
        // Deliver events the kernel already has before we read the buffer.
        // The callback runs on the stream's queue and takes the buffer lock.
        unsafe { FSEventStreamFlushSync(stream) };
        let batch = {
            let watches = WATCHES.lock().unwrap_or_else(|error| error.into_inner());
            let armed = watches.get(&root)?;
            let state = unsafe { &*armed.state };
            state.take()?
        };
        let tip = load_tip(vault)?;
        let changes: Vec<String> = batch.into_iter().collect();
        match apply_changes(vault, tip.coverage(), &changes) {
            Ok(coverage) => {
                // A failed publish must not leave the previous tip in place:
                // its events were already taken off the stream.
                if !save_tip(vault, &coverage) {
                    forget_tip(vault);
                }
                Some(coverage)
            }
            Err(_) => {
                forget_tip(vault);
                None
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = vault;
        None
    }
}

/// Create, schedule and start a stream for `root`. The returned guard owns
/// the callback state from the first line, and every early return drops it,
/// which releases what was made so far. The state only outlives this call
/// inside a started stream.
#[cfg(target_os = "macos")]
fn start_stream(root: &Path) -> Option<Armed> {
    let mut armed = Armed {
        stream: std::ptr::null_mut(),
        queue: std::ptr::null_mut(),
        state: Box::into_raw(Box::new(WatchState::new(root))),
        scheduled: false,
        started: false,
    };
    let c_path = unless_injected!(
        Name,
        None,
        std::ffi::CString::new(root.to_string_lossy().as_bytes()).ok()
    )?;
    unsafe {
        let cf_path = CfOwned::new(unless_injected!(
            CfPath,
            std::ptr::null(),
            CFStringCreateWithCString(std::ptr::null(), c_path.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        ))?;
        let values = [cf_path.0];
        let array = CfOwned::new(unless_injected!(
            CfArray,
            std::ptr::null(),
            CFArrayCreate(std::ptr::null(), values.as_ptr(), 1, &kCFTypeArrayCallBacks)
        ))?;
        drop(cf_path);
        let mut context = FSEventStreamContext {
            version: 0,
            info: armed.state.cast::<c_void>(),
            retain: std::ptr::null(),
            release: std::ptr::null(),
            copy_description: std::ptr::null(),
        };
        armed.stream = unless_injected!(
            Create,
            std::ptr::null_mut(),
            FSEventStreamCreate(
                std::ptr::null(),
                on_events,
                &mut context,
                array.0,
                u64::MAX,
                0.0,
                FLAG_USE_CF_TYPES | FLAG_NO_DEFER | FLAG_WATCH_ROOT | FLAG_FILE_EVENTS,
            )
        );
        drop(array);
        if armed.stream.is_null() {
            return None;
        }
        #[cfg(test)]
        seam::add(seam::Kind::Stream, 1);
        armed.queue = unless_injected!(
            Queue,
            std::ptr::null_mut(),
            dispatch_queue_create(c"hollow.history.incremental".as_ptr(), std::ptr::null())
        );
        if armed.queue.is_null() {
            return None;
        }
        #[cfg(test)]
        seam::add(seam::Kind::Queue, 1);
        FSEventStreamSetDispatchQueue(armed.stream, armed.queue);
        armed.scheduled = true;
        if unless_injected!(Start, 0, FSEventStreamStart(armed.stream)) == 0 {
            return None;
        }
        armed.started = true;
    }
    Some(armed)
}

#[cfg(target_os = "macos")]
extern "C" fn on_events(
    _stream: *mut c_void,
    info: *mut c_void,
    num_events: usize,
    event_paths: *mut c_void,
    event_flags: *const u32,
    _event_ids: *const u64,
) {
    if info.is_null() || event_paths.is_null() || event_flags.is_null() {
        return;
    }
    let state = unsafe { &*(info as *const WatchState) };
    let count = unsafe { CFArrayGetCount(event_paths) };
    let n = (num_events as isize).min(count);
    let mut buf = state.buf.lock().unwrap_or_else(|error| error.into_inner());
    for index in 0..n {
        let flag = unsafe { *event_flags.add(index as usize) };
        if flag & (FLAG_USER_DROPPED | FLAG_KERNEL_DROPPED | FLAG_ROOT_CHANGED) != 0 {
            buf.rescan = true;
            continue;
        }
        let value = unsafe { CFArrayGetValueAtIndex(event_paths, index) };
        let Some(path) = cf_path(value) else {
            buf.rescan = true;
            continue;
        };
        let Ok(rel) = Path::new(&path).strip_prefix(&state.root) else {
            // An event we cannot place under the vault. Walk everything.
            buf.rescan = true;
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.is_empty() || hidden_rel(&rel) {
            if rel.is_empty() {
                buf.rescan = true;
            }
            continue;
        }
        buf.paths.insert(rel);
    }
}

#[cfg(target_os = "macos")]
fn cf_path(value: *const c_void) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let mut buf = vec![0u8; 8192];
    let ok = unsafe {
        CFStringGetCString(
            value,
            buf.as_mut_ptr() as *mut i8,
            buf.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|&byte| byte == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

#[cfg(target_os = "macos")]
const FLAG_USE_CF_TYPES: u32 = 0x0000_0001;
#[cfg(target_os = "macos")]
const FLAG_NO_DEFER: u32 = 0x0000_0002;
#[cfg(target_os = "macos")]
const FLAG_WATCH_ROOT: u32 = 0x0000_0004;
#[cfg(target_os = "macos")]
const FLAG_FILE_EVENTS: u32 = 0x0000_0010;
#[cfg(target_os = "macos")]
const FLAG_USER_DROPPED: u32 = 0x0000_0002;
#[cfg(target_os = "macos")]
const FLAG_KERNEL_DROPPED: u32 = 0x0000_0004;
#[cfg(target_os = "macos")]
const FLAG_ROOT_CHANGED: u32 = 0x0000_0020;
#[cfg(target_os = "macos")]
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

#[cfg(target_os = "macos")]
#[repr(C)]
struct FSEventStreamContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CFArrayCallBacks {
    version: isize,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreServices", kind = "framework")]
#[allow(non_snake_case, non_upper_case_globals)]
extern "C" {
    fn FSEventStreamCreate(
        allocator: *const c_void,
        callback: extern "C" fn(
            *mut c_void,
            *mut c_void,
            usize,
            *mut c_void,
            *const u32,
            *const u64,
        ),
        context: *mut FSEventStreamContext,
        paths: *const c_void,
        since: u64,
        latency: f64,
        flags: u32,
    ) -> *mut c_void;
    fn FSEventStreamSetDispatchQueue(stream: *mut c_void, queue: *mut c_void);
    fn FSEventStreamStart(stream: *mut c_void) -> u8;
    fn FSEventStreamStop(stream: *mut c_void);
    fn FSEventStreamFlushSync(stream: *mut c_void);
    fn FSEventStreamInvalidate(stream: *mut c_void);
    fn FSEventStreamRelease(stream: *mut c_void);
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
#[allow(non_snake_case, non_upper_case_globals)]
extern "C" {
    static kCFTypeArrayCallBacks: CFArrayCallBacks;
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        bytes: *const i8,
        encoding: u32,
    ) -> *const c_void;
    fn CFArrayCreate(
        allocator: *const c_void,
        values: *const *const c_void,
        count: isize,
        callbacks: *const CFArrayCallBacks,
    ) -> *const c_void;
    fn CFArrayGetCount(array: *mut c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *mut c_void, index: isize) -> *const c_void;
    fn CFStringGetCString(string: *const c_void, buffer: *mut i8, size: isize, encoding: u32)
        -> u8;
    fn CFRelease(value: *const c_void);
}

#[cfg(target_os = "macos")]
extern "C" {
    fn dispatch_queue_create(label: *const std::ffi::c_char, attr: *const c_void) -> *mut c_void;
    fn dispatch_sync_f(queue: *mut c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_release(object: *mut c_void);
}

/// Test-only failure switch behind `unless_injected!`, and counters of what
/// watcher setup allocates. Both are per thread, so parallel tests do not
/// see each other's counts.
#[cfg(all(test, target_os = "macos"))]
mod seam {
    use std::cell::Cell;

    /// Watcher setup steps, in the order `start_stream` takes them.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum Step {
        /// The root as a C string, before any CoreFoundation allocation.
        Name,
        CfPath,
        CfArray,
        Create,
        Queue,
        Start,
    }

    #[derive(Clone, Copy)]
    pub(super) enum Kind {
        State,
        Cf,
        Stream,
        Queue,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct Counts {
        pub(super) states: isize,
        pub(super) cf: isize,
        pub(super) streams: isize,
        pub(super) queues: isize,
    }

    impl Counts {
        pub(super) const ZERO: Counts = Counts::new(0, 0, 0, 0);

        pub(super) const fn new(states: isize, cf: isize, streams: isize, queues: isize) -> Self {
            Counts {
                states,
                cf,
                streams,
                queues,
            }
        }

        fn plus(mut self, kind: Kind, delta: isize) -> Self {
            match kind {
                Kind::State => self.states += delta,
                Kind::Cf => self.cf += delta,
                Kind::Stream => self.streams += delta,
                Kind::Queue => self.queues += delta,
            }
            self
        }

        pub(super) fn since(self, before: Counts) -> Counts {
            Counts::new(
                self.states - before.states,
                self.cf - before.cf,
                self.streams - before.streams,
                self.queues - before.queues,
            )
        }
    }

    thread_local! {
        static FAIL_AT: Cell<Option<Step>> = const { Cell::new(None) };
        /// Made and not yet released.
        static LIVE: Cell<Counts> = const { Cell::new(Counts::ZERO) };
        /// Made, ever.
        static MADE: Cell<Counts> = const { Cell::new(Counts::ZERO) };
    }

    pub(super) fn fails(step: Step) -> bool {
        FAIL_AT.with(|at| at.get() == Some(step))
    }

    pub(super) fn fail_at(step: Option<Step>) {
        FAIL_AT.with(|at| at.set(step));
    }

    pub(super) fn add(kind: Kind, delta: isize) {
        LIVE.with(|live| live.set(live.get().plus(kind, delta)));
        if delta > 0 {
            MADE.with(|made| made.set(made.get().plus(kind, delta)));
        }
    }

    pub(super) fn live() -> Counts {
        LIVE.with(Cell::get)
    }

    pub(super) fn made() -> Counts {
        MADE.with(Cell::get)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod watcher_tests {
    use super::seam::{self, Counts, Step};
    use super::*;

    fn temp_root(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("hollow-test-{name}-"))
            .tempdir()
            .unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        (dir, root)
    }

    fn registered(root: &Path) -> bool {
        WATCHES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(root)
    }

    /// Audit E5 low: a setup step that fails releases the callback state and
    /// everything made before it, and registers nothing. Every later full
    /// snapshot retries, so repeated failures must not add up.
    #[test]
    fn a_failed_watcher_setup_releases_what_it_made_and_registers_nothing() {
        let (_dir, root) = temp_root("watch-fail");
        // What each failing step has made by the time it fails: states,
        // CoreFoundation objects, streams, queues.
        let cases = [
            (Step::Name, Counts::new(1, 0, 0, 0)),
            (Step::CfPath, Counts::new(1, 0, 0, 0)),
            (Step::CfArray, Counts::new(1, 1, 0, 0)),
            (Step::Create, Counts::new(1, 2, 0, 0)),
            (Step::Queue, Counts::new(1, 2, 1, 0)),
            (Step::Start, Counts::new(1, 2, 1, 1)),
        ];
        for (step, expected) in cases {
            seam::fail_at(Some(step));
            for attempt in 1..=3 {
                let before = seam::made();
                arm_root(root.clone());
                assert!(
                    !registered(&root),
                    "{step:?} attempt {attempt} registered a stream"
                );
                assert_eq!(
                    seam::made().since(before),
                    expected,
                    "{step:?} attempt {attempt} did not stop at that step"
                );
                assert_eq!(
                    seam::live(),
                    Counts::ZERO,
                    "{step:?} attempt {attempt} left something live"
                );
            }
        }
        seam::fail_at(None);
    }

    /// The callback state lives exactly as long as a started stream, and a
    /// second arm of the same vault makes nothing new.
    #[test]
    fn a_live_watcher_frees_its_state_when_torn_down() {
        let (_dir, root) = temp_root("watch-live");
        arm_root(root.clone());
        assert!(registered(&root), "the stream did not start");
        assert_eq!(seam::live(), Counts::new(1, 0, 1, 1));
        arm_root(root.clone());
        assert_eq!(seam::made(), Counts::new(1, 2, 1, 1));
        {
            let watches = WATCHES.lock().unwrap_or_else(|error| error.into_inner());
            let armed = &watches[&root];
            // The flush runs the callback on the stream's queue.
            unsafe { FSEventStreamFlushSync(armed.stream) };
            if let Some(paths) = unsafe { &*armed.state }.take() {
                assert!(paths
                    .iter()
                    .all(|path| !path.is_empty() && !hidden_rel(path)));
            }
        }
        let armed = WATCHES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&root);
        assert!(armed.is_some());
        drop(armed);
        assert!(!registered(&root));
        assert_eq!(seam::live(), Counts::ZERO);
    }
}
