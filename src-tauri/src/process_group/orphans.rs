//! Runs that outlive the Hollow process that started them (R2).
//!
//! [`GroupGuard`](super::GroupGuard) ends a run's processes only while the
//! Hollow driving it is alive. When Hollow quits, crashes or is relaunched by
//! an install mid-turn, the agent CLI is reparented to launchd and keeps
//! working: on 2026-09-29 such an "ended" turn merged PR #215 and merged main
//! into a branch while the new turn was doing the same. So:
//!
//! 1. **The run ledger.** Every isolated guard adopted in the GUI process is
//!    appended to `{app data}/run-ledger.json` ([`LEDGER_FILE`], under
//!    `HOLLOW_APP_DATA_DIR` when set) as a [`LedgerEntry`]
//!    `{runId, tag, pid, startTime, bootAt, ownerPid, ownerStart}` and
//!    removed when the guard settles (or after its drop sweep). Writes are
//!    whole-file atomic rewrites (temp file, fsync, rename) under a process
//!    mutex and an `flock` on `run-ledger.json.lock`, since a second Hollow (a
//!    dev build beside the installed app) may share the directory. A missing
//!    file is empty; an unreadable one is logged and replaced at the next
//!    write; a malformed entry is dropped on its own.
//! 2. **The boot sweep** ([`start_at_boot`], `lib.rs` setup, before history
//!    reconciliation and `run_queue::recover_at_startup`). An entry is stale
//!    only when its owner Hollow is provably gone: its pid is dead, or alive
//!    with a different start time. This process's own entries and a live
//!    foreign Hollow's are never touched. For each stale entry, a process is
//!    the run's when its environment carries that entry's exact
//!    `HOLLOW_RUN_TAG=<32 hex>` token, or when it is the recorded leader pid
//!    AND still has the recorded start time; the groups those processes lead
//!    or sit in are signalled whole, as [`GroupGuard::settle`] does. Then the
//!    same sequence: SIGTERM, the grace period, SIGKILL from a fresh scan,
//!    verify. What was ended is logged, and the run ids whose leftovers were
//!    stopped go to the run queue, which notes the interrupted row with
//!    [`LEFTOVER_STOPPED_NOTE`].
//! 3. **The quit settle** ([`settle_at_quit`], `RunEvent::Exit`): this
//!    process's live entries get the same sequence, bounded by
//!    [`QUIT_BUDGET`]. Hiding the window to the tray is not a quit. Whatever
//!    the budget cuts short stays in the ledger for the next launch.
//! 4. **The signal settle** ([`QuitSignals`], [`settle_on_signal`], A·R8):
//!    SIGTERM, SIGINT and SIGHUP end a process by default, before
//!    `RunEvent::Exit` could run. The GUI takes them over at setup, runs the
//!    same quit settle (and the services stop) once, then asks Tauri to
//!    exit. A second signal while settling exits at once. SIGHUP is left
//!    alone when Hollow started with it ignored (`nohup`).
//! 5. **The crash reaper** ([`super::reaper`], A·R8): a crash (a panic, which
//!    aborts in release, or a SIGKILL) runs no code at all, so a tiny
//!    `hollow reaper` child started at setup waits for this process to exit
//!    and sweeps its entries right away instead of at the next launch.
//!
//! Never matched: an untagged process, a process carrying some other tag
//! (the install watcher deliberately runs without one), and a reused pid
//! whose start time differs. The only tags ever looked for are ones this
//! ledger recorded. A leftover whose environment `ps -E` cannot read (an
//! Apple platform binary such as `/bin/sleep`, `/bin/zsh` or `git`) is found
//! through its group, as the recorded leader, or, once the leader is gone,
//! as a survivor of the leader's group that started no earlier than the
//! recorded leader ([`group_survivors`]), only while that leader is from
//! this boot and started within [`SURVIVOR_WINDOW`]. Sampled descendants
//! are not persisted, so a platform binary that left the group is still
//! missed, and so is a survivor of an older run.

#[cfg(unix)]
use super::{
    group_alive, identify, own_pgrp, own_pid, pid_alive, ps_scan, signal_group, signal_members,
    Ours, Proc,
};
use super::{StopOutcome, Timings};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

/// The ledger's file name in the app data directory.
pub(crate) const LEDGER_FILE: &str = "run-ledger.json";
/// Appended to an interrupted run-queue row whose leftovers the boot sweep
/// stopped.
pub(crate) const LEFTOVER_STOPPED_NOTE: &str =
    "An earlier turn was still running after Hollow restarted; it was stopped.";
/// The whole quit settle, scans included.
pub(crate) const QUIT_BUDGET: Duration = Duration::from_secs(3);
const LEDGER_VERSION: u32 = 1;
/// Oldest entries go first beyond this (a guard never settled and never
/// swept should not grow the file forever).
const MAX_ENTRIES: usize = 512;
/// An entry whose owner cannot be judged (liveness unreadable) is dropped,
/// never swept, once it is this old.
const UNJUDGED_MAX_AGE_DAYS: i64 = 14;
/// A dead leader's group is searched for survivors ([`group_survivors`])
/// only when that leader started at most this long before the scan. With
/// the reaper and the quit settle, a real survivor is found seconds after
/// its Hollow exits; past this, a leaderless group with the recorded id is
/// more likely a reused one than the run's.
const SURVIVOR_WINDOW: Duration = Duration::from_secs(6 * 60 * 60);
/// How long a ledger write waits for another Hollow's `flock` before
/// writing anyway (the lock only narrows a race between two processes).
const FLOCK_WAIT: Duration = Duration::from_secs(1);

/// One live run, as recorded at adopt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LedgerEntry {
    /// The history run id (a run-queue row's `history_run_id`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) run_id: Option<String>,
    /// The run's `HOLLOW_RUN_TAG` (32 lowercase hex); the entry's key.
    pub(crate) tag: String,
    /// The leader's pid (also its process-group id).
    pub(crate) pid: i32,
    /// The leader's start stamp (`proc_info::process_start_micros`).
    #[serde(default)]
    pub(crate) start_time: Option<u64>,
    /// When the owning Hollow process started.
    pub(crate) boot_at: DateTime<Utc>,
    /// The owning Hollow process, and its start stamp.
    pub(crate) owner_pid: i32,
    #[serde(default)]
    pub(crate) owner_start: Option<u64>,
}

impl LedgerEntry {
    fn is_valid(&self) -> bool {
        self.pid > 1
            && self.owner_pid > 0
            && self.tag.len() == 32
            && self
                .tag
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LedgerDoc {
    version: u32,
    entries: Vec<serde_json::Value>,
}

/// The Hollow process a ledger belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Boot {
    pub(crate) at: DateTime<Utc>,
    pub(crate) pid: i32,
    pub(crate) start: Option<u64>,
}

impl Boot {
    pub(crate) fn this_process(at: DateTime<Utc>) -> Self {
        Self {
            at,
            pid: std::process::id() as i32,
            start: crate::proc_info::own_start_micros(),
        }
    }
}

/// Whose an entry is, as far as this process can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    /// This process (this boot).
    ThisProcess,
    /// Another Hollow that is still running.
    Live,
    /// Its Hollow is gone: the entry is swept.
    Gone,
    /// Liveness could not be read: left alone.
    Unknown,
}

pub(crate) fn owner_of(entry: &LedgerEntry, boot: &Boot) -> Owner {
    if entry.owner_pid == boot.pid
        && (entry.boot_at == boot.at || (boot.start.is_some() && entry.owner_start == boot.start))
    {
        return Owner::ThisProcess;
    }
    match crate::proc_info::pid_alive(entry.owner_pid) {
        Some(false) => Owner::Gone,
        Some(true) => {
            let now = crate::proc_info::process_start_micros(entry.owner_pid);
            match (entry.owner_start, now) {
                (Some(then), Some(now)) if then != now => Owner::Gone,
                (Some(_), Some(_)) => Owner::Live,
                // An alive pid without a comparable stamp may be the owner.
                _ => Owner::Unknown,
            }
        }
        None => Owner::Unknown,
    }
}

/// The run ledger of one Hollow process.
pub struct Ledger {
    path: PathBuf,
    boot: Boot,
    /// This process's live entries, by tag (what the quit settle ends).
    live: Mutex<HashMap<String, LedgerEntry>>,
    write: Mutex<()>,
}

static GLOBAL: OnceLock<Ledger> = OnceLock::new();

/// The GUI process's ledger, once [`start_at_boot`] set it. `None` in MCP
/// subprocesses, CLI subcommands and tests, so their guards record nothing.
pub(crate) fn global() -> Option<&'static Ledger> {
    GLOBAL.get()
}

impl Ledger {
    pub(crate) fn new(path: PathBuf, boot: Boot) -> Self {
        Self {
            path,
            boot,
            live: Mutex::new(HashMap::new()),
            write: Mutex::new(()),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The entry for a leader this process just spawned.
    pub(crate) fn entry(&self, tag: &str, leader: i32, run_id: Option<&str>) -> LedgerEntry {
        LedgerEntry {
            run_id: run_id.map(str::to_string),
            tag: tag.to_string(),
            pid: leader,
            start_time: crate::proc_info::process_start_micros(leader),
            boot_at: self.boot.at,
            owner_pid: self.boot.pid,
            owner_start: self.boot.start,
        }
    }

    /// Append an entry (replacing one with the same tag).
    pub(crate) fn record(&self, entry: LedgerEntry) {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(entry.tag.clone(), entry.clone());
        self.update(|entries| {
            entries.retain(|e| e.tag != entry.tag);
            entries.push(entry);
        });
    }

    /// Remove the entry with `tag` (its run settled).
    pub(crate) fn forget(&self, tag: &str) {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(tag);
        self.forget_all(&HashSet::from([tag.to_string()]));
    }

    fn forget_all(&self, tags: &HashSet<String>) {
        if tags.is_empty() {
            return;
        }
        self.update(|entries| entries.retain(|e| !tags.contains(&e.tag)));
    }

    /// The tags of this process's live entries (`agent_cli::runs_using`).
    pub(crate) fn live_tags(&self) -> HashSet<String> {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    fn live_entries(&self) -> Vec<LedgerEntry> {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Every valid entry on disk. Never fails: a missing file is empty, an
    /// unreadable one is logged and read as empty.
    pub(crate) fn read(&self) -> Vec<LedgerEntry> {
        let _write = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        read_entries(&self.path)
    }

    /// Read-modify-write under both locks, then an atomic rewrite. Failures
    /// are logged, never returned: a run must not fail because its ledger
    /// entry could not be written.
    fn update(&self, change: impl FnOnce(&mut Vec<LedgerEntry>)) {
        let _write = self.write.lock().unwrap_or_else(PoisonError::into_inner);
        let _flock = FileLock::take(&self.path);
        let mut entries = read_entries(&self.path);
        change(&mut entries);
        if entries.len() > MAX_ENTRIES {
            entries.sort_by_key(|e| e.boot_at);
            let excess = entries.len() - MAX_ENTRIES;
            entries.drain(..excess);
        }
        if let Err(error) = write_entries(&self.path, &entries) {
            log(&format!("could not write {}: {error}", self.path.display()));
        }
    }
}

fn log(message: &str) {
    let _ = writeln!(std::io::stderr().lock(), "[orphans] {message}");
}

fn read_entries(path: &Path) -> Vec<LedgerEntry> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log(&format!("could not read {}: {error}", path.display()));
            return Vec::new();
        }
    };
    let doc: LedgerDoc = match serde_json::from_slice(&bytes) {
        Ok(doc) => doc,
        Err(error) => {
            log(&format!(
                "{} is unreadable ({error}); starting a fresh ledger",
                path.display()
            ));
            return Vec::new();
        }
    };
    doc.entries
        .into_iter()
        .filter_map(|value| serde_json::from_value::<LedgerEntry>(value).ok())
        .filter(LedgerEntry::is_valid)
        .collect()
}

fn write_entries(path: &Path, entries: &[LedgerEntry]) -> Result<(), String> {
    let dir = path.parent().ok_or("the ledger path has no parent")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let doc = LedgerDoc {
        version: LEDGER_VERSION,
        entries: entries
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{LEDGER_FILE}.{}.{nanos}.tmp", std::process::id()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|e| format!("create {}: {e}", tmp.display()))?;
        file.write_all(&bytes).map_err(|e| format!("write: {e}"))?;
        file.sync_all().map_err(|e| format!("sync: {e}"))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// An exclusive `flock` on `{ledger}.lock`, released on drop. Best effort:
/// `None` when it cannot be opened or stays held past [`FLOCK_WAIT`].
struct FileLock(#[allow(dead_code)] std::fs::File);

impl FileLock {
    fn take(ledger: &Path) -> Option<Self> {
        let mut name = ledger.file_name()?.to_os_string();
        name.push(".lock");
        let path = ledger.with_file_name(name);
        std::fs::create_dir_all(path.parent()?).ok()?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let end = std::time::Instant::now() + FLOCK_WAIT;
            loop {
                // SAFETY: flock on a descriptor this function owns.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                    return Some(Self(file));
                }
                if std::time::Instant::now() >= end {
                    log("the run ledger lock stayed held; writing without it");
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        #[cfg(not(unix))]
        Some(Self(file))
    }
}

/// What ending one entry's processes found.
#[derive(Debug, Clone)]
pub(crate) struct EndedEntry {
    pub(crate) entry: LedgerEntry,
    /// The processes identified and signalled (empty: nothing was left).
    pub(crate) pids: Vec<i32>,
    pub(crate) outcome: StopOutcome,
}

/// What the boot sweep did.
#[derive(Debug, Default)]
pub(crate) struct BootSweep {
    pub(crate) ended: Vec<EndedEntry>,
    /// History run ids whose leftovers were stopped.
    pub(crate) stopped_runs: HashSet<String>,
}

/// GUI setup, first thing: install the global ledger and end what earlier,
/// dead Hollow processes left running. Blocks (on a helper thread, outside
/// the async runtime) until the sweep finishes: one file read when the
/// ledger has nothing stale, up to about 6 s when leftovers ignore SIGTERM.
/// Returns the history run ids whose leftovers were stopped.
pub(crate) fn start_at_boot(boot_at: DateTime<Utc>) -> HashSet<String> {
    let dir = match crate::agent_settings::app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            log(&format!("no app data directory, so no run ledger: {error}"));
            return HashSet::new();
        }
    };
    let ledger =
        GLOBAL.get_or_init(|| Ledger::new(dir.join(LEDGER_FILE), Boot::this_process(boot_at)));
    #[cfg(unix)]
    {
        let sweep = std::thread::spawn(move || {
            tauri::async_runtime::block_on(sweep_previous(ledger, &Timings::default()))
        })
        .join();
        match sweep {
            Ok(sweep) => sweep.stopped_runs,
            Err(_) => {
                log("the boot sweep panicked");
                HashSet::new()
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ledger;
        HashSet::new()
    }
}

/// End the processes of every entry whose owner Hollow is gone, then drop
/// those entries. A failed process scan keeps them for the next launch.
#[cfg(unix)]
pub(crate) async fn sweep_previous(ledger: &Ledger, t: &Timings) -> BootSweep {
    sweep_judged(ledger, t, |entry| owner_of(entry, &ledger.boot)).await
}

/// The crash reaper's sweep ([`super::reaper`]): end the runs of the Hollow
/// `parent` that just exited, and nobody else's. `parent_start` is its start
/// stamp, read while it was provably alive (it was still the reaper's
/// parent): an entry with that owner and stamp is swept without a liveness
/// check, which a not-yet-reaped zombie would fail. Without a stamp the
/// boot sweep's judgement applies.
#[cfg(unix)]
pub(crate) async fn sweep_exited(
    ledger: &Ledger,
    parent: i32,
    parent_start: Option<u64>,
    t: &Timings,
) -> BootSweep {
    sweep_judged(ledger, t, |entry| {
        if entry.owner_pid != parent {
            // Another Hollow's entry: not this reaper's to judge.
            Owner::Live
        } else if parent_start.is_some() && entry.owner_start == parent_start {
            Owner::Gone
        } else {
            owner_of(entry, &ledger.boot)
        }
    })
    .await
}

/// End the processes of every entry `judge` finds [`Owner::Gone`], then
/// drop those entries (and unjudgeable ones past the age limit).
#[cfg(unix)]
async fn sweep_judged(
    ledger: &Ledger,
    t: &Timings,
    judge: impl Fn(&LedgerEntry) -> Owner,
) -> BootSweep {
    let now = Utc::now();
    let mut stale = Vec::new();
    let mut expired = HashSet::new();
    for entry in ledger.read() {
        match judge(&entry) {
            Owner::Gone => stale.push(entry),
            Owner::Unknown if (now - entry.boot_at).num_days() >= UNJUDGED_MAX_AGE_DAYS => {
                expired.insert(entry.tag);
            }
            Owner::ThisProcess | Owner::Live | Owner::Unknown => {}
        }
    }
    ledger.forget_all(&expired);
    if stale.is_empty() {
        return BootSweep::default();
    }
    let ended = match end_entries(&stale, t, false).await {
        Ok(ended) => ended,
        Err(error) => {
            log(&format!(
                "could not scan for {} earlier run(s) ({error}); trying again at the next launch",
                stale.len()
            ));
            return BootSweep::default();
        }
    };
    let mut sweep = BootSweep::default();
    for done in &ended {
        if done.pids.is_empty() {
            continue;
        }
        log(&format!(
            "stopped {} process(es) left running by run {} (leader pid {}, Hollow pid {} started {}): pids {:?}, {}",
            done.pids.len(),
            done.entry.run_id.as_deref().unwrap_or("(no run id)"),
            done.entry.pid,
            done.entry.owner_pid,
            done.entry.boot_at.to_rfc3339(),
            done.pids,
            done.outcome.as_str(),
        ));
        if let Some(run_id) = &done.entry.run_id {
            sweep.stopped_runs.insert(run_id.clone());
        }
    }
    ledger.forget_all(&stale.iter().map(|e| e.tag.clone()).collect());
    sweep.ended = ended;
    sweep
}

/// `RunEvent::Exit`: end this process's live runs, bounded by
/// [`QUIT_BUDGET`], and drop their entries. Blocks the caller; runs the
/// async sequence on its own thread, outside the async runtime.
pub(crate) fn settle_at_quit() {
    if let Some(ledger) = global() {
        settle_live(ledger);
    }
}

/// [`settle_at_quit`] for `ledger`.
pub(crate) fn settle_live(ledger: &Ledger) {
    let live = ledger.live_entries();
    if live.is_empty() {
        return;
    }
    #[cfg(unix)]
    {
        let entries = live.clone();
        let settled = std::thread::spawn(move || {
            tauri::async_runtime::block_on(async move {
                tokio::time::timeout(QUIT_BUDGET, end_entries(&entries, &quit_timings(), false))
                    .await
            })
        })
        .join();
        match settled {
            Ok(Ok(Ok(ended))) => {
                let pids: usize = ended.iter().map(|e| e.pids.len()).sum();
                log(&format!(
                    "quit: settled {} live run(s), {pids} process(es)",
                    ended.len()
                ));
                ledger.forget_all(&ended.iter().map(|e| e.entry.tag.clone()).collect());
            }
            Ok(Ok(Err(error))) => log(&format!(
                "quit: could not scan ({error}); the next launch sweeps"
            )),
            Ok(Err(_)) => log("quit: settle ran out of time; the next launch sweeps"),
            Err(_) => log("quit: settle panicked; the next launch sweeps"),
        }
    }
}

/// The quit settle's timings: scans, grace and verification all fit inside
/// [`QUIT_BUDGET`].
pub(crate) fn quit_timings() -> Timings {
    Timings {
        grace: Duration::from_millis(1800),
        verify: Duration::from_millis(300),
        snapshot_timeout: Duration::from_millis(400),
        ..Timings::default()
    }
}

/// The signals that end a process by default (SIGTERM from `kill` or an
/// installer, SIGINT from Ctrl-C under `tauri dev`, SIGHUP when that
/// terminal closes), taken over so a live run is settled first (A·R8).
/// Agent runs lead their own process groups, so none of these reach them.
#[cfg(unix)]
pub(crate) struct QuitSignals {
    term: tokio::signal::unix::Signal,
    int: tokio::signal::unix::Signal,
    /// `None` when Hollow started with SIGHUP ignored (`nohup`): it stays
    /// ignored, so a closing terminal never quits it.
    hup: Option<tokio::signal::unix::Signal>,
}

#[cfg(unix)]
impl QuitSignals {
    /// Replace the default action of all three for the rest of the
    /// process's life, except SIGHUP when it is already ignored (the
    /// person asked for that with `nohup`). Must run inside a Tokio runtime.
    pub(crate) fn install() -> std::io::Result<Self> {
        use tokio::signal::unix::{signal, SignalKind};
        let hup = if signal_ignored(libc::SIGHUP) {
            None
        } else {
            Some(signal(SignalKind::hangup())?)
        };
        Ok(Self {
            term: signal(SignalKind::terminate())?,
            int: signal(SignalKind::interrupt())?,
            hup,
        })
    }

    /// The next signal; `None` once the runtime is shutting down.
    async fn next(&mut self) -> Option<i32> {
        tokio::select! {
            Some(()) = self.term.recv() => Some(libc::SIGTERM),
            Some(()) = self.int.recv() => Some(libc::SIGINT),
            Some(()) = recv_some(self.hup.as_mut()) => Some(libc::SIGHUP),
            else => None,
        }
    }
}

/// The next delivery of a signal that may not be watched; `None` at once
/// when it isn't, which disables its `select!` branch.
#[cfg(unix)]
async fn recv_some(signal: Option<&mut tokio::signal::unix::Signal>) -> Option<()> {
    match signal {
        Some(signal) => signal.recv().await,
        None => None,
    }
}

/// Whether `sig`'s disposition is "ignore" right now. Read before anything
/// installs a handler, so it reflects how Hollow was started.
#[cfg(unix)]
fn signal_ignored(sig: libc::c_int) -> bool {
    // SAFETY: an all-zero `sigaction` is a valid value, and a null new
    // action makes sigaction(2) only read the current one into `old`.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        libc::sigaction(sig, std::ptr::null(), &mut old) == 0 && old.sa_sigaction == libc::SIG_IGN
    }
}

/// Wait for the first quit signal, run `settle` (blocking, off the async
/// workers), then hand the signal to `exit`. A second signal while settling
/// exits at once with `128 + signal`: whatever is left stays in the ledger
/// for the reaper or the next launch.
#[cfg(unix)]
pub(crate) async fn settle_on_signal(
    mut signals: QuitSignals,
    settle: impl FnOnce() + Send + 'static,
    exit: impl FnOnce(i32),
) {
    let Some(first) = signals.next().await else {
        return;
    };
    log(&format!(
        "signal {first}: settling live runs before exiting"
    ));
    let settling = tokio::task::spawn_blocking(settle);
    tokio::select! {
        _ = settling => {}
        Some(again) = signals.next() => {
            log(&format!("signal {again} while settling: exiting now"));
            std::process::exit(128 + again);
        }
    }
    exit(first);
}

/// The recorded leader, pinned by pid and start stamp: `{pid → ps lstart}`
/// only when the pid is alive now with the start time the entry recorded.
#[cfg(unix)]
fn pinned_leader(entry: &LedgerEntry, rows: &[Proc]) -> HashMap<i32, String> {
    let Some(start) = entry.start_time else {
        return HashMap::new();
    };
    if crate::proc_info::process_start_micros(entry.pid) != Some(start) {
        return HashMap::new();
    }
    rows.iter()
        .find(|row| row.pid == entry.pid && !row.zombie)
        .map(|row| HashMap::from([(row.pid, row.start.clone())]))
        .unwrap_or_default()
}

/// What is left of the recorded leader's group once the leader itself is
/// gone: `{pid → ps lstart}` for every live process still in group
/// `entry.pid` that started no earlier than the recorded leader. This is how
/// a platform binary (`/bin/zsh`, `git`, whose environment `ps -E` cannot
/// read) is found after the agent CLI above it died. Empty while any process
/// holds the leader's pid (the leader itself is [`pinned_leader`]'s; anyone
/// else means the id was reused), when no start stamp was recorded, and
/// when [`survivor_window_open`] says no.
///
/// The start-time check alone does not guard against a reused id: every
/// member of a reused group started after the old leader. A leaderless
/// group is common (a double-forked daemon's grandchild, the rest of a
/// shell pipeline), and once the old group empties its id can come back
/// as such a group. So the match is bounded from above too: never across a
/// reboot (after a panic or power loss the ledger keeps entries nobody
/// settled, and everything on the new boot is newer), and never past
/// [`SURVIVOR_WINDOW`] after the leader started.
#[cfg(unix)]
fn group_survivors(entry: &LedgerEntry, rows: &[Proc]) -> HashMap<i32, String> {
    let Some(start) = entry.start_time else {
        return HashMap::new();
    };
    if rows.iter().any(|row| row.pid == entry.pid) {
        return HashMap::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_micros()).ok());
    let open = match (
        crate::proc_info::stamp_unix_micros(start),
        crate::proc_info::boot_unix_micros(),
        now,
    ) {
        (Some(leader), Some(boot), Some(now)) => survivor_window_open(entry, leader, boot, now),
        // Unknown: fail safe, match nothing.
        _ => false,
    };
    if !open {
        return HashMap::new();
    }
    rows.iter()
        .filter(|row| row.pgid == entry.pid && !row.zombie)
        .filter(|row| {
            crate::proc_info::process_start_micros(row.pid).is_some_and(|since| since >= start)
        })
        .map(|row| (row.pid, row.start.clone()))
        .collect()
}

/// Whether a dead leader's group may still hold the run's processes (all
/// times in µs since the epoch): the owning Hollow and the leader both
/// started after this machine booted, and the leader at most
/// [`SURVIVOR_WINDOW`] before `now`.
#[cfg(unix)]
fn survivor_window_open(entry: &LedgerEntry, leader: u64, boot: u64, now: u64) -> bool {
    let owner_after_boot =
        u64::try_from(entry.boot_at.timestamp_micros()).is_ok_and(|owner| owner > boot);
    let window = u64::try_from(SURVIVOR_WINDOW.as_micros()).unwrap_or(u64::MAX);
    owner_after_boot && leader > boot && now.saturating_sub(leader) <= window
}

#[cfg(unix)]
fn all_gone(found: &[(Ours, HashMap<i32, String>)]) -> bool {
    found.iter().all(|(ours, _)| {
        ours.groups.iter().all(|group| !group_alive(*group))
            && ours.members.iter().all(|(pid, _, _)| !pid_alive(*pid))
    })
}

/// SIGTERM → grace → SIGKILL → verify for `entries` at once (one grace
/// period whatever their number). A process is an entry's only by its exact
/// tag, its pinned leader pid + start time ([`pinned_leader`]), or as a
/// survivor of the dead leader's group ([`group_survivors`]); nothing else
/// in the process table is ever signalled. Errors only when the first
/// scan fails (nothing was signalled).
#[cfg(unix)]
pub(crate) async fn end_entries(
    entries: &[LedgerEntry],
    t: &Timings,
    fail: bool,
) -> Result<Vec<EndedEntry>, String> {
    let (me, my_group) = (own_pid(), own_pgrp());
    let first = ps_scan(true, t, fail).await?;
    let found: Vec<(Ours, HashMap<i32, String>)> = entries
        .iter()
        .map(|entry| {
            let rows = first.rows(Some(&entry.tag));
            let mut leader = pinned_leader(entry, &rows);
            leader.extend(group_survivors(entry, &rows));
            let ours = identify(&rows, None, &leader, me, my_group);
            // Pin every member by its start time, so the kill pass signals
            // only these processes (or ones still carrying the tag).
            let mut pinned = leader;
            for (pid, _, start) in &ours.members {
                pinned.insert(*pid, start.clone());
            }
            (ours, pinned)
        })
        .collect();
    let signalled: Vec<Vec<i32>> = found
        .iter()
        .map(|(ours, _)| ours.members.iter().map(|(pid, _, _)| *pid).collect())
        .collect();
    let finish = |outcomes: Vec<StopOutcome>| {
        entries
            .iter()
            .zip(signalled.iter())
            .zip(outcomes)
            .map(|((entry, pids), outcome)| EndedEntry {
                entry: entry.clone(),
                pids: pids.clone(),
                outcome,
            })
            .collect::<Vec<_>>()
    };
    if signalled.iter().all(Vec::is_empty) {
        return Ok(finish(vec![StopOutcome::Verified; entries.len()]));
    }

    for (ours, _) in &found {
        signal_members(ours, None, libc::SIGTERM);
    }
    let grace_end = Instant::now() + t.grace;
    while !all_gone(&found) && Instant::now() < grace_end {
        tokio::time::sleep(t.poll).await;
    }

    let mut scans_ok = true;
    match ps_scan(true, t, fail).await {
        Ok(scan) => {
            for (entry, (_, pinned)) in entries.iter().zip(found.iter()) {
                let now = identify(&scan.rows(Some(&entry.tag)), None, pinned, me, my_group);
                signal_members(&now, None, libc::SIGKILL);
            }
        }
        Err(_) => {
            scans_ok = false;
            // A live group id cannot have been reused, so the groups found
            // before are still safe to signal; single pids are not.
            for (ours, _) in &found {
                for group in &ours.groups {
                    if group_alive(*group) {
                        signal_group(*group, libc::SIGKILL);
                    }
                }
            }
        }
    }

    let verify_end = Instant::now() + t.verify;
    while !all_gone(&found) && Instant::now() < verify_end {
        tokio::time::sleep(t.poll).await;
    }
    let outcomes = match ps_scan(true, t, fail).await {
        Ok(scan) => entries
            .iter()
            .zip(found.iter())
            .map(|(entry, (_, pinned))| {
                let left = identify(&scan.rows(Some(&entry.tag)), None, pinned, me, my_group);
                if scans_ok && left.members.is_empty() {
                    StopOutcome::Verified
                } else {
                    StopOutcome::Unconfirmed
                }
            })
            .collect(),
        Err(_) => vec![StopOutcome::Unconfirmed; entries.len()],
    };
    Ok(finish(outcomes))
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::test_support::*;
    use super::super::{isolate, GroupGuard, RunTag, RUN_TAG_ENV};
    use super::*;
    use tokio::process::{Child, Command};

    fn temp_ledger(dir: &tempfile::TempDir, boot: Boot) -> &'static Ledger {
        Box::leak(Box::new(Ledger::new(dir.path().join(LEDGER_FILE), boot)))
    }

    fn now_boot() -> Boot {
        Boot::this_process(Utc::now())
    }

    /// A Hollow process that has exited: a reaped child's pid.
    async fn dead_boot() -> Boot {
        let mut child = Command::new("/usr/bin/true").spawn().expect("spawn true");
        let pid = child.id().unwrap() as i32;
        child.wait().await.unwrap();
        Boot {
            at: Utc::now() - chrono::Duration::minutes(5),
            pid,
            start: Some(1),
        }
    }

    fn helper_env(exe: &str) -> [(&str, &str); 2] {
        [("HOLLOW_TEST_EXE", exe), ("HOLLOW_TEST_SLEEP_SECS", "60")]
    }

    /// The tagged helper, detached into its own group and reparented to
    /// launchd, guarded by a guard adopted into `ledger`.
    async fn tagged_orphan(ledger: &'static Ledger, run_id: &str) -> (GroupGuard, Child, i32) {
        let exe = self_exe();
        let (cmd, mut child) = spawn_isolated(TAGGED_ORPHAN_SCRIPT, &helper_env(&exe));
        let orphan: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let guard = GroupGuard::adopt_in(&cmd, &child, Some(run_id), Some(ledger));
        assert!(wait_exec(orphan, &self_exe_name(), Duration::from_secs(10)).await);
        child.wait().await.unwrap();
        (guard, child, orphan)
    }

    /// The same helper script, detached, with no run tag at all.
    async fn untagged_orphan() -> i32 {
        let exe = self_exe();
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(TAGGED_ORPHAN_SCRIPT)
            .env_remove(RUN_TAG_ENV)
            .envs(helper_env(&exe))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        cmd.process_group(0);
        let mut child = cmd.spawn().expect("spawn untagged helper");
        let pid: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        assert!(wait_exec(pid, &self_exe_name(), Duration::from_secs(10)).await);
        child.wait().await.unwrap();
        pid
    }

    fn cleanup(pids: &[i32]) {
        for pid in pids {
            kill_now(*pid);
        }
    }

    /// `setup` and the run loop need an AppHandle, so the wiring is pinned
    /// in source: the GUI's setup sweeps, then starts core startup
    /// (`core_boot`), whose phases start the reaper and then the vault's
    /// services. Needles are split so this test's text never matches.
    #[test]
    fn boot_sweep_and_quit_settle_are_wired() {
        let lib = include_str!("../lib.rs");
        let boot = include_str!("../core_boot.rs");
        let at = |needle: &str| {
            lib.find(needle)
                .unwrap_or_else(|| panic!("lib.rs no longer has {needle}"))
        };
        let in_boot = |needle: &str| {
            boot.find(needle)
                .unwrap_or_else(|| panic!("core_boot.rs no longer has {needle}"))
        };
        let setup = at(concat!(".setup(", "move |app|"));
        let sweep = at(concat!("orphans::", "start_at_boot(boot_at)"));
        let first_phase = at(concat!("core_boot::", "process_services("));
        let vault_phase = at(concat!("core_boot::", "vault_services("));
        assert!(
            setup < sweep && sweep < first_phase && first_phase < vault_phase,
            "in setup, before core startup"
        );
        let vault_gate = in_boot(concat!("if vault_path", ".exists() {"));
        assert!(
            vault_gate
                < in_boot(concat!(
                    "automated_history::reconcile",
                    "_interrupted_sparing("
                ))
        );
        assert!(vault_gate < in_boot(concat!("usage_ledger::", "reconcile_interrupted(")));
        assert!(vault_gate < in_boot(concat!("run_pump::", "start(")));
        assert!(
            boot.contains(concat!("boot_at, ", "stopped_leftovers)"))
                && lib.contains(concat!("boot_at, ", "stopped_leftovers)")),
            "the stopped runs reach the queue"
        );
        // A quit and a quit signal settle through one once-only function
        // (A·R8), and setup starts the signal watcher and the reaper.
        let pin = |needle: &str| {
            crate::source_pins::find(lib, needle)
                .unwrap_or_else(|| panic!("lib.rs no longer has {needle}"))
        };
        let body = |needle: &str| {
            let start = pin(needle);
            let end = start + lib[start..].find("\n}\n").expect("end of fn");
            &lib[start..end]
        };
        let has = |hay: &str, needle: &str| crate::source_pins::contains(hay, needle);
        // The Exit arm first notes the quit for the freeze watchdog (OC10, so a
        // clean exit is never read as a hang), then settles through the same
        // once-only function.
        let exit = pin(concat!(
            "tauri::RunEvent::",
            "Exit => { freeze_watchdog::note_exit(); stop_children_once() }"
        ));
        let once = body(concat!("fn stop_children", "_once() {"));
        assert!(has(
            once,
            concat!("thread::spawn(process_group::orphans::", "settle_at_quit)")
        ));
        assert!(has(once, concat!("services::", "stop_all_on_exit()")));
        let watcher = body(concat!(
            "fn settle_on_quit",
            "_signals(handle: tauri::AppHandle)"
        ));
        assert!(has(
            watcher,
            concat!(
                "orphans::",
                "settle_on_signal(signals, stop_children_once, exit)"
            )
        ));
        assert!(has(watcher, concat!("handle.", "exit(128 + signal)")));
        let started = pin(concat!("settle_on_quit", "_signals(app.handle().clone());"));
        assert!(
            sweep < started && started < first_phase,
            "after the boot sweep"
        );
        let reaper = crate::source_pins::find(boot, concat!("process_group::reaper::", "spawn();"))
            .expect("core_boot.rs spawns the reaper");
        let process_phase = in_boot(concat!("pub(crate) fn ", "process_services("));
        assert!(
            process_phase < reaper && reaper < vault_gate,
            "in the first phase, not vault-gated"
        );
        assert!(exit > vault_phase);
        let main = include_str!("../main.rs");
        assert!(has(
            main,
            concat!("Some(\"reaper\") => return hollow_lib::", "run_reaper()")
        ));
        assert_eq!(super::super::reaper::SUBCOMMAND, "reaper");
        let services = include_str!("../services.rs");
        assert!(services.contains(concat!("GroupGuard::", "adopt_unrecorded(")));
    }

    #[tokio::test]
    async fn boot_sweep_ends_a_dead_hollows_tagged_run_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        // The earlier Hollow: it adopted the run, then died without settling
        // (forget the guard: no settle, no Drop).
        let earlier = temp_ledger(&dir, dead_boot().await);
        let (guard, _child, orphan) = tagged_orphan(earlier, "run-left-behind").await;
        std::mem::forget(guard);
        // Look-alikes that must survive: the same helper with no tag, and
        // one carrying a tag this ledger never recorded.
        let untagged = untagged_orphan().await;
        let exe = self_exe();
        let (_other_cmd, mut other_child) = spawn_isolated(TAGGED_ORPHAN_SCRIPT, &helper_env(&exe));
        let other: i32 = read_line(&mut other_child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        assert!(wait_exec(other, &self_exe_name(), Duration::from_secs(10)).await);
        other_child.wait().await.unwrap();

        let recorded = earlier.read();
        let this = temp_ledger(&dir, now_boot());
        let sweep = sweep_previous(this, &Timings::fast_for_tests()).await;
        let gone = wait_gone(orphan, Duration::from_secs(3)).await;
        let untagged_alive = is_running(untagged).await;
        let other_alive = is_running(other).await;
        let left = this.read();
        cleanup(&[orphan, untagged, other]);

        assert_eq!(recorded.len(), 1, "adopt recorded the run");
        assert_eq!(recorded[0].run_id.as_deref(), Some("run-left-behind"));
        assert!(gone, "the tagged leftover was ended");
        assert!(untagged_alive, "an untagged look-alike survives");
        assert!(other_alive, "a process with an unrecorded tag survives");
        assert_eq!(
            sweep.stopped_runs,
            HashSet::from(["run-left-behind".to_string()])
        );
        assert_eq!(sweep.ended.len(), 1);
        assert!(sweep.ended[0].pids.contains(&orphan));
        assert_eq!(sweep.ended[0].outcome, StopOutcome::Verified);
        assert!(left.is_empty(), "the swept entry is removed");
    }

    #[tokio::test]
    async fn this_boots_entries_and_a_live_owners_are_never_swept() {
        let dir = tempfile::tempdir().unwrap();
        let boot = now_boot();
        let ledger = temp_ledger(&dir, boot.clone());
        let (guard, _child, orphan) = tagged_orphan(ledger, "run-live").await;
        // A second ledger object for the same process and boot (the sweep's
        // view) must still read the entry as this process's own.
        let same = temp_ledger(&dir, boot);
        let sweep = sweep_previous(same, &Timings::fast_for_tests()).await;
        let alive = is_running(orphan).await;
        let kept = same.read();
        std::mem::forget(guard);
        cleanup(&[orphan]);

        assert!(alive, "an entry from this boot is not swept");
        assert!(sweep.ended.is_empty() && sweep.stopped_runs.is_empty());
        assert_eq!(kept.len(), 1, "and it stays in the ledger");
        assert_eq!(owner_of(&kept[0], &now_boot()), Owner::ThisProcess);
    }

    #[tokio::test]
    async fn a_reused_pid_with_another_start_time_is_not_killed() {
        let dir = tempfile::tempdir().unwrap();
        // A platform binary (its environment is unreadable), so only the
        // pid + start time could ever match it.
        let mut cmd = Command::new("/bin/sleep");
        cmd.arg("60").kill_on_drop(true);
        cmd.process_group(0);
        let mut sleeper = cmd.spawn().unwrap();
        let pid = sleeper.id().unwrap() as i32;
        let start = crate::proc_info::process_start_micros(pid).expect("start stamp");
        let earlier = dead_boot().await;
        let entry = |start_time| LedgerEntry {
            run_id: Some("run-reused".into()),
            tag: RunTag::new().as_str().to_string(),
            pid,
            start_time: Some(start_time),
            boot_at: earlier.at,
            owner_pid: earlier.pid,
            owner_start: earlier.start,
        };
        let this = temp_ledger(&dir, now_boot());
        this.update(|entries| entries.push(entry(start + 1_000_000)));
        let sweep = sweep_previous(this, &Timings::fast_for_tests()).await;
        let survived = matches!(sleeper.try_wait(), Ok(None));
        let dropped = this.read().is_empty();

        // The same pid WITH its real start time is the recorded leader.
        this.update(|entries| entries.push(entry(start)));
        let matched = sweep_previous(this, &Timings::fast_for_tests()).await;
        let ended = tokio::time::timeout(Duration::from_secs(3), sleeper.wait())
            .await
            .is_ok();
        if !ended {
            let _ = sleeper.start_kill();
            let _ = sleeper.wait().await;
        }

        assert!(survived, "a pid whose start time differs is not ours");
        assert!(sweep.stopped_runs.is_empty());
        assert!(dropped, "the stale entry is dropped all the same");
        assert!(ended, "pid + start time match ends the recorded leader");
        assert_eq!(
            matched.stopped_runs,
            HashSet::from(["run-reused".to_string()])
        );
    }

    #[tokio::test]
    async fn a_corrupt_or_missing_ledger_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = temp_ledger(&dir, now_boot());
        // Missing.
        assert!(ledger.read().is_empty());
        assert!(sweep_previous(ledger, &Timings::fast_for_tests())
            .await
            .ended
            .is_empty());
        // Garbage.
        std::fs::write(ledger.path(), b"{not json").unwrap();
        assert!(ledger.read().is_empty());
        assert!(sweep_previous(ledger, &Timings::fast_for_tests())
            .await
            .ended
            .is_empty());
        // One bad entry among good ones: only it is dropped.
        let good = ledger.entry(RunTag::new().as_str(), 424242, Some("run-ok"));
        let doc = serde_json::json!({
            "version": 1,
            "entries": [
                serde_json::to_value(&good).unwrap(),
                { "tag": "not-hex", "pid": 5, "bootAt": Utc::now(), "ownerPid": 1 },
                { "tag": "HOLLOW_RUN_TAG=", "pid": "x" },
                { "tag": "", "pid": 0, "bootAt": Utc::now(), "ownerPid": 1 },
            ],
        });
        std::fs::write(ledger.path(), serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(ledger.read(), vec![good.clone()]);
        // A write after corruption leaves a valid file.
        std::fs::write(ledger.path(), b"\0\0garbage").unwrap();
        ledger.record(good.clone());
        let text = std::fs::read_to_string(ledger.path()).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
        assert_eq!(ledger.read(), vec![good.clone()]);
        ledger.forget(&good.tag);
        assert!(ledger.read().is_empty());
    }

    #[tokio::test]
    async fn settling_removes_the_entry_and_a_failed_scan_keeps_stale_ones() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = temp_ledger(&dir, now_boot());
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("exit 0").kill_on_drop(true);
        let tag = isolate(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let mut guard = GroupGuard::adopt_in(&cmd, &child, Some("run-settled"), Some(ledger));
        let recorded = ledger.read();
        let _ = guard.settle(&mut child, &Timings::fast_for_tests()).await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].tag, tag.as_str());
        assert!(ledger.read().is_empty(), "settle removes the entry");
        assert!(ledger.live_entries().is_empty());

        // A stale entry whose scan fails is kept for the next launch.
        let earlier = dead_boot().await;
        let stale = LedgerEntry {
            run_id: None,
            tag: RunTag::new().as_str().to_string(),
            pid: 999_999,
            start_time: None,
            boot_at: earlier.at,
            owner_pid: earlier.pid,
            owner_start: earlier.start,
        };
        ledger.update(|entries| entries.push(stale.clone()));
        assert!(end_entries(
            std::slice::from_ref(&stale),
            &Timings::fast_for_tests(),
            true
        )
        .await
        .is_err());
        let ended = end_entries(
            std::slice::from_ref(&stale),
            &Timings::fast_for_tests(),
            false,
        )
        .await
        .unwrap();
        assert!(ended[0].pids.is_empty(), "nothing carries a fresh tag");
        assert_eq!(owner_of(&stale, &now_boot()), Owner::Gone);
    }

    /// A platform binary left in the recorded leader's group after the
    /// leader died: its environment (and so its tag) cannot be read and the
    /// leader pid is gone, so only the recorded group finds it (A·R8).
    #[tokio::test]
    async fn boot_sweep_ends_what_is_left_of_a_dead_leaders_group() {
        let dir = tempfile::tempdir().unwrap();
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("/bin/sleep 60 >/dev/null 2>&1 & echo $!; read _")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        cmd.process_group(0);
        let mut leader = cmd.spawn().expect("spawn the leader");
        let leader_pid = leader.id().unwrap() as i32;
        let left: i32 = read_line(&mut leader, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let start = crate::proc_info::process_start_micros(leader_pid).expect("leader start");
        drop(leader.stdin.take());
        leader.wait().await.unwrap();
        let in_group = pgid_of(left) == leader_pid;
        let earlier = dead_boot().await;
        let boot = crate::proc_info::boot_unix_micros().expect("boot time");
        let entry_at = |start_time, boot_at| LedgerEntry {
            run_id: Some("run-group-left".into()),
            // A tag nothing carries: only the group can match.
            tag: RunTag::new().as_str().to_string(),
            pid: leader_pid,
            start_time: Some(start_time),
            boot_at,
            owner_pid: earlier.pid,
            owner_start: earlier.start,
        };
        let entry = |start_time| entry_at(start_time, earlier.at);
        let this = temp_ledger(&dir, now_boot());
        // A leader recorded as starting after the survivor: the group id
        // would have been reused, so the survivor is not the run's.
        this.update(|entries| entries.push(entry(start + 3_600_000_000)));
        let refused = sweep_previous(this, &Timings::fast_for_tests()).await;
        let survived = is_running(left).await;
        // Entries from before this boot (a panic or power loss settled
        // nothing): everything alive now is newer, so a group with the
        // recorded id is a reused one. (Start stamps are epoch µs on macOS.)
        let hour = 3_600_000_000;
        let before_boot = DateTime::from_timestamp_micros((boot - 24 * hour) as i64).unwrap();
        this.update(|entries| entries.push(entry(boot - hour)));
        this.update(|entries| entries.push(entry_at(start, before_boot)));
        // And a leader older than the survivor window.
        this.update(|entries| entries.push(entry(start - 7 * hour)));
        let too_old = sweep_previous(this, &Timings::fast_for_tests()).await;
        let survived_old = is_running(left).await;
        this.update(|entries| entries.push(entry(start)));
        let sweep = sweep_previous(this, &Timings::fast_for_tests()).await;
        let gone = wait_gone(left, Duration::from_secs(3)).await;
        let remaining = this.read();
        cleanup(&[left]);

        assert!(in_group, "the survivor sits in the recorded group");
        assert!(survived, "a group member older than the leader is not ours");
        assert!(refused.stopped_runs.is_empty());
        assert!(
            survived_old,
            "a pre-boot or out-of-window leader's group is never signalled"
        );
        assert!(too_old.stopped_runs.is_empty());
        assert!(gone, "the dead leader's group survivor was ended");
        assert_eq!(
            sweep.stopped_runs,
            HashSet::from(["run-group-left".to_string()])
        );
        assert!(sweep.ended[0].pids.contains(&left));
        assert!(remaining.is_empty(), "the swept entry is removed");
    }

    #[test]
    fn group_survivors_never_cross_a_reboot_or_the_window() {
        let hour: u64 = 3_600_000_000;
        let boot: u64 = 1_800_000_000_000_000;
        let now = boot + 48 * hour;
        let at = |micros: u64| DateTime::from_timestamp_micros(micros as i64).unwrap();
        let entry = |owner: u64| LedgerEntry {
            run_id: None,
            tag: "0".repeat(32),
            pid: 4242,
            start_time: None,
            boot_at: at(owner),
            owner_pid: 4241,
            owner_start: None,
        };
        let open = |owner, leader| survivor_window_open(&entry(owner), leader, boot, now);
        let leader = now - hour;
        assert!(open(leader - hour, leader), "a recent run on this boot");
        assert!(open(boot + 1, now - 6 * hour), "the window's edge");
        assert!(!open(boot + 1, now - 6 * hour - 1), "past the window");
        assert!(
            !open(boot - hour, boot - 1),
            "a leader from before the boot"
        );
        assert!(!open(boot - hour, leader), "an owner from before the boot");
        assert!(!open(boot, leader), "an owner stamped at the boot itself");
    }

    /// The ledger the helper processes below use.
    const LEDGER_ENV: &str = "HOLLOW_TEST_LEDGER";
    /// The pid the reaper helper watches.
    const PARENT_ENV: &str = "HOLLOW_TEST_REAPER_PARENT";
    /// The line a helper prints once its run is live.
    const READY: &str = "HELPER_READY";

    /// The test binary running one ignored helper test, in its own group.
    fn helper_process(test: &str, ledger: &Path) -> Child {
        let exe = self_exe();
        let mut cmd = Command::new(&exe);
        cmd.args([
            "--exact",
            test,
            "--ignored",
            "--test-threads=1",
            "--nocapture",
        ])
        .env("HOLLOW_TEST_EXE", &exe)
        .env(LEDGER_ENV, ledger)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
        cmd.process_group(0);
        cmd.spawn().expect("spawn a helper process")
    }

    /// What follows [`READY`] on the helper's stdout (the test harness
    /// prints its own lines first, and `test <name> ... ` on the same line).
    async fn read_ready(child: &mut Child, within: Duration) -> String {
        use tokio::io::AsyncBufReadExt as _;
        let stdout = child.stdout.take().expect("piped stdout");
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        let ready = tokio::time::timeout(within, async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some((_, rest)) = line.split_once(READY) {
                    return rest.trim().to_string();
                }
            }
            panic!("the helper ended before it was ready");
        })
        .await
        .expect("the helper got ready in time");
        child.stdout = Some(lines.into_inner().into_inner());
        ready
    }

    /// Helper process: a Hollow with a live run that gets a quit signal.
    #[tokio::test]
    #[ignore = "helper process for the quit-signal test"]
    async fn quit_signal_helper() {
        let Ok(path) = std::env::var(LEDGER_ENV) else {
            return;
        };
        let ledger: &'static Ledger = Box::leak(Box::new(Ledger::new(path.into(), now_boot())));
        let signals = QuitSignals::install().expect("take over the quit signals");
        let (guard, _child, orphan) = tagged_orphan(ledger, "run-on-sigterm").await;
        std::mem::forget(guard);
        println!("{READY} {orphan}");
        let settle = move || settle_live(ledger);
        settle_on_signal(signals, settle, |signal| std::process::exit(128 + signal)).await;
    }

    /// Helper process: a Hollow started under `nohup` (SIGHUP ignored).
    #[tokio::test]
    #[ignore = "helper process for the nohup test"]
    async fn nohup_helper() {
        if std::env::var(LEDGER_ENV).is_err() {
            return;
        }
        // SAFETY: setting a disposition to "ignore" touches no memory; it is
        // what `nohup` does before it execs the program.
        unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
        let signals = QuitSignals::install().expect("take over the quit signals");
        println!("{READY} up");
        settle_on_signal(signals, || {}, |signal| std::process::exit(128 + signal)).await;
    }

    /// Wave 2 review nit: a Hollow started with SIGHUP ignored keeps it
    /// ignored (a closing terminal doesn't quit it), while SIGTERM still
    /// settles and exits.
    #[tokio::test]
    async fn sighup_stays_ignored_when_hollow_started_ignoring_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEDGER_FILE);
        let mut hollow = helper_process("process_group::orphans::tests::nohup_helper", &path);
        read_ready(&mut hollow, Duration::from_secs(20)).await;
        let pid = hollow.id().unwrap() as i32;
        // SAFETY: signalling the helper this test started.
        unsafe { libc::kill(pid, libc::SIGHUP) };
        let hung_up = tokio::time::timeout(Duration::from_millis(700), hollow.wait()).await;
        assert!(hung_up.is_err(), "SIGHUP quit it: {hung_up:?}");
        // SAFETY: same helper, still running.
        unsafe { libc::kill(pid, libc::SIGTERM) };
        let status = tokio::time::timeout(Duration::from_secs(10), hollow.wait())
            .await
            .expect("exits")
            .expect("waited");
        assert_eq!(status.code(), Some(128 + libc::SIGTERM), "{status:?}");
    }

    /// A SIGTERM no longer kills Hollow before its runs are settled (A·R8):
    /// the live run's processes are ended and its entry removed, then
    /// Hollow exits through the settle (code 128 + 15, not death by signal).
    #[tokio::test]
    async fn a_quit_signal_settles_live_runs_before_exiting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEDGER_FILE);
        let mut hollow = helper_process("process_group::orphans::tests::quit_signal_helper", &path);
        let orphan: i32 = read_ready(&mut hollow, Duration::from_secs(20))
            .await
            .parse()
            .unwrap();
        let recorded = Ledger::new(path.clone(), now_boot()).read();
        let alive_before = is_running(orphan).await;
        // SAFETY: signalling the helper this test started.
        unsafe { libc::kill(hollow.id().unwrap() as i32, libc::SIGTERM) };
        let status = tokio::time::timeout(Duration::from_secs(10), hollow.wait()).await;
        let gone = wait_gone(orphan, Duration::from_secs(1)).await;
        let left = Ledger::new(path, now_boot()).read();
        cleanup(&[orphan]);

        let status = status.expect("exits").expect("waited");
        assert_eq!(recorded.len(), 1, "the run was recorded");
        assert!(alive_before, "the run was live when the signal came");
        assert_eq!(
            status.code(),
            Some(128 + libc::SIGTERM),
            "exited through the settle, not killed by the signal: {status:?}"
        );
        assert!(gone, "the run's processes were ended before Hollow exited");
        assert!(left.is_empty(), "and its entry removed");
    }

    /// Helper process: a Hollow with a live run and a reaper, that crashes.
    #[tokio::test]
    #[ignore = "helper process for the crash reaper test"]
    async fn crashing_hollow_helper() {
        let Ok(path) = std::env::var(LEDGER_ENV) else {
            return;
        };
        let ledger: &'static Ledger =
            Box::leak(Box::new(Ledger::new(PathBuf::from(&path), now_boot())));
        let (guard, _child, orphan) = tagged_orphan(ledger, "run-on-crash").await;
        std::mem::forget(guard);
        let mut cmd = std::process::Command::new(self_exe());
        cmd.args(["--exact", "process_group::orphans::tests::reaper_helper"])
            .args(["--ignored", "--test-threads=1"])
            .env(PARENT_ENV, std::process::id().to_string())
            .env(LEDGER_ENV, &path);
        let reaper = super::super::reaper::launch(cmd).expect("start the reaper");
        println!("{READY} {orphan} {reaper}");
        // Let the reaper reach its wait, then crash: no settle, no drop.
        tokio::time::sleep(Duration::from_millis(500)).await;
        // SAFETY: this helper process ending itself.
        unsafe { libc::kill(std::process::id() as i32, libc::SIGKILL) };
    }

    /// Helper process: the crash reaper, as `hollow reaper` runs it.
    #[test]
    #[ignore = "helper process for the crash reaper test"]
    fn reaper_helper() {
        let parent = std::env::var(PARENT_ENV);
        let (Ok(parent), Ok(path)) = (parent, std::env::var(LEDGER_ENV)) else {
            return;
        };
        super::super::reaper::run(parent.parse().expect("parent pid"), path.into());
    }

    /// A crash runs no code, so the reaper ends the crashed Hollow's runs
    /// right away instead of leaving them to the next launch (A·R8).
    #[tokio::test]
    async fn a_crashed_hollows_reaper_ends_its_runs() {
        use std::os::unix::process::ExitStatusExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LEDGER_FILE);
        let mut hollow = helper_process(
            "process_group::orphans::tests::crashing_hollow_helper",
            &path,
        );
        let ready = read_ready(&mut hollow, Duration::from_secs(20)).await;
        let pids: Vec<i32> = ready
            .split_whitespace()
            .map(|pid| pid.parse().unwrap())
            .collect();
        let (orphan, reaper) = (pids[0], pids[1]);
        let recorded = Ledger::new(path.clone(), now_boot()).read();
        let status = tokio::time::timeout(Duration::from_secs(10), hollow.wait()).await;
        let gone = wait_gone(orphan, Duration::from_secs(10)).await;
        let reaper_done = wait_gone(reaper, Duration::from_secs(10)).await;
        let left = Ledger::new(path, now_boot()).read();
        cleanup(&[orphan, reaper]);

        let status = status.expect("ends").expect("waited");
        assert_eq!(status.signal(), Some(libc::SIGKILL), "a crash: {status:?}");
        assert_eq!(recorded.len(), 1, "the run was recorded");
        assert!(gone, "the reaper ended the crashed Hollow's run");
        assert!(reaper_done, "and then exited");
        assert!(left.is_empty(), "removing the run's entry");
    }
}
