//! The crash reaper (A·R8): agent runs end when the Hollow that started them
//! exits, however it exits.
//!
//! A quit and a termination signal settle live runs in-process
//! ([`orphans::settle_at_quit`], [`orphans::settle_on_signal`]). A crash runs
//! no code: release builds abort on panic, and nothing survives a SIGKILL.
//! So at setup the GUI starts `hollow reaper <its pid> <run ledger path>`
//! ([`spawn`]), a child that does nothing until its parent exits and then
//! sweeps that parent's ledger entries ([`orphans::sweep_exited`]), the same
//! identification and SIGTERM → grace → SIGKILL as the boot sweep, which
//! stays as the backstop when the reaper itself is gone.
//!
//! Why this shape:
//! - macOS has no `PR_SET_PDEATHSIG`, and agent CLIs move their tool shells
//!   into new groups and sessions, so nothing can make every descendant die
//!   with Hollow by inheritance. A watcher outside Hollow can end them all
//!   through the ledger, which already identifies them.
//! - It waits with kqueue `EVFILT_PROC`/`NOTE_EXIT` on the parent pid: no
//!   wakeups while Hollow runs, and it fires the moment Hollow exits. The
//!   exit itself is judged by `getppid()` changing (the kernel reparents the
//!   reaper when its parent exits), which cannot be fooled by pid reuse; it
//!   is re-checked every [`RECHECK`] in case kqueue fails. A supervisor pipe
//!   would need its write end kept out of every agent CLI Hollow spawns, or
//!   the read end never sees EOF while those agents live.
//! - The reaper leads its own process group (a Ctrl-C or `killpg` aimed at
//!   Hollow's group does not take it down too), ignores SIGHUP and SIGINT,
//!   carries no run tag (no sweep ever matches it), and once its parent is
//!   gone it can live at most [`AFTER_EXIT_LIMIT_SECS`] (an `alarm`), so it
//!   never holds up an install that waits for Hollow's processes to go.
//!
//! Runs it stops get the queue's ordinary interrupted note at the next
//! launch, not [`orphans::LEFTOVER_STOPPED_NOTE`]: by then nothing of them
//! was still running.

use super::orphans;
use std::path::PathBuf;
#[cfg(unix)]
use std::time::Duration;

/// The hidden subcommand (`main.rs`).
pub(crate) const SUBCOMMAND: &str = "reaper";
/// How often the reaper re-checks its parent without a kqueue wakeup.
#[cfg(unix)]
const RECHECK: Duration = Duration::from_secs(5);
/// How long a reaper may live once its parent is gone, sweep included.
#[cfg(unix)]
const AFTER_EXIT_LIMIT_SECS: u32 = 30;

fn log(message: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr().lock(), "[reaper] {message}");
}

/// GUI setup, after the boot sweep installed the ledger: start this
/// process's reaper. Best effort: without one, the next launch sweeps.
pub(crate) fn spawn() {
    let Some(ledger) = orphans::global() else {
        return;
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            log(&format!("no executable path, so no reaper: {error}"));
            return;
        }
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(SUBCOMMAND)
        .arg(std::process::id().to_string())
        .arg(ledger.path());
    if let Err(error) = launch(cmd) {
        log(&format!("could not start: {error}"));
    }
}

/// Start `cmd` as a reaper: its own process group, no run tag, stdin and
/// stdout closed (stderr is Hollow's, for its log lines). Never waited for
/// and never killed (a dropped std `Child` is neither): it exits on its own
/// once its parent has. Returns its pid.
#[cfg(unix)]
pub(crate) fn launch(mut cmd: std::process::Command) -> Result<u32, String> {
    use std::os::unix::process::CommandExt as _;
    cmd.env_remove(super::RUN_TAG_ENV)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .process_group(0);
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    Ok(child.id())
}

#[cfg(not(unix))]
pub(crate) fn launch(_cmd: std::process::Command) -> Result<u32, String> {
    Err("no reaper on this platform".into())
}

/// `hollow reaper <parent pid> <ledger path>`: wait for `parent` to exit,
/// then end the runs its ledger entries still name.
#[cfg(unix)]
pub fn run(parent: i32, ledger: PathBuf) {
    // SAFETY: setting signal dispositions; nothing else runs yet.
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
    // Only a parent that is provably ours (and so alive) is waited for, and
    // its start stamp read now, while that still holds.
    let ours = parent_pid() == parent;
    let parent_start = if ours {
        crate::proc_info::process_start_micros(parent)
    } else {
        None
    };
    if ours {
        wait_parent_exit(parent);
    }
    // SAFETY: alarm only schedules SIGALRM, whose default action ends this
    // process: a hung sweep cannot keep it around.
    unsafe {
        libc::alarm(AFTER_EXIT_LIMIT_SECS);
    }
    let ledger = orphans::Ledger::new(ledger, orphans::Boot::this_process(chrono::Utc::now()));
    let sweep = tauri::async_runtime::block_on(orphans::sweep_exited(
        &ledger,
        parent,
        parent_start,
        &super::Timings::default(),
    ));
    let pids: usize = sweep.ended.iter().map(|e| e.pids.len()).sum();
    if pids > 0 {
        log(&format!(
            "Hollow pid {parent} exited with {} run(s) live; stopped {pids} process(es)",
            sweep.ended.len()
        ));
    }
}

#[cfg(not(unix))]
pub fn run(_parent: i32, _ledger: PathBuf) {}

#[cfg(unix)]
fn parent_pid() -> i32 {
    // SAFETY: getppid cannot fail.
    unsafe { libc::getppid() }
}

/// Block until `parent` is no longer this process's parent: it exited and
/// the kernel reparented this process.
#[cfg(unix)]
fn wait_parent_exit(parent: i32) {
    #[cfg(target_os = "macos")]
    let mut watch = ExitWatch::new(parent);
    while parent_pid() == parent {
        #[cfg(target_os = "macos")]
        if let Some(kq) = &watch {
            if !kq.wait(RECHECK) {
                watch = None;
            }
            continue;
        }
        std::thread::sleep(RECHECK);
    }
}

/// A kqueue watching one pid for `NOTE_EXIT`.
#[cfg(target_os = "macos")]
struct ExitWatch(libc::c_int);

#[cfg(target_os = "macos")]
impl ExitWatch {
    fn new(pid: i32) -> Option<Self> {
        // SAFETY: kqueue returns a new descriptor or -1.
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return None;
        }
        let watch = Self(kq);
        let change = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: one valid change, no event list, no timeout.
        let rc = unsafe {
            libc::kevent(
                watch.0,
                &change,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        };
        (rc == 0).then_some(watch)
    }

    /// Wait up to `limit` for the exit (or any wakeup). False when kqueue
    /// itself fails, so the caller stops using it.
    fn wait(&self, limit: Duration) -> bool {
        // SAFETY: an all-zero kevent is a valid value to be overwritten.
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        let timeout = libc::timespec {
            tv_sec: limit.as_secs() as libc::time_t,
            tv_nsec: 0,
        };
        // SAFETY: no changes; room for one event; a valid timeout.
        let rc = unsafe { libc::kevent(self.0, std::ptr::null(), 0, &mut event, 1, &timeout) };
        rc >= 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
    }
}

#[cfg(target_os = "macos")]
impl Drop for ExitWatch {
    fn drop(&mut self) {
        // SAFETY: closing the descriptor this value owns.
        unsafe {
            libc::close(self.0);
        }
    }
}
