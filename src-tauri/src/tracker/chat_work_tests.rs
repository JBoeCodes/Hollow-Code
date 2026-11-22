//! T3a: which chats get an issue, where, and what a dry run and on record.

use super::*;
use crate::audit::{self, kind};
use crate::evidence::{EvidenceReceipt, ReceiptFile};
use crate::tracker::work::WorkEvent;

const T0: &str = "2026-11-15T10:00:00.000Z";

fn facts(prs: usize, repo: usize, outputs: usize, minutes: i64, reply: usize) -> TurnFacts {
    TurnFacts {
        prs,
        repo_changes: repo,
        outputs,
        working_ms: minutes * 60_000,
        reply_chars: reply,
    }
}

#[test]
fn quick_questions_and_checks_get_none_and_real_work_is_named() {
    // A quick question, a status check, a long wait with a short answer.
    assert_eq!(classify(&facts(0, 0, 0, 1, 400)), None);
    assert_eq!(classify(&facts(0, 0, 0, 4, 3000)), None);
    assert_eq!(classify(&facts(0, 0, 0, 30, 200)), None);
    assert_eq!(classify(&facts(1, 0, 0, 1, 50)), Some(ChatKind::Build));
    assert_eq!(classify(&facts(0, 3, 1, 1, 50)), Some(ChatKind::Build));
    assert_eq!(
        classify(&facts(0, 0, 2, 3, 50)),
        Some(ChatKind::Deliverable)
    );
    assert_eq!(
        classify(&facts(0, 0, 0, RESEARCH_MINUTES, RESEARCH_REPLY_CHARS)),
        Some(ChatKind::Research)
    );
}

/// A turn of `minutes` that opened PR 1450 with its own `gh pr create`.
fn built(minutes: i64) -> EvidenceReceipt {
    let mut r = receipt(minutes, &[]);
    r.pull_requests =
        vec![
            serde_json::from_value(serde_json::json!({"number": 1450, "action": "opened"}))
                .unwrap(),
        ];
    r
}

fn receipt(minutes: i64, outputs: &[&str]) -> EvidenceReceipt {
    let start = chrono::DateTime::parse_from_rfc3339(T0).unwrap();
    EvidenceReceipt {
        started_at: T0.into(),
        finished_at: (start + chrono::Duration::minutes(minutes)).to_rfc3339(),
        files: outputs
            .iter()
            .map(|path| -> ReceiptFile {
                serde_json::from_value(serde_json::json!({
                    "path": path, "changeType": "added", "attribution": "confirmed"
                }))
                .unwrap()
            })
            .collect(),
        ..EvidenceReceipt::default()
    }
}

#[test]
fn facts_come_from_the_turns_own_evidence_never_the_replys_words() {
    let r = receipt(
        12,
        &["workspaces/w/outputs/brief.pdf", "workspaces/w/log/x.md"],
    );
    // A PR named in the reply is not evidence.
    let f = facts_of(
        &r,
        "Opened https://github.com/o/r/pull/77.",
        Some("workspaces/w"),
    );
    assert_eq!((f.prs, f.outputs, f.repo_changes), (0, 1, 0));
    assert_eq!(f.working_ms, 12 * 60_000);
    // A PR the turn opened, and a push, are.
    assert_eq!(facts_of(&built(1), "", Some("workspaces/w")).prs, 1);
    let mut pushed = receipt(1, &[]);
    pushed.outside_effects = Some(
        serde_json::from_value(serde_json::json!({"recorded": [
            {"kind": "shell", "label": "git push · git push origin fix", "tool": "Bash", "outcome": "done", "sequence": 1},
            {"kind": "shell", "label": "git push · git push origin again", "tool": "Bash", "outcome": "failed", "sequence": 2}
        ]}))
        .unwrap(),
    );
    assert_eq!(facts_of(&pushed, "", None).prs, 1);
    // Another chat's change in a shared folder isn't this turn's.
    let mut shared = receipt(1, &["workspaces/w/outputs/a.md"]);
    shared.files[0].attribution = "shared".into();
    assert_eq!(facts_of(&shared, "ok", Some("workspaces/w")).outputs, 0);
    // Only the chat workspace's own outputs/: not a nested one, not another
    // workspace's, not one for a chat in no workspace.
    let elsewhere = receipt(
        1,
        &[
            "workspaces/w/sources/outputs/a.md",
            "workspaces/other/outputs/b.md",
            "outputs/c.md",
        ],
    );
    assert_eq!(facts_of(&elsewhere, "", Some("workspaces/w")).outputs, 0);
    assert_eq!(facts_of(&r, "", None).outputs, 0);
}

#[test]
fn a_quick_question_that_names_a_pr_files_nothing() {
    let v = vault();
    set_running(&v.root, running(AgentWork::On, true));
    work::set_tracking(&v.root, true);
    v.chat("s-20261115-askpr001", "proj", "Is PR 482 merged?");
    let quick = receipt(0, &[]);
    after_turn(
        &v.root,
        "s-20261115-askpr001",
        true,
        "success",
        &quick,
        "Yes, PR 482 merged at 9:14 (https://github.com/o/r/pull/482).",
    );
    assert!(v.events().is_empty());
    assert!(v.writes("s-20261115-askpr001").is_empty());
}

fn running(mode: AgentWork, hol: bool) -> Running {
    Running {
        mode,
        hol,
        linear: true,
    }
}

#[test]
fn tracked_projects_go_to_ai_and_everything_else_to_hol() {
    let on = running(AgentWork::On, true);
    assert_eq!(
        destination(&Place::Tracked("p1".into()), &on),
        Ok(Destination::Ai {
            project: "p1".into()
        })
    );
    assert_eq!(destination(&Place::QuickChats, &on), Ok(Destination::Hol));
    assert_eq!(destination(&Place::Untracked, &on), Ok(Destination::Hol));
    // Without HOL set up, Hollow-only work is refused, never filed in AI.
    let no_hol = running(AgentWork::On, false);
    assert!(destination(&Place::Untracked, &no_hol).is_err());
    assert!(destination(&Place::Tracked("p1".into()), &no_hol).is_ok());
    // The create key is the chat's own, the same on every retry.
    assert_eq!(
        create_key("s-20261115-aaaa0001"),
        create_key("s-20261115-aaaa0001")
    );
    assert_ne!(
        create_key("s-20261115-aaaa0001"),
        create_key("s-20261115-aaaa0002")
    );
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
    fn chat(&self, key: &str, workspace: &str, asked: &str) {
        let cwd = self.root.join("workspaces").join(workspace);
        crate::session_store::start_turn(
            &self.root,
            key,
            "claude",
            &cwd.to_string_lossy(),
            &format!("run-{key}"),
            asked,
        )
        .unwrap();
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

const BUILD_REPLY: &str = "Opened PR 1450 with the fix; tests pass.\n\nDone";

#[test]
fn a_dry_run_records_once_what_it_would_file_and_sends_nothing() {
    let v = vault();
    set_running(&v.root, running(AgentWork::DryRun, true));
    work::set_tracking(&v.root, false);
    // A quick question: nothing at all.
    v.chat("s-20261115-quick001", "proj", "What's on my plate today?");
    after_turn(
        &v.root,
        "s-20261115-quick001",
        true,
        "success",
        &receipt(1, &[]),
        "Three things.",
    );
    assert!(v.writes("s-20261115-quick001").is_empty());
    // A build: one audit row naming the team, nothing queued, no link.
    v.chat(
        "s-20261115-build001",
        "proj",
        "Fix the export totals and open a PR",
    );
    for _ in 0..2 {
        after_turn(
            &v.root,
            "s-20261115-build001",
            true,
            "success",
            &built(20),
            BUILD_REPLY,
        );
    }
    let rows = v.writes("s-20261115-build001");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].outcome.as_deref(), Some("skipped"));
    assert_eq!(rows[0].action.as_deref(), Some("create"));
    let detail = rows[0].detail.clone().unwrap();
    assert!(
        detail.starts_with("Dry run:")
            && detail.contains("build chat")
            && detail.contains("team ENG"),
        "{detail}"
    );
    assert_eq!(rows[0].request_key, Some(create_key("s-20261115-build001")));
    assert!(work::link_for(&v.root, "s-20261115-build001").is_none());
    assert!(v.events().is_empty());
}

#[test]
fn on_files_one_issue_per_real_work_chat_through_the_outbox_facts() {
    let v = vault();
    set_running(&v.root, running(AgentWork::On, true));
    work::set_tracking(&v.root, true);
    v.chat("s-20261115-quick002", "notes", "Is the mini up?");
    after_turn(
        &v.root,
        "s-20261115-quick002",
        true,
        "success",
        &receipt(1, &[]),
        "Yes.",
    );
    assert!(v.events().is_empty(), "a quick question makes no issue");
    // A tracked project's build: team AI, its project.
    v.chat("s-20261115-build002", "proj", "Fix the export totals");
    after_turn(
        &v.root,
        "s-20261115-build002",
        true,
        "success",
        &built(20),
        BUILD_REPLY,
    );
    // Another turn of the same chat (chat.rs runs X1's after-turn first)
    // files nothing new.
    let again = built(20);
    work::after_turn(
        &v.root,
        "s-20261115-build002",
        "success",
        Some(&again),
        BUILD_REPLY,
    );
    after_turn(
        &v.root,
        "s-20261115-build002",
        true,
        "success",
        &again,
        BUILD_REPLY,
    );
    let link = work::link_for(&v.root, "s-20261115-build002").unwrap();
    assert_eq!(
        (link.linked_by.as_str(), link.tracked, link.issue_id),
        ("chat", true, None)
    );
    let events = v.events();
    let creates: Vec<&WorkEvent> = events
        .iter()
        .filter(|(k, e)| k == "s-20261115-build002" && matches!(e, WorkEvent::Create { .. }))
        .map(|(_, e)| e)
        .collect();
    assert_eq!(creates.len(), 1);
    let WorkEvent::Create {
        project_id,
        team_id,
        create_key: key,
        description,
        ..
    } = creates[0]
    else {
        unreachable!()
    };
    assert_eq!((project_id.as_str(), team_id), ("p1", &None));
    assert_eq!(key, &create_key("s-20261115-build002"));
    assert!(!description.contains("Fix the export totals"));
    assert!(
        description.contains("A build chat in workspace proj")
            && description.contains("s-20261115-build002")
    );
    let kinds: Vec<&str> = events
        .iter()
        .map(|(_, e)| match e {
            WorkEvent::Create { .. } => "create",
            WorkEvent::Started { .. } => "started",
            WorkEvent::Outcome { .. } => "outcome",
            WorkEvent::Progress { .. } => "progress",
            WorkEvent::Resolved { .. } => "resolved",
            WorkEvent::Decision { .. } => "decision",
            WorkEvent::Unknown => "unknown",
        })
        .collect();
    // The second turn is a progress note, never a second create or Outcome.
    assert_eq!(kinds, vec!["create", "started", "outcome", "progress"]);
    // A deliverable in an untracked workspace: team HOL, no project.
    v.chat("s-20261115-deliv001", "notes", "Write the brief");
    after_turn(
        &v.root,
        "s-20261115-deliv001",
        true,
        "success",
        &receipt(3, &["workspaces/notes/outputs/brief.md"]),
        "Done",
    );
    let hol = v
        .events()
        .into_iter()
        .find_map(|(k, e)| match e {
            WorkEvent::Create {
                project_id,
                team_id,
                ..
            } if k == "s-20261115-deliv001" => Some((project_id, team_id)),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        hol,
        (String::new(), Some(linear::PERSONAL_TEAM_ID.to_string()))
    );
}

#[test]
fn agents_chats_stopped_turns_and_off_make_nothing() {
    let v = vault();
    set_running(&v.root, running(AgentWork::On, true));
    work::set_tracking(&v.root, true);
    v.chat("s-20261115-agent001", "proj", "Build it");
    after_turn(
        &v.root,
        "s-20261115-agent001",
        false,
        "success",
        &built(20),
        BUILD_REPLY,
    );
    after_turn(
        &v.root,
        "s-20261115-agent001",
        true,
        "cancelled",
        &built(20),
        BUILD_REPLY,
    );
    assert!(v.events().is_empty());
    set_running(&v.root, running(AgentWork::Off, true));
    after_turn(
        &v.root,
        "s-20261115-agent001",
        true,
        "success",
        &built(20),
        BUILD_REPLY,
    );
    assert!(v.events().is_empty() && v.writes("s-20261115-agent001").is_empty());
    // HOL not set up: refused once in the audit, never filed in AI.
    set_running(&v.root, running(AgentWork::On, false));
    v.chat("s-20261115-nohol001", "notes", "Write the brief");
    for _ in 0..2 {
        after_turn(
            &v.root,
            "s-20261115-nohol001",
            true,
            "success",
            &receipt(3, &["workspaces/notes/outputs/b.md"]),
            "Done",
        );
    }
    let rows = v.writes("s-20261115-nohol001");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome.as_deref(), Some("failed"));
    assert!(v.events().is_empty());
}

/// The person's request never leaves Hollow: only the chat's title (its
/// first line, redacted) and host facts reach the issue and its comments.
#[test]
fn the_request_text_never_reaches_the_issue_beyond_the_title() {
    let v = vault();
    set_running(&v.root, running(AgentWork::On, true));
    work::set_tracking(&v.root, true);
    let key = "s-20261115-priv0001";
    v.chat(
        key,
        "proj",
        "Fix the totals\n\nThe board memo says PRIVATE-XYZZY and the key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGH",
    );
    after_turn(&v.root, key, true, "success", &built(20), "Done");
    let events = v.events();
    assert_eq!(events.len(), 3);
    for (_, event) in &events {
        let text = serde_json::to_string(event).unwrap();
        assert!(
            !text.contains("XYZZY") && !text.contains("abcdefghijklmnop"),
            "{text}"
        );
        let comment = match event {
            WorkEvent::Started { .. } => work::render_started(event, key, "work-0"),
            WorkEvent::Outcome { .. } => {
                let head = work::LogHead {
                    at: work::local_stamp("2026-11-15T10:00:00Z"),
                    who: work::who_of(&v.root, key),
                    why: "work on ENG-1, Fix the totals".into(),
                };
                work::render_outcome(event, &head, key, "work-0")
            }
            _ => continue,
        };
        assert!(!comment.contains("XYZZY"), "{comment}");
    }
    let WorkEvent::Create { title, .. } = &events[0].1 else {
        unreachable!()
    };
    assert_eq!(title, "Fix the totals");
}

#[test]
fn a_handed_over_chat_keeps_its_issue_and_never_files_a_second() {
    let v = vault();
    set_running(&v.root, running(AgentWork::On, true));
    work::set_tracking(&v.root, true);
    let (old, new) = ("s-20261115-hand0001", "s-20261115-hand0002");
    v.chat(old, "proj", "Fix the export totals");
    after_turn(&v.root, old, true, "success", &built(20), "Done");
    crate::app_sessions::record_handover(&v.root, old, new, 1).unwrap();
    v.chat(new, "proj", "Carry on");
    work::after_turn(&v.root, new, "success", Some(&built(20)), "Done");
    after_turn(&v.root, new, true, "success", &built(20), "Done");
    let creates = v
        .events()
        .iter()
        .filter(|(_, e)| matches!(e, WorkEvent::Create { .. }))
        .count();
    assert_eq!(creates, 1);
    let link = work::link_for(&v.root, new).expect("the successor has its predecessor's link");
    assert_eq!(link.linked_by, "chat");
    // A dry run is decided for the whole chain too.
    set_running(&v.root, running(AgentWork::DryRun, true));
    let (a, b) = ("s-20261115-hand0003", "s-20261115-hand0004");
    v.chat(a, "proj", "Build it");
    after_turn(&v.root, a, true, "success", &built(20), "Done");
    crate::app_sessions::record_handover(&v.root, a, b, 1).unwrap();
    v.chat(b, "proj", "Go on");
    after_turn(&v.root, b, true, "success", &built(20), "Done");
    assert_eq!(v.writes(a).len(), 1);
    assert!(v.writes(b).is_empty());
}

/// A link left with no fact (an older build's crash between the two
/// writes): the chat is decided again and its create queued once.
#[test]
fn a_chat_link_with_no_fact_is_queued_again() {
    let v = vault();
    set_running(&v.root, running(AgentWork::On, true));
    work::set_tracking(&v.root, true);
    let key = "s-20261115-crash001";
    v.chat(key, "proj", "Fix it");
    work::insert_link(
        &v.root,
        &work::WorkLink {
            session_key: key.into(),
            issue_id: None,
            issue_key: None,
            workspace_id: Some("proj".into()),
            linked_by: work::linked_by::CHAT.into(),
            created_issue: true,
            at: T0.into(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
    )
    .unwrap();
    for _ in 0..2 {
        after_turn(&v.root, key, true, "success", &built(20), "Done");
    }
    let creates = v
        .events()
        .iter()
        .filter(|(k, e)| k == key && matches!(e, WorkEvent::Create { .. }))
        .count();
    assert_eq!(creates, 1);
}

/// Decision: a turn that ended in an error is filed only when its own facts
/// show real work (a PR opened before the error); otherwise nothing.
#[test]
fn an_errored_turn_files_only_with_real_work_facts() {
    let v = vault();
    set_running(&v.root, running(AgentWork::On, true));
    work::set_tracking(&v.root, true);
    v.chat("s-20261115-error001", "proj", "Fix it");
    after_turn(
        &v.root,
        "s-20261115-error001",
        true,
        "error",
        &receipt(20, &[]),
        "",
    );
    assert!(v.events().is_empty());
    after_turn(
        &v.root,
        "s-20261115-error001",
        true,
        "error",
        &built(20),
        "",
    );
    assert!(v
        .events()
        .iter()
        .any(|(_, e)| matches!(e, WorkEvent::Create { .. })));
}
