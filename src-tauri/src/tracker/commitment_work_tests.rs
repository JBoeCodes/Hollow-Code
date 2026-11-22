//! T3e: which commitments get a tracker copy, where, once, with what due
//! date, and how a copy is closed.

use super::*;
use crate::audit::{self, kind};
use crate::tracker::linear;
use crate::tracker::work::WorkEvent;
use crate::vault_commitments::CreateCommitmentInput;
use std::path::PathBuf;

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
/// workspace `notes`, each with a source note.
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
        std::fs::write(folder.join("call.md"), "# Call\n").unwrap();
    }
    Vault { _dir: dir, root }
}

/// The mode began well before any commitment a test makes.
fn long_ago() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

impl Vault {
    fn pass_at(&self, mode: &Running, now: DateTime<Utc>) -> waiting_work::Reconciled {
        let snap = snapshot(&self.root);
        reconcile_commitments(&self.root, mode, snap.as_ref(), now, &|_| true)
    }
    fn pass(&self, mode: &Running) -> waiting_work::Reconciled {
        self.pass_at(mode, Utc::now())
    }
    fn start(&self, mode: &Running) {
        self.pass_at(mode, long_ago());
    }
    fn commit(
        &self,
        title: &str,
        direction: &str,
        workspace: &str,
        due: Option<&str>,
    ) -> Commitment {
        vault_commitments::create(
            &self.root,
            CreateCommitmentInput {
                title: title.into(),
                direction: direction.into(),
                counterparty: "Pat Smith".into(),
                due: due.map(str::to_string),
                source_path: Some(format!("workspaces/{workspace}/call.md")),
                context: "Said on the Tuesday call".into(),
                ..CreateCommitmentInput::default()
            },
        )
        .unwrap()
    }
    fn events(&self) -> Vec<(String, WorkEvent)> {
        work::pending_events(&self.root, 100)
            .unwrap()
            .into_iter()
            .map(|e| (e.session_key, e.event))
            .collect()
    }
    fn writes(&self, key: &str) -> Vec<audit::AuditEntry> {
        [kind::TRACKER_WRITE, kind::TRACKER_WRITE_FAILED]
            .into_iter()
            .flat_map(|k| audit::rows_of(&self.root, key, k))
            .collect()
    }
    /// Pretend the outbox created the issue.
    fn confirm(&self, c: &Commitment, issue_key: &str) {
        let key = link_key(&commitment_id(&c.vault_path));
        work::fill_created(&self.root, &key, "i-1", issue_key).unwrap();
    }
}

fn key_of(c: &Commitment) -> String {
    link_key(&commitment_id(&c.vault_path))
}

#[test]
fn a_commitment_he_owes_is_filed_once_in_hol_assigned_with_its_due_date() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    let c = v.commit(
        "Send Pat the Q4 deck",
        "owed_by_me",
        "notes",
        Some("2026-11-20"),
    );
    assert_eq!(v.pass(&on).filed, 1);
    // Again, and after a restart of the count: never a second.
    assert_eq!(v.pass(&on).filed, 0);
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
    assert_eq!(key, &key_of(&c));
    assert_eq!(ck, &create_key(&commitment_id(&c.vault_path)));
    assert_eq!(project_id, "");
    assert_eq!(team_id.as_deref(), Some(linear::PERSONAL_TEAM_ID));
    assert_eq!(labels, &vec!["commitment".to_string()]);
    assert!(*assign_viewer);
    assert_eq!(due.as_deref(), Some("2026-11-20"));
    // HOL, his own team: the commitment's own title; the description holds
    // host facts only (no counterparty, no context).
    assert_eq!(title, "Send Pat the Q4 deck");
    assert!(description.contains("workspace notes"));
    assert!(!description.contains("Pat") && !description.contains("Tuesday"));
    let link = work::link_for(&v.root, key).unwrap();
    assert_eq!(link.linked_by, work::linked_by::COMMITMENT);
    assert!(link.created_issue);
}

#[test]
fn a_team_commitment_lands_in_ai_with_nothing_of_its_words() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    v.commit("Approve Maria's raise to $28/h", "owed_by_me", "proj", None);
    v.pass(&on);
    let events = v.events();
    let WorkEvent::Create {
        project_id,
        title,
        description,
        team_id,
        due,
        ..
    } = &events[0].1
    else {
        panic!("a create")
    };
    assert_eq!(project_id, "p1");
    assert_eq!(*team_id, None);
    assert_eq!(title, GENERIC_TITLE);
    assert_eq!(*due, None);
    let all = format!("{title}{description}");
    assert!(!all.contains("Maria") && !all.contains("$28") && !all.contains("Pat"));
}

#[test]
fn only_new_open_commitments_he_owes_and_awake_are_filed() {
    let v = vault();
    let on = running(AgentWork::On, true);
    // Made before the switch-on: a backlog, never filed.
    let old = v.commit("Old promise", "owed_by_me", "notes", None);
    v.pass(&on);
    assert!(v.events().is_empty());
    // Owed to him, snoozed, or already closed: none.
    v.commit("Raj sends the invoice", "owed_to_me", "notes", None);
    let snoozed = v.commit("Book the venue", "owed_by_me", "notes", None);
    vault_commitments::update(
        &v.root,
        vault_commitments::UpdateCommitmentInput {
            url: snoozed.url.clone(),
            snoozed_until: Some("2099-01-01".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let closed = v.commit("Call the bank", "owed_by_me", "notes", None);
    vault_commitments::set_status(&v.root, &closed.url, "fulfilled").unwrap();
    assert_eq!(v.pass(&on).filed, 0);
    assert!(work::link_for(&v.root, &key_of(&old)).is_none());
}

#[test]
fn a_dry_run_records_once_off_nothing_and_a_refusal_files_once_hol_is_set_up() {
    let v = vault();
    let dry = running(AgentWork::DryRun, true);
    v.start(&dry);
    let c = v.commit("Send the notes", "owed_by_me", "notes", Some("2026-12-01"));
    assert_eq!(v.pass(&dry).filed, 1);
    assert_eq!(v.pass(&dry).filed, 0);
    let rows = v.writes(&key_of(&c));
    assert_eq!(rows.len(), 1);
    let detail = rows[0].detail.as_deref().unwrap_or("");
    assert!(
        detail.starts_with("Dry run: Hollow would file this commitment as an issue in team PER")
            && detail.contains("due 2026-12-01"),
        "{detail}"
    );
    assert!(v.events().is_empty());

    let w = vault();
    let off = running(AgentWork::Off, true);
    w.commit("Nothing", "owed_by_me", "notes", None);
    assert_eq!(w.pass(&off), waiting_work::Reconciled::default());
    assert!(w.events().is_empty());

    let x = vault();
    let no_hol = running(AgentWork::On, false);
    x.start(&no_hol);
    let c = x.commit("Send it", "owed_by_me", "notes", None);
    assert_eq!(x.pass(&no_hol).filed, 0);
    assert!(x.events().is_empty());
    let hol = running(AgentWork::On, true);
    assert_eq!(x.pass(&hol).filed, 1);
    assert!(work::link_for(&x.root, &key_of(&c)).is_some());
}

#[test]
fn done_or_dropped_closes_at_once_and_a_deleted_note_needs_two_passes() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    let done = v.commit("Send the deck", "owed_by_me", "notes", None);
    let dropped = v.commit("Book the room", "owed_by_me", "notes", None);
    let deleted = v.commit("Write the memo", "owed_by_me", "notes", None);
    v.pass(&on);
    // Not created yet: nothing to close.
    vault_commitments::set_status(&v.root, &done.url, "fulfilled").unwrap();
    assert_eq!(v.pass(&on).closed, 0);
    for (c, key) in [(&done, "PER-1"), (&dropped, "PER-2"), (&deleted, "PER-3")] {
        v.confirm(c, key);
    }
    vault_commitments::set_status(&v.root, &dropped.url, "cancelled").unwrap();
    vault_commitments::trash(&v.root, &deleted.url).unwrap();
    let now = Utc::now();
    // Done and dropped are stored statuses: closed now. The deleted one
    // only counts as absent.
    assert_eq!(v.pass_at(&on, now).closed, 2);
    let mut answers: Vec<String> = v
        .events()
        .into_iter()
        .filter_map(|(_, e)| match e {
            WorkEvent::Resolved { answer } => Some(answer),
            _ => None,
        })
        .collect();
    answers.sort();
    assert_eq!(answers, ["Dropped in Hollow.", "Marked done in Hollow."]);
    // An unreadable folder starts the count again.
    let gone = Some(Snapshot::default()).filter(|_| false);
    reconcile_commitments(&v.root, &on, gone.as_ref(), now, &|_| true);
    assert_eq!(
        v.pass_at(&on, now + chrono::Duration::seconds(30)).closed,
        0
    );
    assert_eq!(
        v.pass_at(&on, now + chrono::Duration::seconds(70)).closed,
        0
    );
    assert_eq!(
        v.pass_at(&on, now + chrono::Duration::seconds(95)).closed,
        1
    );
    // Once: a later pass queues nothing more.
    assert_eq!(
        v.pass_at(&on, now + chrono::Duration::seconds(200)).closed,
        0
    );
    let resolved = v
        .events()
        .into_iter()
        .filter(|(_, e)| matches!(e, WorkEvent::Resolved { .. }))
        .count();
    assert_eq!(resolved, 3);
    // Reopened after its copy closed: no second issue.
    vault_commitments::set_status(&v.root, &done.url, "open").unwrap();
    assert_eq!(v.pass(&on).filed, 0);
}

#[test]
fn an_unparsed_note_is_never_absent_and_another_issue_is_never_closed() {
    let v = vault();
    let on = running(AgentWork::On, true);
    v.start(&on);
    let c = v.commit("Send the deck", "owed_by_me", "notes", None);
    v.pass(&on);
    v.confirm(&c, "PER-1");
    // A hand edit that no longer parses: present, so never read as gone.
    std::fs::write(v.root.join(&c.vault_path), "not a commitment").unwrap();
    let now = Utc::now();
    for s in [0, 70, 140] {
        assert_eq!(v.pass_at(&on, now + chrono::Duration::seconds(s)).closed, 0);
    }
    // A link Hollow didn't create is never closed.
    let other = "commitment:abc";
    work::insert_link_with(
        &v.root,
        &WorkLink {
            session_key: other.into(),
            issue_id: Some("i9".into()),
            issue_key: Some("ENG-9".into()),
            workspace_id: None,
            linked_by: work::linked_by::COMMITMENT.into(),
            created_issue: false,
            at: now.to_rfc3339(),
            depth: 1,
            tracked: true,
            outcome_at: None,
            title: None,
        },
        &[],
    )
    .unwrap();
    for s in [200, 300] {
        assert_eq!(v.pass_at(&on, now + chrono::Duration::seconds(s)).closed, 0);
    }
    assert!(ended_of(None, false).is_some_and(|e| !e.positive));
    assert!(ended_of(None, true).is_none());
}

#[test]
fn a_missing_folder_with_open_copies_is_an_incomplete_read_never_a_mass_close() {
    let v = vault();
    // No folder and no copies: an empty ledger, read whole.
    assert!(snapshot(&v.root).is_some_and(|s| s.commitments.is_empty() && s.present.is_empty()));
    let on = running(AgentWork::On, true);
    v.start(&on);
    let c = v.commit("Send the deck", "owed_by_me", "notes", None);
    v.pass(&on);
    v.confirm(&c, "PER-1");
    // The whole folder goes (a moved vault, a sync mid-way): incomplete.
    std::fs::remove_dir_all(v.root.join("commitments")).unwrap();
    assert!(snapshot(&v.root).is_none());
    let now = Utc::now();
    for s in [0, 70, 140, 600] {
        assert_eq!(v.pass_at(&on, now + chrono::Duration::seconds(s)).closed, 0);
    }
    assert!(!v
        .events()
        .iter()
        .any(|(_, e)| matches!(e, WorkEvent::Resolved { .. })));
    // A vault root that is gone is never an empty ledger either.
    assert!(snapshot(&v.root.join("no-such-vault")).is_none());
}
