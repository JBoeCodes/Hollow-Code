//! The client end to end on fakes: a scripted channel, an in-memory token,
//! a fake codesign, scripted running work, a recording installer launcher,
//! an instant clock and temp folders. No network, Keychain, `/Applications`
//! or real sleep.

use super::client::{
    self, Client, Deps, Helper, LinkConnect, Paths, Sink, Started, Work, NO_READY,
};
use super::config::{self, Config};
use super::fetch::{CheckError, Fetcher, Pending};
use super::install::{self, UpdateInstallOutcome, UpdateWork, WaitEnd};
use super::last::{self, UpdateLastResult};
use super::state::{Channel, UpdatePhase, UpdateState};
use super::token::TokenStore;
use super::unpack::Limits;
use super::unpack_tests::{targz, M};
use crate::update_install::effects::{Clock, Signer};
use reqwest::Url;
use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const TOKEN: &str = "Tq3sVb8xYk1mN0pQrStUvWxYz0123456789-_AbCdEf";
const HOST: &str = "https://updates.example.com";

// ── Fakes ───────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct Channelled {
    script: Arc<Mutex<VecDeque<Result<Option<(serde_json::Value, Vec<u8>)>, CheckError>>>>,
    checks: Arc<Mutex<Vec<(String, String)>>>,
    downloads: Arc<AtomicUsize>,
    /// Runs once inside the next check (something the person does while
    /// the request is out).
    during_check: Hook,
    /// Runs once inside the next download.
    during_download: Hook,
    /// Downloads the client stopped through `progress`.
    stopped: Arc<AtomicUsize>,
}

type Hook = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;

fn run_hook(hook: &Hook) {
    let f = hook.lock().unwrap().take();
    if let Some(f) = f {
        f();
    }
}

struct FakePending {
    manifest: serde_json::Value,
    bytes: Vec<u8>,
    downloads: Arc<AtomicUsize>,
    during_download: Hook,
    stopped: Arc<AtomicUsize>,
}

impl Pending for FakePending {
    fn manifest(&self) -> &serde_json::Value {
        &self.manifest
    }
    fn download_url(&self) -> Url {
        Url::parse(
            self.manifest["platforms"]["darwin-aarch64"]["url"]
                .as_str()
                .unwrap(),
        )
        .unwrap()
    }
    fn download(
        &self,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Vec<u8>, String> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        run_hook(&self.during_download);
        let total = self.bytes.len() as u64;
        for got in [total / 2, total] {
            if !progress(got, Some(total)) {
                self.stopped.fetch_add(1, Ordering::SeqCst);
                return Err(super::fetch::STOPPED.into());
            }
        }
        Ok(self.bytes.clone())
    }
}

impl Fetcher for Channelled {
    fn check(&self, endpoint: &Url, token: &str) -> Result<Option<Box<dyn Pending>>, CheckError> {
        self.checks
            .lock()
            .unwrap()
            .push((endpoint.to_string(), token.to_string()));
        run_hook(&self.during_check);
        let next = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(CheckError::NothingPublished));
        next.map(|found| {
            found.map(|(manifest, bytes)| {
                Box::new(FakePending {
                    manifest,
                    bytes,
                    downloads: self.downloads.clone(),
                    during_download: self.during_download.clone(),
                    stopped: self.stopped.clone(),
                }) as Box<dyn Pending>
            })
        })
    }
}

#[derive(Clone, Default)]
struct Tokens(Arc<Mutex<Option<String>>>);

impl TokenStore for Tokens {
    fn get(&self) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn set(&self, token: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(token.into());
        Ok(())
    }
    fn delete(&self) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

/// codesign stand-in: an ad hoc signature that verifies unless told not to.
#[derive(Clone, Default)]
struct FakeSign(Arc<AtomicBool>);

impl Signer for FakeSign {
    fn verify(&self, _bundle: &Path) -> Result<(), String> {
        if self.0.load(Ordering::SeqCst) {
            Err("code object is not signed at all".into())
        } else {
            Ok(())
        }
    }
    fn team(&self, _bundle: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn verify_requirement(&self, _bundle: &Path, _requirement: &str) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct FakeWork {
    running: Arc<Mutex<Vec<UpdateWork>>>,
    blocked: Arc<Mutex<Option<String>>>,
    /// After this many looks at `running`, the work ends.
    ends_after: Arc<Mutex<Option<usize>>>,
    /// On this look, this work starts (a chat sent between two looks).
    starts_on: Arc<Mutex<Option<(usize, UpdateWork)>>>,
    /// On this look, this runs first (something changes between looks).
    during: Arc<Mutex<Option<(usize, WorkHook)>>>,
    looks: Arc<AtomicUsize>,
}

type WorkHook = Box<dyn FnOnce() + Send>;

impl Work for FakeWork {
    fn running(&self) -> Result<Vec<UpdateWork>, String> {
        let n = self.looks.fetch_add(1, Ordering::SeqCst) + 1;
        let hook = {
            let mut slot = self.during.lock().unwrap();
            if slot.as_ref().is_some_and(|(at, _)| *at == n) {
                slot.take().map(|(_, f)| f)
            } else {
                None
            }
        };
        if let Some(hook) = hook {
            hook();
        }
        let starts = {
            let mut slot = self.starts_on.lock().unwrap();
            if slot.as_ref().is_some_and(|(at, _)| *at == n) {
                slot.take().map(|(_, w)| w)
            } else {
                None
            }
        };
        if let Some(work) = starts {
            self.running.lock().unwrap().push(work);
        }
        if self
            .ends_after
            .lock()
            .unwrap()
            .is_some_and(|after| n >= after)
        {
            self.running.lock().unwrap().clear();
        }
        Ok(self.running.lock().unwrap().clone())
    }
    fn confirm(&self, keys: &[String]) -> Result<(), String> {
        let running = self.running.lock().unwrap().clone();
        if running.iter().all(|w| keys.contains(&w.key)) {
            Ok(())
        } else {
            Err("RESTART_WORK_CHANGED: Running work changed; confirm again".into())
        }
    }
    fn blocked(&self) -> Option<String> {
        self.blocked.lock().unwrap().clone()
    }
}

#[derive(Clone, Default)]
struct FakeHelper {
    spawned: Arc<Mutex<Vec<(PathBuf, Vec<OsString>, PathBuf)>>>,
    /// Quits that went ahead (the last look said yes).
    quits: Arc<AtomicUsize>,
    /// Quits the last look called off.
    aborted: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
    /// The installer dies at once.
    dies: Arc<AtomicBool>,
    /// It dies after the brief watch, before the quit.
    dies_later: Arc<AtomicBool>,
}

struct FakeStarted {
    dies: bool,
    dies_later: bool,
    stops: Arc<AtomicUsize>,
}

impl Started for FakeStarted {
    fn exited_early(&mut self) -> Option<String> {
        self.dies
            .then(|| "The installer stopped as soon as it started (exit status: 4)".to_string())
    }
    fn exited(&mut self) -> Option<String> {
        self.dies_later
            .then(|| "the installer stopped before Hollow quit".to_string())
    }
    fn stop(&mut self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}

impl Helper for FakeHelper {
    fn spawn(&self, exe: &Path, args: &[OsString], log: &Path) -> Result<Box<dyn Started>, String> {
        self.spawned
            .lock()
            .unwrap()
            .push((exe.into(), args.to_vec(), log.into()));
        Ok(Box::new(FakeStarted {
            dies: self.dies.load(Ordering::SeqCst),
            dies_later: self.dies_later.load(Ordering::SeqCst),
            stops: self.stops.clone(),
        }))
    }
    /// The quit's last look runs at once (no delay in tests).
    fn quit(&self, last_look: Box<dyn FnOnce() -> bool + Send>) {
        if last_look() {
            self.quits.fetch_add(1, Ordering::SeqCst);
        } else {
            self.aborted.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// Time moves only when slept.
#[derive(Clone, Default)]
struct FakeClock {
    now: Arc<AtomicU64>,
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        1_763_200_000_000 + self.now.load(Ordering::SeqCst)
    }
    fn sleep(&self, d: Duration) {
        self.now.fetch_add(d.as_millis() as u64, Ordering::SeqCst);
        self.sleeps.lock().unwrap().push(d);
    }
}

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<String>>>);

impl Sink for Events {
    fn state(&self, state: &UpdateState) {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_string(state).unwrap());
    }
}

impl Events {
    fn phases(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|s| {
                let v: serde_json::Value = serde_json::from_str(s).unwrap();
                v["phase"]["phase"].as_str().unwrap().to_string()
            })
            .collect()
    }
}

pub(crate) struct Rig {
    _dir: tempfile::TempDir,
    pub(crate) root: PathBuf,
    channel: Channelled,
    tokens: Tokens,
    sign_fails: Arc<AtomicBool>,
    work: FakeWork,
    helper: FakeHelper,
    clock: FakeClock,
    events: Events,
    pub(crate) client: Arc<Client>,
}

pub(crate) struct Setup {
    pub(crate) pubkey: &'static str,
    pub(crate) token: Option<&'static str>,
    pub(crate) running: &'static str,
    pub(crate) installed: bool,
    /// Another host and channel (the local server of `fixture_tests`).
    pub(crate) host: Option<String>,
    pub(crate) fetcher: Option<Box<dyn Fetcher>>,
    pub(crate) naps: client::Naps,
    /// A review copy (row U10's Local build refusal).
    pub(crate) review_copy: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            pubkey: "RWQ-test-key",
            token: Some(TOKEN),
            running: "0.6.10",
            installed: true,
            host: None,
            fetcher: None,
            naps: client::Naps::Clock,
            review_copy: false,
        }
    }
}

pub(crate) fn rig(setup: Setup) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let channel = Channelled::default();
    let tokens = Tokens::default();
    if let Some(t) = setup.token {
        tokens.set(t).unwrap();
    }
    let sign = FakeSign::default();
    let (work, helper, clock, events) = (
        FakeWork::default(),
        FakeHelper::default(),
        FakeClock::default(),
        Events::default(),
    );
    let app = root.join("Applications").join("Hollow.app");
    std::fs::create_dir_all(&app).unwrap();
    let client = Arc::new(Client::new(Deps {
        config: Config {
            host: config::host_from(setup.host.clone(), false),
            pubkey: setup.pubkey.into(),
            team: config::team_from(Some("-".into())),
            running_version: setup.running.into(),
            review_copy: setup.review_copy,
        },
        paths: paths(&root, setup.installed),
        fetcher: setup.fetcher.unwrap_or_else(|| Box::new(channel.clone())),
        tokens: Box::new(tokens.clone()),
        signer: Box::new(sign.clone()),
        work: Box::new(work.clone()),
        helper: Box::new(helper.clone()),
        clock: Box::new(clock.clone()),
        sink: Box::new(events.clone()),
        limits: Limits::default(),
        own_pid: 1111,
        naps: setup.naps,
    }));
    // As the schedule thread does first (the app's first Keychain read).
    client.load_connection();
    Rig {
        _dir: dir,
        root,
        channel,
        tokens,
        sign_fails: sign.0,
        work,
        helper,
        clock,
        events,
        client,
    }
}

fn paths(root: &Path, installed: bool) -> Paths {
    Paths {
        settings: root.join("data").join("update-settings.json"),
        staging: root.join("data").join("updates").join("staging"),
        last: root.join("data").join("updates").join(last::FILE),
        installed_app: installed.then(|| root.join("Applications").join("Hollow.app")),
        state_dir: Some(root.join("data").join("installer")),
        sessions: None,
    }
}

pub(crate) fn info_plist(version: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.hollow.app</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleExecutable</key><string>hollow</string>
</dict></plist>"#
    )
    .into_bytes()
}

/// A `Hollow.app.tar.gz` whose Info.plist says `version`.
pub(crate) fn app_archive(version: &str) -> Vec<u8> {
    let plist = info_plist(version);
    targz(&[
        M::Dir("Hollow.app/"),
        M::Dir("Hollow.app/Contents/"),
        M::File("Hollow.app/Contents/Info.plist", &plist, 0o644),
        M::Dir("Hollow.app/Contents/MacOS/"),
        M::File("Hollow.app/Contents/MacOS/hollow", b"#!/bin/sh\n", 0o755),
    ])
}

fn manifest(version: &str, channel: &str) -> serde_json::Value {
    serde_json::json!({
        "version": version,
        "notes": format!("What's new in {version}"),
        "pub_date": "2026-11-15T12:00:00Z",
        "platforms": { "darwin-aarch64": {
            "signature": "c2lnbmF0dXJl",
            "url": format!("{HOST}/builds/{version}/Hollow.app.tar.gz"),
        }},
        "channel": channel,
    })
}

impl Rig {
    pub(crate) fn phase_now(&self) -> UpdatePhase {
        self.phase()
    }
    pub(crate) fn publish(&self, version: &str, channel: &str) {
        self.publish_bytes(version, channel, app_archive(version));
    }
    fn publish_bytes(&self, version: &str, channel: &str, bytes: Vec<u8>) {
        self.channel
            .script
            .lock()
            .unwrap()
            .push_back(Ok(Some((manifest(version, channel), bytes))));
    }
    fn answer(&self, result: Result<Option<(serde_json::Value, Vec<u8>)>, CheckError>) {
        self.channel.script.lock().unwrap().push_back(result);
    }
    fn phase(&self) -> UpdatePhase {
        self.client.state().phase
    }
    pub(crate) fn downloads(&self) -> usize {
        self.channel.downloads.load(Ordering::SeqCst)
    }
    pub(crate) fn run(&self, keys: &[&str]) {
        *self.work.running.lock().unwrap() = keys
            .iter()
            .map(|k| UpdateWork {
                key: k.to_string(),
                name: format!("chat {k}"),
            })
            .collect();
    }
    /// How many installers were started.
    pub(crate) fn spawns(&self) -> usize {
        self.helper.spawned.lock().unwrap().len()
    }
    /// codesign fails from now on (row U7's same-verify-path test).
    pub(crate) fn fail_signing(&self) {
        self.sign_fails.store(true, Ordering::SeqCst);
    }
    /// What `Work::blocked` says (a vault lease, an automation).
    pub(crate) fn block(&self, why: Option<&str>) {
        *self.work.blocked.lock().unwrap() = why.map(str::to_string);
    }
    /// On the `nth` look at running work from now, run `f` first.
    pub(crate) fn during_work_look(&self, nth: usize, f: impl FnOnce() + Send + 'static) {
        let at = self.work.looks.load(Ordering::SeqCst) + nth;
        *self.work.during.lock().unwrap() = Some((at, Box::new(f)));
    }
    /// On the `nth` look from now, `name` starts; it ends on look `ends`.
    pub(crate) fn work_starts_on_look(&self, nth: usize, ends: usize, name: &str) {
        let now = self.work.looks.load(Ordering::SeqCst);
        *self.work.starts_on.lock().unwrap() = Some((
            now + nth,
            UpdateWork {
                key: "queue:9".into(),
                name: name.into(),
            },
        ));
        *self.work.ends_after.lock().unwrap() = Some(now + ends);
    }
    /// Quits the last look called off.
    pub(crate) fn aborted(&self) -> usize {
        self.helper.aborted.load(Ordering::SeqCst)
    }
    /// The fake clock's now (ms).
    pub(crate) fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }
    /// Quits that went ahead.
    pub(crate) fn quits(&self) -> usize {
        self.helper.quits.load(Ordering::SeqCst)
    }
    pub(crate) fn args(&self) -> Vec<String> {
        let spawned = self.helper.spawned.lock().unwrap();
        assert_eq!(spawned.len(), 1, "one installer");
        spawned[0]
            .1
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }
}

pub(crate) fn ready_version(phase: &UpdatePhase) -> Option<String> {
    match phase {
        UpdatePhase::Ready { release } => Some(release.version.clone()),
        _ => None,
    }
}

// ── Inert until set up ──────────────────────────────────────────────────

#[test]
fn without_a_public_key_the_client_is_inert_and_never_asks() {
    let r = rig(Setup {
        pubkey: "",
        ..Setup::default()
    });
    assert_eq!(
        r.phase(),
        UpdatePhase::NotSetUp {
            reason: config::NOT_SET_UP.into()
        }
    );
    r.publish("0.6.11", "main");
    r.client.check();
    assert!(r.channel.checks.lock().unwrap().is_empty(), "no request");
    assert!(!r.client.state().configured);
    assert!(matches!(r.phase(), UpdatePhase::NotSetUp { .. }));
}

#[test]
fn without_a_token_it_says_how_to_connect_and_connect_stores_one() {
    let r = rig(Setup {
        token: None,
        ..Setup::default()
    });
    assert_eq!(
        r.phase(),
        UpdatePhase::NotSetUp {
            reason: config::NOT_CONNECTED.into()
        }
    );
    assert!(!r.client.state().connected);
    let refused = r.client.connect("my-secret-but-wrong").unwrap_err();
    assert!(!refused.contains("my-secret-but-wrong"));
    assert_eq!(r.tokens.get().unwrap(), None);
    assert!(r.client.connect(&format!(" {TOKEN} ")).unwrap());
    assert_eq!(r.tokens.get().unwrap().as_deref(), Some(TOKEN));
    let state = r.client.state();
    assert!(state.connected);
    assert_eq!(state.phase, UpdatePhase::Idle);
    // The check sends it as the bearer token to the channel endpoint.
    r.client.check();
    assert_eq!(
        r.channel.checks.lock().unwrap()[0],
        (format!("{HOST}/channels/main.json"), TOKEN.to_string())
    );
    // Disconnect removes it.
    assert!(!r.client.disconnect().unwrap());
    assert_eq!(r.tokens.get().unwrap(), None);
    assert!(matches!(r.phase(), UpdatePhase::NotSetUp { .. }));
}

// ── Check, download, verify ─────────────────────────────────────────────

#[test]
fn a_newer_build_downloads_verifies_and_only_then_is_ready() {
    let r = rig(Setup::default());
    r.events.0.lock().unwrap().clear();
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    let phases = r.events.phases();
    assert_eq!(phases.first().map(String::as_str), Some("checking"));
    assert_eq!(phases.last().map(String::as_str), Some("ready"));
    assert!(phases.contains(&"available".to_string()));
    assert!(phases.contains(&"downloading".to_string()));
    assert_eq!(
        phases.iter().filter(|p| *p == "ready").count(),
        1,
        "ready only once, at the end"
    );
    let bundle = r.root.join("data/updates/staging/0.6.11/Hollow.app");
    assert!(bundle.join("Contents/Info.plist").is_file());
    assert!(r
        .root
        .join("data/updates/staging/0.6.11/ready.json")
        .is_file());
    let state = r.client.state();
    assert!(state.last_checked_at.is_some());
    match state.phase {
        UpdatePhase::Ready { release } => {
            assert_eq!(release.notes, "What's new in 0.6.11");
            assert_eq!(release.channel, Channel::Main);
            assert!(!release.downgrade);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_build_that_fails_the_apps_own_checks_never_becomes_ready() {
    // codesign refuses it.
    let r = rig(Setup::default());
    r.sign_fails.store(true, Ordering::SeqCst);
    r.publish("0.6.11", "main");
    r.client.check();
    match r.phase() {
        UpdatePhase::Failed { reason } => assert!(reason.contains("signature does not verify")),
        other => panic!("{other:?}"),
    }
    assert!(!r.events.phases().contains(&"ready".to_string()));
    assert!(!r.root.join("data/updates/staging/0.6.11").exists());
    // Its Info.plist says another version than the manifest.
    let r = rig(Setup::default());
    r.publish_bytes("0.6.11", "main", app_archive("0.6.12"));
    r.client.check();
    match r.phase() {
        UpdatePhase::Failed { reason } => assert!(reason.contains("not the expected 0.6.11")),
        other => panic!("{other:?}"),
    }
    // An archive with a path that climbs out.
    let r = rig(Setup::default());
    r.publish_bytes(
        "0.6.11",
        "main",
        targz(&[
            M::Dir("Hollow.app/"),
            M::File("Hollow.app/../x", b"x", 0o644),
        ]),
    );
    r.client.check();
    assert!(matches!(r.phase(), UpdatePhase::Failed { .. }));
    assert!(!r.root.join("data/updates/staging/x").exists());
}

#[test]
fn equal_lower_and_skipped_versions_are_not_downloaded() {
    let r = rig(Setup::default());
    r.publish("0.6.10", "main");
    r.client.check();
    assert_eq!(r.phase(), UpdatePhase::UpToDate { skipped: None });
    r.publish("0.6.9", "main");
    r.client.check();
    assert_eq!(r.phase(), UpdatePhase::UpToDate { skipped: None });
    r.client
        .settings_store_for_tests()
        .update(|s| s.skipped_version = Some("0.6.11".into()))
        .unwrap();
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(
        r.phase(),
        UpdatePhase::UpToDate {
            skipped: Some("0.6.11".into())
        }
    );
    assert_eq!(r.downloads(), 0);
    // A newer one than the skipped version is offered.
    r.publish("0.6.12", "main");
    r.client.check();
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.12"));
}

#[test]
fn a_ready_build_is_reused_not_downloaded_again_even_after_a_restart() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(r.downloads(), 1);
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    // A second client on the same folders (the next launch).
    let again = Client::new(Deps {
        config: Config {
            host: config::host_from(None, false),
            pubkey: "k".into(),
            team: config::team_from(Some("-".into())),
            running_version: "0.6.10".into(),
            review_copy: false,
        },
        paths: paths(&r.root, true),
        fetcher: Box::new(r.channel.clone()),
        tokens: Box::new(r.tokens.clone()),
        signer: Box::new(FakeSign::default()),
        work: Box::new(FakeWork::default()),
        helper: Box::new(FakeHelper::default()),
        clock: Box::new(FakeClock::default()),
        sink: Box::new(Events::default()),
        limits: Limits::default(),
        own_pid: 2,
        naps: client::Naps::Clock,
    });
    again.load_connection();
    r.publish("0.6.11", "main");
    again.check();
    assert_eq!(
        ready_version(&again.state().phase).as_deref(),
        Some("0.6.11")
    );
    assert_eq!(r.downloads(), 1, "the staged build was reused");
    // A newer version replaces it and the older folder goes.
    r.publish("0.6.12", "main");
    again.check();
    assert_eq!(
        ready_version(&again.state().phase).as_deref(),
        Some("0.6.12")
    );
    assert!(!r.root.join("data/updates/staging/0.6.11").exists());
}

#[test]
fn offline_keeps_a_ready_download_and_refusals_say_why() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.answer(Err(CheckError::Network("timed out".into())));
    r.client.check();
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    assert!(r.client.state().note.unwrap().contains("still ready"));
    r.answer(Err(CheckError::Unauthorized));
    r.client.check();
    match r.phase() {
        UpdatePhase::Failed { reason } => {
            assert!(reason.contains("did not accept this token (it may have been revoked)"))
        }
        other => panic!("{other:?}"),
    }
    r.answer(Err(CheckError::Forbidden));
    r.client.check();
    match r.phase() {
        UpdatePhase::Failed { reason } => assert!(reason.contains("can't use the Main channel")),
        other => panic!("{other:?}"),
    }
    r.answer(Err(CheckError::NothingPublished));
    r.client.check();
    assert_eq!(r.phase(), UpdatePhase::UpToDate { skipped: None });
}

#[test]
fn a_manifest_pointing_elsewhere_or_at_another_channel_is_never_downloaded() {
    let r = rig(Setup::default());
    let mut m = manifest("0.6.11", "main");
    m["platforms"]["darwin-aarch64"]["url"] =
        "https://elsewhere.example/builds/0.6.11/Hollow.app.tar.gz".into();
    r.answer(Ok(Some((m, app_archive("0.6.11")))));
    r.client.check();
    assert!(matches!(r.phase(), UpdatePhase::Failed { .. }));
    r.publish("0.6.11", "nightly");
    r.client.check();
    assert!(matches!(r.phase(), UpdatePhase::Failed { .. }));
    assert_eq!(r.downloads(), 0);
}

// ── Channel switch and downgrade ────────────────────────────────────────

#[test]
fn a_channel_switch_offers_one_lower_build_and_installs_it_with_allow_downgrade() {
    let r = rig(Setup {
        running: "0.6.20",
        ..Setup::default()
    });
    let state = r.client.set_channel(Channel::Nightly).unwrap();
    assert!(state.downgrade_allowed);
    assert!(state.note.unwrap().contains("Switched to Nightly"));
    r.publish("0.6.15", "nightly");
    r.client.check();
    match r.phase() {
        UpdatePhase::Ready { release } => assert!(release.downgrade),
        other => panic!("{other:?}"),
    }
    let outcome = r.client.install_now(None).unwrap();
    assert_eq!(
        outcome,
        UpdateInstallOutcome::Started {
            version: "0.6.15".into()
        }
    );
    assert!(r.args().contains(&"--allow-downgrade".to_string()));
    // Used once: the next lower build is not offered.
    assert!(!r.client.settings().allow_downgrade_once);
    let r2 = rig(Setup {
        running: "0.6.20",
        ..Setup::default()
    });
    r2.client.set_channel(Channel::Nightly).unwrap();
    // A channel whose newest is not lower uses the permission up.
    r2.publish("0.6.20", "nightly");
    r2.client.check();
    assert!(!r2.client.settings().allow_downgrade_once);
    r2.publish("0.6.15", "nightly");
    r2.client.check();
    assert_eq!(r2.phase(), UpdatePhase::UpToDate { skipped: None });
    // Without a switch, never.
    let r3 = rig(Setup {
        running: "0.6.20",
        ..Setup::default()
    });
    r3.publish("0.6.15", "main");
    r3.client.check();
    assert_eq!(r3.phase(), UpdatePhase::UpToDate { skipped: None });
    // Switching to the channel already set changes nothing.
    assert!(
        !r3.client
            .set_channel(Channel::Main)
            .unwrap()
            .downgrade_allowed
    );
}

#[test]
fn not_now_hides_that_version_only() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    assert!(!r.client.state().dismissed);
    assert!(r.client.dismiss("0.6.11").unwrap().dismissed);
    assert!(r.client.dismiss("../x").is_err());
    r.publish("0.6.12", "main");
    r.client.check();
    assert!(!r.client.state().dismissed, "a newer version shows again");
}

// ── Installing ──────────────────────────────────────────────────────────

#[test]
fn restart_to_update_refuses_while_work_runs_so_the_window_can_offer_the_wait() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.run(&["queue:1"]);
    match r.client.install_now(None).unwrap() {
        UpdateInstallOutcome::WorkRunning { work } => assert_eq!(work[0].key, "queue:1"),
        other => panic!("{other:?}"),
    }
    // A confirmed list that no longer covers what runs.
    r.run(&["queue:1", "queue:2"]);
    assert!(matches!(
        r.client
            .install_now(Some(&["queue:1".to_string()]))
            .unwrap(),
        UpdateInstallOutcome::WorkChanged { .. }
    ));
    // A vault lease or a build turn the list doesn't name.
    r.run(&[]);
    *r.work.blocked.lock().unwrap() = Some("Wait for the COO to finish".into());
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::WorkRunning { .. }
    ));
    assert!(r.helper.spawned.lock().unwrap().is_empty());
    assert_eq!(r.helper.quits.load(Ordering::SeqCst), 0);
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    // The person confirmed exactly what runs: it goes ahead.
    *r.work.blocked.lock().unwrap() = None;
    r.run(&["queue:1"]);
    assert!(matches!(
        r.client
            .install_now(Some(&["queue:1".to_string()]))
            .unwrap(),
        UpdateInstallOutcome::Started { .. }
    ));
}

#[test]
fn restart_to_update_starts_the_installer_from_the_staged_bundle_and_quits() {
    let r = rig(Setup::default());
    assert_eq!(r.client.install_now(None).unwrap_err(), NO_READY);
    r.publish("0.6.11", "main");
    r.client.check();
    let outcome = r.client.install_now(None).unwrap();
    assert_eq!(
        outcome,
        UpdateInstallOutcome::Started {
            version: "0.6.11".into()
        }
    );
    let (exe, _, log) = r.helper.spawned.lock().unwrap()[0].clone();
    let bundle = r.root.join("data/updates/staging/0.6.11/Hollow.app");
    assert_eq!(exe, bundle.join("Contents/MacOS/hollow"));
    assert_eq!(log, r.root.join("data/installer").join(install::LOG));
    let args = r.args();
    let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
    assert_eq!(after("--staged"), bundle.to_string_lossy());
    assert_eq!(after("--expected-version"), "0.6.11");
    assert_eq!(after("--expected-team"), "-");
    assert_eq!(
        after("--app"),
        r.root.join("Applications/Hollow.app").to_string_lossy()
    );
    assert_eq!(
        after("--journal-dir"),
        r.root.join("data/installer").to_string_lossy()
    );
    assert!(args.contains(&"--no-snapshot".to_string()));
    assert!(args.contains(&"--relaunch".to_string()));
    assert!(!args.contains(&"--allow-downgrade".to_string()));
    assert_eq!(r.helper.quits.load(Ordering::SeqCst), 1);
    assert!(matches!(r.phase(), UpdatePhase::Installing { .. }));
    // A second click does nothing more.
    assert!(r.client.install_now(None).is_err());
    // The next launch (another pid, the new version) gets the notice.
    let path = r.root.join("data/updates").join(last::FILE);
    let state_dir = r.root.join("data/installer");
    assert_eq!(
        last::read(&path, Some(&state_dir), "0.6.10", None, 1111),
        None
    );
    match last::read(&path, Some(&state_dir), "0.6.11", None, 2222) {
        Some(UpdateLastResult::Updated { version, notes, .. }) => {
            assert_eq!(version, "0.6.11");
            assert_eq!(notes, "What's new in 0.6.11");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_staged_bundle_is_checked_again_before_the_installer_runs() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.sign_fails.store(true, Ordering::SeqCst);
    let err = r.client.install_now(None).unwrap_err();
    assert!(err.contains("signature does not verify"));
    assert!(r.helper.spawned.lock().unwrap().is_empty());
    assert!(matches!(r.phase(), UpdatePhase::Failed { .. }));
    assert!(!r.root.join("data/updates/staging/0.6.11").exists());
}

#[test]
fn a_dev_run_outside_an_app_bundle_refuses_to_install() {
    let r = rig(Setup {
        installed: false,
        ..Setup::default()
    });
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(r.client.install_now(None).unwrap_err(), client::NOT_AN_APP);
    assert!(r.helper.spawned.lock().unwrap().is_empty());
    // The build itself is fine: it stays ready, with the reason beside it.
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    assert_eq!(r.client.state().note.as_deref(), Some(client::NOT_AN_APP));
    assert!(r
        .root
        .join("data/updates/staging/0.6.11/ready.json")
        .is_file());
    assert!(!r.root.join("data/updates").join(last::FILE).exists());
}

// ── Update when chats finish ────────────────────────────────────────────

#[test]
fn the_wait_has_no_expiry_and_installs_once_work_ends() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.run(&["queue:1"]);
    // Work runs for 2,000 looks: 2,000 x 5 s is close to three hours on the
    // fake clock, far past the old waiter's 30 minutes.
    *r.work.ends_after.lock().unwrap() = Some(2_000);
    let ticket = AtomicU8::new(install::WAITING);
    assert_eq!(
        r.client.wait_loop(&ticket, install::POLL),
        WaitEnd::Installed
    );
    let slept: Duration = r.clock.sleeps.lock().unwrap().iter().sum();
    assert!(slept > Duration::from_secs(2 * 60 * 60), "{slept:?}");
    assert_eq!(r.helper.spawned.lock().unwrap().len(), 1);
    assert_eq!(r.helper.quits.load(Ordering::SeqCst), 1);
}

#[test]
fn a_cancelled_wait_never_installs() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.run(&["queue:1"]);
    let ticket = AtomicU8::new(install::WAITING);
    assert!(install::cancel(&ticket));
    assert_eq!(
        r.client.wait_loop(&ticket, install::POLL),
        WaitEnd::Cancelled
    );
    assert!(r.helper.spawned.lock().unwrap().is_empty());
    // Through the client: waiting shows, Cancel clears it.
    let state = r
        .client
        .install_when_idle(Duration::from_millis(1))
        .unwrap();
    assert!(state.waiting);
    // Asking twice keeps the one wait.
    assert!(
        r.client
            .install_when_idle(Duration::from_millis(1))
            .unwrap()
            .waiting
    );
    assert!(!r.client.cancel_wait().waiting);
    assert!(r.helper.spawned.lock().unwrap().is_empty());
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    // No ready build, no wait.
    let idle = rig(Setup::default());
    assert_eq!(
        idle.client.install_when_idle(install::POLL).unwrap_err(),
        NO_READY
    );
}

#[test]
fn wait_until_idle_loops_until_told_and_stops_on_failure() {
    let ticket = AtomicU8::new(install::WAITING);
    let looks = AtomicUsize::new(0);
    let end = install::wait_until_idle(&ticket, Duration::from_secs(5), &|_| {}, || {
        Ok(looks.fetch_add(1, Ordering::SeqCst) == 10_000)
    });
    assert_eq!(end, WaitEnd::Installed);
    assert_eq!(looks.load(Ordering::SeqCst), 10_001);
    let end = install::wait_until_idle(&ticket, Duration::from_secs(5), &|_| {}, || {
        Err("the disk is full".into())
    });
    assert_eq!(end, WaitEnd::Failed("the disk is full".into()));
    // Claim, release, cancel.
    assert!(install::claim(&ticket));
    assert!(
        !install::cancel(&ticket),
        "an install that started can't be cancelled"
    );
    install::release(&ticket);
    assert!(install::cancel(&ticket));
    assert!(!install::claim(&ticket));
}

// ── The schedule, the notice, the token ─────────────────────────────────

#[test]
fn the_schedule_waits_after_launch_then_checks_every_30_minutes() {
    let r = rig(Setup::default());
    r.client
        .run_schedule(client::LAUNCH_DELAY, client::EVERY, Some(3));
    assert_eq!(
        *r.clock.sleeps.lock().unwrap(),
        vec![
            Duration::from_secs(45),
            Duration::from_secs(1800),
            Duration::from_secs(1800)
        ]
    );
    assert_eq!(r.channel.checks.lock().unwrap().len(), 3);
}

#[test]
fn the_notice_after_a_restart_names_a_went_back_and_is_acknowledged() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.client.install_now(None).unwrap();
    let args = r.args();
    let id = args[args.iter().position(|a| a == "--install-id").unwrap() + 1].clone();
    let state_dir = r.root.join("data/installer");
    std::fs::write(
        state_dir.join(crate::update_install::run::FAILURE_FILE),
        serde_json::json!({
            "schema": 1, "installId": id, "at": "2026-11-15T12:00:00Z",
            "attemptedVersion": "0.6.11", "restoredVersion": "0.6.10",
            "wentBack": true, "reason": "the new version quit twice",
        })
        .to_string(),
    )
    .unwrap();
    let path = r.root.join("data/updates").join(last::FILE);
    match last::read(&path, Some(&state_dir), "0.6.10", None, 9) {
        Some(UpdateLastResult::WentBack {
            attempted,
            restored,
            reason,
            ..
        }) => {
            assert_eq!(attempted, "0.6.11");
            assert_eq!(restored.as_deref(), Some("0.6.10"));
            assert_eq!(reason, "the new version quit twice");
        }
        other => panic!("{other:?}"),
    }
    // Another install's failure file is not this one's.
    std::fs::write(
        state_dir.join(crate::update_install::run::FAILURE_FILE),
        r#"{"installId":"other","wentBack":true,"reason":"x"}"#,
    )
    .unwrap();
    assert!(matches!(
        last::read(&path, Some(&state_dir), "0.6.10", None, 9),
        Some(UpdateLastResult::NotInstalled { reason: None, .. })
    ));
    last::acknowledge(&path).unwrap();
    assert_eq!(last::read(&path, Some(&state_dir), "0.6.11", None, 9), None);
}

#[test]
fn the_token_never_reaches_a_state_an_event_or_a_file_but_the_keychain() {
    let r = rig(Setup {
        token: None,
        ..Setup::default()
    });
    r.client.connect(TOKEN).unwrap();
    r.publish("0.6.11", "main");
    r.client.check();
    r.client.set_channel(Channel::Nightly).unwrap();
    r.answer(Err(CheckError::Unauthorized));
    r.client.check();
    r.client.set_channel(Channel::Main).unwrap();
    r.publish("0.6.11", "main");
    r.client.check();
    r.client.install_now(None).unwrap();
    for event in r.events.0.lock().unwrap().iter() {
        assert!(!event.contains(TOKEN), "{event}");
    }
    assert!(!serde_json::to_string(&r.client.state())
        .unwrap()
        .contains(TOKEN));
    for arg in r.args() {
        assert!(!arg.contains(TOKEN));
    }
    for entry in walkdir::WalkDir::new(&r.root).into_iter().flatten() {
        if entry.file_type().is_file() {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(
                !String::from_utf8_lossy(&bytes).contains(TOKEN),
                "{}",
                entry.path().display()
            );
        }
    }
}

#[test]
fn no_log_line_in_the_updater_mentions_the_token() {
    for (name, source) in [
        ("client.rs", include_str!("client.rs")),
        ("fetch.rs", include_str!("fetch.rs")),
        ("ipc.rs", include_str!("ipc.rs")),
        ("link.rs", include_str!("link.rs")),
        ("mod.rs", include_str!("mod.rs")),
        ("token.rs", include_str!("token.rs")),
        ("config.rs", include_str!("config.rs")),
    ] {
        for line in source.lines() {
            let logs = [
                "eprintln!",
                "println!",
                "elog(",
                "log::",
                "tracing::",
                "dbg!",
            ]
            .iter()
            .any(|m| line.contains(m));
            assert!(
                !(logs && line.to_lowercase().contains("token")),
                "{name}: {line}"
            );
        }
    }
    // The only place a token is formatted is the Authorization header.
    let fetch = include_str!("fetch.rs");
    assert_eq!(fetch.matches("{token}").count(), 1);
    assert!(fetch.contains("format!(\"Bearer {token}\")"));
}

#[test]
fn helper_args_and_the_running_bundle() {
    use crate::update_install::verify::Team;
    let args = install::helper_args(&install::HelperArgs {
        staged: "/s/Hollow.app".into(),
        version: "0.6.3".into(),
        team: Team::Id("TEAM000000".into()),
        app: "/Applications/Hollow.app".into(),
        state_dir: "/j".into(),
        sessions: Some("/S/Sessions".into()),
        allow_downgrade: true,
        install_id: "id-1".into(),
    });
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            "--staged",
            "/s/Hollow.app",
            "--expected-version",
            "0.6.3",
            "--expected-team",
            "TEAM000000",
            "--app",
            "/Applications/Hollow.app",
            "--journal-dir",
            "/j",
            "--snapshot-source",
            "/S/Sessions",
            "--install-id",
            "id-1",
            "--relaunch",
            "--allow-downgrade"
        ]
    );
    // And the installer's own parser takes them.
    let parsed =
        crate::update_install::cli::parse(&args, None, Some("/x".into()), None, 0).unwrap();
    assert!(matches!(
        parsed,
        crate::update_install::cli::Mode::Install(_)
    ));
    assert_eq!(
        install::bundle_of(Path::new("/Applications/Hollow.app/Contents/MacOS/hollow")),
        Some(PathBuf::from("/Applications/Hollow.app"))
    );
    assert_eq!(
        install::bundle_of(Path::new("/repo/target/debug/hollow")),
        None
    );
}

#[test]
fn the_connect_link_reads_only_its_token_and_never_repeats_it() {
    use super::link::{is_updates_link, parse_connect};
    let url = format!("hollow://updates/connect?token={TOKEN}");
    assert!(is_updates_link(&url));
    assert!(is_updates_link("HOLLOW://Updates/connect"));
    assert!(!is_updates_link("hollow://issue?prompt=x"));
    assert!(!is_updates_link("https://updates/connect"));
    assert_eq!(parse_connect(&url).unwrap(), TOKEN);
    assert_eq!(parse_connect(&format!("{url}/")).ok(), None);
    for bad in [
        format!("hollow://updates/connect?token={}", &TOKEN[..40]),
        format!("hollow://updates/connect?token={TOKEN}&x=1"),
        format!("hollow://updates/connect?token={TOKEN}&token={TOKEN}"),
        format!("hollow://updates/connect?token={TOKEN}#frag"),
        format!("hollow://updates/other?token={TOKEN}"),
        format!("hollow://updates:80/connect?token={TOKEN}"),
        format!("hollow://u@updates/connect?token={TOKEN}"),
        "hollow://updates/connect".to_string(),
        format!("hollow://updates/connect?token={}", "A".repeat(600)),
    ] {
        let err = parse_connect(&bad).unwrap_err();
        assert!(!err.contains(&TOKEN[..20]), "{bad}");
    }
}

// ── Review round 1 ──────────────────────────────────────────────────────

const OTHER: &str = "Zz9yXwVuTsRqPoNmLkJiHgFeDcBa0123456789_-QQQ";

#[test]
fn a_link_never_replaces_a_stored_token_until_the_person_confirms() {
    assert_eq!(OTHER.len(), 43);
    let r = rig(Setup::default());
    assert_eq!(
        r.client.connect_from_link(OTHER.into()).unwrap(),
        LinkConnect::Pending
    );
    assert_eq!(r.tokens.get().unwrap().as_deref(), Some(TOKEN), "untouched");
    let state = r.client.state();
    assert!(state.pending_connect);
    assert!(!serde_json::to_string(&state).unwrap().contains(OTHER));
    // Checks still use the stored token.
    r.client.check();
    assert_eq!(r.channel.checks.lock().unwrap()[0].1, TOKEN);
    // Keep the current one.
    assert!(!r.client.discard_connect().pending_connect);
    assert_eq!(r.tokens.get().unwrap().as_deref(), Some(TOKEN));
    assert!(r.client.confirm_connect().is_err(), "nothing waits");
    // Use the new one.
    r.client.connect_from_link(OTHER.into()).unwrap();
    let state = r.client.confirm_connect().unwrap();
    assert!(!state.pending_connect && state.connected);
    assert_eq!(r.tokens.get().unwrap().as_deref(), Some(OTHER));
    // The same token again changes nothing.
    assert_eq!(
        r.client.connect_from_link(OTHER.into()).unwrap(),
        LinkConnect::Same
    );
    for event in r.events.0.lock().unwrap().iter() {
        assert!(!event.contains(OTHER) && !event.contains(TOKEN));
    }
    // A first connect, with nothing stored, stores directly.
    let fresh = rig(Setup {
        token: None,
        ..Setup::default()
    });
    assert_eq!(
        fresh.client.connect_from_link(OTHER.into()).unwrap(),
        LinkConnect::Stored
    );
    assert_eq!(fresh.tokens.get().unwrap().as_deref(), Some(OTHER));
    assert!(!fresh.client.state().pending_connect);
}

#[test]
fn the_same_link_twice_in_a_moment_is_one_open() {
    use super::link::is_repeat;
    let a = format!("hollow://updates/connect?token={OTHER}&r={}", line!());
    assert!(!is_repeat(&a));
    assert!(is_repeat(&a));
    assert!(!is_repeat(&format!("{a}x")));
}

#[test]
fn the_client_reads_no_keychain_item_until_the_schedule_thread_does() {
    #[derive(Default)]
    struct Counting(AtomicUsize);
    impl TokenStore for Counting {
        fn get(&self) -> Result<Option<String>, String> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
        fn set(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            Ok(())
        }
    }
    let r = rig(Setup::default());
    let tokens = Arc::new(Counting::default());
    struct Shared(Arc<Counting>);
    impl TokenStore for Shared {
        fn get(&self) -> Result<Option<String>, String> {
            self.0.get()
        }
        fn set(&self, t: &str) -> Result<(), String> {
            self.0.set(t)
        }
        fn delete(&self) -> Result<(), String> {
            self.0.delete()
        }
    }
    let client = Client::new(Deps {
        config: Config {
            host: config::host_from(None, false),
            pubkey: "k".into(),
            team: config::team_from(Some("-".into())),
            running_version: "0.6.10".into(),
            review_copy: false,
        },
        paths: paths(&r.root, true),
        fetcher: Box::new(r.channel.clone()),
        tokens: Box::new(Shared(tokens.clone())),
        signer: Box::new(FakeSign::default()),
        work: Box::new(FakeWork::default()),
        helper: Box::new(FakeHelper::default()),
        clock: Box::new(FakeClock::default()),
        sink: Box::new(Events::default()),
        limits: Limits::default(),
        own_pid: 3,
        naps: client::Naps::Clock,
    });
    let _ = client.state();
    assert_eq!(tokens.0.load(Ordering::SeqCst), 0);
    client.load_connection();
    assert_eq!(tokens.0.load(Ordering::SeqCst), 1);
    assert!(matches!(client.state().phase, UpdatePhase::NotSetUp { .. }));
}

#[test]
fn verification_comes_before_the_work_check() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.run(&["queue:1"]);
    r.sign_fails.store(true, Ordering::SeqCst);
    let err = r.client.install_now(None).unwrap_err();
    assert!(err.contains("signature does not verify"), "{err}");
}

#[test]
fn work_that_starts_before_the_quit_stops_the_installer_and_keeps_the_build_ready() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    // Look 1 (the work check) is clear; a chat starts before look 2 (the
    // last look before the quit).
    let looks = r.work.looks.load(Ordering::SeqCst);
    *r.work.starts_on.lock().unwrap() = Some((
        looks + 2,
        UpdateWork {
            key: "queue:9".into(),
            name: "A new chat".into(),
        },
    ));
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::Started { .. }
    ));
    assert_eq!(r.helper.quits.load(Ordering::SeqCst), 0);
    assert_eq!(r.helper.aborted.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.helper.stops.load(Ordering::SeqCst),
        1,
        "the installer is stopped"
    );
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    assert!(r
        .client
        .state()
        .note
        .unwrap()
        .contains("A new chat started meanwhile"));
    assert!(!r.root.join("data/updates").join(last::FILE).exists());
}

#[test]
fn from_the_wait_a_late_chat_puts_the_wait_back() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    // The wait's idle look, then install's work check, are clear; the chat
    // starts on the last look.
    let looks = r.work.looks.load(Ordering::SeqCst);
    *r.work.starts_on.lock().unwrap() = Some((
        looks + 3,
        UpdateWork {
            key: "queue:9".into(),
            name: "A new chat".into(),
        },
    ));
    let ticket = AtomicU8::new(install::WAITING);
    r.client.wait_loop(&ticket, install::POLL);
    assert_eq!(r.helper.aborted.load(Ordering::SeqCst), 1);
    let state = r.client.state();
    assert!(state.waiting, "waiting again");
    assert!(matches!(state.phase, UpdatePhase::Ready { .. }));
    assert!(!r.client.cancel_wait().waiting);
}

#[test]
fn an_installer_that_dies_at_once_leaves_hollow_running_and_ready() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.helper.dies.store(true, Ordering::SeqCst);
    let err = r.client.install_now(None).unwrap_err();
    assert!(err.contains("stopped as soon as it started"));
    assert_eq!(
        r.helper.quits.load(Ordering::SeqCst) + r.helper.aborted.load(Ordering::SeqCst),
        0
    );
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    assert!(!r.root.join("data/updates").join(last::FILE).exists());
}

#[test]
fn a_channel_switch_during_a_download_drops_it_and_checks_the_new_channel() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.publish("0.6.12", "nightly");
    let client = r.client.clone();
    *r.channel.during_download.lock().unwrap() = Some(Box::new(move || {
        client.set_channel(Channel::Nightly).unwrap();
    }));
    r.client.check();
    match r.phase() {
        UpdatePhase::Ready { release } => {
            assert_eq!(release.version, "0.6.12");
            assert_eq!(release.channel, Channel::Nightly);
        }
        other => panic!("{other:?}"),
    }
    assert!(!r.root.join("data/updates/staging/0.6.11").exists());
    let checks = r.channel.checks.lock().unwrap().clone();
    assert!(checks[1].0.ends_with("/channels/nightly.json"));
}

#[test]
fn a_disconnect_during_a_check_discards_its_answer() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    let client = r.client.clone();
    *r.channel.during_check.lock().unwrap() = Some(Box::new(move || {
        client.disconnect().unwrap();
    }));
    r.client.check();
    assert_eq!(
        r.phase(),
        UpdatePhase::NotSetUp {
            reason: config::NOT_CONNECTED.into()
        }
    );
    assert_eq!(r.downloads(), 0);
    assert!(!r.root.join("data/updates/staging/0.6.11").exists());
}

#[test]
fn a_restart_click_during_a_recheck_keeps_the_staged_build() {
    for second in ["newer", "nothing"] {
        let r = rig(Setup::default());
        r.publish("0.6.11", "main");
        r.client.check();
        if second == "newer" {
            r.publish("0.6.12", "main");
        } else {
            r.answer(Err(CheckError::NothingPublished));
        }
        let client = r.client.clone();
        *r.channel.during_check.lock().unwrap() = Some(Box::new(move || {
            assert!(matches!(
                client.install_now(None).unwrap(),
                UpdateInstallOutcome::Started { .. }
            ));
        }));
        r.client.check();
        assert!(
            matches!(r.phase(), UpdatePhase::Installing { .. }),
            "{second}"
        );
        assert!(
            r.root
                .join("data/updates/staging/0.6.11/Hollow.app")
                .is_dir(),
            "{second}: the build being installed stays"
        );
        assert!(!r.root.join("data/updates/staging/0.6.12").exists());
    }
}

#[test]
fn the_downgrade_permission_ends_on_a_higher_offer_and_after_24_hours() {
    let r = rig(Setup::default());
    r.client.set_channel(Channel::Nightly).unwrap();
    r.publish("0.6.11", "nightly");
    r.client.check();
    assert!(!r.client.settings().allow_downgrade_once);
    let r = rig(Setup {
        running: "0.6.20",
        ..Setup::default()
    });
    r.client.set_channel(Channel::Nightly).unwrap();
    assert!(r.client.state().downgrade_allowed);
    r.clock.sleep(Duration::from_secs(25 * 60 * 60));
    assert!(!r.client.state().downgrade_allowed);
    r.publish("0.6.15", "nightly");
    r.client.check();
    assert_eq!(r.phase(), UpdatePhase::UpToDate { skipped: None });
    assert!(!r.client.settings().allow_downgrade_once);
}

#[test]
fn a_refused_token_backs_off_to_four_hours_and_success_resets() {
    let r = rig(Setup::default());
    for _ in 0..4 {
        r.answer(Err(CheckError::Unauthorized));
    }
    r.client
        .run_schedule(client::LAUNCH_DELAY, client::EVERY, Some(6));
    let hours: Vec<u64> = r
        .clock
        .sleeps
        .lock()
        .unwrap()
        .iter()
        .map(|d| d.as_secs())
        .collect();
    assert_eq!(hours, vec![45, 3600, 7200, 14400, 14400, 1800]);
}

#[test]
fn a_withdrawn_build_ends_the_wait_and_a_later_ready_clears_the_note() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.run(&["queue:1"]);
    assert!(
        r.client
            .install_when_idle(Duration::from_millis(5))
            .unwrap()
            .waiting
    );
    r.answer(Err(CheckError::NothingPublished));
    r.client.check();
    assert!(!r.client.state().waiting);
    r.client.set_note("old words".into());
    r.publish("0.6.12", "main");
    r.client.check();
    assert_eq!(r.client.state().note, None);
}

// ── Review round 2 ──────────────────────────────────────────────────────

#[test]
fn a_channel_switch_or_disconnect_is_refused_while_installing() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.client.install_now(None).unwrap();
    assert_eq!(
        r.client.set_channel(Channel::Nightly).unwrap_err(),
        client::INSTALLING
    );
    assert_eq!(r.client.settings().channel, Channel::Main);
    assert_eq!(r.client.disconnect().unwrap_err(), client::INSTALLING);
    assert_eq!(r.tokens.get().unwrap().as_deref(), Some(TOKEN));
    assert!(matches!(r.phase(), UpdatePhase::Installing { .. }));
    assert!(r
        .root
        .join("data/updates/staging/0.6.11/Hollow.app")
        .is_dir());
}

#[test]
fn a_stale_download_is_stopped_not_finished() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.publish("0.6.12", "nightly");
    let client = r.client.clone();
    *r.channel.during_download.lock().unwrap() = Some(Box::new(move || {
        client.set_channel(Channel::Nightly).unwrap();
    }));
    r.client.check();
    assert_eq!(
        r.channel.stopped.load(Ordering::SeqCst),
        1,
        "the main download stopped"
    );
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.12"));
}

#[test]
fn a_late_clear_never_erases_a_newer_switchs_permission() {
    let r = rig(Setup::default());
    r.client.set_channel(Channel::Nightly).unwrap();
    let first = r.client.settings().allow_downgrade_since_ms;
    // A newer switch, then a check that started under the first one clears.
    r.clock.sleep(Duration::from_secs(1));
    r.client.set_channel(Channel::Main).unwrap();
    r.client.clear_downgrade(first);
    assert!(
        r.client.settings().allow_downgrade_once,
        "the newer permission stays"
    );
    let second = r.client.settings().allow_downgrade_since_ms;
    r.client.clear_downgrade(second);
    assert!(!r.client.settings().allow_downgrade_once);
}

#[test]
fn an_expired_downgrade_is_not_installed_and_a_future_time_counts_as_expired() {
    let r = rig(Setup {
        running: "0.6.20",
        ..Setup::default()
    });
    r.client.set_channel(Channel::Nightly).unwrap();
    r.publish("0.6.15", "nightly");
    r.client.check();
    assert!(matches!(r.phase(), UpdatePhase::Ready { .. }));
    r.clock.sleep(Duration::from_secs(25 * 60 * 60));
    r.publish("0.6.15", "nightly");
    assert_eq!(
        r.client.install_now(None).unwrap_err(),
        client::DOWNGRADE_EXPIRED
    );
    assert!(r.helper.spawned.lock().unwrap().is_empty());
    assert_eq!(r.phase(), UpdatePhase::UpToDate { skipped: None });
    assert_eq!(
        r.client.state().note.as_deref(),
        Some(client::DOWNGRADE_EXPIRED)
    );
    assert!(!r.client.settings().allow_downgrade_once);
    // A switch time in the future (a clock set back) is expired.
    let s = super::settings::UpdateSettings {
        allow_downgrade_once: true,
        allow_downgrade_since_ms: Some(10_000),
        ..Default::default()
    };
    assert!(!s.downgrade_allowed(9_999));
}

#[test]
fn an_installer_that_dies_before_the_quit_keeps_hollow_running() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.helper.dies_later.store(true, Ordering::SeqCst);
    r.client.install_now(None).unwrap();
    assert_eq!(r.helper.quits.load(Ordering::SeqCst), 0);
    assert_eq!(r.helper.aborted.load(Ordering::SeqCst), 1);
    assert_eq!(ready_version(&r.phase()).as_deref(), Some("0.6.11"));
    assert!(r
        .client
        .state()
        .note
        .unwrap()
        .contains("stopped before Hollow quit"));
    assert!(!r.root.join("data/updates").join(last::FILE).exists());
}

#[test]
fn a_kick_ends_the_schedules_wait_at_once() {
    let r = rig(Setup {
        naps: client::Naps::Interruptible,
        ..Setup::default()
    });
    let client = r.client.clone();
    // An hour's launch delay; the kick (Check now, a reconnect) ends it.
    let schedule = std::thread::spawn(move || {
        client.run_schedule(Duration::from_secs(3600), MAX_WAIT, Some(1));
    });
    r.client.wake_schedule();
    schedule.join().unwrap();
    assert_eq!(r.channel.checks.lock().unwrap().len(), 1);
}

const MAX_WAIT: Duration = Duration::from_secs(4 * 60 * 60);
