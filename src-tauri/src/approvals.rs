//! Inline chat approvals — the bridge that lets the MCP subprocesses
//! (`hollow m365-mcp`, `hollow computer-mcp`) ask the user for permission
//! **inside the chat transcript** instead of popping a native osascript
//! dialog (Claude-desktop style).
//!
//! Two halves, one module:
//!
//! - **Broker (GUI process).** [`start`] binds a per-process Unix socket and
//!   accepts one-shot approval requests from the MCP subprocesses. Each
//!   request is surfaced to the WebView as an `approval-request` Tauri event
//!   (the chat renders an Allow/Deny card); the user's click comes back via
//!   the `approval_respond` command → [`respond`] → the waiting socket
//!   connection. An `approval-resolved` event clears the card on every
//!   outcome — answered, timed out, or the requesting process dying.
//!
//! - **Client (MCP subprocess).** [`confirm_tool`] is the gate the MCP tools
//!   call before an irreversible action. It connects to the broker socket
//!   named by the `HOLLOW_APPROVAL_SOCK` env (which the `server_entry`
//!   builders inject into the spawn), sends the request, and blocks until the
//!   user answers or the timeout lapses. When the app channel is unavailable —
//!   env unset, socket gone, non-unix — it falls back to the old native
//!   Allow/Deny dialog, so an approval gate never silently disappears.
//!
//! Deny / timeout / cancelled-turn all count as **not approved** — identical
//! semantics to the native dialog this replaces.
//!
//! **Binding a request to its tool call (Paperclip PR 21).** The MCP
//! subprocess never sees the agent's `tool_use` id, so the broker binds each
//! request to the in-flight `mcp__hollow_computer__<tool>` call chat.rs saw
//! start ([`note_tool_use`], after it emitted that `toolUse` event). The
//! client sends the tool name and its `tools/call` arguments; a request binds
//! to the OLDEST unbound noted call with the same tool and the same arguments
//! as a JSON value ([`ApprovalRegistry::take_binding`]). The broker waits up
//! to [`BIND_WAIT`] for the call to be noted (the CLI can invoke the server
//! before chat.rs reads the event line), then emits the request unbound, with
//! null ids: never a guess. The request and resolved events carry
//! `toolUseId`, `sessionKey` and `tool`, so the webview badges exactly the row
//! the request belongs to. Resolved events add `reason`:
//! `allowed | denied | timed_out | cancelled`. A timeout is reported by the
//! client itself: before dropping the connection it writes one
//! `{"cancel":"timed_out"}` line. Anything else that ends the connection
//! without a verdict (the turn stopped, the server died) is `cancelled`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::core_host::Core;

/// Env var carrying the broker socket path into the MCP subprocesses.
pub const ENV_SOCK: &str = "HOLLOW_APPROVAL_SOCK";

/// How long the client waits for the user's answer before giving up —
/// matches the old native dialog's `giving up after 120`.
const CLIENT_TIMEOUT_SECS: u64 = 120;

/// How long the broker waits for chat.rs to note the tool call a request
/// belongs to before emitting it unbound (Paperclip PR 21).
const BIND_WAIT: Duration = Duration::from_millis(750);
/// Poll interval while waiting for that binding.
const BIND_POLL: Duration = Duration::from_millis(50);

/// The line a client writes before dropping a connection it gave up on.
const TIMED_OUT_LINE: &str = "timed_out";

/// Tool-name prefix of the confirm-gated computer server.
const GATED_PREFIX: &str = "mcp__hollow_computer__";
/// Tool-name prefix of the Hollow app server (PR A3): any of its tools can
/// carry a confirm action (`run_action` included), so every call is noted,
/// tagged so its names never collide with the computer tools'.
const APP_PREFIX: &str = "mcp__hollow_app__";
/// The tag an app tool's name is noted under (`hollow_app:<tool>`).
pub(crate) const APP_TOOL_TAG: &str = "hollow_app:";
/// At most this many noted calls are kept; the oldest is dropped first.
const REGISTRY_CAP: usize = 64;

/// Tauri event names (frontend listeners live in `store/runtime.ts`).
#[cfg(unix)]
const EVENT_REQUEST: &str = "approval-request";
#[cfg(unix)]
const EVENT_RESOLVED: &str = "approval-resolved";

/// The broker's socket path, set once by [`start`] in the GUI process.
/// `None` in MCP subprocesses (they read the env var instead) and when the
/// broker failed to bind.
static SOCKET_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Pending approvals awaiting a user answer: id → the oneshot that unblocks
/// the requesting connection.
static PENDING: OnceLock<Mutex<HashMap<String, tokio::sync::oneshot::Sender<Answer>>>> =
    OnceLock::new();

static SEQ: AtomicU64 = AtomicU64::new(1);

/// The user's answer to a card: the verdict, and for a Deny the optional
/// words they typed (SA1), which go back to the agent with the refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Answer {
    pub approved: bool,
    pub reason: Option<String>,
}

/// Longest typed Decline reason kept.
const REASON_MAX: usize = 500;

/// A typed reason as it may travel: control characters become spaces, the
/// ends are trimmed, it is cut at [`REASON_MAX`] characters, and an empty
/// one is no reason.
pub(crate) fn clean_reason(raw: &str) -> Option<String> {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cut: String = flat.trim().chars().take(REASON_MAX).collect();
    let cut = cut.trim().to_string();
    (!cut.is_empty()).then_some(cut)
}

/// A card id that no launch reuses: `apr-<launch>-<n>`, where the launch
/// part is this process's start time (milliseconds) and pid in hex. The
/// counter alone restarted at 1 each launch (AG3), so an id kept in a
/// record, a phone answer or a log could name a different card after a
/// restart.
fn next_card_id() -> String {
    static LAUNCH: OnceLock<String> = OnceLock::new();
    let launch = LAUNCH.get_or_init(|| {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("{ms:x}{:x}", std::process::id())
    });
    format!("apr-{launch}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

thread_local! {
    /// The reason typed on the last in-process Decline this thread waited
    /// on, taken by [`take_decline_reason`] right after the ask returns.
    static LAST_DECLINE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

thread_local! {
    /// ND6: the fingerprint the broker is about to run, set by
    /// [`expect_call`] before it asks; the in-process ask binds its card and
    /// grant to it (else to the card's own tool and arguments).
    static EXPECTED_FP: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// ND6: what the last Allow on this thread was given for, taken once by
    /// [`take_grant`].
    static LAST_GRANT: std::cell::RefCell<Option<crate::approval_terms::Grant>> = const { std::cell::RefCell::new(None) };
}

/// ND6: tell the next in-process ask on this thread which call it approves
/// (the executor's own tool or action id and the arguments it will run).
pub(crate) fn expect_call(tool: &str, args: &Value) {
    let fp = crate::approval_terms::fingerprint(tool, args);
    EXPECTED_FP.with(|slot| *slot.borrow_mut() = Some(fp));
}

/// ND6: drop an expectation no ask used.
pub(crate) fn clear_expected_call() {
    EXPECTED_FP.with(|slot| slot.borrow_mut().take());
}

/// ND6: the approval the last in-process ask on this thread gave, spent by
/// taking it (a second call returns `None`). `None` after an answered Allow
/// never happens for a card; it means no card was asked.
pub(crate) fn take_grant() -> Option<crate::approval_terms::Grant> {
    LAST_GRANT.with(|slot| slot.borrow_mut().take())
}

/// Test askers: a grant as the in-process ask would leave it, from the
/// fingerprint `execute` expected (`wrong` flips it).
#[cfg(test)]
pub(crate) fn test_grant_from_expected(wrong: bool) {
    let fingerprint = EXPECTED_FP
        .with(|slot| slot.borrow_mut().take())
        .map(|fp| if wrong { format!("{fp}x") } else { fp })
        .unwrap_or_default();
    LAST_GRANT.with(|slot| {
        *slot.borrow_mut() = Some(crate::approval_terms::Grant {
            card_id: "apr-test".into(),
            fingerprint,
        })
    });
}

/// The typed reason of the Deny the last in-process ask on this thread got
/// (taken: a second call returns `None`).
pub(crate) fn take_decline_reason() -> Option<String> {
    LAST_DECLINE.with(|slot| slot.borrow_mut().take())
}

fn pending() -> &'static Mutex<HashMap<String, tokio::sync::oneshot::Sender<Answer>>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// An approval card on screen now, as the "needs you" query lists it
/// (merged plan slice 4, B·F15).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PendingCard {
    pub id: String,
    pub title: String,
    pub action: String,
    pub kind: String,
    pub session_key: Option<String>,
    /// The agent tool call it was bound to, when it was (AG3).
    pub tool_use_id: Option<String>,
    /// When the card was shown (RFC 3339).
    pub at: String,
    /// When the wait ends unanswered (RFC 3339): the client's own timeout.
    pub expires_at: String,
    /// Raised to the COO chat ([`CooRoute`]): the one place it is answered.
    pub routed_to_coo: bool,
}

/// The cards shown and not yet resolved, by id. A card joins when its
/// request event goes out and leaves when its resolved event does, on every
/// outcome, so the list never holds an answered or timed-out card.
static CARDS: OnceLock<Mutex<HashMap<String, PendingCard>>> = OnceLock::new();

fn cards() -> std::sync::MutexGuard<'static, HashMap<String, PendingCard>> {
    CARDS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Remember a card from the request payload the UI is about to get.
fn show_card(payload: &Value) {
    let text = |key: &str| payload[key].as_str().unwrap_or_default().to_string();
    let id = text("id");
    if id.is_empty() {
        return;
    }
    let now = chrono::Utc::now();
    let card = PendingCard {
        id: id.clone(),
        title: text("title"),
        action: text("action"),
        kind: text("kind"),
        session_key: payload["sessionKey"].as_str().map(str::to_string),
        tool_use_id: payload["toolUseId"].as_str().map(str::to_string),
        routed_to_coo: payload.get("routedTo").is_some_and(|v| !v.is_null()),
        at: now.to_rfc3339(),
        expires_at: (now + chrono::Duration::seconds(card_wait().as_secs() as i64)).to_rfc3339(),
    };
    cards().insert(id.clone(), card);
    notify_card(&id, true);
}

/// Forget a card whose resolved event is going out.
fn close_card(id: &str) {
    let was_open = cards().remove(id).is_some();
    if was_open {
        notify_card(id, false);
    }
}

/// B4: who hears of a card appearing (`true`) or leaving (`false`): the
/// phone API, which pushes it and tells its event streams. Called with no
/// lock held, so it may read [`pending_cards`].
type CardObserver = Box<dyn Fn(&str, bool) + Send + Sync>;
static CARD_OBSERVER: OnceLock<CardObserver> = OnceLock::new();

/// Register the one card observer (a second call is ignored).
pub(crate) fn observe_cards(observer: impl Fn(&str, bool) + Send + Sync + 'static) {
    let _ = CARD_OBSERVER.set(Box::new(observer));
}

fn notify_card(id: &str, shown: bool) {
    if let Some(observer) = CARD_OBSERVER.get() {
        observer(id, shown);
    }
}

/// Takes a shown card off the list, and drops its pending answer channel,
/// when dropped: so both go on every way out of the wait, including the ones
/// that skip the explicit [`close_card`] (the serving future dropped because
/// its task was aborted, or a panic unwinding). Removing an id twice is
/// harmless.
struct ShownCard(String);

impl Drop for ShownCard {
    fn drop(&mut self) {
        close_card(&self.0);
        if let Ok(mut map) = pending().lock() {
            map.remove(&self.0);
        }
    }
}

/// Every approval card waiting for the user's answer now, oldest first.
pub(crate) fn pending_cards() -> Vec<PendingCard> {
    let mut out: Vec<PendingCard> = cards().values().cloned().collect();
    out.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    out
}

/// The bare tool of a call to the approved mail connector that asks first
/// (`mcp__<id>__<tool>` where the connector id names Superhuman), else None.
fn mail_tool(name: &str) -> Option<&str> {
    let (server, tool) = name.strip_prefix("mcp__")?.split_once("__")?;
    (server.contains("superhuman") && !crate::remote_mcp::is_read_tool(tool)).then_some(tool)
}

/// Whether the process answering on `sock` is an ancestor of this one: the
/// Hollow app that started the run this MCP subprocess belongs to (10-11 B4).
/// A listener an agent started itself can't be. The mail bridge trusts the
/// app channel only then, and otherwise asks on the native dialog.
#[cfg(unix)]
pub fn broker_is_ancestor(sock: &str) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(stream) = std::os::unix::net::UnixStream::connect(sock) else {
        return false;
    };
    let Ok(broker) = crate::run_auth::peer_pid(stream.as_raw_fd()) else {
        return false;
    };
    let mut pid = std::process::id() as i32;
    for _ in 0..64 {
        match crate::proc_info::parent_pid(pid) {
            Some(parent) if parent == broker => return true,
            Some(parent) => pid = parent,
            None => return false,
        }
    }
    false
}

#[cfg(not(unix))]
pub fn broker_is_ancestor(_sock: &str) -> bool {
    false
}

/// The socket path MCP spawns should be pointed at, when the broker is up.
pub fn socket_path() -> Option<PathBuf> {
    SOCKET_PATH.get().cloned()
}

/// The `env` map for a confirm-gated MCP `server_entry`: `HOLLOW_VAULT` (the
/// audit-log root, per the computer_mcp convention) plus the approval socket
/// when the broker is up — so the subprocess can route its Allow/Deny
/// requests into the chat instead of a native dialog.
pub fn mcp_env(vault: &std::path::Path) -> Value {
    let mut env = serde_json::Map::new();
    env.insert("HOLLOW_VAULT".into(), json!(vault.to_string_lossy()));
    if let Some(sock) = socket_path() {
        env.insert(ENV_SOCK.into(), json!(sock.to_string_lossy()));
    }
    Value::Object(env)
}

/// Resolve a pending approval without a reason (tests; the `approval_respond`
/// command goes through [`respond_with`]). Errs when the id is unknown:
/// already answered, timed out, or the requesting process died.
#[cfg(test)]
pub fn respond(id: &str, approved: bool) -> Result<(), String> {
    respond_with(id, approved, None)
}

/// [`respond`] with the words typed on a Decline (SA1). A reason on an
/// Allow is dropped: only a refusal carries one.
pub fn respond_with(id: &str, approved: bool, reason: Option<&str>) -> Result<(), String> {
    let answer = Answer {
        approved,
        reason: if approved {
            None
        } else {
            reason.and_then(clean_reason)
        },
    };
    let tx = pending()
        .lock()
        .map_err(|_| "approval registry poisoned".to_string())?
        .remove(id)
        .ok_or_else(|| format!("no pending approval `{id}` (already resolved?)"))?;
    tx.send(answer)
        .map_err(|_| "the approval request was already closed".to_string())
}

// ── B4: answering from a phone, and waiting for one ──────────────────────────

/// Where a card was answered, when it wasn't in the Mac's window: a phone,
/// with what the card's receipt says ("Allowed on Alex's iPhone with Face
/// ID at 7:42 AM"). The window's answers carry none.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnsweredBy {
    /// `device`: an enrolled iPhone.
    pub kind: &'static str,
    pub device_id: String,
    pub device_name: String,
    /// The answer was an Allow signed behind Face ID (a Deny is not).
    pub face_id: bool,
    /// RFC 3339.
    pub at: String,
    pub receipt: String,
}

impl AnsweredBy {
    /// An enrolled phone's answer.
    pub(crate) fn device(
        device_id: &str,
        device_name: &str,
        approved: bool,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        let local = at.with_timezone(&chrono::Local).format("%-I:%M %p");
        let receipt = if approved {
            format!("Allowed on {device_name} with Face ID at {local}")
        } else {
            format!("Denied on {device_name} at {local}")
        };
        Self {
            kind: "device",
            device_id: device_id.to_string(),
            device_name: device_name.to_string(),
            face_id: approved,
            at: at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            receipt,
        }
    }
}

/// Who answered each card a phone just answered, until its waiting side takes
/// the entry to put in the resolved event.
static ANSWERED_BY: OnceLock<Mutex<HashMap<String, AnsweredBy>>> = OnceLock::new();

fn answered_by() -> std::sync::MutexGuard<'static, HashMap<String, AnsweredBy>> {
    ANSWERED_BY
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn take_answered_by(id: &str) -> Option<AnsweredBy> {
    answered_by().remove(id)
}

thread_local! {
    /// The phone that answered the card this thread last asked, for the
    /// audit line ([`take_last_answered_by`]).
    static LAST_ANSWERED_BY: std::cell::RefCell<Option<AnsweredBy>> = const { std::cell::RefCell::new(None) };
}

/// The phone that answered the card this thread last asked (and clear it).
pub(crate) fn take_last_answered_by() -> Option<AnsweredBy> {
    LAST_ANSWERED_BY.with(|slot| slot.borrow_mut().take())
}

/// Answer a card from a phone (the phone API's decision route, after it
/// checked the signature). The first answer wins: the one-shot sender is taken
/// under the registry lock, as [`respond_with`] takes it, so a second answer
/// from the window or another phone finds nothing and errs.
pub(crate) fn respond_from(id: &str, approved: bool, by: AnsweredBy) -> Result<(), String> {
    let tx = pending()
        .lock()
        .map_err(|_| "approval registry poisoned".to_string())?
        .remove(id)
        .ok_or_else(|| format!("no pending approval `{id}` (already resolved?)"))?;
    answered_by().insert(id.to_string(), by);
    let sent = tx.send(Answer {
        approved,
        reason: None,
    });
    if sent.is_err() {
        take_answered_by(id);
        return Err("the approval request was already closed".to_string());
    }
    Ok(())
}

/// Tests (the phone API's): register a pending card `id` and get the
/// receiver its answer arrives on.
#[cfg(test)]
pub(crate) fn test_register_pending(id: &str) -> tokio::sync::oneshot::Receiver<Answer> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    pending().lock().unwrap().insert(id.to_string(), tx);
    rx
}

/// Tests: who answered `id`, taken.
#[cfg(test)]
pub(crate) fn test_take_answered_by(id: &str) -> Option<AnsweredBy> {
    take_answered_by(id)
}

/// How long a card waits with no phone able to answer: the clients' own clock.
pub(crate) const DEFAULT_WAIT: Duration = Duration::from_secs(CLIENT_TIMEOUT_SECS);
/// A phone with approvals on: how long a card waits by default (a setting
/// changes it).
pub(crate) const PHONE_WAIT_DEFAULT_SECS: u64 = 300;
/// The settings' range for that wait.
pub(crate) const PHONE_WAIT_RANGE: std::ops::RangeInclusive<u64> = 120..=900;

/// How long cards wait while a phone has approvals on; 0 = no phone does.
static PHONE_WAIT_SECS: AtomicU64 = AtomicU64::new(0);

/// Set by the phone API when the set of phones with approval keys, or the
/// wait setting, changes: `Some(seconds)` while any phone has approvals on.
pub(crate) fn set_phone_wait(secs: Option<u64>) {
    let secs = secs.map_or(0, |s| {
        s.clamp(*PHONE_WAIT_RANGE.start(), *PHONE_WAIT_RANGE.end())
    });
    PHONE_WAIT_SECS.store(secs, Ordering::SeqCst);
}

/// The wait while a phone has approvals on.
pub(crate) fn phone_wait() -> Option<Duration> {
    match PHONE_WAIT_SECS.load(Ordering::SeqCst) {
        0 => None,
        secs => Some(Duration::from_secs(secs)),
    }
}

/// How long a card shown now waits: the phone wait, else the clients' 120
/// seconds. Without an approval phone nothing changes.
pub(crate) fn card_wait() -> Duration {
    phone_wait().unwrap_or(DEFAULT_WAIT)
}

/// A wait in words, for the refusal an agent reads ("two minutes", "5 minutes").
pub(crate) fn wait_words(wait: Duration) -> String {
    let secs = wait.as_secs();
    if !secs.is_multiple_of(60) {
        return format!("{secs} seconds");
    }
    let minutes = secs / 60;
    match minutes {
        1 => "a minute".to_string(),
        2 => "two minutes".to_string(),
        n => format!("{n} minutes"),
    }
}

/// Answer a card with an interrupt answer (AG3). Deny by default: only a
/// `resolved` answer whose `payload.approved` is `true` allows; a cancelled
/// or malformed one refuses. Not wired to any route yet (the phone approvals
/// row does that); the frontend still answers through `approval_respond`.
#[allow(dead_code)]
pub(crate) fn respond_interrupt(answer: &crate::interrupt::InterruptAnswer) -> Result<(), String> {
    respond_with(
        &answer.interrupt_id,
        answer.approved(),
        answer.decline_reason().as_deref(),
    )
}

// ── Binding registry (GUI process) ──────────────────────────────────────────

/// One confirm-gated tool call chat.rs saw start and has not seen finish.
#[derive(Debug, Clone)]
struct GatedCall {
    seq: u64,
    session_key: String,
    run_id: Option<String>,
    tool_use_id: String,
    /// The bare MCP tool name (`quit_app`), without the server prefix.
    tool: String,
    /// The call's arguments exactly as the agent emitted them.
    input: Value,
    /// Set once an approval request took this call.
    bound: bool,
}

/// The call an approval request was bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub tool_use_id: String,
    pub session_key: String,
}

/// In-flight confirm-gated tool calls, fed by chat.rs and read by the
/// broker. One global instance ([`REGISTRY`]); tests build their own so
/// parallel tests never see each other's calls.
pub(crate) struct ApprovalRegistry {
    calls: Mutex<Vec<GatedCall>>,
    seq: AtomicU64,
}

/// A missing `arguments` and `{}` are the same call: Claude emits `{}` for a
/// tool with no arguments, and an MCP client may omit the field.
fn normalized_args(v: &Value) -> Value {
    if v.is_null() {
        json!({})
    } else {
        v.clone()
    }
}

impl ApprovalRegistry {
    pub(crate) const fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            seq: AtomicU64::new(1),
        }
    }

    fn calls(&self) -> std::sync::MutexGuard<'_, Vec<GatedCall>> {
        self.calls.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Record a started tool call. Only the confirm-gated computer tools
    /// (`mcp__hollow_computer__` + a [`crate::computer_mcp::CONFIRM_TOOLS`]
    /// name) with a non-empty id are kept.
    #[cfg(test)]
    pub(crate) fn note_tool_use(&self, session_key: &str, id: &str, name: &str, input: &Value) {
        self.note_tool_use_at(session_key, None, id, name, input)
    }

    fn note_tool_use_at(
        &self,
        session_key: &str,
        run_id: Option<&str>,
        id: &str,
        name: &str,
        input: &Value,
    ) {
        let tool = if let Some(tool) = name.strip_prefix(GATED_PREFIX) {
            if !crate::computer_mcp::CONFIRM_TOOLS.contains(&tool) {
                return;
            }
            tool.to_string()
        } else if let Some(tool) = name.strip_prefix(APP_PREFIX) {
            format!("{APP_TOOL_TAG}{tool}")
        } else if let Some(tool) = mail_tool(name) {
            // An approved mail connector's write (10-11 B4): its card binds to
            // this call, so it shows in the chat that made it.
            tool.to_string()
        } else {
            return;
        };
        if id.is_empty() {
            return;
        }
        let mut calls = self.calls();
        if calls.iter().any(|c| c.tool_use_id == id) {
            return;
        }
        while calls.len() >= REGISTRY_CAP {
            calls.remove(0);
        }
        calls.push(GatedCall {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            session_key: session_key.to_string(),
            run_id: run_id.map(str::to_string),
            tool_use_id: id.to_string(),
            tool,
            input: normalized_args(input),
            bound: false,
        });
    }

    /// Forget a call whose result arrived.
    pub(crate) fn note_tool_result(&self, id: &str) {
        if id.is_empty() {
            return;
        }
        self.calls().retain(|c| c.tool_use_id != id);
    }

    /// Forget every call of a finished turn's conversation.
    pub(crate) fn forget_session(&self, session_key: &str) {
        self.calls().retain(|c| c.session_key != session_key);
    }

    /// Bind a request to the oldest unbound call with this bare tool name
    /// and exactly these arguments, marking it bound. None when nothing
    /// matches: the request is shown unbound rather than guessed.
    pub(crate) fn take_binding(&self, tool: &str, args: &Value) -> Option<Binding> {
        self.take_binding_for(tool, args, None)
    }

    fn take_binding_for(
        &self,
        tool: &str,
        args: &Value,
        attempt: Option<(&str, &str)>,
    ) -> Option<Binding> {
        let args = normalized_args(args);
        let mut calls = self.calls();
        let hit = calls
            .iter_mut()
            .filter(|c| {
                !c.bound
                    && c.tool == tool
                    && c.input == args
                    && attempt.is_none_or(|(session, run)| {
                        c.session_key == session && c.run_id.as_deref() == Some(run)
                    })
            })
            .min_by_key(|c| c.seq)?;
        hit.bound = true;
        Some(Binding {
            tool_use_id: hit.tool_use_id.clone(),
            session_key: hit.session_key.clone(),
        })
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.calls().len()
    }
}

/// The GUI process's registry.
static REGISTRY: ApprovalRegistry = ApprovalRegistry::new();

pub(crate) fn note_tool_use_at(
    session_key: &str,
    run_id: &str,
    id: &str,
    name: &str,
    input: &Value,
) {
    let attempt = crate::shared_mcp_host::active_tag(run_id);
    REGISTRY.note_tool_use_at(session_key, attempt.as_deref(), id, name, input);
}

/// chat.rs: a tool call's result arrived.
pub fn note_tool_result(id: &str) {
    REGISTRY.note_tool_result(id);
}

/// chat.rs: a turn finished; drop its conversation's leftover calls.
pub fn forget_session(session_key: &str) {
    REGISTRY.forget_session(session_key);
}

// ── Broker (GUI process) ────────────────────────────────────────────────────

/// Where the broker delivers its two events. The app handle in production;
/// a recorder in tests.
#[cfg(unix)]
pub(crate) trait ApprovalSink {
    /// The subagent a bound conversation is, when it is one (SA1). The
    /// default knows none (a test sink).
    fn subagent_origin(&self, session_key: &str) -> Option<SubagentOrigin> {
        let _ = session_key;
        None
    }
    /// Where a card from `session_key` is shown when it comes from a project
    /// lead's chain or a subagent of one (2026-11-20): the COO chat. The
    /// default routes nothing (a test sink).
    fn coo_route(&self, session_key: &str) -> Option<CooRoute> {
        let _ = session_key;
        None
    }
    fn request(&self, payload: Value);
    fn resolved(&self, payload: Value);
    /// Pause the no-output watchdog of the run a pending card waits on, for
    /// as long as the returned holds live (merged plan slice 3, A·R2):
    /// waiting on the user is not silence. The app holds the bound
    /// conversation's live turn (every live turn when the card is unbound,
    /// `agent_sessions::hold_watchdog`); a test sink holds nothing.
    fn hold_watchdog(&self, session_key: Option<&str>) -> Vec<crate::run_activity::CardHold> {
        let _ = session_key;
        Vec::new()
    }
}

/// The conversation a card's session is a subagent of (SA1): the card shows
/// in the parent's chat too, naming the subagent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubagentOrigin {
    /// The subagent's own conversation key.
    pub key: String,
    /// The conversation that started it (`sessions.parent_key`).
    pub parent_key: String,
    /// Its title, when it has one.
    pub title: Option<String>,
}

/// [`SubagentOrigin`] for `key` from the vault's session rows: `Some` only
/// for a conversation another one started as its subagent.
pub(crate) fn subagent_origin_in(vault: &std::path::Path, key: &str) -> Option<SubagentOrigin> {
    let row = crate::session_store::session_by_key(vault, key).ok()??;
    if row.fork_kind.as_deref() != Some(crate::session_store::FORK_SUBAGENT) {
        return None;
    }
    let parent_key = row.parent_key.filter(|p| !p.trim().is_empty())?;
    Some(SubagentOrigin {
        key: row.key,
        parent_key,
        title: row.title.filter(|t| !t.trim().is_empty()),
    })
}

/// The card payload's `subagent` field: `null` unless the bound
/// conversation is a subagent.
fn origin_json<S: ApprovalSink + ?Sized>(sink: &S, session_key: Option<&str>) -> Value {
    match session_key.and_then(|key| sink.subagent_origin(key)) {
        Some(o) => json!({ "key": o.key, "parentKey": o.parent_key, "title": o.title }),
        None => Value::Null,
    }
}

/// Why and where a card is raised to the COO (2026-11-20, Alex: "the card
/// must not appear only in that subordinate chat"). A card from a project
/// lead, or from a subagent anywhere below a lead or the COO, is shown once,
/// in the main COO chat, where the person answers it; the asking chat shows
/// that it waits there. The same card id answers both (the answer is by id,
/// from the COO chat, the bell or a phone), so nothing is duplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CooRoute {
    /// The conversation the request came from.
    pub asked_key: String,
    pub asked_title: Option<String>,
    /// `lead` (a project lead's own chat) or `subagent`.
    pub asked_kind: &'static str,
    /// The COO's current conversation.
    pub coo_key: String,
}

/// Furthest a subagent's parents are followed to find its coordinator.
const ROUTE_STEPS: usize = 12;

/// [`CooRoute`] for `key` from the vault: `Some` for a project lead's
/// conversation, or a subagent whose parents lead (within [`ROUTE_STEPS`])
/// to a lead or to a COO conversation. A subagent of an ordinary chat and
/// the COO's own chat are not routed: the first keeps showing in its parent
/// (SA1), the second already is the COO chat. None too before the COO has
/// a conversation.
pub(crate) fn coo_route_in(vault: &std::path::Path, key: &str) -> Option<CooRoute> {
    let main_cwd = crate::main_agent::main_cwd(vault);
    let in_coo = |k: &str| {
        crate::session_store::session_by_key(vault, k)
            .ok()
            .flatten()
            .is_some_and(|row| main_cwd.as_deref().is_some_and(|c| row.cwd == c))
    };
    let row = crate::session_store::session_by_key(vault, key).ok()??;
    let title = row.title.clone().filter(|t| !t.trim().is_empty());
    let kind = if crate::chat_chain::is_lead_conversation(vault, key) {
        "lead"
    } else {
        let mut at = key.to_string();
        let mut reached = false;
        for _ in 0..ROUTE_STEPS {
            let Some(origin) = subagent_origin_in(vault, &at) else {
                break;
            };
            if crate::chat_chain::is_lead_conversation(vault, &origin.parent_key)
                || in_coo(&origin.parent_key)
            {
                reached = true;
                break;
            }
            at = origin.parent_key;
        }
        if !reached {
            return None;
        }
        "subagent"
    };
    if in_coo(key) {
        return None;
    }
    let (coo_key, _) = crate::chat_chain::main_current(vault)?;
    Some(CooRoute {
        asked_key: key.to_string(),
        asked_title: title,
        asked_kind: kind,
        coo_key,
    })
}

/// The card payload's `routedTo` field: `null` unless the card is raised to
/// the COO ([`coo_route_in`]).
fn route_json<S: ApprovalSink + ?Sized>(sink: &S, session_key: Option<&str>) -> Value {
    match session_key.and_then(|key| sink.coo_route(key)) {
        Some(r) => json!({
            "askedKey": r.asked_key,
            "askedTitle": r.asked_title,
            "askedKind": r.asked_kind,
            "cooKey": r.coo_key,
        }),
        None => Value::Null,
    }
}

#[cfg(unix)]
impl ApprovalSink for Core {
    fn subagent_origin(&self, session_key: &str) -> Option<SubagentOrigin> {
        let (config, _) = crate::config::locate_and_load().ok()?;
        subagent_origin_in(&config.vault_path, session_key)
    }
    fn coo_route(&self, session_key: &str) -> Option<CooRoute> {
        let (config, _) = crate::config::locate_and_load().ok()?;
        coo_route_in(&config.vault_path, session_key)
    }
    fn request(&self, payload: Value) {
        let _ = self.emit(EVENT_REQUEST, payload);
    }
    fn resolved(&self, payload: Value) {
        let _ = self.emit(EVENT_RESOLVED, payload);
    }
    fn hold_watchdog(&self, session_key: Option<&str>) -> Vec<crate::run_activity::CardHold> {
        crate::agent_sessions::hold_watchdog(session_key)
    }
}

/// Bind the approval socket and start accepting requests. Called once at
/// core startup (`core_boot`). Non-fatal on failure: the MCP clients just
/// fall back to the native dialog. With no window (`hollow serve`) every
/// request is refused at once ([`HEADLESS_REFUSAL`]).
#[cfg(unix)]
pub(crate) fn start(app: impl Into<Core>) {
    let app = app.into();
    let dir = crate::instance::cache_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("approvals: cannot create {}: {e}", dir.display());
        return;
    }
    // Per-pid path: two Hollow instances never fight over one socket, and a
    // stale file from a crashed run never shadows a live broker.
    let path = dir.join(format!("approvals-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("approvals: bind {} failed: {e}", path.display());
            return;
        }
    };
    // Only this user may connect (the dir is user-owned cache, but be explicit).
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    let _ = SOCKET_PATH.set(path);
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::UnixListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("approvals: tokio listener: {e}");
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        handle_conn(app, stream).await;
                    });
                }
                Err(e) => {
                    eprintln!("approvals: accept: {e}");
                    break;
                }
            }
        }
    });
}

#[cfg(not(unix))]
pub(crate) fn start(_app: impl Into<Core>) {}

/// What a confirm-gated tool hears from a Hollow with no window to show its
/// card (`hollow serve`, M1 PR 2): a refusal at once, never a two-minute
/// wait. TODO(M1 PR3/M2): route the card to a paired phone or a peer Mac.
pub(crate) const HEADLESS_REFUSAL: &str = "Hollow is running without a window (hollow serve), so nobody can see an approval card; nothing ran. Ask the person to do this in Hollow on their Mac, or put it in your reply for them.";

/// Serve one approval request through the core host and the global
/// registry, once the peer check admits it; refused at once with no window.
#[cfg(unix)]
pub(crate) async fn handle_conn(app: Core, stream: tokio::net::UnixStream) {
    if app.gui().is_none() {
        refuse(stream, HEADLESS_REFUSAL).await;
        return;
    }
    serve_admitted(stream, &app, &REGISTRY, crate::run_auth::global()).await;
}

/// One `{"approved":false,"refused":..}` line, which the client returns as
/// its error (`request_via_app_at`), and no card.
#[cfg(unix)]
pub(crate) async fn refuse(mut stream: tokio::net::UnixStream, refused: &str) {
    use tokio::io::AsyncWriteExt;
    let line = format!("{}\n", json!({ "approved": false, "refused": refused }));
    let _ = stream.write_all(line.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Merged plan slice 6 (review A: "the approval socket has no
/// authentication"): a card is only ever posted for a process inside a run
/// Hollow started and registered (`run_auth::check_peer`). Anything else,
/// another app running as this user included, gets a refusal line and no
/// card.
#[cfg(unix)]
pub(crate) async fn serve_admitted<S: ApprovalSink + ?Sized>(
    mut stream: tokio::net::UnixStream,
    sink: &S,
    registry: &ApprovalRegistry,
    peers: &crate::run_auth::Peers,
) {
    use std::os::fd::AsRawFd;
    use tokio::io::AsyncWriteExt;
    let fd = stream.as_raw_fd();
    if let Err(refused) = crate::run_auth::check_peer(peers, fd, "approval socket", None) {
        let line = format!("{}\n", json!({ "approved": false, "refused": refused }));
        let _ = stream.write_all(line.as_bytes()).await;
        let _ = stream.flush().await;
        return;
    }
    serve_conn(stream, sink, registry).await;
}

/// How a request ended.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Verdict(bool),
    TimedOut,
    Cancelled,
}

#[cfg(unix)]
impl Outcome {
    fn reason(self) -> &'static str {
        match self {
            Outcome::Verdict(true) => "allowed",
            Outcome::Verdict(false) => "denied",
            Outcome::TimedOut => "timed_out",
            Outcome::Cancelled => "cancelled",
        }
    }
}

/// Serve one approval request: read the request line, bind it to its tool
/// call, surface it to the UI, wait for the user's answer (or the client
/// giving up or dying), write the verdict back.
#[cfg(unix)]
pub(crate) async fn serve_conn<S: ApprovalSink + ?Sized>(
    stream: tokio::net::UnixStream,
    sink: &S,
    registry: &ApprovalRegistry,
) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let Ok(Some(first)) = lines.next_line().await else {
        return;
    };
    let req: Value = match serde_json::from_str(&first) {
        Ok(v) => v,
        Err(_) => return,
    };
    let text = |key: &str, default: &str| {
        req.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
            .to_string()
    };
    let title = text("title", "Approval");
    let action = text("action", "");
    let kind = text("kind", "generic");
    if action.is_empty() {
        return;
    }
    let tool = req
        .get("tool")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let args = req.get("args").cloned().unwrap_or(Value::Null);

    // Bind to the tool call chat.rs noted. A request that names no tool (an
    // older client, a non-computer gate) is never bound and never waits.
    let mut binding = None;
    if let Some(tool) = tool.as_deref() {
        let deadline = tokio::time::Instant::now() + BIND_WAIT;
        loop {
            binding = registry.take_binding(tool, &args);
            if binding.is_some() || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(BIND_POLL).await;
        }
    }
    let tool_use_id = binding.as_ref().map(|b| b.tool_use_id.clone());
    let session_key = binding.as_ref().map(|b| b.session_key.clone());

    let id = next_card_id();
    let (tx, rx) = tokio::sync::oneshot::channel::<Answer>();
    if let Ok(mut map) = pending().lock() {
        map.insert(id.clone(), tx);
    } else {
        return;
    }
    let card = json!({
        "id": id,
        "title": title,
        "action": action,
        "kind": kind,
        "tool": tool,
        "toolUseId": tool_use_id,
        "sessionKey": session_key,
        "subagent": origin_json(sink, session_key.as_deref()),
        "routedTo": route_json(sink, session_key.as_deref()),
    });
    show_card(&card);
    let _shown = ShownCard(id.clone());
    sink.request(card);
    // Named, so the holds live until this function returns (after the
    // verdict): the card's wait never counts as the run's silence.
    let _paused = sink.hold_watchdog(session_key.as_deref());
    // B4: a client that asked (`waitFromBroker`) learns how long this card
    // waits, as soon as it shows, and keeps that clock instead of its own 120
    // seconds. One that didn't ask gets no such line, so an older client never
    // reads it as a verdict. This function has no timer of its own: the client
    // stays the one clock for these cards.
    if req.get("waitFromBroker").and_then(Value::as_bool) == Some(true) {
        let line = format!("{}\n", json!({ "waitSecs": card_wait().as_secs() }));
        let _ = write_half.write_all(line.as_bytes()).await;
        let _ = write_half.flush().await;
    }

    // Wait for the user's verdict OR the client giving up. A client whose
    // own timeout lapsed writes one `{"cancel":"timed_out"}` line first; any
    // other second line, EOF or read error means it is gone (the turn was
    // cancelled and the CLI killed the MCP server).
    let mut reason: Option<String> = None;
    let outcome = tokio::select! {
        r = rx => match r {
            Ok(answer) => {
                reason = answer.reason;
                Outcome::Verdict(answer.approved)
            }
            Err(_) => Outcome::Cancelled,
        },
        line = lines.next_line() => match line {
            Ok(Some(l)) if is_timed_out_line(&l) => Outcome::TimedOut,
            _ => Outcome::Cancelled,
        },
    };

    match outcome {
        Outcome::Verdict(approved) => {
            let line = format!("{}\n", json!({ "approved": approved, "reason": reason }));
            let _ = write_half.write_all(line.as_bytes()).await;
            let _ = write_half.flush().await;
        }
        Outcome::TimedOut | Outcome::Cancelled => {
            // Client gone before an answer — drop the pending entry (if the
            // select raced, respond() may already have taken it).
            if let Ok(mut map) = pending().lock() {
                map.remove(&id);
            }
        }
    }
    let approved = matches!(outcome, Outcome::Verdict(true));
    let cancelled = !matches!(outcome, Outcome::Verdict(_));
    close_card(&id);
    sink.resolved(json!({
        "id": id,
        "approved": approved,
        "cancelled": cancelled,
        "reason": outcome.reason(),
        "answeredBy": take_answered_by(&id),
        "toolUseId": tool_use_id,
        "sessionKey": session_key,
        "tool": tool,
    }));
}

/// The broker's wait line (`{"waitSecs": N}`), clamped to a sane range, or
/// `None` for anything else (a verdict, a refusal, junk). A verdict has none of
/// these fields, so a line that carries `approved` or `refused` is never one.
#[cfg(unix)]
fn wait_line(line: &str) -> Option<u64> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("approved").is_some() || v.get("refused").is_some() {
        return None;
    }
    let secs = v.get("waitSecs")?.as_u64()?;
    Some(secs.clamp(1, 3600))
}

/// Whether a client's second line reports its own timeout.
#[cfg(unix)]
fn is_timed_out_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| {
            v.get("cancel")
                .and_then(|c| c.as_str())
                .map(|c| c == TIMED_OUT_LINE)
        })
        .unwrap_or(false)
}

// ── In-process asks (GUI process) ───────────────────────────────────────────

/// How an in-process ask ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Allowed,
    Denied,
    TimedOut,
    Cancelled,
}

impl Verdict {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Verdict::Allowed => "allowed",
            Verdict::Denied => "denied",
            Verdict::TimedOut => "timed_out",
            Verdict::Cancelled => "cancelled",
        }
    }
}

/// The GUI process's own gate (the Hollow app broker, PR A3): the same card
/// a socket request gets, asked from inside this process so the agent's
/// server never holds the verdict. Bound to the agent's tool call exactly
/// like a socket request (`tool` is the noted name, `hollow_app:<tool>`,
/// with its `tools/call` arguments). Blocks this thread, which must not be
/// a runtime worker, until the user answers, `timeout` lapses, or `gone()`
/// says the asking call is over (its connection closed, its run ended).
/// Only an answered Allow is [`Verdict::Allowed`].
#[cfg(unix)]
pub(crate) fn ask_in_process<S: ApprovalSink + ?Sized>(
    sink: &S,
    registry: &ApprovalRegistry,
    card: InProcessCard<'_>,
    timeout: Duration,
    gone: &dyn Fn() -> bool,
) -> Verdict {
    ask_in_process_for(sink, registry, card, timeout, gone, None)
}

#[cfg(unix)]
fn ask_in_process_for<S: ApprovalSink + ?Sized>(
    sink: &S,
    registry: &ApprovalRegistry,
    card: InProcessCard<'_>,
    timeout: Duration,
    gone: &dyn Fn() -> bool,
    attempt: Option<(&str, &str)>,
) -> Verdict {
    LAST_DECLINE.with(|slot| slot.borrow_mut().take());
    LAST_ANSWERED_BY.with(|slot| slot.borrow_mut().take());
    LAST_GRANT.with(|slot| slot.borrow_mut().take());
    let args_hash = EXPECTED_FP
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(|| {
            crate::approval_terms::fingerprint(card.tool.unwrap_or_default(), card.args)
        });
    let mut binding = None;
    if let Some(tool) = card.tool {
        let deadline = std::time::Instant::now() + BIND_WAIT;
        loop {
            if gone() {
                return Verdict::Cancelled;
            }
            binding = registry.take_binding_for(tool, card.args, attempt);
            if binding.is_some() || std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(BIND_POLL);
        }
    }
    let tool_use_id = binding.as_ref().map(|b| b.tool_use_id.clone());
    // An HTTP call is never attached to a different conversation, even if
    // its tool event has not arrived before the bounded binding wait.
    let session_key = attempt
        .map(|(session, _)| session.to_string())
        .or_else(|| binding.as_ref().map(|b| b.session_key.clone()));
    let id = next_card_id();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<Answer>();
    match pending().lock() {
        Ok(mut map) => {
            map.insert(id.clone(), tx);
        }
        Err(_) => return Verdict::Cancelled,
    }
    let shown = json!({
        "id": id,
        "title": card.title,
        "action": card.action,
        "kind": card.kind,
        "tool": card.tool,
        "toolUseId": tool_use_id,
        "sessionKey": session_key,
        "subagent": origin_json(sink, session_key.as_deref()),
        "routedTo": route_json(sink, session_key.as_deref()),
        "argsHash": args_hash,
        "notCovered": crate::approval_terms::not_covered(card.kind == "app-external"),
    });
    show_card(&shown);
    let _shown = ShownCard(id.clone());
    sink.request(shown);
    // Held until the verdict (merged plan slice 3), as in `serve_conn`.
    let _paused = sink.hold_watchdog(session_key.as_deref());
    let deadline = std::time::Instant::now() + timeout;
    let verdict = loop {
        if gone() {
            break Verdict::Cancelled;
        }
        match rx.try_recv() {
            Ok(answer) => {
                break if answer.approved {
                    Verdict::Allowed
                } else {
                    LAST_DECLINE.with(|slot| *slot.borrow_mut() = answer.reason);
                    Verdict::Denied
                }
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => break Verdict::Cancelled,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
        }
        if gone() {
            break Verdict::Cancelled;
        }
        if std::time::Instant::now() >= deadline {
            break Verdict::TimedOut;
        }
        std::thread::sleep(ASK_POLL);
    };
    // No answer yet when the wait ended: take the card out. If its sender is
    // already gone, an answer (the window's or a phone's) won the race with
    // this timeout, so honour it rather than report a verdict nobody gave.
    let mut verdict = verdict;
    if !matches!(verdict, Verdict::Allowed | Verdict::Denied) {
        let taken = pending()
            .lock()
            .map(|mut map| map.remove(&id).is_some())
            .unwrap_or(true);
        if !taken {
            if let Ok(answer) = rx.try_recv() {
                verdict = if answer.approved {
                    Verdict::Allowed
                } else {
                    LAST_DECLINE.with(|slot| *slot.borrow_mut() = answer.reason);
                    Verdict::Denied
                };
            }
        }
    }
    close_card(&id);
    if verdict == Verdict::Allowed {
        LAST_GRANT.with(|slot| {
            *slot.borrow_mut() = Some(crate::approval_terms::Grant {
                card_id: id.clone(),
                fingerprint: args_hash.clone(),
            })
        });
    }
    // B4: a phone's answer says so (`None` from the window).
    let by = take_answered_by(&id);
    LAST_ANSWERED_BY.with(|slot| *slot.borrow_mut() = by.clone());
    sink.resolved(json!({
        "id": id,
        "approved": verdict == Verdict::Allowed,
        "cancelled": !matches!(verdict, Verdict::Allowed | Verdict::Denied),
        "reason": verdict.reason(),
        "answeredBy": by,
        "toolUseId": tool_use_id,
        "sessionKey": session_key,
        "tool": card.tool,
        "argsHash": args_hash,
    }));
    verdict
}

/// How often an in-process ask checks for the answer.
#[cfg(unix)]
const ASK_POLL: Duration = Duration::from_millis(100);

/// What an in-process card shows and which tool call it belongs to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InProcessCard<'a> {
    pub title: &'a str,
    pub action: &'a str,
    /// `app` (a change) or `app-external` (it leaves this Mac).
    pub kind: &'a str,
    /// The noted tool name (`hollow_app:<tool>`), when the call came from a
    /// chat's tool.
    pub tool: Option<&'a str>,
    pub args: &'a Value,
}

/// [`ask_in_process`] through the app's events and the global registry.
#[cfg(unix)]
pub(crate) fn ask_app(app: &Core, card: InProcessCard<'_>, gone: &dyn Fn() -> bool) -> Verdict {
    ask_in_process(app, &REGISTRY, card, card_wait(), gone)
}

#[cfg(unix)]
pub(crate) fn ask_app_scoped(
    app: &Core,
    card: InProcessCard<'_>,
    gone: &dyn Fn() -> bool,
    session: &str,
    run: &str,
) -> Verdict {
    ask_in_process_for(
        app,
        &REGISTRY,
        card,
        card_wait(),
        gone,
        Some((session, run)),
    )
}

/// AG4b: the person's answer to the card of a deferred Claude tool call
/// (`crate::claude_defer`), by the shared in-process card in the turn's own
/// session and run. Blocks until the answer, [`card_wait`] or `gone`. Only
/// an Allow approves: a Deny keeps the person's words, a timeout or a turn
/// that ended declines with a fixed sentence the agent can relay.
pub(crate) fn ask_deferred_call<S: ApprovalSink + ?Sized>(
    sink: &S,
    registry: &ApprovalRegistry,
    card: InProcessCard<'_>,
    timeout: Duration,
    gone: &dyn Fn() -> bool,
    session: &str,
    run: &str,
) -> Answer {
    #[cfg(unix)]
    let verdict = ask_in_process_for(sink, registry, card, timeout, gone, Some((session, run)));
    #[cfg(not(unix))]
    let verdict = {
        let _ = (sink, registry, card, timeout, gone, session, run);
        Verdict::Cancelled
    };
    match verdict {
        Verdict::Allowed => Answer {
            approved: true,
            reason: None,
        },
        Verdict::Denied => Answer {
            approved: false,
            reason: take_decline_reason(),
        },
        Verdict::TimedOut => Answer {
            approved: false,
            reason: Some("no answer came in time".into()),
        },
        Verdict::Cancelled => Answer {
            approved: false,
            reason: Some("the turn ended before an answer".into()),
        },
    }
}

/// [`ask_deferred_call`] through the app's events and the global registry.
pub(crate) fn ask_deferred_app(
    app: &Core,
    card: InProcessCard<'_>,
    gone: &dyn Fn() -> bool,
    session: &str,
    run: &str,
) -> Answer {
    ask_deferred_call(app, &REGISTRY, card, card_wait(), gone, session, run)
}

#[cfg(not(unix))]
pub(crate) fn ask_app_scoped(
    _app: &Core,
    _card: InProcessCard<'_>,
    _gone: &dyn Fn() -> bool,
    _session: &str,
    _run: &str,
) -> Verdict {
    Verdict::Cancelled
}

#[cfg(not(unix))]
pub(crate) fn ask_app(_app: &Core, _card: InProcessCard<'_>, _gone: &dyn Fn() -> bool) -> Verdict {
    Verdict::Cancelled
}

// ── Client (MCP subprocess) ─────────────────────────────────────────────────

/// The approval gate the MCP tools call before an irreversible action.
/// Prefers the inline-chat channel; falls back to the native Allow/Deny
/// dialog when the app channel is unavailable. `title` names the surface
/// ("Microsoft 365", "Computer control"); `kind` tags the request for the
/// UI ("m365" | "computer"). A request without a tool name is never bound
/// to a tool call. `tool` is the bare tool name and `args` its `tools/call`
/// arguments as received, so the broker can bind the request to the agent's
/// tool call (Paperclip PR 21).
pub async fn confirm_tool(
    title: &str,
    action: &str,
    kind: &str,
    tool: Option<&str>,
    args: Option<&Value>,
) -> Result<(), String> {
    if let Some(res) = request_via_app(title, action, kind, tool, args).await {
        return res;
    }
    crate::computer_mcp::native_confirm(&format!("Hollow — {title}"), action).await
}

/// Ask the GUI process for approval over the broker socket. `None` = channel
/// unavailable (caller falls back to the native dialog); `Some(verdict)` =
/// the channel answered authoritatively (including deny and timeout).
#[cfg(unix)]
async fn request_via_app(
    title: &str,
    action: &str,
    kind: &str,
    tool: Option<&str>,
    args: Option<&Value>,
) -> Option<Result<(), String>> {
    let sock = std::env::var(ENV_SOCK).ok().filter(|s| !s.is_empty())?;
    request_via_app_at(
        &sock,
        Duration::from_secs(CLIENT_TIMEOUT_SECS),
        title,
        action,
        kind,
        tool,
        args,
    )
    .await
}

#[cfg(not(unix))]
async fn request_via_app(
    _title: &str,
    _action: &str,
    _kind: &str,
    _tool: Option<&str>,
    _args: Option<&Value>,
) -> Option<Result<(), String>> {
    None
}

/// [`request_via_app`] against an explicit socket and timeout (tests pass a
/// temp socket and a short timeout).
#[cfg(unix)]
pub(crate) async fn request_via_app_at(
    sock: &str,
    timeout: Duration,
    title: &str,
    action: &str,
    kind: &str,
    tool: Option<&str>,
    args: Option<&Value>,
) -> Option<Result<(), String>> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut stream = tokio::net::UnixStream::connect(sock).await.ok()?;
    let req = format!(
        "{}\n",
        json!({
            "title": title,
            "action": action,
            "kind": kind,
            "tool": tool,
            "args": args.cloned().unwrap_or(Value::Null),
            // B4: ask the broker for the card's wait (an older broker sends none).
            "waitFromBroker": true,
        })
    );
    stream.write_all(req.as_bytes()).await.ok()?;
    stream.flush().await.ok()?;

    let (read_half, mut write_half) = stream.split();
    let mut lines = BufReader::new(read_half).lines();
    // The card's wait: this client's own clock until the broker says better
    // (B4: a first line `{"waitSecs": N}`, which is no verdict).
    let mut deadline = tokio::time::Instant::now() + timeout;
    let mut first = true;
    let resp = loop {
        let got = tokio::time::timeout_at(deadline, lines.next_line()).await;
        if let (true, Ok(Ok(Some(line)))) = (first, &got) {
            first = false;
            if let Some(secs) = wait_line(line) {
                deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
                continue;
            }
        }
        break got;
    };
    Some(match resp {
        Ok(Ok(Some(line))) => {
            let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            if v.get("approved").and_then(|b| b.as_bool()) == Some(true) {
                Ok(())
            } else if let Some(refused) = v.get("refused").and_then(Value::as_str) {
                // The peer check turned the request away; no card was shown.
                Err(refused.to_string())
            } else {
                Err(
                    match v
                        .get("reason")
                        .and_then(Value::as_str)
                        .and_then(clean_reason)
                    {
                        Some(why) => format!("the user denied this action: {why}"),
                        None => "the user denied this action".into(),
                    },
                )
            }
        }
        // Broker hung up without a verdict (app quit mid-request).
        Ok(_) => Err("the approval request was cancelled".into()),
        // No answer within the window — same semantics as the dialog timeout.
        // Tell the broker why (best effort) so the card reads "timed out",
        // then drop the stream, which clears it.
        Err(_) => {
            let line = format!("{}\n", json!({ "cancel": TIMED_OUT_LINE }));
            let _ = tokio::time::timeout(Duration::from_millis(500), async {
                let _ = write_half.write_all(line.as_bytes()).await;
                let _ = write_half.flush().await;
            })
            .await;
            Err("the user did not approve (the approval request timed out)".into())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_binding_matches_conversation_and_attempt() {
        let registry = ApprovalRegistry::new();
        let args = json!({"name":"Notes"});
        let tool = "mcp__hollow_computer__quit_app";
        registry.note_tool_use_at("a", Some("old"), "old-call", tool, &args);
        registry.note_tool_use_at("b", Some("other"), "other-call", tool, &args);
        registry.note_tool_use_at("a", Some("new"), "new-call", tool, &args);
        assert_eq!(
            registry
                .take_binding_for("quit_app", &args, Some(("a", "new")))
                .unwrap()
                .tool_use_id,
            "new-call"
        );
        assert!(registry
            .take_binding_for("quit_app", &args, Some(("a", "new")))
            .is_none());
        assert!(registry
            .take_binding_for("quit_app", &args, Some(("b", "new")))
            .is_none());
        assert_eq!(
            registry
                .take_binding_for("quit_app", &args, Some(("a", "old")))
                .unwrap()
                .tool_use_id,
            "old-call"
        );
    }

    /// A recorder sink for in-process asks.
    #[cfg(unix)]
    #[derive(Default)]
    struct Recorded {
        requests: Mutex<Vec<Value>>,
        resolved: Mutex<Vec<Value>>,
    }

    #[cfg(unix)]
    impl ApprovalSink for Recorded {
        fn request(&self, payload: Value) {
            self.requests.lock().unwrap().push(payload);
        }
        fn resolved(&self, payload: Value) {
            self.resolved.lock().unwrap().push(payload);
        }
    }

    /// AG4b: a deferred Claude call's card is scoped to the turn's session,
    /// only an Allow approves, a Deny keeps the person's words, and a gone
    /// turn or a timeout declines with a fixed sentence.
    #[cfg(unix)]
    #[test]
    fn a_deferred_call_card_is_scoped_and_only_allow_approves() {
        let registry = ApprovalRegistry::new();
        let sink = std::sync::Arc::new(Recorded::default());
        let args = json!({"command": "ls"});
        let card = || InProcessCard {
            title: "Claude: Bash",
            action: "Run a shell command: ls",
            kind: "app",
            tool: None,
            args: &args,
        };
        let answer = |nth: usize, approved: bool, reason: Option<&'static str>| {
            let sink = sink.clone();
            std::thread::spawn(move || {
                for _ in 0..250 {
                    let id = sink
                        .requests
                        .lock()
                        .unwrap()
                        .get(nth)
                        .and_then(|r| r["id"].as_str().map(str::to_string));
                    if let Some(id) = id {
                        if respond_with(&id, approved, reason).is_ok() {
                            return;
                        }
                    }
                    // sleep-ok: poll for the card this test's own ask shows
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };
        let ask = |gone: &dyn Fn() -> bool, timeout| {
            ask_deferred_call(&*sink, &registry, card(), timeout, gone, "s-9", "run-9")
        };
        let t = answer(0, true, None);
        let allowed = ask(&|| false, Duration::from_secs(5));
        t.join().unwrap();
        assert_eq!(
            allowed,
            Answer {
                approved: true,
                reason: None
            }
        );
        let request = sink.requests.lock().unwrap()[0].clone();
        assert_eq!(request["sessionKey"], json!("s-9"));
        assert_eq!(request["kind"], json!("app"));

        let t = answer(1, false, Some("use staging"));
        let denied = ask(&|| false, Duration::from_secs(5));
        t.join().unwrap();
        assert_eq!(
            denied,
            Answer {
                approved: false,
                reason: Some("use staging".into())
            }
        );

        let gone = ask(&|| true, Duration::from_secs(5));
        assert!(!gone.approved);
        assert_eq!(
            gone.reason.as_deref(),
            Some("the turn ended before an answer")
        );
        let late = ask(&|| false, Duration::from_millis(200));
        assert!(!late.approved);
        assert_eq!(late.reason.as_deref(), Some("no answer came in time"));
    }

    /// ND6: an Allow is a grant for the exact call its card showed. The
    /// card says what the yes does not cover and carries the fingerprint; a
    /// changed call is void, a Deny gives no grant, and a grant is spent once.
    #[cfg(unix)]
    #[test]
    fn an_allow_is_a_grant_for_the_exact_call_and_a_changed_call_is_void() {
        use crate::approval_terms::{fingerprint, verify};
        let registry = ApprovalRegistry::new();
        let sink = std::sync::Arc::new(Recorded::default());
        let args = json!({ "id": "x.send", "args": { "to": "a@x.com", "n": 1 } });
        let card = InProcessCard {
            title: "Hollow",
            action: "Send to a@x.com",
            kind: "app-external",
            tool: Some("hollow_app:run_action"),
            args: &args,
        };
        let answer = |nth: usize, approved: bool| {
            let sink = sink.clone();
            std::thread::spawn(move || {
                for _ in 0..250 {
                    let id = sink
                        .requests
                        .lock()
                        .unwrap()
                        .get(nth)
                        .and_then(|r| r["id"].as_str().map(str::to_string));
                    if let Some(id) = id {
                        if respond(&id, approved).is_ok() {
                            return;
                        }
                    }
                    // sleep-ok: poll for the card this test's own ask shows
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };
        // Bound to the card's own tool and arguments when nothing says otherwise.
        let t = answer(0, true);
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| false),
            Verdict::Allowed
        );
        t.join().unwrap();
        let grant = take_grant().expect("an Allow is a grant");
        assert!(take_grant().is_none(), "spent by taking it");
        assert!(verify(&grant, "hollow_app:run_action", &args).is_ok());
        let changed = json!({ "id": "x.send", "args": { "to": "b@x.com", "n": 1 } });
        assert!(verify(&grant, "hollow_app:run_action", &changed).is_err());
        let request = sink.requests.lock().unwrap()[0].clone();
        assert_eq!(
            request["argsHash"],
            json!(fingerprint("hollow_app:run_action", &args))
        );
        let lines = request["notCovered"].as_array().unwrap();
        assert_eq!(
            lines.len(),
            4,
            "an outward card also says what it cannot check"
        );
        let resolved = sink.resolved.lock().unwrap()[0].clone();
        assert_eq!(resolved["argsHash"], request["argsHash"]);

        // The executor's own call wins over the card's arguments.
        expect_call("x.send", &json!({ "to": "a@x.com", "n": 1 }));
        let t = answer(1, true);
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| false),
            Verdict::Allowed
        );
        t.join().unwrap();
        let grant = take_grant().unwrap();
        assert!(verify(&grant, "x.send", &json!({ "n": 1, "to": "a@x.com" })).is_ok());
        assert!(verify(&grant, "x.send", &json!({ "n": 2, "to": "a@x.com" })).is_err());

        // A Deny gives no grant, and an unused expectation does not leak.
        expect_call("x.send", &json!({}));
        let t = answer(2, false);
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| false),
            Verdict::Denied
        );
        t.join().unwrap();
        assert!(take_grant().is_none());
    }

    /// PR A3: the broker's own card is bound to the Hollow app tool call, and
    /// only the user's Allow allows it; a Deny, a timeout and a gone call
    /// never do, and each resolves the card.
    #[cfg(unix)]
    #[test]
    fn an_in_process_ask_binds_the_app_call_and_only_allow_allows() {
        let registry = ApprovalRegistry::new();
        let args = json!({ "id": "goals.trash_goal", "args": { "url": "vault:goals/a.md" } });
        registry.note_tool_use("s-1", "tu-app-1", "mcp__hollow_app__run_action", &args);
        registry.note_tool_use(
            "s-1",
            "tu-read",
            "mcp__hollow_tasks__list_tasks",
            &json!({}),
        );
        assert_eq!(
            registry.len(),
            1,
            "only Hollow app calls and confirm-gated computer calls are noted"
        );
        let sink = std::sync::Arc::new(Recorded::default());
        let card = InProcessCard {
            title: "Hollow",
            action: "Trash goal a",
            kind: "app",
            tool: Some("hollow_app:run_action"),
            args: &args,
        };

        // Answer only this test's own card (the pending map is process-wide,
        // and other tests' broker requests sit in it too).
        let answer = |nth: usize, approved: bool| {
            let sink = sink.clone();
            std::thread::spawn(move || {
                for _ in 0..250 {
                    let id = sink
                        .requests
                        .lock()
                        .unwrap()
                        .get(nth)
                        .and_then(|r| r["id"].as_str().map(str::to_string));
                    if let Some(id) = id {
                        if respond(&id, approved).is_ok() {
                            return;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };
        let t = answer(0, true);
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| false),
            Verdict::Allowed
        );
        t.join().unwrap();
        let request = sink.requests.lock().unwrap()[0].clone();
        assert_eq!(request["toolUseId"], json!("tu-app-1"));
        assert_eq!(request["sessionKey"], json!("s-1"));
        assert_eq!(request["kind"], json!("app"));

        let t = answer(1, false);
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| false),
            Verdict::Denied
        );
        t.join().unwrap();
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_millis(250), &|| {
                false
            }),
            Verdict::TimedOut
        );
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| true),
            Verdict::Cancelled
        );
        assert_eq!(sink.requests.lock().unwrap().len(), 3, "no cancelled card");
        assert_eq!(sink.resolved.lock().unwrap().len(), 3);
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| {
                sink.requests.lock().unwrap().len() > 3
            }),
            Verdict::Cancelled
        );
        assert_eq!(sink.requests.lock().unwrap().len(), 4);
        let reasons: Vec<Value> = sink
            .resolved
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["reason"].clone())
            .collect();
        assert_eq!(
            reasons,
            vec![
                json!("allowed"),
                json!("denied"),
                json!("timed_out"),
                json!("cancelled")
            ]
        );
        let ours: Vec<Value> = sink
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["id"].clone())
            .collect();
        assert!(
            pending()
                .lock()
                .unwrap()
                .keys()
                .all(|k| !ours.contains(&json!(k))),
            "no card is left pending"
        );
    }

    /// B·F15: a card on screen is listed by `pending_cards` until it
    /// resolves; an answered card is never left listed.
    #[cfg(unix)]
    #[test]
    fn a_shown_card_is_listed_until_it_resolves() {
        let registry = ApprovalRegistry::new();
        let sink = std::sync::Arc::new(Recorded::default());
        let args = json!({});
        let card = InProcessCard {
            title: "Hollow",
            action: "Listed card for the needs-you query",
            kind: "app",
            tool: None,
            args: &args,
        };
        let seen = std::sync::Arc::new(Mutex::new(None::<PendingCard>));
        let watcher = {
            let sink = sink.clone();
            let seen = seen.clone();
            std::thread::spawn(move || {
                for _ in 0..250 {
                    let id = sink
                        .requests
                        .lock()
                        .unwrap()
                        .first()
                        .and_then(|r| r["id"].as_str().map(str::to_string));
                    if let Some(id) = id {
                        *seen.lock().unwrap() = pending_cards().into_iter().find(|c| c.id == id);
                        respond(&id, true).unwrap();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
        };
        assert_eq!(
            ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| false),
            Verdict::Allowed
        );
        watcher.join().unwrap();
        let listed = seen
            .lock()
            .unwrap()
            .clone()
            .expect("the card was listed while it waited");
        assert_eq!(listed.action, "Listed card for the needs-you query");
        assert_eq!(listed.kind, "app");
        assert!(
            pending_cards().iter().all(|c| c.id != listed.id),
            "a resolved card is not listed"
        );
    }

    /// B·F15: a card whose in-process wait unwinds (here a panicking
    /// `gone` check) is not left listed as waiting forever.
    #[cfg(unix)]
    #[test]
    fn an_unwound_in_process_wait_takes_its_card_off_the_list() {
        let registry = ApprovalRegistry::new();
        let sink = Recorded::default();
        let args = json!({});
        let card = InProcessCard {
            title: "Hollow",
            action: "Card whose wait panics",
            kind: "app",
            tool: None,
            args: &args,
        };
        let listed = Mutex::new(None::<String>);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ask_in_process(&sink, &registry, card, Duration::from_secs(5), &|| {
                let id = sink.requests.lock().unwrap()[0]["id"]
                    .as_str()
                    .unwrap()
                    .to_string();
                if pending_cards().iter().any(|c| c.id == id) {
                    *listed.lock().unwrap() = Some(id);
                }
                panic!("the wait unwinds");
            })
        }));
        assert!(unwound.is_err());
        let id = listed
            .lock()
            .unwrap()
            .clone()
            .expect("the card was listed while it waited");
        assert!(
            pending_cards().iter().all(|c| c.id != id),
            "an unwound wait leaves no card listed"
        );
        assert!(
            !pending().lock().unwrap().contains_key(&id),
            "nor an answer channel"
        );
    }

    #[test]
    fn respond_to_unknown_id_errors() {
        let err = respond("apr-does-not-exist", true).unwrap_err();
        assert!(err.contains("no pending approval"), "{err}");
    }

    #[test]
    fn respond_delivers_the_verdict_once() {
        let (tx, mut rx) = tokio::sync::oneshot::channel::<Answer>();
        pending().lock().unwrap().insert("apr-test-1".into(), tx);
        respond("apr-test-1", true).unwrap();
        assert_eq!(
            rx.try_recv(),
            Ok(Answer {
                approved: true,
                reason: None
            })
        );
        // Second respond on the same id: gone from the registry.
        assert!(respond("apr-test-1", false).is_err());
    }

    /// One test for everything touching the `HOLLOW_APPROVAL_SOCK` env —
    /// cargo runs tests in parallel threads and env vars are process-global,
    /// so splitting these would race.
    #[cfg(unix)]
    #[test]
    fn client_round_trips_over_the_socket_and_falls_back_without_env() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

            // No env → "channel unavailable" (→ native-dialog fallback), not a hang.
            std::env::remove_var(ENV_SOCK);
            assert!(request_via_app("T", "act", "m365", None, None)
                .await
                .is_none());

            // Wire protocol end to end without an AppHandle: a tiny in-test
            // broker that approves anything named "yes" and denies the rest.
            let dir = std::env::temp_dir();
            let path = dir.join(format!("hollow-apr-test-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let listener = tokio::net::UnixListener::bind(&path).unwrap();
            let server = tokio::spawn(async move {
                for _ in 0..2 {
                    let (stream, _) = listener.accept().await.unwrap();
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    let line = lines.next_line().await.unwrap().unwrap();
                    let v: Value = serde_json::from_str(&line).unwrap();
                    let approved = v["action"] == json!("yes");
                    let resp = format!("{}\n", json!({ "approved": approved }));
                    w.write_all(resp.as_bytes()).await.unwrap();
                }
            });

            std::env::set_var(ENV_SOCK, &path);
            assert!(request_via_app("T", "yes", "m365", None, None)
                .await
                .unwrap()
                .is_ok());
            let denied = request_via_app("T", "no", "m365", None, None)
                .await
                .unwrap();
            assert!(denied.unwrap_err().contains("denied"));
            std::env::remove_var(ENV_SOCK);

            server.await.unwrap();
            let _ = std::fs::remove_file(&path);
        });
    }

    /// Paperclip PR 21: a request binds to the call with the same tool and
    /// the same arguments, oldest first, and never to anything else.
    #[test]
    fn binding_matches_tool_and_arguments_and_never_guesses() {
        let reg = ApprovalRegistry::new();
        let quit = "mcp__hollow_computer__quit_app";
        reg.note_tool_use("s-1", "toolu_a", quit, &json!({ "name": "Safari" }));
        reg.note_tool_use("s-1", "toolu_b", quit, &json!({ "name": "Notes" }));
        // Tier 1 never asks, so it is never kept; nor is another server.
        reg.note_tool_use(
            "s-1",
            "toolu_c",
            "mcp__hollow_computer__open_url",
            &json!({ "url": "https://x.test" }),
        );
        reg.note_tool_use(
            "s-1",
            "toolu_d",
            "mcp__hollow_vault__search_vault",
            &json!({ "query": "quit_app" }),
        );
        // An id-less call can never be badged.
        reg.note_tool_use("s-1", "", quit, &json!({ "name": "Mail" }));
        assert_eq!(reg.len(), 2);

        let b = reg
            .take_binding("quit_app", &json!({ "name": "Notes" }))
            .unwrap();
        assert_eq!(
            b,
            Binding {
                tool_use_id: "toolu_b".into(),
                session_key: "s-1".into()
            }
        );
        assert_eq!(
            reg.take_binding("quit_app", &json!({ "name": "Safari" }))
                .unwrap()
                .tool_use_id,
            "toolu_a"
        );
        // Each call binds once; unknown arguments bind nothing.
        assert!(reg
            .take_binding("quit_app", &json!({ "name": "Safari" }))
            .is_none());
        assert!(reg
            .take_binding("quit_app", &json!({ "name": "Mail" }))
            .is_none());
        assert!(reg
            .take_binding("open_url", &json!({ "url": "https://x.test" }))
            .is_none());

        // Identical arguments: the oldest first.
        reg.note_tool_use("s-2", "toolu_e", quit, &json!({ "name": "Music" }));
        reg.note_tool_use("s-2", "toolu_f", quit, &json!({ "name": "Music" }));
        assert_eq!(
            reg.take_binding("quit_app", &json!({ "name": "Music" }))
                .unwrap()
                .tool_use_id,
            "toolu_e"
        );
        assert_eq!(
            reg.take_binding("quit_app", &json!({ "name": "Music" }))
                .unwrap()
                .tool_use_id,
            "toolu_f"
        );

        // No arguments and `{}` are the same call.
        reg.note_tool_use(
            "s-2",
            "toolu_g",
            "mcp__hollow_computer__browser_current_tab",
            &json!({}),
        );
        assert_eq!(
            reg.take_binding("browser_current_tab", &Value::Null)
                .unwrap()
                .tool_use_id,
            "toolu_g"
        );

        // Results and finished sessions remove entries.
        let before = reg.len();
        reg.note_tool_result("toolu_a");
        assert_eq!(reg.len(), before - 1);
        reg.forget_session("s-2");
        assert!(reg.calls().iter().all(|c| c.session_key != "s-2"));

        // Capped at 64, oldest dropped.
        let capped = ApprovalRegistry::new();
        for i in 0..70 {
            capped.note_tool_use(
                "s-3",
                &format!("toolu_{i}"),
                quit,
                &json!({ "name": format!("App{i}") }),
            );
        }
        assert_eq!(capped.len(), REGISTRY_CAP);
        assert!(capped
            .take_binding("quit_app", &json!({ "name": "App0" }))
            .is_none());
        assert!(capped
            .take_binding("quit_app", &json!({ "name": "App69" }))
            .is_some());
    }

    /// Records the broker's events instead of emitting them.
    #[cfg(unix)]
    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<(&'static str, Value)>>,
    }

    #[cfg(unix)]
    impl ApprovalSink for Recorder {
        fn request(&self, payload: Value) {
            self.events.lock().unwrap().push(("request", payload));
        }
        fn resolved(&self, payload: Value) {
            self.events.lock().unwrap().push(("resolved", payload));
        }
        fn hold_watchdog(&self, session_key: Option<&str>) -> Vec<crate::run_activity::CardHold> {
            self.events
                .lock()
                .unwrap()
                .push(("hold", json!(session_key)));
            Vec::new()
        }
    }

    #[cfg(unix)]
    impl Recorder {
        fn nth(&self, kind: &str, n: usize) -> Option<Value> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, _)| *k == kind)
                .nth(n)
                .map(|(_, v)| v.clone())
        }

        async fn wait_nth(&self, kind: &str, n: usize) -> Value {
            for _ in 0..200 {
                if let Some(v) = self.nth(kind, n) {
                    return v;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("no {kind} event #{n}");
        }
    }

    /// A broker on a temp socket serving `conns` connections with
    /// [`serve_conn`], a test registry and a recorder.
    #[cfg(unix)]
    fn test_broker(
        tag: &str,
        conns: usize,
        registry: std::sync::Arc<ApprovalRegistry>,
    ) -> (
        String,
        std::sync::Arc<Recorder>,
        tokio::task::JoinHandle<()>,
    ) {
        let path =
            std::env::temp_dir().join(format!("hollow-apr-{tag}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let recorder = std::sync::Arc::new(Recorder::default());
        let rec = recorder.clone();
        let server = tokio::spawn(async move {
            let mut served = Vec::new();
            for _ in 0..conns {
                let (stream, _) = listener.accept().await.unwrap();
                let rec = rec.clone();
                let reg = registry.clone();
                served.push(tokio::spawn(async move {
                    serve_conn(stream, &*rec, &reg).await;
                }));
            }
            for s in served {
                s.await.unwrap();
            }
        });
        (path.to_string_lossy().into_owned(), recorder, server)
    }

    #[cfg(unix)]
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// Merged plan slice 6 (review A: "the approval socket has no
    /// authentication"): a process outside every run Hollow started gets a
    /// refusal and no card; the same request from inside a registered run
    /// (here this process leads it) gets its card and the user's verdict.
    #[cfg(unix)]
    #[test]
    fn only_a_process_inside_a_registered_run_can_post_a_card() {
        runtime().block_on(async {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let reg = std::sync::Arc::new(ApprovalRegistry::new());
            let rec = std::sync::Arc::new(Recorder::default());
            let line = format!(
                "{}\n",
                json!({ "title": "T", "action": "a forged card", "kind": "computer" })
            );

            let outside = crate::run_auth::Peers::leaked();
            let (mut client, server) = tokio::net::UnixStream::pair().unwrap();
            let serving = {
                let (rec, reg) = (rec.clone(), reg.clone());
                tokio::spawn(async move { serve_admitted(server, &*rec, &reg, outside).await })
            };
            client.write_all(line.as_bytes()).await.unwrap();
            // Refused at once; a served request would wait on its card, so a
            // missing peer check fails here instead of hanging the suite.
            tokio::time::timeout(Duration::from_secs(10), serving)
                .await
                .expect("an outsider's request is refused, not left waiting on a card")
                .unwrap();
            let mut answer = String::new();
            let mut reader = BufReader::new(&mut client);
            reader.read_line(&mut answer).await.unwrap();
            let answer: Value = serde_json::from_str(&answer).unwrap();
            assert_eq!(answer["approved"], json!(false));
            let refused = answer["refused"].as_str().unwrap();
            assert!(refused.contains("Hollow refused"), "{refused}");
            assert!(rec.nth("request", 0).is_none(), "no card for an outsider");

            let inside = crate::run_auth::Peers::leaked();
            inside.register("approvals-test", std::process::id() as i32, Vec::new());
            let (mut client, server) = tokio::net::UnixStream::pair().unwrap();
            let serving = {
                let (rec, reg) = (rec.clone(), reg.clone());
                tokio::spawn(async move { serve_admitted(server, &*rec, &reg, inside).await })
            };
            client.write_all(line.as_bytes()).await.unwrap();
            let req = rec.wait_nth("request", 0).await;
            respond(req["id"].as_str().unwrap(), true).unwrap();
            serving.await.unwrap();
            let mut answer = String::new();
            let mut reader = BufReader::new(&mut client);
            reader.read_line(&mut answer).await.unwrap();
            let answer: Value = serde_json::from_str(&answer).unwrap();
            assert_eq!(answer["approved"], json!(true));
        });
    }

    /// B·F15: a card whose serving future is dropped mid-wait (its task
    /// aborted) is not left listed as waiting forever.
    #[cfg(unix)]
    #[test]
    fn a_dropped_broker_wait_takes_its_card_off_the_list() {
        runtime().block_on(async {
            use tokio::io::AsyncWriteExt;
            let reg = std::sync::Arc::new(ApprovalRegistry::new());
            let rec = std::sync::Arc::new(Recorder::default());
            let (mut client, server) = tokio::net::UnixStream::pair().unwrap();
            let serving = {
                let rec = rec.clone();
                let reg = reg.clone();
                tokio::spawn(async move { serve_conn(server, &*rec, &reg).await })
            };
            let line = format!(
                "{}\n",
                json!({ "title": "T", "action": "dropped mid-wait", "kind": "computer" })
            );
            client.write_all(line.as_bytes()).await.unwrap();
            let req = rec.wait_nth("request", 0).await;
            let id = req["id"].as_str().unwrap().to_string();
            assert!(
                pending_cards().iter().any(|c| c.id == id),
                "listed while it waits"
            );
            serving.abort();
            assert!(serving.await.unwrap_err().is_cancelled());
            assert!(
                pending_cards().iter().all(|c| c.id != id),
                "a dropped wait leaves no card listed"
            );
            assert!(
                !pending().lock().unwrap().contains_key(&id),
                "nor an answer channel"
            );
            drop(client);
        });
    }

    /// Paperclip PR 21: every ending of a request is reported with its
    /// reason, and the legacy `approved` / `cancelled` fields keep meaning
    /// what they meant.
    #[cfg(unix)]
    #[test]
    fn the_broker_reports_allowed_denied_timed_out_and_cancelled() {
        runtime().block_on(async {
            let reg = std::sync::Arc::new(ApprovalRegistry::new());
            let (sock, rec, server) = test_broker("reasons", 4, reg);
            let long = Duration::from_secs(10);

            // Allow.
            let client = {
                let sock = sock.clone();
                tokio::spawn(async move {
                    request_via_app_at(&sock, long, "T", "allow me", "computer", None, None).await
                })
            };
            let req = rec.wait_nth("request", 0).await;
            respond(req["id"].as_str().unwrap(), true).unwrap();
            assert!(client.await.unwrap().unwrap().is_ok());
            let done = rec.wait_nth("resolved", 0).await;
            assert_eq!(done["reason"], json!("allowed"));
            assert_eq!(done["approved"], json!(true));
            assert_eq!(done["cancelled"], json!(false));

            // Deny.
            let client = {
                let sock = sock.clone();
                tokio::spawn(async move {
                    request_via_app_at(&sock, long, "T", "deny me", "computer", None, None).await
                })
            };
            let req = rec.wait_nth("request", 1).await;
            respond(req["id"].as_str().unwrap(), false).unwrap();
            assert!(client
                .await
                .unwrap()
                .unwrap()
                .unwrap_err()
                .contains("denied"));
            let done = rec.wait_nth("resolved", 1).await;
            assert_eq!(done["reason"], json!("denied"));
            assert_eq!(done["approved"], json!(false));
            assert_eq!(done["cancelled"], json!(false));

            // The client's own timeout: its error text is unchanged.
            let timed = request_via_app_at(
                &sock,
                Duration::from_millis(200),
                "T",
                "wait",
                "computer",
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                timed.unwrap_err(),
                "the user did not approve (the approval request timed out)"
            );
            let done = rec.wait_nth("resolved", 2).await;
            assert_eq!(done["reason"], json!("timed_out"));
            assert_eq!(done["approved"], json!(false));
            assert_eq!(done["cancelled"], json!(true));

            // A client that disappears without a line: cancelled.
            {
                use tokio::io::AsyncWriteExt;
                let mut raw = tokio::net::UnixStream::connect(&sock).await.unwrap();
                let line = format!(
                    "{}\n",
                    json!({ "title": "T", "action": "vanish", "kind": "computer" })
                );
                raw.write_all(line.as_bytes()).await.unwrap();
                rec.wait_nth("request", 3).await;
                drop(raw);
            }
            let done = rec.wait_nth("resolved", 3).await;
            assert_eq!(done["reason"], json!("cancelled"));
            assert_eq!(done["approved"], json!(false));
            assert_eq!(done["cancelled"], json!(true));

            server.await.unwrap();
            let _ = std::fs::remove_file(&sock);
        });
    }

    /// Paperclip PR 21: a bound request names the tool call and conversation
    /// it belongs to; an unmatched one arrives with null ids after the wait.
    #[cfg(unix)]
    #[test]
    fn the_request_event_carries_the_bound_tool_use_id_and_session_key() {
        runtime().block_on(async {
            let reg = std::sync::Arc::new(ApprovalRegistry::new());
            reg.note_tool_use(
                "s-1",
                "toolu_1",
                "mcp__hollow_computer__quit_app",
                &json!({ "name": "X" }),
            );
            let (sock, rec, server) = test_broker("bind", 2, reg.clone());
            let long = Duration::from_secs(10);
            let args = json!({ "name": "X" });

            let bound = {
                let sock = sock.clone();
                let args = args.clone();
                tokio::spawn(async move {
                    request_via_app_at(
                        &sock,
                        long,
                        "Computer control",
                        "quit X",
                        "computer",
                        Some("quit_app"),
                        Some(&args),
                    )
                    .await
                })
            };
            let req = rec.wait_nth("request", 0).await;
            assert_eq!(req["toolUseId"], json!("toolu_1"));
            assert_eq!(req["sessionKey"], json!("s-1"));
            assert_eq!(req["tool"], json!("quit_app"));
            // Merged plan slice 3: the pending card pauses its own
            // conversation's watchdog before the user answers.
            assert_eq!(rec.wait_nth("hold", 0).await, json!("s-1"));
            respond(req["id"].as_str().unwrap(), true).unwrap();
            assert!(bound.await.unwrap().unwrap().is_ok());
            let done = rec.wait_nth("resolved", 0).await;
            assert_eq!(done["toolUseId"], json!("toolu_1"));
            assert_eq!(done["sessionKey"], json!("s-1"));
            assert_eq!(done["tool"], json!("quit_app"));

            // The same call again: already bound, so nothing to take.
            let started = tokio::time::Instant::now();
            let unbound = {
                let sock = sock.clone();
                tokio::spawn(async move {
                    request_via_app_at(
                        &sock,
                        long,
                        "Computer control",
                        "quit X",
                        "computer",
                        Some("quit_app"),
                        Some(&args),
                    )
                    .await
                })
            };
            let req = rec.wait_nth("request", 1).await;
            assert!(started.elapsed() >= BIND_WAIT, "{:?}", started.elapsed());
            assert_eq!(req["toolUseId"], Value::Null);
            assert_eq!(req["sessionKey"], Value::Null);
            assert_eq!(req["tool"], json!("quit_app"));
            // Unbound: every live turn is held (None).
            assert_eq!(rec.wait_nth("hold", 1).await, Value::Null);
            respond(req["id"].as_str().unwrap(), false).unwrap();
            assert!(unbound.await.unwrap().unwrap().is_err());

            server.await.unwrap();
            let _ = std::fs::remove_file(&sock);
        });
    }

    /// AG3: ids differ from launch to launch (the launch part is time and
    /// pid), never repeat inside one, and keep the `apr-` prefix.
    #[test]
    fn card_ids_are_launch_stable_and_unique() {
        let a = next_card_id();
        let b = next_card_id();
        assert_ne!(a, b);
        assert!(a.starts_with("apr-") && a.matches('-').count() == 2, "{a}");
        let launch = |id: &str| id.split('-').nth(1).unwrap().to_string();
        assert_eq!(launch(&a), launch(&b), "one launch part per process");
        assert!(
            launch(&a).len() > 8,
            "the launch part is more than a counter: {a}"
        );
    }

    /// AG3: a shown card carries its expiry (the client's own timeout) and
    /// its bound tool call.
    #[test]
    fn a_shown_card_has_an_expiry_after_it_was_shown() {
        let _held = WAIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let id = next_card_id();
        show_card(
            &json!({ "id": id, "title": "T", "action": "a", "kind": "computer",
            "toolUseId": "toolu-1" }),
        );
        let card = pending_cards().into_iter().find(|c| c.id == id).unwrap();
        close_card(&id);
        let at = chrono::DateTime::parse_from_rfc3339(&card.at).unwrap();
        let end = chrono::DateTime::parse_from_rfc3339(&card.expires_at).unwrap();
        assert_eq!((end - at).num_seconds(), CLIENT_TIMEOUT_SECS as i64);
        assert_eq!(card.tool_use_id.as_deref(), Some("toolu-1"));
    }

    #[test]
    fn a_typed_reason_is_cleaned_and_capped() {
        assert_eq!(
            clean_reason("  use\nstaging\t "),
            Some("use staging".into())
        );
        assert_eq!(clean_reason("   "), None);
        assert_eq!(
            clean_reason(&"x".repeat(900)).unwrap().chars().count(),
            REASON_MAX
        );
    }

    /// SA1: the words typed on a Decline reach the agent. Socket gate: the
    /// refusal the MCP tool returns carries them; an Allow never does.
    #[cfg(unix)]
    #[test]
    fn a_decline_reason_reaches_the_socket_client() {
        runtime().block_on(async {
            let reg = std::sync::Arc::new(ApprovalRegistry::new());
            let (sock, rec, server) = test_broker("decline-reason", 2, reg);
            let long = Duration::from_secs(10);
            let client = {
                let sock = sock.clone();
                tokio::spawn(async move {
                    request_via_app_at(&sock, long, "T", "send it", "computer", None, None).await
                })
            };
            let req = rec.wait_nth("request", 0).await;
            respond_with(
                req["id"].as_str().unwrap(),
                false,
                Some("Use the staging\nchannel"),
            )
            .unwrap();
            let err = client.await.unwrap().unwrap().unwrap_err();
            assert_eq!(err, "the user denied this action: Use the staging channel");

            let client = {
                let sock = sock.clone();
                tokio::spawn(async move {
                    request_via_app_at(&sock, long, "T", "send it", "computer", None, None).await
                })
            };
            let req = rec.wait_nth("request", 1).await;
            respond_with(req["id"].as_str().unwrap(), true, Some("ignored")).unwrap();
            assert!(client.await.unwrap().unwrap().is_ok());
            server.await.unwrap();
        });
    }

    /// SA1: the in-process gate (Hollow app cards) hands the typed reason to
    /// its caller after a Deny, and a later plain Deny carries none.
    #[cfg(unix)]
    #[test]
    fn a_decline_reason_reaches_the_in_process_caller() {
        let registry = ApprovalRegistry::new();
        let sink = std::sync::Arc::new(Recorded::default());
        let args = json!({});
        let card = InProcessCard {
            title: "Hollow",
            action: "Trash goal a",
            kind: "app",
            tool: None,
            args: &args,
        };
        for (reason, want) in [
            (Some("keep that goal"), Some("keep that goal")),
            (None, None),
        ] {
            let n = sink.requests.lock().unwrap().len();
            let answering = {
                let sink = sink.clone();
                std::thread::spawn(move || {
                    for _ in 0..250 {
                        let id = sink
                            .requests
                            .lock()
                            .unwrap()
                            .get(n)
                            .and_then(|r| r["id"].as_str().map(str::to_string));
                        if let Some(id) = id {
                            if respond_with(&id, false, reason).is_ok() {
                                return;
                            }
                        }
                        // sleep-ok: poll for the card this test's own ask shows
                        std::thread::sleep(Duration::from_millis(20));
                    }
                })
            };
            let verdict =
                ask_in_process(&*sink, &registry, card, Duration::from_secs(5), &|| false);
            answering.join().unwrap();
            assert_eq!(verdict, Verdict::Denied);
            assert_eq!(take_decline_reason().as_deref(), want);
            assert_eq!(take_decline_reason(), None, "taken once");
        }
    }

    /// A sink that knows the vault's subagents, like the app's.
    #[cfg(unix)]
    struct OriginSink {
        vault: std::path::PathBuf,
        requests: Mutex<Vec<Value>>,
    }

    #[cfg(unix)]
    impl ApprovalSink for OriginSink {
        fn subagent_origin(&self, session_key: &str) -> Option<SubagentOrigin> {
            subagent_origin_in(&self.vault, session_key)
        }
        fn coo_route(&self, session_key: &str) -> Option<CooRoute> {
            coo_route_in(&self.vault, session_key)
        }
        fn request(&self, payload: Value) {
            self.requests.lock().unwrap().push(payload);
        }
        fn resolved(&self, _payload: Value) {}
    }

    /// SA1 (planted): a card raised in a subagent's conversation carries the
    /// subagent and the conversation that started it, so the parent chat can
    /// show it; a card from an ordinary conversation names no subagent.
    #[cfg(unix)]
    #[test]
    fn a_subagents_card_names_it_and_its_parent() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let (parent, child, other) = (
            "s-20261117-aaaa0001",
            "s-20261117-aaaa0002",
            "s-20261117-aaaa0003",
        );
        for key in [parent, child, other] {
            crate::session_store::start_turn(v, key, "claude", &v.to_string_lossy(), "run-1", "hi")
                .unwrap();
        }
        assert!(crate::session_store::set_parent(
            v,
            child,
            parent,
            crate::session_store::FORK_SUBAGENT
        )
        .unwrap());
        let sink = OriginSink {
            vault: v.to_path_buf(),
            requests: Mutex::new(Vec::new()),
        };
        let registry = ApprovalRegistry::new();
        let args = json!({});
        let card = InProcessCard {
            title: "Hollow",
            action: "Trash goal a",
            kind: "app",
            tool: None,
            args: &args,
        };
        for (session, want_parent) in [(child, Some(parent)), (other, None)] {
            let n = sink.requests.lock().unwrap().len();
            let verdict = ask_in_process_for(
                &sink,
                &registry,
                card,
                Duration::from_secs(5),
                &|| sink.requests.lock().unwrap().len() > n,
                Some((session, "run-1")),
            );
            assert_eq!(verdict, Verdict::Cancelled);
            let shown = sink.requests.lock().unwrap()[n].clone();
            assert_eq!(shown["sessionKey"], json!(session));
            match want_parent {
                Some(p) => {
                    assert_eq!(shown["subagent"]["parentKey"], json!(p));
                    assert_eq!(shown["subagent"]["key"], json!(session));
                }
                None => assert_eq!(shown["subagent"], Value::Null),
            }
        }
    }

    /// 2026-11-20: a card from a project lead, or from a subagent anywhere
    /// below a lead or the COO, is raised to the COO chat (`routedTo`); the
    /// COO's own chat, an ordinary chat and an ordinary chat's subagent are
    /// not. One card id carries it (never a copy per chat), and a second ask
    /// raises a second card.
    #[cfg(unix)]
    #[test]
    fn a_leads_or_subagents_card_is_raised_to_the_coo_chat() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let main = crate::workspaces::ensure_main_agent(v).unwrap();
        let start = |key: &str, cwd: &std::path::Path| {
            crate::session_store::start_turn(
                v,
                key,
                "claude",
                &cwd.to_string_lossy(),
                "run-1",
                "hi",
            )
            .unwrap();
        };
        let (coo, lead, sub, grandsub) = (
            "s-20261120-c0000001",
            "s-project-hollow",
            "s-20261120-c0000002",
            "s-20261120-c0000003",
        );
        let (plain, plain_sub, coo_sub) = (
            "s-20261120-c0000004",
            "s-20261120-c0000005",
            "s-20261120-c0000006",
        );
        start(coo, &main);
        for key in [lead, sub, grandsub, plain, plain_sub] {
            start(key, v);
        }
        start(coo_sub, v);
        let sub_of = |child: &str, parent: &str| {
            assert!(crate::session_store::set_parent(
                v,
                child,
                parent,
                crate::session_store::FORK_SUBAGENT
            )
            .unwrap());
        };
        sub_of(sub, lead);
        sub_of(grandsub, sub);
        sub_of(plain_sub, plain);
        sub_of(coo_sub, coo);
        // Routed: the lead, its subagent, that subagent's subagent, and a
        // subagent of the COO itself; each names where it came from.
        for (key, kind) in [
            (lead, "lead"),
            (sub, "subagent"),
            (grandsub, "subagent"),
            (coo_sub, "subagent"),
        ] {
            let route = coo_route_in(v, key).unwrap_or_else(|| panic!("{key} is routed"));
            assert_eq!((route.asked_kind, route.asked_key.as_str()), (kind, key));
            assert_eq!(route.coo_key, coo, "the COO's current conversation");
        }
        // Not routed: the COO chat itself, an ordinary chat, its subagent
        // (SA1 keeps that in the parent), and an unknown key.
        for key in [coo, plain, plain_sub, "s-20261120-nonesuch"] {
            assert_eq!(coo_route_in(v, key), None, "{key}");
        }
        // The card payload carries it once, with the same id for every view.
        let sink = OriginSink {
            vault: v.to_path_buf(),
            requests: Mutex::new(Vec::new()),
        };
        let registry = ApprovalRegistry::new();
        let args = json!({});
        let card = InProcessCard {
            title: "Hollow",
            action: "Start a session",
            kind: "app",
            tool: None,
            args: &args,
        };
        for (session, routed) in [(grandsub, true), (plain_sub, false), (lead, true)] {
            let n = sink.requests.lock().unwrap().len();
            let verdict = ask_in_process_for(
                &sink,
                &registry,
                card,
                Duration::from_secs(5),
                &|| sink.requests.lock().unwrap().len() > n,
                Some((session, "run-1")),
            );
            assert_eq!(verdict, Verdict::Cancelled);
            let requests = sink.requests.lock().unwrap();
            assert_eq!(requests.len(), n + 1, "one request, one card");
            let shown = &requests[n];
            assert_eq!(shown["routedTo"].is_null(), !routed, "{session}");
            if routed {
                assert_eq!(shown["routedTo"]["askedKey"], json!(session));
                assert_eq!(shown["routedTo"]["cooKey"], json!(coo));
            }
        }
        // The pending list says so too, and a card leaves it on every outcome.
        show_card(&json!({ "id": "card-routed", "routedTo": { "askedKey": lead } }));
        show_card(&json!({ "id": "card-plain", "routedTo": null }));
        let listed = pending_cards();
        assert!(
            listed
                .iter()
                .find(|c| c.id == "card-routed")
                .unwrap()
                .routed_to_coo
        );
        assert!(
            !listed
                .iter()
                .find(|c| c.id == "card-plain")
                .unwrap()
                .routed_to_coo
        );
        close_card("card-routed");
        close_card("card-plain");
    }

    /// AG3: an interrupt answer drives the same channel, and a cancelled one
    /// can never approve.
    #[test]
    fn an_interrupt_answer_resolves_a_pending_card_deny_by_default() {
        use crate::interrupt::InterruptAnswer;
        for (answer, want) in [
            (InterruptAnswer::approval("apr-ia-1", true, None), true),
            (
                InterruptAnswer::approval("apr-ia-2", false, Some("no")),
                false,
            ),
            (InterruptAnswer::cancelled("apr-ia-3"), false),
        ] {
            let (tx, mut rx) = tokio::sync::oneshot::channel::<Answer>();
            pending()
                .lock()
                .unwrap()
                .insert(answer.interrupt_id.clone(), tx);
            respond_interrupt(&answer).unwrap();
            let got = rx.try_recv().unwrap();
            assert_eq!(got.approved, want);
            let want_reason = if answer.approved() {
                None
            } else {
                answer.decline_reason()
            };
            assert_eq!(got.reason, want_reason);
        }
    }

    // ── B4: answering from a phone and waiting for one ────────────────────

    /// Tests that set the process-wide phone wait hold this, and the one that
    /// reads a card's expiry too.
    static WAIT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn the_phone_wait_replaces_the_120_seconds_only_while_a_phone_has_approvals_on() {
        let _held = WAIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_phone_wait(None);
        assert_eq!(card_wait(), Duration::from_secs(120));
        assert_eq!(phone_wait(), None);
        set_phone_wait(Some(300));
        assert_eq!(card_wait(), Duration::from_secs(300));
        // Clamped to the settings' range.
        set_phone_wait(Some(5));
        assert_eq!(card_wait(), Duration::from_secs(120));
        set_phone_wait(Some(99_999));
        assert_eq!(card_wait(), Duration::from_secs(900));
        set_phone_wait(None);
        assert_eq!(card_wait(), Duration::from_secs(120));
        assert_eq!(wait_words(Duration::from_secs(120)), "two minutes");
        assert_eq!(wait_words(Duration::from_secs(300)), "5 minutes");
        assert_eq!(wait_words(Duration::from_secs(60)), "a minute");
        assert_eq!(wait_words(Duration::from_secs(90)), "90 seconds");
    }

    #[test]
    fn a_shown_card_expires_after_the_phone_wait() {
        let _held = WAIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_phone_wait(Some(300));
        show_card(&json!({"id": "apr-wait-1", "title": "T", "action": "A", "kind": "app"}));
        let card = pending_cards()
            .into_iter()
            .find(|c| c.id == "apr-wait-1")
            .unwrap();
        let (at, end) = (
            chrono::DateTime::parse_from_rfc3339(&card.at).unwrap(),
            chrono::DateTime::parse_from_rfc3339(&card.expires_at).unwrap(),
        );
        assert_eq!((end - at).num_seconds(), 300);
        close_card("apr-wait-1");
        set_phone_wait(None);
    }

    #[test]
    fn the_first_answer_wins_and_a_phone_answer_says_where_it_came_from() {
        let mut rx = test_register_pending("apr-first-1");
        let by = AnsweredBy::device("ph-0a1b2c3d", "Alex's iPhone", true, chrono::Utc::now());
        assert!(by
            .receipt
            .starts_with("Allowed on Alex's iPhone with Face ID at "));
        respond_from("apr-first-1", true, by.clone()).unwrap();
        // The window's click, a second phone: nothing left to answer.
        assert!(respond("apr-first-1", false).is_err());
        assert!(respond_from("apr-first-1", false, by).is_err());
        assert!(rx.try_recv().unwrap().approved);
        assert_eq!(
            test_take_answered_by("apr-first-1").unwrap().device_id,
            "ph-0a1b2c3d"
        );
        // A deny names no Face ID.
        let _rx = test_register_pending("apr-first-2");
        let deny = AnsweredBy::device("ph-0a1b2c3d", "iPhone", false, chrono::Utc::now());
        assert!(deny.receipt.starts_with("Denied on iPhone at "));
        assert!(!deny.face_id);
        respond_from("apr-first-2", false, deny).unwrap();
        test_take_answered_by("apr-first-2");
        // The window answering first leaves a phone with nothing.
        let _rx = test_register_pending("apr-first-3");
        respond("apr-first-3", true).unwrap();
        let late = AnsweredBy::device("ph-0a1b2c3d", "iPhone", true, chrono::Utc::now());
        assert!(respond_from("apr-first-3", true, late).is_err());
        assert!(test_take_answered_by("apr-first-3").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_client_that_asks_learns_the_wait_and_one_that_does_not_gets_no_such_line() {
        let _held = WAIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        runtime().block_on(async {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            set_phone_wait(Some(300));
            let reg = std::sync::Arc::new(ApprovalRegistry::new());
            let (sock, rec, server) = test_broker("waitline", 2, reg);
            // Asks: the first line is the wait, before any verdict.
            let mut asking = tokio::net::UnixStream::connect(&sock).await.unwrap();
            let line = format!(
                "{}\n",
                json!({"title": "T", "action": "a", "kind": "computer", "waitFromBroker": true})
            );
            asking.write_all(line.as_bytes()).await.unwrap();
            let (read, mut write) = asking.split();
            let mut lines = BufReader::new(read).lines();
            let first: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(first, json!({"waitSecs": 300}));
            let req = rec.wait_nth("request", 0).await;
            respond_from(
                req["id"].as_str().unwrap(),
                true,
                AnsweredBy::device("ph-0a1b2c3d", "iPhone", true, chrono::Utc::now()),
            )
            .unwrap();
            let verdict: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(verdict["approved"], json!(true));
            let done = rec.wait_nth("resolved", 0).await;
            assert_eq!(done["answeredBy"]["kind"], json!("device"));
            assert_eq!(done["answeredBy"]["deviceId"], json!("ph-0a1b2c3d"));
            let _ = write.shutdown().await;
            // Doesn't ask (an older client): its first line is the verdict itself.
            let mut old = tokio::net::UnixStream::connect(&sock).await.unwrap();
            let line = format!(
                "{}\n",
                json!({"title": "T", "action": "b", "kind": "computer"})
            );
            old.write_all(line.as_bytes()).await.unwrap();
            let req = rec.wait_nth("request", 1).await;
            respond(req["id"].as_str().unwrap(), false).unwrap();
            let (read, _w) = old.split();
            let mut lines = BufReader::new(read).lines();
            let first: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert!(first.get("waitSecs").is_none(), "{first}");
            assert_eq!(first["approved"], json!(false));
            let done = rec.wait_nth("resolved", 1).await;
            assert_eq!(done["answeredBy"], Value::Null);
            server.await.unwrap();
        });
        set_phone_wait(None);
    }

    /// A broker that answers one connection with `lines` after reading the
    /// request, pausing `gap` before each.
    #[cfg(unix)]
    async fn scripted_broker(tag: &str, lines: Vec<(Duration, Value)>) -> String {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let path =
            std::env::temp_dir().join(format!("hollow-apr-{tag}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.split();
            let mut request = BufReader::new(read).lines();
            let _ = request.next_line().await;
            for (gap, line) in lines {
                // sleep-ok: the scripted broker paces its lines to test a client's clock
                tokio::time::sleep(gap).await;
                let _ = write.write_all(format!("{line}\n").as_bytes()).await;
            }
            // Hold the connection open a moment so a client's timeout, not EOF, ends it.
            // sleep-ok: keeps the connection open so the client's timeout, not EOF, ends the wait
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    #[test]
    fn a_client_takes_the_brokers_wait_and_keeps_its_own_clock_without_one() {
        runtime().block_on(async {
            let short = Duration::from_millis(300);
            let gap = Duration::from_millis(900);
            // A wait line first stretches the clock, then the verdict arrives after the client's own would have run out.
            let sock = scripted_broker(
                "ext",
                vec![
                    (Duration::ZERO, json!({"waitSecs": 5})),
                    (gap, json!({"approved": true})),
                ],
            )
            .await;
            let got = request_via_app_at(&sock, short, "T", "a", "computer", None, None)
                .await
                .unwrap();
            assert!(got.is_ok(), "{got:?}");
            // No wait line: the same late verdict is too late.
            let sock = scripted_broker("noext", vec![(gap, json!({"approved": true}))]).await;
            let got = request_via_app_at(&sock, short, "T", "a", "computer", None, None)
                .await
                .unwrap();
            assert!(got.unwrap_err().contains("timed out"));
            // A first line that is already a verdict is the verdict, not a wait.
            let sock = scripted_broker(
                "verdict",
                vec![(Duration::ZERO, json!({"approved": true, "waitSecs": 60}))],
            )
            .await;
            let got = request_via_app_at(&sock, short, "T", "a", "computer", None, None)
                .await
                .unwrap();
            assert!(got.is_ok());
            let sock =
                scripted_broker("refused", vec![(Duration::ZERO, json!({"refused": "no"}))]).await;
            let got = request_via_app_at(&sock, short, "T", "a", "computer", None, None)
                .await
                .unwrap();
            assert_eq!(got.unwrap_err(), "no");
            // A wait line is read once, and only as the first line.
            let sock = scripted_broker(
                "twice",
                vec![
                    (Duration::ZERO, json!({"waitSecs": 5})),
                    (Duration::ZERO, json!({"waitSecs": 5})),
                ],
            )
            .await;
            let got = request_via_app_at(&sock, short, "T", "a", "computer", None, None)
                .await
                .unwrap();
            assert!(got.is_err());
        });
    }

    #[cfg(unix)]
    #[test]
    fn the_wait_line_is_clamped_and_never_a_verdict() {
        assert_eq!(wait_line(r#"{"waitSecs": 300}"#), Some(300));
        assert_eq!(wait_line(r#"{"waitSecs": 0}"#), Some(1));
        assert_eq!(wait_line(r#"{"waitSecs": 999999}"#), Some(3600));
        assert_eq!(wait_line(r#"{"waitSecs": "5"}"#), None);
        assert_eq!(wait_line(r#"{"approved": true, "waitSecs": 5}"#), None);
        assert_eq!(wait_line(r#"{"refused": "x", "waitSecs": 5}"#), None);
        assert_eq!(wait_line("not json"), None);
    }
}
