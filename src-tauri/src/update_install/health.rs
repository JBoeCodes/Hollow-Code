//! Is the app closed, and did the new one start?
//!
//! **Closed:** no process runs the installed (or previous) bundle's
//! executable itself (`ps` args equal to the executable, or it followed by a
//! `-flag`; a subcommand child and this installer don't count). The helper
//! is started as Hollow quits, so it waits up to the quit timeout first,
//! like the `.mjs`'s `waitUntilQuit`.
//!
//! **Started (decided for U4):** after `open -n`, an app process of the
//! installed executable appears within the start timeout and is still the
//! same live process when the settle time has passed. The crashes this
//! guards against (2026-11-11: an abort 0.4 s after launch) end well inside
//! the settle time. No marker file: the apps in the field write none yet,
//! and a check that needs the new app's cooperation would roll back every
//! update from an app that predates it. A hung but living app counts as
//! started; the person can still go back from Settings (U6). When the
//! watched process exits, another process of the executable started since
//! the launch (a single-instance handoff) is watched instead.
//!
//! **After a failed try** only the processes this installer's launch
//! started are sent TERM, and only while each pid still has the start stamp
//! it had ([`stop`]); any other process of the executable is reported and
//! left alone, and the installer then stops without going back.
//!
//! All waiting goes through the injected clock, so tests never sleep.

use super::effects::{AppProc, Deps};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) quit_ms: u64,
    pub(crate) appear_ms: u64,
    pub(crate) settle_ms: u64,
    pub(crate) poll_ms: u64,
    /// After the watched process exits, how long to look for a successor
    /// (a single-instance handoff) before calling the start failed.
    pub(crate) handoff_ms: u64,
}

impl Timing {
    /// The longest one attempt may watch: the start timeout plus three
    /// settle times, so an app that keeps respawning can't hold the
    /// installer (and its lock) forever.
    pub(crate) fn watch_cap_ms(&self) -> u64 {
        self.appear_ms
            .saturating_add(self.settle_ms.saturating_mul(3))
    }
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            quit_ms: 60_000,
            appear_ms: 60_000,
            settle_ms: 20_000,
            poll_ms: 500,
            handoff_ms: 1_000,
        }
    }
}

/// Every app process of the given executables.
pub(crate) fn running(exes: &[PathBuf], deps: &Deps) -> Result<Vec<AppProc>, String> {
    let mut all = Vec::new();
    for exe in exes {
        all.extend(deps.procs.app_processes(exe)?);
    }
    Ok(all)
}

fn pids(list: &[AppProc]) -> String {
    list.iter()
        .map(|p| p.pid.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Wait up to `timing.quit_ms` for every process of `exes` to end.
pub(crate) fn wait_until_quit(
    exes: &[PathBuf],
    deps: &Deps,
    timing: &Timing,
) -> Result<(), String> {
    let deadline = deps.clock.now_ms() + timing.quit_ms;
    loop {
        let seen = running(exes, deps)
            .map_err(|e| format!("could not check that Hollow is closed: {e}"))?;
        if seen.is_empty() {
            return Ok(());
        }
        if deps.clock.now_ms() >= deadline {
            return Err(format!(
                "Hollow is still running (pid {}) after {} s, so nothing was swapped; quit it and try again",
                pids(&seen),
                timing.quit_ms / 1000
            ));
        }
        deps.clock.sleep(Duration::from_millis(timing.poll_ms));
    }
}

/// A process this installer's launch started: its pid and start stamp,
/// so a TERM never reaches a reused pid or a Hollow it did not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ours {
    pub(crate) pid: i32,
    pub(crate) started: Option<u64>,
}

/// TERM only the processes this attempt started (`ours`, while each pid
/// still has the start stamp it had), wait for them to end, then refuse if
/// any other process still runs `exe`: that one is reported, never killed.
///
/// `launched_ms` is when this attempt opened the app: a process that
/// started after it (one that appeared after the start timeout) is named as
/// a late start of this launch, not as another Hollow.
pub(crate) fn stop(
    exe: &Path,
    ours: &[Ours],
    launched_ms: u64,
    deps: &Deps,
    timing: &Timing,
) -> Result<(), String> {
    let still_ours = |o: &Ours| {
        deps.procs.alive(o.pid) != Some(false)
            && o.started.is_some()
            && deps.procs.started_micros(o.pid) == o.started
    };
    let termed: Vec<Ours> = ours.iter().copied().filter(|o| still_ours(o)).collect();
    for o in &termed {
        deps.procs.terminate(o.pid);
    }
    let deadline = deps.clock.now_ms() + timing.quit_ms;
    while termed.iter().any(still_ours) {
        if deps.clock.now_ms() >= deadline {
            return Err(format!(
                "the new app (pid {}) did not quit within {} s of a TERM",
                ours.iter()
                    .filter(|o| still_ours(o))
                    .map(|o| o.pid.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                timing.quit_ms / 1000
            ));
        }
        deps.clock.sleep(Duration::from_millis(timing.poll_ms));
    }
    let others: Vec<AppProc> = running(&[exe.to_path_buf()], deps)?
        .into_iter()
        .filter(|p| deps.procs.alive(p.pid) != Some(false))
        .filter(|p| !termed.iter().any(|o| o.pid == p.pid))
        .collect();
    let floor = launched_ms.saturating_sub(2_000);
    let (late, foreign): (Vec<AppProc>, Vec<AppProc>) = others
        .into_iter()
        .partition(|p| p.started_ms != 0 && p.started_ms >= floor);
    let mut problems = Vec::new();
    if !late.is_empty() {
        problems.push(format!(
            "a Hollow process (pid {}) started after the reopen check gave up on it; it was left running",
            pids(&late)
        ));
    }
    if !foreign.is_empty() {
        problems.push(format!(
            "another Hollow (pid {}) that was running before this launch was left alone",
            pids(&foreign)
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// One reopen attempt's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Start {
    Healthy {
        pid: i32,
    },
    LaunchFailed(String),
    NeverAppeared,
    Exited {
        pid: i32,
        after_ms: u64,
    },
    /// Processes kept exiting and handing off past the watch's total cap.
    NeverSettled,
}

impl Start {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Start::Healthy { .. } => "healthy",
            Start::LaunchFailed(_) => "launch-failed",
            Start::NeverAppeared => "never-appeared",
            Start::Exited { .. } => "exited",
            Start::NeverSettled => "never-settled",
        }
    }

    pub(crate) fn pid(&self) -> Option<i32> {
        match self {
            Start::Healthy { pid } | Start::Exited { pid, .. } => Some(*pid),
            _ => None,
        }
    }

    pub(crate) fn detail(&self, timing: &Timing) -> String {
        match self {
            Start::Healthy { pid } => format!(
                "pid {pid} was still running {} s after it started",
                timing.settle_ms / 1000
            ),
            Start::LaunchFailed(why) => format!("could not open the app: {why}"),
            Start::NeverSettled => format!(
                "no Hollow process stayed up {} s within {} s (each one exited and another started)",
                timing.settle_ms / 1000,
                timing.watch_cap_ms() / 1000
            ),
            Start::NeverAppeared => format!(
                "no Hollow process appeared within {} s",
                timing.appear_ms / 1000
            ),
            Start::Exited { pid, after_ms } => {
                format!(
                    "pid {pid} exited {:.1} s after it started",
                    *after_ms as f64 / 1000.0
                )
            }
        }
    }
}

/// One attempt: what happened and every process it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Attempted {
    pub(crate) start: Start,
    pub(crate) ours: Vec<Ours>,
    /// When this attempt opened the app (the injected clock).
    pub(crate) launched_ms: u64,
}

/// A live process of `exe` started since `floor`, other than `skip`.
fn fresh(exe: &Path, floor: u64, skip: &[i32], deps: &Deps) -> Option<AppProc> {
    deps.procs
        .app_processes(exe)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.started_ms == 0 || p.started_ms >= floor)
        .filter(|p| !skip.contains(&p.pid))
        .find(|p| deps.procs.alive(p.pid) != Some(false))
}

/// Open `app` and watch `exe` start (module doc for the rule). When the
/// process it watches exits, another one started since the launch (a
/// single-instance handoff) takes its place for the rest of the settle time.
pub(crate) fn attempt(app: &Path, exe: &Path, deps: &Deps, timing: &Timing) -> Attempted {
    let launched = deps.clock.now_ms();
    let mut ours: Vec<Ours> = Vec::new();
    let done = |start: Start, ours: Vec<Ours>| Attempted {
        start,
        ours,
        launched_ms: launched,
    };
    if let Err(why) = deps.launcher.launch(app) {
        return done(Start::LaunchFailed(why), ours);
    }
    // `ps` start stamps and our clock may differ by part of a second.
    let floor = launched.saturating_sub(2_000);
    let deadline = launched + timing.appear_ms;
    let adopt = |p: AppProc, ours: &mut Vec<Ours>| {
        ours.push(Ours {
            pid: p.pid,
            started: deps.procs.started_micros(p.pid),
        });
        p.pid
    };
    let first = loop {
        // The first sighting may already be dead (a crash at launch).
        let seen = deps
            .procs
            .app_processes(exe)
            .unwrap_or_default()
            .into_iter()
            .find(|p| p.started_ms == 0 || p.started_ms >= floor);
        if let Some(p) = seen {
            break p;
        }
        if deps.clock.now_ms() >= deadline {
            return done(Start::NeverAppeared, ours);
        }
        deps.clock.sleep(Duration::from_millis(timing.poll_ms));
    };
    let cap = launched.saturating_add(timing.watch_cap_ms());
    let mut pid = adopt(first, &mut ours);
    // Each adopted process gets the whole settle time.
    let mut settle_end = deps.clock.now_ms() + timing.settle_ms;
    loop {
        if deps.procs.alive(pid) == Some(false) {
            let gone: Vec<i32> = ours.iter().map(|o| o.pid).collect();
            let look_until = deps.clock.now_ms() + timing.handoff_ms;
            let next = loop {
                if deps.clock.now_ms() >= cap {
                    return done(Start::NeverSettled, ours);
                }
                if let Some(next) = fresh(exe, floor, &gone, deps) {
                    break Some(next);
                }
                if deps.clock.now_ms() >= look_until {
                    break None;
                }
                deps.clock.sleep(Duration::from_millis(timing.poll_ms));
            };
            match next {
                Some(next) => {
                    pid = adopt(next, &mut ours);
                    settle_end = deps.clock.now_ms() + timing.settle_ms;
                    continue;
                }
                None => {
                    let after_ms = deps.clock.now_ms().saturating_sub(launched);
                    return done(Start::Exited { pid, after_ms }, ours);
                }
            }
        }
        if deps.clock.now_ms() >= settle_end {
            return done(Start::Healthy { pid }, ours);
        }
        if deps.clock.now_ms() >= cap {
            return done(Start::NeverSettled, ours);
        }
        deps.clock.sleep(Duration::from_millis(timing.poll_ms));
    }
}
