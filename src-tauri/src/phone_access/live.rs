//! The live buffer and the event streams.
//!
//! **The live buffer.** Hollow stores an answer's prose only when its turn
//! ends, so a phone following a running turn reads it from here: per running
//! turn, the answer text Hollow's window shows so far and the tool calls
//! seen. Every `chat-event` reaches [`tap`] from `chat::emit_event` (the one
//! place they leave Rust, in the order each thread emits them), and
//! [`Fold`] applies them the way the window's reducer does
//! (`src/chatTranscript.ts` and `handleChatEvent` in
//! `src/store/chatRuntime.ts`): deltas join the open streamed bubble, a full
//! text block joins the newest bubble after a blank line, `turnBoundary`, a
//! tool call after a boundary and a notice row start a new bubble. The
//! phone's text is the turn's bubbles joined by a blank line. The window only
//! ever adds to a running answer, so `gen` stays 1 for a turn; it would rise
//! only if the text were rebuilt. At most [`TEXT_CAP`] bytes and
//! [`TOOLS_CAP`] tools are kept; past the text cap nothing more is added and
//! `truncated` is set.
//!
//! **Only while a phone is paired.** [`tap`] copies nothing unless the
//! iPhone app is on and at least one phone is paired (with the iPhone app
//! off it never copies). A copy carries the watching period it was made in,
//! which runs from the moment both hold until the switch goes off or the
//! last phone is revoked ([`Live::follow`]); when it ends every live turn is
//! dropped, and a copy from an earlier period is never folded, so no live
//! view has a gap. Streams opening and closing leave the period alone: the
//! iPhone app swaps its stream when the person changes chats and drops it in
//! the background, and the running turn is still whole when it comes back
//! (second review, finding 1, 2026-11-04). A turn already running when the
//! period started has no live view (`live` is null) until it ends; the app
//! sees it running and refetches it once its row changes.
//!
//! **The streams** (`GET /v1/events`): `hello` first ([`hello`]), then for the stream's
//! chat `live` and `state`; afterwards `append` (coalesced to at most 8 a
//! second), `live` (a turn starts, its tools change or it is truncated;
//! coalesced the same way, and replacing any appends not yet sent), `turn`
//! and `state` for that chat, `sessions` (at most one a second) on every
//! stream, and `ping` every 15 seconds. At most [`PER_DEVICE`] streams per
//! phone and [`IN_ALL`] in all: a new one closes the oldest. What a stream
//! hasn't sent yet is capped in bytes: a new `live` frame replaces every
//! unsent `live` and `append` frame (it carries the whole view), and a stream
//! more than [`STREAM_BYTES`] behind besides its newest `live` frame is
//! closed, dropping what it held (the app reconnects and refetches).
//!
//! **Ids and resume (AG5, 2026-11-17).** Every frame but `hello` and `resync`
//! carries an `id:` line, `<epoch>-<chat>-<list>-<bind>` (hex epoch, decimal
//! cursors, hex bind): a position in a hub-wide, numbered history of what
//! happened, not a frame counter, so an id never goes down but two frames may
//! share one. `chat` says every logged event of the stream's chat up to it was
//! queued to the stream, `list` the same for `sessions` hints (held back
//! separately because they are rate limited and replaying one twice is
//! harmless, while an `append` twice is not), `bind` names the followed chat
//! so an id can't resume another chat. The epoch is new for every [`Live`] and
//! every watching period ([`Live::watch`]), so a restart or a gap in what was
//! copied can never resume. A new stream sending `Last-Event-ID` (or
//! `lastEventId` in the query) is replayed from a bounded in-memory ring
//! ([`RING_EVENTS`] events and [`RING_BYTES`] bytes, oldest dropped first,
//! consecutive appends of one chat merged until some stream has been stamped
//! inside them): `hello` says `resumed: true`, then the missed `append`s
//! (never one the client holds), a whole `live` if its tools changed, `turn`s,
//! one `sessions` hint, and the current `state`. When the id can't be honoured
//! (another epoch, older than the ring, forged, another chat) the stream says
//! `resumed: false` and sends `resync` with a `reason`, then the usual `live`
//! and `state`: the client refetches, as it did before ids existed. A consumer
//! never invents a finish for a stream that ended without a `turn`.

use super::wire::{self, AppendEvent, StateEvent, TurnEvent};
use hyper::body::Bytes;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Live text kept per running turn, in bytes.
pub(crate) const TEXT_CAP: usize = 256 * 1024;
/// Tool calls kept per running turn (the first ones, as a stored turn).
pub(crate) const TOOLS_CAP: usize = 200;
/// `append` and coalesced `live` frames, at most this often per stream.
pub(crate) const APPEND_EVERY: Duration = Duration::from_millis(125);
/// `sessions`, at most this often per stream.
pub(crate) const SESSIONS_EVERY: Duration = Duration::from_secs(1);
pub(crate) const PING_EVERY: Duration = Duration::from_secs(15);
/// How often the list rows and the followed chats' states are compared with
/// what was sent (covers what no `chat-event` announces: a composer send
/// still queued, a new title, an archive).
pub(crate) const RECONCILE_EVERY: Duration = Duration::from_secs(3);
pub(crate) const PER_DEVICE: usize = 4;
pub(crate) const IN_ALL: usize = 16;
/// Bytes a stream may hold unsent besides its newest `live` frame before it
/// is closed as too slow (review finding 6, 2026-11-04: a cap in frames let
/// one stream hold 256 live frames of up to 256 KiB each).
pub(crate) const STREAM_BYTES: usize = 1024 * 1024;
/// AG5: events the resume ring keeps, in all chats together. A consecutive run
/// of one chat's appends is one event while no stream was stamped inside it,
/// so this is minutes of a busy turn, not seconds.
pub(crate) const RING_EVENTS: usize = 1024;
/// AG5: bytes the resume ring keeps (text, keys and [`RING_ENTRY_COST`] each),
/// on top of the live views ([`TEXT_CAP`] each) and the stream queues.
pub(crate) const RING_BYTES: usize = 512 * 1024;
/// AG5: what an entry costs besides its text and keys.
const RING_ENTRY_COST: usize = 64;
/// AG5: a merged run of appends stops growing here.
const RING_MERGE_TEXT: usize = 16 * 1024;
/// The list rows the reconcile compares (the newest ones).
const RECONCILE_ROWS: usize = 200;
/// A tool call's `detail`, at most this many characters.
pub(crate) const DETAIL_MAX: usize = 120;

// ── The fold ─────────────────────────────────────────────────────────────

/// One tool call as the phone sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolRow {
    pub id: String,
    pub name: String,
    /// `running`, `done` or `error`.
    pub status: &'static str,
    pub detail: Option<String>,
}

impl ToolRow {
    pub(crate) fn wire(&self) -> wire::Tool {
        wire::Tool {
            id: self.id.clone(),
            name: self.name.clone(),
            status: self.status.to_string(),
            detail: self.detail.clone(),
        }
    }
}

/// What a tool call did, from its input: `description`, then `file_path`
/// or `path`, then the first line of `command`, then `query`, then `url`;
/// at most [`DETAIL_MAX`] characters. Nothing else of the input, and never
/// a result.
pub(crate) fn tool_detail(input: &Value) -> Option<String> {
    let text = |key: &str| -> Option<String> {
        match input.get(key)? {
            Value::String(s) => Some(s.clone()),
            Value::Array(parts) if key == "command" => Some(
                parts
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        }
    };
    let pick = |key: &str| -> Option<String> {
        let value = text(key)?;
        let value = if key == "command" {
            value.lines().find(|l| !l.trim().is_empty())?.to_string()
        } else {
            value
        };
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    };
    let found = pick("description")
        .or_else(|| pick("file_path"))
        .or_else(|| pick("path"))
        .or_else(|| pick("command"))
        .or_else(|| pick("query"))
        .or_else(|| pick("url"))?;
    Some(cut_chars(&found, DETAIL_MAX))
}

/// `text` cut to at most `max` characters.
pub(crate) fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

/// How an error `result` reads in the window (`src/resultKind.ts`
/// `resultErrorText`).
fn result_error_text(text: &str, kind: Option<&str>) -> String {
    let label = match kind {
        Some("maxTurns") => Some("turn limit reached"),
        Some("providerQuota") => Some("usage limit reached"),
        Some("transient") => Some("rate limited, overloaded or network error"),
        Some("auth") => Some("sign-in problem"),
        Some("resumeFailed") => Some("the previous agent session could not be resumed"),
        _ => None,
    };
    let body = match label {
        Some(label) if !text.starts_with(label) => format!("{label}: {text}"),
        _ => text.to_string(),
    };
    format!("(error: {body})")
}

/// One running turn's answer as the window draws it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Fold {
    text: String,
    truncated: bool,
    tools: Vec<ToolRow>,
    /// The newest transcript row is an assistant bubble of this turn.
    in_bubble: bool,
    /// That bubble is a streamed one (deltas join it).
    partial: bool,
    /// That bubble has text (a full block joins it after a blank line).
    has_text: bool,
    /// `turnBoundary` seen: the next text starts a new bubble.
    boundary: bool,
    /// Any assistant bubble since the turn started (a receipt joins it).
    any_bubble: bool,
}

/// What one event changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Change {
    /// Text added to the end.
    pub appended: String,
    /// The tool list changed.
    pub tools: bool,
    /// This event hit the text cap.
    pub truncated: bool,
}

impl Fold {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn tools(&self) -> &[ToolRow] {
        &self.tools
    }

    /// Add `piece` to the end, up to [`TEXT_CAP`].
    fn push(&mut self, piece: &str, change: &mut Change) {
        if self.truncated || piece.is_empty() {
            return;
        }
        let room = TEXT_CAP - self.text.len();
        let take = if piece.len() <= room {
            piece
        } else {
            let mut end = room;
            while !piece.is_char_boundary(end) {
                end -= 1;
            }
            self.truncated = true;
            change.truncated = true;
            &piece[..end]
        };
        self.text.push_str(take);
        change.appended.push_str(take);
    }

    /// Text into the newest bubble. A bubble's first text follows the
    /// earlier bubbles after a blank line.
    fn add_to_bubble(&mut self, piece: &str, change: &mut Change) {
        if piece.is_empty() {
            return;
        }
        if !self.has_text {
            if !self.text.is_empty() {
                self.push("\n\n", change);
            }
            self.has_text = true;
        }
        self.push(piece, change);
    }

    fn new_bubble(&mut self, partial: bool) {
        self.in_bubble = true;
        self.any_bubble = true;
        self.partial = partial;
        self.has_text = false;
        self.boundary = false;
    }

    /// `appendText`: a full block.
    fn append_text(&mut self, text: &str, change: &mut Change) {
        if !self.boundary && self.in_bubble {
            if self.has_text {
                self.add_to_bubble(&format!("\n\n{text}"), change);
            } else {
                self.add_to_bubble(text, change);
            }
        } else {
            self.new_bubble(false);
            self.add_to_bubble(text, change);
        }
    }

    /// `appendDelta`: streamed text.
    fn append_delta(&mut self, delta: &str, change: &mut Change) {
        if !(!self.boundary && self.in_bubble && self.partial) {
            self.new_bubble(true);
        }
        self.add_to_bubble(delta, change);
    }

    fn tool_use(&mut self, id: &str, name: &str, input: &Value, change: &mut Change) {
        if self.tools.len() < TOOLS_CAP {
            self.tools.push(ToolRow {
                id: id.to_string(),
                name: name.to_string(),
                status: "running",
                detail: tool_detail(input),
            });
            change.tools = true;
        }
        if self.boundary || !self.in_bubble {
            self.new_bubble(false);
        }
    }

    /// Newest first: by id, or without one the newest running call.
    fn tool_result(&mut self, id: &str, is_error: bool, change: &mut Change) {
        let found = self.tools.iter_mut().rev().find(|t| {
            if id.is_empty() {
                t.status == "running"
            } else {
                t.id == id
            }
        });
        if let Some(tool) = found {
            let status = if is_error { "error" } else { "done" };
            if tool.status != status {
                tool.status = status;
                change.tools = true;
            }
        }
    }

    /// `finalize`: streamed bubbles close and running calls read as done.
    fn finalize(&mut self, change: &mut Change) {
        self.partial = false;
        for tool in self.tools.iter_mut().filter(|t| t.status == "running") {
            tool.status = "done";
            change.tools = true;
        }
    }

    /// A row that isn't the answer (a notice, a steer): the next text
    /// starts a new bubble.
    fn notice(&mut self) {
        self.in_bubble = false;
    }

    /// Apply one `chat-event` of this turn.
    pub(crate) fn apply(&mut self, event: &Value) -> Change {
        let mut change = Change::default();
        let text = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or("");
        match text("type") {
            "assistantText" => self.append_text(text("text"), &mut change),
            "assistantDelta" => self.append_delta(text("text"), &mut change),
            "turnBoundary" => self.boundary = true,
            "toolUse" => {
                let input = event.get("input").cloned().unwrap_or(Value::Null);
                self.tool_use(text("id"), text("name"), &input, &mut change);
            }
            "toolResult" => {
                let is_error = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                self.tool_result(text("id"), is_error, &mut change);
            }
            "result" => {
                let kind = event.get("resultKind").and_then(Value::as_str);
                let is_error = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let result = event.get("result").and_then(Value::as_str);
                if kind == Some("refusal") {
                    self.notice();
                } else if let (true, Some(result)) = (is_error, result.filter(|r| !r.is_empty())) {
                    self.append_text(&result_error_text(result, kind), &mut change);
                }
            }
            "error" => {
                let message = text("message");
                self.append_text(&format!("(error: {message})"), &mut change);
            }
            "evidenceReceipt" => {
                if !self.any_bubble {
                    // A bare receipt row: an empty bubble that keeps any
                    // pending boundary.
                    let boundary = self.boundary;
                    self.new_bubble(false);
                    self.boundary = boundary;
                }
            }
            // These finish the bubble and add a row (the window's
            // finalizePartials, then a steer or notice row).
            "steer" | "accountSwitched" | "remoteRun" => {
                self.finalize(&mut change);
                self.boundary = false;
                self.notice();
            }
            // P1a: a paused turn's notice, like a stop's.
            "budgetWarning" | "loginLow" | "sessionReset" | "paused" => self.notice(),
            "cancelled" if event.get("queued").and_then(Value::as_bool) != Some(true) => {
                self.notice()
            }
            "done" => self.finalize(&mut change),
            _ => {}
        }
        change
    }
}

/// One running turn of a chat.
#[derive(Clone, Debug)]
pub(crate) struct LiveTurn {
    pub key: String,
    pub ord: u32,
    pub gen: u32,
    pub started_at: String,
    pub fold: Fold,
    /// What the turn's own events said about how it ended, if the record
    /// can't be read: `cancelled`, `error`.
    pub ended_as: Option<&'static str>,
}

impl LiveTurn {
    pub(crate) fn wire(&self) -> wire::Live {
        wire::Live {
            key: self.key.clone(),
            ord: self.ord,
            gen: self.gen,
            started_at: self.started_at.clone(),
            text: self.fold.text().to_string(),
            truncated: self.fold.truncated(),
            tools: self.fold.tools().iter().map(ToolRow::wire).collect(),
        }
    }
}

// ── What the hub reads from Hollow ───────────────────────────────────────

/// The newest turn of a chat as the session store records it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TurnRecord {
    pub ord: u32,
    pub started_at: String,
    pub status: Option<String>,
}

/// One list row's comparable parts.
pub(crate) type RowDigest = (String, String);

/// Reads the hub needs from Hollow, behind a trait so tests can stand in.
pub(crate) trait Lookup: Send + Sync + 'static {
    /// The chat's newest turn.
    fn newest_turn(&self, key: &str) -> Option<TurnRecord>;
    /// `(state, needsAnswer)`: `idle`, `queued` or `running`.
    fn state(&self, key: &str) -> (String, bool);
    /// Each newest list row's key and a digest of what the list shows.
    fn digest(&self, limit: usize) -> Vec<RowDigest>;
}

// ── Streams ──────────────────────────────────────────────────────────────

/// One SSE frame.
/// AG8: a stream's first frame. `api` is the version every client checks;
/// `build` (`"<version> <commit>"`) and `events` (the kinds this stream may
/// carry) were added 2026-11-17 and an older client ignores them.
///
/// `resumed` (AG5, an older client ignores it) says whether a replay follows
/// a `Last-Event-ID`; `None` for a stream that sent none.
pub(crate) fn hello_for(resumed: Option<bool>) -> Value {
    let build = crate::app_menu::build_identity();
    let mut hello = serde_json::json!({
        "api": wire::API,
        "build": format!("{} {}", build.version, build.short_commit),
        "events": wire::STREAM_EVENTS,
    });
    if let Some(resumed) = resumed {
        hello["resumed"] = Value::Bool(resumed);
    }
    hello
}

pub(crate) fn frame(event: &str, data: &impl serde::Serialize) -> Bytes {
    let json = serde_json::to_string(data).unwrap_or_else(|_| "null".into());
    Bytes::from(format!("event: {event}\ndata: {json}\n\n"))
}

/// AG5: a frame with its `id:` line.
pub(crate) fn frame_with_id(id: &str, event: &str, data: &impl serde::Serialize) -> Bytes {
    let json = serde_json::to_string(data).unwrap_or_else(|_| "null".into());
    Bytes::from(format!("id: {id}\nevent: {event}\ndata: {json}\n\n"))
}

// ── Ids and the resume ring (AG5) ────────────────────────────────────────

/// What the ring remembers of one logged event.
#[derive(Clone, Debug, PartialEq)]
enum Logged {
    /// Text added to a chat's live view.
    Append {
        key: String,
        ord: u32,
        gen: u32,
        text: String,
    },
    /// A chat's live view changed beyond text (a turn started, its tools
    /// changed, it was truncated): replayed as the current whole view.
    Live { key: String, ord: u32 },
    /// A chat's turn ended.
    Turn {
        key: String,
        ord: u32,
        status: String,
    },
    /// List rows that changed.
    Sessions(Vec<String>),
}

impl Logged {
    fn cost(&self) -> usize {
        RING_ENTRY_COST
            + match self {
                Logged::Append { key, text, .. } => key.len() + text.len(),
                Logged::Live { key, .. } => key.len(),
                Logged::Turn { key, status, .. } => key.len() + status.len(),
                Logged::Sessions(keys) => keys.iter().map(String::len).sum(),
            }
    }

    fn chat(&self) -> Option<&str> {
        match self {
            Logged::Append { key, .. } | Logged::Live { key, .. } | Logged::Turn { key, .. } => {
                Some(key)
            }
            Logged::Sessions(_) => None,
        }
    }
}

/// One ring entry: events `first..=last` (more than one only for merged
/// appends).
struct Entry {
    first: u64,
    last: u64,
    what: Logged,
}

/// The newest events, bounded in count and bytes, oldest dropped first.
#[derive(Default)]
struct Ring {
    entries: VecDeque<Entry>,
    bytes: usize,
    /// The newest event number dropped (or never kept): an id below it can't
    /// be replayed.
    floor: u64,
}

impl Ring {
    fn trim(&mut self) {
        while self.entries.len() > RING_EVENTS || self.bytes > RING_BYTES {
            let Some(old) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= old.what.cost();
            self.floor = self.floor.max(old.last);
        }
    }
}

/// A new epoch: random, never `not`.
fn new_epoch(not: u32) -> u32 {
    loop {
        let epoch: u32 = rand::random();
        if epoch != not {
            return epoch;
        }
    }
}

/// Which chat an id belongs to (a short fixed hash; `0` for the list stream).
fn bind_of(session: Option<&str>) -> u16 {
    use std::hash::{Hash, Hasher};
    session.map_or(0, |key| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() & 0xffff) as u16
    })
}

/// A parsed `Last-Event-ID`.
#[derive(Debug, PartialEq)]
struct Cursor {
    epoch: u32,
    chat: u64,
    list: u64,
    bind: u16,
}

fn parse_id(id: &str) -> Option<Cursor> {
    let mut parts = id.trim().split('-');
    let cursor = Cursor {
        epoch: u32::from_str_radix(parts.next()?, 16).ok()?,
        chat: parts.next()?.parse().ok()?,
        list: parts.next()?.parse().ok()?,
        bind: u16::from_str_radix(parts.next()?, 16).ok()?,
    };
    parts.next().is_none().then_some(cursor)
}

/// What a stamp needs from the hub, taken before the streams are borrowed.
struct At {
    epoch: u32,
    seq: u64,
    stamped: Arc<AtomicU64>,
}

/// What an unsent frame is, as far as coalescing goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `live`: the whole live view, so it replaces every unsent `live` and
    /// `append` before it.
    Live,
    /// `append`: text for the live view the app holds.
    Append,
    Other,
}

impl Kind {
    fn of(event: &str) -> Self {
        match event {
            "live" => Kind::Live,
            "append" => Kind::Append,
            _ => Kind::Other,
        }
    }
}

/// One stream's unsent frames, between the hub (which queues them) and the
/// response body (which sends them). Capped in bytes, not frames: a `live`
/// frame carries the whole answer so far, up to [`TEXT_CAP`].
#[derive(Default)]
struct Queue {
    frames: std::collections::VecDeque<(Kind, Bytes)>,
    /// Bytes in `frames`.
    bytes: usize,
    /// Bytes of the one unsent `live` frame (coalescing leaves at most one).
    live_bytes: usize,
    /// The hub let go (closed, revoked, too slow): the body ends once it has
    /// sent what is queued.
    closed: bool,
    /// The body is gone (its connection closed).
    gone: bool,
    waker: Option<std::task::Waker>,
}

type Outbox = Arc<Mutex<Queue>>;

fn queue(outbox: &Outbox) -> std::sync::MutexGuard<'_, Queue> {
    outbox.lock().unwrap_or_else(|e| e.into_inner())
}

/// The hub's end of a stream. Dropping it closes the stream.
struct FrameTx(Outbox);

impl FrameTx {
    /// Queue a frame: a `live` frame first drops the unsent `live` and
    /// `append` frames it makes stale. `false` when the body is gone, or when
    /// the stream fell more than [`STREAM_BYTES`] behind besides its newest
    /// `live` frame; then it closes, dropping what it held (the app
    /// reconnects and refetches).
    fn send(&self, kind: Kind, frame: Bytes) -> bool {
        let mut q = queue(&self.0);
        if q.gone || q.closed {
            return false;
        }
        if kind == Kind::Live {
            q.frames.retain(|(k, _)| *k == Kind::Other);
            q.bytes = q.frames.iter().map(|(_, f)| f.len()).sum();
            q.live_bytes = frame.len();
        }
        q.bytes += frame.len();
        q.frames.push_back((kind, frame));
        let kept = q.bytes - q.live_bytes <= STREAM_BYTES;
        if !kept {
            q.frames.clear();
            q.bytes = 0;
            q.live_bytes = 0;
            q.closed = true;
        }
        let waker = q.waker.take();
        drop(q);
        if let Some(waker) = waker {
            waker.wake();
        }
        kept
    }

    fn is_closed(&self) -> bool {
        let q = queue(&self.0);
        q.gone || q.closed
    }
}

impl Drop for FrameTx {
    fn drop(&mut self) {
        let mut q = queue(&self.0);
        q.closed = true;
        let waker = q.waker.take();
        drop(q);
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// A stream's frames as its response body reads them. Dropping it (the
/// connection closed) frees what was queued.
pub(crate) struct FrameRx(Outbox);

impl FrameRx {
    /// The next frame; `None` once the hub let go and everything queued was
    /// read.
    pub(crate) fn poll_recv(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Bytes>> {
        let mut q = queue(&self.0);
        if let Some(frame) = Self::pop(&mut q) {
            return std::task::Poll::Ready(Some(frame));
        }
        if q.closed {
            return std::task::Poll::Ready(None);
        }
        q.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }

    fn pop(q: &mut Queue) -> Option<Bytes> {
        let (kind, frame) = q.frames.pop_front()?;
        q.bytes -= frame.len();
        if kind == Kind::Live {
            q.live_bytes = 0;
        }
        Some(frame)
    }

    #[cfg(test)]
    pub(crate) async fn recv(&mut self) -> Option<Bytes> {
        std::future::poll_fn(|cx| self.poll_recv(cx)).await
    }

    /// The next frame queued now, if any (tests).
    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Option<Bytes> {
        Self::pop(&mut queue(&self.0))
    }

    /// The bytes queued and not yet read (tests).
    #[cfg(test)]
    pub(crate) fn unsent(&self) -> usize {
        queue(&self.0).bytes
    }
}

impl Drop for FrameRx {
    fn drop(&mut self) {
        let mut q = queue(&self.0);
        q.gone = true;
        q.frames.clear();
        q.bytes = 0;
        q.live_bytes = 0;
    }
}

fn outbox() -> (FrameTx, FrameRx) {
    let outbox: Outbox = Arc::default();
    (FrameTx(outbox.clone()), FrameRx(outbox))
}

struct Stream {
    device_id: String,
    session: Option<String>,
    tx: FrameTx,
    /// Text not yet sent for `(ord, gen)`.
    append: Option<(u32, u32, String)>,
    /// A `live` is due (it carries the whole text: it replaces `append`).
    live_due: bool,
    last_flush: Instant,
    sessions: BTreeSet<String>,
    last_sessions: Option<Instant>,
    last_ping: Instant,
    last_state: Option<(String, bool)>,
    /// AG5: the first logged event this stream holds unsent, per kind (the
    /// ids hold back to just before them).
    append_from: Option<u64>,
    live_from: Option<u64>,
    sessions_from: Option<u64>,
}

impl Stream {
    /// Queue an `event` frame with its `id:`; `false` when the stream is gone
    /// or too far behind ([`FrameTx::send`]). The id's cursors are what this
    /// stream has been sent in full: everything logged, less what it still
    /// holds unsent.
    fn send(&self, event: &str, data: &impl serde::Serialize, at: &At) -> bool {
        let chat = if self.session.is_some() {
            let chat = [self.append_from, self.live_from]
                .into_iter()
                .flatten()
                .min()
                .map_or(at.seq, |first| first - 1);
            at.stamped.fetch_max(chat, AtomicOrdering::AcqRel);
            chat
        } else {
            at.seq
        };
        let list = self.sessions_from.map_or(at.seq, |first| first - 1);
        let id = format!(
            "{:08x}-{chat}-{list}-{:04x}",
            at.epoch,
            bind_of(self.session.as_deref())
        );
        self.tx
            .send(Kind::of(event), frame_with_id(&id, event, data))
    }

    /// A frame without an id (`hello`, `resync`).
    fn send_plain(&self, event: &str, data: &impl serde::Serialize) -> bool {
        self.tx.send(Kind::of(event), frame(event, data))
    }

    /// The chat's events up to the newest are sent or not held: nothing is
    /// pending for the chat.
    fn clear_chat(&mut self) {
        self.append = None;
        self.live_due = false;
        self.append_from = None;
        self.live_from = None;
    }
}

#[derive(Default)]
struct Hub {
    turns: HashMap<String, LiveTurn>,
    /// Oldest first.
    streams: Vec<Stream>,
    /// Each list row's digest as last compared.
    rows: Option<HashMap<String, String>>,
    last_reconcile: Option<Instant>,
    /// The watching period: a number of its own from the moment the iPhone
    /// app is on with a phone paired until the switch goes off or the last
    /// phone is revoked; 0 otherwise.
    period: u64,
    /// The newest period handed out.
    last_period: u64,
    /// AG5: this hub's epoch, new for every watching period.
    epoch: u32,
    /// AG5: the newest logged event's number.
    seq: u64,
    ring: Ring,
    /// AG5: the highest chat cursor any stream of a chat was stamped with:
    /// appends merge only past it.
    stamped: Arc<AtomicU64>,
}

impl Hub {
    fn at(&self) -> At {
        At {
            epoch: self.epoch,
            seq: self.seq,
            stamped: self.stamped.clone(),
        }
    }

    /// Log one event; returns its number. A run of one chat's appends joins
    /// the newest entry while no stream was stamped at or past its start.
    fn log(&mut self, what: Logged) -> u64 {
        self.seq += 1;
        let seq = self.seq;
        if let (
            Logged::Append {
                key,
                ord,
                gen,
                text,
            },
            Some(Entry {
                first,
                last,
                what:
                    Logged::Append {
                        key: k,
                        ord: o,
                        gen: g,
                        text: t,
                    },
            }),
        ) = (&what, self.ring.entries.back_mut())
        {
            if k == key
                && o == ord
                && g == gen
                && t.len() + text.len() <= RING_MERGE_TEXT
                && self.stamped.load(AtomicOrdering::Acquire) < *first
            {
                t.push_str(text);
                *last = seq;
                self.ring.bytes += text.len();
                self.ring.trim();
                return seq;
            }
        }
        self.ring.bytes += what.cost();
        self.ring.entries.push_back(Entry {
            first: seq,
            last: seq,
            what,
        });
        self.ring.trim();
        seq
    }

    /// List rows changed: every stream hears it (rate limited), and the ring
    /// remembers it.
    fn note_sessions(&mut self, keys: impl IntoIterator<Item = String>) {
        let keys: Vec<String> = keys.into_iter().collect();
        if keys.is_empty() {
            return;
        }
        let seq = self.log(Logged::Sessions(keys.clone()));
        for stream in self.streams.iter_mut() {
            stream.sessions.extend(keys.iter().cloned());
            stream.sessions_from.get_or_insert(seq);
        }
    }

    /// Forget the history and start a new epoch: what was copied has a gap.
    fn new_epoch(&mut self) {
        self.epoch = new_epoch(self.epoch);
        self.ring = Ring {
            floor: self.seq,
            ..Ring::default()
        };
    }

    /// What a stream resuming at `cursor` is owed, or why it can't resume.
    fn replay(
        &self,
        key: Option<&str>,
        cursor: &Cursor,
    ) -> Result<Vec<(&'static str, Value)>, &'static str> {
        if cursor.epoch != self.epoch {
            return Err("epoch");
        }
        if cursor.bind != bind_of(key) {
            return Err("chat");
        }
        if cursor.chat > self.seq || cursor.list > self.seq {
            return Err("ahead");
        }
        // A list stream has no chat cursor to check.
        let oldest = if key.is_some() {
            cursor.chat.min(cursor.list)
        } else {
            cursor.list
        };
        if oldest < self.ring.floor {
            return Err("expired");
        }
        let (chat, list) = (cursor.chat, cursor.list);
        // A cursor inside a merged run can't be split.
        if self
            .ring
            .entries
            .iter()
            .any(|e| e.first <= chat && chat < e.last && key.is_some() && e.what.chat() == key)
        {
            return Err("expired");
        }
        let mut out: Vec<(&'static str, Value)> = Vec::new();
        let mut text: Option<(u32, u32, String)> = None;
        let mut live: Option<u32> = None;
        let mut keys: BTreeSet<String> = BTreeSet::new();
        for entry in self.ring.entries.iter() {
            match &entry.what {
                Logged::Sessions(rows) if entry.first > list => keys.extend(rows.iter().cloned()),
                what if what.chat() == key && key.is_some() && entry.first > chat => match what {
                    Logged::Append {
                        ord,
                        gen,
                        text: more,
                        ..
                    } => match text.as_mut() {
                        Some((o, g, t)) if o == ord && g == gen => t.push_str(more),
                        Some(_) => {
                            self.flush_replay(key, &mut text, &mut live, &mut out);
                            text = Some((*ord, *gen, more.clone()));
                        }
                        None => text = Some((*ord, *gen, more.clone())),
                    },
                    Logged::Live { ord, .. } => {
                        text = None;
                        live = Some(*ord);
                    }
                    Logged::Turn { ord, status, .. } => {
                        self.flush_replay(key, &mut text, &mut live, &mut out);
                        out.push((
                            "turn",
                            serde_json::to_value(TurnEvent {
                                key: key.unwrap_or_default().to_string(),
                                ord: *ord,
                                status: status.clone(),
                            })
                            .unwrap_or(Value::Null),
                        ));
                    }
                    Logged::Sessions(_) => {}
                },
                _ => {}
            }
        }
        self.flush_replay(key, &mut text, &mut live, &mut out);
        if !keys.is_empty() {
            out.push((
                "sessions",
                serde_json::to_value(wire::SessionsEvent {
                    keys: keys.into_iter().collect(),
                })
                .unwrap_or(Value::Null),
            ));
        }
        Ok(out)
    }

    /// Replay: the whole live view when its turn still runs (it already holds
    /// every append after the change, so those are dropped), else the
    /// appends; a turn that ended is told by its own `turn` frame.
    fn flush_replay(
        &self,
        key: Option<&str>,
        text: &mut Option<(u32, u32, String)>,
        live: &mut Option<u32>,
        out: &mut Vec<(&'static str, Value)>,
    ) {
        if let Some(ord) = live.take() {
            let running = key
                .and_then(|k| self.turns.get(k))
                .filter(|turn| turn.ord == ord);
            if let Some(turn) = running {
                *text = None;
                out.push((
                    "live",
                    serde_json::to_value(turn.wire()).unwrap_or(Value::Null),
                ));
            }
        }
        if let Some((ord, gen, text)) = text.take() {
            out.push((
                "append",
                serde_json::to_value(AppendEvent {
                    key: key.unwrap_or_default().to_string(),
                    ord,
                    gen,
                    text,
                })
                .unwrap_or(Value::Null),
            ));
        }
    }

    /// Whether an event tapped in `period` still counts (`None`: folded
    /// directly, as the tests do): one copied before the period last ended
    /// would put a gap in a live view.
    fn current(&self, period: Option<u64>) -> bool {
        period.is_none_or(|p| p != 0 && p == self.period)
    }
}

/// How long an answer turn's row is remembered ([`AnswerTurns`]).
const ANSWER_ROWS_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// v1.1: the answer turns phones queued, by run-queue row. A row taken out
/// of the queue before its turn started (Stop at the Mac) sent nothing, so
/// its card comes back, as the window brings back its own
/// (`returnUnsentAnswers`); a turn that started and failed to spawn is
/// `chat::reopen_unstarted_answers`'s. Rows whose `runStarted` arrives
/// before [`Self::note`] are remembered as started, so the order of the
/// two never matters.
#[derive(Default)]
pub(crate) struct AnswerTurns {
    rows: Mutex<HashMap<i64, (String, Instant)>>,
    started: Mutex<HashMap<i64, Instant>>,
}

impl AnswerTurns {
    /// Queue row `queue_id` carries the answers to card `card`.
    pub(crate) fn note(&self, queue_id: i64, card: &str) {
        let now = Instant::now();
        let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
        started.retain(|_, at| now.duration_since(*at) < ANSWER_ROWS_FOR);
        if started.remove(&queue_id).is_some() {
            return;
        }
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        rows.retain(|_, (_, at)| now.duration_since(*at) < ANSWER_ROWS_FOR);
        rows.insert(queue_id, (card.to_string(), now));
    }

    /// Row `queue_id`'s turn started (`runStarted`).
    pub(crate) fn started(&self, queue_id: i64) {
        let now = Instant::now();
        let noted = self
            .rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&queue_id)
            .is_some();
        if !noted {
            let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
            started.retain(|_, at| now.duration_since(*at) < ANSWER_ROWS_FOR);
            started.insert(queue_id, now);
        }
    }

    /// Row `queue_id` ended (`done`): the card of an answer turn that never
    /// started.
    pub(crate) fn ended(&self, queue_id: i64) -> Option<String> {
        self.started
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&queue_id);
        self.rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&queue_id)
            .map(|(card, _)| card)
    }
}

/// What else sees every folded event (v1.1: push notifications and answer
/// turns, `phone_access::on_chat_event`).
pub(crate) type Observer = Box<dyn Fn(&Value) + Send + Sync>;

/// The live buffer and the open streams, shared by the event worker and the
/// server.
pub(crate) struct Live {
    hub: Mutex<Hub>,
    lookup: Arc<dyn Lookup>,
    /// [`Hub::period`], for [`tap`] to read without the lock.
    watching: AtomicU64,
    /// One [`Live::follow`] at a time, so the last one reads the newest
    /// record.
    following: Mutex<()>,
    /// v1.1: the answer turns phones queued.
    pub(crate) answers: AnswerTurns,
    /// v1.1: called with each event the worker folds, after the fold.
    observer: OnceLock<Observer>,
}

impl Live {
    pub(crate) fn new(lookup: Arc<dyn Lookup>) -> Self {
        let hub = Hub {
            epoch: new_epoch(0),
            ..Hub::default()
        };
        Self {
            hub: Mutex::new(hub),
            lookup,
            watching: AtomicU64::new(0),
            following: Mutex::new(()),
            answers: AnswerTurns::default(),
            observer: OnceLock::new(),
        }
    }

    /// v1.1: what else sees each folded event (set once, at start).
    pub(crate) fn observe(&self, observer: Observer) {
        let _ = self.observer.set(observer);
    }

    /// v1.1: the phones with an event stream open for chat `key`: they are
    /// watching it live, so they get no push for it.
    pub(crate) fn watchers(&self, key: &str) -> BTreeSet<String> {
        self.lock()
            .streams
            .iter()
            .filter(|s| s.session.as_deref() == Some(key) && !s.tx.is_closed())
            .map(|s| s.device_id.clone())
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Hub> {
        self.hub.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The watching period now, 0 while the iPhone app is off or no phone is
    /// paired: [`tap`] copies nothing then.
    pub(crate) fn watching(&self) -> u64 {
        self.watching.load(AtomicOrdering::Acquire)
    }

    /// Start or end the watching period: `on` while the iPhone app is on
    /// and at least one phone is paired. Starting begins a new period;
    /// ending drops every live turn, since nothing is copied for them until
    /// a period starts again and their views would have a gap. Streams
    /// opening and closing never move it.
    pub(crate) fn watch(&self, on: bool) {
        let mut hub = self.lock();
        let period = match (hub.period, on) {
            (0, true) => {
                hub.last_period += 1;
                hub.last_period
            }
            (period, false) if period != 0 => {
                hub.turns.clear();
                0
            }
            _ => return,
        };
        hub.period = period;
        // AG5: events were not copied outside a period, so no id from before
        // this change may resume.
        hub.new_epoch();
        self.watching.store(period, AtomicOrdering::Release);
    }

    /// [`Self::watch`] as the record says now (`in_use` reads it), after a
    /// change to the switch or the paired phones. One at a time: each change
    /// writes the record before it calls this, so the last call reads the
    /// newest record and two changes racing can't leave the period on with
    /// the switch off.
    pub(crate) fn follow(&self, in_use: impl FnOnce() -> bool) {
        let _one = self.following.lock().unwrap_or_else(|e| e.into_inner());
        self.watch(in_use());
    }

    /// The running turn of `key`, as `live` shows it.
    pub(crate) fn snapshot(&self, key: &str) -> Option<wire::Live> {
        self.lock().turns.get(key).map(LiveTurn::wire)
    }

    /// The live text of `key`'s running turn (the list's preview).
    pub(crate) fn text(&self, key: &str) -> Option<(u32, String)> {
        self.lock()
            .turns
            .get(key)
            .map(|t| (t.ord, t.fold.text().to_string()))
    }

    /// Open a stream for `device_id` (following `session`, if any) and send
    /// its first frames. Closes the device's oldest past [`PER_DEVICE`] and
    /// the oldest of all past [`IN_ALL`].
    #[cfg(test)]
    pub(crate) fn open(&self, device_id: &str, session: Option<String>, now: Instant) -> FrameRx {
        self.open_from(device_id, session, now, None)
    }

    /// [`Self::open`] for a client that sent `Last-Event-ID`: it is replayed
    /// what it missed when the ring still holds that, else told to refetch
    /// (`resync`).
    pub(crate) fn open_from(
        &self,
        device_id: &str,
        session: Option<String>,
        now: Instant,
        last_event_id: Option<&str>,
    ) -> FrameRx {
        // Read before the lock: the state needs the run queue.
        let state = session.as_ref().map(|key| self.lookup.state(key));
        let (tx, rx) = outbox();
        let mut hub = self.lock();
        while hub
            .streams
            .iter()
            .filter(|s| s.device_id == device_id)
            .count()
            >= PER_DEVICE
        {
            let oldest = hub
                .streams
                .iter()
                .position(|s| s.device_id == device_id)
                .unwrap_or(0);
            hub.streams.remove(oldest);
        }
        while hub.streams.len() >= IN_ALL {
            hub.streams.remove(0);
        }
        let stream = Stream {
            device_id: device_id.to_string(),
            session: session.clone(),
            tx,
            append: None,
            live_due: false,
            last_flush: now,
            sessions: BTreeSet::new(),
            last_sessions: None,
            last_ping: now,
            last_state: state.clone(),
            append_from: None,
            live_from: None,
            sessions_from: None,
        };
        let at = hub.at();
        let replay = last_event_id.map(|id| match parse_id(id) {
            Some(cursor) => hub.replay(session.as_deref(), &cursor),
            None => Err("malformed"),
        });
        stream.send_plain(
            "hello",
            &hello_for(replay.as_ref().map(|replay| replay.is_ok())),
        );
        let state_event = |key: String, (state, needs_answer): (String, bool)| StateEvent {
            key,
            state,
            needs_answer,
        };
        match replay {
            Some(Ok(frames)) => {
                for (event, data) in frames {
                    stream.send(event, &data, &at);
                }
                if let (Some(key), Some(state)) = (session, state) {
                    stream.send("state", &state_event(key, state), &at);
                }
            }
            other => {
                if let Some(Err(reason)) = other {
                    stream.send_plain("resync", &serde_json::json!({ "reason": reason }));
                }
                if let (Some(key), Some(state)) = (session, state) {
                    let live = hub.turns.get(&key).map(LiveTurn::wire);
                    stream.send("live", &live, &at);
                    stream.send("state", &state_event(key, state), &at);
                }
            }
        }
        hub.streams.push(stream);
        rx
    }

    /// Close every stream of `device_id` (a revoke). The watching period
    /// is [`Self::follow`]'s.
    pub(crate) fn close_device(&self, device_id: &str) {
        self.lock().streams.retain(|s| s.device_id != device_id);
    }

    /// Close every stream (the switch turned off).
    pub(crate) fn close_all(&self) {
        self.lock().streams.clear();
    }

    /// A new epoch, as a restart gives (tests).
    #[cfg(test)]
    pub(crate) fn new_epoch_now(&self) {
        self.lock().new_epoch();
    }

    /// The resume ring's `(events, bytes)` (tests).
    #[cfg(test)]
    pub(crate) fn ring_stats(&self) -> (usize, usize) {
        let hub = self.lock();
        (hub.ring.entries.len(), hub.ring.bytes)
    }

    pub(crate) fn stream_count(&self) -> usize {
        self.lock().streams.len()
    }

    /// Something about `key` changed outside a `chat-event` (a phone send,
    /// a stop): its list row and state are compared again now.
    pub(crate) fn touched(&self, key: &str) {
        let state = self.lookup.state(key);
        let mut hub = self.lock();
        hub.note_sessions([key.to_string()]);
        send_state(&mut hub, key, &state);
    }

    /// Fold one `chat-event` and pass on what it changed, whatever the
    /// watching period (tests; the app folds only what [`tap`] copied).
    #[cfg(test)]
    pub(crate) fn ingest(&self, event: &Value) {
        self.fold_in(None, event);
    }

    /// Fold one `chat-event` [`tap`] copied in watching period `period`; one
    /// from an earlier period is dropped ([`Hub::current`]). The observer
    /// sees it next, whatever the period.
    pub(crate) fn ingest_tapped(&self, period: u64, event: &Value) {
        self.fold_in(Some(period), event);
        if let Some(observer) = self.observer.get() {
            observer(event);
        }
    }

    fn fold_in(&self, period: Option<u64>, event: &Value) {
        let Some(key) = event
            .get("conversationKey")
            .and_then(Value::as_str)
            .filter(|k| crate::agent_sessions::is_safe_session_key(k))
            .map(str::to_string)
        else {
            return;
        };
        if !self.lock().current(period) {
            return;
        }
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "runStarted" => self.started(&key, period),
            "done" => self.ended(&key),
            _ => {}
        }
        if kind == "done" {
            return;
        }
        let mut hub = self.lock();
        if !hub.current(period) {
            return;
        }
        let following = hub
            .streams
            .iter()
            .any(|s| s.session.as_deref() == Some(&key));
        let Some(turn) = hub.turns.get_mut(&key) else {
            drop(hub);
            if following && matches!(kind, "interaction" | "cancelled" | "paused" | "error") {
                self.touched(&key);
            }
            return;
        };
        match kind {
            // P1a: a paused turn's status is `cancelled` too.
            "cancelled" | "paused" => turn.ended_as = Some("cancelled"),
            "error" if turn.ended_as.is_none() => turn.ended_as = Some("error"),
            _ => {}
        }
        let change = turn.fold.apply(event);
        let (ord, gen) = (turn.ord, turn.gen);
        // AG5: logged whether or not a stream follows: one that dropped
        // resumes from here.
        let seq = if change.tools || change.truncated {
            hub.log(Logged::Live {
                key: key.clone(),
                ord,
            })
        } else if !change.appended.is_empty() {
            hub.log(Logged::Append {
                key: key.clone(),
                ord,
                gen,
                text: change.appended.clone(),
            })
        } else {
            0
        };
        for stream in hub.streams.iter_mut() {
            if stream.session.as_deref() != Some(&key) {
                continue;
            }
            if change.tools || change.truncated {
                stream.live_due = true;
                stream.live_from.get_or_insert(seq);
                stream.append = None;
                stream.append_from = None;
            } else if !change.appended.is_empty() && !stream.live_due {
                stream.append_from.get_or_insert(seq);
                match stream.append.as_mut() {
                    Some((o, g, text)) if *o == ord && *g == gen => text.push_str(&change.appended),
                    _ => stream.append = Some((ord, gen, change.appended.clone())),
                }
            }
        }
        drop(hub);
        if kind == "interaction" && following {
            self.touched(&key);
        }
    }

    /// `runStarted`: a fresh live turn (its ord and start from the record),
    /// unless the watching period it was tapped in has ended.
    fn started(&self, key: &str, period: Option<u64>) {
        let record = self.lookup.newest_turn(key);
        let state = self.lookup.state(key);
        let mut hub = self.lock();
        if !hub.current(period) {
            return;
        }
        let (ord, started_at) = match record {
            Some(r) => (r.ord, r.started_at),
            None => (
                hub.turns.get(key).map(|t| t.ord + 1).unwrap_or(1),
                chrono::Utc::now().to_rfc3339(),
            ),
        };
        // The same turn starting again (a retry) keeps what the window
        // still shows; a new ord is a new turn.
        let fresh = hub.turns.get(key).is_none_or(|t| t.ord != ord);
        if fresh {
            hub.turns.insert(
                key.to_string(),
                LiveTurn {
                    key: key.to_string(),
                    ord,
                    gen: 1,
                    started_at,
                    fold: Fold::default(),
                    ended_as: None,
                },
            );
        }
        let seq = hub.log(Logged::Live {
            key: key.to_string(),
            ord,
        });
        for stream in hub.streams.iter_mut() {
            if stream.session.as_deref() == Some(key) {
                stream.live_due = true;
                stream.live_from.get_or_insert(seq);
                stream.append = None;
                stream.append_from = None;
            }
        }
        hub.note_sessions([key.to_string()]);
        send_state(&mut hub, key, &state);
    }

    /// `done`: the turn ended. Followers get what was left of it, `turn`
    /// and the new state; every stream hears the list row changed.
    fn ended(&self, key: &str) {
        let record = self.lookup.newest_turn(key);
        let state = self.lookup.state(key);
        let mut hub = self.lock();
        let ended = hub.turns.remove(key);
        if let Some(mut turn) = ended {
            let mut change = Change::default();
            turn.fold.finalize(&mut change);
            let status = record
                .filter(|r| r.ord == turn.ord)
                .and_then(|r| r.status)
                .filter(|s| s != "running")
                .unwrap_or_else(|| turn.ended_as.unwrap_or("success").to_string());
            let live = turn.wire();
            if change.tools {
                hub.log(Logged::Live {
                    key: key.to_string(),
                    ord: turn.ord,
                });
            }
            hub.log(Logged::Turn {
                key: key.to_string(),
                ord: turn.ord,
                status: status.clone(),
            });
            let at = hub.at();
            for stream in hub.streams.iter_mut() {
                if stream.session.as_deref() != Some(key) {
                    continue;
                }
                // What was still waiting goes first, so the app has it all
                // before it refetches.
                let pending_live = stream.live_due || change.tools;
                let pending_append = stream.append.take();
                stream.clear_chat();
                if pending_live {
                    stream.send("live", &live, &at);
                } else if let Some((ord, gen, text)) = pending_append {
                    stream.send(
                        "append",
                        &AppendEvent {
                            key: key.to_string(),
                            ord,
                            gen,
                            text,
                        },
                        &at,
                    );
                }
                stream.send(
                    "turn",
                    &TurnEvent {
                        key: key.to_string(),
                        ord: turn.ord,
                        status: status.clone(),
                    },
                    &at,
                );
            }
        }
        hub.note_sessions([key.to_string()]);
        send_state(&mut hub, key, &state);
    }

    /// Send what is due: coalesced `append`/`live`, `sessions`, `ping`, and
    /// the periodic reconcile. Streams that are gone or too slow close.
    pub(crate) fn tick(&self, now: Instant) {
        let reconcile = {
            let hub = self.lock();
            !hub.streams.is_empty()
                && hub
                    .last_reconcile
                    .is_none_or(|at| now.duration_since(at) >= RECONCILE_EVERY)
        };
        if reconcile {
            self.reconcile(now);
        }
        let mut hub = self.lock();
        let at = hub.at();
        let Hub { turns, streams, .. } = &mut *hub;
        streams.retain_mut(|stream| {
            let mut alive = !stream.tx.is_closed();
            if alive && now.duration_since(stream.last_flush) >= APPEND_EVERY {
                if stream.live_due {
                    stream.clear_chat();
                    stream.last_flush = now;
                    let live = stream
                        .session
                        .as_ref()
                        .and_then(|key| turns.get(key))
                        .map(LiveTurn::wire);
                    if let Some(live) = live {
                        alive = stream.send("live", &live, &at);
                    }
                } else if let Some((ord, gen, text)) = stream.append.take() {
                    stream.append_from = None;
                    stream.last_flush = now;
                    let key = stream.session.clone().unwrap_or_default();
                    alive = stream.send(
                        "append",
                        &AppendEvent {
                            key,
                            ord,
                            gen,
                            text,
                        },
                        &at,
                    );
                }
            }
            let sessions_due = stream
                .last_sessions
                .is_none_or(|at| now.duration_since(at) >= SESSIONS_EVERY);
            if alive && !stream.sessions.is_empty() && sessions_due {
                let keys: Vec<String> = std::mem::take(&mut stream.sessions).into_iter().collect();
                stream.last_sessions = Some(now);
                stream.sessions_from = None;
                alive = stream.send("sessions", &wire::SessionsEvent { keys }, &at);
            }
            if alive && now.duration_since(stream.last_ping) >= PING_EVERY {
                stream.last_ping = now;
                alive = stream.send("ping", &serde_json::json!({}), &at);
            }
            alive
        });
    }

    /// B4: an approval card appeared or resolved: tell every stream, which
    /// reads `GET /v1/approvals` again. Not logged or resumable: a phone that
    /// reconnects reads the list anyway.
    pub(crate) fn approvals_changed(&self) {
        let mut hub = self.lock();
        let at = hub.at();
        hub.streams.retain_mut(|stream| {
            // Phones only: another Mac's stream (`mc-`) has no approvals to read.
            !stream.device_id.starts_with("ph-")
                || stream.send("approvals", &serde_json::json!({}), &at)
        });
    }

    /// Compare the newest list rows and the followed chats' states with
    /// what was last seen.
    fn reconcile(&self, now: Instant) {
        let followed: BTreeSet<String> = {
            let mut hub = self.lock();
            hub.last_reconcile = Some(now);
            hub.streams
                .iter()
                .filter_map(|s| s.session.clone())
                .collect()
        };
        let rows: HashMap<String, String> =
            self.lookup.digest(RECONCILE_ROWS).into_iter().collect();
        let states: Vec<(String, (String, bool))> = followed
            .into_iter()
            .map(|key| {
                let state = self.lookup.state(&key);
                (key, state)
            })
            .collect();
        let mut hub = self.lock();
        if let Some(before) = hub.rows.as_ref() {
            let mut changed: BTreeSet<String> = rows
                .iter()
                .filter(|(key, digest)| before.get(*key) != Some(*digest))
                .map(|(key, _)| key.clone())
                .collect();
            changed.extend(before.keys().filter(|k| !rows.contains_key(*k)).cloned());
            hub.note_sessions(changed);
        }
        hub.rows = Some(rows);
        for (key, state) in states {
            send_state(&mut hub, &key, &state);
        }
    }
}

/// Send `state` to the streams following `key` that last heard another.
fn send_state(hub: &mut Hub, key: &str, state: &(String, bool)) {
    let at = hub.at();
    for stream in hub.streams.iter_mut() {
        if stream.session.as_deref() != Some(key) || stream.last_state.as_ref() == Some(state) {
            continue;
        }
        stream.last_state = Some(state.clone());
        stream.send(
            "state",
            &StateEvent {
                key: key.to_string(),
                state: state.0.clone(),
                needs_answer: state.1,
            },
            &at,
        );
    }
}

// ── The app's worker ─────────────────────────────────────────────────────

/// Where [`tap`] sends events, once [`start`] has run.
static TAP: OnceLock<Mutex<mpsc::Sender<(u64, Value)>>> = OnceLock::new();
static APP_LIVE: OnceLock<Arc<Live>> = OnceLock::new();

/// Every `chat-event`, from `chat::emit_event`, before the window gets it.
/// Unless the iPhone app is on with a phone paired it copies nothing: one
/// atomic read ([`tapped`]). Otherwise a copy into a channel, which the
/// worker thread folds.
pub(crate) fn tap(payload: &Value) {
    let Some(copy) = APP_LIVE.get().and_then(|live| tapped(live, payload)) else {
        return;
    };
    if let Some(tx) = TAP.get() {
        let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(copy);
    }
}

/// What [`tap`] hands the worker for `payload`: nothing outside a watching
/// period, else a copy with the period it was made in (review finding 8,
/// 2026-11-04: it copied every event, tool results included, with the
/// switch off and nothing paired).
pub(crate) fn tapped(live: &Live, payload: &Value) -> Option<(u64, Value)> {
    let period = live.watching();
    (period != 0).then(|| (period, payload.clone()))
}

/// Start the app's live buffer: a worker thread folds tapped events and
/// keeps the streams' timing. Only events tapped in a watching period reach
/// it, so a turn that was already running when the period started has no
/// live view until it ends.
pub(crate) fn start(vault: PathBuf) -> Arc<Live> {
    if let Some(live) = APP_LIVE.get() {
        return live.clone();
    }
    let live = Arc::new(Live::new(Arc::new(super::routes::VaultLookup::new(vault))));
    let (tx, rx) = mpsc::channel::<(u64, Value)>();
    if TAP.set(Mutex::new(tx)).is_err() || APP_LIVE.set(live.clone()).is_err() {
        return APP_LIVE.get().cloned().unwrap_or(live);
    }
    let worker = live.clone();
    let spawned = std::thread::Builder::new()
        .name("phone-live".into())
        .spawn(move || run_worker(&worker, &rx));
    if let Err(error) = spawned {
        eprintln!("[phone] could not start the live buffer: {error}");
    }
    live
}

fn run_worker(live: &Live, rx: &mpsc::Receiver<(u64, Value)>) {
    loop {
        let wait = if live.stream_count() > 0 {
            APPEND_EVERY
        } else {
            Duration::from_secs(5)
        };
        match rx.recv_timeout(wait) {
            Ok((period, event)) => {
                live.ingest_tapped(period, &event);
                // Take what else arrived before sending anything.
                while let Ok((period, event)) = rx.try_recv() {
                    live.ingest_tapped(period, &event);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        live.tick(Instant::now());
    }
}
