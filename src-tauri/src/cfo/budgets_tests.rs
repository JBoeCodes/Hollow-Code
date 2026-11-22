//! L9's budget tests: the pure checks, the policy's validation, and a vault
//! fixture where the right subagent is paused and its parent hears why.

use super::*;
use crate::cfo::settings::{normalize, save, CfoSettings, ProjectBudget, SubagentBudget};
use crate::session_store;
use chrono::TimeZone;
use std::path::Path;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn usage(v: &Path, run: &str, at: &str, input: i64, output: i64, workspace: Option<&str>) {
    session_store::with_write(v, "test usage", |tx| {
        tx.execute(
            "INSERT INTO usage_ledger(run_key, at, started_at, surface, scope, run_id, agent, model, \
             billing, outcome, spawned, basis, input_tokens, cached_input_tokens, output_tokens, \
             cost_usd, cost_status, workspace_id) \
             VALUES(?1, ?2, ?2, 'chat', 'x', ?3, 'claude', 'claude-sonnet-5-5', 'subscription', \
             'success', 1, 'per_run', ?4, 0, ?5, NULL, 'unknown', ?6)",
            rusqlite::params![format!("key-{run}"), at, run, input, output, workspace],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

fn turn(v: &Path, key: &str, run: &str, start: &str, end: &str) {
    let ord = session_store::start_turn(v, key, "claude", "/vault/p", run, "Do the part").unwrap();
    session_store::with_write(v, "test turn", |tx| {
        tx.execute(
            "UPDATE turns SET started_at = ?3, ended_at = ?4, status = 'success' \
             WHERE session_key = ?1 AND ord = ?2",
            rusqlite::params![key, ord, start, end],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

#[test]
fn a_subagent_budget_is_reached_on_tokens_or_working_time() {
    let budget = SubagentBudget {
        tokens: Some(1_000_000),
        minutes: Some(30),
    };
    assert_eq!(
        subagent_reached(
            &budget,
            Used {
                tokens: 999_999,
                working_ms: 29 * 60_000
            }
        ),
        None
    );
    let note = subagent_reached(
        &budget,
        Used {
            tokens: 1_200_000,
            working_ms: 0,
        },
    )
    .unwrap();
    assert!(note.starts_with(PAUSE_PREFIX), "{note}");
    assert!(note.contains("1,200,000 tokens of its 1,000,000"), "{note}");
    assert!(note.contains("sessions.subagent_budget"), "{note}");
    let timed = subagent_reached(
        &budget,
        Used {
            tokens: 0,
            working_ms: 31 * 60_000,
        },
    )
    .unwrap();
    assert!(
        timed.contains("31 minutes of working time of its 30"),
        "{timed}"
    );
    assert_eq!(
        subagent_reached(
            &SubagentBudget::default(),
            Used {
                tokens: u64::MAX,
                working_ms: i64::MAX
            }
        ),
        None,
        "no budget caps nothing"
    );
    assert!(!note.contains('\u{2014}') && !note.contains('\u{2013}'));
}

#[test]
fn a_project_budget_is_reached_in_either_rolling_window() {
    let budget = ProjectBudget {
        workspace_id: "proj".into(),
        five_hour_tokens: Some(100),
        week_tokens: Some(1_000),
    };
    assert_eq!(project_reached(&budget, "P", 99, 999), None);
    let five = project_reached(&budget, "P", 100, 100).unwrap();
    assert!(five.contains("in the last 5 hours"), "{five}");
    let week = project_reached(&budget, "P", 10, 1_500).unwrap();
    assert!(
        week.contains("1,500 tokens in the last 7 days, past its budget of 1,000"),
        "{week}"
    );
}

#[test]
fn the_policy_checks_and_tidies_budgets() {
    let ok = normalize(CfoSettings {
        project_budgets: vec![
            ProjectBudget {
                workspace_id: " proj-a ".into(),
                five_hour_tokens: Some(5),
                week_tokens: None,
            },
            ProjectBudget {
                workspace_id: "proj-b".into(),
                five_hour_tokens: None,
                week_tokens: None,
            },
        ],
        subagent_budget: SubagentBudget {
            tokens: Some(10),
            minutes: Some(60),
        },
        ..CfoSettings::default()
    })
    .unwrap();
    assert_eq!(
        ok.project_budgets.len(),
        1,
        "a budget with no figure is dropped"
    );
    assert_eq!(ok.project_budgets[0].workspace_id, "proj-a");
    let bad = |settings: CfoSettings| normalize(settings).unwrap_err();
    assert!(bad(CfoSettings {
        subagent_budget: SubagentBudget {
            tokens: Some(0),
            minutes: None
        },
        ..CfoSettings::default()
    })
    .contains("more than 0"));
    assert!(bad(CfoSettings {
        subagent_budget: SubagentBudget {
            tokens: None,
            minutes: Some(settings::MAX_BUDGET_MINUTES + 1)
        },
        ..CfoSettings::default()
    })
    .contains("a week"));
    let twice = ProjectBudget {
        workspace_id: "p".into(),
        five_hour_tokens: Some(1),
        week_tokens: None,
    };
    assert!(bad(CfoSettings {
        project_budgets: vec![twice.clone(), twice],
        ..CfoSettings::default()
    })
    .contains("two budgets"));
    // An older policy row (no budget fields) still reads, with no caps.
    let old: CfoSettings =
        serde_json::from_str(r#"{"allow":[],"effortMode":"automatic","reservePercent":20}"#)
            .unwrap();
    assert!(old.project_budgets.is_empty() && old.subagent_budget.is_empty());
}

/// The done condition: a budget fixture pauses the right subagent (the one
/// past its budget, not its sibling under it) and its parent hears why.
#[test]
fn the_subagent_past_its_budget_is_paused_and_its_parent_is_told() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let now = at("2026-11-10T13:00:00Z");
    let lead = "s-20261110-lead0001";
    let (over, under) = ("s-20261110-over0001", "s-20261110-undr0001");
    session_store::start_turn(v, lead, "claude", "/vault/p", "run-lead", "Plan").unwrap();
    turn(
        v,
        over,
        "run-over",
        "2026-11-10T12:00:00+00:00",
        "2026-11-10T12:10:00+00:00",
    );
    turn(
        v,
        under,
        "run-under",
        "2026-11-10T12:00:00+00:00",
        "2026-11-10T12:10:00+00:00",
    );
    for key in [over, under] {
        session_store::set_parent(v, key, lead, session_store::FORK_SUBAGENT).unwrap();
    }
    usage(
        v,
        "run-over",
        "2026-11-10T12:10:00+00:00",
        4_000,
        2_000,
        None,
    );
    usage(v, "run-under", "2026-11-10T12:10:00+00:00", 400, 200, None);
    // Every subagent may use 5,000 tokens.
    save(
        v,
        CfoSettings {
            subagent_budget: SubagentBudget {
                tokens: Some(5_000),
                minutes: None,
            },
            ..CfoSettings::default()
        },
    )
    .unwrap();

    assert_eq!(claim(v, under, None, now), None, "under its budget: runs");
    assert_eq!(claim(v, lead, None, now), None, "a lead is no subagent");
    let Some(ClaimBudget::Fail(note)) = claim(v, over, None, now) else {
        panic!("the subagent past its budget fails at claim");
    };
    assert!(note.contains("6,000 tokens of its 5,000"), "{note}");
    assert_eq!(pause_note(v, over).as_deref(), Some(note.as_str()));
    assert_eq!(pause_note(v, under), None, "its sibling is not paused");
    assert!(sent_for_test()
        .iter()
        .any(|title| title.starts_with("Subagent paused: ")));

    // The parent's report on that child carries the pause.
    let id = crate::run_queue::enqueue(
        v,
        crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some(over),
            "Chat",
            "chat",
        ),
        now,
    )
    .unwrap()
    .id();
    let row = crate::run_queue::get(v, id).unwrap().unwrap();
    let summary = crate::descendants::child_summary(v, over, &row);
    assert!(summary.text(400).contains(&note), "{}", summary.text(400));

    // A subagent can't raise its own budget; its parent can, and that
    // clears the pause.
    assert!(
        set_subagent_budget(v, Some(over), over, Some(10_000), None, now)
            .unwrap_err()
            .contains("own budget")
    );
    assert!(
        set_subagent_budget(v, Some(lead), lead, Some(10_000), None, now)
            .unwrap_err()
            .contains("own budget")
    );
    // Review: neither a sibling nor an automation can change it.
    assert!(
        set_subagent_budget(v, Some(under), over, Some(10_000), None, now)
            .unwrap_err()
            .contains("only the conversation that started")
    );
    assert!(set_subagent_budget(v, None, over, Some(10_000), None, now)
        .unwrap_err()
        .contains("only a conversation"));
    let set = set_subagent_budget(v, Some(lead), over, Some(10_000), None, now).unwrap();
    assert_eq!(set["tokens"], 10_000);
    assert_eq!(pause_note(v, over), None);
    assert_eq!(claim(v, over, None, now), None, "raised: runs again");
    assert!(
        set_subagent_budget(v, Some(lead), "s-20261110-nobody01", Some(1), None, now)
            .unwrap_err()
            .contains("not a subagent")
    );

    // Review: a pause also clears when the default is raised in Settings
    // and the subagent's next turn is claimed.
    set_subagent_budget(v, Some(lead), over, Some(1_000), None, now).unwrap();
    assert!(matches!(
        claim(v, over, None, now),
        Some(ClaimBudget::Fail(_))
    ));
    assert!(pause_note(v, over).is_some());
    session_store::with_write(v, "test", |tx| {
        tx.execute("UPDATE cfo_subagent_budgets SET tokens = NULL", [])
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .unwrap();
    save(
        v,
        CfoSettings {
            subagent_budget: SubagentBudget {
                tokens: Some(50_000),
                minutes: None,
            },
            ..CfoSettings::default()
        },
    )
    .unwrap();
    assert_eq!(claim(v, over, None, now), None);
    assert_eq!(
        pause_note(v, over),
        None,
        "a stale pause is cleared when it runs"
    );
}

/// Review: a subagent's grandparent may change its budget; its own
/// subagent may not.
#[test]
fn only_the_subagents_own_line_may_change_its_budget() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let now = at("2026-11-10T13:00:00Z");
    let (lead, mid, leaf) = (
        "s-20261110-leadbbbb",
        "s-20261110-midbbbbb",
        "s-20261110-leafbbbb",
    );
    for key in [lead, mid, leaf] {
        session_store::start_turn(v, key, "claude", "/vault/p", &format!("run-{key}"), "Go")
            .unwrap();
    }
    session_store::set_parent(v, mid, lead, session_store::FORK_SUBAGENT).unwrap();
    session_store::set_parent(v, leaf, mid, session_store::FORK_SUBAGENT).unwrap();
    assert!(set_subagent_budget(v, Some(lead), leaf, Some(5), None, now).is_ok());
    assert!(set_subagent_budget(v, Some(mid), leaf, Some(5), None, now).is_ok());
    assert!(set_subagent_budget(v, Some(leaf), mid, Some(5), None, now)
        .unwrap_err()
        .contains("only the conversation that started"));
}

/// A project past its 5-hour budget holds its unattended turns; usage that
/// rolled out of the window, or another project's, doesn't count.
#[test]
fn a_project_past_its_budget_holds_its_unattended_work() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let folder = v.join("workspaces/proj-a");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("WORKSPACE.md"),
        "---\nhollow_workspace: 1\ntitle: \"Project A\"\nstatus: active\n---\n\n# Project A\n",
    )
    .unwrap();
    let now = Utc.with_ymd_and_hms(2026, 11, 10, 13, 0, 0).unwrap();
    session_store::start_turn(
        v,
        "s-20261110-leadaaaa",
        "claude",
        "/vault/p",
        "run-l",
        "Go",
    )
    .unwrap();
    usage(
        v,
        "old",
        "2026-11-10T07:00:00+00:00",
        9_000,
        0,
        Some("proj-a"),
    );
    usage(
        v,
        "other",
        "2026-11-10T12:30:00+00:00",
        9_000,
        0,
        Some("proj-b"),
    );
    usage(
        v,
        "recent",
        "2026-11-10T12:30:00+00:00",
        600,
        300,
        Some("proj-a"),
    );
    save(
        v,
        CfoSettings {
            project_budgets: vec![ProjectBudget {
                workspace_id: "proj-a".into(),
                five_hour_tokens: Some(1_000),
                week_tokens: Some(20_000),
            }],
            ..CfoSettings::default()
        },
    )
    .unwrap();
    assert_eq!(
        project_windows(v, "proj-a", now).unwrap(),
        (900, 9_900),
        "the 5-hour window leaves out the 07:00 row and proj-b's"
    );
    assert_eq!(claim(v, "s-20261110-leadaaaa", Some(&folder), now), None);
    usage(
        v,
        "more",
        "2026-11-10T12:50:00+00:00",
        100,
        0,
        Some("proj-a"),
    );
    let Some(ClaimBudget::Hold(note)) = claim(v, "s-20261110-leadaaaa", Some(&folder), now) else {
        panic!("past its 5-hour budget: held");
    };
    assert!(note.starts_with(HOLD_PREFIX), "{note}");
    assert!(
        note.contains("Project A") && note.contains("1,000 tokens in the last 5 hours"),
        "{note}"
    );
    // Outside the project: not held.
    assert_eq!(claim(v, "s-20261110-leadaaaa", Some(v), now), None);
}

/// Delta review: a subagent the COO started in its own folder is not the
/// COO: it can't change a sibling's budget, and the COO still can.
#[test]
fn a_subagent_in_the_coos_folder_is_not_the_coo() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let now = at("2026-11-10T13:00:00Z");
    let cwd = crate::workspaces::ensure_main_agent(v).unwrap();
    let cwd = cwd.to_string_lossy().to_string();
    let (coo, one, two) = (
        "s-20261110-coocccc1",
        "s-20261110-onecccc1",
        "s-20261110-twocccc1",
    );
    for key in [coo, one, two] {
        session_store::start_turn(v, key, "claude", &cwd, &format!("run-{key}"), "Go").unwrap();
    }
    for key in [one, two] {
        session_store::set_parent(v, key, coo, session_store::FORK_SUBAGENT).unwrap();
    }
    assert!(crate::chat_chain::is_main_conversation(v, one));
    assert!(set_subagent_budget(v, Some(one), two, Some(5), None, now).is_err());
    assert!(set_subagent_budget(v, Some(coo), two, Some(5), None, now).is_ok());
    // Its usage is its own chain's, not every COO thread's.
    assert_eq!(budget_chain(v, two), vec![two.to_string()]);
}
