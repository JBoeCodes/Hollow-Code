//! M5 PR2: which answers prove what (the contract on `chat_move::TargetSaid`),
//! and a move with no answer at all waits, owned by this Mac and running
//! nowhere. The moves over real routes are `phone_access/move_send_tests.rs`.
use super::*;
use crate::session_store as store;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

const MACBOOK: &str = "m-3f9c1a07e2b45d18";
const MINI: &str = "m-8d21c4be90f17a3e";

fn refused(status: u16, code: &str) -> MoveAnswer {
    answer(status, json!({"error": {"code": code, "message": "no"}}))
}

#[test]
fn only_a_commit_refusal_from_the_contract_proves_nothing_landed() {
    for code in COMMIT_REFUSALS {
        let status = if code == "too_large" { 413 } else { 409 };
        assert_eq!(
            commit_refusal(&refused(status, code)),
            Some(TargetSaid::CommitRefused),
            "{code}"
        );
    }
    // Missing parts (a second delivery's) answer 400: not proof.
    assert_eq!(commit_refusal(&refused(400, "bad_request")), None);
    // Parts gone, a busy staging area, another Mac's move, a server
    // failure, or a refusal code from a 5xx: no proof.
    for (status, code) in [
        (404, "not_found"),
        (409, "busy"),
        (403, "forbidden"),
        (500, "internal"),
        (503, "conflict"),
        (409, "started"),
    ] {
        assert_eq!(
            commit_refusal(&refused(status, code)),
            None,
            "{status} {code}"
        );
    }
    assert_eq!(commit_refusal(&answer(200, json!({}))), None);
}

#[test]
fn only_a_delete_answer_settles_a_move_and_says_how() {
    for (word, cancelled) in [
        ("notImported", Cancelled::NotImported),
        ("removed", Cancelled::Removed),
        ("already", Cancelled::Already),
    ] {
        assert_eq!(
            delete_said(&answer(200, json!({"cancelled": word}))),
            Some(TargetSaid::Cancelled(cancelled))
        );
    }
    assert_eq!(
        delete_said(&refused(409, "started")),
        Some(TargetSaid::Kept)
    );
    for no_proof in [
        answer(200, json!({})),
        answer(200, json!({"cancelled": "maybe"})),
        refused(409, "conflict"),
        refused(404, "not_found"),
        refused(500, "internal"),
    ] {
        assert_eq!(delete_said(&no_proof), None, "{no_proof:?}");
    }
}

/// A Mac that never answers: every call fails as no answer does.
#[derive(Default)]
struct Silent {
    calls: AtomicUsize,
}

impl MoveLink for Silent {
    fn peer(&self) -> &str {
        MINI
    }
    fn call<'a>(&'a self, _route: MoveRoute<'a>) -> Fut<'a, Result<MoveAnswer, PeerError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(PeerError::Offline("Can't reach it.".into())) })
    }
    fn send<'a>(
        &'a self,
        _key: &'a str,
        _text: &'a str,
        _message_id: &'a str,
    ) -> Fut<'a, Result<Accepted, PeerError>> {
        Box::pin(async { Err(PeerError::Offline("Can't reach it.".into())) })
    }
}

#[tokio::test]
async fn a_move_no_one_answers_waits_leaving_and_never_runs_here() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().canonicalize().unwrap();
    let cwd = crate::workspaces::ensure_quick_chats(&vault).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let key = "s-20261115-sd000001";
    let ord = store::start_turn(&vault, key, "claude", &cwd, "run-1", "Hi").unwrap();
    store::finish_turn(&vault, key, ord, "success").unwrap();
    let link = Silent::default();
    let id = chat_move::new_move_id();
    let sent = send_move(&vault, &link, key, MACBOOK, &id).await.unwrap();
    assert!(matches!(sent, Sent::Waiting(_)), "{sent:?}");
    let home = chat_move::home_of(&vault, key).unwrap().unwrap();
    assert_eq!(
        (home.state.as_str(), home.peer.as_str()),
        (chat_move::LEAVING, MINI)
    );
    assert!(chat_move::admission_refusal(&vault, key).is_some());
    // The first part and the status were tried; nothing was taken as proof.
    assert_eq!(link.calls.load(Ordering::SeqCst), 2);
    // Keep it here with that Mac still silent: the chat still runs nowhere.
    assert!(matches!(
        keep_here(&vault, Some(&link), &id).await,
        Sent::Waiting(_)
    ));
    assert_eq!(chat_move::pending_aborts(&vault).unwrap().len(), 1);
    assert!(chat_move::admission_refusal(&vault, key).is_some());
    // Contact with another Mac settles nothing of this one's.
    struct Other(Silent);
    impl MoveLink for Other {
        fn peer(&self) -> &str {
            "m-1111222233334444"
        }
        fn call<'a>(&'a self, route: MoveRoute<'a>) -> Fut<'a, Result<MoveAnswer, PeerError>> {
            self.0.call(route)
        }
        fn send<'a>(
            &'a self,
            key: &'a str,
            text: &'a str,
            id: &'a str,
        ) -> Fut<'a, Result<Accepted, PeerError>> {
            self.0.send(key, text, id)
        }
    }
    let other = Other(Silent::default());
    assert!(settle_pending(&vault, &other).await.is_empty());
    assert!(retry_leaving(&vault, &other, MACBOOK).await.is_empty());
    assert_eq!(other.0.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn the_continuation_names_the_mac_and_repeats_under_one_id() {
    let text = continuation_text("MacBook Pro");
    assert!(text.contains("moved here from MacBook Pro"), "{text}");
    let id = "mv-0123456789abcdef";
    assert_eq!(continuation_id(id), continuation_id(id));
    assert!(continuation_id(id)
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-'));
}

/// One delivery per move id at a time: a second `send_move`, Keep it here
/// or settle of a move being driven calls nothing and changes nothing, so
/// its staging can't make the first one's commit look refused.
#[tokio::test]
async fn a_move_being_driven_is_never_driven_twice_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().canonicalize().unwrap();
    let cwd = crate::workspaces::ensure_quick_chats(&vault).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    let key = "s-20261115-sd000002";
    let ord = store::start_turn(&vault, key, "claude", &cwd, "run-2", "Hi").unwrap();
    store::finish_turn(&vault, key, ord, "success").unwrap();
    let link = Silent::default();
    let id = chat_move::new_move_id();
    let held = Driving::claim(&id).unwrap();
    assert!(Driving::claim(&id).is_none(), "claimed once");
    let sent = send_move(&vault, &link, key, MACBOOK, &id).await.unwrap();
    assert_eq!(sent, Sent::Busy);
    assert_eq!(link.calls.load(Ordering::SeqCst), 0, "nothing sent");
    assert!(
        chat_move::home_of(&vault, key).unwrap().is_none(),
        "nothing written"
    );
    drop(held);
    // Driven for real now (the Mac is silent): it waits leaving.
    assert!(matches!(
        send_move(&vault, &link, key, MACBOOK, &id).await.unwrap(),
        Sent::Waiting(_)
    ));
    let calls = link.calls.load(Ordering::SeqCst);
    let held = Driving::claim(&id).unwrap();
    assert_eq!(keep_here(&vault, Some(&link), &id).await, Sent::Busy);
    assert!(settle_pending(&vault, &link).await.is_empty());
    assert!(retry_leaving(&vault, &link, MACBOOK)
        .await
        .iter()
        .all(|(_, _, s)| matches!(s, Sent::Waiting(_))));
    assert_eq!(
        link.calls.load(Ordering::SeqCst),
        calls,
        "no call while driven"
    );
    drop(held);
    assert!(Driving::claim(&id).is_some(), "released when dropped");
}

// ── Lane M: a chat longer than one sync, over a stand-in for the link ────

/// The other Mac as a link: its staging area and database are in this
/// process (`vault`), and a call can be made to fail or be refused.
struct Loop {
    vault: std::path::PathBuf,
    /// The move's own id: any other commit is an earlier slice's.
    root: String,
    /// Slice commits so far, and the one (0-based) whose answer is lost.
    slice_commits: AtomicUsize,
    lose_at: AtomicUsize,
    /// A slice commit refused for good (`conflict`).
    refuse_slice: std::sync::atomic::AtomicBool,
    /// The move ids whose first part was sent, in order.
    parts: std::sync::Mutex<Vec<String>>,
}

impl Loop {
    fn new(vault: &std::path::Path, root: &str) -> Self {
        Self {
            vault: vault.to_path_buf(),
            root: root.to_string(),
            slice_commits: AtomicUsize::new(0),
            lose_at: AtomicUsize::new(usize::MAX),
            refuse_slice: Default::default(),
            parts: Default::default(),
        }
    }
}

fn failed(error: MoveError) -> MoveAnswer {
    let status = error.status();
    MoveAnswer {
        status,
        code: Some(error.code.to_string()),
        message: error.message.clone(),
        body: json!({"error": {"code": error.code, "message": error.message}}),
    }
}

impl MoveLink for Loop {
    fn peer(&self) -> &str {
        MINI
    }
    fn call<'a>(&'a self, route: MoveRoute<'a>) -> Fut<'a, Result<MoveAnswer, PeerError>> {
        let out = match route {
            MoveRoute::Part { move_id, n, body } => {
                if n == 0 {
                    self.parts.lock().unwrap().push(move_id.to_string());
                }
                let body: chat_move::PartBody = serde_json::from_value(body.clone()).unwrap();
                match chat_move::staging(&self.vault).put(
                    move_id,
                    MACBOOK,
                    n,
                    &body,
                    std::time::Instant::now(),
                ) {
                    Ok(_) => Ok(answer(200, json!({}))),
                    Err(e) => Ok(failed(e)),
                }
            }
            MoveRoute::Commit { move_id, body } => {
                let is_slice = move_id != self.root;
                let nth = if is_slice {
                    self.slice_commits.fetch_add(1, Ordering::SeqCst)
                } else {
                    usize::MAX - 1
                };
                if is_slice && self.refuse_slice.load(Ordering::SeqCst) {
                    // Refused before it is taken: don't leave its parts
                    // holding one of this Mac's places in the vault's
                    // staging area.
                    chat_move::staging(&self.vault).drop_move(move_id);
                    Ok(failed(MoveError {
                        code: "conflict",
                        message: "Refused.".into(),
                    }))
                } else if is_slice && nth == self.lose_at.load(Ordering::SeqCst) {
                    // Taken there, its answer lost.
                    let body: chat_move::CommitBody = serde_json::from_value(body.clone()).unwrap();
                    let _ = chat_move::commit(&self.vault, move_id, MACBOOK, &body, Utc::now());
                    Err(PeerError::Offline("The answer was lost.".into()))
                } else {
                    let body: chat_move::CommitBody = serde_json::from_value(body.clone()).unwrap();
                    match chat_move::commit(&self.vault, move_id, MACBOOK, &body, Utc::now()) {
                        Ok(imported) => Ok(answer(200, serde_json::to_value(imported).unwrap())),
                        Err(e) => Ok(failed(e)),
                    }
                }
            }
            MoveRoute::Status { move_id } => {
                match chat_move::move_status(&self.vault, move_id, MACBOOK) {
                    Ok(status) => Ok(answer(200, serde_json::to_value(status).unwrap())),
                    Err(e) => Ok(failed(e)),
                }
            }
            MoveRoute::Cancel { move_id } => {
                match chat_move::cancel_move_in(&self.vault, move_id, MACBOOK, Utc::now()) {
                    Ok(cancelled) => Ok(answer(200, json!({"cancelled": cancelled}))),
                    Err(e) => Ok(failed(e)),
                }
            }
        };
        Box::pin(async move { out })
    }
    fn send<'a>(
        &'a self,
        _key: &'a str,
        _text: &'a str,
        _message_id: &'a str,
    ) -> Fut<'a, Result<Accepted, PeerError>> {
        Box::pin(async { Err(PeerError::Offline("no".into())) })
    }
}

fn two_vaults() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let (pa, pb) = (
        a.path().canonicalize().unwrap(),
        b.path().canonicalize().unwrap(),
    );
    crate::workspaces::ensure_quick_chats(&pa).unwrap();
    crate::workspaces::ensure_quick_chats(&pb).unwrap();
    (a, b, pa, pb)
}

fn seed_turns(vault: &Path, key: &str, n: usize) {
    let cwd = crate::workspaces::ensure_quick_chats(vault).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    for i in 0..n {
        let ord =
            store::start_turn(vault, key, "claude", &cwd, &format!("run-{key}-{i}"), "Hi").unwrap();
        store::finish_turn(vault, key, ord, "success").unwrap();
    }
}

fn caps_of(turns: usize) -> chat_move::Caps {
    chat_move::Caps {
        turns,
        ..chat_move::Caps::SYNC
    }
}

fn turn_count(vault: &Path, key: &str) -> usize {
    store::session_by_key(vault, key)
        .unwrap()
        .map_or(0, |s| s.turn_count as usize)
}

#[tokio::test]
async fn a_long_chat_moves_in_slices_over_the_link() {
    let (_a, _b, a, b) = two_vaults();
    let key = "s-20261117-ls000001";
    seed_turns(&a, key, 11);
    let id = chat_move::new_move_id();
    let link = Loop::new(&b, &id);
    let sent = send_move_within(&a, &link, key, MACBOOK, &id, caps_of(5))
        .await
        .unwrap();
    assert_eq!(sent, Sent::Moved);
    assert_eq!(turn_count(&b, key), 11);
    assert_eq!(
        link.parts.lock().unwrap().len(),
        3,
        "two slices and the move"
    );
    assert_eq!(link.parts.lock().unwrap().last(), Some(&id));
    assert_eq!(
        chat_move::home_of(&a, key).unwrap().unwrap().state,
        chat_move::AWAY
    );
}

#[tokio::test]
async fn a_slice_the_link_lost_is_sent_again_and_one_that_landed_never_is() {
    let (_a, _b, a, b) = two_vaults();
    let key = "s-20261117-ls000002";
    seed_turns(&a, key, 16);
    let id = chat_move::new_move_id();
    let link = Loop::new(&b, &id);
    link.lose_at.store(1, Ordering::SeqCst);
    // The first slice lands; the second reaches the other Mac but its
    // answer is lost: the move waits, the chat runs nowhere.
    let sent = send_move_within(&a, &link, key, MACBOOK, &id, caps_of(5))
        .await
        .unwrap();
    assert!(matches!(sent, Sent::Waiting(_)), "{sent:?}");
    assert!(chat_move::admission_refusal(&a, key).is_some());
    assert_eq!(turn_count(&b, key), 0, "nothing there until the last slice");
    let sent_before = link.parts.lock().unwrap().clone();
    assert_eq!(sent_before.len(), 2);

    // The next contact (a restart in between changes nothing: the progress
    // is in the database) resumes at the second slice, under its id.
    let sent = send_move_within(&a, &link, key, MACBOOK, &id, caps_of(5))
        .await
        .unwrap();
    assert_eq!(sent, Sent::Moved);
    let parts = link.parts.lock().unwrap().clone();
    // Slices 1 and 2 and the move itself; slice 0 had landed, so not again.
    assert_eq!(parts.len(), 2 + 3, "{parts:?}");
    assert_eq!(
        parts[2], sent_before[1],
        "the lost slice goes again, same id"
    );
    assert_eq!(parts[0], sent_before[0]);
    assert!(!parts[2..].contains(&parts[0]), "slice 0 was sent again");
    assert_eq!(turn_count(&b, key), 16);
}

#[tokio::test]
async fn a_slice_refused_for_good_calls_the_whole_move_off() {
    let (_a, _b, a, b) = two_vaults();
    let key = "s-20261117-ls000003";
    seed_turns(&a, key, 11);
    let id = chat_move::new_move_id();
    let link = Loop::new(&b, &id);
    link.refuse_slice.store(true, Ordering::SeqCst);
    let sent = send_move_within(&a, &link, key, MACBOOK, &id, caps_of(5))
        .await
        .unwrap();
    assert!(matches!(sent, Sent::Home(_)), "{sent:?}");
    // The chat runs here again, whole; nothing waits on either Mac.
    assert!(chat_move::admission_refusal(&a, key).is_none());
    assert_eq!(turn_count(&a, key), 11);
    assert_eq!(turn_count(&b, key), 0);
    let waiting = |v: &Path, t: &str| -> i64 {
        store::with_read(v, |c| {
            c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get(0))
                .map_err(|e| e.to_string())
        })
        .unwrap()
    };
    assert_eq!(waiting(&a, "chat_slices_out"), 0);
    assert_eq!(waiting(&b, "chat_slices_in"), 0);
}

#[tokio::test]
async fn a_chat_that_fits_one_sync_sends_no_slice() {
    let (_a, _b, a, b) = two_vaults();
    let key = "s-20261117-ls000004";
    seed_turns(&a, key, 5);
    let id = chat_move::new_move_id();
    let link = Loop::new(&b, &id);
    let sent = send_move_within(&a, &link, key, MACBOOK, &id, caps_of(5))
        .await
        .unwrap();
    assert_eq!(sent, Sent::Moved);
    assert_eq!(*link.parts.lock().unwrap(), vec![id]);
    assert_eq!(turn_count(&b, key), 5);
}
