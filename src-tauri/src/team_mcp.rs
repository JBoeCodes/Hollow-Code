//! Hollow-owned MCP delegation to any installed teammate, excluding the primary.
//! Child agents do not receive this server, so delegation cannot recurse.
//!
//! **Usage (Paperclip PR 6).** Every delegation writes one usage-ledger row
//! from this subprocess. The row goes to the configured vault's session store,
//! named by `HOLLOW_VAULT_ROOT`, never by `HOLLOW_VAULT` (the run's cwd, which
//! for a workspace chat or a workflow with its own `cwd` is not the vault). It
//! is credited to the parent run through `HOLLOW_RUN_SURFACE`,
//! `HOLLOW_RUN_SCOPE` and `HOLLOW_RUN_ID` ([`delegate_env`]). Without
//! `HOLLOW_VAULT_ROOT` nothing is recorded. The server ends its in-flight
//! delegation on SIGTERM (the GUI's stop and finish sweep sends one), so that
//! row records `cancelled` with whatever it accumulated; a SIGKILL leaves it
//! `running` for the parent's finish to close as `interrupted`.
//!
//! These writes land only in the Sessions DB outside the vault and in the
//! unwatched `.hollow-health.jsonl`, so they can never raise a `run_guard`
//! flag against the parent run.
//!
//! **Usage budgets (Paperclip PR 8b).** Before it resolves the teammate, a
//! delegation runs the READ-ONLY budget preflight
//! (`budgets::preflight_readonly`: it opens no pause, saves nothing, queues
//! no review and sends no notification, so this subprocess never writes
//! watched vault state). A delegate under an automation (or an unknown
//! parent: fail closed) is refused while a hard-stop budget holds its scope,
//! and a Claude teammate over an enforced token limit mid-run is stopped:
//! only its direct process is killed (the spawn is inherited, so no process
//! group is ever signalled), its partial usage row is written as
//! `budget_stopped`, and the primary gets a tool error. A delegate under
//! chat is never refused or stopped; a warning is prefixed to its result.

use crate::agent::{self, AgentKind, RunSpec};
use crate::budgets;
use crate::usage_ledger::{self, DelegateParent, Meter};
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Default MCP protocol version echoed when a client doesn't send one.
const DEFAULT_PROTOCOL: &str = "2024-11-05";
/// Wall-clock cap on a single delegation (a bounded consultation, F10):
/// long enough for image gen / a multi-step coding task; short enough to
/// stay within an MCP client's per-tool-call timeout. A packet's
/// `budget.seconds` lowers it, never raises it (`delegation::build`).
/// `run_collect` enforces it and kills the teammate (only its direct
/// process: this server never signals a process group; the GUI's stop and
/// finish sweep ends anything the teammate left behind).
const TEAMMATE_TIMEOUT: Duration = Duration::from_secs(crate::delegation::CONSULTATION_SECONDS);
/// Longest a delegation waits for a machine-wide run permit (Paperclip PR 7)
/// before it gives up with a tool error.
const TEAMMATE_PERMIT_WAIT: Duration = Duration::from_secs(crate::delegation::PERMIT_WAIT_SECONDS);
/// Codex's own MCP tool-call timeout for this server (`tool_timeout_sec`;
/// Codex's default is 60 s): the permit wait and the deadline, plus room
/// for the result (F10, D1). Claude's MCP client has no shorter one.
pub const CODEX_TOOL_TIMEOUT_SECS: u64 = 240;
/// This server's name in an mcpServers map.
pub const SERVER_NAME: &str = "hollow_team";

/// Whether Claude may delegate to teammates. Synced from the frontend
/// `teamworkEnabled` setting via the `set_teamwork_enabled` command. Defaults
/// on so a fresh install with a teammate CLI gets teamwork out of the box.
static TEAMWORK_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_teamwork_enabled(v: bool) {
    TEAMWORK_ENABLED.store(v, Ordering::Relaxed);
}
pub fn teamwork_enabled() -> bool {
    TEAMWORK_ENABLED.load(Ordering::Relaxed)
}

/// Role presets — "Codex = Engineer", "Antigravity = Designer & web
/// researcher". Set from Settings via the `set_agent_role` command and
/// carried to the `team-mcp` subprocess as env vars on the spawn (the
/// statics here live in the GUI process; the server reads its env).
/// `role` colors the tool description (when Claude delegates), `brief`
/// is prepended to every delegated task as standing instructions.
#[derive(Debug, Clone, Default)]
pub struct RolePreset {
    pub role: String,
    pub brief: String,
}

static ROLES: Lazy<Mutex<HashMap<&'static str, RolePreset>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Stable env-var stem per teammate. Also the key the frontend uses.
fn role_key(kind: AgentKind) -> Option<&'static str> {
    match kind {
        AgentKind::Codex => Some("CODEX"),
        AgentKind::Antigravity => Some("ANTIGRAVITY"),
        AgentKind::Claude => Some("CLAUDE"),
        AgentKind::Grok => Some("GROK"),
    }
}

pub fn set_role(agent: &str, role: String, brief: String) {
    let key = match agent.to_lowercase().as_str() {
        "claude" => "CLAUDE",
        "codex" => "CODEX",
        "antigravity" => "ANTIGRAVITY",
        "grok" => "GROK",
        _ => return,
    };
    if let Ok(mut m) = ROLES.lock() {
        m.insert(key, RolePreset { role, brief });
    }
}

/// Role for `kind` as seen FROM THE SERVER SUBPROCESS — env first (how the
/// GUI hands presets across the spawn), then the in-process statics (so unit
/// tests and same-process callers see sets immediately).
fn role_for(kind: AgentKind) -> Option<RolePreset> {
    let key = role_key(kind)?;
    let from_env =
        |suffix: &str| request_env(&format!("HOLLOW_{suffix}_{key}")).unwrap_or_default();
    let role = from_env("ROLE");
    let brief = from_env("BRIEF");
    if !role.is_empty() || !brief.is_empty() {
        return Some(RolePreset { role, brief });
    }
    if REQUEST.try_with(|_| ()).is_ok() {
        return None;
    }
    ROLES.lock().ok()?.get(key).cloned()
}

/// MCP allowlist token covering both team tools (whole-server form). Appended
/// to a workflow's `allowed_tools` when that list is restrictive, so a
/// tool-gated workflow can still reach teammates. Must match the server key
/// [`server_entry_for`] returns (`hollow_team`).
pub const TEAM_TOOL_ALLOW: &str = "mcp__hollow_team";

/// The parent run's canonical permission mode. A teammate runs in that mode
/// too, never a stronger one ([`delegate_grant`]).
pub const ENV_PARENT_PERM: &str = "HOLLOW_TEAM_PARENT_PERM";
/// The parent run's allowed and denied tools, as JSON string arrays. A
/// teammate keeps every restriction its parent runs under
/// ([`delegate_grant`]).
pub const ENV_PARENT_ALLOWED: &str = "HOLLOW_TEAM_PARENT_ALLOWED";
pub const ENV_PARENT_DENIED: &str = "HOLLOW_TEAM_PARENT_DENIED";

/// The tool restrictions a run spawns with, carried to its teammates. An
/// empty `allowed` list means unrestricted, as on [`RunSpec`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolLimits {
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
}

impl ToolLimits {
    pub fn new(allowed: &[String], denied: &[String]) -> Self {
        Self {
            allowed: allowed.to_vec(),
            denied: denied.to_vec(),
        }
    }
}

/// What a teammate runs with ([`delegate_grant`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DelegateGrant {
    pub perm: &'static str,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
    /// Told to the primary with the result when Hollow ran the teammate
    /// narrower than the parent, so the limit is visible in the transcript.
    pub note: Option<String>,
}

/// Inert compatibility exclusions: host-client tools no other CLI has
/// (`agent::validate_run_spec` accepts them everywhere).
const INERT_DENIES: &[&str] = &["SendUserFile", "AskUserQuestion", "SendUserMessage"];

/// Whether `kind` at `perm` runs every shell command inside an OS sandbox
/// that refuses Apple Events and LaunchServices, so the parent's
/// `osascript`/`open` deny ([`crate::computer_mcp::BASH_DENY`]) holds without
/// a deny-rule flag. Probed 2026-10-02 against the commands themselves:
/// - Codex 0.153.4 `codex sandbox` (its seatbelt) in `workspace-write`:
///   `osascript` to System Events fails with -10827, `open -a Calculator`
///   can't find the app, and a write to `$HOME` is refused. `plan` is the
///   stricter read-only profile. Not `auto`: its reviewer can approve a
///   command outside the sandbox. Not bypass: no sandbox.
/// - Antigravity: accept-edits and plan deny every command (agy 1.2.14).
///   It has no auto ([`agent::unavailable_mode`]): its `--sandbox` blocked
///   `osascript`/`open` (-54) but not its file tools (agy 1.2.15 matrix).
///   Not bypass.
/// - Grok 1.0.46 `--sandbox read-only` / `workspace` (plan and default /
///   acceptEdits, `agent::grok_sandbox`): `osascript` and `open` fail with
///   -54 and a write to `$HOME` is refused, even at bypass. Not auto or
///   bypass: `--sandbox off`. Grok also takes the deny pair as `--deny`
///   (`agent::holds_deny`), which [`delegate_grant`] checks first.
fn sandbox_blocks_desktop(kind: AgentKind, perm: &str) -> bool {
    match kind {
        AgentKind::Claude => false,
        AgentKind::Codex => matches!(perm, "plan" | "default" | "acceptEdits"),
        AgentKind::Antigravity => matches!(perm, "plan" | "default" | "acceptEdits"),
        AgentKind::Grok => agent::grok_sandbox(perm) != "off",
    }
}

/// The strongest mode at which `kind`'s sandbox still holds the desktop
/// deny, or `None` when it has none ([`sandbox_blocks_desktop`]).
fn desktop_safe_mode(kind: AgentKind) -> Option<&'static str> {
    match kind {
        AgentKind::Codex => Some("acceptEdits"),
        AgentKind::Antigravity => Some("acceptEdits"),
        AgentKind::Claude | AgentKind::Grok => None,
    }
}

fn mode_rank(perm: &str) -> u8 {
    match perm {
        "plan" => 0,
        "default" => 1,
        "acceptEdits" => 2,
        "auto" => 3,
        _ => 4,
    }
}

/// The mode and tool lists a teammate of `kind` runs with, capped by its
/// parent (merged plan 2026-10-02, slice 1; reviews A·S1, B·F03). Before
/// this, an Antigravity teammate ran at unsandboxed bypass under any parent,
/// and every teammate dropped the parent's tool lists.
///
/// - **Mode:** the parent's, carried to the teammate's agent by
///   `agent::inherited_mode` from the parent's agent (`parent_agent`, the
///   server's `HOLLOW_PRIMARY_AGENT`; unknown reads the most restrictive
///   way), the same mapping a started chat gets (Q-R3 round 2), so it never
///   does more than the parent's mode on either axis: a Codex auto parent's
///   Grok runs acceptEdits (its auto has the sandbox off), a Claude
///   `default` or `acceptEdits` parent's Codex runs read-only (Codex can't
///   edit without running commands). A missing or unknown parent mode
///   refuses (fail closed); `plan` never attaches the team server, and
///   refuses here too.
/// - **Tools:** allowed lists intersect (a teammate has no list of its own, so
///   it gets the parent's); denied lists accumulate; a deny always wins, so a
///   denied tool leaves the allowed list. A restricted list that ends empty
///   refuses rather than reading as "unrestricted".
/// - **Providers without tool lists** (all but Claude): an allowed list
///   refuses. A deny is kept only if it is inert ([`INERT_DENIES`]), the
///   provider enforces it as a flag (Grok's `--deny` for the computer-control
///   pair, `agent::holds_deny`), or it is that `osascript`/`open` deny and the
///   teammate's sandbox holds it at this mode ([`sandbox_blocks_desktop`]);
///   otherwise Hollow runs the teammate at the strongest mode whose sandbox
///   does ([`desktop_safe_mode`]) and says so, or refuses.
///
/// Errors are the tool error the primary sees, so a refusal is visible in
/// the transcript.
pub(crate) fn delegate_grant(
    kind: AgentKind,
    parent_agent: Option<AgentKind>,
    parent_perm: Option<&str>,
    parent: &ToolLimits,
) -> Result<DelegateGrant, String> {
    let label = kind.label();
    let mut perm: &'static str = match parent_perm.map(str::trim) {
        Some("bypassPermissions") => "bypassPermissions",
        Some("auto") => "auto",
        Some("acceptEdits") => "acceptEdits",
        Some("default") => "default",
        Some("plan") => {
            return Err("A read-only (plan) run can't delegate: a teammate would be a way to make changes.".into())
        }
        _ => {
            return Err(format!(
                "Hollow didn't start {label}: this run's permission mode is unknown, so a teammate's limits can't be set."
            ))
        }
    };
    // Merged plan slice 2: a mode the teammate can't run (Antigravity's
    // auto) refuses with its reason; never a wider or quietly narrower mode.
    if let Some(reason) = agent::unavailable_mode(kind, perm) {
        return Err(format!("Hollow didn't start {label}. {reason}"));
    }
    let mut notes: Vec<String> = Vec::new();
    let carried = agent::inherited_mode(parent_agent, kind, perm);
    if carried != perm {
        notes.push(if carried == "plan" {
            format!("{label} ran read-only, because its {perm} would do more than this run may.")
        } else {
            format!("{label} ran in {carried}, because its {perm} would do more than this run may.")
        });
        perm = carried;
    }

    let clean = |list: &[String]| {
        let mut out: Vec<String> = Vec::new();
        for tool in list.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            if !out.iter().any(|seen| seen == tool) {
                out.push(tool.to_string());
            }
        }
        out
    };
    let denied = clean(&parent.denied);
    let mut allowed = clean(&parent.allowed);
    allowed.retain(|tool| !denied.contains(tool));
    if !parent.allowed.is_empty() && allowed.is_empty() {
        return Err(format!(
            "Hollow didn't start {label}: none of the tools this run allows is left once its denied tools are removed."
        ));
    }

    if agent::capabilities(kind).tool_restrictions {
        return Ok(DelegateGrant {
            perm,
            allowed,
            denied,
            note: join_notes(notes),
        });
    }
    if !allowed.is_empty() {
        return Err(format!(
            "Hollow didn't start {label}: this run is limited to {}, and {label} can't be held to a tool list. Ask Claude instead, or remove the restriction.",
            allowed.join(", ")
        ));
    }
    let mut kept = Vec::new();
    for tool in denied {
        if INERT_DENIES.contains(&tool.as_str()) || agent::holds_deny(kind, &tool) {
            kept.push(tool);
            continue;
        }
        if !crate::computer_mcp::BASH_DENY.contains(&tool.as_str()) {
            return Err(format!(
                "Hollow didn't start {label}: this run denies {tool}, and {label} can't be held to that. Ask Claude instead."
            ));
        }
        if sandbox_blocks_desktop(kind, perm) {
            continue;
        }
        match desktop_safe_mode(kind).filter(|safe| mode_rank(safe) < mode_rank(perm)) {
            Some(safe) => {
                perm = safe;
                notes.push(format!(
                    "{label} ran inside its sandbox ({}), because this run blocks osascript and open while computer control is on.",
                    if safe == "auto" { "auto" } else { "accept edits" }
                ));
            }
            None => {
                return Err(format!(
                    "Hollow didn't start {label}: this run blocks osascript and open while computer control is on, and {label} can't be held to that. Ask Claude or Codex instead, or turn computer control off."
                ))
            }
        }
    }
    if kind == AgentKind::Antigravity && matches!(perm, "default" | "acceptEdits" | "plan") {
        notes.push(
            "Antigravity ran in its sandbox in accept-edits mode, where it can't write files or run commands. It can write only in Bypass all: its Auto isn't confined to the working folder.".into(),
        );
    }
    Ok(DelegateGrant {
        perm,
        allowed: Vec::new(),
        denied: kept,
        note: join_notes(notes),
    })
}

/// What a teammate of `kind` ran without when it can't take Hollow's per-run
/// MCP config (`agent::capabilities(..).owned_mcp` false: Antigravity, Grok),
/// told with its result so the primary and the transcript see it (merged
/// plan slice 2 item 4). `None` for a provider that takes one. Teammates never
/// get Hollow's servers (`mcp_config: None`); Claude and Codex then run with
/// no MCP at all, while these two load whatever their own config names.
fn no_cards_note(kind: AgentKind) -> Option<&'static str> {
    // A teammate runs `agy -p`, which takes no servers whatever the chat can.
    if agent::capabilities(kind).owned_mcp && kind != AgentKind::Antigravity {
        return None;
    }
    match kind {
        AgentKind::Grok => Some(
            "Grok can't take Hollow's per-run tool setup, so it ran without Hollow's confirm or question cards and app tools. MCP servers set up in ~/.grok still load.",
        ),
        AgentKind::Antigravity => Some(
            "Antigravity can't take Hollow's per-run tool setup, so it ran without Hollow's confirm or question cards and app tools.",
        ),
        AgentKind::Claude | AgentKind::Codex => None,
    }
}

fn join_notes(notes: Vec<String>) -> Option<String> {
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// The parent's limits as the team server reads them back from its env. A
/// list that is present but unreadable refuses: dropping it would lift a
/// restriction.
fn parent_limits(env: &dyn Fn(&str) -> Option<String>) -> Result<ToolLimits, String> {
    let list = |key: &str| -> Result<Vec<String>, String> {
        match env(key).filter(|raw| !raw.trim().is_empty()) {
            None => Ok(Vec::new()),
            Some(raw) => serde_json::from_str::<Vec<String>>(&raw).map_err(|_| {
                "Hollow didn't start the teammate: this run's tool limits couldn't be read."
                    .to_string()
            }),
        }
    };
    Ok(ToolLimits {
        allowed: list(ENV_PARENT_ALLOWED)?,
        denied: list(ENV_PARENT_DENIED)?,
    })
}

/// The environment the `team-mcp` server is spawned with. Pure apart from the
/// role presets it copies. `HOLLOW_VAULT` stays the run's cwd (delegations
/// default to it); `HOLLOW_VAULT_ROOT` is the configured vault, where the
/// server records its delegations' usage; `HOLLOW_RUN_*` name the parent run
/// (the surface, the bare chat session key or workflow slug, and the history
/// run id). None of these names contain KEY, SECRET or TOKEN, so Codex's MCP
/// environment filter passes them through. `HOLLOW_TEAM_PARENT_*` carry the
/// parent's mode and tool limits ([`delegate_grant`]).
fn delegate_env(
    cwd: &Path,
    vault_root: &Path,
    primary: AgentKind,
    parent: &DelegateParent,
    perm: &str,
) -> serde_json::Map<String, Value> {
    let mut env = serde_json::Map::new();
    env.insert(ENV_PARENT_PERM.into(), json!(perm));
    // Always written, empty or not, so a teammate never mistakes a missing
    // key for "unrestricted" by accident of an older spawn.
    env.insert(
        ENV_PARENT_ALLOWED.into(),
        json!(serde_json::to_string(&parent.tool_limits.allowed).unwrap_or_default()),
    );
    env.insert(
        ENV_PARENT_DENIED.into(),
        json!(serde_json::to_string(&parent.tool_limits.denied).unwrap_or_default()),
    );
    env.insert("HOLLOW_VAULT".into(), json!(cwd.to_string_lossy()));
    env.insert(
        usage_ledger::ENV_VAULT_ROOT.into(),
        json!(vault_root.to_string_lossy()),
    );
    env.insert("HOLLOW_PRIMARY_AGENT".into(), json!(primary.as_str()));
    env.insert(
        usage_ledger::ENV_RUN_SURFACE.into(),
        json!(parent.surface.as_str()),
    );
    env.insert(usage_ledger::ENV_RUN_SCOPE.into(), json!(parent.scope));
    env.insert(usage_ledger::ENV_RUN_ID.into(), json!(parent.run_id));
    // The parent's scratch folder (Paperclip PR 15a). Explicit because Codex
    // forwards only a server's own environment; a delegate still validates it
    // (`scratch::inherited`) before use.
    if let Some(scratch) = parent.scratch.as_deref() {
        env.insert(
            crate::scratch::SCRATCH_ENV.into(),
            json!(scratch.to_string_lossy()),
        );
    }
    // Carry role presets across the spawn — the server reads its env.
    if let Ok(roles) = ROLES.lock() {
        for (key, preset) in roles.iter() {
            if !preset.role.trim().is_empty() {
                env.insert(format!("HOLLOW_ROLE_{key}"), json!(preset.role.trim()));
            }
            if !preset.brief.trim().is_empty() {
                env.insert(format!("HOLLOW_BRIEF_{key}"), json!(preset.brief.trim()));
            }
        }
    }
    env
}

/// The `hollow_team` entry for an `--mcp-config` mcpServers map, or `None`
/// when teamwork is off or no teammate CLI is installed. The server is this
/// same binary re-invoked as `team-mcp`, with the run's cwd passed through
/// `HOLLOW_VAULT` so delegations default to it, and the vault root passed
/// separately (see [`delegate_env`]). Chat and workflow spawns merge this
/// with the other Hollow servers' entries in `mcp_attach::server_map_for`
/// (Paperclip PR 12 removed the team-only `mcp_config_json_for` workflows
/// used before).
pub fn server_entry_for(
    cwd: &Path,
    vault_root: &Path,
    primary: AgentKind,
    parent: &DelegateParent,
    perm: &str,
) -> Option<(&'static str, Value)> {
    if !teamwork_enabled() {
        return None;
    }
    if !agent::available().iter().any(|kind| *kind != primary) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let env = delegate_env(cwd, vault_root, primary, parent, perm);
    Some((
        SERVER_NAME,
        json!({
            "command": exe.to_string_lossy(),
            "args": ["team-mcp"],
            "env": env
        }),
    ))
}

/// Entry point for the `team-mcp` subcommand. Blocks on a current-thread
/// runtime serving JSON-RPC over stdio until stdin reaches EOF, or until
/// SIGTERM. On SIGTERM the process is first marked as terminating, then the
/// serve future (and with it an in-flight delegation) is dropped, so that
/// delegation's usage row records `cancelled` with what it accumulated,
/// inside the GUI sweep's grace period, instead of dying with no destructors.
/// The runtime is then shut down without waiting: `tokio::io::stdin()` reads
/// on the blocking pool with an uncancellable read, so an ordinary runtime
/// drop would hang until the parent CLI closed stdin or the GUI's SIGKILL
/// arrived. Order: mark terminating, drop the serve future (the row is
/// written), exit.
pub fn serve() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime");
    let terminated_first = rt.block_on(async {
        tokio::select! {
            _ = serve_async() => false,
            _ = terminated() => true,
        }
    });
    if terminated_first {
        rt.shutdown_background();
    }
}

/// Completes on SIGTERM, after marking the process as terminating (so a
/// usage row dropped from here on records `cancelled`). Never completes when
/// no handler can be installed, which leaves the default action in place.
#[cfg(unix)]
async fn terminated() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            term.recv().await;
            usage_ledger::note_process_terminating();
        }
        Err(_) => std::future::pending::<()>().await,
    }
}

#[cfg(not(unix))]
async fn terminated() {
    std::future::pending::<()>().await
}

async fn serve_async() {
    serve_lines(
        BufReader::new(tokio::io::stdin()),
        tokio::io::stdout(),
        |req: Value| async move { handle(&req).await },
    )
    .await
}

/// The JSON-RPC loop over any line stream: one message per line in, one
/// response per line out, in the order they finish.
///
/// **Requests run at once (F10, D2).** Each request is handled as soon as
/// its line arrives, while earlier ones are still running, so a
/// consultation's wait is measured from when the asking agent sent it: its
/// permit wait and deadline, never another consultation's too (the asking
/// agent's own tool timeout, Codex's 240 s here, counts from the send).
/// Responses carry their request's id, as JSON-RPC requires. The machine-wide
/// run permits still bound how many teammates run. In-flight requests live
/// in this future: dropping it (the SIGTERM path in [`serve`]) drops them
/// all, so each one's usage row records `cancelled`. At EOF the requests
/// already running finish and are answered, as one in flight was before.
async fn serve_lines<R, W, F, Fut>(reader: R, mut writer: W, handler: F)
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
    F: Fn(Value) -> Fut,
    Fut: std::future::Future<Output = Option<Value>>,
{
    let mut lines = reader.lines();
    let mut running: Vec<std::pin::Pin<Box<Fut>>> = Vec::new();
    let mut reading = true;
    while reading || !running.is_empty() {
        tokio::select! {
            line = lines.next_line(), if reading => {
                let raw = match line {
                    Ok(Some(raw)) => raw,
                    _ => {
                        reading = false;
                        continue;
                    }
                };
                let line = raw.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(line) {
                    Ok(req) => running.push(Box::pin(handler(req))),
                    Err(_) => continue, // ignore non-JSON noise
                }
            }
            resp = next_finished(&mut running), if !running.is_empty() => {
                if let Some(resp) = resp {
                    let mut s = resp.to_string();
                    s.push('\n');
                    if writer.write_all(s.as_bytes()).await.is_err() {
                        break;
                    }
                    let _ = writer.flush().await;
                }
            }
        }
    }
}

/// The first of `running` to finish, removed from the list (never resolves
/// while it is empty).
async fn next_finished<Fut>(running: &mut Vec<std::pin::Pin<Box<Fut>>>) -> Fut::Output
where
    Fut: std::future::Future,
{
    std::future::poll_fn(|cx| {
        for i in 0..running.len() {
            if let std::task::Poll::Ready(out) = running[i].as_mut().poll(cx) {
                drop(running.swap_remove(i));
                return std::task::Poll::Ready(out);
            }
        }
        std::task::Poll::Pending
    })
    .await
}

/// Dispatch one JSON-RPC message. Returns `Some(response)` for requests
/// (messages with an `id`) and `None` for notifications (no `id`) and for
/// methods that don't warrant a reply.
async fn handle(req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    match method {
        "initialize" => {
            // Echo the client's requested protocol version for max compatibility.
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
                    "serverInfo": { "name": "hollow-team", "version": env!("CARGO_PKG_VERSION") },
                }),
            ))
        }
        "tools/list" => Some(result(id, json!({ "tools": tool_specs() }))),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            Some(call_tool(id, name, &args).await)
        }
        "ping" => Some(result(id, json!({}))),
        // Notifications (no id) and anything else: reply with an error only
        // when it's a request that carries an id.
        _ => id.map(|id| error(Some(id), -32601, &format!("method not found: {method}"))),
    }
}

tokio::task_local! {
    static REQUEST: Context;
}

/// Captured from the selected server entry, never the host process globals.
#[derive(Clone)]
pub(crate) struct Context {
    pub env: std::collections::BTreeMap<String, String>,
    pub permit: crate::shared_mcp::Permit,
}

fn request_env(key: &str) -> Option<String> {
    REQUEST
        .try_with(|ctx| ctx.env.get(key).cloned())
        .unwrap_or_else(|_| std::env::var(key).ok())
}

pub(crate) async fn handle_in(req: &Value, ctx: Context) -> Option<Value> {
    if !ctx.permit.valid() {
        return None;
    }
    // The task retains its request Permit until the owned consultation has
    // actually stopped. Dropping the HTTP future cancels authority, while
    // the task observes it and awaits group cleanup and usage accounting.
    let req = req.clone();
    tokio::spawn(REQUEST.scope(ctx, async move { handle(&req).await }))
        .await
        .ok()
        .flatten()
}

/// Tool schemas — only for teammates whose CLI actually resolves on this
/// machine. The descriptions are load-bearing: they're how Claude decides
/// when to delegate, so they spell out each teammate's sweet spot.
fn primary_agent() -> Option<AgentKind> {
    request_env("HOLLOW_PRIMARY_AGENT").and_then(|s| AgentKind::from_opt(&s))
}

fn tool_specs() -> Vec<Value> {
    let agents: Vec<_> = agent::available()
        .into_iter()
        .filter(|kind| Some(*kind) != primary_agent())
        .collect();
    if agents.is_empty() {
        return Vec::new();
    }
    let roles = agents
        .iter()
        .map(|kind| {
            let role = role_for(*kind).map(|r| r.role).unwrap_or_default();
            format!(
                "{}: {}",
                kind.as_str(),
                if role.is_empty() {
                    kind.label().to_string()
                } else {
                    role
                }
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    vec![json!({
        "name": "ask_agent",
        "description": ask_agent_description(&roles),
        "inputSchema": {
            "type": "object",
            "properties": {
                "agent": { "type": "string", "enum": agents.iter().map(|k| k.as_str()).collect::<Vec<_>>() },
                "task": { "type": "string", "description": "Full instruction and context for the teammate." },
                "cwd": { "type": "string", "description": "Optional working directory, otherwise the current workspace." },
                "packet": crate::delegation::packet_schema()
            },
            "required": ["agent", "task"]
        }
    })]
}

/// `ask_agent`'s description: the bounded-consultation contract (F10, D1),
/// when to use the durable child task instead, and the teammates.
fn ask_agent_description(roles: &str) -> String {
    format!(
        "A {} with another installed agent: one self-contained question or small task, answered in this tool call (you wait; Hollow ends it after {}s, sooner if packet.budget.seconds says so; cut off, it returns what it wrote so far, marked Incomplete). Nothing durable: no conversation in History, and the teammate gets none of Hollow's tools, profile or conversation, only its packet, your task and the working folder. For longer work, work that needs Hollow's tools, or work to keep, start a {} with hollow_app start_session instead, when you have it. Available teammates: {roles}. Include relevant context and an absolute output path when needed. Returns its text and created file paths. Teammates do not delegate further.",
        crate::delegation::CONSULTATION_NAME,
        TEAMMATE_TIMEOUT.as_secs(),
        crate::delegation::CHILD_TASK_NAME,
    )
}

async fn call_tool(id: Option<Value>, name: &str, args: &Value) -> Value {
    let env = |key: &str| request_env(key);
    // In production the budget store ignores the vault (one global file);
    // test builds key it by vault.
    let root = usage_ledger::DelegateContext::from_env(env).vault_root;
    let store = budgets::Store::for_app(root.as_deref().unwrap_or_else(|| Path::new("/")));
    call_tool_with(
        id,
        name,
        args,
        &env,
        store,
        Utc::now(),
        Teammate::installed(),
    )
    .await
}

/// A teammate run in flight ([`Teammate::run`]).
type TeammateRun<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = agent::Collected> + Send + 'a>>;

/// How a delegation resolves its teammate's model and runs it:
/// [`Teammate::installed`] in production (the saved settings and the
/// installed CLI). Tests inject a scripted shell instead, so no teammate CLI,
/// and no model, is ever reached from a test.
#[derive(Clone, Copy)]
struct Teammate {
    resolve: fn(AgentKind) -> Result<Option<String>, String>,
    /// Runs the teammate with the consultation's deadline.
    run: for<'a> fn(AgentKind, &'a RunSpec, &'a Meter, Duration) -> TeammateRun<'a>,
}

impl Teammate {
    fn installed() -> Self {
        fn resolve(kind: AgentKind) -> Result<Option<String>, String> {
            crate::agent_settings::resolve(Some(kind.as_str()), None).map(|(_, model)| model)
        }
        fn run<'a>(
            kind: AgentKind,
            spec: &'a RunSpec,
            meter: &'a Meter,
            deadline: Duration,
        ) -> TeammateRun<'a> {
            let permit = REQUEST.try_with(|ctx| ctx.permit.clone()).ok();
            Box::pin(async move {
                let Some(permit) = permit else {
                    return agent::run_collect(kind, spec, Some(deadline), None, meter).await;
                };
                let stop = crate::process_group::StopSignal::new();
                let running = agent::run_collect(kind, spec, Some(deadline), Some(&stop), meter);
                tokio::pin!(running);
                tokio::select! {
                    result = &mut running => result,
                    _ = async {
                        while permit.valid() { tokio::time::sleep(Duration::from_millis(50)).await; }
                    } => {
                        stop.request();
                        running.await
                    }
                }
            })
        }
        Teammate { resolve, run }
    }
}

/// [`call_tool`] with its environment, budget store and teammate injected
/// (tests drive the refusal and budget-stop paths without touching the
/// process env or any installed CLI).
async fn call_tool_with(
    id: Option<Value>,
    name: &str,
    args: &Value,
    env: &(dyn Fn(&str) -> Option<String> + Sync),
    store: Result<budgets::Store, String>,
    now: DateTime<Utc>,
    teammate: Teammate,
) -> Value {
    let kind = match name {
        "ask_agent" => match args
            .get("agent")
            .and_then(Value::as_str)
            .and_then(AgentKind::from_opt)
        {
            Some(kind) => kind,
            None => return tool_error(id, "Choose an installed teammate with the agent argument."),
        },
        "ask_claude" => AgentKind::Claude,
        "ask_antigravity" => AgentKind::Antigravity,
        "ask_codex" => AgentKind::Codex,
        other => return tool_error(id, &format!("unknown tool: {other}")),
    };
    // Self-hosted models Q5 review: a Claude teammate of a self-hosted chat
    // would run on the person's Claude login (or, with the chat's endpoint
    // inherited, without the run's pinned settings), so it is refused. Any
    // other teammate runs with every self-hosted variable stripped
    // (`self_hosted::strip_inherited`, in the command builder).
    if kind == AgentKind::Claude && crate::self_hosted::inherited_from(env) {
        return tool_error(id, crate::self_hosted::NO_CLAUDE_TEAMMATE);
    }
    if Some(kind) == primary_agent() {
        return tool_error(id, "The primary agent cannot delegate to itself.");
    }
    // Merged plan slice 1: the teammate is capped by its parent's mode and
    // tool limits, or refused with the reason (`delegate_grant`).
    let grant = match parent_limits(env).and_then(|limits| {
        // The parent's agent, as the server was spawned for it
        // (`delegate_env`), read through `env` like the other limits.
        let parent_agent = env("HOLLOW_PRIMARY_AGENT").and_then(|s| AgentKind::from_opt(&s));
        delegate_grant(kind, parent_agent, env(ENV_PARENT_PERM).as_deref(), &limits)
    }) {
        Ok(grant) => grant,
        Err(error) => return tool_error(id, &error),
    };
    // Paperclip PR 8b: the read-only budget preflight, before the teammate
    // is even resolved. Never writes (no pause, save, review or notice).
    let context = usage_ledger::DelegateContext::from_env(env);
    let intent = budgets::RunIntent::delegate(
        Some(kind),
        budgets::ParentRun {
            surface: context.parent_surface,
            scope: context.scope.clone(),
        },
    );
    let (budget, budget_warnings) =
        match budgets::preflight_readonly(&store, context.vault_root.as_deref(), &intent, now) {
            budgets::Preflight::Blocked(notice) => return tool_error(id, &notice.message),
            budgets::Preflight::Warn(budget, notices) => (budget, notices),
            budgets::Preflight::Clear(budget) => (budget, Vec::new()),
        };
    let model = match (teammate.resolve)(kind) {
        Ok(model) => model,
        Err(error) => return tool_error(id, &error),
    };
    // COO rebuild L7 (M4): the teammate an agent names stays inside a
    // non-empty CFO allowlist (read only; this is a helper process).
    let cfo = crate::cfo::peek_settings(context.vault_root.as_deref());
    if let Err(error) = crate::cfo::check_named(&cfo, kind, model.as_deref()) {
        return tool_error(id, &error.replace("Nothing started.", "Nothing ran."));
    }
    // The teammate's saved reasoning effort (Paperclip PR 14a), read from
    // the same app settings file as its model; an unreadable file is a tool
    // error, and an unsupported level fails the delegate's validation.
    let saved_effort = match crate::agent_settings::effort(kind) {
        Ok(effort) => effort,
        Err(error) => return tool_error(id, &error),
    };
    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if task.is_empty() {
        return tool_error(id, "missing required argument `task`");
    }
    // COO rebuild L7: Automatic asks the CFO for the task's level; Fixed
    // keeps the saved one. Read only (this is a helper process).
    let effort = crate::cfo::run_effort(&cfo, &task, crate::cfo::Origin::Subagent, saved_effort);
    // F10 (D1): the bounded consultation's task packet, checked before
    // anything runs; the host adds the asking chat's settled constraints
    // and changed files (read only, from its task state).
    let request = match crate::delegation::parse(args.get("packet")) {
        Ok(request) => request,
        Err(error) => return tool_error(id, &format!("{error} Nothing ran.")),
    };
    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_cwd);
    // Resolve to an absolute path: Antigravity resolves RELATIVE paths against
    // its own scratch workspace (not the process cwd), so a "./img.png" would
    // land outside the vault. Steering it to absolute paths under `cwd` keeps
    // artifacts in the vault where the rest of the app can see them.
    let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
    // Paperclip PR 19: never run a teammate in a system or shared root (`/`,
    // `/tmp`, bare `~`, …); the configured vault itself always passes. Before
    // the run permit, so a refused delegation never holds a slot.
    let vault_root = env("HOLLOW_VAULT_ROOT")
        .filter(|root| !root.is_empty())
        .map(PathBuf::from);
    if let Some(reason) = agent::unsafe_working_dir(&cwd, vault_root.as_deref()) {
        return tool_error(id, &agent::unsafe_cwd_error(&reason));
    }

    // Augment the task so any artifacts land in the working dir via absolute
    // paths (harmless for Codex, essential for Antigravity image output).
    let mut directive = format!(
        "[Working directory: {dir}. Save any files you create UNDER this directory \
using ABSOLUTE paths — e.g. {dir}/output.png. Do not use './' relative paths; they \
may resolve to a scratch location outside the workspace.]",
        dir = cwd.display(),
    );
    if kind == AgentKind::Antigravity {
        // Steer Antigravity to its NATIVE Gemini image model rather than
        // scripting shapes with Pillow (which is slow and not what's wanted).
        directive.push_str(
            " [For any image creation/editing, ALWAYS use your built-in generate_image \
tool (native Gemini image generation) — do NOT draw or synthesize images with Python, \
Pillow, matplotlib, SVG, or any other code. Generate with generate_image, then move/copy \
the result to the absolute path above if needed.]",
        );
    }
    // Role preset: standing instructions ride ahead of every delegation so
    // the teammate works in character without the primary restating it.
    let role_preamble = role_for(kind)
        .filter(|p| !p.role.trim().is_empty() || !p.brief.trim().is_empty())
        .map(|p| {
            let mut s = String::new();
            if !p.role.trim().is_empty() {
                s.push_str(&format!("[You are this team's {}.]", p.role.trim()));
            }
            if !p.brief.trim().is_empty() {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&format!("[Standing instructions: {}]", p.brief.trim()));
            }
            s.push_str("\n\n");
            s
        })
        .unwrap_or_default();
    let parent = match (
        context.vault_root.as_deref(),
        context.parent_surface,
        context.scope.as_deref(),
    ) {
        (Some(vault), Some(usage_ledger::Surface::Chat), Some(key)) => {
            crate::delegation::Parent::of_conversation(
                vault,
                key,
                primary_agent().map(AgentKind::as_str),
            )
        }
        _ => crate::delegation::Parent {
            agent: primary_agent().map(|kind| kind.as_str().to_string()),
            ..Default::default()
        },
    };
    let packet =
        crate::delegation::build(crate::delegation::Contract::Consultation, request, parent);
    let rendered = packet.render();
    let deadline = packet
        .deadline_seconds
        .map_or(TEAMMATE_TIMEOUT, Duration::from_secs);
    // The packet first, then the role, the task and the folder directive.
    let mut prompt = crate::delegation::prepend(
        &rendered.text,
        &format!("{role_preamble}{task}\n\n{directive}"),
    );
    // The parent run's scratch folder (Paperclip PR 15a), only when it
    // validates structurally (`TMPDIR` here is already that folder, so it
    // proves nothing). The delegate reuses it and never removes it.
    let parent_scratch = crate::scratch::inherited(env);
    if parent_scratch.is_some() {
        prompt.push_str("\n\n");
        prompt.push_str(crate::scratch::SCRATCH_GUIDANCE);
    }

    // Snapshot the cwd's top level so we can report files the teammate creates
    // (the common case: artifacts land in the working dir). Deeper paths are
    // still named in the teammate's own result text.
    let before = snapshot_files(&cwd);

    let spec = RunSpec {
        prompt,
        resume: None,
        model,
        perm: grant.perm.to_string(),
        cwd: cwd.clone(),
        allowed_tools: grant.allowed.clone(),
        disallowed_tools: grant.denied.clone(),
        interactive: false,
        // Teammates never get the team server themselves — no recursion.
        mcp_config: None,
        append_system_prompt: None,
        // This runs inside `hollow team-mcp`, so it must never create a
        // process group: the delegate shares this server's group (the CLI's
        // under Claude, the server's own under Codex) and inherits
        // HOLLOW_RUN_TAG, which is how the GUI's stop walk reaches it.
        process_scope: if REQUEST.try_with(|_| ()).is_ok() {
            agent::ProcessScope::GuiOwned
        } else {
            agent::ProcessScope::Inherited
        },
        // A Claude teammate under a token budget streams partial messages
        // for its usage (PR 8b). The dollar backstop is GUI-only.
        usage_stream: kind == AgentKind::Claude && budget.usage_stream(),
        max_budget_usd: None,
        extras: agent::RunExtras {
            effort,
            scratch: parent_scratch.map(crate::scratch::ScratchSpec::Dir),
            // OB22: the teammate's compaction window from Settings, when set.
            env: crate::agent_settings::compaction_env(kind),
            ..agent::RunExtras::default()
        },
    };

    // `run_collect` validates too; checking first keeps a spec the CLI
    // can't honour from waiting for a permit.
    if let Err(error) = agent::validate_run_spec(kind, &spec) {
        return tool_error(id, &error);
    }

    // A delegate runs inside another agent's run but never holds the vault
    // lease itself, so it takes one of the machine-wide run permits first
    // (Paperclip PR 7): a cross-process flock beside the app data, which an
    // in-process semaphore could not give this subprocess. The wait is
    // bounded; a full house returns a tool error the primary can report. The
    // permit creates no process group and signals nothing, so the delegate
    // still shares this server's group (PR 1), and it is held until the
    // delegate has ended.
    let _permit = match crate::run_permit::acquire(
        &crate::run_permit::permits_dir(),
        "this delegation",
        TEAMMATE_PERMIT_WAIT,
        None,
    )
    .await
    {
        Ok(permit) => permit,
        Err(error) => return tool_error(id, &error),
    };
    if REQUEST.try_with(|ctx| !ctx.permit.valid()).unwrap_or(false) {
        return tool_error(id, "This attempt ended while waiting; nothing ran.");
    }
    // The row goes to the vault root the GUI passed, credited to the parent
    // run; never to `HOLLOW_VAULT` (the cwd).
    let meter = Meter::delegate_from_env(env).with_budget(budget);
    // The run's manifest view lists the consultation (F10, D1), and its
    // record says how it ended (OF04); a call dropped mid-run (SIGTERM)
    // records `cancelled`.
    let consultation = crate::delegation::Consultation::start(
        context.vault_root.as_deref(),
        context.run_id.as_deref(),
        packet.record(&rendered, "ask_agent", kind.as_str(), None),
    );
    let collected = (teammate.run)(kind, &spec, &meter, deadline).await;
    let (outcome, returned_bytes) = consultation_outcome(&collected);
    consultation.finish(outcome, returned_bytes);
    // A budget stop (PR 8b): only this teammate's direct process was killed
    // and its partial usage row written; the primary is told why.
    if let Some(notice) = &collected.budget_stop {
        return tool_error(id, &budget_stop_text(&notice.message, kind));
    }
    // Levels crossed mid-run by a warn-only teammate (under chat, or a soft
    // budget) join the preflight's warnings in front of the result.
    let mut budget_warnings = budget_warnings;
    budget_warnings.extend(collected.budget_warnings);
    // The grant's note, then what a provider without Hollow's per-run MCP
    // config ran without (merged plan slice 2 item 4).
    let notes: Vec<String> = grant
        .note
        .iter()
        .cloned()
        .chain(no_cards_note(kind).map(String::from))
        .collect();
    let note = join_notes(notes.clone());
    // A timed-out consultation's results add a Claude packet budget too
    // short for most consultations (OF04); a finished answer never says it
    // may come back incomplete.
    let timeout_note = join_notes(
        notes
            .into_iter()
            .chain(short_budget_note(kind, deadline))
            .collect(),
    );
    match (collected.timed_out, collected.result) {
        (false, Ok(text)) => {
            let mut body = if text.trim().is_empty() {
                format!("{} finished (no text output).", kind.label())
            } else {
                text
            };
            if let Some(note) = &note {
                body = format!("[{note}]\n\n{body}");
            }
            body.push_str(&created_files_text(&cwd, &before));
            tool_text(id, &with_budget_warnings(&budget_warnings, body))
        }
        (false, Err(e)) => match &note {
            Some(note) => tool_error(id, &format!("{} failed: {e} [{note}]", kind.label())),
            None => tool_error(id, &format!("{} failed: {e}", kind.label())),
        },
        // OF04: a teammate cut off at its deadline returns what it wrote so
        // far, flagged first and last so it never reads as a finished answer.
        // Still a tool error: the consultation did not complete.
        (true, _) if !collected.partial_text.trim().is_empty() => {
            let body = incomplete_text(
                kind,
                deadline,
                timeout_note.as_deref(),
                &collected.partial_text,
                &created_files_text(&cwd, &before),
            );
            tool_error(id, &with_incomplete_warnings(&budget_warnings, body))
        }
        // Cut off before it wrote anything: the timeout error, as before.
        (true, _) => {
            let timed_out = format!("{} timed out after {}s", kind.label(), deadline.as_secs());
            match &timeout_note {
                Some(note) => tool_error(id, &format!("{timed_out} [{note}]")),
                None => tool_error(id, &timed_out),
            }
        }
    }
}

/// A Claude packet budget under this many seconds draws a note (OF04): 19%
/// of measured consultations timed out at 60–90 s budgets or the default.
const CLAUDE_SHORT_BUDGET_SECONDS: u64 = 90;

/// The note on a Claude consultation whose packet budget set a deadline
/// under [`CLAUDE_SHORT_BUDGET_SECONDS`] (OF04).
fn short_budget_note(kind: AgentKind, deadline: Duration) -> Option<String> {
    (kind == AgentKind::Claude && deadline.as_secs() < CLAUDE_SHORT_BUDGET_SECONDS).then(|| {
        format!(
            "packet.budget.seconds {} is short for a Claude teammate: Claude consultations often need {CLAUDE_SHORT_BUDGET_SECONDS} s or more, so the answer may come back incomplete.",
            deadline.as_secs()
        )
    })
}

/// The first line of an incomplete consultation's result (OF04).
fn incomplete_header(kind: AgentKind, deadline: Duration) -> String {
    format!(
        "Incomplete: {} was stopped at the {} s deadline; what it wrote so far is below and is not a finished answer. For longer work, continue with start_session (a {}) instead.",
        kind.label(),
        deadline.as_secs(),
        crate::delegation::CHILD_TASK_NAME,
    )
}

/// The most bytes of a cut-off teammate's text an incomplete result carries
/// (OF04), so the result stays small enough that its closing line survives
/// any client-side clipping.
const PARTIAL_TEXT_MAX: usize = 32 * 1024;

/// `partial` without trailing whitespace, capped at [`PARTIAL_TEXT_MAX`]
/// bytes by keeping its end (the newest text, and what the teammate was
/// doing when it was cut off), with how many earlier bytes were dropped.
fn capped_partial(partial: &str) -> (&str, usize) {
    let partial = partial.trim_end();
    if partial.len() <= PARTIAL_TEXT_MAX {
        return (partial, 0);
    }
    let mut start = partial.len() - PARTIAL_TEXT_MAX;
    while !partial.is_char_boundary(start) {
        start += 1;
    }
    (&partial[start..], start)
}

/// An incomplete consultation's result (OF04): the [`incomplete_header`],
/// any note, the partial text ([`capped_partial`], after an
/// `[earlier N bytes omitted]` line when capped), the created files
/// ([`created_files_text`]) and a closing line that repeats the flag.
fn incomplete_text(
    kind: AgentKind,
    deadline: Duration,
    note: Option<&str>,
    partial: &str,
    files: &str,
) -> String {
    let mut body = incomplete_header(kind, deadline);
    body.push_str("\n\n");
    if let Some(note) = note {
        body.push_str(&format!("[{note}]\n\n"));
    }
    let (kept, omitted) = capped_partial(partial);
    if omitted > 0 {
        body.push_str(&format!("[earlier {omitted} bytes omitted]\n\n"));
    }
    body.push_str(kept);
    body.push_str(files.trim_end());
    body.push_str(&format!(
        "\n\n[End of {}'s incomplete output: it was cut off at the deadline.]",
        kind.label()
    ));
    body
}

/// An incomplete result with its budget warnings after its first line, so
/// the `Incomplete:` flag always leads.
fn with_incomplete_warnings(warnings: &[budgets::BudgetNotice], body: String) -> String {
    if warnings.is_empty() {
        return body;
    }
    match body.split_once("\n\n") {
        Some((header, rest)) => {
            format!(
                "{header}\n\n{}",
                with_budget_warnings(warnings, rest.to_string())
            )
        }
        None => with_budget_warnings(warnings, body),
    }
}

/// The files the teammate created in `cwd`'s top level since `before`, as
/// a list after its result (empty when none).
fn created_files_text(cwd: &Path, before: &HashSet<String>) -> String {
    let mut created: Vec<String> = snapshot_files(cwd)
        .into_iter()
        .filter(|f| !before.contains(f))
        .collect();
    if created.is_empty() {
        return String::new();
    }
    created.sort();
    let mut text = String::from("\n\nFiles the teammate created/updated in the working dir:\n");
    for f in created {
        text.push_str(&format!("- {f}\n"));
    }
    text
}

/// How a consultation ended and how many bytes of its teammate's text it
/// returns (OF04): the answer's (trimmed) on success, what it wrote so far
/// on a timeout (as capped by [`capped_partial`]), none on a failure or a
/// budget stop.
fn consultation_outcome(collected: &agent::Collected) -> (crate::delegation::Outcome, usize) {
    use crate::delegation::Outcome;
    if collected.timed_out {
        let partial = &collected.partial_text;
        let bytes = if partial.trim().is_empty() {
            0
        } else {
            capped_partial(partial).0.len()
        };
        return (Outcome::Timeout, bytes);
    }
    if collected.stopped {
        return (Outcome::Cancelled, 0);
    }
    match (&collected.result, &collected.budget_stop) {
        (Ok(text), None) => (Outcome::Ok, text.trim().len()),
        _ => (Outcome::Error, 0),
    }
}

/// The tool error of a teammate a usage budget stopped (Paperclip PR 8b).
fn budget_stop_text(message: &str, kind: AgentKind) -> String {
    format!(
        "{message} The {} teammate was stopped; its partial usage was recorded.",
        kind.label()
    )
}

/// A delegation's result with its budget warnings (a delegate under chat
/// over a warn level or a limit) in front, so the primary sees them.
fn with_budget_warnings(warnings: &[budgets::BudgetNotice], body: String) -> String {
    if warnings.is_empty() {
        return body;
    }
    let notes: Vec<&str> = warnings.iter().map(|n| n.message.as_str()).collect();
    format!("[Budget warning: {}]\n\n{body}", notes.join(" "))
}

/// Default working directory for a delegation: the vault (passed in by the
/// spawner via `HOLLOW_VAULT`), falling back to the process cwd.
fn default_cwd() -> PathBuf {
    request_env("HOLLOW_VAULT")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Top-level file names in `dir` (non-recursive, cheap). Used to diff before/
/// after a delegation so newly-written artifacts can be surfaced to Claude.
fn snapshot_files(dir: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if entry.path().is_file() {
                out.insert(entry.file_name().to_string_lossy().to_string());
            }
        }
    }
    out
}

fn result(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } })
}

/// A successful tool result (MCP `content` convention).
fn tool_text(id: Option<Value>, text: &str) -> Value {
    result(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
    )
}

/// A tool-level failure — reported as a result with `isError: true` (not a
/// JSON-RPC error) so Claude sees the message and can react / retry.
fn tool_error(id: Option<Value>, text: &str) -> Value {
    result(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": true }),
    )
}

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

    /// F10 (D2): a consultation's wait is measured from when the asking
    /// agent sent it. The server used to answer one request at a time, so a
    /// second `ask_agent` sent while the first ran waited for the first's
    /// permit wait and deadline (210 s) before its own began: 420 s in all,
    /// past Codex's 240 s `tool_timeout_sec` for this server, which then gave
    /// up on a teammate that was still working. Two requests whose handling
    /// takes `slow` each must both be answered in about `slow`, not twice it.
    #[test]
    fn a_second_consultation_is_not_queued_behind_the_first() {
        use tokio::io::AsyncReadExt;
        let slow = std::time::Duration::from_millis(400);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let elapsed = runtime.block_on(async {
            let (client, server) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server);
            let served = serve_lines(BufReader::new(server_read), server_write, move |req: Value| async move {
                if req["method"] == json!("tools/call") {
                    tokio::time::sleep(slow).await;
                }
                req.get("id").cloned().map(|id| result(Some(id), json!({})))
            });
            let (mut client_read, mut client_write) = tokio::io::split(client);
            let ask = async {
                let started = std::time::Instant::now();
                for id in [1, 2] {
                    let line = format!(
                        "{}\n",
                        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": "ask_agent", "arguments": {} } })
                    );
                    client_write.write_all(line.as_bytes()).await.unwrap();
                }
                client_write.flush().await.unwrap();
                let mut seen = String::new();
                let mut buf = [0u8; 1024];
                while seen.matches('\n').count() < 2 {
                    let n = client_read.read(&mut buf).await.unwrap();
                    assert!(n > 0, "the server closed before answering both");
                    seen.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                let ids: HashSet<i64> = seen
                    .lines()
                    .map(|l| serde_json::from_str::<Value>(l).unwrap()["id"].as_i64().unwrap())
                    .collect();
                assert_eq!(ids, HashSet::from([1, 2]), "{seen}");
                drop(client_write);
                started.elapsed()
            };
            tokio::select! {
                elapsed = ask => elapsed,
                _ = served => panic!("the server stopped before answering"),
            }
        });
        assert!(
            elapsed < slow * 3 / 2,
            "both answered after {elapsed:?}: the second waited for the first ({slow:?} each)"
        );
    }

    /// F10 (D2): the stop sweep's SIGTERM drops the serve loop, and with it
    /// every consultation in flight, not only the first: each one's usage
    /// row then records `cancelled` (its meter's drop while terminating),
    /// and none is answered.
    #[test]
    fn a_stop_ends_every_consultation_in_flight() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;
        struct Running(Arc<AtomicUsize>);
        impl Drop for Running {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let ended = Arc::new(AtomicUsize::new(0));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let answered = runtime.block_on(async {
            let (client, server) = tokio::io::duplex(64 * 1024);
            let (server_read, server_write) = tokio::io::split(server);
            let counter = Arc::clone(&ended);
            let served = serve_lines(BufReader::new(server_read), server_write, move |req: Value| {
                let running = Running(Arc::clone(&counter));
                async move {
                    let _running = running;
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    req.get("id").cloned().map(|id| result(Some(id), json!({})))
                }
            });
            let (mut client_read, mut client_write) = tokio::io::split(client);
            for id in [1, 2] {
                let line = format!(
                    "{}\n",
                    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": "ask_agent", "arguments": {} } })
                );
                client_write.write_all(line.as_bytes()).await.unwrap();
            }
            client_write.flush().await.unwrap();
            // The stop: the loop is dropped while both are running.
            tokio::select! {
                _ = served => panic!("the server answered a 60 s consultation"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
            }
            drop(client_write);
            let mut rest = Vec::new();
            use tokio::io::AsyncReadExt;
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(200),
                client_read.read_to_end(&mut rest),
            )
            .await;
            rest
        });
        assert_eq!(ended.load(Ordering::SeqCst), 2, "both consultations ended");
        assert!(answered.is_empty(), "nothing was answered");
    }

    #[test]
    fn initialize_echoes_protocol_and_names_server() {
        let req = json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {} }
        });
        let resp = block_on(handle(&req)).unwrap();
        assert_eq!(resp["id"], json!(1));
        assert_eq!(resp["result"]["protocolVersion"], json!("2025-06-18"));
        assert_eq!(resp["result"]["serverInfo"]["name"], json!("hollow-team"));
        assert!(resp["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_returns_an_array_with_valid_schema() {
        let req = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        let resp = block_on(handle(&req)).unwrap();
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        // Every advertised tool has the required MCP shape.
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert_eq!(t["inputSchema"]["type"], json!("object"));
            assert!(t["inputSchema"]["properties"]["task"].is_object());
        }
        // Tool names, when present, are from the known set.
        for t in tools {
            let n = t["name"].as_str().unwrap();
            assert!(n == "ask_agent", "unexpected tool {n}");
            // F10 (D1): it names its contract and takes a task packet.
            let about = t["description"].as_str().unwrap();
            assert!(about.contains("bounded consultation"), "{about}");
            assert_eq!(
                t["inputSchema"]["properties"]["packet"],
                crate::delegation::packet_schema()
            );
        }
    }

    /// F10 (D1): the description names the bounded consultation, its
    /// deadline and what it lacks, and points longer work to the durable
    /// child task, whatever teammates are installed.
    #[test]
    fn ask_agent_names_its_contract_and_the_other_one() {
        let about = ask_agent_description("codex: Engineer");
        assert!(about.starts_with("A bounded consultation with another installed agent"));
        assert!(about.contains("Hollow ends it after 180s"), "{about}");
        assert!(
            about.contains("what it wrote so far, marked Incomplete"),
            "{about}"
        );
        assert!(about.contains("durable child task with hollow_app start_session"));
        assert!(about.contains("none of Hollow's tools, profile or conversation"));
        assert!(about.contains("Available teammates: codex: Engineer."));
        let schema = crate::delegation::packet_schema();
        for field in ["constraints", "sources", "expected", "budget"] {
            assert!(schema["properties"][field].is_object(), "{field}");
        }
    }

    #[test]
    fn notification_gets_no_response_but_unknown_request_errors() {
        // Notification (no id) → no reply.
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(block_on(handle(&notif)).is_none());
        // Unknown method with an id → JSON-RPC method-not-found.
        let bad = json!({ "jsonrpc": "2.0", "id": 9, "method": "does/not/exist" });
        let resp = block_on(handle(&bad)).unwrap();
        assert_eq!(resp["error"]["code"], json!(-32601));
    }

    #[test]
    fn role_presets_round_trip_via_setter() {
        set_role("codex", "Engineer".into(), "Prefer small diffs.".into());
        let p = role_for(AgentKind::Codex).expect("preset stored");
        assert_eq!(p.role, "Engineer");
        assert_eq!(p.brief, "Prefer small diffs.");
        // Claude can also be a teammate when another adapter is primary.
        set_role("claude", "Boss".into(), String::new());
        assert_eq!(role_for(AgentKind::Claude).unwrap().role, "Boss");
    }

    #[test]
    fn unknown_tool_is_a_tool_error_not_a_crash() {
        let req = json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "ask_nobody", "arguments": { "task": "hi" } }
        });
        let resp = block_on(handle(&req)).unwrap();
        assert_eq!(resp["result"]["isError"], json!(true));
        assert!(resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }

    #[test]
    fn delegate_env_separates_vault_root_from_cwd_and_names_the_parent() {
        let parent = DelegateParent {
            surface: usage_ledger::Surface::Chat,
            scope: "s-1".into(),
            run_id: "R9".into(),
            scratch: None,
            tool_limits: ToolLimits::new(&["Read".into()], &["Bash(osascript:*)".into()]),
        };
        let env = delegate_env(
            Path::new("/tmp/x"),
            Path::new("/v"),
            AgentKind::Codex,
            &parent,
            "acceptEdits",
        );
        assert!(
            !env.contains_key(crate::scratch::SCRATCH_ENV),
            "no scratch, no key"
        );
        assert_eq!(env["HOLLOW_VAULT"], json!("/tmp/x"));
        assert_eq!(env["HOLLOW_VAULT_ROOT"], json!("/v"));
        assert_eq!(env["HOLLOW_RUN_SURFACE"], json!("chat"));
        assert_eq!(env["HOLLOW_RUN_SCOPE"], json!("s-1"));
        assert_eq!(env["HOLLOW_RUN_ID"], json!("R9"));
        assert_eq!(env["HOLLOW_PRIMARY_AGENT"], json!("codex"));
        for key in env.keys() {
            for word in ["KEY", "SECRET", "TOKEN"] {
                assert!(!key.contains(word), "Codex would strip {key}");
            }
        }
        // The delegate's meter reads the same encoding back.
        let context = usage_ledger::DelegateContext::from_env(|key| {
            env.get(key).and_then(Value::as_str).map(str::to_string)
        });
        assert_eq!(context.vault_root.as_deref(), Some(Path::new("/v")));
        assert_eq!(context.parent_surface, Some(usage_ledger::Surface::Chat));
        assert_eq!(context.scope.as_deref(), Some("s-1"));
        assert_eq!(context.run_id.as_deref(), Some("R9"));
        assert_eq!(
            env[ENV_PARENT_PERM],
            json!("acceptEdits"),
            "the parent's mode rides along"
        );
        // So do its tool limits, read back exactly.
        let limits =
            parent_limits(&|key: &str| env.get(key).and_then(Value::as_str).map(str::to_string))
                .unwrap();
        assert_eq!(limits, parent.tool_limits);
        assert_eq!(env[ENV_PARENT_DENIED], json!(r#"["Bash(osascript:*)"]"#));
    }

    // ── Merged plan slice 1: the delegation cap ─────────────────────────

    /// What a command can actually do, read from its argv, for the harmless
    /// forbidden actions this cap is checked against. Mode names are not
    /// compared: two providers' "acceptEdits" are different sandboxes. "May"
    /// counts as can (an auto reviewer or classifier may approve).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Effective {
        /// Edit a file in its working folder without asking.
        write_in_cwd: bool,
        /// Write a file outside its working folder.
        write_outside: bool,
        /// Drive the desktop with `osascript`/`open`, around Hollow's verbs.
        desktop: bool,
        /// Use a tool the parent's allowed list leaves out.
        unlisted_tool: bool,
    }

    fn argv(cmd: &tokio::process::Command) -> Vec<String> {
        cmd.as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    /// Capabilities per provider, from the flags that grant them. Claude:
    /// headless `-p` denies whatever its mode would prompt for
    /// (fixtures/interactions/README.md), so only acceptEdits and up edit,
    /// and only auto and up run Bash or write outside the folder. Grok: its
    /// headless `-p` ran every command in every mode (probed 2026-10-02, plan
    /// included), so only its `--sandbox` profile and `--deny` rules limit
    /// it, whatever the mode says. Codex and Antigravity: their sandboxes, as
    /// probed (`sandbox_blocks_desktop`).
    fn effective(kind: AgentKind, args: &[String]) -> Effective {
        let has = |flag: &str| args.iter().any(|a| a == flag);
        match kind {
            AgentKind::Claude => {
                let mode = after(args, "--permission-mode").expect("a permission mode");
                let edits = matches!(mode, "acceptEdits" | "auto" | "bypassPermissions");
                let free = matches!(mode, "auto" | "bypassPermissions");
                let denies_desktop = has("Bash(osascript:*)") && has("Bash(open:*)");
                Effective {
                    write_in_cwd: edits,
                    write_outside: free,
                    desktop: free && !denies_desktop,
                    unlisted_tool: !has("--tools"),
                }
            }
            AgentKind::Grok => {
                assert!(
                    after(args, "--permission-mode").is_some(),
                    "a permission mode"
                );
                // Each rule must follow its own `--deny`.
                let denies_desktop = crate::computer_mcp::BASH_DENY
                    .iter()
                    .all(|rule| args.windows(2).any(|w| w[0] == "--deny" && w[1] == *rule));
                let (write_in_cwd, write_outside, desktop) = match after(args, "--sandbox") {
                    Some("read-only") => (false, false, false),
                    Some("workspace") => (true, false, false),
                    Some("off") => (true, true, !denies_desktop),
                    // Without a flag a resumed session keeps whatever profile
                    // it started with, so every Grok run must name one.
                    None => panic!("a Grok run without --sandbox: {args:?}"),
                    Some(other) => panic!("unprobed Grok sandbox profile {other}"),
                };
                Effective {
                    write_in_cwd,
                    write_outside,
                    desktop,
                    unlisted_tool: true,
                }
            }
            AgentKind::Codex => {
                if has("--dangerously-bypass-approvals-and-sandbox") {
                    return Effective {
                        write_in_cwd: true,
                        write_outside: true,
                        desktop: true,
                        unlisted_tool: true,
                    };
                }
                // A fresh run names its sandbox with `-s`; `exec resume`
                // takes none and reads the `sandbox_mode` key instead.
                let resumed = args
                    .iter()
                    .find_map(|a| a.strip_prefix("sandbox_mode=\"")?.strip_suffix('"'));
                // A permission profile (auto's, or an edit mode's nested-
                // `.git` deny) is `:workspace`: it writes the cwd.
                let profile = args.iter().any(|a| a.starts_with("default_permissions="));
                let write_in_cwd = match after(args, "-s").or(resumed) {
                    Some("read-only") => false,
                    Some("workspace-write") => true,
                    None if profile => true,
                    None => panic!("a Codex run without a sandbox: {args:?}"),
                    Some(other) => panic!("unprobed Codex sandbox {other}"),
                };
                let reviewer = args.iter().any(|a| a.starts_with("approvals_reviewer="));
                Effective {
                    write_in_cwd,
                    write_outside: reviewer,
                    desktop: reviewer,
                    unlisted_tool: true,
                }
            }
            AgentKind::Antigravity => {
                if !has("--sandbox") {
                    return Effective {
                        write_in_cwd: true,
                        write_outside: true,
                        desktop: true,
                        unlisted_tool: true,
                    };
                }
                // Headless accept-edits and plan deny every write and command.
                let skip = has("--dangerously-skip-permissions");
                // The sandbox let a write into ~/Library/Caches through
                // (docs/history/BUILD_LOG-2026-10-01-to-09.md, 2026-10-02), so outside writes count as may.
                Effective {
                    write_in_cwd: skip,
                    write_outside: skip,
                    desktop: false,
                    unlisted_tool: true,
                }
            }
        }
    }

    /// `child` can do nothing `parent` can't.
    fn within(child: Effective, parent: Effective) -> bool {
        (!child.write_in_cwd || parent.write_in_cwd)
            && (!child.write_outside || parent.write_outside)
            && (!child.desktop || parent.desktop)
            && (!child.unlisted_tool || parent.unlisted_tool)
    }

    /// `RunSpec` by another name, so `agent`'s source count of spec literals
    /// against inherited process scopes still sees one per literal.
    type Spec = RunSpec;

    fn run_spec(
        perm: &str,
        resume: Option<&str>,
        allowed: &[String],
        denied: &[String],
        mcp: Option<String>,
    ) -> Spec {
        RunSpec {
            prompt: "p".into(),
            resume: resume.map(str::to_string),
            model: None,
            perm: perm.into(),
            cwd: PathBuf::from("/tmp/ws"),
            allowed_tools: allowed.to_vec(),
            disallowed_tools: denied.to_vec(),
            interactive: true,
            mcp_config: mcp,
            append_system_prompt: None,
            process_scope: agent::ProcessScope::Inherited,
            usage_stream: false,
            max_budget_usd: None,
            extras: agent::RunExtras::default(),
        }
    }

    /// The team server's environment as the parent CLI is handed it, read
    /// back out of the private config file the parent's argv names (merged
    /// plan slice 6), which is removed once read. Claude names it with
    /// `--mcp-config`; Codex hands it to each server it starts through
    /// `hollow mcp-launch <file> …` (`agent::CodexLaunch`).
    fn team_env_from_argv(kind: AgentKind, args: &[String]) -> serde_json::Map<String, Value> {
        let file = match kind {
            AgentKind::Claude => after(args, "--mcp-config").expect("an MCP config"),
            AgentKind::Codex => {
                let marker = format!("[\"{}\",\"", crate::run_auth::LAUNCH_SUBCOMMAND);
                let arg = args
                    .iter()
                    .find(|a| a.contains(&marker))
                    .expect("a launched server");
                let start = arg.find(&marker).unwrap() + marker.len();
                &arg[start..start + arg[start..].find('"').unwrap()]
            }
            other => panic!("{other:?} takes no Hollow MCP config"),
        };
        let config = crate::run_auth::config_from_arg(file).expect("a readable MCP config");
        let _ = std::fs::remove_file(file);
        config["mcpServers"]["hollow_team"]["env"]
            .as_object()
            .unwrap()
            .clone()
    }

    /// Reviews A·S1 and B·F03 (merged plan slice 1). For every parent mode
    /// a run can delegate from × every provider × the parent's tool limits ×
    /// a fresh and a resumed parent, the teammate's command, built by the
    /// real builders from what the parent's argv carries, can do nothing
    /// the parent's can't, or the delegation is refused. Before this, an
    /// Antigravity teammate of an accept-edits chat ran with
    /// `--dangerously-skip-permissions` and no sandbox. A Claude parent here;
    /// a Codex one in the next test.
    #[test]
    fn a_teammate_can_do_nothing_its_parent_cannot() {
        let desktop: Vec<String> = crate::computer_mcp::BASH_DENY
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut chat_denied: Vec<String> = vec!["SendUserFile".into(), "AskUserQuestion".into()];
        chat_denied.extend(desktop.clone());
        let listed: Vec<String> = vec!["Read".into(), "Write".into(), "Edit".into()];
        let limit_sets: Vec<(&str, ToolLimits)> = vec![
            ("none", ToolLimits::default()),
            (
                "chat + computer control",
                ToolLimits::new(&[], &chat_denied),
            ),
            ("workflow allow list", ToolLimits::new(&listed, &[])),
            ("allow + deny", ToolLimits::new(&listed, &["Write".into()])),
        ];
        let (ran, refused) = cap_holds(
            AgentKind::Claude,
            &["default", "acceptEdits", "auto", "bypassPermissions"],
            &limit_sets,
        );
        assert!(
            ran > 0 && refused > 0,
            "both outcomes are exercised ({ran} ran, {refused} refused)"
        );
    }

    /// H13 (2026-10-15): the same cap under a Codex parent, in each sandbox
    /// and approval setting a Codex run takes: `plan` (`-s read-only`),
    /// `default` and `acceptEdits` (`workspace-write`, approvals never),
    /// `auto` (`workspace-write` and the auto reviewer) and bypass (no
    /// sandbox), fresh and resumed (`sandbox_mode`). The case that looks
    /// like an escape is `default`: a Codex parent there writes its folder
    /// (today; PROVIDER_MATRIX item 5 is open, and the test holds either
    /// way), and its teammates get less (a Codex one runs read-only, a Claude one
    /// asks, which headless denies). A `plan` parent never gets the team
    /// server (`mcp_attach`) and refuses every teammate. A Codex run can't
    /// hold an allowed list or any deny but the inert host-only ones
    /// (`agent::validate_run_spec`; chat adds the computer-control pair for
    /// Claude only), so those are the only limits it hands down.
    #[test]
    fn a_teammate_of_a_codex_parent_can_do_nothing_it_cannot() {
        let host_only: Vec<String> = vec!["SendUserFile".into(), "AskUserQuestion".into()];
        let limit_sets: Vec<(&str, ToolLimits)> = vec![
            ("none", ToolLimits::default()),
            ("chat", ToolLimits::new(&[], &host_only)),
        ];
        let perms = [
            "plan",
            "default",
            "acceptEdits",
            "auto",
            "bypassPermissions",
        ];
        let (ran, refused) = cap_holds(AgentKind::Codex, &perms, &limit_sets);
        // Per limit set × fresh/resumed: four teammates at each of the four
        // modes that delegate (Antigravity's auto is back, 2026-11-20), ran;
        // plan's four refused. The 64 grants include 16
        // Codex→Codex ones, which `call_tool_with` refuses first in the product.
        assert_eq!((ran, refused), (64, 16));
        // The limits a Codex parent never runs under, so never hands down.
        let desktop: Vec<String> = crate::computer_mcp::BASH_DENY
            .iter()
            .map(|s| s.to_string())
            .collect();
        let desktop_deny = ToolLimits::new(&[], &desktop);
        let allow_list = ToolLimits::new(&["Read".into()], &[]);
        for limits in [desktop_deny, allow_list] {
            let spec = run_spec("auto", None, &limits.allowed, &limits.denied, None);
            assert!(
                agent::validate_run_spec(AgentKind::Codex, &spec).is_err(),
                "{limits:?}"
            );
        }
    }

    /// For every parent mode in `perms` × every provider × each of
    /// `limit_sets` × a fresh and a resumed `parent` run, the teammate's
    /// command, built by the real builders from the mode and limits the
    /// parent's argv hands its team server, can do nothing the parent's
    /// command can't, or the delegation is refused. Every provider includes
    /// the parent's own, which `call_tool_with` refuses first. Returns how
    /// many delegations ran and how many were refused.
    fn cap_holds(
        parent: AgentKind,
        perms: &[&str],
        limit_sets: &[(&str, ToolLimits)],
    ) -> (usize, usize) {
        // A resumed Claude takes `--resume`, a resumed Codex `exec resume`.
        let resume_flag = if parent == AgentKind::Claude {
            "--resume"
        } else {
            "resume"
        };
        let mut refused = 0;
        let mut ran = 0;
        for &perm in perms {
            for (label, limits) in limit_sets {
                for resume in [None, Some("sid-parent")] {
                    let parent_info = DelegateParent {
                        surface: usage_ledger::Surface::Chat,
                        scope: "s".into(),
                        run_id: "R".into(),
                        scratch: None,
                        tool_limits: limits.clone(),
                    };
                    let team = delegate_env(
                        Path::new("/tmp/ws"),
                        Path::new("/v"),
                        parent,
                        &parent_info,
                        perm,
                    );
                    let mcp = json!({ "mcpServers": { "hollow_team": { "command": "/x/hollow", "args": ["team-mcp"], "env": team } } }).to_string();
                    let parent_spec =
                        run_spec(perm, resume, &limits.allowed, &limits.denied, Some(mcp));
                    agent::validate_run_spec(parent, &parent_spec)
                        .unwrap_or_else(|e| panic!("{perm} {label}: the parent runs: {e}"));
                    let parent_args =
                        argv(&agent::command_without_codex_listing(parent, &parent_spec));
                    assert_eq!(
                        resume.is_some(),
                        parent_args.iter().any(|a| a == resume_flag),
                        "{perm} {label}"
                    );
                    let parent_can = effective(parent, &parent_args);
                    let env_map = team_env_from_argv(parent, &parent_args);
                    let env =
                        |key: &str| env_map.get(key).and_then(Value::as_str).map(str::to_string);
                    for kind in AgentKind::all() {
                        let case = format!(
                            "{} parent {perm} × {} × {label} × {}",
                            parent.as_str(),
                            kind.as_str(),
                            if resume.is_some() { "resumed" } else { "fresh" }
                        );
                        let grant = match parent_limits(&env).and_then(|l| {
                            delegate_grant(kind, Some(parent), env(ENV_PARENT_PERM).as_deref(), &l)
                        }) {
                            Ok(grant) => grant,
                            Err(error) => {
                                // A plan parent's refusal is about the run.
                                assert!(
                                    if perm == "plan" {
                                        error.starts_with("A read-only (plan) run can't delegate")
                                    } else {
                                        error.contains(kind.label())
                                    },
                                    "{case}: a refusal names the teammate or the plan run: {error}"
                                );
                                refused += 1;
                                continue;
                            }
                        };
                        // A teammate never resumes; it's built like team-mcp builds it.
                        let child = run_spec(grant.perm, None, &grant.allowed, &grant.denied, None);
                        agent::validate_run_spec(kind, &child)
                            .unwrap_or_else(|e| panic!("{case}: {e}"));
                        let child_args = argv(&agent::command_without_codex_listing(kind, &child));
                        let child_can = effective(kind, &child_args);
                        assert!(within(child_can, parent_can), "{case}: teammate {child_can:?} exceeds parent {parent_can:?} ({child_args:?})");
                        ran += 1;
                    }
                }
            }
        }
        (ran, refused)
    }

    /// The exact grants behind that property, including the cases that
    /// used to escalate and the ones that keep teamwork working.
    #[test]
    fn delegate_grants_follow_the_parent() {
        let none = ToolLimits::default();
        let grant = |kind, perm: &str, limits: &ToolLimits| {
            delegate_grant(kind, Some(AgentKind::Claude), Some(perm), limits)
        };
        // A Claude parent's mode, carried by `agent::inherited_mode` (Q-R3
        // round 2): never more on either axis. Claude's acceptEdits runs no
        // commands, so Codex reads only and Grok runs its read-only sandbox;
        // auto gives Grok acceptEdits (its auto has the sandbox off), and
        // Antigravity can't run auto (merged plan slice 2, checked below).
        for (kind, perm, want) in [
            (AgentKind::Claude, "acceptEdits", "acceptEdits"),
            (AgentKind::Codex, "acceptEdits", "plan"),
            (AgentKind::Grok, "acceptEdits", "default"),
            (AgentKind::Antigravity, "acceptEdits", "acceptEdits"),
            (AgentKind::Claude, "auto", "auto"),
            (AgentKind::Codex, "auto", "auto"),
            (AgentKind::Grok, "auto", "acceptEdits"),
            (AgentKind::Claude, "bypassPermissions", "bypassPermissions"),
            (AgentKind::Codex, "bypassPermissions", "bypassPermissions"),
            (AgentKind::Grok, "bypassPermissions", "bypassPermissions"),
            (
                AgentKind::Antigravity,
                "bypassPermissions",
                "bypassPermissions",
            ),
        ] {
            let got = grant(kind, perm, &none).unwrap();
            assert_eq!(got.perm, want, "{kind:?} {perm}");
            assert_eq!(
                got.note
                    .is_some_and(|n| n.contains("would do more than this run")),
                want != perm,
                "{kind:?} {perm}"
            );
        }
        // The review's case: a Codex auto parent consulting Grok never gets
        // Grok's auto (sandbox off); its own Codex keeps auto.
        let codex_auto =
            delegate_grant(AgentKind::Grok, Some(AgentKind::Codex), Some("auto"), &none).unwrap();
        assert_eq!(codex_auto.perm, "acceptEdits");
        assert_eq!(
            delegate_grant(
                AgentKind::Claude,
                Some(AgentKind::Codex),
                Some("default"),
                &none
            )
            .unwrap()
            .perm,
            "acceptEdits",
            "Codex's default writes the folder; Claude's acceptEdits does no more"
        );
        // An unknown parent agent reads the most restrictive way.
        assert_eq!(
            delegate_grant(AgentKind::Claude, None, Some("acceptEdits"), &none)
                .unwrap()
                .perm,
            "default"
        );
        // An auto parent's Antigravity runs its own auto (2026-11-20), with or
        // without limits: never bypass.
        let desktop: Vec<String> = crate::computer_mcp::BASH_DENY
            .iter()
            .map(|s| s.to_string())
            .collect();
        for limits in [none.clone(), ToolLimits::new(&[], &desktop)] {
            if let Ok(granted) = grant(AgentKind::Antigravity, "auto", &limits) {
                assert_ne!(granted.perm, "bypassPermissions");
            }
        }
        assert_eq!(
            grant(AgentKind::Claude, "default", &none).unwrap().perm,
            "default"
        );
        assert_eq!(
            grant(AgentKind::Codex, "default", &none).unwrap().perm,
            "plan",
            "Codex can't ask before edits"
        );
        // Fail closed: no mode, an unknown one, or plan.
        for perm in [None, Some(""), Some("yolo"), Some("plan")] {
            assert!(
                delegate_grant(AgentKind::Codex, Some(AgentKind::Claude), perm, &none).is_err(),
                "{perm:?}"
            );
        }
        // Accept-edits Antigravity is sandboxed and says so.
        let agy = grant(AgentKind::Antigravity, "acceptEdits", &none).unwrap();
        let note = agy.note.unwrap();
        assert!(
            note.contains("only in Bypass all") && !note.contains("Switch the chat to Auto"),
            "{note}"
        );

        // Allowed lists pass to Claude with denies removed; deny wins.
        let limits = ToolLimits::new(
            &["Read".into(), "Write".into(), " Read ".into()],
            &["Write".into()],
        );
        let claude = grant(AgentKind::Claude, "acceptEdits", &limits).unwrap();
        assert_eq!(claude.allowed, vec!["Read".to_string()]);
        assert_eq!(claude.denied, vec!["Write".to_string()]);
        let emptied = ToolLimits::new(&["Write".into()], &["Write".into()]);
        assert!(
            grant(AgentKind::Claude, "acceptEdits", &emptied).is_err(),
            "an emptied list is not unrestricted"
        );
        for kind in [AgentKind::Codex, AgentKind::Antigravity, AgentKind::Grok] {
            assert!(
                grant(kind, "bypassPermissions", &limits).is_err(),
                "{kind:?} can't hold a tool list"
            );
            assert!(
                grant(kind, "auto", &ToolLimits::new(&[], &["WebFetch".into()])).is_err(),
                "{kind:?}"
            );
        }

        // The computer-control deny: kept by a sandbox, narrowed to one, or refused.
        let deny = ToolLimits::new(&[], &desktop);
        assert_eq!(
            grant(AgentKind::Claude, "bypassPermissions", &deny)
                .unwrap()
                .denied,
            desktop
        );
        // A Codex parent's own acceptEdits: its sandbox holds the deny.
        let codex = delegate_grant(
            AgentKind::Codex,
            Some(AgentKind::Codex),
            Some("acceptEdits"),
            &deny,
        )
        .unwrap();
        assert_eq!((codex.perm, codex.note.is_none()), ("acceptEdits", true));
        for perm in ["auto", "bypassPermissions"] {
            let codex = grant(AgentKind::Codex, perm, &deny).unwrap();
            assert_eq!(codex.perm, "acceptEdits", "{perm}");
            assert!(codex.note.unwrap().contains("osascript"));
        }
        // Antigravity can't hold a deny list as a flag, so its auto reads like
        // its bypass here: never kept as auto, never wider.
        if let Ok(agy_auto) = grant(AgentKind::Antigravity, "auto", &deny) {
            assert!(!matches!(agy_auto.perm, "auto" | "bypassPermissions"));
        }
        let agy = grant(AgentKind::Antigravity, "bypassPermissions", &deny).unwrap();
        assert_eq!(
            agy.perm, "acceptEdits",
            "narrowed to its sandbox, never to auto"
        );
        assert!(agy.note.unwrap().contains("osascript"));
        // Grok keeps a Grok parent's mode and takes the pair as `--deny`
        // (slice 2 probe); its other denies are still unprobed and refuse.
        for perm in ["default", "acceptEdits", "auto", "bypassPermissions"] {
            let grok =
                delegate_grant(AgentKind::Grok, Some(AgentKind::Grok), Some(perm), &deny).unwrap();
            assert_eq!(
                (grok.perm, grok.note, &grok.denied),
                (perm, None, &desktop),
                "{perm}"
            );
        }
        assert!(grant(
            AgentKind::Grok,
            "auto",
            &ToolLimits::new(&[], &["Bash(rm:*)".into()])
        )
        .is_err());
        // Host-only exclusions are inert everywhere.
        let host = ToolLimits::new(&[], &["SendUserFile".into(), "AskUserQuestion".into()]);
        assert_eq!(
            grant(AgentKind::Grok, "auto", &host).unwrap().perm,
            "acceptEdits",
            "a Claude auto parent's Grok: its auto has the sandbox off"
        );

        // A list present but unreadable refuses; an absent one is empty.
        assert!(parent_limits(
            &|key: &str| (key == ENV_PARENT_DENIED).then(|| "not json".to_string())
        )
        .is_err());
        assert_eq!(
            parent_limits(&|_: &str| None).unwrap(),
            ToolLimits::default()
        );
    }

    /// Self-hosted models Q5 review: which teammates ran for
    /// [`a_self_hosted_chat_never_asks_a_claude_teammate`].
    static SELF_HOSTED_RAN: std::sync::Mutex<Vec<AgentKind>> = std::sync::Mutex::new(Vec::new());

    fn self_hosted_teammate<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
    ) -> TeammateRun<'a> {
        SELF_HOSTED_RAN.lock().unwrap().push(kind);
        scripted_oneshot(kind, spec, meter, deadline)
    }

    /// Self-hosted models Q5 review: inside a self-hosted chat's run (its
    /// marker, or a run key as its Anthropic token), a Claude teammate is
    /// refused with why and never runs; another teammate still runs (its
    /// command loses the self-hosted variables, `self_hosted_tests`).
    #[test]
    fn a_self_hosted_chat_never_asks_a_claude_teammate() {
        let dir = budget_vault();
        let v = dir.path();
        let ask = |agent_name: &str, extra: (&str, &str)| {
            let mut env_map = parent_env(v, "chat");
            env_map.insert(ENV_PARENT_PERM.into(), "acceptEdits".into());
            env_map.insert(extra.0.into(), extra.1.into());
            let env = |key: &str| env_map.get(key).cloned();
            let response = block_on(async {
                Some(
                    call_tool_with(
                        Some(json!(7)),
                        "ask_agent",
                        &json!({ "agent": agent_name, "task": "check", "cwd": v.to_string_lossy() }),
                        &env,
                        budgets::Store::for_app(v),
                        Utc::now(),
                        Teammate { resolve: resolve_default, run: self_hosted_teammate },
                    )
                    .await,
                )
            })
            .unwrap();
            tool_text_of(&response)
        };
        SELF_HOSTED_RAN.lock().unwrap().clear();
        for extra in [
            (crate::self_hosted::MARKER_ENV, "qwen"),
            (
                crate::self_hosted::AUTH_TOKEN_ENV,
                "hsr_0123456789abcdef0123456789abcdef",
            ),
        ] {
            let (is_error, text) = ask("claude", extra);
            assert!(is_error, "{text}");
            assert_eq!(text, crate::self_hosted::NO_CLAUDE_TEAMMATE);
        }
        assert!(SELF_HOSTED_RAN.lock().unwrap().is_empty(), "no Claude ran");
        let (is_error, text) = ask("codex", (crate::self_hosted::MARKER_ENV, "qwen"));
        assert!(!is_error, "{text}");
        assert_eq!(*SELF_HOSTED_RAN.lock().unwrap(), vec![AgentKind::Codex]);
    }

    /// What [`spec_capturing_teammate`] saw: the spec's mode and tool lists.
    static CAPTURED_SPECS: std::sync::Mutex<Vec<(String, Vec<String>, Vec<String>)>> =
        std::sync::Mutex::new(Vec::new());

    fn spec_capturing_teammate<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
    ) -> TeammateRun<'a> {
        CAPTURED_SPECS.lock().unwrap().push((
            spec.perm.clone(),
            spec.allowed_tools.clone(),
            spec.disallowed_tools.clone(),
        ));
        scripted_oneshot(kind, spec, meter, deadline)
    }

    /// The grant reaches the spec `call_tool_with` runs, its note reaches
    /// the primary, and a refusal never runs the teammate.
    #[test]
    fn call_tool_runs_the_teammate_at_its_grant_or_not_at_all() {
        let dir = budget_vault();
        let v = dir.path();
        let call = |agent_name: &str, perm: Option<&str>, denied: &str| {
            let mut env_map = parent_env(v, "chat");
            match perm {
                Some(perm) => env_map.insert(ENV_PARENT_PERM.into(), perm.into()),
                None => env_map.remove(ENV_PARENT_PERM),
            };
            env_map.insert(ENV_PARENT_DENIED.into(), denied.into());
            let env = |key: &str| env_map.get(key).cloned();
            CAPTURED_SPECS.lock().unwrap().clear();
            let response =
                block_on(async {
                    Some(
                    call_tool_with(
                        Some(json!(4)),
                        "ask_agent",
                        &json!({ "agent": agent_name, "task": "draw", "cwd": v.to_string_lossy() }),
                        &env,
                        budgets::Store::for_app(v),
                        Utc::now(),
                        Teammate { resolve: resolve_default, run: spec_capturing_teammate },
                    )
                    .await,
                )
                })
                .unwrap();
            let seen = CAPTURED_SPECS.lock().unwrap().clone();
            (tool_text_of(&response), seen)
        };
        let ((is_error, text), seen) = call("antigravity", Some("acceptEdits"), "[]");
        assert!(!is_error, "{text}");
        assert_eq!(
            seen,
            vec![("acceptEdits".to_string(), vec![], vec![])],
            "not bypass"
        );
        assert!(
            text.starts_with("[Antigravity ran in its sandbox"),
            "{text}"
        );

        let deny = r#"["SendUserFile","Bash(osascript:*)","Bash(open:*)"]"#;
        let ((is_error, text), seen) = call("codex", Some("bypassPermissions"), deny);
        assert!(!is_error, "{text}");
        assert_eq!(
            seen,
            vec![(
                "acceptEdits".to_string(),
                vec![],
                vec!["SendUserFile".to_string()]
            )]
        );
        let ((is_error, _), seen) = call("claude", Some("auto"), deny);
        assert!(!is_error);
        assert_eq!(seen[0].2.len(), 3, "Claude gets every deny as a flag");

        // Q-R3 round 2: an auto parent's Grok runs acceptEdits (its auto
        // has the sandbox off), and still holds the deny.
        let ((is_error, text), seen) = call("grok", Some("auto"), deny);
        assert!(!is_error, "{text}");
        assert!(text.contains("Grok ran in acceptEdits"), "{text}");
        assert_eq!(
            seen,
            vec![(
                "acceptEdits".to_string(),
                vec![],
                serde_json::from_str::<Vec<String>>(deny).unwrap()
            )],
            "Grok holds the deny"
        );

        for (agent_name, perm, denied) in [
            ("codex", None, "[]"),
            ("grok", Some("auto"), r#"["WebFetch"]"#),
            ("codex", Some("auto"), "{"),
        ] {
            let ((is_error, text), seen) = call(agent_name, perm, denied);
            assert!(
                is_error && seen.is_empty(),
                "{agent_name} {perm:?} {denied}: {text}"
            );
            assert!(text.starts_with("Hollow didn't start"), "{text}");
        }
    }

    /// Merged plan slice 2 item 4: a Grok or Antigravity teammate's result
    /// says it ran without Hollow's cards and app tools (Grok's also that its
    /// own `~/.grok` MCP servers still load), after any grant note; a Claude
    /// or Codex teammate's result carries no such note.
    #[test]
    fn a_teammate_without_owned_mcp_says_hollow_cards_are_absent() {
        let dir = budget_vault();
        let v = dir.path();
        let call = |agent_name: &str, perm: &str| {
            let mut env_map = parent_env(v, "chat");
            env_map.insert(ENV_PARENT_PERM.into(), perm.into());
            env_map.insert(ENV_PARENT_DENIED.into(), "[]".into());
            let env = |key: &str| env_map.get(key).cloned();
            let response =
                block_on(async {
                    Some(
                    call_tool_with(
                        Some(json!(5)),
                        "ask_agent",
                        &json!({ "agent": agent_name, "task": "draw", "cwd": v.to_string_lossy() }),
                        &env,
                        budgets::Store::for_app(v),
                        Utc::now(),
                        Teammate { resolve: resolve_default, run: scripted_oneshot },
                    )
                    .await,
                )
                })
                .unwrap();
            tool_text_of(&response)
        };
        let (is_error, text) = call("grok", "bypassPermissions");
        assert!(!is_error, "{text}");
        assert!(
            text.starts_with("[Grok can't take Hollow's per-run tool setup"),
            "{text}"
        );
        assert!(
            text.contains("confirm or question cards") && text.contains("~/.grok still load"),
            "{text}"
        );

        let (is_error, text) = call("antigravity", "acceptEdits");
        assert!(!is_error, "{text}");
        assert!(
            text.starts_with("[Antigravity ran in its sandbox"),
            "the grant note leads: {text}"
        );
        assert!(
            text.contains("Antigravity can't take Hollow's per-run tool setup"),
            "{text}"
        );
        assert!(!text.contains("~/.grok"), "{text}");

        for (agent_name, perm) in [("claude", "auto"), ("codex", "bypassPermissions")] {
            let (is_error, text) = call(agent_name, perm);
            assert!(!is_error, "{agent_name}: {text}");
            assert!(!text.contains("per-run tool setup"), "{agent_name}: {text}");
        }
    }

    /// A teammate whose time bound elapsed, without starting anything.
    fn timed_out_teammate<'a>(
        _kind: AgentKind,
        _spec: &'a RunSpec,
        _meter: &'a Meter,
        _deadline: Duration,
    ) -> TeammateRun<'a> {
        Box::pin(async {
            agent::Collected {
                timed_out: true,
                ..agent::Collected::failed("timed out".into())
            }
        })
    }

    /// Wave-1 review: a Grok or Antigravity teammate that times out still
    /// says it ran without Hollow's cards, as its success and failure results
    /// do; a Claude teammate's timeout carries no such note.
    #[test]
    fn a_timed_out_teammate_without_owned_mcp_says_hollow_cards_are_absent() {
        let dir = budget_vault();
        let v = dir.path();
        let call = |agent_name: &str, perm: &str| {
            let mut env_map = parent_env(v, "chat");
            env_map.insert(ENV_PARENT_PERM.into(), perm.into());
            env_map.insert(ENV_PARENT_DENIED.into(), "[]".into());
            let env = |key: &str| env_map.get(key).cloned();
            let args = json!({ "agent": agent_name, "task": "draw", "cwd": v.to_string_lossy() });
            let response = block_on(async {
                Some(
                    call_tool_with(
                        Some(json!(6)),
                        "ask_agent",
                        &args,
                        &env,
                        budgets::Store::for_app(v),
                        Utc::now(),
                        Teammate {
                            resolve: resolve_default,
                            run: timed_out_teammate,
                        },
                    )
                    .await,
                )
            })
            .unwrap();
            tool_text_of(&response)
        };
        let (is_error, text) = call("grok", "auto");
        assert!(is_error, "{text}");
        assert!(text.starts_with("Grok timed out after"), "{text}");
        assert!(
            text.contains("Grok can't take Hollow's per-run tool setup")
                && text.contains("confirm or question cards"),
            "{text}"
        );

        let (is_error, text) = call("antigravity", "acceptEdits");
        assert!(is_error, "{text}");
        assert!(
            text.contains("Antigravity can't take Hollow's per-run tool setup"),
            "{text}"
        );

        let (is_error, text) = call("claude", "auto");
        assert!(is_error, "{text}");
        assert!(text.starts_with("Claude timed out after"), "{text}");
        assert!(!text.contains("per-run tool setup"), "{text}");
    }

    /// Paperclip PR 15a: the parent's scratch folder rides the team server's
    /// explicit environment; the existing keys are unchanged.
    #[test]
    fn delegate_env_carries_the_parent_scratch_dir() {
        let parent = DelegateParent {
            surface: usage_ledger::Surface::Workflow,
            scope: "brief".into(),
            run_id: "R9".into(),
            scratch: Some(PathBuf::from("/private/tmp/hollow-run/R9")),
            tool_limits: Default::default(),
        };
        let env = delegate_env(
            Path::new("/tmp/x"),
            Path::new("/v"),
            AgentKind::Claude,
            &parent,
            "acceptEdits",
        );
        assert_eq!(
            env[crate::scratch::SCRATCH_ENV],
            json!("/private/tmp/hollow-run/R9")
        );
        assert_eq!(
            env["HOLLOW_RUN_ID"],
            json!("R9"),
            "the marker's run id is checked against it"
        );
        assert_eq!(env["HOLLOW_VAULT"], json!("/tmp/x"));
        for key in env.keys() {
            for word in ["KEY", "SECRET", "TOKEN"] {
                assert!(!key.contains(word), "Codex would strip {key}");
            }
        }
    }

    /// What [`capturing_teammate`] saw: the spec's scratch and its prompt.
    static CAPTURED: std::sync::Mutex<Vec<(Option<crate::scratch::ScratchSpec>, String)>> =
        std::sync::Mutex::new(Vec::new());

    /// A teammate that records what `call_tool_with` put on its spec, then
    /// runs like [`scripted_oneshot`].
    fn capturing_teammate<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
    ) -> TeammateRun<'a> {
        CAPTURED
            .lock()
            .unwrap()
            .push((spec.extras.scratch.clone(), spec.prompt.clone()));
        scripted_oneshot(kind, spec, meter, deadline)
    }

    /// Paperclip PR 15a, through `call_tool_with` with the injected
    /// environment (never the process's): a parent scratch folder that
    /// validates (its marker names HOLLOW_RUN_ID) reaches the teammate's spec
    /// as `ScratchSpec::Dir` with the guidance appended to its prompt; a
    /// run-id mismatch, or no folder, leaves `scratch: None` and no guidance.
    /// The delegate never creates or removes the folder.
    #[cfg(unix)]
    #[test]
    fn a_delegate_reuses_only_a_validated_parent_scratch() {
        let dir = budget_vault();
        let v = dir.path();
        let tmp = tempfile::Builder::new()
            .prefix("scratch-team-")
            .tempdir()
            .unwrap();
        let base = std::fs::canonicalize(tmp.path())
            .unwrap()
            .join("hollow-run");
        let folder = crate::scratch::create_in(
            &base,
            "parent-folder",
            "R-parent",
            std::process::id() as i32,
        )
        .unwrap();
        let folder_text = folder.path().to_string_lossy().into_owned();
        let delegate = |run_id: &str, scratch: Option<&str>| {
            let mut env_map = parent_env(v, "workflow");
            env_map.insert("HOLLOW_RUN_ID".into(), run_id.into());
            // The real team-mcp situation: TMPDIR already is the folder.
            env_map.insert("TMPDIR".into(), format!("{folder_text}/"));
            if let Some(scratch) = scratch {
                env_map.insert(crate::scratch::SCRATCH_ENV.into(), scratch.into());
            }
            CAPTURED.lock().unwrap().clear();
            let env = |key: &str| env_map.get(key).cloned();
            let response = block_on(async {
                Some(
                    call_tool_with(
                        Some(json!(9)),
                        "ask_agent",
                        &json!({ "agent": "claude", "task": "summarize", "cwd": v.to_string_lossy() }),
                        &env,
                        budgets::Store::for_app(v),
                        Utc::now(),
                        Teammate { resolve: resolve_default, run: capturing_teammate },
                    )
                    .await,
                )
            })
            .unwrap();
            let (is_error, text) = tool_text_of(&response);
            assert!(!is_error, "{text}");
            let mut seen = CAPTURED.lock().unwrap();
            assert_eq!(seen.len(), 1, "the teammate ran once");
            seen.pop().unwrap()
        };

        let (scratch, prompt) = delegate("R-parent", Some(&folder_text));
        assert_eq!(
            scratch,
            Some(crate::scratch::ScratchSpec::Dir(
                folder.path().to_path_buf()
            ))
        );
        assert!(
            prompt.ends_with(&format!("\n\n{}", crate::scratch::SCRATCH_GUIDANCE)),
            "{prompt}"
        );
        assert!(prompt.contains("summarize"), "{prompt}");

        for (run_id, scratch_env) in [("R-other", Some(folder_text.as_str())), ("R-parent", None)] {
            let (scratch, prompt) = delegate(run_id, scratch_env);
            assert_eq!(scratch, None, "{run_id} {scratch_env:?}");
            assert!(
                !prompt.contains(crate::scratch::SCRATCH_GUIDANCE),
                "{prompt}"
            );
        }
        assert!(
            folder.path().join(crate::scratch::MARKER).exists(),
            "the delegate never removes it"
        );
        let body = fn_body(
            include_str!("team_mcp.rs"),
            concat!("async fn ", "call_tool_with("),
        );
        assert!(
            !body.contains(concat!("scratch::", "create")),
            "a delegate never creates scratch"
        );
        folder.remove_now();
    }

    /// `serve` and `call_tool` run in the subprocess against real CLIs, so
    /// this pins the wiring in source; the needles are split so this test's
    /// own text never matches them.
    #[test]
    fn delegations_meter_from_the_vault_root_and_end_cleanly_on_sigterm() {
        let source = include_str!("team_mcp.rs");
        let body = |needle: &str| {
            let start = source.find(needle).unwrap_or_else(|| panic!("{needle}"));
            let end = start + source[start..].find("\n}\n").expect("end of fn");
            &source[start..end]
        };
        let call = body(concat!("async fn ", "call_tool_with("));
        let metered = call
            .find(concat!("Meter::", "delegate_from_env("))
            .expect("delegate meter");
        let run = call
            .find(concat!("(teammate.", "run)("))
            .expect("runs the teammate");
        assert!(metered < run);
        assert!(call[run..].contains(concat!("&spec, ", "&meter, deadline)")));
        // The installed teammate is the metered one-shot runner, bounded by
        // the consultation's deadline (at most TEAMMATE_TIMEOUT, F10 D1).
        let installed = body(concat!("fn ", "installed() -> Self {"));
        assert!(crate::source_pins::contains(
            installed,
            concat!(
                "agent::run_collect(kind, spec, ",
                "Some(deadline), None, meter)"
            )
        ));
        assert!(crate::source_pins::contains(
            call,
            concat!(".map_or(TEAMMATE_TIMEOUT, ", "Duration::from_secs)")
        ));
        assert!(installed.contains(concat!("agent_settings::", "resolve(")));
        // Paperclip PR 8b: the read-only budget preflight comes before the
        // teammate is resolved, and nothing here may write budget state.
        let preflight = call
            .find(concat!("budgets::", "preflight_readonly("))
            .expect("preflight");
        let resolved = call
            .find(concat!("(teammate.", "resolve)("))
            .expect("resolve");
        assert!(preflight < resolved);
        for writer in [
            concat!("budgets::", "preflight("),
            concat!("budgets::", "after_run("),
            concat!("review_", "queue::"),
            concat!("notifi", "cations::"),
        ] {
            assert!(
                !source.contains(writer),
                "team-mcp must never call {writer}"
            );
        }
        let serve = body(concat!("pub fn ", "serve("));
        assert!(serve.contains(concat!("tokio::", "select!")));
        assert!(
            serve.contains(concat!("terminated", "()")),
            "serve races the SIGTERM future"
        );
        assert!(
            serve.contains(concat!("shutdown", "_background()")),
            "a SIGTERM exits without waiting on the uncancellable stdin read"
        );
        let handler = body(concat!("#[cfg(unix)]\nasync fn ", "terminated("));
        let signal = handler
            .find(concat!("SignalKind::", "terminate()"))
            .expect("SIGTERM handler");
        let noted = handler
            .find(concat!("note_process", "_terminating("))
            .expect("marks terminating");
        assert!(
            signal < noted,
            "terminating is noted before the serve future is dropped"
        );
        assert!(
            !source.contains(concat!("pub fn ", "server_entry(")),
            "the cwd-as-vault wrapper stays gone"
        );
        assert!(!source.contains(concat!("pub fn ", "mcp_config_json(")));
    }

    // ── Paperclip PR 8b: budgets in team-mcp ────────────────────────────

    fn budget_vault() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("vault-team-")
            .tempdir()
            .unwrap()
    }

    /// A hard-stop daily token budget on the "brief" workflow, already over
    /// its limit in the vault's ledger.
    fn over_budget(v: &Path) -> Vec<u8> {
        let file: budgets::BudgetsFile = serde_json::from_value(json!({
            "schemaVersion": 1,
            "budgets": [{
                "id": "b-brief",
                "scope": "workflow:brief",
                "metric": "tokens",
                "window": "day",
                "limit": 100.0,
                "hardStop": true,
            }],
        }))
        .unwrap();
        let store = budgets::Store::for_app(v).unwrap();
        store.save(&file).unwrap();
        let mut row = crate::session_store::tests::usage_row_fixture(
            "over-1",
            &crate::session_store::db_time(Utc::now()),
            "success",
        );
        row.surface = "workflow".into();
        row.scope = Some("brief".into());
        row.input_tokens = Some(500);
        row.output_tokens = Some(0);
        row.cost_status = "unpriced".into();
        crate::session_store::upsert_usage(v, &row).unwrap();
        std::fs::read(store.path()).unwrap()
    }

    fn parent_env(v: &Path, surface: &str) -> HashMap<String, String> {
        HashMap::from([
            (
                "HOLLOW_VAULT_ROOT".to_string(),
                v.to_string_lossy().into_owned(),
            ),
            ("HOLLOW_RUN_SURFACE".to_string(), surface.to_string()),
            ("HOLLOW_RUN_SCOPE".to_string(), "brief".to_string()),
            ("HOLLOW_RUN_ID".to_string(), "R-parent".to_string()),
            (ENV_PARENT_PERM.to_string(), "acceptEdits".to_string()),
        ])
    }

    /// A teammate that must never be reached (a refused delegation).
    fn unreachable_teammate() -> Teammate {
        fn resolve(_: AgentKind) -> Result<Option<String>, String> {
            panic!("a refused delegation never resolves its teammate")
        }
        fn run<'a>(_: AgentKind, _: &'a RunSpec, _: &'a Meter, _: Duration) -> TeammateRun<'a> {
            panic!("a refused delegation never starts its teammate")
        }
        Teammate { resolve, run }
    }

    fn resolve_default(_: AgentKind) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// The process group the scripted delegate joins (plan test 14's parent
    /// leader P); only the budget-kill test sets it.
    static PARENT_GROUP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    /// A scripted Claude teammate standing in for the installed CLI: it
    /// checks what `call_tool_with` handed it (partial-message streaming and
    /// the budgeted meter), then runs `/bin/sh` through the real
    /// `collect_command` with the meter's budget, the way `run_collect`
    /// would. The delegate joins [`PARENT_GROUP`] without a run tag (an
    /// inherited spawn inside team-mcp), records its pid in the working
    /// folder, prints the chat-tool capture up to msg-003's final usage and
    /// then sleeps, so only the budget guard can end it.
    fn scripted_over_budget<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        _deadline: Duration,
    ) -> TeammateRun<'a> {
        Box::pin(async move {
            assert_eq!(kind, AgentKind::Claude);
            assert!(
                spec.usage_stream,
                "a Claude teammate under a token budget streams its usage"
            );
            assert!(
                spec.max_budget_usd.is_none(),
                "the dollar backstop is GUI-only"
            );
            assert!(
                meter
                    .budget()
                    .is_some_and(|budget| !budget.checks.is_empty()),
                "the meter carries the budget"
            );
            let mut cmd = tokio::process::Command::new("/bin/sh");
            cmd.arg("-c")
                .arg("echo $$ > \"$P\"; cat \"$F\"; exec /bin/sleep 30")
                .env("F", spec.cwd.join("first-delta.jsonl.txt"))
                .env("P", spec.cwd.join("delegate.pid"))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            #[cfg(unix)]
            cmd.process_group(PARENT_GROUP.load(std::sync::atomic::Ordering::SeqCst));
            let ledger =
                usage_ledger::UsageRun::begin(meter.clone(), Some(kind), spec.model.clone());
            agent::collect_command(
                kind,
                cmd,
                agent::ParseState::for_spec(kind, spec),
                Some(Duration::from_secs(20)),
                None,
                ledger,
            )
            .await
        })
    }

    /// A scripted Claude teammate that prints the one-shot capture and exits.
    fn scripted_oneshot<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        _deadline: Duration,
    ) -> TeammateRun<'a> {
        Box::pin(async move {
            let fixture = format!(
                "{}/fixtures/usage/claude-oneshot.jsonl",
                env!("CARGO_MANIFEST_DIR")
            );
            let mut cmd = tokio::process::Command::new("/bin/sh");
            cmd.arg("-c")
                .arg("cat \"$F\"")
                .env("F", fixture)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let ledger =
                usage_ledger::UsageRun::begin(meter.clone(), Some(kind), spec.model.clone());
            agent::collect_command(
                kind,
                cmd,
                agent::ParseState::for_spec(kind, spec),
                Some(Duration::from_secs(20)),
                None,
                ledger,
            )
            .await
        })
    }

    fn save_budget(v: &Path, scope: &str, limit: f64) {
        let file: budgets::BudgetsFile = serde_json::from_value(json!({
            "schemaVersion": 1,
            "budgets": [{
                "id": "b-delegate",
                "scope": scope,
                "metric": "tokens",
                "window": "day",
                "limit": limit,
                "hardStop": true,
            }],
        }))
        .unwrap();
        budgets::Store::for_app(v).unwrap().save(&file).unwrap();
    }

    fn tool_text_of(response: &Value) -> (bool, String) {
        let result = &response["result"];
        (
            result["isError"] == json!(true),
            result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )
    }

    /// A teammate that resolves but must never start (a delegation refused
    /// after resolution).
    fn never_started_teammate() -> Teammate {
        fn run<'a>(_: AgentKind, _: &'a RunSpec, _: &'a Meter, _: Duration) -> TeammateRun<'a> {
            panic!("a refused delegation never starts its teammate")
        }
        Teammate {
            resolve: resolve_default,
            run,
        }
    }

    fn fn_body<'a>(source: &'a str, needle: &str) -> &'a str {
        let start = source.find(needle).unwrap_or_else(|| panic!("{needle}"));
        let end = start + source[start..].find("\n}\n").expect("end of fn");
        &source[start..end]
    }

    /// Paperclip PR 19: a delegation into a system or shared root is a tool
    /// error, before the run permit is taken (so it never holds a slot) and
    /// before the teammate starts. Paperclip PR 20: the permit's own refusal
    /// (busy, stopped, or held back by memory pressure) reaches the primary
    /// as the delegation's tool error text; this subprocess cannot see the
    /// GUI's session state.
    #[cfg(unix)]
    #[test]
    fn an_unsafe_working_folder_is_refused_before_the_permit() {
        let dir = budget_vault();
        let v = dir.path();
        let env_map = parent_env(v, "chat");
        let env = |key: &str| env_map.get(key).cloned();
        let response = block_on(async {
            Some(
                call_tool_with(
                    Some(json!(1)),
                    "ask_agent",
                    &json!({ "agent": "codex", "task": "list the files", "cwd": "/tmp" }),
                    &env,
                    budgets::Store::for_app(v),
                    Utc::now(),
                    never_started_teammate(),
                )
                .await,
            )
        })
        .unwrap();
        let (is_error, text) = tool_text_of(&response);
        assert!(is_error);
        assert!(text.starts_with(agent::UNSAFE_CWD_ERROR), "{text}");

        let body = fn_body(
            include_str!("team_mcp.rs"),
            concat!("async fn ", "call_tool_with("),
        );
        let check = body
            .find(concat!("agent::", "unsafe_working_dir("))
            .expect("the cwd check");
        let permit = body
            .find(concat!("run_permit::", "acquire("))
            .expect("the permit");
        assert!(
            check < permit,
            "the folder is checked before a slot is taken"
        );
        let after = &body[permit..];
        let refusal = after
            .find(concat!("Err(error) => return ", "tool_error(id, &error)"))
            .expect("a permit refusal becomes the tool error");
        let run = after
            .find(concat!("(teammate.", "run)("))
            .expect("the teammate runs");
        assert!(refusal < run);
        assert!(
            !body.contains(concat!("agent_", "sessions::")),
            "no GUI session state here"
        );
    }

    // PR 8 plan test 16 and refresh O2: a delegate under a paused automation
    // is refused BEFORE the teammate is resolved or started (the injected
    // teammate panics if reached, so a regression can never run a real
    // CLI), and the refusal writes nothing: no pause, no review item, no
    // notification, no budgets.json change, no flag on the parent's run.
    #[test]
    fn a_paused_scope_refuses_the_delegation_and_writes_nothing() {
        let dir = budget_vault();
        let v = dir.path();
        let before = over_budget(v);
        let run = crate::run_guard::begin_for_test(v, "workflow", &[]);
        let env_map = parent_env(v, "workflow");
        let env = |key: &str| env_map.get(key).cloned();
        let store = budgets::Store::for_app(v);
        let response = block_on(async {
            Some(
                call_tool_with(
                    Some(json!(1)),
                    "ask_agent",
                    &json!({ "agent": "codex", "task": "write the brief" }),
                    &env,
                    store,
                    Utc::now(),
                    unreachable_teammate(),
                )
                .await,
            )
        })
        .unwrap();
        let (is_error, text) = tool_text_of(&response);
        assert!(is_error);
        assert!(text.starts_with(budgets::BUDGET_PAUSED_PREFIX), "{text}");

        let store = budgets::Store::for_app(v).unwrap();
        assert_eq!(
            std::fs::read(store.path()).unwrap(),
            before,
            "budgets.json untouched"
        );
        assert!(store.load().unwrap().pauses.is_empty(), "no pause opened");
        assert!(!v.join(".review-queue.json").exists(), "no review item");
        assert!(budgets::sent_for_test(v).is_empty(), "no notification");
        assert!(
            crate::run_guard::finish_for_test(v, &run).is_empty(),
            "no flag on the parent"
        );

        // Under chat the same budget never refuses (read-only check only: a
        // real delegation would start a teammate CLI).
        let chat = budgets::RunIntent::delegate(
            Some(AgentKind::Codex),
            budgets::ParentRun {
                surface: Some(usage_ledger::Surface::Chat),
                scope: Some("s-1".into()),
            },
        );
        assert!(
            budgets::preflight_readonly(&Ok(store), Some(v), &chat, Utc::now())
                .blocked()
                .is_none()
        );
    }

    // PR 8 plan test 14 and refresh O2, through `call_tool_with`: a Claude
    // delegate under a workflow that crosses an enforced token limit mid-run
    // is killed ALONE. It joined a parent leader's process group without a
    // run tag (as a delegate inside team-mcp does); afterwards the delegate
    // is gone and the parent group still lives, because an inherited run's
    // stop signals only its direct process. Its partial row goes to the
    // vault root's store credited to the parent, the primary gets the stop
    // as a tool error, and the parent's tracked run raises no flag.
    #[cfg(unix)]
    #[test]
    fn a_budget_killed_delegate_is_killed_alone_and_records_its_partial_row() {
        use crate::process_group::test_support::{spawn_isolated, wait_gone};
        let dir = budget_vault();
        let v = dir.path();
        save_budget(v, "workflow:brief", 18_050.0);
        let head: String = crate::usage::tests::CHAT_TOOL
            .lines()
            .take(19)
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(v.join("first-delta.jsonl.txt"), head).unwrap();
        let run = crate::run_guard::begin_for_test(v, "workflow", &[]);
        let env_map = parent_env(v, "workflow");
        let env = |key: &str| env_map.get(key).cloned();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (_parent_cmd, mut parent) = spawn_isolated("exec /bin/sleep 60", &[]);
            let leader = parent.id().unwrap() as i32;
            PARENT_GROUP.store(leader, std::sync::atomic::Ordering::SeqCst);
            let started = std::time::Instant::now();
            let response = call_tool_with(
                Some(json!(7)),
                "ask_agent",
                &json!({ "agent": "claude", "task": "write the brief", "cwd": v.to_string_lossy() }),
                &env,
                budgets::Store::for_app(v),
                Utc::now(),
                Teammate {
                    resolve: resolve_default,
                    run: scripted_over_budget,
                },
            )
            .await;
            // The guard ended it, not the scripted child's 20 s timeout: the
            // stop-reason prefix below proves that. A wall-clock bound here
            // (it was 15 s) failed gates on a loaded machine (overnight
            // 2026-10-15 H1), so only a bound just under the timeout stays.
            assert!(started.elapsed() < Duration::from_secs(20), "ended before the timeout");
            let (is_error, text) = tool_text_of(&response);
            assert!(is_error, "{text}");
            assert!(text.starts_with(budgets::BUDGET_STOP_PREFIX), "{text}");
            assert!(
                text.ends_with("The Claude teammate was stopped; its partial usage was recorded."),
                "{text}"
            );
            let delegate: i32 = std::fs::read_to_string(v.join("delegate.pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert!(wait_gone(delegate, Duration::from_secs(5)).await, "the delegate was killed");
            // SAFETY: signal 0 only probes for existence.
            assert_eq!(unsafe { libc::kill(leader, 0) }, 0, "the parent group survives");
            let _ = parent.start_kill();
            let _ = parent.wait().await;
        });
        let rows = crate::session_store::usage_rows_since(v, "").unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].surface, "delegate");
        assert_eq!(rows[0].parent_surface.as_deref(), Some("workflow"));
        assert_eq!(rows[0].scope.as_deref(), Some("brief"));
        assert_eq!(rows[0].outcome, "budget_stopped");
        assert_eq!(rows[0].cost_status, "partial");
        assert_eq!(
            (rows[0].input_tokens, rows[0].output_tokens),
            (Some(18_012), Some(74))
        );
        for scratch in ["first-delta.jsonl.txt", "delegate.pid"] {
            let _ = std::fs::remove_file(v.join(scratch));
        }
        assert!(!v.join(".review-queue.json").exists());
        assert!(
            crate::run_guard::finish_for_test(v, &run).is_empty(),
            "no flag on the parent"
        );
    }

    // PR 8 plan test 15, through `call_tool_with`: a delegate under chat that
    // crosses its limit mid-run is never stopped; the level it crossed is
    // shown to the primary once, in front of the result.
    #[cfg(unix)]
    #[test]
    fn a_delegate_under_chat_is_warned_once_and_never_stopped() {
        let dir = budget_vault();
        let v = dir.path();
        save_budget(v, "global", 100.0);
        let env_map = parent_env(v, "chat");
        let env = |key: &str| env_map.get(key).cloned();
        let response = block_on(async {
            Some(
                call_tool_with(
                    Some(json!(8)),
                    "ask_agent",
                    &json!({ "agent": "claude", "task": "summarize", "cwd": v.to_string_lossy() }),
                    &env,
                    budgets::Store::for_app(v),
                    Utc::now(),
                    Teammate {
                        resolve: resolve_default,
                        run: scripted_oneshot,
                    },
                )
                .await,
            )
        })
        .unwrap();
        let (is_error, text) = tool_text_of(&response);
        assert!(!is_error, "{text}");
        assert!(text.starts_with("[Budget warning: "), "{text}");
        assert_eq!(text.matches("[Budget warning: ").count(), 1);
        assert!(
            text.ends_with("ok"),
            "the teammate's result survives: {text}"
        );
        let rows = crate::session_store::usage_rows_since(v, "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, "success");
        assert!(!v.join(".review-queue.json").exists());
    }

    // ── F10 · D1: the bounded consultation's task packet ─────────────────

    /// What [`prompt_capturing_teammate`] saw: each spec's prompt.
    static CAPTURED_PROMPTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    fn prompt_capturing_teammate<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
    ) -> TeammateRun<'a> {
        CAPTURED_PROMPTS.lock().unwrap().push(spec.prompt.clone());
        scripted_oneshot(kind, spec, meter, deadline)
    }

    /// The parent conversation `key`'s stored task-state record: one
    /// constraint the user settled and one correction.
    fn seed_parent_task_state(v: &Path, key: &str) {
        let payload = json!({
            "schema": crate::task_state::SCHEMA,
            "record": {
                "goal": { "text": "Draft the launch note.", "turn": 1 },
                "constraints": [
                    { "text": "Always write dates as ISO 8601.", "turn": 2 }
                ],
                "corrections": [
                    { "text": "No, the audience is the finance team.", "turn": 3 }
                ],
                "artifacts": [
                    { "path": "outputs/launch-note.md", "change": "create", "turn": 3 }
                ]
            }
        });
        crate::session_store::put_derived_state(
            v,
            key,
            crate::task_state::KIND,
            &crate::session_store::DerivedRow {
                revision: 3,
                through_ord: 3,
                payload: payload.to_string(),
            },
        )
        .unwrap();
    }

    /// F10 (red first): a bounded consultation from a chat used to reach the
    /// teammate with none of the parent conversation's settled constraints.
    /// Now its packet leads the prompt and carries them, with the files the
    /// parent's turns changed as sources.
    #[test]
    fn a_consultation_receives_the_parents_settled_constraints() {
        let dir = budget_vault();
        let v = dir.path();
        let key = "s-20261007-d1parent";
        seed_parent_task_state(v, key);
        let mut env_map = parent_env(v, "chat");
        env_map.insert("HOLLOW_RUN_SCOPE".into(), key.into());
        env_map.insert(ENV_PARENT_DENIED.into(), "[]".into());
        let env = |key: &str| env_map.get(key).cloned();
        CAPTURED_PROMPTS.lock().unwrap().clear();
        let response = block_on(async {
            Some(
                call_tool_with(
                    Some(json!(11)),
                    "ask_agent",
                    &json!({ "agent": "claude", "task": "Check the dates in the note.", "cwd": v.to_string_lossy() }),
                    &env,
                    budgets::Store::for_app(v),
                    Utc::now(),
                    Teammate {
                        resolve: resolve_default,
                        run: prompt_capturing_teammate,
                    },
                )
                .await,
            )
        })
        .unwrap();
        let (is_error, text) = tool_text_of(&response);
        assert!(!is_error, "{text}");
        let prompts = CAPTURED_PROMPTS.lock().unwrap().clone();
        assert_eq!(prompts.len(), 1);
        let prompt = &prompts[0];
        assert!(
            prompt.contains("Always write dates as ISO 8601."),
            "the parent's settled constraint reaches the consultation: {prompt}"
        );
        assert!(
            prompt.contains("No, the audience is the finance team."),
            "{prompt}"
        );
        assert!(
            prompt.starts_with("<hollow-task-packet>"),
            "the packet comes first: {prompt}"
        );
        assert!(prompt.contains("outputs/launch-note.md"), "{prompt}");
        assert!(
            !prompt.contains("Draft the launch note."),
            "a consultation gets only the packet, not the parent's goal: {prompt}"
        );
    }

    /// The deadline each run of [`deadline_capturing_teammate`] was given.
    static CAPTURED_DEADLINES: std::sync::Mutex<Vec<Duration>> = std::sync::Mutex::new(Vec::new());

    fn deadline_capturing_teammate<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
    ) -> TeammateRun<'a> {
        CAPTURED_DEADLINES.lock().unwrap().push(deadline);
        scripted_oneshot(kind, spec, meter, deadline)
    }

    /// F10 (D1): a packet's budget lowers the consultation's deadline (never
    /// past 180 s), the parent's own constraint leads, the delegation is
    /// recorded for the asking run, and a bad packet runs nothing.
    #[test]
    fn a_consultation_packet_bounds_its_deadline_and_is_recorded() {
        let dir = budget_vault();
        let v = dir.path();
        let mut env_map = parent_env(v, "workflow");
        env_map.insert(ENV_PARENT_DENIED.into(), "[]".into());
        env_map.insert("HOLLOW_RUN_ID".into(), "R-d1-consult".into());
        let env = |key: &str| env_map.get(key).cloned();
        let call = |packet: Value, teammate: Teammate| {
            block_on(async {
                Some(
                    call_tool_with(
                        Some(json!(12)),
                        "ask_agent",
                        &json!({ "agent": "codex", "task": "check", "cwd": v.to_string_lossy(), "packet": packet }),
                        &env,
                        budgets::Store::for_app(v),
                        Utc::now(),
                        teammate,
                    )
                    .await,
                )
            })
            .unwrap()
        };
        let capturing = Teammate {
            resolve: resolve_default,
            run: deadline_capturing_teammate,
        };
        CAPTURED_DEADLINES.lock().unwrap().clear();
        let (is_error, text) = tool_text_of(&call(
            json!({ "constraints": ["Answer in one line."], "budget": { "seconds": 45, "usd": 0.2 }, "expected": "yes or no" }),
            capturing,
        ));
        assert!(!is_error, "{text}");
        let (is_error, _) =
            tool_text_of(&call(json!({ "budget": { "seconds": 3_600 } }), capturing));
        assert!(!is_error);
        assert_eq!(
            *CAPTURED_DEADLINES.lock().unwrap(),
            vec![Duration::from_secs(45), TEAMMATE_TIMEOUT]
        );
        let recorded = crate::delegation::for_run(v, "R-d1-consult");
        assert_eq!(recorded.len(), 2);
        let first = &recorded[0];
        assert_eq!(
            (
                first.kind.as_str(),
                first.tool.as_str(),
                first.agent.as_str()
            ),
            ("consultation", "ask_agent", "codex")
        );
        assert_eq!(first.deadline_seconds, Some(45));
        assert_eq!(first.budget_usd, Some(0.2));
        assert_eq!(first.constraints, 1);
        assert_eq!(
            first.from_task_state, 0,
            "a workflow parent has no conversation"
        );
        assert_eq!(first.child_session, None);
        assert_eq!(recorded[1].deadline_seconds, Some(180));

        let (is_error, text) = tool_text_of(&call(
            json!({ "budget": { "minutes": 2 } }),
            never_started_teammate(),
        ));
        assert!(is_error && text.contains("Nothing ran."), "{text}");
        assert_eq!(crate::delegation::for_run(v, "R-d1-consult").len(), 2);
    }

    /// What [`streams_then_hangs`] writes before it hangs.
    const PARTIAL_ANSWER: &str = "Found two callers so far: a.rs and b.rs.";

    /// A scripted Claude teammate that runs `/bin/sh` through the real
    /// `collect_command` at the consultation's own deadline: it prints the
    /// stream-json line `$L` (when set) and then sleeps past that deadline,
    /// so only the deadline can end it.
    fn hanging_teammate<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
        line: Option<String>,
    ) -> TeammateRun<'a> {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new("/bin/sh");
            cmd.arg("-c")
                .arg("[ -n \"$L\" ] && printf '%s\\n' \"$L\"; exec /bin/sleep 30")
                .env("L", line.unwrap_or_default())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let ledger =
                usage_ledger::UsageRun::begin(meter.clone(), Some(kind), spec.model.clone());
            agent::collect_command(
                kind,
                cmd,
                agent::ParseState::for_spec(kind, spec),
                Some(deadline),
                None,
                ledger,
            )
            .await
        })
    }

    /// Streams [`PARTIAL_ANSWER`] as one assistant text block, then hangs.
    fn streams_then_hangs<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
    ) -> TeammateRun<'a> {
        let line = json!({
            "type": "assistant",
            "message": { "content": [{ "type": "text", "text": PARTIAL_ANSWER }] },
        });
        hanging_teammate(kind, spec, meter, deadline, Some(line.to_string()))
    }

    /// Hangs without writing anything.
    fn hangs_silently<'a>(
        kind: AgentKind,
        spec: &'a RunSpec,
        meter: &'a Meter,
        deadline: Duration,
    ) -> TeammateRun<'a> {
        hanging_teammate(kind, spec, meter, deadline, None)
    }

    /// One Claude consultation with a 2 s packet budget for run `run_id`:
    /// the tool result and the run's delegation records.
    fn consult_claude(
        v: &Path,
        run_id: &str,
        teammate: Teammate,
    ) -> ((bool, String), Vec<crate::run_manifest::DelegationRecord>) {
        let mut env_map = parent_env(v, "chat");
        env_map.insert(ENV_PARENT_PERM.into(), "auto".into());
        env_map.insert(ENV_PARENT_DENIED.into(), "[]".into());
        env_map.insert("HOLLOW_RUN_ID".into(), run_id.into());
        let env = |key: &str| env_map.get(key).cloned();
        let args = json!({
            "agent": "claude",
            "task": "find the callers",
            "cwd": v.to_string_lossy(),
            "packet": { "budget": { "seconds": 2 } },
        });
        let response = block_on(async {
            Some(
                call_tool_with(
                    Some(json!(13)),
                    "ask_agent",
                    &args,
                    &env,
                    budgets::Store::for_app(v),
                    Utc::now(),
                    teammate,
                )
                .await,
            )
        })
        .unwrap();
        (
            tool_text_of(&response),
            crate::delegation::for_run(v, run_id),
        )
    }

    /// OF04: a teammate that streamed text and then hung past its deadline
    /// returns that text, flagged Incomplete first and last with the way to
    /// longer work, and its record says it timed out, how long it ran and
    /// how many bytes it returned. The 2 s Claude budget draws its note.
    #[test]
    fn a_timed_out_consultation_returns_its_partial_text_and_records_the_timeout() {
        let dir = budget_vault();
        let v = dir.path();
        let teammate = Teammate {
            resolve: resolve_default,
            run: streams_then_hangs,
        };
        let ((is_error, text), recorded) = consult_claude(v, "R-of04-partial", teammate);
        assert!(
            is_error,
            "an incomplete consultation is not a success: {text}"
        );
        let first_line = text.lines().next().unwrap_or_default();
        assert!(
            first_line.starts_with("Incomplete: Claude was stopped at the 2 s deadline"),
            "{text}"
        );
        assert!(first_line.contains("continue with start_session"), "{text}");
        assert!(
            text.contains(PARTIAL_ANSWER),
            "the partial text comes back: {text}"
        );
        assert!(
            text.contains("packet.budget.seconds 2 is short for a Claude teammate"),
            "{text}"
        );
        let trailer = "[End of Claude's incomplete output: it was cut off at the deadline.]";
        assert!(text.trim_end().ends_with(trailer), "{text}");

        assert_eq!(recorded.len(), 1);
        let record = &recorded[0];
        assert_eq!(record.outcome.as_deref(), Some("timeout"));
        assert_eq!(record.incomplete, Some(true));
        assert_eq!(record.returned_bytes, Some(PARTIAL_ANSWER.len() as u64));
        assert_eq!(record.deadline_seconds, Some(2));
        // The deadline ended it, not the 30 s sleep: at least the deadline,
        // well short of the sleep.
        let ran = record.duration_ms.expect("a duration");
        assert!((2_000..15_000).contains(&ran), "{ran} ms");
    }

    /// OF04: a teammate that wrote nothing before its deadline returns the
    /// timeout error it always did, with the 2 s budget's note; its record
    /// still says timeout, with no bytes returned.
    #[test]
    fn a_silent_timed_out_consultation_keeps_the_timeout_error() {
        let dir = budget_vault();
        let v = dir.path();
        let teammate = Teammate {
            resolve: resolve_default,
            run: hangs_silently,
        };
        let ((is_error, text), recorded) = consult_claude(v, "R-of04-silent", teammate);
        assert!(is_error, "{text}");
        assert!(text.starts_with("Claude timed out after 2s"), "{text}");
        assert!(!text.contains("Incomplete:"), "{text}");
        assert!(
            text.contains("packet.budget.seconds 2 is short for a Claude teammate"),
            "{text}"
        );
        assert_eq!(recorded.len(), 1);
        let record = &recorded[0];
        assert_eq!(record.outcome.as_deref(), Some("timeout"));
        assert_eq!(record.incomplete, Some(true));
        assert_eq!(record.returned_bytes, Some(0));
    }

    /// OF04: a teammate that answers records `ok`, complete, with the
    /// answer's bytes ("ok" in the one-shot capture), and its answer carries
    /// no short-budget note though its budget was 2 s.
    #[test]
    fn a_finished_consultation_records_ok_and_its_bytes() {
        let dir = budget_vault();
        let v = dir.path();
        let teammate = Teammate {
            resolve: resolve_default,
            run: scripted_oneshot,
        };
        let ((is_error, text), recorded) = consult_claude(v, "R-of04-ok", teammate);
        assert!(!is_error, "{text}");
        assert!(!text.contains("Incomplete:"), "{text}");
        assert!(
            !text.contains("is short for a Claude teammate"),
            "a finished answer draws no short-budget note: {text}"
        );
        assert_eq!(recorded.len(), 1);
        let record = &recorded[0];
        assert_eq!(record.outcome.as_deref(), Some("ok"));
        assert_eq!(record.incomplete, Some(false));
        assert_eq!(record.returned_bytes, Some("ok".len() as u64));
        assert!(record.duration_ms.is_some());
    }

    /// OF04: only a Claude teammate under a packet budget below 90 s draws
    /// the short-budget note, and only a timed-out consultation's result
    /// carries it (the timeout tests above; a finished answer never does,
    /// `a_finished_consultation_records_ok_and_its_bytes`).
    #[test]
    fn only_a_short_claude_budget_draws_the_note() {
        let secs = Duration::from_secs;
        assert!(short_budget_note(AgentKind::Claude, secs(89)).is_some());
        assert!(short_budget_note(AgentKind::Claude, secs(90)).is_none());
        assert!(short_budget_note(AgentKind::Claude, TEAMMATE_TIMEOUT).is_none());
        assert!(short_budget_note(AgentKind::Codex, secs(30)).is_none());
    }

    /// OF04: a partial text over 32 KB keeps its end, after a line saying
    /// how many earlier bytes were dropped, so the closing Incomplete line
    /// always survives; the cut never splits a character; a short partial
    /// is untouched; the record's bytes are the capped text's.
    #[test]
    fn an_incomplete_result_caps_its_partial_text_keeping_the_end() {
        let secs = Duration::from_secs;
        let trailer = "[End of Claude's incomplete output: it was cut off at the deadline.]";
        // "é" is two bytes: an odd head makes the naive cut land mid-char.
        let partial = format!("x{}THE END", "é".repeat(PARTIAL_TEXT_MAX));
        let (kept, omitted) = capped_partial(&partial);
        assert!(kept.len() <= PARTIAL_TEXT_MAX);
        assert_eq!(omitted + kept.len(), partial.len());
        assert!(kept.ends_with("THE END"));
        let text = incomplete_text(AgentKind::Claude, secs(60), None, &partial, "");
        assert!(text.starts_with("Incomplete: Claude"), "{text:.200}");
        assert!(
            text.contains(&format!("[earlier {omitted} bytes omitted]\n\n")),
            "{text:.400}"
        );
        assert!(text.contains("THE END"));
        assert!(text.ends_with(trailer));
        assert!(text.len() < PARTIAL_TEXT_MAX + 1024);

        let short = incomplete_text(AgentKind::Claude, secs(60), None, "So far: a.rs.\n", "");
        assert!(!short.contains("bytes omitted"), "{short}");
        assert!(short.contains("So far: a.rs.\n\n[End of"), "{short}");

        let mut collected = agent::Collected::failed("Claude timed out".into());
        collected.timed_out = true;
        collected.partial_text = partial.clone();
        let (outcome, bytes) = consultation_outcome(&collected);
        assert_eq!(outcome, crate::delegation::Outcome::Timeout);
        assert_eq!(bytes, kept.len());
    }

    #[test]
    fn budget_warnings_lead_the_delegation_result() {
        assert_eq!(with_budget_warnings(&[], "done".into()), "done");
        let notice = budgets::backstop_notice(AgentKind::Claude);
        let text = with_budget_warnings(std::slice::from_ref(&notice), "done".into());
        assert_eq!(
            text,
            format!("[Budget warning: {}]\n\ndone", notice.message)
        );
    }
}
