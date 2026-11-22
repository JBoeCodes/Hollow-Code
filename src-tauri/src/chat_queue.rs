//! Queued chat turns (Paperclip PR 7b, D3; split out of `chat.rs` by B14,
//! 2026-10-16, no behaviour change): a chat send accepted as a run-queue
//! row, the pump's admission of that row, and how an executed turn ends.
//! A child of `chat` (attached with `#[path]`, as `chat_tests.rs` is), so
//! `use super::*` keeps every `chat.rs` item in reach; `chat.rs` re-exports
//! what callers use, so they keep their `chat::` paths. Items the rest of
//! `chat.rs` reads are `pub(super)`.

use super::*;

// ── Queued chat turns (Paperclip PR 7b, D3) ─────────────────────────────────
//
// A chat send is a run-queue row. `chat_send` calls [`accept_turn`], which
// checks the working folder (so `CHAT_CONTEXT_RECOVERED` is still an invoke
// error, before anything is queued), enqueues a priority-0 row and returns
// at once. The pump (`run_pump::step`) claims the row when the vault lease is
// free and calls [`admit_turn`], which takes the lease through
// `automated_history::try_begin_owned`: the only lease call on the chat
// path. `run_pump::execute` then awaits [`execute_turn`], which runs the turn
// inside that lease and returns a [`TurnEnd`] instead of emitting `done`; the
// pump ends the row, releases the chat-row gate, and only then emits `done`.
// A queued turn never preempts a running automation: it waits for the lease
// like any row, first in line (priority 0, exempt from the automation gap).

/// The run-queue label of a chat row.
const CHAT_ROW_LABEL: &str = "Chat";
/// The refusal while a chat turn is already queued or running.
pub(crate) const CHAT_ALREADY_QUEUED: &str =
    "A chat message is already queued or running; wait for it to finish, or stop it first.";

/// What the composer asked for, as `chat_send` resolved it.
#[derive(Debug, Clone)]
pub(crate) struct ChatSendRequest {
    /// The composed request: conversation, operator, page and intent blocks
    /// around the user's own text.
    pub prompt: String,
    pub session_id: Option<String>,
    /// Hollow's own session key from the previous turn; minted when absent
    /// or malformed.
    pub session_key: Option<String>,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub agent: AgentKind,
    /// The send's nonce, echoed on every event of the turn.
    pub client_turn_id: Option<String>,
    /// How many `hollow_app` session starts deep the turn runs (0: the
    /// user's own send). Its `hollow_app` server gets this depth.
    pub app_depth: u32,
    /// Track U: the chat's Ultracode switch (`effort::ULTRACODE` instead of
    /// the agent's saved effort, for this turn only).
    pub ultracode: bool,
    /// Canvas C3 wave 2 C4: the chat's own Canvases choice (`off`, `helps`,
    /// `most`), set in the model menu; `None` follows Settings, Chat,
    /// Canvases ([`crate::visual_prompt::resolve`]).
    pub canvases: Option<String>,
    /// Agent effectiveness F4 (K1 review): the earlier conversations this
    /// one continues from, oldest first (the composer's Continue from
    /// summary, or its reset when the working folder vanished). Checked at
    /// accept time ([`crate::context_compiler::continued_from_keys`]); the
    /// context compiler bundles their recorded turns on a fresh native
    /// session. A handover's successor (HB06) names its predecessor here and
    /// gets only its task state. Empty for every other send.
    pub continued_from: Vec<String>,
    /// An unattended turn Hollow queued with nobody watching (the night
    /// queue, master plan F1 + #16): its Claude account is picked like an
    /// automation's long work (away from one at 80% or more), and it counts
    /// as unattended for the Main agent's limits from admission on
    /// ([`queued_unattended`]). The person's own sends never set it.
    pub unattended: bool,
    /// The message is the person's own words, typed in a composer
    /// (`commands::chat_send`: a chat, the Main agent page's included). Only such a
    /// turn feeds post-turn learning ([`learning_job`]): a task packet an agent
    /// wrote, Hollow's wake and check-in messages, a resume, a retry, a night
    /// or runbook item never do, so their rules aren't proposed as the
    /// person's preferences. Every other sender leaves it false.
    pub typed: bool,
    /// A turn Hollow queued itself: its tag (`hollow:wake`, `hollow:checkin`,
    /// `hollow:weekly`, `hollow:wake_me`, … : `main_agent::ASKED_BY_*`),
    /// recorded as the turn's `asked_by` (OB20), so it never reads as the
    /// person's or another agent's. It also makes the turn host-opened for
    /// the context compiler (OB19): a conversation it continues from (a Main
    /// agent day thread's predecessor) gives its task state and key, never
    /// its turns as a bundle. `None` for every other send.
    pub host_asker: Option<&'static str>,
    /// A request Hollow wrote and the person sent unchanged (a canvas view,
    /// its Refresh, Explain visually; overnight 9 A1): its tag
    /// (`task_state::canned_asker`), recorded as the turn's `asked_by`, so
    /// its words are never the person's constraints. Unlike
    /// [`ChatSendRequest::host_asker`] it never makes the turn host-opened,
    /// and it can only lower what a turn teaches (never typed). `None` for
    /// every other send.
    pub canned: Option<&'static str>,
    /// A Main agent worker batch's chat (`project_workers`, overnight N6):
    /// like an unattended turn, it follows Settings, Devices,
    /// Overnight and worker chats for where it runs
    /// (`remote_session::unattended_runner`). Nothing else reads it.
    pub worker: bool,
}

/// A queued chat row's payload. It holds the composed prompt (operator and
/// page context included), so `run_queue`'s single terminal helper clears it
/// the moment the row ends; `detail` keeps only the user's own text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatTurnPayload {
    pub(super) prompt: String,
    pub(super) session_id: Option<String>,
    pub(super) cwd: String,
    pub(super) model: Option<String>,
    pub(super) permission_mode: Option<String>,
    pub(super) agent: String,
    /// The user's own text (the composed prompt minus its injected blocks).
    pub(super) asked: String,
    client_turn_id: Option<String>,
    /// [`ChatSendRequest::app_depth`]; absent (0) on the user's own sends
    /// and on rows queued before PR A2.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(super) app_depth: u32,
    /// [`ChatSendRequest::ultracode`]; absent (off) on rows queued before
    /// Track U.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) ultracode: bool,
    /// A phone composer's explicit effort for this turn. Older queued rows
    /// follow the agent's saved preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) effort: Option<String>,
    /// [`ChatSendRequest::canvases`], checked at accept time; absent (follow
    /// the setting) on rows queued before Canvas C3 wave 2 C4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) canvases: Option<String>,
    /// The structured request (agent effectiveness F2): `prompt` split once,
    /// here at admission, into the user's request, its explicit and resolved
    /// entities and the injected blocks. Absent on rows queued before it,
    /// which read as their `asked` text ([`ChatTurnPayload::turn_request`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) request: Option<crate::turn_request::TurnRequest>,
    /// [`ChatSendRequest::continued_from`], checked; absent (none) on rows
    /// queued before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) continued_from: Vec<String>,
    /// [`ChatSendRequest::unattended`]; absent (attended) on every other row.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) unattended: bool,
    /// [`ChatSendRequest::typed`]; absent (not the person's words, so no
    /// learning) on every other row, rows queued before it included.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) typed: bool,
    /// [`ChatSendRequest::host_asker`]; absent (not Hollow's own turn) on
    /// every other row, rows queued before it included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) host_asker: Option<String>,
    /// [`ChatSendRequest::canned`]; absent (the person's own words, or
    /// another sender's) on every other row, rows queued before it included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) canned: Option<String>,
    /// [`ChatSendRequest::worker`]; absent (not a worker batch's chat) on
    /// every other row, rows queued before it included.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) worker: bool,
    /// L1 (COO rebuild, 2026-11-10): the conversation that started this
    /// turn's conversation as its subagent (`start_session`, a worker batch,
    /// a swarm part). Recorded as the session's `parent_key` with
    /// `fork_kind = 'subagent'` when the turn starts, and counted by the
    /// pump's per-parent limit (`run_pump::Hold::ParentLimit`). Absent on
    /// every other row, a message to an existing conversation included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) parent: Option<String>,
}

/// Whether queue row `id` is a chat turn Hollow queued unattended
/// ([`ChatSendRequest::unattended`]). Read from the row's own payload, so it
/// holds from admission on: before the night queue registers the row, and
/// across a restart.
pub(crate) fn queued_unattended(vault: &Path, id: i64) -> bool {
    crate::run_queue::get(vault, id)
        .ok()
        .flatten()
        .is_some_and(|row| row.payload.get("unattended") == Some(&serde_json::Value::Bool(true)))
}

impl ChatTurnPayload {
    /// The admitted request, or, for a row queued before the fields existed,
    /// its `asked` text with the entities it names (no conversation lookup).
    fn turn_request(&self, vault: &Path) -> crate::turn_request::TurnRequest {
        self.request
            .clone()
            .unwrap_or_else(|| crate::turn_request::TurnRequest::from_text(vault, &self.asked))
    }

    /// The person sent this turn: depth 0 (not an agent's `hollow_app`
    /// start or send), not unattended (not a night item, an isolated wake or
    /// an unattended runbook step) and not a turn Hollow queued itself
    /// (`host_asker`: a Main agent wake, check-in or swarm wake, the weekly
    /// update). An empty `host_asker` is absent. `commands::send_chat` asks
    /// the same of its input. `run_queue::payload_closes_cards` matches this.
    pub(crate) fn person_sent(&self) -> bool {
        let host = self.host_asker.as_deref().filter(|asker| !asker.is_empty());
        self.app_depth == 0 && !self.unattended && host.is_none()
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A nonce the frontend minted (`ct-…`, `queue-…`): echoed only, never a path
/// or a query, but still bounded and plain.
fn is_safe_client_turn_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// What every `chat-event` of one queued turn carries: the send's
/// `clientTurnId` nonce and the turn's run-queue row id, so the composer can
/// tell its turn's events apart even when `runStarted` beats the
/// `SendAccepted` reply, and a withdrawn turn's late events never end a newer
/// send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnTag {
    pub(super) client_turn_id: Option<String>,
    pub(super) queue_id: i64,
    /// The conversation the turn belongs to (build programme PR B4), sent as
    /// `conversationKey` on every event so a window showing another
    /// conversation buffers them instead of drawing them into the wrong one.
    /// Routing only: unlike `sessionKey` on `done`/`runStarted`, the
    /// frontend never ADOPTS it (a recovered `done` still hands back none).
    pub(super) conversation_key: Option<String>,
}

impl TurnTag {
    /// The tag of a chat row. Read from the row in memory: its payload is
    /// cleared in the database as soon as the row ends.
    pub(crate) fn of(row: &QueueRow) -> TurnTag {
        TurnTag {
            client_turn_id: row
                .payload
                .get("clientTurnId")
                .and_then(Value::as_str)
                .filter(|id| is_safe_client_turn_id(id))
                .map(str::to_string),
            queue_id: row.id,
            conversation_key: row
                .source_id
                .clone()
                .filter(|key| crate::agent_sessions::is_safe_session_key(key)),
        }
    }

    pub(super) fn apply(&self, mut payload: Value) -> Value {
        if let Some(object) = payload.as_object_mut() {
            object.insert("clientTurnId".into(), json!(self.client_turn_id));
            object.insert("queueId".into(), json!(self.queue_id));
            object.insert("conversationKey".into(), json!(self.conversation_key));
        }
        payload
    }
}

/// Emits one turn's `chat-event`s, each tagged with its [`TurnTag`].
pub(super) struct TurnEvents<'a> {
    pub(super) app: &'a Core,
    pub(super) tag: TurnTag,
    /// The person's own attended, top-level chat (not an agent's, a night
    /// item's or a bare persona/bake-off run): only it may get Settings'
    /// model retitle (morning N5). Set once the payload is read.
    pub(super) retitle: std::sync::atomic::AtomicBool,
    /// Self-hosted models Q10: the chat's own self-hosted entry, when its
    /// retitle may run there (and nowhere else).
    pub(super) retitle_on: std::sync::OnceLock<String>,
    /// The person's own attended, top-level chat, whatever its model (T3a:
    /// only it may get a tracker issue for real work).
    pub(super) person_chat: std::sync::atomic::AtomicBool,
}

impl TurnEvents<'_> {
    pub(super) fn emit(&self, payload: Value) {
        emit(self.app, self.tag.apply(payload));
    }

    /// The turn's `done`, which the PUMP emits once the row is terminal.
    pub(super) fn done(&self, session_id: Option<String>, session_key: &str) -> Value {
        self.tag.apply(crate::chat_event::build::done(
            session_id,
            Some(session_key),
        ))
    }
}

/// How a chat row ended without its turn ever starting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueuedEnd {
    /// Taken out of the queue (Stop, Edit or Discard), or a cancel that
    /// landed while the pump was admitting it.
    Cancelled,
    /// Could not be admitted (a lease only a restart can clear, an
    /// unreadable row, a history failure).
    Failed(String),
    /// Its working folder vanished before the pump claimed it.
    Recovered(String),
}

/// The `chat-event`s a turn that never started ends with (Paperclip PR 7b).
/// Built from the row in memory, before its payload is cleared. A recovered
/// turn's `done` carries no `sessionKey` or `sessionId`, so the composer never
/// adopts the dead conversation's key back after resetting to the vault.
pub(crate) fn queued_end_events(row: &QueueRow, end: &QueuedEnd) -> Vec<Value> {
    let tag = TurnTag::of(row);
    let key = row.source_id.clone();
    let done = crate::chat_event::build::done(None, key.as_deref());
    match end {
        QueuedEnd::Cancelled => vec![
            tag.apply(crate::chat_event::build::cancelled_queued()),
            tag.apply(done),
        ],
        QueuedEnd::Failed(message) => vec![
            tag.apply(crate::chat_event::build::error(message.clone())),
            tag.apply(done),
        ],
        QueuedEnd::Recovered(message) => vec![
            tag.apply(crate::chat_event::build::error(message.clone())),
            tag.apply(crate::chat_event::build::done_recovered()),
        ],
    }
}

/// An accepted chat send: the row that now carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcceptedTurn {
    pub queue_id: i64,
    pub session_key: String,
    pub position: u32,
    /// Who holds the vault lease the turn waits for, when anyone does.
    pub waiting_for: Option<String>,
    /// The send named a conversation that continued elsewhere (or started a
    /// Main agent conversation while the current one works): it went to the
    /// chain's current conversation, `session_key` (`chat_chain::route_send`).
    pub forwarded: Option<crate::chat_chain::Routed>,
}

impl AcceptedTurn {
    pub(crate) fn into_wire(self) -> crate::types::SendAccepted {
        crate::types::SendAccepted {
            session_key: self.session_key,
            queue_id: self.queue_id,
            queue_position: self.position,
            phase: crate::agent_sessions::SessionPhase::Queued
                .as_str()
                .to_string(),
            waiting_for: self.waiting_for,
            held_id: None,
            forwarded_from: self
                .forwarded
                .as_ref()
                .map(|r| r.from.clone().unwrap_or_default()),
            forwarded_title: self.forwarded.and_then(|r| r.to_title),
        }
    }
}

/// Accept a chat send: check it, then enqueue it. Sync (SQLite), so the
/// command runs it on a blocking worker. `Err` means nothing was queued:
/// `CHAT_CONTEXT_RECOVERED: {path}` when the working folder vanished (the
/// composer keeps the text and resets to the vault, as before PR 7b),
/// `UNSAFE_WORKING_DIRECTORY: …` for a system or shared folder (Paperclip
/// PR 19: the composer keeps the text and the folder, and asks for another),
/// the restart hint behind a lease only a restart can clear, or
/// [`CHAT_ALREADY_QUEUED`] while another chat turn is queued or running.
pub(crate) fn accept_turn(
    vault: &Path,
    req: ChatSendRequest,
    now: DateTime<Utc>,
) -> Result<AcceptedTurn, String> {
    accept_turn_with_effort(vault, req, now, None)
}

/// Queue an explicit effort atomically with the turn, never in global settings.
pub(crate) fn accept_turn_with_effort(
    vault: &Path,
    req: ChatSendRequest,
    now: DateTime<Utc>,
    effort: Option<String>,
) -> Result<AcceptedTurn, String> {
    accept_subagent_turn(vault, req, now, effort, None)
}

/// [`accept_turn_with_effort`] for a turn that starts a subagent of
/// conversation `parent` (L1): the parent rides in the payload
/// ([`ChatTurnPayload::parent`]). A key that isn't a session key is dropped.
pub(crate) fn accept_subagent_turn(
    vault: &Path,
    req: ChatSendRequest,
    now: DateTime<Utc>,
    effort: Option<String>,
    parent: Option<String>,
) -> Result<AcceptedTurn, String> {
    accept_keyed_turn(vault, req, now, effort, parent, None)
}

/// [`accept_subagent_turn`] with the row's run-queue key: a held send's
/// (`steering::queue_key`), so a drain after a restart can tell its turn was
/// already queued (`run_queue::row_with_key`) and never sends it twice.
pub(crate) fn accept_keyed_turn(
    vault: &Path,
    req: ChatSendRequest,
    now: DateTime<Utc>,
    effort: Option<String>,
    parent: Option<String>,
    row_key: Option<String>,
) -> Result<AcceptedTurn, String> {
    let parent = parent.filter(|key| crate::agent_sessions::is_safe_session_key(key));
    if effort
        .as_deref()
        .is_some_and(|e| !crate::effort::LEVELS.contains(&e))
    {
        return Err("Unknown reasoning effort.".into());
    }
    // One conversation from the person's side (2026-11-07): a send for a
    // conversation that continued elsewhere goes to the chain's current one,
    // whoever sent it (a race at handover, the phone, `send_to_session`, a
    // held send), and so does a new Main agent thread while one works.
    let mut req = req;
    let forwarded = crate::chat_chain::route_send(
        vault,
        req.session_key.as_deref(),
        &req.cwd,
        req.app_depth,
        !req.continued_from.is_empty(),
    );
    // A new thread in the Main agent's folder, as `route_send` joins one.
    let joins_main =
        req.session_key.is_none() && req.app_depth == 0 && req.continued_from.is_empty();
    if let Some(routed) = &forwarded {
        apply_route(vault, &mut req, routed)?;
    }
    // Lane Q (2026-11-13): the composer sends its chat's own level with every
    // send, so a send typed in a conversation that continued elsewhere named
    // the old conversation's level and beat the current one's. A level equal
    // to the old conversation's own is dropped (the target's own applies when
    // the turn runs); one that differs (a phone send's pick) is kept.
    let effort = match forwarded.as_ref().and_then(|r| r.from.as_deref()) {
        Some(from)
            if effort.is_some()
                && effort
                    == crate::session_store::chat_effort(vault, from)
                        .ok()
                        .flatten() =>
        {
            None
        }
        _ => effort,
    };
    let row_cwd = req.cwd.clone();
    let agent = req.agent;
    let session_id = req
        .session_id
        .filter(|id| !(agent == AgentKind::Antigravity && id == "antigravity"));
    // Before anything is enqueued, so the composer's recovery flow is unchanged.
    let (cwd, session_id) = prepare_chat_send_context(req.cwd, vault, session_id)?;
    // A lease only a restart can clear fails at once rather than queueing a
    // turn that would hang; a live holder (this process's own tracked run
    // included, however its manifest reads) is simply waited for.
    // A workspace's turn never waits for the vault lease (PR B3), so a stale
    // one can't refuse it.
    let scoped = run_scope(vault, &cwd).is_some();
    let holder = crate::automated_history::lease_holder(vault).filter(|_| !scoped);
    if let Some(message) = crate::run_pump::refuse_interactive_in(vault, holder.as_ref()) {
        return Err(message);
    }
    // Hollow owns session identity; a key absent or malformed on the wire is
    // minted fresh, never trusted.
    let session_key = req
        .session_key
        .filter(|key| crate::agent_sessions::is_safe_session_key(key))
        .unwrap_or_else(crate::agent_sessions::mint_session_key);
    crate::project_agents::validate(vault, &cwd, &session_key)?;
    // 7.6: a send into a side chat runs in no stronger a mode than the chat
    // it sits beside, whoever sent it (`side_chat::clamp_admitted`).
    let req_mode =
        crate::side_chat::clamp_admitted(vault, &session_key, req.permission_mode.take());
    let client_turn_id = req.client_turn_id.filter(|id| is_safe_client_turn_id(id));
    // The structured request, split from the composed prompt once, here
    // (agent effectiveness F2). Its `request` is the user's own text
    // (`sessions::strip_injected_context`): the session title, the row's
    // `detail` (restored to the composer if Hollow quits before the turn
    // starts) and the retrieval query all use it. A reference to an earlier
    // entity ("that project") is resolved now, from this conversation.
    let request = crate::turn_request::TurnRequest::admit(vault, &req.prompt, Some(&session_key));
    let asked = request.request.clone();
    let asked_text = asked.clone();
    let req_host = req.host_asker;
    let payload = ChatTurnPayload {
        prompt: req.prompt,
        session_id,
        cwd: cwd.to_string_lossy().into_owned(),
        model: req.model,
        permission_mode: req_mode,
        agent: agent.as_str().to_string(),
        asked: asked.clone(),
        client_turn_id,
        app_depth: req.app_depth,
        ultracode: req.ultracode,
        effort,
        // Only a known choice rides along; anything else follows the setting.
        canvases: req
            .canvases
            .as_deref()
            .and_then(crate::visual_prompt::CanvasFrequency::parse)
            .map(|choice| choice.as_str().to_string()),
        request: Some(request),
        continued_from: crate::context_compiler::continued_from_keys(
            &req.continued_from,
            &session_key,
        ),
        unattended: req.unattended,
        typed: req.typed,
        host_asker: req.host_asker.map(str::to_string),
        canned: req.canned.map(str::to_string),
        worker: req.worker,
        // Never the conversation's own key (a send to itself is refused
        // earlier; this keeps a bad caller from parenting a chat to itself).
        parent: parent.filter(|p| p != &session_key),
    };
    let payload =
        serde_json::to_value(&payload).map_err(|e| format!("queue the chat turn: {e}"))?;
    let mut row = NewRow::new(
        RunSource::Chat,
        Some(session_key.as_str()),
        CHAT_ROW_LABEL,
        "chat",
    )
    .with_payload(payload);
    row.detail = Some(asked);
    row.idempotency_key = row_key;
    // The gate is held across the check, repair and enqueue: the pump marks a
    // chat row before claiming it, so a row it is admitting is never repaired.
    let enqueued = crate::chat_rows::with_active_rows(vault, |current| {
        // Checked again under the gate (review of #1220): a conversation
        // that handed over between the route above and here would take the
        // send. Its last turn records the handover before its row ends, so
        // an idle conversation's successor is already known; a send that
        // lost that race is refused as busy, and its retry (or the person's
        // held send, drained) goes to the successor.
        if crate::chat_chain::current_of(vault, &session_key).is_some() {
            return Ok(ChatEnqueue::Busy);
        }
        // R1 (review of #1336): a fresh follow-up found this conversation
        // idle and is starting from it; this send waits until it is queued.
        if crate::chat_rows::is_reserved(&session_key) {
            return Ok(ChatEnqueue::Busy);
        }
        // A successor whose handover was taken back (its first row never
        // committed, reliability 11-13) is never started by a send already
        // routed to it (the desktop's send routes before admission).
        if crate::app_sessions::taken_back(&session_key) {
            return Err(crate::app_sessions::TAKEN_BACK_REFUSED.to_string());
        }
        // A send forwarded to a successor whose handover was then taken back
        // is retried as busy, so it goes to the conversation it was typed in.
        if let Some(from) = forwarded.as_ref().and_then(|r| r.from.as_deref()) {
            if crate::chat_chain::current_of(vault, from).as_deref() != Some(session_key.as_str()) {
                return Ok(ChatEnqueue::Busy);
            }
        }
        // Two new Main agent threads sent at once both minted a key before
        // either was queued: the second finds the first now (as busy).
        if joins_main
            && forwarded.is_none()
            && crate::chat_chain::route_send(vault, None, &row_cwd, 0, false).is_some()
        {
            return Ok(ChatEnqueue::Busy);
        }
        // A handover's successor: its record first, so it never runs (or
        // is queued) without one (`app_sessions::write_pending_handover`).
        let handover = crate::app_sessions::write_pending_handover(vault, &session_key)?;
        let enqueued = run_queue::enqueue_chat(vault, row, current, now);
        if handover && !matches!(enqueued, Ok(ChatEnqueue::Accepted { .. })) {
            crate::app_sessions::unwrite_handover(vault, &session_key);
        }
        enqueued
    })?;
    let (queue_id, position) = match enqueued {
        ChatEnqueue::Busy => return Err(CHAT_ALREADY_QUEUED.to_string()),
        ChatEnqueue::Accepted { id, position } => (id, position),
    };
    super::send_prep::start(vault, queue_id, &session_key);
    // A vault-root turn behind the app's own bookkeeping (a tracker sync's
    // write phase) names nobody: it is Working, not Queued (2026-11-16).
    let waiting_for = holder
        .filter(|holder| !crate::run_pump::is_internal_holder_text(&holder.describe()))
        .map(|holder| crate::agent_sessions::describe_holder(&holder));
    // The banner's notes are of messages, not of Hollow's own turns.
    if let Some(routed) = forwarded.as_ref().filter(|_| req_host.is_none()) {
        crate::chat_chain::record_forward(vault, routed, &asked_text, &now.to_rfc3339());
    }
    Ok(AcceptedTurn {
        queue_id,
        session_key,
        position,
        waiting_for,
        forwarded,
    })
}

/// Point `req` at the chain's current conversation (`chat_chain::Routed`):
/// its key, native session, folder and agent (the saved model when the
/// agent differs), with the note of where it was sent for the agent.
pub(crate) fn apply_route(
    vault: &Path,
    req: &mut ChatSendRequest,
    routed: &crate::chat_chain::Routed,
) -> Result<(), String> {
    req.session_key = Some(routed.to.clone());
    req.session_id = routed.session_id.clone();
    if !routed.cwd.is_empty() {
        req.cwd = PathBuf::from(&routed.cwd);
    }
    // The current conversation's own agent, model and mode
    // (`chat_chain::routed_settings`), never the old tab's.
    let (agent, model, mode) =
        crate::chat_chain::routed_settings(vault, routed, req.permission_mode.as_deref());
    if let Some(agent) = agent {
        let (kind, model) = crate::agent_settings::resolve(Some(&agent), model.as_deref())?;
        req.agent = kind;
        req.model = model;
    }
    req.permission_mode = Some(mode);
    req.continued_from.clear();
    let sender = if req.host_asker.is_some() || req.unattended {
        crate::chat_chain::Sender::Hollow
    } else if req.app_depth > 0 {
        crate::chat_chain::Sender::Agent
    } else {
        crate::chat_chain::Sender::Person
    };
    req.prompt = crate::chat_chain::with_forward_block(&req.prompt, routed, sender);
    Ok(())
}

/// What the pump's admission of a chat row came to.
pub(crate) enum ChatAdmission {
    /// It holds the vault lease; [`execute_turn`] it.
    Admitted(AdmittedTurn),
    /// Another writer holds the lease; nothing was written.
    Deferred(LeaseHolder),
    /// The working folder vanished while the turn waited
    /// (`CHAT_CONTEXT_RECOVERED: {path}`); nothing was taken.
    Recovered(String),
    /// It cannot run (an unreadable row, a history failure).
    Invalid(String),
    /// P1a: a queued resume whose pause is gone (another turn of the chat
    /// started meanwhile) or no longer its own claim: it ends as a cancelled
    /// queued row, so the chat never continues twice. Nothing was taken.
    Superseded,
}

/// Clears the live-phase record of one admitted turn if it is still that
/// turn's when dropped (a turn dropped unexecuted must not leave "busy"
/// behind). `finish_observed_run` clears it explicitly after `finish_run`.
pub(super) struct LiveRecord(String);

impl Drop for LiveRecord {
    fn drop(&mut self) {
        crate::agent_sessions::clear_run(&self.0);
    }
}

/// An admitted chat turn. It holds the vault lease (a [`RunLease`],
/// finalized as cancelled through the public `finish_run` if dropped, so its
/// protected-change flags are still computed and notified), its stop
/// registration and its live-phase record. Declared in drop order: the lease
/// is released before the record and the stop slot.
pub(crate) struct AdmittedTurn {
    pub(super) lease: RunLease,
    pub(super) live: LiveRecord,
    pub(super) registration: StopRegistration,
    pub(super) tag: TurnTag,
    pub(super) session_key: String,
    pub(super) payload: ChatTurnPayload,
    pub(super) agent: AgentKind,
    pub(super) cwd: PathBuf,
    pub(super) session_id: Option<String>,
    /// The turn's inputs, started at admission (J3a).
    pub(super) recall_prefetch: InputsPrefetch,
}

impl AdmittedTurn {
    /// The history run this admission started.
    pub(crate) fn run_id(&self) -> &str {
        self.lease.id()
    }

    /// The conversation this turn runs in.
    pub(crate) fn session_key(&self) -> &str {
        &self.session_key
    }

    /// A build workspace's turn, admitted on its own folder rather than the
    /// vault lease (PR B3): the pump runs it beside the vault lane.
    pub(crate) fn is_scoped(&self) -> bool {
        self.lease.scope().is_some()
    }

    /// End a turn that never started (a cancel that landed while the pump
    /// admitted it): the run is finalized as cancelled and its record and
    /// stop slot released. No `runStarted` was emitted and no turn row exists.
    pub(crate) fn cancel_unstarted(self) -> Result<crate::automated_history::HistoryRun, String> {
        let AdmittedTurn {
            lease,
            live,
            registration,
            ..
        } = self;
        let finished = lease.finish("cancelled");
        drop(live);
        drop(registration);
        finished
    }

    /// Whether a stop was requested for this turn (tests).
    #[cfg(test)]
    pub(crate) fn stop_requested(&self) -> bool {
        self.registration.signal.requested()
    }

    /// Finalize without executing, the way a turn that wrote its own changes
    /// would be finalized (tests).
    #[cfg(test)]
    pub(crate) fn finish_for_test(self, status: &str) -> crate::automated_history::HistoryRun {
        let AdmittedTurn {
            lease,
            live,
            registration,
            ..
        } = self;
        let run = lease.finish(status).expect("finish the admitted turn");
        drop(live);
        drop(registration);
        run
    }
}

/// Admit a claimed chat row: re-check its working folder, take the vault
/// lease (`try_begin_owned`, the ONLY lease call on the chat path; it
/// delegates to `try_begin_run`, so PR 5's protected-change tracking starts
/// under the history lock), then claim the live record and the keyed stop
/// slot. Called only by `run_pump`. Sync: it snapshots the vault baseline,
/// so the pump runs it on a blocking worker, holding no queue lock.
pub(crate) fn admit_turn(vault: &Path, row: &QueueRow) -> ChatAdmission {
    admit_checked_turn(vault, row)
}

/// Tests: admission's read-only local checks for a queued chat row (payload,
/// agent, folder, project binding). No lease, live record or process is taken.
#[cfg(test)]
pub(crate) fn priority_turn_ready(vault: &Path, row: &QueueRow) -> bool {
    if row.source != crate::run_queue::RunSource::Chat || row.cancel_requested {
        return false;
    }
    let Ok(payload) = serde_json::from_value::<ChatTurnPayload>(row.payload.clone()) else {
        return false;
    };
    let Some(session) = row
        .source_id
        .as_deref()
        .filter(|key| crate::agent_sessions::is_safe_session_key(key))
    else {
        return false;
    };
    if AgentKind::from_opt(&payload.agent).is_none() {
        return false;
    }
    let Ok((cwd, _)) =
        prepare_chat_send_context(PathBuf::from(&payload.cwd), vault, payload.session_id)
    else {
        return false;
    };
    crate::project_agents::validate(vault, &cwd, session).is_ok()
}

fn admit_checked_turn(vault: &Path, row: &QueueRow) -> ChatAdmission {
    let payload: ChatTurnPayload = match serde_json::from_value(row.payload.clone()) {
        Ok(payload) => payload,
        Err(error) => {
            return ChatAdmission::Invalid(format!(
                "the queued chat message could not be read: {error}"
            ))
        }
    };
    let Some(session_key) = row
        .source_id
        .clone()
        .filter(|key| crate::agent_sessions::is_safe_session_key(key))
    else {
        return ChatAdmission::Invalid("the queued chat message has no valid session key".into());
    };
    let Some(agent) = AgentKind::from_opt(&payload.agent) else {
        return ChatAdmission::Invalid(format!("unknown chat agent `{}`", payload.agent));
    };
    // M5: a chat moving to, or living on, another Mac never runs here, so
    // it is never owned by two Macs (`chat_move`).
    if let Some(refusal) = crate::chat_move::admission_refusal(vault, &session_key) {
        return ChatAdmission::Invalid(refusal);
    }
    // P1a: a resume runs only while its own claim still holds the pause.
    if let Some(token) = payload
        .client_turn_id
        .as_deref()
        .filter(|id| crate::chat_pause::is_resume_token(Some(id)))
    {
        if !crate::chat_pause::claim_held(vault, &session_key, token) {
            return ChatAdmission::Superseded;
        }
    }
    // The folder can vanish while the turn waits. Nothing is taken then, and
    // the recovered `done` carries no key (see `queued_end_events`).
    let (cwd, session_id) = match prepare_chat_send_context(
        PathBuf::from(&payload.cwd),
        vault,
        payload.session_id.clone(),
    ) {
        Ok(context) => context,
        Err(error) if error.starts_with(CHAT_CONTEXT_RECOVERED_ERROR) => {
            return ChatAdmission::Recovered(error)
        }
        Err(error) => return ChatAdmission::Invalid(error),
    };
    if let Err(error) = crate::project_agents::validate(vault, &cwd, &session_key) {
        return ChatAdmission::Invalid(error);
    }
    // J3a: the turn's inputs, recall's query embedding with them, start now,
    // beside the lease's vault snapshot below. A turn that isn't admitted
    // drops it (nothing recorded: `turn_inputs_by`); the thread finishes on
    // its own. A row that already waited for the lease (`wait`) starts it
    // only once the lease is held, so a row retried every few seconds
    // behind a long run doesn't take the one embedding slot each time.
    let resumes = asks_resume(session_id.as_deref());
    let early_prefetch = row
        .wait
        .is_none()
        .then(|| prefetch_inputs(vault, &payload, &session_key, resumes));
    // A workspace's turn holds only its own folder (build programme PR B3;
    // every workspace since 2026-09-28), so it runs beside automations and
    // other workspaces' conversations instead of waiting for the vault, and
    // (since 2026-10-07) beside the workspace's other conversations unless
    // it is a build workspace's worktree.
    // Since F6 part B a build worktree's turn holds only its worktree's
    // lock, so a workspace's worktrees and its folder's conversations run
    // at once.
    let mode = run_scope_mode(vault, &cwd);
    let admitted = match mode {
        Some((scope, mode)) => crate::automated_history::try_begin_scoped_mode(
            vault,
            &scope,
            mode,
            "chat",
            session_id.as_deref(),
        ),
        None => crate::automated_history::try_begin_owned(vault, "chat", session_id.as_deref()),
    };
    let lease = match admitted {
        Ok(lease) => lease,
        Err(BeginRunError::Busy(holder)) => return ChatAdmission::Deferred(holder),
        Err(BeginRunError::Failed(error)) => return ChatAdmission::Invalid(error),
    };
    // Claimed only once the lease is held, and released after it, so the
    // live detail a refusal adds never outlives the lease.
    crate::agent_sessions::begin(
        &session_key,
        agent,
        lease.id(),
        crate::agent_sessions::SessionPhase::Preparing,
    );
    let live = LiveRecord(lease.id().to_string());
    let recall_prefetch =
        early_prefetch.unwrap_or_else(|| prefetch_inputs(vault, &payload, &session_key, resumes));
    // Reachable by the stop button from here on: retrieval now runs inside
    // the lease, so a stop during it is honored before spawn.
    let registration = register_stop_row(&session_key, Some(row.id));
    ChatAdmission::Admitted(AdmittedTurn {
        lease,
        live,
        registration,
        tag: TurnTag::of(row),
        session_key,
        payload,
        agent,
        cwd,
        session_id,
        recall_prefetch,
    })
}

/// The user's side of a turn's exchange, as learning reads it (agent
/// effectiveness F2): the request itself, never the composed prompt, so
/// profile, page or conversation blocks can't trigger an extraction; and
/// the notes it named or resolved, which the extractor may target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LearnFrom {
    pub(super) user: String,
    pub(super) references: Vec<String>,
    /// Self-hosted models Q10: the chat's own self-hosted entry, where its
    /// learning extraction runs (and nowhere else); `None` for any other chat.
    pub(super) self_hosted: Option<String>,
}

/// What a chat turn's recall, connector selection and learning read (agent
/// effectiveness F2): the admitted request plus its resolved entities.
/// Retrieval reads `asked`, the same request text, unless the turn's
/// `retrieval_basis` gives another query (overnight OB18, OB25).
pub(super) struct TurnInputs {
    pub(super) recall: crate::vault_learning::RecallResult,
    pub(super) connector_query: crate::context_rank::ConnectorQuery,
    pub(super) learn_from: LearnFrom,
    /// What attached sources are ranked against (F3 part 2).
    pub(super) source_query: String,
    /// Why the request looks visual, if it does (F6, G1): read from the
    /// request and its entities only, never the composed prompt.
    pub(super) visual_cue: Option<&'static str>,
    /// What retrieval, recall and attached sources read: a wake's or
    /// check-in's subject, a short follow-up's query with its prior's
    /// topic, or the request; and whether the turn skips retrieval.
    pub(super) retrieval_basis: crate::turn_request::RetrievalBasis,
    /// The workspace recall ran for, which its usage record names (J3a
    /// review: recorded once, by [`record_turn_recall`]).
    pub(super) recall_scope: Option<String>,
}

/// The inputs of a turn in conversation `session_key`; `resumed`: it asks
/// to resume its native session (a follow-up that adds nothing is then
/// `skipped-followup`, overnight OB25, until the resume plan says the
/// session starts fresh: `RetrievalBasis::on_plan`).
#[cfg(test)]
pub(super) fn turn_inputs(
    vault: &Path,
    payload: &ChatTurnPayload,
    session_key: &str,
    resumed: bool,
) -> TurnInputs {
    let inputs = turn_inputs_by(vault, payload, session_key, resumed, true);
    record_turn_recall(vault, &inputs);
    inputs
}

/// Records the usage of the recall a turn uses (`memory_maintenance::
/// record_recall`), once per turn (J3a review): [`turn_inputs_by`] ranks
/// without recording, so a prefetch that is dropped or replaced counts
/// nothing.
fn record_turn_recall(vault: &Path, inputs: &TurnInputs) {
    if crate::memory_maintenance::record_recall(
        vault,
        &inputs.recall.observed,
        &inputs.recall.items,
        inputs.recall_scope.as_deref(),
    )
    .is_err()
    {
        eprintln!("[memory] usage could not be recorded; saved facts were left unchanged");
    }
}

/// A turn's inputs computed ahead of it (J3a): started at admission, before
/// the lease, on a `hollow-recall-prefetch` thread.
pub(super) type InputsPrefetch = crate::recall_prefetch::Prefetch<TurnInputs>;

/// Start [`turn_inputs`] for an accepted turn now (J3a), so its recall's
/// query embedding runs beside admission instead of in the turn.
pub(super) fn prefetch_inputs(
    vault: &Path,
    payload: &ChatTurnPayload,
    session_key: &str,
    resumed: bool,
) -> InputsPrefetch {
    let vault = vault.to_path_buf();
    let payload = payload.clone();
    let key = session_key.to_string();
    crate::recall_prefetch::Prefetch::start(move || {
        turn_inputs_by(&vault, &payload, &key, resumed, true)
    })
}

/// The turn's inputs from its prefetch (J3a): taken when ready within
/// [`crate::recall_prefetch::RECALL_PREFETCH_CUTOFF`] (no wait at all while
/// recalls run late) and within `RECALL_MEANING_WAIT` of admission;
/// otherwise computed here with recall ranked by stems only
/// (`TurnRequest::recall_past_cutoff`), so recall never holds the turn.
pub(super) fn turn_inputs_prefetched(
    vault: &Path,
    payload: &ChatTurnPayload,
    session_key: &str,
    resumed: bool,
    prefetch: InputsPrefetch,
) -> (TurnInputs, crate::recall_prefetch::Pickup) {
    let cutoff = if crate::vault_embed_index::recall_contended() {
        std::time::Duration::ZERO
    } else {
        crate::recall_prefetch::RECALL_PREFETCH_CUTOFF
    };
    let (ready, pickup) = prefetch.take(cutoff, crate::vault_embed_index::RECALL_MEANING_WAIT);
    let inputs =
        ready.unwrap_or_else(|| turn_inputs_by(vault, payload, session_key, resumed, false));
    record_turn_recall(vault, &inputs);
    (inputs, pickup)
}

/// Whether a turn with native session id `session_id` asks to resume it
/// (OB25): `execute_turn`'s hint, and the prefetch's (J3a).
pub(super) fn asks_resume(session_id: Option<&str>) -> bool {
    session_id
        .map(str::trim)
        .is_some_and(|id| !id.is_empty() && id != "antigravity")
}

/// [`turn_inputs`] without recording recall's usage (J3a review: the
/// caller records the recall it uses, once: [`record_turn_recall`]);
/// `meaning`: recall may wait (boundedly) for its query vector. Without it
/// (J3a, a prefetch past its cutoff) recall ranks by stems and starts no
/// embedding.
pub(super) fn turn_inputs_by(
    vault: &Path,
    payload: &ChatTurnPayload,
    session_key: &str,
    resumed: bool,
    meaning: bool,
) -> TurnInputs {
    let request = payload.turn_request(vault);
    let (user, references) = request.learning_side();
    let workspace = crate::workspaces::workspace_for_cwd(vault, Path::new(&payload.cwd))
        .ok()
        .flatten()
        .map(|w| w.id);
    let retrieval_basis = request.basis_in(vault, session_key, resumed);
    let based = retrieval_basis
        .query
        .as_deref()
        .map(|query| request.reading(query));
    let read = based.as_ref().unwrap_or(&request);
    // Connector selection sends Jev only this turn's words: a host turn's
    // subject is part of its request, a follow-up's prior is not, so a
    // follow-up's connectors are still chosen from the request as typed.
    // (The evidence rerank sees the basis query, prior terms included.)
    let connector_query = if retrieval_basis.basis.is_host() {
        read.connector_query()
    } else {
        request.connector_query()
    };
    // Recall's history cue: a follow-up's own words (not its prior's); a
    // host turn's subject (not Hollow's template and child summaries, whose
    // paths and dates read as history).
    let cue = if retrieval_basis.basis.is_host() {
        &read.request
    } else {
        &request.request
    };
    // A turn that retrieves nothing doesn't embed for recall's meaning
    // either (OF26 review): no retrieval would reuse the vector. That is a
    // bare run (`execute_turn` asks the same of the admitted folder, which
    // is this one) or a skipped basis. A skipped follow-up whose resume
    // plan later starts a fresh session does retrieve (`on_plan`); its
    // recall has ranked by stems by then.
    let bare = crate::mcp_attach::bare_scratch(vault, Path::new(&payload.cwd));
    let recall = crate::turn_request::unrecorded(|| {
        if bare || retrieval_basis.basis.skips() {
            read.recall_without_meaning(vault, workspace.as_deref(), cue)
        } else if meaning {
            read.recall_cued_by(vault, workspace.as_deref(), cue)
        } else {
            read.recall_past_cutoff(vault, workspace.as_deref(), cue)
        }
    });
    TurnInputs {
        recall,
        connector_query,
        learn_from: LearnFrom {
            user,
            references,
            self_hosted: None,
        },
        source_query: read.source_query(),
        // L10: a subagent's message never carries the visual guide.
        visual_cue: if super::is_subagent_turn(vault, session_key, payload.parent.as_deref()) {
            None
        } else {
            turn_visual_cue(
                &request.request,
                &request.entities,
                payload.host_asker.as_deref(),
            )
        },
        retrieval_basis,
        recall_scope: workspace,
    }
}

/// Why a turn looks visual ([`TurnInputs::visual_cue`]). A Main agent
/// check-in never does: it answers in a few lines and question cards, and
/// its own text (`main_agent::PULSE_GUIDANCE` says "options"; the signals
/// carry counts) read as cues, so every check-in carried the ~11 KB visual
/// guide in its message. The guide stays on demand, named by the index.
pub(super) fn turn_visual_cue(
    request: &str,
    entities: &[String],
    host_asker: Option<&str>,
) -> Option<&'static str> {
    if host_asker == Some(crate::main_agent::ASKED_BY_CHECKIN) {
        return None;
    }
    crate::visual_prompt::visual_cue(request, entities)
}

/// Whether a turn may teach (fewer learnings, 10-11): only the person's
/// own typed words ([`ChatSendRequest::typed`]) in an attended, top-level,
/// non-bare turn. A delegated chat's first message is the task packet the
/// Main agent wrote ("don't push", "never install"): before, the extractor
/// proposed those rules as the person's preferences.
pub(super) fn turn_learns(typed: bool, app_depth: u32, unattended: bool, bare: bool) -> bool {
    typed && app_depth == 0 && !unattended && !bare
}

/// The learning job a turn that ended with `status` hands the pump: only
/// a successful one learns, from the user's request (`learn_from`) and the
/// agent's reply, both without Hollow's hidden blocks (an ask-canvas's
/// answer JSON, the suggested next message: `next_step::strip`).
pub(super) fn learning_job(
    learn_from: Option<LearnFrom>,
    status: &str,
    assistant: &str,
    run_id: String,
    changed: bool,
) -> Option<LearningJob> {
    learn_from
        .filter(|_| status == "success")
        .map(|from| LearningJob {
            user: crate::next_step::strip(&from.user),
            references: from.references,
            assistant: crate::next_step::strip(assistant),
            run_id,
            changed,
            self_hosted: from.self_hosted,
        })
}

/// A detached post-turn learning extraction (see [`spawn_learning`]).
pub(crate) struct LearningJob {
    pub(super) user: String,
    pub(super) references: Vec<String>,
    pub(super) assistant: String,
    pub(super) run_id: String,
    pub(super) changed: bool,
    /// The chat's own self-hosted entry (Q10), where it may run only.
    pub(super) self_hosted: Option<String>,
}

/// How an executed turn ended. [`execute_turn`] never emits `done`: the pump
/// ends the row first, then emits [`TurnEnd::done`], then hands
/// [`TurnEnd::learning`] to [`spawn_learning`].
pub(crate) struct TurnEnd {
    /// The history status: `success`, `error` or `cancelled`.
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) done: Value,
    pub(crate) learning: Option<LearningJob>,
}

/// End an admitted turn whose user message could not be committed to its
/// conversation's transcript (merged plan slice 3, B·F10). It never
/// started: no `runStarted`, no usage row, no turn row and so no receipt;
/// only its run is finalized, as an error. The pump's `done` then puts the
/// text back in the composer (a `done` before `runStarted` does that), and
/// the queue row keeps the text for a reload (`run_queue::terminal_update`).
/// Before, the turn ran on as ordinal 0 with nothing in the transcript, and
/// the row's text was cleared when it ended.
pub(super) fn end_unstarted_turn(
    out: &TurnEvents<'_>,
    lease: RunLease,
    session_key: &str,
    message: String,
) -> TurnEnd {
    if let Err(error) = lease.finish("error") {
        eprintln!("[chat] finishing an unstarted turn's run failed: {error}");
    }
    out.emit(crate::chat_event::build::error(message.clone()));
    TurnEnd {
        status: "error".into(),
        error: Some(message),
        done: out.done(None, session_key),
        learning: None,
    }
}

/// How a chat turn whose agent exited ends (Paperclip PR 14a).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ExitOutcome {
    /// The history status: `success` or `error`.
    pub(super) run_status: &'static str,
    /// The turn's error: its stored `error` event (when `store`) and
    /// `TurnEnd.error`, which the run-queue row shows.
    pub(super) error: Option<String>,
    /// Append `error` to the session store's trace as an `error` event.
    pub(super) store: bool,
    /// Also emit it as an `error` chat event (labelled, with `resultKind`).
    pub(super) emit: bool,
}

/// Decide how an exited turn ends from its result kind. The label is applied
/// here, once (`ResultKind::label_error`), and the frontend shows an `error`
/// event verbatim:
/// - a refusal is an error (no learning), recorded as
///   `result_kind::REFUSAL_ERROR` but NOT emitted: the `result` event's
///   `resultKind: "refusal"` already shows a neutral notice;
/// - denied actions (Antigravity's headless denials) are an error, stored
///   and emitted, even though the CLI said success;
/// - a failed exit's message (`exit_message`, built only then) is labelled,
///   stored and emitted, as before plus the label;
/// - an error envelope at exit 0 was already shown by its `result` event, so
///   nothing new is stored or emitted; only `TurnEnd.error` gets its text;
/// - any other failure kind at exit 0 (API-error messages with no result
///   line) is an error too, stored and emitted, since nothing showed it: the
///   status never says success while the receipt records a failure kind.
pub(super) fn exit_outcome(
    kind: crate::result_kind::ResultKind,
    record: Option<&crate::result_kind::ResultRecord>,
    exit_success: bool,
    reported_failure: bool,
    exit_message: impl FnOnce() -> String,
) -> ExitOutcome {
    use crate::result_kind::ResultKind;
    if let Some(error) = crate::result_kind::override_error(kind, record) {
        let emit = kind != ResultKind::Refusal;
        return ExitOutcome {
            run_status: "error",
            error: Some(error),
            store: true,
            emit,
        };
    }
    if !exit_success {
        return ExitOutcome {
            run_status: "error",
            error: Some(kind.label_error(&exit_message())),
            store: true,
            emit: true,
        };
    }
    if reported_failure || kind.is_failure() {
        let text = record
            .and_then(crate::result_kind::ResultRecord::error_text)
            .unwrap_or_else(|| "the agent reported an error".to_string());
        return ExitOutcome {
            run_status: "error",
            error: Some(kind.label_error(&text)),
            store: !reported_failure,
            emit: !reported_failure,
        };
    }
    ExitOutcome {
        run_status: "success",
        error: None,
        store: false,
        emit: false,
    }
}

/// R1: a turn whose account ran out and could not move ends with the
/// failover's message (stored and emitted), never a success.
pub(super) fn failover_outcome(outcome: ExitOutcome, failover_stop: Option<String>) -> ExitOutcome {
    match failover_stop {
        Some(message) if outcome.run_status == "error" => ExitOutcome {
            run_status: "error",
            error: Some(message),
            store: true,
            emit: true,
        },
        _ => outcome,
    }
}

/// The `error` trace row and `error` chat event an exited turn records
/// (Paperclip PR 14a): the stored text and the emitted message are the
/// same, already-labelled string, and the event carries the turn's kind.
pub(super) fn exit_error_records(
    outcome: &ExitOutcome,
    kind: crate::result_kind::ResultKind,
) -> (Option<&str>, Option<Value>) {
    let Some(message) = outcome.error.as_deref().filter(|_| outcome.store) else {
        return (None, None);
    };
    let event = outcome
        .emit
        .then(|| crate::chat_event::build::error_kind(message, kind.as_str()));
    (Some(message), event)
}

/// The envelope's own kind for a live `result` event (Paperclip PR 14a),
/// from everything tracked so far. The frontend labels an error result with
/// it and shows a refusal as a neutral notice.
pub(super) fn live_result_kind(
    agent: AgentKind,
    results: &crate::result_kind::ResultTracker,
) -> crate::result_kind::ResultKind {
    results
        .record()
        .map(|record| crate::result_kind::classify_envelope(agent, &record))
        .unwrap_or(crate::result_kind::ResultKind::Success)
}

/// Whether a fenced `hollow-questions` block becomes a question card: agents
/// without Hollow-owned MCP (Antigravity, Grok) ask that way. Only a reply the agent finished: a success, or one whose
/// only problem is actions the permission mode denied (the CLI reported
/// success, and under Hollow's default mapping every headless write is
/// denied, so a question after a tried write must still reach the user).
/// Never a failed or stopped one.
pub(super) fn fenced_card_allowed(
    agent: AgentKind,
    run_status: &str,
    kind: crate::result_kind::ResultKind,
) -> bool {
    !crate::agent::capabilities(agent).owned_mcp
        && (run_status == "success" || kind == crate::result_kind::ResultKind::PermissionDenied)
}

/// The native id a turn's stable layers (G2) are recorded under, or `None`
/// to leave the stored baseline as it was: only when the agent took the
/// prompt. That is a real agent (not the build-error stand-in), a
/// `finished` reply (a success, or one whose only problem is denied
/// actions, as [`fenced_card_allowed`] reads it) and an id this attempt's
/// own stream `announced`, never the resumed id the stream was seeded
/// with (`TurnStream::last_session_id`, which a failed resume still
/// carries). A prompt
/// composed against what one session holds (`relative_to`, a delta or the
/// held configured half) is held only by that same session; a full one by
/// whichever id was announced. Keeping the old baseline after a failure
/// is safe for the envelope and configured half: the session still holds
/// at least that, so the next turn resends what this one carried. Not for
/// the per-turn layers (S02 review): a failed turn's message may still be
/// in the session, its mode line the latest there, so `execute_turn` first
/// drops the layers it carried in another text
/// (`stable_layers::unsettle`).
pub(super) fn stable_layers_holder<'a>(
    agent_runs: bool,
    finished: bool,
    announced: Option<&'a str>,
    relative_to: &str,
) -> Option<&'a str> {
    let native = announced.filter(|_| agent_runs && finished)?;
    (relative_to.is_empty() || relative_to == native).then_some(native)
}

/// Put back question cards answered for a turn whose agent never ran (it
/// failed or was stopped before spawn, failed to spawn, or produced no
/// stdout), and emit them, so an answer is never lost (Paperclip PR 13). A
/// failure only logs: it must never change the turn's outcome.
pub(super) fn reopen_unstarted_answers(
    out: &TurnEvents<'_>,
    vault: &Path,
    session_key: &str,
    turn_ord: i64,
) {
    match interactions::reopen_unstarted(vault, session_key, turn_ord) {
        Ok(rows) => {
            for row in &rows {
                out.emit(interactions::event_payload(row));
            }
        }
        Err(error) => eprintln!("[interactions] reopening an unsent answer failed: {error}"),
    }
}

/// The cards answered for a turn whose agent never ran, once the turn's
/// context exists: reopened ([`reopen_unstarted_answers`]), unless the turn
/// had already announced a session reset. Then they expire as
/// `session_reset` instead (cross-m34 H4): the answer turn's text stays in
/// the transcript and reaches the fresh session through its bundle, so a
/// card coming back would only ask again what was answered.
pub(super) fn settle_unstarted_answers<S: ChatSink + ?Sized>(sink: &S, cx: &TurnCx<'_>) {
    let settled = if cx
        .reset_announced
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        interactions::expire_unstarted(cx.vault, cx.session_key, cx.turn_ord, "session_reset")
    } else {
        interactions::reopen_unstarted(cx.vault, cx.session_key, cx.turn_ord)
    };
    match settled {
        Ok(rows) => {
            for row in &rows {
                sink.emit(interactions::event_payload(row));
            }
        }
        Err(error) => eprintln!("[interactions] settling an unsent answer failed: {error}"),
    }
}

#[cfg(test)]
mod e1_card_tests {
    use super::ChatTurnPayload;

    #[test]
    fn e1_card_empty_host_asker_is_a_person_send() {
        let empty: ChatTurnPayload = serde_json::from_value(serde_json::json!({
            "prompt": "typed",
            "cwd": "/",
            "agent": "claude",
            "asked": "typed",
            "hostAsker": ""
        }))
        .unwrap();
        assert!(empty.person_sent());
        let wake: ChatTurnPayload = serde_json::from_value(serde_json::json!({
            "prompt": "wake",
            "cwd": "/",
            "agent": "claude",
            "asked": "wake",
            "hostAsker": "hollow:wake"
        }))
        .unwrap();
        assert!(!wake.person_sent());
    }
}

#[cfg(test)]
mod priority_admission_tests {
    use super::*;
    #[test]
    fn priority_turn_rechecks_payload_and_vanished_context_without_admitting() {
        let vault = tempfile::tempdir().unwrap();
        let cwd = vault.path().join("queued-context");
        std::fs::create_dir(&cwd).unwrap();
        let id=crate::run_queue::enqueue(vault.path(),crate::run_queue::NewRow::new(crate::run_queue::RunSource::Chat,Some("s-priority-ready"),"Chat","typed")
            .with_payload(serde_json::json!({"prompt":"question","asked":"question","agent":"claude","cwd":cwd,"typed":true})),chrono::Utc::now()).unwrap().id();
        let row = crate::run_queue::get(vault.path(), id).unwrap().unwrap();
        assert!(priority_turn_ready(vault.path(), &row));
        let mut malformed = row.clone();
        malformed.payload["permissionMode"] = serde_json::json!({"bad":true});
        assert!(!priority_turn_ready(vault.path(), &malformed));
        let mut wrong_project = row.clone();
        wrong_project.source_id = Some("s-project-missing".into());
        assert!(!priority_turn_ready(vault.path(), &wrong_project));
        std::fs::remove_dir(&cwd).unwrap();
        assert!(!priority_turn_ready(vault.path(), &row));
        let still = crate::run_queue::get(vault.path(), id).unwrap().unwrap();
        assert_eq!(still.status, crate::run_queue::RowStatus::Queued);
        assert!(still.history_run_id.is_none());
    }
}
