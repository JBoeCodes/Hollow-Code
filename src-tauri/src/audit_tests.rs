//! X2: the audit record's store: append-only, hash-chained, filtered reads.

use super::*;

fn vault() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn row(kind: &str, session: &str, issue: Option<&str>, workspace: Option<&str>) -> AuditEntry {
    AuditEntry {
        kind: kind.into(),
        session_key: Some(session.into()),
        asked_by: Some("person".into()),
        issue_id: issue.map(str::to_string),
        issue_key: issue.map(|i| i.replace('i', "AI-")),
        workspace_id: workspace.map(str::to_string),
        outcome: Some("ok".into()),
        ..AuditEntry::default()
    }
}

fn all(v: &Path) -> Vec<AuditEntry> {
    query(
        v,
        &AuditFilter {
            limit: Some(MAX_LIMIT),
            ..AuditFilter::default()
        },
    )
    .unwrap()
}

#[test]
fn rows_chain_by_hash_and_update_or_delete_is_refused() {
    let dir = vault();
    let v = dir.path();
    let first = record(v, row(kind::RUN_STARTED, "s-1", Some("i1"), None)).unwrap();
    let second = record(v, row(kind::RUN_ENDED, "s-1", Some("i1"), None)).unwrap();
    assert_eq!(first.prev_hash, "");
    assert_eq!(second.prev_hash, first.hash);
    assert_eq!(second.hash.len(), 64);
    assert!(first.id < second.id, "ids are time-ordered");
    assert!(first.id.starts_with('a') && first.id.len() == 29);
    for sql in [
        "UPDATE audit_log SET detail = 'changed'",
        "DELETE FROM audit_log",
    ] {
        let error = session_store::with_write(v, "audit edit", |tx| {
            tx.execute(sql, []).map_err(|e| e.to_string())
        })
        .unwrap_err();
        assert!(error.contains("append-only"), "{error}");
    }
    assert_eq!(all(v).len(), 2);
    let check = verify(v).unwrap();
    assert!(
        check.ok && check.rows == 2 && check.first_bad.is_none(),
        "{check:?}"
    );
}

#[test]
fn verify_names_an_edited_row_and_a_gap() {
    let dir = vault();
    let v = dir.path();
    let ids: Vec<String> = (0..4)
        .map(|n| {
            record(v, row(kind::TRACKER_WRITE, &format!("s-{n}"), None, None))
                .unwrap()
                .id
        })
        .collect();
    tamper(
        v,
        &format!(
            "UPDATE audit_log SET detail = 'quietly changed' WHERE id = '{}';",
            ids[1]
        ),
    );
    let check = verify(v).unwrap();
    assert!(!check.ok);
    assert_eq!(check.first_bad.as_deref(), Some(ids[1].as_str()));
    assert_eq!(check.problem.as_deref(), Some("edited"));

    let dir = vault();
    let v = dir.path();
    let ids: Vec<String> = (0..4)
        .map(|n| {
            record(v, row(kind::TRACKER_WRITE, &format!("s-{n}"), None, None))
                .unwrap()
                .id
        })
        .collect();
    tamper(
        v,
        &format!("DELETE FROM audit_log WHERE id = '{}';", ids[2]),
    );
    let check = verify(v).unwrap();
    assert_eq!(check.rows, 3);
    assert_eq!(check.first_bad.as_deref(), Some(ids[3].as_str()));
    assert_eq!(check.problem.as_deref(), Some("gap"));
}

#[test]
fn reads_filter_by_issue_session_workspace_kind_since_and_limit_newest_first() {
    let dir = vault();
    let v = dir.path();
    record(v, row(kind::RUN_STARTED, "s-a", Some("i1"), Some("proj")));
    record(v, row(kind::ISSUE_LINKED, "s-a", Some("i1"), Some("proj")));
    record(v, row(kind::RUN_STARTED, "s-b", Some("i2"), Some("other")));
    record(v, row(kind::RUN_ENDED, "s-a", Some("i1"), Some("proj")));
    let q = |filter: AuditFilter| query(v, &filter).unwrap();
    let issue = q(AuditFilter {
        issue: Some("i1".into()),
        ..AuditFilter::default()
    });
    let kinds: Vec<&str> = issue.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [kind::RUN_ENDED, kind::ISSUE_LINKED, kind::RUN_STARTED]
    );
    // The key matches too, case-insensitively.
    let by_key = q(AuditFilter {
        issue: Some("ai-1".into()),
        ..AuditFilter::default()
    });
    assert_eq!(by_key.len(), 3);
    assert_eq!(
        q(AuditFilter {
            session: Some("s-b".into()),
            ..AuditFilter::default()
        })
        .len(),
        1
    );
    assert_eq!(
        q(AuditFilter {
            workspace: Some("proj".into()),
            kind: Some(kind::RUN_STARTED.into()),
            ..AuditFilter::default()
        })
        .len(),
        1
    );
    assert_eq!(
        q(AuditFilter {
            limit: Some(2),
            ..AuditFilter::default()
        })
        .len(),
        2
    );
    assert!(q(AuditFilter {
        since: Some("2999-01-01T00:00:00Z".into()),
        ..AuditFilter::default()
    })
    .is_empty());
    assert_eq!(
        q(AuditFilter {
            since: Some("2000-01-01T00:00:00Z".into()),
            ..AuditFilter::default()
        })
        .len(),
        4
    );
    assert!(query(
        v,
        &AuditFilter {
            kind: Some("deleted_everything".into()),
            ..AuditFilter::default()
        }
    )
    .unwrap_err()
    .contains("Unknown audit kind"));
}

#[test]
fn a_write_with_a_request_key_is_one_row_per_outcome() {
    let dir = vault();
    let v = dir.path();
    let write = |outcome: &'static str| Write {
        action: "comment",
        asked_by: "person".into(),
        issue_id: Some("i1"),
        request_key: Some("c-1"),
        outcome,
        ..Write::default()
    };
    tracker_write(v, write("failed"));
    tracker_write(v, write("failed"));
    tracker_write(v, write("ok"));
    tracker_write(v, write("ok"));
    let rows = all(v);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].kind, kind::TRACKER_WRITE);
    assert_eq!(rows[1].kind, kind::TRACKER_WRITE_FAILED);
    assert_eq!(rows[0].request_key.as_deref(), Some("c-1"));
    assert!(rows[0].fields.is_none(), "the key is its own column");
}

#[test]
fn an_audit_database_error_is_counted_and_never_returned() {
    let dir = vault();
    let v = dir.path();
    record(v, row(kind::RUN_STARTED, "s-1", None, None));
    tamper(v, "DROP TABLE audit_log;");
    let before = errors();
    assert!(record(v, row(kind::RUN_ENDED, "s-1", None, None)).is_none());
    assert!(errors() > before);
}

#[test]
fn the_asker_is_the_enclosing_scope_else_the_person() {
    assert_eq!(asker(), "person");
    assert_eq!(asking_sync("s-agent", asker), "s-agent");
    assert_eq!(asker(), "person");
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let inside = rt.block_on(asking("automation:nightly".into(), async { asker() }));
    assert_eq!(inside, "automation:nightly");
    assert_eq!(caller_asker("chat", " s-1 "), "s-1");
    assert_eq!(caller_asker("workflow", "nightly"), "automation:nightly");
    assert_eq!(caller_asker("ask", "run-9"), "ask:run-9");
    assert!(is_session("s-1") && !is_session("person") && !is_session("ask:run-9"));
}

#[test]
fn verify_sees_a_deleted_tail() {
    let dir = vault();
    let v = dir.path();
    let ids: Vec<String> = (0..3)
        .map(|n| {
            record(v, row(kind::TRACKER_WRITE, &format!("s-{n}"), None, None))
                .unwrap()
                .id
        })
        .collect();
    assert!(verify(v).unwrap().ok);
    tamper(
        v,
        &format!("DELETE FROM audit_log WHERE id = '{}';", ids[2]),
    );
    let check = verify(v).unwrap();
    assert!(!check.ok);
    assert_eq!(check.rows, 2);
    assert_eq!(check.problem.as_deref(), Some("tail"));
    assert_eq!(check.first_bad.as_deref(), Some(ids[2].as_str()));
}

#[test]
fn free_text_is_redacted_and_a_row_is_bounded() {
    let dir = vault();
    let v = dir.path();
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let mut fields = json!({
        "after": {"title": format!("Rotate {secret}")},
        "many": (0..100).collect::<Vec<i32>>(),
        "deep": {"a": {"b": {"c": {"d": 1}}}},
    });
    fields["k".repeat(300).as_str()] = json!(-0.0);
    let stored = record(
        v,
        AuditEntry {
            kind: kind::TRACKER_WRITE_FAILED.into(),
            detail: Some(format!("The tracker said no to {secret}")),
            fields: Some(fields),
            request_key: Some("r".repeat(500)),
            ..AuditEntry::default()
        },
    )
    .unwrap();
    let fields = stored.fields.clone().unwrap();
    assert!(!stored.detail.as_deref().unwrap().contains(secret));
    assert!(!fields.to_string().contains(secret));
    assert_eq!(fields["many"].as_array().unwrap().len(), MAX_ITEMS + 1);
    assert_eq!(fields["deep"]["a"]["b"]["c"], "(nested)");
    assert!(fields
        .as_object()
        .unwrap()
        .keys()
        .all(|k| k.chars().count() <= 65));
    let long_key = fields
        .as_object()
        .unwrap()
        .keys()
        .find(|k| k.starts_with("kkk"))
        .unwrap();
    assert_eq!(fields[long_key.as_str()], json!(0), "-0.0 is stored as 0");
    assert!(stored.request_key.as_deref().unwrap().chars().count() <= 201);
    // A row too big even so keeps its columns and gives up its fields.
    let huge = record(
        v,
        AuditEntry {
            kind: kind::TRACKER_WRITE.into(),
            fields: Some(json!({
                "a": vec!["x".repeat(500); 40],
                "b": vec!["y".repeat(500); 40],
            })),
            ..AuditEntry::default()
        },
    )
    .unwrap();
    assert_eq!(huge.fields, Some(json!({"truncated": true})));
    assert!(
        canonical(&huge, Some(&huge.fields.as_ref().unwrap().to_string())).len() <= MAX_ROW_BYTES
    );
    assert!(verify(v).unwrap().ok);
}

#[test]
fn numbers_are_stored_finite_and_verify_rejects_an_infinite_one() {
    let dir = vault();
    let v = dir.path();
    let zero = record(
        v,
        AuditEntry {
            kind: kind::RUN_ENDED.into(),
            cost_usd: Some(-0.0),
            ..AuditEntry::default()
        },
    )
    .unwrap();
    assert_eq!(zero.cost_usd.map(f64::to_bits), Some(0f64.to_bits()));
    let inf = record(
        v,
        AuditEntry {
            kind: kind::RUN_ENDED.into(),
            cost_usd: Some(f64::INFINITY),
            ..AuditEntry::default()
        },
    )
    .unwrap();
    assert_eq!(inf.cost_usd, None);
    assert!(verify(v).unwrap().ok);
    tamper(
        v,
        &format!(
            "UPDATE audit_log SET cost_usd = 9e999 WHERE id = '{}';",
            inf.id
        ),
    );
    let check = verify(v).unwrap();
    assert_eq!(check.first_bad.as_deref(), Some(inf.id.as_str()));
    assert_eq!(check.problem.as_deref(), Some("edited"));
}

/// Q-R35: a field whose key names a credential loses its text even when the
/// value has no credential shape the redaction pass would know.
#[test]
fn a_secret_named_field_is_masked_by_its_key() {
    let dir = vault();
    let stored = record(
        dir.path(),
        AuditEntry {
            kind: kind::TRACKER_WRITE.into(),
            fields: Some(json!({
                "token": "plainvalue",
                "after": {"apiKey": "short1", "client_secret": "abc", "title": "plainvalue"},
                "input_tokens": 12,
                "issue_key": "ENG-12",
            })),
            ..AuditEntry::default()
        },
    )
    .unwrap();
    let fields = stored.fields.unwrap();
    assert_eq!(fields["token"], "[redacted]");
    assert_eq!(fields["after"]["apiKey"], "[redacted]");
    assert_eq!(fields["after"]["client_secret"], "[redacted]");
    assert_eq!(fields["after"]["title"], "plainvalue");
    assert_eq!(fields["input_tokens"], 12);
    assert_eq!(fields["issue_key"], "ENG-12");
    for key in [
        "deviceToken",
        "Authorization",
        "DB_PASSWORD",
        "session-cookie",
    ] {
        assert!(secret_key(key), "{key}");
    }
    for key in ["title", "input_tokens", "issue_key", "key", "state"] {
        assert!(!secret_key(key), "{key}");
    }
}
