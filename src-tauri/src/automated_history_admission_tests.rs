//! SE5 (session efficiency, 2026-11-17): scoped admissions walk their
//! folders at once instead of one at a time, and the baseline they take is
//! still complete before the admission returns, still incremental (the OB09
//! stat cache) and still exactly what a full walk sees. Attached from
//! `automated_history.rs` as `#[path] mod admission_tests`, so `super::` is
//! `crate::automated_history`.

use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Condvar};
use std::time::Duration;

/// How a walk left the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Left {
    /// Every walk the gate wanted was there at once.
    Met,
    /// An exclusive `HISTORY_LOCK` holder (another test in this process:
    /// the lock is global) queued while this walk held the lock shared, so
    /// a later admission correctly waits behind it (writer preference) and
    /// overlap can't be shown this time. Not a serialization.
    WriterQueued,
    /// Released by the test, or the bound passed with no other walk and no
    /// writer queued: the admissions were serialized.
    Alone,
}

/// A test's hold on the walks of one vault: each walk that reaches the
/// gate (after its stat cache load) waits until `want` walks have arrived,
/// an exclusive holder queues, the test releases it, or [`GATE_BOUND`] has
/// passed, and records how it left.
pub(super) struct WalkGate {
    want: usize,
    /// Leave when an exclusive holder queues ([`Left::WriterQueued`]):
    /// overlap gates only. A hold gate keeps its walk until released.
    yields: bool,
    /// (walks arrived, released by the test)
    state: Mutex<(usize, bool)>,
    cv: Condvar,
    met: Mutex<Vec<Left>>,
}

/// A generous bound, so a slow machine never reads as serialized; only a
/// failing (serialized) run waits it out.
const GATE_BOUND: Duration = Duration::from_secs(30);

impl WalkGate {
    fn arrived(&self) -> usize {
        self.state.lock().unwrap().0
    }

    fn met(&self) -> Vec<Left> {
        self.met.lock().unwrap().clone()
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.cv.notify_all();
    }

    /// Waits (bounded) until `n` walks are at the gate.
    fn wait_for(&self, n: usize) {
        // sleep-ok: a bounded wait for a walk on another thread to arrive.
        let state = self.state.lock().unwrap();
        let _ = self
            .cv
            .wait_timeout_while(state, GATE_BOUND, |(arrived, _)| *arrived < n)
            .unwrap();
    }
}

static WALK_GATES: Lazy<Mutex<HashMap<PathBuf, Arc<WalkGate>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Called by every non-broker walk in tests: waits at `vault`'s gate if a
/// test set one.
pub(super) fn walk_gate(vault: &Path) {
    let gate = WALK_GATES.lock().unwrap().get(vault).cloned();
    let Some(gate) = gate else {
        return;
    };
    let mut state = gate.state.lock().unwrap();
    state.0 += 1;
    gate.cv.notify_all();
    let deadline = Instant::now() + GATE_BOUND;
    let left = loop {
        if state.0 >= gate.want {
            break Left::Met;
        }
        if state.1 || Instant::now() >= deadline {
            break Left::Alone;
        }
        // Lock order: the gate's mutex, then the lock's internal leaf.
        if gate.yields && HISTORY_LOCK.state().writers_waiting > 0 {
            break Left::WriterQueued;
        }
        // sleep-ok: a bounded wait for the other admission's walk, woken
        // early by its arrival; it also re-checks for a queued writer.
        state = gate
            .cv
            .wait_timeout(state, Duration::from_millis(5))
            .unwrap()
            .0;
    };
    drop(state);
    gate.met.lock().unwrap().push(left);
}

/// Sets `vault`'s gate while alive.
struct Gate {
    vault: PathBuf,
    gate: Arc<WalkGate>,
}

impl Gate {
    /// Holds two walks until both are there (or a writer queues).
    fn overlap(vault: &Path) -> Self {
        Self::set(vault, 2, true)
    }

    /// Holds every walk until the test releases it.
    fn hold(vault: &Path) -> Self {
        Self::set(vault, usize::MAX, false)
    }

    fn set(vault: &Path, want: usize, yields: bool) -> Self {
        let gate = Arc::new(WalkGate {
            want,
            yields,
            state: Mutex::new((0, false)),
            cv: Condvar::new(),
            met: Mutex::new(Vec::new()),
        });
        WALK_GATES
            .lock()
            .unwrap()
            .insert(vault.to_path_buf(), gate.clone());
        Gate {
            vault: vault.to_path_buf(),
            gate,
        }
    }
}

impl Drop for Gate {
    fn drop(&mut self) {
        WALK_GATES.lock().unwrap().remove(&self.vault);
        self.gate.release();
    }
}

/// Moves this thread's snapshot clock ahead (so files written a minute ago
/// are settled and the stat cache vouches for them) while alive.
struct ClockAhead;

impl ClockAhead {
    fn by(secs: u64) -> Self {
        SNAPSHOT_CLOCK_AHEAD.with(|clock| clock.set(Duration::from_secs(secs)));
        ClockAhead
    }
}

impl Drop for ClockAhead {
    fn drop(&mut self) {
        SNAPSHOT_CLOCK_AHEAD.with(|clock| clock.set(Duration::ZERO));
    }
}

fn a_minute_ago() -> SystemTime {
    SystemTime::now() - Duration::from_secs(60)
}

fn write_old(path: &Path, bytes: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(a_minute_ago()).unwrap();
}

/// A vault with `files` files in each of `workspaces`, written a minute ago.
fn vault_with(workspaces: &[&str], files: usize) -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    for id in workspaces {
        for i in 0..files {
            let path = dir.path().join(format!("workspaces/{id}/f{i}.md"));
            write_old(&path, &format!("{id} file {i}"));
        }
    }
    dir
}

fn admit(vault: &Path, scope: &str) -> RunLease {
    try_begin_scoped_shared(vault, scope, "chat", None).unwrap()
}

/// The baseline admission wrote for `lease`'s run.
fn baseline_of(vault: &Path, lease: &RunLease) -> Vec<SnapshotEntry> {
    read_manifest(vault, lease.id()).unwrap().baseline
}

/// `scope`'s capture by a full walk that neither reads nor writes the stat
/// cache (no file system vouched for), so every file is read and hashed.
fn full_walk(vault: &Path, scope: &str) -> Vec<SnapshotEntry> {
    REAL_CTIME_OVERRIDE.with(|forced| forced.set(Some(false)));
    let entries = snapshot_under(vault, Some(scope)).unwrap();
    REAL_CTIME_OVERRIDE.with(|forced| forced.set(None));
    entries
}

fn reads() -> usize {
    CONTENT_READS.with(|reads| reads.replace(0))
}

/// Runs `attempt` (two admissions at once behind a fresh gate wanting both)
/// until both walks were in progress together, at most 50 times. An attempt
/// where a walk left alone (released by nothing, no writer queued) means
/// the admissions were serialized: that fails at once. An attempt spoiled
/// by another test's exclusive holder queuing is retried: writer
/// preference makes the second admission wait then, by design.
fn overlapping(mut attempt: impl FnMut() -> Vec<Left>) {
    for _ in 0..50 {
        let left = attempt();
        assert!(
            !left.contains(&Left::Alone),
            "a walk ran alone: the admissions were serialized ({left:?})"
        );
        if left == [Left::Met, Left::Met] {
            return;
        }
    }
    panic!("50 attempts, each spoiled by a queued exclusive holder");
}

/// (a) Two chats starting at once, in two workspaces or in the same one,
/// walk their folders at the same time: neither waits for the other's walk
/// to end. Before SE5 the first held `HISTORY_LOCK` for its whole walk, so
/// the second only reached its walk after the first's gate timed out.
#[test]
fn simultaneous_admissions_walk_their_folders_at_once() {
    for scopes in [
        ["workspaces/w1", "workspaces/w2"],
        ["workspaces/w1", "workspaces/w1"],
    ] {
        overlapping(|| {
            let dir = vault_with(&["w1", "w2"], 20);
            let v = dir.path().to_path_buf();
            let gate = Gate::overlap(&v);
            let leases: Vec<RunLease> = std::thread::scope(|s| {
                let handles: Vec<_> = scopes
                    .iter()
                    .map(|scope| {
                        let v = v.clone();
                        s.spawn(move || admit(&v, scope))
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });
            let left = gate.gate.met();
            drop(gate);
            for lease in leases {
                lease.finish("success").unwrap();
            }
            left
        });
    }
}

/// Two admissions that walk at once both keep their stat cache entries: the
/// second save merges into what the first wrote instead of writing back the
/// cache it loaded before the first saved (a lost update would cost the
/// next baseline a full read of the first folder).
#[test]
fn simultaneous_admissions_keep_each_others_stat_cache() {
    overlapping(|| {
        let dir = vault_with(&["w1", "w2"], 10);
        let v = dir.path().to_path_buf();
        let gate = Gate::overlap(&v);
        let leases: Vec<RunLease> = std::thread::scope(|s| {
            let handles: Vec<_> = ["workspaces/w1", "workspaces/w2"]
                .into_iter()
                .map(|scope| {
                    let v = v.clone();
                    s.spawn(move || {
                        let _clock = ClockAhead::by(60);
                        admit(&v, scope)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let left = gate.gate.met();
        drop(gate);
        // Whatever the overlap, neither save may lose the other's entries.
        let cached = load_stat_cache(&v);
        for id in ["w1", "w2"] {
            let n = cached
                .keys()
                .filter(|path| path.starts_with(&format!("workspaces/{id}/")))
                .count();
            assert_eq!(n, 10, "{id}'s entries survived the other's save");
        }
        for lease in leases {
            lease.finish("success").unwrap();
        }
        left
    });
}

/// (b) A second admission's baseline reads only the file that changed since
/// the first (the stat cache vouches for the rest) and equals what a full
/// walk that reads every file captures.
#[test]
fn an_incremental_baseline_reads_only_changed_files_and_equals_a_full_walk() {
    let dir = vault_with(&["w1"], 50);
    let v = dir.path();
    let _clock = ClockAhead::by(60);
    reads();
    let first = admit(v, "workspaces/w1");
    assert_eq!(reads(), 50, "a cold baseline reads every file");
    first.finish("success").unwrap();
    write_old(&v.join("workspaces/w1/f7.md"), "w1 file 7, edited");
    reads();
    let second = admit(v, "workspaces/w1");
    assert_eq!(reads(), 1, "only the edited file is read and hashed");
    let baseline = baseline_of(v, &second);
    assert_eq!(baseline.len(), 50);
    assert_eq!(baseline, full_walk(v, "workspaces/w1"));
    second.finish("success").unwrap();
}

/// (c) The join: an admission returns only once its baseline is on disk,
/// so the agent's first write (which can only follow the admission) is
/// always a change of the run, measured against the version before it.
#[test]
fn an_admission_returns_only_after_its_baseline_is_written() {
    let dir = vault_with(&["w1"], 3);
    let v = dir.path().to_path_buf();
    // Held at the gate until the test releases it.
    let gate = Gate::hold(&v);
    let returned = AtomicUsize::new(0);
    let records = || {
        fs::read_dir(runs_root(&v))
            .map(|entries| entries.count())
            .unwrap_or(0)
    };
    let lease = std::thread::scope(|s| {
        let admission = s.spawn(|| {
            let lease = admit(&v, "workspaces/w1");
            returned.store(1, Ordering::SeqCst);
            lease
        });
        gate.gate.wait_for(1);
        assert_eq!(gate.gate.arrived(), 1, "the walk reached the gate");
        assert_eq!(returned.load(Ordering::SeqCst), 0, "not admitted mid-walk");
        assert_eq!(records(), 0, "no record before the baseline is walked");
        gate.gate.release();
        let lease = admission.join().unwrap();
        assert_eq!(records(), 1, "the record is on disk when begin returns");
        lease
    });
    drop(gate);
    let baseline = baseline_of(&v, &lease);
    assert_eq!(baseline.len(), 3);
    let before = hash_at(&baseline, "workspaces/w1/f1.md");
    fs::write(v.join("workspaces/w1/f1.md"), "the agent's first write").unwrap();
    let run = lease.finish("success").unwrap();
    assert_eq!(run.changes.len(), 1);
    let change = &run.changes[0];
    assert_eq!(change.vault_path, "workspaces/w1/f1.md");
    assert_eq!(change.change_type, "modify");
    assert_eq!(change.before_hash.as_deref(), Some(before.as_str()));
}

/// A peer's finish never sees a run that is admitted but has no baseline
/// yet: it waits for the walk. Otherwise the peer would read no record for
/// the new run and mark every file it changed as possibly the new run's
/// too (`peer_paths` of a missing record), which the new run never touched.
#[test]
fn a_peers_finish_waits_for_a_baseline_in_progress() {
    let dir = vault_with(&["w1"], 3);
    let v = dir.path().to_path_buf();
    let first = admit(&v, "workspaces/w1");
    fs::write(v.join("workspaces/w1/f0.md"), "the first chat's edit").unwrap();
    let gate = Gate::hold(&v);
    let finished = AtomicUsize::new(0);
    let (second, first_run) = std::thread::scope(|s| {
        let admission = s.spawn(|| admit(&v, "workspaces/w1"));
        gate.gate.wait_for(1);
        let finish = s.spawn(|| {
            let run = first.finish("success").unwrap();
            finished.store(1, Ordering::SeqCst);
            run
        });
        // sleep-ok: gives the finish time to (wrongly) complete mid-walk.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(finished.load(Ordering::SeqCst), 0, "no finish mid-walk");
        gate.gate.release();
        (admission.join().unwrap(), finish.join().unwrap())
    });
    drop(gate);
    assert_eq!(first_run.changes.len(), 1);
    assert!(
        first_run.changes[0].also_changed_by.is_empty(),
        "the second chat changed nothing: {:?}",
        first_run.changes[0].also_changed_by
    );
    let second_run = second.finish("success").unwrap();
    assert!(second_run.changes.is_empty());
}

/// (d) A file changed (same size, old mtime put back), one deleted and one
/// added during a run are recorded exactly as a full walk records them, and
/// the next incremental baseline equals a full walk's.
#[test]
fn changes_between_baselines_are_seen_as_a_full_walk_sees_them() {
    let dir = vault_with(&["w1"], 5);
    let v = dir.path();
    let _clock = ClockAhead::by(60);
    // Warm the cache, so the run's baseline is incremental.
    admit(v, "workspaces/w1").finish("success").unwrap();
    reads();
    let run = admit(v, "workspaces/w1");
    assert_eq!(reads(), 0, "every file vouched for");
    let before = baseline_of(v, &run);
    assert_eq!(before, full_walk(v, "workspaces/w1"));
    // Same length as "w1 file 1", rewritten in place with its original
    // mtime put back: a size-and-mtime cache would be fooled; ctime isn't.
    let f1 = v.join("workspaces/w1/f1.md");
    let mtime = fs::metadata(&f1).unwrap().modified().unwrap();
    fs::write(&f1, "w1 FILE 1").unwrap();
    fs::File::options()
        .write(true)
        .open(&f1)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_eq!(fs::metadata(&f1).unwrap().modified().unwrap(), mtime);
    fs::remove_file(v.join("workspaces/w1/f2.md")).unwrap();
    write_old(&v.join("workspaces/w1/new.md"), "added");
    let finished = run.finish("success").unwrap();
    let mut seen: Vec<(String, String)> = finished
        .changes
        .iter()
        .map(|c| (c.vault_path.clone(), c.change_type.clone()))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        [
            ("workspaces/w1/f1.md".to_string(), "modify".to_string()),
            ("workspaces/w1/f2.md".to_string(), "delete".to_string()),
            ("workspaces/w1/new.md".to_string(), "create".to_string()),
        ]
    );
    let next = admit(v, "workspaces/w1");
    assert_eq!(baseline_of(v, &next), full_walk(v, "workspaces/w1"));
    next.finish("success").unwrap();
}

fn hash_at(entries: &[SnapshotEntry], path: &str) -> String {
    entries
        .iter()
        .find(|entry| entry.vault_path == path)
        .unwrap()
        .hash
        .clone()
}

/// Object cleanup keeps the objects the stat cache names, drops the entries
/// (and objects) of a folder that is gone, and keeps none past the storage
/// ceiling; `storage_status` counts what is on disk.
#[test]
fn cleanup_keeps_cached_objects_only_for_live_folders_within_the_ceiling() {
    let dir = vault_with(&["w1", "w2"], 4);
    let v = dir.path();
    let _clock = ClockAhead::by(60);
    admit(v, "workspaces/w1").finish("success").unwrap();
    admit(v, "workspaces/w2").finish("success").unwrap();
    let hashes = |id: &str| -> BTreeSet<String> {
        load_stat_cache(v)
            .into_iter()
            .filter(|(path, _)| path.starts_with(&format!("workspaces/{id}/")))
            .map(|(_, file)| file.1)
            .collect()
    };
    let stored = || -> BTreeSet<String> {
        fs::read_dir(objects_root(v))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect()
    };
    let (w1, w2) = (hashes("w1"), hashes("w2"));
    assert_eq!((w1.len(), w2.len()), (4, 4));
    assert!(
        w1.is_subset(&stored()) && w2.is_subset(&stored()),
        "kept by the cache"
    );

    fs::remove_dir_all(v.join("workspaces/w1")).unwrap();
    enforce_retention(v, MAX_RUNS, MAX_AGE_DAYS, MAX_STORAGE_BYTES).unwrap();
    assert!(
        hashes("w1").is_empty(),
        "a gone folder's entries are dropped"
    );
    assert!(stored().is_disjoint(&w1), "and their objects collected");
    assert!(w2.is_subset(&stored()), "a live folder's stay");
    let status = storage_status(v).unwrap();
    let on_disk: u64 = stored()
        .iter()
        .map(|hash| fs::metadata(object_path(v, hash).unwrap()).unwrap().len())
        .sum();
    assert_eq!(status.object_count, stored().len());
    assert_eq!(status.object_bytes, on_disk);

    // Past the ceiling the cache's objects go (records are settled first,
    // so the cache never costs one).
    enforce_retention(v, MAX_RUNS, MAX_AGE_DAYS, 1).unwrap();
    assert!(
        stored().is_disjoint(&w2),
        "the cache keeps nothing over the ceiling"
    );
    let (_, reads) = {
        CONTENT_READS.with(|reads| reads.set(0));
        let lease = admit(v, "workspaces/w2");
        let n = CONTENT_READS.with(|reads| reads.get());
        lease.finish("success").unwrap();
        ((), n)
    };
    assert_eq!(reads, 4, "the next baseline reads its files again");
}

/// `HISTORY_LOCK`'s lock: an exclusive waiter keeps new shared holders out
/// (so chat starts can't hold a finish off), and a guard is released when
/// its holder panics.
#[test]
fn history_lock_prefers_a_waiting_exclusive_holder_and_releases_on_panic() {
    let lock = Arc::new(HistoryLock::default());
    lock.lock_shared();
    let order = Arc::new(Mutex::new(Vec::new()));
    let writer = {
        let (lock, order) = (lock.clone(), order.clone());
        std::thread::spawn(move || {
            lock.lock_exclusive();
            order.lock().unwrap().push("exclusive");
            lock.unlock(true);
        })
    };
    let deadline = Instant::now() + GATE_BOUND;
    while lock.state().writers_waiting == 0 && Instant::now() < deadline {
        // sleep-ok: bounded poll for the writer to queue.
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(lock.state().writers_waiting, 1);
    let reader = {
        let (lock, order) = (lock.clone(), order.clone());
        std::thread::spawn(move || {
            lock.lock_shared();
            order.lock().unwrap().push("shared");
            lock.unlock(false);
        })
    };
    // sleep-ok: gives a wrongly admitted shared holder time to show.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(lock.state().readers, 1, "the new shared holder waits");
    lock.unlock(false);
    writer.join().unwrap();
    reader.join().unwrap();
    assert_eq!(*order.lock().unwrap(), ["exclusive", "shared"]);

    // The global lock's guard releases on unwind.
    let panicked = std::thread::spawn(|| {
        let _guard = lock_history();
        panic!("holder panics");
    })
    .join();
    assert!(panicked.is_err());
    // Would wait forever had the panicking holder's guard not released it.
    drop(lock_history());
}
