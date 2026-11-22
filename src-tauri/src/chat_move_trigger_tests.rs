//! M5 PR2: the sleep notice runs the mover once, turns stop at a safe point
//! or outright, the runner is picked, the setting's default, and the
//! release token stays in the person's own commands. The lid-close move
//! over real routes is `phone_access/move_send_tests.rs`.
use super::*;
use crate::session_store as store;
use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

/// Counts the mover's calls.
#[derive(Default)]
struct Counting {
    on: AtomicBool,
    moves: AtomicUsize,
    budgets: Mutex<Vec<Duration>>,
    wakes: AtomicUsize,
}

impl SleepMover for Counting {
    fn enabled(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }
    fn move_running(&self, budget: Duration) -> Fut<'_, usize> {
        self.moves.fetch_add(1, Ordering::SeqCst);
        self.budgets.lock().unwrap().push(budget);
        Box::pin(async { 1 })
    }
    fn woke(&self) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_sleep_notice_runs_the_mover_once_and_idle_sleep_never() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mover = Arc::new(Counting::default());
    mover.on.store(true, Ordering::SeqCst);
    let trigger = Trigger::new(mover.clone(), runtime.handle().clone());
    assert_eq!(power_event(0xE000_0280), Some(Power::WillSleep));
    assert_eq!(power_event(0xE000_0270), Some(Power::CanSleep));
    assert_eq!(power_event(0xE000_0300), Some(Power::PoweredOn));
    assert_eq!(power_event(0xE000_0320), None, "will power on: nothing");

    assert!(!trigger.on_power(Power::CanSleep));
    assert_eq!(
        mover.moves.load(Ordering::SeqCst),
        0,
        "idle sleep moves nothing"
    );
    assert!(trigger.on_power(Power::WillSleep));
    assert_eq!(mover.moves.load(Ordering::SeqCst), 1);
    assert_eq!(*mover.budgets.lock().unwrap(), vec![SLEEP_BUDGET]);
    assert!(!trigger.on_power(Power::PoweredOn));
    assert_eq!(mover.wakes.load(Ordering::SeqCst), 1);
    assert_eq!(
        mover.moves.load(Ordering::SeqCst),
        1,
        "waking moves nothing"
    );

    // The setting off, or no runner paired: a sleep moves nothing.
    mover.on.store(false, Ordering::SeqCst);
    assert!(!trigger.on_power(Power::WillSleep));
    assert_eq!(mover.moves.load(Ordering::SeqCst), 1);
}

/// A mover that panics never leaves the trigger stuck: the next sleep runs
/// it again.
#[test]
fn a_mover_that_panics_never_blocks_the_next_sleep() {
    struct Panicking(AtomicUsize);
    impl SleepMover for Panicking {
        fn enabled(&self) -> bool {
            true
        }
        fn move_running(&self, _budget: Duration) -> Fut<'_, usize> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                let moved: usize = "none".parse().expect("mover failed");
                moved
            })
        }
        fn woke(&self) {}
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mover = Arc::new(Panicking(AtomicUsize::new(0)));
    let trigger = Trigger::new(mover.clone(), runtime.handle().clone());
    for _ in 0..2 {
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            trigger.on_power(Power::WillSleep)
        }));
        assert!(caught.is_err());
    }
    assert_eq!(mover.0.load(Ordering::SeqCst), 2, "the flag was cleared");
}

/// A move open past 15 minutes is told once; a newer one isn't yet.
#[test]
fn a_move_stuck_past_fifteen_minutes_is_told_once() {
    let (_dir, running, key) = running_chat(true);
    let vault = running.vault.clone();
    let ord = running.turns.lock().unwrap().remove(&key).unwrap();
    store::finish_turn(&vault, &key, ord, "success").unwrap();
    let id = chat_move::new_move_id();
    let then = chrono::Utc::now() - chrono::Duration::minutes(20);
    chat_move::begin_move_out(
        &vault,
        &key,
        "m-3f9c1a07e2b45d18",
        "m-8d21c4be90f17a3e",
        &id,
        then,
    )
    .unwrap();
    let now = chrono::Utc::now();
    assert!(
        report_stuck(&vault, now - chrono::Duration::minutes(10)).is_empty(),
        "not yet"
    );
    assert_eq!(report_stuck(&vault, now), vec![key.clone()]);
    assert!(report_stuck(&vault, now).is_empty(), "told once");
}

#[test]
fn moving_on_sleep_is_off_unless_turned_on_and_needs_the_runner() {
    let mut settings = crate::agent_settings::AgentSettings::default();
    assert!(!move_on_sleep(&settings), "absent is off");
    settings.move_on_sleep = Some(false);
    assert!(!move_on_sleep(&settings));
    settings.move_on_sleep = Some(true);
    assert!(move_on_sleep(&settings));
    // On, and the runner is paired: a sleep moves. Anything less doesn't.
    let runner = [record("m-8d21c4be90f17a3e", &["runner"])];
    let other = [record("m-1111222233334444", &["desk"])];
    assert!(sleep_moves(Ok(settings.clone()), &runner, Some("runner")));
    assert!(
        !sleep_moves(Ok(settings.clone()), &other, Some("runner")),
        "not the runner"
    );
    assert!(
        !sleep_moves(Ok(settings.clone()), &runner, None),
        "no runner named"
    );
    assert!(
        !sleep_moves(Err("unreadable".into()), &runner, Some("runner")),
        "a read error is off"
    );
    let absent = crate::agent_settings::AgentSettings::default();
    assert!(
        !sleep_moves(Ok(absent), &runner, Some("runner")),
        "absent is off"
    );
    // A save without the field (Settings' payload today) keeps the choice.
    let mut saved = crate::agent_settings::AgentSettings::default();
    crate::agent_settings::carry_over(&settings, &mut saved);
    assert_eq!(saved.move_on_sleep, Some(true));
    saved.move_on_sleep = Some(false);
    crate::agent_settings::carry_over(&settings, &mut saved);
    assert_eq!(saved.move_on_sleep, Some(false), "a choice sent wins");
}

fn record(id: &str, hosts: &[&str]) -> MachineRecord {
    serde_json::from_value(serde_json::json!({
        "machineId": id, "name": format!("Mac {id}"),
        "sshHosts": hosts, "deviceId": "mc-1a2b3c4d", "pairedAt": "2026-11-15T00:00:00Z",
    }))
    .unwrap()
}

#[test]
fn only_the_paired_mac_the_runner_alias_reaches_takes_chats() {
    assert!(pick_runner(&[], Some("runner")).is_err());
    let one = [record("m-8d21c4be90f17a3e", &["a"])];
    assert!(
        pick_runner(&one, None).is_err(),
        "the only Mac, but no runner named"
    );
    assert!(pick_runner(&one, Some("runner")).is_err(), "not the runner");
    assert!(pick_runner(&one, Some("off")).is_err());
    let two = [
        record("m-1111222233334444", &["desk"]),
        record("m-8d21c4be90f17a3e", &["runner-lan", "runner"]),
    ];
    assert_eq!(
        pick_runner(&two, Some("runner")).unwrap().machine_id,
        "m-8d21c4be90f17a3e"
    );
    assert!(pick_runner(&two, None).is_err());
    assert!(pick_runner(&two, Some("elsewhere")).is_err());
}

/// M5 UI: the window offers Move to runner, and names the sleep's Mac,
/// only for the Mac the pickers choose, and only once it lists `moves`.
#[test]
fn the_window_names_the_move_targets_only_when_they_take_moves() {
    use crate::commands::peer_api::move_targets;
    let taking = |id: &str, hosts: &[&str]| MachineRecord {
        capabilities: vec!["chats".into(), "moves".into()],
        ..record(id, hosts)
    };
    let mini = "m-8d21c4be90f17a3e";
    assert_eq!(move_targets(&[], Some("runner")), (None, None));
    // The only Mac, no runner named: Move to runner, never the sleep.
    let one = [taking(mini, &["a"])];
    assert_eq!(move_targets(&one, None), (Some(mini.into()), None));
    // It is the runner: both.
    assert_eq!(
        move_targets(&one, Some("a")),
        (Some(mini.into()), Some(mini.into()))
    );
    // An older Hollow there (no `moves`): neither.
    let old = [record(mini, &["a"])];
    assert_eq!(move_targets(&old, Some("a")), (None, None));
    // Two, the runner not among them: neither.
    let two = [
        taking("m-1111222233334444", &["desk"]),
        taking(mini, &["b"]),
    ];
    assert_eq!(move_targets(&two, Some("a")), (None, None));
    assert_eq!(
        move_targets(&two, Some("b")),
        (Some(mini.into()), Some(mini.into()))
    );
}

#[test]
fn a_plan_keeps_time_to_send_and_to_settle_a_stop() {
    let now = Instant::now();
    let plan = StopPlan::sending_by(now + SLEEP_BUDGET);
    assert!(plan.safe_by <= plan.stop_by);
    assert!(plan.stop_by <= now + SLEEP_BUDGET);
    // 20 s: 8 kept for sending, 3 of the 12 left for a stop to settle.
    assert!(plan.stop_by + SEND_RESERVE <= now + SLEEP_BUDGET + POLL);
    assert!(plan.safe_by + STOP_SETTLE <= plan.stop_by + POLL);
    // Already late: nothing is negative, everything is now.
    let late = StopPlan::sending_by(now);
    assert!(late.safe_by <= late.stop_by);
}

/// Running turns in a vault: a safe-stop ask ends the ones that reach a
/// safe point; Stop ends any.
struct Running {
    vault: PathBuf,
    turns: Mutex<HashMap<String, i64>>,
    reaches_safe_point: bool,
    asked: AtomicUsize,
    stopped: AtomicUsize,
}

impl Running {
    fn end(&self, key: &str) -> bool {
        match self.turns.lock().unwrap().remove(key) {
            Some(ord) => {
                store::finish_turn(&self.vault, key, ord, "cancelled").unwrap();
                true
            }
            None => false,
        }
    }
}

impl Turns for Running {
    fn running(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.turns.lock().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }
    fn ask_safe_stop(&self, key: &str) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.reaches_safe_point && self.end(key)
    }
    fn stop_now(&self, key: &str) -> bool {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        self.end(key)
    }
    fn ended(&self, vault: &Path, key: &str) -> bool {
        !chat_move::turn_in_flight_now(vault, key)
    }
}

fn running_chat(reaches_safe_point: bool) -> (tempfile::TempDir, Running, String) {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().canonicalize().unwrap();
    let cwd = crate::workspaces::ensure_quick_chats(&vault).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let key = "s-20261115-tr000001".to_string();
    let ord = store::start_turn(&vault, &key, "claude", &cwd, "run-1", "Long job").unwrap();
    let running = Running {
        vault,
        turns: Mutex::new(HashMap::from([(key.clone(), ord)])),
        reaches_safe_point,
        asked: AtomicUsize::new(0),
        stopped: AtomicUsize::new(0),
    };
    (dir, running, key)
}

#[tokio::test]
async fn a_turn_stops_at_its_safe_point_or_is_stopped_outright() {
    for safe in [true, false] {
        let (_dir, running, key) = running_chat(safe);
        let now = Instant::now();
        let plan = StopPlan {
            safe_by: now + POLL,
            stop_by: now + Duration::from_secs(30),
        };
        let vault = running.vault.clone();
        let ended = stop_turns(&running, &vault, std::slice::from_ref(&key), plan).await;
        assert_eq!(ended, vec![key.clone()], "safe point reached: {safe}");
        assert_eq!(running.asked.load(Ordering::SeqCst), 1);
        assert_eq!(
            running.stopped.load(Ordering::SeqCst),
            usize::from(!safe),
            "Stop only when no safe point came: {safe}"
        );
        assert!(!chat_move::turn_in_flight_now(&vault, &key));
    }
}

/// `UserConfirmed` (the release token) is made only in the person's own
/// commands: every file under `src/` that names `by_the_person` is one of
/// these three.
#[test]
fn only_the_persons_own_commands_make_the_release_token() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let allowed = [
        "chat_move.rs",
        "chat_move_tests.rs",
        "commands/chat_move_api.rs",
        "chat_move_trigger_tests.rs",
    ];
    let mut stack = vec![root.clone()];
    let mut makers = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                let word = concat!("by_the", "_person");
                let names = text.match_indices(word).any(|(at, _)| {
                    let next = text[at + word.len()..].chars().next();
                    !next.is_some_and(|c| c.is_alphanumeric() || c == '_')
                });
                if names {
                    let rel = path.strip_prefix(&root).unwrap();
                    makers.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    makers.sort();
    for maker in &makers {
        assert!(
            allowed.contains(&maker.as_str()),
            "{maker} makes the release token"
        );
    }
    assert!(makers.iter().any(|m| m == "commands/chat_move_api.rs"));
}

/// Move to runner (the person's click) takes the runner the alias reaches,
/// else the only paired Mac; several with none matching is refused, naming
/// the alias and the aliases compared. The sleep move takes only the
/// runner.
#[test]
fn move_to_runner_takes_the_only_paired_mac_and_sleep_only_the_runner() {
    let one = [record("m-8d21c4be90f17a3e", &["home-mac"])];
    assert_eq!(
        pick_for_move(&one, Some("build-box")).unwrap().machine_id,
        "m-8d21c4be90f17a3e"
    );
    assert!(pick_for_move(&one, None).is_ok());
    assert!(
        pick_runner(&one, Some("build-box")).is_err(),
        "sleep: runner only"
    );
    let two = [
        record("m-1111222233334444", &["desk"]),
        record("m-8d21c4be90f17a3e", &["home-mac"]),
    ];
    let refused = pick_for_move(&two, Some("build-box")).unwrap_err();
    assert!(refused.contains("\"build-box\""), "{refused}");
    assert!(refused.contains("desk, home-mac"), "{refused}");
    assert!(pick_for_move(&two, None).is_err());
    assert!(pick_for_move(&[], None).is_err());
    let three = [
        record("m-1111222233334444", &["desk"]),
        record("m-8d21c4be90f17a3e", &["home-mac", "build-box"]),
    ];
    assert_eq!(
        pick_for_move(&three, Some("build-box")).unwrap().machine_id,
        "m-8d21c4be90f17a3e"
    );
}
