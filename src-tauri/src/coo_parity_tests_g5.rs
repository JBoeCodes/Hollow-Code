//! L13 COO parity, group g5: workspaces, triage, vault answers, weekly review,
//! today's brief, meeting recap, value baselines, result export, recordings.

use crate::app_actions::{find, validate, Kind, Tier};
use serde_json::json;
use std::cell::{Cell, RefCell};

const ALL: &[(&str, &str, Tier)] = &[
    (
        "workspaces.set_runs_on",
        "workspace_set_runs_on",
        Tier::Confirm,
    ),
    ("vault.answer_question", "vault_answer_ask", Tier::Write),
    ("vault.save_answer", "vault_answer_save", Tier::Write),
    ("digest.weekly_review", "weekly_review", Tier::Read),
    (
        "digest.weekly_review_take",
        "weekly_review_take",
        Tier::Write,
    ),
    (
        "digest.weekly_review_save",
        "weekly_review_save",
        Tier::Write,
    ),
    ("sessions.write_today_brief", "today_brief", Tier::Write),
    ("agenda.recap_meeting", "meeting_recap", Tier::Write),
    (
        "workspaces.export_result_markdown",
        "result_export_markdown",
        Tier::Confirm,
    ),
    ("value.set_baseline", "set_baseline", Tier::Confirm),
    (
        "workspaces.preview_source_refresh",
        "preview_source_refresh",
        Tier::Read,
    ),
    (
        "workspaces.refresh_source",
        "apply_source_refresh",
        Tier::Confirm,
    ),
    (
        "workspaces.create_main_standing_orders",
        "create_main_standing_orders",
        Tier::Confirm,
    ),
    (
        "workspaces.restore_workspace",
        "restore_workspace",
        Tier::Write,
    ),
    (
        "workspaces.trashed_workspaces",
        "trashed_workspaces",
        Tier::Read,
    ),
    (
        "recordings.keep_transcript",
        "keep_recording_transcript",
        Tier::Write,
    ),
    ("workspaces.triage_arrivals", "triage_arrivals", Tier::Read),
    ("workspaces.triage_suggest", "triage_suggest", Tier::Read),
    ("workspaces.triage_mark", "triage_mark", Tier::Write),
];

#[test]
fn every_g5_action_resolves_with_its_tier_and_command() {
    for (id, command, tier) in ALL {
        let action = find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(action.tier, *tier, "{id}");
        assert_eq!(action.command, Some(*command), "{id}");
        assert!(id.starts_with(&format!("{}.", action.area)), "{id}");
    }
    assert_eq!(ALL.len(), 19);
}

#[test]
fn every_confirm_action_has_a_card_and_every_model_call_runs_outside_the_lock() {
    use crate::app_actions::Lease;
    for (id, _, tier) in ALL {
        let action = find(id).unwrap();
        if *tier == Tier::Confirm {
            assert!(action.card.is_some(), "{id} needs its own card");
        }
    }
    // A model call (or app-data state) never holds History's lock.
    for id in [
        "workspaces.set_runs_on",
        "workspaces.export_result_markdown",
        "value.set_baseline",
        "vault.answer_question",
    ] {
        assert_eq!(find(id).unwrap().lease, Lease::Outside, "{id}");
    }
    // A model call outside History's lock, its vault writes inside the run.
    for id in [
        "digest.weekly_review_take",
        "sessions.write_today_brief",
        "agenda.recap_meeting",
    ] {
        assert_eq!(find(id).unwrap().lease, Lease::Own, "{id}");
    }
}

#[test]
fn none_of_them_takes_a_secret_and_the_required_params_validate() {
    for (id, _, _) in ALL {
        let action = find(id).unwrap();
        for p in action.params {
            assert!(!matches!(p.kind, Kind::Secret), "{id}.{}", p.name);
            let name = p.name.to_lowercase();
            assert!(
                !["token", "password", "apikey", "secret", "pairing"]
                    .iter()
                    .any(|w| name.contains(w)),
                "{id}.{} looks like a secret",
                p.name
            );
        }
    }
    let good = validate(
        find("workspaces.triage_mark").unwrap(),
        &json!({ "id": "import:a", "action": "dismiss" }),
    );
    assert!(good.is_ok());
    let bad = validate(
        find("workspaces.triage_mark").unwrap(),
        &json!({ "id": "import:a", "action": "delete" }),
    );
    assert!(bad.is_err(), "an unknown triage action is refused");
    assert!(validate(
        find("agenda.recap_meeting").unwrap(),
        &json!({ "meetingId": "m" })
    )
    .is_err());
}

#[test]
fn a_recap_answer_never_carries_the_transcript_or_the_recap_text() {
    let outcome = crate::meeting_recap::RecapOutcome::Recap {
        pack: crate::prep_pack::PrepPack {
            meeting_id: "m1".into(),
            subject: "Ops sync".into(),
            start: "2026-11-09T15:00:00Z".into(),
            path: "workspaces/w/outputs/recaps/ops.md".into(),
            workspace_id: Some("w".into()),
            markdown: "SECRET PAYROLL LINE from the transcript".into(),
            kept: false,
            content_hash: "abc".into(),
            edited: false,
        },
        queued: 2,
        dropped: 1,
    };
    let value = crate::meeting_recap::recap_summary(&outcome);
    let text = value.to_string();
    assert!(!text.contains("PAYROLL"), "{text}");
    assert_eq!(value["path"], "workspaces/w/outputs/recaps/ops.md");
    assert_eq!(value["queued"], 2);
    assert!(value.get("markdown").is_none());
}

struct Fake {
    reply: String,
    calls: Cell<usize>,
}

impl crate::today_brief::Writer for Fake {
    async fn complete(&self, _prompt: &str) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.reply.clone())
    }
}

impl crate::weekly_review::Writer for Fake {
    async fn complete(&self, _prompt: &str) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        Ok(self.reply.clone())
    }
}

const REPLY: &str = r#"{"paragraph":"A calm day.","recommendations":[{"title":"Finish the plan","why":"due today","ask":"Help me finish the plan"}]}"#;

#[test]
fn the_brief_save_goes_through_the_commit_and_a_cached_brief_skips_it() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let writer = Fake {
        reply: REPLY.into(),
        calls: Cell::new(0),
    };
    let commits = RefCell::new(0);
    let first = tauri::async_runtime::block_on(crate::today_brief::brief_in(
        v,
        "2026-11-10",
        "facts",
        false,
        &writer,
        |keep| {
            *commits.borrow_mut() += 1;
            keep()
        },
    ))
    .unwrap();
    assert_eq!(*commits.borrow(), 1);
    assert_eq!(writer.calls.get(), 1);
    assert!(v.join(&first.path).is_file());
    let again = tauri::async_runtime::block_on(crate::today_brief::brief_in(
        v,
        "2026-11-10",
        "other",
        false,
        &writer,
        |keep| {
            *commits.borrow_mut() += 1;
            keep()
        },
    ))
    .unwrap();
    assert_eq!(again, first);
    assert_eq!(*commits.borrow(), 1, "a cached brief writes nothing");
    assert_eq!(writer.calls.get(), 1, "and calls no model");
    // A refused commit (the run ended) saves nothing.
    let refused = tauri::async_runtime::block_on(crate::today_brief::brief_in(
        v,
        "2026-11-11",
        "facts",
        false,
        &writer,
        |_| Err("run ended".into()),
    ));
    assert_eq!(refused.unwrap_err(), "run ended");
    assert!(crate::today_brief::cached(v, "2026-11-11")
        .unwrap()
        .brief
        .is_none());
}

#[test]
fn the_take_save_goes_through_the_commit_and_a_cached_take_skips_it() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    crate::workspaces::main_agent_workspace(v).unwrap();
    let writer = Fake {
        reply: REPLY.into(),
        calls: Cell::new(0),
    };
    let commits = Cell::new(0);
    let first = tauri::async_runtime::block_on(crate::weekly_review::take_in(
        v,
        "2026-W45",
        "facts",
        false,
        &writer,
        |keep| {
            commits.set(commits.get() + 1);
            keep()
        },
    ))
    .unwrap();
    assert_eq!(first.priorities[0].ask, "Help me finish the plan");
    let again = tauri::async_runtime::block_on(crate::weekly_review::take_in(
        v,
        "2026-W45",
        "facts",
        false,
        &writer,
        |keep| {
            commits.set(commits.get() + 1);
            keep()
        },
    ))
    .unwrap();
    assert_eq!(again, first);
    assert_eq!((commits.get(), writer.calls.get()), (1, 1));
    let regenerated = tauri::async_runtime::block_on(crate::weekly_review::take_in(
        v,
        "2026-W45",
        "facts",
        true,
        &writer,
        |keep| {
            commits.set(commits.get() + 1);
            keep()
        },
    ))
    .unwrap();
    assert_eq!(regenerated.week, "2026-W45");
    assert_eq!((commits.get(), writer.calls.get()), (2, 2));
}

#[test]
fn standing_orders_are_created_once_and_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    // Without the COO's workspace there is nothing to put them in.
    assert!(crate::commands::workspaces_api::create_standing_orders_noted(v).is_err());
    crate::workspaces::main_agent_workspace(v).unwrap();
    let path = v
        .join("workspaces")
        .join(crate::workspaces::MAIN_AGENT_ID)
        .join(crate::workspaces::STANDING_ORDERS);
    // The workspace is seeded with them; once the person deletes them, the
    // action puts the default back.
    std::fs::remove_file(&path).unwrap();
    assert!(crate::commands::workspaces_api::create_standing_orders_noted(v).unwrap());
    std::fs::write(&path, "the person's own orders").unwrap();
    assert!(!crate::commands::workspaces_api::create_standing_orders_noted(v).unwrap());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "the person's own orders"
    );
}

#[test]
fn an_export_never_replaces_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let target = dir.path().join("mine.md");
    std::fs::write(&target, "keep me").unwrap();
    let err = crate::result_export::export_new(&vault, "w1", "workspaces/w1/outputs/a.md", &target)
        .unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
    // A name that is free goes on to the normal checks (this Result isn't there).
    let free = dir.path().join("new.md");
    let err = crate::result_export::export_new(&vault, "w1", "workspaces/w1/outputs/a.md", &free)
        .unwrap_err();
    assert!(!err.contains("already exists"), "{err}");
    assert!(!free.exists());
}

#[test]
fn a_baseline_result_holds_only_the_persons_numbers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(crate::value::FILE);
    let input = crate::value::BaselineInput {
        work_kind: Some("data".into()),
        minutes: Some(90.0),
        note: Some("by hand".into()),
        ..Default::default()
    };
    let stored = crate::value::set_at(&path, &input, chrono::Utc::now()).unwrap();
    let text = serde_json::to_string(&stored).unwrap().to_lowercase();
    assert!(text.contains("\"minutes\":90"), "{text}");
    for word in ["token", "password", "apikey", "secret"] {
        assert!(!text.contains(word), "{word}: {text}");
    }
    // Both a work kind and a run id is refused.
    let both = crate::value::BaselineInput {
        work_kind: Some("data".into()),
        run_id: Some("r1".into()),
        minutes: Some(5.0),
        ..Default::default()
    };
    assert!(crate::value::set_at(&path, &both, chrono::Utc::now()).is_err());
}

#[test]
fn the_triage_state_round_trips_through_the_action_inner() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let now = chrono::Utc::now();
    let s = crate::triage::mark_inner(v, "import:a", "dismiss", None, None, now).unwrap();
    assert!(s.dismissed.contains_key("import:a"));
    let s = crate::triage::mark_inner(v, "import:a", "restore", None, None, now).unwrap();
    assert!(s.dismissed.is_empty() && s.filed.is_empty());
    assert!(crate::triage::mark_inner(v, "import:a", "file", None, None, now).is_err());
}

/// L13 review: the COO's export publishes exclusively. A file that appears
/// after the early existence check (here: made just before the publish) is
/// never replaced, and the finished temp file is cleaned up; a free name
/// gets the file. The person's publish still replaces (their dialog asked).
#[test]
fn an_exclusive_publish_refuses_a_file_that_appeared_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("late.md");
    let tmp = dir.path().join(".hollow-result-1.partial");
    std::fs::write(&tmp, "the export").unwrap();
    std::fs::write(&target, "made meanwhile").unwrap();
    let err = crate::result_export::publish(&tmp, &target, true).unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "made meanwhile");
    assert!(!tmp.exists(), "the temp file is removed");
    let free = dir.path().join("free.md");
    std::fs::write(&tmp, "the export").unwrap();
    crate::result_export::publish(&tmp, &free, true).unwrap();
    assert_eq!(std::fs::read_to_string(&free).unwrap(), "the export");
    assert!(!tmp.exists());
    std::fs::write(&tmp, "replacement").unwrap();
    crate::result_export::publish(&tmp, &target, false).unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "replacement");
}
