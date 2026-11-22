//! Row U7: the overnight installer's rule (a table), its looks on the
//! client fakes of `client_tests` (the same verify and restart checks as
//! Restart to update), the thread's wake and stop, and the morning
//! notification, posted once.

use super::client::Naps;
use super::client_tests::{rig, Rig, Setup};
use super::last::{self, UpdateLastResult};
use super::overnight::{self, decide, Facts, Hold, Look, Surroundings};
use super::state::{Channel, UpdatePhase, UpdateRelease};
use crate::phone_access::lid::Source;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ── The rule ────────────────────────────────────────────────────────────

fn release(version: &str) -> UpdateRelease {
    UpdateRelease {
        version: version.into(),
        notes: "What's new".into(),
        pub_date: None,
        channel: Channel::Main,
        commit: None,
        downgrade: false,
        local: None,
    }
}

/// Everything allows an install at 3:00 AM.
fn allowed() -> Facts {
    Facts {
        enabled: true,
        ready: Some(release("0.6.11")),
        channel: Channel::Main,
        channel_allowed: true,
        skipped: None,
        dismissed: false,
        pending_token: false,
        go_back_waiting: false,
        minute: 3 * 60,
        work: None,
        since_last_start: None,
    }
}

#[test]
fn the_window_and_conditions_decide_each_look() {
    type Change = fn(&mut Facts);
    let rows: Vec<(&str, Change, Source, Result<(), Hold>)> = vec![
        ("in window, quiet, on power", |_| {}, Source::Power, Ok(())),
        (
            "2:00 opens the window",
            |f| f.minute = 120,
            Source::Power,
            Ok(()),
        ),
        (
            "4:59 is still in",
            |f| f.minute = 299,
            Source::Power,
            Ok(()),
        ),
        (
            "1:59 is before",
            |f| f.minute = 119,
            Source::Power,
            Err(Hold::OutOfWindow),
        ),
        (
            "5:00 is after",
            |f| f.minute = 300,
            Source::Power,
            Err(Hold::OutOfWindow),
        ),
        (
            "afternoon",
            |f| f.minute = 15 * 60,
            Source::Power,
            Err(Hold::OutOfWindow),
        ),
        ("on battery", |_| {}, Source::Battery, Err(Hold::OnBattery)),
        (
            "power unreadable",
            |_| {},
            Source::Unknown,
            Err(Hold::PowerUnknown),
        ),
        (
            "a chat running",
            |f| f.work = Some("chat a".into()),
            Source::Power,
            Err(Hold::Work("chat a".into())),
        ),
        (
            "started 5 minutes ago",
            |f| f.since_last_start = Some(Duration::from_secs(5 * 60)),
            Source::Power,
            Err(Hold::StartedRecently),
        ),
        (
            "started 10 minutes ago",
            |f| f.since_last_start = Some(Duration::from_secs(10 * 60)),
            Source::Power,
            Ok(()),
        ),
        (
            "skipped version",
            |f| f.skipped = Some("0.6.11".into()),
            Source::Power,
            Err(Hold::Skipped),
        ),
        (
            "another skipped version",
            |f| f.skipped = Some("0.6.10".into()),
            Source::Power,
            Ok(()),
        ),
        (
            "Local build staged",
            |f| {
                f.ready.as_mut().unwrap().local = Some(super::local::UpdateLocalBuild {
                    workspace_id: "ws".into(),
                    branch: "feature".into(),
                    build_id: "b1".into(),
                    dirty: false,
                })
            },
            Source::Power,
            Err(Hold::LocalBuild),
        ),
        (
            "setting off",
            |f| f.enabled = false,
            Source::Power,
            Err(Hold::Off),
        ),
        (
            "nothing ready",
            |f| f.ready = None,
            Source::Power,
            Err(Hold::NothingReady),
        ),
        (
            "Not now on this version",
            |f| f.dismissed = true,
            Source::Power,
            Err(Hold::NotNow),
        ),
        (
            "a build of another channel",
            |f| f.channel = Channel::Nightly,
            Source::Power,
            Err(Hold::OtherChannel),
        ),
        (
            "a channel the token can't use",
            |f| f.channel_allowed = false,
            Source::Power,
            Err(Hold::OtherChannel),
        ),
        (
            "a link's token waits",
            |f| f.pending_token = true,
            Source::Power,
            Err(Hold::PendingToken),
        ),
        (
            "a go back waits",
            |f| f.go_back_waiting = true,
            Source::Power,
            Err(Hold::GoBackWaiting),
        ),
    ];
    for (name, change, power, want) in rows {
        let mut facts = allowed();
        change(&mut facts);
        assert_eq!(decide(&facts, || power), want, "{name}");
    }
}

#[test]
fn power_is_read_only_when_everything_else_allows() {
    let asked = AtomicUsize::new(0);
    let mut facts = allowed();
    facts.minute = 12 * 60;
    let _ = decide(&facts, || {
        asked.fetch_add(1, Ordering::SeqCst);
        Source::Power
    });
    assert_eq!(asked.load(Ordering::SeqCst), 0, "no pmset by day");
    let _ = decide(&allowed(), || {
        asked.fetch_add(1, Ordering::SeqCst);
        Source::Power
    });
    assert_eq!(asked.load(Ordering::SeqCst), 1);
}

// ── Looks on the client ─────────────────────────────────────────────────

struct Env {
    minute: AtomicU32,
    power: Mutex<Source>,
    last_started: Mutex<Option<u64>>,
    other: Mutex<Option<String>>,
}

impl Env {
    fn night() -> Self {
        Self {
            minute: AtomicU32::new(3 * 60),
            power: Mutex::new(Source::Power),
            last_started: Mutex::new(None),
            other: Mutex::new(None),
        }
    }
}

impl Surroundings for Env {
    fn local_minute(&self, _now_ms: u64) -> u32 {
        self.minute.load(Ordering::SeqCst)
    }
    fn power(&self) -> Source {
        *self.power.lock().unwrap()
    }
    fn last_started_ms(&self) -> Option<u64> {
        *self.last_started.lock().unwrap()
    }
    fn other_work(&self) -> Option<String> {
        self.other.lock().unwrap().clone()
    }
}

/// A rig with 0.6.11 verified and ready, and the setting on.
fn ready_rig(setup: Setup) -> Rig {
    let r = rig(setup);
    r.publish("0.6.11", "main");
    r.client.check();
    assert!(matches!(r.phase_now(), UpdatePhase::Ready { .. }));
    // Through the store: `set_overnight` would leave a wake pending.
    r.client
        .settings_store_for_tests()
        .update(|s| s.install_overnight = true)
        .unwrap();
    r
}

fn started_record(r: &Rig) -> last::Started {
    last::read_started(&r.root.join("data").join("updates").join(last::FILE))
        .expect("last-update.json")
}

#[test]
fn a_quiet_night_on_power_installs_through_the_manual_path() {
    let r = ready_rig(Setup::default());
    let env = Env::night();
    assert_eq!(
        r.client.overnight_look(&env),
        Look::Started("0.6.11".into())
    );
    assert_eq!(r.spawns(), 1);
    assert_eq!(r.quits(), 1);
    let args = r.args();
    assert!(args.contains(&"--expected-version".to_string()));
    assert!(args.contains(&"0.6.11".to_string()));
    let record = started_record(&r);
    assert!(record.overnight, "marked for the morning notification");
    assert!(!record.overnight_notified);
}

#[test]
fn each_condition_holds_the_look_without_starting_anything() {
    let cases: Vec<(&str, Box<dyn Fn(&Rig, &Env)>, Hold)> = vec![
        (
            "out of window",
            Box::new(|_, e| e.minute.store(6 * 60, Ordering::SeqCst)),
            Hold::OutOfWindow,
        ),
        (
            "on battery",
            Box::new(|_, e| *e.power.lock().unwrap() = Source::Battery),
            Hold::OnBattery,
        ),
        (
            "chat running",
            Box::new(|r, _| r.run(&["s-1"])),
            Hold::Work("chat s-1".into()),
        ),
        (
            "automation running",
            Box::new(|r, _| r.block(Some("An automation is running"))),
            Hold::Work("An automation is running".into()),
        ),
        (
            "build job",
            Box::new(|_, e| *e.other.lock().unwrap() = Some("A build job".into())),
            Hold::Work("A build job".into()),
        ),
        (
            "started 5 minutes ago",
            Box::new(|r, e| *e.last_started.lock().unwrap() = Some(r.now_ms() - 5 * 60 * 1000)),
            Hold::StartedRecently,
        ),
        (
            "skipped version",
            Box::new(|r, _| {
                r.client
                    .settings_store_for_tests()
                    .update(|s| s.skipped_version = Some("0.6.11".into()))
                    .unwrap();
            }),
            Hold::Skipped,
        ),
        (
            "Not now",
            Box::new(|r, _| {
                r.client.dismiss("0.6.11").unwrap();
            }),
            Hold::NotNow,
        ),
        (
            "setting off",
            Box::new(|r, _| {
                r.client.set_overnight(false).unwrap();
            }),
            Hold::Off,
        ),
    ];
    for (name, arrange, want) in cases {
        let r = ready_rig(Setup::default());
        let env = Env::night();
        arrange(&r, &env);
        assert_eq!(r.client.overnight_look(&env), Look::Held(want), "{name}");
        assert_eq!(r.spawns(), 0, "{name}: no installer");
        assert!(
            matches!(r.phase_now(), UpdatePhase::Ready { .. }),
            "{name}: still ready"
        );
    }
}

#[test]
fn a_build_that_fails_verification_is_refused_like_restart_to_update() {
    let r = ready_rig(Setup::default());
    r.fail_signing();
    let look = r.client.overnight_look(&Env::night());
    assert!(
        matches!(&look, Look::Refused(why) if why.contains("didn't pass Hollow's checks")),
        "{look:?}"
    );
    assert_eq!(r.spawns(), 0, "verified before any spawn");
    assert!(matches!(r.phase_now(), UpdatePhase::Failed { .. }));
}

#[test]
fn not_running_from_an_app_refuses_and_keeps_the_build() {
    let r = ready_rig(Setup {
        installed: false,
        ..Setup::default()
    });
    let look = r.client.overnight_look(&Env::night());
    assert!(matches!(look, Look::Refused(_)), "{look:?}");
    assert_eq!(r.spawns(), 0);
}

// ── The thread ──────────────────────────────────────────────────────────

#[test]
fn the_thread_looks_every_ten_minutes_until_the_window_and_installs_once() {
    let r = ready_rig(Setup::default());
    let env = Env::night();
    env.minute.store(60, Ordering::SeqCst);
    let looks = r.client.run_overnight(&env, overnight::EVERY, Some(3));
    assert_eq!(looks, vec![Look::Held(Hold::OutOfWindow); 3]);
    env.minute.store(130, Ordering::SeqCst);
    let looks = r.client.run_overnight(&env, overnight::EVERY, Some(3));
    // It keeps looking (the quit ends it); the build is installing, so
    // nothing more starts.
    assert_eq!(
        looks,
        vec![
            Look::Started("0.6.11".into()),
            Look::Held(Hold::NothingReady),
            Look::Held(Hold::NothingReady),
        ]
    );
    assert_eq!(r.spawns(), 1);
}

#[test]
fn a_last_look_that_calls_the_install_off_leaves_the_next_look_able_to_install() {
    let r = ready_rig(Setup::default());
    let env = Env::night();
    // Looks at running work: 1 the facts, 2 the install's check, 3 the last
    // look before the quit, where a chat starts; it ends by look 4.
    r.work_starts_on_look(3, 4, "A new chat");
    let looks = r.client.run_overnight(&env, overnight::EVERY, Some(2));
    assert_eq!(
        looks,
        vec![
            Look::Started("0.6.11".into()),
            Look::Started("0.6.11".into())
        ]
    );
    assert_eq!(r.aborted(), 1, "the first quit was called off");
    assert_eq!(r.quits(), 1, "the second went ahead");
    assert!(
        !r.client.state().waiting,
        "no \"when chats finish\" wait was armed (it would install at any hour)"
    );
    assert!(started_record(&r).overnight);
}

// ── What the install reads again ────────────────────────────────────────

#[test]
fn a_build_replaced_since_the_look_is_not_installed() {
    let r = ready_rig(Setup::default());
    r.publish("0.6.12", "main");
    let client = r.client.clone();
    // While the look reads running work, a check stages 0.6.12.
    r.during_work_look(1, move || {
        client.check();
    });
    assert_eq!(
        r.client.overnight_look(&Env::night()),
        Look::Held(Hold::Changed)
    );
    assert_eq!(r.spawns(), 0);
    assert!(
        matches!(r.phase_now(), UpdatePhase::Ready { release } if release.version == "0.6.12"),
        "the newer build waits for the next look"
    );
}

#[test]
fn a_skip_not_now_or_channel_change_during_the_install_is_refused_at_the_claim() {
    type Change = Box<dyn FnOnce(&Rig) -> Box<dyn FnOnce() + Send>>;
    let cases: Vec<(&str, Change, Hold)> = vec![
        (
            "skipped",
            Box::new(|r: &Rig| {
                let c = r.client.clone();
                Box::new(move || {
                    c.settings_store_for_tests()
                        .update(|s| s.skipped_version = Some("0.6.11".into()))
                        .unwrap();
                })
            }),
            Hold::Skipped,
        ),
        (
            "Not now",
            Box::new(|r: &Rig| {
                let c = r.client.clone();
                Box::new(move || {
                    c.dismiss("0.6.11").unwrap();
                })
            }),
            Hold::NotNow,
        ),
        (
            "channel switched",
            Box::new(|r: &Rig| {
                let c = r.client.clone();
                Box::new(move || {
                    c.settings_store_for_tests()
                        .update(|s| s.channel = Channel::Nightly)
                        .unwrap();
                })
            }),
            Hold::OtherChannel,
        ),
        (
            "channel no longer allowed",
            Box::new(|r: &Rig| {
                let c = r.client.clone();
                Box::new(move || {
                    c.settings_store_for_tests()
                        .update(|s| s.allowed_channels = Some(vec![Channel::Nightly]))
                        .unwrap();
                })
            }),
            Hold::OtherChannel,
        ),
        (
            "setting off",
            Box::new(|r: &Rig| {
                let c = r.client.clone();
                Box::new(move || {
                    c.settings_store_for_tests()
                        .update(|s| s.install_overnight = false)
                        .unwrap();
                })
            }),
            Hold::Off,
        ),
    ];
    for (name, change, want) in cases {
        let r = ready_rig(Setup::default());
        // Look 2 is the install's work check: after verify, before the claim.
        r.during_work_look(2, change(&r));
        assert_eq!(
            r.client.overnight_look(&Env::night()),
            Look::Held(want),
            "{name}"
        );
        assert_eq!(r.spawns(), 0, "{name}: no installer");
        assert!(
            matches!(r.phase_now(), UpdatePhase::Ready { .. }),
            "{name}: still ready"
        );
    }
}

#[test]
fn turning_the_setting_on_wakes_the_thread_at_once() {
    let r = ready_rig(Setup {
        naps: Naps::Interruptible,
        ..Setup::default()
    });
    // Off without a wake (as at launch), so the first nap is a full hour.
    r.client
        .settings_store_for_tests()
        .update(|s| s.install_overnight = false)
        .unwrap();
    let env = Arc::new(Env::night());
    let (client, e) = (r.client.clone(), env.clone());
    let thread = std::thread::spawn(move || {
        client.run_overnight(e.as_ref(), Duration::from_secs(3600), Some(2))
    });
    // The first look sees the setting off; the hour's nap ends at the switch.
    std::thread::sleep(Duration::from_millis(200));
    r.client.set_overnight(true).unwrap();
    let looks = thread.join().unwrap();
    assert_eq!(
        looks,
        vec![Look::Held(Hold::Off), Look::Started("0.6.11".into())]
    );
}

// ── The morning notification ────────────────────────────────────────────

fn record(overnight: bool, to: &str) -> last::Started {
    last::Started {
        schema: 1,
        install_id: "20261115-030000-cccc".into(),
        from_version: "0.6.10".into(),
        to_version: to.into(),
        notes: "Faster search.\nFixed the sidebar.".into(),
        channel: Channel::Main,
        at: "2026-11-15T03:00:00.000Z".into(),
        pid: 1,
        build_id: None,
        acknowledged: false,
        kind: last::StartedKind::Install,
        restore_data: false,
        skip_settled: false,
        local: None,
        overnight,
        overnight_notified: false,
    }
}

#[test]
fn the_morning_notification_is_posted_once_and_only_for_an_overnight_install() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(last::FILE);
    last::write(&path, &record(true, "0.6.11")).unwrap();
    let notice = last::overnight_notice(&path, None, "0.6.11", None, 2).expect("a notice");
    assert_eq!(notice.title, "Hollow updated overnight to 0.6.11");
    assert_eq!(notice.body, "Faster search.\nFixed the sidebar.");
    assert_eq!(notice.kind, "success");
    assert_eq!(
        last::overnight_notice(&path, None, "0.6.11", None, 2),
        None,
        "never twice"
    );
    // The window's own notice is untouched.
    assert!(matches!(
        last::read(&path, None, "0.6.11", None, 2),
        Some(UpdateLastResult::Updated { .. })
    ));
    // Not in the app that wrote it.
    last::write(&path, &record(true, "0.6.11")).unwrap();
    assert_eq!(last::overnight_notice(&path, None, "0.6.11", None, 1), None);
    // A person's own install gets none.
    last::write(&path, &record(false, "0.6.11")).unwrap();
    assert_eq!(last::overnight_notice(&path, None, "0.6.11", None, 2), None);
}

#[test]
fn a_failed_or_rolled_back_overnight_install_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(last::FILE);
    last::write(&path, &record(true, "0.6.11")).unwrap();
    let notice = last::overnight_notice(&path, Some(dir.path()), "0.6.10", None, 2).unwrap();
    assert_eq!(
        notice.title,
        "The overnight update to 0.6.11 didn't install"
    );
    assert!(notice.body.starts_with("Hollow is still on 0.6.10."));
    assert_eq!(notice.kind, "warning");

    last::write(&path, &record(true, "0.6.11")).unwrap();
    std::fs::write(
        dir.path().join(crate::update_install::run::FAILURE_FILE),
        serde_json::json!({
            "installId": "20261115-030000-cccc",
            "wentBack": true,
            "attemptedVersion": "0.6.11",
            "restoredVersion": "0.6.10",
            "reason": "The new version failed to start twice.",
            "at": "2026-11-15T03:05:00.000Z",
        })
        .to_string(),
    )
    .unwrap();
    let notice = last::overnight_notice(&path, Some(dir.path()), "0.6.10", None, 2).unwrap();
    assert_eq!(
        notice.title,
        "The overnight update to 0.6.11 was rolled back"
    );
    assert_eq!(
        notice.body,
        "Hollow went back to 0.6.10. The new version failed to start twice."
    );
    assert_eq!(
        last::overnight_notice(&path, Some(dir.path()), "0.6.10", None, 2),
        None
    );
}

#[test]
fn long_notes_are_cut() {
    let notes = "x".repeat(4000);
    let n = overnight::notice(
        &UpdateLastResult::Updated {
            version: "0.6.11".into(),
            notes,
            channel: Channel::Main,
            at: String::new(),
            local: None,
        },
        "0.6.11",
    );
    assert!(n.body.chars().count() <= 1501);
}

#[test]
fn the_reason_is_cut_and_names_no_home_folder() {
    let home = "/Users/someone";
    let why = format!(
        "Couldn't move {home}/Applications/Hollow.app: {}",
        "x".repeat(1000)
    );
    let plain = overnight::plain_reason(&why, Some(home));
    assert!(!plain.contains(home), "{plain}");
    assert!(plain.starts_with("Couldn't move ~/Applications/Hollow.app: "));
    assert!(plain.chars().count() <= 301, "300 and the ellipsis");
    assert!(plain.ends_with('…'));
    assert_eq!(overnight::plain_reason("Short.", Some(home)), "Short.");
    assert_eq!(
        overnight::plain_reason("At /Users/someone/", Some("/Users/someone/")),
        "At ~/"
    );

    // Through the notification, with this process's home.
    let home = std::env::var("HOME").unwrap();
    let n = overnight::notice(
        &UpdateLastResult::NotInstalled {
            attempted: "0.6.11".into(),
            reason: Some(format!(
                "The copy in {home}/Library failed. {}",
                "y".repeat(600)
            )),
            at: String::new(),
        },
        "0.6.10",
    );
    assert!(!n.body.contains(&home), "{}", n.body);
    assert!(n.body.contains("~/Library"));
    assert!(n.body.chars().count() <= "Hollow is still on 0.6.10. ".len() + 301);
}
