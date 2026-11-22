//! Go back (row U6) on fakes and temp folders: the plan and its schema
//! verdict, the flow's checks and claim, the no-expiry wait, the skip and
//! its Undo, and the notice after the restart. No codesign, `open`,
//! network, Keychain or `/Applications`; the helper is never run.

use super::client::{self, Client, Deps, Helper, Paths, Sink, Started, Work};
use super::config::{self, Config};
use super::fetch::{CheckError, Fetcher, Pending};
use super::goback::{self, UpdateSchemaVerdict};
use super::goback_run::{self, UpdateGoBackOutcome};
use super::install::{self, UpdateWork, WaitAction, WaitEnd};
use super::last::{self, UpdateLastResult};
use super::state::{decide, Decision, UpdatePhase, UpdateState};
use super::token::TokenStore;
use super::unpack::Limits;
use crate::update_install::effects::{Clock, Signer};
use crate::update_install::journal::{Journal, Kind, Record};
use crate::update_install::{snapshot, Layout};
use reqwest::Url;
use rusqlite::Connection;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── Fakes ───────────────────────────────────────────────────────────────

/// The channel offers nothing.
struct Nothing;
impl Fetcher for Nothing {
    fn check(&self, _: &Url, _: &str) -> Result<Option<Box<dyn Pending>>, CheckError> {
        Ok(None)
    }
}

struct Token;
impl TokenStore for Token {
    fn get(&self) -> Result<Option<String>, String> {
        Ok(Some(super::client_tests::TOKEN.into()))
    }
    fn set(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

struct Sign;
impl Signer for Sign {
    fn verify(&self, _: &Path) -> Result<(), String> {
        Ok(())
    }
    fn team(&self, _: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn verify_requirement(&self, _: &Path, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Default)]
struct FakeWork {
    running: Arc<Mutex<Vec<UpdateWork>>>,
    /// The work ends after this many looks.
    ends_after: Arc<Mutex<Option<usize>>>,
    /// On this look, a chat starts.
    starts_on: Arc<Mutex<Option<usize>>>,
    looks: Arc<AtomicUsize>,
}

impl Work for FakeWork {
    fn running(&self) -> Result<Vec<UpdateWork>, String> {
        let n = self.looks.fetch_add(1, Ordering::SeqCst) + 1;
        if *self.starts_on.lock().unwrap() == Some(n) {
            self.running.lock().unwrap().push(UpdateWork {
                key: "queue:9".into(),
                name: "A new chat".into(),
            });
        }
        if self.ends_after.lock().unwrap().is_some_and(|a| n >= a) {
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
        None
    }
}

#[derive(Clone, Default)]
struct FakeHelper {
    spawned: Arc<Mutex<Vec<(PathBuf, Vec<OsString>)>>>,
    quits: Arc<AtomicUsize>,
    aborted: Arc<AtomicUsize>,
    stops: Arc<AtomicUsize>,
}

struct Running(Arc<AtomicUsize>);
impl Started for Running {
    fn exited_early(&mut self) -> Option<String> {
        None
    }
    fn exited(&mut self) -> Option<String> {
        None
    }
    fn stop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Helper for FakeHelper {
    fn spawn(
        &self,
        exe: &Path,
        args: &[OsString],
        _log: &Path,
    ) -> Result<Box<dyn Started>, String> {
        self.spawned
            .lock()
            .unwrap()
            .push((exe.to_path_buf(), args.to_vec()));
        Ok(Box::new(Running(self.stops.clone())))
    }
    fn quit(&self, last_look: Box<dyn FnOnce() -> bool + Send>) {
        if last_look() {
            self.quits.fetch_add(1, Ordering::SeqCst);
        } else {
            self.aborted.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[derive(Clone, Default)]
struct FakeClock(Arc<AtomicU64>);
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        1_763_200_000_000 + self.0.load(Ordering::SeqCst)
    }
    fn sleep(&self, d: Duration) {
        self.0.fetch_add(d.as_millis() as u64, Ordering::SeqCst);
    }
}

struct Quiet;
impl Sink for Quiet {
    fn state(&self, _: &UpdateState) {}
}

// ── The Mac ─────────────────────────────────────────────────────────────

const RUNNING: &str = "0.6.20";
const PREVIOUS: &str = "0.6.19";
/// The snapshot's time; chats after it are "since the update".
const SNAPSHOT_AT: &str = "2026-11-15T09:30:00.000Z";

fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.hollow.app</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleExecutable</key><string>hollow</string>
</dict></plist>"#
    )
}

fn make_bundle(at: &Path, version: &str) {
    std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
    std::fs::write(at.join("Contents/Info.plist"), info_plist(version)).unwrap();
    std::fs::write(
        at.join("Contents/MacOS/hollow"),
        format!("binary {version}"),
    )
    .unwrap();
}

struct Mac {
    _dir: tempfile::TempDir,
    root: PathBuf,
    state: PathBuf,
    sessions: PathBuf,
    live: PathBuf,
    work: FakeWork,
    helper: FakeHelper,
    client: Arc<Client>,
}

fn add_chat(db: &Path, key: &str, at: &str) {
    let conn = Connection::open(db).unwrap();
    conn.execute(
        "INSERT INTO sessions(key, agent, cwd, created_at, last_activity_at) VALUES(?1, 'claude', '/', ?2, ?2)",
        [key, at],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO turns(session_key, ord, started_at) VALUES(?1, 0, ?2)",
        [key, at],
    )
    .unwrap();
}

impl Mac {
    /// 0.6.20 installed over 0.6.19 (kept as previous) by a healthy journaled
    /// install that snapshotted the Sessions database, which has one chat
    /// from before. `previous: false`: a fresh Mac with no previous copy.
    fn new(previous: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let app = root.join("Applications").join("Hollow.app");
        make_bundle(&app, RUNNING);
        if previous {
            make_bundle(&Layout::for_app(&app).previous, PREVIOUS);
        }
        let state = root.join("data").join("installer");
        let sessions = root.join("support").join("Sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let live = sessions.join("vault-aaaa.db");
        {
            let conn = Connection::open(&live).unwrap();
            crate::session_store::apply_schema(&conn).unwrap();
        }
        add_chat(&live, "s-before", "2026-11-14T10:00:00+00:00");
        if previous {
            let folder = state
                .join(snapshot::DIR)
                .join(format!("20261115-093000-aaaa-from-{PREVIOUS}"));
            let snap = snapshot::take(std::slice::from_ref(&live), &folder).unwrap();
            let mut record =
                Record::new(Kind::Install, "20261115-093000-aaaa", 1, &app, SNAPSHOT_AT);
            record.from_version = Some(PREVIOUS.into());
            record.to_version = Some(RUNNING.into());
            record.snapshot = Some(snap);
            let mut j = Journal::create(&state, record).unwrap();
            for step in [
                "prepared",
                "staged-verified",
                "snapshot-written",
                "promoted",
                "healthy",
            ] {
                j.step(step, SNAPSHOT_AT, serde_json::json!({})).unwrap();
            }
            goback::record_known_schema(&state, PREVIOUS);
        }
        let (work, helper) = (FakeWork::default(), FakeHelper::default());
        let client = Arc::new(Client::new(Deps {
            config: Config {
                host: config::host_from(None, false),
                pubkey: "RWQ-test-key".into(),
                team: config::team_from(Some("-".into())),
                running_version: RUNNING.into(),
                review_copy: false,
            },
            paths: Paths {
                settings: root.join("data").join("update-settings.json"),
                staging: root.join("data").join("updates").join("staging"),
                last: root.join("data").join("updates").join(last::FILE),
                installed_app: Some(app),
                state_dir: Some(state.clone()),
                sessions: Some(sessions.clone()),
            },
            fetcher: Box::new(Nothing),
            tokens: Box::new(Token),
            signer: Box::new(Sign),
            work: Box::new(work.clone()),
            helper: Box::new(helper.clone()),
            clock: Box::new(FakeClock::default()),
            sink: Box::new(Quiet),
            limits: Limits::default(),
            own_pid: 4242,
            naps: client::Naps::Clock,
        }));
        client.load_connection();
        Mac {
            _dir: dir,
            root,
            state,
            sessions,
            live,
            work,
            helper,
            client,
        }
    }

    /// A newer build stamped the Sessions database (a format 0.6.19 doesn't
    /// know) and two chats came since the update.
    fn newer_sessions(&self) {
        add_chat(&self.live, "s-after-1", "2026-11-15T11:00:00.123456+00:00");
        add_chat(&self.live, "s-after-2", "2026-11-15T12:00:00Z");
        Connection::open(&self.live)
            .unwrap()
            .execute_batch("PRAGMA user_version = 1;")
            .unwrap();
    }

    fn args(&self) -> Vec<String> {
        let spawned = self.helper.spawned.lock().unwrap();
        assert_eq!(spawned.len(), 1, "one helper");
        spawned[0]
            .1
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn skipped(&self) -> Option<String> {
        self.client.settings().skipped_version
    }
}

// ── The plan ────────────────────────────────────────────────────────────

#[test]
fn a_fresh_mac_or_a_dev_run_cannot_go_back_and_says_why() {
    let fresh = Mac::new(false);
    let plan = fresh.client.go_back_plan();
    assert!(!plan.available);
    assert_eq!(plan.disabled_reason.as_deref(), Some(goback::NO_PREVIOUS));
    assert!(plan.previous.is_none());
    assert!(matches!(
        fresh.client.go_back(false, None),
        UpdateGoBackOutcome::Refused { reason } if reason == goback::NO_PREVIOUS
    ));
    // Not in an Applications folder.
    let plan = goback::plan(&goback::PlanInputs {
        running_version: RUNNING,
        installed_app: Some(Path::new("/tmp/build/Hollow.app")),
        state_dir: Some(&fresh.state),
        support: None,
        review_copy: false,
        busy: false,
        lock_live: false,
        skipped_version: None,
        waiting: false,
    });
    assert_eq!(
        plan.disabled_reason.as_deref(),
        Some(goback::NOT_FROM_APPLICATIONS)
    );
    // An install starting, or another installer holding the lock.
    let mac = Mac::new(true);
    let app = mac.root.join("Applications/Hollow.app");
    let mut inputs = goback::PlanInputs {
        running_version: RUNNING,
        installed_app: Some(&app),
        state_dir: Some(&mac.state),
        support: None,
        review_copy: false,
        busy: true,
        lock_live: false,
        skipped_version: None,
        waiting: false,
    };
    assert_eq!(
        goback::plan(&inputs).disabled_reason.as_deref(),
        Some(goback::BUSY)
    );
    inputs.busy = false;
    inputs.lock_live = true;
    let locked = goback::plan(&inputs);
    assert_eq!(locked.disabled_reason.as_deref(), Some(goback::LOCKED));
    assert_eq!(locked.previous.unwrap().version, PREVIOUS, "still named");
}

#[test]
fn a_previous_build_that_knows_every_store_is_a_plain_swap() {
    let mac = Mac::new(true);
    let plan = mac.client.go_back_plan();
    assert!(plan.available, "{:?}", plan.disabled_reason);
    let previous = plan.previous.clone().unwrap();
    assert_eq!(previous.version, PREVIOUS);
    assert_eq!(plan.running_installed_at.as_deref(), Some(SNAPSHOT_AT));
    assert_eq!(plan.schema, Some(UpdateSchemaVerdict::Compatible));
    let snap = plan.snapshot.unwrap();
    assert!(snap.intact && snap.files == 1, "{snap:?}");
    assert_eq!(snap.at, SNAPSHOT_AT);
    assert!(plan.restore_available);
    assert_eq!(plan.will_skip, RUNNING);
    assert!(plan.indexes_off.is_empty());
}

#[test]
fn a_newer_sessions_format_needs_the_restore_and_names_what_is_set_aside() {
    let mac = Mac::new(true);
    mac.newer_sessions();
    let plan = mac.client.go_back_plan();
    assert!(plan.available, "{:?}", plan.disabled_reason);
    match plan.schema.clone().unwrap() {
        UpdateSchemaVerdict::NeedsRestore {
            newer,
            lost,
            summary,
        } => {
            assert_eq!(newer.len(), 1);
            assert_eq!(newer[0].store, "sessions");
            assert_eq!(newer[0].file, "vault-aaaa.db");
            assert_eq!((newer[0].current, newer[0].known), (1, Some(0)));
            let lost = lost.expect("counted");
            assert_eq!((lost.chats, lost.messages), (2, 2));
            assert_eq!(lost.since, SNAPSHOT_AT);
            assert!(
                lost.summary
                    .starts_with("2 chats started and 2 messages since"),
                "{}",
                lost.summary
            );
            assert!(summary.contains(PREVIOUS), "{summary}");
        }
        other => panic!("{other:?}"),
    }
    // A plain swap is refused; the restore is the way.
    assert!(matches!(
        mac.client.go_back(false, None),
        UpdateGoBackOutcome::Refused { reason } if reason.contains("restore data")
    ));
    assert!(mac.helper.spawned.lock().unwrap().is_empty());
    // A damaged snapshot leaves no way back, and says so.
    let (journals, _) = crate::update_install::journal::list(&mac.state);
    let copy = journals[0].record.snapshot.clone().unwrap().files[0]
        .copy
        .clone();
    std::fs::write(&copy, b"damaged").unwrap();
    let plan = mac.client.go_back_plan();
    assert!(!plan.available && !plan.restore_available);
    assert!(
        plan.disabled_reason.unwrap().contains("no intact copy"),
        "plain words"
    );
}

#[test]
fn an_unknown_previous_schema_counts_a_stamped_blind_store_as_newer() {
    // A previous build that predates known-schemas.json: the snapshot's
    // recorded Sessions version (0) stands in for it.
    let mac = Mac::new(true);
    std::fs::remove_file(mac.state.join(goback::KNOWN_FILE)).unwrap();
    assert_eq!(
        mac.client.go_back_plan().schema,
        Some(UpdateSchemaVerdict::Compatible)
    );
    mac.newer_sessions();
    assert!(matches!(
        mac.client.go_back_plan().schema,
        Some(UpdateSchemaVerdict::NeedsRestore { .. })
    ));
}

#[test]
fn a_newer_index_only_says_it_stays_off() {
    let mac = Mac::new(true);
    let index = mac.sessions.parent().unwrap().join("SearchIndex");
    std::fs::create_dir_all(&index).unwrap();
    let newer = crate::vault_search_index::SCHEMA_VERSION + 1;
    Connection::open(index.join("vault-aaaa.db"))
        .unwrap()
        .execute_batch(&format!(
            "CREATE TABLE meta(x); PRAGMA user_version = {newer};"
        ))
        .unwrap();
    let plan = mac.client.go_back_plan();
    assert!(plan.available);
    assert_eq!(plan.schema, Some(UpdateSchemaVerdict::Compatible));
    assert_eq!(plan.indexes_off.len(), 1);
    assert!(
        plan.indexes_off[0].contains("keyword search index"),
        "{:?}",
        plan.indexes_off
    );
}

#[test]
fn known_schemas_are_recorded_per_version_and_kept_short() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..40 {
        goback::record_known_schema(dir.path(), &format!("0.6.{n}"));
    }
    goback::record_known_schema(dir.path(), "not a version");
    let text = std::fs::read_to_string(dir.path().join(goback::KNOWN_FILE)).unwrap();
    let known: serde_json::Value = serde_json::from_str(&text).unwrap();
    let map = known.as_object().unwrap();
    assert_eq!(map.len(), 30);
    assert!(map.contains_key("0.6.39") && !map.contains_key("0.6.9"));
    assert_eq!(
        known["0.6.39"]["searchIndex"],
        crate::vault_search_index::SCHEMA_VERSION
    );
    assert_eq!(
        known["0.6.39"]["sessions"],
        crate::session_store::SCHEMA_VERSION
    );
}

/// Review M4: the restore puts back only the Sessions databases, so a
/// newer store outside the snapshot (the companion ledger) refuses Go back
/// instead of offering a restore that wouldn't cover it.
#[test]
fn a_newer_store_the_snapshot_does_not_hold_refuses_go_back() {
    let mac = Mac::new(true);
    let companion = mac.sessions.parent().unwrap().join("Companion");
    std::fs::create_dir_all(&companion).unwrap();
    let newer = crate::companion_inbox::LEDGER_SCHEMA_VERSION + 1;
    Connection::open(companion.join("applied.db"))
        .unwrap()
        .execute_batch(&format!(
            "CREATE TABLE applied(x); PRAGMA user_version = {newer};"
        ))
        .unwrap();
    let plan = mac.client.go_back_plan();
    assert!(!plan.available && !plan.restore_available);
    match plan.schema.clone().unwrap() {
        UpdateSchemaVerdict::NeedsRestore { newer, .. } => {
            assert_eq!(newer.len(), 1);
            assert_eq!(newer[0].store, "companion");
        }
        other => panic!("{other:?}"),
    }
    let why = plan.disabled_reason.unwrap();
    assert!(
        why.contains("applied.db") && why.contains("doesn't include it"),
        "{why}"
    );
    assert!(matches!(
        mac.client.go_back(true, None),
        UpdateGoBackOutcome::Refused { .. }
    ));
    assert!(mac.helper.spawned.lock().unwrap().is_empty());
    // With the Sessions database newer too, it is still refused.
    mac.newer_sessions();
    assert!(!mac.client.go_back_plan().available);
}

/// Review M5: the plan's `quick_check` of a snapshot copy is kept while the
/// copy is unchanged; any change to the file checks it again.
#[test]
fn the_plans_snapshot_check_is_cached_while_the_copy_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("copy.db");
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE t(x TEXT); INSERT INTO t VALUES('a'); PRAGMA user_version = 2;",
        )
        .unwrap();
    assert_eq!(goback::cached_check(&db, 2), Ok(()));
    // Damage the pages but keep the size and the modification time: the
    // cached answer stands, so no full check ran.
    let mtime = std::fs::metadata(&db).unwrap().modified().unwrap();
    let mut bytes = std::fs::read(&db).unwrap();
    let len = bytes.len();
    for b in bytes.iter_mut().take(len).skip(100) {
        *b = 0xA5;
    }
    std::fs::write(&db, &bytes).unwrap();
    let f = std::fs::File::options().write(true).open(&db).unwrap();
    f.set_modified(mtime).unwrap();
    drop(f);
    assert_eq!(
        goback::cached_check(&db, 2),
        Ok(()),
        "served from the cache"
    );
    // A changed time is a changed file: checked again, and it fails.
    let f = std::fs::File::options().write(true).open(&db).unwrap();
    f.set_modified(mtime + Duration::from_secs(5)).unwrap();
    drop(f);
    assert!(goback::cached_check(&db, 2).is_err());
    // A different expected version is its own entry.
    assert!(goback::cached_check(&db, 3).is_err());
}

/// Review LOW: the launch's skip doesn't depend on the window not having
/// acknowledged the notice yet, and it is applied once (an Undo after it
/// stays undone at the next launch).
#[test]
fn the_launch_skip_applies_once_even_after_the_notice_is_acknowledged() {
    let older = Mac::new(true);
    let path = older.root.join("data/updates").join(last::FILE);
    let record = started_record(last::StartedKind::Install, RUNNING, "0.6.21");
    last::write(&path, &record).unwrap();
    std::fs::create_dir_all(&older.state).unwrap();
    std::fs::write(
        older.state.join(crate::update_install::run::FAILURE_FILE),
        serde_json::json!({
            "installId": record.install_id,
            "wentBack": true,
            "attemptedVersion": "0.6.21",
            "restoredVersion": RUNNING,
            "reason": "it quit twice",
        })
        .to_string(),
    )
    .unwrap();
    // The window acknowledged before the launch thread got here.
    older.client.ack_last_result().unwrap();
    assert!(older.client.last_result().is_none());
    older.client.settle_go_back_skip();
    assert_eq!(older.skipped().as_deref(), Some("0.6.21"));
    // Undo, then the next launch: the skip isn't applied again.
    older.client.unskip().unwrap();
    older.client.settle_go_back_skip();
    assert_eq!(older.skipped(), None);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["acknowledged"], true, "neither flag lost the other");
    assert_eq!(saved["skipSettled"], true);
}

// ── The flow ────────────────────────────────────────────────────────────

#[test]
fn go_back_clones_this_app_for_the_helper_skips_this_version_and_quits() {
    let mac = Mac::new(true);
    match mac.client.go_back(false, None) {
        UpdateGoBackOutcome::Started { version } => assert_eq!(version, PREVIOUS),
        other => panic!("{other:?}"),
    }
    let exe = mac.helper.spawned.lock().unwrap()[0].0.clone();
    assert!(
        exe.starts_with(mac.state.join(goback_run::HELPER_DIR)),
        "the helper runs from a clone, not a bundle it swaps: {exe:?}"
    );
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        format!("binary {RUNNING}").into_bytes()
    );
    let args = mac.args();
    assert_eq!(args[0], "--go-back");
    let app = mac.root.join("Applications/Hollow.app");
    assert!(args
        .windows(2)
        .any(|w| w[0] == "--app" && Path::new(&w[1]) == app));
    assert!(args
        .windows(2)
        .any(|w| w[0] == "--journal-dir" && Path::new(&w[1]) == mac.state));
    assert!(args.contains(&"--relaunch".to_string()));
    assert!(!args.contains(&"--restore-snapshot".to_string()));
    assert_eq!(mac.helper.quits.load(Ordering::SeqCst), 1);
    assert_eq!(mac.skipped().as_deref(), Some(RUNNING));
    assert!(matches!(
        mac.client.state().phase,
        UpdatePhase::GoingBack { ref version } if version == PREVIOUS
    ));
    // Nothing else starts now.
    assert!(mac.client.check().phase.busy());
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(mac.root.join("data/updates").join(last::FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(record["kind"], "goBack");
    assert_eq!(record["fromVersion"], RUNNING);
    assert_eq!(record["toVersion"], PREVIOUS);
    assert_eq!(record["restoreData"], false);
    // No Undo of the skip while the go back is starting.
    assert_eq!(
        mac.client.unskip().unwrap_err(),
        goback_run::UNSKIP_GOING_BACK
    );
    assert_eq!(mac.skipped().as_deref(), Some(RUNNING));
}

#[test]
fn the_restore_passes_the_snapshot_folder_to_the_helper() {
    let mac = Mac::new(true);
    mac.newer_sessions();
    assert!(matches!(
        mac.client.go_back(true, None),
        UpdateGoBackOutcome::Started { .. }
    ));
    let args = mac.args();
    let at = args
        .iter()
        .position(|a| a == "--restore-snapshot")
        .expect("passed");
    let folder = PathBuf::from(&args[at + 1]);
    assert!(
        folder.starts_with(mac.state.join(snapshot::DIR)),
        "{folder:?}"
    );
    assert!(folder
        .to_string_lossy()
        .ends_with(&format!("-from-{PREVIOUS}")));
}

#[test]
fn running_work_is_an_answer_and_nothing_changes() {
    let mac = Mac::new(true);
    *mac.work.running.lock().unwrap() = vec![UpdateWork {
        key: "queue:1".into(),
        name: "chat 1".into(),
    }];
    assert!(matches!(
        mac.client.go_back(false, None),
        UpdateGoBackOutcome::WorkRunning { work } if work.len() == 1
    ));
    // The restart dialog's keys confirm it; a different list doesn't.
    let other = ["queue:2".to_string()];
    assert!(matches!(
        mac.client.go_back(false, Some(&other)),
        UpdateGoBackOutcome::WorkChanged { .. }
    ));
    assert!(mac.helper.spawned.lock().unwrap().is_empty());
    assert_eq!(mac.skipped(), None);
    assert!(!mac.client.state().phase.busy());
    let helpers = mac.state.join(goback_run::HELPER_DIR);
    assert!(
        std::fs::read_dir(&helpers).map_or(true, |mut d| d.next().is_none()),
        "no helper copy is left"
    );
    let keys = ["queue:1".to_string()];
    assert!(matches!(
        mac.client.go_back(false, Some(&keys)),
        UpdateGoBackOutcome::Started { .. }
    ));
}

#[test]
fn work_that_starts_before_the_quit_stops_the_helper_and_undoes_the_skip() {
    let mac = Mac::new(true);
    // The flow's work look is clear; the chat starts on the last look.
    let looks = mac.work.looks.load(Ordering::SeqCst);
    *mac.work.starts_on.lock().unwrap() = Some(looks + 2);
    assert!(matches!(
        mac.client.go_back(false, None),
        UpdateGoBackOutcome::Started { .. }
    ));
    assert_eq!(mac.helper.aborted.load(Ordering::SeqCst), 1);
    assert_eq!(mac.helper.stops.load(Ordering::SeqCst), 1);
    assert_eq!(mac.skipped(), None, "the skip is undone");
    let state = mac.client.state();
    assert!(!state.phase.busy());
    assert!(state.note.unwrap().contains("A new chat"));
    assert!(!mac.root.join("data/updates").join(last::FILE).exists());
}

#[test]
fn go_back_when_chats_finish_waits_with_no_expiry_and_a_check_does_not_end_it() {
    let mac = Mac::new(true);
    *mac.work.running.lock().unwrap() = vec![UpdateWork {
        key: "queue:1".into(),
        name: "chat 1".into(),
    }];
    *mac.work.ends_after.lock().unwrap() = Some(2_000);
    let ticket = AtomicU8::new(install::WAITING);
    assert_eq!(
        mac.client.wait_loop_for(
            &ticket,
            WaitAction::GoBack {
                restore_data: false
            },
            install::POLL,
            None
        ),
        WaitEnd::Installed
    );
    assert_eq!(mac.helper.spawned.lock().unwrap().len(), 1);
    assert_eq!(mac.skipped().as_deref(), Some(RUNNING));

    // Through the client: the state says which wait runs; a check that
    // finds nothing ends only an update's wait; Cancel ends this one.
    let mac = Mac::new(true);
    *mac.work.running.lock().unwrap() = vec![UpdateWork {
        key: "queue:1".into(),
        name: "chat 1".into(),
    }];
    let state = mac
        .client
        .go_back_when_idle(false, Duration::from_millis(1))
        .unwrap();
    assert!(state.go_back_waiting && !state.waiting);
    assert!(mac.client.go_back_plan().waiting);
    mac.client.check();
    assert!(mac.client.state().go_back_waiting);
    assert!(!mac.client.cancel_wait().go_back_waiting);
    // Not available, no wait.
    let fresh = Mac::new(false);
    assert_eq!(
        fresh
            .client
            .go_back_when_idle(false, install::POLL)
            .unwrap_err(),
        goback::NO_PREVIOUS
    );
}

// ── The skip and its Undo ───────────────────────────────────────────────

#[test]
fn the_skipped_version_is_offered_again_only_when_newer_or_after_undo() {
    // Went back from 0.6.20 to 0.6.19.
    let skipped = Some("0.6.20");
    assert_eq!(
        decide("0.6.19", "0.6.20", skipped, false),
        Decision::Skipped("0.6.20".into())
    );
    assert_eq!(
        decide("0.6.19", "0.6.21", skipped, false),
        Decision::Offer { downgrade: false }
    );
    assert_eq!(
        decide("0.6.19", "0.6.20", None, false),
        Decision::Offer { downgrade: false },
        "after Undo"
    );
    let mac = Mac::new(true);
    mac.client
        .settings_store_for_tests()
        .update(|s| s.skipped_version = Some(RUNNING.into()))
        .unwrap();
    assert_eq!(mac.client.state().skipped_version.as_deref(), Some(RUNNING));
    assert_eq!(mac.client.unskip().unwrap().skipped_version, None);
    assert_eq!(mac.client.go_back_plan().skipped_version, None);
}

fn started_record(kind: last::StartedKind, from: &str, to: &str) -> last::Started {
    last::Started {
        schema: 1,
        install_id: "20261115-140000-bbbb".into(),
        from_version: from.into(),
        to_version: to.into(),
        notes: String::new(),
        channel: super::state::Channel::Main,
        at: "2026-11-15T14:00:00.000Z".into(),
        pid: 1,
        build_id: None,
        acknowledged: false,
        kind,
        restore_data: true,
        skip_settled: false,
        overnight: false,
        overnight_notified: false,
        local: None,
    }
}

#[test]
fn after_the_restart_the_notice_says_went_back_or_why_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(last::FILE);
    last::write(
        &path,
        &started_record(last::StartedKind::GoBack, "0.6.20", "0.6.19"),
    )
    .unwrap();
    match last::read(&path, Some(dir.path()), "0.6.19", None, 9) {
        Some(UpdateLastResult::WentBack {
            attempted,
            restored,
            manual,
            data_restored,
            ..
        }) => {
            assert_eq!(attempted, "0.6.20");
            assert_eq!(restored.as_deref(), Some("0.6.19"));
            assert!(manual && data_restored);
        }
        other => panic!("{other:?}"),
    }
    // Still 0.6.20: it didn't happen; the journal says why.
    let jdir = dir.path().join(crate::update_install::journal::DIR);
    std::fs::create_dir_all(&jdir).unwrap();
    std::fs::write(
        jdir.join("20261115-140000-bbbb.json"),
        r#"{"steps":[{"step":"prepared"},{"step":"failed","reason":"the snapshot is not intact"}]}"#,
    )
    .unwrap();
    match last::read(&path, Some(dir.path()), "0.6.20", None, 9) {
        Some(UpdateLastResult::NotWentBack { target, reason, .. }) => {
            assert_eq!(target, "0.6.19");
            assert_eq!(reason.as_deref(), Some("the snapshot is not intact"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn at_launch_the_skip_follows_what_happened() {
    // A go back that didn't happen: 0.6.20 is no longer skipped.
    let mac = Mac::new(true);
    mac.client
        .settings_store_for_tests()
        .update(|s| s.skipped_version = Some(RUNNING.into()))
        .unwrap();
    last::write(
        &mac.root.join("data/updates").join(last::FILE),
        &started_record(last::StartedKind::GoBack, RUNNING, PREVIOUS),
    )
    .unwrap();
    mac.client.settle_go_back_skip();
    assert_eq!(mac.skipped(), None);

    // The automatic go back (the new version failed to start twice): this
    // older app skips the version that failed.
    let older = Mac::new(true);
    let mut record = started_record(last::StartedKind::Install, RUNNING, "0.6.21");
    record.restore_data = false;
    last::write(&older.root.join("data/updates").join(last::FILE), &record).unwrap();
    std::fs::create_dir_all(&older.state).unwrap();
    std::fs::write(
        older.state.join(crate::update_install::run::FAILURE_FILE),
        serde_json::json!({
            "installId": record.install_id,
            "wentBack": true,
            "attemptedVersion": "0.6.21",
            "restoredVersion": RUNNING,
            "reason": "it quit twice",
        })
        .to_string(),
    )
    .unwrap();
    older.client.settle_go_back_skip();
    assert_eq!(older.skipped().as_deref(), Some("0.6.21"));
}
