//! The phone API's routes (`docs/PHONE_CHAT_API.md`), built on Hollow's own
//! reads and the composer's send path.
//! v1.2 Send settings follow the chat's inheritance rules (controls.rs).
//! Only effort is one-send; global preferences and saved window modes stay intact.
//!
//! - **Lists and reads** come from the session store
//!   (`list_sessions_after`, `read_conversation`, `last_permission`), the
//!   run queue (queued and running, as the window reads it), question cards
//!   (`interactions`) and the workspace list, as Hollow's own lists show
//!   them.
//! - **A send** is the person's composer send for that chat
//!   (`commands::send_unless_busy`, which is `send_chat` when the chat is
//!   idle): its agent, the model its newest turn asked for
//!   (`wake_me::wake_model`), its folder, the permission mode the window
//!   shows for it ([`chat_permission`]: the person's choice for the chat,
//!   which the window reports, else its last turn's) and its native session
//!   unless that mode is of another class than the session's, at depth 0,
//!   typed, after the composer's intent line. A chat with a turn queued or
//!   running, or a message the window holds for after one, is `busy` (409):
//!   a phone's send is never held, since a held send waits where only the
//!   window that sent it shows it.
//! - **A new chat** is the composer's New chat in that workspace: its folder,
//!   the agent asked for (or the default) with that agent's saved model, and
//!   the mode Settings gives new chats.
//! - **Never twice.** A send and a new chat run in their own task from the
//!   repeat check to the remembered answer ([`to_the_end`]), so a phone that
//!   hangs up halfway can't leave a queued turn without its answer, and its
//!   retry with the same `clientMessageId` gets the first answer.
//! - **Stop** is the window's Stop button (`chat::cancel`).
//! - **Another Mac** (M2, `docs/phone-api/machines.md`): a machine token
//!   reaches only the routes [`ROUTES`] gives it (default deny), and a phone
//!   token never reaches a machine's.

use super::approvals_api;
use super::attachments;
use super::calendar_api;
use super::canvases;
use super::live::{self, Live, Lookup, RowDigest, TurnRecord};
use super::live_activity::Activities;
use super::notes_api;
use super::search_api;
use super::store::{Attempt, Device, DeviceKind, NotLive, Pairing, Store};
use super::tasks_api;
use super::wire;
use super::workspaces_api;
use crate::commands::ChatSendInput;
use crate::types::{AgentSessionSummary, ChatInteraction, SendAccepted};
use hyper::body::Bytes;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A request body, at most (413 past it). The two upload routes ([`is_upload`])
/// take a file instead, up to [`attachments::FILE_MAX`].
pub(crate) const BODY_MAX: usize = 256 * 1024;

/// v1.2: whether `method path` is an upload, `POST /v1/sessions/{key}/attachments`
/// or `POST /v1/workspaces/{id}/attachments`, whose body is a file rather than
/// JSON (`docs/phone-api/attachments.md`).
pub(crate) fn is_upload(method: &str, path: &str) -> bool {
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').skip(1).collect();
    method == "POST"
        && matches!(
            parts.as_slice(),
            ["v1", "sessions" | "workspaces", _, "attachments"]
        )
}
/// A message, at most this many characters after trimming (413 past it).
pub(crate) const TEXT_MAX: usize = 100_000;
/// A stored turn's text, at most this many characters (`truncated` past it).
pub(crate) const TURN_TEXT_MAX: usize = 200_000;
/// A list preview, at most this many characters.
pub(crate) const PREVIEW_MAX: usize = 160;
/// A repeated `clientMessageId` from the same phone within this long gets
/// the first answer again and queues nothing.
pub(crate) const IDEMPOTENT_FOR: Duration = Duration::from_secs(10 * 60);
/// Answers kept for repeats, at most.
const IDEMPOTENT_KEPT: usize = 1_000;
/// How long a send waits to report a turn that starts at once as running.
const START_GRACE: Duration = Duration::from_millis(400);
/// The workspace list is read again after this long.
const HOMES_FOR: Duration = Duration::from_secs(5);
/// Workspaces `GET /v1/workspaces` offers, at most (built-ins included).
const WORKSPACES_MAX: usize = 100;

/// What the composer puts before the person's words in every send
/// (`composePayload` in `src/components/chat/chatSendController.ts`; a test
/// holds the two together).
pub(crate) const CHAT_INTENT: &str = "<hollow-chat-intent>\nInfer the appropriate approach from the user's current request: answer, act, review, or automate as needed. Adapt as the conversation changes. Previous UI mode selections are historical and do not constrain this request. Respect configured permissions and confirmation requirements.\n</hollow-chat-intent>\n\n";

/// Hollow's fences a list preview leaves out.
const PREVIEW_FENCES: [&str; 4] = [
    crate::canvas_fence::CANVAS_FENCE,
    crate::next_step::FENCE_LANG,
    crate::next_step::CANVAS_ANSWER_LANG,
    crate::interaction_schema::FENCE_LANG,
];

/// The busy answer (fixture `error-busy.json`).
pub(crate) const BUSY_MESSAGE: &str =
    "Hollow is still answering in this chat. Send again when the turn ends, or stop it.";
/// The closed card answer (fixture `error-question-closed.json`).
pub(crate) const QUESTION_CLOSED_MESSAGE: &str =
    "This question card is no longer open. Reply in the chat instead.";
/// `GET /v1/waiting` lists at most this many chats.
pub(crate) const WAITING_MAX: usize = 100;
/// Where a chat runs, as the phone names it (v1.1). M6 (2026-11-15): a
/// request may name the runner `runner` or `mac-mini`; answers keep saying
/// `mac-mini`, the value the shipped iPhone app reads, until a wire version
/// that says `runner` is agreed with it.
pub(crate) const THIS_MAC: &str = "this-mac";
pub(crate) const MAC_MINI: &str = crate::agent_settings::LEGACY_RUNNER;

pub(crate) type Fut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// What a route asks of the running Hollow. The app's is [`AppHost`]; tests
/// stand in their own.
pub(crate) trait Host: Send + Sync + 'static {
    fn control_options(
        &self,
        agent: Option<crate::agent::AgentKind>,
    ) -> Fut<'_, Result<super::controls::Options, ApiError>>;
    fn send_controlled(
        &self,
        input: ChatSendInput,
        effort: Option<String>,
        existing: bool,
        gate: Gate,
    ) -> Fut<'_, Result<SendAccepted, String>>;
    /// Whether this process takes turns now (`run_pump::accepting`).
    fn accepting(&self) -> Result<(), String>;
    /// `chat_send` for a chat that may be busy, never held: sent now when it
    /// has no turn queued or running and the window holds nothing for after
    /// one, else refused as already queued (`commands::send_unless_busy`).
    /// `gate` is the phone's commit-point check (R1), run once the send holds
    /// the held-send order and before it queues anything.
    fn send_unless_busy(
        &self,
        input: ChatSendInput,
        gate: Gate,
    ) -> Fut<'_, Result<SendAccepted, String>>;
    /// The Stop button: whether anything was stopped.
    fn stop(&self, key: String) -> Fut<'_, Result<bool, String>>;
    /// The agent a new chat runs: the one asked for, if installed, else
    /// Hollow's default (`agent_settings::resolve`).
    fn agent_for(
        &self,
        asked: Option<crate::agent::AgentKind>,
    ) -> Result<crate::agent::AgentKind, String>;
    /// Tell the window a phone queued a turn (`app-session-started`: the
    /// workspace joins the sidebar, the lists refresh).
    fn announce(&self, key: &str, workspace_id: Option<&str>, cwd: &str);
    /// Settings shows something new (a phone paired).
    fn changed(&self);
    /// Ask Hollow's window for its whole record of mode choices
    /// ([`MODES_WANTED`]); it answers through `phone_access_chat_modes` with
    /// `replace` ([`WindowModes::heard_whole`]).
    fn want_modes(&self);
    /// v1.1: the remote runner set up on this Mac and its last known state
    /// (`remote_status`), `None` when none is set up. Never waits for a
    /// check.
    fn runner(&self) -> Option<wire::Runner>;
    /// v1.1: what placing a turn reads (Settings, Devices and this Mac's
    /// runner), for Runs on (`remote_session::PlaceReads`).
    fn place(&self) -> crate::remote_session::PlaceReads;
    /// v1.1: a question card changed outside the window (a phone answered
    /// it, or its answer came back): the window shows it as its own Send
    /// would (`interaction` chat event).
    fn card_changed(&self, card: &ChatInteraction);
    /// A2: this Hollow's accounts for `GET /v1/machine/usage`
    /// (`peer_usage::local_accounts`). Never a token.
    fn machine_usage(&self) -> Vec<wire::AccountUsage> {
        crate::peer_usage::local_accounts(chrono::Utc::now())
    }
    /// H4b: each task's `source_label` for `GET /v1/tasks`, from this
    /// app's saved settings (`sources_api::label_tasks_for`), so the phone
    /// says what the desktop says. Test hosts leave tasks unlabeled.
    fn label_tasks(&self, _tasks: &mut [crate::types::Task]) {}
}

// ── The window's modes ───────────────────────────────────────────────────

/// The event that asks Hollow's main window for its whole record of mode
/// choices (`reportPhoneModes` in `src/store/chatRuntime.ts`).
pub(crate) const MODES_WANTED: &str = "phone-access-modes-wanted";
/// How long a phone's request waits for the window's answer: the request
/// that starts a round of asks ([`ASK_FOR`]), and any in this first part of
/// the round.
pub(crate) const WINDOW_WAIT: Duration = Duration::from_secs(2);
/// Within a round the window is asked again this often, so one whose
/// listener registers at any time in it (the first seconds after a slow
/// launch) hears an ask within this long.
pub(crate) const ASK_EVERY: Duration = Duration::from_millis(250);
/// A round of asks lasts this long unless the window answers, and the first
/// request after it starts another: a silent window holds one request a
/// wait this often at most.
pub(crate) const ASK_FOR: Duration = Duration::from_secs(30);

/// How long [`WindowModes`] waits and asks: [`WINDOW_WAIT`], [`ASK_EVERY`]
/// and [`ASK_FOR`] (tests shorten them).
#[derive(Clone, Copy, Debug)]
pub(crate) struct AskTiming {
    pub wait: Duration,
    pub every: Duration,
    pub round: Duration,
}

impl Default for AskTiming {
    fn default() -> Self {
        Self {
            wait: WINDOW_WAIT,
            every: ASK_EVERY,
            round: ASK_FOR,
        }
    }
}

/// The permission modes Hollow's window reported, beyond the record: the
/// mode for new chats, and whether the window's whole record of chat mode
/// choices reached this process. The record keeps the choices across
/// restarts, but the window can lose its own (cleared web storage, or its
/// Plan fallback after a full storage, second review 2026-11-04), so after
/// a launch the copy counts only once the window has sent it whole again:
/// a phone request that needs it asks, in rounds, until the window answers
/// ([`Self::settle`]), no launch call.
pub(crate) struct WindowModes {
    /// The permission mode Settings gives new chats and the agent it is for,
    /// as the window last said, kept in the record across restarts (none
    /// until the window has said one). One value under one lock (Q-R3 round
    /// 3), so a phone request never pairs one report's mode with another's
    /// agent: read it with [`Self::new_chat`], set it with
    /// [`Self::report_new_chat`] or [`Self::restore_new_chat`].
    new_chat: Mutex<Option<NewChatMode>>,
    fresh: std::sync::atomic::AtomicBool,
    /// Bumped by each whole record.
    whole: tokio::sync::watch::Sender<u64>,
    /// When the round of asks under way began, while the window hasn't
    /// answered. Cleared when the copy falls behind, so the next request
    /// starts a round.
    round: Mutex<Option<Instant>>,
    timing: AskTiming,
    /// False in `hollow serve` ([`Self::without_window`]): no window to
    /// ask, and the record is the whole truth.
    window: bool,
}

/// The window's new-chat mode and the agent it is for (its preferred
/// agent; None when the report didn't say, an older window or record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewChatMode {
    pub mode: String,
    pub agent: Option<String>,
}

impl Default for WindowModes {
    fn default() -> Self {
        Self::timed(AskTiming::default())
    }
}

impl WindowModes {
    /// With its own [`AskTiming`] (Hollow's is the default).
    pub(crate) fn timed(timing: AskTiming) -> Self {
        Self {
            new_chat: Mutex::new(None),
            fresh: std::sync::atomic::AtomicBool::new(false),
            whole: tokio::sync::watch::channel(0).0,
            round: Mutex::new(None),
            timing,
            window: true,
        }
    }

    /// `hollow serve` (M1 PR 3): there is no window, so nothing is ever
    /// asked or waited for. A new chat takes the record's stored mode for
    /// new chats (`hollow serve set-default-mode`), else Plan; each chat's
    /// own choice is the record's as always.
    pub(crate) fn without_window() -> Self {
        let modes = Self {
            window: false,
            ..Self::default()
        };
        modes.fresh.store(true, std::sync::atomic::Ordering::SeqCst);
        modes
    }

    /// Whether a window reports these modes (false in `hollow serve`).
    pub(crate) fn has_window(&self) -> bool {
        self.window
    }

    /// The new-chat mode and its agent, as one value.
    pub(crate) fn new_chat(&self) -> Option<NewChatMode> {
        self.new_chat
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The window reported its new-chat mode: mode and agent replace the
    /// last report's together.
    pub(crate) fn report_new_chat(&self, mode: NewChatMode) {
        *self.new_chat.lock().unwrap_or_else(|e| e.into_inner()) = Some(mode);
    }

    /// The record's copy at launch, both fields together, unless the window
    /// already said one in this process.
    pub(crate) fn restore_new_chat(&self, mode: NewChatMode) {
        self.new_chat
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert(mode);
    }

    /// The window's whole record arrived (and the record took it).
    pub(crate) fn heard_whole(&self) {
        self.fresh.store(true, std::sync::atomic::Ordering::SeqCst);
        self.whole.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Whether the window's whole record arrived in this process.
    pub(crate) fn fresh(&self) -> bool {
        self.fresh.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// A report went unwritten (the switch off and no phone paired), so the
    /// record lags the window: the next phone request starts a round of
    /// asks, and may wait.
    pub(crate) fn fell_behind(&self) {
        if !self.window {
            return;
        }
        self.fresh.store(false, std::sync::atomic::Ordering::SeqCst);
        *self.round.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Before a phone's request relies on the window's mode choices, unless
    /// the whole record arrived in this process. A request that finds no
    /// round of asks under way starts one: `ask` the window at once, then
    /// every [`ASK_EVERY`] for [`ASK_FOR`] or until it answers. That request
    /// and any in the round's first [`WINDOW_WAIT`] wait that long for the
    /// answer; a later one goes on with the record as it is, and the first
    /// request after the round starts another. So a silent window holds one
    /// request a wait per round, and a window whose listener registers at
    /// any time is heard within [`ASK_EVERY`] in a round, or by the next
    /// request's round after one. The review of the third review's fixes
    /// (2026-11-05): with one wait in all and only each request asking, the
    /// first request after a late listener registered used the old record,
    /// however late it came.
    pub(crate) async fn settle(self: &Arc<Self>, ask: impl Fn() + Send + Sync + 'static) {
        if self.fresh() {
            return;
        }
        let mut heard = self.whole.subscribe();
        heard.borrow_and_update();
        let now = Instant::now();
        let (began, starts) = {
            let mut round = self.round.lock().unwrap_or_else(|e| e.into_inner());
            match *round {
                Some(began) if now.duration_since(began) < self.timing.round => (began, false),
                _ => {
                    *round = Some(now);
                    (now, true)
                }
            }
        };
        if starts {
            ask();
            if !self.fresh() {
                self.keep_asking(began, ask);
            }
        }
        let until = began + self.timing.wait;
        if self.fresh() || now >= until {
            return;
        }
        let until = tokio::time::Instant::from_std(until);
        let _ = tokio::time::timeout_at(until, heard.changed()).await;
    }

    /// The rest of the round that began at `began`: `ask` every
    /// [`ASK_EVERY`] until the window answers, the round ends or a newer
    /// round takes over (the copy fell behind meanwhile).
    fn keep_asking(self: &Arc<Self>, began: Instant, ask: impl Fn() + Send + Sync + 'static) {
        let modes = Arc::downgrade(self);
        let (every, ends) = (self.timing.every, began + self.timing.round);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let Some(modes) = modes.upgrade() else {
                    return;
                };
                if modes.fresh() || !modes.in_round(began) || Instant::now() >= ends {
                    return;
                }
                ask();
            }
        });
    }

    /// Whether the round under way is the one that began at `began`.
    fn in_round(&self, began: Instant) -> bool {
        *self.round.lock().unwrap_or_else(|e| e.into_inner()) == Some(began)
    }
}

/// A new chat's mode in `hollow serve` when the record stores none.
pub(crate) const HEADLESS_DEFAULT_MODE: &str = "plan";

/// The record's stored mode for new chats and its agent, when it names one
/// of Hollow's modes.
fn stored_new_chat(store: &Store) -> Option<NewChatMode> {
    let record = store.read().ok()?;
    let mode = record.new_chat_mode.filter(|m| super::known_mode(m))?;
    Some(NewChatMode {
        mode,
        agent: record.new_chat_agent,
    })
}

// ── Errors and replies ───────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub attempts_left: Option<u32>,
}

/// An error message, at most this many characters.
const MESSAGE_MAX: usize = 400;

/// `text` as the contract's "one plain sentence the app may show as it is":
/// one line, at most [`MESSAGE_MAX`] characters, ending in a full stop. The
/// causes Hollow passes on (a refused send, an unreadable file) come from
/// its own internals, which may end in a colon, a question mark or nothing.
fn sentence(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = live::cut_chars(&line, MESSAGE_MAX - 1);
    let line = line.trim_end_matches(|c: char| {
        c.is_whitespace() || matches!(c, '.' | '!' | '?' | ':' | ';' | ',' | '-' | '\u{2026}')
    });
    if line.is_empty() {
        return "Hollow couldn't do that.".to_string();
    }
    format!("{line}.")
}

impl ApiError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: sentence(&message.into()),
            attempts_left: None,
        }
    }
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "bad_request", message)
    }
    pub(crate) fn unauthorized() -> Self {
        Self::new(
            401,
            "unauthorized",
            "This iPhone isn't paired with Hollow anymore. Pair it again in Hollow's Settings.",
        )
    }
    pub(crate) fn forbidden() -> Self {
        Self::new(
            403,
            "forbidden",
            "Only the Tailscale account signed in on this Mac can use Hollow.",
        )
    }
    /// Phone bearer requests may match or lower the chat/window/headless
    /// permission mode, never escalate above it.
    pub(crate) fn permission_escalation() -> Self {
        Self::new(
            403,
            "forbidden",
            "The iPhone can match or lower this chat's permission mode, not raise it. Change the permission mode on your Mac first.",
        )
    }
    /// Traffic Tailscale Funnel passed on from the internet.
    pub(crate) fn funnel() -> Self {
        Self::new(
            403,
            "forbidden",
            "Hollow answers only on your tailnet, never over Tailscale Funnel.",
        )
    }
    pub(crate) fn pairing_closed() -> Self {
        Self::new(
            403,
            "pairing_closed",
            "Pairing isn't open. In Hollow on your Mac, open Settings, Devices, Companion devices, and choose Pair an iPhone.",
        )
    }
    /// B4: no approvals code window is open for this phone.
    pub(crate) fn approvals_closed() -> Self {
        Self::new(
            403,
            "approvals_closed",
            "Approvals aren't open for this iPhone. In Hollow on your Mac, open Settings, Devices, Companion devices, and choose Approvals from this iPhone.",
        )
    }
    /// B4: this phone has no approval keys (turn approvals on first).
    pub(crate) fn approval_key_required() -> Self {
        Self::new(
            403,
            "approval_key_required",
            "This iPhone isn't set up to approve from the phone. Turn on approvals in the app's Settings.",
        )
    }
    /// B4: the decision's challenge isn't this phone's open one for this card.
    pub(crate) fn approval_expired() -> Self {
        Self::new(
            409,
            "approval_expired",
            "That step took too long. Ask for a new challenge and try again.",
        )
    }
    /// B4: the card was answered, timed out or cancelled already.
    pub(crate) fn approval_closed() -> Self {
        Self::new(
            409,
            "approval_closed",
            "That card was already answered, or its wait ended.",
        )
    }
    /// B4: the card's text changed since the phone showed it.
    pub(crate) fn approval_changed() -> Self {
        Self::new(
            409,
            "approval_changed",
            "The card changed. Read it again before you answer.",
        )
    }
    /// B4: the signature doesn't match the phone's key.
    pub(crate) fn bad_signature() -> Self {
        Self::new(
            400,
            "bad_signature",
            "That answer wasn't signed by this iPhone's approval key.",
        )
    }
    /// B4: an enrollment challenge that ended, was used or isn't this phone's.
    pub(crate) fn challenge_expired() -> Self {
        Self::new(
            409,
            "challenge_expired",
            "That step took too long. Ask for a new challenge and try again.",
        )
    }
    /// B4: the App Attest attestation didn't check out (`why` is a fixed reason).
    pub(crate) fn bad_attestation(why: &'static str) -> Self {
        Self::new(
            400,
            "bad_attestation",
            format!("Apple's attestation for this iPhone was refused: {why}."),
        )
    }
    pub(crate) fn wrong_code(attempts_left: u32) -> Self {
        Self {
            attempts_left: Some(attempts_left),
            ..Self::new(
                403,
                "wrong_code",
                "That code doesn't match. Check the code Hollow shows and try again.",
            )
        }
    }
    /// M2: a machine token on a route machines may not use (fixture
    /// `machines/error-machine-forbidden.json`).
    pub(crate) fn not_for_machines() -> Self {
        Self::new(
            403,
            "forbidden",
            "Another Mac can't use this through Hollow's link.",
        )
    }
    /// M2: a phone token on a route only another Mac uses.
    pub(crate) fn machines_only() -> Self {
        Self::new(
            403,
            "forbidden",
            "Only another Mac paired with Hollow uses this, not an iPhone.",
        )
    }
    /// M2: machine pairing or a machine token from anywhere but this Mac's
    /// own loopback (an SSH tunnel lands there; `tailscale serve` doesn't).
    pub(crate) fn loopback_only() -> Self {
        Self::new(
            403,
            "forbidden",
            "Another Mac reaches Hollow only through its SSH link, never over the tailnet.",
        )
    }
    /// M2: no machine pairing window is open.
    pub(crate) fn machine_pairing_closed() -> Self {
        Self::new(
            403,
            "pairing_closed",
            "Pairing another Mac isn't open. On that Mac run hollow serve pair --machine, or in Hollow's Settings choose Let another Mac connect.",
        )
    }
    /// M2: the server listens for a paired Mac while the iPhone switch is
    /// off; a phone's token waits for the switch.
    pub(crate) fn phones_off() -> Self {
        Self::new(
            503,
            "unavailable",
            "The iPhone app is off in Hollow's Settings on this Mac.",
        )
    }
    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "not_found", message)
    }
    pub(crate) fn busy(message: impl Into<String>) -> Self {
        Self::new(409, "busy", message)
    }
    /// v1.1: a question card that can't be answered any more (fixture
    /// `error-question-closed.json`).
    pub(crate) fn question_closed() -> Self {
        Self::new(409, "question_closed", QUESTION_CLOSED_MESSAGE)
    }
    /// v1.2: an ask-canvas that can't take these answers (not an ask-canvas,
    /// not its current revision, or answered; fixture
    /// `canvases/error-canvas-closed.json`).
    pub(crate) fn canvas_closed(message: impl Into<String>) -> Self {
        Self::new(409, "canvas_closed", message)
    }
    pub(crate) fn too_large(message: impl Into<String>) -> Self {
        Self::new(413, "too_large", message)
    }
    /// v1.2: a message names an attachment that isn't in the chat's folder
    /// (fixture `attachments/error-attachment-gone.json`).
    pub(crate) fn attachment_gone() -> Self {
        Self::new(
            404,
            "attachment_gone",
            "One of the attachments is no longer on the Mac. Attach it again.",
        )
    }
    pub(crate) fn misdirected() -> Self {
        Self::new(
            421,
            "misdirected",
            "This request wasn't addressed to Hollow.",
        )
    }
    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(503, "unavailable", message)
    }
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        let cause = message.into();
        let cause = cause.trim();
        if cause.is_empty() {
            return Self::new(500, "internal", "Hollow couldn't do that.");
        }
        Self::new(500, "internal", format!("Hollow couldn't do that: {cause}"))
    }

    pub(crate) fn body(&self) -> wire::ErrorBody {
        wire::ErrorBody {
            error: wire::ErrorDetail {
                code: self.code,
                message: self.message.clone(),
                attempts_left: self.attempts_left,
            },
        }
    }
}

/// A route's answer.
pub(crate) enum Reply {
    Json {
        status: u16,
        body: Bytes,
    },
    /// An event stream: its frames.
    Events(live::FrameRx),
    /// v1.1: a status with no body (`204` for the push routes).
    Empty(u16),
    /// v1.2: a file's bytes (an attachment), with the type Hollow worked out
    /// and a plain name for `Content-Disposition`: `inline` for a picture,
    /// `attachment` for any other file.
    Bytes {
        status: u16,
        content_type: String,
        filename: String,
        inline: bool,
        body: Bytes,
    },
}

pub(crate) fn json(status: u16, value: &impl Serialize) -> Reply {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    Reply::Json {
        status,
        body: Bytes::from(body),
    }
}

impl From<ApiError> for Reply {
    fn from(error: ApiError) -> Self {
        json(error.status, &error.body())
    }
}

// ── Context ──────────────────────────────────────────────────────────────

/// What every request shares.
pub(crate) struct Ctx {
    pub vault: PathBuf,
    pub store: Arc<Store>,
    pub pairing: Arc<Pairing>,
    pub live: Arc<Live>,
    pub host: Arc<dyn Host>,
    /// The Mac's name as its owner sees it (Sharing settings).
    pub mac_name: String,
    /// The permission modes the window reported ([`WindowModes`]).
    pub modes: Arc<WindowModes>,
    /// v1.2: the Live Activity push tokens phones registered
    /// (`POST /v1/live-activity`), shared with the pusher that uses them.
    pub(super) activities: Arc<Activities>,
    /// M2: the machine pairing window, apart from the phone's ([`Self::pairing`]):
    /// a phone code never pairs a Mac, nor the reverse.
    pub machine_pairing: Arc<Pairing>,
    /// B4: the approvals code window and its enrollment challenge.
    pub(crate) approval_enroll: Arc<super::approvals_api::Enroll>,
    /// B4 PR 2: open decision challenges and bad signatures per phone.
    pub(crate) approval_decisions: Arc<super::approvals_api::Decisions>,
    /// B4: the root an App Attest chain must end at (Apple's, pinned; tests
    /// use their own).
    pub(crate) attest_root: &'static [u8],
    /// M2: where `GET /v1/machine` reads which Mac this is.
    pub machine: MachineSource,
    /// M2: the iPhone switch is off while the server listens for a paired
    /// Mac, so a phone's token is answered 503 (the app sets it; false
    /// otherwise, as before M2 a server ran only with the switch on).
    pub phones_off: Arc<AtomicBool>,
    pub(super) sends: tokio::sync::Mutex<HashMap<(String, String), (Instant, u16, Bytes)>>,
    homes: Mutex<Option<(Instant, Arc<Homes>)>>,
    /// v1.2: the answers to uploads, kept for a repeated `clientUploadId`
    /// like `sends`.
    pub(super) uploads_seen: Mutex<HashMap<(String, String), (Instant, u16, Bytes)>>,
    /// v1.2: uploads running at once ([`attachments::UPLOADS_AT_ONCE`]).
    pub(super) uploading: Arc<tokio::sync::Semaphore>,
    /// v1.2: downloads running at once ([`attachments::DOWNLOADS_AT_ONCE`]).
    pub(super) downloading: Arc<tokio::sync::Semaphore>,
    /// v1.2: held from an upload's repeat check to its remembered answer when
    /// it carries a `clientUploadId`, so two overlapping retries save once
    /// (the body is read before this, so it is held only for the save).
    pub(super) uploads_saving: tokio::sync::Mutex<()>,
    /// Tests: how many upload saves have begun in their own task, so a test
    /// can hang up the phone only once the save owns the file's bytes.
    #[cfg(test)]
    pub(super) saves_started: std::sync::atomic::AtomicUsize,
    /// Tests: how many sends have reached the `sends` lock, so a test can
    /// revoke the phone while one waits there.
    #[cfg(test)]
    pub(super) sends_waiting: std::sync::atomic::AtomicUsize,
}

impl Ctx {
    pub(crate) fn new(
        vault: PathBuf,
        store: Arc<Store>,
        pairing: Arc<Pairing>,
        live: Arc<Live>,
        host: Arc<dyn Host>,
        mac_name: String,
        modes: Arc<WindowModes>,
        activities: Arc<Activities>,
    ) -> Self {
        Self {
            vault,
            store,
            pairing,
            live,
            host,
            mac_name,
            modes,
            activities,
            machine_pairing: Arc::new(Pairing::default()),
            approval_enroll: Arc::new(super::approvals_api::Enroll::default()),
            approval_decisions: Arc::new(super::approvals_api::Decisions::default()),
            attest_root: super::app_attest::APPLE_ROOT_DER,
            machine: MachineSource::Unset,
            phones_off: Arc::new(AtomicBool::new(false)),
            sends: tokio::sync::Mutex::new(HashMap::new()),
            homes: Mutex::new(None),
            uploads_seen: Mutex::new(HashMap::new()),
            uploading: Arc::new(tokio::sync::Semaphore::new(
                super::attachments::UPLOADS_AT_ONCE,
            )),
            downloading: Arc::new(tokio::sync::Semaphore::new(
                super::attachments::DOWNLOADS_AT_ONCE,
            )),
            uploads_saving: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            saves_started: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            sends_waiting: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Before a request relies on the window's mode choices: its whole
    /// record, if the window can send it ([`WindowModes::settle`]).
    async fn settle_modes(&self) {
        let host = self.host.clone();
        self.modes.settle(move || host.want_modes()).await;
    }

    /// Every workspace's folders and what the list shows of each, read at
    /// most every [`HOMES_FOR`].
    fn homes(&self) -> Result<Arc<Homes>, String> {
        let mut cached = self.homes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, homes)) = cached.as_ref() {
            if at.elapsed() < HOMES_FOR {
                return Ok(homes.clone());
            }
        }
        let homes = Arc::new(Homes::read(&self.vault)?);
        *cached = Some((Instant::now(), homes.clone()));
        Ok(homes)
    }
}

/// M2: where `GET /v1/machine` reads which Mac this is.
pub(crate) enum MachineSource {
    /// None set: the route answers 503 (a server built without one).
    Unset,
    /// The app's or serve's own `~/.config/hollow/machine.json`.
    App { headless: bool },
    /// A `machine.json` at this path (tests); `runner`: its vault is the
    /// runner COO's (as `hollow serve runner-coo on` makes it, M3a).
    #[cfg_attr(not(test), allow(dead_code))]
    At {
        path: PathBuf,
        headless: bool,
        runner: bool,
    },
}

impl MachineSource {
    /// This Mac as another Mac sees it, its `machineId` written on first
    /// need ([`crate::context_roots::ensure_machine_id`]); with `vault`, the
    /// runner COO's chat key when this Hollow runs one (M3a, `coo`).
    pub(crate) fn info(
        &self,
        mac_name: &str,
        vault: Option<&Path>,
    ) -> Result<wire::MachineInfo, String> {
        let (machine_id, slug, headless) = match self {
            Self::Unset => return Err("This Hollow can't say which Mac it is".into()),
            Self::App { headless } => (
                crate::context_roots::ensure_machine_id()?,
                crate::context_roots::machine_slug(),
                *headless,
            ),
            Self::At { path, headless, .. } => (
                crate::context_roots::ensure_machine_id_at(path)?,
                crate::context_roots::machine_slug_at(path),
                *headless,
            ),
        };
        let role = self.role();
        let coo = vault.and_then(|vault| match self {
            Self::At { .. } => crate::runner_coo::peer_coo_for(role, vault),
            _ => crate::runner_coo::peer_coo(vault),
        });
        let runner = vault.is_some() && role == crate::runner_coo::Role::Runner;
        let build = crate::app_menu::build_identity();
        Ok(wire::MachineInfo {
            machine_id,
            slug,
            name: mac_name.to_string(),
            app: "hollow",
            api: wire::API,
            peer_api: wire::PEER_API,
            version: build.version,
            commit: build.short_commit,
            headless,
            capabilities: crate::runner_coo::capabilities(
                &wire::CAPABILITIES,
                coo.as_deref(),
                runner,
            ),
            coo,
        })
    }

    /// This Hollow's runner COO role as a peer sees it: the process's, or
    /// the test source's `runner` flag.
    pub(crate) fn role(&self) -> crate::runner_coo::Role {
        match self {
            Self::At { runner: true, .. } => crate::runner_coo::Role::Runner,
            Self::At { .. } => crate::runner_coo::Role::Single,
            _ => crate::runner_coo::role(),
        }
    }
}

/// Which workspace a chat's folder belongs to, and each workspace's row.
pub(crate) struct Homes {
    homes: crate::workspaces::SessionHomes,
    info: HashMap<String, crate::workspaces::Workspace>,
    vault: PathBuf,
}

impl Homes {
    pub(crate) fn read(vault: &Path) -> Result<Self, String> {
        let homes = crate::workspaces::SessionHomes::read(vault)?;
        let info = crate::workspaces::list(vault)?
            .into_iter()
            .map(|w| (w.id.clone(), w))
            .collect();
        Ok(Self {
            homes,
            info,
            vault: vault.to_path_buf(),
        })
    }

    /// The workspace a chat in folder `cwd` belongs to; `None` for the vault
    /// root or a folder that is no workspace's.
    pub(crate) fn workspace(&self, cwd: &str) -> Option<wire::WorkspaceRef> {
        if Path::new(cwd) == self.vault {
            return None;
        }
        let id = self.homes.workspace_for(cwd)?;
        let w = self.info.get(&id)?;
        Some(wire::WorkspaceRef {
            id: w.id.clone(),
            title: w.title.clone(),
            kind: crate::workspaces::origin_kind(w).to_string(),
        })
    }
}

// ── Chat state ───────────────────────────────────────────────────────────

/// Queued and running chats, from the run queue as the window reads it
/// (Working, not Queued, 2026-11-17): a row with a real wait (a limit,
/// memory, another run's folder or lane, another process running the queue)
/// is `queued`; a row this process is admitting or running, and a queued row
/// nothing truly holds back, is `running`, the window's Working ("Preparing"
/// before the agent runs; the wire has no word for that part). A row no
/// process holds (left by a crash) is neither.
pub(crate) struct QueueView {
    by_key: HashMap<String, &'static str>,
}

impl QueueView {
    pub(crate) fn read(vault: &Path) -> Self {
        use crate::run_queue::{RowStatus, RunSource};
        let active = crate::chat::active_chat_rows(vault);
        let rows = crate::run_queue::live_rows(vault).unwrap_or_default();
        // Read once, and only when a queued chat row needs it.
        let facts = std::cell::OnceCell::new();
        let now = chrono::Utc::now();
        let mut by_key: HashMap<String, &'static str> = HashMap::new();
        for row in &rows {
            if row.source != RunSource::Chat {
                continue;
            }
            let Some(key) = row.source_id.clone() else {
                continue;
            };
            let state = match row.status {
                RowStatus::Queued
                    if crate::run_pump::item_with_reason(
                        vault,
                        row,
                        None,
                        facts.get_or_init(|| crate::run_pump::QueueFacts::read(vault, &rows)),
                        now,
                    )
                    .reads_working() =>
                {
                    "running"
                }
                RowStatus::Queued => "queued",
                RowStatus::Claimed | RowStatus::Running if active.contains(&row.id) => "running",
                _ => continue,
            };
            let slot = by_key.entry(key).or_insert(state);
            if state == "running" {
                *slot = state;
            }
        }
        Self { by_key }
    }

    pub(crate) fn state(&self, key: &str) -> &'static str {
        self.by_key.get(key).copied().unwrap_or("idle")
    }
}

/// The chats with a question card waiting for the person.
pub(crate) fn waiting_cards(vault: &Path) -> HashSet<String> {
    crate::interactions::list_open_every(vault)
        .unwrap_or_default()
        .into_iter()
        .filter(|card| card.status == "pending")
        .map(|card| card.session_key)
        .collect()
}

/// One chat's `(state, needsAnswer)`.
pub(crate) fn chat_state(vault: &Path, key: &str) -> (String, bool) {
    let state = QueueView::read(vault).state(key).to_string();
    let waiting = crate::interactions::list_pending(vault, key)
        .unwrap_or_default()
        .iter()
        .any(|card| card.status == "pending");
    (state, waiting)
}

/// [`Lookup`] over the vault's own stores.
pub(crate) struct VaultLookup {
    vault: PathBuf,
}

impl VaultLookup {
    pub(crate) fn new(vault: PathBuf) -> Self {
        Self { vault }
    }
}

impl Lookup for VaultLookup {
    fn newest_turn(&self, key: &str) -> Option<TurnRecord> {
        crate::session_store::latest_turn(&self.vault, key)
            .ok()
            .flatten()
            .map(|t| TurnRecord {
                ord: t.ord,
                started_at: t.started_at,
                status: t.status,
            })
    }

    fn state(&self, key: &str) -> (String, bool) {
        chat_state(&self.vault, key)
    }

    fn digest(&self, limit: usize) -> Vec<RowDigest> {
        let queue = QueueView::read(&self.vault);
        let waiting = waiting_cards(&self.vault);
        crate::session_store::list_digest(&self.vault, limit)
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                let digest = format!(
                    "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
                    row.title.unwrap_or_default(),
                    row.last_activity_at,
                    row.turn_count,
                    queue.state(&row.key),
                    waiting.contains(&row.key)
                );
                (row.key, digest)
            })
            .collect()
    }
}

// ── Shaping ──────────────────────────────────────────────────────────────

/// A list preview: Hollow's fences removed, whitespace collapsed, at most
/// [`PREVIEW_MAX`] characters; `None` when nothing is left.
pub(crate) fn preview(text: &str) -> Option<String> {
    let (plain, _) = crate::next_step::split(text, &PREVIEW_FENCES);
    let collapsed = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= PREVIEW_MAX {
        return Some(collapsed);
    }
    let mut cut = live::cut_chars(&collapsed, PREVIEW_MAX - 1);
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    Some(cut)
}

/// A turn's text, cut at [`TURN_TEXT_MAX`] characters.
pub(crate) fn turn_text(text: &str) -> (String, bool) {
    match text.char_indices().nth(TURN_TEXT_MAX) {
        Some((at, _)) => (text[..at].to_string(), true),
        None => (text.to_string(), false),
    }
}

/// Where a chat runs its next turn from the person (v1.1 `runsOn`), as
/// Hollow places it (`remote_session::turn_place` for an attended turn):
/// `mac-mini` when its workspace's own Runs on, else Settings, Devices,
/// Runs on for a build workspace, sends a Claude or Codex turn there and
/// the chat isn't on a self-hosted model (those stay here); `this-mac`
/// otherwise. `None` when the chat's folder is gone. `chosen` caches the
/// workspace rule per folder and agent within one read.
pub(crate) fn runs_on_of(
    vault: &Path,
    summary: &AgentSessionSummary,
    reads: crate::remote_session::PlaceReads,
    chosen: &mut HashMap<(String, String), bool>,
) -> Option<String> {
    let cwd = Path::new(&summary.cwd);
    if !cwd.is_dir() {
        return None;
    }
    let Some(kind) = crate::agent::AgentKind::from_opt(&summary.agent) else {
        return Some(THIS_MAC.into());
    };
    let runner = *chosen
        .entry((summary.cwd.clone(), summary.agent.clone()))
        .or_insert_with(|| crate::remote_session::chosen_runner(vault, cwd, kind, reads));
    // Only a chat Runs on would move reads its model (one query).
    let self_hosted = runner
        && crate::self_hosted::chat_on_self_hosted(
            kind,
            crate::wake_me::wake_model(vault, &summary.key, &summary.agent).as_deref(),
        );
    Some(
        if runner && !self_hosted {
            MAC_MINI
        } else {
            THIS_MAC
        }
        .to_string(),
    )
}

#[allow(clippy::too_many_arguments)]
fn session_row(
    summary: &AgentSessionSummary,
    homes: &Homes,
    queue: &QueueView,
    waiting: &HashSet<String>,
    exchange: Option<&(u32, Option<String>, Option<String>)>,
    live: Option<(u32, String)>,
    runs_on: Option<String>,
) -> wire::Session {
    let preview = {
        let live_text = live
            .filter(|(ord, _)| exchange.is_none_or(|(newest, _, _)| newest == ord))
            .and_then(|(_, text)| preview(&text));
        live_text
            .or_else(|| exchange.and_then(|(_, _, answer)| answer.as_deref().and_then(preview)))
            .or_else(|| exchange.and_then(|(_, asked, _)| asked.as_deref().and_then(preview)))
    };
    wire::Session {
        key: summary.key.clone(),
        title: summary.title.clone().filter(|t| !t.trim().is_empty()),
        agent: summary.agent.clone(),
        workspace: homes.workspace(&summary.cwd),
        created_at: summary.created_at.clone(),
        last_activity_at: summary.last_activity_at.clone(),
        turn_count: summary.turn_count,
        state: queue.state(&summary.key).to_string(),
        needs_answer: waiting.contains(&summary.key),
        preview,
        runs_on,
        moved_to: None,
        moved_from: None,
    }
}

/// M5 UI: `movedTo` / `movedFrom` on rows another Mac reads (never a
/// phone's: the iPhone contract has no such fields). A read failure marks
/// nothing.
pub(crate) fn mark_moved(vault: &Path, rows: &mut [wire::Session]) {
    let Ok(homes) = crate::chat_move::homes(vault) else {
        return;
    };
    for row in rows {
        let Some(home) = homes.get(&row.key) else {
            continue;
        };
        match home.state.as_str() {
            crate::chat_move::AWAY => row.moved_to = Some(home.peer.clone()),
            crate::chat_move::ARRIVED => row.moved_from = Some(home.peer.clone()),
            _ => {}
        }
    }
}

/// Rows for `summaries`, read together.
pub(crate) fn session_rows(
    ctx: &Ctx,
    summaries: &[AgentSessionSummary],
) -> Result<Vec<wire::Session>, String> {
    let homes = ctx.homes()?;
    let queue = QueueView::read(&ctx.vault);
    let waiting = waiting_cards(&ctx.vault);
    let keys: Vec<String> = summaries.iter().map(|s| s.key.clone()).collect();
    let exchanges = crate::session_store::newest_exchanges(&ctx.vault, &keys)?;
    let reads = ctx.host.place();
    let mut chosen = HashMap::new();
    Ok(summaries
        .iter()
        .map(|s| {
            session_row(
                s,
                &homes,
                &queue,
                &waiting,
                exchanges.get(&s.key),
                ctx.live.text(&s.key),
                runs_on_of(&ctx.vault, s, reads, &mut chosen),
            )
        })
        .collect())
}

/// What a card asked, as the phone shows it: the questions of a questions
/// card; a confirmation, checkbox or verdicts card as the one question it
/// asks (a verdicts card as one per item).
pub(crate) fn questions_of(card: &ChatInteraction) -> Option<wire::Questions> {
    use crate::interaction_schema::{AnswerMode, Mode, QuestionRequest};
    let request: QuestionRequest = serde_json::from_value(card.request.clone()).ok()?;
    let title = request.title.clone().unwrap_or_default();
    let option = |o: &crate::interaction_schema::QuestionOption| wire::QuestionOption {
        id: o.id.clone(),
        label: o.label.clone(),
        description: o.description.clone(),
        recommended: o.recommended,
    };
    let plain = |id: &str, label: &str| wire::QuestionOption {
        id: id.to_string(),
        label: label.to_string(),
        description: None,
        recommended: false,
    };
    let items = match request.mode {
        Mode::Questions => request
            .questions
            .iter()
            .map(|q| wire::Question {
                id: q.id.clone(),
                header: q.header.clone().unwrap_or_else(|| title.clone()),
                prompt: q.prompt.clone(),
                // The contract's words: `single`, `multi` or `text`.
                mode: match q.answer_mode {
                    AnswerMode::Single => "single",
                    AnswerMode::Multi => "multi",
                    AnswerMode::Text => "text",
                }
                .to_string(),
                required: q.required,
                // A text question is the person's own words already.
                allows_other: q.allow_custom && q.answer_mode != AnswerMode::Text,
                options: q.options.iter().map(option).collect(),
            })
            .collect(),
        Mode::Confirmation => {
            let c = request.confirmation.as_ref()?;
            vec![wire::Question {
                id: "confirmation".into(),
                header: title,
                prompt: c.prompt.clone(),
                mode: "single".into(),
                required: true,
                allows_other: false,
                options: vec![
                    plain("accept", c.accept_label.as_deref().unwrap_or("Accept")),
                    plain("reject", c.reject_label.as_deref().unwrap_or("Reject")),
                ],
            }]
        }
        Mode::Checkbox => {
            let c = request.checkbox.as_ref()?;
            vec![wire::Question {
                id: "checkbox".into(),
                header: title,
                prompt: c.prompt.clone(),
                mode: "multi".into(),
                required: c.min_selected.is_some_and(|min| min > 0),
                allows_other: false,
                options: c.options.iter().map(option).collect(),
            }]
        }
        Mode::Verdicts => {
            let v = request.verdicts.as_ref()?;
            let verdicts: Vec<wire::QuestionOption> = v
                .verdicts
                .iter()
                .map(|verdict| {
                    let id = serde_json::to_value(verdict)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_default();
                    plain(&id, verdict.label())
                })
                .collect();
            v.items
                .iter()
                .map(|item| wire::Question {
                    id: item.id.clone(),
                    header: if title.is_empty() {
                        v.prompt.clone()
                    } else {
                        title.clone()
                    },
                    prompt: item
                        .description
                        .clone()
                        .map(|d| format!("{}: {d}", item.label))
                        .unwrap_or_else(|| item.label.clone()),
                    mode: "single".into(),
                    required: true,
                    allows_other: false,
                    options: verdicts.clone(),
                })
                .collect()
        }
    };
    Some(wire::Questions {
        id: card.id.clone(),
        state: if card.status == "pending" {
            "pending"
        } else {
            "closed"
        }
        .to_string(),
        items,
    })
}

/// The ord in one of Hollow's own turn ids (`<key>-<ord>-user`).
fn ord_of(key: &str, id: &str) -> Option<u32> {
    id.strip_prefix(key)?
        .strip_prefix('-')?
        .split('-')
        .next()?
        .parse()
        .ok()
}

fn turn_of(
    key: &str,
    turn: &crate::session_store::ConversationTurn,
    cards: &HashMap<u32, &ChatInteraction>,
) -> Option<wire::Turn> {
    let ord = ord_of(key, &turn.id)?;
    let assistant = turn.role == "assistant";
    let (text, truncated) = turn_text(&turn.text);
    Some(wire::Turn {
        id: turn.id.clone(),
        ord,
        role: turn.role.clone(),
        text,
        agent: turn.agent.clone(),
        started_at: turn.started_at.clone(),
        status: turn.status.clone(),
        error: if assistant {
            turn.error.clone().filter(|e| !e.trim().is_empty())
        } else {
            None
        },
        truncated,
        tools: if assistant {
            turn.tools
                .iter()
                .take(live::TOOLS_CAP)
                .map(|t| wire::Tool {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    status: match t.status.as_str() {
                        "running" => "running",
                        "error" => "error",
                        _ if t.is_error => "error",
                        _ => "done",
                    }
                    .to_string(),
                    detail: live::tool_detail(&t.input),
                })
                .collect()
        } else {
            Vec::new()
        },
        questions: if assistant {
            cards.get(&ord).and_then(|card| questions_of(card))
        } else {
            None
        },
        // v1.2: filled by `detail` from the turns' recorded attachments.
        attachments: Vec::new(),
    })
}

// ── Routes ───────────────────────────────────────────────────────────────

/// A spawned task that didn't finish (it panicked) answers the contract's
/// generic internal error. The `JoinError`'s own text ("task 12 panicked
/// with message ...") is Hollow's internals, not a sentence for the phone
/// (second review, finding 6, 2026-11-04), so it goes to the log instead.
fn task_failed(error: tokio::task::JoinError) -> ApiError {
    eprintln!("[phone] a request's task stopped: {error}");
    ApiError::internal("")
}

/// Run blocking store reads off the async workers.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(task_failed)?
}

pub(crate) fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == name).then(|| {
            urlencoding::decode(&v.replace('+', " "))
                .map(|v| v.into_owned())
                .unwrap_or_default()
        })
    })
}

pub(crate) fn bounded(
    query: &str,
    name: &str,
    default: usize,
    max: usize,
) -> Result<usize, ApiError> {
    match query_param(query, name) {
        None => Ok(default),
        Some(raw) => match raw.trim().parse::<usize>() {
            Ok(n) if (1..=max).contains(&n) => Ok(n),
            _ => Err(ApiError::bad_request(format!(
                "{name} must be a whole number from 1 to {max}."
            ))),
        },
    }
}

pub(super) fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_| ApiError::bad_request("The request body isn't the JSON Hollow expects."))
}

/// The message text, trimmed: 400 when empty, 413 past [`TEXT_MAX`].
///
/// A message with attachments (v1.2, `attachmentIds`) may have no words.
fn message_text(text: Option<&str>, attachments: bool) -> Result<String, ApiError> {
    let text = text.unwrap_or("").trim();
    if text.is_empty() && !attachments {
        return Err(ApiError::bad_request("The message is empty."));
    }
    if text.chars().count() > TEXT_MAX {
        return Err(ApiError::too_large(format!(
            "That message is longer than Hollow takes ({TEXT_MAX} characters)."
        )));
    }
    Ok(text.to_string())
}

/// A `clientMessageId`: a UUID from the app (letters, digits and dashes).
pub(super) fn message_id(id: Option<&str>) -> Result<String, ApiError> {
    let id = id.unwrap_or("").trim();
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ApiError::bad_request(
            "clientMessageId must be the message's UUID.",
        ));
    }
    Ok(id.to_ascii_lowercase())
}

/// A chat that exists and isn't archived.
pub(crate) fn live_chat(vault: &Path, key: &str) -> Result<AgentSessionSummary, ApiError> {
    let missing = || ApiError::not_found("That chat isn't in Hollow, or it was archived.");
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(missing());
    }
    match crate::session_store::is_archived(vault, key) {
        Ok(Some(false)) => {}
        Ok(_) => return Err(missing()),
        Err(e) => return Err(ApiError::internal(e)),
    }
    crate::session_store::session_by_key(vault, key)
        .map_err(ApiError::internal)?
        .ok_or_else(missing)
}

// ── Who may use which route (M2) ─────────────────────────────────────────

/// Who may call a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// No token (`health` and the two pairing routes).
    Open,
    /// A phone's token only.
    Phone,
    /// A phone's or another Mac's.
    Both,
    /// Another Mac's only.
    Machine,
}

/// Every route [`dispatch`] answers, and who may call it (`{}` is one path
/// part). The peer link is live control only: a Mac may list and read
/// chats, send to one that exists, stop it, answer its card and follow the
/// event stream, and nothing else (no tasks, Waiting on you, files, notes,
/// canvases, search, push or new chats). A route not in this table is
/// refused to a Mac, and a test fails until every arm of `dispatch` is here,
/// so a phone route added later never reaches a Mac by accident.
pub(crate) const ROUTES: &[(&str, &str, Scope)] = &[
    ("GET", "/v1/health", Scope::Open),
    ("POST", "/v1/pair", Scope::Open),
    ("POST", "/v1/machines/pair", Scope::Open),
    ("GET", "/v1/me", Scope::Phone),
    ("GET", "/v1/machine", Scope::Machine),
    ("GET", "/v1/machine/usage", Scope::Machine),
    ("POST", "/v1/machine/coo", Scope::Machine),
    ("DELETE", "/v1/machines/me", Scope::Machine),
    (
        "POST",
        "/v1/machines/moves/{id}/parts/{part}",
        Scope::Machine,
    ),
    ("POST", "/v1/machines/moves/{id}/commit", Scope::Machine),
    ("GET", "/v1/machines/moves/{id}", Scope::Machine),
    ("DELETE", "/v1/machines/moves/{id}", Scope::Machine),
    ("GET", "/v1/chat-controls", Scope::Phone),
    ("GET", "/v1/sessions", Scope::Both),
    ("POST", "/v1/sessions", Scope::Phone),
    ("GET", "/v1/sessions/{key}", Scope::Both),
    ("POST", "/v1/sessions/{key}/messages", Scope::Both),
    (
        "POST",
        "/v1/sessions/{key}/questions/{question}/answer",
        Scope::Both,
    ),
    ("POST", "/v1/sessions/{key}/stop", Scope::Both),
    ("POST", "/v1/sessions/{key}/attachments", Scope::Phone),
    ("POST", "/v1/workspaces/{id}/attachments", Scope::Phone),
    ("GET", "/v1/sessions/{key}/attachments/{id}", Scope::Phone),
    ("GET", "/v1/workspaces", Scope::Phone),
    ("GET", "/v1/tasks", Scope::Phone),
    ("GET", "/v1/tasks/statuses", Scope::Phone),
    ("POST", "/v1/tasks", Scope::Phone),
    ("POST", "/v1/tasks/status", Scope::Phone),
    ("POST", "/v1/tasks/update", Scope::Phone),
    ("POST", "/v1/notes", Scope::Phone),
    ("GET", "/v1/workspaces/overview", Scope::Phone),
    ("GET", "/v1/workspaces/{id}", Scope::Phone),
    ("GET", "/v1/workspaces/{id}/file", Scope::Phone),
    ("GET", "/v1/search", Scope::Phone),
    ("GET", "/v1/calendar", Scope::Phone),
    ("GET", "/v1/waiting", Scope::Phone),
    ("GET", "/v1/approvals", Scope::Phone),
    ("POST", "/v1/approvals/key/challenge", Scope::Phone),
    ("POST", "/v1/approvals/key", Scope::Phone),
    ("DELETE", "/v1/approvals/key", Scope::Phone),
    ("POST", "/v1/approvals/{id}/challenge", Scope::Phone),
    ("POST", "/v1/approvals/{id}/decision", Scope::Phone),
    ("POST", "/v1/push", Scope::Phone),
    ("DELETE", "/v1/push", Scope::Phone),
    ("POST", "/v1/live-activity", Scope::Phone),
    ("DELETE", "/v1/live-activity", Scope::Phone),
    ("GET", "/v1/events", Scope::Both),
    ("GET", "/v1/canvas-kit", Scope::Phone),
    ("GET", "/v1/sessions/{key}/canvases", Scope::Phone),
    ("GET", "/v1/sessions/{key}/canvases/{id}", Scope::Phone),
    (
        "POST",
        "/v1/sessions/{key}/canvases/{id}/answer",
        Scope::Phone,
    ),
    ("GET", "/v1/workspaces/{id}/canvases", Scope::Phone),
    ("GET", "/v1/workspaces/{id}/canvases/{canvas}", Scope::Phone),
];

/// The scope of `method parts` in [`ROUTES`] (`None`: no such route). Of
/// the rows that match, the most specific (fewest `{}` parts) wins, as
/// `dispatch`'s literal arms come before its placeholder ones.
pub(crate) fn scope_of(method: &str, parts: &[&str]) -> Option<Scope> {
    ROUTES
        .iter()
        .filter_map(|(m, pattern, scope)| {
            let want: Vec<&str> = pattern.split('/').skip(1).collect();
            let same = *m == method
                && want.len() == parts.len()
                && want
                    .iter()
                    .zip(parts)
                    .all(|(w, p)| is_placeholder(w) || w == p);
            same.then(|| (want.iter().filter(|w| is_placeholder(w)).count(), *scope))
        })
        .min_by_key(|(placeholders, _)| *placeholders)
        .map(|(_, scope)| scope)
}

/// A `{name}` part of a [`ROUTES`] pattern.
pub(crate) fn is_placeholder(part: &str) -> bool {
    part.starts_with('{') && part.ends_with('}')
}

/// The turn's `clientTurnId`: `phone-<id>` from a phone, `mac-<id>` from
/// another Mac (M2), so a turn record says where it came from.
fn turn_id(device_id: &str, id: &str) -> String {
    if device_id.starts_with("mc-") {
        format!("mac-{id}")
    } else {
        format!("phone-{id}")
    }
}

/// Whether a device of `kind` may call `method parts`: a Mac only what
/// [`ROUTES`] gives it (default deny), a phone anything but a Mac's own.
pub(crate) fn may_call(kind: DeviceKind, method: &str, parts: &[&str]) -> Result<(), ApiError> {
    match (kind, scope_of(method, parts)) {
        (DeviceKind::Machine, Some(Scope::Both | Scope::Machine)) => Ok(()),
        (DeviceKind::Machine, _) => Err(ApiError::not_for_machines()),
        (DeviceKind::Phone, Some(Scope::Machine)) => Err(ApiError::machines_only()),
        (DeviceKind::Phone, _) => Ok(()),
    }
}

/// Answer a request. `device` is the paired phone (or Mac) the token named
/// (`None` only for `health` and the pairing routes, which need none).
pub(crate) async fn dispatch(
    ctx: &Arc<Ctx>,
    device: Option<&Device>,
    method: &str,
    path: &str,
    query: &str,
    body: Bytes,
) -> Reply {
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').skip(1).collect();
    if let Some(device) = device {
        if let Err(refused) = may_call(device.kind, method, &parts) {
            return refused.into();
        }
    }
    let result = match (method, parts.as_slice(), device) {
        ("GET", ["v1", "health"], _) => Ok(json(200, &wire::Health::now())),
        ("POST", ["v1", "pair"], _) => pair(ctx, &body),
        // M2: another Mac (docs/phone-api/machines.md).
        ("POST", ["v1", "machines", "pair"], _) => machine_pair(ctx, &body).await,
        (_, _, None) => Err(ApiError::unauthorized()),
        ("GET", ["v1", "machine"], Some(_)) => machine_info(ctx).await,
        ("GET", ["v1", "machine", "usage"], Some(_)) => machine_usage(ctx).await,
        ("POST", ["v1", "machine", "coo"], Some(device)) => {
            machine_coo_start(ctx, device, &body).await
        }
        ("DELETE", ["v1", "machines", "me"], Some(device)) => machine_forget(ctx, device).await,
        // M5: a chat moving here from another Mac, its history sent once.
        ("POST", ["v1", "machines", "moves", id, "parts", part], Some(device)) => {
            move_part(ctx, device, id, part, &body).await
        }
        ("POST", ["v1", "machines", "moves", id, "commit"], Some(device)) => {
            move_commit(ctx, device, id, &body).await
        }
        ("GET", ["v1", "machines", "moves", id], Some(device)) => move_get(ctx, device, id).await,
        ("DELETE", ["v1", "machines", "moves", id], Some(device)) => {
            move_cancel(ctx, device, id).await
        }
        ("GET", ["v1", "me"], Some(device)) => Ok(json(
            200,
            &wire::Me {
                device_id: device.device_id.clone(),
                device_name: device.name.clone(),
                mac_name: ctx.mac_name.clone(),
                api: wire::API,
                runner: ctx.host.runner(),
            },
        )),
        ("GET", ["v1", "chat-controls"], Some(_)) => control_options(ctx, query).await,
        ("GET", ["v1", "sessions"], Some(device)) => list(ctx, device, query).await,
        ("POST", ["v1", "sessions"], Some(device)) => new_chat(ctx, device, &body).await,
        ("GET", ["v1", "sessions", key], Some(device)) => detail(ctx, device, key, query).await,
        ("POST", ["v1", "sessions", key, "messages"], Some(device)) => {
            send(ctx, device, key, &body).await
        }
        ("POST", ["v1", "sessions", key, "questions", question, "answer"], Some(device)) => {
            answer(ctx, device, key, question, &body).await
        }
        ("POST", ["v1", "sessions", key, "stop"], Some(device)) => stop(ctx, device, key).await,
        // v1.2: attachments (docs/phone-api/attachments.md).
        ("POST", ["v1", "sessions", key, "attachments"], Some(device)) => {
            let target = attachments::Target::Chat((*key).to_string());
            attachments::upload(ctx, device, target, query, body).await
        }
        ("POST", ["v1", "workspaces", id, "attachments"], Some(device)) => {
            let target = attachments::Target::Workspace((*id).to_string());
            attachments::upload(ctx, device, target, query, body).await
        }
        ("GET", ["v1", "sessions", key, "attachments", id], Some(_)) => {
            attachments::download(ctx, key, id).await
        }
        ("GET", ["v1", "workspaces"], Some(_)) => workspaces(ctx).await,
        // v1.2 (lane I3): the Tasks tab's routes (`docs/phone-api/tasks.md`).
        ("GET", ["v1", "tasks"], Some(_)) => tasks_api::list(ctx, query).await,
        ("GET", ["v1", "tasks", "statuses"], Some(_)) => tasks_api::statuses(ctx).await,
        ("POST", ["v1", "tasks"], Some(device)) => tasks_api::create(ctx, device, &body).await,
        ("POST", ["v1", "tasks", "status"], Some(device)) => {
            tasks_api::status(ctx, device, &body).await
        }
        ("POST", ["v1", "tasks", "update"], Some(device)) => {
            tasks_api::update(ctx, device, &body).await
        }
        // v1.2 (lane I3): a quick note from the phone (`docs/phone-api/notes.md`).
        ("POST", ["v1", "notes"], Some(device)) => notes_api::create(ctx, device, &body).await,
        // v1.2 (lane I3): the Workspaces tab's read-only routes.
        ("GET", ["v1", "workspaces", "overview"], Some(_)) => {
            workspaces_api::overview(ctx, query).await
        }
        ("GET", ["v1", "workspaces", id], Some(_)) => workspaces_api::detail(ctx, id).await,
        ("GET", ["v1", "workspaces", id, "file"], Some(_)) => {
            workspaces_api::file(ctx, id, query).await
        }
        // v1.2 (lane I3): search over chats and workspaces.
        ("GET", ["v1", "search"], Some(_)) => search_api::search(ctx, query).await,
        // v1.2 (build run 11-17, B3): the Today tab's meetings, read-only, work data only.
        ("GET", ["v1", "calendar"], Some(_)) => calendar_api::list(ctx, query).await,
        ("GET", ["v1", "waiting"], Some(_)) => waiting(ctx).await,
        // v1.2 (build run 11-17, B4): approvals from the phone behind Face ID.
        ("GET", ["v1", "approvals"], Some(device)) => approvals_api::list(ctx, device).await,
        ("POST", ["v1", "approvals", "key", "challenge"], Some(device)) => {
            approvals_api::key_challenge(ctx, device).await
        }
        ("POST", ["v1", "approvals", "key"], Some(device)) => {
            approvals_api::key_enroll(ctx, device, &body).await
        }
        ("DELETE", ["v1", "approvals", "key"], Some(device)) => {
            approvals_api::key_forget(ctx, device).await
        }
        ("POST", ["v1", "approvals", id, "challenge"], Some(device)) => {
            approvals_api::decision_challenge(ctx, device, id).await
        }
        ("POST", ["v1", "approvals", id, "decision"], Some(device)) => {
            approvals_api::decide(ctx, device, id, &body).await
        }
        ("POST", ["v1", "push"], Some(device)) => push_register(ctx, device, &body).await,
        ("DELETE", ["v1", "push"], Some(device)) => push_forget(ctx, device).await,
        // v1.2: Live Activities (`docs/phone-api/live-activity.md`).
        ("POST", ["v1", "live-activity"], Some(device)) => {
            live_activity_register(ctx, device, &body).await
        }
        ("DELETE", ["v1", "live-activity"], Some(device)) => {
            live_activity_forget(ctx, device, query).await
        }
        ("GET", ["v1", "events"], Some(device)) => events(ctx, device, query).await,
        // v1.2: canvases (`docs/phone-api/canvases.md`).
        ("GET", ["v1", "canvas-kit"], Some(_)) => Ok(canvases::kit_reply()),
        ("GET", ["v1", "sessions", key, "canvases"], Some(_)) => {
            canvases::chat_list(ctx, key).await
        }
        ("GET", ["v1", "sessions", key, "canvases", id], Some(_)) => {
            canvases::chat_read(ctx, key, id, query).await
        }
        ("POST", ["v1", "sessions", key, "canvases", id, "answer"], Some(device)) => {
            canvases::answer(ctx, device, key, id, &body).await
        }
        ("GET", ["v1", "workspaces", id, "canvases"], Some(_)) => {
            canvases::workspace_list(ctx, id).await
        }
        ("GET", ["v1", "workspaces", id, "canvases", canvas], Some(_)) => {
            canvases::workspace_read(ctx, id, canvas, query).await
        }
        _ => Err(ApiError::not_found("Hollow has no such route.")),
    };
    result.unwrap_or_else(Reply::from)
}

async fn control_options(ctx: &Ctx, query: &str) -> Result<Reply, ApiError> {
    let selection = super::controls::Selection {
        agent: query_param(query, "agent"),
        ..Default::default()
    };
    let options = ctx.host.control_options(selection.agent_kind()?).await?;
    Ok(json(200, &options))
}

fn pair(ctx: &Ctx, body: &[u8]) -> Result<Reply, ApiError> {
    let request: wire::PairRequest = parse_body(body)?;
    let code = request
        .code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| ApiError::bad_request("The pairing code is missing."))?;
    let name = request.device_name.as_deref().unwrap_or("").trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err(ApiError::bad_request(
            "deviceName must be 1 to 80 characters.",
        ));
    }
    let model = request
        .device_model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    if model.is_some_and(|m| m.chars().count() > 80 || m.chars().any(char::is_control)) {
        return Err(ApiError::bad_request(
            "deviceModel must be at most 80 characters.",
        ));
    }
    let now = chrono::Utc::now();
    match ctx.pairing.attempt(code, now) {
        Attempt::Closed => Err(ApiError::pairing_closed()),
        Attempt::Wrong { attempts_left } => {
            ctx.host.changed();
            Err(ApiError::wrong_code(attempts_left))
        }
        Attempt::Paired => {
            let (device, token) = ctx
                .store
                .pair(name, model, now)
                .map_err(ApiError::internal)?;
            // The first phone paired with the switch on starts copying.
            ctx.live.follow(|| ctx.store.viewers_ready());
            ctx.host.changed();
            Ok(json(
                200,
                &wire::PairResponse {
                    token,
                    device_id: device.device_id,
                    device_name: device.name,
                    mac_name: ctx.mac_name.clone(),
                    api: wire::API,
                },
            ))
        }
    }
}

/// A Mac's name or version as it sent it: 1 to `max` characters, no
/// control characters.
fn plain(text: Option<&str>, max: usize) -> Option<String> {
    let text = text?.trim();
    (!text.is_empty() && text.chars().count() <= max && !text.chars().any(char::is_control))
        .then(|| text.to_string())
}

/// What an unpaired caller reads when this Mac can't say which Mac it is.
pub(crate) const MACHINE_UNKNOWN: &str =
    "This Mac couldn't read which Mac it is, so it paired nothing. Open a new pairing window and try again.";

/// `GET /v1/machine`: which Mac and Hollow this is.
async fn machine_info(ctx: &Arc<Ctx>) -> Result<Reply, ApiError> {
    let ctx = ctx.clone();
    let info = blocking(move || {
        ctx.machine
            .info(&ctx.mac_name, Some(&ctx.vault))
            .map_err(ApiError::unavailable)
    })
    .await?;
    Ok(json(200, &info))
}

/// Two Macs starting the runner COO at once start one conversation.
static COO_START: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// M3a follow-up: `POST /v1/machine/coo`, a paired Mac starts the runner
/// COO's first conversation (`docs/phone-api/machines.md`) with the text the
/// person's Start its COO sends. Only on the runner and only while its COO
/// has no conversation (`GET /v1/machine` lists `coo-start` then): another
/// Mac may still not start any other chat. It starts in the `main`
/// workspace as a phone's new chat does there (this Hollow's new-chat mode).
/// 404 `not_found` when this Hollow isn't the runner; 409 `busy` when its
/// COO has a conversation (send to it, its key is `GET /v1/machine`'s `coo`).
async fn machine_coo_start(
    ctx: &Arc<Ctx>,
    device: &Device,
    body: &[u8],
) -> Result<Reply, ApiError> {
    if ctx.machine.role() != crate::runner_coo::Role::Runner {
        return Err(ApiError::not_found("This Mac doesn't run a runner COO."));
    }
    let request: wire::CooStartRequest = parse_body(body)?;
    let text = message_text(request.text.as_deref(), false)?;
    let id = message_id(request.client_message_id.as_deref())?;
    let _one = COO_START.lock().await;
    let vault = ctx.vault.clone();
    let has =
        blocking(move || crate::runner_coo::prepare(&vault).map_err(ApiError::internal)).await?;
    if has.is_some() {
        return Err(ApiError::busy(
            "Its runner COO already has a conversation: send to that one.",
        ));
    }
    to_the_end(new_chat_once(
        ctx.clone(),
        device.device_id.clone(),
        crate::workspaces::MAIN_AGENT_ID.to_string(),
        text,
        Vec::new(),
        id,
        None,
        None,
        super::controls::Selection::default(),
    ))
    .await
}

/// A2: `GET /v1/machine/usage`, this Hollow's account snapshot
/// (`docs/phone-api/machines.md`), so another Mac spreading runs over the
/// same logins doesn't pick the account this one just picked.
async fn machine_usage(ctx: &Arc<Ctx>) -> Result<Reply, ApiError> {
    let ctx = ctx.clone();
    let usage = blocking(move || {
        let info = ctx
            .machine
            .info(&ctx.mac_name, None)
            .map_err(ApiError::unavailable)?;
        let mut accounts = ctx.host.machine_usage();
        accounts.truncate(wire::USAGE_ACCOUNTS_MAX);
        Ok(wire::MachineUsage {
            usage_api: wire::USAGE_API,
            machine_id: info.machine_id,
            observed_at: crate::peer_usage::rfc3339(chrono::Utc::now()),
            accounts,
        })
    })
    .await?;
    Ok(json(200, &usage))
}

/// `POST /v1/machines/pair`: another Mac pairs with the code the machine
/// window shows (the same rules as a phone's: 10 minutes or one use, five
/// wrong codes close it). The token goes back once and only its SHA-256 is
/// kept; the same Mac pairing again replaces its old device.
async fn machine_pair(ctx: &Arc<Ctx>, body: &[u8]) -> Result<Reply, ApiError> {
    let request: wire::MachinePairRequest = parse_body(body)?;
    let code = request
        .code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| ApiError::bad_request("The pairing code is missing."))?
        .to_string();
    let machine = request
        .machine
        .ok_or_else(|| ApiError::bad_request("machine is missing."))?;
    let machine_id = machine
        .machine_id
        .as_deref()
        .map(str::trim)
        .filter(|id| super::store::valid_machine_id(id))
        .ok_or_else(|| {
            ApiError::bad_request("machine.machineId must be m- and 16 lower-case hex digits.")
        })?
        .to_string();
    let name = plain(machine.name.as_deref(), 80)
        .ok_or_else(|| ApiError::bad_request("machine.name must be 1 to 80 characters."))?;
    if let Some(slug) = machine.slug.as_deref() {
        if crate::context_roots::valid_slug(slug).is_err() {
            return Err(ApiError::bad_request(
                "machine.slug must be a machine name: lower-case letters, digits and single dashes.",
            ));
        }
    }
    if request.client_version.is_some() && plain(request.client_version.as_deref(), 80).is_none() {
        return Err(ApiError::bad_request(
            "clientVersion must be at most 80 characters.",
        ));
    }
    let now = chrono::Utc::now();
    match ctx.machine_pairing.attempt(&code, now) {
        Attempt::Closed => Err(ApiError::machine_pairing_closed()),
        Attempt::Wrong { attempts_left } => {
            ctx.host.changed();
            Err(ApiError::wrong_code(attempts_left))
        }
        Attempt::Paired => {
            // Which Mac this is, after the code (an unauthenticated caller
            // learns nothing before it): a Hollow that can't say pairs
            // nothing, and its own error (a path, serde's words) stays here.
            let info = {
                let ctx = ctx.clone();
                blocking(move || {
                    ctx.machine
                        .info(&ctx.mac_name, Some(&ctx.vault))
                        .map_err(|_| ApiError::unavailable(MACHINE_UNKNOWN))
                })
                .await?
            };
            let (device, token, replaced) = {
                let ctx = ctx.clone();
                blocking(move || {
                    ctx.store
                        .pair_machine(&name, &machine_id, now)
                        .map_err(ApiError::internal)
                })
                .await?
            };
            if let Some(old) = replaced {
                ctx.live.close_device(&old);
            }
            // A Mac paired with the iPhone switch off starts copying too.
            ctx.live.follow(|| ctx.store.viewers_ready());
            ctx.host.changed();
            Ok(json(
                200,
                &wire::MachinePairResponse {
                    token,
                    device_id: device.device_id,
                    machine: info,
                    api: wire::API,
                    peer_api: wire::PEER_API,
                },
            ))
        }
    }
}

/// `DELETE /v1/machines/me`: the calling Mac unpairs itself (its Forget);
/// its streams close and its token is dead at once.
async fn machine_forget(ctx: &Arc<Ctx>, device: &Device) -> Result<Reply, ApiError> {
    commit_point(&ctx.store, &device.device_id, "machine-forget")?;
    {
        let (store, id) = (ctx.store.clone(), device.device_id.clone());
        blocking(move || store.revoke(&id).map_err(ApiError::internal)).await?;
    }
    ctx.live.close_device(&device.device_id);
    ctx.live.follow(|| ctx.store.viewers_ready());
    ctx.host.changed();
    Ok(Reply::Empty(204))
}

// ── M5: moving a chat here (`chat_move`, docs/phone-api/machines.md) ─────

impl From<crate::chat_move::MoveError> for ApiError {
    fn from(error: crate::chat_move::MoveError) -> Self {
        match error.code {
            "internal" => ApiError::internal(error.message),
            code => ApiError::new(error.status(), code, error.message),
        }
    }
}

/// The calling Mac's `machineId` (a machine device always has one).
fn caller_machine(device: &Device) -> Result<String, ApiError> {
    device
        .machine_id
        .clone()
        .ok_or_else(ApiError::machines_only)
}

/// `POST /v1/machines/moves/{id}/parts/{n}`: one part of a move's bundle,
/// kept in memory until the commit.
async fn move_part(
    ctx: &Arc<Ctx>,
    device: &Device,
    id: &str,
    part: &str,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let caller = caller_machine(device)?;
    let n: usize = part
        .parse()
        .map_err(|_| ApiError::bad_request("The part number must be a whole number."))?;
    let request: crate::chat_move::PartBody = parse_body(body)?;
    commit_point(&ctx.store, &device.device_id, "move-part")?;
    let have =
        crate::chat_move::staging(&ctx.vault).put(id, &caller, n, &request, Instant::now())?;
    Ok(json(200, &serde_json::json!({"moveId": id, "parts": have})))
}

/// `POST /v1/machines/moves/{id}/commit`: join the parts and import the
/// chat; a commit repeated after it landed answers again with `already`.
async fn move_commit(
    ctx: &Arc<Ctx>,
    device: &Device,
    id: &str,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let caller = caller_machine(device)?;
    let request: crate::chat_move::CommitBody = parse_body(body)?;
    let (ctx2, device_id, id) = (ctx.clone(), device.device_id.clone(), id.to_string());
    let imported = blocking(move || {
        commit_point(&ctx2.store, &device_id, "move-commit")?;
        crate::chat_move::commit(&ctx2.vault, &id, &caller, &request, chrono::Utc::now())
            .map_err(ApiError::from)
    })
    .await?;
    if !imported.already {
        ctx.host.changed();
    }
    Ok(json(200, &imported))
}

/// `GET /v1/machines/moves/{id}`: whether a move landed here.
async fn move_get(ctx: &Arc<Ctx>, device: &Device, id: &str) -> Result<Reply, ApiError> {
    let caller = caller_machine(device)?;
    let (vault, id) = (ctx.vault.clone(), id.to_string());
    let status = blocking(move || {
        crate::chat_move::move_status(&vault, &id, &caller).map_err(ApiError::from)
    })
    .await?;
    Ok(json(200, &status))
}

/// `DELETE /v1/machines/moves/{id}`: call a move off. Before its commit a
/// tombstone refuses the commit; after it the chat goes again unless it
/// already ran here (409 `started`: the calling Mac finishes instead).
async fn move_cancel(ctx: &Arc<Ctx>, device: &Device, id: &str) -> Result<Reply, ApiError> {
    let caller = caller_machine(device)?;
    let (ctx2, device_id, id) = (ctx.clone(), device.device_id.clone(), id.to_string());
    let done = blocking(move || {
        commit_point(&ctx2.store, &device_id, "move-cancel")?;
        crate::chat_move::cancel_move_in(&ctx2.vault, &id, &caller, chrono::Utc::now())
            .map_err(ApiError::from)
    })
    .await?;
    if done == crate::chat_move::Cancelled::Removed {
        ctx.host.changed();
    }
    Ok(json(200, &serde_json::json!({"cancelled": done})))
}

async fn list(ctx: &Arc<Ctx>, device: &Device, query: &str) -> Result<Reply, ApiError> {
    let machine = device.kind == DeviceKind::Machine;
    let limit = bounded(query, "limit", 50, 200)?;
    let after = match query_param(query, "cursor").filter(|c| !c.is_empty()) {
        None => None,
        Some(cursor) => Some(
            crate::page_cursor::Keyset::decode(&cursor)
                .map_err(|_| ApiError::bad_request("That cursor isn't one Hollow gave out."))?,
        ),
    };
    let ctx = ctx.clone();
    let page = blocking(move || {
        let after_key = after
            .as_ref()
            .map(|k| (k.at.as_deref().unwrap_or(""), k.id.as_str()));
        // One Main agent conversation (2026-11-07): its current one stands for
        // the Main agent; the earlier ones open from it (`conversation_chain`).
        let keep = crate::chat_chain::list_filter(&ctx.vault);
        let (rows, more) =
            crate::session_store::list_sessions_after(&ctx.vault, keep, after_key, limit)
                .map_err(ApiError::internal)?;
        let next_cursor = rows
            .last()
            .filter(|_| more)
            .map(|s| crate::page_cursor::Keyset::new(Some(&s.last_activity_at), &s.key).encode());
        let mut sessions = session_rows(&ctx, &rows).map_err(ApiError::internal)?;
        if machine {
            mark_moved(&ctx.vault, &mut sessions);
        }
        Ok(wire::SessionPage {
            sessions,
            next_cursor,
        })
    })
    .await?;
    Ok(json(200, &page))
}

async fn detail(
    ctx: &Arc<Ctx>,
    device: &Device,
    key: &str,
    query: &str,
) -> Result<Reply, ApiError> {
    let machine = device.kind == DeviceKind::Machine;
    let limit = bounded(query, "limit", 20, 100)?;
    let before = match query_param(query, "before") {
        None => None,
        Some(raw) => Some(
            raw.trim()
                .parse::<u32>()
                .map_err(|_| ApiError::bad_request("before must be a turn number."))?,
        ),
    };
    // The chat's mode below is the window's, so its record must be current.
    ctx.settle_modes().await;
    let ctx = ctx.clone();
    let key = key.to_string();
    let detail = blocking(move || {
        let summary = live_chat(&ctx.vault, &key)?;
        let conversation = crate::session_store::read_conversation(&ctx.vault, &key)
            .map_err(ApiError::internal)?;
        let mut ords: Vec<u32> = conversation
            .turns
            .iter()
            .filter_map(|t| ord_of(&key, &t.id))
            .filter(|ord| before.is_none_or(|b| *ord < b))
            .collect();
        ords.sort_unstable();
        ords.dedup();
        let has_more = ords.len() > limit;
        let keep: HashSet<u32> = ords.iter().rev().take(limit).copied().collect();
        let cards =
            crate::interactions::list_for_session(&ctx.vault, &key).map_err(ApiError::internal)?;
        // The newest card each turn posted (an older one was superseded).
        let by_ord: HashMap<u32, &ChatInteraction> = cards
            .iter()
            .filter(|c| c.turn_ord > 0)
            .map(|c| (c.turn_ord as u32, c))
            .collect();
        let mut turns: Vec<wire::Turn> = conversation
            .turns
            .iter()
            .filter(|t| ord_of(&key, &t.id).is_some_and(|ord| keep.contains(&ord)))
            .filter_map(|t| turn_of(&key, t, &by_ord))
            .collect();
        // v1.2: the files each user turn carried.
        attachments::fill(&ctx.vault, &key, &summary.cwd, &mut turns);
        let mut session =
            session_rows(&ctx, std::slice::from_ref(&summary)).map_err(ApiError::internal)?;
        if machine {
            mark_moved(&ctx.vault, &mut session);
        }
        let session = session
            .pop()
            .ok_or_else(|| ApiError::internal("the chat's row"))?;
        // The mode a send from the phone runs in, which is the one the
        // window shows for this chat.
        let chosen = chosen_mode(
            &ctx.store,
            &key,
            !ctx.modes.has_window(),
            &summary.agent,
            conversation.permission_mode.as_deref(),
        )?;
        let permission = chat_permission(
            &summary.agent,
            chosen.as_deref(),
            conversation.permission_mode.as_deref(),
        );
        Ok(wire::SessionDetail {
            session,
            permission_mode: Some(permission.mode),
            turns,
            has_more,
            live: ctx.live.snapshot(&key),
        })
    })
    .await?;
    Ok(json(200, &detail))
}

/// The sentence a queued turn's wait reads as (the window's "Queued ·
/// waiting for …").
fn waiting_sentence(waiting_for: Option<&str>) -> Option<String> {
    let what = waiting_for.map(str::trim).filter(|w| !w.is_empty())?;
    let what = what.trim_end_matches('.');
    Some(format!("Waiting for {what}."))
}

/// Whether live queue row `id` reads Working: claimed or running, or queued
/// with nothing truly holding it back ([`crate::run_pump::item_with_reason`]).
async fn working(vault: &Path, id: i64) -> bool {
    let vault = vault.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let rows = crate::run_queue::live_rows(&vault).ok()?;
        let row = rows.iter().find(|row| row.id == id)?;
        let facts = crate::run_pump::QueueFacts::read(&vault, &rows);
        Some(
            crate::run_pump::item_with_reason(&vault, row, None, &facts, chrono::Utc::now())
                .reads_working(),
        )
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(false)
}

/// Whether queue row `id` is running yet, waiting up to [`START_GRACE`].
async fn started_soon(vault: &Path, id: i64) -> bool {
    let deadline = Instant::now() + START_GRACE;
    loop {
        let vault = vault.to_path_buf();
        let running = tokio::task::spawn_blocking(move || {
            crate::run_queue::get(&vault, id)
                .ok()
                .flatten()
                .is_some_and(|row| row.status == crate::run_queue::RowStatus::Running)
        })
        .await
        .unwrap_or(false);
        if running || Instant::now() >= deadline {
            return running;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The modes the window's permission picker offers (`knownMode` in
/// `src/permModes.ts`).
pub(crate) const WINDOW_MODES: [&str; 4] = ["acceptEdits", "auto", "plan", "bypassPermissions"];

/// The mode a phone's send to a chat runs in, and whether it may resume the
/// chat's native session.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatPermission {
    pub mode: String,
    pub resume_native: bool,
}

/// Decided as the window decides them for the chat it shows
/// (`conversationPermission` in `src/permModes.ts`): the person's choice for
/// this chat (`chosen`, which the window reports), else the mode its last
/// turn recorded, else Edits only; the native session only when the
/// recorded mode is of the same class as that mode
/// (`chat_resume::mode_class`: Plan reads only and the other modes act;
/// Grok's sandbox profiles are each their own class), so a phone never
/// resumes a session started with broader permissions than the chat shows.
pub(crate) fn chat_permission(
    agent: &str,
    chosen: Option<&str>,
    recorded: Option<&str>,
) -> ChatPermission {
    let known = |mode: &&str| WINDOW_MODES.contains(mode);
    let recorded = recorded.filter(known);
    let mode = chosen.filter(known).or(recorded).unwrap_or("acceptEdits");
    // An agent Hollow doesn't name has the plain classes, as in the window.
    let kind = crate::agent::AgentKind::from_opt(agent).unwrap_or(crate::agent::AgentKind::Claude);
    let class = |m: &str| crate::chat_resume::mode_class(kind, m);
    ChatPermission {
        mode: mode.to_string(),
        resume_native: recorded.is_some_and(|r| class(r) == class(mode)),
    }
}

/// The mode the person chose for chat `key` ([`Store::chat_mode`]). With
/// no window (`hollow serve`), a chat with no choice and no recorded mode
/// takes the stored mode for new chats (inherited onto its agent), else
/// Plan: never [`chat_permission`]'s Edits-only fallback, which is the
/// window's.
pub(crate) fn chosen_mode(
    store: &Store,
    key: &str,
    windowless: bool,
    agent: &str,
    recorded: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let chosen = store.chat_mode(key).map_err(ApiError::unavailable)?;
    if !windowless {
        return Ok(chosen);
    }
    // A choice the window doesn't show would fall to Edits only too.
    let chosen = chosen.filter(|mode| WINDOW_MODES.contains(&mode.as_str()));
    let recorded = recorded.filter(|mode| WINDOW_MODES.contains(mode));
    if chosen.is_some() || recorded.is_some() {
        return Ok(chosen);
    }
    let kind = crate::agent::AgentKind::from_opt(agent).unwrap_or(crate::agent::AgentKind::Claude);
    let stored = stored_new_chat(store).map(|NewChatMode { mode, agent }| {
        let from = agent.as_deref().and_then(crate::agent::AgentKind::from_opt);
        crate::agent::inherited_mode(from, kind, &mode)
    });
    // A mode the window doesn't show would fall back to Edits only.
    let mode = stored
        .filter(|mode| WINDOW_MODES.contains(mode))
        .unwrap_or(HEADLESS_DEFAULT_MODE);
    Ok(Some(mode.to_string()))
}

/// The native session a send resumes: the chat's newest one, when its mode
/// class allows it and it can be resumed natively (the window's
/// `resumable === 'native'`).
fn native_session(summary: &AgentSessionSummary, permission: &ChatPermission) -> Option<String> {
    (permission.resume_native && summary.resumable == "native")
        .then(|| summary.harness_session_id.clone())
        .flatten()
}

/// v1.2: the answer a repeated `clientMessageId` from this phone already
/// got, if any. The canvas answer route asks before its own checks, so a
/// retry after its answer went gets that answer, not "already answered".
pub(super) async fn remembered(ctx: &Ctx, device: &str, id: &str) -> Option<Reply> {
    let mut sends = ctx.sends.lock().await;
    repeated(&mut sends, device, id)
}

/// A repeat of a send this phone made in the last [`IDEMPOTENT_FOR`].
pub(super) fn repeated(
    sends: &mut HashMap<(String, String), (Instant, u16, Bytes)>,
    device: &str,
    id: &str,
) -> Option<Reply> {
    sends.retain(|_, (at, _, _)| at.elapsed() < IDEMPOTENT_FOR);
    sends
        .get(&(device.to_string(), id.to_string()))
        .map(|(_, status, body)| Reply::Json {
            status: *status,
            body: body.clone(),
        })
}

pub(super) fn remember(
    sends: &mut HashMap<(String, String), (Instant, u16, Bytes)>,
    device: &str,
    id: &str,
    status: u16,
    value: &impl Serialize,
) -> Reply {
    let body = Bytes::from(serde_json::to_vec(value).unwrap_or_default());
    if sends.len() >= IDEMPOTENT_KEPT {
        if let Some(oldest) = sends
            .iter()
            .min_by_key(|(_, (at, _, _))| *at)
            .map(|(k, _)| k.clone())
        {
            sends.remove(&oldest);
        }
    }
    sends.insert(
        (device.to_string(), id.to_string()),
        (Instant::now(), status, body.clone()),
    );
    Reply::Json { status, body }
}

/// A send or stop Hollow refused, as the phone hears it: a chat that can't
/// take another turn now is `busy`, a Hollow that takes no turns now is
/// `unavailable`.
fn send_error(host: &dyn Host, error: String) -> ApiError {
    if error == super::store::REVOKED_TEXT {
        return ApiError::unauthorized();
    }
    if error.contains(crate::chat::CHAT_ALREADY_QUEUED) {
        return ApiError::busy(BUSY_MESSAGE);
    }
    if host.accepting().is_err() {
        return ApiError::unavailable(error);
    }
    ApiError::internal(error)
}

/// The queued-or-running answer for an accepted turn (a phone's send is
/// never held): `running` once it runs or while nothing truly holds it back
/// (the window's Working, 2026-11-17), `queued` only for a real wait.
async fn accepted_state(ctx: &Ctx, accepted: &SendAccepted) -> (String, Option<String>) {
    if started_soon(&ctx.vault, accepted.queue_id).await
        || working(&ctx.vault, accepted.queue_id).await
    {
        ("running".into(), None)
    } else {
        (
            "queued".into(),
            waiting_sentence(accepted.waiting_for.as_deref()),
        )
    }
}

/// Run a send to its end even when the phone hangs up. hyper drops a
/// request's handler when its connection closes while the answer is still
/// pending (a phone losing its network, `tailscale serve` closing the
/// upstream), which would cut a send off after Hollow queued the turn but
/// before its answer was remembered, or inside the queueing itself; a retry
/// with the same `clientMessageId` would then queue the message again. So
/// the work runs in its own task, which the handler waits for and a dropped
/// handler leaves running. A task that panics answers the generic internal
/// error ([`task_failed`]).
pub(crate) async fn to_the_end(
    work: impl std::future::Future<Output = Result<Reply, ApiError>> + Send + 'static,
) -> Result<Reply, ApiError> {
    tokio::spawn(work).await.map_err(task_failed)?
}

async fn send(ctx: &Arc<Ctx>, device: &Device, key: &str, body: &[u8]) -> Result<Reply, ApiError> {
    let request: wire::SendRequest = parse_body(body)?;
    // M2: another Mac sends text only, in the mode this Mac keeps for the
    // chat: no attachments (files stay on their own Mac) and no controls.
    if device.kind == DeviceKind::Machine {
        if request.attachment_ids.is_some() {
            return Err(ApiError::bad_request(
                "Another Mac sends text only: attachmentIds isn't taken.",
            ));
        }
        if request.controls.is_some() {
            return Err(ApiError::bad_request(
                "Another Mac's send runs as this Mac keeps the chat: controls isn't taken.",
            ));
        }
    }
    let controls = request.controls.unwrap_or_default();
    controls.validate()?;
    let files = attachments::ids_of(request.attachment_ids.as_deref())?;
    let text = message_text(request.text.as_deref(), !files.is_empty())?;
    let id = message_id(request.client_message_id.as_deref())?;
    to_the_end(send_once(
        ctx.clone(),
        device.device_id.clone(),
        key.to_string(),
        text,
        files,
        id,
        controls,
    ))
    .await
}

/// The phone `device_id` is still paired: checked again right before a
/// mutation that may have waited on a lock (`sends`, `uploads_saving`) after
/// the server's token checks (Q-R20). A phone revoked meanwhile gets the 401
/// an unknown token gets.
pub(super) fn device_still_paired(ctx: &Ctx, device_id: &str) -> Result<(), ApiError> {
    commit_point(&ctx.store, device_id, "paired")
}

/// The commit-point check (R1): the one place a route asks whether its phone
/// is still paired, just before an effect lands and after every wait that
/// can be long. A phone revoked since its token was checked gets the 401 an
/// unknown token gets and the effect does not land. Sync, so a closure on a
/// blocking thread can call it right before its write. `point` names the
/// commit point for the tests' [`checkpoint`] seam.
pub(super) fn commit_point(
    store: &Store,
    device_id: &str,
    point: &'static str,
) -> Result<(), ApiError> {
    #[cfg(test)]
    checkpoint(device_id, point);
    #[cfg(not(test))]
    let _ = point;
    store
        .ensure_live(device_id)
        .map_err(|not_live| match not_live {
            NotLive::Revoked => ApiError::unauthorized(),
            NotLive::Unreadable(error) => ApiError::unavailable(error),
        })
}

/// What code that returns `String` errors (the send path) calls at its
/// commit point; [`send_error`] turns a refusal back into the 401.
pub(crate) type Gate = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// A [`Gate`] for `device_id`'s request at the commit point `point`.
pub(super) fn gate_for(store: &Arc<Store>, device_id: &str, point: &'static str) -> Gate {
    let (store, device_id) = (store.clone(), device_id.to_string());
    Arc::new(move || {
        #[cfg(test)]
        checkpoint(&device_id, point);
        #[cfg(not(test))]
        let _ = point;
        store.ensure_live(&device_id).map_err(|e| e.to_string())
    })
}

/// Tests: what runs at a commit point just before its check, once, keyed by
/// phone and point, so a test can revoke the phone in the gap a long wait
/// leaves.
#[cfg(test)]
type Hook = Box<dyn FnOnce() + Send>;
#[cfg(test)]
pub(super) static CHECKPOINTS: std::sync::LazyLock<Mutex<HashMap<(String, &'static str), Hook>>> =
    std::sync::LazyLock::new(Mutex::default);

#[cfg(test)]
fn checkpoint(device_id: &str, point: &'static str) {
    let hook = CHECKPOINTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(device_id.to_string(), point));
    if let Some(hook) = hook {
        hook();
    }
}

/// [`send`]'s work, from the repeat check to the remembered answer. `files`
/// are the attachment ids the message names (v1.2), found again in the chat's
/// folder before anything is queued. v1.2's canvas answer sends through it
/// too (`canvases::answer`), with no files.
pub(super) async fn send_once(
    ctx: Arc<Ctx>,
    device_id: String,
    key: String,
    text: String,
    files: Vec<String>,
    id: String,
    controls: super::controls::Selection,
) -> Result<Reply, ApiError> {
    let key = key.as_str();
    // The send runs in the mode the window shows, so its record must be
    // current.
    ctx.settle_modes().await;
    // One send at a time, so a repeat waits for the first and gets its
    // answer instead of queueing twice.
    #[cfg(test)]
    ctx.sends_waiting
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut sends = ctx.sends.lock().await;
    if let Some(reply) = repeated(&mut sends, &device_id, &id) {
        return Ok(reply);
    }
    let host = ctx.host.clone();
    let effort = controls.effort.clone();
    let windowless = !ctx.modes.has_window();
    let (summary, input) = {
        let vault = ctx.vault.clone();
        let store = ctx.store.clone();
        let key = key.to_string();
        let turn = turn_id(&device_id, &id);
        blocking(move || {
            let summary = live_chat(&vault, &key)?;
            let kind = host
                .agent_for(
                    controls
                        .agent_kind()?
                        .or_else(|| crate::agent::AgentKind::from_opt(&summary.agent)),
                )
                .map_err(ApiError::bad_request)?;
            let prior_model = crate::wake_me::wake_model(&vault, &key, &summary.agent);
            let same_agent = kind.as_str() == summary.agent;
            let model = controls
                .model
                .clone()
                .or_else(|| same_agent.then(|| prior_model.clone()).flatten());
            let recorded =
                crate::session_store::last_permission(&vault, &key).map_err(ApiError::internal)?;
            let chosen = chosen_mode(
                &store,
                &key,
                windowless,
                &summary.agent,
                recorded.as_deref(),
            )?;
            // The mode the window shows for this chat; across a mode class
            // a fresh native session, as the window starts one.
            let inherited =
                chat_permission(&summary.agent, chosen.as_deref(), recorded.as_deref()).mode;
            // Onto another agent the chat's mode is inherited, not picked for
            // it, so Grok never gets auto (sandbox off) and Antigravity never
            // its unavailable auto (`agent::inherited_mode`). A mode the
            // phone names may match or lower the chat's ceiling, never raise it
            // (`controls.permission`).
            let ceiling = inherited.clone();
            let inherited = if same_agent {
                inherited
            } else {
                crate::agent::inherited_mode(
                    crate::agent::AgentKind::from_opt(&summary.agent),
                    kind,
                    &inherited,
                )
                .to_string()
            };
            let selected_mode = controls.permission(kind, &inherited, &ceiling)?;
            let permission =
                chat_permission(kind.as_str(), Some(&selected_mode), recorded.as_deref());
            let attached = attachments::blocks(&attachments::resolve(&summary.cwd, &files)?);
            let input = ChatSendInput {
                prompt: format!("{CHAT_INTENT}{text}{attached}"),
                session_id: if same_agent && model == prior_model {
                    native_session(&summary, &permission)
                } else {
                    None
                },
                session_key: Some(key.clone()),
                cwd: Some(summary.cwd.clone()),
                model,
                permission_mode: Some(permission.mode),
                agent: Some(kind.as_str().to_string()),
                client_turn_id: Some(turn),
                app_depth: 0,
                ultracode: false,
                canvases: None,
                continued_from: Vec::new(),
                unattended: false,
                // The person's own words, as the composer marks them.
                typed: true,
                host_asker: None,
                canned: None,
                worker: false,
            };
            Ok((summary, input))
        })
        .await?
    };
    // The phone may have been revoked while this waited for `sends`.
    device_still_paired(&ctx, &device_id)?;
    ctx.host.accepting().map_err(ApiError::unavailable)?;
    let accepted = ctx
        .host
        .send_controlled(
            input,
            effort,
            true,
            gate_for(&ctx.store, &device_id, "send-held-order"),
        )
        .await
        .map_err(|e| send_error(ctx.host.as_ref(), e))?;
    let (state, waiting_for) = accepted_state(&ctx, &accepted).await;
    let workspace = ctx
        .homes()
        .ok()
        .and_then(|h| h.workspace(&summary.cwd))
        .map(|w| w.id);
    // A forwarded send runs in the chain's current conversation.
    ctx.host
        .announce(&accepted.session_key, workspace.as_deref(), &summary.cwd);
    ctx.live.touched(&accepted.session_key);
    Ok(remember(
        &mut sends,
        &device_id,
        &id,
        202,
        &wire::SendAccepted {
            accepted: true,
            state,
            queue_id: accepted.queue_id,
            waiting_for,
            // Never held: a busy chat is 409 instead.
            held_id: None,
            forwarded_to: accepted
                .forwarded_from
                .is_some()
                .then(|| accepted.session_key.clone()),
        },
    ))
}

async fn new_chat(ctx: &Arc<Ctx>, device: &Device, body: &[u8]) -> Result<Reply, ApiError> {
    let request: wire::NewChatRequest = parse_body(body)?;
    let controls = request.controls.unwrap_or_default();
    controls.validate()?;
    let workspace_id = request
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .ok_or_else(|| ApiError::bad_request("workspaceId is missing."))?
        .to_string();
    let files = attachments::ids_of(request.attachment_ids.as_deref())?;
    let text = message_text(request.text.as_deref(), !files.is_empty())?;
    let id = message_id(request.client_message_id.as_deref())?;
    let asked_agent = match request
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        None => None,
        Some(agent) => Some(crate::agent::AgentKind::from_opt(agent).ok_or_else(|| {
            ApiError::bad_request(format!("{agent} isn't an agent Hollow runs."))
        })?),
    };
    let runs_on = match request
        .runs_on
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        None => None,
        Some(asked) => match crate::agent_settings::place_choice(asked) {
            // Stored and compared as `runner` from here on.
            Some(place) => Some(place.to_string()),
            None => {
                return Err(ApiError::bad_request(format!(
                    "runsOn must be this-mac, runner or mac-mini, not {asked}."
                )))
            }
        },
    };
    to_the_end(new_chat_once(
        ctx.clone(),
        device.device_id.clone(),
        workspace_id,
        text,
        files,
        id,
        asked_agent,
        runs_on,
        controls,
    ))
    .await
}

/// v1.1: a new chat's `runsOn`, as the Runs on control in a new chat's
/// empty state sets it in Hollow's window (`RunsOnControl.tsx`): Runs on is
/// the workspace's, so when the chat would run elsewhere the workspace's own
/// choice changes (`build_set_runs_on` for a build workspace,
/// `workspace_set_runs_on` for a vault workspace, whose This Mac is its
/// default); when it already runs there nothing changes.
pub(crate) fn place_new_chat(
    vault: &Path,
    workspace_id: &str,
    cwd: &str,
    agent: crate::agent::AgentKind,
    wanted: &str,
    reads: crate::remote_session::PlaceReads,
) -> Result<Option<PlacementUndo>, ApiError> {
    use crate::agent::AgentKind;
    let movable = matches!(agent, AgentKind::Claude | AgentKind::Codex);
    let builtin = crate::workspaces::is_builtin(workspace_id);
    // `this-mac` or the runner by either name (M6); written as `runner`.
    let to_runner = crate::agent_settings::is_runner(wanted);
    let wanted = if to_runner {
        crate::agent_settings::RUNNER
    } else {
        wanted
    };
    if to_runner {
        if builtin {
            return Err(ApiError::bad_request(
                "Chats and the COO always run on this Mac.",
            ));
        }
        if (reads.runner)().is_none() {
            return Err(ApiError::bad_request(
                "No remote runner is set up on this Mac.",
            ));
        }
        if !movable {
            return Err(ApiError::bad_request(
                "Only Claude and Codex chats run on the remote runner.",
            ));
        }
    }
    if builtin || !movable {
        return Ok(None);
    }
    let folder = Path::new(cwd);
    let there = crate::remote_session::chosen_runner(vault, folder, agent, reads);
    if there == to_runner {
        return Ok(None);
    }
    if let Some(id) = crate::build_workspaces::workspace_for_worktree(vault, folder) {
        // As stored, so an undo puts an old `mac-mini` back as it was.
        let before = crate::build_workspaces::stored_runs_on(vault, &id);
        crate::build_workspaces::set_runs_on(vault, &id, Some(wanted))
            .map_err(ApiError::internal)?;
        Ok(Some(PlacementUndo::Build {
            id,
            before,
            wrote: Some(wanted.to_string()),
        }))
    } else if let Some(id) = crate::remote_folder::workspace_of(vault, folder) {
        let before = crate::remote_folder::stored_runs_on(vault, &id);
        let wrote = to_runner.then(|| crate::agent_settings::RUNNER.to_string());
        crate::remote_folder::set_runs_on(vault, &id, wrote.as_deref())
            .map_err(ApiError::internal)?;
        Ok(Some(PlacementUndo::Folder { id, before, wrote }))
    } else {
        Err(ApiError::bad_request(
            "This workspace's chats always run on this Mac.",
        ))
    }
}

/// What [`place_new_chat`] changed, to put back when the chat's send is not
/// accepted: the phone's `runsOn` must not outlive a send that failed (ENG-223).
/// `before` is the value as stored (an old `mac-mini` kept, M6 review).
/// `wrote` is the value the placement stored: the undo applies only while the
/// setting still holds it, so a choice the person made meanwhile (in the
/// window, or from another phone) is never overwritten.
pub(crate) enum PlacementUndo {
    Build {
        id: String,
        before: Option<String>,
        wrote: Option<String>,
    },
    Folder {
        id: String,
        before: Option<String>,
        wrote: Option<String>,
    },
}

impl PlacementUndo {
    /// Best effort: a failed restore leaves the new choice, as before.
    pub(crate) fn restore(self, vault: &Path) {
        match self {
            Self::Build { id, before, wrote } => {
                let now = crate::build_workspaces::binding_for(vault, &id).map(|b| b.runs_on);
                if now == Some(wrote) {
                    let _ = crate::build_workspaces::restore_runs_on(vault, &id, before.as_deref());
                }
            }
            Self::Folder { id, before, wrote } => {
                if crate::remote_folder::runs_on_for(vault, &id) == wrote {
                    let _ = crate::remote_folder::restore_runs_on(vault, &id, before.as_deref());
                }
            }
        }
    }
}

/// [`new_chat`]'s work, from the repeat check to the remembered answer.
async fn new_chat_once(
    ctx: Arc<Ctx>,
    device_id: String,
    workspace_id: String,
    text: String,
    files: Vec<String>,
    id: String,
    asked_agent: Option<crate::agent::AgentKind>,
    runs_on: Option<String>,
    controls: super::controls::Selection,
) -> Result<Reply, ApiError> {
    // The chat starts in the window's mode for new chats.
    ctx.settle_modes().await;
    let mut sends = ctx.sends.lock().await;
    if let Some(reply) = repeated(&mut sends, &device_id, &id) {
        return Ok(reply);
    }
    let cwd = {
        let vault = ctx.vault.clone();
        let workspace_id = workspace_id.clone();
        blocking(move || new_chat_folder(&vault, &workspace_id)).await?
    };
    let asked_agent = controls.agent_kind()?.or(asked_agent);
    // v1.2: the files the message names, found in the folder the chat runs in
    // before anything is queued or the workspace's Runs on changes.
    let attached = {
        let cwd = cwd.clone();
        blocking(move || Ok(attachments::blocks(&attachments::resolve(&cwd, &files)?))).await?
    };
    // Any agent may run the COO (L6): its New chat takes the asked agent too.
    let kind = ctx
        .host
        .agent_for(asked_agent)
        .map_err(ApiError::bad_request)?;
    let windowless = !ctx.modes.has_window();
    let mode = {
        // Serve has no window: the record's stored default, read now, so a
        // `set-default-mode` takes effect at once (M1 PR 3).
        let reported = if windowless {
            let store = ctx.store.clone();
            blocking(move || Ok(stored_new_chat(&store))).await?
        } else {
            ctx.modes.new_chat()
        };
        // The new-chat mode is the one the window reports for its preferred
        // agent (`reportPhoneNewChatMode`, which names that agent), so it is
        // always inherited from that agent, whatever agent the phone asked
        // for (review 2026-11-12: the backend's default agent can differ
        // from the window's): `agent::inherited_mode` never gives the chat
        // more than the mode does there (Q-R3), so Grok never gets its auto
        // (sandbox off), Antigravity never its (unavailable), and a Codex
        // chat from Antigravity's acceptEdits (read only) reads only. A
        // report that doesn't name its agent (an older window) reads the
        // most restrictive way. With no report at all the chat starts in
        // Hollow's default, its own agent's acceptEdits. A mode the phone
        // picks for this chat may match or lower that ceiling, never raise it
        // (`controls.permission`).
        let (wanted, from) = match reported {
            Some(NewChatMode { mode, agent }) => (
                mode,
                agent.as_deref().and_then(crate::agent::AgentKind::from_opt),
            ),
            // Serve with nothing stored: the most restrictive mode.
            None if windowless => (HEADLESS_DEFAULT_MODE.to_string(), Some(kind)),
            None => (crate::app_mcp::DEFAULT_MODE.to_string(), Some(kind)),
        };
        controls.permission(
            kind,
            crate::agent::inherited_mode(from, kind, &wanted),
            &wanted,
        )?
    };
    // The phone may have been revoked while this waited for `sends`.
    device_still_paired(&ctx, &device_id)?;
    ctx.host.accepting().map_err(ApiError::unavailable)?;
    // Runs on first, so the chat's first turn is placed by it.
    let mut placed: Option<PlacementUndo> = None;
    if let Some(place) = runs_on {
        let (vault, workspace_id, cwd) = (ctx.vault.clone(), workspace_id.clone(), cwd.clone());
        let (store, device_id) = (ctx.store.clone(), device_id.clone());
        let reads = ctx.host.place();
        placed = blocking(move || {
            // The workspace's Runs on is changed here: the phone must still
            // be paired when the closure gets its thread.
            commit_point(&store, &device_id, "placement")?;
            place_new_chat(&vault, &workspace_id, &cwd, kind, &place, reads)
        })
        .await?;
    }
    let input = ChatSendInput {
        prompt: format!("{CHAT_INTENT}{text}{attached}"),
        session_id: None,
        session_key: None,
        cwd: Some(cwd.clone()),
        model: controls.model,
        permission_mode: Some(mode),
        agent: Some(kind.as_str().to_string()),
        client_turn_id: Some(turn_id(&device_id, &id)),
        app_depth: 0,
        ultracode: false,
        canvases: None,
        continued_from: Vec::new(),
        unattended: false,
        typed: true,
        host_asker: None,
        canned: None,
        worker: false,
    };
    // The send is not accepted: the workspace's Runs on goes back to what it
    // was, so a refused chat leaves nothing of the phone's `runsOn`.
    let undo = |placed: Option<PlacementUndo>| {
        if let Some(placed) = placed {
            placed.restore(&ctx.vault);
        }
    };
    if let Err(e) = ctx.host.accepting() {
        undo(placed);
        return Err(ApiError::unavailable(e));
    }
    let accepted = match ctx
        .host
        .send_controlled(
            input,
            controls.effort,
            false,
            gate_for(&ctx.store, &device_id, "new-chat-held-order"),
        )
        .await
    {
        Ok(accepted) => accepted,
        Err(e) => {
            undo(placed);
            return Err(send_error(ctx.host.as_ref(), e));
        }
    };
    let (state, waiting_for) = accepted_state(&ctx, &accepted).await;
    ctx.host
        .announce(&accepted.session_key, Some(&workspace_id), &cwd);
    ctx.live.touched(&accepted.session_key);
    Ok(remember(
        &mut sends,
        &device_id,
        &id,
        202,
        &wire::NewChatAccepted {
            accepted: true,
            key: accepted.session_key.clone(),
            state,
            queue_id: accepted.queue_id,
            waiting_for,
        },
    ))
}

/// The folder a new chat in `workspace_id` runs in: Quick chats and the
/// Main agent are made on first use; another workspace must exist and not
/// be archived.
pub(super) fn new_chat_folder(vault: &Path, workspace_id: &str) -> Result<String, ApiError> {
    let missing = || ApiError::not_found("That workspace isn't in Hollow.");
    let builtin = crate::workspaces::is_quick_chats(workspace_id)
        || crate::workspaces::is_main_agent(workspace_id);
    if !builtin {
        let w = crate::workspaces::summary(vault, workspace_id).map_err(|_| missing())?;
        if crate::workspaces::is_archived_status(&w.status) {
            return Err(missing());
        }
    }
    crate::workspaces::session_dir_for(vault, workspace_id)
        .map(|dir| dir.to_string_lossy().into_owned())
        .map_err(|e| {
            if builtin {
                ApiError::internal(e)
            } else {
                missing()
            }
        })
}

async fn stop(ctx: &Arc<Ctx>, device: &Device, key: &str) -> Result<Reply, ApiError> {
    {
        let vault = ctx.vault.clone();
        let key = key.to_string();
        blocking(move || live_chat(&vault, &key).map(|_| ())).await?;
    }
    // The chat was checked on a blocking thread: a phone revoked meanwhile
    // must not stop the turn.
    commit_point(&ctx.store, &device.device_id, "stop")?;
    let stopped = ctx
        .host
        .stop(key.to_string())
        .await
        .map_err(|e| send_error(ctx.host.as_ref(), e))?;
    ctx.live.touched(key);
    Ok(json(200, &wire::Stopped { stopped }))
}

async fn workspaces(ctx: &Arc<Ctx>) -> Result<Reply, ApiError> {
    let ctx = ctx.clone();
    let rows = blocking(move || workspace_rows(&ctx).map_err(ApiError::internal)).await?;
    Ok(json(200, &wire::Workspaces { workspaces: rows }))
}

/// Quick chats, the Main agent, then the workspaces the Work page lists as
/// current, ordered as it orders them: by their newest conversation, else
/// by when the brief changed.
fn workspace_rows(ctx: &Ctx) -> Result<Vec<wire::WorkspaceRow>, String> {
    use crate::workspaces::{MAIN_AGENT_ID, MAIN_AGENT_TITLE, QUICK_CHATS_ID, QUICK_CHATS_TITLE};
    let homes = ctx.homes()?;
    // v1.1: any workspace's chats but the built-ins' may run on a runner,
    // once one is set up (Runs on applies to build and vault workspaces).
    let runner = (ctx.host.place().runner)().is_some();
    let status_of = |id: &str| {
        homes
            .info
            .get(id)
            .map(|w| w.status.clone())
            .unwrap_or_else(|| "active".into())
    };
    let mut rows = vec![
        wire::WorkspaceRow {
            id: QUICK_CHATS_ID.into(),
            title: QUICK_CHATS_TITLE.into(),
            kind: "quick".into(),
            status: status_of(QUICK_CHATS_ID),
            can_run_on_mini: false,
        },
        wire::WorkspaceRow {
            id: MAIN_AGENT_ID.into(),
            title: MAIN_AGENT_TITLE.into(),
            kind: "main".into(),
            status: status_of(MAIN_AGENT_ID),
            can_run_on_mini: false,
        },
    ];
    // Each workspace's newest conversation.
    let mut newest: HashMap<String, String> = HashMap::new();
    for row in crate::session_store::list_digest(&ctx.vault, 1_000)? {
        if let Some(id) = homes.homes.workspace_for(&row.cwd) {
            let slot = newest.entry(id).or_default();
            if row.last_activity_at > *slot {
                *slot = row.last_activity_at;
            }
        }
    }
    let mut current: Vec<&crate::workspaces::Workspace> = homes
        .info
        .values()
        .filter(|w| !crate::workspaces::is_builtin(&w.id))
        .filter(|w| {
            crate::workspaces::CURRENT_STATUSES
                .iter()
                .any(|s| w.status.to_lowercase() == *s)
        })
        .collect();
    let sort_key = |w: &crate::workspaces::Workspace| -> String {
        newest
            .get(&w.id)
            .cloned()
            .unwrap_or_else(|| w.modified.clone())
    };
    current.sort_by(|a, b| {
        sort_key(b)
            .cmp(&sort_key(a))
            .then_with(|| a.title.cmp(&b.title))
    });
    rows.extend(
        current
            .into_iter()
            .take(WORKSPACES_MAX - rows.len())
            .map(|w| wire::WorkspaceRow {
                id: w.id.clone(),
                title: w.title.clone(),
                kind: crate::workspaces::origin_kind(w).to_string(),
                status: w.status.clone(),
                can_run_on_mini: runner,
            }),
    );
    Ok(rows)
}

// ── v1.1: Waiting on you ─────────────────────────────────────────────────

/// A card's first question as a preview: its prompt, else its header.
pub(crate) fn card_preview(card: &ChatInteraction) -> Option<String> {
    let questions = questions_of(card)?;
    let first = questions.items.first()?;
    preview(&first.prompt).or_else(|| preview(&first.header))
}

/// `GET /v1/waiting`: every chat with a pending question card (the chats
/// whose `needsAnswer` is true), newest card first, at most
/// [`WAITING_MAX`]; archived chats never.
async fn waiting(ctx: &Arc<Ctx>) -> Result<Reply, ApiError> {
    let ctx = ctx.clone();
    let items = blocking(move || waiting_items(&ctx).map_err(ApiError::internal)).await?;
    Ok(json(200, &wire::Waiting { items }))
}

pub(crate) fn waiting_items(ctx: &Ctx) -> Result<Vec<wire::WaitingItem>, String> {
    let homes = ctx.homes()?;
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    // B4: the approval cards come first, one per card, oldest first.
    for card in approvals_api::open_cards() {
        let summary = card
            .session_key
            .as_deref()
            .and_then(|key| live_chat(&ctx.vault, key).ok());
        items.push(wire::WaitingItem {
            session_key: card.session_key.clone().unwrap_or_default(),
            question_id: card.id.clone(),
            title: summary
                .as_ref()
                .and_then(|s| s.title.clone())
                .filter(|t| !t.trim().is_empty()),
            workspace_title: summary
                .as_ref()
                .and_then(|s| homes.workspace(&s.cwd).map(|w| w.title)),
            kind: "approval".into(),
            since: card.at.clone(),
            preview: Some(card.title.clone()),
        });
        if items.len() >= WAITING_MAX {
            return Ok(items);
        }
    }
    // Newest first, so a chat's first card here is its newest.
    for card in crate::interactions::list_open_every(&ctx.vault)? {
        if card.status != "pending" || !seen.insert(card.session_key.clone()) {
            continue;
        }
        let Ok(summary) = live_chat(&ctx.vault, &card.session_key) else {
            continue;
        };
        items.push(wire::WaitingItem {
            session_key: card.session_key.clone(),
            question_id: card.id.clone(),
            title: summary.title.clone().filter(|t| !t.trim().is_empty()),
            workspace_title: homes.workspace(&summary.cwd).map(|w| w.title),
            kind: "question".into(),
            since: card.created_at.clone(),
            preview: card_preview(&card),
        });
        if items.len() >= WAITING_MAX {
            break;
        }
    }
    Ok(items)
}

// ── v1.1: Answering a question card ──────────────────────────────────────

/// One answer value: none, one string, or a list of strings.
fn answer_values(id: &str, value: Option<&serde_json::Value>) -> Result<Vec<String>, ApiError> {
    use serde_json::Value;
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(one)) => Ok(vec![one.clone()]),
        Some(Value::Array(many)) => many
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "The answer to `{id}` must be text or a list of texts."
                    ))
                })
            })
            .collect(),
        Some(_) => Err(ApiError::bad_request(format!(
            "The answer to `{id}` must be text, a list of texts or null."
        ))),
    }
}

/// The phone's `answers` as the answer Hollow's card Send submits
/// (`interaction_schema::Answer`): each item's id names one of the card's
/// questions as the phone shows them ([`questions_of`]); an option's id
/// chooses it, other words are the person's own (when the question takes
/// them) or a text question's text. Whether the whole answer fits the card
/// (required questions, one choice, lengths) is `interactions::resolve`'s
/// check, as for the window's Send.
pub(crate) fn hollow_answer(
    card: &ChatInteraction,
    answers: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, ApiError> {
    use crate::interaction_schema::{AnswerMode, Mode, QuestionRequest};
    use serde_json::json;
    let request: QuestionRequest =
        serde_json::from_value(card.request.clone()).map_err(|_| ApiError::question_closed())?;
    let shown = questions_of(card).ok_or_else(ApiError::question_closed)?;
    if let Some(unknown) = answers
        .keys()
        .find(|id| !shown.items.iter().any(|item| &&item.id == id))
    {
        return Err(ApiError::bad_request(format!(
            "`{unknown}` isn't a question on this card."
        )));
    }
    let values = |id: &str| answer_values(id, answers.get(id));
    match request.mode {
        Mode::Questions => {
            let mut out = serde_json::Map::new();
            for q in &request.questions {
                let given = values(&q.id)?;
                let is_option = |v: &str| q.options.iter().any(|o| o.id == v);
                let one = match q.answer_mode {
                    AnswerMode::Text => match given.as_slice() {
                        [] => json!({}),
                        [text] => json!({ "text": text }),
                        _ => {
                            return Err(ApiError::bad_request(format!(
                                "`{}` takes one text.",
                                q.id
                            )))
                        }
                    },
                    AnswerMode::Single | AnswerMode::Multi => {
                        let (chosen, own): (Vec<String>, Vec<String>) =
                            given.into_iter().partition(|v| is_option(v));
                        if own.len() > 1 {
                            return Err(ApiError::bad_request(format!(
                                "`{}` takes at most one answer in your own words.",
                                q.id
                            )));
                        }
                        let mut one = json!({ "selected": chosen });
                        if let Some(other) = own.into_iter().next() {
                            if !q.allow_custom {
                                return Err(ApiError::bad_request(format!(
                                    "`{other}` isn't one of the options for `{}`.",
                                    q.id
                                )));
                            }
                            one["other"] = json!(other);
                        }
                        one
                    }
                };
                out.insert(q.id.clone(), one);
            }
            Ok(json!({ "answers": out }))
        }
        Mode::Confirmation => match values("confirmation")?.as_slice() {
            [] => Ok(json!({})),
            [one] if one == "accept" => Ok(json!({ "accepted": true })),
            [one] if one == "reject" => Ok(json!({ "accepted": false })),
            _ => Err(ApiError::bad_request(
                "A confirmation takes accept or reject.",
            )),
        },
        Mode::Checkbox => Ok(json!({ "selected": values("checkbox")? })),
        Mode::Verdicts => {
            let mut verdicts = serde_json::Map::new();
            for item in shown.items {
                match values(&item.id)?.as_slice() {
                    [] => {}
                    [verdict] if item.options.iter().any(|o| &o.id == verdict) => {
                        verdicts.insert(item.id.clone(), json!({ "verdict": verdict }));
                    }
                    _ => {
                        return Err(ApiError::bad_request(format!(
                            "`{}` takes one of its verdicts.",
                            item.id
                        )))
                    }
                }
            }
            Ok(json!({ "verdicts": verdicts }))
        }
    }
}

/// A refusal from `interactions::resolve`, as the phone hears it: a card
/// that isn't pending, or whose file changed, is closed; an answer that
/// doesn't fit is the request's fault, in Hollow's own words.
fn resolve_error(error: String) -> ApiError {
    let after = |prefix: &str| {
        error
            .strip_prefix(prefix)
            .map(|rest| rest.trim_start_matches(':').trim().to_string())
    };
    if error.starts_with(crate::interactions::CLOSED)
        || error.starts_with(crate::interactions::STALE)
    {
        return ApiError::question_closed();
    }
    if let Some(why) = after(crate::interactions::INVALID) {
        let why = why
            .strip_prefix("invalid answer: ")
            .unwrap_or(&why)
            .to_string();
        let mut chars = why.chars();
        let why = match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => why,
        };
        return ApiError::bad_request(why);
    }
    ApiError::internal(error)
}

async fn answer(
    ctx: &Arc<Ctx>,
    device: &Device,
    key: &str,
    question: &str,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: wire::AnswerRequest = parse_body(body)?;
    let id = message_id(request.client_message_id.as_deref())?;
    let answers = request
        .answers
        .ok_or_else(|| ApiError::bad_request("answers is missing."))?;
    to_the_end(answer_once(
        ctx.clone(),
        device.device_id.clone(),
        key.to_string(),
        question.to_string(),
        answers,
        id,
    ))
    .await
}

/// [`answer`]'s work, from the repeat check to the remembered answer: what
/// the card's Send does in the window (`submitInteraction` in
/// `src/components/chat/chatSendController.ts`). Hollow resolves the card
/// against the native session the send resumes, then sends the formatted
/// answers verbatim, with no intent line, as the chat's next turn; if
/// nothing could be queued, the card is opened again, as the window's
/// `returnUnsentAnswers` does.
async fn answer_once(
    ctx: Arc<Ctx>,
    device_id: String,
    key: String,
    question: String,
    answers: serde_json::Map<String, serde_json::Value>,
    id: String,
) -> Result<Reply, ApiError> {
    // The answer turn runs in the mode the window shows for the chat.
    ctx.settle_modes().await;
    let mut sends = ctx.sends.lock().await;
    if let Some(reply) = repeated(&mut sends, &device_id, &id) {
        return Ok(reply);
    }
    // The phone may have been revoked while this waited for `sends`; the
    // card is resolved below, so check before anything is consumed.
    device_still_paired(&ctx, &device_id)?;
    // Nothing is consumed while Hollow takes no turns.
    ctx.host.accepting().map_err(ApiError::unavailable)?;
    let windowless = !ctx.modes.has_window();
    let (summary, input, answered) = {
        let vault = ctx.vault.clone();
        let store = ctx.store.clone();
        let key = key.clone();
        let id = id.clone();
        let device_id = device_id.clone();
        blocking(move || {
            let summary = live_chat(&vault, &key)?;
            let card = crate::interactions::list_for_session(&vault, &key)
                .map_err(ApiError::internal)?
                .into_iter()
                .find(|card| card.id == question && card.status == "pending")
                .ok_or_else(ApiError::question_closed)?;
            let answer = hollow_answer(&card, &answers)?;
            // As for a send: a chat with a turn queued or running is busy,
            // and the card stays open.
            if QueueView::read(&vault).state(&key) != "idle" {
                return Err(ApiError::busy(BUSY_MESSAGE));
            }
            let recorded =
                crate::session_store::last_permission(&vault, &key).map_err(ApiError::internal)?;
            let chosen = chosen_mode(
                &store,
                &key,
                windowless,
                &summary.agent,
                recorded.as_deref(),
            )?;
            let permission =
                chat_permission(&summary.agent, chosen.as_deref(), recorded.as_deref());
            let native = native_session(&summary, &permission);
            // The card is consumed here: the phone must still be paired.
            commit_point(&store, &device_id, "answer")?;
            let resolved =
                crate::interactions::resolve(&vault, &card.id, &answer, native.as_deref())
                    .map_err(resolve_error)?;
            let input = ChatSendInput {
                // Sent verbatim, as the window sends a card's answers.
                prompt: resolved.prompt,
                session_id: native,
                session_key: Some(key.clone()),
                cwd: Some(summary.cwd.clone()),
                model: crate::wake_me::wake_model(&vault, &key, &summary.agent),
                permission_mode: Some(permission.mode),
                agent: Some(summary.agent.clone()),
                client_turn_id: Some(turn_id(&device_id, &id)),
                app_depth: 0,
                ultracode: false,
                canvases: None,
                continued_from: Vec::new(),
                unattended: false,
                typed: true,
                host_asker: None,
                canned: None,
                worker: false,
            };
            Ok((summary, input, resolved.interaction))
        })
        .await?
    };
    let accepted = match ctx
        .host
        .send_unless_busy(input, gate_for(&ctx.store, &device_id, "answer-held-order"))
        .await
    {
        Ok(accepted) => accepted,
        Err(error) => {
            // Nothing was queued: the card comes back, as the window's does.
            let vault = ctx.vault.clone();
            let card = answered.id.clone();
            if let Ok(Ok(reopened)) =
                tokio::task::spawn_blocking(move || crate::interactions::reopen(&vault, &card))
                    .await
            {
                ctx.host.card_changed(&reopened);
            }
            ctx.live.touched(&key);
            return Err(send_error(ctx.host.as_ref(), error));
        }
    };
    // Before it can start: a row that ends unstarted (taken out of the
    // queue at the Mac) brings the card back, as the window's would.
    ctx.live.answers.note(accepted.queue_id, &answered.id);
    ctx.host.card_changed(&answered);
    let (state, waiting_for) = accepted_state(&ctx, &accepted).await;
    let workspace = ctx
        .homes()
        .ok()
        .and_then(|h| h.workspace(&summary.cwd))
        .map(|w| w.id);
    ctx.host.announce(&key, workspace.as_deref(), &summary.cwd);
    ctx.live.touched(&key);
    Ok(remember(
        &mut sends,
        &device_id,
        &id,
        202,
        &wire::SendAccepted {
            accepted: true,
            state,
            queue_id: accepted.queue_id,
            waiting_for,
            held_id: None,
            forwarded_to: accepted
                .forwarded_from
                .is_some()
                .then(|| accepted.session_key.clone()),
        },
    ))
}

// ── v1.1: Push ───────────────────────────────────────────────────────────

/// `POST /v1/push`: this phone's APNs device token, which Apple service it
/// belongs to and the pushes it wants; a new one replaces the old.
async fn push_register(ctx: &Arc<Ctx>, device: &Device, body: &[u8]) -> Result<Reply, ApiError> {
    let request: wire::PushRequest = parse_body(body)?;
    let token = request
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| super::push::valid_device_token(t))
        .ok_or_else(|| ApiError::bad_request("token must be the APNs device token as hex."))?
        .to_ascii_lowercase();
    let environment = match request.environment.as_deref().map(str::trim) {
        Some(env @ (super::push::PRODUCTION | super::push::DEVELOPMENT)) => env.to_string(),
        _ => {
            return Err(ApiError::bad_request(
                "environment must be production or development.",
            ))
        }
    };
    // Unknown events are left out, so a later one never breaks this app.
    let events = super::push::known_events(request.events.as_deref());
    let target = super::store::PushTarget {
        token,
        environment,
        events,
        registered_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, false),
    };
    let (store, device_id) = (ctx.store.clone(), device.device_id.clone());
    let kept = blocking(move || {
        store
            .set_push(&device_id, Some(target))
            .map_err(ApiError::internal)
    })
    .await?;
    if !kept {
        return Err(ApiError::unauthorized());
    }
    ctx.host.changed();
    Ok(Reply::Empty(204))
}

/// `DELETE /v1/push`: this phone wants no more pushes (the app's Unpair).
async fn push_forget(ctx: &Arc<Ctx>, device: &Device) -> Result<Reply, ApiError> {
    let (store, device_id) = (ctx.store.clone(), device.device_id.clone());
    let found =
        blocking(move || store.set_push(&device_id, None).map_err(ApiError::internal)).await?;
    // A phone revoked meanwhile has nothing to forget: 401, not a 204 that
    // reads as done.
    if !found {
        return Err(ApiError::unauthorized());
    }
    // v1.2: Unpair takes its Live Activity tokens along.
    ctx.activities.forget(&device.device_id, None);
    ctx.host.changed();
    Ok(Reply::Empty(204))
}

// ── v1.2: Live Activities ────────────────────────────────────────────────

/// `POST /v1/live-activity`: the push token of the Live Activity this phone
/// started for chat `sessionKey`, which Hollow then updates and ends with
/// Apple's Live Activity pushes while the app is closed. One per phone and
/// chat (a new one replaces), in memory only. A key that isn't a chat is
/// ignored with the same 204, so a 404 always means a Hollow without v1.2.
async fn live_activity_register(
    ctx: &Arc<Ctx>,
    device: &Device,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: wire::LiveActivityRequest = parse_body(body)?;
    let key = request
        .session_key
        .as_deref()
        .map(str::trim)
        .filter(|k| crate::agent_sessions::is_safe_session_key(k))
        .ok_or_else(|| ApiError::bad_request("sessionKey must be a chat's key."))?
        .to_string();
    let token = request
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| super::push::valid_device_token(t))
        .ok_or_else(|| {
            ApiError::bad_request("token must be the Live Activity's push token as hex.")
        })?
        .to_ascii_lowercase();
    let environment = match request.environment.as_deref().map(str::trim) {
        Some(env @ (super::push::PRODUCTION | super::push::DEVELOPMENT)) => env.to_string(),
        _ => {
            return Err(ApiError::bad_request(
                "environment must be production or development.",
            ))
        }
    };
    let (vault, chat) = (ctx.vault.clone(), key.clone());
    let known = blocking(move || Ok(live_chat(&vault, &chat).is_ok())).await?;
    // After the chat lookup's wait, right before the token is kept: a phone
    // revoked meanwhile must not leave a registration behind. One that
    // slips in between this check and the insert is dropped by the pusher's
    // prune of phones no longer paired (`Activities::retain_devices`).
    commit_point(&ctx.store, &device.device_id, "live-activity")?;
    if known {
        ctx.activities.register(
            &device.device_id,
            &key,
            &token,
            &environment,
            chrono::Utc::now().timestamp(),
        );
    }
    Ok(Reply::Empty(204))
}

/// `DELETE /v1/live-activity?session=<key>`: this phone's activity for that
/// chat is over, or with no `session` every one it registered. 204 whether
/// or not there was a token.
async fn live_activity_forget(
    ctx: &Arc<Ctx>,
    device: &Device,
    query: &str,
) -> Result<Reply, ApiError> {
    commit_point(&ctx.store, &device.device_id, "live-activity-forget")?;
    let session = query_param(query, "session").filter(|s| !s.is_empty());
    ctx.activities.forget(&device.device_id, session.as_deref());
    Ok(Reply::Empty(204))
}

async fn events(ctx: &Arc<Ctx>, device: &Device, query: &str) -> Result<Reply, ApiError> {
    let session = query_param(query, "session").filter(|s| !s.is_empty());
    if let Some(key) = session.clone() {
        let vault = ctx.vault.clone();
        blocking(move || live_chat(&vault, &key).map(|_| ())).await?;
    }
    let (live, store) = (ctx.live.clone(), ctx.store.clone());
    let device_id = device.device_id.clone();
    let last_event_id = query_param(query, "lastEventId").filter(|id| !id.is_empty());
    let rx = blocking(move || {
        let rx = live.open_from(
            &device_id,
            session,
            Instant::now(),
            last_event_id.as_deref(),
        );
        // A revoke takes the phone out of the record before it closes the
        // phone's streams, so a stream that opened after that close (its
        // token passed just before the revoke) finds the phone gone here.
        match store.has_device(&device_id) {
            Ok(true) => Ok(rx),
            _ => {
                live.close_device(&device_id);
                Err(ApiError::unauthorized())
            }
        }
    })
    .await?;
    Ok(Reply::Events(rx))
}
