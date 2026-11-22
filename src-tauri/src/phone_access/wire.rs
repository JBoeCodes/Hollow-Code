//! The phone chat API's bodies (`docs/PHONE_CHAT_API.md`, fixtures in
//! `docs/phone-api-fixtures/`). Field names are camelCase and every
//! optional field is present as `null`, exactly as the fixtures show; the
//! tests compare these types' JSON with the fixtures.

use serde::{Deserialize, Serialize};

/// The API version every body that names one carries.
pub(crate) const API: u32 = 1;

/// AG8: the event kinds a `GET /v1/events` stream may carry, listed in its
/// `hello`. A peer Mac drops (and logs once) any kind not in its own copy.
pub(crate) const STREAM_EVENTS: [&str; 9] = [
    "hello",
    "live",
    "state",
    "append",
    "turn",
    "sessions",
    "ping",
    "resync",
    // B4: an approval card appeared or resolved; phones only (`ph-` ids).
    "approvals",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Health {
    pub ok: bool,
    pub app: &'static str,
    pub api: u32,
}

impl Health {
    pub(crate) fn now() -> Self {
        Self {
            ok: true,
            app: "hollow",
            api: API,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Me {
    pub device_id: String,
    pub device_name: String,
    pub mac_name: String,
    pub api: u32,
    /// v1.1: the remote runner Runs on can send chats to, `null` when none
    /// is set up.
    pub runner: Option<Runner>,
}

/// v1.1: the remote runner as `GET /v1/me` names it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Runner {
    /// The runner machine's name from Hollow's last check, else its SSH
    /// host.
    pub name: String,
    /// Its last check said it takes work.
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PairRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub device_model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PairResponse {
    pub token: String,
    pub device_id: String,
    pub device_name: String,
    pub mac_name: String,
    pub api: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct WorkspaceRef {
    pub id: String,
    pub title: String,
    pub kind: String,
}

/// A chat as the list shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    pub key: String,
    pub title: Option<String>,
    pub agent: String,
    pub workspace: Option<WorkspaceRef>,
    pub created_at: String,
    pub last_activity_at: String,
    pub turn_count: u32,
    pub state: String,
    pub needs_answer: bool,
    pub preview: Option<String>,
    /// v1.1: where the chat's next turn runs, `this-mac` or `mac-mini` (the
    /// runner's wire name, kept for the shipped iPhone app; M6);
    /// `None` when Hollow can't tell (its folder is gone).
    pub runs_on: Option<String>,
    /// M5: the chat moved from this Mac to the Mac with this machine id
    /// (`chat_homes` `away`): it lives there now. Set only on rows another
    /// Mac reads (`routes::mark_moved`); absent for a phone, whose contract
    /// (`docs/PHONE_CHAT_API.md`) has no such field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
    /// M5: the chat moved here from the Mac with this machine id
    /// (`chat_homes` `arrived`). Machine callers only, as `moved_to`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moved_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionPage {
    pub sessions: Vec<Session>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Tool {
    pub id: String,
    pub name: String,
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct QuestionOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Question {
    pub id: String,
    /// The question's header, else the card's title, else empty (never
    /// null, as the fixture shows it).
    pub header: String,
    pub prompt: String,
    pub mode: String,
    /// v1.1: a null answer to it is refused.
    pub required: bool,
    /// v1.1: a `single` or `multi` question also takes the person's own
    /// words.
    pub allows_other: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Questions {
    /// v1.1: the card's id (`qi-` and 16 hex), the answer route's
    /// `questionId`.
    pub id: String,
    /// `pending` or `closed`.
    pub state: String,
    pub items: Vec<Question>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Turn {
    pub id: String,
    pub ord: u32,
    pub role: String,
    pub text: String,
    pub agent: String,
    pub started_at: String,
    pub status: Option<String>,
    pub error: Option<String>,
    pub truncated: bool,
    pub tools: Vec<Tool>,
    pub questions: Option<Questions>,
    /// v1.2: the files a user turn carried, only when it carried any
    /// (`docs/phone-api/attachments.md`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

/// v1.2: one attached file (`docs/phone-api/attachments.md`, Shape).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Attachment {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub mime: String,
    pub size: u64,
    pub available: bool,
}

/// 201 for an upload.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct UploadResponse {
    pub attachment: Attachment,
}

/// The part of a running turn the transcript doesn't hold yet.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Live {
    pub key: String,
    pub ord: u32,
    pub gen: u32,
    pub started_at: String,
    pub text: String,
    pub truncated: bool,
    pub tools: Vec<Tool>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionDetail {
    pub session: Session,
    pub permission_mode: Option<String>,
    pub turns: Vec<Turn>,
    pub has_more: bool,
    pub live: Option<Live>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendRequest {
    #[serde(default)]
    pub controls: Option<super::controls::Selection>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub client_message_id: Option<String>,
    /// v1.2: ids from the upload answers (`attachments.md`).
    #[serde(default)]
    pub attachment_ids: Option<Vec<String>>,
}

/// 202 for a send. `held_id` is the contract's for a send held for after
/// the running turn (`state: "held"`); Hollow never holds a phone's send
/// (Deviations (server) in the contract), so it is never sent.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SendAccepted {
    pub accepted: bool,
    pub state: String,
    pub queue_id: i64,
    pub waiting_for: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub held_id: Option<String>,
    /// The chat this message went to, when the one it named had continued
    /// there (a handover, the Main agent's next conversation; one Main
    /// conversation, 2026-11-07): follow it for the reply. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarded_to: Option<String>,
}

/// M3a follow-up: `POST /v1/machine/coo`, the runner COO's first message
/// from a paired Mac (text only).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CooStartRequest {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub client_message_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewChatRequest {
    #[serde(default)]
    pub controls: Option<super::controls::Selection>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub client_message_id: Option<String>,
    /// v1.1: `this-mac`, or the runner as `runner` or `mac-mini` (M6, both
    /// accepted); absent or null keeps Hollow's own
    /// choice for the workspace.
    #[serde(default)]
    pub runs_on: Option<String>,
    /// v1.2: ids from the upload answers (`attachments.md`).
    #[serde(default)]
    pub attachment_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewChatAccepted {
    pub accepted: bool,
    pub key: String,
    pub state: String,
    pub queue_id: i64,
    pub waiting_for: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Stopped {
    pub stopped: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceRow {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    /// v1.1: a new chat here may run on the remote runner.
    pub can_run_on_mini: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Workspaces {
    pub workspaces: Vec<WorkspaceRow>,
}

/// v1.1: one chat waiting for the person (`GET /v1/waiting`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WaitingItem {
    pub session_key: String,
    pub question_id: String,
    pub title: Option<String>,
    pub workspace_title: Option<String>,
    /// `question`, or (B4) `approval`: then `questionId` is the card's id and
    /// `sessionKey` may be empty for a card no chat owns.
    pub kind: String,
    pub since: String,
    pub preview: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Waiting {
    pub items: Vec<WaitingItem>,
}

/// v1.1: a question card's answer from the phone. `answers` maps each
/// item's id to a string, a list of strings or null; Hollow checks each
/// against the card ([`super::routes::hollow_answer`]).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnswerRequest {
    #[serde(default)]
    pub client_message_id: Option<String>,
    #[serde(default)]
    pub answers: Option<serde_json::Map<String, serde_json::Value>>,
}

/// v1.1: `POST /v1/push`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct PushRequest {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<String>>,
}

/// v1.2: `POST /v1/live-activity` (`docs/phone-api/live-activity.md`, fixture
/// `live-activity/register-request.json`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiveActivityRequest {
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
}

/// v1.1: what Apple delivers to the phone (fixture `push-payload.json`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushPayload {
    pub aps: Aps,
    /// The chat a tap opens; absent on a test notification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    /// `turnFinished`, `question`, or `test` (Settings' test notification).
    pub kind: String,
    /// v1.2 (`docs/phone-api/notification-actions.md`): a question card the
    /// phone can answer with one tap, for the app's notification service
    /// extension to turn into buttons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card: Option<PushCard>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Aps {
    pub alert: Alert,
    pub sound: String,
    #[serde(rename = "thread-id", skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// v1.2: the notification category whose buttons the app registered
    /// (`hollow.turn`, `hollow.question`); absent when there are none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// v1.2: `1` on a question the app's extension may add buttons to.
    #[serde(rename = "mutable-content", skip_serializing_if = "Option::is_none")]
    pub mutable_content: Option<u8>,
}

/// v1.2: the one question of a card, as a push carries it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushCard {
    pub question_id: String,
    pub item_id: String,
    pub options: Vec<PushCardOption>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PushCardOption {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Alert {
    pub title: String,
    pub body: String,
}

/// `state` on the event stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StateEvent {
    pub key: String,
    pub state: String,
    pub needs_answer: bool,
}

/// `append` on the event stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct AppendEvent {
    pub key: String,
    pub ord: u32,
    pub gen: u32,
    pub text: String,
}

/// `turn` on the event stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct TurnEvent {
    pub key: String,
    pub ord: u32,
    pub status: String,
}

/// `sessions` on the event stream.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct SessionsEvent {
    pub keys: Vec<String>,
}

/// Every error body: `{"error": {"code", "message"}}`, with
/// `attemptsLeft` inside `error` for a wrong pairing code.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_left: Option<u32>,
}

// ── M2: another Mac (`docs/phone-api/machines.md`) ───────────────────────

/// The peer API's version: what another Mac may ask of this one. Separate
/// from [`API`], which the iPhone app reads.
pub(crate) const PEER_API: u32 = 1;

/// What another Mac may do here, in `GET /v1/machine` (a later Hollow adds
/// to it; a client shows only what it finds). A2 (2026-11-15) added
/// `usage`: `GET /v1/machine/usage`; M5 added
/// `moves` (the chat move routes) and, PR3, `appends` (an offline sync's
/// append bundle on those routes, `chat_offline`).
pub(crate) const CAPABILITIES: [&str; 8] = [
    "chats", "send", "stop", "cards", "events", "usage", "moves", "appends",
];

/// A2: the version of `GET /v1/machine/usage`'s shape. A client that finds a
/// larger one than it reads uses nothing from it.
pub(crate) const USAGE_API: u32 = 1;
/// A2: `GET /v1/machine/usage` lists at most this many accounts (8 added
/// Claude accounts, the Claude Code login and Codex fit with room).
pub(crate) const USAGE_ACCOUNTS_MAX: usize = 16;

/// A2: `GET /v1/machine/usage`, this Hollow's account snapshot, so another
/// Mac spreading runs over the same logins does not pick the account this
/// one just picked. Never a token, an email or an account id: an account is
/// named by its shared `key` (`peer_usage::shared_key`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineUsage {
    pub usage_api: u32,
    /// This Mac, as `GET /v1/machine` says.
    pub machine_id: String,
    /// When this snapshot was made (RFC 3339 UTC).
    pub observed_at: String,
    /// At most [`USAGE_ACCOUNTS_MAX`]; accounts with no shared key are left
    /// out (another Mac could never match them).
    pub accounts: Vec<AccountUsage>,
}

/// One account in [`MachineUsage`].
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountUsage {
    /// `ak-` and 16 hex digits, the same on every Mac for the same login.
    pub key: String,
    /// `claude` | `codex`.
    pub provider: &'static str,
    /// The account's name in Settings (at most 80 characters).
    pub label: String,
    /// Who pays for it, when known (the organisation), at most 80 characters.
    pub billing: Option<String>,
    /// Whether this Hollow's runs use it now (its routing would pick it, or
    /// it is the only one).
    pub routed: bool,
    /// Percent of the fuller live window; null when never measured.
    pub load_percent: Option<f64>,
    /// Out of plan allowance until `blockedUntil`.
    pub blocked: bool,
    pub blocked_until: Option<String>,
    /// The seven-day window's reset: the same instant on every Mac for one
    /// account, so a client can tell two accounts that share a key apart.
    pub seven_day_resets_at: Option<String>,
    /// When `loadPercent` was measured.
    pub measured_at: Option<String>,
    /// Runs this Hollow started on it in the last ten minutes.
    pub recent_picks: u32,
    pub last_picked_at: Option<String>,
}

/// `GET /v1/machine`: which Mac and Hollow this is.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineInfo {
    /// `m-` and 16 hex digits, from this Mac's `machine.json`.
    pub machine_id: String,
    /// The person's name for this Mac (`machine.json`'s `slug`), `null`
    /// until one is set.
    pub slug: Option<String>,
    /// The Mac's name as its owner sees it.
    pub name: String,
    pub app: &'static str,
    pub api: u32,
    pub peer_api: u32,
    pub version: String,
    pub commit: String,
    /// `hollow serve` (no window), rather than the app.
    pub headless: bool,
    /// [`CAPABILITIES`], plus `coo` when this Hollow runs the runner COO,
    /// or `coo-start` while that COO has no conversation yet (M3a,
    /// `runner_coo::capabilities`).
    pub capabilities: Vec<&'static str>,
    /// M3a: the runner COO's current chat key, with the `coo` capability;
    /// absent on a Mac that isn't the runner or whose runner COO has no
    /// conversation yet (`runner_coo::peer_coo`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coo: Option<String>,
}

/// `POST /v1/machines/pair`'s `machine`: the Mac that pairs.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineRef {
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachinePairRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub machine: Option<MachineRef>,
    #[serde(default)]
    pub client_version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachinePairResponse {
    pub token: String,
    /// `mc-` and 8 hex digits.
    pub device_id: String,
    /// This Mac, as `GET /v1/machine` says.
    pub machine: MachineInfo,
    pub api: u32,
    pub peer_api: u32,
}
