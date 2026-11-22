//! CH1 tests: a branch continues from the chosen turn, the original is
//! unchanged, and the refusals. The worktree case (CP1's checkpoint kept)
//! is in `turn_checkpoints_tests.rs`, beside CP1's git rig.

use super::*;
use crate::session_store as store;

/// A fixed time: tests never read the clock.
fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-18T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn vault() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    // The store is keyed by the vault folder's name: one per test.
    let vault = tmp.path().join(tmp.path().file_name().unwrap());
    std::fs::create_dir_all(&vault).unwrap();
    (tmp, vault)
}

fn turn(vault: &Path, key: &str, run: &str, asked: &str, answer: &str) -> i64 {
    let cwd = vault.to_string_lossy().into_owned();
    let ord = store::start_turn(vault, key, "claude", &cwd, run, asked).unwrap();
    store::append_event(vault, key, ord, "assistant", answer).unwrap();
    store::finish_turn(vault, key, ord, "success").unwrap();
    ord
}

/// Chat `key`'s turns `(ord, run id, asked, reply)` and its session row
/// `(title, turn_count, last_activity_at, parent_key, fork_kind)`.
#[allow(clippy::type_complexity)]
fn snapshot(
    vault: &Path,
    key: &str,
) -> (
    Vec<(i64, Option<String>, String, String)>,
    (Option<String>, i64, String, Option<String>, Option<String>),
) {
    store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT ord, run_id, asked FROM turns WHERE session_key = ?1 ORDER BY ord")
            .unwrap();
        let turns = stmt
            .query_map(params![key], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .map(|r| {
                let (ord, run, asked) = r.unwrap();
                let reply = store::transcript_text(conn, key, ord).unwrap();
                (ord, run, asked, reply)
            })
            .collect();
        let row = conn
            .query_row(
                "SELECT title, turn_count, last_activity_at, parent_key, fork_kind \
                 FROM sessions WHERE key = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        Ok((turns, row))
    })
    .unwrap()
}

fn history(vault: &Path, key: &str, before: i64) -> Vec<String> {
    store::conversation_entries(vault, key, before)
        .unwrap()
        .into_iter()
        .map(|e| e.text)
        .collect()
}

#[test]
fn a_branch_continues_from_the_chosen_turn_and_the_original_is_unchanged() {
    let (_tmp, v) = vault();
    let k = "s-20261118-ch1a0001";
    turn(&v, k, "run-1", "Plan the sidebar", "One");
    turn(&v, k, "run-2", "Try tabs", "Two");
    turn(&v, k, "run-3", "Try a tree", "Three");
    store::rename_session(&v, k, "Sidebar").unwrap();
    // A task packet's constraints and a check's step, which a fresh
    // session's task-state record reads.
    store::with_write(&v, "test packet", |tx| {
        tx.execute(
            "UPDATE turns SET packet_constraints = '[{\"text\":\"keep it\",\"from\":\"user\"}]' \
             WHERE session_key = ?1 AND ord = 1",
            params![k],
        )
        .and_then(|_| {
            tx.execute(
                "INSERT INTO steps(session_key, turn_ord, seq, run_id, kind, tool_id, started_at, \
                 status, checkpoint) VALUES(?1, 1, 1, 'run-1', 'tool', 't1', 'x', 'ok', \
                 '{\"checkOutcome\":\"passed\"}')",
                params![k],
            )
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let before = snapshot(&v, k);

    let b = branch(&v, k, "run-2", at()).unwrap();
    assert_eq!(b.through, 2);
    assert_eq!(b.turns, 2);
    assert_eq!(b.title.as_deref(), Some("Sidebar (branch)"));
    assert_eq!(b.checkpoint, None, "not an isolated worktree");
    assert_ne!(b.key, k);

    // The original is unchanged.
    assert_eq!(snapshot(&v, k), before);
    // The branch holds the turns up to the chosen one, with their replies.
    let (turns, row) = snapshot(&v, &b.key);
    // The copies carry no run id: a run id names one turn, the original's.
    let without_runs: Vec<_> = before.0[..2]
        .iter()
        .map(|(o, _, a, r)| (*o, None, a.clone(), r.clone()))
        .collect();
    assert_eq!(turns, without_runs);
    assert_eq!(
        store::session_for_run(&v, "run-2").unwrap().map(|s| s.key),
        Some(k.to_string())
    );
    assert_eq!(
        crate::step_spine::turn_of_run(&v, "run-1").unwrap(),
        Some((k.to_string(), 1))
    );
    let copied: (Option<String>, i64, Option<String>) = store::with_read(&v, |c| {
        c.query_row(
            "SELECT t.packet_constraints, \
             (SELECT COUNT(*) FROM steps s WHERE s.session_key = ?1 AND s.turn_ord = 1), \
             (SELECT s.run_id FROM steps s WHERE s.session_key = ?1 AND s.turn_ord = 1) \
             FROM turns t WHERE t.session_key = ?1 AND t.ord = 1",
            params![b.key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    assert!(copied.0.unwrap().contains("keep it"));
    assert_eq!((copied.1, copied.2), (1, None));
    assert_eq!(row.0.as_deref(), Some("Sidebar (branch)"));
    assert_eq!(row.1, 2);
    assert_eq!(row.3.as_deref(), Some(k));
    assert_eq!(row.4.as_deref(), Some(FORK_KIND));
    // No native session: its next turn starts fresh with its own turns.
    let state = store::resume_state(&v, &b.key).unwrap();
    assert!(state.open.as_ref().unwrap().harness_session_id.is_none());
    let words = history(&v, &b.key, 3);
    assert!(words.iter().any(|w| w.contains("Two")), "{words:?}");
    assert!(!words.iter().any(|w| w.contains("Three")), "{words:?}");

    // The branch continues; the original still is as it was.
    let next = turn(&v, &b.key, "run-b1", "Try a rail", "Rail works");
    assert_eq!(next, 3);
    assert_eq!(snapshot(&v, k), before);
    assert!(history(&v, k, 4).iter().any(|w| w.contains("Three")));

    let info = info(&v, &b.key).unwrap().unwrap();
    assert_eq!(info.from_key, k);
    assert_eq!(info.from_title.as_deref(), Some("Sidebar"));
    assert_eq!((info.through, info.own_turns), (2, 1));
    assert_eq!(info.checkpoint, None);
    assert_eq!(super::info(&v, k).unwrap(), None);

    // Leaving: the written handoff of the branch's own turns only.
    let text = handoff(&v, &b.key).unwrap();
    assert!(text.contains("after turn 2"), "{text}");
    assert!(text.contains("Objective: Try a rail"), "{text}");
    assert!(text.contains("State: Rail works"), "{text}");
    assert!(
        text.contains(&format!("Evidence: branch chat {}", b.key)),
        "{text}"
    );
    assert!(text.ends_with("Next action: "), "{text}");
    assert!(!text.contains("Try tabs"), "{text}");
    assert!(!text.contains("refs/hollow"), "{text}");
}

#[test]
fn a_branch_is_refused_where_it_would_mislead() {
    let (_tmp, v) = vault();
    let k = "s-20261118-ch1a0002";
    turn(&v, k, "run-1", "One", "1");
    let cwd = v.to_string_lossy().into_owned();
    let running = store::start_turn(&v, k, "claude", &cwd, "run-2", "Two").unwrap();

    let err = branch(&v, k, "run-2", at()).unwrap_err();
    assert!(err.contains("still running"), "{err}");
    let err = branch(&v, k, "run-9", at()).unwrap_err();
    assert!(err.contains("isn't in this chat"), "{err}");
    let err = branch(&v, k, "  ", at()).unwrap_err();
    assert!(err.contains("no run"), "{err}");
    let err = branch(&v, "../x", "run-1", at()).unwrap_err();
    assert!(err.contains("isn't a chat"), "{err}");
    let off = "s-20261118-ch1a0009";
    turn(&v, off, "run-o", "Offline", "o");
    store::with_write(&v, "test offline", |tx| {
        tx.execute(
            "UPDATE sessions SET fork_kind = 'offline' WHERE key = ?1",
            params![off],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let err = branch(&v, off, "run-o", at()).unwrap_err();
    assert!(err.contains("offline copy"), "{err}");
    store::finish_turn(&v, k, running, "success").unwrap();

    // A turn a CP1 restore undid.
    store::append_event(
        &v,
        k,
        2,
        crate::turn_checkpoints::RESTORE_EVENT,
        r#"{"before":2,"through":2}"#,
    )
    .unwrap();
    let err = branch(&v, k, "run-2", at()).unwrap_err();
    assert!(err.contains("restore undid"), "{err}");

    // A chat that moved to another Mac.
    store::with_write(&v, "test home", |tx| {
        tx.execute(
            "INSERT INTO chat_homes(session_key, state, peer, move_id, at) \
             VALUES(?1, 'away', 'peer-1', 'm-1', '2026-11-18T00:00:00Z')",
            params![k],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let err = branch(&v, k, "run-1", at()).unwrap_err();
    assert!(err.contains("another Mac"), "{err}");

    // Nothing was made by any refusal.
    let branches: i64 = store::with_read(&v, |c| {
        c.query_row("SELECT COUNT(*) FROM chat_branches", [], |r| r.get(0))
            .map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!(branches, 0);
}

#[test]
fn a_restore_before_the_branch_point_stays_undone_in_the_branch() {
    let (_tmp, v) = vault();
    let k = "s-20261118-ch1a0003";
    turn(&v, k, "run-1", "One", "kept-one");
    turn(&v, k, "run-2", "Two", "undone-two");
    store::append_event(
        &v,
        k,
        2,
        crate::turn_checkpoints::RESTORE_EVENT,
        r#"{"before":2,"through":2}"#,
    )
    .unwrap();
    turn(&v, k, "run-3", "Three", "kept-three");

    let b = branch(&v, k, "run-3", at()).unwrap();
    let words = history(&v, &b.key, 4);
    assert!(words.iter().any(|w| w.contains("kept-one")), "{words:?}");
    assert!(words.iter().any(|w| w.contains("kept-three")), "{words:?}");
    assert!(!words.iter().any(|w| w.contains("undone-two")), "{words:?}");

    // No turns of its own yet: no handoff.
    let err = handoff(&v, &b.key).unwrap_err();
    assert!(err.contains("no turns of its own"), "{err}");
    assert!(handoff(&v, k).unwrap_err().contains("isn't a branch"));
}
