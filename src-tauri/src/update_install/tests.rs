//! Unit tests on fakes and temp folders: no codesign, no `open`, no sleep,
//! never `/Applications` or the real app data. The real-bundle tests are in
//! `fixture_tests.rs`.

use super::effects::{AppProc, Clock, Deps, Fs, Launcher, Procs, RealFs, Signer};
use super::health::Timing;
use super::journal::{self, Journal};
use super::run::{self, GoBackArgs, InstallArgs};
use super::verify::Team;
use super::{cli, lock, verify, Exit, Layout, DEFAULT_TEAM};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

// ------------------------------------------------------------ fixtures

pub(super) fn info_plist(version: &str, id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{id}</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>CFBundleExecutable</key><string>hollow</string>
<key>CFBundleName</key><string>Hollow</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
"#
    )
}

/// A fake bundle: Info.plist, an executable whose bytes name the version,
/// and `Contents/fake-sign` (`ok:<team>`, `ok:-` for ad hoc, or `bad`),
/// which [`FakeSigner`] reads, so a copy keeps its "signature".
fn make_bundle(at: &Path, version: &str, sign: &str) {
    make_bundle_with_id(at, version, sign, super::BUNDLE_ID);
}

fn make_bundle_with_id(at: &Path, version: &str, sign: &str, id: &str) {
    std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
    std::fs::write(at.join("Contents/Info.plist"), info_plist(version, id)).unwrap();
    std::fs::write(
        at.join("Contents/MacOS/hollow"),
        format!("binary {version}"),
    )
    .unwrap();
    std::fs::write(at.join("Contents/fake-sign"), sign).unwrap();
}

fn version_of(bundle: &Path) -> Option<String> {
    verify::read_info(bundle).ok().map(|i| i.version)
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(entry.path())?, &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

// ------------------------------------------------------------ fakes

pub(super) struct FakeClock(pub(super) Cell<u64>);

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.get()
    }
    fn sleep(&self, d: Duration) {
        self.0.set(self.0.get() + d.as_millis() as u64);
    }
}

/// The real file system, plus: every rename and swap counted, a panic (a
/// crash) or an error at a chosen one, and whether the watched install path
/// was ever absent after a move.
pub(super) struct FakeFs {
    pub(super) ops: Cell<usize>,
    /// `(op index, after the op?)`: panic there, like a killed process.
    pub(super) crash_at: Cell<Option<(usize, bool)>>,
    pub(super) fail_at: Cell<Option<usize>>,
    pub(super) watch: RefCell<Option<PathBuf>>,
    pub(super) absent: Cell<bool>,
    pub(super) unquarantined: RefCell<Vec<(PathBuf, bool)>>,
}

impl FakeFs {
    pub(super) fn new() -> Self {
        Self {
            ops: Cell::new(0),
            crash_at: Cell::new(None),
            fail_at: Cell::new(None),
            watch: RefCell::new(None),
            absent: Cell::new(false),
            unquarantined: RefCell::new(Vec::new()),
        }
    }

    fn mutate(&self, op: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        let k = self.ops.get();
        self.ops.set(k + 1);
        if self.fail_at.get() == Some(k) {
            return Err(io::Error::other("injected failure"));
        }
        if self.crash_at.get() == Some((k, false)) {
            panic!("injected crash before move {k}");
        }
        op()?;
        if let Some(watch) = self.watch.borrow().as_ref() {
            if !watch.exists() {
                self.absent.set(true);
            }
        }
        if self.crash_at.get() == Some((k, true)) {
            panic!("injected crash after move {k}");
        }
        Ok(())
    }
}

impl Fs for FakeFs {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.mutate(|| RealFs.rename(from, to))
    }
    fn swap(&self, a: &Path, b: &Path) -> io::Result<()> {
        self.mutate(|| RealFs.swap(a, b))
    }
    fn remove_all(&self, path: &Path) -> io::Result<()> {
        RealFs.remove_all(path)
    }
    fn copy_bundle(&self, from: &Path, to: &Path) -> Result<(), String> {
        let _ = RealFs.remove_all(to);
        copy_tree(from, to).map_err(|e| e.to_string())
    }
    fn clear_quarantine(&self, path: &Path) {
        // Recorded, with whether the copy was already there (it must be).
        self.unquarantined
            .borrow_mut()
            .push((path.to_path_buf(), path.exists()));
    }
}

pub(super) struct FakeSigner;

impl Signer for FakeSigner {
    fn verify(&self, bundle: &Path) -> Result<(), String> {
        match std::fs::read_to_string(bundle.join("Contents/fake-sign")) {
            Ok(s) if s.starts_with("ok:") => Ok(()),
            _ => Err("a sealed resource is missing or invalid".into()),
        }
    }
    fn team(&self, bundle: &Path) -> Result<Option<String>, String> {
        let s = std::fs::read_to_string(bundle.join("Contents/fake-sign"))
            .map_err(|e| e.to_string())?;
        // `ok:<team>[:self-signed]`: the second form carries the team text
        // without a Developer ID chain.
        let team = s
            .trim()
            .trim_start_matches("ok:")
            .split(':')
            .next()
            .unwrap_or("");
        Ok((team != "-").then(|| team.to_string()))
    }
    fn verify_requirement(&self, bundle: &Path, requirement: &str) -> Result<(), String> {
        // Satisfied when the requirement names the fake signature's team.
        let team = self
            .team(bundle)?
            .ok_or("ad hoc satisfies no team requirement")?;
        let sign = std::fs::read_to_string(bundle.join("Contents/fake-sign")).unwrap_or_default();
        if !sign.ends_with(":self-signed")
            && requirement.contains(&format!("subject.OU] = \"{team}\""))
            && requirement.starts_with("=anchor apple generic")
        {
            Ok(())
        } else {
            Err("code failed to satisfy specified code requirement(s)".into())
        }
    }
}

/// How the app behaves when the fake launcher opens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Behaves {
    /// Starts and stays up.
    Healthy,
    /// Appears, then is gone at the first liveness check.
    Crashes,
    /// Never appears.
    Never,
    /// `open` itself fails.
    OpenFails,
    /// The first process exits at once and hands off to a second one that
    /// stays up (single-instance handoff).
    HandsOff,
    /// The same, but the second process appears half a second later.
    HandsOffLate,
    /// Every process exits after 5 s and hands off to a new one, forever.
    Respawns,
}

/// The processes the fakes share: pid -> (exe, alive, start).
#[derive(Default)]
pub(super) struct World {
    pub(super) procs: HashMap<i32, (PathBuf, bool, u64)>,
    pub(super) next_pid: i32,
    pub(super) script: Vec<Behaves>,
    pub(super) launches: Vec<PathBuf>,
    /// Lock-holder probes: dead pids and start stamps.
    pub(super) dead: HashSet<i32>,
    pub(super) starts: HashMap<i32, u64>,
    pub(super) terminated: Vec<i32>,
    /// Processes that appear at a time: (when, pid, exe).
    pub(super) pending: Vec<(u64, i32, PathBuf)>,
    pub(super) clock: Option<Rc<FakeClock>>,
    pub(super) respawning: HashSet<i32>,
    pub(super) respawn_exe: Option<PathBuf>,
}

pub(super) struct FakeProcs(pub(super) Rc<RefCell<World>>);

impl Procs for FakeProcs {
    fn app_processes(&self, exe: &Path) -> Result<Vec<AppProc>, String> {
        let mut w = self.0.borrow_mut();
        let now = w.clock.as_ref().map(|c| c.now_ms()).unwrap_or(0);
        let due: Vec<(u64, i32, PathBuf)> =
            w.pending.iter().filter(|p| p.0 <= now).cloned().collect();
        w.pending.retain(|p| p.0 > now);
        for (at, pid, exe) in due {
            w.procs.insert(pid, (exe, true, at));
            w.starts.insert(pid, at * 1000);
        }
        let found: Vec<AppProc> = w
            .procs
            .iter()
            .filter(|(_, (e, _, _))| e == exe)
            .map(|(pid, (_, _, started))| AppProc {
                pid: *pid,
                started_ms: *started,
            })
            .collect();
        // A crashed process shows up in one listing, then is gone (`ps`
        // lists no dead process); it stays dead for `alive`.
        let dead: Vec<i32> = w
            .procs
            .iter()
            .filter(|(_, (e, alive, _))| e == exe && !alive)
            .map(|(pid, _)| *pid)
            .collect();
        for pid in dead {
            w.procs.remove(&pid);
            w.dead.insert(pid);
        }
        Ok(found)
    }
    fn alive(&self, pid: i32) -> Option<bool> {
        let mut w = self.0.borrow_mut();
        // A respawning process lives 5 s, then hands off to a new one
        // that appears 100 ms later and does the same.
        if w.respawning.contains(&pid) {
            let now = w.clock.as_ref().map(|c| c.now_ms()).unwrap_or(0);
            let started = w.procs.get(&pid).map(|p| p.2).unwrap_or(0);
            if w.procs.contains_key(&pid) && now >= started + 5_000 {
                w.procs.remove(&pid);
                w.dead.insert(pid);
                w.next_pid += 1;
                let next = 50_000 + w.next_pid;
                let exe = w.respawn_exe.clone().unwrap_or_default();
                w.pending.push((now + 100, next, exe));
                w.respawning.insert(next);
            }
        }
        if let Some((_, alive, _)) = w.procs.get(&pid) {
            return Some(*alive);
        }
        Some(!w.dead.contains(&pid))
    }
    fn started_micros(&self, pid: i32) -> Option<u64> {
        self.0.borrow().starts.get(&pid).copied()
    }
    fn terminate(&self, pid: i32) {
        let mut w = self.0.borrow_mut();
        w.procs.remove(&pid);
        w.dead.insert(pid);
        w.terminated.push(pid);
    }
    fn own_pid(&self) -> i32 {
        std::process::id() as i32
    }
}

pub(super) struct FakeLauncher {
    pub(super) world: Rc<RefCell<World>>,
    pub(super) clock: Rc<FakeClock>,
}

impl Launcher for FakeLauncher {
    fn launch(&self, app: &Path) -> Result<(), String> {
        let mut w = self.world.borrow_mut();
        w.launches.push(app.to_path_buf());
        let behaves = if w.script.is_empty() {
            Behaves::Healthy
        } else {
            w.script.remove(0)
        };
        if behaves == Behaves::OpenFails {
            return Err("LSOpenURLsWithRole() failed".into());
        }
        if behaves == Behaves::Never {
            return Ok(());
        }
        let now = self.clock.now_ms();
        let exe = app.join("Contents/MacOS/hollow");
        let mut spawn = |alive: bool| {
            w.next_pid += 1;
            let pid = 50_000 + w.next_pid;
            w.procs.insert(pid, (exe.clone(), alive, now));
            w.starts.insert(pid, now * 1000);
        };
        match behaves {
            Behaves::HandsOff => {
                spawn(false);
                spawn(true);
            }
            Behaves::Respawns => {
                spawn(true);
                let pid = 50_000 + w.next_pid;
                w.respawning.insert(pid);
                w.respawn_exe = Some(exe.clone());
            }
            Behaves::HandsOffLate => {
                spawn(false);
                w.next_pid += 1;
                let pid = 50_000 + w.next_pid;
                w.pending.push((now + 500, pid, exe.clone()));
            }
            other => spawn(other == Behaves::Healthy),
        }
        Ok(())
    }
}

// ------------------------------------------------------------ harness

pub(super) struct H {
    pub(super) _dir: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) apps: PathBuf,
    pub(super) state: PathBuf,
    pub(super) world: Rc<RefCell<World>>,
    pub(super) clock: Rc<FakeClock>,
    pub(super) fs: FakeFs,
    pub(super) procs: FakeProcs,
    pub(super) launcher: FakeLauncher,
}

pub(super) const TEAM_OK: &str = "ok:TEAM000000";

impl H {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let apps = root.join("Applications");
        std::fs::create_dir_all(&apps).unwrap();
        let world = Rc::new(RefCell::new(World::default()));
        let clock = Rc::new(FakeClock(Cell::new(1_763_200_000_000)));
        world.borrow_mut().clock = Some(clock.clone());
        Self {
            _dir: dir,
            state: root.join("state"),
            apps,
            root,
            procs: FakeProcs(world.clone()),
            launcher: FakeLauncher {
                world: world.clone(),
                clock: clock.clone(),
            },
            world,
            clock,
            fs: FakeFs::new(),
        }
    }

    pub(super) fn app(&self) -> PathBuf {
        self.apps.join("Hollow.app")
    }

    pub(super) fn layout(&self) -> Layout {
        Layout::for_app(&self.app())
    }

    /// Installed 0.6.1 with a previous 0.6.0.
    pub(super) fn with_installed(self) -> Self {
        make_bundle(&self.app(), "0.6.1", TEAM_OK);
        make_bundle(&self.layout().previous, "0.6.0", TEAM_OK);
        self
    }

    pub(super) fn staged(&self, version: &str, sign: &str) -> PathBuf {
        let at = self.root.join("download").join(version).join("Hollow.app");
        make_bundle(&at, version, sign);
        at
    }

    pub(super) fn args(&self, staged: &Path, version: &str) -> InstallArgs {
        InstallArgs {
            staged: staged.to_path_buf(),
            expected_version: version.into(),
            team: Team::Id(DEFAULT_TEAM.into()),
            app: self.app(),
            state_dir: self.state.clone(),
            snapshot_source: None,
            relaunch: true,
            allow_downgrade: false,
            install_id: format!("20261115-100000-{:08x}", rand::random::<u32>()),
            timing: Timing::default(),
        }
    }

    pub(super) fn deps<'a>(&'a self, checkpoint: &'a dyn Fn(&str)) -> Deps<'a> {
        Deps {
            fs: &self.fs,
            signer: &FakeSigner,
            procs: &self.procs,
            launcher: &self.launcher,
            clock: &*self.clock,
            checkpoint,
        }
    }

    pub(super) fn install(&self, args: &InstallArgs) -> super::Outcome {
        run::install(args, &self.deps(&|_| {}))
    }

    pub(super) fn journal(&self, outcome: &super::Outcome) -> journal::Record {
        let path = outcome.journal.clone().expect("a journal");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    pub(super) fn script(&self, script: &[Behaves]) {
        self.world.borrow_mut().script = script.to_vec();
    }

    pub(super) fn leftovers(&self) -> Vec<PathBuf> {
        let l = self.layout();
        [l.staging, l.discard]
            .into_iter()
            .filter(|p| p.exists())
            .collect()
    }
}

fn steps(record: &journal::Record) -> Vec<&str> {
    record.steps.iter().map(|s| s.step.as_str()).collect()
}

// ------------------------------------------------------------ the swap

#[test]
fn the_swap_puts_the_new_app_in_place_keeps_the_previous_and_never_leaves_the_path_empty() {
    let h = H::new().with_installed();
    *h.fs.watch.borrow_mut() = Some(h.app());
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert_eq!(version_of(&h.layout().previous).as_deref(), Some("0.6.1"));
    assert!(h.leftovers().is_empty(), "{:?}", h.leftovers());
    assert!(
        !h.fs.absent.get(),
        "the install path was empty after a move"
    );
    assert!(staged.exists(), "the caller's staged bundle is left alone");
    assert_eq!(
        *h.fs.unquarantined.borrow(),
        vec![(h.layout().staging, true)],
        "the quarantine flag is cleared from the copy, once, after it exists"
    );
    let record = h.journal(&out);
    assert_eq!(
        steps(&record),
        [
            "prepared",
            "staged-verified",
            "snapshot-written",
            "promoted",
            "started",
            "healthy"
        ]
    );
    assert_eq!(record.from_version.as_deref(), Some("0.6.1"));
    assert_eq!(record.to_version.as_deref(), Some("0.6.2"));
    assert_eq!(record.moves_done, 3);
    assert_eq!(h.world.borrow().launches, vec![h.app()]);
    // The lock is released.
    assert!(!h.state.join(lock::LOCK_FILE).exists());
}

#[test]
fn a_fresh_mac_installs_with_one_rename_and_no_previous() {
    let h = H::new();
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert!(!h.layout().previous.exists());
    assert_eq!(h.journal(&out).moves_done, 1);
}

#[test]
fn an_installed_app_that_does_not_verify_never_replaces_a_verified_previous() {
    let h = H::new().with_installed();
    std::fs::write(h.app().join("Contents/fake-sign"), "bad").unwrap();
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version_of(&h.layout().previous).as_deref(), Some("0.6.0"));
    assert!(h.leftovers().is_empty());
}

#[test]
fn no_relaunch_installs_without_opening_and_settles_unchecked() {
    let h = H::new().with_installed();
    let staged = h.staged("0.6.2", TEAM_OK);
    let mut args = h.args(&staged, "0.6.2");
    args.relaunch = false;
    let out = h.install(&args);
    assert_eq!(out.exit, Exit::Ok);
    assert!(h.world.borrow().launches.is_empty());
    assert_eq!(h.journal(&out).state, "unchecked");
    assert!(h.leftovers().is_empty());
}

#[test]
fn a_move_that_fails_is_undone_and_the_old_apps_stay() {
    for k in 0..3 {
        let h = H::new().with_installed();
        let staged = h.staged("0.6.2", TEAM_OK);
        h.fs.fail_at.set(Some(k));
        let out = h.install(&h.args(&staged, "0.6.2"));
        assert_eq!(out.exit, Exit::Failed, "move {k}: {}", out.message);
        assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"), "move {k}");
        assert_eq!(
            version_of(&h.layout().previous).as_deref(),
            Some("0.6.0"),
            "move {k}"
        );
        assert!(h.leftovers().is_empty(), "move {k}: {:?}", h.leftovers());
        assert_eq!(h.journal(&out).state, "failed");
    }
}

// ------------------------------------------------------------ reopen check

#[test]
fn two_failed_starts_swap_back_reopen_the_previous_and_write_the_failure_file() {
    let h = H::new().with_installed();
    *h.fs.watch.borrow_mut() = Some(h.app());
    h.script(&[Behaves::Crashes, Behaves::Never, Behaves::Healthy]);
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::WentBack, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert_eq!(version_of(&h.layout().previous).as_deref(), Some("0.6.0"));
    assert!(h.leftovers().is_empty(), "{:?}", h.leftovers());
    assert!(!h.fs.absent.get());
    // Two tries of the new app, then the previous one reopened.
    assert_eq!(h.world.borrow().launches.len(), 3);
    let record = h.journal(&out);
    assert_eq!(record.state, "rolled-back");
    assert_eq!(record.moves_done, 0);
    let outcomes: Vec<_> = record.attempts.iter().map(|a| a.outcome.as_str()).collect();
    assert_eq!(outcomes, ["exited", "never-appeared"]);

    let failure: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(h.state.join(run::FAILURE_FILE)).unwrap())
            .unwrap();
    assert_eq!(failure["schema"], 1);
    assert_eq!(failure["attemptedVersion"], "0.6.2");
    assert_eq!(failure["restoredVersion"], "0.6.1");
    assert_eq!(failure["wentBack"], true);
    assert_eq!(failure["attempts"].as_array().unwrap().len(), 2);
    assert!(failure["reason"]
        .as_str()
        .unwrap()
        .contains("did not start in 2 tries"));
    assert_eq!(failure["installId"], record.id);
}

#[test]
fn one_failed_start_then_a_healthy_one_keeps_the_update() {
    let h = H::new().with_installed();
    h.script(&[Behaves::OpenFails, Behaves::Healthy]);
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    let record = h.journal(&out);
    assert_eq!(record.attempts.len(), 2);
    assert_eq!(record.state, "healthy");
    assert!(!h.state.join(run::FAILURE_FILE).exists());
}

#[test]
fn a_start_that_dies_before_the_settle_time_is_a_failed_start() {
    // Health rule: alive through the settle window. A process that appears
    // and is gone by the first liveness check counts as exited.
    let h = H::new().with_installed();
    h.script(&[Behaves::Crashes, Behaves::Crashes, Behaves::Healthy]);
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::WentBack);
    let outcomes: Vec<_> = h
        .journal(&out)
        .attempts
        .into_iter()
        .map(|a| a.outcome)
        .collect();
    assert_eq!(outcomes, ["exited", "exited"]);
}

#[test]
fn hollow_still_running_is_waited_for_then_refused_with_nothing_moved() {
    let h = H::new().with_installed();
    h.world
        .borrow_mut()
        .procs
        .insert(4321, (h.app().join("Contents/MacOS/hollow"), true, 1));
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::AppRunning, "{}", out.message);
    assert!(out.message.contains("4321"));
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert!(out.journal.is_none(), "refused before the journal");
}

// ------------------------------------------------------------ snapshot

fn sessions_db(dir: &Path, name: &str, version: i64) -> rusqlite::Connection {
    std::fs::create_dir_all(dir).unwrap();
    let conn = rusqlite::Connection::open(dir.join(name)).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE turns(id INTEGER PRIMARY KEY, body TEXT);
         INSERT INTO turns(body) VALUES ('one'), ('two'), ('three');
         PRAGMA user_version = {version};"
    ))
    .unwrap();
    conn
}

#[test]
fn the_sessions_snapshot_is_written_consistent_and_recorded_with_its_user_version() {
    let h = H::new().with_installed();
    let support = h.root.join("support");
    // Open writers while the snapshot runs (an online copy).
    let live_a = sessions_db(&support.join("Sessions"), "vault-aaaa.db", 7);
    let _live_b = sessions_db(&support.join("Sessions"), "other-bbbb.db", 3);
    live_a
        .execute("INSERT INTO turns(body) VALUES ('four')", [])
        .unwrap();
    let staged = h.staged("0.6.2", TEAM_OK);
    let mut args = h.args(&staged, "0.6.2");
    args.snapshot_source = Some((support.clone(), true));
    let out = h.install(&args);
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);

    let snap = h.journal(&out).snapshot.unwrap();
    assert!(!snap.skipped);
    let folder = snap.folder.clone().unwrap();
    assert!(folder.starts_with(h.state.join("snapshots")));
    assert!(folder.to_string_lossy().ends_with("-from-0.6.1"));
    assert_eq!(snap.files.len(), 2);
    let a = snap
        .files
        .iter()
        .find(|f| f.copy.ends_with("vault-aaaa.db"))
        .unwrap();
    assert_eq!(a.user_version, 7);
    let copy =
        rusqlite::Connection::open_with_flags(&a.copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let rows: i64 = copy
        .query_row("SELECT count(*) FROM turns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 4, "the copy holds every committed row, the WAL's too");
    let uv: i64 = copy
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(uv, 7);
    let quick: String = copy
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(quick, "ok");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&a.copy).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn a_failed_snapshot_stops_the_install_before_anything_moves() {
    for broken in ["missing", "garbage"] {
        let h = H::new().with_installed();
        let source = h.root.join("support/Sessions");
        if broken == "garbage" {
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(source.join("vault.db"), vec![0x5au8; 8192]).unwrap();
        }
        let staged = h.staged("0.6.2", TEAM_OK);
        let mut args = h.args(&staged, "0.6.2");
        args.snapshot_source = Some((source, true));
        let out = h.install(&args);
        assert_eq!(out.exit, Exit::SnapshotFailed, "{broken}: {}", out.message);
        assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
        assert!(h.leftovers().is_empty());
        assert_eq!(h.fs.ops.get(), 0, "{broken}: nothing moved");
        assert_eq!(h.journal(&out).state, "failed");
    }
}

#[test]
fn no_snapshot_is_recorded_as_skipped_and_a_missing_default_source_is_empty() {
    let h = H::new().with_installed();
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert!(h.journal(&out).snapshot.unwrap().skipped);

    let h = H::new().with_installed();
    let staged = h.staged("0.6.2", TEAM_OK);
    let mut args = h.args(&staged, "0.6.2");
    args.snapshot_source = Some((h.root.join("no-such-support"), false));
    let out = h.install(&args);
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    let snap = h.journal(&out).snapshot.unwrap();
    assert!(!snap.skipped && snap.files.is_empty());
}

// ------------------------------------------------------------ the lock

#[test]
fn a_second_installer_is_refused_while_one_holds_the_lock() {
    let h = H::new().with_installed();
    let held = lock::acquire(
        &h.state,
        "first",
        "install",
        std::process::id() as i32,
        "now",
        &h.procs,
    )
    .unwrap();
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Busy, "{}", out.message);
    assert!(out.message.contains("another Hollow installer is running"));
    assert!(out.message.contains("first"));
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert!(out.journal.is_none());
    // A go back is refused the same way.
    let back = run::go_back(&go_back_args(&h), &h.deps(&|_| {}));
    assert_eq!(back.exit, Exit::Busy);
    drop(held);
    assert!(!h.state.join(lock::LOCK_FILE).exists());
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
}

#[test]
fn the_mjs_installers_lock_refuses_this_installer_too() {
    let h = H::new().with_installed();
    std::fs::create_dir_all(&h.state).unwrap();
    // The `.mjs` writes pid, installId, what, startedAt and no start stamp.
    let mjs = format!(
        r#"{{"pid":{},"installId":"20261002-150755-d7007a7b","what":"install","startedAt":"2026-11-15T10:00:00.000Z"}}"#,
        std::process::id()
    );
    std::fs::write(h.state.join(lock::LOCK_FILE), &mjs).unwrap();
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Busy);
    assert_eq!(
        std::fs::read_to_string(h.state.join(lock::LOCK_FILE)).unwrap(),
        mjs
    );
}

#[test]
fn a_stale_lock_is_taken_over_when_its_pid_is_gone_or_reused() {
    for case in ["dead", "reused", "damaged"] {
        let h = H::new().with_installed();
        std::fs::create_dir_all(&h.state).unwrap();
        let text = match case {
            "dead" => {
                h.world.borrow_mut().dead.insert(999_991);
                r#"{"pid":999991,"what":"install"}"#.to_string()
            }
            "reused" => {
                // Alive, but started at another time than the lock says.
                h.world.borrow_mut().starts.insert(4242, 2_000);
                r#"{"pid":4242,"pidStartedAt":1000,"what":"install"}"#.to_string()
            }
            _ => "{\"pid\":".to_string(),
        };
        std::fs::write(h.state.join(lock::LOCK_FILE), text).unwrap();
        let staged = h.staged("0.6.2", TEAM_OK);
        let out = h.install(&h.args(&staged, "0.6.2"));
        assert_eq!(out.exit, Exit::Ok, "{case}: {}", out.message);
        assert!(!h.state.join(lock::LOCK_FILE).exists(), "{case}");
        let leftover_claims = std::fs::read_dir(&h.state)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("takeover")
            })
            .count();
        assert_eq!(leftover_claims, 0, "{case}");
    }
}

#[test]
fn a_live_holder_with_the_same_start_stamp_keeps_the_lock() {
    let h = H::new();
    h.world.borrow_mut().starts.insert(4242, 1_000);
    std::fs::create_dir_all(&h.state).unwrap();
    std::fs::write(
        h.state.join(lock::LOCK_FILE),
        r#"{"pid":4242,"pidStartedAt":1000}"#,
    )
    .unwrap();
    let got = lock::acquire(&h.state, "x", "install", 1, "now", &h.procs);
    assert!(matches!(got, Err(lock::LockError::Busy(_))), "{got:?}");
}

// ------------------------------------------------------------ refusals

#[test]
fn the_wrong_team_version_bundle_id_or_signature_is_refused_with_nothing_moved() {
    let cases: Vec<(&str, Box<dyn Fn(&H) -> InstallArgs>)> = vec![
        (
            "other team",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", "ok:ZZZZZZZZZZ");
                h.args(&s, "0.6.2")
            }),
        ),
        (
            "the team's text without a Developer ID",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", "ok:TEAM000000:self-signed");
                h.args(&s, "0.6.2")
            }),
        ),
        (
            "ad hoc when a team is expected",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", "ok:-");
                h.args(&s, "0.6.2")
            }),
        ),
        (
            "a team when ad hoc is expected",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", TEAM_OK);
                let mut a = h.args(&s, "0.6.2");
                a.team = Team::AdHoc;
                a
            }),
        ),
        (
            "wrong version",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", TEAM_OK);
                h.args(&s, "0.6.3")
            }),
        ),
        (
            "bad signature",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", "bad");
                h.args(&s, "0.6.2")
            }),
        ),
        (
            "other bundle id",
            Box::new(|h: &H| {
                let s = h.root.join("dl/Hollow.app");
                make_bundle_with_id(&s, "0.6.2", TEAM_OK, "com.hollow.app.candidate");
                h.args(&s, "0.6.2")
            }),
        ),
        (
            "downgrade",
            Box::new(|h: &H| {
                let s = h.staged("0.5.9", TEAM_OK);
                h.args(&s, "0.5.9")
            }),
        ),
        (
            "inside the installed app",
            Box::new(|h: &H| {
                let s = h.app().join("Contents/Resources/Hollow.app");
                make_bundle(&s, "0.6.2", TEAM_OK);
                h.args(&s, "0.6.2")
            }),
        ),
        (
            "a symlink to a bundle",
            Box::new(|h: &H| {
                let real = h.staged("0.6.2", TEAM_OK);
                let link = h.root.join("link/Hollow.app");
                std::fs::create_dir_all(link.parent().unwrap()).unwrap();
                std::os::unix::fs::symlink(&real, &link).unwrap();
                h.args(&link, "0.6.2")
            }),
        ),
        (
            "a symlink escaping the bundle",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", TEAM_OK);
                std::os::unix::fs::symlink("/etc/passwd", s.join("Contents/Resources-link"))
                    .unwrap();
                h.args(&s, "0.6.2")
            }),
        ),
        (
            "a relative symlink climbing out",
            Box::new(|h: &H| {
                let s = h.staged("0.6.2", TEAM_OK);
                std::os::unix::fs::symlink("../../../../x", s.join("Contents/MacOS/up")).unwrap();
                h.args(&s, "0.6.2")
            }),
        ),
    ];
    for (name, make) in cases {
        // Each refusal names its own rule.
        let expect = match name {
            "other team" => "signed by team ZZZZZZZZZZ, not the expected TEAM000000",
            "the team's text without a Developer ID" => {
                "not a Developer ID signature of team TEAM000000"
            }
            "ad hoc when a team is expected" => {
                "signed by team none (ad hoc), not the expected TEAM000000"
            }
            "a team when ad hoc is expected" => {
                "signed by team TEAM000000, not the expected none (ad hoc)"
            }
            "wrong version" => "version 0.6.2, not the expected 0.6.3",
            "bad signature" => "signature does not verify",
            "other bundle id" => "bundle identifier is com.hollow.app.candidate",
            "downgrade" => "0.5.9 is older than the installed 0.6.1",
            "inside the installed app" => "which the install replaces",
            "a symlink to a bundle" => "is a symlink; pass the bundle itself",
            "a symlink escaping the bundle" | "a relative symlink climbing out" => {
                "has a symlink that points outside it"
            }
            other => panic!("no expected message for {other}"),
        };
        let h = H::new().with_installed();
        let args = make(&h);
        let out = h.install(&args);
        assert_eq!(out.exit, Exit::Refused, "{name}: {}", out.message);
        assert!(out.message.contains(expect), "{name}: {}", out.message);
        assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"), "{name}");
        assert_eq!(
            version_of(&h.layout().previous).as_deref(),
            Some("0.6.0"),
            "{name}"
        );
        assert!(h.leftovers().is_empty(), "{name}: {:?}", h.leftovers());
        assert_eq!(h.fs.ops.get(), 0, "{name}: nothing moved");
        assert_eq!(h.journal(&out).state, "failed", "{name}");
    }
}

#[test]
fn an_inside_symlink_is_fine_and_a_downgrade_is_allowed_with_the_flag() {
    let h = H::new().with_installed();
    let s = h.staged("0.5.9", TEAM_OK);
    std::os::unix::fs::symlink("MacOS/hollow", s.join("Contents/current")).unwrap();
    let mut args = h.args(&s, "0.5.9");
    args.allow_downgrade = true;
    let out = h.install(&args);
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.5.9"));
}

#[test]
fn an_install_folder_this_account_cannot_write_is_refused_in_plain_words() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: a plain getter.
    if unsafe { libc::geteuid() } == 0 {
        return; // root writes through 0555, so the refusal can't be staged
    }
    let h = H::new().with_installed();
    let staged = h.staged("0.6.2", TEAM_OK);
    std::fs::set_permissions(&h.apps, std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = h.install(&h.args(&staged, "0.6.2"));
    std::fs::set_permissions(&h.apps, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.exit, Exit::NotWritable, "{}", out.message);
    assert!(out.message.contains("administrator"), "{}", out.message);
    assert!(out.message.contains("Nothing was changed"));
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert!(!h.state.exists(), "refused before the lock or a journal");
}

#[test]
fn leftovers_no_journal_explains_and_an_unfinished_mjs_install_are_refused() {
    for leftover in ["staging", "mjs"] {
        let h = H::new().with_installed();
        let path = match leftover {
            "staging" => h.layout().staging,
            _ => h.layout().foreign[0].clone(),
        };
        make_bundle(&path, "0.6.0", TEAM_OK);
        let staged = h.staged("0.6.2", TEAM_OK);
        let out = h.install(&h.args(&staged, "0.6.2"));
        assert_eq!(out.exit, Exit::NeedsHand, "{leftover}: {}", out.message);
        assert!(path.exists(), "{leftover}: never deleted on a guess");
        assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    }
}

// ------------------------------------------------------------ crash recovery

fn recover(h: &H) -> Result<Vec<String>, String> {
    run::recover_all(&h.layout(), &h.state, &h.deps(&|_| {}))
}

fn only_journal(h: &H) -> Journal {
    let (mut all, bad) = journal::list(&h.state);
    assert!(bad.is_empty());
    assert_eq!(all.len(), 1);
    all.remove(0)
}

/// Run an install that "crashes" (panics) where `arm` says; return the harness.
fn crashed_install(arm: impl FnOnce(&H), checkpoint: &dyn Fn(&str), script: &[Behaves]) -> H {
    let h = H::new().with_installed();
    h.script(script);
    arm(&h);
    let staged = h.staged("0.6.2", TEAM_OK);
    let args = h.args(&staged, "0.6.2");
    let deps = h.deps(checkpoint);
    let crashed =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run::install(&args, &deps)));
    assert!(crashed.is_err(), "the install should have crashed");
    // A real crash leaves the lock with a dead pid; the guard's drop
    // released it here, which recovery also handles.
    h
}

#[test]
fn a_crash_at_each_journal_state_is_recovered_by_the_next_run() {
    let cases = [
        ("prepared", "0.6.1", "failed"),
        ("staged-verified", "0.6.1", "failed"),
        ("snapshot-written", "0.6.1", "failed"),
        ("promoted", "0.6.2", "healthy"),
        ("started", "0.6.2", "healthy"),
    ];
    for (state, installed, settled) in cases {
        let target = state.to_string();
        let hook = move |s: &str| {
            if s == target {
                panic!("injected crash at {s}");
            }
        };
        let h = crashed_install(|_| {}, &hook, &[]);
        assert_eq!(only_journal(&h).record.state, state);
        recover(&h).unwrap_or_else(|e| panic!("{state}: {e}"));
        let j = only_journal(&h);
        assert_eq!(j.record.state, settled, "{state}");
        assert_eq!(version_of(&h.app()).as_deref(), Some(installed), "{state}");
        let previous = if installed == "0.6.2" {
            "0.6.1"
        } else {
            "0.6.0"
        };
        assert_eq!(
            version_of(&h.layout().previous).as_deref(),
            Some(previous),
            "{state}"
        );
        assert!(h.leftovers().is_empty(), "{state}: {:?}", h.leftovers());
        if settled == "healthy" {
            assert_eq!(j.record.steps.last().unwrap().detail["confirmed"], false);
        }
        // And the next install goes ahead (the started app quit meanwhile).
        h.world.borrow_mut().procs.clear();
        let staged = h.staged("0.6.3", TEAM_OK);
        let out = h.install(&h.args(&staged, "0.6.3"));
        assert_eq!(out.exit, Exit::Ok, "{state}: {}", out.message);
    }
}

#[test]
fn a_crash_while_going_back_after_two_failed_starts_finishes_going_back() {
    for state in ["started", "rolling-back"] {
        let crash_on_second = Cell::new(0);
        let target = state.to_string();
        let hook = move |s: &str| {
            if s == target {
                crash_on_second.set(crash_on_second.get() + 1);
                if s == "rolling-back" || crash_on_second.get() == 2 {
                    panic!("injected crash at {s}");
                }
            }
        };
        let h = crashed_install(|_| {}, &hook, &[Behaves::Crashes, Behaves::Crashes]);
        recover(&h).unwrap();
        assert_eq!(only_journal(&h).record.state, "rolled-back", "{state}");
        assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"), "{state}");
        assert_eq!(
            version_of(&h.layout().previous).as_deref(),
            Some("0.6.0"),
            "{state}"
        );
        assert!(h.leftovers().is_empty(), "{state}");
        let failure: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(h.state.join(run::FAILURE_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(failure["wentBack"], true, "{state}");
    }
}

#[test]
fn a_crash_before_or_after_any_single_move_is_recovered_and_the_path_is_never_empty() {
    // Three moves (swap, previous -> discard, staging -> previous).
    for k in 0..3 {
        for after in [false, true] {
            let h = crashed_install(
                |h| {
                    h.fs.crash_at.set(Some((k, after)));
                    *h.fs.watch.borrow_mut() = Some(h.app());
                },
                &|_| {},
                &[],
            );
            assert!(
                !h.fs.absent.get(),
                "move {k} after={after}: the path was empty"
            );
            h.fs.crash_at.set(None);
            recover(&h).unwrap_or_else(|e| panic!("move {k} after={after}: {e}"));
            let j = only_journal(&h);
            // Only a crash after the last move, before its journal write,
            // leaves every move done: the new app verifies and stands.
            let finished = k == 2 && after;
            let (installed, previous, state) = if finished {
                ("0.6.2", "0.6.1", "healthy")
            } else if k == 0 && !after {
                // Nothing had moved.
                ("0.6.1", "0.6.0", "failed")
            } else {
                ("0.6.1", "0.6.0", "rolled-back")
            };
            let tag = format!("move {k} after={after}");
            assert_eq!(j.record.state, state, "{tag}");
            assert_eq!(version_of(&h.app()).as_deref(), Some(installed), "{tag}");
            assert_eq!(
                version_of(&h.layout().previous).as_deref(),
                Some(previous),
                "{tag}"
            );
            assert!(h.leftovers().is_empty(), "{tag}: {:?}", h.leftovers());
            assert!(!h.fs.absent.get(), "{tag}: recovery emptied the path");
        }
    }
}

#[test]
fn a_crash_while_undoing_is_finished_by_recovery() {
    // The second move fails, and the undo of the first crashes after it ran.
    let h = H::new().with_installed();
    let staged = h.staged("0.6.2", TEAM_OK);
    h.fs.fail_at.set(Some(1));
    h.fs.crash_at.set(Some((2, true)));
    let args = h.args(&staged, "0.6.2");
    let deps = h.deps(&|_| {});
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run::install(&args, &deps)))
            .is_err()
    );
    h.fs.crash_at.set(None);
    assert_eq!(only_journal(&h).record.state, "rolling-back");
    recover(&h).unwrap();
    assert_eq!(only_journal(&h).record.state, "rolled-back");
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert_eq!(version_of(&h.layout().previous).as_deref(), Some("0.6.0"));
    assert!(h.leftovers().is_empty());
}

#[test]
fn an_unreadable_or_other_schema_journal_is_set_aside_and_does_not_block() {
    for (name, text) in [
        ("damaged", "{ half".to_string()),
        ("schema", r#"{"schema":99,"kind":"install","id":"x","pid":1,"startedAt":"","state":"prepared","app":"/x/Hollow.app"}"#.to_string()),
    ] {
        let h = H::new().with_installed();
        let dir = h.state.join(journal::DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.json"), &text).unwrap();
        let staged = h.staged("0.6.2", TEAM_OK);
        let out = h.install(&h.args(&staged, "0.6.2"));
        assert_eq!(out.exit, Exit::Ok, "{name}: {}", out.message);
        assert!(dir.join("x.json.bad").exists(), "{name}");
        assert!(!dir.join("x.json").exists(), "{name}");
    }
}

#[test]
fn a_journal_from_an_older_shape_still_reads_with_defaults() {
    // Optional fields default; `plan` (which may be null) and `movesDone`
    // must be present: a missing one makes the journal unreadable.
    let text = r#"{"schema":1,"kind":"install","id":"old","pid":1,"startedAt":"2026-11-15T00:00:00.000Z","state":"healthy","app":"/x/Hollow.app","plan":null,"movesDone":0}"#;
    let record: journal::Record = serde_json::from_str(text).unwrap();
    assert!(record.plan.is_none() && record.steps.is_empty() && record.attempts.is_empty());
    let missing = r#"{"schema":1,"kind":"install","id":"old","pid":1,"startedAt":"","state":"promoted","app":"/x/Hollow.app"}"#;
    assert!(serde_json::from_str::<journal::Record>(missing).is_err());
    let no_plan = r#"{"schema":1,"kind":"install","id":"old","pid":1,"startedAt":"","state":"promoted","app":"/x/Hollow.app","movesDone":0}"#;
    assert!(
        serde_json::from_str::<journal::Record>(no_plan).is_err(),
        "a missing plan is not read as none"
    );
}

#[test]
fn a_set_aside_journal_keeps_an_unexplained_leftover_blocking() {
    let h = H::new().with_installed();
    let dir = h.state.join(journal::DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.json"), "{ half").unwrap();
    make_bundle(&h.layout().staging, "0.6.0", TEAM_OK);
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::NeedsHand, "{}", out.message);
    assert!(h.layout().staging.exists());
}

#[test]
fn a_discard_left_by_a_failed_removal_is_retried_after_a_healthy_install() {
    let h = H::new().with_installed();
    let staged = h.staged("0.6.2", TEAM_OK);
    assert_eq!(h.install(&h.args(&staged, "0.6.2")).exit, Exit::Ok);
    // As if removing it had failed: the newest journal says healthy.
    make_bundle(&h.layout().discard, "0.6.0", TEAM_OK);
    h.world.borrow_mut().procs.clear();
    let staged = h.staged("0.6.3", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.3"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert!(h.leftovers().is_empty());
}

// ------------------------------------------------------------ go back

fn go_back_args(h: &H) -> GoBackArgs {
    GoBackArgs {
        app: h.app(),
        state_dir: h.state.clone(),
        relaunch: true,
        install_id: format!("20261115-110000-{:08x}", rand::random::<u32>()),
        timing: Timing::default(),
        restore: None,
    }
}

#[test]
fn go_back_swaps_the_previous_and_the_installed_app_journaled() {
    let h = H::new().with_installed();
    *h.fs.watch.borrow_mut() = Some(h.app());
    let out = run::go_back(&go_back_args(&h), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.0"));
    assert_eq!(version_of(&h.layout().previous).as_deref(), Some("0.6.1"));
    assert!(!h.fs.absent.get());
    let record = h.journal(&out);
    assert_eq!(record.kind, journal::Kind::GoBack);
    assert_eq!(
        steps(&record),
        ["prepared", "promoted", "started", "healthy"]
    );
    // Going back twice is going forward again.
    h.world.borrow_mut().procs.clear();
    let out = run::go_back(&go_back_args(&h), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::Ok);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
}

#[test]
fn a_go_back_that_fails_part_way_without_a_restore_undoes_its_moves() {
    // The go back plan is one swap today, so a part way failure with no
    // restore cannot happen through `go_back`; the failure path still has to
    // put every done move back (it ends in a reopen of the installed app).
    let h = H::new().with_installed();
    let (a, b, c, d) = (
        h.root.join("a"),
        h.root.join("b"),
        h.root.join("c"),
        h.root.join("d"),
    );
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&c).unwrap();
    let mv = |from: &Path, to: &Path| journal::Move {
        op: journal::Op::Rename,
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        ino: super::swap::ino(from).unwrap(),
    };
    let mut record = journal::Record::new(
        journal::Kind::GoBack,
        "20261117-210000-00000001",
        1,
        &h.app(),
        "2026-11-17T21:00:00Z",
    );
    record.plan = Some(vec![mv(&a, &b), mv(&c, &d)]);
    let mut j = Journal::create(&h.state, record).unwrap();
    // The first move happens, the second fails.
    h.fs.fail_at.set(Some(h.fs.ops.get() + 1));
    let failed = super::swap::run_moves(&h.fs, &mut j).unwrap_err();
    assert_eq!(j.record.moves_done, 1);
    assert!(b.exists() && !a.exists());
    let out = run::undo_go_back(&mut j, &h.deps(&|_| {}), None, Exit::Failed, failed);
    assert_eq!(out.exit, Exit::Failed, "{}", out.message);
    assert!(
        out.message.contains("every move was undone"),
        "{}",
        out.message
    );
    assert!(a.exists() && !b.exists(), "the first move is back");
    assert!(c.exists() && !d.exists());
    assert_eq!(j.record.moves_done, 0);
    assert_eq!(j.record.state, "failed");
}

#[test]
fn go_back_without_a_previous_or_with_a_broken_one_is_refused() {
    let h = H::new();
    make_bundle(&h.app(), "0.6.1", TEAM_OK);
    let out = run::go_back(&go_back_args(&h), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::NothingToGoBack);

    let h = H::new().with_installed();
    std::fs::write(h.layout().previous.join("Contents/fake-sign"), "bad").unwrap();
    let out = run::go_back(&go_back_args(&h), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::Refused);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
}

#[test]
fn go_back_whose_app_does_not_start_says_so_and_does_not_reverse() {
    let h = H::new().with_installed();
    h.script(&[Behaves::Crashes]);
    let out = run::go_back(&go_back_args(&h), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::StartFailed, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.0"));
    assert_eq!(h.journal(&out).state, "start-failed");
}

// ------------------------------------------------------------ CLI and pieces

fn parse(args: &[&str]) -> Result<cli::Mode, String> {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    cli::parse(
        &args,
        None,
        Some("/tmp/state".into()),
        Some("/tmp/Sessions".into()),
        1_763_200_000_000,
    )
}

#[test]
fn the_command_line_parses_install_go_back_and_status() {
    let Ok(cli::Mode::Install(a)) = parse(&[
        "--staged",
        "/tmp/x/Hollow.app",
        "--expected-version",
        "0.6.2",
    ]) else {
        panic!("install");
    };
    assert_eq!(a.team, Team::Id(DEFAULT_TEAM.into()));
    assert_eq!(a.app, PathBuf::from("/Applications/Hollow.app"));
    assert_eq!(
        a.snapshot_source,
        Some((PathBuf::from("/tmp/Sessions"), false))
    );
    assert!(a.relaunch && !a.allow_downgrade);
    assert!(a.install_id.starts_with("20251115-"), "{}", a.install_id);

    let Ok(cli::Mode::Install(a)) = parse(&[
        "--staged",
        "/tmp/x/Hollow.app",
        "--expected-version",
        "0.6.2",
        "--expected-team",
        "-",
        "--no-snapshot",
        "--no-relaunch",
        "--allow-downgrade",
        "--app",
        "/tmp/a/Hollow.app",
        "--journal-dir",
        "/tmp/j",
        "--install-id",
        "a b",
        "--settle-secs",
        "3",
    ]) else {
        panic!("install with options");
    };
    assert_eq!(a.team, Team::AdHoc);
    assert_eq!(a.snapshot_source, None);
    assert!(!a.relaunch && a.allow_downgrade);
    assert_eq!(a.state_dir, PathBuf::from("/tmp/j"));
    assert_eq!(a.install_id, "a_b");
    assert_eq!(a.timing.settle_ms, 3_000);

    let args: Vec<String> = ["--staged", "/x/Hollow.app", "--expected-version", "1"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let Ok(cli::Mode::Install(a)) = cli::parse(
        &args,
        Some("ABCDE12345".into()),
        Some("/s".into()),
        Some("/t".into()),
        0,
    ) else {
        panic!("env team")
    };
    assert_eq!(a.team, Team::Id("ABCDE12345".into()));

    assert!(matches!(parse(&["--go-back"]), Ok(cli::Mode::GoBack(_))));
    assert!(matches!(parse(&["--status"]), Ok(cli::Mode::Status { .. })));
    assert!(matches!(parse(&["--help"]), Ok(cli::Mode::Help)));
}

#[test]
fn a_bad_command_line_is_a_usage_error() {
    for bad in [
        vec![],
        vec!["--staged", "/x/Hollow.app"],
        vec!["--expected-version", "1"],
        vec!["--bogus"],
        vec!["--staged"],
        vec!["--go-back", "--status"],
        vec!["--status", "--app", "relative/Hollow.app"],
        vec!["--status", "--app", "/x/NotAnApp"],
        vec![
            "--staged",
            "/x/Hollow.app",
            "--expected-version",
            "1",
            "--expected-team",
            "no way",
        ],
        vec!["--go-back", "--settle-secs", "soon"],
        vec!["--go-back", "--journal-dir", "rel"],
    ] {
        assert!(parse(&bad).is_err(), "{bad:?}");
    }
    let args = vec![
        "--staged".to_string(),
        "/x/Hollow.app".into(),
        "--expected-version".into(),
        "1".into(),
    ];
    assert!(
        cli::parse(&args, None, Some("/s".into()), None, 0).is_err(),
        "no Sessions default needs a flag"
    );
    assert!(cli::USAGE.contains("--go-back") && cli::USAGE.contains("8 new version did not start"));
}

#[test]
fn every_exit_status_has_a_distinct_code_and_name() {
    let all = [
        Exit::Ok,
        Exit::Failed,
        Exit::Usage,
        Exit::Busy,
        Exit::Refused,
        Exit::NotWritable,
        Exit::SnapshotFailed,
        Exit::AppRunning,
        Exit::WentBack,
        Exit::NeedsHand,
        Exit::NothingToGoBack,
        Exit::StartFailed,
        Exit::RestoreFailed,
    ];
    let codes: HashSet<i32> = all.iter().map(|e| e.code()).collect();
    let names: HashSet<&str> = all.iter().map(|e| e.name()).collect();
    assert_eq!(codes.len(), all.len());
    assert_eq!(names.len(), all.len());
    assert_eq!(Exit::WentBack.code(), 8);
    let line = cli::result_line(&super::Outcome::new(Exit::Busy, "x"));
    assert_eq!(line["exit"], 3);
    assert_eq!(line["code"], "busy");
    assert_eq!(line["ok"], false);
}

#[test]
fn status_reads_both_apps_the_lock_and_the_last_failure_without_writing() {
    let h = H::new().with_installed();
    let s = cli::status(&h.app(), &h.state, &h.deps(&|_| {}));
    assert_eq!(s["installed"]["version"], "0.6.1");
    assert_eq!(s["previous"]["version"], "0.6.0");
    assert_eq!(s["previous"]["team"], DEFAULT_TEAM);
    assert!(s["lock"].is_null() && s["lastFailure"].is_null());
    assert!(!h.state.exists());
}

#[test]
fn small_parsers_read_codesign_ps_and_versions() {
    use super::effects::{app_pids, team_from_codesign};
    assert_eq!(
        team_from_codesign("Identifier=x\nTeamIdentifier=TEAM000000\n"),
        Ok(Some("TEAM000000".into()))
    );
    assert_eq!(
        team_from_codesign("Signature=adhoc\nTeamIdentifier=not set\n"),
        Ok(None)
    );
    // Only a line that starts with the key counts, and only one may.
    assert_eq!(
        team_from_codesign("Identifier=a TeamIdentifier=EVIL000000\nTeamIdentifier=TEAM000000\n"),
        Ok(Some("TEAM000000".into()))
    );
    assert!(team_from_codesign("TeamIdentifier=EVIL000000\nTeamIdentifier=TEAM000000\n").is_err());
    assert!(team_from_codesign("Identifier=x\n").is_err());
    let ps = "  101 /A/Hollow.app/Contents/MacOS/hollow\n  102 /A/Hollow.app/Contents/MacOS/hollow team-mcp\n  103 /A/Hollow.app/Contents/MacOS/hollow -psn_0_1\n  104 /A/Hollow.app/Contents/MacOS/hollow install-update --go-back\n  105 /bin/zsh\n";
    assert_eq!(
        app_pids(ps, Path::new("/A/Hollow.app/Contents/MacOS/hollow")),
        vec![101, 103]
    );
    use std::cmp::Ordering::*;
    assert_eq!(verify::compare_versions("0.6.10", "0.6.9"), Some(Greater));
    assert_eq!(verify::compare_versions("0.6", "0.6.0"), Some(Equal));
    assert_eq!(verify::compare_versions("0.6.x", "0.6.0"), None);
    assert!(verify::check_downgrade("0.6.2", "0.6.1", false).is_err());
    assert!(verify::check_downgrade("0.6.2", "0.6.2", false).is_ok());
    assert!(verify::check_downgrade("0.6.2", "weird", false).is_err());
    assert!(verify::check_downgrade("0.6.2", "0.6.1", true).is_ok());
}

#[test]
fn the_detached_command_runs_install_update_in_its_own_group_without_hollow_variables() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("hollow");
    let out = dir.path().join("seen");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$1 $2 pid=$$ pgid=$(ps -o pgid= -p $$ | tr -d ' ') stray=${{HOLLOW_RUN_TAG-unset}} keep=${{KEEP_ME-unset}}\" > {}\n",
            out.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = vec![
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("HOLLOW_RUN_TAG".into(), "run-1".into()),
        ("HOLLOW_APP_DATA_DIR".into(), "/x".into()),
        ("KEEP_ME".into(), "yes".into()),
    ];
    let mut cmd = super::detach::detached_command(
        &bin,
        &["--go-back".into()],
        &dir.path().join("log"),
        env.into_iter(),
    )
    .unwrap();
    let names: Vec<String> = cmd
        .get_envs()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|k| !k.starts_with("HOLLOW_")), "{names:?}");
    let status = cmd.spawn().unwrap().wait().unwrap();
    assert!(status.success());
    let seen = std::fs::read_to_string(&out).unwrap();
    let field = |name: &str| {
        seen.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{name}=")))
            .unwrap()
            .to_string()
    };
    assert!(seen.starts_with("install-update --go-back "), "{seen}");
    assert_eq!(field("pid"), field("pgid"), "its own process group: {seen}");
    assert_eq!(field("stray"), "unset");
    assert_eq!(
        field("keep"),
        "unset",
        "only PATH, HOME, TMPDIR and LANG pass"
    );
    assert!(
        names
            .iter()
            .all(|k| super::detach::ENV_ALLOWED.contains(&k.as_str())),
        "{names:?}"
    );
    let mode = std::fs::metadata(dir.path().join("log"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

// ------------------------------------------------------------ round 1 review

#[test]
fn the_team_requirement_checks_the_team_before_interpolating_it() {
    use super::effects::team_requirement;
    let req = team_requirement("TEAM000000").unwrap();
    assert_eq!(
        req,
        "=anchor apple generic and certificate leaf[subject.OU] = \"TEAM000000\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    );
    for bad in [
        "team000000",
        "45WB5HXFG",
        "TEAM0000003",
        "45WB5\"XFG3",
        "",
        "-",
    ] {
        assert!(team_requirement(bad).is_err(), "{bad}");
    }
    assert!(Team::parse("team000000").is_err());
    assert!(Team::parse("SHORT").is_err());
}

#[test]
fn ditto_keeps_forks_attributes_and_acls_and_drops_quarantine() {
    use super::effects::ditto_args;
    assert_eq!(
        ditto_args(true),
        ["--rsrc", "--extattr", "--acl", "--noqtn", "--clone"]
    );
    assert_eq!(
        ditto_args(false),
        ["--rsrc", "--extattr", "--acl", "--noqtn"]
    );
}

#[test]
fn snapshot_prune_keeps_the_newest_three_and_current_and_ignores_strays() {
    use super::snapshot::prune;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let make = |name: &str, age_secs: u64| {
        let p = root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        let when = std::time::SystemTime::now() - Duration::from_secs(age_secs);
        std::fs::File::open(&p).unwrap().set_modified(when).unwrap();
        p
    };
    // Names sort the other way round from their ages: mtime decides.
    let a = make("a-from-0.6.1", 10);
    let b = make("b-from-0.6.1", 20);
    let c = make("c-from-0.6.1", 30);
    let d = make("d-from-0.6.1", 40);
    let current = make("e-from-0.6.1", 50);
    let stray = make("not-a-snapshot", 60);
    let target = make("link-target", 70);
    std::os::unix::fs::symlink(&target, root.join("z-from-link")).unwrap();
    prune(root, 3, Some(&current));
    assert!(
        a.exists() && b.exists() && c.exists(),
        "the newest three stay"
    );
    assert!(!d.exists(), "the fourth newest goes");
    assert!(current.exists(), "current is never removed");
    assert!(stray.exists(), "a folder without -from- is not ours");
    assert!(target.exists());
    assert!(
        std::fs::symlink_metadata(root.join("z-from-link")).is_ok(),
        "a symlink is never followed or removed"
    );
}

#[test]
fn the_snapshot_is_removed_when_a_snapshot_fails_and_kept_after_failed_starts() {
    for case in ["snapshot", "went-back"] {
        let h = H::new().with_installed();
        let support = h.root.join("support");
        let conn = sessions_db(&support.join("Sessions"), "vault.db", 2);
        if case == "snapshot" {
            // A second, broken database fails the snapshot after the first
            // copy was written.
            std::fs::write(support.join("Sessions/zz.db"), vec![0x5au8; 8192]).unwrap();
        } else {
            h.script(&[Behaves::Crashes, Behaves::Crashes]);
        }
        drop(conn);
        let staged = h.staged("0.6.2", TEAM_OK);
        let mut args = h.args(&staged, "0.6.2");
        args.snapshot_source = Some((support.clone(), true));
        let out = h.install(&args);
        let want = if case == "snapshot" {
            Exit::SnapshotFailed
        } else {
            Exit::WentBack
        };
        assert_eq!(out.exit, want, "{case}: {}", out.message);
        let snaps = h.state.join("snapshots");
        let left = std::fs::read_dir(&snaps).map(|d| d.count()).unwrap_or(0);
        if case == "snapshot" {
            assert_eq!(left, 0, "{case}: the partial snapshot was removed");
        } else {
            // The new version ran, and may have migrated the stores: U6
            // restores from this one.
            assert_eq!(left, 1, "{case}: the pre-update snapshot is kept");
        }
    }
}

#[test]
fn a_closed_wal_database_without_wal_or_shm_files_snapshots_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let conn = sessions_db(dir.path(), "vault.db", 5);
    drop(conn);
    assert!(!dir.path().join("vault.db-wal").exists());
    assert!(!dir.path().join("vault.db-shm").exists());
    let snap =
        super::snapshot::take(&[dir.path().join("vault.db")], &dir.path().join("out")).unwrap();
    assert_eq!(snap.files[0].user_version, 5);
}

#[test]
fn a_single_instance_handoff_during_the_reopen_counts_as_started() {
    let h = H::new().with_installed();
    h.script(&[Behaves::HandsOff]);
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(h.journal(&out).attempts.len(), 1);
}

#[test]
fn stop_terms_only_the_process_this_installer_started() {
    use super::health::{stop, Ours};
    let h = H::new();
    let exe = h.app().join("Contents/MacOS/hollow");
    {
        let mut w = h.world.borrow_mut();
        w.procs.insert(900, (exe.clone(), true, 5));
        w.starts.insert(900, 5_000);
        // 901 was ours, but the pid now belongs to a later process.
        w.procs.insert(901, (exe.clone(), true, 9));
        w.starts.insert(901, 9_000);
    }
    let deps = h.deps(&|_| {});
    let ours = [
        Ours {
            pid: 900,
            started: Some(5_000),
        },
        Ours {
            pid: 901,
            started: Some(6_000),
        },
    ];
    let err = stop(&exe, &ours, h.clock.now_ms(), &deps, &Timing::default()).unwrap_err();
    assert_eq!(h.world.borrow().terminated, vec![900]);
    assert!(err.contains("another Hollow (pid 901)"), "{err}");
    assert!(h.world.borrow().procs.contains_key(&901), "never killed");
}

#[test]
fn another_hollow_running_after_a_failed_start_is_reported_and_left_alone() {
    let h = H::new().with_installed();
    h.script(&[Behaves::Crashes]);
    let world = h.world.clone();
    let exe = h.app().join("Contents/MacOS/hollow");
    let hook = move |s: &str| {
        if s == "promoted" {
            // Started long before the launch: not this installer's.
            world.borrow_mut().procs.insert(777, (exe.clone(), true, 1));
        }
    };
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = run::install(&h.args(&staged, "0.6.2"), &h.deps(&hook));
    assert_eq!(out.exit, Exit::NeedsHand, "{}", out.message);
    assert!(out.message.contains("pid 777"), "{}", out.message);
    assert!(h.world.borrow().terminated.is_empty());
    assert!(h.world.borrow().procs.contains_key(&777));
}

#[test]
fn wait_flags_are_capped() {
    assert!(parse(&["--go-back", "--settle-secs", "3600"]).is_ok());
    assert!(parse(&["--go-back", "--settle-secs", "3601"]).is_err());
    assert!(parse(&["--go-back", "--quit-timeout-secs", "18446744073709551615"]).is_err());
}

// ------------------------------------------------------------ round 2 review

#[test]
fn a_rollback_before_any_start_removes_the_snapshot_it_never_needed() {
    let h = H::new().with_installed();
    let support = h.root.join("support");
    drop(sessions_db(&support.join("Sessions"), "vault.db", 2));
    // Crash after the swap, before its journal write: recovery undoes it.
    h.fs.crash_at.set(Some((0, true)));
    let staged = h.staged("0.6.2", TEAM_OK);
    let mut args = h.args(&staged, "0.6.2");
    args.snapshot_source = Some((support, true));
    let deps = h.deps(&|_| {});
    let crashed =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run::install(&args, &deps)));
    assert!(crashed.is_err());
    h.fs.crash_at.set(None);
    assert_eq!(
        std::fs::read_dir(h.state.join("snapshots"))
            .unwrap()
            .count(),
        1
    );
    recover(&h).unwrap();
    assert_eq!(only_journal(&h).record.state, "rolled-back");
    assert!(only_journal(&h).record.attempts.is_empty());
    assert_eq!(
        std::fs::read_dir(h.state.join("snapshots"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn the_run_after_an_exit_9_with_one_failed_start_goes_back_instead_of_keeping_it() {
    let h = H::new().with_installed();
    h.script(&[Behaves::Crashes]);
    let world = h.world.clone();
    let exe = h.app().join("Contents/MacOS/hollow");
    let hook = move |s: &str| {
        if s == "promoted" {
            world.borrow_mut().procs.insert(777, (exe.clone(), true, 1));
        }
    };
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = run::install(&h.args(&staged, "0.6.2"), &h.deps(&hook));
    assert_eq!(out.exit, Exit::NeedsHand, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    // The other Hollow quit; the next run recovers.
    h.world.borrow_mut().procs.clear();
    recover(&h).unwrap();
    let j = only_journal(&h);
    assert_eq!(
        j.record.state, "rolled-back",
        "never healthy confirmed:false"
    );
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert_eq!(version_of(&h.layout().previous).as_deref(), Some("0.6.0"));
    assert!(h.leftovers().is_empty());
    let failure: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(h.state.join(run::FAILURE_FILE)).unwrap())
            .unwrap();
    assert_eq!(failure["wentBack"], true);
}

#[test]
fn a_set_aside_journal_that_was_moving_apps_blocks_until_a_person_looks() {
    let h = H::new().with_installed();
    let dir = h.state.join(journal::DIR);
    std::fs::create_dir_all(&dir).unwrap();
    // Readable JSON, but `plan` and `movesDone` are missing in a moving state.
    std::fs::write(
        dir.join("y.json"),
        r#"{"schema":1,"kind":"install","id":"y","pid":1,"startedAt":"","state":"promoted","app":"/x/Hollow.app"}"#,
    )
    .unwrap();
    for run in 0..2 {
        let staged = h.staged("0.6.2", TEAM_OK);
        let out = h.install(&h.args(&staged, "0.6.2"));
        assert_eq!(out.exit, Exit::NeedsHand, "run {run}: {}", out.message);
        assert!(
            out.message.contains("middle of moving apps"),
            "{}",
            out.message
        );
        assert!(dir.join("y.json.bad").exists());
        assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    }
}

#[test]
fn an_old_set_aside_journal_no_longer_blocks_the_leftover_retry_and_extras_are_pruned() {
    let h = H::new().with_installed();
    let dir = h.state.join(journal::DIR);
    std::fs::create_dir_all(&dir).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    for k in 0..7 {
        let p = dir.join(format!("old{k}.json.bad"));
        std::fs::write(&p, "{ half").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(old - Duration::from_secs(k))
            .unwrap();
    }
    let staged = h.staged("0.6.2", TEAM_OK);
    assert_eq!(h.install(&h.args(&staged, "0.6.2")).exit, Exit::Ok);
    let bad = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".bad")
        })
        .count();
    assert_eq!(bad, 5, "set-aside journals beyond 5 are pruned");
    // A discard whose removal failed, with only older .bad files around.
    make_bundle(&h.layout().discard, "0.6.0", TEAM_OK);
    h.world.borrow_mut().procs.clear();
    let staged = h.staged("0.6.3", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.3"));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert!(h.leftovers().is_empty());
}

#[test]
fn a_late_handoff_is_found_within_the_poll_and_gets_a_whole_settle_time() {
    let h = H::new().with_installed();
    h.script(&[Behaves::HandsOffLate]);
    let exe = h.app().join("Contents/MacOS/hollow");
    let timing = Timing::default();
    let before = h.clock.now_ms();
    let got = super::health::attempt(&h.app(), &exe, &h.deps(&|_| {}), &timing);
    let super::health::Start::Healthy { pid } = got.start else {
        panic!("{:?}", got.start);
    };
    assert_eq!(pid, 50_002, "the successor");
    assert_eq!(got.ours.len(), 2);
    assert!(
        h.clock.now_ms() - before >= 500 + timing.settle_ms,
        "the settle clock restarted"
    );
}

#[test]
fn a_process_that_appears_after_the_start_timeout_is_named_as_a_late_start() {
    use super::health::stop;
    let h = H::new();
    let exe = h.app().join("Contents/MacOS/hollow");
    let launched = h.clock.now_ms();
    {
        let mut w = h.world.borrow_mut();
        w.procs.insert(950, (exe.clone(), true, launched + 61_000));
    }
    let err = stop(&exe, &[], launched, &h.deps(&|_| {}), &Timing::default()).unwrap_err();
    assert!(
        err.contains("started after the reopen check gave up"),
        "{err}"
    );
    assert!(!err.contains("another Hollow"), "{err}");
    assert!(h.world.borrow().terminated.is_empty());
}

// ------------------------------------------------------------ round 3 review

#[test]
fn recovery_keeps_an_install_whose_last_attempt_was_healthy() {
    // [failed, healthy], then a crash before `healthy` was written.
    let seen = Cell::new(0);
    let hook = move |s: &str| {
        if s == "started" {
            seen.set(seen.get() + 1);
            if seen.get() == 2 {
                panic!("injected crash after the healthy try");
            }
        }
    };
    let h = crashed_install(|_| {}, &hook, &[Behaves::Crashes, Behaves::Healthy]);
    let outcomes: Vec<String> = only_journal(&h)
        .record
        .attempts
        .iter()
        .map(|a| a.outcome.clone())
        .collect();
    assert_eq!(outcomes, ["exited", "healthy"]);
    recover(&h).unwrap();
    let j = only_journal(&h);
    assert_eq!(j.record.state, "healthy");
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert_eq!(version_of(&h.layout().previous).as_deref(), Some("0.6.1"));
    assert!(h.leftovers().is_empty());
    // No wedge: the next install goes ahead.
    h.world.borrow_mut().procs.clear();
    let staged = h.staged("0.6.3", TEAM_OK);
    assert_eq!(h.install(&h.args(&staged, "0.6.3")).exit, Exit::Ok);
}

#[test]
fn recovery_rolls_back_an_install_whose_last_attempt_failed() {
    let hook = |s: &str| {
        if s == "started" {
            panic!("injected crash after the failed try");
        }
    };
    let h = crashed_install(|_| {}, &hook, &[Behaves::Crashes]);
    recover(&h).unwrap();
    assert_eq!(only_journal(&h).record.state, "rolled-back");
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert!(h.state.join(run::FAILURE_FILE).exists());
}

#[test]
fn xattr_clears_quarantine_without_following_symlinks() {
    assert_eq!(
        super::effects::xattr_args(),
        ["-d", "-r", "-s", "com.apple.quarantine"]
    );
}

#[test]
fn an_app_that_keeps_respawning_is_a_failed_start_within_the_watch_cap() {
    let h = H::new().with_installed();
    h.script(&[Behaves::Respawns]);
    let exe = h.app().join("Contents/MacOS/hollow");
    let timing = Timing::default();
    let before = h.clock.now_ms();
    let got = super::health::attempt(&h.app(), &exe, &h.deps(&|_| {}), &timing);
    assert_eq!(got.start, super::health::Start::NeverSettled);
    let took = h.clock.now_ms() - before;
    assert!(
        took >= timing.watch_cap_ms() && took <= timing.watch_cap_ms() + timing.poll_ms,
        "{took}"
    );
    assert!(got.ours.len() > 3, "it followed several handoffs first");
    assert!(got
        .start
        .detail(&timing)
        .contains("each one exited and another started"));
}

#[test]
fn the_blocking_set_aside_message_says_how_to_go_back_by_hand() {
    let h = H::new().with_installed();
    let dir = h.state.join(journal::DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("y.json"), r#"{"state":"started"}"#).unwrap();
    let staged = h.staged("0.6.2", TEAM_OK);
    let out = h.install(&h.args(&staged, "0.6.2"));
    assert_eq!(out.exit, Exit::NeedsHand);
    for needle in [
        ".Hollow.app.previous",
        "install-macos-app.mjs --rollback",
        "--go-back is blocked",
    ] {
        assert!(out.message.contains(needle), "{needle}: {}", out.message);
    }
}
