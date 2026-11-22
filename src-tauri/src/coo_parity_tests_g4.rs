//! COO parity L13, group g4: the agent actions for private memories, skill
//! lessons, memory maintenance, skill updates and notes, steering and held
//! sends, review wake, vault conflict copies, interrupted chats, the task
//! tracker's key and settings, agent CLI restore and cancel, and connected
//! project workspaces. Tempdir vaults and the in-memory secret store only.

use crate::app_actions::{find, validate, Cx, Kind, Lease, Tier, ACTIONS};
use serde_json::{json, Value};
use std::path::Path;

/// Every g4 action: id, the Tauri command it mirrors, tier and lease.
const G4: &[(&str, &str, Tier, Lease)] = &[
    (
        "workspaces.open_project_workspace",
        "open_project_workspace",
        Tier::Write,
        Lease::Own,
    ),
    (
        "agents.restore_cli",
        "restore_agent_cli",
        Tier::Confirm,
        Lease::Outside,
    ),
    (
        "agents.cancel_cli_update",
        "cancel_agent_cli_update",
        Tier::Write,
        Lease::Outside,
    ),
    (
        "learning.list_private_memories",
        "list_private_memories",
        Tier::Read,
        Lease::Wrap,
    ),
    (
        "learning.list_skill_lessons",
        "list_skill_lessons",
        Tier::Read,
        Lease::Wrap,
    ),
    (
        "learning.promote_private_memory",
        "promote_private_memory",
        Tier::Write,
        Lease::Wrap,
    ),
    (
        "learning.memory_maintenance",
        "memory_maintenance",
        Tier::Write,
        Lease::Wrap,
    ),
    (
        "tracker.set_key",
        "set_tracker_key",
        Tier::Confirm,
        Lease::Outside,
    ),
    (
        "tracker.clear_key",
        "clear_tracker_key",
        Tier::Confirm,
        Lease::Outside,
    ),
    (
        "tracker.use_keychain_key",
        "use_keychain_tracker_key",
        Tier::Confirm,
        Lease::Outside,
    ),
    (
        "tracker.set_settings",
        "set_tracker_settings",
        Tier::Confirm,
        Lease::Outside,
    ),
    (
        "sessions.list_interrupted_chats",
        "list_interrupted_chats",
        Tier::Read,
        Lease::Wrap,
    ),
    (
        "sessions.resume_interrupted_chats",
        "resume_interrupted_chats",
        Tier::Confirm,
        Lease::Outside,
    ),
    (
        "vault.conflict_copy_pair",
        "conflict_copy_pair",
        Tier::Read,
        Lease::Wrap,
    ),
    (
        "vault.resolve_conflict_copy",
        "resolve_conflict_copy",
        Tier::Confirm,
        Lease::Wrap,
    ),
    (
        "vault.dismiss_conflict_copy",
        "dismiss_conflict_copy",
        Tier::Write,
        Lease::Outside,
    ),
    (
        "vault.undismiss_conflict_copy",
        "undismiss_conflict_copy",
        Tier::Write,
        Lease::Outside,
    ),
    (
        "review.wake_review_item",
        "wake_review_item",
        Tier::Write,
        Lease::Wrap,
    ),
    (
        "sessions.steer_session",
        "chat_steer",
        Tier::Write,
        Lease::Outside,
    ),
    (
        "sessions.cancel_held_send",
        "chat_cancel_held",
        Tier::Write,
        Lease::Outside,
    ),
    (
        "skills.stage_skill_update",
        "stage_skill_update",
        Tier::External,
        Lease::Outside,
    ),
    (
        "skills.set_skill_note",
        "set_skill_note",
        Tier::Confirm,
        Lease::Outside,
    ),
];

/// A key that must never come back out of an action.
const SECRET: &str = "lin_api_cooParityG4Secret0123";

fn app_on(v: &Path) -> tauri::App<tauri::test::MockRuntime> {
    let mut config = crate::config::Config::placeholder();
    config.vault_path = v.to_path_buf();
    tauri::test::mock_builder()
        .manage(crate::commands::AppState::new(
            config,
            v.join("config.json"),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}

/// Run action `id`'s handler as the test chat (`Cx::for_test`).
fn run(app: &tauri::App<tauri::test::MockRuntime>, id: &str, args: Value) -> Result<Value, String> {
    let action = find(id).unwrap_or_else(|| panic!("{id} is not an action"));
    let args = validate(action, &args).unwrap_or_else(|e| panic!("{id}: {e}"));
    let cx = Cx::for_test(Box::new(app.handle().clone()));
    tauri::async_runtime::block_on((action.run)(cx, args))
}

fn vault() -> tempfile::TempDir {
    // Not `.tmp…`: the vault walk skips hidden folders, its root too.
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("notes")).unwrap();
    dir
}

#[test]
fn every_g4_action_resolves_with_its_tier_command_and_lease() {
    for (id, command, tier, lease) in G4 {
        let action = find(id).unwrap_or_else(|| panic!("{id} is not an action"));
        assert_eq!(action.tier, *tier, "{id}");
        assert_eq!(action.command, Some(*command), "{id}");
        assert_eq!(action.lease, *lease, "{id}");
        assert_eq!(id.split('.').next(), Some(action.area), "{id}");
        assert_eq!(
            ACTIONS.iter().filter(|a| a.id == *id).count(),
            1,
            "{id} is registered once"
        );
    }
}

/// Approving or rejecting a learning, a skill lesson or a review item, and
/// the lifecycle edit that closes a waiting learning proposal, stay the
/// person's: no action mirrors them, under any id.
#[test]
fn the_persons_decisions_stay_out_of_reach() {
    for name in [
        "resolve_learning_proposal",
        "resolve_learning_proposals_batch",
        "undo_learning_batch",
        "resolve_skill_lesson",
        "undo_skill_lesson",
        "resolve_review_item",
        "undo_review_reject",
        "edit_memory_maintenance",
        "edit_tracker_item",
    ] {
        assert!(
            ACTIONS
                .iter()
                .all(|a| a.command != Some(name) && !a.id.ends_with(name)),
            "{name} must have no action"
        );
    }
}

/// Every g4 confirm action is placed for auto mode (all of them ask), and
/// every external one has its own card naming where it goes.
#[test]
fn g4_cards_ask_where_they_must() {
    for (id, _, tier, _) in G4 {
        match tier {
            Tier::Confirm => assert!(crate::app_actions::auto_asks(id), "{id}"),
            Tier::External => assert!(find(id).unwrap().card.is_some(), "{id}"),
            _ => {}
        }
    }
    assert!(find("sessions.resume_interrupted_chats")
        .unwrap()
        .card
        .is_some());
}

/// The tracker key goes in only as a secret parameter: the card shows it
/// masked, and no other g4 action takes a secret.
#[test]
fn the_tracker_key_is_a_secret_parameter_and_its_card_masks_it() {
    for (id, ..) in G4 {
        let secrets: Vec<&str> = find(id)
            .unwrap()
            .params
            .iter()
            .filter(|p| matches!(p.kind, Kind::Secret))
            .map(|p| p.name)
            .collect();
        let expected: &[&str] = if *id == "tracker.set_key" {
            &["key"]
        } else {
            &[]
        };
        assert_eq!(secrets, expected, "{id}");
    }
    let action = find("tracker.set_key").unwrap();
    let args = validate(action, &json!({ "key": SECRET })).unwrap();
    let card = crate::app_actions::generic_card(action, &args);
    assert!(!card.text().contains(SECRET), "{}", card.text());
    assert!(card.text().contains("••••"), "{}", card.text());
}

/// The tests that save or copy Linear's bundle key take turns: the
/// in-memory store is shared, and one test's clear would race another's.
static LINEAR_KEY: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn tracker_state(dir: &Path) -> crate::commands::AppState {
    let vault = dir.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let config: crate::config::Config = serde_json::from_value(json!({
        "vaultPath": vault,
        "tracker": {
            "provider": "linear",
            "linear": { "teamId": "11111111-1111-4111-8111-111111111111", "teamKey": "ENG" }
        }
    }))
    .unwrap();
    crate::commands::AppState::new(config, dir.join("config.json"))
}

/// Saving, copying and clearing the tracker key return the status, whose
/// `key` says saved or missing and never carries the value; a refusal
/// doesn't echo it either.
#[test]
fn tracker_key_results_never_carry_the_key() {
    use crate::commands::tracker_api::{
        copy_keychain_tracker_key, remove_tracker_key, save_tracker_key,
    };
    let _key = LINEAR_KEY.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let state = tracker_state(dir.path());
    let refused = save_tracker_key(&state, "lin_api_has a space").unwrap_err();
    assert!(!refused.contains("lin_api"), "{refused}");

    let saved = serde_json::to_string(&save_tracker_key(&state, SECRET).unwrap()).unwrap();
    assert!(!saved.contains(SECRET), "{saved}");
    assert!(saved.contains("\"key\":\"saved\""), "{saved}");
    let cleared = serde_json::to_string(&remove_tracker_key(&state).unwrap()).unwrap();
    assert!(cleared.contains("\"key\":\"missing\""), "{cleared}");

    crate::secret_store::tests::seed_item("hollow-linear", "default", SECRET);
    let copied = serde_json::to_string(&copy_keychain_tracker_key(&state).unwrap()).unwrap();
    assert!(!copied.contains(SECRET), "{copied}");
    assert!(copied.contains("\"key\":\"saved\""), "{copied}");
    remove_tracker_key(&state).unwrap();

    // No tracker chosen: refused before the Keychain is touched.
    let none = crate::commands::AppState::new(
        crate::config::Config::placeholder(),
        dir.path().join("none.json"),
    );
    let first = save_tracker_key(&none, SECRET).unwrap_err();
    assert!(first.contains("Choose a task tracker"), "{first}");
    assert!(!first.contains(SECRET));
}

/// `tracker.configure` (T6b) refuses any team but Engineering before
/// writing anything, saves the block through Settings' path, copies the
/// Keychain item when asked, and never returns the key.
#[test]
fn tracker_configure_refuses_another_team_and_never_echoes_the_key() {
    use crate::commands::tracker_api::configure_tracker;
    use crate::tracker::settings::TrackerConfigure;
    let _key = LINEAR_KEY.lock().unwrap_or_else(|e| e.into_inner());
    let action = find("tracker.configure").unwrap();
    assert_eq!((action.tier, action.lease), (Tier::Confirm, Lease::Outside));
    assert!(action
        .params
        .iter()
        .all(|p| !matches!(p.kind, Kind::Secret)));
    let dir = tempfile::tempdir().unwrap();
    let state = tracker_state(dir.path());
    let path = dir.path().join("config.json");
    let original = "{\"vaultPath\": \"/vault\"}";
    std::fs::write(&path, original).unwrap();
    crate::secret_store::tests::seed_item("hollow-linear", "default", SECRET);
    let ai = "11111111-1111-4111-8111-111111111111";
    for (team, key) in [
        ("8d8e6207-0000-4000-8000-000000000000", None),
        (ai, Some("LP")),
    ] {
        let request = TrackerConfigure {
            team_id: team.into(),
            team_key: key.map(Into::into),
            copy_keychain_key: true,
            ..Default::default()
        };
        let refused = configure_tracker(&state, &request).unwrap_err();
        assert!(
            refused.contains("team ENG") && !refused.contains(SECRET),
            "{refused}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "nothing written"
        );
    }
    let request: TrackerConfigure = serde_json::from_value(json!({
        "teamId": ai, "pollSeconds": 300, "trackAgentWork": true, "copyKeychainKey": true
    }))
    .unwrap();
    let status = configure_tracker(&state, &request).unwrap();
    let wire = serde_json::to_string(&status).unwrap();
    assert!(
        !wire.contains(SECRET) && wire.contains("\"key\":\"saved\""),
        "{wire}"
    );
    assert_eq!(
        crate::tracker::settings::saved_key("linear")
            .unwrap()
            .as_deref(),
        Some(SECRET)
    );
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        saved["tracker"],
        json!({ "provider": "linear", "pollSeconds": 300, "trackAgentWork": true,
                "linear": { "teamId": ai, "teamKey": "ENG" } })
    );
    assert_eq!(saved["vaultPath"], json!("/vault"), "other keys are kept");
    crate::commands::tracker_api::remove_tracker_key(&state).unwrap();
}

/// `tracker.set_settings` writes config.json's tracker block (kept for the
/// next launch) and says a restart is needed; a bad value changes nothing.
#[test]
fn tracker_settings_are_saved_for_the_next_launch() {
    use crate::commands::tracker_api::save_tracker_settings;
    use crate::tracker::settings::TrackerSettingsUpdate;
    let dir = tempfile::tempdir().unwrap();
    let state = tracker_state(dir.path());
    let path = dir.path().join("config.json");
    std::fs::write(&path, "{\"vaultPath\": \"/vault\"}").unwrap();
    let bad = TrackerSettingsUpdate {
        poll_seconds: Some(5),
        ..Default::default()
    };
    assert!(save_tracker_settings(&state, &bad).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\"vaultPath\": \"/vault\"}"
    );
    let update = TrackerSettingsUpdate {
        provider: Some("linear".into()),
        poll_seconds: Some(600),
        ..Default::default()
    };
    let status = save_tracker_settings(&state, &update).unwrap();
    assert!(status.restart_needed);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["tracker"]["provider"], json!("linear"));
    assert_eq!(saved["vaultPath"], json!("/vault"), "other keys are kept");
}

/// H4b: turning the tracker off while tasks follow it is a switch, not a
/// Save: refused before any write. Once tasks are pinned to the vault (a
/// switch ran), the tracker row may turn it off.
#[test]
fn turning_the_tracker_off_under_tracked_tasks_is_refused() {
    use crate::commands::tracker_api::{save_tracker_settings, TRACKER_OFF_IS_A_SWITCH};
    use crate::tracker::settings::TrackerSettingsUpdate;
    let dir = tempfile::tempdir().unwrap();
    let state = tracker_state(dir.path());
    let path = dir.path().join("config.json");
    let tracked = "{\"vaultPath\": \"/vault\", \"tracker\": {\"provider\": \"linear\"}}";
    std::fs::write(&path, tracked).unwrap();
    let off = TrackerSettingsUpdate {
        provider: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(
        save_tracker_settings(&state, &off).unwrap_err(),
        TRACKER_OFF_IS_A_SWITCH
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), tracked);
    let pinned = "{\"vaultPath\": \"/vault\", \"tracker\": {\"provider\": \"linear\"}, \"sources\": {\"version\": 1, \"tasks\": {\"owner\": \"vault\"}}}";
    std::fs::write(&path, pinned).unwrap();
    assert!(save_tracker_settings(&state, &off).is_ok());
}

/// The agent's resolve runs inside the run that holds the vault (History
/// records the move); the person's path refuses on the edit lease there.
/// Both refuse a file that changed since its hashes were read.
#[test]
fn a_conflict_copy_resolves_inside_the_run_that_holds_the_vault() {
    use crate::conflict_copies::{pair, resolve, resolve_for_live_run};
    let dir = vault();
    let v = dir.path();
    std::fs::write(v.join("notes/Plan.md"), "old\n").unwrap();
    std::fs::write(v.join("notes/Plan 2.md"), "new\n").unwrap();
    crate::vault::invalidate_walk_cache();
    let p = pair(v, "notes/Plan 2.md").unwrap();
    let held = crate::automated_history::try_begin_run(v, "chat", None).unwrap();
    assert!(resolve(v, "notes/Plan 2.md", false, &p.copy_hash, &p.original_hash).is_err());
    assert!(v.join("notes/Plan 2.md").exists(), "nothing moved");
    let stale =
        resolve_for_live_run(v, "notes/Plan 2.md", false, "0", &p.original_hash).unwrap_err();
    assert!(stale.contains("changed since"), "{stale}");
    assert_eq!(
        resolve_for_live_run(v, "notes/Plan 2.md", false, &p.copy_hash, &p.original_hash).unwrap(),
        "notes/Plan.md"
    );
    assert!(!v.join("notes/Plan 2.md").exists());
    assert_eq!(
        std::fs::read_to_string(v.join("notes/Plan.md")).unwrap(),
        "old\n"
    );
    crate::automated_history::finish_run(v, held, "success").unwrap();
}

/// Read the pair, mark it Not a copy and take the mark back, through the
/// actions; nothing in the vault changes.
#[test]
fn a_pair_is_read_dismissed_and_offered_again_through_the_actions() {
    let dir = vault();
    let v = dir.path();
    std::fs::write(v.join("notes/Chapter.md"), "part one").unwrap();
    std::fs::write(v.join("notes/Chapter 2.md"), "part two").unwrap();
    crate::vault::invalidate_walk_cache();
    let app = app_on(v);
    let copy = json!({ "copy": "notes/Chapter 2.md" });
    let read = run(&app, "vault.conflict_copy_pair", copy.clone()).unwrap();
    assert_eq!(read["originalText"], json!("part one"));
    assert!(read["copyHash"].as_str().is_some_and(|h| !h.is_empty()));
    let dismissed = run(&app, "vault.dismiss_conflict_copy", copy.clone()).unwrap();
    assert_eq!(dismissed["original"], json!("notes/Chapter.md"));
    assert!(crate::conflict_copies::scan(v).is_empty());
    run(&app, "vault.undismiss_conflict_copy", copy.clone()).unwrap();
    assert_eq!(crate::conflict_copies::scan(v).len(), 1, "offered again");
    assert!(run(&app, "vault.undismiss_conflict_copy", copy).is_err());
    assert_eq!(
        std::fs::read_to_string(v.join("notes/Chapter 2.md")).unwrap(),
        "part two"
    );
}

/// Wake now puts a snoozed item back to pending and settles nothing; an
/// item that isn't snoozed is refused.
#[test]
fn waking_a_snoozed_item_makes_it_pending_again() {
    let dir = vault();
    let v = dir.path();
    let item = crate::review_queue::enqueue(
        v,
        crate::review_queue::NewReviewItem {
            kind: "heartbeat_task".into(),
            fingerprint: "task:coo-g4-wake".into(),
            payload: json!({ "title": "Call the vendor" }),
            risk: "local_write".into(),
            source: "heartbeat".into(),
            source_path: None,
        },
    )
    .unwrap();
    let app = app_on(v);
    let id = json!({ "id": item.id });
    assert!(run(&app, "review.wake_review_item", id.clone()).is_err());
    let until = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    crate::review_queue::snooze(v, &item.id, &until).unwrap();
    let woken = run(&app, "review.wake_review_item", id).unwrap();
    assert_eq!(woken["status"], json!("pending"));
}

/// A steer and a held-send cancel never reach the calling conversation,
/// and a steer reaches only a conversation the caller started.
#[test]
fn steering_is_only_for_conversations_the_caller_started() {
    let dir = vault();
    let app = app_on(dir.path());
    // `Cx::for_test`'s own conversation.
    let own = "s-20260927-test";
    let refused = run(
        &app,
        "sessions.steer_session",
        json!({ "session": own, "text": "faster" }),
    )
    .unwrap_err();
    assert!(refused.contains("you are running in"), "{refused}");
    let cancel = run(
        &app,
        "sessions.cancel_held_send",
        json!({ "session": own, "heldId": "held-0000000000000001" }),
    )
    .unwrap_err();
    assert!(cancel.contains("you are running in"), "{cancel}");
    let stranger = run(
        &app,
        "sessions.steer_session",
        json!({ "session": "s-20261110-not-mine", "text": "faster" }),
    )
    .unwrap_err();
    assert!(
        stranger.contains("only a conversation you started"),
        "{stranger}"
    );
    let none = run(
        &app,
        "sessions.cancel_held_send",
        json!({ "session": "s-20261110-not-mine", "heldId": "held-0000000000000001" }),
    )
    .unwrap();
    assert_eq!(none["cancelled"], json!(false));
}

/// `steer_turn` (the person's Steer now and the COO's action) refuses a
/// conversation with no running turn, with the steering code the composer
/// reads, and emits nothing.
#[test]
fn a_steer_needs_a_running_turn() {
    let dir = vault();
    let error = crate::commands::steer_turn(
        None,
        dir.path(),
        "s-20261110-idle",
        "faster",
        None,
        crate::steering::SteerBy::Agent { run_id: "run-test" },
    )
    .unwrap_err();
    assert_eq!(error, crate::steering::STEER_NOT_RUNNING);
}

/// L13 review: the COO steers only a conversation it created (a
/// `start_session` child task), and only while the turn it asked for runs:
/// never one it only sent to, nor a child whose turn ended or waits queued.
#[test]
fn only_a_created_childs_running_turn_can_be_steered() {
    use crate::descendants::tests::{child_turn, record_child};
    use crate::run_queue::RowStatus;
    let dir = vault();
    let v = dir.path();
    let app = app_on(v);
    let caller = Cx::for_test(Box::new(app.handle().clone())).caller;
    // Created and running: its turn's row.
    let running = child_turn(v, "s-20261110-kid-run", "R-kid-run", RowStatus::Running);
    record_child(v, &caller.run_id, "s-20261110-kid-run", running);
    assert_eq!(
        crate::commands::sessions_api::steerable_child(v, &caller, "s-20261110-kid-run"),
        Ok(running)
    );
    // Only sent to (someone else started it), even with a running turn.
    let sent = child_turn(v, "s-20261110-sent", "R-sent", RowStatus::Running);
    let record = crate::run_manifest::DelegationRecord {
        kind: "childTask".into(),
        tool: "send_to_session".into(),
        agent: "claude".into(),
        child_session: Some("s-20261110-sent".into()),
        child_queue_id: Some(sent),
        at: crate::session_store::db_time(chrono::Utc::now()),
        ..Default::default()
    };
    crate::delegation::store(v, &caller.run_id, &record);
    let refused =
        crate::commands::sessions_api::steerable_child(v, &caller, "s-20261110-sent").unwrap_err();
    assert!(
        refused.contains("only a conversation you started"),
        "{refused}"
    );
    // Created, but the turn it asked for ended or is still queued.
    for (key, status) in [
        ("s-20261110-kid-done", RowStatus::Done),
        ("s-20261110-kid-queued", RowStatus::Queued),
    ] {
        let row = child_turn(v, key, "R-kid-other", status);
        record_child(v, &caller.run_id, key, row);
        let refused = crate::commands::sessions_api::steerable_child(v, &caller, key).unwrap_err();
        assert!(
            refused.contains("no running turn you asked for"),
            "{key}: {refused}"
        );
    }
}

/// L13 review: the COO cancels only a held send it sent itself; the
/// person's own Queue for after (no sending run) and another run's stay held.
#[test]
fn a_held_send_is_cancelled_only_by_the_run_that_sent_it() {
    let key = "s-20261110-held-owner";
    let held = |id: &str, run: Option<&str>| crate::steering::Held {
        id: id.into(),
        after: crate::steering::After::Queue,
        at: "2026-11-10T09:00:00Z".into(),
        input: crate::steering::HeldInput {
            prompt: "later".into(),
            delegation_run: run.map(str::to_string),
            ..Default::default()
        },
    };
    crate::steering::hold(key, held("held-person", None)).unwrap();
    crate::steering::hold(key, held("held-other", Some("run-other"))).unwrap();
    crate::steering::hold(key, held("held-mine", Some("run-test"))).unwrap();
    for id in ["held-person", "held-other"] {
        let refused = crate::steering::cancel_held_from(key, id, "run-test").unwrap_err();
        assert!(
            refused.contains("isn't a message you sent"),
            "{id}: {refused}"
        );
    }
    assert_eq!(crate::steering::held_count(key), 3, "nothing was removed");
    assert_eq!(
        crate::steering::cancel_held_from(key, "held-mine", "run-test"),
        Ok(true)
    );
    assert_eq!(
        crate::steering::cancel_held_from(key, "held-mine", "run-test"),
        Ok(false)
    );
    assert_eq!(crate::steering::held_count(key), 2);
    // Leave the shared store as it was.
    assert_eq!(crate::steering::cancel_held(key, "held-person"), Ok(true));
    assert_eq!(crate::steering::cancel_held(key, "held-other"), Ok(true));
}

/// Resume requests are the keys asked for, trimmed and each once; the card
/// refuses when none of them is waiting to resume.
#[test]
fn resuming_takes_each_offered_chat_once_and_refuses_none() {
    use crate::commands::sessions_api::{resume_interrupted_card, resume_requests};
    let keys = [
        " s-20261110-a ".to_string(),
        "s-20261110-a".to_string(),
        String::new(),
        "s-20261110-b".to_string(),
    ];
    let requests = resume_requests(&keys);
    let asked: Vec<&str> = requests.iter().map(|r| r.session_key.as_str()).collect();
    assert_eq!(asked, ["s-20261110-a", "s-20261110-b"]);
    assert!(requests
        .iter()
        .all(|r| r.canvases.is_none() && !r.ultracode));
    let dir = vault();
    let app = app_on(dir.path());
    let action = find("sessions.resume_interrupted_chats").unwrap();
    let args = validate(action, &json!({ "sessions": ["s-20261110-a"] })).unwrap();
    let cx = Cx::for_test(Box::new(app.handle().clone()));
    let refused = resume_interrupted_card(&cx, &args).unwrap_err();
    assert!(refused.contains("None of these chats"), "{refused}");
}

/// L13 review: the COO's steer is stored `by: agent` with its run, so the
/// chat's task state never reads it as the person's direction; the person's
/// own steer on the same turn stays theirs.
#[test]
fn a_coo_steer_is_stored_as_the_agents_never_the_persons() {
    let dir = vault();
    let v = dir.path();
    let key = "s-20261110-steered";
    let cwd = v.to_str().unwrap();
    let ord = crate::session_store::start_turn(
        v,
        key,
        "claude",
        cwd,
        "R-steered",
        "Refactor the parser.",
    )
    .unwrap();
    let _live = crate::steering::open(
        key,
        "R-steered",
        (
            crate::agent::AgentKind::Claude,
            crate::steering::SteerSupport::Native,
        ),
        ord,
        v.to_path_buf(),
        serde_json::Map::new(),
    );
    let agents = crate::commands::steer_turn(
        None,
        v,
        key,
        "Don't touch the public API.",
        None,
        crate::steering::SteerBy::Agent { run_id: "run-test" },
    )
    .unwrap();
    assert!(agents.to_string().contains("run-test"), "{agents}");
    crate::commands::steer_turn(
        None,
        v,
        key,
        "Keep the old error names.",
        None,
        crate::steering::SteerBy::Person,
    )
    .unwrap();
    let turns = crate::session_store::task_turns(v, key, 0, i64::MAX).unwrap();
    let turn = turns
        .iter()
        .find(|t| t.ord == ord)
        .expect("the steered turn");
    assert_eq!(turn.agent_steers, ["Don't touch the public API."]);
    assert_eq!(turn.steers, ["Keep the old error names."]);
}
