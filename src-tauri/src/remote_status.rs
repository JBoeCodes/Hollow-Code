//! The remote runner's state (overnight N1 and N2, 2026-11-02): Settings,
//! Devices shows it under Runs on with Check now, and agents read it
//! with `build.runner_status`. One ssh call runs [`RUNNER_STATUS_PL`] there:
//! the machine's name, its `hollow-run-preflight` line (free disk, memory,
//! toolchain versions), the remote sessions running there now and its newest
//! gate run record. The last answer stays in memory, so Settings shows it at
//! once and asks again only when it is old or the person presses Check now.
//!
//! The runner runs branch code, so everything it prints is read leniently
//! (a field of the wrong type is dropped, not the whole answer) and cut to
//! [`FIELD_MAX`] characters before it reaches Settings or an agent.
//!
//! [`likely_causes`] turns ssh's error into what to check when the runner
//! doesn't answer; a turn's refusal (`remote_session::unreachable_message`)
//! and the status share it, and `hollow-slot --remote` says the same in
//! shell (`r_why` in `build_slots::SLOT_TOOL`).

use crate::remote_session::{configured_runner, valid_host, SSH_OPTS};
use serde::Serialize;
use serde_json::Value;
use std::io::Read;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// How long one check may take in all (ssh's own connect timeout is 10 s;
/// the preflight there gets 15 s).
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// The longest text taken from the runner for one field.
const FIELD_MAX: usize = 300;

/// What the runner said when it was last checked.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunnerStatus {
    /// The SSH host Runs on's Mac mini uses; `None` when none is set up.
    pub host: Option<String>,
    /// `none` (no runner set up), `unchecked`, `ready`, `refusing` (its
    /// preflight said no or didn't finish), `unreachable` (ssh couldn't reach
    /// it, or the check stalled) or `error` (it answered, but the check
    /// failed).
    pub state: &'static str,
    /// When it was checked (RFC 3339), and how long the check took.
    pub checked_at: Option<String>,
    pub millis: Option<u64>,
    /// Why it isn't ready, with what to check.
    pub message: Option<String>,
    /// The machine's own name there (`hostname -s`).
    pub machine: Option<String>,
    /// Its system, `darwin` or `linux` (perl's `$^O` there; a Linux runner,
    /// 2026-11-03). `None` from a runner whose probe predates it.
    pub os: Option<String>,
    /// Its `hollow-run-preflight` status line, `None` when it has none.
    pub preflight: Option<String>,
    /// Remote sessions whose supervisor is running there now.
    pub running: Option<u64>,
    /// Whether a gate (`node scripts/gate-run.mjs`) is running there now.
    pub gate_running: bool,
    /// Its newest gate run record (`hollow-gate-runs/`), from any repository.
    pub last_gate: Option<RunnerGate>,
    /// Who it belongs to and who else can log in there (remote runners 2,
    /// `runner_owner::judge`); `None` until it answered a check.
    pub owner: Option<crate::runner_owner::RunnerOwner>,
    /// Smart routing (2026-11-03): `hollow-slot`'s latest decisions and the
    /// runner's tools, read fresh from this Mac's slot folder for each
    /// answer (`commands::build_api::runner_status`); never cached here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<crate::build_slots::RouteStatus>,
    /// When the check finished, on both clocks (for [`status`]'s reuse and
    /// [`recently_unavailable_state`]).
    #[serde(skip)]
    finished: Option<Stamp>,
    /// A failure an ssh call found ([`note_failure`]), not a check: Check
    /// now never takes it for its own answer. Like a check's, an
    /// unreachable one goes when an ssh call gets through
    /// ([`note_answered`]) and a refusing one when a preparation passes the
    /// preflight ([`note_prepared`]).
    #[serde(skip)]
    noted: bool,
}

/// When something finished, on both clocks. macOS's `Instant`
/// (`mach_absolute_time`) stops while the Mac sleeps, so on its own a
/// failure found just before the lid closed would still look a moment old
/// long after wake; the wall clock keeps counting through sleep but can be
/// set back.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stamp {
    mono: Instant,
    wall: SystemTime,
}

impl Stamp {
    fn now() -> Stamp {
        Stamp {
            mono: Instant::now(),
            wall: SystemTime::now(),
        }
    }

    /// How long ago this was at `now` by the wall clock, when both clocks
    /// say at most `within`. `None` when either says longer (the Mac slept
    /// through it, or was awake that long), and when the wall clock now
    /// reads earlier than it did (set back, so how long is unknown: such a
    /// failure isn't taken as recent, and the next turn asks the runner
    /// itself).
    fn age_within(&self, now: Stamp, within: Duration) -> Option<Duration> {
        let wall = now.wall.duration_since(self.wall).ok()?;
        let mono = now.mono.saturating_duration_since(self.mono);
        (wall <= within && mono <= within).then_some(wall)
    }
}

/// One gate run record on the runner (`scripts/gate-run.mjs`), as much of it
/// as the status shows.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunnerGate {
    pub start: String,
    /// `None` while it runs, or when it was killed before it could finish.
    pub end: Option<String>,
    pub seconds: Option<f64>,
    pub ok: Option<bool>,
    pub exit_code: Option<i64>,
    pub failed_step: Option<String>,
    pub commit: Option<String>,
    /// The checkout it ran for, as the runner names its folder for it (the
    /// worktree's folder name here, `ri27-mm5`).
    pub checkout: Option<String>,
}

impl RunnerStatus {
    fn new(host: Option<&str>, state: &'static str) -> RunnerStatus {
        RunnerStatus {
            host: host.map(str::to_string),
            state,
            checked_at: None,
            millis: None,
            message: None,
            machine: None,
            os: None,
            preflight: None,
            running: None,
            gate_running: false,
            last_gate: None,
            owner: None,
            routing: None,
            finished: None,
            noted: false,
        }
    }
}

/// `text` on one line, at most [`FIELD_MAX`] characters.
pub(crate) fn clip(text: &str) -> String {
    let one: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one = one.trim();
    if one.chars().count() <= FIELD_MAX {
        one.to_string()
    } else {
        let mut cut: String = one.chars().take(FIELD_MAX).collect();
        cut.push('…');
        cut
    }
}

/// A string field, clipped; `None` when absent, empty or not a string.
fn text_field(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(clip)
        .filter(|s| !s.is_empty())
}

/// The newest gate record as the probe printed it; `None` unless it has a
/// start time. Fields of the wrong type are dropped one by one.
fn gate_from(v: &Value) -> Option<RunnerGate> {
    let start = text_field(v, "start")?;
    Some(RunnerGate {
        start,
        end: text_field(v, "end"),
        seconds: v.get("seconds").and_then(Value::as_f64),
        ok: v.get("ok").and_then(Value::as_bool),
        exit_code: v.get("exitCode").and_then(Value::as_i64),
        failed_step: text_field(v, "failedStep"),
        commit: text_field(v, "commit"),
        checkout: text_field(v, "checkout"),
    })
}

/// The runner's last status, and what happened since that a check begun
/// earlier must not overwrite (lane S10, 2026-11-03: a check takes up to
/// [`CHECK_TIMEOUT`], so its "refusing" could land after a preparation
/// passed meanwhile).
struct Slot {
    status: Option<RunnerStatus>,
    /// When `status` was last written or cleared.
    changed: Option<Instant>,
    /// The host an ssh call last got through to, and when.
    answered: Option<(String, Instant)>,
    /// The host a preparation last passed the preflight of, and when.
    prepared: Option<(String, Instant)>,
}

impl Slot {
    fn set(&mut self, status: Option<RunnerStatus>) {
        self.status = status;
        self.changed = Some(Instant::now());
    }

    /// Whether a check of `host` begun at `began` that found `state` may
    /// become the last status: not when the status changed since, nor when
    /// a call since disproved it (any call that got through an unreachable
    /// one, a passing preparation a refusing one).
    fn takes_check(&self, host: &str, state: &str, began: Instant) -> bool {
        let since = |mark: &Option<(String, Instant)>| {
            mark.as_ref()
                .is_some_and(|(h, at)| h == host && *at > began)
        };
        self.changed.is_none_or(|at| at <= began)
            && match state {
                "unreachable" => !since(&self.answered) && !since(&self.prepared),
                "refusing" => !since(&self.prepared),
                _ => true,
            }
    }
}

static LAST: Mutex<Slot> = Mutex::new(Slot {
    status: None,
    changed: None,
    answered: None,
    prepared: None,
});
static CHECKING: Mutex<()> = Mutex::new(());

/// The runner's state: the last check's answer, or with `fresh` a new check
/// (blocking, at most [`CHECK_TIMEOUT`]; call it off the async runtime).
/// Checks never run two at once: one asked while another runs waits for it
/// and gets its answer (Settings reopened mid-check doesn't log in again);
/// one asked after it finished checks again, so Check now always asks. Its
/// answer is returned, but kept as the last status only when nothing newer
/// came meanwhile ([`Slot::takes_check`]).
pub(crate) fn status(fresh: bool) -> RunnerStatus {
    let Some(host) = configured_runner() else {
        return RunnerStatus::new(None, "none");
    };
    let last = || {
        LAST.lock()
            .ok()
            .and_then(|last| last.status.clone())
            .filter(|s| s.host.as_deref() == Some(host.as_str()))
    };
    if !fresh {
        return last().unwrap_or_else(|| RunnerStatus::new(Some(&host), "unchecked"));
    }
    let asked = Instant::now();
    let _one = CHECKING.lock().unwrap_or_else(|e| e.into_inner());
    // Only a check's answer: a failure an ssh call noted meanwhile isn't one.
    let since = |s: &RunnerStatus| !s.noted && s.finished.is_some_and(|at| at.mono >= asked);
    if let Some(done) = last().filter(since) {
        return done;
    }
    let began = Instant::now();
    let checked = check("ssh", &host);
    if let Ok(mut slot) = LAST.lock() {
        if slot.takes_check(&host, checked.state, began) {
            slot.set(Some(checked.clone()));
        }
    }
    checked
}

/// Why `host` can't take a turn, when its last check (at most `within`
/// old) said so: it didn't answer, or its preflight refused (overnight N6:
/// unattended work then runs here at once instead of waiting on ssh's
/// connect timeout). `None` when it was ready, never checked, or the check
/// is older by either clock ([`Stamp::age_within`]). With the status's
/// state (`unreachable`, a stalled check included, or `refusing`), so a
/// route (RT1) takes its cause from the status, not from the sentence.
pub(crate) fn recently_unavailable_state(
    host: &str,
    within: Duration,
) -> Option<(&'static str, String)> {
    let last = LAST.lock().ok()?.status.clone()?;
    unavailable_state_in(&last, host, within, Stamp::now())
}

/// [`unavailable_in`] with the status's state.
fn unavailable_state_in(
    last: &RunnerStatus,
    host: &str,
    within: Duration,
    now: Stamp,
) -> Option<(&'static str, String)> {
    unavailable_in(last, host, within, now).map(|why| (last.state, why))
}

/// RT1's test: what [`recently_unavailable_state`] says for a status of
/// `state` with `message` or `preflight`, checked just now.
#[cfg(test)]
pub(crate) fn recent_state_for_test(
    host: &str,
    state: &'static str,
    message: Option<String>,
    preflight: Option<&str>,
) -> Option<(&'static str, String)> {
    let mut status = RunnerStatus::new(Some(host), state);
    status.message = message;
    status.preflight = preflight.map(str::to_string);
    let now = Stamp::now();
    status.finished = Some(now);
    unavailable_state_in(&status, host, Duration::from_secs(300), now)
}

/// A check that didn't finish in `secs` (a stalled connection, or a check
/// stuck there): the runner counts as unreachable.
pub(crate) fn stalled_message(host: &str, secs: u64) -> String {
    format!(
        "{host} didn't finish the check in {secs} s (a stalled connection, or a check stuck there). {}",
        likely_causes("")
    )
}

/// Whether the runner's last status, at most `within` old, was unreachable
/// (not a refusal: it answered). Overnight R1's turns that aren't going
/// there skip asking it about an earlier turn then.
pub(crate) fn recently_unreachable(host: &str, within: Duration) -> bool {
    LAST.lock()
        .ok()
        .and_then(|l| l.status.clone())
        .is_some_and(|last| {
            last.state == "unreachable"
                && unavailable_in(&last, host, within, Stamp::now()).is_some()
        })
}

/// [`recently_unavailable_state`] at `now`; "N minutes ago" is the wall clock's.
fn unavailable_in(last: &RunnerStatus, host: &str, within: Duration, now: Stamp) -> Option<String> {
    if last.host.as_deref() != Some(host) {
        return None;
    }
    let ago = last.finished?.age_within(now, within)?.as_secs() / 60;
    let when = match ago {
        0 => "under a minute ago".to_string(),
        1 => "a minute ago".to_string(),
        n => format!("{n} minutes ago"),
    };
    match last.state {
        "unreachable" => Some(format!(
            "When checked {when}, {}",
            last.message.as_deref().unwrap_or("it didn't answer.")
        )),
        "refusing" => Some(format!(
            "When checked {when}, it wasn't taking work ({}).",
            last.preflight
                .as_deref()
                .unwrap_or("its preflight didn't finish")
        )),
        _ => None,
    }
}

/// What a failed ssh call to `host` found (overnight N6): `unreachable`
/// with ssh's error, or `refusing` with the preflight's line, kept as the
/// runner's last status, as a check finding the same would.
pub(crate) fn note_failure(host: &str, state: &'static str, said: &str) {
    if let Ok(mut slot) = LAST.lock() {
        let status = failure_status(slot.status.as_ref(), host, state, said);
        slot.set(Some(status));
    }
}

/// An ssh call to `host` got through: an unreachable status, noted or a
/// check's, no longer holds, so it goes (the status reads as unchecked
/// until the next check, which Settings then makes when it opens). A
/// refusing one stays: an unrelated call getting through says nothing about
/// the preflight ([`note_prepared`] clears that). `began` is when the call
/// started: a status found after that isn't the call's to end.
pub(crate) fn note_answered(host: &str, began: Instant) {
    clear_ended(host, false, began);
}

/// A preparation of `host` passed its preflight (`remote_session`'s host
/// preparation got through): a refusing status, noted or a check's, no
/// longer holds either, so it goes as an unreachable one does.
pub(crate) fn note_prepared(host: &str, began: Instant) {
    clear_ended(host, true, began);
}

fn clear_ended(host: &str, prepared: bool, began: Instant) {
    if let Ok(mut slot) = LAST.lock() {
        let mark = Some((host.to_string(), Instant::now()));
        if prepared {
            slot.prepared = mark.clone();
        }
        slot.answered = mark;
        if slot
            .status
            .as_ref()
            .is_some_and(|s| ended_by(s, host, prepared, began))
        {
            slot.set(None);
        }
    }
}

/// Whether an ssh call to `host` that got through (`prepared`: a host
/// preparation, so its preflight passed) ends what `last` said: an
/// unreachable status ends with any, a refusing one only with a
/// preparation, anything else (ready, a failed check) with neither; and
/// only one found by the time the call began (`began`), not one found while
/// it ran.
fn ended_by(last: &RunnerStatus, host: &str, prepared: bool, began: Instant) -> bool {
    last.host.as_deref() == Some(host)
        && last.finished.is_none_or(|at| at.mono <= began)
        && match last.state {
            "unreachable" => true,
            "refusing" => prepared,
            _ => false,
        }
}

/// The status a noted failure leaves: the state and message for what ssh
/// said, with what the last status of the same host knew about the machine,
/// its sessions and its gates (still the newest known).
fn failure_status(
    previous: Option<&RunnerStatus>,
    host: &str,
    state: &'static str,
    said: &str,
) -> RunnerStatus {
    let mut status = RunnerStatus::new(Some(host), state);
    status.noted = true;
    if let Some(before) = previous.filter(|p| p.host.as_deref() == Some(host)) {
        status.machine = before.machine.clone();
        status.os = before.os.clone();
        status.running = before.running;
        status.last_gate = before.last_gate.clone();
        status.preflight = before.preflight.clone();
        status.owner = before.owner.clone();
    }
    status.checked_at = Some(chrono::Utc::now().to_rfc3339());
    let said = clip(said.lines().last().unwrap_or(""));
    let said = said.trim_end_matches('.');
    match state {
        "unreachable" => {
            status.message = Some(format!(
                "{host} didn't answer{}. {}",
                if said.is_empty() {
                    String::new()
                } else {
                    format!(": {said}")
                },
                likely_causes(said)
            ));
        }
        _ => {
            status.preflight = (!said.is_empty()).then(|| said.to_string());
            status.message = Some(format!(
                "{host} isn't taking work now ({}).",
                if said.is_empty() {
                    "its preflight said no"
                } else {
                    said
                }
            ));
        }
    }
    status.finished = Some(Stamp::now());
    status
}

/// Ask `host` through `ssh` how it is. Never fails: what went wrong is the
/// answer's `state` and `message`.
pub(crate) fn check(ssh: &str, host: &str) -> RunnerStatus {
    check_within(ssh, host, CHECK_TIMEOUT)
}

fn check_within(ssh: &str, host: &str, limit: Duration) -> RunnerStatus {
    let started = Instant::now();
    let mut status = RunnerStatus::new(Some(host), "error");
    status.checked_at = Some(chrono::Utc::now().to_rfc3339());
    let done = |mut status: RunnerStatus| {
        status.millis = Some(started.elapsed().as_millis() as u64);
        status.finished = Some(Stamp::now());
        status
    };
    if !valid_host(host) {
        status.message = Some(format!("{host} is not an SSH host name."));
        return done(status);
    }
    // Exit 255 must mean ssh itself failed (as `remote_session::ssh_call`).
    let command = format!(
        "/usr/bin/perl -e {}; s=$?; [ \"$s\" -ne 255 ] || s=3; exit $s",
        crate::remote_session::shell_quote(RUNNER_STATUS_PL)
    );
    let (code, stdout, stderr) = match run_bounded(ssh, host, &command, limit) {
        Ok(out) => out,
        Err(Bounded::Start(why)) => {
            status.message = Some(why);
            return done(status);
        }
        Err(Bounded::Stalled) => {
            status.state = "unreachable";
            status.message = Some(stalled_message(host, limit.as_secs()));
            return done(status);
        }
    };
    let last_err = clip(stderr.lines().last().unwrap_or(""));
    let last_err = last_err.trim_end_matches('.');
    match code {
        Some(255) => {
            status.state = "unreachable";
            status.message = Some(format!(
                "{host} didn't answer{}. {}",
                if last_err.is_empty() {
                    String::new()
                } else {
                    format!(": {last_err}")
                },
                likely_causes(last_err)
            ));
        }
        Some(0) => {
            let probe = stdout
                .lines()
                .rev()
                .find(|line| line.trim_start().starts_with('{'))
                .and_then(|line| serde_json::from_str::<Value>(line).ok())
                .filter(Value::is_object);
            let Some(probe) = probe else {
                status.message = Some(format!("{host} answered, but not with its state."));
                return done(status);
            };
            status.machine = text_field(&probe, "machine");
            status.os = text_field(&probe, "os");
            status.preflight = text_field(&probe, "preflight");
            status.running = probe.get("running").and_then(Value::as_u64);
            status.gate_running = probe
                .get("gateRunning")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            status.last_gate = probe.get("lastGate").and_then(gate_from);
            status.owner = Some(crate::runner_owner::judge(
                &probe,
                &crate::runner_owner::Mine::for_host(ssh, host),
            ));
            if probe.get("preflightStalled").and_then(Value::as_bool) == Some(true) {
                status.state = "refusing";
                status.message = Some(format!(
                    "{host} answered, but its hollow-run-preflight didn't finish in 15 s. Turns set to run there and hollow-slot calls sent there wait on it too; check it there."
                ));
            } else if probe.get("preflightOk").and_then(Value::as_bool) == Some(false) {
                status.state = "refusing";
                status.message = Some(format!(
                    "{host} isn't taking work now ({}). Turns set to run there stop with this reason; hollow-slot runs build and test commands here instead.",
                    status.preflight.as_deref().unwrap_or("its preflight said no")
                ));
            } else {
                status.state = "ready";
            }
        }
        _ => {
            status.message = Some(format!(
                "{host} answered, but its check failed ({}).",
                if last_err.is_empty() {
                    "no message"
                } else {
                    last_err
                }
            ));
        }
    }
    done(status)
}

/// Why [`run_bounded`] has no answer.
#[derive(Debug)]
pub(crate) enum Bounded {
    /// ssh couldn't start.
    Start(String),
    /// Nothing finished within the limit (ssh was killed).
    Stalled,
}

/// `ssh <SSH_OPTS> host command` with stdin closed, killed after `limit`:
/// (exit code, stdout, stderr). The child stays here, polled with
/// `try_wait`, so a kill never reaches a reaped (reusable) pid.
pub(crate) fn run_bounded(
    ssh: &str,
    host: &str,
    command: &str,
    limit: Duration,
) -> Result<(Option<i32>, String, String), Bounded> {
    let started = Instant::now();
    let mut child = std::process::Command::new(ssh)
        .args(SSH_OPTS)
        .args([host, command])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| Bounded::Start(format!("ssh couldn't start ({e}).")))?;
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                // At most 1 MB: the answer is one short line.
                let _ = pipe.by_ref().take(1 << 20).read_to_end(&mut bytes);
            }
            let _ = tx.send(bytes);
        });
        rx
    };
    let out = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Bounded::Stalled);
            }
        }
    };
    // ssh has exited; its pipes close with it unless something it started
    // here still holds them.
    let grace = Duration::from_secs(2);
    let stdout = out.recv_timeout(grace).unwrap_or_default();
    let stderr = err.recv_timeout(grace).unwrap_or_default();
    Ok((
        status.code(),
        String::from_utf8_lossy(&stdout).into_owned(),
        String::from_utf8_lossy(&stderr).into_owned(),
    ))
}

/// What to check when ssh couldn't reach the runner, from its error line
/// (overnight N2). `r_why` in `build_slots::SLOT_TOOL` matches the same
/// errors with shorter advice.
pub(crate) fn likely_causes(ssh_error: &str) -> &'static str {
    let e = ssh_error.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| e.contains(n));
    if has(&[
        "could not resolve hostname",
        "nodename nor servname",
        "name or service not known",
    ]) {
        "This Mac can't look up its address: if it is on a tailnet, check that Tailscale is running and connected on this Mac, on the tailnet the runner is on."
    } else if has(&["connection refused"]) {
        "It is on but not taking SSH connections: Remote Login is off there, or it is still starting up."
    } else if has(&["permission denied", "too many authentication failures"]) {
        "It answered but refused this Mac's key: check the IdentityFile for it in ~/.ssh/config and that key in the runner account's ~/.ssh/authorized_keys."
    } else if has(&["host key verification failed", "identification has changed"]) {
        "Its host key isn't the one this Mac knows. If it was reinstalled, confirm the new key and update ~/.ssh/known_hosts; nothing connects until then."
    } else if has(&[
        "connection closed",
        "connection reset",
        "kex_exchange_identification",
        "broken pipe",
    ]) {
        "It dropped the connection while logging in. After several failed logins its SSH server refuses for a while; wait a few minutes and check again."
    } else {
        "It may be off, asleep or restarting, or the network between the two is down. After a restart it waits at the FileVault unlock screen until someone signs in on it; nothing runs there until then."
    }
}

/// Run on the runner by ssh: `/usr/bin/perl -e` this, no arguments, no
/// stdin. Prints one JSON line (ASCII): the machine's name, its system
/// (`os`: `darwin` or `linux`; on Linux the process lists come from `/proc`),
/// its preflight line and whether it passed (found as the host preparation finds it; given
/// 15 s, then its process group is killed and `preflightStalled` is set), the
/// remote sessions whose supervisor is alive in `~/hollow-remote/run`,
/// whether a gate is running (a `node … scripts/gate-run.mjs` process of this
/// account), and the newest gate run record in any repository in
/// `~/hollow-remote/repos` (record names start with their start time), its
/// owner record (`~/hollow-remote/owner.json`: installId, key, at) and the
/// SHA256 fingerprints of the keys its account lets in (`keys`: each once,
/// at most 50, from `ssh-keygen -l` on `~/.ssh/authorized_keys` and
/// `authorized_keys2`, `/usr/bin/ssh-keygen` first; `keyCount` how many
/// unique, `keyLines` the key lines those files hold, `keysRead` the
/// fingerprints read; all absent when a file can't be read or ssh-keygen is
/// missing). Only regular files are read, owner.json only up to 4 KB. Every text is cut to 300
/// characters there too. Changes nothing there.
pub(crate) const RUNNER_STATUS_PL: &str = r#"
use strict; use warnings; use JSON::PP; use POSIX ();
my $home = $ENV{HOME} // ''; my $root = "$home/hollow-remote";
# Text for the answer: control characters as spaces, trimmed, 300 at most.
# $bytes first decodes what a command printed (UTF-8 when it is).
my $cut = sub { my $t = shift // ''; $t =~ s/[\x00-\x1f\x7f]+/ /g; $t =~ s/\A\s+|\s+\z//g; return length($t) > 300 ? substr($t, 0, 300) : $t };
my $bytes = sub { my $t = shift // ''; utf8::decode($t); return $cut->($t) };
my %o;
$o{machine} = $bytes->(scalar(`/bin/hostname -s 2>/dev/null`));
# A Linux runner (2026-11-03) may have no hostname command: its node name,
# without a domain. The system itself is $^O: darwin or linux.
if ($o{machine} eq '') { my $n = (POSIX::uname())[1] // ''; $n =~ s/\..*//s; $o{machine} = $bytes->($n) }
$o{os} = $cut->($^O);
# A process's command line: /proc on Linux (procps may be missing there),
# ps on macOS. And all of this account's, a line each.
my $cmd_of = sub { my $p = shift; if ($^O eq 'linux') { open(my $f, '<', "/proc/$p/cmdline") or return ''; local $/; my $c = <$f> // ''; close $f; $c =~ tr/\0/ /; $c =~ s/ \z//; return $c } return `/bin/ps -o command= -p $p 2>/dev/null` // '' };
my $mine_of = sub {
  return `/bin/ps -x -o command= 2>/dev/null` // '' unless $^O eq 'linux';
  my $all = ''; opendir(my $pd, '/proc') or return '';
  for my $p (grep { /\A\d+\z/ } readdir $pd) { my @st = stat "/proc/$p"; $all .= $cmd_of->($p) . "\n" if @st && $st[4] == $< }
  return $all;
};
my $pre = '';
for my $p (split(/:/, $ENV{PATH} // ''), "$home/.local/bin") { if ($p ne '' && -x "$p/hollow-run-preflight") { $pre = "$p/hollow-run-preflight"; last } }
if ($pre) {
  my ($out, $ok) = ('', 0);
  if (pipe(my $r, my $w)) {
    my $pid = fork();
    if (defined $pid && $pid == 0) {
      setpgrp(0, 0); close $r; open(STDOUT, '>&', $w); open(STDERR, '>&', $w); open(STDIN, '<', '/dev/null');
      exec { $pre } $pre or POSIX::_exit(127);
    }
    close $w;
    if (defined $pid) {
      my $timed_out = 0;
      eval {
        local $SIG{ALRM} = sub { die "timeout\n" };
        alarm 15; local $/; $out = <$r> // ''; waitpid($pid, 0); alarm 0;
      };
      if ($@) { $timed_out = 1; kill('KILL', -$pid); kill('KILL', $pid); waitpid($pid, 0) }
      if ($timed_out) { $out = "hollow-run-preflight didn't finish in 15 s"; $ok = 0; $o{preflightStalled} = JSON::PP::true } else { $ok = $? == 0 }
    }
  }
  my @lines = grep { /\S/ } split /\n/, ($out // '');
  $o{preflight} = $bytes->($lines[-1] // ''); $o{preflightOk} = $ok ? JSON::PP::true : JSON::PP::false;
}
my $n = 0;
if (opendir(my $rd, "$root/run")) {
  for my $r (readdir $rd) {
    next unless $r =~ /\A[0-9a-f]{12}\z/;
    open(my $f, '<', "$root/run/$r/pid") or next; my $p = <$f> // ''; close $f; chomp $p;
    next unless $p =~ /\A\d+\z/ && kill(0, $p);
    my $cmd = $cmd_of->($p);
    $n++ if $cmd =~ /hollow-remote-run/;
  }
}
$o{running} = $n;
my $mine = $mine_of->();
$o{gateRunning} = ($mine =~ m{^\S*node\s+(?:\S*/)?scripts/gate-run\.mjs}m) ? JSON::PP::true : JSON::PP::false;
my ($best, $bestname);
if (opendir(my $d, "$root/repos")) {
  for my $r (readdir $d) {
    next unless $r =~ /\A[A-Za-z0-9._-]+\.git\z/;
    my $g = "$root/repos/$r/hollow-gate-runs";
    opendir(my $gd, $g) or next;
    for my $f (readdir $gd) {
      next unless $f =~ /\A\d{4}-\d\d-\d\dT[0-9-]+Z-[0-9a-z]+\.json\z/;
      if (!defined $bestname || $f gt $bestname) { $bestname = $f; $best = "$g/$f" }
    }
  }
}
if ($best && -s $best < 1048576 && open(my $fh, '<', $best)) {
  local $/; my $text = <$fh>; close $fh;
  my $j = eval { JSON::PP->new->utf8->decode($text // '') };
  if (ref $j eq 'HASH') {
    my %g;
    for my $k (qw(start end failedStep commit)) { $g{$k} = $cut->($j->{$k}) if defined $j->{$k} && !ref $j->{$k} }
    for my $k (qw(seconds exitCode ok)) { $g{$k} = $j->{$k} if defined $j->{$k} && (!ref $j->{$k} || JSON::PP::is_bool($j->{$k})) }
    my $c = $j->{checkout} // ''; $c = '' if ref $c; $c =~ s{.*/}{}; $c =~ s/-[0-9a-f]{12}\z//;
    $g{checkout} = $cut->($c) if $c ne '';
    $o{lastGate} = \%g if defined $g{start} && $g{start} ne '';
  }
}
if (-f "$root/owner.json" && (-s _) <= 4096 && open(my $of, '<', "$root/owner.json")) {
  my $t = ''; read($of, $t, 4096); close $of;
  my $j = eval { JSON::PP->new->utf8->decode($t) };
  if (ref $j eq 'HASH') { my %w; for my $k (qw(installId key at)) { $w{$k} = $cut->($j->{$k}) if defined $j->{$k} && !ref $j->{$k} } $o{owner} = \%w }
}
my $kg = '';
for my $p ('/usr/bin', split(/:/, $ENV{PATH} // '')) { if ($p ne '' && -x "$p/ssh-keygen") { $kg = "$p/ssh-keygen"; last } }
my @fps; my $lines = 0; my $unread = $kg ? 0 : 1;
for my $ak ("$home/.ssh/authorized_keys", "$home/.ssh/authorized_keys2") {
  last if $unread;
  next unless -e $ak;
  if (!-f $ak) { $unread = 1; last }
  my $here = 0;
  if (open(my $kf, '<', $ak)) { while (my $l = <$kf>) { $here++ if $l =~ /\S/ && $l !~ /\A\s*#/; last if $. > 100000 } close $kf } else { $unread = 1; last }
  $lines += $here;
  next unless $here;
  open(my $saved, '>&', \*STDERR); open(STDERR, '>', '/dev/null');
  my $ok = open(my $kh, '-|', $kg, '-l', '-E', 'sha256', '-f', $ak);
  open(STDERR, '>&', $saved);
  if (!$ok) { $unread = 1; last }
  while (my $l = <$kh>) { push @fps, $1 if $l =~ /\A\d+\s+(SHA256:[A-Za-z0-9+\/=]+)\s/ }
  close $kh;
}
if (!$unread) {
  my %seen; my @uniq = grep { !$seen{$_}++ } @fps;
  $o{keys} = [ @uniq > 50 ? @uniq[0 .. 49] : @uniq ]; $o{keyCount} = scalar @uniq; $o{keyLines} = $lines; $o{keysRead} = scalar @fps;
}
print JSON::PP->new->ascii->canonical->encode(\%o), "\n";
"#;

/// The last status's state when it is `host`'s (other modules' tests of
/// the ssh calls that note or clear it).
#[cfg(test)]
pub(crate) fn state_of(host: &str) -> Option<&'static str> {
    LAST.lock()
        .ok()?
        .status
        .as_ref()
        .filter(|s| s.host.as_deref() == Some(host))
        .map(|s| s.state)
}

#[cfg(test)]
#[path = "remote_status_tests.rs"]
mod tests;
