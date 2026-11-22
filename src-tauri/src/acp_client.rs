//! OC1 (2026-11-18): one Grok chat turn over the Agent Client Protocol
//! (ACP), behind Settings' per-agent switch (`agents.acp.grok`, off by
//! default). ACP is JSON-RPC 2.0, one object per line, over the CLI's stdin
//! and stdout: the host sends `initialize`, `session/new` (or
//! `session/load`) and `session/prompt`; the agent streams
//! `session/update` notifications and may ask the host things
//! (`session/request_permission`, `fs/*`, `terminal/*`). Written from the
//! protocol's spec and the 2026-11-17 live probe of `grok agent stdio`
//! (grok 1.0.88; vault `outputs/hollow-build-run-2026-11-17/oc1-grok-acp-probe.md`).
//!
//! **A pure state machine**, like `codex_app_server::AppSession`:
//! [`AcpSession`] never touches a process. The chat turn spawns the agent
//! through `agent::build_command` (so its process group, PATH, env, scratch
//! and Stop are the headless run's), writes what the session returns to the
//! agent's stdin and hands it each stdout line `process_group::drive` reads
//! (bounded at `process_group::STDOUT_LINE_MAX`; a longer line arrives as a
//! dropped-line event and is ignored here). The tests replay probe-shaped
//! frames through it and through a scripted stand-in agent.
//!
//! - **Handshake**, each request written once the one before it is answered:
//!   `initialize {protocolVersion: 1, clientCapabilities: {fs: {readTextFile:
//!   false, writeTextFile: false}, terminal: false}}` (no fs or terminal
//!   offered; an answer naming another protocol version ends the turn), then
//!   `session/new {cwd, mcpServers: [], _meta}` or, for a resumed turn,
//!   `session/load {sessionId, cwd, mcpServers: [], _meta}` (whose history
//!   replay is dropped), then `session/set_config_option` for the model and
//!   the reasoning effort when the run names them (probed: `grok agent -m`
//!   does not reach a stdio session, a plain string value does), then
//!   `session/prompt` with the prompt as one text block. A refused request
//!   ends the turn with a failed `result`; a load refused because the
//!   session is gone says `LOAD_FAILED`, which `result_kind` reads as a
//!   failed resume, so the chat retries fresh as it does for headless
//!   `--resume`; any other refused load fails the turn and keeps the history.
//! - **Headless lines.** What the agent says becomes the Claude stream-json
//!   lines `grok -p --output-format streaming-messages-json
//!   --include-partial-messages` prints, which `agent::parse_line` already
//!   reads (fixtures/grok), so the chat UI, records and receipts see the same
//!   events:
//!
//!   | ACP | headless line |
//!   |---|---|
//!   | `session/new` or `session/load` answer | `system/init {session_id}` |
//!   | first chunk of a message | `stream_event message_start` |
//!   | `agent_thought_chunk` | `stream_event content_block_start {thinking}` (once per message) |
//!   | `agent_message_chunk` (text) | `stream_event content_block_delta {text_delta}` |
//!   | message end (a tool call, the turn's end) | `assistant {thinking?, text}` |
//!   | `tool_call` | `assistant {tool_use {id: toolCallId, name: title, input: rawInput}}` |
//!   | `tool_call_update` `completed` / `failed` | `user {tool_result {content, is_error}}` |
//!   | `session/prompt` answer | `result {subtype, is_error, result, stop_reason, usage, session_id}` |
//!
//!   Usage comes from the answer's `_meta.usage` (or `usage`): Grok's ACP
//!   `inputTokens` is the full prompt (its docs), so the headless
//!   `input_tokens` is it less the cached read and write tokens (live
//!   2026-11-18: `inputTokens`, `outputTokens`, `cachedReadTokens`,
//!   `cacheCreationTokens`, `costUsdTicks`). No usage reads as unknown,
//!   never zero; the cost only when Grok calls it complete. `end_turn`, `max_tokens` and `refusal` are
//!   a success with that stop reason; `max_turn_requests` is
//!   `error_max_turns`; `cancelled` is nothing after a stop Hollow asked
//!   ([`AcpSession::note_stopping`]) and a failure otherwise. Output that
//!   ends before the answer is a failed `result` ([`AcpSession::ended_early`]).
//!   Everything else is dropped: `plan`, `user_message_chunk`, commands and
//!   mode or config updates, every `_x.ai/*` notification, another
//!   session's updates, and anything before the prompt is sent.
//! - **Agent requests.** `session/request_permission` is answered with the
//!   request's own reject option (`reject_once`, else `reject_always`, else
//!   `cancelled`), never an allow: the headless run can't ask either, and an
//!   Allow/Deny card over ACP is unproven (the probe's Grok never asked).
//!   The declined tool's result reads as an error, as a headless denial
//!   does. Every other request (`fs/*`, `terminal/*`, `_x.ai/*`) gets
//!   JSON-RPC "method not found".
//! - **No steer.** ACP has no steer method (Grok queues a second prompt,
//!   unprobed), so `steering` still gives Grok none.

use crate::agent::AgentKind;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// The ACP version Hollow speaks (what grok 1.0.88 answered).
pub const PROTOCOL_VERSION: u64 = 1;

/// The words a refused Grok `session/load` starts its failure with
/// (`"{label} couldn't load session"`); `result_kind`'s resume patterns match
/// them. Tests pin the Grok one.
#[cfg(test)]
pub const LOAD_FAILED: &str = "Grok couldn't load session";

/// One CLI's ACP dialect: how its stdio server starts and whether Hollow's
/// switch may select it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dialect {
    pub agent: AgentKind,
    /// The words after the program that start its stdio server.
    pub server: &'static [&'static str],
    /// Probed live against a signed-in CLI.
    pub probed: bool,
    /// Settings' switch can turn it on.
    pub selectable: bool,
}

/// The dialects Hollow knows. Grok's is probed (`grok agent stdio`), behind
/// Settings' switch. Antigravity's (`agy acp`) was probed live on agy 1.5.13
/// (2026-11-20): `initialize`, `session/new` with `mcpServers`, config options
/// (`model` as a base id, `thought_level`, `mode`), `session/request_permission`
/// and `usage_update`. Its chat turns run over it with no switch
/// (`agent::antigravity_acp`); `selectable` stays false so the switch never
/// lists it.
pub const DIALECTS: [Dialect; 2] = [
    Dialect {
        agent: AgentKind::Grok,
        server: &["agent", "stdio"],
        probed: true,
        selectable: true,
    },
    Dialect {
        agent: AgentKind::Antigravity,
        server: &["acp"],
        probed: true,
        selectable: false,
    },
];

pub fn dialect(kind: AgentKind) -> Option<&'static Dialect> {
    DIALECTS.iter().find(|d| d.agent == kind)
}

/// Whether the per-agent switch can put `kind` on ACP.
pub fn selectable(kind: AgentKind) -> bool {
    dialect(kind).is_some_and(|d| d.selectable)
}

/// How a Hollow permission mode runs over Grok's ACP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrokAcpMode {
    /// `grok --sandbox <profile>` (probed: the top-level flag reaches a
    /// stdio session; `sandbox-events.jsonl` records the profile).
    pub sandbox: &'static str,
    /// `grok agent --always-approve` and `_meta.yoloMode: true`.
    pub always_approve: bool,
}

/// Hollow's permission mode over Grok's ACP server, or why the turn stays on
/// the headless path. A mode goes over ACP only when it maps exactly onto
/// what headless `grok -p` gets (`agent::build_grok`), never looser:
///
/// | mode | transport | why |
/// |---|---|---|
/// | `bypassPermissions` | ACP | headless is `--permission-mode bypassPermissions --sandbox off`; ACP gets `--sandbox off` and always-approve (`grok agent --always-approve` plus `_meta.yoloMode`), Grok's own name for the same mode |
/// | `auto` | headless | `_meta.autoMode` exists, but Grok's docs say a configured always-approve supersedes it on `session/new`, so the turn could run looser than headless `--permission-mode auto` |
/// | `plan`, `default`, `acceptEdits` | headless | a stdio session takes no permission mode: `grok --permission-mode` is ignored there (probed: the session's startup hints say `permission_mode: None` and the config's mode is seeded), and ACP can only turn auto or always-approve on, so it would start in the person's configured mode, not the one Hollow asks for |
///
/// Any other value is headless too (headless runs it as acceptEdits).
/// Matched untrimmed, exactly as `agent::build_grok` and
/// `agent::grok_sandbox` match it: a padded mode runs as acceptEdits there,
/// so it must not reach ACP as bypass.
pub fn grok_mode(perm: &str) -> Result<GrokAcpMode, &'static str> {
    match perm {
        "bypassPermissions" => Ok(GrokAcpMode {
            sandbox: "off",
            always_approve: true,
        }),
        "auto" => Err("a configured always-approve would supersede ACP's auto mode"),
        "plan" | "default" | "acceptEdits" => {
            Err("a stdio session can't be put in this permission mode")
        }
        _ => Err("an unknown mode runs headless"),
    }
}

/// What a session needs to run one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartParams {
    pub cwd: String,
    /// The contextual prompt headless would read from `--prompt-file`.
    pub prompt: String,
    /// The native session to load instead of starting one.
    pub resume: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// `session/new`'s and `session/load`'s `_meta`.
    pub session_meta: Value,
    /// Hollow's mode, named on the `system/init` line.
    pub perm: String,
    /// Whose server this is: its name in failures and its config option ids.
    pub agent: AgentKind,
    /// Antigravity: the agent's own mode id (`default`, `accept-edits`,
    /// `auto`, `plan`) set as the `mode` config option; `None` leaves it.
    pub mode: Option<String>,
    /// Route the agent's `session/request_permission` to the person's
    /// approval cards ([`Step::asks`]) instead of declining it.
    pub ask: bool,
    /// `session/new`'s `mcpServers`: per session, never an agent-wide file.
    pub mcp_servers: Vec<Value>,
}

/// What one stdout line produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Step {
    /// Headless stream-json lines, in order, for `agent::parse_line`.
    pub lines: Vec<String>,
    /// JSON-RPC lines to write to the agent's stdin, in order.
    pub writes: Vec<String>,
    /// The turn is over: close stdin so the agent exits.
    pub finished: bool,
    /// Permission requests the person must answer (`StartParams::ask`): each
    /// is answered later with [`PermissionAsk::reply`].
    pub asks: Vec<PermissionAsk>,
}

/// ACP's `mcpServers` for a turn, from the `{"mcpServers": {...}}` config
/// Hollow builds for Claude and Codex (`mcp_attach`, the person's shared
/// connectors included): a stdio entry (`command`, `args`, an `env` map)
/// becomes `{name, command, args, env: [{name, value}]}`, an `http` or `sse`
/// entry `{type, name, url, headers: [{name, value}]}`. Anything else (a
/// claude.ai proxy, which only Claude's login can open) is left out. Sent in
/// `session/new` for this chat only: never written to agy's own config.
pub fn mcp_servers_from_config(config: Option<&str>) -> Vec<Value> {
    let Some(servers) = config
        .filter(|c| !c.trim().is_empty())
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|v| v.get("mcpServers").and_then(Value::as_object).cloned())
    else {
        return Vec::new();
    };
    let pairs = |value: Option<&Value>| -> Vec<Value> {
        match value {
            Some(Value::Object(map)) => map
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|v| json!({"name": k, "value": v})))
                .collect(),
            Some(Value::Array(list)) => list.clone(),
            _ => Vec::new(),
        }
    };
    servers
        .iter()
        .filter_map(|(name, entry)| {
            if let Some(command) = entry.get("command").and_then(Value::as_str) {
                return Some(json!({
                    "name": name,
                    "command": command,
                    "args": entry.get("args").cloned().unwrap_or_else(|| json!([])),
                    "env": pairs(entry.get("env")),
                }));
            }
            let kind = entry.get("type").and_then(Value::as_str)?;
            let url = entry.get("url").and_then(Value::as_str)?;
            matches!(kind, "http" | "sse").then(|| {
                json!({
                    "type": kind,
                    "name": name,
                    "url": url,
                    "headers": pairs(entry.get("headers")),
                })
            })
        })
        .collect()
}

/// One `session/request_permission` waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionAsk {
    /// The request's JSON-RPC id, handed back to answer it.
    pub rpc_id: Value,
    pub tool_call_id: String,
    /// The tool's own name (`run_command`, `write_file`, an MCP tool).
    pub tool: String,
    /// What the agent shows for it (the command line, a title).
    pub title: String,
    pub input: Value,
    /// The options the agent offered (`allow_once`, `reject_once`, ...).
    pub options: Vec<Value>,
}

impl PermissionAsk {
    /// The JSON-RPC answer: the request's own allow or reject option, or
    /// `cancelled` when it offered none of that kind. Independent of the
    /// session, so the card's task can write it when the person clicks.
    pub fn reply(&self, allow: bool) -> String {
        let pick = |kinds: &[&str]| {
            kinds.iter().find_map(|kind| {
                self.options.iter().find_map(|o| {
                    (o.get("kind").and_then(Value::as_str) == Some(*kind))
                        .then(|| o.get("optionId").and_then(Value::as_str))
                        .flatten()
                        .map(str::to_string)
                })
            })
        };
        let chosen = if allow {
            pick(&["allow_once"])
        } else {
            pick(&["reject_once", "reject_always"])
        };
        let outcome = match chosen {
            Some(option) => json!({"outcome": "selected", "optionId": option}),
            None => json!({"outcome": "cancelled"}),
        };
        json!({"jsonrpc": "2.0", "id": self.rpc_id, "result": {"outcome": outcome}}).to_string()
    }

    /// What the approval card says Hollow is being asked to do.
    pub fn action_text(&self) -> String {
        let text = |key: &str| {
            self.input
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let what = match self.tool.as_str() {
            "run_command" => text("CommandLine").map(|c| format!("Run the command: {c}")),
            "write_file" | "write_to_file" | "edit_file" | "replace_file_content" => text("Path")
                .or_else(|| text("TargetFile"))
                .map(|p| format!("Change the file: {p}")),
            _ => None,
        };
        let mut said = what
            .or_else(|| (!self.title.is_empty()).then(|| self.title.clone()))
            .unwrap_or_else(|| format!("Use the {} tool", self.tool));
        if said.chars().count() > 600 {
            said = said.chars().take(600).collect::<String>() + "...";
        }
        said
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Initialize,
    NewSession,
    LoadSession,
    SetConfig { id: &'static str, value: String },
    Prompt,
}

/// The message being streamed: its id, text and whether it had thinking.
#[derive(Debug, Default)]
struct Message {
    id: String,
    text: String,
    thinking: bool,
}

#[derive(Debug, Default)]
struct Tool {
    /// The latest content an update carried (ACP replaces, never appends).
    output: Option<String>,
    done: bool,
}

/// One turn's ACP conversation. See the module docs.
#[derive(Debug)]
pub struct AcpSession {
    params: StartParams,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    configs: Vec<(&'static str, String)>,
    session_id: Option<String>,
    /// `session/prompt` is written: updates count from here.
    prompting: bool,
    stopping: bool,
    finished: bool,
    messages: u64,
    message: Option<Message>,
    last_text: String,
    tools: HashMap<String, Tool>,
    declined: Vec<String>,
    declined_set: HashSet<String>,
}

impl AcpSession {
    /// A session and its first write (`initialize`).
    pub fn start(params: StartParams) -> (Self, Vec<String>) {
        let mut configs = Vec::new();
        if let Some(model) = params
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            configs.push(("model", model.to_string()));
        }
        // Grok names its effort option `reasoning_effort`; Antigravity's is
        // `thought_level`, and its `mode` is a config option too.
        let effort_id = if params.agent == AgentKind::Antigravity {
            "thought_level"
        } else {
            "reasoning_effort"
        };
        if let Some(effort) = params
            .effort
            .as_deref()
            .map(str::trim)
            .filter(|e| !e.is_empty())
        {
            configs.push((effort_id, effort.to_string()));
        }
        if let Some(mode) = params
            .mode
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            configs.push(("mode", mode.to_string()));
        }
        let mut session = AcpSession {
            params,
            next_id: 1,
            pending: HashMap::new(),
            configs,
            session_id: None,
            prompting: false,
            stopping: false,
            finished: false,
            messages: 0,
            message: None,
            last_text: String::new(),
            tools: HashMap::new(),
            declined: Vec::new(),
            declined_set: HashSet::new(),
        };
        let first = session.request(
            Pending::Initialize,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false,
                },
                "clientInfo": {"name": "hollow", "version": env!("CARGO_PKG_VERSION")},
            }),
        );
        (session, vec![first])
    }

    /// The agent's name in what Hollow says about its server.
    fn label(&self) -> &'static str {
        self.params.agent.label()
    }

    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// The agent's native session id, once `session/new` or `session/load`
    /// answered.
    #[allow(dead_code)]
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Hollow's Stop (or a turn guard) is ending the process group: a
    /// `cancelled` answer or the output ending is that stop, not a failure.
    pub fn note_stopping(&mut self) {
        self.stopping = true;
    }

    /// The output ended before the turn did: a failed `result`, once.
    /// `None` after a stop or once finished.
    pub fn ended_early(&mut self) -> Option<String> {
        if self.finished || self.stopping {
            return None;
        }
        let stage = if self.prompting {
            "during its turn"
        } else {
            "before its turn started"
        };
        let mut lines = Vec::new();
        self.close_message(&mut lines);
        let line = self.failure(format!(
            "{}'s ACP session ended {stage}, with no answer to Hollow's request.",
            self.label()
        ));
        self.finished = true;
        // The open message's text is not delivered after the fact: the
        // failure is the turn's outcome, as a headless crash's would be.
        Some(line)
    }

    /// One stdout line from the agent.
    pub fn on_line(&mut self, raw: &str) -> Step {
        let mut step = Step::default();
        let raw = raw.trim();
        if raw.is_empty() || crate::process_group::dropped_line_bytes(raw).is_some() {
            return step;
        }
        let Ok(Value::Object(msg)) = serde_json::from_str::<Value>(raw) else {
            // Not JSON-RPC (a stray print): nothing to act on.
            return step;
        };
        match (msg.get("method").and_then(Value::as_str), msg.get("id")) {
            (Some(method), Some(id)) if !id.is_null() => {
                let id = id.clone();
                match self.permission_ask(method, &id, msg.get("params")) {
                    Some(ask) => step.asks.push(ask),
                    None => step
                        .writes
                        .push(self.answer_request(method, id, msg.get("params"))),
                }
            }
            (Some(method), _) => {
                if method == "session/update" && !self.finished {
                    self.on_update(msg.get("params"), &mut step.lines);
                }
                // `_x.ai/*` and any other notification: ignored.
            }
            (None, Some(id)) => {
                if let Some(pending) = id.as_u64().and_then(|n| self.pending.remove(&n)) {
                    self.on_response(pending, &msg, &mut step);
                } else if id.is_null() && !self.finished && !self.pending.is_empty() {
                    // An error the server couldn't tie to a request (it
                    // couldn't read one of Hollow's): the request it lost
                    // is never answered, so the turn ends now, not at the
                    // watchdog.
                    if let Some(error) = msg.get("error") {
                        self.fail(
                            &mut step,
                            format!(
                                "{}'s ACP server couldn't read a request: {}",
                                self.label(),
                                error_text(error)
                            ),
                        );
                    }
                }
            }
            (None, None) => {}
        }
        step.finished = self.finished;
        step
    }

    fn request(&mut self, pending: Pending, method: &str, params: Value) -> String {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, pending);
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    }

    /// A permission request the person answers, when this session routes
    /// them ([`StartParams::ask`]); `None` leaves it to [`Self::answer_request`]
    /// (declined, as Grok's always are).
    fn permission_ask(
        &mut self,
        method: &str,
        id: &Value,
        params: Option<&Value>,
    ) -> Option<PermissionAsk> {
        if method != "session/request_permission" || !self.params.ask || self.finished {
            return None;
        }
        let params = params?;
        let call = params.get("toolCall")?;
        let options = params
            .get("options")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let text = |key: &str| {
            call.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        Some(PermissionAsk {
            rpc_id: id.clone(),
            tool_call_id: text("toolCallId").unwrap_or_default().to_string(),
            tool: self.named(call),
            title: text("title").unwrap_or_default().to_string(),
            input: call
                .get("rawInput")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(Value::Null),
            options,
        })
    }

    /// The person declined `tool_call_id` (its card's task wrote the answer):
    /// the tool reads as the denial it was, as an automatic decline does.
    pub fn note_declined(&mut self, tool_call_id: &str) {
        if self.declined_set.insert(tool_call_id.to_string()) {
            self.declined.push(tool_call_id.to_string());
        }
    }

    fn answer_request(&mut self, method: &str, id: Value, params: Option<&Value>) -> String {
        if method == "session/request_permission" {
            let params = params.cloned().unwrap_or(Value::Null);
            if let Some(tool) = params
                .pointer("/toolCall/toolCallId")
                .and_then(Value::as_str)
            {
                if self.declined_set.insert(tool.to_string()) {
                    self.declined.push(tool.to_string());
                }
            }
            let options = params
                .get("options")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let pick = |kind: &str| {
                options.iter().find_map(|o| {
                    (o.get("kind").and_then(Value::as_str) == Some(kind))
                        .then(|| o.get("optionId").and_then(Value::as_str))
                        .flatten()
                        .map(str::to_string)
                })
            };
            let outcome = match pick("reject_once").or_else(|| pick("reject_always")) {
                Some(option) => json!({"outcome": "selected", "optionId": option}),
                None => json!({"outcome": "cancelled"}),
            };
            return json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}}).to_string();
        }
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: Hollow offers {method} to no agent"),
            },
        })
        .to_string()
    }

    fn on_response(&mut self, pending: Pending, msg: &Map<String, Value>, step: &mut Step) {
        if self.finished {
            return;
        }
        let error = msg.get("error").map(error_text);
        let result = msg.get("result").cloned().unwrap_or(Value::Null);
        match pending {
            Pending::Initialize => {
                if let Some(error) = error {
                    let who = self.label();
                    return self.fail(
                        step,
                        format!("{who}'s ACP server refused to start: {error}"),
                    );
                }
                let version = result.get("protocolVersion").and_then(Value::as_u64);
                if version != Some(PROTOCOL_VERSION) {
                    let said = version.map_or_else(|| "none".to_string(), |v| v.to_string());
                    return self.fail(
                        step,
                        format!(
                            "{}'s ACP server speaks protocol version {said}; Hollow speaks {PROTOCOL_VERSION}.",
                            self.label()
                        ),
                    );
                }
                let mut params = json!({
                    "cwd": self.params.cwd,
                    "mcpServers": self.params.mcp_servers,
                });
                if self.params.session_meta.is_object() {
                    params["_meta"] = self.params.session_meta.clone();
                }
                let write = match self.params.resume.clone() {
                    Some(id) => {
                        params["sessionId"] = json!(id);
                        self.request(Pending::LoadSession, "session/load", params)
                    }
                    None => self.request(Pending::NewSession, "session/new", params),
                };
                step.writes.push(write);
            }
            Pending::NewSession => {
                if let Some(error) = error {
                    let who = self.label();
                    return self.fail(step, format!("{who} couldn't start a session: {error}"));
                }
                let Some(id) = result.get("sessionId").and_then(Value::as_str) else {
                    let who = self.label();
                    return self.fail(step, format!("{who} started a session with no id."));
                };
                self.session_id = Some(id.to_string());
                self.session_ready(step);
            }
            Pending::LoadSession => {
                let id = self.params.resume.clone().unwrap_or_default();
                if let Some(error) = error {
                    // Only a session Grok can't find (or can't resume under
                    // this sandbox) is a failed resume the chat retries
                    // fresh, as headless `--resume`'s; any other refusal
                    // (sign-in, network, a limit) fails the turn and keeps
                    // the chat's history.
                    let missing = msg.get("error").is_some_and(load_missing);
                    let message = if missing {
                        format!(
                            "{} couldn't load session \"{id}\" over ACP: {error}",
                            self.label()
                        )
                    } else {
                        format!(
                            "{}'s ACP server refused to reopen this chat's session: {error}",
                            self.label()
                        )
                    };
                    return self.fail(step, message);
                }
                self.session_id = Some(id);
                self.session_ready(step);
            }
            Pending::SetConfig { id, value } => {
                if let Some(error) = error {
                    // Antigravity's thinking level is the one setting a turn
                    // can do without: a model that lacks the level runs at
                    // its own default instead of failing the turn.
                    if self.params.agent == AgentKind::Antigravity && id == "thought_level" {
                        return self.next_config_or_prompt(step);
                    }
                    let what = match id {
                        "model" => "model",
                        "mode" => "permission mode",
                        _ => "reasoning effort",
                    };
                    return self.fail(
                        step,
                        format!("{} refused the {what} {value}: {error}", self.label()),
                    );
                }
                self.next_config_or_prompt(step);
            }
            Pending::Prompt => {
                let mut lines = Vec::new();
                self.close_message(&mut lines);
                self.decline_leftovers(&mut lines);
                step.lines.extend(lines);
                if let Some(error) = error {
                    return self.fail(step, error);
                }
                self.finish(step, &result);
            }
        }
    }

    fn session_ready(&mut self, step: &mut Step) {
        let mut init = json!({
            "type": "system",
            "subtype": "init",
            "session_id": self.session_id,
            "cwd": self.params.cwd,
            "permissionMode": self.params.perm,
            "transport": "acp",
        });
        if let Some(model) = self.configs.iter().find(|(id, _)| *id == "model") {
            init["model"] = json!(model.1);
        }
        step.lines.push(init.to_string());
        self.next_config_or_prompt(step);
    }

    fn next_config_or_prompt(&mut self, step: &mut Step) {
        let session = self.session_id.clone().unwrap_or_default();
        if !self.configs.is_empty() {
            let (id, value) = self.configs.remove(0);
            let write = self.request(
                Pending::SetConfig {
                    id,
                    value: value.clone(),
                },
                "session/set_config_option",
                json!({"sessionId": session, "configId": id, "value": value}),
            );
            step.writes.push(write);
            return;
        }
        let write = self.request(
            Pending::Prompt,
            "session/prompt",
            json!({
                "sessionId": session,
                "prompt": [{"type": "text", "text": self.params.prompt}],
            }),
        );
        self.prompting = true;
        step.writes.push(write);
    }

    fn finish(&mut self, step: &mut Step, result: &Value) {
        let stop = result
            .get("stopReason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
            .to_string();
        let (subtype, is_error) = match stop.as_str() {
            "max_turn_requests" => ("error_max_turns", true),
            "cancelled" if self.stopping => {
                // Hollow's own stop: no terminal line, as headless leaves none.
                self.finished = true;
                return;
            }
            "cancelled" => ("error_during_execution", true),
            _ => ("success", false),
        };
        let text = if stop == "cancelled" {
            format!("{} cancelled the turn.", self.label())
        } else {
            self.last_text.clone()
        };
        let mut line = json!({
            "type": "result",
            "subtype": subtype,
            "is_error": is_error,
            "result": text,
            "stop_reason": stop,
            "session_id": self.session_id,
        });
        if let Some(usage) = prompt_usage(result) {
            line["usage"] = usage;
        }
        if let Some(cost) = prompt_cost(result) {
            line["total_cost_usd"] = json!(cost);
        }
        step.lines.push(line.to_string());
        self.finished = true;
    }

    fn fail(&mut self, step: &mut Step, message: String) {
        step.lines.push(self.failure(message));
        self.finished = true;
        step.finished = true;
    }

    fn failure(&self, message: String) -> String {
        json!({
            "type": "result",
            "subtype": "error_during_execution",
            "is_error": true,
            "result": message,
            "errors": [message],
            "session_id": self.session_id,
        })
        .to_string()
    }

    fn on_update(&mut self, params: Option<&Value>, out: &mut Vec<String>) {
        // Before the prompt (setup, a load's history replay) nothing counts.
        if !self.prompting {
            return;
        }
        let Some(params) = params else { return };
        if params.get("sessionId").and_then(Value::as_str) != self.session_id.as_deref() {
            return;
        }
        let Some(update) = params.get("update") else {
            return;
        };
        match update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            "agent_message_chunk" => {
                let Some(text) = chunk_text(update) else {
                    return;
                };
                self.open_message(out);
                if let Some(message) = self.message.as_mut() {
                    message.text.push_str(text);
                }
                out.push(stream_event(json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": text},
                })));
            }
            "agent_thought_chunk" => {
                // Reasoning after this message's text starts the next one.
                if self.message.as_ref().is_some_and(|m| !m.text.is_empty()) {
                    self.close_message(out);
                }
                self.open_message(out);
                if let Some(message) = self.message.as_mut() {
                    if !message.thinking {
                        message.thinking = true;
                        out.push(stream_event(json!({
                            "type": "content_block_start",
                            "index": 0,
                            "content_block": {"type": "thinking", "thinking": ""},
                        })));
                    }
                }
            }
            "tool_call" => {
                self.close_message(out);
                let Some(id) = tool_id(update) else { return };
                if !self.tools.contains_key(&id) {
                    self.tool_use(&id, update, out);
                }
                self.tool_progress(&id, update, out);
            }
            "tool_call_update" => {
                let Some(id) = tool_id(update) else { return };
                if !self.tools.contains_key(&id) {
                    self.close_message(out);
                    self.tool_use(&id, update, out);
                }
                self.tool_progress(&id, update, out);
            }
            // Antigravity's `usage_update {used, size}`: how full the context
            // is and how big its window is (1,048,576 on Gemini 4, probed
            // 2026-11-20), as a `system/context` line the headless parser
            // turns into the gauge's figure.
            "usage_update" if self.params.agent == AgentKind::Antigravity => {
                let used = update.get("used").and_then(Value::as_u64).unwrap_or(0);
                let size = update.get("size").and_then(Value::as_u64).unwrap_or(0);
                if used > 0 && size > 0 {
                    out.push(
                        json!({"type": "system", "subtype": "context", "tokens": used, "window": size})
                            .to_string(),
                    );
                }
            }
            // `plan`, `user_message_chunk`, commands, modes, config: dropped.
            _ => {}
        }
    }

    fn open_message(&mut self, out: &mut Vec<String>) {
        if self.message.is_some() {
            return;
        }
        self.messages += 1;
        let id = format!("acp_msg_{}", self.messages);
        out.push(stream_event(json!({
            "type": "message_start",
            "message": {
                "id": id,
                "type": "message",
                "role": "assistant",
                "content": [],
            },
        })));
        self.message = Some(Message {
            id,
            ..Message::default()
        });
    }

    fn close_message(&mut self, out: &mut Vec<String>) {
        let Some(message) = self.message.take() else {
            return;
        };
        let mut content = Vec::new();
        if message.thinking {
            content.push(json!({"type": "thinking", "thinking": ""}));
        }
        if !message.text.is_empty() {
            content.push(json!({"type": "text", "text": message.text}));
            self.last_text = message.text.clone();
        }
        if content.is_empty() {
            return;
        }
        out.push(assistant(&message.id, content));
    }

    /// The tool's headless name. Antigravity names an MCP tool
    /// `mcp_<server>_<tool>`; Hollow's cards, preapprovals and tool chips read
    /// Claude's `mcp__<server>__<tool>`, so a name that starts with one of
    /// this session's servers is spelled that way.
    fn named(&self, update: &Value) -> String {
        let name = tool_name(update);
        let mut servers: Vec<&str> = self
            .params
            .mcp_servers
            .iter()
            .filter_map(|s| s.get("name").and_then(Value::as_str))
            .collect();
        servers.sort_by_key(|s| std::cmp::Reverse(s.len()));
        for server in servers {
            if let Some(tool) = name.strip_prefix(&format!("mcp_{server}_")) {
                return format!("mcp__{server}__{tool}");
            }
        }
        name
    }

    fn tool_use(&mut self, id: &str, update: &Value, out: &mut Vec<String>) {
        self.messages += 1;
        let input = update
            .get("rawInput")
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        out.push(assistant(
            &format!("acp_msg_{}", self.messages),
            vec![json!({
                "type": "tool_use",
                "id": id,
                "name": self.named(update),
                "input": input,
            })],
        ));
        self.tools.insert(id.to_string(), Tool::default());
    }

    fn tool_progress(&mut self, id: &str, update: &Value, out: &mut Vec<String>) {
        let declined = self.declined_set.contains(id);
        let Some(tool) = self.tools.get_mut(id) else {
            return;
        };
        if let Some(text) = tool_output(update) {
            tool.output = Some(text);
        }
        let status = update.get("status").and_then(Value::as_str).unwrap_or("");
        if tool.done || !matches!(status, "completed" | "failed") {
            return;
        }
        tool.done = true;
        let failed = status == "failed" || declined;
        let content = tool.output.clone().unwrap_or_else(|| {
            if declined {
                declined_text()
            } else {
                String::new()
            }
        });
        out.push(tool_result(id, &content, failed));
    }

    /// A declined tool the agent never closed reads as the denial it was.
    fn decline_leftovers(&mut self, out: &mut Vec<String>) {
        for id in self.declined.clone() {
            match self.tools.get_mut(&id) {
                Some(tool) if !tool.done => {
                    tool.done = true;
                    let content = tool.output.clone().unwrap_or_else(declined_text);
                    out.push(tool_result(&id, &content, true));
                }
                _ => {}
            }
        }
    }
}

fn declined_text() -> String {
    "This request was not approved: the person declined it, or this turn can't ask for approval."
        .to_string()
}

fn stream_event(event: Value) -> String {
    json!({"type": "stream_event", "event": event, "parent_tool_use_id": null}).to_string()
}

fn assistant(id: &str, content: Vec<Value>) -> String {
    json!({
        "type": "assistant",
        "message": {
            "id": id,
            "type": "message",
            "role": "assistant",
            "content": content,
        },
        "parent_tool_use_id": null,
    })
    .to_string()
}

fn tool_result(id: &str, content: &str, is_error: bool) -> String {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": id,
                "content": content,
                "is_error": is_error,
            }],
        },
        "parent_tool_use_id": null,
    })
    .to_string()
}

fn chunk_text(update: &Value) -> Option<&str> {
    let content = update.get("content")?;
    if content.get("type").and_then(Value::as_str) != Some("text") {
        return None;
    }
    content
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
}

fn tool_id(update: &Value) -> Option<String> {
    update
        .get("toolCallId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// The headless tool name: Grok's ACP `title` is the tool's own name
/// (`run_terminal_command` in the probe); a prose title falls back to the
/// ACP `kind`.
fn tool_name(update: &Value) -> String {
    // Antigravity names the tool in `name` (`run_command`, `write_file`,
    // `mcp_<server>_<tool>`) and puts the command or a summary in `title`.
    if let Some(name) = update
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_string();
    }
    let title = update
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let plain = !title.is_empty()
        && title
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'));
    if plain {
        return title.to_string();
    }
    update
        .get("kind")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty())
        .unwrap_or("tool")
        .to_string()
}

/// A tool update's output as text: its `content` blocks' text (a diff names
/// its file), else its `rawOutput`.
fn tool_output(update: &Value) -> Option<String> {
    let mut parts = Vec::new();
    for block in update
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("content") => {
                if let Some(text) = block.pointer("/content/text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some("diff") => {
                let path = block
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("a file");
                parts.push(format!("Edited {path}"));
            }
            _ => {}
        }
    }
    if !parts.is_empty() {
        return Some(parts.join("\n"));
    }
    match update.get("rawOutput") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// A JSON-RPC error's words: its message, and its `data` when that adds any.
/// Whether a refused `session/load` says the session is gone: ACP's
/// "resource not found" (-32002), or the words headless `--resume` fails
/// with (`result_kind`'s Grok pattern) and the usual not-found ones.
fn load_missing(error: &Value) -> bool {
    if error.get("code").and_then(Value::as_i64) == Some(-32002) {
        return true;
    }
    let text = error_text(error).to_lowercase();
    [
        "not found",
        "unknown session",
        "no such session",
        "does not exist",
        "failed to restore session",
        "cannot resume this session under sandbox profile",
    ]
    .iter()
    .any(|words| text.contains(words))
}

fn error_text(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error")
        .to_string();
    match error.get("data") {
        Some(Value::String(data)) if !data.is_empty() && data != &message => {
            format!("{message} ({data})")
        }
        Some(Value::Object(data)) => match data.get("message").and_then(Value::as_str) {
            Some(text) if text != message => format!("{message} ({text})"),
            _ => message,
        },
        _ => message,
    }
}

/// The prompt answer's usage as headless `usage` (`input_tokens` uncached),
/// or `None` when it reports none.
fn prompt_usage(result: &Value) -> Option<Value> {
    let usage = result
        .pointer("/_meta/usage")
        .or_else(|| result.get("usage"))
        .filter(|u| u.is_object())?;
    let n = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| usage.get(*k).and_then(Value::as_u64))
    };
    // Headless names: already uncached.
    if let Some(input) = n(&["input_tokens"]) {
        let mut out = json!({
            "input_tokens": input,
            "output_tokens": n(&["output_tokens"]).unwrap_or(0),
            "cache_read_input_tokens": n(&["cache_read_input_tokens"]).unwrap_or(0),
            "cache_creation_input_tokens": n(&["cache_creation_input_tokens"]).unwrap_or(0),
        });
        if n(&["output_tokens"]).is_none() {
            out.as_object_mut()?.remove("output_tokens");
        }
        return Some(out);
    }
    // ACP names: `inputTokens` is the full prompt.
    let input = n(&["inputTokens"])?;
    let read = n(&["cachedReadTokens", "cacheReadInputTokens"]).unwrap_or(0);
    // Live 2026-11-18 (grok 1.0.88): `cacheCreationTokens`; ACP's own name
    // is `cachedWriteTokens`.
    let write = n(&[
        "cacheCreationTokens",
        "cachedWriteTokens",
        "cacheCreationInputTokens",
    ])
    .unwrap_or(0);
    Some(json!({
        "input_tokens": input.saturating_sub(read).saturating_sub(write),
        "output_tokens": n(&["outputTokens"]).unwrap_or(0),
        "cache_read_input_tokens": read,
        "cache_creation_input_tokens": write,
    }))
}

/// The prompt's cost in dollars from `_meta.usage.costUsdTicks` (Grok's
/// docs: 1e10 ticks to the dollar, as headless `total_cost_usd_ticks`), only
/// when Grok calls it complete. Its per-model `modelUsage` is not passed on:
/// its `inputTokens` may count the full prompt, unlike headless's.
fn prompt_cost(result: &Value) -> Option<f64> {
    let usage = result.pointer("/_meta/usage")?;
    let flagged = |key: &str| usage.get(key).and_then(Value::as_bool) == Some(true);
    if flagged("costIsPartial") || flagged("usageIsIncomplete") {
        return None;
    }
    let ticks = usage.get("costUsdTicks").and_then(Value::as_u64)?;
    Some(ticks as f64 / 1e10)
}

#[cfg(test)]
#[path = "acp_client_tests.rs"]
mod tests;
