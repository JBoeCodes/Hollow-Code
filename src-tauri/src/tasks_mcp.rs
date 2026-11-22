//! The `hollow_tasks` MCP stdio server (Paperclip PR 12): Hollow's task list
//! as tools for Claude and Codex chats and automations, modeled on
//! Paperclip's native-runner semantic action catalog.
//!
//! Task tools: `list_tasks`, `get_task`, `next_claimable` (reads), and
//! `create_task`, `update_task`, `set_status`, `claim_task`, `release_task`,
//! `add_comment`, `propose_tasks` (writes); plus the project tools.
//!
//! Claims (S09): `claim_task` moves an open, unclaimed task to `In progress`
//! with `claimed_by: <surface>:<scope>` (the chat session key or workflow
//! slug the broker's proof vouches for) and `claimed_at:` in its frontmatter,
//! as a compare-and-swap on the contentHash read just before, so of two
//! racing claims one wins and the other is `TASK_CONFLICT`. `release_task`
//! (owner only) hands it back. Tracker-owned statuses can't be claimed.
//!
//! **This process never writes the vault** (contract rule 12: a Hollow MCP
//! subprocess cannot share the GUI's in-process locks, so a write from here
//! would race the Tasks page, meeting and heartbeat accepts and tracker sync).
//! Reads go straight to the vault root named by `HOLLOW_VAULT_ROOT` (never
//! `HOLLOW_VAULT`, which other servers use for the run's cwd). Every write is
//! forwarded as `{op, args}` over the per-pid Unix socket named by
//! `HOLLOW_TASKS_SOCK` to the GUI's task broker (`task_broker.rs`), which
//! re-validates it and runs it through the same in-process writers the Tauri
//! commands use, inside the requesting run's live lease. The request carries
//! the run identity the GUI put in this server's environment
//! (`HOLLOW_RUN_ID` / `_SURFACE` / `_SCOPE`, `HOLLOW_TASKS_ACCESS`) and the
//! GUI-minted `HOLLOW_TASKS_PROOF` over them; nothing validated here is
//! trusted there. None of these names contain KEY, SECRET or TOKEN, which
//! Codex strips from an MCP server's environment.
//!
//! Access: a read-only server (chat in plan mode, or an automation whose
//! `allowed_tools` grant no write-capable built-in and name neither
//! `mcp__hollow_tasks` nor one of its write tools per tool) lists only
//! `list_tasks` and `get_task`, and its proof
//! cannot authorize a write.
//!
//! Launched as the hidden `hollow tasks-mcp` subcommand (see `main.rs`).
//! Attached by `mcp_attach` to Claude and Codex chats and automations while
//! Settings, Agent tools, Task tools is on (persisted in
//! `agent-tools.json`, loaded before the run-queue pump starts). Its tools
//! are preapproved (`agent::owned_preapprovals`; Codex gets
//! `default_tools_approval_mode="approve"`) because a headless CLI never
//! prompts. JSON-RPC shapes follow the other Hollow servers.

use crate::types::Task;
use crate::usage_ledger::{self, DelegateParent};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The server key in an mcpServers map (and Codex's `hollow_` prefix rule).
pub const SERVER_NAME: &str = "hollow_tasks";
/// The whole-server allowlist token, pushed into a tool-restricted
/// automation's `allowed_tools` beside `team_mcp::TEAM_TOOL_ALLOW`, and the
/// explicit opt-in that gives such an automation write access.
pub const TASKS_TOOL_ALLOW: &str = "mcp__hollow_tasks";
/// The broker socket the GUI bound.
pub const ENV_SOCK: &str = "HOLLOW_TASKS_SOCK";
/// The GUI-minted proof over the run identity and access.
pub const ENV_PROOF: &str = "HOLLOW_TASKS_PROOF";
/// `read` or `write`.
pub const ENV_ACCESS: &str = "HOLLOW_TASKS_ACCESS";

/// Default MCP protocol version echoed when a client doesn't send one.
const DEFAULT_PROTOCOL: &str = "2024-11-05";
/// `list_tasks` default and maximum page sizes.
const LIST_DEFAULT: usize = 50;
const LIST_MAX: usize = 200;
/// `get_task` returns this many trailing activity lines.
const ACTIVITY_TAIL: usize = 20;
/// How long a write waits for the GUI's answer.
const BROKER_TIMEOUT_SECS: u64 = 30;

/// Prompt guidance for a run with write access: byte-stable and path-free
/// (Claude caches the appended prompt), shared by the chat
/// append prompt and the workflow `<hollow-tasks>` block.
pub const PROMPT_GUIDANCE: &str = "Task tools (hollow_tasks): use these instead of editing tasks/ files. list_tasks/get_task return ref and contentHash; check duplicates before creating. create_task needs idempotencyKey, reused on retry. update_task requires expectedHash; on TASK_CONFLICT reread and reapply. set_status finishes/reopens (recurring tasks roll forward); pass expectedHash. add_comment records local progress. claim_task takes an open task; next_claimable checks blockers; release_task hands it back. Never coordinate with marker files. propose_tasks queues unrequested suggestions. Project tools manage records; sidebar projects use hollow_app list_workspaces/create_workspace. For tracker mirrors read tracker.issue, including writableFields, comments and conflicts; preserve provider identity and read-only fields. New tracked work uses tracker.create_task or tracker.create_project (no card; team AI, HOL for Hollow-only work; stable idempotencyKey). TASK_INVALID lists values. Anything for the person also appears in Hollow.";

/// Prompt guidance for a read-only run (plan mode, or a tool-restricted
/// automation). Byte-stable like [`PROMPT_GUIDANCE`].
pub const PROMPT_GUIDANCE_READ_ONLY: &str = "Task tools (hollow_tasks, read-only in this run): list_tasks, get_task, next_claimable, list_projects and get_project read task-project records. For Hollow sidebar projects use hollow_app list_workspaces. This run can't create, change or propose tasks; describe the change you would make in your reply instead.";

/// What a run's server may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TasksAccess {
    ReadOnly,
    ReadWrite,
}

impl TasksAccess {
    /// A chat turn: plan mode reads only; every other mode may write.
    pub fn for_chat(permission_mode: &str) -> Self {
        if permission_mode.trim() == "plan" {
            TasksAccess::ReadOnly
        } else {
            TasksAccess::ReadWrite
        }
    }

    /// An automation: plan mode reads only. A non-empty `allowed_tools`
    /// (a PR 5-stamped field) reads only unless it grants a write-capable
    /// built-in (`Write`, `Edit`, `MultiEdit`, `NotebookEdit`, `Bash`,
    /// `Bash(…)`), lists the whole server (`mcp__hollow_tasks`) or names one
    /// of its write tools (`mcp__hollow_tasks__create_task`, …), so an author
    /// who restricted a workflow to reading never silently hands it task
    /// writes. A per-tool read entry (`mcp__hollow_tasks__list_tasks`,
    /// `__get_task`) stays read-only.
    pub fn for_workflow(permission_mode: &str, allowed_tools: &[String]) -> Self {
        if permission_mode.trim() == "plan" {
            return TasksAccess::ReadOnly;
        }
        if allowed_tools.iter().all(|t| t.trim().is_empty()) {
            return TasksAccess::ReadWrite;
        }
        let grants_write = allowed_tools.iter().map(|t| t.trim()).any(|tool| {
            matches!(
                tool,
                "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "Bash"
            ) || tool.starts_with("Bash(")
                || tool == TASKS_TOOL_ALLOW
                || tool
                    .strip_prefix(concat!("mcp__hollow_tasks", "__"))
                    .is_some_and(|name| WRITE_TOOLS.contains(&name))
        });
        if grants_write {
            TasksAccess::ReadWrite
        } else {
            TasksAccess::ReadOnly
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TasksAccess::ReadOnly => "read",
            TasksAccess::ReadWrite => "write",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "read" => Some(TasksAccess::ReadOnly),
            "write" => Some(TasksAccess::ReadWrite),
            _ => None,
        }
    }
}

/// Settings, Agent tools, Task tools. Off until the GUI loads the persisted
/// switch (`task_broker::load_settings`, in setup before the run-queue pump
/// starts), so nothing attaches the server before the user's choice is known.
static TASKS_MCP_ENABLED: AtomicBool = AtomicBool::new(false);

/// Held by every test that sets the process-wide switch and depends on it
/// (or turns it off), so parallel tests don't see each other's value.
#[cfg(test)]
pub(crate) static SWITCH_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn set_enabled(enabled: bool) {
    TASKS_MCP_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    TASKS_MCP_ENABLED.load(Ordering::Relaxed)
}

/// The `hollow_tasks` entry for a run's mcpServers map, or `None` when the
/// switch is off, the GUI's broker isn't listening, or the executable path is
/// unknown. `vault_root` is the configured vault (never the run's cwd).
pub fn server_entry(
    vault_root: &Path,
    parent: &DelegateParent,
    access: TasksAccess,
) -> Option<(&'static str, Value)> {
    let (on, sock) = attach_state();
    if !on {
        return None;
    }
    let sock = sock?;
    let exe = std::env::current_exe().ok()?;
    let proof = crate::task_broker::proof_for(
        &parent.run_id,
        parent.surface.as_str(),
        &parent.scope,
        access,
    );
    Some(entry_for(&exe, &sock, vault_root, parent, access, &proof))
}

/// The switch and the broker's socket, as [`server_entry`] reads them. A test
/// on this thread may stand in for both ([`with_attach_override`]), so the
/// attachment rules in `mcp_attach` can be driven without binding the
/// process-global broker.
fn attach_state() -> (bool, Option<PathBuf>) {
    #[cfg(test)]
    if let Some(state) = ATTACH_OVERRIDE.with(|o| o.borrow().clone()) {
        return state;
    }
    (enabled(), crate::task_broker::socket_path())
}

#[cfg(test)]
thread_local! {
    static ATTACH_OVERRIDE: std::cell::RefCell<Option<(bool, Option<PathBuf>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` with this thread's switch and broker socket replaced (test only;
/// restored afterwards, also on panic).
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
    vault_root: &Path,
    parent: &DelegateParent,
    access: TasksAccess,
    proof: &str,
) -> (&'static str, Value) {
    let mut env = serde_json::Map::new();
    env.insert(
        usage_ledger::ENV_VAULT_ROOT.into(),
        json!(vault_root.to_string_lossy()),
    );
    env.insert(ENV_SOCK.into(), json!(sock.to_string_lossy()));
    env.insert(usage_ledger::ENV_RUN_ID.into(), json!(parent.run_id));
    env.insert(
        usage_ledger::ENV_RUN_SURFACE.into(),
        json!(parent.surface.as_str()),
    );
    env.insert(usage_ledger::ENV_RUN_SCOPE.into(), json!(parent.scope));
    env.insert(ENV_ACCESS.into(), json!(access.as_str()));
    env.insert(ENV_PROOF.into(), json!(proof));
    (
        SERVER_NAME,
        json!({
            "command": exe.to_string_lossy(),
            "args": ["tasks-mcp"],
            "env": env,
        }),
    )
}

// ── Broker client ────────────────────────────────────────────────────────

/// Forwards one write to the GUI's broker. The socket client is the only
/// production implementation; tests plug in an in-process one.
pub(crate) trait BrokerClient {
    fn call(&self, op: &str, args: &Value) -> Result<Value, String>;
}

/// Who a request is from, as the GUI stated it in this server's environment.
#[derive(Debug, Clone)]
pub(crate) struct RunIdentity {
    pub run_id: String,
    pub surface: String,
    pub scope: String,
    pub access: TasksAccess,
    pub proof: String,
}

impl RunIdentity {
    /// The one-line request the broker reads.
    pub(crate) fn request(&self, op: &str, args: &Value) -> Value {
        json!({
            "v": 1,
            "op": op,
            "runId": self.run_id,
            "surface": self.surface,
            "scope": self.scope,
            "access": self.access.as_str(),
            "proof": self.proof,
            "args": args,
        })
    }
}

/// The blocking Unix-socket client: one connection per request, one line
/// each way, a 30 s answer timeout.
#[cfg(unix)]
pub(crate) struct SocketClient {
    pub path: PathBuf,
    pub identity: RunIdentity,
}

#[cfg(unix)]
impl BrokerClient for SocketClient {
    fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        let mut stream = UnixStream::connect(&self.path).map_err(|e| {
            format!("Hollow's task broker is unavailable ({e}); nothing was written. Hollow may have quit.")
        })?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(BROKER_TIMEOUT_SECS)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let mut line = self.identity.request(op, args).to_string();
        line.push('\n');
        stream
            .write_all(line.as_bytes())
            .and_then(|_| stream.flush())
            .map_err(|e| {
                format!("Could not reach Hollow's task broker ({e}); nothing was written.")
            })?;
        let mut reader = BufReader::new(stream);
        let mut answer = String::new();
        match reader.read_line(&mut answer) {
            Ok(0) => Err("Hollow closed the connection before answering; the write may have completed. Retry with the same idempotencyKey, or read the task again, before writing again.".into()),
            Ok(_) => parse_broker_answer(&answer),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(format!(
                "Hollow did not answer within {BROKER_TIMEOUT_SECS} seconds; the write may have completed. Retry with the same idempotencyKey, or read the task again, before writing again."
            )),
            Err(e) => Err(format!("Reading Hollow's answer failed ({e}); the write may have completed. Read the task again before retrying.")),
        }
    }
}

/// `{ok: true, result}` → the result; `{ok: false, error}` → the error.
pub(crate) fn parse_broker_answer(line: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("Hollow's task broker sent an unreadable answer ({e})."))?;
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Hollow's task broker refused the request.")
            .to_string())
    }
}

// ── Server ───────────────────────────────────────────────────────────────

/// Everything one request needs.
pub(crate) struct Ctx<'a> {
    pub vault_root: Option<PathBuf>,
    pub access: TasksAccess,
    pub client: Option<&'a dyn BrokerClient>,
}

/// Entry point for the `tasks-mcp` subcommand: a synchronous JSON-RPC loop
/// over stdio until stdin closes. Blocking I/O is fine: the CLI sends one
/// request at a time, and SIGTERM (the GUI's finish sweep) ends the process
/// with nothing to clean up.
pub fn serve() {
    use std::io::{BufRead, Write};
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let vault_root = var(usage_ledger::ENV_VAULT_ROOT).map(PathBuf::from);
    let access = var(ENV_ACCESS)
        .and_then(|a| TasksAccess::parse(&a))
        .unwrap_or(TasksAccess::ReadOnly);
    #[cfg(unix)]
    let socket = match (
        var(ENV_SOCK),
        var(usage_ledger::ENV_RUN_ID),
        var(usage_ledger::ENV_RUN_SURFACE),
        var(usage_ledger::ENV_RUN_SCOPE),
        var(ENV_PROOF),
    ) {
        (Some(sock), Some(run_id), Some(surface), Some(scope), Some(proof))
            if access == TasksAccess::ReadWrite =>
        {
            Some(SocketClient {
                path: PathBuf::from(sock),
                identity: RunIdentity {
                    run_id,
                    surface,
                    scope,
                    access,
                    proof,
                },
            })
        }
        _ => None,
    };
    #[cfg(unix)]
    let client: Option<&dyn BrokerClient> = socket.as_ref().map(|c| c as &dyn BrokerClient);
    #[cfg(not(unix))]
    let client: Option<&dyn BrokerClient> = None;
    let ctx = Ctx {
        vault_root,
        access,
        client,
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for raw in stdin.lock().lines() {
        let Ok(raw) = raw else { break };
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(resp) = handle(&req, &ctx) {
            let mut out = stdout.lock();
            let mut s = resp.to_string();
            s.push('\n');
            if out
                .write_all(s.as_bytes())
                .and_then(|_| out.flush())
                .is_err()
            {
                break;
            }
        }
    }
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
                    "serverInfo": { "name": "hollow-tasks", "version": env!("CARGO_PKG_VERSION") },
                }),
            ))
        }
        "tools/list" => Some(result(id, json!({ "tools": tool_specs(ctx.access) }))),
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
                Ok(value) => tool_text(id, &pretty(&value)),
                Err(error) => tool_error(id, &error),
            })
        }
        "ping" => Some(result(id, json!({}))),
        _ => id.map(|id| error(Some(id), -32601, &format!("method not found: {method}"))),
    }
}

/// The list reads whose results go through the shared envelope (OB1,
/// overnight 11-17, `mcp_envelope`). Their rows are already headers (a
/// task's body is `get_task`), so a long list pages by bytes.
pub(crate) const ENVELOPED: [&str; 2] = ["list_tasks", "list_projects"];

/// An [`ENVELOPED`] tool's call: `maxBytes` and `cursor` taken out, the
/// list read as before, its text held to the budget, and the envelope as a
/// second text block.
fn call_enveloped(id: Option<Value>, name: &str, mut args: Value, ctx: &Ctx) -> Value {
    let req = match crate::mcp_envelope::Request::take(name, &mut args) {
        Ok(req) => req,
        Err(error) => return tool_error(id, &error),
    };
    let value = match call_tool(name, &args, ctx) {
        Ok(value) => value,
        Err(error) => return tool_error(id, &error),
    };
    let (pointer, more) = if name == "list_tasks" {
        (
            "/tasks",
            "More tasks match than limit: raise limit (at most 200) or narrow with query, status, project or ready.",
        )
    } else {
        (
            "/projects",
            "More projects match than limit: raise limit (at most 200) or narrow with query or status.",
        )
    };
    let row_limit = (value.get("truncated") == Some(&json!(true))).then_some(more);
    match crate::mcp_envelope::fit_json(&req, &value, pointer, false, None, row_limit, "", &pretty)
    {
        Ok(fitted) => {
            let mut answer = tool_text(id, &fitted.text);
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

/// The write tools; everything but `list_tasks` and `get_task`. Naming one
/// of them per tool in a workflow's `allowed_tools` also opts the automation
/// into write access ([`TasksAccess::for_workflow`]).
pub(crate) const WRITE_TOOLS: [&str; 10] = [
    "create_task",
    "update_task",
    "set_status",
    "claim_task",
    "release_task",
    "add_comment",
    "propose_tasks",
    "create_project",
    "update_project",
    "set_project_status",
];

fn call_tool(name: &str, args: &Value, ctx: &Ctx) -> Result<Value, String> {
    match name {
        "list_tasks" => list_tasks(root_of(ctx)?, args),
        "get_task" => get_task(root_of(ctx)?, args),
        "list_projects" => list_projects(root_of(ctx)?, args),
        "get_project" => get_project(root_of(ctx)?, args),
        "next_claimable" => next_claimable(root_of(ctx)?, args),
        write if WRITE_TOOLS.contains(&write) => {
            if ctx.access == TasksAccess::ReadOnly {
                return Err(
                    "This run can read tasks but not change them (plan mode, or an automation whose allowed tools don't include task writes). Nothing was written; describe the change in your reply instead."
                        .into(),
                );
            }
            let Some(client) = ctx.client else {
                return Err("Task writes are unavailable in this run: Hollow's task broker isn't connected. Nothing was written.".into());
            };
            if !args.is_object() {
                return Err(format!("{write}: arguments must be an object."));
            }
            if write == "claim_task" {
                return client.call(write, &with_read_hash(ctx, args));
            }
            client.call(write, args)
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

/// `claim_task`'s arguments with the contentHash of the task as read now
/// filled in as `expectedHash` (unless the caller sent one), so the broker's
/// claim is a compare-and-swap on this read. Without a readable vault or
/// ref the arguments go as they are; the broker still checks under its lock.
fn with_read_hash(ctx: &Ctx, args: &Value) -> Value {
    let sent = args
        .get("expectedHash")
        .and_then(Value::as_str)
        .is_some_and(|h| !h.trim().is_empty());
    let read = || {
        let vault = root_of(ctx).ok()?;
        let reference = args.get("ref").and_then(Value::as_str)?;
        let rel = crate::vault_tasks::resolve_task_ref(vault, reference).ok()?;
        let text = std::fs::read_to_string(vault.join(rel)).ok()?;
        Some(crate::vault_tasks::content_hash(&text))
    };
    let hash = if sent { None } else { read() };
    let mut args = args.clone();
    if let (Some(hash), Value::Object(map)) = (hash, &mut args) {
        map.insert("expectedHash".into(), json!(hash));
    }
    args
}

/// The open, unclaimed task whose blockers are all done, highest priority
/// then oldest first (`vault_tasks::next_claimable`), in `project` if given
/// (a name or projectUrl, as `list_tasks` matches it); `task: null` when none.
fn next_claimable(vault: &Path, args: &Value) -> Result<Value, String> {
    let project = match args.get("project") {
        None | Some(Value::Null) => None,
        Some(Value::String(p)) => Some(p.trim().to_lowercase()).filter(|p| !p.is_empty()),
        Some(_) => return Err("next_claimable: project must be a string.".into()),
    };
    let tasks: Vec<Task> = load_tasks(vault)
        .into_iter()
        .filter(|t| match project.as_deref() {
            None => true,
            Some(p) => {
                t.project_url
                    .as_deref()
                    .is_some_and(|u| u.to_lowercase() == p)
                    || t.project_name
                        .as_deref()
                        .is_some_and(|n| n.trim().to_lowercase() == p)
            }
        })
        .collect();
    let task = crate::vault_tasks::next_claimable(vault, tasks);
    Ok(json!({ "task": task.as_ref().map(compact) }))
}

fn root_of<'a>(ctx: &'a Ctx) -> Result<&'a Path, String> {
    ctx.vault_root
        .as_deref()
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            "Hollow's vault is unavailable to this server (HOLLOW_VAULT_ROOT is missing).".into()
        })
}

/// Every task, with readiness (`vault_tasks::list`) and tracker locks.
fn load_tasks(vault: &Path) -> Vec<Task> {
    let mut tasks = crate::vault_tasks::list(vault);
    crate::vault_tasks::mark_task_locks(vault, &mut tasks);
    tasks
}

fn list_tasks(vault: &Path, args: &Value) -> Result<Value, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let terms: Vec<String> = text("query")
        .map(|q| {
            q.to_lowercase()
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let status = text("status").map(str::to_lowercase);
    let project = text("project").map(str::to_lowercase);
    let ready = match args.get("ready") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("list_tasks: ready must be true or false.".into()),
    };
    let limit = match args.get("limit") {
        None | Some(Value::Null) => LIST_DEFAULT,
        Some(v) => v
            .as_u64()
            .filter(|n| *n >= 1)
            .map(|n| (n as usize).min(LIST_MAX))
            .ok_or("list_tasks: limit must be a positive whole number.")?,
    };
    let mut matched: Vec<Task> = load_tasks(vault)
        .into_iter()
        .filter(|t| match status.as_deref() {
            None => true,
            Some("open") => {
                !crate::vault_tasks::task_is_done(t) && !crate::vault_tasks::task_is_canceled(t)
            }
            Some("done") => crate::vault_tasks::task_is_done(t),
            Some(exact) => t.status.trim().to_lowercase() == exact,
        })
        .filter(|t| match project.as_deref() {
            None => true,
            Some(p) => {
                t.project_url
                    .as_deref()
                    .is_some_and(|u| u.to_lowercase() == p)
                    || t.project_name
                        .as_deref()
                        .is_some_and(|n| n.trim().to_lowercase() == p)
            }
        })
        .filter(|t| ready.is_none_or(|r| t.ready == r))
        .filter(|t| {
            if terms.is_empty() {
                return true;
            }
            let hay = format!("{}\n{}", t.title, t.description).to_lowercase();
            terms.iter().all(|term| hay.contains(term.as_str()))
        })
        .collect();
    matched.sort_by(|a, b| a.url.cmp(&b.url));
    let total = matched.len();
    let tasks: Vec<Value> = matched.iter().take(limit).map(compact).collect();
    Ok(json!({ "total": total, "truncated": total > limit, "tasks": tasks }))
}

fn get_task(vault: &Path, args: &Value) -> Result<Value, String> {
    let reference = args
        .get("ref")
        .and_then(Value::as_str)
        .ok_or("get_task: ref is required (a task's ref from list_tasks).")?;
    let rel = crate::vault_tasks::resolve_task_ref(vault, reference)?;
    let url = format!("vault:{rel}");
    let task = load_tasks(vault)
        .into_iter()
        .find(|t| t.url == url)
        .ok_or_else(|| format!("{rel} is not a readable task (it has no title)."))?;
    let mut full = compact(&task);
    let held: Vec<&str> = crate::vault_tasks::held_modal_fields(&task)
        .into_iter()
        .collect();
    let activity: Vec<&str> = task
        .activity
        .as_deref()
        .map(|a| {
            let lines: Vec<&str> = a.lines().filter(|l| !l.trim().is_empty()).collect();
            lines[lines.len().saturating_sub(ACTIVITY_TAIL)..].to_vec()
        })
        .unwrap_or_default();
    if let Value::Object(map) = &mut full {
        map.insert("description".into(), json!(task.description));
        map.insert(
            "subtasks".into(),
            json!(task
                .subtasks
                .iter()
                .map(|s| json!({ "text": s.text, "done": s.done }))
                .collect::<Vec<_>>()),
        );
        map.insert("doneWhen".into(), json!(task.done_when));
        map.insert("activity".into(), json!(activity));
        map.insert("section".into(), json!(task.section));
        map.insert("durationMin".into(), json!(task.duration_min));
        map.insert("blocks".into(), json!(task.blocks));
        map.insert("unresolvedBlockers".into(), json!(task.unresolved_blockers));
        map.insert(
            "blockerWarnings".into(),
            json!(task
                .blocker_warnings
                .iter()
                .map(|w| json!({ "url": w.url, "kind": w.kind }))
                .collect::<Vec<_>>()),
        );
        map.insert("heldFields".into(), json!(held));
        if let Some(tracker) = &task.tracker {
            let detail = crate::tracker_sync::seams::detail(vault, &rel, &tracker.id);
            map.insert(
                "tracker".into(),
                match detail {
                    Ok(detail) => json!(detail),
                    Err(error) => json!({"identity":tracker,"error":error}),
                },
            );
        }
        // A key on a task that isn't in progress is stale: no claim (S24).
        let claim = std::fs::read_to_string(vault.join(&rel))
            .ok()
            .and_then(|text| crate::vault_tasks::live_claim_of(&text, &task.status));
        map.insert(
            "claimedBy".into(),
            json!(claim.as_ref().map(|(owner, _)| owner)),
        );
        map.insert(
            "claimedAt".into(),
            json!(claim.as_ref().and_then(|(_, at)| at.as_ref())),
        );
    }
    Ok(full)
}

/// One project as `list_projects` returns it.
fn project_row(project: &crate::types::Project, text: &str) -> Value {
    json!({
        "ref": project.url,
        "title": project.name,
        "status": project.status,
        "priority": project.priority,
        "targetDate": project.target_date,
        "contentHash": crate::vault_tasks::content_hash(text),
    })
}

/// Every project note with its text, sorted by title.
fn load_projects(vault: &Path) -> Vec<(crate::types::Project, String)> {
    let mut out: Vec<_> = crate::vault_projects::project_files(vault)
        .into_iter()
        .filter_map(|(rel, path)| {
            let text = std::fs::read_to_string(&path).ok()?;
            let project = crate::vault_projects::parse(&rel, &text, vault)?;
            Some((project, text))
        })
        .collect();
    out.sort_by_key(|(p, _)| p.name.to_lowercase());
    out
}

fn list_projects(vault: &Path, args: &Value) -> Result<Value, String> {
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let terms: Vec<String> = text("query")
        .map(|q| {
            q.to_lowercase()
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let status = text("status").map(str::to_lowercase);
    let limit = match args.get("limit") {
        None | Some(Value::Null) => LIST_DEFAULT,
        Some(v) => v
            .as_u64()
            .filter(|n| *n >= 1)
            .map(|n| (n as usize).min(LIST_MAX))
            .ok_or("list_projects: limit must be a positive whole number.")?,
    };
    let rows: Vec<Value> = load_projects(vault)
        .iter()
        .filter(|(p, _)| {
            status
                .as_deref()
                .is_none_or(|s| p.status.trim().to_lowercase() == s)
        })
        .filter(|(p, _)| {
            let hay = format!("{}\n{}", p.name, p.description).to_lowercase();
            terms.iter().all(|t| hay.contains(t.as_str()))
        })
        .map(|(p, text)| project_row(p, text))
        .collect();
    let total = rows.len();
    let projects: Vec<Value> = rows.into_iter().take(limit).collect();
    Ok(json!({ "total": total, "truncated": total > limit, "projects": projects }))
}

fn get_project(vault: &Path, args: &Value) -> Result<Value, String> {
    let reference = args
        .get("ref")
        .and_then(Value::as_str)
        .ok_or("get_project: ref is required (a project's ref from list_projects).")?;
    let rel = crate::vault_projects::resolve_ref(vault, reference)?;
    let url = format!("vault:{rel}");
    let (project, text) = load_projects(vault)
        .into_iter()
        .find(|(p, _)| p.url == url)
        .ok_or_else(|| format!("{rel} is not a readable project (it has no title)."))?;
    let mut full = project_row(&project, &text);
    let open_tasks = crate::vault_tasks::list(vault)
        .iter()
        .filter(|t| t.project_url.as_deref() == Some(url.as_str()))
        .filter(|t| {
            !crate::vault_tasks::task_is_done(t) && !crate::vault_tasks::task_is_canceled(t)
        })
        .count();
    if let Value::Object(map) = &mut full {
        map.insert("description".into(), json!(project.description));
        map.insert("currentState".into(), json!(project.current_state));
        map.insert("desiredState".into(), json!(project.desired_state));
        map.insert("nonGoals".into(), json!(project.non_goals));
        map.insert("successEvidence".into(), json!(project.success_evidence));
        map.insert("goalUrls".into(), json!(project.goal_urls));
        map.insert("openTasks".into(), json!(open_tasks));
    }
    Ok(full)
}

/// The list row: identity, the fields an agent plans with, and the CAS
/// baseline.
fn compact(t: &Task) -> Value {
    json!({
        "ref": t.url,
        "title": t.title,
        "status": t.status,
        "priority": t.priority,
        "due": t.due,
        "dueTime": t.due_time,
        "recurrence": t.recurrence,
        "project": t.project_name,
        "projectUrl": t.project_url,
        "labels": t.labels,
        "ready": t.ready,
        "blockedBy": t.blocked_by,
        "contentHash": t.content_hash,
        "trackerLinked": t.tracker.is_some(),
        "tracker": t.tracker,
    })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

// ── Tool schemas ─────────────────────────────────────────────────────────

pub(crate) fn tool_specs(access: TasksAccess) -> Vec<Value> {
    let ref_prop = json!({ "type": "string", "description": "The task's ref as list_tasks returns it (vault:tasks/<name>.md)." });
    let project_ref_prop = json!({ "type": "string", "description": "The project's ref as list_projects returns it." });
    let hash_prop = json!({ "type": "string", "description": "The contentHash you last read for this task (list_tasks or get_task)." });
    let key_prop = json!({ "type": "string", "description": "A key you choose for this exact piece of work. Reuse it when you retry; use a new key for new work." });
    let mut tools = vec![
        json!({
            "name": "list_tasks",
            "description": "List Hollow's tasks with their ref, status, due date, readiness and contentHash. Filters combine.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words that must all appear in the title or description." },
                    "status": { "type": "string", "description": "open, done, or an exact status such as \"In progress\"." },
                    "project": { "type": "string", "description": "A project name or projectUrl." },
                    "ready": { "type": "boolean", "description": "true: not finished and every blocker done; false: the rest." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": LIST_MAX, "description": "At most this many (default 50)." }
                }
            }
        }),
        json!({
            "name": "get_task",
            "description": "Read one task in full: description, steps, Done when, recent activity, blockers, contentHash and the fields its tracker issue owns (heldFields).",
            "annotations": { "readOnlyHint": true },
            "inputSchema": { "type": "object", "properties": { "ref": ref_prop }, "required": ["ref"] }
        }),
        json!({
            "name": "list_projects",
            "description": "List task-project records with ref, status and contentHash. These are not Hollow's sidebar projects: use hollow_app list_workspaces for those. Filters combine.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words that must all appear in the title or description." },
                    "status": { "type": "string", "description": "An exact status such as \"In progress\"." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": LIST_MAX, "description": "At most this many (default 50)." }
                }
            }
        }),
        json!({
            "name": "get_project",
            "description": "Read one project in full: description, outcome fields, goal links, open task count and contentHash.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": { "type": "object", "properties": { "ref": project_ref_prop }, "required": ["ref"] }
        }),
        json!({
            "name": "next_claimable",
            "description": "The task to claim next: open, unclaimed and every blocker done, highest priority then oldest first. Returns {task: null} when none.",
            "annotations": { "readOnlyHint": true },
            "inputSchema": {
                "type": "object",
                "properties": { "project": { "type": "string", "description": "Only this project (a name or projectUrl)." } }
            }
        }),
    ];
    for tool in &mut tools {
        if tool["name"]
            .as_str()
            .is_some_and(|name| ENVELOPED.contains(&name))
        {
            crate::mcp_envelope::add_schema(tool);
        }
    }
    if access == TasksAccess::ReadOnly {
        return tools;
    }
    let fields = json!({
        "title": { "type": "string", "description": "One line, at most 240 characters." },
        "due": { "type": "string", "description": "YYYY-MM-DD." },
        "dueTime": { "type": "string", "description": "HH:MM, 24-hour." },
        "durationMin": { "type": "integer", "minimum": 1, "maximum": 1440 },
        "priority": { "type": "string", "description": "P1 (highest) to P4." },
        "recurrence": { "type": "string", "description": "A phrase such as \"every week\" or \"every weekday\"." },
        "section": { "type": "string" },
        "labels": { "type": "array", "items": { "type": "string" } },
        "description": { "type": "string" },
        "blockedBy": { "type": "array", "items": { "type": "string" }, "description": "Refs of the tasks this one waits on." }
    });
    let mut create_props = fields.as_object().cloned().unwrap_or_default();
    create_props.insert("idempotencyKey".into(), key_prop.clone());
    create_props.insert(
        "status".into(),
        json!({ "type": "string", "description": "Default \"Not started\"." }),
    );
    create_props.insert(
        "projectUrl".into(),
        json!({ "type": "string", "description": "A project note (vault:projects/…md)." }),
    );
    let mut update_fields = fields.as_object().cloned().unwrap_or_default();
    for (key, description) in [
        ("assignee", "Tracker member name or id; empty clears."),
        ("cycle", "Tracker cycle name or id; empty clears."),
        ("milestone", "Tracker milestone name or id; empty clears."),
        ("parentKey", "Tracker parent key or id; empty clears."),
        ("project", "Tracker project name or id; empty clears."),
    ] {
        update_fields.insert(
            key.into(),
            json!({"type":"string","description":description}),
        );
    }
    update_fields.insert(
        "estimate".into(),
        json!({"type":["number","null"],"description":"Tracker estimate; null clears."}),
    );
    update_fields.insert(
        "projectUrl".into(),
        json!({ "type": "string", "description": "Move the task to this project (a ref from list_projects)." }),
    );
    update_fields.insert(
        "subtasks".into(),
        json!({ "type": "array", "items": { "type": "object", "properties": { "text": { "type": "string" }, "done": { "type": "boolean" } }, "required": ["text"] }, "description": "Replaces the task's steps." }),
    );
    tools.extend([
        json!({
            "name": "create_task",
            "description": "Create a local task. The same idempotencyKey again returns it (disposition duplicate); reused for other work: IDEMPOTENCY_MISMATCH.",
            "inputSchema": { "type": "object", "properties": create_props, "required": ["idempotencyKey", "title"] }
        }),
        json!({
            "name": "update_task",
            "description": "Change fields of a task (an empty string clears a text field). Fails with TASK_CONFLICT when the task changed since you read expectedHash, TASK_INVALID for a value that isn't allowed, and TASK_LINKED for a field its tracker issue holds (heldFields in get_task). Use set_status for the status.",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": ref_prop, "expectedHash": hash_prop, "fields": { "type": "object", "properties": update_fields } },
                "required": ["ref", "expectedHash", "fields"]
            }
        }),
        json!({
            "name": "set_status",
            "description": "Set a task's status (Done finishes it; a recurring task rolls forward to its next date instead, and then expectedHash is required).",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": ref_prop, "status": { "type": "string" }, "expectedHash": hash_prop },
                "required": ["ref", "status"]
            }
        }),
        json!({
            "name": "claim_task",
            "description": "Claim an open task for this conversation or automation so no other agent starts it: moves it to In progress and records you as claimedBy. already: true means this conversation or automation already held the claim (nothing changed). Fails with TASK_CLAIMED (naming the owner) when someone already holds it or it isn't open, TASK_CONFLICT when another claim landed first (read it again), and TASK_LINKED when its tracker issue owns its status.",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": ref_prop, "expectedHash": hash_prop },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "release_task",
            "description": "Hand back a task you claimed: clears claimedBy and returns an In progress task to Not started. Only the claim's owner can release it (TASK_CLAIMED otherwise).",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": ref_prop, "expectedHash": hash_prop },
                "required": ["ref"]
            }
        }),
        json!({
            "name": "add_comment",
            "description": "Append a timestamped entry to the task's ## Activity section. With an idempotencyKey a retry writes nothing twice.",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": ref_prop, "body": { "type": "string", "description": "At most 4,000 characters and 40 lines." }, "idempotencyKey": key_prop, "expectedHash": hash_prop },
                "required": ["ref", "body"]
            }
        }),
        json!({
            "name": "propose_tasks",
            "description": "Suggest tasks for the user to review in Tasks → Task suggestions. Creates nothing. At most 10 per call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "items": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string" },
                                "due": { "type": "string", "description": "YYYY-MM-DD." },
                                "priority": { "type": "string", "description": "P1 to P4." },
                                "note": { "type": "string", "description": "Why this task, in a sentence or two." },
                                "projectUrl": { "type": "string" },
                                "labels": { "type": "array", "items": { "type": "string" } },
                                "evidence": { "type": "string", "description": "A short quote or path that supports it." }
                            },
                            "required": ["title"]
                        }
                    }
                },
                "required": ["items"]
            }
        }),
    ]);
    let project_fields = json!({
        "title": { "type": "string", "description": "One line." },
        "priority": { "type": "string", "description": "P1 (highest) to P4, or high/medium/low." },
        "targetDate": { "type": "string", "description": "YYYY-MM-DD." },
        "description": { "type": "string" },
        "currentState": { "type": "string", "description": "Where it stands now." },
        "desiredState": { "type": "string", "description": "Where it should end up." },
        "nonGoals": { "type": "string" },
        "successEvidence": { "type": "string", "description": "How you'll know it worked." },
        "goalUrls": { "type": "array", "items": { "type": "string" }, "description": "Goals it serves (vault:goals/….md); [] clears." }
    });
    let mut create_project_props = project_fields.as_object().cloned().unwrap_or_default();
    create_project_props.insert("idempotencyKey".into(), key_prop.clone());
    create_project_props.insert(
        "status".into(),
        json!({ "type": "string", "description": "Default \"discovery\"." }),
    );
    tools.extend([
        json!({
            "name": "create_project",
            "description": "Create a legacy task-project note in an existing projects/ folder. For a local Hollow sidebar project use hollow_app create_workspace {title, goal}. This tool does not create sidebar projects. The same idempotencyKey again returns the note.",
            "inputSchema": { "type": "object", "properties": create_project_props, "required": ["idempotencyKey", "title"] }
        }),
        json!({
            "name": "update_project",
            "description": "Change fields of a project (an empty string clears a text field). Fails with TASK_CONFLICT when the project changed since you read expectedHash and TASK_INVALID for a value that isn't allowed. Use set_project_status for the status.",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": project_ref_prop, "expectedHash": hash_prop, "fields": { "type": "object", "properties": project_fields } },
                "required": ["ref", "expectedHash", "fields"]
            }
        }),
        json!({
            "name": "set_project_status",
            "description": "Set a project's status; a project note moves to that status's folder.",
            "inputSchema": {
                "type": "object",
                "properties": { "ref": project_ref_prop, "status": { "type": "string" }, "expectedHash": hash_prop },
                "required": ["ref", "status"]
            }
        }),
    ]);
    tools
}

// ── JSON-RPC helpers (same shapes as the other Hollow servers) ───────────

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

/// A refusal: the readable sentence first, as before, then its code and
/// next call (BI5, `mcp_envelope::classify`).
fn tool_error(id: Option<Value>, text: &str) -> Value {
    let content = crate::mcp_envelope::refusal_content(crate::mcp_envelope::Server::Tasks, text);
    result(id, json!({ "content": content, "isError": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_ledger::Surface;
    use std::cell::RefCell;

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

    fn tool_names(access: TasksAccess) -> Vec<String> {
        tool_specs(access)
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    fn required(access: TasksAccess, name: &str) -> Vec<String> {
        let spec = tool_specs(access)
            .into_iter()
            .find(|t| t["name"] == json!(name))
            .unwrap();
        spec["inputSchema"]["required"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    }

    /// A vault root that `walk`-based readers don't prune (a dot-prefixed
    /// temp root would be skipped).
    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn put(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// Records every forwarded call and answers with a fixed value.
    struct Recorder {
        calls: RefCell<Vec<(String, Value)>>,
    }

    impl BrokerClient for Recorder {
        fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
            self.calls.borrow_mut().push((op.to_string(), args.clone()));
            Ok(json!({ "forwarded": op }))
        }
    }

    #[test]
    fn tools_list_matches_access() {
        assert_eq!(
            tool_names(TasksAccess::ReadWrite),
            [
                "list_tasks",
                "get_task",
                "list_projects",
                "get_project",
                "next_claimable",
                "create_task",
                "update_task",
                "set_status",
                "claim_task",
                "release_task",
                "add_comment",
                "propose_tasks",
                "create_project",
                "update_project",
                "set_project_status"
            ]
        );
        assert_eq!(
            tool_names(TasksAccess::ReadOnly),
            [
                "list_tasks",
                "get_task",
                "list_projects",
                "get_project",
                "next_claimable"
            ]
        );
        let rw = TasksAccess::ReadWrite;
        assert_eq!(required(rw, "get_task"), ["ref"]);
        assert_eq!(required(rw, "create_task"), ["idempotencyKey", "title"]);
        assert_eq!(
            required(rw, "update_task"),
            ["ref", "expectedHash", "fields"]
        );
        assert_eq!(required(rw, "set_status"), ["ref", "status"]);
        assert_eq!(required(rw, "claim_task"), ["ref"]);
        assert_eq!(required(rw, "release_task"), ["ref"]);
        assert!(required(rw, "next_claimable").is_empty());
        assert_eq!(required(rw, "add_comment"), ["ref", "body"]);
        assert_eq!(required(rw, "propose_tasks"), ["items"]);
        assert_eq!(required(rw, "get_project"), ["ref"]);
        assert_eq!(required(rw, "create_project"), ["idempotencyKey", "title"]);
        assert_eq!(
            required(rw, "update_project"),
            ["ref", "expectedHash", "fields"]
        );
        assert_eq!(required(rw, "set_project_status"), ["ref", "status"]);
        assert!(required(rw, "list_tasks").is_empty());

        let ctx = Ctx {
            vault_root: None,
            access: TasksAccess::ReadOnly,
            client: None,
        };
        let listed = handle(&req(2, "tools/list", Value::Null), &ctx).unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 5);
        let init = handle(
            &req(3, "initialize", json!({ "protocolVersion": "2025-06-18" })),
            &ctx,
        )
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], json!("hollow-tasks"));
        assert!(handle(
            &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            &ctx
        )
        .is_none());
        let unknown = handle(
            &json!({ "jsonrpc": "2.0", "id": 9, "method": "nope" }),
            &ctx,
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], json!(-32601));
    }

    #[test]
    fn workflow_access_follows_allowed_tools() {
        let list = |tools: &[&str]| tools.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        use TasksAccess::*;
        assert_eq!(TasksAccess::for_workflow("plan", &[]), ReadOnly);
        assert_eq!(
            TasksAccess::for_workflow("plan", &list(&["Write"])),
            ReadOnly
        );
        assert_eq!(TasksAccess::for_workflow("acceptEdits", &[]), ReadWrite);
        assert_eq!(TasksAccess::for_workflow("", &[]), ReadWrite);
        assert_eq!(
            TasksAccess::for_workflow("acceptEdits", &list(&["Read", "Grep"])),
            ReadOnly
        );
        assert_eq!(
            TasksAccess::for_workflow("acceptEdits", &list(&["Read", "Write"])),
            ReadWrite
        );
        assert_eq!(
            TasksAccess::for_workflow("acceptEdits", &list(&["Bash(npm:*)"])),
            ReadWrite
        );
        assert_eq!(
            TasksAccess::for_workflow("acceptEdits", &list(&["Read", "mcp__hollow_tasks"])),
            ReadWrite
        );
        assert_eq!(
            TasksAccess::for_workflow("acceptEdits", &list(&["Read", "mcp__hollow_team"])),
            ReadOnly
        );
        // Per tool: a read tool alone stays read-only; a write tool opts in.
        assert_eq!(
            TasksAccess::for_workflow(
                "acceptEdits",
                &list(&["Read", "mcp__hollow_tasks__list_tasks"])
            ),
            ReadOnly
        );
        assert_eq!(
            TasksAccess::for_workflow("acceptEdits", &list(&["mcp__hollow_tasks__get_task"])),
            ReadOnly
        );
        assert_eq!(
            TasksAccess::for_workflow("acceptEdits", &list(&["mcp__hollow_tasks__nonsense"])),
            ReadOnly
        );
        // WRITE_TOOLS is exactly what a read-write server adds.
        let names = |access| {
            tool_specs(access)
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        let (read, write) = (names(ReadOnly), names(ReadWrite));
        let added: Vec<&str> = write
            .iter()
            .map(String::as_str)
            .filter(|n| !read.iter().any(|r| r == n))
            .collect();
        assert_eq!(added, WRITE_TOOLS);
        for tool in WRITE_TOOLS {
            let entry = format!("mcp__hollow_tasks__{tool}");
            assert_eq!(
                TasksAccess::for_workflow("acceptEdits", &list(&["Read", entry.as_str()])),
                ReadWrite,
                "{tool}"
            );
        }
        assert_eq!(TasksAccess::for_chat("plan"), ReadOnly);
        assert_eq!(TasksAccess::for_chat("acceptEdits"), ReadWrite);
        assert_eq!(TasksAccess::for_chat("bypassPermissions"), ReadWrite);
        for access in [ReadOnly, ReadWrite] {
            assert_eq!(TasksAccess::parse(access.as_str()), Some(access));
        }
        assert_eq!(TasksAccess::parse("admin"), None);
    }

    #[test]
    fn writes_without_a_broker_are_errors_never_direct_writes() {
        let dir = vault();
        let v = dir.path();
        std::fs::create_dir_all(v.join("tasks")).unwrap();
        let brokerless = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadWrite,
            client: None,
        };
        let created = call(
            "create_task",
            json!({ "idempotencyKey": "k", "title": "Draft it" }),
            &brokerless,
        );
        assert!(is_error(&created), "{created}");
        assert!(text_of(&created).contains("Nothing was written"));
        assert_eq!(std::fs::read_dir(v.join("tasks")).unwrap().count(), 0);

        // A read-only server refuses every write tool, even with a client.
        let recorder = Recorder {
            calls: RefCell::new(Vec::new()),
        };
        let read_only = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadOnly,
            client: Some(&recorder),
        };
        for tool in WRITE_TOOLS {
            let resp = call(tool, json!({}), &read_only);
            assert!(is_error(&resp), "{tool}: {resp}");
            assert!(text_of(&resp).contains("can read tasks but not change them"));
        }
        assert!(recorder.calls.borrow().is_empty());

        // With write access every write is forwarded verbatim, never done here.
        let writer = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadWrite,
            client: Some(&recorder),
        };
        let resp = call(
            "set_status",
            json!({ "ref": "vault:tasks/a.md", "status": "Done" }),
            &writer,
        );
        assert!(!is_error(&resp), "{resp}");
        assert_eq!(
            recorder.calls.borrow()[0],
            (
                "set_status".to_string(),
                json!({ "ref": "vault:tasks/a.md", "status": "Done" })
            )
        );
        assert!(is_error(&call("drop_table", json!({}), &writer)));
        assert_eq!(std::fs::read_dir(v.join("tasks")).unwrap().count(), 0);
    }

    /// S09: claim_task is forwarded with the contentHash read just before
    /// (a caller's own expectedHash wins), and next_claimable is a local read
    /// that skips claimed and blocked tasks and filters by project.
    #[test]
    fn claim_task_forwards_the_read_hash_and_next_claimable_reads_locally() {
        let dir = vault();
        let v = dir.path();
        let open = "---\ntitle: Open\nstatus: Not started\nproject: Ops\n---\n";
        put(v, "tasks/open.md", open);
        put(
            v,
            "tasks/waits.md",
            "---\ntitle: Waits\nstatus: Not started\npriority: P1\nblocked_by: \"vault:tasks/open.md\"\n---\n",
        );
        put(
            v,
            "tasks/held.md",
            "---\ntitle: Held\nstatus: In progress\npriority: P1\nclaimed_by: \"chat:s-9\"\n---\n",
        );
        let recorder = Recorder {
            calls: RefCell::new(Vec::new()),
        };
        let writer = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadWrite,
            client: Some(&recorder),
        };
        let open_ref = json!({ "ref": "vault:tasks/open.md" });
        let resp = call("claim_task", open_ref, &writer);
        assert!(!is_error(&resp), "{resp}");
        let pinned = json!({ "ref": "vault:tasks/open.md", "expectedHash": "abc" });
        assert!(!is_error(&call("claim_task", pinned.clone(), &writer)));
        let hash = crate::vault_tasks::content_hash(open);
        let read = json!({ "ref": "vault:tasks/open.md", "expectedHash": hash });
        assert_eq!(
            *recorder.calls.borrow(),
            [
                ("claim_task".to_string(), read),
                ("claim_task".to_string(), pinned)
            ]
        );
        let next = |args: Value| {
            let resp = call("next_claimable", args, &writer);
            assert!(!is_error(&resp), "{resp}");
            let value: Value = serde_json::from_str(&text_of(&resp)).unwrap();
            value["task"]["ref"].clone()
        };
        // The P1 tasks are blocked and claimed; the open one is next.
        let first = json!("vault:tasks/open.md");
        assert_eq!(next(json!({})), first);
        assert_eq!(next(json!({ "project": "ops" })), first);
        assert_eq!(next(json!({ "project": "Elsewhere" })), Value::Null);
        let held_ref = json!({ "ref": "vault:tasks/held.md" });
        let held = text_of(&call("get_task", held_ref, &writer));
        let held: Value = serde_json::from_str(&held).unwrap();
        assert_eq!(held["claimedBy"], json!("chat:s-9"));
        assert_eq!(recorder.calls.borrow().len(), 2, "reads forward nothing");
    }

    /// S24: a `claimed_by` left on a task that isn't in progress is stale,
    /// so get_task reports no claim.
    #[test]
    fn get_task_reads_a_stale_claim_key_as_no_claim() {
        let dir = vault();
        let v = dir.path();
        put(
            v,
            "tasks/stale.md",
            "---\ntitle: Stale\nstatus: Not started\nclaimed_by: \"chat:gone\"\nclaimed_at: \"2026-10-01T09:00:00+00:00\"\n---\n",
        );
        let reader = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadOnly,
            client: None,
        };
        let resp = call(
            "get_task",
            json!({ "ref": "vault:tasks/stale.md" }),
            &reader,
        );
        assert!(!is_error(&resp), "{resp}");
        let task: Value = serde_json::from_str(&text_of(&resp)).unwrap();
        assert_eq!(task["claimedBy"], Value::Null);
        assert_eq!(task["claimedAt"], Value::Null);
    }

    /// S24: `src/workflowPreview.ts` mirrors [`WRITE_TOOLS`] to decide an
    /// automation's task access; the two lists must match, in order.
    #[test]
    fn workflow_preview_task_write_tools_match_write_tools() {
        let ts = include_str!("../../src/workflowPreview.ts");
        let start = ts
            .find("const TASK_WRITE_TOOLS")
            .expect("src/workflowPreview.ts declares TASK_WRITE_TOOLS");
        let rest = &ts[start..];
        let open = rest.find('[').expect("TASK_WRITE_TOOLS is an array");
        let close = rest.find(']').expect("TASK_WRITE_TOOLS is an array");
        let names: Vec<&str> = rest[open + 1..close]
            .split(',')
            .map(|n| n.trim().trim_matches(|c: char| c == '\'' || c == '"'))
            .filter(|n| !n.is_empty())
            .collect();
        assert_eq!(
            names, WRITE_TOOLS,
            "src/workflowPreview.ts TASK_WRITE_TOOLS"
        );
    }

    #[test]
    fn list_and_get_read_the_vault_root_directly() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-plan.md", "---\ntitle: Draft the rollout plan\nstatus: In progress\npriority: P1\nproject: Launch\nproject_url: vault:projects/active/Launch.md\n---\n\nPlan the rollout in phases.\n\n- [ ] outline\n- [x] owners\n\n## Done when\n\nSigned off.\n\n## Activity\n\n- 2026-09-20 10:00 you: started\n");
        put(v, "tasks/ship-it.md", "---\ntitle: Ship it\nstatus: Not started\nblocked_by: vault:tasks/draft-plan.md\n---\n");
        put(
            v,
            "tasks/old.md",
            "---\ntitle: Old rollout notes\nstatus: Done\n---\n",
        );
        let ctx = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadOnly,
            client: None,
        };
        let listed = |args: Value| -> Value {
            let resp = call("list_tasks", args, &ctx);
            assert!(!is_error(&resp), "{resp}");
            serde_json::from_str(&text_of(&resp)).unwrap()
        };
        let refs = |value: &Value| -> Vec<String> {
            value["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["ref"].as_str().unwrap().to_string())
                .collect()
        };

        assert_eq!(listed(json!({}))["total"], json!(3));
        assert_eq!(
            refs(&listed(json!({ "status": "open" }))),
            ["vault:tasks/draft-plan.md", "vault:tasks/ship-it.md"]
        );
        assert_eq!(
            refs(&listed(json!({ "status": "done" }))),
            ["vault:tasks/old.md"]
        );
        assert_eq!(
            refs(&listed(json!({ "status": "in progress" }))),
            ["vault:tasks/draft-plan.md"]
        );
        assert_eq!(
            refs(&listed(json!({ "project": "launch" }))),
            ["vault:tasks/draft-plan.md"]
        );
        assert_eq!(
            refs(&listed(
                json!({ "project": "vault:projects/active/Launch.md" })
            )),
            ["vault:tasks/draft-plan.md"]
        );
        // Every term, in any order, across title and description.
        assert_eq!(
            refs(&listed(json!({ "query": "phases rollout" }))),
            ["vault:tasks/draft-plan.md"]
        );
        assert_eq!(refs(&listed(json!({ "query": "rollout" }))).len(), 2);
        // Readiness comes from the task list (PR 11b): Ship it waits on an open task.
        assert_eq!(
            refs(&listed(json!({ "ready": true }))),
            ["vault:tasks/draft-plan.md"]
        );
        assert!(refs(&listed(json!({ "ready": false })))
            .contains(&"vault:tasks/ship-it.md".to_string()));
        let one = listed(json!({ "limit": 1 }));
        assert_eq!(one["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(one["truncated"], json!(true));
        assert!(is_error(&call("list_tasks", json!({ "limit": 0 }), &ctx)));

        let got = call("get_task", json!({ "ref": "tasks/draft-plan.md" }), &ctx);
        assert!(!is_error(&got), "{got}");
        let task: Value = serde_json::from_str(&text_of(&got)).unwrap();
        assert_eq!(task["ref"], json!("vault:tasks/draft-plan.md"));
        assert_eq!(task["description"], json!("Plan the rollout in phases."));
        assert_eq!(
            task["subtasks"],
            json!([{ "text": "outline", "done": false }, { "text": "owners", "done": true }])
        );
        assert_eq!(task["doneWhen"], json!("Signed off."));
        assert_eq!(task["activity"], json!(["- 2026-09-20 10:00 you: started"]));
        assert_eq!(task["blocks"], json!(["vault:tasks/ship-it.md"]));
        assert_eq!(task["heldFields"], json!([]));
        let hash = crate::vault_tasks::content_hash(
            &std::fs::read_to_string(v.join("tasks/draft-plan.md")).unwrap(),
        );
        assert_eq!(task["contentHash"], json!(hash));
        assert!(is_error(&call(
            "get_task",
            json!({ "ref": "vault:../x.md" }),
            &ctx
        )));
        assert!(is_error(&call("get_task", json!({}), &ctx)));
        let rootless = Ctx {
            vault_root: None,
            access: TasksAccess::ReadOnly,
            client: None,
        };
        assert!(text_of(&call("list_tasks", json!({}), &rootless)).contains("HOLLOW_VAULT_ROOT"));
    }

    #[test]
    fn server_entry_carries_the_vault_root_and_run_identity() {
        let parent = DelegateParent {
            surface: Surface::Workflow,
            scope: "brief".into(),
            run_id: "R1".into(),
            scratch: None,
            tool_limits: Default::default(),
        };
        let (name, entry) = entry_for(
            Path::new("/Applications/Hollow.app/Contents/MacOS/hollow"),
            Path::new("/tmp/hollow/tasks-7.sock"),
            Path::new("/vault"),
            &parent,
            TasksAccess::ReadWrite,
            "abc123",
        );
        assert_eq!(name, "hollow_tasks");
        assert_eq!(entry["args"], json!(["tasks-mcp"]));
        let env = entry["env"].as_object().unwrap();
        assert_eq!(env["HOLLOW_VAULT_ROOT"], json!("/vault"));
        assert!(
            !env.contains_key("HOLLOW_VAULT"),
            "the vault root, never the run's cwd"
        );
        assert_eq!(env["HOLLOW_TASKS_SOCK"], json!("/tmp/hollow/tasks-7.sock"));
        assert_eq!(env["HOLLOW_RUN_ID"], json!("R1"));
        assert_eq!(env["HOLLOW_RUN_SURFACE"], json!("workflow"));
        assert_eq!(env["HOLLOW_RUN_SCOPE"], json!("brief"));
        assert_eq!(env["HOLLOW_TASKS_ACCESS"], json!("write"));
        assert_eq!(env["HOLLOW_TASKS_PROOF"], json!("abc123"));
        for key in env.keys() {
            for filtered in ["KEY", "SECRET", "TOKEN"] {
                assert!(!key.contains(filtered), "Codex strips {key}");
            }
        }
        // Without the GUI's broker (no socket in this test process) or with
        // the switch off, nothing attaches.
        assert!(server_entry(Path::new("/vault"), &parent, TasksAccess::ReadWrite).is_none());
    }

    #[test]
    fn broker_answers_are_unwrapped_and_refusals_become_errors() {
        assert_eq!(
            parse_broker_answer("{\"ok\":true,\"result\":{\"a\":1}}\n").unwrap(),
            json!({ "a": 1 })
        );
        assert_eq!(
            parse_broker_answer("{\"ok\":false,\"error\":\"TASK_CONFLICT: x\"}").unwrap_err(),
            "TASK_CONFLICT: x"
        );
        assert!(parse_broker_answer("not json")
            .unwrap_err()
            .contains("unreadable"));
        let identity = RunIdentity {
            run_id: "R".into(),
            surface: "chat".into(),
            scope: "s-1".into(),
            access: TasksAccess::ReadWrite,
            proof: "p".into(),
        };
        let request = identity.request("add_comment", &json!({ "ref": "tasks/a.md" }));
        assert_eq!(
            request,
            json!({ "v": 1, "op": "add_comment", "runId": "R", "surface": "chat", "scope": "s-1", "access": "write", "proof": "p", "args": { "ref": "tasks/a.md" } })
        );
    }

    #[test]
    fn prompt_guidance_is_path_free_and_names_the_rules() {
        for guidance in [PROMPT_GUIDANCE, PROMPT_GUIDANCE_READ_ONLY] {
            assert!(guidance.starts_with("Task tools (hollow_tasks"));
            assert!(
                !guidance.contains('/') || guidance.contains("tasks/"),
                "no paths: {guidance}"
            );
            assert!(!guidance.contains('\n'));
        }
        for rule in [
            "idempotencyKey",
            "expectedHash",
            "TASK_CONFLICT",
            "propose_tasks",
            "set_status",
            "add_comment",
            "claim_task",
            "next_claimable",
            "release_task",
        ] {
            assert!(PROMPT_GUIDANCE.contains(rule), "{rule}");
        }
        assert!(PROMPT_GUIDANCE_READ_ONLY.contains("read-only"));
    }

    /// The subprocess reads the vault and forwards writes; it never writes
    /// the vault or the review queue itself, never reaches history, and never
    /// calls the broker's in-process entry points. Needles are built with
    /// `concat!` so this file never matches itself (agent.rs's
    /// `OWNED_MCP_SOURCES` scans it for the same substrings).
    #[test]
    fn tasks_mcp_never_writes_the_vault_itself() {
        let source = include_str!("tasks_mcp.rs");
        for needle in [
            concat!("vault::", "mutate_note"),
            concat!("write_", "note"),
            concat!("atomic_", "write"),
            concat!("create_", "note"),
            concat!("review_", "queue::"),
            concat!("vault_tasks::", "upsert"),
            concat!("vault_tasks::", "mutate"),
            concat!("vault_tasks::", "set_status"),
            concat!("vault_tasks::", "apply_update"),
            concat!("vault_tasks::", "create_from_input"),
            concat!("vault_tasks::", "create_agent_task"),
            concat!("vault_tasks::", "append_activity"),
            concat!("vault_tasks::", "claim_task"),
            concat!("vault_tasks::", "release_task"),
            concat!("task_broker::", "dispatch"),
            concat!("task_broker::", "start"),
            concat!("automated_", "history::"),
        ] {
            assert!(
                !source.contains(needle),
                "tasks_mcp.rs must not contain `{needle}`"
            );
        }
    }

    /// OB1: `list_tasks` pages by bytes under `maxBytes` with a cursor, each
    /// task once; a list cut by its own `limit` says `row_limit`; the
    /// envelope is a second text block.
    #[test]
    fn list_tasks_pages_under_its_budget_and_says_when_limit_cut_it() {
        let dir = vault();
        let v = dir.path();
        let long = "word ".repeat(40);
        for i in 0..60 {
            put(
                v,
                &format!("tasks/t{i:02}.md"),
                &format!(
                    "---\ntitle: Task {i:02} {long}\nstatus: Not started\nlabels: [a, b, c]\n---\n"
                ),
            );
        }
        let ctx = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadOnly,
            client: None,
        };
        let envelope_of = |resp: &Value| -> Value {
            let blocks = resp["result"]["content"].as_array().unwrap();
            assert_eq!(blocks.len(), 2, "{resp}");
            let env: Value = serde_json::from_str(blocks[1]["text"].as_str().unwrap()).unwrap();
            env["envelope"].clone()
        };
        let first = call(
            "list_tasks",
            json!({ "limit": 60, "maxBytes": 8_000 }),
            &ctx,
        );
        assert!(text_of(&first).len() <= 8_000);
        let env = envelope_of(&first);
        assert_eq!(env["capReasons"], json!(["byte_budget"]), "{env}");
        let mut seen: Vec<String> = Vec::new();
        let mut take = |resp: &Value| {
            let page: Value = serde_json::from_str(&text_of(resp)).unwrap();
            for t in page["tasks"].as_array().unwrap() {
                seen.push(t["ref"].as_str().unwrap().to_string());
            }
        };
        take(&first);
        let mut cursor = env["nextCursor"].as_str().map(str::to_string);
        while let Some(c) = cursor {
            let next = call(
                "list_tasks",
                json!({ "limit": 60, "maxBytes": 8_000, "cursor": c }),
                &ctx,
            );
            assert!(!is_error(&next), "{next}");
            take(&next);
            cursor = envelope_of(&next)["nextCursor"]
                .as_str()
                .map(str::to_string);
        }
        assert_eq!(seen.len(), 60);
        seen.dedup();
        assert_eq!(seen.len(), 60, "each task once");
        // The default limit (50) cut the list: row_limit, text unchanged
        // when it fits the budget it asked for.
        let cut = call("list_tasks", json!({ "maxBytes": 80_000 }), &ctx);
        let env = envelope_of(&cut);
        assert_eq!(env["capReasons"], json!(["row_limit"]), "{env}");
        assert_eq!(env["complete"], json!(false));
        let plain = call("list_tasks", json!({ "limit": 1 }), &ctx);
        let value: Value = serde_json::from_str(&text_of(&plain)).unwrap();
        assert_eq!(
            text_of(&plain),
            pretty(&value),
            "under budget: the old text"
        );
        // A cursor from list_projects is refused here.
        let mut probe = json!({ "limit": 60 });
        let other = crate::mcp_envelope::Request::take("list_projects", &mut probe).unwrap();
        let resp = call(
            "list_tasks",
            json!({ "limit": 60, "cursor": other.cursor_at(3) }),
            &ctx,
        );
        assert!(
            is_error(&resp) && text_of(&resp).contains("another query"),
            "{resp}"
        );
    }
}
