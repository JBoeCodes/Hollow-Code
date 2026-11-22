use super::*;
use std::sync::Mutex;

const T0: u64 = 1_800_000_000_000;

#[derive(Clone)]
struct FakeClock(Arc<AtomicU64>);

impl FakeClock {
    fn at(ms: u64) -> Self {
        Self(Arc::new(AtomicU64::new(ms)))
    }
    fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.now()
    }
}

/// A heartbeat the test stamps by hand (0 = none yet).
#[derive(Clone, Default)]
struct FakeBeat(Arc<AtomicU64>);

impl FakeBeat {
    fn stamp(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}

impl Beat for FakeBeat {
    fn ping(&self) {}
    fn last_ms(&self) -> Option<u64> {
        match self.0.load(Ordering::SeqCst) {
            0 => None,
            ms => Some(ms),
        }
    }
}

/// Counts calls, returns a canned report, and can spend fake time like the
/// real one does.
#[derive(Clone)]
struct FakeSampler {
    calls: Arc<Mutex<Vec<u32>>>,
    report: Result<String, String>,
    spends: Option<(FakeClock, u64)>,
}

impl FakeSampler {
    fn new(report: Result<String, String>) -> Self {
        Self {
            calls: Arc::default(),
            report,
            spends: None,
        }
    }
    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl Sampler for FakeSampler {
    fn sample(&self, pid: u32) -> Result<String, String> {
        self.calls.lock().unwrap().push(pid);
        if let Some((clock, ms)) = &self.spends {
            clock.advance(*ms);
        }
        self.report.clone()
    }
}

/// A report in `sample`'s real layout (cut down): the main thread blocked in
/// a file read, an idle worker, and a path with the home folder in it.
fn report() -> String {
    "\
Analysis of sampling hollow (pid 3634) every 20 milliseconds
Process:         hollow [3634]
Path:            /Users/dev/Applications/Hollow.app/Contents/MacOS/hollow

Call graph:
    150 Thread_1   DispatchQueue_1: com.apple.main-thread  (serial)
    + 150 start  (in dyld) + 6688  [0x1836c7e80]
    +   150 ???  (in hollow)  load address 0x1020a0000 + 0x8c7c  [0x1020a8c7c]
    +     150 -[NSApplication run]  (in AppKit) + 396  [0x18810790c]
    +       150 read_vault_note  (in /Users/dev/Hollow/Vault/lib.dylib) + 8  [0x1]
    +         150 read  (in libsystem_kernel.dylib) + 8  [0x2]
    +       3 other_branch  (in hollow) + 4  [0x3]
    150 Thread_2
    + 150 start_wqthread  (in libsystem_pthread.dylib) + 8  [0x4]
    +   150 __workq_kernreturn  (in libsystem_kernel.dylib) + 8  [0x5]

Total number in stack (recursive counted multiple, when >=5):

Sort by top of stack, same collapsed (when >= 5):
        read  (in libsystem_kernel.dylib)        150
        __workq_kernreturn  (in libsystem_kernel.dylib)        150

Binary Images:
       0x1020a0000 -        0x1020a0a03 +hollow (1) <AAAA> /Users/dev/Applications/Hollow.app/Contents/MacOS/hollow
"
    .to_string()
}

struct Rig {
    dog: Watchdog,
    clock: FakeClock,
    beat: FakeBeat,
    sampler: FakeSampler,
    dir: tempfile::TempDir,
}

fn rig_with(sampler: FakeSampler, clock: FakeClock) -> Rig {
    let beat = FakeBeat::default();
    let dir = tempfile::tempdir().unwrap();
    let dog = Watchdog::new(
        Box::new(clock.clone()),
        Box::new(beat.clone()),
        Box::new(sampler.clone()),
        dir.path().join("diagnostics"),
        "/Users/dev".into(),
        4242,
    );
    Rig {
        dog,
        clock,
        beat,
        sampler,
        dir,
    }
}

fn rig_with_flag(flag: &'static AtomicBool) -> Rig {
    let mut r = rig();
    // Rebuild the watchdog around the same fakes, with the test's flag.
    r.dog = Watchdog::new(
        Box::new(r.clock.clone()),
        Box::new(r.beat.clone()),
        Box::new(r.sampler.clone()),
        r.dir.path().join("diagnostics"),
        "/Users/dev".into(),
        4242,
    )
    .with_exit_flag(flag);
    r
}

fn rig() -> Rig {
    rig_with(FakeSampler::new(Ok(report())), FakeClock::at(T0))
}

impl Rig {
    /// One second passes; the app answers the beat when `alive`.
    fn second(&mut self, alive: bool) -> Event {
        self.clock.advance(1_000);
        if alive {
            self.beat.stamp(self.clock.now());
        }
        self.dog.tick()
    }
    fn notes(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.dir.path().join("diagnostics"))
            .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

#[test]
fn a_planted_ten_second_block_writes_one_note_with_stacks() {
    let mut r = rig();
    // Healthy for a while, then the main thread stops answering at T.
    for _ in 0..5 {
        assert_eq!(r.second(true), Event::None);
    }
    let last_beat = r.clock.now();
    let mut noted = Vec::new();
    for _ in 1..=9 {
        if let Event::Noted(p) = r.second(false) {
            noted.push((r.clock.now() - last_beat, p));
        }
    }
    // Flagged at the 8 s tick, once, and not again while the stall goes on.
    assert_eq!(noted.len(), 1);
    assert_eq!(noted[0].0, 8_000);
    assert_eq!(r.sampler.calls(), 1);
    assert_eq!(r.sampler.calls.lock().unwrap()[0], 4242);

    // The beat returns after 10 s: the note gets its final duration.
    assert_eq!(r.second(true), Event::Recovered { seconds: 10 });
    let text = std::fs::read_to_string(&noted[0].1).unwrap();
    assert!(
        text.starts_with("Hollow was unresponsive for at least 8 s."),
        "{text}"
    );
    assert!(text.contains("Main thread, 150 samples"), "{text}");
    assert!(text.contains("read_vault_note"), "{text}");
    assert!(text.contains("Top of stack, all threads:"), "{text}");
    assert!(
        text.contains("Recovered: Hollow answered again after about"),
        "{text}"
    );
    assert_eq!(r.notes().len(), 1);
}

#[test]
fn the_note_reports_the_full_ten_seconds_when_the_beat_returns() {
    let mut r = rig();
    r.second(true);
    for _ in 1..=9 {
        r.second(false);
    }
    // Beat stamped at the tick ten seconds after the last good one.
    assert_eq!(r.second(true), Event::Recovered { seconds: 10 });
    let text = std::fs::read_to_string(&r.notes()[0]).unwrap();
    assert!(text.trim_end().ends_with("about 10 s."), "{text}");
}

#[test]
fn a_sleep_gap_writes_no_note_and_runs_no_sample() {
    let mut r = rig();
    for _ in 0..3 {
        r.second(true);
    }
    // The lid closes for an hour: one tick sees the clock jump.
    r.clock.advance(3_600_000);
    assert_eq!(r.dog.tick(), Event::Slept);
    // The stale heartbeat from before the sleep does not count against us
    // while the main thread wakes up and answers.
    for _ in 0..7 {
        assert_eq!(r.second(false), Event::None);
    }
    assert_eq!(r.second(true), Event::None);
    for _ in 0..20 {
        assert_eq!(r.second(true), Event::None);
    }
    assert_eq!(r.sampler.calls(), 0);
    assert!(r.notes().is_empty());
}

#[test]
fn a_real_stall_after_waking_is_still_caught() {
    let mut r = rig();
    r.second(true);
    r.clock.advance(600_000);
    assert_eq!(r.dog.tick(), Event::Slept);
    let mut found = false;
    for _ in 0..9 {
        found |= matches!(r.second(false), Event::Noted(_));
    }
    assert!(found);
    assert_eq!(r.sampler.calls(), 1);
}

#[test]
fn a_clock_set_backwards_is_not_a_hang() {
    let mut r = rig();
    r.second(true);
    r.clock.0.fetch_sub(86_400_000, Ordering::SeqCst);
    assert_eq!(r.dog.tick(), Event::Slept);
    assert_eq!(r.sampler.calls(), 0);
}

#[test]
fn nothing_is_judged_before_the_first_heartbeat() {
    let mut r = rig();
    for _ in 0..60 {
        assert_eq!(r.second(false), Event::None);
    }
    assert_eq!(r.sampler.calls(), 0);
    assert!(r.notes().is_empty());
}

#[test]
fn the_samples_own_seconds_are_not_read_as_a_sleep() {
    let clock = FakeClock::at(T0);
    let mut sampler = FakeSampler::new(Ok(report()));
    // The real sample takes 3+ s: more than the sleep gap.
    sampler.spends = Some((clock.clone(), 3_500));
    let mut r = rig_with(sampler, clock);
    r.second(true);
    let mut events = Vec::new();
    for _ in 0..20 {
        events.push(r.second(false));
    }
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Noted(_)))
            .count(),
        1,
        "{events:?}"
    );
    assert!(!events.contains(&Event::Slept), "{events:?}");
    assert_eq!(r.sampler.calls(), 1);
}

#[test]
fn it_re_arms_after_the_beat_returns() {
    let mut r = rig();
    r.second(true);
    for round in 1..=2 {
        for _ in 0..9 {
            r.second(false);
        }
        assert!(matches!(r.second(true), Event::Recovered { .. }), "{round}");
        for _ in 0..3 {
            r.second(true);
        }
    }
    // Both hangs are noted; the minute between samples spares the second.
    assert_eq!(r.sampler.calls(), 1);
    assert_eq!(r.notes().len(), 2);
}

#[test]
fn a_sleep_during_a_hang_leaves_the_note_without_a_recovery_line() {
    let mut r = rig();
    r.second(true);
    let mut path = None;
    for _ in 0..9 {
        if let Event::Noted(p) = r.second(false) {
            path = Some(p);
        }
    }
    r.clock.advance(120_000);
    assert_eq!(r.dog.tick(), Event::Slept);
    r.second(true);
    let text = std::fs::read_to_string(path.unwrap()).unwrap();
    assert!(!text.contains("Recovered"), "{text}");
}

#[test]
fn a_failed_sample_still_leaves_a_note_that_says_so() {
    let mut r = rig_with(
        FakeSampler::new(Err(
            "sample exited with exit status: 1 in /Users/builder/x".into()
        )),
        FakeClock::at(T0),
    );
    r.second(true);
    for _ in 0..9 {
        r.second(false);
    }
    let text = std::fs::read_to_string(&r.notes()[0]).unwrap();
    assert!(text.contains("Sampling failed: sample exited"), "{text}");
    assert!(text.contains("/Users/<user>/x"), "{text}");
    assert!(!text.contains("builder"), "{text}");
}

#[test]
fn only_the_newest_twenty_notes_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("keep-me.txt");
    std::fs::write(&other, "x").unwrap();
    for i in 0..25u64 {
        write_note(dir.path(), T0 + i * 1_000, "n").unwrap();
    }
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(NOTE_PREFIX))
        .collect();
    names.sort();
    assert_eq!(names.len(), KEEP_NOTES);
    assert_eq!(
        names[0],
        format!("freeze-{}.txt", utc_stamp(T0 + 5_000, "%Y%m%dT%H%M%SZ"))
    );
    assert!(other.exists());
}

#[test]
fn two_notes_in_one_second_do_not_overwrite_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_note(dir.path(), T0, "a").unwrap();
    let b = write_note(dir.path(), T0, "b").unwrap();
    assert_ne!(a, b);
    assert_eq!(std::fs::read_to_string(a).unwrap(), "a");
}

#[test]
fn the_reducer_keeps_the_main_threads_heaviest_stack_and_the_leaf_summary() {
    let out = reduce(&report(), "/Users/dev");
    let main = out.find("Main thread, 150 samples").unwrap();
    let top = out.find("Top of stack, all threads:").unwrap();
    assert!(main < top);
    // Heaviest path only: the 3-sample side branch and the idle worker's
    // thread stack are left out of the main section.
    let stack = &out[main..top];
    assert!(stack.contains("150 -[NSApplication run]"), "{out}");
    assert!(
        stack.contains("150 read  (in libsystem_kernel.dylib)"),
        "{out}"
    );
    assert!(!stack.contains("other_branch"), "{out}");
    assert!(!stack.contains("start_wqthread"), "{out}");
    // Leaf summary, all threads.
    assert!(
        out.contains("__workq_kernreturn  (in libsystem_kernel.dylib)        150"),
        "{out}"
    );
    // Header, binary images and process path are not copied.
    assert!(!out.contains("Binary Images"), "{out}");
    assert!(!out.contains("Process:"), "{out}");
}

#[test]
fn the_reducer_redacts_the_home_folder_and_any_other_user() {
    let out = reduce(&report(), "/Users/dev");
    assert!(out.contains("(in ~/Hollow/Vault/lib.dylib)"), "{out}");
    assert!(!out.contains("dev"), "{out}");
    assert_eq!(
        redact("a /Users/builder/x b /Users/dev/y", "/Users/dev"),
        "a /Users/<user>/x b ~/y"
    );
    // An empty or root home never rewrites every slash.
    assert_eq!(redact("/a/b", ""), "/a/b");
    assert_eq!(redact("/a/b", "/"), "/a/b");
}

#[test]
fn the_reducer_is_bounded_even_for_a_huge_report() {
    let mut raw = String::from("Call graph:\n    9 Thread_1 main-thread\n");
    for depth in 0..5_000usize {
        raw.push_str(&format!(
            "    + {}9 frame_{depth} {}\n",
            " ".repeat(depth.min(60)),
            "x".repeat(500)
        ));
    }
    raw.push_str("\nTotal number in stack\n\nSort by top of stack\n");
    for i in 0..500 {
        raw.push_str(&format!("        leaf_{i}  (in lib)        9\n"));
    }
    raw.push_str("\nBinary Images:\n");
    let out = reduce(&raw, "");
    assert!(out.contains("outer frames omitted"));
    assert_eq!(out.matches("frame_").count(), MAX_FRAMES);
    assert_eq!(out.matches("leaf_").count(), MAX_TOP);
    assert!(
        out.lines().all(|l| l.len() <= MAX_LINE + 8),
        "line too long"
    );
    let note = note_text(9_000, T0, &out);
    assert!(note.len() <= MAX_NOTE + 1);
}

#[test]
fn a_report_without_a_call_graph_gives_a_note_that_says_no_sample() {
    assert_eq!(reduce("garbage\n", "/Users/dev"), "");
    let note = note_text(8_000, T0, "");
    assert!(note.contains("No stack sample was available."));
    assert!(note.contains("2027-01-15T08:00:00Z"), "{note}");
}

#[test]
fn the_monitor_needs_a_beat_and_a_full_threshold() {
    let mut m = Monitor::new(8_000, 3_000);
    assert_eq!(m.step(T0, None), Step::Idle);
    assert_eq!(m.step(T0 + 1_000, Some(T0)), Step::Idle);
    assert_eq!(m.step(T0 + 2_000, Some(T0)), Step::Idle);
    assert_eq!(m.step(T0 + 3_000, Some(T0)), Step::Idle);
    assert_eq!(m.step(T0 + 4_000, Some(T0)), Step::Idle);
    assert_eq!(m.step(T0 + 5_000, Some(T0)), Step::Idle);
    assert_eq!(m.step(T0 + 6_000, Some(T0)), Step::Idle);
    assert_eq!(m.step(T0 + 7_000, Some(T0)), Step::Idle);
    assert_eq!(
        m.step(T0 + 8_000, Some(T0)),
        Step::Hang { stalled_ms: 8_000 }
    );
    assert_eq!(m.step(T0 + 9_000, Some(T0)), Step::Idle);
    assert_eq!(
        m.step(T0 + 10_000, Some(T0 + 10_000)),
        Step::Recovered { stalled_ms: 10_000 }
    );
}

/// Real `sample` on a harmless child process; macOS only, and run by hand
/// (`cargo test freeze -- --ignored`) since the tests above never run it.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "runs the real /usr/bin/sample"]
fn the_real_sampler_reads_a_report_for_a_child_process() {
    let mut child = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let got = SampleCommand {
        dir: dir.path().to_path_buf(),
        exiting: Box::leak(Box::new(AtomicBool::new(false))),
    }
    .sample(child.id());
    let _ = child.kill();
    let _ = child.wait();
    let raw = got.expect("sample runs on an own process with no extra rights");
    let out = reduce(&raw, "/Users/nobody");
    assert!(out.contains("Main thread,"), "{out}");
    assert!(out.contains("Top of stack"), "{out}");
    // The temp file is removed.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

fn leaked_flag(on: bool) -> &'static AtomicBool {
    Box::leak(Box::new(AtomicBool::new(on)))
}

fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_folder_is_owner_only_and_each_note_is_0600() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("diagnostics");
    let note = write_note(&folder, T0, "n").unwrap();
    assert_eq!(mode(&folder), 0o700);
    assert_eq!(mode(&note), 0o600);
    // A folder created loose by an earlier build is tightened.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    write_note(&folder, T0 + 1_000, "m").unwrap();
    assert_eq!(mode(&folder), 0o700);
}

#[test]
fn leftover_raw_reports_are_swept_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".sample-123.tmp"), "raw").unwrap();
    std::fs::write(dir.path().join(".sample-9.tmp"), "raw").unwrap();
    std::fs::write(dir.path().join("freeze-x.txt"), "note").unwrap();
    std::fs::write(dir.path().join("other.tmp"), "o").unwrap();
    sweep_samples(dir.path());
    assert!(!dir.path().join(".sample-123.tmp").exists());
    assert!(!dir.path().join(".sample-9.tmp").exists());
    assert!(dir.path().join("freeze-x.txt").exists());
    assert!(dir.path().join("other.tmp").exists());
    // Writing a note prunes, which sweeps too.
    std::fs::write(dir.path().join(".sample-5.tmp"), "raw").unwrap();
    write_note(dir.path(), T0, "n").unwrap();
    assert!(!dir.path().join(".sample-5.tmp").exists());
}

#[test]
fn nothing_starts_once_the_app_is_quitting() {
    let flag = leaked_flag(false);
    let mut r = rig_with_flag(flag);
    r.second(true);
    for _ in 0..4 {
        r.second(false);
    }
    flag.store(true, Ordering::SeqCst);
    for _ in 0..20 {
        assert_eq!(r.second(false), Event::None);
    }
    assert_eq!(r.sampler.calls(), 0);
    assert!(r.notes().is_empty());
}

/// A sampler that starts the quit while it "runs".
struct QuitsMidSample(&'static AtomicBool);

impl Sampler for QuitsMidSample {
    fn sample(&self, _pid: u32) -> Result<String, String> {
        self.0.store(true, Ordering::SeqCst);
        Ok(report())
    }
}

#[test]
fn a_quit_during_the_sample_writes_no_note() {
    let flag = leaked_flag(false);
    let clock = FakeClock::at(T0);
    let beat = FakeBeat::default();
    let dir = tempfile::tempdir().unwrap();
    let mut dog = Watchdog::new(
        Box::new(clock.clone()),
        Box::new(beat.clone()),
        Box::new(QuitsMidSample(flag)),
        dir.path().join("diagnostics"),
        String::new(),
        1,
    )
    .with_exit_flag(flag);
    beat.stamp(clock.now());
    dog.tick();
    let mut events = Vec::new();
    for _ in 0..9 {
        clock.advance(1_000);
        events.push(dog.tick());
    }
    assert!(events.iter().all(|e| *e == Event::None), "{events:?}");
    assert!(!dir.path().join("diagnostics").exists());
}

#[test]
fn samples_are_at_least_a_minute_apart_but_every_hang_still_gets_a_note() {
    let mut r = rig();
    r.second(true);
    let hang = |r: &mut Rig| {
        for _ in 0..9 {
            r.second(false);
        }
        assert!(matches!(r.second(true), Event::Recovered { .. }));
    };
    hang(&mut r); // sampled
    hang(&mut r); // about 20 s later: no sample
    assert_eq!(r.sampler.calls(), 1);
    let notes = r.notes();
    assert_eq!(notes.len(), 2);
    let second = std::fs::read_to_string(&notes[1]).unwrap();
    assert!(
        second.contains("No stack sample: the last one was"),
        "{second}"
    );
    assert!(!second.contains("Main thread"), "{second}");
    // A minute and more on, the next hang is sampled again.
    for _ in 0..60 {
        r.second(true);
    }
    hang(&mut r);
    assert_eq!(r.sampler.calls(), 2);
    assert_eq!(r.notes().len(), 3);
}

#[test]
fn the_switch_accepts_zero_off_false_and_no_in_any_case() {
    for on in ["0", "off", "OFF", "False", " no ", "No", "false"] {
        assert!(disabled_by(Some(on)), "{on}");
    }
    for keep in ["1", "on", "true", "yes", "", "maybe"] {
        assert!(!disabled_by(Some(keep)), "{keep:?}");
    }
    assert!(!disabled_by(None));
}

/// A child that never finishes, or whose status look fails.
#[derive(Default)]
struct FakeChild {
    look_fails: bool,
    finishes_after: Option<u32>,
    looks: u32,
    killed: bool,
    waited: bool,
}

impl Reaped for FakeChild {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        use std::os::unix::process::ExitStatusExt;
        self.looks += 1;
        if self.look_fails {
            return Err(std::io::Error::other("lost the child"));
        }
        Ok(self
            .finishes_after
            .filter(|n| self.looks > *n)
            .map(|_| std::process::ExitStatus::from_raw(0)))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        self.killed = true;
        Ok(())
    }
    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        use std::os::unix::process::ExitStatusExt;
        self.waited = true;
        Ok(std::process::ExitStatus::from_raw(0))
    }
}

/// Fake time: each look at the clock is one second later.
fn ticking() -> impl Fn() -> Duration {
    let n = std::cell::Cell::new(0u64);
    move || {
        n.set(n.get() + 1);
        Duration::from_secs(n.get())
    }
}

#[test]
fn a_sample_that_overruns_is_killed_and_reaped() {
    let mut child = FakeChild::default();
    let got = wait_bounded(
        &mut child,
        Duration::from_secs(10),
        &ticking(),
        &|| {},
        &|| false,
    );
    assert!(got.unwrap_err().contains("ran over 10 s"));
    assert!(child.killed && child.waited);
    assert_eq!(child.looks, 11);
}

#[test]
fn a_failed_status_look_kills_and_reaps_the_child_too() {
    let mut child = FakeChild {
        look_fails: true,
        ..Default::default()
    };
    let got = wait_bounded(
        &mut child,
        Duration::from_secs(10),
        &ticking(),
        &|| {},
        &|| false,
    );
    assert!(got.unwrap_err().contains("lost the child"));
    assert!(child.killed && child.waited);
}

#[test]
fn a_quit_kills_and_reaps_the_child_and_a_finished_one_is_left_alone() {
    let mut child = FakeChild::default();
    let got = wait_bounded(
        &mut child,
        Duration::from_secs(10),
        &ticking(),
        &|| {},
        &|| true,
    );
    assert!(got.unwrap_err().contains("quitting"));
    assert!(child.killed && child.waited);

    let mut done = FakeChild {
        finishes_after: Some(2),
        ..Default::default()
    };
    let got = wait_bounded(
        &mut done,
        Duration::from_secs(10),
        &ticking(),
        &|| {},
        &|| false,
    );
    assert!(got.unwrap().success());
    assert!(!done.killed && !done.waited);
}
