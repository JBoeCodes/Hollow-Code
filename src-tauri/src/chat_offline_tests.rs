//! M5 PR3: offline sync between two Macs' Sessions DBs (two vaults in one
//! process): a chat moved to the "mini" is continued on the "MacBook" while
//! the mini is out of reach, then synced: appended once, a second delivery a
//! no-op, a fork with both histories when the mini moved on, a partial
//! delivery completed, a refusal kept here, and one owner per chat at every
//! step. The same over real routes: `phone_access/move_send_tests.rs`.
use super::*;
use crate::chat_move::{
    abort_move_out, admission_refusal, begin_move_out, cancel_move_in, commit, finish_move_out,
    home_of, import_move, move_status, new_move_id, part_bodies, sha256_hex, staging, CommitBody,
    PartBody, TargetSaid, LEAVING,
};
use crate::chat_move_send;
use crate::session_store as store;
use chrono::Utc;
use std::path::PathBuf;
use std::time::Instant;

/// A Mac id of these tests' own (not `peer_tests::MACBOOK`).
const MACBOOK: &str = "m-3f9c1a07e2b45d19";
const MINI: &str = "m-8d21c4be90f17a3e";

struct Two {
    _a: tempfile::TempDir,
    _b: tempfile::TempDir,
    /// The MacBook.
    a: PathBuf,
    /// The mini.
    b: PathBuf,
}

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
    format!("s-20261115-0f{n:06}")
}

/// One finished turn of chat `key` in `vault`, run id `run`.
fn turn(vault: &Path, key: &str, run: &str, asked: &str, answer: &str) {
    let cwd = store::session_by_key(vault, key)
        .unwrap()
        .map(|s| s.cwd)
        .unwrap_or_else(|| {
            crate::workspaces::ensure_quick_chats(vault)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        });
    let ord = store::start_turn(vault, key, "claude", &cwd, run, asked).unwrap();
    store::record_turn_permission(vault, key, ord, "plan").unwrap();
    store::append_event(vault, key, ord, "assistant", answer).unwrap();
    store::finish_turn(vault, key, ord, "success").unwrap();
}

/// `(ord, run id, asked)` of chat `key`'s turns, in order, with whether
/// each has a transcript.
fn turns_of(vault: &Path, key: &str) -> Vec<(i64, String, String, bool)> {
    store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT t.ord, t.run_id, t.asked, x.turn_ord IS NOT NULL FROM turns t \
                 LEFT JOIN transcripts x ON x.session_key = t.session_key AND x.turn_ord = t.ord \
                 WHERE t.session_key = ?1 ORDER BY t.ord",
            )
            .unwrap();
        let rows = stmt
            .query_map(params![key], |row| {
                Ok((
                    row.get(0)?,
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    row.get(3)?,
                ))
            })
            .unwrap();
        Ok(rows.map(Result::unwrap).collect())
    })
    .unwrap()
}

fn runs(vault: &Path, key: &str) -> Vec<String> {
    turns_of(vault, key).into_iter().map(|t| t.1).collect()
}

/// `(parent_key, fork_kind)` of chat `key`.
fn lineage(vault: &Path, key: &str) -> (Option<String>, Option<String>) {
    store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT parent_key, fork_kind FROM sessions WHERE key = ?1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap()
}

/// Send `bytes` as parts and commit them, as the sending Mac does.
fn send(vault: &Path, move_id: &str, bytes: &[u8]) -> Result<Imported, MoveError> {
    let bodies = part_bodies(bytes);
    for (n, body) in bodies.iter().enumerate() {
        let body: PartBody = serde_json::from_value(body.clone()).unwrap();
        // `vault`'s own staging area: tests running alongside have theirs.
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

/// Chat `k` with two exchanges on the MacBook, moved to the mini.
fn moved(two: &Two, k: &str) {
    turn(
        &two.a,
        k,
        &format!("{k}-1"),
        "Plan the trip",
        "Here is a plan.",
    );
    turn(&two.a, k, &format!("{k}-2"), "Shorter", "Two days.");
    store::set_chat_effort(&two.a, k, Some("high")).unwrap();
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
}

/// Sync offline chat `o` from the MacBook to the mini: begin, deliver,
/// finish, and fold the answer in. The import's answer.
fn sync(two: &Two, o: &str) -> Imported {
    let id = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    assert_eq!(bundle.version, chat_move::APPEND_VERSION);
    let imported = send(&two.b, &id, &bytes).unwrap();
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    let status = move_status(&two.b, &id, MACBOOK).unwrap();
    assert_eq!(status.state, "imported");
    fold(
        &two.a,
        o,
        &id,
        status.outcome.as_deref().unwrap(),
        status.key.as_deref().unwrap(),
        Utc::now(),
    )
    .unwrap();
    imported
}

/// Which Macs own chat `key`: has it and isn't `away` (nor folded away).
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
fn offline_turns_sync_once_and_a_second_sync_adds_nothing() {
    let two = two();
    let k = key(1);
    moved(&two, &k);
    assert!(admission_refusal(&two.a, &k).is_some());

    // Continue here offline: a chat of the MacBook's own, a copy of K.
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    assert_ne!(o, k);
    assert_eq!(
        continue_offline(&two.a, &k, Utc::now()).unwrap(),
        o,
        "one open"
    );
    assert_eq!(
        lineage(&two.a, &o),
        (Some(k.clone()), Some(FORK_KIND.into()))
    );
    assert_eq!(runs(&two.a, &o), runs(&two.a, &k));
    assert_eq!(admission_refusal(&two.a, &o), None, "it runs here");
    assert!(admission_refusal(&two.a, &k).is_some(), "K still doesn't");
    assert_eq!(owners(&two, &k), 1);
    assert_eq!(owners(&two, &o), 1);
    assert_eq!(
        store::chat_effort(&two.a, &o).unwrap().as_deref(),
        Some("high")
    );
    // Its next turn starts a fresh native session with the history.
    let state = store::resume_state(&two.a, &o).unwrap();
    assert!(state.open.as_ref().unwrap().harness_session_id.is_none());

    turn(&two.a, &o, "off-1", "Add a museum", "Added the Prado.");
    turn(&two.a, &o, "off-2", "And lunch", "Booked lunch.");
    let id = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    // Only the delta travels, after the last common turn.
    let append = bundle.append.clone().unwrap();
    assert_eq!(append.to, k);
    assert_eq!(append.base.ord, 2);
    assert_eq!(
        append.base.run_id.as_deref(),
        Some(format!("{k}-2").as_str())
    );
    assert_eq!(bundle.turns.len(), 2);
    assert!(bundle.native.is_none());
    // While it is sent, it runs nowhere.
    assert_eq!(home_of(&two.a, &o).unwrap().unwrap().state, LEAVING);
    assert!(admission_refusal(&two.a, &o).is_some());

    let imported = send(&two.b, &id, &bytes).unwrap();
    assert_eq!(
        (imported.key.as_str(), imported.turns, imported.already),
        (k.as_str(), 2, false)
    );
    assert_eq!(imported.outcome, Some(APPENDED));
    let want = vec![
        format!("{k}-1"),
        format!("{k}-2"),
        "off-1".to_string(),
        "off-2".to_string(),
    ];
    assert_eq!(runs(&two.b, &k), want);
    assert!(turns_of(&two.b, &k).iter().all(|t| t.3), "transcripts too");
    assert!(store::session_by_key(&two.b, &o).unwrap().is_none());

    // A repeated commit and a repeated import change nothing.
    let repeat = import_move(&two.b, &bundle, MACBOOK, Utc::now()).unwrap();
    assert!(repeat.already);
    assert_eq!(repeat.outcome, Some(APPENDED));
    // The same turns under another sync id: each run id once.
    let mut again = bundle.clone();
    again.move_id = new_move_id();
    let second = import_move(&two.b, &again, MACBOOK, Utc::now()).unwrap();
    assert_eq!((second.turns, second.outcome), (0, Some(APPENDED)));
    assert_eq!(runs(&two.b, &k), want);
    // Its DELETE answers started: the turns stand.
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "started"
    );

    // The MacBook folds the answer in: K's replica has the turns, O goes.
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    let status = move_status(&two.b, &id, MACBOOK).unwrap();
    assert_eq!(
        (status.outcome.as_deref(), status.key.as_deref()),
        (Some(APPENDED), Some(k.as_str()))
    );
    fold(&two.a, &o, &id, APPENDED, &k, Utc::now()).unwrap();
    fold(&two.a, &o, &id, APPENDED, &k, Utc::now()).unwrap();
    assert_eq!(runs(&two.a, &k), want);
    let mine: Vec<_> = turns_of(&two.a, &k)
        .into_iter()
        .map(|t| (t.0, t.2))
        .collect();
    let theirs: Vec<_> = turns_of(&two.b, &k)
        .into_iter()
        .map(|t| (t.0, t.2))
        .collect();
    assert_eq!(mine, theirs, "the same ords and words on both Macs");
    assert!(store::session_by_key(&two.a, &o).unwrap().is_none());
    assert!(home_of(&two.a, &o).unwrap().is_none());
    assert!(
        admission_refusal(&two.a, &o).is_some(),
        "O's turns are in K"
    );
    assert_eq!(owners(&two, &k), 1);
    assert_eq!(
        store::session_by_key(&two.a, &k)
            .unwrap()
            .unwrap()
            .turn_count,
        4
    );

    // A second offline stretch builds on the synced turns and appends again.
    let o2 = continue_offline(&two.a, &k, Utc::now()).unwrap();
    assert_ne!(o2, o);
    turn(&two.a, &o2, "off-3", "Dinner too", "Booked dinner.");
    let imported = sync(&two, &o2);
    assert_eq!((imported.turns, imported.outcome), (1, Some(APPENDED)));
    assert_eq!(runs(&two.b, &k).len(), 5);
    assert_eq!(runs(&two.a, &k), runs(&two.b, &k));
}

#[test]
fn a_mini_that_moved_on_gets_a_fork_with_both_histories() {
    let two = two();
    let k = key(2);
    moved(&two, &k);
    // The mini ran a turn of its own (the continuation, say).
    turn(&two.b, &k, "mini-1", "Continue", "Continued.");
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "Add a museum", "Added the Prado.");
    turn(&two.a, &o, "off-2", "And lunch", "Booked lunch.");

    let imported = sync(&two, &o);
    assert_eq!(
        (imported.key.as_str(), imported.turns, imported.outcome),
        (o.as_str(), 2, Some(FORKED))
    );
    // K there is untouched; the fork holds the common turns, then the
    // offline ones. Nothing lost, nothing twice.
    assert_eq!(
        runs(&two.b, &k),
        vec![format!("{k}-1"), format!("{k}-2"), "mini-1".into()]
    );
    assert_eq!(
        runs(&two.b, &o),
        vec![
            format!("{k}-1"),
            format!("{k}-2"),
            "off-1".to_string(),
            "off-2".to_string()
        ]
    );
    assert!(turns_of(&two.b, &o).iter().all(|t| t.3));
    assert_eq!(
        lineage(&two.b, &o),
        (Some(k.clone()), Some(FORK_KIND.into()))
    );
    let arrived = home_of(&two.b, &o).unwrap().unwrap();
    assert_eq!(
        (arrived.state.as_str(), arrived.peer.as_str()),
        (ARRIVED, MACBOOK)
    );
    assert_eq!(admission_refusal(&two.b, &o), None, "the fork runs there");
    assert_eq!(
        store::chat_effort(&two.b, &o).unwrap().as_deref(),
        Some("high")
    );
    // Here: O moved there as itself; one owner each.
    assert_eq!(home_of(&two.a, &o).unwrap().unwrap().state, AWAY);
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, FORKED);
    assert_eq!(runs(&two.a, &o), runs(&two.b, &o));
    assert_eq!(owners(&two, &o), 1);
    assert_eq!(owners(&two, &k), 1);
    // A next offline stretch of K starts a new chat again.
    assert_ne!(continue_offline(&two.a, &k, Utc::now()).unwrap(), o);
}

#[test]
fn a_partial_delivery_completes_and_a_prefix_landed_earlier_counts_once() {
    let two = two();
    let k = key(3);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    turn(&two.a, &o, "off-2", "Two", "2");
    turn(&two.a, &o, "off-3", "Three", "3");
    let id = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();

    // An earlier delivery landed only the first offline turn (a sync cut
    // short, released and sent again): the rest appends, nothing twice.
    let mut first = bundle.clone();
    first.move_id = new_move_id();
    first.turns.truncate(1);
    import_move(&two.b, &first, MACBOOK, Utc::now()).unwrap();
    assert_eq!(runs(&two.b, &k).len(), 3);

    // This delivery's parts arrive, its connection drops before the commit:
    // nothing landed, the parts are staged, and the retry (the same bytes
    // under the same id, O frozen meanwhile) completes it.
    let bodies = part_bodies(&bytes);
    let body: PartBody = serde_json::from_value(bodies[0].clone()).unwrap();
    staging(&two.b)
        .put(&id, MACBOOK, 0, &body, Instant::now())
        .unwrap();
    assert_eq!(
        move_status(&two.b, &id, MACBOOK).unwrap().state,
        "receiving"
    );
    assert!(admission_refusal(&two.a, &o).is_some(), "frozen while sent");
    let (retried, again) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    assert_eq!(retried, bundle);
    assert_eq!(again, bytes);
    let imported = send(&two.b, &id, &again).unwrap();
    assert_eq!((imported.turns, imported.outcome), (2, Some(APPENDED)));
    assert_eq!(
        runs(&two.b, &k),
        vec![
            format!("{k}-1"),
            format!("{k}-2"),
            "off-1".to_string(),
            "off-2".to_string(),
            "off-3".to_string()
        ]
    );
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    fold(&two.a, &o, &id, APPENDED, &k, Utc::now()).unwrap();
    assert_eq!(runs(&two.a, &k), runs(&two.b, &k));
}

#[test]
fn a_refused_sync_keeps_the_offline_chat_here_whole() {
    let two = two();
    let k = key(4);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    // The mini's K is not the one the MacBook knows (its base turn has
    // another run id): refused, nothing lands.
    store::with_write(&two.b, "test", |tx| {
        tx.execute(
            "UPDATE turns SET run_id = 'other' WHERE session_key = ?1 AND ord = 2",
            params![k],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let refused = send(&two.b, &id, &bytes).unwrap_err();
    assert_eq!(refused.code, "conflict");
    assert!(store::session_by_key(&two.b, &o).unwrap().is_none());
    assert_eq!(runs(&two.b, &k).len(), 2);
    abort_move_out(&two.a, &id, MINI, TargetSaid::CommitRefused, Utc::now()).unwrap();
    // Home again. A refusal alone keeps nothing (a tombstone's `cancelled`
    // is sent again); the driver keeps it on a `conflict` or `too_large`
    // (`chat_move_send::deliver`), as here.
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, OPEN);
    keep_move(&two.a, &id, "Refused there.", Utc::now()).unwrap();
    // An ordinary chat now: it runs here and a later move carries it whole
    // (a plain move, not a sync).
    assert!(home_of(&two.a, &o).unwrap().is_none());
    assert_eq!(admission_refusal(&two.a, &o), None);
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, KEPT);
    let (plain, _) = begin_move_out(&two.a, &o, MACBOOK, MINI, &new_move_id(), Utc::now()).unwrap();
    assert!(plain.append.is_none());
    assert_eq!(plain.version, chat_move::BUNDLE_VERSION);
    assert_eq!(plain.turns.len(), 3);
}

#[test]
fn a_chat_that_lives_here_or_came_from_elsewhere_isnt_continued_or_appended() {
    let two = two();
    let k = key(5);
    turn(&two.a, &k, "here-1", "Hi", "Hello.");
    // A chat this Mac owns runs here: nothing to continue offline.
    assert_eq!(
        continue_offline(&two.a, &k, Utc::now()).unwrap_err().code,
        "conflict"
    );
    // An append from a Mac the chat didn't come from is refused.
    moved(&two, &key(6));
    let o = continue_offline(&two.a, &key(6), Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let id = new_move_id();
    let (bundle, _) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let mut stranger = bundle.clone();
    stranger.from = "m-0000000000000001".into();
    assert_eq!(
        import_move(&two.b, &stranger, "m-0000000000000001", Utc::now())
            .unwrap_err()
            .code,
        "conflict"
    );
}

#[test]
fn an_append_is_checked_on_the_receiving_mac() {
    let two = two();
    let k = key(7);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    turn(&two.a, &o, "off-2", "Two", "2");
    let (bundle, _) =
        begin_move_out(&two.a, &o, MACBOOK, MINI, &new_move_id(), Utc::now()).unwrap();
    let refused = |b: &MoveBundle| {
        import_move(&two.b, b, MACBOOK, Utc::now())
            .unwrap_err()
            .code
    };
    let mut b = bundle.clone();
    b.turns[1].run_id = None;
    assert_eq!(refused(&b), "bad_request");
    let mut b = bundle.clone();
    b.turns[1].run_id = b.turns[0].run_id.clone();
    assert_eq!(refused(&b), "bad_request");
    let mut b = bundle.clone();
    b.turns[0].ord = 2;
    assert_eq!(refused(&b), "bad_request", "a turn at or before the base");
    let mut b = bundle.clone();
    b.version = chat_move::BUNDLE_VERSION;
    assert_eq!(refused(&b), "bad_request", "an append is version 2");
    let mut b = bundle.clone();
    b.append.as_mut().unwrap().to = o.clone();
    assert_eq!(refused(&b), "bad_request");
    let mut b = bundle.clone();
    b.turns[0].status = Some("running".into());
    assert_eq!(refused(&b), "bad_request");
    let mut b = bundle.clone();
    b.turns[0].permission_mode = Some("yolo".into());
    assert_eq!(refused(&b), "bad_request");
    // Nothing landed from any of them.
    assert_eq!(runs(&two.b, &k).len(), 2);
    // An effort Hollow doesn't know is dropped, never refused.
    let mut b = bundle.clone();
    b.turns[0].effort = Some("ultra".into());
    assert_eq!(
        import_move(&two.b, &b, MACBOOK, Utc::now())
            .unwrap()
            .outcome,
        Some(APPENDED)
    );
}

#[test]
fn a_forked_sync_called_off_before_a_turn_ran_there_is_undone() {
    let two = two();
    let k = key(8);
    moved(&two, &k);
    turn(&two.b, &k, "mini-1", "Continue", "Continued.");
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    assert_eq!(send(&two.b, &id, &bytes).unwrap().outcome, Some(FORKED));
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now()).unwrap(),
        chat_move::Cancelled::Removed
    );
    assert!(store::session_by_key(&two.b, &o).unwrap().is_none());
    assert_eq!(
        move_status(&two.b, &id, MACBOOK).unwrap().state,
        "cancelled"
    );
    assert_eq!(runs(&two.b, &k).len(), 3);
    abort_move_out(
        &two.a,
        &id,
        MINI,
        TargetSaid::Cancelled(chat_move::Cancelled::Removed),
        Utc::now(),
    )
    .unwrap();
    // Home and still open: the next contact syncs it again.
    assert!(home_of(&two.a, &o).unwrap().is_none());
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, OPEN);
    assert_eq!(sync(&two, &o).outcome, Some(FORKED));
    assert_eq!(runs(&two.b, &o).len(), 3);
}

/// The person's release of a sync between its landing and its fold, done
/// as `chat_move::force_release` does (the token is made only in the
/// person's own commands): the move `released`, the home record gone.
fn release_by_hand(vault: &Path, move_id: &str, key: &str) {
    store::with_write(vault, "test", |tx| {
        tx.execute(
            "UPDATE chat_moves SET state = 'released' WHERE move_id = ?1",
            params![move_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM chat_homes WHERE session_key = ?1",
            params![key],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

/// Begin and deliver offline chat `o`'s sync; finish it here, no fold.
fn deliver(two: &Two, o: &str) -> (String, Imported) {
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let imported = send(&two.b, &id, &bytes).unwrap();
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    (id, imported)
}

/// M1: an appended sync sent again under a new id (released before its
/// fold) after the chat there moved on, or was archived, adds nothing and
/// makes no fork of copies.
#[test]
fn an_appended_sync_sent_again_after_the_chat_moved_on_adds_nothing() {
    let two = two();
    let k = key(9);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    turn(&two.a, &o, "off-2", "Two", "2");
    let (id, imported) = deliver(&two, &o);
    assert_eq!(imported.outcome, Some(APPENDED));
    release_by_hand(&two.a, &id, &o);
    // The chat there runs a turn of its own, then the sync comes again.
    turn(&two.b, &k, "mini-1", "Go on", "Went on.");
    let (id, again) = deliver(&two, &o);
    assert_eq!(
        (again.outcome, again.turns, again.key.as_str()),
        (Some(APPENDED), 0, k.as_str())
    );
    assert!(
        store::session_by_key(&two.b, &o).unwrap().is_none(),
        "no fork"
    );
    let there = runs(&two.b, &k);
    assert_eq!(
        there,
        vec![
            format!("{k}-1"),
            format!("{k}-2"),
            "off-1".into(),
            "off-2".into(),
            "mini-1".into()
        ]
    );
    // Its DELETE answers started.
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "started"
    );
    // Released again and the chat there archived: still nothing added, no
    // fork.
    release_by_hand(&two.a, &id, &o);
    store::with_write(&two.b, "test", |tx| {
        tx.execute(
            "UPDATE sessions SET archived = 1 WHERE key = ?1",
            params![k],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let (id, again) = deliver(&two, &o);
    assert_eq!((again.outcome, again.turns), (Some(APPENDED), 0));
    assert!(
        store::session_by_key(&two.b, &o).unwrap().is_none(),
        "no fork"
    );
    assert_eq!(runs(&two.b, &k), there);
    // The fold here takes each turn once.
    fold(&two.a, &o, &id, APPENDED, &k, Utc::now()).unwrap();
    assert_eq!(
        runs(&two.a, &k),
        vec![
            format!("{k}-1"),
            format!("{k}-2"),
            "off-1".into(),
            "off-2".into()
        ]
    );
}

/// M1: a forked sync released before its fold and sent again goes to the
/// fork it made, adding nothing; new offline turns since join that fork.
#[test]
fn a_forked_sync_sent_again_lands_in_its_own_fork_once() {
    let two = two();
    let k = key(10);
    moved(&two, &k);
    turn(&two.b, &k, "mini-1", "Go on", "Went on.");
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let (id, first) = deliver(&two, &o);
    assert_eq!(first.outcome, Some(FORKED));
    release_by_hand(&two.a, &id, &o);
    let (_, again) = deliver(&two, &o);
    assert_eq!((again.outcome, again.turns), (Some(FORKED), 0));
    assert_eq!(runs(&two.b, &o).len(), 3);
    assert_eq!(runs(&two.b, &k).len(), 3, "K untouched");
    // Released again, one more offline turn: it joins the fork, once.
    let id = home_of(&two.a, &o).unwrap().unwrap().move_id;
    release_by_hand(&two.a, &id, &o);
    turn(&two.a, &o, "off-2", "Two", "2");
    let (id, more) = deliver(&two, &o);
    assert_eq!(
        (more.outcome, more.turns, more.key.as_str()),
        (Some(FORKED), 1, o.as_str())
    );
    assert_eq!(
        runs(&two.b, &o),
        vec![
            format!("{k}-1"),
            format!("{k}-2"),
            "off-1".into(),
            "off-2".into()
        ]
    );
    // The answer naming the earlier fork stands against a DELETE.
    assert_eq!(
        cancel_move_in(&two.b, &id, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "started"
    );
    fold(&two.a, &o, &id, FORKED, &o, Utc::now()).unwrap();
    assert_eq!(runs(&two.a, &o), runs(&two.b, &o));
}

/// M2: Keep it here on an offline chat's sync keeps it this Mac's: it is
/// never sent again as a sync.
#[test]
fn keep_it_here_on_an_offline_sync_keeps_the_chat_here() {
    let two = two();
    let k = key(11);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let id = new_move_id();
    begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    assert!(ready(&two.a, MINI).unwrap().is_empty(), "leaving");
    chat_move::begin_abort(&two.a, &id, Utc::now()).unwrap();
    let said = cancel_move_in(&two.b, &id, MACBOOK, Utc::now()).unwrap();
    chat_move::settle_abort(&two.a, &id, MINI, TargetSaid::Cancelled(said), Utc::now()).unwrap();
    assert!(home_of(&two.a, &o).unwrap().is_none());
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, KEPT);
    assert!(ready(&two.a, MINI).unwrap().is_empty());
    assert!(pending_peers(&two.a).is_empty());
    assert_eq!(runs(&two.b, &k).len(), 2);
}

/// M3: a chat is kept only while it is home: a sync that is out (leaving)
/// or landed (away) is never marked kept.
#[test]
fn a_sync_that_is_out_or_landed_is_never_kept() {
    let two = two();
    let k = key(12);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let id = new_move_id();
    begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    keep(&two.a, &o, "Kept.", Utc::now()).unwrap();
    keep_move(&two.a, &id, "Refused there.", Utc::now()).unwrap();
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, OPEN);
    let bytes = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now())
        .unwrap()
        .1;
    send(&two.b, &id, &bytes).unwrap();
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    keep(&two.a, &o, "Kept.", Utc::now()).unwrap();
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, OPEN);
    assert_eq!(unfolded(&two.a, MINI).unwrap(), vec![(o.clone(), id)]);
}

/// M4: the caps count the turns a sync carries, not the copy of the chat
/// it continues: a chat near the turn cap still syncs its offline turns.
#[test]
fn a_long_chat_still_syncs_its_offline_turns() {
    let two = two();
    let k = key(13);
    let cwd = crate::workspaces::ensure_quick_chats(&two.a)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let long = chat_move::TURNS_MAX as i64 - 5;
    store::start_turn(&two.a, &k, "claude", &cwd, &format!("{k}-1"), "First").unwrap();
    store::finish_turn(&two.a, &k, 1, "success").unwrap();
    store::with_write(&two.a, "test", |tx| {
        for ord in 2..=long {
            tx.execute(
                "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked) \
                 VALUES(?1, ?2, ?3, '2026-11-15T10:00:00Z', '2026-11-15T10:00:01Z', 'success', 'More')",
                params![k, ord, format!("{k}-{ord}")],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .unwrap();
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &k, MACBOOK, MINI, &id, Utc::now()).unwrap();
    send(&two.b, &id, &bytes).unwrap();
    finish_move_out(&two.a, &id, Utc::now()).unwrap();
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    for n in 0..10 {
        turn(&two.a, &o, &format!("off-{n}"), "Again", "Yes.");
    }
    assert!(runs(&two.a, &o).len() > chat_move::TURNS_MAX);
    let imported = sync(&two, &o);
    assert_eq!((imported.turns, imported.outcome), (10, Some(APPENDED)));
    assert_eq!(runs(&two.b, &k).len() as i64, long + 10);
}

/// M5: turns added to the chat there close its native session (which never
/// saw them): its next turn starts fresh with the whole conversation, and a
/// send naming the old id resets.
#[test]
fn appended_turns_start_a_fresh_native_session_there() {
    use crate::chat_resume::{plan_resume_with, ResetReason, ResumePlan};
    let two = two();
    let k = key(14);
    moved(&two, &k);
    let native = "0b1c2d3e-4f50-4617-8899-aabbccddeeff";
    store::record_harness_id(&two.b, &k, native).unwrap();
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    assert_eq!(sync(&two, &o).outcome, Some(APPENDED));
    let state = store::resume_state(&two.b, &k).unwrap();
    assert!(state.open.as_ref().unwrap().harness_session_id.is_none());
    assert_eq!(
        plan_resume_with(Some(native), &state, "fp", false),
        ResumePlan::Reset(ResetReason::Moved)
    );
    let words: Vec<String> = store::conversation_entries(&two.b, &k, 10)
        .unwrap()
        .into_iter()
        .map(|e| e.text)
        .collect();
    assert!(words.iter().any(|w| w.contains("One")), "{words:?}");
}

/// Lows: a forged run id (one of the common turns) is refused; a sender's
/// future activity time is clamped; a folded turn keeps its feedback vote.
#[test]
fn forged_turns_and_future_times_are_refused_or_clamped_and_votes_kept() {
    let two = two();
    let k = key(15);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let id = new_move_id();
    let (bundle, _) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let mut forged = bundle.clone();
    forged.move_id = new_move_id();
    forged.turns[0].run_id = Some(format!("{k}-1"));
    assert_eq!(
        import_move(&two.b, &forged, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(runs(&two.b, &k).len(), 2);
    let mut future = bundle.clone();
    future.session.last_activity_at = "2999-01-01T00:00:00Z".into();
    import_move(&two.b, &future, MACBOOK, Utc::now()).unwrap();
    let last = store::session_by_key(&two.b, &k)
        .unwrap()
        .unwrap()
        .last_activity_at;
    assert!(!last.starts_with("2999"), "{last}");
    // The vote on the offline turn follows it into K here.
    store::with_write(&two.a, "test", |tx| {
        tx.execute(
            "INSERT INTO turn_feedback(session_key, turn_ord, run_id, vote, snapshot_json, created_at, updated_at) \
             VALUES(?1, 3, 'off-1', 'up', '{}', 'x', 'x')",
            params![o],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    // The real sync (its id begun above) reaches nothing new there now.
    store::with_write(&two.a, "test", |tx| {
        tx.execute("DELETE FROM chat_homes WHERE session_key = ?1", params![o])
            .and_then(|_| tx.execute("DELETE FROM chat_moves WHERE move_id = ?1", params![id]))
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .unwrap();
    let (id, _) = deliver(&two, &o);
    fold(&two.a, &o, &id, APPENDED, &k, Utc::now()).unwrap();
    let vote: (String, i64) = store::with_read(&two.a, |conn| {
        conn.query_row(
            "SELECT session_key, turn_ord FROM turn_feedback WHERE run_id = 'off-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!(vote, (k.clone(), 3));
}

/// Round 2 (1): a re-send whose first turns landed earlier (appended),
/// after the chat there moved on, forks from where the histories part with
/// only the missing turns: nothing in two chats twice. Out of order is
/// refused.
#[test]
fn a_partial_resend_after_the_chat_moved_on_forks_only_the_missing_turns() {
    let two = two();
    let k = key(16);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let (id, first) = deliver(&two, &o);
    assert_eq!((first.outcome, first.turns), (Some(APPENDED), 1));
    release_by_hand(&two.a, &id, &o);
    turn(&two.a, &o, "off-2", "Two", "2");
    turn(&two.b, &k, "mini-1", "Go on", "Went on.");

    let id = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let mut odd = bundle.clone();
    odd.move_id = new_move_id();
    odd.turns[0].run_id = Some("off-x".into());
    odd.turns[1].run_id = Some("off-1".into());
    assert_eq!(
        import_move(&two.b, &odd, MACBOOK, Utc::now())
            .unwrap_err()
            .code,
        "conflict",
        "out of order"
    );
    let forked = send(&two.b, &id, &bytes).unwrap();
    assert_eq!((forked.outcome, forked.turns), (Some(FORKED), 1));
    assert_eq!(
        runs(&two.b, &k),
        vec![
            format!("{k}-1"),
            format!("{k}-2"),
            "off-1".to_string(),
            "mini-1".to_string()
        ]
    );
    assert_eq!(
        runs(&two.b, &o),
        vec![
            format!("{k}-1"),
            format!("{k}-2"),
            "off-1".to_string(),
            "off-2".to_string()
        ],
        "the fork parts after off-1"
    );
    let all: Vec<String> = runs(&two.b, &k)
        .into_iter()
        .chain(runs(&two.b, &o))
        .collect();
    assert_eq!(all.iter().filter(|r| *r == "off-2").count(), 1);
}

/// A link that is never reached (a sync refused before sending).
struct Unreached;

impl MoveLink for Unreached {
    fn peer(&self) -> &str {
        MINI
    }
    fn call<'a>(
        &'a self,
        _route: MoveRoute<'a>,
    ) -> chat_move_send::Fut<
        'a,
        Result<crate::peer_client::MoveAnswer, crate::peer_client::PeerError>,
    > {
        panic!("nothing is sent")
    }
    fn send<'a>(
        &'a self,
        _key: &'a str,
        _text: &'a str,
        _message_id: &'a str,
    ) -> chat_move_send::Fut<'a, Result<crate::peer_client::Accepted, crate::peer_client::PeerError>>
    {
        panic!("nothing is sent")
    }
}

/// `n` finished offline turns in chat `o` from ord `first`, run ids
/// `off-0`, `off-1`, ...
fn offline_turns(vault: &Path, o: &str, first: i64, n: i64) {
    store::with_write(vault, "test", |tx| {
        for i in 0..n {
            tx.execute(
                "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked) \
                 VALUES(?1, ?2, ?3, '2026-11-15T10:00:00.000Z', '2026-11-15T10:00:01.000Z', 'success', 'More')",
                params![o, first + i, format!("off-{i}")],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .unwrap();
}

fn off_runs(n: i64) -> Vec<String> {
    (0..n).map(|i| format!("off-{i}")).collect()
}

/// M5 slicing: offline turns past what one sync carries go in slices. The
/// first lands, the chat comes back here open with its base at the last
/// turn sent, and the next sync carries the rest: all of them end in the
/// chat there and in this Mac's copy, once each, in order.
#[test]
fn an_offline_stretch_past_one_sync_goes_in_slices_and_completes() {
    let two = two();
    let k = key(17);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    let n = chat_move::TURNS_MAX as i64 + 1;
    offline_turns(&two.a, &o, 3, n);

    let first = sync(&two, &o);
    assert_eq!(
        (first.turns, first.outcome),
        (chat_move::TURNS_MAX, Some(APPENDED))
    );
    let open = offline_of(&two.a, &o).unwrap().unwrap();
    assert_eq!(open.state, OPEN);
    assert_eq!(open.base_ord, 2 + chat_move::TURNS_MAX as i64);
    assert_eq!(
        open.base_run.as_deref(),
        Some(format!("off-{}", chat_move::TURNS_MAX - 1).as_str())
    );
    assert!(home_of(&two.a, &o).unwrap().is_none(), "back here");
    assert_eq!(admission_refusal(&two.a, &o), None, "it runs here");
    assert_eq!(runs(&two.a, &o).len() as i64, 2 + n, "nothing left it yet");
    assert_eq!(
        runs(&two.a, &k).len(),
        2,
        "this Mac's copy waits for the end"
    );

    let rest = sync(&two, &o);
    assert_eq!((rest.turns, rest.outcome), (1, Some(APPENDED)));
    let want: Vec<String> = [format!("{k}-1"), format!("{k}-2")]
        .into_iter()
        .chain(off_runs(n))
        .collect();
    assert_eq!(runs(&two.b, &k), want);
    assert_eq!(runs(&two.a, &k), want);
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, APPENDED);
    assert!(store::session_by_key(&two.a, &o).unwrap().is_none());
    assert_eq!(owners(&two, &k), 1);
}

/// M5 slicing: the mini moved on before the first slice, so it forked;
/// the next slice goes into that fork, never a second one.
#[test]
fn a_sliced_stretch_that_forked_sends_the_rest_into_the_fork() {
    let two = two();
    let k = key(21);
    moved(&two, &k);
    turn(&two.b, &k, "mini-1", "Continue", "Continued.");
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    let n = chat_move::TURNS_MAX as i64 + 2;
    offline_turns(&two.a, &o, 3, n);

    let first = sync(&two, &o);
    assert_eq!(
        (first.key.as_str(), first.turns, first.outcome),
        (o.as_str(), chat_move::TURNS_MAX, Some(FORKED))
    );
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, OPEN);
    assert!(home_of(&two.a, &o).unwrap().is_none(), "back here");

    let rest = sync(&two, &o);
    assert_eq!(
        (rest.key.as_str(), rest.turns, rest.outcome),
        (o.as_str(), 2, Some(FORKED))
    );
    let want: Vec<String> = [format!("{k}-1"), format!("{k}-2")]
        .into_iter()
        .chain(off_runs(n))
        .collect();
    assert_eq!(runs(&two.b, &o), want);
    assert_eq!(
        runs(&two.b, &k),
        vec![format!("{k}-1"), format!("{k}-2"), "mini-1".into()]
    );
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, FORKED);
    assert_eq!(home_of(&two.a, &o).unwrap().unwrap().state, AWAY);
    assert_eq!(owners(&two, &o), 1);
}

/// Review fix 1: a slice's answer folded twice (read again, or a retry
/// after the fold) moves the base once and folds nothing early, before and
/// after the next slice is out; the stretch still completes once.
#[test]
fn a_slice_folded_twice_moves_its_base_once() {
    let two = two();
    let k = key(23);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    let n = chat_move::TURNS_MAX as i64 + 3;
    offline_turns(&two.a, &o, 3, n);
    let first = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &first, Utc::now()).unwrap();
    send(&two.b, &first, &bytes).unwrap();
    finish_move_out(&two.a, &first, Utc::now()).unwrap();
    assert_eq!(
        fold(&two.a, &o, &first, APPENDED, &k, Utc::now()).unwrap(),
        Folded::Sliced
    );
    let base = offline_of(&two.a, &o).unwrap().unwrap().base_ord;
    assert_eq!(base, 2 + chat_move::TURNS_MAX as i64);
    // The same answer again: nothing moves, nothing is folded here.
    assert_eq!(
        fold(&two.a, &o, &first, APPENDED, &k, Utc::now()).unwrap(),
        Folded::Stale
    );
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().base_ord, base);
    assert_eq!(runs(&two.a, &k).len(), 2, "nothing folded early");
    assert_eq!(runs(&two.a, &o).len() as i64, 2 + n);
    // Another chat's finished move is no answer of O's.
    let other: String = store::with_read(&two.a, |conn| {
        conn.query_row(
            "SELECT move_id FROM chat_moves WHERE session_key = ?1 AND direction = 'out'",
            params![k],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    assert_eq!(
        fold(&two.a, &o, &other, APPENDED, &k, Utc::now())
            .unwrap_err()
            .code,
        "conflict"
    );
    // With the next slice out (O away under it), the first's answer again
    // still does nothing.
    let second = new_move_id();
    let (bundle, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &second, Utc::now()).unwrap();
    assert_eq!(bundle.turns.len(), 3, "the rest only");
    send(&two.b, &second, &bytes).unwrap();
    finish_move_out(&two.a, &second, Utc::now()).unwrap();
    assert_eq!(
        fold(&two.a, &o, &first, APPENDED, &k, Utc::now()).unwrap(),
        Folded::Stale
    );
    assert_eq!(runs(&two.a, &k).len(), 2, "still nothing folded early");
    assert_eq!(
        fold(&two.a, &o, &second, APPENDED, &k, Utc::now()).unwrap(),
        Folded::Settled
    );
    let want: Vec<String> = [format!("{k}-1"), format!("{k}-2")]
        .into_iter()
        .chain(off_runs(n))
        .collect();
    assert_eq!(runs(&two.b, &k), want);
    assert_eq!(runs(&two.a, &k), want);
    // And once more after the end: settled, nothing changes.
    assert_eq!(
        fold(&two.a, &o, &second, APPENDED, &k, Utc::now()).unwrap(),
        Folded::Settled
    );
    assert_eq!(runs(&two.a, &k), want);
}

/// Low: offline turns of mixed sizes past 16 MiB: each slice's bundle
/// stays under the cap, and together they carry every turn.
#[test]
fn slices_of_mixed_turns_stay_under_the_bundle_cap() {
    let two = two();
    let k = key(24);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    let n = 48;
    offline_turns(&two.a, &o, 3, n);
    store::with_write(&two.a, "test", |tx| {
        for i in 0..n {
            // 1 MiB, 64 KiB and 4 bytes, in turn: about 17 MiB in all.
            let size = [1024 * 1024, 64 * 1024, 4][i as usize % 3];
            let payload = serde_json::json!({ "text": "x".repeat(size) }).to_string();
            tx.execute(
                "INSERT INTO transcripts(session_key, turn_ord, payload) VALUES(?1, ?2, ?3)",
                params![o, 3 + i, payload],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .unwrap();
    let (mut carried, mut sliced) = (0, 0);
    for _ in 0..4 {
        sliced += 1;
        let id = new_move_id();
        let (bundle, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
        assert!(bytes.len() <= chat_move::BUNDLE_MAX, "{}", bytes.len());
        carried += bundle.turns.len();
        send(&two.b, &id, &bytes).unwrap();
        finish_move_out(&two.a, &id, Utc::now()).unwrap();
        if fold(&two.a, &o, &id, APPENDED, &k, Utc::now()).unwrap() != Folded::Sliced {
            break;
        }
    }
    assert_eq!(carried as i64, n);
    assert!(sliced > 1, "more than one slice");
    assert_eq!(runs(&two.b, &k).len() as i64, 2 + n);
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, APPENDED);
}

/// A slice holds its turns in order up to the byte budget; the first turn
/// always (one too large alone is refused whole by the encoder).
#[test]
fn a_slice_stops_at_its_byte_budget() {
    let big = |n: i64| MovedTurn {
        ord: n,
        run_id: Some(format!("off-{n}")),
        started_at: "2026-11-15T10:00:00.000Z".into(),
        ended_at: None,
        status: Some("success".into()),
        asked: None,
        permission_mode: None,
        effort: None,
        asked_by: None,
        transcript: Some(serde_json::Value::String(
            "x".repeat(SLICE_BYTES / 2 - 1024),
        )),
    };
    let mut slice = Slice::default();
    assert!(slice.takes(&big(1)));
    assert!(slice.takes(&big(2)));
    assert!(!slice.takes(&big(3)));
    let mut alone = Slice::default();
    let mut huge = big(1);
    huge.transcript = Some(serde_json::Value::String("x".repeat(SLICE_BYTES + 1)));
    assert!(alone.takes(&huge), "the first turn is always taken");
}

/// One offline turn larger than a sync carries can't sync in any slice:
/// the chat is kept here whole, saying why where the person can read it.
#[tokio::test]
async fn an_offline_turn_past_the_cap_is_kept_here_with_a_reason() {
    let two = two();
    let k = key(22);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    offline_turns(&two.a, &o, 3, 1);
    let payload = serde_json::json!({ "text": "x".repeat(chat_move::BUNDLE_MAX) }).to_string();
    store::with_write(&two.a, "test", |tx| {
        tx.execute(
            "INSERT INTO transcripts(session_key, turn_ord, payload) VALUES(?1, 3, ?2)",
            params![o, payload],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
    .unwrap();
    let synced = sync_with(&two.a, &Unreached, MACBOOK).await;
    let [(key, Synced::Kept(why))] = synced.as_slice() else {
        panic!("{synced:?}")
    };
    assert_eq!(key, &o);
    assert!(why.contains("16 MiB"), "{why}");
    let kept = offline_chats(&two.a).unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].state, KEPT);
    assert_eq!(kept[0].note.as_deref(), Some(why.as_str()));
    assert_eq!(admission_refusal(&two.a, &o), None, "it runs here");
    assert!(sync_with(&two.a, &Unreached, MACBOOK).await.is_empty());
}

/// Round 2 (3): this Mac's copy of the chat went while the offline chat was
/// open: the fold writes no turn under a key with no chat, and O goes (its
/// turns are in that chat there).
#[test]
fn a_fold_with_no_copy_of_the_chat_here_writes_no_orphan_turns() {
    let two = two();
    let k = key(18);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let (id, imported) = deliver(&two, &o);
    assert_eq!(imported.outcome, Some(APPENDED));
    store::with_write(&two.a, "test", |tx| {
        chat_move::clear_chat_rows(tx, &k)?;
        tx.execute("DELETE FROM sessions WHERE key = ?1", params![k])
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .unwrap();
    fold(&two.a, &o, &id, APPENDED, &k, Utc::now()).unwrap();
    assert!(turns_of(&two.a, &k).is_empty(), "no orphan turns");
    assert!(store::session_by_key(&two.a, &o).unwrap().is_none());
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, APPENDED);
    assert_eq!(runs(&two.b, &k).len(), 3, "the turns are there");
}

/// Round 2 (4): the chat there moving on from it (`leaving`) or living
/// elsewhere is `busy`, never a refusal for good; a chat that came back to
/// this Mac leaves its offline chat waiting, not kept, and not sent.
#[test]
fn a_chat_moving_or_back_here_makes_the_sync_wait_not_kept() {
    let two = two();
    let k = key(19);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let set_home = |vault: &Path, state: &str| {
        store::with_write(vault, "test", |tx| {
            tx.execute(
                "UPDATE chat_homes SET state = ?2 WHERE session_key = ?1",
                params![k, state],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap()
    };
    set_home(&two.b, LEAVING);
    let id = new_move_id();
    let (_, bytes) = begin_move_out(&two.a, &o, MACBOOK, MINI, &id, Utc::now()).unwrap();
    let refused = send(&two.b, &id, &bytes).unwrap_err();
    assert_eq!(refused.code, "busy");
    assert!(chat_move_send::commit_refusal(&chat_move_send::answer(
        409,
        serde_json::json!({"error": {"code": refused.code}})
    ))
    .is_none());
    assert_eq!(runs(&two.b, &k).len(), 2);
    // Back home here and released: K came back to this Mac.
    staging(&two.b).drop_move(&id);
    release_by_hand(&two.a, &id, &o);
    set_home(&two.a, ARRIVED);
    assert_eq!(offline_of(&two.a, &o).unwrap().unwrap().state, OPEN);
    assert!(ready(&two.a, MINI).unwrap().is_empty());
    assert!(pending_peers(&two.a).is_empty());
}

/// Round 2 (6): times the other Mac sends are read as times, normalised to
/// this Mac's form, and clamped to now, per turn too.
#[test]
fn sent_times_are_normalised_and_never_in_the_future() {
    let now = Utc::now();
    assert_eq!(not_after("2999-01-01T00:00:00Z", now), db_time(now));
    assert_eq!(not_after("not a time", now), db_time(now));
    assert_eq!(
        not_after("2026-11-15T12:00:00+02:00", now),
        "2026-11-15T10:00:00.000Z"
    );
    let two = two();
    let k = key(20);
    moved(&two, &k);
    let o = continue_offline(&two.a, &k, Utc::now()).unwrap();
    turn(&two.a, &o, "off-1", "One", "1");
    let (mut bundle, _) =
        begin_move_out(&two.a, &o, MACBOOK, MINI, &new_move_id(), Utc::now()).unwrap();
    bundle.turns[0].started_at = "2999-01-01T00:00:00Z".into();
    bundle.turns[0].ended_at = Some("2026-11-15T12:00:00+02:00".into());
    import_move(&two.b, &bundle, MACBOOK, Utc::now()).unwrap();
    let (started, ended): (String, String) = store::with_read(&two.b, |conn| {
        conn.query_row(
            "SELECT started_at, ended_at FROM turns WHERE run_id = 'off-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap();
    assert!(!started.starts_with("2999"), "{started}");
    assert_eq!(ended, "2026-11-15T10:00:00.000Z");
}
