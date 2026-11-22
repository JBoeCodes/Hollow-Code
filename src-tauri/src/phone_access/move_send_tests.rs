//! M5 PR2: a chat moves from this "MacBook" (a second vault, so a second
//! Sessions DB) to the rig, which stands in for the runner with its real
//! routes, over a paired peer client on loopback TCP: the move driver
//! (`chat_move_send`), the trigger (`chat_move_trigger`) with a stand-in
//! for this Mac's running turns and the sleep notice, and a stand-in link
//! that loses an answer or the whole Mac. No real ssh, Keychain or sleep.
use super::peer_tests::{me as this_mac, MACBOOK};
use super::*;
use crate::chat_move;
use crate::chat_move_send::{self, Fut, MoveLink, PeerLink, Sent};
use crate::chat_move_trigger::{
    self as trigger, Me, MoveOutcome, Power, SleepMover, StopPlan, Turns as _,
};
use crate::peer_client::{self, Accepted, Dial, MoveAnswer, MoveRoute, PeerClient, PeerError};
use crate::peer_machines::Machines;

/// This "MacBook": its own vault and Quick chats folder.
struct Source {
    _dir: tempfile::TempDir,
    vault: PathBuf,
    cwd: String,
}

fn source() -> Source {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().canonicalize().unwrap();
    let cwd = crate::workspaces::ensure_quick_chats(&vault).unwrap();
    Source {
        cwd: cwd.to_string_lossy().into_owned(),
        _dir: dir,
        vault,
    }
}

fn me() -> Me {
    Me {
        machine_id: MACBOOK.into(),
        name: "MacBook Pro".into(),
    }
}

/// Pair with the rig and make its link; the rig has the Quick chats folder
/// too (the vault syncs).
async fn link_to(rig: &Rig) -> (PeerLink, tempfile::TempDir) {
    link_as(rig, this_mac()).await
}

/// [`link_to`] as another Mac (its own `machineId`, so its moves take
/// their own places in the rig's staging area).
async fn link_as(rig: &Rig, mac: peer_client::ThisMac) -> (PeerLink, tempfile::TempDir) {
    crate::workspaces::ensure_quick_chats(&rig.vault).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let machines = Arc::new(Machines::at(dir.path()));
    let code = rig.open_machine_window();
    let bare = PeerClient::new(Dial::Tcp(rig.port), rig.port, "Test Mac");
    let (_, client) = peer_client::pair_with_code(
        &machines,
        bare,
        &code,
        &mac,
        vec!["runner".into()],
        rig.port,
    )
    .await
    .unwrap();
    (PeerLink::new(Arc::new(client)).unwrap(), dir)
}

/// A finished chat with two exchanges and a native id, in the source.
fn idle_chat(src: &Source, key: &str) {
    seed_chat(
        &src.vault,
        key,
        "claude",
        &src.cwd,
        &[("Plan the trip", "Here is a plan."), ("Book it", "Booked.")],
        "plan",
    );
    session_store::record_harness_id(&src.vault, key, "0b1c2d3e-4f50-4617-8899-aabbccddeeff")
        .unwrap();
}

/// The chat's run queue rows on `vault`.
fn queued(vault: &Path, key: &str) -> i64 {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM run_queue WHERE source = 'chat' AND source_id = ?1",
            [key],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap()
}

/// Which of the two Macs own chat `key` (has it, and it isn't `away`).
fn owners(src: &Source, rig: &Rig, key: &str) -> usize {
    [&src.vault, &rig.vault]
        .into_iter()
        .filter(|vault| {
            let has = session_store::session_by_key(vault, key).unwrap().is_some();
            let home = chat_move::home_of(vault, key).unwrap();
            has && home.is_none_or(|h| h.state != chat_move::AWAY)
        })
        .count()
}

/// This Mac's running turns: a safe-stop ask ends the turn (cancelled,
/// its reply kept), as P1a's pause at a safe point does.
struct Running {
    vault: PathBuf,
    turns: Mutex<HashMap<String, i64>>,
    asked: AtomicUsize,
    stopped: AtomicUsize,
}

impl Running {
    fn new(vault: &Path) -> Self {
        Self {
            vault: vault.to_path_buf(),
            turns: Mutex::new(HashMap::new()),
            asked: AtomicUsize::new(0),
            stopped: AtomicUsize::new(0),
        }
    }

    fn start(&self, key: &str, cwd: &str, asked: &str) {
        let run = format!("run-live-{key}");
        let ord = session_store::start_turn(&self.vault, key, "claude", cwd, &run, asked).unwrap();
        session_store::append_event(&self.vault, key, ord, "assistant", "Halfway there").unwrap();
        self.turns.lock().unwrap().insert(key.to_string(), ord);
    }

    fn end(&self, key: &str) -> bool {
        let Some(ord) = self.turns.lock().unwrap().remove(key) else {
            return false;
        };
        session_store::finish_turn(&self.vault, key, ord, "cancelled").unwrap();
        true
    }
}

impl trigger::Turns for Running {
    fn running(&self) -> Vec<String> {
        self.turns.lock().unwrap().keys().cloned().collect()
    }
    fn ask_safe_stop(&self, key: &str) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.end(key)
    }
    fn stop_now(&self, key: &str) -> bool {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        self.end(key)
    }
    fn ended(&self, vault: &Path, key: &str) -> bool {
        !chat_move::turn_in_flight_now(vault, key)
    }
}

/// What the stand-in link loses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lose {
    Nothing,
    /// The commit lands there; its answer never comes back.
    CommitAnswer,
    /// The commit never reaches it.
    CommitRequest,
    /// The commit lands, then the Mac is gone (every later call fails).
    CommitAnswerThenAll,
    /// The Mac is out of reach.
    All,
    /// The commit never answers, nor reaches it (cut off by the caller).
    HangCommit,
    /// The commit is answered with this refusal (409) and never reaches it.
    RefuseCommit(&'static str),
    /// A Hollow from before PR3: a version-2 commit is answered 400.
    OldReceiver,
    /// `GET` of a move gets no answer; everything else goes through.
    StatusLost,
    /// The first commit goes through; a later one lands, then the Mac is
    /// gone (a slice out and unanswered).
    LaterCommitAnswerThenAll,
}

/// A real link with answers lost on purpose; it keeps what it sent.
struct Lossy {
    inner: PeerLink,
    lose: Mutex<Lose>,
    parts: Mutex<Vec<Value>>,
    commit: Mutex<Option<Value>>,
    /// Told when a hung commit is reached.
    hung: tokio::sync::Notify,
    /// Commits seen.
    commits: AtomicUsize,
}

impl Lossy {
    fn new(inner: PeerLink, lose: Lose) -> Self {
        Self {
            inner,
            lose: Mutex::new(lose),
            parts: Mutex::new(Vec::new()),
            commit: Mutex::new(None),
            hung: tokio::sync::Notify::new(),
            commits: AtomicUsize::new(0),
        }
    }

    fn set(&self, lose: Lose) {
        *self.lose.lock().unwrap() = lose;
    }

    fn gone() -> PeerError {
        PeerError::Offline("Can't reach Test Mac: the link failed.".into())
    }
}

impl MoveLink for Lossy {
    fn peer(&self) -> &str {
        self.inner.peer()
    }
    fn call<'a>(&'a self, route: MoveRoute<'a>) -> Fut<'a, Result<MoveAnswer, PeerError>> {
        Box::pin(async move {
            let lose = *self.lose.lock().unwrap();
            if lose == Lose::All {
                return Err(Self::gone());
            }
            match route {
                MoveRoute::Part { body, .. } => self.parts.lock().unwrap().push(body.clone()),
                MoveRoute::Status { .. } if lose == Lose::StatusLost => return Err(Self::gone()),
                MoveRoute::Commit { body, .. } => {
                    *self.commit.lock().unwrap() = Some(body.clone());
                    let first = self.commits.fetch_add(1, Ordering::SeqCst) == 0;
                    let lose = match lose {
                        Lose::LaterCommitAnswerThenAll if first => Lose::Nothing,
                        Lose::LaterCommitAnswerThenAll => Lose::CommitAnswerThenAll,
                        other => other,
                    };
                    match lose {
                        Lose::RefuseCommit(code) => {
                            // A real refusal takes the staged parts with
                            // it: drop them there too (from the rig's
                            // staging area, which holds 2 moves per Mac).
                            let id = route.move_id().to_string();
                            self.inner.call(MoveRoute::Cancel { move_id: &id }).await?;
                            return Ok(chat_move_send::answer(
                                409,
                                json!({"error": {"code": code, "message": "no"}}),
                            ));
                        }
                        Lose::OldReceiver => {
                            return Ok(chat_move_send::answer(
                                400,
                                json!({"error": {"code": "bad_request", "message": "That move comes from another Hollow version; update both Macs."}}),
                            ))
                        }
                        Lose::CommitRequest => return Err(Self::gone()),
                        Lose::HangCommit => {
                            self.hung.notify_one();
                            std::future::pending::<()>().await
                        }
                        Lose::CommitAnswer | Lose::CommitAnswerThenAll => {
                            let landed = self.inner.call(route).await?;
                            assert_eq!(landed.status, 200, "{}", landed.message);
                            if lose == Lose::CommitAnswerThenAll {
                                self.set(Lose::All);
                            }
                            return Err(Self::gone());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            self.inner.call(route).await
        })
    }
    fn send<'a>(
        &'a self,
        key: &'a str,
        text: &'a str,
        id: &'a str,
    ) -> Fut<'a, Result<Accepted, PeerError>> {
        Box::pin(async move {
            if *self.lose.lock().unwrap() == Lose::All {
                return Err(Self::gone());
            }
            self.inner.send(key, text, id).await
        })
    }
}

fn plan() -> StopPlan {
    StopPlan::sending_by(tokio::time::Instant::now() + trigger::SLEEP_BUDGET)
}

/// The lid-close acceptance test (the M5 plan, PR2): a sleep notice with a
/// chat running moves it once, its turn stopped and its history imported
/// there, this Mac refusing its turns from then on, and one continuation
/// queued there, which runs there: the rig admits it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sleep_moves_a_running_chat_once_and_it_keeps_going_there() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let link = Arc::new(link);
    let key = "s-20261115-mv000001";
    idle_chat(&src, key);
    let turns = Arc::new(Running::new(&src.vault));
    turns.start(key, &src.cwd, "Now pack the bags");

    struct Mover {
        vault: PathBuf,
        turns: Arc<Running>,
        link: Arc<PeerLink>,
        calls: AtomicUsize,
    }
    impl SleepMover for Mover {
        fn enabled(&self) -> bool {
            true
        }
        fn move_running(&self, budget: Duration) -> Fut<'_, usize> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                let by = tokio::time::Instant::now() + budget;
                let results = trigger::move_running(
                    &self.vault,
                    self.turns.as_ref(),
                    self.link.as_ref(),
                    &me(),
                    "Test Mac",
                    by,
                )
                .await;
                assert!(results.iter().all(|r| r.continued), "{results:?}");
                results
                    .iter()
                    .filter(|r| r.outcome == MoveOutcome::Moved)
                    .count()
            })
        }
        fn woke(&self) {}
    }
    let mover = Arc::new(Mover {
        vault: src.vault.clone(),
        turns: turns.clone(),
        link: link.clone(),
        calls: AtomicUsize::new(0),
    });
    let notice = trigger::Trigger::new(mover.clone(), tokio::runtime::Handle::current());
    let ran = tokio::task::spawn_blocking(move || notice.on_power(Power::WillSleep))
        .await
        .unwrap();
    assert!(ran);
    assert_eq!(mover.calls.load(Ordering::SeqCst), 1);
    assert_eq!(turns.asked.load(Ordering::SeqCst), 1, "a safe point asked");
    assert_eq!(turns.stopped.load(Ordering::SeqCst), 0, "it came: no Stop");

    // One owner: the rig. This Mac refuses the chat's turns.
    assert_eq!(owners(&src, &rig, key), 1);
    let away = chat_move::home_of(&src.vault, key).unwrap().unwrap();
    assert_eq!(
        (away.state.as_str(), away.peer.as_str()),
        (chat_move::AWAY, rig.machine_id().as_str())
    );
    assert!(chat_move::admission_refusal(&src.vault, key).is_some());
    // The history landed once: three turns, the stopped one with its reply.
    let arrived = chat_move::home_of(&rig.vault, key).unwrap().unwrap();
    assert_eq!(
        (arrived.state.as_str(), arrived.peer.as_str()),
        (chat_move::ARRIVED, MACBOOK)
    );
    let detail = rig
        .call(
            "GET",
            &format!("/v1/sessions/{key}"),
            Some(&rig.token()),
            None,
        )
        .await
        .json();
    let turns_there = detail["turns"].as_array().unwrap();
    assert!(turns_there.len() >= 3, "{detail}");
    // The continuation: queued once there, as the person's own send, and
    // the run queue there admits it (no home record refuses it).
    let sent = rig.host.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "one continuation");
    assert_eq!(sent[0].session_key.as_deref(), Some(key));
    // The route wraps the person's words in its usual intent block.
    assert!(
        sent[0]
            .prompt
            .ends_with(&chat_move_send::continuation_text("MacBook Pro")),
        "{}",
        sent[0].prompt
    );
    assert_eq!(queued(&rig.vault, key), 1);
    assert!(chat_move::admission_refusal(&rig.vault, key).is_none());

    // A retried continuation runs nothing twice; a second move of the same
    // chat is refused here (it lives there now).
    let retried =
        chat_move_send::continue_there(link.as_ref(), key, &away.move_id, "MacBook Pro").await;
    assert_eq!(queued(&rig.vault, key), 1, "{retried:?}");
    let refused = chat_move_send::send_move(
        &src.vault,
        link.as_ref(),
        key,
        MACBOOK,
        &chat_move::new_move_id(),
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code, "moved");
    // A second sleep with nothing running moves nothing.
    assert!(turns.running().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_commit_answer_finishes_and_a_lost_commit_is_called_off_with_proof() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;

    // The commit landed; its answer was lost. The status says imported:
    // the move finishes, one owner, the rig.
    let landed = "s-20261115-mv000002";
    idle_chat(&src, landed);
    let lossy = Lossy::new(link, Lose::CommitAnswer);
    let sent = chat_move_send::send_move(
        &src.vault,
        &lossy,
        landed,
        MACBOOK,
        &chat_move::new_move_id(),
    )
    .await
    .unwrap();
    assert_eq!(sent, Sent::Moved);
    assert_eq!(owners(&src, &rig, landed), 1);
    assert!(session_store::session_by_key(&rig.vault, landed)
        .unwrap()
        .is_some());

    // The commit never arrived. The status says receiving, so the move's
    // DELETE goes first and its answer is the proof: the chat is home, and
    // a late commit of the same move is refused there.
    let lost = "s-20261115-mv000003";
    idle_chat(&src, lost);
    lossy.set(Lose::CommitRequest);
    lossy.parts.lock().unwrap().clear();
    let id = chat_move::new_move_id();
    let sent = chat_move_send::send_move(&src.vault, &lossy, lost, MACBOOK, &id)
        .await
        .unwrap();
    assert!(matches!(sent, Sent::Home(_)), "{sent:?}");
    assert!(chat_move::home_of(&src.vault, lost).unwrap().is_none());
    assert!(chat_move::admission_refusal(&src.vault, lost).is_none());
    assert!(session_store::session_by_key(&rig.vault, lost)
        .unwrap()
        .is_none());
    assert_eq!(owners(&src, &rig, lost), 1);
    lossy.set(Lose::Nothing);
    let parts = lossy.parts.lock().unwrap().clone();
    for (n, body) in parts.iter().enumerate() {
        let route = MoveRoute::Part {
            move_id: &id,
            n,
            body,
        };
        lossy.inner.call(route).await.unwrap();
    }
    let commit = lossy.commit.lock().unwrap().clone().unwrap();
    let late = lossy
        .inner
        .call(MoveRoute::Commit {
            move_id: &id,
            body: &commit,
        })
        .await
        .unwrap();
    assert_eq!(
        (late.status, late.code.as_deref()),
        (409, Some("cancelled"))
    );
    assert!(session_store::session_by_key(&rig.vault, lost)
        .unwrap()
        .is_none());
}

/// With the other Mac out of reach a move waits (owned here, running
/// nowhere); Keep it here marks it aborting; the next contact sends its
/// DELETE and settles it on the answer: home when nothing ran there,
/// there when a turn already ran.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pending_abort_sends_its_delete_on_the_next_contact() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let lossy = Lossy::new(link, Lose::All);

    // Nothing reached it: Keep it here, then contact: home again.
    let first = "s-20261115-mv000004";
    idle_chat(&src, first);
    let id = chat_move::new_move_id();
    let sent = chat_move_send::send_move(&src.vault, &lossy, first, MACBOOK, &id)
        .await
        .unwrap();
    assert!(matches!(sent, Sent::Waiting(_)), "{sent:?}");
    assert!(chat_move::admission_refusal(&src.vault, first).is_some());
    let kept = chat_move_send::keep_here(&src.vault, Some(&lossy), &id).await;
    assert!(matches!(kept, Sent::Waiting(_)), "{kept:?}");
    assert!(
        chat_move::admission_refusal(&src.vault, first).is_some(),
        "still runs nowhere"
    );
    lossy.set(Lose::Nothing);
    let settled = chat_move_send::settle_pending(&src.vault, &lossy).await;
    assert_eq!(settled.len(), 1);
    assert!(matches!(settled[0].2, Sent::Home(_)), "{settled:?}");
    assert!(chat_move::home_of(&src.vault, first).unwrap().is_none());
    assert!(chat_move::pending_aborts(&src.vault).unwrap().is_empty());
    let status = lossy
        .inner
        .call(MoveRoute::Status { move_id: &id })
        .await
        .unwrap();
    assert_eq!(status.body["state"], "cancelled", "the tombstone holds");

    // The commit landed, then the Mac went; a turn ran there meanwhile.
    // Keep it here waits; the next contact's DELETE answers `started`, so
    // the move stands: the chat is the rig's.
    let second = "s-20261115-mv000005";
    idle_chat(&src, second);
    lossy.set(Lose::CommitAnswerThenAll);
    let id = chat_move::new_move_id();
    let sent = chat_move_send::send_move(&src.vault, &lossy, second, MACBOOK, &id)
        .await
        .unwrap();
    assert!(matches!(sent, Sent::Waiting(_)), "{sent:?}");
    let kept = chat_move_send::keep_here(&src.vault, Some(&lossy), &id).await;
    assert!(matches!(kept, Sent::Waiting(_)), "{kept:?}");
    let cwd = session_store::session_by_key(&rig.vault, second)
        .unwrap()
        .unwrap()
        .cwd;
    let ord =
        session_store::start_turn(&rig.vault, second, "claude", &cwd, "run-x", "More").unwrap();
    session_store::finish_turn(&rig.vault, second, ord, "success").unwrap();
    // While both wait, exactly one Mac owns it: the rig (this Mac's copy is
    // leaving and runs nothing).
    assert!(chat_move::admission_refusal(&src.vault, second).is_some());
    lossy.set(Lose::Nothing);
    let settled = chat_move_send::settle_pending(&src.vault, &lossy).await;
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].2, Sent::Moved);
    let home = chat_move::home_of(&src.vault, second).unwrap().unwrap();
    assert_eq!(home.state, chat_move::AWAY);
    assert_eq!(owners(&src, &rig, second), 1);
    // Settled moves aren't sent again.
    assert!(chat_move_send::settle_pending(&src.vault, &lossy)
        .await
        .is_empty());
}

/// A running chat whose move got no answer waits; the next contact sends
/// it again under the same move id, it lands, and its continuation (owed
/// since the sleep) is sent then, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_with_no_answer_lands_on_the_next_contact_and_continues_once() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let lossy = Lossy::new(link, Lose::All);
    let key = "s-20261115-mv000006";
    idle_chat(&src, key);
    let turns = Running::new(&src.vault);
    turns.start(key, &src.cwd, "Keep going");
    let result =
        trigger::move_chat(&src.vault, &turns, &lossy, &me(), "Test Mac", key, plan()).await;
    assert_eq!(result.outcome, MoveOutcome::Waiting, "{result:?}");
    assert!(!result.continued);
    assert_eq!(owners(&src, &rig, key), 1);
    assert!(chat_move::admission_refusal(&src.vault, key).is_some());

    lossy.set(Lose::Nothing);
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &me()).await, 1);
    assert_eq!(
        chat_move::home_of(&src.vault, key).unwrap().unwrap().state,
        chat_move::AWAY
    );
    assert_eq!(owners(&src, &rig, key), 1);
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1, "continued once");
    assert_eq!(queued(&rig.vault, key), 1);
    // Nothing open: another contact does nothing.
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &me()).await, 0);
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
}

/// A chat that can't move (here: its folder is outside the vault, so the
/// other Mac has no such folder) is refused before its turn is touched: it
/// keeps running here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chat_that_cant_move_keeps_its_turn_running() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let outside = tempfile::tempdir().unwrap();
    let key = "s-20261115-mv000007";
    let turns = Running::new(&src.vault);
    turns.start(key, &outside.path().to_string_lossy(), "Check in");
    let result =
        trigger::move_chat(&src.vault, &turns, &link, &me(), "Test Mac", key, plan()).await;
    assert_eq!(result.outcome, MoveOutcome::Refused, "{result:?}");
    assert_eq!(turns.asked.load(Ordering::SeqCst), 0, "never asked to stop");
    assert!(chat_move::turn_in_flight_now(&src.vault, key));
    assert!(chat_move::home_of(&src.vault, key).unwrap().is_none());
}

/// A send cut off mid-move (the sleep's budget ran out, Hollow quit) still
/// owes the chat its continuation: the next contact lands the move and
/// sends it, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_cut_off_still_owes_its_continuation() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let lossy = Lossy::new(link, Lose::HangCommit);
    let key = "s-20261115-mv000008";
    idle_chat(&src, key);
    let turns = Running::new(&src.vault);
    turns.start(key, &src.cwd, "Keep going");
    // Cut off (dropped) once the commit is on its way and unanswered.
    let mac = me();
    tokio::select! {
        result = trigger::move_chat(&src.vault, &turns, &lossy, &mac, "Test Mac", key, plan()) => {
            panic!("the commit hangs: {result:?}")
        }
        _ = lossy.hung.notified() => {}
    }
    assert!(trigger::owes(key), "owed before anything was sent");
    assert_eq!(
        chat_move::home_of(&src.vault, key).unwrap().unwrap().state,
        chat_move::LEAVING
    );
    lossy.set(Lose::Nothing);
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &me()).await, 1);
    assert_eq!(owners(&src, &rig, key), 1);
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1, "continued once");
    assert!(!trigger::owes(key));
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &me()).await, 0);
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
}

/// Lane M: the continuation a running chat's move owes is kept in a file.
/// The app quits with the move sent and unanswered; after the restart the
/// next contact lands the move and sends the continuation, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_owed_continuation_survives_a_restart() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let lossy = Lossy::new(link, Lose::CommitAnswerThenAll);
    let key = "s-20261118-mvowe001";
    idle_chat(&src, key);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Peers").join(crate::chat_move_owed::FILE);
    trigger::keep_owed_at(file.clone());
    let turns = Running::new(&src.vault);
    turns.start(key, &src.cwd, "Keep going");
    let mac = me();
    let result =
        trigger::move_chat(&src.vault, &turns, &lossy, &mac, "Test Mac", key, plan()).await;
    assert_eq!(result.outcome, MoveOutcome::Waiting, "{result:?}");
    assert!(trigger::owes(key));
    let kept = crate::chat_move_owed::Book::load(&file);
    assert!(kept.owed_moves().iter().any(|(_, k)| k == key), "written");

    // The app quits and starts again: memory is gone, the file isn't.
    trigger::forget_owed_in_memory(key);
    assert!(!trigger::owes(key));
    trigger::keep_owed_at(file.clone());
    assert!(trigger::owes(key), "read back at boot");

    lossy.set(Lose::Nothing);
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &mac).await, 1);
    assert_eq!(owners(&src, &rig, key), 1);
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1, "continued once");
    assert!(!trigger::owes(key));
    let kept = crate::chat_move_owed::Book::load(&file);
    assert!(
        !kept.owed_moves().iter().any(|(_, k)| k == key),
        "delivered"
    );
    // Another restart and contact send nothing more.
    trigger::keep_owed_at(file.clone());
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &mac).await, 0);
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);

    // A move that landed while its continuation could not be sent leaves it
    // owed; after a restart the next contact replays it, once. (One test,
    // not two: the book's file is the process's, so two would race.)
    let (link, _peers) = link_to(&rig).await;
    let lossy = Lossy::new(link, Lose::CommitAnswerThenAll);
    let key = "s-20261118-mvowe002";
    let before = rig.host.sent.lock().unwrap().len();
    idle_chat(&src, key);
    let turns = Running::new(&src.vault);
    turns.start(key, &src.cwd, "Keep going");
    let mac = me();
    let result =
        trigger::move_chat(&src.vault, &turns, &lossy, &mac, "Test Mac", key, plan()).await;
    assert_eq!(result.outcome, MoveOutcome::Waiting, "{result:?}");
    // The move lands (finished here) but nothing sent the continuation.
    lossy.set(Lose::Nothing);
    let move_id = chat_move::home_of(&src.vault, key)
        .unwrap()
        .unwrap()
        .move_id;
    let sent = chat_move_send::send_move(&src.vault, &lossy, key, &mac.machine_id, &move_id)
        .await
        .unwrap();
    assert_eq!(sent, chat_move_send::Sent::Moved);
    assert!(trigger::owes(key), "owed: the move was finished apart");
    assert_eq!(rig.host.sent.lock().unwrap().len(), before);

    // Restart, then contact: the owed continuation is sent, once.
    trigger::forget_owed_in_memory(key);
    trigger::keep_owed_at(file.clone());
    assert!(trigger::owes(key));
    trigger::retry_with(&src.vault, &lossy, &mac).await;
    assert_eq!(
        rig.host.sent.lock().unwrap().len(),
        before + 1,
        "replayed once"
    );
    assert!(!trigger::owes(key));
    trigger::retry_with(&src.vault, &lossy, &mac).await;
    assert_eq!(rig.host.sent.lock().unwrap().len(), before + 1);
    // `replay_owed` hands a landed move to `after_settled` as `Moved`: doing
    // that again for a move whose continuation went out (the pause clear and
    // the slices' landing both repeat) sends and moves nothing more.
    for _ in 0..2 {
        trigger::after_settled(
            &src.vault,
            Some(&lossy as &dyn chat_move_send::MoveLink),
            Some(&mac),
            &move_id,
            key,
            &chat_move_send::Sent::Moved,
        )
        .await;
    }
    assert_eq!(rig.host.sent.lock().unwrap().len(), before + 1);
    assert_eq!(owners(&src, &rig, key), 1);
    assert!(!trigger::owes(key));
    trigger::stop_keeping_owed();
}

/// Keep it here settles the continuation a running chat's move owed: once
/// the move is called off (now, or by the next contact's `DELETE`), nothing
/// is owed any more and nothing is sent there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_it_here_drops_the_continuation_owed() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let lossy = Lossy::new(link, Lose::All);
    let turns = Running::new(&src.vault);
    let mac = me();

    // Called off while that Mac is silent; the next contact settles it.
    let later = "s-20261115-mv000009";
    idle_chat(&src, later);
    turns.start(later, &src.cwd, "Keep going");
    let result =
        trigger::move_chat(&src.vault, &turns, &lossy, &mac, "Test Mac", later, plan()).await;
    assert_eq!(result.outcome, MoveOutcome::Waiting, "{result:?}");
    assert!(trigger::owes(later));
    let kept = trigger::keep_chat_here(&src.vault, Some(&lossy), Some(&mac), later)
        .await
        .unwrap();
    assert_eq!(kept.outcome, MoveOutcome::Waiting);
    assert!(trigger::owes(later), "still owed while it waits");

    // Called off with that Mac answering: settled at once.
    let now = "s-20261115-mv000010";
    idle_chat(&src, now);
    turns.start(now, &src.cwd, "Keep going");
    let result =
        trigger::move_chat(&src.vault, &turns, &lossy, &mac, "Test Mac", now, plan()).await;
    assert_eq!(result.outcome, MoveOutcome::Waiting, "{result:?}");
    lossy.set(Lose::Nothing);
    let kept = trigger::keep_chat_here(&src.vault, Some(&lossy), Some(&mac), now)
        .await
        .unwrap();
    assert_eq!(kept.outcome, MoveOutcome::Kept, "{kept:?}");
    assert!(!trigger::owes(now));
    assert!(chat_move::home_of(&src.vault, now).unwrap().is_none());

    // The next contact sends the first one's DELETE (settle_pending).
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &mac).await, 1);
    assert!(!trigger::owes(later));
    assert!(chat_move::home_of(&src.vault, later).unwrap().is_none());
    assert_eq!(
        rig.host.sent.lock().unwrap().len(),
        0,
        "no continuation sent"
    );
    assert_eq!(owners(&src, &rig, later), 1);
}

/// A move whose retry is refused before it is sent (its folder went) is
/// not left waiting forever: it is settled with that Mac's answer to its
/// `DELETE`, and the chat runs here again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_retry_refused_before_sending_comes_home_with_proof() {
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let lossy = Lossy::new(link, Lose::All);
    let folder = src.vault.join("Trips");
    std::fs::create_dir_all(&folder).unwrap();
    let key = "s-20261115-mv000011";
    seed_chat(
        &src.vault,
        key,
        "claude",
        &folder.to_string_lossy(),
        &[("Plan it", "Planned.")],
        "plan",
    );
    let id = chat_move::new_move_id();
    let sent = chat_move_send::send_move(&src.vault, &lossy, key, MACBOOK, &id)
        .await
        .unwrap();
    assert!(matches!(sent, Sent::Waiting(_)), "{sent:?}");
    std::fs::remove_dir_all(&folder).unwrap();
    lossy.set(Lose::Nothing);
    let settled = chat_move_send::retry_leaving(&src.vault, &lossy, MACBOOK).await;
    assert_eq!(settled.len(), 1);
    assert!(matches!(settled[0].2, Sent::Home(_)), "{settled:?}");
    assert!(chat_move::home_of(&src.vault, key).unwrap().is_none());
    assert!(chat_move::leaving_moves(&src.vault).unwrap().is_empty());
    let status = lossy
        .inner
        .call(MoveRoute::Status { move_id: &id })
        .await
        .unwrap();
    assert_eq!(status.body["state"], "cancelled");
}

// ── M5 PR3: offline sync over the real routes ────────────────────────────

/// One finished turn of chat `key` on `vault`, run id `run`.
fn offline_turn(vault: &Path, key: &str, run: &str, asked: &str) {
    let cwd = session_store::session_by_key(vault, key)
        .unwrap()
        .unwrap()
        .cwd;
    let ord = session_store::start_turn(vault, key, "claude", &cwd, run, asked).unwrap();
    session_store::append_event(vault, key, ord, "assistant", "Done.").unwrap();
    session_store::finish_turn(vault, key, ord, "success").unwrap();
}

/// Chat `key`'s run ids on `vault`, in order.
fn run_ids(vault: &Path, key: &str) -> Vec<String> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT run_id FROM turns WHERE session_key = ?1 ORDER BY ord")
            .unwrap();
        let rows = stmt.query_map([key], |row| row.get(0)).unwrap();
        Ok(rows.map(Result::unwrap).collect())
    })
    .unwrap()
}

/// Chat `key` moved from the source to the rig over the routes, then
/// continued offline on the source with two turns: the offline chat's key.
async fn moved_then_offline(src: &Source, rig: &Rig, link: &dyn MoveLink, key: &str) -> String {
    idle_chat(src, key);
    let sent = chat_move_send::send_move(&src.vault, link, key, MACBOOK, &chat_move::new_move_id())
        .await
        .unwrap();
    assert_eq!(sent, Sent::Moved);
    assert_eq!(owners(src, rig, key), 1);
    let offline = crate::chat_offline::continue_offline(&src.vault, key, Utc::now()).unwrap();
    offline_turn(&src.vault, &offline, "off-1", "Add a museum");
    offline_turn(&src.vault, &offline, "off-2", "And lunch");
    offline
}

/// The offline acceptance test (the M5 plan, PR3): a chat that moved to the
/// rig and was continued on this Mac offline syncs on the next contact,
/// its turns added once; a second contact syncs nothing; this Mac's replica
/// matches the rig's chat.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_turns_sync_once_over_the_routes_and_a_second_contact_adds_nothing() {
    use crate::chat_offline::{self, Synced};
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let key = "s-20261115-0f000101";
    let offline = moved_then_offline(&src, &rig, &link, key).await;

    let synced = chat_offline::sync_with(&src.vault, &link, MACBOOK).await;
    assert_eq!(
        synced,
        vec![(offline.clone(), Synced::Appended(key.into()))]
    );
    let there = run_ids(&rig.vault, key);
    assert_eq!(there.len(), 4, "{there:?}");
    assert_eq!(&there[2..], ["off-1", "off-2"]);
    assert_eq!(run_ids(&src.vault, key), there, "the replica matches");
    assert!(session_store::session_by_key(&rig.vault, &offline)
        .unwrap()
        .is_none());
    assert!(session_store::session_by_key(&src.vault, &offline)
        .unwrap()
        .is_none());
    assert!(chat_move::admission_refusal(&src.vault, &offline).is_some());
    assert_eq!(owners(&src, &rig, key), 1);

    // The next contact: nothing to sync, nothing added.
    assert!(chat_offline::sync_with(&src.vault, &link, MACBOOK)
        .await
        .is_empty());
    assert_eq!(run_ids(&rig.vault, key), there);
    assert!(chat_offline::pending_peers(&src.vault).is_empty());
}

/// The rig ran a turn of the chat meanwhile: the offline turns go to a new
/// chat there (after the common turns), the chat itself untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_sync_after_the_rig_moved_on_forks_with_both_histories() {
    use crate::chat_offline::{self, Synced};
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let key = "s-20261115-0f000102";
    let offline = moved_then_offline(&src, &rig, &link, key).await;
    offline_turn(&rig.vault, key, "rig-1", "Continue");

    let synced = chat_offline::sync_with(&src.vault, &link, MACBOOK).await;
    assert_eq!(synced, vec![(offline.clone(), Synced::Forked)]);
    let kept = run_ids(&rig.vault, key);
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[2], "rig-1");
    let fork = run_ids(&rig.vault, &offline);
    assert_eq!(&fork[..2], &kept[..2], "the common turns");
    assert_eq!(&fork[2..], ["off-1", "off-2"]);
    assert_eq!(run_ids(&src.vault, &offline), fork);
    assert_eq!(owners(&src, &rig, key), 1);
    assert_eq!(owners(&src, &rig, &offline), 1);
    assert!(chat_offline::sync_with(&src.vault, &link, MACBOOK)
        .await
        .is_empty());
}

/// The rig is out of reach when the sync starts (the chat waits, running
/// nowhere), then the commit's answer is lost: the next contact sends the
/// same sync again, finds it landed, folds it in, once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cut_off_offline_sync_finishes_on_the_next_contact_once() {
    use crate::chat_offline::{self, Synced};
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let key = "s-20261115-0f000103";
    let offline = moved_then_offline(&src, &rig, &link, key).await;
    let lossy = Lossy::new(link, Lose::All);

    let synced = chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert!(
        matches!(synced.as_slice(), [(k, Synced::Waiting(_))] if *k == offline),
        "{synced:?}"
    );
    assert_eq!(
        chat_move::home_of(&src.vault, &offline)
            .unwrap()
            .unwrap()
            .state,
        chat_move::LEAVING
    );
    assert!(chat_move::admission_refusal(&src.vault, &offline).is_some());
    assert_eq!(run_ids(&rig.vault, key).len(), 2, "nothing landed");

    // The rig answers again but the commit's answer is lost: the retry
    // (the same move id) lands it, `GET` says how, and it is folded in.
    lossy.set(Lose::CommitAnswer);
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &me()).await, 1);
    let there = run_ids(&rig.vault, key);
    assert_eq!(&there[2..], ["off-1", "off-2"]);
    assert_eq!(run_ids(&src.vault, key), there);
    assert_eq!(
        chat_offline::offline_of(&src.vault, &offline)
            .unwrap()
            .unwrap()
            .state,
        chat_offline::APPENDED
    );
    lossy.set(Lose::Nothing);
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &me()).await, 0);
    assert!(chat_offline::sync_with(&src.vault, &lossy, MACBOOK)
        .await
        .is_empty());
    assert_eq!(run_ids(&rig.vault, key), there);
}

/// Review fixes: a version-2 sync to a Hollow from before PR3 (400, no
/// proof) is asked about, called off and comes home, still to sync; a
/// `cancelled` refusal keeps nothing; a `conflict` keeps the chat here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_offline_sync_refused_comes_home_and_is_kept_only_for_good() {
    use crate::chat_offline::{self, Synced};
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let key = "s-20261115-0f000104";
    let offline = moved_then_offline(&src, &rig, &link, key).await;
    let lossy = Lossy::new(link, Lose::OldReceiver);
    let state = || {
        chat_offline::offline_of(&src.vault, &offline)
            .unwrap()
            .unwrap()
            .state
    };

    let synced = chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert!(matches!(synced[0].1, Synced::Waiting(_)), "{synced:?}");
    assert!(
        chat_move::home_of(&src.vault, &offline).unwrap().is_none(),
        "home"
    );
    assert_eq!(state(), chat_offline::OPEN, "still to sync");
    assert_eq!(run_ids(&rig.vault, key).len(), 2);

    lossy.set(Lose::RefuseCommit("cancelled"));
    chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert!(chat_move::home_of(&src.vault, &offline).unwrap().is_none());
    assert_eq!(state(), chat_offline::OPEN, "a tombstone keeps nothing");

    lossy.set(Lose::RefuseCommit("conflict"));
    let synced = chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert!(matches!(synced[0].1, Synced::Kept(_)), "{synced:?}");
    assert_eq!(state(), chat_offline::KEPT);
    lossy.set(Lose::Nothing);
    assert!(chat_offline::sync_with(&src.vault, &lossy, MACBOOK)
        .await
        .is_empty());
    assert_eq!(run_ids(&rig.vault, key).len(), 2);
    assert_eq!(chat_move::admission_refusal(&src.vault, &offline), None);
}

/// Review fixes: the sync landed but `GET` went unanswered, so the fold
/// waits (the chat runs nowhere here); the next contact folds it, once. An
/// archived offline chat isn't kept for it, it waits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fold_cut_off_after_the_sync_landed_finishes_on_the_next_contact() {
    use crate::chat_offline::{self, Synced};
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let key = "s-20261115-0f000105";
    let offline = moved_then_offline(&src, &rig, &link, key).await;
    let lossy = Lossy::new(link, Lose::Nothing);

    // Archived here: not sent now, not kept either.
    let archive = |on: i64| {
        session_store::with_write(&src.vault, "test", |tx| {
            tx.execute(
                "UPDATE sessions SET archived = ?2 WHERE key = ?1",
                rusqlite::params![offline, on],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap()
    };
    archive(1);
    let synced = chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert!(matches!(synced[0].1, Synced::Waiting(_)), "{synced:?}");
    assert_eq!(
        chat_offline::offline_of(&src.vault, &offline)
            .unwrap()
            .unwrap()
            .state,
        chat_offline::OPEN
    );
    archive(0);

    lossy.set(Lose::StatusLost);
    let synced = chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert!(matches!(synced[0].1, Synced::Waiting(_)), "{synced:?}");
    assert_eq!(
        chat_move::home_of(&src.vault, &offline)
            .unwrap()
            .unwrap()
            .state,
        chat_move::AWAY
    );
    assert!(chat_move::admission_refusal(&src.vault, &offline).is_some());
    assert_eq!(run_ids(&rig.vault, key).len(), 4, "landed there");
    assert_eq!(run_ids(&src.vault, key).len(), 2, "not folded here yet");

    lossy.set(Lose::Nothing);
    let synced = chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert_eq!(
        synced,
        vec![(offline.clone(), Synced::Appended(key.into()))]
    );
    assert_eq!(run_ids(&src.vault, key), run_ids(&rig.vault, key));
    assert!(chat_offline::sync_with(&src.vault, &lossy, MACBOOK)
        .await
        .is_empty());
    assert_eq!(run_ids(&rig.vault, key).len(), 4);
}

/// Review fixes: Keep it here on an offline chat's sync with the rig
/// answering: home, kept, never sent again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_it_here_on_an_offline_sync_keeps_it_over_the_routes() {
    use crate::chat_offline;
    let rig = rig().await;
    let src = source();
    let (link, _peers) = link_to(&rig).await;
    let key = "s-20261115-0f000106";
    let offline = moved_then_offline(&src, &rig, &link, key).await;
    let lossy = Lossy::new(link, Lose::All);
    let synced = chat_offline::sync_with(&src.vault, &lossy, MACBOOK).await;
    assert!(matches!(synced[0].1, chat_offline::Synced::Waiting(_)));
    lossy.set(Lose::Nothing);
    let kept = trigger::keep_chat_here(&src.vault, Some(&lossy), Some(&me()), &offline)
        .await
        .unwrap();
    assert_eq!(kept.outcome, MoveOutcome::Kept, "{kept:?}");
    assert_eq!(
        chat_offline::offline_of(&src.vault, &offline)
            .unwrap()
            .unwrap()
            .state,
        chat_offline::KEPT
    );
    assert!(chat_offline::sync_with(&src.vault, &lossy, MACBOOK)
        .await
        .is_empty());
    assert_eq!(trigger::retry_with(&src.vault, &lossy, &me()).await, 0);
    assert_eq!(run_ids(&rig.vault, key).len(), 2);
}

/// Review fix 2 (M5 slicing): the person's Move to runner on an offline
/// copy longer than one sync sends every slice over the routes, one after
/// another, and says moved only once all of its turns are there. As a Mac
/// of its own, so its large staged slices never crowd the other tests'.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn move_to_runner_on_a_long_offline_copy_sends_every_slice() {
    const OTHER: &str = "m-5a1ce5a1ce5a1ce5";
    let rig = rig().await;
    let src = source();
    let mut mac = this_mac();
    mac.machine_id = OTHER.into();
    let (link, _peers) = link_as(&rig, mac).await;
    let me = Me {
        machine_id: OTHER.into(),
        name: "MacBook Air".into(),
    };
    let key = "s-20261115-0f000111";
    idle_chat(&src, key);
    let sent = chat_move_send::send_move(&src.vault, &link, key, OTHER, &chat_move::new_move_id())
        .await
        .unwrap();
    assert_eq!(sent, Sent::Moved);
    let offline = crate::chat_offline::continue_offline(&src.vault, key, Utc::now()).unwrap();
    offline_turn(&src.vault, &offline, "off-1", "Add a museum");
    offline_turn(&src.vault, &offline, "off-2", "And lunch");
    let more = chat_move::TURNS_MAX as i64 + 1;
    session_store::with_write(&src.vault, "test", |tx| {
        for i in 0..more {
            tx.execute(
                "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked) \
                 VALUES(?1, ?2, ?3, '2026-11-15T10:00:00.000Z', '2026-11-15T10:00:01.000Z', 'success', 'More')",
                rusqlite::params![offline, 5 + i, format!("bulk-{i}")],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .unwrap();
    let turns = Running::new(&src.vault);
    let result =
        trigger::move_chat(&src.vault, &turns, &link, &me, "Test Mac", &offline, plan()).await;
    assert_eq!(result.outcome, MoveOutcome::Moved, "{result:?}");
    let there = run_ids(&rig.vault, key);
    assert_eq!(there.len() as i64, 4 + more, "every offline turn, once");
    assert_eq!(&there[2..4], ["off-1", "off-2"]);
    assert_eq!(
        there.last().map(String::as_str),
        Some(format!("bulk-{}", more - 1).as_str())
    );
    assert_eq!(run_ids(&src.vault, key), there, "the replica matches");
    assert!(session_store::session_by_key(&src.vault, &offline)
        .unwrap()
        .is_none());
    assert_eq!(owners(&src, &rig, key), 1);
}

/// Delta review: slice 1's answer read again while slice 2 is out and
/// unanswered (a running copy moved at sleep) is stale: nothing is sent,
/// the continuation stays owed to slice 2, and it goes once slice 2 lands.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_slice_answer_leaves_the_continuation_owed_to_the_next_slice() {
    const OTHER: &str = "m-5a1ce5a1ce5a1ce6";
    let rig = rig().await;
    let src = source();
    let mut mac = this_mac();
    mac.machine_id = OTHER.into();
    let (link, _peers) = link_as(&rig, mac).await;
    let me = Me {
        machine_id: OTHER.into(),
        name: "MacBook Air".into(),
    };
    let key = "s-20261115-0f000112";
    idle_chat(&src, key);
    let sent = chat_move_send::send_move(&src.vault, &link, key, OTHER, &chat_move::new_move_id())
        .await
        .unwrap();
    assert_eq!(sent, Sent::Moved);
    let offline = crate::chat_offline::continue_offline(&src.vault, key, Utc::now()).unwrap();
    let more = chat_move::TURNS_MAX as i64;
    session_store::with_write(&src.vault, "test", |tx| {
        for i in 0..more {
            tx.execute(
                "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked) \
                 VALUES(?1, ?2, ?3, '2026-11-15T10:00:00.000Z', '2026-11-15T10:00:01.000Z', 'success', 'More')",
                rusqlite::params![offline, 3 + i, format!("bulk-{i}")],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .unwrap();
    // Its turn runs as the Mac sleeps: slice 1 lands, slice 2 (the turn)
    // lands there but its answer never comes back.
    let turns = Running::new(&src.vault);
    let cwd = session_store::session_by_key(&src.vault, &offline)
        .unwrap()
        .unwrap()
        .cwd;
    turns.start(&offline, &cwd, "Keep going");
    let lossy = Lossy::new(link, Lose::LaterCommitAnswerThenAll);
    let result = trigger::move_chat(
        &src.vault,
        &turns,
        &lossy,
        &me,
        "Test Mac",
        &offline,
        plan(),
    )
    .await;
    assert_eq!(result.outcome, MoveOutcome::Waiting, "{result:?}");
    assert!(trigger::owes(&offline), "owed to slice 2");
    let outs = |vault: &Path| -> Vec<String> {
        session_store::with_read(vault, |conn| {
            let mut stmt = conn
                .prepare("SELECT move_id FROM chat_moves WHERE session_key = ?1 AND direction = 'out' ORDER BY rowid")
                .unwrap();
            let rows = stmt.query_map([&offline], |row| row.get(0)).unwrap();
            Ok(rows.map(Result::unwrap).collect())
        })
        .unwrap()
    };
    let moves = outs(&src.vault);
    assert_eq!(moves.len(), 2);

    // Slice 1's answer read again, with the Mac back: stale, nothing sent.
    lossy.set(Lose::Nothing);
    trigger::after_settled(
        &src.vault,
        Some(&lossy),
        Some(&me),
        &moves[0],
        &offline,
        &Sent::Moved,
    )
    .await;
    assert!(trigger::owes(&offline), "still owed to slice 2");
    assert_eq!(outs(&src.vault), moves, "no third send");
    assert!(rig.host.sent.lock().unwrap().is_empty());

    // Slice 2 settles on the next contact: every turn there, continued once.
    trigger::retry_with(&src.vault, &lossy, &me).await;
    assert!(!trigger::owes(&offline));
    let there = run_ids(&rig.vault, key);
    assert_eq!(there.len() as i64, 2 + more + 1, "{}", there.len());
    assert_eq!(
        there.last().map(String::as_str),
        Some(format!("run-live-{offline}").as_str())
    );
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1, "continued once");
}
