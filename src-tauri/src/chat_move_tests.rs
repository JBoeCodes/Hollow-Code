//! M5 PR1: a chat moves between two Macs' Sessions DBs (two vaults in one
//! process, so two databases), its history imported once, resumable there,
//! and owned by exactly one Mac at every step.
use super::*;
use crate::chat_resume::{plan_resume_with, ResetReason, ResumePlan};
use crate::session_store as store;
use chrono::Utc;
use std::path::Path;

const MACBOOK: &str = "m-3f9c1a07e2b45d18";
const MINI: &str = "m-8d21c4be90f17a3e";
const NATIVE: &str = "0b1c2d3e-4f50-4617-8899-aabbccddeeff";

struct Two {
    _a: tempfile::TempDir,
    _b: tempfile::TempDir,
    a: PathBuf,
    b: PathBuf,
}

/// Two Macs' vaults, each with its Quick chats folder (the vault syncs, so
/// both have it).
fn two() -> Two {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let a = a_dir.path().canonicalize().unwrap();
    let b = b_dir.path().canonicalize().unwrap();
    crate::workspaces::ensure_quick_chats(&a).unwrap();
    crate::workspaces::ensure_quick_chats(&b).unwrap();
    Two {
        _a: a_dir,
        _b: b_dir,
        a,
        b,
    }
}

fn key(n: u32) -> String {
    format!("s-20261115-m5{n:06}")
}

/// A finished chat with `turns` exchanges, a native session id and an
/// effort, in `vault`'s Quick chats.
fn seed(vault: &Path, key: &str, turns: &[(&str, &str)]) {
    let cwd = crate::workspaces::ensure_quick_chats(vault).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    for (i, (asked, answer)) in turns.iter().enumerate() {
        let ord = store::start_turn(vault, key, "claude", &cwd, &format!("run-{key}-{i}"), asked)
            .unwrap();
        store::record_turn_permission(vault, key, ord, "plan").unwrap();
        store::append_event(vault, key, ord, "assistant", answer).unwrap();
        store::finish_turn(vault, key, ord, "success").unwrap();
    }
    store::record_harness_id(vault, key, NATIVE).unwrap();
    store::set_chat_effort(vault, key, Some("high")).unwrap();
}

/// Send `bytes` as parts and commit them, as the sending Mac does (PR2).
fn send(vault: &Path, move_id: &str, bytes: &[u8]) -> Result<Imported, MoveError> {
    let bodies = part_bodies(bytes);
    for (n, body) in bodies.iter().enumerate() {
        let body: PartBody = serde_json::from_value(body.clone()).unwrap();
        staging(vault)
            .put(move_id, MACBOOK, n, &body, Instant::now())
            .unwrap();
    }
    let commit_body = CommitBody {
        parts: Some(bodies.len()),
        sha256: Some(sha256_hex(bytes)),
    };
    commit(vault, move_id, MACBOOK, &commit_body, Utc::now())
}

/// Which Macs own chat `key`: one with the chat and no home record, or
/// `leaving` or `arrived`.
fn owners(two: &Two, key: &str) -> usize {
    [&two.a, &two.b]
        .into_iter()
        .filter(|vault| {
            let has = store::session_by_key(vault, key).unwrap().is_some();
            let home = home_of(vault, key).unwrap();
            has && home.is_none_or(|h| h.state != AWAY)
        })
        .count()
}

#[test]
fn a_chat_moves_once_and_its_next_turn_starts_fresh_with_the_history() {
    let two = two();
    let k = key(1);
    seed(
        &two.a,
        &k,
        &[
            ("Plan the trip", "Here is a plan."),
            ("Shorter", "Two days."),
        ],
    );
    let before = store::conversation_entries(&two.a, &k, 3).unwrap();
    assert_eq!(owners(&two, &k), 1);

    let move_id = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &move_id, Utc::now()).unwrap();
    assert_eq!(bundle.turns.len(), 2);
    assert_eq!(bundle.native.as_ref().map(|n| n.id.as_str()), Some(NATIVE));
    assert_eq!(bundle.effort.as_deref(), Some("high"));
    // Leaving: still the MacBook's alone, and it runs nothing.
    assert_eq!(owners(&two, &k), 1);
    assert!(admission_refusal(&two.a, &k).is_some());
    // A retry of the same move gives the same bundle.
    let (again, _) = begin_move_out(&two.a, &k, MACBOOK, MINI, &move_id, Utc::now()).unwrap();
    assert_eq!(again, bundle);

    let imported = send(&two.b, &move_id, &bytes).unwrap();
    assert_eq!((imported.turns, imported.already), (2, false));
    // A second commit (its answer was lost) and a second import are no-ops.
    let commit_body = CommitBody {
        parts: Some(1),
        sha256: Some(sha256_hex(&bytes)),
    };
    let repeat = commit(&two.b, &move_id, MACBOOK, &commit_body, Utc::now()).unwrap();
    assert!(repeat.already);
    let repeat = import_move(&two.b, &bundle, MACBOOK, Utc::now()).unwrap();
    assert!(repeat.already);
    let there = store::session_by_key(&two.b, &k).unwrap().unwrap();
    assert_eq!(there.turn_count, 2);
    assert_eq!(
        Path::new(&there.cwd),
        crate::workspaces::ensure_quick_chats(&two.b).unwrap()
    );
    finish_move_out(&two.a, &move_id, Utc::now()).unwrap();
    finish_move_out(&two.a, &move_id, Utc::now()).unwrap();
    assert_eq!(owners(&two, &k), 1);
    let away = home_of(&two.a, &k).unwrap().unwrap();
    assert_eq!((away.state.as_str(), away.peer.as_str()), (AWAY, MINI));
    let arrived = home_of(&two.b, &k).unwrap().unwrap();
    assert_eq!(
        (arrived.state.as_str(), arrived.peer.as_str()),
        (ARRIVED, MACBOOK)
    );
    assert!(admission_refusal(&two.a, &k).is_some());
    assert_eq!(admission_refusal(&two.b, &k), None);
    assert_eq!(homes(&two.b).unwrap().len(), 1);

    // Resumable on the mini: the same conversation, a fresh native session
    // (no id there), and a stale id from a composer resets as a move.
    let after = store::conversation_entries(&two.b, &k, 3).unwrap();
    let text = |e: &[crate::session_store::ContextEntry]| {
        e.iter()
            .map(|e| (e.role.clone(), e.text.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(text(&after), text(&before));
    assert_eq!(after.len(), 4);
    let state = store::resume_state(&two.b, &k).unwrap();
    assert!(state.open.as_ref().unwrap().harness_session_id.is_none());
    assert_eq!(
        plan_resume_with(Some(NATIVE), &state, "fp", false),
        ResumePlan::Reset(ResetReason::Moved)
    );
    assert_eq!(plan_resume_with(None, &state, "fp", false), ResumePlan::New);
    assert!(crate::context_compiler::Session::of(&ResumePlan::New).fresh());
    assert_eq!(
        store::chat_effort(&two.b, &k).unwrap().as_deref(),
        Some("high")
    );
    assert_eq!(
        store::last_permission(&two.b, &k).unwrap().as_deref(),
        Some("plan")
    );
    // The next turn there numbers on from the moved ones.
    let cwd = there.cwd.clone();
    let ord = store::start_turn(&two.b, &k, "claude", &cwd, "run-mini-1", "Go on").unwrap();
    assert_eq!(ord, 3);
}

#[test]
fn a_refused_or_failed_move_leaves_exactly_one_owner() {
    let two = two();
    let k = key(2);
    seed(&two.a, &k, &[("One", "Yes.")]);
    // A turn running: refused, nothing written.
    let cwd = crate::workspaces::ensure_quick_chats(&two.a).unwrap();
    let ord =
        store::start_turn(&two.a, &k, "claude", &cwd.to_string_lossy(), "run-x", "Two").unwrap();
    let busy = begin_move_out(&two.a, &k, MACBOOK, MINI, &new_move_id(), Utc::now()).unwrap_err();
    assert_eq!(busy.code, "busy");
    assert_eq!(home_of(&two.a, &k).unwrap(), None);
    store::finish_turn(&two.a, &k, ord, "success").unwrap();

    // Aborted before anything landed: home again, runs again.
    let first = new_move_id();
    begin_move_out(&two.a, &k, MACBOOK, MINI, &first, Utc::now()).unwrap();
    let second = begin_move_out(&two.a, &k, MACBOOK, MINI, &new_move_id(), Utc::now());
    assert_eq!(second.unwrap_err().code, "busy", "one move at a time");
    let none_there = TargetSaid::Cancelled(Cancelled::NotImported);
    abort_move_out(&two.a, &first, MINI, none_there, Utc::now()).unwrap();
    abort_move_out(&two.a, &first, MINI, none_there, Utc::now()).unwrap();
    assert_eq!(home_of(&two.a, &k).unwrap(), None);
    assert_eq!(admission_refusal(&two.a, &k), None);
    assert!(finish_move_out(&two.a, &first, Utc::now()).is_err());
    assert_eq!(owners(&two, &k), 1);

    // Called off on the mini before its commit: the tombstone refuses a
    // late commit, the MacBook aborts, still one owner.
    let late = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &late, Utc::now()).unwrap();
    assert_eq!(
        move_status(&two.b, &late, MACBOOK).unwrap().state,
        "unknown"
    );
    assert_eq!(
        cancel_move_in(&two.b, &late, MACBOOK, Utc::now()).unwrap(),
        Cancelled::NotImported
    );
    assert_eq!(send(&two.b, &late, &bytes).unwrap_err().code, "cancelled");
    assert_eq!(
        import_move(&two.b, &bundle, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "cancelled"
    );
    assert!(store::session_by_key(&two.b, &k).unwrap().is_none());
    abort_move_out(
        &two.a,
        &late,
        MINI,
        TargetSaid::Cancelled(Cancelled::NotImported),
        Utc::now(),
    )
    .unwrap();
    assert_eq!(owners(&two, &k), 1);
    assert_eq!(
        move_status(&two.b, &late, MACBOOK).unwrap().state,
        "cancelled"
    );
}

#[test]
fn a_cancel_after_the_import_undoes_it_unless_the_chat_ran_there() {
    let two = two();
    let k = key(3);
    seed(&two.a, &k, &[("One", "Yes.")]);
    // Imported, then called off (the MacBook chose Keep it here): gone.
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    assert_eq!(move_status(&two.b, &id, MACBOOK).unwrap().state, "imported");
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now()).unwrap(),
        Cancelled::Removed
    );
    assert!(store::session_by_key(&two.b, &k).unwrap().is_none());
    assert_eq!(home_of(&two.b, &k).unwrap(), None);
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now()).unwrap(),
        Cancelled::Already
    );
    abort_move_out(
        &two.a,
        &id,
        MINI,
        TargetSaid::Cancelled(Cancelled::Removed),
        Utc::now(),
    )
    .unwrap();
    assert_eq!(owners(&two, &k), 1);

    // Imported and a turn ran there: the move stands, the MacBook finishes.
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    let cwd = store::session_by_key(&two.b, &k).unwrap().unwrap().cwd;
    store::start_turn(&two.b, &k, "claude", &cwd, "run-mini", "Carry on").unwrap();
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "started"
    );
    // Another Mac can't call it off or read it.
    let stranger = "m-0000000000000001";
    assert_eq!(
        cancel_move_in(&two.b, &id, stranger, Utc::now())
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        move_status(&two.b, &id, stranger).unwrap_err().code,
        "not_found"
    );
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    assert_eq!(owners(&two, &k), 1);
}

#[test]
fn a_chat_comes_back_and_other_keys_conflict() {
    let two = two();
    let k = key(4);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let out = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &out, Utc::now()).unwrap();
    // Only the Mac the bundle names may send it.
    assert_eq!(
        import_move(&two.b, &bundle, "m-0000000000000001", Utc::now())
            .unwrap_err()
            .code,
        "forbidden"
    );
    send(&two.b, &out, &bytes).unwrap();
    finish_move_out(&two.a, &out, Utc::now()).unwrap();
    // It runs a turn on the mini, then comes back.
    let cwd = store::session_by_key(&two.b, &k).unwrap().unwrap().cwd;
    let ord = store::start_turn(&two.b, &k, "claude", &cwd, "run-mini", "Two").unwrap();
    store::append_event(&two.b, &k, ord, "assistant", "Done on the mini.").unwrap();
    store::finish_turn(&two.b, &k, ord, "success").unwrap();
    let back = new_move_id();
    let (_, bytes) = begin_move_out(&two.b, &k, MINI, MACBOOK, &back, Utc::now()).unwrap();
    let bodies = part_bodies(&bytes);
    for (n, body) in bodies.iter().enumerate() {
        let body: PartBody = serde_json::from_value(body.clone()).unwrap();
        staging(&two.a)
            .put(&back, MINI, n, &body, Instant::now())
            .unwrap();
    }
    let commit_body = CommitBody {
        parts: Some(bodies.len()),
        sha256: Some(sha256_hex(&bytes)),
    };
    let imported = commit(&two.a, &back, MINI, &commit_body, Utc::now()).unwrap();
    assert_eq!(imported.turns, 2);
    finish_move_out(&two.b, &back, Utc::now()).unwrap();
    assert_eq!(owners(&two, &k), 1);
    assert_eq!(home_of(&two.a, &k).unwrap().unwrap().state, ARRIVED);
    assert_eq!(home_of(&two.b, &k).unwrap().unwrap().state, AWAY);
    let entries = store::conversation_entries(&two.a, &k, 10).unwrap();
    assert_eq!(entries.last().unwrap().text, "Done on the mini.");
    assert_eq!(
        store::session_by_key(&two.a, &k)
            .unwrap()
            .unwrap()
            .turn_count,
        2
    );

    // A key the mini has of its own is never overwritten.
    let other = key(5);
    seed(&two.a, &other, &[("A", "B")]);
    seed(&two.b, &other, &[("C", "D")]);
    let id = new_move_id();
    let (bundle, _) = begin_move_out(&two.a, &other, MACBOOK, MINI, &id, Utc::now()).unwrap();
    assert_eq!(
        import_move(&two.b, &bundle, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "conflict"
    );
    let mine = store::conversation_entries(&two.b, &other, 10).unwrap();
    assert_eq!(mine[0].text, "C");
}

#[test]
fn the_main_agents_chat_and_a_missing_place_stay_home() {
    let two = two();
    let k = key(6);
    let main = crate::workspaces::ensure_main_agent(&two.a).unwrap();
    store::start_turn(
        &two.a,
        &k,
        "claude",
        &main.to_string_lossy(),
        "run-main",
        "Hi",
    )
    .unwrap();
    store::finish_turn(&two.a, &k, 1, "success").unwrap();
    let refused = begin_move_out(&two.a, &k, MACBOOK, MINI, &new_move_id(), Utc::now());
    assert_eq!(refused.unwrap_err().code, "conflict");
    assert_eq!(home_of(&two.a, &k).unwrap(), None);

    // A folder the mini's vault doesn't have yet.
    let k = key(7);
    let notes = two.a.join("Notes");
    std::fs::create_dir_all(&notes).unwrap();
    store::start_turn(
        &two.a,
        &k,
        "claude",
        &notes.to_string_lossy(),
        "run-n",
        "Hi",
    )
    .unwrap();
    store::finish_turn(&two.a, &k, 1, "success").unwrap();
    let id = new_move_id();
    let (bundle, _) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    assert_eq!(bundle.place.vault_path.as_deref(), Some("Notes"));
    assert_eq!(
        import_move(&two.b, &bundle, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "not_here"
    );
    let mut escape = bundle.clone();
    escape.place.vault_path = Some("../elsewhere".into());
    assert_eq!(
        import_move(&two.b, &escape, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "bad_request"
    );
    std::fs::create_dir_all(two.b.join("Notes")).unwrap();
    import_move(&two.b, &bundle, MACBOOK, Utc::now()).unwrap();
}

#[test]
fn staging_checks_parts_digest_and_room() {
    let staging = Staging::default();
    let id = new_move_id();
    let bytes = vec![b'x'; PART_MAX + 10];
    let bodies = part_bodies(&bytes);
    assert_eq!(bodies.len(), 2);
    let part = |v: &Value| serde_json::from_value::<PartBody>(v.clone()).unwrap();
    let now = Instant::now();
    assert_eq!(
        staging
            .put(&id, MACBOOK, 0, &part(&bodies[0]), now)
            .unwrap(),
        1
    );
    // Another Mac can't add to it; a part past the end is refused.
    assert_eq!(
        staging
            .put(&id, MINI, 1, &part(&bodies[1]), now)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        staging
            .put(&id, MACBOOK, 2, &part(&bodies[1]), now)
            .unwrap_err()
            .code,
        "bad_request"
    );
    let good = CommitBody {
        parts: Some(2),
        sha256: Some(sha256_hex(&bytes)),
    };
    assert_eq!(
        staging.take(&id, MACBOOK, &good).unwrap_err().code,
        "bad_request"
    );
    staging
        .put(&id, MACBOOK, 1, &part(&bodies[1]), now)
        .unwrap();
    let wrong = CommitBody {
        parts: Some(2),
        sha256: Some(sha256_hex(b"other")),
    };
    assert_eq!(
        staging.take(&id, MACBOOK, &wrong).unwrap_err().code,
        "bad_request"
    );
    assert_eq!(staging.take(&id, MACBOOK, &good).unwrap(), bytes);
    assert_eq!(staging.staged_parts(&id, MACBOOK), 0);

    // Room for STAGED_MAX moves at once; old ones are swept.
    let small = part(&part_bodies(b"{}")[0]);
    // Two moves from one Mac, four in all.
    for _ in 0..STAGED_PER_MAC {
        staging
            .put(&new_move_id(), MACBOOK, 0, &small, now)
            .unwrap();
    }
    let third = staging.put(&new_move_id(), MACBOOK, 0, &small, now);
    assert_eq!(third.unwrap_err().code, "busy", "two per Mac");
    for _ in 0..STAGED_PER_MAC {
        staging.put(&new_move_id(), MINI, 0, &small, now).unwrap();
    }
    let fifth = staging.put(&new_move_id(), "m-0000000000000001", 0, &small, now);
    assert_eq!(fifth.unwrap_err().code, "busy", "four in all");
    let later = now + STAGED_FOR + Duration::from_secs(1);
    staging
        .put(&new_move_id(), MACBOOK, 0, &small, later)
        .unwrap();
    // Bad ids and sizes.
    assert!(staging.put("mv-nope", MACBOOK, 0, &small, now).is_err());
    let huge = PartBody {
        of: Some(1),
        data: Some(base64::engine::general_purpose::STANDARD.encode(vec![0u8; PART_MAX + 1])),
    };
    assert_eq!(
        staging
            .put(&new_move_id(), MACBOOK, 0, &huge, later)
            .unwrap_err()
            .code,
        "too_large"
    );
    assert!(valid_move_id(&new_move_id()));
    assert!(!valid_move_id("mv-ABCDEF0123456789"));
}

#[test]
fn the_place_is_checked_on_the_receiving_mac() {
    let two = two();
    let k = key(8);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let id = new_move_id();
    let (bundle, _) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let with_place = |place: Place| {
        let mut b = bundle.clone();
        b.place = place;
        import_move(&two.b, &b, MACBOOK, Utc::now())
            .unwrap_err()
            .code
    };
    let path = |p: &str| Place {
        workspace_id: None,
        vault_path: Some(p.into()),
        worktree: false,
    };
    // A synced symlink leading outside the vault.
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), two.b.join("Notes")).unwrap();
    assert_eq!(with_place(path("Notes")), "bad_request");
    // A link to the Main agent's folder, the folder itself, a dot-folder.
    let main = crate::workspaces::ensure_main_agent(&two.b).unwrap();
    std::os::unix::fs::symlink(&main, two.b.join("Inbox")).unwrap();
    assert_eq!(with_place(path("Inbox")), "conflict");
    assert_eq!(with_place(path("workspaces/main")), "conflict");
    std::fs::create_dir_all(two.b.join(".git")).unwrap();
    assert_eq!(with_place(path(".git")), "bad_request");
    // A build workspace's worktree the mini has no live binding for.
    let worktree = Place {
        workspace_id: Some(crate::workspaces::QUICK_CHATS_ID.into()),
        vault_path: None,
        worktree: true,
    };
    assert_eq!(with_place(worktree), "not_here");
    assert!(store::session_by_key(&two.b, &k).unwrap().is_none());
}

#[test]
fn a_bundle_with_unfinished_turns_or_unknown_controls_is_refused() {
    let two = two();
    let k = key(9);
    seed(&two.a, &k, &[("One", "Yes."), ("Two", "Sure.")]);
    let id = new_move_id();
    let (bundle, _) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let refused = |edit: &dyn Fn(&mut MoveBundle)| {
        let mut b = bundle.clone();
        edit(&mut b);
        import_move(&two.b, &b, MACBOOK, Utc::now())
            .unwrap_err()
            .code
    };
    assert_eq!(
        refused(&|b| b.turns[1].status = Some("running".into())),
        "bad_request"
    );
    assert_eq!(refused(&|b| b.turns[1].status = None), "bad_request");
    assert_eq!(
        refused(&|b| b.turns[0].permission_mode = Some("yolo".into())),
        "bad_request"
    );
    assert!(store::session_by_key(&two.b, &k).unwrap().is_none());
    let mut fine = bundle.clone();
    fine.effort = Some(crate::effort::ULTRACODE.into());
    import_move(&two.b, &fine, MACBOOK, Utc::now()).unwrap();
}

#[test]
fn keep_it_here_with_the_other_mac_out_of_reach_waits_for_its_answer() {
    let two = two();
    let k = key(10);
    seed(&two.a, &k, &[("One", "Yes.")]);
    // The commit landed, its answer was lost, the mini is out of reach.
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    begin_abort(&two.a, &id, Utc::now()).unwrap();
    begin_abort(&two.a, &id, Utc::now()).unwrap();
    // It runs nowhere here until the mini answers: one owner, the mini.
    assert!(admission_refusal(&two.a, &k).is_some());
    assert_eq!(admission_refusal(&two.b, &k), None);
    assert_eq!(
        pending_aborts(&two.a).unwrap(),
        vec![(id.clone(), k.clone(), MINI.to_string())]
    );
    // Back in reach: a turn already ran there, so the mini keeps it.
    let cwd = store::session_by_key(&two.b, &k).unwrap().unwrap().cwd;
    store::start_turn(&two.b, &k, "claude", &cwd, "run-mini", "More").unwrap();
    let said = cancel_move_in(&two.b, &id, MACBOOK, Utc::now()).unwrap_err();
    assert_eq!(said.code, "started");
    settle_abort(&two.a, &id, MINI, TargetSaid::Kept, Utc::now()).unwrap();
    settle_abort(&two.a, &id, MINI, TargetSaid::Kept, Utc::now()).unwrap();
    let done = begin_abort(&two.a, &id, Utc::now()).unwrap_err();
    assert_eq!(
        done.code, "conflict",
        "a finished move is never taken for one being called off"
    );
    assert_eq!(
        home_of(&two.a, &k).unwrap().unwrap().state,
        AWAY,
        "a done move stays done"
    );
    assert_eq!(home_of(&two.a, &k).unwrap().unwrap().state, AWAY);
    assert!(pending_aborts(&two.a).unwrap().is_empty());

    // Another chat: nothing ran there, the mini removes it, home again.
    let k = key(11);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    begin_abort(&two.a, &id, Utc::now()).unwrap();
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now()).unwrap(),
        Cancelled::Removed
    );
    settle_abort(
        &two.a,
        &id,
        MINI,
        TargetSaid::Cancelled(Cancelled::Removed),
        Utc::now(),
    )
    .unwrap();
    assert_eq!(
        begin_abort(&two.a, &id, Utc::now()).unwrap(),
        Aborting::Aborted
    );
    assert_eq!(home_of(&two.a, &k).unwrap(), None);
    assert_eq!(admission_refusal(&two.a, &k), None);
    assert_eq!(owners(&two, &k), 1);
    // The same move id again is a clean conflict, not a database error.
    let again = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap_err();
    assert_eq!(again.code, "conflict");
}

#[test]
fn an_abort_needs_the_other_macs_answer() {
    let two = two();
    let k = key(12);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    // Settling needs an aborting move, and a DELETE answer.
    let early = settle_abort(&two.a, &id, MINI, TargetSaid::Kept, Utc::now()).unwrap_err();
    assert_eq!(early.code, "conflict");
    begin_abort(&two.a, &id, Utc::now()).unwrap();
    let wrong = settle_abort(&two.a, &id, MINI, TargetSaid::CommitRefused, Utc::now()).unwrap_err();
    assert_eq!(wrong.code, "bad_request");
    assert_eq!(home_of(&two.a, &k).unwrap().unwrap().state, LEAVING);
    // A commit refused outright is proof enough from leaving.
    let k2 = key(13);
    seed(&two.a, &k2, &[("One", "Yes.")]);
    seed(&two.b, &k2, &[("Other", "Mine.")]);
    let id2 = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k2, MACBOOK, MINI, &id2, Utc::now()).unwrap();
    assert_eq!(send(&two.b, &id2, &bytes).unwrap_err().code, "conflict");
    abort_move_out(&two.a, &id2, MINI, TargetSaid::CommitRefused, Utc::now()).unwrap();
    assert_eq!(home_of(&two.a, &k2).unwrap(), None);
}

#[test]
fn the_person_releases_a_chat_whose_other_mac_is_gone() {
    let two = two();
    // Stuck aborting: the mini never answers again.
    let k = key(14);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let id = new_move_id();
    begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    begin_abort(&two.a, &id, Utc::now()).unwrap();
    assert!(admission_refusal(&two.a, &k).is_some());
    force_release(&two.a, &id, UserConfirmed::by_the_person(), Utc::now()).unwrap();
    force_release(&two.a, &id, UserConfirmed::by_the_person(), Utc::now()).unwrap();
    assert_eq!(home_of(&two.a, &k).unwrap(), None);
    assert_eq!(admission_refusal(&two.a, &k), None);
    assert!(pending_aborts(&two.a).unwrap().is_empty());
    // Away on a Mac that was forgotten.
    let k = key(15);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    force_release(&two.a, &id, UserConfirmed::by_the_person(), Utc::now()).unwrap();
    assert_eq!(admission_refusal(&two.a, &k), None);
    // A released move is not settled or aborted afterwards.
    let late = abort_move_out(&two.a, &id, MINI, TargetSaid::CommitRefused, Utc::now());
    assert_eq!(late.unwrap_err().code, "conflict");
}

#[test]
fn an_effort_hollow_doesnt_know_never_stops_a_move() {
    let two = two();
    let k = key(16);
    seed(
        &two.a,
        &k,
        &[("One", "Yes."), ("Two", "Sure."), ("Three", "Fine.")],
    );
    // The chat's own effort is "high" (seed); turns hold what they recorded.
    store::record_turn_effort(&two.a, &k, 1, "minimal").unwrap();
    store::record_turn_effort(&two.a, &k, 2, "ultra").unwrap();
    store::record_turn_effort(&two.a, &k, 3, crate::effort::ULTRACODE).unwrap();
    let id = new_move_id();
    let (bundle, _) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    assert_eq!(bundle.effort.as_deref(), Some("high"));
    let sent: Vec<_> = bundle.turns.iter().map(|t| t.effort.as_deref()).collect();
    assert_eq!(sent, [Some("low"), None, Some(crate::effort::ULTRACODE)]);
    // A receiver given unknown ones anyway drops them and imports the rest.
    let mut odd = bundle.clone();
    odd.effort = Some("extreme".into());
    odd.turns[0].effort = Some("huge".into());
    odd.turns[1].effort = Some("minimal".into());
    import_move(&two.b, &odd, MACBOOK, Utc::now()).unwrap();
    assert_eq!(store::chat_effort(&two.b, &k).unwrap(), None);
    let stored: Vec<Option<String>> = (1..=3)
        .map(|ord| {
            crate::session_store::with_read(&two.b, |conn| {
                conn.query_row(
                    "SELECT effort FROM turns WHERE session_key = ?1 AND ord = ?2",
                    rusqlite::params![k, ord],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())
            })
            .unwrap()
        })
        .collect();
    assert_eq!(
        stored,
        [
            None,
            Some("low".to_string()),
            Some(crate::effort::ULTRACODE.to_string())
        ]
    );
}

#[test]
fn an_answer_counts_only_from_the_moves_own_mac_and_state() {
    let two = two();
    let k = key(17);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let id = new_move_id();
    begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let stranger = "m-0000000000000001";
    let none_there = TargetSaid::Cancelled(Cancelled::NotImported);
    let from_stranger = abort_move_out(&two.a, &id, stranger, none_there, Utc::now());
    assert_eq!(from_stranger.unwrap_err().code, "forbidden");
    begin_abort(&two.a, &id, Utc::now()).unwrap();
    let settle = settle_abort(&two.a, &id, stranger, none_there, Utc::now());
    assert_eq!(settle.unwrap_err().code, "forbidden");
    // A commit refusal settles only a move still leaving.
    let late = abort_move_out(&two.a, &id, MINI, TargetSaid::CommitRefused, Utc::now());
    assert_eq!(late.unwrap_err().code, "conflict");
    assert_eq!(home_of(&two.a, &k).unwrap().unwrap().state, LEAVING);
    assert!(admission_refusal(&two.a, &k).is_some());
}

#[test]
fn a_release_over_a_home_that_changed_is_refused_and_changes_nothing() {
    let two = two();
    let k = key(18);
    seed(&two.a, &k, &[("One", "Yes.")]);
    let id = new_move_id();
    begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    // Someone rewrote the home record under the move.
    crate::session_store::with_write(&two.a, "test", |tx| {
        tx.execute(
            "UPDATE chat_homes SET move_id = 'mv-ffffffffffffffff' WHERE session_key = ?1",
            rusqlite::params![k],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let none_there = TargetSaid::Cancelled(Cancelled::NotImported);
    let refused = abort_move_out(&two.a, &id, MINI, none_there, Utc::now()).unwrap_err();
    assert_eq!(refused.code, "conflict");
    // The move is still leaving: the transaction rolled back.
    assert_eq!(
        begin_abort(&two.a, &id, Utc::now()).unwrap(),
        Aborting::Waiting
    );
}

/// `force_release` and its proof stay off every route an agent, a phone,
/// another Mac or `hollow serve` reaches (PR2 wires them to the person's
/// own clicks only).
#[test]
fn no_route_or_action_module_can_release_a_chat() {
    for (name, source) in [
        (
            "phone_access/routes.rs",
            include_str!("phone_access/routes.rs"),
        ),
        ("app_actions.rs", include_str!("app_actions.rs")),
        ("app_mcp.rs", include_str!("app_mcp.rs")),
        ("serve_actions.rs", include_str!("serve_actions.rs")),
        ("serve.rs", include_str!("serve.rs")),
        ("commands/peer_api.rs", include_str!("commands/peer_api.rs")),
    ] {
        for word in ["force_release", "UserConfirmed", "by_the_person"] {
            assert!(!source.contains(word), "{name} names {word}");
        }
    }
}
