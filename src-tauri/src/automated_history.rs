//! Content-addressed history for files changed while an automated Hollow run
//! is active.
//!
//! A run snapshots every eligible vault file before the agent starts, then
//! compares that baseline with the vault when the run finishes. Only the
//! resulting create/modify/delete set is retained in the final manifest. File
//! bytes live in a SHA-256 object store, so unchanged content is shared across
//! runs and a restore never depends on a mutable source file.
//!
//! History is deliberately hidden under `.hollow-history/`; the vault walker
//! and watcher already ignore dot-prefixed paths. All manifests and objects
//! are written through the vault's atomic-write primitive. Restores use a
//! compare-and-swap check against the recorded after-version and create their
//! own history run before touching the target, making a restore reversible.
//!
//! **Admission.** One O_EXCL lease file (`.hollow-history/active-run.nosync`; `active-run` before night ST6) excludes
//! every full-vault writer, across processes: agent runs (chat, workflow,
//! heartbeat, ingest, history restore) record their run id in it, background
//! writers record an owner prefix (`tracker-sync-`, `workflow-scheduler-`), and
//! a history clear records `history-clear`. [`try_begin_run`] is the only
//! admission point: it either takes the lease or returns
//! [`BeginRunError::Busy`] naming the [`LeaseHolder`]. [`lease_holder`] only
//! *names* the current holder. It is advisory and racy by nature, so callers
//! may use it to word a refusal or to skip work, never to decide to spawn.
//!
//! **Protected-change flags (Paperclip PR 5).** Every run [`try_begin_run`]
//! admits (chat, workflow, heartbeat and ingest, since [`try_begin_owned`]
//! and [`begin_run`] delegate to it) is tracked in memory by
//! [`crate::run_guard`] from inside the admission itself; history restores
//! go through `begin_run_inner` and stay untracked. At finish the run's
//! [`HistoryRun::flags`] are computed from that in-memory baseline and the
//! live disk, never from the agent-writable manifest. The public
//! [`finish_run`] and [`reconcile_interrupted`] then notify after releasing
//! `HISTORY_LOCK` (`run_guard::after_finish`). Lock order: `HISTORY_LOCK` →
//! `workflow_permissions::write_guard` → run_guard `TRACKED`.
//!
//! **Owned leases and release wake-ups (Paperclip PR 7).** [`try_begin_owned`]
//! wraps [`try_begin_run`] (so PR 5's tracking still applies) in a
//! [`RunLease`]: not `Clone`, only mintable here, and finalized as
//! `cancelled` if it is dropped unfinished, so an admitted run can never leak
//! the lease across a fallible step. [`lease_is`] checks at call time that
//! the lease file still names that run. Every removal of the lease file
//! ([`release_run_lease`], a successful [`clear_history`], startup's
//! `clear_stale_run_lease`) notifies [`lease_released`], so the run-queue
//! pump waiting on a holder wakes instead of polling (at most once per
//! 250 ms, `run_pump::busy_wait`, S14); `notify_one` never blocks, so
//! notifying under `HISTORY_LOCK` is safe.
//!
//! **Lease liveness (Paperclip PR 18).** The lease file records a
//! [`LeaseRecord`], one line of JSON: `{"owner","pid","startedAt"}`, where
//! `startedAt` is the owning PROCESS's start stamp
//! ([`crate::proc_info::own_start_micros`]). [`parse_lease`] still accepts
//! the legacy bare-owner text. While it holds a lease this process also
//! keeps a `flock` on the vault's lease lock ([`crate::lease_lock`], outside
//! the vault) in `HELD_OWNERS`, released after the lease file is gone. A
//! lease is *reclaimable* (its owner exited) only when ALL of these hold
//! ([`owner_exited`]): this process holds nothing on the vault (no
//! `HELD_OWNERS` entry and no `run_guard` tracking), the lease lock is free,
//! and the recorded pid is > 1, not ours, and either gone or started at a
//! different time (pid reuse). A legacy record, an unreadable start, a
//! permission error or an unreadable lock never is. The pid alone is never
//! trusted, because the vault file is agent-writable: an agent that rewrites
//! its own run's lease with a dead pid is refused by the first two guards.
//! "Reclaimable" is deliberately a different word from PR 7b's *dead* lease
//! holder ([`dead_lease_holder`]: a lease that stays `Stale` or `Unknown`
//! and only a restart clears); a reclaimable lease reads as free there too.
//!
//! [`lease_holder`] and [`dead_lease_holder`] report a reclaimable lease as
//! free (`None`), so the pump, the heartbeat and Run now go on to admit, and
//! the admission reclaims it (`reclaim_locked`, under `HISTORY_LOCK`, at the
//! top of `begin_run_inner`, `background_lease` and [`clear_history`]): a
//! run owner whose manifest is still running is finalized as `interrupted`
//! (reconcile mode; safe because the first guard proved this process does
//! not track it), anything else just has its lease removed. A reclaim is
//! exclusive across processes: it takes the lease lock BEFORE touching the
//! file, re-reads the lease under it, and hands the same lock to the new
//! lease (`acquire_lease_file_with`), so two processes racing to reclaim one
//! dead lease can never unlink each other's fresh lease. A headless process
//! (no notification app) never finalizes a run, so PR 5's one notification
//! per flagged run is never lost; it leaves those for the GUI. What a reclaim must announce (the flags notification, one
//! health record, closing the dead run's `running` usage rows) is queued in
//! `RECLAIM_NOTICES` and drained per vault by each public entry point after
//! it drops `HISTORY_LOCK`. Startup reconciliation spares only a run whose
//! lease lock another live process holds ([`foreign_live_owner`]). A crash
//! still leaves a legacy lease, a live owner's lease or one whose lock
//! cannot be read to the normal startup recovery path.
//!
//! **Lock order** (every PR that adds a lock extends this paragraph, per the
//! Milestone 3/4 cross-PR analysis M12): `HISTORY_LOCK` →
//! `workflow_permissions::write_guard` → {`vault::MUTATE_LOCK`, the stamp
//! store, the review queue} → run_guard `TRACKED`. [`with_live_run`]
//! (Paperclip PR 12) takes `HISTORY_LOCK` → {`vault::MUTATE_LOCK`,
//! `vault_tasks::CREATE_LOCK`, the review queue's `QUEUE_LOCK`} → run_guard
//! `TRACKED`, and never the permission write guard. The object store flock
//! (Q-R9, [`objects_in_flight`] shared, object cleanup exclusive) is taken
//! under `HISTORY_LOCK`, never before it; both waits are bounded, and
//! cleanup never runs on a thread that holds it shared. `HELD_OWNERS`,
//! `RECLAIM_NOTICES` and `RELEASE_NOTICES` are leaves: taken briefly, never
//! while calling out. Nothing takes `HISTORY_LOCK` while holding the vault
//! index, search-index or embed-index locks. Work that takes the session
//! store's `WRITE_LOCK` (usage rows), writes notifications or verifies
//! workflows runs after `HISTORY_LOCK` is dropped (`drain_reclaim_notices`,
//! `Finalized::notify`), never under it.
//!
//! **Writes on a live run's behalf (Paperclip PR 12).** [`with_live_run`]
//! re-classifies the lease under `HISTORY_LOCK` and runs its closure while
//! still holding it, so a write made for a run by another component is either
//! inside that run's finalizing diff or refused (lock order: above). For a
//! scoped run, [`with_live_run_touching`] (chat concurrency F7, 2026-10-20)
//! snapshots only the folders the write names instead of the whole vault.

use crate::flock::HeldLock;
use crate::lease_lock::LockProbe;
use crate::run_guard::FlagMode;
use chrono::{DateTime, Duration, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const HISTORY_DIR: &str = ".hollow-history";
const OBJECTS_DIR: &str = "objects";
const RUNS_DIR: &str = "runs";
/// The lease file. `.nosync` keeps iCloud Drive from syncing it (night ST6,
/// 2026-10-14): it is created and removed on every run, and in an
/// iCloud-synced vault that left a conflict copy (`active-run 2` …
/// `active-run 71`) on almost every run. The exclusion itself is unchanged.
const ACTIVE_RUN_FILE: &str = "active-run.nosync";
/// The lease's name before night ST6. An older Hollow build on the same vault
/// still takes this one; see [`settle_legacy_lease`].
const LEGACY_ACTIVE_RUN_FILE: &str = "active-run";
/// The snapshot stat cache (OB09): path + stat → hash, so an unchanged file
/// is neither re-read nor re-hashed. A plain file beside the lease rather
/// than under `objects/` or `runs/`, which GC and the manifest scan list.
/// `.nosync` for the same reason as the lease: it is rewritten on most runs
/// and its inode/device numbers mean nothing on another Mac.
const STAT_CACHE_FILE: &str = "stat-cache.nosync";
/// Bumped whenever [`CachedFile`]'s shape changes; any other version is
/// read as an empty cache and rebuilt. 2: the object's own stat.
const STAT_CACHE_VERSION: u32 = 2;
/// A cache file larger than this is ignored (and rebuilt), and a body
/// larger than this is never saved: about 200,000 entries, far past any
/// vault History captures.
const MAX_STAT_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// git's racy-index window (OB09): a file whose mtime or ctime is this close
/// to the snapshot start may still be written within the same timestamp
/// tick, so it is always read and never stored as vouched.
const RACY_WINDOW_NS: i64 = 2_000_000_000;
const MAX_ELIGIBLE_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RUNS: usize = 200;
/// Event rows ([`record_event`]: a link opened from Linear) keep their own
/// budget, so a burst of them can't push real runs out of [`MAX_RUNS`].
const MAX_EVENT_ROWS: usize = 100;
const MAX_AGE_DAYS: i64 = 90;
const MAX_STORAGE_BYTES: u64 = 512 * 1024 * 1024;
/// A run's repository-changes sidecar (Paperclip PR 16):
/// `runs/{id}.repo.json`. Its stem fails [`validate_run_id`], so it is never
/// read as a manifest.
const REPO_SIDECAR_SUFFIX: &str = ".repo.json";
const MAX_REPO_SIDECAR_BYTES: u64 = 4 * 1024 * 1024;
/// A lease file larger than this is malformed; `release_run_lease` refuses it
/// and [`lease_holder`] reports it as [`LeaseHolder::Unknown`]. The JSON
/// [`LeaseRecord`] (a 180-character owner, a pid and a start stamp) stays
/// well under it; `acquire_lease_file` refuses a record that would not.
const MAX_LEASE_BYTES: u64 = 512;
/// Longest owner the lease file records.
const MAX_LEASE_OWNER_CHARS: usize = 180;
/// Longest holder label (a workflow slug or run source) carried into a
/// human-facing busy message or run-log note.
const MAX_HOLDER_LABEL_CHARS: usize = 80;

/// Lease owner prefix for tracker sync's background writes.
pub(crate) const TRACKER_SYNC_OWNER_PREFIX: &str = "tracker-sync-";
/// Lease owner prefix for the workflow scheduler's short bookkeeping writes
/// (misfire and busy skips, `next_run_at` heals).
pub(crate) const SCHEDULER_OWNER_PREFIX: &str = "workflow-scheduler-";
/// Lease owner prefix for a user's workspace edit from the rail (archive,
/// rename, status, delete), so the edit never lands in a run's capture.
pub(crate) const WORKSPACE_EDIT_OWNER_PREFIX: &str = "workspace-edit-";
/// Lease owner prefix for applying a paired companion device's op
/// (`companion_inbox`): one short task write, never inside a run's capture.
pub(crate) const COMPANION_OWNER_PREFIX: &str = "companion-";
/// Lease owner recorded while [`clear_history`] removes the history tree.
pub(crate) const HISTORY_CLEAR_OWNER: &str = "history-clear";

static HISTORY_LOCK: Lazy<HistoryLock> = Lazy::new(HistoryLock::default);
/// Seals the stat cache (OB09). `.hollow-history` is inside the vault, so an
/// agent can write the cache; an entry it forged (a new stat with an old
/// hash) would hide its edit from History. The key lives only in this
/// process's memory, so a forged or replaced file fails verification and is
/// rebuilt; the cost is one full read per app launch.
static STAT_CACHE_KEY: Lazy<ring::hmac::Key> = Lazy::new(new_stat_cache_key);

fn new_stat_cache_key() -> ring::hmac::Key {
    let secret: [u8; 32] = rand::random();
    ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &secret)
}

/// Scoped runs (build programme PR B3), per canonical vault. A leaf lock:
/// taken under `HISTORY_LOCK` or alone, never while calling out.
static SCOPES: Lazy<Mutex<HashMap<PathBuf, ScopeState>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Which scoped runs are live, and what every full-vault run must leave out
/// of its diff. A full-vault run records the sequence number it began at; a
/// scope that is live at its finish, or was released after it began, was
/// written by another run meanwhile, so the full-vault run's changes and
/// flags under that folder are dropped.
#[derive(Default)]
struct ScopeState {
    seq: u64,
    /// Live scoped run id → its folder.
    live: HashMap<String, String>,
    /// Live scoped run id → the source it began with (`chat`), which
    /// [`with_live_run`] matches instead of the agent-writable manifest.
    live_sources: HashMap<String, String>,
    /// Live scoped run id → what Hollow app and task writes made on its
    /// behalf changed outside its folder (see [`with_live_run`]). The
    /// authority at finish; the running manifest keeps a copy for startup
    /// recovery.
    broker_changes: HashMap<String, Vec<HistoryChange>>,
    /// (folder, sequence number at release) of finished scoped runs, kept
    /// while a full-vault run that began before the release is live.
    released: Vec<(String, u64)>,
    /// Live full-vault run id → the sequence number it began at.
    vault_runs: HashMap<String, u64>,
    /// Live scoped runs that hold their folder alone (a workspace archive,
    /// an update's reservation; tests): no other run is admitted to that
    /// folder while one is live, and one is never admitted beside another.
    exclusive: HashSet<String>,
    /// Live scoped run id → the lock key it holds alone (a build worktree's
    /// turn, `worktree:<canonical path>`; several worktrees per build
    /// workspace, F6 part B): one run per key, while the run is a shared
    /// run of its folder for History ([`ScopeMode::Worktree`]).
    locks: HashMap<String, String>,
    /// Live scoped runs given a build workspace's PRIMARY worktree as theirs
    /// (its own turn, or a folder chat admitted while the workspace had no
    /// lane, told to work there and given its build folder; F6 part B delta
    /// review). The mode is decided at admission, so a lane added or removed
    /// later can't put two of these, or one beside a worktree turn, at once
    /// ([`ScopeMode::PrimaryWorktree`], [`ScopeMode::PrimaryFolder`]).
    given_primary: HashSet<String>,
    /// Live scoped run id → every run that was live in the same folder at
    /// any time since it began (parallel chats in one workspace, 2026-10-07).
    /// Kept for a finished peer too, so the run that finishes second still
    /// sees it; the finishing run marks what both changed
    /// ([`mark_overlaps`]).
    peers: HashMap<String, BTreeSet<String>>,
    /// Live scoped run id → the vault paths its own successful structured
    /// writes named, handed over by its finish ([`note_own_writes`], chat
    /// concurrency F1, 2026-10-20). Present only when they are known; a run
    /// without an entry gets no [`HistoryChange::unattributed`] marks.
    own_writes: HashMap<String, BTreeSet<String>>,
    /// Live scoped run id → the paths in its own folder that Hollow app and
    /// task writes on its behalf changed ([`scoped_live_write`]): its own
    /// changes too, though no tool call of the agent named them.
    on_behalf: HashMap<String, BTreeSet<String>>,
    /// Live scoped run id → Hollow's own writes in its folder made for
    /// someone else (a finished peer's session record, F3): path → what
    /// those writes did. A change that is exactly theirs is left out of the
    /// run's diff ([`note_host_write`]).
    host_writes: HashMap<String, BTreeMap<String, HostWrite>>,
    /// Live scoped run id → the paths Hollow wrote at its own finish for
    /// the folder's sake (`shared_docs` merge-backs and conflict copies,
    /// [`host_write_for`]): in its History, never credited to it.
    host_made: HashMap<String, BTreeSet<String>>,
    /// Live run id (scoped or full-vault) → each file a Hollow app or task
    /// write on its behalf ([`with_live_run`]) wrote, with the file's hash
    /// right after the latest such write (`None`: gone). Its finish marks a
    /// change whose final version is still exactly that one
    /// [`HistoryChange::hollow_write`] (2026-11-13), which its receipt
    /// carries and the quiet check credits as the run's own write.
    hollow_wrote: HashMap<String, BTreeMap<String, Option<String>>>,
    /// Tracker sync lease owner → the workspace folders its write phase
    /// writes. These known writers commit atomically per file and attribute
    /// host changes, so scoped chats can start beside them (SE4). A lease
    /// missing here has unknown write guarantees and holds conversations.
    sync_folders: HashMap<String, BTreeSet<String>>,
}

/// What Hollow's own writes of one file did while a scoped run was live
/// (chat concurrency F3, 2026-10-20): the file's hash before the first of
/// them and after the last (`None`: absent). `broken` when the file changed
/// between two of them, so something else wrote it too.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HostWrite {
    first_before: Option<String>,
    last_after: Option<String>,
    broken: bool,
}

fn scopes() -> std::sync::MutexGuard<'static, HashMap<PathBuf, ScopeState>> {
    SCOPES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Record a full-vault run's start (called under `HISTORY_LOCK`).
fn note_vault_run_began(vault: &Path, run_id: &str) {
    let mut all = scopes();
    let state = all.entry(canonical_vault(vault)).or_default();
    state.seq += 1;
    let seq = state.seq;
    state.vault_runs.insert(run_id.to_string(), seq);
}

/// Where run `run_id`'s baseline and diff live: `Some(folder)` for a live
/// scoped run, `None` for a live full-vault run, and for a run this process
/// did not admit (startup recovery) the manifest's own record.
fn scope_for(vault: &Path, run_id: &str, manifest_scope: Option<&str>) -> Option<String> {
    let all = scopes();
    if let Some(state) = all.get(&canonical_vault(vault)) {
        if let Some(scope) = state.live.get(run_id) {
            return Some(scope.clone());
        }
        if state.vault_runs.contains_key(run_id) {
            return None;
        }
    }
    manifest_scope.map(str::to_string)
}

/// The folders a finishing full-vault run leaves out of its diff (see
/// [`ScopeState`]). Empty for a run this process did not admit.
fn scopes_to_exclude(vault: &Path, run_id: &str) -> Vec<String> {
    let all = scopes();
    let Some(state) = all.get(&canonical_vault(vault)) else {
        return Vec::new();
    };
    let Some(began) = state.vault_runs.get(run_id).copied() else {
        return Vec::new();
    };
    let mut out: Vec<String> = state.live.values().cloned().collect();
    out.extend(
        state
            .released
            .iter()
            .filter(|(_, at)| *at > began)
            .map(|(scope, _)| scope.clone()),
    );
    out.sort();
    out.dedup();
    out
}

/// Forget a finished run in the registry: a scoped run's folder is released,
/// a full-vault run stops holding the released log.
fn forget_registry_run(vault: &Path, run_id: &str) {
    let mut all = scopes();
    let Some(state) = all.get_mut(&canonical_vault(vault)) else {
        return;
    };
    state.live_sources.remove(run_id);
    state.broker_changes.remove(run_id);
    state.own_writes.remove(run_id);
    state.on_behalf.remove(run_id);
    state.host_writes.remove(run_id);
    state.host_made.remove(run_id);
    state.hollow_wrote.remove(run_id);
    state.exclusive.remove(run_id);
    state.locks.remove(run_id);
    state.given_primary.remove(run_id);
    // A live peer keeps this id in its own set: it marks the overlap when it
    // finishes, and its entry goes with it.
    state.peers.remove(run_id);
    if let Some(scope) = state.live.remove(run_id) {
        state.seq += 1;
        let seq = state.seq;
        state.released.push((scope, seq));
    }
    state.vault_runs.remove(run_id);
    // Keep only what a still-live full-vault run began before.
    match state.vault_runs.values().min().copied() {
        Some(oldest) => state.released.retain(|(_, at)| *at > oldest),
        None => state.released.clear(),
    }
}

/// Forget a scoped run whose admission failed: as [`forget_registry_run`],
/// and out of every live peer's set, since it never ran beside them.
fn forget_unadmitted(vault: &Path, run_id: &str) {
    forget_registry_run(vault, run_id);
    if let Some(state) = scopes().get_mut(&canonical_vault(vault)) {
        for peers in state.peers.values_mut() {
            peers.remove(run_id);
        }
    }
}

fn under_scope(path: &str, scope: &str) -> bool {
    path.strip_prefix(scope)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// True while a scoped run is live on `vault` (an install waits then: it
/// quits Hollow). Restores and clears no longer ask this (F5, 2026-10-20):
/// they refuse only for what a live run could be writing.
pub(crate) fn scoped_run_live(vault: &Path) -> bool {
    scopes()
        .get(&canonical_vault(vault))
        .is_some_and(|state| !state.live.is_empty())
}

/// Stable identities for the exit/restart snapshot, including scoped work
/// that is not represented by a chat registry entry or queue row.
pub(crate) fn scoped_run_ids(vault: &Path) -> Vec<String> {
    let mut ids = scopes()
        .get(&canonical_vault(vault))
        .map(|state| state.live.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    ids.sort();
    ids
}

/// How many scoped runs are live on `vault` (tests; the run pump counts
/// chat turns, [`scoped_turn_counts`]).
#[cfg(test)]
pub(crate) fn scoped_run_count(vault: &Path) -> usize {
    scopes()
        .get(&canonical_vault(vault))
        .map_or(0, |state| state.live.len())
}

/// The live scoped runs that are agent chat turns (begun with source
/// `chat`; a workspace archive's lease is not a turn), for the run pump's
/// one budget (2026-10-20): how many there are, and per folder how many of
/// them share it (a build worktree's exclusive turn is in the first count,
/// never the second).
pub(crate) fn scoped_turn_counts(vault: &Path) -> (usize, HashMap<String, usize>) {
    let all = scopes();
    let Some(state) = all.get(&canonical_vault(vault)) else {
        return (0, HashMap::new());
    };
    let mut total = 0;
    let mut shared: HashMap<String, usize> = HashMap::new();
    for (id, scope) in &state.live {
        if state.live_sources.get(id).map(String::as_str) != Some(CHAT_TURN_SOURCE) {
            continue;
        }
        total += 1;
        if !holds_alone(state, id) {
            *shared.entry(scope.clone()).or_default() += 1;
        }
    }
    (total, shared)
}

/// The source a chat turn's run begins with (`chat_queue`).
const CHAT_TURN_SOURCE: &str = "chat";

/// The folder live chat turn `run_id` holds, and whether it holds it alone,
/// if it is a live scoped chat turn (a wait's "behind this turn's slot"
/// note, 2026-10-20).
pub(crate) fn scoped_turn_of(vault: &Path, run_id: &str) -> Option<(String, bool)> {
    let all = scopes();
    let state = all.get(&canonical_vault(vault))?;
    if state.live_sources.get(run_id).map(String::as_str) != Some(CHAT_TURN_SOURCE) {
        return None;
    }
    let scope = state.live.get(run_id)?;
    Some((scope.clone(), holds_alone(state, run_id)))
}

/// Whether live run `id` is admitted alone on something: its folder
/// (exclusive) or a worktree's lock key. Such a run is outside its
/// folder's per-workspace chat count.
fn holds_alone(state: &ScopeState, id: &str) -> bool {
    state.exclusive.contains(id) || state.locks.contains_key(id)
}

/// A live scoped run in `scope`, if any (several may share a workspace's
/// own folder; this names one).
pub(crate) fn scope_holder(vault: &Path, scope: &str) -> Option<String> {
    scopes().get(&canonical_vault(vault)).and_then(|state| {
        state
            .live
            .iter()
            .find(|(_, s)| s.as_str() == scope)
            .map(|(id, _)| id.clone())
    })
}

/// Every live scoped run in `scope` (a workspace merge refuses unless the
/// only one is its caller's own).
pub(crate) fn scope_holders(vault: &Path, scope: &str) -> Vec<String> {
    scopes()
        .get(&canonical_vault(vault))
        .map(|state| {
            state
                .live
                .iter()
                .filter(|(_, s)| s.as_str() == scope)
                .map(|(id, _)| id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The live run that keeps a new scoped run out of `scope`, if any: for an
/// exclusive run (a build workspace's worktree) any live run there, for a
/// shared one (a workspace's own folder) only an exclusive one (tests; the
/// run pump asks [`scope_blocker_kind`]).
#[cfg(test)]
pub(crate) fn scope_blocker(vault: &Path, scope: &str, exclusive: bool) -> Option<String> {
    scopes()
        .get(&canonical_vault(vault))
        .and_then(|state| blocker_in(state, scope, &ScopeMode::from_exclusive(exclusive)))
}

/// How a scoped run is admitted to its folder `scope` (several worktrees
/// per build workspace, F6 part B; docs/BUILD_WORKSPACES_MULTI.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ScopeMode {
    /// Beside other shared runs (a workspace's own vault folder).
    Shared,
    /// Alone in its folder (a workspace archive's lease; tests).
    Exclusive,
    /// A build worktree's turn: a shared run of its folder for History
    /// (what it and a folder chat both change there is marked shared), but
    /// alone on its lock key, `worktree:<canonical worktree path>`, so one
    /// turn runs per worktree and two worktrees of one workspace run at once.
    Worktree(String),
    /// The primary worktree's turn once its workspace has lanes: as
    /// [`Self::Worktree`], and it also waits for any run in its folder given
    /// the primary (a folder chat admitted before the first lane).
    PrimaryWorktree(String),
    /// A folder chat given the primary worktree (its workspace has no lane:
    /// told to work there, given its build folder): shared with the folder's
    /// other conversations, but it waits for any worktree turn in its folder
    /// (one admitted while a lane existed).
    PrimaryFolder,
}

impl ScopeMode {
    /// The mode the old `exclusive` flag meant.
    #[cfg(test)]
    pub(crate) fn from_exclusive(exclusive: bool) -> Self {
        if exclusive {
            Self::Exclusive
        } else {
            Self::Shared
        }
    }

    /// Whether a run in this mode is admitted alone on something (its folder
    /// or a lock key): outside the folder's per-workspace chat count.
    pub(crate) fn alone(&self) -> bool {
        !matches!(self, Self::Shared | Self::PrimaryFolder)
    }
}

/// Whether live run `id` keeps a `mode` request out of a folder
/// ([`blocker_in`], [`blocked_by`]); `in_scope`: the run is in that folder.
fn blocks(state: &ScopeState, id: &str, in_scope: bool, mode: &ScopeMode) -> bool {
    let exclusive = in_scope && state.exclusive.contains(id);
    let lock = |key: &str| state.locks.get(id).is_some_and(|k| k == key);
    match mode {
        ScopeMode::Exclusive => in_scope,
        ScopeMode::Shared => exclusive,
        ScopeMode::Worktree(key) => exclusive || lock(key),
        ScopeMode::PrimaryWorktree(key) => {
            exclusive || lock(key) || (in_scope && state.given_primary.contains(id))
        }
        ScopeMode::PrimaryFolder => exclusive || (in_scope && state.locks.contains_key(id)),
    }
}

/// The live run that keeps a `mode` request out of `scope`, if any: for an
/// exclusive request any live run in `scope`; for a shared one an exclusive
/// run there; for a worktree turn an exclusive run there or any live run
/// holding the same lock key (wherever it is).
fn blocker_in(state: &ScopeState, scope: &str, mode: &ScopeMode) -> Option<String> {
    let mut ids: Vec<&String> = state
        .live
        .iter()
        .filter(|(id, s)| blocks(state, id, s.as_str() == scope, mode))
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    ids.first().map(|id| (*id).clone())
}

/// The live run that keeps a build worktree's own work (a turn, a commit or
/// a removal) out: one holding its lock key `key`, or one holding its
/// workspace's folder `scope` alone. A folder chat doesn't own the git
/// index, so it never does (F6 part B).
#[cfg(test)]
pub(crate) fn worktree_holder(vault: &Path, scope: &str, key: &str) -> Option<String> {
    mode_holder(vault, scope, &ScopeMode::Worktree(key.to_string()))
}

/// The live run that keeps a `mode` request out of `scope`, if any
/// ([`blocker_in`]): what a build worktree's own work (a commit, a
/// removal) waits for, in the mode that worktree's turns take
/// (`build_workspaces::turn_mode`).
pub(crate) fn mode_holder(vault: &Path, scope: &str, mode: &ScopeMode) -> Option<String> {
    scopes()
        .get(&canonical_vault(vault))
        .and_then(|state| blocker_in(state, scope, mode))
}

/// Whether a live run keeps a new scoped run out of `scope`, by the
/// blocker's own kind: for an exclusive request (a build workspace's
/// worktree) any live run there blocks, for a shared one (a workspace's own
/// folder) only an exclusive one. `Some(true)` when the blocker holds
/// `scope` to itself, `Some(false)` when it is a shared run keeping an
/// `exclusive` request out, `None` when nothing blocks. The run pump's
/// folder rule and its waiting reason (overnight A2; `run_pump::Hold`).
/// Advisory, like [`lease_holder`]; admission decides again under the lock.
/// Returns the blocking run's id with its kind, so a waiting reason can name
/// it (P1-46).
pub(crate) fn scope_blocker_kind(
    vault: &Path,
    scope: &str,
    mode: &ScopeMode,
) -> Option<(String, bool)> {
    scopes().get(&canonical_vault(vault)).and_then(|state| {
        blocker_in(state, scope, mode).map(|id| {
            let kind = holds_alone(state, &id);
            (id, kind)
        })
    })
}

/// True when run `run_id` is one of the live runs keeping a new scoped run
/// (`exclusive` or shared) out of `scope`: a turn whose child would wait for
/// the turn itself.
pub(crate) fn blocked_by(vault: &Path, scope: &str, mode: &ScopeMode, run_id: &str) -> bool {
    scopes().get(&canonical_vault(vault)).is_some_and(|state| {
        state
            .live
            .get(run_id)
            .is_some_and(|s| blocks(state, run_id, s == scope, mode))
    })
}

/// How many OTHER runs are live in live scoped run `run_id`'s folder right
/// now (0 for a full-vault run or one that isn't live). Tests only since the
/// shared-workspace note stopped depending on a live peer (concurrency PR 2).
#[cfg(test)]
pub(crate) fn live_peers(vault: &Path, run_id: &str) -> usize {
    scopes().get(&canonical_vault(vault)).map_or(0, |state| {
        state.live.get(run_id).map_or(0, |scope| {
            state
                .live
                .iter()
                .filter(|(id, s)| id.as_str() != run_id && *s == scope)
                .count()
        })
    })
}

/// Live scoped run `run_id`'s folder and whether it holds that folder alone
/// (an exclusive run; a build worktree's turn shares its folder since F6
/// part B); `None` for a full-vault run or one that isn't
/// live. The shared-workspace note and the shared-doc safety net read it
/// (concurrency PR 2, 2026-10-20).
pub(crate) fn live_scope(vault: &Path, run_id: &str) -> Option<(String, bool)> {
    scopes().get(&canonical_vault(vault)).and_then(|state| {
        state
            .live
            .get(run_id)
            .map(|scope| (scope.clone(), state.exclusive.contains(run_id)))
    })
}

/// The worktree lock key live run `run_id` holds, if any (a build
/// worktree's turn in a workspace with several worktrees, F6 part B): its
/// working folder isn't its scope, and Hollow's per-workspace build actions
/// know which worktree it works in.
pub(crate) fn lock_of(vault: &Path, run_id: &str) -> Option<String> {
    scopes()
        .get(&canonical_vault(vault))
        .and_then(|state| state.locks.get(run_id).cloned())
}

/// The runs that shared `run_id`'s folder while it was live, when this
/// process admitted it; `None` for a run it did not (startup recovery).
fn registry_peers(vault: &Path, run_id: &str) -> Option<BTreeSet<String>> {
    let all = scopes();
    let state = all.get(&canonical_vault(vault))?;
    if !state.live.contains_key(run_id) {
        return None;
    }
    Some(state.peers.get(run_id).cloned().unwrap_or_default())
}

/// Hand live scoped run `run_id` the vault-relative paths its own
/// successful structured writes named (chat concurrency F1, 2026-10-20),
/// just before its finish: a chat turn's are
/// `EvidenceCollector::own_written_paths`. At the finish, a change in its
/// folder that none of them (nor a Hollow write on its behalf) named is
/// marked [`HistoryChange::unattributed`] when the run ever shared the
/// folder. Nothing for a run that isn't a live scoped run.
pub(crate) fn note_own_writes(vault: &Path, run_id: &str, paths: BTreeSet<String>) {
    let mut all = scopes();
    let Some(state) = all.get_mut(&canonical_vault(vault)) else {
        return;
    };
    if state.live.contains_key(run_id) {
        state
            .own_writes
            .entry(run_id.to_string())
            .or_default()
            .extend(paths);
    }
}

/// Run Hollow's own write `write` of vault file `rel` so that no live
/// scoped run counts it as its change (chat concurrency F3, 2026-10-20): a
/// finished turn's session record (`workspaces::record_session`) lands in
/// its workspace folder while the folder's other conversations are still
/// live, and was showing up in their History and receipts. `write` reports
/// what it actually committed ([`HostCommit`]: the bytes it replaced and
/// wrote, `None` when it wrote nothing), never what is on disk afterwards,
/// so a peer's edit landing just after it is never taken for Hollow's. Under
/// `HISTORY_LOCK` (so no finish diffs between the write and the note), that
/// is noted for every live scoped run whose folder holds `rel`; at its
/// finish a change that is exactly those writes is left out, unless the
/// run's own writes named the file. Anything else that wrote the file too
/// keeps it in, and a `write` that fails notes nothing. Lock
/// order: `HISTORY_LOCK` → whatever `write` takes (`vault::MUTATE_LOCK`, the
/// indexes), as for [`with_live_run`]; never call it holding the session
/// store's `WRITE_LOCK`. Called on a thread that already holds
/// `HISTORY_LOCK` (inside [`with_live_run`]), it runs under that hold.
pub(crate) fn host_write<T>(
    vault: &Path,
    rel: &str,
    write: impl FnOnce() -> Result<(T, Option<HostCommit>), String>,
) -> Result<T, String> {
    host_write_for(vault, None, || {
        let (value, commit) = write()?;
        Ok((
            value,
            commit.map(|c| (rel.to_string(), c)).into_iter().collect(),
        ))
    })
}

/// [`host_write`] for writes whose paths are known only once made (a
/// conflict copy's free name): `write` reports each `(vault path, commit)`.
/// A write made at run `made_for`'s own finish (`shared_docs`' merge-back
/// and conflict copies, while that run is still live) stays in that run's
/// History, where it belongs, but is never credited to it
/// ([`HistoryChange::unattributed`], unless its own writes named the file);
/// every other live run leaves it out like any host write.
pub(crate) fn host_write_for<T>(
    vault: &Path,
    made_for: Option<&str>,
    write: impl FnOnce() -> Result<(T, Vec<(String, HostCommit)>), String>,
) -> Result<T, String> {
    // Inside `with_live_run` (an agent's Hollow app write, such as moving a
    // chat, which rewrites its record) this thread, or the broker thread
    // waiting for this worker, holds the lock already.
    let _guard = (!history_lock_held_here()).then(lock_history);
    let (value, commits) = write()?;
    for (rel, HostCommit { before, after }) in commits {
        if before != after {
            note_host_write(vault, &rel, before, after, made_for);
        }
    }
    Ok(value)
}

/// What a [`host_write`] committed: the sha256 of the bytes it replaced and
/// of the bytes it wrote (`None`: the file was absent, or it removed it).
pub(crate) struct HostCommit {
    before: Option<String>,
    after: Option<String>,
}

impl HostCommit {
    pub(crate) fn new(before: Option<&[u8]>, after: Option<&[u8]>) -> Self {
        Self {
            before: before.map(hash_bytes),
            after: after.map(hash_bytes),
        }
    }
}

/// True when `owned` (a path an own write resolved to) and `path` (History's
/// name for a change) are the same vault file (chat concurrency F1 review,
/// 2026-10-20). An existing file is resolved by the file system on both
/// sides, so only an exact match counts. A file that is gone has no
/// identity left to resolve: its names match ignoring letter case, but only
/// when the volume holding its nearest surviving folder says it ignores case
/// ([`case_insensitive_at`]); unknown means no match.
pub(crate) fn same_file(vault: &Path, owned: &str, path: &str) -> bool {
    if owned == path {
        return true;
    }
    if owned.to_lowercase() != path.to_lowercase() {
        return false;
    }
    let Ok(full) = crate::vault::safe_join(vault, path) else {
        return false;
    };
    if fs::symlink_metadata(&full).is_ok() {
        return false;
    }
    let parent = full.ancestors().skip(1).find(|dir| dir.is_dir());
    parent.is_some_and(|dir| case_insensitive_at(dir) == Some(true))
}

/// True when `path` is one of `owned` ([`same_file`]).
fn owns(vault: &Path, owned: &BTreeSet<String>, path: &str) -> bool {
    owned.contains(path) || owned.iter().any(|name| same_file(vault, name, path))
}

/// Whether the volume holding `dir` ignores letter case, as the file system
/// itself reports it (`pathconf(_PC_CASE_SENSITIVE)` on macOS: 0 ignores
/// case, 1 doesn't). `None` when it can't say, and off macOS.
fn case_insensitive_at(dir: &Path) -> Option<bool> {
    #[cfg(test)]
    if let Some(forced) = CASE_OVERRIDE.with(std::cell::Cell::get) {
        return forced;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_dir = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: `c_dir` is a NUL-terminated path that outlives the call,
        // which only reads it.
        match unsafe { libc::pathconf(c_dir.as_ptr(), libc::_PC_CASE_SENSITIVE) } {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dir;
        None
    }
}

#[cfg(test)]
thread_local! {
    /// Tests force [`case_insensitive_at`]'s answer on their own thread.
    static CASE_OVERRIDE: std::cell::Cell<Option<Option<bool>>> =
        const { std::cell::Cell::new(None) };
}

/// Force [`case_insensitive_at`]'s answer on this thread (`None` clears it).
#[cfg(test)]
pub(crate) fn force_case_insensitive_for_test(answer: Option<Option<bool>>) {
    CASE_OVERRIDE.with(|slot| slot.set(answer));
}

/// The paths live scoped run `run_id`'s own writes named
/// ([`note_own_writes`]; empty when none were handed over).
fn own_writes_for(vault: &Path, run_id: &str) -> BTreeSet<String> {
    scopes()
        .get(&canonical_vault(vault))
        .and_then(|state| state.own_writes.get(run_id).cloned())
        .unwrap_or_default()
}

/// Live scoped run `run_id`'s [`HostWrite`]s, by path (empty for any
/// other run).
fn host_writes_for(vault: &Path, run_id: &str) -> BTreeMap<String, HostWrite> {
    scopes()
        .get(&canonical_vault(vault))
        .and_then(|state| state.host_writes.get(run_id).cloned())
        .unwrap_or_default()
}

impl HostWrite {
    /// True when `change` is exactly what Hollow's writes did: nothing else
    /// wrote the file between them, it was as the first found it when the
    /// run began, and it is as the last left it now.
    fn explains(&self, change: &HistoryChange) -> bool {
        fn side(expected: &Option<String>, coverage: Coverage, hash: &Option<String>) -> bool {
            match expected {
                None => coverage == Coverage::Absent,
                Some(want) => coverage == Coverage::Captured && hash.as_ref() == Some(want),
            }
        }
        !self.broken
            && side(
                &self.first_before,
                change.coverage_before(),
                &change.before_hash,
            )
            && side(
                &self.last_after,
                change.coverage_after(),
                &change.after_hash,
            )
    }
}

/// Mark each change of run `run_id` in its folder `scope` that neither its
/// own structured writes ([`note_own_writes`]) nor Hollow's writes on its
/// behalf named as [`HistoryChange::unattributed`] (chat concurrency F1,
/// 2026-10-20). Called only for a run that shared its folder; nothing when
/// its own writes were never handed over (a dropped lease, startup
/// recovery), since then History can't tell. What Hollow wrote for it
/// outside the folder (its broker changes) is always its own.
fn mark_unattributed(vault: &Path, run_id: &str, scope: &str, changes: &mut [HistoryChange]) {
    let (own, on_behalf) = {
        let all = scopes();
        let Some(state) = all.get(&canonical_vault(vault)) else {
            return;
        };
        let Some(own) = state.own_writes.get(run_id).cloned() else {
            return;
        };
        (
            own,
            state.on_behalf.get(run_id).cloned().unwrap_or_default(),
        )
    };
    for change in changes
        .iter_mut()
        .filter(|c| under_scope(&c.vault_path, scope))
    {
        change.unattributed =
            !owns(vault, &own, &change.vault_path) && !owns(vault, &on_behalf, &change.vault_path);
    }
}

/// Mark what Hollow wrote at run `run_id`'s own finish ([`host_write_for`]
/// with `made_for`) as not its own, unless its own writes named the file:
/// a merge-back or a conflict copy is the folder's, not the agent's work.
fn mark_host_made(vault: &Path, run_id: &str, changes: &mut [HistoryChange]) {
    let (made, own) = {
        let all = scopes();
        let Some(state) = all.get(&canonical_vault(vault)) else {
            return;
        };
        let Some(made) = state.host_made.get(run_id).cloned() else {
            return;
        };
        (
            made,
            state.own_writes.get(run_id).cloned().unwrap_or_default(),
        )
    };
    for change in changes
        .iter_mut()
        .filter(|c| made.contains(&c.vault_path) && !owns(vault, &own, &c.vault_path))
    {
        change.unattributed = true;
    }
}

/// [`host_write`]'s note: for every live scoped run whose folder holds
/// `rel`, extend that file's [`HostWrite`] chain.
fn note_host_write(
    vault: &Path,
    rel: &str,
    before: Option<String>,
    after: Option<String>,
    made_for: Option<&str>,
) {
    let mut all = scopes();
    let Some(state) = all.get_mut(&canonical_vault(vault)) else {
        return;
    };
    if let Some(run) = made_for.filter(|run| state.live.contains_key(*run)) {
        state
            .host_made
            .entry(run.to_string())
            .or_default()
            .insert(rel.to_string());
    }
    let runs: Vec<String> = state
        .live
        .iter()
        .filter(|(id, scope)| under_scope(rel, scope) && Some(id.as_str()) != made_for)
        .map(|(id, _)| id.clone())
        .collect();
    for run in runs {
        let writes = state.host_writes.entry(run).or_default();
        match writes.get_mut(rel) {
            Some(chain) => {
                chain.broken |= chain.last_after != before;
                chain.last_after = after.clone();
            }
            None => {
                writes.insert(
                    rel.to_string(),
                    HostWrite {
                        first_before: before.clone(),
                        last_after: after.clone(),
                        broken: false,
                    },
                );
            }
        }
    }
}

/// The live scoped run in one of `folders` (`workspaces/<id>`), if any:
/// what keeps a sync that writes those folders waiting (F5, 2026-10-20).
/// With no folders named, any live scoped run. Advisory here;
/// [`tracker_mirror_write_lease`] decides under the lock.
fn sync_blocker(vault: &Path, folders: SyncFolders) -> Option<String> {
    scopes()
        .get(&canonical_vault(vault))
        .and_then(|state| match folders {
            Some(folders) => live_in_folders(state, folders),
            None => state.live.keys().min().cloned(),
        })
}

fn live_in_folders(state: &ScopeState, folders: &BTreeSet<String>) -> Option<String> {
    let mut ids: Vec<&String> = state
        .live
        .iter()
        .filter(|(_, scope)| folder_in(folders, scope))
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    ids.first().map(|id| (*id).clone())
}

/// Only an unknown tracker writer holds scoped admission. Known mirror
/// writers use atomic per-file CAS and History host attribution.
fn sync_blocks_chat(vault: &Path, owner: &str) -> bool {
    !scopes()
        .get(&canonical_vault(vault))
        .is_some_and(|state| state.sync_folders.contains_key(owner))
}

/// The queue uses the same actual lease coverage as scoped admission. A
/// foreign-process sync with unknown coverage conservatively holds it.
pub(crate) fn tracker_blocks_scope(vault: &Path, _scope: &str) -> bool {
    read_lease_owner(vault).is_some_and(|owner| {
        owner.trim().starts_with(TRACKER_SYNC_OWNER_PREFIX) && sync_blocks_chat(vault, owner.trim())
    })
}

/// True when `scope` is one of `folders`, ignoring case: a ledger's spelling
/// of a folder names the same folder on a case-insensitive Mac volume.
fn folder_in(folders: &BTreeSet<String>, scope: &str) -> bool {
    let scope = scope.to_lowercase();
    folders.iter().any(|folder| folder.to_lowercase() == scope)
}

/// True when `run_id` holds the full-vault lease (lock-free and advisory,
/// like [`lease_holder`]): a Hollow app call from the holding run, acting on
/// its own behalf after the user allowed it, is not "another writer".
pub(crate) fn lease_is_run(vault: &Path, run_id: &str) -> bool {
    !run_id.is_empty() && read_lease_owner(vault).as_deref() == Some(run_id)
}

/// True while `run_id` is live: it holds the full-vault lease, or it is a
/// live scoped run.
pub(crate) fn run_is_live(vault: &Path, run_id: &str) -> bool {
    if run_id.is_empty() {
        return false;
    }
    if scopes()
        .get(&canonical_vault(vault))
        .is_some_and(|state| state.live.contains_key(run_id))
    {
        return true;
    }
    read_lease_owner(vault).as_deref() == Some(run_id)
}
static RUN_SEQ: AtomicU64 = AtomicU64::new(0);

/// Notified after the full-vault lease file is removed (see the module doc).
/// `notify_one` stores a permit when nobody is waiting, so a release that
/// lands between a waiter's check and its wait is never lost.
static LEASE_RELEASED: Lazy<tokio::sync::Notify> = Lazy::new(tokio::sync::Notify::new);

/// Vaults whose lease release notified, so a test can prove its own release
/// fired the wake-up (other tests share the process-global `Notify`).
#[cfg(test)]
static RELEASE_NOTICES: Lazy<Mutex<Vec<PathBuf>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// The leases THIS process holds, keyed by (canonical vault, owner), each
/// with its lease-lock file when it could be taken (Paperclip PR 18).
/// Inserted when a lease file is created, removed after it is released, so
/// the kernel lock outlives the file by a moment and never the other way
/// round. A leaf lock.
static HELD_OWNERS: Lazy<Mutex<HashMap<(PathBuf, String), Option<HeldLock>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// What a reclaim still has to announce once `HISTORY_LOCK` is dropped
/// (see `drain_reclaim_notices`). A leaf lock.
static RECLAIM_NOTICES: Lazy<Mutex<Vec<ReclaimNotice>>> = Lazy::new(|| Mutex::new(Vec::new()));

#[cfg(test)]
thread_local! {
    /// Unit tests have no notification app; a test that exercises the GUI's
    /// reclaim of a manifested run switches this on for its own thread.
    static TEST_GUI_RECLAIM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Runs inside `reclaim_locked` after the lease was judged reclaimable and
    /// before this process takes the lease lock: a test plays a concurrent
    /// reclaimer in another process there.
    static TEST_BEFORE_RECLAIM_HOLD: std::cell::RefCell<Option<Box<dyn FnMut(&Path)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
static RESTORE_APPLY_FAILURE: Lazy<Mutex<Option<(PathBuf, String)>>> =
    Lazy::new(|| Mutex::new(None));

#[cfg(test)]
static RESTORE_APPLY_MUTATION: Lazy<Mutex<Option<(PathBuf, String, Vec<u8>)>>> =
    Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRunHandle {
    pub id: String,
    /// How long admission's vault snapshot and manifest write took
    /// (Paperclip PR 15c's `baselineSnapshotMs`). None on a handle built by
    /// hand or deserialized from before the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_ms: Option<u64>,
}

/// One file a run changed. Generated into `src/generated/ipc.ts` with
/// [`HistoryRun`] (see `ts_bindings`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HistoryChange {
    pub vault_path: String,
    /// `create`, `modify`, or `delete`.
    pub change_type: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub before_size: Option<u64>,
    pub after_size: Option<u64>,
    /// True when every version that exists is valid UTF-8 and can be shown as
    /// text. Binary versions remain restorable but `read_version` rejects them.
    pub text: bool,
    /// True when either side carries Notion import provenance. This is
    /// historical metadata, never an authority flag: restore changes the
    /// canonical local note and never writes to Notion.
    #[serde(default)]
    pub notion_mirror: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub before_notion_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub after_notion_id: Option<String>,
    /// The file existed on that side but History holds no copy of it, or
    /// History can't tell whether it existed (see `before_coverage`). That
    /// version has no object and restore refuses the change: it would
    /// otherwise trash or overwrite a file History never had (merged plan
    /// slice 1, review B·F01). Kept beside the coverage fields so a record
    /// read by an older build still refuses.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub before_uncaptured: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub after_uncaptured: bool,
    /// What History knows about each side (merged plan slice 4, review
    /// B·F01): absent, captured, uncaptured with its reason, or unknown.
    /// Omitted on records written before it; read a side through
    /// [`HistoryChange::coverage_before`] / [`HistoryChange::coverage_after`],
    /// which derive it for those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub before_coverage: Option<Coverage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub after_coverage: Option<Coverage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub restored_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub restored_by_run_id: Option<String>,
    /// Other runs in the same workspace folder, live at the same time as this
    /// one, whose changes list this file too (parallel chats in one
    /// workspace, 2026-10-07), or whose diff will: a peer still live at this
    /// run's finish is marked from its baseline (chat concurrency F1,
    /// 2026-10-20). Both diffs cover the whole folder, so History can't tell
    /// whose edit it was: a restore of this file could undo the other
    /// conversation's work, and refuses unless forced. Credit goes by
    /// [`HistoryChange::unattributed`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<String>>", optional))]
    pub also_changed_by: Vec<String>,
    /// Not this run's own change (chat concurrency F1, 2026-10-20): the run
    /// shared its workspace folder with another run at some point, and none
    /// of its own successful structured writes (Edit, Write, MultiEdit,
    /// NotebookEdit, apply_patch, Codex's file_change) or Hollow's writes on
    /// its behalf named this file. Its diff only saw it change, so the file
    /// is shown as another chat's and isn't credited to this run. Set only
    /// when the run's own writes are known (a chat turn's finish); restore
    /// still goes by [`HistoryChange::also_changed_by`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub unattributed: bool,
    /// "Changes unknown" (Q-R11, 2026-11-15): the run ended with its process
    /// (a crash or forced quit, closed at the next startup), so its diff is
    /// the disk at that startup against its baseline, which also holds
    /// anything the person edited while Hollow was down. Restore refuses
    /// unless forced ([`unverified_refusal`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub unverified: bool,
    /// A Hollow app or task write made on this run's behalf
    /// ([`with_live_run`], the task broker) wrote this file, and the run
    /// left it exactly as that write did (2026-11-13). Copied into the
    /// run's receipt (`ReceiptFile::hollow_write`), where the quiet check
    /// credits it as the run's own write, as it does a `confirmed` one; a
    /// later write by anyone else during the run leaves it unset. Not in
    /// the generated IPC types.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(skip))]
    pub hollow_write: bool,
}

/// One automated run's record (`list_history_runs`, `get_history_run`, the
/// restores). Generated into `src/generated/ipc.ts` (see `ts_bindings`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HistoryRun {
    pub id: String,
    pub source: String,
    pub source_id: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// Caller-supplied terminal status (`success`, `error`, `cancelled`, or
    /// `interrupted`). `running` is used until `finish_run` completes.
    pub status: String,
    #[serde(default)]
    pub changes: Vec<HistoryChange>,
    /// Protected changes the run made that Hollow did not attribute to itself
    /// (workflow permission fields, `me/`, agent instructions and config,
    /// watched dot-state, the run's own history record). Computed at finish by
    /// `run_guard`; always empty for history restores.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<crate::run_guard::RunFlag>>", optional))]
    pub flags: Vec<crate::run_guard::RunFlag>,
    /// A scoped run's vault folder (build programme PR B3): its baseline and
    /// diff cover only this vault-relative folder (a build workspace's
    /// `workspaces/<id>`), and it holds no full-vault lease. `None` for every
    /// full-vault run. Display and startup-recovery only: a live run's scope
    /// comes from the in-process registry, never from this agent-writable
    /// copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub scope: Option<String>,
    /// A scoped run's peers: the other runs that were live in the same folder
    /// while it was (parallel chats in one workspace, 2026-10-07). Display
    /// only; the files they both changed carry
    /// [`HistoryChange::also_changed_by`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<String>>", optional))]
    pub overlaps: Vec<String>,
}

/// Why History holds no copy of a file that existed (merged plan slice 4).
/// Generated as `HistoryUncapturedReason`: the bare name is too generic for
/// one flat TS namespace.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "HistoryUncapturedReason"))]
#[serde(rename_all = "camelCase")]
pub enum UncapturedReason {
    /// Over the 2 MiB per-file limit.
    TooLarge,
    /// Its metadata or bytes could not be read.
    Unreadable,
    /// Not a regular file History stores: a symlink, socket or other special
    /// file at a visible path.
    Excluded,
}

/// What History knows about one side of a change (merged plan slice 4,
/// review B·F01; decision 5). Only `absent` and `captured` can be restored.
/// On the wire: `{"state":"uncaptured","reason":"tooLarge"}`. Generated as
/// `HistoryCoverage` (see [`UncapturedReason`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "HistoryCoverage"))]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Coverage {
    /// The file did not exist on that side.
    Absent,
    /// History holds that version's bytes.
    Captured,
    /// The file existed and History holds no copy. `reason` is `None` only
    /// on records from slice 1, which didn't keep it ("over 2 MiB or
    /// unreadable").
    Uncaptured {
        #[serde(default)]
        reason: Option<UncapturedReason>,
    },
    /// History can't tell whether the file existed: a folder above it could
    /// not be listed, or the record contradicts itself.
    Unknown,
}

impl Coverage {
    /// A restore can put this side back: there was no file, or History holds
    /// its bytes.
    pub fn restorable(self) -> bool {
        matches!(self, Coverage::Absent | Coverage::Captured)
    }

    /// Whether a file existed on this side; `None` when History can't tell.
    fn existed(self) -> Option<bool> {
        match self {
            Coverage::Absent => Some(false),
            Coverage::Captured | Coverage::Uncaptured { .. } => Some(true),
            Coverage::Unknown => None,
        }
    }
}

/// `create`, `delete` or `modify` from what existed on each side. A side
/// History can't vouch for (unknown) makes it a `modify`, so restore never
/// reads it as "there was no file".
fn change_type_for(before: Coverage, after: Coverage) -> &'static str {
    match (before.existed(), after.existed()) {
        (Some(false), Some(true)) => "create",
        (Some(true), Some(false)) => "delete",
        _ => "modify",
    }
}

/// One side of a change, from its stored coverage and its older fields.
/// Records from before slice 4 store none: a hash reads as captured, the
/// slice 1 flag as uncaptured, neither as absent. A record that disagrees
/// with itself (the manifest is agent-writable) reads as unknown, never as
/// restorable.
fn side_coverage(stored: Option<Coverage>, hash: Option<&str>, flagged: bool) -> Coverage {
    match (stored, hash.is_some(), flagged) {
        (Some(Coverage::Captured) | None, true, false) => Coverage::Captured,
        (Some(Coverage::Absent) | None, false, false) => Coverage::Absent,
        (Some(gap @ (Coverage::Uncaptured { .. } | Coverage::Unknown)), _, _) => gap,
        (None, false, true) => Coverage::Uncaptured { reason: None },
        _ => Coverage::Unknown,
    }
}

impl HistoryChange {
    /// What History knows about the version before the run.
    pub fn coverage_before(&self) -> Coverage {
        side_coverage(
            self.before_coverage,
            self.before_hash.as_deref(),
            self.before_uncaptured,
        )
    }

    /// What History knows about the version after the run.
    pub fn coverage_after(&self) -> Coverage {
        side_coverage(
            self.after_coverage,
            self.after_hash.as_deref(),
            self.after_uncaptured,
        )
    }

    /// History can restore this change: it holds every version that existed.
    pub fn recoverable(&self) -> bool {
        self.coverage_before().restorable() && self.coverage_after().restorable()
    }
}

impl HistoryRun {
    /// Every change in the run can be restored (decision 5). A run with any
    /// uncaptured or unknown side never claims to be fully recoverable.
    #[cfg(test)]
    pub fn fully_recoverable(&self) -> bool {
        self.changes.iter().all(HistoryChange::recoverable)
    }
}

/// A file a restore can't return: which side History lacks, and what it
/// knows about that side instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrestorable {
    pub vault_path: String,
    /// `before` or `after`.
    pub side: &'static str,
    pub coverage: Coverage,
}

/// The files a restore of `changes` can't return, in path order: what the
/// restore preview lists before anything is written (merged plan slice 4).
/// Restore refuses while it is non-empty, and its refusal names each file.
pub fn restore_preview(changes: &[HistoryChange]) -> Vec<Unrestorable> {
    let mut blocked: Vec<Unrestorable> = changes
        .iter()
        .filter_map(|change| {
            let (side, coverage) = [
                ("before", change.coverage_before()),
                ("after", change.coverage_after()),
            ]
            .into_iter()
            .find(|(_, coverage)| !coverage.restorable())?;
            Some(Unrestorable {
                vault_path: change.vault_path.clone(),
                side,
                coverage,
            })
        })
        .collect();
    blocked.sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
    blocked
}

/// A finalized run, whether this call finalized it (false for an idempotent
/// re-finish), and for a tracked run the source it began with: the manifest's
/// own copy is agent-writable, so notifications use this one.
#[derive(Debug)]
struct Finalized {
    run: HistoryRun,
    newly: bool,
    origin: Option<(String, Option<String>)>,
}

impl Finalized {
    /// Notify about a newly finalized run's flags. Call only after
    /// `HISTORY_LOCK` is released: this verifies flagged workflows (the
    /// permission write guard) and writes a notification.
    fn notify(&self, vault: &Path) {
        if !self.newly || self.run.flags.is_empty() {
            return;
        }
        let (source, source_id) = match &self.origin {
            Some((source, source_id)) => (source.as_str(), source_id.as_deref()),
            None => (self.run.source.as_str(), self.run.source_id.as_deref()),
        };
        crate::run_guard::after_finish(
            vault,
            source,
            source_id,
            self.run.scope.as_deref(),
            &self.run.flags,
            &self.run.changes,
        );
    }
}

/// History's footprint and limits (`history_storage_status`). Generated into
/// `src/generated/ipc.ts` (see `ts_bindings`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HistoryStorageStatus {
    pub run_count: usize,
    pub active_run_count: usize,
    pub object_count: usize,
    /// Bytes occupied by all regular files under `.hollow-history`, including
    /// manifests. This is the user-facing disk-usage number.
    pub total_bytes: u64,
    pub object_bytes: u64,
    pub max_runs: usize,
    pub max_age_days: u64,
    pub max_bytes: u64,
    pub oldest_run_at: Option<String>,
    pub newest_run_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SnapshotEntry {
    vault_path: String,
    hash: String,
    size: u64,
    text: bool,
}

/// A visible file a snapshot holds no copy of (merged plan slice 4), with
/// the size and modification time it had then, so a diff can tell whether
/// it changed. Reads slice 1's bare path as an entry with nothing known.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", from = "UncapturedWire")]
struct UncapturedEntry {
    vault_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<UncapturedReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_ns: Option<u64>,
}

impl UncapturedEntry {
    fn new(vault_path: String, reason: UncapturedReason, meta: Option<&fs::Metadata>) -> Self {
        Self {
            vault_path,
            reason: Some(reason),
            size: meta.map(fs::Metadata::len),
            modified_ns: meta
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .and_then(|since| u64::try_from(since.as_nanos()).ok()),
        }
    }

    /// Equal on both sides of a run means History saw no change. Two
    /// unreadable stamps compare equal: History can't see that file at all.
    fn stamp(&self) -> (Option<u64>, Option<u64>) {
        (self.size, self.modified_ns)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum UncapturedWire {
    Path(String),
    Entry(UncapturedFields),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UncapturedFields {
    vault_path: String,
    #[serde(default)]
    reason: Option<UncapturedReason>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    modified_ns: Option<u64>,
}

impl From<UncapturedWire> for UncapturedEntry {
    fn from(wire: UncapturedWire) -> Self {
        match wire {
            UncapturedWire::Path(vault_path) => Self {
                vault_path,
                reason: None,
                size: None,
                modified_ns: None,
            },
            UncapturedWire::Entry(fields) => Self {
                vault_path: fields.vault_path,
                reason: fields.reason,
                size: fields.size,
                modified_ns: fields.modified_ns,
            },
        }
    }
}

/// What one snapshot saw ([`snapshot_coverage`]).
#[derive(Debug, Default)]
struct SnapshotCoverage {
    captured: Vec<SnapshotEntry>,
    uncaptured: Vec<UncapturedEntry>,
    /// Folders the walk couldn't list, vault-relative (`""` is the vault
    /// root): whether any file below them existed is unknown.
    unlisted: Vec<String>,
    /// A broker walk ([`snapshot_roots`]) met a link below one of its
    /// roots, which a write could follow out of them (F7 review).
    links: bool,
}

#[derive(Debug, Default)]
struct NotionMirrorMetadata {
    is_mirror: bool,
    notion_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunManifest {
    run: HistoryRun,
    /// Present only while a run is active. Final manifests retain just the
    /// changed before/after hashes, avoiding a full-vault manifest forever.
    #[serde(default)]
    baseline: Vec<SnapshotEntry>,
    /// Files present at the baseline that it holds no copy of (over 2 MiB,
    /// unreadable, or not a regular file). Cleared with the baseline when the
    /// run finishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    uncaptured: Vec<UncapturedEntry>,
    /// Folders the baseline couldn't list (merged plan slice 4). Cleared with
    /// the baseline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    unlisted: Vec<String>,
}

/// Just the `run` half of a manifest. Naming a lease holder must not
/// materialize an active run's full-vault baseline; serde skips it unread.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestHead {
    run: HistoryRun,
}

/// Who holds the full-vault lease, classified from the lease file's owner.
///
/// Run ids are resolved through their manifest (source, source id, start);
/// background owners are recognized by prefix before any manifest lookup,
/// because `validate_run_id` accepts `tracker-sync-…` and `history-clear` too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseHolder {
    /// An agent or restore run. `source` is `None` while the lease exists but
    /// its manifest has not been written yet (the run is starting).
    Run {
        id: String,
        source: Option<String>,
        source_id: Option<String>,
        started_at: Option<String>,
    },
    /// The lease names a run whose manifest is already finalized: its release
    /// failed or the process died between the two. Only startup reconciliation
    /// clears it.
    Stale {
        id: String,
    },
    TrackerSync,
    Scheduler,
    /// A short workspace edit from the rail (archive, rename, delete).
    WorkspaceEdit,
    /// A companion device's change being applied (`companion_inbox`).
    Companion,
    HistoryClear,
    /// A lease this build cannot classify (malformed, foreign, unreadable).
    Unknown,
}

impl LeaseHolder {
    /// A single-line, human-facing name for the holder.
    pub fn describe(&self) -> String {
        match self {
            LeaseHolder::Run {
                source, source_id, ..
            } => match source.as_deref() {
                None => "an automated run that is starting".into(),
                Some("chat") => "a chat turn".into(),
                Some("workflow") => match source_id.as_deref().and_then(holder_label) {
                    Some(slug) => format!("workflow `{slug}`"),
                    None => "a workflow run".into(),
                },
                Some("heartbeat") => "the heartbeat".into(),
                Some("ingest") => "an import".into(),
                Some("historyRestore") => "a history restore".into(),
                Some("ask") => "an Ask Hollow run".into(),
                Some(crate::context_compact::ARCHIVE_SOURCE) => "Archive older entries".into(),
                Some(other) => match holder_label(other) {
                    Some(label) => format!("an automated `{label}` run"),
                    None => "another automated run".into(),
                },
            },
            LeaseHolder::Stale { .. } => "a stale lease left by an earlier run".into(),
            LeaseHolder::TrackerSync => "task tracker sync".into(),
            LeaseHolder::Scheduler => "the workflow scheduler".into(),
            LeaseHolder::WorkspaceEdit => "a workspace edit".into(),
            LeaseHolder::Companion => "a companion device's change".into(),
            LeaseHolder::HistoryClear => "a history clear".into(),
            LeaseHolder::Unknown => "another automated run".into(),
        }
    }

    /// The refusal a caller shows when this holder blocks admission.
    pub fn busy_message(&self) -> String {
        let mut message = format!(
            "Hollow is busy: {} holds the vault; try again when it finishes",
            self.describe()
        );
        if matches!(self, LeaseHolder::Stale { .. } | LeaseHolder::Unknown) {
            message.push_str(" (if this persists, restart Hollow to reconcile a stale lease)");
        }
        message
    }

    /// The history run id, for an agent or restore run.
    pub fn run_id(&self) -> Option<&str> {
        match self {
            LeaseHolder::Run { id, .. } => Some(id.as_str()),
            _ => None,
        }
    }

    /// True when the holder is this workflow's own run.
    pub fn is_workflow(&self, slug: &str) -> bool {
        matches!(
            self,
            LeaseHolder::Run { source: Some(source), source_id: Some(id), .. }
                if source == "workflow" && id == slug
        )
    }

    /// True for the scheduler's own bookkeeping lease.
    pub fn is_scheduler(&self) -> bool {
        matches!(self, LeaseHolder::Scheduler)
    }
}

/// Keep a slug or source printable on one line: `[A-Za-z0-9 _.-]` only,
/// whitespace collapsed, bounded. `None` when nothing printable is left.
fn holder_label(value: &str) -> Option<String> {
    let kept: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(*c, ' ' | '_' | '.' | '-'))
        .collect();
    let label: String = kept
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_HOLDER_LABEL_CHARS)
        .collect();
    (!label.is_empty()).then_some(label)
}

/// Why a full-vault run could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginRunError {
    /// Another writer holds the lease. Nothing was written.
    Busy(LeaseHolder),
    /// The lease or the baseline could not be written (I/O, layout, snapshot).
    Failed(String),
}

impl std::fmt::Display for BeginRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BeginRunError::Busy(holder) => f.write_str(&holder.busy_message()),
            BeginRunError::Failed(error) => f.write_str(error),
        }
    }
}

impl From<BeginRunError> for String {
    fn from(error: BeginRunError) -> Self {
        error.to_string()
    }
}

/// Take the full-vault lease and snapshot the baseline, or say who holds it.
///
/// This is the single admission point for every full-vault writer. It is
/// atomic with the lease itself (O_EXCL, under `HISTORY_LOCK`), so a caller
/// that needs to know whether it may run must call this, not
/// [`lease_holder`], and must do nothing with side effects before it
/// succeeds.
///
/// Protected-change tracking ([`crate::run_guard::begin_tracking`]) starts
/// here, still under `HISTORY_LOCK`, so every admission path is covered: the
/// run-queue pump (workflows, the heartbeat and, since Paperclip PR 7b, chat
/// turns via `chat::admit_turn`), ingest and the heartbeat's Run now go
/// through [`try_begin_owned`], and
/// [`begin_run`] (the tool-path ingest) delegates to it. It must stay here: a run admitted without
/// an entry finishes in reconcile mode, which cannot see dot-state, agent
/// config or manifest edits and trusts the manifest's source. History
/// restores use `begin_run_inner` and are deliberately untracked.
pub fn try_begin_run(
    vault: &Path,
    source: &str,
    source_id: Option<&str>,
) -> Result<HistoryRunHandle, BeginRunError> {
    let result = {
        let _guard = lock_history();
        begin_run_inner(vault, source, source_id).inspect(|handle| {
            let mut history_files = Vec::with_capacity(2);
            if let Ok(manifest) = manifest_path(vault, &handle.id) {
                history_files.push(manifest);
            }
            history_files.push(active_run_path(vault));
            crate::run_guard::begin_tracking(
                vault,
                &handle.id,
                source.trim(),
                source_id.map(str::trim).filter(|id| !id.is_empty()),
                &history_files,
            );
        })
    };
    // A lease reclaimed on the way in announces itself outside the lock.
    drain_reclaim_notices(vault);
    result
}

/// Snapshot the vault before an automated run starts. The returned handle is
/// intentionally tiny; the durable baseline is already on disk before this
/// function returns. A thin wrapper over [`try_begin_run`] for callers that
/// only need the refusal text.
pub fn begin_run(
    vault: &Path,
    source: &str,
    source_id: Option<&str>,
) -> Result<HistoryRunHandle, String> {
    try_begin_run(vault, source, source_id).map_err(String::from)
}

/// An admitted full-vault run that finalizes itself if dropped (Paperclip
/// PR 7). Not `Clone` and only mintable by [`try_begin_owned`], unlike the
/// serializable [`HistoryRunHandle`], so holding one means this process
/// really took the lease for this run. [`RunLease::finish`] and the drop path
/// both go through the public [`finish_run`], so the run's protected-change
/// flags are computed and notified either way. Finishing takes
/// `HISTORY_LOCK`, the permission write guard and the review queue lock, so
/// never drop or finish one while holding the session store's `WRITE_LOCK`
/// or the run queue's `WAITERS` (inside a `with_write` closure, say).
#[derive(Debug)]
pub(crate) struct RunLease {
    vault: PathBuf,
    /// `Some` until `finish` consumes the lease.
    handle: Option<HistoryRunHandle>,
    /// When admission completed (after the baseline), on the monotonic
    /// clock: Paperclip PR 15c's `spawnMs` is measured from here.
    admitted: Instant,
    /// Admission's baseline duration, kept once `finish` took the handle.
    baseline_ms: Option<u64>,
    /// A scoped run's folder (PR B3); `None` for a full-vault lease.
    scope: Option<String>,
}

impl RunLease {
    /// The history run id this lease admitted.
    pub(crate) fn id(&self) -> &str {
        self.handle.as_ref().map(|h| h.id.as_str()).unwrap_or("")
    }

    /// How long admission's vault snapshot took (Paperclip PR 15c).
    pub(crate) fn baseline_ms(&self) -> Option<u64> {
        self.baseline_ms
    }

    /// When admission completed, monotonic: the preparation timeline's
    /// clock (agent effectiveness F8, `prep_timing`).
    pub(crate) fn admitted_at(&self) -> Instant {
        self.admitted
    }

    /// Milliseconds since admission completed, monotonic (Paperclip PR 15c).
    pub(crate) fn admitted_elapsed_ms(&self) -> u64 {
        u64::try_from(self.admitted.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// A scoped run's folder (build programme PR B3), or `None` when this
    /// lease holds the whole vault.
    pub(crate) fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    /// The vault this lease was taken on.
    pub(crate) fn vault(&self) -> &Path {
        &self.vault
    }

    /// Finalize the run with `status` and release the lease.
    pub(crate) fn finish(mut self, status: &str) -> Result<HistoryRun, String> {
        match self.handle.take() {
            Some(handle) => finish_run(&self.vault, handle, status),
            None => Err("the run lease was already finished".into()),
        }
    }
}

impl Drop for RunLease {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let id = handle.id.clone();
            if let Err(error) = finish_run(&self.vault, handle, "cancelled") {
                eprintln!("[history] could not finalize dropped run {id}: {error}");
            }
        }
    }
}

/// [`try_begin_run`], returning an owned [`RunLease`]. The run-queue pump
/// admits every automation through this (workflows via
/// `workflows::admit_runnable`, the heartbeat via its gate); interactive
/// ingest and the heartbeat's Run now button use it as direct holders. Chat
/// turns are pump rows since Paperclip PR 7b (`chat::admit_turn` is the only
/// chat caller); history restore uses the untracked `begin_run_inner`.
pub(crate) fn try_begin_owned(
    vault: &Path,
    source: &str,
    source_id: Option<&str>,
) -> Result<RunLease, BeginRunError> {
    let handle = try_begin_run(vault, source, source_id)?;
    Ok(RunLease {
        vault: vault.to_path_buf(),
        baseline_ms: handle.baseline_ms,
        handle: Some(handle),
        admitted: Instant::now(),
        scope: None,
    })
}

/// True when `vault`'s lease file still names `lease`'s run. A lease-held
/// caller uses this before skipping the cross-process permit: an agent that
/// rewrote `.hollow-history/active-run.nosync` makes it false, so the caller takes a
/// permit (the safe side), and the rewrite itself is flagged on the run as a
/// `historyManifest` change.
pub(crate) fn lease_is(vault: &Path, lease: &RunLease) -> bool {
    let id = lease.id();
    if id.is_empty() || !same_vault(vault, lease.vault()) {
        return false;
    }
    run_is_live(vault, id)
}

fn same_vault(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Wakes a task waiting for the full-vault lease to be released.
pub(crate) fn lease_released() -> &'static tokio::sync::Notify {
    &LEASE_RELEASED
}

fn notify_lease_released(vault: &Path) {
    #[cfg(test)]
    {
        let canonical = fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
        RELEASE_NOTICES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(canonical);
    }
    #[cfg(not(test))]
    {
        let _ = vault;
    }
    LEASE_RELEASED.notify_one();
}

/// True when a lease release on `vault` has notified since the process began.
#[cfg(test)]
fn release_notified(vault: &Path) -> bool {
    let canonical = fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    RELEASE_NOTICES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&canonical)
}

#[cfg(test)]
fn forget_release_notices(vault: &Path) {
    let canonical = fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    RELEASE_NOTICES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|seen| *seen != canonical);
}

/// Name whoever holds the full-vault lease, or `None` when it is free.
///
/// Advisory and racy: the answer can be stale by the time the caller acts.
/// Use it only to word a refusal or to SKIP work; [`try_begin_run`] is the
/// only admission, so never check this and then spawn. It takes no lock
/// (`try_begin_run` holds `HISTORY_LOCK` for a whole snapshot) and creates no
/// directory, so it is safe to call from any tick on a vault that has never
/// had a run.
///
/// A lease whose owner exited ([`lease_reclaimable`], Paperclip PR 18) reads
/// as free, so a caller that skips work while the vault is held goes on to
/// admit, and the admission reclaims it. Checking that may probe the lease
/// lock outside the vault, never anything inside it.
pub fn lease_holder(vault: &Path) -> Option<LeaseHolder> {
    let record = read_lease_record(vault)?;
    if lease_reclaimable(vault, &record) {
        return None;
    }
    Some(classify_owner(vault, &record.owner))
}

/// The refusal [`with_live_run`] returns when `run_id` no longer holds the
/// vault (it finished, or another writer holds the lease).
pub(crate) const LIVE_RUN_FINISHED_ERROR: &str = "Task writes are available only while this Hollow chat turn or automation holds the vault; this run has finished.";

/// Run `f` only while the run `run_id`, started by one of `sources`, is
/// live, as ONE step under `HISTORY_LOCK` (Paperclip PR 12). This is how a
/// write made on a run's behalf by another component (the `hollow_tasks` and
/// `hollow_app` brokers) is bound to that run without check-then-act:
/// [`finish_run`] diffs the vault and releases the run under the same lock,
/// so `f`'s writes land either wholly before the finalizing diff (and are
/// recorded in that run's history and receipt) or not at all. They can never
/// land unrecorded, or inside the NEXT run's baseline, even when a stop, a
/// timeout or a budget kill finishes the run while a request is in flight.
///
/// A full-vault run must hold the lease. A live scoped run (a workspace's
/// conversation, which holds only its folder) gets a write of its own
/// instead ([`scoped_live_write`]): Hollow takes the full-vault lease in
/// that run's name for just this call (waiting up to
/// [`SCOPED_WRITE_WAIT`] for another holder to finish), snapshots the vault
/// on both sides of `f` (only where it can write, when the caller says,
/// [`with_live_run_touching`]), and adds what changed outside every live
/// workspace folder to that run's History. That covers task files,
/// workspaces, goals and settings notes an agent changes through Hollow,
/// without giving the agent itself any access beyond its folder.
///
/// Anything else is refused with [`LIVE_RUN_FINISHED_ERROR`] before `f` runs:
/// no lease, a [`LeaseHolder::Run`] with another id, a source outside
/// `sources`, a run still snapshotting its baseline (`source: None`), a stale
/// lease, a lease whose owner process exited, and every background holder. The lease file and manifest are
/// agent-writable, so this guards against stale or foreign callers, not
/// against the running agent itself.
///
/// Lock order: `HISTORY_LOCK` → {`vault::MUTATE_LOCK`,
/// `vault_tasks::CREATE_LOCK`, `review_queue` `QUEUE_LOCK`} → run_guard
/// `TRACKED`. `f` must never call [`finish_run`], [`try_begin_run`],
/// [`begin_run`], [`list_runs`] or anything else that takes `HISTORY_LOCK`
/// (it is not reentrant), and must not take the permission write guard.
/// It blocks while a run is snapshotting, so call it on a blocking worker.
pub(crate) fn with_live_run<T>(
    vault: &Path,
    run_id: &str,
    sources: &[&str],
    f: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    with_live_run_touching(vault, run_id, sources, Touches::Vault, f)
}

/// Where a write made on a live run's behalf can change vault files (chat
/// concurrency F7, 2026-10-20). A scoped run's write snapshots only these
/// before and after `f` ([`scoped_live_write`]), instead of the whole vault
/// twice under `HISTORY_LOCK`, which queued every chat's task write behind
/// two full walks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Touches {
    /// Anywhere, or not known: the whole vault is snapshotted.
    Vault,
    /// Only files at or under these vault-relative paths (folders or
    /// files). A promise: a change `f` makes elsewhere goes unrecorded, so
    /// list every folder the write can create, change, move or delete in.
    Under(Vec<String>),
}

/// [`with_live_run`] for a write that changes files only where `touches`
/// says (chat concurrency F7, 2026-10-20). The admission, the lease and
/// every refusal are the same; only a scoped run's snapshots shrink.
pub(crate) fn with_live_run_touching<T>(
    vault: &Path,
    run_id: &str,
    sources: &[&str],
    touches: Touches,
    f: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let f = || {
        if !crate::shared_mcp::request_current() {
            return Err(LIVE_RUN_FINISHED_ERROR.to_string());
        }
        f()
    };
    #[cfg(test)]
    let wait = SCOPED_WRITE_WAIT_OVERRIDE.with(|value| value.get().unwrap_or(SCOPED_WRITE_WAIT));
    #[cfg(not(test))]
    let wait = SCOPED_WRITE_WAIT;
    let deadline = Instant::now() + wait;
    loop {
        let guard = lock_history();
        if !crate::shared_mcp::request_current() {
            return Err(LIVE_RUN_FINISHED_ERROR.to_string());
        }
        // A lease whose owner exited (Paperclip PR 18) names no live run.
        let holder = read_lease_record(vault)
            .filter(|record| !lease_reclaimable(vault, record))
            .map(|record| classify_owner(vault, &record.owner));
        let scope = live_scope_for_sources(vault, run_id, sources);
        let holds_vault = scope.is_none()
            && match &holder {
                Some(LeaseHolder::Run {
                    id,
                    source: Some(source),
                    ..
                }) => !run_id.is_empty() && id == run_id && sources.iter().any(|s| s == source),
                _ => false,
            };
        if holds_vault {
            #[cfg(test)]
            run_before_live_write_hook();
            let (result, written) = crate::vault::logging_writes(f);
            note_hollow_wrote(vault, run_id, &vault_relative_hashes(vault, &written));
            return result;
        }
        let Some(scope) = scope else {
            return Err(LIVE_RUN_FINISHED_ERROR.to_string());
        };
        // A lease this scoped run's own earlier write failed to release is
        // its own: let it go rather than wait on itself.
        if holder.as_ref().and_then(LeaseHolder::run_id) == Some(run_id) {
            release_run_lease(vault, run_id)?;
        }
        let taken = reclaim_locked(vault)
            .and_then(|reclaimed| acquire_lease_file_with(vault, run_id, reclaimed));
        match taken {
            Ok(()) => {
                let result = scoped_live_write(vault, run_id, &scope, &touches, f);
                drop(guard);
                drain_reclaim_notices(vault);
                return result;
            }
            Err(BeginRunError::Busy(holder)) => {
                drop(guard);
                drain_reclaim_notices(vault);
                if Instant::now() >= deadline {
                    return Err(format!(
                        "{} holds the vault, so this change couldn't be recorded in this conversation's History; nothing changed. Try again when it finishes.",
                        capitalized(&holder.describe())
                    ));
                }
                std::thread::sleep(SCOPED_WRITE_POLL);
            }
            Err(BeginRunError::Failed(error)) => {
                drop(guard);
                drain_reclaim_notices(vault);
                return Err(error);
            }
        }
    }
}

/// How long a scoped run's broker write waits for another holder of the
/// full-vault lease (an automation, a vault-root conversation, tracker sync)
/// before it is refused, and how often it looks.
#[cfg(not(test))]
const SCOPED_WRITE_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
#[cfg(test)]
const SCOPED_WRITE_WAIT: std::time::Duration = std::time::Duration::from_millis(300);
#[cfg(test)]
thread_local! {
    static SCOPED_WRITE_WAIT_OVERRIDE: std::cell::Cell<Option<std::time::Duration>> = const { std::cell::Cell::new(None) };
}
const SCOPED_WRITE_POLL: std::time::Duration = std::time::Duration::from_millis(100);

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The folder of `run_id` when it is a live scoped run that began with one
/// of `sources` (from the in-process registry, never the manifest).
fn live_scope_for_sources(vault: &Path, run_id: &str, sources: &[&str]) -> Option<String> {
    if run_id.is_empty() {
        return None;
    }
    let all = scopes();
    let state = all.get(&canonical_vault(vault))?;
    let source = state.live_sources.get(run_id)?;
    if !sources.iter().any(|s| s == source) {
        return None;
    }
    state.live.get(run_id).cloned()
}

/// The paths a scoped broker write's snapshots walk (chat concurrency F7,
/// 2026-10-20): `touches`, tidied (no trailing slash, no path inside
/// another), or `None` for the whole vault. That is the answer when
/// `touches` says so, and whenever a path is one History can't walk on its
/// own: hidden or escaping, or with a link (or a file) on the way or at
/// its end. The whole-vault walk never follows a link, but a write through
/// one changes the file it points at, which only a whole walk would see.
fn broker_roots(vault: &Path, touches: &Touches) -> Option<Vec<String>> {
    let Touches::Under(paths) = touches else {
        return None;
    };
    let mut roots = Vec::new();
    for path in paths {
        validate_vault_path(path).ok()?;
        let parts: Vec<String> = Path::new(path)
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let mut at = vault.to_path_buf();
        for (i, part) in parts.iter().enumerate() {
            at.push(part);
            match fs::symlink_metadata(&at) {
                Ok(meta) if meta.file_type().is_symlink() => return None,
                Ok(meta) if meta.is_dir() || i + 1 == parts.len() => {}
                Ok(_) => return None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => return None,
            }
        }
        roots.push(parts.join("/"));
    }
    roots.sort();
    roots.dedup();
    let inside = |root: &String| {
        roots
            .iter()
            .any(|other| other != root && Path::new(root).starts_with(other))
    };
    Some(roots.iter().filter(|root| !inside(root)).cloned().collect())
}

/// `f` for the live scoped run `run_id` (folder `scope`), with the
/// full-vault lease just taken in its name and `HISTORY_LOCK` held (see
/// [`with_live_run`]). What changed outside every live workspace folder is
/// added to the run's History. Its own folder is left out (its finish diffs
/// that), and so is every other live workspace's folder, as a full-vault
/// run leaves them out: those conversations may be writing there right now.
/// The lease is always released before this returns.
///
/// Both snapshots cover only what `touches` names (chat concurrency F7,
/// 2026-10-20; [`broker_roots`]), or the whole vault when it can't say:
/// nothing else writes outside the live folders while this holds the lease.
fn scoped_live_write<T>(
    vault: &Path,
    run_id: &str,
    scope: &str,
    touches: &Touches,
    f: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let mut roots = broker_roots(vault, touches);
    let snapshot = |roots: &Option<Vec<String>>| match roots {
        Some(roots) => snapshot_roots(vault, roots),
        None => snapshot_coverage(vault, None),
    };
    // Its before-versions are named only once the changes are recorded
    // (Q-R9), after the write itself.
    let objects = match objects_in_flight(vault) {
        Ok(objects) => objects,
        Err(error) => {
            let _ = release_run_lease(vault, run_id);
            return Err(format!(
                "History couldn't take its snapshot, so nothing ran: {error}"
            ));
        }
    };
    let mut before = snapshot(&roots);
    // A link inside a named folder (`projects/active` → `notes/`) lets the
    // write land outside it, where only the whole walk looks (F7 review,
    // 2026-10-20): snapshot the whole vault instead.
    if before.as_ref().is_ok_and(|before| before.links) {
        roots = None;
        before = snapshot(&roots);
    }
    let mut before = match before {
        Ok(before) => before,
        Err(error) => {
            let _ = release_run_lease(vault, run_id);
            return Err(format!(
                "History couldn't take its snapshot, so nothing ran: {error}"
            ));
        }
    };
    #[cfg(test)]
    run_before_live_write_hook();
    let (result, written) = crate::vault::logging_writes(f);
    let hashes = vault_relative_hashes(vault, &written);
    note_hollow_wrote(vault, run_id, &hashes);
    let written: BTreeSet<String> = hashes.into_keys().collect();
    // A file the write's own vault writes touched outside the folders it
    // named (F7 post-write check, 2026-10-20) was never in the before
    // snapshot: it is snapshotted after and recorded with no earlier
    // version known, never dropped. Dot-files History never diffs aren't.
    let escaped = escaped_writes(roots.as_deref(), &written);
    if !escaped.is_empty() {
        eprintln!("[history] a Hollow write for scoped run {run_id} changed {escaped:?} outside the folders it named; recorded with no earlier version");
        before.unlisted.extend(escaped.iter().cloned());
    }
    let after_roots = roots
        .as_ref()
        .map(|roots| roots.iter().chain(&escaped).cloned().collect());
    let recorded = snapshot(&after_roots).and_then(|after| {
        let mut changes = diff_snapshots(
            vault,
            &before.captured,
            &after.captured,
            Gaps::of(&before),
            Gaps::of(&after),
        )?;
        let live: Vec<String> = scopes()
            .get(&canonical_vault(vault))
            .map(|state| state.live.values().cloned().collect())
            .unwrap_or_default();
        // What it wrote in the run's own folder is left to the finish's
        // diff, but noted as the run's own (chat concurrency F1): no tool
        // call of the agent names it. Only the files its own vault writes
        // touched (`vault::logging_writes`): a peer's write landing in the
        // folder meanwhile is in the diff too, and isn't this run's.
        let in_folder: BTreeSet<String> = changes
            .iter()
            .filter(|change| under_scope(&change.vault_path, scope))
            .filter(|change| written.contains(&change.vault_path))
            .map(|change| change.vault_path.clone())
            .collect();
        if !in_folder.is_empty() {
            if let Some(state) = scopes().get_mut(&canonical_vault(vault)) {
                state
                    .on_behalf
                    .entry(run_id.to_string())
                    .or_default()
                    .extend(in_folder);
            }
        }
        changes.retain(|change| {
            !under_scope(&change.vault_path, scope)
                && !live.iter().any(|s| under_scope(&change.vault_path, s))
        });
        record_broker_changes(vault, run_id, changes)
    });
    drop(objects);
    let released = release_run_lease(vault, run_id);
    match (result, recorded) {
        (Ok(value), Ok(())) => released.map(|()| value),
        (Ok(_), Err(error)) => {
            eprintln!("[history] a change made for scoped run {run_id} went unrecorded: {error}");
            Err(format!("The change was made, but History couldn't record it for this conversation: {error}"))
        }
        (Err(error), recorded) => {
            if let Err(record_error) = recorded {
                eprintln!("[history] could not record a failed change for scoped run {run_id}: {record_error}");
            }
            Err(error)
        }
    }
}

/// The vault-relative files in `written` outside every one of `roots`
/// (none when the whole vault was snapshotted), leaving out what History
/// never diffs (dot-files, unsafe names).
fn escaped_writes(roots: Option<&[String]>, written: &BTreeSet<String>) -> Vec<String> {
    let Some(roots) = roots else {
        return Vec::new();
    };
    written
        .iter()
        .filter(|path| validate_vault_path(path).is_ok())
        .filter(|path| !roots.iter().any(|root| Path::new(path).starts_with(root)))
        .cloned()
        .collect()
}

/// Note what a Hollow write on live run `run_id`'s behalf wrote (`written`,
/// vault-relative), each file with the hash of the bytes that write put
/// there, taken by the write primitive as it wrote them
/// ([`crate::vault::WriteLog`]; `None`: moved, removed or unknown, never
/// credited), read at the run's finish by [`mark_hollow_writes`]. Not
/// hashed afterwards (review 2026-11-13): a write by the person, the run's
/// shell or another app between the broker's write and that later hash
/// would be taken for the broker's. A later write to the same file
/// replaces its hash.
fn note_hollow_wrote(vault: &Path, run_id: &str, written: &BTreeMap<String, Option<String>>) {
    if run_id.is_empty() || written.is_empty() {
        return;
    }
    let hashes = written
        .iter()
        .map(|(rel, hash)| (rel.clone(), hash.clone()));
    let mut all = scopes();
    let state = all.entry(canonical_vault(vault)).or_default();
    state
        .hollow_wrote
        .entry(run_id.to_string())
        .or_default()
        .extend(hashes);
}

/// Mark each change of run `run_id` whose final version is exactly what a
/// Hollow write on its behalf left ([`note_hollow_wrote`]) as
/// [`HistoryChange::hollow_write`] (2026-11-13). A file changed again by
/// anything else after that write (the agent's shell, the person, another
/// app) ends with another hash and stays unmarked.
fn mark_hollow_writes(vault: &Path, run_id: &str, changes: &mut [HistoryChange]) {
    let wrote = {
        let all = scopes();
        let Some(wrote) = all
            .get(&canonical_vault(vault))
            .and_then(|state| state.hollow_wrote.get(run_id))
        else {
            return;
        };
        wrote.clone()
    };
    for change in changes.iter_mut() {
        change.hollow_write = change.after_hash.is_some()
            && wrote
                .get(&change.vault_path)
                .is_some_and(|hash| *hash == change.after_hash);
    }
}

/// A write log's `paths` (absolute, as the vault's write primitives were
/// given them, each with the hash its latest write left) keyed by
/// vault-relative path; anything outside the vault is dropped. One file
/// logged under both spellings of the vault with different hashes gets
/// none: which write came last isn't known.
fn vault_relative_hashes(
    vault: &Path,
    paths: &BTreeMap<PathBuf, Option<String>>,
) -> BTreeMap<String, Option<String>> {
    let roots = [vault.to_path_buf(), canonical_vault(vault)];
    let mut relative: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (path, hash) in paths {
        let Some(rel) = roots.iter().find_map(|root| path.strip_prefix(root).ok()) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        match relative.get_mut(&rel) {
            Some(seen) if *seen != *hash => *seen = None,
            Some(_) => {}
            None => {
                relative.insert(rel, hash.clone());
            }
        }
    }
    relative
}

/// Merge `changes` into the live scoped run's broker changes and keep a
/// copy in its running manifest (for startup recovery). Under
/// `HISTORY_LOCK`. Hollow's own rewrite of the manifest is attributed to
/// the host, so run_guard doesn't flag it as the agent's.
fn record_broker_changes(
    vault: &Path,
    run_id: &str,
    changes: Vec<HistoryChange>,
) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let merged = {
        let mut all = scopes();
        let state = all.entry(canonical_vault(vault)).or_default();
        let entry = state.broker_changes.entry(run_id.to_string()).or_default();
        merge_changes(entry, changes);
        entry.clone()
    };
    let mut manifest = read_manifest(vault, run_id)?;
    if manifest.run.finished_at.is_some() {
        return Err("its history record was already finished".into());
    }
    manifest.run.changes = merged;
    let path = manifest_path(vault, run_id)?;
    let before = crate::run_guard::history_file_hash(&path);
    write_manifest(vault, &manifest)?;
    let after = crate::run_guard::history_file_hash(&path);
    crate::run_guard::note_host_history_write(run_id, &path, before.as_deref(), after.as_deref());
    Ok(())
}

/// Fold `later` changes into `earlier` by path: a path changed twice keeps
/// its first before and its last after, and one that provably ends as it
/// began is dropped (a side History holds no copy of never proves that).
/// Sorted by path.
fn merge_changes(earlier: &mut Vec<HistoryChange>, later: Vec<HistoryChange>) {
    for change in later {
        match earlier
            .iter()
            .position(|c| c.vault_path == change.vault_path)
        {
            None => earlier.push(change),
            Some(at) => {
                let first = earlier.remove(at);
                let (before, after) = (first.coverage_before(), change.coverage_after());
                if before.restorable() && before == after && first.before_hash == change.after_hash
                {
                    continue;
                }
                earlier.push(HistoryChange {
                    change_type: change_type_for(before, after).into(),
                    before_hash: first.before_hash,
                    before_size: first.before_size,
                    before_notion_id: first.before_notion_id,
                    before_uncaptured: !before.restorable(),
                    before_coverage: Some(before),
                    text: first.text && change.text,
                    notion_mirror: first.notion_mirror || change.notion_mirror,
                    ..change
                });
            }
        }
    }
    earlier.sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
}

/// The live scoped run's broker changes (see [`record_broker_changes`]).
fn broker_changes_for(vault: &Path, run_id: &str) -> Option<Vec<HistoryChange>> {
    scopes()
        .get(&canonical_vault(vault))
        .and_then(|state| state.broker_changes.get(run_id).cloned())
}

#[cfg(test)]
thread_local! {
    /// Runs inside [`with_live_run`], under `HISTORY_LOCK`, just before `f`.
    static BEFORE_LIVE_WRITE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

/// Arm a one-shot hook that [`with_live_run`] runs on this thread, under
/// `HISTORY_LOCK`, before its closure.
#[cfg(test)]
pub(crate) fn set_before_live_write_hook(hook: impl FnOnce() + 'static) {
    BEFORE_LIVE_WRITE.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
thread_local! {
    /// Runs inside [`clear_all_but`] after its checks, before it removes
    /// anything: a test swaps a history folder there.
    static BEFORE_CLEAR_DELETE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn set_before_clear_delete_hook(hook: impl FnOnce() + 'static) {
    BEFORE_CLEAR_DELETE.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_before_clear_delete_hook() {
    if let Some(hook) = BEFORE_CLEAR_DELETE.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

#[cfg(test)]
fn run_before_live_write_hook() {
    let hook = BEFORE_LIVE_WRITE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
thread_local! {
    /// Runs inside [`dead_lease_holder`] on this thread, after the caller's
    /// lock-free read looked dead and before the first read under
    /// `HISTORY_LOCK`.
    static BEFORE_DEAD_LEASE_READ: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

/// Arm a one-shot hook that [`dead_lease_holder`] runs on this thread before
/// it takes `HISTORY_LOCK`: a test parking the lease in a transient state
/// rewrites it only once the caller has looked (OF09), with no sleep.
#[cfg(test)]
pub(crate) fn set_before_dead_lease_read_hook(hook: impl FnOnce() + 'static) {
    BEFORE_DEAD_LEASE_READ.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

/// Disarm that hook; true when it was still armed (it never ran).
#[cfg(test)]
pub(crate) fn disarm_before_dead_lease_read_hook() -> bool {
    BEFORE_DEAD_LEASE_READ.with(|slot| slot.borrow_mut().take().is_some())
}

#[cfg(test)]
fn run_before_dead_lease_read_hook() {
    let hook = BEFORE_DEAD_LEASE_READ.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

/// How long [`dead_lease_holder`] waits before its second read.
const DEAD_LEASE_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// The lease holder when it is `Stale` or `Unknown` and STAYS that way, else
/// `None` (Paperclip PR 7b). A healthy lease reads that way for a moment:
/// [`acquire_lease_file`] creates `active-run.nosync` empty before writing and
/// fsyncing the owner (→ `Unknown`), and a finishing run writes its finalized
/// manifest before releasing the lease (→ `Stale`). So the lock-free
/// [`lease_holder`] alone must never fail interactive work. Both in-process
/// windows run under `HISTORY_LOCK`, so each read here takes it; the second
/// read, [`DEAD_LEASE_SETTLE`] later, covers another process's windows. Only
/// the same owner text classified the same way both times counts. Blocks for
/// a snapshot when a run is starting and sleeps briefly, so call it only on a
/// blocking worker and only after a lock-free read already looked dead.
///
/// A lease whose owner EXITED is not dead in this sense (Paperclip PR 18):
/// it is reclaimable, reads as free (`None`) in both reads, and the next
/// admission reclaims it, so interactive work is admitted rather than given
/// the restart hint.
pub(crate) fn dead_lease_holder(vault: &Path) -> Option<LeaseHolder> {
    dead_lease_holder_after(vault, DEAD_LEASE_SETTLE)
}

/// Hold `HISTORY_LOCK` the way an in-process acquire or finish does, so a
/// test can park the lease in one of its transient states.
#[cfg(test)]
pub(crate) fn hold_history_lock_for_test() -> HistoryGuard {
    lock_history()
}

fn dead_lease_holder_after(vault: &Path, settle: std::time::Duration) -> Option<LeaseHolder> {
    #[cfg(test)]
    run_before_dead_lease_read_hook();
    let read = || {
        let _guard = lock_history();
        let record = read_lease_record(vault)?;
        if lease_reclaimable(vault, &record) {
            return None;
        }
        let holder = classify_owner(vault, &record.owner);
        matches!(holder, LeaseHolder::Stale { .. } | LeaseHolder::Unknown)
            .then_some((record.owner, holder))
    };
    let first = read()?;
    std::thread::sleep(settle);
    let second = read()?;
    (first == second).then_some(second.1)
}

/// Compare the current vault to the persisted baseline and finalize a run.
/// Calling this twice is idempotent: an already-finalized run is returned.
///
/// The run's protected-change flags are computed while finalizing; a newly
/// finalized run with flags then gets its workflow reviews and ONE warning
/// notification, after `HISTORY_LOCK` is released. That happens even when
/// finalizing fails for a tracked run (its history record was deleted,
/// corrupted or forged, say): the error is still returned, but the flags are
/// never lost with it.
pub fn finish_run(
    vault: &Path,
    handle: HistoryRunHandle,
    status: &str,
) -> Result<HistoryRun, String> {
    // While the run is still live, so a merge it makes lands in its own
    // diff (concurrency PR 2, 2026-10-20). Its writes are host writes made
    // for this run ([`host_write_for`], PR 1): each takes `HISTORY_LOCK`
    // briefly, then the vault mutate lock, so call it holding neither.
    crate::shared_docs::run_finishing(vault, &handle.id);
    let result = {
        let _guard = lock_history();
        finish_run_inner(vault, &handle.id, status, FlagMode::Tracked)
    };
    match result {
        Ok(finalized) => {
            finalized.notify(vault);
            Ok(finalized.run)
        }
        Err(failure) => {
            if let Some(finalized) = &failure.flagged {
                finalized.notify(vault);
            }
            Err(failure.message)
        }
    }
}

/// One finished History row for something that changed no file, such as a
/// `hollow://` link opened from Linear (`linear_link.rs`). It takes neither
/// the full-vault lease nor a baseline (a link must open while a run is
/// live), only `HISTORY_LOCK` for the one manifest write, so it is never
/// restorable and holds no objects. `source` is what the row is labelled
/// with; `source_id` says which (an identifier, never a prompt).
pub fn record_event(
    vault: &Path,
    source: &str,
    source_id: Option<&str>,
) -> Result<HistoryRun, String> {
    let _guard = lock_history();
    ensure_history_dirs(vault)?;
    let now = Utc::now().to_rfc3339();
    let manifest = RunManifest {
        run: HistoryRun {
            id: new_run_id(),
            source: bounded(source.trim(), 120, "unknown"),
            source_id: source_id
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| bounded(s, 500, "")),
            started_at: now.clone(),
            finished_at: Some(now),
            status: "success".into(),
            changes: Vec::new(),
            flags: Vec::new(),
            scope: None,
            overlaps: Vec::new(),
        },
        baseline: Vec::new(),
        uncaptured: Vec::new(),
        unlisted: Vec::new(),
    };
    write_manifest(vault, &manifest)?;
    Ok(manifest.run)
}

/// Return newest-first history runs, including an in-flight run if one exists.
pub fn list_runs(vault: &Path, limit: usize) -> Result<Vec<HistoryRun>, String> {
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    let _guard = lock_history();
    let manifests = read_all_manifests(vault)?;
    let all: HashMap<String, HistoryRun> = manifests
        .into_iter()
        .map(|m| (m.run.id.clone(), current_paths(vault, &aliases, m.run)))
        .collect();
    let mut runs: Vec<HistoryRun> = all.values().cloned().collect();
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    runs.truncate(limit);
    recheck_all(&mut runs, &all);
    Ok(runs)
}

/// Return newest-first runs whose finalized change set includes `vault_path`.
pub fn list_note_history(
    vault: &Path,
    vault_path: &str,
    limit: usize,
) -> Result<Vec<HistoryRun>, String> {
    validate_vault_path(vault_path)?;
    // A note in a moved project's workspace folder had its old id in the
    // path when earlier runs changed it (`workspace_aliases`).
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    let _guard = lock_history();
    let all: HashMap<String, HistoryRun> = read_all_manifests(vault)?
        .into_iter()
        .map(|m| (m.run.id.clone(), current_paths(vault, &aliases, m.run)))
        .collect();
    let mut out: Vec<HistoryRun> = all
        .values()
        .filter(|run| {
            run.changes
                .iter()
                .any(|c| aliases.same_vault_path(&c.vault_path, vault_path))
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    out.truncate(limit);
    recheck_all(&mut out, &all);
    Ok(out)
}

pub fn get_run(vault: &Path, run_id: &str) -> Result<HistoryRun, String> {
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    let _guard = lock_history();
    let mut run = current_paths(vault, &aliases, read_manifest(vault, run_id)?.run);
    run.changes = recheck_current(vault, &aliases, &run);
    Ok(run)
}

/// A run recorded before Move Hollow home names a moved project's old
/// workspace folder (`workspaces/<old id>/...`); the move renamed it to the
/// new id (`workspace_aliases`). Name the folder it is in now, in memory only:
/// the manifest on disk keeps what was recorded. A path is left alone while
/// a folder with the old id still exists.
pub(crate) fn current_paths(
    vault: &Path,
    aliases: &crate::workspace_aliases::Aliases,
    mut run: HistoryRun,
) -> HistoryRun {
    if aliases.is_empty() {
        return run;
    }
    for change in &mut run.changes {
        if let Some(now) = current_path(vault, aliases, &change.vault_path) {
            change.vault_path = now;
        }
    }
    // A scoped run's folder too, so the overlap recheck (`under_scope`)
    // still matches its mapped paths.
    if let Some(now) = run
        .scope
        .as_deref()
        .and_then(|scope| current_path(vault, aliases, scope))
    {
        run.scope = Some(now);
    }
    run
}

/// One vault path as it is now ([`current_paths`]'s rule), or `None` when
/// it stays.
fn current_path(
    vault: &Path,
    aliases: &crate::workspace_aliases::Aliases,
    path: &str,
) -> Option<String> {
    let now = aliases.current_vault_path(path)?;
    let old_folder = path.split('/').take(2).collect::<Vec<_>>().join("/");
    (!vault.join(old_folder).exists()).then_some(now)
}

/// [`recheck_overlaps`] with each peer run read from disk, its paths named
/// as they are now ([`current_paths`]).
fn recheck_current(
    vault: &Path,
    aliases: &crate::workspace_aliases::Aliases,
    run: &HistoryRun,
) -> Vec<HistoryChange> {
    recheck_overlaps(run, &|id| {
        read_manifest(vault, id)
            .ok()
            .map(|m| current_paths(vault, aliases, m.run))
    })
}

/// Vault-relative paths in an active run's baseline: the files history
/// tracked when the run began. Hidden (dot-prefixed), oversized and non-regular
/// files are never in a baseline, so a path's absence does not prove the run
/// created it. A finished run's manifest keeps no baseline and yields an empty
/// set, so only ask while the run is still active.
pub fn baseline_paths(vault: &Path, handle: &HistoryRunHandle) -> Result<HashSet<String>, String> {
    let _guard = lock_history();
    let manifest = read_manifest(vault, &handle.id)?;
    if manifest.run.finished_at.is_some() {
        return Err(format!("history run {} already finished", handle.id));
    }
    Ok(manifest
        .baseline
        .into_iter()
        .map(|entry| entry.vault_path)
        .collect())
}

/// Read a UTF-8 before/after version. `Ok(None)` means the file did not exist
/// at that side of the change. Binary content remains safely stored and
/// restorable, but is intentionally not coerced through this text API.
pub fn read_version(
    vault: &Path,
    run_id: &str,
    vault_path: &str,
    version: &str,
) -> Result<Option<String>, String> {
    validate_vault_path(vault_path)?;
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    let _guard = lock_history();
    let manifest = read_manifest(vault, run_id)?;
    let change = manifest
        .run
        .changes
        .iter()
        .find(|c| c.vault_path == vault_path)
        .or_else(|| {
            manifest
                .run
                .changes
                .iter()
                .find(|c| aliases.same_vault_path(&c.vault_path, vault_path))
        })
        .ok_or_else(|| format!("run {run_id} has no change for {vault_path}"))?;
    let (hash, coverage) = match version {
        "before" => (change.before_hash.as_deref(), change.coverage_before()),
        "after" => (change.after_hash.as_deref(), change.coverage_after()),
        _ => return Err("version must be `before` or `after`".into()),
    };
    if !coverage.restorable() {
        return Err(coverage_error(vault_path, version, coverage));
    }
    let Some(hash) = hash else { return Ok(None) };
    let bytes = read_object(vault, hash)?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{version} version of {vault_path} is binary"))
}

/// Restore one changed file to its pre-run state. Unless `force` is true, the
/// current file must still match the recorded after-version. A separate
/// `historyRestore` run is snapshotted before the mutation and returned, so
/// this restore can itself be undone. Refused while a live conversation could
/// be writing the file ([`live_writer_refusal`]).
pub fn restore_change(
    vault: &Path,
    run_id: &str,
    vault_path: &str,
    force: bool,
) -> Result<HistoryRun, String> {
    validate_vault_path(vault_path)?;
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    let result = {
        let _guard = lock_history();
        restore_change_inner(vault, &aliases, run_id, vault_path, force)
    };
    drain_reclaim_notices(vault);
    result
}

/// Restore every change from a finalized run as one preflighted operation.
///
/// Unless force is true, every current path must match its recorded
/// after-version before any live vault path is touched. force bypasses only
/// that recorded-hash comparison; malformed manifests, unsafe paths,
/// symlinks, unsupported files, and changes that race the completed preflight
/// are always rejected.
pub fn restore_run(vault: &Path, run_id: &str, force: bool) -> Result<HistoryRun, String> {
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    let result = {
        let _guard = lock_history();
        restore_run_inner(vault, &aliases, run_id, force)
    };
    drain_reclaim_notices(vault);
    result
}

pub fn storage_status(vault: &Path) -> Result<HistoryStorageStatus, String> {
    let _guard = lock_history();
    storage_status_inner(vault)
}

/// Remove every history manifest and object, and with the tree every run's
/// repository sidecar (Paperclip PR 16). This never touches a non-hidden
/// vault file. A symlink at `.hollow-history` is unlinked rather than followed.
pub fn clear_history(vault: &Path) -> Result<(), String> {
    let result = {
        let _guard = lock_history();
        clear_history_locked(vault)
    };
    drain_reclaim_notices(vault);
    result
}

fn clear_history_locked(vault: &Path) -> Result<(), String> {
    validate_vault_root(vault)?;
    let root = history_root(vault);
    let meta = match fs::symlink_metadata(&root) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("stat history: {e}")),
    };
    if meta.file_type().is_symlink() || meta.is_file() {
        return fs::remove_file(&root).map_err(|e| format!("clear history: {e}"));
    }

    // A lease an exited process left (and its running manifest) is reclaimed
    // first, so the active-run check below sees it finalized.
    let reclaimed = reclaim_locked(vault).map_err(String::from)?;
    // Held through the removal (after the reclaim, whose finish runs its own
    // cleanup): a run another process is starting may have stored objects
    // its record doesn't name yet, and its record would then name versions
    // the clear removed (Q-R9).
    let Some(_objects) = objects_cleanup_lock(vault) else {
        return Err("History is saving a run's versions right now, so nothing was cleared. Try again in a moment.".into());
    };
    // A conversation running in a workspace keeps its record (F5,
    // 2026-10-20: the clear used to refuse while any ran). Only runs this
    // process admitted and still holds live count; any other running record
    // still refuses the clear.
    let live: BTreeSet<String> = scopes()
        .get(&canonical_vault(vault))
        .map(|state| state.live.keys().cloned().collect())
        .unwrap_or_default();
    let active = read_all_manifests(vault)?
        .into_iter()
        .find(|manifest| run_is_active(&manifest.run) && !live.contains(&manifest.run.id));
    if let Some(active) = active {
        return Err(format!(
            "cannot clear history while automated run {} ({}) is active",
            active.run.id, active.run.source
        ));
    }

    // Claim the same durable lease used by full-vault captures. Besides
    // rejecting a live run, this closes the cross-process check/delete race:
    // another begin_run cannot create a baseline while this tree is removed.
    acquire_lease_file_with(vault, HISTORY_CLEAR_OWNER, reclaimed).map_err(String::from)?;
    if !live.is_empty() {
        let cleared = clear_all_but(vault, &live);
        let released = release_run_lease(vault, HISTORY_CLEAR_OWNER);
        return cleared.and(released);
    }
    // Records first (review round 2 of #1383): every run record, step pin
    // file and as-of snapshot goes before any object, so a clear that fails
    // or is killed midway leaves stored versions no record names (collected
    // by a later cleanup), never a record whose versions are gone.
    let records = remove_history_records(&root);
    #[cfg(test)]
    let records = records.and_then(|()| clear_fault_before_objects());
    if let Err(error) = records {
        let _ = release_run_lease(vault, HISTORY_CLEAR_OWNER);
        return Err(format!("clear history: {error}"));
    }
    match fs::remove_dir_all(&root) {
        // The lease file went with the tree; its lock goes after it.
        Ok(()) => {
            forget_held(vault, HISTORY_CLEAR_OWNER);
            notify_lease_released(vault);
            Ok(())
        }
        Err(error) => {
            let _ = release_run_lease(vault, HISTORY_CLEAR_OWNER);
            Err(format!("clear history: {error}"))
        }
    }
}

/// [`clear_history`] beside live conversations (F5, 2026-10-20): remove
/// everything under `.hollow-history` except the lease this clear holds and
/// what the `live` scoped runs still need: their records and repository
/// sidecars, their step pins, and every stored version their baselines,
/// changes so far and step pins name (their finish diffs against those, and
/// a restore of what they changed reads them). Under `HISTORY_LOCK`, the
/// clear's lease and the object store lock held exclusively, so no run
/// starts, finishes or writes through Hollow meanwhile. A live record that
/// can't be read refuses before anything is removed.
fn clear_all_but(vault: &Path, live: &BTreeSet<String>) -> Result<(), String> {
    let manifests = live
        .iter()
        .map(|id| read_manifest(vault, id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!("History couldn't read a running conversation's record, so nothing was cleared: {error}")
        })?;
    let mut keep = referenced_hashes(&manifests);
    // A linked runs/, objects/ or step-pins/ folder would be unlinked below
    // with the live runs' records, versions or pins in it (delta review of
    // PR 861): refuse before anything is removed.
    for leaf in [RUNS_DIR, OBJECTS_DIR, STEP_PINS_DIR] {
        validate_existing_history_layout(vault, leaf).map_err(|error| {
            format!("History's folders aren't as expected, so nothing was cleared: {error}")
        })?;
    }
    // A pin file that can't be read would keep the file and lose its
    // versions (review of PR 861): refuse instead.
    for id in live {
        let pins = read_step_pins_checked(vault, id).map_err(|error| {
            format!("History couldn't read a running conversation's step versions, so nothing was cleared: {error}")
        })?;
        keep.extend(pins);
    }
    if let Some(state) = scopes().get(&canonical_vault(vault)) {
        for change in live
            .iter()
            .filter_map(|id| state.broker_changes.get(id))
            .flatten()
        {
            keep.extend(change.before_hash.iter().cloned());
            keep.extend(change.after_hash.iter().cloned());
        }
    }
    let kept_ids: HashSet<String> = live.iter().cloned().collect();
    #[cfg(test)]
    run_before_clear_delete_hook();
    let root = history_root(vault);
    let entries = fs::read_dir(&root).map_err(|e| format!("clear history: {e}"))?;
    // A protected folder that stopped being a real folder after the checks
    // above is left in place and reported, never unlinked (delta review of
    // PR 861).
    let mut left = Vec::new();
    // Objects last (review round 2 of #1383): every record goes first, so a
    // failure midway never leaves a record whose versions are gone.
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("clear history: {e}"))?;
    entries.sort_by_key(|entry| entry.file_name() == OBJECTS_DIR);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("clear history: {e}"))?;
        let real_dir = meta.is_dir() && !meta.file_type().is_symlink();
        let removed = match name.as_str() {
            ACTIVE_RUN_FILE | LEGACY_ACTIVE_RUN_FILE => continue,
            RUNS_DIR if real_dir => {
                clear_runs_but(&path, &kept_ids);
                continue;
            }
            OBJECTS_DIR if real_dir => {
                // `clear_runs_but` is best effort: a record it couldn't
                // remove keeps every version, and so does a `runs/` that
                // can't be listed (review round 3 of #1383).
                match unkept_record(vault, &kept_ids) {
                    Ok(None) => {}
                    Ok(Some(id)) => {
                        return Err(format!(
                            "History couldn't remove run {id}'s record, so it kept the stored versions"
                        ));
                    }
                    Err(error) => {
                        return Err(format!(
                            "History couldn't list its run records ({error}), so it kept the stored versions"
                        ));
                    }
                }
                #[cfg(test)]
                clear_fault_before_objects()?;
                gc_objects(vault, &keep, None)?;
                continue;
            }
            STEP_PINS_DIR if real_dir => {
                sweep_step_pins(vault, &kept_ids);
                continue;
            }
            RUNS_DIR | OBJECTS_DIR | STEP_PINS_DIR => {
                left.push(name);
                continue;
            }
            _ if real_dir => fs::remove_dir_all(&path),
            _ => fs::remove_file(&path),
        };
        removed.map_err(|e| format!("clear history: {name}: {e}"))?;
    }
    if !left.is_empty() {
        left.sort();
        return Err(format!(
            "History cleared the rest but left {}: it is no longer a plain folder",
            left.join(", ")
        ));
    }
    Ok(())
}

/// A run record (or repository sidecar) still in `runs/` for a run not in
/// `keep`, after [`clear_runs_but`]. A `runs/` that can't be listed, or an
/// entry that can't be read, is an error, never "no record left" (review
/// round 3 of #1383): the caller then keeps every stored version. A missing
/// `runs/` holds no record.
fn unkept_record(vault: &Path, keep: &HashSet<String>) -> Result<Option<String>, String> {
    let runs = history_root(vault).join(RUNS_DIR);
    let entries = match fs::read_dir(&runs) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        let id = name
            .strip_suffix(REPO_SIDECAR_SUFFIX)
            .or_else(|| name.strip_suffix(".json"))
            .unwrap_or(&name)
            .to_string();
        if !keep.contains(&id) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Every entry of the history folder but `objects/` and the lease files: run
/// records, step pins, as-of snapshots and caches. A link is unlinked, never
/// followed. Clear's first step, before its objects go with the tree.
fn remove_history_records(root: &Path) -> Result<(), String> {
    let entries = fs::read_dir(root).map_err(|e| e.to_string())?;
    // An entry that can't be read fails the clear before any object goes
    // (review round 3 of #1383), never skipped as if it weren't there.
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            OBJECTS_DIR | ACTIVE_RUN_FILE | LEGACY_ACTIVE_RUN_FILE
        ) {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("{name}: {e}"))?;
        let removed = if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

// Test fault: fail a clear after its records are gone, before any object.
#[cfg(test)]
thread_local! {
    pub(super) static CLEAR_FAULT_BEFORE_OBJECTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn clear_fault_before_objects() -> Result<(), String> {
    if CLEAR_FAULT_BEFORE_OBJECTS.with(std::cell::Cell::get) {
        return Err("injected failure before the objects".into());
    }
    Ok(())
}

/// Remove every entry of `runs/` but the kept runs' records and
/// repository sidecars (a symlink is unlinked, never followed).
fn clear_runs_but(runs: &Path, keep: &HashSet<String>) {
    let Ok(entries) = fs::read_dir(runs) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let id = name
            .strip_suffix(REPO_SIDECAR_SUFFIX)
            .or_else(|| name.strip_suffix(".json"))
            .unwrap_or_default();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if keep.contains(id) && !is_dir {
            continue;
        }
        let path = entry.path();
        let _ = if is_dir {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        };
    }
}

/// Write (`Some`) or remove (`None`) run `run_id`'s repository-changes
/// sidecar, `.hollow-history/runs/{run_id}.repo.json` (Paperclip PR 16).
/// Only while the lease still names that run (read through
/// [`read_lease_record`], PR 18), so a stale or foreign caller can never
/// attach changes to another run's record. Takes `HISTORY_LOCK` for the
/// check and the write only; the git capture itself runs before, outside
/// the lock. `Some` refuses anything over [`MAX_REPO_SIDECAR_BYTES`] and
/// writes through the atomic-write primitive, which replaces (never
/// follows) a symlink planted at the path. `None` removes a file the agent
/// may have planted there. The sidecar is display-only: restore never reads
/// it. Written before `finish_run`, so that finish's retention already
/// counts it.
pub(crate) fn store_repo_sidecar(
    vault: &Path,
    run_id: &str,
    bytes: Option<&[u8]>,
) -> Result<(), String> {
    let _guard = lock_history();
    let path = repo_sidecar_path(vault, run_id)?;
    let owner = read_lease_record(vault).map(|record| record.owner);
    let scoped_live = scopes()
        .get(&canonical_vault(vault))
        .is_some_and(|state| state.live.contains_key(run_id));
    if owner.as_deref() != Some(run_id) && !scoped_live {
        return Err(format!(
            "repository changes for run {run_id} were not stored: the vault lease no longer names this run"
        ));
    }
    match bytes {
        Some(bytes) => {
            if bytes.len() as u64 > MAX_REPO_SIDECAR_BYTES {
                return Err(format!(
                    "repository changes for run {run_id} exceed {} bytes",
                    MAX_REPO_SIDECAR_BYTES
                ));
            }
            ensure_history_dirs(vault)?;
            crate::vault::atomic_write(&path, bytes)
        }
        None => {
            if !validate_existing_history_layout(vault, RUNS_DIR)? {
                return Ok(());
            }
            remove_repo_sidecar(&path)
        }
    }
}

/// The bytes of run `run_id`'s repository-changes sidecar, or `None` when it
/// has none. Refuses a symlink or non-regular file and anything over
/// [`MAX_REPO_SIDECAR_BYTES`]; never creates a directory.
pub(crate) fn read_repo_sidecar_bytes(
    vault: &Path,
    run_id: &str,
) -> Result<Option<Vec<u8>>, String> {
    let _guard = lock_history();
    if !validate_existing_history_layout(vault, RUNS_DIR)? {
        return Ok(None);
    }
    let path = repo_sidecar_path(vault, run_id)?;
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("stat repository changes: {error}")),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("repository changes record is not a regular file".into());
    }
    if meta.len() > MAX_REPO_SIDECAR_BYTES {
        return Err("repository changes record is too large".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("open repository changes: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| format!("stat repository changes: {error}"))?
        .is_file()
    {
        return Err("repository changes record is not a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_REPO_SIDECAR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read repository changes: {error}"))?;
    if bytes.len() as u64 > MAX_REPO_SIDECAR_BYTES {
        return Err("repository changes record is too large".into());
    }
    Ok(Some(bytes))
}

fn repo_sidecar_path(vault: &Path, run_id: &str) -> Result<PathBuf, String> {
    validate_run_id(run_id)?;
    Ok(runs_root(vault).join(format!("{run_id}{REPO_SIDECAR_SUFFIX}")))
}

/// Bytes of run `run_id`'s sidecar when it is a regular file, else 0.
fn repo_sidecar_size(vault: &Path, run_id: &str) -> u64 {
    repo_sidecar_path(vault, run_id)
        .ok()
        .and_then(|path| fs::symlink_metadata(path).ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .unwrap_or(0)
}

fn remove_repo_sidecar(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove repository changes: {error}")),
    }
}

/// Remove every `{id}.repo.json` whose run is not in `keep` (its manifest
/// was pruned or never existed). Only names shaped like a Hollow run id
/// (`run-…`, passing `validate_run_id`) are sidecars; anything else is left
/// alone.
fn sweep_orphan_repo_sidecars(vault: &Path, keep: &HashSet<String>) {
    let Ok(entries) = fs::read_dir(runs_root(vault)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = name.strip_suffix(REPO_SIDECAR_SUFFIX) else {
            continue;
        };
        if !id.starts_with("run-") || validate_run_id(id).is_err() || keep.contains(id) {
            continue;
        }
        let path = entry.path();
        let is_link_or_file = fs::symlink_metadata(&path)
            .map(|meta| meta.is_file() || meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_link_or_file {
            let _ = fs::remove_file(path);
        }
    }
}

/// Finalize manifests left in `running` state by a crash or forced shutdown.
/// This is intended for application startup, when no legitimate run is live
/// in THIS process.
///
/// There is no in-memory baseline for such a run, so its flags come from the
/// manifest's change set (reconcile mode: workflow, `me/` and instruction
/// changes only). Runs finalized before an error still notify.
///
/// Spares the run another live process provably holds (Paperclip PR 18,
/// [`foreign_live_owner`]); see [`reconcile_interrupted_sparing`], which
/// `lib.rs` calls directly so the usage ledger can spare the same run.
#[cfg(test)]
pub fn reconcile_interrupted(vault: &Path) -> Result<usize, String> {
    reconcile_interrupted_sparing(vault, foreign_live_owner(vault).as_deref())
}

/// [`reconcile_interrupted`] that leaves `spare`'s manifest running and the
/// lease file in place when `spare` is `Some` (the owner another live Hollow
/// process holds the lease lock for; `lib.rs` computes it once with
/// [`foreign_live_owner`] and spares the same run in the usage ledger). With
/// `None` it behaves exactly as before PR 18: every running manifest is
/// finalized and any lease is cleared.
pub fn reconcile_interrupted_sparing(vault: &Path, spare: Option<&str>) -> Result<usize, String> {
    let spare = spare.map(str::trim).filter(|owner| !owner.is_empty());
    let mut finalized = Vec::new();
    let result = {
        let _guard = lock_history();
        (|| -> Result<usize, String> {
            let mut ids: Vec<String> = read_all_manifests(vault)?
                .into_iter()
                .filter(|m| m.run.finished_at.is_none() || m.run.status == "running")
                .map(|m| m.run.id)
                .filter(|id| Some(id.as_str()) != spare)
                .collect();
            ids.sort();
            for id in &ids {
                finalized.push(finish_run_inner(
                    vault,
                    id,
                    "interrupted",
                    FlagMode::Reconcile,
                )?);
            }
            // A crash can occur after the manifest was finalized (or before it
            // was created) but before its lease was removed. Startup
            // reconciliation is explicitly called before any legitimate new
            // run, so clear that orphan, unless another live process holds
            // it (its lock is held: the spared owner).
            if spare.is_none() {
                clear_stale_run_lease(vault)?;
            }
            // Removed only when the lease lock is free: an older build that
            // still takes the pre-ST6 name may be mid-run.
            let _ = settle_legacy_lease(vault);
            tidy_lease_conflict_copies(vault);
            Ok(ids.len())
        })()
    };
    for run in &finalized {
        run.notify(vault);
    }
    result
}

thread_local! {
    /// Whether this thread works under `HISTORY_LOCK`: it holds a
    /// [`HistoryGuard`], or it is the worker [`run_under_hold`] started and
    /// its holder is blocked waiting for. So [`host_write`] called inside
    /// [`with_live_run`] runs under that hold instead of waiting on it
    /// forever (chat concurrency F3, 2026-10-20).
    static HISTORY_LOCK_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `HISTORY_LOCK` (SE5, 2026-11-17): exclusive for everything that reads or
/// writes History, as the plain mutex it replaced was, and SHARED for a
/// scoped run's admission ([`begin_scoped`]) only. Scoped admissions then
/// walk their folders at the same time (ten chats starting together no
/// longer admit one at a time), while every other holder (a finish, a live
/// write, a restore, retention, a full-vault admission) still sees each
/// admission whole: registered, baseline walked and record written, or not
/// begun. Writer-preferring: once an exclusive holder waits, no new
/// admission starts, so a stream of chat starts can't hold a finish off.
/// Hand-rolled because `std::sync::RwLock` promises no policy.
#[derive(Default)]
struct HistoryLock {
    state: Mutex<HistoryLockState>,
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct HistoryLockState {
    writer: bool,
    readers: usize,
    writers_waiting: usize,
}

impl HistoryLock {
    fn state(&self) -> std::sync::MutexGuard<'_, HistoryLockState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wait<'a>(
        &self,
        state: std::sync::MutexGuard<'a, HistoryLockState>,
    ) -> std::sync::MutexGuard<'a, HistoryLockState> {
        self.changed.wait(state).unwrap_or_else(|e| e.into_inner())
    }

    fn lock_exclusive(&self) {
        let mut state = self.state();
        state.writers_waiting += 1;
        while state.writer || state.readers > 0 {
            state = self.wait(state);
        }
        state.writers_waiting -= 1;
        state.writer = true;
    }

    fn lock_shared(&self) {
        let mut state = self.state();
        while state.writer || state.writers_waiting > 0 {
            state = self.wait(state);
        }
        state.readers += 1;
    }

    fn unlock(&self, exclusive: bool) {
        let mut state = self.state();
        if exclusive {
            state.writer = false;
        } else {
            state.readers -= 1;
        }
        drop(state);
        self.changed.notify_all();
    }
}

/// `HISTORY_LOCK`, held exclusively: marks this thread as the holder until
/// dropped. Not `Send`, like the `MutexGuard` it replaced: the holder flag
/// is thread-local, so a guard dropped on another thread would clear the
/// wrong thread's flag (the `*const ()` marker keeps it on its thread).
pub(crate) struct HistoryGuard {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for HistoryGuard {
    fn drop(&mut self) {
        HISTORY_LOCK_HELD.with(|held| held.set(false));
        HISTORY_LOCK.unlock(true);
    }
}

fn lock_history() -> HistoryGuard {
    HISTORY_LOCK.lock_exclusive();
    HISTORY_LOCK_HELD.with(|held| held.set(true));
    HistoryGuard {
        _not_send: std::marker::PhantomData,
    }
}

/// `HISTORY_LOCK`, held shared by a scoped admission (SE5). It does not mark
/// the thread as a holder: nothing an admission calls writes on a live
/// run's behalf ([`host_write`] would take the lock exclusively, and a
/// thread holding it shared must never ask for that).
struct AdmissionGuard {
    /// Not `Send` either (see [`HistoryGuard`]).
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        HISTORY_LOCK.unlock(false);
    }
}

fn lock_history_for_admission() -> AdmissionGuard {
    HISTORY_LOCK.lock_shared();
    AdmissionGuard {
        _not_send: std::marker::PhantomData,
    }
}

/// True when this thread works under `HISTORY_LOCK` (see
/// [`HISTORY_LOCK_HELD`]).
pub(crate) fn history_lock_held_here() -> bool {
    HISTORY_LOCK_HELD.with(std::cell::Cell::get)
}

// Object store lock (Q-R9, 2026-11-13). `HISTORY_LOCK` keeps this process's
// object cleanup away from its own writes, but another Hollow process (the
// headless MCP host, a second window's process) has its own. Such a process
// stores a run's objects before the run record that names them exists, and
// cleanup keeps only what records, pins, as-of snapshots and its own
// process's incremental tip name, so a cleanup in between deleted them and a
// later restore failed at `read_restore_object`. A `flock` outside the vault
// (`lease_lock::objects_lock_path`) closes that gap: every write that stores
// or reuses objects holds it shared from before its snapshot until its
// record is written ([`objects_in_flight`]), and cleanup holds it
// exclusively ([`objects_cleanup_lock`]) from its first record read to its
// last unlink. Cleanup never waits long: when a write holds the lock it
// skips this round and the next finish tries again, so a skipped cleanup
// only leaves garbage for later, never removes anything. A write never
// stores or reuses an object without the lock (review round 2 of #1383):
// one that can't get it refuses, and its run goes without History's
// coverage the way a failed snapshot does. An unrecorded run is honest; a
// record that can't restore is not.

thread_local! {
    /// How many [`ObjectsInFlight`] guards this thread holds. Cleanup on a
    /// thread that still has objects in flight would conflict with its own
    /// shared lock (a second open file description), so it is refused.
    static OBJECTS_IN_FLIGHT_HERE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Test override of [`OBJECTS_SHARED_WAIT`], so a refusal test doesn't
    /// wait 30 s.
    #[cfg(test)]
    static OBJECTS_SHARED_WAIT_TEST: std::cell::Cell<Option<std::time::Duration>> = const { std::cell::Cell::new(None) };
}

/// How long a write that can't be deferred (a finish, a live write, a step
/// pin, an as-of snapshot) waits for another process's cleanup before it
/// refuses. A cleanup reads the run records and unlinks objects within
/// [`OBJECTS_CLEANUP_DELETE_WINDOW`], so it is normally done well before;
/// the bound keeps a stopped (SIGSTOP) process from wedging History.
const OBJECTS_SHARED_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// How long a run's admission waits before it reports History busy, so the
/// caller defers the run (the run queue and chat retry it) instead of
/// holding `HISTORY_LOCK` through a cleanup.
const OBJECTS_BEGIN_WAIT: std::time::Duration = std::time::Duration::from_millis(250);
/// How long after taking the exclusive lock a retention cleanup may still
/// delete: it checks before every unlink and, past this, stops and leaves
/// the rest for the next round. Writers never go ahead without the lock, so
/// this is not what keeps objects safe; it bounds how long a cleanup keeps
/// writers waiting, below [`OBJECTS_SHARED_WAIT`] so a cleanup alone does
/// not make a write refuse. Clear is not bounded by it.
const OBJECTS_CLEANUP_DELETE_WINDOW: std::time::Duration = std::time::Duration::from_secs(20);
const _: () = assert!(OBJECTS_CLEANUP_DELETE_WINDOW.as_millis() < OBJECTS_SHARED_WAIT.as_millis());
/// Tries cleanup makes for the exclusive lock, [`OBJECTS_LOCK_POLL`] apart,
/// before it skips this round.
const OBJECTS_CLEANUP_TRIES: usize = 50;
const OBJECTS_LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(5);

fn objects_shared_wait() -> std::time::Duration {
    #[cfg(test)]
    if let Some(wait) = OBJECTS_SHARED_WAIT_TEST.with(std::cell::Cell::get) {
        return wait;
    }
    OBJECTS_SHARED_WAIT
}

/// A write's shared hold on the vault's object store lock: while it lives no
/// cleanup in any process removes an object.
pub(crate) struct ObjectsInFlight {
    _lock: HeldLock,
}

impl Drop for ObjectsInFlight {
    fn drop(&mut self) {
        OBJECTS_IN_FLIGHT_HERE.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// Why a write got no [`ObjectsInFlight`]: a cleanup held the lock for the
/// whole wait, or the lock couldn't be opened or taken.
#[derive(Debug)]
enum ObjectsLockRefusal {
    Busy,
    Failed(String),
}

impl ObjectsLockRefusal {
    fn message(&self) -> String {
        match self {
            Self::Busy => "History's version store is being cleaned up by another Hollow process, so this change was not recorded".into(),
            Self::Failed(error) => format!("History couldn't lock its version store ({error}), so this change was not recorded"),
        }
    }
}

/// Hold `vault`'s object store lock shared for a write that stores or reuses
/// objects before its record names them. Take it before the snapshot (or
/// object write) and drop it once the record is on disk, and before any
/// [`enforce_retention`] on the same thread. Waits at most
/// [`OBJECTS_SHARED_WAIT`], then refuses: the caller stores nothing and its
/// run goes without History's coverage.
pub(crate) fn objects_in_flight(vault: &Path) -> Result<ObjectsInFlight, String> {
    objects_in_flight_within(vault, objects_shared_wait()).map_err(|refusal| {
        let message = refusal.message();
        eprintln!("[history] {message}");
        message
    })
}

/// [`objects_in_flight`] for a run's admission: a short wait, and `Busy`
/// when a cleanup holds the lock, so the caller defers the run.
fn objects_in_flight_for_begin(vault: &Path) -> Result<ObjectsInFlight, BeginRunError> {
    objects_in_flight_within(vault, OBJECTS_BEGIN_WAIT).map_err(|refusal| match refusal {
        ObjectsLockRefusal::Busy => BeginRunError::Busy(LeaseHolder::HistoryClear),
        failed @ ObjectsLockRefusal::Failed(_) => {
            let message = failed.message();
            eprintln!("[history] {message}");
            BeginRunError::Failed(message)
        }
    })
}

fn objects_in_flight_within(
    vault: &Path,
    wait: std::time::Duration,
) -> Result<ObjectsInFlight, ObjectsLockRefusal> {
    // Open and lock errors are retried within the same wait (a passing
    // failure such as descriptors running out), then refuse like a busy
    // lock: a write never goes ahead without it.
    let deadline = Instant::now() + wait;
    loop {
        let attempt = crate::lease_lock::open_objects_lock(vault)
            .map_err(|error| ObjectsLockRefusal::Failed(format!("open: {error}")))
            .and_then(|file| match HeldLock::try_take_shared(file) {
                Ok(lock) => Ok(lock),
                Err(std::fs::TryLockError::WouldBlock) => Err(ObjectsLockRefusal::Busy),
                Err(std::fs::TryLockError::Error(error)) => {
                    Err(ObjectsLockRefusal::Failed(format!("lock: {error}")))
                }
            });
        match attempt {
            Ok(lock) => {
                OBJECTS_IN_FLIGHT_HERE.with(|n| n.set(n.get() + 1));
                return Ok(ObjectsInFlight { _lock: lock });
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(OBJECTS_LOCK_POLL),
            Err(refusal) => return Err(refusal),
        }
    }
}

/// A cleanup's exclusive hold on the object store lock and when it was
/// taken: a retention cleanup may delete only within
/// [`OBJECTS_CLEANUP_DELETE_WINDOW`] of that.
struct CleanupHold {
    _lock: HeldLock,
    taken: Instant,
}

impl CleanupHold {
    /// Whether this cleanup may still unlink. Checked before every delete.
    fn may_delete(&self) -> bool {
        self.taken.elapsed() < OBJECTS_CLEANUP_DELETE_WINDOW
    }
}

/// `vault`'s object store lock, exclusively, for one object cleanup. `None`
/// (cleanup skips this round, logged) when a write in any process holds it
/// past [`OBJECTS_CLEANUP_TRIES`], when the lock can't be opened, or when
/// this thread itself has objects in flight.
fn objects_cleanup_lock(vault: &Path) -> Option<CleanupHold> {
    let in_flight_here = OBJECTS_IN_FLIGHT_HERE.with(std::cell::Cell::get);
    debug_assert_eq!(
        in_flight_here, 0,
        "object cleanup on a thread that still holds objects in flight"
    );
    if in_flight_here > 0 {
        eprintln!("[history] object cleanup skipped: this thread still has objects in flight");
        return None;
    }
    for attempt in 0..OBJECTS_CLEANUP_TRIES {
        let file = match crate::lease_lock::open_objects_lock(vault) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("[history] object cleanup skipped: could not open the object store lock: {error}");
                return None;
            }
        };
        match HeldLock::try_take(file) {
            Ok(lock) => {
                return Some(CleanupHold {
                    _lock: lock,
                    taken: Instant::now(),
                })
            }
            Err(std::fs::TryLockError::WouldBlock) if attempt + 1 < OBJECTS_CLEANUP_TRIES => {
                std::thread::sleep(OBJECTS_LOCK_POLL);
            }
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(error)) => {
                eprintln!("[history] object cleanup skipped: could not take the object store lock: {error}");
                return None;
            }
        }
    }
    eprintln!("[history] object cleanup skipped: a History write in progress holds the object store lock; the next one retries");
    None
}

/// Run `f` on a fresh thread that works under this thread's
/// `HISTORY_LOCK` hold, and wait for it (`vault::spawn_blocking_logged`,
/// when a Hollow app tool's command, run inside [`with_live_run`], hands its
/// work to a worker). The loan is structured (review 2026-10-20): this
/// thread is blocked in the join for the worker's whole life, so it can't
/// drop its [`HistoryGuard`] meanwhile, and only one worker borrows a hold
/// at a time. A fresh thread rather than inline, so `f` may itself block on
/// the async runtime. The worker's panic comes back as the `Err`.
pub(crate) fn run_under_hold<R: Send>(f: impl FnOnce() -> R + Send) -> std::thread::Result<R> {
    debug_assert!(history_lock_held_here());
    let f = crate::shared_mcp::carry(f);
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                struct Lent;
                impl Drop for Lent {
                    fn drop(&mut self) {
                        HISTORY_LOCK_HELD.with(|held| held.set(false));
                    }
                }
                HISTORY_LOCK_HELD.with(|held| held.set(true));
                let _lent = Lent;
                f()
            })
            .join()
    })
}

fn run_is_active(run: &HistoryRun) -> bool {
    run.finished_at.is_none() || run.status == "running"
}

fn active_run_path(vault: &Path) -> PathBuf {
    history_root(vault).join(ACTIVE_RUN_FILE)
}

/// A lease under the pre-ST6 name (an older build on this vault still takes
/// it). Busy while its holder may be alive: the vault's lease lock is held, or
/// its recorded pid is still running (an older build takes the file before
/// the lock). Removed only when nothing says it's alive: the lock is free and
/// the record is pre-PR-18 (no pid) or its owner exited ([`lease_reclaimable`]).
/// An unknown lock state with no live pid leaves it alone and doesn't block.
fn settle_legacy_lease(vault: &Path) -> Result<(), BeginRunError> {
    let Some(record) = read_legacy_lease_record(vault) else {
        return Ok(());
    };
    let busy = || {
        Err(BeginRunError::Busy(classify_owner(
            vault,
            record.owner.trim(),
        )))
    };
    match crate::lease_lock::probe(vault) {
        LockProbe::Held => busy(),
        LockProbe::Free if record.pid.is_none() || lease_reclaimable(vault, &record) => {
            let _ = fs::remove_file(history_root(vault).join(LEGACY_ACTIVE_RUN_FILE));
            Ok(())
        }
        LockProbe::Free => busy(),
        LockProbe::Unknown => match record.pid.map(crate::proc_info::pid_alive) {
            Some(Some(true)) => busy(),
            _ => Ok(()),
        },
    }
}

/// iCloud's conflict copies of the lease (`active-run 2`, `active-run 2.nosync`
/// …): never read as a lease, removed at startup. Only small regular files with
/// exactly that name shape go.
fn tidy_lease_conflict_copies(vault: &Path) {
    let Ok(entries) = fs::read_dir(history_root(vault)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let stem = name.strip_suffix(".nosync").unwrap_or(name);
        let copy = stem
            .strip_prefix("active-run ")
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
        if !copy {
            continue;
        }
        if fs::symlink_metadata(entry.path())
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_LEASE_BYTES)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// A plain lease for tests; every production caller reclaims first and goes
/// through [`acquire_lease_file_with`].
#[cfg(test)]
fn acquire_lease_file(vault: &Path, owner: &str) -> Result<(), BeginRunError> {
    acquire_lease_file_with(vault, owner, None)
}

/// Claim the one full-vault capture lease. `create_new` makes this exclusion
/// durable and atomic across async callers and even separate Hollow processes;
/// callers never wait while holding an async runtime worker. A refusal names
/// the recorded owner as a [`LeaseHolder`].
///
/// The file records a [`LeaseRecord`] with this process's pid and start
/// stamp (Paperclip PR 18). Right after the exclusive create, before the
/// record is written, this process takes the vault's lease lock and keeps it
/// in `HELD_OWNERS` until [`release_run_lease`] removes the file. When a
/// reclaim already holds that lock (`reclaimed`, from [`reclaim_locked`]) it
/// is reused instead of taken again: two descriptors of one process conflict
/// under `flock`, and letting it go in between would let another process's
/// reclaim in.
fn acquire_lease_file_with(
    vault: &Path,
    owner: &str,
    reclaimed: Option<HeldLock>,
) -> Result<(), BeginRunError> {
    let record = LeaseRecord {
        owner: bounded(owner, MAX_LEASE_OWNER_CHARS, "unknown"),
        pid: Some(own_pid()),
        started_at: crate::proc_info::own_start_micros(),
    };
    let payload = serde_json::to_string(&record)
        .map(|line| format!("{line}\n"))
        .map_err(|error| {
            BeginRunError::Failed(format!("encode automated history lease: {error}"))
        })?;
    if payload.len() as u64 > MAX_LEASE_BYTES {
        return Err(BeginRunError::Failed(
            "automated history lease owner is too long".into(),
        ));
    }
    ensure_history_dirs(vault).map_err(BeginRunError::Failed)?;
    // A reclaim already holds the lease lock, so its probe would read Held.
    if reclaimed.is_none() {
        settle_legacy_lease(vault)?;
    }
    let path = active_run_path(vault);
    let mut retried = false;
    let mut file = loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                match read_lease_owner(vault) {
                    Some(recorded) => {
                        return Err(BeginRunError::Busy(classify_owner(vault, &recorded)))
                    }
                    // The lease vanished between the refused create and the
                    // read. Every in-process release takes HISTORY_LOCK, which
                    // the caller holds, so only another process can do that:
                    // try exactly once more rather than name a holder that
                    // no longer exists.
                    None if !retried => {
                        retried = true;
                        continue;
                    }
                    None => return Err(BeginRunError::Busy(LeaseHolder::Unknown)),
                }
            }
            Err(error) => {
                return Err(BeginRunError::Failed(format!(
                    "acquire automated history lease: {error}"
                )))
            }
        }
    };
    // Any entry this process still keeps for the vault is stale: the
    // exclusive create just proved no lease file existed. Dropping it first
    // also unlocks its descriptor, which would otherwise conflict with ours.
    drop(take_held_for_vault(vault));
    let lock = reclaimed.or_else(|| crate::lease_lock::hold(vault));
    if let Err(error) = file
        .write_all(payload.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&path);
        drop(lock);
        return Err(BeginRunError::Failed(format!(
            "persist automated history lease: {error}"
        )));
    }
    HELD_OWNERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert((canonical_vault(vault), record.owner), lock);
    Ok(())
}

/// What the lease file records (Paperclip PR 18): the owner (a run id, a
/// background prefix, `history-clear`), and for a lease written by this
/// build the owning process's pid and start stamp. One line of JSON,
/// `{"owner":…,"pid":…,"startedAt":…}`; extra keys are ignored so the format
/// can grow. A legacy lease (a pre-PR-18 build, or a hand-written file) is
/// the bare owner text and parses with no pid, which is never reclaimable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LeaseRecord {
    pub(crate) owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pid: Option<i32>,
    /// The owning process's start ([`crate::proc_info::process_start_micros`]),
    /// compared with the live process's start to catch pid reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<u64>,
}

impl LeaseRecord {
    /// A lease that exists but cannot be read or parsed: owner `""`, which
    /// classifies as [`LeaseHolder::Unknown`].
    fn unreadable() -> Self {
        LeaseRecord {
            owner: String::new(),
            pid: None,
            started_at: None,
        }
    }
}

/// Parse lease file text: JSON when it starts with `{` (malformed JSON reads
/// as [`LeaseRecord::unreadable`]), else the legacy bare owner.
pub(crate) fn parse_lease(text: &str) -> LeaseRecord {
    let text = text.trim();
    if text.starts_with('{') {
        return match serde_json::from_str::<LeaseRecord>(text) {
            Ok(mut record) => {
                record.owner = record.owner.trim().to_string();
                record
            }
            Err(_) => LeaseRecord::unreadable(),
        };
    }
    LeaseRecord {
        owner: text.to_string(),
        pid: None,
        started_at: None,
    }
}

/// Read the lease record without taking a lock or creating a directory.
/// `None` means no lease (or no history tree we can trust); an unreadable,
/// oversized or non-regular lease reads as [`LeaseRecord::unreadable`].
/// Every reader of the lease file goes through this (or
/// [`read_lease_owner`]), never the raw text.
pub(crate) fn read_lease_record(vault: &Path) -> Option<LeaseRecord> {
    read_lease_record_at(vault, &active_run_path(vault))
}

/// A pre-ST6 `active-run` lease's record, read like [`read_lease_record`].
fn read_legacy_lease_record(vault: &Path) -> Option<LeaseRecord> {
    read_lease_record_at(vault, &history_root(vault).join(LEGACY_ACTIVE_RUN_FILE))
}

fn read_lease_record_at(vault: &Path, path: &Path) -> Option<LeaseRecord> {
    match fs::symlink_metadata(history_root(vault)) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => return None,
        Ok(_) => {}
        Err(_) => return None,
    }
    let path = path.to_path_buf();
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(LeaseRecord::unreadable()),
    };
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > MAX_LEASE_BYTES {
        return Some(LeaseRecord::unreadable());
    }
    match fs::read_to_string(&path) {
        Ok(recorded) => Some(parse_lease(&recorded)),
        // Released between the stat and the read.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(LeaseRecord::unreadable()),
    }
}

/// The recorded lease owner ([`read_lease_record`]'s owner). `Some("")`
/// means a lease exists but is malformed or unreadable, which classifies as
/// [`LeaseHolder::Unknown`].
pub(crate) fn read_lease_owner(vault: &Path) -> Option<String> {
    read_lease_record(vault).map(|record| record.owner)
}

fn own_pid() -> i32 {
    std::process::id() as i32
}

fn canonical_vault(vault: &Path) -> PathBuf {
    fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf())
}

/// Remove and return every `HELD_OWNERS` entry for `vault`. The caller drops
/// the result (unlocking) once it no longer needs the lease file to exist.
fn take_held_for_vault(vault: &Path) -> Vec<Option<HeldLock>> {
    let canonical = canonical_vault(vault);
    let mut held = HELD_OWNERS.lock().unwrap_or_else(|e| e.into_inner());
    let keys: Vec<(PathBuf, String)> = held
        .keys()
        .filter(|(held_vault, _)| *held_vault == canonical)
        .cloned()
        .collect();
    keys.into_iter()
        .filter_map(|key| held.remove(&key))
        .collect()
}

/// Forget this process's hold on `owner`'s lease (its lock is released when
/// the entry drops, after the mutex is released).
fn forget_held(vault: &Path, owner: &str) {
    let removed = HELD_OWNERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(canonical_vault(vault), owner.trim().to_string()));
    drop(removed);
}

/// True while this process holds, or is running, anything on `vault`: a
/// lease it created (`HELD_OWNERS`) or a tracked run (`run_guard`).
fn holds_in_process(vault: &Path) -> bool {
    let canonical = canonical_vault(vault);
    let held = HELD_OWNERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .any(|(held_vault, _)| *held_vault == canonical);
    held || crate::run_guard::is_tracking(vault)
}

/// The facts [`owner_exited`] decides from. The probes are injected so the
/// rule is testable without real processes.
pub(crate) struct ExitCtx<'a> {
    pub(crate) record: &'a LeaseRecord,
    pub(crate) own_pid: i32,
    /// This process holds a lease on the vault or tracks a run there.
    pub(crate) in_process_holds: bool,
    /// The vault's lease lock ([`crate::lease_lock::probe`]).
    pub(crate) lock: LockProbe,
    /// `Some(false)` when the pid is gone, `Some(true)` when it exists,
    /// `None` when that cannot be told (a permission error).
    pub(crate) alive: &'a dyn Fn(i32) -> Option<bool>,
    /// A live process's start stamp, when readable.
    pub(crate) start_of: &'a dyn Fn(i32) -> Option<u64>,
}

/// Whether a lease's owner provably EXITED, so its lease may be reclaimed.
/// True only when ALL hold: nothing in this process holds the vault, the
/// lease lock is free, the recorded pid is > 1 and not ours, and that pid is
/// gone or belongs to a process that started at a different time. Every
/// uncertain case (a legacy record, an unreadable lock, a permission error,
/// a live pid whose start is unknown or unrecorded) is false. Pure.
pub(crate) fn owner_exited(ctx: &ExitCtx<'_>) -> bool {
    if ctx.in_process_holds || ctx.lock != LockProbe::Free {
        return false;
    }
    let Some(pid) = ctx.record.pid else {
        return false;
    };
    if pid <= 1 || pid == ctx.own_pid {
        return false;
    }
    match (ctx.alive)(pid) {
        Some(false) => true,
        Some(true) => match (ctx.record.started_at, (ctx.start_of)(pid)) {
            (Some(recorded), Some(actual)) => recorded != actual,
            _ => false,
        },
        None => false,
    }
}

/// [`owner_exited`] wired to this process's state and the real probes. The
/// cheap exits (a legacy record, our own pid, an in-process hold) need no
/// probe at all. A process that may not finalize another process's run (the
/// headless CLI) does not count a dead owner whose manifest is still running
/// as reclaimable, so its advisory reads and its admissions agree.
pub(crate) fn lease_reclaimable(vault: &Path, record: &LeaseRecord) -> bool {
    let Some(pid) = record.pid else {
        return false;
    };
    let own = own_pid();
    if pid <= 1 || pid == own || holds_in_process(vault) {
        return false;
    }
    let exited = owner_exited(&ExitCtx {
        record,
        own_pid: own,
        in_process_holds: false,
        lock: crate::lease_lock::probe(vault),
        alive: &crate::proc_info::pid_alive,
        start_of: &crate::proc_info::process_start_micros,
    });
    exited && (may_finalize_reclaimed_runs() || !owner_has_running_manifest(vault, &record.owner))
}

/// The owner is a run whose manifest is still running.
fn owner_has_running_manifest(vault: &Path, owner: &str) -> bool {
    matches!(
        classify_owner(vault, owner),
        LeaseHolder::Run {
            source: Some(_),
            ..
        }
    )
}

/// The owner another LIVE process holds the lease for, if any (Paperclip
/// PR 18): a new-format record whose pid is not ours while the vault's lease
/// lock is held (by someone else: this process holds nothing at startup).
/// Startup reconciliation spares exactly this run, in history and in the
/// usage ledger. A spoofed live pid with a free lock is not spared, and a
/// legacy lease never is (both keep the pre-PR-18 clear-on-restart path).
pub fn foreign_live_owner(vault: &Path) -> Option<String> {
    // An older build mid-run holds the pre-ST6 name (night ST6 review).
    [read_lease_record(vault), read_legacy_lease_record(vault)]
        .into_iter()
        .flatten()
        .find_map(|record| {
            let pid = record.pid?;
            if pid == own_pid() || record.owner.trim().is_empty() {
                return None;
            }
            (crate::lease_lock::probe(vault) == LockProbe::Held)
                .then(|| record.owner.trim().to_string())
        })
}

/// One thing a reclaim still has to announce after `HISTORY_LOCK` is dropped.
#[derive(Debug)]
struct ReclaimNotice {
    /// Canonical vault, so a drain only takes its own vault's notices.
    vault: PathBuf,
    /// The reclaimed run's finalized record (its flags notify).
    finalized: Option<Finalized>,
    /// The exited owner's pid.
    pid: i32,
    /// The reclaimed run, whose `running` usage rows are closed.
    run_id: Option<String>,
    /// The lease was actually reclaimed (a health record says so).
    reclaimed: bool,
}

/// Whether this process may finalize another process's manifested run: only
/// the GUI (a registered notification app) can deliver PR 5's one
/// notification per flagged run.
fn may_finalize_reclaimed_runs() -> bool {
    #[cfg(test)]
    {
        if TEST_GUI_RECLAIM.with(|cell| cell.get()) {
            return true;
        }
    }
    crate::notifications::app_registered()
}

/// Reclaim a lease whose owner exited (the caller holds `HISTORY_LOCK`). See
/// the module doc. Never calls `reconcile_interrupted`: it finalizes exactly
/// the dead owner's run, and only after [`lease_reclaimable`] proved this
/// process tracks nothing on the vault.
///
/// Exclusive across processes: `HISTORY_LOCK` is per process, and removing
/// the dead owner's lease is a read, compare, unlink sequence, so two
/// processes reclaiming the same lease could otherwise unlink the lease the
/// faster one had just created. The reclaim therefore takes the vault's
/// lease lock BEFORE touching the file and keeps it: a concurrent reclaimer's
/// probe (or hold) then reads it held and backs off. The lock is returned so
/// the caller's [`acquire_lease_file_with`] keeps holding it for the new
/// lease; `None` when nothing was reclaimed (the lock, if taken, is dropped).
fn reclaim_locked(vault: &Path) -> Result<Option<HeldLock>, BeginRunError> {
    let Some(record) = read_lease_record(vault) else {
        return Ok(None);
    };
    if !lease_reclaimable(vault, &record) {
        return Ok(None);
    }
    #[cfg(test)]
    TEST_BEFORE_RECLAIM_HOLD.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(vault);
        }
    });
    // Another reclaimer (or a new holder) has the lock: leave it to them.
    let Some(lock) = crate::lease_lock::hold_quiet(vault) else {
        return Ok(None);
    };
    // With the lock held no other process can be mid-reclaim; the lease must
    // still be exactly the one judged reclaimable, or someone got there first.
    if read_lease_record(vault).as_ref() != Some(&record) {
        return Ok(None);
    }
    let pid = record.pid.unwrap_or_default();
    let owner = record.owner.trim().to_string();
    let canonical = canonical_vault(vault);
    let mut notices = Vec::with_capacity(1);
    // `lease_reclaimable` already left a manifested run alone in a process
    // that may not finalize it (the headless CLI leaves those for the GUI).
    if owner_has_running_manifest(vault, &owner) {
        if !may_finalize_reclaimed_runs() {
            return Ok(None);
        }
        eprintln!("[history] reclaiming lease {owner} left by exited process {pid}");
        match finish_run_inner(vault, &owner, "interrupted", FlagMode::Reconcile) {
            Ok(finalized) => notices.push(ReclaimNotice {
                vault: canonical,
                finalized: Some(finalized),
                pid,
                run_id: Some(owner),
                reclaimed: true,
            }),
            Err(failure) => {
                if let Some(finalized) = failure.flagged {
                    push_reclaim_notices(vec![ReclaimNotice {
                        vault: canonical,
                        finalized: Some(finalized),
                        pid,
                        run_id: None,
                        reclaimed: false,
                    }]);
                }
                return Err(BeginRunError::Failed(format!(
                    "reclaim a lease left by exited process {pid}: {}",
                    failure.message
                )));
            }
        }
    } else {
        eprintln!("[history] reclaiming lease {owner} left by exited process {pid}");
        release_run_lease(vault, &owner).map_err(|error| {
            BeginRunError::Failed(format!(
                "reclaim a lease left by exited process {pid}: {error}"
            ))
        })?;
        notices.push(ReclaimNotice {
            vault: canonical,
            finalized: None,
            pid,
            run_id: None,
            reclaimed: true,
        });
    }
    push_reclaim_notices(notices);
    Ok(Some(lock))
}

fn push_reclaim_notices(notices: Vec<ReclaimNotice>) {
    if notices.is_empty() {
        return;
    }
    RECLAIM_NOTICES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(notices);
}

/// Announce `vault`'s reclaims: the reclaimed run's flags notification, one
/// health record, and its (and its delegates') `running` usage rows closed as
/// `interrupted`. Call only after `HISTORY_LOCK` is dropped: this verifies
/// flagged workflows, writes a notification and takes the session store's
/// `WRITE_LOCK`. Other vaults' notices stay queued for their own drain.
fn drain_reclaim_notices(vault: &Path) {
    let canonical = canonical_vault(vault);
    let mine: Vec<ReclaimNotice> = {
        let mut all = RECLAIM_NOTICES.lock().unwrap_or_else(|e| e.into_inner());
        if all.is_empty() {
            return;
        }
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *all)
            .into_iter()
            .partition(|notice| notice.vault == canonical);
        *all = rest;
        mine
    };
    for notice in mine {
        if let Some(finalized) = &notice.finalized {
            finalized.notify(vault);
        }
        if notice.reclaimed {
            crate::health::record(
                vault,
                "history",
                crate::health::Outcome::Partial,
                format!(
                    "Reclaimed the vault lease from a Hollow process that exited (pid {})",
                    notice.pid
                ),
            );
        }
        if let Some(run_id) = &notice.run_id {
            let at = crate::session_store::db_time(Utc::now());
            if let Err(error) =
                crate::session_store::interrupt_running_usage_for_run(vault, run_id, &at)
            {
                eprintln!(
                    "[history] could not close the usage rows of reclaimed run {run_id}: {error}"
                );
            }
        }
    }
}

/// Classify a recorded owner. Background prefixes are matched first because
/// `validate_run_id` also accepts them.
fn classify_owner(vault: &Path, owner: &str) -> LeaseHolder {
    let owner = owner.trim();
    if owner.starts_with(TRACKER_SYNC_OWNER_PREFIX) {
        return LeaseHolder::TrackerSync;
    }
    if owner.starts_with(SCHEDULER_OWNER_PREFIX) {
        return LeaseHolder::Scheduler;
    }
    if owner.starts_with(WORKSPACE_EDIT_OWNER_PREFIX) {
        return LeaseHolder::WorkspaceEdit;
    }
    if owner.starts_with(COMPANION_OWNER_PREFIX) {
        return LeaseHolder::Companion;
    }
    if owner == HISTORY_CLEAR_OWNER {
        return LeaseHolder::HistoryClear;
    }
    if validate_run_id(owner).is_err() {
        return LeaseHolder::Unknown;
    }
    match read_manifest_run(vault, owner) {
        Ok(run) if !run_is_active(&run) => LeaseHolder::Stale { id: run.id },
        Ok(run) => LeaseHolder::Run {
            id: run.id,
            source: Some(run.source),
            source_id: run.source_id,
            started_at: Some(run.started_at),
        },
        // begin_run_inner writes the lease before the manifest, so a run id
        // without one is a run that is still snapshotting its baseline.
        Err(_) if owner.starts_with("run-") => LeaseHolder::Run {
            id: owner.to_string(),
            source: None,
            source_id: None,
            started_at: None,
        },
        Err(_) => LeaseHolder::Unknown,
    }
}

/// Exclude agent captures while a background writer (tracker sync, the
/// workflow scheduler's bookkeeping) changes local files. No history run is
/// created for these short writes. A crash leaves the existing lease for the
/// next admission to reclaim once its owner provably exited (Paperclip
/// PR 18: a new-format record, a free lease lock, a gone or reused pid);
/// otherwise (a legacy lease, a live owner, an unreadable lock) for the
/// normal startup recovery path instead of guessing it is stale.
#[derive(Debug)]
pub(crate) struct BackgroundWriteLease {
    vault: PathBuf,
    owner: String,
}
impl Drop for BackgroundWriteLease {
    fn drop(&mut self) {
        let _guard = lock_history();
        let _ = release_run_lease(&self.vault, &self.owner);
        if let Some(state) = scopes().get_mut(&canonical_vault(&self.vault)) {
            state.sync_folders.remove(&self.owner);
        }
    }
}

/// `Busy` naming a live conversation (a scoped run) that holds a folder.
fn busy_chat(id: String) -> BeginRunError {
    BeginRunError::Busy(LeaseHolder::Run {
        id,
        source: Some("chat".into()),
        source_id: None,
        started_at: None,
    })
}

/// What a sync lease writes: `None` for every folder (a caller that can't
/// say), `Some` for just those folders.
type SyncFolders<'a> = Option<&'a BTreeSet<String>>;

fn background_lease(
    vault: &Path,
    prefix: &str,
    folders: SyncFolders,
) -> Result<BackgroundWriteLease, BeginRunError> {
    let result = {
        let _guard = lock_history();
        (|| -> Result<BackgroundWriteLease, BeginRunError> {
            validate_vault_root(vault).map_err(BeginRunError::Failed)?;
            if prefix == TRACKER_SYNC_OWNER_PREFIX && folders.is_none() {
                if let Some(id) = sync_blocker(vault, folders) {
                    return Err(busy_chat(id));
                }
            }
            let reclaimed = reclaim_locked(vault)?;
            let owner = format!("{prefix}{}", new_run_id());
            acquire_lease_file_with(vault, &owner, reclaimed)?;
            if let Some(folders) = folders.filter(|_| prefix == TRACKER_SYNC_OWNER_PREFIX) {
                scopes()
                    .entry(canonical_vault(vault))
                    .or_default()
                    .sync_folders
                    .insert(owner.clone(), folders.clone());
            }
            Ok(BackgroundWriteLease {
                vault: vault.into(),
                owner,
            })
        })()
    };
    drain_reclaim_notices(vault);
    result
}

/// The lease a workspace edit from the rail holds for its write (archive,
/// rename, status, delete). Refuses while any run holds the vault.
pub(crate) fn workspace_edit_lease(vault: &Path) -> Result<BackgroundWriteLease, BeginRunError> {
    background_lease(vault, WORKSPACE_EDIT_OWNER_PREFIX, None)
}

/// The lease the companion applier holds while it writes one device op.
/// Refuses while any run holds the vault; the op then waits in the inbox.
pub(crate) fn companion_write_lease(vault: &Path) -> Result<BackgroundWriteLease, BeginRunError> {
    background_lease(vault, COMPANION_OWNER_PREFIX, None)
}

/// A tracker sync lease whose folders the caller can't name: it holds the
/// whole vault and every conversation. Tests use it as "a background
/// writer holds the vault" (it replaced the Notion lease on 2026-11-10).
#[cfg(test)]
pub(crate) fn background_write_lease(vault: &Path) -> Result<BackgroundWriteLease, BeginRunError> {
    background_lease(vault, TRACKER_SYNC_OWNER_PREFIX, None)
}

/// Tracker sync keeps root writers excluded, but scoped chats run beside
/// it, including in its mirror folders. Every mirror commit must use atomic
/// CAS/create inside `host_write`, with no network wait under History.
pub(crate) fn tracker_mirror_write_lease(
    vault: &Path,
    folders: &BTreeSet<String>,
) -> Result<BackgroundWriteLease, BeginRunError> {
    background_lease(vault, TRACKER_SYNC_OWNER_PREFIX, Some(folders))
}

/// The lease the workflow scheduler holds for its own `workflows/*.md`
/// bookkeeping, so a skip row or `next_run_at` heal is never written inside
/// (and attributed to) another run's capture. Hold it only for the write and
/// drop it before admitting a run, or the admission refuses on this lease.
pub(crate) fn scheduler_write_lease(vault: &Path) -> Result<BackgroundWriteLease, BeginRunError> {
    background_lease(vault, SCHEDULER_OWNER_PREFIX, None)
}

/// Remove the lease only when it still belongs to `owner` (compared through
/// [`parse_lease`], so both the JSON record and a legacy bare owner match).
/// A mismatched lease may represent another process and is never unlinked by
/// a finishing run. Whatever happens, this process stops holding `owner`'s
/// lease: its `HELD_OWNERS` entry (and so its lease lock) is dropped AFTER
/// the file is gone.
/// Release a finishing run: a scoped run's folder in the registry, a
/// full-vault run's lease file (then its registry entry).
fn release_for(vault: &Path, run_id: &str, scope: Option<&str>) -> Result<(), String> {
    if scope.is_some() {
        // A broker write took the full-vault lease in this run's name only
        // for its call and released it; this clears one whose release
        // failed. Best effort: it removes nothing another owner holds, and a
        // lease this run never held can't fail its finish.
        let _ = release_run_lease(vault, run_id);
        forget_registry_run(vault, run_id);
        return Ok(());
    }
    let released = release_run_lease(vault, run_id);
    forget_registry_run(vault, run_id);
    released
}

fn validate_scope(vault: &Path, scope: &str) -> Result<(), String> {
    validate_vault_path(scope)?;
    let mut parts = scope.split('/');
    if parts.next() != Some("workspaces")
        || parts.next().is_none_or(str::is_empty)
        || parts.next().is_some()
    {
        return Err(format!(
            "a scoped run's folder must be workspaces/<id>, not {scope}"
        ));
    }
    match fs::symlink_metadata(vault.join(scope)) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
        _ => Err(format!("{scope} is not a workspace folder")),
    }
}

/// Admit a scoped run (build programme PR B3): a conversation in one
/// workspace (a build workspace's worktree outside the vault, or since
/// 2026-09-28 any workspace's own vault folder), writing its vault folder
/// `scope` (`workspaces/<id>`), so it needs no full-vault lease and runs
/// beside full-vault runs. Its baseline and diff cover only `scope`; every
/// full-vault run that overlaps it leaves `scope` out of its own diff.
/// Tracked by `run_guard` like a full-vault run, so its protected-change
/// flags still come from memory (a flag may also name a concurrent run's
/// change; see the build log).
///
/// This admits an EXCLUSIVE run (a workspace archive, a branch update's
/// reservation; a build worktree's turn holds its worktree's lock instead
/// since F6 part B, [`try_begin_scoped_mode`]): it is `Busy`, naming one of them, while any run is live in
/// `scope`, and keeps every other run out while it is. For a workspace's
/// own folder, where several conversations may run at once, use
/// [`try_begin_scoped_shared`]; how many the run pump starts at once is its
/// own rule (`run_pump::QueueFacts::hold`, 2026-10-20).
pub(crate) fn try_begin_scoped(
    vault: &Path,
    scope: &str,
    source: &str,
    source_id: Option<&str>,
) -> Result<RunLease, BeginRunError> {
    begin_scoped(vault, scope, source, source_id, ScopeMode::Exclusive)
}

/// Admit a SHARED scoped run (parallel chats in one workspace, 2026-10-07):
/// like [`try_begin_scoped`], but beside other shared runs in the same
/// folder; only an exclusive one there is `Busy`. Each run's diff still
/// covers the whole folder, so what overlapping runs both changed is marked
/// on both (`HistoryChange::also_changed_by`; the first to finish marks its
/// side from the live peer's baseline, the second both), and a restore
/// refuses those files unless forced. A chat turn's own writes decide what
/// is credited to it (`HistoryChange::unattributed`, [`note_own_writes`]).
/// Tests only since F6 part B: chat turns pass their mode to
/// [`try_begin_scoped_mode`].
#[cfg(test)]
pub(crate) fn try_begin_scoped_shared(
    vault: &Path,
    scope: &str,
    source: &str,
    source_id: Option<&str>,
) -> Result<RunLease, BeginRunError> {
    begin_scoped(vault, scope, source, source_id, ScopeMode::Shared)
}

/// Admit a scoped run in `mode` ([`ScopeMode`]): a build worktree's turn
/// passes [`ScopeMode::Worktree`] with its lock key, so it is `Busy` only
/// while another run holds that key (or the folder alone), and runs beside
/// the folder's own conversations and the workspace's other worktrees
/// (F6 part B, 2026-11-18).
pub(crate) fn try_begin_scoped_mode(
    vault: &Path,
    scope: &str,
    mode: ScopeMode,
    source: &str,
    source_id: Option<&str>,
) -> Result<RunLease, BeginRunError> {
    begin_scoped(vault, scope, source, source_id, mode)
}

fn begin_scoped(
    vault: &Path,
    scope: &str,
    source: &str,
    source_id: Option<&str>,
    mode: ScopeMode,
) -> Result<RunLease, BeginRunError> {
    let handle = {
        // Shared (SE5): other scoped admissions walk beside this one; the
        // registry ([`scopes`]) decides who is admitted, atomically.
        let _guard = lock_history_for_admission();
        begin_scoped_inner(vault, scope, source, source_id, &mode).inspect(|handle| {
            let history_files: Vec<PathBuf> =
                manifest_path(vault, &handle.id).into_iter().collect();
            crate::run_guard::begin_tracking(
                vault,
                &handle.id,
                source.trim(),
                source_id.map(str::trim).filter(|id| !id.is_empty()),
                &history_files,
            );
        })
    }?;
    // What the shared docs read at its start (concurrency PR 2).
    crate::shared_docs::run_began(vault, &handle.id, scope);
    Ok(RunLease {
        vault: vault.to_path_buf(),
        baseline_ms: handle.baseline_ms,
        handle: Some(handle),
        admitted: Instant::now(),
        scope: Some(scope.to_string()),
    })
}

fn begin_scoped_inner(
    vault: &Path,
    scope: &str,
    source: &str,
    source_id: Option<&str>,
    mode: &ScopeMode,
) -> Result<HistoryRunHandle, BeginRunError> {
    validate_vault_root(vault).map_err(BeginRunError::Failed)?;
    validate_scope(vault, scope).map_err(BeginRunError::Failed)?;
    // Known tracker mirror writers commit atomically per file. Only an
    // unknown writer retains the conservative admission hold.
    if let Some(owner) = read_lease_owner(vault) {
        let owner = owner.trim();
        if owner.starts_with(TRACKER_SYNC_OWNER_PREFIX) && sync_blocks_chat(vault, owner) {
            return Err(BeginRunError::Busy(LeaseHolder::TrackerSync));
        }
    }
    ensure_history_dirs(vault).map_err(BeginRunError::Failed)?;
    let id = new_run_id();
    {
        let mut all = scopes();
        let state = all.entry(canonical_vault(vault)).or_default();
        if let Some(holder) = blocker_in(state, scope, mode) {
            return Err(BeginRunError::Busy(LeaseHolder::Run {
                id: holder,
                source: Some(source.trim().to_string()),
                source_id: None,
                started_at: None,
            }));
        }
        let sharing: Vec<String> = state
            .live
            .iter()
            .filter(|(_, s)| s.as_str() == scope)
            .map(|(peer, _)| peer.clone())
            .collect();
        for peer in &sharing {
            state
                .peers
                .entry(peer.clone())
                .or_default()
                .insert(id.clone());
        }
        state
            .peers
            .insert(id.clone(), sharing.into_iter().collect());
        match mode {
            ScopeMode::Exclusive => {
                state.exclusive.insert(id.clone());
            }
            ScopeMode::Worktree(key) => {
                state.locks.insert(id.clone(), key.clone());
            }
            ScopeMode::PrimaryWorktree(key) => {
                state.locks.insert(id.clone(), key.clone());
                state.given_primary.insert(id.clone());
            }
            ScopeMode::PrimaryFolder => {
                state.given_primary.insert(id.clone());
            }
            ScopeMode::Shared => {}
        }
        state.live.insert(id.clone(), scope.to_string());
        state
            .live_sources
            .insert(id.clone(), source.trim().to_string());
    }
    let started_at = Utc::now().to_rfc3339();
    let baseline_started = Instant::now();
    // The baseline's objects are named once the record is written (Q-R9).
    let objects = match objects_in_flight_for_begin(vault) {
        Ok(objects) => objects,
        Err(error) => {
            forget_unadmitted(vault, &id);
            return Err(error);
        }
    };
    let baseline = match snapshot_coverage(vault, Some(scope)) {
        Ok(coverage) => coverage,
        Err(error) => {
            forget_unadmitted(vault, &id);
            return Err(BeginRunError::Failed(error));
        }
    };
    let manifest = RunManifest {
        run: HistoryRun {
            id: id.clone(),
            source: bounded(source.trim(), 120, "unknown"),
            source_id: source_id
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| bounded(s, 500, "")),
            started_at,
            finished_at: None,
            status: "running".into(),
            changes: Vec::new(),
            flags: Vec::new(),
            scope: Some(scope.to_string()),
            overlaps: Vec::new(),
        },
        baseline: baseline.captured,
        uncaptured: baseline.uncaptured,
        unlisted: baseline.unlisted,
    };
    if let Err(error) = write_manifest(vault, &manifest) {
        forget_unadmitted(vault, &id);
        return Err(BeginRunError::Failed(error));
    }
    drop(objects);
    let baseline_ms = u64::try_from(baseline_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(HistoryRunHandle {
        id,
        baseline_ms: Some(baseline_ms),
    })
}

fn release_run_lease(vault: &Path, owner: &str) -> Result<(), String> {
    let result = release_lease_file(vault, owner);
    forget_held(vault, owner);
    result
}

fn release_lease_file(vault: &Path, owner: &str) -> Result<(), String> {
    let path = active_run_path(vault);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("stat automated history lease: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("automated history lease is not a regular file".into());
    }
    if metadata.len() > MAX_LEASE_BYTES {
        return Err("automated history lease is malformed".into());
    }
    let recorded = fs::read_to_string(&path)
        .map_err(|error| format!("read automated history lease: {error}"))?;
    if parse_lease(&recorded).owner != owner.trim() {
        return Ok(());
    }
    fs::remove_file(path).map_err(|error| format!("release automated history lease: {error}"))?;
    notify_lease_released(vault);
    Ok(())
}

/// Startup-only cleanup for a lease orphaned without a running manifest.
fn clear_stale_run_lease(vault: &Path) -> Result<(), String> {
    let path = active_run_path(vault);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Err("automated history lease is a directory".into())
        }
        Ok(_) => {
            fs::remove_file(path)
                .map_err(|error| format!("clear stale automated history lease: {error}"))?;
            drop(take_held_for_vault(vault));
            notify_lease_released(vault);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            drop(take_held_for_vault(vault));
            Ok(())
        }
        Err(error) => Err(format!("stat automated history lease: {error}")),
    }
}

#[derive(Debug, Clone)]
struct RestorePlanEntry {
    change: HistoryChange,
    target: PathBuf,
    before_bytes: Option<Vec<u8>>,
    current_bytes: Option<Vec<u8>>,
    current_hash: Option<String>,
}

fn begin_run_inner(
    vault: &Path,
    source: &str,
    source_id: Option<&str>,
) -> Result<HistoryRunHandle, BeginRunError> {
    ensure_history_dirs(vault).map_err(BeginRunError::Failed)?;
    // A lease whose owner exited is reclaimed first (its running manifest
    // finalized as interrupted); the active-manifest scan below stays as
    // defense in depth.
    let reclaimed = reclaim_locked(vault)?;
    if let Some(active) = read_all_manifests(vault)
        .map_err(BeginRunError::Failed)?
        .into_iter()
        // A scoped run holds only its folder, never the vault (PR B3).
        .find(|manifest| run_is_active(&manifest.run) && manifest.run.scope.is_none())
    {
        return Err(BeginRunError::Busy(LeaseHolder::Run {
            id: active.run.id,
            source: Some(active.run.source),
            source_id: active.run.source_id,
            started_at: Some(active.run.started_at),
        }));
    }
    let id = new_run_id();
    acquire_lease_file_with(vault, &id, reclaimed)?;
    note_vault_run_began(vault, &id);
    let started_at = Utc::now().to_rfc3339();
    // The baseline's cost, snapshot through manifest write (Paperclip PR 15c).
    let baseline_started = Instant::now();
    // The baseline's objects are named once the record is written (Q-R9).
    let objects = match objects_in_flight_for_begin(vault) {
        Ok(objects) => objects,
        Err(error) => {
            let _ = release_run_lease(vault, &id);
            forget_registry_run(vault, &id);
            return Err(error);
        }
    };
    let baseline = match snapshot_for_run(vault, None) {
        Ok(coverage) => coverage,
        Err(error) => {
            let _ = release_run_lease(vault, &id);
            forget_registry_run(vault, &id);
            return Err(BeginRunError::Failed(error));
        }
    };
    let manifest = RunManifest {
        run: HistoryRun {
            id: id.clone(),
            source: bounded(source.trim(), 120, "unknown"),
            source_id: source_id
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| bounded(s, 500, "")),
            started_at,
            finished_at: None,
            status: "running".into(),
            changes: Vec::new(),
            flags: Vec::new(),
            scope: None,
            overlaps: Vec::new(),
        },
        baseline: baseline.captured,
        uncaptured: baseline.uncaptured,
        unlisted: baseline.unlisted,
    };
    if let Err(error) = write_manifest(vault, &manifest) {
        let _ = release_run_lease(vault, &id);
        forget_registry_run(vault, &id);
        return Err(BeginRunError::Failed(error));
    }
    drop(objects);
    let baseline_ms = u64::try_from(baseline_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(HistoryRunHandle {
        id,
        baseline_ms: Some(baseline_ms),
    })
}

/// A finish that failed. For a tracked run `flagged` still carries the run's
/// flags (persisted in a Hollow-written record when possible), so the caller
/// notifies anyway: an agent must not be able to silence its flags by
/// breaking its own history record.
#[derive(Debug)]
struct FinishFailure {
    message: String,
    flagged: Option<Finalized>,
}

impl FinishFailure {
    fn plain(message: String) -> Self {
        Self {
            message,
            flagged: None,
        }
    }
}

impl std::fmt::Display for FinishFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<FinishFailure> for String {
    fn from(failure: FinishFailure) -> Self {
        failure.message
    }
}

fn finish_run_inner(
    vault: &Path,
    run_id: &str,
    status: &str,
    mode: FlagMode,
) -> Result<Finalized, FinishFailure> {
    finish_run_inner_with_retention(vault, run_id, status, true, mode)
}

/// Finalize `run_id`. `mode` says how its flags are computed:
/// [`FlagMode::Tracked`] (a live run: the in-memory entry when it exists,
/// otherwise the change set), [`FlagMode::Reconcile`] (startup: the change
/// set) or [`FlagMode::None`] (history restores: nothing).
///
/// A tracked run's flags are computed FIRST, from memory and the live disk
/// only, so nothing below that reads the agent-writable history can hide
/// them. If its manifest is missing, unreadable, names another run or is
/// already finalized (legitimately impossible while the in-memory entry
/// exists: the first finish takes it), or anything else fails before the
/// final manifest write, Hollow writes its own finalized record for the run
/// (no change list, a `historyManifest` flag naming what failed), releases
/// the lease and returns a [`FinishFailure`] that still carries the flags.
fn finish_run_inner_with_retention(
    vault: &Path,
    run_id: &str,
    status: &str,
    apply_retention: bool,
    mode: FlagMode,
) -> Result<Finalized, FinishFailure> {
    // Take the in-memory entry FIRST, so no error path below can leak it.
    let tracked = crate::run_guard::take_tracking(run_id);
    let Some(tracked) = tracked.filter(|_| mode != FlagMode::None) else {
        return finalize_manifest(vault, run_id, status, apply_retention, mode, None)
            .map_err(|error| FinishFailure::plain(error.into()));
    };
    let origin = (
        tracked.source().to_string(),
        tracked.source_id().map(str::to_string),
    );
    let started_at = tracked.started_at().to_string();
    let mut flags =
        crate::run_guard::compute_flags(vault, FlagMode::Tracked, Some(tracked), "", &[], &|_| {
            None
        });
    // A shared doc written over during the run (concurrency PR 2).
    flags.extend(crate::shared_docs::take_flags(run_id));
    let tracked_finish = TrackedFinish {
        flags,
        origin,
        started_at,
    };
    match finalize_manifest(
        vault,
        run_id,
        status,
        apply_retention,
        mode,
        Some(&tracked_finish),
    ) {
        Ok(finalized) => Ok(finalized),
        Err(FinalizeError::AfterWrite(message, finalized)) => Err(FinishFailure {
            message,
            flagged: Some(finalized),
        }),
        Err(FinalizeError::BeforeWrite(message)) => {
            let finalized = record_broken_run(
                vault,
                run_id,
                status,
                apply_retention,
                tracked_finish,
                &message,
            );
            Err(FinishFailure {
                message,
                flagged: Some(finalized),
            })
        }
    }
}

/// What a tracked run brings to [`finalize_manifest`]: its flags (already
/// computed) and the source it began with.
struct TrackedFinish {
    flags: Vec<crate::run_guard::RunFlag>,
    origin: (String, Option<String>),
    started_at: String,
}

/// Where [`finalize_manifest`] failed: before the final manifest write (the
/// record on disk can't be trusted or completed), or after it (the run is
/// finalized; releasing the lease or retention failed).
enum FinalizeError {
    BeforeWrite(String),
    AfterWrite(String, Finalized),
}

impl From<FinalizeError> for String {
    fn from(error: FinalizeError) -> Self {
        match error {
            FinalizeError::BeforeWrite(message) | FinalizeError::AfterWrite(message, _) => message,
        }
    }
}

/// A tracked run's history record could not be finalized normally (see
/// [`finish_run_inner_with_retention`]): write Hollow's own finalized record
/// from what it knows in memory, best effort, and release the lease.
fn record_broken_run(
    vault: &Path,
    run_id: &str,
    status: &str,
    apply_retention: bool,
    tracked: TrackedFinish,
    reason: &str,
) -> Finalized {
    let TrackedFinish {
        mut flags,
        origin,
        started_at,
    } = tracked;
    let manifest_rel = format!("{HISTORY_DIR}/{RUNS_DIR}/{run_id}.json");
    crate::run_guard::ensure_manifest_flag(&mut flags, &manifest_rel, "modify", reason);
    let scope = scope_for(vault, run_id, None);
    // Its own changes are lost, so every finished peer's files in the folder
    // may be its work too (a live peer reads this record at its finish).
    let overlaps: Vec<String> = match &scope {
        Some(scope) => {
            let peers = registry_peers(vault, run_id).unwrap_or_default();
            for peer_id in &peers {
                let Ok(mut peer) = read_manifest(vault, peer_id) else {
                    continue;
                };
                if peer.run.finished_at.is_none() || peer.run.scope.as_deref() != Some(scope) {
                    continue;
                }
                let mut touched = add_once(&mut peer.run.overlaps, run_id);
                for change in peer
                    .run
                    .changes
                    .iter_mut()
                    .filter(|c| under_scope(&c.vault_path, scope))
                {
                    touched |= add_once(&mut change.also_changed_by, run_id);
                }
                if touched {
                    if let Err(error) = write_manifest(vault, &peer) {
                        eprintln!("[history] could not mark run {peer_id}'s files shared with run {run_id}: {error}");
                    }
                }
            }
            peers.into_iter().collect()
        }
        None => Vec::new(),
    };
    let run = HistoryRun {
        id: run_id.to_string(),
        source: bounded(origin.0.trim(), 120, "unknown"),
        source_id: origin.1.as_deref().map(|id| bounded(id, 500, "")),
        started_at,
        finished_at: Some(Utc::now().to_rfc3339()),
        status: bounded(status.trim(), 80, "unknown"),
        changes: Vec::new(),
        flags,
        scope: scope.clone(),
        overlaps,
    };
    let manifest = RunManifest {
        run: run.clone(),
        baseline: Vec::new(),
        uncaptured: Vec::new(),
        unlisted: Vec::new(),
    };
    if let Err(error) = write_manifest(vault, &manifest) {
        eprintln!("[history] could not rewrite the history record of run {run_id}: {error}");
    }
    if let Err(error) = release_for(vault, run_id, scope.as_deref()) {
        eprintln!("[history] could not release the lease of run {run_id}: {error}");
    }
    if apply_retention {
        if let Err(error) = enforce_retention(vault, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES) {
            eprintln!("[history] retention after run {run_id} failed: {error}");
        }
    }
    Finalized {
        run,
        newly: true,
        origin: Some(origin),
    }
}

/// Diff the manifest's baseline against the vault and write the finalized
/// manifest. `tracked` carries a tracked run's precomputed flags; otherwise
/// flags come from the change set per `mode`.
fn finalize_manifest(
    vault: &Path,
    run_id: &str,
    status: &str,
    apply_retention: bool,
    mode: FlagMode,
    tracked: Option<&TrackedFinish>,
) -> Result<Finalized, FinalizeError> {
    let before_write = FinalizeError::BeforeWrite;
    let mut manifest = read_manifest(vault, run_id).map_err(before_write)?;
    // A live run's folder comes from the registry; only startup recovery
    // reads the manifest's own (agent-writable) record.
    let scope = scope_for(vault, run_id, manifest.run.scope.as_deref());
    if manifest.run.finished_at.is_some() {
        if tracked.is_some() {
            return Err(FinalizeError::BeforeWrite(
                "the history record was already marked finished while the run was still live"
                    .into(),
            ));
        }
        release_for(vault, run_id, scope.as_deref()).map_err(before_write)?;
        return Ok(Finalized {
            run: manifest.run,
            newly: false,
            origin: None,
        });
    }

    // The after-versions are named once the finalized record is written
    // (Q-R9). Dropped before retention, which cleans up objects itself.
    // Refused: a tracked run gets Hollow's own record with no change list
    // (`record_broken_run`), as for a failed snapshot.
    let objects = objects_in_flight(vault).map_err(before_write)?;
    let after = snapshot_for_run(vault, scope.as_deref()).map_err(before_write)?;
    let baseline_gaps = Gaps {
        uncaptured: &manifest.uncaptured,
        unlisted: &manifest.unlisted,
    };
    let mut changes = diff_snapshots(
        vault,
        &manifest.baseline,
        &after.captured,
        baseline_gaps,
        Gaps::of(&after),
    )
    .map_err(before_write)?;

    // A scoped run records only its folder (its baseline is agent-writable,
    // so an entry planted outside the folder is not a change of this run). A
    // full-vault run leaves out every folder a scoped run wrote meanwhile.
    //
    // What Hollow app and task writes changed outside a scoped run's folder
    // on its behalf joins it: from the registry for a live run (Hollow's own
    // record), or at startup recovery from the running manifest's copy,
    // which only [`record_broker_changes`] writes there.
    let excluded = match &scope {
        Some(scope) => {
            let broker = match broker_changes_for(vault, run_id) {
                Some(broker) => broker,
                None if tracked.is_none() => manifest.run.changes.clone(),
                None => Vec::new(),
            };
            // Hollow's own writes in the folder for someone else (a peer's
            // session record, F3) are not this run's change, unless the
            // run's own writes named the file too.
            let host = host_writes_for(vault, run_id);
            let own = own_writes_for(vault, run_id);
            changes.retain(|change: &HistoryChange| {
                let hollows = host
                    .get(&change.vault_path)
                    .is_some_and(|write| write.explains(change));
                under_scope(&change.vault_path, scope)
                    && (!hollows || owns(vault, &own, &change.vault_path))
            });
            changes.extend(
                broker
                    .into_iter()
                    .filter(|change| !under_scope(&change.vault_path, scope)),
            );
            changes.sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
            Vec::new()
        }
        None => {
            let excluded = scopes_to_exclude(vault, run_id);
            changes.retain(|change: &HistoryChange| {
                !excluded
                    .iter()
                    .any(|scope| under_scope(&change.vault_path, scope))
            });
            excluded
        }
    };

    // A tracked run's flags were computed up front from its in-memory baseline
    // and the live disk (the manifest was hashed then, before this final write
    // replaced it). An untracked finish derives them from the change set.
    let (flags, origin) = match tracked {
        Some(t) => (t.flags.clone(), Some(t.origin.clone())),
        None => {
            let effective = match mode {
                FlagMode::None => FlagMode::None,
                _ => FlagMode::Reconcile,
            };
            let flags = crate::run_guard::compute_flags(
                vault,
                effective,
                None,
                &manifest.run.source,
                &changes,
                &|hash: &str| read_object(vault, hash).ok(),
            );
            (flags, None)
        }
    };
    if let Some((source, source_id)) = &origin {
        // The manifest's own copy is agent-writable; record the source the
        // run began with.
        manifest.run.source = bounded(source.trim(), 120, "unknown");
        manifest.run.source_id = source_id.as_deref().map(|id| bounded(id, 500, ""));
    }

    let mut flags = flags;
    flags.retain(|flag| !excluded.iter().any(|scope| under_scope(&flag.path, scope)));
    let finished_at = Utc::now();
    // Parallel chats in one workspace (2026-10-07): what this run and a
    // finished peer in its folder both changed is marked on both records.
    let (overlaps, peer_records) = match &scope {
        Some(scope) => {
            let peers = registry_peers(vault, run_id).unwrap_or_else(|| {
                interval_peers(vault, run_id, scope, &manifest.run.started_at, finished_at)
            });
            // Chat concurrency F1 (2026-10-20): a run that ever shared its
            // folder is credited only with what its own writes named.
            let shared = !peers.is_empty();
            let marked = mark_overlaps(vault, run_id, scope, &mut changes, peers, &after);
            if shared {
                mark_unattributed(vault, run_id, scope, &mut changes);
            }
            mark_host_made(vault, run_id, &mut changes);
            marked
        }
        None => (Vec::new(), Vec::new()),
    };
    mark_hollow_writes(vault, run_id, &mut changes);
    // Q-R11: closed by recovery after its process died, with no record of
    // when it last wrote, so no entry can be vouched for as the run's own.
    if tracked.is_none() && mode == FlagMode::Reconcile && status.trim() == "interrupted" {
        for change in &mut changes {
            change.unverified = true;
        }
    }
    manifest.run.finished_at = Some(finished_at.to_rfc3339());
    manifest.run.status = bounded(status.trim(), 80, "unknown");
    manifest.run.changes = changes;
    manifest.run.flags = flags;
    manifest.run.scope = scope.clone();
    manifest.run.overlaps = overlaps;
    manifest.baseline.clear();
    manifest.uncaptured.clear();
    manifest.unlisted.clear();
    write_manifest(vault, &manifest).map_err(before_write)?;
    // Best effort: this run is finalized either way, and its own record
    // already names the files it shares.
    for peer in peer_records {
        if let Err(error) = write_manifest(vault, &peer) {
            eprintln!(
                "[history] could not mark run {}'s files shared with run {run_id}: {error}",
                peer.run.id
            );
        }
    }
    drop(objects);
    let finalized = Finalized {
        run: manifest.run,
        newly: true,
        origin,
    };
    if let Err(error) = release_for(vault, run_id, scope.as_deref()) {
        return Err(FinalizeError::AfterWrite(error, finalized));
    }
    if apply_retention {
        if let Err(error) = enforce_retention(vault, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES) {
            return Err(FinalizeError::AfterWrite(error, finalized));
        }
    }
    Ok(finalized)
}

/// One side's gaps for a diff: what its snapshot saw but holds no copy of,
/// and the folders it couldn't list ([`snapshot_coverage`]).
#[derive(Clone, Copy)]
struct Gaps<'a> {
    uncaptured: &'a [UncapturedEntry],
    unlisted: &'a [String],
}

impl<'a> Gaps<'a> {
    fn of(coverage: &'a SnapshotCoverage) -> Self {
        Self {
            uncaptured: &coverage.uncaptured,
            unlisted: &coverage.unlisted,
        }
    }

    fn by_path(&self) -> BTreeMap<&'a str, &'a UncapturedEntry> {
        self.uncaptured
            .iter()
            .map(|e| (e.vault_path.as_str(), e))
            .collect()
    }

    /// This side's coverage of `path`, given its capture and gap entries.
    fn coverage(
        &self,
        path: &str,
        captured: Option<&SnapshotEntry>,
        gap: Option<&UncapturedEntry>,
    ) -> Coverage {
        match (captured, gap) {
            (Some(_), _) => Coverage::Captured,
            (None, Some(gap)) => Coverage::Uncaptured { reason: gap.reason },
            (None, None) if self.unlisted_above(path) => Coverage::Unknown,
            (None, None) => Coverage::Absent,
        }
    }

    fn unlisted_above(&self, path: &str) -> bool {
        self.unlisted
            .iter()
            .any(|dir| dir.is_empty() || dir == path || under_scope(path, dir))
    }
}

/// What changed between two snapshots, sorted by path, with each side's
/// [`Coverage`]. The type follows what existed: a file present on both sides
/// is a `modify` even when one side is uncaptured, never a `create` or
/// `delete` that restore would undo by trashing or overwriting it (review
/// B·F01). A file uncaptured on both sides is a change when its size or
/// modification time moved; a side under a folder its snapshot couldn't
/// list is unknown.
fn diff_snapshots(
    vault: &Path,
    before: &[SnapshotEntry],
    after: &[SnapshotEntry],
    before_gaps: Gaps<'_>,
    after_gaps: Gaps<'_>,
) -> Result<Vec<HistoryChange>, String> {
    let before_by_path: BTreeMap<&str, &SnapshotEntry> =
        before.iter().map(|s| (s.vault_path.as_str(), s)).collect();
    let after_by_path: BTreeMap<&str, &SnapshotEntry> =
        after.iter().map(|s| (s.vault_path.as_str(), s)).collect();
    let before_gap = before_gaps.by_path();
    let after_gap = after_gaps.by_path();
    let paths: BTreeSet<&str> = before_by_path
        .keys()
        .chain(after_by_path.keys())
        .chain(before_gap.keys())
        .chain(after_gap.keys())
        .copied()
        .collect();

    let mut changes = Vec::new();
    for path in paths {
        let before = before_by_path.get(path).copied();
        let after = after_by_path.get(path).copied();
        let before_uncaptured = before_gap.get(path).copied();
        let after_uncaptured = after_gap.get(path).copied();
        if same_version(before, after, before_uncaptured, after_uncaptured) {
            continue;
        }
        let before_coverage = before_gaps.coverage(path, before, before_uncaptured);
        let after_coverage = after_gaps.coverage(path, after, after_uncaptured);
        let text = before.map(|s| s.text).unwrap_or(true) && after.map(|s| s.text).unwrap_or(true);
        let before_notion = notion_mirror_metadata(vault, before)?;
        let after_notion = notion_mirror_metadata(vault, after)?;
        changes.push(HistoryChange {
            vault_path: path.to_string(),
            change_type: change_type_for(before_coverage, after_coverage).into(),
            before_hash: before.map(|s| s.hash.clone()),
            after_hash: after.map(|s| s.hash.clone()),
            before_size: before.map(|s| s.size),
            after_size: after.map(|s| s.size),
            text,
            notion_mirror: before_notion.is_mirror || after_notion.is_mirror,
            before_notion_id: before_notion.notion_id,
            after_notion_id: after_notion.notion_id,
            before_uncaptured: !before_coverage.restorable(),
            after_uncaptured: !after_coverage.restorable(),
            before_coverage: Some(before_coverage),
            after_coverage: Some(after_coverage),
            restored_at: None,
            restored_by_run_id: None,
            also_changed_by: Vec::new(),
            unattributed: false,
            unverified: false,
            hollow_write: false,
        });
    }
    Ok(changes)
}

/// True when a path's two sides (its capture and gap entries in each
/// snapshot) are provably the same version: the same hash, or uncaptured
/// on both sides with the same size and modification time.
fn same_version(
    before: Option<&SnapshotEntry>,
    after: Option<&SnapshotEntry>,
    before_uncaptured: Option<&UncapturedEntry>,
    after_uncaptured: Option<&UncapturedEntry>,
) -> bool {
    match (before, after, before_uncaptured, after_uncaptured) {
        (Some(b), Some(a), _, _) => b.hash == a.hash,
        (None, None, Some(b), Some(a)) => b.stamp() == a.stamp(),
        _ => false,
    }
}

/// The paths under `scope` that a LIVE peer's diff would hold if it
/// finished now: its running manifest's baseline against `after`, this
/// finish's snapshot of the same folder (chat concurrency F1, 2026-10-20).
/// The baseline is agent-writable, so this only ever adds marks.
fn live_peer_paths(peer: &RunManifest, after: &SnapshotCoverage, scope: &str) -> BTreeSet<String> {
    let before: BTreeMap<&str, &SnapshotEntry> = peer
        .baseline
        .iter()
        .map(|e| (e.vault_path.as_str(), e))
        .collect();
    let now: BTreeMap<&str, &SnapshotEntry> = after
        .captured
        .iter()
        .map(|e| (e.vault_path.as_str(), e))
        .collect();
    let before_gap = Gaps {
        uncaptured: &peer.uncaptured,
        unlisted: &peer.unlisted,
    }
    .by_path();
    let now_gap = Gaps::of(after).by_path();
    before
        .keys()
        .chain(now.keys())
        .chain(before_gap.keys())
        .chain(now_gap.keys())
        .filter(|path| under_scope(path, scope))
        .filter(|path| {
            !same_version(
                before.get(*path).copied(),
                now.get(*path).copied(),
                before_gap.get(*path).copied(),
                now_gap.get(*path).copied(),
            )
        })
        .map(|path| path.to_string())
        .collect()
}

/// The runs whose record says they were live in `scope` while run `run_id`
/// was (from `started_at` to `finished_at`): startup recovery's stand-in for
/// the registry's peer set. Records are agent-writable, so this only ever
/// adds peers, which makes a restore more cautious, never less.
fn interval_peers(
    vault: &Path,
    run_id: &str,
    scope: &str,
    started_at: &str,
    finished_at: DateTime<Utc>,
) -> BTreeSet<String> {
    let parse = |stamp: &str| {
        DateTime::parse_from_rfc3339(stamp)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    };
    let Some(began) = parse(started_at) else {
        return BTreeSet::new();
    };
    let manifests = match read_all_manifests(vault) {
        Ok(manifests) => manifests,
        Err(error) => {
            eprintln!("[history] could not look for runs beside run {run_id}: {error}");
            return BTreeSet::new();
        }
    };
    manifests
        .into_iter()
        .filter(|m| m.run.id != run_id && m.run.scope.as_deref() == Some(scope))
        .filter(|m| {
            let peer_began = parse(&m.run.started_at);
            let peer_ended = m.run.finished_at.as_deref().and_then(parse);
            peer_began.is_none_or(|at| at <= finished_at) && peer_ended.is_none_or(|at| at >= began)
        })
        .map(|m| m.run.id)
        .collect()
}

/// Mark what run `run_id` (its `changes`) and each FINISHED peer in `scope`
/// both changed under the folder, on both sides. A peer whose record is
/// gone, unreadable or broken ([`peer_paths`]) may have changed anything, so
/// every file of this run in the folder is marked with it. A peer still
/// live (chat concurrency F1, 2026-10-20) marks this run's side now, from
/// what its own diff would hold ([`live_peer_paths`] against `after`, this
/// finish's snapshot), so the receipt written at this finish already says
/// so; the peer marks its side, and this record again, at its own finish,
/// since this run is in its set. Returns the peer list for the run's record
/// and the peer records to write back. Under `HISTORY_LOCK`.
fn mark_overlaps(
    vault: &Path,
    run_id: &str,
    scope: &str,
    changes: &mut [HistoryChange],
    peers: BTreeSet<String>,
    after: &SnapshotCoverage,
) -> (Vec<String>, Vec<RunManifest>) {
    let mut records = Vec::new();
    for peer_id in &peers {
        let record = read_manifest(vault, peer_id).ok();
        if let Some(live) = record.as_ref().filter(|m| m.run.finished_at.is_none()) {
            let paths = live_peer_paths(live, after, scope);
            for change in changes
                .iter_mut()
                .filter(|c| under_scope(&c.vault_path, scope) && paths.contains(&c.vault_path))
            {
                add_once(&mut change.also_changed_by, peer_id);
            }
            continue;
        }
        let paths = peer_paths(record.as_ref().map(|m| &m.run), scope);
        for change in changes
            .iter_mut()
            .filter(|c| under_scope(&c.vault_path, scope))
            .filter(|c| paths.as_ref().is_none_or(|p| p.contains(&c.vault_path)))
        {
            add_once(&mut change.also_changed_by, peer_id);
        }
        let (Some(mut peer), Some(_)) = (record, paths) else {
            continue;
        };
        let mut touched = add_once(&mut peer.run.overlaps, run_id);
        for theirs in peer.run.changes.iter_mut().filter(|theirs| {
            changes
                .iter()
                .any(|c| c.vault_path == theirs.vault_path && under_scope(&c.vault_path, scope))
        }) {
            touched |= add_once(&mut theirs.also_changed_by, run_id);
        }
        if touched {
            records.push(peer);
        }
    }
    (peers.into_iter().collect(), records)
}

/// What a peer's record says it changed under `scope`, or `None` when that
/// can't be trusted: there is no record (pruned, unreadable), it is still
/// running, it is for another folder, or it was broken or tampered with (a
/// `historyManifest` flag). `None` means it may have changed any file.
fn peer_paths(peer: Option<&HistoryRun>, scope: &str) -> Option<BTreeSet<String>> {
    let peer = peer?;
    if peer.finished_at.is_none()
        || peer.scope.as_deref() != Some(scope)
        || peer
            .flags
            .iter()
            .any(|flag| flag.kind == crate::run_guard::FLAG_HISTORY_MANIFEST)
    {
        return None;
    }
    Some(
        peer.changes
            .iter()
            .map(|c| c.vault_path.clone())
            .filter(|path| under_scope(path, scope))
            .collect(),
    )
}

/// `run`'s changes with each overlap marked again from its peers' records
/// as they are now (`peer` looks one up), on top of the stored marks: a
/// failed write-back, or a peer that is still running, was pruned or broke,
/// is never missed. Restores and History's listings use this, so the stored
/// marks are never the only guard.
fn recheck_overlaps(
    run: &HistoryRun,
    peer: &dyn Fn(&str) -> Option<HistoryRun>,
) -> Vec<HistoryChange> {
    let mut changes = run.changes.clone();
    let Some(scope) = run.scope.as_deref() else {
        return changes;
    };
    for peer_id in &run.overlaps {
        let paths = peer_paths(peer(peer_id).as_ref(), scope);
        for change in changes
            .iter_mut()
            .filter(|c| under_scope(&c.vault_path, scope))
            .filter(|c| paths.as_ref().is_none_or(|p| p.contains(&c.vault_path)))
        {
            add_once(&mut change.also_changed_by, peer_id);
        }
    }
    changes
}

/// [`recheck_overlaps`] over runs read from the same listing.
fn recheck_all(runs: &mut [HistoryRun], all: &HashMap<String, HistoryRun>) {
    for run in runs.iter_mut().filter(|run| !run.overlaps.is_empty()) {
        run.changes = recheck_overlaps(run, &|id| all.get(id).cloned());
    }
}

/// Push `id` unless it is already listed (kept sorted); true when it was added.
fn add_once(ids: &mut Vec<String>, id: &str) -> bool {
    match ids.binary_search_by(|have| have.as_str().cmp(id)) {
        Ok(_) => false,
        Err(at) => {
            ids.insert(at, id.to_string());
            true
        }
    }
}

fn restore_change_inner(
    vault: &Path,
    aliases: &crate::workspace_aliases::Aliases,
    run_id: &str,
    vault_path: &str,
    force: bool,
) -> Result<HistoryRun, String> {
    let mut original = read_manifest(vault, run_id)?;
    if original.run.finished_at.is_none() {
        return Err("cannot restore from an unfinished history run".into());
    }
    // A moved workspace's file is restored where it is now.
    original.run = current_paths(vault, aliases, original.run);
    let change = recheck_current(vault, aliases, &original.run)
        .into_iter()
        .find(|c| aliases.same_vault_path(&c.vault_path, vault_path))
        .ok_or_else(|| format!("run {run_id} has no change for {vault_path}"))?;

    // Use the same complete preflight and apply-time CAS recheck as whole-run
    // restore. In particular, creating the recovery baseline no longer leaves
    // a window in which a later edit can be overwritten by this single-file
    // restore without detection.
    let mut plan = build_restore_plan(vault, std::slice::from_ref(&change), force)?;
    let entry = plan
        .pop()
        .ok_or_else(|| format!("run {run_id} has no change for {vault_path}"))?;

    // Untracked on purpose (begin_run_inner, FlagMode::None): a restore is the
    // user reverting a change, never an escalation to flag.
    let restore_handle =
        begin_run_inner(vault, "historyRestore", Some(run_id)).map_err(String::from)?;
    if let Err(error) = apply_restore_entry(vault, &entry) {
        let _ = finish_run_inner(vault, &restore_handle.id, "error", FlagMode::None);
        return Err(error);
    }

    let restore_run = finish_run_inner(vault, &restore_handle.id, "success", FlagMode::None)?.run;

    // Best-effort annotation of the original change. The restore itself is
    // already complete and durably represented by `restore_run`, so an audit
    // annotation failure must not falsely report that the content restore
    // failed.
    if let Ok(mut latest) = read_manifest(vault, run_id) {
        if let Some(c) = latest
            .run
            .changes
            .iter_mut()
            .find(|c| aliases.same_vault_path(&c.vault_path, vault_path))
        {
            c.restored_at = Some(Utc::now().to_rfc3339());
            c.restored_by_run_id = Some(restore_run.id.clone());
            let _ = write_manifest(vault, &latest);
        }
    }

    Ok(restore_run)
}

fn restore_run_inner(
    vault: &Path,
    aliases: &crate::workspace_aliases::Aliases,
    run_id: &str,
    force: bool,
) -> Result<HistoryRun, String> {
    let mut original = read_manifest(vault, run_id)?;
    if original.run.finished_at.is_none() || original.run.status == "running" {
        return Err("cannot restore from an unfinished history run".into());
    }
    // A moved workspace's files are restored where they are now.
    original.run = current_paths(vault, aliases, original.run);

    // This performs every structural, path, object, and CAS check before a
    // recovery manifest is created or any live vault path is changed.
    let plan = build_restore_plan(
        vault,
        &recheck_current(vault, aliases, &original.run),
        force,
    )?;
    let restore_run = apply_restore_plan(vault, run_id, &plan)?;

    // Every original change receives the same audit stamp. The history lock
    // prevents another Hollow restore from interleaving this manifest update.
    let restored_at = Utc::now().to_rfc3339();
    let mut latest = read_manifest(vault, run_id).map_err(|error| {
        format!(
            "content restored in {}, but original run annotation failed: {error}",
            restore_run.id
        )
    })?;
    for change in &mut latest.run.changes {
        change.restored_at = Some(restored_at.clone());
        change.restored_by_run_id = Some(restore_run.id.clone());
    }
    write_manifest(vault, &latest).map_err(|error| {
        format!(
            "content restored in {}, but original run annotation failed: {error}",
            restore_run.id
        )
    })?;
    enforce_retention(vault, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES)?;

    Ok(restore_run)
}

/// Apply a preflighted restore plan as one `historyRestore` run (rolled back
/// and finalized as `error` on a failure part-way), so the restore itself
/// can be undone. Retention runs after the caller's own annotation.
fn apply_restore_plan(
    vault: &Path,
    run_id: &str,
    plan: &[RestorePlanEntry],
) -> Result<HistoryRun, String> {
    // One baseline covers the entire multi-file operation. If application
    // fails, the same manifest is finalized as error after best-effort rollback.
    let restore_handle =
        begin_run_inner(vault, "historyRestore", Some(run_id)).map_err(String::from)?;
    let mut applied = Vec::new();
    for (index, entry) in plan.iter().enumerate() {
        match apply_restore_entry(vault, entry) {
            Ok(true) => applied.push(index),
            Ok(false) => {}
            Err(error) => {
                let rollback_errors = rollback_restore_entries(vault, plan, &applied);
                let finalize_error =
                    finish_run_inner(vault, &restore_handle.id, "error", FlagMode::None).err();
                let mut detail = format!(
                    "whole-run restore failed at {}: {error}",
                    entry.change.vault_path
                );
                if !rollback_errors.is_empty() {
                    detail.push_str(&format!(
                        "; rollback warnings: {}",
                        rollback_errors.join(" | ")
                    ));
                }
                if let Some(finalize_error) = finalize_error {
                    detail.push_str(&format!(
                        "; could not finalize restore audit run: {finalize_error}"
                    ));
                }
                return Err(detail);
            }
        }
    }

    // Keep the source manifest present until its changes have been annotated.
    // The ordinary retention pass follows immediately afterwards.
    Ok(finish_run_inner_with_retention(
        vault,
        &restore_handle.id,
        "success",
        false,
        FlagMode::None,
    )?
    .run)
}

/// The most step objects one run pins, and their most bytes: one heavy turn
/// must not push other runs' records out under the storage ceiling.
const MAX_STEP_OBJECTS: usize = 2_000;
const MAX_STEP_PIN_BYTES: u64 = 64 * 1024 * 1024;
const STEP_PINS_DIR: &str = "step-pins";

fn step_pins_path(vault: &Path, run_id: &str) -> Result<PathBuf, String> {
    validate_run_id(run_id)?;
    Ok(history_root(vault)
        .join(STEP_PINS_DIR)
        .join(format!("{run_id}.txt")))
}

fn read_step_pins(vault: &Path, run_id: &str) -> Vec<String> {
    step_pins_path(vault, run_id)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|h| validate_hash(h).is_ok())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// [`read_step_pins`] that fails instead of skipping: an unreadable,
/// non-UTF-8 or non-regular pin file, or a line that isn't a hash. No file
/// is no pins.
fn read_step_pins_checked(vault: &Path, run_id: &str) -> Result<Vec<String>, String> {
    let path = step_pins_path(vault, run_id)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("step pins for {run_id}: {e}")),
        Ok(meta) if !meta.is_file() => {
            return Err(format!("step pins for {run_id} are not a regular file"))
        }
        Ok(_) => {}
    }
    let bytes = fs::read(&path).map_err(|e| format!("step pins for {run_id}: {e}"))?;
    let text =
        String::from_utf8(bytes).map_err(|_| format!("step pins for {run_id} are not text"))?;
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            validate_hash(line)
                .map(|()| line.to_string())
                .map_err(|_| format!("step pins for {run_id} name something that isn't a version"))
        })
        .collect()
}

/// Objects the step spine pinned for the retained runs: referenced like
/// their changes, so they live exactly as long as the run's record.
fn pinned_hashes(vault: &Path, manifests: &[RunManifest]) -> HashSet<String> {
    manifests
        .iter()
        .flat_map(|m| read_step_pins(vault, &m.run.id))
        .collect()
}

/// Remove the pins of every run whose record is gone.
fn sweep_step_pins(vault: &Path, kept: &HashSet<String>) {
    let Ok(entries) = fs::read_dir(history_root(vault).join(STEP_PINS_DIR)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let keep = name
            .strip_suffix(".txt")
            .is_some_and(|id| kept.contains(id));
        if !keep && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Store `bytes` in History's object store for live run `run_id` (the step
/// spine's per-step file versions, master plan F6) and return their
/// SHA-256. A small sidecar (`.hollow-history/step-pins/<run>.txt`) pins
/// the object, so retention keeps it exactly as long as the run's record;
/// a run that isn't live stores nothing.
pub(crate) fn store_step_object(
    vault: &Path,
    run_id: &str,
    bytes: &[u8],
) -> Result<String, String> {
    if !run_is_live(vault, run_id) {
        return Err(format!("run {run_id} is not live"));
    }
    let hash = hash_bytes(bytes);
    let _guard = lock_history();
    // The object is named once the pin file lists it (Q-R9).
    let _objects = objects_in_flight(vault)?;
    let mut pins = read_step_pins(vault, run_id);
    if !pins.contains(&hash) {
        if pins.len() >= MAX_STEP_OBJECTS {
            return Err(format!(
                "run {run_id} pins {MAX_STEP_OBJECTS} step versions already"
            ));
        }
        let pinned: u64 = pins
            .iter()
            .filter_map(|h| object_path(vault, h).ok())
            .filter_map(|p| fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        if pinned + bytes.len() as u64 > MAX_STEP_PIN_BYTES {
            return Err(format!(
                "run {run_id} pins {} MiB of step versions already",
                MAX_STEP_PIN_BYTES / 1024 / 1024
            ));
        }
    }
    write_object(vault, &hash, bytes)?;
    if !pins.contains(&hash) {
        pins.push(hash.clone());
        let path = step_pins_path(vault, run_id)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("step pins: {e}"))?;
        }
        crate::vault::atomic_write(&path, format!("{}\n", pins.join("\n")).as_bytes())?;
    }
    Ok(hash)
}

/// One file's wanted version for [`restore_to_versions`]: `None` = absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WantedVersion {
    pub vault_path: String,
    pub hash: Option<String>,
    pub size: Option<u64>,
}

/// Put files a finished run changed back to chosen versions (the step
/// spine's "retry from step n", master plan F6): History objects, or absent.
/// Every file must still be exactly as the run left it (its recorded
/// after-version), every wanted object must verify, and a file another
/// conversation also changed refuses, all before anything is touched: a
/// refusal changes nothing. A path the run's record doesn't list refuses
/// too (History can't vouch for what is there). Applied as one
/// `historyRestore` run, so it can be undone. Never forced.
pub(crate) fn restore_to_versions(
    vault: &Path,
    run_id: &str,
    wanted: &[WantedVersion],
) -> Result<Option<HistoryRun>, String> {
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    let result = {
        let _guard = lock_history();
        restore_to_versions_inner(vault, &aliases, run_id, wanted)
    };
    drain_reclaim_notices(vault);
    result
}

fn restore_to_versions_inner(
    vault: &Path,
    aliases: &crate::workspace_aliases::Aliases,
    run_id: &str,
    wanted: &[WantedVersion],
) -> Result<Option<HistoryRun>, String> {
    let mut original = read_manifest(vault, run_id)?;
    if original.run.finished_at.is_none() || original.run.status == "running" {
        return Err("cannot restore from an unfinished history run".into());
    }
    original.run = current_paths(vault, aliases, original.run);
    let recorded = recheck_current(vault, aliases, &original.run);
    let mut changes = Vec::new();
    for want in wanted {
        validate_vault_path(&want.vault_path)?;
        // An old-id path asked for means the file where it is now.
        let mapped;
        let want = match current_path(vault, aliases, &want.vault_path) {
            Some(now) => {
                mapped = WantedVersion {
                    vault_path: now,
                    ..want.clone()
                };
                &mapped
            }
            None => want,
        };
        let Some(change) = recorded
            .iter()
            .find(|c| aliases.same_vault_path(&c.vault_path, &want.vault_path))
        else {
            return Err(format!(
                "history restore conflict: run {run_id} has no record of {}, so History can't vouch for it",
                want.vault_path
            ));
        };
        if !change.coverage_after().restorable() {
            return Err(coverage_error(
                &want.vault_path,
                "after",
                change.coverage_after(),
            ));
        }
        if want.hash == change.after_hash {
            // Nothing to put back, provided it is still as the run left it.
            let target = safe_restore_target(vault, &want.vault_path)?;
            if file_sha256(&target)? != change.after_hash {
                return Err(format!(
                    "history restore conflict: {} no longer matches the recorded after-version",
                    want.vault_path
                ));
            }
            continue;
        }
        let change_type = match (&want.hash, &change.after_hash) {
            (None, Some(_)) => "create",
            (Some(_), None) => "delete",
            (Some(_), Some(_)) => "modify",
            (None, None) => continue,
        };
        changes.push(HistoryChange {
            vault_path: want.vault_path.clone(),
            change_type: change_type.into(),
            before_hash: want.hash.clone(),
            after_hash: change.after_hash.clone(),
            before_size: want.size,
            after_size: change.after_size,
            text: change.text,
            notion_mirror: change.notion_mirror,
            before_notion_id: None,
            after_notion_id: None,
            before_uncaptured: false,
            after_uncaptured: false,
            before_coverage: Some(if want.hash.is_some() {
                Coverage::Captured
            } else {
                Coverage::Absent
            }),
            after_coverage: change.after_coverage,
            restored_at: None,
            restored_by_run_id: None,
            also_changed_by: change.also_changed_by.clone(),
            unattributed: change.unattributed,
            // Carried so the refusal holds for a retry from a step too.
            unverified: change.unverified,
            hollow_write: false,
        });
    }
    if changes.is_empty() {
        return Ok(None);
    }
    let plan = build_restore_plan(vault, &changes, false)?;
    let run = apply_restore_plan(vault, run_id, &plan)?;
    enforce_retention(vault, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES)?;
    Ok(Some(run))
}

/// Why a restore must not write `changes`' files now (F5, 2026-10-20): a
/// file inside a folder where a conversation is running (a workspace's own
/// folder, shared, or a build workspace's, exclusive), or one Hollow changed
/// on such a conversation's behalf outside it ([`with_live_run`]). Its agent
/// or a later Hollow write may still change the file, and its finish would
/// claim the restore's write as its own. Everything else restores while
/// conversations run elsewhere; a run holding the vault lease still refuses
/// at `begin_run_inner`. Called under `HISTORY_LOCK`, which every scoped
/// admission and broker write takes too, so none can start between this
/// check and the restore's write. Names the first conversation's folder
/// and up to three of its files.
fn live_writer_refusal(vault: &Path, changes: &[HistoryChange]) -> Option<String> {
    let all = scopes();
    let state = all.get(&canonical_vault(vault))?;
    if state.live.is_empty() {
        return None;
    }
    // Every path as it is spelled on disk, case-folded (review of PR 861):
    // `workspaces/WB/x`, `./workspaces//wb/x` and `workspaces/wb/x` are one
    // file on a case-insensitive Mac volume.
    let root = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    let scopes: Vec<(String, &String)> = state
        .live
        .values()
        .map(|scope| (disk_key(&root, scope), scope))
        .collect();
    let brokered: Vec<(String, &String)> = state
        .broker_changes
        .iter()
        .filter_map(|(id, made)| state.live.get(id).map(|scope| (made, scope)))
        .flat_map(|(made, scope)| made.iter().map(move |c| (c, scope)))
        .map(|(change, scope)| (disk_key(&root, &change.vault_path), scope))
        .collect();
    // (folder, whether through Hollow) → the files, for the first file hit.
    let mut hit: Option<((String, bool), Vec<&str>)> = None;
    for change in changes {
        let path = change.vault_path.as_str();
        let key = disk_key(&root, path);
        let inside = scopes
            .iter()
            .filter(|(folder, _)| under_scope(&key, folder))
            .map(|(_, scope)| *scope)
            .min()
            .map(|scope| (scope.clone(), false));
        let brokered = || {
            brokered
                .iter()
                .filter(|(made, _)| *made == key)
                .map(|(_, scope)| *scope)
                .min()
                .map(|scope| (scope.clone(), true))
        };
        let Some(key) = inside.or_else(brokered) else {
            continue;
        };
        match &mut hit {
            None => hit = Some((key, vec![path])),
            Some((first, paths)) if *first == key => paths.push(path),
            Some(_) => {}
        }
    }
    let ((scope, brokered), paths) = hit?;
    let mut listed = paths.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    if paths.len() > 3 {
        listed.push_str(&format!(" and {} more", paths.len() - 3));
    }
    Some(if brokered {
        format!(
            "A conversation is running in {scope} and changed {listed} through Hollow; try again when it finishes"
        )
    } else {
        format!(
            "A conversation is running in {scope}, where this restore would change {listed}; try again when it finishes"
        )
    })
}

/// The components of vault path `rel` (`.` and repeated separators
/// dropped).
fn path_parts(rel: &str) -> Vec<String> {
    Path::new(rel)
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Vault path `rel` as it is spelled on disk: its deepest existing ancestor
/// (the file itself when it exists) resolved by the filesystem, which on a
/// case- and normalization-insensitive Mac volume returns the stored
/// spelling, then the rest as given. Falls back to the plain components when
/// that ancestor resolves outside the vault.
fn disk_spelling(root: &Path, rel: &str) -> String {
    let parts = path_parts(rel);
    let mut resolved = Vec::new();
    let mut rest = 0;
    for depth in (1..=parts.len()).rev() {
        let probe = root.join(parts[..depth].join("/"));
        if fs::symlink_metadata(&probe).is_err() {
            continue;
        }
        if let Some(inner) = probe
            .canonicalize()
            .ok()
            .and_then(|canon| canon.strip_prefix(root).ok().map(path_parts_of))
        {
            resolved = inner;
            rest = depth;
        }
        break;
    }
    resolved.extend(parts[rest..].iter().cloned());
    resolved.join("/")
}

fn path_parts_of(path: &Path) -> Vec<String> {
    path_parts(&path.to_string_lossy())
}

/// [`disk_spelling`], case-folded, for comparing two spellings of one file
/// (a case-only variant of a path that doesn't exist yet folds too; on a
/// case-sensitive volume this refuses a little more, never less).
fn disk_key(root: &Path, rel: &str) -> String {
    disk_spelling(root, rel).to_lowercase()
}

/// The start of a restore's refusal for files another conversation also
/// changed; the History panel matches it to offer a forced restore.
pub(crate) const SHARED_RESTORE_CONFLICT: &str = "history restore conflict (shared)";

/// Q-R11: the refusal for files of a run closed after its process died.
/// "conflict" so the History panel offers a forced restore with this text.
fn unverified_refusal(changes: &[HistoryChange]) -> Option<String> {
    let unknown: Vec<&str> = changes
        .iter()
        .filter(|c| c.unverified)
        .map(|c| c.vault_path.as_str())
        .collect();
    (!unknown.is_empty()).then(|| {
        format!(
            "history restore conflict: changes unknown: the run ended with Hollow's process, so {} may include edits you made while Hollow was down, and restoring could undo them",
            unknown.join(", ")
        )
    })
}

fn build_restore_plan(
    vault: &Path,
    source_changes: &[HistoryChange],
    force: bool,
) -> Result<Vec<RestorePlanEntry>, String> {
    let mut changes = source_changes.to_vec();
    changes.sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
    validate_restore_change_paths(&changes)?;
    // Forcing never overrides this: the other conversation is still writing.
    if let Some(refusal) = live_writer_refusal(vault, &changes) {
        return Err(refusal);
    }

    // Nothing is planned while any change has a side History holds no copy
    // of: restoring it would trash or overwrite a file with no copy to return
    // to. The refusal lists every such file (the restore preview).
    let blocked = restore_preview(&changes);
    if !blocked.is_empty() {
        return Err(unrestorable_error(&blocked));
    }
    // A file another conversation in the same workspace also changed while
    // this run was live (2026-10-07): History can't tell whose edit it was,
    // so restoring it could undo that conversation's work. Forcing is the
    // person's explicit choice.
    let shared: Vec<&str> = changes
        .iter()
        .filter(|c| !c.also_changed_by.is_empty())
        .map(|c| c.vault_path.as_str())
        .collect();
    if !force && !shared.is_empty() {
        return Err(format!(
            "{SHARED_RESTORE_CONFLICT}: {} {} also changed by another conversation running at the same time in this workspace, so restoring could undo its work too",
            shared.join(", "),
            if shared.len() == 1 { "was" } else { "were" }
        ));
    }
    if !force {
        if let Some(refusal) = unverified_refusal(&changes) {
            return Err(refusal);
        }
    }
    let mut plan = Vec::with_capacity(changes.len());
    let mut needs_trash = false;
    for change in changes {
        validate_restore_change_shape(&change)?;
        let target = safe_restore_target(vault, &change.vault_path)?;
        let (current_bytes, current_hash) =
            load_current_restore_state(&target, &change.vault_path)?;

        if !force && current_hash.as_deref() != change.after_hash.as_deref() {
            return Err(format!(
                "history restore conflict: {} no longer matches the recorded after-version",
                change.vault_path
            ));
        }
        if current_hash.as_deref() == change.after_hash.as_deref() {
            if let (Some(bytes), Some(recorded_size)) = (current_bytes.as_ref(), change.after_size)
            {
                if bytes.len() as u64 != recorded_size {
                    return Err(format!(
                        "history manifest after-size mismatch for {}",
                        change.vault_path
                    ));
                }
            }
        }

        // Load and hash-verify every before object during preflight. No target
        // mutation starts until the complete plan has passed this loop.
        let before_bytes = match (change.before_hash.as_deref(), change.before_size) {
            (Some(hash), Some(size)) => Some(read_restore_object(vault, hash, size)?),
            (None, None) => None,
            _ => {
                return Err(format!(
                    "history manifest before-version is incomplete for {}",
                    change.vault_path
                ))
            }
        };
        if before_bytes.is_none() && current_bytes.is_some() {
            needs_trash = true;
        }

        plan.push(RestorePlanEntry {
            change,
            target,
            before_bytes,
            current_bytes,
            current_hash,
        });
    }

    if needs_trash {
        preflight_trash_dir(vault)?;
    }
    Ok(plan)
}

fn validate_restore_change_shape(change: &HistoryChange) -> Result<(), String> {
    for hash in [change.before_hash.as_deref(), change.after_hash.as_deref()]
        .into_iter()
        .flatten()
    {
        validate_hash(hash)?;
    }
    for size in [change.before_size, change.after_size]
        .into_iter()
        .flatten()
    {
        if size > MAX_ELIGIBLE_FILE_BYTES {
            return Err(format!(
                "history version exceeds restore size limit for {}",
                change.vault_path
            ));
        }
    }

    let valid = match change.change_type.as_str() {
        "create" => {
            change.before_hash.is_none()
                && change.before_size.is_none()
                && change.after_hash.is_some()
                && change.after_size.is_some()
        }
        "modify" => {
            change.before_hash.is_some()
                && change.before_size.is_some()
                && change.after_hash.is_some()
                && change.after_size.is_some()
        }
        "delete" => {
            change.before_hash.is_some()
                && change.before_size.is_some()
                && change.after_hash.is_none()
                && change.after_size.is_none()
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "malformed history change {} for {}",
            change.change_type, change.vault_path
        ))
    }
}

fn validate_restore_change_paths(changes: &[HistoryChange]) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for change in changes {
        validate_vault_path(&change.vault_path)?;
        let path = PathBuf::from(&change.vault_path);
        if !paths.insert(path) {
            return Err(format!(
                "history run contains duplicate path: {}",
                change.vault_path
            ));
        }
    }

    for path in &paths {
        for ancestor in path.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            if paths.contains(ancestor) {
                return Err(format!(
                    "history run contains overlapping paths: {} and {}",
                    ancestor.display(),
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

fn load_current_restore_state(
    target: &Path,
    vault_path: &str,
) -> Result<(Option<Vec<u8>>, Option<String>), String> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
        Err(error) => return Err(format!("stat restore target {vault_path}: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "restore target is not a regular file: {vault_path}"
        ));
    }
    if metadata.len() > MAX_ELIGIBLE_FILE_BYTES {
        return Err(format!(
            "current restore target exceeds size limit: {vault_path}"
        ));
    }
    let bytes = fs::read(target).map_err(|error| format!("read restore target: {error}"))?;
    if bytes.len() as u64 > MAX_ELIGIBLE_FILE_BYTES {
        return Err(format!(
            "current restore target exceeds size limit: {vault_path}"
        ));
    }
    let hash = hash_bytes(&bytes);
    Ok((Some(bytes), Some(hash)))
}

fn read_restore_object(vault: &Path, hash: &str, expected_size: u64) -> Result<Vec<u8>, String> {
    validate_existing_history_layout(vault, OBJECTS_DIR)?;
    let path = object_path(vault, hash)?;
    let metadata =
        fs::symlink_metadata(&path).map_err(|error| format!("stat history object: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("history object {hash} is not a regular file"));
    }
    if metadata.len() != expected_size || metadata.len() > MAX_ELIGIBLE_FILE_BYTES {
        return Err(format!("history object {hash} has an unexpected size"));
    }
    let bytes = read_object(vault, hash)?;
    if bytes.len() as u64 != expected_size {
        return Err(format!("history object {hash} has an unexpected size"));
    }
    Ok(bytes)
}

fn preflight_trash_dir(vault: &Path) -> Result<(), String> {
    let trash = vault.join(".trash");
    match fs::symlink_metadata(&trash) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err("vault trash path is a symlink".into())
        }
        Ok(metadata) if !metadata.is_dir() => Err("vault trash path is not a directory".into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("stat vault trash: {error}")),
    }
}

fn apply_restore_entry(vault: &Path, entry: &RestorePlanEntry) -> Result<bool, String> {
    #[cfg(test)]
    if should_inject_restore_failure(vault, &entry.change.vault_path) {
        return Err("injected restore application failure".into());
    }

    #[cfg(test)]
    inject_restore_apply_mutation(vault, &entry.change.vault_path, &entry.target)?;

    let observed = current_hash(&entry.target)?;
    if observed != entry.current_hash {
        return Err(format!(
            "{} changed after restore preflight",
            entry.change.vault_path
        ));
    }

    // The file as run_guard knows it, asked before a delete removes it.
    let rel = restore_rel(vault, &entry.change.vault_path);
    let existed_key = observed
        .is_some()
        .then(|| crate::run_guard::protected_key(vault, &rel));
    match entry.before_bytes.as_deref() {
        Some(bytes) => {
            if observed.as_deref() == entry.change.before_hash.as_deref() {
                return Ok(false);
            }
            atomic_restore_write(vault, &entry.change.vault_path, &entry.target, bytes)?;
        }
        None => {
            if observed.is_none() {
                return Ok(false);
            }
            preflight_trash_dir(vault)?;
            crate::vault::delete_note(vault, &entry.change.vault_path)?;
        }
    }
    // The person's restore is Hollow's write, not a live conversation's
    // agent (review of PR 861): a protected file's new bytes are recorded,
    // so a live run's protected-file check doesn't flag them.
    let key = existed_key.unwrap_or_else(|| crate::run_guard::protected_key(vault, &rel));
    crate::run_guard::note_host_vault_write(
        vault,
        &key,
        entry.current_bytes.as_deref(),
        entry.before_bytes.as_deref(),
    );
    Ok(true)
}

/// Vault path `rel` as run_guard names protected files: the on-disk
/// spelling, under the canonical vault.
fn restore_rel(vault: &Path, rel: &str) -> String {
    let root = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    disk_spelling(&root, rel)
}

#[cfg(test)]
fn inject_restore_apply_mutation(
    vault: &Path,
    vault_path: &str,
    target: &Path,
) -> Result<(), String> {
    let root = vault.canonicalize().ok();
    let mutation = {
        let mut guard = RESTORE_APPLY_MUTATION
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let matches = guard
            .as_ref()
            .is_some_and(|(expected_root, expected_path, _)| {
                root.as_ref() == Some(expected_root) && expected_path == vault_path
            });
        matches.then(|| guard.take()).flatten()
    };
    if let Some((_, _, bytes)) = mutation {
        fs::write(target, bytes).map_err(|error| format!("inject restore race: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
fn should_inject_restore_failure(vault: &Path, vault_path: &str) -> bool {
    let root = vault.canonicalize().ok();
    RESTORE_APPLY_FAILURE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|(expected_root, expected_path)| {
            root.as_ref() == Some(expected_root) && expected_path == vault_path
        })
        .unwrap_or(false)
}

fn rollback_restore_entries(
    vault: &Path,
    plan: &[RestorePlanEntry],
    applied: &[usize],
) -> Vec<String> {
    let mut errors = Vec::new();
    for index in applied.iter().rev().copied() {
        let entry = &plan[index];
        let rel = restore_rel(vault, &entry.change.vault_path);
        let existed_key = entry
            .target
            .exists()
            .then(|| crate::run_guard::protected_key(vault, &rel));
        let expected_applied_hash = entry.change.before_hash.as_deref();
        match current_hash(&entry.target) {
            Ok(observed) if observed.as_deref() != expected_applied_hash => {
                errors.push(format!(
                    "{} changed again; rollback skipped",
                    entry.change.vault_path
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!("{}: {error}", entry.change.vault_path));
                continue;
            }
            Ok(_) => {}
        }

        let rollback = match entry.current_bytes.as_deref() {
            Some(bytes) => {
                atomic_restore_write(vault, &entry.change.vault_path, &entry.target, bytes)
            }
            None => {
                if entry.target.exists() {
                    preflight_trash_dir(vault)
                        .and_then(|_| crate::vault::delete_note(vault, &entry.change.vault_path))
                } else {
                    Ok(())
                }
            }
        };
        match rollback {
            Ok(()) => crate::run_guard::note_host_vault_write(
                vault,
                &existed_key.unwrap_or_else(|| crate::run_guard::protected_key(vault, &rel)),
                entry.before_bytes.as_deref(),
                entry.current_bytes.as_deref(),
            ),
            Err(error) => errors.push(format!("{}: {error}", entry.change.vault_path)),
        }
    }
    errors
}

fn atomic_restore_write(
    vault: &Path,
    vault_path: &str,
    target: &Path,
    bytes: &[u8],
) -> Result<(), String> {
    // Re-resolve immediately before writing so a symlink or non-directory
    // ancestor introduced after preflight remains a hard failure.
    let resolved = safe_restore_target(vault, vault_path)?;
    if resolved != target {
        return Err(format!(
            "restore target changed after preflight: {vault_path}"
        ));
    }
    crate::vault::atomic_write(target, bytes)?;
    refresh_restored_file(vault, vault_path, target, bytes);
    Ok(())
}

fn refresh_restored_file(vault: &Path, vault_path: &str, target: &Path, bytes: &[u8]) {
    crate::vault::invalidate_walk_cache();
    crate::vault::invalidate_index();
    if let Ok(content) = std::str::from_utf8(bytes) {
        crate::vault::refresh_index_for_file(vault_path, content);
        let metadata = target.metadata().ok();
        let mtime = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .map(|modified| DateTime::<Utc>::from(modified).to_rfc3339())
            .unwrap_or_default();
        let size = metadata
            .as_ref()
            .map(|metadata| metadata.len())
            .unwrap_or(bytes.len() as u64);
        crate::vault_search_index::on_write(vault, vault_path, content, &mtime);
        crate::vault_embed_index::on_write(vault, vault_path, content, &mtime, size);
    } else {
        crate::vault_search_index::on_remove(vault, vault_path);
        crate::vault_embed_index::on_remove(vault, vault_path);
    }
}

fn notion_mirror_metadata(
    vault: &Path,
    snapshot: Option<&SnapshotEntry>,
) -> Result<NotionMirrorMetadata, String> {
    let Some(snapshot) = snapshot else {
        return Ok(NotionMirrorMetadata::default());
    };
    if !snapshot.text {
        return Ok(NotionMirrorMetadata::default());
    }
    let bytes = read_object(vault, &snapshot.hash)?;
    let content = std::str::from_utf8(&bytes)
        .map_err(|_| format!("history text object {} is not UTF-8", snapshot.hash))?;
    let (frontmatter, _) = crate::frontmatter::parse_map(content);
    let notion_id = frontmatter
        .get("notion_id")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let notion_source = frontmatter
        .get("source")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("notion"));
    Ok(NotionMirrorMetadata {
        is_mirror: notion_source || notion_id.is_some(),
        notion_id,
    })
}

/// Why one side of a change can't be read or restored.
fn coverage_error(vault_path: &str, side: &str, coverage: Coverage) -> String {
    let why = match coverage {
        Coverage::Uncaptured {
            reason: Some(UncapturedReason::TooLarge),
        } => "it was over 2 MiB",
        Coverage::Uncaptured {
            reason: Some(UncapturedReason::Unreadable),
        } => "it couldn't be read",
        Coverage::Uncaptured {
            reason: Some(UncapturedReason::Excluded),
        } => "it isn't a regular file History stores",
        Coverage::Uncaptured { reason: None } => "it was over 2 MiB or unreadable",
        Coverage::Unknown => {
            return format!(
                "History can't tell whether the {side} version of {vault_path} existed (a folder above it couldn't be read), so it can't show or restore it, and won't touch the file"
            );
        }
        Coverage::Absent | Coverage::Captured => {
            return format!("History holds the {side} version of {vault_path}");
        }
    };
    format!(
        "History didn't capture the {side} version of {vault_path} ({why}), so it can't show or restore it, and won't touch the file"
    )
}

/// The restore refusal for every file in a restore preview's blocked list.
fn unrestorable_error(blocked: &[Unrestorable]) -> String {
    let reasons: Vec<String> = blocked
        .iter()
        .map(|file| coverage_error(&file.vault_path, file.side, file.coverage))
        .collect();
    match reasons.as_slice() {
        [only] => only.clone(),
        _ => format!(
            "History can't restore {} files in this run, so it restored nothing: {}",
            reasons.len(),
            reasons.join("; ")
        ),
    }
}

#[cfg(test)]
fn snapshot(vault: &Path) -> Result<Vec<SnapshotEntry>, String> {
    snapshot_under(vault, None)
}

/// [`snapshot`] of one scoped run's folder (PR B3). A folder that is gone
/// (the workspace was deleted) snapshots as empty.
#[cfg(test)]
fn snapshot_scoped(vault: &Path, scope: &str) -> Result<Vec<SnapshotEntry>, String> {
    validate_vault_path(scope)?;
    snapshot_under(vault, Some(scope))
}

#[cfg(test)]
fn snapshot_under(vault: &Path, scope: Option<&str>) -> Result<Vec<SnapshotEntry>, String> {
    snapshot_coverage(vault, scope).map(|coverage| coverage.captured)
}

/// The captured files under `scope` (or the whole vault), the visible files
/// it holds no copy of (over 2 MiB, unreadable, or not a regular file), and
/// the folders it couldn't list. A diff needs all three ([`diff_snapshots`]):
/// a file missing from one side's capture was not necessarily missing from
/// the vault (merged plan slices 1 and 4, review B·F01).
/// The before and after snapshot of a run. A scoped run still walks its
/// folder. A full-vault run, after the first snapshot in this process,
/// reapplies only the paths the file-system events named since that
/// snapshot ([`incremental`]). Tests keep the full walk, so a missed
/// event cannot hide a change from the suite. A failed incremental
/// attempt walks the vault, which is also the first snapshot.
fn snapshot_for_run(vault: &Path, scope: Option<&str>) -> Result<SnapshotCoverage, String> {
    if scope.is_some() {
        return snapshot_coverage(vault, scope);
    }
    // Tests keep the full walk. A `cfg!(test)` branch left in this function
    // is unreachable under `cargo test`, and the watcher would then look
    // unused to clippy `--tests`.
    #[cfg(test)]
    {
        snapshot_coverage(vault, None)
    }
    #[cfg(not(test))]
    {
        if incremental::armed(vault) {
            if let Some(coverage) = incremental::try_since_tip(vault) {
                return Ok(coverage);
            }
        } else {
            incremental::arm(vault);
        }
        let coverage = snapshot_coverage(vault, None)?;
        // Failure deletes a stale tip. This snapshot's coverage still stands,
        // and the next one walks.
        let _ = incremental::save_tip(vault, &coverage);
        Ok(coverage)
    }
}

fn snapshot_coverage(vault: &Path, scope: Option<&str>) -> Result<SnapshotCoverage, String> {
    if let Some(scope) = scope {
        validate_vault_path(scope)?;
    }
    validate_vault_root(vault)?;
    ensure_history_dirs(vault)?;
    let root = match scope {
        Some(scope) => {
            let root = vault.join(scope);
            match fs::symlink_metadata(&root) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => root,
                // A scope folder must never be followed through a link.
                Ok(_) => return Err(format!("run folder {scope} is not a plain folder")),
                Err(_) => return Ok(SnapshotCoverage::default()),
            }
        }
        None => vault.to_path_buf(),
    };
    let root_rel = root
        .strip_prefix(vault)
        .unwrap_or(Path::new(""))
        .to_path_buf();
    let scope_rel = scope.map(|_| root_rel.as_path());
    walk_coverage(vault, std::slice::from_ref(&root), scope_rel, false)
}

/// [`snapshot_coverage`] of just `roots`, vault-relative folders or files,
/// for a scoped run's broker write (chat concurrency F7, 2026-10-20); only
/// for paths [`broker_roots`] cleared. A root that doesn't exist is left
/// out (absent, not unknown, so a file the write creates there reads as
/// created), and a link is a gap, never followed (one below a root sets
/// [`SnapshotCoverage::links`], and the write falls back to the whole
/// vault). It neither reads nor writes the stat cache: loading it costs as
/// much as the vault has entries (about 65 ms in a debug build at 20,000
/// files, against 11 ms to read 220 task files), and a file the write just
/// changed is too new to vouch for anyway.
fn snapshot_roots(vault: &Path, roots: &[String]) -> Result<SnapshotCoverage, String> {
    validate_vault_root(vault)?;
    ensure_history_dirs(vault)?;
    let mut walk = Vec::new();
    for rel in roots {
        validate_vault_path(rel)?;
        let path = vault.join(rel);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            // Anything else the walk reports (unlisted when it can't look).
            _ => walk.push(path),
        }
    }
    walk_coverage(vault, &walk, None, true)
}

/// The walk behind [`snapshot_coverage`] (`roots` is the vault or one
/// scope folder, `scope_rel` that folder) and [`snapshot_roots`]
/// (`broker`: several roots, a root link is not followed, no stat cache).
fn walk_coverage(
    vault: &Path,
    roots: &[PathBuf],
    scope_rel: Option<&Path>,
    broker: bool,
) -> Result<SnapshotCoverage, String> {
    // The stat cache (OB09). Taken before the walk, so every file written
    // after the snapshot started is inside the racy window.
    let settled_before = racy_threshold_ns();
    // Only a file on the vault root's own device, and only when that file
    // system keeps a real ctime and inode: elsewhere (FAT, exFAT, network
    // shares, FUSE, a mount inside the vault) every file is read, as
    // before OB09, and the cache is neither read nor written.
    let vouch_dev = vouching_device(vault);
    // A broker walk (F7) reads its few files instead: loading the cache
    // costs as much as the whole vault has entries.
    let (mut cache, loaded_at) = if vouch_dev.is_some() && !broker {
        load_stat_cache_versioned(vault)
    } else {
        (HashMap::new(), 0)
    };
    // SE5's tests hold a walk here, after its cache load, to show two
    // admissions walk at once.
    #[cfg(test)]
    if !broker {
        admission_tests::walk_gate(vault);
    }
    // What this walk vouches for, all of it under `scope_rel`.
    let mut fresh: HashMap<String, CachedFile> = HashMap::new();
    let mut out = SnapshotCoverage::default();
    let walks = roots.iter().flat_map(|root| {
        WalkDir::new(root)
            .follow_links(false)
            .follow_root_links(!broker)
            .into_iter()
            .filter_entry(move |entry| !hidden_entry(entry, root))
            .map(move |entry| (root, entry))
    });
    for (root, entry) in walks {
        #[cfg(test)]
        SNAPSHOT_VISITS.with(|visits| visits.set(visits.get() + 1));
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                // Whatever is below a folder the walk couldn't list is
                // unknown, not absent. An error with no path covers the root.
                let rel = error
                    .path()
                    .unwrap_or(root.as_path())
                    .strip_prefix(vault)
                    .map(|rel| rel.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                out.unlisted.push(rel);
                continue;
            }
        };
        // A broker root may be a file: only a folder is skipped there.
        if (!broker && entry.path() == root) || entry.file_type().is_dir() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(vault) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        // A name History can't store safely is left out entirely.
        if validate_vault_path(&rel).is_err() {
            continue;
        }
        let meta = entry.metadata().ok();
        if !entry.file_type().is_file() {
            out.links |= broker && entry.file_type().is_symlink();
            let gap = UncapturedEntry::new(rel, UncapturedReason::Excluded, meta.as_ref());
            out.uncaptured.push(gap);
            continue;
        }
        // Stat taken before the read: a store needs it to match the stat
        // taken after (OB09).
        let stat = meta
            .as_ref()
            .and_then(file_stat)
            .filter(|stat| Some(stat.4) == vouch_dev);
        if let Some(stat) = stat {
            // An unchanged, settled file whose object is unchanged too:
            // reuse the hash and skip the read and `write_object`. A missing
            // or touched object (retention, damage) falls through and is
            // verified or written again.
            if let Some(cached) = cache.get(&rel) {
                if cached.0 == stat
                    && stat.settled(settled_before)
                    && object_stat(vault, &cached.1) == Some(cached.3)
                {
                    out.captured.push(SnapshotEntry {
                        vault_path: rel.clone(),
                        hash: cached.1.clone(),
                        size: stat.0,
                        text: cached.2,
                    });
                    fresh.insert(rel, cached.clone());
                    continue;
                }
            }
        }
        let bytes = match &meta {
            None => Err(UncapturedReason::Unreadable),
            Some(meta) if meta.len() > MAX_ELIGIBLE_FILE_BYTES => Err(UncapturedReason::TooLarge),
            Some(_) => match count_content_read(fs::read(entry.path())) {
                Err(_) => Err(UncapturedReason::Unreadable),
                Ok(bytes) if bytes.len() as u64 > MAX_ELIGIBLE_FILE_BYTES => {
                    Err(UncapturedReason::TooLarge)
                }
                Ok(bytes) => Ok(bytes),
            },
        };
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(reason) => {
                out.uncaptured
                    .push(UncapturedEntry::new(rel, reason, meta.as_ref()));
                continue;
            }
        };
        let hash = hash_bytes(&bytes);
        write_object(vault, &hash, &bytes)?;
        let text = std::str::from_utf8(&bytes).is_ok();
        // Vouch only for a settled file whose stat didn't move across the
        // read: then these bytes are the ones that stat describes (OB09).
        // With it goes the stat of the object `write_object` just verified
        // or wrote, settled too, so a later in-place rewrite of the object
        // (even to the same size) moves it and the object is checked again.
        let size = bytes.len() as u64;
        if let Some(before) = stat {
            if before.settled(settled_before) && before.0 == size {
                let after = fs::symlink_metadata(entry.path())
                    .ok()
                    .as_ref()
                    .and_then(file_stat);
                let object = object_stat(vault, &hash);
                let object = object.filter(|obj| obj.settled(settled_before) && obj.0 == size);
                if let Some(object) = object.filter(|_| after == Some(before)) {
                    let file = CachedFile(before, hash.clone(), text, object);
                    fresh.insert(rel.clone(), file);
                }
            }
        }
        out.captured.push(SnapshotEntry {
            vault_path: rel,
            hash,
            size,
            text,
        });
    }
    // A full snapshot's cache is what it saw; a scoped one replaces only the
    // entries under its folder and keeps the rest. Either way a path it saw
    // but couldn't capture, or didn't see, loses its entry (OB09).
    let in_scope = |path: &str| match scope_rel {
        Some(scope) => Path::new(path).starts_with(scope),
        None => true,
    };
    // Unchanged (same paths, same entries): no rewrite. A broker walk
    // never writes it (F7).
    let replaced = || cache.keys().filter(|path| in_scope(path)).count();
    let kept = |(path, file): (&String, &CachedFile)| cache.get(path) == Some(file);
    let changed = !broker && (replaced() != fresh.len() || !fresh.iter().all(kept));
    if changed {
        // Scoped admissions walk at once (SE5), so another one may have
        // saved since this walk loaded: merge into what is on disk now, not
        // into this walk's older copy, or its folder's entries are lost.
        let _io = stat_cache_io();
        if scope_rel.is_some() {
            if stat_cache_version(vault) != loaded_at {
                cache = load_stat_cache(vault);
            }
            cache.retain(|path, _| !in_scope(path));
            cache.extend(fresh);
        } else {
            cache = fresh;
        }
        save_stat_cache(vault, cache);
        bump_stat_cache_version(vault);
    }
    out.captured.sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
    out.uncaptured
        .sort_by(|a, b| a.vault_path.cmp(&b.vault_path));
    out.unlisted.sort();
    out.unlisted.dedup();
    Ok(out)
}

// ── Snapshot stat cache (OB09) ─────────────────────────────────────────────
// A 3,096-file workspace spent ~1.5 s per snapshot reading and hashing files
// that hadn't changed. The cache maps a path to the stat it had when it was
// last read and the hash of those bytes. Every entry is checked against a
// live stat before use, so a stale, lost or replayed entry costs a read,
// never a missed edit; a wrong one would hide an edit, hence the guards.

/// What must be unchanged for a cached hash to stand: size, mtime and ctime
/// (nanoseconds since the epoch), inode and device. ctime can't be set from
/// user space, so a rewrite that restores the old size and mtime still
/// moves it; the inode catches a file replaced by rename.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileStat(u64, i64, i64, u64, u64);

impl FileStat {
    /// Outside the racy window: neither timestamp is within
    /// [`RACY_WINDOW_NS`] of the snapshot start (`threshold` is that start
    /// minus the window). Stricter than git, which checks mtime only.
    fn settled(&self, threshold: i64) -> bool {
        self.1 < threshold && self.2 < threshold
    }
}

/// One cache entry: the stat, the SHA-256 of the bytes read under it,
/// whether they were UTF-8 ([`SnapshotEntry::text`]), and the stat of that
/// hash's object once it was verified or written. Serialized as an array to
/// keep the file small.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CachedFile(FileStat, String, bool, FileStat);

#[derive(Serialize, Deserialize)]
struct StatCache {
    version: u32,
    files: HashMap<String, CachedFile>,
}

#[cfg(unix)]
fn file_stat(meta: &fs::Metadata) -> Option<FileStat> {
    use std::os::unix::fs::MetadataExt;
    Some(FileStat(
        meta.size(),
        nanos(meta.mtime(), meta.mtime_nsec())?,
        nanos(meta.ctime(), meta.ctime_nsec())?,
        meta.ino(),
        meta.dev(),
    ))
}

/// Seconds and nanoseconds as one count (`None` past the year 2262).
#[cfg(unix)]
fn nanos(secs: i64, nsec: i64) -> Option<i64> {
    secs.checked_mul(1_000_000_000)?.checked_add(nsec)
}

/// No ctime or inode to check here, so no file is ever vouched for and
/// every snapshot reads every file, as before OB09.
#[cfg(not(unix))]
fn file_stat(_meta: &fs::Metadata) -> Option<FileStat> {
    None
}

/// The snapshot start minus [`RACY_WINDOW_NS`], in nanoseconds since the
/// epoch. Tests move the clock ahead to make a just-written file settled.
fn racy_threshold_ns() -> i64 {
    let now = SystemTime::now();
    #[cfg(test)]
    let now = now + SNAPSHOT_CLOCK_AHEAD.with(|ahead| ahead.get());
    let now = now
        .duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    now.saturating_sub(RACY_WINDOW_NS)
}

/// The stat of object `hash` when it is a regular file: one `lstat` in
/// place of [`write_object`]'s read and re-hash. A cached entry stands only
/// while this equals the stat taken when the object was last verified or
/// written, so an object truncated, replaced or rewritten in place (even to
/// the same size) is read and repaired from the file as before OB09.
fn object_stat(vault: &Path, hash: &str) -> Option<FileStat> {
    let path = object_path(vault, hash).ok()?;
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    file_stat(&meta)
}

/// The vault root's device when its file system keeps a ctime and inode
/// History can trust ([`keeps_real_ctime`]); `None` vouches for nothing.
/// One `stat` and one `statfs` per snapshot.
fn vouching_device(vault: &Path) -> Option<u64> {
    let real = file_system_name(vault).is_some_and(|name| keeps_real_ctime(&name));
    // Tests force either answer (`REAL_CTIME_OVERRIDE`).
    #[cfg(test)]
    let real = REAL_CTIME_OVERRIDE.with(|f| f.get()).unwrap_or(real);
    if !real {
        return None;
    }
    let meta = fs::metadata(vault).ok()?;
    file_stat(&meta).map(|stat| stat.4)
}

/// The file systems whose ctime and inode History trusts (OB09): APFS and
/// HFS+ on macOS ([`file_system_name`] gives `f_fstypename`), ext2/3/4, XFS
/// and Btrfs on Linux. FAT/exFAT have no ctime; SMB, NFS, AFP, WebDAV and
/// FUSE report whatever the server or driver makes up; anything unknown is
/// treated the same way, and every file there is read.
fn keeps_real_ctime(file_system: &str) -> bool {
    matches!(file_system, "apfs" | "hfs" | "ext4" | "xfs" | "btrfs")
}

/// The vault's file system type, as `statfs` names it (`None` when it
/// can't be read).
#[cfg(target_os = "macos")]
fn file_system_name(path: &Path) -> Option<String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stat` is a valid buffer that
    // `statfs` fills in full when it returns 0.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `statfs` returned 0, so `stat` is initialised.
    let stat = unsafe { stat.assume_init() };
    let name = stat.f_fstypename.iter().take_while(|&&byte| byte != 0);
    Some(name.map(|&byte| byte as u8 as char).collect())
}

/// Linux names no file system type, so the `statfs` magic number is mapped
/// to [`keeps_real_ctime`]'s names; ext2, ext3 and ext4 share one.
#[cfg(target_os = "linux")]
fn file_system_name(path: &Path) -> Option<String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stat` is a valid buffer that
    // `statfs` fills in full when it returns 0.
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `statfs` returned 0, so `stat` is initialised.
    let stat = unsafe { stat.assume_init() };
    linux_file_system_name(stat.f_type as u32).map(str::to_string)
}

#[cfg(target_os = "linux")]
fn linux_file_system_name(magic: u32) -> Option<&'static str> {
    match magic {
        0xEF53 => Some("ext4"),
        0x5846_5342 => Some("xfs"),
        0x9123_683E => Some("btrfs"),
        _ => None,
    }
}

/// Elsewhere no file system is known to keep a real ctime.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn file_system_name(_path: &Path) -> Option<String> {
    None
}

fn stat_cache_path(vault: &Path) -> PathBuf {
    history_root(vault).join(STAT_CACHE_FILE)
}

/// Serializes this process's stat cache loads and saves (SE5): scoped
/// admissions walk at once, under `HISTORY_LOCK` shared. A leaf lock.
static STAT_CACHE_IO: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
/// Per canonical vault, how many times this process saved the stat cache: a
/// walk whose load is still current saves its own copy without reading the
/// file again. Read and bumped only under `STAT_CACHE_IO` (nested inside
/// it, a leaf).
static STAT_CACHE_SAVES: Lazy<Mutex<HashMap<PathBuf, u64>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn stat_cache_io() -> std::sync::MutexGuard<'static, ()> {
    STAT_CACHE_IO.lock().unwrap_or_else(|e| e.into_inner())
}

fn stat_cache_version(vault: &Path) -> u64 {
    let key = canonical_vault(vault);
    let saves = STAT_CACHE_SAVES.lock().unwrap_or_else(|e| e.into_inner());
    saves.get(&key).copied().unwrap_or(0)
}

fn bump_stat_cache_version(vault: &Path) {
    let key = canonical_vault(vault);
    let mut saves = STAT_CACHE_SAVES.lock().unwrap_or_else(|e| e.into_inner());
    *saves.entry(key).or_default() += 1;
}

/// The objects this process's stat cache names, for object cleanup to keep.
/// First drops every entry whose folder is gone (a deleted workspace), so
/// its objects go with it instead of staying named until a walk that never
/// comes; the pruned cache is saved.
fn stat_cache_hashes(vault: &Path) -> HashSet<String> {
    let _io = stat_cache_io();
    let mut cache = load_stat_cache(vault);
    let before = cache.len();
    let mut folders: HashMap<PathBuf, bool> = HashMap::new();
    cache.retain(|path, _| {
        let folder = Path::new(path).parent().unwrap_or(Path::new(""));
        *folders.entry(folder.to_path_buf()).or_insert_with(|| {
            fs::symlink_metadata(vault.join(folder))
                .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        })
    });
    let hashes = cache.values().map(|file| file.1.clone()).collect();
    if cache.len() != before {
        save_stat_cache(vault, cache);
        bump_stat_cache_version(vault);
    }
    hashes
}

/// [`load_stat_cache`] and the save count it reflects, read together.
fn load_stat_cache_versioned(vault: &Path) -> (HashMap<String, CachedFile>, u64) {
    let _io = stat_cache_io();
    (load_stat_cache(vault), stat_cache_version(vault))
}

/// The cache, or an empty one when it is missing, too big, not a regular
/// file, unsealed (another process, a forgery, a torn write), unparseable
/// or another version. Never an error: the snapshot then reads everything.
fn load_stat_cache(vault: &Path) -> HashMap<String, CachedFile> {
    read_stat_cache(&stat_cache_path(vault))
        .and_then(|bytes| open_stat_cache(&bytes))
        .unwrap_or_default()
}

/// The cache file's bytes, judged on the open handle rather than the path:
/// a link is refused (`O_NOFOLLOW`), a FIFO swapped in neither blocks the
/// open (`O_NONBLOCK`) nor passes the regular-file check, and a file grown
/// past the cap is read only to one byte over it, then refused.
fn read_stat_cache(path: &Path) -> Option<Vec<u8>> {
    let cap = stat_cache_cap();
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > cap {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= cap).then_some(bytes)
}

/// [`MAX_STAT_CACHE_BYTES`]; tests lower it (`STAT_CACHE_CAP`).
fn stat_cache_cap() -> u64 {
    #[cfg(test)]
    if let Some(cap) = STAT_CACHE_CAP.with(|cap| cap.get()) {
        return cap;
    }
    MAX_STAT_CACHE_BYTES
}

/// `<base64 HMAC>\n<json>` → the entries, verified with [`STAT_CACHE_KEY`].
fn open_stat_cache(bytes: &[u8]) -> Option<HashMap<String, CachedFile>> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let split = bytes.iter().position(|&b| b == b'\n')?;
    let (tag, body) = (&bytes[..split], &bytes[split + 1..]);
    let tag = BASE64.decode(tag).ok()?;
    ring::hmac::verify(&STAT_CACHE_KEY, body, &tag).ok()?;
    let cache: StatCache = serde_json::from_slice(body).ok()?;
    if cache.version != STAT_CACHE_VERSION {
        return None;
    }
    let mut files = cache.files;
    files.retain(|path, file| sound_cache_entry(path, file));
    Some(files)
}

/// An entry worth keeping: a path History could capture and a real hash.
fn sound_cache_entry(path: &str, file: &CachedFile) -> bool {
    validate_vault_path(path).is_ok() && validate_hash(&file.1).is_ok()
}

/// The file [`open_stat_cache`] reads: the HMAC of `body`, a newline, `body`.
fn seal_stat_cache(body: &[u8]) -> Vec<u8> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let tag = ring::hmac::sign(&STAT_CACHE_KEY, body);
    let mut out = BASE64.encode(tag.as_ref()).into_bytes();
    out.push(b'\n');
    out.extend_from_slice(body);
    out
}

/// Best effort: a cache that can't be written costs the next snapshot its
/// reads, never this one its result. Batched (plain fsync, no drive flush):
/// the cache is disposable, and a torn file fails its seal. Not written
/// once `.hollow-history` is gone (`clear_history` removed it), which
/// would otherwise recreate the folder for this one file. Nor written past
/// the cap, which every load would ignore: the old file is removed instead,
/// so no stale entry outlives it.
fn save_stat_cache(vault: &Path, files: HashMap<String, CachedFile>) {
    // `symlink_metadata`: a link in its place is not a folder.
    let root = fs::symlink_metadata(history_root(vault));
    if !root.is_ok_and(|meta| meta.is_dir()) {
        return;
    }
    let cache = StatCache {
        version: STAT_CACHE_VERSION,
        files,
    };
    let body = match serde_json::to_vec(&cache) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("[history] stat cache not saved: {error}");
            return;
        }
    };
    let path = stat_cache_path(vault);
    let sealed = seal_stat_cache(&body);
    if sealed.len() as u64 > stat_cache_cap() {
        eprintln!("[history] stat cache not saved: over the size cap");
        if let Err(error) = fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[history] stale stat cache not removed: {error}");
            }
        }
        return;
    }
    if let Err(error) = crate::vault::atomic_write_batched(&path, &sealed) {
        eprintln!("[history] stat cache not saved: {error}");
    }
}

#[cfg(test)]
thread_local! {
    /// File and object content reads on this thread (OB09's tests).
    static CONTENT_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Entries snapshot walks visited on this thread (F7's tests: a scoped
    /// broker write must not walk the whole vault).
    static SNAPSHOT_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Added to the snapshot start, so a test can make a file it just wrote
    /// settled without waiting out the racy window.
    static SNAPSHOT_CLOCK_AHEAD: std::cell::Cell<std::time::Duration> =
        const { std::cell::Cell::new(std::time::Duration::ZERO) };
    /// Forces [`vouching_device`]'s file-system answer (`None`: ask statfs).
    static REAL_CTIME_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
    /// Replaces [`MAX_STAT_CACHE_BYTES`] (`None`: the real cap).
    static STAT_CACHE_CAP: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// The entries snapshot walks visited on this thread since the last call
/// (F7's tests), resetting the count.
#[cfg(test)]
pub(crate) fn take_snapshot_visits() -> usize {
    SNAPSHOT_VISITS.with(|visits| visits.replace(0))
}

/// Passes `read` through; in tests, counts it in `CONTENT_READS`.
fn count_content_read<T>(read: T) -> T {
    #[cfg(test)]
    CONTENT_READS.with(|reads| reads.set(reads.get() + 1));
    read
}

fn hidden_entry(entry: &walkdir::DirEntry, vault: &Path) -> bool {
    if entry.path() == vault {
        return false;
    }
    entry
        .file_name()
        .to_str()
        .map(|name| name.starts_with('.'))
        .unwrap_or(true)
}

fn validate_vault_root(vault: &Path) -> Result<(), String> {
    let meta = fs::metadata(vault).map_err(|e| format!("vault root: {e}"))?;
    if !meta.is_dir() {
        return Err("vault root is not a directory".into());
    }
    vault
        .canonicalize()
        .map(|_| ())
        .map_err(|e| format!("canonicalize vault root: {e}"))
}

fn validate_vault_path(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("vault path is empty".into());
    }
    let p = Path::new(path);
    if p.is_absolute() {
        return Err(format!("absolute path not allowed: {path}"));
    }
    let mut saw_normal = false;
    for component in p.components() {
        match component {
            Component::Normal(value) => {
                saw_normal = true;
                let name = value.to_string_lossy();
                if name.starts_with('.') {
                    return Err(format!("hidden vault path is not eligible: {path}"));
                }
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(format!("path escape: {path}")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!("absolute path not allowed: {path}"))
            }
        }
    }
    if saw_normal {
        Ok(())
    } else {
        Err(format!("vault path has no file component: {path}"))
    }
}

/// Resolve a restore destination without traversing symlinks, even when a
/// symlink still points somewhere inside the vault. History snapshots never
/// include symlinks, so accepting one during restore would turn a manifest's
/// regular-file path into a different write target.
fn safe_restore_target(vault: &Path, vault_path: &str) -> Result<PathBuf, String> {
    validate_vault_path(vault_path)?;
    let root = vault
        .canonicalize()
        .map_err(|e| format!("canonicalize vault root: {e}"))?;
    let mut probe = root;
    let components: Vec<_> = Path::new(vault_path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect();
    for (index, value) in components.iter().enumerate() {
        probe.push(value);
        match fs::symlink_metadata(&probe) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("restore path contains a symlink: {vault_path}"));
            }
            Ok(meta) if index + 1 < components.len() && !meta.is_dir() => {
                return Err(format!(
                    "restore path has a non-directory ancestor: {vault_path}"
                ));
            }
            Ok(meta) if index + 1 == components.len() && !meta.is_file() => {
                return Err(format!(
                    "restore target is not a regular file: {vault_path}"
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(format!("stat restore path: {e}")),
        }
    }
    crate::vault::safe_join(vault, vault_path)
}

fn ensure_history_dirs(vault: &Path) -> Result<(), String> {
    validate_vault_root(vault)?;
    let root = history_root(vault);
    ensure_real_dir(&root)?;
    ensure_real_dir(&root.join(OBJECTS_DIR))?;
    ensure_real_dir(&root.join(RUNS_DIR))
}

fn ensure_real_dir(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(format!("history path is a symlink: {}", path.display()))
        }
        Ok(meta) if !meta.is_dir() => Err(format!(
            "history path is not a directory: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|e| format!("create history directory: {e}"))
        }
        Err(e) => Err(format!("stat history directory: {e}")),
    }
}

fn history_root(vault: &Path) -> PathBuf {
    vault.join(HISTORY_DIR)
}

fn objects_root(vault: &Path) -> PathBuf {
    history_root(vault).join(OBJECTS_DIR)
}

fn runs_root(vault: &Path) -> PathBuf {
    history_root(vault).join(RUNS_DIR)
}

fn validate_run_id(run_id: &str) -> Result<(), String> {
    if run_id.is_empty()
        || run_id.len() > 180
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("invalid history run id".into());
    }
    Ok(())
}

fn validate_hash(hash: &str) -> Result<(), String> {
    if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("invalid history object hash".into())
    }
}

fn manifest_path(vault: &Path, run_id: &str) -> Result<PathBuf, String> {
    validate_run_id(run_id)?;
    Ok(runs_root(vault).join(format!("{run_id}.json")))
}

fn object_path(vault: &Path, hash: &str) -> Result<PathBuf, String> {
    validate_hash(hash)?;
    Ok(objects_root(vault).join(hash.to_ascii_lowercase()))
}

fn write_manifest(vault: &Path, manifest: &RunManifest) -> Result<(), String> {
    ensure_history_dirs(vault)?;
    let path = manifest_path(vault, &manifest.run.id)?;
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| format!("serialize history manifest: {e}"))?;
    bytes.push(b'\n');
    // Objects are written batched ([`write_object`]): this one flush makes
    // every object written so far durable before a manifest can name it.
    crate::vault::flush_dir(&objects_root(vault));
    crate::vault::atomic_write(&path, &bytes)
}

fn read_manifest(vault: &Path, run_id: &str) -> Result<RunManifest, String> {
    validate_existing_history_layout(vault, RUNS_DIR)?;
    let path = manifest_path(vault, run_id)?;
    let bytes = fs::read(&path).map_err(|e| format!("read history run {run_id}: {e}"))?;
    let manifest: RunManifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse history run {run_id}: {e}"))?;
    if manifest.run.id != run_id {
        return Err(format!("history manifest id mismatch for {run_id}"));
    }
    Ok(manifest)
}

/// Read only a manifest's run header, with the same layout and id checks as
/// [`read_manifest`]. Never creates a directory.
fn read_manifest_run(vault: &Path, run_id: &str) -> Result<HistoryRun, String> {
    if !validate_existing_history_layout(vault, RUNS_DIR)? {
        return Err(format!("history run {run_id} has no manifest"));
    }
    let path = manifest_path(vault, run_id)?;
    let bytes = fs::read(&path).map_err(|e| format!("read history run {run_id}: {e}"))?;
    let head: ManifestHead =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse history run {run_id}: {e}"))?;
    if head.run.id != run_id {
        return Err(format!("history manifest id mismatch for {run_id}"));
    }
    Ok(head.run)
}

fn read_all_manifests(vault: &Path) -> Result<Vec<RunManifest>, String> {
    scan_manifests(vault).map(|scan| scan.manifests)
}

/// Every readable manifest, and the run files that wouldn't read or parse.
struct ManifestScan {
    manifests: Vec<RunManifest>,
    unreadable: Vec<String>,
}

fn scan_manifests(vault: &Path) -> Result<ManifestScan, String> {
    if !validate_existing_history_layout(vault, RUNS_DIR)? {
        return Ok(ManifestScan {
            manifests: Vec::new(),
            unreadable: Vec::new(),
        });
    }
    let root = runs_root(vault);
    let entries = match fs::read_dir(&root) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ManifestScan {
                manifests: Vec::new(),
                unreadable: Vec::new(),
            })
        }
        Err(e) => return Err(format!("read history runs: {e}")),
    };
    let mut out = Vec::new();
    let mut unreadable = Vec::new();
    let entries = entries.map(|entry| entry.map(|entry| entry.path()));
    #[cfg(test)]
    let entries = entries.chain(scan_fault_entry());
    for entry in entries {
        // An entry that can't be read could be a run record, so it counts as
        // unreadable, never skipped as if it weren't there: retention then
        // prunes and collects nothing (review of #1383).
        let path = match entry {
            Ok(path) => path,
            Err(error) => {
                unreadable.push(format!("an entry that couldn't be listed ({error})"));
                continue;
            }
        };
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if validate_run_id(stem).is_err() {
            continue;
        }
        let manifest = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<RunManifest>(&bytes).ok())
            .filter(|manifest| manifest.run.id == stem);
        match manifest {
            Some(manifest) => out.push(manifest),
            None => unreadable.push(stem.to_string()),
        }
    }
    Ok(ManifestScan {
        manifests: out,
        unreadable,
    })
}

#[cfg(test)]
thread_local! {
    pub(super) static SCAN_FAULT_ENTRY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// With [`SCAN_FAULT_ENTRY`] set, [`scan_manifests`] meets one `runs/` entry
/// it can't read, which a real file system rarely produces on demand.
#[cfg(test)]
fn scan_fault_entry() -> Option<std::io::Result<PathBuf>> {
    SCAN_FAULT_ENTRY
        .with(std::cell::Cell::get)
        .then(|| Err(std::io::Error::other("injected unreadable entry")))
}

fn write_object(vault: &Path, hash: &str, bytes: &[u8]) -> Result<(), String> {
    ensure_history_dirs(vault)?;
    let path = object_path(vault, hash)?;
    if path.exists() {
        let existing = count_content_read(fs::read(&path));
        let existing = existing.map_err(|e| format!("read history object: {e}"))?;
        if hash_bytes(&existing) == hash.to_ascii_lowercase() {
            return Ok(());
        }
        // A corrupt object is repaired atomically from the known-good source.
    }
    // Batched: a full-vault baseline writes one object per file at run
    // start, so the drive-cache flush is deferred to the one in
    // [`write_manifest`] instead of two per object.
    crate::vault::atomic_write_batched(&path, bytes)
}

fn read_object(vault: &Path, hash: &str) -> Result<Vec<u8>, String> {
    validate_existing_history_layout(vault, OBJECTS_DIR)?;
    let expected = hash.to_ascii_lowercase();
    let path = object_path(vault, &expected)?;
    let bytes = fs::read(&path).map_err(|e| format!("read history object {expected}: {e}"))?;
    if hash_bytes(&bytes) != expected {
        return Err(format!(
            "history object {expected} failed SHA-256 verification"
        ));
    }
    Ok(bytes)
}

/// Largest version [`line_stats`] reads; bigger files get no line counts.
const LINE_STATS_MAX_BYTES: usize = 1024 * 1024;

/// Lines added and removed between two history objects (either side absent
/// for a create or delete), for the receipt's `+a −d` file chips. `None` when
/// an object is missing, fails verification, is over 1 MiB or is not UTF-8:
/// the chip then shows no counts rather than a guess. Read-only; takes no
/// history lock (objects are immutable once written).
pub(crate) fn line_stats(
    vault: &Path,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<(u32, u32)> {
    let read = |hash: Option<&str>| -> Option<String> {
        let Some(hash) = hash else {
            return Some(String::new());
        };
        let bytes = read_object(vault, hash).ok()?;
        if bytes.len() > LINE_STATS_MAX_BYTES {
            return None;
        }
        String::from_utf8(bytes).ok()
    };
    let before = read(before)?;
    let after = read(after)?;
    Some(diff_line_counts(&before, &after))
}

/// `(added, removed)` lines from `before` to `after`. Common leading and
/// trailing lines are trimmed; the middle is an exact LCS when it is small
/// (≤ 4M cells) and a multiset difference otherwise, which can undercount a
/// moved line but never overcounts.
pub(crate) fn diff_line_counts(before: &str, after: &str) -> (u32, u32) {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let a = &a[prefix..a.len() - suffix];
    let b = &b[prefix..b.len() - suffix];
    let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    if a.is_empty() || b.is_empty() {
        return (clamp(b.len()), clamp(a.len()));
    }
    let common = if a.len().saturating_mul(b.len()) <= 4_000_000 {
        let mut prev = vec![0u32; b.len() + 1];
        let mut row = vec![0u32; b.len() + 1];
        for x in a {
            for (j, y) in b.iter().enumerate() {
                row[j + 1] = if x == y {
                    prev[j] + 1
                } else {
                    row[j].max(prev[j + 1])
                };
            }
            std::mem::swap(&mut prev, &mut row);
        }
        prev[b.len()] as usize
    } else {
        let mut counts: std::collections::HashMap<&str, isize> = std::collections::HashMap::new();
        for x in a {
            *counts.entry(x).or_default() += 1;
        }
        let mut common = 0;
        for y in b {
            if let Some(n) = counts.get_mut(y) {
                if *n > 0 {
                    *n -= 1;
                    common += 1;
                }
            }
        }
        common
    };
    (clamp(b.len() - common), clamp(a.len() - common))
}

// ── As-of snapshots (master plan F5) ───────────────────────────────────────
// `asof.rs` keeps its snapshot manifests in `.hollow-history/snapshots/` and
// their contents in this object store; [`enforce_retention`] counts their
// references so object GC never takes a snapshot's content.

/// `.hollow-history/` of `vault`.
pub(crate) fn history_dir(vault: &Path) -> PathBuf {
    history_root(vault)
}

/// SHA-256 hex, as History names objects.
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    hash_bytes(bytes)
}

/// Run `f` under History's lock (so object GC can't interleave).
pub(crate) fn with_history_lock<T>(f: impl FnOnce() -> T) -> T {
    let _guard = lock_history();
    f()
}

/// Store `bytes` as object `hash` (verified), then flush the objects folder.
/// Call under [`with_history_lock`].
pub(crate) fn store_object(vault: &Path, hash: &str, bytes: &[u8]) -> Result<(), String> {
    if hash_bytes(bytes) != hash.to_ascii_lowercase() {
        return Err("history object hash mismatch".into());
    }
    write_object(vault, hash, bytes)?;
    crate::vault::flush_dir(&objects_root(vault));
    Ok(())
}

/// Read and verify object `hash`.
pub(crate) fn load_object(vault: &Path, hash: &str) -> Result<Vec<u8>, String> {
    read_object(vault, hash)
}

#[cfg(test)]
pub(crate) fn run_retention_for_test(vault: &Path) {
    let _guard = lock_history();
    enforce_retention(vault, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES).unwrap();
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// sha256 hex of a regular file's current bytes, `None` when it is absent
/// (Paperclip PR 9b's quiet check compares it with a history change's
/// `after_hash`). [`current_hash`] with a neutral error: a symlink, a
/// directory or any read failure is `Err`, which the quiet check treats as a
/// change, never as "unchanged".
pub(crate) fn file_sha256(path: &Path) -> Result<Option<String>, String> {
    current_hash(path).map_err(|error| {
        error
            .replace("restore target", "file")
            .replace("stat file", "stat")
    })
}

fn current_hash(path: &Path) -> Result<Option<String>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("stat restore target: {e}")),
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(format!(
            "restore target is not a regular file: {}",
            path.display()
        ));
    }
    let mut file = fs::File::open(path).map_err(|e| format!("read restore target: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read restore target: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Some(
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    ))
}

fn new_run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("run-{nanos}-{}-{seq}", std::process::id())
}

fn bounded(value: &str, max_chars: usize, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.chars().take(max_chars).collect()
}

fn referenced_hashes(manifests: &[RunManifest]) -> HashSet<String> {
    let mut refs = HashSet::new();
    for manifest in manifests {
        for snap in &manifest.baseline {
            refs.insert(snap.hash.clone());
        }
        for change in &manifest.run.changes {
            if let Some(hash) = &change.before_hash {
                refs.insert(hash.clone());
            }
            if let Some(hash) = &change.after_hash {
                refs.insert(hash.clone());
            }
        }
    }
    refs
}

fn object_bytes_for_refs(vault: &Path, refs: &HashSet<String>) -> u64 {
    refs.iter()
        .filter_map(|hash| object_path(vault, hash).ok())
        .filter_map(|path| fs::metadata(path).ok())
        .map(|m| m.len())
        .sum()
}

/// A row [`record_event`] wrote: finished, no baseline, no changes, from an
/// event source (not a run of an agent).
fn is_event_row(manifest: &RunManifest) -> bool {
    manifest.run.source == crate::linear_link::SOURCE
        && manifest.run.scope.is_none()
        && manifest.run.changes.is_empty()
        && manifest.baseline.is_empty()
}

fn enforce_retention(
    vault: &Path,
    max_runs: usize,
    max_age_days: i64,
    max_bytes: u64,
) -> Result<(), String> {
    // Held from the first record read to the last unlink, so no write in any
    // process stores objects its record doesn't name yet meanwhile (Q-R9).
    // Busy: this round is skipped, and nothing is pruned or collected.
    let Some(objects) = objects_cleanup_lock(vault) else {
        return Ok(());
    };
    // Object GC keeps only what the readable manifests reference, so one that
    // won't parse would lose its objects, and with them its restore. Nothing
    // is pruned until every run reads (merged plan slice 1, review B·F01).
    let ManifestScan {
        manifests,
        unreadable,
    } = scan_manifests(vault)?;
    if !unreadable.is_empty() {
        eprintln!(
            "[history] retention skipped: {} run record(s) couldn't be read ({}); nothing was pruned",
            unreadable.len(),
            unreadable.join(", ")
        );
        return Ok(());
    }
    let now = Utc::now();
    let cutoff = now - Duration::days(max_age_days.max(0));

    let (mut active, mut finished): (Vec<RunManifest>, Vec<RunManifest>) = manifests
        .into_iter()
        .partition(|m| m.run.finished_at.is_none() || m.run.status == "running");
    finished.sort_by(|a, b| b.run.started_at.cmp(&a.run.started_at));
    let (events, finished): (Vec<RunManifest>, Vec<RunManifest>) =
        finished.into_iter().partition(is_event_row);

    let finished_budget = max_runs.saturating_sub(active.len());
    let mut kept_finished = Vec::new();
    let mut removed_ids = Vec::new();
    for (rows, budget) in [(finished, finished_budget), (events, MAX_EVENT_ROWS)] {
        for (index, manifest) in rows.into_iter().enumerate() {
            let stamp = manifest
                .run
                .finished_at
                .as_deref()
                .unwrap_or(&manifest.run.started_at);
            let recent = DateTime::parse_from_rfc3339(stamp)
                .map(|d| d.with_timezone(&Utc) >= cutoff)
                .unwrap_or(true);
            if index < budget && recent {
                kept_finished.push(manifest);
            } else {
                removed_ids.push(manifest.run.id);
            }
        }
    }

    // Newest first again across both budgets: the ceiling pops the oldest.
    kept_finished.sort_by(|a, b| b.run.started_at.cmp(&a.run.started_at));

    // The storage ceiling removes oldest completed runs until all referenced
    // objects fit. Active runs are never pruned, even if their initial vault
    // baseline temporarily exceeds the ceiling.
    loop {
        let mut retained = active.clone();
        retained.extend(kept_finished.clone());
        let mut refs = referenced_hashes(&retained);
        refs.extend(pinned_hashes(vault, &retained));
        // A run's repository sidecar (Paperclip PR 16) counts with it.
        let manifest_bytes = retained
            .iter()
            .map(|m| {
                serialized_manifest_size(m).saturating_add(repo_sidecar_size(vault, &m.run.id))
            })
            .fold(0u64, u64::saturating_add);
        if object_bytes_for_refs(vault, &refs).saturating_add(manifest_bytes) <= max_bytes
            || kept_finished.is_empty()
        {
            break;
        }
        if let Some(oldest) = kept_finished.pop() {
            removed_ids.push(oldest.run.id);
        }
    }

    for id in removed_ids {
        if let Ok(path) = manifest_path(vault, &id) {
            let _ = fs::remove_file(path);
        }
        if let Ok(path) = repo_sidecar_path(vault, &id) {
            let _ = remove_repo_sidecar(&path);
        }
    }

    active.extend(kept_finished);
    let kept_ids: HashSet<String> = active.iter().map(|m| m.run.id.clone()).collect();
    sweep_orphan_repo_sidecars(vault, &kept_ids);
    sweep_step_pins(vault, &kept_ids);
    let mut refs = referenced_hashes(&active);
    refs.extend(pinned_hashes(vault, &active));
    // As-of snapshots (F5) share the store: their objects are kept too.
    match crate::asof::referenced(vault) {
        Ok(snapshot_refs) => refs.extend(snapshot_refs),
        Err(e) => {
            eprintln!("[history] object GC skipped: {e}; nothing was collected");
            return Ok(());
        }
    }
    // The incremental tip names objects a finalized manifest no longer does.
    // Only this process's tip opens here; another process's tip objects can
    // go, and its next snapshot stores any it still needs again
    // (`incremental::apply_changes`, Q-R9).
    refs.extend(incremental::tip_hashes(vault));
    // So does the stat cache (SE5), within the storage ceiling: a scoped
    // run's baseline reuses a cached hash only while its object is still
    // stored, so collecting the objects of a folder's unchanged files made
    // the next chat there read and hash the whole folder again. Only this
    // process's sealed cache opens here; an entry goes when its file
    // changes or leaves at its folder's next walk, or here once its folder
    // is gone. The cache is disposable, so when the records' objects and
    // its own would pass `max_bytes` it is the cache's that go, never a
    // record (the walk then reads those files again).
    let cached = stat_cache_hashes(vault);
    let manifest_bytes = active
        .iter()
        .map(|m| serialized_manifest_size(m).saturating_add(repo_sidecar_size(vault, &m.run.id)))
        .fold(0u64, u64::saturating_add);
    let mut with_cache = refs.clone();
    with_cache.extend(cached);
    if object_bytes_for_refs(vault, &with_cache).saturating_add(manifest_bytes) <= max_bytes {
        refs = with_cache;
    }
    gc_objects(vault, &refs, Some(&objects))?;
    Ok(())
}

fn serialized_manifest_size(manifest: &RunManifest) -> u64 {
    serde_json::to_vec_pretty(manifest)
        .map(|bytes| bytes.len().saturating_add(1) as u64)
        .unwrap_or(u64::MAX)
}

/// Remove every object `keep` doesn't name. Under the exclusive object
/// store lock. With `window` (retention), only while that hold may still
/// delete: past it, it stops and the next cleanup takes the rest.
fn gc_objects(
    vault: &Path,
    keep: &HashSet<String>,
    window: Option<&CleanupHold>,
) -> Result<(), String> {
    let root = objects_root(vault);
    let entries = match fs::read_dir(&root) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read history objects: {e}")),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !keep.contains(name) {
            if window.is_some_and(|hold| !hold.may_delete()) {
                eprintln!("[history] object cleanup stopped at its {}s window; the next one takes the rest", OBJECTS_CLEANUP_DELETE_WINDOW.as_secs());
                return Ok(());
            }
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

fn storage_status_inner(vault: &Path) -> Result<HistoryStorageStatus, String> {
    validate_vault_root(vault)?;
    let manifests = read_all_manifests(vault)?;
    let mut stamps: Vec<String> = manifests.iter().map(|m| m.run.started_at.clone()).collect();
    stamps.sort();
    let active_run_count = manifests
        .iter()
        .filter(|m| m.run.finished_at.is_none() || m.run.status == "running")
        .count();

    let mut object_count = 0usize;
    let mut object_bytes = 0u64;
    if validate_existing_history_layout(vault, OBJECTS_DIR)? {
        let entries =
            fs::read_dir(objects_root(vault)).map_err(|e| format!("read history objects: {e}"))?;
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    object_count += 1;
                    object_bytes = object_bytes.saturating_add(meta.len());
                }
            }
        }
    }

    let mut total_bytes = 0u64;
    let root = history_root(vault);
    if root.exists() {
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() {
                if let Ok(meta) = entry.metadata() {
                    total_bytes = total_bytes.saturating_add(meta.len());
                }
            }
        }
    }

    Ok(HistoryStorageStatus {
        run_count: manifests.len(),
        active_run_count,
        object_count,
        total_bytes,
        object_bytes,
        max_runs: MAX_RUNS,
        max_age_days: MAX_AGE_DAYS as u64,
        max_bytes: MAX_STORAGE_BYTES,
        oldest_run_at: stamps.first().cloned(),
        newest_run_at: stamps.last().cloned(),
    })
}

/// Validate existing history directories without creating them. Read-only
/// APIs should stay read-only, but they must also never follow a replaced
/// `.hollow-history`, `runs`, or `objects` symlink outside the vault.
fn validate_existing_history_layout(vault: &Path, leaf: &str) -> Result<bool, String> {
    validate_vault_root(vault)?;
    for path in [history_root(vault), history_root(vault).join(leaf)] {
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!("history path is a symlink: {}", path.display()));
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(format!(
                    "history path is not a directory: {}",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(format!("stat history path: {e}")),
        }
    }
    Ok(true)
}

#[path = "history_incremental.rs"]
mod incremental;

#[cfg(test)]
#[path = "automated_history_tests.rs"]
mod tests;

/// SE5: parallel scoped admission and its baseline (2026-11-17).
#[cfg(test)]
#[path = "automated_history_admission_tests.rs"]
mod admission_tests;

/// Scoped runs (build programme PR B3).
#[cfg(test)]
mod scoped_tests {
    use super::*;

    fn vault_with_workspace(ids: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        for id in ids {
            let folder = dir.path().join("workspaces").join(id);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("WORKSPACE.md"), "brief").unwrap();
        }
        fs::create_dir_all(dir.path().join("tasks")).unwrap();
        fs::write(dir.path().join("tasks/a.md"), "task").unwrap();
        dir
    }

    fn paths(run: &HistoryRun) -> Vec<String> {
        run.changes.iter().map(|c| c.vault_path.clone()).collect()
    }

    #[test]
    fn a_scoped_run_and_a_vault_run_each_record_only_their_own_writes() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let vault_run = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        // The scoped run starts while the vault lease is held.
        let scoped = try_begin_scoped(v, "workspaces/w1", "chat", Some("s-1")).unwrap();
        assert!(lease_is(v, &scoped), "a live scoped run is its own lease");
        assert!(run_is_live(v, scoped.id()));
        fs::write(
            v.join("workspaces/w1/WORKSPACE.md"),
            "brief, updated by the build",
        )
        .unwrap();
        fs::write(v.join("workspaces/w1/PLAN.md"), "plan").unwrap();
        fs::write(v.join("tasks/a.md"), "task, updated by the routine").unwrap();
        let scoped_run = scoped.finish("success").unwrap();
        assert_eq!(scoped_run.scope.as_deref(), Some("workspaces/w1"));
        assert_eq!(
            paths(&scoped_run),
            ["workspaces/w1/PLAN.md", "workspaces/w1/WORKSPACE.md"]
        );
        // The routine finishes after the build: the build's folder is not its change.
        let routine = vault_run.finish("success").unwrap();
        assert_eq!(paths(&routine), ["tasks/a.md"]);
        assert_eq!(routine.scope, None);
        assert!(!scoped_run_live(v));
    }

    #[test]
    fn a_vault_run_begun_after_a_scoped_run_released_its_folder_records_it_again() {
        let dir = vault_with_workspace(&["w2"]);
        let v = dir.path();
        let scoped = try_begin_scoped(v, "workspaces/w2", "chat", None).unwrap();
        scoped.finish("success").unwrap();
        let later = try_begin_owned(v, "chat", None).unwrap();
        fs::write(
            v.join("workspaces/w2/WORKSPACE.md"),
            "edited by a vault run",
        )
        .unwrap();
        assert_eq!(
            paths(&later.finish("success").unwrap()),
            ["workspaces/w2/WORKSPACE.md"]
        );
    }

    #[test]
    fn a_scoped_run_ignores_writes_outside_its_folder() {
        let dir = vault_with_workspace(&["w3"]);
        let v = dir.path();
        let scoped = try_begin_scoped(v, "workspaces/w3", "chat", None).unwrap();
        fs::write(v.join("tasks/b.md"), "outside the scope").unwrap();
        fs::write(v.join("workspaces/w3/outputs.md"), "inside").unwrap();
        assert_eq!(
            paths(&scoped.finish("success").unwrap()),
            ["workspaces/w3/outputs.md"]
        );
    }

    #[test]
    fn one_scoped_run_per_folder_and_vault_runs_are_never_blocked_by_one() {
        let dir = vault_with_workspace(&["w4", "w5"]);
        let v = dir.path();
        let first = try_begin_scoped(v, "workspaces/w4", "chat", None).unwrap();
        match try_begin_scoped(v, "workspaces/w4", "chat", None) {
            Err(BeginRunError::Busy(LeaseHolder::Run { id, .. })) => assert_eq!(id, first.id()),
            other => panic!("expected Busy, got {other:?}"),
        }
        let other = try_begin_scoped(v, "workspaces/w5", "chat", None).unwrap();
        assert!(lease_holder(v).is_none(), "scoped runs hold no vault lease");
        let vault_run =
            try_begin_owned(v, "heartbeat", None).expect("a vault run starts beside them");
        vault_run.finish("success").unwrap();
        other.finish("success").unwrap();
        first.finish("success").unwrap();
    }

    /// The person's restore refusal when a live run's folder holds a path
    /// it would write (F5, 2026-10-20).
    fn refused_for(error: &str, scope: &str, path: &str) -> bool {
        error.starts_with("A conversation is running in ")
            && error.contains(scope)
            && error.contains(path)
            && error.ends_with("try again when it finishes")
    }

    /// F5 (2026-10-20): a restore refuses only when a live run could be
    /// writing a path it would change, not whenever any workspace has a
    /// conversation running. Here a chat runs in `wb` while a change in
    /// `wa` and a vault-root change restore (each way: one file, the whole
    /// run, chosen versions), and the restore's own record leaves out what
    /// the `wb` chat wrote meanwhile.
    #[test]
    fn a_restore_elsewhere_runs_while_a_workspace_chat_runs() {
        let dir = vault_with_workspace(&["wa", "wb"]);
        let v = dir.path();
        let in_a = try_begin_scoped_shared(v, "workspaces/wa", "chat", None).unwrap();
        fs::write(v.join("workspaces/wa/notes.md"), "a's notes").unwrap();
        fs::write(v.join("workspaces/wa/WORKSPACE.md"), "brief, by a").unwrap();
        let in_a = in_a.finish("success").unwrap();
        let routine = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        fs::write(v.join("tasks/a.md"), "task, by the routine").unwrap();
        let routine = routine.finish("success").unwrap();

        let chat_b = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        fs::write(v.join("workspaces/wb/PLAN.md"), "b's plan").unwrap();
        let restored = restore_change(v, &in_a.id, "workspaces/wa/notes.md", false)
            .expect("a change in another workspace restores");
        assert!(!v.join("workspaces/wa/notes.md").exists());
        assert_eq!(paths(&restored), ["workspaces/wa/notes.md"]);
        let versions = restore_to_versions(
            v,
            &in_a.id,
            &[WantedVersion {
                vault_path: "workspaces/wa/WORKSPACE.md".into(),
                hash: Some(hash_bytes(b"brief")),
                size: Some(5),
            }],
        )
        .expect("chosen versions in another workspace restore")
        .unwrap();
        assert_eq!(paths(&versions), ["workspaces/wa/WORKSPACE.md"]);
        restore_run(v, &routine.id, false).expect("a vault-root change restores");
        assert_eq!(fs::read_to_string(v.join("tasks/a.md")).unwrap(), "task");
        assert_eq!(
            fs::read_to_string(v.join("workspaces/wb/PLAN.md")).unwrap(),
            "b's plan",
            "the running chat's file is untouched"
        );
        let b = chat_b.finish("success").unwrap();
        assert_eq!(paths(&b), ["workspaces/wb/PLAN.md"]);
    }

    /// A restore of a path inside a folder where a conversation is running
    /// still refuses, forced or not, naming the folder and the path, and
    /// writes nothing: for a workspace's own folder (shared) and a build
    /// workspace's (exclusive), each restore entry point.
    #[test]
    fn a_restore_inside_a_running_workspace_still_refuses() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        let earlier = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        fs::write(v.join("workspaces/wb/notes.md"), "notes").unwrap();
        let earlier = earlier.finish("success").unwrap();
        for exclusive in [false, true] {
            let live = if exclusive {
                try_begin_scoped(v, "workspaces/wb", "chat", None).unwrap()
            } else {
                try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap()
            };
            let path = "workspaces/wb/notes.md";
            for force in [false, true] {
                let one = restore_change(v, &earlier.id, path, force).unwrap_err();
                assert!(refused_for(&one, "workspaces/wb", path), "{one}");
                let all = restore_run(v, &earlier.id, force).unwrap_err();
                assert!(refused_for(&all, "workspaces/wb", path), "{all}");
            }
            let versions = restore_to_versions(
                v,
                &earlier.id,
                &[WantedVersion {
                    vault_path: path.into(),
                    hash: None,
                    size: None,
                }],
            )
            .unwrap_err();
            assert!(refused_for(&versions, "workspaces/wb", path), "{versions}");
            assert_eq!(fs::read_to_string(v.join(path)).unwrap(), "notes");
            assert!(
                list_runs(v, 50)
                    .unwrap()
                    .iter()
                    .all(|run| run.source != "historyRestore"),
                "no restore record was started"
            );
            live.finish("success").unwrap();
        }
        restore_change(v, &earlier.id, "workspaces/wb/notes.md", false)
            .expect("restores once the conversation ends");
    }

    /// A conversation in one workspace can still change a vault-root file
    /// through Hollow (a task write on its behalf). Until it finishes, a
    /// restore of that file refuses, naming the workspace; other vault-root
    /// files restore.
    #[test]
    fn a_restore_refuses_a_file_a_running_conversation_changed_through_hollow() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        fs::write(v.join("tasks/b.md"), "task b").unwrap();
        let routine = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        fs::write(v.join("tasks/a.md"), "task, by the routine").unwrap();
        fs::write(v.join("tasks/b.md"), "task b, by the routine").unwrap();
        let routine = routine.finish("success").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        let id = chat.id().to_string();
        with_live_run(v, &id, &["chat"], || {
            fs::write(v.join("tasks/a.md"), "task, by b's chat").map_err(|e| e.to_string())
        })
        .unwrap();
        let refused = restore_change(v, &routine.id, "tasks/a.md", true).unwrap_err();
        assert!(
            refused_for(&refused, "workspaces/wb", "tasks/a.md"),
            "{refused}"
        );
        let refused = restore_run(v, &routine.id, true).unwrap_err();
        assert!(
            refused_for(&refused, "workspaces/wb", "tasks/a.md"),
            "{refused}"
        );
        assert_eq!(
            fs::read_to_string(v.join("tasks/a.md")).unwrap(),
            "task, by b's chat"
        );
        restore_change(v, &routine.id, "tasks/b.md", false).expect("an untouched file restores");
        assert_eq!(fs::read_to_string(v.join("tasks/b.md")).unwrap(), "task b");
        chat.finish("success").unwrap();
    }

    /// A restore never runs beside a vault-lane run: that refusal stays.
    #[test]
    fn a_restore_still_waits_for_a_run_holding_the_vault() {
        let dir = vault_with_workspace(&["wa"]);
        let v = dir.path();
        let earlier = try_begin_owned(v, "chat", None).unwrap();
        fs::write(v.join("tasks/a.md"), "changed").unwrap();
        let earlier = earlier.finish("success").unwrap();
        let routine = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        let busy = restore_change(v, &earlier.id, "tasks/a.md", true).unwrap_err();
        assert!(busy.starts_with("Hollow is busy"), "{busy}");
        assert_eq!(fs::read_to_string(v.join("tasks/a.md")).unwrap(), "changed");
        routine.finish("success").unwrap();
        restore_change(v, &earlier.id, "tasks/a.md", false).expect("restores after");
    }

    /// F5 (2026-10-20): clearing History while a conversation runs in a
    /// workspace clears every finished record and keeps the running one's
    /// record and the stored versions it needs (its baseline, what Hollow
    /// changed for it, its step versions), so it finishes and restores
    /// normally afterwards.
    #[test]
    fn clear_history_keeps_a_running_conversations_record() {
        let dir = vault_with_workspace(&["wa", "wb"]);
        let v = dir.path();
        let finished = try_begin_owned(v, "chat", None).unwrap();
        fs::write(v.join("tasks/a.md"), "changed").unwrap();
        let finished = finished.finish("success").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        let id = chat.id().to_string();
        with_live_run(v, &id, &["chat"], || {
            fs::write(v.join("tasks/a.md"), "changed again, by b").map_err(|e| e.to_string())
        })
        .unwrap();
        let step = store_step_object(v, &id, b"a step version").unwrap();
        fs::create_dir_all(history_root(v).join("snapshots")).unwrap();
        fs::write(history_root(v).join("snapshots/2026-10-19.json"), "{}").unwrap();

        clear_history(v).expect("clears beside a workspace conversation");

        assert!(get_run(v, &finished.id).is_err(), "finished record cleared");
        assert!(!history_root(v).join("snapshots").exists());
        let left: Vec<String> = list_runs(v, 50)
            .unwrap()
            .into_iter()
            .map(|run| run.id)
            .collect();
        assert_eq!(left, [id.as_str()]);
        assert_eq!(read_object(v, &step).unwrap(), b"a step version");
        assert!(lease_holder(v).is_none(), "the clear let the vault go");

        fs::write(v.join("workspaces/wb/WORKSPACE.md"), "brief, by b").unwrap();
        let run = chat.finish("success").unwrap();
        assert_eq!(paths(&run), ["tasks/a.md", "workspaces/wb/WORKSPACE.md"]);
        restore_run(v, &run.id, false).expect("its record restores after the clear");
        assert_eq!(
            fs::read_to_string(v.join("workspaces/wb/WORKSPACE.md")).unwrap(),
            "brief"
        );
        assert_eq!(fs::read_to_string(v.join("tasks/a.md")).unwrap(), "changed");
    }

    /// SE4: known atomic mirror writers coexist with scoped chats in either
    /// admission order, including chats in the mirror's own folder.
    #[test]
    fn tracker_sync_writes_its_mirror_beside_a_chat_in_another() {
        let x = "px";
        let y = "py";
        let z = "pz";
        let dir = vault_with_workspace(&[x, y, z]);
        let v = dir.path();
        let folder = |id: &str| format!("workspaces/{id}");
        let mirror_x: BTreeSet<String> = [folder(x)].into();
        let chat_y = try_begin_scoped_shared(v, &folder(y), "chat", None).unwrap();
        let sync =
            tracker_mirror_write_lease(v, &mirror_x).expect("sync writes X while a chat runs in Y");
        try_begin_scoped_shared(v, &folder(x), "chat", None)
            .expect("a chat starts during sync in its own folder")
            .finish("success")
            .unwrap();
        try_begin_scoped_shared(v, &folder(z), "chat", None)
            .expect("a chat in a folder sync isn't writing starts")
            .finish("success")
            .unwrap();
        drop(sync);
        let chat_x = try_begin_scoped_shared(v, &folder(x), "chat", None).unwrap();
        let sync = tracker_mirror_write_lease(v, &mirror_x)
            .expect("atomic sync starts beside an existing chat");
        drop(sync);
        assert_eq!(
            sync_blocker(v, Some(&mirror_x)),
            Some(chat_x.id().to_string())
        );
        chat_x.finish("success").unwrap();
        assert_eq!(sync_blocker(v, Some(&mirror_x)), None);
        chat_y.finish("success").unwrap();
    }

    /// Rewrite `run_id`'s change for `from` to name the file `to` (an older
    /// record's spelling of the same file).
    fn respell(v: &Path, run_id: &str, from: &str, to: &str) {
        let mut manifest = read_manifest(v, run_id).unwrap();
        for change in &mut manifest.run.changes {
            if change.vault_path == from {
                change.vault_path = to.to_string();
            }
        }
        write_manifest(v, &manifest).unwrap();
    }

    /// Review of PR 861: another spelling of a file in a running
    /// conversation's folder (a case-only variant, which is the same file on
    /// a case-insensitive Mac volume, or `./` and `//`) still refuses, and
    /// so does another spelling of a file Hollow changed for it.
    #[test]
    fn a_restore_refuses_every_spelling_of_a_live_runs_file() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        fs::write(v.join("workspaces/wb/note.md"), "note").unwrap();
        let routine = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        fs::write(v.join("workspaces/wb/note.md"), "note, by the routine").unwrap();
        fs::write(v.join("tasks/a.md"), "task, by the routine").unwrap();
        let routine = routine.finish("success").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        let id = chat.id().to_string();
        with_live_run(v, &id, &["chat"], || {
            fs::write(v.join("tasks/a.md"), "task, by b's chat").map_err(|e| e.to_string())
        })
        .unwrap();
        let mut current = (
            "workspaces/wb/note.md".to_string(),
            "tasks/a.md".to_string(),
        );
        for (note, task) in [
            ("workspaces/WB/note.md", "TASKS/a.md"),
            ("./workspaces//wb/note.md", "tasks//a.md"),
            ("workspaces/wb/NOTE.md", "./tasks/A.md"),
        ] {
            respell(v, &routine.id, &current.0, note);
            respell(v, &routine.id, &current.1, task);
            current = (note.to_string(), task.to_string());
            let refused = restore_change(v, &routine.id, note, true).unwrap_err();
            assert!(
                refused.starts_with("A conversation is running in workspaces/wb"),
                "{note}: {refused}"
            );
            let refused = restore_change(v, &routine.id, task, true).unwrap_err();
            assert!(refused.contains("through Hollow"), "{task}: {refused}");
            let refused = restore_run(v, &routine.id, true).unwrap_err();
            assert!(
                refused.starts_with("A conversation is running in workspaces/wb"),
                "{refused}"
            );
        }
        assert_eq!(
            fs::read_to_string(v.join("workspaces/wb/note.md")).unwrap(),
            "note, by the routine"
        );
        assert_eq!(
            fs::read_to_string(v.join("tasks/a.md")).unwrap(),
            "task, by b's chat"
        );
        chat.finish("success").unwrap();
    }

    /// Review of PR 861: the person restoring a protected vault-root file
    /// (me/Guardrails.md) beside a running workspace conversation is Hollow's
    /// write, not that conversation's agent: no protected-change flag.
    #[test]
    fn a_protected_file_restore_beside_a_chat_flags_nothing() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        fs::create_dir_all(v.join("me")).unwrap();
        fs::write(v.join("me/Guardrails.md"), "guardrails").unwrap();
        let routine = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        fs::write(v.join("me/Guardrails.md"), "guardrails, by the routine").unwrap();
        let routine = routine.finish("success").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        restore_change(v, &routine.id, "me/Guardrails.md", true).expect("restores beside the chat");
        assert_eq!(
            fs::read_to_string(v.join("me/Guardrails.md")).unwrap(),
            "guardrails"
        );
        let run = chat.finish("success").unwrap();
        assert!(run.flags.is_empty(), "{:?}", run.flags);
    }

    /// Review of PR 861: a running conversation's step pins that can't be
    /// read refuse the clear before anything is removed, so its pinned
    /// versions never go while its pin file stays.
    #[test]
    fn clear_history_refuses_an_unreadable_live_pin_file() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        let finished = try_begin_owned(v, "chat", None).unwrap();
        fs::write(v.join("tasks/a.md"), "changed").unwrap();
        let finished = finished.finish("success").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        let id = chat.id().to_string();
        let step = store_step_object(v, &id, b"a step version").unwrap();
        let pins = step_pins_path(v, &id).unwrap();
        for bad in [
            format!("{step}\nnot-a-hash\n").into_bytes(),
            [step.as_bytes(), b"\n\xff\xfe\n"].concat(),
        ] {
            fs::write(&pins, bad).unwrap();
            let refused = clear_history(v).unwrap_err();
            assert!(refused.contains("nothing was cleared"), "{refused}");
            assert_eq!(read_object(v, &step).unwrap(), b"a step version");
            assert!(get_run(v, &finished.id).is_ok(), "nothing cleared");
            assert!(lease_holder(v).is_none(), "the clear let the vault go");
        }
        chat.finish("success").unwrap();
    }

    /// Delta review of PR 861: a restore of an instruction file stored in
    /// another case (`claude.md`, which run_guard knows as `CLAUDE.md` on a
    /// case-insensitive volume) beside a live chat flags nothing either.
    #[test]
    fn a_case_variant_instruction_file_restore_flags_nothing() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        fs::write(v.join("claude.md"), "instructions").unwrap();
        let routine = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        fs::write(v.join("claude.md"), "instructions, by the routine").unwrap();
        let routine = routine.finish("success").unwrap();
        let path = &routine.changes[0].vault_path;
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        restore_change(v, &routine.id, path, true).expect("restores beside the chat");
        assert_eq!(
            fs::read_to_string(v.join("claude.md")).unwrap(),
            "instructions"
        );
        let run = chat.finish("success").unwrap();
        assert!(run.flags.is_empty(), "{:?}", run.flags);
    }

    /// Delta review of PR 861: a step-pins folder that is a symbolic link
    /// refuses the clear before anything is removed, so a live run's pins
    /// aren't unlinked with it.
    #[cfg(unix)]
    #[test]
    fn clear_history_refuses_a_symlinked_step_pins_folder() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        let finished = try_begin_owned(v, "chat", None).unwrap();
        fs::write(v.join("tasks/a.md"), "changed").unwrap();
        let finished = finished.finish("success").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        let id = chat.id().to_string();
        let step = store_step_object(v, &id, b"a step version").unwrap();
        let pins = history_root(v).join(STEP_PINS_DIR);
        let elsewhere = v.join("pins-elsewhere");
        fs::rename(&pins, &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &pins).unwrap();
        let refused = clear_history(v).unwrap_err();
        assert!(refused.contains("nothing was cleared"), "{refused}");
        assert!(fs::symlink_metadata(&pins)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(read_object(v, &step).unwrap(), b"a step version");
        assert!(get_run(v, &finished.id).is_ok(), "nothing cleared");
        assert!(lease_holder(v).is_none());
        chat.finish("success").unwrap();
    }

    /// Delta review of PR 861: a history folder swapped for a symbolic link
    /// after the clear's checks is left alone and reported, never unlinked.
    #[cfg(unix)]
    #[test]
    fn clear_history_never_unlinks_a_history_folder_swapped_mid_clear() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        let finished = try_begin_owned(v, "chat", None).unwrap();
        fs::write(v.join("tasks/a.md"), "changed").unwrap();
        finished.finish("success").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        let id = chat.id().to_string();
        store_step_object(v, &id, b"a step version").unwrap();
        let pins = history_root(v).join(STEP_PINS_DIR);
        let elsewhere = v.join("pins-elsewhere");
        let (from, to) = (pins.clone(), elsewhere.clone());
        set_before_clear_delete_hook(move || {
            fs::rename(&from, &to).unwrap();
            std::os::unix::fs::symlink(&to, &from).unwrap();
        });
        let refused = clear_history(v).unwrap_err();
        assert!(refused.contains(STEP_PINS_DIR), "{refused}");
        assert!(fs::symlink_metadata(&pins)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(elsewhere.join(format!("{id}.txt")).exists());
        assert!(lease_holder(v).is_none());
        chat.finish("success").unwrap();
    }

    /// Delta review of PR 861: a host write to a new file is noted under its
    /// own name, not under another spelling of a file that is gone (on a
    /// case-sensitive volume they are different files; here the old one was
    /// deleted by the agent).
    #[test]
    fn a_host_write_is_never_noted_under_a_gone_files_other_spelling() {
        let dir = vault_with_workspace(&["wb"]);
        let v = dir.path();
        fs::create_dir_all(v.join("me")).unwrap();
        fs::write(v.join("me/A.md"), "a").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/wb", "chat", None).unwrap();
        fs::remove_file(v.join("me/A.md")).unwrap();
        fs::write(v.join("me/a.md"), "new").unwrap();
        crate::run_guard::note_host_vault_write(v, "me/a.md", None, Some(b"new"));
        let run = chat.finish("success").unwrap();
        assert!(
            run.flags.iter().all(|flag| flag.path != "me/a.md"),
            "{:?}",
            run.flags
        );
    }

    /// Known atomic writers never hold scoped chats; unknown writers keep
    /// the conservative exclusion in both admission orders.
    #[test]
    fn only_an_unknown_sync_lease_holds_scoped_conversations() {
        let x = "px";
        let y = "py";
        let dir = vault_with_workspace(&[x, y, "w1"]);
        let v = dir.path();
        let folder = |id: &str| format!("workspaces/{id}");
        let sync = tracker_mirror_write_lease(v, &[folder(x), folder("w1")].into()).unwrap();
        try_begin_scoped_shared(v, &folder("w1"), "chat", None)
            .expect("known mirror writer does not hold its own folder")
            .finish("success")
            .unwrap();
        try_begin_scoped_shared(v, &folder(y), "chat", None)
            .expect("a folder the sync isn't writing")
            .finish("success")
            .unwrap();
        drop(sync);
        let unnamed = background_write_lease(v).unwrap();
        for id in [y, "w1"] {
            assert!(matches!(
                try_begin_scoped_shared(v, &folder(id), "chat", None),
                Err(BeginRunError::Busy(LeaseHolder::TrackerSync))
            ));
        }
        drop(unnamed);
        let chat = try_begin_scoped_shared(v, &folder(x), "chat", None).unwrap();
        assert!(matches!(
            background_write_lease(v),
            Err(BeginRunError::Busy(_))
        ));
        chat.finish("success").unwrap();
    }

    #[test]
    fn a_dropped_scoped_run_finalizes_as_cancelled_and_frees_its_folder() {
        let dir = vault_with_workspace(&["w7"]);
        let v = dir.path();
        let id = {
            let scoped = try_begin_scoped(v, "workspaces/w7", "chat", None).unwrap();
            scoped.id().to_string()
        };
        assert_eq!(get_run(v, &id).unwrap().status, "cancelled");
        assert!(scope_holder(v, "workspaces/w7").is_none());
        try_begin_scoped(v, "workspaces/w7", "chat", None)
            .unwrap()
            .finish("success")
            .unwrap();
    }

    #[test]
    fn startup_recovery_finalizes_a_crashed_scoped_run_within_its_folder() {
        let dir = vault_with_workspace(&["w8"]);
        let v = dir.path();
        let id = new_run_id();
        let baseline = snapshot_scoped(v, "workspaces/w8").unwrap();
        write_manifest(
            v,
            &RunManifest {
                run: HistoryRun {
                    id: id.clone(),
                    source: "chat".into(),
                    source_id: None,
                    started_at: Utc::now().to_rfc3339(),
                    finished_at: None,
                    status: "running".into(),
                    changes: Vec::new(),
                    flags: Vec::new(),
                    scope: Some("workspaces/w8".into()),
                    overlaps: Vec::new(),
                },
                baseline,
                uncaptured: Vec::new(),
                unlisted: Vec::new(),
            },
        )
        .unwrap();
        fs::write(
            v.join("workspaces/w8/WORKSPACE.md"),
            "written before the crash",
        )
        .unwrap();
        fs::write(v.join("tasks/a.md"), "someone else").unwrap();
        // A crashed scoped run never blocks a vault run.
        try_begin_owned(v, "chat", None)
            .unwrap()
            .finish("success")
            .unwrap();
        assert_eq!(reconcile_interrupted(v).unwrap(), 1);
        let run = get_run(v, &id).unwrap();
        assert_eq!(run.status, "interrupted");
        assert_eq!(paths(&run), ["workspaces/w8/WORKSPACE.md"]);
        // Q-R11: a run closed by recovery says its changes are unknown, and
        // its Undo needs force; a normally finished run is not marked.
        assert!(run.changes.iter().all(|c| c.unverified));
        let refused = restore_change(v, &id, "workspaces/w8/WORKSPACE.md", false).unwrap_err();
        assert!(
            refused.contains("conflict") && refused.contains("changes unknown"),
            "{refused}"
        );
        assert_eq!(
            fs::read_to_string(v.join("workspaces/w8/WORKSPACE.md")).unwrap(),
            "written before the crash"
        );
        restore_change(v, &id, "workspaces/w8/WORKSPACE.md", true).unwrap();
    }

    fn change<'a>(run: &'a HistoryRun, path: &str) -> &'a HistoryChange {
        run.changes
            .iter()
            .find(|c| c.vault_path == path)
            .unwrap_or_else(|| panic!("{path} in {:?}", paths(run)))
    }

    /// Parallel chats in one workspace (2026-10-07): two conversations in the
    /// same workspace folder run at once. Each diff covers the whole folder,
    /// so the file written while both were live is in both records, marked
    /// on both with the other run; a file written before the second began,
    /// or after the first finished, is one run's alone.
    #[test]
    fn two_conversations_in_one_workspace_run_at_once_and_mark_what_both_changed() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let first = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-1")).unwrap();
        fs::write(v.join("workspaces/w1/a.md"), "first's, alone").unwrap();
        let second = try_begin_scoped_shared(v, "workspaces/w1", "chat", Some("s-2"))
            .expect("a second conversation in the same workspace starts at once");
        assert_eq!(scoped_run_count(v), 2);
        assert_eq!(
            (live_peers(v, first.id()), live_peers(v, second.id())),
            (1, 1)
        );
        assert!(lease_holder(v).is_none(), "neither holds the vault");
        fs::write(v.join("workspaces/w1/b.md"), "written while both ran").unwrap();
        let (first_id, second_id) = (first.id().to_string(), second.id().to_string());
        let first_run = first.finish("success").unwrap();
        assert_eq!(
            paths(&first_run),
            ["workspaces/w1/a.md", "workspaces/w1/b.md"]
        );
        assert_eq!(first_run.overlaps, vec![second_id.clone()]);
        // Its peer is still live; what the peer's diff holds too is marked
        // already (chat concurrency F1, 2026-10-20), the file from before
        // the peer began is not.
        assert_eq!(
            change(&first_run, "workspaces/w1/b.md").also_changed_by,
            vec![second_id.clone()]
        );
        assert!(change(&first_run, "workspaces/w1/a.md")
            .also_changed_by
            .is_empty());
        fs::write(v.join("workspaces/w1/c.md"), "second's, alone").unwrap();
        assert_eq!(live_peers(v, second.id()), 0, "alone again");
        let second_run = second.finish("success").unwrap();
        assert_eq!(
            paths(&second_run),
            ["workspaces/w1/b.md", "workspaces/w1/c.md"]
        );
        assert_eq!(second_run.overlaps, vec![first_id.clone()]);
        assert_eq!(
            change(&second_run, "workspaces/w1/b.md").also_changed_by,
            vec![first_id.clone()]
        );
        assert!(change(&second_run, "workspaces/w1/c.md")
            .also_changed_by
            .is_empty());
        // The first run's record is marked too, once the second finished.
        let first_run = get_run(v, &first_id).unwrap();
        assert_eq!(
            change(&first_run, "workspaces/w1/b.md").also_changed_by,
            [second_id]
        );
        assert!(change(&first_run, "workspaces/w1/a.md")
            .also_changed_by
            .is_empty());
        assert!(!scoped_run_live(v));
    }

    /// `pathconf` reads the folder's own volume (F1 delta review): its
    /// answer agrees with what a lookup in another letter case finds there.
    #[cfg(target_os = "macos")]
    #[test]
    fn pathconf_reads_the_folders_own_volume() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Mixed.md"), "x").unwrap();
        let ignores_case = dir.path().join("mIXED.md").exists();
        assert_eq!(case_insensitive_at(dir.path()), Some(ignores_case));
    }

    fn owned(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    /// Chat concurrency F1 (2026-10-20, red first): two conversations in one
    /// workspace folder each write their own file while both are live. Each
    /// run's History credits only the file its own writes named; the
    /// other's is marked unattributed (and still shared, for restore).
    #[test]
    fn each_shared_run_credits_only_the_files_its_own_writes_named() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let one = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let two = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/one.md"), "one's").unwrap();
        fs::write(v.join("workspaces/w1/two.md"), "two's").unwrap();
        let (one_id, two_id) = (one.id().to_string(), two.id().to_string());
        note_own_writes(v, &one_id, owned(&["workspaces/w1/one.md"]));
        note_own_writes(v, &two_id, owned(&["workspaces/w1/two.md"]));
        let one_run = one.finish("success").unwrap();
        let two_run = two.finish("success").unwrap();
        for (run, mine, theirs) in [
            (&one_run, "workspaces/w1/one.md", "workspaces/w1/two.md"),
            (&two_run, "workspaces/w1/two.md", "workspaces/w1/one.md"),
        ] {
            assert!(
                !change(run, mine).unattributed,
                "{mine} is {}'s own",
                run.id
            );
            assert!(
                change(run, theirs).unattributed,
                "{theirs} isn't {}'s",
                run.id
            );
        }
        // Stored that way, not only returned.
        let stored = get_run(v, &one_id).unwrap();
        assert!(change(&stored, "workspaces/w1/two.md").unattributed);
        assert!(!change(&stored, "workspaces/w1/one.md").unattributed);
    }

    /// F1: a run alone in its folder the whole time keeps every change as
    /// its own, whatever its tool calls named; one whose writes aren't
    /// known (no note) gets no marks either.
    #[test]
    fn a_run_alone_or_with_unknown_writes_gets_no_unattributed_marks() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let alone = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/shell.md"), "sed -i").unwrap();
        note_own_writes(v, alone.id(), BTreeSet::new());
        let run = alone.finish("success").unwrap();
        assert!(!change(&run, "workspaces/w1/shell.md").unattributed);

        let one = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let two = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/x.md"), "x").unwrap();
        let run = one.finish("success").unwrap();
        assert!(!change(&run, "workspaces/w1/x.md").unattributed);
        two.finish("success").unwrap();
    }

    /// F1 (red first): a read-only run beside a writer is credited with
    /// nothing: every change in its diff is the writer's.
    #[test]
    fn a_read_only_run_beside_a_writer_is_credited_with_nothing() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let writer = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let reader = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/report.md"), "the writer's").unwrap();
        note_own_writes(v, writer.id(), owned(&["workspaces/w1/report.md"]));
        note_own_writes(v, reader.id(), BTreeSet::new());
        let read_run = reader.finish("success").unwrap();
        assert_eq!(paths(&read_run), ["workspaces/w1/report.md"]);
        assert!(read_run.changes.iter().all(|c| c.unattributed));
        let write_run = writer.finish("success").unwrap();
        assert!(!change(&write_run, "workspaces/w1/report.md").unattributed);
    }

    /// F1 (red first): a run that finishes while its peer is still live
    /// marks what that peer's diff will also hold (the peer's baseline
    /// against the folder now), instead of waiting for the peer's finish.
    #[test]
    fn a_run_finishing_beside_a_live_peer_still_gets_the_overlap_mark() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let early = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/before.md"), "early's, alone").unwrap();
        let late = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/both.md"), "while both ran").unwrap();
        let early_id = early.id().to_string();
        let late_run = late.finish("success").unwrap();
        assert_eq!(late_run.overlaps, vec![early_id.clone()]);
        assert_eq!(
            change(&late_run, "workspaces/w1/both.md").also_changed_by,
            vec![early_id.clone()],
            "the live peer's diff holds it too"
        );
        // The early run (still live) finishes later and marks its side.
        let early_run = early.finish("success").unwrap();
        assert!(change(&early_run, "workspaces/w1/before.md")
            .also_changed_by
            .is_empty());
        assert_eq!(
            change(&early_run, "workspaces/w1/both.md").also_changed_by,
            vec![late_run.id.clone()]
        );
    }

    /// Chat concurrency F3 (red first): Hollow's own write in a folder where
    /// a conversation is live (a finished peer's session record) is not
    /// that conversation's change; an agent edit of the same file after it
    /// still is.
    #[test]
    fn a_host_write_in_a_live_runs_folder_stays_out_of_its_diff() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        fs::create_dir_all(v.join("workspaces/w1/sessions")).unwrap();
        let live = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        for (name, before, text) in [
            ("s-peer.md", None, "one"),
            ("s-peer.md", Some("one"), "two"),
            ("s-new.md", None, "x"),
        ] {
            host_put(v, &format!("workspaces/w1/sessions/{name}"), before, text);
        }
        // The agent edits one record after Hollow wrote it.
        fs::write(v.join("workspaces/w1/sessions/s-new.md"), "agent").unwrap();
        fs::write(v.join("workspaces/w1/mine.md"), "mine").unwrap();
        let run = live.finish("success").unwrap();
        assert_eq!(
            paths(&run),
            ["workspaces/w1/mine.md", "workspaces/w1/sessions/s-new.md"]
        );
    }

    /// Hollow writes `text` over `before` (`None`: absent) as a host write.
    fn host_put(v: &Path, rel: &str, before: Option<&str>, text: &str) {
        host_write(v, rel, || {
            fs::write(v.join(rel), text).map_err(|e| e.to_string())?;
            let commit = HostCommit::new(before.map(str::as_bytes), Some(text.as_bytes()));
            Ok(((), Some(commit)))
        })
        .unwrap();
    }

    /// F3 review (red first): a host write notes the bytes it committed,
    /// never what is on disk after it. A peer's edit landing right after
    /// Hollow's write stays in History; a write that failed after touching
    /// the file notes nothing; and a file the run's own writes named is
    /// never hidden, even when it ends exactly as Hollow left it.
    #[test]
    fn a_host_write_notes_only_what_it_committed() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        fs::create_dir_all(v.join("workspaces/w1/sessions")).unwrap();
        let live = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let raced = "workspaces/w1/sessions/s-raced.md";
        host_write(v, raced, || {
            fs::write(v.join(raced), "hollow's record").map_err(|e| e.to_string())?;
            // The peer's agent edits it before the write reports back.
            fs::write(v.join(raced), "the peer's edit").map_err(|e| e.to_string())?;
            Ok(((), Some(HostCommit::new(None, Some(b"hollow's record")))))
        })
        .unwrap();
        let failed = "workspaces/w1/sessions/s-failed.md";
        let error = host_write(v, failed, || -> Result<((), Option<HostCommit>), String> {
            fs::write(v.join(failed), "half").map_err(|e| e.to_string())?;
            Err("refused".into())
        })
        .unwrap_err();
        assert_eq!(error, "refused");
        let named = "workspaces/w1/sessions/s-named.md";
        host_put(v, named, None, "hollow's");
        note_own_writes(v, live.id(), owned(&[named]));
        let run = live.finish("success").unwrap();
        assert_eq!(paths(&run), [failed, named, raced]);
    }

    /// F3 merge review (red first): an app tool's command does its write
    /// on a `spawn_blocking_logged` worker while the broker thread, holding
    /// `HISTORY_LOCK` in `with_live_run`, waits for it. A host write there
    /// (adopting a conversation writes its session record) works under the
    /// broker's hold instead of waiting on it forever.
    #[test]
    fn a_host_write_on_a_worker_runs_under_the_brokers_hold() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path().to_path_buf();
        fs::create_dir_all(v.join("workspaces/w1/sessions")).unwrap();
        let caller = try_begin_scoped_shared(&v, "workspaces/w1", "chat", None).unwrap();
        let peer = try_begin_scoped_shared(&v, "workspaces/w1", "chat", None).unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let (vault, id) = (v.clone(), caller.id().to_string());
        std::thread::spawn(move || {
            let rel = "workspaces/w1/sessions/s-adopted.md";
            let result = with_live_run(&vault, &id, &["chat"], || {
                let worker_vault = vault.clone();
                tauri::async_runtime::block_on(crate::vault::spawn_blocking_logged(move || {
                    host_write(&worker_vault, rel, || {
                        fs::write(worker_vault.join(rel), "record").map_err(|e| e.to_string())?;
                        Ok(((), Some(HostCommit::new(None, Some(b"record")))))
                    })
                }))
                .map_err(|e| e.to_string())?
            });
            let _ = done.send(result);
        });
        let Ok(result) = finished.recv_timeout(std::time::Duration::from_secs(10)) else {
            std::mem::forget((caller, peer));
            panic!("the worker's host write waited on the broker's HISTORY_LOCK");
        };
        result.unwrap();
        let run = peer.finish("success").unwrap();
        assert!(paths(&run).is_empty(), "{:?}", paths(&run));
        caller.finish("success").unwrap();
    }

    /// F3 hold review (red first): under a `HISTORY_LOCK` hold, a worker
    /// `spawn_blocking_logged` starts has finished, under the hold, before
    /// the call returns, so it can never outlive the hold (the handle is
    /// dropped unawaited here); two started before either is awaited run
    /// one after the other, never side by side. Without a hold the worker
    /// works under none and takes the lock itself.
    #[test]
    fn a_worker_under_a_hold_finishes_before_its_spawn_returns_and_runs_alone() {
        use std::sync::atomic::{AtomicBool, AtomicUsize};
        use std::sync::Arc;
        let log = Arc::new(Mutex::new(Vec::<String>::new()));
        let inside = Arc::new(AtomicBool::new(false));
        let clock = Arc::new(AtomicUsize::new(0));
        let worker = |name: &'static str| {
            let (log, inside, clock) = (log.clone(), inside.clone(), clock.clone());
            move || {
                assert!(history_lock_held_here(), "{name} works under the hold");
                assert!(
                    !inside.swap(true, Ordering::SeqCst),
                    "{name} ran beside another"
                );
                log.lock().unwrap().push(format!(
                    "{name} in {}",
                    clock.fetch_add(1, Ordering::SeqCst)
                ));
                std::thread::yield_now();
                log.lock().unwrap().push(format!(
                    "{name} out {}",
                    clock.fetch_add(1, Ordering::SeqCst)
                ));
                inside.store(false, Ordering::SeqCst);
            }
        };
        let after_spawns = {
            let _hold = hold_history_lock_for_test();
            let first = crate::vault::spawn_blocking_logged(worker("first"));
            drop(crate::vault::spawn_blocking_logged(worker("second")));
            let after = clock.fetch_add(1, Ordering::SeqCst);
            tauri::async_runtime::block_on(first).unwrap();
            after
        };
        assert_eq!(
            *log.lock().unwrap(),
            ["first in 0", "first out 1", "second in 2", "second out 3"]
        );
        assert_eq!(
            after_spawns, 4,
            "both had finished when the spawns returned"
        );
        let unheld = tauri::async_runtime::block_on(crate::vault::spawn_blocking_logged(
            history_lock_held_here,
        ))
        .unwrap();
        assert!(!unheld, "no hold to work under");
    }

    /// F3 hold review 2 (red first): under a hold the handle is already
    /// finished and resolves on its first poll with no runtime at all, so
    /// awaiting it never waits for a runtime worker (they may all be
    /// blocked on the lock this thread holds). A panic in the closure comes
    /// back as an error naming it.
    #[test]
    fn a_worker_under_a_hold_resolves_on_the_first_poll_without_a_runtime() {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};
        let _hold = hold_history_lock_for_test();
        let mut done = crate::vault::spawn_blocking_logged(|| 7);
        assert!(matches!(done, crate::vault::LoggedJoin::Ready(_)));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            std::pin::Pin::new(&mut done).poll(&mut cx),
            Poll::Ready(Ok(7))
        ));
        let mut failed = crate::vault::spawn_blocking_logged(|| -> () { panic!("boom") });
        match std::pin::Pin::new(&mut failed).poll(&mut cx) {
            Poll::Ready(Err(error)) => assert!(error.to_string().contains("boom"), "{error}"),
            _ => panic!("the panic comes back as an error"),
        }
    }

    /// F3 (red first): a host write made from inside [`with_live_run`],
    /// which already holds `HISTORY_LOCK` on this thread (an agent's
    /// `sessions.move_agent_session` rewrites the moved chat's record),
    /// runs instead of waiting on itself, and is still left out of a live
    /// peer's diff.
    #[test]
    fn a_host_write_inside_a_live_write_does_not_wait_on_itself() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path().to_path_buf();
        fs::create_dir_all(v.join("workspaces/w1/sessions")).unwrap();
        let caller = try_begin_scoped_shared(&v, "workspaces/w1", "chat", None).unwrap();
        let peer = try_begin_scoped_shared(&v, "workspaces/w1", "chat", None).unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let (vault, id) = (v.clone(), caller.id().to_string());
        std::thread::spawn(move || {
            let rel = "workspaces/w1/sessions/s-moved.md";
            let result = with_live_run(&vault, &id, &["chat"], || {
                host_write(&vault, rel, || {
                    fs::write(vault.join(rel), "record").map_err(|e| e.to_string())?;
                    Ok(((), Some(HostCommit::new(None, Some(b"record")))))
                })
            });
            let _ = done.send(result);
        });
        let Ok(result) = finished.recv_timeout(std::time::Duration::from_secs(10)) else {
            // Dropping the leases would wait on the stuck lock too.
            std::mem::forget((caller, peer));
            panic!("the host write waited on HISTORY_LOCK, which its own thread held");
        };
        result.unwrap();
        let run = peer.finish("success").unwrap();
        assert!(paths(&run).is_empty(), "{:?}", paths(&run));
        caller.finish("success").unwrap();
    }

    /// An exclusive run (a workspace archive) keeps a shared one out of its
    /// folder and the other way round, while shared runs sit side by side.
    #[test]
    fn an_exclusive_scoped_run_holds_its_folder_alone() {
        let blocked_by = |v: &Path, scope: &str, exclusive: bool, id: &str| {
            blocked_by(v, scope, &ScopeMode::from_exclusive(exclusive), id)
        };
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let build = try_begin_scoped(v, "workspaces/w1", "chat", None).unwrap();
        assert_eq!(
            scope_blocker(v, "workspaces/w1", false),
            Some(build.id().to_string())
        );
        match try_begin_scoped_shared(v, "workspaces/w1", "chat", None) {
            Err(BeginRunError::Busy(LeaseHolder::Run { id, .. })) => assert_eq!(id, build.id()),
            other => panic!("expected Busy, got {:?}", other.map(|_| ())),
        }
        assert!(blocked_by(v, "workspaces/w1", false, build.id()));
        build.finish("success").unwrap();

        let chat = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        assert_eq!(scope_blocker(v, "workspaces/w1", false), None);
        assert_eq!(
            scope_blocker(v, "workspaces/w1", true),
            Some(chat.id().to_string())
        );
        assert!(!blocked_by(v, "workspaces/w1", false, chat.id()));
        assert!(blocked_by(v, "workspaces/w1", true, chat.id()));
        assert!(matches!(
            try_begin_scoped(v, "workspaces/w1", "chat", None),
            Err(BeginRunError::Busy(_))
        ));
        let another = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        another.finish("success").unwrap();
        chat.finish("success").unwrap();
        try_begin_scoped(v, "workspaces/w1", "chat", None)
            .expect("free again")
            .finish("success")
            .unwrap();
    }

    /// F6 part B: a build worktree's turn holds its lock alone. A second
    /// turn on that lock waits; another worktree's turn and the folder's own
    /// conversations run beside it, each a shared run of the folder whose
    /// common changes History marks; an exclusive run still keeps it out.
    #[test]
    fn a_worktree_turn_holds_its_lock_and_shares_its_folder() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let a = ScopeMode::Worktree("worktree:/a".into());
        let b = ScopeMode::Worktree("worktree:/b".into());
        let begin = |mode: &ScopeMode| {
            try_begin_scoped_mode(v, "workspaces/w1", mode.clone(), "chat", None)
        };
        let lane_a = begin(&a).unwrap();
        match begin(&a) {
            Err(BeginRunError::Busy(LeaseHolder::Run { id, .. })) => assert_eq!(id, lane_a.id()),
            other => panic!("expected Busy, got {:?}", other.map(|_| ())),
        }
        assert!(blocked_by(v, "workspaces/w1", &a, lane_a.id()));
        assert!(!blocked_by(v, "workspaces/w1", &b, lane_a.id()));
        assert!(!blocked_by(
            v,
            "workspaces/w1",
            &ScopeMode::Shared,
            lane_a.id()
        ));
        assert_eq!(
            worktree_holder(v, "workspaces/w1", "worktree:/a"),
            Some(lane_a.id().to_string())
        );
        assert_eq!(worktree_holder(v, "workspaces/w1", "worktree:/b"), None);
        let lane_b = begin(&b).expect("another worktree at once");
        let folder = begin(&ScopeMode::Shared).expect("the folder's conversations too");
        assert_eq!(
            scope_blocker_kind(v, "workspaces/w1", &a),
            Some((lane_a.id().to_string(), true)),
            "held alone, by its lock"
        );
        // Outside the folder's per-workspace count; still in the total.
        let (total, shared) = scoped_turn_counts(v);
        assert_eq!((total, shared.get("workspaces/w1").copied()), (3, Some(1)));
        assert_eq!(
            live_scope(v, lane_a.id()).map(|(_, alone)| alone),
            Some(false)
        );
        assert!(matches!(
            try_begin_scoped(v, "workspaces/w1", "chat", None),
            Err(BeginRunError::Busy(_))
        ));
        fs::write(v.join("workspaces/w1/PLAN.md"), "both").unwrap();
        let a_id = lane_a.id().to_string();
        for lease in [lane_a, lane_b, folder] {
            lease.finish("success").unwrap();
        }
        // What a worktree turn and its peers both changed is marked shared.
        let refused = restore_change(v, &a_id, "workspaces/w1/PLAN.md", false).unwrap_err();
        assert!(refused.starts_with(SHARED_RESTORE_CONFLICT), "{refused}");
        let archive = try_begin_scoped(v, "workspaces/w1", "archive", None).unwrap();
        assert!(
            begin(&a).is_err(),
            "an exclusive run keeps worktree turns out"
        );
        assert_eq!(
            worktree_holder(v, "workspaces/w1", "worktree:/a"),
            Some(archive.id().to_string())
        );
        archive.finish("success").unwrap();
    }

    /// Restoring a file another conversation also changed could undo its
    /// work: refused (one file or the whole run, naming the file) unless
    /// forced. The run's own files still restore one at a time.
    #[test]
    fn a_shared_file_restores_only_when_forced() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let first = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/a.md"), "first's").unwrap();
        let second = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/WORKSPACE.md"), "brief, by both").unwrap();
        let first_id = first.id().to_string();
        first.finish("success").unwrap();
        second.finish("success").unwrap();

        let refused = restore_run(v, &first_id, false).unwrap_err();
        assert!(
            refused.starts_with(SHARED_RESTORE_CONFLICT)
                && refused.contains("workspaces/w1/WORKSPACE.md")
                && !refused.contains("a.md"),
            "{refused}"
        );
        let refused =
            restore_change(v, &first_id, "workspaces/w1/WORKSPACE.md", false).unwrap_err();
        assert!(refused.starts_with(SHARED_RESTORE_CONFLICT), "{refused}");
        assert_eq!(
            fs::read_to_string(v.join("workspaces/w1/WORKSPACE.md")).unwrap(),
            "brief, by both",
            "nothing written"
        );
        restore_change(v, &first_id, "workspaces/w1/a.md", false).expect("its own file restores");
        assert!(!v.join("workspaces/w1/a.md").exists());
        restore_change(v, &first_id, "workspaces/w1/WORKSPACE.md", true).expect("forced");
        assert_eq!(
            fs::read_to_string(v.join("workspaces/w1/WORKSPACE.md")).unwrap(),
            "brief"
        );
    }

    /// After a crash, the registry is gone: recovery finds the peers from the
    /// records' own times and folder, and marks what both changed.
    #[test]
    fn startup_recovery_marks_what_crashed_peers_both_changed() {
        let dir = vault_with_workspace(&["w8", "w9"]);
        let v = dir.path();
        let crashed = |scope: &str| {
            let id = new_run_id();
            let baseline = snapshot_scoped(v, scope).unwrap();
            write_manifest(
                v,
                &RunManifest {
                    run: HistoryRun {
                        id: id.clone(),
                        source: "chat".into(),
                        source_id: None,
                        started_at: Utc::now().to_rfc3339(),
                        finished_at: None,
                        status: "running".into(),
                        changes: Vec::new(),
                        flags: Vec::new(),
                        scope: Some(scope.into()),
                        overlaps: Vec::new(),
                    },
                    baseline,
                    uncaptured: Vec::new(),
                    unlisted: Vec::new(),
                },
            )
            .unwrap();
            id
        };
        let a = crashed("workspaces/w8");
        let b = crashed("workspaces/w8");
        let elsewhere = crashed("workspaces/w9");
        fs::write(v.join("workspaces/w8/WORKSPACE.md"), "by one of them").unwrap();
        fs::write(v.join("workspaces/w9/WORKSPACE.md"), "another workspace").unwrap();
        assert_eq!(reconcile_interrupted(v).unwrap(), 3);
        let (run_a, run_b) = (get_run(v, &a).unwrap(), get_run(v, &b).unwrap());
        assert_eq!(run_a.overlaps, vec![b.clone()]);
        assert_eq!(run_b.overlaps, vec![a.clone()]);
        assert_eq!(
            change(&run_a, "workspaces/w8/WORKSPACE.md").also_changed_by,
            [b]
        );
        assert_eq!(
            change(&run_b, "workspaces/w8/WORKSPACE.md").also_changed_by,
            [a]
        );
        let other = get_run(v, &elsewhere).unwrap();
        assert!(other.overlaps.is_empty());
        assert!(other.changes.iter().all(|c| c.also_changed_by.is_empty()));
    }

    /// The run that started second finishing first: the other one marks
    /// both records when it finishes. With three at once, each pair is marked.
    #[test]
    fn overlaps_are_marked_whichever_finishes_first_and_for_three_runs() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let a = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let b = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let c = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let ids = [a.id().to_string(), b.id().to_string(), c.id().to_string()];
        fs::write(v.join("workspaces/w1/x.md"), "while all three ran").unwrap();
        c.finish("success").unwrap();
        b.finish("success").unwrap();
        a.finish("success").unwrap();
        for (i, id) in ids.iter().enumerate() {
            let run = get_run(v, id).unwrap();
            let mut others: Vec<String> = ids
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, id)| id.clone())
                .collect();
            others.sort();
            assert_eq!(run.overlaps, others, "run {i}");
            assert_eq!(
                change(&run, "workspaces/w1/x.md").also_changed_by,
                others,
                "run {i}"
            );
        }
    }

    /// Review of the first cut: a peer whose record is gone (pruned,
    /// unreadable) or broken may have changed anything, so every file of the
    /// run in the folder counts as shared, and a restore asks first. A peer
    /// write-back that never landed is caught again when History is read and
    /// when a restore is planned.
    #[test]
    fn a_missing_or_unmarked_peer_record_still_guards_the_restore() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        // Pruned: the first run's record is gone before the second finishes.
        let first = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let second = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/first.md"), "the first run's work").unwrap();
        let first_id = first.id().to_string();
        first.finish("success").unwrap();
        fs::remove_file(manifest_path(v, &first_id).unwrap()).unwrap();
        fs::write(v.join("workspaces/w1/second.md"), "the second's").unwrap();
        let second_run = second.finish("success").unwrap();
        for path in ["workspaces/w1/first.md", "workspaces/w1/second.md"] {
            assert_eq!(
                change(&second_run, path).also_changed_by,
                vec![first_id.clone()],
                "{path}"
            );
        }
        let refused = restore_run(v, &second_run.id, false).unwrap_err();
        assert!(refused.starts_with(SHARED_RESTORE_CONFLICT), "{refused}");
        assert!(
            v.join("workspaces/w1/first.md").exists(),
            "the first run's work stays"
        );

        // A write-back that never landed: strip the marks from both records.
        let one = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let two = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        fs::write(v.join("workspaces/w1/both.md"), "while both ran").unwrap();
        let one_id = one.id().to_string();
        one.finish("success").unwrap();
        let two_id = two.id().to_string();
        two.finish("success").unwrap();
        for id in [&one_id, &two_id] {
            let mut record = read_manifest(v, id).unwrap();
            for change in &mut record.run.changes {
                change.also_changed_by.clear();
            }
            write_manifest(v, &record).unwrap();
        }
        let listed = list_runs(v, 50).unwrap();
        let one_run = listed.iter().find(|r| r.id == one_id).unwrap();
        assert_eq!(
            change(one_run, "workspaces/w1/both.md").also_changed_by,
            vec![two_id.clone()]
        );
        assert_eq!(
            change(&get_run(v, &two_id).unwrap(), "workspaces/w1/both.md").also_changed_by,
            vec![one_id.clone()]
        );
        let refused = restore_change(v, &one_id, "workspaces/w1/both.md", false).unwrap_err();
        assert!(refused.starts_with(SHARED_RESTORE_CONFLICT), "{refused}");
        assert!(v.join("workspaces/w1/both.md").exists());
    }

    #[test]
    fn scopes_must_be_plain_workspace_folders() {
        let dir = vault_with_workspace(&["w9"]);
        let v = dir.path();
        for bad in [
            "tasks",
            "workspaces",
            "workspaces/w9/sub",
            "workspaces/../tasks",
            "/etc",
            "workspaces/missing",
        ] {
            assert!(
                matches!(
                    try_begin_scoped(v, bad, "chat", None),
                    Err(BeginRunError::Failed(_))
                ),
                "{bad} must be refused"
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(v.join("tasks"), v.join("workspaces/linked")).unwrap();
            assert!(matches!(
                try_begin_scoped(v, "workspaces/linked", "chat", None),
                Err(BeginRunError::Failed(_))
            ));
        }
        assert!(!scoped_run_live(v));
    }

    /// 2026-09-29: a Hollow app or task write made for a workspace's
    /// conversation (a scoped run) takes the vault in that run's name for
    /// the call and joins that run's History: outside its folder, but never
    /// what another live workspace wrote meanwhile. A path written twice is
    /// one change; one written and removed again is none.
    #[test]
    fn a_scoped_runs_broker_writes_join_its_history_and_nothing_else() {
        let dir = vault_with_workspace(&["w1", "w2"]);
        let v = dir.path();
        let w1 = try_begin_scoped(v, "workspaces/w1", "chat", Some("s-1")).unwrap();
        let w2 = try_begin_scoped(v, "workspaces/w2", "chat", Some("s-2")).unwrap();
        let id = w1.id().to_string();
        with_live_run(v, &id, &["chat"], || {
            assert!(
                lease_is_run(v, &id),
                "the vault is held in the caller's name during the write"
            );
            fs::write(v.join("tasks/new.md"), "created").unwrap();
            fs::write(v.join("tasks/a.md"), "task, changed through Hollow").unwrap();
            fs::write(v.join("tasks/gone.md"), "short-lived").unwrap();
            // The other conversation writes its own folder at the same time.
            fs::write(v.join("workspaces/w2/PLAN.md"), "w2's plan").unwrap();
            Ok(())
        })
        .unwrap();
        assert!(
            lease_holder(v).is_none(),
            "the vault is free again after the call"
        );
        with_live_run(v, &id, &["chat"], || {
            fs::write(v.join("tasks/new.md"), "created, then edited").unwrap();
            fs::remove_file(v.join("tasks/gone.md")).unwrap();
            Ok(())
        })
        .unwrap();
        fs::write(v.join("workspaces/w1/PLAN.md"), "w1's plan").unwrap();
        // Refused for a source the run didn't begin with, and nothing runs.
        let wrong = with_live_run(v, &id, &["workflow"], || -> Result<(), String> {
            panic!("must not run")
        });
        assert_eq!(wrong.unwrap_err(), LIVE_RUN_FINISHED_ERROR);

        let other = w2.finish("success").unwrap();
        assert_eq!(paths(&other), ["workspaces/w2/PLAN.md"]);
        let run = w1.finish("success").unwrap();
        assert_eq!(
            paths(&run),
            ["tasks/a.md", "tasks/new.md", "workspaces/w1/PLAN.md"]
        );
        let created = run
            .changes
            .iter()
            .find(|c| c.vault_path == "tasks/new.md")
            .unwrap();
        assert_eq!(
            (created.change_type.as_str(), created.before_hash.is_none()),
            ("create", true)
        );
        assert_eq!(
            read_object(v, created.after_hash.as_deref().unwrap()).unwrap(),
            b"created, then edited"
        );
        assert!(
            run.flags.is_empty(),
            "Hollow's own manifest rewrite isn't the agent's: {:?}",
            run.flags
        );
        // Once it finished, nothing more is written for it.
        let late = with_live_run(v, &id, &["chat"], || -> Result<(), String> {
            panic!("must not run")
        });
        assert_eq!(late.unwrap_err(), LIVE_RUN_FINISHED_ERROR);

        // The whole run restores, the task changes included.
        restore_run(v, &run.id, false).unwrap();
        assert_eq!(fs::read_to_string(v.join("tasks/a.md")).unwrap(), "task");
        assert!(!v.join("tasks/new.md").exists());
    }

    /// While another run holds the vault, a scoped run's broker write waits
    /// and then is refused without running (its change would land in that
    /// run's diff); once the vault is free it goes through.
    #[test]
    fn a_scoped_broker_write_waits_for_the_vault_and_then_refuses() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let routine = try_begin_owned(v, "workflow", Some("routine")).unwrap();
        let scoped = try_begin_scoped(v, "workspaces/w1", "chat", Some("s-1")).unwrap();
        let id = scoped.id().to_string();
        let busy = with_live_run(v, &id, &["chat"], || -> Result<(), String> {
            panic!("must not run")
        })
        .unwrap_err();
        assert!(
            busy.contains("holds the vault") && busy.contains("nothing changed"),
            "{busy}"
        );
        let routine_run = routine.finish("success").unwrap();
        assert!(routine_run.changes.is_empty());
        with_live_run(v, &id, &["chat"], || {
            fs::write(v.join("tasks/a.md"), "later").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(paths(&scoped.finish("success").unwrap()), ["tasks/a.md"]);
    }

    /// F1 review (red first): what a Hollow app or task write on a run's
    /// behalf wrote in its own folder is its own; a peer's direct write
    /// that lands in the folder during that write is not.
    #[test]
    fn only_what_a_broker_write_wrote_is_credited_on_the_runs_behalf() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let caller = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let peer = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        with_live_run(v, caller.id(), &["chat"], || {
            crate::vault::atomic_write(&v.join("workspaces/w1/broker.md"), b"for the caller")?;
            fs::write(v.join("workspaces/w1/peer.md"), "the peer's, meanwhile")
                .map_err(|e| e.to_string())
        })
        .unwrap();
        note_own_writes(v, caller.id(), BTreeSet::new());
        let run = caller.finish("success").unwrap();
        assert!(!change(&run, "workspaces/w1/broker.md").unattributed);
        assert!(change(&run, "workspaces/w1/peer.md").unattributed);
        peer.finish("success").unwrap();
    }

    /// A closure that fails after writing still has its writes recorded,
    /// and passes its own error through.
    #[test]
    fn a_failed_scoped_broker_write_is_still_recorded() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let scoped = try_begin_scoped(v, "workspaces/w1", "chat", None).unwrap();
        let error = with_live_run(v, scoped.id(), &["chat"], || -> Result<(), String> {
            fs::write(v.join("tasks/half.md"), "half").unwrap();
            Err("nope".into())
        })
        .unwrap_err();
        assert_eq!(error, "nope");
        assert_eq!(paths(&scoped.finish("error").unwrap()), ["tasks/half.md"]);
    }

    /// After a crash, startup recovery finalizes a scoped run with the
    /// broker changes its running manifest kept.
    #[test]
    fn startup_recovery_keeps_a_scoped_runs_broker_changes() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        let scoped = try_begin_scoped(v, "workspaces/w1", "chat", None).unwrap();
        let id = scoped.id().to_string();
        with_live_run(v, &id, &["chat"], || {
            fs::write(v.join("tasks/a.md"), "changed").map_err(|e| e.to_string())
        })
        .unwrap();
        fs::write(v.join("workspaces/w1/PLAN.md"), "plan").unwrap();
        // The process "dies": nothing in memory survives.
        std::mem::forget(scoped);
        forget_registry_run(v, &id);
        crate::run_guard::take_tracking(&id);
        assert_eq!(reconcile_interrupted(v).unwrap(), 1);
        let run = get_run(v, &id).unwrap();
        assert_eq!(run.status, "interrupted");
        assert_eq!(paths(&run), ["tasks/a.md", "workspaces/w1/PLAN.md"]);
    }

    /// Fill `folder` with `count` small files (a big vault around a write).
    fn fill(v: &Path, folder: &str, count: usize) {
        fs::create_dir_all(v.join(folder)).unwrap();
        for i in 0..count {
            fs::write(v.join(format!("{folder}/n{i}.md")), format!("note {i}")).unwrap();
        }
    }

    fn visits_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
        take_snapshot_visits();
        let out = f();
        (out, take_snapshot_visits())
    }

    /// Chat concurrency F7 (2026-10-20, red first): a scoped run's task
    /// write snapshots only the folder it touches, not the whole vault twice
    /// (nor the run's own folder, which its finish diffs), and its History
    /// still holds exactly what the write changed: a change, a create and a
    /// delete.
    #[test]
    fn a_scoped_broker_write_snapshots_only_what_it_touches() {
        let dir = vault_with_workspace(&["w1", "w2"]);
        let v = dir.path();
        fill(v, "notes", 400);
        fill(v, "workspaces/w1/notes", 200);
        fs::write(v.join("tasks/b.md"), "task b").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let id = chat.id().to_string();
        let touches = Touches::Under(vec!["tasks".into()]);
        let (written, visits) = visits_during(|| {
            with_live_run_touching(v, &id, &["chat"], touches, || {
                fs::write(v.join("tasks/a.md"), "task, by w1's chat").map_err(|e| e.to_string())?;
                fs::write(v.join("tasks/new.md"), "new").map_err(|e| e.to_string())?;
                fs::remove_file(v.join("tasks/b.md")).map_err(|e| e.to_string())
            })
        });
        written.unwrap();
        // tasks/ and its two or three files, on each side of the write.
        assert!(
            visits <= 8,
            "a task write walked {visits} entries; tasks/ holds three files"
        );
        assert!(lease_holder(v).is_none(), "the vault is free again");
        fs::write(v.join("workspaces/w1/mine.md"), "mine").unwrap();
        let run = chat.finish("success").unwrap();
        assert_eq!(
            paths(&run),
            [
                "tasks/a.md",
                "tasks/b.md",
                "tasks/new.md",
                "workspaces/w1/mine.md"
            ]
        );
        let kinds: Vec<&str> = run.changes.iter().map(|c| c.change_type.as_str()).collect();
        assert_eq!(kinds, ["modify", "delete", "create", "create"]);
        restore_run(v, &run.id, false).expect("what the write changed restores");
        assert_eq!(fs::read_to_string(v.join("tasks/b.md")).unwrap(), "task b");
        assert!(!v.join("tasks/new.md").exists());
    }

    /// F7: a touched file (a workspace's project note) and a folder that
    /// doesn't exist yet are snapshotted as such; what the write creates
    /// there is recorded. A path History can't walk safely (hidden, an
    /// escape, a link on the way) falls back to the whole vault and still
    /// records.
    #[test]
    fn a_broker_write_records_touched_files_and_falls_back_for_unsafe_paths() {
        let dir = vault_with_workspace(&["w1", "w2"]);
        let v = dir.path();
        fill(v, "notes", 100);
        fs::write(v.join("workspaces/w2/PROJECT.md"), "project").unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let id = chat.id().to_string();
        let touches = Touches::Under(vec!["workspaces/w2/PROJECT.md".into(), "projects/".into()]);
        let (written, visits) = visits_during(|| {
            with_live_run_touching(v, &id, &["chat"], touches, || {
                fs::write(v.join("workspaces/w2/PROJECT.md"), "project, moved on")
                    .map_err(|e| e.to_string())?;
                fs::create_dir_all(v.join("projects/active")).map_err(|e| e.to_string())?;
                fs::write(v.join("projects/active/p.md"), "p").map_err(|e| e.to_string())
            })
        });
        written.unwrap();
        assert!(visits <= 6, "walked {visits} entries");
        #[cfg(unix)]
        std::os::unix::fs::symlink(v.join("tasks"), v.join("linked")).unwrap();
        for unsafe_path in ["../tasks", ".hollow-history", "linked/a.md"] {
            let touches = Touches::Under(vec![unsafe_path.into()]);
            let (written, visits) = visits_during(|| {
                with_live_run_touching(v, &id, &["chat"], touches, || {
                    fs::write(v.join("notes/n1.md"), unsafe_path).map_err(|e| e.to_string())
                })
            });
            written.unwrap();
            assert!(
                visits > 200,
                "{unsafe_path}: the whole vault, twice ({visits})"
            );
        }
        let run = chat.finish("success").unwrap();
        assert_eq!(
            paths(&run),
            [
                "notes/n1.md",
                "projects/active/p.md",
                "workspaces/w2/PROJECT.md"
            ]
        );
    }

    /// F7 review (2026-10-20, red first): a link below a named folder
    /// (`tasks/linked` → `notes/`) can carry the write out of it, so the
    /// write snapshots the whole vault and History holds the file it
    /// changed through the link.
    #[cfg(unix)]
    #[test]
    fn a_link_inside_a_named_folder_falls_back_to_the_whole_vault() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        fill(v, "notes", 100);
        std::os::unix::fs::symlink("../notes", v.join("tasks/linked")).unwrap();
        let chat = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let id = chat.id().to_string();
        let touches = Touches::Under(vec!["tasks".into()]);
        let (written, visits) = visits_during(|| {
            with_live_run_touching(v, &id, &["chat"], touches, || {
                fs::write(v.join("tasks/linked/n1.md"), "through the link")
                    .map_err(|e| e.to_string())
            })
        });
        written.unwrap();
        assert!(visits > 200, "the whole vault, twice ({visits})");
        let run = chat.finish("success").unwrap();
        assert_eq!(paths(&run), ["notes/n1.md"]);
    }

    /// F7 post-write check (2026-10-20, red first): a vault write the
    /// write logged (`vault::logging_writes`) outside the folders it named
    /// is still in the run's History, with no earlier version known; a
    /// dot-file it wrote (History never diffs those) is not, and neither
    /// one makes the write walk the vault.
    #[test]
    fn a_logged_write_outside_the_named_folders_is_still_recorded() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path();
        fill(v, "notes", 100);
        let chat = try_begin_scoped_shared(v, "workspaces/w1", "chat", None).unwrap();
        let id = chat.id().to_string();
        let touches = Touches::Under(vec!["tasks".into()]);
        let (written, visits) = visits_during(|| {
            with_live_run_touching(v, &id, &["chat"], touches, || {
                crate::vault::atomic_write(&v.join("notes/n1.md"), b"outside tasks/")?;
                crate::vault::atomic_write(&v.join(".review-queue.json"), b"[]")?;
                crate::vault::atomic_write(&v.join("tasks/a.md"), b"named")
            })
        });
        written.unwrap();
        assert!(visits <= 6, "walked {visits} entries");
        let run = chat.finish("success").unwrap();
        assert_eq!(paths(&run), ["notes/n1.md", "tasks/a.md"]);
        assert_eq!(
            change(&run, "notes/n1.md").coverage_before(),
            Coverage::Unknown
        );
        assert_eq!(
            change(&run, "tasks/a.md").coverage_before(),
            Coverage::Captured
        );
    }

    /// F7's timing, on a synthetic 20,000-file vault with a warm stat
    /// cache: one scoped run's task write with whole-vault snapshots against
    /// `tasks/` only (220 task files), then ten conversations' writes at
    /// once. By hand: `cargo test --lib broker_write_timing -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "a timing benchmark on 20,000 files; run by hand with `cargo test --lib broker_write_timing -- --ignored --nocapture`"]
    fn broker_write_timing_on_a_20k_file_vault() {
        let ids: Vec<String> = (0..10).map(|i| format!("w{i}")).collect();
        let dir = vault_with_workspace(&ids.iter().map(String::as_str).collect::<Vec<_>>());
        let v = dir.path().to_path_buf();
        for d in 0..100 {
            fill(&v, &format!("notes/d{d}"), 200);
        }
        fill(&v, "tasks/more", 219);
        let settle =
            || SNAPSHOT_CLOCK_AHEAD.with(|clock| clock.set(std::time::Duration::from_secs(10)));
        settle();
        // Warm the stat cache as a running Hollow has it.
        snapshot_coverage(&v, None).unwrap();
        let chats: Vec<RunLease> = ids
            .iter()
            .map(|id| {
                try_begin_scoped_shared(&v, &format!("workspaces/{id}"), "chat", None).unwrap()
            })
            .collect();
        let tasks = Touches::Under(vec!["tasks".into(), "projects".into()]);
        for (label, touches) in [("whole vault", Touches::Vault), ("tasks/ only", tasks)] {
            let mut times: Vec<std::time::Duration> = (0..5)
                .map(|i| {
                    let started = Instant::now();
                    with_live_run_touching(&v, chats[0].id(), &["chat"], touches.clone(), || {
                        fs::write(v.join("tasks/a.md"), format!("{label} {i}"))
                            .map_err(|e| e.to_string())
                    })
                    .unwrap();
                    started.elapsed()
                })
                .collect();
            times.sort();
            let started = Instant::now();
            let writers: Vec<_> = chats
                .iter()
                .enumerate()
                .map(|(n, chat)| {
                    let (v, id, touches) = (v.clone(), chat.id().to_string(), touches.clone());
                    std::thread::spawn(move || {
                        settle();
                        with_live_run_touching(&v, &id, &["chat"], touches, || {
                            fs::write(v.join(format!("tasks/t{n}.md")), "x")
                                .map_err(|e| e.to_string())
                        })
                    })
                })
                .collect();
            let failed = writers
                .into_iter()
                .map(|writer| writer.join().map_or(true, |w| w.is_err()))
                .filter(|failed| *failed)
                .count();
            let together = started.elapsed();
            assert_eq!(failed, 0);
            eprintln!(
                "[F7 timing] {label}: one write median {:?} (min {:?}, max {:?}); ten chats at once {together:?}",
                times[2],
                times[0],
                times[4],
            );
        }
        for chat in chats {
            chat.finish("success").unwrap();
        }
    }

    /// F7 (regression guard): a vault-lane run live while a scoped run's
    /// task write waits keeps exactly its own changes, the write lands once
    /// it finishes and is that conversation's alone, and the next vault-lane
    /// run starts from it (nothing to record).
    #[test]
    fn a_vault_run_beside_a_scoped_task_write_keeps_its_history_exact() {
        let dir = vault_with_workspace(&["w1"]);
        let v = dir.path().to_path_buf();
        let chat = try_begin_scoped_shared(&v, "workspaces/w1", "chat", None).unwrap();
        let routine = try_begin_owned(&v, "workflow", Some("routine")).unwrap();
        fs::write(v.join("routine.md"), "by the routine").unwrap();
        let (vault, id) = (v.clone(), chat.id().to_string());
        let writer = std::thread::spawn(move || {
            // Full-suite contention can delay the releasing thread beyond
            // the short timeout used by refusal tests. Use the production
            // budget here while keeping refusal cases fast.
            SCOPED_WRITE_WAIT_OVERRIDE
                .with(|value| value.set(Some(std::time::Duration::from_secs(20))));
            let touches = Touches::Under(vec!["tasks".into()]);
            with_live_run_touching(&vault, &id, &["chat"], touches, || {
                fs::write(vault.join("tasks/a.md"), "by w1's chat").map_err(|e| e.to_string())
            })
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(
            fs::read_to_string(v.join("tasks/a.md")).unwrap(),
            "task",
            "the write waits for the vault"
        );
        let routine_run = routine.finish("success").unwrap();
        writer
            .join()
            .unwrap()
            .expect("lands once the routine lets go");
        assert_eq!(paths(&routine_run), ["routine.md"]);
        assert_eq!(
            fs::read_to_string(v.join("tasks/a.md")).unwrap(),
            "by w1's chat"
        );
        let next = try_begin_owned(&v, "workflow", Some("routine")).unwrap();
        assert!(paths(&next.finish("success").unwrap()).is_empty());
        let chat_run = chat.finish("success").unwrap();
        assert_eq!(paths(&chat_run), ["tasks/a.md"]);
    }
}
