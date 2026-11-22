//! Tests for the Linear adapter, against `tracker::stub` (fixtures follow
//! Linear's public GraphQL schema), plus the read-only live smoke (ignored).

use super::*;
use crate::tracker::stub::{self, issue_node, StubReply, StubRequest, StubServer};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const TEAM: &str = crate::tracker::linear::TEAM_ID;
const OTHER: &str = "team-other-0001";
const KEY: &str = "lin_api_stubkey_never_logged_0123456789";
const T0: &str = "2026-11-08T05:00:00.000Z";
const T1: &str = "2026-11-08T05:30:00.000Z";

fn adapter(server: &StubServer) -> LinearAdapter {
    LinearAdapter::with_url(&server.url, TEAM, KEY).unwrap()
}

/// `{"data": {field: {nodes, pageInfo}}}`, the last page.
fn page(field: &str, nodes: Vec<Value>) -> StubReply {
    let mut data = Map::new();
    data.insert(
        field.to_string(),
        json!({ "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } }),
    );
    StubReply::data(Value::Object(data))
}

/// `nodes` narrowed the way Linear narrows an issues query's filter (team,
/// id, ids, number).
fn filtered(req: &StubRequest, nodes: &[Value]) -> Vec<Value> {
    let filter = &req.variables()["filter"];
    let team = filter.pointer("/team/id/eq").and_then(Value::as_str);
    let id = filter.pointer("/id/eq").and_then(Value::as_str);
    let ids: Option<Vec<&str>> = filter
        .pointer("/id/in")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).collect());
    let number = filter.pointer("/number/eq").and_then(Value::as_u64);
    nodes
        .iter()
        .filter(|node| team.is_none_or(|team| node["team"]["id"] == team))
        .filter(|node| id.is_none_or(|id| node["id"] == id))
        .filter(|node| {
            ids.as_ref()
                .is_none_or(|ids| ids.contains(&node["id"].as_str().unwrap_or("")))
        })
        .filter(|node| {
            number.is_none_or(|number| {
                node["identifier"]
                    .as_str()
                    .unwrap_or("")
                    .ends_with(&format!("-{number}"))
            })
        })
        .cloned()
        .collect()
}

fn operations(server: &StubServer) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|req| req.operation().unwrap_or_default())
        .collect()
}

fn sent(server: &StubServer, operation: &str) -> StubRequest {
    server
        .requests()
        .into_iter()
        .find(|req| req.operation().as_deref() == Some(operation))
        .unwrap_or_else(|| panic!("no {operation} request"))
}

// ---- The six LB T2 tests ------------------------------------------------------

#[tokio::test]
async fn another_teams_node_in_a_reply_is_dropped() {
    let mut ours = issue_node("i1", "ENG-1", TEAM, T0);
    ours["relations"]["nodes"] = json!([
        { "type": "blocks", "relatedIssue": { "identifier": "ENG-2", "team": { "id": TEAM } } },
        { "type": "related", "relatedIssue": { "identifier": "OT-9", "team": { "id": OTHER } } }
    ]);
    ours["children"]["nodes"] = json!([{ "identifier": "OT-8", "team": { "id": OTHER } }]);
    ours["parent"] = json!({ "id": "x9", "team": { "id": OTHER } });
    let theirs = issue_node("x1", "OT-1", OTHER, T1);
    // A server that ignores the team filter (Linear never should).
    let server = stub::serve(move |_req| page("issues", vec![ours.clone(), theirs.clone()])).await;

    let got = adapter(&server).issues_changed(None, None).await.unwrap();
    assert_eq!(got.items.len(), 1);
    let issue = &got.items[0];
    assert_eq!(
        (issue.key.as_str(), issue.team_id.as_str()),
        ("ENG-1", TEAM)
    );
    assert_eq!(
        issue.relations,
        vec![TrackerRelation {
            kind: RelationKind::Blocks,
            key: "ENG-2".into()
        }]
    );
    assert!(issue.children.is_empty());
    assert_eq!(issue.parent_id, None);
    assert!(got.next.is_none());
    assert_eq!(operations(&server), vec!["Issues"], "nothing read again");
}

#[tokio::test]
async fn an_unscoped_request_is_refused_by_the_stub_and_every_adapter_read_names_the_team() {
    let issues = vec![
        issue_node("i1", "ENG-1", TEAM, T0),
        issue_node("x1", "OT-1", OTHER, T1),
    ];
    let server = stub::serve(move |req| {
        if let Some(refused) = stub::unscoped(req, TEAM) {
            return refused;
        }
        match req.operation().as_deref() {
            Some("Issues" | "IssueDetail" | "IssueTarget" | "Attachments") => {
                page("issues", filtered(req, &issues))
            }
            Some("Comments") => page("comments", vec![]),
            _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unexpected operation"),
        }
    })
    .await;
    let adapter = adapter(&server);

    let got = adapter.issues_changed(Some(T0), None).await.unwrap();
    assert_eq!(got.items.len(), 1);
    assert!(adapter
        .comments_changed(None, None)
        .await
        .unwrap()
        .items
        .is_empty());
    assert_eq!(adapter.issue("ENG-1").await.unwrap().id, "i1");
    assert_eq!(
        adapter.issue("x1").await,
        Err(TrackerError::NotFound),
        "another team's issue is never read"
    );
    let links = adapter
        .attachments(&["i1".to_string(), "x1".to_string()])
        .await
        .unwrap();
    assert_eq!(links, vec![("i1".to_string(), Vec::new())]);
    let read = server.requests().len();
    assert!(server
        .requests()
        .iter()
        .all(|req| stub::unscoped(req, TEAM).is_none()));

    let refused = adapter
        .post(
            "query Leak { issues(first: 5) { nodes { id } } }",
            json!({}),
        )
        .await;
    assert_eq!(refused.unwrap_err(), TrackerError::OutOfScope);
    assert_eq!(server.requests().len(), read + 1);
}

#[tokio::test]
async fn every_read_names_the_team_in_its_filter_or_its_team_argument() {
    let empty = json!({ "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } });
    let server = stub::serve(move |req| match req.operation().as_deref() {
        Some("Team") => {
            StubReply::data(json!({ "team": { "id": TEAM, "key": "ENG", "name": "Stub team" } }))
        }
        Some("States" | "Members" | "Cycles") => StubReply::data(
            json!({ "team": { "id": TEAM, "states": empty, "members": empty, "cycles": empty } }),
        ),
        Some("Labels") => StubReply::data(json!({ "issueLabels": empty })),
        Some("Issues" | "IssueDetail" | "IssueTarget" | "Attachments" | "IssueHistory") => {
            StubReply::data(json!({ "issues": empty }))
        }
        Some("Comments") => StubReply::data(json!({ "comments": empty })),
        Some("Projects" | "Milestones" | "ProjectTarget") => {
            StubReply::data(json!({ "projects": empty }))
        }
        other => panic!("unexpected operation {other:?}"),
    })
    .await;
    let adapter = adapter(&server);

    assert_eq!(adapter.scope().await.unwrap().key, "ENG");
    adapter.states().await.unwrap();
    adapter.labels().await.unwrap();
    adapter.members().await.unwrap();
    adapter.cycles().await.unwrap();
    adapter.issues_changed(None, None).await.unwrap();
    adapter.comments_changed(None, None).await.unwrap();
    adapter.attachments(&["i1".to_string()]).await.unwrap();
    assert_eq!(adapter.issue("ENG-1").await, Err(TrackerError::NotFound));
    assert_eq!(
        adapter.issue_history("ENG-1").await,
        Err(TrackerError::NotFound)
    );
    adapter.projects_changed(None).await.unwrap();
    adapter.milestones(&["p1".to_string()]).await.unwrap();
    let patch = IssuePatch {
        title: Some("Changed".into()),
        ..IssuePatch::default()
    };
    assert_eq!(
        adapter.update_issue("ENG-1", patch).await,
        Err(TrackerError::OutOfScope)
    );
    let rename = ProjectPatch {
        name: Some("Renamed".into()),
        ..ProjectPatch::default()
    };
    assert_eq!(
        adapter.update_project("p1", rename).await,
        Err(TrackerError::OutOfScope)
    );

    let mut seen = BTreeSet::new();
    for req in server.requests() {
        let operation = req.operation().unwrap_or_default();
        let vars = req.variables();
        let team = match operation.as_str() {
            "Team" | "States" | "Members" | "Cycles" => vars.pointer("/team"),
            "Labels" => {
                assert_eq!(vars.pointer("/filter/or/1/team/null"), Some(&json!(true)));
                vars.pointer("/filter/or/0/team/id/eq")
            }
            "Issues" | "IssueDetail" | "IssueTarget" | "Attachments" | "IssueHistory" => {
                vars.pointer("/filter/team/id/eq")
            }
            "Comments" => vars.pointer("/filter/issue/team/id/eq"),
            "Projects" | "Milestones" | "ProjectTarget" => {
                vars.pointer("/filter/accessibleTeams/some/id/eq")
            }
            other => panic!("unexpected operation {other}"),
        };
        assert_eq!(team, Some(&json!(TEAM)), "{operation}: {vars}");
        assert!(
            !req.query().trim_start().starts_with("mutation"),
            "{operation}"
        );
        seen.insert(operation);
    }
    assert_eq!(seen.len(), 14, "every read was checked: {seen:?}");
}

#[tokio::test]
async fn a_429_or_ratelimited_reply_backs_off_and_a_good_reply_clears_it() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let server = stub::serve(move |_req| match counter.fetch_add(1, Ordering::SeqCst) {
        0 => StubReply::status(429),
        1 => StubReply::error("RATELIMITED", "Rate limit exceeded"),
        _ => {
            StubReply::data(json!({ "viewer": { "id": "u1", "name": "Stub User", "email": null } }))
        }
    })
    .await;
    let adapter = adapter(&server).with_backoff(Duration::from_millis(250), Duration::from_secs(2));

    // A 429: the first window.
    assert!(matches!(
        adapter.whoami().await,
        Err(TrackerError::RateLimited { reset_at: Some(_) })
    ));
    assert_eq!(lock(&adapter.backoff).streak, 1);
    // Inside it: refused without a request.
    assert!(matches!(
        adapter.whoami().await,
        Err(TrackerError::RateLimited { .. })
    ));
    assert_eq!(server.requests().len(), 1);

    tokio::time::sleep(Duration::from_millis(300)).await;
    // RATELIMITED in the body: the window doubles.
    assert!(matches!(
        adapter.whoami().await,
        Err(TrackerError::RateLimited { .. })
    ));
    assert_eq!(lock(&adapter.backoff).streak, 2);
    let until = lock(&adapter.backoff).until.unwrap();
    assert!(until - Utc::now() > chrono::Duration::milliseconds(300));

    tokio::time::sleep(Duration::from_millis(550)).await;
    assert_eq!(adapter.whoami().await.unwrap().id, "u1");
    assert_eq!(lock(&adapter.backoff).streak, 0);
    assert!(lock(&adapter.backoff).until.is_none());
    assert_eq!(server.requests().len(), 3);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_mutation_on_an_out_of_scope_issue_is_refused_before_anything_is_written() {
    let issues = vec![
        issue_node("i1", "ENG-1", TEAM, T0),
        issue_node("x1", "OT-1", OTHER, T0),
    ];
    let shared =
        json!({ "id": "p-shared", "teams": { "nodes": [{ "id": TEAM }, { "id": OTHER }] } });
    let server = stub::serve(move |req| {
        if let Some(refused) = stub::unscoped(req, TEAM) {
            return refused;
        }
        match req.operation().as_deref() {
            Some("IssueTarget") => page("issues", filtered(req, &issues)),
            Some("ProjectTarget") => page("projects", vec![shared.clone()]),
            _ => StubReply::error("FORBIDDEN", "stub: nothing is written in this test"),
        }
    })
    .await;
    let adapter = adapter(&server);
    let patch = IssuePatch {
        title: Some("Changed".into()),
        ..IssuePatch::default()
    };

    assert_eq!(
        adapter.update_issue("x1", patch.clone()).await,
        Err(TrackerError::OutOfScope)
    );
    assert_eq!(
        adapter.update_issue("OT-1", patch).await,
        Err(TrackerError::OutOfScope),
        "a key from another team doesn't match the team's own ENG-1"
    );
    assert_eq!(
        adapter.archive_issue("x1").await,
        Err(TrackerError::OutOfScope)
    );
    assert_eq!(
        adapter
            .add_link(
                LinkTarget::Issue("x1".into()),
                "https://example.com/notion",
                "Notion archive"
            )
            .await,
        Err(TrackerError::OutOfScope)
    );
    let child = NewIssue {
        title: "Child".into(),
        parent_id: Some("x1".into()),
        ..NewIssue::default()
    };
    assert_eq!(
        adapter.create_issue(child).await,
        Err(TrackerError::OutOfScope)
    );
    let rename = ProjectPatch {
        name: Some("Renamed".into()),
        ..ProjectPatch::default()
    };
    assert_eq!(
        adapter.update_project("p-shared", rename).await,
        Err(TrackerError::OutOfScope),
        "a project shared with another team isn't written"
    );
    assert!(
        server
            .requests()
            .iter()
            .all(|req| !req.query().trim_start().starts_with("mutation")),
        "no mutation was sent"
    );
}

#[tokio::test]
async fn comments_on_another_teams_issue_are_dropped_and_a_comment_only_change_arrives_through_its_cursor(
) {
    let comment = |id: &str, issue: Value| {
        json!({
            "id": id, "body": "Fixture comment", "createdAt": T1, "updatedAt": T1,
            "editedAt": null, "parent": null, "user": { "id": "u1", "name": "Stub User" },
            "issue": issue
        })
    };
    let comments = vec![
        comment("c2", json!({ "id": "x1", "team": { "id": OTHER } })),
        comment("c1", json!({ "id": "i1", "team": { "id": TEAM } })),
        comment("c3", Value::Null),
    ];
    let server = stub::serve(move |req| {
        if let Some(refused) = stub::unscoped(req, TEAM) {
            return refused;
        }
        match req.operation().as_deref() {
            // The issue itself hasn't changed since T0.
            Some("Issues") => page("issues", vec![]),
            Some("Comments") => page("comments", comments.clone()),
            _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unexpected operation"),
        }
    })
    .await;
    let adapter = adapter(&server);

    assert!(adapter
        .issues_changed(Some(T0), None)
        .await
        .unwrap()
        .items
        .is_empty());
    let changed = adapter.comments_changed(Some(T0), None).await.unwrap();
    assert_eq!(changed.items.len(), 1);
    let change = &changed.items[0];
    assert_eq!(
        (
            change.issue_id.as_str(),
            change.team_id.as_str(),
            change.updated_at.as_str()
        ),
        ("i1", TEAM, T1)
    );
    assert_eq!(change.comment.id, "c1");
    assert_eq!(change.comment.author.as_deref(), Some("Stub User"));
    assert_eq!(
        sent(&server, "Comments").variables()["filter"],
        json!({ "issue": { "team": { "id": { "eq": TEAM } } }, "updatedAt": { "gt": T0 } })
    );
}

#[tokio::test]
async fn the_detail_query_stays_under_the_point_cap_with_50_comments_and_50_attachments() {
    let mut node = issue_node("i1", "ENG-1", TEAM, T0);
    node["comments"] = json!({ "nodes": (0..50).map(|n| json!({
        "id": format!("c{n}"), "body": "Fixture", "createdAt": T0, "editedAt": null,
        "parent": null, "user": null
    })).collect::<Vec<_>>() });
    node["attachments"] = json!({ "nodes": (0..50).map(|n| json!({
        "id": format!("a{n}"), "title": format!("Link {n}"), "url": format!("https://example.com/{n}"),
        "sourceType": if n % 2 == 0 { Value::Null } else { json!("github") }, "createdAt": T0
    })).collect::<Vec<_>>() });
    let server =
        stub::serve(move |req| page("issues", filtered(req, std::slice::from_ref(&node)))).await;

    let issue = adapter(&server).issue("ENG-1").await.unwrap();
    assert_eq!(issue.comments.len(), 50);
    assert_eq!((issue.links.len(), issue.attachments.len()), (25, 25));
    assert_eq!(issue.attachments[0].source.as_deref(), Some("github"));
    let query = sent(&server, "IssueDetail").query().to_string();
    assert!(query.contains("comments(first: 50)") && query.contains("attachments(first: 50)"));
    let cost = points(&query);
    assert!(cost < POINT_CAP, "the detail query costs {cost} points");
}

// ---- Points ---------------------------------------------------------------------

/// Linear's complexity rules (developers.linear.app, rate limiting), counted
/// generously: a scalar field 0.1 point, an object 1, and a connection
/// multiplies everything under it, `pageInfo` included, by its `first` (50
/// when absent). Counted in tenths, rounded up.
fn points(query: &str) -> u64 {
    let text: Vec<char> = query.chars().collect();
    let mut at = text
        .iter()
        .position(|c| *c == '{')
        .expect("a selection set")
        + 1;
    let (tenths, _) = selection(&text, &mut at);
    tenths.div_ceil(10)
}

/// One selection set's cost in tenths, and whether it lists `nodes` (a
/// connection).
fn selection(text: &[char], at: &mut usize) -> (u64, bool) {
    let mut tenths = 0;
    let mut has_nodes = false;
    loop {
        skip_space(text, at);
        match text.get(*at) {
            None => return (tenths, has_nodes),
            Some('}') => {
                *at += 1;
                return (tenths, has_nodes);
            }
            _ => {}
        }
        let mut name = word(text, at);
        skip_space(text, at);
        if text.get(*at) == Some(&':') {
            *at += 1;
            skip_space(text, at);
            name = word(text, at);
            skip_space(text, at);
        }
        assert!(!name.is_empty(), "unexpected {:?} at {}", text.get(*at), at);
        let mut first = None;
        if text.get(*at) == Some(&'(') {
            first = first_argument(&arguments(text, at));
            skip_space(text, at);
        }
        has_nodes |= name == "nodes";
        if text.get(*at) == Some(&'{') {
            *at += 1;
            let (inner, connection) = selection(text, at);
            let times = first.unwrap_or(if connection { 50 } else { 1 });
            tenths += 10 + times * inner;
        } else {
            tenths += 1;
        }
    }
}

fn skip_space(text: &[char], at: &mut usize) {
    while text
        .get(*at)
        .is_some_and(|c| c.is_whitespace() || *c == ',')
    {
        *at += 1;
    }
}

fn word(text: &[char], at: &mut usize) -> String {
    let start = *at;
    while text
        .get(*at)
        .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
    {
        *at += 1;
    }
    text[start..*at].iter().collect()
}

/// The text inside a balanced `( … )`, leaving `at` after it.
fn arguments(text: &[char], at: &mut usize) -> String {
    let start = *at + 1;
    let mut depth = 0;
    while let Some(c) = text.get(*at) {
        *at += 1;
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return text[start..*at - 1].iter().collect();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced arguments");
}

fn first_argument(arguments: &str) -> Option<u64> {
    let rest = arguments.split("first:").nth(1)?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[test]
fn the_point_counter_follows_linears_rules() {
    assert_eq!(points("query A { viewer { id name } }"), 2);
    assert_eq!(points("query B { issues(first: 10) { nodes { id } } }"), 12);
    assert_eq!(points("query C { issues { nodes { id title } } }"), 61);
    assert_eq!(
        points("query D($f: IssueFilter) { issues(filter: $f, first: 2) { nodes { id labels(first: 3) { nodes { name } } } } }"),
        // 1 + 2 * (1 + 0.1 + (1 + 3 * 1.1)) = 11.8
        12
    );
}

#[test]
fn every_query_stays_under_the_point_cap() {
    let queries = [
        ("Viewer", VIEWER_QUERY.to_string()),
        ("Team", TEAM_QUERY.to_string()),
        ("States", states_query()),
        ("Labels", labels_query()),
        ("Members", members_query()),
        ("Cycles", cycles_query()),
        ("Milestones", milestones_query()),
        ("Issues", issues_query()),
        ("IssueDetail", detail_query()),
        ("IssueHistory", history_query()),
        ("IssueTarget", ISSUE_TARGET_QUERY.to_string()),
        ("Comments", comments_query()),
        ("CommentCreate", COMMENT_CREATE_MUTATION.to_string()),
        ("Attachments", attachments_query()),
        ("Projects", projects_query()),
        ("ProjectTarget", PROJECT_TARGET_QUERY.to_string()),
        ("IssueCreate", issue_create_mutation()),
        ("IssueUpdate", issue_update_mutation()),
        ("IssueArchive", ISSUE_ARCHIVE_MUTATION.to_string()),
        ("ProjectCreate", project_create_mutation()),
        ("ProjectUpdate", project_update_mutation()),
        ("IssueLink", issue_link_mutation()),
        ("ProjectLink", PROJECT_LINK_MUTATION.to_string()),
        ("ProjectStatusUpdate", STATUS_UPDATE_MUTATION.to_string()),
        ("FileUpload", FILE_UPLOAD_MUTATION.to_string()),
        ("AttachmentCreate", ATTACHMENT_CREATE_MUTATION.to_string()),
        ("LabelCreate", LABEL_CREATE_MUTATION.to_string()),
    ];
    for (name, query) in queries {
        let mut cost = points(&query);
        if name == "Milestones" {
            // Linear counted this doubly nested shape 5,555 where this counter
            // said 3,356 (live, 2026-11-08, 50 projects): scale by that.
            cost = cost * 17 / 10;
        }
        eprintln!("points {name}: {cost}");
        assert!(cost > 0 && cost < POINT_CAP, "{name}: {cost}");
        let operation = query
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .split('(')
            .next()
            .unwrap_or("");
        assert_eq!(operation, name, "the stub reads the operation name");
        assert_eq!(
            query.matches('{').count(),
            query.matches('}').count(),
            "{name} is balanced"
        );
    }
}

// ---- The rest of the adapter -----------------------------------------------------

#[test]
fn the_backoff_starts_at_30_seconds_and_doubles_to_15_minutes() {
    let delay = |streak| backoff_delay(BACKOFF_BASE, BACKOFF_MAX, streak);
    assert_eq!(delay(1), Duration::from_secs(30));
    assert_eq!(delay(2), Duration::from_secs(60));
    assert_eq!(delay(5), Duration::from_secs(480));
    assert_eq!(delay(6), Duration::from_secs(900));
    assert_eq!(delay(64), Duration::from_secs(900));
}

#[tokio::test]
async fn requests_carry_the_key_as_is_and_record_the_rate_headers() {
    let server = stub::serve(|_req| {
        StubReply::data(
            json!({ "viewer": { "id": "u1", "name": "Stub User", "email": "stub@example.com" } }),
        )
        .with_rate(2400, 2500, 1_762_578_000_000)
        .with_header("X-RateLimit-Complexity-Remaining", "2900000")
        .with_header("X-RateLimit-Complexity-Limit", "3000000")
        .with_header("X-Complexity", "2")
    })
    .await;
    let adapter = adapter(&server);
    assert!(adapter.rate().is_none());

    let me = adapter.whoami().await.unwrap();
    assert_eq!(
        me,
        TrackerUser {
            id: "u1".into(),
            name: "Stub User".into(),
            email: Some("stub@example.com".into()),
            display_name: None,
        }
    );
    let req = sent(&server, "Viewer");
    assert_eq!(
        req.headers["authorization"], KEY,
        "the key as is, no Bearer"
    );
    assert_eq!(req.headers["content-type"], "application/json");
    let rate = adapter.rate().unwrap();
    assert_eq!(
        (rate.requests_remaining, rate.requests_limit),
        (Some(2400), Some(2500))
    );
    assert_eq!(
        (rate.complexity_remaining, rate.complexity_limit),
        (Some(2_900_000), Some(3_000_000))
    );
    assert_eq!(rate.reset_at.as_deref(), Some("2025-11-08T05:00:00.000Z"));
    assert!(!rate.is_low());
    assert_eq!(adapter.last_points(), Some(2));
    assert!(!format!("{adapter:?}").contains(KEY));
    assert_eq!(adapter.provider(), PROVIDER);
    assert_eq!(adapter.writable_fields(), &Field::ALL);
}

#[tokio::test]
async fn a_refused_key_reads_as_auth_and_no_error_carries_the_key() {
    let server = stub::serve(|req| match req.operation().as_deref() {
        Some("Viewer") => StubReply::error("AUTHENTICATION_ERROR", "Authentication required"),
        Some("Team") => StubReply::status(401),
        Some("Members") => StubReply::data(json!({ "team": null })),
        // A hostile server: the key and work text as the code and the path.
        Some("Labels") => {
            let echoed = req.headers["authorization"].clone();
            StubReply {
                status: 200,
                headers: Vec::new(),
                body: json!({ "errors": [{
                    "message": "x",
                    "path": [echoed, "Secret plan", "issueLabels"],
                    "extensions": { "code": echoed }
                }] })
                .to_string(),
            }
        }
        // A server message that echoes what it was sent: the key and work text.
        _ => StubReply {
            status: 400,
            headers: Vec::new(),
            body: json!({ "errors": [{
                "message": format!("Argument Validation Error: 'Secret plan' sent with {}", req.headers["authorization"]),
                "path": ["team", "states"],
                "extensions": { "code": "INVALID_INPUT" }
            }] })
            .to_string(),
        },
    })
    .await;
    let adapter = adapter(&server);

    assert_eq!(adapter.whoami().await, Err(TrackerError::Auth));
    assert_eq!(adapter.scope().await, Err(TrackerError::Auth));
    let invalid = adapter.states().await.unwrap_err();
    assert_eq!(
        invalid,
        TrackerError::Provider("INVALID_INPUT at team.states".into()),
        "the code and the field path, nothing from the message"
    );
    assert!(!invalid.to_string().contains("Secret plan"));
    let hostile = adapter.labels().await.unwrap_err();
    assert_eq!(
        hostile,
        TrackerError::Provider("an unrecognized error at issueLabels".into()),
        "an unknown code and path words Hollow didn't send are left out"
    );
    let unseen = adapter.members().await.unwrap_err();
    assert!(
        matches!(unseen, TrackerError::NotConfigured(_)),
        "{unseen:?}"
    );
    for error in [TrackerError::Auth, invalid, unseen, hostile] {
        assert!(!error.to_string().contains(KEY));
        assert!(!format!("{error:?}").contains(KEY));
    }
}

#[tokio::test]
async fn an_issue_maps_every_detail_field() {
    let other_side = |key: &str| json!({ "identifier": key, "team": { "id": TEAM } });
    let mut node = issue_node("i1", "ENG-7", TEAM, T0);
    node["title"] = json!("Mapped");
    node["description"] = json!("Body");
    node["priority"] = json!(2.0);
    node["dueDate"] = json!("2026-11-20");
    node["estimate"] = json!(3);
    node["archivedAt"] = json!(T1);
    node["state"] = json!({ "id": "s-done", "name": "Delivered", "type": "completed", "position": 4.0, "color": "#5e6ad2" });
    node["assignee"] = json!({ "id": "u1", "name": "Stub User", "email": "stub@example.com" });
    node["creator"] = json!({ "id": "u2", "name": "Other User" });
    node["project"] = json!({ "id": "p1" });
    node["parent"] = json!({ "id": "i0", "team": { "id": TEAM } });
    node["cycle"] =
        json!({ "id": "cy1", "number": 12.0, "name": null, "startsAt": T0, "endsAt": T1 });
    node["projectMilestone"] = json!({ "id": "m1", "name": "Beta", "targetDate": "2026-12-01", "project": { "id": "p1" } });
    node["labels"]["nodes"] = json!([{ "name": "Bug" }, { "name": "Hollow" }]);
    node["children"]["nodes"] = json!([other_side("ENG-8")]);
    node["relations"]["nodes"] = json!([
        { "type": "blocks", "relatedIssue": other_side("ENG-9") },
        { "type": "duplicate", "relatedIssue": other_side("ENG-3") },
        { "type": "similar", "relatedIssue": other_side("ENG-4") }
    ]);
    node["inverseRelations"]["nodes"] = json!([
        { "type": "blocks", "issue": other_side("ENG-10") },
        { "type": "related", "issue": other_side("ENG-11") },
        { "type": "duplicate", "issue": other_side("ENG-12") }
    ]);
    let server =
        stub::serve(move |req| page("issues", filtered(req, std::slice::from_ref(&node)))).await;

    let issue = adapter(&server).issue("i1").await.unwrap();
    assert_eq!(
        (
            issue.key.as_str(),
            issue.title.as_str(),
            issue.description.as_str()
        ),
        ("ENG-7", "Mapped", "Body")
    );
    assert_eq!(issue.state.kind, StateKind::Completed);
    assert!(issue.state.kind.is_done(), "the kind decides, not the name");
    assert_eq!(issue.state.name, "Delivered");
    assert_eq!(issue.priority, 2);
    assert_eq!(issue.due.as_deref(), Some("2026-11-20"));
    assert_eq!(issue.estimate, Some(3.0));
    assert_eq!(issue.archived_at.as_deref(), Some(T1));
    assert_eq!(
        issue.assignee.unwrap().email.as_deref(),
        Some("stub@example.com")
    );
    assert_eq!(issue.creator.unwrap().name, "Other User");
    assert_eq!(issue.project_id.as_deref(), Some("p1"));
    assert_eq!(issue.parent_id.as_deref(), Some("i0"));
    let cycle = issue.cycle.unwrap();
    assert_eq!((cycle.number, cycle.name), (12, None));
    let milestone = issue.milestone.unwrap();
    assert_eq!(
        (milestone.name.as_str(), milestone.project_id.as_deref()),
        ("Beta", Some("p1"))
    );
    assert_eq!(issue.labels, vec!["Bug", "Hollow"]);
    assert_eq!(issue.children, vec!["ENG-8"]);
    let relation = |kind, key: &str| TrackerRelation {
        kind,
        key: key.into(),
    };
    assert_eq!(
        issue.relations,
        vec![
            relation(RelationKind::Blocks, "ENG-9"),
            relation(RelationKind::DuplicateOf, "ENG-3"),
            relation(RelationKind::BlockedBy, "ENG-10"),
            relation(RelationKind::Related, "ENG-11"),
        ]
    );
    assert_eq!(issue.created_at, "2026-11-07T21:27:00.000Z");
    assert!(!issue.trashed);
}

#[tokio::test]
async fn an_issue_whose_nested_list_came_back_full_is_read_again_in_detail() {
    let labels = |count: u32| {
        json!((0..count)
            .map(|n| json!({ "name": format!("L{n}") }))
            .collect::<Vec<_>>())
    };
    let mut listed = issue_node("i1", "ENG-1", TEAM, T0);
    listed["labels"]["nodes"] = labels(NESTED_PAGE);
    let mut full = listed.clone();
    full["labels"]["nodes"] = labels(NESTED_PAGE + 5);
    let server = stub::serve(move |req| match req.operation().as_deref() {
        Some("Issues") => page("issues", vec![listed.clone()]),
        _ => page("issues", filtered(req, std::slice::from_ref(&full))),
    })
    .await;

    let got = adapter(&server).issues_changed(None, None).await.unwrap();
    assert_eq!(got.items[0].labels.len(), NESTED_PAGE as usize + 5);
    assert_eq!(operations(&server), vec!["Issues", "IssueDetail"]);
}

#[tokio::test]
async fn writes_send_only_their_fields_in_the_team_and_check_the_reply() {
    let replies = Arc::new(AtomicUsize::new(0));
    let counter = replies.clone();
    let ours = issue_node("i1", "ENG-1", TEAM, T0);
    let server = stub::serve(move |req| match req.operation().as_deref() {
        Some("IssueTarget") => page("issues", filtered(req, std::slice::from_ref(&ours))),
        Some("IssueCreate") => {
            StubReply::data(json!({ "issueCreate": { "success": true, "issue": issue_node("i2", "ENG-2", TEAM, T1) } }))
        }
        Some("IssueUpdate") => {
            // The second update's reply claims another team.
            let team = if counter.fetch_add(1, Ordering::SeqCst) == 0 { TEAM } else { OTHER };
            StubReply::data(json!({ "issueUpdate": { "success": true, "issue": issue_node("i1", "ENG-1", team, T1) } }))
        }
        Some("IssueArchive") => StubReply::data(json!({ "issueArchive": { "success": false } })),
        _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unexpected operation"),
    })
    .await;
    let adapter = adapter(&server);

    let created = adapter
        .create_issue(NewIssue {
            title: " New task ".into(),
            label_ids: vec!["l1".into()],
            parent_id: Some("ENG-1".into()),
            ..NewIssue::default()
        })
        .await
        .unwrap();
    assert_eq!(created.key, "ENG-2");
    assert_eq!(
        sent(&server, "IssueCreate").variables()["input"],
        json!({ "teamId": TEAM, "title": "New task", "labelIds": ["l1"], "parentId": "i1" })
    );

    let patch = IssuePatch {
        due: Some(None),
        assignee_id: Some(Some("u2".into())),
        estimate: Some(Some(3.0)),
        ..IssuePatch::default()
    };
    let updated = adapter.update_issue("ENG-1", patch.clone()).await.unwrap();
    assert_eq!(updated.updated_at, T1);
    let update = sent(&server, "IssueUpdate");
    assert_eq!(update.variables()["id"], "i1");
    assert_eq!(
        update.variables()["input"],
        json!({ "dueDate": null, "assigneeId": "u2", "estimate": 3.0 })
    );
    assert_eq!(
        adapter.update_issue("ENG-1", patch).await,
        Err(TrackerError::OutOfScope),
        "a reply from another team is refused"
    );
    assert!(matches!(
        adapter.archive_issue("ENG-1").await,
        Err(TrackerError::Provider(_))
    ));
    assert!(matches!(
        adapter
            .add_link(LinkTarget::Issue("ENG-1".into()), "notion page", "Notion")
            .await,
        Err(TrackerError::Provider(_))
    ));
    assert!(matches!(
        adapter.create_issue(NewIssue::default()).await,
        Err(TrackerError::Provider(_))
    ));
}

#[tokio::test]
async fn projects_and_milestones_come_from_the_team_only() {
    let project = |id: &str, teams: &[&str]| {
        json!({
            "id": id, "url": format!("https://linear.app/example/project/{id}"), "name": format!("Project {id}"),
            "description": "", "startDate": null, "targetDate": "2026-12-01", "updatedAt": T0,
            "progress": 0.5, "status": { "name": "In Progress", "type": "started" }, "lead": null,
            "teams": { "nodes": teams.iter().map(|team| json!({ "id": team })).collect::<Vec<_>>() },
            "externalLinks": { "nodes": [{ "id": "el1", "label": "GitHub", "url": "https://github.com/example/repo", "createdAt": T0 }] }
        })
    };
    let mut shared = project("p3", &[TEAM, OTHER]);
    shared["identifier"] = json!("P-ENG-3");
    let projects = vec![project("p1", &[TEAM]), project("p2", &[OTHER]), shared];
    let server = stub::serve(move |req| match req.operation().as_deref() {
        Some("Projects") => page("projects", projects.clone()),
        Some("Milestones") => page(
            "projects",
            vec![json!({ "id": "p1", "projectMilestones": { "nodes": [{ "id": "m1", "name": "Beta", "targetDate": null }] } })],
        ),
        _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unexpected operation"),
    })
    .await;
    let adapter = adapter(&server);

    let got = adapter.projects_changed(Some(T0)).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(
        (got[0].id.as_str(), got[0].status.kind.as_str()),
        ("p1", "started")
    );
    assert_eq!(got[1].key.as_deref(), Some("P-ENG-3"));
    assert_eq!(
        got[1].team_ids,
        vec![TEAM.to_string()],
        "another team's id never leaves the adapter"
    );
    assert_eq!(got[0].links[0].url, "https://github.com/example/repo");
    assert_eq!(
        sent(&server, "Projects").variables()["filter"],
        json!({ "accessibleTeams": { "some": { "id": { "eq": TEAM } } }, "updatedAt": { "gt": T0 } })
    );
    let milestones = adapter.milestones(&["p1".to_string()]).await.unwrap();
    assert_eq!(milestones.len(), 1);
    assert_eq!(milestones[0].project_id.as_deref(), Some("p1"));
    assert_eq!(
        sent(&server, "Milestones").variables()["filter"]["accessibleTeams"],
        json!({ "some": { "id": { "eq": TEAM } } })
    );
}

#[test]
fn the_adapter_needs_this_provider_the_api_transport_a_team_and_a_key() {
    let mut config = TrackerConfig {
        provider: Some(PROVIDER.into()),
        team_id: Some(TEAM.into()),
        ..TrackerConfig::default()
    };
    assert!(LinearAdapter::configured(&config, Ok(Some(KEY.into()))).is_ok());
    let refused = |config: &TrackerConfig, key: Result<Option<String>, ()>| {
        matches!(
            LinearAdapter::configured(config, key),
            Err(TrackerError::NotConfigured(_))
        )
    };
    assert!(refused(&config, Ok(None)));
    assert!(refused(&config, Err(())));
    config.transport = "mcp".into();
    assert!(refused(&config, Ok(Some(KEY.into()))));
    config.transport = "graphql".into();
    config.team_id = None;
    assert!(refused(&config, Ok(Some(KEY.into()))));
    config.team_id = Some(TEAM.into());
    config.provider = None;
    assert!(refused(&config, Ok(Some(KEY.into()))));
    assert!(matches!(
        LinearAdapter::new(TEAM, "bad\nkey"),
        Err(TrackerError::NotConfigured(_))
    ));
    assert!(
        matches!(
            LinearAdapter::new("ENG", KEY),
            Err(TrackerError::NotConfigured(_))
        ),
        "a team key in place of its id"
    );
    assert_eq!(issue_number("ENG-37"), Some(37));
    assert_eq!(issue_number("11111111-1111-4111-8111-111111111111"), None);
    assert_eq!(issue_number("AI-"), None);
}

// ---- Live -------------------------------------------------------------------------

/// The read-only live smoke (overnight board rule 8). On this Mac only, the
/// key read into the environment inside one command:
///
/// `LINEAR_API_KEY="$(security find-generic-password -s hollow-linear -a default -w)"
/// TRACKER_SMOKE_TEAM=<team id> hollow-slot --here cargo test --lib
/// tracker::linear::tests::live_read_only_smoke -- --ignored --nocapture`
///
/// It calls reads only and prints counts, points and schema checks, never
/// issue text or names.
#[tokio::test]
#[ignore = "live and read-only: needs LINEAR_API_KEY and TRACKER_SMOKE_TEAM"]
async fn live_read_only_smoke() {
    let (Ok(key), Ok(team)) = (
        std::env::var("LINEAR_API_KEY"),
        std::env::var("TRACKER_SMOKE_TEAM"),
    ) else {
        eprintln!("smoke: no key");
        return;
    };
    let adapter = LinearAdapter::new(&team, &key).expect("adapter");
    drop(key);
    let report =
        |what: &str, result: Result<usize, TrackerError>, adapter: &LinearAdapter| match result {
            Ok(count) => eprintln!("smoke {what}: {count} (points {:?})", adapter.last_points()),
            Err(error) => eprintln!("smoke {what}: FAILED {error}"),
        };
    report("viewer", adapter.whoami().await.map(|_| 1), &adapter);
    match adapter.scope().await {
        Ok(scope) => eprintln!(
            "smoke team: key {} (id matches: {})",
            scope.key,
            scope.id == team
        ),
        Err(error) => eprintln!("smoke team: FAILED {error}"),
    }
    report("states", adapter.states().await.map(|v| v.len()), &adapter);
    report("labels", adapter.labels().await.map(|v| v.len()), &adapter);
    report(
        "members",
        adapter.members().await.map(|v| v.len()),
        &adapter,
    );
    report("cycles", adapter.cycles().await.map(|v| v.len()), &adapter);
    let issues = adapter.issues_changed(None, None).await;
    eprintln!("smoke issues estimate: {} points", points(&issues_query()));
    let ids: Vec<String> = issues
        .as_ref()
        .map(|page| page.items.iter().map(|issue| issue.id.clone()).collect())
        .unwrap_or_default();
    if let Ok(page) = &issues {
        eprintln!("smoke issues: more pages {}", page.next.is_some());
    }
    report("issues page", issues.map(|page| page.items.len()), &adapter);
    report(
        "comments page",
        adapter
            .comments_changed(None, None)
            .await
            .map(|page| page.items.len()),
        &adapter,
    );
    match adapter.attachments(&ids).await {
        Ok(found) => {
            let mut sources = BTreeSet::new();
            let mut count = 0;
            for (_, links) in &found {
                count += links.len();
                sources.extend(links.iter().filter_map(|link| link.source.clone()));
            }
            eprintln!(
                "smoke attachments: {count} on {} issues, sources {sources:?} (points {:?})",
                found.len(),
                adapter.last_points()
            );
        }
        Err(error) => eprintln!("smoke attachments: FAILED {error}"),
    }
    if let Some(first) = ids.first() {
        match adapter.issue(first).await {
            Ok(issue) => eprintln!(
                "smoke detail: {} comments, {} links, {} attachments (points {:?}, estimate {})",
                issue.comments.len(),
                issue.links.len(),
                issue.attachments.len(),
                adapter.last_points(),
                points(&detail_query())
            ),
            Err(error) => eprintln!("smoke detail: FAILED {error}"),
        }
    }
    let projects = adapter.projects_changed(None).await;
    let project_ids: Vec<String> = projects
        .as_ref()
        .map(|list| list.iter().map(|project| project.id.clone()).collect())
        .unwrap_or_default();
    report("projects", projects.map(|list| list.len()), &adapter);
    report(
        "milestones",
        adapter.milestones(&project_ids).await.map(|v| v.len()),
        &adapter,
    );
    eprintln!("smoke rate: {:?}", adapter.rate());

    // The write paths can't run tonight; check their names against the schema.
    let schema = adapter
        .post(
            "query Schema { m: __type(name: \"Mutation\") { fields { name } } \
             ic: __type(name: \"IssueCreateInput\") { inputFields { name } } \
             iu: __type(name: \"IssueUpdateInput\") { inputFields { name } } \
             pc: __type(name: \"ProjectCreateInput\") { inputFields { name } } \
             pu: __type(name: \"ProjectUpdateInput\") { inputFields { name } } \
             el: __type(name: \"EntityExternalLinkCreateInput\") { inputFields { name } } \
             p: __type(name: \"Project\") { fields { name } } }",
            json!({}),
        )
        .await;
    match schema {
        Ok(schema) => {
            let listed = |alias: &str, list: &str| -> Vec<String> {
                schema[alias][list]
                    .as_array()
                    .map(|fields| {
                        fields
                            .iter()
                            .filter_map(|f| f["name"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let wanted: [(&str, &str, &[&str]); 6] = [
                (
                    "m",
                    "fields",
                    &[
                        "issueCreate",
                        "issueUpdate",
                        "issueArchive",
                        "projectCreate",
                        "projectUpdate",
                        "attachmentLinkURL",
                        "entityExternalLinkCreate",
                    ],
                ),
                (
                    "ic",
                    "inputFields",
                    &[
                        "teamId",
                        "title",
                        "description",
                        "stateId",
                        "priority",
                        "assigneeId",
                        "labelIds",
                        "projectId",
                        "dueDate",
                        "parentId",
                        "estimate",
                        "cycleId",
                        "projectMilestoneId",
                    ],
                ),
                (
                    "iu",
                    "inputFields",
                    &[
                        "title",
                        "description",
                        "stateId",
                        "priority",
                        "assigneeId",
                        "labelIds",
                        "projectId",
                        "dueDate",
                        "parentId",
                        "estimate",
                        "cycleId",
                        "projectMilestoneId",
                    ],
                ),
                (
                    "pc",
                    "inputFields",
                    &[
                        "teamIds",
                        "name",
                        "description",
                        "leadId",
                        "startDate",
                        "targetDate",
                        "statusId",
                    ],
                ),
                (
                    "pu",
                    "inputFields",
                    &[
                        "name",
                        "description",
                        "leadId",
                        "startDate",
                        "targetDate",
                        "statusId",
                    ],
                ),
                ("el", "inputFields", &["projectId", "url", "label"]),
            ];
            for (alias, list, names) in wanted {
                let have = listed(alias, list);
                let missing: Vec<&&str> = names
                    .iter()
                    .filter(|name| !have.iter().any(|h| h == *name))
                    .collect();
                eprintln!("smoke schema {alias}: missing {missing:?}");
            }
            let keys: Vec<String> = listed("p", "fields")
                .into_iter()
                .filter(|name| {
                    let name = name.to_ascii_lowercase();
                    name.contains("ident") || name.contains("slug") || name == "key"
                })
                .collect();
            eprintln!("smoke schema project key fields: {keys:?}");
        }
        Err(error) => eprintln!("smoke schema: FAILED {error}"),
    }
}

#[tokio::test]
async fn company_comments_are_scoped_complete_and_recheck_authority_before_mutation() {
    let server = stub::serve(|req| {
        if let Some(error) = stub::unscoped(req, TEAM) { return error; }
        match req.operation().as_deref() {
            Some("IssueTarget") => page("issues", vec![issue_node("i1", "ENG-1", TEAM, T0)]),
            Some("Comments") => {
                assert_eq!(req.variables().pointer("/filter/issue/id/eq"), Some(&json!("i1")));
                assert_eq!(req.variables().pointer("/filter/issue/team/id/eq"), Some(&json!(TEAM)));
                let node = |id| json!({"id":id,"body":"fixture","createdAt":T0,"updatedAt":T0,
                    "issue":{"id":"i1","team":{"id":TEAM}}});
                if req.variables()["after"].is_null() {
                    StubReply::data(json!({"comments":{"nodes":[node("c1")],"pageInfo":{"hasNextPage":true,"endCursor":"next"}}}))
                } else { page("comments", vec![node("c2")]) }
            }
            Some("CommentCreate") => {
                assert_eq!(req.variables()["input"]["issueId"], "i1");
                StubReply::data(json!({"commentCreate":{"success":true,"comment":{
                    "id":"c3","body":req.variables()["input"]["body"],"createdAt":T0,"updatedAt":T0,
                    "issue":{"id":"i1","team":{"id":TEAM}}}}}))
            }
            _ => StubReply::status(500),
        }
    }).await;
    let remote = adapter(&server);
    assert_eq!(remote.issue_comments("i1").await.unwrap().len(), 2);
    assert!(remote
        .create_issue_comment("i1", "fixed host text", &|| Err(TrackerError::OutOfScope))
        .await
        .is_err());
    assert!(!operations(&server).contains(&"CommentCreate".to_string()));
    let result = remote
        .create_issue_comment("i1", "fixed host text", &|| Ok(()))
        .await
        .unwrap();
    assert_eq!(result.comment.id, "c3");
    assert_eq!(result.team_id, TEAM);
}

#[tokio::test]
async fn company_comment_enumeration_refuses_incomplete_or_unsafe_pages() {
    for kind in [
        "missing-page-info",
        "missing-cursor",
        "repeated-cursor",
        "duplicate-id",
        "wrong-issue",
        "wrong-team",
        "missing-nodes",
    ] {
        let server = stub::serve(move |req| {
            if req.operation().as_deref() == Some("IssueTarget") {
                return page("issues", vec![issue_node("i1", "ENG-1", TEAM, T0)]);
            }
            let node = json!({"id":"c1","body":"fixture","createdAt":T0,"updatedAt":T0,
                "issue":{"id":if kind == "wrong-issue" {"i2"} else {"i1"},
                         "team":{"id":if kind == "wrong-team" {OTHER} else {TEAM}}}});
            let mut conn =
                json!({"nodes":[node.clone()],"pageInfo":{"hasNextPage":false,"endCursor":null}});
            match kind {
                "missing-page-info" => {
                    conn.as_object_mut().unwrap().remove("pageInfo");
                }
                "missing-cursor" => {
                    conn["pageInfo"]["hasNextPage"] = json!(true);
                }
                "repeated-cursor" => {
                    conn["nodes"] = json!([]);
                    conn["pageInfo"] = json!({"hasNextPage":true,"endCursor":"same"});
                }
                "duplicate-id" => {
                    conn["nodes"] = json!([node.clone(), node]);
                }
                "missing-nodes" => {
                    conn.as_object_mut().unwrap().remove("nodes");
                }
                _ => {}
            }
            StubReply::data(json!({"comments":conn}))
        })
        .await;
        assert!(
            adapter(&server).issue_comments("i1").await.is_err(),
            "{kind}"
        );
    }
}

#[tokio::test]
async fn company_comment_mutation_refuses_other_team_and_bad_receipts() {
    let server = stub::serve(|_| page("issues", vec![issue_node("i1", "ENG-1", OTHER, T0)])).await;
    assert!(adapter(&server)
        .create_issue_comment("i1", "safe", &|| Ok(()))
        .await
        .is_err());
    assert!(!operations(&server).contains(&"CommentCreate".to_string()));
    for kind in [
        "wrong-issue",
        "wrong-team",
        "changed-body",
        "missing-success",
    ] {
        let server = stub::serve(move |req| {
            if req.operation().as_deref() == Some("IssueTarget") {
                return page("issues", vec![issue_node("i1", "ENG-1", TEAM, T0)]);
            }
            let mut payload = json!({"success":true,"comment":{"id":"c1",
                "body":if kind == "changed-body" {"different"} else {"safe"},"createdAt":T0,"updatedAt":T0,
                "issue":{"id":if kind == "wrong-issue" {"i2"} else {"i1"},
                    "team":{"id":if kind == "wrong-team" {OTHER} else {TEAM}}}}});
            if kind == "missing-success" { payload.as_object_mut().unwrap().remove("success"); }
            StubReply::data(json!({"commentCreate":payload}))
        }).await;
        assert!(
            adapter(&server)
                .create_issue_comment("i1", "safe", &|| Ok(()))
                .await
                .is_err(),
            "{kind}"
        );
    }
}

// ---- Issue history (the issue detail's activity) -----------------------------

/// A stub that answers `IssueHistory` with these entries on ENG-1 (and ENG-9
/// of another team, which the adapter must never return).
async fn history_server(entries: Value) -> StubServer {
    history_server_with(json!({ "nodes": entries })).await
}

/// The same stub with ENG-1's whole history connection given (its pageInfo).
async fn history_server_with(connection: Value) -> StubServer {
    let mut ours = issue_node("i1", "ENG-1", TEAM, T0);
    ours["history"] = connection;
    let mut theirs = issue_node("x9", "ENG-9", OTHER, T0);
    theirs["history"] =
        json!({ "nodes": [{ "id": "hx", "createdAt": T0, "updatedDescription": true }] });
    let issues = vec![ours, theirs];
    stub::serve(move |req| {
        if let Some(refused) = stub::unscoped(req, TEAM) {
            return refused;
        }
        match req.operation().as_deref() {
            Some("IssueHistory") => page("issues", filtered(req, &issues)),
            _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unexpected operation"),
        }
    })
    .await
}

#[tokio::test]
async fn an_issue_history_maps_each_kind_of_change_oldest_first() {
    let server = history_server(json!([
        { "id": "h3", "createdAt": T1, "actor": { "name": "Ana" },
          "fromState": { "name": "Todo", "type": "unstarted" },
          "toState": { "name": "In Progress", "type": "started" },
          "fromAssignee": null, "toAssignee": { "name": "Ana" } },
        { "id": "h1", "createdAt": T0, "actor": { "name": "Ben" },
          "fromPriority": 0, "toPriority": 2,
          "addedLabels": [{ "name": "Bug" }], "removedLabels": [{ "name": "Idea" }] },
        { "id": "h2", "createdAt": "2026-11-08T05:10:00.000Z", "actor": null,
          "fromProject": null, "toProject": { "name": "Hollow" },
          "fromTitle": "Old title", "toTitle": "New title", "updatedDescription": true },
        { "id": "h4", "createdAt": T1, "actor": { "name": "Ana" },
          "fromPriority": 2, "toPriority": 2, "addedLabels": [], "removedLabels": null }
    ]))
    .await;
    let history = adapter(&server).issue_history("ENG-1").await.unwrap();
    assert!(!history.more, "no pageInfo reads as nothing left out");
    let events = history.events;
    let kinds: Vec<(&str, HistoryKind)> = events.iter().map(|e| (e.id.as_str(), e.kind)).collect();
    assert_eq!(
        kinds,
        vec![
            ("h1:priority", HistoryKind::Priority),
            ("h1:labels", HistoryKind::Labels),
            ("h2:project", HistoryKind::Project),
            ("h2:title", HistoryKind::Title),
            ("h2:description", HistoryKind::Description),
            ("h3:state", HistoryKind::State),
            ("h3:assignee", HistoryKind::Assignee),
        ],
        "oldest first, one event per field, and an entry that changed nothing shown is left out"
    );
    let by = |id: &str| events.iter().find(|e| e.id == id).unwrap();
    let state = by("h3:state");
    assert_eq!(state.actor.as_deref(), Some("Ana"));
    assert_eq!(
        (state.from.as_deref(), state.to.as_deref()),
        (Some("Todo"), Some("In Progress"))
    );
    assert_eq!(
        (state.from_kind, state.to_kind),
        (Some(StateKind::Unstarted), Some(StateKind::Started))
    );
    let assignee = by("h3:assignee");
    assert_eq!(
        (assignee.from.as_deref(), assignee.to.as_deref()),
        (None, Some("Ana"))
    );
    let priority = by("h1:priority");
    assert_eq!(
        (priority.from.as_deref(), priority.to.as_deref()),
        (Some("0"), Some("2"))
    );
    let labels = by("h1:labels");
    assert_eq!(
        (labels.added.clone(), labels.removed.clone()),
        (vec!["Bug".to_string()], vec!["Idea".to_string()])
    );
    let project = by("h2:project");
    assert_eq!(project.actor, None, "an automation has no name");
    assert_eq!(
        (project.from.as_deref(), project.to.as_deref()),
        (None, Some("Hollow"))
    );
    let title = by("h2:title");
    assert_eq!(
        (title.from.as_deref(), title.to.as_deref()),
        (Some("Old title"), Some("New title"))
    );
    let request = sent(&server, "IssueHistory");
    assert_eq!(
        request.variables().pointer("/filter/team/id/eq"),
        Some(&json!(TEAM))
    );
    assert!(!request.query().trim_start().starts_with("mutation"));
}

#[tokio::test]
async fn a_history_with_a_next_page_says_older_entries_were_left_out() {
    let entry = json!({ "id": "h1", "createdAt": T1, "actor": { "name": "Ana" },
        "fromPriority": 0, "toPriority": 2 });
    let server = history_server_with(json!({
        "nodes": [entry.clone()], "pageInfo": { "hasNextPage": true }
    }))
    .await;
    let history = adapter(&server).issue_history("ENG-1").await.unwrap();
    assert!(
        history.more,
        "Linear holds older entries than the newest page"
    );
    assert_eq!(history.events.len(), 1, "the newest page is still shown");
    assert!(sent(&server, "IssueHistory")
        .query()
        .contains("toTitle updatedDescription } pageInfo { hasNextPage } }"));

    let server = history_server_with(json!({
        "nodes": [entry], "pageInfo": { "hasNextPage": false }
    }))
    .await;
    let history = adapter(&server).issue_history("ENG-1").await.unwrap();
    assert!(!history.more, "the whole history fit in one page");
    assert_eq!(history.events.len(), 1);
}

#[tokio::test]
async fn a_malformed_history_field_is_ignored_and_a_node_with_no_valid_change_is_skipped() {
    let server = history_server(json!([
        // Wrong-type state, and a state object with no string name.
        { "id": "m1", "createdAt": T0, "fromState": null, "toState": 42 },
        { "id": "m2", "createdAt": T0, "fromState": { "name": 7 }, "toState": { "type": "started" } },
        // Junk label entries are dropped; a node with only junk is skipped.
        { "id": "m3", "createdAt": T0, "addedLabels": [7, null, { "name": 3 }], "removedLabels": "Bug" },
        { "id": "m4", "createdAt": T0, "toPriority": "high", "fromTitle": 5, "toTitle": "x",
          "toAssignee": [1], "toProject": "Hollow" },
        // One bad field does not hide the valid one beside it.
        { "id": "m5", "createdAt": T1, "toState": 42, "fromProject": null,
          "toProject": { "name": "Hollow" },
          "addedLabels": [7, { "name": "Bug" }, { "name": 3 }] },
        // A valid node still maps.
        { "id": "ok", "createdAt": T1, "fromState": { "name": "Todo", "type": "unstarted" },
          "toState": { "name": "Done", "type": "completed" } }
    ]))
    .await;
    let events = adapter(&server)
        .issue_history("ENG-1")
        .await
        .unwrap()
        .events;
    let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["m5:labels", "m5:project", "ok:state"]);
    let labels = events.iter().find(|e| e.id == "m5:labels").unwrap();
    assert_eq!(labels.added, vec!["Bug".to_string()]);
    let state = events.iter().find(|e| e.id == "ok:state").unwrap();
    assert_eq!(
        (state.from.as_deref(), state.to.as_deref()),
        (Some("Todo"), Some("Done"))
    );
}

#[tokio::test]
async fn an_empty_history_is_empty_and_another_teams_issue_is_never_read() {
    let server = history_server(json!([])).await;
    let adapter = adapter(&server);
    assert_eq!(
        adapter.issue_history("i1").await.unwrap(),
        TrackerHistory::default()
    );
    assert_eq!(
        adapter.issue_history("x9").await,
        Err(TrackerError::NotFound),
        "another team's issue answers with nothing"
    );
}

#[tokio::test]
async fn a_history_read_that_fails_says_why_without_the_key() {
    let server = stub::serve(|req| match req.operation().as_deref() {
        Some("IssueHistory") => StubReply {
            status: 400,
            headers: Vec::new(),
            body: json!({ "errors": [{
                "message": format!("Cannot query field with {}", req.headers["authorization"]),
                "path": ["issues", "history"],
                "extensions": { "code": "GRAPHQL_VALIDATION_FAILED" }
            }] })
            .to_string(),
        },
        _ => StubReply::status(500),
    })
    .await;
    let error = adapter(&server).issue_history("ENG-1").await.unwrap_err();
    assert!(matches!(error, TrackerError::Provider(_)), "{error:?}");
    assert!(!error.to_string().contains(KEY));
    assert!(!format!("{error:?}").contains(KEY));

    // A reply without the history list is malformed, never an empty feed.
    let bare = stub::serve(|req| {
        let node = issue_node("i1", "ENG-1", TEAM, T0);
        match req.operation().as_deref() {
            Some("IssueHistory") => page("issues", vec![node]),
            _ => StubReply::status(500),
        }
    })
    .await;
    assert!(matches!(
        adapter(&bare).issue_history("ENG-1").await,
        Err(TrackerError::Provider(_))
    ));
}

#[tokio::test]
async fn a_create_naming_another_team_is_refused_before_any_request() {
    let server = stub::serve(|_| StubReply::data(json!({}))).await;
    let shared = adapter(&server);
    for team in [
        crate::tracker::linear::PERSONAL_TEAM_ID,
        "11111111-2222-3333-4444-555555555555",
        "LP",
    ] {
        let issue = NewIssue {
            title: "x".into(),
            team_id: Some(team.into()),
            ..Default::default()
        };
        assert_eq!(
            shared.create_issue(issue).await.unwrap_err(),
            TrackerError::OutOfScope,
            "{team}"
        );
        let project = NewProject {
            name: "x".into(),
            team_id: Some(team.into()),
            ..Default::default()
        };
        assert_eq!(
            shared.create_project(project).await.unwrap_err(),
            TrackerError::OutOfScope,
            "{team}"
        );
    }
    assert!(server.requests().is_empty(), "nothing was sent");
    // The scoped copy for the personal team names that team and no other.
    let own = shared
        .for_team(crate::tracker::linear::PERSONAL_TEAM_ID)
        .unwrap()
        .unwrap();
    assert!(shared.for_team(TEAM).is_none());
    assert!(shared
        .for_team("11111111-2222-3333-4444-555555555555")
        .unwrap()
        .is_err());
    drop(own);
}

// ---- R3: the organization pin on writes, and a lost create in a hidden team --

#[tokio::test]
async fn a_write_asks_for_the_organization_once_and_refuses_another() {
    let org = |id: &'static str| {
        stub::serve(move |req| match req.operation().as_deref() {
            Some("HollowOrgGuard") => {
                StubReply::data(json!({ "viewer": { "organization": { "id": id } } }))
            }
            Some("IssueCreate") => StubReply::data(
                json!({ "issueCreate": { "success": true, "issue": issue_node("i2", "ENG-2", TEAM, T1) } }),
            ),
            _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unexpected operation"),
        })
    };
    let other = org("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").await;
    let refused = LinearAdapter::with_url(&other.url, TEAM, KEY)
        .unwrap()
        .pin_org()
        .create_issue(NewIssue {
            title: "x".into(),
            ..NewIssue::default()
        })
        .await;
    assert!(
        matches!(refused, Err(TrackerError::NotConfigured(_))),
        "{refused:?}"
    );
    let ops: Vec<_> = other
        .requests()
        .iter()
        .filter_map(|r| r.operation())
        .collect();
    assert_eq!(ops, ["HollowOrgGuard"], "no write was sent");

    let ours = org(crate::tracker::graphql::ORG_ID).await;
    let a = LinearAdapter::with_url(&ours.url, TEAM, KEY)
        .unwrap()
        .pin_org();
    for _ in 0..2 {
        a.create_issue(NewIssue {
            title: "x".into(),
            ..NewIssue::default()
        })
        .await
        .unwrap();
    }
    let ops: Vec<_> = ours
        .requests()
        .iter()
        .filter_map(|r| r.operation())
        .collect();
    assert_eq!(
        ops,
        ["HollowOrgGuard", "IssueCreate", "IssueCreate"],
        "asked once"
    );
}

#[tokio::test]
async fn a_missing_create_is_absent_only_when_the_key_sees_other_teams() {
    let landing = |visible: Vec<&'static str>| {
        stub::serve(move |req| match req.operation().as_deref() {
            Some("IssueLanding") => StubReply::data(json!({ "issues": { "nodes": [] } })),
            Some("TeamsVisible") => StubReply::data(
                json!({ "teams": { "nodes": visible.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>() } }),
            ),
            _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unexpected operation"),
        })
    };
    // A key that also sees LP and PRO: an empty read-back means it never arrived.
    let open = landing(vec![TEAM, "lp-team-id", "pro-team-id"]).await;
    let found = adapter(&open).create_landing("c1").await;
    assert!(matches!(found, Ok(CreateLanding::Absent)), "{found:?}");
    // A key limited to AI and HOL can't tell a lost create from one moved to a
    // team it can't see: an error, so the journal stays `attempted`.
    let limited = landing(vec![TEAM, PERSONAL_TEAM_ID]).await;
    let found = adapter(&limited).create_landing("c1").await;
    assert!(matches!(found, Err(TrackerError::Provider(_))), "{found:?}");
}

// ---- T1: team routing and Hollow's labels ------------------------------------

#[test]
fn team_for_routes_tracked_work_to_ai_and_hollow_only_work_to_hol() {
    assert_eq!(team_for(Work::Tracked, None), Ok(TEAM_ID));
    assert_eq!(team_for(Work::HollowOnly, None), Ok(PERSONAL_TEAM_ID));
    assert_eq!(team_for(Work::Tracked, Some("  ")), Ok(TEAM_ID));
    // A named allowed team wins, by key (any case) or id.
    assert_eq!(team_for(Work::Tracked, Some("per")), Ok(PERSONAL_TEAM_ID));
    assert_eq!(team_for(Work::HollowOnly, Some("ENG")), Ok(TEAM_ID));
    assert_eq!(team_for(Work::HollowOnly, Some(TEAM_ID)), Ok(TEAM_ID));
    // A HOL-only label marks Hollow-only work; needs-review lives in both.
    assert_eq!(Work::of_labels(&["Commitment".into()]), Work::HollowOnly);
    assert_eq!(Work::of_labels(&["hollow-lesson".into()]), Work::HollowOnly);
    assert_eq!(Work::of_labels(&["agent".into()]), Work::HollowOnly);
    assert_eq!(Work::of_labels(&["needs-review".into()]), Work::Tracked);
    assert_eq!(Work::of_labels(&[]), Work::Tracked);
}

#[test]
fn team_for_refuses_lp_pro_and_every_other_team() {
    for work in [Work::Tracked, Work::HollowOnly] {
        for named in [
            "LP",
            "lp",
            "PRO",
            "pro",
            OTHER,
            "team-lp-0001-fixture",
            "Engineering",
        ] {
            let error = team_for(work, Some(named)).unwrap_err();
            assert!(
                error.contains("Hollow works only with") && error.contains("Nothing was sent"),
                "{named}: {error}"
            );
        }
    }
}

#[test]
fn labels_for_each_team_follow_d8() {
    assert_eq!(labels_for(TEAM_ID), ["needs-review", "commitment"]);
    assert_eq!(
        labels_for(PERSONAL_TEAM_ID),
        ["needs-review", "commitment", "agent", "hollow-lesson"]
    );
    assert!(labels_for(OTHER).is_empty());
    // Only a team commitment (T3e) is a HOL-default label AI keeps too.
    for own in HOLLOW_ONLY_LABELS {
        assert!(
            HOL_LABELS.contains(own) && (*own == "commitment") == AI_LABELS.contains(own),
            "{own}"
        );
    }
}

#[tokio::test]
async fn a_label_is_created_in_the_scoped_team_and_a_reply_from_another_team_fails() {
    let server = stub::serve(|req| match req.operation().as_deref() {
        Some("LabelCreate") => {
            let input = &req.variables()["input"];
            let team = if input["name"] == "elsewhere" {
                OTHER
            } else {
                input["teamId"].as_str().unwrap()
            };
            StubReply::data(json!({"issueLabelCreate":{"success":true,"issueLabel":{
                "id":"l-new","name":input["name"],"color":null,"team":{"id":team},"parent":null}}}))
        }
        _ => StubReply::status(500),
    })
    .await;
    let hol = adapter(&server).scoped_to(PERSONAL_TEAM_ID).unwrap();
    let made = hol.create_label("commitment").await.unwrap();
    assert_eq!(
        (made.id.as_str(), made.name.as_str()),
        ("l-new", "commitment")
    );
    let create = sent(&server, "LabelCreate");
    assert_eq!(create.variables()["input"]["teamId"], PERSONAL_TEAM_ID);
    assert!(hol.create_label("elsewhere").await.is_err());
    assert!(hol.create_label(" ").await.is_err());
}
