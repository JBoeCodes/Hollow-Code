//! The `hollow_app` MCP stdio server (PR A1 of the full Hollow connector):
//! Hollow itself as tools, so an agent in a chat or automation can do what
//! the user can do in the app. A1 delivers the foundation and every read;
//! writes, confirm cards, settings and the external tier follow (A2-A5).
//!
//! Tools: a handful of first-class ones for the common reads
//! (`list_workspaces`, `read_workspace`, `list_sessions`, `read_session`,
//! `list_automations`, `read_automation`, `build_status`), each an alias of
//! one registry action, and a catalog for everything else: `list_actions`
//! and `describe_action` answer from the registry compiled into this binary
//! (`app_actions::ACTIONS`), and `run_action` runs any action by id.
//!
//! **This process does nothing itself** (the rule for every Hollow MCP
//! subprocess: it cannot share the GUI's state or locks). Every call is
//! forwarded as one JSON line over the per-pid Unix socket named by
//! `HOLLOW_APP_SOCK` to the GUI's broker (`app_broker.rs`), which checks the
//! GUI-minted `HOLLOW_APP_PROOF` over the run identity (`HOLLOW_RUN_ID` /
//! `_SURFACE` / `_SCOPE`), `HOLLOW_APP_ACCESS` and `HOLLOW_APP_DEPTH`,
//! validates the arguments against the registry, applies the action's tier
//! and runs the same Tauri command the UI does. None of these names contain
//! KEY, SECRET or TOKEN, which Codex strips from an MCP server's environment.
//!
//! Attached by `mcp_attach` to Claude and Codex chats and automations while
//! Settings, Connectors, Built into Hollow, Hollow app is on (`agent-tools.json`, loaded
//! before the run-queue pump starts). Preapproved like the other owned
//! servers (`agent::owned_preapprovals`): the broker, not the CLI's prompt,
//! decides what needs the user.
//!
//! Launched as the hidden `hollow app-mcp` subcommand (see `main.rs`).

use crate::app_actions::{self, Tier};
use crate::mcp_envelope;
use crate::usage_ledger::{self, DelegateParent};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The server key in an mcpServers map (and Codex's `hollow_` prefix rule).
pub const SERVER_NAME: &str = "hollow_app";
/// The whole-server allowlist token, and the opt-in that attaches this
/// server to a tool-restricted automation.
pub const APP_TOOL_ALLOW: &str = "mcp__hollow_app";
/// The broker socket the GUI bound.
pub const ENV_SOCK: &str = "HOLLOW_APP_SOCK";
/// The GUI-minted proof over the run identity, access and depth.
pub const ENV_PROOF: &str = "HOLLOW_APP_PROOF";
/// `read` or `write`.
pub const ENV_ACCESS: &str = "HOLLOW_APP_ACCESS";
/// How many `hollow_app`-started sessions deep this run is (0 for a chat the
/// user started or an automation).
pub const ENV_DEPTH: &str = "HOLLOW_APP_DEPTH";
/// The run's permission mode (`plan`, `default`, `acceptEdits`, `auto`,
/// `bypassPermissions`): a session it starts may have at most this one
/// without the user's card (A2).
pub const ENV_PERM: &str = "HOLLOW_APP_PERM";

const DEFAULT_PROTOCOL: &str = "2024-11-05";
/// How long a read waits for the GUI's answer (a git status or a tracker
/// check can take a few seconds).
const BROKER_TIMEOUT_SECS: u64 = 60;
/// How long anything else waits: a change may first wait up to two minutes
/// for the user's Allow on a card (`approvals`), then run; a browser sign-in
/// (A5) runs up to five minutes after that.
pub(crate) const BROKER_CHANGE_TIMEOUT_SECS: u64 = 450;
/// Codex's own MCP tool-call timeout for this server (`tool_timeout_sec`;
/// Codex's default is 60 s, shorter than a card): a little over the above.
pub const CODEX_TOOL_TIMEOUT_SECS: u64 = 480;

/// Prompt guidance: byte-stable per (mode, surface, access) and path-free,
/// for the workflow `<hollow-app>` block (a chat uses
/// [`chat_prompt_guidance`]). Its card
/// sentences come from the mode contract ([`crate::mode_contract::app_cards`],
/// F6/Q1). `automation`: a workflow's run, which no one can answer.
/// `read_only`: the server lists only reads (plan mode, or an automation
/// limited to reading), so no change or card is described.
pub fn prompt_guidance(perm: &str, automation: bool, read_only: bool) -> String {
    guidance_with(
        &crate::mode_contract::app_cards(perm, automation),
        read_only,
        true,
    )
}

/// [`prompt_guidance`] for a chat (HB01, evening 10-16): the same text in
/// every mode, with [`crate::mode_contract::APP_CARDS_POINTER`] in place of
/// the card sentences, which go in the turn's `<hollow-mode>` line
/// (`mode_contract::turn_line`). A chat's appended prompt is in its native
/// session's fingerprint, so mode wording here reset the session whenever
/// the mode flipped. It leaves out [`CHILD_ANSWER`] (OB16, overnight
/// 10-17): a chat whose app tools can change things is always followed by
/// `delegation::guidance`'s durable child task line, which says the same.
pub fn chat_prompt_guidance(read_only: bool) -> String {
    guidance_with(crate::mode_contract::APP_CARDS_POINTER, read_only, false)
}

/// How a started session's answer is read, in the workflow guidance.
const CHILD_ANSWER: &str = " A session you start runs on its own: wait_for_session waits for its turn to end (bounded); read_session reads its answer.";

/// `child_answer`: whether to say [`CHILD_ANSWER`].
fn guidance_with(cards: &str, read_only: bool, child_answer: bool) -> String {
    const READS: &str = "Hollow app tools (hollow_app): sidebar projects are local workspaces. Reads: list_workspaces, read_workspace, list_sessions, read_session, wait_for_session, list_automations, read_automation, build_status, get_settings and run_script (many calls in one script).";
    const CHANGES: &str = "Changes: create_workspace (a local sidebar project), update_workspace, start_session, send_to_session, stop_session, manage_session, save_automation, run_automation, change_setting, build_commit, build_publish (push or pull request) and navigate (which opens a page in Hollow's window, or with page browser a web address in the side panel's browser beside the conversation: use it to show the user a dev server or site you started); every change lands in this run's History & recovery and is refused once the run ends.";
    const SETTINGS: &str = "Settings the window owns (theme, panes, notifications and more) change only while Hollow's window is open.";
    const CATALOG: &str = "list_actions shows everything else (tasks, notes, goals, history, usage, people, notifications and more) with its tier, and run_action runs one by its id with the arguments describe_action lists.";
    const PREFER: &str = "Prefer these over editing Hollow's files directly: they validate like the app and keep its state consistent.";
    const LARGE: &str =
        "Large results come back cut to what fits; narrow them with the action's arguments.";
    if read_only {
        return format!("{READS} This run can only read: describe a change you would make in your reply instead. {CATALOG} {LARGE}");
    }
    let child = if child_answer { CHILD_ANSWER } else { "" };
    format!("{READS} {CHANGES} {cards} {SETTINGS} {CATALOG} {PREFER}{child} {LARGE}")
}

/// What a run's server may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAccess {
    Read,
    Write,
}

impl AppAccess {
    /// A chat turn: plan mode reads only; every other mode may write.
    pub fn for_chat(permission_mode: &str) -> Self {
        if permission_mode.trim() == "plan" {
            AppAccess::Read
        } else {
            AppAccess::Write
        }
    }

    /// An automation, or `None` when this server must not attach. An
    /// unrestricted automation gets it (read-only in plan mode). A
    /// tool-restricted one (a stamped `allowed_tools`) gets it only when the
    /// list names it, whole (`mcp__hollow_app`) or per tool, and then writes
    /// only when it names the whole server and isn't in plan mode: a
    /// reviewed allowlist is never widened to all of Hollow silently.
    pub fn for_workflow(permission_mode: &str, allowed_tools: &[String]) -> Option<Self> {
        let plan = permission_mode.trim() == "plan";
        let tools: Vec<&str> = allowed_tools
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        if tools.is_empty() {
            return Some(if plan {
                AppAccess::Read
            } else {
                AppAccess::Write
            });
        }
        let whole = tools.contains(&APP_TOOL_ALLOW);
        let per_tool = tools
            .iter()
            .any(|t| t.starts_with(concat!("mcp__hollow_app", "__")));
        match (whole, per_tool) {
            (true, _) if !plan => Some(AppAccess::Write),
            (true, _) | (false, true) => Some(AppAccess::Read),
            (false, false) => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AppAccess::Read => "read",
            AppAccess::Write => "write",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "read" => Some(AppAccess::Read),
            "write" => Some(AppAccess::Write),
            _ => None,
        }
    }

    /// Whether an action of `tier` may be listed for this run.
    pub(crate) fn allows(self, tier: Tier) -> bool {
        tier == Tier::Read || self == AppAccess::Write
    }
}

/// Settings, Connectors, Built into Hollow, Hollow app. Off until the GUI loads the persisted
/// switch (`app_broker::load_settings`), like the task tools.
static APP_MCP_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    APP_MCP_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    APP_MCP_ENABLED.load(Ordering::Relaxed)
}

/// What a run's server may do and who started the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGrant {
    pub access: AppAccess,
    /// 0 for a run the user or a schedule started; a session started through
    /// `hollow_app` runs at its caller's depth + 1 (A2).
    pub depth: u32,
    /// The run's permission mode, as the agent was spawned with it.
    pub perm: String,
    /// L1: the depth the server lists the session starters at: 0 for the
    /// COO's and a project lead's turns whoever messaged them
    /// (`app_sessions::start_depth`), else [`Self::depth`]. Only what is
    /// advertised; not in the proof (the broker checks starts itself,
    /// `app_sessions::effective_depth`).
    pub list_depth: u32,
}

impl AppGrant {
    pub fn new(access: AppAccess, depth: u32, perm: &str) -> Self {
        AppGrant {
            access,
            depth,
            perm: normalize_perm(perm).to_string(),
            list_depth: depth,
        }
    }

    /// [`Self::new`] listing the starters at `list_depth` (L1).
    pub fn listing_from(mut self, list_depth: u32) -> Self {
        self.list_depth = list_depth.min(self.depth);
        self
    }
}

/// L1: [`AppGrant::list_depth`] in the server's environment.
pub const ENV_LIST_DEPTH: &str = "HOLLOW_APP_LIST_DEPTH";

/// The permission modes, weakest first. A mode Hollow doesn't know ranks as
/// `acceptEdits`, the adapters' fallback (`agent::build_*`).
pub const PERMISSION_MODES: [&str; 5] = [
    "plan",
    "default",
    "acceptEdits",
    "auto",
    "bypassPermissions",
];

/// `perm` as one of [`PERMISSION_MODES`] (empty or unknown: `acceptEdits`).
pub fn normalize_perm(perm: &str) -> &'static str {
    PERMISSION_MODES
        .iter()
        .copied()
        .find(|m| *m == perm.trim())
        .unwrap_or("acceptEdits")
}

/// How strong a permission mode is (0 = plan … 4 = bypassPermissions).
pub fn perm_rank(perm: &str) -> usize {
    let perm = normalize_perm(perm);
    PERMISSION_MODES
        .iter()
        .position(|m| *m == perm)
        .unwrap_or(2)
}

/// Hollow's default mode: what a chat with no recorded mode runs in.
pub const DEFAULT_MODE: &str = "acceptEdits";

/// The mode a turn Hollow runs in a chat without the person typing (a wake,
/// a check-in, a child's end, an event trigger, an automation in the chat, a
/// send from another chat, a held send, a routed send) starts from: the
/// chat's own last mode (`session_store::last_permission`), normalized, or
/// [`DEFAULT_MODE`] when it recorded none (2026-11-12: before, these turns
/// were capped at `acceptEdits`). A caller may only lower it ([`lowered`]);
/// what an unattended turn may do is limited by Hollow's cards and the
/// unattended allow list (`main_agent::unattended`), not by its mode.
pub fn own_mode(last: Option<&str>) -> &'static str {
    match last {
        Some(last) => normalize_perm(last),
        None => unrecorded_mode(),
    }
}

/// The mode a turn Hollow starts itself takes when nothing says one (no
/// recorded mode, a workflow with no `permission_mode`, a send with none):
/// [`DEFAULT_MODE`] in the app. `hollow serve` has nobody at a window to
/// see what such a turn does, so there it is the mode stored with `hollow
/// serve set-default-mode`, never wider than [`DEFAULT_MODE`], else Plan.
pub fn unrecorded_mode() -> &'static str {
    if !crate::serve::serving() {
        return DEFAULT_MODE;
    }
    unrecorded_mode_in_serve(crate::phone_access::stored_default_mode().as_deref())
}

/// [`unrecorded_mode`] in serve, for the stored mode `stored`.
pub fn unrecorded_mode_in_serve(stored: Option<&str>) -> &'static str {
    match stored
        .map(str::trim)
        .filter(|m| PERMISSION_MODES.contains(m))
    {
        Some(mode) => lowered(mode, Some(DEFAULT_MODE)),
        None => "plan",
    }
}

/// `mode` lowered to `cap` when `cap` is weaker; a cap never raises it.
pub fn lowered(mode: &str, cap: Option<&str>) -> &'static str {
    let mode = normalize_perm(mode);
    match cap.map(normalize_perm) {
        Some(cap) if perm_rank(cap) < perm_rank(mode) => cap,
        _ => mode,
    }
}

/// The `hollow_app` entry for a run's mcpServers map, or `None` when the
/// switch is off, the broker isn't listening, or the executable is unknown.
pub fn server_entry(parent: &DelegateParent, grant: &AppGrant) -> Option<(&'static str, Value)> {
    let (on, sock) = attach_state();
    if !on {
        return None;
    }
    let sock = sock?;
    let exe = std::env::current_exe().ok()?;
    let proof = crate::app_broker::proof_for(
        &parent.run_id,
        parent.surface.as_str(),
        &parent.scope,
        grant,
    );
    Some(entry_for(&exe, &sock, parent, grant, &proof))
}

fn attach_state() -> (bool, Option<PathBuf>) {
    #[cfg(test)]
    if let Some(state) = ATTACH_OVERRIDE.with(|o| o.borrow().clone()) {
        return state;
    }
    (enabled(), crate::app_broker::socket_path())
}

#[cfg(test)]
thread_local! {
    static ATTACH_OVERRIDE: std::cell::RefCell<Option<(bool, Option<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` with this thread's switch and broker socket replaced (test only).
#[cfg(test)]
pub(crate) fn with_attach_override<R>(
    enabled: bool,
    sock: Option<PathBuf>,
    f: impl FnOnce() -> R,
) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ATTACH_OVERRIDE.with(|o| *o.borrow_mut() = None);
        }
    }
    ATTACH_OVERRIDE.with(|o| *o.borrow_mut() = Some((enabled, sock)));
    let _reset = Reset;
    f()
}

/// The pure entry builder behind [`server_entry`].
pub(crate) fn entry_for(
    exe: &Path,
    sock: &Path,
    parent: &DelegateParent,
    grant: &AppGrant,
    proof: &str,
) -> (&'static str, Value) {
    let mut env = serde_json::Map::new();
    env.insert(ENV_SOCK.into(), json!(sock.to_string_lossy()));
    env.insert(usage_ledger::ENV_RUN_ID.into(), json!(parent.run_id));
    env.insert(
        usage_ledger::ENV_RUN_SURFACE.into(),
        json!(parent.surface.as_str()),
    );
    env.insert(usage_ledger::ENV_RUN_SCOPE.into(), json!(parent.scope));
    env.insert(ENV_ACCESS.into(), json!(grant.access.as_str()));
    env.insert(ENV_DEPTH.into(), json!(grant.depth.to_string()));
    if grant.list_depth != grant.depth {
        env.insert(ENV_LIST_DEPTH.into(), json!(grant.list_depth.to_string()));
    }
    env.insert(ENV_PERM.into(), json!(grant.perm));
    env.insert(ENV_PROOF.into(), json!(proof));
    (
        SERVER_NAME,
        json!({
            "command": exe.to_string_lossy(),
            "args": ["app-mcp"],
            "env": env,
        }),
    )
}

// ── Broker client ────────────────────────────────────────────────────────

/// Forwards one action to the GUI's broker. Tests plug in an in-process one.
/// `via` is the tool the agent called and its arguments exactly as sent,
/// which the broker uses only to put a card on the right tool row.
pub(crate) trait Broker {
    fn call(&self, action: &str, args: &Value, via: Via<'_>) -> Result<Value, String>;

    /// The note the broker's answers carried since the last take (repo
    /// features R2a: the context gauge's growth step or compaction
    /// pointer), sent once as a second text block of a tool result.
    fn take_note(&self) -> Option<String> {
        None
    }
}

/// The agent's own tool call behind a forwarded action.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Via<'a> {
    pub tool: &'a str,
    pub args: &'a Value,
}

/// Who a request is from, as the GUI stated it in this server's environment.
#[derive(Debug, Clone)]
pub(crate) struct RunIdentity {
    pub run_id: String,
    pub surface: String,
    pub scope: String,
    pub access: AppAccess,
    pub depth: u32,
    pub perm: String,
    pub proof: String,
}

impl RunIdentity {
    /// The one-line request the broker reads.
    pub(crate) fn request(&self, action: &str, args: &Value, via: Via<'_>) -> Value {
        json!({
            "v": 1,
            "action": action,
            "runId": self.run_id,
            "surface": self.surface,
            "scope": self.scope,
            "access": self.access.as_str(),
            "depth": self.depth,
            "perm": self.perm,
            "proof": self.proof,
            "args": args,
            "tool": via.tool,
            "toolArgs": via.args,
        })
    }
}

/// The blocking Unix-socket client: one connection per call, one line each
/// way.
#[cfg(unix)]
pub(crate) struct SocketClient {
    pub path: PathBuf,
    pub identity: RunIdentity,
    /// Notes the answers carried, not yet sent (R2a). Tool calls run on
    /// their own threads ([`serve_lines`]), so the next result that
    /// finishes sends it, whichever call's answer brought it.
    pub note: std::sync::Mutex<Option<String>>,
}

/// One broker answer line: the result or the refusal, and the note it
/// carried (R2a). `tasks_mcp::parse_broker_answer` reads the same shape
/// without a note.
pub(crate) fn parse_app_answer(line: &str) -> Result<(Value, Option<String>), String> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("Hollow's app broker sent an unreadable answer ({e})."))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Hollow's app broker refused the request.")
            .to_string());
    }
    let note = value
        .get("note")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string);
    Ok((value.get("result").cloned().unwrap_or(Value::Null), note))
}

#[cfg(unix)]
impl SocketClient {
    fn keep_note(&self, note: String) {
        let mut slot = self.note.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_mut() {
            Some(held) => {
                held.push('\n');
                held.push_str(&note);
            }
            None => *slot = Some(note),
        }
    }
}

#[cfg(unix)]
impl Broker for SocketClient {
    fn call(&self, action: &str, args: &Value, via: Via<'_>) -> Result<Value, String> {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let mut stream = UnixStream::connect(&self.path).map_err(|e| {
            format!("Hollow's app broker is unavailable ({e}); nothing ran. Hollow may have quit.")
        })?;
        let wait = broker_wait_secs(action);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(wait)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let mut line = self.identity.request(action, args, via).to_string();
        line.push('\n');
        stream
            .write_all(line.as_bytes())
            .and_then(|_| stream.flush())
            .map_err(|e| format!("Could not reach Hollow's app broker ({e}); nothing ran."))?;
        let mut reader = BufReader::new(stream);
        let mut answer = String::new();
        match reader.read_line(&mut answer) {
            Ok(0) => Err("Hollow closed the connection before answering.".into()),
            Ok(_) => parse_app_answer(&answer).map(|(result, note)| {
                if let Some(note) = note {
                    self.keep_note(note);
                }
                result
            }),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Err(format!("Hollow did not answer within {wait} seconds."))
            }
            Err(e) => Err(format!("Reading Hollow's answer failed ({e}).")),
        }
    }

    fn take_note(&self) -> Option<String> {
        self.note.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// How long this server waits for the GUI's answer to `action`: a read 60
/// s, a change 450 s (it may wait on a card), and `wait_for_session` its
/// longest wait plus room for the answer (F10, D2), all within Codex's
/// [`CODEX_TOOL_TIMEOUT_SECS`] for this server.
pub(crate) fn broker_wait_secs(action: &str) -> u64 {
    if action == "sessions.wait" {
        return crate::delegation::WAIT_ANSWER_SECONDS;
    }
    // A script runs up to `codemode::SERVED_OVERALL` and may wait on cards.
    if action == "codemode.run" {
        return BROKER_CHANGE_TIMEOUT_SECS;
    }
    match app_actions::find(action).map(|a| a.tier) {
        Some(Tier::Read) => BROKER_TIMEOUT_SECS,
        _ => BROKER_CHANGE_TIMEOUT_SECS,
    }
}

// ── Server ───────────────────────────────────────────────────────────────

/// Everything one request needs.
pub(crate) struct Ctx<'a> {
    pub access: AppAccess,
    /// This run's `HOLLOW_APP_DEPTH`: at [`MAX_START_DEPTH`] or deeper the
    /// session starters aren't listed (the broker refuses them anyway).
    pub depth: u32,
    pub broker: Option<&'a dyn Broker>,
}

/// A run this many `hollow_app`-started sessions deep can't start another.
pub(crate) const MAX_START_DEPTH: u32 = 2;

/// `list_actions`' page when the caller names no `limit` (F9, wave 4 · V2).
pub(crate) const DEFAULT_PAGE: usize = 25;
/// The largest page `list_actions` returns.
pub(crate) const MAX_PAGE: usize = 100;
/// A listed action's summary is cut to one line of at most this many
/// characters; `describe_action` has the whole of it.
pub(crate) const SUMMARY_LINE_CHARS: usize = 140;

/// Entry point for the `app-mcp` subcommand: a JSON-RPC loop over stdio
/// until stdin closes ([`serve_lines`]: tool calls run at once).
pub fn serve() {
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let access = var(ENV_ACCESS)
        .and_then(|a| AppAccess::parse(&a))
        .unwrap_or(AppAccess::Read);
    let depth: u32 = var(ENV_DEPTH).and_then(|d| d.parse().ok()).unwrap_or(0);
    // L1: what the starters are listed at (never deeper than the run).
    let list_depth: u32 = var(ENV_LIST_DEPTH)
        .and_then(|d| d.parse().ok())
        .map_or(depth, |d: u32| d.min(depth));
    #[cfg(unix)]
    let socket = match (
        var(ENV_SOCK),
        var(usage_ledger::ENV_RUN_ID),
        var(usage_ledger::ENV_RUN_SURFACE),
        var(usage_ledger::ENV_RUN_SCOPE),
        var(ENV_PROOF),
    ) {
        (Some(sock), Some(run_id), Some(surface), Some(scope), Some(proof)) => Some(SocketClient {
            path: PathBuf::from(sock),
            identity: RunIdentity {
                run_id,
                surface,
                scope,
                access,
                depth,
                perm: var(ENV_PERM).unwrap_or_default(),
                proof,
            },
            note: std::sync::Mutex::new(None),
        }),
        _ => None,
    };
    #[cfg(unix)]
    let broker: Option<&(dyn Broker + Sync)> = socket.as_ref().map(|c| c as &(dyn Broker + Sync));
    #[cfg(not(unix))]
    let broker: Option<&(dyn Broker + Sync)> = None;
    let stdin = std::io::stdin();
    serve_lines(stdin.lock(), std::io::stdout(), |req: &Value| {
        let ctx = Ctx {
            access,
            depth: list_depth,
            broker: broker.map(|b| b as &dyn Broker),
        };
        handle(req, &ctx)
    });
}

/// The JSON-RPC loop over any line stream: one message per line in, one
/// response per line out, a tool call's in the order the calls finish.
///
/// **Tool calls run at once (F10, D2 review).** Each `tools/call` runs on
/// its own thread as soon as its line arrives, so its wait counts from when
/// the agent sent it: a Codex primary's [`CODEX_TOOL_TIMEOUT_SECS`] for this
/// server counts from the send, and a `wait_for_session` (up to
/// `delegation::WAIT_ANSWER_SECONDS`) or a change waiting on a card must
/// never queue behind another call's wait. The GUI's broker already serves
/// each connection on its own thread. Responses carry their request's id, as
/// JSON-RPC requires. Everything else (the handshake, `tools/list`,
/// notifications) is answered in line order, before any later message is
/// read. At EOF the calls already running finish and are answered; a failed
/// write stops reading.
pub(crate) fn serve_lines<R, W, H>(reader: R, writer: W, handler: H)
where
    R: std::io::BufRead,
    W: std::io::Write + Send,
    H: Fn(&Value) -> Option<Value> + Sync,
{
    let writer = std::sync::Mutex::new(writer);
    let closed = AtomicBool::new(false);
    let answer = |resp: Option<Value>| {
        let Some(resp) = resp else { return };
        let mut s = resp.to_string();
        s.push('\n');
        let mut out = writer.lock().unwrap_or_else(|e| e.into_inner());
        if out
            .write_all(s.as_bytes())
            .and_then(|_| out.flush())
            .is_err()
        {
            closed.store(true, Ordering::SeqCst);
        }
    };
    let (handler, answer) = (&handler, &answer);
    std::thread::scope(|scope| {
        for raw in reader.lines() {
            if closed.load(Ordering::SeqCst) {
                break;
            }
            let Ok(raw) = raw else { break };
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(req) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if req.get("method").and_then(Value::as_str) != Some("tools/call") {
                answer(handler(&req));
                continue;
            }
            let call = req.clone();
            let spawned = std::thread::Builder::new()
                .name("hollow-app-call".into())
                .spawn_scoped(scope, move || answer(handler(&call)));
            if spawned.is_err() {
                // No thread to spare: answer it here, as before.
                answer(handler(&req));
            }
        }
    });
}

/// Dispatch one JSON-RPC message: `Some(response)` for requests, `None` for
/// notifications.
pub(crate) fn handle(req: &Value, ctx: &Ctx) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => {
            let pv = req
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL);
            Some(result(
                id,
                json!({
                    "protocolVersion": pv,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "hollow-app", "version": env!("CARGO_PKG_VERSION") },
                }),
            ))
        }
        "tools/list" => Some(result(
            id,
            json!({ "tools": tool_specs(ctx.access, ctx.depth) }),
        )),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if ENVELOPED.contains(&name) {
                return Some(call_enveloped(id, name, args, ctx));
            }
            Some(match call_tool(name, &args, ctx) {
                Ok(value) => {
                    let note = ctx.broker.and_then(|b| b.take_note());
                    tool_text(id, &render(&value), note.as_deref())
                }
                Err(error) => tool_error(id, &error),
            })
        }
        "ping" => Some(result(id, json!({}))),
        _ => id.map(|id| error(Some(id), -32601, &format!("method not found: {method}"))),
    }
}

/// The first-class reads whose results go through the shared envelope
/// (OB1, overnight 11-17, `mcp_envelope`): a byte budget, `maxBytes` and
/// `cursor`, headers-only items past the whole ones. `run_action` of the
/// same action ids is not enveloped (the broker's own cut applies).
pub(crate) const ENVELOPED: [&str; 2] = ["list_sessions", "read_session"];

/// A session row's headers-only form keeps these.
const SESSION_HEADER: [&str; 10] = [
    "key",
    "title",
    "agent",
    "status",
    "running",
    "lastActivity",
    "lastActivityAt",
    "turnCount",
    "parentKey",
    "endedAt",
];

/// A conversation message's headers-only form keeps these (and adds
/// `turn` and `textBytes`).
const TURN_HEADER: [&str; 6] = ["id", "role", "agent", "startedAt", "endedAt", "status"];

/// An [`ENVELOPED`] tool's call: the envelope's arguments taken out, the
/// action run as before, its result held to the budget, and the envelope
/// as the last text block.
fn call_enveloped(id: Option<Value>, name: &str, mut args: Value, ctx: &Ctx) -> Value {
    let req = match mcp_envelope::Request::take(name, &mut args) {
        Ok(req) => req,
        Err(error) => return tool_error(id, &error),
    };
    // list_sessions asks for one row more than its limit, so `row_limit`
    // says whether the list really goes on; the extra row is dropped
    // before anything renders, so the text is what `limit` gives.
    let probe = (name == "list_sessions")
        .then(|| sessions_probe(&args))
        .flatten();
    let called = match &probe {
        Some((probe_args, _)) => forward(
            "sessions.overview",
            probe_args,
            ctx,
            Via {
                tool: name,
                args: &args,
            },
        ),
        None => call_tool(name, &args, ctx),
    };
    let mut value = match called {
        Ok(value) => value,
        Err(error) if error.contains("unknown argument") => {
            return tool_error(
                id,
                &format!("{error} {name} also takes maxBytes and cursor (the result envelope's)."),
            )
        }
        Err(error) => return tool_error(id, &error),
    };
    let note = ctx.broker.and_then(|b| b.take_note());
    let fitted = if name == "list_sessions" {
        let more = probe.is_some_and(|(_, limit)| trim_probe(&mut value, limit));
        fit_sessions(&req, more, &value)
    } else {
        fit_conversation(&req, &value)
    };
    match fitted {
        Ok(fitted) => {
            let mut answer = tool_text(id, &fitted.text, note.as_deref());
            if let Some(content) = answer
                .pointer_mut("/result/content")
                .and_then(Value::as_array_mut)
            {
                content.push(fitted.envelope.block());
            }
            answer
        }
        Err(error) => tool_error(id, &format!("{name}: {error}")),
    }
}

/// `list_sessions` held to its budget: rows in order, the rest headers only.
/// `list_sessions`' arguments with `limit` one higher, and the limit the
/// caller meant; `None` for `mine` (which ignores limit), a limit already
/// at its 200 maximum, or one that isn't a number (the broker's argument
/// check answers that).
fn sessions_probe(args: &Value) -> Option<(Value, usize)> {
    if args.get("mine").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let detail = args.get("detail").and_then(Value::as_bool) == Some(true);
    let limit = match args.get("limit") {
        None | Some(Value::Null) => {
            if detail {
                30
            } else {
                12
            }
        }
        Some(v) => v.as_u64()?,
    };
    if limit >= 200 {
        return None;
    }
    let mut probe = args.clone();
    probe["limit"] = json!(limit + 1);
    Some((probe, limit as usize))
}

/// Drop a probe's extra row (and the detail run queue's, which the same
/// limit cuts): whether there was one.
fn trim_probe(value: &mut Value, limit: usize) -> bool {
    let mut more = false;
    if let Some(rows) = value.get_mut("sessions").and_then(Value::as_array_mut) {
        more = rows.len() > limit;
        rows.truncate(limit);
    }
    if let Some(queue) = value.get_mut("runQueue").and_then(Value::as_array_mut) {
        queue.truncate(limit);
    }
    more
}

fn fit_sessions(
    req: &mcp_envelope::Request,
    more: bool,
    value: &Value,
) -> Result<mcp_envelope::Fitted, String> {
    let row_limit = more.then_some("More sessions exist past limit: raise limit (at most 200).");
    let header = |row: &Value| mcp_envelope::header_of(row, &SESSION_HEADER);
    mcp_envelope::fit_json(
        req,
        value,
        "/sessions",
        false,
        Some(&header),
        row_limit,
        "A headerOnly session's full row: read_session with its key, or this call with a larger maxBytes.",
        &render,
    )
}

/// `read_session` held to its budget: the newest messages whole, older ones
/// headers only, pages running back from the newest.
fn fit_conversation(
    req: &mcp_envelope::Request,
    value: &Value,
) -> Result<mcp_envelope::Fitted, String> {
    let count = |key: &str| value.get(key).and_then(Value::as_u64);
    let row_limit =
        matches!((count("totalTurns"), count("returnedTurns")), (Some(t), Some(r)) if t > r)
            .then_some("Earlier turns exist: lastTurns (up to 100), or fromTurn and toTurn.");
    let header = |row: &Value| {
        let mut out = mcp_envelope::header_of(row, &TURN_HEADER);
        let turn = row
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| {
                id.strip_suffix("-user")
                    .or_else(|| id.strip_suffix("-assistant"))
            })
            .and_then(|rest| rest.rsplit_once('-'))
            .and_then(|(_, ord)| ord.parse::<u64>().ok());
        if let Some(turn) = turn {
            out["turn"] = json!(turn);
        }
        let text = row.get("text").and_then(Value::as_str).map_or(0, str::len);
        out["textBytes"] = json!(text);
        out
    };
    mcp_envelope::fit_json(
        req,
        value,
        "/turns",
        true,
        Some(&header),
        row_limit,
        "A headerOnly message's text: read_session with fromTurn and toTurn set to its turn.",
        &render,
    )
}

/// The first-class tools: tool name → the registry action it runs. Kept
/// short on purpose (Codex lists every tool's schema up front; see the A6
/// build-log entry): everything else is one `run_action` away.
pub(crate) const FIRST_CLASS: &[(&str, &str)] = &[
    ("list_workspaces", "workspaces.list_workspaces"),
    ("read_workspace", "workspaces.read_workspace"),
    ("create_workspace", "workspaces.create_workspace"),
    ("update_workspace", "workspaces.update_workspace"),
    ("list_sessions", "sessions.overview"),
    ("read_session", "sessions.conversation"),
    ("wait_for_session", "sessions.wait"),
    ("start_session", "sessions.start"),
    ("send_to_session", "sessions.send"),
    ("stop_session", "sessions.stop"),
    ("manage_session", "sessions.manage"),
    ("list_automations", "automations.list_workflows"),
    ("read_automation", "automations.read_workflow"),
    ("save_automation", "automations.save"),
    ("run_automation", "automations.run_workflow"),
    ("get_settings", "settings.get"),
    ("change_setting", "settings.change"),
    ("build_status", "build.overview"),
    ("build_commit", "build.build_commit"),
    ("build_publish", "build.publish"),
    ("navigate", "ui.navigate"),
    ("run_script", "codemode.run"),
];

/// The first-class tool that runs action `id`, if any.
pub(crate) fn tool_for(id: &str) -> Option<&'static str> {
    FIRST_CLASS
        .iter()
        .find(|(_, action)| *action == id)
        .map(|(tool, _)| *tool)
}

/// Tools that start a session: not listed at [`MAX_START_DEPTH`].
const STARTERS: [&str; 2] = ["start_session", "send_to_session"];

fn call_tool(name: &str, args: &Value, ctx: &Ctx) -> Result<Value, String> {
    match name {
        "list_actions" => list_actions(args, ctx.access, ctx.depth),
        "describe_action" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("describe_action: `id` is required.")?;
            let action = app_actions::find(id).ok_or_else(|| unknown_action(id, ctx))?;
            let mut row = app_actions::describe(action);
            row["available"] = json!(ctx.access.allows(action.tier));
            Ok(row)
        }
        "run_action" => {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .ok_or("run_action: `id` is required (an action id from list_actions).")?;
            let action_args = args.get("args").cloned().unwrap_or(Value::Null);
            forward(id, &action_args, ctx, Via { tool: name, args })
        }
        tool => match FIRST_CLASS.iter().find(|(t, _)| *t == tool) {
            Some((_, id)) => forward(id, args, ctx, Via { tool, args }),
            None => Err(format!("unknown tool: {tool}")),
        },
    }
}

/// Resolve `id` in the registry (never anything else), refuse what this
/// run's access can't do, check the arguments, then forward to the broker,
/// which checks all of it again.
fn forward(id: &str, args: &Value, ctx: &Ctx, via: Via<'_>) -> Result<Value, String> {
    let action = app_actions::find(id).ok_or_else(|| unknown_action(id, ctx))?;
    if !ctx.access.allows(action.tier) {
        return Err(format!("{id} changes Hollow, and this run can only read it (plan mode, a build workspace's turn, or an automation limited to reading). Nothing ran; describe the change in your reply instead."));
    }
    app_actions::validate(action, args)?;
    let broker = ctx
        .broker
        .ok_or("Hollow app tools are unavailable in this run: Hollow's app broker isn't connected. Nothing ran.")?;
    broker.call(id, args, via)
}

fn unknown_action(id: &str, ctx: &Ctx) -> String {
    unknown_action_for(id, |a| offered(a, ctx.access, ctx.depth))
}

/// The unknown-action message, suggesting only actions `usable` allows (what
/// the run can't use is never advertised, F9 V2): a first-class tool's name
/// passed as an id names its action; else the nearest ids.
fn unknown_action_for(id: &str, usable: impl Fn(&app_actions::Action) -> bool) -> String {
    let tool = FIRST_CLASS
        .iter()
        .find(|(name, _)| *name == id.trim())
        .and_then(|(_, action)| app_actions::find(action))
        .filter(|a| usable(a));
    let near = match tool {
        Some(a) => vec![a.id],
        None => nearest_actions(
            id,
            app_actions::ACTIONS
                .iter()
                .filter(|a| usable(a))
                .map(|a| a.id),
        ),
    };
    if near.is_empty() {
        format!("unknown action `{id}`; list_actions shows every action id.")
    } else {
        format!(
            "unknown action `{id}`; did you mean {}? list_actions shows every action id.",
            near.iter()
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(" or ")
        )
    }
}

/// Up to three action ids sharing the most words with a mistyped `id`
/// (overnight S3: agents guessed `sessions.start_session` for
/// `sessions.start`). Words split on `.`, `_` and `-`, singular and plural
/// folded; the area alone is no match, a single shared word counts only in
/// the same area (`get`, `list` are everywhere), and a match in the same
/// area wins ties.
pub(crate) fn nearest_actions<'a>(id: &str, ids: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    fn words(s: &str) -> Vec<String> {
        s.to_ascii_lowercase()
            .split(['.', '_', '-'])
            .filter(|w| !w.is_empty())
            .map(|w| {
                w.strip_suffix('s')
                    .filter(|r| r.len() > 2)
                    .unwrap_or(w)
                    .to_string()
            })
            .collect()
    }
    let (area, verb) = id.split_once('.').unwrap_or(("", id));
    let (area, verb) = (words(area), words(verb));
    let mut scored: Vec<(usize, bool, &str)> = ids
        .filter_map(|cand| {
            let (c_area, c_verb) = cand.split_once('.').unwrap_or(("", cand));
            let c_verb = words(c_verb);
            let hits = verb
                .iter()
                .filter(|w| c_verb.contains(w) || words(c_area).contains(w))
                .count();
            let verb_hit = verb.iter().any(|w| c_verb.contains(w));
            let same_area = words(c_area) == area;
            (verb_hit && (hits > 1 || same_area)).then_some((hits, same_area, cand))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.len().cmp(&b.2.len()))
            .then(a.2.cmp(b.2))
    });
    scored.into_iter().take(3).map(|(_, _, c)| c).collect()
}

/// Whether this run's server may offer `action` at all (F9, wave 4 · V2):
/// its tier fits the access, and a run too deep to start a session isn't
/// offered the starters. What it can't use is never advertised.
fn offered(action: &app_actions::Action, access: AppAccess, depth: u32) -> bool {
    access.allows(action.tier)
        && (action.id != "sessions.start_workers" || depth == 0)
        && (depth < MAX_START_DEPTH
            || !tool_for(action.id).is_some_and(|tool| STARTERS.contains(&tool)))
}

/// `summary` as one line of at most [`SUMMARY_LINE_CHARS`] characters: its
/// first sentence when that fits, else cut with "…".
fn summary_line(summary: &str) -> String {
    let flat = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    let first = flat.find(". ").map_or(flat.as_str(), |end| &flat[..=end]);
    if first.chars().count() <= SUMMARY_LINE_CHARS {
        return first.to_string();
    }
    let mut cut: String = first.chars().take(SUMMARY_LINE_CHARS - 1).collect();
    cut.push('…');
    cut
}

/// A query's words: lower case, at least two characters, a plural `s`
/// dropped, filler words left out.
fn query_terms(query: &str) -> Vec<String> {
    const FILLER: [&str; 14] = [
        "a", "an", "and", "the", "of", "to", "for", "in", "on", "with", "my", "me", "it", "is",
    ];
    let mut terms: Vec<String> = Vec::new();
    for raw in query.split(|c: char| !c.is_alphanumeric()) {
        let word = raw.to_lowercase();
        if word.chars().count() < 2 || FILLER.contains(&word.as_str()) {
            continue;
        }
        let word = match word.strip_suffix('s') {
            Some(stem) if word.len() > 3 && !word.ends_with("ss") => stem.to_string(),
            _ => word,
        };
        if !terms.contains(&word) {
            terms.push(word);
        }
    }
    terms
}

/// How well `action` matches `terms`: (terms matched, weight). A term in the
/// id counts 3, in the first-class tool name (its title) 2, in the summary
/// 1; the whole query equal to the id or the tool name ranks it first.
fn query_score(action: &app_actions::Action, query: &str, terms: &[String]) -> (usize, usize) {
    let id = action.id.to_lowercase();
    let title = tool_for(action.id).unwrap_or("").to_lowercase();
    let summary = action.summary.to_lowercase();
    let whole = query.trim().to_lowercase();
    let mut matched = 0;
    let mut weight = if whole == id || (!title.is_empty() && whole == title) {
        100
    } else {
        0
    };
    for term in terms {
        let hit = if id.contains(term.as_str()) {
            3
        } else if title.contains(term.as_str()) {
            2
        } else if summary.contains(term.as_str()) {
            1
        } else {
            0
        };
        if hit > 0 {
            matched += 1;
            weight += hit;
        }
    }
    (matched, weight)
}

/// The catalog (F9, wave 4 · V2): the actions this run may use, filtered by
/// `area`, `tier` and `query` (its words against the id, the first-class
/// tool name and the summary, best match first), a page at a time
/// (`limit`, default [`DEFAULT_PAGE`], at most [`MAX_PAGE`]; `cursor` from
/// the last page's `nextCursor`), each with one bounded summary line.
fn list_actions(args: &Value, access: AppAccess, depth: u32) -> Result<Value, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let area = text("area");
    let tier = match text("tier") {
        None => None,
        Some(t) => Some(
            [Tier::Read, Tier::Write, Tier::Confirm, Tier::External]
                .into_iter()
                .find(|tier| tier.as_str() == t)
                .ok_or_else(|| format!("list_actions: `tier` must be read, write, confirm or external (got `{t}`)."))?,
        ),
    };
    let query = text("query");
    let terms = query.map(query_terms).unwrap_or_default();
    if query.is_some() && terms.is_empty() {
        return Err("list_actions: `query` needs a word of at least two letters (an id, a tool name or what the action does).".into());
    }
    let limit = match args.get("limit") {
        None | Some(Value::Null) => DEFAULT_PAGE,
        Some(v) => v
            .as_u64()
            .map(|n| (n as usize).clamp(1, MAX_PAGE))
            .ok_or("list_actions: `limit` must be a whole number.")?,
    };
    let start = match text("cursor") {
        None => 0,
        Some(c) => c.parse::<usize>().map_err(|_| {
            "list_actions: `cursor` must be a nextCursor from an earlier page.".to_string()
        })?,
    };
    let in_area = |a: &&app_actions::Action| {
        area.is_none_or(|area| a.area == area || a.area.starts_with(&format!("{area}.")))
    };
    let mut rows: Vec<(&app_actions::Action, (usize, usize))> = app_actions::ACTIONS
        .iter()
        .filter(|a| offered(a, access, depth))
        .filter(in_area)
        .filter(|a| tier.is_none_or(|t| a.tier == t))
        .map(|a| (a, query.map_or((0, 0), |q| query_score(a, q, &terms))))
        .filter(|(_, (matched, _))| query.is_none() || *matched > 0)
        .collect();
    if query.is_some() {
        // Stable: equal matches keep the registry's order.
        rows.sort_by(|(_, a), (_, b)| b.cmp(a));
    }
    if rows.is_empty() {
        if let Some(area) = area {
            if !app_actions::ACTIONS.iter().any(|a| in_area(&a)) {
                let mut areas: Vec<&str> = app_actions::ACTIONS.iter().map(|a| a.area).collect();
                areas.dedup();
                return Err(format!(
                    "no actions in area `{area}`; areas: {}.",
                    areas.join(", ")
                ));
            }
        }
    }
    let count = rows.len();
    let page: Vec<Value> = rows
        .iter()
        .skip(start)
        .take(limit)
        .map(|(a, _)| {
            let mut row = json!({
                "id": a.id,
                "tier": a.tier.as_str(),
                "summary": summary_line(a.summary),
            });
            if let Some(tool) = tool_for(a.id) {
                row["tool"] = json!(tool);
            }
            // Its arguments on one line, so most calls go straight to
            // run_action without a describe first (overnight 9 A7).
            let args = app_actions::signature(a.params);
            if !args.is_empty() {
                row["args"] = json!(args);
            }
            // Only a level other than stable is shown (C11), so a stable
            // row reads as before.
            let (level, _) = app_actions::stability_of(a.id);
            if level != app_actions::Stability::Stable {
                row["stability"] = json!(level.as_str());
            }
            row
        })
        .collect();
    let mut out = json!({ "count": count, "returned": page.len(), "actions": page });
    // How many of all the matching actions are not stable, only when some
    // are: describe_action says why.
    let not_stable = rows
        .iter()
        .filter(|(a, _)| app_actions::stability_of(a.id).0 != app_actions::Stability::Stable)
        .count();
    if not_stable > 0 {
        out["notStable"] = json!(not_stable);
    }
    let next = start + limit;
    if next < count {
        out["nextCursor"] = json!(next.to_string());
    }
    if count == 0 {
        out["note"] = json!("No action this run may use matches. Try other words, or no filter; an action whose tier this run can't use (plan mode reads only) isn't listed.");
    }
    Ok(out)
}

/// A first-class tool's description: the action's summary, plus what its
/// tier means for the user.
fn tool_description(action: &app_actions::Action) -> String {
    match action.tier {
        Tier::Read => action.summary.to_string(),
        Tier::Write => format!("{} Recorded in this run's History & recovery.", action.summary),
        Tier::Confirm => format!("{} The user approves it on a card in the chat first (unless the chat's permission mode already approves it).", action.summary),
        Tier::External => format!("{} Leaves this Mac: the user approves it on a card that names the destination (none in a bypass-permissions chat).", action.summary),
    }
}

/// Tool results as text: pretty JSON when small, compact when large (the
/// broker already cut it to size).
fn render(value: &Value) -> String {
    let compact = value.to_string();
    if compact.len() > 20_000 {
        return compact;
    }
    serde_json::to_string_pretty(value).unwrap_or(compact)
}

// ── Tool schemas ─────────────────────────────────────────────────────────

fn tool_specs(access: AppAccess, depth: u32) -> Vec<Value> {
    let mut tools: Vec<Value> = FIRST_CLASS
        .iter()
        .filter_map(|(tool, id)| app_actions::find(id).map(|action| (tool, action)))
        .filter(|(_, action)| access.allows(action.tier))
        .filter(|(tool, _)| depth < MAX_START_DEPTH || !STARTERS.contains(tool))
        .map(|(tool, action)| {
            json!({
                "name": tool,
                "description": tool_description(action),
                "annotations": {
                    // run_script's own tier is read (each inner call is
                    // checked on its own), but a script may change things.
                    "readOnlyHint": action.tier == Tier::Read && *tool != "run_script",
                    "destructiveHint": matches!(action.tier, Tier::Confirm | Tier::External),
                },
                "inputSchema": app_actions::input_schema(action.params),
            })
        })
        .map(|mut tool| {
            if tool["name"]
                .as_str()
                .is_some_and(|name| ENVELOPED.contains(&name))
            {
                mcp_envelope::add_schema(&mut tool);
            }
            tool
        })
        .collect();
    tools.extend([
        json!({
            "name": "list_actions",
            "description": "Find the Hollow actions this run may use: pass a query (words matched against the id, the tool name and what it does; best match first) and/or an area or tier. Each row is an id, its tier (read, write, confirm, external) and a one-line summary; a page holds 25 by default (limit up to 100) and nextCursor fetches the next. A row with stability (experimental, internal, deprecated) may change; describe_action says why. describe_action gives one action's arguments and full summary; run_action runs it.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What you want to do, or an action id or tool name (for example \"read skill\", \"usage budget\", \"notes.write_note\")." },
                    "area": { "type": "string", "description": "Only this area (for example sessions, history, settings, vault, usage)." },
                    "tier": { "type": "string", "enum": ["read", "write", "confirm", "external"], "description": "Only actions of this tier." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Rows per page (default 25)." },
                    "cursor": { "type": "string", "description": "The nextCursor of the previous page." }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "describe_action",
            "description": "One action's arguments (a JSON schema), tier and summary.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "An action id from list_actions." } },
                "required": ["id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "run_action",
            "description": "Run one Hollow action by id with its arguments (describe_action lists them). Reads run at once; writes land in this run's History & recovery; confirm and external actions first ask the user on a card in the chat unless its permission mode already approves them.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "An action id from list_actions." },
                    "args": { "type": "object", "description": "The action's arguments." }
                },
                "required": ["id"],
                "additionalProperties": false
            }
        }),
    ]);
    // The hot set loads with the session instead of behind Claude Code's
    // ToolSearch (HB05): other clients should ignore `_meta` (by the MCP
    // spec; not checked against Codex or Grok).
    for tool in &mut tools {
        if tool["name"]
            .as_str()
            .is_some_and(|name| ALWAYS_LOAD.contains(&name))
        {
            tool["_meta"] = json!({ "anthropic/alwaysLoad": true });
        }
    }
    tools
}

/// The app tools Claude Code loads with the session (HB05, evening 10-16),
/// through each tool's `_meta` `anthropic/alwaysLoad`. Claude Code defers
/// every MCP tool behind ToolSearch: 190 of 399 Claude chat turns (48%)
/// called it, 228 calls at p50 1.8 s (p95 8.9 s), each re-reading the whole
/// context, and 397 of 399 selections were exact `select:` calls, led by
/// these six. Whichever of them this run's access and depth list. A $0
/// probe (Claude Code 2.1.x against a local endpoint) showed a flagged tool
/// sent in full and the rest still deferred; the server-level flag is left
/// unset, because it holds startup.
pub(crate) const ALWAYS_LOAD: [&str; 6] = [
    "run_action",
    "list_sessions",
    "read_session",
    "describe_action",
    "start_session",
    "send_to_session",
];

// ── JSON-RPC helpers (same shapes as the other Hollow servers) ───────────

fn result(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } })
}

/// A tool result, and the broker's note (R2a) as a second text block.
fn tool_text(id: Option<Value>, text: &str, note: Option<&str>) -> Value {
    let mut content = vec![json!({ "type": "text", "text": text })];
    if let Some(note) = note {
        content.push(json!({ "type": "text", "text": note }));
    }
    result(id, json!({ "content": content, "isError": false }))
}

/// A refusal: the readable sentence first, as before, then its code and
/// next call (BI5, `mcp_envelope::classify`).
fn tool_error(id: Option<Value>, text: &str) -> Value {
    let content = crate::mcp_envelope::refusal_content(crate::mcp_envelope::Server::App, text);
    result(id, json!({ "content": content, "isError": true }))
}

/// The unknown-action sentence with every action usable, for the refusal
/// table test in `mcp_envelope` (BI5).
#[cfg(test)]
pub(crate) fn unknown_action_for_tests(id: &str) -> String {
    unknown_action_for(id, |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_ledger::Surface;
    use std::cell::RefCell;

    /// BI5: a refusal keeps its sentence as the first block and ends with
    /// the code block; an answer that is not a refusal gains nothing.
    #[test]
    fn refusals_carry_a_code_and_the_next_call() {
        let rec = recorder();
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&rec),
        };
        let code_of = |resp: &Value| -> Value {
            let blocks = resp["result"]["content"].as_array().unwrap();
            assert_eq!(blocks.len(), 2, "{resp}");
            serde_json::from_str::<Value>(blocks[1]["text"].as_str().unwrap()).unwrap()["error"]
                .clone()
        };
        let resp = call("run_action", json!({ "id": "sessions.overveiw" }), &ctx);
        assert!(is_error(&resp));
        assert!(
            text_of(&resp).starts_with("unknown action `sessions.overveiw`"),
            "{resp}"
        );
        let err = code_of(&resp);
        assert_eq!(err["code"], "UNKNOWN_ACTION");
        assert_eq!(err["next"], "list_actions");
        let resp = call(
            "run_action",
            json!({ "id": "sessions.start", "args": { "workspace": "w", "prompt": "p" } }),
            &ctx,
        );
        assert_eq!(code_of(&resp)["code"], "NOT_PERMITTED", "{resp}");
        let resp = call(
            "run_action",
            json!({ "id": "sessions.overview", "args": { "nope": 1 } }),
            &ctx,
        );
        assert_eq!(code_of(&resp)["code"], "INVALID_ARGUMENT", "{resp}");
        assert_eq!(code_of(&resp)["next"], "describe_action");
        let resp = call("no_such_tool", json!({}), &ctx);
        assert_eq!(code_of(&resp)["code"], "UNKNOWN_TOOL", "{resp}");
        let resp = call(
            "list_sessions",
            json!({ "cursor": "c1.3.abcdefabcdef" }),
            &ctx,
        );
        assert_eq!(code_of(&resp)["code"], "CURSOR_INVALID", "{resp}");
        let detached = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: None,
        };
        let resp = call("list_sessions", json!({}), &detached);
        assert_eq!(code_of(&resp)["code"], "UNAVAILABLE", "{resp}");
        let ok = call("run_action", json!({ "id": "sessions.overview" }), &ctx);
        assert!(!is_error(&ok));
        assert_eq!(ok["result"]["content"].as_array().unwrap().len(), 1);
    }

    /// C11: a list_actions row shows its stability only when it is not
    /// stable, with a count of such rows; an all-stable page reads as before.
    #[test]
    fn list_actions_shows_levels_other_than_stable() {
        let page = list_actions(&json!({ "query": "ui.read_tab" }), AppAccess::Write, 0).unwrap();
        let row = page["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "ui.read_tab")
            .cloned()
            .expect("ui.read_tab is listed");
        assert_eq!(row["stability"], "experimental");
        assert!(page["notStable"].as_u64().unwrap() >= 1);
        let page = list_actions(&json!({ "area": "settings" }), AppAccess::Write, 0).unwrap();
        assert!(page.get("notStable").is_none(), "{page}");
        for row in page["actions"].as_array().unwrap() {
            assert!(row.get("stability").is_none(), "{row}");
        }
    }

    /// OC4: start_session and send_to_session take requestId, and the
    /// status read is a catalog action.
    #[test]
    fn the_session_starters_take_a_request_id() {
        for id in ["sessions.start", "sessions.send"] {
            let action = app_actions::find(id).unwrap();
            assert!(action.params.iter().any(|p| p.name == "requestId"), "{id}");
        }
        let status = app_actions::find("sessions.request_status").expect("the status read");
        assert_eq!(status.tier, Tier::Read);
        assert!(status
            .summary
            .contains("Absent is not proof nothing happened"));
    }

    #[test]
    fn a_mistyped_action_id_suggests_the_nearest_real_ones() {
        let all = || app_actions::ACTIONS.iter().map(|a| a.id);
        let any = |_: &app_actions::Action| true;
        assert_eq!(
            nearest_actions("sessions.start_session", all())[0],
            "sessions.start"
        );
        assert!(unknown_action_for("sessions.start_session", any)
            .contains("did you mean `sessions.start`"));
        // Review: never an action the run can't use (a read-only run here).
        let reads = |a: &app_actions::Action| a.tier == app_actions::Tier::Read;
        assert!(!unknown_action_for("sessions.start_session", reads).contains("`sessions.start`"));
        // A first-class tool's name names its action.
        assert!(unknown_action_for("read_session", any).contains("`sessions.conversation`"));
        // One everyday word matches only within the same area.
        assert!(!nearest_actions("sessions.get", all())
            .iter()
            .any(|n| n.starts_with("settings.")));
        // The area alone is no match; nothing close says only where to look.
        assert!(nearest_actions("sessions.zzz", all()).is_empty());
        assert_eq!(
            unknown_action_for("nothing.like_it", any),
            "unknown action `nothing.like_it`; list_actions shows every action id."
        );
        // Two shared words in another area still help; at most three.
        let near = nearest_actions("tasks.agent_models", all());
        assert!(near.contains(&"agents.agent_models"), "{near:?}");
        assert!(near.len() <= 3);
    }

    fn req(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn call(name: &str, args: Value, ctx: &Ctx) -> Value {
        handle(
            &req(1, "tools/call", json!({ "name": name, "arguments": args })),
            ctx,
        )
        .unwrap()
    }

    fn text_of(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    fn is_error(resp: &Value) -> bool {
        resp["result"]["isError"] == json!(true)
    }

    struct Recorder {
        calls: RefCell<Vec<(String, Value)>>,
        via: RefCell<Vec<(String, Value)>>,
    }

    impl Broker for Recorder {
        fn call(&self, action: &str, args: &Value, via: Via<'_>) -> Result<Value, String> {
            self.calls
                .borrow_mut()
                .push((action.to_string(), args.clone()));
            self.via
                .borrow_mut()
                .push((via.tool.to_string(), via.args.clone()));
            Ok(json!({ "ran": action }))
        }
    }

    fn recorder() -> Recorder {
        Recorder {
            calls: RefCell::new(Vec::new()),
            via: RefCell::new(Vec::new()),
        }
    }

    fn listed(access: AppAccess, depth: u32) -> Vec<String> {
        let ctx = Ctx {
            access,
            depth,
            broker: None,
        };
        let listed = handle(&req(1, "tools/list", Value::Null), &ctx).unwrap();
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    /// HB05: only the hot set carries `anthropic/alwaysLoad`, whichever of
    /// it the run's access and depth list; the size it adds is printed for
    /// the PR (`cargo test --lib always_load -- --nocapture`).
    #[test]
    fn only_the_hot_tools_always_load() {
        for (access, depth) in [
            (AppAccess::Write, 0),
            (AppAccess::Read, 0),
            (AppAccess::Write, MAX_START_DEPTH),
        ] {
            let tools = tool_specs(access, depth);
            let mut hot_bytes = 0;
            for tool in &tools {
                let name = tool["name"].as_str().unwrap();
                let flagged = tool["_meta"]["anthropic/alwaysLoad"] == json!(true);
                assert_eq!(flagged, ALWAYS_LOAD.contains(&name), "{name}");
                if flagged {
                    hot_bytes += serde_json::to_string(tool).unwrap().len();
                }
            }
            let listed: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
            // A renamed first-class tool can't drop out of the hot set unseen.
            let hot = ALWAYS_LOAD.iter().filter(|n| listed.contains(n)).count();
            let expected = if access == AppAccess::Write && depth == 0 {
                6
            } else {
                4
            };
            assert_eq!(hot, expected, "{access:?} depth {depth}");
            eprintln!(
                "ALWAYS_LOAD {access:?} depth {depth}: {} of {} tools, {hot_bytes} bytes",
                tools.iter().filter(|t| t.get("_meta").is_some()).count(),
                tools.len()
            );
        }
        // No server-level flag: it would hold the session's startup.
        let ctx = Ctx {
            access: AppAccess::Write,
            depth: 0,
            broker: None,
        };
        let init = handle(&req(1, "initialize", Value::Null), &ctx).unwrap();
        assert!(!init.to_string().contains("alwaysLoad"), "{init}");
    }

    /// Repo features R4: `read_session` is always loaded for Claude and
    /// listed on every Codex turn, so its `query`, `fromTurn` and `toTurn`
    /// may add at most 300 bytes to its schema (measured 2026-10-18: 1096
    /// bytes before R4, 1387 after, as listed with `_meta`), an absolute
    /// ceiling of 1396 bytes. OB1 (2026-11-17) adds the envelope's
    /// `maxBytes` and `cursor` (`mcp_envelope::schema_properties`), at most
    /// 220 bytes more. Prints both sizes (`cargo test --lib
    /// read_session_schema -- --nocapture`).
    #[test]
    fn read_session_schema_stays_within_its_growth_cap() {
        const READ_SESSION_CEILING: usize = 1096 + 300 + 220;
        let tools = tool_specs(AppAccess::Read, 0);
        let tool = tools
            .iter()
            .find(|t| t["name"] == json!("read_session"))
            .unwrap();
        let after = serde_json::to_string(tool).unwrap().len();
        // Without R4's fields, for the printout.
        let mut before = tool.clone();
        let properties = before["inputSchema"]["properties"].as_object_mut().unwrap();
        for field in ["query", "fromTurn", "toTurn"] {
            assert!(properties.remove(field).is_some(), "{field}");
        }
        let before = serde_json::to_string(&before).unwrap().len();
        eprintln!("read_session schema: {before} -> {after} bytes");
        assert!(after <= READ_SESSION_CEILING, "{before} -> {after}");
    }

    /// LF1: the verdict tool is a catalog action, not a new first-class
    /// tool, so the tool list is unchanged; its own schema stays small.
    #[test]
    fn the_verdict_tool_is_in_the_catalog_and_adds_no_tool_to_the_list() {
        for access in [AppAccess::Read, AppAccess::Write] {
            let names = listed(access, 0);
            assert!(names.iter().all(|n| !n.contains("submit")), "{names:?}");
        }
        let action = app_actions::find("sessions.submit_result").expect("the verdict action");
        assert_eq!(action.tier, app_actions::Tier::Write);
        let schema = app_actions::input_schema(action.params);
        let bytes = serde_json::to_string(&schema).unwrap().len();
        eprintln!("sessions.submit_result schema: {bytes} bytes");
        assert!(bytes <= 1800, "{bytes}");
        assert_eq!(schema["required"], json!(["verdict", "reason"]));
        let page = list_actions(&json!({ "query": "submit_result" }), AppAccess::Write, 0).unwrap();
        assert!(
            page.to_string().contains("sessions.submit_result"),
            "{page}"
        );
        // Stable bytes: two renders of the schema are identical.
        assert_eq!(
            schema,
            app_actions::input_schema(app_actions::find("sessions.submit_result").unwrap().params)
        );
    }

    #[test]
    fn tools_list_has_the_first_class_reads_and_the_catalog() {
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: None,
        };
        let raw = handle(&req(1, "tools/list", Value::Null), &ctx).unwrap();
        let read = listed(AppAccess::Read, 0);
        let write = listed(AppAccess::Write, 0);
        assert_eq!(
            &read[read.len() - 3..],
            ["list_actions", "describe_action", "run_action"]
        );
        for (tool, id) in FIRST_CLASS {
            let Some(action) = app_actions::find(id) else {
                continue;
            };
            assert!(
                write.contains(&tool.to_string()),
                "{tool} is listed with write access"
            );
            assert_eq!(
                read.contains(&tool.to_string()),
                action.tier == Tier::Read,
                "{tool}: a read-only run lists reads only"
            );
        }
        for tool in [
            "list_workspaces",
            "read_workspace",
            "list_sessions",
            "read_session",
            "list_automations",
            "read_automation",
            "build_status",
        ] {
            assert!(read.contains(&tool.to_string()), "{tool}");
        }
        // Too deep to start a session: the starters aren't listed.
        let deep = listed(AppAccess::Write, MAX_START_DEPTH);
        for starter in STARTERS {
            assert!(
                write.contains(&starter.to_string()) && !deep.contains(&starter.to_string()),
                "{starter}"
            );
        }
        assert!(deep.contains(&"stop_session".to_string()));
        let tools = raw["result"]["tools"].as_array().unwrap();
        let read_session = tools
            .iter()
            .find(|t| t["name"] == json!("read_session"))
            .unwrap();
        assert_eq!(read_session["inputSchema"]["required"], json!(["key"]));
        assert_eq!(read_session["annotations"]["readOnlyHint"], json!(true));
        let init = handle(
            &req(2, "initialize", json!({ "protocolVersion": "2025-06-18" })),
            &ctx,
        )
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], json!("hollow-app"));
        assert_eq!(init["result"]["protocolVersion"], json!("2025-06-18"));
        assert!(handle(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            &ctx
        )
        .is_none());
        assert_eq!(
            handle(
                &json!({ "jsonrpc": "2.0", "id": 9, "method": "nope" }),
                &ctx
            )
            .unwrap()["error"]["code"],
            json!(-32601)
        );
    }

    /// F10 (D2): `wait_for_session` is a listed read, even in plan mode,
    /// bounded by its own schema, and this server waits for the GUI's answer
    /// to it longer than its longest wait but well within Codex's tool
    /// timeout for this server.
    #[test]
    fn wait_for_session_is_a_bounded_read() {
        for access in [AppAccess::Read, AppAccess::Write] {
            let ctx = Ctx {
                access,
                depth: MAX_START_DEPTH,
                broker: None,
            };
            let raw = handle(&req(1, "tools/list", Value::Null), &ctx).unwrap();
            let tools = raw["result"]["tools"].as_array().unwrap();
            let wait = tools
                .iter()
                .find(|t| t["name"] == json!("wait_for_session"))
                .expect("listed");
            assert_eq!(wait["annotations"]["readOnlyHint"], json!(true));
            // key or keys (overnight OB10a): the handler takes exactly one.
            assert!(wait["inputSchema"].get("required").is_none(), "{wait}");
            assert_eq!(
                wait["inputSchema"]["properties"]["keys"]["type"],
                json!("array")
            );
            assert_eq!(
                wait["inputSchema"]["properties"]["timeoutSeconds"]["maximum"],
                json!(crate::delegation::WAIT_MAX_SECONDS)
            );
            let about = wait["description"].as_str().unwrap();
            assert!(about.contains("default 60, at most 300"), "{about}");
            assert!(
                about.contains("reported once at the start of your next turn"),
                "{about}"
            );
            assert!(
                about.contains("end the turn rather than wait past one call"),
                "{about}"
            );
        }
        let waits = broker_wait_secs("sessions.wait");
        assert_eq!(waits, crate::delegation::WAIT_ANSWER_SECONDS);
        assert!(waits > crate::delegation::WAIT_MAX_SECONDS);
        assert!(waits + crate::delegation::TIMEOUT_MARGIN_SECONDS <= CODEX_TOOL_TIMEOUT_SECS);
        assert_eq!(
            broker_wait_secs("sessions.conversation"),
            BROKER_TIMEOUT_SECS
        );
        assert_eq!(
            broker_wait_secs("sessions.start"),
            BROKER_CHANGE_TIMEOUT_SECS
        );
        assert!(prompt_guidance("acceptEdits", false, false).contains("wait_for_session waits"));
    }

    /// A broker whose `sessions.wait` takes `slow`, and that answers
    /// everything else at once (it is `Sync`, as the socket client is).
    struct SlowWaits {
        slow: std::time::Duration,
    }

    impl Broker for SlowWaits {
        fn call(&self, action: &str, _args: &Value, _via: Via<'_>) -> Result<Value, String> {
            if action == "sessions.wait" {
                std::thread::sleep(self.slow);
            }
            Ok(json!({ "ran": action }))
        }
    }

    /// Records each answer line with when it was written.
    #[derive(Clone)]
    struct Lines(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Lines {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// F10 (D2 review): a Codex primary's tool timeout for this server
    /// counts from when it sends the call, so two `wait_for_session` calls
    /// sent together must both be answered within one wait, not the second
    /// after the first's (on D2's first head the loop answered one request
    /// at a time: 315 s plus 315 s against Codex's 480 s). A call that
    /// doesn't wait is not held behind one that does, and the handshake is
    /// still answered first.
    #[test]
    fn a_second_wait_is_not_queued_behind_the_first() {
        let slow = std::time::Duration::from_millis(400);
        let broker = SlowWaits { slow };
        let input = [
            req(0, "initialize", json!({ "protocolVersion": DEFAULT_PROTOCOL })),
            req(1, "tools/call", json!({ "name": "wait_for_session", "arguments": { "key": "child-a", "timeoutSeconds": 300 } })),
            req(2, "tools/call", json!({ "name": "wait_for_session", "arguments": { "key": "child-b", "timeoutSeconds": 300 } })),
            req(3, "tools/call", json!({ "name": "read_session", "arguments": { "key": "child-a" } })),
        ]
        .iter()
        .map(|r| format!("{r}\n"))
        .collect::<String>();
        let out = Lines(Default::default());
        let started = std::time::Instant::now();
        serve_lines(std::io::Cursor::new(input), out.clone(), |r: &Value| {
            let ctx = Ctx {
                access: AppAccess::Read,
                depth: 0,
                broker: Some(&broker),
            };
            handle(r, &ctx)
        });
        let elapsed = started.elapsed();
        let written = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        let ids: Vec<i64> = written
            .lines()
            .map(|l| {
                serde_json::from_str::<Value>(l).unwrap()["id"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
        assert_eq!(ids.len(), 4, "{written}");
        assert_eq!(ids[0], 0, "the handshake is answered first: {written}");
        assert_eq!(ids[1], 3, "a read isn't held behind the waits: {written}");
        assert_eq!(
            written.matches("sessions.wait").count(),
            2,
            "both waits ran: {written}"
        );
        assert!(
            elapsed < slow * 3 / 2,
            "all answered after {elapsed:?}: the second wait waited for the first ({slow:?} each)"
        );
    }

    /// F10 D1: the durable child task's tools name their contract as
    /// listed to the agent, start_session points a quick question to the
    /// bounded consultation, and the starters take a task packet object.
    #[test]
    fn the_session_tools_name_the_durable_child_task_contract() {
        use crate::delegation::{CHILD_TASK_NAME, CONSULTATION_NAME, PACKET_ABOUT};
        let ctx = Ctx {
            access: AppAccess::Write,
            depth: 0,
            broker: None,
        };
        let raw = handle(&req(1, "tools/list", Value::Null), &ctx).unwrap();
        let tools = raw["result"]["tools"].as_array().unwrap();
        let tool = |name: &str| {
            tools
                .iter()
                .find(|t| t["name"] == json!(name))
                .unwrap_or_else(|| panic!("{name} is listed"))
                .clone()
        };
        for name in ["start_session", "send_to_session", "read_session"] {
            let description = tool(name)["description"].as_str().unwrap().to_string();
            assert!(
                description.contains(CHILD_TASK_NAME),
                "{name}: {description}"
            );
        }
        let start = tool("start_session");
        assert!(
            start["description"]
                .as_str()
                .unwrap()
                .contains(&format!("{CONSULTATION_NAME} (hollow_team ask_agent)")),
            "{start}"
        );
        for name in STARTERS {
            let schema = &tool(name)["inputSchema"];
            assert_eq!(
                schema["properties"]["packet"]["type"],
                json!("object"),
                "{name}"
            );
            assert_eq!(
                schema["properties"]["packet"]["description"],
                json!(PACKET_ABOUT),
                "{name}"
            );
            let required = schema["required"].as_array().unwrap();
            assert!(!required.contains(&json!("packet")), "{name}: optional");
        }
    }

    #[test]
    fn calls_are_forwarded_by_registry_id_and_nothing_else() {
        let rec = recorder();
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&rec),
        };
        let resp = call("read_session", json!({ "key": "s-1" }), &ctx);
        assert!(!is_error(&resp), "{resp}");
        let resp = call(
            "run_action",
            json!({ "id": "usage.usage_summary", "args": { "window": "week" } }),
            &ctx,
        );
        assert!(!is_error(&resp), "{resp}");
        assert_eq!(
            *rec.calls.borrow(),
            vec![
                ("sessions.conversation".to_string(), json!({ "key": "s-1" })),
                (
                    "usage.usage_summary".to_string(),
                    json!({ "window": "week" })
                ),
            ]
        );
        // Unknown ids, the user's own clicks, and bad arguments never reach the broker.
        for (tool, args) in [
            (
                "run_action",
                json!({ "id": "approval_respond", "args": {} }),
            ),
            (
                "run_action",
                json!({ "id": "interactions.resolve_interaction" }),
            ),
            ("run_action", json!({ "id": "../../etc" })),
            ("run_action", json!({})),
            (
                "run_action",
                json!({ "id": "usage.usage_summary", "args": { "window": "year" } }),
            ),
            ("read_session", json!({})),
            ("drop_table", json!({})),
        ] {
            let resp = call(tool, args.clone(), &ctx);
            assert!(is_error(&resp), "{tool} {args}: {resp}");
        }
        assert_eq!(rec.calls.borrow().len(), 2);
        let brokerless = Ctx {
            access: AppAccess::Write,
            depth: 0,
            broker: None,
        };
        assert!(
            text_of(&call("list_workspaces", json!({}), &brokerless)).contains("isn't connected")
        );
    }

    #[test]
    fn the_catalog_answers_locally() {
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: None,
        };
        let all: Value =
            serde_json::from_str(&text_of(&call("list_actions", json!({}), &ctx))).unwrap();
        let reads = app_actions::ACTIONS
            .iter()
            .filter(|a| a.tier == Tier::Read)
            .count();
        assert_eq!(
            all["count"],
            json!(reads),
            "a read-only run is offered reads"
        );
        let settings: Value = serde_json::from_str(&text_of(&call(
            "list_actions",
            json!({ "area": "settings", "limit": 100 }),
            &ctx,
        )))
        .unwrap();
        assert!(settings["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["id"].as_str().unwrap().starts_with("settings.")));
        assert!(settings["count"].as_u64().unwrap() > 5);
        assert!(text_of(&call("list_actions", json!({ "area": "nope" }), &ctx)).contains("areas:"));
        let described: Value = serde_json::from_str(&text_of(&call(
            "describe_action",
            json!({ "id": "vault.vault_note" }),
            &ctx,
        )))
        .unwrap();
        assert_eq!(described["inputSchema"]["required"], json!(["path"]));
        assert_eq!(described["available"], json!(true));
        assert!(is_error(&call(
            "describe_action",
            json!({ "id": "nope" }),
            &ctx
        )));
    }

    fn catalog_page(args: Value, ctx: &Ctx) -> Value {
        let resp = call("list_actions", args.clone(), ctx);
        assert!(!is_error(&resp), "{args}: {resp}");
        serde_json::from_str(&text_of(&resp)).unwrap()
    }

    fn page_ids(page: &Value) -> Vec<String> {
        page["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// F9 (wave 4 · V2): a task finds its action by words, a page at a
    /// time, with one bounded line each; `describe_action` has the rest.
    #[test]
    fn list_actions_finds_by_query_and_pages_with_bounded_lines() {
        let ctx = Ctx {
            access: AppAccess::Write,
            depth: 0,
            broker: None,
        };
        // The id, the first-class tool name (its title) and the summary
        // all match; the best match comes first.
        let skills = catalog_page(json!({ "query": "read skill" }), &ctx);
        assert_eq!(page_ids(&skills)[0], "skills.read_skill", "{skills}");
        let titled = catalog_page(json!({ "query": "read_session" }), &ctx);
        assert_eq!(page_ids(&titled)[0], "sessions.conversation", "{titled}");
        assert_eq!(titled["actions"][0]["tool"], json!("read_session"));
        let described = catalog_page(json!({ "query": "usage budget" }), &ctx);
        assert!(
            page_ids(&described)[..3]
                .iter()
                .any(|id| id.contains("budget")),
            "{described}"
        );
        // A default page, bounded one-line summaries and a cursor.
        let first = catalog_page(json!({}), &ctx);
        assert_eq!(first["actions"].as_array().unwrap().len(), DEFAULT_PAGE);
        assert_eq!(first["returned"], json!(DEFAULT_PAGE));
        assert_eq!(first["nextCursor"], json!(DEFAULT_PAGE.to_string()));
        let total = first["count"].as_u64().unwrap() as usize;
        assert!(total > DEFAULT_PAGE);
        // Walking every page with the largest limit visits each action once.
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut args = json!({ "limit": 1000 });
            if let Some(c) = &cursor {
                args["cursor"] = json!(c);
            }
            let page = catalog_page(args, &ctx);
            let ids = page_ids(&page);
            assert!(ids.len() <= MAX_PAGE, "a page is at most {MAX_PAGE}");
            for row in page["actions"].as_array().unwrap() {
                let line = row["summary"].as_str().unwrap();
                assert!(
                    line.chars().count() <= SUMMARY_LINE_CHARS && !line.contains('\n'),
                    "{line}"
                );
                let id = row["id"].as_str().unwrap();
                let full = app_actions::find(id).unwrap().summary;
                assert!(full.starts_with(line.trim_end_matches('…')), "{id}");
            }
            seen.extend(ids);
            match page["nextCursor"].as_str() {
                Some(next) => cursor = Some(next.to_string()),
                None => break,
            }
        }
        assert_eq!(seen.len(), total);
        let mut unique = seen.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), total, "no action twice");
        // Filters: tier and area combine with the query.
        let confirm = catalog_page(json!({ "tier": "confirm", "limit": 100 }), &ctx);
        assert!(confirm["actions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["tier"] == json!("confirm")));
        let in_area = catalog_page(json!({ "area": "skills", "query": "skill" }), &ctx);
        assert!(page_ids(&in_area)
            .iter()
            .all(|id| id.starts_with("skills.")));
        // Bad arguments are refused, not ignored.
        for args in [
            json!({ "cursor": "nope" }),
            json!({ "tier": "admin" }),
            json!({ "query": "a" }),
        ] {
            assert!(
                is_error(&call("list_actions", args.clone(), &ctx)),
                "{args}"
            );
        }
        // The full detail stays with describe_action.
        let detail: Value = serde_json::from_str(&text_of(&call(
            "describe_action",
            json!({ "id": "skills.read_skill" }),
            &ctx,
        )))
        .unwrap();
        assert!(detail["inputSchema"]["properties"]["path"].is_object());
    }

    /// Overnight 9 A7: each listed row carries its arguments on one bounded
    /// line, matching the action's parameters, so run_action needs no
    /// describe first; a row for an action with none has no `args`.
    #[test]
    fn list_actions_rows_carry_bounded_argument_signatures() {
        let ctx = Ctx {
            access: AppAccess::Write,
            depth: 0,
            broker: None,
        };
        let page = catalog_page(json!({ "query": "read skill" }), &ctx);
        let row = &page["actions"][0];
        assert_eq!(row["id"], json!("skills.read_skill"));
        assert!(row["args"].as_str().unwrap().contains("path"), "{row}");
        let mut cursor: Option<String> = None;
        let (mut with, mut without) = (0, 0);
        loop {
            let mut args = json!({ "limit": MAX_PAGE });
            if let Some(c) = &cursor {
                args["cursor"] = json!(c);
            }
            let page = catalog_page(args, &ctx);
            for row in page["actions"].as_array().unwrap() {
                let action = app_actions::find(row["id"].as_str().unwrap()).unwrap();
                match row["args"].as_str() {
                    None => {
                        assert!(action.params.is_empty(), "{row}");
                        without += 1;
                    }
                    Some(sig) => {
                        assert!(sig.len() <= app_actions::SIGNATURE_MAX_BYTES, "{sig}");
                        assert!(!sig.contains('\n'));
                        let first = action.params[0];
                        let star = if first.required { "*" } else { "" };
                        assert!(sig.starts_with(&format!("{}{star}", first.name)), "{sig}");
                        with += 1;
                    }
                }
            }
            match page["nextCursor"].as_str() {
                Some(next) => cursor = Some(next.to_string()),
                None => break,
            }
        }
        assert!(with > 0 && without > 0, "{with} with, {without} without");
    }

    /// F9 (wave 4 · V2): what this run can't use isn't advertised: a
    /// read-only run lists reads only, and a run too deep to start a session
    /// doesn't list the starters.
    #[test]
    fn list_actions_never_advertises_what_this_run_cannot_use() {
        let all = |ctx: &Ctx| -> Vec<Value> {
            let mut rows = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let mut args = json!({ "limit": MAX_PAGE });
                if let Some(c) = &cursor {
                    args["cursor"] = json!(c);
                }
                let page = catalog_page(args, ctx);
                rows.extend(page["actions"].as_array().unwrap().iter().cloned());
                match page["nextCursor"].as_str() {
                    Some(next) => cursor = Some(next.to_string()),
                    None => return rows,
                }
            }
        };
        let read = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: None,
        };
        let rows = all(&read);
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|a| a["tier"] == json!("read")),
            "reads only"
        );
        let hidden = catalog_page(json!({ "query": "write_user_skill" }), &read);
        assert!(!page_ids(&hidden).contains(&"skills.write_user_skill".to_string()));
        let deep = Ctx {
            access: AppAccess::Write,
            depth: MAX_START_DEPTH,
            broker: None,
        };
        let ids: Vec<String> = all(&deep)
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect();
        for starter in ["sessions.start", "sessions.send"] {
            assert!(!ids.contains(&starter.to_string()), "{starter}");
        }
        assert!(ids.contains(&"sessions.stop".to_string()));
        let shallow = Ctx {
            access: AppAccess::Write,
            depth: 0,
            broker: None,
        };
        let ids: Vec<String> = all(&shallow)
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_string())
            .collect();
        assert!(ids.contains(&"sessions.start".to_string()));
        assert_eq!(
            ids.len(),
            app_actions::ACTIONS.len(),
            "a full grant lists everything"
        );
    }

    #[test]
    fn workflow_access_never_widens_a_restricted_allowlist() {
        let list = |tools: &[&str]| tools.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        use AppAccess::*;
        assert_eq!(AppAccess::for_workflow("acceptEdits", &[]), Some(Write));
        assert_eq!(AppAccess::for_workflow("plan", &[]), Some(Read));
        assert_eq!(
            AppAccess::for_workflow("acceptEdits", &list(&["", " "])),
            Some(Write),
            "blank entries are no restriction"
        );
        assert_eq!(
            AppAccess::for_workflow("acceptEdits", &list(&["Read", "Write"])),
            None,
            "restricted and not named: not attached"
        );
        assert_eq!(
            AppAccess::for_workflow("acceptEdits", &list(&["Read", "mcp__hollow_app"])),
            Some(Write)
        );
        assert_eq!(
            AppAccess::for_workflow("plan", &list(&["mcp__hollow_app"])),
            Some(Read)
        );
        assert_eq!(
            AppAccess::for_workflow("acceptEdits", &list(&["mcp__hollow_app__list_sessions"])),
            Some(Read)
        );
        assert_eq!(
            AppAccess::for_workflow("acceptEdits", &list(&["mcp__hollow_tasks"])),
            None
        );
        assert_eq!(AppAccess::for_chat("plan"), Read);
        assert_eq!(AppAccess::for_chat("acceptEdits"), Write);
        for access in [Read, Write] {
            assert_eq!(AppAccess::parse(access.as_str()), Some(access));
        }
        assert_eq!(AppAccess::parse("admin"), None);
        assert!(
            Read.allows(Tier::Read) && !Read.allows(Tier::Write) && !Read.allows(Tier::Confirm)
        );
        assert!(Write.allows(Tier::Confirm));
    }

    #[test]
    fn server_entry_carries_the_run_identity_depth_and_proof() {
        let parent = DelegateParent {
            surface: Surface::Chat,
            scope: "s-1".into(),
            run_id: "R1".into(),
            scratch: None,
            tool_limits: Default::default(),
        };
        let (name, entry) = entry_for(
            Path::new("/Applications/Hollow.app/Contents/MacOS/hollow"),
            Path::new("/tmp/hollow/app-7.sock"),
            &parent,
            &AppGrant::new(AppAccess::Write, 0, "auto"),
            "abc123",
        );
        assert_eq!(name, "hollow_app");
        assert_eq!(entry["args"], json!(["app-mcp"]));
        let env = entry["env"].as_object().unwrap();
        assert_eq!(env["HOLLOW_APP_SOCK"], json!("/tmp/hollow/app-7.sock"));
        assert_eq!(env["HOLLOW_RUN_ID"], json!("R1"));
        assert_eq!(env["HOLLOW_RUN_SURFACE"], json!("chat"));
        assert_eq!(env["HOLLOW_RUN_SCOPE"], json!("s-1"));
        assert_eq!(env["HOLLOW_APP_ACCESS"], json!("write"));
        assert_eq!(env["HOLLOW_APP_DEPTH"], json!("0"));
        assert_eq!(env["HOLLOW_APP_PERM"], json!("auto"));
        assert_eq!(env["HOLLOW_APP_PROOF"], json!("abc123"));
        for key in env.keys() {
            for filtered in ["KEY", "SECRET", "TOKEN"] {
                assert!(!key.contains(filtered), "Codex strips {key}");
            }
        }
        // No broker in this test process, or the switch off: nothing attaches.
        let grant = AppGrant::new(AppAccess::Write, 0, "acceptEdits");
        assert!(server_entry(&parent, &grant).is_none());
        with_attach_override(false, Some(PathBuf::from("/tmp/x.sock")), || {
            assert!(server_entry(&parent, &grant).is_none())
        });
        let identity = RunIdentity {
            run_id: "R".into(),
            surface: "chat".into(),
            scope: "s".into(),
            access: AppAccess::Read,
            depth: 1,
            perm: "plan".into(),
            proof: "p".into(),
        };
        let via_args = json!({ "id": "vault.vault_tree" });
        assert_eq!(
            identity.request(
                "vault.vault_tree",
                &json!({}),
                Via {
                    tool: "run_action",
                    args: &via_args
                }
            ),
            json!({ "v": 1, "action": "vault.vault_tree", "runId": "R", "surface": "chat", "scope": "s", "access": "read", "depth": 1, "perm": "plan", "proof": "p", "args": {}, "tool": "run_action", "toolArgs": { "id": "vault.vault_tree" } })
        );
        // An unknown or empty mode is normalized to acceptEdits, never to bypass.
        assert_eq!(AppGrant::new(AppAccess::Write, 0, "").perm, "acceptEdits");
        assert_eq!(
            AppGrant::new(AppAccess::Write, 0, "root").perm,
            "acceptEdits"
        );
        assert!(
            perm_rank("plan") < perm_rank("default")
                && perm_rank("acceptEdits") < perm_rank("auto")
                && perm_rank("auto") < perm_rank("bypassPermissions")
        );
    }

    /// A turn Hollow starts itself with no mode: the app's default in the
    /// app (tests never serve); in serve the stored mode, never wider than
    /// that default, else Plan (M1 review round 3).
    #[test]
    fn an_unrecorded_turn_in_serve_is_never_wider_than_the_apps_default() {
        assert!(!crate::serve::serving());
        assert_eq!(unrecorded_mode(), DEFAULT_MODE);
        assert_eq!(own_mode(None), DEFAULT_MODE);
        assert_eq!(unrecorded_mode_in_serve(None), "plan");
        assert_eq!(unrecorded_mode_in_serve(Some("nonsense")), "plan");
        assert_eq!(unrecorded_mode_in_serve(Some("plan")), "plan");
        assert_eq!(unrecorded_mode_in_serve(Some("default")), "default");
        assert_eq!(unrecorded_mode_in_serve(Some("acceptEdits")), "acceptEdits");
        assert_eq!(
            unrecorded_mode_in_serve(Some("bypassPermissions")),
            "acceptEdits"
        );
        assert_eq!(unrecorded_mode_in_serve(Some("auto")), "acceptEdits");
    }

    /// 2026-11-12: a chat's own mode is kept as it is (bypass and auto
    /// too), normalized; none recorded or an unknown one is Hollow's default.
    /// A cap only ever lowers it.
    #[test]
    fn a_chats_own_mode_is_kept_and_a_cap_only_lowers_it() {
        for mode in PERMISSION_MODES {
            assert_eq!(own_mode(Some(mode)), mode);
            assert_eq!(own_mode(Some(format!(" {mode} ").as_str())), mode);
            assert_eq!(lowered(mode, None), mode);
            for cap in PERMISSION_MODES {
                let want = if perm_rank(cap) < perm_rank(mode) {
                    cap
                } else {
                    mode
                };
                assert_eq!(lowered(mode, Some(cap)), want, "{mode} capped by {cap}");
            }
        }
        assert_eq!(own_mode(None), DEFAULT_MODE);
        assert_eq!(own_mode(Some("nonsense")), DEFAULT_MODE);
        assert_eq!(DEFAULT_MODE, "acceptEdits");
        // An unknown cap is the default, never stronger than the mode.
        assert_eq!(lowered("bypassPermissions", Some("root")), "acceptEdits");
        assert_eq!(lowered("plan", Some("root")), "plan");
    }

    #[test]
    fn prompt_guidance_is_path_free_and_names_the_tools() {
        let registered: Vec<&str> = FIRST_CLASS
            .iter()
            .filter(|(_, id)| app_actions::find(id).is_some())
            .map(|(t, _)| *t)
            .collect();
        for perm in PERMISSION_MODES {
            for automation in [false, true] {
                for read_only in [false, true] {
                    let text = prompt_guidance(perm, automation, read_only);
                    assert!(text.starts_with("Hollow app tools (hollow_app)"));
                    assert!(!text.contains('/') && !text.contains('\n'), "{text}");
                    assert_eq!(text, prompt_guidance(perm, automation, read_only));
                    for tool in ["list_actions", "run_action", "describe_action"] {
                        assert!(text.contains(tool), "{tool}");
                    }
                    for tool in &registered {
                        let is_read = app_actions::find(
                            FIRST_CLASS.iter().find(|(t, _)| t == tool).unwrap().1,
                        )
                        .unwrap()
                        .tier
                            == Tier::Read;
                        assert_eq!(
                            text.contains(tool),
                            is_read || !read_only,
                            "{perm} {automation} {read_only}: {tool}"
                        );
                    }
                }
            }
        }
    }

    /// OB16's trim: a chat's writable app guidance leaves out how a started
    /// session's answer is read, because the durable child task line that
    /// always follows it says so; a workflow's keeps it.
    #[test]
    fn a_chat_reads_how_to_follow_a_child_once() {
        let chat = chat_prompt_guidance(false);
        assert!(!chat.contains(CHILD_ANSWER.trim()), "{chat}");
        let delegating = crate::delegation::guidance(false, true).unwrap();
        for said in [
            "runs on its own",
            "wait_for_session waits",
            "read_session reads its answer",
        ] {
            assert!(delegating.contains(said), "{said}");
        }
        assert!(prompt_guidance("acceptEdits", false, false).contains(CHILD_ANSWER));
    }

    /// Repo features R2a: the note a broker answer carried goes once, as a
    /// second text block after the tool's own result; a broker with no note
    /// (every existing one) still gives one block.
    #[test]
    fn a_brokers_note_is_sent_once_as_a_second_text_block() {
        struct Noting(RefCell<Option<String>>);
        impl Broker for Noting {
            fn call(&self, action: &str, _args: &Value, _via: Via<'_>) -> Result<Value, String> {
                Ok(json!({ "ran": action }))
            }
            fn take_note(&self) -> Option<String> {
                self.0.borrow_mut().take()
            }
        }
        let noting = Noting(RefCell::new(Some("[Hollow: a note.]".into())));
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&noting),
        };
        let run = |ctx: &Ctx| call("run_action", json!({ "id": "sessions.context" }), ctx);
        let first = run(&ctx);
        let blocks = first["result"]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2, "{first}");
        assert!(text_of(&first).contains("sessions.context"), "{first}");
        assert_eq!(
            blocks[1],
            json!({ "type": "text", "text": "[Hollow: a note.]" })
        );
        let second = run(&ctx);
        assert_eq!(second["result"]["content"].as_array().unwrap().len(), 1);
        let rec = recorder();
        let plain = run(&Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&rec),
        });
        assert_eq!(plain["result"]["content"].as_array().unwrap().len(), 1);
        // The answer line's note, read beside its result.
        assert_eq!(
            parse_app_answer(r#"{"ok":true,"result":{"a":1},"note":"[Hollow: n.]"}"#),
            Ok((json!({ "a": 1 }), Some("[Hollow: n.]".into())))
        );
        assert_eq!(
            parse_app_answer(r#"{"ok":true,"result":1}"#),
            Ok((json!(1), None))
        );
        assert_eq!(
            parse_app_answer(r#"{"ok":false,"error":"no","note":"x"}"#),
            Err("no".into())
        );
    }

    /// R2a: a socket client keeps every note its answers carried until a
    /// result sends them, then forgets them.
    #[cfg(unix)]
    #[test]
    fn a_socket_client_keeps_its_notes_until_taken() {
        let client = SocketClient {
            path: PathBuf::from("/nonexistent/hollow-app.sock"),
            identity: RunIdentity {
                run_id: "run-1".into(),
                surface: "chat".into(),
                scope: "s-1".into(),
                access: AppAccess::Read,
                depth: 0,
                perm: String::new(),
                proof: String::new(),
            },
            note: std::sync::Mutex::new(None),
        };
        assert_eq!(client.take_note(), None);
        client.keep_note("one".into());
        client.keep_note("two".into());
        assert_eq!(client.take_note().as_deref(), Some("one\ntwo"));
        assert_eq!(client.take_note(), None);
    }

    /// The subprocess forwards; it never runs a handler, reaches the GUI's
    /// state or the vault, or starts the broker. Needles are built with
    /// `concat!` so this file never matches itself.
    #[test]
    fn app_mcp_never_acts_itself() {
        let source = include_str!("app_mcp.rs");
        for needle in [
            concat!(".r", "un)("),
            concat!("(action.", "run"),
            concat!("app_broker::", "dispatch"),
            concat!("app_broker::", "start"),
            concat!("commands", "::"),
            concat!("automated_", "history::"),
            concat!("atomic_", "write"),
            concat!("std::fs::", "write"),
        ] {
            assert!(
                !source.contains(needle),
                "app_mcp.rs must not contain `{needle}`"
            );
        }
    }

    /// OB1: `list_sessions` detail over its budget comes back under it, the
    /// rows past the whole ones headers only, with a cursor that pages the
    /// rest; an old caller's small answer is the same text with the
    /// envelope block after it; the envelope's arguments never reach the
    /// broker; a cursor from other arguments is refused.
    #[test]
    fn list_sessions_detail_stays_under_its_budget_with_a_cursor_for_the_rest() {
        struct Sessions(RefCell<Vec<Value>>);
        impl Broker for Sessions {
            fn call(&self, _action: &str, args: &Value, _via: Via<'_>) -> Result<Value, String> {
                self.0.borrow_mut().push(args.clone());
                let n: usize = if args["detail"] == json!(true) { 30 } else { 2 };
                let n = n.min(args["limit"].as_u64().unwrap_or(200) as usize);
                let rows: Vec<Value> = (0..n)
                    .map(|i| json!({ "key": format!("s-{i}"), "title": "t", "stints": "x".repeat(1_700) }))
                    .collect();
                Ok(json!({ "sessions": rows, "live": [], "runQueue": [], "runQueueTotal": 0 }))
            }
        }
        let broker = Sessions(RefCell::new(Vec::new()));
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&broker),
        };
        let envelope_of = |resp: &Value| -> Value {
            let blocks = resp["result"]["content"].as_array().unwrap();
            let last: Value =
                serde_json::from_str(blocks.last().unwrap()["text"].as_str().unwrap()).unwrap();
            last["envelope"].clone()
        };
        // An old caller, small answer: the same text, then the envelope.
        let small = call("list_sessions", json!({}), &ctx);
        let rows: Vec<Value> = (0..2)
            .map(|i| json!({ "key": format!("s-{i}"), "title": "t", "stints": "x".repeat(1_700) }))
            .collect();
        let want = json!({ "sessions": rows, "live": [], "runQueue": [], "runQueueTotal": 0 });
        assert_eq!(text_of(&small), render(&want));
        assert_eq!(small["result"]["content"].as_array().unwrap().len(), 2);
        assert_eq!(envelope_of(&small)["complete"], json!(true));
        // Detail: 30 rows of about 1.7 KB is over the 24 KB budget.
        let first = call("list_sessions", json!({ "detail": true }), &ctx);
        assert!(!is_error(&first), "{first}");
        assert!(text_of(&first).len() <= mcp_envelope::DEFAULT_MAX_BYTES);
        let env = envelope_of(&first);
        assert_eq!(env["budgeted"], json!(true), "{env}");
        assert_eq!(env["complete"], json!(false), "{env}");
        assert_eq!(env["capReasons"][0], json!("byte_budget"), "{env}");
        let page: Value = serde_json::from_str(&text_of(&first)).unwrap();
        let listed = page["sessions"].as_array().unwrap();
        assert!(listed.len() < 30 && listed.len() > 1, "{}", listed.len());
        assert!(listed[0].get("stints").is_some(), "whole rows first");
        assert_eq!(
            listed.last().unwrap()["headerOnly"],
            json!(true),
            "then headers"
        );
        let mut whole: Vec<String> = Vec::new();
        let mut collect = |text: &str| {
            let page: Value = serde_json::from_str(text).unwrap();
            for row in page["sessions"].as_array().unwrap() {
                if row.get("stints").is_some() {
                    whole.push(row["key"].as_str().unwrap().to_string());
                }
            }
        };
        collect(&text_of(&first));
        let mut cursor = env["nextCursor"].as_str().map(str::to_string);
        while let Some(c) = cursor {
            let next = call(
                "list_sessions",
                json!({ "detail": true, "cursor": c }),
                &ctx,
            );
            assert!(!is_error(&next), "{next}");
            assert!(text_of(&next).len() <= mcp_envelope::DEFAULT_MAX_BYTES);
            collect(&text_of(&next));
            cursor = envelope_of(&next)["nextCursor"]
                .as_str()
                .map(str::to_string);
        }
        assert_eq!(whole, (0..30).map(|i| format!("s-{i}")).collect::<Vec<_>>());
        // Bigger on request; exactly `limit` sessions is no row limit.
        let big = call(
            "list_sessions",
            json!({ "detail": true, "maxBytes": 80_000 }),
            &ctx,
        );
        assert_eq!(envelope_of(&big)["budgeted"], json!(false));
        assert_eq!(envelope_of(&big)["complete"], json!(true), "{big}");
        // Fewer than exist: row_limit, and the text holds `limit` rows.
        let cut = call(
            "list_sessions",
            json!({ "detail": true, "limit": 10, "maxBytes": 80_000 }),
            &ctx,
        );
        assert_eq!(
            envelope_of(&cut)["capReasons"],
            json!(["row_limit"]),
            "{cut}"
        );
        let page: Value = serde_json::from_str(&text_of(&cut)).unwrap();
        assert_eq!(page["sessions"].as_array().unwrap().len(), 10);
        // The broker never saw maxBytes or cursor.
        assert!(broker
            .0
            .borrow()
            .iter()
            .all(|a| a.get("maxBytes").is_none() && a.get("cursor").is_none()));
        // A cursor from other arguments, and a near-miss name.
        let other = call(
            "list_sessions",
            json!({ "detail": true, "limit": 5, "cursor": env["nextCursor"] }),
            &ctx,
        );
        assert!(
            is_error(&other) && text_of(&other).contains("another query"),
            "{other}"
        );
        let miss = call("list_sessions", json!({ "max_bytes": 9000 }), &ctx);
        assert!(text_of(&miss).contains("`maxBytes`"), "{miss}");
        // Both tools list the two arguments.
        for tool in tool_specs(AppAccess::Read, 0) {
            if ENVELOPED.contains(&tool["name"].as_str().unwrap()) {
                assert!(tool["inputSchema"]["properties"]["maxBytes"].is_object());
                assert!(tool["inputSchema"]["properties"]["cursor"].is_object());
            }
        }
    }

    /// OB1: an unknown argument to an enveloped tool names the envelope's
    /// two as well.
    #[test]
    fn an_enveloped_tools_unknown_argument_names_the_envelope_args() {
        let rec = recorder();
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&rec),
        };
        let resp = call("list_sessions", json!({ "pages": 2 }), &ctx);
        assert!(is_error(&resp));
        assert!(text_of(&resp).contains("maxBytes and cursor"), "{resp}");
    }

    /// OB1: `read_session` over its budget keeps the newest messages whole
    /// and lists older ones headers only, each with its turn number.
    #[test]
    fn read_session_keeps_the_newest_turns_whole_under_its_budget() {
        struct Long;
        impl Broker for Long {
            fn call(&self, _action: &str, _args: &Value, _via: Via<'_>) -> Result<Value, String> {
                let turns: Vec<Value> = (1..=6)
                    .flat_map(|n| {
                        [
                            json!({ "id": format!("s-1-{n}-user"), "role": "user", "text": "q" }),
                            json!({ "id": format!("s-1-{n}-assistant"), "role": "assistant", "text": "a".repeat(6_000) }),
                        ]
                    })
                    .collect();
                Ok(json!({ "turns": turns, "totalTurns": 6, "returnedTurns": 6 }))
            }
        }
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&Long),
        };
        let resp = call("read_session", json!({ "key": "s-1" }), &ctx);
        assert!(text_of(&resp).len() <= mcp_envelope::DEFAULT_MAX_BYTES);
        let page: Value = serde_json::from_str(&text_of(&resp)).unwrap();
        let turns = page["turns"].as_array().unwrap();
        let last = turns.last().unwrap();
        assert_eq!(last["id"], json!("s-1-6-assistant"));
        assert!(last.get("text").is_some(), "the newest answer is whole");
        assert_eq!(
            turns[0]["headerOnly"],
            json!(true),
            "older ones headers only"
        );
        let header = turns
            .iter()
            .find(|t| t["headerOnly"] == json!(true) && t["role"] == json!("assistant"))
            .unwrap();
        assert!(header["turn"].as_u64().is_some_and(|n| n < 6), "{header}");
        assert_eq!(header["textBytes"], json!(6_000));
        // The cursor pages back to the oldest turn.
        let mut cursor = {
            let blocks = resp["result"]["content"].as_array().unwrap();
            let env: Value =
                serde_json::from_str(blocks.last().unwrap()["text"].as_str().unwrap()).unwrap();
            env["envelope"]["nextCursor"].as_str().map(str::to_string)
        };
        let mut oldest_whole = None;
        while let Some(c) = cursor {
            let page = call("read_session", json!({ "key": "s-1", "cursor": c }), &ctx);
            assert!(!is_error(&page), "{page}");
            let parsed: Value = serde_json::from_str(&text_of(&page)).unwrap();
            if let Some(first) = parsed["turns"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t.get("text").is_some())
            {
                oldest_whole = Some(first["id"].clone());
            }
            let blocks = page["result"]["content"].as_array().unwrap();
            let env: Value =
                serde_json::from_str(blocks.last().unwrap()["text"].as_str().unwrap()).unwrap();
            cursor = env["envelope"]["nextCursor"].as_str().map(str::to_string);
        }
        assert_eq!(oldest_whole, Some(json!("s-1-1-user")));
    }

    /// OB1 review: a child's 30 KB answer, alone over the default budget,
    /// comes back whole (the broker's own 80000-character cut still
    /// applies), flagged `item_over_budget`, never as a header whose reader
    /// is held to the same budget.
    #[test]
    fn read_session_returns_a_long_newest_answer_whole() {
        struct OneLong;
        impl Broker for OneLong {
            fn call(&self, _action: &str, _args: &Value, _via: Via<'_>) -> Result<Value, String> {
                Ok(json!({
                    "turns": [
                        { "id": "c-1-1-user", "role": "user", "text": "go" },
                        { "id": "c-1-1-assistant", "role": "assistant", "text": "z".repeat(30_000) },
                    ],
                    "totalTurns": 1,
                    "returnedTurns": 1,
                }))
            }
        }
        let ctx = Ctx {
            access: AppAccess::Read,
            depth: 0,
            broker: Some(&OneLong),
        };
        let resp = call("read_session", json!({ "key": "c-1" }), &ctx);
        let page: Value = serde_json::from_str(&text_of(&resp)).unwrap();
        let turns = page["turns"].as_array().unwrap();
        let answer = turns.last().unwrap();
        assert_eq!(answer["text"].as_str().unwrap().len(), 30_000, "whole");
        let blocks = resp["result"]["content"].as_array().unwrap();
        let env: Value =
            serde_json::from_str(blocks.last().unwrap()["text"].as_str().unwrap()).unwrap();
        let reasons = env["envelope"]["capReasons"].to_string();
        assert!(reasons.contains("item_over_budget"), "{env}");
        // The prompt comes on the next page, whole.
        let cursor = env["envelope"]["nextCursor"].as_str().unwrap().to_string();
        let next = call(
            "read_session",
            json!({ "key": "c-1", "cursor": cursor }),
            &ctx,
        );
        let page: Value = serde_json::from_str(&text_of(&next)).unwrap();
        assert_eq!(page["turns"][0]["text"], json!("go"));
    }
}
