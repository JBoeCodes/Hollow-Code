//! One typed schema for `chat-event` (AG2): every payload [`crate::chat::emit_event`]
//! sends is a [`ChatEventEnvelope`], a [`ChatEvent`] variant plus the queued
//! turn's tag fields.
//!
//! `src/generated/chatEvent.ts` is generated from these types by
//! `ts_bindings.rs` (ts-rs, test builds only), and the frontend's
//! `ChatEventPayload` is that generated type, so a field added, renamed or
//! dropped here changes the TS type and fails `tsc` where the UI reads it.
//! In test builds the emit funnel checks every payload round-trips through
//! the envelope ([`assert_conforms`]): a builder that writes a field the
//! schema lacks, or an unknown `type`, fails the test that emitted it. Each
//! variant has a golden fixture under `tests/fixtures/chat-events/`, and the
//! AG1 grammar corpus is parsed with these types too.
//!
//! Builders go through [`build`] (and [`ChatEvent::wire`]), so a builder that
//! writes a field the schema lacks does not compile. `chat.rs`'s sites are
//! and the queue, interaction, steer and held-send sites are converted too;
//! `remoteRun` stays a `Value` and so stays `json!`. Nested objects that already
//! have a frontend type (`TurnUsage`, `SteerWire`, the receipt, a question
//! card) stay `Value` here and are named for TS with `#[ts(type = ..)]`.
//! ts-rs drops `nullable` when a field has a `type` override, so a nullable
//! override spells `| null` itself.
//! Hollow's own design; no AG-UI schema or generator was copied.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fields every event of a queued-era turn carries (`chat_queue::TurnTag`):
/// the send's nonce, its run-queue row and its conversation. Routing only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatTurnTag {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional = nullable))]
    pub client_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number | null"))]
    pub queue_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional = nullable))]
    pub conversation_key: Option<String>,
}

/// One `chat-event` payload as the window receives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct ChatEventEnvelope {
    #[serde(flatten)]
    pub tag: ChatTurnTag,
    #[serde(flatten)]
    pub event: ChatEvent,
}

/// Every `chat-event` type. The grammar's `KNOWN` list is this one
/// (`chat_event_tests::grammar_knows_every_variant`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ChatEvent {
    Session(SessionEvent),
    RunStarted(RunStartedEvent),
    AssistantText(AssistantTextEvent),
    AssistantDelta(AssistantTextEvent),
    TurnBoundary(TurnBoundaryEvent),
    ToolUse(ToolUseEvent),
    ToolResult(ToolResultEvent),
    ToolImages(ToolImagesEvent),
    EvidenceReceipt(EvidenceReceiptEvent),
    Result(ResultEvent),
    Usage(UsageEvent),
    Activity(ActivityEvent),
    BudgetWarning(BudgetWarningEvent),
    LoginLow(SessionMessageEvent),
    Panel(PanelEvent),
    Interaction(InteractionEvent),
    Cancelled(CancelledEvent),
    Paused(PausedEvent),
    SessionReset(SessionResetEvent),
    AccountSwitched(AccountSwitchedEvent),
    /// The Mac mini runner's notices (`remote_session.rs`); its phases are
    /// the frontend's `RemoteRunEvent` union.
    RemoteRun(#[cfg_attr(test, ts(type = "WithoutType<RemoteRunEvent>"))] Value),
    Error(ErrorEvent),
    Done(DoneEvent),
    Steer(SteerEvent),
    HeldSent(HeldSentEvent),
    AgentContinued(AgentContinuedEvent),
    HeldFailed(HeldFailedEvent),
}

/// The `type` of every variant, in declaration order.
#[cfg(test)]
pub(crate) const TYPES: &[&str] = &[
    "session",
    "runStarted",
    "assistantText",
    "assistantDelta",
    "turnBoundary",
    "toolUse",
    "toolResult",
    "toolImages",
    "evidenceReceipt",
    "result",
    "usage",
    "activity",
    "budgetWarning",
    "loginLow",
    "panel",
    "interaction",
    "cancelled",
    "paused",
    "sessionReset",
    "accountSwitched",
    "remoteRun",
    "error",
    "done",
    "steer",
    "heldSent",
    "agentContinued",
    "heldFailed",
];

/// Declares a variant's field struct with the attributes every one shares.
macro_rules! event_struct {
    ($(#[$meta:meta])* $name:ident { $($body:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[cfg_attr(test, derive(ts_rs::TS))]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name { $($body)* }
    };
}

event_struct!(
    /// The native session id the agent announced.
    SessionEvent { pub session_id: String }
);

event_struct!(
    /// A turn started. `asked` is the person's own text; `rt` the recall trace.
    RunStartedEvent {
        pub run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub session_key: Option<String>,
        #[serde(default)]
        #[cfg_attr(test, ts(type = "EvidenceRecallItemWire[]"))]
        pub recall: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub asked: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional, type = "SteerSupportRow | null"))]
        pub steer_support: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub permission_mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional, type = "ChatRecallTrace | null"))]
        pub rt: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub unattended: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub attended: Option<bool>,
    }
);

event_struct!(
    /// `assistantText` (a whole block) and `assistantDelta` (a streamed
    /// piece). AG7: `parentToolUseId` names the spawning subagent call.
    AssistantTextEvent {
        pub text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub parent_tool_use_id: Option<String>,
    }
);

event_struct!(
    /// A new text segment starts (between tool calls).
    TurnBoundaryEvent {}
);

event_struct!(
    /// A tool call. Grok and Antigravity calls have no id.
    ToolUseEvent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub id: Option<String>,
        pub name: String,
        #[cfg_attr(test, ts(type = "unknown"))]
        pub input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub parent_tool_use_id: Option<String>,
    }
);

event_struct!(
    /// A tool call's result.
    ToolResultEvent {
        pub id: String,
        pub content: String,
        pub is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub parent_tool_use_id: Option<String>,
    }
);

event_struct!(
    /// Images a tool call returned (data URLs).
    ToolImagesEvent {
        pub id: String,
        pub images: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub parent_tool_use_id: Option<String>,
    }
);

event_struct!(
    /// The turn's execution receipt.
    EvidenceReceiptEvent {
        #[cfg_attr(test, ts(type = "EvidenceReceiptWire"))]
        pub receipt: Value,
    }
);

event_struct!(
    /// The agent's result envelope. `costUsd` is the raw harness figure.
    ResultEvent {
        pub result: Option<String>,
        pub cost_usd: Option<f64>,
        pub is_error: bool,
        pub subtype: String,
        pub session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub result_kind: Option<String>,
    }
);

event_struct!(
    /// The finished turn's usage-ledger row, on every exit.
    UsageEvent {
        #[cfg_attr(test, ts(type = "TurnUsage"))]
        pub usage: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub session_key: Option<String>,
    }
);

event_struct!(
    /// The running turn's activity pulse, every 15 s.
    ActivityEvent {
        pub run_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub session_key: Option<String>,
        pub last_event_at: Option<String>,
        #[serde(default)]
        #[cfg_attr(test, ts(type = "ProgressPhase | null"))]
        pub progress: Value,
    }
);

event_struct!(
    /// A usage budget warning for the conversation (chat only warns).
    BudgetWarningEvent {
        pub session_key: String,
        #[cfg_attr(test, ts(type = "BudgetNotice"))]
        pub notice: Value,
    }
);

event_struct!(
    /// `loginLow`: a system line for one conversation.
    SessionMessageEvent {
        pub session_key: String,
        pub message: String,
    }
);

event_struct!(
    /// AG6: one generic panel channel. `panel` names the panel
    /// (`hollow:context`, `hollow:gate`); `snapshot` is its whole current
    /// state and replaces the last one for that conversation and panel. It
    /// never goes back to the agent and never marks a turn's lifecycle: use a
    /// typed variant for that. Panel snapshots are replace-only; a patch form
    /// waits for a feature that needs deltas.
    PanelEvent {
        pub session_key: String,
        pub panel: String,
        #[cfg_attr(test, ts(type = "unknown"))]
        pub snapshot: Value,
    }
);

event_struct!(
    /// A question or approval card was recorded, superseded or expired.
    InteractionEvent {
        #[cfg_attr(test, ts(type = "ChatInteraction"))]
        pub interaction: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub session_key: Option<String>,
    }
);

event_struct!(
    /// A stopped turn, after its processes ended and its receipt was written.
    CancelledEvent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional, type = "'verified' | 'unconfirmed' | null"))]
        pub stop: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub queued: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub before_spawn: Option<bool>,
    }
);

event_struct!(
    /// The turn paused at a safe point. `at` and `resumeAt` are unix seconds.
    PausedEvent {
        pub session_key: String,
        pub note: String,
        #[cfg_attr(test, ts(type = "number"))]
        pub at: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional, type = "PauseReason"))]
        pub reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional, type = "number | null"))]
        pub resume_at: Option<i64>,
    }
);

event_struct!(
    /// The conversation moved to a fresh native session.
    SessionResetEvent {
        #[cfg_attr(test, ts(type = "SessionResetReason"))]
        pub reason: String,
        pub agent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub session_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub changed: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub away: Option<String>,
    }
);

event_struct!(
    /// The turn's Claude account ran out and it resumed on another (labels only).
    AccountSwitchedEvent {
        pub from: String,
        pub to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional, type = "'limit' | 'ceiling' | null"))]
        pub reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub session_key: Option<String>,
    }
);

event_struct!(
    /// A failure, already labelled by the backend.
    ErrorEvent {
        pub message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub result_kind: Option<String>,
    }
);

event_struct!(
    /// The turn is over. `recovered`: its folder vanished before it ran.
    DoneEvent {
        pub session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional = nullable))]
        pub session_key: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        pub recovered: Option<bool>,
    }
);

event_struct!(
    /// A steer of the running turn, when sent and as its state moves.
    SteerEvent {
        pub session_key: String,
        #[cfg_attr(test, ts(type = "SteerWire"))]
        pub steer: Value,
    }
);

event_struct!(
    /// A send held for after a turn went. Its `clientTurnId` and `queueId`
    /// are the envelope's tag fields.
    HeldSentEvent {
        pub held_id: String,
        #[cfg_attr(test, ts(type = "'queue' | 'stop'"))]
        pub after: String,
        pub session_key: String,
    }
);

event_struct!(
    /// The turn went on with another agent when its own ran out. Its
    /// `clientTurnId` and `queueId` are the envelope's tag fields.
    AgentContinuedEvent {
        pub from: String,
        pub to: String,
        pub prompt: String,
        pub session_key: String,
    }
);

event_struct!(
    /// A held send could not go.
    HeldFailedEvent {
        pub held_id: String,
        pub error: String,
        pub prompt: String,
        pub session_key: String,
    }
);

impl ChatEvent {
    /// The payload as it goes on the wire, with no turn tag (the sink's
    /// `TurnTag` adds that). Building through the variant structs means a
    /// builder that names a field the schema lacks does not compile.
    pub(crate) fn wire(self) -> Value {
        self.wire_with(ChatTurnTag::default())
    }

    /// The payload with the turn's `tag` fields (held-send and continuation
    /// events built outside a turn's sink).
    pub(crate) fn wire_with(self, tag: ChatTurnTag) -> Value {
        serde_json::to_value(ChatEventEnvelope { tag, event: self })
            .expect("a chat event serializes")
    }
}

/// `value` as a JSON value (a nested wire type the schema keeps as `Value`).
fn val(value: impl Serialize) -> Value {
    serde_json::to_value(value).expect("a wire value serializes")
}

/// `payload` with each of `keys` present as an explicit `null` when the
/// builder left it out. The earlier `json!` builders wrote these as `null`,
/// and the transcript keeps the bytes, so the typed ones do too.
fn keep_null(mut payload: Value, keys: &[&str]) -> Value {
    if let Some(map) = payload.as_object_mut() {
        for key in keys {
            map.entry(*key).or_insert(Value::Null);
        }
    }
    payload
}

/// The typed builders of `chat.rs`'s construction sites. Each takes what the
/// `json!` it replaced read and writes the same bytes
/// (`chat_event_tests::builders_write_the_old_bytes`).
pub(crate) mod build {
    use super::*;

    pub(crate) fn session(session_id: &str) -> Value {
        ChatEvent::Session(SessionEvent {
            session_id: session_id.into(),
        })
        .wire()
    }

    pub(crate) fn turn_boundary() -> Value {
        ChatEvent::TurnBoundary(TurnBoundaryEvent {}).wire()
    }

    pub(crate) fn assistant_text(text: &str) -> Value {
        ChatEvent::AssistantText(AssistantTextEvent {
            text: text.into(),
            parent_tool_use_id: None,
        })
        .wire()
    }

    pub(crate) fn assistant_delta(text: &str) -> Value {
        ChatEvent::AssistantDelta(AssistantTextEvent {
            text: text.into(),
            parent_tool_use_id: None,
        })
        .wire()
    }

    /// `id` is the empty string for the agents whose calls carry none (Grok,
    /// Antigravity), as the old builder wrote it.
    pub(crate) fn tool_use(id: &str, name: &str, input: Value) -> Value {
        ChatEvent::ToolUse(ToolUseEvent {
            id: Some(id.into()),
            name: name.into(),
            input,
            parent_tool_use_id: None,
        })
        .wire()
    }

    pub(crate) fn tool_result(id: &str, content: &str, is_error: bool) -> Value {
        ChatEvent::ToolResult(ToolResultEvent {
            id: id.into(),
            content: content.into(),
            is_error,
            parent_tool_use_id: None,
        })
        .wire()
    }

    pub(crate) fn tool_images(id: &str, images: Vec<String>) -> Value {
        ChatEvent::ToolImages(ToolImagesEvent {
            id: id.into(),
            images,
            parent_tool_use_id: None,
        })
        .wire()
    }

    pub(crate) fn evidence_receipt(receipt: impl Serialize) -> Value {
        ChatEvent::EvidenceReceipt(EvidenceReceiptEvent {
            receipt: val(receipt),
        })
        .wire()
    }

    pub(crate) fn result(
        result: Option<String>,
        cost_usd: Option<f64>,
        is_error: bool,
        subtype: &str,
        result_kind: &str,
        session_id: Option<String>,
    ) -> Value {
        ChatEvent::Result(ResultEvent {
            result,
            cost_usd,
            is_error,
            subtype: subtype.into(),
            session_id,
            result_kind: Some(result_kind.into()),
        })
        .wire()
    }

    pub(crate) fn usage(usage: Value, session_key: &str) -> Value {
        ChatEvent::Usage(UsageEvent {
            usage,
            session_key: Some(session_key.into()),
        })
        .wire()
    }

    pub(crate) fn error(message: impl Into<String>) -> Value {
        ChatEvent::Error(ErrorEvent {
            message: message.into(),
            result_kind: None,
        })
        .wire()
    }

    pub(crate) fn error_kind(message: impl Into<String>, result_kind: &str) -> Value {
        ChatEvent::Error(ErrorEvent {
            message: message.into(),
            result_kind: Some(result_kind.into()),
        })
        .wire()
    }

    pub(crate) fn budget_warning(session_key: &str, notice: impl Serialize) -> Value {
        ChatEvent::BudgetWarning(BudgetWarningEvent {
            session_key: session_key.into(),
            notice: val(notice),
        })
        .wire()
    }

    pub(crate) fn login_low(session_key: &str, message: &str) -> Value {
        ChatEvent::LoginLow(SessionMessageEvent {
            session_key: session_key.into(),
            message: message.into(),
        })
        .wire()
    }

    pub(crate) fn panel(session_key: &str, panel: &str, snapshot: Value) -> Value {
        ChatEvent::Panel(PanelEvent {
            session_key: session_key.into(),
            panel: panel.into(),
            snapshot,
        })
        .wire()
    }

    /// `stop` is `None` for a turn stopped before its agent started; the
    /// old builder wrote it as `null`.
    pub(crate) fn cancelled(stop: Option<&str>, before_spawn: bool) -> Value {
        keep_null(
            ChatEvent::Cancelled(CancelledEvent {
                stop: stop.map(str::to_string),
                queued: None,
                before_spawn: before_spawn.then_some(true),
            })
            .wire(),
            &["stop"],
        )
    }

    pub(crate) fn paused(session_key: &str, note: &str, at: i64, reason: &str) -> Value {
        keep_null(
            ChatEvent::Paused(PausedEvent {
                session_key: session_key.into(),
                note: note.into(),
                at,
                reason: Some(reason.into()),
                resume_at: None,
            })
            .wire(),
            &["resumeAt"],
        )
    }

    pub(crate) fn activity(
        run_id: &str,
        session_key: &str,
        last_event_at: Option<String>,
        progress: impl Serialize,
    ) -> Value {
        ChatEvent::Activity(ActivityEvent {
            run_id: run_id.into(),
            session_key: Some(session_key.into()),
            last_event_at,
            progress: val(progress),
        })
        .wire()
    }

    pub(crate) fn account_switched(from: &str, to: &str, reason: &str, session_key: &str) -> Value {
        ChatEvent::AccountSwitched(AccountSwitchedEvent {
            from: from.into(),
            to: to.into(),
            reason: Some(reason.into()),
            session_key: Some(session_key.into()),
        })
        .wire()
    }

    pub(crate) fn session_reset(
        reason: &str,
        agent: &str,
        session_key: &str,
        changed: Option<Vec<String>>,
        away: Option<String>,
    ) -> Value {
        ChatEvent::SessionReset(SessionResetEvent {
            reason: reason.into(),
            agent: agent.into(),
            session_key: Some(session_key.into()),
            changed,
            away,
        })
        .wire()
    }

    /// The turn's `done`. A recovered turn's carries neither `sessionId` nor
    /// `sessionKey` (the old builder wrote only `recovered`).
    /// `sessionKey` is `null` when the row had none, as the old builder wrote it.
    pub(crate) fn done(session_id: Option<String>, session_key: Option<&str>) -> Value {
        keep_null(
            ChatEvent::Done(DoneEvent {
                session_id,
                session_key: session_key.map(str::to_string),
                recovered: None,
            })
            .wire(),
            &["sessionKey"],
        )
    }

    pub(crate) fn done_recovered() -> Value {
        let mut payload = ChatEvent::Done(DoneEvent {
            session_id: None,
            session_key: None,
            recovered: Some(true),
        })
        .wire();
        if let Some(map) = payload.as_object_mut() {
            map.remove("sessionId");
        }
        payload
    }

    /// A queued row cancelled before its turn started.
    pub(crate) fn cancelled_queued() -> Value {
        keep_null(
            ChatEvent::Cancelled(CancelledEvent {
                stop: None,
                queued: Some(true),
                before_spawn: None,
            })
            .wire(),
            &["stop"],
        )
    }

    pub(crate) fn interaction(interaction: impl Serialize, session_key: &str) -> Value {
        ChatEvent::Interaction(InteractionEvent {
            interaction: val(interaction),
            session_key: Some(session_key.into()),
        })
        .wire()
    }

    pub(crate) fn steer(session_key: &str, steer: impl Serialize) -> Value {
        ChatEvent::Steer(SteerEvent {
            session_key: session_key.into(),
            steer: val(steer),
        })
        .wire()
    }

    pub(crate) fn held_failed(
        held_id: &str,
        error: &str,
        prompt: &str,
        session_key: &str,
        conversation_key: &str,
    ) -> Value {
        ChatEvent::HeldFailed(HeldFailedEvent {
            held_id: held_id.into(),
            error: error.into(),
            prompt: prompt.into(),
            session_key: session_key.into(),
        })
        .wire_with(ChatTurnTag {
            conversation_key: Some(conversation_key.into()),
            ..ChatTurnTag::default()
        })
    }

    /// `clientTurnId` is written as `null` when the held send had none.
    pub(crate) fn held_sent(
        held_id: &str,
        after: impl Serialize,
        session_key: &str,
        queue_id: i64,
        client_turn_id: Option<String>,
        conversation_key: &str,
    ) -> Value {
        keep_null(
            ChatEvent::HeldSent(HeldSentEvent {
                held_id: held_id.into(),
                after: val(after).as_str().unwrap_or_default().to_string(),
                session_key: session_key.into(),
            })
            .wire_with(ChatTurnTag {
                client_turn_id,
                queue_id: Some(queue_id),
                conversation_key: Some(conversation_key.into()),
            }),
            &["clientTurnId"],
        )
    }

    pub(crate) fn agent_continued(
        from: &str,
        to: &str,
        prompt: &str,
        session_key: &str,
        queue_id: i64,
        client_turn_id: &str,
        conversation_key: &str,
    ) -> Value {
        ChatEvent::AgentContinued(AgentContinuedEvent {
            from: from.into(),
            to: to.into(),
            prompt: prompt.into(),
            session_key: session_key.into(),
        })
        .wire_with(ChatTurnTag {
            client_turn_id: Some(client_turn_id.into()),
            queue_id: Some(queue_id),
            conversation_key: Some(conversation_key.into()),
        })
    }

    /// `runStarted`: every optional field the old builder wrote is written
    /// as `null` when empty.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_started(
        run_id: &str,
        session_key: &str,
        recall: impl Serialize,
        asked: Option<String>,
        steer_support: Option<impl Serialize>,
        permission_mode: Option<String>,
        rt: Option<impl Serialize>,
        unattended: Option<bool>,
        attended: Option<bool>,
    ) -> Value {
        keep_null(
            ChatEvent::RunStarted(RunStartedEvent {
                run_id: run_id.into(),
                session_key: Some(session_key.into()),
                recall: val(recall),
                asked,
                steer_support: steer_support.map(val),
                permission_mode,
                rt: rt.map(val),
                unattended,
                attended,
            })
            .wire(),
            &[
                "steerSupport",
                "rt",
                "asked",
                "permissionMode",
                "unattended",
                "attended",
            ],
        )
    }
}

/// `payload` without its top-level `null` fields: an absent optional and an
/// explicit `null` read the same to the window.
#[cfg(test)]
fn without_nulls(payload: &Value) -> Value {
    match payload {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Why `payload` is not a [`ChatEventEnvelope`]: it does not parse, or a
/// field is lost on the way through (one the schema does not have).
#[cfg(test)]
pub(crate) fn conformance_error(payload: &Value) -> Option<String> {
    let parsed: ChatEventEnvelope = match serde_json::from_value(payload.clone()) {
        Ok(parsed) => parsed,
        Err(error) => return Some(format!("does not parse: {error}")),
    };
    // These two always name their queue row (and `agentContinued` its send);
    // `ChatEventPayload` declares the tag fields required on them.
    let tag = &parsed.tag;
    match parsed.event {
        ChatEvent::HeldSent(_) if tag.queue_id.is_none() => {
            return Some("heldSent without queueId".into());
        }
        ChatEvent::AgentContinued(_) if tag.queue_id.is_none() || tag.client_turn_id.is_none() => {
            return Some("agentContinued without queueId and clientTurnId".into());
        }
        _ => {}
    }
    let back = serde_json::to_value(&parsed).expect("an envelope serializes");
    let (want, have) = (without_nulls(payload), without_nulls(&back));
    if want == have {
        return None;
    }
    let lost: Vec<&String> = want
        .as_object()
        .map(|m| {
            m.keys()
                .filter(|k| have.get(k.as_str()) != m.get(k.as_str()))
                .collect()
        })
        .unwrap_or_default();
    Some(format!("fields not in the schema or changed: {lost:?}"))
}

/// Test builds: every emitted payload must conform, so a builder that drifts
/// from the schema fails the test that emitted it.
#[cfg(test)]
pub(crate) fn assert_conforms(payload: &Value) {
    // A payload with no `type` is not a chat event at all (a host helper test
    // sends arbitrary bytes through the funnel to compare transports); every
    // real event carries one, and an unknown `type` still fails below.
    if payload.get("type").is_none() {
        return;
    }
    if let Some(error) = conformance_error(payload) {
        panic!("chat-event payload is not in the AG2 schema (chat_event.rs): {error}\n{payload}");
    }
}

#[cfg(test)]
#[path = "chat_event_tests.rs"]
mod chat_event_tests;
