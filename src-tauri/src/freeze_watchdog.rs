//! UI freeze watchdog (overnight 11-17, row OC10): evidence when Hollow stops
//! answering.
//!
//! A frozen window used to leave nothing behind: the only watchdogs were for
//! agent turns. Now a plain std thread ticks once a second and asks the main
//! (AppKit) thread to stamp a heartbeat ([`MainThreadBeat`], through
//! `AppHandle::run_on_main_thread`, so no Tauri command and no frontend code).
//! When the stamp is older than [`THRESHOLD_MS`] it runs macOS
//! `/usr/bin/sample <own pid> 3 20 -mayDie -file <tmp>`, reduces the report
//! to a few stack lines and writes one note under
//! `{app data}/diagnostics/freeze-<UTC stamp>.txt`. When the heartbeat comes
//! back the same note gets a line saying how long the stall lasted. Newest
//! [`KEEP_NOTES`] notes are kept. The folder is 0700 and each note 0600; at
//! most one sample per [`MIN_SAMPLE_GAP_MS`] (a hang inside it still gets a
//! note, saying so); nothing starts once the app is quitting
//! ([`note_exit`]); the raw report's temp file is swept at start and with
//! every prune, since a force-quit mid-sample would leave it unredacted. The note is for support: no UI, no
//! notification; the line also goes to stderr through [`crate::launch::elog`].
//!
//! **Why the main thread, not the webview.** `sample <pid>` captures this
//! process, so the evidence is only useful when this process is the one that
//! is stuck: a blocked AppKit main thread is the beach ball. A page stuck in
//! a script loop runs in WebKit's separate content process, which a sample of
//! this pid would not show (it would need that child's pid; not done). The
//! main-thread beat also needs no registered command, so it adds nothing to
//! the route ledger or the command map.
//!
//! **Sleep and startup.** The wall clock is read every tick. A jump of more
//! than [`SLEEP_GAP_MS`] since the previous tick (the machine slept, the
//! clock was set, or this thread was starved) re-baselines everything and
//! reports nothing. Nothing is judged until the first heartbeat has arrived,
//! so a long launch is not a hang. The sample's own 3+ seconds are not a gap
//! ([`Monitor::resync`]), and one hang gives one note: it re-arms only after
//! a beat returns.
//!
//! **Redaction.** The home folder becomes `~` and any other `/Users/<name>`
//! becomes `/Users/<user>`. Only thread, symbol and library names are kept
//! (the release binary is stripped, so Hollow's own frames are addresses).
//! Nothing from the vault or a chat is ever read.
//!
//! Tests inject [`Clock`], [`Beat`] and [`Sampler`], so none of them sleeps
//! or runs `sample`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How often the watchdog thread wakes.
pub(crate) const TICK: Duration = Duration::from_secs(1);
/// A heartbeat this old is a hang.
pub(crate) const THRESHOLD_MS: u64 = 8_000;
/// A gap this long between two of the watchdog's own ticks is not a hang.
pub(crate) const SLEEP_GAP_MS: u64 = 3_000;
/// Notes kept in the diagnostics folder, newest first.
pub(crate) const KEEP_NOTES: usize = 20;
/// Seconds and interval (ms) handed to `sample`.
const SAMPLE_SECS: &str = "3";
const SAMPLE_INTERVAL_MS: &str = "20";
/// `sample` is killed (by its handle) when it runs longer than this.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(10);
/// Most bytes read back from the sample file.
const SAMPLE_READ_CAP: u64 = 16 * 1024 * 1024;
/// Leaf-most frames kept from the main thread's heaviest stack.
const MAX_FRAMES: usize = 14;
/// Lines kept from the "sort by top of stack" summary.
const MAX_TOP: usize = 6;
/// Longest kept line, and longest note, in bytes.
const MAX_LINE: usize = 200;
const MAX_NOTE: usize = 6 * 1024;
const NOTE_PREFIX: &str = "freeze-";
/// The raw report's temp file: `.sample-<pid>.tmp` (never redacted, so it is
/// removed before every run and swept at start and with each prune).
const SAMPLE_PREFIX: &str = ".sample-";
/// At least this long between two samples, so a flapping app does not keep
/// `sample` (and its CPU) running.
pub(crate) const MIN_SAMPLE_GAP_MS: u64 = 60_000;

/// Set when the app is quitting (`RunEvent::Exit`): nothing new starts.
static EXITING: AtomicBool = AtomicBool::new(false);

/// Called from `RunEvent::Exit`: no sample or note starts after this.
pub(crate) fn note_exit() {
    EXITING.store(true, Ordering::SeqCst);
}

/// Whether `HOLLOW_FREEZE_WATCHDOG` turns the watchdog off: `0`, `off`,
/// `false` or `no`, any case.
pub(crate) fn disabled_by(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        )
    })
}

/// Wall-clock milliseconds since the Unix epoch (injected in tests).
pub(crate) trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// The liveness probe: [`Beat::ping`] asks the thread being watched to stamp
/// the time; [`Beat::last_ms`] is the newest stamp, `None` before the first.
pub(crate) trait Beat: Send + Sync {
    fn ping(&self);
    fn last_ms(&self) -> Option<u64>;
}

/// Produces a raw `sample` report for a pid (injected in tests).
pub(crate) trait Sampler: Send + Sync {
    fn sample(&self, pid: u32) -> Result<String, String>;
}

// ── The decision ─────────────────────────────────────────────────────────

/// What one tick decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Nothing to report (not armed, healthy, or still inside one hang).
    Idle,
    /// The clock jumped: re-baselined, nothing reported.
    Slept,
    /// The heartbeat has been missing for `stalled_ms`; sample now.
    Hang { stalled_ms: u64 },
    /// The heartbeat is back after a hang that lasted `stalled_ms`.
    Recovered { stalled_ms: u64 },
}

/// The pure state machine: times in, verdict out.
#[derive(Debug)]
pub(crate) struct Monitor {
    threshold_ms: u64,
    gap_ms: u64,
    last_tick: Option<u64>,
    /// After a clock jump, a beat older than this does not count against us.
    rebase: u64,
    /// The last beat before the current hang.
    hung_since: Option<u64>,
}

impl Monitor {
    pub(crate) fn new(threshold_ms: u64, gap_ms: u64) -> Self {
        Self {
            threshold_ms,
            gap_ms,
            last_tick: None,
            rebase: 0,
            hung_since: None,
        }
    }

    /// Judge one tick at wall time `now` given the newest beat.
    pub(crate) fn step(&mut self, now: u64, beat: Option<u64>) -> Step {
        let previous = self.last_tick.replace(now);
        let jumped = previous.is_some_and(|p| now < p || now - p > self.gap_ms);
        if jumped {
            self.rebase = now;
            self.hung_since = None;
            return Step::Slept;
        }
        let Some(beat) = beat else {
            return Step::Idle;
        };
        let alive_at = beat.max(self.rebase);
        match self.hung_since {
            Some(since) if beat > since => {
                self.hung_since = None;
                Step::Recovered {
                    stalled_ms: beat.saturating_sub(since),
                }
            }
            Some(_) => Step::Idle,
            None => {
                let stalled_ms = now.saturating_sub(alive_at);
                if stalled_ms >= self.threshold_ms {
                    self.hung_since = Some(alive_at);
                    Step::Hang { stalled_ms }
                } else {
                    Step::Idle
                }
            }
        }
    }

    /// The caller spent time on its own (the sample): that is not a sleep.
    pub(crate) fn resync(&mut self, now: u64) {
        self.last_tick = Some(now);
    }
}

// ── The report reducer ───────────────────────────────────────────────────

fn users_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"/Users/[^/\s)\]]+").expect("static regex"))
}

/// The home folder as `~` and any other user folder as `/Users/<user>`.
pub(crate) fn redact(text: &str, home: &str) -> String {
    let replaced = if home.len() > 1 {
        text.replace(home, "~")
    } else {
        text.to_string()
    };
    users_re()
        .replace_all(&replaced, "/Users/<user>")
        .into_owned()
}

fn clip(line: &str, max: usize) -> String {
    if line.len() <= max {
        return line.to_string();
    }
    let mut end = max;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &line[..end])
}

/// A call-graph line: (column of its count, count, text after the count).
fn graph_line(line: &str) -> Option<(usize, u64, &str)> {
    let col = line.find(|c: char| !matches!(c, ' ' | '+' | '!' | ':' | '|'))?;
    let rest = &line[col..];
    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits == 0 {
        return None;
    }
    let count = rest[..digits].parse().ok()?;
    Some((col, count, rest[digits..].trim()))
}

/// The main thread's heaviest stack (outermost first) and the summary of
/// leaf functions, from a raw `sample` report. Empty text when the report has
/// no call graph.
pub(crate) fn reduce(raw: &str, home: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let ends = |l: &str| {
        let t = l.trim_start();
        t.starts_with("Total number in stack")
            || t.starts_with("Sort by top of stack")
            || t.starts_with("Binary Images")
    };
    let mut out = String::new();
    if let Some(start) = lines
        .iter()
        .position(|l| l.trim_start().starts_with("Call graph:"))
    {
        let graph: Vec<(usize, u64, &str)> = lines[start + 1..]
            .iter()
            .take_while(|l| !ends(l))
            .filter_map(|l| graph_line(l))
            .collect();
        let top_col = graph.iter().map(|g| g.0).min();
        let headers: Vec<usize> = graph
            .iter()
            .enumerate()
            .filter(|(_, g)| Some(g.0) == top_col)
            .map(|(i, _)| i)
            .collect();
        let main = headers
            .iter()
            .copied()
            .find(|&i| graph[i].2.contains("main-thread"))
            .or_else(|| headers.first().copied());
        if let Some(h) = main {
            let (head_col, head_count, head_text) = graph[h];
            let mut path = Vec::new();
            let mut prev = head_col;
            for &(col, count, text) in &graph[h + 1..] {
                if col <= prev {
                    break;
                }
                path.push((count, text));
                prev = col;
            }
            out.push_str(&format!(
                "Main thread, {head_count} samples: {}\nHeaviest stack, outermost first:\n",
                clip(head_text, MAX_LINE)
            ));
            let skip = path.len().saturating_sub(MAX_FRAMES);
            if skip > 0 {
                out.push_str(&format!("  ... {skip} outer frames omitted\n"));
            }
            for (count, text) in &path[skip..] {
                out.push_str(&format!("  {count} {}\n", clip(text, MAX_LINE)));
            }
        }
    }
    if let Some(start) = lines
        .iter()
        .position(|l| l.trim_start().starts_with("Sort by top of stack"))
    {
        out.push_str("Top of stack, all threads:\n");
        for line in lines[start + 1..]
            .iter()
            .map(|l| l.trim())
            .skip_while(|l| l.is_empty())
            .take_while(|l| !l.is_empty())
            .take(MAX_TOP)
        {
            out.push_str(&format!("  {}\n", clip(line, MAX_LINE)));
        }
    }
    redact(&out, home)
}

fn utc_stamp(ms: u64, fmt: &str) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| t.format(fmt).to_string())
        .unwrap_or_else(|| "unknown time".into())
}

fn seconds(ms: u64) -> u64 {
    (ms + 500) / 1000
}

/// The note text for one hang: header, then the reduced stacks.
pub(crate) fn note_text(stalled_ms: u64, at_ms: u64, stacks: &str) -> String {
    let mut text = format!(
        "Hollow was unresponsive for at least {} s.\nDetected: {} (UTC)\nBuild: {}\n\n{}",
        seconds(stalled_ms),
        utc_stamp(at_ms, "%Y-%m-%dT%H:%M:%SZ"),
        env!("CARGO_PKG_VERSION"),
        if stacks.trim().is_empty() {
            "No stack sample was available.\n"
        } else {
            stacks
        },
    );
    if text.len() > MAX_NOTE {
        let mut end = MAX_NOTE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('\n');
    }
    text
}

// ── Notes on disk ────────────────────────────────────────────────────────

fn write_note(dir: &Path, at_ms: u64, text: &str) -> std::io::Result<PathBuf> {
    private_dir(dir)?;
    let stamp = utc_stamp(at_ms, "%Y%m%dT%H%M%SZ");
    let mut path = dir.join(format!("{NOTE_PREFIX}{stamp}.txt"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{NOTE_PREFIX}{stamp}-{n}.txt"));
        n += 1;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path)?.write_all(text.as_bytes())?;
    prune(dir, KEEP_NOTES);
    Ok(path)
}

/// Delete the oldest of our own notes beyond `keep` (names sort by time).
fn prune(dir: &Path, keep: usize) {
    sweep_samples(dir);
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut notes: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(NOTE_PREFIX) && n.ends_with(".txt"))
        })
        .collect();
    notes.sort();
    let extra = notes.len().saturating_sub(keep);
    for old in notes.into_iter().take(extra) {
        let _ = std::fs::remove_file(old);
    }
}

/// Remove every leftover raw report (`.sample-*.tmp`): one is orphaned,
/// unredacted, when the app is force-quit while `sample` runs.
pub(crate) fn sweep_samples(dir: &Path) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for path in read.filter_map(|e| e.ok().map(|e| e.path())) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(SAMPLE_PREFIX) && name.ends_with(".tmp") {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn append(path: &Path, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

// ── The watchdog ─────────────────────────────────────────────────────────

/// What a tick did, for the log line and the tests.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Event {
    None,
    Slept,
    Noted(PathBuf),
    Recovered { seconds: u64 },
}

pub(crate) struct Watchdog {
    clock: Box<dyn Clock>,
    beat: Box<dyn Beat>,
    sampler: Box<dyn Sampler>,
    dir: PathBuf,
    home: String,
    pid: u32,
    monitor: Monitor,
    open_note: Option<PathBuf>,
    last_sample_ms: Option<u64>,
    exiting: &'static AtomicBool,
}

impl Watchdog {
    pub(crate) fn new(
        clock: Box<dyn Clock>,
        beat: Box<dyn Beat>,
        sampler: Box<dyn Sampler>,
        dir: PathBuf,
        home: String,
        pid: u32,
    ) -> Self {
        Self {
            clock,
            beat,
            sampler,
            dir,
            home,
            pid,
            monitor: Monitor::new(THRESHOLD_MS, SLEEP_GAP_MS),
            open_note: None,
            last_sample_ms: None,
            exiting: &EXITING,
        }
    }

    /// Use another quit flag (tests; the app uses [`note_exit`]).
    #[cfg(test)]
    pub(crate) fn with_exit_flag(mut self, flag: &'static AtomicBool) -> Self {
        self.exiting = flag;
        self
    }

    /// One pass: judge the heartbeat as it stands, then ask for a new one.
    pub(crate) fn tick(&mut self) -> Event {
        if self.exiting.load(Ordering::SeqCst) {
            return Event::None;
        }
        let now = self.clock.now_ms();
        let step = self.monitor.step(now, self.beat.last_ms());
        self.beat.ping();
        match step {
            Step::Idle => Event::None,
            Step::Slept => {
                self.open_note = None;
                Event::Slept
            }
            Step::Hang { stalled_ms } => {
                let recent = self
                    .last_sample_ms
                    .filter(|&t| now.saturating_sub(t) < MIN_SAMPLE_GAP_MS);
                let stacks = if let Some(t) = recent {
                    format!(
                        "No stack sample: the last one was {} s ago (one at most every {} s).\n",
                        seconds(now.saturating_sub(t)),
                        MIN_SAMPLE_GAP_MS / 1000
                    )
                } else {
                    let got = self.sampler.sample(self.pid);
                    self.last_sample_ms = Some(self.clock.now_ms());
                    match got {
                        Ok(raw) => reduce(&raw, &self.home),
                        Err(why) => redact(&format!("Sampling failed: {why}\n"), &self.home),
                    }
                };
                self.monitor.resync(self.clock.now_ms());
                // A quit that began during the sample writes nothing.
                if self.exiting.load(Ordering::SeqCst) {
                    return Event::None;
                }
                match write_note(&self.dir, now, &note_text(stalled_ms, now, &stacks)) {
                    Ok(path) => {
                        crate::launch::elog(&format!(
                            "[freeze-watchdog] unresponsive for {} s, note {}",
                            seconds(stalled_ms),
                            path.display()
                        ));
                        self.open_note = Some(path.clone());
                        Event::Noted(path)
                    }
                    Err(why) => {
                        crate::launch::elog(&format!("[freeze-watchdog] cannot write note: {why}"));
                        Event::None
                    }
                }
            }
            Step::Recovered { stalled_ms } => {
                if let Some(path) = self.open_note.take() {
                    append(
                        &path,
                        &format!(
                            "\nRecovered: Hollow answered again after about {} s.",
                            seconds(stalled_ms)
                        ),
                    );
                }
                Event::Recovered {
                    seconds: seconds(stalled_ms),
                }
            }
        }
    }
}

// ── The real pieces ──────────────────────────────────────────────────────

struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Stamps the time from the main thread. One ping in flight at a time, so a
/// blocked main thread does not pile up a queue of closures.
struct MainThreadBeat {
    app: tauri::AppHandle,
    last: Arc<AtomicU64>,
    pending: Arc<AtomicBool>,
}

impl Beat for MainThreadBeat {
    fn ping(&self) {
        if self.pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let (last, pending) = (self.last.clone(), self.pending.clone());
        let queued = self.app.run_on_main_thread({
            let (last, pending) = (last, pending.clone());
            move || {
                last.store(SystemClock.now_ms(), Ordering::Release);
                pending.store(false, Ordering::Release);
            }
        });
        if queued.is_err() {
            self.pending.store(false, Ordering::Release);
        }
    }

    fn last_ms(&self) -> Option<u64> {
        match self.last.load(Ordering::Acquire) {
            0 => None,
            ms => Some(ms),
        }
    }
}

/// What the sample wait needs from the child process (a fake in tests).
pub(crate) trait Reaped {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus>;
}

impl Reaped for std::process::Child {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        std::process::Child::try_wait(self)
    }
    fn kill(&mut self) -> std::io::Result<()> {
        std::process::Child::kill(self)
    }
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        std::process::Child::wait(self)
    }
}

/// Wait for `child` at most `timeout` (by `elapsed`), pausing with `pause`
/// between looks. On an overrun, a failed look, or a quit, the child is
/// killed (by its handle, never by name) and reaped, so none is left behind.
pub(crate) fn wait_bounded(
    child: &mut dyn Reaped,
    timeout: Duration,
    elapsed: &dyn Fn() -> Duration,
    pause: &dyn Fn(),
    quitting: &dyn Fn() -> bool,
) -> Result<std::process::ExitStatus, String> {
    let stop = |child: &mut dyn Reaped, why: String| {
        let _ = child.kill();
        let _ = child.wait();
        Err(why)
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if quitting() => return stop(child, "the app is quitting".into()),
            Ok(None) if elapsed() > timeout => {
                return stop(
                    child,
                    format!("sample ran over {} s and was stopped", timeout.as_secs()),
                )
            }
            Ok(None) => pause(),
            Err(e) => return stop(child, e.to_string()),
        }
    }
}

/// Owner-only folder (0700) for the notes and the raw report.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Runs `/usr/bin/sample` on a pid and reads its report back from a
/// temporary file in the (0700) diagnostics folder, removed before and after.
struct SampleCommand {
    dir: PathBuf,
    exiting: &'static AtomicBool,
}

impl Sampler for SampleCommand {
    fn sample(&self, pid: u32) -> Result<String, String> {
        private_dir(&self.dir).map_err(|e| e.to_string())?;
        let file = self.dir.join(format!("{SAMPLE_PREFIX}{pid}.tmp"));
        let _ = std::fs::remove_file(&file);
        let mut child = std::process::Command::new("/usr/bin/sample")
            .arg(pid.to_string())
            .arg(SAMPLE_SECS)
            .arg(SAMPLE_INTERVAL_MS)
            .arg("-mayDie")
            .arg("-file")
            .arg(&file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start sample: {e}"))?;
        let started = Instant::now();
        let status = wait_bounded(
            &mut child,
            SAMPLE_TIMEOUT,
            &|| started.elapsed(),
            &|| std::thread::sleep(Duration::from_millis(100)),
            &|| self.exiting.load(Ordering::SeqCst),
        );
        let mut text = String::new();
        if let Ok(f) = std::fs::File::open(&file) {
            let mut bytes = Vec::new();
            let _ = f.take(SAMPLE_READ_CAP).read_to_end(&mut bytes);
            text = String::from_utf8_lossy(&bytes).into_owned();
        }
        let _ = std::fs::remove_file(&file);
        match status {
            Ok(s) if s.success() && !text.is_empty() => Ok(text),
            Ok(s) => Err(format!("sample exited with {s}")),
            Err(e) if text.is_empty() => Err(e),
            // Overran but left a partial report: keep what it wrote.
            Err(_) => Ok(text),
        }
    }
}

/// Start the watchdog thread (GUI only: `hollow serve` has no main loop).
/// `HOLLOW_FREEZE_WATCHDOG` = `0`, `off`, `false` or `no` turns it off.
pub(crate) fn start(app: tauri::AppHandle) {
    if disabled_by(std::env::var("HOLLOW_FREEZE_WATCHDOG").ok().as_deref()) {
        return;
    }
    let dir = match crate::agent_settings::app_data_dir() {
        Ok(dir) => dir.join("diagnostics"),
        Err(why) => {
            crate::launch::elog(&format!("[freeze-watchdog] no app data folder: {why}"));
            return;
        }
    };
    // A raw report left by a force-quit mid-sample is unredacted: remove it.
    sweep_samples(&dir);
    let home = dirs::home_dir()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    let spawned = std::thread::Builder::new()
        .name("freeze-watchdog".into())
        .spawn(move || {
            let mut dog = Watchdog::new(
                Box::new(SystemClock),
                Box::new(MainThreadBeat {
                    app,
                    last: Arc::new(AtomicU64::new(0)),
                    pending: Arc::new(AtomicBool::new(false)),
                }),
                Box::new(SampleCommand {
                    dir: dir.clone(),
                    exiting: &EXITING,
                }),
                dir,
                home,
                std::process::id(),
            );
            loop {
                std::thread::sleep(TICK);
                dog.tick();
            }
        });
    if let Err(why) = spawned {
        crate::launch::elog(&format!("[freeze-watchdog] cannot start: {why}"));
    }
}

#[cfg(test)]
#[path = "freeze_watchdog_tests.rs"]
mod tests;
