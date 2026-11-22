//! The peer link's commands (M2 PR 3, `docs/reference/peers.md`): the
//! MacBook lists its paired Macs, pairs and forgets one, reads another Mac's
//! chats live and steers one (send, stop, answer a card at its home).
//!
//! **One hub.** [`Hub`] keeps, per paired Mac, its one [`Tunnel`] (never two:
//! pairing borrows the hub's through [`peer_client::TunnelFor`]), a client
//! holding its token (read from the Keychain once, dropped on a 401, a new
//! pairing or Forget), what the UI shows ([`PeerState`] and a sentence), and
//! at most two event streams: the chat list's and the open remote chat's.
//! Every Mac's record goes through the one `Machines` registry the hub holds.
//! The app's hub is process-wide ([`Hub::for_app`]); tests make their own.
//!
//! **Nothing is kept.** Rows, previews and transcripts pass through to the
//! webview per request and per frame (`peer-chat-event`); the hub holds no
//! copy, writes nothing but the record's `lastSeenAt`, and never touches the
//! vault, the Sessions DB or the `interactions` table: a remote chat's card
//! is answered at its home. The webview keeps the last list it fetched in
//! memory for the offline view (`src/store/peerSessions.ts`).
//!
//! **Events.** `peer-machines-changed` carries [`PeerMachinesView`] whenever
//! a Mac's state or sentence changes; `peer-chat-event` carries a
//! [`PeerChatEvent`] per frame or refetch of a watched stream. Both go
//! through `Core::emit`, so a headless host would run the same code.
//!
//! **Tiers.** Pairing, Forget, the alias order, send, stop and a card's
//! answer are the person's (user-only); the reads and the watch commands are
//! the window's plumbing. No agent action reaches another Mac through these:
//! an action's answer would land in this Mac's transcript, and a remote
//! chat's words must not be kept here. M3a's dispatcher COO has its own
//! narrow action (`sessions.route_to_runner`, `dispatcher_coo`), which sends
//! through [`Hub::send`] and returns no remote words. Its routed turns are
//! watched through the Mac's list stream ([`Hub::follow_routed`]), which
//! carries a state and no words, for one notification when each ends.

use crate::core_host::Core;
use crate::peer_client::{
    self, Accepted, Dial, Follow, FollowEvent, FollowOptions, FollowSink, PairSetup, PeerClient,
    PeerError, PeerState, RemoteSession, SessionDetail, ThisMac,
};
use crate::peer_machines::{MachineRecord, Machines};
use crate::peer_tunnel::{Spawner, Tunnel};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};
use std::time::Duration;

/// Event: a Mac's state changed ([`PeerMachinesView`]).
pub(crate) const MACHINES_CHANGED: &str = "peer-machines-changed";
/// Event: a watched stream's frame or refetch ([`PeerChatEvent`]).
pub(crate) const CHAT_EVENT: &str = "peer-chat-event";
/// A chat list page, and the most rows read per Mac.
const PAGE: u32 = 200;
const ROWS_MAX: usize = 400;
/// Forget's unpair there, at most (then it is forgotten here anyway).
const FORGET_WAIT: Duration = Duration::from_secs(15);
/// How often idle tunnels are looked at.
const IDLE_SWEEP: Duration = Duration::from_secs(60);
/// A `clientMessageId`, at most.
const MESSAGE_ID_MAX: usize = 128;

// ── What the window gets ─────────────────────────────────────────────────

/// Which Hollow owns a chat (stamped by this Mac; the wire has no `home`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct MachineRef {
    pub machine_id: String,
    pub slug: Option<String>,
    pub name: String,
}

/// One paired Mac as the strip and Settings show it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerMachine {
    pub machine_id: String,
    pub slug: Option<String>,
    pub name: String,
    /// SSH aliases, tried in this order.
    pub ssh_hosts: Vec<String>,
    pub remote_port: u16,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    pub last_version: Option<String>,
    pub state: PeerState,
    /// Why it is offline, needs pairing or an update: a plain sentence.
    pub message: Option<String>,
    /// When this Hollow last heard from it or failed to (this run only);
    /// null until the first call.
    pub checked_at: Option<String>,
    /// The alias its tunnel is up through, when one is.
    pub alias: Option<String>,
    /// M5 UI: its last `GET /v1/machine` listed `moves` (it takes a chat
    /// moved to it).
    pub moves: bool,
}

/// Whether a paired Mac said it takes moved chats (`moves`).
fn takes_moves(record: &MachineRecord) -> bool {
    record.capabilities.iter().any(|c| c == "moves")
}

/// [`PeerMachinesView::move_to`] and [`PeerMachinesView::sleep_to`]: the
/// Macs the trigger's own pickers choose, each only when it takes moves.
pub(crate) fn move_targets(
    records: &[MachineRecord],
    runner_alias: Option<&str>,
) -> (Option<String>, Option<String>) {
    use crate::chat_move_trigger::{pick_for_move, pick_runner};
    let taking = |r: Result<&MachineRecord, String>| {
        r.ok()
            .filter(|r| takes_moves(r))
            .map(|r| r.machine_id.clone())
    };
    (
        taking(pick_for_move(records, runner_alias)),
        taking(pick_runner(records, runner_alias)),
    )
}

/// `peer_machines` and the `peer-machines-changed` payload.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerMachinesView {
    /// This Mac (local rows' home); null when its `machine.json` can't be
    /// read.
    pub this_mac: Option<MachineRef>,
    pub machines: Vec<PeerMachine>,
    /// The alias Pair a Mac offers first: this Mac's remote runner
    /// (`~/.config/hollow/slot-remote`), when one is set.
    pub suggested_alias: Option<String>,
    /// M5 UI: the paired Mac Move to runner moves a chat to
    /// (`chat_move_trigger::pick_for_move`), when it lists `moves`; the
    /// window offers Move to runner only then.
    pub move_to: Option<String>,
    /// M5 UI: the paired Mac a sleep moves running chats to
    /// (`chat_move_trigger::pick_runner`), when it lists `moves`; Settings
    /// names it beside Move running chats when this Mac sleeps.
    pub sleep_to: Option<String>,
}

/// One chat on another Mac: the wire's row plus its home.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerSessionRow {
    pub home: MachineRef,
    #[serde(flatten)]
    pub session: RemoteSession,
}

/// `peer_sessions`: a Mac's chats, newest first.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerSessionsPage {
    pub home: MachineRef,
    pub sessions: Vec<PeerSessionRow>,
    /// It has more than [`ROWS_MAX`] chats; the newest are here.
    pub truncated: bool,
}

/// `peer_detail`: one remote chat's turns, as sent.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerChatDetail {
    pub home: MachineRef,
    pub detail: SessionDetail,
}

/// `peer-chat-event`. `kind` is a frame's name (`live`, `append`, `state`,
/// `turn`, `sessions`), or `detail` ([`PeerChatDetail`]), `list`
/// ([`PeerSessionsPage`]), `connected` (`{reconnected}`), `disconnected`
/// (`{message, retryInMs}`) or `ended` ([`PeerFailure`]).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerChatEvent {
    pub machine_id: String,
    /// The watched chat; null for the list's stream.
    pub key: Option<String>,
    pub kind: String,
    #[cfg_attr(test, ts(type = "unknown"))]
    pub data: Value,
}

/// Why a peer command failed: the kind, a sentence, and the Mac's state
/// after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PeerFailureCode {
    Offline,
    PairingNeeded,
    UpdateNeeded,
    Forbidden,
    PairingClosed,
    WrongCode,
    NotFound,
    Busy,
    QuestionClosed,
    BadRequest,
    TooLarge,
    Misdirected,
    Unavailable,
    Local,
    Internal,
}

/// A peer command's error (what `invoke` rejects with).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerFailure {
    pub code: PeerFailureCode,
    pub message: String,
    pub state: PeerState,
    /// A wrong pairing code: tries left.
    pub attempts_left: Option<u32>,
}

impl From<PeerError> for PeerFailure {
    fn from(e: PeerError) -> Self {
        use PeerFailureCode as C;
        let attempts_left = match &e {
            PeerError::WrongCode { attempts_left, .. } => *attempts_left,
            _ => None,
        };
        let code = match &e {
            PeerError::Offline(_) => C::Offline,
            PeerError::TokenRevoked(_) | PeerError::PairingNeeded(_) => C::PairingNeeded,
            PeerError::UpdateNeeded(_) => C::UpdateNeeded,
            PeerError::Forbidden(_) => C::Forbidden,
            PeerError::PairingClosed(_) => C::PairingClosed,
            PeerError::WrongCode { .. } => C::WrongCode,
            PeerError::NotFound(_) => C::NotFound,
            PeerError::Busy(_) => C::Busy,
            PeerError::QuestionClosed(_) => C::QuestionClosed,
            PeerError::BadRequest(_) => C::BadRequest,
            PeerError::TooLarge(_) => C::TooLarge,
            PeerError::Misdirected(_) => C::Misdirected,
            PeerError::Unavailable(_) => C::Unavailable,
            PeerError::Local(_) => C::Local,
            PeerError::Internal(_) => C::Internal,
        };
        Self {
            code,
            message: e.message().to_string(),
            state: e.state(),
            attempts_left,
        }
    }
}

impl std::fmt::Display for PeerFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn local(message: impl Into<String>) -> PeerFailure {
    PeerError::Local(message.into()).into()
}

fn bad(message: impl Into<String>) -> PeerFailure {
    PeerError::BadRequest(message.into()).into()
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ── The hub ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Status {
    state: PeerState,
    message: Option<String>,
    checked_at: Option<String>,
}

/// One paired Mac in this process.
struct Peer {
    tunnel: Arc<Tunnel>,
    /// Each stream's id: a stream's frames and errors count only while it
    /// is the one in its slot, so a stream stopped (unwatched, replaced by
    /// another chat, or dropped by a new pairing) says nothing late.
    next_stream: AtomicU64,
    client: Mutex<Option<Arc<PeerClient>>>,
    status: Mutex<Status>,
    /// The chat list's stream, by its id.
    list: Mutex<Option<(u64, Follow)>>,
    /// The open remote chat's stream, by its key and id.
    chat: Mutex<Option<(String, u64, Follow)>>,
    /// M3a follow-up: the list stream runs only for routed turns
    /// ([`Hub::follow_routed`]); it stops once none is left. A window watch
    /// takes it over.
    list_for_routes: AtomicBool,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Peer {
    fn new(tunnel: Arc<Tunnel>, revoked: bool) -> Self {
        Self {
            tunnel,
            next_stream: AtomicU64::new(1),
            client: Mutex::new(None),
            status: Mutex::new(Self::new_status(revoked)),
            list: Mutex::new(None),
            chat: Mutex::new(None),
            list_for_routes: AtomicBool::new(false),
        }
    }

    /// Before the first call: unchecked (`offline`, no `checkedAt`), or
    /// pairing needed when the record says its token is dead.
    fn new_status(revoked: bool) -> Status {
        if revoked {
            Status {
                state: PeerState::PairingNeeded,
                message: Some("That Mac no longer accepts this one. Pair it again.".into()),
                checked_at: None,
            }
        } else {
            Status {
                state: PeerState::Offline,
                message: None,
                checked_at: None,
            }
        }
    }

    fn watching(&self) -> bool {
        locked(&self.list).is_some() || locked(&self.chat).is_some()
    }

    /// Whether stream `id` is still the one in its slot.
    fn current(&self, id: u64) -> bool {
        locked(&self.list).as_ref().is_some_and(|(i, _)| *i == id)
            || locked(&self.chat)
                .as_ref()
                .is_some_and(|(_, i, _)| *i == id)
    }

    fn stop_watching(&self) {
        let list = locked(&self.list).take();
        let chat = locked(&self.chat).take();
        drop((list, chat));
    }
}

type MeFn = Arc<dyn Fn() -> Result<ThisMac, String> + Send + Sync>;
type DialFn = Arc<dyn Fn(&MachineRecord, &Arc<Tunnel>) -> Dial + Send + Sync>;

/// What the hub needs (the app's in [`Hub::for_app`]; tests give stand-ins).
pub(crate) struct HubSetup {
    pub machines: Arc<Machines>,
    /// The tunnels' socket folder; `None`: `peer_tunnel::socket_dir()`.
    pub socket_dir: Option<PathBuf>,
    pub spawner: Arc<dyn Spawner>,
    /// The ssh program pairing reads the code with.
    pub ssh: OsString,
    pub pair_command: String,
    pub remote_port: u16,
    pub me: MeFn,
    /// How a client dials; `None`: through the Mac's tunnel.
    pub dial: Option<DialFn>,
    pub follow: FollowOptions,
    /// Pair a Mac's first alias ([`PeerMachinesView::suggested_alias`]).
    pub suggest: fn() -> Option<String>,
}

/// Every paired Mac's tunnel, client, state and streams.
pub(crate) struct Hub {
    setup: HubSetup,
    core: Mutex<Option<Core>>,
    peers: Mutex<HashMap<String, Arc<Peer>>>,
    /// This Mac, once read.
    this: Mutex<Option<MachineRef>>,
    /// M3a follow-up: routed turns waiting to end, all Macs.
    routed: Mutex<Vec<crate::dispatcher_coo::RoutedTurn>>,
    /// Lane M: the same turns on disk, with their delivered and expired
    /// states, so a restart picks them up and a notification never fires
    /// twice (`routed_store`). `None`: kept in memory only (tests).
    routed_book: Mutex<Option<(PathBuf, crate::routed_store::Book)>>,
    /// The pairing attempt in flight, if any (one at a time): what Cancel
    /// stops.
    pairing: Mutex<Option<PairingNow>>,
}

/// A pairing attempt in flight: its Cancel and the tunnel it made, if yet.
struct PairingNow {
    cancel: Arc<peer_client::PairCancel>,
    made: Arc<Mutex<Option<(String, bool)>>>,
}

static APP_HUB: OnceLock<Arc<Hub>> = OnceLock::new();

impl Hub {
    pub(crate) fn new(setup: HubSetup) -> Arc<Self> {
        Arc::new(Self {
            setup,
            core: Mutex::new(None),
            peers: Mutex::new(HashMap::new()),
            this: Mutex::new(None),
            routed: Mutex::new(Vec::new()),
            routed_book: Mutex::new(None),
            pairing: Mutex::new(None),
        })
    }

    /// The app's hub (`<support>/Peers/machines.json`, real ssh), made once;
    /// its idle sweep closes a tunnel nobody used for ten minutes.
    pub(crate) fn for_app() -> Result<Arc<Self>, PeerFailure> {
        if let Some(hub) = APP_HUB.get() {
            return Ok(hub.clone());
        }
        let machines = Machines::for_app()
            .ok_or_else(|| local("Hollow couldn't find its support folder on this Mac."))?;
        let hub = Self::new(HubSetup {
            machines: Arc::new(machines),
            socket_dir: None,
            spawner: Arc::new(crate::peer_tunnel::SshSpawner::new()),
            ssh: "ssh".into(),
            pair_command: peer_client::DEFAULT_PAIR_COMMAND.to_string(),
            remote_port: crate::peer_machines::DEFAULT_REMOTE_PORT,
            me: Arc::new(ThisMac::read),
            dial: None,
            follow: FollowOptions::default(),
            suggest: crate::remote_session::configured_runner,
        });
        if APP_HUB.set(hub.clone()).is_ok() {
            if let Some(path) = crate::routed_store::app_file() {
                hub.keep_routed(path);
            }
            let weak = Arc::downgrade(&hub);
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(IDLE_SWEEP).await;
                    let Some(hub) = weak.upgrade() else { return };
                    hub.close_idle(crate::peer_tunnel::IDLE_CLOSE).await;
                }
            });
        }
        Ok(APP_HUB.get().cloned().unwrap_or(hub))
    }

    /// Where events go (each command passes the app's).
    pub(crate) fn attach(&self, core: Core) {
        *locked(&self.core) = Some(core);
    }

    fn emit<S: Serialize>(&self, event: &str, payload: S) {
        let core = locked(&self.core).clone();
        if let Some(core) = core {
            if let Err(e) = core.emit(event, payload) {
                crate::launch::elog(&format!("[peer] {event}: {e}"));
            }
        }
    }

    async fn records(&self) -> Result<Vec<MachineRecord>, PeerFailure> {
        let machines = self.setup.machines.clone();
        tokio::task::spawn_blocking(move || machines.list())
            .await
            .map_err(|e| local(e.to_string()))?
            .map_err(local)
    }

    async fn record(&self, machine_id: &str) -> Result<MachineRecord, PeerFailure> {
        self.records()
            .await?
            .into_iter()
            .find(|m| m.machine_id == machine_id)
            .ok_or_else(|| {
                PeerError::NotFound("That Mac isn't paired with this Hollow.".into()).into()
            })
    }

    fn socket_dir(&self) -> Result<PathBuf, PeerError> {
        match &self.setup.socket_dir {
            Some(dir) => Ok(dir.clone()),
            None => crate::peer_tunnel::socket_dir().map_err(|e| PeerError::Local(e.to_string())),
        }
    }

    /// The Mac's one [`Peer`], made with its tunnel on first need and kept
    /// for good (one tunnel and one socket per Mac, its port fixed). Only a
    /// pairing (`adopt`) changes an existing tunnel's aliases: a call that
    /// read the record before a pairing or a reorder never puts the old ones
    /// back. A pairing at another port than the tunnel's is refused before
    /// anything changes (moving a live tunnel would point its running
    /// streams at another Hollow); nothing is torn down here.
    fn peer_for(
        &self,
        machine_id: &str,
        hosts: Vec<String>,
        remote_port: u16,
        revoked: bool,
        adopt: bool,
    ) -> Result<Arc<Peer>, PeerError> {
        let mut peers = locked(&self.peers);
        if let Some(peer) = peers.get(machine_id) {
            if adopt {
                let port = peer.tunnel.remote_port();
                if port != remote_port {
                    return Err(PeerError::Local(format!(
                        "That Mac is paired here through port {port}. Forget it first to pair it through port {remote_port}."
                    )));
                }
                peer.tunnel.set_hosts(hosts);
            }
            return Ok(peer.clone());
        }
        let tunnel = Tunnel::new(
            machine_id,
            hosts,
            remote_port,
            &self.socket_dir()?,
            self.setup.spawner.clone(),
        )
        .map_err(|e| PeerError::Local(e.to_string()))?;
        let peer = Arc::new(Peer::new(Arc::new(tunnel), revoked));
        peers.insert(machine_id.to_string(), peer.clone());
        Ok(peer)
    }

    fn peer(&self, record: &MachineRecord) -> Result<Arc<Peer>, PeerError> {
        self.peer_for(
            &record.machine_id,
            record.ssh_hosts.clone(),
            record.remote_port,
            record.token_revoked,
            false,
        )
    }

    /// The tunnel's aliases as the saved record has them now (after a
    /// pairing that failed partway).
    async fn restore_from_record(&self, machine_id: &str) {
        let (Ok(record), Some(peer)) = (self.record(machine_id).await, self.existing(machine_id))
        else {
            return;
        };
        peer.tunnel.set_hosts(record.ssh_hosts);
    }

    /// The Mac's peer, if the hub has made it (never makes one).
    fn existing(&self, machine_id: &str) -> Option<Arc<Peer>> {
        locked(&self.peers).get(machine_id).cloned()
    }

    /// [`Hub::peer`] for a call, checked against the record again after:
    /// a Forget that ran meanwhile must not leave a tunnel behind.
    async fn peer_checked(&self, record: &MachineRecord) -> Result<Arc<Peer>, PeerFailure> {
        let peer = self.peer(record).map_err(PeerFailure::from)?;
        let id = &record.machine_id;
        if self.records().await?.iter().any(|r| &r.machine_id == id) {
            return Ok(peer);
        }
        let gone = {
            let mut peers = locked(&self.peers);
            match peers.get(id) {
                Some(p) if Arc::ptr_eq(p, &peer) => peers.remove(id),
                _ => None,
            }
        };
        if let Some(gone) = gone {
            gone.stop_watching();
            gone.tunnel.close().await;
        }
        Err(PeerError::NotFound("That Mac isn't paired with this Hollow.".into()).into())
    }

    fn dial(&self, record: &MachineRecord, peer: &Peer) -> Dial {
        match &self.setup.dial {
            Some(dial) => dial(record, &peer.tunnel),
            None => Dial::Tunnel(peer.tunnel.clone()),
        }
    }

    /// The Mac's client, its token read from the Keychain once.
    fn client(&self, record: &MachineRecord, peer: &Peer) -> Result<Arc<PeerClient>, PeerError> {
        let mut held = locked(&peer.client);
        if let Some(client) = held.as_ref() {
            return Ok(client.clone());
        }
        let client = Arc::new(PeerClient::for_record(
            record,
            self.dial(record, peer),
            self.setup.machines.clone(),
        )?);
        *held = Some(client.clone());
        Ok(client)
    }

    /// Set what the UI shows for `peer`; tell the window when it changed.
    async fn set_status(&self, peer: &Peer, state: PeerState, message: Option<String>) {
        let next = Status {
            state,
            message,
            checked_at: if state == PeerState::Connecting {
                locked(&peer.status).checked_at.clone()
            } else {
                Some(now())
            },
        };
        let changed = {
            let mut status = locked(&peer.status);
            let changed = status.state != next.state || status.message != next.message;
            *status = next;
            changed
        };
        if changed {
            self.machines_changed().await;
        }
    }

    /// Note how a call to `peer` went.
    async fn note(&self, peer: &Peer, result: Result<(), &PeerError>) {
        match result {
            Ok(()) => self.set_status(peer, PeerState::Online, None).await,
            // Something on this Mac, or refused before a request: the
            // other Mac's state is unknown from it.
            Err(PeerError::Local(_) | PeerError::BadRequest(_)) => {}
            Err(e) => {
                let state = e.state();
                if state == PeerState::PairingNeeded {
                    // A dead token: the next call reads the Keychain again
                    // (after a new pairing it holds the new one).
                    locked(&peer.client).take();
                }
                let message = (state != PeerState::Online).then(|| e.message().to_string());
                self.set_status(peer, state, message).await;
            }
        }
    }

    /// Run `call` with the Mac's client: `connecting` while its tunnel
    /// opens, its state from how it went.
    async fn with_client<T, F, Fut>(&self, machine_id: &str, call: F) -> Result<T, PeerFailure>
    where
        F: FnOnce(Arc<PeerClient>) -> Fut,
        Fut: std::future::Future<Output = Result<T, PeerError>>,
    {
        let record = self.record(machine_id).await?;
        let peer = self.peer_checked(&record).await?;
        let client = match self.client(&record, &peer) {
            Ok(client) => client,
            Err(e) => {
                self.note(&peer, Err(&e)).await;
                return Err(e.into());
            }
        };
        // A tunnel held closed by Cancel isn't connecting: the call fails
        // at once with Cancel's sentence (no flicker back to Connecting).
        if self.setup.dial.is_none()
            && !peer.tunnel.stopped()
            && peer.tunnel.alias().await.is_none()
        {
            self.set_status(&peer, PeerState::Connecting, None).await;
        }
        let out = call(client).await;
        self.note(&peer, out.as_ref().map(|_| ())).await;
        out.map_err(PeerFailure::from)
    }

    /// This Mac as it introduces itself (`machine.json`, its name), read
    /// once off the async threads.
    async fn me(&self) -> Result<ThisMac, String> {
        let me = self.setup.me.clone();
        tokio::task::spawn_blocking(move || me())
            .await
            .map_err(|e| e.to_string())?
    }

    async fn this_mac(&self) -> Option<MachineRef> {
        if let Some(known) = locked(&self.this).clone() {
            return Some(known);
        }
        match self.me().await {
            Ok(me) => {
                let known = MachineRef {
                    machine_id: me.machine_id,
                    slug: me.slug,
                    name: me.name,
                };
                *locked(&self.this) = Some(known.clone());
                Some(known)
            }
            Err(e) => {
                crate::launch::elog(&format!("[peer] this Mac's identity: {e}"));
                None
            }
        }
    }

    async fn machine_of(&self, record: &MachineRecord) -> PeerMachine {
        let peer = self.existing(&record.machine_id);
        let status = peer
            .as_ref()
            .map(|p| locked(&p.status).clone())
            .unwrap_or_else(|| Peer::new_status(record.token_revoked));
        let alias = match &peer {
            Some(peer) if self.setup.dial.is_none() => peer.tunnel.alias().await,
            _ => None,
        };
        PeerMachine {
            machine_id: record.machine_id.clone(),
            slug: record.slug.clone(),
            name: record.name.clone(),
            ssh_hosts: record.ssh_hosts.clone(),
            remote_port: record.remote_port,
            paired_at: record.paired_at.clone(),
            last_seen_at: record.last_seen_at.clone(),
            last_version: record.last_version.clone(),
            state: status.state,
            message: status.message,
            checked_at: status.checked_at,
            alias,
            moves: takes_moves(record),
        }
    }

    /// This Mac and every paired one, as they stand.
    pub(crate) async fn view(&self) -> Result<PeerMachinesView, PeerFailure> {
        let records = self.records().await?;
        let mut machines = Vec::with_capacity(records.len());
        for record in &records {
            machines.push(self.machine_of(record).await);
        }
        let runner = (self.setup.suggest)();
        let (move_to, sleep_to) = move_targets(&records, runner.as_deref());
        Ok(PeerMachinesView {
            this_mac: self.this_mac().await,
            machines,
            suggested_alias: runner,
            move_to,
            sleep_to,
        })
    }

    async fn machines_changed(&self) {
        match self.view().await {
            Ok(view) => self.emit(MACHINES_CHANGED, view),
            Err(e) => crate::launch::elog(&format!("[peer] {e}")),
        }
    }

    /// `peer_machines`: with `check`, ask every Mac who it is first (`GET
    /// /v1/machine`, skipping a tunnel's backoff: the person's Check now).
    pub(crate) async fn machines(
        self: &Arc<Self>,
        check: bool,
    ) -> Result<PeerMachinesView, PeerFailure> {
        if check {
            let mut checks = tokio::task::JoinSet::new();
            for record in self.records().await? {
                if let Some(peer) = self.existing(&record.machine_id) {
                    peer.tunnel.retry_now();
                }
                let hub = self.clone();
                checks.spawn(async move {
                    let _ = hub
                        .with_client(&record.machine_id, |client| async move {
                            client.machine().await
                        })
                        .await;
                });
            }
            while checks.join_next().await.is_some() {}
        }
        self.view().await
    }

    /// A2: every paired Mac's account snapshot, for `peer_usage`'s
    /// background fetch. `paired` hears the Macs asked (once, before any
    /// answer); `each` hears every `(machineId, snapshot)` as it arrives,
    /// `None` for a Mac that doesn't list `usage`, answers as another Mac,
    /// or fails. A Mac whose token is dead is not asked. When the paired
    /// Macs can't be read, nobody hears anything (what is held stays).
    pub(crate) async fn usage_snapshots(
        self: &Arc<Self>,
        paired: impl FnOnce(&[String]),
        each: impl Fn(String, Option<crate::peer_usage::PeerUsage>),
    ) {
        let Ok(records) = self.records().await else {
            return;
        };
        let records: Vec<MachineRecord> =
            records.into_iter().filter(|r| !r.token_revoked).collect();
        paired(
            &records
                .iter()
                .map(|r| r.machine_id.clone())
                .collect::<Vec<_>>(),
        );
        let mut asks = tokio::task::JoinSet::new();
        for record in records {
            let hub = self.clone();
            asks.spawn(async move {
                let id = record.machine_id.clone();
                let usage = hub
                    .with_client(&record.machine_id, |client| async move {
                        let info = client.machine().await?;
                        if !crate::peer_usage::lists_usage(&info.capabilities) {
                            return Ok(None);
                        }
                        client.usage().await
                    })
                    .await
                    .ok()
                    .flatten();
                let usage = crate::peer_usage::from_peer(&id, usage);
                (id, usage)
            });
        }
        while let Some(done) = asks.join_next().await {
            if let Ok((id, usage)) = done {
                each(id, usage);
            }
        }
    }

    /// `peer_pair`: read a code over ssh at `alias`, open the Mac's tunnel
    /// and trade the code for a token.
    ///
    /// `address` is what the person typed: an ssh alias, `user@host`, a
    /// tailnet or dotted name or an IPv4 address, each with an optional
    /// `:port` (Hollow's port there). It is checked first
    /// ([`crate::peer_machines::parse_pair_address`]); the Mac is then
    /// reached the same way as any other, over the person's own ssh login.
    pub(crate) async fn pair(self: &Arc<Self>, address: &str) -> Result<PeerMachine, PeerFailure> {
        let typed = crate::peer_machines::parse_pair_address(address).map_err(bad)?;
        self.pair_at(&typed.host, typed.port.unwrap_or(self.setup.remote_port))
            .await
    }

    /// `peer_pair_cancel` (Cancel on the Pairing step): stops the pairing in
    /// flight, if one. Its code read ends, the tunnel stops opening, and
    /// nothing is kept (see [`peer_client::pair_over_ssh_cancellable`]).
    /// `true` when a pairing was running.
    pub(crate) fn cancel_pairing(&self) -> bool {
        let now = locked(&self.pairing);
        let Some(now) = now.as_ref() else {
            return false;
        };
        now.cancel.cancel();
        let made = locked(&now.made).clone();
        if let Some((id, _)) = made {
            if let Some(peer) = self.existing(&id) {
                // Its opening fails at once instead of waiting out ssh.
                peer.tunnel.stop_opening();
            }
        }
        true
    }

    /// [`Hub::pair`] with the port its Hollow answers on there.
    pub(crate) async fn pair_at(
        self: &Arc<Self>,
        alias: &str,
        remote_port: u16,
    ) -> Result<PeerMachine, PeerFailure> {
        let alias = alias.trim();
        if !crate::remote_session::valid_host(alias) {
            return Err(bad(crate::peer_machines::host_refusal(alias)));
        }
        // A Mac already reached through this alias on another port is
        // refused before ssh opens a pairing window there (`tunnel_for`
        // checks again once the code names the Mac).
        let elsewhere = locked(&self.peers).values().find_map(|p| {
            let port = p.tunnel.remote_port();
            (port != remote_port && p.tunnel.hosts().iter().any(|h| h == alias)).then_some(port)
        });
        if let Some(port) = elsewhere {
            return Err(local(format!(
                "{alias} is paired here through port {port}. Forget that Mac in Settings first to pair it through port {remote_port}."
            )));
        }
        let cancel = Arc::new(peer_client::PairCancel::default());
        let made = Arc::new(Mutex::new(None::<(String, bool)>));
        {
            let mut now = locked(&self.pairing);
            if now.is_some() {
                return Err(local(
                    "A pairing is already running. Cancel it or wait for it to finish.".to_string(),
                ));
            }
            *now = Some(PairingNow {
                cancel: cancel.clone(),
                made: made.clone(),
            });
        }
        // Cleared however this attempt ends (an early return included).
        struct Done<'a>(&'a Mutex<Option<PairingNow>>);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                *locked(self.0) = None;
            }
        }
        let _done = Done(&self.pairing);
        let me = self.me().await.map_err(local)?;
        let more_hosts = peer_client::hosts_after(&self.setup.ssh, alias).await;
        let setup = PairSetup {
            ssh: self.setup.ssh.clone(),
            alias: alias.to_string(),
            more_hosts,
            command: self.setup.pair_command.clone(),
            remote_port,
            machines: self.setup.machines.clone(),
            me,
        };
        let tunnel_for: peer_client::TunnelFor = {
            let made = made.clone();
            Box::new(move |id, hosts, port| {
                let existed = locked(&self.peers).contains_key(id);
                let peer = self.peer_for(id, hosts, port, false, true)?;
                *locked(&made) = Some((id.to_string(), existed));
                // Pairing is the person's own try: this Mac's tunnel doesn't
                // wait out a backoff from earlier failures. Cleared here,
                // right before the pairing's own opening (a reconnect while
                // the code was read could have set a new one), and without
                // waiting for an opening in progress.
                peer.tunnel.retry_now();
                Ok(peer.tunnel.clone())
            })
        };
        match peer_client::pair_over_ssh_cancellable(setup, tunnel_for, Some(cancel)).await {
            Ok(paired) => {
                let id = paired.record.machine_id.clone();
                let peer = self.peer(&paired.record).map_err(PeerFailure::from)?;
                // Streams and a client from an earlier pairing are dead now
                // (only now: a failed pairing leaves a working one alone).
                // The streams that ran start again with the new token.
                let list = locked(&peer.list).is_some();
                let chat = locked(&peer.chat).as_ref().map(|(k, _, _)| k.clone());
                peer.stop_watching();
                *locked(&peer.client) = Some(Arc::new(paired.client));
                self.set_status(&peer, PeerState::Online, None).await;
                // A stream that can't start again is logged; the window
                // watches again on the `peer-machines-changed` below.
                let restarts = list.then_some(None).into_iter().chain(chat.map(Some));
                for key in restarts {
                    if let Err(e) = self.watch(&id, key).await {
                        crate::launch::elog(&format!("[peer] a stream after pairing: {e}"));
                    }
                }
                self.machines_changed().await;
                Ok(self.machine_of(&paired.record).await)
            }
            Err(e) => {
                // A tunnel opened only for this pairing goes with it.
                let made = locked(&made).take();
                match made {
                    Some((id, false)) => {
                        let peer = locked(&self.peers).remove(&id);
                        if let Some(peer) = peer {
                            peer.stop_watching();
                            peer.tunnel.close().await;
                        }
                    }
                    // An earlier pairing's peer gets its aliases and port back.
                    Some((id, true)) => self.restore_from_record(&id).await,
                    None => {}
                }
                self.machines_changed().await;
                Err(e.into())
            }
        }
    }

    /// `peer_forget`: unpair this Mac there when it answers within
    /// [`FORGET_WAIT`], then drop its record, token, streams and tunnel here.
    pub(crate) async fn forget(&self, machine_id: &str) -> Result<bool, PeerFailure> {
        let record = self
            .records()
            .await?
            .into_iter()
            .find(|m| m.machine_id == machine_id);
        // Made if needed: unpairing there goes through its tunnel, which
        // closes below with the peer.
        let peer = record.as_ref().and_then(|r| self.peer(r).ok());
        let client = match (&record, &peer) {
            (Some(record), Some(peer)) => self.client(record, peer).ok(),
            _ => None,
        };
        let machines = &self.setup.machines;
        let forgot = match tokio::time::timeout(
            FORGET_WAIT,
            peer_client::forget_machine(machines, machine_id, client.as_deref()),
        )
        .await
        {
            Ok(done) => done,
            Err(_) => peer_client::forget_machine(machines, machine_id, None).await,
        }
        .map_err(PeerFailure::from)?;
        let peer = locked(&self.peers).remove(machine_id);
        if let Some(peer) = peer {
            peer.stop_watching();
            peer.tunnel.close().await;
        }
        self.machines_changed().await;
        Ok(forgot)
    }

    /// M5 PR2: the paired Macs' records (the move trigger picks the one a
    /// chat moves to).
    pub(crate) async fn paired(&self) -> Result<Vec<MachineRecord>, PeerFailure> {
        self.records().await
    }

    /// M5 PR2: a paired Mac's client for a chat's move (`chat_move_send`),
    /// its tunnel and token as every other call's. The Mac's shown state is
    /// left as it is: the move's own answers say how it went.
    pub(crate) async fn move_client(
        &self,
        machine_id: &str,
    ) -> Result<Arc<PeerClient>, PeerFailure> {
        let record = self.record(machine_id).await?;
        let peer = self.peer_checked(&record).await?;
        match self.client(&record, &peer) {
            Ok(client) => Ok(client),
            Err(e) => {
                self.note(&peer, Err(&e)).await;
                Err(e.into())
            }
        }
    }

    /// `peer_cancel_connect` (the strip's and Settings' Cancel while a Mac
    /// connects): its tunnel stops opening and stays closed until Retry,
    /// Check now, an alias change or a new pairing; the Mac reads offline
    /// with Cancel's sentence. A tunnel that is up is left alone.
    pub(crate) async fn cancel_connect(
        &self,
        machine_id: &str,
    ) -> Result<PeerMachine, PeerFailure> {
        let record = self.record(machine_id).await?;
        if let Some(peer) = self.existing(machine_id) {
            peer.tunnel.stop_opening();
            if locked(&peer.status).state == PeerState::Connecting {
                let why = crate::peer_tunnel::TunnelError::Stopped.to_string();
                self.set_status(&peer, PeerState::Offline, Some(why)).await;
            }
        }
        Ok(self.machine_of(&record).await)
    }

    /// `peer_reorder_hosts`: the Mac's aliases, in the order to try them.
    pub(crate) async fn set_hosts(
        &self,
        machine_id: &str,
        hosts: Vec<String>,
    ) -> Result<PeerMachine, PeerFailure> {
        let hosts: Vec<String> = hosts.into_iter().map(|h| h.trim().to_string()).collect();
        {
            let (machines, id, hosts) = (
                self.setup.machines.clone(),
                machine_id.to_string(),
                hosts.clone(),
            );
            tokio::task::spawn_blocking(move || machines.set_hosts(&id, hosts))
                .await
                .map_err(|e| local(e.to_string()))?
                .map_err(bad)?;
        }
        let record = self.record(machine_id).await?;
        if let Ok(peer) = self.peer(&record) {
            peer.tunnel.set_hosts(hosts);
            peer.tunnel.retry_now();
        }
        self.machines_changed().await;
        Ok(self.machine_of(&record).await)
    }

    fn home_of(record: &MachineRecord) -> MachineRef {
        MachineRef {
            machine_id: record.machine_id.clone(),
            slug: record.slug.clone(),
            name: record.name.clone(),
        }
    }

    fn rows(home: &MachineRef, sessions: Vec<RemoteSession>) -> Vec<PeerSessionRow> {
        sessions
            .into_iter()
            .map(|session| PeerSessionRow {
                home: home.clone(),
                session,
            })
            .collect()
    }

    /// `peer_sessions`: up to [`ROWS_MAX`] of the Mac's chats, newest first.
    pub(crate) async fn sessions(&self, machine_id: &str) -> Result<PeerSessionsPage, PeerFailure> {
        let home = Self::home_of(&self.record(machine_id).await?);
        let (sessions, truncated) = self
            .with_client(machine_id, |client| async move {
                let mut sessions = Vec::new();
                let mut cursor: Option<String> = None;
                loop {
                    let page = client.sessions(cursor.as_deref(), PAGE).await?;
                    sessions.extend(page.sessions);
                    match page.next_cursor {
                        Some(next) if sessions.len() < ROWS_MAX => cursor = Some(next),
                        more => return Ok((sessions, more.is_some())),
                    }
                }
            })
            .await?;
        let truncated = truncated || sessions.len() > ROWS_MAX;
        let mut rows = Self::rows(&home, sessions);
        rows.truncate(ROWS_MAX);
        Ok(PeerSessionsPage {
            home,
            sessions: rows,
            truncated,
        })
    }

    /// `peer_detail`: a remote chat's newest `limit` turns below `before`.
    pub(crate) async fn detail(
        &self,
        machine_id: &str,
        key: &str,
        before: Option<u32>,
        limit: Option<u32>,
    ) -> Result<PeerChatDetail, PeerFailure> {
        let home = Self::home_of(&self.record(machine_id).await?);
        let detail = self
            .with_client(machine_id, |client| async move {
                client.detail(key, before, limit.unwrap_or(20)).await
            })
            .await?;
        Ok(PeerChatDetail { home, detail })
    }

    /// `peer_send`: text only, as the chat's composer at its home sends it.
    pub(crate) async fn send(
        &self,
        machine_id: &str,
        key: &str,
        text: &str,
        client_message_id: &str,
    ) -> Result<Accepted, PeerFailure> {
        message_id(client_message_id)?;
        if text.trim().is_empty() {
            return Err(bad("Type a message first."));
        }
        self.with_client(machine_id, |client| async move {
            client.send(key, text, client_message_id).await
        })
        .await
    }

    /// `peer_stop`: whether a running or queued turn was stopped there.
    pub(crate) async fn stop(&self, machine_id: &str, key: &str) -> Result<bool, PeerFailure> {
        self.with_client(machine_id, |client| async move { client.stop(key).await })
            .await
    }

    /// M3a: the Mac's runner COO chat key now (`GET /v1/machine`, which also
    /// updates the record's `coo`); `None` when it runs none. For the
    /// dispatcher's route only (`dispatcher_coo`), never the window.
    pub(crate) async fn runner_coo(&self, machine_id: &str) -> Result<Option<String>, PeerFailure> {
        let info = self
            .with_client(machine_id, |client| async move { client.machine().await })
            .await?;
        Ok(info.runner_coo().map(str::to_string))
    }

    /// M3a follow-up: start `machine_id`'s runner COO's first conversation
    /// (the person's Start its COO, `peer_start_coo`), with
    /// `dispatcher_coo::first_message` from this Mac's name. Its record then
    /// names that conversation, so this Mac's COO dispatches at once. When
    /// it already has one, that one's key (nothing is sent). The key.
    pub(crate) async fn start_coo(&self, machine_id: &str) -> Result<String, PeerFailure> {
        let from = self
            .me()
            .await
            .map(|me| me.name)
            .unwrap_or_else(|_| "this Mac".into());
        let text = crate::dispatcher_coo::first_message(&from);
        let message_id = format!("coo-start-{:032x}", rand::random::<u128>());
        let started = self
            .with_client(machine_id, |client| {
                let (text, message_id) = (text.clone(), message_id.clone());
                async move { client.start_coo(&text, &message_id).await }
            })
            .await;
        let key = match started {
            Ok(key) => key,
            // It has one already (another Mac, the phone, a wake): that one.
            Err(e) if e.code == PeerFailureCode::Busy => {
                return self.runner_coo(machine_id).await?.ok_or(e);
            }
            Err(e) => {
                // Not the runner any more, or older: read what it says now.
                let _ = self.runner_coo(machine_id).await;
                return Err(e);
            }
        };
        let (machines, id, coo) = (
            self.setup.machines.clone(),
            machine_id.to_string(),
            key.clone(),
        );
        let _ = tokio::task::spawn_blocking(move || {
            machines.set_coo(&id, Some(&coo), Utc::now());
        })
        .await;
        self.machines_changed().await;
        Ok(key)
    }

    /// M3a: ask each of `machine_ids` whether it runs a runner COO (the
    /// app's hourly check, `dispatcher_coo::start_refresh`), so its record's
    /// `cooSeenAt` stays fresh. A Mac that doesn't answer is noted offline
    /// as any call is; nothing else happens. The Macs are asked at once,
    /// each for at most `dispatcher_coo::CHECK_WAIT`, so one dead Mac never
    /// holds up the others.
    pub(crate) async fn refresh_coo(self: &Arc<Self>, machine_ids: &[String]) {
        let mut asks = tokio::task::JoinSet::new();
        for id in machine_ids {
            let (hub, id) = (self.clone(), id.clone());
            asks.spawn(async move {
                let wait = crate::dispatcher_coo::CHECK_WAIT;
                let said = match tokio::time::timeout(wait, hub.runner_coo(&id)).await {
                    Ok(Ok(_)) => return,
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => format!("no answer within {} s", wait.as_secs()),
                };
                crate::launch::elog(&format!(
                    "[dispatcher] asking {id} for its runner COO: {said}"
                ));
            });
        }
        while asks.join_next().await.is_some() {}
    }

    /// `peer_answer`: a card answered at the chat's home.
    pub(crate) async fn answer(
        &self,
        machine_id: &str,
        key: &str,
        question_id: &str,
        client_message_id: &str,
        answers: &Map<String, Value>,
    ) -> Result<Accepted, PeerFailure> {
        message_id(client_message_id)?;
        for (item, value) in answers {
            let ok = match value {
                Value::Null | Value::String(_) => true,
                Value::Array(items) => items.iter().all(Value::is_string),
                _ => false,
            };
            if !ok {
                return Err(bad(format!(
                    "The answer to `{}` must be text, a list of text or empty.",
                    peer_client::remote_text(item)
                )));
            }
        }
        self.with_client(machine_id, |client| async move {
            client
                .answer(key, question_id, client_message_id, answers)
                .await
        })
        .await
    }

    /// `peer_watch`: follow the Mac's chat list (`key` none) or one chat.
    /// One of each per Mac: a new chat replaces the one watched.
    pub(crate) async fn watch(
        self: &Arc<Self>,
        machine_id: &str,
        key: Option<String>,
    ) -> Result<(), PeerFailure> {
        self.watch_as(machine_id, key, false).await
    }

    /// [`Self::watch`]; `for_routes`: the list is watched for routed turns
    /// only (it stops when they end), unless the window watches it too.
    async fn watch_as(
        self: &Arc<Self>,
        machine_id: &str,
        key: Option<String>,
        for_routes: bool,
    ) -> Result<(), PeerFailure> {
        let record = self.record(machine_id).await?;
        let peer = self.peer_checked(&record).await?;
        let client = match self.client(&record, &peer) {
            Ok(client) => client,
            Err(e) => {
                self.note(&peer, Err(&e)).await;
                return Err(e.into());
            }
        };
        let id = peer.next_stream.fetch_add(1, Ordering::SeqCst);
        let start = || {
            let sink = self.sink(&record, &peer, key.clone(), id);
            peer_client::follow(client, key.clone(), self.setup.follow.clone(), sink)
        };
        // The check and the start under the slot's lock: two calls at once
        // start one stream.
        match &key {
            Some(key) => {
                let mut slot = locked(&peer.chat);
                if slot
                    .as_ref()
                    .is_some_and(|(k, _, f)| k == key && !f.is_finished())
                {
                    return Ok(());
                }
                *slot = Some((key.clone(), id, start()));
            }
            None => {
                let mut slot = locked(&peer.list);
                if slot.as_ref().is_some_and(|(_, f)| !f.is_finished()) {
                    if !for_routes {
                        peer.list_for_routes.store(false, Ordering::SeqCst);
                    }
                    return Ok(());
                }
                peer.list_for_routes.store(for_routes, Ordering::SeqCst);
                *slot = Some((id, start()));
            }
        }
        Ok(())
    }

    /// `peer_unwatch`: stop following the list (`key` none) or that chat.
    pub(crate) fn unwatch(&self, machine_id: &str, key: Option<&str>) {
        let Some(peer) = locked(&self.peers).get(machine_id).cloned() else {
            return;
        };
        let stopped = match key {
            // Routed turns still wait on it: kept for them.
            None if self.routes_wait_on(machine_id) => {
                peer.list_for_routes.store(true, Ordering::SeqCst);
                None
            }
            None => locked(&peer.list).take().map(|(_, follow)| follow),
            Some(key) => {
                let mut chat = locked(&peer.chat);
                match chat.as_ref() {
                    Some((k, _, _)) if k == key => chat.take().map(|(_, _, follow)| follow),
                    _ => None,
                }
            }
        };
        drop(stopped);
    }

    /// M3a follow-up: watch for `turn` to end on its Mac (the dispatcher's
    /// route, `dispatcher_coo::route_action`), through that Mac's list
    /// stream, started for it when the window doesn't follow the list. One
    /// notification when it ends ([`Self::saw_rows`]); nothing of the chat
    /// is kept. A stream that can't start is logged: the route stands.
    pub(crate) async fn follow_routed(self: &Arc<Self>, turn: crate::dispatcher_coo::RoutedTurn) {
        let machine_id = turn.machine_id.clone();
        // Kept on disk first. A chat with a turn waiting already adds
        // nothing (one notification covers it).
        let now_ms = chrono::Utc::now().timestamp_millis();
        let added = self
            .with_book(|book| book.add(turn.entry(), now_ms))
            .unwrap_or(true);
        if added {
            locked(&self.routed).push(turn);
        }
        if let Err(e) = self.watch_as(&machine_id, None, true).await {
            crate::launch::elog(&format!(
                "[dispatcher] can't watch {machine_id} for the routed turn's end: {e}"
            ));
        }
    }

    /// Lane M: keep routed turns in the file at `path` from now on, reading
    /// what it holds (the app's, once, in [`Self::for_app`]).
    pub(crate) fn keep_routed(&self, path: PathBuf) {
        let mut book = crate::routed_store::Book::load(&path);
        book.prune(chrono::Utc::now().timestamp_millis());
        *locked(&self.routed_book) = Some((path, book));
    }

    /// Change the routed turns' book and write it. `None` when this hub
    /// keeps none; a write that fails is logged (the memory list goes on).
    fn with_book<R>(&self, f: impl FnOnce(&mut crate::routed_store::Book) -> R) -> Option<R> {
        let mut held = locked(&self.routed_book);
        let (path, book) = held.as_mut()?;
        let out = f(book);
        if let Err(e) = book.save(path) {
            crate::launch::elog(&format!("[dispatcher] routed turns not saved: {e}"));
        }
        Some(out)
    }

    /// Lane M, at boot: watch again for each routed turn the file says still
    /// waits (a turn that ended while Hollow was closed ends at the first
    /// list, with its one notification). Idempotent: a turn already
    /// followed is not added twice.
    pub(crate) async fn resume_routed(self: &Arc<Self>) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let turns: Vec<crate::dispatcher_coo::RoutedTurn> = self
            .with_book(|book| {
                book.prune(now_ms);
                book.pending()
                    .into_iter()
                    .map(|e| crate::dispatcher_coo::RoutedTurn::restored(e, now_ms))
                    .collect()
            })
            .unwrap_or_default();
        let mut machines = std::collections::BTreeSet::new();
        for turn in turns {
            machines.insert(turn.machine_id.clone());
            let mut held = locked(&self.routed);
            if !held.iter().any(|t| t.id() == turn.id()) {
                held.push(turn);
            }
        }
        for machine_id in machines {
            if let Err(e) = self.watch_as(&machine_id, None, true).await {
                crate::launch::elog(&format!(
                    "[dispatcher] can't watch {machine_id} for a routed turn's end: {e}"
                ));
            }
        }
    }

    /// Whether a routed turn on `machine_id` still waits to end.
    fn routes_wait_on(&self, machine_id: &str) -> bool {
        locked(&self.routed)
            .iter()
            .any(|t| t.machine_id == machine_id)
    }

    /// A list of `machine_id`'s rows came: each routed turn there that
    /// ended gets its notification; expired ones are dropped. With none
    /// left, a list stream started only for them stops.
    fn saw_rows(self: &Arc<Self>, machine_id: &str, rows: &[RemoteSession]) {
        let now = std::time::Instant::now();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let (mut ended, mut busy, mut expired) = (Vec::new(), Vec::new(), Vec::new());
        locked(&self.routed).retain_mut(|t| {
            if t.machine_id != machine_id {
                return true;
            }
            let state = rows
                .iter()
                .find(|r| r.key == t.key)
                .map(|r| r.state.as_str());
            let was_busy = t.seen_busy;
            match t.saw(state, now) {
                crate::dispatcher_coo::Seen::Waiting => {
                    if t.seen_busy && !was_busy {
                        busy.push(t.id());
                    }
                    true
                }
                crate::dispatcher_coo::Seen::Ended => {
                    ended.push(t.clone());
                    false
                }
                crate::dispatcher_coo::Seen::Expired => {
                    expired.push(t.id());
                    false
                }
            }
        });
        // Each ended turn takes its one notification on disk first: a turn
        // already delivered (a replayed list, a second stream) gives none.
        let ended: Vec<_> = self
            .with_book(|book| {
                for id in &busy {
                    book.note_busy(id);
                }
                for id in &expired {
                    book.expire(id, now_ms);
                }
                ended
                    .iter()
                    .filter(|t| book.claim(&t.id(), now_ms))
                    .cloned()
                    .collect()
            })
            .unwrap_or(ended);
        if !ended.is_empty() {
            let core = locked(&self.core).clone();
            let hub = self.clone();
            tokio::task::spawn_blocking(move || {
                for t in &ended {
                    if !crate::dispatcher_coo::notify_ended(core.as_ref(), t) {
                        // Not posted: given back, so the next list tries again.
                        let again = hub
                            .with_book(|book| book.unclaim(&t.id(), now_ms))
                            .unwrap_or(true);
                        if again {
                            locked(&hub.routed).push(t.clone());
                        }
                    }
                }
            });
        }
        self.release_route_list(machine_id);
    }

    /// Stop `machine_id`'s list stream when it ran only for routed turns
    /// and none waits on it now. Off the stream's own callback (the stream
    /// may be the caller).
    fn release_route_list(&self, machine_id: &str) {
        if self.routes_wait_on(machine_id) {
            return;
        }
        let Some(peer) = self.existing(machine_id) else {
            return;
        };
        if peer.list_for_routes.swap(false, Ordering::SeqCst) {
            let stopped = locked(&peer.list).take();
            tokio::spawn(async move { drop(stopped) });
        }
    }

    /// Routed turns older than `dispatcher_coo::ROUTED_FOLLOW_MAX` are
    /// forgotten (the idle sweep), and their list streams released.
    fn forget_old_routes(&self) {
        let max = crate::dispatcher_coo::ROUTED_FOLLOW_MAX;
        let mut gone = Vec::new();
        let mut ids = Vec::new();
        locked(&self.routed).retain(|t| {
            let keep = t.at.elapsed() < max;
            if !keep {
                gone.push(t.machine_id.clone());
                ids.push(t.id());
            }
            keep
        });
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.with_book(|book| {
            for id in &ids {
                book.expire(id, now_ms);
            }
            book.prune(now_ms);
        });
        for machine_id in gone {
            self.release_route_list(&machine_id);
        }
    }

    /// A restart of the app, as far as the routed turns go: the memory list,
    /// the book and `machine_id`'s list stream are gone (tests).
    #[cfg(test)]
    pub(crate) fn forget_routed_in_memory(&self, machine_id: &str) {
        locked(&self.routed).clear();
        *locked(&self.routed_book) = None;
        self.unwatch(machine_id, None);
    }

    /// The routed turns still waited on (tests).
    #[cfg(test)]
    pub(crate) fn routed_waiting(&self) -> Vec<String> {
        locked(&self.routed).iter().map(|t| t.key.clone()).collect()
    }

    /// What a stream reports, turned into `peer-chat-event` and the Mac's
    /// state. Holds the hub and the peer weakly (the peer holds the stream).
    fn sink(
        self: &Arc<Self>,
        record: &MachineRecord,
        peer: &Arc<Peer>,
        key: Option<String>,
        stream: u64,
    ) -> FollowSink {
        let hub: Weak<Hub> = Arc::downgrade(self);
        let peer: Weak<Peer> = Arc::downgrade(peer);
        let home = Self::home_of(record);
        Arc::new(move |event: FollowEvent| {
            let (Some(hub), Some(peer)) = (hub.upgrade(), peer.upgrade()) else {
                return;
            };
            if !peer.current(stream) {
                return;
            }
            let (kind, data, status): (String, Value, Option<Result<(), PeerError>>) = match event {
                FollowEvent::Connected { reconnected } => (
                    "connected".into(),
                    json!({ "reconnected": reconnected }),
                    Some(Ok(())),
                ),
                FollowEvent::Event(frame) => (frame.event, frame.data, None),
                FollowEvent::Detail(detail) => (
                    "detail".into(),
                    serde_json::to_value(PeerChatDetail {
                        home: home.clone(),
                        detail: *detail,
                    })
                    .unwrap_or(Value::Null),
                    None,
                ),
                FollowEvent::Sessions(page) => {
                    // M3a follow-up: a routed turn there may have ended.
                    hub.saw_rows(&home.machine_id, &page.sessions);
                    let mut sessions = Self::rows(&home, page.sessions);
                    let truncated = page.next_cursor.is_some() || sessions.len() > ROWS_MAX;
                    sessions.truncate(ROWS_MAX);
                    (
                        "list".into(),
                        serde_json::to_value(PeerSessionsPage {
                            home: home.clone(),
                            sessions,
                            truncated,
                        })
                        .unwrap_or(Value::Null),
                        None,
                    )
                }
                FollowEvent::Disconnected { message, retry_in } => (
                    "disconnected".into(),
                    json!({ "message": message, "retryInMs": retry_in.as_millis() as u64 }),
                    Some(Err(PeerError::Offline(message))),
                ),
                FollowEvent::Ended(e) => (
                    "ended".into(),
                    serde_json::to_value(PeerFailure::from(e.clone())).unwrap_or(Value::Null),
                    Some(Err(e)),
                ),
            };
            hub.emit(
                CHAT_EVENT,
                PeerChatEvent {
                    machine_id: home.machine_id.clone(),
                    key: key.clone(),
                    kind,
                    data,
                },
            );
            if let Some(result) = status {
                tokio::spawn(async move {
                    if peer.current(stream) {
                        hub.note(&peer, result.as_ref().map(|_| ())).await;
                    }
                });
            }
        })
    }

    /// Close every tunnel unused for `idle` whose Mac has no stream open.
    pub(crate) async fn close_idle(&self, idle: Duration) {
        self.forget_old_routes();
        let peers: Vec<Arc<Peer>> = locked(&self.peers).values().cloned().collect();
        for peer in peers {
            if !peer.watching() {
                peer.tunnel.close_if_idle(idle).await;
            }
        }
    }

    /// The Mac's one tunnel, when the hub has made it (tests).
    #[cfg(test)]
    pub(crate) fn tunnel(&self, machine_id: &str) -> Option<Arc<Tunnel>> {
        locked(&self.peers)
            .get(machine_id)
            .map(|p| p.tunnel.clone())
    }

    /// Whether the Mac has a running list stream and which chat it follows
    /// (tests).
    #[cfg(test)]
    pub(crate) fn watched(&self, machine_id: &str) -> (bool, Option<String>) {
        locked(&self.peers)
            .get(machine_id)
            .map(|p| {
                (
                    locked(&p.list)
                        .as_ref()
                        .is_some_and(|(_, f)| !f.is_finished()),
                    locked(&p.chat)
                        .as_ref()
                        .filter(|(_, _, f)| !f.is_finished())
                        .map(|(k, _, _)| k.clone()),
                )
            })
            .unwrap_or((false, None))
    }
}

fn message_id(id: &str) -> Result<(), PeerFailure> {
    let ok = !id.is_empty()
        && id.len() <= MESSAGE_ID_MAX
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if ok {
        Ok(())
    } else {
        Err(bad("That message id isn't one Hollow makes."))
    }
}

/// Peer commands answer only Hollow's main window.
pub(crate) fn main_window(webview: &tauri::Webview) -> Result<(), PeerFailure> {
    if webview.label() != "main" {
        return Err(local("Use another Mac's chats from Hollow's main window."));
    }
    Ok(())
}

pub(crate) fn hub(webview: &tauri::Webview) -> Result<Arc<Hub>, PeerFailure> {
    main_window(webview)?;
    let hub = Hub::for_app()?;
    hub.attach(Core::from(tauri::Manager::app_handle(webview).clone()));
    Ok(hub)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// This Mac and the paired Macs with their state; `check` asks each first.
#[tauri::command]
pub async fn peer_machines(
    webview: tauri::Webview,
    check: Option<bool>,
) -> Result<PeerMachinesView, PeerFailure> {
    hub(&webview)?.machines(check.unwrap_or(false)).await
}

/// Pair with the Mac at SSH `alias` (Settings' Pair a Mac).
#[tauri::command]
pub async fn peer_pair(webview: tauri::Webview, alias: String) -> Result<PeerMachine, PeerFailure> {
    let paired = hub(&webview)?.pair(&alias).await;
    if paired.is_ok() {
        // F3: a new Mac pair may be offered "Connect this Mac's context".
        crate::mac_context::after_change();
    }
    paired
}

// `peer_forget` (Forget this Mac) is `commands::chat_move_api`'s: it also
// releases the chats moved to that Mac (M5 PR2), which this module may not.

/// Cancel the pairing in flight (Settings' Cancel on the Pairing step);
/// `true` when one was running. Nothing is kept for it.
#[tauri::command]
pub async fn peer_pair_cancel(webview: tauri::Webview) -> Result<bool, PeerFailure> {
    Ok(hub(&webview)?.cancel_pairing())
}

/// A paired Mac's SSH aliases, in the order to try them.
#[tauri::command]
pub async fn peer_reorder_hosts(
    webview: tauri::Webview,
    machine_id: String,
    hosts: Vec<String>,
) -> Result<PeerMachine, PeerFailure> {
    hub(&webview)?.set_hosts(&machine_id, hosts).await
}

/// Cancel connecting to a paired Mac (its tunnel stays closed until Retry).
#[tauri::command]
pub async fn peer_cancel_connect(
    webview: tauri::Webview,
    machine_id: String,
) -> Result<PeerMachine, PeerFailure> {
    hub(&webview)?.cancel_connect(&machine_id).await
}

/// A paired Mac's chats, newest first (never kept here).
#[tauri::command]
pub async fn peer_sessions(
    webview: tauri::Webview,
    machine_id: String,
) -> Result<PeerSessionsPage, PeerFailure> {
    hub(&webview)?.sessions(&machine_id).await
}

/// One remote chat's turns (never kept here).
#[tauri::command]
pub async fn peer_detail(
    webview: tauri::Webview,
    machine_id: String,
    key: String,
    before: Option<u32>,
    limit: Option<u32>,
) -> Result<PeerChatDetail, PeerFailure> {
    hub(&webview)?
        .detail(&machine_id, &key, before, limit)
        .await
}

/// Follow a Mac's chat list (`key` null) or one remote chat live.
#[tauri::command]
pub async fn peer_watch(
    webview: tauri::Webview,
    machine_id: String,
    key: Option<String>,
) -> Result<(), PeerFailure> {
    hub(&webview)?.watch(&machine_id, key).await
}

/// Stop following a Mac's chat list (`key` null) or that chat.
#[tauri::command]
pub async fn peer_unwatch(
    webview: tauri::Webview,
    machine_id: String,
    key: Option<String>,
) -> Result<(), PeerFailure> {
    hub(&webview)?.unwatch(&machine_id, key.as_deref());
    Ok(())
}

/// Send a remote chat a message, at its home.
#[tauri::command]
pub async fn peer_send(
    webview: tauri::Webview,
    machine_id: String,
    key: String,
    text: String,
    client_message_id: String,
) -> Result<Accepted, PeerFailure> {
    hub(&webview)?
        .send(&machine_id, &key, &text, &client_message_id)
        .await
}

/// M3a follow-up: start the runner COO's first conversation on that Mac
/// (Settings, Devices, COO on this Mac, Start its COO); its key.
#[tauri::command]
pub async fn peer_start_coo(
    webview: tauri::Webview,
    machine_id: String,
) -> Result<String, PeerFailure> {
    hub(&webview)?.start_coo(&machine_id).await
}

/// Stop a remote chat's turn, at its home.
#[tauri::command]
pub async fn peer_stop(
    webview: tauri::Webview,
    machine_id: String,
    key: String,
) -> Result<bool, PeerFailure> {
    hub(&webview)?.stop(&machine_id, &key).await
}

/// Answer a remote chat's card, at its home (this Mac's cards never see it).
#[tauri::command]
pub async fn peer_answer(
    webview: tauri::Webview,
    machine_id: String,
    key: String,
    question_id: String,
    client_message_id: String,
    answers: Map<String, Value>,
) -> Result<Accepted, PeerFailure> {
    hub(&webview)?
        .answer(
            &machine_id,
            &key,
            &question_id,
            &client_message_id,
            &answers,
        )
        .await
}
