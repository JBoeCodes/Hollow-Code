//! M5 PR2 (2026-11-15): when a chat moves, and how it keeps going.
//!
//! [`chat_move_send`] moves one chat; this module decides which and when,
//! and brings a running turn to a stop first:
//!
//! - **A running turn** can't move mid-flight (its agent process lives on
//!   this Mac). It is asked to stop at its next safe point (P1a's pause:
//!   `chat::request_pause_for`, the next model request with no tool call
//!   open); one that reaches none by the budget's safe-point time is stopped
//!   outright (Stop). It ends `cancelled` with its partial reply kept, as an
//!   interrupted turn does. Once the chat moved, its new home gets one
//!   continuation turn ([`chat_move_send::continue_there`], a
//!   `clientMessageId` made from the move id, so a retry runs it once).
//! - **The sleep notice** (macOS): Hollow registers for system power
//!   (`IORegisterForSystemPower` on its own thread and run loop) and, on
//!   `kIOMessageSystemWillSleep`, moves every running chat of the person's
//!   to the paired runner within [`SLEEP_BUDGET`], then allows the sleep (the
//!   system waits up to 30 seconds, Apple QA1340; the network is still up).
//!   A safe point rarely comes that soon, so a sleep usually stops the turn
//!   outright. Idle sleep's `kIOMessageCanSystemSleep` is never vetoed. With
//!   Keep Hollow running with the lid closed on (`phone_access::lid`),
//!   system sleep is off and no notice comes: nothing moves. A Mac that
//!   loses power sends nothing; its chats resume here after a restart.
//! - **The setting** (`AgentSettings::move_on_sleep`, Settings' control is
//!   the UI PR's): absent is off, and a read error is off; on, it acts only
//!   while the runner (the paired Mac whose aliases include
//!   `~/.config/hollow/slot-remote`'s, [`pick_runner`]) is paired and answers
//!   within [`ANSWER_WITHIN`]. Never in `hollow serve` (the runner itself) or
//!   a candidate build ([`starts_here`]).
//! - **Move to runner** (`commands::chat_move_api`): the person's own move of
//!   one chat, running or not; a chat that was only waiting moves with no
//!   continuation.
//! - **Retries**: every [`RETRY_EVERY`] and at wake, every move still
//!   `leaving` is sent again and every `aborting` one gets its `DELETE`
//!   ([`chat_move_send::retry_leaving`], [`chat_move_send::settle_pending`]).
//!
//! Design: the M5 plan, sections 2.6 and 2.7.

use crate::chat_move;
use crate::chat_move_send::{self, Fut, MoveLink, PeerLink, Sent};
use crate::peer_machines::MachineRecord;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;
use tokio::time::Instant;

/// How long a sleep notice gives the moves before the sleep is allowed.
pub(crate) const SLEEP_BUDGET: Duration = Duration::from_secs(20);
/// Of a budget, what is kept for sending (the rest is for stopping turns).
pub(crate) const SEND_RESERVE: Duration = Duration::from_secs(8);
/// Of the stopping time, what is kept for turns stopped outright to end.
pub(crate) const STOP_SETTLE: Duration = Duration::from_secs(3);
/// Move to runner: how long a running turn may take to reach a safe point.
pub(crate) const MANUAL_SAFE_WAIT: Duration = Duration::from_secs(30);
/// The paired Mac must answer `GET /v1/machine` within this.
pub(crate) const ANSWER_WITHIN: Duration = Duration::from_secs(2);
/// Open moves are looked at again this often (and at wake).
pub(crate) const RETRY_EVERY: Duration = Duration::from_secs(60);
/// How often a stopping turn is looked at.
const POLL: Duration = Duration::from_millis(100);

// ── The result the person sees ───────────────────────────────────────────

/// How a move ended for the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub(crate) enum MoveOutcome {
    /// The chat lives on the other Mac now.
    Moved,
    /// It stays (or is back) on this Mac.
    Kept,
    /// The other Mac hasn't answered: the chat runs on neither until it
    /// does (tried again every minute and at wake).
    Waiting,
    /// It can't move (the reason says why); nothing changed.
    Refused,
    /// Its move is being sent or settled right now; try again in a moment.
    Busy,
}

/// One chat's move (`chat_move_to_runner`, `chat_move_keep_here`,
/// `chat_move_release_here`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatMoveResult {
    pub key: String,
    pub outcome: MoveOutcome,
    /// The other Mac's name, when there is one.
    pub to: Option<String>,
    /// One sentence for the person.
    pub message: String,
    /// The chat's continuation turn was taken there.
    pub continued: bool,
}

impl ChatMoveResult {
    pub(crate) fn new(key: &str, outcome: MoveOutcome, to: Option<&str>, message: String) -> Self {
        Self {
            key: key.to_string(),
            outcome,
            to: to.map(str::to_string),
            message,
            continued: false,
        }
    }
}

/// This Mac, as a move names it.
#[derive(Clone, Debug)]
pub(crate) struct Me {
    pub machine_id: String,
    pub name: String,
}

impl Me {
    /// From `machine.json` and the Mac's name (`peer_client::ThisMac`).
    pub(crate) fn read() -> Result<Self, String> {
        let me = crate::peer_client::ThisMac::read()?;
        Ok(Self {
            machine_id: me.machine_id,
            name: me.name,
        })
    }
}

// ── Running turns ────────────────────────────────────────────────────────

/// What the trigger asks of this Mac's running turns (the app's
/// [`AppTurns`]; tests stand in for it).
pub(crate) trait Turns: Send + Sync {
    /// The person's own chats with a turn running here.
    fn running(&self) -> Vec<String>;
    /// Ask `key`'s turn to stop at its next safe point.
    fn ask_safe_stop(&self, key: &str) -> bool;
    /// Stop `key`'s turn now.
    fn stop_now(&self, key: &str) -> bool;
    /// Whether `key` has nothing queued, claimed or running any more.
    fn ended(&self, vault: &Path, key: &str) -> bool;
}

/// The app's running turns (`chat`'s stop slots and P1a's pause).
pub(crate) struct AppTurns;

impl Turns for AppTurns {
    fn running(&self) -> Vec<String> {
        crate::chat::running_own_chats()
    }
    fn ask_safe_stop(&self, key: &str) -> bool {
        crate::chat::request_pause_for(key)
    }
    fn stop_now(&self, key: &str) -> bool {
        crate::chat::cancel_running(key)
    }
    fn ended(&self, vault: &Path, key: &str) -> bool {
        !crate::chat::turn_live(vault, key)
            && crate::agent_sessions::live_for_key(key).is_none()
            && !chat_move::turn_in_flight_now(vault, key)
    }
}

/// When turns must have stopped: at a safe point by `safe_by`, then stopped
/// outright and ended by `stop_by`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StopPlan {
    pub safe_by: Instant,
    pub stop_by: Instant,
}

impl StopPlan {
    /// A plan that sends by `by`: [`SEND_RESERVE`] (at most half of what is
    /// left) for sending, and of the rest [`STOP_SETTLE`] (at most half) for
    /// turns stopped outright.
    pub(crate) fn sending_by(by: Instant) -> Self {
        let now = Instant::now();
        let left = by.saturating_duration_since(now);
        let stop_by = now + left.saturating_sub(SEND_RESERVE.min(left / 2));
        let stopping = stop_by.saturating_duration_since(now);
        let safe_by = now + stopping.saturating_sub(STOP_SETTLE.min(stopping / 2));
        Self { safe_by, stop_by }
    }
}

/// Bring `keys`' turns to a stop by `plan`: a safe point asked of each at
/// once, then Stop for the ones still running at `plan.safe_by`. The keys
/// whose turns ended, in their order.
pub(crate) async fn stop_turns(
    turns: &dyn Turns,
    vault: &Path,
    keys: &[String],
    plan: StopPlan,
) -> Vec<String> {
    for key in keys {
        turns.ask_safe_stop(key);
    }
    let mut stopped = false;
    loop {
        let open: Vec<&String> = keys.iter().filter(|k| !turns.ended(vault, k)).collect();
        let now = Instant::now();
        if open.is_empty() || now >= plan.stop_by {
            break;
        }
        if !stopped && now >= plan.safe_by {
            for key in open {
                turns.stop_now(key);
            }
            stopped = true;
        }
        tokio::time::sleep(POLL).await;
    }
    keys.iter()
        .filter(|k| turns.ended(vault, k))
        .cloned()
        .collect()
}

// ── Continuations owed ───────────────────────────────────────────────────

/// Continuations owed to chats that were running when they moved, and the
/// notifications already told about moves (Lane M: kept in a file once
/// [`keep_owed_at`] ran, so a restart doesn't lose a continuation or tell a
/// move's notice again; before that, and in tests, in memory only).
/// Owed before the move is sent and cleared once the continuation is taken
/// or the move comes home.
struct OwedBook {
    path: Option<PathBuf>,
    book: crate::chat_move_owed::Book,
}

fn owed_book() -> &'static Mutex<OwedBook> {
    static BOOK: OnceLock<Mutex<OwedBook>> = OnceLock::new();
    BOOK.get_or_init(|| {
        Mutex::new(OwedBook {
            path: None,
            book: Default::default(),
        })
    })
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Change the book and, when it is kept in a file, write it. A write that
/// fails is logged: the memory copy goes on.
fn with_owed<R>(f: impl FnOnce(&mut crate::chat_move_owed::Book) -> R) -> R {
    let mut held = owed_book().lock().unwrap_or_else(PoisonError::into_inner);
    let out = f(&mut held.book);
    if let Some(path) = &held.path {
        if let Err(error) = held.book.save(path) {
            crate::launch::elog(&format!(
                "[chat move] owed continuations not saved: {error}"
            ));
        }
    }
    out
}

/// Keep the owed continuations and told notices in the file at `path` from
/// now on, reading what it holds (`start`, at boot). Entries already owed
/// in memory stay.
pub(crate) fn keep_owed_at(path: PathBuf) {
    let mut held = owed_book().lock().unwrap_or_else(PoisonError::into_inner);
    let mut loaded = crate::chat_move_owed::Book::load(&path);
    for (move_id, key) in held.book.owed_moves() {
        loaded.owe(&move_id, &key, now_ms());
    }
    loaded.prune(now_ms());
    held.book = loaded;
    held.path = Some(path);
}

fn owe(move_id: &str, key: &str) {
    with_owed(|book| book.owe(move_id, key, now_ms()));
}

/// Whether a continuation is owed for chat `key` (tests).
#[cfg(test)]
pub(crate) fn owes(key: &str) -> bool {
    owed_book()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .book
        .owes(key)
}

/// Take move `move_id`'s owed continuation (its chat key), once. The file
/// is written only when there was one.
fn take_owed(move_id: &str) -> Option<String> {
    let owed = owed_book()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .book
        .owed_moves()
        .into_iter()
        .any(|(id, _)| id == move_id);
    owed.then(|| with_owed(|book| book.take(move_id))).flatten()
}

/// A restart, as far as chat `key`'s owed continuation goes (tests): the
/// entry leaves memory without the file being written, so only the file
/// still has it. Other tests' entries stay: the book is the process's.
#[cfg(test)]
pub(crate) fn forget_owed_in_memory(key: &str) {
    let mut held = owed_book().lock().unwrap_or_else(PoisonError::into_inner);
    held.book.owed.retain(|o| o.key != key);
}

/// Stop mirroring the book to a file (tests).
#[cfg(test)]
pub(crate) fn stop_keeping_owed() {
    owed_book()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .path = None;
}

/// The continuations owed to moves that landed, replayed on contact (a
/// restart lost the send): each goes through [`after_settled`] as a move
/// that landed does, which sends it once. A move called off or released
/// owes none. A move still open is left to the retry that settles it.
async fn replay_owed(vault: &Path, link: &dyn MoveLink, me: &Me) {
    let owed = owed_book()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .book
        .owed_moves();
    for (move_id, key) in owed {
        let (v, id) = (vault.to_path_buf(), move_id.clone());
        let state = tokio::task::spawn_blocking(move || chat_move::settled_out(&v, &id))
            .await
            .ok()
            .and_then(Result::ok);
        match state {
            // Landed: send it (or, for a sliced offline chat, what
            // `after_settled` decides).
            Some(Some((chat_move::Settled::Done, peer))) if peer == link.peer() => {
                after_settled(vault, Some(link), Some(me), &move_id, &key, &Sent::Moved).await;
            }
            // Open, or another Mac's: not now.
            Some(Some((chat_move::Settled::Open | chat_move::Settled::Done, _))) => {}
            // Called off or released: nothing is owed.
            Some(Some((chat_move::Settled::Gone, _))) => {
                take_owed(&move_id);
            }
            // No such move in this vault (the book is the process's, and
            // an entry waits for its own vault's move; it expires), or the
            // read failed: not ours to drop.
            Some(None) | None => {}
        }
    }
}

// ── One chat ─────────────────────────────────────────────────────────────

/// Move chat `key` to the Mac `link` calls (`to`, its name). A running turn
/// is stopped first by `plan`; a chat that can't move is refused before its
/// turn is touched.
pub(crate) async fn move_chat(
    vault: &Path,
    turns: &dyn Turns,
    link: &dyn MoveLink,
    me: &Me,
    to: &str,
    key: &str,
    plan: StopPlan,
) -> ChatMoveResult {
    let refused =
        |message: String| ChatMoveResult::new(key, MoveOutcome::Refused, Some(to), message);
    if let Err(error) = check_movable(vault, key, &me.machine_id).await {
        return refused(error.message);
    }
    let running = turns.running().iter().any(|k| k == key);
    if running {
        let ended = stop_turns(turns, vault, &[key.to_string()], plan).await;
        if ended.is_empty() {
            return refused("Its turn didn't stop in time, so the chat stays on this Mac.".into());
        }
    } else if !turns.ended(vault, key) {
        return refused(
            "It has a message waiting to start here. Move it once that turn has run.".into(),
        );
    }
    send_one(vault, link, me, to, key, running).await
}

async fn check_movable(vault: &Path, key: &str, from: &str) -> Result<(), chat_move::MoveError> {
    let (v, k, f) = (vault.to_path_buf(), key.to_string(), from.to_string());
    tokio::task::spawn_blocking(move || chat_move::movable(&v, &k, &f))
        .await
        .unwrap_or_else(|e| Err(chat_move::MoveError::from(e.to_string())))
}

/// Send chat `key` (its turn, if `was_running`, stopped already) and, once
/// it moved, its continuation.
async fn send_one(
    vault: &Path,
    link: &dyn MoveLink,
    me: &Me,
    to: &str,
    key: &str,
    was_running: bool,
) -> ChatMoveResult {
    let move_id = chat_move::new_move_id();
    // Owed before anything is sent: a send cut off (the sleep's budget, a
    // quit) still owes the chat its continuation once the move lands.
    if was_running {
        owe(&move_id, key);
    }
    let sent = chat_move_send::send_move(vault, link, key, &me.machine_id, &move_id).await;
    let mut result = match sent {
        Err(error) => {
            take_owed(&move_id);
            return ChatMoveResult::new(key, MoveOutcome::Refused, Some(to), error.message);
        }
        Ok(Sent::Moved) => ChatMoveResult::new(
            key,
            MoveOutcome::Moved,
            Some(to),
            format!("Moved to {to}; it keeps going there."),
        ),
        Ok(Sent::Home(why)) => ChatMoveResult::new(
            key,
            MoveOutcome::Kept,
            Some(to),
            format!("It stays on this Mac: {why}"),
        ),
        Ok(Sent::Busy) => ChatMoveResult::new(
            key,
            MoveOutcome::Busy,
            Some(to),
            "This chat's move is being sent right now. Try again in a moment.".into(),
        ),
        Ok(Sent::Waiting(why)) => ChatMoveResult::new(
            key,
            MoveOutcome::Waiting,
            Some(to),
            format!("{to} hasn't answered ({why}). The chat runs on neither Mac until it does; Hollow tries again every minute."),
        ),
    };
    if result.outcome == MoveOutcome::Moved {
        moved_away(vault, key);
    }
    match result.outcome {
        MoveOutcome::Moved => {
            // PR3: an offline chat's turns may have joined the chat it
            // continued there; the continuation goes where they are. A long
            // offline stretch goes in slices, sent one after another here.
            let (placed, last) = land_slices(vault, link, me, &move_id, key).await;
            match placed {
                Placed::There(there) if was_running => {
                    result.continued = continue_or_owe(link, &there, &last, &me.name).await;
                }
                Placed::There(_) => {}
                Placed::Unplaced => drop_unplaced(vault, &last, key),
                Placed::Rest { moving, why } => {
                    result.outcome = MoveOutcome::Waiting;
                    result.message = rest_message(to, moving, &why);
                    if !moving {
                        drop_sliced(vault, &last, key);
                    }
                }
            }
        }
        // Still owed (when it was running) until the move settles.
        MoveOutcome::Waiting => {}
        MoveOutcome::Kept | MoveOutcome::Refused | MoveOutcome::Busy => {
            take_owed(&move_id);
        }
    }
    result
}

/// Where a landed move's turns are once [`land_slices`] is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Placed {
    /// Under this key on the other Mac.
    There(String),
    /// Landed, but that Mac's answer can't be had now.
    Unplaced,
    /// An offline chat whose turns go in slices, not all there yet: the
    /// next slice is out (`moving`: it runs nowhere until that Mac answers)
    /// or follows on the next contact (it is back here).
    Rest { moving: bool, why: String },
}

/// Once move `move_id` of chat `key` landed: fold the answer in and, while
/// it was one slice of an offline chat's turns, send the next at once (at
/// most `chat_offline::SLICES_PER_CONTACT` slices in all, the first
/// included), the owed continuation moving with each. A stale answer (a
/// slice folded already) does nothing: the continuation stays where it is
/// owed. Where the turns are, and the move id the continuation (if any) is
/// owed under.
pub(crate) async fn land_slices(
    vault: &Path,
    link: &dyn MoveLink,
    me: &Me,
    move_id: &str,
    key: &str,
) -> (Placed, String) {
    let mut id = move_id.to_string();
    for _ in 1..crate::chat_offline::SLICES_PER_CONTACT {
        match crate::chat_offline::landed(vault, link, &id, key).await {
            crate::chat_offline::Landed::There(there) => return (Placed::There(there), id),
            crate::chat_offline::Landed::Unplaced => return (Placed::Unplaced, id),
            crate::chat_offline::Landed::Stale => return (stale(), id),
            crate::chat_offline::Landed::Sliced => {}
        }
        let next = chat_move::new_move_id();
        if let Some(owed_key) = take_owed(&id) {
            owe(&next, &owed_key);
        }
        id = next;
        match chat_move_send::send_move(vault, link, key, &me.machine_id, &id).await {
            Ok(Sent::Moved) => moved_away(vault, key),
            // Out and unanswered: settled by the retry (`after_settled`),
            // which sends what is left after it.
            Ok(Sent::Waiting(why)) => return (Placed::Rest { moving: true, why }, id),
            Ok(Sent::Home(why)) => return (Placed::Rest { moving: false, why }, id),
            Ok(Sent::Busy) => {
                let why = "a turn of it is running".to_string();
                return (Placed::Rest { moving: false, why }, id);
            }
            Err(error) => {
                return (
                    Placed::Rest {
                        moving: false,
                        why: error.message,
                    },
                    id,
                )
            }
        }
    }
    let why = "more than one contact carries".to_string();
    match crate::chat_offline::landed(vault, link, &id, key).await {
        crate::chat_offline::Landed::There(there) => (Placed::There(there), id),
        crate::chat_offline::Landed::Unplaced => (Placed::Unplaced, id),
        crate::chat_offline::Landed::Stale => (stale(), id),
        crate::chat_offline::Landed::Sliced => (Placed::Rest { moving: false, why }, id),
    }
}

/// A stale slice answer: another send of the chat goes on (counted as a
/// move still out, so no owed continuation is dropped).
fn stale() -> Placed {
    Placed::Rest {
        moving: true,
        why: "another part of it is on its way".into(),
    }
}

/// The result's sentence when only part of an offline chat's turns synced.
fn rest_message(to: &str, moving: bool, why: &str) -> String {
    if moving {
        format!("Part of its offline turns synced to {to}; the rest is on its way ({why}). The chat runs on neither Mac until {to} answers; Hollow tries again every minute.")
    } else {
        format!("Part of its offline turns synced to {to}; the rest follows on the next contact ({why}). It stays on this Mac until then.")
    }
}

/// An offline chat's turns go in slices and not all are there yet: its
/// owed continuation is dropped (the chat is back here, its turn ended),
/// and the person is told once.
fn drop_sliced(vault: &Path, move_id: &str, key: &str) {
    if take_owed(move_id).is_some() {
        crate::launch::elog(&format!(
            "[chat move] {key}: synced in part; no continuation sent"
        ));
        report_once(
            vault,
            move_id,
            "sliced",
            "A chat moved but didn't continue",
            "A chat continued offline had more turns than one sync carries, so they sync in parts and its stopped turn wasn't continued. It stays on this Mac until the rest has synced; then continue it on the other Mac.".into(),
        );
    }
}

/// PR3: an offline chat's sync landed but where its turns went isn't known
/// yet (no answer to `GET`): its owed continuation is dropped, never sent to
/// a key that may not be there, and the person is told once (the chat
/// waits for them there; the next contact folds the answer in here).
fn drop_unplaced(vault: &Path, move_id: &str, key: &str) {
    if take_owed(move_id).is_some() {
        crate::launch::elog(&format!(
            "[chat move] {key}: landed, but where isn't known yet; no continuation sent"
        ));
        report_once(
            vault,
            move_id,
            "unplaced",
            "A chat moved but didn't continue",
            "A chat continued offline was synced to the other Mac while its turn was running, but that Mac didn't say where its turns went, so the turn wasn't continued there. Open the chat on that Mac to continue it.".into(),
        );
    }
}

/// A chat that moved away keeps no paused record here (its native session
/// is this Mac's; the chat continues there).
fn moved_away(vault: &Path, key: &str) {
    if let Err(error) = crate::chat_pause::clear(vault, key) {
        crate::launch::elog(&format!("[chat move] {key}: {error}"));
    }
}

async fn continue_or_owe(link: &dyn MoveLink, key: &str, move_id: &str, from: &str) -> bool {
    match chat_move_send::continue_there(link, key, move_id, from).await {
        Ok(accepted) if accepted.accepted => {
            take_owed(move_id);
            true
        }
        Ok(_) | Err(_) => {
            owe(move_id, key);
            false
        }
    }
}

/// Move every running chat of the person's that can move to the Mac `link`
/// calls, done by `by`: safe points asked of all at once, Stop for the rest
/// ([`StopPlan::sending_by`]), then each chat sent in turn. A send cut off
/// at `by` leaves its chat `leaving` (the next contact settles it).
pub(crate) async fn move_running(
    vault: &Path,
    turns: &dyn Turns,
    link: &dyn MoveLink,
    me: &Me,
    to: &str,
    by: Instant,
) -> Vec<ChatMoveResult> {
    let mut keys = Vec::new();
    for key in turns.running() {
        if check_movable(vault, &key, &me.machine_id).await.is_ok() {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        return Vec::new();
    }
    let ended = stop_turns(turns, vault, &keys, StopPlan::sending_by(by)).await;
    let mut out = Vec::new();
    for key in keys {
        if !ended.contains(&key) {
            out.push(ChatMoveResult::new(
                &key,
                MoveOutcome::Refused,
                Some(to),
                "Its turn didn't stop in time, so the chat stays on this Mac.".into(),
            ));
            continue;
        }
        let sent = tokio::time::timeout_at(by, send_one(vault, link, me, to, &key, true)).await;
        out.push(sent.unwrap_or_else(|_| {
            ChatMoveResult::new(
                &key,
                MoveOutcome::Waiting,
                Some(to),
                "The Mac went to sleep before the move was answered; it finishes on the next contact.".into(),
            )
        }));
    }
    out
}

/// On contact with `link`'s Mac: settle the moves called off, send the ones
/// still leaving again, and send any continuation owed for a move that now
/// landed. How many moves settled.
pub(crate) async fn retry_with(vault: &Path, link: &dyn MoveLink, me: &Me) -> usize {
    let mut settled = 0;
    let pending = chat_move_send::settle_pending(vault, link).await;
    let leaving = chat_move_send::retry_leaving(vault, link, &me.machine_id).await;
    for (move_id, key, sent) in pending.into_iter().chain(leaving) {
        note_settled(vault, &move_id, &key, &sent);
        settled += usize::from(matches!(sent, Sent::Moved | Sent::Home(_)));
        after_settled(vault, Some(link), Some(me), &move_id, &key, &sent).await;
    }
    // Lane M: a continuation a restart lost the send of (its move landed).
    replay_owed(vault, link, me).await;
    settled
}

/// Once move `move_id` of chat `key` settled, whichever way (a send, a
/// retry, a pending abort's `DELETE`, Keep it here): a chat that moved
/// keeps no pause record here and gets its owed continuation (owed again
/// when it can't be sent now: no `link` or `me`, or no answer); a chat that
/// came home owes none. Waiting or busy: nothing yet.
pub(crate) async fn after_settled(
    vault: &Path,
    link: Option<&dyn MoveLink>,
    me: Option<&Me>,
    move_id: &str,
    key: &str,
    sent: &Sent,
) {
    match sent {
        Sent::Moved => {
            moved_away(vault, key);
            // PR3: an offline chat's sync is folded in on that Mac's answer,
            // and the rest of a sliced one sent after it.
            let (placed, last) = match (link, me) {
                (Some(link), Some(me)) => land_slices(vault, link, me, move_id, key).await,
                (Some(link), None) => {
                    let placed = match crate::chat_offline::landed(vault, link, move_id, key).await
                    {
                        crate::chat_offline::Landed::There(there) => Placed::There(there),
                        crate::chat_offline::Landed::Unplaced => Placed::Unplaced,
                        crate::chat_offline::Landed::Stale => stale(),
                        crate::chat_offline::Landed::Sliced => Placed::Rest {
                            moving: false,
                            why: "the rest follows on the next contact".into(),
                        },
                    };
                    (placed, move_id.to_string())
                }
                (None, _) => (Placed::There(key.to_string()), move_id.to_string()),
            };
            let there = match placed {
                Placed::There(there) => there,
                Placed::Unplaced => return drop_unplaced(vault, &last, key),
                Placed::Rest { moving: true, .. } => return,
                Placed::Rest { moving: false, .. } => return drop_sliced(vault, &last, key),
            };
            if take_owed(&last).is_some() {
                match (link, me) {
                    (Some(link), Some(me)) => {
                        continue_or_owe(link, &there, &last, &me.name).await;
                    }
                    _ => owe(&last, key),
                }
            }
        }
        Sent::Home(_) => {
            take_owed(move_id);
        }
        Sent::Waiting(_) | Sent::Busy => {}
    }
}

/// Forget move `move_id`'s owed continuation (the person released it).
pub(crate) fn drop_owed(move_id: &str) {
    take_owed(move_id);
}

/// Keep it here on chat `key` (a `leaving` chat): its move called off on
/// the other Mac's answer when `link` reaches it, or left `aborting` for
/// the next contact; the continuation owed is settled as the move is.
pub(crate) async fn keep_chat_here(
    vault: &Path,
    link: Option<&dyn MoveLink>,
    me: Option<&Me>,
    key: &str,
) -> Result<ChatMoveResult, String> {
    let move_id = match chat_move::home_of(vault, key)? {
        Some(home) if home.state == chat_move::LEAVING => home.move_id,
        Some(home) if home.state == chat_move::AWAY => {
            return Err("This chat moved to another Mac and keeps going there.".into())
        }
        _ => return Err("This chat isn't moving; it is this Mac's.".into()),
    };
    let sent = chat_move_send::keep_here(vault, link, &move_id).await;
    after_settled(vault, link, me, &move_id, key, &sent).await;
    let (outcome, message) = match sent {
        Sent::Home(_) => (MoveOutcome::Kept, "Kept on this Mac; it runs here again."),
        Sent::Moved => (
            MoveOutcome::Moved,
            "It already ran on the other Mac, so it stays there.",
        ),
        Sent::Busy => (
            MoveOutcome::Busy,
            "This chat's move is being sent right now. Try Keep it here again in a moment.",
        ),
        Sent::Waiting(_) => (
            MoveOutcome::Waiting,
            "The other Mac hasn't answered. The chat runs here once it does, or Release here if it never comes back.",
        ),
    };
    Ok(ChatMoveResult::new(key, outcome, None, message.into()))
}

/// A move open longer than this is reported once (log and notification).
pub(crate) const STUCK_AFTER: Duration = Duration::from_secs(15 * 60);

/// Report `kind` of move `move_id` once: marked before the notification is
/// posted (so a crash between loses it rather than telling it twice, and a
/// restart doesn't tell it again) and taken back when posting fails, so the
/// next time tries again.
fn report_once(vault: &Path, move_id: &str, kind: &'static str, title: &str, body: String) -> bool {
    if !with_owed(|book| book.mark_reported(move_id, kind, now_ms())) {
        return false;
    }
    if notify(vault, title, body) {
        return true;
    }
    with_owed(|book| book.unmark_reported(move_id, kind));
    false
}

/// Post a warning in Hollow's notifications; false when it failed.
pub(crate) fn notify(vault: &Path, title: &str, body: String) -> bool {
    let input = crate::notifications::NotificationInput {
        title: title.to_string(),
        body,
        kind: "warning".into(),
        source: Some("chat-move".into()),
        action_label: None,
        action_url: None,
        notify_native: false,
    };
    match crate::notifications::create_registered(vault, input) {
        Some(Err(error)) => {
            crate::launch::elog(&format!("[chat move] notification: {error}"));
            false
        }
        _ => true,
    }
}

/// Log how a retried move settled; a move that came home after all (its
/// folder gone, too large, refused there) is told once.
fn note_settled(vault: &Path, move_id: &str, key: &str, sent: &Sent) {
    match sent {
        Sent::Waiting(_) | Sent::Busy => {}
        Sent::Moved => crate::launch::elog(&format!("[chat move] {key}: {move_id} landed")),
        Sent::Home(why) => {
            crate::launch::elog(&format!("[chat move] {key}: {move_id} came home: {why}"));
            report_once(
                vault,
                move_id,
                "home",
                "A chat didn't move",
                format!("It runs on this Mac again: {why}"),
            );
        }
    }
}

/// Moves open longer than [`STUCK_AFTER`] (the other Mac silent): logged
/// and told once each, with what the person can do.
pub(crate) fn report_stuck(vault: &Path, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
    let before = now - chrono::Duration::from_std(STUCK_AFTER).unwrap_or_default();
    let stuck = chat_move::open_since(vault, before).unwrap_or_default();
    let mut told = Vec::new();
    for (move_id, key, _) in stuck {
        let told_now = report_once(
            vault,
            &move_id,
            "stuck",
            "A chat is waiting to move",
            "The other Mac hasn't answered for 15 minutes, so the chat runs on neither Mac. Choose Keep it here on the chat to run it on this Mac once that Mac answers, or Release here if it never comes back.".into(),
        );
        if told_now {
            crate::launch::elog(&format!(
                "[chat move] {key}: {move_id} still open after 15 minutes"
            ));
            told.push(key);
        }
    }
    told
}

// ── The sleep notice ─────────────────────────────────────────────────────

/// A system power message, as the trigger reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Power {
    /// `kIOMessageSystemWillSleep`: the system is going to sleep.
    WillSleep,
    /// `kIOMessageCanSystemSleep`: idle sleep asks; never vetoed.
    CanSleep,
    /// `kIOMessageSystemHasPoweredOn`: awake again.
    PoweredOn,
}

/// The power message `message` (IOKit's `kIOMessage...` value), if the
/// trigger reads it.
pub(crate) fn power_event(message: u32) -> Option<Power> {
    match message {
        0xE000_0280 => Some(Power::WillSleep),
        0xE000_0270 => Some(Power::CanSleep),
        0xE000_0300 => Some(Power::PoweredOn),
        _ => None,
    }
}

/// What a sleep notice moves (the app's [`AppMover`]; tests count calls).
pub(crate) trait SleepMover: Send + Sync {
    /// Whether a sleep moves anything now (the setting on, a Mac paired).
    fn enabled(&self) -> bool;
    /// Move the running chats within `budget`; how many moved.
    fn move_running(&self, budget: Duration) -> Fut<'_, usize>;
    /// The Mac woke: look at the open moves now.
    fn woke(&self);
}

/// The sleep notice's handler: on `WillSleep`, with the setting on, runs
/// the mover once within [`SLEEP_BUDGET`] and returns once it is done, so
/// the caller allows the sleep after it.
pub(crate) struct Trigger {
    mover: Arc<dyn SleepMover>,
    runtime: tokio::runtime::Handle,
    moving: AtomicBool,
}

impl Trigger {
    pub(crate) fn new(mover: Arc<dyn SleepMover>, runtime: tokio::runtime::Handle) -> Self {
        Self {
            mover,
            runtime,
            moving: AtomicBool::new(false),
        }
    }

    /// Handle `event` on the power thread (never an async one): blocks for
    /// at most [`SLEEP_BUDGET`] on `WillSleep`. Whether the mover ran.
    pub(crate) fn on_power(&self, event: Power) -> bool {
        match event {
            Power::CanSleep => false,
            Power::PoweredOn => {
                self.mover.woke();
                false
            }
            Power::WillSleep => {
                if !self.mover.enabled() || self.moving.swap(true, Ordering::SeqCst) {
                    return false;
                }
                // Cleared however this ends, a panic in the mover included.
                struct Moving<'a>(&'a AtomicBool);
                impl Drop for Moving<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::SeqCst);
                    }
                }
                let _moving = Moving(&self.moving);
                let mover = self.mover.clone();
                let moved = self.runtime.block_on(async move {
                    tokio::time::timeout(SLEEP_BUDGET, mover.move_running(SLEEP_BUDGET)).await
                });
                if let Ok(n) = moved {
                    crate::launch::elog(&format!("[chat move] sleep: {n} chat(s) moved"));
                }
                true
            }
        }
    }
}

/// Whether the setting moves running chats when this Mac sleeps: only when
/// the person turned it on (absent is off: the runner may bill another
/// account, so moving chats while the person sleeps is their choice).
pub(crate) fn move_on_sleep(settings: &crate::agent_settings::AgentSettings) -> bool {
    settings.move_on_sleep == Some(true)
}

/// Whether a sleep moves anything: the setting read and on (a read error is
/// off), and a runner named (`~/.config/hollow/slot-remote`) that is one of
/// the paired Macs.
pub(crate) fn sleep_moves(
    settings: Result<crate::agent_settings::AgentSettings, String>,
    records: &[MachineRecord],
    runner_alias: Option<&str>,
) -> bool {
    settings.is_ok_and(|s| move_on_sleep(&s)) && pick_runner(records, runner_alias).is_ok()
}

/// The paired Mac a sleep moves chats to: only the runner, the paired Mac
/// whose ssh aliases include this Mac's runner alias
/// (`~/.config/hollow/slot-remote`). Any other paired Mac never takes a chat
/// the person didn't move.
pub(crate) fn pick_runner<'a>(
    records: &'a [MachineRecord],
    runner_alias: Option<&str>,
) -> Result<&'a MachineRecord, String> {
    let alias = runner_alias
        .map(str::trim)
        .filter(|a| !a.is_empty() && *a != "off")
        .ok_or("No runner is set up. Name this Mac's runner in Settings first.")?;
    records
        .iter()
        .find(|r| r.ssh_hosts.iter().any(|h| h == alias))
        .ok_or_else(|| not_matched(records, alias))
}

/// The paired Mac Move to runner moves a chat to (the person's own click):
/// the one the runner alias reaches, else the only one paired (it must also
/// take moves, checked when it answers). Several and none matching: refused,
/// naming the alias and the aliases compared.
pub(crate) fn pick_for_move<'a>(
    records: &'a [MachineRecord],
    runner_alias: Option<&str>,
) -> Result<&'a MachineRecord, String> {
    if let Ok(runner) = pick_runner(records, runner_alias) {
        return Ok(runner);
    }
    match records {
        [] => Err("No Mac is paired with this one. Pair one in Settings first.".into()),
        [only] => Ok(only),
        _ => Err(match runner_alias.map(str::trim).filter(|a| !a.is_empty()) {
            Some(alias) => not_matched(records, alias),
            None => "Several Macs are paired and no runner is named. Name this Mac's runner in Settings first.".into(),
        }),
    }
}

fn not_matched(records: &[MachineRecord], alias: &str) -> String {
    let compared: Vec<&str> = records
        .iter()
        .flat_map(|r| r.ssh_hosts.iter().map(String::as_str))
        .collect();
    format!(
        "No paired Mac is reached through the runner alias \"{alias}\" (paired through: {}).",
        if compared.is_empty() {
            "none".to_string()
        } else {
            compared.join(", ")
        }
    )
}

/// The runner's link, its name, checked: it answers `GET /v1/machine`
/// within `within` as the paired Mac and takes moves.
/// `sleep`: the automatic move, which takes only the runner
/// ([`pick_runner`]); otherwise the person's Move to runner
/// ([`pick_for_move`]).
pub(crate) async fn runner_link(
    within: Duration,
    sleep: bool,
) -> Result<(PeerLink, String), String> {
    let hub = crate::commands::peer_api::Hub::for_app().map_err(|e| e.message)?;
    let records = hub.paired().await.map_err(|e| e.message)?;
    let runner = crate::remote_session::configured_runner();
    let record = if sleep {
        pick_runner(&records, runner.as_deref())?
    } else {
        pick_for_move(&records, runner.as_deref())?
    }
    .clone();
    link_for(&hub, &record.machine_id, Some(within))
        .await
        .map(|link| (link, record.name))
}

/// The link to paired Mac `machine_id`; with `within`, checked first.
pub(crate) async fn link_for(
    hub: &crate::commands::peer_api::Hub,
    machine_id: &str,
    within: Option<Duration>,
) -> Result<PeerLink, String> {
    let client = hub.move_client(machine_id).await.map_err(|e| e.message)?;
    let link = PeerLink::new(client).ok_or("That Mac isn't paired with this Hollow.")?;
    if let Some(within) = within {
        let info = tokio::time::timeout(within, link.client().machine())
            .await
            .map_err(|_| format!("{} didn't answer in time.", link.client().name()))?
            .map_err(|e| e.message().to_string())?;
        if !info.capabilities.iter().any(|c| c == "moves") {
            return Err(format!(
                "{}'s Hollow can't take chats yet. Update Hollow there.",
                link.client().name()
            ));
        }
    }
    Ok(link)
}

/// The vault as config.json names it now (read each time, so a vault that
/// moved since launch is the one used); `None` without one on disk.
pub(crate) fn current_vault() -> Option<PathBuf> {
    crate::config::locate_and_load()
        .ok()
        .map(|(config, _)| config.vault_path)
        .filter(|vault| vault.exists())
}

/// [`current_vault`] off the async threads: the one vault path source for
/// the trigger and the person's move commands.
pub(crate) async fn vault_now() -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(current_vault)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| "Hollow can't find its vault on this Mac.".into())
}

/// The app's mover.
struct AppMover;

impl SleepMover for AppMover {
    fn enabled(&self) -> bool {
        let records = crate::peer_machines::Machines::for_app()
            .and_then(|m| m.list().ok())
            .unwrap_or_default();
        sleep_moves(
            crate::agent_settings::get_settings(),
            &records,
            crate::remote_session::configured_runner().as_deref(),
        )
    }

    fn move_running(&self, budget: Duration) -> Fut<'_, usize> {
        Box::pin(async move {
            let by = Instant::now() + budget;
            if AppTurns.running().is_empty() {
                return 0;
            }
            let Ok(vault) = vault_now().await else {
                return 0;
            };
            let (link, to) = match runner_link(ANSWER_WITHIN, true).await {
                Ok(found) => found,
                Err(why) => {
                    crate::launch::elog(&format!("[chat move] sleep: nothing moved: {why}"));
                    return 0;
                }
            };
            let me = match tokio::task::spawn_blocking(Me::read).await {
                Ok(Ok(me)) => me,
                _ => return 0,
            };
            let results = move_running(&vault, &AppTurns, &link, &me, &to, by).await;
            for result in &results {
                crate::launch::elog(&format!(
                    "[chat move] {}: {:?}: {}",
                    result.key, result.outcome, result.message
                ));
            }
            results
                .iter()
                .filter(|r| r.outcome == MoveOutcome::Moved)
                .count()
        })
    }

    fn woke(&self) {
        retry_signal().notify_one();
    }
}

fn retry_signal() -> &'static tokio::sync::Notify {
    static RETRY: OnceLock<tokio::sync::Notify> = OnceLock::new();
    RETRY.get_or_init(tokio::sync::Notify::new)
}

/// The open moves' Macs (moves `leaving` or `aborting`) and, since PR3,
/// the Macs with an offline chat to sync or fold in.
fn open_peers(vault: &Path) -> BTreeSet<String> {
    let mut peers = crate::chat_offline::pending_peers(vault);
    for list in [
        chat_move::leaving_moves(vault),
        chat_move::pending_aborts(vault),
    ] {
        peers.extend(
            list.unwrap_or_default()
                .into_iter()
                .map(|(_, _, peer)| peer),
        );
    }
    peers
}

async fn retry_open() {
    let Ok(vault) = vault_now().await else {
        return;
    };
    let vault = vault.as_path();
    let v = vault.to_path_buf();
    let peers = tokio::task::spawn_blocking(move || {
        report_stuck(&v, chrono::Utc::now());
        if let Err(error) = chat_move::prune_slices_in(&v, chrono::Utc::now()) {
            crate::launch::elog(&format!("[chat move] {error}"));
        }
        open_peers(&v)
    })
    .await
    .unwrap_or_default();
    if peers.is_empty() {
        return;
    }
    let Ok(hub) = crate::commands::peer_api::Hub::for_app() else {
        return;
    };
    let me = match tokio::task::spawn_blocking(Me::read).await {
        Ok(Ok(me)) => me,
        _ => return,
    };
    let v = vault.to_path_buf();
    let offline = tokio::task::spawn_blocking(move || crate::chat_offline::pending_peers(&v))
        .await
        .unwrap_or_default();
    for peer in peers {
        if let Ok(link) = link_for(&hub, &peer, None).await {
            retry_with(vault, &link, &me).await;
            if offline.contains(&peer) && takes_appends(&link).await {
                for (key, synced) in
                    crate::chat_offline::sync_with(vault, &link, &me.machine_id).await
                {
                    crate::launch::elog(&format!("[chat offline] {key}: {synced:?}"));
                }
            }
        }
    }
}

/// PR3: whether `link`'s Mac answers now and takes offline syncs
/// (`appends` in `GET /v1/machine`).
async fn takes_appends(link: &PeerLink) -> bool {
    tokio::time::timeout(ANSWER_WITHIN, link.client().machine())
        .await
        .ok()
        .and_then(Result::ok)
        .is_some_and(|info| info.capabilities.iter().any(|c| c == "appends"))
}

/// Whether this process starts the trigger: the app with its window (never
/// `hollow serve`, the runner itself: a runner that sleeps must not move its
/// chats away), the run queue's owner, never a candidate build (even one
/// allowed background work), and only once configured.
pub(crate) fn starts_here(gui: bool, queue_owned: bool, candidate: bool, configured: bool) -> bool {
    gui && queue_owned && !candidate && configured
}

/// Start the trigger (`core_boot`, behind [`starts_here`]): the retry loop,
/// and on macOS the sleep notice's thread. The vault is read each time.
pub(crate) fn start() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    // Lane M: continuations owed and notices told before a restart.
    if let Some(path) = crate::chat_move_owed::app_file() {
        keep_owed_at(path);
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(RETRY_EVERY) => {}
                _ = retry_signal().notified() => {}
            }
            retry_open().await;
        }
    });
    #[cfg(target_os = "macos")]
    {
        let runtime = tauri::async_runtime::handle().inner().clone();
        let trigger = Trigger::new(Arc::new(AppMover), runtime);
        power::listen(trigger);
    }
}

/// The system power notifications (IOKit), on a thread with its own run
/// loop. The callback allows every power change it is asked about, after
/// the trigger is done with a `WillSleep`.
#[cfg(target_os = "macos")]
mod power {
    use super::{power_event, Trigger};
    use std::ffi::c_void;
    use std::sync::OnceLock;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IORegisterForSystemPower(
            refcon: *mut c_void,
            port: *mut *mut c_void,
            callback: extern "C" fn(*mut c_void, u32, u32, *mut c_void),
            notifier: *mut u32,
        ) -> u32;
        fn IONotificationPortGetRunLoopSource(port: *mut c_void) -> *mut c_void;
        fn IOAllowPowerChange(kernel_port: u32, notification_id: isize) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopGetCurrent() -> *mut c_void;
        fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
        fn CFRunLoopRun();
    }

    /// The root power domain's connection (`IORegisterForSystemPower`).
    static ROOT: OnceLock<u32> = OnceLock::new();
    static TRIGGER: OnceLock<Trigger> = OnceLock::new();

    /// `kIOMessageCanSystemSleep` and `kIOMessageSystemWillSleep` need an
    /// answer; the others don't.
    fn needs_answer(message: u32) -> bool {
        matches!(message, 0xE000_0270 | 0xE000_0280)
    }

    extern "C" fn on_message(
        _refcon: *mut c_void,
        _service: u32,
        message: u32,
        argument: *mut c_void,
    ) {
        if let (Some(event), Some(trigger)) = (power_event(message), TRIGGER.get()) {
            // A panic must never keep the Mac from sleeping.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                trigger.on_power(event);
            }));
        }
        if needs_answer(message) {
            if let Some(root) = ROOT.get() {
                // SAFETY: the connection IOKit gave this thread, and the
                // message's own notification id.
                unsafe {
                    IOAllowPowerChange(*root, argument as isize);
                }
            }
        }
    }

    pub(super) fn listen(trigger: Trigger) {
        if TRIGGER.set(trigger).is_err() {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("hollow-power".into())
            .spawn(|| {
                let mut port: *mut c_void = std::ptr::null_mut();
                let mut notifier: u32 = 0;
                // SAFETY: IOKit's documented registration; the port and
                // notifier live for the thread, which runs for the process.
                unsafe {
                    let root = IORegisterForSystemPower(
                        std::ptr::null_mut(),
                        &mut port,
                        on_message,
                        &mut notifier,
                    );
                    if root == 0 || port.is_null() {
                        crate::launch::elog("[chat move] system power notices unavailable");
                        return;
                    }
                    let _ = ROOT.set(root);
                    let source = IONotificationPortGetRunLoopSource(port);
                    CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode);
                    CFRunLoopRun();
                }
            });
        if let Err(error) = spawned {
            crate::launch::elog(&format!("[chat move] power thread: {error}"));
        }
    }
}

#[cfg(test)]
#[path = "chat_move_trigger_tests.rs"]
mod tests;
