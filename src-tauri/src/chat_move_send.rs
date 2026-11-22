//! M5 PR2 (2026-11-15): the sending side of a chat's move to another Mac.
//!
//! [`chat_move`] keeps who owns a chat and checks every step's proof; this
//! module drives a move over the peer link (M2's tunnel and client): it
//! marks the chat `leaving` ([`chat_move::begin_move_out`]), sends the
//! bundle in parts, commits, and then finishes or calls the move off only on
//! what the other Mac actually answered. Design: the M5 plan
//! (`outputs/hollow-build-run-2026-11-14/m5-plan.md` in the Hollow vault
//! workspace, sections 2.4 to 2.6); wire: `docs/phone-api/machines.md`.
//!
//! **Proof comes from the wire only.** A [`TargetSaid`] is made here and
//! nowhere else, from a [`MoveAnswer`] the link received for that move id
//! ([`commit_refusal`], [`delete_said`]), and passed with the link's own
//! [`MoveLink::peer`], the paired record's `machineId` the client called
//! (`PeerClient::machine` refuses a Mac answering as another one). A
//! timeout, a transport error, a 5xx or a status of `unknown` is no answer:
//! the move's `DELETE` goes first (its tombstone refuses a late commit), and
//! when that gets no answer either the chat stays `leaving`, running nowhere,
//! until the next contact ([`retry_leaving`], [`settle_pending`]).
//!
//! **Never two owners, never none.** Every outcome is one of [`Sent`]: the
//! other Mac imported it (`Moved`, the chat `away` here), the other Mac said
//! nothing landed (`Home`, the chat runs here again), or no answer settled
//! it (`Waiting`, the chat `leaving` or `aborting`, running on neither until
//! one does).

use crate::chat_move::{self, Cancelled, MoveError, TargetSaid};
use crate::peer_client::{Accepted, MoveAnswer, MoveRoute, PeerClient, PeerError};
use chrono::Utc;
use serde_json::json;
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// A boxed future, for [`MoveLink`]'s calls.
pub(crate) type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The commit refusals that prove nothing landed (the contract on
/// [`TargetSaid`]): any other answer to a commit is no proof. Not
/// `bad_request`: the staging area answers it when parts are missing, which
/// a second delivery of the same move can cause while the first one's
/// commit lands.
const COMMIT_REFUSALS: [&str; 4] = ["conflict", "not_here", "too_large", "cancelled"];

/// How this Mac reaches the other one for a move: the paired Mac's client
/// in the app ([`PeerLink`]); tests put a stand-in around one to lose an
/// answer.
pub(crate) trait MoveLink: Send + Sync {
    /// The `machineId` of the paired Mac this link calls: every answer it
    /// brings back is that Mac's.
    fn peer(&self) -> &str;
    /// One move route, its answer as received.
    fn call<'a>(&'a self, route: MoveRoute<'a>) -> Fut<'a, Result<MoveAnswer, PeerError>>;
    /// `POST /v1/sessions/{key}/messages` there (the continuation).
    fn send<'a>(
        &'a self,
        key: &'a str,
        text: &'a str,
        message_id: &'a str,
    ) -> Fut<'a, Result<Accepted, PeerError>>;
}

/// A paired Mac's client as a [`MoveLink`].
pub(crate) struct PeerLink {
    client: Arc<PeerClient>,
    peer: String,
}

impl PeerLink {
    /// The link for a paired Mac's client; `None` for a client with no
    /// paired record (it couldn't say whose answers it brings).
    pub(crate) fn new(client: Arc<PeerClient>) -> Option<Self> {
        let peer = client.machine_id()?.to_string();
        Some(Self { client, peer })
    }

    pub(crate) fn client(&self) -> &PeerClient {
        &self.client
    }
}

impl MoveLink for PeerLink {
    fn peer(&self) -> &str {
        &self.peer
    }
    fn call<'a>(&'a self, route: MoveRoute<'a>) -> Fut<'a, Result<MoveAnswer, PeerError>> {
        Box::pin(async move { self.client.move_call(&route).await })
    }
    fn send<'a>(
        &'a self,
        key: &'a str,
        text: &'a str,
        message_id: &'a str,
    ) -> Fut<'a, Result<Accepted, PeerError>> {
        Box::pin(async move { self.client.send(key, text, message_id).await })
    }
}

/// How a move that was begun stands now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Sent {
    /// The other Mac imported the chat: it lives there now (`away` here).
    Moved,
    /// The other Mac said nothing of it landed: the chat runs here again.
    Home(String),
    /// No answer settled it: the chat runs on neither Mac until the other
    /// one answers (the next contact sends again, or its `DELETE`).
    Waiting(String),
    /// Another call of this process is driving the move right now
    /// ([`Driving`]); nothing was done. That call settles it.
    Busy,
}

/// A move this process is driving now (sending, asking, calling off): one
/// delivery per move id at a time, process-wide, so two deliveries can't
/// race (a second one's parts could make the first one's commit look
/// refused while it lands). Released when dropped.
pub(crate) struct Driving(String);

fn driving() -> &'static Mutex<HashSet<String>> {
    static DRIVING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DRIVING.get_or_init(Default::default)
}

impl Driving {
    /// Claim move `move_id`; `None` while another call drives it.
    pub(crate) fn claim(move_id: &str) -> Option<Self> {
        let mut held = driving().lock().unwrap_or_else(PoisonError::into_inner);
        held.insert(move_id.to_string())
            .then(|| Self(move_id.to_string()))
    }
}

impl Drop for Driving {
    fn drop(&mut self) {
        driving()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

/// Run a Sessions DB call off the async threads.
async fn off_thread<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, MoveError> + Send + 'static,
) -> Result<T, MoveError> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(MoveError::from(format!("chat move: {e}"))))
}

/// Whether `answer` to this move's commit proves nothing landed: a 4xx with
/// one of [`COMMIT_REFUSALS`] (never a 404, a 403 or a 409 `busy`).
pub(crate) fn commit_refusal(answer: &MoveAnswer) -> Option<TargetSaid> {
    let refused = (400..500).contains(&answer.status)
        && answer
            .code
            .as_deref()
            .is_some_and(|code| COMMIT_REFUSALS.contains(&code));
    refused.then_some(TargetSaid::CommitRefused)
}

/// What `answer` to this move's `DELETE` proves: 200 `{"cancelled": ...}`
/// (nothing of the move is there now) or 409 `started` (a turn ran there,
/// the move stands). Anything else is no answer.
pub(crate) fn delete_said(answer: &MoveAnswer) -> Option<TargetSaid> {
    match (answer.status, answer.code.as_deref()) {
        (200, _) => {
            let cancelled = match answer.body["cancelled"].as_str()? {
                "notImported" => Cancelled::NotImported,
                "removed" => Cancelled::Removed,
                "already" => Cancelled::Already,
                _ => return None,
            };
            Some(TargetSaid::Cancelled(cancelled))
        }
        (409, Some("started")) => Some(TargetSaid::Kept),
        _ => None,
    }
}

/// Move chat `key` from this Mac (`from`, its `machineId`) to the Mac
/// `link` calls, as move `move_id`: mark it `leaving` and send it. A
/// refusal before anything was sent changes nothing (`Err`). Called again
/// with the id of a move still `leaving`, it sends the same chat again (the
/// other Mac answers a commit that already landed with `already`).
pub(crate) async fn send_move(
    vault: &Path,
    link: &dyn MoveLink,
    key: &str,
    from: &str,
    move_id: &str,
) -> Result<Sent, MoveError> {
    send_move_within(vault, link, key, from, move_id, chat_move::Caps::SYNC).await
}

/// [`send_move`] under `caps`: a chat longer than one sync carries goes in
/// slices ([`chat_move::begin_move_out_sliced`]), each earlier slice sent and
/// landed before the next is asked for, the move's own bundle last. A slice
/// the other Mac never answered is sent again under its id on the next call
/// (progress is in the sending Mac's database); one it refused calls the
/// whole move off, and its `DELETE` clears the slices waiting there.
pub(crate) async fn send_move_within(
    vault: &Path,
    link: &dyn MoveLink,
    key: &str,
    from: &str,
    move_id: &str,
    caps: chat_move::Caps,
) -> Result<Sent, MoveError> {
    let Some(_driving) = Driving::claim(move_id) else {
        return Ok(Sent::Busy);
    };
    for round in 0.. {
        let (v, k, f, to, id) = (
            vault.to_path_buf(),
            key.to_string(),
            from.to_string(),
            link.peer().to_string(),
            move_id.to_string(),
        );
        let begun = off_thread(move || {
            chat_move::begin_move_out_sliced(&v, &k, &f, &to, &id, caps, Utc::now())
        })
        .await;
        let (_, bytes, next) = match begun {
            Ok(begun) => begun,
            // Slices are out there: a refusal now calls the move off.
            Err(error) if round > 0 && error.code != "internal" => {
                return Ok(recover(vault, link, move_id, error.message).await)
            }
            Err(error) => return Err(error),
        };
        let slice_id = match next {
            chat_move::Next::Move => return Ok(deliver(vault, link, move_id, &bytes).await),
            chat_move::Next::Slice(slice_id) => slice_id,
        };
        match deliver_slice(link, &slice_id, &bytes).await {
            SliceSent::Landed => {
                let (v, id) = (vault.to_path_buf(), slice_id);
                if let Err(error) =
                    off_thread(move || chat_move::slice_landed(&v, &id, Utc::now())).await
                {
                    return Ok(Sent::Waiting(error.message));
                }
            }
            SliceSent::NoAnswer(why) => return Ok(Sent::Waiting(why)),
            SliceSent::Refused(why) => return Ok(recover(vault, link, move_id, why).await),
        }
    }
    unreachable!("the loop ends in a return")
}

/// How an earlier slice's delivery ended.
enum SliceSent {
    /// Its commit was answered 200: it waits on the other Mac.
    Landed,
    /// No answer, or one that says try again (busy, parts gone, a failure
    /// there): the move waits and the slice goes again.
    NoAnswer(String),
    /// A refusal that will come again (a conflict, another version, a move
    /// called off there): the move is called off.
    Refused(String),
}

/// Send an earlier slice's parts and commit under its own id.
async fn deliver_slice(link: &dyn MoveLink, slice_id: &str, bytes: &[u8]) -> SliceSent {
    let said = |answer: &MoveAnswer| {
        let refused = (400..500).contains(&answer.status)
            && answer.status != 404
            && answer.code.as_deref() != Some("busy");
        if refused {
            SliceSent::Refused(answer.message.clone())
        } else {
            SliceSent::NoAnswer(answer.message.clone())
        }
    };
    let parts = chat_move::part_bodies(bytes);
    for (n, body) in parts.iter().enumerate() {
        let route = MoveRoute::Part {
            move_id: slice_id,
            n,
            body,
        };
        match link.call(route).await {
            Ok(answer) if answer.status == 200 => {}
            Ok(answer) => return said(&answer),
            Err(error) => return SliceSent::NoAnswer(error.message().into()),
        }
    }
    let commit = json!({"parts": parts.len(), "sha256": chat_move::sha256_hex(bytes)});
    let route = MoveRoute::Commit {
        move_id: slice_id,
        body: &commit,
    };
    match link.call(route).await {
        Ok(answer) if answer.status == 200 => SliceSent::Landed,
        Ok(answer) => said(&answer),
        Err(error) => SliceSent::NoAnswer(error.message().into()),
    }
}

/// Send the parts and the commit; settle on the answer.
async fn deliver(vault: &Path, link: &dyn MoveLink, move_id: &str, bytes: &[u8]) -> Sent {
    let parts = chat_move::part_bodies(bytes);
    for (n, body) in parts.iter().enumerate() {
        let route = MoveRoute::Part { move_id, n, body };
        match link.call(route).await {
            Ok(answer) if answer.status == 200 => {}
            // A part refused or lost: nothing was committed, but parts may
            // be staged there. Ask, then call it off.
            Ok(answer) => return recover(vault, link, move_id, answer.message).await,
            Err(error) => return recover(vault, link, move_id, error.message().into()).await,
        }
    }
    let commit = json!({"parts": parts.len(), "sha256": chat_move::sha256_hex(bytes)});
    let route = MoveRoute::Commit {
        move_id,
        body: &commit,
    };
    match link.call(route).await {
        Ok(answer) if answer.status == 200 => finish(vault, move_id).await,
        Ok(answer) => match commit_refusal(&answer) {
            Some(said) => {
                let refusal = answer.message.clone();
                let sent = settle(vault, link, move_id, said, false, answer.message).await;
                // PR3: an offline sync refused for good (that Mac's chat is
                // gone or isn't the one this Mac knows; too large) stays
                // this Mac's chat; a tombstone (`cancelled`) is sent again.
                let definitive = matches!(answer.code.as_deref(), Some("conflict" | "too_large"));
                if definitive && matches!(sent, Sent::Home(_)) {
                    let (v, id, note) = (vault.to_path_buf(), move_id.to_string(), refusal.clone());
                    let kept = off_thread(move || {
                        crate::chat_offline::keep_move(&v, &id, &note, Utc::now())
                    })
                    .await;
                    if let Err(error) = kept {
                        crate::launch::elog(&format!(
                            "[chat offline] {move_id}: {}",
                            error.message
                        ));
                    }
                }
                sent
            }
            None => recover(vault, link, move_id, answer.message).await,
        },
        Err(error) => recover(vault, link, move_id, error.message().into()).await,
    }
}

/// The other Mac said the move landed: the chat is `away` here.
async fn finish(vault: &Path, move_id: &str) -> Sent {
    let (v, id) = (vault.to_path_buf(), move_id.to_string());
    match off_thread(move || chat_move::finish_move_out(&v, &id, Utc::now())).await {
        Ok(()) => Sent::Moved,
        Err(error) => Sent::Waiting(error.message),
    }
}

/// No answer to a part or the commit: ask whether it landed; if it did,
/// finish; if not (or the answer says nothing), call it off.
async fn recover(vault: &Path, link: &dyn MoveLink, move_id: &str, why: String) -> Sent {
    match link.call(MoveRoute::Status { move_id }).await {
        Ok(answer) if answer.status == 200 && answer.body["state"] == "imported" => {
            finish(vault, move_id).await
        }
        // `receiving`, `unknown`, `cancelled` or a refusal: never proof.
        Ok(_) => call_off(vault, link, move_id, false, why).await,
        Err(error) => Sent::Waiting(error.message().into()),
    }
}

/// Send the move's `DELETE` and settle on its answer. `aborting`: the move
/// is being called off already ([`chat_move::begin_abort`]).
async fn call_off(
    vault: &Path,
    link: &dyn MoveLink,
    move_id: &str,
    aborting: bool,
    why: String,
) -> Sent {
    match link.call(MoveRoute::Cancel { move_id }).await {
        Ok(answer) => match delete_said(&answer) {
            Some(said) => settle(vault, link, move_id, said, aborting, why).await,
            None => Sent::Waiting(answer.message),
        },
        Err(error) => Sent::Waiting(error.message().into()),
    }
}

/// Record the other Mac's answer `said` for move `move_id`.
async fn settle(
    vault: &Path,
    link: &dyn MoveLink,
    move_id: &str,
    said: TargetSaid,
    aborting: bool,
    why: String,
) -> Sent {
    let (v, id, peer) = (
        vault.to_path_buf(),
        move_id.to_string(),
        link.peer().to_string(),
    );
    let done = off_thread(move || {
        if aborting {
            chat_move::settle_abort(&v, &id, &peer, said, Utc::now())
        } else {
            chat_move::abort_move_out(&v, &id, &peer, said, Utc::now())
        }
    })
    .await;
    match (done, said) {
        (Ok(()), TargetSaid::Kept) => Sent::Moved,
        (Ok(()), _) => Sent::Home(why),
        (Err(error), _) => Sent::Waiting(error.message),
    }
}

/// Keep it here: call move `move_id` off. With the other Mac reachable
/// (`link`), its `DELETE` settles it at once; without, the move waits
/// `aborting` (the chat still runs nowhere) for [`settle_pending`].
pub(crate) async fn keep_here(vault: &Path, link: Option<&dyn MoveLink>, move_id: &str) -> Sent {
    let (v, id) = (vault.to_path_buf(), move_id.to_string());
    match off_thread(move || chat_move::begin_abort(&v, &id, Utc::now())).await {
        Ok(chat_move::Aborting::Aborted) => {
            return Sent::Home("The move was already called off.".into())
        }
        Ok(chat_move::Aborting::Waiting) => {}
        Err(error) => return Sent::Waiting(error.message),
    }
    // A delivery under way settles the move itself (a commit that lands
    // finishes it, anything else leaves it aborting for the next contact).
    let Some(_driving) = Driving::claim(move_id) else {
        return Sent::Busy;
    };
    match link {
        Some(link) => call_off(vault, link, move_id, true, "Kept on this Mac.".into()).await,
        None => Sent::Waiting("The other Mac hasn't answered yet.".into()),
    }
}

/// On contact with `link`'s Mac: send the `DELETE` of every move to it
/// that was called off while it was out of reach, and settle each on its
/// answer. `(move_id, key, how it stands)`.
pub(crate) async fn settle_pending(
    vault: &Path,
    link: &dyn MoveLink,
) -> Vec<(String, String, Sent)> {
    let v = vault.to_path_buf();
    let pending = off_thread(move || chat_move::pending_aborts(&v).map_err(MoveError::from))
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for (move_id, key, peer) in pending {
        if peer != link.peer() {
            continue;
        }
        let Some(_driving) = Driving::claim(&move_id) else {
            continue;
        };
        let sent = call_off(vault, link, &move_id, true, "Kept on this Mac.".into()).await;
        out.push((move_id, key, sent));
    }
    out
}

/// On contact with `link`'s Mac: send every move to it that is still
/// `leaving` again, under its own id. `(move_id, key, how it stands)`.
pub(crate) async fn retry_leaving(
    vault: &Path,
    link: &dyn MoveLink,
    from: &str,
) -> Vec<(String, String, Sent)> {
    let v = vault.to_path_buf();
    let leaving = off_thread(move || chat_move::leaving_moves(&v).map_err(MoveError::from))
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for (move_id, key, peer) in leaving {
        if peer != link.peer() {
            continue;
        }
        let sent = match send_move(vault, link, &key, from, &move_id).await {
            Ok(sent) => sent,
            // A failure on this Mac may pass (the database busy): try again.
            Err(error) if error.code == "internal" => Sent::Waiting(error.message),
            // Refused before sending again (its folder gone, too large):
            // settle it with that Mac's answer, so it never waits forever.
            Err(error) => match Driving::claim(&move_id) {
                Some(_driving) => recover(vault, link, &move_id, error.message).await,
                None => Sent::Busy,
            },
        };
        out.push((move_id, key, sent));
    }
    out
}

/// The continuation's words: the person's own send at the chat's new home.
pub(crate) fn continuation_text(from_name: &str) -> String {
    format!(
        "This chat moved here from {from_name} while a turn was running. Continue from where it stopped."
    )
}

/// The continuation's `clientMessageId`, from the move's id, so a retried
/// continuation runs once.
pub(crate) fn continuation_id(move_id: &str) -> String {
    format!("moved-{move_id}")
}

/// Send the moved chat its one continuation turn at its new home.
pub(crate) async fn continue_there(
    link: &dyn MoveLink,
    key: &str,
    move_id: &str,
    from_name: &str,
) -> Result<Accepted, PeerError> {
    let text = continuation_text(from_name);
    let id = continuation_id(move_id);
    link.send(key, &text, &id).await
}

/// A move answer for tests and stand-ins: `status` with `body`.
#[cfg(test)]
pub(crate) fn answer(status: u16, body: serde_json::Value) -> MoveAnswer {
    MoveAnswer {
        status,
        code: body["error"]["code"].as_str().map(str::to_string),
        message: format!("answered {status}"),
        body,
    }
}

#[cfg(test)]
#[path = "chat_move_send_tests.rs"]
mod tests;
