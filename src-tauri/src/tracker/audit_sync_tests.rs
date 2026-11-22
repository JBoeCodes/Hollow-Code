//! X2: every tracker write through the IPC edit paths and the agent paths
//! (actions, the task broker) leaves exactly one audit row; a write outside
//! the team is refused before any request and leaves a failed row; an audit
//! error never fails the write.

use super::*;
use crate::audit::{self, kind, AuditEntry, AuditFilter};

const MUTATIONS: [&str; 6] = [
    "IssueCreate",
    "IssueUpdate",
    "IssueArchive",
    "CommentCreate",
    "ProjectCreate",
    "ProjectUpdate",
];

fn mutations(f: &Fixture) -> usize {
    f.server
        .requests()
        .iter()
        .filter(|r| {
            r.operation()
                .as_deref()
                .is_some_and(|op| MUTATIONS.contains(&op))
        })
        .count()
}

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

fn title_edit(f: &Fixture, title: &str) -> TrackerEdit {
    TrackerEdit {
        id: "i1".into(),
        values: BTreeMap::from([("title".into(), json!(title))]),
        expected: mirror::local_values(&f.task()).unwrap(),
    }
}

#[tokio::test]
async fn every_person_and_agent_write_path_leaves_exactly_one_record() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.sync().await.unwrap();
    let writes = |f: &Fixture| rows(f, kind::TRACKER_WRITE);
    assert!(writes(&f).is_empty());

    // A person's comment, and its retry: one row.
    api::comment_with(&f.config, &f.adapter(), "i1", "Looks good.", "c-1")
        .await
        .unwrap();
    api::comment_with(&f.config, &f.adapter(), "i1", "Looks good.", "c-1")
        .await
        .unwrap();
    let got = writes(&f);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].action.as_deref(), Some("comment"));
    assert_eq!(got[0].asked_by.as_deref(), Some("person"));
    assert_eq!(got[0].issue_id.as_deref(), Some("i1"));
    assert_eq!(got[0].outcome.as_deref(), Some("ok"));
    assert!(got[0].session_key.is_none());

    // An agent action's comment: its session asked.
    audit::asking(
        "s-20261110-agent001".into(),
        api::comment_with(&f.config, &f.adapter(), "i1", "Shipped.", "c-2"),
    )
    .await
    .unwrap();
    let got = writes(&f);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].asked_by.as_deref(), Some("s-20261110-agent001"));
    assert_eq!(got[0].session_key.as_deref(), Some("s-20261110-agent001"));

    // The issue editor's edit: no row until it is sent, then one, with the
    // field before and after.
    let old = frontmatter::parse(&f.task())
        .get("title")
        .unwrap()
        .to_string();
    edit(&f.config, title_edit(&f, "Audited title"))
        .await
        .unwrap();
    assert_eq!(writes(&f).len(), 2);
    f.sync().await.unwrap();
    assert_eq!(f.updates(), 1);
    let got = writes(&f);
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].action.as_deref(), Some("update"));
    assert_eq!(got[0].asked_by.as_deref(), Some("person"));
    let fields = got[0].fields.as_ref().unwrap();
    assert_eq!(fields["before"]["title"], json!(old));
    assert_eq!(fields["after"]["title"], json!("Audited title"));

    // An agent's edit through the task broker: its run asked.
    let before = f.task();
    audit::asking_sync("s-20261110-agent002", || {
        crate::task_broker::dispatch_companion(
            f.root(), "fixture-device", "Fixture phone", "audit-edit", "update_task",
            &json!({"ref": "vault:tasks/tracker-i1.md", "expectedHash": vault_tasks::content_hash(&before),
                "fields": {"title": "Edited by an agent"}}),
            Local::now(),
        )
    })
    .unwrap();
    assert_eq!(writes(&f).len(), 3);
    f.sync().await.unwrap();
    let got = writes(&f);
    assert_eq!(got.len(), 4);
    assert_eq!(got[0].asked_by.as_deref(), Some("s-20261110-agent002"));

    // A create, a project create and an archive: one row each.
    let input = NewIssue {
        title: "One audited create".into(),
        ..Default::default()
    };
    api::create_with(&f.config, &f.adapter(), "req-a", input.clone())
        .await
        .unwrap();
    api::create_with(&f.config, &f.adapter(), "req-a", input)
        .await
        .unwrap();
    let project = TrackerNewProject {
        name: "One audited project".into(),
        ..Default::default()
    };
    api::create_project_with(&f.config, &f.adapter(), project, Some("proj-a"))
        .await
        .unwrap();
    api::archive_with(&f.config, &f.adapter(), "i1")
        .await
        .unwrap();
    let got = writes(&f);
    assert_eq!(got.len(), 7);
    let actions: Vec<&str> = got[..3]
        .iter()
        .filter_map(|e| e.action.as_deref())
        .collect();
    assert_eq!(actions, ["archive", "create", "create"]);
    // Every request that wrote to the tracker has its row.
    assert_eq!(mutations(&f), got.len());
    assert!(rows(&f, kind::TRACKER_WRITE_FAILED).is_empty());
    assert!(audit::verify(f.root()).unwrap().ok);
}

#[tokio::test]
async fn nothing_outside_the_team_is_written_and_each_refusal_is_recorded() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.sync().await.unwrap();
    let sent = mutations(&f);
    // An issue and a project of another team (the mirror never holds them).
    let lp = "i-lp-0001";
    assert!(api::comment_with(&f.config, &f.adapter(), lp, "Hi", "c-lp")
        .await
        .is_err());
    assert!(api::archive_with(&f.config, &f.adapter(), lp)
        .await
        .is_err());
    let mut other = title_edit(&f, "Not ours");
    other.id = lp.into();
    assert!(edit(&f.config, other).await.is_err());
    let input = NewIssue {
        title: "Into another team".into(),
        project_id: Some("p-lp-0001".into()),
        ..Default::default()
    };
    assert!(api::create_with(&f.config, &f.adapter(), "req-lp", input)
        .await
        .is_err());
    let fields = serde_json::Map::from_iter([("title".to_string(), json!("Not ours"))]);
    assert!(
        seams::edit_task(f.root(), &format!("tasks/tracker-{lp}.md"), "hash", &fields).is_err()
    );
    f.sync().await.unwrap();
    assert_eq!(mutations(&f), sent, "the tracker saw no write");
    assert!(rows(&f, kind::TRACKER_WRITE).is_empty());
    let failed = rows(&f, kind::TRACKER_WRITE_FAILED);
    let mut actions: Vec<&str> = failed.iter().filter_map(|e| e.action.as_deref()).collect();
    actions.sort_unstable();
    assert_eq!(
        actions,
        ["archive", "comment", "create", "update", "update"]
    );
    assert!(failed
        .iter()
        .all(|e| e.outcome.as_deref() == Some("failed") && e.detail.is_some()));
}

#[tokio::test]
async fn an_audit_error_never_fails_the_tracker_write() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    audit::tamper(f.root(), "DROP TABLE audit_log;");
    let errors = audit::errors();
    let out = api::comment_with(&f.config, &f.adapter(), "i1", "Still posts.", "c-err")
        .await
        .unwrap();
    assert_eq!(out.note, "Comment posted.");
    assert_eq!(comment_creates(&f), 1);
    assert!(audit::errors() > errors);
}

/// Review fix 1: an edit the tracker keeps refusing is one failed row
/// however many passes retry it, and its later success one ok row.
#[tokio::test]
async fn a_refused_edit_retried_every_pass_is_one_failed_row_then_one_ok_row() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    f.remote.lock().unwrap().reject_updates = true;
    edit(&f.config, title_edit(&f, "Refused for now"))
        .await
        .unwrap();
    for _ in 0..3 {
        let _ = f.sync().await;
    }
    assert!(f.updates() >= 2, "each pass retried the send");
    let failed = rows(&f, kind::TRACKER_WRITE_FAILED);
    assert_eq!(failed.len(), 1);
    assert!(failed[0]
        .request_key
        .as_deref()
        .is_some_and(|k| k.starts_with("update:i1:")));
    f.remote.lock().unwrap().reject_updates = false;
    f.sync().await.unwrap();
    f.sync().await.unwrap();
    assert_eq!(rows(&f, kind::TRACKER_WRITE).len(), 1);
    assert_eq!(rows(&f, kind::TRACKER_WRITE_FAILED).len(), 1);
}

/// Review fix 5: a refusal before the audited write function (the tracker
/// off, a request that doesn't validate) still leaves its failed row.
#[tokio::test]
async fn a_refusal_in_the_outer_write_functions_is_recorded() {
    let f = Fixture::new().await;
    f.sync().await.unwrap();
    let mut off = f.config.clone();
    off.tracker.provider = None;
    assert!(comment(&off, "i1", "Hi", "c-off").await.is_err());
    assert!(archive(&off, "i1").await.is_err());
    assert!(
        create_project(&off, TrackerNewProject::default(), Some("p-off"))
            .await
            .is_err()
    );
    let task = TrackerNewTask {
        title: "Into a project nobody has".into(),
        project_id: Some("p-unknown-0001".into()),
        ..Default::default()
    };
    assert!(create_named(&f.config, task, Some("t-bad")).await.is_err());
    let mut actions: Vec<String> = rows(&f, kind::TRACKER_WRITE_FAILED)
        .into_iter()
        .filter_map(|e| e.action)
        .collect();
    actions.sort_unstable();
    assert_eq!(actions, ["archive", "comment", "create", "create"]);
    assert_eq!(mutations(&f), 0);
}

/// Review fix 8: a person's later edit of the file is never filed under
/// the agent that staged an earlier one.
#[tokio::test]
async fn a_person_edit_after_an_agent_edit_is_the_persons() {
    let f = Fixture::new().await;
    seams::register_running(&f.config);
    f.sync().await.unwrap();
    let before = f.task();
    audit::asking_sync("s-20261110-agent003", || {
        crate::task_broker::dispatch_companion(
            f.root(), "fixture-device", "Fixture phone", "agent-edit", "update_task",
            &json!({"ref": "vault:tasks/tracker-i1.md", "expectedHash": vault_tasks::content_hash(&before),
                "fields": {"title": "The agent's title"}}),
            Local::now(),
        )
    })
    .unwrap();
    f.local_edit("title", "The person's title");
    f.sync().await.unwrap();
    let got = rows(&f, kind::TRACKER_WRITE);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].asked_by.as_deref(), Some("person"));
    assert_eq!(
        got[0].fields.as_ref().unwrap()["after"]["title"],
        "The person's title"
    );
}
