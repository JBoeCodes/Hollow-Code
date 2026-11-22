//! Steering PR 3 (2026-10-21): one Codex chat turn over `codex app-server`,
//! so a person can steer it mid-turn. `codex exec` reads its stdin to EOF
//! before it starts and can't be steered; the app-server (JSON-RPC 2.0, one
//! object per line on stdio) has `turn/steer` and `turn/interrupt`
//! (codex-cli 0.163.0, probed 2026-10-21; design note and evidence in the
//! vault's `outputs/steering-2026-10-21/`). A steerable Codex chat turn
//! (`agent::live_codex_input`) runs one session per attempt in
//! `chat::drive_turn_attempts`; every other Codex run is `codex exec`.
//!
//! **A pure state machine.** [`AppSession`] never touches a process: the
//! caller writes what it returns to the server's stdin and hands it each
//! stdout line ([`AppSession::on_line`]), so the tests replay recorded
//! sessions (`fixtures/codex/app-server-*.jsonl`). One session is one turn:
//! - **Handshake**, each request written only once the one before it is
//!   answered: `initialize` (client `hollow`), the `initialized`
//!   notification, `thread/start` (cwd, model, approval policy, sandbox,
//!   config; `ephemeral` for a one-shot) or `thread/resume {threadId,
//!   excludeTurns: true}` with the same settings, then `turn/start` with the
//!   prompt as one text input. Request ids start at 1 and only grow. A
//!   handshake request the server refuses ends the turn with a
//!   `turn.failed` carrying the server's words (a resume's "no rollout
//!   found …" still reads as a failed resume, `result_kind`).
//! - **Exec lines.** What the server says becomes the `codex exec --json`
//!   lines `agent::parse_codex` already reads, so tool rows, file-change
//!   attribution, receipts and usage keep their tested parser:
//!
//!   | app-server | exec line |
//!   |---|---|
//!   | `thread/start`/`thread/resume` answer | `thread.started {thread_id}` |
//!   | `turn/started` | `turn.started` |
//!   | `item/started`, `item/completed` | `item.started`, `item.completed`, the item renamed: `agentMessage` → `agent_message {text}`, `reasoning` → `reasoning {text}` (its summary), `commandExecution` → `command_execution {command, aggregated_output, exit_code, status}`, `fileChange` → `file_change {changes: [{path, kind}], status}` (`declined` reads `failed`, so nothing is attributed), `mcpToolCall` → `mcp_tool_call {server, tool, arguments, result: {content, structured_content}, error, status}`, `webSearch` → `web_search {query, action}`, `collabAgentToolCall` → `collab_tool_call` (keys in snake case). Statuses in snake case (`in_progress`). |
//!   | `turn/plan/updated` | `todo_list {items: [{text, completed}]}`: started, then updated, completed at the turn's end |
//!   | `warning` | `item.completed` of an `error {message}` item |
//!   | `error` | `error {message}`; nothing when `willRetry` (the server retries it, so it isn't the turn's end) |
//!   | `thread/tokenUsage/updated` | nonterminal `turn.usage` snapshot: this turn's tokens (the last `total` less what the thread held before the turn's first model call); a successful `turn.completed` supplies the final report. No usable notification means unknown, never zero. |
//!   | `account/rateLimits/updated` | passive `account.rate_limits` snapshot, normalized and cached for this run's provider/login; never starts a request |
//!   | `turn/completed` `completed` / `failed` / `interrupted` | `turn.completed {usage}` / `turn.failed {error: {message, codex_error_info}}` / nothing after a stop Hollow asked or noted ([`AppSession::note_stopping`]; exec's SIGINT leaves no terminal line either), else `turn.failed` ("Codex interrupted the turn…"), so it never reads as a success |
//!
//!   Output that ends before the turn did is `turn.failed` too
//!   ([`AppSession::ended_early`]).
//!
//!   Everything else is dropped: deltas (exec streams none), status and
//!   server notifications, items exec has no name for (`plan`, `hookPrompt`,
//!   `dynamicToolCall`, `imageView`, `imageGeneration`, `sleep`,
//!   `subAgentActivity`, review-mode and `contextCompaction`), and every
//!   `userMessage` (exec echoes no user input; a steered one is reported by
//!   [`AppSession::steer_events`] instead). Only the active turn's
//!   notifications count: an interrupted turn's command keeps running and
//!   completes later under its old turn id (evidence, gotcha 4), and a
//!   resumed thread's first usage line names its previous turn.
//! - **Server requests are refused at once.** `codex exec` can't ask
//!   anyone, so a request the server makes (an approval, user input, an MCP
//!   elicitation, a dynamic tool call) gets the decline of its own shape and
//!   the turn goes on as exec's would; anything else gets a JSON-RPC
//!   "method not found". In Hollow's modes the policy is `never` (auto's
//!   requests go to Codex's own reviewer), so these are a backstop.
//! - **Steers.** [`AppSession::steer`] writes `turn/steer {threadId,
//!   expectedTurnId, clientUserMessageId, input}`; before `turn/start` is
//!   answered there is no turn id, so it is held. The reader never writes a
//!   steer: [`Step::held_steers`] tells the caller's input writer to take
//!   them ([`AppSession::take_held_steers`]), and a later steer writes any
//!   still held first, so steers keep their order. A stop settles held
//!   steers as undelivered.
//!   [`AppSession::steer_events`] drains each steer's outcome once:
//!   `Delivered` when its `userMessage` item appears in the turn (matched by
//!   the echoed client id, else by its text, else in order), `Refused` with
//!   the server's message when the request is rejected (`no active turn to
//!   steer`), `Undelivered` when the turn ends first. After the turn ends,
//!   or once a stop is asked, a steer is refused with
//!   [`SteerRefusal::Ended`].
//! - **Interrupt** writes `turn/interrupt` at most once (a repeated one went
//!   unanswered for over a minute in a probe, gotcha 6), waiting for the
//!   turn id if needed; before `turn/start` is written it just ends the
//!   session.
//!
//! The command comes from `agent::build_codex_app_server`, the thread's
//! settings from `agent::codex_app_server_start`.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// The client name the server records (its `userAgent` and `originator`).
const CLIENT_NAME: &str = "hollow";
/// Why a legacy approval request is denied.
const NO_APPROVALS: &str = "Hollow runs Codex without approval prompts";

/// What one turn starts with: `agent::codex_app_server_start` builds it
/// from a run spec.
#[derive(Debug, Clone, Default)]
pub struct StartParams {
    /// The whole prompt, as `codex exec` would get it on argv.
    pub prompt: String,
    /// The thread to continue (`thread/resume`); `None` starts one.
    pub resume: Option<String>,
    pub cwd: String,
    pub model: Option<String>,
    /// `never`, `on-request`, … (`AskForApproval`).
    pub approval_policy: String,
    /// `auto_review` for auto mode, else the user's own setting.
    pub approvals_reviewer: Option<String>,
    /// `read-only`, `workspace-write` or `danger-full-access`; empty under a
    /// permission profile.
    pub sandbox: String,
    /// A one-shot keeps no session file (exec's `--ephemeral`).
    pub ephemeral: bool,
    /// More config overrides (`model_reasoning_effort`, …); left out when
    /// empty.
    pub config: Map<String, Value>,
}

/// What one server line produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// `codex exec --json` lines, in order, for `agent::parse_line`.
    pub exec_lines: Vec<String>,
    /// JSON-RPC lines to write to the server's stdin, in order.
    pub writes: Vec<String>,
    /// The turn is over: the process may be closed.
    pub finished: bool,
    /// Held steers can go now (the turn id is known): the caller's input
    /// writer takes them with [`AppSession::take_held_steers`], behind
    /// whatever it already has queued, so steers reach the server in the
    /// order they were sent. The reader never writes a steer itself.
    pub held_steers: bool,
}

/// Why [`AppSession::steer`] wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteerRefusal {
    /// The turn ended, or a stop was asked: send it as the next turn.
    Ended,
    /// Nothing to send.
    Empty,
}

/// One steer's outcome, reported once by [`AppSession::steer_events`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteerUpdate {
    /// Its message joined the turn; the model reads it at its next step.
    Delivered { client_id: String },
    /// The server rejected it, in its own words.
    Refused { client_id: String, reason: String },
    /// The turn ended before the message reached it.
    Undelivered { client_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initializing,
    StartingThread,
    StartingTurn,
    Running,
    Ended,
}

/// What an outstanding request was.
#[derive(Debug, Clone, Copy)]
enum Pending {
    Initialize,
    Thread,
    Turn,
    Steer(usize),
    Interrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SteerState {
    /// Waiting for the turn id.
    Held,
    Sent,
    Accepted,
    /// Reported.
    Settled,
}

struct Steer {
    client_id: String,
    text: String,
    state: SteerState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    NotAsked,
    /// Asked before the turn id was known.
    Waiting,
    Sent,
}

/// One `TokenUsageBreakdown`.
#[derive(Debug, Clone, Copy, Default)]
struct Tokens {
    input: u64,
    cached: u64,
    cache_write: u64,
    output: u64,
    reasoning: u64,
}

impl Tokens {
    fn of(v: &Value) -> Option<Self> {
        let n = |key: &str| v.get(key).and_then(Value::as_u64).unwrap_or(0);
        let input = v.get("inputTokens")?.as_u64()?;
        let output = v.get("outputTokens")?.as_u64()?;
        let cached = n("cachedInputTokens");
        if cached > input {
            return None;
        }
        Some(Self {
            input,
            cached,
            cache_write: n("cacheWriteInputTokens"),
            output,
            reasoning: n("reasoningOutputTokens"),
        })
    }

    fn less(self, o: Self) -> Self {
        Self {
            input: self.input.saturating_sub(o.input),
            cached: self.cached.saturating_sub(o.cached),
            cache_write: self.cache_write.saturating_sub(o.cache_write),
            output: self.output.saturating_sub(o.output),
            reasoning: self.reasoning.saturating_sub(o.reasoning),
        }
    }
}

/// One Codex turn over `codex app-server` (module doc).
pub struct AppSession {
    params: StartParams,
    phase: Phase,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    turn_started: bool,
    steers: Vec<Steer>,
    updates: Vec<SteerUpdate>,
    user_items: HashSet<String>,
    prompt_seen: bool,
    stop: Stop,
    /// What the thread held before this turn, and its latest total.
    usage: Option<(Tokens, Tokens)>,
    /// The running todo list: its id and items.
    todo: Option<(String, Value)>,
    next_item: u64,
    /// The last error the server said it would retry, for an
    /// [`AppSession::ended_early`] turn.
    last_retry: Option<String>,
    /// An error the server won't retry was already passed on as exec's
    /// `error` line: that is the turn's failure.
    reported_error: bool,
    /// One bounded notice per turn; repeated approval requests stay declined.
    approval_warned: bool,
}

impl AppSession {
    /// A session and its first write, the `initialize` request.
    pub fn start(params: StartParams) -> (Self, Vec<String>) {
        let mut session = Self {
            params,
            phase: Phase::Initializing,
            next_id: 1,
            pending: HashMap::new(),
            thread_id: None,
            turn_id: None,
            turn_started: false,
            steers: Vec::new(),
            updates: Vec::new(),
            user_items: HashSet::new(),
            prompt_seen: false,
            stop: Stop::NotAsked,
            usage: None,
            todo: None,
            next_item: 0,
            last_retry: None,
            reported_error: false,
            approval_warned: false,
        };
        let init = session.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": CLIENT_NAME,
                    "title": null,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {"experimentalApi": false, "requestAttestation": false},
            }),
            Pending::Initialize,
        );
        (session, vec![init])
    }

    /// The turn is over (or never started).
    pub fn finished(&self) -> bool {
        self.phase == Phase::Ended
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn thread_id(&self) -> Option<&str> {
        self.thread_id.as_deref()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    /// One line of the server's stdout. Anything that isn't a JSON object
    /// is ignored.
    pub fn on_line(&mut self, line: &str) -> Step {
        let mut step = Step::default();
        if let Ok(msg) = serde_json::from_str::<Value>(line.trim()) {
            let method = msg.get("method").and_then(Value::as_str);
            let id = msg.get("id").filter(|v| !v.is_null());
            let params = msg.get("params").unwrap_or(&Value::Null);
            match (method, id) {
                (Some(method), Some(id)) => {
                    step.writes.push(answer_request(method, id));
                    // v2 approvals identify their turn. Legacy or pre-turn
                    // requests are still refused, but never attributed to an
                    // unknown turn just to show a warning.
                    if method.ends_with("/requestApproval")
                        && !self.approval_warned
                        && self.phase != Phase::Ended
                        && self.active(params, params.get("turnId").and_then(Value::as_str))
                    {
                        self.approval_warned = true;
                        step.exec_lines.push(json!({"type": "item.completed", "item": {
                            "id": self.item_id(), "type": "error",
                            "message": "Codex requested approval that Hollow cannot provide here. The request was declined. Review the task and permissions before trying again."
                        }}).to_string());
                    }
                }
                (Some(method), None) if self.phase != Phase::Ended => {
                    self.on_notification(method, params, &mut step)
                }
                (None, Some(id)) => self.on_response(id, &msg, &mut step),
                _ => {}
            }
        }
        step.finished = self.finished();
        step
    }

    /// Steers the running turn with `text`; `client_id` names it in
    /// [`Self::steer_events`]. Before the turn id is known the steer is
    /// held (no writes) and written once `turn/start` is answered.
    pub fn steer(&mut self, client_id: &str, text: &str) -> Result<Vec<String>, SteerRefusal> {
        if self.finished() || self.stop != Stop::NotAsked {
            return Err(SteerRefusal::Ended);
        }
        if text.trim().is_empty() {
            return Err(SteerRefusal::Empty);
        }
        self.steers.push(Steer {
            client_id: client_id.to_string(),
            text: text.to_string(),
            state: SteerState::Held,
        });
        // Any steer still held goes first: the steers keep their order.
        Ok(self.take_held_steers())
    }

    /// The held steers' `turn/steer` writes, oldest first, once the turn id
    /// is known ([`Step::held_steers`]); nothing before that, or once a
    /// stop was asked.
    pub fn take_held_steers(&mut self) -> Vec<String> {
        if self.turn_id.is_none() || self.stop != Stop::NotAsked || self.finished() {
            return Vec::new();
        }
        (0..self.steers.len())
            .filter(|&i| self.steers[i].state == SteerState::Held)
            .collect::<Vec<_>>()
            .into_iter()
            .map(|i| self.send_steer(i))
            .collect()
    }

    /// Stops the turn: `turn/interrupt`, once. (A chat turn's Stop ends the
    /// process group instead, as for exec.) Asked before the turn id is
    /// known it is written when `turn/start` is answered; asked before
    /// `turn/start` was written, the session just ends.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn interrupt(&mut self) -> Vec<String> {
        if self.finished() || self.stop != Stop::NotAsked {
            return Vec::new();
        }
        if self.turn_id.is_some() {
            return vec![self.send_interrupt()];
        }
        if self.phase == Phase::StartingTurn {
            self.stop = Stop::Waiting;
            self.settle_held();
        } else {
            self.stop = Stop::Sent;
            self.end();
        }
        Vec::new()
    }

    /// Hollow is stopping the turn some other way (a chat turn's Stop ends
    /// the process group): an `interrupted` completion the server prints as
    /// it shuts down is that stop, not a failure. Writes nothing; held
    /// steers are reported undelivered.
    pub fn note_stopping(&mut self) {
        if self.stop == Stop::NotAsked {
            self.stop = Stop::Sent;
            self.settle_held();
        }
    }

    /// The server's output ended (it exited on its own) before the turn
    /// did: exec's `turn.failed` saying so, with the last retried error for
    /// context, and the session ends. `None` once the turn ended, and when
    /// an error the server won't retry was already reported: that error
    /// stays the turn's result (its words decide, e.g., the outage backoff).
    pub fn ended_early(&mut self) -> Option<String> {
        if self.finished() {
            return None;
        }
        if self.reported_error {
            self.end();
            return None;
        }
        let mut message =
            "Codex exited before the turn finished. Continue to try again, or open Agent updates."
                .to_string();
        if let Some(error) = &self.last_retry {
            message.push_str(&format!(" (last error: {error})"));
        }
        self.end();
        Some(json!({"type": "turn.failed", "error": {"message": message}}).to_string())
    }

    /// Each steer's outcome since the last call, oldest first.
    pub fn steer_events(&mut self) -> Vec<SteerUpdate> {
        std::mem::take(&mut self.updates)
    }

    fn request(&mut self, method: &str, params: Value, kind: Pending) -> String {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, kind);
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
    }

    fn send_steer(&mut self, i: usize) -> String {
        let steer = &mut self.steers[i];
        steer.state = SteerState::Sent;
        let params = json!({
            "threadId": self.thread_id,
            "expectedTurnId": self.turn_id,
            "clientUserMessageId": steer.client_id,
            "input": [text_input(&steer.text)],
        });
        self.request("turn/steer", params, Pending::Steer(i))
    }

    fn send_interrupt(&mut self) -> String {
        self.stop = Stop::Sent;
        // A steer still held never goes after a stop.
        self.settle_held();
        let params = json!({"threadId": self.thread_id, "turnId": self.turn_id});
        self.request("turn/interrupt", params, Pending::Interrupt)
    }

    fn on_response(&mut self, id: &Value, msg: &Value, step: &mut Step) {
        let Some(kind) = id.as_u64().and_then(|id| self.pending.remove(&id)) else {
            return;
        };
        let error = msg.get("error").filter(|e| !e.is_null()).map(|e| {
            e.get("message")
                .and_then(Value::as_str)
                .map_or_else(|| e.to_string(), str::to_string)
        });
        let result = msg.get("result").unwrap_or(&Value::Null);
        match kind {
            Pending::Steer(i) => {
                let steer = &mut self.steers[i];
                if steer.state == SteerState::Settled {
                    return;
                }
                if let Some(reason) = error {
                    steer.state = SteerState::Settled;
                    self.updates.push(SteerUpdate::Refused {
                        client_id: steer.client_id.clone(),
                        reason,
                    });
                } else {
                    steer.state = SteerState::Accepted;
                }
            }
            // Its outcome is the turn's `turn/completed`; an error means the
            // turn had already ended.
            Pending::Interrupt => {}
            _ if self.finished() => {}
            Pending::Initialize => match error {
                Some(e) => self.fail("initialize", &e, step),
                None => {
                    step.writes
                        .push(json!({"jsonrpc": "2.0", "method": "initialized"}).to_string());
                    let write = self.thread_request();
                    step.writes.push(write);
                    self.phase = Phase::StartingThread;
                }
            },
            Pending::Thread => {
                let method = self.thread_method();
                let thread = result
                    .pointer("/thread/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match (error, thread) {
                    (Some(e), _) => self.fail(method, &e, step),
                    (None, None) => self.fail(method, "the answer named no thread", step),
                    (None, Some(thread)) => {
                        step.exec_lines.push(
                            json!({"type": "thread.started", "thread_id": thread}).to_string(),
                        );
                        self.thread_id = Some(thread);
                        let params = json!({
                            "threadId": self.thread_id,
                            "input": [text_input(&self.params.prompt)],
                        });
                        let write = self.request("turn/start", params, Pending::Turn);
                        step.writes.push(write);
                        self.phase = Phase::StartingTurn;
                    }
                }
            }
            Pending::Turn => {
                let turn = result
                    .pointer("/turn/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match (error, turn) {
                    (Some(e), _) => self.fail("turn/start", &e, step),
                    (None, None) => self.fail("turn/start", "the answer named no turn", step),
                    (None, Some(turn)) => self.turn_known(turn, step),
                }
            }
        }
    }

    fn thread_method(&self) -> &'static str {
        if self.params.resume.is_some() {
            "thread/resume"
        } else {
            "thread/start"
        }
    }

    fn thread_request(&mut self) -> String {
        let p = &self.params;
        let mut params = Map::new();
        if let Some(thread) = &p.resume {
            params.insert("threadId".into(), json!(thread));
            params.insert("excludeTurns".into(), json!(true));
        }
        if let Some(model) = p.model.as_deref().filter(|m| !m.is_empty()) {
            params.insert("model".into(), json!(model));
        }
        if !p.cwd.is_empty() {
            params.insert("cwd".into(), json!(p.cwd));
        }
        params.insert("approvalPolicy".into(), json!(p.approval_policy));
        if let Some(reviewer) = &p.approvals_reviewer {
            params.insert("approvalsReviewer".into(), json!(reviewer));
        }
        // Empty: the run's permission profile (config) applies; a thread
        // `sandbox` would override it.
        if !p.sandbox.is_empty() {
            params.insert("sandbox".into(), json!(p.sandbox));
        }
        if !p.config.is_empty() {
            params.insert("config".into(), Value::Object(p.config.clone()));
        }
        if p.ephemeral && p.resume.is_none() {
            params.insert("ephemeral".into(), json!(true));
        }
        let method = self.thread_method();
        self.request(method, Value::Object(params), Pending::Thread)
    }

    /// The turn id is known (`turn/start`'s answer, or `turn/started` if it
    /// came first; called for both): a waiting stop goes out, else the held
    /// steers may (through the caller's writer, [`Step::held_steers`]).
    fn turn_known(&mut self, turn: String, step: &mut Step) {
        if self.turn_id.is_none() {
            self.turn_id = Some(turn);
        }
        self.phase = Phase::Running;
        match self.stop {
            Stop::Waiting => {
                let write = self.send_interrupt();
                step.writes.push(write);
            }
            // A stop already went (on the other of the two): nothing more.
            Stop::Sent => {}
            Stop::NotAsked => {
                step.held_steers |= self.steers.iter().any(|s| s.state == SteerState::Held);
            }
        }
    }

    /// Every held steer is reported undelivered (a stop was asked).
    fn settle_held(&mut self) {
        for steer in &mut self.steers {
            if steer.state == SteerState::Held {
                steer.state = SteerState::Settled;
                self.updates.push(SteerUpdate::Undelivered {
                    client_id: steer.client_id.clone(),
                });
            }
        }
    }

    /// The handshake failed: the turn ends with the server's words.
    fn fail(&mut self, what: &str, message: &str, step: &mut Step) {
        let action = match what {
            "initialize" => "connect",
            "thread/start" => "start the conversation",
            "thread/resume" => "resume the conversation",
            _ => "start the turn",
        };
        let detail = message.trim();
        let punctuation = if detail.ends_with(['.', '!', '?']) {
            ""
        } else {
            "."
        };
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(" {detail}{punctuation}")
        };
        let message = format!(
            "Codex could not {action}.{detail} Continue to try again, or open Agent updates."
        );
        step.exec_lines
            .push(json!({"type": "turn.failed", "error": {"message": message}}).to_string());
        self.end();
    }

    fn end(&mut self) {
        self.phase = Phase::Ended;
        for steer in &mut self.steers {
            if steer.state != SteerState::Settled {
                steer.state = SteerState::Settled;
                self.updates.push(SteerUpdate::Undelivered {
                    client_id: steer.client_id.clone(),
                });
            }
        }
    }

    fn our_thread(&self, params: &Value) -> bool {
        match params.get("threadId").and_then(Value::as_str) {
            Some(thread) => Some(thread) == self.thread_id.as_deref(),
            None => true,
        }
    }

    /// Whether a notification belongs to the active turn.
    fn active(&self, params: &Value, turn: Option<&str>) -> bool {
        self.our_thread(params) && turn.is_some() && turn == self.turn_id.as_deref()
    }

    fn on_notification(&mut self, method: &str, params: &Value, step: &mut Step) {
        let turn_id = params.get("turnId").and_then(Value::as_str);
        match method {
            "account/rateLimits/updated" => {
                if let Some(limits) = params.get("rateLimits").filter(|v| v.is_object()) {
                    step.exec_lines
                        .push(json!({"type": "account.rate_limits", "limits": limits}).to_string());
                }
            }
            "turn/started" => {
                let Some(turn) = params.pointer("/turn/id").and_then(Value::as_str) else {
                    return;
                };
                if self.turn_id.is_none()
                    && self.phase == Phase::StartingTurn
                    && self.our_thread(params)
                {
                    self.turn_known(turn.to_string(), step);
                }
                if self.active(params, Some(turn)) && !self.turn_started {
                    self.turn_started = true;
                    step.exec_lines
                        .push(json!({"type": "turn.started"}).to_string());
                }
            }
            "turn/completed" => {
                let turn = params.pointer("/turn/id").and_then(Value::as_str);
                if self.active(params, turn) {
                    let turn = params.get("turn").cloned().unwrap_or(Value::Null);
                    self.turn_completed(&turn, step);
                }
            }
            "item/started" | "item/completed" if self.active(params, turn_id) => {
                let item = params.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("userMessage") {
                    self.user_message(item);
                } else if let Some(item) = exec_item(item) {
                    let kind = if method == "item/started" {
                        "item.started"
                    } else {
                        "item.completed"
                    };
                    step.exec_lines
                        .push(json!({"type": kind, "item": item}).to_string());
                }
            }
            "thread/tokenUsage/updated" if self.active(params, turn_id) => {
                let usage = params.get("tokenUsage").unwrap_or(&Value::Null);
                let Some(total) = Tokens::of(usage.get("total").unwrap_or(&Value::Null)) else {
                    return;
                };
                let Some(last) = Tokens::of(usage.get("last").unwrap_or(&Value::Null)) else {
                    return;
                };
                let before = self.usage.map_or_else(|| total.less(last), |(b, _)| b);
                self.usage = Some((before, total));
                // Publish while running: Stop may kill the process before
                // it can print a terminal notification.
                step.exec_lines
                    .push(json!({"type": "turn.usage", "usage": self.turn_usage()}).to_string());
            }
            "turn/plan/updated" if self.active(params, turn_id) => {
                let items: Vec<Value> = params
                    .get("plan")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|s| {
                        json!({
                            "text": s.get("step").and_then(Value::as_str).unwrap_or(""),
                            "completed": s.get("status").and_then(Value::as_str) == Some("completed"),
                        })
                    })
                    .collect();
                let items = Value::Array(items);
                let kind = if let Some((_, current)) = &mut self.todo {
                    *current = items;
                    "item.updated"
                } else {
                    let id = self.item_id();
                    self.todo = Some((id, items));
                    "item.started"
                };
                let line = self.todo_line(kind);
                step.exec_lines.push(line);
            }
            // An error the server retries itself ("Reconnecting… 1/5") is
            // not the turn's end: exec's parser would read it as a failed
            // result. Only one it won't retry, or `turn/completed`, is. Its
            // words are kept in case the server then exits.
            "error"
                if self.active(params, turn_id)
                    && params.get("willRetry").and_then(Value::as_bool) == Some(true) =>
            {
                self.last_retry = params
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            "error"
                if self.active(params, turn_id)
                    && params.get("willRetry").and_then(Value::as_bool) != Some(true) =>
            {
                let message = params
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("codex error");
                self.reported_error = true;
                step.exec_lines
                    .push(json!({"type": "error", "message": message}).to_string());
            }
            "warning" if self.our_thread(params) => {
                let message = params.get("message").and_then(Value::as_str).unwrap_or("");
                let item = json!({"id": self.item_id(), "type": "error", "message": message});
                step.exec_lines
                    .push(json!({"type": "item.completed", "item": item}).to_string());
            }
            _ => {}
        }
    }

    fn item_id(&mut self) -> String {
        let id = format!("item_{}", self.next_item);
        self.next_item += 1;
        id
    }

    fn todo_line(&self, kind: &str) -> String {
        let (id, items) = self.todo.as_ref().expect("a todo list");
        json!({"type": kind, "item": {"id": id, "type": "todo_list", "items": items}}).to_string()
    }

    fn turn_completed(&mut self, turn: &Value, step: &mut Step) {
        let status = turn.get("status").and_then(Value::as_str).unwrap_or("");
        if status == "inProgress" {
            return;
        }
        if status != "interrupted" && self.todo.is_some() {
            let line = self.todo_line("item.completed");
            step.exec_lines.push(line);
        }
        match status {
            "completed" => {
                // No usage notification: no usage, never a measured zero.
                let line = match self.usage {
                    Some(_) => json!({"type": "turn.completed", "usage": self.turn_usage()}),
                    None => json!({"type": "turn.completed"}),
                };
                step.exec_lines.push(line.to_string());
            }
            // Hollow's own stop: nothing, as exec's SIGINT leaves nothing
            // (and a chat turn's Stop ends the process group anyway).
            "interrupted" if self.stop != Stop::NotAsked => {}
            other => {
                let error = turn.get("error").unwrap_or(&Value::Null);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .filter(|m| !m.trim().is_empty())
                    .map_or_else(
                        || match other {
                            // Codex ended the turn itself: never a success.
                            "interrupted" => {
                                "Codex interrupted the turn before it finished. Continue to try again.".to_string()
                            }
                            _ => "Codex ended the turn without saying why. Continue to try again.".to_string(),
                        },
                        str::to_string,
                    );
                // Keep an unknown future status available for diagnostics,
                // without exposing its protocol spelling as user copy.
                let mut out = json!({"message": message, "status": other});
                if let Some(info) = error.get("codexErrorInfo").and_then(error_info) {
                    out["codex_error_info"] = json!(info);
                }
                step.exec_lines
                    .push(json!({"type": "turn.failed", "error": out}).to_string());
            }
        }
        self.end();
    }

    fn turn_usage(&self) -> Value {
        let Some((before, total)) = self.usage else {
            return Value::Null;
        };
        let t = total.less(before);
        json!({
            "input_tokens": t.input,
            "cached_input_tokens": t.cached,
            "cache_write_input_tokens": t.cache_write,
            "output_tokens": t.output,
            "reasoning_output_tokens": t.reasoning,
        })
    }

    /// A `userMessage` item of the active turn: the prompt or a steer.
    fn user_message(&mut self, item: &Value) {
        let id = item.get("id").and_then(Value::as_str).unwrap_or("");
        if !self.user_items.insert(id.to_string()) {
            return;
        }
        let open = |s: &Steer| matches!(s.state, SteerState::Sent | SteerState::Accepted);
        if let Some(client) = item.get("clientId").and_then(Value::as_str) {
            if let Some(i) = self
                .steers
                .iter()
                .position(|s| open(s) && s.client_id == client)
            {
                return self.delivered(i);
            }
        }
        let text: Vec<&str> = item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect();
        let text = text.join("\n");
        if !self.prompt_seen && text == self.params.prompt {
            self.prompt_seen = true;
            return;
        }
        if let Some(i) = self.steers.iter().position(|s| open(s) && s.text == text) {
            return self.delivered(i);
        }
        if !self.prompt_seen {
            self.prompt_seen = true;
            return;
        }
        if let Some(i) = self
            .steers
            .iter()
            .position(|s| s.state == SteerState::Accepted)
        {
            self.delivered(i);
        }
    }

    fn delivered(&mut self, i: usize) {
        let steer = &mut self.steers[i];
        steer.state = SteerState::Settled;
        self.updates.push(SteerUpdate::Delivered {
            client_id: steer.client_id.clone(),
        });
    }
}

fn text_input(text: &str) -> Value {
    json!({"type": "text", "text": text, "text_elements": []})
}

/// The answer to a server request (module doc: `codex exec` can't ask, so
/// nothing is ever approved).
fn answer_request(method: &str, id: &Value) -> String {
    let result = match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({"decision": "decline"})
        }
        "item/permissions/requestApproval" => json!({"permissions": {}, "scope": "turn"}),
        "item/tool/requestUserInput" => json!({"answers": {}}),
        "mcpServer/elicitation/request" => {
            json!({"action": "decline", "content": null, "_meta": null})
        }
        "item/tool/call" => json!({"contentItems": [], "success": false}),
        "execCommandApproval" | "applyPatchApproval" => {
            json!({"decision": {"denied": {"rejection": NO_APPROVALS}}})
        }
        _ => {
            let message = format!("Hollow does not answer {method}");
            return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}})
                .to_string();
        }
    };
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

/// `inProgress` → `in_progress`.
fn snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn snake_status(item: &Value) -> Value {
    item.get("status")
        .and_then(Value::as_str)
        .map_or(Value::Null, |s| json!(snake(s)))
}

/// `codexErrorInfo` as exec's `codex_error_info`: `serverOverloaded` →
/// `server_overloaded`; a variant with fields by its name.
fn error_info(info: &Value) -> Option<String> {
    match info {
        Value::String(s) => Some(snake(s)),
        Value::Object(map) if map.len() == 1 => map.keys().next().map(|k| snake(k)),
        _ => None,
    }
}

/// An app-server `ThreadItem` as the `codex exec --json` item of the same
/// kind (module doc), or `None` for one exec has no name for.
fn exec_item(item: &Value) -> Option<Value> {
    let id = item.get("id").cloned().unwrap_or(Value::Null);
    let field = |key: &str| item.get(key).cloned().unwrap_or(Value::Null);
    let text = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    Some(match item.get("type").and_then(Value::as_str)? {
        "agentMessage" => json!({"id": id, "type": "agent_message", "text": text("text")}),
        "reasoning" => {
            let summary: Vec<&str> = item
                .get("summary")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            json!({"id": id, "type": "reasoning", "text": summary.join("\n")})
        }
        "commandExecution" => json!({
            "id": id,
            "type": "command_execution",
            "command": text("command"),
            "aggregated_output": text("aggregatedOutput"),
            "exit_code": field("exitCode"),
            "status": snake_status(item),
        }),
        "fileChange" => {
            let changes: Vec<Value> = item
                .get("changes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|c| {
                    json!({
                        "path": c.get("path").cloned().unwrap_or(Value::Null),
                        "kind": c.pointer("/kind/type").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect();
            // Exec has no `declined`: a change not applied is `failed`.
            let status = match item.get("status").and_then(Value::as_str) {
                Some("declined") => json!("failed"),
                _ => snake_status(item),
            };
            json!({"id": id, "type": "file_change", "changes": changes, "status": status})
        }
        "mcpToolCall" => {
            let result = item.get("result").filter(|r| !r.is_null()).map_or(
                Value::Null,
                |r| json!({"content": r.get("content").cloned().unwrap_or(Value::Null),
                    "structured_content": r.get("structuredContent").cloned().unwrap_or(Value::Null)}),
            );
            json!({
                "id": id,
                "type": "mcp_tool_call",
                "server": field("server"),
                "tool": field("tool"),
                "arguments": field("arguments"),
                "result": result,
                "error": field("error"),
                "status": snake_status(item),
            })
        }
        "webSearch" => json!({
            "id": id,
            "type": "web_search",
            "query": text("query"),
            "action": field("action"),
        }),
        "collabAgentToolCall" => {
            let mut out = Map::new();
            for (key, value) in item.as_object()? {
                out.insert(snake(key), value.clone());
            }
            out.insert("type".into(), json!("collab_tool_call"));
            if let Some(status) = item.get("status") {
                out.insert(
                    "status".into(),
                    status.as_str().map_or(status.clone(), |s| json!(snake(s))),
                );
            }
            Value::Object(out)
        }
        _ => return None,
    })
}

#[cfg(test)]
#[path = "codex_app_server_tests.rs"]
mod tests;
