//! Ending every process an agent run started, not just the CLI.
//!
//! Stop used to SIGKILL only the direct agent CLI (`kill_on_drop`,
//! `start_kill`, `Child::kill`) and then release the full-vault lease while
//! the CLI's tool shells, test runners and dev servers kept running. Both
//! installed CLIs move shell-tool commands into NEW process groups (checked on
//! 2026-09-25: Claude Code 2.1.280 runs each Bash-tool zsh as its own group
//! leader; Codex 0.153.4 gives shell commands and helpers their own groups and
//! leaves them running after the CLI's group is sent SIGTERM), and a
//! backgrounded job is reparented to launchd the moment its shell exits. So a
//! process is treated as the run's when any of these holds:
//!
//! 1. it is in the leader's group (a GUI-owned spawn leads its own group);
//! 2. it descends from the leader (by ppid) in the snapshot taken at stop or
//!    finish, whatever group it moved into;
//! 3. it was recorded, as pid plus `ps` start time, by the once-a-second
//!    sampling that runs while the leader is alive, and that start time still
//!    matches (so a reused pid is never taken for one of ours); or
//! 4. its environment carries this run's exact `HOLLOW_RUN_TAG=<32 hex>`
//!    token, which survives setsid, double-fork and reparenting.
//!
//! [`drive`] and [`run_to_completion`] are the only waiters for such a child.
//! They return only after [`GroupGuard::settle`] has sent SIGTERM to every
//! identified group, SIGKILLed survivors after a grace period, reaped the
//! direct child and rescanned. Callers release the vault lease after they
//! return, so the lease never outlives the cleanup.
//!
//! Limits, stated so the receipt wording stays honest: macOS hides the
//! environment of Apple platform binaries (`/bin/sh`, `/bin/sleep`, `perl`)
//! from `ps -E`, so a detached platform binary whose launching shell exited
//! between two samples is invisible to every mechanism above. `Verified`
//! therefore means "no process Hollow could find from this run remains".
//!
//! Never isolate or signal a process group from inside a Hollow MCP
//! subprocess (`hollow team-mcp` and friends): those children must keep the
//! server's group and inherited tag so the GUI's sweep reaches them. Use
//! [`GroupGuard::inherited`] semantics there (a spec whose scope is inherited).
//!
//! A run ends one of five ways ([`Ended`]): it exits, the caller's stop is
//! requested, its deadline passes, its usage budget's guard requests
//! [`Supervision::limit`] (`Ended::LimitReached`, Paperclip PR 8b), or the
//! no-output watchdog finds it quiet for too long (`Ended::Stalled`,
//! Paperclip PR 15b, [`Supervision::watch`]; every agent since merged plan
//! slice 3, paused while a Hollow card is pending). A limit and a stall are
//! terminated exactly like a stop (a stall with the longer
//! [`Timings::stall_grace`]); the variants exist so the caller reports them
//! as errors, never as the user's cancel. For an inherited run (a team-mcp
//! delegate) that means only its direct process.
//!
//! Activity (Paperclip PR 15b): with a [`Watch`], `drive` stamps the run's
//! [`RunActivity`] on every stdout and stderr line, and its once-a-second
//! sample ([`GroupGuard::sample`], which now also reads each process's
//! cumulative CPU time) feeds an [`ActivityProbe`]: a change in the run's
//! process set, or at least 15 cs of CPU within [`Timings::activity_window`],
//! counts as activity for the watchdog.
//!
//! Runs that outlive Hollow itself (a quit, crash or install relaunch
//! mid-run) are [`orphans`]' job: every isolated guard is recorded in the
//! app-data run ledger at adopt and removed at settle, the next launch ends
//! what a dead Hollow's entries still name, and a clean quit settles this
//! process's live entries. A termination signal settles them the same way,
//! and a crash is caught by the [`reaper`] child, which sweeps them the
//! moment this process exits (A·R8).

pub mod orphans;
pub mod reaper;

use crate::run_activity::{stall_step, RunActivity, StallStep};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::io::Write as _;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Notify;
use tokio::time::{Instant, MissedTickBehavior};

/// Environment variable carrying a run's identity token. The name must never
/// contain KEY, SECRET or TOKEN: Codex strips such variables from the shell
/// environment it gives tool commands by default.
pub const RUN_TAG_ENV: &str = "HOLLOW_RUN_TAG";
/// Longest stderr line kept; the rest of an over-long line is discarded.
pub const STDERR_LINE_MAX: usize = 4096;
/// Longest agent stdout line [`drive`] delivers. A longer one (a huge base64
/// tool result) is never buffered past this and never clipped into broken
/// JSON: `on_line` gets [`dropped_line_event`] in its place.
pub const STDOUT_LINE_MAX: usize = 16 * 1024 * 1024;
/// `type` of the event [`drive`] delivers in place of an over-long stdout
/// line. No agent CLI uses it; [`crate::agent::parse_line`] recognizes it.
pub const DROPPED_LINE_TYPE: &str = "hollow_stdout_line_dropped";
/// Newest stderr bytes an agent run keeps for its error message.
pub const STDERR_TAIL_MAX: usize = 16 * 1024;
/// Newest stderr bytes a short CLI call ([`run_to_completion`]) keeps.
pub const CAPTURE_STDERR_MAX: usize = 64 * 1024;
/// Recorded (pid, start time) identities per run. Exceeding it makes the stop
/// outcome unconfirmed rather than silently forgetting a process.
const RECORD_CAP: usize = 4096;
/// Stand-in deadline for timers that are not armed.
const FAR: Duration = Duration::from_secs(366 * 24 * 60 * 60);

/// What Hollow could establish after ending a run's processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// Every scan succeeded and no process Hollow could identify remains.
    Verified,
    /// A scan failed, a process survived, or identity tracking overflowed.
    Unconfirmed,
}

impl StopOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            StopOutcome::Verified => "verified",
            StopOutcome::Unconfirmed => "unconfirmed",
        }
    }

    /// [`StopOutcome::as_str`] read back.
    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "verified" => Some(StopOutcome::Verified),
            "unconfirmed" => Some(StopOutcome::Unconfirmed),
            _ => None,
        }
    }

    /// The outcome of two steps of one run: a single unconfirmed step makes
    /// the whole run unconfirmed.
    pub fn combine(a: Option<Self>, b: Option<Self>) -> Option<Self> {
        match (a, b) {
            (Some(Self::Unconfirmed), _) | (_, Some(Self::Unconfirmed)) => Some(Self::Unconfirmed),
            (Some(Self::Verified), _) | (_, Some(Self::Verified)) => Some(Self::Verified),
            _ => None,
        }
    }
}

/// Every wait in the stop sequence, bounded.
#[derive(Debug, Clone, Copy)]
pub struct Timings {
    /// SIGTERM → SIGKILL grace period.
    pub grace: Duration,
    /// Bound on reaping the direct child after SIGKILL.
    pub reap: Duration,
    /// Window for the post-kill verification rescans.
    pub verify: Duration,
    /// Poll interval while waiting out the grace period.
    pub poll: Duration,
    /// Descendant sampling interval while the leader runs.
    pub sample_every: Duration,
    /// How long to keep reading stdout after the leader exits while a
    /// straggler still holds the pipe open.
    pub post_exit_drain: Duration,
    /// How long to keep reading stdout after termination finished.
    pub post_stop_drain: Duration,
    /// How long a leader may keep running after it closed stdout.
    pub post_eof_exit: Duration,
    /// Bound on one `ps` scan.
    pub snapshot_timeout: Duration,
    /// Window over which CPU time must grow by [`ACTIVITY_CPU_CS`] to count
    /// as process activity for the watchdog (Paperclip PR 15b).
    pub activity_window: Duration,
    /// SIGTERM → SIGKILL grace after a watchdog stall: the contract's 5 s.
    /// A stall settles with `max(grace, stall_grace)` ([`settle_timings`]).
    pub stall_grace: Duration,
    /// After the leader exited on its own, how long its processes may take
    /// to exit by themselves (an MCP server closing on stdin EOF) before
    /// the ones still alive count as left running (overnight A3). Only
    /// spent while some are alive.
    pub linger: Duration,
}

/// CPU centiseconds within [`Timings::activity_window`] that count as
/// process activity (1 % of one core over the default 15 s window).
pub const ACTIVITY_CPU_CS: u64 = 15;

impl Default for Timings {
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(3),
            reap: Duration::from_secs(5),
            verify: Duration::from_secs(1),
            poll: Duration::from_millis(25),
            sample_every: Duration::from_secs(1),
            post_exit_drain: Duration::from_secs(2),
            post_stop_drain: Duration::from_secs(2),
            post_eof_exit: Duration::from_secs(10),
            snapshot_timeout: Duration::from_secs(2),
            activity_window: Duration::from_secs(15),
            stall_grace: Duration::from_secs(5),
            linger: Duration::from_millis(500),
        }
    }
}

/// The timings a run is settled with after it `ended`: a watchdog stall gets
/// the longer stall grace; every other ending is unchanged.
pub fn settle_timings(ended: Ended, t: &Timings) -> Timings {
    match ended {
        Ended::Stalled => Timings {
            grace: t.grace.max(t.stall_grace),
            ..*t
        },
        Ended::Exited | Ended::Stopped | Ended::TimedOut | Ended::LimitReached => *t,
    }
}

impl Timings {
    #[cfg(test)]
    pub(crate) fn fast_for_tests() -> Self {
        Self {
            grace: Duration::from_millis(300),
            reap: Duration::from_secs(2),
            verify: Duration::from_secs(1),
            poll: Duration::from_millis(20),
            sample_every: Duration::from_millis(100),
            post_exit_drain: Duration::from_millis(200),
            post_stop_drain: Duration::from_millis(500),
            post_eof_exit: Duration::from_millis(200),
            snapshot_timeout: Duration::from_secs(2),
            activity_window: Duration::from_millis(200),
            stall_grace: Duration::from_millis(300),
            linger: Duration::from_millis(300),
        }
    }
}

/// A run's random identity token (32 lowercase hex digits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTag(String);

impl RunTag {
    pub fn new() -> Self {
        Self(format!("{:032x}", rand::random::<u128>()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RunTag {
    fn default() -> Self {
        Self::new()
    }
}

/// Make a GUI-spawned command lead its own process group and carry a fresh
/// run tag. Returns the tag so owned MCP servers can be given the same one.
pub fn isolate(cmd: &mut Command) -> RunTag {
    let tag = RunTag::new();
    isolate_with(cmd, &tag);
    tag
}

/// [`isolate`] with a caller-chosen tag (already injected into MCP env).
pub fn isolate_with(cmd: &mut Command, tag: &RunTag) {
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    cmd.env(RUN_TAG_ENV, tag.as_str());
}

/// Make a synchronous GUI-side probe (`std::process::Command`) lead its own
/// process group, so a timeout can end everything it started with
/// [`end_probe_group`]. GUI process only, like [`isolate`].
pub fn isolate_probe(cmd: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = cmd;
}

/// SIGKILL whatever is left of a probe isolated by [`isolate_probe`]; its
/// pid is its group id. Call it while the probe is still unreaped, or right
/// after reaping it: a group id cannot be reused while any member remains,
/// and a group that is already gone is left alone.
pub fn end_probe_group(pid: u32) {
    #[cfg(unix)]
    if let Ok(pgid) = i32::try_from(pid) {
        if group_alive(pgid) {
            signal_group(pgid, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Add the run tag to every server's `env` in a `{"mcpServers":{…}}` config,
/// so a Hollow MCP server carries it even under a CLI (Codex) that passes MCP
/// servers only whitelisted environment variables. Invalid input is returned
/// unchanged; the adapter's own validation reports it.
pub fn with_run_tag(mcp_config: &str, tag: &RunTag) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(mcp_config) else {
        return mcp_config.to_string();
    };
    let Some(servers) = value
        .get_mut("mcpServers")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return mcp_config.to_string();
    };
    for server in servers.values_mut() {
        let Some(object) = server.as_object_mut() else {
            continue;
        };
        let env = object
            .entry("env")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(env) = env.as_object_mut() {
            env.insert(
                RUN_TAG_ENV.to_string(),
                serde_json::Value::String(tag.as_str().to_string()),
            );
        }
    }
    value.to_string()
}

/// A stop request that cannot be lost: the flag is set before waiters are
/// woken, and [`StopSignal::wait`] re-checks it after registering interest,
/// so a request made before, during or after the wait starts is observed.
/// Once requested it stays requested. Every run owns a fresh signal, so none
/// is ever cleared: clearing a shared flag when a new run starts is how a
/// cancel aimed at the previous run gets lost.
#[derive(Debug, Default)]
pub struct StopSignal {
    requested: AtomicBool,
    notify: Notify,
}

impl StopSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    pub async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.requested() {
                return;
            }
            notified.await;
        }
    }
}

async fn wait_stop(stop: Option<&StopSignal>) {
    match stop {
        Some(stop) => stop.wait().await,
        None => std::future::pending::<()>().await,
    }
}

/// Owns the identity of one spawned run and ends it on [`GroupGuard::settle`]
/// (or, as a last resort, on drop).
pub struct GroupGuard {
    /// The direct child's pid, which is also its process-group id. `None`
    /// means the command was not isolated: only the direct child is killed.
    leader: Option<i32>,
    tag: Option<String>,
    /// pid → `ps` start time for every process identified as the run's.
    recorded: HashMap<i32, String>,
    overflow: bool,
    /// A descendant sample failed, so a process may have gone unrecorded.
    scan_gap: bool,
    armed: bool,
    /// The run ledger this run was recorded in, and its key (the tag):
    /// removed at settle, or after the drop sweep ([`orphans`]).
    ledger: Option<(&'static orphans::Ledger, String)>,
    /// The run's registration with the brokers' peer check and its private
    /// MCP config file (`run_auth`, merged plan slice 6): released, and the
    /// file removed, at settle or drop.
    binding: Option<crate::run_auth::Binding>,
    /// Processes of the run, other than its leader, still alive when settle
    /// began (overnight A3): after a normal exit, work the agent left running.
    leftovers: usize,
    #[cfg(test)]
    fail_scans: bool,
}

impl GroupGuard {
    /// Adopt a freshly spawned child. Isolation is derived from the command
    /// itself (the run tag it explicitly set) plus the child actually leading
    /// its own group, never from a caller-supplied flag. An isolated child is
    /// recorded in the run ledger (GUI process only; see [`orphans`]).
    pub fn adopt(cmd: &Command, child: &Child) -> Self {
        Self::adopt_in(cmd, child, None, orphans::global())
    }

    /// [`GroupGuard::adopt`] for a run with a history run id, which the
    /// ledger keeps so the next launch can note the run-queue row whose
    /// leftovers it stopped.
    pub fn adopt_run(cmd: &Command, child: &Child, run_id: Option<&str>) -> Self {
        Self::adopt_in(cmd, child, run_id, orphans::global())
    }

    /// [`GroupGuard::adopt`] without a ledger entry, for a child whose own
    /// registry already ends it after Hollow restarts (`services.rs` keeps
    /// `services.json` with pid + start time).
    pub fn adopt_unrecorded(cmd: &Command, child: &Child) -> Self {
        Self::adopt_in(cmd, child, None, None)
    }

    pub(crate) fn adopt_in(
        cmd: &Command,
        child: &Child,
        run_id: Option<&str>,
        ledger: Option<&'static orphans::Ledger>,
    ) -> Self {
        let tag = cmd
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == OsStr::new(RUN_TAG_ENV))
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned());
        let Some(tag) = tag else {
            return Self::inherited();
        };
        match leader_of(child) {
            Some(leader) => {
                let ledger = ledger.map(|ledger| {
                    ledger.record(ledger.entry(&tag, leader, run_id));
                    (ledger, tag.clone())
                });
                let binding = crate::run_auth::adopt(cmd.as_std(), &tag, leader);
                Self {
                    leader: Some(leader),
                    tag: Some(tag),
                    recorded: HashMap::new(),
                    overflow: false,
                    scan_gap: false,
                    armed: true,
                    ledger,
                    binding,
                    leftovers: 0,
                    #[cfg(test)]
                    fail_scans: false,
                }
            }
            None => Self::inherited(),
        }
    }

    /// A guard that never signals a group: stop and finish reach only the
    /// direct child. Used for children spawned without isolation.
    pub fn inherited() -> Self {
        Self {
            leader: None,
            tag: None,
            recorded: HashMap::new(),
            overflow: false,
            scan_gap: false,
            armed: false,
            ledger: None,
            binding: None,
            leftovers: 0,
            #[cfg(test)]
            fail_scans: false,
        }
    }

    pub fn is_isolated(&self) -> bool {
        self.leader.is_some()
    }

    /// How many of the run's processes other than its leader were alive when
    /// [`Self::settle`] began (0 before it ran, or for a non-isolated run).
    pub fn leftovers(&self) -> usize {
        self.leftovers
    }

    #[cfg(test)]
    pub(crate) fn failing_scans(mut self) -> Self {
        self.fail_scans = true;
        self
    }

    #[cfg(test)]
    fn fail_scans(&self) -> bool {
        self.fail_scans
    }

    #[cfg(not(test))]
    fn fail_scans(&self) -> bool {
        false
    }

    /// End the run: SIGTERM every identified group, SIGKILL survivors after
    /// the grace period, reap the direct child, and rescan. Returns `None`
    /// for a non-isolated child (only it was killed), otherwise whether the
    /// cleanup was verified. The guard disarms afterwards either way.
    pub async fn settle(&mut self, child: &mut Child, t: &Timings) -> Option<StopOutcome> {
        if let Some(binding) = &self.binding {
            binding.revoke();
        }
        let leader = self.leader;
        let outcome = match leader {
            #[cfg(unix)]
            Some(leader) => Some(self.settle_group(leader, child, t).await),
            _ => {
                if matches!(child.try_wait(), Ok(None)) {
                    let _ = child.start_kill();
                }
                let _ = tokio::time::timeout(t.reap, child.wait()).await;
                None
            }
        };
        self.armed = false;
        if let Some((ledger, tag)) = self.ledger.take() {
            ledger.forget(&tag);
        }
        self.binding = None;
        outcome
    }

    /// Record the run's live descendants (pid + start time) while the leader
    /// runs, so a job reparented to launchd before stop is still known.
    /// Returns what the sample saw of the run (its pids and their summed CPU
    /// time) for the watchdog's [`ActivityProbe`], or `None` when the scan
    /// failed or the run is not isolated.
    pub(crate) async fn sample(&mut self, t: &Timings) -> Option<ProcessDigest> {
        #[cfg(unix)]
        {
            let leader = self.leader?;
            return match snapshot(false, None, t, self.fail_scans()).await {
                Ok(rows) => {
                    let ours = identify(
                        &rows,
                        Some((leader, true)),
                        &self.recorded,
                        own_pid(),
                        own_pgrp(),
                    );
                    let live: HashMap<i32, &str> = rows
                        .iter()
                        .filter(|row| !row.zombie)
                        .map(|row| (row.pid, row.start.as_str()))
                        .collect();
                    self.recorded
                        .retain(|pid, start| live.get(pid).is_some_and(|s| *s == start.as_str()));
                    self.record(&ours);
                    Some(digest_of(&rows, &ours))
                }
                Err(_) => {
                    self.scan_gap = true;
                    None
                }
            };
        }
        #[cfg(not(unix))]
        {
            let _ = t;
            None
        }
    }

    fn record(&mut self, ours: &Ours) {
        for (pid, _, start) in &ours.members {
            if self.recorded.len() >= RECORD_CAP && !self.recorded.contains_key(pid) {
                self.overflow = true;
                continue;
            }
            self.recorded.insert(*pid, start.clone());
        }
    }
}

#[cfg(unix)]
impl GroupGuard {
    async fn scan(&self, with_env: bool, t: &Timings) -> Result<Vec<Proc>, String> {
        snapshot(with_env, self.tag.as_deref(), t, self.fail_scans()).await
    }

    /// The leader reference identify() may trust: `held` is true while the
    /// direct child is unreaped, i.e. its pid cannot have been reused.
    fn leader_ref(leader: i32, child: &Child) -> Option<(i32, bool)> {
        Some((leader, child.id().is_some()))
    }

    async fn settle_group(&mut self, leader: i32, child: &mut Child, t: &Timings) -> StopOutcome {
        let own_pid = own_pid();
        let own_pgid = own_pgrp();
        let mut scans_ok = !self.scan_gap;

        // 1. Identify and record everything that is ours BEFORE any signal:
        //    once the leader dies its children reparent and lose the ppid link.
        let _ = child.try_wait();
        let first = match self.scan(true, t).await {
            Ok(rows) => identify(
                &rows,
                Self::leader_ref(leader, child),
                &self.recorded,
                own_pid,
                own_pgid,
            ),
            Err(_) => {
                scans_ok = false;
                Ours::default()
            }
        };
        self.record(&first);
        // 1b. Overnight A3: after the leader exited on its own, wait (at most
        //     `linger`, only while some are alive) for its processes to exit
        //     by themselves, then count the ones still running: work it left.
        //     Never after Hollow's own stop (the leader is still alive then).
        let _ = child.try_wait();
        let others = |ours: &Ours| {
            ours.members
                .iter()
                .filter(|(pid, _, _)| *pid != leader && pid_alive(*pid))
                .count()
        };
        if child.id().is_none() && !t.linger.is_zero() && others(&first) > 0 {
            let until = Instant::now() + t.linger;
            while Instant::now() < until && others(&first) > 0 {
                tokio::time::sleep(t.poll).await;
            }
            // Counted from a fresh scan, so a zombie (dead, not yet reaped)
            // is not work left running.
            if others(&first) > 0 {
                if let Ok(rows) = self.scan(false, t).await {
                    self.leftovers = first
                        .members
                        .iter()
                        .filter(|(pid, _, start)| {
                            *pid != leader
                                && rows
                                    .iter()
                                    .any(|r| r.pid == *pid && !r.zombie && r.start == *start)
                        })
                        .count();
                }
            }
        }

        // 2. Normal finish with nothing left: one scan, no signals.
        let _ = child.try_wait();
        if scans_ok && first.members.is_empty() && child.id().is_none() && !group_alive(leader) {
            return verdict(true, &first, true, self.overflow);
        }

        // 3. SIGTERM the leader's group, every identified group, and members
        //    that share a group we must never signal.
        signal_found(leader, child.id().is_some(), &first, libc::SIGTERM);

        // 4. Grace period. try_wait reaps a zombie leader so it cannot keep
        //    its group looking alive.
        let grace_end = Instant::now() + t.grace;
        loop {
            let _ = child.try_wait();
            let leader_done = child.id().is_none() && !group_alive(leader);
            let rest_done = first.groups.iter().all(|group| !group_alive(*group))
                && first.pids.iter().all(|pid| !pid_alive(*pid));
            if (leader_done && rest_done) || Instant::now() >= grace_end {
                break;
            }
            tokio::time::sleep(t.poll).await;
        }

        // 5. SIGKILL survivors from a fresh snapshot, so every pid signalled
        //    still carries the start time it was recorded with.
        match self.scan(false, t).await {
            Ok(rows) => {
                let _ = child.try_wait();
                let now = identify(
                    &rows,
                    Self::leader_ref(leader, child),
                    &self.recorded,
                    own_pid,
                    own_pgid,
                );
                self.record(&now);
                signal_found(leader, child.id().is_some(), &now, libc::SIGKILL);
            }
            Err(_) => {
                scans_ok = false;
                // Without a scan, pids cannot be re-identified; a live group id
                // cannot have been reused, so groups are still safe to signal.
                let alive = Ours {
                    members: Vec::new(),
                    groups: first
                        .groups
                        .iter()
                        .copied()
                        .filter(|group| group_alive(*group))
                        .collect(),
                    pids: BTreeSet::new(),
                };
                signal_found(leader, child.id().is_some(), &alive, libc::SIGKILL);
            }
        }

        // 6. Reap the direct child.
        let reaped = matches!(tokio::time::timeout(t.reap, child.wait()).await, Ok(Ok(_)));

        // 7. Verify, killing anything newly found, up to three rescans.
        let verify_end = Instant::now() + t.verify;
        let mut remaining = Ours::default();
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.scan(true, t).await {
                Ok(rows) => {
                    remaining = identify(
                        &rows,
                        Self::leader_ref(leader, child),
                        &self.recorded,
                        own_pid,
                        own_pgid,
                    );
                    if remaining.members.is_empty() && !group_alive(leader) {
                        break;
                    }
                    self.record(&remaining);
                    signal_found(leader, child.id().is_some(), &remaining, libc::SIGKILL);
                }
                Err(_) => scans_ok = false,
            }
            if attempt >= 3 || Instant::now() >= verify_end {
                break;
            }
            tokio::time::sleep(t.verify / 4).await;
        }
        verdict(
            scans_ok && reaped && !self.scan_gap,
            &remaining,
            !group_alive(leader),
            self.overflow,
        )
    }
}

impl Drop for GroupGuard {
    /// A future that was dropped mid-run gets no verification, but the group
    /// is still SIGKILLed, and escaped processes are swept best-effort.
    fn drop(&mut self) {
        if let Some(binding) = &self.binding {
            binding.revoke();
        }
        if !self.armed {
            return;
        }
        self.armed = false;
        #[cfg(unix)]
        {
            if let Some(leader) = self.leader {
                if group_alive(leader) {
                    signal_group(leader, libc::SIGKILL);
                }
                // Without a runtime the ledger entry stays, so a clean quit
                // or the next launch still ends what the SIGKILL missed.
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    let tag = self.tag.take();
                    let recorded = std::mem::take(&mut self.recorded);
                    let fail = self.fail_scans();
                    let ledger = self.ledger.take();
                    drop(runtime.spawn(async move {
                        sweep_after_drop(leader, tag, recorded, fail).await;
                        if let Some((ledger, tag)) = ledger {
                            ledger.forget(&tag);
                        }
                    }));
                }
            }
        }
    }
}

#[cfg(unix)]
async fn sweep_after_drop(
    leader: i32,
    tag: Option<String>,
    recorded: HashMap<i32, String>,
    fail: bool,
) {
    let t = Timings::default();
    for pass in 0..2 {
        if pass > 0 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let Ok(rows) = snapshot(true, tag.as_deref(), &t, fail).await else {
            continue;
        };
        let found = identify(
            &rows,
            Some((leader, false)),
            &recorded,
            own_pid(),
            own_pgrp(),
        );
        signal_found(leader, false, &found, libc::SIGKILL);
    }
}

#[cfg(unix)]
fn leader_of(child: &Child) -> Option<i32> {
    let pid = i32::try_from(child.id()?).ok().filter(|pid| *pid > 1)?;
    // SAFETY: getpgid only reads the process table.
    let pgid = unsafe { libc::getpgid(pid) };
    if pgid == pid || (pgid == -1 && last_errno() == Some(libc::ESRCH)) {
        Some(pid)
    } else {
        let _ = writeln!(
            std::io::stderr().lock(),
            "[process_group] pid {pid} is not its own group leader (pgid {pgid}); stop will signal only it"
        );
        None
    }
}

#[cfg(not(unix))]
fn leader_of(_child: &Child) -> Option<i32> {
    None
}

// ── Supervising a run ───────────────────────────────────────────────────────

/// Where a child's stderr lines go besides the bounded tail.
#[derive(Debug, Clone)]
pub enum StderrPolicy {
    Silent,
    /// Echo each line to Hollow's own stderr, prefixed with the label.
    Echo(String),
}

pub struct Supervision<'a> {
    pub stop: Option<&'a StopSignal>,
    /// A usage budget's stop (Paperclip PR 8b): the run's budget guard
    /// requests it from inside `on_line` when an enforced limit is crossed.
    /// Never cleared. Ends the run as [`Ended::LimitReached`], with exactly
    /// the same termination as a stop; a user stop requested at the same
    /// moment wins (it is checked first).
    pub limit: Option<&'a StopSignal>,
    pub deadline: Option<Duration>,
    pub stderr: StderrPolicy,
    pub timings: Timings,
    /// The run's activity clock and, for a chat turn or workflow run of any
    /// agent, the no-output watchdog (Paperclip PR 15b, merged plan slice
    /// 3). `None` for everything else (one-shots, delegates, probes).
    pub watch: Option<Watch>,
}

/// What `drive` stamps and watches (Paperclip PR 15b).
#[derive(Debug, Clone)]
pub struct Watch {
    /// Restamped on every stdout and stderr line and on process activity.
    pub activity: Arc<RunActivity>,
    /// End the run as [`Ended::Stalled`] after this long with neither output
    /// nor process activity. `None` only stamps (no watchdog).
    pub stall_after: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Exited,
    Stopped,
    TimedOut,
    /// The run's budget guard requested [`Supervision::limit`] (Paperclip PR
    /// 8b). Kept apart from `Stopped` so a budget stop is reported as an
    /// error, never as the user's own cancel. A GUI-owned run's whole process
    /// tree is ended as for a stop; an inherited one (a team-mcp delegate)
    /// loses only its direct process.
    LimitReached,
    /// The no-output watchdog ([`Watch::stall_after`], Paperclip PR 15b)
    /// found the run quiet for too long: no stdout or stderr line and no
    /// process activity. Ended like a stop, with [`Timings::stall_grace`].
    /// An error (the `stalled` result kind), never a cancel, never retried.
    Stalled,
}

pub struct Driven {
    pub ended: Ended,
    pub status: Option<std::io::Result<ExitStatus>>,
    pub outcome: Option<StopOutcome>,
    pub missing_stdout: bool,
    pub stdout_error: Option<String>,
    pub stderr_tail: String,
    /// The leader's exit status was observed before [`GroupGuard::settle`]
    /// began (Paperclip PR 9b). A signal in that status therefore came from
    /// outside Hollow, never from its own stop sequence. False when stdout
    /// was missing (settle ran first) and when the loop ended with the leader
    /// still running.
    pub leader_exited_first: bool,
    /// [`GroupGuard::leftovers`]: the run's processes other than its leader
    /// still alive when settle began. With [`Self::leader_exited_first`],
    /// work the agent left running when it ended (overnight A3).
    pub leftovers: usize,
}

impl Driven {
    /// Processes the agent left running when it exited on its own (overnight
    /// A3): [`Self::leftovers`] after a normal exit observed before settle,
    /// else 0 (after Hollow's own stop, leftovers are expected).
    pub fn left_running(&self) -> usize {
        if self.ended == Ended::Exited && self.leader_exited_first {
            self.leftovers
        } else {
            0
        }
    }
}

/// Own `child` until it and everything it started are gone. Stdout lines go
/// to `on_line` as they arrive, including lines written while the run is
/// being terminated. Returns after [`GroupGuard::settle`], on normal exit,
/// stop, or deadline alike.
pub async fn drive<F: FnMut(&str)>(
    child: Child,
    guard: GroupGuard,
    sup: Supervision<'_>,
    mut on_line: F,
) -> Driven {
    // Declared in this order so the guard drops (SIGKILLs) before the child
    // if this future is itself dropped mid-run.
    let mut child = child;
    let mut guard = guard;
    let Supervision {
        stop,
        limit,
        deadline,
        stderr: stderr_policy,
        timings: t,
        watch,
    } = sup;
    let activity: Option<Arc<RunActivity>> = watch.as_ref().map(|w| Arc::clone(&w.activity));
    let stall_after: Option<Duration> = watch.as_ref().and_then(|w| w.stall_after);
    if let Some(activity) = &activity {
        activity.mark_spawned();
    }

    let tail = Arc::new(Mutex::new(Tail::new(STDERR_TAIL_MAX)));
    let stderr_task = child.stderr.take().map(|pipe| {
        let tail = Arc::clone(&tail);
        let activity = activity.clone();
        tokio::spawn(
            async move { drain_into(pipe, &stderr_policy, &tail, activity.as_deref()).await },
        )
    });

    let Some(stdout) = child.stdout.take() else {
        let outcome = guard.settle(&mut child, &t).await;
        let status = child.try_wait().transpose();
        let stderr_tail = finish_tail(stderr_task, &tail, t.post_stop_drain).await;
        return Driven {
            ended: Ended::Exited,
            status,
            outcome,
            missing_stdout: true,
            stdout_error: None,
            stderr_tail,
            leader_exited_first: false,
            leftovers: guard.leftovers(),
        };
    };

    // Bounded and lenient (A·R3): strict `lines()` ended the read at the
    // first invalid UTF-8 byte (the live agent was then killed by the
    // post-EOF wait) and buffered a huge line whole.
    let mut lines = BoundedLines::new(stdout, STDOUT_LINE_MAX);
    let mut status: Option<std::io::Result<ExitStatus>> = None;
    let mut eof = false;
    let mut stdout_error: Option<String> = None;
    let far = Instant::now() + FAR;
    let deadline_timer =
        tokio::time::sleep_until(deadline.map(|d| Instant::now() + d).unwrap_or(far));
    tokio::pin!(deadline_timer);
    let exit_drain = tokio::time::sleep_until(far);
    tokio::pin!(exit_drain);
    let eof_wait = tokio::time::sleep_until(far);
    tokio::pin!(eof_wait);
    let every = t.sample_every.max(Duration::from_millis(10));
    let mut sampler = tokio::time::interval_at(Instant::now() + every, every);
    sampler.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let sampling = guard.is_isolated();
    // The watchdog (Paperclip PR 15b): armed only with both an activity
    // clock and a limit. When its timer fires, `stall_step` decides over
    // monotonic time whether the run has really been quiet that long, or
    // re-arms it for the remainder.
    let stall_limit = match (&activity, stall_after) {
        (Some(_), Some(limit)) => Some(limit.max(Duration::from_millis(1))),
        _ => None,
    };
    let stall_timer =
        tokio::time::sleep_until(stall_limit.map(|l| Instant::now() + l).unwrap_or(far));
    tokio::pin!(stall_timer);
    let mut probe = ActivityProbe::new(t.activity_window);

    let ended = loop {
        if eof && status.is_some() {
            break Ended::Exited;
        }
        tokio::select! {
            biased;
            _ = wait_stop(stop) => break Ended::Stopped,
            _ = wait_stop(limit) => break Ended::LimitReached,
            _ = &mut deadline_timer, if deadline.is_some() => break Ended::TimedOut,
            _ = &mut stall_timer, if stall_limit.is_some() => {
                let limit_ms = i64::try_from(stall_limit.unwrap_or(FAR).as_millis()).unwrap_or(i64::MAX);
                // A pending Hollow card pauses it (merged plan slice 3).
                let last = activity.as_ref().map_or(0, |a| a.quiet_since_mono_ms());
                match stall_step(crate::run_activity::mono_ms(), last, limit_ms) {
                    StallStep::Fire => break Ended::Stalled,
                    StallStep::RearmIn(ms) => {
                        stall_timer.as_mut().reset(Instant::now() + Duration::from_millis(ms));
                    }
                }
            }
            exit = child.wait(), if status.is_none() => {
                status = Some(exit);
                if !eof {
                    exit_drain.as_mut().reset(Instant::now() + t.post_exit_drain);
                }
            }
            line = lines.next_checked(), if !eof => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(activity) = &activity {
                            activity.touch_stdout();
                        }
                        on_line(deliverable(line, &tail).as_str())
                    }
                    Ok(None) => eof = true,
                    Err(error) => {
                        stdout_error = Some(error.to_string());
                        eof = true;
                    }
                }
                if eof && status.is_none() {
                    eof_wait.as_mut().reset(Instant::now() + t.post_eof_exit);
                }
            }
            // The leader exited but a straggler still holds stdout open.
            _ = &mut exit_drain, if status.is_some() && !eof => break Ended::Exited,
            // Stdout closed but the leader is still running; settle ends it.
            _ = &mut eof_wait, if eof && status.is_none() => break Ended::Exited,
            _ = sampler.tick(), if sampling && status.is_none() => {
                let digest = guard.sample(&t).await;
                if let (Some(activity), Some(digest)) = (&activity, digest) {
                    if probe.observe(Instant::now().into_std(), &digest) {
                        activity.touch_process();
                    }
                }
            }
        }
    };

    // Read before settle can reap the leader: a status seen here was not
    // caused by Hollow's own stop sequence.
    let leader_exited_first = status.is_some();
    // A stall settles with the longer stall grace (the contract's 5 s).
    let settle_t = settle_timings(ended, &t);

    // Terminate while still reading stdout, so events a dying agent writes
    // (tool calls the ingest audit and chat evidence need) are not lost, and
    // so a CLI blocked on a full stdout pipe can make progress to exit.
    let outcome = {
        let settling = guard.settle(&mut child, &settle_t);
        tokio::pin!(settling);
        let mut settled: Option<Option<StopOutcome>> = None;
        let drain = tokio::time::sleep_until(far);
        tokio::pin!(drain);
        loop {
            if let Some(done) = settled {
                if eof {
                    break done;
                }
            }
            tokio::select! {
                biased;
                done = &mut settling, if settled.is_none() => {
                    settled = Some(done);
                    drain.as_mut().reset(Instant::now() + t.post_stop_drain);
                }
                line = lines.next_checked(), if !eof => match line {
                    Ok(Some(line)) => {
                        if let Some(activity) = &activity {
                            activity.touch_stdout();
                        }
                        on_line(deliverable(line, &tail).as_str())
                    }
                    Ok(None) => eof = true,
                    Err(error) => {
                        if stdout_error.is_none() {
                            stdout_error = Some(error.to_string());
                        }
                        eof = true;
                    }
                },
                _ = &mut drain, if settled.is_some() => break settled.flatten(),
            }
        }
    };

    let status = status.or_else(|| child.try_wait().transpose());
    let stderr_tail = finish_tail(stderr_task, &tail, t.post_stop_drain).await;
    Driven {
        ended,
        status,
        outcome,
        missing_stdout: false,
        stdout_error,
        stderr_tail,
        leader_exited_first,
        leftovers: guard.leftovers(),
    }
}

/// Result of a short, non-streaming CLI call.
pub struct GroupOutput {
    pub status: Option<ExitStatus>,
    pub wait_error: Option<std::io::Error>,
    pub stdout: Vec<u8>,
    /// Newest [`CAPTURE_STDERR_MAX`] bytes, lines joined with `\n`.
    pub stderr: String,
    pub timed_out: bool,
    /// `stop` was requested before the CLI exited; its processes were ended.
    pub stopped: bool,
    /// What settling established. [`run_to_completion`] also reports an
    /// unconfirmed cleanup on Hollow's stderr itself, for callers with no
    /// receipt to put it in.
    pub outcome: Option<StopOutcome>,
    /// Writing `input` to the child's stdin failed.
    pub input_error: Option<std::io::Error>,
    /// [`run_to_completion_capped`] stopped reading stdout at its cap and
    /// closed the pipe. A child still writing then usually dies of SIGPIPE,
    /// so check this BEFORE `status`: a truncated read is not a failure.
    pub stdout_truncated: bool,
}

/// Run a short GUI-owned CLI (plaud, codex probes, `claude mcp list`) in its
/// own group, capture its output, and settle it like an agent run. Replaces
/// `cmd.output()` / `wait_with_output()` wrapped in a timeout, which left the
/// CLI's children running when the timeout dropped the future. A `stop`
/// request ends the run the same way, so a caller never has to drop this
/// future to cancel it.
pub async fn run_to_completion(
    cmd: &mut Command,
    input: Option<&[u8]>,
    limit: Duration,
    stop: Option<&StopSignal>,
) -> std::io::Result<GroupOutput> {
    run_to_completion_capped(cmd, input, limit, stop, usize::MAX).await
}

/// [`run_to_completion`] that keeps at most `stdout_cap` bytes of stdout.
/// At the cap it stops reading and drops the pipe, so a child that keeps
/// writing gets EPIPE/SIGPIPE instead of filling memory (Paperclip PR 16's
/// git reads). `stdout_truncated` then says so; a truncated child usually
/// exits with SIGPIPE, so callers must check `stdout_truncated` before
/// `status`. Settling is unchanged.
pub async fn run_to_completion_capped(
    cmd: &mut Command,
    input: Option<&[u8]>,
    limit: Duration,
    stop: Option<&StopSignal>,
    stdout_cap: usize,
) -> std::io::Result<GroupOutput> {
    isolate(cmd);
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    let child = cmd.spawn()?;
    let guard = GroupGuard::adopt(cmd, &child);
    let mut child = child;
    let mut guard = guard;
    let t = Timings::default();

    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stdout_task = child.stdout.take().map(|pipe| {
        let buf = Arc::clone(&stdout_buf);
        tokio::spawn(async move { read_capped_into(pipe, &buf, stdout_cap).await })
    });
    let tail = Arc::new(Mutex::new(Tail::new(CAPTURE_STDERR_MAX)));
    let stderr_task = child.stderr.take().map(|pipe| {
        let tail = Arc::clone(&tail);
        tokio::spawn(async move { drain_into(pipe, &StderrPolicy::Silent, &tail, None).await })
    });
    let input_task = match (child.stdin.take(), input) {
        (Some(mut stdin), Some(bytes)) => {
            let bytes = bytes.to_vec();
            Some(tokio::spawn(async move {
                let written = stdin.write_all(&bytes).await;
                drop(stdin);
                written
            }))
        }
        _ => None,
    };

    let (wait_error, timed_out, stopped) = tokio::select! {
        biased;
        _ = wait_stop(stop) => (None, false, true),
        exit = child.wait() => (exit.err(), false, false),
        _ = tokio::time::sleep(limit) => (None, true, false),
    };
    // A short CLI call never reads what it left running (overnight A3's
    // linger is for agent runs).
    let settle_t = Timings {
        linger: Duration::ZERO,
        ..t
    };
    let outcome = guard.settle(&mut child, &settle_t).await;
    if outcome == Some(StopOutcome::Unconfirmed) {
        let program = cmd.as_std().get_program().to_string_lossy().into_owned();
        let _ = writeln!(
            std::io::stderr().lock(),
            "[process_group] {program}: cleanup unconfirmed; a process it started may still be running"
        );
    }

    let input_error = join_within(input_task, t.post_stop_drain)
        .await
        .and_then(Result::err);
    let stdout_truncated = join_within(stdout_task, t.post_stop_drain)
        .await
        .unwrap_or(false);
    let stdout = std::mem::take(&mut *stdout_buf.lock().unwrap_or_else(PoisonError::into_inner));
    let stderr = finish_tail(stderr_task, &tail, t.post_stop_drain).await;
    let status = child.try_wait().ok().flatten();
    Ok(GroupOutput {
        status,
        wait_error,
        stdout,
        stderr,
        timed_out,
        stopped,
        outcome,
        input_error,
        stdout_truncated,
    })
}

/// Read `reader` into `out` until EOF or `cap` bytes. Returns true when it
/// stopped at the cap: reaching the cap counts as truncated even if EOF would
/// have followed, since waiting to find out could block. The reader is
/// dropped on return, closing the pipe.
async fn read_capped_into<R: AsyncRead + Unpin>(
    mut reader: R,
    out: &Mutex<Vec<u8>>,
    cap: usize,
) -> bool {
    let mut chunk = vec![0u8; 8192];
    let mut kept = 0usize;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => return false,
            Ok(n) => {
                let take = n.min(cap.saturating_sub(kept));
                if take > 0 {
                    out.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .extend_from_slice(&chunk[..take]);
                    kept += take;
                }
                if kept >= cap {
                    return true;
                }
            }
        }
    }
}

/// Join a helper task within `within`, aborting it if a missed straggler
/// still holds its pipe open. Whatever it buffered stays readable.
async fn join_within<T>(task: Option<tokio::task::JoinHandle<T>>, within: Duration) -> Option<T> {
    let mut task = task?;
    match tokio::time::timeout(within, &mut task).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(_)) => None,
        Err(_) => {
            task.abort();
            None
        }
    }
}

/// Join the stderr reader and return the tail collected so far.
async fn finish_tail(
    task: Option<tokio::task::JoinHandle<()>>,
    tail: &Mutex<Tail>,
    within: Duration,
) -> String {
    let _ = join_within(task, within).await;
    let joined = tail.lock().unwrap_or_else(PoisonError::into_inner).joined();
    joined
}

// ── Bounded line reading (stderr, and agent stdout) ─────────────────────────

/// Line reader that never buffers more than `max` bytes of one line: the
/// excess is discarded up to the next newline. Decodes lossily, so an invalid
/// UTF-8 byte is replaced, never an error. The line in progress lives in the
/// struct, not in the `next_*` future, so both readers are cancel-safe inside
/// `select!` (`drive` polls stdout beside timers that fire mid-line).
pub struct BoundedLines<R> {
    reader: BufReader<R>,
    max: usize,
    /// The current line's kept bytes, at most `max`.
    line: Vec<u8>,
    /// Every byte the current line has had so far, kept or discarded.
    seen: u64,
    /// The current line has had at least one byte (or its newline).
    started: bool,
}

/// One line from [`BoundedLines::next_checked`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedLine {
    /// A whole line, lossily decoded, its newline (and a trailing `\r`) gone.
    Line(String),
    /// A line longer than the reader's `max`. Its first `max` bytes were held
    /// and then thrown away, the rest never buffered. `bytes` is its full
    /// length without the newline.
    TooLong { bytes: u64 },
}

impl<R: AsyncRead + Unpin> BoundedLines<R> {
    pub fn new(reader: R, max: usize) -> Self {
        Self {
            reader: BufReader::new(reader),
            max,
            line: Vec::new(),
            seen: 0,
            started: false,
        }
    }

    /// The next line, clipped to `max` bytes (stderr: a clipped line is still
    /// worth showing).
    pub async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        Ok(self.next_raw().await?.map(|(line, _)| finish_line(line)))
    }

    /// The next line, or [`BoundedLine::TooLong`] in place of one over `max`
    /// (agent stdout: a clipped JSON event would be silently corrupt).
    pub async fn next_checked(&mut self) -> std::io::Result<Option<BoundedLine>> {
        let max = u64::try_from(self.max).unwrap_or(u64::MAX);
        Ok(self.next_raw().await?.map(|(line, seen)| {
            if seen > max {
                BoundedLine::TooLong { bytes: seen }
            } else {
                BoundedLine::Line(finish_line(line))
            }
        }))
    }

    /// The next line's kept bytes and its full length. The only await is
    /// `fill_buf`, which consumes nothing, so a cancelled call loses nothing.
    async fn next_raw(&mut self) -> std::io::Result<Option<(Vec<u8>, u64)>> {
        loop {
            let buf = self.reader.fill_buf().await?;
            if buf.is_empty() {
                if !self.started {
                    return Ok(None);
                }
                return Ok(Some(self.take_line()));
            }
            self.started = true;
            let room = self.max.saturating_sub(self.line.len());
            match buf.iter().position(|byte| *byte == b'\n') {
                Some(end) => {
                    self.line.extend_from_slice(&buf[..end.min(room)]);
                    self.seen += end as u64;
                    self.reader.consume(end + 1);
                    return Ok(Some(self.take_line()));
                }
                None => {
                    let len = buf.len();
                    self.line.extend_from_slice(&buf[..len.min(room)]);
                    self.seen += len as u64;
                    self.reader.consume(len);
                }
            }
        }
    }

    fn take_line(&mut self) -> (Vec<u8>, u64) {
        let seen = std::mem::take(&mut self.seen);
        self.started = false;
        (std::mem::take(&mut self.line), seen)
    }
}

/// The one-line JSON event [`drive`] hands `on_line` in place of a stdout line
/// of `bytes` bytes that exceeded `limit`.
pub fn dropped_line_event(bytes: u64, limit: usize) -> String {
    serde_json::json!({
        "type": DROPPED_LINE_TYPE,
        "message": "line too long, dropped",
        "bytes": bytes,
        "limit": limit,
    })
    .to_string()
}

/// The dropped line's length when `line` is a [`dropped_line_event`].
pub fn dropped_line_bytes(line: &str) -> Option<u64> {
    if !line.contains(DROPPED_LINE_TYPE) {
        return None;
    }
    let event: serde_json::Value = serde_json::from_str(line).ok()?;
    if event.get("type")?.as_str()? != DROPPED_LINE_TYPE {
        return None;
    }
    event.get("bytes")?.as_u64()
}

/// The stdout reader's next line as `drive` delivers it: an over-long line
/// becomes a [`dropped_line_event`], also noted in the run's stderr tail so a
/// failed run's error says why an event is missing.
fn deliverable(line: BoundedLine, tail: &Mutex<Tail>) -> String {
    match line {
        BoundedLine::Line(line) => line,
        BoundedLine::TooLong { bytes } => {
            let note = format!(
                "hollow: dropped an agent stdout line of {bytes} bytes (limit {STDOUT_LINE_MAX})"
            );
            tail.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(note);
            dropped_line_event(bytes, STDOUT_LINE_MAX)
        }
    }
}

fn finish_line(mut line: Vec<u8>) -> String {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8_lossy(&line).into_owned()
}

/// Newest lines up to a byte budget.
struct Tail {
    lines: VecDeque<String>,
    bytes: usize,
    max: usize,
}

impl Tail {
    fn new(max: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            bytes: 0,
            max,
        }
    }

    fn push(&mut self, mut line: String) {
        if line.len() + 1 > self.max {
            let mut keep = self.max.saturating_sub(1);
            while keep > 0 && !line.is_char_boundary(keep) {
                keep -= 1;
            }
            line.truncate(keep);
        }
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        while self.bytes > self.max {
            match self.lines.pop_front() {
                Some(old) => self.bytes -= old.len() + 1,
                None => break,
            }
        }
    }

    fn joined(&self) -> String {
        self.lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Read stderr lines into `tail`, echoing per `policy`. Each line also
/// restamps the run's activity clock, when it has one (Paperclip PR 15b).
async fn drain_into<R: AsyncRead + Unpin>(
    reader: R,
    policy: &StderrPolicy,
    tail: &Mutex<Tail>,
    activity: Option<&RunActivity>,
) {
    let mut lines = BoundedLines::new(reader, STDERR_LINE_MAX);
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(activity) = activity {
            activity.touch_output();
        }
        if let StderrPolicy::Echo(label) = policy {
            // Never eprintln!: a closed stderr must not abort the process.
            let _ = writeln!(std::io::stderr().lock(), "{label} {line}");
        }
        tail.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
    }
}

/// Read stderr to EOF, echoing per `policy`, and return the newest
/// `max_bytes` of it as `\n`-joined lines. Test-only: production readers
/// share their [`Tail`] with the waiter through [`drain_into`].
#[cfg(test)]
async fn drain_stderr_tail<R: AsyncRead + Unpin>(
    reader: R,
    policy: StderrPolicy,
    max_bytes: usize,
) -> String {
    let tail = Mutex::new(Tail::new(max_bytes));
    drain_into(reader, &policy, &tail, None).await;
    let joined = tail.lock().unwrap_or_else(PoisonError::into_inner).joined();
    joined
}

// ── Scanning and signalling ─────────────────────────────────────────────────

/// One `ps` row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(unix), allow(dead_code))]
struct Proc {
    pid: i32,
    ppid: i32,
    pgid: i32,
    zombie: bool,
    /// `ps` lstart, whitespace-normalized; with the pid, a process identity.
    start: String,
    /// The row's environment carried this run's exact tag token.
    tagged: bool,
    /// Cumulative CPU time in centiseconds (`ps time=`), read only from the
    /// environment-free scan (Paperclip PR 15b); `None` otherwise.
    cpu_cs: Option<u64>,
}

/// Parse `pid ppid pgid stat <5 lstart tokens>` followed by `time` (the
/// environment-free scan) or by `command + environment` (`with_env`).
/// Malformed lines are skipped. `tagged` requires `with_env` and the exact
/// whitespace-delimited token `HOLLOW_RUN_TAG=<tag>`; `cpu_cs` is read only
/// without `with_env`, where a command line can never be mistaken for it.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse_ps(out: &str, with_env: bool, tag: Option<&str>) -> Vec<Proc> {
    let needle = tag.map(|tag| format!("{RUN_TAG_ENV}={tag}"));
    out.lines()
        .filter_map(|line| {
            let mut tokens = line.split_whitespace();
            let pid: i32 = tokens.next()?.parse().ok()?;
            let ppid: i32 = tokens.next()?.parse().ok()?;
            let pgid: i32 = tokens.next()?.parse().ok()?;
            let stat = tokens.next()?;
            let start: Vec<&str> = tokens.by_ref().take(5).collect();
            if start.len() != 5 {
                return None;
            }
            let cpu_cs = if with_env {
                None
            } else {
                tokens.next().and_then(parse_cputime)
            };
            let tagged = with_env
                && needle
                    .as_deref()
                    .is_some_and(|needle| tokens.any(|token| token == needle));
            Some(Proc {
                pid,
                ppid,
                pgid,
                zombie: stat.starts_with('Z'),
                start: start.join(" "),
                tagged,
                cpu_cs,
            })
        })
        .collect()
}

/// Parse a `ps time=` value, `[[dd-]hh:]mm:ss[.cc]` (macOS prints minutes
/// past 59, e.g. `123:45.67`; procps prints `[dd-]hh:mm:ss`), into
/// centiseconds. `None` for anything else.
pub(crate) fn parse_cputime(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    let (days, clock) = match raw.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>().ok()?, clock),
        None => (0, raw),
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (h.parse::<u64>().ok()?, *m, *s),
        _ => return None,
    };
    let minutes: u64 = minutes.parse().ok()?;
    let (whole, fraction) = match seconds.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (seconds, ""),
    };
    let whole: u64 = whole.parse().ok()?;
    if !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut digits = fraction.chars().map(|c| u64::from(c as u8 - b'0'));
    let centis = digits.next().unwrap_or(0) * 10 + digits.next().unwrap_or(0);
    let total_seconds = ((days * 24 + hours) * 60 + minutes) * 60 + whole;
    Some(total_seconds * 100 + centis)
}

/// What one descendant sample saw of the run (Paperclip PR 15b).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessDigest {
    /// The run's live pids.
    pub members: BTreeSet<i32>,
    /// Their summed cumulative CPU time, in centiseconds.
    pub cpu_cs: u64,
}

#[cfg_attr(not(unix), allow(dead_code))]
fn digest_of(rows: &[Proc], ours: &Ours) -> ProcessDigest {
    let members: BTreeSet<i32> = ours.members.iter().map(|(pid, _, _)| *pid).collect();
    let cpu_cs = rows
        .iter()
        .filter(|row| members.contains(&row.pid))
        .filter_map(|row| row.cpu_cs)
        .sum();
    ProcessDigest { members, cpu_cs }
}

/// Decides whether a run's processes are doing work while it prints
/// nothing (Paperclip PR 15b, ported from Paperclip's output-inactivity
/// monitor): a change in its set of pids, or at least [`ACTIVITY_CPU_CS`] of
/// CPU time within the window, counts; an idle process ticking a few
/// centiseconds does not. Pure: the caller passes the time.
#[derive(Debug)]
pub struct ActivityProbe {
    window: std::time::Duration,
    last_members: Option<BTreeSet<i32>>,
    /// Where the current CPU window started: (time, summed CPU then).
    baseline: Option<(std::time::Instant, u64)>,
}

impl ActivityProbe {
    pub fn new(window: std::time::Duration) -> Self {
        Self {
            window,
            last_members: None,
            baseline: None,
        }
    }

    /// Fold in one sample; true when it shows activity. The first sample
    /// only sets the baseline (the spawn itself already counted).
    pub fn observe(&mut self, now: std::time::Instant, digest: &ProcessDigest) -> bool {
        let Some(previous) = self.last_members.replace(digest.members.clone()) else {
            self.baseline = Some((now, digest.cpu_cs));
            return false;
        };
        if previous != digest.members {
            self.baseline = Some((now, digest.cpu_cs));
            return true;
        }
        let (since, cpu_then) = self.baseline.unwrap_or((now, digest.cpu_cs));
        if digest.cpu_cs >= cpu_then.saturating_add(ACTIVITY_CPU_CS) {
            self.baseline = Some((now, digest.cpu_cs));
            return true;
        }
        if now.saturating_duration_since(since) >= self.window {
            // The window passed without enough work: start a new one.
            self.baseline = Some((now, digest.cpu_cs));
        }
        false
    }
}

/// Processes identified as the run's in one snapshot.
#[derive(Debug, Default)]
#[cfg_attr(not(unix), allow(dead_code))]
struct Ours {
    /// (pid, pgid, start time).
    members: Vec<(i32, i32, String)>,
    /// Groups to signal whole: member groups other than pgid ≤ 1 and
    /// Hollow's own group.
    groups: BTreeSet<i32>,
    /// Members that sit in a group we must never signal; signalled by pid.
    pids: BTreeSet<i32>,
}

/// Identify the run's live processes. `leader` is `(pid, held)`, where `held`
/// means the direct child is still unreaped so its pid cannot have been
/// reused. An unheld leader row is trusted only if its start time matches the
/// recorded one; if no row has the leader's pid, rows still in its group are
/// leftovers of ours, because a group id is never reused while it exists.
#[cfg_attr(not(unix), allow(dead_code))]
fn identify(
    rows: &[Proc],
    leader: Option<(i32, bool)>,
    recorded: &HashMap<i32, String>,
    own_pid: i32,
    own_pgid: i32,
) -> Ours {
    let leader = leader
        .filter(
            |(pid, held)| match rows.iter().find(|row| row.pid == *pid) {
                Some(row) => *held || recorded.get(pid).is_some_and(|start| *start == row.start),
                None => true,
            },
        )
        .map(|(pid, _)| pid);

    let mut descendants: HashSet<i32> = HashSet::new();
    if let Some(leader) = leader {
        let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
        for row in rows {
            children.entry(row.ppid).or_default().push(row.pid);
        }
        let mut visited: HashSet<i32> = HashSet::from([leader]);
        let mut queue: VecDeque<i32> = VecDeque::from([leader]);
        while let Some(parent) = queue.pop_front() {
            for child in children.get(&parent).into_iter().flatten() {
                if visited.insert(*child) {
                    descendants.insert(*child);
                    queue.push_back(*child);
                }
            }
        }
    }

    let mut ours = Ours::default();
    for row in rows {
        if row.zombie || row.pid <= 1 || row.pid == own_pid {
            continue;
        }
        let is_ours = leader.is_some_and(|leader| row.pgid == leader || row.pid == leader)
            || descendants.contains(&row.pid)
            || recorded
                .get(&row.pid)
                .is_some_and(|start| *start == row.start)
            || row.tagged;
        if !is_ours {
            continue;
        }
        ours.members.push((row.pid, row.pgid, row.start.clone()));
        if row.pgid > 1 && row.pgid != own_pgid {
            ours.groups.insert(row.pgid);
        } else {
            ours.pids.insert(row.pid);
        }
    }
    ours
}

/// Verified only when every scan succeeded, nothing identified remains, the
/// leader's group is gone, and identity tracking never overflowed.
#[cfg_attr(not(unix), allow(dead_code))]
fn verdict(
    all_scans_ok: bool,
    remaining: &Ours,
    leader_group_gone: bool,
    overflow: bool,
) -> StopOutcome {
    if all_scans_ok && remaining.members.is_empty() && leader_group_gone && !overflow {
        StopOutcome::Verified
    } else {
        StopOutcome::Unconfirmed
    }
}

/// One read-only process-table scan. `with_env` adds each process's
/// environment so the run tag can be matched; the output is never logged.
#[cfg(unix)]
async fn snapshot(
    with_env: bool,
    tag: Option<&str>,
    t: &Timings,
    fail: bool,
) -> Result<Vec<Proc>, String> {
    let scan = ps_scan(with_env, t, fail).await?;
    Ok(scan.rows(tag))
}

/// One `ps` scan's raw output, parsed per tag by [`PsScan::rows`], so a
/// sweep can match several tags against a single process-table read.
#[cfg(unix)]
struct PsScan {
    text: String,
    with_env: bool,
    env_in_ps: bool,
}

#[cfg(unix)]
impl PsScan {
    /// The scan's rows, `tagged` for exactly `tag` (Linux reads it from
    /// `/proc/<pid>/environ`).
    fn rows(&self, tag: Option<&str>) -> Vec<Proc> {
        let rows = parse_ps(&self.text, self.env_in_ps, tag);
        #[cfg(not(target_os = "macos"))]
        let rows = tag_from_proc(rows, self.with_env, tag);
        #[cfg(target_os = "macos")]
        let _ = self.with_env;
        rows
    }
}

#[cfg(unix)]
async fn ps_scan(with_env: bool, t: &Timings, fail: bool) -> Result<PsScan, String> {
    if fail {
        return Err("process scan unavailable".into());
    }
    let env_in_ps = with_env && cfg!(target_os = "macos");
    let mut cmd = Command::new(if cfg!(target_os = "macos") {
        "/bin/ps"
    } else {
        "ps"
    });
    cmd.arg("-A");
    if env_in_ps {
        cmd.args(["-E", "-ww"]);
    }
    // Without the environment, `time` (cumulative CPU) follows lstart, for
    // the watchdog's activity probe (Paperclip PR 15b).
    cmd.arg("-o").arg(if env_in_ps {
        "pid=,ppid=,pgid=,stat=,lstart=,command="
    } else {
        "pid=,ppid=,pgid=,stat=,lstart=,time="
    });
    cmd.env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(t.snapshot_timeout, cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("process scan: {error}")),
        Err(_) => return Err("process scan timed out".into()),
    };
    if !output.status.success() {
        return Err(format!("process scan exited {}", output.status));
    }
    Ok(PsScan {
        text: String::from_utf8_lossy(&output.stdout).into_owned(),
        with_env,
        env_in_ps,
    })
}

/// Linux: procps `ps` cannot print environments; read /proc instead.
#[cfg(all(unix, not(target_os = "macos")))]
fn tag_from_proc(mut rows: Vec<Proc>, with_env: bool, tag: Option<&str>) -> Vec<Proc> {
    let (true, Some(tag)) = (with_env, tag) else {
        return rows;
    };
    let needle = format!("{RUN_TAG_ENV}={tag}");
    for row in &mut rows {
        if let Ok(raw) = std::fs::read(format!("/proc/{}/environ", row.pid)) {
            row.tagged = raw
                .split(|byte| *byte == 0)
                .any(|entry| entry == needle.as_bytes());
        }
    }
    rows
}

#[cfg(unix)]
fn own_pid() -> i32 {
    std::process::id() as i32
}

#[cfg(unix)]
fn own_pgrp() -> i32 {
    // SAFETY: getpgrp cannot fail.
    unsafe { libc::getpgrp() }
}

#[cfg(unix)]
fn last_errno() -> Option<i32> {
    std::io::Error::last_os_error().raw_os_error()
}

/// Signal a whole group. Refuses pgid ≤ 1 and Hollow's own group.
#[cfg(unix)]
fn signal_group(pgid: i32, signal: i32) -> bool {
    if pgid <= 1 || pgid == own_pgrp() {
        return false;
    }
    // SAFETY: killpg only delivers a signal; the group was identified as the
    // run's in a snapshot taken just before (or is the unreaped leader's).
    unsafe { libc::killpg(pgid, signal) == 0 }
}

/// Signal one process. Refuses pid ≤ 1 and Hollow itself.
#[cfg(unix)]
fn signal_pid(pid: i32, signal: i32) -> bool {
    if pid <= 1 || pid == own_pid() {
        return false;
    }
    // SAFETY: kill only delivers a signal to a pid identified just before.
    unsafe { libc::kill(pid, signal) == 0 }
}

#[cfg(unix)]
fn group_alive(pgid: i32) -> bool {
    if pgid <= 1 {
        return false;
    }
    // SAFETY: signal 0 only probes for existence.
    let rc = unsafe { libc::kill(-pgid, 0) };
    rc == 0 || last_errno() != Some(libc::ESRCH)
}

#[cfg(unix)]
fn pid_alive(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    // SAFETY: signal 0 only probes for existence.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || last_errno() != Some(libc::ESRCH)
}

/// Signal the leader's group (only while its id is provably ours: the child
/// is unreaped, or the group still has members so the id cannot be reused),
/// every other identified group, and the pid-only members.
#[cfg(unix)]
fn signal_found(leader: i32, leader_held: bool, ours: &Ours, signal: i32) {
    if leader_held || group_alive(leader) {
        signal_group(leader, signal);
    }
    signal_members(ours, Some(leader), signal);
}

/// Signal every identified group (except `skip`, already signalled) and the
/// pid-only members.
#[cfg(unix)]
fn signal_members(ours: &Ours, skip: Option<i32>, signal: i32) {
    for group in &ours.groups {
        if Some(*group) != skip {
            signal_group(*group, signal);
        }
    }
    for pid in &ours.pids {
        signal_pid(*pid, signal);
    }
}

#[cfg(all(test, unix))]
pub(crate) mod test_support {
    use super::*;

    fn shell(script: &str, env: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd
    }

    /// `/bin/sh -c script` leading its own group with a fresh run tag.
    pub(crate) fn spawn_isolated(script: &str, env: &[(&str, &str)]) -> (Command, Child) {
        let mut cmd = shell(script, env);
        isolate(&mut cmd);
        let child = cmd.spawn().expect("spawn isolated test shell");
        (cmd, child)
    }

    /// `/bin/sh -c script` leading its own group with a fresh run tag,
    /// returned unspawned (for `agent::collect_command` tests).
    pub(crate) fn isolated_shell(script: &str, env: &[(&str, &str)]) -> Command {
        let mut cmd = shell(script, env);
        isolate(&mut cmd);
        cmd
    }

    /// `/bin/sh -c script` in the test process's own group, untagged.
    pub(crate) fn spawn_plain(script: &str) -> (Command, Child) {
        let mut cmd = shell(script, &[]);
        let child = cmd.spawn().expect("spawn plain test shell");
        (cmd, child)
    }

    /// Read one stdout line, then hand stdout back so the pipe stays open.
    pub(crate) async fn read_line(child: &mut Child, within: Duration) -> String {
        let stdout = child.stdout.take().expect("piped stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        tokio::time::timeout(within, reader.read_line(&mut line))
            .await
            .expect("a stdout line in time")
            .expect("read stdout line");
        child.stdout = Some(reader.into_inner());
        line.trim().to_string()
    }

    pub(crate) fn pgid_of(pid: i32) -> i32 {
        // SAFETY: getpgid only reads the process table.
        unsafe { libc::getpgid(pid) }
    }

    /// Running means present and not a zombie.
    pub(crate) async fn is_running(pid: i32) -> bool {
        if !pid_alive(pid) {
            return false;
        }
        let pid_arg = pid.to_string();
        match Command::new("ps")
            .args(["-o", "stat=", "-p", pid_arg.as_str()])
            .output()
            .await
        {
            Ok(output) => {
                let stat = String::from_utf8_lossy(&output.stdout);
                let stat = stat.trim();
                !stat.is_empty() && !stat.starts_with('Z')
            }
            Err(_) => true,
        }
    }

    pub(crate) async fn wait_gone(pid: i32, within: Duration) -> bool {
        let end = Instant::now() + within;
        loop {
            if !is_running(pid).await {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Wait until `pid`'s command line contains `needle`. To wait for an
    /// exec, pass something only the exec'd argv holds, such as
    /// [`self_exe_name`]: until it execs, a forked job still shows its
    /// launching shell's argv, which already names the helper test.
    pub(crate) async fn wait_exec(pid: i32, needle: &str, within: Duration) -> bool {
        let end = Instant::now() + within;
        let pid_arg = pid.to_string();
        loop {
            if let Ok(output) = Command::new("ps")
                .args(["-ww", "-o", "command=", "-p", pid_arg.as_str()])
                .output()
                .await
            {
                if String::from_utf8_lossy(&output.stdout).contains(needle) {
                    return true;
                }
            }
            if Instant::now() >= end {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub(crate) fn self_exe() -> String {
        std::env::current_exe()
            .expect("test binary path")
            .to_string_lossy()
            .into_owned()
    }

    /// The test binary's file name (ASCII, e.g. `hollow_lib-<hash>`). It
    /// appears in a helper's argv only once the helper has exec'd, because
    /// the launching script refers to the binary as `"$HOLLOW_TEST_EXE"`.
    pub(crate) fn self_exe_name() -> String {
        std::env::current_exe()
            .expect("test binary path")
            .file_name()
            .expect("test binary file name")
            .to_string_lossy()
            .into_owned()
    }

    pub(crate) fn kill_now(pid: i32) {
        if pid > 1 {
            // SAFETY: test cleanup of a pid the test itself started.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }

    /// A self-exec'd copy of the test binary running the ignored
    /// `sleeper_helper` test: a NON-platform binary whose environment `ps -E`
    /// can read, detached into its own group and reparented to launchd once
    /// the launching shell exits. Only the run tag can find it afterwards.
    pub(crate) const TAGGED_ORPHAN_SCRIPT: &str = "set -m; \"$HOLLOW_TEST_EXE\" --exact process_group::tests::sleeper_helper --ignored --test-threads=1 >/dev/null 2>&1 & echo $!";
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::test_support::*;
    use super::*;

    /// Helper process for the tag tests (run by self-exec, never by the suite).
    #[test]
    #[ignore = "helper process for the run-tag sweep tests"]
    fn sleeper_helper() {
        let seconds: u64 = std::env::var("HOLLOW_TEST_SLEEP_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
            .min(60);
        std::thread::sleep(Duration::from_secs(seconds));
    }

    fn row(pid: i32, ppid: i32, pgid: i32) -> Proc {
        Proc {
            pid,
            ppid,
            pgid,
            zombie: false,
            start: format!("start-{pid}"),
            tagged: false,
            cpu_cs: None,
        }
    }

    fn fast<'a>() -> Supervision<'a> {
        Supervision {
            stop: None,
            limit: None,
            deadline: Some(Duration::from_secs(30)),
            stderr: StderrPolicy::Silent,
            timings: Timings::fast_for_tests(),
            watch: None,
        }
    }

    /// A supervision with an activity clock and, optionally, a watchdog.
    fn watched<'a>(activity: &Arc<RunActivity>, stall_after: Option<Duration>) -> Supervision<'a> {
        Supervision {
            watch: Some(Watch {
                activity: Arc::clone(activity),
                stall_after,
            }),
            ..fast()
        }
    }

    /// Paperclip PR 15b: `ps time=` values in both platform shapes, and CPU
    /// time read only from the environment-free scan.
    #[test]
    fn parse_cputime_and_sampling_format() {
        assert_eq!(parse_cputime("0:00.02"), Some(2));
        assert_eq!(parse_cputime("1:02.50"), Some(6250));
        assert_eq!(parse_cputime("1:02:03"), Some(372_300));
        assert_eq!(parse_cputime("2-01:00:00"), Some(17_640_000));
        // macOS prints minutes past 59.
        assert_eq!(parse_cputime("123:45.67"), Some((123 * 60 + 45) * 100 + 67));
        assert_eq!(parse_cputime("0:01.5"), Some(150));
        for bad in [
            "",
            "-",
            "abc",
            "1:2:3:4",
            "x-01:00:00",
            "0:0a.00",
            "0:00.x1",
        ] {
            assert_eq!(parse_cputime(bad), None, "{bad:?}");
        }

        let plain = "  101     1   101 Ss   Fri Sep  5 10:00:00 2026     0:01.25\n  102   101   101 S    Fri Sep  5 10:00:01 2026 -\n";
        let rows = parse_ps(plain, false, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].cpu_cs, Some(125));
        assert_eq!(rows[0].start, "Fri Sep 5 10:00:00 2026");
        assert_eq!(
            rows[1].cpu_cs, None,
            "an unreadable time is unknown, not zero"
        );
        // With the environment, the token after lstart is the command.
        let env = " 200 1 200 S Fri Sep 5 10:00:00 2026 0:09.00 HOME=/u\n";
        assert!(parse_ps(env, true, None).iter().all(|r| r.cpu_cs.is_none()));

        // A digest sums only the run's own members.
        let mut ours_rows = vec![row(100, 50, 100), row(101, 100, 100), row(300, 1, 300)];
        ours_rows[0].cpu_cs = Some(10);
        ours_rows[1].cpu_cs = Some(5);
        ours_rows[2].cpu_cs = Some(999);
        let ours = identify(&ours_rows, Some((100, true)), &HashMap::new(), 50, 50);
        let digest = digest_of(&ours_rows, &ours);
        assert_eq!(digest.members, BTreeSet::from([100, 101]));
        assert_eq!(digest.cpu_cs, 15);
    }

    #[test]
    fn activity_probe_counts_membership_and_cpu_not_idle() {
        let window = Duration::from_secs(15);
        let t0 = std::time::Instant::now();
        let digest = |members: &[i32], cpu: u64| ProcessDigest {
            members: members.iter().copied().collect(),
            cpu_cs: cpu,
        };
        let mut probe = ActivityProbe::new(window);
        assert!(
            !probe.observe(t0, &digest(&[1, 2], 100)),
            "the first sample is only the baseline"
        );
        assert!(
            !probe.observe(t0 + Duration::from_secs(1), &digest(&[1, 2], 101)),
            "+1 cs is idle"
        );
        assert!(
            probe.observe(t0 + Duration::from_secs(2), &digest(&[1, 2, 3], 101)),
            "a new pid"
        );
        assert!(!probe.observe(t0 + Duration::from_secs(3), &digest(&[1, 2, 3], 110)));
        assert!(
            probe.observe(t0 + Duration::from_secs(4), &digest(&[1, 2, 3], 116)),
            "+15 cs within the window"
        );
        assert!(
            probe.observe(t0 + Duration::from_secs(5), &digest(&[1, 2], 116)),
            "a pid left"
        );
        // Slow CPU spread over more than a window never adds up.
        assert!(!probe.observe(t0 + Duration::from_secs(12), &digest(&[1, 2], 126)));
        assert!(
            !probe.observe(t0 + Duration::from_secs(21), &digest(&[1, 2], 128)),
            "the window restarts"
        );
        assert!(!probe.observe(t0 + Duration::from_secs(30), &digest(&[1, 2], 140)));
        assert!(
            probe.observe(t0 + Duration::from_secs(31), &digest(&[1, 2], 145)),
            "+17 cs in the new window"
        );
    }

    #[test]
    fn stalled_settles_with_the_stall_grace() {
        let t = Timings::default();
        assert_eq!(t.stall_grace, Duration::from_secs(5));
        assert_eq!(
            settle_timings(Ended::Stalled, &t).grace,
            Duration::from_secs(5)
        );
        for ended in [
            Ended::Exited,
            Ended::Stopped,
            Ended::TimedOut,
            Ended::LimitReached,
        ] {
            assert_eq!(settle_timings(ended, &t).grace, t.grace, "{ended:?}");
        }
        // Never shorter than the ordinary grace.
        let long = Timings {
            grace: Duration::from_secs(9),
            ..t
        };
        assert_eq!(
            settle_timings(Ended::Stalled, &long).grace,
            Duration::from_secs(9)
        );
        let fast = Timings::fast_for_tests();
        assert_eq!(
            settle_timings(Ended::Stalled, &fast).grace,
            fast.stall_grace
        );
    }

    /// Overnight A3: an agent that exits on its own with a background job
    /// still running reports it (`left_running`), and the job is ended; one
    /// that leaves nothing reports none.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_normal_exit_reports_the_background_work_it_left() {
        let (cmd, child) = spawn_isolated("/bin/sleep 30 >/dev/null 2>&1 & echo started", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let driven = drive(child, guard, watched(&activity, None), |_| {}).await;
        assert_eq!(driven.ended, Ended::Exited);
        assert!(driven.leader_exited_first);
        assert_eq!(driven.left_running(), 1, "the backgrounded sleep");
        assert_eq!(
            driven.outcome,
            Some(StopOutcome::Verified),
            "and it was ended"
        );

        let (cmd, child) = spawn_isolated("echo done", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let driven = drive(child, guard, watched(&activity, None), |_| {}).await;
        assert_eq!(driven.left_running(), 0);

        // One that exits by itself shortly after the leader (an MCP server
        // closing on stdin EOF) is not work left running: the linger waits
        // for it. The control with a 1 ms linger counts it.
        let lingering = "/bin/sleep 0.6 >/dev/null 2>&1 & echo started";
        let with_linger = |ms: u64| {
            let mut sup = watched(&activity, None);
            sup.timings.linger = Duration::from_millis(ms);
            sup
        };
        let (cmd, child) = spawn_isolated(lingering, &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let driven = drive(child, guard, with_linger(1_500), |_| {}).await;
        assert_eq!(driven.left_running(), 0, "it exited within the linger");
        let (cmd, child) = spawn_isolated(lingering, &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let driven = drive(child, guard, with_linger(1), |_| {}).await;
        assert_eq!(driven.left_running(), 1, "without the wait it is counted");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drive_stamps_activity_on_stdout_and_stderr() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("echo err >&2; /bin/sleep 0.2")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        isolate(&mut cmd);
        let child = cmd.spawn().unwrap();
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let driven = drive(child, guard, watched(&activity, None), |_| {}).await;
        assert_eq!(driven.ended, Ended::Exited);
        assert!(
            activity.first_output_after_spawn_ms().is_some(),
            "a stderr line is output"
        );
        assert!(activity.last_output_at().is_some());
        // F11: but it is not the agent's first stdout (nor text).
        let first = activity.first_outputs();
        assert!(first.any.is_some());
        assert_eq!((first.stdout, first.text), (None, None), "stderr only");

        // stderr first, stdout later: the two are told apart.
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("echo err >&2; /bin/sleep 0.3; echo out")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        isolate(&mut cmd);
        let child = cmd.spawn().unwrap();
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let driven = drive(child, guard, watched(&activity, None), |_| {}).await;
        assert_eq!(driven.ended, Ended::Exited);
        let first = activity.first_outputs();
        let (any, stdout) = (first.any.unwrap(), first.stdout.unwrap());
        assert!(
            stdout >= any + 200,
            "stderr at {any} ms, stdout at {stdout} ms"
        );

        let (cmd, child) = spawn_isolated("echo out", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let driven = drive(child, guard, watched(&activity, None), |_| {}).await;
        assert_eq!(driven.ended, Ended::Exited);
        assert!(
            activity.first_output_after_spawn_ms().is_some(),
            "a stdout line is output"
        );
        assert!(activity.last_output_at().is_some());
    }

    /// Paperclip PR 15b: a run with no output and no process work is ended
    /// by the watchdog, its whole tree included, with a verified cleanup.
    #[cfg(unix)]
    #[tokio::test]
    async fn watchdog_stops_a_silent_run_and_verifies_cleanup() {
        let (cmd, child) = spawn_isolated("/bin/sleep 60 & /bin/sleep 60", &[]);
        let leader = child.id().unwrap() as i32;
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let started = std::time::Instant::now();
        let driven = drive(
            child,
            guard,
            watched(&activity, Some(Duration::from_millis(400))),
            |_| {},
        )
        .await;
        assert_eq!(driven.ended, Ended::Stalled);
        // The watchdog (`Stalled` above), not the 60 s sleeps.
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{:?}",
            started.elapsed()
        );
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        assert!(!driven.leader_exited_first, "Hollow's own stop ended it");
        assert!(wait_gone(leader, Duration::from_secs(3)).await);
    }

    /// Merged plan slice 3 (A·R2): a run waiting on a pending Hollow card
    /// is not quiet. A silent run held past several limits exits on its own
    /// instead of stalling; once the card is released (here, mid-run), the
    /// quiet clock restarts and the watchdog ends it a full limit later.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_pending_card_pauses_the_watchdog() {
        use crate::run_activity::CardHold;
        // Held for the whole run: exits normally after 1.2 s, three limits.
        let (cmd, child) = spawn_isolated("/bin/sleep 1.2", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let hold = CardHold::new(Arc::clone(&activity));
        let driven = drive(
            child,
            guard,
            watched(&activity, Some(Duration::from_millis(400))),
            |_| {},
        )
        .await;
        drop(hold);
        assert_eq!(driven.ended, Ended::Exited, "a pending card never trips it");

        // Released after 1 s: the stall comes a limit after the release, not
        // a limit after the spawn, and still leaves no child behind.
        let (cmd, child) = spawn_isolated("/bin/sleep 60 & /bin/sleep 60", &[]);
        let leader = child.id().unwrap() as i32;
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let hold = CardHold::new(Arc::clone(&activity));
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            drop(hold);
        });
        let started = std::time::Instant::now();
        let driven = drive(
            child,
            guard,
            watched(&activity, Some(Duration::from_millis(400))),
            |_| {},
        )
        .await;
        release.await.unwrap();
        assert_eq!(driven.ended, Ended::Stalled);
        assert!(
            started.elapsed() >= Duration::from_millis(1400),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        assert!(wait_gone(leader, Duration::from_secs(3)).await);
    }

    /// Output and process churn both keep a run alive; neither ends as a
    /// stall.
    #[cfg(unix)]
    #[tokio::test]
    async fn watchdog_is_reset_by_output_and_by_process_churn() {
        // Periodic output, well inside the limit, for longer than the limit,
        // from ONE process that spawns nothing and barely uses CPU: a shell
        // loop around `/bin/sleep` would change the sampled process set
        // every tick and pass on churn alone.
        let (cmd, child) = spawn_isolated(
            "exec /usr/bin/perl -e '$|=1; for (1..6) { print \"tick\\n\"; select(undef, undef, undef, 0.15) }'",
            &[],
        );
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let driven = drive(
            child,
            guard,
            watched(&activity, Some(Duration::from_millis(500))),
            |_| {},
        )
        .await;
        assert_eq!(driven.ended, Ended::Exited, "output resets the watchdog");

        // No output at all, but a new child every 100 ms: the sampled
        // process set keeps changing.
        let (cmd, child) = spawn_isolated(
            "for i in 1 2 3 4 5 6 7 8 9 10 11 12; do /bin/sleep 0.1; done",
            &[],
        );
        let guard = GroupGuard::adopt(&cmd, &child);
        let activity = Arc::new(RunActivity::new());
        let driven = drive(
            child,
            guard,
            watched(&activity, Some(Duration::from_millis(600))),
            |_| {},
        )
        .await;
        assert_eq!(
            driven.ended,
            Ended::Exited,
            "process churn resets the watchdog"
        );
        assert!(activity.last_output_at().is_none(), "it never printed");
    }

    #[test]
    fn parse_ps_rows_and_exact_tag_tokens() {
        let out = "  101     1   101 Ss   Fri Sep  5 10:00:00 2026\n  102   101   101 Z    Fri Sep  5 10:00:01 2026\ngarbage\n  12 x 3 S Fri Sep 5 10:00:00 2026\n  103   101   103 S+   Fri Sep 5 10:00:02\n";
        let rows = parse_ps(out, false, None);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!((rows[0].pid, rows[0].ppid, rows[0].pgid), (101, 1, 101));
        assert_eq!(rows[0].start, "Fri Sep 5 10:00:00 2026");
        assert!(!rows[0].zombie);
        assert!(rows[1].zombie);

        let tag = "0123456789abcdef0123456789abcdef";
        let env = format!(
            " 200 1 200 S Fri Sep 5 10:00:00 2026 /usr/bin/node x.js HOME=/u {RUN_TAG_ENV}={tag}\n\
             201 1 201 S Fri Sep 5 10:00:00 2026 node {RUN_TAG_ENV}={tag}x\n\
             202 1 202 S Fri Sep 5 10:00:00 2026 node X{RUN_TAG_ENV}={tag}\n\
             203 1 203 S Fri Sep 5 10:00:00 2026 node {RUN_TAG_ENV}=ffffffffffffffffffffffffffffffff\n"
        );
        let rows = parse_ps(&env, true, Some(tag));
        assert_eq!(rows.len(), 4);
        let tagged: Vec<i32> = rows.iter().filter(|r| r.tagged).map(|r| r.pid).collect();
        assert_eq!(tagged, vec![200]);
        assert!(parse_ps(&env, false, Some(tag)).iter().all(|r| !r.tagged));
        assert!(parse_ps(&env, true, None).iter().all(|r| !r.tagged));
    }

    #[test]
    fn identify_covers_descendants_recorded_and_tagged_never_own_group() {
        let (own_pid, own_pgid) = (50, 50);
        let mut rows = vec![
            row(100, own_pid, 100),  // leader
            row(101, 100, 100),      // in-group child
            row(102, 101, 102),      // escaped, below an in-group intermediate
            row(103, 102, 103),      // below an escaped node
            row(104, 1, 104),        // recorded orphan, start time matches
            row(105, 1, 105),        // recorded pid now reused (start differs)
            row(106, 1, 106),        // tagged, unrelated parent
            row(107, 101, own_pgid), // joined Hollow's own group
            row(108, 100, 100),      // zombie
            row(1, 0, 1),            // launchd
            row(own_pid, 1, own_pgid),
            row(200, 201, 200), // unrelated ppid cycle
            row(201, 200, 200),
            row(300, 1, 300), // unrelated
        ];
        rows[6].tagged = true;
        rows[8].zombie = true;
        let recorded = HashMap::from([
            (104, "start-104".to_string()),
            (105, "an earlier process".to_string()),
        ]);
        let ours = identify(&rows, Some((100, true)), &recorded, own_pid, own_pgid);
        let pids: BTreeSet<i32> = ours.members.iter().map(|m| m.0).collect();
        assert_eq!(pids, BTreeSet::from([100, 101, 102, 103, 104, 106, 107]));
        assert_eq!(ours.groups, BTreeSet::from([100, 102, 103, 104, 106]));
        assert!(!ours.groups.contains(&own_pgid));
        assert_eq!(ours.pids, BTreeSet::from([107]));

        // A ppid cycle reachable from the leader terminates.
        let cyclic = vec![row(100, 101, 100), row(101, 100, 100)];
        assert_eq!(
            identify(
                &cyclic,
                Some((100, true)),
                &HashMap::new(),
                own_pid,
                own_pgid
            )
            .members
            .len(),
            2
        );

        // After the leader is reaped, a new process reusing its pid is not ours.
        let reused = vec![row(100, 1, 100), row(120, 100, 100)];
        let recorded = HashMap::from([(100, "the original leader".to_string())]);
        assert!(
            identify(&reused, Some((100, false)), &recorded, own_pid, own_pgid)
                .members
                .is_empty()
        );

        // With the leader gone, rows left in its group are still ours.
        let leftovers = vec![row(130, 1, 100)];
        assert_eq!(
            identify(
                &leftovers,
                Some((100, false)),
                &HashMap::new(),
                own_pid,
                own_pgid
            )
            .members
            .len(),
            1
        );
    }

    #[test]
    fn verdict_requires_every_scan_and_no_survivors() {
        let none = Ours::default();
        let some = Ours {
            members: vec![(9, 9, "s".into())],
            ..Ours::default()
        };
        assert_eq!(verdict(true, &none, true, false), StopOutcome::Verified);
        for (scans, remaining, gone, overflow) in [
            (false, &none, true, false),
            (true, &some, true, false),
            (true, &none, false, false),
            (true, &none, true, true),
        ] {
            assert_eq!(
                verdict(scans, remaining, gone, overflow),
                StopOutcome::Unconfirmed
            );
        }
    }

    #[tokio::test]
    async fn bounded_lines_clip_and_tail_keeps_newest() {
        let mut data = vec![b'x'; 100_000];
        data.extend_from_slice(b"\nshort\n");
        let mut lines = BoundedLines::new(&data[..], STDERR_LINE_MAX);
        let first = lines.next_line().await.unwrap().unwrap();
        assert!(first.len() <= STDERR_LINE_MAX);
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("short"));
        assert_eq!(lines.next_line().await.unwrap(), None);

        let invalid: &[u8] = b"ok \xff\xfe bytes\r\n";
        let mut lines = BoundedLines::new(invalid, STDERR_LINE_MAX);
        let line = lines.next_line().await.unwrap().unwrap();
        assert!(
            line.starts_with("ok ") && line.ends_with(" bytes"),
            "{line:?}"
        );

        let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        let tail = drain_stderr_tail(text.as_bytes(), StderrPolicy::Silent, 1024).await;
        assert!(tail.len() <= 1024, "{}", tail.len());
        assert!(tail.ends_with("line 999"));
    }

    /// A·R3: an over-cap line is reported, not clipped, and never buffered
    /// past the cap, even while each read is cancelled mid-line the way
    /// `drive`'s `select!` cancels it; the lines around it still arrive.
    #[tokio::test]
    async fn checked_lines_drop_an_over_cap_line_without_buffering_it() {
        const MAX: usize = 1024;
        const LONG: usize = 4 * 1024 * 1024;
        const WAIT: Duration = Duration::from_millis(25);
        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
        let feeder = tokio::spawn(async move {
            writer.write_all(b"{\"before\":1}\n").await.unwrap();
            let chunk = vec![b'x'; 64 * 1024];
            for _ in 0..LONG / chunk.len() {
                writer.write_all(&chunk).await.unwrap();
            }
            let _ = go_rx.await;
            writer.write_all(b"\n{\"after\":2}\n").await.unwrap();
        });
        let mut lines = BoundedLines::new(reader, MAX);
        assert_eq!(
            lines.next_checked().await.unwrap(),
            Some(BoundedLine::Line("{\"before\":1}".to_string()))
        );
        // The long line has no newline yet: every read waits and is cancelled.
        let mut rounds = 0;
        while lines.seen < LONG as u64 {
            rounds += 1;
            assert!(rounds < 400, "only {} of {LONG} bytes read", lines.seen);
            let read = tokio::time::timeout(WAIT, lines.next_checked()).await;
            assert!(read.is_err(), "no line can end before its newline");
        }
        assert_eq!(lines.seen, LONG as u64, "a cancelled read loses no bytes");
        assert!(lines.line.len() <= MAX);
        assert!(
            lines.line.capacity() <= 2 * MAX,
            "buffered {} bytes of a {LONG}-byte line",
            lines.line.capacity()
        );
        go_tx.send(()).unwrap();
        assert_eq!(
            lines.next_checked().await.unwrap(),
            Some(BoundedLine::TooLong { bytes: LONG as u64 })
        );
        assert_eq!(
            lines.next_checked().await.unwrap(),
            Some(BoundedLine::Line("{\"after\":2}".to_string()))
        );
        feeder.await.unwrap();
        assert_eq!(lines.next_checked().await.unwrap(), None);
    }

    /// A·R3: one invalid UTF-8 byte on stdout used to end the read (strict
    /// `lines()`), and the post-EOF wait then killed the still-running agent.
    #[cfg(unix)]
    #[tokio::test]
    async fn drive_reads_past_invalid_utf8_on_stdout() {
        let (cmd, child) = spawn_isolated(
            r#"printf '{"a":1}\n\377 not utf-8\n'; /bin/sleep 0.5; printf '{"b":2}\n'"#,
            &[],
        );
        let guard = GroupGuard::adopt(&cmd, &child);
        let mut seen: Vec<String> = Vec::new();
        let driven = drive(child, guard, fast(), |line| seen.push(line.to_string())).await;
        assert_eq!(driven.stdout_error, None);
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(seen[0], r#"{"a":1}"#);
        assert_eq!(seen[1], "\u{FFFD} not utf-8");
        assert_eq!(seen[2], r#"{"b":2}"#);
        assert_eq!(driven.ended, Ended::Exited);
        assert!(
            matches!(driven.status, Some(Ok(status)) if status.success()),
            "the agent ran to its own exit"
        );
    }

    /// A·R3: a stdout line over [`STDOUT_LINE_MAX`] reaches `on_line` as the
    /// dropped-line event, never as the line or a clipped piece of it, and
    /// the next line still arrives.
    #[cfg(unix)]
    #[tokio::test]
    async fn drive_replaces_an_over_cap_stdout_line_with_a_dropped_event() {
        let long = STDOUT_LINE_MAX + 1000;
        let script =
            format!(r"echo first; head -c {long} /dev/zero | tr '\000' x; echo; echo after");
        let (cmd, child) = spawn_isolated(&script, &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let mut seen: Vec<String> = Vec::new();
        let driven = drive(child, guard, fast(), |line| seen.push(line.to_string())).await;
        let lengths: Vec<usize> = seen.iter().map(String::len).collect();
        assert_eq!(driven.stdout_error, None);
        assert_eq!(seen.len(), 3, "line lengths {lengths:?}");
        assert_eq!(seen[0], "first");
        assert_eq!(dropped_line_bytes(&seen[1]), Some(long as u64));
        assert_eq!(seen[2], "after");
        assert!(
            driven.stderr_tail.contains("dropped an agent stdout line"),
            "{:?}",
            driven.stderr_tail
        );
        assert!(matches!(driven.status, Some(Ok(status)) if status.success()));
    }

    #[tokio::test]
    async fn stop_signal_wait_is_race_free() {
        let stop = StopSignal::new();
        stop.request();
        tokio::time::timeout(Duration::from_millis(200), stop.wait())
            .await
            .expect("a request made before wait() is observed");
        assert!(stop.requested(), "a request stays set");

        let stop = Arc::new(StopSignal::new());
        assert!(!stop.requested());
        let waiter = {
            let stop = Arc::clone(&stop);
            tokio::spawn(async move { stop.wait().await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        stop.request();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("a request made during wait() wakes it")
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn killing_only_the_direct_child_orphans_the_grandchild() {
        // Control: this is the pre-PR-1 behavior (kill_on_drop / start_kill).
        let (_cmd, mut child) = spawn_plain("/bin/sleep 60 & echo $!; wait");
        let grandchild: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        child.start_kill().unwrap();
        child.wait().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let survived = is_running(grandchild).await;
        kill_now(grandchild);
        assert!(wait_gone(grandchild, Duration::from_secs(3)).await);
        assert!(
            survived,
            "SIGKILL of the direct child alone leaves the grandchild running"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_grandchild_forked_in_new_group() {
        let (cmd, child) = spawn_isolated("/bin/sleep 60 & echo $!; wait", &[]);
        let leader = child.id().unwrap() as i32;
        let guard = GroupGuard::adopt(&cmd, &child);
        assert!(guard.is_isolated());
        let stop = StopSignal::new();
        let mut seen: Vec<(i32, i32)> = Vec::new();
        let driven = drive(
            child,
            guard,
            Supervision {
                stop: Some(&stop),
                ..fast()
            },
            |line| {
                if let Ok(pid) = line.trim().parse::<i32>() {
                    seen.push((pid, pgid_of(pid)));
                }
                stop.request();
            },
        )
        .await;
        assert_eq!(driven.ended, Ended::Stopped);
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        let (grandchild, pgid) = seen[0];
        assert_eq!(pgid, leader, "the grandchild shares the leader's group");
        assert!(wait_gone(grandchild, Duration::from_secs(3)).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_escalates_to_sigkill_when_group_ignores_sigterm() {
        let (cmd, mut child) = spawn_isolated("trap '' TERM; /bin/sleep 60 & echo $!; wait", &[]);
        let grandchild: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let mut guard = GroupGuard::adopt(&cmd, &child);
        let t = Timings::fast_for_tests();
        let started = std::time::Instant::now();
        assert_eq!(
            guard.settle(&mut child, &t).await,
            Some(StopOutcome::Verified)
        );
        assert!(
            started.elapsed() >= t.grace,
            "SIGKILL waits out the grace period"
        );
        assert!(wait_gone(grandchild, Duration::from_secs(3)).await);
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "the leader was reaped"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn group_kill_reaches_nested_child_spawned_without_new_group() {
        // The team-mcp pattern: an inner process spawned WITHOUT a new group.
        let (cmd, mut child) = spawn_isolated(
            "trap '' TERM; /bin/sh -c 'trap \"\" TERM; /bin/sleep 60 & echo $!; wait' & wait",
            &[],
        );
        let leader = child.id().unwrap() as i32;
        let sleeper: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        assert_eq!(pgid_of(sleeper), leader);
        let mut guard = GroupGuard::adopt(&cmd, &child);
        assert_eq!(
            guard.settle(&mut child, &Timings::fast_for_tests()).await,
            Some(StopOutcome::Verified)
        );
        assert!(wait_gone(sleeper, Duration::from_secs(3)).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_sweeps_descendant_that_moved_into_its_own_group() {
        // The CLI pattern: the tool command leads a group of its own.
        let (cmd, mut child) = spawn_isolated("set -m; /bin/sleep 60 & echo $!; wait", &[]);
        let leader = child.id().unwrap() as i32;
        let sleeper: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while pgid_of(sleeper) != sleeper && Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(pgid_of(sleeper), sleeper);
        assert_ne!(sleeper, leader);
        let guard = GroupGuard::adopt(&cmd, &child);
        let stop = StopSignal::new();
        stop.request();
        let driven = drive(
            child,
            guard,
            Supervision {
                stop: Some(&stop),
                ..fast()
            },
            |_| {},
        )
        .await;
        assert_eq!(driven.ended, Ended::Stopped);
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        assert!(wait_gone(sleeper, Duration::from_secs(3)).await);
    }

    /// Paperclip PR 8b: a budget limit ends the run exactly like a stop, as
    /// its own `Ended` variant; a user stop requested at the same moment wins.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_limit_ends_the_run_like_a_stop_and_a_user_stop_wins_a_tie() {
        let (cmd, child) = spawn_isolated("echo $$; exec /bin/sleep 30", &[]);
        let leader = child.id().unwrap() as i32;
        let guard = GroupGuard::adopt(&cmd, &child);
        let limit = StopSignal::new();
        limit.request();
        let driven = drive(
            child,
            guard,
            Supervision {
                limit: Some(&limit),
                ..fast()
            },
            |_| {},
        )
        .await;
        assert_eq!(driven.ended, Ended::LimitReached);
        assert!(driven.outcome.is_some(), "the group was settled");
        assert!(wait_gone(leader, Duration::from_secs(3)).await);

        let (cmd, child) = spawn_isolated("exec /bin/sleep 30", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let (stop, limit) = (StopSignal::new(), StopSignal::new());
        stop.request();
        limit.request();
        let driven = drive(
            child,
            guard,
            Supervision {
                stop: Some(&stop),
                limit: Some(&limit),
                ..fast()
            },
            |_| {},
        )
        .await;
        assert_eq!(driven.ended, Ended::Stopped, "the user's stop wins a tie");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sampled_descendant_in_own_group_is_killed_after_parent_exits() {
        // A platform binary (env unreadable) detached into its own group,
        // reparented to launchd when its shell exits: only the start-time
        // record taken by sampling identifies it at finish.
        let (cmd, child) = spawn_isolated(
            "set -m; /bin/sleep 60 >/dev/null 2>&1 & echo $!; /bin/sleep 1; exit 0",
            &[],
        );
        let leader = child.id().unwrap() as i32;
        let guard = GroupGuard::adopt(&cmd, &child);
        let (mut sleeper, mut sleeper_pgid) = (0, 0);
        let driven = drive(child, guard, fast(), |line| {
            if let Ok(pid) = line.trim().parse::<i32>() {
                // Job control moves the job into its own group; allow the
                // shell a moment in case it has not yet.
                let until = std::time::Instant::now() + Duration::from_millis(500);
                while pgid_of(pid) != pid && std::time::Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(5));
                }
                sleeper = pid;
                sleeper_pgid = pgid_of(pid);
            }
        })
        .await;
        assert!(sleeper > 1);
        let gone = wait_gone(sleeper, Duration::from_secs(3)).await;
        if !gone {
            kill_now(sleeper);
        }
        // Otherwise the leader-group rule would find it and this test would
        // prove nothing about sampling.
        assert_eq!(sleeper_pgid, sleeper, "the job leads its own group");
        assert_ne!(sleeper_pgid, leader, "the job left the leader's group");
        assert_eq!(driven.ended, Ended::Exited);
        assert!(matches!(driven.status, Some(Ok(status)) if status.success()));
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        assert!(gone);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn finish_sweep_finds_tagged_orphan_that_left_group_and_parent() {
        let exe = self_exe();
        let (cmd, mut child) = spawn_isolated(
            TAGGED_ORPHAN_SCRIPT,
            &[
                ("HOLLOW_TEST_EXE", exe.as_str()),
                ("HOLLOW_TEST_SLEEP_SECS", "60"),
            ],
        );
        let orphan: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let mut guard = GroupGuard::adopt(&cmd, &child);
        assert!(wait_exec(orphan, &self_exe_name(), Duration::from_secs(10)).await);
        child.wait().await.unwrap(); // the leader exits at once
        tokio::time::sleep(Duration::from_millis(300)).await;
        let running = is_running(orphan).await;
        let own_group = pgid_of(orphan) == orphan;
        let outcome = guard.settle(&mut child, &Timings::fast_for_tests()).await;
        let gone = wait_gone(orphan, Duration::from_secs(3)).await;
        if !gone {
            kill_now(orphan);
        }
        assert!(running, "the helper must outlive its launching shell");
        assert!(own_group);
        assert_eq!(outcome, Some(StopOutcome::Verified));
        assert!(gone);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn normal_finish_sweeps_in_group_straggler() {
        let (cmd, child) = spawn_isolated("/bin/sleep 60 >/dev/null 2>&1 & echo $!", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let mut straggler = 0;
        let driven = drive(child, guard, fast(), |line| {
            if let Ok(pid) = line.trim().parse::<i32>() {
                straggler = pid;
            }
        })
        .await;
        assert_eq!(driven.ended, Ended::Exited);
        assert!(matches!(driven.status, Some(Ok(status)) if status.success()));
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        assert!(wait_gone(straggler, Duration::from_secs(3)).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drive_keeps_reading_after_stop_until_processes_exit() {
        let (cmd, child) = spawn_isolated(
            "trap 'echo bye; exit 0' TERM; echo ready; while :; do /bin/sleep 1; done",
            &[],
        );
        let guard = GroupGuard::adopt(&cmd, &child);
        let stop = StopSignal::new();
        let mut seen: Vec<String> = Vec::new();
        let driven = drive(
            child,
            guard,
            Supervision {
                stop: Some(&stop),
                ..fast()
            },
            |line| {
                if line == "ready" {
                    stop.request();
                }
                seen.push(line.to_string());
            },
        )
        .await;
        assert_eq!(seen, vec!["ready".to_string(), "bye".to_string()]);
        assert_eq!(driven.ended, Ended::Stopped);
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drive_sweeps_straggler_holding_stdout_after_leader_exit() {
        let (cmd, child) = spawn_isolated("/bin/sleep 60 & echo $!", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let started = std::time::Instant::now();
        let mut straggler = 0;
        let driven = drive(child, guard, fast(), |line| {
            if let Ok(pid) = line.trim().parse::<i32>() {
                straggler = pid;
            }
        })
        .await;
        // Far short of the straggler's 60 s sleep (and the 30 s deadline).
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(driven.ended, Ended::Exited);
        assert!(matches!(driven.status, Some(Ok(status)) if status.success()));
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        assert!(wait_gone(straggler, Duration::from_secs(3)).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drive_bounds_wait_when_stdout_closes_but_leader_hangs() {
        let (cmd, child) = spawn_isolated("exec 1>&-; /bin/sleep 60", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let started = std::time::Instant::now();
        let driven = drive(child, guard, fast(), |_| {}).await;
        // The cause: the post-EOF wait ended it (`Exited`, not the 30 s
        // deadline's `TimedOut`) while the leader still ran, and settle
        // killed it (not its own exit after the 60 s sleep).
        assert_eq!(driven.ended, Ended::Exited);
        assert!(!driven.leader_exited_first, "settle ended the leader");
        // Only rules out a hang.
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{:?}",
            started.elapsed()
        );
        assert!(matches!(driven.status, Some(Ok(status)) if !status.success()));
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
    }

    /// Paperclip PR 9b: a signal the leader died of before settle came from
    /// outside Hollow; one settle delivered is Hollow's own.
    #[cfg(unix)]
    #[tokio::test]
    async fn drive_reports_whether_the_leader_exited_before_the_stop_sequence() {
        use std::os::unix::process::ExitStatusExt;

        let (cmd, child) = spawn_isolated("kill -9 $$", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let driven = drive(child, guard, fast(), |_| {}).await;
        assert_eq!(driven.ended, Ended::Exited);
        assert!(driven.leader_exited_first, "the leader died before settle");
        let status = match driven.status {
            Some(Ok(status)) => status,
            other => panic!("expected an exit status, got {other:?}"),
        };
        assert_eq!(status.signal(), Some(libc::SIGKILL));

        // Stdout closes while the leader keeps running: the loop ends first
        // and settle's own SIGTERM is what ends it.
        let (cmd, child) = spawn_isolated("exec 1>&-; /bin/sleep 30", &[]);
        let guard = GroupGuard::adopt(&cmd, &child);
        let driven = drive(child, guard, fast(), |_| {}).await;
        assert_eq!(driven.ended, Ended::Exited);
        assert!(!driven.leader_exited_first, "settle ended the leader");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drive_honors_stop_requested_before_it_starts() {
        let (cmd, child) = spawn_isolated("/bin/sleep 60 & echo $!; wait", &[]);
        let leader = child.id().unwrap() as i32;
        let guard = GroupGuard::adopt(&cmd, &child);
        let stop = StopSignal::new();
        stop.request();
        let driven = drive(
            child,
            guard,
            Supervision {
                stop: Some(&stop),
                ..fast()
            },
            |_| {},
        )
        .await;
        assert_eq!(driven.ended, Ended::Stopped);
        assert_eq!(driven.outcome, Some(StopOutcome::Verified));
        assert!(!group_alive(leader));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_armed_guard_sigkills_group_and_sweeps_tag() {
        // (a) The synchronous group SIGKILL, even when TERM is ignored.
        let (cmd, mut child) = spawn_isolated("trap '' TERM; /bin/sleep 60 & echo $!; wait", &[]);
        let grandchild: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let guard = GroupGuard::adopt(&cmd, &child);
        drop(guard);
        drop(child);
        assert!(wait_gone(grandchild, Duration::from_secs(3)).await);

        // (b) The best-effort async sweep finds a tagged orphan by its tag.
        let exe = self_exe();
        let (cmd, mut child) = spawn_isolated(
            TAGGED_ORPHAN_SCRIPT,
            &[
                ("HOLLOW_TEST_EXE", exe.as_str()),
                ("HOLLOW_TEST_SLEEP_SECS", "60"),
            ],
        );
        let orphan: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let guard = GroupGuard::adopt(&cmd, &child);
        assert!(wait_exec(orphan, &self_exe_name(), Duration::from_secs(10)).await);
        child.wait().await.unwrap();
        drop(guard);
        let gone = wait_gone(orphan, Duration::from_secs(5)).await;
        if !gone {
            kill_now(orphan);
        }
        assert!(gone);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_scan_is_never_verified() {
        let (cmd, mut child) = spawn_isolated("/bin/sleep 60 & echo $!; wait", &[]);
        let grandchild: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let mut guard = GroupGuard::adopt(&cmd, &child).failing_scans();
        assert_eq!(
            guard.settle(&mut child, &Timings::fast_for_tests()).await,
            Some(StopOutcome::Unconfirmed)
        );
        // The leader-group signal does not depend on a scan.
        assert!(wait_gone(grandchild, Duration::from_secs(3)).await);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn guard_never_signals_invalid_or_own_group() {
        // Signal 0 only probes, so a regression here cannot hurt the suite.
        for pgid in [0, 1, -5, own_pgrp()] {
            assert!(!signal_group(pgid, 0), "pgid {pgid}");
        }
        for pid in [0, 1, own_pid()] {
            assert!(!signal_pid(pid, 0), "pid {pid}");
        }

        let (cmd, mut child) = spawn_plain("/bin/sleep 60 & echo $!; wait");
        let grandchild: i32 = read_line(&mut child, Duration::from_secs(5))
            .await
            .parse()
            .unwrap();
        let mut guard = GroupGuard::adopt(&cmd, &child);
        assert!(!guard.is_isolated());
        assert_eq!(
            guard.settle(&mut child, &Timings::fast_for_tests()).await,
            None
        );
        assert!(
            matches!(child.try_wait(), Ok(Some(_))),
            "the direct child was killed"
        );
        let survived = is_running(grandchild).await;
        kill_now(grandchild);
        assert!(wait_gone(grandchild, Duration::from_secs(3)).await);
        assert!(
            survived,
            "a non-isolated guard never signals beyond the direct child"
        );

        let (cmd, mut child) = spawn_isolated("exit 0", &[]);
        child.wait().await.unwrap();
        assert!(child.id().is_none());
        assert!(!GroupGuard::adopt(&cmd, &child).is_isolated());
    }

    /// The plaud / codex_transform / run_admin path: a timeout ends the CLI
    /// and the grandchild still holding its stdout, instead of dropping a
    /// `wait_with_output()` future and leaving the grandchild running.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_to_completion_timeout_ends_group_holding_stdout() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("/bin/sleep 60 & echo $!; wait");
        let started = std::time::Instant::now();
        let output = run_to_completion(&mut cmd, None, Duration::from_millis(300), None)
            .await
            .expect("spawn");
        let grandchild: i32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap();
        let gone = wait_gone(grandchild, Duration::from_secs(3)).await;
        if !gone {
            kill_now(grandchild);
        }
        assert!(output.timed_out);
        assert!(!output.stopped);
        assert_eq!(output.outcome, Some(StopOutcome::Verified));
        assert!(gone, "the grandchild outlived the timeout");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    /// The Codex text-ingest cancel path: a stop request ends the CLI and its
    /// grandchild before the call returns, instead of the caller dropping
    /// the future and releasing the lease while only Drop's sweep runs.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_to_completion_stop_ends_group_before_returning() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("/bin/sleep 60 & echo $!; wait");
        let stop = Arc::new(StopSignal::new());
        let requester = {
            let stop = Arc::clone(&stop);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                stop.request();
            })
        };
        let output = run_to_completion(&mut cmd, None, Duration::from_secs(30), Some(&stop))
            .await
            .expect("spawn");
        requester.await.unwrap();
        let grandchild: i32 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .unwrap();
        let gone = wait_gone(grandchild, Duration::from_millis(200)).await;
        if !gone {
            kill_now(grandchild);
        }
        assert!(output.stopped);
        assert!(!output.timed_out);
        assert_eq!(output.outcome, Some(StopOutcome::Verified));
        assert!(gone, "the grandchild must be gone when the call returns");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_to_completion_feeds_input_and_captures_output() {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("cat; echo oops >&2");
        let output = run_to_completion(&mut cmd, Some(b"hello\n"), Duration::from_secs(10), None)
            .await
            .expect("spawn");
        assert!(!output.timed_out);
        assert!(!output.stopped);
        assert!(output.input_error.is_none());
        assert!(output.status.is_some_and(|status| status.success()));
        assert_eq!(output.stdout, b"hello\n");
        assert_eq!(output.stderr, "oops");
        assert_eq!(output.outcome, Some(StopOutcome::Verified));
        assert!(!output.stdout_truncated);
    }

    /// Paperclip PR 16: a capped read stops at the cap and closes the pipe,
    /// so an endless writer dies of SIGPIPE promptly (its status is then a
    /// failure, which is why callers check `stdout_truncated` first). The
    /// uncapped wrapper still returns the whole output.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_to_completion_capped_stops_reading_at_the_cap() {
        let mut cmd = Command::new("/usr/bin/yes");
        let started = std::time::Instant::now();
        let output = run_to_completion_capped(&mut cmd, None, Duration::from_secs(5), None, 4096)
            .await
            .expect("spawn");
        assert_eq!(output.stdout.len(), 4096);
        assert!(output.stdout_truncated);
        assert!(!output.timed_out, "the pipe close ended it, not the limit");
        assert!(!output.status.is_some_and(|status| status.success()));
        // `timed_out` above is the cause; this only rules out a hang.
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "{:?}",
            started.elapsed()
        );

        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg("/usr/bin/yes | /usr/bin/head -c 20000");
        let output = run_to_completion(&mut cmd, None, Duration::from_secs(10), None)
            .await
            .expect("spawn");
        assert_eq!(output.stdout.len(), 20000);
        assert!(!output.stdout_truncated);
        assert!(output.status.is_some_and(|status| status.success()));
    }

    /// The synchronous `codex mcp list` probe pattern: the probe leads its own
    /// group, so ending that group reaches a grandchild still holding the
    /// probe's stdout, and the reader then sees EOF instead of blocking.
    #[cfg(unix)]
    #[tokio::test]
    async fn probe_group_end_reaches_grandchild_holding_stdout() {
        use std::io::{BufRead, Read};
        let mut probe = std::process::Command::new("/bin/sh");
        probe
            .arg("-c")
            .arg("/bin/sleep 60 & echo $!; wait")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        isolate_probe(&mut probe);
        let mut child = probe.spawn().expect("spawn probe");
        let pid = child.id();
        assert_eq!(
            pgid_of(pid as i32),
            pid as i32,
            "the probe leads its own group"
        );
        let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let grandchild: i32 = line.trim().parse().unwrap();
        end_probe_group(pid);
        let _ = child.wait();
        let gone = wait_gone(grandchild, Duration::from_secs(3)).await;
        if !gone {
            kill_now(grandchild);
        }
        assert!(gone);
        // Every writer is gone, so this reaches EOF rather than blocking.
        let mut rest = Vec::new();
        assert!(stdout.read_to_end(&mut rest).is_ok());
        // Idempotent once the group is gone.
        end_probe_group(pid);
    }

    #[test]
    fn combined_outcome_is_the_weaker_step() {
        use StopOutcome::{Unconfirmed, Verified};
        assert_eq!(StopOutcome::combine(None, None), None);
        assert_eq!(StopOutcome::combine(Some(Verified), None), Some(Verified));
        assert_eq!(StopOutcome::combine(None, Some(Verified)), Some(Verified));
        assert_eq!(
            StopOutcome::combine(Some(Verified), Some(Verified)),
            Some(Verified)
        );
        assert_eq!(
            StopOutcome::combine(Some(Verified), Some(Unconfirmed)),
            Some(Unconfirmed)
        );
        assert_eq!(
            StopOutcome::combine(Some(Unconfirmed), None),
            Some(Unconfirmed)
        );
    }
}
