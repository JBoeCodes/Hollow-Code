//! The cross-process run permit (Paperclip PR 7): at most [`PERMIT_SLOTS`]
//! agent CLIs that do NOT hold the full-vault lease may run at once, across
//! every Hollow process on this machine.
//!
//! The full-vault lease (`automated_history`) already serializes every run
//! that writes the vault. Some agent work never takes it: the one-shot text
//! helpers (`claude_runner`, used by task, project, meeting and learning
//! assists) and the teammate delegates that `hollow team-mcp` starts from
//! inside another agent's run. An in-process semaphore cannot bound the
//! latter, because each team-mcp server is its own process, so the permit is
//! a set of `flock` files instead: `slot-{n}.lock` in [`permits_dir`]. The
//! kernel releases a lock when its descriptor closes or its process dies, so
//! a crash never leaves a slot taken.
//!
//! Rules:
//! - Every function takes the directory, so tests use their own temp dirs and
//!   never contend with each other or with an installed Hollow.
//! - A caller that already holds the full-vault lease for this run skips the
//!   permit (it is already the one lease-holding run), after checking with
//!   `automated_history::lease_is` that the lease file still names it.
//! - Permit holders never wait on the lease or the run queue, so the permit
//!   and the lease cannot deadlock.
//! - Safe inside MCP subprocesses: it creates no process group and signals
//!   nothing (PR 1's rule for `team_mcp.rs`).
//!
//! **Pressure gate (Paperclip PR 20).** [`acquire`] also holds a caller back
//! while the machine is under pressure: when `node` processes number at
//! least `CLAUDE_NODE_PROC_LIMIT` (default 60, the toolchain guard's own
//! backstop; unset, zero or garbage means 60) or macOS reports memory
//! pressure at warning or critical (`kern.memorystatus_vm_pressure_level`
//! ≥ 2; the free-memory percentage is only quoted). A gated caller does not
//! even try the slots; it keeps polling within its existing wait and, if the
//! pressure outlasts it, gets [`pressure_message`] instead of the busy one.
//! There is no second limit and no separate wait: the 2-slot permit stays
//! the concurrency limit. An unreadable probe never gates. `hollow team-mcp`
//! returns the refusal as the delegation's tool error text (it cannot see
//! the GUI's session state); a GUI one-shot returns it as its error. Budgets
//! are not consulted here.

use crate::flock::HeldLock;
use crate::process_group::StopSignal;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Concurrent lease-bypassing agent CLIs allowed machine-wide.
pub const PERMIT_SLOTS: usize = 2;
/// How often a waiting caller retries the slots.
const POLL: Duration = Duration::from_millis(250);

/// A held slot. Dropping it unlocks the slot at once ([`HeldLock`]), even
/// while a child being spawned still holds a copy of the descriptor.
#[derive(Debug)]
pub struct Permit {
    #[cfg_attr(not(test), allow(dead_code))]
    lock: HeldLock,
    /// Which slot this is; only the tests look.
    #[cfg(test)]
    slot: usize,
}

impl Permit {
    /// Which slot (0-based) this permit holds.
    #[cfg(test)]
    pub fn slot(&self) -> usize {
        self.slot
    }

    /// A second descriptor on the slot's lock (tests only).
    #[cfg(test)]
    fn descriptor_copy(&self) -> std::fs::File {
        self.lock.descriptor_copy()
    }
}

/// Where the slot files live: under the app data dir, outside the vault.
/// Test builds get a per-process temp dir, so a test can never lock a slot an
/// installed Hollow is using (pinned by
/// `permit_files_stay_out_of_the_real_app_data_dir`).
#[cfg(not(test))]
pub fn permits_dir() -> PathBuf {
    crate::instance::support_dir()
        .map(|dir| dir.join("RunPermits"))
        .unwrap_or_else(|| std::env::temp_dir().join("hollow-run-permits"))
}

#[cfg(test)]
pub fn permits_dir() -> PathBuf {
    crate::test_temp::process_dir("hollow-test-permits-")
}

fn ensure_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create run permit dir: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best effort: the slots hold no data, but nobody else needs them.
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Take a free slot now, or `Ok(None)` when every slot is held. `Err` only
/// when the slot files themselves cannot be opened.
pub fn try_acquire(dir: &Path) -> Result<Option<Permit>, String> {
    ensure_dir(dir)?;
    for slot in 0..PERMIT_SLOTS {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(format!("slot-{slot}.lock")))
            .map_err(|e| format!("open run permit slot {slot}: {e}"))?;
        match HeldLock::try_take(file) {
            Ok(lock) => {
                return Ok(Some(Permit {
                    lock,
                    #[cfg(test)]
                    slot,
                }))
            }
            Err(std::fs::TryLockError::WouldBlock) => continue,
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("lock run permit slot {slot}: {e}"))
            }
        }
    }
    Ok(None)
}

/// The refusal when no slot frees up within the wait.
pub fn busy_message(label: &str, waited: Duration) -> String {
    format!(
        "Hollow is already running {PERMIT_SLOTS} background agents; {label} could not start \
         within {}s. Try again when one of them finishes.",
        waited.as_secs()
    )
}

/// The node-process backstop when `CLAUDE_NODE_PROC_LIMIT` is unset or bad.
pub const DEFAULT_NODE_PROC_LIMIT: usize = 60;
/// `kern.memorystatus_vm_pressure_level` at or above this gates (2 is
/// macOS's warning level, 4 critical).
const PRESSURE_LEVEL_GATE: i32 = 2;

/// One reading of the machine ([`sample_system`]). `None` fields could not
/// be read and never gate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PressureSample {
    pub node_procs: Option<usize>,
    pub vm_pressure_level: Option<i32>,
    pub free_pct: Option<u8>,
}

/// Why a caller is held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    NodeProcesses { count: usize, limit: usize },
    MemoryPressure { level: i32, free_pct: Option<u8> },
}

/// Whether `sample` gates a caller. Pure.
pub fn assess(sample: &PressureSample, node_limit: usize) -> Option<Pressure> {
    if let Some(count) = sample.node_procs {
        if count >= node_limit {
            return Some(Pressure::NodeProcesses {
                count,
                limit: node_limit,
            });
        }
    }
    match sample.vm_pressure_level {
        Some(level) if level >= PRESSURE_LEVEL_GATE => Some(Pressure::MemoryPressure {
            level,
            free_pct: sample.free_pct,
        }),
        _ => None,
    }
}

/// `CLAUDE_NODE_PROC_LIMIT` through `get` (the environment in production):
/// a positive integer, else [`DEFAULT_NODE_PROC_LIMIT`]. Note that launchd
/// does not pass a shell's environment, so the installed app uses 60.
pub fn node_limit_from_env(get: impl Fn(&str) -> Option<String>) -> usize {
    get("CLAUDE_NODE_PROC_LIMIT")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|limit| *limit > 0)
        .unwrap_or(DEFAULT_NODE_PROC_LIMIT)
}

#[cfg(test)]
thread_local! {
    static TEST_PRESSURE: std::cell::Cell<PressureSample> =
        const { std::cell::Cell::new(PressureSample { node_procs: None, vm_pressure_level: None, free_pct: None }) };
}

/// Make [`sample_system`] report `sample` on this thread (tests only; the
/// default is a clear, all-unknown sample, so machine state never leaks
/// into the suite).
#[cfg(test)]
pub fn set_test_pressure(sample: PressureSample) {
    TEST_PRESSURE.with(|cell| cell.set(sample));
}

/// Read the machine now ([`crate::proc_info`]). In tests, the sample
/// [`set_test_pressure`] set on this thread.
pub fn sample_system() -> PressureSample {
    #[cfg(test)]
    {
        TEST_PRESSURE.with(|cell| cell.get())
    }
    #[cfg(not(test))]
    {
        PressureSample {
            node_procs: crate::proc_info::node_process_count(),
            vm_pressure_level: crate::proc_info::vm_pressure_level(),
            free_pct: crate::proc_info::free_memory_pct(),
        }
    }
}

/// macOS's memory pressure level now when it is warning or worse (the
/// gate [`assess`] applies), else `None`: the run pump's memory gate
/// which allows three bootstrap turns before holding new starts. Only the memory
/// signal (one sysctl, no process-table walk): the node-process backstop
/// stays the consults' own. In tests, what [`set_test_pressure`] set on
/// this thread.
pub fn memory_pressure_level() -> Option<i32> {
    #[cfg(test)]
    let level = TEST_PRESSURE.with(|cell| cell.get()).vm_pressure_level;
    #[cfg(not(test))]
    let level = crate::proc_info::vm_pressure_level();
    level.filter(|level| *level >= PRESSURE_LEVEL_GATE)
}

/// The refusal when pressure outlasts the wait. One line, no em dash.
pub fn pressure_message(label: &str, waited: Duration, pressure: &Pressure) -> String {
    let reason = match pressure {
        Pressure::NodeProcesses { count, limit } => {
            format!("{count} node processes are running (the limit is {limit})")
        }
        Pressure::MemoryPressure { level, free_pct } => {
            let name = match level {
                2 => "warning".to_string(),
                4 => "critical".to_string(),
                other => format!("level {other}"),
            };
            match free_pct {
                Some(free) => format!("macOS reports memory pressure ({name}, {free}% free)"),
                None => format!("macOS reports memory pressure ({name})"),
            }
        }
    };
    format!(
        "Hollow held back {label} for {}s: {reason}. Try again when some work finishes.",
        waited.as_secs()
    )
}

/// Wait up to `max_wait` for a slot, polling every 250 ms, while the machine
/// is not under pressure (see the module doc). Returns `Err` at once when
/// `stop` is requested, [`pressure_message`] when pressure held the caller
/// back until the wait was spent, and [`busy_message`] otherwise.
pub async fn acquire(
    dir: &Path,
    label: &str,
    max_wait: Duration,
    stop: Option<&StopSignal>,
) -> Result<Permit, String> {
    let limit = node_limit_from_env(|key| std::env::var(key).ok());
    acquire_with(dir, label, max_wait, stop, &sample_system, limit).await
}

/// [`acquire`] with the machine sample and node limit injected.
pub async fn acquire_with(
    dir: &Path,
    label: &str,
    max_wait: Duration,
    stop: Option<&StopSignal>,
    sample: &(dyn Fn() -> PressureSample + Sync),
    node_limit: usize,
) -> Result<Permit, String> {
    let started = Instant::now();
    loop {
        if stop.is_some_and(|s| s.requested()) {
            return Err(format!(
                "{label} was stopped while waiting for a background agent slot"
            ));
        }
        let pressure = assess(&sample(), node_limit);
        if pressure.is_none() {
            if let Some(permit) = try_acquire(dir)? {
                return Ok(permit);
            }
        }
        let waited = started.elapsed();
        if waited >= max_wait {
            return Err(match &pressure {
                Some(pressure) => pressure_message(label, max_wait, pressure),
                None => busy_message(label, max_wait),
            });
        }
        let pause = POLL.min(max_wait - waited);
        match stop {
            Some(stop) => {
                tokio::select! {
                    _ = tokio::time::sleep(pause) => {}
                    _ = stop.wait() => {}
                }
            }
            None => tokio::time::sleep(pause).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn permits_are_exclusive_across_handles_and_release_on_drop() {
        let dir = TempDir::new().unwrap();
        let first = try_acquire(dir.path()).unwrap().expect("slot one is free");
        let second = try_acquire(dir.path()).unwrap().expect("slot two is free");
        assert_ne!(first.slot(), second.slot());
        assert!(
            try_acquire(dir.path()).unwrap().is_none(),
            "a third caller finds every slot held"
        );
        let freed = first.slot();
        // A child another test is spawning right now holds a copy of the
        // descriptor until it execs; this copy plays that child, so the
        // release must not wait for it (merged plan wave 2 lock flake).
        let child_copy = first.descriptor_copy();
        drop(first);
        let again = try_acquire(dir.path())
            .unwrap()
            .expect("dropping a permit frees its slot");
        assert_eq!(again.slot(), freed);
        drop(second);
        drop(again);
        drop(child_copy);
    }

    #[tokio::test]
    async fn a_saturated_permit_times_out_or_stops_with_a_busy_error() {
        let dir = TempDir::new().unwrap();
        let _a = try_acquire(dir.path()).unwrap().unwrap();
        let _b = try_acquire(dir.path()).unwrap().unwrap();

        let started = Instant::now();
        let error = acquire(
            dir.path(),
            "a task assist",
            Duration::from_millis(300),
            None,
        )
        .await
        .expect_err("both slots are held");
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(error.contains("background agents"), "{error}");
        assert!(error.contains("a task assist"), "{error}");

        let stop = StopSignal::new();
        let stopper = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.request();
        };
        let started = Instant::now();
        let (result, ()) = tokio::join!(
            acquire(
                dir.path(),
                "a delegate",
                Duration::from_secs(30),
                Some(&stop)
            ),
            stopper
        );
        let error = result.expect_err("a stop ends the wait");
        assert!(error.contains("stopped"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a stop is honoured promptly, not after the full wait"
        );
    }

    // ── Pressure gate (Paperclip PR 20) ──

    fn nodes(count: usize) -> PressureSample {
        PressureSample {
            node_procs: Some(count),
            vm_pressure_level: Some(1),
            free_pct: Some(40),
        }
    }

    #[test]
    fn assess_gates_on_node_count_and_memory_pressure() {
        assert_eq!(assess(&nodes(59), 60), None);
        assert_eq!(
            assess(&nodes(60), 60),
            Some(Pressure::NodeProcesses {
                count: 60,
                limit: 60
            })
        );
        for level in [2, 4] {
            let sample = PressureSample {
                node_procs: Some(3),
                vm_pressure_level: Some(level),
                free_pct: Some(12),
            };
            assert_eq!(
                assess(&sample, 60),
                Some(Pressure::MemoryPressure {
                    level,
                    free_pct: Some(12)
                })
            );
        }
        assert_eq!(
            assess(&PressureSample::default(), 60),
            None,
            "unreadable probes never gate"
        );

        let env = |value: Option<&str>| {
            let value = value.map(str::to_string);
            node_limit_from_env(move |key| {
                assert_eq!(key, "CLAUDE_NODE_PROC_LIMIT");
                value.clone()
            })
        };
        assert_eq!(env(Some("80")), 80);
        assert_eq!(env(Some(" 80 ")), 80);
        assert_eq!(env(Some("0")), DEFAULT_NODE_PROC_LIMIT);
        assert_eq!(env(Some("x")), DEFAULT_NODE_PROC_LIMIT);
        assert_eq!(env(None), DEFAULT_NODE_PROC_LIMIT);

        let message = pressure_message(
            "this helper",
            Duration::from_secs(90),
            &Pressure::MemoryPressure {
                level: 2,
                free_pct: Some(12),
            },
        );
        assert_eq!(
            message,
            "Hollow held back this helper for 90s: macOS reports memory pressure (warning, 12% free). Try again when some work finishes."
        );
        assert!(!message.contains('\u{2014}'), "no em dash");
    }

    #[tokio::test]
    async fn a_pressured_permit_waits_then_refuses_with_the_pressure_reason_and_clears_mid_wait() {
        let dir = TempDir::new().unwrap();
        // Free slots, but too many node processes for the whole wait.
        let pressured = || nodes(75);
        let started = Instant::now();
        let error = acquire_with(
            dir.path(),
            "a task assist",
            Duration::from_millis(300),
            None,
            &pressured,
            60,
        )
        .await
        .expect_err("pressure outlasts the wait");
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(error.contains("node processes"), "{error}");
        assert!(error.contains("a task assist"), "{error}");
        assert!(
            try_acquire(dir.path()).unwrap().is_some(),
            "a gated caller never took a slot"
        );

        // Pressured for two samples, then clear: the caller gets its slot.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let clearing = || {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 {
                nodes(75)
            } else {
                nodes(1)
            }
        };
        let permit = acquire_with(
            dir.path(),
            "a delegate",
            Duration::from_secs(10),
            None,
            &clearing,
            60,
        )
        .await
        .expect("the pressure cleared mid-wait");
        assert!(calls.load(std::sync::atomic::Ordering::SeqCst) >= 3);
        drop(permit);

        // A stop during pressure ends the wait with the stopped error.
        let stop = StopSignal::new();
        let stopper = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.request();
        };
        let (result, ()) = tokio::join!(
            acquire_with(
                dir.path(),
                "a delegate",
                Duration::from_secs(30),
                Some(&stop),
                &pressured,
                60
            ),
            stopper
        );
        assert!(result.expect_err("stopped").contains("stopped"));

        // The production entry point samples through the test override,
        // which is clear unless a test sets it on its own thread.
        set_test_pressure(PressureSample {
            node_procs: None,
            vm_pressure_level: Some(4),
            free_pct: Some(3),
        });
        let error = acquire(dir.path(), "a one-shot", Duration::from_millis(250), None)
            .await
            .expect_err("critical memory pressure");
        assert!(
            error.contains("memory pressure (critical, 3% free)"),
            "{error}"
        );
        set_test_pressure(PressureSample::default());
        drop(
            acquire(dir.path(), "a one-shot", Duration::from_millis(250), None)
                .await
                .unwrap(),
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_probe_reads_this_mac() {
        let nodes = crate::proc_info::node_process_count().expect("the process table is readable");
        let _ = nodes;
        let level = crate::proc_info::vm_pressure_level().expect("the pressure level is readable");
        assert!(
            matches!(level, 1 | 2 | 4),
            "unexpected pressure level {level}"
        );
        let free = crate::proc_info::free_memory_pct().expect("the free-memory level is readable");
        assert!(free <= 100);
        assert_eq!(
            crate::proc_info::process_start_micros(std::process::id() as i32),
            crate::proc_info::own_start_micros()
        );
        assert!(crate::proc_info::own_start_micros().is_some());
    }

    #[test]
    fn permit_files_stay_out_of_the_real_app_data_dir() {
        let dir = permits_dir();
        assert!(dir.starts_with(std::env::temp_dir()), "{dir:?}");
        if let Some(real) = dirs::data_dir() {
            assert!(!dir.starts_with(real.join("Hollow")), "{dir:?}");
        }
    }

    /// Helper process for the cross-process test (run by self-exec, never by
    /// the suite): hold one slot in `HOLLOW_PERMIT_HELPER_DIR`, write a marker
    /// file, then sleep until killed. Bounded, so an orphan exits by itself.
    #[test]
    #[ignore = "helper process for the cross-process permit test"]
    fn hold_permit_helper() {
        let Ok(dir) = std::env::var("HOLLOW_PERMIT_HELPER_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let Ok(Some(permit)) = try_acquire(&dir) else {
            return;
        };
        let marker = dir.join(format!("held-{}", std::process::id()));
        let _ = std::fs::write(marker, permit.slot().to_string());
        std::thread::sleep(Duration::from_secs(30));
        drop(permit);
    }

    /// Kills and reaps the helper processes however the test ends.
    #[cfg(unix)]
    struct Helpers(Vec<std::process::Child>);

    #[cfg(unix)]
    impl Drop for Helpers {
        fn drop(&mut self) {
            for child in &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_permit_held_by_another_process_blocks_until_that_process_exits() {
        let dir = TempDir::new().unwrap();
        let exe = std::env::current_exe().expect("the test binary");
        let mut helpers = Helpers(Vec::new());
        for _ in 0..PERMIT_SLOTS {
            let child = std::process::Command::new(&exe)
                .args([
                    "--exact",
                    "run_permit::tests::hold_permit_helper",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env("HOLLOW_PERMIT_HELPER_DIR", dir.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn the helper");
            helpers.0.push(child);
            // One at a time, so the helpers never race each other for a slot.
            let held = helpers.0.len();
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let markers = std::fs::read_dir(dir.path())
                    .map(|entries| {
                        entries
                            .filter_map(Result::ok)
                            .filter(|e| e.file_name().to_string_lossy().starts_with("held-"))
                            .count()
                    })
                    .unwrap_or(0);
                if markers >= held {
                    break;
                }
                assert!(Instant::now() < deadline, "helper {held} never took a slot");
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        assert!(
            try_acquire(dir.path()).unwrap().is_none(),
            "slots held by other processes are unavailable here"
        );
        drop(helpers);
        let permit = try_acquire(dir.path())
            .unwrap()
            .expect("the kernel released the dead processes' locks");
        drop(permit);
    }
}
