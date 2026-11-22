//! The importer against a fixture vault, a canned Notion (served by
//! `tracker::stub`) and the tracker adapter on the same stub.

use super::notion::{NotionClient, Request};
use super::*;
use crate::tracker::linear::LinearAdapter;
use crate::tracker::stub::{self, StubReply, StubRequest, StubServer};
use serde_json::json;
use std::fs;
use std::sync::{Arc, Mutex};

const TEAM: &str = DEFAULT_TEAM_ID;
const OTHER: &str = "team-lp-0001";
const TOKEN: &str = "fixtureNotionToken0123456789abcdef";
const KEY: &str = "lin_api_fixture_key_never_shown_0123456789";
const T0: &str = "2026-11-08T05:00:00.000Z";

const P1: &str = "35fb237252968170a333fceb8eb30fb8";
const P2: &str = "35fb0000000000000000000000000002";
const T_OPEN: &str = "3dcb237252968009b21cc8d083b951ee";
const T_SPLIT: &str = "406b2372529681f4ac4dc2cd5b564607";
const T_DONE: &str = "3f9b0000000000000000000000000003";
const T_DIFF: &str = "4000000000000000000000000000000a";
const T_CITE: &str = "4000000000000000000000000000000b";
const T_NONE: &str = "4000000000000000000000000000000c";
const MEETING: &str = "34db0000000000000000000000000001";
const PERSON_MAP: &str = "248f0000000000000000000000000001";
const PERSON_MEMBER: &str = "a07b0000000000000000000000000001";
const DOMAIN: &str = "92ef0000000000000000000000000001";

fn source_id(kind: records::Kind) -> String {
    format!("{:032x}", 0xd5_0000 + kind as u64)
}

fn rich(kind: &str, text: &str) -> Value {
    json!({ "id": kind, "type": kind, kind: [{ "plain_text": text }] })
}
fn select(kind: &str, name: &str) -> Value {
    json!({ "id": "s", "type": kind, kind: { "name": name } })
}
fn date(start: &str) -> Value {
    json!({ "id": "d", "type": "date", "date": { "start": start, "end": null } })
}
fn relation(ids: &[&str]) -> Value {
    json!({ "id": "r", "type": "relation", "has_more": false,
        "relation": ids.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>() })
}
fn people(names: &[&str]) -> Value {
    json!({ "id": "p", "type": "people", "people": names.iter().map(|n| json!({ "name": n })).collect::<Vec<_>>() })
}
fn task_id(n: u64) -> Value {
    json!({ "id": "u", "type": "unique_id", "unique_id": { "prefix": "TASK", "number": n } })
}
fn page(id: &str, props: Value) -> Value {
    json!({ "object": "page", "id": id, "url": format!("https://app.notion.com/p/{id}"),
        "created_time": "2026-10-01T12:00:00.000Z", "last_edited_time": "2026-11-08T03:14:00.000Z",
        "properties": props })
}

fn notion_rows(base: &str) -> BTreeMap<String, Vec<Value>> {
    use records::Kind::*;
    let task = |id: &str, name: &str, status: &str, n: u64| {
        page(
            id,
            json!({ "Task name": rich("title", name), "Status": select("status", status), "Task ID": task_id(n),
            "Project": relation(&[P1]) }),
        )
    };
    let mut open = task(T_OPEN, "Confirm IT is finished", "Not started", 10);
    open["properties"]["Meeting"] = relation(&[MEETING]);
    open["properties"]["Assignee"] = people(&["Sam Taylor"]);
    let mut done = task(T_DONE, "Ship the old thing", "Done", 12);
    done["properties"]["Completed"] = date("2026-10-20");
    let rows = [
        (
            Projects,
            vec![
                page(
                    P1,
                    json!({
                        "Name": rich("title", "Shift Log Form"), "Status": select("status", "Active"),
                        "Description": rich("rich_text", "The Notion description."), "Target date": date("2026-12-01"),
                        "Department": select("select", "AI"), "Hours Saved / yr": { "id": "h", "type": "number", "number": 120 },
                        "Requested by": people(&["Sam Taylor"]), "Stakeholders": relation(&[PERSON_MAP]),
                        "Meetings": relation(&[MEETING]),
                        "Working Files": { "id": "w", "type": "files", "files": [
                            { "name": "notes.txt", "type": "file", "file": { "url": format!("{base}/files/small.txt") } },
                            { "name": "huge.bin", "type": "file", "file": { "url": format!("{base}/files/big.bin") } },
                            { "name": "Drive", "type": "external", "external": { "url": "https://example.com/drive" } }
                        ] }
                    }),
                ),
                page(
                    P2,
                    json!({ "Name": rich("title", "Old idea"), "Status": select("status", "Idea") }),
                ),
            ],
        ),
        (
            Tasks,
            vec![
                open,
                task(
                    T_SPLIT,
                    "Shift Log app: accept counter exports",
                    "In progress",
                    11,
                ),
                done,
                task(T_DIFF, "Renamed since", "Planning", 13),
                task(T_CITE, "Cited by its issue", "Not started", 14),
                task(T_NONE, "Nobody filed this", "Not started", 15),
            ],
        ),
        (
            Meetings,
            vec![page(
                MEETING,
                json!({
                    "Name": rich("title", "Weekly sync"), "Date": date("2026-11-03"), "Category": select("select", "Team"),
                    "Attendees": relation(&[PERSON_MAP]), "Projects": relation(&[P1]), "Action Items": relation(&[T_OPEN])
                }),
            )],
        ),
        (
            PeopleMap,
            vec![page(
                PERSON_MAP,
                json!({
                    "Name": rich("title", "Sam Taylor"), "Relationship": select("select", "Peer")
                }),
            )],
        ),
        (
            PeopleMembers,
            vec![page(
                PERSON_MEMBER,
                json!({
                    "Name": rich("title", "Sam  taylor"), "Email": { "id": "e", "type": "email", "email": "sam@example.com" }
                }),
            )],
        ),
        (
            Domains,
            vec![page(DOMAIN, json!({ "Name": rich("title", "Payroll") }))],
        ),
        (WarRoom, vec![]),
        (Captures, vec![]),
        (Docs, vec![]),
        (AiSubscriptions, vec![]),
    ];
    rows.into_iter().map(|(k, v)| (source_id(k), v)).collect()
}

fn notion_reply(req: &StubRequest, base: &str) -> StubReply {
    let ok = |v: Value| StubReply {
        status: 200,
        headers: Vec::new(),
        body: v.to_string(),
    };
    if req.path.starts_with("/files/") {
        let body = if req.path.ends_with("big.bin") {
            "x".repeat(2048)
        } else {
            "hello".into()
        };
        return StubReply {
            status: 200,
            headers: Vec::new(),
            body,
        };
    }
    if req.headers.get("authorization").map(String::as_str)
        != Some(format!("Bearer {TOKEN}").as_str())
    {
        return StubReply { status: 401, headers: Vec::new(),
            body: json!({ "code": "unauthorized", "message": format!("API token {TOKEN} is invalid.") }).to_string() };
    }
    let path = req.path.split('?').next().unwrap();
    let rest = path.strip_prefix("/v1/data_sources/").unwrap_or("");
    if let Some(id) = rest.strip_suffix("/query") {
        let mut rows = notion_rows(base).remove(id).unwrap_or_default();
        // Two pages for the tasks, to exercise the cursor.
        if id == source_id(records::Kind::Tasks) {
            let second = rows.split_off(3);
            if req.body["start_cursor"] == "c2" {
                return ok(json!({ "results": second, "has_more": false, "next_cursor": null }));
            }
            return ok(json!({ "results": rows, "has_more": true, "next_cursor": "c2" }));
        }
        return ok(json!({ "results": rows, "has_more": false, "next_cursor": null }));
    }
    if !rest.is_empty() {
        return ok(
            json!({ "object": "data_source", "id": rest, "title": [{ "plain_text": format!("Fixture {rest}") }] }),
        );
    }
    if path.starts_with("/v1/blocks/") {
        let blocks = if path.contains(MEETING) {
            json!([{ "id": "b1", "type": "heading_2", "has_children": false, "heading_2": { "rich_text": [{ "plain_text": "Notes" }] } },
                   { "id": "b2", "type": "bulleted_list_item", "has_children": false, "bulleted_list_item": { "rich_text": [{ "plain_text": "Ship it" }] } }])
        } else {
            json!([])
        };
        return ok(json!({ "results": blocks, "has_more": false, "next_cursor": null }));
    }
    StubReply::status(404)
}

fn issue(id: &str, key: &str, title: &str, team: &str, description: &str) -> Value {
    let mut node = stub::issue_node(id, key, team, T0);
    node["title"] = json!(title);
    node["description"] = json!(description);
    node["project"] = json!({ "id": "p1" });
    node["attachments"] = json!({ "nodes": [] });
    node
}

fn tracker_issues() -> Vec<Value> {
    vec![
        issue(
            "i1",
            "ENG-1",
            "Confirm IT's part is finished",
            TEAM,
            "Remote text.",
        ),
        issue("i24", "ENG-24", "Run the first real upload", TEAM, ""),
        issue("i27", "ENG-27", "Counter export: duplicate paid", TEAM, ""),
        issue("i31", "ENG-31", "Counter export: zero rows", TEAM, ""),
        issue("i40", "ENG-40", "New title", TEAM, ""),
        issue(
            "i50",
            "ENG-50",
            "Something else",
            TEAM,
            &format!("Hollow task: `tasks/notion-{T_CITE}.md`"),
        ),
        issue("i60", "ENG-60", "Unmapped issue", TEAM, ""),
        issue("lp1", "LP-1", "Another team's issue", OTHER, ""),
    ]
}

fn tracker_reply(req: &StubRequest) -> StubReply {
    let connection = |nodes: Vec<Value>| json!({ "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } });
    let page =
        |field: &str, nodes: Vec<Value>| StubReply::data(json!({ field: connection(nodes) }));
    let filter = &req.variables()["filter"];
    let ids = filter.pointer("/id/in").and_then(Value::as_array).cloned();
    match req.operation().as_deref() {
        Some("Viewer") => StubReply::data(
            json!({ "viewer": { "id": "u1", "name": "Fixture Person", "email": null } }),
        ),
        Some("Team") => {
            StubReply::data(json!({ "team": { "id": TEAM, "key": "ENG", "name": "Fixture AI" } }))
        }
        Some("States") => StubReply::data(json!({ "team": { "id": TEAM, "states": connection(vec![
            json!({ "id": "state-todo", "name": "Todo", "type": "unstarted", "position": 0, "team": { "id": TEAM } })]) } })),
        Some("Members") => {
            StubReply::data(json!({ "team": { "id": TEAM, "members": connection(vec![
            json!({ "id": "u2", "name": "Sam Taylor", "email": null })]) } }))
        }
        Some("Labels") => page("issueLabels", vec![]),
        Some("Cycles") => {
            StubReply::data(json!({ "team": { "id": TEAM, "cycles": connection(vec![]) } }))
        }
        Some("Projects" | "ProjectTarget") => page(
            "projects",
            vec![json!({
            "id": "p1", "identifier": "P-ENG-10", "url": "https://linear.app/example/project/p1", "name": "Shift Log Form",
            "description": format!("Vault: `workspaces/notion-{P1}`"), "status": { "name": "In Progress", "type": "started" },
            "startDate": null, "targetDate": null, "updatedAt": T0, "progress": 0.5, "lead": null,
            "teams": { "nodes": [{ "id": TEAM }] }, "externalLinks": { "nodes": [] } })],
        ),
        Some("Milestones") => page("projects", vec![]),
        // The list leaks another team's node; the adapter must drop it.
        Some("Issues") => page("issues", tracker_issues()),
        Some("IssueDetail" | "IssueTarget" | "Attachments") => page(
            "issues",
            tracker_issues()
                .into_iter()
                .filter(|n| n["team"]["id"] == TEAM)
                .filter(|n| ids.as_ref().is_none_or(|ids| ids.contains(&n["id"])))
                .filter(|n| filter.pointer("/id/eq").is_none_or(|id| &n["id"] == id))
                .collect(),
        ),
        Some("Comments") => page(
            "comments",
            vec![json!({ "id": "c1", "body": "Looks right.", "createdAt": T0,
            "updatedAt": T0, "issue": { "id": "i1", "team": { "id": TEAM } }, "user": { "name": "Fixture Person" } })],
        ),
        other => panic!("the importer sent a tracker operation it must not: {other:?}"),
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    notion: StubServer,
    tracker: StubServer,
}

impl Fixture {
    async fn new() -> Fixture {
        let dir = tempfile::Builder::new()
            .prefix("tracker-import-")
            .tempdir()
            .unwrap();
        let base = Arc::new(Mutex::new(String::new()));
        let shared = base.clone();
        let notion = stub::serve(move |req| notion_reply(req, &shared.lock().unwrap())).await;
        *base.lock().unwrap() = notion.url.trim_end_matches("/graphql").to_string();
        let tracker = stub::serve(tracker_reply).await;
        let f = Fixture {
            dir,
            notion,
            tracker,
        };
        f.seed();
        f
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("vault")
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root().join(rel)).unwrap_or_default()
    }

    fn seed(&self) {
        let task = |id: &str, title: &str, status: &str| {
            format!("---\nnotion_id: \"{id}\"\ndescription: \"The Notion summary of {id}.\"\nnotion_project_ids: \"[\\\"{P1}\\\"]\"\nproject: \"Shift Log Form\"\nproject_url: \"vault:workspaces/notion-{P1}/PROJECT.md\"\nstatus: \"{status}\"\ntitle: \"{title}\"\n---\n\n## Activity\n\n- 2026-11-02 agent: a note that must survive.\n")
        };
        self.write(
            &format!("tasks/notion-{T_OPEN}.md"),
            &task(T_OPEN, "Confirm IT is finished", "Not started"),
        );
        self.write(
            &format!("tasks/notion-{T_SPLIT}.md"),
            &task(
                T_SPLIT,
                "Shift Log app: accept counter exports",
                "In progress",
            ),
        );
        self.write(
            &format!("tasks/notion-{T_DONE}.md"),
            &task(T_DONE, "Ship the old thing", "Done"),
        );
        self.write(
            &format!("tasks/notion-{T_DIFF}.md"),
            &task(T_DIFF, "Renamed since", "Planning"),
        );
        self.write(
            &format!("tasks/notion-{T_CITE}.md"),
            &task(T_CITE, "Cited by its issue", "Not started"),
        );
        self.write(
            &format!("tasks/notion-{T_NONE}.md"),
            &task(T_NONE, "Nobody filed this", "Not started"),
        );
        let project = |id: &str, title: &str| {
            format!("---\nnotion_id: \"{id}\"\ndescription: \"Old description.\"\nstatus: \"Active\"\ntitle: \"{title}\"\n---\n")
        };
        self.write(
            &format!("workspaces/notion-{P1}/PROJECT.md"),
            &project(P1, "Shift Log Form"),
        );
        self.write(
            &format!("workspaces/notion-{P1}/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: \"Shift Log Form\"\n---\n\n# Shift Log Form\n",
        );
        self.write(
            &format!("workspaces/notion-{P1}/NOTION.md"),
            "# Notion mirror\n",
        );
        self.write(
            &format!("workspaces/notion-{P2}/PROJECT.md"),
            &project(P2, "Old idea"),
        );
        self.write(".notion-sync.json", "{\"records\":{}}");
        self.write(".notion-sync 2.json", "{\"records\":{}}");
        self.write(".notion-sync-backups/one.json", "{}");
        let row = |key: &str, title: &str, file: &str| {
            format!(
                "| {key} | {title} | Todo |  | src | `vault:tasks/notion-{file}.md` | OPS-1 |\n"
            )
        };
        let mapping = format!(
            "# Mapping\n\n| Linear | Title | Status | Alex | Source | Hollow task | Was (Legacy) |\n|---|---|---|---|---|---|---|\n{}{}{}{}{}| ENG-60 | Unmapped issue | Todo |  | src | none | OPS-9 |\n",
            row("ENG-1", "Confirm IT's part is finished", T_OPEN),
            row("ENG-24", "Run the first real upload", T_SPLIT),
            row("ENG-27", "Counter export: duplicate paid", T_SPLIT),
            row("ENG-31", "Counter export: zero rows", T_SPLIT),
            row("ENG-40", "Old title", T_DIFF),
        );
        fs::write(self.dir.path().join("mapping.md"), mapping).unwrap();
    }

    fn settings(&self) -> Settings {
        let sources: serde_json::Map<String, Value> = records::Kind::ALL
            .iter()
            .map(|k| (k.config_key().to_string(), json!(source_id(*k))))
            .collect();
        Settings::from_value(&json!({
            "notion": { "token": TOKEN, "twoWaySync": false },
            "tracker": { "provider": "linear", "linear": { "teamId": TEAM, "teamKey": "ENG" } },
            "trackerImport": { "notionSources": sources }
        }))
    }

    fn args(&self, mode: Mode) -> Args {
        Args {
            config: self.dir.path().join("config.json"),
            vault: self.root(),
            staging: Some(self.dir.path().join("staging")),
            mapping: vec![self.dir.path().join("mapping.md")],
            backup_dir: Some(self.dir.path().join("backup")),
            member: "Sam Taylor".into(),
            bind_projects: Vec::new(),
            mode,
        }
    }

    fn notion_client(&self) -> NotionClient {
        NotionClient::with_base(
            &format!("{}/v1", self.notion.url.trim_end_matches("/graphql")),
            TOKEN,
        )
        .unwrap()
    }

    fn adapter(&self) -> LinearAdapter {
        LinearAdapter::with_url(&self.tracker.url, TEAM, KEY).unwrap()
    }

    async fn run(&self, mode: Mode) -> Result<String, String> {
        let notion = self.notion_client();
        let tracker = self.adapter();
        let remotes = Remotes {
            notion: Some(&notion),
            tracker: Some(&tracker),
            file_cap: 1024,
            secrets: vec![TOKEN.into(), KEY.into()],
        };
        run_with(&self.args(mode), &self.settings(), &remotes).await
    }
}

/// Every file under `root` (dot files included) and its SHA-256.
fn hashes(root: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                out.insert(rel, sha256(&fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn manifest_of(summary: &str) -> PathBuf {
    PathBuf::from(summary.rsplit_once("Manifest: ").expect(summary).1.trim())
}

#[tokio::test]
async fn importer_round_trip_is_idempotent_and_rolls_back_byte_for_byte() {
    let f = Fixture::new().await;
    let root = f.root();
    let original = hashes(&root);

    // Dry run: planned files and the report in staging, the vault untouched.
    f.run(Mode::DryRun).await.unwrap();
    assert_eq!(
        hashes(&root),
        original,
        "a dry run touches nothing in the vault"
    );
    let staging = f.dir.path().join("staging");
    let report: Value =
        serde_json::from_slice(&fs::read(staging.join("report.json")).unwrap()).unwrap();
    for (key, want) in [
        ("projectsNotion", 2),
        ("projectsTracked", 1),
        ("projectsVaultOnly", 1),
        ("openTasks", 5),
        ("openMapped", 2),
        ("doneTasks", 1),
        ("meetings", 1),
        ("people", 1),
        ("peopleInBoth", 1),
        ("domains", 1),
        ("workingFiles", 1),
        ("trackerIssues", 7),
        ("trackerIssueFilesNew", 4),
    ] {
        assert_eq!(report[key], want, "{key}: {report:#}");
    }
    assert_eq!(report["openMatched"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["openNeedIssue"][0]["path"],
        format!("tasks/notion-{T_NONE}.md")
    );
    assert_eq!(report["mappingNotApplied"].as_array().unwrap().len(), 1);
    assert!(report["mappingNotApplied"][0]["detail"]
        .as_str()
        .unwrap()
        .contains("New title"));
    assert_eq!(report["workingFilesSkipped"][0]["title"], "huge.bin");
    assert_eq!(report["archiveMoves"].as_array().unwrap().len(), 4);
    assert_eq!(
        report["droppedOutOfScope"], 0,
        "the adapter drops the LP node itself"
    );
    assert_eq!(report["errors"], json!([]), "{report:#}");
    assert!(
        fs::read_to_string(staging.join(format!("tasks/notion-{T_SPLIT}.md")))
            .unwrap()
            .contains("tracker_key: \"ENG-24\"")
    );
    let md = fs::read_to_string(staging.join("report.md")).unwrap();
    assert!(md.contains("Open tasks that need an issue: 1"));

    // Apply: records, frozen done tasks, Working Files, archive moves.
    let applied = f.run(Mode::Apply).await.unwrap();
    let m1 = manifest_of(&applied);
    let done = f.read(&format!("tasks/notion-{T_DONE}.md"));
    assert!(
        done.contains("tracker: \"none\"") && done.contains("archived_from: \"notion\""),
        "{done}"
    );
    assert!(
        done.contains("completed: \"2026-10-20\"") && done.contains("notion_task_id: \"TASK-12\"")
    );
    assert!(!f
        .read(&format!("tasks/notion-{T_OPEN}.md"))
        .contains("notion_project_ids"));
    assert!(f
        .read(&format!("tasks/notion-{T_OPEN}.md"))
        .contains("a note that must survive"));
    let meeting = f.read("library/records/meetings/2026/2026-11-03-weekly-sync.md");
    assert!(
        meeting.contains("  - \"library/records/people/sam-taylor.md\""),
        "{meeting}"
    );
    assert!(
        meeting.contains(&format!("  - \"tasks/notion-{T_OPEN}.md\""))
            && meeting.contains("- Ship it")
    );
    let person = f.read("library/records/people/sam-taylor.md");
    assert!(
        person.contains("membership: \"workspace member\"")
            && person.contains("email: \"sam@example.com\""),
        "{person}"
    );
    let project = f.read(&format!("workspaces/notion-{P1}/PROJECT.md"));
    assert!(
        project.contains("hours_saved_per_year: \"120\"")
            && project.contains("## Notion description (final, 2026-11-08)")
    );
    assert!(
        project.contains("requested_by:\n  - \"library/records/people/sam-taylor.md\""),
        "{project}"
    );
    assert_eq!(
        f.read(&format!(
            "workspaces/notion-{P1}/sources/notion-files/notes.txt"
        )),
        "hello"
    );
    assert!(!root
        .join(format!(
            "workspaces/notion-{P1}/sources/notion-files/huge.bin"
        ))
        .exists());
    assert!(
        !root.join(".notion-sync.json").exists()
            && !root
                .join(format!("workspaces/notion-{P1}/NOTION.md"))
                .exists()
    );
    let applied_once = hashes(&root);
    assert_eq!(
        f.run(Mode::Apply).await.unwrap(),
        "apply: nothing to change."
    );
    assert_eq!(
        hashes(&root),
        applied_once,
        "a second apply changes nothing"
    );

    // Rekey: identity, the split, the ledger.
    let rekeyed = f.run(Mode::Rekey).await.unwrap();
    let m2 = manifest_of(&rekeyed);
    let split = f.read(&format!("tasks/notion-{T_SPLIT}.md"));
    assert!(
        split.contains("tracker_key: \"ENG-24\"")
            && split.contains("- ENG-27: ")
            && split.contains("- ENG-31: "),
        "{split}"
    );
    for id in ["i27", "i31"] {
        let sibling = f.read(&format!("tasks/linear-{id}.md"));
        assert!(
            sibling.contains(&format!("tracker_id: \"{id}\""))
                && sibling.contains("Split from `vault:tasks/notion-"),
            "{sibling}"
        );
    }
    let open = f.read(&format!("tasks/notion-{T_OPEN}.md"));
    assert!(
        open.contains("tracker_key: \"ENG-1\"")
            && open.contains("## Notion summary")
            && open.contains("Looks right."),
        "{open}"
    );
    assert!(
        !open.contains("\ndescription: "),
        "the Notion summary moved to the body"
    );
    assert!(f
        .read(&format!("tasks/notion-{T_CITE}.md"))
        .contains("tracker_key: \"ENG-50\""));
    assert!(
        !f.read(&format!("tasks/notion-{T_DIFF}.md"))
            .contains("tracker_id"),
        "a row whose title differs isn't applied"
    );
    assert!(!f
        .read(&format!("tasks/notion-{T_NONE}.md"))
        .contains("tracker_id"));
    assert!(root.join("tasks/linear-i40.md").exists() && root.join("tasks/linear-i60.md").exists());
    assert!(!root.join("tasks/linear-lp1.md").exists());
    let project = f.read(&format!("workspaces/notion-{P1}/PROJECT.md"));
    assert!(
        project.contains("tracker_project: \"p1\"") && project.contains("hours_saved_per_year"),
        "{project}"
    );
    let ledger = crate::tracker_sync::load(&root, "linear", TEAM).unwrap();
    assert_eq!(ledger.records.len(), 8, "7 issues and 1 project");
    assert!(!ledger.records.contains_key("lp1"));
    let rekeyed_once = hashes(&root);
    assert_eq!(
        f.run(Mode::Rekey).await.unwrap(),
        "rekey: nothing to change."
    );
    assert_eq!(
        hashes(&root),
        rekeyed_once,
        "a second rekey changes nothing"
    );

    // The first sync pass after the migration pulls and pushes nothing.
    let copy = f.dir.path().join("sync-copy");
    copy_tree(&root, &copy);
    let config: crate::config::Config = serde_json::from_value(json!({
        "vaultPath": copy, "notion": { "twoWaySync": false },
        "tracker": { "provider": "linear", "linear": { "teamId": TEAM, "teamKey": "ENG" } }
    }))
    .unwrap();
    let before_sync: BTreeMap<_, _> = hashes(&copy)
        .into_iter()
        .filter(|(k, _)| k.starts_with("tasks/") || k.ends_with("PROJECT.md"))
        .collect();
    let outcome = crate::tracker_sync::sync_with(&config, &f.adapter())
        .await
        .unwrap();
    assert_eq!(
        (outcome.pulled, outcome.pushed, outcome.conflicts),
        (0, 0, 0),
        "{outcome:?}"
    );
    let after_sync: BTreeMap<_, _> = hashes(&copy)
        .into_iter()
        .filter(|(k, _)| k.starts_with("tasks/") || k.ends_with("PROJECT.md"))
        .collect();
    assert_eq!(after_sync, before_sync);
    // Once sync has run, a rekey would replace its history: refused.
    let mut on_copy = f.args(Mode::Rekey);
    on_copy.vault = copy.clone();
    let notion = f.notion_client();
    let adapter = f.adapter();
    let remotes = Remotes {
        notion: Some(&notion),
        tracker: Some(&adapter),
        file_cap: 1024,
        secrets: vec![],
    };
    let refused = run_with(&on_copy, &f.settings(), &remotes)
        .await
        .unwrap_err();
    assert!(refused.contains("already run"), "{refused}");

    // Rollback, newest first, puts every byte back.
    files::rollback(&root, &m2).unwrap();
    assert_eq!(hashes(&root), applied_once);
    files::rollback(&root, &m1).unwrap();
    assert_eq!(hashes(&root), original, "rollback is byte-identical");
    assert!(
        !root.join("library").exists() && !root.join(".archive").exists(),
        "created folders are gone"
    );
    assert!(
        files::rollback(&root, &m1).is_err(),
        "a manifest rolls back once"
    );

    // Neither service ever saw a write.
    for req in f.notion.requests() {
        let path = req.path.split('?').next().unwrap();
        assert!(
            req.method == "GET"
                || (req.method == "POST"
                    && path.starts_with("/v1/data_sources/")
                    && path.ends_with("/query")),
            "{} {}",
            req.method,
            req.path
        );
    }
    assert!(f
        .tracker
        .requests()
        .iter()
        .all(|r| !r.query().trim_start().starts_with("mutation")));
}

/// A mixed-build rollback: while config.json still has `notion.twoWaySync`
/// on, --apply and --rekey refuse before writing anything; a dry run still
/// stages.
#[tokio::test]
async fn apply_and_rekey_refuse_while_two_way_sync_is_on() {
    let f = Fixture::new().await;
    let root = f.root();
    let original = hashes(&root);
    let mut settings = f.settings();
    settings.two_way_sync = true;
    let notion = f.notion_client();
    let tracker = f.adapter();
    let remotes = Remotes {
        notion: Some(&notion),
        tracker: Some(&tracker),
        file_cap: 1024,
        secrets: vec![TOKEN.into(), KEY.into()],
    };
    for mode in [Mode::Apply, Mode::Rekey] {
        assert_eq!(
            run_with(&f.args(mode), &settings, &remotes)
                .await
                .unwrap_err(),
            TWO_WAY_BLOCK
        );
    }
    assert_eq!(hashes(&root), original, "nothing was written");
    assert!(!f.dir.path().join("backup").exists());
    assert!(run_with(&f.args(Mode::DryRun), &settings, &remotes)
        .await
        .is_ok());
}

#[tokio::test]
async fn the_notion_client_can_only_read() {
    let id = "33333333333343338333333333333331".to_string();
    let every = [
        Request::PageProperty {
            page: id.clone(),
            property: "a%3Bb".into(),
            cursor: Some("c2".into()),
        },
        Request::BlockChildren {
            id: id.clone(),
            cursor: None,
        },
        Request::DataSource { id: id.clone() },
        Request::Query {
            id: id.clone(),
            cursor: None,
        },
    ];
    for request in &every {
        let path = request.path().unwrap();
        match request {
            Request::Query { .. } => {
                assert_eq!(request.method(), reqwest::Method::POST);
                assert_eq!(path, format!("/data_sources/{id}/query"));
            }
            _ => assert_eq!(request.method(), reqwest::Method::GET, "{request:?}"),
        }
    }
    assert!(Request::DataSource {
        id: "../users/me".into()
    }
    .path()
    .is_err());
    assert!(Request::BlockChildren {
        id: id.clone(),
        cursor: Some("a&b=c".into())
    }
    .path()
    .is_err());

    let server = stub::serve(|_| StubReply::data(json!({}))).await;
    let client = NotionClient::with_base(&server.url.replace("/graphql", "/v1"), TOKEN).unwrap();
    for request in &every {
        client.send(request).await.unwrap();
    }
    let seen: Vec<(String, String)> = server
        .requests()
        .into_iter()
        .map(|r| (r.method, r.path))
        .collect();
    assert_eq!(seen.iter().filter(|(m, _)| m == "POST").count(), 1);
    assert!(
        seen.iter()
            .all(|(m, p)| m == "GET" || p.ends_with("/query")),
        "{seen:?}"
    );

    // By construction: the client file names no other method and no
    // generic request builder.
    let source = include_str!("notion.rs");
    for forbidden in [
        ".put(",
        ".patch(",
        ".delete(",
        ".request(",
        "Method::PUT",
        "Method::PATCH",
        "Method::DELETE",
        "Method::from",
    ] {
        assert!(!source.contains(forbidden), "{forbidden}");
    }
    assert_eq!(source.matches("self.http.post(").count(), 1);
}

#[tokio::test]
async fn errors_never_carry_the_token_or_the_key() {
    let f = Fixture::new().await;
    // Notion echoing the header it got: the client's error hides the token.
    let echo = stub::serve(|req| StubReply { status: 401, headers: Vec::new(),
        body: json!({ "code": "unauthorized", "message": format!("{} is invalid", req.headers["authorization"]) }).to_string() }).await;
    let client = NotionClient::with_base(&echo.url.replace("/graphql", "/v1"), TOKEN).unwrap();
    let error = client
        .send(&Request::DataSource { id: P1.into() })
        .await
        .unwrap_err();
    assert!(
        error.contains("401") && error.contains("[redacted]") && !error.contains(TOKEN),
        "{error}"
    );
    assert!(!format!("{client:?}").contains(TOKEN));

    // A tracker that echoes the key in its error: the run's error hides it.
    let tracker = stub::serve(|_| StubReply::error("INTERNAL", &format!("bad key {KEY}"))).await;
    let adapter = LinearAdapter::with_url(&tracker.url, TEAM, KEY).unwrap();
    let notion = f.notion_client();
    let remotes = Remotes {
        notion: Some(&notion),
        tracker: Some(&adapter),
        file_cap: 1024,
        secrets: vec![TOKEN.into(), KEY.into()],
    };
    let error = run_with(&f.args(Mode::Rekey), &f.settings(), &remotes)
        .await
        .unwrap_err();
    assert!(!error.contains(KEY) && !error.contains(TOKEN), "{error}");
    assert_eq!(
        scrub(&format!("a {KEY} b secret_abcdefgh123"), &[]),
        "a [redacted] b [redacted]"
    );
}

#[test]
fn arguments_need_exactly_one_mode_and_its_folder() {
    let args = |s: &str| parse_args(&s.split(' ').map(String::from).collect::<Vec<_>>());
    assert!(args("--config c --vault v --dry-run --apply --staging s").is_err());
    assert!(args("--config c --vault v --dry-run").is_err());
    assert!(args("--config c --vault v --apply").is_err());
    let ok = args("--config c --vault v --rekey --backup-dir b --mapping m1 --mapping m2").unwrap();
    assert_eq!((ok.mode, ok.mapping.len()), (Mode::Rekey, 2));
    let settings = Settings::from_value(&json!({ "notionToken": " legacy " }));
    assert_eq!(settings.notion_token.as_deref(), Some("legacy"));
    assert!(settings.tracker_assumed && settings.scope() == DEFAULT_TEAM_ID);
}

fn write_manifest(dir: &Path, vault: &Path, entries: Value) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join("manifest.json");
    let manifest = json!({ "version": 1, "mode": "apply", "createdAt": "", "entries": entries,
        "vault": vault.canonicalize().unwrap().to_string_lossy() });
    fs::write(&path, manifest.to_string()).unwrap();
    path
}

#[test]
fn a_manifest_path_outside_the_vault_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    let outside = dir.path().join("outside.md");
    fs::write(&outside, "keep").unwrap();
    let sha = sha256(b"keep");
    for path in [
        outside.to_string_lossy().to_string(),
        "../outside.md".into(),
    ] {
        let m = write_manifest(
            &dir.path().join("b"),
            &vault,
            json!([{ "action": "created", "path": path, "afterSha256": sha }]),
        );
        assert!(files::rollback(&vault, &m).is_err(), "{path}");
        assert!(outside.exists(), "{path} was not removed");
    }
    let m = write_manifest(
        &dir.path().join("c"),
        &vault,
        json!([{ "action": "changed", "path": "a.md",
        "beforeSha256": sha, "afterSha256": sha, "backup": "../../outside.md" }]),
    );
    assert!(
        files::rollback(&vault, &m).is_err(),
        "a backup outside files/ is refused"
    );
}

#[test]
fn a_failed_rollback_can_run_again() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("a.md"), "edited since").unwrap();
    let m = write_manifest(
        &dir.path().join("b"),
        &vault,
        json!([{ "action": "created", "path": "a.md", "afterSha256": sha256(b"as written") }]),
    );
    assert!(files::rollback(&vault, &m)
        .unwrap_err()
        .contains("changed after the run"));
    assert!(
        !dir.path().join("b/rolled-back.json").exists(),
        "no marker after a failed rollback"
    );
    fs::write(vault.join("a.md"), "as written").unwrap();
    files::rollback(&vault, &m).unwrap();
    assert!(!vault.join("a.md").exists() && dir.path().join("b/rolled-back.json").exists());
}

#[tokio::test]
async fn only_team_ai_and_a_scrubbed_preflight() {
    let f = Fixture::new().await;
    let other = Settings::from_value(&json!({ "notion": { "token": TOKEN },
        "tracker": { "provider": "linear", "linear": { "teamId": "8d8e6207-0000-4000-8000-000000000000", "teamKey": "LP" } } }));
    let notion = f.notion_client();
    let tracker = f.adapter();
    let remotes = Remotes {
        notion: Some(&notion),
        tracker: Some(&tracker),
        file_cap: 1024,
        secrets: vec![TOKEN.into(), KEY.into()],
    };
    let refused = run_with(&f.args(Mode::DryRun), &other, &remotes)
        .await
        .unwrap_err();
    assert!(refused.contains("only for team ENG"), "{refused}");
    assert!(!f.dir.path().join("staging/report.json").exists());

    let mut args = f.args(Mode::Preflight);
    args.member = format!("Someone {TOKEN}");
    run_with(&args, &f.settings(), &remotes).await.unwrap();
    let notes = fs::read_to_string(f.dir.path().join("staging/preflight.txt")).unwrap();
    assert!(
        notes.contains("Tracker team: Fixture AI") && notes.contains("[redacted]: not a member"),
        "{notes}"
    );
    assert!(!notes.contains(TOKEN), "{notes}");
}

#[test]
fn planted_links_are_never_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.md");
    fs::write(&outside, "keep").unwrap();
    let vault = dir.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    // The old fixed temp name and the destination itself, both links out.
    std::os::unix::fs::symlink(&outside, vault.join(".a.md.tracker-import.tmp")).unwrap();
    std::os::unix::fs::symlink(&outside, vault.join("a.md")).unwrap();
    files::write_atomic(&vault.join("a.md"), b"new").unwrap();
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    assert!(!fs::symlink_metadata(vault.join("a.md"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(vault.join("a.md")).unwrap(), "new");
    // A reused staging folder with a link where a report goes.
    let staging = dir.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    std::os::unix::fs::symlink(&outside, staging.join("report.json")).unwrap();
    assert!(files::write_out(&staging, "report.json", b"x").is_err());
    std::os::unix::fs::symlink(dir.path(), staging.join("tasks")).unwrap();
    assert!(files::write_out(&staging, "tasks/outside.md", b"x").is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
}

// ---- T6b: binding tracker projects to workspaces, and members ----------------

fn project_node(id: &str, key: &str, name: &str, description: &str) -> Value {
    json!({ "id": id, "identifier": key, "url": format!("https://linear.app/example/project/{id}"),
        "name": name, "description": description, "status": { "name": "In Progress", "type": "started" },
        "startDate": null, "targetDate": null, "updatedAt": T0, "progress": 0.0, "lead": null,
        "teams": { "nodes": [{ "id": TEAM }] }, "externalLinks": { "nodes": [] } })
}

fn binding_issues() -> Vec<Value> {
    let mut nodes = tracker_issues();
    for (id, key, project, description) in [
        ("i70", "ENG-70", "pA", ""),
        ("i71", "ENG-71", "pA", ""),
        ("i72", "ENG-72", "pA", ""),
        ("i80", "ENG-80", "pB", ""),
        ("i81", "ENG-81", "pB", ""),
        (
            "i90",
            "ENG-90",
            "pD",
            "Plan: `workspaces/local-jev/PLAN.md`.",
        ),
        (
            "i95",
            "ENG-95",
            "pE",
            "See `workspaces/local-lone/PLAN.md`.",
        ),
        ("i96", "ENG-96", "pE", ""),
        ("i97", "ENG-97", "pE", ""),
        (
            "i100",
            "ENG-100",
            "pF",
            "Notes: `workspaces/local-contest/PLAN.md`.",
        ),
    ] {
        let mut node = issue(id, key, &format!("Issue {key}"), TEAM, description);
        node["project"] = json!({ "id": project });
        nodes.push(node);
    }
    for n in 200..212 {
        let key = format!("AI-{n}");
        let mut node = issue(&format!("i{n}"), &key, &format!("Issue {key}"), TEAM, "");
        node["project"] = json!({ "id": "pG" });
        nodes.push(node);
    }
    nodes
}

/// The fixture tracker with four more projects: Atlas (its mapping rows
/// cite a local workspace 2 to 1), Sage (a 1 to 1 tie), Chris (bound by
/// `--bind-project` although its description cites a workspace) and jev
/// (no rows; one issue's description cites its workspace). Sam's tracker
/// name is his email address.
fn binding_reply(req: &StubRequest) -> StubReply {
    let connection = |nodes: Vec<Value>| json!({ "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } });
    let filter = &req.variables()["filter"];
    let ids = filter.pointer("/id/in").and_then(Value::as_array).cloned();
    match req.operation().as_deref() {
        Some("Members") => {
            StubReply::data(json!({ "team": { "id": TEAM, "members": connection(vec![
            json!({ "id": "u2", "name": "sam.taylor@example.com", "displayName": "sam.taylor",
                    "email": "sam.taylor@example.com", "active": true })]) } }))
        }
        Some("Projects" | "ProjectTarget") => StubReply::data(json!({ "projects": connection(vec![
            project_node("p1", "P-ENG-10", "Shift Log Form", &format!("Vault: `workspaces/notion-{P1}`")),
            project_node("pA", "P-ENG-22", "Atlas 2.0", ""),
            project_node("pB", "P-ENG-8", "Sage Ledger AI Gateway", ""),
            project_node("pC", "P-ENG-14", "Chris Requests", "Old notes: `workspaces/local-sage-a/PLAN.md`"),
            project_node("pD", "P-ENG-15", "jev", ""),
            project_node("pE", "P-ENG-16", "Lone vote", ""),
            project_node("pF", "P-ENG-1", "Contest one", ""),
            project_node("pG", "P-ENG-2", "Contest two", ""),
        ]) })),
        Some("Issues") => StubReply::data(json!({ "issues": connection(binding_issues()) })),
        Some("IssueDetail" | "IssueTarget" | "Attachments") => {
            StubReply::data(json!({ "issues": connection(
            binding_issues()
                .into_iter()
                .filter(|n| n["team"]["id"] == TEAM)
                .filter(|n| ids.as_ref().is_none_or(|ids| ids.contains(&n["id"])))
                .filter(|n| filter.pointer("/id/eq").is_none_or(|id| &n["id"] == id))
                .collect()) }))
        }
        _ => tracker_reply(req),
    }
}

/// Every workspace PROJECT.md that names `id` as its tracker project.
fn project_homes(root: &Path, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root.join("workspaces")).unwrap().flatten() {
        let text = fs::read_to_string(entry.path().join("PROJECT.md")).unwrap_or_default();
        if crate::frontmatter::parse(&text).get("tracker_project") == Some(id) {
            out.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    out
}

#[tokio::test]
async fn tracker_projects_bind_by_override_mapping_sources_and_issue_cites() {
    let f = Fixture::new().await;
    let root = f.root();
    for dir in [
        "local-atlas",
        "local-sage-a",
        "local-sage-b",
        "local-matt",
        "local-jev",
        "local-lone",
        "local-contest",
    ] {
        f.write(
            &format!("workspaces/{dir}/WORKSPACE.md"),
            &format!("---\nhollow_workspace: 1\ntitle: \"{dir}\"\n---\n\n# {dir}\n"),
        );
    }
    let rows = |key: &str, source: &str| {
        format!(
            "| {key} | Issue {key} | Todo |  | {source} | `vault:tasks/t6b-{key}.md` | OPS-1 |\n"
        )
    };
    let contest: String = (200..212)
        .map(|n| rows(&format!("AI-{n}"), "`workspaces/local-contest/PLAN.md`"))
        .collect();
    // Other tables in the file add no votes: counted, these would tie
    // Atlas between local-atlas and local-sage-b.
    let other_tables = "\n| Key | Title | Status | Who | Source |\n|---|---|---|---|---|\n| ENG-70 | x | Todo |  | `workspaces/local-sage-b/PLAN.md` |\n| ENG-71 | x | Todo |  | `workspaces/local-sage-b/PLAN.md` |\n\n| Key | Title | Status | Who | Source | Task | Was |\n|---|---|---|---|---|---|---|\n| ENG-72 | x | Todo |  | `workspaces/local-sage-b/PLAN.md` | none | OPS-1 |\n";
    fs::write(
        f.dir.path().join("binding.md"),
        format!(
            "| Linear | Title | Status | Alex | Source | Hollow task | Was |\n|---|---|---|---|---|---|---|\n{}{}{}{}{}{contest}{other_tables}",
            rows("ENG-70", "`workspaces/local-atlas/PLAN.md` (section 2)"),
            rows("ENG-71", "`workspaces/local-atlas/WORKSPACE.md`"),
            rows("ENG-72", &format!("`workspaces/notion-{P2}/PLAN.md`")),
            rows("ENG-80", "`workspaces/local-sage-a/PLAN.md`"),
            rows("ENG-81", "`workspaces/local-sage-b/PLAN.md`"),
        ),
    )
    .unwrap();
    let tracker = stub::serve(binding_reply).await;
    let adapter = LinearAdapter::with_url(&tracker.url, TEAM, KEY).unwrap();
    let notion = f.notion_client();
    let remotes = Remotes {
        notion: Some(&notion),
        tracker: Some(&adapter),
        file_cap: 1024,
        secrets: vec![TOKEN.into(), KEY.into()],
    };
    let mut args = f.args(Mode::Rekey);
    args.mapping.push(f.dir.path().join("binding.md"));
    args.bind_projects = vec![
        ("P-ENG-14".into(), "local-matt".into()),
        ("P-ENG-99".into(), "local-matt".into()),
    ];
    let summary = run_with(&args, &f.settings(), &remotes).await.unwrap();
    let report: Value = serde_json::from_slice(
        &fs::read(manifest_of(&summary).parent().unwrap().join("report.json")).unwrap(),
    )
    .unwrap();
    let detail = |title: &str| {
        report["projectsBound"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["title"] == title)
            .map(|i| {
                (
                    i["path"].as_str().unwrap().to_string(),
                    i["detail"].as_str().unwrap().to_string(),
                )
            })
    };
    let (path, why) = detail("Atlas 2.0").expect("Atlas is bound");
    assert_eq!(path, "workspaces/local-atlas/PROJECT.md");
    assert!(
        why.contains("mapping rows")
            && why.contains("2 of 3 issues")
            && why.contains("local-atlas 2")
            && !why.contains("local-sage-b"),
        "{why}"
    );
    // A workspace two projects claim goes to the far stronger one.
    let (path, why) = detail("Contest two").expect("P-ENG-2's 12 rows win the workspace");
    assert_eq!(path, "workspaces/local-contest/PROJECT.md");
    assert!(
        why.contains("12 of 12 issues") && why.contains("P-ENG-1 1"),
        "{why}"
    );
    assert!(detail("Contest one").is_none() && detail("Lone vote").is_none());
    let (path, why) = detail("Chris Requests").expect("the override binds Chris");
    assert_eq!(path, "workspaces/local-matt/PROJECT.md");
    assert!(
        why.contains("--bind-project"),
        "the override beats its description: {why}"
    );
    let (path, why) = detail("jev").expect("jev is bound by its issue's description");
    assert_eq!(path, "workspaces/local-jev/PROJECT.md");
    assert!(
        why.contains("issue descriptions") && why.contains("1 of 1"),
        "{why}"
    );
    assert!(detail("Shift Log Form").is_some());
    assert!(
        detail("Sage Ledger AI Gateway").is_none(),
        "a 1 to 1 tie stays unbound"
    );
    let unbound: BTreeMap<String, String> = report["trackerProjectsWithoutWorkspace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["title"].as_str().unwrap().to_string(),
                i["detail"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(unbound.len(), 3, "{unbound:#?}");
    let sage = &unbound["Sage Ledger AI Gateway"];
    assert!(
        sage.contains("no clear winner") && sage.contains("--bind-project P-ENG-8="),
        "{sage}"
    );
    let lone = &unbound["Lone vote"];
    assert!(
        lone.contains("1 of 3 issues") && lone.contains("local-lone"),
        "a lone vote doesn't bind a 3-issue project: {lone}"
    );
    let loser = &unbound["Contest one"];
    assert!(
        loser.contains("local-contest") && loser.contains("P-ENG-2"),
        "{loser}"
    );
    assert!(
        report["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.as_str().unwrap().contains("P-ENG-99")),
        "{report:#}"
    );

    // The local workspace had no PROJECT.md: it gets one, its brief untouched.
    let made = f.read("workspaces/local-atlas/PROJECT.md");
    let fm = crate::frontmatter::parse(&made);
    assert_eq!(fm.get("tracker_project"), Some("pA"), "{made}");
    assert_eq!(fm.get("tracker"), Some("linear"));
    assert_eq!(fm.get("title"), Some("Atlas 2.0"));
    assert!(
        made.contains("Keep notes and decisions in WORKSPACE.md"),
        "{made}"
    );
    assert!(!f
        .read("workspaces/local-atlas/WORKSPACE.md")
        .contains("tracker"));

    // A second run changes nothing.
    let once = hashes(&root);
    assert_eq!(
        run_with(&args, &f.settings(), &remotes).await.unwrap(),
        "rekey: nothing to change."
    );
    assert_eq!(hashes(&root), once);

    // The first sync pass keeps every bound project in its workspace: no
    // second workspace names it, and its files stay as the rekey wrote them.
    let copy = f.dir.path().join("sync-copy");
    copy_tree(&root, &copy);
    let config: crate::config::Config = serde_json::from_value(json!({
        "vaultPath": copy, "notion": { "twoWaySync": false },
        "tracker": { "provider": "linear", "linear": { "teamId": TEAM, "teamKey": "ENG" } }
    }))
    .unwrap();
    crate::tracker_sync::sync_with(&config, &adapter)
        .await
        .unwrap();
    for (id, dir) in [
        ("p1", format!("notion-{P1}")),
        ("pA", "local-atlas".into()),
        ("pC", "local-matt".into()),
        ("pD", "local-jev".into()),
    ] {
        assert_eq!(project_homes(&copy, id), vec![dir.clone()], "{id}");
        for file in ["PROJECT.md", "WORKSPACE.md"] {
            assert_eq!(
                fs::read(copy.join(format!("workspaces/{dir}/{file}"))).unwrap(),
                fs::read(root.join(format!("workspaces/{dir}/{file}"))).unwrap(),
                "{dir}/{file}"
            );
        }
    }

    // The preflight finds Sam, whose tracker name is his email address.
    let mut pre = f.args(Mode::Preflight);
    pre.member = "Sam Taylor".into();
    let text = run_with(&pre, &f.settings(), &remotes).await.unwrap();
    assert!(text.contains("Sam Taylor: member"), "{text}");
    pre.member = "SAM.TAYLOR@example.com".into();
    let text = run_with(&pre, &f.settings(), &remotes).await.unwrap();
    assert!(text.contains(": member"), "{text}");
}

#[test]
fn bind_project_needs_a_key_and_a_folder() {
    let args = |s: &str| parse_args(&s.split(' ').map(String::from).collect::<Vec<_>>());
    let ok = args("--config c --vault v --rekey --backup-dir b --bind-project P-ENG-22=workspaces/local-x/ --bind-project p9=local-y").unwrap();
    assert_eq!(
        ok.bind_projects,
        vec![
            ("P-ENG-22".to_string(), "local-x".to_string()),
            ("p9".to_string(), "local-y".to_string())
        ]
    );
    assert!(args("--config c --vault v --rekey --backup-dir b --bind-project P-ENG-22").is_err());
    assert!(args("--config c --vault v --rekey --backup-dir b --bind-project =local-x").is_err());
}

#[test]
fn members_answer_to_their_name_display_name_and_email() {
    let sam = crate::tracker::TrackerUser {
        id: "u2".into(),
        name: "sam.taylor@example.com".into(),
        email: Some("sam.taylor@example.com".into()),
        display_name: Some("staylor".into()),
    };
    for wanted in [
        "Sam Taylor",
        "  sam   taylor ",
        "SAM.TAYLOR@EXAMPLE.COM",
        "STaylor",
    ] {
        assert!(sam.answers_to(wanted), "{wanted}");
    }
    for wanted in ["", "Sam", "Taylor", "sam.taylor@other.com"] {
        assert!(!sam.answers_to(wanted), "{wanted}");
    }
    // Assigning a task by any of those resolves to the one member.
    let meta: crate::tracker::ipc::TrackerTeamMeta = serde_json::from_value(json!({
        "team": { "id": TEAM, "key": "ENG", "name": "ENG" },
        "states": [], "labels": [], "members": [sam], "cycles": [], "milestones": [], "projects": []
    }))
    .unwrap();
    for wanted in ["Sam Taylor", "sam.taylor@example.com", "u2"] {
        let values = BTreeMap::from([("assignee".to_string(), json!(wanted))]);
        let patch = crate::tracker::sync_fields::patch(
            &values,
            &meta,
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert_eq!(patch.assignee_id, Some(Some("u2".into())), "{wanted}");
    }
}

/// Only mapping rows (a 6th column `vault:tasks/<file>.md`) vote; other
/// tables with an issue key in a mapping file don't.
#[test]
fn only_mapping_rows_cite_sources() {
    let text = "| ENG-1 | t | s | j | `workspaces/a/PLAN.md` | `vault:tasks/x.md` | OPS-1 |\n\
                | ENG-2 | t | s | j | `workspaces/b/PLAN.md` |\n\
                | ENG-3 | t | s | j | `workspaces/c/PLAN.md` | none | OPS-2 |\n";
    assert_eq!(
        rekey::parse_sources(text),
        BTreeMap::from([("ENG-1".to_string(), vec!["a".to_string()])])
    );
}

/// A PROJECT.md naming a project the team doesn't have is an error line,
/// not a panic, and the file is left alone.
#[tokio::test]
async fn a_project_the_team_lacks_is_reported_not_a_panic() {
    let f = Fixture::new().await;
    let stale = "---\ntracker: \"linear\"\ntracker_project: \"gone\"\ntitle: \"Gone\"\n---\n";
    f.write("workspaces/local-stale/PROJECT.md", stale);
    let summary = f.run(Mode::Rekey).await.unwrap();
    let report: Value = serde_json::from_slice(
        &fs::read(manifest_of(&summary).parent().unwrap().join("report.json")).unwrap(),
    )
    .unwrap();
    assert!(
        report["errors"].as_array().unwrap().iter().any(|e| e
            .as_str()
            .unwrap()
            .contains("workspaces/local-stale/PROJECT.md names tracker project gone")),
        "{report:#}"
    );
    assert_eq!(report["projectsTracked"], 1);
    assert_eq!(f.read("workspaces/local-stale/PROJECT.md"), stale);
}
