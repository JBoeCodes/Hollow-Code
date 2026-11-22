//! X1: how a run gets its issue, and what is recorded at its start.

use super::*;
use serde_json::json;

const TEAM: &str = crate::tracker::linear::TEAM_ID;

pub(super) fn issue(id: &str, key: &str, title: &str, project: &str, kind: &str) -> TrackerIssue {
    serde_json::from_value(json!({
        "id": id, "key": key, "url": format!("https://tracker.example/{key}"), "title": title,
        "state": {"id": format!("state-{kind}"), "name": kind, "kind": kind},
        "projectId": project, "teamId": TEAM,
        "updatedAt": "2026-11-10T10:00:00.000Z", "createdAt": "2026-11-01T10:00:00.000Z"
    }))
    .unwrap()
}

fn mirror() -> Mirror {
    Mirror {
        issues: vec![
            issue(
                "i-packet",
                "ENG-1",
                "Something else entirely",
                "p1",
                "unstarted",
            ),
            issue(
                "i-task",
                "ENG-2",
                "Write the export runbook",
                "p1",
                "backlog",
            ),
            issue(
                "i-match",
                "ENG-3",
                "Fix the Portal export totals",
                "p1",
                "started",
            ),
            issue(
                "i-closed",
                "ENG-4",
                "Fix the Portal export totals again",
                "p1",
                "completed",
            ),
        ],
    }
}

fn facts(title: &str) -> StartFacts {
    StartFacts {
        title: title.into(),
        prompt: format!("{title}\n\nDo it."),
        workspace_id: Some("proj".into()),
        cwd: "/vault/workspaces/proj".into(),
        parent_key: Some("s-project-proj".into()),
        depth: 1,
        asked_by: "s-project-proj".into(),
        ..StartFacts::default()
    }
}

fn parent() -> WorkLink {
    WorkLink {
        session_key: "s-20261110-parent01".into(),
        issue_id: Some("i-parent".into()),
        issue_key: Some("ENG-9".into()),
        workspace_id: Some("proj".into()),
        linked_by: linked_by::MATCHED.into(),
        created_issue: false,
        at: "2026-11-10T10:00:00.000Z".into(),
        depth: 1,
        tracked: true,
        outcome_at: None,
        title: None,
    }
}

fn linked(resolution: &Resolution) -> (Option<&str>, &str) {
    match resolution {
        Resolution::Link {
            issue_id,
            linked_by,
            ..
        } => (issue_id.as_deref(), linked_by),
        Resolution::Create { .. } => (None, "create"),
        Resolution::None => (None, "none"),
    }
}

/// X0 section 3: packet, parent, task, title match, create, in that order;
/// a helper only inherits; a tie or a closed issue never matches.
#[test]
fn each_step_picks_its_issue_and_the_order_holds() {
    let m = mirror();
    let tracked = Place::Tracked("p1".into());
    let tasks = vec!["i-task".to_string()];
    let run = |facts: &StartFacts, parent: Option<&WorkLink>, tasks: &[String]| {
        resolve(Some(&m), true, &tracked, facts, parent, tasks, true).unwrap()
    };
    let mut f = facts("Fix the Portal export totals");
    f.packet_issue = Some("eng-1".into());
    let p = parent();
    assert_eq!(
        linked(&run(&f, Some(&p), &tasks)),
        (Some("i-packet"), "packet")
    );
    f.packet_issue = None;
    assert_eq!(
        linked(&run(&f, Some(&p), &tasks)),
        (Some("i-parent"), "inherited")
    );
    assert_eq!(linked(&run(&f, None, &tasks)), (Some("i-task"), "task"));
    assert_eq!(linked(&run(&f, None, &[])), (Some("i-match"), "matched"));
    // "Totals export" scores 0.4 against ENG-3: below the bar, so create.
    let other = facts("Totals export for the board");
    let created = run(&other, None, &[]);
    assert_eq!(linked(&created).1, "create");
    let Resolution::Create { project_id, .. } = created else {
        unreachable!()
    };
    assert_eq!(project_id, "p1");

    // A tie links nothing: two open issues score the same.
    let mut tie = mirror();
    tie.issues.push(issue(
        "i-twin",
        "ENG-5",
        "Fix the Portal export totals",
        "p1",
        "unstarted",
    ));
    let r = resolve(Some(&tie), true, &tracked, &f, None, &[], true).unwrap();
    assert_eq!(linked(&r).1, "create");
    // A closed issue is never matched (ENG-4 alone would score 1.0).
    let mut closed = mirror();
    closed.issues.retain(|i| i.id == "i-closed");
    let again = facts("Fix the Portal export totals again");
    let r = resolve(Some(&closed), true, &tracked, &again, None, &[], true).unwrap();
    assert_eq!(linked(&r).1, "create");
}

#[test]
fn a_helper_inherits_and_never_creates() {
    let m = mirror();
    let tracked = Place::Tracked("p1".into());
    let mut helper = facts("Fix the Portal export totals");
    helper.depth = 2;
    let p = parent();
    let r = resolve(Some(&m), true, &tracked, &helper, Some(&p), &[], true).unwrap();
    assert_eq!(linked(&r), (Some("i-parent"), "inherited"));
    // No parent link: nothing, not even the title match or a create.
    let r = resolve(
        Some(&m),
        true,
        &tracked,
        &helper,
        None,
        &["i-task".into()],
        true,
    )
    .unwrap();
    assert_eq!(r, Resolution::None);
}

#[test]
fn untracked_quick_chats_automations_and_the_switch_never_create() {
    let m = mirror();
    let f = facts("A brand new piece of work");
    for place in [Place::Untracked, Place::QuickChats] {
        assert_eq!(
            resolve(Some(&m), true, &place, &f, None, &[], true).unwrap(),
            Resolution::None
        );
    }
    let tracked = Place::Tracked("p1".into());
    let mut auto = f.clone();
    auto.automation = true;
    assert_eq!(
        resolve(Some(&m), true, &tracked, &auto, None, &[], true).unwrap(),
        Resolution::None
    );
    // An automation still links to an existing issue.
    auto.title = "Fix the Portal export totals".into();
    let r = resolve(Some(&m), true, &tracked, &auto, None, &[], true).unwrap();
    assert_eq!(linked(&r), (Some("i-match"), "matched"));
    // The switch off: links still resolve, but nothing is created.
    assert_eq!(
        resolve(Some(&m), true, &tracked, &f, None, &[], false).unwrap(),
        Resolution::None
    );
    // quick-chats takes a named issue; an untracked workspace refuses one.
    let mut named = f.clone();
    named.packet_issue = Some("ENG-2".into());
    let r = resolve(Some(&m), true, &Place::QuickChats, &named, None, &[], true).unwrap();
    assert_eq!(linked(&r), (Some("i-task"), "packet"));
    let refused = resolve(Some(&m), true, &Place::Untracked, &named, None, &[], true).unwrap_err();
    assert!(refused.contains("isn't a tracked project") && refused.contains("Nothing started"));
}

/// A named issue outside team ENG is refused before anything starts or is
/// sent: another team's node never reaches the mirror, and a tracker scoped
/// to another team refuses every named issue.
#[test]
fn a_named_issue_outside_team_ai_refuses_the_start() {
    let mut state = crate::tracker_sync::Ledger::new("linear", TEAM);
    for (id, key, team) in [("i-ai", "ENG-7", TEAM), ("i-lp", "LP-7", "team-lp-0001")] {
        let mut i = issue(id, key, "Fixture", "p1", "unstarted");
        i.team_id = team.into();
        state.records.insert(
            id.into(),
            serde_json::from_value(json!({
                "id": id, "key": key, "kind": "task", "path": format!("tasks/tracker-{id}.md"),
                "workspaceId": null, "base": {}, "issue": i, "project": null
            }))
            .unwrap(),
        );
    }
    let m = Mirror::of(&state);
    assert_eq!(m.issues.len(), 1, "the scope filter drops the other team");
    let tracked = Place::Tracked("p1".into());
    let mut f = facts("Anything");
    f.packet_issue = Some("LP-7".into());
    let error = resolve(Some(&m), true, &tracked, &f, None, &[], true).unwrap_err();
    assert!(error.contains("isn't in team ENG's mirror"), "{error}");
    f.packet_issue = Some("ENG-7".into());
    assert!(resolve(Some(&m), true, &tracked, &f, None, &[], true).is_ok());
    let error = resolve(Some(&m), false, &tracked, &f, None, &[], true).unwrap_err();
    assert!(error.contains("isn't set up for team ENG"), "{error}");
}

#[test]
fn a_retried_start_has_the_same_create_key() {
    let a = facts("Draft the Q4 parking report");
    let mut b = a.clone();
    b.prompt = "Draft the Q4 parking report\n\nPlease, again.".into();
    b.title = "draft the q4  parking report".into();
    assert_eq!(create_key(&a, "p1"), create_key(&b, "p1"));
    assert!(create_key(&a, "p1").starts_with("work-") && create_key(&a, "p1").len() == 37);
    let mut other = a.clone();
    other.parent_key = Some("s-project-other".into());
    assert_ne!(create_key(&a, "p1"), create_key(&other, "p1"));
}

#[test]
fn comment_bodies_are_fixed_capped_redacted_and_carry_their_record_id() {
    let id = "work-0123456789abcdef0123456789abcdef";
    let started = WorkEvent::Started {
        title: "Ship it with ghp_abcdefghijklmnopqrstuvwxyz0123".into(),
        asked_by: "s-project-proj".into(),
        agent: "claude".into(),
        model: "claude-sonnet-5-5".into(),
        effort: "high".into(),
    };
    let body = render_started(&started, "s-20261110-child001", id);
    assert!(body.starts_with("Hollow: agent work started\n"));
    assert!(body.ends_with(&format!("Session: s-20261110-child001\nHollow record {id}")));
    assert!(!body.contains("ghp_abcdefghijklmnop"), "{body}");
    let outcome = WorkEvent::Outcome {
        status: "success".into(),
        result: Some("Done".into()),
        prs: pull_requests("Opened https://github.com/x/y/pull/1258 and PR #1260; see #7."),
        results: vec!["workspaces/proj/outputs/report.md".into(); 3],
        checks: vec!["test passed".into()],
        files_changed: 0,
        working_ms: 61 * 60_000,
        tokens: 1234,
        cost_usd: Some(1.5),
        cost_partial: true,
        reason: None,
    };
    let head = LogHead {
        at: "2026-11-15 10:00 (UTC+00:00)".into(),
        who: "Fix it (subagent)".into(),
        why: "work on ENG-1, Fix it".into(),
    };
    let body = render_outcome(&outcome, &head, "s-1", id);
    assert!(body.contains("Cost: at least $1.50"), "{body}");
    assert!(body.contains(" Pull requests 1258, 1260."), "{body}");
    assert!(!body.contains('#'));
    assert!(body.contains("What: Finished (Done).") && body.contains("1 h 1 min"));
    let long = vec![
        (
            "x".repeat(280),
            Some("https://e.example/".repeat(30)),
            vec![]
        );
        12
    ];
    let body = render_progress(&long, &head, "s-1", id);
    assert!(body.len() <= COMMENT_MAX_BYTES && body.ends_with(id));
    assert!(
        body.contains("\nWhy: work on ENG-1, Fix it\nChecked: no check recorded\n"),
        "Why and Checked survive a full comment: {body}"
    );
    assert_eq!(
        result_word("Work.\n\n**Partial: tests left**\n"),
        Some("Partial".into())
    );
    assert_eq!(result_word("Doneish"), None);
}

/// The link is recorded at start whatever the switch says; the tracker's
/// facts only when it is on, and never for an inherited (helper) link.
#[test]
fn begin_records_the_link_always_and_facts_only_when_tracking() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let link = |by: &'static str| Resolution::Link {
        issue_id: Some("i-1".into()),
        issue_key: Some("ENG-1".into()),
        linked_by: by,
    };
    let start_at = |key: &str, resolution: Resolution, track: bool, depth: u32| {
        let mut f = facts("Fix it");
        f.depth = depth;
        begin(
            v,
            key,
            Prepared {
                facts: f,
                resolution,
                track,
            },
        );
    };
    let start =
        |key: &str, resolution: Resolution, track: bool| start_at(key, resolution, track, 1);
    start("s-20261110-off00001", link(linked_by::MATCHED), false);
    start("s-20261110-on000001", link(linked_by::MATCHED), true);
    // A helper (depth 2) records its link and never posts; a depth-1 lane
    // that inherited its orchestrator's issue does post.
    start_at("s-20261110-helper01", link(linked_by::INHERITED), true, 2);
    start("s-20261110-lane0001", link(linked_by::INHERITED), true);
    start("s-20261110-none0001", Resolution::None, true);
    start(
        "s-20261110-create01",
        Resolution::Create {
            project_id: "p1".into(),
            create_key: "work-k".into(),
        },
        true,
    );
    let off = link_for(v, "s-20261110-off00001").unwrap();
    assert_eq!(
        (off.issue_key.as_deref(), off.linked_by.as_str()),
        (Some("ENG-1"), "matched")
    );
    assert!(link_for(v, "s-20261110-helper01").is_some());
    assert!(link_for(v, "s-20261110-none0001").is_none());
    let created = link_for(v, "s-20261110-create01").unwrap();
    assert!(created.created_issue && created.issue_id.is_none());
    let events = pending_events(v, 50).unwrap();
    let kinds: Vec<(&str, &str)> = events
        .iter()
        .map(|e| (e.session_key.as_str(), e.event.kind()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("s-20261110-on000001", "started"),
            ("s-20261110-lane0001", "started"),
            ("s-20261110-create01", "create"),
            ("s-20261110-create01", "started"),
        ]
    );
    // A second record of the same start changes nothing.
    start("s-20261110-off00001", link(linked_by::PACKET), false);
    assert_eq!(
        link_for(v, "s-20261110-off00001").unwrap().linked_by,
        "matched"
    );
    fill_created(v, "s-20261110-create01", "i-new", "ENG-50").unwrap();
    assert_eq!(links_for_issue(v, "i-new").len(), 1);
}

/// The switch is read from this process's running config: with it off a
/// linked run's turn end queues nothing; on, the first turn end is the
/// Outcome and a later one a progress note. A handover's successor keeps
/// its predecessor's link (and its Outcome already posted).
#[test]
fn turn_ends_report_only_while_tracking_and_a_handover_keeps_the_link() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    let key = "s-20261110-turn0001";
    let link = Resolution::Link {
        issue_id: Some("i-1".into()),
        issue_key: Some("ENG-1".into()),
        linked_by: linked_by::MATCHED,
    };
    begin(
        v,
        key,
        Prepared {
            facts: facts("Fix it"),
            resolution: link,
            track: true,
        },
    );
    let kinds = |v: &Path| -> Vec<&'static str> {
        pending_events(v, 50)
            .unwrap()
            .iter()
            .map(|e| e.event.kind())
            .collect()
    };
    set_tracking(v, false);
    after_turn(v, key, "success", None, "Done");
    assert_eq!(kinds(v), vec!["started"], "off: nothing queued");
    set_tracking(v, true);
    after_turn(v, key, "error", None, "Blocked: no access");
    after_turn(v, key, "success", None, "Opened PR 12.\n\nDone");
    assert_eq!(kinds(v), vec!["started", "outcome", "progress"]);
    let events = pending_events(v, 50).unwrap();
    let WorkEvent::Outcome { reason, .. } = &events[1].event else {
        unreachable!()
    };
    assert_eq!(reason.as_deref(), Some("The turn ended with an error."));
    let WorkEvent::Progress { line, .. } = &events[2].event else {
        unreachable!()
    };
    assert!(
        line.starts_with("A follow-up turn finished (Done); pull requests 12"),
        "{line}"
    );
    continue_link(v, key, "s-20261110-turn0002");
    let next = link_for(v, "s-20261110-turn0002").unwrap();
    assert_eq!(next.issue_id.as_deref(), Some("i-1"));
    assert!(next.tracked && next.outcome_at.is_some());
}

#[test]
fn only_the_ai_team_id_counts_as_team_ai() {
    let config = |team: &str| -> Config {
        serde_json::from_value(json!({"vaultPath": "/v", "tracker": {"provider": "linear",
            "linear": {"teamId": team, "teamKey": "ENG"}}}))
        .unwrap()
    };
    assert!(ai_scope(&config(AI_TEAM_ID)));
    assert!(
        !ai_scope(&config("team-renamed-ai-0001")),
        "a key named AI is not team ENG"
    );
}

/// A sync ledger holding `issues` (id, kind, completed_at) for team `team`.
pub(crate) fn ledger_with(
    team: &str,
    issues: &[(&str, &str, Option<&str>)],
) -> crate::tracker_sync::Ledger {
    let records: serde_json::Map<String, Value> = issues
        .iter()
        .map(|(id, kind, completed)| {
            let mut issue = json!({
                "id": id, "key": format!("AI-{id}"), "url": "https://tracker.example/x", "title": id,
                "state": {"id": format!("state-{kind}"), "name": kind, "kind": kind},
                "teamId": team, "updatedAt": "2026-11-10T12:00:00.000Z", "createdAt": "2026-11-01T10:00:00.000Z"
            });
            if let Some(at) = completed {
                issue["completedAt"] = json!(at);
            }
            (
                id.to_string(),
                json!({"id": id, "key": format!("AI-{id}"), "kind": "issue", "path": format!("tasks/{id}.md"),
                       "workspaceId": null, "base": {}, "issue": issue, "project": null}),
            )
        })
        .collect();
    serde_json::from_value(json!({
        "version": 1, "provider": "linear", "scope": team, "cursorIssues": null, "cursorProjects": null,
        "cursorComments": null, "lastSync": null, "lastError": null, "records": records
    }))
    .unwrap()
}

/// G4: a run counts as accepted through its issue only when the issue is
/// Done now and was completed after the run started.
#[test]
fn a_run_is_accepted_when_its_issue_is_done_after_it_started() {
    let state = ledger_with(
        TEAM,
        &[
            ("after", "completed", Some("2026-11-10T12:00:00.000Z")),
            ("before", "completed", Some("2026-11-10T08:00:00.000Z")),
            // Done once (completedAt kept), then reopened.
            ("reopened", "started", Some("2026-11-10T12:00:00.000Z")),
            // Done, but mirrored before Hollow read completedAt.
            ("unread", "completed", None),
            ("canceled", "canceled", Some("2026-11-10T12:00:00.000Z")),
        ],
    );
    let done = done_issues(&state);
    let mut ids: Vec<&String> = done.keys().collect();
    ids.sort();
    assert_eq!(
        ids,
        ["after", "before", "unread"],
        "reopened and canceled aren't Done"
    );
    let run = |run: &str, issue: &str| LinkedRun {
        run_id: run.into(),
        started_at: "2026-11-10T10:00:00.000Z".into(),
        issue_id: issue.into(),
    };
    let runs = [
        run("r-after", "after"),
        run("r-before", "before"),
        run("r-reopened", "reopened"),
        run("r-unread", "unread"),
        run("r-canceled", "canceled"),
        run("r-unlinked", "nowhere"),
        // A later turn of the same session, started after the issue was done.
        LinkedRun {
            run_id: "r-late".into(),
            started_at: "2026-11-10T13:00:00.000Z".into(),
            issue_id: "after".into(),
        },
    ];
    let accepted = accepted_by_issue_from(&runs, &done);
    assert_eq!(
        accepted.into_iter().collect::<Vec<_>>(),
        vec!["r-after".to_string()]
    );
    // Another team's Done issue never counts.
    let mut foreign = ledger_with(
        "team-other",
        &[("after", "completed", Some("2026-11-10T12:00:00.000Z"))],
    );
    foreign.scope = TEAM.into();
    assert!(done_issues(&foreign).is_empty());
}

/// G4: the link and turn reads join a run to its issue, and each session's
/// runs are found for the issue detail's `accepted`.
#[test]
fn linked_runs_join_turns_to_their_issue() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    for (key, issue) in [("s-1", Some("i-1")), ("s-2", None)] {
        insert_link(
            v,
            &WorkLink {
                session_key: key.into(),
                issue_id: issue.map(Into::into),
                issue_key: issue.map(|_| "ENG-1".into()),
                workspace_id: None,
                linked_by: linked_by::MATCHED.into(),
                created_issue: false,
                at: "2026-11-10T10:00:00.000Z".into(),
                depth: 1,
                tracked: false,
                outcome_at: None,
                title: None,
            },
        )
        .unwrap();
    }
    session_store::with_write(v, "test turns", |tx| {
        for (key, ord, run) in [("s-1", 1, "r1"), ("s-1", 2, "r2"), ("s-2", 1, "r3")] {
            tx.execute(
                "INSERT INTO turns(session_key, ord, run_id, started_at, status) VALUES(?1, ?2, ?3, ?4, 'done')",
                params![key, ord, run, "2026-11-10T10:00:00.000Z"],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .unwrap();
    let mut linked: Vec<String> = linked_runs(v)
        .into_iter()
        .map(|r| format!("{}:{}", r.run_id, r.issue_id))
        .collect();
    linked.sort();
    assert_eq!(
        linked,
        vec!["r1:i-1", "r2:i-1"],
        "a pending create has no issue yet"
    );
    let by_session = runs_of_sessions(v, &["s-1".into(), "s-2".into()]);
    assert_eq!(by_session["s-1"].len(), 2);
    assert_eq!(by_session["s-2"], vec!["r3".to_string()]);
}

/// T3g: a build log comment is dated, names who wrote it, and says what,
/// why and how it was checked, from host facts only; a planted token in any
/// of them is masked in the posted body.
#[test]
fn build_log_comments_are_dated_structured_and_redacted() {
    let id = "work-0123456789abcdef0123456789abcdef";
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123";
    let outcome = WorkEvent::Outcome {
        status: "error".into(),
        result: None,
        prs: vec![12],
        results: vec!["workspaces/proj/outputs/report.md".into()],
        checks: vec![format!("test failed (cargo test --token {secret})")],
        files_changed: 3,
        working_ms: 120_000,
        tokens: 10,
        cost_usd: None,
        cost_partial: false,
        reason: Some(format!("The turn failed: {secret}.")),
    };
    let head = LogHead {
        at: local_stamp("2026-11-15T15:04:00Z"),
        who: format!("Ship {secret} (subagent)"),
        why: format!("work on ENG-7, Ship {secret}"),
    };
    let body = render_outcome(&outcome, &head, "s-20261115-child001", id);
    assert!(!body.contains("ghp_abcdefghijklmnop"), "{body}");
    let mut lines = body.lines();
    let first = lines.next().unwrap();
    let local = chrono::DateTime::parse_from_rfc3339("2026-11-15T15:04:00Z")
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string();
    assert!(
        first.starts_with(&format!("Build log, {local} (UTC")) && first.ends_with("(subagent)"),
        "{first}"
    );
    assert!(body.contains("\nWhat: Failed. The turn failed: "), "{body}");
    assert!(
        body.contains("Pull requests 12. 3 files changed in repositories. Results: workspaces/proj/outputs/report.md."),
        "{body}"
    );
    assert!(body.contains("\nWhy: work on ENG-7, Ship "), "{body}");
    assert!(
        body.contains("\nChecked: test failed (cargo test --token "),
        "{body}"
    );
    // No check recorded says so; progress notes carry their own checks.
    let quiet = WorkEvent::Outcome {
        status: "success".into(),
        result: Some("Done".into()),
        prs: vec![],
        results: vec![],
        checks: vec![],
        files_changed: 0,
        working_ms: 0,
        tokens: 0,
        cost_usd: None,
        cost_partial: false,
        reason: None,
    };
    let body = render_outcome(&quiet, &head, "s-1", id);
    assert!(body.contains("\nChecked: no check recorded\n"), "{body}");
    let notes = vec![
        (
            "A follow-up turn finished (Done)".to_string(),
            None,
            vec!["build passed".to_string()],
        ),
        (
            format!("note {secret}"),
            None,
            vec!["build passed".to_string(), "lint passed".to_string()],
        ),
    ];
    let body = render_progress(&notes, &head, "s-1", id);
    assert!(body.starts_with("Build log, "), "{body}");
    assert!(
        body.contains("\nWhat:\n- A follow-up turn finished (Done)\n- note "),
        "{body}"
    );
    assert!(
        body.contains("\nChecked: build passed, lint passed\n"),
        "{body}"
    );
    assert!(!body.contains("ghp_abcdefghijklmnop"), "{body}");
}
