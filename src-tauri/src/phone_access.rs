//! The phone chat API (2026-11-04): the Mac side of the Hollow iPhone app.
//! The contract is `docs/PHONE_CHAT_API.md`, with fixtures in
//! `docs/phone-api-fixtures/`.
//!
//! - **Where it listens.** `127.0.0.1:47680` only, while Settings,
//!   Connections → Companion devices → iPhone app is on ([`server`]).
//!   `tailscale serve --bg --https=8443 http://127.0.0.1:47680` publishes it
//!   to the tailnet (Settings' Publish on Tailscale); never Funnel. Off by
//!   default; once on, it starts with Hollow.
//! - **Who may call.** The Host check, the `Tailscale-User-Login` check
//!   against this Mac's own login ([`tailscale::Identity`]; never Funnel,
//!   and no tailnet request without a user), and a device token for
//!   everything but `health` and `pair`. Tokens are kept only as
//!   SHA-256 hashes in `<support>/Companion/phone.json` (0600, never the
//!   vault; [`store`]). Pairing opens an in-memory window with an 8-character
//!   code; revoking a phone closes its event streams.
//! - **What it does** ([`routes`]): lists and reads chats from the session
//!   store, the run queue and question cards; sends as the person's own
//!   composer send; starts a chat as New chat does; stops as Stop does.
//! - **Live answers** ([`live`]): while the iPhone app is on and a phone
//!   is paired, every `chat-event` passes through [`tap`] (from
//!   `chat::emit_event`) into a fold of the window's own reducer, so a phone
//!   follows a running turn word for word, also across a stream swap or a
//!   while in the background; otherwise the tap copies nothing.
//! - **v1.1** (2026-11-05): the Waiting on you list, answering a question
//!   card as the window's card Send does, Runs on through Hollow's own
//!   machinery ([`routes`]), and push notifications ([`push`]) from what the
//!   live worker folds ([`on_chat_event`]), signed with a key Settings
//!   imports into the Keychain.
//! - **v1.2** (2026-11-07): canvases ([`canvases`], contract
//!   `docs/phone-api/canvases.md`): the canvas kit a phone document is built
//!   from, a chat's and a workspace's saved canvases, and an ask-canvas's
//!   answer sent as the window's Send under it.
//! - **The lid** ([`lid`], 2026-11-05): Keep Hollow running with the lid
//!   closed, a root helper installed and removed with macOS's password
//!   prompt, which turns system sleep off only while this Hollow runs with
//!   the iPhone app on and a phone paired (on power, unless On battery too).
//!
//! - **Another Mac** (M2, 2026-11-15, `docs/phone-api/machines.md`): a Mac
//!   pairs through its own window (`hollow serve pair --machine`, or Settings'
//!   [`phone_access_machine_pairing_start`]) and `POST /v1/machines/pair`; its
//!   token reaches only the peer routes ([`routes::ROUTES`]: list and read
//!   chats, send, stop, answer a card, the event stream). The server also
//!   listens, and live views are copied, while a Mac is paired with the
//!   iPhone switch off (a phone's token then gets 503).
//!
//! The Settings commands here are the person's own clicks, in Hollow's main
//! window only. Since L13 (2026-11-10) the COO reaches most of them through
//! the `companion.phone_*` actions, which call the same inner functions
//! without the window check, behind the person's card for every change, and
//! get the status with the pairing code and its QR link left out
//! ([`for_agent`]). The permission modes the window reports for new chats
//! and each chat stay the window's alone (`user-only` in
//! `docs/HOLLOW_APP_COMMAND_MAP.tsv`).

mod app_attest;
mod approvals_api;
pub(crate) mod attachments;
mod calendar_api;
mod canvases;
mod controls;
pub(crate) mod lid;
mod live;
mod live_activity;
mod notes_api;
mod push;
mod routes;
mod search_api;
mod server;
mod store;
mod tailscale;
mod tasks_api;
mod wire;
mod workspaces_api;

#[cfg(test)]
#[path = "phone_access_tests.rs"]
mod tests;

pub(crate) use live::tap;
pub(crate) use store::valid_machine_id;
/// AG8: the stream event kinds a peer Mac reads (`peer_client::DroppedEvents`).
pub(crate) use wire::STREAM_EVENTS;
/// A2: the account snapshot's wire shape (`GET /v1/machine/usage`).
pub(crate) use wire::{AccountUsage, USAGE_ACCOUNTS_MAX};
#[cfg(test)]
pub(crate) use wire::{MachineUsage, USAGE_API};

/// M4: the Macs paired with this one as a server (their `machineId` and
/// name). Empty when the record can't be read.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn paired_machines() -> Vec<(Option<String>, String)> {
    Store::for_app()
        .and_then(|store| store.read().ok())
        .map(|record| {
            record
                .machines
                .into_iter()
                .map(|m| (m.machine_id, m.name))
                .collect()
        })
        .unwrap_or_default()
}

use crate::core_host::Core;
use routes::{Ctx, Fut, Host, WindowModes};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use store::{Pairing, Report, Store};
use tailscale::Identity;

/// Settings reads the iPhone app's status again (a phone paired or was
/// revoked, the switch moved, a wrong code was tried).
pub(crate) const EVENT: &str = "phone-access-changed";
/// How often the server reads this Mac's tailnet identity again.
const IDENTITY_EVERY: Duration = Duration::from_secs(60);
/// Settings reads Tailscale again when its copy is older than this.
const SETTINGS_FRESH: Duration = Duration::from_secs(10);
/// The scheme the iPhone app opens from the Camera.
const PAIR_LINK: &str = "hollow-companion://pair";
/// v1.1: `GET /v1/me` asks for a new runner check when the last one is
/// older than this (Settings' own rule).
const RUNNER_FRESH: chrono::Duration = chrono::Duration::minutes(5);
/// A `.p8` key file, at most (Apple's are about 250 bytes).
const KEY_FILE_MAX: u64 = 16 * 1024;

/// The running app's phone access.
struct App {
    app: Core,
    host: Arc<dyn Host>,
    vault: PathBuf,
    store: Arc<Store>,
    pairing: Arc<Pairing>,
    /// M2: the machine pairing window, apart from the phone's.
    machine_pairing: Arc<Pairing>,
    /// B4: the approvals code window, and the decision challenges and bad
    /// signatures, shared with the server's `Ctx`.
    approval_enroll: Arc<approvals_api::Enroll>,
    approval_decisions: Arc<approvals_api::Decisions>,
    /// M2: the iPhone switch is off (the server may still listen for a Mac).
    phones_off: Arc<AtomicBool>,
    /// `hollow serve`: no window.
    windowless: bool,
    live: Arc<live::Live>,
    identity: Arc<Identity>,
    mac_name: String,
    modes: Arc<WindowModes>,
    server: tokio::sync::Mutex<Listening>,
    serve: Mutex<Option<Result<tailscale::ServeInfo, String>>>,
    /// v1.1: push notifications.
    push: Arc<push::Pusher>,
}

#[derive(Default)]
struct Listening {
    server: Option<server::Server>,
    error: Option<String>,
}

static APP: OnceLock<Arc<App>> = OnceLock::new();

/// Hollow's side effects for a route: the composer's own paths.
struct AppHost {
    app: Core,
    config: Arc<crate::config::Config>,
}

impl Host for AppHost {
    fn control_options(
        &self,
        agent: Option<crate::agent::AgentKind>,
    ) -> Fut<'_, Result<controls::Options, routes::ApiError>> {
        Box::pin(controls::catalog(agent))
    }

    fn send_controlled(
        &self,
        input: crate::commands::ChatSendInput,
        effort: Option<String>,
        existing: bool,
        gate: routes::Gate,
    ) -> Fut<'_, Result<crate::types::SendAccepted, String>> {
        if effort.is_none() {
            return if existing {
                self.send_unless_busy(input, gate)
            } else {
                // A new chat takes the held-send order and runs the gate too:
                // a phone revoked while it waits must not queue the chat.
                Box::pin(async move {
                    let _order = crate::commands::held_order(move || gate()).await?;
                    crate::commands::send_chat_on(Some(&self.app), &self.config, input, None, None)
                        .await
                })
            };
        }
        Box::pin(crate::commands::send_phone_chat(
            &self.app,
            &self.config,
            input,
            effort,
            existing,
            move || gate(),
        ))
    }

    fn accepting(&self) -> Result<(), String> {
        crate::run_pump::accepting()
    }

    fn send_unless_busy(
        &self,
        input: crate::commands::ChatSendInput,
        gate: routes::Gate,
    ) -> Fut<'_, Result<crate::types::SendAccepted, String>> {
        Box::pin(crate::commands::send_unless_busy(
            &self.app,
            &self.config,
            input,
            move || gate(),
        ))
    }

    fn stop(&self, key: String) -> Fut<'_, Result<bool, String>> {
        Box::pin(crate::chat::cancel(
            self.app.clone(),
            self.config.vault_path.clone(),
            Some(key),
        ))
    }

    fn agent_for(
        &self,
        asked: Option<crate::agent::AgentKind>,
    ) -> Result<crate::agent::AgentKind, String> {
        crate::agent_settings::resolve(asked.map(|a| a.as_str()), None).map(|(kind, _)| kind)
    }

    fn announce(&self, key: &str, workspace_id: Option<&str>, cwd: &str) {
        let _ = self.app.emit(
            crate::app_sessions::STARTED_EVENT,
            serde_json::json!({
                "sessionKey": key,
                "workspaceId": workspace_id,
                "cwd": cwd,
                "startedBy": "phone",
            }),
        );
    }

    fn changed(&self) {
        let _ = self.app.emit(EVENT, ());
    }

    fn want_modes(&self) {
        let _ = self.app.emit_to("main", routes::MODES_WANTED, ());
    }

    fn runner(&self) -> Option<wire::Runner> {
        let host = crate::remote_session::configured_runner()?;
        let status = crate::remote_status::status(false);
        let stale = status
            .checked_at
            .as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .is_none_or(|at| chrono::Utc::now().signed_duration_since(at) > RUNNER_FRESH);
        if stale {
            check_runner_soon();
        }
        Some(wire::Runner {
            name: status
                .machine
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(host),
            available: status.state == "ready",
        })
    }

    fn place(&self) -> crate::remote_session::PlaceReads {
        crate::remote_session::PlaceReads::LIVE
    }

    fn card_changed(&self, card: &crate::types::ChatInteraction) {
        crate::chat::emit_event(&self.app, crate::interactions::event_payload(card));
    }

    fn label_tasks(&self, tasks: &mut [crate::types::Task]) {
        if let Some(state) = self.app.try_state::<crate::commands::AppState>() {
            crate::commands::sources_api::label_tasks_for(state, tasks);
        }
    }
}

/// A runner check under way for a phone's `/v1/me`.
static RUNNER_CHECKING: AtomicBool = AtomicBool::new(false);

/// Check the runner in the background (one ssh call, read-only, as
/// Settings' check), unless one is under way: a route never waits for it.
fn check_runner_soon() {
    if RUNNER_CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("phone-runner-check".into())
        .spawn(|| {
            let _ = crate::remote_status::status(true);
            RUNNER_CHECKING.store(false, Ordering::SeqCst);
        });
    if spawned.is_err() {
        RUNNER_CHECKING.store(false, Ordering::SeqCst);
    }
}

/// v1.1: what else a chat event the live worker folded starts. An answer
/// turn a phone queued that ends before it started brings its card back
/// (`live::AnswerTurns`); and push notifications ([`push::Pusher::observe`]).
fn on_chat_event(state: &Arc<App>, event: &serde_json::Value) {
    let kind = event.get("type").and_then(serde_json::Value::as_str);
    let queue_id = event.get("queueId").and_then(serde_json::Value::as_i64);
    match (kind, queue_id) {
        (Some("runStarted"), Some(id)) => state.live.answers.started(id),
        (Some("done"), Some(id)) => {
            if let Some(card) = state.live.answers.ended(id) {
                let state = state.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Ok(reopened) = crate::interactions::reopen(&state.vault, &card) {
                        state.host.card_changed(&reopened);
                        state.live.touched(&reopened.session_key);
                    }
                });
            }
        }
        _ => {}
    }
    state.push.observe(event);
}

/// The Mac's name as Sharing settings shows it (`scutil --get
/// ComputerName`), else its host name.
pub(crate) fn mac_name() -> String {
    let named = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|name| !name.is_empty());
    named
        .or_else(|| {
            std::process::Command::new("/bin/hostname")
                .arg("-s")
                .output()
                .ok()
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| "Mac".to_string())
}

/// Start phone access for `vault` (app setup, the run queue's owner only):
/// the live buffer always, the server when Settings left it on.
pub(crate) fn start(app: impl Into<Core>, vault: PathBuf) {
    let app = app.into();
    let Some(store) = Store::for_app() else {
        eprintln!("[phone] no support folder; the iPhone app is off");
        return;
    };
    let config = app
        .try_state::<crate::commands::AppState>()
        .map(|state| state.config.clone());
    let Some(config) = config else {
        return;
    };
    let live = live::start(vault.clone());
    let windowless = app.gui().is_none();
    let host: Arc<dyn Host> = Arc::new(AppHost {
        app: app.clone(),
        config,
    });
    let store = Arc::new(store);
    let (pusher, jobs) = push::Pusher::new(
        vault.clone(),
        store.clone(),
        live.clone(),
        Arc::new(push::KeychainKey),
        push::Endpoints::default(),
        push::wall_clock(),
    );
    tauri::async_runtime::spawn(pusher.clone().drain(jobs));
    let state = Arc::new(App {
        app,
        host,
        vault,
        store,
        pairing: Arc::new(Pairing::default()),
        machine_pairing: Arc::new(Pairing::default()),
        approval_enroll: Arc::new(approvals_api::Enroll::default()),
        approval_decisions: Arc::new(approvals_api::Decisions::default()),
        phones_off: Arc::new(AtomicBool::new(true)),
        windowless,
        live,
        identity: Arc::new(Identity::default()),
        mac_name: mac_name(),
        // No window in `hollow serve`: the record's modes, never an ask.
        modes: Arc::new(if windowless {
            WindowModes::without_window()
        } else {
            WindowModes::default()
        }),
        server: tokio::sync::Mutex::new(Listening::default()),
        serve: Mutex::new(None),
        push: pusher,
    });
    if APP.set(state.clone()).is_err() {
        return;
    }
    {
        let observed = state.clone();
        state
            .live
            .observe(Box::new(move |event| on_chat_event(&observed, event)));
    }
    // B4: an approval card shown or resolved: push it, tell the streams.
    {
        let observed = state.clone();
        crate::approvals::observe_cards(move |id, shown| {
            observed.live.approvals_changed();
            if shown {
                observed.push.approval_opened(id);
            }
        });
    }
    lid::start(state.store.clone());
    sync_phone_wait(&state.store);
    tauri::async_runtime::spawn(async move {
        // On with a phone paired, or a Mac paired: live views from the
        // first turn on.
        state.live.follow(|| state.store.viewers_ready());
        match state.store.read() {
            Ok(record) => {
                // The mode for new chats the window reported before this
                // launch, until it says it again.
                if let Some(mode) = record.new_chat_mode.filter(|m| known_mode(m)) {
                    state.modes.restore_new_chat(routes::NewChatMode {
                        mode,
                        agent: record.new_chat_agent,
                    });
                }
                state.phones_off.store(!record.enabled, Ordering::SeqCst);
                if record.enabled || !record.machines.is_empty() {
                    listen(&state).await;
                }
            }
            Err(error) => eprintln!("[phone] {error}"),
        }
    });
}

/// The loopback port `hollow serve --phone-port` asked for (M1 PR 2), so a
/// serve beside this Mac's own Hollow can listen; the app never sets it.
static PORT_OVERRIDE: OnceLock<u16> = OnceLock::new();

pub(crate) fn set_port(port: u16) {
    let _ = PORT_OVERRIDE.set(port);
}

fn listen_port() -> u16 {
    PORT_OVERRIDE.get().copied().unwrap_or(server::PORT)
}

/// One of Hollow's permission modes (`app_mcp::PERMISSION_MODES`).
fn known_mode(mode: &str) -> bool {
    crate::app_mcp::PERMISSION_MODES.contains(&mode)
}

fn app() -> Result<Arc<App>, String> {
    APP.get().cloned().ok_or_else(|| {
        "The iPhone app isn't available in this copy of Hollow (only the Hollow that runs this vault's chats serves it).".to_string()
    })
}

/// Start the server (switch on, or at launch). A port in use is kept as
/// the error Settings shows.
async fn listen(state: &Arc<App>) {
    let mut listening = state.server.lock().await;
    if listening.server.is_some() {
        return;
    }
    let mut ctx = Ctx::new(
        state.vault.clone(),
        state.store.clone(),
        state.pairing.clone(),
        state.live.clone(),
        state.host.clone(),
        state.mac_name.clone(),
        state.modes.clone(),
        state.push.activities(),
    );
    ctx.machine_pairing = state.machine_pairing.clone();
    ctx.approval_enroll = state.approval_enroll.clone();
    ctx.approval_decisions = state.approval_decisions.clone();
    ctx.machine = routes::MachineSource::App {
        headless: state.windowless,
    };
    ctx.phones_off = state.phones_off.clone();
    let ctx = Arc::new(ctx);
    let bound = server::Server::bind(listen_port()).await;
    match bound.and_then(|listener| server::Server::start(listener, ctx, state.identity.clone())) {
        Ok(server) => {
            eprintln!("[phone] the iPhone app's API listens on {}", server.addr);
            let mut closed = server.subscribe();
            listening.server = Some(server);
            listening.error = None;
            let identity = state.identity.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let _ = identity.refresh().await;
                    tokio::select! {
                        _ = tokio::time::sleep(IDENTITY_EVERY) => {}
                        _ = closed.changed() => return,
                    }
                }
            });
        }
        Err(error) => {
            eprintln!("[phone] {error}");
            listening.error = Some(error);
        }
    }
}

/// Stop the server and close every stream and the pairing window.
async fn quiet(state: &Arc<App>) {
    let mut listening = state.server.lock().await;
    if let Some(server) = listening.server.take() {
        server.stop();
    }
    listening.error = None;
    state.live.close_all();
    state.pairing.close();
    state.machine_pairing.close();
}

/// M2: after a revoke, whether the server has nobody left to listen for:
/// the iPhone switch is off, no Mac is paired and no machine window is open.
pub(crate) fn nothing_to_listen_for(
    phones_off: bool,
    machines_paired: bool,
    machine_window_open: bool,
) -> bool {
    phones_off && !machines_paired && !machine_window_open
}

/// M2: the iPhone switch went off while a Mac stays paired: the server keeps
/// listening for the Mac, and every phone's streams and the phone pairing
/// window close (a phone's token gets 503 until the switch is on).
fn quiet_phones(state: &Arc<App>) {
    state.pairing.close();
    if let Ok(record) = state.store.read() {
        for device in &record.devices {
            state.live.close_device(&device.device_id);
        }
    }
}

// ── Settings ─────────────────────────────────────────────────────────────

/// A paired phone as Settings lists it (never its token's hash).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneDevice {
    pub device_id: String,
    pub name: String,
    pub model: Option<String>,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
    /// v1.1: the phone registered for push notifications.
    pub push: bool,
    /// B4: the phone answers approval cards with its keys (Face ID on Allow).
    pub approvals: Option<PhoneApprovals>,
}

/// B4: a phone's approval keys, as Settings shows them.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneApprovals {
    /// The 8 hex digits both screens show, `a1b2 c3d4`.
    pub fingerprint: String,
    pub enrolled_at: String,
}

/// B4: the open approvals code window, for one phone (Settings, Devices,
/// Companion devices, the phone's row, Approvals from this iPhone).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneApprovalsCode {
    pub device_id: String,
    /// `XXXX-XXXX`, the code the phone types.
    pub code: String,
    pub expires_at: String,
}

/// M2: another Mac paired with this one, as Settings lists it (never its
/// token's hash).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneMachine {
    /// `mc-` and 8 hex digits: what Revoke takes.
    pub device_id: String,
    pub machine_id: Option<String>,
    pub name: String,
    pub paired_at: String,
    pub last_seen_at: Option<String>,
}

/// M2: the open machine pairing window (Let another Mac connect).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachinePairing {
    /// `XXXX-XXXX`.
    pub code: String,
    pub expires_at: String,
}

/// The open pairing window as Settings shows it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhonePairing {
    /// `XXXX-XXXX`.
    pub code: String,
    pub expires_at: String,
    /// The pairing link the QR code holds; `None` until Tailscale names this
    /// Mac (the app also takes the address and code typed by hand).
    pub link: Option<String>,
    /// The link as an SVG QR code.
    pub qr_svg: Option<String>,
}

/// Tailscale as Settings shows it.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneTailscale {
    pub installed: bool,
    pub running: bool,
    pub dns_name: Option<String>,
    pub login: Option<String>,
    /// `tailscale serve` publishes the phone API on 8443.
    pub published: bool,
    /// 8443 is funnelled to the internet (Hollow never does that).
    pub funnel: bool,
    /// 8443 serves something else (its target).
    pub other: Option<String>,
    /// Tailnet ports `tailscale serve --tcp` forwards straight to the phone
    /// API's port in the saved serve config, which passes on nothing of the
    /// caller, so Hollow can't tell its requests from this Mac's own. A
    /// problem; Stop publishing removes them.
    pub tcp_forwards: Vec<u16>,
    /// The same from a `tailscale serve` still running in a terminal: a
    /// problem only Control-C there ends, which Settings says.
    pub foreground_forwards: Vec<u16>,
    /// The same in a Tailscale service's config: a problem Settings shows
    /// with the command that removes it.
    pub service_forwards: Vec<PhoneServiceForward>,
    pub error: Option<String>,
}

/// A raw TCP forward to the phone API in a Tailscale service's config.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneServiceForward {
    /// `svc:<name>`.
    pub service: String,
    pub port: u16,
    /// `tailscale serve --service=<name> --tcp=<port> off` (the CLI's full
    /// path when no folder of the person's PATH holds `tailscale`), which
    /// the person runs (`tailscale::ServiceForward::stop`); null for a name
    /// that isn't `svc:` and lower-case letters, digits and hyphens, which
    /// Settings says in words how to remove.
    pub stop: Option<String>,
}

/// Settings, Devices, Companion devices, iPhone app.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneAccessStatus {
    /// This Hollow serves the iPhone app (the one running this vault's chats).
    pub available: bool,
    pub unavailable_reason: Option<String>,
    pub enabled: bool,
    pub listening: bool,
    /// Why it isn't listening although on (the port is taken).
    pub bind_error: Option<String>,
    pub port: u16,
    /// The record of paired phones couldn't be read.
    pub record_error: Option<String>,
    pub tailscale: PhoneTailscale,
    /// `https://<MagicDNS name>:8443`, once Tailscale names this Mac.
    pub base_url: Option<String>,
    pub mac_name: String,
    pub devices: Vec<PhoneDevice>,
    pub pairing: Option<PhonePairing>,
    /// v1.1: push notifications (`None` when this Hollow doesn't serve the
    /// iPhone app).
    pub push: Option<push::PhonePush>,
    /// M2: the other Macs paired with this one (the server side).
    pub machines: Vec<PhoneMachine>,
    /// B4: the open approvals code window, if any (never for an agent).
    pub approvals_code: Option<PhoneApprovalsCode>,
    /// B4: seconds a card waits while a phone has approvals on.
    pub approval_wait_secs: u64,
    /// B4: phones whose approval keys were dropped for bad signatures since
    /// Hollow started (device id, RFC 3339).
    pub approvals_dropped: Vec<(String, String)>,
    /// M2: the open machine pairing window (never for an agent).
    pub machine_pairing: Option<MachinePairing>,
}

/// The pairing link: `hollow-companion://pair?url=…&code=…&mac=…`.
fn pairing_link(base_url: &str, code: &str, mac: &str) -> String {
    format!(
        "{PAIR_LINK}?url={}&code={}&mac={}",
        urlencoding::encode(base_url),
        urlencoding::encode(code),
        urlencoding::encode(mac)
    )
}

/// `text` as an SVG QR code (dark modules on white, with its quiet zone).
fn qr_svg(text: &str) -> Option<String> {
    let code =
        qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color<'_>>()
            .min_dimensions(240, 240)
            .dark_color(qrcode::render::svg::Color("#000000"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
    )
}

/// The base URL the iPhone app uses.
fn base_url(dns_name: Option<&str>) -> Option<String> {
    dns_name.map(|name| format!("https://{name}:{}", tailscale::SERVE_PORT))
}

async fn read_tailscale(state: &Arc<App>, fresh: bool) -> PhoneTailscale {
    let installed = tailscale::cli().is_some();
    if installed && (fresh || state.identity.age().is_none_or(|age| age > SETTINGS_FRESH)) {
        let _ = state.identity.refresh().await;
        let serve = tailscale::serve_status(server::PORT).await;
        *state.serve.lock().unwrap_or_else(|e| e.into_inner()) = Some(serve);
    }
    let me = state.identity.current();
    let serve = state
        .serve
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let (serve, error) = match serve {
        Some(Ok(info)) => (info, None),
        Some(Err(error)) => (tailscale::ServeInfo::default(), Some(error)),
        None => (tailscale::ServeInfo::default(), None),
    };
    // How the person's terminal runs Tailscale, for a service forward's
    // command: their login shell's PATH once read, else Hollow's own.
    let path: Vec<PathBuf> = match crate::shell_path::folders() {
        Some(dirs) => dirs.iter().map(PathBuf::from).collect(),
        None => std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default(),
    };
    let tool = tailscale::terminal_tool(tailscale::cli().as_deref(), &path);
    PhoneTailscale {
        installed,
        running: me.running,
        dns_name: me.dns_name,
        login: me.login,
        published: serve.published,
        funnel: serve.funnel,
        other: serve.other,
        tcp_forwards: serve.tcp_forwards,
        foreground_forwards: serve.foreground_forwards,
        service_forwards: serve
            .service_forwards
            .iter()
            .map(|forward| PhoneServiceForward {
                service: forward.service.clone(),
                port: forward.port,
                stop: forward.stop(&tool),
            })
            .collect(),
        error,
    }
}

/// Who a status is for. Settings (the person opened the screen) moves a
/// damaged `phone.json` aside; an agent's read never changes anything, so
/// it leaves a damaged file where it is (`Store::read`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Settings,
    Agent,
}

async fn status_of(state: &Arc<App>, fresh_tailscale: bool, view: View) -> PhoneAccessStatus {
    let record = {
        let store = state.store.clone();
        tauri::async_runtime::spawn_blocking(move || match view {
            View::Settings => store.read_for_settings(),
            View::Agent => store.read(),
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    };
    let (enabled, devices, machines, approval_wait, record_error) = match record {
        Ok(record) => (
            record.enabled,
            record.devices,
            record.machines,
            record.approval_wait_secs,
            None,
        ),
        Err(error) => (false, Vec::new(), Vec::new(), None, Some(error)),
    };
    let (listening, bind_error) = {
        let listening = state.server.lock().await;
        (listening.server.is_some(), listening.error.clone())
    };
    let tailscale = read_tailscale(state, fresh_tailscale).await;
    let base_url = base_url(tailscale.dns_name.as_deref());
    let pairing = state.pairing.current(chrono::Utc::now()).map(|window| {
        let link = base_url
            .as_deref()
            .map(|url| pairing_link(url, &window.code, &state.mac_name));
        PhonePairing {
            code: window.display,
            expires_at: window
                .expires
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            qr_svg: link.as_deref().and_then(qr_svg),
            link,
        }
    });
    PhoneAccessStatus {
        available: true,
        unavailable_reason: None,
        enabled,
        listening,
        bind_error,
        port: server::PORT,
        record_error,
        tailscale,
        base_url,
        mac_name: state.mac_name.clone(),
        devices: devices
            .into_iter()
            .map(|d| PhoneDevice {
                device_id: d.device_id,
                name: d.name,
                model: d.model,
                paired_at: d.paired_at,
                last_seen_at: d.last_seen_at,
                push: d.push.is_some(),
                approvals: d.approval_keys.map(|keys| PhoneApprovals {
                    fingerprint: keys.fingerprint,
                    enrolled_at: keys.enrolled_at,
                }),
            })
            .collect(),
        pairing,
        push: Some({
            let pusher = state.push.clone();
            tauri::async_runtime::spawn_blocking(move || pusher.status())
                .await
                .unwrap_or_default()
        }),
        machines: machines
            .into_iter()
            .map(|d| PhoneMachine {
                device_id: d.device_id,
                machine_id: d.machine_id,
                name: d.name,
                paired_at: d.paired_at,
                last_seen_at: d.last_seen_at,
            })
            .collect(),
        machine_pairing: state
            .machine_pairing
            .current(chrono::Utc::now())
            .map(|window| MachinePairing {
                code: window.display,
                expires_at: window
                    .expires
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            }),
        approvals_code: state
            .approval_enroll
            .current(chrono::Utc::now())
            .map(|(window, _)| PhoneApprovalsCode {
                device_id: window.device_id,
                code: window.display,
                expires_at: window
                    .expires
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            }),
        approval_wait_secs: approval_wait.unwrap_or(crate::approvals::PHONE_WAIT_DEFAULT_SECS),
        approvals_dropped: state.approval_decisions.dropped_list(),
    }
}

/// Refuses a call from any window but Hollow's main one.
fn main_window(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("Change the iPhone app in Hollow's main window.".into());
    }
    Ok(())
}

/// The iPhone app's status, the open pairing code and its QR link included:
/// the window's only (an agent gets [`for_agent`]'s). `fresh` (Settings'
/// Check again) reads Tailscale now rather than its copy from the last ten
/// seconds.
pub(crate) async fn status_now(fresh: bool, view: View) -> PhoneAccessStatus {
    match APP.get().cloned() {
        Some(state) => status_of(&state, fresh, view).await,
        None => PhoneAccessStatus {
            available: false,
            unavailable_reason: app().err(),
            port: server::PORT,
            ..PhoneAccessStatus::default()
        },
    }
}

/// The iPhone app's status for Settings (the open pairing code included,
/// so never for an agent).
#[tauri::command]
pub async fn phone_access_status(
    webview: tauri::Webview,
    fresh: Option<bool>,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    Ok(status_now(fresh.unwrap_or(false), View::Settings).await)
}

/// The iPhone app switch (the person's click): on starts the server now and
/// at every launch; off stops it, closes every stream and the pairing
/// window.
#[tauri::command]
pub async fn phone_access_set_enabled(
    webview: tauri::Webview,
    enabled: bool,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    set_enabled(enabled, View::Settings).await
}

pub(crate) async fn set_enabled(enabled: bool, view: View) -> Result<PhoneAccessStatus, String> {
    let state = app()?;
    {
        let store = state.store.clone();
        tauri::async_runtime::spawn_blocking(move || store.set_enabled(enabled))
            .await
            .map_err(|e| e.to_string())??;
    }
    state.phones_off.store(!enabled, Ordering::SeqCst);
    sync_phone_wait(&state.store);
    // Off ends every live view unless a Mac is paired; on with a phone
    // paired starts copying.
    state.live.follow(|| state.store.viewers_ready());
    // Off lets the Mac sleep with its lid closed again.
    lid::sync_now();
    if enabled {
        listen(&state).await;
    } else if state.store.machines_paired() {
        quiet_phones(&state);
    } else {
        quiet(&state).await;
    }
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, true, view).await)
}

/// Open a pairing window (the person's click): a new code for 10 minutes or
/// one use, closing any open one.
#[tauri::command]
pub async fn phone_access_pairing_start(
    webview: tauri::Webview,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    pairing_start(View::Settings).await
}

pub(crate) async fn pairing_start(view: View) -> Result<PhoneAccessStatus, String> {
    let state = app()?;
    // M2: the server may listen for a paired Mac with the switch off.
    if state.server.lock().await.server.is_none() || state.phones_off.load(Ordering::SeqCst) {
        return Err("Turn on the iPhone app first, so the iPhone can reach this Mac.".into());
    }
    state.pairing.open(chrono::Utc::now());
    Ok(status_of(&state, true, view).await)
}

#[tauri::command]
pub async fn phone_access_pairing_cancel(
    webview: tauri::Webview,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    pairing_cancel(View::Settings).await
}

pub(crate) async fn pairing_cancel(view: View) -> Result<PhoneAccessStatus, String> {
    let state = app()?;
    state.pairing.close();
    Ok(status_of(&state, false, view).await)
}

/// B4: open the approvals code window for one phone (Settings, Devices,
/// Companion devices, the phone's row, Approvals from this iPhone, Turn on):
/// an 8-character code for 10 minutes or one use, closing any other approvals
/// window. The phone types it to enroll its keys.
#[tauri::command]
pub async fn phone_access_approvals_start(
    webview: tauri::Webview,
    device_id: String,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    let state = app()?;
    let id = device_id.trim().to_string();
    let known = {
        let store = state.store.clone();
        let id = id.clone();
        tauri::async_runtime::spawn_blocking(move || store.read())
            .await
            .map_err(|e| e.to_string())??
            .devices
            .iter()
            .any(|d| d.device_id == id)
    };
    if !known {
        return Err("That iPhone isn't paired with this Mac anymore.".into());
    }
    state.approval_decisions.clear_dropped(&id);
    state.approval_enroll.open(&id, chrono::Utc::now());
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, false, View::Settings).await)
}

/// B4: close the approvals code window (the person's click).
#[tauri::command]
pub async fn phone_access_approvals_cancel(
    webview: tauri::Webview,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    let state = app()?;
    state.approval_enroll.close();
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, false, View::Settings).await)
}

/// B4: turn approvals off for one phone from the Mac (the person's click):
/// its keys are dropped, and its next answer gets `approval_key_required`.
#[tauri::command]
pub async fn phone_access_approvals_off(
    webview: tauri::Webview,
    device_id: String,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    let state = app()?;
    let id = device_id.trim().to_string();
    {
        let store = state.store.clone();
        let id = id.clone();
        tauri::async_runtime::spawn_blocking(move || store.set_approval_keys(&id, None))
            .await
            .map_err(|e| e.to_string())??;
    }
    state.approval_decisions.forget(&id);
    sync_phone_wait(&state.store);
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, false, View::Settings).await)
}

/// B4: how long a card waits while a phone has approvals on (the person's
/// choice beside the toggle), 2 to 15 minutes.
#[tauri::command]
pub async fn phone_access_approvals_wait(
    webview: tauri::Webview,
    seconds: u64,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    let state = app()?;
    {
        let store = state.store.clone();
        tauri::async_runtime::spawn_blocking(move || store.set_approval_wait(seconds))
            .await
            .map_err(|e| e.to_string())??;
    }
    sync_phone_wait(&state.store);
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, false, View::Settings).await)
}

/// The wait cards get while any phone has approvals on; `None` when none does.
pub(crate) fn phone_wait_of(record: &store::Record) -> Option<u64> {
    // The iPhone switch off means no phone can reach this Mac to answer.
    (record.enabled && record.devices.iter().any(|d| d.approval_keys.is_some())).then(|| {
        record
            .approval_wait_secs
            .unwrap_or(crate::approvals::PHONE_WAIT_DEFAULT_SECS)
    })
}

/// Tell the approval broker the phone wait (B4): after phones gain or lose
/// approval keys, a revoke, the wait setting, and at start. Tests leave the
/// process-wide wait alone.
pub(crate) fn sync_phone_wait(store: &Store) {
    #[cfg(not(test))]
    crate::approvals::set_phone_wait(store.read().ok().as_ref().and_then(phone_wait_of));
    #[cfg(test)]
    let _ = store;
}

/// The phone API changed which phones have approval keys.
pub(crate) fn approval_keys_changed(ctx: &routes::Ctx) {
    sync_phone_wait(&ctx.store);
}

/// M2: open the machine pairing window (Settings' Let another Mac connect,
/// the person's click): a code for 10 minutes or one use, apart from the
/// phone's, that only `POST /v1/machines/pair` takes. The server starts
/// listening if it wasn't (a Mac pairs with the iPhone switch off too).
#[tauri::command]
pub async fn phone_access_machine_pairing_start(
    webview: tauri::Webview,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    let state = app()?;
    machine_pairing_open(&state).await?;
    Ok(status_of(&state, false, View::Settings).await)
}

/// Close the machine pairing window (the person's click).
#[tauri::command]
pub async fn phone_access_machine_pairing_cancel(
    webview: tauri::Webview,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    let state = app()?;
    state.machine_pairing.close();
    Ok(status_of(&state, false, View::Settings).await)
}

/// Open the machine window once the server listens: `(XXXX-XXXX, closes at)`.
async fn machine_pairing_open(state: &Arc<App>) -> Result<(String, String), String> {
    listen(state).await;
    {
        let listening = state.server.lock().await;
        if listening.server.is_none() {
            return Err(listening.error.clone().unwrap_or_else(|| {
                "Hollow's link for other Macs could not listen on this Mac.".to_string()
            }));
        }
    }
    let window = state.machine_pairing.open(chrono::Utc::now());
    let _ = state.app.emit(EVENT, ());
    Ok((
        window.display,
        window
            .expires
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    ))
}

/// Remove a paired phone (the person's click): its next request gets 401
/// and its event streams close now.
#[tauri::command]
pub async fn phone_access_revoke(
    webview: tauri::Webview,
    device_id: String,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    revoke(&device_id, View::Settings).await
}

pub(crate) async fn revoke(device_id: &str, view: View) -> Result<PhoneAccessStatus, String> {
    let state = app()?;
    let id = device_id.trim().to_string();
    {
        let store = state.store.clone();
        let id = id.clone();
        tauri::async_runtime::spawn_blocking(move || store.revoke(&id))
            .await
            .map_err(|e| e.to_string())??;
    }
    state.live.close_device(&id);
    tasks_api::forget_device(&id);
    // B4: its decision challenges go with it, and the wait follows who is left.
    state.approval_decisions.forget(&id);
    sync_phone_wait(&state.store);
    // v1.2: its Live Activity tokens go with it.
    state.push.activities().forget(&id, None);
    // The last phone (or Mac) revoked ends every live view.
    state.live.follow(|| state.store.viewers_ready());
    // ...and lets the Mac sleep with its lid closed again.
    lid::sync_now();
    // M2: the last Mac revoked with the iPhone switch off: nothing is left
    // to listen for, unless a machine window is open (a Mac about to pair).
    if nothing_to_listen_for(
        state.phones_off.load(Ordering::SeqCst),
        state.store.machines_paired(),
        state.machine_pairing.current(chrono::Utc::now()).is_some(),
    ) {
        quiet(&state).await;
    }
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, false, view).await)
}

/// Publish on Tailscale (the person's click): `tailscale serve --bg
/// --https=8443 http://127.0.0.1:47680`, tailnet only.
#[tauri::command]
pub async fn phone_access_publish(webview: tauri::Webview) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    publish(View::Settings).await
}

pub(crate) async fn publish(view: View) -> Result<PhoneAccessStatus, String> {
    let state = app()?;
    tailscale::publish(server::PORT).await?;
    Ok(status_of(&state, true, view).await)
}

/// Stop publishing (the person's click): `--tcp=<port> off` for each raw
/// TCP forward to the phone API in the saved serve config, then `tailscale
/// serve --https=8443 --set-path=/ off` when 8443's `/` serves Hollow, as
/// `tailscale serve status` reads now, each tried even when an earlier one
/// was refused, and none after one times out (`tailscale::unpublish`).
/// Another app's 8443, or another mount on it, stays, and so does
/// everything when the status can't be read. A foreground or service
/// forward stays: Settings says how to stop it.
#[tauri::command]
pub async fn phone_access_unpublish(webview: tauri::Webview) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    unpublish(View::Settings).await
}

pub(crate) async fn unpublish(view: View) -> Result<PhoneAccessStatus, String> {
    let state = app()?;
    tailscale::unpublish(tailscale::serve_status(server::PORT).await).await?;
    Ok(status_of(&state, true, view).await)
}

/// The permission mode Settings, Chat gives new chats, as the window
/// holds it: a chat a phone starts begins in it, as New chat would. The
/// window says it when the person changes it and when Companion devices
/// opens; the record keeps it across restarts, so launch makes no call.
#[tauri::command]
pub async fn phone_access_new_chat_mode(
    webview: tauri::Webview,
    mode: String,
    agent: Option<String>,
) -> Result<(), String> {
    main_window(&webview)?;
    let mode = mode.trim().to_string();
    if !known_mode(&mode) {
        return Err(format!("{mode} isn't a permission mode."));
    }
    // The agent the mode is for (the window's preferred agent, Q-R3); an
    // unknown name is dropped, so the mode reads the most restrictive way.
    let agent = agent
        .as_deref()
        .and_then(crate::agent::AgentKind::from_opt)
        .map(|kind| kind.as_str().to_string());
    let Some(state) = APP.get().cloned() else {
        return Ok(());
    };
    state.modes.report_new_chat(routes::NewChatMode {
        mode: mode.clone(),
        agent: agent.clone(),
    });
    let store = state.store.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        store.set_new_chat_mode(&mode, agent.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;
    if report == Report::Unused {
        state.modes.fell_behind();
    }
    Ok(())
}

/// Each chat's permission mode as the person chose it in the window
/// (`hollow.chat.modes.v1`, `[key, mode]` pairs, oldest first): a phone's
/// send runs in the mode the window shows for that chat. The window says a
/// choice when the person makes one (a new chat's first send included), and
/// its whole record (`replace`) when the one it keeps on disk changed
/// wholesale, when Companion devices opens and when Hollow asks
/// (`routes::MODES_WANTED`, after a launch once a phone request relies on
/// the copy: at once and every 250 ms for 30 seconds, until the window
/// answers, `routes::WindowModes::settle`); `plan` is the window's
/// fallback for chats it has no choice for (its storage was full). The
/// record keeps them across restarts, so launch makes no call.
#[tauri::command]
pub async fn phone_access_chat_modes(
    webview: tauri::Webview,
    choices: Vec<(String, String)>,
    replace: bool,
    plan: bool,
) -> Result<(), String> {
    main_window(&webview)?;
    let choices = chat_mode_rows(choices);
    let Some(state) = APP.get().cloned() else {
        return Ok(());
    };
    let store = state.store.clone();
    let report =
        tauri::async_runtime::spawn_blocking(move || store.set_chat_modes(&choices, replace, plan))
            .await
            .map_err(|e| e.to_string())??;
    // Unwritten (the switch off, nothing paired): the next phone request
    // asks for the whole record again.
    match report {
        Report::Unused => state.modes.fell_behind(),
        Report::Kept | Report::Same if replace => state.modes.heard_whole(),
        Report::Kept | Report::Same => {}
    }
    Ok(())
}

/// The window's choices Hollow keeps: a Hollow chat key and a mode its
/// picker offers; anything else is left out.
fn chat_mode_rows(choices: Vec<(String, String)>) -> Vec<(String, String)> {
    choices
        .into_iter()
        .filter(|(key, mode)| {
            crate::agent_sessions::is_safe_session_key(key)
                && routes::WINDOW_MODES.contains(&mode.as_str())
        })
        .collect()
}

// ── v1.1: push notifications ─────────────────────────────────────────────

/// Import the APNs key (the person's click, Settings, Devices, Companion devices,
/// iPhone app → Import push key): the `.p8` file the person picked, its Key
/// ID (else the one in Apple's file name, `AuthKey_<Key ID>.p8`) and the
/// team id (default `TEAM000000`). Checked, then kept only in Hollow's
/// Keychain item; the file stays where it is, and its text is never logged
/// or returned.
#[tauri::command]
pub async fn phone_push_import_key(
    webview: tauri::Webview,
    path: String,
    key_id: Option<String>,
    team_id: Option<String>,
) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    push_import_key(path, key_id, team_id, View::Settings).await
}

/// The key file an import reads, checked: `(pem, Key ID, team id)`. Its
/// errors never quote the file, so a wrong path (any file at all) tells
/// nobody what it holds.
pub(crate) fn read_key_file(
    path: &str,
    key_id: Option<String>,
    team_id: Option<String>,
) -> Result<(String, String, String), String> {
    let path = PathBuf::from(path.trim());
    let size = std::fs::metadata(&path)
        .map_err(|_| "Hollow couldn't open that file.".to_string())?
        .len();
    if size > KEY_FILE_MAX {
        return Err("That file is too large to be an APNs key (.p8).".to_string());
    }
    let pem = std::fs::read_to_string(&path)
        .map_err(|_| "Hollow couldn't read that file.".to_string())?;
    let key_id = key_id
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .or_else(|| push::key_id_from_file_name(&path))
        .ok_or_else(|| {
            "Enter the key's Key ID (Apple shows it beside the key, and names the file AuthKey_<Key ID>.p8).".to_string()
        })?;
    let team_id = team_id
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| push::TEAM_ID.to_string());
    // Checked before the app is asked, so a file that isn't a key says so
    // the same way wherever the import comes from.
    push::PushKey::parse(&pem, &key_id, &team_id)?;
    Ok((pem, key_id, team_id))
}

/// Import the key from `path` into Hollow's Keychain item; the file stays
/// where it is, and its text goes nowhere else.
pub(crate) async fn push_import_key(
    path: String,
    key_id: Option<String>,
    team_id: Option<String>,
    view: View,
) -> Result<PhoneAccessStatus, String> {
    let (pem, key_id, team_id) =
        tauri::async_runtime::spawn_blocking(move || read_key_file(&path, key_id, team_id))
            .await
            .map_err(|e| e.to_string())??;
    let state = app()?;
    let pusher = state.push.clone();
    tauri::async_runtime::spawn_blocking(move || pusher.import(&pem, &key_id, &team_id))
        .await
        .map_err(|e| e.to_string())??;
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, false, view).await)
}

/// Remove the APNs key (the person's click): no pushes until one is
/// imported again.
#[tauri::command]
pub async fn phone_push_remove_key(webview: tauri::Webview) -> Result<PhoneAccessStatus, String> {
    main_window(&webview)?;
    push_remove_key(View::Settings).await
}

pub(crate) async fn push_remove_key(view: View) -> Result<PhoneAccessStatus, String> {
    let state = app()?;
    let pusher = state.push.clone();
    tauri::async_runtime::spawn_blocking(move || pusher.remove_key())
        .await
        .map_err(|e| e.to_string())??;
    let _ = state.app.emit(EVENT, ());
    Ok(status_of(&state, false, view).await)
}

/// Send a test notification to every paired iPhone with notifications on
/// (the person's click). Says how many Apple took, or why none went.
#[tauri::command]
pub async fn phone_push_test(webview: tauri::Webview) -> Result<String, String> {
    main_window(&webview)?;
    push_test().await
}

pub(crate) async fn push_test() -> Result<String, String> {
    let state = app()?;
    let sent = state.push.test(&state.mac_name).await?;
    let _ = state.app.emit(EVENT, ());
    Ok(if sent == 1 {
        "Sent a test notification to 1 iPhone.".to_string()
    } else {
        format!("Sent a test notification to {sent} iPhones.")
    })
}

// ── The COO's actions (L13) ──────────────────────────────────────────────
//
// The `companion.phone_*` actions call the inner functions above without
// the window check, after the person's card for every change. A pairing
// code lets a phone act as the person, so what the COO gets back never
// holds one, nor its link or QR code: whether a code is open and until
// when. The push key goes in as a file path (the file stays put and its
// text reaches nobody).

/// Where the person reads an open pairing code.
const WHERE_THE_CODE_SHOWS: &str = "Hollow's Settings (Devices, Companion devices, iPhone app)";

/// What an agent reads instead of the record's own error: a damaged
/// `phone.json`'s serde text can quote a phone's push token.
pub(crate) const AGENT_RECORD_ERROR: &str = "The record of paired phones couldn't be read (phone-record). Opening Hollow's Settings (Devices, Companion devices, iPhone app) moves a damaged one aside and says why.";

/// What an agent reads instead of a push error: Apple's answer can name a
/// phone's device token.
pub(crate) const AGENT_PUSH_ERROR: &str = "The last push failed, or the saved key couldn't be read (phone-push). Hollow's Settings (Devices, Companion devices, iPhone app) shows why.";

/// Host-written refusals that quote nothing a phone, Apple, Tailscale or a
/// file wrote; an agent may read these as they are.
const AGENT_SAFE_ERRORS: [&str; 5] = [
    "Turn on the iPhone app first, so the iPhone can reach this Mac.",
    "The iPhone app isn't available in this copy of Hollow (only the Hollow that runs this vault's chats serves it).",
    "Import the push key first.",
    "No paired iPhone has turned on notifications yet. Open the Hollow app on the iPhone and allow them.",
    "No notification went.",
];

/// An agent's error for a failed iPhone app change: one of the fixed
/// refusals above as it is, anything else (the record's serde text, Apple's,
/// Tailscale's or the Keychain's own words) as a content-free sentence.
pub(crate) fn agent_error(what: &str, error: String) -> String {
    if AGENT_SAFE_ERRORS.contains(&error.as_str()) {
        return error;
    }
    format!("Hollow couldn't {what} (phone-error). Doing it from Hollow's Settings (Devices, Companion devices, iPhone app) shows the reason.")
}

/// The status as the COO sees it: `pairing` (the code, its link and its QR
/// code) left out, `pairingOpen` and `pairingExpiresAt` in its place, and
/// the record's and the pushes' own errors replaced by fixed sentences.
pub(crate) fn for_agent(mut status: PhoneAccessStatus) -> serde_json::Value {
    let expires = status.pairing.take().map(|p| p.expires_at);
    let machine_expires = status.machine_pairing.take().map(|p| p.expires_at);
    // B4: an approvals code lets a phone enroll keys; never an agent's.
    status.approvals_code = None;
    if status.record_error.is_some() {
        status.record_error = Some(AGENT_RECORD_ERROR.to_string());
    }
    if let Some(push) = status.push.as_mut() {
        if push.last_error.is_some() {
            push.last_error = Some(AGENT_PUSH_ERROR.to_string());
        }
    }
    let mut value = serde_json::to_value(&status).unwrap_or(serde_json::Value::Null);
    if let Some(map) = value.as_object_mut() {
        map.remove("pairing");
        map.insert(
            "pairingOpen".into(),
            serde_json::Value::Bool(expires.is_some()),
        );
        map.insert(
            "pairingExpiresAt".into(),
            expires.map_or(serde_json::Value::Null, serde_json::Value::String),
        );
        // M2: a machine code lets another Mac steer chats; never an agent's.
        map.remove("machinePairing");
        map.insert(
            "machinePairingOpen".into(),
            serde_json::Value::Bool(machine_expires.is_some()),
        );
        map.insert(
            "machinePairingExpiresAt".into(),
            machine_expires.map_or(serde_json::Value::Null, serde_json::Value::String),
        );
    }
    value
}

/// Settings reads the iPhone app's status again (a change the COO made).
fn changed() {
    if let Some(state) = APP.get() {
        let _ = state.app.emit(EVENT, ());
    }
}

/// The COO's status read: never moves a damaged record aside.
pub(crate) async fn agent_status(fresh: bool) -> serde_json::Value {
    for_agent(status_now(fresh, View::Agent).await)
}

pub(crate) async fn agent_set_enabled(enabled: bool) -> Result<serde_json::Value, String> {
    let what = if enabled {
        "turn the iPhone app on"
    } else {
        "turn the iPhone app off"
    };
    set_enabled(enabled, View::Agent)
        .await
        .map(for_agent)
        .map_err(|e| agent_error(what, e))
}

/// Open a phone pairing window for the person: the code shows only in
/// Settings, which is told to read it again.
pub(crate) async fn agent_pairing_start() -> Result<serde_json::Value, String> {
    let status = pairing_start(View::Agent)
        .await
        .map_err(|e| agent_error("open an iPhone pairing code", e))?;
    changed();
    let mut value = for_agent(status);
    value["note"] = serde_json::Value::String(format!(
        "A pairing code is open for 10 minutes or one use. The person reads it (and its QR code) in {WHERE_THE_CODE_SHOWS} and scans or types it on the iPhone; agents never see it."
    ));
    Ok(value)
}

pub(crate) async fn agent_pairing_cancel() -> Result<serde_json::Value, String> {
    let status = pairing_cancel(View::Agent)
        .await
        .map_err(|e| agent_error("close the iPhone pairing code", e))?;
    changed();
    Ok(for_agent(status))
}

pub(crate) async fn agent_revoke(device_id: &str) -> Result<serde_json::Value, String> {
    revoke(device_id, View::Agent)
        .await
        .map(for_agent)
        .map_err(|e| agent_error("revoke that iPhone", e))
}

pub(crate) async fn agent_publish() -> Result<serde_json::Value, String> {
    let status = publish(View::Agent)
        .await
        .map_err(|e| agent_error("publish the iPhone app on Tailscale", e))?;
    changed();
    Ok(for_agent(status))
}

pub(crate) async fn agent_unpublish() -> Result<serde_json::Value, String> {
    let status = unpublish(View::Agent)
        .await
        .map_err(|e| agent_error("stop publishing the iPhone app on Tailscale", e))?;
    changed();
    Ok(for_agent(status))
}

/// Import the push key from a file path. The file's own checks
/// ([`read_key_file`], whose errors never quote the file) answer as they
/// are; a failure after them (the Keychain, the record) is content-free.
pub(crate) async fn agent_push_import_key(
    path: String,
    key_id: Option<String>,
    team_id: Option<String>,
) -> Result<serde_json::Value, String> {
    {
        let (path, key_id, team_id) = (path.clone(), key_id.clone(), team_id.clone());
        tauri::async_runtime::spawn_blocking(move || read_key_file(&path, key_id, team_id))
            .await
            .map_err(|_| "Hollow couldn't read that file.".to_string())??;
    }
    push_import_key(path, key_id, team_id, View::Agent)
        .await
        .map(for_agent)
        .map_err(|e| agent_error("import the push key", e))
}

pub(crate) async fn agent_push_remove_key() -> Result<serde_json::Value, String> {
    push_remove_key(View::Agent)
        .await
        .map(for_agent)
        .map_err(|e| agent_error("remove the push key", e))
}

pub(crate) async fn agent_push_test() -> Result<serde_json::Value, String> {
    let sent = push_test()
        .await
        .map_err(|e| agent_error("send a test notification", e))?;
    Ok(serde_json::json!({ "sent": sent }))
}

// ── `hollow serve`'s commands (M1 PR 3, `serve_control.rs`) ──────────────
//
// The person at the serving Mac's terminal: the Settings clicks a headless
// Hollow has no window for. A running serve takes them over its control
// socket; with serve stopped the record is changed on disk.

/// A mode and agent for `set-default-mode`, checked: one of Hollow's modes
/// and, when named, one of its agents (stored by its own name).
pub(crate) fn checked_default_mode(
    mode: &str,
    agent: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let mode = mode.trim();
    if !known_mode(mode) {
        return Err(format!(
            "`{mode}` is not a permission mode; use one of {}.",
            crate::app_mcp::PERMISSION_MODES.join(", ")
        ));
    }
    let agent = match agent {
        None => None,
        Some(name) => Some(
            crate::agent::AgentKind::from_opt(name)
                .ok_or_else(|| format!("`{}` is not an agent Hollow runs.", name.trim()))?
                .as_str()
                .to_string(),
        ),
    };
    Ok((mode.to_string(), agent))
}

/// `hollow serve pair`: open a pairing window in this process. The code
/// (`XXXX-XXXX`) and when it closes go back to the command only: never a
/// log, an event or an agent.
pub(crate) async fn serve_pairing_start() -> Result<(String, String), String> {
    let state = app()?;
    if state.server.lock().await.server.is_none() || state.phones_off.load(Ordering::SeqCst) {
        return Err(
            "The phone API is off or could not listen. Run `hollow serve enable-phone` first."
                .into(),
        );
    }
    let window = state.pairing.open(chrono::Utc::now());
    Ok((
        window.display,
        window
            .expires
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    ))
}

/// M2: `hollow serve pair --machine`: open the machine window in this
/// process (listening first if the iPhone switch is off). The code goes
/// back to the command only, with which Mac this is, so the other Mac can
/// check it reached the right one.
pub(crate) async fn serve_machine_pairing_start() -> Result<ServeMachineWindow, String> {
    let state = app()?;
    let info = {
        let windowless = state.windowless;
        let name = state.mac_name.clone();
        tauri::async_runtime::spawn_blocking(move || {
            routes::MachineSource::App {
                headless: windowless,
            }
            .info(&name, None)
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let (code, expires_at) = machine_pairing_open(&state).await?;
    Ok(ServeMachineWindow {
        code,
        expires_at,
        machine_id: info.machine_id,
        slug: info.slug,
        name: info.name,
    })
}

/// What `hollow serve pair --machine` gets back.
pub(crate) struct ServeMachineWindow {
    pub code: String,
    pub expires_at: String,
    pub machine_id: String,
    pub slug: Option<String>,
    pub name: String,
}

/// `hollow serve enable-phone`/`disable-phone` in a running serve: whether
/// the phone API listens afterwards, and its bind error if it could not.
pub(crate) async fn serve_set_enabled(enabled: bool) -> Result<(bool, Option<String>), String> {
    let status = set_enabled(enabled, View::Agent).await?;
    Ok((status.listening, status.bind_error))
}

/// `hollow serve set-default-mode` in a running serve.
pub(crate) async fn serve_set_default_mode(
    mode: String,
    agent: Option<String>,
) -> Result<(), String> {
    let state = app()?;
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || store.set_default_mode(&mode, agent.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

/// The same changes with serve stopped: the record on disk.
pub(crate) fn offline_set_enabled(enabled: bool) -> Result<(), String> {
    Store::for_app()
        .ok_or("no support folder for Hollow's phone record")?
        .set_enabled(enabled)
}

pub(crate) fn offline_set_default_mode(mode: &str, agent: Option<&str>) -> Result<(), String> {
    Store::for_app()
        .ok_or("no support folder for Hollow's phone record")?
        .set_default_mode(mode, agent)
}

/// The mode stored with `hollow serve set-default-mode` (the record's
/// `newChatMode`), for the turns serve starts itself
/// (`app_mcp::unrecorded_mode`). `None` before the phone API's state exists.
pub(crate) fn stored_default_mode() -> Option<String> {
    APP.get()?.store.read().ok()?.new_chat_mode
}
