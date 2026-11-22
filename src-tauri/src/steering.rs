//! Mid-turn steering (2026-10-21, outputs/steering-2026-10-21/design.md).
//!
//! While a chat turn runs, the person can send one of three things from that
//! chat's own composer:
//! - **Steer now** ([`steer`]): the message goes into the running agent and is
//!   taken into account in this same turn, at its next step. Only agents whose
//!   transport has a true steer ([`resolve_support`]): Claude Code over
//!   `--input-format stream-json`, where a user line written mid-turn is held
//!   until the open tool returns and then folded into the turn
//!   (`command_lifecycle` `queued` → `started`), and qualified Codex versions over
//!   `codex app-server` (steering PR 3), where the attempt's
//!   `codex_app_server::AppSession` writes it as `turn/steer` and its
//!   steered `userMessage` item marks it delivered ([`mark`]).
//! - **Queue for after** and **Stop and send** ([`hold`]): the send is held
//!   here and goes as the conversation's next turn when this one ends
//!   (`run_pump`, [`take_held`]); Stop and send also stops the turn first.
//!
//! A steer is the person's own message: it is recorded in the session store
//! under the running turn (`steer` / `steerState` events), drawn inline in the
//! running reply, put on the receipt and read by task-state. The UI's
//! `chat_steer` command steers, and so does the COO's `sessions.steer_session`
//! action, only on a chat it started (COO parity L13); no agent tool does.
//!
//! Locks: [`LIVE`] and [`HELD`] are leaves, never held while writing the
//! session store or emitting.

use crate::agent::AgentKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use tokio::sync::mpsc::UnboundedSender;

/// No turn is running in this conversation (it ended, or never started).
pub const STEER_NOT_RUNNING: &str = "STEER_NOT_RUNNING";
/// This conversation's agent has no true steer on Hollow's transport.
pub const STEER_UNSUPPORTED: &str = "STEER_UNSUPPORTED";
/// The turn has finished its answer and its input is closed: the message
/// goes as the next one instead (the composer re-sends it with `after`).
pub const STEER_ENDING: &str = "STEER_ENDING";
/// An empty steer, one longer than [`MAX_STEER_CHARS`], a repeated id or
/// one past [`MAX_STEERS_PER_TURN`].
pub const STEER_INVALID: &str = "STEER_INVALID";
/// The steer couldn't be recorded in the session store, so it wasn't sent.
pub const STEER_NOT_STORED: &str = "STEER_NOT_STORED";
/// The longest steer accepted, in characters: an interjection, and its
/// stored record stays whole under the session store's event cap. A longer
/// message goes as Queue for after.
pub const MAX_STEER_CHARS: usize = 3_000;
/// The longest steer text as serialized JSON (escapes included), so its
/// stored record stays under the session store's 8,000-character event cap.
pub const MAX_STEER_JSON: usize = 7_000;
/// The steers one turn accepts (a runaway composer can't flood the agent).
pub const MAX_STEERS_PER_TURN: usize = 20;
/// The longest refusal reason a steer record keeps ([`refuse`]).
pub const MAX_REFUSAL_CHARS: usize = 200;
/// Held sends per conversation.
pub const MAX_HELD: usize = 5;
/// How long a turn whose answer ended keeps its input open for a steer that
/// was written but not yet read, before closing it anyway.
pub const LATE_STEER_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// A steer id the composer minted: echoed back to the agent as the message's
/// `uuid` and on every event, so bounded and plain.
pub fn is_safe_id(id: &str) -> bool {
    (1..=80).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// What Steer now really does for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SteerSupport {
    /// The running agent takes the message at its next step, in this turn.
    Native,
    /// No true steer: the composer offers Queue for after and Stop and send.
    None,
}

/// Steer now per agent, on the transport Hollow runs it with today. Probed
/// 2026-10-21 (design note): Codex steers only over `app-server`, which its
/// qualified chat turns use (`crate::codex_app_server`); older or unknown
/// versions use exec without steering. Grok steers
/// only over ACP (Hollow runs `grok -p`), and Antigravity's stream input
/// only queues a whole next turn.
///
/// AC1 (2026-11-17): the agent's `steer` capability decides first, so no
/// agent is named here; a false flag is [`SteerSupport::None`] whatever the
/// version. A true flag is then held to [`qualified_codex`] for the agent
/// whose transport depends on a CLI version: that is a second, version
/// dependent gate on top of the capability, not a replacement for it.
pub fn support_for(agent: AgentKind, version: Option<&str>) -> SteerSupport {
    if steer_gate_reason(agent, version).is_some() {
        SteerSupport::None
    } else {
        SteerSupport::Native
    }
}

/// Why Steer now is not available for `agent` at `version`, or `None` when it
/// is: the one source for [`support_for`] and [`support_info`]'s note, so
/// each gate carries its own reason. The first gate is the agent's `steer`
/// capability (its own reason, at any version); the second is a CLI version
/// floor for the agent whose transport depends on one.
fn steer_gate_reason(agent: AgentKind, version: Option<&str>) -> Option<String> {
    if let Some(reason) = crate::agent::unsupported_reason(agent, crate::agent::Feature::Steer) {
        return Some(reason);
    }
    match agent {
        AgentKind::Codex if !qualified_codex(version) => Some(format!(
            "{} needs a verified version (0.163.0 or newer) to take a message mid-turn. Open Agent updates to check it. You can queue a message for after, or Stop and send.",
            agent.label()
        )),
        _ => None,
    }
}

/// The recorded 2026-10-21 qualification starts at 0.163.0. Only a complete
/// stable version qualifies; unknown, partial and prerelease reads use exec.
fn qualified_codex(version: Option<&str>) -> bool {
    let Some(version) = version else { return false };
    let mut parts = version.split('.');
    let mut number = || {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse::<u64>().ok()
    };
    let parsed = (number(), number(), number());
    matches!(parsed, (Some(a), Some(b), Some(c)) if (a, b, c) >= (0, 163, 0))
        && parts.next().is_none()
}

/// Resolve outside LIVE's lock, once for a turn. The version cache is shared
/// with Agent updates and invalidated by binary path/mtime changes.
pub async fn resolve_support(agent: AgentKind) -> SteerSupportInfo {
    let version = if agent == AgentKind::Codex {
        crate::agent_cli::steering_version(agent).await
    } else {
        None
    };
    support_info(agent, version.as_deref())
}

/// One agent's row for the composer: what Steer now does, in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteerSupportInfo {
    pub agent: String,
    pub support: SteerSupport,
    /// One plain sentence the composer shows beside the choice.
    pub note: String,
}

/// Every agent's [`SteerSupportInfo`].
pub async fn support_table() -> Vec<SteerSupportInfo> {
    let mut rows = Vec::new();
    for agent in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Grok,
        AgentKind::Antigravity,
    ] {
        rows.push(resolve_support(agent).await);
    }
    rows
}

pub fn support_info(agent: AgentKind, version: Option<&str>) -> SteerSupportInfo {
    let support = support_for(agent, version);
    let note = steer_gate_reason(agent, version).unwrap_or_else(|| {
        format!(
            "{} takes your message at its next step, in this turn.",
            agent.label()
        )
    });
    SteerSupportInfo {
        agent: agent.as_str().to_string(),
        support,
        note,
    }
}

/// Where a steer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SteerState {
    /// Written to the agent (or waiting for the next attempt's input); the
    /// agent takes it at its next step.
    Sent,
    /// The agent took it into this turn.
    Delivered,
    /// The turn ended (or was stopped) before the agent took it.
    NotDelivered,
}

impl SteerState {
    pub fn as_str(self) -> &'static str {
        match self {
            SteerState::Sent => "sent",
            SteerState::Delivered => "delivered",
            SteerState::NotDelivered => "notDelivered",
        }
    }
}

/// One steer, as stored, emitted and read back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteerRecord {
    pub id: String,
    pub text: String,
    /// RFC 3339, when the person sent it.
    pub at: String,
    /// How many tool calls the turn had made when it was sent: where the
    /// chat draws it among the steps.
    pub after_tools: usize,
    pub state: SteerState,
    /// The agent's own words when it refused the steer ([`refuse`], P1-30),
    /// cut at [`MAX_REFUSAL_CHARS`]; absent otherwise and in older records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Who steered (COO parity L13 review): absent in the live record of the
    /// person's own steer (its stored copy says `person`), `agent` when the
    /// COO steered a chat it started (`sessions.steer_session`). An agent's
    /// steer is never the person's direction ([`crate::task_state`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// With `by: agent`, the steering run's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_run: Option<String>,
}

/// Who a steer is from: `None` for the person's own composer; for the COO's
/// `sessions.steer_session` (L13), the steering run's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerBy<'a> {
    Person,
    Agent { run_id: &'a str },
}

impl SteerBy<'_> {
    /// The stored record's `by`: `person` or `agent`.
    pub fn as_str(self) -> &'static str {
        match self {
            SteerBy::Person => "person",
            SteerBy::Agent { .. } => "agent",
        }
    }
}

/// A line for the running agent's input, a steer for it, or the end of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeMsg {
    Line(String),
    /// A person's steer for an agent whose input line only the attempt can
    /// write ([`input_msg`]): Codex's `turn/steer` names the running turn,
    /// which only the attempt's `codex_app_server::AppSession` knows.
    Steer {
        id: String,
        text: String,
    },
    /// Write the steers the attempt's session held until it knew the turn
    /// id (`codex_app_server::Step::held_steers`). Through the writer, so
    /// they go before any steer queued after them.
    HeldSteers,
    Close,
}

/// One running turn that may be steered.
struct Live {
    run_id: String,
    agent: AgentKind,
    support: SteerSupport,
    turn_ord: i64,
    vault: PathBuf,
    /// The turn's `chat-event` tag fields (`clientTurnId`, `queueId`,
    /// `conversationKey`), so a steer's event routes like the turn's own.
    tag: Map<String, Value>,
    /// The current attempt's input; `None` between attempts.
    pipe: Option<UnboundedSender<PipeMsg>>,
    /// The answer ended and the input is closed: a steer now is too late.
    closing: bool,
    steers: Vec<SteerRecord>,
    /// Steers accepted but not yet stored: never written to the agent, and
    /// left out of [`settle`], until [`publish`] (review: delivery must never
    /// precede the stored message).
    unpublished: Vec<String>,
    tools: usize,
    /// Counts attempts ([`attach`]), so a late-steer timer of an earlier
    /// attempt never closes a later one's input.
    attempt: u64,
    /// The answer's `result` came while a steer was on its way in: the input
    /// stays open for it ([`AfterResult::Waiting`]). Cleared when a steer
    /// starts a new step (that step's own `result` decides next).
    after_result: bool,
    /// Counts those waits, so a late-steer timer acts only on its own.
    wait: u64,
}

static LIVE: Mutex<Option<HashMap<String, Live>>> = Mutex::new(None);

fn with_live<R>(f: impl FnOnce(&mut HashMap<String, Live>) -> R) -> R {
    let mut guard = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// Removes the turn's entry when the turn ends (only its own run's).
#[must_use = "the turn stays steerable only while the guard lives"]
pub struct LiveGuard {
    session_key: String,
    run_id: String,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        with_live(|live| {
            if live
                .get(&self.session_key)
                .is_some_and(|entry| entry.run_id == self.run_id)
            {
                live.remove(&self.session_key);
            }
        });
    }
}

/// Register a running turn: from its transcript commit to its finish.
pub fn open(
    session_key: &str,
    run_id: &str,
    transport: (AgentKind, SteerSupport),
    turn_ord: i64,
    vault: PathBuf,
    tag: Map<String, Value>,
) -> LiveGuard {
    with_live(|live| {
        live.insert(
            session_key.to_string(),
            Live {
                run_id: run_id.to_string(),
                agent: transport.0,
                support: transport.1,
                turn_ord,
                vault,
                tag,
                pipe: None,
                closing: false,
                steers: Vec::new(),
                unpublished: Vec::new(),
                tools: 0,
                attempt: 0,
                after_result: false,
                wait: 0,
            },
        );
    });
    LiveGuard {
        session_key: session_key.to_string(),
        run_id: run_id.to_string(),
    }
}

/// What [`steer`] accepted: the record plus where to write and route it.
#[derive(Debug, Clone)]
pub struct Accepted {
    pub record: SteerRecord,
    pub vault: PathBuf,
    pub turn_ord: i64,
    pub tag: Map<String, Value>,
}

/// Steer now, first half: accept `text` for the running turn of
/// `session_key`. Nothing reaches the agent yet: the caller stores the
/// record, then [`publish`]es it (or [`forget`]s it when the store failed).
/// Only tests call this person-only form; the live paths call [`steer_by`].
#[cfg(test)]
pub fn steer(
    session_key: &str,
    text: &str,
    id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Accepted, &'static str> {
    steer_by(session_key, text, id, now, SteerBy::Person)
}

/// [`steer`] from `by`: the COO's steer of a chat it started carries
/// `by: agent` and its run in the live record too, so the turn's receipt and
/// the chat's event say whose it was.
pub fn steer_by(
    session_key: &str,
    text: &str,
    id: &str,
    now: chrono::DateTime<chrono::Utc>,
    by: SteerBy<'_>,
) -> Result<Accepted, &'static str> {
    let text = text.trim();
    // The stored record must stay whole under the session store's event cap,
    // escapes included (review: 1,500 control characters escape to 9,000).
    let serialized = serde_json::to_string(text).map_or(usize::MAX, |s| s.len());
    if text.is_empty() || text.chars().count() > MAX_STEER_CHARS || serialized > MAX_STEER_JSON {
        return Err(STEER_INVALID);
    }
    with_live(|live| {
        let entry = live.get_mut(session_key).ok_or(STEER_NOT_RUNNING)?;
        if entry.support != SteerSupport::Native {
            return Err(STEER_UNSUPPORTED);
        }
        if entry.closing {
            return Err(STEER_ENDING);
        }
        if entry.steers.len() >= MAX_STEERS_PER_TURN {
            return Err(STEER_INVALID);
        }
        let record = SteerRecord {
            id: id.to_string(),
            text: text.to_string(),
            at: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            after_tools: entry.tools,
            state: SteerState::Sent,
            reason: None,
            by: match by {
                SteerBy::Person => None,
                SteerBy::Agent { .. } => Some(by.as_str().to_string()),
            },
            by_run: match by {
                SteerBy::Person => None,
                SteerBy::Agent { run_id } => Some(run_id.to_string()),
            },
        };
        if entry.steers.iter().any(|s| s.id == id) {
            return Err(STEER_INVALID);
        }
        entry.steers.push(record.clone());
        entry.unpublished.push(id.to_string());
        Ok(Accepted {
            record,
            vault: entry.vault.clone(),
            turn_ord: entry.turn_ord,
            tag: entry.tag.clone(),
        })
    })
}

/// Steer now, second half: the record is stored, so write it to the agent
/// (at once when the attempt's input is open, else at the next attempt's
/// start, [`attach`]). [`STEER_ENDING`] when the turn closed its input or
/// ended meanwhile: the steer is dropped from the turn and the caller sends
/// it as the next message instead.
pub fn publish(session_key: &str, id: &str) -> Result<(), &'static str> {
    with_live(|live| {
        let Some(entry) = live.get_mut(session_key) else {
            return Err(STEER_ENDING);
        };
        let Some(at) = entry.unpublished.iter().position(|u| u == id) else {
            return Err(STEER_ENDING);
        };
        entry.unpublished.remove(at);
        if entry.closing {
            entry.steers.retain(|s| s.id != id);
            return Err(STEER_ENDING);
        }
        if let Some(steer) = entry.steers.iter().find(|s| s.id == id) {
            if let Some(pipe) = &entry.pipe {
                // A send fails only when the writer is gone (the attempt is
                // ending); the next attempt's `attach` writes it then.
                let _ = pipe.send(input_msg(entry.agent, id, &steer.text));
            }
        }
        Ok(())
    })
}

/// Drop an accepted steer that could not be stored.
pub fn forget(session_key: &str, id: &str) {
    with_live(|live| {
        if let Some(entry) = live.get_mut(session_key) {
            entry.unpublished.retain(|u| u != id);
            entry.steers.retain(|s| s.id != id);
        }
    });
}

/// The agent-input line for a person's message (Claude Code's stream-json
/// user message; `uuid` comes back on its `command_lifecycle` events).
pub fn input_line(agent: AgentKind, id: &str, text: &str) -> String {
    match agent {
        // Grok shares Claude's stream format but has no input channel, and a
        // Codex steer goes as [`PipeMsg::Steer`] ([`input_msg`]); only
        // Claude's lines are written as they are.
        AgentKind::Claude | AgentKind::Grok | AgentKind::Codex | AgentKind::Antigravity => json!({
            "type": "user",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
            "parent_tool_use_id": null,
            "session_id": "",
            "uuid": id,
        })
        .to_string(),
    }
}

/// What carries a person's message to `agent`'s running attempt: Claude's
/// [`input_line`], or for Codex a [`PipeMsg::Steer`] the attempt turns into
/// `turn/steer`.
pub fn input_msg(agent: AgentKind, id: &str, text: &str) -> PipeMsg {
    match agent {
        AgentKind::Codex => PipeMsg::Steer {
            id: id.to_string(),
            text: text.to_string(),
        },
        AgentKind::Claude | AgentKind::Grok | AgentKind::Antigravity => {
            PipeMsg::Line(input_line(agent, id, text))
        }
    }
}

/// An attempt's input opened (its first message already written): the
/// stored steers this turn took that no attempt delivered yet (an account
/// switch ended the attempt that had them) are written to `pipe` now, in
/// order, and later steers go there too. Under the registry lock, so a new
/// steer can never overtake an older one. Returns the attempt's number for
/// [`close_if_stalled`]; `None` when no such turn is registered.
pub fn attach(session_key: &str, run_id: &str, pipe: UnboundedSender<PipeMsg>) -> Option<u64> {
    with_live(|live| {
        let entry = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)?;
        entry.closing = false;
        entry.after_result = false;
        entry.attempt += 1;
        for steer in entry
            .steers
            .iter()
            .filter(|s| s.state == SteerState::Sent && !entry.unpublished.contains(&s.id))
        {
            let _ = pipe.send(input_msg(entry.agent, &steer.id, &steer.text));
        }
        entry.pipe = Some(pipe);
        Some(entry.attempt)
    })
}

/// The attempt's process is gone: its input with it.
pub fn detach(session_key: &str, run_id: &str) {
    with_live(|live| {
        if let Some(entry) = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)
        {
            entry.pipe = None;
        }
    });
}

/// An attempt that tracks its steers itself (Codex over app-server) says
/// where steer `id` went: `Delivered` when its message joined the turn,
/// `NotDelivered` when the agent refused it. Only a steer still `sent`
/// moves; returns it.
pub fn mark(session_key: &str, run_id: &str, id: &str, next: SteerState) -> Vec<SteerRecord> {
    mark_with(session_key, run_id, id, next, None)
}

/// The agent refused steer `id` (Codex's `turn/steer` error): `NotDelivered`
/// with its `reason`, cut at [`MAX_REFUSAL_CHARS`], so the chat can say why
/// (P1-30). Only a steer still `sent` moves; returns it.
pub fn refuse(session_key: &str, run_id: &str, id: &str, reason: &str) -> Vec<SteerRecord> {
    let reason: String = reason.trim().chars().take(MAX_REFUSAL_CHARS).collect();
    let reason = (!reason.is_empty()).then_some(reason);
    mark_with(session_key, run_id, id, SteerState::NotDelivered, reason)
}

fn mark_with(
    session_key: &str,
    run_id: &str,
    id: &str,
    next: SteerState,
    reason: Option<String>,
) -> Vec<SteerRecord> {
    with_live(|live| {
        let Some(entry) = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)
        else {
            return Vec::new();
        };
        entry
            .steers
            .iter_mut()
            .filter(|s| s.id == id && s.state == SteerState::Sent)
            .map(|s| {
                s.state = next;
                s.reason = reason.clone();
                s.clone()
            })
            .collect()
    })
}

/// The attempt's turn is over (Codex: `turn/completed`): its input closes,
/// so the agent exits, and from now on a steer gets [`STEER_ENDING`]. One
/// still `sent` waits for the next attempt ([`attach`]) or [`settle`].
pub fn close_input(session_key: &str, run_id: &str) {
    with_live(|live| {
        if let Some(entry) = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)
        {
            close_entry(entry);
        }
    });
}

/// The turn made another tool call (where later steers are drawn).
pub fn note_tool(session_key: &str, run_id: &str) {
    with_live(|live| {
        if let Some(entry) = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)
        {
            entry.tools += 1;
        }
    });
}

/// Read one stdout line of a steerable attempt: a `command_lifecycle` event
/// for one of this turn's steers moves it (`started`/`completed` →
/// delivered, `cancelled` → not delivered). Returns the steers that moved.
pub fn observe_line(session_key: &str, run_id: &str, line: &str) -> Vec<SteerRecord> {
    if !line.contains("command_lifecycle") {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    if value.get("type").and_then(Value::as_str) != Some("command_lifecycle") {
        return Vec::new();
    }
    let Some(id) = value.get("command_uuid").and_then(Value::as_str) else {
        return Vec::new();
    };
    let next = match value.get("state").and_then(Value::as_str) {
        Some("started" | "completed") => SteerState::Delivered,
        Some("cancelled") => SteerState::NotDelivered,
        _ => return Vec::new(),
    };
    with_live(|live| {
        let Some(entry) = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)
        else {
            return Vec::new();
        };
        let moved: Vec<SteerRecord> = entry
            .steers
            .iter_mut()
            .filter(|s| s.id == id && s.state == SteerState::Sent)
            .map(|s| {
                s.state = next;
                s.clone()
            })
            .collect();
        if entry.after_result && !moved.is_empty() {
            if next == SteerState::Delivered {
                // A new step runs; its own `result` closes the input.
                entry.after_result = false;
            } else if !has_untaken(entry) {
                // Review: the steer the input waited for was cancelled and
                // nothing else is on its way, so no `result` will follow.
                close_entry(entry);
            }
        }
        moved
    })
}

/// A published steer the agent hasn't taken yet.
fn has_untaken(entry: &Live) -> bool {
    entry
        .steers
        .iter()
        .any(|s| s.state == SteerState::Sent && !entry.unpublished.contains(&s.id))
}

/// What a `result` line means for the attempt's input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterResult {
    /// Nothing more is coming: the input was closed, so the agent exits.
    Closed,
    /// A steer is still on its way in (written but not yet taken, or the
    /// CLI says more turns follow): the input stays open for it. The caller
    /// checks again after [`LATE_STEER_GRACE`] ([`close_if_stalled`] with
    /// this wait's number).
    Waiting(u64),
    /// Not a steerable turn (or not this run's).
    NotLive,
}

/// A `result` line arrived. `queued` is its `queued_turn_count` (0 when
/// absent).
pub fn on_result(session_key: &str, run_id: &str, queued: u64) -> AfterResult {
    with_live(|live| {
        let Some(entry) = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)
        else {
            return AfterResult::NotLive;
        };
        if queued > 0 || has_untaken(entry) {
            entry.after_result = true;
            entry.wait += 1;
            return AfterResult::Waiting(entry.wait);
        }
        close_entry(entry);
        AfterResult::Closed
    })
}

/// The late-steer grace ran out for wait `wait` of attempt `attempt`: close
/// the input if that wait is still open, i.e. no steer started a new step
/// since that `result` (a step that started keeps it open until its own
/// `result`). Never another attempt's or another wait's input. True when it
/// closed; from then on a steer gets [`STEER_ENDING`]. Closing with a steer
/// still unread is safe: Claude Code runs what it already read before EOF.
pub fn close_if_stalled(session_key: &str, run_id: &str, attempt: u64, wait: u64) -> bool {
    with_live(|live| {
        let Some(entry) = live.get_mut(session_key).filter(|entry| {
            entry.run_id == run_id
                && entry.attempt == attempt
                && entry.wait == wait
                && entry.after_result
                && entry.pipe.is_some()
        }) else {
            return false;
        };
        close_entry(entry);
        true
    })
}

fn close_entry(entry: &mut Live) {
    entry.closing = true;
    if let Some(pipe) = entry.pipe.take() {
        let _ = pipe.send(PipeMsg::Close);
    }
}

/// The turn is finishing: every steer it never delivered is marked so.
/// Returns all of the turn's steers (for the receipt) and those that moved.
pub fn settle(session_key: &str, run_id: &str) -> (Vec<SteerRecord>, Vec<SteerRecord>) {
    with_live(|live| {
        let Some(entry) = live
            .get_mut(session_key)
            .filter(|entry| entry.run_id == run_id)
        else {
            return (Vec::new(), Vec::new());
        };
        close_entry(entry);
        // A steer still being stored is the caller's ([`publish`] then
        // refuses it as ending), never on this turn's receipt.
        let unpublished = std::mem::take(&mut entry.unpublished);
        entry.steers.retain(|s| !unpublished.contains(&s.id));
        let mut moved = Vec::new();
        for steer in &mut entry.steers {
            if steer.state == SteerState::Sent {
                steer.state = SteerState::NotDelivered;
                moved.push(steer.clone());
            }
        }
        (entry.steers.clone(), moved)
    })
}

/// The `chat-event` for a steer, before the turn's tag is applied.
pub fn steer_event(session_key: &str, record: &SteerRecord) -> Value {
    crate::chat_event::build::steer(session_key, record)
}

/// The stored `steerState` payload, with a refusal's reason (P1-30).
pub fn state_payload(record: &SteerRecord) -> String {
    let mut payload = json!({ "id": record.id, "state": record.state.as_str() });
    if let Some(reason) = &record.reason {
        payload["reason"] = json!(reason);
    }
    payload.to_string()
}

/// Apply a turn's tag fields to a payload (the command path, which has no
/// `TurnEvents`).
pub fn tagged(mut payload: Value, tag: &Map<String, Value>) -> Value {
    if let Some(object) = payload.as_object_mut() {
        for (key, value) in tag {
            object.insert(key.clone(), value.clone());
        }
    }
    payload
}

// ── Held sends: Queue for after, Stop and send ──────────────────────────────

/// What the person chose for a send while a turn runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum After {
    /// Send it when the running turn ends.
    Queue,
    /// Stop the running turn, then send it.
    Stop,
}

impl After {
    pub fn parse(value: Option<&str>) -> Result<Option<After>, String> {
        match value.map(str::trim) {
            None | Some("") => Ok(None),
            Some("queue") => Ok(Some(After::Queue)),
            Some("stop") => Ok(Some(After::Stop)),
            Some(other) => Err(format!("Unknown send choice: {other}.")),
        }
    }
}

/// The person's send as it is held: `commands::ChatSendInput` for their own
/// composer (depth 0, attended, typed unless canned), in a form that
/// survives a restart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HeldInput {
    pub prompt: String,
    pub session_id: Option<String>,
    pub session_key: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub agent: Option<String>,
    pub client_turn_id: Option<String>,
    pub ultracode: bool,
    pub canvases: Option<String>,
    pub continued_from: Vec<String>,
    pub canned: Option<String>,
    /// `ChatSendInput::typed` as `chat_send` set it, kept as it was.
    pub typed: bool,
    /// L1: an agent's `send_to_session` to a conversation with a turn
    /// running is held like the person's Queue for after. Its depth (0 for
    /// the person's own), its effort, whether it is unattended, and the
    /// asking run's delegation record (JSON) with that run's id, stored once
    /// the held send is queued so its end is still reported to the asker.
    #[serde(skip_serializing_if = "is_zero")]
    pub app_depth: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unattended: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation_run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegation_record: Option<String>,
    /// How many times an agent's held send failed to queue (L1 review):
    /// it is tried again on the next drain, and dropped after the third.
    #[serde(skip_serializing_if = "is_zero")]
    pub attempts: u32,
    /// An agent's held send: the sender's cap on the conversation's own
    /// mode (`app_sessions::SendMode::cap`), applied to the mode the
    /// conversation has when the send goes (`commands::refreshed_agent_input`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_cap: Option<String>,
    /// An agent's held send lifted to its sender's own mode
    /// (`app_sessions::SendMode::lift`): the mode the conversation runs the
    /// send in when that is stronger than its own stored one, still capped
    /// by `mode_cap` (`commands::refreshed_agent_input`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode_lift: Option<String>,
    /// R8b: why this held send failed to queue. A failed send stays saved,
    /// never sent again, until the window says it holds the person's words
    /// (`chat_ack_held_failed`), so a quit between the `heldFailed` event and
    /// the composer keeping them loses nothing: the next launch reports it
    /// once more ([`unreported_failures`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// One held send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    pub id: String,
    pub after: After,
    /// RFC 3339, when the person sent it.
    pub at: String,
    pub input: HeldInput,
}

type HeldMap = HashMap<String, VecDeque<Held>>;

/// The held sends in memory: the ones waiting, and the ones a drain has
/// taken and not yet settled.
#[derive(Clone, Default)]
struct Store {
    queues: HeldMap,
    /// Taken by [`take_held`] and not yet queued ([`settle_held`]) or put
    /// back ([`put_back`]): out of the queue in memory, but still saved at
    /// the front of its conversation's queue (2026-11-13: the removal used
    /// to be saved at the take, so a quit between the take and the run
    /// queue's commit lost the message).
    sending: HashMap<String, Held>,
    /// Failed held sends already reported to the window in this process
    /// (R8b): only the others are reported again, and keep the sweeper up.
    reported: std::collections::HashSet<String>,
}

impl Store {
    /// What is saved: every send not yet queued, a taken one first.
    fn saved(&self) -> HeldMap {
        let mut out = self.queues.clone();
        for (key, item) in &self.sending {
            out.entry(key.clone()).or_default().push_front(item.clone());
        }
        out.retain(|_, queue| !queue.is_empty());
        out
    }
}

/// Held sends, by conversation; loaded from [`held_file`] on first use and
/// written back after every change (review: a held send the person was told
/// about must survive a quit; the next launch sends it once the
/// conversation is idle).
static HELD: Mutex<Option<Store>> = Mutex::new(None);

#[cfg(not(test))]
fn held_file() -> Option<PathBuf> {
    Some(crate::instance::support_dir()?.join("held-sends.json"))
}

#[cfg(test)]
fn held_file() -> Option<PathBuf> {
    None
}

/// The saved held sends. A file that can't be read or parsed is moved aside
/// (kept for recovery, never overwritten) and the store starts empty.
fn load_held() -> HeldMap {
    let Some(path) = held_file() else {
        return HeldMap::new();
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return HeldMap::new(),
        Err(error) => {
            eprintln!("[steering] reading held sends failed: {error}");
            set_aside(&path);
            return HeldMap::new();
        }
    };
    serde_json::from_slice::<HeldMap>(&bytes).unwrap_or_else(|error| {
        eprintln!("[steering] held sends unreadable ({error}); kept aside");
        set_aside(&path);
        HeldMap::new()
    })
}

/// False once an unreadable file couldn't be moved aside: then nothing is
/// saved over it (holds are refused) until Hollow restarts.
static HELD_WRITABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn set_aside(path: &std::path::Path) {
    for _ in 0..8 {
        let aside = path.with_extension(format!(
            "unreadable-{}-{:08x}.json",
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.f"),
            rand::random::<u32>()
        ));
        if aside.exists() {
            continue;
        }
        if std::fs::rename(path, &aside).is_ok() {
            return;
        }
        break;
    }
    eprintln!("[steering] the unreadable held sends couldn't be moved aside; holding nothing new");
    HELD_WRITABLE.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Run `f` on the held sends. When it changed them they are saved before
/// the change counts: a save that fails puts the old ones back and is an
/// error (review: a send the person was told is held must survive a quit,
/// and a cancel that didn't save must not report success).
fn with_held<R>(f: impl FnOnce(&mut HeldMap) -> (R, bool)) -> Result<R, String> {
    with_held_as(Save::OrUndo, f)
}

fn with_held_as<R>(save: Save, f: impl FnOnce(&mut HeldMap) -> (R, bool)) -> Result<R, String> {
    with_store_as(save, |store| f(&mut store.queues))
}

/// What a failed save does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Save {
    /// Undo the change and fail (a hold, a cancel, a take).
    OrUndo,
    /// Keep the change in memory anyway (putting a taken send back, or
    /// settling a queued one: undoing it would lose or repeat the send; the
    /// sweeper still sends from memory, and the next save writes it).
    OrKeep,
}

fn with_store_as<R>(save: Save, f: impl FnOnce(&mut Store) -> (R, bool)) -> Result<R, String> {
    let mut guard = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    let held = guard.get_or_insert_with(|| Store {
        queues: load_held(),
        sending: HashMap::new(),
        reported: Default::default(),
    });
    let before = held.clone();
    let (out, changed) = f(held);
    held.queues.retain(|_, queue| !queue.is_empty());
    if changed {
        if let Some(path) = held_file() {
            let saved = if HELD_WRITABLE.load(std::sync::atomic::Ordering::SeqCst) {
                serde_json::to_vec(&held.saved())
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| crate::vault::atomic_write_private(&path, &bytes))
            } else {
                Err("an unreadable earlier file is in the way".to_string())
            };
            if let Err(error) = saved {
                let error = format!("Hollow couldn't save the waiting message: {error}");
                if save == Save::OrKeep {
                    eprintln!("[steering] {error}; kept in memory, saved again later");
                    HELD_DIRTY.store(true, std::sync::atomic::Ordering::SeqCst);
                    return Ok(out);
                }
                *held = before;
                return Err(error);
            }
            HELD_DIRTY.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    Ok(out)
}

/// A kept-in-memory change ([`Save::OrKeep`]) is still unsaved.
static HELD_DIRTY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Save the held sends again if a kept change is still unsaved (the
/// sweeper, every few seconds while anything is held).
pub fn retry_save() {
    if !HELD_DIRTY.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let _ = with_held_as(Save::OrKeep, |_| ((), true));
}

/// The reason a send for `kind` can't be held (its `queue` capability is
/// false), or `None`; `None` for an unresolved agent (the caller resolves the
/// default first). Pure, so a caller asks it before any side effect: Stop and
/// send must not stop a turn for a send that then can't be held.
pub fn queue_refusal(kind: Option<AgentKind>) -> Option<String> {
    kind.and_then(|kind| crate::agent::require(kind, crate::agent::Feature::Queue).err())
}

/// [`queue_refusal`] for an explicit capability row and label (tests).
#[cfg(test)]
fn queue_refusal_in(row: crate::agent::Flags, label: &str) -> Option<String> {
    crate::agent::reason_in(row, label, crate::agent::Feature::Queue)
}

/// Hold a send behind any the conversation already holds. `Err` when it
/// already holds [`MAX_HELD`], or (AC1) when the send names an agent whose
/// `queue` capability is false: that refusal comes first, before anything is
/// stored or saved, with the capability's reason. A send that names no agent
/// (the chat's default) is checked by the caller once it has resolved one:
/// `agent::require(kind, Feature::Queue)`.
pub fn hold(session_key: &str, item: Held) -> Result<(), String> {
    if let Some(reason) = queue_refusal(item.input.agent.as_deref().and_then(AgentKind::from_opt)) {
        return Err(reason);
    }
    with_held(|held| {
        let queue = held.entry(session_key.to_string()).or_default();
        // A failed one awaits only the window's acknowledgement (R8b): it
        // is not waiting for a turn, so it doesn't count against the cap.
        if queue
            .iter()
            .filter(|held| held.input.failed.is_none())
            .count()
            >= MAX_HELD
        {
            let error =
                format!("This chat already has {MAX_HELD} messages waiting for the turn to end.");
            return (Err(error), false);
        }
        queue.push_back(item);
        (Ok(()), true)
    })?
}

/// The next held send of `session_key`, oldest first, taken for sending.
/// Nothing is saved: it stays in the saved file, first in its queue, until
/// the drain settles it once its turn is queued ([`settle_held`]) or puts
/// it back ([`put_back`]), so a quit in between never loses it. One is in
/// flight per conversation (the drain runs in `HELD_ORDER`): a taken one
/// never settled (a drain that stopped part way) goes back first.
pub fn take_held(session_key: &str) -> Option<Held> {
    with_store_as(Save::OrKeep, |store| {
        if let Some(stale) = store.sending.remove(session_key) {
            store
                .queues
                .entry(session_key.to_string())
                .or_default()
                .push_front(stale);
        }
        // A failed one waits for the window's acknowledgement, never retried.
        let next = store.queues.get_mut(session_key).and_then(|queue| {
            let at = queue.iter().position(|item| item.input.failed.is_none())?;
            queue.remove(at)
        });
        if let Some(item) = &next {
            store.sending.insert(session_key.to_string(), item.clone());
        }
        // The saved sends are the same: the taken one is still among them.
        (next, false)
    })
    .unwrap_or_else(|error| {
        eprintln!("[steering] {error}");
        None
    })
}

/// The taken send `id` of `session_key` went (its turn is queued) or is
/// given up on: remove it, and save that. A save that fails keeps the
/// removal in memory for the sweeper's next save; a quit before then leaves
/// it in the file, and the drain finds its queued row by [`queue_key`]
/// instead of sending it twice. True when it was the one taken.
pub fn settle_held(session_key: &str, id: &str) -> bool {
    with_store_as(Save::OrKeep, |store| {
        let taken = store
            .sending
            .get(session_key)
            .is_some_and(|item| item.id == id);
        if taken {
            store.sending.remove(session_key);
        }
        (taken, taken)
    })
    .unwrap_or_else(|error| {
        eprintln!("[steering] {error}");
        false
    })
}

/// The taken send `id` of `session_key` could not be queued and the person's
/// window has to be told (R8b): keep it saved, marked failed with `error`,
/// instead of removing it like [`settle_held`]. It is never sent again; the
/// window's acknowledgement ([`ack_failed`]) removes it. It counts as
/// reported in this process. True when it was the one taken.
pub fn mark_failed(session_key: &str, id: &str, error: &str) -> bool {
    with_store_as(Save::OrKeep, |store| {
        let taken = store
            .sending
            .get(session_key)
            .is_some_and(|item| item.id == id);
        if !taken {
            return (false, false);
        }
        let mut item = store.sending.remove(session_key).unwrap();
        item.input.failed = Some(error.to_string());
        store.reported.insert(item.id.clone());
        store
            .queues
            .entry(session_key.to_string())
            .or_default()
            .push_front(item);
        (true, true)
    })
    .unwrap_or_else(|error| {
        eprintln!("[steering] {error}");
        false
    })
}

/// The failed held sends of `session_key` not yet reported in this process
/// (found in the saved file after a restart), now counted as reported.
pub fn unreported_failures(session_key: &str) -> Vec<Held> {
    with_store_as(Save::OrKeep, |store| {
        let found: Vec<Held> = store
            .queues
            .get(session_key)
            .into_iter()
            .flatten()
            .filter(|item| item.input.failed.is_some() && !store.reported.contains(&item.id))
            .cloned()
            .collect();
        for item in &found {
            store.reported.insert(item.id.clone());
        }
        (found, false)
    })
    .unwrap_or_default()
}

/// The run-queue key a held send's turn is queued under
/// (`run_queue::row_with_key`): a send found queued after a restart is not
/// sent again.
pub fn queue_key(id: &str) -> String {
    format!("held:{id}")
}

/// Put a send back at the front (its dispatch was refused: another send got
/// in first, so it waits for that turn instead). The taken copy is replaced,
/// never doubled: `item` may carry a changed `attempts`.
pub fn put_back(session_key: &str, item: Held) {
    with_store_as(Save::OrKeep, |store| {
        if store
            .sending
            .get(session_key)
            .is_some_and(|taken| taken.id == item.id)
        {
            store.sending.remove(session_key);
        }
        store
            .queues
            .entry(session_key.to_string())
            .or_default()
            .push_front(item);
        ((), true)
    })
    .unwrap_or_else(|error| eprintln!("[steering] {error}"));
}

/// The held sends as the next save writes them (tests: what a restart
/// would load).
#[cfg(test)]
pub(crate) fn saved_view() -> HeldMap {
    let mut guard = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    let held = guard.get_or_insert_with(Store::default);
    serde_json::from_slice(&serde_json::to_vec(&held.saved()).unwrap()).unwrap()
}

/// Cancel one held send (the chip's Cancel). `Ok(true)` when it was still
/// held; an error when the cancel couldn't be saved (it stays held).
pub fn cancel_held(session_key: &str, id: &str) -> Result<bool, String> {
    with_held(|held| {
        let Some(queue) = held.get_mut(session_key) else {
            return (false, false);
        };
        // A failed send is already settled (R8b): it is not cancellable, so
        // Stop and a chip's Cancel see it as gone, as they did before it
        // was kept for the window's acknowledgement ([`ack_failed`]).
        let before = queue.len();
        queue.retain(|item| item.id != id || item.input.failed.is_some());
        let removed = queue.len() != before;
        (removed, removed)
    })
}

/// The window holds a failed held send's words (R8b): remove the failed
/// entry `id`. `Ok(true)` when one was removed; never touches a send still
/// held, so it can't cancel anything the person could still Stop.
pub fn ack_failed(session_key: &str, id: &str) -> Result<bool, String> {
    with_held(|held| {
        let Some(queue) = held.get_mut(session_key) else {
            return (false, false);
        };
        let before = queue.len();
        queue.retain(|item| item.id != id || item.input.failed.is_none());
        let removed = queue.len() != before;
        (removed, removed)
    })
}

/// The COO's `sessions.cancel_held_send` (L13 review): cancel held send
/// `id` only when `run_id` is the run that sent it (its `delegation_run`),
/// checked under the same lock that removes it. `Ok(false)` when it is no
/// longer held; an error, and it stays held, when another sender's (the
/// person's own Queue for after included).
pub fn cancel_held_from(session_key: &str, id: &str, run_id: &str) -> Result<bool, String> {
    with_held(|held| {
        let Some(queue) = held.get_mut(session_key) else {
            return (Ok(false), false);
        };
        let Some(at) = queue.iter().position(|item| item.id == id) else {
            return (Ok(false), false);
        };
        let sender = queue[at].input.delegation_run.as_deref();
        if run_id.is_empty() || sender != Some(run_id) {
            return (
                Err(format!("{id} isn't a message you sent (sessions.send returns the heldId of yours); it stays held. Nothing changed.")),
                false,
            );
        }
        queue.remove(at);
        (Ok(true), true)
    })?
}

/// How many sends `session_key` holds, a taken one not yet settled included
/// (a drain that stopped part way leaves it for the next).
pub fn held_count(session_key: &str) -> usize {
    with_store_as(Save::OrUndo, |store| {
        let waiting = store.queues.get(session_key).map_or(0, |queue| {
            queue
                .iter()
                .filter(|item| item.input.failed.is_none())
                .count()
        });
        (
            waiting + usize::from(store.sending.contains_key(session_key)),
            false,
        )
    })
    .unwrap_or(0)
}

/// Every conversation holding a send (the sweeper's list).
pub fn held_conversations() -> Vec<String> {
    with_store_as(Save::OrUndo, |store| {
        // A failed one only until it is reported (R8b), or while a save is
        // still pending (so the sweeper's `retry_save` writes its mark);
        // then it waits for the window's acknowledgement without keeping
        // the sweeper up.
        let dirty = HELD_DIRTY.load(std::sync::atomic::Ordering::SeqCst);
        let mut keys: Vec<String> = store
            .saved()
            .into_iter()
            .filter(|(_, queue)| {
                queue.iter().any(|item| {
                    item.input.failed.is_none() || dirty || !store.reported.contains(&item.id)
                })
            })
            .map(|(key, _)| key)
            .collect();
        keys.sort();
        (keys, false)
    })
    .unwrap_or_default()
}

#[cfg(test)]
#[path = "steering_tests.rs"]
mod tests;
