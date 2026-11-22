//! In-process MCP **stdio server** that gives the embedded chat agent vetted
//! computer-control verbs — the "Jarvis" surface. Tier 1 (`open_url`,
//! `open_app`, `list_apps`) launches things through `/usr/bin/open` with
//! Rust-side validation and no prompt; Tier 2 (`browser_current_tab`,
//! `browser_list_tabs`, `quit_app`) drives AppleScript and is gated behind a
//! native macOS Allow/Deny dialog per call.
//!
//! Launched as the hidden `hollow computer-mcp` subcommand (see `main.rs`),
//! which chat `claude` runs register via `--mcp-config` alongside the
//! teamwork server (see [`crate::team_mcp`] — same JSON-RPC-over-stdio
//! skeleton). Chat-only by design: workflows run unattended, where a confirm
//! dialog can't be answered and desktop side effects aren't wanted.
//!
//! Security model — the embedded agent reads synced external content
//! (emails, Slack, Teams), so prompt injection is the threat to design for:
//! - **No free-form scripting tool.** Fixed verbs only; every AppleScript is
//!   composed here from validated parts, never model-supplied.
//! - **URLs are http/https only** (no `file://` / `javascript:` schemes).
//! - **App names resolve against what's actually installed** — never an
//!   arbitrary path.
//! - **Tier 2 requires a click on a native dialog** before anything runs.
//! - **Every call is appended to `{vault}/.computer-actions.jsonl`** (dot
//!   prefix → ignored by the vault watcher).
//! - The chat spawn additionally disallows `Bash(osascript:*)` /
//!   `Bash(open:*)` while this server is attached (see [`BASH_DENY`] and
//!   `chat.rs`), so these vetted verbs are the agent's one path to desktop
//!   control. Defense-in-depth, not a hard boundary.
//!
//! Browsers (PER-35): Safari, Chrome, Arc, Edge, Brave and Helium by name,
//! plus any running Chromium app whose bundle declares Chrome's AppleScript
//! dictionary (found from `ps` and its Info.plist and sdef, no Apple Events,
//! so the Automation prompt only ever names the browser a tool drives).
//! Firefox and LibreWolf have no AppleScript tab dictionary; their session
//! file (`recovery.jsonlz4`) needs an LZ4 decoder and LibreWolf often doesn't
//! keep one, so they are left out and the tool descriptions say so.

use serde_json::{json, Value};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Default MCP protocol version echoed when a client doesn't send one.
const DEFAULT_PROTOCOL: &str = "2024-11-05";
/// How long the Tier-2 confirm dialog waits before auto-denying.
const CONFIRM_TIMEOUT_SECS: u64 = 120;
/// Wall-clock cap on a single AppleScript / launcher invocation.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);
/// Cap on tabs reported by `browser_list_tabs` so a tab-hoarder window can't
/// flood the model's context.
const MAX_TABS: usize = 40;

/// The Tier-2 tools, each gated behind an Allow/Deny approval. The approval
/// broker binds a request to the agent's `mcp__hollow_computer__<tool>` call
/// only for these names (Paperclip PR 21, [`crate::approvals`]).
pub const CONFIRM_TOOLS: &[&str] = &["browser_current_tab", "browser_list_tabs", "quit_app"];

/// Whether the chat agent gets the computer-control server at all. Synced
/// from the frontend `computerControlEnabled` setting via the
/// `set_computer_control_enabled` command. Defaults on (mirrors teamwork).
static COMPUTER_CONTROL_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_computer_control_enabled(v: bool) {
    COMPUTER_CONTROL_ENABLED.store(v, Ordering::Relaxed);
    if !v {
        // Off ends every see-and-act session now, not at the next call.
        crate::computer_broker::end_all_sessions();
    }
}
pub fn computer_control_enabled() -> bool {
    COMPUTER_CONTROL_ENABLED.load(Ordering::Relaxed)
}

/// Bash permission-rule patterns the chat spawn disallows while this server
/// is attached, so raw `osascript` / `open` can't bypass the vetted,
/// confirm-gated verbs. Prefix-matching rules — friction against a prompt-
/// injected detour, not a sandbox.
pub const BASH_DENY: &[&str] = &["Bash(osascript:*)", "Bash(open:*)"];

/// Set on the server of a `bypassPermissions` run: its tier-2 verbs run
/// without the Allow/Deny card (user decision, 2026-10-02: bypass asks
/// nothing). The GUI writes it into the spawn's environment; the agent
/// can't change a server's environment once it runs.
pub const ENV_BYPASS: &str = "HOLLOW_COMPUTER_BYPASS";

/// The see-and-act link (Cua CU1): the computer broker's socket and this
/// run's identity and proof, written by the GUI into a chat server's
/// environment when the broker listens (never in `plan`). Without them the
/// server offers its seven verbs only.
pub const ENV_BROKER_SOCK: &str = "HOLLOW_COMPUTER_SOCK";
/// The run id rides the key every Hollow server uses (`HOLLOW_RUN_ID`), so
/// `run_auth` registers it and the broker's peer check finds the run.
const ENV_BROKER_RUN: &str = crate::usage_ledger::ENV_RUN_ID;
const ENV_BROKER_SCOPE: &str = "HOLLOW_COMPUTER_SCOPE";
const ENV_BROKER_PERM: &str = "HOLLOW_COMPUTER_PERM";
const ENV_BROKER_PROOF: &str = "HOLLOW_COMPUTER_PROOF";
/// A call's answer waits at most this long (a first call may start the
/// Driver: a 20 s handshake plus a 90 s call).
const BROKER_TIMEOUT: Duration = Duration::from_secs(125);
/// A tool list waits at most this long: agent CLIs drop a server whose
/// list is slow, and the seven verbs with it.
const BROKER_LIST_TIMEOUT: Duration = Duration::from_secs(5);
/// A broker answer is read up to this many bytes (results are capped there).
const BROKER_ANSWER_MAX: u64 = 16 << 20;

/// The environment that carries a run's see-and-act link: the broker's
/// socket, the run, scope, mode and the proof for exactly those. Empty when
/// the broker isn't listening, the mode is `plan` or there is no run id. The
/// MCP server's entry and `hollow computer` ([`crate::computer_cli`]) read the
/// same variables.
pub(crate) fn broker_env(perm: &str, run_id: &str, scope: &str) -> Vec<(&'static str, String)> {
    let Some(sock) = crate::computer_broker::socket_path() else {
        return Vec::new();
    };
    if perm == "plan" || run_id.trim().is_empty() {
        return Vec::new();
    }
    vec![
        (ENV_BROKER_SOCK, sock.to_string_lossy().to_string()),
        (ENV_BROKER_RUN, run_id.trim().to_string()),
        (ENV_BROKER_SCOPE, scope.trim().to_string()),
        (ENV_BROKER_PERM, perm.to_string()),
        (
            ENV_BROKER_PROOF,
            crate::computer_broker::proof_for(run_id, scope, perm),
        ),
    ]
}

/// The `hollow_computer` entry for an `--mcp-config` mcpServers map, or
/// `None` when the user has computer control switched off. The server is
/// this same binary re-invoked as `computer-mcp`; `HOLLOW_VAULT` carries the
/// chat cwd so the audit log lands next to where the agent is working.
/// `perm` is the run's canonical permission mode ([`ENV_BYPASS`]).
///
/// `run_id` and `scope` name the run for the see-and-act link: when the
/// computer broker listens and the mode isn't `plan`, the entry carries its
/// socket and a proof for exactly this run, scope and mode.
pub fn server_entry(
    vault: &Path,
    perm: &str,
    run_id: &str,
    scope: &str,
) -> Option<(&'static str, Value)> {
    if !computer_control_enabled() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut env = crate::approvals::mcp_env(vault);
    if perm == "bypassPermissions" {
        env[ENV_BYPASS] = json!("1");
    }
    for (key, value) in broker_env(perm, run_id, scope) {
        env[key] = json!(value);
    }
    Some((
        "hollow_computer",
        json!({
            "command": exe.to_string_lossy(),
            "args": ["computer-mcp"],
            "env": env
        }),
    ))
}

/// Entry point for the `computer-mcp` subcommand. Blocks on a current-thread
/// runtime serving JSON-RPC over stdio until stdin reaches EOF.
pub fn serve() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime");
    rt.block_on(serve_async());
}

async fn serve_async() {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();
    while let Ok(Some(raw)) = lines.next_line().await {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // ignore non-JSON noise
        };
        if let Some(resp) = handle(&req).await {
            let mut s = resp.to_string();
            s.push('\n');
            if stdout.write_all(s.as_bytes()).await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
        }
    }
}

/// Dispatch one JSON-RPC message. Returns `Some(response)` for requests
/// (messages with an `id`) and `None` for notifications.
async fn handle(req: &Value) -> Option<Value> {
    let ctx = Context::from_env();
    let link = BrokerLink::from_env();
    handle_with(req, &ctx, link.as_ref()).await
}

/// The see-and-act link a chat server carries ([`ENV_BROKER_SOCK`]).
pub(crate) struct BrokerLink {
    pub sock: PathBuf,
    pub run_id: String,
    pub scope: String,
    pub perm: String,
    pub proof: String,
}

impl BrokerLink {
    pub(crate) fn from_env() -> Option<Self> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Some(Self {
            sock: PathBuf::from(get(ENV_BROKER_SOCK)?),
            run_id: get(ENV_BROKER_RUN)?,
            scope: std::env::var(ENV_BROKER_SCOPE).unwrap_or_default(),
            perm: get(ENV_BROKER_PERM)?,
            proof: get(ENV_BROKER_PROOF)?,
        })
    }

    /// One request to the broker, one answer back.
    pub(crate) async fn ask(
        &self,
        op: &str,
        tool: &str,
        args: &Value,
        within: Duration,
    ) -> Result<Value, String> {
        self.ask_approved(op, tool, args, &[], within).await
    }

    /// [`Self::ask`] with the cards the person has just allowed for this call.
    pub(crate) async fn ask_approved(
        &self,
        op: &str,
        tool: &str,
        args: &Value,
        approved: &[String],
        within: Duration,
    ) -> Result<Value, String> {
        let mut line = json!({
            "op": op,
            "run_id": self.run_id,
            "scope": self.scope,
            "perm": self.perm,
            "proof": self.proof,
            "tool": tool,
            "approved": approved,
            "arguments": args,
        })
        .to_string();
        line.push('\n');
        let exchange = async {
            use tokio::io::AsyncReadExt;
            let stream = tokio::net::UnixStream::connect(&self.sock)
                .await
                .map_err(|e| {
                    format!("Hollow's computer broker isn't reachable ({e}); nothing ran.")
                })?;
            let (read, mut write) = stream.into_split();
            write
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("Hollow's computer broker hung up ({e}); nothing ran."))?;
            let mut buf = Vec::new();
            BufReader::new(read)
                .take(BROKER_ANSWER_MAX)
                .read_until(b'\n', &mut buf)
                .await
                .map_err(|e| format!("Hollow's computer broker hung up ({e})."))?;
            serde_json::from_slice::<Value>(&buf)
                .map_err(|_| "Hollow's computer broker gave no answer.".to_string())
        };
        tokio::time::timeout(within, exchange)
            .await
            .map_err(|_| "Hollow's computer broker didn't answer in time.".to_string())?
    }
}

/// Immutable request state. HTTP callers never borrow the host's environment.
pub(crate) struct Context {
    pub bypass: bool,
    pub audit_root: Option<PathBuf>,
    pub approval: Option<crate::shared_mcp::Approval>,
}

impl Context {
    /// The shell path's context ([`crate::computer_cli`]): never bypass. An
    /// agent controls its own shell's environment, so a bypass flag read from
    /// it would let it skip the tier-2 card; only Hollow writes that flag into
    /// the MCP server's environment. Tier 2 always asks here.
    pub(crate) fn for_cli() -> Self {
        Self {
            bypass: false,
            ..Self::from_env()
        }
    }

    pub(crate) fn from_env() -> Self {
        Self {
            bypass: std::env::var(ENV_BYPASS).is_ok_and(|v| v == "1"),
            audit_root: std::env::var("HOLLOW_VAULT")
                .ok()
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            approval: None,
        }
    }
}

/// Serve one message with no see-and-act link (the shared HTTP host).
pub(crate) async fn handle_in(req: &Value, ctx: &Context) -> Option<Value> {
    handle_with(req, ctx, None).await
}

/// Serve one message. With `link`, the broker's classified tools are listed
/// after the seven verbs and any other name is forwarded to it.
pub(crate) async fn handle_with(
    req: &Value,
    ctx: &Context,
    link: Option<&BrokerLink>,
) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    match method {
        "initialize" => {
            let pv = req
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_PROTOCOL);
            Some(result(
                id,
                json!({
                    "protocolVersion": pv,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "hollow-computer", "version": env!("CARGO_PKG_VERSION") },
                }),
            ))
        }
        "tools/list" => {
            let mut tools = tool_specs(ctx.bypass);
            if let Some(link) = link {
                tools.extend(broker_tools(link).await);
            }
            Some(result(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match link {
                Some(link) if !crate::computer_broker::NATIVE_VERBS.contains(&name) => {
                    Some(forward_call(id, name, &args, link, ctx).await)
                }
                _ => Some(call_tool_in(id, name, &args, ctx).await),
            }
        }
        "ping" => Some(result(id, json!({}))),
        _ => id.map(|id| error(Some(id), -32601, &format!("method not found: {method}"))),
    }
}

/// Tool schemas. Descriptions are load-bearing — they teach the agent the
/// tier model (Tier 1 runs immediately; Tier 2 asks the user, who may
/// refuse) so it can set expectations in conversation. `bypass` is this
/// server's [`ENV_BYPASS`]: there tier 2 runs at once, and the descriptions
/// say so (`mode_contract::computer_tier2`, F6).
fn tool_specs(bypass: bool) -> Vec<Value> {
    let tier2 = crate::mode_contract::computer_tier2(bypass);
    let browser_prop = json!({
        "type": "string",
        "description": "Which browser: safari, chrome, arc, edge, brave or helium, or the app name of any other running Chromium browser. Omit to use the browser currently running. Firefox and LibreWolf tabs can't be read (no AppleScript tab support)."
    });
    vec![
        json!({
            "name": "open_url",
            "description": "Open a link on the user's screen — in their default browser, or a specific one \
        via `browser`. Only http/https URLs are allowed. Tier-1 action: runs immediately, no confirmation dialog. \
        IMPORTANT: when the user names a site or destination instead of giving an explicit URL, call \
        search_bookmarks first and open the bookmarked URL — only guess a URL when no bookmark matches. \
        Always use this instead of shell `open` for URLs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The http(s) URL to open." },
                    "browser": browser_prop
                },
                "required": ["url"]
            }
        }),
        json!({
            "name": "search_bookmarks",
            "description": "Search the user's browser bookmarks on this Mac (Chrome, Edge, Brave and Helium \
        profiles, any other running Chromium browser, plus Safari when readable; not Firefox or LibreWolf) by keyword, matching bookmark names and URLs. ALWAYS call this \
        before open_url whenever the user asks to open or navigate to a site by name (e.g. 'open HRIS') — if a \
        bookmark matches, open its exact URL instead of guessing one. Read-only, Tier-1, no confirmation dialog.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Keyword(s) to match against bookmark names and URLs (case-insensitive; multiple words must all appear)." }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "open_app",
            "description": "Launch a desktop application on the user's Mac by name, e.g. 'Notion', 'Slack', \
        'Visual Studio Code'. The name is resolved against the apps actually installed; if it's unknown or ambiguous \
        the error lists candidates — call list_apps to browse. Tier-1 action: runs immediately, no confirmation dialog. \
        Always use this instead of shell `open -a`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Application name (case-insensitive; partial names resolve when unambiguous)." }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "list_apps",
            "description": "List the desktop applications installed on the user's Mac — the valid targets for \
        open_app and quit_app. Read-only, Tier-1.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "browser_current_tab",
            "description": format!("Read the title and URL of the active tab in the user's browser. {tier2} \
        First use may also trigger a one-time macOS Automation permission prompt for the target browser."),
            "inputSchema": {
                "type": "object",
                "properties": { "browser": browser_prop }
            }
        }),
        json!({
            "name": "browser_list_tabs",
            "description": format!("List the title and URL of every tab in the front window of the user's browser \
        (capped at 40). {tier2}"),
            "inputSchema": {
                "type": "object",
                "properties": { "browser": browser_prop }
            }
        }),
        json!({
            "name": "quit_app",
            "description": format!("Gracefully quit a running application by name (resolved against installed apps; \
        the app may still prompt to save unsaved work). {tier2}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Application name (case-insensitive; partial names resolve when unambiguous)." }
                },
                "required": ["name"]
            }
        }),
    ]
}

async fn call_tool_in(id: Option<Value>, name: &str, args: &Value, ctx: &Context) -> Value {
    if let Some(approval) = &ctx.approval {
        if !approval.valid() {
            return tool_error(id, "This attempt has ended; nothing ran.");
        }
    }
    let outcome = match name {
        "open_url" => open_url(args).await,
        "search_bookmarks" => search_bookmarks_text(args),
        "open_app" => open_app(args).await,
        "list_apps" => list_apps_text(),
        "browser_current_tab" => browser_current_tab(args, ctx).await,
        "browser_list_tabs" => browser_list_tabs(args, ctx).await,
        "quit_app" => quit_app(args, ctx).await,
        other => Err(format!("unknown tool: {other}")),
    };
    audit(name, args, &outcome, ctx.audit_root.as_deref());
    match outcome {
        Ok(text) => tool_text(id, &text),
        Err(e) => tool_error(id, &e),
    }
}

/// The broker's tools for this run: none when it refuses (not set up,
/// switched off) or doesn't answer, and never one named like a verb here.
async fn broker_tools(link: &BrokerLink) -> Vec<Value> {
    let Ok(answer) = link
        .ask("tools", "", &Value::Null, BROKER_LIST_TIMEOUT)
        .await
    else {
        return Vec::new();
    };
    if answer.get("ok").and_then(Value::as_bool) != Some(true) {
        return Vec::new();
    }
    answer
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter(|t| {
                    t.get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|n| !crate::computer_broker::NATIVE_VERBS.contains(&n))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Forward a see-and-act call to the broker. Its result passes through as
/// the broker capped it. The vault log gets the tool, a digest of the
/// arguments (typed text never) and whether it ran, never the result.
async fn forward_call(
    id: Option<Value>,
    name: &str,
    args: &Value,
    link: &BrokerLink,
    ctx: &Context,
) -> Value {
    let answer = ask_with_cards(link, name, args, ctx).await;
    let outcome = match &answer {
        Ok(a) if a.get("ok").and_then(Value::as_bool) == Some(true) => Ok(format!(
            "ran ({} tier)",
            a.get("tier").and_then(Value::as_str).unwrap_or("?")
        )),
        Ok(a) => Err(a
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("refused; nothing ran.")
            .to_string()),
        Err(e) => Err(e.clone()),
    };
    // The log line never carries the broker's words: a Driver error can
    // echo what was typed.
    let logged = match &outcome {
        Ok(text) => Ok(text.clone()),
        Err(_) => Err("refused or failed".to_string()),
    };
    audit(name, &args_shape(args), &logged, ctx.audit_root.as_deref());
    match (answer, outcome) {
        (Ok(mut a), Ok(_)) => {
            let result = a.get_mut("result").map(Value::take).unwrap_or(Value::Null);
            let content = result.get("content").cloned().unwrap_or_else(|| json!([]));
            let is_error = result
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": { "content": content, "isError": is_error } })
        }
        (_, Err(e)) => tool_error(id, &e),
        (Err(e), _) => tool_error(id, &e),
    }
}

/// Most cards one call may raise (sandbox use or first use of this Mac, an app
/// or two, tier 2, outward, secure field).
const MAX_CARDS_PER_CALL: usize = 8;

/// Send the call; while the broker says a card is needed, show it and send
/// the call again with the answers. A denied card ends the call (nothing
/// ran). Cards for outward acts and secure fields ask in bypass too: the
/// broker only raises them when the mode says so (CU2), and bypass is no
/// answer to them.
async fn ask_with_cards(
    link: &BrokerLink,
    name: &str,
    args: &Value,
    ctx: &Context,
) -> Result<Value, String> {
    let mut approved: Vec<String> = Vec::new();
    for _ in 0..=MAX_CARDS_PER_CALL {
        let answer = link
            .ask_approved("call", name, args, &approved, BROKER_TIMEOUT)
            .await?;
        let Some(card) = answer.get("needsCard") else {
            return Ok(answer);
        };
        let kind = card.get("kind").and_then(Value::as_str).unwrap_or("");
        let text = card.get("text").and_then(Value::as_str).unwrap_or("");
        if kind.is_empty() || text.is_empty() || approved.iter().any(|k| k == kind) {
            return Err("The broker's approval request was malformed; nothing ran.".into());
        }
        ask_person(ctx, name, args, text).await?;
        approved.push(kind.to_string());
    }
    Err("Too many approvals asked for one call; nothing ran.".into())
}

/// A card the person must answer whatever the mode ([`confirm`] skips it in
/// bypass; the always-ask cards must not).
async fn ask_person(ctx: &Context, tool: &str, args: &Value, text: &str) -> Result<(), String> {
    if let Some(approval) = &ctx.approval {
        if !approval.valid() {
            return Err("This attempt has ended; nothing ran.".into());
        }
        return approval
            .confirm("Computer control", text, "computer", tool, args)
            .await;
    }
    crate::approvals::confirm_tool("Computer control", text, "computer", Some(tool), Some(args))
        .await
}

/// What the vault log keeps of a forwarded call's arguments: their keys
/// and size, never values (a hash of a short typed PIN could be reversed).
fn args_shape(args: &Value) -> Value {
    let keys: Vec<&str> = args
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    json!({ "keys": keys, "bytes": args.to_string().len() })
}

// ── Tier 1: open / list ─────────────────────────────────────────────────────

async fn open_url(args: &Value) -> Result<String, String> {
    let url = validate_url(arg_str(args, "url").unwrap_or(""))?;
    // BG1: the person's own host lists (host-scope.json), when they wrote
    // any: deny wins, an allow list denies the rest. No file, no limit.
    if let crate::host_scope::Decision::Denied(why) =
        crate::host_scope::Scope::user().decide_url(&url)
    {
        return Err(format!("Not opened: {why}."));
    }
    if cfg!(target_os = "macos") {
        let mut cmd = tokio::process::Command::new("/usr/bin/open");
        let in_browser = match arg_str(args, "browser") {
            Some(spec) => {
                let b = resolve_browser(Some(spec))?;
                match b.bundle_id.as_deref() {
                    Some(id) => cmd.arg("-b").arg(id),
                    None => cmd.arg("-a").arg(b.app.as_ref()),
                };
                format!(" in {}", b.label())
            }
            None => String::new(),
        };
        cmd.arg(&url);
        run_quiet(cmd).await?;
        Ok(format!("Opened {url}{in_browser}."))
    } else {
        let mut cmd = tokio::process::Command::new("xdg-open");
        cmd.arg(&url);
        run_quiet(cmd).await?;
        Ok(format!("Opened {url}."))
    }
}

async fn open_app(args: &Value) -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("open_app is macOS-only".into());
    }
    let name = arg_str(args, "name").unwrap_or("");
    let apps = installed_apps();
    let app = resolve_app(&apps, name)?;
    let mut cmd = tokio::process::Command::new("/usr/bin/open");
    cmd.arg(&app.path);
    run_quiet(cmd).await?;
    Ok(format!("Launched {}.", app.name))
}

fn list_apps_text() -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("list_apps is macOS-only".into());
    }
    let apps = installed_apps();
    if apps.is_empty() {
        return Err("no installed applications found".into());
    }
    let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
    Ok(format!(
        "{} installed apps:\n{}",
        names.len(),
        names.join("\n")
    ))
}

// ── Tier 1: bookmark search ─────────────────────────────────────────────────

/// Cap on hits returned to the model.
const MAX_BOOKMARK_HITS: usize = 15;
/// Largest Chromium `Bookmarks` file read (a heavy user's is a few MB).
const BOOKMARKS_FILE_MAX: u64 = 64 << 20;

#[derive(Clone, Debug, PartialEq)]
struct Bookmark {
    title: String,
    url: String,
    source: Cow<'static, str>,
}

fn search_bookmarks_text(args: &Value) -> Result<String, String> {
    let query = arg_str(args, "query").ok_or("missing required argument `query`")?;
    let (books, notes) = collect_bookmarks();
    Ok(format_bookmark_results(&books, &notes, query))
}

fn format_bookmark_results(books: &[Bookmark], notes: &[String], query: &str) -> String {
    let hits = search_bookmarks_in(books, query);
    if hits.is_empty() {
        let mut out = format!(
            "No bookmarks match `{query}` (searched {} bookmarks). Fall back to opening the URL you think is right with open_url.",
            books.len()
        );
        // Unreadable-source notes only matter on a miss — the bookmark might
        // live in the store we couldn't read. On a hit they're just noise.
        for n in notes {
            out.push_str("\nNote: ");
            out.push_str(n);
        }
        out
    } else {
        let lines: Vec<String> = hits
            .iter()
            .take(MAX_BOOKMARK_HITS)
            .map(|b| format!("{}\t{}\t({})", b.title, b.url, b.source))
            .collect();
        let more = if hits.len() > MAX_BOOKMARK_HITS {
            format!(
                "\n(+{} more matches not shown — narrow the query)",
                hits.len() - MAX_BOOKMARK_HITS
            )
        } else {
            String::new()
        };
        format!(
            "{} bookmark(s) match `{query}` (name<TAB>url<TAB>browser):\n{}{}",
            hits.len(),
            lines.join("\n"),
            more
        )
    }
}

/// Rank matches: exact title, then the full phrase in the title, then all
/// query tokens in the title, then all tokens across title+URL. Shorter
/// titles first within a rank (a tighter match is usually the intended one).
fn search_bookmarks_in<'a>(books: &'a [Bookmark], query: &str) -> Vec<&'a Bookmark> {
    let ql = query.trim().to_lowercase();
    let tokens: Vec<&str> = ql.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(&Bookmark, u8)> = books
        .iter()
        .filter_map(|b| {
            let t = b.title.to_lowercase();
            let u = b.url.to_lowercase();
            let rank = if t == ql {
                0
            } else if t.contains(&ql) {
                1
            } else if tokens.iter().all(|tok| t.contains(tok)) {
                2
            } else if tokens.iter().all(|tok| t.contains(tok) || u.contains(tok)) {
                3
            } else {
                return None;
            };
            Some((b, rank))
        })
        .collect();
    hits.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.title.len().cmp(&b.0.title.len())));
    hits.into_iter().map(|(b, _)| b).collect()
}

/// Read every bookmark store we know how to parse. Returns the bookmarks
/// plus human-readable notes about sources that exist but couldn't be read
/// (Safari's file is TCC-protected without Full Disk Access).
fn collect_bookmarks() -> (Vec<Bookmark>, Vec<String>) {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    // Chromium family: one JSON file per profile dir. The browser key pairs
    // each store with its BROWSERS entry so the user's system-default browser
    // can be scanned first — on cross-browser duplicate URLs, the dedupe
    // below keeps the default browser's title.
    let default_key = default_browser().map(|b| b.key.as_ref());
    let mut known = CHROMIUM_STORES.to_vec();
    known.sort_by_key(|&(_, _, key)| if Some(key) == default_key { 0 } else { 1 });
    let mut order: Vec<(String, Cow<'static, str>)> = known
        .into_iter()
        .map(|(rel, source, _)| (rel.to_string(), Cow::Borrowed(source)))
        .collect();
    // Any other running Chromium browser with the Chrome dictionary: its
    // profile folder is usually named for its bundle id or its app name.
    let Some(support) = dirs::home_dir().map(|h| h.join("Library/Application Support")) else {
        return (out, notes);
    };
    for b in chrome_dictionary_browsers_in(&running_apps()) {
        for rel in chromium_store_candidates(b.bundle_id.as_deref(), &b.app) {
            if is_chromium_user_data(&support.join(&rel)) {
                order.push((rel, b.app.clone()));
            }
        }
    }
    let mut scanned = std::collections::HashSet::new();
    for (rel, source) in order {
        if !scanned.insert(rel.clone()) {
            continue;
        }
        for file in chromium_profile_files_in(&support.join(&rel)) {
            chromium_bookmarks(&file, source.clone(), &mut out);
        }
    }
    safari_bookmarks(&mut out, &mut notes);
    // Dedupe by URL across browsers/profiles — first sighting wins.
    let mut seen = std::collections::HashSet::new();
    out.retain(|b| seen.insert(b.url.clone()));
    (out, notes)
}

/// Known Chromium bookmark stores: folder under `~/Library/Application
/// Support`, the source label shown in results, and the [`BROWSERS`] key.
const CHROMIUM_STORES: &[(&str, &str, &str)] = &[
    ("Google/Chrome", "Chrome", "chrome"),
    ("Microsoft Edge", "Edge", "edge"),
    ("BraveSoftware/Brave-Browser", "Brave", "brave"),
    // Helium keeps its profiles under its bundle id (checked on 0.21.3.1).
    ("net.imput.helium", "Helium", "helium"),
];

/// Where a Chromium browser found at runtime likely keeps its profiles:
/// a folder named for its bundle id, then one named for the app. Only plain
/// single-segment names are offered, so nothing outside Application Support
/// can be named.
fn chromium_store_candidates(bundle_id: Option<&str>, app: &str) -> Vec<String> {
    let plain =
        |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains('/') && !s.contains('\\');
    let mut out = Vec::new();
    for c in bundle_id.into_iter().chain(std::iter::once(app.trim())) {
        if plain(c) && !out.iter().any(|o: &String| o == c) {
            out.push(c.to_string());
        }
    }
    out
}

/// A Chromium user-data folder has a `Local State` file at its top; a
/// candidate folder without one is not scanned.
fn is_chromium_user_data(dir: &Path) -> bool {
    std::fs::symlink_metadata(dir.join("Local State")).is_ok_and(|m| m.file_type().is_file())
}

/// Profile dirs (`Default`, `Profile 1`, …) under a Chromium browser's
/// user-data folder that contain a `Bookmarks` JSON file.
fn chromium_profile_files_in(base: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(base) {
        for e in rd.flatten() {
            let p = e.path().join("Bookmarks");
            if p.is_file() {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

fn chromium_bookmarks(file: &Path, source: Cow<'static, str>, out: &mut Vec<Bookmark>) {
    // Regular files only, capped: a runtime browser's folder isn't trusted.
    let Some(data) = read_small_file(file, BOOKMARKS_FILE_MAX) else {
        return;
    };
    let Ok(v) = serde_json::from_slice::<Value>(&data) else {
        return;
    };
    let Some(roots) = v.get("roots").and_then(|r| r.as_object()) else {
        return;
    };
    for (key, node) in roots {
        if key == "trash" {
            continue; // Brave keeps deleted bookmarks here
        }
        walk_chromium(node, source.clone(), out);
    }
}

fn walk_chromium(node: &Value, source: Cow<'static, str>, out: &mut Vec<Bookmark>) {
    if node.get("type").and_then(|t| t.as_str()) == Some("url") {
        if let Some(url) = node.get("url").and_then(|u| u.as_str()) {
            let title = node.get("name").and_then(|n| n.as_str()).unwrap_or(url);
            out.push(Bookmark {
                title: title.to_string(),
                url: url.to_string(),
                source,
            });
        }
        return;
    }
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for c in children {
            walk_chromium(c, source.clone(), out);
        }
    }
}

/// Safari's bookmarks live in a binary plist that macOS TCC-protects: without
/// Full Disk Access the read fails with PermissionDenied, which we surface as
/// a note instead of an error so the Chromium results still flow.
fn safari_bookmarks(out: &mut Vec<Bookmark>, notes: &mut Vec<String>) {
    let Some(home) = dirs::home_dir() else { return };
    let path = home.join("Library/Safari/Bookmarks.plist");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            notes.push(
                "Safari bookmarks couldn't be read (macOS protects ~/Library/Safari — grant Hollow Full Disk Access to include them).".to_string(),
            );
            return;
        }
    };
    let Ok(val) = plist::Value::from_reader(std::io::Cursor::new(bytes)) else {
        notes.push("Safari bookmarks file exists but couldn't be parsed.".to_string());
        return;
    };
    walk_safari(&val, out);
}

fn walk_safari(v: &plist::Value, out: &mut Vec<Bookmark>) {
    let plist::Value::Dictionary(d) = v else {
        return;
    };
    if d.get("WebBookmarkType").and_then(|t| t.as_string()) == Some("WebBookmarkTypeLeaf") {
        if let Some(url) = d.get("URLString").and_then(|u| u.as_string()) {
            let title = d
                .get("URIDictionary")
                .and_then(|ud| ud.as_dictionary())
                .and_then(|ud| ud.get("title"))
                .and_then(|t| t.as_string())
                .unwrap_or(url);
            out.push(Bookmark {
                title: title.to_string(),
                url: url.to_string(),
                source: Cow::Borrowed("Safari"),
            });
        }
        return;
    }
    if let Some(plist::Value::Array(children)) = d.get("Children") {
        for c in children {
            walk_safari(c, out);
        }
    }
}

// ── Tier 2: AppleScript behind a native confirm ─────────────────────────────

async fn browser_current_tab(args: &Value, ctx: &Context) -> Result<String, String> {
    let b = resolve_browser(arg_str(args, "browser"))?;
    confirm(
        ctx,
        "browser_current_tab",
        args,
        &format!("Claude wants to read the active tab in {}.", b.label()),
    )
    .await?;
    let target = b.tell_target();
    let script = match b.dialect {
        Dialect::Safari => format!(
            "tell {0} to return (URL of current tab of front window) & \"\\n\" & (name of current tab of front window)",
            target
        ),
        Dialect::Chromium => format!(
            "tell {0} to return (URL of active tab of front window) & \"\\n\" & (title of active tab of front window)",
            target
        ),
    };
    let out = osascript(&script, SCRIPT_TIMEOUT)
        .await
        .map_err(|e| tier2_err(&b.label(), &e))?;
    let mut lines = out.lines();
    let url = lines.next().unwrap_or("").trim().to_string();
    let title = lines.next().unwrap_or("").trim().to_string();
    Ok(format!(
        "Active tab in {}:\nURL: {url}\nTitle: {title}",
        b.label()
    ))
}

async fn browser_list_tabs(args: &Value, ctx: &Context) -> Result<String, String> {
    let b = resolve_browser(arg_str(args, "browser"))?;
    confirm(
        ctx,
        "browser_list_tabs",
        args,
        &format!("Claude wants to list the open tabs in {}.", b.label()),
    )
    .await?;
    // One field name differs between dialects; the loop shape is shared.
    let title_of = match b.dialect {
        Dialect::Safari => "name",
        Dialect::Chromium => "title",
    };
    let script = format!(
        "set out to \"\"\ntell {0}\nrepeat with t in tabs of front window\nset out to out & (URL of t) & \"\\t\" & ({1} of t) & \"\\n\"\nend repeat\nend tell\nreturn out",
        b.tell_target(),
        title_of
    );
    let out = osascript(&script, SCRIPT_TIMEOUT)
        .await
        .map_err(|e| tier2_err(&b.label(), &e))?;
    let tabs: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    let shown = tabs
        .iter()
        .take(MAX_TABS)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let more = if tabs.len() > MAX_TABS {
        format!("\n(+{} more not shown)", tabs.len() - MAX_TABS)
    } else {
        String::new()
    };
    Ok(format!(
        "{} tabs in the front window of {} (URL<TAB>title):\n{shown}{more}",
        tabs.len(),
        b.label()
    ))
}

async fn quit_app(args: &Value, ctx: &Context) -> Result<String, String> {
    let name = arg_str(args, "name").unwrap_or("");
    let apps = installed_apps();
    let app = resolve_app(&apps, name)?.clone();
    if app.name.eq_ignore_ascii_case("hollow") {
        return Err("refusing to quit Hollow itself — that would kill this conversation".into());
    }
    confirm(
        ctx,
        "quit_app",
        args,
        &format!("Claude wants to quit {}.", app.name),
    )
    .await?;
    let script = format!(
        "tell application \"{}\" to quit",
        applescript_escape(&app.name)
    );
    osascript(&script, SCRIPT_TIMEOUT)
        .await
        .map_err(|e| tier2_err(&app.name, &e))?;
    Ok(format!("Asked {} to quit.", app.name))
}

/// Gate for a Tier-2 action: an inline Allow/Deny card in the chat when the
/// app's approval broker is reachable (see [`crate::approvals`]), else the
/// native dialog. Deny / timeout / cancelled turn all count as "not approved".
/// `tool` (a [`CONFIRM_TOOLS`] name) and `args` (the `tools/call` arguments
/// exactly as received) let the broker bind the request to the agent's tool
/// call, so the chat badges that row (Paperclip PR 21).
async fn confirm(ctx: &Context, tool: &str, args: &Value, action: &str) -> Result<(), String> {
    if let Some(approval) = &ctx.approval {
        if !approval.valid() {
            return Err("This attempt has ended; nothing ran.".into());
        }
        if ctx.bypass {
            return Ok(());
        }
        return approval
            .confirm("Computer control", action, "computer", tool, args)
            .await;
    }
    if ctx.bypass {
        return Ok(());
    }
    crate::approvals::confirm_tool(
        "Computer control",
        action,
        "computer",
        Some(tool),
        Some(args),
    )
    .await
}

/// The native-dialog confirm primitive — the FALLBACK path for
/// [`crate::approvals::confirm_tool`] when the inline-chat broker is unreachable
/// (also covering the M365 write tools). `display dialog` runs in the
/// osascript process itself — no Apple Events to another app, so no TCC
/// prompt for the confirmation itself. Deny (the cancel button) makes
/// osascript exit non-zero with error -128; a `giving up after` timeout
/// returns success with `gave up:true`. Both count as "not approved".
pub(crate) async fn native_confirm(title: &str, action: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("the native confirmation dialog is macOS-only".into());
    }
    let script = format!(
        "display dialog \"{}\" with title \"{}\" buttons {{\"Deny\", \"Allow\"}} \
default button \"Allow\" cancel button \"Deny\" with icon caution giving up after {CONFIRM_TIMEOUT_SECS}",
        applescript_escape(action),
        applescript_escape(title),
    );
    match osascript(&script, Duration::from_secs(CONFIRM_TIMEOUT_SECS + 15)).await {
        Ok(out) if dialog_allowed(&out) => Ok(()),
        Ok(_) => Err("the user did not approve (the confirmation dialog timed out)".into()),
        Err(e) if e.contains("-128") => Err("the user denied this action".into()),
        Err(e) => Err(format!("confirmation dialog failed: {e}")),
    }
}

/// `display dialog` outcome → approved? Timeout reports `gave up:true` with
/// an empty button, so require the Allow button AND no give-up.
fn dialog_allowed(stdout: &str) -> bool {
    stdout.contains("button returned:Allow") && !stdout.contains("gave up:true")
}

fn tier2_err(app: &str, e: &str) -> String {
    format!(
        "AppleScript against {app} failed: {e}. Likely causes: {app} isn't running or has no window open, \
or macOS hasn't granted Hollow Automation permission for {app} \
(System Settings → Privacy & Security → Automation → Hollow)."
    )
}

// ── Shared plumbing ─────────────────────────────────────────────────────────

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Validate a model-supplied URL: http/https only, no whitespace or control
/// characters, non-empty host. This is the whole defense against `file://`,
/// `javascript:` and friends — keep it strict.
fn validate_url(raw: &str) -> Result<String, String> {
    let url = raw.trim();
    if url.is_empty() {
        return Err("missing required argument `url`".into());
    }
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(format!(
            "only http:// and https:// URLs are allowed (got `{url}`)"
        ));
    }
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("URL contains whitespace or control characters".into());
    }
    let after_scheme = &url[url.find("://").map(|i| i + 3).unwrap_or(0)..];
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() {
        return Err("URL has no host".into());
    }
    Ok(url.to_string())
}

/// Escape a string for embedding inside an AppleScript double-quoted literal.
/// Newlines/tabs become their escapes; other control characters are dropped.
fn applescript_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Runs inside `hollow computer-mcp`, so it never creates a process group:
/// the child keeps this server's group and inherited run tag, which the
/// GUI's stop and finish sweep reaches.
async fn osascript(script: &str, timeout: Duration) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("/usr/bin/osascript");
    cmd.arg("-e").arg(script);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| "osascript timed out".to_string())?
        .map_err(|e| format!("spawn osascript: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("osascript exited {}", out.status)
        } else {
            stderr
        })
    }
}

/// Run a launcher command (`open` / `xdg-open`), discarding output on
/// success and surfacing stderr on failure. Like [`osascript`], this runs
/// inside the MCP subprocess and never creates a process group. Apps that
/// `open` launches are launchd children, deliberately outside any run's
/// cleanup: Hollow never kills an app the user asked it to open.
async fn run_quiet(mut cmd: tokio::process::Command) -> Result<(), String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| "launcher timed out".to_string())?
        .map_err(|e| format!("spawn launcher: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("launcher exited {}", out.status)
        } else {
            stderr
        })
    }
}

// ── Browser + app resolution ────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dialect {
    Safari,
    Chromium,
}

/// A browser the tab tools can drive. The known ones are constants in
/// [`BROWSERS`]; any other running Chromium browser that speaks Google
/// Chrome's AppleScript dictionary is found at call time
/// ([`chrome_dictionary_browser`]) and gets owned strings (PER-35).
#[derive(Clone, Debug, PartialEq)]
struct Browser {
    key: Cow<'static, str>,
    app: Cow<'static, str>,
    dialect: Dialect,
    /// When set, AppleScript and `open` address the app by bundle id rather
    /// than by name, so a same-named app elsewhere on disk can't be hit; and
    /// whether it runs is checked by bundle id, not by process name.
    bundle_id: Option<Cow<'static, str>>,
    /// Set only for a browser found at runtime: the bundle it was read from.
    /// Its name comes from its own Info.plist, so the confirm and result
    /// text show the bundle id and path too (see [`Browser::label`]).
    bundle_path: Option<PathBuf>,
}

impl Browser {
    const fn known(key: &'static str, app: &'static str, dialect: Dialect) -> Browser {
        Browser {
            key: Cow::Borrowed(key),
            app: Cow::Borrowed(app),
            dialect,
            bundle_id: None,
            bundle_path: None,
        }
    }

    /// The `tell` target of an AppleScript. A bundle id is checked to be
    /// plain reverse-DNS before it is used, so it can't break the quoting.
    fn tell_target(&self) -> String {
        match self.bundle_id.as_deref() {
            Some(id) if valid_bundle_id(id) => format!("application id \"{id}\""),
            _ => format!("application \"{}\"", applescript_escape(&self.app)),
        }
    }

    /// How the confirm card, results and errors name it. A browser found at
    /// runtime names itself, so its bundle id and path go with the name: an
    /// app calling itself "Safari" can't pass for Safari on the card.
    fn label(&self) -> String {
        match (&self.bundle_path, self.bundle_id.as_deref()) {
            (Some(path), Some(id)) => {
                format!("{} (bundle id {id}, {})", self.app, path.display())
            }
            _ => self.app.to_string(),
        }
    }
}

/// Supported browsers. Order matters: when no `browser` is specified, the
/// first one with a running process wins.
const BROWSERS: &[Browser] = &[
    Browser::known("chrome", "Google Chrome", Dialect::Chromium),
    Browser::known("arc", "Arc", Dialect::Chromium),
    Browser::known("safari", "Safari", Dialect::Safari),
    Browser::known("edge", "Microsoft Edge", Dialect::Chromium),
    Browser::known("brave", "Brave Browser", Dialect::Chromium),
    // Helium (imput, Chromium based) speaks the Chrome dictionary. Addressed
    // and checked by bundle id: an older, unrelated floating-window app is
    // also "Helium".
    Browser {
        key: Cow::Borrowed("helium"),
        app: Cow::Borrowed("Helium"),
        dialect: Dialect::Chromium,
        bundle_id: Some(Cow::Borrowed("net.imput.helium")),
        bundle_path: None,
    },
];

/// What the "no browser" and "unsupported browser" errors tell the model.
const UNSUPPORTED_NOTE: &str =
    "or any running Chromium browser with Chrome's AppleScript tab support. \
Firefox and LibreWolf have no AppleScript tab dictionary, so their tabs can't be read";

fn known_keys() -> String {
    BROWSERS
        .iter()
        .map(|b| b.key.as_ref())
        .collect::<Vec<_>>()
        .join(", ")
}

fn resolve_browser(spec: Option<&str>) -> Result<Browser, String> {
    match spec {
        Some(s) => {
            let q = s.trim().to_lowercase();
            if let Some(b) = BROWSERS.iter().find(|b| browser_matches(b, &q)) {
                return Ok(b.clone());
            }
            chrome_dictionary_browsers_in(&running_apps())
                .into_iter()
                .find(|b| browser_matches(b, &q))
                .ok_or_else(|| {
                    format!(
                        "unsupported browser `{s}`: supported are {}, {UNSUPPORTED_NOTE}",
                        known_keys()
                    )
                })
        }
        None => {
            // One `ps` snapshot, taken only when a check needs it (a browser
            // known by bundle id, or the runtime scan).
            let snapshot = std::cell::OnceCell::new();
            let apps = || snapshot.get_or_init(running_apps).as_slice();
            let running = |b: &Browser| browser_running(b, apps, app_running);
            let detect = || chrome_dictionary_browsers_in(apps());
            resolve_default(&default_handler_ids(), &running, &detect)
        }
    }
}

/// Whether `b` runs: by bundle id among the running apps when it has one
/// (so another app with the same process name doesn't count), else by exact
/// process name.
fn browser_running<'a>(
    b: &Browser,
    apps: impl FnOnce() -> &'a [RunningApp],
    by_name: impl FnOnce(&str) -> bool,
) -> bool {
    match b.bundle_id.as_deref() {
        Some(id) => apps().iter().any(|a| a.id.eq_ignore_ascii_case(id)),
        None => by_name(&b.app),
    }
}

/// The no-argument choice: the user's default browser (LaunchServices) when
/// it is a known one and running; then the first running known browser in
/// [`BROWSERS`] order; only then the runtime scan (default first). The scan
/// reads files in every running app bundle, so it runs last and only when
/// nothing known answers.
fn resolve_default(
    defaults: &[String],
    running: &dyn Fn(&Browser) -> bool,
    detect: &dyn Fn() -> Vec<Browser>,
) -> Result<Browser, String> {
    if let Some(b) = defaults.iter().find_map(|id| browser_for_bundle_id(id)) {
        if running(b) {
            return Ok(b.clone());
        }
    }
    if let Some(b) = BROWSERS.iter().find(|b| running(b)) {
        return Ok(b.clone());
    }
    pick_detected(detect(), defaults).ok_or_else(|| {
        format!(
            "no supported browser is running. Pass `browser` ({}) {UNSUPPORTED_NOTE}; or open one with open_app first",
            known_keys()
        )
    })
}

fn browser_matches(b: &Browser, q: &str) -> bool {
    b.key == q
        || b.app.to_lowercase() == q
        || b.bundle_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(q))
}

/// Among browsers found at runtime, the system default one if it is there,
/// else the first.
fn pick_detected(found: Vec<Browser>, default_ids: &[String]) -> Option<Browser> {
    let is_default = |b: &Browser| {
        b.bundle_id
            .as_deref()
            .is_some_and(|id| default_ids.iter().any(|d| d.eq_ignore_ascii_case(id)))
    };
    let i = found.iter().position(is_default).unwrap_or(0);
    found.into_iter().nth(i)
}

/// Map a macOS bundle id to a supported browser, for the system-default
/// lookup and to keep known browsers out of the runtime scan. Exact ids of
/// the apps the [`BROWSERS`] entries address only: Chrome Beta or Brave
/// Nightly is a different app, so it goes through the scan and is told by
/// its own bundle id rather than sent to the stable one.
fn browser_for_bundle_id(id: &str) -> Option<&'static Browser> {
    let key = match id.to_lowercase().as_str() {
        "com.apple.safari" => "safari",
        "com.google.chrome" => "chrome",
        "com.brave.browser" => "brave",
        "com.microsoft.edgemac" => "edge",
        "company.thebrowser.browser" => "arc",
        "net.imput.helium" => "helium",
        _ => return None,
    };
    BROWSERS.iter().find(|b| b.key == key)
}

/// The http/https handler bundle ids LaunchServices records (what "Default
/// web browser" in System Settings writes), in its order. Empty when unset
/// (a fresh Mac defaults to Safari implicitly).
fn default_handler_ids() -> Vec<String> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let path = home
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    plist::Value::from_reader(std::io::Cursor::new(bytes))
        .map(|v| default_handler_ids_from_launchservices(&v))
        .unwrap_or_default()
}

/// The user's system default browser when it is one of [`BROWSERS`].
fn default_browser() -> Option<&'static Browser> {
    default_handler_ids()
        .iter()
        .find_map(|id| browser_for_bundle_id(id))
}

fn default_handler_ids_from_launchservices(v: &plist::Value) -> Vec<String> {
    let Some(handlers) = v
        .as_dictionary()
        .and_then(|d| d.get("LSHandlers"))
        .and_then(|h| h.as_array())
    else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for h in handlers {
        let Some(d) = h.as_dictionary() else { continue };
        let scheme = d
            .get("LSHandlerURLScheme")
            .and_then(|s| s.as_string())
            .unwrap_or("");
        if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https") {
            if let Some(id) = d.get("LSHandlerRoleAll").and_then(|s| s.as_string()) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

#[cfg(test)]
fn default_browser_from_launchservices(v: &plist::Value) -> Option<&'static Browser> {
    default_handler_ids_from_launchservices(v)
        .iter()
        .find_map(|id| browser_for_bundle_id(id))
}

/// Plain reverse-DNS: letters, digits, dots, hyphens and underscores only.
fn valid_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Read a small regular file without following a symlink at its last
/// component and without blocking on a FIFO or device: `None` unless it is
/// a regular file of at most `max` bytes. The bundle files the runtime scan
/// reads come from any running app, so they are not trusted to be sane.
fn read_small_file(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > max {
        return None;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = opts.open(path).ok()?;
    if !file.metadata().ok()?.file_type().is_file() {
        return None;
    }
    let mut buf = Vec::new();
    file.take(max + 1).read_to_end(&mut buf).ok()?;
    (buf.len() as u64 <= max).then_some(buf)
}

/// Info.plist and sdef cap for the runtime scan.
const BUNDLE_FILE_MAX: u64 = 4 << 20;

fn read_info_plist(bundle: &Path) -> Option<plist::Dictionary> {
    let bytes = read_small_file(&bundle.join("Contents/Info.plist"), BUNDLE_FILE_MAX)?;
    plist::Value::from_reader(std::io::Cursor::new(bytes))
        .ok()?
        .into_dictionary()
}

/// A running app: its outer `.app` bundle and its bundle id.
#[derive(Clone, Debug, PartialEq)]
struct RunningApp {
    bundle: PathBuf,
    id: String,
}

/// Every running app with a readable bundle id, from `ps` and each bundle's
/// Info.plist. No Apple Events, so no macOS Automation prompt.
fn running_apps() -> Vec<RunningApp> {
    let Ok(out) = std::process::Command::new("/bin/ps")
        .args(["-axo", "comm="])
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut seen = std::collections::HashSet::new();
    let mut apps = Vec::new();
    for bundle in text.lines().filter_map(app_bundle_of_executable) {
        if !seen.insert(bundle.clone()) {
            continue;
        }
        let id = read_info_plist(&bundle).and_then(|d| {
            d.get("CFBundleIdentifier")
                .and_then(|v| v.as_string())
                .filter(|id| valid_bundle_id(id))
                .map(str::to_string)
        });
        if let Some(id) = id {
            apps.push(RunningApp { bundle, id });
        }
    }
    apps
}

/// The running apps outside [`BROWSERS`] that speak the Chrome AppleScript
/// dictionary. Reads files inside each bundle only, so the Automation prompt
/// only ever names the browser a tool then drives.
fn chrome_dictionary_browsers_in(apps: &[RunningApp]) -> Vec<Browser> {
    apps.iter()
        .filter(|a| browser_for_bundle_id(&a.id).is_none())
        .filter_map(|a| chrome_dictionary_browser(&a.bundle))
        .filter(|b| !collides_with_known(b))
        .collect()
}

/// A runtime browser may not take a known browser's key or name, so `safari`
/// or "Google Chrome" always means the real one.
fn collides_with_known(b: &Browser) -> bool {
    let app = b.app.to_lowercase();
    BROWSERS.iter().any(|k| {
        let known_app = k.app.to_lowercase();
        k.key == b.key || k.key == app || known_app == app || known_app == b.key
    })
}

/// The `.app` bundle a process runs from, when its executable is the app's
/// own `Contents/MacOS` binary. Helper apps nested inside another bundle
/// (Chromium renderers live in `Frameworks/.../Helpers/*.app`) give `None`.
fn app_bundle_of_executable(exe: &str) -> Option<PathBuf> {
    let i = exe.find(".app/Contents/MacOS/")?;
    let root = &exe[..i + 4];
    if !root.starts_with('/') || root[..i].contains(".app/") {
        return None;
    }
    Some(PathBuf::from(root))
}

/// A browser descriptor for `bundle` when it is a Chromium-family app that
/// speaks the Chrome dictionary: AppleScript enabled in its Info.plist, a
/// scripting definition with Chrome's `active tab` property, and a Chromium
/// `<Name> Framework.framework`. Electron apps ship the framework but no such
/// dictionary, so they don't pass.
fn chrome_dictionary_browser(bundle: &Path) -> Option<Browser> {
    let contents = bundle.join("Contents");
    let d = read_info_plist(bundle)?;
    let scriptable = match d.get("NSAppleScriptEnabled") {
        Some(plist::Value::Boolean(b)) => *b,
        Some(plist::Value::String(s)) => {
            s.eq_ignore_ascii_case("yes") || s.eq_ignore_ascii_case("true")
        }
        _ => false,
    };
    if !scriptable {
        return None;
    }
    let id = d.get("CFBundleIdentifier")?.as_string()?;
    if !valid_bundle_id(id) {
        return None;
    }
    let sdef = d.get("OSAScriptingDefinition")?.as_string()?;
    if sdef.is_empty() || sdef.contains('/') || sdef.contains("..") {
        return None;
    }
    let dict = read_small_file(&contents.join("Resources").join(sdef), BUNDLE_FILE_MAX)?;
    if !String::from_utf8_lossy(&dict).contains("name=\"active tab\"") {
        return None;
    }
    let chromium = std::fs::read_dir(contents.join("Frameworks"))
        .ok()?
        .flatten()
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with(" Framework.framework")
        });
    if !chromium {
        return None;
    }
    // The name shows on the confirm card: printable, one line, short. An
    // empty name after cleaning falls back to the bundle's file name.
    let name = d
        .get("CFBundleName")
        .and_then(|v| v.as_string())
        .map(display_name)
        .filter(|n| !n.is_empty())
        .or_else(|| {
            bundle
                .file_stem()
                .map(|s| display_name(&s.to_string_lossy()))
        })
        .unwrap_or_default();
    if name.is_empty() {
        return None;
    }
    Some(Browser {
        key: Cow::Owned(name.to_lowercase()),
        app: Cow::Owned(name),
        dialect: Dialect::Chromium,
        bundle_id: Some(Cow::Owned(id.to_string())),
        bundle_path: Some(bundle.to_path_buf()),
    })
}

/// A name fit for the confirm card: no control characters and no invisible
/// Unicode format characters (bidi overrides, zero-width marks), trimmed,
/// at most 64 characters.
fn display_name(raw: &str) -> String {
    let invisible = |c: char| {
        matches!(c,
            '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}')
    };
    raw.chars()
        .filter(|&c| !c.is_control() && !invisible(c))
        .collect::<String>()
        .trim()
        .chars()
        .take(64)
        .collect()
}

/// Exact-name process check. Cheap enough to call synchronously.
fn app_running(name: &str) -> bool {
    std::process::Command::new("/usr/bin/pgrep")
        .arg("-x")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[derive(Clone, Debug, PartialEq)]
struct AppEntry {
    name: String,
    path: PathBuf,
}

fn app_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
}

fn installed_apps() -> Vec<AppEntry> {
    installed_apps_in(&app_roots())
}

/// Scan roots (plus one level of subfolders, e.g. `/Applications/Utilities`)
/// for `*.app` bundles. First sighting of a name wins, so the root order
/// gives `/Applications` precedence.
fn installed_apps_in(roots: &[PathBuf]) -> Vec<AppEntry> {
    let mut out: Vec<AppEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        scan_apps(root, 0, &mut out, &mut seen);
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn scan_apps(
    dir: &Path,
    depth: u8,
    out: &mut Vec<AppEntry>,
    seen: &mut std::collections::HashSet<String>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(stem) = name.strip_suffix(".app") {
            if seen.insert(stem.to_lowercase()) {
                out.push(AppEntry {
                    name: stem.to_string(),
                    path,
                });
            }
        } else if depth == 0 && !name.starts_with('.') {
            scan_apps(&path, 1, out, seen);
        }
    }
}

/// Resolve a model-supplied app name against the installed set: exact
/// (case-insensitive) match first, then a unique substring match. Ambiguity
/// and misses return errors that teach the model what to do next.
fn resolve_app<'a>(apps: &'a [AppEntry], query: &str) -> Result<&'a AppEntry, String> {
    let mut q = query.trim().to_lowercase();
    if let Some(stripped) = q.strip_suffix(".app") {
        q = stripped.to_string();
    }
    if q.is_empty() {
        return Err("missing required argument `name`".into());
    }
    if let Some(hit) = apps.iter().find(|a| a.name.to_lowercase() == q) {
        return Ok(hit);
    }
    let subs: Vec<&AppEntry> = apps
        .iter()
        .filter(|a| a.name.to_lowercase().contains(&q))
        .collect();
    match subs.len() {
        1 => Ok(subs[0]),
        0 => Err(format!(
            "no installed app matches `{query}` — call list_apps to see what's available"
        )),
        n => Err(format!(
            "`{query}` is ambiguous — matches: {}{}",
            subs.iter()
                .take(8)
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            if n > 8 { ", …" } else { "" }
        )),
    }
}

// ── Audit log ───────────────────────────────────────────────────────────────

/// Append one line per tool call to `{HOLLOW_VAULT}/.computer-actions.jsonl`.
/// Dot-prefixed, so the vault watcher and tree ignore it. Best-effort — an
/// unwritable log never blocks the action's result.
fn audit(tool: &str, args: &Value, outcome: &Result<String, String>, root: Option<&Path>) {
    let Some(vault) = root else {
        return;
    };
    let (ok, text) = match outcome {
        Ok(t) => (true, t.as_str()),
        Err(e) => (false, e.as_str()),
    };
    let mut summary: String = text.chars().take(300).collect();
    if text.chars().count() > 300 {
        summary.push('…');
    }
    let entry = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "tool": tool,
        "args": args,
        "ok": ok,
        "outcome": summary,
    });
    let path = vault.join(".computer-actions.jsonl");
    if let Ok(mut f) = crate::app_broker::open_audit_append(&path) {
        use std::io::Write;
        let _ = writeln!(f, "{entry}");
    }
}

// ── JSON-RPC helpers (same shapes as team_mcp) ──────────────────────────────

fn result(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } })
}

fn tool_text(id: Option<Value>, text: &str) -> Value {
    result(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
    )
}

fn tool_error(id: Option<Value>, text: &str) -> Value {
    result(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
    )
}

#[cfg(test)]
#[path = "computer_mcp_broker_tests.rs"]
pub(crate) mod broker_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on(f: impl std::future::Future<Output = Option<Value>>) -> Option<Value> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    /// Paperclip PR 21: every Tier-2 gate names its tool (a CONFIRM_TOOLS
    /// literal) and passes the call's arguments, so the approval broker can
    /// bind the request to the agent's tool call; and CONFIRM_TOOLS is
    /// exactly the set of tools whose schema says "Tier-2".
    #[test]
    fn every_confirm_names_its_gated_tool() {
        let whole = include_str!("computer_mcp.rs");
        let source = &whole[..whole
            .find(concat!("#[cfg(test)]\nmod ", "tests {"))
            .unwrap()];
        let calls: Vec<&str> = source
            .match_indices("    confirm(")
            .map(|(i, _)| &source[i..])
            .collect();
        assert_eq!(calls.len(), CONFIRM_TOOLS.len(), "one gate per Tier-2 tool");
        for (call, tool) in calls.iter().zip(CONFIRM_TOOLS) {
            let head: String = call.chars().take(160).collect();
            let head = head.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                head.starts_with(&format!("confirm( ctx, \"{tool}\", args,")),
                "{head}"
            );
        }
        assert!(crate::source_pins::contains(
            source,
            concat!(
                "confirm_tool(\"Computer control\", action, \"computer\", ",
                "Some(tool), Some(args))"
            )
        ));

        for bypass in [false, true] {
            let specs = tool_specs(bypass);
            let tier2: Vec<String> = specs
                .iter()
                .filter(|t| t["description"].as_str().unwrap_or("").contains("Tier-2"))
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect();
            assert_eq!(tier2, CONFIRM_TOOLS);
            // F6: the descriptions say what this server's mode does.
            for t in specs
                .iter()
                .filter(|t| tier2.contains(&t["name"].as_str().unwrap().to_string()))
            {
                let text = t["description"].as_str().unwrap();
                assert!(
                    text.contains(crate::mode_contract::computer_tier2(bypass)),
                    "{text}"
                );
                for phrase in crate::mode_contract::HOLLOW_INTERNAL_CONFIRM {
                    assert!(!bypass || !text.contains(phrase), "{phrase}: {text}");
                }
            }
        }
    }

    #[test]
    fn initialize_names_server_and_echoes_protocol() {
        let req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {} }
        });
        let resp = block_on(handle(&req)).unwrap();
        assert_eq!(resp["result"]["protocolVersion"], json!("2025-06-18"));
        assert_eq!(
            resp["result"]["serverInfo"]["name"],
            json!("hollow-computer")
        );
    }

    #[test]
    fn tools_list_has_all_verbs_with_valid_schema() {
        let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let resp = block_on(handle(&req)).unwrap();
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            vec![
                "open_url",
                "search_bookmarks",
                "open_app",
                "list_apps",
                "browser_current_tab",
                "browser_list_tabs",
                "quit_app"
            ]
        );
        for t in tools {
            assert!(t["description"].is_string());
            assert_eq!(t["inputSchema"]["type"], json!("object"));
        }
    }

    #[test]
    fn unknown_tool_is_a_tool_error_not_a_crash() {
        let req = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "format_disk", "arguments": {} }
        });
        let resp = block_on(handle(&req)).unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }

    #[test]
    fn validate_url_allows_http_s_only() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/a/b?q=1#frag").is_ok());
        assert!(validate_url("HTTPS://Example.com").is_ok()); // scheme case-insensitive
        assert!(validate_url("  https://example.com  ").is_ok()); // trimmed

        assert!(validate_url("").is_err());
        assert!(validate_url("example.com").is_err()); // no scheme
        assert!(validate_url("file:///etc/passwd").is_err());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("ftp://example.com").is_err());
        assert!(validate_url("https://").is_err()); // no host
        assert!(validate_url("https://exa mple.com").is_err()); // embedded whitespace
        assert!(validate_url("https://example.com/\u{0007}").is_err()); // control char
    }

    #[test]
    fn open_url_with_bad_scheme_is_rejected_through_the_tool_layer() {
        // End-to-end through tools/call: validation fires before any launcher
        // spawn, so this is side-effect-free.
        let req = json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "open_url", "arguments": { "url": "file:///etc/passwd" } }
        });
        let resp = block_on(handle(&req)).unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        let msg = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            msg.contains("http"),
            "should explain the scheme rule: {msg}"
        );
    }

    #[test]
    fn applescript_escape_neutralizes_literal_breakouts() {
        assert_eq!(
            applescript_escape(r#"say "hi" \ there"#),
            r#"say \"hi\" \\ there"#
        );
        assert_eq!(applescript_escape("a\nb\tc"), "a\\nb\\tc");
        // Other control chars are dropped entirely.
        assert_eq!(applescript_escape("x\u{0007}y"), "xy");
    }

    #[test]
    fn dialog_allowed_parses_all_outcomes() {
        assert!(dialog_allowed("button returned:Allow, gave up:false"));
        assert!(dialog_allowed("button returned:Allow")); // no giving-up clause
        assert!(!dialog_allowed("button returned:, gave up:true")); // timeout
        assert!(!dialog_allowed("button returned:Allow, gave up:true")); // stale Allow + timeout
        assert!(!dialog_allowed(""));
    }

    #[test]
    fn resolve_app_exact_substring_ambiguous_and_missing() {
        let apps = vec![
            AppEntry {
                name: "Safari".into(),
                path: PathBuf::from("/Applications/Safari.app"),
            },
            AppEntry {
                name: "Slack".into(),
                path: PathBuf::from("/Applications/Slack.app"),
            },
            AppEntry {
                name: "Visual Studio Code".into(),
                path: PathBuf::from("/Applications/Visual Studio Code.app"),
            },
            AppEntry {
                name: "Sara's Tools".into(),
                path: PathBuf::from("/Applications/Sara's Tools.app"),
            },
        ];
        // Exact, case-insensitive, with or without .app suffix.
        assert_eq!(resolve_app(&apps, "slack").unwrap().name, "Slack");
        assert_eq!(resolve_app(&apps, "Slack.app").unwrap().name, "Slack");
        // Unique substring.
        assert_eq!(
            resolve_app(&apps, "studio code").unwrap().name,
            "Visual Studio Code"
        );
        // Ambiguous substring lists candidates.
        let err = resolve_app(&apps, "sa").unwrap_err();
        assert!(err.contains("ambiguous") && err.contains("Safari"), "{err}");
        // Missing teaches list_apps.
        let err = resolve_app(&apps, "Photoshop").unwrap_err();
        assert!(err.contains("list_apps"), "{err}");
        // Empty.
        assert!(resolve_app(&apps, "  ").is_err());
    }

    #[test]
    fn resolve_browser_by_key_name_and_unknown() {
        assert_eq!(
            resolve_browser(Some("chrome")).unwrap().app,
            "Google Chrome"
        );
        assert_eq!(
            resolve_browser(Some("Google Chrome")).unwrap().app,
            "Google Chrome"
        );
        assert_eq!(resolve_browser(Some("SAFARI")).unwrap().app, "Safari");
        let helium = resolve_browser(Some("Helium")).unwrap();
        assert_eq!(helium.dialect, Dialect::Chromium);
        assert_eq!(helium.tell_target(), "application id \"net.imput.helium\"");
        assert_eq!(
            resolve_browser(Some("net.imput.helium")).unwrap().key,
            "helium"
        );
        let err = resolve_browser(Some("netscape")).unwrap_err();
        assert!(err.contains("supported") && err.contains("helium"), "{err}");
        assert!(err.contains("LibreWolf"), "{err}");
    }

    #[test]
    fn tell_target_uses_bundle_id_only_when_plain() {
        let chrome = resolve_browser(Some("chrome")).unwrap();
        assert_eq!(chrome.tell_target(), "application \"Google Chrome\"");
        let odd = Browser {
            key: "x".into(),
            app: "Odd \"App\"".into(),
            dialect: Dialect::Chromium,
            bundle_id: Some("bad\" & id".into()),
            bundle_path: None,
        };
        assert_eq!(odd.tell_target(), "application \"Odd \\\"App\\\"\"");
        assert!(valid_bundle_id("net.imput.helium"));
        assert!(valid_bundle_id("com.vivaldi.Vivaldi"));
        assert!(!valid_bundle_id(""));
        assert!(!valid_bundle_id("a b"));
        assert!(!valid_bundle_id("a\"b"));
        assert!(!valid_bundle_id("a/b"));
        assert!(!valid_bundle_id(&"a".repeat(256)));
        assert!(valid_bundle_id(&"a".repeat(255)));
    }

    #[test]
    fn app_bundle_of_executable_skips_nested_helpers() {
        assert_eq!(
            app_bundle_of_executable("/Applications/Helium.app/Contents/MacOS/Helium"),
            Some(PathBuf::from("/Applications/Helium.app"))
        );
        assert_eq!(
            app_bundle_of_executable(
                "/Applications/Helium.app/Contents/Frameworks/Helium Framework.framework/Versions/157.0.8103.78/Helpers/Helium Helper (Renderer).app/Contents/MacOS/Helium Helper (Renderer)"
            ),
            None
        );
        assert_eq!(app_bundle_of_executable("/usr/sbin/cfprefsd"), None);
        assert_eq!(
            app_bundle_of_executable("Helium.app/Contents/MacOS/Helium"),
            None
        );
    }

    /// A fake app bundle: Info.plist, optional sdef and Frameworks entry.
    fn fake_bundle(
        root: &Path,
        name: &str,
        id: &str,
        scriptable: bool,
        sdef: Option<&str>,
        framework: Option<&str>,
    ) -> PathBuf {
        use plist::{Dictionary, Value as P};
        let app = root.join(format!("{name}.app"));
        let contents = app.join("Contents");
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        std::fs::create_dir_all(contents.join("Frameworks")).unwrap();
        let mut d = Dictionary::new();
        d.insert("CFBundleIdentifier".into(), P::String(id.into()));
        d.insert("CFBundleName".into(), P::String(name.into()));
        d.insert("NSAppleScriptEnabled".into(), P::Boolean(scriptable));
        if let Some(text) = sdef {
            d.insert(
                "OSAScriptingDefinition".into(),
                P::String("scripting.sdef".into()),
            );
            std::fs::write(contents.join("Resources/scripting.sdef"), text).unwrap();
        }
        if let Some(f) = framework {
            std::fs::create_dir_all(contents.join("Frameworks").join(f)).unwrap();
        }
        P::Dictionary(d)
            .to_file_xml(contents.join("Info.plist"))
            .unwrap();
        app
    }

    #[test]
    fn chrome_dictionary_detection_reads_the_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let chrome_sdef = r#"<dictionary><suite name="Chromium Suite"><class name="window"><property name="active tab" code="acTa" type="tab"/></class></suite></dictionary>"#;
        // A Chromium browser with the Chrome dictionary (Helium's shape).
        let helium = fake_bundle(
            dir.path(),
            "Helium",
            "net.imput.helium",
            true,
            Some(chrome_sdef),
            Some("Helium Framework.framework"),
        );
        let b = chrome_dictionary_browser(&helium).expect("detected");
        assert_eq!(b.app, "Helium");
        assert_eq!(b.key, "helium");
        assert_eq!(b.bundle_id.as_deref(), Some("net.imput.helium"));
        assert_eq!(b.dialect, Dialect::Chromium);
        // An unknown one works the same, with no code change.
        let next = fake_bundle(
            dir.path(),
            "Nextium",
            "org.example.nextium",
            true,
            Some(chrome_sdef),
            Some("Nextium Framework.framework"),
        );
        assert_eq!(chrome_dictionary_browser(&next).unwrap().key, "nextium");
        // Electron: Chromium framework, no scripting dictionary.
        let electron = fake_bundle(
            dir.path(),
            "Chatty",
            "com.example.chatty",
            false,
            None,
            Some("Electron Framework.framework"),
        );
        assert!(chrome_dictionary_browser(&electron).is_none());
        // Scriptable but not the Chrome dictionary (Safari-like).
        let other = fake_bundle(
            dir.path(),
            "Otherfari",
            "com.example.otherfari",
            true,
            Some(r#"<property name="current tab" code="cTab"/>"#),
            Some("Otherfari Framework.framework"),
        );
        assert!(chrome_dictionary_browser(&other).is_none());
        // Chrome dictionary text but no Chromium framework.
        let noframe = fake_bundle(
            dir.path(),
            "Noframe",
            "com.example.noframe",
            true,
            Some(chrome_sdef),
            None,
        );
        assert!(chrome_dictionary_browser(&noframe).is_none());
    }

    fn running(bundle: &Path, id: &str) -> RunningApp {
        RunningApp {
            bundle: bundle.to_path_buf(),
            id: id.to_string(),
        }
    }

    fn set_plist_key(app: &Path, key: &str, value: plist::Value) {
        let path = app.join("Contents/Info.plist");
        let mut d = plist::Value::from_file(&path)
            .unwrap()
            .into_dictionary()
            .unwrap();
        d.insert(key.into(), value);
        plist::Value::Dictionary(d).to_file_xml(&path).unwrap();
    }

    const CHROME_SDEF: &str = r#"<dictionary><class name="window"><property name="active tab" code="acTa" type="tab"/></class></dictionary>"#;

    #[test]
    fn detection_rejects_escaping_sdef_names_symlinks_and_fifos() {
        let dir = tempfile::tempdir().unwrap();
        let mk = |name: &str| {
            fake_bundle(
                dir.path(),
                name,
                &format!("org.example.{}", name.to_lowercase()),
                true,
                Some(CHROME_SDEF),
                Some(&format!("{name} Framework.framework")),
            )
        };
        // Control: the plain bundle passes.
        assert!(chrome_dictionary_browser(&mk("Plain")).is_some());
        // An sdef name that climbs out of Resources.
        let up = mk("Upward");
        std::fs::write(up.join("Contents/x"), CHROME_SDEF).unwrap();
        set_plist_key(
            &up,
            "OSAScriptingDefinition",
            plist::Value::String("../x".into()),
        );
        assert!(chrome_dictionary_browser(&up).is_none());
        // An sdef that is a symlink (to a good file, so only the link fails).
        let link = mk("Linked");
        let sdef = link.join("Contents/Resources/scripting.sdef");
        std::fs::remove_file(&sdef).unwrap();
        std::os::unix::fs::symlink(
            dir.path()
                .join("Plain.app/Contents/Resources/scripting.sdef"),
            &sdef,
        )
        .unwrap();
        assert!(chrome_dictionary_browser(&link).is_none());
        // An sdef that is a FIFO: rejected at once, no blocking read.
        let fifo = mk("Piped");
        let sdef = fifo.join("Contents/Resources/scripting.sdef");
        std::fs::remove_file(&sdef).unwrap();
        let c = std::ffi::CString::new(sdef.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert!(chrome_dictionary_browser(&fifo).is_none());
        // An Info.plist that is a symlink.
        let plink = mk("PlistLink");
        let info = plink.join("Contents/Info.plist");
        std::fs::remove_file(&info).unwrap();
        std::os::unix::fs::symlink(dir.path().join("Plain.app/Contents/Info.plist"), &info)
            .unwrap();
        assert!(chrome_dictionary_browser(&plink).is_none());
        // read_small_file: size cap and non-files.
        let big = dir.path().join("big");
        std::fs::write(&big, vec![b'a'; 11]).unwrap();
        assert_eq!(read_small_file(&big, 11).map(|v| v.len()), Some(11));
        assert!(read_small_file(&big, 10).is_none());
        assert!(read_small_file(dir.path(), 1 << 20).is_none());
        assert!(read_small_file(Path::new("/dev/zero"), 1 << 20).is_none());
    }

    #[test]
    fn detected_browsers_show_their_bundle_and_cannot_pose_as_known_ones() {
        let dir = tempfile::tempdir().unwrap();
        let fake = |name: &str, id: &str| {
            let app = fake_bundle(
                dir.path(),
                name,
                id,
                true,
                Some(CHROME_SDEF),
                Some("Chromium Framework.framework"),
            );
            running(&app, id)
        };
        let apps = vec![
            fake("Safari", "org.evil.safari"),
            fake("Chrome", "org.evil.chrome"),
            fake("Nextium", "org.example.nextium"),
            // A known browser's exact id stays with its BROWSERS entry.
            fake("Helium", "net.imput.helium"),
        ];
        let found = chrome_dictionary_browsers_in(&apps);
        assert_eq!(found.len(), 1, "{found:?}");
        let b = &found[0];
        assert_eq!(b.app, "Nextium");
        let label = b.label();
        assert!(label.contains("org.example.nextium"), "{label}");
        assert!(label.contains("Nextium.app"), "{label}");
        // Known browsers keep their plain name.
        assert_eq!(resolve_browser(Some("helium")).unwrap().label(), "Helium");
    }

    #[test]
    fn known_browsers_with_a_bundle_id_run_by_bundle_id_not_name() {
        let helium = resolve_browser(Some("helium")).unwrap();
        let other = running(
            Path::new("/Applications/Helium.app"),
            "com.jadengeller.helium",
        );
        let real = running(Path::new("/Applications/Helium.app"), "net.imput.helium");
        // An unrelated app whose process is also "Helium" does not count.
        assert!(!browser_running(
            &helium,
            || std::slice::from_ref(&other),
            |_| true
        ));
        assert!(browser_running(
            &helium,
            || std::slice::from_ref(&real),
            |_| false
        ));
        // Name-only entries ask by process name and never take the snapshot.
        let chrome = resolve_browser(Some("chrome")).unwrap();
        assert!(browser_running(
            &chrome,
            || -> &[RunningApp] { panic!("no snapshot needed") },
            |n| n == "Google Chrome"
        ));
    }

    #[test]
    fn resolve_default_order_and_the_no_browser_error() {
        let detected = |name: &str, id: &str| Browser {
            key: name.to_lowercase().into(),
            app: name.to_string().into(),
            dialect: Dialect::Chromium,
            bundle_id: Some(id.to_string().into()),
            bundle_path: Some(PathBuf::from(format!("/Applications/{name}.app"))),
        };
        let none = || -> Vec<Browser> { Vec::new() };
        let no_scan = || -> Vec<Browser> { panic!("scan must not run") };
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // The default known browser wins when it runs, over the static order.
        let b = resolve_default(
            &ids(&["net.imput.helium"]),
            &|b: &Browser| b.key == "chrome" || b.key == "helium",
            &no_scan,
        )
        .unwrap();
        assert_eq!(b.key, "helium");
        // Default not running: first running known browser in BROWSERS order.
        let b = resolve_default(
            &ids(&["com.apple.safari"]),
            &|b: &Browser| b.key == "brave" || b.key == "helium",
            &no_scan,
        )
        .unwrap();
        assert_eq!(b.key, "brave");
        // Nothing known runs: the scan, default first.
        let scan = || vec![detected("One", "a.one"), detected("Two", "b.two")];
        let b = resolve_default(&ids(&["b.two"]), &|_: &Browser| false, &scan).unwrap();
        assert_eq!(b.app, "Two");
        let b = resolve_default(&[], &|_: &Browser| false, &scan).unwrap();
        assert_eq!(b.app, "One");
        // Nothing at all: the error teaches the way out.
        let err = resolve_default(&[], &|_: &Browser| false, &none).unwrap_err();
        assert!(err.starts_with("no supported browser is running"), "{err}");
        assert!(err.contains("helium") && err.contains("LibreWolf"), "{err}");
    }

    #[test]
    fn only_folders_with_local_state_are_chromium_user_data() {
        let dir = tempfile::tempdir().unwrap();
        let yes = dir.path().join("org.example.nextium");
        let no = dir.path().join("Nextium");
        std::fs::create_dir_all(yes.join("Default")).unwrap();
        std::fs::create_dir_all(no.join("Default")).unwrap();
        std::fs::write(yes.join("Local State"), "{}").unwrap();
        std::fs::create_dir_all(no.join("Local State")).unwrap(); // a folder, not the file
        assert!(is_chromium_user_data(&yes));
        assert!(!is_chromium_user_data(&no));
        assert!(!is_chromium_user_data(&dir.path().join("missing")));
    }

    #[test]
    fn detected_names_are_cleaned_and_fall_back_to_the_bundle_stem() {
        assert_eq!(display_name("  Nex\u{202E}tium\u{200B}\n "), "Nextium");
        assert_eq!(display_name(&"x".repeat(80)).len(), 64);
        let dir = tempfile::tempdir().unwrap();
        let app = fake_bundle(
            dir.path(),
            "Blankname",
            "org.example.blank",
            true,
            Some(CHROME_SDEF),
            Some("Blankname Framework.framework"),
        );
        set_plist_key(
            &app,
            "CFBundleName",
            plist::Value::String(" \u{200B}\t ".into()),
        );
        assert_eq!(chrome_dictionary_browser(&app).unwrap().app, "Blankname");
    }

    #[test]
    fn pick_detected_prefers_the_default_browser() {
        let mk = |name: &str, id: &str| Browser {
            key: name.to_lowercase().into(),
            app: name.to_string().into(),
            dialect: Dialect::Chromium,
            bundle_id: Some(id.to_string().into()),
            bundle_path: None,
        };
        let found = vec![mk("One", "a.one"), mk("Two", "b.two")];
        assert_eq!(
            pick_detected(found.clone(), &["B.TWO".into()]).unwrap().app,
            "Two"
        );
        assert_eq!(pick_detected(found, &[]).unwrap().app, "One");
        assert!(pick_detected(Vec::new(), &[]).is_none());
    }

    #[test]
    fn chromium_store_paths_cover_helium_and_runtime_browsers() {
        let helium = CHROMIUM_STORES
            .iter()
            .find(|s| s.2 == "helium")
            .expect("helium store");
        assert_eq!((helium.0, helium.1), ("net.imput.helium", "Helium"));
        assert!(CHROMIUM_STORES
            .iter()
            .all(|s| BROWSERS.iter().any(|b| b.key == s.2)));
        assert_eq!(
            chromium_store_candidates(Some("org.example.nextium"), "Nextium"),
            vec!["org.example.nextium".to_string(), "Nextium".to_string()]
        );
        assert_eq!(
            chromium_store_candidates(Some("../x"), " .. "),
            Vec::<String>::new()
        );

        // Profile discovery over a fixture user-data folder.
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("net.imput.helium");
        for p in ["Default", "Profile 1", "System Profile"] {
            std::fs::create_dir_all(base.join(p)).unwrap();
        }
        let roots = r#"{"roots":{"bookmark_bar":{"type":"folder","children":[{"type":"url","name":"Imput","url":"https://imput.net/"}]}}}"#;
        std::fs::write(base.join("Default/Bookmarks"), roots).unwrap();
        std::fs::write(base.join("Profile 1/Bookmarks"), roots).unwrap();
        let files = chromium_profile_files_in(&base);
        assert_eq!(
            files,
            vec![
                base.join("Default/Bookmarks"),
                base.join("Profile 1/Bookmarks")
            ]
        );
        let mut out = Vec::new();
        chromium_bookmarks(&files[0], "Helium".into(), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, "Helium");
        assert!(chromium_profile_files_in(&dir.path().join("missing")).is_empty());
    }

    #[test]
    fn installed_apps_in_finds_bundles_one_level_deep() {
        let root = crate::test_temp::process_dir("hollow-apps-test-");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Foo.app")).unwrap();
        std::fs::create_dir_all(root.join("Utilities/Bar.app")).unwrap();
        std::fs::create_dir_all(root.join("Utilities/Deeper/Baz.app")).unwrap(); // too deep
        std::fs::write(root.join("note.app"), "not a dir").unwrap(); // file, not a bundle
        let apps = installed_apps_in(std::slice::from_ref(&root));
        let names: Vec<&str> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Bar", "Foo"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn server_entry_respects_the_toggle() {
        let _toggles = crate::mcp_attach::SERVER_TOGGLES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let vault = PathBuf::from("/tmp/vault");
        set_computer_control_enabled(false);
        assert!(server_entry(&vault, "acceptEdits", "run-1", "chat-1").is_none());
        set_computer_control_enabled(true);
        let (name, entry) =
            server_entry(&vault, "acceptEdits", "run-1", "chat-1").expect("entry when enabled");
        assert_eq!(name, "hollow_computer");
        assert_eq!(entry["args"], json!(["computer-mcp"]));
        assert_eq!(entry["env"]["HOLLOW_VAULT"], json!("/tmp/vault"));
        assert!(
            entry["env"].get(ENV_BYPASS).is_none(),
            "tier 2 asks outside bypass"
        );
        for perm in ["auto", "plan"] {
            assert!(
                server_entry(&vault, perm, "run-1", "chat-1").unwrap().1["env"]
                    .get(ENV_BYPASS)
                    .is_none(),
                "{perm}"
            );
        }
        let (_, bypass) = server_entry(&vault, "bypassPermissions", "run-1", "chat-1").unwrap();
        assert_eq!(bypass["env"][ENV_BYPASS], json!("1"), "bypass asks nothing");
    }

    #[test]
    fn chromium_bookmark_walk_finds_nested_urls_and_skips_trash() {
        let dir = crate::test_temp::process_dir("hollow-bm-test-");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("Bookmarks");
        std::fs::write(
            &file,
            serde_json::to_string(&json!({
                "roots": {
                    "bookmark_bar": {
                        "type": "folder",
                        "children": [
                            { "type": "url", "name": "HRIS Pro", "url": "https://acme.hris.net/" },
                            { "type": "folder", "children": [
                                { "type": "url", "name": "Team Wiki", "url": "https://wiki.example.com/" }
                            ]}
                        ]
                    },
                    "other": { "type": "folder", "children": [
                        { "type": "url", "name": "News", "url": "https://news.example.com/" }
                    ]},
                    "trash": { "type": "folder", "children": [
                        { "type": "url", "name": "Deleted", "url": "https://gone.example.com/" }
                    ]}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let mut out = Vec::new();
        chromium_bookmarks(&file, "Chrome".into(), &mut out);
        let titles: Vec<&str> = out.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, vec!["HRIS Pro", "Team Wiki", "News"]);
        assert!(out.iter().all(|b| b.source == "Chrome"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safari_bookmark_walk_reads_leaf_dicts() {
        use plist::{Dictionary, Value as P};
        let mut uri = Dictionary::new();
        uri.insert("title".into(), P::String("HRIS Pro".into()));
        let mut leaf = Dictionary::new();
        leaf.insert(
            "WebBookmarkType".into(),
            P::String("WebBookmarkTypeLeaf".into()),
        );
        leaf.insert(
            "URLString".into(),
            P::String("https://acme.hris.net/".into()),
        );
        leaf.insert("URIDictionary".into(), P::Dictionary(uri));
        let mut root = Dictionary::new();
        root.insert(
            "WebBookmarkType".into(),
            P::String("WebBookmarkTypeList".into()),
        );
        root.insert("Children".into(), P::Array(vec![P::Dictionary(leaf)]));

        let mut out = Vec::new();
        walk_safari(&P::Dictionary(root), &mut out);
        assert_eq!(
            out,
            vec![Bookmark {
                title: "HRIS Pro".into(),
                url: "https://acme.hris.net/".into(),
                source: "Safari".into()
            }]
        );
    }

    #[test]
    fn bookmark_search_ranks_title_matches_first_and_requires_all_tokens() {
        let books = vec![
            Bookmark {
                title: "HRIS Pro".into(),
                url: "https://acme.hris.net/".into(),
                source: "Chrome".into(),
            },
            Bookmark {
                title: "Payroll runbook".into(),
                url: "https://wiki.example.com/hris".into(),
                source: "Chrome".into(),
            },
            Bookmark {
                title: "HRIS Pro time entry guide".into(),
                url: "https://docs.example.com/".into(),
                source: "Safari".into(),
            },
            Bookmark {
                title: "News".into(),
                url: "https://news.example.com/".into(),
                source: "Chrome".into(),
            },
        ];
        // Exact title beats longer title-phrase match beats URL-only match.
        let hits = search_bookmarks_in(&books, "hris pro");
        let titles: Vec<&str> = hits.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, vec!["HRIS Pro", "HRIS Pro time entry guide"]);
        // URL matches count when the title doesn't have the token.
        let hits = search_bookmarks_in(&books, "hris");
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].title, "HRIS Pro");
        // Case-insensitive; multi-token queries require every token somewhere.
        assert_eq!(search_bookmarks_in(&books, "HRIS guide").len(), 1);
        assert!(search_bookmarks_in(&books, "hris missingword").is_empty());
        assert!(search_bookmarks_in(&books, "   ").is_empty());
    }

    #[test]
    fn default_browser_resolves_from_launchservices_handlers() {
        use plist::{Dictionary, Value as P};
        fn handler(scheme: &str, bundle: &str) -> P {
            let mut d = Dictionary::new();
            d.insert("LSHandlerURLScheme".into(), P::String(scheme.into()));
            d.insert("LSHandlerRoleAll".into(), P::String(bundle.into()));
            P::Dictionary(d)
        }
        let mut root = Dictionary::new();
        root.insert(
            "LSHandlers".into(),
            P::Array(vec![
                handler("mailto", "com.apple.mail"),   // irrelevant scheme
                handler("http", "com.brave.browser"),  // the one that counts
                handler("https", "com.google.chrome"), // first http hit wins
            ]),
        );
        let b = default_browser_from_launchservices(&P::Dictionary(root)).expect("browser");
        assert_eq!(b.key, "brave");

        // Unknown bundle id or no http handler → None.
        let mut root = Dictionary::new();
        root.insert(
            "LSHandlers".into(),
            P::Array(vec![handler("http", "org.mozilla.firefox")]),
        );
        assert!(default_browser_from_launchservices(&P::Dictionary(root)).is_none());
        assert!(default_browser_from_launchservices(&P::Dictionary(Dictionary::new())).is_none());
    }

    #[test]
    fn browser_for_bundle_id_maps_known_browsers() {
        assert_eq!(
            browser_for_bundle_id("com.brave.browser").unwrap().key,
            "brave"
        );
        assert_eq!(
            browser_for_bundle_id("COM.APPLE.SAFARI").unwrap().key,
            "safari"
        );
        assert_eq!(
            browser_for_bundle_id("company.thebrowser.browser")
                .unwrap()
                .key,
            "arc"
        );
        assert_eq!(
            browser_for_bundle_id("net.imput.helium").unwrap().app,
            "Helium"
        );
        assert!(browser_for_bundle_id("org.mozilla.firefox").is_none());
        // Beta and Canary builds are other apps: not sent to the stable one.
        assert!(browser_for_bundle_id("com.google.chrome.beta").is_none());
        assert!(browser_for_bundle_id("com.brave.browser.nightly").is_none());
    }

    #[test]
    fn unreadable_source_notes_only_surface_on_a_miss() {
        let books = vec![Bookmark {
            title: "HRIS Pro".into(),
            url: "https://acme.hris.net/".into(),
            source: "Brave".into(),
        }];
        let notes = vec!["Safari bookmarks couldn't be read.".to_string()];
        // Hit → clean result, no note.
        let hit = format_bookmark_results(&books, &notes, "hris");
        assert!(hit.contains("acme.hris.net"));
        assert!(!hit.contains("Note:"), "{hit}");
        // Miss → the note explains where the bookmark might be hiding.
        let miss = format_bookmark_results(&books, &notes, "nope");
        assert!(miss.contains("No bookmarks match"));
        assert!(
            miss.contains("Note: Safari bookmarks couldn't be read."),
            "{miss}"
        );
    }

    #[test]
    fn search_bookmarks_requires_a_query() {
        let resp = block_on(handle(&json!({
            "jsonrpc": "2.0", "id": 6, "method": "tools/call",
            "params": { "name": "search_bookmarks", "arguments": {} }
        })))
        .unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("query"));
    }

    #[test]
    fn bash_deny_patterns_are_permission_rules() {
        for p in BASH_DENY {
            assert!(p.starts_with("Bash(") && p.ends_with(":*)"), "{p}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn audit_refuses_symlinks_and_non_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside.txt");
        std::fs::write(&target, "untouched").unwrap();
        let log = dir.path().join(".computer-actions.jsonl");
        std::os::unix::fs::symlink(&target, &log).unwrap();

        audit("click", &json!({}), &Ok("ok".into()), Some(dir.path()));
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "untouched",
            "symlink target must never be written to"
        );

        std::fs::remove_file(&log).unwrap();
        std::fs::create_dir(&log).unwrap();
        audit("click", &json!({}), &Ok("ok".into()), Some(dir.path()));
        assert!(log.is_dir());
    }
}
