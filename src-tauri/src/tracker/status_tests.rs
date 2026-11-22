//! A18 (the Friday status) and A19 (a canvas copy on an issue) against a
//! fixture mirror and `tracker::stub`. No live tracker.

use super::*;
use crate::tracker::linear::LinearAdapter;
use crate::tracker::status;
use crate::tracker::stub::{self, StubReply, StubRequest, StubServer};
use chrono::TimeZone;
use std::sync::Mutex as StdMutex;

const TEAM: &str = crate::tracker::linear::TEAM_ID;
const OTHER: &str = "team-other-0001";
const NOTE: &str = "workspaces/ws-receipts/outputs/status/2026-11-13-friday-status.md";
const DIGEST: &str = "workspaces/main/outputs/status/2026-11-13-friday-status.md";

/// Friday 13 November 2026, 15:00 local.
fn friday() -> DateTime<Local> {
    Local.with_ymd_and_hms(2026, 11, 13, 15, 0, 0).unwrap()
}

/// A time `days` before [`friday`], in RFC 3339.
fn ago(days: i64) -> String {
    (friday() - ChronoDuration::days(days))
        .with_timezone(&Utc)
        .to_rfc3339()
}

fn issue(id: &str, key: &str, kind: StateKind, updated: &str, due: Option<&str>) -> TrackerIssue {
    TrackerIssue {
        id: id.into(),
        key: key.into(),
        url: format!("https://linear.app/example/issue/{key}"),
        title: format!("Fixture {key}"),
        description: String::new(),
        state: TrackerState {
            id: format!("s-{}", kind.as_str()),
            name: kind.as_str().into(),
            kind,
            position: 0.0,
            color: None,
        },
        assignee: None,
        priority: 0,
        due: due.map(str::to_string),
        project_id: Some("p1".into()),
        labels: vec![],
        parent_id: None,
        team_id: TEAM.into(),
        updated_at: updated.into(),
        archived_at: None,
        trashed: false,
        estimate: None,
        cycle: None,
        milestone: None,
        creator: None,
        created_at: "2026-11-01T00:00:00Z".into(),
        completed_at: None,
        children: vec![],
        relations: vec![],
        links: vec![],
        attachments: vec![],
        comments: vec![],
    }
}

fn record(
    id: &str,
    kind: &str,
    path: &str,
    issue: Option<TrackerIssue>,
    project: Option<TrackerProject>,
) -> Record {
    Record {
        id: id.into(),
        key: id.into(),
        kind: kind.into(),
        path: path.into(),
        workspace_id: None,
        base: Values::new(),
        conflicts: vec![],
        error: None,
        remote_missing: false,
        state: "active".into(),
        issue,
        project,
    }
}

/// The fixture mirror: project p1 (Receipts rebuild) with
/// ENG-1 done this week, ENG-2 done three weeks ago, ENG-3 in progress,
/// ENG-4 overdue, ENG-5 blocked by ENG-3, ENG-6 next (due), ENG-7 next (no due).
fn mirror() -> Vec<TrackerIssue> {
    let mut blocked = issue("i5", "ENG-5", StateKind::Unstarted, &ago(1), None);
    blocked.relations.push(TrackerRelation {
        kind: RelationKind::BlockedBy,
        key: "ENG-3".into(),
    });
    let mut next = issue(
        "i6",
        "ENG-6",
        StateKind::Unstarted,
        &ago(1),
        Some("2026-11-20"),
    );
    next.assignee = Some(TrackerUser {
        id: "u1".into(),
        name: "Sam Taylor".into(),
        email: None,
        display_name: None,
    });
    vec![
        issue("i1", "ENG-1", StateKind::Completed, &ago(1), None),
        issue("i2", "ENG-2", StateKind::Completed, &ago(21), None),
        issue("i3", "ENG-3", StateKind::Started, &ago(2), None),
        issue(
            "i4",
            "ENG-4",
            StateKind::Unstarted,
            &ago(5),
            Some("2026-11-10"),
        ),
        blocked,
        next,
        issue("i7", "ENG-7", StateKind::Unstarted, &ago(1), None),
    ]
}

fn project(teams: &[&str]) -> TrackerProject {
    TrackerProject {
        id: "p1".into(),
        key: Some("P-ENG-1".into()),
        url: "https://linear.app/example/project/p1".into(),
        name: "Receipts rebuild".into(),
        description: String::new(),
        status: TrackerProjectStatus {
            name: "In Progress".into(),
            kind: "started".into(),
        },
        lead: None,
        start_date: None,
        target_date: Some("2026-12-01".into()),
        team_ids: teams.iter().map(|t| t.to_string()).collect(),
        updated_at: ago(1),
        progress: None,
        links: vec![],
    }
}

#[derive(Default)]
struct Remote {
    /// The project's teams as the tracker reports them.
    project_teams: Vec<String>,
    /// HTTP status for these operations instead of a reply.
    fail: BTreeMap<String, u16>,
    /// The team the tracker says an issue is in.
    issue_team: Option<String>,
    /// Whether the status note existed when the status update arrived.
    note_first: Option<bool>,
    vault: std::path::PathBuf,
}

fn reply(req: &StubRequest, remote: &mut Remote) -> StubReply {
    if let Some(refused) = stub::unscoped(req, TEAM) {
        return refused;
    }
    if req.method == "PUT" {
        return StubReply {
            status: 200,
            headers: vec![],
            body: String::new(),
        };
    }
    let op = req.operation().unwrap_or_default();
    if let Some(code) = remote.fail.get(&op) {
        return StubReply::status(*code);
    }
    let host = req.headers.get("host").cloned().unwrap_or_default();
    let vars = req.variables();
    match op.as_str() {
        "Team" => StubReply::data(json!({"team":{"id":TEAM,"key":"ENG","name":"Engineering"}})),
        "ProjectTarget" => {
            let teams: Vec<Value> = remote
                .project_teams
                .iter()
                .map(|t| json!({"id": t}))
                .collect();
            StubReply::data(json!({"projects":{"nodes":[{"id":"p1","teams":{"nodes":teams}}]}}))
        }
        "ProjectStatusUpdate" => {
            remote.note_first = Some(remote.vault.join(NOTE).exists());
            let project = vars["input"]["projectId"].clone();
            StubReply::data(
                json!({"projectUpdateCreate":{"success":true,"projectUpdate":{"id":"pu-1","url":"https://linear.app/example/update/pu-1","createdAt":"2026-11-13T20:00:00.000Z","project":{"id":project}}}}),
            )
        }
        "FileUpload" => StubReply::data(json!({"fileUpload":{"success":true,"uploadFile":{
            "uploadUrl": format!("http://{host}/upload/canvas"),
            "assetUrl": format!("http://{host}/assets/canvas"),
            "headers": [{"key":"x-upload-meta","value":"hollow"}]
        }}})),
        "IssueTarget" => {
            let id = vars
                .pointer("/filter/id/eq")
                .and_then(Value::as_str)
                .unwrap_or("");
            let team = remote.issue_team.clone().unwrap_or_else(|| TEAM.into());
            StubReply::data(
                json!({"issues":{"nodes":[{"id":id,"identifier":"ENG-3","team":{"id":team}}]}}),
            )
        }
        "AttachmentCreate" => {
            let input = &vars["input"];
            StubReply::data(json!({"attachmentCreate":{"success":true,"attachment":{
                "id":"att-1","title":input["title"],"url":input["url"],"sourceType":"upload",
                "createdAt":"2026-11-13T20:00:00.000Z","issue":{"id":input["issueId"],"team":{"id":TEAM}}
            }}}))
        }
        _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unknown operation"),
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
    remote: Arc<StdMutex<Remote>>,
    server: StubServer,
}

impl Fixture {
    async fn new(post: bool) -> Self {
        Self::with_team(post, TEAM).await
    }

    async fn with_team(post: bool, team: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = root.join("workspaces/ws-receipts");
        fs::create_dir_all(&ws).unwrap();
        fs::create_dir_all(root.join("workspaces/main")).unwrap();
        fs::create_dir_all(root.join("workspaces/ws-plain")).unwrap();
        fs::write(
            ws.join("PROJECT.md"),
            "---\ntitle: \"Receipts rebuild\"\ntracker: \"linear\"\ntracker_project: \"p1\"\n---\n\n# Receipts rebuild\n",
        )
        .unwrap();
        fs::write(
            ws.join("BUILD_LOG.md"),
            "# Build log\n\n## 2026-11-12 · Shipped the matcher\n\nWhy.\n\n## 2026-10-30 · Older work\n",
        )
        .unwrap();
        fs::write(
            root.join("workspaces/ws-plain/PROJECT.md"),
            "---\ntitle: \"Vault only\"\n---\n",
        )
        .unwrap();
        let mut ledger = Ledger::new("linear", TEAM);
        for i in mirror() {
            let (id, path) = (i.id.clone(), format!("tasks/tracker-{}.md", i.id));
            ledger
                .records
                .insert(id.clone(), record(&id, "task", &path, Some(i), None));
        }
        ledger.records.insert(
            "p1".into(),
            record(
                "p1",
                "project",
                "workspaces/ws-receipts/PROJECT.md",
                None,
                Some(project(&[TEAM])),
            ),
        );
        save(root, &ledger).unwrap();
        let config = serde_json::from_value(json!({
            "vaultPath": root,
            "notion": {"twoWaySync": false},
            "tracker": {"provider": "linear", "postStatusUpdates": post,
                        "linear": {"teamId": team, "teamKey": "ENG"}}
        }))
        .unwrap();
        let remote = Arc::new(StdMutex::new(Remote {
            project_teams: vec![TEAM.into()],
            vault: root.to_path_buf(),
            ..Remote::default()
        }));
        let handler = remote.clone();
        let server = stub::serve(move |req| reply(req, &mut handler.lock().unwrap())).await;
        Fixture {
            _dir: dir,
            config,
            remote,
            server,
        }
    }

    fn root(&self) -> &Path {
        &self.config.vault_path
    }

    fn adapter(&self) -> LinearAdapter {
        LinearAdapter::with_url(&self.server.url, TEAM, "lin_api_fixture_only_never_real").unwrap()
    }

    fn ops(&self) -> Vec<String> {
        self.server
            .requests()
            .iter()
            .map(|r| {
                if r.method == "PUT" {
                    format!("PUT {}", r.path)
                } else {
                    r.operation().unwrap_or_default()
                }
            })
            .collect()
    }

    fn count(&self, op: &str) -> usize {
        self.ops().iter().filter(|o| *o == op).count()
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root().join(rel)).unwrap()
    }

    fn ledger(&self) -> Ledger {
        load(self.root(), "linear", TEAM).unwrap()
    }

    fn canvas(&self, revision: u64) -> posts::CanvasCopy {
        posts::CanvasCopy {
            workspace_id: "ws-receipts".into(),
            canvas_id: "release-status".into(),
            revision,
            fence_sha256: "ab".repeat(32),
            title: "Release status".into(),
            html: "<hc-canvas title=\"Release status\"><hc-text>Green.</hc-text></hc-canvas>"
                .into(),
        }
    }
}

fn notices() -> Vec<crate::notifications::NotificationInput> {
    status::NOTICES.with(|sent| std::mem::take(&mut *sent.borrow_mut()))
}

// ── A18 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_status_reads_done_this_week_overdue_blocked_and_next_from_the_mirror() {
    let f = Fixture::new(false).await;
    let written = status::write_notes(&f.config, None, friday()).unwrap();
    let note = f.read(NOTE);
    let section = |name: &str| -> String {
        let start = note
            .find(&format!("## {name}\n"))
            .unwrap_or_else(|| panic!("{name}"));
        let rest = &note[start + name.len() + 4..];
        rest[..rest.find("\n## ").unwrap_or(rest.len())].to_string()
    };
    let done = section("Done this week");
    assert!(done.contains("[ENG-1]"), "{done}");
    assert!(
        !done.contains("ENG-2"),
        "three weeks ago is not this week: {done}"
    );
    assert!(section("In progress").contains("[ENG-3]"));
    let stuck = section("Blocked or overdue");
    assert!(
        stuck.contains("[ENG-4]") && stuck.contains("due 2026-11-10, overdue"),
        "{stuck}"
    );
    assert!(
        stuck.contains("[ENG-5]") && stuck.contains("blocked by ENG-3"),
        "{stuck}"
    );
    let next = section("Next up");
    let (six, seven) = (next.find("ENG-6").unwrap(), next.find("ENG-7").unwrap());
    assert!(six < seven, "a due date comes first: {next}");
    assert!(next.contains("Sam Taylor, due 2026-11-20"), "{next}");
    assert!(
        !next.contains("ENG-4") && !next.contains("ENG-5"),
        "stuck work is listed once"
    );
    let log = section("Build log this week");
    assert!(
        log.contains("2026-11-12 Shipped the matcher") && !log.contains("Older work"),
        "{log}"
    );
    let fm = crate::frontmatter::parse(&note);
    assert_eq!(fm.get("tracker_project"), Some("p1"));
    assert_eq!(fm.get("week"), Some("2026-W46"));
    let p = &written.status.projects[0];
    assert_eq!((p.done, p.in_progress, p.blocked, p.next), (1, 1, 2, 2));
    assert_eq!(p.health, ProjectHealth::AtRisk);
    assert!(
        note.contains("At risk: 1 done this week, 1 in progress, 2 blocked or overdue, 2 next up.")
    );
    assert!(
        !note.contains('\u{2014}') && !note.contains('\u{2013}'),
        "no long dashes"
    );
    // Health from the counts.
    assert_eq!(status::health(2, 1, 0, 0), ProjectHealth::OnTrack);
    assert_eq!(status::health(0, 0, 1, 0), ProjectHealth::OffTrack);
    assert_eq!(status::health(1, 1, 3, 3), ProjectHealth::OffTrack);
}

#[tokio::test]
async fn notes_land_in_the_workspace_and_the_coo_digest_with_one_notification() {
    let f = Fixture::new(false).await;
    notices();
    let out = status::run(&f.config, None, friday(), Some(&f.adapter()))
        .await
        .unwrap();
    assert_eq!(out.date, "2026-11-13");
    assert_eq!(out.projects.len(), 1, "only tracked workspaces");
    assert_eq!(out.projects[0].path, NOTE);
    assert_eq!(out.digest_path.as_deref(), Some(DIGEST));
    let digest = f.read(DIGEST);
    let fm = crate::frontmatter::parse(&digest);
    assert_eq!(
        fm.get("title"),
        Some(out.herald_line.as_str()),
        "the Herald reads the title"
    );
    assert!(out
        .herald_line
        .starts_with("Friday status 2026-11-13: 1 project, 1 done this week"));
    assert!(digest.contains("](../../../ws-receipts/outputs/status/2026-11-13-friday-status.md)"));
    let sent = notices();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].title, "Friday status is ready");
    assert_eq!(
        sent[0].action_url.as_deref(),
        Some(&*format!("vault:{DIGEST}"))
    );
    assert!(
        out.copies.is_empty() && f.server.requests().is_empty(),
        "the setting is off"
    );

    // One project: its note only, and the notification opens it.
    fs::remove_file(f.root().join(DIGEST)).unwrap();
    let one = status::run(&f.config, Some("ws-receipts"), friday(), Some(&f.adapter()))
        .await
        .unwrap();
    assert!(one.digest_path.is_none() && !f.root().join(DIGEST).exists());
    assert_eq!(
        notices()[0].action_url.as_deref(),
        Some(&*format!("vault:{NOTE}"))
    );
    for refused in ["ws-plain", "main", "missing"] {
        assert!(
            status::run(&f.config, Some(refused), friday(), None)
                .await
                .is_err(),
            "{refused}"
        );
    }
}

#[tokio::test]
async fn the_tracker_copy_goes_only_with_the_setting_after_the_note_once_a_week() {
    let f = Fixture::new(true).await;
    let out = status::run(&f.config, None, friday(), Some(&f.adapter()))
        .await
        .unwrap();
    assert_eq!(
        f.remote.lock().unwrap().note_first,
        Some(true),
        "the note is written first"
    );
    assert_eq!(out.copies.len(), 1);
    assert!(out.copies[0].posted, "{}", out.copies[0].note);
    assert_eq!(f.count("ProjectStatusUpdate"), 1);
    let sent = f
        .server
        .requests()
        .into_iter()
        .find(|r| r.operation().as_deref() == Some("ProjectStatusUpdate"))
        .unwrap();
    assert_eq!(sent.variables()["input"]["health"], "atRisk");
    assert!(sent.variables()["input"]["body"]
        .as_str()
        .unwrap()
        .contains("## Blocked or overdue"));
    let key = posts::status_key("p1", "2026-W46");
    assert_eq!(
        f.ledger().status_posts[&key].update_id.as_deref(),
        Some("pu-1")
    );
    // Again the same week: the note is rewritten, the tracker isn't.
    let again = status::run(&f.config, None, friday(), Some(&f.adapter()))
        .await
        .unwrap();
    assert!(again.copies[0].posted);
    assert_eq!(f.count("ProjectStatusUpdate"), 1);
}

#[tokio::test]
async fn a_failed_copy_keeps_the_note_and_records_why() {
    let f = Fixture::new(true).await;
    f.remote
        .lock()
        .unwrap()
        .fail
        .insert("ProjectStatusUpdate".into(), 500);
    let out = status::run(&f.config, None, friday(), Some(&f.adapter()))
        .await
        .unwrap();
    assert!(f.root().join(NOTE).exists() && f.root().join(DIGEST).exists());
    assert!(!out.copies[0].posted);
    assert!(
        out.copies[0].note.contains("unconfirmed"),
        "{}",
        out.copies[0].note
    );
    let key = posts::status_key("p1", "2026-W46");
    assert!(f.ledger().status_posts[&key].error.is_some());
    // An unconfirmed copy is never resent.
    f.remote.lock().unwrap().fail.clear();
    status::run(&f.config, None, friday(), Some(&f.adapter()))
        .await
        .unwrap();
    assert_eq!(f.count("ProjectStatusUpdate"), 1);
}

#[tokio::test]
async fn a_status_copy_outside_team_ai_is_refused_before_it_is_sent() {
    // A project the tracker shares with another team is read, never written.
    let f = Fixture::new(true).await;
    f.remote.lock().unwrap().project_teams = vec![TEAM.into(), OTHER.into()];
    let out = status::run(&f.config, None, friday(), Some(&f.adapter()))
        .await
        .unwrap();
    assert!(!out.copies[0].posted);
    assert_eq!(f.count("ProjectStatusUpdate"), 0);
    assert!(
        f.ledger().status_posts.is_empty(),
        "nothing went out, so it may be tried again"
    );
    assert!(f.root().join(NOTE).exists());

    // Another team configured: refused before any request.
    let g = Fixture::with_team(true, OTHER).await;
    let refused = post_status_with(
        &g.config,
        &g.adapter(),
        "p1",
        "Body",
        ProjectHealth::OnTrack,
        "2026-W46",
    )
    .await
    .unwrap_err();
    assert_eq!(refused, posts::TEAM_AI_ONLY);
    assert!(g.server.requests().is_empty());

    // The setting off: refused before any request too.
    let h = Fixture::new(false).await;
    assert!(post_status_with(
        &h.config,
        &h.adapter(),
        "p1",
        "Body",
        ProjectHealth::OnTrack,
        "2026-W46"
    )
    .await
    .is_err());
    assert!(h.server.requests().is_empty());
}

// ── A19 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_canvas_copy_uploads_then_attaches_once() {
    let f = Fixture::new(false).await;
    let out = attach_canvas_with(&f.config, &f.adapter(), &f.canvas(3), "i3")
        .await
        .unwrap();
    assert_eq!(out.id.as_deref(), Some("att-1"));
    assert_eq!(out.key.as_deref(), Some("ENG-3"));
    assert!(out.note.contains("stays in Hollow"), "{}", out.note);
    assert_eq!(
        f.ops(),
        [
            "Team",
            "FileUpload",
            "PUT /upload/canvas",
            "IssueTarget",
            "AttachmentCreate"
        ]
    );
    let requests = f.server.requests();
    let upload = &requests[1];
    assert_eq!(upload.variables()["filename"], "release-status.canvas.html");
    assert_eq!(
        upload.variables()["contentType"],
        "text/html; charset=utf-8"
    );
    let put = &requests[2];
    assert!(
        !put.headers.contains_key("authorization"),
        "the signed address gets no key"
    );
    assert_eq!(put.headers["x-upload-meta"], "hollow");
    assert!(put.headers["content-type"].starts_with("text/html"));
    let attach = &requests[4];
    assert_eq!(attach.variables()["input"]["issueId"], "i3");
    assert_eq!(
        attach.variables()["input"]["title"],
        "Canvas: Release status (v3)"
    );
    assert!(attach.variables()["input"]["url"]
        .as_str()
        .unwrap()
        .ends_with("/assets/canvas"));
    let ledger = f.ledger();
    let mirrored = ledger.records["i3"].issue.as_ref().unwrap();
    assert!(mirrored.attachments.iter().any(|a| a.id == "att-1"));

    // The same version on the same issue: nothing new goes out.
    let again = attach_canvas_with(&f.config, &f.adapter(), &f.canvas(3), "i3")
        .await
        .unwrap();
    assert!(again.note.contains("already on ENG-3"), "{}", again.note);
    assert_eq!(f.count("FileUpload"), 1);
    assert_eq!(f.count("AttachmentCreate"), 1);
    // A new version is a new copy.
    attach_canvas_with(&f.config, &f.adapter(), &f.canvas(4), "i3")
        .await
        .unwrap();
    assert_eq!(f.count("AttachmentCreate"), 2);
}

#[tokio::test]
async fn an_uncertain_attach_retries_with_the_same_upload() {
    let f = Fixture::new(false).await;
    f.remote
        .lock()
        .unwrap()
        .fail
        .insert("AttachmentCreate".into(), 500);
    let out = attach_canvas_with(&f.config, &f.adapter(), &f.canvas(3), "i3")
        .await
        .unwrap();
    assert!(
        out.id.is_none() && out.note.contains("unconfirmed"),
        "{}",
        out.note
    );
    f.remote.lock().unwrap().fail.clear();
    let done = attach_canvas_with(&f.config, &f.adapter(), &f.canvas(3), "i3")
        .await
        .unwrap();
    assert_eq!(done.id.as_deref(), Some("att-1"));
    assert_eq!(f.count("FileUpload"), 1, "the upload is kept");
    assert_eq!(f.count("AttachmentCreate"), 2);
}

#[tokio::test]
async fn a_canvas_copy_outside_team_ai_is_refused() {
    // Another team configured: nothing is read or sent.
    let g = Fixture::with_team(false, OTHER).await;
    assert_eq!(
        attach_canvas_with(&g.config, &g.adapter(), &g.canvas(1), "i3")
            .await
            .unwrap_err(),
        posts::TEAM_AI_ONLY
    );
    assert!(g.server.requests().is_empty());

    // An issue the mirror doesn't hold: refused before any upload.
    let f = Fixture::new(false).await;
    assert!(
        attach_canvas_with(&f.config, &f.adapter(), &f.canvas(1), "elsewhere")
            .await
            .is_err()
    );
    assert_eq!(f.count("FileUpload"), 0);

    // The tracker says the issue moved to another team: never attached.
    f.remote.lock().unwrap().issue_team = Some(OTHER.into());
    let moved = attach_canvas_with(&f.config, &f.adapter(), &f.canvas(1), "i3").await;
    assert!(moved.is_err());
    assert_eq!(f.count("AttachmentCreate"), 0);
}

#[test]
fn canvas_copy_names_and_keys_are_stable() {
    assert_eq!(
        posts::canvas_filename("Q4: Release / status!", "c1"),
        "q4-release-status.canvas.html"
    );
    assert_eq!(posts::canvas_filename("   ", "c1"), "c1.canvas.html");
    let key = posts::attach_key("ws", "c1", 3, "i3");
    assert_eq!(key, posts::attach_key("ws", "c1", 3, "i3"));
    assert_ne!(key, posts::attach_key("ws", "c1", 4, "i3"));
    assert_ne!(key, posts::attach_key("ws", "c1", 3, "i4"));
    assert!(valid_id(&key) && valid_id(&posts::status_key("p1", "2026-W46")));
}
