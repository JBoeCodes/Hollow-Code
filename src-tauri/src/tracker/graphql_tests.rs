//! `tracker.graphql`: the parser, the org guard, the team guard, the delete
//! card and key redaction, against `tracker::stub`.

use super::*;
use crate::app_actions::{self, Args};
use crate::audit::{self, kind, AuditFilter};
use crate::tracker::stub::{self, StubReply, StubServer};

const KEY: &str = "lin_api_stubkey_never_logged_0123456789";
const OTHER_TEAM: &str = "11111111-2222-3333-4444-555555555555";
const PRO_TEAM: &str = "99999999-8888-7777-6666-555555555555";

fn adapter(server: &StubServer) -> LinearAdapter {
    LinearAdapter::with_url(&server.url, TEAM_ID, KEY).unwrap()
}

/// Another organization's id, for a key that isn't Acme's.
const OTHER_ORG: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

/// The org guard's reply for an organization with this id, urlKey and name.
fn org_as(id: &str, url_key: &str, name: &str) -> StubReply {
    StubReply::data(json!({
        "viewer": { "organization": { "id": id, "urlKey": url_key, "name": name } }
    }))
}

/// The org guard's reply: Acme's own id and urlKey for "Acme
/// Mobility", another organization's for any other name.
fn org(name: &str) -> StubReply {
    if name == ORG_NAME {
        org_as(ORG_ID, "example-org", name)
    } else {
        org_as(OTHER_ORG, "some-other-company", name)
    }
}

/// Answers the org guard with `name` and everything else with `data`.
async fn server(name: &'static str, data: Value) -> StubServer {
    server_as(org(name), data).await
}

/// Answers the org guard with `guard` and everything else with `data`.
async fn server_as(guard: StubReply, data: Value) -> StubServer {
    stub::serve(move |req| {
        if req.operation().as_deref() == Some("HollowOrgGuard") {
            guard.clone()
        } else {
            StubReply::data(data.clone())
        }
    })
    .await
}

fn vault() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn rows(vault: &Path) -> Vec<audit::AuditEntry> {
    audit::query(
        vault,
        &AuditFilter {
            limit: Some(audit::MAX_LIMIT),
            ..AuditFilter::default()
        },
    )
    .unwrap()
    .into_iter()
    .filter(|r| r.action.as_deref() == Some("graphql"))
    .collect()
}

// A call with no team to guard and no by-id target: it reaches the org guard
// and the send. (R3: `projectStatusCreate` is refused now, so it is an issue.)
const PAUSED: &str =
    "mutation CreatePaused($input: IssueCreateInput!) { issueCreate(input: $input) { success } }";

// ---- Parser ------------------------------------------------------------------

#[test]
fn parse_reads_names_fields_aliases_comments_and_strings() {
    let op = parse("# note\nquery Q($a: String = \"}\") @x { me: viewer { id } teams(filter: {name: {eq: \"{\"}}) { nodes { id } } }").unwrap();
    assert_eq!(op.kind, OpKind::Query);
    assert_eq!(op.name.as_deref(), Some("Q"));
    let names: Vec<_> = op.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["viewer", "teams"]);
    let anon = parse("{ viewer { id } }").unwrap();
    assert_eq!(anon.label(), "viewer");
}

#[test]
fn parse_refuses_what_it_cannot_read_one_operation_or_a_subscription() {
    assert!(parse("query A { viewer { id } } query B { viewer { id } }").is_err());
    assert!(parse("").is_err());
    assert!(parse("query { viewer { id }").is_err());
    assert!(parse("query { a(b: \"x) { id } }").is_err());
    let sub = preflight("subscription S { issueCreated { id } }", &Value::Null).unwrap_err();
    assert!(sub.contains("subscriptions"), "{sub}");
    let spread = preflight("mutation M { ...F } fragment F on Mutation { issueCreate(input: {teamId: \"x\"}) { success } }", &Value::Null)
        .unwrap_err();
    assert!(spread.contains("fragment"), "{spread}");
    // Fragments in a query are fine.
    assert!(preflight(
        "query { viewer { ...F } } fragment F on User { id }",
        &Value::Null
    )
    .is_ok());
}

// ---- The delete card ---------------------------------------------------------

#[test]
fn delete_archive_and_remove_mutations_ask_in_every_mode() {
    for q in [
        "mutation { issueDelete(id: \"x\") { success } }",
        "mutation { a: projectStatusArchive(id: \"x\") { success } }",
        "mutation RemoveThing { issueUpdate(id: \"x\", input: {}) { success } }",
        "mutation { issueRelationRemove: issueLabelRemove(id: \"x\") { success } }",
        "mutation { issueUpdate(id: \"x\", input: {trashed: true}) { success } }",
        "mutation { userSuspend(id: \"x\") { success } }",
        "mutation { webhookRotateSecret(id: \"x\") { success } }",
        "mutation { issueLabelRetire(id: \"x\") { success } }",
    ] {
        assert!(asks_always(q, &Value::Null), "{q}");
    }
    for q in [
        PAUSED,
        "query DeleteEverything { issues { nodes { id } } }",
        "mutation { issueUpdate(id: \"x\", input: {title: \"delete archive remove\"}) { success } }",
        "mutation { commentCreate(input: {body: \"x\"}) { success } }",
    ] {
        assert!(!asks_always(q, &Value::Null), "{q}");
    }
    // A flag in the variables counts too.
    let update =
        "mutation($i: IssueUpdateInput!) { issueUpdate(id: \"x\", input: $i) { success } }";
    assert!(asks_always(update, &json!({ "i": { "trashed": true } })));
    assert!(!asks_always(update, &json!({ "i": { "title": "x" } })));
    let action = app_actions::find("tracker.graphql").unwrap();
    let call = |q: &str| Args(serde_json::from_value(json!({ "query": q })).unwrap());
    let delete = call("mutation { issueDelete(id: \"x\") { success } }");
    let create = call(PAUSED);
    for mode in [
        "bypassPermissions",
        "auto",
        "acceptEdits",
        "default",
        "plan",
    ] {
        assert!(
            app_actions::asks_in_mode(action, &delete, mode),
            "delete in {mode}"
        );
    }
    assert!(!app_actions::asks_in_mode(
        action,
        &create,
        "bypassPermissions"
    ));
    // T1: a Write action now; a call that isn't destructive or
    // access-granting asks in no mode (its card builder returns none).
    assert_eq!(action.tier, app_actions::Tier::Write);
    assert!(!app_actions::asks_in_mode(action, &create, "auto"));
    let created = preflight(PAUSED, &Value::Null).unwrap();
    assert!(!created.destructive && !created.access);
    assert!(
        preflight(
            "mutation { issueDelete(id: \"x\") { success } }",
            &Value::Null
        )
        .unwrap()
        .destructive
    );
}

// ---- The team guard ----------------------------------------------------------

#[test]
fn a_mutation_naming_another_team_is_refused_everywhere_it_can_hide() {
    let create = |input: &str| format!("mutation {{ issueCreate(input: {input}) {{ success }} }}");
    // Inline literal.
    assert!(preflight(
        &create(&format!("{{teamId: \"{OTHER_TEAM}\", title: \"x\"}}")),
        &Value::Null
    )
    .is_err());
    assert!(preflight(&create("{teamId: \"LP\"}"), &Value::Null).is_err());
    assert!(preflight(&create("{teamId: 7}"), &Value::Null).is_err());
    // A variable named anything, referenced by the field.
    let q = "mutation($t: String!) { issueCreate(input: {teamId: $t}) { success } }";
    assert!(preflight(q, &json!({ "t": OTHER_TEAM })).is_err());
    assert!(preflight(q, &json!({ "t": TEAM_ID })).is_ok());
    assert!(preflight(q, &json!({ "t": "eng" })).is_ok());
    // A whole-input variable, a list, a nested object, a `...TeamId` key.
    let whole = "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }";
    assert!(preflight(whole, &json!({ "i": { "teamId": OTHER_TEAM } })).is_err());
    assert!(preflight(whole, &json!({ "i": { "title": "x", "teamId": TEAM_ID } })).is_ok());
    let project = "mutation($i: ProjectCreateInput!) { projectCreate(input: $i) { success } }";
    assert!(preflight(
        project,
        &json!({ "i": { "teamIds": [TEAM_ID, OTHER_TEAM] } })
    )
    .is_err());
    assert!(preflight(project, &json!({ "i": { "teamIds": [TEAM_ID] } })).is_ok());
    assert!(preflight(
        whole,
        &json!({ "i": { "x": { "targetTeamId": OTHER_TEAM } } })
    )
    .is_err());
    assert!(preflight(&create("{teamIds: [\"a\"]}"), &Value::Null).is_err());
    assert!(preflight(
        &create(&format!("{{team: {{id: \"{OTHER_TEAM}\"}}}}")),
        &Value::Null
    )
    .is_err());
    // No team named and nothing to guard: it goes through.
    assert!(preflight(
        PAUSED,
        &json!({ "input": { "title": "Paused", "priority": 1 } })
    )
    .is_ok());
}

#[test]
fn team_level_mutations_need_the_ai_teams_id() {
    assert!(preflight(
        "mutation { teamCreate(input: {name: \"x\"}) { success } }",
        &Value::Null
    )
    .is_err());
    let update = |id: &str| {
        format!("mutation {{ teamUpdate(id: \"{id}\", input: {{name: \"x\"}}) {{ success }} }}")
    };
    assert!(preflight(&update(OTHER_TEAM), &Value::Null).is_err());
    assert!(preflight(&update(TEAM_ID), &Value::Null).is_ok());
    assert!(preflight("mutation { teamDelete { success } }", &Value::Null).is_err());
    let var = "mutation($id: String!) { teamUnarchive(id: $id) { success } }";
    assert!(preflight(var, &json!({ "id": OTHER_TEAM })).is_err());
    assert!(preflight(var, &json!({ "id": TEAM_ID })).is_ok());
    // A read names AI or HOL only (R3); the allowed ones go through.
    assert!(preflight(
        &format!("query {{ team(id: \"{OTHER_TEAM}\") {{ name }} }}"),
        &Value::Null
    )
    .is_err());
    assert!(preflight(
        &format!("query {{ team(id: \"{TEAM_ID}\") {{ name }} }}"),
        &Value::Null
    )
    .is_ok());
}

// ---- The org guard -----------------------------------------------------------

#[tokio::test]
async fn another_organization_is_refused_before_the_request() {
    let server = server(
        "Some Other Company",
        json!({ "projectStatusCreate": { "success": true } }),
    )
    .await;
    let dir = vault();
    let error = call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        PAUSED,
        Some(json!({ "input": {} })),
    )
    .await
    .unwrap_err();
    assert!(error.contains("Example Org"), "{error}");
    let ops: Vec<_> = server
        .requests()
        .iter()
        .map(|r| r.operation().unwrap_or_default())
        .collect();
    assert_eq!(ops, ["HollowOrgGuard"], "only the guard was sent");
    let logged = rows(dir.path());
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].kind, kind::TRACKER_WRITE_FAILED);
}

/// #1301: the name is not the check. A key whose organization calls itself
/// Example Org under another id is refused, and so is one with
/// Acme's urlKey but another id; the guard asks for all three fields.
#[tokio::test]
async fn an_organization_named_acme_with_another_id_is_refused() {
    for guard in [
        org_as(OTHER_ORG, "acme-impostor", ORG_NAME),
        org_as(OTHER_ORG, "example-org", ORG_NAME),
        StubReply::data(json!({ "viewer": { "organization": { "name": ORG_NAME } } })),
    ] {
        let server = server_as(guard, json!({ "projectStatusCreate": { "success": true } })).await;
        let dir = vault();
        let error = call(
            &adapter(&server),
            dir.path(),
            "person".into(),
            PAUSED,
            Some(json!({ "input": {} })),
        )
        .await
        .unwrap_err();
        assert!(error.contains(ORG_ID), "{error}");
        assert!(error.contains("Nothing was sent"), "{error}");
        let seen = server.requests();
        assert_eq!(seen.len(), 1, "only the guard was sent");
        assert_eq!(seen[0].operation().as_deref(), Some("HollowOrgGuard"));
        let asked = seen[0].query();
        assert!(asked.contains("organization { id urlKey name }"), "{asked}");
    }
}

#[tokio::test]
async fn the_right_organization_runs_the_guard_first_then_the_request() {
    let server = server(
        "Example Org",
        json!({ "projectStatusCreate": { "success": true } }),
    )
    .await;
    let dir = vault();
    let reply = call(
        &adapter(&server),
        dir.path(),
        "session-1".into(),
        PAUSED,
        Some(json!({ "input": { "name": "Paused", "type": "paused" } })),
    )
    .await
    .unwrap();
    assert_eq!(reply["data"]["projectStatusCreate"]["success"], true);
    let seen = server.requests();
    let ops: Vec<_> = seen
        .iter()
        .map(|r| r.operation().unwrap_or_default())
        .collect();
    assert_eq!(ops, ["HollowOrgGuard", "CreatePaused"]);
    assert_eq!(seen[1].variables()["input"]["type"], "paused");
    assert_eq!(
        seen[1].headers.get("authorization").map(String::as_str),
        Some(KEY)
    );
    // One audit row, with the operation and the variables.
    let logged = rows(dir.path());
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].kind, kind::TRACKER_WRITE);
    assert_eq!(logged[0].asked_by.as_deref(), Some("session-1"));
    let after = &logged[0].fields.as_ref().unwrap()["after"];
    assert_eq!(after["operation"], "CreatePaused");
    assert_eq!(after["variables"]["input"]["name"], "Paused");
}

#[tokio::test]
async fn a_team_refusal_sends_nothing_and_leaves_a_failed_row() {
    let server = server("Example Org", json!({})).await;
    let dir = vault();
    let q = "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }";
    let error = call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        q,
        Some(json!({ "i": { "teamId": OTHER_TEAM } })),
    )
    .await
    .unwrap_err();
    assert!(error.contains("Engineering"), "{error}");
    assert!(server.requests().is_empty());
    assert_eq!(rows(dir.path())[0].kind, kind::TRACKER_WRITE_FAILED);
}

// ---- Redaction ---------------------------------------------------------------

#[tokio::test]
async fn the_key_never_reaches_the_result_or_the_audit() {
    let data = json!({
        "echo": format!("the key is {KEY}"),
        "webhook": { "url": "https://x.test", "secret": "whsec_live_abcdef", "signingSecret": "s3cr3t" },
        "token": "abc",
        "other": "lin_oauth_ABCDEFGHIJKL",
    });
    let server = stub::serve(move |req| match req.operation().as_deref() {
        Some("HollowOrgGuard") => org("Example Org"),
        Some("Fails") => StubReply::error("INVALID_INPUT", &format!("bad {KEY} here")),
        _ => StubReply::data(data.clone()),
    })
    .await;
    let dir = vault();
    let vars = json!({ "apiKey": "hunter2", "note": format!("see {KEY}"), "name": "ok" });
    let reply = call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        "query Reads($apiKey: String) { webhooks { nodes { url } } }",
        Some(vars.clone()),
    )
    .await
    .unwrap();
    let text = reply.to_string();
    assert!(
        !text.contains(KEY)
            && !text.contains("whsec_live")
            && !text.contains("s3cr3t")
            && !text.contains("lin_oauth"),
        "{text}"
    );
    assert!(text.contains("https://x.test"));
    let failing = call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        "query Fails { viewer { id } }",
        None,
    )
    .await
    .unwrap();
    assert!(!failing.to_string().contains(KEY), "{failing}");
    assert_eq!(failing["errors"][0]["code"], "INVALID_INPUT");
    let stored = serde_json::to_string(&rows(dir.path())).unwrap();
    assert!(
        !stored.contains(KEY) && !stored.contains("hunter2"),
        "{stored}"
    );
    assert!(stored.contains("Reads"));
    assert_eq!(rows(dir.path()).len(), 2);
    assert_eq!(
        rows(dir.path())[0].kind,
        kind::TRACKER_WRITE_FAILED,
        "a GraphQL error is a failed row"
    );
}

// ---- Rate limit --------------------------------------------------------------

#[tokio::test]
async fn a_rate_limit_closes_the_window_and_the_next_call_sends_nothing() {
    let server = stub::serve(|_| StubReply::status(429)).await;
    let dir = vault();
    let a = adapter(&server);
    let first = call(&a, dir.path(), "person".into(), "{ viewer { id } }", None)
        .await
        .unwrap_err();
    assert!(first.contains("slow down"), "{first}");
    let sent = server.requests().len();
    let second = call(&a, dir.path(), "person".into(), "{ viewer { id } }", None)
        .await
        .unwrap_err();
    assert!(
        second.contains("rate limit is spent") || second.contains("rate"),
        "{second}"
    );
    assert_eq!(
        server.requests().len(),
        sent,
        "inside the window nothing is sent"
    );
}

#[tokio::test]
async fn no_requests_left_closes_the_window_after_a_good_reply() {
    let reset = (Utc::now() + Duration::minutes(5)).timestamp_millis() as u64;
    let server = stub::serve(move |req| {
        if req.operation().as_deref() == Some("HollowOrgGuard") {
            org("Example Org")
        } else {
            StubReply::data(json!({ "viewer": { "id": "u1" } })).with_rate(0, 5000, reset)
        }
    })
    .await;
    let dir = vault();
    let a = adapter(&server);
    assert!(
        call(&a, dir.path(), "person".into(), "{ viewer { id } }", None)
            .await
            .is_ok()
    );
    let sent = server.requests().len();
    let error = call(&a, dir.path(), "person".into(), "{ viewer { id } }", None)
        .await
        .unwrap_err();
    assert!(error.contains("rate limit is spent"), "{error}");
    assert_eq!(server.requests().len(), sent);
}

#[test]
fn a_variable_default_cannot_carry_another_team() {
    let q = "mutation($input: IssueCreateInput = {teamId: \"11111111-2222-3333-4444-555555555555\", title: \"x\"}) { issueCreate(input: $input) { success } }";
    let error = preflight(q, &Value::Null).unwrap_err();
    assert!(error.contains("defaults"), "{error}");
    let q = "mutation($t: String! = \"LP\") { issueCreate(input: {teamId: $t}) { success } }";
    assert!(preflight(q, &Value::Null).is_err());
    // A query's defaults are harmless, and a default is fine when no mutation has it.
    assert!(preflight(
        "query($n: Int = 5) { issues(first: $n) { nodes { id } } }",
        &Value::Null
    )
    .is_ok());
    // A team object with no id or key can't be checked.
    let whole = "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }";
    assert!(preflight(whole, &json!({ "i": { "team": { "name": "Other" } } })).is_err());
}

#[tokio::test]
async fn a_secret_named_object_is_masked_all_the_way_down() {
    let server = server(
        "Example Org",
        json!({ "hook": { "secret": { "value": "abc123" } } }),
    )
    .await;
    let dir = vault();
    let reply = call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        "query Q($t: JSON) { webhooks { nodes { id } } }",
        Some(json!({ "t": { "token": { "value": "zzz999" } } })),
    )
    .await
    .unwrap();
    assert!(!reply.to_string().contains("abc123"), "{reply}");
    assert!(!serde_json::to_string(&rows(dir.path()))
        .unwrap()
        .contains("zzz999"));
}

// ---- By-id mutations check their team (2026-11-12) ---------------------------

/// Answers the org guard, the team guard (`AI-…` in the AI team, `LP-…` in LP,
/// anything else not found) and everything else with `data`.
async fn team_server(data: Value) -> StubServer {
    stub::serve(move |req| match req.operation().as_deref() {
        Some("HollowOrgGuard") => org("Example Org"),
        Some("HollowTeamGuard") => {
            let mut out = Map::new();
            let ai = json!({ "id": TEAM_ID, "key": "ENG" });
            let lp = json!({ "id": OTHER_TEAM, "key": "LP" });
            for (alias, id) in req.variables().as_object().unwrap() {
                let id = id.as_str().unwrap();
                // `SPAN-` is a project in AI and LP; `WS-` exists in no team
                // (a workspace label).
                let teams = if id.starts_with("ENG-") {
                    vec![ai.clone()]
                } else if id.starts_with("PER-") {
                    vec![json!({ "id": PERSONAL_TEAM_ID, "key": "PER" })]
                } else if id.starts_with("LP-") {
                    vec![lp.clone()]
                } else if id.starts_with("PRO-") {
                    vec![json!({ "id": PRO_TEAM, "key": "PRO" })]
                } else if id.starts_with("SPAN-") {
                    vec![ai.clone(), lp.clone()]
                } else if id.starts_with("WS-") {
                    Vec::new()
                } else {
                    out.insert(alias.clone(), Value::Null);
                    continue;
                };
                let is = |root: &str| req.query().contains(&format!("{alias}: {root}("));
                let team = teams.first().cloned().unwrap_or(Value::Null);
                let many = json!({ "nodes": teams, "pageInfo": { "hasNextPage": false } });
                let node = if is("comment") || is("attachment") {
                    json!({ "issue": { "team": team } })
                } else if is("project") {
                    json!({ "teams": many })
                } else if is("projectMilestone") {
                    json!({ "project": { "teams": many } })
                } else {
                    json!({ "team": team })
                };
                out.insert(alias.clone(), node);
            }
            StubReply::data(Value::Object(out))
        }
        _ => StubReply::data(data.clone()),
    })
    .await
}

fn ops(server: &StubServer) -> Vec<String> {
    server
        .requests()
        .iter()
        .map(|r| r.operation().unwrap_or_default())
        .collect()
}

#[tokio::test]
async fn an_update_on_another_teams_issue_is_refused_after_reading_its_team() {
    let server = team_server(json!({ "issueUpdate": { "success": true } })).await;
    let dir = vault();
    let q = format!(
        "mutation Pull {{ issueUpdate(id: \"LP-12\", input: {{teamId: \"{TEAM_ID}\"}}) {{ success }} }}"
    );
    let error = call(&adapter(&server), dir.path(), "person".into(), &q, None)
        .await
        .unwrap_err();
    assert!(
        error.contains("team LP") && error.contains("Engineering"),
        "{error}"
    );
    assert_eq!(ops(&server), ["HollowOrgGuard", "HollowTeamGuard"]);
    assert_eq!(server.requests()[1].variables()["t0"], "LP-12");
    let logged = rows(dir.path());
    assert_eq!(logged[0].kind, kind::TRACKER_WRITE_FAILED);
    // The audit row has the inline arguments the variables don't.
    let after = &logged[0].fields.as_ref().unwrap()["after"];
    assert!(
        after["query"].as_str().unwrap().contains("LP-12"),
        "{after}"
    );
}

#[tokio::test]
async fn an_update_on_an_ai_issue_goes_through() {
    let server = team_server(json!({ "issueUpdate": { "success": true } })).await;
    let dir = vault();
    let q = "mutation Rename($id: String!, $i: IssueUpdateInput!) { issueUpdate(id: $id, input: $i) { success } }";
    let reply = call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        q,
        Some(json!({ "id": "ENG-5", "i": { "title": "x" } })),
    )
    .await
    .unwrap();
    assert_eq!(reply["data"]["issueUpdate"]["success"], true);
    assert_eq!(
        ops(&server),
        ["HollowOrgGuard", "HollowTeamGuard", "Rename"]
    );
    assert!(
        !asks_always(q, &json!({ "id": "ENG-5" })),
        "a resolvable target doesn't ask"
    );
}

#[tokio::test]
async fn comments_and_attachments_on_another_teams_issue_are_refused() {
    let server = team_server(json!({})).await;
    let dir = vault();
    let a = adapter(&server);
    // A comment on an LP issue, the issueId inside a whole-input variable.
    let create = "mutation($i: CommentCreateInput!) { commentCreate(input: $i) { success } }";
    let error = call(
        &a,
        dir.path(),
        "person".into(),
        create,
        Some(json!({ "i": { "issueId": "LP-3", "body": "x" } })),
    )
    .await
    .unwrap_err();
    assert!(error.contains("issue \"LP-3\""), "{error}");
    // A comment edited by id: read through its issue.
    let edit = "mutation { commentUpdate(id: \"LP-c1\", input: {body: \"x\"}) { success } }";
    let error = call(&a, dir.path(), "person".into(), edit, None)
        .await
        .unwrap_err();
    assert!(
        error.contains("comment") && error.contains("team LP"),
        "{error}"
    );
    // An attachment linked to an LP issue by a top-level issueId.
    let link =
        "mutation { attachmentLinkURL(issueId: \"LP-9\", url: \"https://x.test\") { success } }";
    assert!(call(&a, dir.path(), "person".into(), link, None)
        .await
        .is_err());
    // A relation from an AI issue to an LP one, and a list in a batch.
    let rel = "mutation { issueRelationCreate(input: {issueId: \"ENG-1\", relatedIssueId: \"LP-2\", type: related}) { success } }";
    assert!(call(&a, dir.path(), "person".into(), rel, None)
        .await
        .is_err());
    let batch = "mutation($ids: [UUID!]!) { issueBatchUpdate(ids: $ids, input: {}) { success } }";
    assert!(call(
        &a,
        dir.path(),
        "person".into(),
        batch,
        Some(json!({ "ids": ["ENG-1", "LP-4"] }))
    )
    .await
    .is_err());
    // A state of another team.
    let state = "mutation { workflowStateUpdate(id: \"LP-s\", input: {name: \"x\"}) { success } }";
    assert!(call(&a, dir.path(), "person".into(), state, None)
        .await
        .is_err());
    // Not found: refused, not sent.
    let gone = "mutation { issueUpdate(id: \"nope\", input: {title: \"x\"}) { success } }";
    let error = call(&a, dir.path(), "person".into(), gone, None)
        .await
        .unwrap_err();
    assert!(error.contains("couldn't confirm"), "{error}");
    // None of them reached Linear.
    assert!(ops(&server)
        .iter()
        .all(|o| o == "HollowOrgGuard" || o == "HollowTeamGuard"));
    // The same on AI goes through.
    assert!(call(
        &a,
        dir.path(),
        "person".into(),
        create,
        Some(json!({ "i": { "issueId": "ENG-3", "body": "x" } })),
    )
    .await
    .is_ok());
}

#[test]
fn targets_read_ids_inline_from_variables_and_lists() {
    let found = |q: &str, v: Value| preflight(q, &v).unwrap().targets;
    let issue = |id: &str| Target {
        kind: Entity::Issue,
        id: id.into(),
    };
    assert_eq!(
        found(
            "mutation { issueAddLabel(id: \"ENG-1\", labelId: \"l\") { success } }",
            Value::Null
        ),
        [issue("ENG-1")]
    );
    assert_eq!(
        found(
            "mutation($p: String) { issueCreate(input: {teamId: \"ENG\", parentId: $p}) { success } }",
            json!({ "p": "LP-7" })
        ),
        [issue("LP-7")]
    );
    // A label's parentId is a label group, read as a label.
    assert_eq!(
        found(
            "mutation { issueLabelCreate(input: {teamId: \"ENG\", parentId: \"g\"}) { success } }",
            Value::Null
        ),
        [Target {
            kind: Entity::Label,
            id: "g".into()
        }]
    );
    // A create's own nested id names nothing Hollow can read: refused.
    assert!(preflight(
        "mutation { issueCreate(input: {id: \"new\", teamId: \"ENG\"}) { success } }",
        &Value::Null
    )
    .is_err());
    // A read by id names no team, so it isn't team-guarded (a recorded gap).
    assert!(found("query { issue(id: \"LP-1\") { id } }", Value::Null).is_empty());
}

#[test]
fn projects_labels_and_cycles_by_id_are_read_and_the_rest_refused() {
    let action = app_actions::find("tracker.graphql").unwrap();
    let args = |q: &str| Args(serde_json::from_value(json!({ "query": q })).unwrap());
    let target = |kind, id: &str| Target {
        kind,
        id: id.into(),
    };
    for (q, want) in [
        (
            "mutation { projectUpdate(id: \"p1\", input: {teamIds: [\"ENG\"]}) { success } }",
            target(Entity::Project, "p1"),
        ),
        (
            "mutation { issueLabelUpdate(id: \"l1\", input: {name: \"x\"}) { success } }",
            target(Entity::Label, "l1"),
        ),
        (
            "mutation { cycleUpdate(id: \"c1\", input: {name: \"x\"}) { success } }",
            target(Entity::Cycle, "c1"),
        ),
        (
            "mutation { projectMilestoneUpdate(id: \"m1\", input: {name: \"x\"}) { success } }",
            target(Entity::Milestone, "m1"),
        ),
        (
            "mutation { projectUpdateCreate(input: {projectId: \"p2\", body: \"x\"}) { success } }",
            target(Entity::Project, "p2"),
        ),
    ] {
        assert_eq!(preflight(q, &Value::Null).unwrap().targets, [want], "{q}");
        // The read decides, not a card.
        assert!(!asks_always(q, &Value::Null), "{q}");
        assert!(
            !app_actions::asks_in_mode(action, &args(q), "bypassPermissions"),
            "{q}"
        );
    }
    // What Hollow can't read the team of is refused before any card.
    for q in [
        "mutation($id: String!) { documentUpdate(id: $id, input: {title: \"x\"}) { success } }",
        "mutation { issueUpdate(id: {x: 1}, input: {}) { success } }",
        "mutation { customerUpdate(id: \"c\", input: {name: \"x\"}) { success } }",
    ] {
        let error = preflight(q, &json!({ "id": "d1" })).unwrap_err();
        assert!(
            error.contains("can't read which team") && error.contains("Nothing was sent"),
            "{q}: {error}"
        );
    }
    // People need no team read; a project's lead and members don't refuse it.
    let create = "mutation { projectCreate(input: {name: \"x\", teamIds: [\"ENG\"], leadId: \"u\", memberIds: [\"v\"]}) { success } }";
    assert!(preflight(create, &Value::Null).unwrap().targets.is_empty());
}

// ---- Access and outside-the-org mutations ask --------------------------------

#[test]
fn access_and_webhook_mutations_ask_in_every_mode() {
    let action = app_actions::find("tracker.graphql").unwrap();
    let args = |q: &str| Args(serde_json::from_value(json!({ "query": q })).unwrap());
    for q in [
        "mutation { organizationInviteCreate(input: {email: \"a@b.c\"}) { success } }",
        "mutation { userPromoteAdmin(id: \"u\") { success } }",
        "mutation { userUpdate(id: \"u\", input: {}) { success } }",
        "mutation { organizationUpdate(input: {name: \"x\"}) { success } }",
        "mutation { w: webhookCreate(input: {url: \"https://x.test\", allPublicTeams: true}) { success } }",
        "mutation { apiKeyCreate(input: {label: \"x\"}) { success } }",
        "mutation { oauthClientCreate(input: {name: \"x\"}) { success } }",
        "mutation { integrationGithubConnect(code: \"x\") { success } }",
        "mutation { airbyteIntegrationConnect(input: {}) { success } }",
        "mutation { teamMembershipCreate(input: {teamId: \"ENG\", userId: \"u\"}) { success } }",
    ] {
        assert!(asks_always(q, &Value::Null), "{q}");
        assert!(app_actions::asks_in_mode(action, &args(q), "bypassPermissions"), "{q}");
        if let Ok(summary) = preflight(q, &Value::Null) {
            assert!(summary.access, "{q}");
        }
    }
    // A read of the same names is a read.
    assert!(!asks_always(
        "query { organization { name } users { nodes { name } } }",
        &Value::Null
    ));
    assert!(!asks_always(PAUSED, &Value::Null));
}

// ---- Secret-named fields --------------------------------------------------------

#[test]
fn selecting_a_secret_named_field_is_refused_whatever_its_alias() {
    for q in [
        "{ webhooks { nodes { s: secret } } }",
        "{ webhooks { nodes { url secret } } }",
        "query { viewer { ...F } } fragment F on User { x: apiToken }",
        "{ x { ... on Thing { signingKey } } }",
        "mutation { webhookUpdate(id: \"w\", input: {}) { webhook { harmless: secret } } }",
        "{ oauthClients { nodes { clientSecret } } }",
        "{ thing { passphrase } }",
    ] {
        let error = preflight(q, &Value::Null).unwrap_err();
        assert!(error.contains("secret-named"), "{q}: {error}");
    }
    for q in [
        "{ teams { nodes { id key name } } }",
        "{ organization { name urlKey } }",
        "query Q($token: String) { issues(filter: {title: {eq: \"secret\"}}) { nodes { secretive: id } } }",
        "{ issues { nodes { team { key } } } }",
        "query { viewer { ...F @include(if: true) } } fragment F on User { id }",
    ] {
        assert!(preflight(q, &Value::Null).is_ok(), "{q}");
    }
}

#[test]
fn secret_names_cover_keys_and_pass_words() {
    for name in [
        "secret",
        "signingSecret",
        "accessToken",
        "password",
        "apiKey",
        "api_key",
        "privateKey",
        "signingKey",
        "webhookKeys",
        "pass",
        "passphrase",
        "passkey",
        "smtp_pass",
        "credentials",
        "Authorization",
    ] {
        assert!(secret_name(name), "{name}");
    }
    for name in [
        "key",
        "keys",
        "urlKey",
        "teamKey",
        "teamKeys",
        "id",
        "name",
        "bypassTriage",
        "url",
    ] {
        assert!(!secret_name(name), "{name}");
    }
}

#[tokio::test]
async fn the_audit_row_keeps_the_query_with_secret_literals_masked_and_capped() {
    let server = team_server(json!({ "issueCreate": { "success": true } })).await;
    let dir = vault();
    let long = "y".repeat(5_000);
    let q = format!(
        "mutation Make {{ issueCreate(input: {{teamId: \"ENG\", description: \"see {KEY}\", privateKey: \"pk_live_zzz\", password: \"\"\"hunter2\"\"\", title: \"{long}\"}}) {{ success }} }}"
    );
    call(&adapter(&server), dir.path(), "person".into(), &q, None)
        .await
        .unwrap();
    let logged = rows(dir.path());
    let stored = logged[0].fields.as_ref().unwrap()["after"]["query"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        stored.contains("mutation Make") && stored.contains("teamId: \"ENG\""),
        "{stored}"
    );
    assert!(
        !stored.contains(KEY) && !stored.contains("pk_live_zzz") && !stored.contains("hunter2"),
        "{stored}"
    );
    // The adapter masks the literal ("••••"); since R5b the redactor also
    // knows a `privateKey` name and may write its own mask over it.
    assert!(
        stored.contains("privateKey: \"••••\"") || stored.contains("privateKey: \"[redacted]\""),
        "{stored}"
    );
    assert!(
        stored.chars().count() <= AUDIT_QUERY + 1,
        "{}",
        stored.len()
    );
    // A refused call keeps its query too.
    let refused = "mutation { issueCreate(input: {teamId: \"LP\"}) { success } }";
    assert!(call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        refused,
        None
    )
    .await
    .is_err());
    let logged = rows(dir.path());
    let after = &logged[0].fields.as_ref().unwrap()["after"];
    assert!(
        after["query"].as_str().unwrap().contains("teamId: \"LP\""),
        "{after}"
    );
}

// ---- Unchecked ids fail closed (Codex review of #1350) -----------------------

#[test]
fn an_id_the_guard_cannot_check_is_refused() {
    let refused = |q: &str, v: Value| preflight(q, &v).is_err();
    let found = |q: &str, v: Value| preflight(q, &v).unwrap().targets;
    assert!(refused(
        "mutation($i: ReactionCreateInput!) { reactionCreate(input: $i) { success } }",
        json!({ "i": { "projectUpdateId": "u1", "emoji": "+1" } })
    ));
    assert!(refused(
        "mutation { reactionCreate(input: {projectUpdateId: \"u1\", emoji: \"+1\"}) { success } }",
        Value::Null
    ));
    // A project, label or cycle id off an AI issue has its team read.
    assert_eq!(
        found(
            "mutation { commentCreate(input: {issueId: \"ENG-1\", projectId: \"p\"}) { success } }",
            Value::Null
        )
        .len(),
        2
    );
    // On an AI issue (teamId checked, or the issue's team read) they need no read.
    assert!(found(
        "mutation { issueCreate(input: {teamId: \"ENG\", title: \"x\", templateId: \"t\", labelIds: [\"l\"]}) { success } }",
        Value::Null
    )
    .is_empty());
    // R3: a workflow state and a source comment are read like any target.
    assert_eq!(
        found(
            "mutation { issueCreate(input: {teamId: \"ENG\", title: \"x\", stateId: \"s\", sourceCommentId: \"c\"}) { success } }",
            Value::Null
        ),
        [
            Target { kind: Entity::State, id: "s".into() },
            Target { kind: Entity::Comment, id: "c".into() },
        ]
    );
    // An issue's project is read even when the issue's team is known
    // (2026-11-13): Linear may add the issue's team to that project.
    assert_eq!(
        found(
            "mutation($i: IssueUpdateInput!) { issueUpdate(id: \"ENG-4\", input: $i) { success } }",
            json!({ "i": { "projectId": "p", "assigneeId": "u" } })
        ),
        [
            Target {
                kind: Entity::Issue,
                id: "ENG-4".into()
            },
            Target {
                kind: Entity::Project,
                id: "p".into()
            }
        ]
    );
    // A create with no team named has no team to lean on.
    assert!(refused(
        "mutation { issueCreate(input: {title: \"x\", templateId: \"t\"}) { success } }",
        Value::Null
    ));
}

#[tokio::test]
async fn attribute_ids_on_an_ai_issue_send_with_no_extra_read() {
    let server = team_server(json!({ "issueCreate": { "success": true } })).await;
    let dir = vault();
    let a = adapter(&server);
    let create = "mutation Make { issueCreate(input: {teamId: \"ENG\", title: \"x\", templateId: \"t\", labelIds: [\"l\"]}) { success } }";
    assert!(call(&a, dir.path(), "person".into(), create, None)
        .await
        .is_ok());
    // The issue's project is read with it, and sent when it is AI's.
    let update =
        "mutation Move { issueUpdate(id: \"ENG-4\", input: {projectId: \"ENG-p\"}) { success } }";
    assert!(call(&a, dir.path(), "person".into(), update, None)
        .await
        .is_ok());
    assert_eq!(
        ops(&server),
        [
            "HollowOrgGuard",
            "Make",
            "HollowOrgGuard",
            "HollowTeamGuard",
            "Move"
        ]
    );
    let guard = &server.requests()[3];
    assert_eq!(guard.variables()["t1"], "ENG-p");
    assert!(
        guard.query().contains("t1: project(id: $t1)"),
        "{}",
        guard.query()
    );
}

#[tokio::test]
async fn an_issues_project_or_milestone_in_another_team_is_refused() {
    let server = team_server(json!({ "issueUpdate": { "success": true } })).await;
    let dir = vault();
    let a = adapter(&server);
    let refused = [
        (
            "mutation Move { issueUpdate(id: \"ENG-4\", input: {projectId: \"LP-p\"}) { success } }",
            Value::Null,
            "project \"LP-p\" is in team LP",
        ),
        (
            "mutation Move { issueCreate(input: {teamId: \"ENG\", title: \"x\", projectMilestoneId: \"LP-m\"}) { success } }",
            Value::Null,
            "project milestone \"LP-m\" is in team LP",
        ),
        (
            "mutation Move($ids: [UUID!]!, $i: IssueUpdateInput!) { issueBatchUpdate(ids: $ids, input: $i) { success } }",
            json!({ "ids": ["ENG-1", "ENG-2"], "i": { "projectId": "SPAN-p" } }),
            "project \"SPAN-p\" spans team LP",
        ),
        (
            "mutation Move { issueUpdate(id: \"ENG-4\", input: {projectId: \"gone\"}) { success } }",
            Value::Null,
            "couldn't confirm which team project \"gone\"",
        ),
    ];
    for (q, vars, want) in refused {
        let error = call(&a, dir.path(), "person".into(), q, Some(vars))
            .await
            .unwrap_err();
        assert!(
            error.contains(want) && error.contains("Nothing was sent"),
            "{q}: {error}"
        );
    }
    assert!(!ops(&server).contains(&"Move".to_string()));
}

// ---- Audit text from the parsed document ---------------------------------------

#[tokio::test]
async fn the_audit_text_masks_secret_values_through_commas_comments_and_defaults() {
    let server = team_server(json!({})).await;
    let dir = vault();
    let a = adapter(&server);
    for q in [
        "mutation { webhookUpdate(id: \"w\", input: {secret: , \"s1-comma\"}) { success } }",
        "mutation { webhookUpdate(id: \"w\", input: {secret: # note\n \"s2-comment\"}) { success } }",
        "mutation { x(input: {signingKey: [\"s3-list\", \"more\"], password: {v: \"s4-object\"}}) { success } }",
        "query Q($token: String = \"s5-default\", $n: Int = 5) { viewer { id } }",
        "mutation { x(input: {privateKey: \"\"\"s6-block\"\"\"}) { success } }",
        // Refused (another team), still audited.
        "mutation { issueCreate(input: {teamId: \"LP\", password: , \"s7-refused\"}) { success } }",
    ] {
        let _ = call(&a, dir.path(), "person".into(), q, None).await;
    }
    let logged = rows(dir.path());
    let texts: Vec<String> = logged
        .iter()
        .map(|r| {
            r.fields.as_ref().unwrap()["after"]["query"]
                .as_str()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let all = texts.join("\n");
    for secret in ["s1-", "s2-", "s3-", "more", "s4-", "s5-", "s6-", "s7-"] {
        assert!(!all.contains(secret), "{secret} in {all}");
    }
    // Masked here, or by the shared redactor after it.
    let masked = |prefix: &str| {
        all.contains(&format!("{prefix}\"••••\""))
            || all.contains(&format!("{prefix}\"[redacted]\""))
    };
    assert!(masked("secret: "), "{all}");
    assert!(masked("privateKey: "), "{all}");
    assert!(masked("$token: String = "), "{all}");
    assert!(all.contains("$n: Int = 5"), "{all}");
    assert!(all.contains("teamId: \"LP\""), "{all}");
    // A document that doesn't parse keeps no text.
    let _ = call(
        &a,
        dir.path(),
        "person".into(),
        "mutation { x(secret: \"s8-bad\" { y } }",
        None,
    )
    .await;
    let after = rows(dir.path())[0].fields.as_ref().unwrap()["after"].clone();
    assert!(after["query"].is_null(), "{after}");
    assert!(!after.to_string().contains("s8-bad"), "{after}");
}

// ---- A null or missing team is no team (Codex round 2 on #1350) ---------------

#[test]
fn the_allowlist_needs_a_concrete_ai_team() {
    let refused = |q: &str, v: Value| preflight(q, &v).is_err();
    let reads = |q: &str, v: Value| !preflight(q, &v).unwrap().targets.is_empty();
    // teamId null, inline or through a variable, or a variable left missing:
    // the cycle is read, the state refused.
    assert!(reads(
        "mutation { issueCreate(input: {teamId: null, title: \"x\", cycleId: \"c\"}) { success } }",
        Value::Null
    ));
    let var =
        "mutation($t: String) { issueCreate(input: {teamId: $t, cycleId: \"c\"}) { success } }";
    assert!(reads(var, Value::Null));
    assert!(reads(var, json!({ "t": null })));
    assert!(!reads(var, json!({ "t": TEAM_ID })));
    let whole = "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }";
    assert!(refused(
        whole,
        json!({ "i": { "teamId": null, "templateId": "t" } })
    ));
    assert!(!refused(
        whole,
        json!({ "i": { "teamId": "ENG", "templateId": "t" } })
    ));
}

#[test]
fn a_bare_id_nested_in_an_input_is_refused() {
    let refused = |q: &str, v: Value| preflight(q, &v).is_err();
    assert!(refused(
        "mutation { issueCreate(input: {id: \"x\", teamId: \"ENG\", title: \"x\"}) { success } }",
        Value::Null
    ));
    assert!(refused(
        "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }",
        json!({ "i": { "teamId": "ENG", "x": { "id": "y" } } })
    ));
    // The field's own root `id` is the read target, not a nested one.
    assert!(!refused(
        "mutation { issueUpdate(id: \"ENG-1\", input: {title: \"x\"}) { success } }",
        Value::Null
    ));
}

// ---- The second team: exactly AI and HOL (2026-11-12) ------------------------

#[test]
fn a_mutation_may_name_the_shared_or_the_personal_team_and_no_other() {
    let create = |input: &str| format!("mutation {{ issueCreate(input: {input}) {{ success }} }}");
    // Both teams, by id and by key (any case), inline and by variable.
    for named in [TEAM_ID, PERSONAL_TEAM_ID, "ENG", "PER", "per"] {
        assert!(
            preflight(
                &create(&format!("{{teamId: \"{named}\", title: \"x\"}}")),
                &Value::Null
            )
            .is_ok(),
            "{named}"
        );
    }
    let q = "mutation($t: String!) { issueCreate(input: {teamId: $t}) { success } }";
    assert!(preflight(q, &json!({ "t": PERSONAL_TEAM_ID })).is_ok());
    let project = "mutation($i: ProjectCreateInput!) { projectCreate(input: $i) { success } }";
    assert!(preflight(
        project,
        &json!({ "i": { "teamIds": [TEAM_ID, PERSONAL_TEAM_ID] } })
    )
    .is_ok());
    // LP and PRO, by key and by id, alone or beside an allowed team.
    for refused in ["LP", "PRO", "pro", OTHER_TEAM, PRO_TEAM] {
        assert!(
            preflight(
                &create(&format!("{{teamId: \"{refused}\", title: \"x\"}}")),
                &Value::Null
            )
            .is_err(),
            "{refused}"
        );
        assert!(preflight(q, &json!({ "t": refused })).is_err(), "{refused}");
        assert!(
            preflight(
                project,
                &json!({ "i": { "teamIds": [PERSONAL_TEAM_ID, refused] } })
            )
            .is_err(),
            "{refused}"
        );
        let update = format!(
            "mutation {{ teamUpdate(id: \"{refused}\", input: {{name: \"x\"}}) {{ success }} }}"
        );
        assert!(preflight(&update, &Value::Null).is_err(), "{refused}");
    }
    // A team update names the personal team's id too.
    let update = format!(
        "mutation {{ teamUpdate(id: \"{PERSONAL_TEAM_ID}\", input: {{name: \"x\"}}) {{ success }} }}"
    );
    assert!(preflight(&update, &Value::Null).is_ok());
    // Without the personal team set up, it is refused like LP and PRO.
    assert!(preflight_with(q, &json!({ "t": PERSONAL_TEAM_ID }), false).is_err());
    assert!(preflight_with(q, &json!({ "t": "PER" }), false).is_err());
    assert!(preflight_with(q, &json!({ "t": TEAM_ID }), false).is_ok());
    assert!(
        preflight(q, &json!({ "t": " HOL " })).is_err(),
        "no padding"
    );
}

#[tokio::test]
async fn an_update_on_a_personal_team_issue_passes_the_team_read_and_pro_is_refused() {
    let server = team_server(json!({ "issueUpdate": { "success": true } })).await;
    let dir = vault();
    let q = |key: &str| {
        format!(
            "mutation Move {{ issueUpdate(id: \"{key}\", input: {{title: \"x\"}}) {{ success }} }}"
        )
    };
    // Without the personal team in the config it is refused like any other.
    assert!(call(
        &adapter(&server),
        dir.path(),
        "person".into(),
        &q("PER-3"),
        None
    )
    .await
    .is_err());
    assert!(!ops(&server).contains(&"Move".to_string()));
    let adapter = adapter(&server).with_personal(true);
    call(&adapter, dir.path(), "person".into(), &q("PER-3"), None)
        .await
        .unwrap();
    assert!(ops(&server).ends_with(&[
        "HollowOrgGuard".into(),
        "HollowTeamGuard".into(),
        "Move".into()
    ]));
    for other in ["OPS-1", "LP-12"] {
        let before = server.requests().len();
        let error = call(&adapter, dir.path(), "person".into(), &q(other), None)
            .await
            .unwrap_err();
        assert!(error.contains("Nothing was sent"), "{other}: {error}");
        assert!(
            server.requests()[before..]
                .iter()
                .all(|r| r.operation().as_deref() != Some("Move")),
            "{other}"
        );
    }
}

// ---- Unresolved by-id targets are team-checked, never sent on a card (2026-11-13)

#[tokio::test]
async fn a_project_in_another_team_is_refused_and_one_in_ai_is_sent() {
    let server = team_server(json!({ "projectUpdate": { "success": true } })).await;
    let dir = vault();
    let a = adapter(&server);
    let q = |id: &str| {
        format!(
            "mutation Status {{ projectUpdate(id: \"{id}\", input: {{name: \"x\"}}) {{ success }} }}"
        )
    };
    let error = call(&a, dir.path(), "person".into(), &q("LP-p"), None)
        .await
        .unwrap_err();
    assert!(
        error.contains("project \"LP-p\" is in team LP") && error.contains("Nothing was sent"),
        "{error}"
    );
    // A project in AI and LP spans a team Hollow doesn't use.
    let error = call(&a, dir.path(), "person".into(), &q("SPAN-p"), None)
        .await
        .unwrap_err();
    assert!(error.contains("spans team LP"), "{error}");
    assert!(!ops(&server).contains(&"Status".to_string()));
    let reply = call(&a, dir.path(), "person".into(), &q("ENG-p"), None)
        .await
        .unwrap();
    assert_eq!(reply["data"]["projectUpdate"]["success"], true);
    assert!(ops(&server).ends_with(&[
        "HollowOrgGuard".into(),
        "HollowTeamGuard".into(),
        "Status".into()
    ]));
    assert!(server
        .requests()
        .iter()
        .any(|r| r.query().contains("t0: project(id: $t0) { teams {")));
}

#[tokio::test]
async fn a_workspace_label_a_pro_label_and_an_lp_cycle_are_refused() {
    let server = team_server(json!({})).await;
    let dir = vault();
    let a = adapter(&server);
    let error = call(
        &a,
        dir.path(),
        "person".into(),
        "mutation Rename { issueLabelUpdate(id: \"WS-l\", input: {name: \"x\"}) { success } }",
        None,
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("label \"WS-l\" is in no team") && error.contains("shared across teams"),
        "{error}"
    );
    let label = "mutation Rename($i: IssueLabelUpdateInput!) { issueLabelUpdate(id: \"PRO-l\", input: $i) { success } }";
    let error = call(
        &a,
        dir.path(),
        "person".into(),
        label,
        Some(json!({ "i": { "name": "x" } })),
    )
    .await
    .unwrap_err();
    assert!(error.contains("team PRO"), "{error}");
    let cycle = "mutation Rename { cycleUpdate(id: \"LP-c\", input: {name: \"x\"}) { success } }";
    assert!(call(&a, dir.path(), "person".into(), cycle, None)
        .await
        .unwrap_err()
        .contains("team LP"));
    // A milestone is read through its project.
    let milestone = "mutation Rename { projectMilestoneUpdate(id: \"SPAN-m\", input: {name: \"x\"}) { success } }";
    assert!(call(&a, dir.path(), "person".into(), milestone, None)
        .await
        .unwrap_err()
        .contains("spans team LP"));
    assert!(ops(&server)
        .iter()
        .all(|o| o == "HollowOrgGuard" || o == "HollowTeamGuard"));
}

#[tokio::test]
async fn an_unresolvable_target_is_refused_not_sent_even_after_a_card() {
    let server = team_server(json!({ "documentUpdate": { "success": true } })).await;
    let dir = vault();
    let a = adapter(&server);
    let q = "mutation Edit { documentUpdate(id: \"doc-1\", input: {title: \"x\"}) { success } }";
    // `call` is what runs once a card is approved: it still refuses.
    let error = call(&a, dir.path(), "person".into(), q, None)
        .await
        .unwrap_err();
    assert!(
        error.contains("documentUpdate") && error.contains("Nothing was sent"),
        "{error}"
    );
    assert!(server.requests().is_empty(), "{:?}", ops(&server));
    assert_eq!(rows(dir.path())[0].kind, kind::TRACKER_WRITE_FAILED);
}

// ---- Security review of #1388 (2026-11-13) -----------------------------------

#[test]
fn a_team_updates_parent_must_be_an_allowed_team_and_its_input_ids_are_read() {
    let update = |input: &str| {
        format!("mutation {{ teamUpdate(id: \"{TEAM_ID}\", input: {input}) {{ success }} }}")
    };
    // Under LP, under nothing, by a key, by a variable, inside a whole input.
    for input in [
        format!("{{parentId: \"{OTHER_TEAM}\"}}"),
        "{parentId: null}".to_string(),
        "{parentId: \"LP\"}".to_string(),
    ] {
        let error = preflight(&update(&input), &Value::Null).unwrap_err();
        assert!(
            error.contains("teamUpdate's parentId") && error.contains("Nothing was sent"),
            "{input}: {error}"
        );
    }
    let var = format!(
        "mutation($p: String) {{ teamUpdate(id: \"{TEAM_ID}\", input: {{parentId: $p}}) {{ success }} }}"
    );
    assert!(preflight(&var, &json!({ "p": OTHER_TEAM })).is_err());
    assert!(preflight(&var, &Value::Null).is_err());
    let whole = "mutation($id: String!, $i: TeamUpdateInput!) { teamUpdate(id: $id, input: $i) { success } }";
    assert!(preflight(
        whole,
        &json!({ "id": TEAM_ID, "i": { "name": "x", "parentId": OTHER_TEAM } })
    )
    .is_err());
    // Under the other allowed team is fine.
    assert!(preflight(
        &update(&format!("{{parentId: \"{PERSONAL_TEAM_ID}\"}}")),
        &Value::Null
    )
    .is_ok());
    assert!(preflight(
        whole,
        &json!({ "id": TEAM_ID, "i": { "parentId": PERSONAL_TEAM_ID } })
    )
    .is_ok());
    // Any other id in the input is read or refused like any field's.
    for input in [
        "{defaultIssueStateId: \"s\"}",
        "{id: \"x\"}",
        "{x: {ids: [\"y\"]}}",
    ] {
        let error = preflight(&update(input), &Value::Null).unwrap_err();
        assert!(
            error.contains("can't read which team teamUpdate"),
            "{input}: {error}"
        );
    }
    assert!(preflight(
        whole,
        &json!({ "id": TEAM_ID, "i": { "markedAsDuplicateWorkflowStateId": "s" } })
    )
    .is_err());
    // A plain rename still goes through with no read.
    assert!(preflight(&update("{name: \"x\"}"), &Value::Null)
        .unwrap()
        .targets
        .is_empty());
}

#[tokio::test]
async fn a_team_update_under_lp_sends_nothing_at_all() {
    let server = team_server(json!({ "teamUpdate": { "success": true } })).await;
    let dir = vault();
    let q = format!(
        "mutation Nest {{ teamUpdate(id: \"{TEAM_ID}\", input: {{parentId: \"{OTHER_TEAM}\"}}) {{ success }} }}"
    );
    let error = call(&adapter(&server), dir.path(), "person".into(), &q, None)
        .await
        .unwrap_err();
    assert!(error.contains("Nothing was sent"), "{error}");
    assert!(server.requests().is_empty(), "{:?}", ops(&server));
}

#[test]
fn an_issue_label_create_needs_an_allowed_team() {
    for (q, v) in [
        (
            "mutation { issueLabelCreate(input: {name: \"x\"}) { success } }",
            Value::Null,
        ),
        (
            "mutation { issueLabelCreate(input: {name: \"x\", teamId: null}) { success } }",
            Value::Null,
        ),
        (
            "mutation($i: IssueLabelCreateInput!) { issueLabelCreate(input: $i) { success } }",
            json!({ "i": { "name": "x" } }),
        ),
        (
            "mutation($t: String) { issueLabelCreate(input: {name: \"x\", teamId: $t}) { success } }",
            json!({ "t": null }),
        ),
    ] {
        let error = preflight(q, &v).unwrap_err();
        assert!(
            error.contains("workspace label")
                && error.contains("shared across teams")
                && error.contains("Nothing was sent"),
            "{q}: {error}"
        );
    }
    assert!(preflight(
        "mutation { issueLabelCreate(input: {name: \"x\", teamId: \"ENG\"}) { success } }",
        &Value::Null
    )
    .is_ok());
    assert!(preflight(
        "mutation($i: IssueLabelCreateInput!) { issueLabelCreate(input: $i) { success } }",
        &json!({ "i": { "name": "x", "teamId": PERSONAL_TEAM_ID } })
    )
    .is_ok());
    // Other workspace-level creates are refused too (R3, N18).
    assert!(preflight(
        "mutation { projectLabelCreate(input: {name: \"x\"}) { success } }",
        &Value::Null
    )
    .is_err());
}

#[test]
fn a_projects_status_and_labels_need_no_read_once_its_teams_are_known() {
    let found = |q: &str, v: Value| preflight(q, &v).map(|s| s.targets);
    let project = |id: &str| Target {
        kind: Entity::Project,
        id: id.into(),
    };
    assert_eq!(
        found(
            "mutation { projectUpdate(id: \"p1\", input: {statusId: \"s\", labelIds: [\"pl\"]}) { success } }",
            Value::Null
        )
        .unwrap(),
        [project("p1")]
    );
    assert_eq!(
        found(
            "mutation($i: ProjectUpdateInput!) { projectUpdate(id: \"p1\", input: $i) { success } }",
            json!({ "i": { "statusId": "s" } })
        )
        .unwrap(),
        [project("p1")]
    );
    assert!(found(
        "mutation { projectCreate(input: {name: \"x\", teamIds: [\"ENG\"], statusId: \"s\", labelIds: [\"pl\"]}) { success } }",
        Value::Null
    )
    .unwrap()
    .is_empty());
    assert!(found(
        "mutation($i: ProjectCreateInput!) { projectCreate(input: $i) { success } }",
        json!({ "i": { "teamIds": [TEAM_ID], "statusId": "s" } })
    )
    .unwrap()
    .is_empty());
    // No team known, or not a project: still refused.
    for (q, v) in [
        (
            "mutation { projectCreate(input: {name: \"x\", statusId: \"s\"}) { success } }",
            Value::Null,
        ),
        (
            "mutation($i: ProjectCreateInput!) { projectCreate(input: $i) { success } }",
            json!({ "i": { "teamIds": [], "statusId": "s" } }),
        ),
        (
            "mutation { issueUpdate(id: \"ENG-1\", input: {statusId: \"s\"}) { success } }",
            Value::Null,
        ),
        (
            "mutation { projectArchive(id: \"p1\", statusId: \"s\") { success } }",
            Value::Null,
        ),
    ] {
        assert!(found(q, v).is_err(), "{q}");
    }
}

#[tokio::test]
async fn a_project_status_change_is_sent_only_for_a_project_in_ai() {
    let server = team_server(json!({ "projectUpdate": { "success": true } })).await;
    let dir = vault();
    let a = adapter(&server);
    let q = |id: &str| {
        format!("mutation Status {{ projectUpdate(id: \"{id}\", input: {{statusId: \"s\"}}) {{ success }} }}")
    };
    let error = call(&a, dir.path(), "person".into(), &q("LP-p"), None)
        .await
        .unwrap_err();
    assert!(error.contains("team LP"), "{error}");
    assert!(!ops(&server).contains(&"Status".to_string()));
    call(&a, dir.path(), "person".into(), &q("ENG-p"), None)
        .await
        .unwrap();
    assert!(ops(&server).ends_with(&[
        "HollowOrgGuard".into(),
        "HollowTeamGuard".into(),
        "Status".into()
    ]));
}

#[test]
fn a_nested_ids_key_is_refused() {
    let refused = |q: &str, v: Value| preflight(q, &v).is_err();
    assert!(refused(
        "mutation { issueCreate(input: {teamId: \"ENG\", x: {ids: [\"y\"]}}) { success } }",
        Value::Null
    ));
    assert!(refused(
        "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }",
        json!({ "i": { "teamId": "ENG", "ids": ["y"] } })
    ));
    // The field's own root `ids` is still the read target.
    assert!(!refused(
        "mutation($ids: [UUID!]!) { issueBatchUpdate(ids: $ids, input: {title: \"x\"}) { success } }",
        json!({ "ids": ["ENG-1"] })
    ));
}

/// Answers the org guard and the team guard with `guard`, anything else with
/// success.
async fn guard_server(guard: impl Fn(&str) -> StubReply + Send + Sync + 'static) -> StubServer {
    stub::serve(move |req| match req.operation().as_deref() {
        Some("HollowOrgGuard") => org("Example Org"),
        Some("HollowTeamGuard") => guard(req.variables()["t0"].as_str().unwrap_or("")),
        _ => StubReply::data(json!({ "ok": true })),
    })
    .await
}

#[tokio::test]
async fn a_team_list_cut_short_or_without_page_info_is_refused() {
    let ai = json!({ "id": TEAM_ID, "key": "ENG" });
    let server = guard_server(move |id| {
        let teams = match id {
            "more" => json!({ "nodes": [ai.clone()], "pageInfo": { "hasNextPage": true } }),
            "nopage" => json!({ "nodes": [ai.clone()] }),
            _ => json!({ "nodes": [ai.clone()], "pageInfo": {} }),
        };
        StubReply::data(json!({ "t0": { "teams": teams.clone(), "project": { "teams": teams } } }))
    })
    .await;
    let dir = vault();
    let a = adapter(&server);
    for q in [
        "mutation Go { projectUpdate(id: \"more\", input: {name: \"x\"}) { success } }",
        "mutation Go { projectUpdate(id: \"nopage\", input: {name: \"x\"}) { success } }",
        "mutation Go { projectUpdate(id: \"nullpage\", input: {name: \"x\"}) { success } }",
        "mutation Go { projectMilestoneUpdate(id: \"more\", input: {name: \"x\"}) { success } }",
        "mutation Go { projectMilestoneUpdate(id: \"nopage\", input: {name: \"x\"}) { success } }",
    ] {
        let error = call(&a, dir.path(), "person".into(), q, None)
            .await
            .unwrap_err();
        assert!(
            error.contains("couldn't confirm") && error.contains("Nothing was sent"),
            "{q}: {error}"
        );
    }
    assert!(!ops(&server).contains(&"Go".to_string()));
}

#[tokio::test]
async fn a_project_label_or_cycle_not_found_is_refused() {
    let server = team_server(json!({ "ok": true })).await;
    let dir = vault();
    let a = adapter(&server);
    for q in [
        "mutation Go { projectUpdate(id: \"gone-p\", input: {name: \"x\"}) { success } }",
        "mutation Go { issueLabelUpdate(id: \"gone-l\", input: {name: \"x\"}) { success } }",
        "mutation Go { cycleUpdate(id: \"gone-c\", input: {name: \"x\"}) { success } }",
        "mutation Go { projectMilestoneUpdate(id: \"gone-m\", input: {name: \"x\"}) { success } }",
    ] {
        let error = call(&a, dir.path(), "person".into(), q, None)
            .await
            .unwrap_err();
        assert!(
            error.contains("couldn't confirm") && error.contains("Nothing was sent"),
            "{q}: {error}"
        );
    }
    assert!(!ops(&server).contains(&"Go".to_string()));
}

#[tokio::test]
async fn the_team_guard_failing_sends_nothing() {
    let q = "mutation Go { issueUpdate(id: \"ENG-1\", input: {title: \"x\"}) { success } }";
    for reply in [
        // Not JSON at all: a transport-level failure.
        StubReply {
            status: 502,
            headers: Vec::new(),
            body: "bad gateway".into(),
        },
        StubReply::status(500),
        StubReply::error("INTERNAL_SERVER_ERROR", "boom"),
    ] {
        let server = guard_server(move |_| reply.clone()).await;
        let dir = vault();
        let error = call(&adapter(&server), dir.path(), "person".into(), q, None)
            .await
            .unwrap_err();
        assert!(error.contains("Nothing was sent"), "{error}");
        assert_eq!(ops(&server), ["HollowOrgGuard", "HollowTeamGuard"]);
        assert_eq!(rows(dir.path())[0].kind, kind::TRACKER_WRITE_FAILED);
    }
}

// ---- One scope guard (R3, 2026-11-14) -----------------------------------------
//
// Allowed: Engineering and Personal only. LP and PRO are named below as the
// teams that are refused, by key and by id, for reads and for every create.

#[test]
fn queries_naming_lp_pro_or_any_other_team_are_refused() {
    let read = |q: &str, v: Value| preflight(q, &v);
    for team in ["LP", "PRO", "lp", OTHER_TEAM, PRO_TEAM] {
        // Inline filter, by key and by id, and a list.
        for filter in [
            format!("{{team: {{key: {{eq: \"{team}\"}}}}}}"),
            format!("{{team: {{id: {{eq: \"{team}\"}}}}}}"),
            format!("{{team: {{key: {{in: [\"ENG\", \"{team}\"]}}}}}}"),
        ] {
            let q = format!("query {{ issues(filter: {filter}) {{ nodes {{ id }} }} }}");
            assert!(read(&q, Value::Null).is_err(), "{q}");
        }
        // A team argument, a nested field's, and `team(id:)` / `teams(filter:)`.
        for q in [
            format!("query {{ issues(teamId: \"{team}\") {{ nodes {{ id }} }} }}"),
            format!("query {{ viewer {{ id teams(filter: {{key: {{eq: \"{team}\"}}}}) {{ nodes {{ id }} }} }} }}"),
            format!("query {{ team(id: \"{team}\") {{ issues {{ nodes {{ id }} }} }} }}"),
            format!("query {{ teams(filter: {{id: {{eq: \"{team}\"}}}}) {{ nodes {{ id }} }} }}"),
            format!("query {{ organization {{ id }} x: team(id: \"{team}\") {{ id }} }}"),
        ] {
            assert!(read(&q, Value::Null).is_err(), "{q}");
        }
        // A variable, bare or inside a whole filter.
        let by_var =
            "query($t: String!) { issues(filter: {team: {key: {eq: $t}}}) { nodes { id } } }";
        assert!(read(by_var, json!({ "t": team })).is_err(), "{team}");
        let whole = "query($f: IssueFilter) { issues(filter: $f) { nodes { id } } }";
        assert!(
            read(whole, json!({ "f": { "team": { "key": { "eq": team } } } })).is_err(),
            "{team}"
        );
        let id_var = "query($team: ID!) { issues(teamId: $team) { nodes { id } } }";
        assert!(read(id_var, json!({ "team": team })).is_err(), "{team}");
    }
    // An operator that selects the other teams is refused even over AI.
    for filter in [
        "{team: {key: {neq: \"ENG\"}}}",
        "{team: {key: {nin: [\"ENG\"]}}}",
        "{team: {or: [{key: {eq: \"ENG\"}}, {key: {eq: \"PRO\"}}]}}",
        "{team: {key: {startsWith: \"A\"}}}",
    ] {
        let q = format!("query {{ issues(filter: {filter}) {{ nodes {{ id }} }} }}");
        assert!(read(&q, Value::Null).is_err(), "{q}");
    }
    // AI and HOL, and a read that names no team, go through.
    for q in [
        "query { issues(filter: {team: {key: {eq: \"ENG\"}}}) { nodes { id } } }".into(),
        format!("query {{ issues(filter: {{team: {{id: {{in: [\"{TEAM_ID}\", \"{PERSONAL_TEAM_ID}\"]}}}}}}) {{ nodes {{ id }} }} }}"),
        "query { team(id: \"PER\") { issues { nodes { id } } } }".into(),
        "query { viewer { id } }".into(),
        "query($t: String!) { issues(filter: {team: {key: {eq: $t}}}) { nodes { id } } }".into(),
    ] {
        let v = json!({ "t": "ENG" });
        assert!(read(&q, v).is_ok(), "{q}");
    }
}

#[tokio::test]
async fn a_refused_read_of_lp_sends_nothing() {
    let server = team_server(json!({ "issues": { "nodes": [] } })).await;
    let dir = vault();
    let q = "query Peek { issues(filter: {team: {key: {eq: \"LP\"}}}) { nodes { id } } }";
    let error = call(&adapter(&server), dir.path(), "person".into(), q, None)
        .await
        .unwrap_err();
    assert!(error.contains("Engineering"), "{error}");
    assert!(
        ops(&server).is_empty(),
        "no request, not even the org guard"
    );
}

#[test]
fn every_workspace_wide_create_is_refused_and_a_team_create_names_ai_or_hol() {
    let refused = |q: &str, v: Value| preflight(q, &v).is_err();
    // Creates with no team of their own: refused whatever they carry.
    for (field, kind) in [
        ("initiativeCreate", "InitiativeCreateInput"),
        ("projectLabelCreate", "ProjectLabelCreateInput"),
        ("projectStatusCreate", "ProjectStatusCreateInput"),
    ] {
        let q = format!("mutation($i: {kind}!) {{ {field}(input: $i) {{ success }} }}");
        assert!(refused(&q, json!({ "i": { "name": "x" } })), "{field}");
        assert!(
            refused(
                &format!("mutation {{ {field}(input: {{name: \"x\"}}) {{ success }} }}"),
                Value::Null
            ),
            "{field}"
        );
        for team in ["LP", "PRO", OTHER_TEAM, PRO_TEAM] {
            assert!(
                refused(&q, json!({ "i": { "name": "x", "teamId": team } })),
                "{field} {team}"
            );
        }
    }
    // Creates that may be team-scoped: refused with no team and with LP or PRO.
    for (field, kind) in [
        ("templateCreate", "TemplateCreateInput"),
        ("customViewCreate", "CustomViewCreateInput"),
        ("issueLabelCreate", "IssueLabelCreateInput"),
    ] {
        let q = format!("mutation($i: {kind}!) {{ {field}(input: $i) {{ success }} }}");
        assert!(
            refused(&q, json!({ "i": { "name": "x" } })),
            "{field} no team"
        );
        assert!(
            refused(&q, json!({ "i": { "name": "x", "teamId": null } })),
            "{field} null"
        );
        for team in ["LP", "PRO", "pro", OTHER_TEAM, PRO_TEAM] {
            assert!(
                refused(&q, json!({ "i": { "name": "x", "teamId": team } })),
                "{field} {team}"
            );
            let inline = format!(
                "mutation {{ {field}(input: {{name: \"x\", teamId: \"{team}\"}}) {{ success }} }}"
            );
            assert!(refused(&inline, Value::Null), "{field} {team}");
        }
        assert!(
            !refused(&q, json!({ "i": { "name": "x", "teamId": "ENG" } })),
            "{field} AI"
        );
        assert!(
            !refused(
                &q,
                json!({ "i": { "name": "x", "teamId": PERSONAL_TEAM_ID } })
            ),
            "{field} HOL"
        );
    }
    // Team creates of the other kinds, LP and PRO named.
    for (field, kind) in [
        ("issueCreate", "IssueCreateInput"),
        ("projectCreate", "ProjectCreateInput"),
    ] {
        let q = format!("mutation($i: {kind}!) {{ {field}(input: $i) {{ success }} }}");
        for team in ["LP", "PRO", OTHER_TEAM, PRO_TEAM] {
            assert!(
                refused(&q, json!({ "i": { "teamId": team, "teamIds": [team] } })),
                "{field} {team}"
            );
        }
    }
}

#[test]
fn a_project_update_may_not_clear_its_teams() {
    let update =
        "mutation($i: ProjectUpdateInput!) { projectUpdate(id: \"p1\", input: $i) { success } }";
    assert!(preflight(update, &json!({ "i": { "teamIds": [] } })).is_err());
    assert!(preflight(update, &json!({ "i": { "teamIds": null } })).is_err());
    assert!(preflight(update, &json!({ "i": { "teamIds": ["LP"] } })).is_err());
    assert!(preflight(update, &json!({ "i": { "teamIds": [TEAM_ID] } })).is_ok());
    assert!(preflight(update, &json!({ "i": { "name": "x" } })).is_ok());
    assert!(preflight(
        "mutation { projectUpdate(id: \"p1\", input: {teamIds: []}) { success } }",
        &Value::Null
    )
    .is_err());
    let bare =
        "mutation($t: [String!]) { projectUpdate(id: \"p1\", input: {teamIds: $t}) { success } }";
    assert!(preflight(bare, &json!({ "t": [] })).is_err());
    assert!(preflight(bare, &json!({ "t": ["PRO"] })).is_err());
    assert!(preflight(bare, &json!({ "t": ["ENG"] })).is_ok());
}

#[tokio::test]
async fn a_source_comment_and_a_state_are_read_for_their_team() {
    let server = team_server(json!({ "issueCreate": { "success": true } })).await;
    let dir = vault();
    let a = adapter(&server);
    let make = |field: &str, id: &str| {
        format!(
            "mutation Make {{ issueCreate(input: {{teamId: \"ENG\", title: \"x\", {field}: \"{id}\"}}) {{ success }} }}"
        )
    };
    // A comment or state of an LP or PRO issue is refused after its team is read.
    for (field, id, team) in [
        ("sourceCommentId", "LP-3", "LP"),
        ("sourceCommentId", "PRO-3", "PRO"),
        ("stateId", "LP-3", "LP"),
        ("stateId", "PRO-3", "PRO"),
    ] {
        let error = call(&a, dir.path(), "person".into(), &make(field, id), None)
            .await
            .unwrap_err();
        assert!(
            error.contains(&format!("team {team}")),
            "{field} {id}: {error}"
        );
    }
    assert!(ops(&server).iter().all(|o| o != "Make"), "never sent");
    // An AI one goes through, after the read.
    let before = ops(&server).len();
    assert!(call(
        &a,
        dir.path(),
        "person".into(),
        &make("sourceCommentId", "ENG-3"),
        None
    )
    .await
    .is_ok());
    assert_eq!(
        &ops(&server)[before..],
        ["HollowOrgGuard", "HollowTeamGuard", "Make"]
    );
    // The linked project is read too: an LP project is refused.
    let project = "mutation Make { issueCreate(input: {teamId: \"ENG\", title: \"x\", projectId: \"LP-9\"}) { success } }";
    let error = call(&a, dir.path(), "person".into(), project, None)
        .await
        .unwrap_err();
    assert!(error.contains("team LP"), "{error}");
}

// ---- R3 review round: the bypasses (2026-11-14) --------------------------------

/// 1: a `teamId` nested inside a template or view doesn't scope it.
#[test]
fn a_team_id_nested_in_a_template_or_view_does_not_scope_it() {
    for field in ["templateCreate", "customViewCreate"] {
        for nested in ["templateData", "filters"] {
            let inline = format!(
                "mutation {{ {field}(input: {{name: \"x\", {nested}: {{teamId: \"ENG\"}}}}) {{ success }} }}"
            );
            assert!(
                preflight(&inline, &Value::Null).is_err(),
                "{field} {nested}"
            );
            let q = format!("mutation($i: X!) {{ {field}(input: $i) {{ success }} }}");
            assert!(
                preflight(
                    &q,
                    &json!({ "i": { "name": "x", nested: { "teamId": "ENG" } } })
                )
                .is_err(),
                "{field} {nested} var"
            );
        }
        // Its own top-level teamId of AI or HOL scopes it, inline or by variable.
        let own = format!(
            "mutation {{ {field}(input: {{name: \"x\", teamId: \"ENG\", filters: {{x: 1}}}}) {{ success }} }}"
        );
        assert!(preflight(&own, &Value::Null).is_ok(), "{field}");
        let by_var = format!(
            "mutation($t: String) {{ {field}(input: {{name: \"x\", teamId: $t}}) {{ success }} }}"
        );
        assert!(
            preflight(&by_var, &json!({ "t": TEAM_ID })).is_ok(),
            "{field}"
        );
        // LP and PRO at the top, with AI nested, are still refused.
        for team in ["LP", "PRO"] {
            let q = format!(
                "mutation {{ {field}(input: {{teamId: \"{team}\", templateData: {{teamId: \"ENG\"}}}}) {{ success }} }}"
            );
            assert!(preflight(&q, &Value::Null).is_err(), "{field} {team}");
        }
    }
}

/// 2: the allowlist: any other `...Create` is refused.
#[test]
fn only_known_team_scoped_creates_pass() {
    for field in [
        "customerCreate",
        "customerStatusCreate",
        "customerTierCreate",
        "emojiCreate",
        "roadmapCreate",
        "initiativeCreate",
        "documentCreate",
        "initiativeToProjectCreate",
        "projectStatusCreate",
        "projectLabelCreate",
    ] {
        for team in ["LP", "PRO", "ENG"] {
            let q = format!(
                "mutation {{ {field}(input: {{name: \"x\", teamId: \"{team}\"}}) {{ success }} }}"
            );
            assert!(preflight(&q, &Value::Null).is_err(), "{field} {team}");
        }
    }
    for field in [
        "issueCreate",
        "projectCreate",
        "cycleCreate",
        "workflowStateCreate",
        "projectMilestoneCreate",
    ] {
        let q = format!("mutation {{ {field}(input: {{name: \"x\", teamId: \"ENG\", teamIds: [\"ENG\"]}}) {{ success }} }}");
        assert!(preflight(&q, &Value::Null).is_ok(), "{field}");
    }
    // The access-granting ones keep asking on a card instead.
    assert!(asks_always(
        "mutation { webhookCreate(input: {url: \"u\", teamId: \"ENG\"}) { success } }",
        &Value::Null
    ));
}

/// 3: a default can't stand in for a missing variable.
#[test]
fn a_query_default_naming_a_team_must_be_passed_explicitly() {
    for team in ["LP", "PRO"] {
        let q = format!(
            "query($t: String = \"{team}\") {{ issues(filter: {{team: {{key: {{eq: $t}}}}}}) {{ nodes {{ id }} }} }}"
        );
        assert!(preflight(&q, &json!({})).is_err(), "{team}");
        assert!(preflight(&q, &Value::Null).is_err(), "{team}");
        // Passed explicitly it is checked like any variable.
        assert!(preflight(&q, &json!({ "t": team })).is_err(), "{team}");
        assert!(preflight(&q, &json!({ "t": "ENG" })).is_ok(), "{team}");
        let whole = format!(
            "query($f: IssueFilter = {{team: {{key: {{eq: \"{team}\"}}}}}}) {{ issues(filter: $f) {{ nodes {{ id }} }} }}"
        );
        assert!(preflight(&whole, &json!({})).is_err(), "{team}");
    }
    // A numeric default stays fine.
    assert!(preflight(
        "query($n: Int = 50) { issues(first: $n) { nodes { id } } }",
        &json!({})
    )
    .is_ok());
}

/// 4: a variable filter is as strict as an inline one.
#[test]
fn a_variable_team_filter_allows_only_id_key_eq_and_in() {
    let q = "query($f: TeamFilter) { issues(filter: {team: $f}) { nodes { id } } }";
    for f in [
        json!({ "key": { "eq": "ENG" }, "name": { "eq": "x" } }),
        json!({ "key": { "eq": "ENG" }, "or": [{ "key": { "eq": "LP" } }] }),
        json!({ "key": { "eq": "ENG" }, "or": [{ "key": { "eq": "PRO" } }] }),
        json!({ "key": { "neq": "ENG" } }),
        json!({}),
    ] {
        assert!(preflight(q, &json!({ "f": f })).is_err(), "{f}");
    }
    assert!(preflight(q, &json!({ "f": { "key": { "eq": "ENG" } } })).is_ok());
}

/// 5: `teams(filter:)` by variable or by name.
#[test]
fn a_teams_filter_may_only_name_ai_or_hol_by_id_or_key() {
    let by_var = "query($f: TeamFilter) { teams(filter: $f) { nodes { id } } }";
    for f in [
        json!({ "key": { "eq": "LP" } }),
        json!({ "key": { "in": ["ENG", "PRO"] } }),
        json!({ "name": { "eq": "Launchpad" } }),
        json!({ "or": [{ "key": { "eq": "PRO" } }] }),
        json!({ "id": { "eq": OTHER_TEAM } }),
        json!({ "id": { "eq": PRO_TEAM } }),
    ] {
        assert!(preflight(by_var, &json!({ "f": f })).is_err(), "{f}");
    }
    assert!(preflight(by_var, &json!({ "f": { "key": { "eq": "ENG" } } })).is_ok());
    for q in [
        "query { teams(filter: {name: {eq: \"Launchpad\"}}) { nodes { id } } }",
        "query { teams(filter: {name: {contains: \"PRO\"}}) { nodes { id } } }",
        "query { teams(filter: {or: [{key: {eq: \"ENG\"}}, {key: {eq: \"LP\"}}]}) { nodes { id } } }",
    ] {
        assert!(preflight(q, &Value::Null).is_err(), "{q}");
    }
}

/// 6: a team relation not named `team...`.
#[test]
fn any_argument_key_containing_team_is_held_to_ai_and_hol() {
    for team in ["LP", "PRO"] {
        for q in [
            format!("query {{ projects(filter: {{accessibleTeams: {{some: {{key: {{eq: \"{team}\"}}}}}}}}) {{ nodes {{ id }} }} }}"),
            format!("query {{ issues(filter: {{subTeam: {{key: {{eq: \"{team}\"}}}}}}) {{ nodes {{ id }} }} }}"),
            format!("query {{ projects(filter: {{accessibleTeams: {{neq: \"{team}\"}}}}) {{ nodes {{ id }} }} }}"),
        ] {
            assert!(preflight(&q, &Value::Null).is_err(), "{q}");
        }
        let by_var = "query($f: ProjectFilter) { projects(filter: $f) { nodes { id } } }";
        assert!(
            preflight(
                by_var,
                &json!({ "f": { "accessibleTeams": { "some": { "key": { "eq": team } } } } })
            )
            .is_err(),
            "{team}"
        );
    }
    assert!(preflight(
        "query { projects(filter: {accessibleTeams: {some: {key: {eq: \"ENG\"}}}}) { nodes { id } } }",
        &Value::Null
    )
    .is_ok());
}

/// 7: a mutation's selection set can't read another team either.
#[test]
fn a_mutation_selection_cannot_read_lp_or_pro() {
    for team in ["LP", "PRO"] {
        let q = format!(
            "mutation {{ issueCreate(input: {{teamId: \"ENG\", title: \"x\"}}) {{ success issue {{ team {{ issues(filter: {{team: {{key: {{eq: \"{team}\"}}}}}}) {{ nodes {{ id }} }} }} }} }} }}"
        );
        assert!(preflight(&q, &Value::Null).is_err(), "{team}");
        let q = format!(
            "mutation {{ issueCreate(input: {{teamId: \"ENG\", title: \"x\"}}) {{ success x: team(id: \"{team}\") {{ id }} }} }}"
        );
        assert!(preflight(&q, &Value::Null).is_err(), "{team}");
    }
}

// ---- R3 round 2 (2026-11-14) ---------------------------------------------------

/// The refusal text of a call, which must hold `why`.
fn refusal(q: &str, v: Value, why: &str) {
    let error = preflight(q, &v).expect_err(q);
    assert!(error.contains(why), "{q}: wanted {why:?} in {error}");
    assert!(error.contains("Nothing was sent"), "{error}");
}

#[test]
fn upserts_and_other_create_spellings_are_judged_like_creates() {
    for team in ["LP", "PRO"] {
        for field in [
            "customerUpsert",
            "customerNeedCreate",
            "createThing",
            "initiativeUpsert",
        ] {
            // Named in the LP or PRO context, and with the team in the input.
            refusal(
                &format!("mutation {{ {field}(input: {{name: \"x\", domains: [\"{team}.example\"]}}) {{ success }} }}"),
                Value::Null,
                "workspace-wide object",
            );
            refusal(
                &format!("mutation($i: X!) {{ {field}(input: $i) {{ success }} }}"),
                json!({ "i": { "name": team } }),
                "workspace-wide object",
            );
            refusal(
                &format!("mutation {{ {field}(input: {{name: \"x\", teamId: \"{team}\"}}) {{ success }} }}"),
                Value::Null,
                "Engineering",
            );
        }
    }
    assert!(preflight(
        "mutation { issueCreate(input: {teamId: \"ENG\", title: \"x\"}) { success } }",
        &Value::Null
    )
    .is_ok());
}

#[test]
fn team_named_flags_are_accepted_and_team_named_ids_are_not() {
    for q in [
        "query { projects(includeSubTeams: true) { nodes { id } } }",
        "query { teams(includeArchived: false) { nodes { id } } }",
        "mutation { issueCreate(input: {teamId: \"ENG\", title: \"x\", replaceTeamLabels: false}) { success } }",
        "mutation { issueLabelCreate(input: {teamId: \"ENG\", name: \"x\", allPublicTeams: false}) { success } }",
    ] {
        assert!(preflight(q, &Value::Null).is_ok(), "{q}");
    }
    assert!(preflight(
        "query($b: Boolean) { projects(includeSubTeams: $b) { nodes { id } } }",
        &json!({ "b": true })
    )
    .is_ok());
    assert!(preflight(
        "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }",
        &json!({ "i": { "teamId": "ENG", "title": "x", "replaceTeamLabels": false } })
    )
    .is_ok());
    for team in ["LP", "PRO"] {
        refusal(
            &format!("query {{ projects(includeSubTeams: \"{team}\") {{ nodes {{ id }} }} }}"),
            Value::Null,
            "a read may not name a team",
        );
    }
}

#[test]
fn a_mutations_variables_are_walked_for_any_team_named_key() {
    for team in ["LP", "PRO"] {
        refusal(
            "mutation($i: IssueCreateInput!) { issueCreate(input: $i) { success } }",
            json!({ "i": { "teamId": "ENG", "title": "x", "accessibleTeams": { "some": { "key": { "eq": team } } } } }),
            "Engineering",
        );
    }
}

#[test]
fn administrable_teams_filters_are_held_like_teams() {
    for team in ["LP", "PRO"] {
        refusal(
            &format!("query {{ administrableTeams(filter: {{key: {{eq: \"{team}\"}}}}) {{ nodes {{ id }} }} }}"),
            Value::Null,
            "a read may not name a team",
        );
        refusal(
            "query($f: TeamFilter) { administrableTeams(filter: $f) { nodes { id } } }",
            json!({ "f": { "name": { "eq": team } } }),
            "a read may not name a team",
        );
    }
    refusal(
        "query { administrableTeams(filter: {name: {eq: \"x\"}}) { nodes { id } } }",
        Value::Null,
        "a read may not name a team",
    );
}

/// The earlier rounds' bypasses, now asserting why each is refused.
#[test]
fn each_bypass_is_refused_for_its_own_reason() {
    refusal(
        "mutation { templateCreate(input: {name: \"x\", templateData: {teamId: \"ENG\"}}) { success } }",
        Value::Null,
        "workspace-wide object",
    );
    refusal(
        "mutation { initiativeCreate(input: {name: \"x\"}) { success } }",
        Value::Null,
        "workspace-wide object",
    );
    refusal(
        "query($t: String = \"LP\") { issues(filter: {team: {key: {eq: $t}}}) { nodes { id } } }",
        json!({}),
        "default that holds a literal",
    );
    refusal(
        "query($f: TeamFilter) { issues(filter: {team: $f}) { nodes { id } } }",
        json!({ "f": { "key": { "eq": "ENG" }, "or": [{ "key": { "eq": "PRO" } }] } }),
        "a read may not name a team",
    );
    refusal(
        "query { issues(filter: {team: {key: {eq: \"LP\"}}}) { nodes { id } } }",
        Value::Null,
        "Engineering",
    );
    refusal(
        "mutation($i: ProjectUpdateInput!) { projectUpdate(id: \"p1\", input: $i) { success } }",
        json!({ "i": { "teamIds": [] } }),
        "empty teamIds",
    );
}
