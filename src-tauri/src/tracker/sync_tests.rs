//! Stub GraphQL and fixture vault regressions for the sync engine.

use super::*;
use crate::tracker::linear::LinearAdapter;
use crate::tracker::stub::{self, StubReply, StubRequest, StubServer};
use crate::vault_tasks;
use std::sync::Mutex as StdMutex;

const TEAM: &str = crate::tracker::linear::TEAM_ID;
const OTHER: &str = "team-other-0001";
const HOL: &str = crate::tracker::linear::PERSONAL_TEAM_ID;
const T0: &str = "2026-11-08T05:00:00.000Z";
const T1: &str = "2026-11-08T05:30:00.000Z";

#[derive(Default)]
struct RemoteState {
    issues: Vec<Value>,
    comments: Vec<Value>,
    projects: Vec<Value>,
    fail: Option<u16>,
    lose_create_reply: bool,
    lose_project_reply: bool,
    lose_archive_reply: bool,
    lose_comment_reply: bool,
    comment_target_failure: Option<u16>,
    project_create_out_of_scope: bool,
    leak_scope: bool,
    truncate_detail: bool,
    /// X2: every issue update is refused (an edit the tracker rejects).
    reject_updates: bool,
    /// Q-R22: a create's read-back by id answers without a nodes list.
    malformed_landing: bool,
    /// The key sees only AI and HOL (R3): a create moved into another team reads as missing.
    restricted_key: bool,
    /// T1: labels created through `issueLabelCreate`, each with its team.
    labels: Vec<Value>,
}

fn page(field: &str, nodes: Vec<Value>) -> StubReply {
    let mut data = serde_json::Map::new();
    data.insert(
        field.into(),
        json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}}),
    );
    StubReply::data(Value::Object(data))
}

fn filtered(req: &StubRequest, nodes: &[Value]) -> Vec<Value> {
    let filter = &req.variables()["filter"];
    let id = filter.pointer("/id/eq").and_then(Value::as_str);
    let ids = filter.pointer("/id/in").and_then(Value::as_array);
    let team = filter.pointer("/team/id/eq").and_then(Value::as_str);
    let since = filter
        .pointer("/updatedAt/gt")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
    nodes
        .iter()
        .filter(|n| id.is_none_or(|id| n["id"] == id))
        .filter(|n| ids.is_none_or(|ids| ids.contains(&n["id"])))
        .filter(|n| team.is_none_or(|team| n["team"]["id"] == team))
        .filter(|n| {
            since.is_none_or(|since| {
                DateTime::parse_from_rfc3339(n["updatedAt"].as_str().unwrap_or(T0))
                    .is_ok_and(|at| at > since)
            })
        })
        .cloned()
        .collect()
}

fn respond(req: &StubRequest, state: &mut RemoteState) -> StubReply {
    // The team a query is scoped to (the personal team's pass names its own).
    let team = req.variables()["team"]
        .as_str()
        .or_else(|| {
            req.variables()
                .pointer("/filter/team/id/eq")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            req.variables()
                .pointer("/filter/issue/team/id/eq")
                .and_then(Value::as_str)
        })
        .unwrap_or(TEAM)
        .to_string();
    let team = team.as_str();
    if let Some(refused) = stub::unscoped(req, team) {
        return refused;
    }
    if let Some(code) = state.fail {
        return StubReply::status(code);
    }
    if req.operation().as_deref() == Some("IssueTarget") {
        if let Some(code) = state.comment_target_failure {
            return StubReply::status(code);
        }
    }
    let connection = |nodes: Vec<Value>| json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}});
    match req.operation().as_deref() {
        Some("Team") => {
            let key = if team == HOL { "PER" } else { "ENG" };
            StubReply::data(json!({"team":{"id":team,"key":key,"name":"Fixture team"}}))
        }
        Some("States") => StubReply::data(
            json!({"team":{"id":team,"states":connection(vec![json!({"id":"state-todo","name":"Todo","type":"unstarted","position":0,"team":{"id":team}}),json!({"id":"state-done","name":"Done","type":"completed","position":1,"team":{"id":team}})])}}),
        ),
        Some("Viewer") => {
            StubReply::data(json!({"viewer":{"id":"user-1","name":"Fixture Person","email":null}}))
        }
        Some("Members") => StubReply::data(
            json!({"team":{"id":team,"members":connection(vec![json!({"id":"user-1","name":"Fixture Person","email":null})])}}),
        ),
        Some("Labels") => {
            // The team's own labels (the `or` filter's first branch) and the
            // workspace's `Feature`.
            let asked = req
                .variables()
                .pointer("/filter/or/0/team/id/eq")
                .and_then(Value::as_str)
                .unwrap_or(team);
            let mut nodes = vec![json!({"id":"label-1","name":"Feature","color":"#fff"})];
            nodes.extend(
                state
                    .labels
                    .iter()
                    .filter(|l| l["team"]["id"] == asked)
                    .cloned(),
            );
            StubReply::data(json!({"issueLabels":connection(nodes)}))
        }
        Some("LabelCreate") => {
            let input = &req.variables()["input"];
            let label = json!({"id":format!("label-new-{}", state.labels.len() + 1),"name":input["name"],"color":null,"team":{"id":input["teamId"]},"parent":null});
            state.labels.push(label.clone());
            StubReply::data(json!({"issueLabelCreate":{"success":true,"issueLabel":label}}))
        }
        Some("Cycles") => StubReply::data(json!({"team":{"id":team,"cycles":connection(vec![])}})),
        Some("Projects") => page("projects", state.projects.clone()),
        Some("Milestones") => page("projects", vec![]),
        Some("ProjectTarget") => page("projects", state.projects.clone()),
        Some("ProjectCreate") => {
            let input = &req.variables()["input"];
            let id = format!("new-project-{}", state.projects.len() + 1);
            let project = json!({"id":id,"identifier":"P-ENG-99","url":format!("https://linear.app/example/project/{id}"),"name":input["name"],"description":input.get("description").cloned().unwrap_or(json!("")),"status":{"name":"Started","type":"started"},"lead":null,"startDate":null,"targetDate":input.get("targetDate").cloned().unwrap_or(Value::Null),"updatedAt":T1,"progress":0.0,"teams":{"nodes":[{"id":TEAM}]},"externalLinks":{"nodes":[]}});
            let mut project = project;
            if state.project_create_out_of_scope {
                project["teams"] = json!({"nodes":[{"id":OTHER}]});
            }
            state.projects.push(project.clone());
            if state.lose_project_reply {
                StubReply::status(503)
            } else {
                StubReply::data(json!({"projectCreate":{"success":true,"project":project}}))
            }
        }
        Some("IssueArchive") => {
            let id = req.variables()["id"].as_str().unwrap();
            let issue = state.issues.iter_mut().find(|i| i["id"] == id).unwrap();
            issue["archivedAt"] = json!(T1);
            if state.lose_archive_reply {
                StubReply::status(503)
            } else {
                StubReply::data(json!({"issueArchive":{"success":true}}))
            }
        }
        Some("ProjectUpdate") => {
            let id = req.variables()["id"].as_str().unwrap();
            let project = state.projects.iter_mut().find(|p| p["id"] == id).unwrap();
            for (key, value) in req.variables()["input"].as_object().unwrap() {
                if key == "leadId" {
                    project["lead"] = if value.is_null() {
                        Value::Null
                    } else {
                        json!({"id":value,"name":"Fixture Person","email":null})
                    };
                } else {
                    project[key] = value.clone();
                }
            }
            project["updatedAt"] = json!(T1);
            StubReply::data(json!({"projectUpdate":{"success":true,"project":project}}))
        }
        Some("TeamsVisible") => {
            let mut nodes = vec![json!({"id": TEAM}), json!({"id": HOL})];
            if !state.restricted_key {
                nodes.push(json!({"id": "team-lp-fixture"}));
            }
            StubReply::data(json!({"teams": {"nodes": nodes}}))
        }
        Some("IssueLanding") if state.malformed_landing => StubReply::data(json!({"issues": {}})),
        // Any team, id and team only (Q-R22).
        Some("IssueLanding") => {
            let id = req.variables()["id"].clone();
            let nodes = state
                .issues
                .iter()
                .filter(|issue| issue["id"] == id)
                .map(|issue| json!({"id": issue["id"], "team": issue["team"]}))
                .collect();
            page("issues", nodes)
        }
        Some("Issues") if state.leak_scope => page("issues", state.issues.clone()),
        Some("IssueDetail") if state.truncate_detail => {
            let mut issues = filtered(req, &state.issues);
            for issue in &mut issues {
                for field in ["comments", "attachments"] {
                    if let Some(nodes) = issue[field]["nodes"].as_array_mut() {
                        nodes.truncate(1);
                    }
                }
            }
            page("issues", issues)
        }
        Some("Issues" | "IssueDetail" | "IssueTarget" | "Attachments" | "IssueHistory") => {
            page("issues", filtered(req, &state.issues))
        }
        Some("Comments") => {
            let target = req
                .variables()
                .pointer("/filter/issue/id/eq")
                .and_then(Value::as_str);
            page(
                "comments",
                state
                    .comments
                    .iter()
                    .filter(|comment| target.is_none_or(|id| comment["issue"]["id"] == id))
                    .cloned()
                    .collect(),
            )
        }
        Some("CommentCreate") => {
            let input = &req.variables()["input"];
            let id = format!("comment-{}", state.comments.len() + 1);
            let comment = json!({"id":id,"body":input["body"],"createdAt":T1,"updatedAt":T1,
                "issue":{"id":input["issueId"],"team":{"id":TEAM}},"user":{"name":"Fixture Person"}});
            state.comments.push(comment.clone());
            if state.lose_comment_reply {
                StubReply::status(503)
            } else {
                StubReply::data(json!({"commentCreate":{"success":true,"comment":comment}}))
            }
        }
        Some("IssueUpdate") if state.reject_updates => {
            StubReply::data(json!({"issueUpdate":{"success":false,"issue":null}}))
        }
        Some("IssueUpdate") => {
            let id = req.variables()["id"].as_str().unwrap();
            let issue = state.issues.iter_mut().find(|i| i["id"] == id).unwrap();
            let input = req.variables()["input"].as_object().unwrap();
            for (key, value) in input {
                match key.as_str() {
                    "stateId" => {
                        issue["state"] = if value == "state-done" {
                            json!({"id":"state-done","name":"Done","type":"completed"})
                        } else {
                            json!({"id":"state-todo","name":"Todo","type":"unstarted"})
                        }
                    }
                    "assigneeId" => {
                        issue["assignee"] = if value.is_null() {
                            Value::Null
                        } else {
                            json!({"id":value,"name":"Fixture Person","email":null})
                        }
                    }
                    "projectId" => {
                        issue["project"] = if value.is_null() {
                            Value::Null
                        } else {
                            json!({"id":value})
                        }
                    }
                    "labelIds" => {
                        issue["labels"] = json!({"nodes":value.as_array().unwrap().iter().map(|_| json!({"name":"Feature"})).collect::<Vec<_>>()})
                    }
                    _ => issue[key] = value.clone(),
                }
            }
            issue["updatedAt"] = json!(T1);
            StubReply::data(json!({"issueUpdate":{"success":true,"issue":issue}}))
        }
        Some("IssueCreate") => {
            let input = &req.variables()["input"];
            let id = input["id"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("created-{}", state.issues.len() + 1));
            // Linear refuses a second create with an id it has (Q-R22).
            if state.issues.iter().any(|issue| issue["id"] == id.as_str()) {
                return StubReply::error("CONFLICT", "stub: an issue with this id exists");
            }
            let made_in = input["teamId"].as_str().unwrap_or(TEAM);
            let made_key = if made_in == HOL { "PER-99" } else { "ENG-99" };
            let mut issue = stub::issue_node(&id, made_key, made_in, T1);
            issue["title"] = input["title"].clone();
            issue["description"] = input.get("description").cloned().unwrap_or(json!(""));
            issue["createdAt"] =
                json!(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
            issue["updatedAt"] = issue["createdAt"].clone();
            if !input["projectId"].is_null() {
                issue["project"] = json!({"id":input["projectId"]});
            }
            state.issues.push(issue.clone());
            if state.lose_create_reply {
                StubReply::status(503)
            } else {
                StubReply::data(json!({"issueCreate":{"success":true,"issue":issue}}))
            }
        }
        _ => panic!("unexpected tracker operation {:?}", req.operation()),
    }
}

/// S-F1: the in-process sync lock is per vault root since 2026-11-13 (`LOCKS` in
/// tracker_sync.rs), so tests on separate temp roots no longer contend, and
/// `work_tests` and `status_tests` (one temp root per test, no real sync) need no
/// gate (R13, 2026-11-15). This module's fixtures still take turns on this gate:
/// they share the process-wide `WAKE`/`PENDING` flags and the stub-server timing
/// its sync assertions read. Not poisonable.
static TEST_SYNC_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
    remote: Arc<StdMutex<RemoteState>>,
    server: StubServer,
    /// Declared last so it drops last: the lock is released after the stub
    /// server and the vault are gone.
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    fn seed_project(&self) {
        let mut state = self.remote.lock().unwrap();
        state.projects.push(json!({"id":"p1","identifier":"P-ENG-1","url":"https://linear.app/example/project/p1","name":"Fixture project","description":"Remote project description.","status":{"name":"Started","type":"started"},"startDate":null,"targetDate":"2026-12-01","updatedAt":T0,"progress":0.25,"lead":null,"teams":{"nodes":[{"id":TEAM}]},"externalLinks":{"nodes":[]}}));
        state.issues[0]["project"] = json!({"id":"p1"});
    }
    fn project_text(&self) -> String {
        fs::read_to_string(self.root().join(&self.ledger().records["p1"].path)).unwrap()
    }
    fn edit_project(&self, key: &str, value: &str) {
        let path = self.ledger().records["p1"].path.clone();
        let text = frontmatter::patch(
            &self.project_text(),
            &[(key, frontmatter::KeyEdit::Quoted(value.into()))],
            None,
        )
        .unwrap();
        vault::write_note(self.root(), &path, &text).unwrap();
    }
    async fn new() -> Self {
        let serial = TEST_SYNC_SERIAL.lock().await;
        let builder = &mut tempfile::Builder::new();
        builder.prefix("vault-tracker-");
        let dir = match std::env::var_os("HOLLOW_TRACKER_FIXTURE_DIR") {
            Some(parent) => {
                fs::create_dir_all(&parent).unwrap();
                builder.tempdir_in(parent).unwrap()
            }
            None => builder.tempdir().unwrap(),
        };
        let config = serde_json::from_value(json!({"vaultPath":dir.path(),"notion":{"twoWaySync":false},"tracker":{"provider":"linear","linear":{"teamId":TEAM,"teamKey":"ENG"}}})).unwrap();
        let mut issue = stub::issue_node("i1", "ENG-1", TEAM, T0);
        issue["description"] = json!("Remote description.");
        issue["attachments"] = json!({"nodes":[]});
        issue["comments"] = json!({"nodes":[]});
        let remote = Arc::new(StdMutex::new(RemoteState {
            issues: vec![issue],
            ..Default::default()
        }));
        let handler = remote.clone();
        let server = stub::serve(move |req| respond(req, &mut handler.lock().unwrap())).await;
        Self {
            _dir: dir,
            config,
            remote,
            server,
            _serial: serial,
        }
    }
    /// The same fixture with the personal team set up and one issue in it.
    async fn two_teams() -> Self {
        let mut f = Self::new().await;
        f.config = serde_json::from_value(json!({"vaultPath":f.root(),"notion":{"twoWaySync":false},"tracker":{"provider":"linear","linear":{"teamId":TEAM,"teamKey":"ENG","personalTeamId":HOL,"personalTeamKey":"PER"}}})).unwrap();
        let mut own = stub::issue_node("h1", "PER-1", HOL, T0);
        own["description"] = json!("Only Alex and Hollow.");
        own["attachments"] = json!({"nodes":[]});
        own["comments"] = json!({"nodes":[]});
        f.remote.lock().unwrap().issues.push(own);
        f
    }
    fn adapter(&self) -> LinearAdapter {
        LinearAdapter::with_url(&self.server.url, TEAM, "lin_api_fixture_only_never_real").unwrap()
    }
    async fn sync(&self) -> Result<TrackerSyncOutcome, String> {
        sync_with(&self.config, &self.adapter()).await
    }
    fn root(&self) -> &Path {
        &self.config.vault_path
    }
    fn ledger(&self) -> Ledger {
        load(self.root(), "linear", TEAM).unwrap()
    }
    fn task(&self) -> String {
        fs::read_to_string(self.root().join(&self.ledger().records["i1"].path)).unwrap()
    }
    fn replace_task(&self, text: &str) {
        let path = self.ledger().records["i1"].path.clone();
        vault::write_note(self.root(), &path, text).unwrap();
    }
    fn local_edit(&self, key: &str, value: &str) {
        let next = frontmatter::patch(
            &self.task(),
            &[(key, frontmatter::KeyEdit::Quoted(value.into()))],
            None,
        )
        .unwrap();
        self.replace_task(&next);
    }
    fn updates(&self) -> usize {
        self.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("IssueUpdate"))
            .count()
    }
}

#[test]
fn validates_mirror_paths_and_rejects_hidden_or_cross_kind_destinations() {
    assert!(path_kind("task", "tasks/kept-path.md"));
    assert!(path_kind("project", "workspaces/project/PROJECT.md"));
    for path in [
        "../tasks/x.md",
        "tasks/../x.md",
        "tasks/.hidden/x.md",
        "/tasks/x.md",
        "workspaces/x/WORKSPACE.md",
    ] {
        assert!(!path_kind("task", path), "{path}");
    }
}

#[tokio::test]
async fn pull_creates_the_file_with_all_details_and_a_second_pass_does_not_rewrite_it() {
    let f = Fixture::new().await;
    let report = f.sync().await.unwrap();
    assert_eq!(report.pulled, 1);
    let before = f.task();
    let fm = frontmatter::parse(&before);
    assert_eq!(fm.get("tracker_id"), Some("i1"));
    assert_eq!(fm.get("status_type"), Some("unstarted"));
    for key in [
        "estimate",
        "cycle",
        "milestone",
        "creator",
        "created_at",
        "updated_at",
        "sub_issues",
        "blocked_by",
        "blocking",
        "related",
        "links",
        "attachments",
        "comment_count",
    ] {
        assert!(fm.get(key).is_some(), "{key}");
    }
    assert!(before.contains("Remote description."));
    let report = f.sync().await.unwrap();
    assert_eq!(report.pulled, 0);
    assert_eq!(f.task(), before);
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn a_local_edit_pushes_only_its_field_and_keeps_vault_notes_and_unknown_lists() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let with_notes = frontmatter::patch(
        &f.task(),
        &[(
            "unknown",
            frontmatter::KeyEdit::Block(vec!["unknown:".into(), "  - keep exactly".into()]),
        )],
        None,
    )
    .unwrap();
    f.replace_task(&format!(
        "{with_notes}\n## Personal notes\nKeep these exact bytes.\n"
    ));
    f.local_edit("title", "A local title");
    let report = f.sync().await.unwrap();
    assert_eq!(report.pushed, 1);
    let requests = f.server.requests();
    let update = requests
        .iter()
        .find(|r| r.operation().as_deref() == Some("IssueUpdate"))
        .unwrap();
    assert_eq!(
        update.variables()["input"],
        json!({"title":"A local title"})
    );
    assert!(f
        .task()
        .ends_with("## Personal notes\nKeep these exact bytes.\n"));
    assert!(f.task().contains("unknown:\n  - keep exactly\n"));
    assert!(f.ledger().outbox.is_empty());
}

#[tokio::test]
async fn both_changed_keeps_remote_and_records_the_local_conflict() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.local_edit("title", "Local edit");
    {
        let mut remote = f.remote.lock().unwrap();
        remote.issues[0]["title"] = json!("Remote edit");
        remote.issues[0]["updatedAt"] = json!(T1);
    }
    let report = f.sync().await.unwrap();
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.pushed, 0);
    assert_eq!(
        frontmatter::parse(&f.task()).get("title"),
        Some("Remote edit")
    );
    assert!(f.task().contains("your edit to title was not applied"));
    assert_eq!(
        f.ledger().records["i1"].conflicts[0].local,
        json!("Local edit")
    );
    assert_eq!(f.updates(), 0);
    let waiting =
        crate::needs_you::list(f.root(), Vec::new(), Some(&["conflict".into()]), None, None)
            .unwrap();
    assert!(waiting
        .items
        .iter()
        .any(|i| i.id == "conflict:tracker:i1:title"
            && i.path.as_deref() == Some("tasks/tracker-i1.md")));
    edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("priority".into(), json!(2))]),
            expected: mirror::local_values(&f.task()).unwrap(),
        },
    )
    .await
    .unwrap();
    assert!(f.task().contains("Local edit"));
    assert_eq!(open_conflicts(f.root()).unwrap().len(), 1);
    let current = mirror::local_values(&f.task()).unwrap();
    edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("title".into(), json!("Remote edit"))]),
            expected: current,
        },
    )
    .await
    .unwrap();
    f.sync().await.unwrap();
    assert!(open_conflicts(f.root()).unwrap().is_empty());
}

#[tokio::test]
async fn a_disjoint_remote_change_and_local_edit_merge_without_a_conflict() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.local_edit("title", "Local edit");
    {
        let mut remote = f.remote.lock().unwrap();
        remote.issues[0]["dueDate"] = json!("2026-11-20");
        remote.issues[0]["updatedAt"] = json!(T1);
    }
    let report = f.sync().await.unwrap();
    assert_eq!((report.pushed, report.conflicts), (1, 0));
    assert_eq!(frontmatter::parse(&f.task()).get("due"), Some("2026-11-20"));
}

#[tokio::test]
async fn archived_and_trashed_files_are_marked_and_never_deleted() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.local_edit("title", "Keep my pending archive edit");
    {
        let mut remote = f.remote.lock().unwrap();
        remote.issues[0]["archivedAt"] = json!(T1);
        remote.issues[0]["trashed"] = json!(true);
        remote.issues[0]["updatedAt"] = json!(T1);
    }
    f.sync().await.unwrap();
    assert_eq!(
        frontmatter::parse(&f.task()).get("tracker_state"),
        Some("archived")
    );
    assert!(f.task().contains("Keep my pending archive edit"));
    assert_eq!(
        f.ledger().records["i1"].conflicts[0].local,
        json!("Keep my pending archive edit")
    );
    assert_eq!(f.updates(), 0);
    assert!(view(&f.config, &TrackerViewQuery::default())
        .unwrap()
        .rows
        .is_empty());
    assert_eq!(
        view(
            &f.config,
            &TrackerViewQuery {
                include_archived: true,
                ..Default::default()
            }
        )
        .unwrap()
        .rows
        .len(),
        1
    );
}

#[tokio::test]
async fn another_teams_node_never_reaches_the_vault_and_a_moved_issue_stays_stopped() {
    let f = Fixture::new().await;
    f.remote.lock().unwrap().leak_scope = true;
    f.remote
        .lock()
        .unwrap()
        .issues
        .push(stub::issue_node("outside", "OT-1", OTHER, T1));
    f.sync().await.unwrap();
    assert!(!f.ledger().records.contains_key("outside"));
    assert!(!f.root().join("tasks/tracker-outside.md").exists());
    // A local edit makes the next pass re-read its target. Once it moved
    // outside the team, the scoped detail query sees nothing.
    f.local_edit("title", "Do not push this");
    f.remote.lock().unwrap().issues[0]["team"]["id"] = json!(OTHER);
    f.sync().await.unwrap();
    assert_eq!(
        frontmatter::parse(&f.task()).get("tracker_state"),
        Some("moved")
    );
    f.sync().await.unwrap();
    assert_eq!(f.ledger().records["i1"].state, "moved");
    assert!(f.task().contains("Do not push this"));
    assert_eq!(f.ledger().records["i1"].conflicts.len(), 1);
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn rate_limited_and_offline_edits_persist_then_drain_after_restart() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.local_edit("title", "Queue me offline");
    f.remote.lock().unwrap().fail = Some(503);
    assert!(f.sync().await.is_err());
    assert_eq!(
        f.ledger().outbox["i1"].values["title"],
        json!("Queue me offline")
    );
    let calls = f.server.requests().len();
    assert!(f.sync().await.is_err());
    assert_eq!(
        f.server.requests().len(),
        calls,
        "engine backoff survives a new adapter"
    );
    let mut state = f.ledger();
    state.retry_at = None;
    save(f.root(), &state).unwrap();
    f.remote.lock().unwrap().fail = Some(429);
    assert!(f.sync().await.is_err());
    assert!(f.ledger().retry_at.is_some());
    let mut state = f.ledger();
    state.retry_at = None;
    save(f.root(), &state).unwrap();
    f.remote.lock().unwrap().fail = None;
    let report = f.sync().await.unwrap();
    assert_eq!(report.pushed, 1);
    assert!(f.ledger().outbox.is_empty());
    assert_eq!(f.ledger().last_error, None);
}

#[tokio::test]
async fn comment_only_updates_arrive_and_read_only_edits_are_restored_without_a_push() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let before_issue_cursor = f.ledger().cursor_issues;
    f.local_edit("creator", "Not the creator");
    let body = "A comment that doesn't update the issue.\n<!-- /tracker:comments -->";
    f.remote.lock().unwrap().comments = vec![
        json!({"id":"comment-1","body":body,"createdAt":T1,"updatedAt":T1,"editedAt":null,"user":{"id":"user-1","name":"Fixture Person"},"parent":null,"issue":{"id":"i1","team":{"id":TEAM}}}),
    ];
    f.sync().await.unwrap();
    assert_eq!(f.ledger().cursor_issues, before_issue_cursor);
    assert_eq!(f.ledger().cursor_comments.as_deref(), Some(T1));
    assert_eq!(frontmatter::parse(&f.task()).get("creator"), Some(""));
    assert_eq!(
        frontmatter::parse(&f.task()).get("comment_count"),
        Some("1")
    );
    assert!(f
        .task()
        .contains("A comment that doesn't update the issue."));
    assert!(mirror::block(&f.task(), "comments")
        .unwrap()
        .unwrap()
        .contains(body));
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn a_lost_create_reply_is_adopted_after_restart_without_a_second_create() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.remote.lock().unwrap().lose_create_reply = true;
    let input = NewIssue {
        title: "Journaled fixture task".into(),
        ..Default::default()
    };
    let created = api::create_with(&f.config, &f.adapter(), "request-1", input.clone())
        .await
        .unwrap();
    assert!(created.queued);
    assert!(f.ledger().creates["request-1"].attempted);
    // A fresh adapter and ledger, no in-memory completion to rely on.
    f.sync().await.unwrap();
    assert!(f.ledger().creates["request-1"].issue.is_some());
    let retry = api::create_with(&f.config, &f.adapter(), "request-1", input)
        .await
        .unwrap();
    assert!(!retry.queued);
    let creates = f
        .server
        .requests()
        .iter()
        .filter(|r| r.operation().as_deref() == Some("IssueCreate"))
        .count();
    assert_eq!(creates, 1);
    assert_eq!(
        f.ledger()
            .records
            .values()
            .filter(|r| r.key == "ENG-99")
            .count(),
        1
    );
}

#[tokio::test]
async fn the_first_pass_over_an_m4_style_vault_changes_no_mirror_bytes() {
    let f = Fixture::new().await;
    f.remote.lock().unwrap().projects.push(json!({"id":"p1","identifier":"P-ENG-1","url":"https://linear.app/example/project/p1","name":"Fixture project","description":"Remote project description.","status":{"name":"Started","type":"started"},"startDate":null,"targetDate":"2026-12-01","updatedAt":T0,"progress":0.25,"lead":null,"teams":{"nodes":[{"id":TEAM}]},"externalLinks":{"nodes":[]}}));
    f.remote.lock().unwrap().issues[0]["project"] = json!({"id":"p1"});
    let project = f.adapter().projects_changed(None).await.unwrap().remove(0);
    let project_path = "workspaces/notion-legacy/PROJECT.md";
    let project_text = mirror::project("---\nnotion_id: legacy-project\nrequested_by:\n  - person-kept\n---\n## Notion description (final, 2026-11-08)\nKept archive text.\n", "linear", &project).unwrap();
    vault::create_note(f.root(), project_path, &project_text).unwrap();
    let workspace_path = "workspaces/notion-legacy/WORKSPACE.md";
    let workspace_text = "---\ntitle: Fixture project\ntracker_project: p1\n---\n## Decisions\nKeep all decisions.\n";
    vault::create_note(f.root(), workspace_path, workspace_text).unwrap();
    let issue = f.adapter().issue("i1").await.unwrap();
    let base = mirror::values(
        &issue,
        &BTreeMap::from([("p1".into(), project.clone())]),
        &BTreeMap::new(),
    );
    let legacy = "---\nnotion_id: legacy-id\nnotion_task_id: TASK-62\nmeeting: library/records/meetings/kept.md\nunknown:\n  - keep this list\n---\n## Vault notes\nKeep me exactly.\n";
    let text = mirror::task(
        legacy,
        "linear",
        &issue,
        &base,
        Some(project_path),
        "active",
        false,
        "",
    )
    .unwrap();
    let path = "tasks/an-existing-name.md";
    vault::create_note(f.root(), path, &text).unwrap();
    let mut state = Ledger::new("linear", TEAM);
    state.records.insert(
        "p1".into(),
        Record {
            id: "p1".into(),
            key: "P-ENG-1".into(),
            kind: "project".into(),
            path: project_path.into(),
            workspace_id: Some("notion-legacy".into()),
            base: Values::new(),
            conflicts: Vec::new(),
            error: None,
            remote_missing: false,
            state: active(),
            issue: None,
            project: Some(project),
        },
    );
    state.records.insert(
        issue.id.clone(),
        Record {
            id: issue.id.clone(),
            key: issue.key.clone(),
            kind: "task".into(),
            path: path.into(),
            workspace_id: None,
            base,
            conflicts: Vec::new(),
            error: None,
            remote_missing: false,
            state: active(),
            issue: Some(issue),
            project: None,
        },
    );
    save(f.root(), &state).unwrap();
    let before = fs::metadata(f.root().join(path))
        .unwrap()
        .modified()
        .unwrap();
    let report = f.sync().await.unwrap();
    assert_eq!(report.pulled, 0);
    assert_eq!(report.pushed, 0);
    assert_eq!(fs::read_to_string(f.root().join(path)).unwrap(), text);
    assert_eq!(
        fs::metadata(f.root().join(path))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    assert!(!f.root().join("tasks/tracker-i1.md").exists());
    assert_eq!(
        fs::read_to_string(f.root().join(project_path)).unwrap(),
        project_text
    );
    assert_eq!(
        fs::read_to_string(f.root().join(workspace_path)).unwrap(),
        workspace_text
    );
    assert!(!f.root().join("workspaces/linear-p1").exists());
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn tracker_workspace_materializes_context_and_keeps_its_id_after_rename() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let state = f.ledger();
    let rel = state.records["p1"].path.clone();
    assert_eq!(rel, "workspaces/linear-fixture-project-p1/PROJECT.md");
    let id = state.records["p1"].workspace_id.as_deref().unwrap();
    let workspace = crate::workspaces::summary(f.root(), id).unwrap();
    assert_eq!(crate::workspaces::origin_kind(&workspace), "tracker");
    assert!(crate::workspaces::update_meta(f.root(), id, Some("Rename locally"), None).is_err());
    assert!(crate::workspaces::update_meta(f.root(), id, None, Some("archived")).is_err());
    assert!(crate::workspaces::trash(f.root(), id).is_err());
    assert!(crate::run_attribution::set_workspace(f.root(), id, Some("Operations"), None).is_err());
    let local = crate::workspaces::create(f.root(), "Other workspace", "Fixture").unwrap();
    assert!(crate::workspace_merge::plan(f.root(), id, &local.id).is_err());
    let brief_path = format!("workspaces/{id}/WORKSPACE.md");
    let before = fs::read_to_string(f.root().join(&brief_path)).unwrap();
    let brief = frontmatter::parse(&before);
    assert_eq!(brief.get("tracker_project"), Some("p1"));
    assert_eq!(brief.get("scope"), Some("work"));
    vault::create_note(
        f.root(),
        &format!("workspaces/{id}/NOTION.md"),
        "Retained archive context",
    )
    .unwrap();
    let envelope = crate::workspaces::context(f.root(), &f.root().join(format!("workspaces/{id}")))
        .unwrap()
        .unwrap();
    assert!(envelope.contains("TRACKER.md"));
    assert!(envelope.contains("Retained archive context"));
    fs::rename(
        f.root().join(format!("workspaces/{id}")),
        f.root().join("workspaces/renamed-folder"),
    )
    .unwrap();
    assert!(crate::workspaces::tracker_managed(
        f.root(),
        "renamed-folder"
    ));
    assert!(
        crate::workspaces::update_meta(f.root(), "renamed-folder", None, Some("archived")).is_err()
    );
    f.sync().await.unwrap();
    assert_eq!(
        f.ledger().records["p1"].path,
        "workspaces/renamed-folder/PROJECT.md"
    );
    assert_eq!(
        f.ledger().records["p1"].workspace_id.as_deref(),
        Some("renamed-folder")
    );
    assert!(!f.root().join(format!("workspaces/{id}")).exists());
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn tracker_workspace_refuses_an_unbound_destination_on_every_retry() {
    let f = Fixture::new().await;
    f.seed_project();
    let rel = "workspaces/linear-fixture-project-p1/WORKSPACE.md";
    let authored = "---\nhollow_workspace: 1\ntitle: My own workspace\nstatus: active\n---\nKeep this authored brief.\n";
    vault::create_note(f.root(), rel, authored).unwrap();
    for _ in 0..2 {
        assert!(!f.sync().await.unwrap().errors.is_empty());
        assert_eq!(fs::read_to_string(f.root().join(rel)).unwrap(), authored);
        assert!(!f
            .root()
            .join("workspaces/linear-fixture-project-p1/PROJECT.md")
            .exists());
    }
    assert_eq!(f.ledger().records["p1"].state, "initializing");
}

#[tokio::test]
async fn track_in_links_the_existing_workspace_and_the_pass_adopts_it_without_writing_back() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let dir = f.root().join("workspaces/vault-only");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("WORKSPACE.md"),
        "---\nhollow_workspace: 1\ntitle: \"Vault only\"\n---\n\n# Vault only\n\nMy brief.\n",
    )
    .unwrap();
    fs::write(
        dir.join("PROJECT.md"),
        "---\ntitle: \"Vault only\"\nstatus: \"Dropped\"\ndepartment: \"AI\"\n---\n\nNotes.\n",
    )
    .unwrap();
    let input = TrackerNewProject {
        name: "Vault only".into(),
        workspace_id: Some("vault-only".into()),
        ..Default::default()
    };
    let receipt = api::create_project_with(&f.config, &f.adapter(), input.clone(), None)
        .await
        .unwrap();
    let id = receipt.id.unwrap();
    let linked = fs::read_to_string(dir.join("PROJECT.md")).unwrap();
    let fm = frontmatter::parse(&linked);
    assert_eq!(fm.get("tracker_project"), Some(id.as_str()));
    assert_eq!(fm.get("department"), Some("AI"));
    f.sync().await.unwrap();
    let state = f.ledger();
    assert_eq!(state.records[&id].path, "workspaces/vault-only/PROJECT.md");
    assert!(fs::read_to_string(dir.join("WORKSPACE.md"))
        .unwrap()
        .contains("My brief."));
    assert!(fs::read_to_string(dir.join("PROJECT.md"))
        .unwrap()
        .contains("department: \"AI\""));
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectUpdate"))
            .count(),
        0,
        "the linked file already matches the tracker"
    );
    // Tracked once: a second Track in is refused before any request.
    let creates = || {
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectCreate"))
            .count()
    };
    let before = creates();
    assert!(
        api::create_project_with(&f.config, &f.adapter(), input, Some("again"))
            .await
            .unwrap_err()
            .contains("already tracked")
    );
    assert_eq!(creates(), before);
}

#[tokio::test]
async fn a_track_in_cut_short_after_the_create_is_finished_by_the_next_pass() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let dir = f.root().join("workspaces/cut-short");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("WORKSPACE.md"),
        "---\nhollow_workspace: 1\ntitle: \"Cut short\"\n---\n\n# Cut short\n",
    )
    .unwrap();
    let before = "---\ntitle: \"Cut short\"\ntype: \"Report\"\n---\n\nNotes.\n";
    fs::write(dir.join("PROJECT.md"), before).unwrap();
    let input = TrackerNewProject {
        name: "Cut short".into(),
        workspace_id: Some("cut-short".into()),
        ..Default::default()
    };
    let id = api::create_project_with(&f.config, &f.adapter(), input, Some("cut"))
        .await
        .unwrap()
        .id
        .unwrap();
    // As if Hollow stopped right after the create was journaled.
    let mut state = f.ledger();
    state.records.remove(&id);
    save(f.root(), &state).unwrap();
    fs::write(dir.join("PROJECT.md"), before).unwrap();
    f.sync().await.unwrap();
    let state = f.ledger();
    assert_eq!(state.records[&id].path, "workspaces/cut-short/PROJECT.md");
    let mirrors = state
        .records
        .values()
        .filter(|r| r.project.as_ref().is_some_and(|p| p.id == id))
        .count();
    assert_eq!(mirrors, 1, "one tracker project, one mirror");
    let text = fs::read_to_string(dir.join("PROJECT.md")).unwrap();
    assert!(text.contains("type: \"Report\"") && text.contains(&id));
    // A second pass is quiet.
    f.sync().await.unwrap();
}

#[tokio::test]
async fn tracker_workspace_create_receipts_wait_for_the_allocated_mirror() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let input = TrackerNewProject {
        name: "Same created name".into(),
        ..Default::default()
    };
    for key in ["create-one", "create-two"] {
        let receipt = api::create_project_with(&f.config, &f.adapter(), input.clone(), Some(key))
            .await
            .unwrap();
        assert!(!receipt.queued);
        assert!(receipt.id.is_some());
        assert!(
            receipt.path.is_none(),
            "no local mirror has been published yet"
        );
    }
    f.sync().await.unwrap();
    let state = f.ledger();
    let mut paths = BTreeSet::new();
    for key in ["create-one", "create-two"] {
        let receipt = api::create_project_with(&f.config, &f.adapter(), input.clone(), Some(key))
            .await
            .unwrap();
        let id = receipt.id.unwrap();
        let path = receipt.path.unwrap();
        assert_eq!(path, state.records[&id].path);
        assert!(path.contains("linear-same-created-name-new-proj"));
        assert!(f.root().join(&path).exists());
        assert!(paths.insert(path));
    }
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectCreate"))
            .count(),
        2
    );
}

#[tokio::test]
async fn tracker_workspace_gardener_preserves_tracker_tasks_without_notion_ids() {
    let f = Fixture::new().await;
    f.remote.lock().unwrap().issues[0]["title"] = json!("[Hollow test] linked tracker fixture");
    f.sync().await.unwrap();
    let tasks = crate::vault_tasks::list(f.root());
    let task = tasks.iter().find(|t| t.tracker.is_some()).unwrap();
    assert!(task.notion_id.is_none());
    let scan = crate::gardener::scan(f.root(), Utc::now()).unwrap();
    let path = task.url.strip_prefix("vault:").unwrap();
    let item = scan.items.iter().find(|t| t.target == path).unwrap();
    assert_eq!(item.action, crate::gardener::GardenAction::Manual);
    assert!(!item.title.contains("Notion"));
    assert!(f.root().join(path).exists());
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn tracker_workspace_short_id_collisions_keep_distinct_leased_destinations() {
    let f = Fixture::new().await;
    f.seed_project();
    {
        let mut remote = f.remote.lock().unwrap();
        remote.projects[0]["id"] = json!("abcdefgh-one");
        let mut second = remote.projects[0].clone();
        second["id"] = json!("abcdefgh-two");
        remote.projects.push(second);
        remote.issues[0]["project"] = Value::Null;
    }
    for _ in 0..2 {
        let report = f.sync().await.unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let state = f.ledger();
        let one = &state.records["abcdefgh-one"];
        let two = &state.records["abcdefgh-two"];
        assert_eq!(
            one.path,
            "workspaces/linear-fixture-project-abcdefgh/PROJECT.md"
        );
        assert_eq!(
            two.path,
            "workspaces/linear-fixture-project-abcdefgh-abcdefgh-two/PROJECT.md"
        );
        for record in [one, two] {
            assert_eq!(record.state, "active");
            let text = fs::read_to_string(f.root().join(&record.path)).unwrap();
            assert_eq!(
                frontmatter::parse(&text).get("tracker_project"),
                Some(record.id.as_str())
            );
            assert!(f
                .root()
                .join(record.path.replace("PROJECT.md", "TRACKER.md"))
                .exists());
        }
    }
    {
        let mut remote = f.remote.lock().unwrap();
        let mut third = remote.projects[0].clone();
        third["id"] = json!("ABCDEFGH");
        remote.projects.push(third);
    }
    // Advance the scheduled project refresh before checking a new project.
    let mut refresh = f.ledger();
    refresh.projects_at = None;
    save(f.root(), &refresh).unwrap();
    assert!(f.sync().await.unwrap().errors.is_empty());
    let state = f.ledger();
    assert_eq!(
        state.records["abcdefgh-one"].path,
        "workspaces/linear-fixture-project-abcdefgh/PROJECT.md"
    );
    assert_ne!(
        state.records["ABCDEFGH"].path.to_lowercase(),
        state.records["abcdefgh-one"].path.to_lowercase()
    );
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectUpdate"))
            .count(),
        0
    );
}

#[tokio::test]
async fn tracker_workspace_recovers_only_an_unfinished_initial_publication() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let mut state = f.ledger();
    let rel = state.records["p1"].path.clone();
    state.records.get_mut("p1").unwrap().state = "initializing".into();
    state.projects_at = None;
    save(f.root(), &state).unwrap();
    fs::remove_file(f.root().join(&rel)).unwrap();
    f.remote.lock().unwrap().projects[0]["name"] = json!("New provider name");
    f.sync().await.unwrap();
    assert_eq!(f.ledger().records["p1"].state, "active");
    assert!(f.project_text().contains("New provider name"));
    assert_eq!(f.ledger().records["p1"].path, rel);
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectUpdate"))
            .count(),
        0
    );
    fs::remove_file(f.root().join(&rel)).unwrap();
    assert!(f.sync().await.unwrap_err().contains("missing"));
    assert!(!f.root().join(&rel).exists());
}

#[tokio::test]
async fn tracker_workspace_preserves_authored_generated_context_destination() {
    let f = Fixture::new().await;
    f.seed_project();
    let path = "workspaces/linear-fixture-project-p1/TRACKER.md";
    vault::create_note(f.root(), path, "Authored tracker notes").unwrap();
    assert!(!f.sync().await.unwrap().errors.is_empty());
    assert_eq!(
        fs::read_to_string(f.root().join(path)).unwrap(),
        "Authored tracker notes"
    );
    assert!(!f
        .root()
        .join("workspaces/linear-fixture-project-p1/WORKSPACE.md")
        .exists());
}

#[test]
fn tracker_workspace_ordinary_similar_title_remains_editable() {
    let root = tempfile::tempdir().unwrap();
    let local =
        crate::workspaces::create(root.path(), "Linear project notes", "Local fixture").unwrap();
    assert!(!crate::workspaces::tracker_managed(root.path(), &local.id));
    crate::workspaces::update_meta(root.path(), &local.id, Some("Ordinary notes"), None).unwrap();
    assert!(!crate::workspaces::is_notion_mirror(
        "notion-linked-edits-e676455ab944cf3b"
    ));
    assert!(crate::workspaces::is_notion_mirror(
        "notion-00000000000000000000000000000002"
    ));
}

#[tokio::test]
async fn tracker_workspace_missing_binding_never_selects_a_same_filename() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let fields = BTreeMap::from([
        ("tracker".into(), "linear".into()),
        ("tracker_project".into(), "missing".into()),
        (
            "tracker_project_path".into(),
            "workspaces/missing/PROJECT.md".into(),
        ),
    ]);
    assert!(matches!(
        crate::goal_context::resolve_workspace_project(f.root(), &fields, "Fixture project"),
        crate::goal_context::ProjectLink::Unresolved { .. }
    ));
    assert!(crate::brief_read::project_of(f.root(), &fields).is_none());
    let mut wrong_id = fields;
    wrong_id.insert(
        "tracker_project_path".into(),
        f.ledger().records["p1"].path.clone(),
    );
    assert!(crate::brief_read::project_of(f.root(), &wrong_id).is_none());
}

#[tokio::test]
async fn tracker_workspace_initializing_rechecks_a_file_created_after_load() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let mut state = f.ledger();
    let record = state.records.get_mut("p1").unwrap();
    record.state = "initializing".into();
    let source = record.project.clone().unwrap();
    let rel = record.path.clone();
    let authored = "---\ntitle: Authored after the initial ledger read\nstatus: Started\n---\nKeep this note.\n";
    fs::write(f.root().join(&rel), authored).unwrap();
    let _lease =
        crate::automated_history::tracker_mirror_write_lease(f.root(), &folders(&state, []))
            .unwrap();
    assert!(reconcile_project(
        f.root(),
        &f.adapter(),
        &mut state,
        &source,
        &BTreeMap::new(),
        &rel,
        &mut outcome()
    )
    .await
    .is_err());
    assert_eq!(fs::read_to_string(f.root().join(&rel)).unwrap(), authored);
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectUpdate"))
            .count(),
        0
    );
}

#[tokio::test]
async fn a_change_tied_at_the_cursor_timestamp_is_not_skipped() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.remote.lock().unwrap().issues[0]["title"] = json!("Another update in the same millisecond");
    f.sync().await.unwrap();
    assert_eq!(
        frontmatter::parse(&f.task()).get("title"),
        Some("Another update in the same millisecond")
    );
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn bad_ledgers_fail_closed_before_any_network() {
    let f = Fixture::new().await;
    fs::write(f.root().join(STATE), b"broken").unwrap();
    assert!(f.sync().await.unwrap_err().contains("unreadable"));
    assert_eq!(fs::read(f.root().join(STATE)).unwrap(), b"broken");
    assert!(f.server.requests().is_empty());
}

#[tokio::test]
async fn an_edit_pins_each_changed_field_and_queues_the_local_value() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let current = mirror::local_values(&f.task()).unwrap();
    let values = BTreeMap::from([("title".into(), json!("From the task editor"))]);
    let wrong = TrackerEdit {
        id: "i1".into(),
        values: values.clone(),
        expected: BTreeMap::from([("title".into(), json!("stale"))]),
    };
    assert!(edit(&f.config, wrong)
        .await
        .unwrap_err()
        .contains("changed"));
    assert!(f.ledger().outbox.is_empty());
    let out = edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values,
            expected: current,
        },
    )
    .await
    .unwrap();
    assert!(out.queued);
    assert_eq!(
        view(&f.config, &TrackerViewQuery::default()).unwrap().rows[0].title,
        "From the task editor"
    );
    assert_eq!(
        issue(&f.config, "i1").unwrap().issue.title,
        "From the task editor"
    );
    assert_eq!(
        frontmatter::parse(&f.task()).get("tracker_pending"),
        Some("true")
    );
    f.sync().await.unwrap();
    assert_eq!(f.updates(), 1);
    assert_eq!(
        frontmatter::parse(&f.task()).get("title"),
        Some("From the task editor")
    );
}

#[tokio::test]
async fn successive_offline_edits_keep_every_pending_field_visible_after_restart() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    for values in [
        BTreeMap::from([("title".into(), json!("First offline edit"))]),
        BTreeMap::from([("priority".into(), json!(2))]),
    ] {
        edit(
            &f.config,
            TrackerEdit {
                id: "i1".into(),
                values,
                expected: mirror::local_values(&f.task()).unwrap(),
            },
        )
        .await
        .unwrap();
    }
    // Each read reloads the durable ledger, just as a restarted process does.
    let detail = issue(&f.config, "i1").unwrap();
    assert_eq!(detail.issue.title, "First offline edit");
    assert_eq!(detail.issue.priority, 2);
    assert_eq!(f.ledger().outbox["i1"].values.len(), 2);
    assert_eq!(
        view(&f.config, &TrackerViewQuery::default()).unwrap().rows[0].title,
        "First offline edit"
    );
    f.sync().await.unwrap();
    assert_eq!(f.updates(), 1);
    assert!(f.ledger().outbox.is_empty());
    let detail = issue(&f.config, "i1").unwrap();
    assert_eq!(detail.issue.title, "First offline edit");
    assert_eq!(detail.issue.priority, 2);
    // Reverting every queued change clears both the queue and its file flag.
    for title in ["Temporary offline edit", "First offline edit"] {
        edit(
            &f.config,
            TrackerEdit {
                id: "i1".into(),
                values: BTreeMap::from([("title".into(), json!(title))]),
                expected: mirror::local_values(&f.task()).unwrap(),
            },
        )
        .await
        .unwrap();
    }
    assert!(f.ledger().outbox.is_empty());
    assert_eq!(
        frontmatter::parse(&f.task()).get("tracker_pending"),
        Some("false")
    );
}

#[tokio::test]
async fn picker_ids_and_mirror_names_share_one_comparison_value() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let out = edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("status".into(), json!("state-done"))]),
            expected: BTreeMap::from([("status".into(), json!("state-todo"))]),
        },
    )
    .await
    .unwrap();
    assert!(out.queued);
    assert_eq!(frontmatter::parse(&f.task()).get("status"), Some("Done"));
    assert_eq!(f.ledger().outbox["i1"].values["status"], json!("Done"));
    {
        let mut remote = f.remote.lock().unwrap();
        remote.issues[0]["state"] = json!({"id":"state-done","name":"Done","type":"completed"});
        remote.issues[0]["updatedAt"] = json!(T1);
    }
    let report = f.sync().await.unwrap();
    assert_eq!((report.pushed, report.conflicts), (0, 0));
    assert!(open_conflicts(f.root()).unwrap().is_empty());
}

#[tokio::test]
async fn project_edits_use_the_same_outbox_and_keep_their_notes() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let path = f.ledger().records["p1"].path.clone();
    vault::write_note(
        f.root(),
        &path,
        &format!(
            "{}\n## Private context\nKeep this exactly.\n",
            f.project_text()
        ),
    )
    .unwrap();
    f.edit_project("title", "Local project title");
    f.remote.lock().unwrap().fail = Some(503);
    assert!(f.sync().await.is_err());
    assert_eq!(
        f.ledger().outbox["p1"].values["title"],
        json!("Local project title")
    );
    let mut state = f.ledger();
    state.retry_at = None;
    save(f.root(), &state).unwrap();
    f.remote.lock().unwrap().fail = None;
    let report = f.sync().await.unwrap();
    assert_eq!(report.pushed, 1);
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("title"),
        Some("Local project title")
    );
    assert!(f
        .project_text()
        .ends_with("## Private context\nKeep this exactly.\n"));
    assert_eq!(
        frontmatter::parse(&f.task()).get("project"),
        Some("Local project title")
    );
    let requests = f.server.requests();
    let write = requests
        .iter()
        .find(|r| r.operation().as_deref() == Some("ProjectUpdate"))
        .unwrap();
    assert_eq!(
        write.variables()["input"],
        json!({"name":"Local project title"})
    );
    assert!(f.ledger().outbox.is_empty());
}

#[tokio::test]
async fn project_conflicts_and_unsupported_local_status_edits_keep_both_values() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    f.edit_project("title", "Local project title");
    {
        let mut state = f.remote.lock().unwrap();
        state.projects[0]["name"] = json!("Remote project title");
        state.projects[0]["updatedAt"] = json!(T1);
    }
    let report = f.sync().await.unwrap();
    assert_eq!((report.pushed, report.conflicts), (0, 1));
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("title"),
        Some("Remote project title")
    );
    assert!(f.project_text().contains("Local project title"));
    f.edit_project("status", "A status with no known id");
    let report = f.sync().await.unwrap();
    assert_eq!(report.pushed, 0);
    assert!(f.project_text().contains("A status with no known id"));
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("status"),
        Some("Started")
    );
    assert!(open_conflicts(f.root())
        .unwrap()
        .iter()
        .any(|c| c.kind == "project"));
}

#[tokio::test]
async fn unsafe_paths_and_cross_team_create_receipts_are_refused_before_reads() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let before = f.task();
    let calls = f.server.requests().len();
    let mut state = f.ledger();
    state.records.get_mut("i1").unwrap().path = "workspaces/another/WORKSPACE.md".into();
    save(f.root(), &state).unwrap();
    assert!(f.sync().await.unwrap_err().contains("unsafe"));
    assert_eq!(f.server.requests().len(), calls);
    state.records.get_mut("i1").unwrap().path = "tasks/tracker-i1.md".into();
    let mut outside = state.records["i1"].issue.clone().unwrap();
    outside.team_id = OTHER.into();
    state.creates.insert(
        "bad".into(),
        Create {
            agent_key: "bad".into(),
            input: NewIssue {
                title: "Fixture only".into(),
                ..Default::default()
            },
            at: T0.into(),
            attempted: true,
            issue: Some(outside),
            error: None,
            issue_id: None,
            resends: 0,
        },
    );
    save(f.root(), &state).unwrap();
    assert!(f.sync().await.unwrap_err().contains("create journal"));
    assert_eq!(f.server.requests().len(), calls);
    assert_eq!(
        fs::read_to_string(f.root().join("tasks/tracker-i1.md")).unwrap(),
        before
    );
}

#[tokio::test]
async fn a_second_editor_request_recovers_unwritten_intent_and_keeps_acknowledgements() {
    for revert in [false, true] {
        let f = Fixture::new().await;
        f.sync().await.unwrap();
        let baseline = mirror::local_values(&f.task()).unwrap();
        if revert {
            edit(
                &f.config,
                TrackerEdit {
                    id: "i1".into(),
                    values: BTreeMap::from([("title".into(), json!("Already reverted"))]),
                    expected: baseline.clone(),
                },
            )
            .await
            .unwrap();
        }
        let before = mirror::local_values(&f.task()).unwrap();
        let mut after = before.clone();
        after.insert(
            "title".into(),
            if revert {
                baseline["title"].clone()
            } else {
                json!("Only in the first journal")
            },
        );
        let mut state = f.ledger();
        state
            .records
            .get_mut("i1")
            .unwrap()
            .conflicts
            .push(Conflict {
                field: "title".into(),
                local: json!("Previously rejected title"),
                remote: baseline["title"].clone(),
                at: T0.into(),
            });
        let mut acknowledged = state.records["i1"].clone();
        acknowledged.conflicts.clear();
        // The first editor request dies after its journal save. Its file
        // has not been written and a second request arrives before capture.
        api::stage_edit(&mut state, &acknowledged, before, after);
        save(f.root(), &state).unwrap();
        edit(
            &f.config,
            TrackerEdit {
                id: "i1".into(),
                values: BTreeMap::from([("status".into(), json!("Done"))]),
                expected: mirror::local_values(&f.task()).unwrap(),
            },
        )
        .await
        .unwrap();
        let detail = issue(&f.config, "i1").unwrap();
        assert_eq!(
            detail.issue.title,
            if revert {
                "Fixture ENG-1"
            } else {
                "Only in the first journal"
            }
        );
        assert_eq!(detail.issue.state.kind, StateKind::Completed);
        assert!(f.ledger().records["i1"].conflicts.is_empty());
        assert!(!f.task().contains("Previously rejected title"));
        f.sync().await.unwrap();
        assert_eq!(f.updates(), 1);
    }
}

#[tokio::test]
async fn reverting_all_pending_fields_survives_a_pre_file_write_crash() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let baseline = mirror::local_values(&f.task()).unwrap();
    edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("title".into(), json!("An edit the person reverted"))]),
            expected: baseline.clone(),
        },
    )
    .await
    .unwrap();
    let mut state = f.ledger();
    let record = state.records["i1"].clone();
    // The editor saves its journal before its file. Stop at that boundary,
    // then recover exactly as the next offline sync capture does.
    assert!(!api::stage_edit(
        &mut state,
        &record,
        mirror::local_values(&f.task()).unwrap(),
        baseline.clone(),
    ));
    save(f.root(), &state).unwrap();
    {
        let _lease =
            crate::automated_history::tracker_mirror_write_lease(f.root(), &folders(&state, []))
                .unwrap();
        capture(f.root(), &mut state).unwrap();
    }
    assert_eq!(mirror::local_values(&f.task()).unwrap(), baseline);
    assert!(f.ledger().outbox.is_empty());
    assert_eq!(
        frontmatter::parse(&f.task()).get("tracker_pending"),
        Some("false")
    );
    assert_eq!(f.sync().await.unwrap().pushed, 0);
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn broker_tracker_edits_keep_owned_blocks_and_queue_all_fields() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.seed_project();
    f.sync().await.unwrap();
    let old = f.task();
    assert!(vault_tasks::apply_update(
        f.root(),
        "tasks/tracker-i1.md",
        &crate::types::UpdateTaskInput {
            description: Some("A legacy parser must not rewrite owned blocks".into()),
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(f.task(), old);
    let result = crate::task_broker::dispatch_companion(
        f.root(),"fixture-device","Fixture phone","fixture-update","update_task",
        &json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":vault_tasks::content_hash(&old),"fields":{
            "title":"Changed through the shared broker","priority":"High","assignee":"user-1","labels":["Feature"],"estimate":3.0,"due":"2026-12-01","dueTime":"09:30"
        }}), Local::now(),
    ).unwrap();
    assert_eq!(result["tracker"]["queued"], true);
    assert_eq!(f.updates(), 0);
    let task = vault_tasks::get(f.root(), "tasks/tracker-i1.md").unwrap();
    assert_eq!(task.priority.as_deref(), Some("P2"));
    assert_eq!(task.due_time.as_deref(), Some("09:30"));
    assert_eq!(task.tracker.as_ref().unwrap().key.as_deref(), Some("ENG-1"));
    let detail = issue(&f.config, "i1").unwrap();
    assert_eq!(detail.issue.title, "Changed through the shared broker");
    assert_eq!(detail.issue.priority, 2);
    assert_eq!(detail.issue.assignee.as_ref().unwrap().id, "user-1");
    assert_eq!(detail.issue.labels, vec!["Feature"]);
    assert_eq!(detail.issue.estimate, Some(3.0));
    assert_eq!(
        mirror::block(&f.task(), "comments").unwrap(),
        mirror::block(&old, "comments").unwrap()
    );
    f.sync().await.unwrap();
    assert_eq!(f.updates(), 1);
    assert!(f.ledger().outbox.is_empty());
}

#[tokio::test]
async fn broker_tracker_uses_immutable_running_writer_policy() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let before = f.task();
    let ledger = fs::read(f.root().join(STATE)).unwrap();
    let mut running = f.config.clone();
    running.tracker.provider = None;
    seams::register_running(&running);
    // A later saved config must never replace the running writer policy.
    seams::register_running(&f.config);
    let refusal = crate::task_broker::dispatch_companion(
        f.root(), "fixture-device", "Fixture phone", "blocked-tracker-edit", "set_status",
        &json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":vault_tasks::content_hash(&before),"status":"Delivered"}),
        Local::now(),
    ).unwrap_err();
    assert!(format!("{refusal:?}").contains("Choose a task tracker"));
    assert_eq!(f.task(), before);
    assert_eq!(fs::read(f.root().join(STATE)).unwrap(), ledger);
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn project_create_lost_reply_and_archive_lost_reply_are_never_resent() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.remote.lock().unwrap().lose_project_reply = true;
    let input = TrackerNewProject {
        name: "One project request".into(),
        ..Default::default()
    };
    for _ in 0..2 {
        assert!(
            api::create_project_with(
                &f.config,
                &f.adapter(),
                input.clone(),
                Some("project-request-1")
            )
            .await
            .unwrap()
            .queued
        );
    }
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectCreate"))
            .count(),
        1
    );
    assert!(api::create_project_with(
        &f.config,
        &f.adapter(),
        TrackerNewProject {
            name: "Different request".into(),
            ..Default::default()
        },
        Some("project-request-1")
    )
    .await
    .is_err());
    f.remote.lock().unwrap().lose_archive_reply = true;
    for _ in 0..2 {
        assert!(
            api::archive_with(&f.config, &f.adapter(), "i1")
                .await
                .unwrap()
                .queued
        );
    }
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("IssueArchive"))
            .count(),
        1
    );
    let mut state = f.ledger();
    state.last_sync = None;
    save(f.root(), &state).unwrap();
    f.sync().await.unwrap();
    assert!(f.ledger().archives["i1"].confirmed);
    assert!(f.root().join("tasks/tracker-i1.md").exists());
}

#[tokio::test]
async fn project_seam_queues_owned_fields_and_parks_unsupported_status() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let rel = f.ledger().records["p1"].path.clone();
    let old = f.project_text();
    {
        let _guard = seams::hold_edit_lock(f.root(), Duration::from_secs(20)).unwrap();
        let state = f.ledger();
        let _lease =
            crate::automated_history::tracker_mirror_write_lease(f.root(), &folders(&state, []))
                .unwrap();
        seams::edit_task(
            f.root(),
            &rel,
            &vault_tasks::content_hash(&old),
            &serde_json::from_value(
                json!({"title":"Queued project title","status":"Unmapped project status","currentState":"Keep this project outcome","priority":"P2","goalUrls":["vault:goals/fixture.md"]}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    assert!(f.ledger().outbox.contains_key("p1"));
    f.sync().await.unwrap();
    assert!(f.project_text().contains("Queued project title"));
    assert!(f.ledger().records["p1"]
        .conflicts
        .iter()
        .any(|c| c.local == json!("Unmapped project status")));
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("status"),
        Some("Started")
    );
    assert_eq!(f.ledger().records["p1"].path, rel);
    let text = f.project_text();
    let fm = frontmatter::parse(&text);
    assert_eq!(fm.get("current_state"), Some("Keep this project outcome"));
    assert_eq!(
        crate::vault_projects::parse_goal_urls(fm.get("goal_urls").unwrap_or("")),
        vec!["vault:goals/fixture.md"]
    );
}

#[tokio::test]
async fn staged_project_and_local_metadata_recover_before_network() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let mut state = f.ledger();
    let record = state.records["p1"].clone();
    let old = f.project_text();
    let before = mirror::local_project_values(&old).unwrap();
    let mut after = before.clone();
    after.insert("title".into(), json!("Recovered project title"));
    api::stage_edit(&mut state, &record, before, after);
    api::stage_local(
        &mut state,
        "p1",
        &old,
        &Values::from([("current_state".into(), json!("Keep local project outcome"))]),
    );
    save(f.root(), &state).unwrap();
    f.remote.lock().unwrap().fail = Some(503);
    assert!(f.sync().await.is_err());
    assert!(f.project_text().contains("Recovered project title"));
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("current_state"),
        Some("Keep local project outcome")
    );
    assert!(f.ledger().outbox["p1"].staged.is_none());
}

#[tokio::test]
async fn staged_local_task_metadata_refuses_a_newer_third_value() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let mut state = f.ledger();
    let record = state.records["i1"].clone();
    let old = f.task();
    let before = mirror::local_values(&old).unwrap();
    api::stage_edit(&mut state, &record, before.clone(), before);
    api::stage_local(
        &mut state,
        "i1",
        &old,
        &Values::from([("due_time".into(), json!("09:30"))]),
    );
    save(f.root(), &state).unwrap();
    f.replace_task(
        &mirror::patch_local(&old, &Values::from([("due_time".into(), json!("10:45"))])).unwrap(),
    );
    let changed = f.task();
    assert!(f.sync().await.unwrap_err().contains("newer edit"));
    assert_eq!(f.task(), changed);
    assert!(f.ledger().outbox["i1"].staged.is_some());
}

#[tokio::test]
async fn tracker_editor_refuses_fields_missing_from_adapter_metadata() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let mut state = f.ledger();
    state.writable_fields = Some(vec![Field::Priority]);
    save(f.root(), &state).unwrap();
    let before = f.task();
    assert_eq!(
        issue(&f.config, "i1").unwrap().writable_fields,
        vec![Field::Priority]
    );
    let refusal = edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("title".into(), json!("Refused title"))]),
            expected: mirror::local_values(&before).unwrap(),
        },
    )
    .await
    .unwrap_err();
    assert!(refusal.contains("does not allow"));
    assert_eq!(f.task(), before);
    assert!(f.ledger().outbox.is_empty());
}

#[tokio::test]
async fn a6_review_old_ledger_edits_offline_but_explicit_empty_stays_read_only() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let mut raw: Value = serde_json::from_slice(&fs::read(f.root().join(STATE)).unwrap()).unwrap();
    raw.as_object_mut().unwrap().remove("writableFields");
    fs::write(f.root().join(STATE), serde_json::to_vec(&raw).unwrap()).unwrap();
    f.remote.lock().unwrap().fail = Some(503);
    assert!(f.sync().await.is_err());
    let before = f.task();
    seams::user_edit(
        &f.config,
        "tasks/tracker-i1.md".into(),
        Some(format!(" {}\n", vault_tasks::content_hash(&before))),
        serde_json::from_value(json!({"title":"Legacy offline edit"})).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        f.ledger().outbox["i1"].values["title"],
        "Legacy offline edit"
    );
    let mut raw: Value = serde_json::from_slice(&fs::read(f.root().join(STATE)).unwrap()).unwrap();
    raw["writableFields"] = json!([]);
    fs::write(f.root().join(STATE), serde_json::to_vec(&raw).unwrap()).unwrap();
    let before = f.task();
    let error = seams::user_edit(
        &f.config,
        "tasks/tracker-i1.md".into(),
        Some(vault_tasks::content_hash(&before)),
        serde_json::from_value(json!({"title":"Refused edit"})).unwrap(),
    )
    .await
    .unwrap_err();
    assert!(error.contains("does not allow"));
    assert_eq!(f.task(), before);
    let mut unknown = f.ledger();
    unknown.provider = "future-provider".into();
    unknown.writable_fields = None;
    assert!(unknown.writable_fields().is_empty());
}

#[tokio::test]
async fn a6_review_recurring_status_rolls_once_and_refuses_ambiguous_reopen() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.sync().await.unwrap();
    f.local_edit("recurrence", "every day");
    let before = f.task();
    let now = Local::now();
    let expected = crate::quick_add::next_occurrence("every day", now.date_naive())
        .unwrap()
        .to_string();
    let args = json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":vault_tasks::content_hash(&before),"status":"Delivered"});
    let result = crate::task_broker::dispatch_companion(
        f.root(),
        "phone",
        "Phone",
        "repeat",
        "set_status",
        &args,
        now,
    )
    .unwrap();
    assert_eq!(result["rolledForwardTo"], expected);
    assert_eq!(
        frontmatter::parse(&f.task()).get("due"),
        Some(expected.as_str())
    );
    assert_eq!(frontmatter::parse(&f.task()).get("status"), Some("Todo"));
    assert!(crate::task_broker::dispatch_companion(
        f.root(),
        "phone",
        "Phone",
        "repeat",
        "set_status",
        &args,
        now
    )
    .is_err());
    let mut state = f.ledger();
    let mut duplicate = state.meta.as_ref().unwrap().states[0].clone();
    duplicate.id = "another-todo".into();
    state.meta.as_mut().unwrap().states.push(duplicate);
    save(f.root(), &state).unwrap();
    let before = f.task();
    let refusal = crate::task_broker::dispatch_companion(f.root(), "phone", "Phone", "ambiguous", "set_status",
        &json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":vault_tasks::content_hash(&before),"status":"Done"}), now).unwrap_err();
    assert!(format!("{refusal:?}").contains("unique"));
    assert_eq!(f.task(), before);
}

#[tokio::test]
async fn a6_review_claim_and_release_are_local_and_keep_provider_bytes() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.sync().await.unwrap();
    let before = f.task();
    let run =
        crate::automated_history::try_begin_run(f.root(), "chat", Some("fixture-claim")).unwrap();
    let request = |op: &str, hash: &str, scope: &str| crate::task_broker::BrokerRequest {
        v: 1,
        op: op.into(),
        run_id: run.id.clone(),
        surface: "chat".into(),
        scope: scope.into(),
        access: crate::tasks_mcp::TasksAccess::ReadWrite.as_str().into(),
        proof: crate::task_broker::proof_for(
            &run.id,
            "chat",
            scope,
            crate::tasks_mcp::TasksAccess::ReadWrite,
        ),
        args: json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":hash}),
    };
    let claimed = crate::task_broker::dispatch_at(
        f.root(),
        &request(
            "claim_task",
            &vault_tasks::content_hash(&before),
            "fixture-claim",
        ),
        true,
        Local::now(),
    )
    .unwrap();
    assert_eq!(claimed.result["status"], "Todo");
    let during = f.task();
    assert!(during.contains("chat:fixture-claim"));
    let repeat = crate::task_broker::dispatch_at(
        f.root(),
        &request(
            "claim_task",
            &vault_tasks::content_hash(&during),
            "fixture-claim",
        ),
        true,
        Local::now(),
    )
    .unwrap();
    assert_eq!(repeat.result["already"], true);
    assert_eq!(
        mirror::local_values(&during).unwrap(),
        mirror::local_values(&before).unwrap()
    );
    assert_eq!(
        mirror::block(&during, "description").unwrap(),
        mirror::block(&before, "description").unwrap()
    );
    assert_eq!(
        mirror::block(&during, "comments").unwrap(),
        mirror::block(&before, "comments").unwrap()
    );
    let refused = crate::task_broker::dispatch_at(
        f.root(),
        &request("release_task", &vault_tasks::content_hash(&during), "other"),
        true,
        Local::now(),
    )
    .unwrap_err();
    assert!(refused.contains("TASK_CLAIMED"));
    assert_eq!(f.task(), during);
    assert!(crate::task_broker::dispatch_at(
        f.root(),
        &request(
            "release_task",
            &vault_tasks::content_hash(&before),
            "fixture-claim"
        ),
        true,
        Local::now()
    )
    .unwrap_err()
    .contains("TASK_CONFLICT"));
    let mut task = vec![vault_tasks::get(f.root(), "tasks/tracker-i1.md").unwrap()];
    vault_tasks::mark_task_locks(f.root(), &mut task);
    assert!(vault_tasks::next_claimable(f.root(), task).is_none());
    let mut completed = f.ledger().records["i1"].issue.clone().unwrap();
    completed.state.kind = StateKind::Completed;
    completed.state.name = "Done".into();
    let closed = mirror::task(
        &during,
        "linear",
        &completed,
        &mirror::values(&completed, &BTreeMap::new(), &BTreeMap::new()),
        None,
        "active",
        false,
        "",
    )
    .unwrap();
    assert!(vault_tasks::claim_of(&closed).is_none());
    crate::task_broker::dispatch_at(
        f.root(),
        &request(
            "release_task",
            &vault_tasks::content_hash(&during),
            "fixture-claim",
        ),
        true,
        Local::now(),
    )
    .unwrap();
    let after = f.task();
    assert_eq!(
        mirror::local_values(&after).unwrap(),
        mirror::local_values(&before).unwrap()
    );
    assert!(vault_tasks::claim_of(&after).is_none());
    assert!(f.ledger().outbox.is_empty());
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn a6_review_project_hash_trims_only_outer_whitespace() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let rel = f.ledger().records["p1"].path.clone();
    let before = f.project_text();
    seams::user_edit(
        &f.config,
        rel.clone(),
        Some(format!(" {}\n", vault_tasks::content_hash(&before))),
        serde_json::from_value(json!({"title":"Padded project hash"})).unwrap(),
    )
    .await
    .unwrap();
    let after = f.project_text();
    assert!(seams::user_edit(
        &f.config,
        rel,
        Some(vault_tasks::content_hash(&before)),
        serde_json::from_value(json!({"title":"Stale project hash"})).unwrap()
    )
    .await
    .unwrap_err()
    .contains("TASK_CONFLICT"));
    assert_eq!(f.project_text(), after);
}

#[tokio::test]
async fn a6_review_staged_completion_replays_state_kind_and_clears_claim() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let old = mirror::patch_local(
        &f.task(),
        &Values::from([
            ("claimed_by".into(), json!("chat:fixture")),
            ("claimed_at".into(), json!(T0)),
        ]),
    )
    .unwrap();
    f.replace_task(&old);
    let mut state = f.ledger();
    let record = state.records["i1"].clone();
    let before = mirror::local_values(&old).unwrap();
    let mut after = before.clone();
    after.insert("status".into(), json!("Done"));
    api::stage_edit(&mut state, &record, before, after);
    save(f.root(), &state).unwrap();
    f.remote.lock().unwrap().fail = Some(503);
    assert!(f.sync().await.is_err());
    let next = f.task();
    let fm = frontmatter::parse(&next);
    assert_eq!(fm.get("status"), Some("Done"));
    assert_eq!(fm.get("status_type"), Some("completed"));
    assert!(vault_tasks::claim_of(&next).is_none());
    assert!(f.ledger().outbox["i1"].staged.is_none());
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn a_pass_keeps_whom_the_key_belongs_to_and_reads_it_once() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let viewer = f.ledger().meta.unwrap().viewer.unwrap();
    assert_eq!(
        (viewer.id.as_str(), viewer.name.as_str()),
        ("user-1", "Fixture Person")
    );
    let viewers = |f: &Fixture| {
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("Viewer"))
            .count()
    };
    assert_eq!(viewers(&f), 1);
    f.sync().await.unwrap();
    assert_eq!(
        viewers(&f),
        1,
        "the viewer is read with the hourly metadata, not every pass"
    );
    // A ledger saved before the viewer was kept reads it on the next pass.
    let mut state = f.ledger();
    state.meta.as_mut().unwrap().viewer = None;
    save(f.root(), &state).unwrap();
    f.sync().await.unwrap();
    assert_eq!(viewers(&f), 2);
    assert!(f.ledger().meta.unwrap().viewer.is_some());
}

#[tokio::test]
async fn a6_review_unrelated_ambiguous_names_do_not_hide_completion() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let mut state = f.ledger();
    let member = state.meta.as_ref().unwrap().members[0].clone();
    let mut twin = member.clone();
    twin.id = "user-twin".into();
    state.meta.as_mut().unwrap().members.push(twin);
    state
        .records
        .get_mut("i1")
        .unwrap()
        .issue
        .as_mut()
        .unwrap()
        .assignee = Some(member.clone());
    let mut project = state.records["p1"].clone();
    project.id = "p2".into();
    project.path = "workspaces/linear-p2/PROJECT.md".into();
    project.project.as_mut().unwrap().id = "p2".into();
    state.records.insert("p2".into(), project);
    save(f.root(), &state).unwrap();
    let old = mirror::patch_local(
        &f.task(),
        &Values::from([
            ("assignee".into(), json!(member.name)),
            ("claimed_by".into(), json!("chat:fixture")),
            ("claimed_at".into(), json!(T0)),
        ]),
    )
    .unwrap();
    f.replace_task(&old);
    seams::user_edit(
        &f.config,
        "tasks/tracker-i1.md".into(),
        Some(vault_tasks::content_hash(&old)),
        serde_json::from_value(json!({"status":"Done"})).unwrap(),
    )
    .await
    .unwrap();
    let next = f.task();
    assert_eq!(
        frontmatter::parse(&next).get("status_type"),
        Some("completed")
    );
    assert!(vault_tasks::claim_of(&next).is_none());
    assert_eq!(
        issue(&f.config, "i1").unwrap().issue.state.kind,
        StateKind::Completed
    );
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn a6_review_tracker_blocker_keys_resolve_uniquely_by_state_kind() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let mut task = vault_tasks::get(f.root(), "tasks/tracker-i1.md").unwrap();
    task.blocked_by = vec!["ENG-41".into()];
    let mut blocker = task.clone();
    blocker.url = "vault:tasks/blocker.md".into();
    blocker.status = "Shipped".into();
    blocker.blocked_by.clear();
    let meta = blocker.tracker.as_mut().unwrap();
    meta.id = "i41".into();
    meta.key = Some("ENG-41".into());
    meta.status_type = Some(StateKind::Completed);
    let mut tasks = vec![task, blocker];
    vault_tasks::link_blockers(&mut tasks);
    assert!(tasks[0].ready);
    assert!(tasks[0].unresolved_blockers.is_empty());
    let mut duplicate = tasks[1].clone();
    duplicate.url = "vault:tasks/copied-blocker.md".into();
    tasks.push(duplicate);
    vault_tasks::link_blockers(&mut tasks);
    assert!(!tasks[0].ready);
    assert_eq!(tasks[0].unresolved_blockers, vec!["ENG-41"]);
}

#[tokio::test]
async fn a6_review_project_create_scope_error_is_durable_and_never_resent() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.remote.lock().unwrap().project_create_out_of_scope = true;
    for _ in 0..2 {
        let error = api::create_project_with(
            &f.config,
            &f.adapter(),
            TrackerNewProject {
                name: "Scope refusal".into(),
                ..Default::default()
            },
            Some("scope-refusal"),
        )
        .await
        .unwrap_err();
        assert_eq!(error, TrackerError::OutOfScope.to_string());
    }
    assert_eq!(
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("ProjectCreate"))
            .count(),
        1
    );
    let state = f.ledger();
    assert!(state.project_creates["scope-refusal"].attempted);
    assert!(state.project_creates["scope-refusal"].project.is_none());
    assert!(state.project_creates["scope-refusal"].error.is_some());
}

#[tokio::test]
async fn a6_review_detail_binds_the_requested_file_and_rejects_unsupported_fields() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.sync().await.unwrap();
    let before = f.task();
    vault::write_note(f.root(), "tasks/copied.md", &before).unwrap();
    let ctx = crate::tasks_mcp::Ctx {
        vault_root: Some(f.root().to_path_buf()),
        access: crate::tasks_mcp::TasksAccess::ReadOnly,
        client: None,
    };
    let detail = |rel: &str| {
        let result = crate::tasks_mcp::handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_task","arguments":{"ref":format!("vault:{rel}")}}}), &ctx).unwrap();
        serde_json::from_str::<Value>(result["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    assert!(detail("tasks/copied.md")["tracker"]["error"].is_string());
    assert!(detail("tasks/tracker-i1.md")["tracker"]["issue"].is_object());
    for fields in [
        json!({"subtasks":[]}),
        json!({"blockedBy":[]}),
        json!({"creator":"user-1"}),
        json!({"unknown":true}),
    ] {
        assert!(crate::task_broker::dispatch_companion(f.root(), "phone", "Phone", "invalid-field", "update_task",
            &json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":vault_tasks::content_hash(&before),"fields":fields}), Local::now()).is_err());
        assert_eq!(f.task(), before);
        assert!(f.ledger().outbox.is_empty());
    }
}

#[tokio::test]
async fn edit_lock_refuses_busy_on_the_same_root_but_not_another() {
    let f = Fixture::new().await;
    let other = tempfile::tempdir().unwrap();
    let held = seams::hold_edit_lock(f.root(), Duration::from_secs(1)).unwrap();
    let busy = seams::hold_edit_lock(f.root(), Duration::from_millis(100));
    assert!(matches!(busy, Err(e) if e.contains("sync is busy")));
    // Another vault root has its own in-process lock and flock.
    drop(seams::hold_edit_lock(other.path(), Duration::from_millis(100)).unwrap());
    // A sync on the same root, holding the lock, also makes an edit wait.
    drop(held);
    let sync_guard = root_lock(f.root()).lock_owned().await;
    let busy = seams::hold_edit_lock(f.root(), Duration::from_millis(100));
    assert!(matches!(busy, Err(e) if e.contains("sync is busy")));
    drop(sync_guard);
    drop(seams::hold_edit_lock(f.root(), Duration::from_millis(100)).unwrap());
}

#[tokio::test]
async fn tracker_user_and_root_bridge_refuse_a_different_running_team() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let before = f.task();
    let mut other = f.config.clone();
    other.tracker.team_id = Some(OTHER.into());
    let refusal = seams::user_edit(
        &other,
        "tasks/tracker-i1.md".into(),
        Some(vault_tasks::content_hash(&before)),
        serde_json::from_value(json!({"title":"Wrong team write"})).unwrap(),
    )
    .await
    .unwrap_err();
    assert!(refusal.contains("running configuration"));
    seams::register_running(&other);
    let refusal = seams::hold_running_edit_lock(f.root(), Duration::from_secs(1));
    assert!(matches!(refusal, Err(e) if e.contains("running configuration")));
    assert_eq!(f.task(), before);
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn broker_tracker_activity_never_edits_provider_comment_headings_or_checklists() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.remote.lock().unwrap().comments = vec![json!({
        "id":"comment-owned","body":"## Activity\n- Provider activity\n- [ ] Provider checklist",
        "createdAt":T1,"updatedAt":T1,"editedAt":null,"user":{"id":"user-1","name":"Fixture Person"},
        "parent":null,"issue":{"id":"i1","team":{"id":TEAM}}
    })];
    f.sync().await.unwrap();
    f.replace_task(&format!("{}\n- [ ] A local checklist step\n", f.task()));
    let before = f.task();
    let task = vault_tasks::get(f.root(), "tasks/tracker-i1.md").unwrap();
    assert_eq!(task.subtasks.len(), 1);
    assert_eq!(task.subtasks[0].text, "A local checklist step");
    let args = json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":task.content_hash,"body":"A local progress note","idempotencyKey":"fixture-note"});
    let result = crate::task_broker::dispatch_companion(
        f.root(),
        "fixture-device",
        "Fixture phone",
        "fixture-activity",
        "add_comment",
        &args,
        Local::now(),
    )
    .unwrap();
    assert_eq!(result["disposition"], "applied");
    assert_eq!(
        mirror::block(&f.task(), "comments").unwrap(),
        mirror::block(&before, "comments").unwrap()
    );
    let task = vault_tasks::get(f.root(), "tasks/tracker-i1.md").unwrap();
    assert!(task
        .activity
        .as_deref()
        .unwrap()
        .contains("A local progress note"));
    assert!(!task
        .activity
        .as_deref()
        .unwrap()
        .contains("Provider activity"));
}

#[tokio::test]
async fn broker_tracker_legacy_status_and_stale_hash_use_the_shared_editor() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.sync().await.unwrap();
    let old = f.task();
    let hash = vault_tasks::content_hash(&old);
    crate::task_broker::dispatch_companion(
        f.root(),
        "fixture-device",
        "Fixture phone",
        "fixture-status",
        "set_status",
        &json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":hash,"status":"Delivered"}),
        Local::now(),
    )
    .unwrap();
    assert_eq!(
        issue(&f.config, "i1").unwrap().issue.state.kind,
        StateKind::Completed
    );
    let changed = f.task();
    let rejected = crate::task_broker::dispatch_companion(
        f.root(),
        "fixture-device",
        "Fixture phone",
        "fixture-stale",
        "update_task",
        &json!({"ref":"vault:tasks/tracker-i1.md","expectedHash":hash,"fields":{"title":"Stale edit"}}),
        Local::now(),
    );
    assert!(rejected.is_err());
    assert_eq!(f.task(), changed);
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn a_staged_editor_intent_recovers_and_a_missing_mirror_keeps_its_outbox() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let before = mirror::local_values(&f.task()).unwrap();
    let mut after = before.clone();
    after.insert("title".into(), json!("Survives the file-write crash"));
    after.insert("status".into(), json!("Done"));
    let mut state = f.ledger();
    state.outbox.insert(
        "i1".into(),
        Outbox {
            values: after
                .iter()
                .filter(|(key, value)| before.get(*key) != Some(*value))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            base: state.records["i1"].base.clone(),
            staged: Some(StagedEdit {
                before,
                after,
                local_before: Values::new(),
                local_after: Values::new(),
            }),
            asked_by: None,
            project_id: None,
            generation: 0,
        },
    );
    save(f.root(), &state).unwrap();
    f.sync().await.unwrap();
    assert_eq!(f.updates(), 1);
    assert_eq!(
        issue(&f.config, "i1").unwrap().issue.title,
        "Survives the file-write crash"
    );
    assert_eq!(
        issue(&f.config, "i1").unwrap().issue.state.kind,
        StateKind::Completed
    );
    assert!(f.ledger().outbox.is_empty());
    edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("title".into(), json!("Keep this pending value"))]),
            expected: mirror::local_values(&f.task()).unwrap(),
        },
    )
    .await
    .unwrap();
    let pending_file = f.task();
    let path = f.ledger().records["i1"].path.clone();
    fs::remove_file(f.root().join(&path)).unwrap();
    let report = f.sync().await.unwrap();
    assert!(!report.errors.is_empty());
    assert_eq!(f.updates(), 1);
    assert!(!f.root().join(&path).exists());
    assert_eq!(
        f.ledger().outbox["i1"].values["title"],
        json!("Keep this pending value")
    );
    vault::create_note(f.root(), &path, &pending_file).unwrap();
    f.sync().await.unwrap();
    assert_eq!(f.updates(), 2);
    assert!(f.ledger().outbox.is_empty());
}

#[tokio::test]
async fn a_broken_item_keeps_its_retry_snapshot_without_freezing_other_cursors() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let good = f.task();
    f.replace_task(&good.replace("<!-- /tracker:description -->", "BROKEN"));
    f.remote
        .lock()
        .unwrap()
        .issues
        .push(stub::issue_node("i2", "ENG-2", TEAM, T1));
    let report = f.sync().await.unwrap();
    assert!(!report.errors.is_empty());
    assert_eq!(f.ledger().cursor_issues.as_deref(), Some(T1));
    assert!(f.ledger().records["i1"].issue.is_some());
    assert!(f.root().join("tasks/tracker-i2.md").is_file());
    f.replace_task(&good);
    assert!(f.sync().await.unwrap().errors.is_empty());
}

/// An attempted create journaled at T0 with `issue_id`, and a person's
/// issue made after it with the same title, body and project (Q-R22).
fn journal_attempted_create(f: &Fixture, issue_id: Option<&str>) {
    let input = NewIssue {
        title: "Same title".into(),
        description: Some("Requested body".into()),
        ..Default::default()
    };
    let mut state = f.ledger();
    state.creates.insert(
        "crashed".into(),
        Create {
            agent_key: "crashed".into(),
            input,
            at: T0.into(),
            attempted: true,
            issue: None,
            error: None,
            issue_id: issue_id.map(str::to_string),
            resends: 0,
        },
    );
    save(f.root(), &state).unwrap();
    let mut person = stub::issue_node("person", "ENG-9", TEAM, T1);
    person["title"] = json!("Same title");
    person["createdAt"] = json!(T1);
    person["description"] = json!("Requested body");
    f.remote.lock().unwrap().issues.push(person);
}

fn issue_creates(f: &Fixture) -> Vec<Value> {
    f.server
        .requests()
        .iter()
        .filter(|request| request.operation().as_deref() == Some("IssueCreate"))
        .map(|request| request.variables()["input"].clone())
        .collect()
}

#[tokio::test]
async fn create_recovery_adopts_only_the_issue_with_the_journaled_id() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let id = "5b0c3c52-6a8e-4d7c-9f1e-2a4b6c8d0e1f";
    journal_attempted_create(&f, Some(id));
    let mut landed = stub::issue_node(id, "ENG-10", TEAM, T1);
    landed["title"] = json!("Same title");
    f.remote.lock().unwrap().issues.push(landed);
    f.sync().await.unwrap();
    let entry = &f.ledger().creates["crashed"];
    assert_eq!(entry.issue.as_ref().unwrap().id, id);
    assert!(entry.error.is_none());
    assert!(issue_creates(&f).is_empty());
}

/// R3 (#1402 residual): a key limited to AI and HOL can't tell a create that
/// never arrived from one moved into a team it can't see, so the journal keeps
/// `attempted`, nothing is resent, and no second issue is made.
#[tokio::test]
async fn a_lost_create_stays_attempted_when_the_key_cannot_see_other_teams() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let id = "8a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    journal_attempted_create(&f, Some(id));
    f.remote.lock().unwrap().restricted_key = true;
    f.sync().await.unwrap();
    let entry = f.ledger().creates["crashed"].clone();
    assert!(entry.attempted, "the attempt is kept");
    assert!(entry.issue.is_none());
    assert!(entry.error.unwrap().contains("could not be checked"));
    assert!(issue_creates(&f).is_empty(), "nothing is resent");
}

#[tokio::test]
async fn a_lost_create_is_resent_with_its_id_and_never_adopts_a_persons_issue() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let id = "7d1e2f30-4a5b-4c6d-8e7f-90a1b2c3d4e5";
    journal_attempted_create(&f, Some(id));
    f.sync().await.unwrap();
    let sent = issue_creates(&f);
    assert_eq!(sent.len(), 1, "the pass resends it once");
    assert_eq!(sent[0]["id"], json!(id));
    let entry = f.ledger().creates["crashed"].clone();
    assert_eq!(
        entry.issue.unwrap().id,
        id,
        "its own issue, not the person's"
    );
    assert!(entry.error.is_none());
    assert_eq!(entry.resends, 1);
    // A later explicit create of the same request sends nothing more.
    let input = NewIssue {
        title: "Same title".into(),
        description: Some("Requested body".into()),
        ..Default::default()
    };
    let again = api::create_with(&f.config, &f.adapter(), "crashed", input)
        .await
        .unwrap();
    assert_eq!(again.id.as_deref(), Some(id));
    assert_eq!(issue_creates(&f).len(), 1);
}

#[tokio::test]
async fn a_create_moved_to_another_team_or_an_unclear_read_back_is_never_resent() {
    for moved in [true, false] {
        let f = Fixture::new().await;
        f.sync().await.unwrap();
        let id = "9e8d7c6b-5a49-4382-8170-6f5e4d3c2b1a";
        journal_attempted_create(&f, Some(id));
        if moved {
            let landed = stub::issue_node(id, "PER-3", OTHER, T1);
            f.remote.lock().unwrap().issues.push(landed);
        } else {
            f.remote.lock().unwrap().malformed_landing = true;
        }
        f.sync().await.unwrap();
        let entry = f.ledger().creates["crashed"].clone();
        assert!(entry.issue.is_none());
        assert!(entry.attempted, "moved {moved}: the attempt is kept");
        let error = entry.error.unwrap();
        assert!(
            error.contains(if moved {
                "another team"
            } else {
                "could not be checked"
            }),
            "{error}"
        );
        assert!(issue_creates(&f).is_empty());
    }
}

#[tokio::test]
async fn an_old_journal_without_an_id_is_never_adopted_by_its_fields() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    journal_attempted_create(&f, None);
    f.sync().await.unwrap();
    let entry = f.ledger().creates["crashed"].clone();
    assert!(entry.issue.is_none());
    assert!(entry.attempted);
    assert!(entry.error.unwrap().contains("no unique confirmed outcome"));
    assert!(issue_creates(&f).is_empty());
}

#[tokio::test]
async fn unavailable_project_without_snapshot_still_marks_its_bound_file() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    f.edit_project("title", "Keep this project intent");
    let mut state = f.ledger();
    state.records.get_mut("p1").unwrap().project = None;
    state.last_sync = None;
    save(f.root(), &state).unwrap();
    f.remote.lock().unwrap().projects.clear();
    f.sync().await.unwrap();
    assert!(f.ledger().records["p1"].remote_missing);
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("tracker_state"),
        Some("moved")
    );
    assert!(f.project_text().contains("Keep this project intent"));
}

#[tokio::test]
async fn unavailable_task_without_snapshot_still_marks_its_bound_file() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("title".into(), json!("Keep this unavailable task intent"))]),
            expected: mirror::local_values(&f.task()).unwrap(),
        },
    )
    .await
    .unwrap();
    let before = f.task();
    let mut state = f.ledger();
    state.records.get_mut("i1").unwrap().issue = None;
    state.last_sync = None;
    save(f.root(), &state).unwrap();
    f.remote.lock().unwrap().issues.clear();
    f.sync().await.unwrap();
    let state = f.ledger();
    let after = f.task();
    assert!(state.records["i1"].remote_missing);
    assert_eq!(
        frontmatter::parse(&after).get("tracker_state"),
        Some("moved")
    );
    assert_eq!(
        frontmatter::parse(&after).get("tracker_pending"),
        Some("false")
    );
    assert!(after.contains("Keep this unavailable task intent"));
    assert!(mirror::block(&after, "conflicts")
        .unwrap()
        .is_some_and(|s| s.contains("Keep this unavailable task intent")));
    assert_eq!(
        after,
        mirror::unavailable_record(&before, &conflict_text(&state.records["i1"], "Linear"))
            .unwrap()
    );
    assert!(!state.outbox.contains_key("i1"));
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn unavailable_task_does_not_recreate_a_missing_bound_file() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("title".into(), json!("Keep the unsent missing-file edit"))]),
            expected: mirror::local_values(&f.task()).unwrap(),
        },
    )
    .await
    .unwrap();
    let mut state = f.ledger();
    state.last_sync = None;
    let path = state.records["i1"].path.clone();
    save(f.root(), &state).unwrap();
    fs::remove_file(f.root().join(&path)).unwrap();
    f.remote.lock().unwrap().issues.clear();
    f.sync().await.unwrap();
    assert!(!f.root().join(&path).exists());
    let state = f.ledger();
    assert!(state.records["i1"].remote_missing);
    assert!(state.records["i1"]
        .conflicts
        .iter()
        .any(|c| c.local == json!("Keep the unsent missing-file edit")));
    assert_eq!(f.updates(), 0);
}

#[tokio::test]
async fn missing_projects_park_local_edits_and_returning_projects_resume() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let project = f.remote.lock().unwrap().projects[0].clone();
    f.edit_project("title", "Keep the unsent project title");
    f.remote.lock().unwrap().projects.clear();
    let report = f.sync().await.unwrap();
    assert!(report.errors.is_empty());
    assert!(f.ledger().records["p1"].remote_missing);
    assert!(!f.ledger().outbox.contains_key("p1"));
    assert!(f.project_text().contains("Keep the unsent project title"));
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("tracker_state"),
        Some("moved")
    );
    f.remote.lock().unwrap().projects.push(project);
    let mut state = f.ledger();
    state.last_sync = None;
    save(f.root(), &state).unwrap();
    f.sync().await.unwrap();
    assert!(!f.ledger().records["p1"].remote_missing);
    assert_eq!(
        frontmatter::parse(&f.project_text()).get("tracker_state"),
        Some("active")
    );
}

#[tokio::test]
async fn unknown_baseline_keys_cannot_patch_vault_owned_frontmatter() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let old = frontmatter::patch(
        &f.task(),
        &[(
            "vault_only",
            frontmatter::KeyEdit::Quoted("Keep private fixture bytes".into()),
        )],
        None,
    )
    .unwrap();
    f.replace_task(&old);
    let mut state = f.ledger();
    let record = &state.records["i1"];
    let mut values = record.base.clone();
    values.insert("vault_only".into(), json!("Must not replace it"));
    let rendered = mirror::task(
        &old,
        "linear",
        record.issue.as_ref().unwrap(),
        &values,
        None,
        "active",
        false,
        "",
    )
    .unwrap();
    assert_eq!(
        frontmatter::parse(&rendered).get("vault_only"),
        Some("Keep private fixture bytes")
    );
    state.records.get_mut("i1").unwrap().base = values;
    save(f.root(), &state).unwrap();
    let calls = f.server.requests().len();
    assert!(f.sync().await.unwrap_err().contains("unsafe"));
    assert_eq!(f.server.requests().len(), calls);
    assert_eq!(
        fs::read_to_string(f.root().join("tasks/tracker-i1.md")).unwrap(),
        old
    );
}

#[tokio::test]
async fn ledger_paths_cannot_claim_an_unrelated_task_or_project() {
    let f = Fixture::new().await;
    f.seed_project();
    f.sync().await.unwrap();
    let original = f.ledger();
    let calls = f.server.requests().len();
    let private = "---\ntitle: Private fixture\n---\nKeep these bytes.\n";
    for (id, rel) in [
        ("i1", "tasks/private.md"),
        ("p1", "workspaces/Journal/PROJECT.md"),
    ] {
        vault::create_note(f.root(), rel, private).unwrap();
        let mut state = original.clone();
        state.records.get_mut(id).unwrap().path = rel.into();
        save(f.root(), &state).unwrap();
        assert!(f.sync().await.unwrap_err().contains("identity"));
        assert_eq!(f.server.requests().len(), calls);
        assert_eq!(fs::read_to_string(f.root().join(rel)).unwrap(), private);
    }
    save(f.root(), &original).unwrap();
    assert!(f.sync().await.is_ok());
}

#[tokio::test]
async fn a_symbolic_link_cannot_redirect_a_mirror_outside_the_leases_folder() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let target = "workspaces/another/PROJECT.md";
    vault::create_note(
        f.root(),
        target,
        "---\ntitle: private\n---\nKeep unchanged.\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(f.root().join(target), f.root().join("tasks/linked.md")).unwrap();
    let mut state = f.ledger();
    state.records.get_mut("i1").unwrap().path = "tasks/linked.md".into();
    save(f.root(), &state).unwrap();
    let calls = f.server.requests().len();
    assert!(f.sync().await.unwrap_err().contains("symbolic link"));
    assert_eq!(f.server.requests().len(), calls);
    assert_eq!(
        fs::read_to_string(f.root().join(target)).unwrap(),
        "---\ntitle: private\n---\nKeep unchanged.\n"
    );
}

#[tokio::test]
async fn local_pushes_retain_independently_paged_comments_and_attachments() {
    let f = Fixture::new().await;
    f.remote.lock().unwrap().truncate_detail = true;
    f.remote.lock().unwrap().issues[0]["attachments"] = json!({"nodes":[{"id":"file-1","title":"Fixture file","url":"https://example.test/file","sourceType":"upload","createdAt":T0},{"id":"file-2","title":"Second paged file","url":"https://example.test/file-2","sourceType":"upload","createdAt":T0}]});
    f.remote.lock().unwrap().comments = vec![
        json!({"id":"comment-1","body":"Keep the paged comment.","createdAt":T0,"updatedAt":T0,"editedAt":null,"user":{"name":"Fixture Person"},"parent":null,"issue":{"id":"i1","team":{"id":TEAM}}}),
    ];
    f.sync().await.unwrap();
    f.local_edit("title", "Local change with detail");
    f.sync().await.unwrap();
    let state = f.ledger();
    let issue = state.records["i1"].issue.as_ref().unwrap();
    assert_eq!(issue.comments.len(), 1);
    assert_eq!(issue.attachments.len(), 2);
    assert!(f.task().contains("Keep the paged comment."));
    assert!(f.task().contains("Fixture file"));
    assert!(f.task().contains("Second paged file"));
    f.remote.lock().unwrap().comments.clear();
    let mut state = f.ledger();
    state.last_sync = None;
    save(f.root(), &state).unwrap();
    f.sync().await.unwrap();
    assert!(f.ledger().records["i1"]
        .issue
        .as_ref()
        .unwrap()
        .comments
        .is_empty());
    assert!(!f.task().contains("Keep the paged comment."));
}

#[test]
fn a_daily_full_pass_is_still_owed_when_the_mac_wakes_after_three() {
    let mut state = Ledger::new("linear", TEAM);
    state.last_sync = Some(T0.into());
    state.full_date = Some("2026-11-07".into());
    assert!(!full_due(&state, "2026-11-08", 2));
    for hour in [3, 4, 12, 23] {
        assert!(full_due(&state, "2026-11-08", hour));
    }
    state.full_date = Some("2026-11-08".into());
    assert!(!full_due(&state, "2026-11-08", 12));
}

#[tokio::test]
async fn nonempty_label_arrays_round_trip_without_becoming_new_local_edits() {
    let f = Fixture::new().await;
    f.remote.lock().unwrap().issues[0]["labels"] = json!({"nodes":[{"name":"Feature"}]});
    f.sync().await.unwrap();
    let before = f.task();
    assert_eq!(
        mirror::local_values(&before).unwrap()["labels"],
        json!(["Feature"])
    );
    f.sync().await.unwrap();
    assert_eq!(f.task(), before);
    assert_eq!(f.updates(), 0);
}

#[test]
fn the_tracker_lease_admits_chats_but_still_excludes_root_writers() {
    use crate::automated_history::{self as history, BeginRunError, LeaseHolder};
    let dir = tempfile::Builder::new()
        .prefix("vault-tracker-lease-")
        .tempdir()
        .unwrap();
    let root = dir.path();
    for id in ["tracked", "another"] {
        vault::create_note(
            root,
            &format!("workspaces/{id}/WORKSPACE.md"),
            "---\ntitle: fixture\n---\n",
        )
        .unwrap();
    }
    let folders = BTreeSet::from(["workspaces/tracked".into()]);
    vault::create_note(
        root,
        "tasks/shared.md",
        "---\ntitle: shared\n---\nOriginal.\n",
    )
    .unwrap();
    let sync = history::tracker_mirror_write_lease(root, &folders).unwrap();
    assert!(matches!(
        history::lease_holder(root),
        Some(LeaseHolder::TrackerSync)
    ));
    assert_eq!(
        history::lease_holder(root).unwrap().describe(),
        "task tracker sync"
    );
    assert!(!history::tracker_blocks_scope(root, "workspaces/tracked"));
    assert!(!history::tracker_blocks_scope(root, "workspaces/another"));
    history::try_begin_scoped_shared(root, "workspaces/tracked", "chat", None)
        .unwrap()
        .finish("success")
        .unwrap();
    let other = history::try_begin_scoped_shared(root, "workspaces/another", "chat", None).unwrap();
    let wrote = std::cell::Cell::new(false);
    let blocked = history::with_live_run(root, other.id(), &["chat"], || {
        wrote.set(true);
        vault::write_note(root, "tasks/shared.md", "Must not write.")
    });
    let refused = blocked.unwrap_err();
    assert!(
        refused.to_lowercase().contains("task tracker sync"),
        "{refused}"
    );
    assert!(!wrote.get());
    assert!(fs::read_to_string(root.join("tasks/shared.md"))
        .unwrap()
        .contains("Original."));
    assert!(matches!(
        history::tracker_mirror_write_lease(root, &BTreeSet::new()),
        Err(BeginRunError::Busy(LeaseHolder::TrackerSync))
    ));
    drop(sync);
    history::with_live_run(root, other.id(), &["chat"], || {
        vault::write_note(root, "tasks/shared.md", "After the sync lease.")
    })
    .unwrap();
    other.finish("success").unwrap();
    let first = history::tracker_mirror_write_lease(root, &BTreeSet::new()).unwrap();
    assert!(matches!(
        history::tracker_mirror_write_lease(root, &folders),
        Err(BeginRunError::Busy(LeaseHolder::TrackerSync))
    ));
    drop(first);
    let chat = history::try_begin_scoped_shared(root, "workspaces/tracked", "chat", None).unwrap();
    let sync = history::tracker_mirror_write_lease(root, &folders).unwrap();
    drop(sync);
    chat.finish("success").unwrap();
    assert!(history::tracker_mirror_write_lease(root, &folders).is_ok());
}

#[test]
fn tracker_atomic_commits_preserve_chat_edits_and_history_attribution() {
    use crate::automated_history as history;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let scope = "workspaces/tracked";
    let mirror = "workspaces/tracked/PROJECT.md";
    let new_note = "workspaces/tracked/TRACKER.md";
    vault::create_note(root, "workspaces/tracked/WORKSPACE.md", "brief").unwrap();
    vault::create_note(root, mirror, "original").unwrap();
    let sync = history::tracker_mirror_write_lease(root, &[scope.into()].into()).unwrap();
    let chat = history::try_begin_scoped_shared(root, scope, "chat", None).unwrap();
    assert!(write(root, mirror, "synced", Some("original")).unwrap());
    assert!(write(root, new_note, "new context", None).unwrap());
    let finished = chat.finish("success").unwrap();
    assert!(finished.changes.is_empty(), "{:?}", finished.changes);

    let chat = history::try_begin_scoped_shared(root, scope, "chat", None).unwrap();
    fs::write(root.join(mirror), "agent edit").unwrap();
    assert!(write(root, mirror, "stale sync", Some("synced")).is_err());
    assert_eq!(fs::read_to_string(root.join(mirror)).unwrap(), "agent edit");
    assert!(write(root, new_note, "replace existing", None).is_err());
    assert_eq!(
        fs::read_to_string(root.join(new_note)).unwrap(),
        "new context"
    );
    let finished = chat.finish("success").unwrap();
    assert!(finished.changes.iter().any(|c| c.vault_path == mirror));
    drop(sync);

    let chat = history::try_begin_scoped_shared(root, scope, "chat", None).unwrap();
    history::with_live_run(root, chat.id(), &["chat"], || {
        write_local(root, mirror, "broker edit", Some("agent edit"))
    })
    .unwrap();
    let finished = chat.finish("success").unwrap();
    assert!(
        finished.changes.iter().any(|c| c.vault_path == mirror),
        "a requested editor write remains the chat's change"
    );
}

fn comment_creates(f: &Fixture) -> usize {
    f.server
        .requests()
        .iter()
        .filter(|r| r.operation().as_deref() == Some("CommentCreate"))
        .count()
}

#[tokio::test]
async fn a_person_comment_posts_once_and_joins_the_mirrored_thread() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let out = api::comment_with(&f.config, &f.adapter(), "i1", "  Looks good.  ", "c-1")
        .await
        .unwrap();
    let id = out.id.unwrap();
    assert_eq!(out.note, "Comment posted.");
    assert_eq!(comment_creates(&f), 1);
    let ledger = f.ledger();
    let thread = &ledger.records["i1"].issue.as_ref().unwrap().comments;
    assert!(thread.iter().any(|c| c.id == id && c.body == "Looks good."));
    // The same key again never posts twice; another comment needs another key.
    let again = api::comment_with(&f.config, &f.adapter(), "i1", "Looks good.", "c-1")
        .await
        .unwrap();
    assert_eq!(again.id.as_deref(), Some(id.as_str()));
    assert_eq!(comment_creates(&f), 1);
    assert!(
        api::comment_with(&f.config, &f.adapter(), "i1", "Something else", "c-1")
            .await
            .is_err()
    );
    // The next pass reads the comment again by id, once.
    f.sync().await.unwrap();
    let ledger = f.ledger();
    let thread = &ledger.records["i1"].issue.as_ref().unwrap().comments;
    assert_eq!(thread.iter().filter(|c| c.id == id).count(), 1);
}

#[tokio::test]
async fn a_lost_comment_reply_is_unconfirmed_and_never_resent() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.remote.lock().unwrap().lose_comment_reply = true;
    let out = api::comment_with(&f.config, &f.adapter(), "i1", "Shipped.", "c-lost")
        .await
        .unwrap();
    assert!(out.id.is_none());
    assert!(out.note.contains("unconfirmed"), "{}", out.note);
    f.remote.lock().unwrap().lose_comment_reply = false;
    api::comment_with(&f.config, &f.adapter(), "i1", "Shipped.", "c-lost")
        .await
        .unwrap();
    assert_eq!(comment_creates(&f), 1);
}

#[tokio::test]
async fn comments_are_refused_before_any_request_off_the_team_or_empty() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    for (id, body) in [("missing", "Hi"), ("i1", "   ")] {
        assert!(api::comment_with(&f.config, &f.adapter(), id, body, "c-x")
            .await
            .is_err());
    }
    let long = "é".repeat(api::COMMENT_MAX / 2 + 1);
    assert!(
        api::comment_with(&f.config, &f.adapter(), "i1", &long, "c-y")
            .await
            .is_err()
    );
    let mut state = f.ledger();
    state.records.get_mut("i1").unwrap().remote_missing = true;
    save(f.root(), &state).unwrap();
    assert!(
        api::comment_with(&f.config, &f.adapter(), "i1", "Hi", "c-z")
            .await
            .is_err()
    );
    assert_eq!(comment_creates(&f), 0);
    assert!(f.ledger().comments.is_empty());
}

#[tokio::test]
async fn a_comment_cut_short_mid_send_reads_unconfirmed_and_is_never_sent() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let mut state = f.ledger();
    state.comments.insert(
        "c-crash".into(),
        CommentPost {
            issue_id: "i1".into(),
            body: "Half sent".into(),
            attempted: true,
            comment_id: None,
            error: None,
        },
    );
    save(f.root(), &state).unwrap();
    let out = api::comment_with(&f.config, &f.adapter(), "i1", "Half sent", "c-crash")
        .await
        .unwrap();
    assert!(
        out.id.is_none() && out.note.contains("unconfirmed"),
        "{}",
        out.note
    );
    assert_eq!(comment_creates(&f), 0);
}

#[tokio::test]
async fn an_issue_history_is_read_on_demand_by_key_and_never_written_to_the_mirror() {
    let f = Fixture::new().await;
    f.remote.lock().unwrap().issues[0]["history"] = json!({"nodes":[{
        "id":"h1","createdAt":T1,"actor":{"name":"Ana"},
        "fromState":{"name":"Todo","type":"unstarted"},"toState":{"name":"In Progress","type":"started"}
    }]});
    assert!(
        api::history_with(&f.config, &f.adapter(), "i1")
            .await
            .is_err(),
        "an issue that isn't mirrored yet is refused before any read"
    );
    assert!(f.server.requests().is_empty());
    f.sync().await.unwrap();
    let ledger_before = fs::read(f.root().join(STATE)).unwrap();
    let task_before = f.task();
    let history = api::history_with(&f.config, &f.adapter(), "ENG-1")
        .await
        .unwrap();
    assert!(!history.more);
    let events = history.events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, tracker::HistoryKind::State);
    assert_eq!(events[0].actor.as_deref(), Some("Ana"));
    assert_eq!(events[0].to.as_deref(), Some("In Progress"));
    assert_eq!(f.task(), task_before, "the mirror file is untouched");
    assert_eq!(fs::read(f.root().join(STATE)).unwrap(), ledger_before);
    f.remote.lock().unwrap().fail = Some(500);
    let error = api::history_with(&f.config, &f.adapter(), "i1")
        .await
        .unwrap_err();
    assert!(
        !error.contains("lin_api_fixture_only_never_real"),
        "{error}"
    );
}

/// X2: the audit rows the write paths leave.
#[path = "audit_sync_tests.rs"]
mod audit_writes;

#[tokio::test]
async fn the_personal_team_syncs_beside_the_shared_one_each_marked_with_its_team() {
    let f = Fixture::two_teams().await;
    f.sync().await.unwrap();
    let ledger = f.ledger();
    assert_eq!(ledger.scope, TEAM);
    assert!(ledger.records.contains_key("i1") && ledger.records.contains_key("h1"));
    let shared = fs::read_to_string(f.root().join(&ledger.records["i1"].path)).unwrap();
    let own = fs::read_to_string(f.root().join(&ledger.records["h1"].path)).unwrap();
    assert_eq!(frontmatter::parse(&shared).get("tracker_team"), Some("ENG"));
    assert_eq!(frontmatter::parse(&own).get("tracker_team"), Some("PER"));
    // The personal team has its own metadata and cursors; the shared team's
    // stay in the primary slots.
    assert_eq!(ledger.meta_for(HOL).unwrap().team.key, "PER");
    assert_eq!(ledger.meta_for(TEAM).unwrap().team.key, "ENG");
    // A second pass is quiet, and the view lists both with their keys.
    f.sync().await.unwrap();
    let view = api::view(&f.config, &TrackerViewQuery::default()).unwrap();
    let keys: Vec<_> = view
        .rows
        .iter()
        .map(|r| (r.key.clone(), r.team_key.clone()))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("ENG-1".into(), Some("ENG".into())),
            ("PER-1".into(), Some("PER".into()))
        ]
    );
    assert_eq!(view.team_keys, vec!["ENG", "PER"]);
    let only = api::view(
        &f.config,
        &TrackerViewQuery {
            team_key: Some("PER".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(only.rows.len(), 1);
    assert_eq!(only.rows[0].key, "PER-1");
}

#[tokio::test]
async fn a_node_of_a_third_team_in_the_personal_teams_pages_never_reaches_the_vault() {
    let f = Fixture::two_teams().await;
    f.remote.lock().unwrap().leak_scope = true;
    f.remote.lock().unwrap().issues.push(stub::issue_node(
        "lp1",
        "LP-1",
        "team-lp-0001-fixture",
        T1,
    ));
    f.sync().await.unwrap();
    let ledger = f.ledger();
    assert!(!ledger.records.contains_key("lp1"));
    assert!(ledger.records.contains_key("h1"));
}

#[tokio::test]
async fn a_create_names_its_team_and_only_the_two_teams_resolve() {
    let f = Fixture::two_teams().await;
    f.sync().await.unwrap();
    for refused in ["LP", "PRO", "lp", "team-lp-0001-fixture", "11111111"] {
        let before = f.server.requests().len();
        let task = TrackerNewTask {
            title: "Nope".into(),
            team_key: Some(refused.into()),
            ..Default::default()
        };
        let error = api::create_with_team_for_test(&f.config, &f.server.url, task)
            .await
            .unwrap_err();
        assert!(
            error.contains("Hollow works only with"),
            "{refused}: {error}"
        );
        assert_eq!(f.server.requests().len(), before, "{refused}: nothing sent");
    }
    let made = api::create_with_team_for_test(
        &f.config,
        &f.server.url,
        TrackerNewTask {
            title: "Mine".into(),
            team_key: Some("per".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(made.key.as_deref(), Some("PER-99"));
    let create = f
        .server
        .requests()
        .into_iter()
        .find(|r| r.operation().as_deref() == Some("IssueCreate"))
        .unwrap();
    assert_eq!(create.variables()["input"]["teamId"], HOL);
    // The shared team stays the default.
    let shared = api::create_with_team_for_test(
        &f.config,
        &f.server.url,
        TrackerNewTask {
            title: "Shared".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(shared.key.as_deref(), Some("ENG-99"));
}

#[tokio::test]
async fn a_hollow_j_write_leaves_the_ledger_readable_for_the_configured_team() {
    let f = Fixture::two_teams().await;
    f.sync().await.unwrap();
    // personalTeamId alone resolves its own key.
    let mut config = f.config.clone();
    config.tracker.personal_team_key = None;
    assert_eq!(api::resolve_team(&config, Some("PER")).unwrap(), HOL);
    // A create in HOL saves the ledger focused on HOL; a reader asking for
    // the configured team still gets AI, and edits are not refused.
    api::create_with_team_for_test(
        &f.config,
        &f.server.url,
        TrackerNewTask {
            title: "Mine".into(),
            team_key: Some("PER".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let state = seams::cached_at(f.root()).unwrap();
    assert_eq!(state.scope, TEAM);
    assert!(state.has_team(HOL));
}

#[tokio::test]
async fn a_failing_personal_pass_does_not_fail_the_shared_teams_sync() {
    let f = Fixture::two_teams().await;
    f.sync().await.unwrap();
    // The personal team's pages now fail; the shared team's run still ends Ok.
    f.remote.lock().unwrap().issues[1]["team"]["id"] = json!(OTHER);
    f.remote.lock().unwrap().leak_scope = true;
    let outcome = f.sync().await;
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(f.ledger().scope, TEAM);
}

/// Review of #1399 (2026-11-13): the syncing flag is per vault root, so one
/// root's pass finishing doesn't report another root's running pass idle,
/// and every spelling of one root reads the same flag.
#[test]
fn one_roots_sync_finishing_leaves_the_others_syncing() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let first = SyncingFlag::start(a.path());
    let second = SyncingFlag::start(b.path());
    assert!(is_syncing(a.path()) && is_syncing(b.path()));
    assert!(is_syncing(&a.path().join(".")));
    drop(second);
    assert!(is_syncing(a.path()), "a's pass still runs");
    assert!(!is_syncing(b.path()));
    drop(first);
    assert!(!is_syncing(a.path()));
}

/// Q-R23: a queued edit made against a project by name journals its provider
/// id, so a rename before delivery still lands on the project that was
/// picked. `rename` changes the remote projects (p1 "Fixture project", p2
/// "Second project", p3 "Third project") before the next pass, which is
/// forced to refresh them (a stale snapshot would hide the rename).
async fn renamed_project_edit(rename: impl FnOnce(&mut Vec<Value>)) {
    let f = Fixture::new().await;
    f.seed_project();
    {
        let mut remote = f.remote.lock().unwrap();
        for (id, ident, name) in [
            ("p2", "P-ENG-2", "Second project"),
            ("p3", "P-ENG-3", "Third project"),
        ] {
            let mut next = remote.projects[0].clone();
            next["id"] = json!(id);
            next["identifier"] = json!(ident);
            next["name"] = json!(name);
            remote.projects.push(next);
        }
    }
    f.sync().await.unwrap();
    let current = mirror::local_values(&f.task()).unwrap();
    let out = edit(
        &f.config,
        TrackerEdit {
            id: "i1".into(),
            values: BTreeMap::from([("project".into(), json!("Second project"))]),
            expected: BTreeMap::from([("project".into(), current["project"].clone())]),
        },
    )
    .await
    .unwrap();
    assert!(out.queued);
    assert_eq!(f.ledger().outbox["i1"].project_id.as_deref(), Some("p2"));
    {
        let mut remote = f.remote.lock().unwrap();
        rename(&mut remote.projects);
        for project in remote.projects.iter_mut() {
            project["updatedAt"] = json!(T1);
        }
    }
    let mut state = f.ledger();
    state.projects_at = None;
    save(f.root(), &state).unwrap();
    let report = f.sync().await.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let requests = f.server.requests();
    let update = requests
        .iter()
        .find(|r| r.operation().as_deref() == Some("IssueUpdate"))
        .expect("the queued project edit was lost");
    assert_eq!(update.variables()["input"], json!({"projectId": "p2"}));
    assert!(f.ledger().outbox.is_empty());
}

#[tokio::test]
async fn a_queued_project_edit_follows_its_id_when_two_names_swap() {
    renamed_project_edit(|p| {
        p[0]["name"] = json!("Second project");
        p[1]["name"] = json!("Renamed project");
    })
    .await;
}

#[tokio::test]
async fn a_queued_project_edit_follows_its_id_when_a_third_takes_the_old_name() {
    renamed_project_edit(|p| {
        p[1]["name"] = json!("Renamed project");
        p[2]["name"] = json!("Second project");
    })
    .await;
}

#[tokio::test]
async fn a_queued_project_edit_is_not_lost_when_the_issues_own_project_takes_the_name() {
    renamed_project_edit(|p| {
        p[0]["name"] = json!("Second project");
        p[1]["name"] = json!("Elsewhere");
    })
    .await;
}

// ---- T1: team routing and Hollow's labels ------------------------------------

#[tokio::test]
async fn hollows_labels_are_created_once_per_team_and_a_second_ensure_creates_nothing() {
    let f = Fixture::two_teams().await;
    let ai = f.adapter();
    let hol = f.adapter().scoped_to(HOL).unwrap();
    let creates = |f: &Fixture| {
        f.server
            .requests()
            .iter()
            .filter(|r| r.operation().as_deref() == Some("LabelCreate"))
            .count()
    };
    let first_ai = ensure_labels_with(&f.config, &ai, TEAM).await.unwrap();
    assert_eq!(first_ai.team, "ENG");
    assert_eq!(first_ai.created, ["needs-review", "commitment"]);
    let first_hol = ensure_labels_with(&f.config, &hol, HOL).await.unwrap();
    assert_eq!(first_hol.team, "PER");
    assert_eq!(
        first_hol.created,
        ["needs-review", "commitment", "agent", "hollow-lesson"]
    );
    assert_eq!(creates(&f), 6);
    // Each create named its own team: never a workspace label.
    for request in f.server.requests() {
        if request.operation().as_deref() == Some("LabelCreate") {
            let team = request.variables()["input"]["teamId"].as_str().unwrap();
            assert!(team == TEAM || team == HOL, "{team}");
        }
    }
    // Idempotent: the second run reads them and creates none.
    let again_ai = ensure_labels_with(&f.config, &ai, TEAM).await.unwrap();
    let again_hol = ensure_labels_with(&f.config, &hol, HOL).await.unwrap();
    assert!(again_ai.created.is_empty() && again_hol.created.is_empty());
    assert_eq!(again_hol.present.len(), 4);
    assert_eq!(creates(&f), 6, "a second ensure makes no create");
    // An adapter for one team can't ensure another's labels.
    assert!(ensure_labels_with(&f.config, &ai, HOL).await.is_err());
    assert!(ensure_labels_with(&f.config, &ai, OTHER).await.is_err());
    assert_eq!(creates(&f), 6);
}

#[tokio::test]
async fn a_hollow_only_label_routes_a_create_to_hol_and_lp_or_pro_are_refused() {
    let f = Fixture::two_teams().await;
    f.remote.lock().unwrap().labels.push(
        json!({"id":"label-hol-c","name":"commitment","color":null,"team":{"id":HOL},"parent":null}),
    );
    f.sync().await.unwrap();
    let made = api::create_with_team_for_test(
        &f.config,
        &f.server.url,
        TrackerNewTask {
            title: "A promise about Hollow".into(),
            labels: vec!["commitment".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(made.key.as_deref(), Some("PER-99"));
    let create = f
        .server
        .requests()
        .into_iter()
        .rfind(|r| r.operation().as_deref() == Some("IssueCreate"))
        .unwrap();
    assert_eq!(create.variables()["input"]["teamId"], HOL);
    assert_eq!(
        create.variables()["input"]["labelIds"],
        json!(["label-hol-c"])
    );
    for refused in ["LP", "PRO"] {
        let before = f.server.requests().len();
        let error = api::create_with_team_for_test(
            &f.config,
            &f.server.url,
            TrackerNewTask {
                title: "Nope".into(),
                labels: vec!["commitment".into()],
                team_key: Some(refused.into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("Hollow works only with"),
            "{refused}: {error}"
        );
        assert_eq!(f.server.requests().len(), before, "{refused}: nothing sent");
    }
    // Without the personal team set up, Hollow-only work is refused, not
    // filed in AI.
    let mut shared_only = f.config.clone();
    shared_only.tracker.personal_team_id = None;
    shared_only.tracker.personal_team_key = None;
    let error = api::resolve_routed(&shared_only, crate::tracker::linear::Work::HollowOnly, None)
        .unwrap_err();
    assert!(error.contains("Personal"), "{error}");
}

// ── Sources of truth (H4a): generations, switches and lost access ───────

mod sources_h4a {
    use super::*;
    use crate::sources::migration::{self, Env, Target};
    use crate::sources::{
        label, owner_of, Access, Context, ItemFacts, ItemKind, LabelKind, LedgerFacts, Owner,
        OwnerKind, WAS_TRACKED_MARK,
    };

    fn tracked_ctx() -> Context {
        let map: BTreeMap<String, Value> = serde_json::from_value(
            json!({"tracker": {"provider":"linear","linear":{"teamId":TEAM,"teamKey":"ENG"}}}),
        )
        .unwrap();
        Context::from_map(&map, Access::default())
    }

    /// The fixture's config.json in its own folder, and a switch's paths.
    fn env(f: &Fixture, dir: &tempfile::TempDir) -> Env {
        let config_path = dir.path().join("config.json");
        let config = json!({"vaultPath": f.root(), "tracker": {"provider":"linear","linear":{"teamId":TEAM,"teamKey":"ENG"}}});
        fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        Env {
            vault: f.root().to_path_buf(),
            migrations: Env::migrations_for(&config_path),
            config_path,
        }
    }

    /// Every request that would change the tracker.
    fn mutations(f: &Fixture) -> usize {
        f.server
            .requests()
            .iter()
            .filter(|r| {
                r.operation().is_some_and(|op| {
                    ["Update", "Archive", "Create", "Delete"]
                        .iter()
                        .any(|verb| op.contains(verb))
                })
            })
            .count()
    }

    fn all_tasks(to: OwnerKind) -> Target {
        Target {
            workspace: None,
            to,
        }
    }

    #[tokio::test]
    async fn a_ledger_from_before_generations_keeps_its_bytes() {
        let f = Fixture::new().await;
        f.sync().await.unwrap();
        f.local_edit("title", "Queued offline");
        f.remote.lock().unwrap().fail = Some(503);
        assert!(f.sync().await.is_err());
        let raw = fs::read(f.root().join(STATE)).unwrap();
        let text = String::from_utf8(raw.clone()).unwrap();
        for key in ["\"generation\"", "\"held\"", "\"storedInHollow\""] {
            assert!(
                !text.contains(key),
                "{key} is written only once a switch sets it"
            );
        }
        let state = f.ledger();
        assert_eq!(state.generation, 0);
        assert_eq!(state.outbox["i1"].generation, 0);
        save(f.root(), &state).unwrap();
        assert_eq!(fs::read(f.root().join(STATE)).unwrap(), raw);
    }

    #[tokio::test]
    async fn a_queued_edit_never_crosses_a_generation() {
        let f = Fixture::two_teams().await;
        f.sync().await.unwrap();
        f.local_edit("title", "Queued under the old owner");
        f.remote.lock().unwrap().fail = Some(503);
        assert!(f.sync().await.is_err());
        assert_eq!(f.ledger().outbox["i1"].generation, 0);
        // The owner changed while it waited (a restored ledger, an older
        // build's queue): the generation moved on.
        let mut state = f.ledger();
        state.generation = 1;
        state.retry_at = None;
        save(f.root(), &state).unwrap();
        f.remote.lock().unwrap().fail = None;
        let report = f.sync().await.unwrap();
        assert_eq!(f.updates(), 0, "the held edit never reached the tracker");
        let state = f.ledger();
        assert!(state.outbox.is_empty());
        assert_eq!(
            state.held["i1"].values["title"],
            json!("Queued under the old owner")
        );
        assert!(state.records["i1"]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("is held")));
        assert!(report.errors.iter().any(|e| e.contains("is held")));
        assert!(f.task().contains("Queued under the old owner"));
        assert_eq!(LedgerFacts::of(&state).held, 1);
        // Another pass still sends nothing for it...
        f.sync().await.unwrap();
        assert_eq!(f.updates(), 0);
        // ...while an edit made under the current generation goes as before.
        let path = f.ledger().records["h1"].path.clone();
        let text = fs::read_to_string(f.root().join(&path)).unwrap();
        let next = frontmatter::patch(
            &text,
            &[(
                "title",
                frontmatter::KeyEdit::Quoted("Sent under generation 1".into()),
            )],
            None,
        )
        .unwrap();
        vault::write_note(f.root(), &path, &next).unwrap();
        f.sync().await.unwrap();
        assert_eq!(f.updates(), 1);
        assert!(f.ledger().outbox.is_empty());
        assert!(f.ledger().held.contains_key("i1"));
        // Rolling the switch back (its generation again) releases the edit
        // to the destination it was queued for.
        let mut state = f.ledger();
        state.generation = 0;
        save(f.root(), &state).unwrap();
        f.sync().await.unwrap();
        assert!(f.ledger().held.is_empty());
        assert_eq!(f.updates(), 2);
        assert_eq!(
            issue(&f.config, "i1").unwrap().issue.title,
            "Queued under the old owner"
        );
    }

    #[tokio::test]
    async fn a_switch_to_hollow_stops_the_pass_and_switching_back_changes_no_mirror_bytes() {
        let f = Fixture::new().await;
        f.seed_project();
        f.sync().await.unwrap();
        let dir = tempfile::Builder::new()
            .prefix("vault-sources-config-")
            .tempdir()
            .unwrap();
        let env = env(&f, &dir);
        let task = f.task();
        let project = f.project_text();
        let changes = mutations(&f);
        let out = migration::apply(&env, &all_tasks(OwnerKind::Vault))
            .await
            .unwrap();
        assert_eq!(out.plan.tracker_writes, 0);
        assert_eq!(
            (out.plan.mirrored_tasks, out.plan.mirrored_projects),
            (1, 1)
        );
        assert!(f.ledger().stored_in_hollow);
        assert_eq!(f.task(), task, "the task mirror is kept byte for byte");
        assert!(f.project_text().starts_with(&project));
        assert!(f.project_text().contains(WAS_TRACKED_MARK));
        // A pass while tasks are stored in Hollow reads and sends nothing.
        let calls = f.server.requests().len();
        f.local_edit("title", "Edited while stored in Hollow");
        let report = f.sync().await.unwrap();
        assert_eq!((report.pulled, report.pushed), (0, 0));
        assert_eq!(f.server.requests().len(), calls, "no request at all");
        assert!(
            f.ledger().outbox.is_empty(),
            "nothing queued for the tracker"
        );
        // The editor says where the task lives instead of queueing it.
        let err = edit(
            &f.config,
            TrackerEdit {
                id: "i1".into(),
                values: BTreeMap::from([("title".into(), json!("From the editor"))]),
                expected: mirror::local_values(&f.task()).unwrap(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("stored in Hollow"), "{err}");
        // Switch back with the file as it was...
        f.replace_task(&task);
        migration::apply(&env, &all_tasks(OwnerKind::Tracker))
            .await
            .unwrap();
        let state = f.ledger();
        assert!(!state.stored_in_hollow);
        assert_eq!(state.generation, 2);
        assert_eq!(f.project_text(), project, "the Was tracked line is gone");
        // ...and its first pass changes no mirror bytes and sends nothing.
        f.sync().await.unwrap();
        assert_eq!(f.task(), task);
        assert_eq!(f.project_text(), project);
        assert_eq!(f.updates(), 0);
        assert_eq!(
            mutations(&f),
            changes,
            "nothing in the tracker was archived, deleted, edited or created"
        );
    }

    #[tokio::test]
    async fn vault_only_keys_are_never_patched() {
        let f = Fixture::new().await;
        f.sync().await.unwrap();
        let with_local = frontmatter::patch(
            &f.task(),
            &[
                ("due_time", frontmatter::KeyEdit::Quoted("09:30".into())),
                (
                    "recurrence",
                    frontmatter::KeyEdit::Quoted("every week".into()),
                ),
                ("my_own_note", frontmatter::KeyEdit::Quoted("keep".into())),
            ],
            None,
        )
        .unwrap();
        f.replace_task(&format!("{with_local}\n## Mine\nExact bytes.\n"));
        let before: BTreeMap<String, String> =
            frontmatter::parse(&f.task()).pairs.into_iter().collect();
        {
            let mut remote = f.remote.lock().unwrap();
            let issue = &mut remote.issues[0];
            issue["title"] = json!("Renamed in the tracker");
            issue["priority"] = json!(2);
            issue["state"] = json!({"id":"state-done","name":"Done","type":"completed"});
            issue["updatedAt"] = json!(T1);
        }
        f.sync().await.unwrap();
        let after: BTreeMap<String, String> =
            frontmatter::parse(&f.task()).pairs.into_iter().collect();
        let binding = crate::sources::resolve(&tracked_ctx(), f.root(), None);
        let changed: Vec<&String> = before
            .keys()
            .chain(after.keys())
            .filter(|key| before.get(*key) != after.get(*key))
            .collect();
        assert!(changed.iter().any(|k| k.as_str() == "title"));
        for key in changed {
            assert_eq!(
                owner_of(&binding, ItemKind::Task, key),
                Owner::Tracker("linear".into()),
                "{key} changed but isn't the tracker's"
            );
        }
        for key in ["due_time", "recurrence", "my_own_note"] {
            assert_eq!(before.get(key), after.get(key), "{key}");
            assert_eq!(owner_of(&binding, ItemKind::Task, key), Owner::Vault);
        }
        assert!(f.task().ends_with("## Mine\nExact bytes.\n"));
    }

    #[tokio::test]
    async fn lost_access_keeps_the_mirror_and_never_reads_as_stored() {
        let f = Fixture::new().await;
        f.sync().await.unwrap();
        let task = f.task();
        let last_read = f.ledger().last_sync.clone();
        f.remote.lock().unwrap().fail = Some(401);
        assert!(f.sync().await.is_err());
        assert_eq!(f.task(), task, "the mirror is kept");
        let facts = LedgerFacts::read(f.root()).unwrap();
        assert!(facts.failures > 0);
        assert_eq!(facts.last_read, last_read);
        let ctx = Context {
            access: Access {
                key: crate::tracker::settings::KeyState::Saved,
                last_read: facts.last_read.clone(),
                failures: facts.failures,
            },
            ..tracked_ctx()
        };
        let status = crate::sources::status(&ctx, f.root(), Some(&facts), Some(&[][..]));
        let row = &status.rows[0];
        assert_eq!(row.binding.owner, OwnerKind::Tracker);
        assert_eq!(row.label.kind, LabelKind::Unreachable);
        assert!(
            row.label.text.starts_with("Linear unreachable since"),
            "{}",
            row.label.text
        );
        let item = label(&row.binding, &ItemFacts::from_text(&task));
        assert_eq!(item.kind, LabelKind::Unreachable);
        assert_ne!(item.text, "Stored in Hollow");
        assert_eq!(
            owner_of(&row.binding, ItemKind::Task, "title"),
            Owner::Tracker("linear".into())
        );
    }
}
