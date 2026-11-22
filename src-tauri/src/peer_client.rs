//! The HTTP client for another Mac's Hollow (M2 PR 2, the MacBook side of
//! the peer link; the server side and the routes are
//! `docs/phone-api/machines.md`).
//!
//! **Transport.** hyper's HTTP/1 client, one connection per request, over
//! [`Dial`]: the SSH tunnel's Unix socket in the app ([`Dial::Tunnel`], which
//! opens the tunnel first), or a loopback TCP port in tests. reqwest can't
//! dial a Unix socket. Every request sets `Host: 127.0.0.1:<remote port>`
//! itself (the other Mac's Host check compares with its own port, not the
//! socket this Mac dialed) and never an `X-Forwarded-*` or `Tailscale-*`
//! header (either would make it a refused, non-loopback call there).
//!
//! **Token.** `Authorization: Bearer <token>`, the token read from
//! `secret_store` (`peer:<machineId>`) when the client is made
//! ([`PeerClient::for_record`]). It never reaches a log, an error or Debug
//! output. A 401 saying `unauthorized` marks the Mac's record `tokenRevoked`
//! ([`PeerError::TokenRevoked`], the UI's "pairing needed"); any other answer
//! but a 401 touches its `lastSeenAt` and clears the mark.
//!
//! **Limits.** At most 8 requests at once per client, 10 seconds per request
//! (an event stream's head included) counted once the connection is open (a
//! tunnel's opening has its own per-alias bound), JSON bodies up to 4 MiB.
//! Text the other side controls goes through [`remote_text`].
//!
//! **Events** ([`follow`]): `GET /v1/events` read frame by frame. No bytes for
//! [`WATCHDOG`] (the server pings every 15 seconds) closes it; it reconnects
//! after up to 1, 2, 5, 10, then 30 seconds (jittered, PL1 `peer_link`) (starting over only after a stream
//! stayed up [`STABLE_AFTER`]) and, because the server never replays,
//! fetches again what it follows (the chat's detail, or the list) after every
//! reconnect. A followed chat's detail is also fetched again when its turn
//! ends or an `append` doesn't follow the `live` view held; after any such
//! refetch, appends are held until the stream is quiet ([`SETTLE_QUIET`]) and
//! the chat is fetched once more, so text queued before the snapshot isn't
//! shown twice. A dead token, a
//! Mac that needs an update or a chat that's gone ends it.
//!
//! **Pairing** ([`pair_over_ssh_with`]): `ssh <alias> hollow serve pair --machine
//! --json` (stdin closed, stdout capped at 4 KiB, 20 seconds) prints the other
//! Mac's code and identity; the tunnel opens, `POST /v1/machines/pair` trades
//! the code for a token, the token goes to `secret_store`, the record to
//! `peer_machines`, and `GET /v1/machine` with the new token is the check.
//! The code is never in argv or a log.

use crate::peer_machines::{MachineRecord, Machines};
use crate::peer_tunnel::{Tunnel, TunnelError};
use chrono::Utc;
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::Instant;

/// The peer protocol this client speaks (the server's `wire::PEER_API`).
pub(crate) const PEER_API: u32 = 1;
/// The phone API version an event stream's `hello` must carry.
const STREAM_API: u64 = 1;
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const BODY_CAP: usize = 4 * 1024 * 1024;
/// No bytes on an event stream for this long closes it.
pub(crate) const WATCHDOG: Duration = Duration::from_secs(40);
/// Reconnect waits, in seconds; the last repeats.
pub(crate) const RECONNECT: [u64; 5] = [1, 2, 5, 10, 30];
/// A chat list followed is fetched again at most this often.
pub(crate) const LIST_DEBOUNCE: Duration = Duration::from_secs(2);
/// After a chat is fetched again, `append` frames are held until the stream
/// has been quiet this long, then the chat is fetched once more.
pub(crate) const SETTLE_QUIET: Duration = Duration::from_millis(500);
/// ...or at most this long, while text keeps arriving.
pub(crate) const SETTLE_MAX: Duration = Duration::from_secs(2);
/// A stream up this long resets the reconnect waits.
pub(crate) const STABLE_AFTER: Duration = Duration::from_secs(30);
/// Text the other Mac (or ssh) controls is shown at most this long.
pub(crate) const REMOTE_TEXT_MAX: usize = 200;
const IN_FLIGHT: usize = 8;
/// `hollow serve pair --machine --json`'s stdout, at most.
const PAIR_STDOUT_CAP: usize = 4096;
const PAIR_TIMEOUT: Duration = Duration::from_secs(20);
/// What the other Mac runs to open its machine window: the serve binary and
/// data folder `docs/HEADLESS_SERVE.md` installs, relative to that account's
/// home (ssh starts there).
pub(crate) const DEFAULT_PAIR_COMMAND: &str =
    "hollow-serve/bin/hollow serve pair --machine --json --data-dir=\"$HOME/hollow-serve/data\"";

/// Text another Mac or its ssh controls (an error message, a name, a line of
/// stderr), made safe to show: control and direction-changing characters
/// become spaces, runs of spaces one, and it is cut at [`REMOTE_TEXT_MAX`]
/// characters.
pub(crate) fn remote_text(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| {
            let invisible = matches!(c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}');
            if c.is_control() || invisible {
                ' '
            } else {
                c
            }
        })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= REMOTE_TEXT_MAX {
        return clean;
    }
    let mut cut: String = clean.chars().take(REMOTE_TEXT_MAX - 1).collect();
    cut.push('\u{2026}');
    cut
}

// ── States and errors ────────────────────────────────────────────────────

/// What the UI shows for a Mac (`peer-machines-changed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PeerState {
    /// The strip's own row (the webview makes it from `thisMac`).
    #[allow(dead_code)]
    ThisMac,
    Online,
    Connecting,
    Offline,
    PairingNeeded,
    UpdateNeeded,
}

/// Why a call to another Mac failed. Every message is a plain sentence a
/// person can read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PeerError {
    /// The Mac couldn't be reached: the tunnel, ssh, its Hollow not
    /// answering, or no answer in time.
    Offline(String),
    /// It answered 401: this Mac's token is dead there. Pair again.
    TokenRevoked(String),
    /// No token saved for it here. Pair again.
    PairingNeeded(String),
    /// Its Hollow is older (no peer routes) or newer than this one speaks.
    UpdateNeeded(String),
    /// 403 `forbidden`: a route a Mac may not use, or not over this link.
    Forbidden(String),
    /// Pairing: no machine window open there.
    PairingClosed(String),
    /// Pairing: the code didn't match.
    WrongCode {
        message: String,
        attempts_left: Option<u32>,
    },
    NotFound(String),
    /// 409 `busy`: the chat is running or has a send queued.
    Busy(String),
    /// 409 `question_closed`: the card was answered or replaced there.
    QuestionClosed(String),
    BadRequest(String),
    TooLarge(String),
    /// 421: the Host header didn't name its port.
    Misdirected(String),
    /// 503 there.
    Unavailable(String),
    /// Something on this Mac: the record or the Keychain.
    Local(String),
    Internal(String),
}

impl PeerError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Offline(m)
            | Self::TokenRevoked(m)
            | Self::PairingNeeded(m)
            | Self::UpdateNeeded(m)
            | Self::Forbidden(m)
            | Self::PairingClosed(m)
            | Self::NotFound(m)
            | Self::Busy(m)
            | Self::QuestionClosed(m)
            | Self::BadRequest(m)
            | Self::TooLarge(m)
            | Self::Misdirected(m)
            | Self::Unavailable(m)
            | Self::Local(m)
            | Self::Internal(m) => m,
            Self::WrongCode { message, .. } => message,
        }
    }

    /// The Mac's state after this error.
    pub(crate) fn state(&self) -> PeerState {
        match self {
            Self::Offline(_) => PeerState::Offline,
            Self::TokenRevoked(_) | Self::PairingNeeded(_) => PeerState::PairingNeeded,
            Self::UpdateNeeded(_) => PeerState::UpdateNeeded,
            _ => PeerState::Online,
        }
    }

    /// Following events stops for good on this (no reconnect helps).
    pub(crate) fn is_final(&self) -> bool {
        matches!(
            self,
            Self::TokenRevoked(_)
                | Self::PairingNeeded(_)
                | Self::UpdateNeeded(_)
                | Self::Forbidden(_)
                | Self::NotFound(_)
                | Self::Misdirected(_)
                | Self::Local(_)
        )
    }
}

impl std::fmt::Display for PeerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

// ── What the other Mac sends ─────────────────────────────────────────────
// Mirrors of `phone_access::wire`'s shapes, read leniently: a field a later
// Hollow adds is ignored, one it drops reads as its default.

/// `GET /v1/machine`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineInfo {
    pub machine_id: String,
    #[serde(default)]
    pub slug: Option<String>,
    pub name: String,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub api: u32,
    #[serde(default)]
    pub peer_api: u32,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub headless: bool,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// M3a: the runner COO's chat key, beside the `coo` capability.
    #[serde(default)]
    pub coo: Option<String>,
}

impl MachineInfo {
    /// The runner COO's chat key, when this Mac says it runs one.
    pub(crate) fn runner_coo(&self) -> Option<&str> {
        let says = self
            .capabilities
            .iter()
            .any(|c| c == crate::runner_coo::CAPABILITY);
        self.coo.as_deref().filter(|key| says && !key.is_empty())
    }

    /// `0.5.17 (abc1234)`.
    pub(crate) fn version_line(&self) -> String {
        if self.commit.is_empty() {
            self.version.clone()
        } else {
            format!("{} ({})", self.version, self.commit)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub(crate) struct RemoteWorkspace {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub kind: String,
}

/// A chat row there (`wire::Session`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteSession {
    pub key: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub workspace: Option<RemoteWorkspace>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub last_activity_at: String,
    #[serde(default)]
    pub turn_count: u32,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub needs_answer: bool,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub runs_on: Option<String>,
    /// M5: it moved from that Mac to this machine id (it lives there now).
    #[serde(default)]
    pub moved_to: Option<String>,
    /// M5: it moved to that Mac from this machine id.
    #[serde(default)]
    pub moved_from: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionPage {
    pub sessions: Vec<RemoteSession>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// A running turn's live view there (`wire::Live`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteLive {
    pub key: String,
    pub ord: u32,
    pub gen: u32,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub truncated: bool,
    /// `wire::Tool`s, passed on as sent (`src/store/peerChat.ts` reads them).
    #[serde(default)]
    #[cfg_attr(test, ts(type = "Array<unknown>"))]
    pub tools: Vec<Value>,
}

/// `GET /v1/sessions/{key}`. Turns stay as the other Mac sent them (the
/// webview renders them; Rust only reads the live view's place).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionDetail {
    pub session: RemoteSession,
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// `wire::Turn`s as sent (`src/store/peerChat.ts` reads them).
    #[serde(default)]
    #[cfg_attr(test, ts(type = "Array<unknown>"))]
    pub turns: Vec<Value>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub live: Option<RemoteLive>,
}

/// A send or an answer taken there (`wire::SendAccepted`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct Accepted {
    pub accepted: bool,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub queue_id: i64,
    #[serde(default)]
    pub waiting_for: Option<String>,
    #[serde(default)]
    pub forwarded_to: Option<String>,
}

/// M5 PR2: a move route and its move id ([`PeerClient::move_call`]).
#[derive(Clone, Copy, Debug)]
pub(crate) enum MoveRoute<'a> {
    /// `POST .../parts/{n}` with `{"of", "data"}`.
    Part {
        move_id: &'a str,
        n: usize,
        body: &'a Value,
    },
    /// `POST .../commit` with `{"parts", "sha256"}`.
    Commit { move_id: &'a str, body: &'a Value },
    /// `GET /v1/machines/moves/{id}`.
    Status { move_id: &'a str },
    /// `DELETE /v1/machines/moves/{id}`.
    Cancel { move_id: &'a str },
}

impl MoveRoute<'_> {
    pub(crate) fn move_id(&self) -> &str {
        match self {
            Self::Part { move_id, .. }
            | Self::Commit { move_id, .. }
            | Self::Status { move_id }
            | Self::Cancel { move_id } => move_id,
        }
    }
}

/// What the other Mac answered to a move route, as received: the HTTP
/// status, the error's `code` when it sent one, a sentence (its message made
/// safe, [`remote_text`], or "<Mac> answered <status>."), and the body.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MoveAnswer {
    pub status: u16,
    pub code: Option<String>,
    pub message: String,
    pub body: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairAnswer {
    token: String,
    device_id: String,
    machine: MachineInfo,
    #[serde(default)]
    peer_api: u32,
}

/// One event-stream frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SseEvent {
    pub event: String,
    pub data: Value,
    /// AG5: the frame's `id:` line, absent from a Hollow older than that.
    pub id: Option<String>,
}

// ── The client ───────────────────────────────────────────────────────────

/// How a client reaches the other Mac's Hollow.
#[derive(Clone)]
pub(crate) enum Dial {
    /// The tunnel's socket, opening the tunnel first.
    Tunnel(Arc<Tunnel>),
    /// A loopback TCP port (tests: straight to a server).
    #[cfg_attr(not(test), allow(dead_code))]
    Tcp(u16),
}

/// The record a client keeps current: 401 marks it, answers touch it.
#[derive(Clone)]
struct Recorded {
    machines: Arc<Machines>,
    machine_id: String,
}

/// A client for one Mac.
pub(crate) struct PeerClient {
    dial: Dial,
    host: String,
    /// That Mac's name, for messages.
    name: String,
    token: Option<String>,
    record: Option<Recorded>,
    in_flight: Arc<Semaphore>,
    timeout: Duration,
}

impl std::fmt::Debug for PeerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerClient")
            .field("host", &self.host)
            .field("name", &self.name)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl PeerClient {
    /// A client without a token (pairing), for a Mac whose Hollow answers on
    /// `remote_port` there.
    pub(crate) fn new(dial: Dial, remote_port: u16, name: &str) -> Self {
        Self {
            dial,
            host: format!("127.0.0.1:{remote_port}"),
            name: match remote_text(name) {
                name if name.is_empty() => "That Mac".to_string(),
                name => name,
            },
            token: None,
            record: None,
            in_flight: Arc::new(Semaphore::new(IN_FLIGHT)),
            timeout: REQUEST_TIMEOUT,
        }
    }

    pub(crate) fn with_token(mut self, token: String) -> Self {
        self.token = Some(token);
        self
    }

    /// A shorter request timeout (tests).
    #[cfg(test)]
    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The client for a paired Mac: its token from `secret_store`; a 401
    /// marks `record` in `machines`, an answer touches it (and clears that
    /// mark: a record marked after one 401 still gets a client, so a good
    /// answer can show the mark was wrong).
    pub(crate) fn for_record(
        record: &MachineRecord,
        dial: Dial,
        machines: Arc<Machines>,
    ) -> Result<Self, PeerError> {
        let mut client = Self::new(dial, record.remote_port, &record.name);
        client.record = Some(Recorded {
            machines,
            machine_id: record.machine_id.clone(),
        });
        let token = crate::peer_machines::read_token(&record.machine_id)
            .map_err(PeerError::Local)?
            .ok_or_else(|| {
                PeerError::PairingNeeded(format!(
                    "This Mac has no key for {} saved. Pair it again in Settings.",
                    client.name
                ))
            })?;
        Ok(client.with_token(token))
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// The paired record's `machineId` this client calls (none for a
    /// pairing's bare client).
    pub(crate) fn machine_id(&self) -> Option<&str> {
        self.record.as_ref().map(|r| r.machine_id.as_str())
    }

    /// The Host header every request carries.
    #[cfg(test)]
    pub(crate) fn host_header(&self) -> &str {
        &self.host
    }

    fn revoked(&self) -> PeerError {
        PeerError::TokenRevoked(format!(
            "{} no longer accepts this Mac: it was unpaired there. Pair it again in Settings.",
            self.name
        ))
    }

    fn offline(&self, why: &str) -> PeerError {
        PeerError::Offline(format!("Can't reach {}: {why}.", self.name))
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        accept: &str,
    ) -> Result<hyper::Request<Full<Bytes>>, PeerError> {
        let mut req = hyper::Request::builder()
            .method(method)
            .uri(path)
            .header(hyper::header::HOST, &self.host)
            .header(hyper::header::ACCEPT, accept)
            .header(hyper::header::USER_AGENT, "Hollow-peer/1");
        if let Some(token) = &self.token {
            req = req.header(hyper::header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let bytes = match body {
            Some(body) => {
                req = req.header(hyper::header::CONTENT_TYPE, "application/json");
                Bytes::from(
                    serde_json::to_vec(body).map_err(|e| PeerError::Internal(e.to_string()))?,
                )
            }
            None => Bytes::new(),
        };
        req.body(Full::new(bytes))
            .map_err(|e| PeerError::Internal(format!("Hollow couldn't make that request ({e}).")))
    }

    /// A new connection to the other Mac. With a tunnel this opens it first,
    /// bounded by the tunnel itself (every alias gets its own readiness wait),
    /// never by a request's timeout: a request that timed out while the first
    /// alias hung would cancel the opening before the next alias was tried.
    async fn connect(&self) -> Result<Io, PeerError> {
        match &self.dial {
            Dial::Tcp(port) => tokio::net::TcpStream::connect(("127.0.0.1", *port))
                .await
                .map(Io::Tcp)
                .map_err(|e| self.offline(&format!("nothing answers on its port ({e})"))),
            Dial::Tunnel(tunnel) => {
                let path = tunnel
                    .ensure()
                    .await
                    .map_err(|e| PeerError::Offline(e.to_string()))?;
                self.connect_unix(&path).await
            }
        }
    }

    /// PL1: before a command's retry, close the tunnel the failed try used
    /// (opened no later than `since`), so the retry opens a fresh one instead
    /// of going over a link that went quiet.
    async fn close_link_from(&self, since: Instant) {
        if let Dial::Tunnel(tunnel) = &self.dial {
            if tunnel.close_if_opened_before(since).await {
                crate::launch::elog(&format!(
                    "[peer] closed the link to {} before sending again",
                    self.name
                ));
            }
        }
    }

    async fn connect_unix(&self, path: &Path) -> Result<Io, PeerError> {
        tokio::net::UnixStream::connect(path)
            .await
            .map(Io::Unix)
            .map_err(|e| self.offline(&format!("the link's socket refused ({e})")))
    }

    /// Send `req` over `io`; the response's head.
    async fn exchange(
        &self,
        io: Io,
        req: hyper::Request<Full<Bytes>>,
    ) -> Result<hyper::Response<Incoming>, PeerError> {
        match io {
            Io::Tcp(io) => self.over(io, req).await,
            Io::Unix(io) => self.over(io, req).await,
        }
    }

    async fn over<IO>(
        &self,
        io: IO,
        req: hyper::Request<Full<Bytes>>,
    ) -> Result<hyper::Response<Incoming>, PeerError>
    where
        IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
                .await
                .map_err(|e| self.offline(&format!("the link failed ({e})")))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        sender.send_request(req).await.map_err(|_| {
            self.offline("its Hollow didn't answer (it may not be running there, or its phone and peer link is off)")
        })
    }

    /// One JSON exchange: `(status, body)`, bounded in time and size.
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Bytes), PeerError> {
        let _permit = self
            .in_flight
            .acquire()
            .await
            .map_err(|_| PeerError::Internal("The client closed.".into()))?;
        let req = self.request(method, path, body, "application/json")?;
        let io = self.connect().await?;
        let work = async {
            let resp = self.exchange(io, req).await?;
            let status = resp.status().as_u16();
            let body = read_capped(resp.into_body(), BODY_CAP)
                .await
                .map_err(|e| match e {
                    Capped::TooLarge => PeerError::TooLarge(format!(
                        "{} sent more than Hollow reads at once.",
                        self.name
                    )),
                    Capped::Failed(why) => self.offline(&format!("the answer was cut off ({why})")),
                })?;
            Ok::<_, PeerError>((status, body))
        };
        let out = tokio::time::timeout(self.timeout, work)
            .await
            .map_err(|_| {
                self.offline(&format!(
                    "it didn't answer within {} seconds",
                    self.timeout.as_secs().max(1)
                ))
            })??;
        self.noted(out.0, &out.1).await;
        Ok(out)
    }

    /// Keep the record current, off the async threads: a 401 whose body says
    /// `unauthorized` marks it, any other answer but a 401 touches it.
    async fn noted(&self, status: u16, body: &[u8]) {
        let Some(record) = self.record.clone() else {
            return;
        };
        let revoked = status == 401
            && serde_json::from_slice::<Value>(body)
                .ok()
                .is_some_and(|v| v["error"]["code"] == "unauthorized");
        if status == 401 && !revoked {
            return;
        }
        let _ = tokio::task::spawn_blocking(move || {
            if revoked {
                if let Err(e) = record.machines.mark_token_revoked(&record.machine_id) {
                    crate::launch::elog(&format!("[peer] {e}"));
                }
            } else {
                record.machines.touch(&record.machine_id, None, Utc::now());
            }
        })
        .await;
    }

    /// The error an answer that isn't `ok` means.
    fn error_of(&self, status: u16, body: &[u8]) -> PeerError {
        let detail: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let detail = &detail["error"];
        let code = detail["code"].as_str().unwrap_or("");
        let message = detail["message"]
            .as_str()
            .map(remote_text)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| format!("{} answered {status}.", self.name));
        match (status, code) {
            (401, _) | (_, "unauthorized") => self.revoked(),
            (421, _) | (_, "misdirected") => PeerError::Misdirected(format!(
                "{} refused the request's address (its Hollow answers on another port than this Mac has saved).",
                self.name
            )),
            (_, "forbidden") => PeerError::Forbidden(message),
            (_, "pairing_closed") => PeerError::PairingClosed(message),
            (_, "wrong_code") => PeerError::WrongCode {
                message,
                attempts_left: detail["attemptsLeft"].as_u64().map(|n| n as u32),
            },
            (_, "not_found") => PeerError::NotFound(message),
            (_, "busy") => PeerError::Busy(message),
            (_, "question_closed") => PeerError::QuestionClosed(message),
            (_, "bad_request") => PeerError::BadRequest(message),
            (_, "too_large") => PeerError::TooLarge(message),
            (503, _) | (_, "unavailable") => PeerError::Unavailable(message),
            _ => PeerError::Internal(message),
        }
    }

    /// A JSON call whose `ok` statuses parse as `T`.
    async fn json<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        ok: &[u16],
    ) -> Result<T, PeerError> {
        let (status, bytes) = self.call(method, path, body).await?;
        if !ok.contains(&status) {
            return Err(self.error_of(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|e| {
            PeerError::Internal(format!(
                "{} answered in a shape this Hollow doesn't read ({e}). Update Hollow on both Macs.",
                self.name
            ))
        })
    }

    /// Any route, as JSON (tests and the route-scope check).
    #[cfg(test)]
    pub(crate) async fn call_json(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, PeerError> {
        self.json(method, path, body, &[200, 201, 202]).await
    }

    /// `GET /v1/machine`: which Mac answers, checked against this client's
    /// peer API.
    pub(crate) async fn machine(&self) -> Result<MachineInfo, PeerError> {
        let (status, bytes) = self.call("GET", "/v1/machine", None).await?;
        if status == 404 {
            return Err(PeerError::UpdateNeeded(format!(
                "{}'s Hollow is older than the peer link. Update Hollow there.",
                self.name
            )));
        }
        if status != 200 {
            return Err(self.error_of(status, &bytes));
        }
        let info: MachineInfo = serde_json::from_slice(&bytes).map_err(|e| {
            PeerError::Internal(format!(
                "{} sent an identity Hollow can't read ({e}).",
                self.name
            ))
        })?;
        check_peer_api(&self.name, info.peer_api)?;
        if let Some(record) = self.record.clone() {
            if info.machine_id != record.machine_id {
                return Err(PeerError::PairingNeeded(format!(
                    "{} answers as another Mac now ({}). Pair it again in Settings.",
                    self.name,
                    remote_text(&info.name)
                )));
            }
            let version = info.version_line();
            let coo = info.runner_coo().map(str::to_string);
            let capabilities = info.capabilities.clone();
            let announced_slug = info.slug.clone();
            let _ = tokio::task::spawn_blocking(move || {
                record
                    .machines
                    .touch(&record.machine_id, Some(&version), Utc::now());
                // M3a: whether this Mac's COO is a dispatcher.
                record
                    .machines
                    .set_coo(&record.machine_id, coo.as_deref(), Utc::now());
                // M5 UI: whether it takes moved chats (`moves`).
                record
                    .machines
                    .set_capabilities(&record.machine_id, &capabilities);
                // Its own announcement is the truth about its name: a rename
                // updates the record, keeps the old slug as an alias and
                // re-stamps owners that named it.
                if let Some(old) = record.machines.refresh_slug(
                    &record.machine_id,
                    announced_slug.as_deref(),
                    crate::context_roots::machine_slug().as_deref(),
                    Utc::now(),
                ) {
                    if let Some(vault) = crate::chat_move_trigger::current_vault() {
                        crate::runs_on::after_peer_rename(&vault, &old, &record.machine_id);
                    }
                }
            })
            .await;
        }
        Ok(info)
    }

    /// A2: `GET /v1/machine/usage`, the other Mac's account snapshot
    /// (call only when its `capabilities` list `usage`). `None` when it
    /// speaks a newer usage shape than this Hollow reads; at most
    /// [`crate::peer_usage::ACCOUNTS_MAX`] accounts are kept.
    pub(crate) async fn usage(&self) -> Result<Option<crate::peer_usage::PeerUsage>, PeerError> {
        let mut usage: crate::peer_usage::PeerUsage =
            self.json("GET", "/v1/machine/usage", None, &[200]).await?;
        if usage.usage_api == 0 || usage.usage_api > crate::peer_usage::USAGE_API {
            return Ok(None);
        }
        usage.accounts.truncate(crate::peer_usage::ACCOUNTS_MAX);
        Ok(Some(usage))
    }

    /// `GET /v1/sessions`: a page of its chats, newest first.
    pub(crate) async fn sessions(
        &self,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<SessionPage, PeerError> {
        let mut path = format!("/v1/sessions?limit={}", limit.clamp(1, 200));
        if let Some(cursor) = cursor.filter(|c| !c.is_empty()) {
            path.push_str("&cursor=");
            path.push_str(&encode(cursor));
        }
        self.json("GET", &path, None, &[200]).await
    }

    /// `GET /v1/sessions/{key}`: its newest `limit` turns below `before`.
    pub(crate) async fn detail(
        &self,
        key: &str,
        before: Option<u32>,
        limit: u32,
    ) -> Result<SessionDetail, PeerError> {
        let mut path = format!(
            "/v1/sessions/{}?limit={}",
            segment(key, "chat")?,
            limit.clamp(1, 100)
        );
        if let Some(before) = before {
            path.push_str(&format!("&before={before}"));
        }
        self.json("GET", &path, None, &[200]).await
    }

    /// `POST /v1/sessions/{key}/messages`: text only. `client_message_id`
    /// makes a retry safe, so a link that drops before the answer gets the
    /// same send once more (PL1, `peer_link::retry_command`).
    pub(crate) async fn send(
        &self,
        key: &str,
        text: &str,
        client_message_id: &str,
    ) -> Result<Accepted, PeerError> {
        let path = format!("/v1/sessions/{}/messages", segment(key, "chat")?);
        let body = json!({"text": text, "clientMessageId": client_message_id});
        crate::peer_link::retry_command(
            "a send",
            || self.json("POST", &path, Some(&body), &[200, 202]),
            |since| self.close_link_from(since),
        )
        .await
    }

    /// M3a follow-up: `POST /v1/machine/coo`, start the runner COO's first
    /// conversation there with `text` (only while it has none: `busy`
    /// otherwise). The new conversation's key.
    pub(crate) async fn start_coo(
        &self,
        text: &str,
        client_message_id: &str,
    ) -> Result<String, PeerError> {
        let body = json!({"text": text, "clientMessageId": client_message_id});
        let started: Value = self
            .json("POST", "/v1/machine/coo", Some(&body), &[200, 202])
            .await?;
        started["key"]
            .as_str()
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                PeerError::Internal(format!(
                    "{} started its COO but named no conversation.",
                    self.name
                ))
            })
    }

    /// `POST /v1/sessions/{key}/stop`: whether a running turn was stopped.
    pub(crate) async fn stop(&self, key: &str) -> Result<bool, PeerError> {
        let path = format!("/v1/sessions/{}/stop", segment(key, "chat")?);
        let body: Value = self.json("POST", &path, None, &[200]).await?;
        Ok(body["stopped"].as_bool().unwrap_or(false))
    }

    /// `POST /v1/sessions/{key}/questions/{id}/answer`: answer a card at the
    /// chat's home. `answers` maps each item id to a string, a list of them,
    /// or null, as the contract says.
    pub(crate) async fn answer(
        &self,
        key: &str,
        question_id: &str,
        client_message_id: &str,
        answers: &Map<String, Value>,
    ) -> Result<Accepted, PeerError> {
        let path = format!(
            "/v1/sessions/{}/questions/{}/answer",
            segment(key, "chat")?,
            segment(question_id, "card")?
        );
        let body = json!({"clientMessageId": client_message_id, "answers": answers});
        // Saved by id there too: a dropped link gets it once more (PL1).
        crate::peer_link::retry_command(
            "a card answer",
            || self.json("POST", &path, Some(&body), &[200, 202]),
            |since| self.close_link_from(since),
        )
        .await
    }

    /// M5 PR2: one of the move routes (`docs/phone-api/machines.md`, Moving
    /// a chat here) for move `route.move_id()`. Any answer comes back as it
    /// was received ([`MoveAnswer`]: its status, error code and body), so
    /// the move driver (`chat_move_send`) can tell a refusal from no answer;
    /// a transport failure, a timeout, a cut-off body or a 401 is an `Err`,
    /// never an answer.
    pub(crate) async fn move_call(&self, route: &MoveRoute<'_>) -> Result<MoveAnswer, PeerError> {
        let id = segment(route.move_id(), "move")?;
        let (method, path, body) = match route {
            MoveRoute::Part { n, body, .. } => (
                "POST",
                format!("/v1/machines/moves/{id}/parts/{n}"),
                Some(*body),
            ),
            MoveRoute::Commit { body, .. } => (
                "POST",
                format!("/v1/machines/moves/{id}/commit"),
                Some(*body),
            ),
            MoveRoute::Status { .. } => ("GET", format!("/v1/machines/moves/{id}"), None),
            MoveRoute::Cancel { .. } => ("DELETE", format!("/v1/machines/moves/{id}"), None),
        };
        let (status, bytes) = self.call(method, &path, body).await?;
        if status == 401 {
            return Err(self.error_of(status, &bytes));
        }
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let code = body["error"]["code"].as_str().map(str::to_string);
        let message = body["error"]["message"]
            .as_str()
            .map(remote_text)
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| format!("{} answered {status}.", self.name));
        Ok(MoveAnswer {
            status,
            code,
            message,
            body,
        })
    }

    /// `DELETE /v1/machines/me`: unpair this Mac there.
    pub(crate) async fn unpair_me(&self) -> Result<(), PeerError> {
        let (status, bytes) = self.call("DELETE", "/v1/machines/me", None).await?;
        match status {
            200..=299 => Ok(()),
            _ => Err(self.error_of(status, &bytes)),
        }
    }

    /// `POST /v1/machines/pair`: trade a code from that Mac's machine window
    /// for a token. Needs no token.
    async fn pair(&self, code: &str, me: &ThisMac) -> Result<PairAnswer, PeerError> {
        let body = json!({
            "code": code,
            "machine": {"machineId": me.machine_id, "slug": me.slug, "name": me.name},
            "clientVersion": me.version,
        });
        self.json("POST", "/v1/machines/pair", Some(&body), &[200])
            .await
    }

    /// `GET /v1/events` (`session` follows one chat's live view too): the
    /// open stream, its silence bounded by `watchdog`.
    pub(crate) async fn events(
        &self,
        session: Option<&str>,
        watchdog: Duration,
        last_event_id: Option<&str>,
    ) -> Result<EventStream, PeerError> {
        let path = match session {
            Some(key) => format!("/v1/events?session={}", segment(key, "chat")?),
            None => "/v1/events".to_string(),
        };
        let mut req = self.request("GET", &path, None, "text/event-stream")?;
        // AG5: resume where the last stream left off. Only an id of the shape
        // the server gives out (hex digits and dashes) is sent.
        if let Some(id) = last_event_id.filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        }) {
            if let Ok(value) = hyper::header::HeaderValue::from_str(id) {
                req.headers_mut().insert("last-event-id", value);
            }
        }
        let io = self.connect().await?;
        let resp = tokio::time::timeout(self.timeout, self.exchange(io, req))
            .await
            .map_err(|_| self.offline("its event stream didn't open in time"))??;
        let status = resp.status().as_u16();
        if status != 200 {
            let body = tokio::time::timeout(self.timeout, read_capped(resp.into_body(), BODY_CAP))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            self.noted(status, &body).await;
            return Err(self.error_of(status, &body));
        }
        self.noted(status, b"").await;
        Ok(EventStream {
            body: resp.into_body(),
            buf: Vec::new(),
            watchdog,
            name: self.name.clone(),
            tunnel: match &self.dial {
                Dial::Tunnel(tunnel) => Some(tunnel.clone()),
                _ => None,
            },
        })
    }
}

/// AG8: what a stream's `hello` says. Read leniently: a field this Hollow
/// doesn't know is ignored, so a newer server can add to it.
#[derive(Debug, PartialEq)]
pub(crate) struct StreamHello {
    /// The other Mac's `"<version> <commit>"`, absent from a Hollow older
    /// than AG8 (2026-11-17).
    pub build: Option<String>,
    /// AG5: whether the server replayed from the `Last-Event-ID` sent; absent
    /// when none was sent or the server is older than that.
    pub resumed: Option<bool>,
}

/// AG8: a stream's `hello`, checked against [`STREAM_API`]. A different
/// version is a clear update notice, naming which side is behind; a hello
/// that doesn't carry an integer `api` is fatal, never guessed at.
pub(crate) fn read_hello(name: &str, data: &Value) -> Result<StreamHello, PeerError> {
    let Some(api) = data
        .as_object()
        .and_then(|o| o.get("api"))
        .and_then(Value::as_u64)
    else {
        return Err(PeerError::UpdateNeeded(format!(
            "{name} began its stream with a hello this Hollow can't read. Update Hollow on both Macs."
        )));
    };
    let build = data["build"].as_str().map(remote_text);
    let theirs = match &build {
        Some(build) => format!("{api}, build {build}"),
        None => api.to_string(),
    };
    if api > STREAM_API {
        return Err(PeerError::UpdateNeeded(format!(
            "{name}'s Hollow streams a newer API ({theirs}) than this one ({STREAM_API}). Update Hollow on this Mac."
        )));
    }
    if api < STREAM_API {
        return Err(PeerError::UpdateNeeded(format!(
            "{name}'s Hollow streams an older API ({theirs}) than this one ({STREAM_API}). Update Hollow on {name}."
        )));
    }
    Ok(StreamHello {
        build,
        resumed: data["resumed"].as_bool(),
    })
}

/// AG8: distinct unknown event kinds one stream remembers (and warns about).
pub(crate) const DROPPED_KINDS_MAX: usize = 32;

/// AG8: the event kinds a stream sent that this Hollow doesn't read
/// ([`STREAM_EVENTS`](crate::phone_access::STREAM_EVENTS)).
/// They are dropped, not shown and not fatal, so a newer peer's additions
/// can't break the link; the log gets one warning per kind per stream.
#[derive(Debug, Default)]
pub(crate) struct DroppedEvents {
    kinds: std::collections::BTreeMap<String, u32>,
}

impl DroppedEvents {
    /// `None` for a kind this Hollow reads. Otherwise the event is dropped:
    /// `Some(warning)` the first time the kind is seen, `Some("")` after
    /// (and for any kind past [`DROPPED_KINDS_MAX`]).
    pub(crate) fn note(&mut self, name: &str, build: Option<&str>, kind: &str) -> Option<String> {
        if crate::phone_access::STREAM_EVENTS.contains(&kind) {
            return None;
        }
        if let Some(count) = self.kinds.get_mut(kind) {
            *count = count.saturating_add(1);
            return Some(String::new());
        }
        if self.kinds.len() >= DROPPED_KINDS_MAX {
            return Some(String::new());
        }
        self.kinds.insert(kind.to_string(), 1);
        let from = match build {
            Some(build) => format!("{name} (build {build})"),
            None => name.to_string(),
        };
        Some(format!(
            "[peer] dropping `{}` events from {from}: this Hollow doesn't know that kind. Update Hollow on this Mac to see them.",
            remote_text(kind)
        ))
    }

    /// Each dropped kind and how many of it (tests).
    #[cfg(test)]
    pub(crate) fn counts(&self) -> Vec<(String, u32)> {
        self.kinds.iter().map(|(k, n)| (k.clone(), *n)).collect()
    }
}

fn check_peer_api(name: &str, theirs: u32) -> Result<(), PeerError> {
    if theirs > PEER_API {
        return Err(PeerError::UpdateNeeded(format!(
            "{name}'s Hollow speaks a newer peer link ({theirs}) than this one ({PEER_API}). Update Hollow on this Mac."
        )));
    }
    Ok(())
}

/// A path segment: a chat key or card id as Hollow gives them out.
fn segment<'a>(value: &'a str, what: &str) -> Result<&'a str, PeerError> {
    let ok = !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && value != "."
        && value != "..";
    if ok {
        Ok(value)
    } else {
        Err(PeerError::BadRequest(format!(
            "That {what} id isn't one Hollow gives out."
        )))
    }
}

/// A query value, percent-encoded.
fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A connection, before the request.
enum Io {
    Tcp(tokio::net::TcpStream),
    Unix(tokio::net::UnixStream),
}

enum Capped {
    TooLarge,
    Failed(String),
}

async fn read_capped(mut body: Incoming, cap: usize) -> Result<Bytes, Capped> {
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| Capped::Failed(e.to_string()))?;
        if let Ok(data) = frame.into_data() {
            if out.len() + data.len() > cap {
                return Err(Capped::TooLarge);
            }
            out.extend_from_slice(&data);
        }
    }
    Ok(Bytes::from(out))
}

// ── Event streams ────────────────────────────────────────────────────────

/// An open `GET /v1/events`, read a frame at a time.
pub(crate) struct EventStream {
    body: Incoming,
    buf: Vec<u8>,
    watchdog: Duration,
    name: String,
    /// The tunnel it reads through, kept from looking idle.
    tunnel: Option<Arc<Tunnel>>,
}

impl EventStream {
    /// The next frame; `None` when the other Mac closed the stream. No bytes
    /// within the watchdog is an error ([`PeerError::Offline`]).
    pub(crate) async fn next(&mut self) -> Result<Option<SseEvent>, PeerError> {
        loop {
            if let Some(event) = take_frame(&mut self.buf) {
                return Ok(Some(event));
            }
            if self.buf.len() > BODY_CAP {
                return Err(PeerError::TooLarge(format!(
                    "{} sent an event larger than Hollow reads.",
                    self.name
                )));
            }
            let frame = tokio::time::timeout(self.watchdog, self.body.frame())
                .await
                .map_err(|_| {
                    PeerError::Offline(format!(
                        "No word from {} for {} seconds; reconnecting.",
                        self.name,
                        self.watchdog.as_secs()
                    ))
                })?;
            match frame {
                None => return Ok(None),
                Some(Err(e)) => {
                    return Err(PeerError::Offline(format!(
                        "The link to {} dropped ({e}).",
                        self.name
                    )))
                }
                Some(Ok(frame)) => {
                    if let Some(tunnel) = &self.tunnel {
                        tunnel.touch();
                    }
                    if let Ok(data) = frame.into_data() {
                        self.buf.extend_from_slice(&data);
                    }
                }
            }
        }
    }
}

/// One complete `event:`/`data:` frame off the front of `buf`.
fn take_frame(buf: &mut Vec<u8>) -> Option<SseEvent> {
    loop {
        // A blank line ends a frame (`\n\n`, or `\r\n\r\n`).
        let end = buf.windows(2).enumerate().find_map(|(i, w)| match w {
            b"\n\n" => Some(i + 2),
            b"\n\r" if buf.get(i + 2) == Some(&b'\n') => Some(i + 3),
            _ => None,
        })?;
        let raw: Vec<u8> = buf.drain(..end).collect();
        let text = String::from_utf8_lossy(&raw).replace('\r', "");
        let mut event = None;
        let mut id = None;
        let mut data: Vec<&str> = Vec::new();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("id:") {
                id = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("event:") {
                event = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("data:") {
                data.push(v.strip_prefix(' ').unwrap_or(v));
            }
        }
        if event.is_none() && data.is_empty() {
            continue; // a comment or a blank keep-alive
        }
        let data = data.join("\n");
        return Some(SseEvent {
            event: event.unwrap_or_else(|| "message".into()),
            data: serde_json::from_str(&data).unwrap_or(Value::String(data)),
            id,
        });
    }
}

/// What [`follow`] reports.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FollowEvent {
    /// The stream is open (`reconnected` after a drop).
    Connected { reconnected: bool },
    /// A frame for the UI: `live`, `append`, `state`, `turn`, `sessions`.
    Event(SseEvent),
    /// The followed chat, fetched again (after a reconnect, a turn's end, or
    /// an `append` that didn't follow the live view held).
    Detail(Box<SessionDetail>),
    /// The list, fetched again (a list stream: after a reconnect or a
    /// `sessions` frame, at most once per [`LIST_DEBOUNCE`]).
    Sessions(SessionPage),
    /// The stream dropped; the next try is in `retry_in`.
    Disconnected { message: String, retry_in: Duration },
    /// Following stopped for good.
    Ended(PeerError),
}

/// Where [`follow`] sends what happens.
pub(crate) type FollowSink = Arc<dyn Fn(FollowEvent) + Send + Sync>;

#[derive(Clone, Debug)]
pub(crate) struct FollowOptions {
    pub watchdog: Duration,
    pub reconnect: Vec<Duration>,
    pub list_debounce: Duration,
    /// A stream up this long resets the reconnect waits.
    pub stable_after: Duration,
    pub settle_quiet: Duration,
    pub settle_max: Duration,
    /// Draws each reconnect wait from its step (PL1).
    pub jitter: crate::peer_link::Jitter,
}

impl Default for FollowOptions {
    fn default() -> Self {
        Self {
            watchdog: WATCHDOG,
            reconnect: RECONNECT.iter().map(|s| Duration::from_secs(*s)).collect(),
            list_debounce: LIST_DEBOUNCE,
            stable_after: STABLE_AFTER,
            settle_quiet: SETTLE_QUIET,
            settle_max: SETTLE_MAX,
            jitter: crate::peer_link::Jitter::random(),
        }
    }
}

/// A running [`follow`]; dropping it stops following.
pub(crate) struct Follow {
    task: tokio::task::JoinHandle<()>,
}

impl Follow {
    pub(crate) fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for Follow {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Follow a Mac's event stream: one chat's (`session`) or the list's.
pub(crate) fn follow(
    client: Arc<PeerClient>,
    session: Option<String>,
    options: FollowOptions,
    sink: FollowSink,
) -> Follow {
    Follow {
        task: tokio::spawn(run_follow(client, session, options, sink)),
    }
}

async fn run_follow(
    client: Arc<PeerClient>,
    session: Option<String>,
    options: FollowOptions,
    sink: FollowSink,
) {
    let mut attempt = 0usize;
    let mut reconnected = false;
    let mut cursor = Cursor::default();
    loop {
        let mut connected_at = None;
        // AG5: ask to resume, unless a refetch was still settling when the
        // last stream ended (what it held back is not in the id).
        let resume = if cursor.pending || cursor.lost {
            None
        } else {
            cursor.last_id.clone()
        };
        cursor.pending = false;
        cursor.lost = false;
        let result = match client
            .events(session.as_deref(), options.watchdog, resume.as_deref())
            .await
        {
            Ok(mut stream) => {
                follow_stream(
                    &client,
                    &mut stream,
                    session.as_deref(),
                    &options,
                    &sink,
                    reconnected,
                    resume.is_some(),
                    &mut cursor,
                    &mut connected_at,
                )
                .await
            }
            Err(e) => Err(e),
        };
        // Only a stream that stayed up a while starts the waits over: one that
        // drops right after `hello` keeps backing off.
        if connected_at.is_some_and(|at: Instant| at.elapsed() >= options.stable_after) {
            attempt = 0;
        }
        let error = match result {
            Ok(()) => PeerError::Offline(format!("{} closed the event stream.", client.name())),
            Err(e) if e.is_final() => {
                sink(FollowEvent::Ended(e));
                return;
            }
            Err(e) => e,
        };
        let waits = &options.reconnect;
        let retry_in = options.jitter.wait(
            waits
                .get(attempt.min(waits.len().saturating_sub(1)))
                .copied()
                .unwrap_or(Duration::from_secs(1)),
        );
        attempt += 1;
        sink(FollowEvent::Disconnected {
            message: error.message().to_string(),
            retry_in,
        });
        tokio::time::sleep(retry_in).await;
        reconnected = true;
    }
}

/// AG5: what a follow carries from one stream to the next so a reconnect can
/// resume.
#[derive(Default)]
struct Cursor {
    /// The newest `id:` read.
    last_id: Option<String>,
    /// The `(ord, gen)` of the live view held.
    held: Option<(u32, u32)>,
    /// A refetch was settling, or a list refetch was due, when the stream
    /// ended: the next one refetches instead of resuming.
    pending: bool,
    /// A refetch failed: text it would have shown is missing, so nothing
    /// later on this stream may let the next connection resume past it.
    lost: bool,
}

/// After a chat is fetched again: `append` frames the server queued before
/// that snapshot would add text it already holds, so appends are held until
/// the stream is quiet ([`FollowOptions::settle_quiet`], or at most
/// `settle_max` while text keeps coming), then the chat is fetched once more.
struct Settle {
    quiet_until: Instant,
    give_up_at: Instant,
    held_any: bool,
}

impl Settle {
    fn new(options: &FollowOptions) -> Self {
        let now = Instant::now();
        Self {
            quiet_until: now + options.settle_quiet,
            give_up_at: now + options.settle_max,
            held_any: false,
        }
    }

    fn due(&self) -> Instant {
        if self.held_any {
            self.quiet_until.min(self.give_up_at)
        } else {
            self.quiet_until
        }
    }
}

async fn follow_stream(
    client: &PeerClient,
    stream: &mut EventStream,
    session: Option<&str>,
    options: &FollowOptions,
    sink: &FollowSink,
    reconnected: bool,
    asked_to_resume: bool,
    cursor: &mut Cursor,
    connected_at: &mut Option<Instant>,
) -> Result<(), PeerError> {
    let hello = match stream.next().await? {
        None => return Ok(()),
        Some(hello) if hello.event == "hello" => read_hello(client.name(), &hello.data)?,
        Some(other) => {
            return Err(PeerError::Internal(format!(
                "{} began its stream with `{}`, not hello.",
                client.name(),
                remote_text(&other.event)
            )))
        }
    };
    // AG8: kinds a newer peer sends that this Hollow doesn't read.
    let mut dropped = DroppedEvents::default();
    *connected_at = Some(Instant::now());
    sink(FollowEvent::Connected { reconnected });
    // What changed while the stream was down is read again, unless the
    // server replayed it (AG5: `resumed`, after a `Last-Event-ID`).
    let resumed = asked_to_resume && hello.resumed == Some(true);
    if !resumed {
        cursor.held = None;
        cursor.last_id = None;
    }
    let mut settle: Option<Settle> = None;
    let mut last_list = Instant::now();
    if reconnected && !resumed {
        match session {
            Some(key) => {
                cursor.held = refetch_detail(client, key, sink, cursor).await?;
                settle = Some(Settle::new(options));
            }
            None => refetch_list(client, sink, cursor).await?,
        }
    }
    let mut list_due: Option<Instant> = None;
    loop {
        cursor.pending = settle.is_some() || list_due.is_some();
        let wake = [list_due, settle.as_ref().map(Settle::due)]
            .into_iter()
            .flatten()
            .min();
        let next = match wake {
            Some(at) => tokio::select! {
                next = stream.next() => next?,
                _ = tokio::time::sleep_until(at) => {
                    let now = Instant::now();
                    if list_due.is_some_and(|due| due <= now) {
                        list_due = None;
                        last_list = now;
                        refetch_list(client, sink, cursor).await?;
                    }
                    if let (Some(due), Some(key)) = (settle.as_ref().map(Settle::due), session) {
                        if due <= now {
                            let held_any = settle.take().is_some_and(|s| s.held_any);
                            if held_any {
                                cursor.held = refetch_detail(client, key, sink, cursor).await?;
                                settle = Some(Settle::new(options));
                            }
                        }
                    }
                    continue;
                }
            },
            None => stream.next().await?,
        };
        let Some(event) = next else {
            return Ok(());
        };
        if let (Some(id), false) = (&event.id, cursor.lost) {
            cursor.last_id = Some(id.clone());
        }
        match event.event.as_str() {
            "ping" | "hello" => {}
            // AG5: the server couldn't resume (its `hello` said so too): the
            // refetch above already covers it.
            "resync" => {}
            "live" => {
                // A whole view: what follows on this stream continues from it.
                settle = None;
                cursor.held = live_place(&event.data);
                sink(FollowEvent::Event(event));
            }
            "append" => {
                if let Some(key) = session {
                    if let Some(settling) = settle.as_mut() {
                        settling.held_any = true;
                        settling.quiet_until = Instant::now() + options.settle_quiet;
                        continue;
                    }
                    let place = live_place(&event.data);
                    if place.is_none() || place != cursor.held {
                        cursor.held = refetch_detail(client, key, sink, cursor).await?;
                        settle = Some(Settle::new(options));
                        continue;
                    }
                }
                sink(FollowEvent::Event(event));
            }
            "turn" => {
                sink(FollowEvent::Event(event));
                cursor.held = None;
                if let Some(key) = session {
                    cursor.held = refetch_detail(client, key, sink, cursor).await?;
                    settle = Some(Settle::new(options));
                }
            }
            "sessions" => {
                sink(FollowEvent::Event(event));
                if session.is_none() && list_due.is_none() {
                    let at = last_list + options.list_debounce;
                    if at <= Instant::now() {
                        last_list = Instant::now();
                        refetch_list(client, sink, cursor).await?;
                    } else {
                        list_due = Some(at);
                    }
                }
            }
            "state" => sink(FollowEvent::Event(event)),
            kind => {
                // AG8: dropped, never shown and never fatal; logged once
                // per kind, not put in the chat.
                match dropped.note(client.name(), hello.build.as_deref(), kind) {
                    Some(warning) if !warning.is_empty() => crate::launch::elog(&warning),
                    Some(_) => {}
                    None => sink(FollowEvent::Event(event)),
                }
            }
        }
    }
}

fn live_place(data: &Value) -> Option<(u32, u32)> {
    Some((data["ord"].as_u64()? as u32, data["gen"].as_u64()? as u32))
}

/// Fetch the chat again; a failure that isn't final is left for the next
/// event (the stream is still up).
async fn refetch_detail(
    client: &PeerClient,
    key: &str,
    sink: &FollowSink,
    cursor: &mut Cursor,
) -> Result<Option<(u32, u32)>, PeerError> {
    match client.detail(key, None, 20).await {
        Ok(detail) => {
            let held = detail.live.as_ref().map(|l| (l.ord, l.gen));
            sink(FollowEvent::Detail(Box::new(detail)));
            Ok(held)
        }
        Err(e) if e.is_final() => Err(e),
        Err(e) => {
            crate::launch::elog(&format!("[peer] refetching a chat: {e}"));
            // AG5: what this refetch would have shown is missing, so no
            // later reconnect may resume past it.
            cursor.last_id = None;
            cursor.lost = true;
            Ok(None)
        }
    }
}

async fn refetch_list(
    client: &PeerClient,
    sink: &FollowSink,
    cursor: &mut Cursor,
) -> Result<(), PeerError> {
    match client.sessions(None, 200).await {
        Ok(page) => {
            sink(FollowEvent::Sessions(page));
            Ok(())
        }
        Err(e) if e.is_final() => Err(e),
        Err(e) => {
            crate::launch::elog(&format!("[peer] refetching the chat list: {e}"));
            cursor.last_id = None;
            cursor.lost = true;
            Ok(())
        }
    }
}

// ── Pairing and forgetting ───────────────────────────────────────────────

/// This Mac, as it introduces itself when it pairs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ThisMac {
    pub machine_id: String,
    pub slug: Option<String>,
    pub name: String,
    /// `0.5.17 (abc1234)`.
    pub version: String,
}

impl ThisMac {
    /// From `machine.json` (its `machineId` written once if missing), the
    /// Mac's name and this build.
    pub(crate) fn read() -> Result<Self, String> {
        let build = crate::app_menu::build_identity();
        Ok(Self {
            machine_id: crate::context_roots::ensure_machine_id()?,
            slug: crate::context_roots::machine_slug(),
            name: crate::phone_access::mac_name(),
            version: format!("{} ({})", build.version, build.short_commit),
        })
    }
}

/// `hollow serve pair --machine --json`'s line. Its Debug and Display never
/// show the code.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PairCode {
    pub code: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub machine: bool,
    #[serde(default)]
    pub machine_id: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl std::fmt::Debug for PairCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairCode")
            .field("code", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .field("machine", &self.machine)
            .field("machine_id", &self.machine_id)
            .field("slug", &self.slug)
            .field("name", &self.name)
            .finish()
    }
}

impl std::fmt::Display for PairCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the code.
        write!(
            f,
            "a pairing window on {}",
            remote_text(self.name.as_deref().unwrap_or("that Mac"))
        )
    }
}

/// The first line of `stdout` that is a machine window's answer.
pub(crate) fn parse_pair_code(alias: &str, stdout: &str) -> Result<PairCode, PeerError> {
    let found = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<PairCode>(l.trim()).ok())
        .next()
        .ok_or_else(|| {
            PeerError::Unavailable(format!(
                "{alias} didn't print a pairing code. Check that Hollow runs there (hollow serve)."
            ))
        })?;
    let id_ok = found
        .machine_id
        .as_deref()
        .is_some_and(crate::phone_access::valid_machine_id);
    if !found.machine || !id_ok || found.code.trim().is_empty() {
        return Err(PeerError::UpdateNeeded(format!(
            "{alias}'s Hollow opened no window for another Mac. Update Hollow there."
        )));
    }
    Ok(found)
}

/// Run `command` on `alias` over ssh (the hardened options, stdin closed,
/// stdout capped, bounded in time) and read its machine window's code.
pub(crate) async fn read_pair_code(
    ssh: &OsString,
    alias: &str,
    command: &str,
) -> Result<PairCode, PeerError> {
    use tokio::io::AsyncReadExt as _;
    if !crate::remote_session::valid_host(alias) {
        return Err(PeerError::BadRequest(crate::peer_machines::host_refusal(
            alias,
        )));
    }
    // Exit 255 must mean ssh itself failed (as `remote_session` does).
    let wrapped = format!("{command}; s=$?; [ \"$s\" -ne 255 ] || s=3; exit $s");
    let mut cmd = tokio::process::Command::new(ssh);
    cmd.args(crate::remote_session::SSH_OPTS)
        .args(["-T", "--", alias, &wrapped])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    crate::process_group::isolate(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| PeerError::Local(format!("ssh couldn't start on this Mac ({e}).")))?;
    let _guard = crate::process_group::GroupGuard::adopt(&cmd, &child);
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let work = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let read_out = async {
            if let Some(s) = stdout {
                let _ = s
                    .take(PAIR_STDOUT_CAP as u64 + 1)
                    .read_to_end(&mut out)
                    .await;
            }
        };
        let read_err = async {
            if let Some(s) = stderr {
                let _ = s.take(2048).read_to_end(&mut err).await;
            }
        };
        tokio::join!(read_out, read_err);
        let status = child.wait().await;
        (status, out, err)
    };
    let (status, out, err) = tokio::time::timeout(PAIR_TIMEOUT, work)
        .await
        .map_err(|_| {
            PeerError::Offline(format!(
                "{alias} didn't give a pairing code within {} seconds.",
                PAIR_TIMEOUT.as_secs()
            ))
        })?;
    if out.len() > PAIR_STDOUT_CAP {
        return Err(PeerError::Unavailable(format!(
            "{alias} printed more than a pairing code. Check the command Hollow runs there."
        )));
    }
    let err = String::from_utf8_lossy(&err).trim().to_string();
    let last = remote_text(err.lines().last().unwrap_or(""));
    let last = last.trim_end_matches('.');
    match status.ok().and_then(|s| s.code()) {
        Some(0) => parse_pair_code(alias, &String::from_utf8_lossy(&out)),
        Some(255) => Err(PeerError::Offline(
            TunnelError::Unreachable {
                alias: alias.to_string(),
                ssh_error: err,
            }
            .to_string(),
        )),
        Some(127) => Err(PeerError::Unavailable(format!(
            "Hollow isn't where this Mac looked on {alias} (hollow-serve/bin/hollow). Install hollow serve there first."
        ))),
        _ if last.is_empty() => Err(PeerError::Unavailable(format!(
            "{alias} couldn't open a pairing window."
        ))),
        _ => Err(PeerError::Unavailable(format!(
            "{alias} couldn't open a pairing window: {last}."
        ))),
    }
}

/// Trade `code` for a token through `client` (no token yet), save the token
/// and the record, then check it with `GET /v1/machine`. `hosts` and
/// `remote_port` are how this Mac reaches it.
pub(crate) async fn pair_with_code(
    machines: &Arc<Machines>,
    client: PeerClient,
    code: &PairCode,
    me: &ThisMac,
    hosts: Vec<String>,
    remote_port: u16,
) -> Result<(MachineRecord, PeerClient), PeerError> {
    let expected = code.machine_id.clone().unwrap_or_default();
    let known = machines.list().map_err(PeerError::Local)?;
    if !known.iter().any(|m| m.machine_id == expected)
        && known.len() >= crate::peer_machines::MACHINES_MAX
    {
        return Err(PeerError::Local(format!(
            "Hollow pairs with at most {} other Macs. Forget one first.",
            crate::peer_machines::MACHINES_MAX
        )));
    }
    let answer = client.pair(&code.code, me).await?;
    let paired = PeerClient::new(client.dial.clone(), remote_port, &answer.machine.name)
        .with_token(answer.token.clone());
    if answer.machine.machine_id != expected {
        // Not the Mac that gave the code: give the token back unused. A
        // record this Mac already keeps for the expected one is left alone.
        let _ = paired.unpair_me().await;
        return Err(PeerError::Internal(format!(
            "The Mac that answered ({}) isn't the one that gave the code ({}). Nothing was paired.",
            remote_text(&answer.machine.name),
            remote_text(code.name.as_deref().unwrap_or("another Mac"))
        )));
    }
    match finish_pairing(machines, &client.dial, &answer, hosts, remote_port).await {
        Ok(done) => Ok(done),
        Err(e) => {
            // Paired there but not usable here: unpair there (best effort)
            // and keep nothing here, record or token. The same Mac pairing
            // again had its old device replaced there already, so its old
            // record was dead anyway.
            let _ = paired.unpair_me().await;
            let (machines, id) = (machines.clone(), expected);
            let forgot = tokio::task::spawn_blocking(move || machines.forget(&id)).await;
            if let Ok(Err(why)) = forgot {
                crate::launch::elog(&format!("[peer] after a failed pairing: {why}"));
            }
            Err(e)
        }
    }
}

/// Everything after the other Mac handed out a token: the version check, the
/// token saved, the record written, and `GET /v1/machine` with the new token.
async fn finish_pairing(
    machines: &Arc<Machines>,
    dial: &Dial,
    answer: &PairAnswer,
    hosts: Vec<String>,
    remote_port: u16,
) -> Result<(MachineRecord, PeerClient), PeerError> {
    let info = &answer.machine;
    check_peer_api(&info.name, answer.peer_api.max(info.peer_api))?;
    crate::peer_machines::save_token(&info.machine_id, &answer.token).map_err(PeerError::Local)?;
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let record = MachineRecord {
        machine_id: info.machine_id.clone(),
        slug: info.slug.clone(),
        name: info.name.clone(),
        ssh_hosts: hosts,
        remote_port,
        device_id: answer.device_id.clone(),
        paired_at: now.clone(),
        last_seen_at: Some(now.clone()),
        last_version: Some(info.version_line()),
        peer_api: info.peer_api.max(1),
        token_revoked: false,
        coo: info.runner_coo().map(str::to_string),
        coo_seen_at: Some(now.clone()),
        coo_since: info.runner_coo().map(|_| now.clone()),
        old_slugs: Vec::new(),
        capabilities: info.capabilities.clone(),
        extra: Map::new(),
    };
    {
        let (machines, record) = (machines.clone(), record.clone());
        tokio::task::spawn_blocking(move || machines.upsert(record))
            .await
            .map_err(|e| PeerError::Local(e.to_string()))?
            .map_err(PeerError::Local)?;
    }
    let client = PeerClient::for_record(&record, dial.clone(), machines.clone())?;
    // Checks the id too: another Mac answering now is `PairingNeeded`.
    client.machine().await?;
    Ok((record, client))
}

/// What pairing over ssh needs (the app's in `commands::peer_api`).
pub(crate) struct PairSetup {
    /// The ssh program.
    pub ssh: OsString,
    /// The alias the code is read through; the tunnel tries it first.
    pub alias: String,
    /// Further aliases for the tunnel, in order.
    pub more_hosts: Vec<String>,
    /// What runs there ([`DEFAULT_PAIR_COMMAND`]).
    pub command: String,
    pub remote_port: u16,
    pub machines: Arc<Machines>,
    pub me: ThisMac,
}

/// A paired Mac: its record, its open tunnel and a client with its token.
pub(crate) struct Paired {
    pub record: MachineRecord,
    /// The tunnel it talked through (the app's hub already holds it).
    #[cfg_attr(not(test), allow(dead_code))]
    pub tunnel: Arc<Tunnel>,
    pub client: PeerClient,
}

/// The tunnel a pairing talks through, once the code named the Mac:
/// `(machineId, aliases in order, remote port)`. The app's hands out the one
/// tunnel it keeps for that Mac, so a Mac never has two.
pub(crate) type TunnelFor<'a> =
    Box<dyn FnOnce(&str, Vec<String>, u16) -> Result<Arc<Tunnel>, PeerError> + Send + 'a>;

/// The aliases a pairing through `alias` keeps: `alias`, then its `-lan`
/// alias when `~/.ssh/config` names one (`ssh -G`, off the async threads).
pub(crate) async fn hosts_after(ssh: &OsString, alias: &str) -> Vec<String> {
    let (ssh, first) = (ssh.to_string_lossy().into_owned(), alias.to_string());
    tokio::task::spawn_blocking(move || {
        crate::peer_machines::hosts_for(&first, |a| crate::peer_machines::resolved_host(&ssh, a))
    })
    .await
    .unwrap_or_default()
    .into_iter()
    .skip(1)
    .collect()
}

/// What Cancel says to the pairing it stopped.
pub(crate) const PAIR_CANCELLED: &str = "Pairing was cancelled. Nothing was paired.";

/// A pairing attempt's Cancel: set once, seen by the attempt at its next
/// step (the ssh code read stops at once, a trade already sent is undone).
#[derive(Default)]
pub(crate) struct PairCancel {
    flag: std::sync::atomic::AtomicBool,
    wake: tokio::sync::Notify,
}

impl PairCancel {
    pub(crate) fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Resolves once [`PairCancel::cancel`] ran (also when it already had).
    pub(crate) async fn cancelled(&self) {
        loop {
            let woken = self.wake.notified();
            tokio::pin!(woken);
            // Registered before the flag is read: a cancel in between wakes it.
            woken.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            woken.await;
        }
    }
}

/// Pair with the Mac at `setup.alias` over ssh: its code from
/// `setup.command`, then the tunnel `tunnel_for` gives for the Mac it named.
#[cfg(test)]
pub(crate) async fn pair_over_ssh_with(
    setup: PairSetup,
    tunnel_for: TunnelFor<'_>,
) -> Result<Paired, PeerError> {
    pair_over_ssh_cancellable(setup, tunnel_for, None).await
}

/// [`pair_over_ssh_with`] that `cancel` can stop. Nothing is kept for a
/// cancelled pairing: the code read (ssh is killed with its future) and the
/// steps before the trade stop at once; a trade that already went through is
/// unpaired there and forgotten here (record and token), as a failed one is.
pub(crate) async fn pair_over_ssh_cancellable(
    setup: PairSetup,
    tunnel_for: TunnelFor<'_>,
    cancel: Option<Arc<PairCancel>>,
) -> Result<Paired, PeerError> {
    let stop = || PeerError::Local(PAIR_CANCELLED.to_string());
    let cancelled = |c: &Option<Arc<PairCancel>>| c.as_ref().is_some_and(|c| c.is_cancelled());
    let read = read_pair_code(&setup.ssh, &setup.alias, &setup.command);
    let code = match &cancel {
        Some(c) => tokio::select! {
            code = read => code?,
            () = c.cancelled() => return Err(stop()),
        },
        None => read.await?,
    };
    if cancelled(&cancel) {
        return Err(stop());
    }
    let machine_id = code.machine_id.clone().unwrap_or_default();
    // A Mac already paired here: its pairing is replaced by the trade (the
    // old token is dead there), so a Cancel that comes too late must leave
    // the new one standing, not forget the Mac.
    let existed = setup
        .machines
        .list()
        .map(|l| l.iter().any(|m| m.machine_id == machine_id))
        .unwrap_or(false);
    let mut hosts = vec![setup.alias.clone()];
    hosts.extend(setup.more_hosts.into_iter().filter(|h| *h != setup.alias));
    let tunnel = tunnel_for(&machine_id, hosts.clone(), setup.remote_port)?;
    let name = code.name.clone().unwrap_or_else(|| setup.alias.clone());
    let bare = PeerClient::new(Dial::Tunnel(tunnel.clone()), setup.remote_port, &name);
    let traded = pair_with_code(
        &setup.machines,
        bare,
        &code,
        &setup.me,
        hosts,
        setup.remote_port,
    )
    .await;
    // Cancel is honoured up to here. One that lands after this check lets
    // the pairing complete: it is then a normal pairing, and Forget undoes it.
    if cancelled(&cancel) && !(existed && traded.is_ok()) {
        // Undo a trade that got through: no half-written peer. (Not for a Mac
        // that was already paired: see `existed`.)
        if let Ok((record, client)) = traded {
            let _ = client.unpair_me().await;
            let (machines, id) = (setup.machines.clone(), record.machine_id);
            let forgot = tokio::task::spawn_blocking(move || machines.forget(&id)).await;
            if let Ok(Err(why)) = forgot {
                crate::launch::elog(&format!("[peer] after a cancelled pairing: {why}"));
            }
        }
        return Err(stop());
    }
    let (record, client) = traded?;
    Ok(Paired {
        record,
        tunnel,
        client,
    })
}

/// Forget a Mac: unpair this Mac there first when `client` reaches it
/// (best effort), then drop the record and its token here. `Ok(false)` when
/// it wasn't paired.
pub(crate) async fn forget_machine(
    machines: &Machines,
    machine_id: &str,
    client: Option<&PeerClient>,
) -> Result<bool, PeerError> {
    if let Some(client) = client {
        if let Err(e) = client.unpair_me().await {
            crate::launch::elog(&format!("[peer] unpairing there first: {e}"));
        }
    }
    machines.forget(machine_id).map_err(PeerError::Local)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_parse_across_reads_and_skip_comments() {
        let mut buf =
            b": keep-alive\n\nevent: hello\ndata: {\"api\":1}\n\nevent: ping\nda".to_vec();
        assert_eq!(
            take_frame(&mut buf),
            Some(SseEvent {
                event: "hello".into(),
                data: json!({"api": 1}),
                id: None,
            })
        );
        assert_eq!(take_frame(&mut buf), None);
        buf.extend_from_slice(b"ta: {}\r\n\r\n");
        assert_eq!(take_frame(&mut buf).unwrap().event, "ping");
        assert!(buf.is_empty());
    }

    #[test]
    fn errors_name_the_ui_state_and_which_end_following() {
        let offline = PeerError::Offline("x".into());
        assert_eq!(offline.state(), PeerState::Offline);
        assert!(!offline.is_final());
        assert_eq!(
            PeerError::TokenRevoked("x".into()).state(),
            PeerState::PairingNeeded
        );
        assert_eq!(
            PeerError::PairingNeeded("x".into()).state(),
            PeerState::PairingNeeded
        );
        assert_eq!(
            PeerError::UpdateNeeded("x".into()).state(),
            PeerState::UpdateNeeded
        );
        assert_eq!(PeerError::Busy("x".into()).state(), PeerState::Online);
        assert!(PeerError::TokenRevoked("x".into()).is_final());
        assert_eq!(
            serde_json::to_value([PeerState::ThisMac, PeerState::PairingNeeded]).unwrap(),
            json!(["this-mac", "pairing-needed"])
        );
    }

    #[test]
    fn segments_and_queries_never_carry_a_path_or_a_raw_byte() {
        assert!(segment("s-20261115-ffffffff", "chat").is_ok());
        for bad in ["", "..", "a/b", "a?b", "a b", "%2e"] {
            assert!(segment(bad, "chat").is_err(), "{bad}");
        }
        assert_eq!(encode("a+b/c=="), "a%2Bb%2Fc%3D%3D");
    }

    #[test]
    fn a_stream_hello_reads_its_build_and_names_a_version_mismatch_both_ways() {
        // AG8: a hello from a newer build with fields this one doesn't know
        // is read; the extra fields are ignored.
        let hello = read_hello(
            "Mac mini",
            &json!({"api": 1, "build": "9.9.9 abc1234", "events": ["hello", "x"], "future": {"a": 1}}),
        )
        .unwrap();
        assert_eq!(hello.build.as_deref(), Some("9.9.9 abc1234"));
        // An older server's hello (`{"api": 1}` alone) still works.
        assert_eq!(
            read_hello("Mac mini", &json!({"api": 1})).unwrap().build,
            None
        );
        let message = |data: Value| match read_hello("Mac mini", &data) {
            Err(PeerError::UpdateNeeded(m)) => m,
            other => panic!("{data}: {other:?}"),
        };
        let newer = message(json!({"api": 2, "build": "9.9.9 abc1234"}));
        assert!(newer.contains("newer"), "{newer}");
        assert!(newer.contains("9.9.9 abc1234"), "{newer}");
        assert!(newer.contains("Update Hollow on this Mac"), "{newer}");
        let older = message(json!({"api": 0}));
        assert!(older.contains("older"), "{older}");
        assert!(older.contains("Update Hollow on Mac mini"), "{older}");
        // A malformed hello is fatal, not guessed at.
        for bad in [json!(null), json!([]), json!({"api": "1"}), json!({})] {
            assert!(message(bad).contains("can't read"));
        }
        // A build string from the other Mac is shown as one clean line.
        let odd = read_hello("Mac mini", &json!({"api": 1, "build": "1.0\n\u{202E}x"})).unwrap();
        assert_eq!(odd.build.as_deref(), Some("1.0 x"));
    }

    #[test]
    fn unknown_stream_events_warn_once_per_kind() {
        let mut dropped = DroppedEvents::default();
        let build = Some("9.9.9 abc1234");
        assert_eq!(dropped.note("Mac mini", build, "state"), None, "known");
        let first = dropped.note("Mac mini", build, "accountSwitched").unwrap();
        assert!(first.contains("`accountSwitched`"), "{first}");
        assert!(first.contains("Mac mini"), "{first}");
        assert!(first.contains("9.9.9 abc1234"), "{first}");
        assert_eq!(
            dropped.note("Mac mini", build, "accountSwitched"),
            Some(String::new())
        );
        assert_eq!(
            dropped.note("Mac mini", build, "accountSwitched"),
            Some(String::new())
        );
        assert!(dropped
            .note("Mac mini", build, "budgetWarning")
            .unwrap()
            .contains("budgetWarning"));
        assert_eq!(
            dropped.counts(),
            vec![
                ("accountSwitched".to_string(), 3),
                ("budgetWarning".to_string(), 1)
            ]
        );
        // A peer inventing kinds without end can't grow the set or the log
        // past the cap.
        for n in 0..(DROPPED_KINDS_MAX * 2) {
            let _ = dropped.note("Mac mini", None, &format!("k{n}"));
        }
        assert_eq!(dropped.counts().len(), DROPPED_KINDS_MAX);
        assert_eq!(
            dropped.note("Mac mini", None, "another"),
            Some(String::new())
        );
    }

    #[test]
    fn a_pair_line_must_be_a_machine_window_and_never_shows_its_code() {
        let line = r#"{"code":"7KQ2-M9XA","expiresAt":"2026-11-16T03:22:44Z","machine":true,"machineId":"m-8d21c4be90f17a3e","slug":"mac-mini","name":"Mac mini"}"#;
        let code = parse_pair_code("runner", &format!("motd\n{line}\n")).unwrap();
        assert_eq!(code.code, "7KQ2-M9XA");
        assert_eq!(code.machine_id.as_deref(), Some("m-8d21c4be90f17a3e"));
        assert!(!format!("{code}").contains("7KQ2"));
        let phone = r#"{"code":"7KQ2-M9XA","expiresAt":null}"#;
        assert!(matches!(
            parse_pair_code("runner", phone),
            Err(PeerError::UpdateNeeded(_))
        ));
        assert!(matches!(
            parse_pair_code("runner", "Pairing code: 7KQ2-M9XA"),
            Err(PeerError::Unavailable(_))
        ));
    }

    #[test]
    fn the_token_never_shows_in_debug() {
        let client =
            PeerClient::new(Dial::Tcp(1), 47680, "Mac mini").with_token("sekrit-token".into());
        let text = format!("{client:?}");
        assert!(!text.contains("sekrit"));
        assert!(text.contains("127.0.0.1:47680"));
    }

    #[tokio::test]
    async fn a_silent_stream_trips_the_watchdog_and_follow_reconnects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let opened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = {
            let opened = opened.clone();
            tokio::spawn(async move {
                let mut held = Vec::new();
                loop {
                    let (mut s, _) = listener.accept().await.unwrap();
                    let mut buf = [0u8; 4096];
                    let n = s.read(&mut buf).await.unwrap();
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    if head.starts_with("GET /v1/sessions") {
                        // The refetch after a reconnect.
                        let body = r#"{"sessions":[],"nextCursor":null}"#;
                        let reply = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        s.write_all(reply.as_bytes()).await.unwrap();
                        continue;
                    }
                    assert!(head.starts_with("GET /v1/events HTTP/1.1"), "{head}");
                    assert!(
                        head.to_ascii_lowercase().contains("host: 127.0.0.1:47680"),
                        "{head}"
                    );
                    opened.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let hello = "event: hello\ndata: {\"api\":1}\n\n";
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{hello}\r\n",
                        hello.len()
                    );
                    s.write_all(reply.as_bytes()).await.unwrap();
                    // Then silence: the connection stays open.
                    held.push(s);
                }
            })
        };
        let client =
            Arc::new(PeerClient::new(Dial::Tcp(port), 47680, "Mac mini").with_token("t".into()));
        // The stream itself.
        let mut stream = client
            .events(None, Duration::from_millis(200), None)
            .await
            .unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().event, "hello");
        let silent = stream.next().await.unwrap_err();
        assert_eq!(silent.state(), PeerState::Offline);
        assert!(
            silent.message().starts_with("No word from Mac mini"),
            "{silent}"
        );
        // Following: connect, the watchdog drops it, it reconnects.
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink: FollowSink = {
            let seen = seen.clone();
            Arc::new(move |e| seen.lock().unwrap().push(e))
        };
        let options = FollowOptions {
            watchdog: Duration::from_millis(200),
            reconnect: vec![Duration::from_millis(50), Duration::from_millis(80)],
            list_debounce: Duration::from_millis(10),
            jitter: crate::peer_link::Jitter::fixed(1.0),
            ..FollowOptions::default()
        };
        let follow = follow(client.clone(), None, options, sink);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let drops = seen
                .lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, FollowEvent::Disconnected { .. }))
                .count();
            if drops >= 2 {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                seen.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(follow);
        server.abort();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0], FollowEvent::Connected { reconnected: false });
        assert!(
            matches!(&seen[1], FollowEvent::Disconnected { message, .. } if message.starts_with("No word from Mac mini for 0 seconds"))
        );
        assert!(opened.load(std::sync::atomic::Ordering::SeqCst) >= 3);
        // A stream that drops soon after `hello` doesn't start the waits over.
        let waits: Vec<Duration> = seen
            .iter()
            .filter_map(|e| match e {
                FollowEvent::Disconnected { retry_in, .. } => Some(*retry_in),
                _ => None,
            })
            .collect();
        assert_eq!(
            waits[..2],
            [Duration::from_millis(50), Duration::from_millis(80)]
        );
    }

    #[test]
    fn remote_text_is_one_short_line_without_control_characters() {
        assert_eq!(remote_text("a\nb\x1b[31m\u{202E}c  d"), "a b [31m c d");
        let long = remote_text(&"x".repeat(500));
        assert_eq!(long.chars().count(), REMOTE_TEXT_MAX);
        assert!(long.ends_with('\u{2026}'));
        let client = PeerClient::new(Dial::Tcp(1), 1, "Mac\u{0007} mini\n");
        assert_eq!(client.name(), "Mac mini");
        assert_eq!(PeerClient::new(Dial::Tcp(1), 1, "\n").name(), "That Mac");
    }

    #[test]
    fn a_pair_codes_debug_never_shows_the_code() {
        let code = PairCode {
            code: "7KQ2-M9XA".into(),
            expires_at: None,
            machine: true,
            machine_id: Some("m-8d21c4be90f17a3e".into()),
            slug: None,
            name: Some("Mac mini".into()),
        };
        let text = format!("{code:?}");
        assert!(
            !text.contains("7KQ2") && text.contains("m-8d21c4be90f17a3e"),
            "{text}"
        );
    }

    // ── A scripted server: one answer per request, or an event stream ──

    enum Scripted {
        Json(u16, Value),
        /// Frames after their delays (ms), then the stream stays open.
        Stream(Vec<(u64, String)>),
    }

    type Handler = Arc<dyn Fn(&str, &str, Option<&str>) -> Scripted + Send + Sync>;

    async fn scripted(handler: Handler) -> (u16, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let end = loop {
                        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break end;
                        }
                        match s.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..end]).to_string();
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap_or("").split(' ');
                    let (method, path) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
                    let mut auth = None;
                    let mut length = 0usize;
                    for line in lines {
                        if let Some((k, v)) = line.split_once(':') {
                            match k.trim().to_ascii_lowercase().as_str() {
                                "authorization" => auth = Some(v.trim().to_string()),
                                "content-length" => length = v.trim().parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                    }
                    while buf.len() < end + 4 + length {
                        match s.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    match handler(method, path, auth.as_deref()) {
                        Scripted::Json(status, body) => {
                            let body = body.to_string();
                            let reply = format!(
                                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = s.write_all(reply.as_bytes()).await;
                        }
                        Scripted::Stream(frames) => {
                            let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
                            let _ = s.write_all(head.as_bytes()).await;
                            for (delay, frame) in frames {
                                tokio::time::sleep(Duration::from_millis(delay)).await;
                                let chunk = format!("{:x}\r\n{frame}\r\n", frame.len());
                                if s.write_all(chunk.as_bytes()).await.is_err() {
                                    return;
                                }
                            }
                            tokio::time::sleep(Duration::from_secs(60)).await;
                        }
                    }
                });
            }
        });
        (port, task)
    }

    fn sse(event: &str, data: Value) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    fn info(id: &str) -> Value {
        json!({"machineId": id, "slug": "mac-mini", "name": "Mac mini", "app": "hollow", "api": 1,
            "peerApi": 1, "version": "0.5.17", "commit": "abc1234", "headless": true,
            "capabilities": ["chats", "send", "stop", "cards", "events"]})
    }

    fn me() -> ThisMac {
        ThisMac {
            machine_id: "m-3f9c1a07e2b45d18".into(),
            slug: None,
            name: "MacBook Pro".into(),
            version: "0.5.17 (abc1234)".into(),
        }
    }

    fn filler(n: u8) -> MachineRecord {
        MachineRecord {
            machine_id: format!("m-00000000000000f{n}"),
            slug: None,
            name: format!("Filler {n}"),
            ssh_hosts: vec!["runner".into()],
            remote_port: 47680,
            device_id: "mc-1a2b3c4d".into(),
            paired_at: "2026-11-15T00:00:00Z".into(),
            last_seen_at: None,
            last_version: None,
            peer_api: 1,
            token_revoked: false,
            coo: None,
            coo_seen_at: None,
            coo_since: None,
            old_slugs: Vec::new(),
            capabilities: Vec::new(),
            extra: Map::new(),
        }
    }

    /// Every way pairing can fail after the other Mac handed out a token:
    /// the token goes back (`DELETE /v1/machines/me` with it) and nothing is
    /// kept here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pairing_that_fails_after_the_token_gives_it_back_and_keeps_nothing() {
        for (mode, id) in [
            ("save-fails", "m-00000000000000a1"),
            ("cap", "m-00000000000000a2"),
            ("check-fails", "m-00000000000000a3"),
            ("other-id", "m-00000000000000a4"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let machines = Arc::new(Machines::at(dir.path()));
            let calls = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            let handler: Handler = {
                let (calls, machines) = (calls.clone(), machines.clone());
                Arc::new(move |method, path, auth| {
                    calls
                        .lock()
                        .unwrap()
                        .push(format!("{method} {path} {}", auth.unwrap_or("-")));
                    match (method, path) {
                        ("POST", "/v1/machines/pair") => {
                            if mode == "cap" {
                                // Another pairing filled the list meanwhile.
                                for n in 1..=4 {
                                    machines.upsert(filler(n)).unwrap();
                                }
                            }
                            Scripted::Json(
                                200,
                                json!({"token": format!("tok-{mode}"),
                                "deviceId": "mc-0000000a", "machine": info(id), "api": 1, "peerApi": 1}),
                            )
                        }
                        ("GET", "/v1/machine") => match mode {
                            "check-fails" => Scripted::Json(
                                500,
                                json!({"error": {"code": "internal", "message": "Hollow couldn't do that."}}),
                            ),
                            "other-id" => Scripted::Json(200, info("m-00000000000000ff")),
                            _ => Scripted::Json(200, info(id)),
                        },
                        ("DELETE", "/v1/machines/me") => Scripted::Json(200, json!({})),
                        _ => Scripted::Json(
                            404,
                            json!({"error": {"code": "not_found", "message": "No."}}),
                        ),
                    }
                })
            };
            let (port, server) = scripted(handler).await;
            if mode == "save-fails" {
                *crate::peer_machines::tests::FAIL_SAVE.lock().unwrap() = Some(id.into());
            }
            let code = PairCode {
                code: "P33RC0DE".into(),
                expires_at: None,
                machine: true,
                machine_id: Some(id.into()),
                slug: None,
                name: Some("Mac mini".into()),
            };
            let bare = PeerClient::new(Dial::Tcp(port), port, "Mac mini");
            let err = pair_with_code(&machines, bare, &code, &me(), vec!["runner".into()], port)
                .await
                .unwrap_err();
            *crate::peer_machines::tests::FAIL_SAVE.lock().unwrap() = None;
            match mode {
                "save-fails" | "cap" => {
                    assert!(matches!(err, PeerError::Local(_)), "{mode}: {err:?}")
                }
                "check-fails" => assert!(matches!(err, PeerError::Internal(_)), "{mode}: {err:?}"),
                _ => assert!(
                    matches!(err, PeerError::PairingNeeded(_)),
                    "{mode}: {err:?}"
                ),
            }
            let calls = calls.lock().unwrap().clone();
            assert!(
                calls.contains(&format!("DELETE /v1/machines/me Bearer tok-{mode}")),
                "{mode}: {calls:?}"
            );
            assert_eq!(machines.get(id).unwrap(), None, "{mode}");
            assert_eq!(
                crate::peer_machines::read_token(id).unwrap(),
                None,
                "{mode}"
            );
            let kept = machines.list().unwrap().len();
            assert_eq!(kept, if mode == "cap" { 4 } else { 0 }, "{mode}");
            server.abort();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn appends_queued_before_a_refetch_are_not_shown_twice() {
        const KEY: &str = "s-20261115-0000aaaa";
        let details = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler: Handler = {
            let details = details.clone();
            Arc::new(move |method, path, _| {
                if path.starts_with("/v1/events") {
                    return Scripted::Stream(vec![
                        (0, sse("hello", json!({"api": 1}))),
                        (
                            50,
                            sse("turn", json!({"key": KEY, "ord": 2, "status": "success"})),
                        ),
                        // Queued before the refetch's snapshot: already in it.
                        (
                            100,
                            sse(
                                "append",
                                json!({"key": KEY, "ord": 3, "gen": 1, "text": "B"}),
                            ),
                        ),
                        // Long after: new text.
                        (
                            1500,
                            sse(
                                "append",
                                json!({"key": KEY, "ord": 3, "gen": 1, "text": "D"}),
                            ),
                        ),
                    ]);
                }
                assert_eq!(
                    (method, path),
                    ("GET", "/v1/sessions/s-20261115-0000aaaa?limit=20")
                );
                let n = details.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let text = ["AB", "ABC", "ABCD"][n.min(2)];
                Scripted::Json(
                    200,
                    json!({
                        "session": {"key": KEY, "state": "running"},
                        "turns": [], "hasMore": false,
                        "live": {"key": KEY, "ord": 3, "gen": 1, "text": text}
                    }),
                )
            })
        };
        let (port, server) = scripted(handler).await;
        let client =
            Arc::new(PeerClient::new(Dial::Tcp(port), port, "Mac mini").with_token("t".into()));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink: FollowSink = {
            let seen = seen.clone();
            Arc::new(move |e| seen.lock().unwrap().push(e))
        };
        let follow = follow(client, Some(KEY.into()), FollowOptions::default(), sink);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let appended = |e: &FollowEvent| matches!(e, FollowEvent::Event(f) if f.event == "append");
        while !seen.lock().unwrap().iter().any(appended) {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                seen.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(follow);
        server.abort();
        let seen = seen.lock().unwrap();
        let appends: Vec<&Value> = seen
            .iter()
            .filter_map(|e| match e {
                FollowEvent::Event(f) if f.event == "append" => Some(&f.data["text"]),
                _ => None,
            })
            .collect();
        assert_eq!(appends, [&json!("D")], "{seen:?}");
        let texts: Vec<String> = seen
            .iter()
            .filter_map(|e| match e {
                FollowEvent::Detail(d) => d.live.as_ref().map(|l| l.text.clone()),
                _ => None,
            })
            .collect();
        // The refetch after `turn`, then one more once the held append's
        // stream went quiet.
        assert_eq!(texts, ["AB", "ABC"], "{seen:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_newer_peers_unknown_events_are_dropped_and_the_stream_goes_on() {
        const KEY: &str = "s-20261117-0000aaaa";
        let handler: Handler = Arc::new(move |_, path, _| {
            assert!(path.starts_with("/v1/events"), "{path}");
            Scripted::Stream(vec![
                (
                    0,
                    sse(
                        "hello",
                        json!({"api": 1, "build": "9.9.9 abc1234", "events": ["accountSwitched"], "future": true}),
                    ),
                ),
                (10, sse("accountSwitched", json!({"account": "a"}))),
                (10, sse("accountSwitched", json!({"account": "b"}))),
                (10, sse("accountSwitched", json!({"account": "c"}))),
                (
                    10,
                    sse(
                        "state",
                        json!({"key": KEY, "state": "running", "needsAnswer": false}),
                    ),
                ),
            ])
        });
        let (port, server) = scripted(handler).await;
        let client =
            Arc::new(PeerClient::new(Dial::Tcp(port), port, "Mac mini").with_token("t".into()));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink: FollowSink = {
            let seen = seen.clone();
            Arc::new(move |e| seen.lock().unwrap().push(e))
        };
        let follow = follow(client, None, FollowOptions::default(), sink);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let state = |e: &FollowEvent| matches!(e, FollowEvent::Event(f) if f.event == "state");
        while !seen.lock().unwrap().iter().any(state) {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                seen.lock().unwrap()
            );
            // sleep-ok: polls a real local stream; bounded by the deadline above.
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(follow);
        server.abort();
        let seen = seen.lock().unwrap();
        assert!(
            matches!(seen[0], FollowEvent::Connected { reconnected: false }),
            "{seen:?}"
        );
        let events: Vec<&str> = seen
            .iter()
            .filter_map(|e| match e {
                FollowEvent::Event(f) => Some(f.event.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(events, ["state"], "{seen:?}");
        assert!(
            !seen
                .iter()
                .any(|e| matches!(e, FollowEvent::Disconnected { .. } | FollowEvent::Ended(_))),
            "{seen:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_an_unauthorized_401_marks_the_record_and_a_good_answer_clears_it() {
        let answers = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler: Handler = {
            let answers = answers.clone();
            Arc::new(move |_, _, _| {
                match answers.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                    0 => Scripted::Json(
                        401,
                        json!({"error": {"code": "something_else", "message": "Hm."}}),
                    ),
                    1 => Scripted::Json(
                        401,
                        json!({"error": {"code": "unauthorized", "message": "No."}}),
                    ),
                    _ => Scripted::Json(200, json!({"sessions": [], "nextCursor": null})),
                }
            })
        };
        let (port, server) = scripted(handler).await;
        let dir = tempfile::tempdir().unwrap();
        let machines = Arc::new(Machines::at(dir.path()));
        let mut record = filler(9);
        record.machine_id = "m-00000000000000b1".into();
        machines.upsert(record.clone()).unwrap();
        crate::peer_machines::save_token(&record.machine_id, "t").unwrap();
        let client = PeerClient::for_record(&record, Dial::Tcp(port), machines.clone()).unwrap();
        let marked = || {
            machines
                .get(&record.machine_id)
                .unwrap()
                .unwrap()
                .token_revoked
        };
        assert!(matches!(
            client.sessions(None, 1).await,
            Err(PeerError::TokenRevoked(_))
        ));
        assert!(
            !marked(),
            "a 401 that doesn't say unauthorized leaves the record"
        );
        assert!(client.sessions(None, 1).await.is_err());
        assert!(marked());
        // A marked record still gets a client; a good answer clears the mark.
        let now = machines.get(&record.machine_id).unwrap().unwrap();
        let again = PeerClient::for_record(&now, Dial::Tcp(port), machines.clone()).unwrap();
        again.sessions(None, 1).await.unwrap();
        assert!(!marked());
        server.abort();
        machines.forget(&record.machine_id).unwrap();
    }
}
