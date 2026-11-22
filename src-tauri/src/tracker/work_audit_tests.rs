//! X2: every subagent start and end and every agent work tracker write
//! leaves its audit rows; the Outcome's numbers are the run's audit rows;
//! nothing outside team ENG is written, and each refusal is recorded.

use super::*;
use crate::audit::{self, kind, AuditEntry, AuditFilter};
use crate::session_store;

fn rows(f: &Fixture, kind: &str) -> Vec<AuditEntry> {
    audit::query(
        f.root(),
        &AuditFilter {
            kind: Some(kind.into()),
            limit: Some(audit::MAX_LIMIT),
            ..AuditFilter::default()
        },
    )
    .unwrap()
}

fn of(f: &Fixture, session: &str, kind: &str) -> Vec<AuditEntry> {
    rows(f, kind)
        .into_iter()
        .filter(|e| e.session_key.as_deref() == Some(session))
        .collect()
}

fn turn(v: &Path, key: &str, run: &str, start: &str, end: &str) {
    let ord = session_store::start_turn(v, key, "claude", "/vault/p", run, "Do the part").unwrap();
    session_store::with_write(v, "test turn", |tx| {
        tx.execute(
            "UPDATE turns SET started_at = ?3, ended_at = ?4, status = 'success' WHERE session_key = ?1 AND ord = ?2",
            rusqlite::params![key, ord, start, end],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

fn usage(v: &Path, scope: &str, run: &str, n: u32, tokens: (i64, i64), cost: Option<f64>) {
    session_store::with_write(v, "test usage", |tx| {
        tx.execute(
            "INSERT INTO usage_ledger(run_key, at, started_at, surface, scope, run_id, agent, model, \
             billing, outcome, spawned, basis, input_tokens, output_tokens, cost_usd, cost_status, \
             account_ref) VALUES(?1, ?2, ?2, 'chat', ?3, ?4, 'claude', 'claude-sonnet-5-5', \
             'subscription', 'success', 1, 'per_run', ?5, ?6, ?7, 'known', 'acct-0a1b2c3d4e5f')",
            rusqlite::params![
                format!("key-{run}-{n}"),
                "2026-11-10T12:00:00+00:00",
                scope,
                run,
                tokens.0,
                tokens.1,
                cost
            ],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

#[tokio::test]
async fn every_subagent_start_and_end_leaves_one_record_and_the_outcome_sums_them() {
    let f = fixture(
        true,
        &[("i1", "ENG-1", "Fix the Portal export totals", "state-todo")],
    )
    .await;
    let key = "s-20261110-aud00001";
    let mut facts = f.facts("Fix the Portal export totals");
    facts.agent = Some("claude".into());
    facts.effort = Some("high".into());
    f.start(key, facts);
    let started = of(&f, key, kind::RUN_STARTED);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].asked_by.as_deref(), Some("s-project-proj"));
    assert_eq!(started[0].parent_key.as_deref(), Some("s-project-proj"));
    assert_eq!(started[0].root_key.as_deref(), Some("s-project-proj"));
    assert_eq!(started[0].effort.as_deref(), Some("high"));
    let linked = of(&f, key, kind::ISSUE_LINKED);
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].issue_key.as_deref(), Some("ENG-1"));
    assert_eq!(linked[0].detail.as_deref(), Some("linked by matched"));

    turn(
        f.root(),
        key,
        "run-a1",
        "2026-11-10T12:00:00+00:00",
        "2026-11-10T12:01:00+00:00",
    );
    usage(f.root(), key, "run-a1", 1, (1000, 200), Some(0.25));
    usage(f.root(), key, "run-a1", 2, (500, 100), None);
    work::after_turn(f.root(), key, "success", None, "All set.\n\nDone");
    let ended = of(&f, key, kind::RUN_ENDED);
    assert_eq!(ended.len(), 1);
    let row = &ended[0];
    assert_eq!(
        (row.tokens_in, row.tokens_out, row.working_ms, row.cost_usd),
        (Some(1500), Some(300), Some(60_000), Some(0.25))
    );
    assert_eq!(row.account.as_deref(), Some("acct-0a1b2c3d4e5f"));
    assert_eq!(row.outcome.as_deref(), Some("ok"));
    assert_eq!(row.issue_id.as_deref(), Some("i1"));
    assert_eq!(row.fields.as_ref().unwrap()["turns"], 1);
    assert_eq!(row.fields.as_ref().unwrap()["costPartial"], true);
    // The Outcome the tracker gets carries exactly the audit's numbers.
    let outcome = work::pending_events(f.root(), 10)
        .unwrap()
        .into_iter()
        .find_map(|e| match e.event {
            work::WorkEvent::Outcome {
                working_ms,
                tokens,
                cost_usd,
                cost_partial,
                ..
            } => Some((working_ms, tokens, cost_usd, cost_partial)),
            _ => None,
        })
        .unwrap();
    assert_eq!(outcome, (60_000, 1800, Some(0.25), true));
    // A follow-up turn's end: one more row.
    work::after_turn(f.root(), key, "cancelled", None, "");
    let ended = of(&f, key, kind::RUN_ENDED);
    assert_eq!(ended.len(), 2);
    assert_eq!(ended[0].outcome.as_deref(), Some("stopped"));
    // The first turn was the start's; the follow-up nobody sent is the person's.
    assert_eq!(ended[1].asked_by.as_deref(), Some("s-project-proj"));
    assert_eq!(ended[0].asked_by.as_deref(), Some("person"));

    // An untracked subagent (no link) still leaves both rows; a person's
    // own chat leaves none.
    let loose = "s-20261110-aud00002";
    let mut facts = f.facts("Tidy my notes");
    facts.workspace_id = None;
    facts.cwd = f.root().to_string_lossy().into_owned();
    f.start(loose, facts);
    assert!(work::link_for(f.root(), loose).is_none());
    assert_eq!(of(&f, loose, kind::RUN_STARTED).len(), 1);
    assert!(of(&f, loose, kind::ISSUE_LINKED).is_empty());
    turn(
        f.root(),
        loose,
        "run-b1",
        "2026-11-10T12:00:00+00:00",
        "2026-11-10T12:00:30+00:00",
    );
    session_store::set_parent(
        f.root(),
        loose,
        "s-project-proj",
        session_store::FORK_SUBAGENT,
    )
    .unwrap();
    work::after_turn(f.root(), loose, "error", None, "");
    let ended = of(&f, loose, kind::RUN_ENDED);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].outcome.as_deref(), Some("failed"));
    assert_eq!(ended[0].working_ms, Some(30_000));
    let own = "s-20261110-person01";
    turn(
        f.root(),
        own,
        "run-c1",
        "2026-11-10T12:00:00+00:00",
        "2026-11-10T12:00:10+00:00",
    );
    work::after_turn(f.root(), own, "success", None, "");
    assert!(of(&f, own, kind::RUN_ENDED).is_empty());
    assert!(audit::verify(f.root()).unwrap().ok);
}

#[tokio::test]
async fn every_agent_work_write_leaves_one_record_matching_the_requests() {
    let mut f = fixture(
        true,
        &[("i1", "ENG-1", "Fix the Portal export totals", "state-todo")],
    )
    .await;
    let key = "s-20261110-aud00003";
    f.start(key, f.facts("Fix the Portal export totals"));
    f.pass().await;
    let sent = f.sent("IssueUpdate") + f.sent("CommentCreate");
    assert_eq!(sent, 2, "the move and the Started comment");
    let writes = rows(&f, kind::TRACKER_WRITE);
    assert_eq!(writes.len(), sent);
    let mut actions: Vec<&str> = writes.iter().filter_map(|e| e.action.as_deref()).collect();
    actions.sort_unstable();
    assert_eq!(actions, ["comment", "update"]);
    assert!(writes.iter().all(|e| e.asked_by.as_deref() == Some(key)
        && e.issue_key.as_deref() == Some("ENG-1")
        && e.workspace_id.as_deref() == Some("proj")));
    let moved = writes
        .iter()
        .find(|e| e.action.as_deref() == Some("update"))
        .unwrap();
    let fields = moved.fields.as_ref().unwrap();
    assert_eq!(fields["before"]["state"], "state-todo");
    assert_eq!(fields["after"]["state"], "state-progress");
    // A pass with nothing to send adds nothing.
    f.pass().await;
    assert_eq!(rows(&f, kind::TRACKER_WRITE).len(), sent);

    // A create: its row, then the run's link to the new issue.
    let mut f = fixture(true, &[]).await;
    let key = "s-20261110-aud00004";
    f.start(key, f.facts("Draft the Q4 parking report"));
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 1);
    let created: Vec<AuditEntry> = rows(&f, kind::TRACKER_WRITE)
        .into_iter()
        .filter(|e| e.action.as_deref() == Some("create"))
        .collect();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].issue_key.as_deref(), Some("ENG-50"));
    let linked = of(&f, key, kind::ISSUE_LINKED);
    assert_eq!(linked.len(), 1);
    let made = f.state.creates.values().next().unwrap().issue_id.clone();
    assert_eq!(linked[0].issue_id, made);
    assert!(rows(&f, kind::TRACKER_WRITE_FAILED).is_empty());
}

#[tokio::test]
async fn nothing_outside_team_ai_is_written_and_each_refusal_is_recorded() {
    let mut f = fixture(
        true,
        &[("i1", "ENG-1", "Fix the Portal export totals", "state-todo")],
    )
    .await;
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    // A start naming another team's issue is refused before anything goes.
    let mut facts = f.facts("Fix the LP gate");
    facts.packet_issue = Some("LP-7".into());
    assert!(work::prepare(&f.config, facts).is_err());
    let failed = rows(&f, kind::TRACKER_WRITE_FAILED);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].action.as_deref(), Some("link"));
    assert_eq!(failed[0].issue_key.as_deref(), Some("LP-7"));
    // A fact naming an issue outside the mirror (another team's) is dropped.
    let key = "s-20261110-aud00005";
    work::insert_link(
        f.root(),
        &work::WorkLink {
            session_key: key.into(),
            issue_id: Some("i-lp-0001".into()),
            issue_key: Some("LP-1".into()),
            workspace_id: Some("proj".into()),
            linked_by: linked_by::PACKET.into(),
            created_issue: false,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
    )
    .unwrap();
    work::add_event(
        f.root(),
        key,
        &work::WorkEvent::Started {
            title: "Fix the LP gate".into(),
            asked_by: "person".into(),
            agent: "claude".into(),
            model: "default".into(),
            effort: "default".into(),
        },
    )
    .unwrap();
    f.pass().await;
    assert_eq!(f.sent("IssueUpdate") + f.sent("CommentCreate"), 0);
    let failed = rows(&f, kind::TRACKER_WRITE_FAILED);
    assert_eq!(failed.len(), 2);
    assert_eq!(failed[0].issue_id.as_deref(), Some("i-lp-0001"));
    // A create into a project outside the team (here: gone) is refused.
    let mut f = fixture(true, &[]).await;
    let key = "s-20261110-aud00006";
    f.start(key, f.facts("Draft the Q4 parking report"));
    f.state.records.get_mut("p1").unwrap().remote_missing = true;
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 0);
    let failed = rows(&f, kind::TRACKER_WRITE_FAILED);
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].action.as_deref(), Some("create"));
    assert!(rows(&f, kind::TRACKER_WRITE).is_empty());
}

/// Review fix 5: a child task sent to a conversation is a run start asked
/// by its sender; its turn is the sender's, a later typed one the person's.
#[tokio::test]
async fn a_sent_child_task_is_a_run_start_and_a_typed_turn_is_the_persons() {
    let f = fixture(false, &[]).await;
    let key = "s-20261110-aud00007";
    turn(
        f.root(),
        key,
        "run-d1",
        "2026-11-10T12:00:00+00:00",
        "2026-11-10T12:00:05+00:00",
    );
    session_store::set_parent(
        f.root(),
        key,
        "s-project-proj",
        session_store::FORK_SUBAGENT,
    )
    .unwrap();
    work::record_sent(
        f.root(),
        key,
        "s-20261110-lead0001",
        (Some("claude".into()), None, Some("low".into())),
        Some("proj".into()),
    );
    let started = of(&f, key, kind::RUN_STARTED);
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].asked_by.as_deref(), Some("s-20261110-lead0001"));
    assert_eq!(started[0].parent_key.as_deref(), Some("s-project-proj"));
    work::after_turn(f.root(), key, "success", None, "");
    work::after_turn(f.root(), key, "success", None, "");
    let ended = of(&f, key, kind::RUN_ENDED);
    assert_eq!(ended.len(), 2);
    assert_eq!(ended[1].asked_by.as_deref(), Some("s-20261110-lead0001"));
    assert_eq!(ended[0].asked_by.as_deref(), Some("person"));
}
