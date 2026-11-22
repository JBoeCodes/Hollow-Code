//! T3b: which Waiting on you items get a tracker copy, where, once, and how
//! a copy is closed.

use super::*;
use crate::audit::{self, kind};
use serde_json::json;
use std::path::PathBuf;

const T0: &str = "2026-11-15T10:00:00.000Z";

fn at(minutes: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
        + chrono::Duration::minutes(minutes)
}

fn running(mode: AgentWork, hol: bool) -> Running {
    Running {
        mode,
        hol,
        linear: true,
    }
}

struct Vault {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

/// A vault with a tracked project `proj` (AI project p1) and an untracked
/// workspace `notes`.
fn vault() -> Vault {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (id, project) in [
        ("proj", "tracker: linear\ntracker_project: p1\n"),
        ("notes", ""),
    ] {
        let folder = root.join("workspaces").join(id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("WORKSPACE.md"),
            format!("---\nhollow_workspace: 1\ntitle: {id}\n---\n"),
        )
        .unwrap();
        std::fs::write(
            folder.join("PROJECT.md"),
            format!("---\ntitle: {id}\n{project}---\n"),
        )
        .unwrap();
    }
    Vault { _dir: dir, root }
}

impl Vault {
    /// One pass at `minutes` after T0, every close allowed.
    fn pass(
        &self,
        mode: &Running,
        items: &[NeedsYouItem],
        complete: bool,
        minutes: i64,
    ) -> Reconciled {
        reconcile_items(&self.root, mode, items, complete, at(minutes), &|_| true)
    }
    /// The mode starts before T0, so items at T0 appeared after it.
    fn start(&self, mode: &Running) {
        self.pass(mode, &[], true, -1);
    }
    fn writes(&self, key: &str) -> Vec<audit::AuditEntry> {
        [kind::TRACKER_WRITE, kind::TRACKER_WRITE_FAILED]
            .into_iter()
            .flat_map(|k| audit::rows_of(&self.root, key, k))
            .collect()
    }
    fn events(&self) -> Vec<(String, WorkEvent)> {
        work::pending_events(&self.root, 100)
            .unwrap()
            .into_iter()
            .map(|e| (e.session_key, e.event))
            .collect()
    }
}

fn item_at(
    kind: &str,
    source: &str,
    title: &str,
    workspace: Option<&str>,
    minutes: i64,
) -> NeedsYouItem {
    NeedsYouItem {
        at: Some(at(minutes).to_rfc3339()),
        ..item(kind, source, title, workspace)
    }
}

fn item(kind: &str, source: &str, title: &str, workspace: Option<&str>) -> NeedsYouItem {
    NeedsYouItem {
        id: format!("{kind}:{source}"),
        kind: kind.into(),
        source_id: source.into(),
        title: title.into(),
        detail: Some("Private detail the person wrote".into()),
        state: "pending".into(),
        at: Some(T0.into()),
        workspace_id: workspace.map(str::to_string),
        session_key: Some("s-20261115-ask00001".into()),
        path: None,
        category: None,
        link: None,
    }
}

#[test]
fn an_item_that_waited_is_filed_once_in_hol_with_the_label_and_the_viewer() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    let ask = item(
        "question",
        "q1",
        "Which venue for the offsite?",
        Some("notes"),
    );
    // Answered within ten minutes: never an issue.
    assert_eq!(
        v.pass(&on, std::slice::from_ref(&ask), true, 5),
        Reconciled::default()
    );
    assert!(v.events().is_empty());
    // Waited ten minutes: one copy. Again, and after a "restart": none more.
    for _ in 0..3 {
        v.pass(&on, std::slice::from_ref(&ask), true, 11);
    }
    let events = v.events();
    assert_eq!(events.len(), 1);
    let (
        key,
        WorkEvent::Create {
            create_key: ck,
            project_id,
            title,
            description,
            team_id,
            labels,
            assign_viewer,
            due,
        },
    ) = &events[0]
    else {
        panic!("a create")
    };
    assert_eq!(key, "waiting:question:q1");
    assert_eq!(ck, &create_key("question:q1"));
    assert_eq!(project_id, "");
    assert_eq!(team_id.as_deref(), Some(linear::PERSONAL_TEAM_ID));
    assert_eq!(labels, &vec!["needs-review".to_string()]);
    assert!(*assign_viewer);
    assert_eq!(*due, None);
    // HOL is the person's own team: the item's own title.
    assert_eq!(title, "Which venue for the offsite?");
    assert!(description.contains("A question is waiting on you in Hollow (workspace notes)"));
    assert!(!description.contains("Private detail"));
    let link = work::link_for(&v.root, "waiting:question:q1").unwrap();
    assert_eq!(
        (link.linked_by.as_str(), link.created_issue, link.issue_id),
        ("waiting", true, None)
    );
    // Approvals, failures and the restart offer stay in Hollow only.
    let other = [
        item("approval", "a1", "Run it?", None),
        item("failure", "n1", "Sync failed", None),
        item("interrupted", "x", "Resume 2 chats", None),
        ask.clone(),
    ];
    v.pass(&on, &other, true, 30);
    assert_eq!(v.events().len(), 1);
}

#[test]
fn a_team_item_in_ai_carries_nothing_of_the_items_words() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    let private = "Approve Maria's raise to $28/h?";
    let ask = item("question", "q-raise", private, Some("proj"));
    v.pass(&on, std::slice::from_ref(&ask), true, 20);
    let events = v.events();
    let WorkEvent::Create {
        project_id,
        team_id,
        labels,
        title,
        description,
        ..
    } = &events[0].1
    else {
        panic!("a create")
    };
    assert_eq!((project_id.as_str(), team_id), ("p1", &None));
    assert_eq!(labels, &vec![LABEL.to_string()]);
    assert_eq!(title, "A question waiting on you");
    assert!(!description.contains("Maria") && !description.contains("$28"));
    // Answered by choosing an option that repeats the words: team ENG's
    // comment says only that it was answered; HOL's may name the option.
    crate::session_store::start_turn(
        &v.root,
        "s-20261115-ask00001",
        "claude",
        &v.root.to_string_lossy(),
        "run-1",
        "ask",
    )
    .unwrap();
    let arguments: Result<serde_json::Value, String> = Ok(json!({"questions": [{"id": "raise",
        "header": "Raise", "prompt": private, "options": [
            {"id": "yes", "label": "Approve Maria's raise to $28/h"}, {"id": "no", "label": "Not now"}]}]}));
    let card = crate::interactions::record(
        &v.root,
        &crate::interactions::NewInteraction {
            session_key: "s-20261115-ask00001",
            turn_ord: 1,
            run_id: "run-1",
            agent: crate::agent::AgentKind::Claude,
            native_session_id: None,
            tool_use_id: "toolu-raise",
            source: crate::interactions::InteractionSource::Mcp,
            arguments: &arguments,
            cwd: &v.root,
            vault_root: &v.root,
        },
    )
    .unwrap()
    .interaction;
    crate::interactions::resolve(
        &v.root,
        &card.id,
        &json!({"answers": {"raise": {"selected": ["yes"]}}}),
        None,
    )
    .unwrap();
    let id = format!("question:{}", card.id);
    let shared = answer_of(&v.root, &id, true).unwrap();
    assert_eq!(
        shared,
        Ended {
            text: "Answered in Hollow.".into(),
            positive: true
        }
    );
    assert!(answer_of(&v.root, &id, false)
        .unwrap()
        .text
        .contains("Maria"));
    // The whole close for an AI copy: the comment never has the words.
    let key = link_key(&ask.id);
    work::fill_created(&v.root, &key, "i-raise", "ENG-7").unwrap();
    let link = work::link_for(&v.root, &key).unwrap();
    assert!(close(&v.root, &link, "Answered in Hollow.".into()));
    let comment = work::render_resolved(
        &v.events().last().unwrap().1,
        work::linked_by::WAITING,
        "work-0",
    );
    assert!(
        !comment.contains("Maria") && !comment.contains("$28"),
        "{comment}"
    );
}

#[test]
fn only_items_that_appear_after_the_switch_on_are_copied() {
    let v = vault();
    let dry = running(AgentWork::DryRun, true);
    let on = running(AgentWork::On, true);
    // Waiting since before dry run began: never copied, not even previewed.
    let old = item_at("review", "old", "Old", None, -60);
    v.start(&dry);
    v.pass(&dry, std::slice::from_ref(&old), true, 20);
    assert!(v.writes(&link_key(&old.id)).is_empty());
    // Seen in the dry run, then the switch goes on: the backlog stays put.
    let seen = item_at("review", "seen", "Seen", None, 0);
    v.pass(&dry, std::slice::from_ref(&seen), true, 20);
    assert_eq!(v.writes(&link_key(&seen.id)).len(), 1);
    v.pass(&on, &[old.clone(), seen.clone()], true, 30);
    assert!(v.events().is_empty());
    // An item with no time can't show when it appeared: never copied.
    let timeless = NeedsYouItem {
        at: None,
        ..item("result", "r", "R", None)
    };
    // One that appears after the switch-on is.
    let fresh = item_at("review", "fresh", "Fresh", None, 31);
    v.pass(&on, &[old, seen, timeless, fresh], true, 45);
    let keys: Vec<String> = v.events().into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, vec!["waiting:review:fresh".to_string()]);
    // Off and on again: a new start, so what waited meanwhile isn't filed.
    let meanwhile = item_at("review", "meanwhile", "M", None, 50);
    v.pass(&running(AgentWork::Off, true), &[], true, 60);
    v.pass(&on, std::slice::from_ref(&meanwhile), true, 80);
    assert_eq!(v.events().len(), 1);
}

#[test]
fn a_dry_run_records_once_off_nothing_and_a_refusal_files_once_hol_is_set_up() {
    let v = vault();
    let dry = running(AgentWork::DryRun, true);
    v.start(&dry);
    let review = item("review", "r1", "A lesson", None);
    let key = link_key(&review.id);
    for _ in 0..2 {
        v.pass(&dry, std::slice::from_ref(&review), true, 15);
    }
    let rows = v.writes(&key);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].detail.as_deref().unwrap().starts_with(
        "Dry run: Hollow would file this waiting review item as an issue in team PER"
    ));
    assert!(v.events().is_empty() && work::link_for(&v.root, &key).is_none());
    // Off: nothing at all.
    let off = item_at("review", "r2", "Another", None, 20);
    v.pass(
        &running(AgentWork::Off, true),
        std::slice::from_ref(&off),
        true,
        40,
    );
    assert!(v.writes(&link_key(&off.id)).is_empty());
    // On without HOL: refused once, under its own key, never filed in AI.
    let no_hol = running(AgentWork::On, false);
    v.pass(&no_hol, &[], true, 41);
    let other = item_at("review", "r3", "Third", None, 42);
    for minutes in [55, 56] {
        v.pass(&no_hol, std::slice::from_ref(&other), true, minutes);
    }
    let refused = v.writes(&link_key(&other.id));
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].kind, kind::TRACKER_WRITE_FAILED);
    assert_eq!(
        refused[0].request_key.as_deref(),
        Some(format!("{}-refused", create_key(&other.id)).as_str())
    );
    assert!(v.events().is_empty());
    // HOL set up: the same item is filed now.
    v.pass(
        &running(AgentWork::On, true),
        std::slice::from_ref(&other),
        true,
        57,
    );
    assert_eq!(v.events().len(), 1);
}

#[test]
fn a_failed_create_is_tried_again_after_an_hour_not_given_up() {
    let v = vault();
    let on = running(AgentWork::On, true);
    // Days before the real clock, so the audit row's own time is later.
    v.pass(&on, &[], true, -100_000);
    let review = item_at("review", "rf", "Retry me", None, -50_000);
    let key = link_key(&review.id);
    assert_eq!(
        v.pass(&on, std::slice::from_ref(&review), true, -49_000)
            .filed,
        1
    );
    // The outbox gave up on it: a failed row under the create key, untied.
    work::record_tracker_write(
        &v.root,
        &work::TrackerWrite {
            session_key: &key,
            issue_id: "",
            action: "create",
            outcome: "failed",
            request_key: Some(&create_key(&review.id)),
            detail: Some("refused".into()),
            ..work::TrackerWrite::default()
        },
    );
    work::drop_link(&v.root, &key).unwrap();
    let failed = v
        .writes(&key)
        .into_iter()
        .find(|r| r.kind == kind::TRACKER_WRITE_FAILED)
        .unwrap();
    let now = DateTime::parse_from_rfc3339(&failed.at)
        .unwrap()
        .with_timezone(&Utc)
        + chrono::Duration::minutes(1);
    let within = reconcile_items(
        &v.root,
        &on,
        std::slice::from_ref(&review),
        true,
        now,
        &|_| true,
    );
    assert_eq!(within.filed, 0);
    let later = now + chrono::Duration::minutes(RETRY_MINUTES + 1);
    let after = reconcile_items(
        &v.root,
        &on,
        std::slice::from_ref(&review),
        true,
        later,
        &|_| true,
    );
    assert_eq!(after.filed, 1);
    assert!(work::link_for(&v.root, &key).is_some());
}

#[test]
fn closes_need_a_positive_signal_or_two_absent_passes_and_a_completed_state() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    let conflict = item(
        "conflict",
        "tracker:i9:title",
        "Title differs",
        Some("proj"),
    );
    let snoozed = crate::review_queue::enqueue(
        &v.root,
        crate::review_queue::NewReviewItem {
            kind: "learning".into(),
            fingerprint: "a".repeat(64),
            payload: json!({"title": "A lesson"}),
            risk: "local_write".into(),
            source: "chat".into(),
            source_path: None,
        },
    )
    .unwrap();
    let review = item("review", &snoozed.id, "A lesson", None);
    v.pass(&on, &[conflict.clone(), review.clone()], true, 30);
    assert_eq!(v.events().len(), 2);
    let closes = |v: &Vault| {
        v.events()
            .into_iter()
            .filter(|(_, e)| matches!(e, WorkEvent::Resolved { .. }))
            .collect::<Vec<_>>()
    };
    // Gone before its issue exists: nothing to close yet.
    v.pass(&on, &[], true, 30);
    assert!(closes(&v).is_empty());
    for (n, it) in [&conflict, &review].into_iter().enumerate() {
        let id = format!("i-{n}");
        work::fill_created(&v.root, &link_key(&it.id), &id, &format!("HOL-{n}")).unwrap();
    }
    // Gone, but a source couldn't be read: nothing, and it doesn't count.
    v.pass(&on, &[], false, 31);
    // A conflict has no store that says it ended: one absent pass isn't enough.
    crate::review_queue::snooze(&v.root, &snoozed.id, "2099-01-01T00:00:00Z").unwrap();
    assert_eq!(v.pass(&on, &[], true, 32).closed, 0);
    // No completed state in its team: the close waits, audited once, retried.
    for minutes in [33, 34] {
        let blocked = reconcile_items(&v.root, &on, &[], true, at(minutes), &|_| false);
        assert_eq!(blocked.closed, 0);
    }
    let waits: Vec<_> = v
        .writes(&link_key(&conflict.id))
        .into_iter()
        .filter(|r| {
            r.request_key
                .as_deref()
                .is_some_and(|k| k.ends_with("-no-completed-state"))
        })
        .collect();
    assert_eq!(waits.len(), 1);
    assert!(work::open_links_by(&v.root, work::linked_by::WAITING)
        .iter()
        .any(|l| l.session_key == link_key(&conflict.id)));
    // Absent again with a state to close in: it closes, once; the snoozed
    // review item keeps its copy open.
    assert_eq!(v.pass(&on, &[], true, 35).closed, 1);
    v.pass(&on, &[], true, 36);
    let done = closes(&v);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].0, link_key(&conflict.id));
    assert_eq!(
        done[0].1,
        WorkEvent::Resolved {
            answer: "Resolved in Hollow.".into()
        }
    );
    // Rejected: its store says so, so it closes at once.
    crate::review_queue::wake(&v.root, &snoozed.id).unwrap();
    crate::review_queue::reject(&v.root, &snoozed.id).unwrap();
    v.pass(&on, &[], true, 40);
    assert!(closes(&v).iter().any(|(k, e)| k == &link_key(&review.id)
        && e == &WorkEvent::Resolved {
            answer: "Rejected in Hollow.".into()
        }));
    // A reappearing item never makes a second issue.
    v.pass(&on, &[conflict], true, 50);
    assert_eq!(
        v.events()
            .iter()
            .filter(|(_, e)| matches!(e, WorkEvent::Create { .. }))
            .count(),
        2
    );
}

#[test]
fn a_link_hollow_did_not_create_is_never_closed() {
    let v = vault();
    let key = link_key("conflict:tracker:i1:title");
    work::insert_link(
        &v.root,
        &work::WorkLink {
            session_key: key.clone(),
            issue_id: Some("i1".into()),
            issue_key: Some("ENG-1".into()),
            workspace_id: None,
            linked_by: work::linked_by::WAITING.into(),
            created_issue: false,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
    )
    .unwrap();
    let on = running(AgentWork::On, true);
    for minutes in [30, 31, 32] {
        assert_eq!(v.pass(&on, &[], true, minutes).closed, 0);
    }
    assert!(v.events().is_empty());
}

#[test]
fn a_question_answer_is_summarised_without_the_persons_own_words() {
    let request = json!({"questions": [{"id": "venue", "options": [
        {"id": "a", "label": "Portland"}, {"id": "b", "label": "Denver"}
    ]}]});
    let chosen = json!({"answers": {"venue": {"selected": ["b"]}}});
    assert_eq!(answer_summary(&request, Some(&chosen)), "Denver");
    let written = json!({"answers": {"venue": {"other": "my sister's flat, code 4412"}}});
    assert_eq!(
        answer_summary(&request, Some(&written)),
        "a written answer (kept in Hollow)"
    );
    assert_eq!(
        answer_summary(
            &json!({}),
            Some(&json!({"accepted": false, "note": "not now"}))
        ),
        "no; a written answer (kept in Hollow)"
    );
    assert_eq!(answer_summary(&json!({}), None), "an answer");
    // A planted secret in an item's title is masked before it is filed.
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    let secret = item(
        "review",
        "r9",
        "Rotate ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        None,
    );
    v.pass(&on, &[secret], true, 20);
    let WorkEvent::Create { title, .. } = &v.events()[0].1 else {
        panic!("a create")
    };
    assert!(!title.contains("AAAAAAAAAAAAAAAA"), "{title}");
}

fn conflict_link(v: &Vault, source: &str) -> String {
    let key = link_key(&format!("conflict:{source}"));
    work::insert_link(
        &v.root,
        &work::WorkLink {
            session_key: key.clone(),
            issue_id: Some(format!("i-{source}")),
            issue_key: Some("ENG-9".into()),
            workspace_id: None,
            linked_by: work::linked_by::WAITING.into(),
            created_issue: true,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
    )
    .unwrap();
    key
}

fn closed(v: &Vault) -> usize {
    v.events()
        .iter()
        .filter(|(_, e)| matches!(e, WorkEvent::Resolved { .. }))
        .count()
}

#[test]
fn two_absent_passes_must_be_a_minute_apart_and_anything_else_starts_over() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    conflict_link(&v, "a");
    let secs = |s: i64| at(30) + chrono::Duration::seconds(s);
    let pass = |s: i64, complete: bool, mode: &Running| {
        reconcile_items(&v.root, mode, &[], complete, secs(s), &|_| true)
    };
    // Two complete passes 30 s apart: not yet.
    pass(0, true, &on);
    assert_eq!(pass(30, true, &on).closed, 0);
    // An incomplete pass clears the count; so does a dry run (a mode change).
    pass(70, false, &on);
    assert_eq!(pass(80, true, &on).closed, 0);
    assert_eq!(pass(100, true, &on).closed, 0);
    pass(200, true, &running(AgentWork::DryRun, true));
    assert_eq!(pass(300, true, &on).closed, 0);
    assert_eq!(closed(&v), 0);
    // A minute apart, nothing between: it closes.
    assert_eq!(pass(361, true, &on).closed, 1);
    assert_eq!(closed(&v), 1);
}

#[test]
fn a_missing_tracker_scope_never_restarts_the_clock_and_old_changed_results_are_backlog() {
    let v = vault();
    let on = running(AgentWork::On, true);
    // The first pass sees a result that changed before Hollow ever looked:
    // backlog, even though its file time (here) looks new.
    let touched = NeedsYouItem {
        state: "changed".into(),
        ..item_at(
            "result",
            "workspaces/notes/outputs/old.md",
            "Old",
            Some("notes"),
            0,
        )
    };
    v.pass(&on, std::slice::from_ref(&touched), true, -1);
    v.pass(&on, std::slice::from_ref(&touched), true, 30);
    assert!(v.events().is_empty());
    // A result that changes later is first seen then and filed.
    let fresh = NeedsYouItem {
        state: "changed".into(),
        ..item_at(
            "result",
            "workspaces/notes/outputs/new.md",
            "New",
            Some("notes"),
            -500,
        )
    };
    v.pass(&on, &[touched.clone(), fresh.clone()], true, 31);
    assert!(v.events().is_empty());
    // Its file time says long ago; its first-seen time decides.
    let later = at(31) + chrono::Duration::minutes(11);
    reconcile_items(&v.root, &on, &[touched, fresh], true, later, &|_| true);
    let keys: Vec<String> = v.events().into_iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        vec![link_key("result:workspaces/notes/outputs/new.md")]
    );
    // The setting is on but the tracker isn't set up this pass: the sync
    // reads it as off, and the mode's start stays.
    let before = crate::session_store::derived_state(&v.root, SINCE_KEY, SINCE_KIND)
        .unwrap()
        .unwrap()
        .payload;
    let config: Config = serde_json::from_value(json!({
        "vaultPath": v.root, "tracker": {"trackAgentWork": "on"}
    }))
    .unwrap();
    assert_eq!(chat_work::running_of(&config).mode, AgentWork::Off);
    reconcile(&config);
    let after = crate::session_store::derived_state(&v.root, SINCE_KEY, SINCE_KIND)
        .unwrap()
        .unwrap()
        .payload;
    assert_eq!(before, after);
}
