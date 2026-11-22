//! X1: the agent work outbox against a stub tracker and a fixture vault.

use super::*;
use crate::tracker::linear::LinearAdapter;
use crate::tracker::stub::{self, StubReply, StubServer};
use crate::tracker::work::{self, linked_by, Prepared, Resolution, StartFacts};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const TEAM: &str = work::AI_TEAM_ID;
const T0: &str = "2026-11-10T10:00:00.000Z";

#[derive(Default)]
struct Remote {
    issues: Vec<Value>,
    comments: Vec<Value>,
    lose_comment_reply: bool,
    low_rate: bool,
    /// Q-R22: the next create is refused before it lands.
    refuse_create: bool,
    /// Every issue read answers 503.
    fail_reads: bool,
}

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
    state: Ledger,
    server: StubServer,
    remote: Arc<Mutex<Remote>>,
}

fn state_node(id: &str) -> Value {
    let (name, kind) = match id {
        "state-backlog" => ("Backlog", "backlog"),
        "state-todo" => ("Todo", "unstarted"),
        "state-progress" => ("In Progress", "started"),
        "state-review" => ("In Review", "started"),
        _ => ("Done", "completed"),
    };
    json!({"id": id, "name": name, "type": kind})
}

fn reply(req: &stub::StubRequest, remote: &mut Remote) -> StubReply {
    if let Some(refused) = stub::unscoped(req, TEAM) {
        return refused;
    }
    let page = |field: &str, nodes: Vec<Value>| {
        StubReply::data(
            json!({field: {"nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}}),
        )
    };
    let out = match req.operation().as_deref() {
        Some("IssueTarget" | "IssueDetail") if remote.fail_reads => StubReply::status(503),
        Some("IssueTarget" | "IssueDetail") => {
            let id = req
                .variables()
                .pointer("/filter/id/eq")
                .cloned()
                .unwrap_or(Value::Null);
            let found: Vec<Value> = remote
                .issues
                .iter()
                .filter(|i| i["id"] == id)
                .cloned()
                .collect();
            page("issues", found)
        }
        Some("IssueUpdate") => {
            let id = req.variables()["id"].clone();
            let target = req.variables()["input"]["stateId"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let issue = remote.issues.iter_mut().find(|i| i["id"] == id).unwrap();
            issue["state"] = state_node(&target);
            issue["updatedAt"] = json!("2026-11-10T11:00:00.000Z");
            StubReply::data(json!({"issueUpdate": {"success": true, "issue": issue.clone()}}))
        }
        Some("IssueCreate") if remote.refuse_create => {
            remote.refuse_create = false;
            StubReply::status(503)
        }
        Some("IssueCreate") => {
            let input = &req.variables()["input"];
            // The id Hollow chose (Q-R22).
            let id = input["id"].as_str().unwrap_or("i-created");
            let mut issue = stub::issue_node(id, "ENG-50", TEAM, T0);
            issue["title"] = input["title"].clone();
            issue["project"] = json!({"id": input["projectId"]});
            remote.issues.push(issue.clone());
            StubReply::data(json!({"issueCreate": {"success": true, "issue": issue}}))
        }
        Some("LabelCreate") => {
            // The label's id names it, so a test can tell which was used.
            let name = req.variables()["input"]["name"].clone();
            let id = format!("l-{}", name.as_str().unwrap_or(""));
            StubReply::data(json!({"issueLabelCreate": {"success": true, "issueLabel":
                {"id": id, "name": name, "color": null, "team": {"id": TEAM}, "parent": null}}}))
        }
        Some("Comments") => page("comments", remote.comments.clone()),
        Some("CommentCreate") => {
            let input = &req.variables()["input"];
            let comment = json!({"id": format!("c{}", remote.comments.len() + 1), "body": input["body"],
                "createdAt": "2026-11-10T11:00:00.000Z", "updatedAt": "2026-11-10T11:00:00.000Z",
                "issue": {"id": input["issueId"], "team": {"id": TEAM}}});
            remote.comments.push(comment.clone());
            if remote.lose_comment_reply {
                StubReply::status(503)
            } else {
                StubReply::data(json!({"commentCreate": {"success": true, "comment": comment}}))
            }
        }
        _ => StubReply::status(500),
    };
    if remote.low_rate {
        out.with_rate(10, 2500, 60_000)
    } else {
        out
    }
}

fn mirrored(id: &str, key: &str, title: &str, state: &str) -> (Value, TrackerIssue) {
    let mut node = stub::issue_node(id, key, TEAM, T0);
    node["title"] = json!(title);
    node["state"] = state_node(state);
    node["project"] = json!({"id": "p1"});
    let mut issue = work_tests_issue(id, key, title);
    issue.state = serde_json::from_value(
        json!({"id": state, "name": node["state"]["name"], "kind": node["state"]["type"]}),
    )
    .unwrap();
    (node, issue)
}

fn work_tests_issue(id: &str, key: &str, title: &str) -> TrackerIssue {
    serde_json::from_value(json!({
        "id": id, "key": key, "url": "https://tracker.example/x", "title": title,
        "state": {"id": "state-todo", "name": "Todo", "kind": "unstarted"},
        "projectId": "p1", "teamId": TEAM, "updatedAt": T0, "createdAt": T0
    }))
    .unwrap()
}

/// A tracked workspace `proj` (project p1) and a mirror of `issues`.
async fn fixture(track: bool, issues: &[(&str, &str, &str, &str)]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::create_dir_all(root.join("workspaces/proj")).unwrap();
    std::fs::write(
        root.join("workspaces/proj/PROJECT.md"),
        "---\ntitle: Proj\ntracker: linear\ntracker_project: p1\n---\n",
    )
    .unwrap();
    let config: Config = serde_json::from_value(json!({
        "vaultPath": root,
        "tracker": {"provider": "linear", "trackAgentWork": track, "linear": {"teamId": TEAM, "teamKey": "ENG"}}
    }))
    .unwrap();
    work::set_tracking(&root, track);
    let mut state = Ledger::new("linear", TEAM);
    state.meta = Some(
        serde_json::from_value(json!({
            "team": {"id": TEAM, "key": "ENG", "name": "ENG"},
            "states": [
                {"id": "state-backlog", "name": "Backlog", "kind": "backlog", "position": 0},
                {"id": "state-todo", "name": "Todo", "kind": "unstarted", "position": 1},
                {"id": "state-progress", "name": "In Progress", "kind": "started", "position": 2},
                {"id": "state-review", "name": "In Review", "kind": "started", "position": 3},
                {"id": "state-done", "name": "Done", "kind": "completed", "position": 4}
            ],
            "labels": [], "members": [], "cycles": [], "milestones": [], "projects": []
        }))
        .unwrap(),
    );
    state.records.insert("p1".into(), serde_json::from_value(json!({
        "id": "p1", "key": "P-ENG-1", "kind": "project", "path": "workspaces/proj/PROJECT.md",
        "workspaceId": "proj", "base": {}, "issue": null,
        "project": {"id": "p1", "url": "https://tracker.example/p1", "name": "Proj",
            "status": {"name": "Started", "kind": "started"}, "teamIds": [TEAM], "updatedAt": T0}
    })).unwrap());
    let mut remote = Remote::default();
    for (id, key, title, st) in issues {
        let (node, issue) = mirrored(id, key, title, st);
        remote.issues.push(node);
        state.records.insert(
            (*id).into(),
            serde_json::from_value(json!({
                "id": id, "key": key, "kind": "task", "path": format!("tasks/tracker-{id}.md"),
                "workspaceId": null, "base": {}, "issue": issue, "project": null
            }))
            .unwrap(),
        );
    }
    let remote = Arc::new(Mutex::new(remote));
    let shared = remote.clone();
    let server = stub::serve(move |req| reply(req, &mut shared.lock().unwrap())).await;
    Fixture {
        _dir: dir,
        config,
        state,
        server,
        remote,
    }
}

impl Fixture {
    fn root(&self) -> &Path {
        &self.config.vault_path
    }
    fn adapter(&self) -> LinearAdapter {
        LinearAdapter::with_url(&self.server.url, TEAM, "fixture-key").unwrap()
    }
    fn sent(&self, op: &str) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some(op))
            .count()
    }
    fn facts(&self, title: &str) -> StartFacts {
        StartFacts {
            title: title.into(),
            prompt: title.into(),
            workspace_id: Some("proj".into()),
            cwd: self
                .root()
                .join("workspaces/proj")
                .to_string_lossy()
                .into_owned(),
            parent_key: Some("s-project-proj".into()),
            depth: 1,
            asked_by: "s-project-proj".into(),
            ..StartFacts::default()
        }
    }
    /// Resolve against the fixture mirror (saved first, as a pass would).
    fn start(&self, key: &str, facts: StartFacts) -> Prepared {
        crate::tracker_sync::save(self.root(), &self.state).unwrap();
        let prepared = work::prepare(&self.config, facts).unwrap();
        work::begin(self.root(), key, prepared.clone());
        prepared
    }
    async fn pass(&mut self) -> TrackerSyncOutcome {
        let mut report = TrackerSyncOutcome {
            pulled: 0,
            pushed: 0,
            conflicts: 0,
            dropped_out_of_scope: 0,
            errors: Vec::new(),
            finished_at: String::new(),
        };
        let adapter = self.adapter();
        sync(&self.config, &adapter, &mut self.state, &mut report).await;
        report
    }
    fn comments(&self) -> Vec<String> {
        self.remote
            .lock()
            .unwrap()
            .comments
            .iter()
            .map(|c| c["body"].as_str().unwrap().to_string())
            .collect()
    }
}

/// The switch off: the link records, no fact is queued and the pass makes
/// zero tracker requests.
#[tokio::test]
async fn the_switch_off_sends_nothing_while_links_still_record() {
    let mut f = fixture(
        false,
        &[("i1", "ENG-1", "Fix the Portal export totals", "state-todo")],
    )
    .await;
    let prepared = f.start(
        "s-20261110-off00001",
        f.facts("Fix the Portal export totals"),
    );
    assert!(matches!(
        prepared.resolution,
        Resolution::Link {
            linked_by: linked_by::MATCHED,
            ..
        }
    ));
    assert_eq!(
        work::link_for(f.root(), "s-20261110-off00001")
            .unwrap()
            .issue_key
            .as_deref(),
        Some("ENG-1")
    );
    assert!(work::pending_events(f.root(), 10).unwrap().is_empty());
    f.pass().await;
    assert!(f.server.requests().is_empty());
}

/// Started: one move to the first started state, only from backlog or
/// unstarted, then the Started comment. Outcome on success moves to In
/// Review only when Hollow made the last move; never Done.
#[tokio::test]
async fn status_moves_from_backlog_only_and_in_review_only_when_hollow_moved_last() {
    let mut f = fixture(
        true,
        &[
            (
                "i1",
                "ENG-1",
                "Fix the Portal export totals",
                "state-backlog",
            ),
            ("i2", "ENG-2", "Write the export runbook", "state-progress"),
        ],
    )
    .await;
    f.start(
        "s-20261110-run00001",
        f.facts("Fix the Portal export totals"),
    );
    f.start("s-20261110-run00002", f.facts("Write the export runbook"));
    f.pass().await;
    assert_eq!(f.sent("IssueUpdate"), 1, "only the backlog issue moves");
    let update = f
        .server
        .requests()
        .into_iter()
        .find(|r| r.operation().as_deref() == Some("IssueUpdate"))
        .unwrap();
    assert_eq!(
        update.variables()["input"],
        json!({"stateId": "state-progress"})
    );
    assert_eq!(f.sent("CommentCreate"), 2);
    assert!(f
        .comments()
        .iter()
        .all(|b| b.starts_with("Hollow: agent work started")));

    // The mirror shows Hollow's move: In Review after a successful outcome.
    let moved = |f: &mut Fixture, updated: &str| {
        let issue = f
            .state
            .records
            .get_mut("i1")
            .unwrap()
            .issue
            .as_mut()
            .unwrap();
        issue.state = serde_json::from_value(
            json!({"id": "state-progress", "name": "In Progress", "kind": "started"}),
        )
        .unwrap();
        issue.updated_at = updated.into();
    };
    moved(&mut f, "2026-11-10T11:00:00.000Z");
    let outcome = |status: &str| work::WorkEvent::Outcome {
        status: status.into(),
        result: Some("Done".into()),
        prs: vec![1258],
        results: vec![],
        checks: vec![],
        files_changed: 0,
        working_ms: 60_000,
        tokens: 10,
        cost_usd: None,
        cost_partial: false,
        reason: None,
    };
    work::add_event(f.root(), "s-20261110-run00001", &outcome("success")).unwrap();
    f.pass().await;
    let review = f
        .server
        .requests()
        .into_iter()
        .rfind(|r| r.operation().as_deref() == Some("IssueUpdate"))
        .unwrap();
    assert_eq!(
        review.variables()["input"],
        json!({"stateId": "state-review"})
    );
    assert_eq!(f.sent("IssueUpdate"), 2);

    // A person moved it since (the mirror changed after Hollow's last write):
    // the comment posts, the status stays.
    let mut g = fixture(
        true,
        &[("i1", "ENG-1", "Fix the Portal export totals", "state-todo")],
    )
    .await;
    g.start(
        "s-20261110-run00003",
        g.facts("Fix the Portal export totals"),
    );
    g.pass().await;
    moved(&mut g, "2026-11-10T15:00:00.000Z");
    work::add_event(g.root(), "s-20261110-run00003", &outcome("success")).unwrap();
    g.pass().await;
    assert_eq!(g.sent("IssueUpdate"), 1, "only Hollow's first move");
    assert_eq!(g.sent("CommentCreate"), 2);
    // A failed run comments and never moves.
    work::add_event(g.root(), "s-20261110-run00003", &outcome("error")).unwrap();
    g.pass().await;
    assert_eq!(g.sent("IssueUpdate"), 1);
    assert!(g
        .comments()
        .last()
        .unwrap()
        .contains("\nWhat: Failed (Done)."));
}

/// Two starts of the same work (a retry) make one issue; both runs link it.
#[tokio::test]
async fn a_retried_start_creates_once() {
    let mut f = fixture(true, &[]).await;
    let a = f.start(
        "s-20261110-new00001",
        f.facts("Draft the Q4 parking report"),
    );
    let b = f.start(
        "s-20261110-new00002",
        f.facts("Draft the Q4 parking report"),
    );
    assert_eq!(a.resolution, b.resolution);
    assert!(matches!(a.resolution, Resolution::Create { .. }));
    f.pass().await;
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 1);
    // Sent with a UUID v4 of Hollow's own (Q-R22), so recovery reads it back.
    let create = f
        .server
        .requests()
        .into_iter()
        .find(|r| r.operation().as_deref() == Some("IssueCreate"))
        .unwrap();
    let id = create.variables()["input"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!((id.len(), &id[14..15]), (36, "4"));
    let journaled = f.state.creates.values().next().unwrap().issue_id.clone();
    assert_eq!(journaled.as_deref(), Some(id.as_str()));
    for key in ["s-20261110-new00001", "s-20261110-new00002"] {
        let link = work::link_for(f.root(), key).unwrap();
        assert_eq!(
            (link.issue_key.as_deref(), link.linked_by.as_str()),
            (Some("ENG-50"), "created")
        );
    }
    // A Todo issue moves once to started; each run posts its Started.
    assert_eq!(f.sent("IssueUpdate"), 1);
    assert_eq!(f.comments().len(), 2);
}

/// At most 4 tracking comments an issue an hour (the rest wait), 30 across
/// Hollow, 4 sends a pass, and a low rate budget pauses tracking.
#[tokio::test]
async fn comment_caps_and_a_low_rate_budget_hold_writes_back() {
    let mut f = fixture(
        true,
        &[
            ("i1", "ENG-1", "One", "state-progress"),
            ("i2", "ENG-2", "Two", "state-progress"),
        ],
    )
    .await;
    for n in 0..6 {
        f.start(&format!("s-20261110-cap0000{n}"), f.facts("One"));
    }
    f.pass().await;
    f.pass().await;
    assert_eq!(f.sent("CommentCreate"), 4, "four an issue an hour");
    f.start("s-20261110-cap00010", f.facts("Two"));
    f.pass().await;
    assert_eq!(f.sent("CommentCreate"), 5, "another issue still has room");
    // 30 across Hollow: nothing more goes this hour.
    let now = now();
    for n in 0..30 {
        let mut op = op(
            &PendingEvent {
                id: 900 + n,
                session_key: "s-x".into(),
                at: now.clone(),
                event: work::WorkEvent::Progress {
                    line: "x".into(),
                    link: None,
                    checks: vec![],
                },
            },
            TEAM,
            "i-other",
            OpKind::Comment,
        );
        op.delivery = Delivery::Skipped;
        op.sent_at = Some(now.clone());
        op.body = Some(format!("b {}", op.id));
        f.state.work.enqueue(&format!("zz{n}"), op).unwrap();
    }
    f.start("s-20261110-cap00011", f.facts("Two"));
    f.pass().await;
    assert_eq!(f.sent("CommentCreate"), 5);

    // Rate low: the first send reads the budget, the next waits.
    let mut g = fixture(
        true,
        &[
            ("i1", "ENG-1", "One", "state-progress"),
            ("i2", "ENG-2", "Two", "state-progress"),
        ],
    )
    .await;
    g.remote.lock().unwrap().low_rate = true;
    g.start("s-20261110-rate0001", g.facts("One"));
    g.start("s-20261110-rate0002", g.facts("Two"));
    g.pass().await;
    assert_eq!(g.sent("CommentCreate"), 1);
}

/// One progress comment an issue every 15 minutes; notes in between fold
/// into the next one.
#[tokio::test]
async fn progress_folds_into_one_comment_every_fifteen_minutes() {
    let mut f = fixture(true, &[("i1", "ENG-1", "One", "state-progress")]).await;
    f.start("s-20261110-prog0001", f.facts("One"));
    let note = |line: &str| work::WorkEvent::Progress {
        line: line.into(),
        link: None,
        checks: vec![],
    };
    work::add_event(f.root(), "s-20261110-prog0001", &note("first")).unwrap();
    work::add_event(f.root(), "s-20261110-prog0001", &note("second")).unwrap();
    f.pass().await;
    work::add_event(f.root(), "s-20261110-prog0001", &note("third")).unwrap();
    f.pass().await;
    let progress: Vec<String> = f
        .comments()
        .into_iter()
        .filter(|b| b.starts_with("Build log, ") && b.contains("\nWhat:\n"))
        .collect();
    assert_eq!(progress.len(), 1);
    assert!(progress[0].contains("- first\n- second\n"));
    assert_eq!(
        work::pending_events(f.root(), 10).unwrap().len(),
        1,
        "third waits"
    );
    // Fifteen minutes later it goes.
    for (_, op) in f
        .state
        .work
        .entries
        .iter_mut()
        .filter(|(_, op)| op.progress)
    {
        op.queued_at = (Utc::now() - Duration::minutes(16)).to_rfc3339();
    }
    f.pass().await;
    let progress: Vec<String> = f
        .comments()
        .into_iter()
        .filter(|b| b.starts_with("Build log, ") && b.contains("\nWhat:\n"))
        .collect();
    assert_eq!(progress.len(), 2);
    assert!(progress[1].contains("- third"));
}

/// The attempt is saved before the send; a lost reply, or a crash after the
/// save and before the send, is confirmed by the embedded id and never sent
/// twice.
#[tokio::test]
async fn a_crash_between_persist_and_send_never_double_posts() {
    let mut f = fixture(true, &[("i1", "ENG-1", "One", "state-progress")]).await;
    f.remote.lock().unwrap().lose_comment_reply = true;
    f.start("s-20261110-lost0001", f.facts("One"));
    let mut report = TrackerSyncOutcome {
        pulled: 0,
        pushed: 0,
        conflicts: 0,
        dropped_out_of_scope: 0,
        errors: vec![],
        finished_at: String::new(),
    };
    let adapter = f.adapter();
    sync(&f.config, &adapter, &mut f.state, &mut report).await;
    assert!(
        !report.errors.is_empty(),
        "the lost reply is reported, not fatal"
    );
    assert_eq!(f.sent("CommentCreate"), 1);
    // What the disk holds is the attempt.
    let raw = std::fs::read(f.root().join(crate::tracker_sync::STATE)).unwrap();
    let mut disk: Ledger = serde_json::from_slice(&raw).unwrap();
    let (key, attempt) = disk
        .work
        .entries()
        .next()
        .map(|(k, o)| (k.clone(), o.clone()))
        .unwrap();
    assert_eq!(attempt.delivery, Delivery::Unconfirmed);
    disk.work.entries.get_mut(&key).unwrap().next_check_at =
        Some((Utc::now() - Duration::minutes(1)).to_rfc3339());
    f.state = disk;
    f.remote.lock().unwrap().lose_comment_reply = false;
    f.pass().await;
    assert_eq!(
        f.state.work.get(&key).unwrap().delivery,
        Delivery::Confirmed
    );
    assert_eq!(f.sent("CommentCreate"), 1, "reconciled, not resent");

    // Crashed after the save, before the send: no comment carries the id,
    // so it stays unconfirmed and is never sent.
    let mut g = fixture(true, &[("i1", "ENG-1", "One", "state-progress")]).await;
    g.start("s-20261110-lost0002", g.facts("One"));
    let events = work::pending_events(g.root(), 10).unwrap();
    let mut comment = op(&events[0], TEAM, "i1", OpKind::Comment);
    comment.body = Some(format!(
        "Hollow: agent work started\n\nHollow record {}",
        comment.id
    ));
    comment.delivery = Delivery::Unconfirmed;
    comment.sent_at = Some(now());
    comment.next_check_at = Some((Utc::now() - Duration::minutes(1)).to_rfc3339());
    g.state.work.enqueue("crashed", comment).unwrap();
    work::mark_journaled(g.root(), &[events[0].id]).unwrap();
    for _ in 0..2 {
        g.pass().await;
    }
    assert_eq!(g.sent("CommentCreate"), 0);
    assert_eq!(
        g.state.work.get("crashed").unwrap().delivery,
        Delivery::Unconfirmed
    );
    assert!(g.state.work.validate(&[TEAM.to_string()]).is_ok());
}

fn fact(id: i64) -> PendingEvent {
    PendingEvent {
        id,
        session_key: "s-20261110-fixture1".into(),
        at: now(),
        event: work::WorkEvent::Progress {
            line: "x".into(),
            link: None,
            checks: vec![],
        },
    }
}

/// A send that provably didn't go out (the issue left the team) settles as
/// Skipped with its reason, and the pass goes on to the next write.
#[tokio::test]
async fn a_proven_no_send_is_skipped_and_the_pass_goes_on() {
    let mut f = fixture(
        true,
        &[
            ("i1", "ENG-1", "One", "state-progress"),
            ("i2", "ENG-2", "Two", "state-progress"),
        ],
    )
    .await;
    f.start("s-20261110-gone0001", f.facts("One"));
    f.start("s-20261110-gone0002", f.facts("Two"));
    f.remote.lock().unwrap().issues.retain(|i| i["id"] != "i1");
    f.pass().await;
    assert_eq!(
        f.sent("CommentCreate"),
        1,
        "the other issue's comment still goes"
    );
    let skipped: Vec<&WorkOp> = f
        .state
        .work
        .entries
        .values()
        .filter(|o| o.issue_id == "i1")
        .collect();
    assert_eq!(skipped[0].delivery, Delivery::Skipped);
    assert!(skipped[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("no longer in the team"));
}

/// A queued move is rechecked against the mirror right before it is sent:
/// a person's move wins and the write is Skipped.
#[tokio::test]
async fn a_queued_move_is_rechecked_and_a_persons_move_wins() {
    let mut f = fixture(true, &[("i1", "ENG-1", "One", "state-progress")]).await;
    let mut start = op(&fact(1), TEAM, "i1", OpKind::MoveStarted);
    start.state_id = Some("state-progress".into());
    f.state.work.enqueue("ev1-a-move", start).unwrap();
    let mut review = op(&fact(2), TEAM, "i1", OpKind::MoveReview);
    review.state_id = Some("state-review".into());
    f.state.work.enqueue("ev2-b-review", review).unwrap();
    f.pass().await;
    assert_eq!(f.sent("IssueUpdate"), 0);
    for key in ["ev1-a-move", "ev2-b-review"] {
        assert_eq!(
            f.state.work.get(key).unwrap().delivery,
            Delivery::Skipped,
            "{key}"
        );
    }
}

/// Q-R24: the pass's mirror still says Todo, but a person moved the issue to
/// Done in Linear after it was read. The move re-reads right before the
/// write, sends nothing, and the newer status survives.
#[tokio::test]
async fn an_automatic_move_never_overwrites_a_newer_linear_status() {
    let mut f = fixture(true, &[("i1", "ENG-1", "One", "state-todo")]).await;
    let mut start = op(&fact(1), TEAM, "i1", OpKind::MoveStarted);
    start.state_id = Some("state-progress".into());
    start.seen_state = Some("state-todo".into());
    f.state.work.enqueue("ev1-a-move", start).unwrap();
    // Linear moved on after the mirror snapshot.
    {
        let mut remote = f.remote.lock().unwrap();
        remote.issues[0]["state"] = state_node("state-done");
        remote.issues[0]["updatedAt"] = json!("2026-11-10T10:30:00.000Z");
    }
    f.pass().await;
    assert_eq!(f.sent("IssueUpdate"), 0);
    assert_eq!(
        f.state.work.get("ev1-a-move").unwrap().delivery,
        Delivery::Skipped
    );
    assert_eq!(
        f.remote.lock().unwrap().issues[0]["state"]["id"],
        "state-done"
    );
}

/// A failed re-read before a move sends nothing, waits behind a cooldown and
/// is recorded as not sent, not as a failed update.
#[tokio::test]
async fn a_failed_reread_backs_off_and_sends_nothing() {
    let mut f = fixture(true, &[("i1", "ENG-1", "One", "state-todo")]).await;
    let mut start = op(&fact(1), TEAM, "i1", OpKind::MoveStarted);
    start.state_id = Some("state-progress".into());
    f.state.work.enqueue("ev1-a-move", start).unwrap();
    f.remote.lock().unwrap().fail_reads = true;
    f.pass().await;
    let waiting = f.state.work.get("ev1-a-move").unwrap().clone();
    assert_eq!(waiting.delivery, Delivery::Pending);
    assert_eq!(waiting.reason.as_deref(), Some(REREAD_FAILED));
    assert!(waiting.next_check_at.is_some());
    assert_eq!(f.sent("IssueUpdate"), 0);
    let reads = f.sent("IssueDetail") + f.sent("IssueTarget");
    f.pass().await;
    assert_eq!(f.sent("IssueDetail") + f.sent("IssueTarget"), reads);
}

/// With the switch off the pass makes no request, lets waiting writes go as
/// Skipped and takes every untaken fact, so turning it on never back-posts.
#[tokio::test]
async fn the_switch_off_lets_waiting_writes_and_facts_go() {
    let mut f = fixture(false, &[("i1", "ENG-1", "One", "state-progress")]).await;
    let mut comment = op(&fact(1), TEAM, "i1", OpKind::Comment);
    comment.body = Some(format!("Hollow: progress\n\nHollow record {}", comment.id));
    f.state.work.enqueue("ev1-progress", comment).unwrap();
    work::add_event(f.root(), "s-20261110-fixture1", &fact(1).event).unwrap();
    f.pass().await;
    assert!(f.server.requests().is_empty());
    let entry = f.state.work.get("ev1-progress").unwrap();
    assert_eq!(entry.delivery, Delivery::Skipped);
    assert_eq!(entry.reason.as_deref(), Some("Tracking agent work is off."));
    assert!(work::pending_events(f.root(), 10).unwrap().is_empty());
}

/// An uncertain comment is rechecked three times, then let go as Skipped;
/// a full journal evicts settled entries and never fails an enqueue.
#[tokio::test]
async fn uncertain_writes_settle_after_three_rechecks_and_a_full_journal_never_fails() {
    let mut f = fixture(true, &[("i1", "ENG-1", "One", "state-progress")]).await;
    let mut comment = op(&fact(1), TEAM, "i1", OpKind::Comment);
    comment.body = Some(format!("Hollow: progress\n\nHollow record {}", comment.id));
    comment.delivery = Delivery::Unconfirmed;
    comment.sent_at = Some(now());
    f.state.work.enqueue("lost", comment).unwrap();
    for _ in 0..MAX_RECHECKS {
        f.state.work.entries.get_mut("lost").unwrap().next_check_at =
            Some((Utc::now() - Duration::minutes(1)).to_rfc3339());
        f.pass().await;
    }
    let lost = f.state.work.get("lost").unwrap();
    assert_eq!(
        (lost.delivery, lost.checks),
        (Delivery::Skipped, MAX_RECHECKS)
    );
    assert_eq!(f.sent("CommentCreate"), 0, "never resent");

    let mut full = WorkJournal::default();
    for n in 0..MAX_ENTRIES {
        full.enqueue(
            &format!("k{n:05}"),
            op(&fact(n as i64), TEAM, "i1", OpKind::Comment),
        )
        .unwrap();
    }
    assert!(full
        .enqueue("one-more", op(&fact(9), TEAM, "i1", OpKind::Comment))
        .is_ok());
    assert!(full.get("one-more").is_none(), "dropped, not failed");
    full.entries.get_mut("k00000").unwrap().delivery = Delivery::Skipped;
    full.enqueue("one-more", op(&fact(9), TEAM, "i1", OpKind::Comment))
        .unwrap();
    assert!(full.get("one-more").is_some() && full.get("k00000").is_none());
}

/// A create checks its project right before it is sent: a moved or missing
/// project is refused, nothing is created, and the run is untied.
#[tokio::test]
async fn a_create_into_a_missing_project_is_refused_and_the_run_untied() {
    let mut f = fixture(true, &[]).await;
    let key = "s-20261110-proj0001";
    let prepared = f.start(key, f.facts("Draft the Q4 parking report"));
    assert!(matches!(prepared.resolution, Resolution::Create { .. }));
    f.state.records.get_mut("p1").unwrap().remote_missing = true;
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 0);
    assert!(work::link_for(f.root(), key).is_none());
    assert!(work::pending_events(f.root(), 10).unwrap().is_empty());
}

/// A move queued while the issue was in Backlog is skipped when a person has
/// moved it to Todo since: both are open, but the state differs.
#[tokio::test]
async fn a_queued_move_skips_a_persons_backlog_to_todo_move() {
    let mut f = fixture(true, &[("i1", "ENG-1", "One", "state-todo")]).await;
    let mut start = op(&fact(1), TEAM, "i1", OpKind::MoveStarted);
    start.state_id = Some("state-progress".into());
    start.seen_state = Some("state-backlog".into());
    f.state.work.enqueue("ev1-a-move", start).unwrap();
    f.pass().await;
    assert_eq!(f.sent("IssueUpdate"), 0);
    assert_eq!(
        f.state.work.get("ev1-a-move").unwrap().delivery,
        Delivery::Skipped
    );
}

/// With the switch off the cancel is local and saved first: the sync pass
/// runs it before any remote call, so an offline pass still cancels.
#[tokio::test]
async fn the_switch_off_cancel_is_saved_before_any_remote_call() {
    let mut f = fixture(false, &[("i1", "ENG-1", "One", "state-progress")]).await;
    let mut comment = op(&fact(1), TEAM, "i1", OpKind::Comment);
    comment.body = Some(format!("Hollow: progress\n\nHollow record {}", comment.id));
    f.state.work.enqueue("ev1-progress", comment).unwrap();
    work::add_event(f.root(), "s-20261110-fixture1", &fact(1).event).unwrap();
    assert!(cancel_if_off(&f.config, &mut f.state).unwrap());
    let raw = std::fs::read(f.root().join(crate::tracker_sync::STATE)).unwrap();
    let disk: Ledger = serde_json::from_slice(&raw).unwrap();
    assert_eq!(
        disk.work.get("ev1-progress").unwrap().delivery,
        Delivery::Skipped
    );
    assert!(work::pending_events(f.root(), 10).unwrap().is_empty());
    assert!(f.server.requests().is_empty());
    // The pass calls it before its first remote request.
    let src = include_str!("../tracker_sync.rs");
    let body = &src[src.find("pub(crate) async fn sync_with(").unwrap()..];
    let cancel = body.find("work_outbox::cancel_if_off(").unwrap();
    assert!(cancel < body.find("let result = pass(").unwrap());
}

/// X2: the audit rows runs and agent work writes leave.
#[path = "work_audit_tests.rs"]
mod audit_rows;

/// Q-R22: a create that never landed is sent again with the id saved on
/// disk before the first send, so it can never make a second issue.
#[tokio::test]
async fn a_create_that_never_landed_is_resent_with_its_saved_id() {
    let mut f = fixture(true, &[]).await;
    f.remote.lock().unwrap().refuse_create = true;
    let key = "s-20261110-new00003";
    f.start(key, f.facts("Draft the Q4 parking report"));
    f.pass().await;
    let saved = crate::tracker_sync::load(f.root(), &f.state.provider, &f.state.scope).unwrap();
    let entry = saved.creates.values().next().unwrap().clone();
    assert!(entry.attempted && entry.issue.is_none());
    let id = entry.issue_id.clone().unwrap();
    // What the pass's recovery does when the id reads back absent.
    let create_key = saved.creates.keys().next().unwrap().clone();
    let journal = f.state.creates.get_mut(&create_key).unwrap();
    journal.attempted = false;
    f.pass().await;
    let sent: Vec<String> = f
        .server
        .requests()
        .iter()
        .filter(|r| r.operation().as_deref() == Some("IssueCreate"))
        .map(|r| r.variables()["input"]["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(sent, vec![id.clone(), id.clone()]);
    let link = work::link_for(f.root(), key).unwrap();
    assert_eq!(link.issue_id.as_deref(), Some(id.as_str()));
}

/// T3a: a person's build chat in a tracked project, with Track agent work
/// on, is filed once in team ENG through this outbox (In Progress from
/// Hollow's Started move); a retried turn makes no second issue, and a quick
/// question makes none.
#[tokio::test]
async fn a_real_work_chat_is_filed_once_and_a_quick_question_never() {
    use crate::tracker::chat_work::{self, Running};
    use crate::tracker::settings::AgentWork;
    let mut f = fixture(true, &[]).await;
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    let root = f.root().to_path_buf();
    std::fs::write(
        root.join("workspaces/proj/WORKSPACE.md"),
        "---\nhollow_workspace: 1\ntitle: Proj\n---\n",
    )
    .unwrap();
    chat_work::set_running(
        &root,
        Running {
            mode: AgentWork::On,
            hol: false,
            linear: true,
        },
    );
    let cwd = root.join("workspaces/proj").to_string_lossy().into_owned();
    let receipt: crate::evidence::EvidenceReceipt = serde_json::from_value(json!({
        "id": "r1", "runId": "run-1", "source": "chat", "sourceId": null,
        "startedAt": T0, "finishedAt": "2026-11-10T10:30:00.000Z", "status": "success",
        "verificationState": "verified", "files": [], "checks": [], "failures": [],
        "historyRunId": "h1"
    }))
    .unwrap();
    let quick = receipt.clone();
    let mut built = receipt;
    built.pull_requests =
        vec![serde_json::from_value(json!({"number": 1450, "action": "opened"})).unwrap()];
    for (key, asked, reply, receipt) in [
        (
            "s-20261115-quick009",
            "Is PR 1450 merged?",
            "Yes, PR 1450 merged.",
            &quick,
        ),
        (
            "s-20261115-build009",
            "Fix the export totals",
            "Opened PR 1450.\n\nDone",
            &built,
        ),
    ] {
        crate::session_store::start_turn(&root, key, "claude", &cwd, "run-1", asked).unwrap();
        for _ in 0..2 {
            chat_work::after_turn(&root, key, true, "success", receipt, reply);
        }
    }
    f.pass().await;
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 1);
    let link = work::link_for(&root, "s-20261115-build009").unwrap();
    assert_eq!(
        (link.issue_key.as_deref(), link.linked_by.as_str()),
        (Some("ENG-50"), "chat")
    );
    assert!(work::link_for(&root, "s-20261115-quick009").is_none());
    // The Todo issue moved to In Progress once; Started and Outcome posted.
    assert_eq!(f.sent("IssueUpdate"), 1);
    assert_eq!(f.comments().len(), 2);
}

/// T3a: a create in team PER's pass carries the team and its `agent`
/// label; team ENG's is unchanged.
#[test]
fn a_hol_create_names_its_team_and_the_agent_label() {
    let mut state = Ledger::new("linear", crate::tracker::linear::PERSONAL_TEAM_ID);
    let plain = NewIssue {
        title: "t".into(),
        ..NewIssue::default()
    };
    assert_eq!(
        hol_input(&state, plain.clone()).label_ids,
        Vec::<String>::new()
    );
    state.meta = Some(
        serde_json::from_value(json!({
            "team": {"id": crate::tracker::linear::PERSONAL_TEAM_ID, "key": "PER", "name": "Personal"},
            "states": [], "members": [], "cycles": [], "milestones": [], "projects": [],
            "labels": [{"id": "l-agent", "name": "agent"}, {"id": "l-c", "name": "commitment"}]
        }))
        .unwrap(),
    );
    let hol = hol_input(&state, plain.clone());
    assert_eq!(hol.label_ids, vec!["l-agent".to_string()]);
    assert_eq!(
        hol.team_id.as_deref(),
        Some(crate::tracker::linear::PERSONAL_TEAM_ID)
    );
    assert!(project_in_scope(&state, ""));
    let ai = Ledger::new("linear", TEAM);
    assert_eq!(hol_input(&ai, plain.clone()), plain);
    assert!(!project_in_scope(&ai, ""));
}

/// T3a: a chat's create that fails (its project is gone) unties the chat and
/// leaves an audit row under its create key, so later turns never retry it.
#[tokio::test]
async fn a_failed_chat_create_is_not_retried_every_turn() {
    use crate::tracker::chat_work::{self, Running};
    use crate::tracker::settings::AgentWork;
    let mut f = fixture(true, &[]).await;
    f.state.records.remove("p1");
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    let root = f.root().to_path_buf();
    std::fs::write(
        root.join("workspaces/proj/WORKSPACE.md"),
        "---\nhollow_workspace: 1\ntitle: Proj\n---\n",
    )
    .unwrap();
    chat_work::set_running(
        &root,
        Running {
            mode: AgentWork::On,
            hol: false,
            linear: true,
        },
    );
    let cwd = root.join("workspaces/proj").to_string_lossy().into_owned();
    let receipt: crate::evidence::EvidenceReceipt = serde_json::from_value(json!({
        "id": "r1", "runId": "run-1", "source": "chat", "sourceId": null,
        "startedAt": T0, "finishedAt": "2026-11-10T10:30:00.000Z", "status": "success",
        "verificationState": "verified", "files": [], "checks": [], "failures": [],
        "historyRunId": "h1", "pullRequests": [{"number": 7, "action": "opened"}]
    }))
    .unwrap();
    let key = "s-20261115-fail0001";
    crate::session_store::start_turn(&root, key, "claude", &cwd, "run-1", "Fix it").unwrap();
    chat_work::after_turn(&root, key, true, "success", &receipt, "Done");
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 0);
    assert!(work::link_for(&root, key).is_none());
    chat_work::after_turn(&root, key, true, "success", &receipt, "Done");
    assert!(work::link_for(&root, key).is_none());
    assert!(work::pending_events(&root, 100).unwrap().is_empty());
}

/// T3b: a Waiting on you item in a tracked project lands in team ENG,
/// assigned to the signed-in user with `needs-review` (made once when the
/// team lacks it); answered in Hollow, it gets one comment and is closed.
#[tokio::test]
async fn a_waiting_copy_is_created_assigned_labelled_and_closed_on_answer() {
    use crate::tracker::chat_work::Running;
    use crate::tracker::settings::AgentWork;
    use crate::tracker::waiting_work::{self, reconcile_items};
    let mut f = fixture(true, &[]).await;
    f.state.meta.as_mut().unwrap().viewer =
        Some(serde_json::from_value(json!({"id": "u-viewer", "name": "Signed In"})).unwrap());
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    let root = f.root().to_path_buf();
    let on = Running {
        mode: AgentWork::On,
        hol: false,
        linear: true,
    };
    let waiting = crate::needs_you::NeedsYouItem {
        id: "conflict:tracker:i9:title".into(),
        kind: "conflict".into(),
        source_id: "i9".into(),
        title: "Approve Maria's raise to $28/h?".into(),
        detail: None,
        state: "conflict".into(),
        at: Some(T0.into()),
        workspace_id: Some("proj".into()),
        session_key: None,
        path: None,
        category: Some("task".into()),
        link: None,
    };
    let yes = |_: &work::WorkLink| true;
    let before = chrono::DateTime::parse_from_rfc3339("2026-11-09T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    reconcile_items(&root, &on, &[], true, before, &yes);
    let later = chrono::Utc::now();
    reconcile_items(
        &root,
        &on,
        std::slice::from_ref(&waiting),
        true,
        later,
        &yes,
    );
    f.pass().await;
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 1);
    assert_eq!(f.sent("LabelCreate"), 1);
    let create = f
        .server
        .requests()
        .into_iter()
        .find(|r| r.operation().as_deref() == Some("IssueCreate"))
        .unwrap();
    let input = &create.variables()["input"];
    assert_eq!(input["assigneeId"], json!("u-viewer"));
    assert_eq!(input["labelIds"], json!(["l-needs-review"]));
    assert_eq!(input["projectId"], json!("p1"));
    // Team AI is shared: a generic title, nothing of the item's words.
    assert_eq!(input["title"], json!("A sync conflict waiting on you"));
    let sent = serde_json::to_string(input).unwrap();
    assert!(!sent.contains("Maria") && !sent.contains("$28"), "{sent}");
    let key = waiting_work::link_key(&waiting.id);
    let link = work::link_for(&root, &key).unwrap();
    assert_eq!(link.issue_key.as_deref(), Some("ENG-50"));
    // No Started move or comment: a copy is not a run.
    assert_eq!(f.sent("IssueUpdate"), 0);
    assert!(f.comments().is_empty());
    // Answered in Hollow (absent on two complete passes): one comment
    // saying so, then Done; once.
    for minutes in [1, 3, 5] {
        let at = later + chrono::Duration::minutes(minutes);
        reconcile_items(&root, &on, &[], true, at, &yes);
    }
    f.pass().await;
    f.pass().await;
    let comments = f.comments();
    assert_eq!(comments.len(), 1);
    assert!(
        comments[0].contains("Resolved in Hollow."),
        "{}",
        comments[0]
    );
    assert!(!comments[0].contains("Maria"));
    assert_eq!(f.sent("IssueUpdate"), 1);
    let issue = f.remote.lock().unwrap().issues[0].clone();
    assert_eq!(issue["state"]["id"], json!("state-done"));
    // A second item reuses the label: no second LabelCreate.
    let mut second = waiting.clone();
    second.id = "conflict:tracker:i8:title".into();
    reconcile_items(&root, &on, &[second], true, later, &yes);
    f.pass().await;
    assert_eq!(f.sent("LabelCreate"), 1);
}

/// T3e: a commitment he owes in a tracked project is created in team ENG,
/// assigned to the signed-in user with `commitment` and its due date;
/// marked done in Hollow, it gets one comment and is closed.
#[tokio::test]
async fn a_commitment_copy_is_created_assigned_with_its_due_date_and_closed_when_done() {
    use crate::tracker::chat_work::Running;
    use crate::tracker::commitment_work::{self, reconcile_commitments, snapshot};
    use crate::tracker::settings::AgentWork;
    let mut f = fixture(true, &[]).await;
    f.state.meta.as_mut().unwrap().viewer =
        Some(serde_json::from_value(json!({"id": "u-viewer", "name": "Signed In"})).unwrap());
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    let root = f.root().to_path_buf();
    std::fs::write(root.join("workspaces/proj/call.md"), "# Call\n").unwrap();
    let on = Running {
        mode: AgentWork::On,
        hol: false,
        linear: true,
    };
    let yes = |_: &work::WorkLink| true;
    let pass = |at: chrono::DateTime<chrono::Utc>| {
        let snap = snapshot(&root);
        reconcile_commitments(&root, &on, snap.as_ref(), at, &yes)
    };
    pass(
        chrono::DateTime::parse_from_rfc3339("2026-11-09T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    );
    let c = crate::vault_commitments::create(
        &root,
        crate::vault_commitments::CreateCommitmentInput {
            title: "Send Maria the raise letter".into(),
            direction: "owed_by_me".into(),
            counterparty: "Maria".into(),
            due: Some("2026-11-20".into()),
            source_path: Some("workspaces/proj/call.md".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let later = chrono::Utc::now();
    assert_eq!(pass(later).filed, 1);
    f.pass().await;
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 1);
    assert_eq!(f.sent("LabelCreate"), 1);
    let create = f
        .server
        .requests()
        .into_iter()
        .find(|r| r.operation().as_deref() == Some("IssueCreate"))
        .unwrap();
    let input = &create.variables()["input"];
    assert_eq!(input["assigneeId"], json!("u-viewer"));
    // The label made and used is `commitment`, not `needs-review`.
    assert_eq!(input["labelIds"], json!(["l-commitment"]));
    let made = f
        .server
        .requests()
        .into_iter()
        .find(|r| r.operation().as_deref() == Some("LabelCreate"))
        .unwrap();
    assert_eq!(made.variables()["input"]["name"], json!("commitment"));
    assert_eq!(input["dueDate"], json!("2026-11-20"));
    assert_eq!(input["projectId"], json!("p1"));
    assert_eq!(input["title"], json!(commitment_work::GENERIC_TITLE));
    let sent = serde_json::to_string(input).unwrap();
    assert!(!sent.contains("Maria") && !sent.contains("raise"), "{sent}");
    let key = commitment_work::link_key(&commitment_work::commitment_id(&c.vault_path));
    let link = work::link_for(&root, &key).unwrap();
    assert_eq!(link.issue_key.as_deref(), Some("ENG-50"));
    assert_eq!(f.sent("IssueUpdate"), 0);
    assert!(f.comments().is_empty());
    // Marked done in Hollow: one comment saying so, then Done; once.
    crate::vault_commitments::set_status(&root, &c.url, "fulfilled").unwrap();
    assert_eq!(pass(later + chrono::Duration::minutes(1)).closed, 1);
    assert_eq!(pass(later + chrono::Duration::minutes(2)).closed, 0);
    f.pass().await;
    f.pass().await;
    let comments = f.comments();
    assert_eq!(comments.len(), 1);
    assert!(
        comments[0].contains("Hollow: commitment closed")
            && comments[0].contains("Marked done in Hollow."),
        "{}",
        comments[0]
    );
    assert!(!comments[0].contains("Maria"));
    assert_eq!(f.sent("IssueUpdate"), 1);
    let issue = f.remote.lock().unwrap().issues[0].clone();
    assert_eq!(issue["state"]["id"], json!("state-done"));
}

/// T3b: the close never touches an issue Hollow did not create for the
/// item, and a person who already closed it wins.
#[tokio::test]
async fn a_waiting_close_never_touches_another_issue() {
    let mut f = fixture(true, &[("i1", "ENG-1", "Theirs", "state-todo")]).await;
    let root = f.root().to_path_buf();
    let key = "waiting:conflict:tracker:i1:title";
    work::insert_link_with(
        &root,
        &work::WorkLink {
            session_key: key.into(),
            issue_id: Some("i1".into()),
            issue_key: Some("ENG-1".into()),
            workspace_id: None,
            linked_by: linked_by::WAITING.into(),
            created_issue: false,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
        &[WorkEvent::Resolved {
            answer: "Resolved in Hollow.".into(),
        }],
    )
    .unwrap();
    f.pass().await;
    assert_eq!(f.sent("IssueUpdate"), 0);
    assert!(f.comments().is_empty());
}

/// T3b: a Waiting on you copy's create that never reached the tracker is
/// sent again with the same id (after the pass's recovery read it absent),
/// even long after, never given up; and with no completed state the close
/// fact waits, untaken.
#[tokio::test]
async fn a_waiting_create_that_never_landed_is_sent_again_and_a_close_waits_for_a_done_state() {
    let mut f = fixture(true, &[]).await;
    let root = f.root().to_path_buf();
    let key = "waiting:conflict:tracker:i7:title";
    let create = crate::tracker::waiting_work::create_key("conflict:tracker:i7:title");
    work::insert_link_with(
        &root,
        &work::WorkLink {
            session_key: key.into(),
            issue_id: None,
            issue_key: None,
            workspace_id: Some("proj".into()),
            linked_by: linked_by::WAITING.into(),
            created_issue: true,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
        &[WorkEvent::Create {
            create_key: create.clone(),
            project_id: "p1".into(),
            title: "A sync conflict waiting on you".into(),
            description: "d".into(),
            team_id: None,
            labels: Vec::new(),
            assign_viewer: true,
            due: None,
        }],
    )
    .unwrap();
    f.remote.lock().unwrap().refuse_create = true;
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 1);
    // Recovery read its id back absent, an hour ago: not attempted, an error.
    let entry = f.state.creates.get_mut(&create).unwrap();
    assert!(entry.issue.is_none());
    entry.attempted = false;
    entry.error = Some("did not reach the tracker".into());
    entry.at = "2026-11-10T09:00:00.000Z".into();
    crate::tracker_sync::save(&root, &f.state).unwrap();
    f.pass().await;
    assert_eq!(f.sent("IssueCreate"), 2);
    let link = work::link_for(&root, key).unwrap();
    assert!(link.issue_id.is_some());
    // The team has no completed state: the close fact waits, audited once.
    f.state
        .meta
        .as_mut()
        .unwrap()
        .states
        .retain(|s| s.kind != StateKind::Completed);
    crate::tracker_sync::save(&root, &f.state).unwrap();
    work::finish_link(
        &root,
        key,
        &WorkEvent::Resolved {
            answer: "Resolved in Hollow.".into(),
        },
    )
    .unwrap();
    f.pass().await;
    f.pass().await;
    assert!(f.comments().is_empty());
    assert_eq!(f.sent("IssueUpdate"), 0);
    assert!(work::pending_events(&root, 100)
        .unwrap()
        .iter()
        .any(|e| matches!(e.event, WorkEvent::Resolved { .. })));
    let waits = crate::audit::rows_of(&root, key, crate::audit::kind::TRACKER_WRITE)
        .into_iter()
        .filter(|r| {
            r.request_key
                .as_deref()
                .is_some_and(|k| k.ends_with("-no-completed-state"))
        })
        .count();
    assert_eq!(waits, 1);
}

/// A later build's write or fact never bricks this one: an unknown op loads
/// and is never sent; an unknown fact is read as Unknown and taken.
#[test]
fn a_later_builds_unknown_write_loads_and_is_never_sent() {
    let op: WorkOp = serde_json::from_value(json!({
        "id": format!("work-{}", "a".repeat(32)), "sessionKey": "s", "teamId": TEAM,
        "issueId": "i1", "kind": "moveSomewhereNew", "delivery": "pending",
        "queuedAt": T0
    }))
    .unwrap();
    assert_eq!(op.kind, OpKind::Unknown);
    let mut journal = WorkJournal::default();
    journal.enqueue("ev000000000001-x", op).unwrap();
    assert!(journal.validate(&[TEAM.to_string()]).is_ok());
    assert!(journal.due_keys(chrono::Utc::now()).is_empty());
    let fact: WorkEvent = serde_json::from_value(json!({"kind": "somethingNew", "x": 1})).unwrap();
    assert_eq!(fact, WorkEvent::Unknown);
}

/// T3b: a create that keeps failing is resent a few times with its id,
/// then given up for good, audited, and never sent again.
#[tokio::test]
async fn a_waiting_create_that_keeps_failing_is_given_up_after_its_resends() {
    let mut f = fixture(true, &[]).await;
    let root = f.root().to_path_buf();
    let key = "waiting:conflict:tracker:i6:title";
    let create = crate::tracker::waiting_work::create_key("conflict:tracker:i6:title");
    work::insert_link_with(
        &root,
        &work::WorkLink {
            session_key: key.into(),
            issue_id: None,
            issue_key: None,
            workspace_id: Some("proj".into()),
            linked_by: linked_by::WAITING.into(),
            created_issue: true,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
        &[WorkEvent::Create {
            create_key: create.clone(),
            project_id: "p1".into(),
            title: "A sync conflict waiting on you".into(),
            description: "d".into(),
            team_id: None,
            labels: Vec::new(),
            assign_viewer: false,
            due: None,
        }],
    )
    .unwrap();
    for _ in 0..8 {
        f.remote.lock().unwrap().refuse_create = true;
        f.pass().await;
        // The pass's recovery reads its id back absent each time.
        if let Some(entry) = f.state.creates.get_mut(&create) {
            entry.attempted = false;
            entry.error = Some("did not reach the tracker".into());
            crate::tracker_sync::save(&root, &f.state).unwrap();
        }
    }
    // The first send and its three resends; nothing after.
    assert_eq!(
        f.sent("IssueCreate"),
        1 + crate::tracker::work_outbox::WAITING_RESENDS as usize
    );
    assert!(work::link_for(&root, key).is_none());
    let gave_up = crate::audit::rows_of(&root, key, crate::audit::kind::TRACKER_WRITE_FAILED)
        .into_iter()
        .filter(|r| r.request_key.as_deref() == Some(format!("{create}-gave-up").as_str()))
        .count();
    assert_eq!(gave_up, 1);
}

/// T3g helpers: a session row, and a link on `issue` made by `by` with the
/// title captured as `work::begin` captures it.
fn session_with_link(root: &Path, key: &str, title: &str, by: &'static str, issue: &str) {
    crate::session_store::start_turn(
        root,
        key,
        "claude",
        &root.join("workspaces/proj").to_string_lossy(),
        &format!("run-{key}"),
        title,
    )
    .unwrap();
    work::insert_link(
        root,
        &work::WorkLink {
            session_key: key.into(),
            issue_id: Some(issue.into()),
            issue_key: Some("ENG-1".into()),
            workspace_id: Some("proj".into()),
            linked_by: by.into(),
            created_issue: false,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: work::run_title(title),
        },
    )
    .unwrap();
}

fn receipt_with_checks(id: &str, commands: &[&str]) -> crate::evidence::EvidenceReceipt {
    crate::evidence::EvidenceReceipt {
        id: id.into(),
        checks: commands
            .iter()
            .enumerate()
            .map(|(n, command)| crate::evidence::ReceiptCheck {
                command: (*command).into(),
                kind: "test".into(),
                outcome: "passed".into(),
                sequence: n as u64 + 1,
                excerpt: None,
            })
            .collect(),
        ..Default::default()
    }
}

fn rename(root: &Path, key: &str, title: &str) {
    crate::session_store::with_write(root, "test rename", |tx| {
        tx.execute(
            "UPDATE sessions SET title = ?2 WHERE key = ?1",
            rusqlite::params![key, title],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

/// T3g pass: a worker chat and its subagent each leave their own dated
/// build log comment on the issue (What, Why, Checked). A token planted in
/// the subagent's title is masked; a check command shows only its runner
/// (no arguments, paths or file names); a title changed after the start is
/// never used.
#[tokio::test]
async fn a_chat_and_its_subagent_each_leave_a_dated_build_log_comment() {
    let mut f = fixture(true, &[("i1", "ENG-1", "Fix the totals", "state-progress")]).await;
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123";
    let chat = "s-20261115-chat0001";
    let sub = "s-20261115-sub00001";
    session_with_link(f.root(), chat, "Fix the totals", linked_by::CHAT, "i1");
    session_with_link(
        f.root(),
        sub,
        &format!("Check the totals with {secret}"),
        linked_by::INHERITED,
        "i1",
    );
    crate::session_store::set_parent(f.root(), sub, chat, crate::session_store::FORK_SUBAGENT)
        .unwrap();
    rename(f.root(), sub, "Renamed later: client payroll dispute");
    rename(f.root(), chat, "Renamed later: client payroll dispute");
    work::after_turn(
        f.root(),
        chat,
        "success",
        Some(&receipt_with_checks(
            "r-chat",
            &[
                "cd src-tauri && cargo test --manifest-path ~/Hollow/Projects/Client-Foo/Cargo.toml",
                "pytest tests/test_acme_payroll_dispute.py",
            ],
        )),
        "Fixed it in PR #12.\n\nDone",
    );
    work::after_turn(
        f.root(),
        sub,
        "success",
        Some(&receipt_with_checks(
            "r-sub",
            &[&format!(
                "npm test -- --token {secret} https://acme.example/x"
            )],
        )),
        "Checked.\n\nDone",
    );
    f.pass().await;
    let logs: Vec<String> = f
        .comments()
        .into_iter()
        .filter(|b| b.starts_with("Build log, "))
        .collect();
    assert_eq!(logs.len(), 2, "{logs:?}");
    let stamp = chrono::Local::now().format("%Y-%m-%d").to_string();
    for body in &logs {
        assert!(body.starts_with(&format!("Build log, {stamp} ")), "{body}");
        assert!(
            body.contains("\nWhy: work on ENG-1, Fix the totals\n"),
            "{body}"
        );
        for private in [
            "ghp_abcdefghijklmnop",
            "Renamed later",
            "test_acme_payroll_dispute.py",
            "pytest tests/",
            "cargo test --manifest-path ~/Hollow/Projects/Client-Foo/Cargo.toml",
            "Client-Foo",
            "acme.example",
            "--token",
        ] {
            assert!(!body.contains(private), "{private} in {body}");
        }
    }
    let chat_body = logs.iter().find(|b| b.contains("(chat)\n")).unwrap();
    assert!(chat_body.starts_with(&format!("Build log, {stamp} ")));
    assert!(
        chat_body.contains(", Fix the totals (chat)\n"),
        "{chat_body}"
    );
    assert!(
        chat_body.contains("\nWhat: Finished (Done). Pull requests 12.\n"),
        "{chat_body}"
    );
    assert!(
        chat_body.contains("\nChecked: test passed (cargo test), test passed (pytest)\n"),
        "{chat_body}"
    );
    let sub_body = logs.iter().find(|b| b.contains("(subagent)\n")).unwrap();
    assert!(sub_body.contains(", Check the totals with "), "{sub_body}");
    assert!(
        sub_body.contains("\nChecked: test passed (npm test)\n"),
        "{sub_body}"
    );
    assert!(sub_body.contains(&format!("Session: {sub}")), "{sub_body}");
}

/// T3g: only a known runner and its step survive from a check command.
#[test]
fn a_check_command_shows_only_its_runner() {
    let cases = [
        ("pytest tests/test_acme_payroll_dispute.py", Some("pytest")),
        (
            "cargo test --manifest-path ~/Hollow/Projects/Client-Foo/Cargo.toml",
            Some("cargo test"),
        ),
        (
            "cd src-tauri && CARGO_BUILD_JOBS=8 cargo clippy --lib -- -D warnings",
            Some("cargo clippy"),
        ),
        (
            "hollow-slot --on runner-host-lan npm run gate",
            Some("npm run gate"),
        ),
        ("npm run deploy-acme-site", Some("npm run")),
        ("npx vitest run src/acmePayroll.test.ts", Some("vitest")),
        ("python3 -m pytest -k acme", Some("pytest")),
        ("python3 scripts/acme_report.py", None),
        ("/Users/x/Client-Foo/bin/run-checks --all", None),
        ("curl https://acme.example/health", None),
        ("node scripts/check-acme.mjs", Some("node")),
        (
            "cargo --manifest-path ~/Client-Foo/Cargo.toml test",
            Some("cargo"),
        ),
        ("make -C ~/Client-Foo test", Some("make")),
    ];
    for (command, want) in cases {
        assert_eq!(work::runner_of(command).as_deref(), want, "{command}");
    }
}

/// T3g: a dry run leaves one audit row for the run's outcome, then at most
/// one progress row per issue every 15 minutes, keyed so a repeat (with or
/// without a receipt) adds no row; it queues and sends nothing.
#[tokio::test]
async fn a_dry_run_audits_build_log_comments_folded_and_sends_nothing() {
    use crate::tracker::chat_work::{set_running, Running};
    use crate::tracker::settings::AgentWork;
    let mut f = fixture(
        false,
        &[("i1", "ENG-1", "Fix the totals", "state-progress")],
    )
    .await;
    let key = "s-20261115-dry00001";
    session_with_link(f.root(), key, "Fix the totals", linked_by::CHAT, "i1");
    let dry = |mode| Running {
        mode,
        hol: false,
        linear: true,
    };
    set_running(f.root(), dry(AgentWork::DryRun));
    let rows = |f: &Fixture| -> Vec<crate::audit::AuditEntry> {
        crate::audit::rows_of(f.root(), key, crate::audit::kind::TRACKER_WRITE)
            .into_iter()
            .filter(|r| r.action.as_deref() == Some("comment"))
            .collect()
    };
    let receipt = receipt_with_checks("r-dry", &["cargo test"]);
    work::after_turn(f.root(), key, "success", Some(&receipt), "Done");
    work::after_turn(f.root(), key, "success", Some(&receipt), "Done");
    assert!(work::pending_events(f.root(), 10).unwrap().is_empty());
    let got = rows(&f);
    assert_eq!(got.len(), 1, "a repeat of the same turn adds no row");
    assert_eq!(got[0].outcome.as_deref(), Some("skipped"));
    assert_eq!(got[0].request_key.as_deref(), Some("build-log-r-dry"));
    assert!(got[0].detail.as_deref().unwrap().starts_with(
        "Dry run: Hollow would post this run's build log comment (its outcome) on ENG-1"
    ));
    // Later turns with no receipt: progress, keyed by the turn count.
    let link = work::link_for(f.root(), key).unwrap();
    let now = chrono::Utc::now();
    work::dry_run_build_log(f.root(), &link, None, now);
    work::dry_run_build_log(f.root(), &link, None, now);
    let got = rows(&f);
    assert_eq!(got.len(), 2, "the same turn without a receipt adds no row");
    assert_eq!(
        got[1].request_key.as_deref(),
        Some(format!("build-log-{key}-turn-1").as_str())
    );
    assert!(got[1]
        .detail
        .as_deref()
        .unwrap()
        .contains("a progress build log comment on ENG-1"));
    // Another turn inside 15 minutes folds; after them it goes.
    crate::session_store::start_turn(f.root(), key, "claude", "/v", "run-2", "x").unwrap();
    work::dry_run_build_log(f.root(), &link, None, now + Duration::minutes(5));
    assert_eq!(rows(&f).len(), 2, "folded into the last progress row");
    work::dry_run_build_log(f.root(), &link, None, now + Duration::minutes(16));
    let got = rows(&f);
    assert_eq!(got.len(), 3);
    assert_eq!(
        got[2].request_key.as_deref(),
        Some(format!("build-log-{key}-turn-2").as_str())
    );
    f.pass().await;
    assert_eq!(f.sent("CommentCreate"), 0);
    // Off: nothing at all.
    set_running(f.root(), dry(AgentWork::Off));
    let other = receipt_with_checks("r-off", &["cargo test"]);
    work::after_turn(f.root(), key, "success", Some(&other), "Done");
    assert_eq!(rows(&f).len(), 3);
}

// ── DR1b: a decision's build log comment ───────────────────────────────────

fn write_decision(root: &Path, name: &str, extra: &str) -> String {
    std::fs::create_dir_all(root.join("decisions")).unwrap();
    let rel = format!("decisions/2026-11-17-{name}.md");
    std::fs::write(
        root.join(&rel),
        format!(
            "---\ntype: decision\ntitle: Queue runs on plain files\ndecided_at: 2026-11-17\n\
             decided_by: Alex\nscope: global\nstatus: revisit\nevidence: s-1#turn2\n{extra}---\n\n# Queue\n"
        ),
    )
    .unwrap();
    rel
}

fn decision_rows(f: &Fixture, key: &str) -> Vec<crate::audit::AuditEntry> {
    crate::audit::rows_of(f.root(), key, crate::audit::kind::TRACKER_WRITE)
        .into_iter()
        .filter(|r| {
            r.request_key
                .as_deref()
                .is_some_and(|k| k.starts_with("decision-log-"))
        })
        .collect()
}

#[test]
fn a_decision_comment_has_four_lines_and_omits_an_empty_rejected() {
    let at = "2026-11-17T12:00:00Z";
    let full = work::WorkEvent::Decision {
        change: "settled".into(),
        what: "Queue runs on plain files".into(),
        status: "settled".into(),
        why: Some("Redis is one more thing to run".into()),
        rejected: "Use Redis -> too heavy; Use SQLite".into(),
        verified: Some("cargo test queue passed".into()),
    };
    let body = work::render_decision(&full, at, "s-1", "work-1");
    for line in [
        "What: Queue runs on plain files (settled)",
        "Why: Redis is one more thing to run",
        "Rejected: Use Redis -> too heavy; Use SQLite",
        "Verified: cargo test queue passed",
    ] {
        assert!(body.contains(&format!("\n{line}\n")), "{line}\n{body}");
    }
    assert!(body.starts_with("Build log, 2026-11-"), "{body}");
    assert!(body.contains("Decision settled"), "{body}");
    let bare = work::WorkEvent::Decision {
        change: "recorded".into(),
        what: "Queue runs on plain files".into(),
        status: "settled".into(),
        why: None,
        rejected: String::new(),
        verified: None,
    };
    let body = work::render_decision(&bare, at, "s-1", "work-2");
    assert!(!body.contains("Rejected"), "{body}");
    assert!(
        !body.contains("\n\n\n"),
        "no blank gap where it was: {body}"
    );
    assert!(
        body.contains("\nWhy: not recorded\nVerified: no check recorded\n"),
        "{body}"
    );
}

#[tokio::test]
async fn a_decision_dry_run_audits_once_and_posts_nothing() {
    use crate::tracker::chat_work::{set_running, Running};
    use crate::tracker::settings::AgentWork;
    let mut f = fixture(
        false,
        &[("i1", "ENG-1", "Fix the totals", "state-progress")],
    )
    .await;
    let key = "s-20261117-dec00001";
    session_with_link(f.root(), key, "Fix the totals", linked_by::CHAT, "i1");
    set_running(
        f.root(),
        Running {
            mode: AgentWork::DryRun,
            hol: false,
            linear: true,
        },
    );
    let rel = write_decision(
        f.root(),
        "dry",
        "issue: eng-1\nrejected: Use Redis -> too heavy\n",
    );
    crate::records::set_decision_status(f.root(), &rel, "settled").unwrap();
    // The same status again is no change: no second row.
    crate::records::set_decision_status(f.root(), &rel, "settled").unwrap();
    let rows = decision_rows(&f, key);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].outcome.as_deref(), Some("skipped"));
    assert!(rows[0]
        .detail
        .as_deref()
        .unwrap()
        .starts_with("Dry run: Hollow would post a decision build log comment on ENG-1"));
    assert!(work::pending_events(f.root(), 10).unwrap().is_empty());
    f.pass().await;
    assert_eq!(f.sent("CommentCreate"), 0);
}

#[tokio::test]
async fn a_decision_with_no_linked_issue_makes_nothing() {
    use crate::tracker::chat_work::{set_running, Running};
    use crate::tracker::settings::AgentWork;
    let mut f = fixture(true, &[("i1", "ENG-1", "Fix the totals", "state-progress")]).await;
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    let key = "s-20261117-dec00002";
    session_with_link(f.root(), key, "Fix the totals", linked_by::CHAT, "i1");
    for mode in [AgentWork::DryRun, AgentWork::On] {
        set_running(
            f.root(),
            Running {
                mode,
                hol: false,
                linear: true,
            },
        );
        // No `issue` at all, then an issue no run is linked to.
        let none = write_decision(f.root(), "none", "");
        let other = write_decision(f.root(), "other", "issue: ENG-99\n");
        crate::records::set_decision_status(f.root(), &none, "settled").unwrap();
        crate::records::set_decision_status(f.root(), &other, "settled").unwrap();
        crate::records::set_decision_status(f.root(), &none, "revisit").unwrap();
        crate::records::set_decision_status(f.root(), &other, "revisit").unwrap();
    }
    assert!(decision_rows(&f, key).is_empty());
    assert!(work::pending_events(f.root(), 10).unwrap().is_empty());
    f.pass().await;
    assert_eq!(f.sent("CommentCreate"), 0);
}

#[tokio::test]
async fn a_decision_comment_posts_through_the_outbox_when_tracking_is_on() {
    use crate::tracker::chat_work::{set_running, Running};
    use crate::tracker::settings::AgentWork;
    let mut f = fixture(true, &[("i1", "ENG-1", "Fix the totals", "state-progress")]).await;
    crate::tracker_sync::save(f.root(), &f.state).unwrap();
    let key = "s-20261117-dec00003";
    session_with_link(f.root(), key, "Fix the totals", linked_by::CHAT, "i1");
    set_running(
        f.root(),
        Running {
            mode: AgentWork::On,
            hol: false,
            linear: true,
        },
    );
    let rel = write_decision(
        f.root(),
        "on",
        "issue: ENG-1\nwhy: Plain files are simplest\nverified: queue tests passed\nrejected: Use Redis -> too heavy\n",
    );
    crate::records::set_decision_status(f.root(), &rel, "settled").unwrap();
    assert_eq!(work::pending_events(f.root(), 10).unwrap().len(), 1);
    f.pass().await;
    let logs: Vec<String> = f
        .comments()
        .into_iter()
        .filter(|b| b.contains("Decision settled"))
        .collect();
    assert_eq!(logs.len(), 1, "{logs:?}");
    for line in [
        "What: Queue runs on plain files (settled)",
        "Why: Plain files are simplest",
        "Rejected: Use Redis -> too heavy",
        "Verified: queue tests passed",
    ] {
        assert!(logs[0].contains(line), "{line}\n{}", logs[0]);
    }
}
