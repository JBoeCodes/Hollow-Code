//! Tests for [`super`]: the recorded app-server sessions (codex-cli 0.163.0,
//! `fixtures/codex/app-server-*.jsonl`, the server's lines only) replayed
//! through [`AppSession`], and what it writes back checked line by line.

use super::*;
use crate::agent::{parse_line, AgentEvent, AgentKind, ParseState};
use serde_json::json;

const STEER: &str = include_str!("../fixtures/codex/app-server-steer.jsonl");
const INTERRUPT: &str = include_str!("../fixtures/codex/app-server-interrupt.jsonl");
const RESUME: &str = include_str!("../fixtures/codex/app-server-resume.jsonl");
const APPROVAL: &str = include_str!("../fixtures/codex/app-server-approval.jsonl");

const STEER_THREAD: &str = "01a15f7d-8b34-7e63-b0a1-f15c3f526973";
const STEER_TURN: &str = "01a15f7d-8bb7-7692-aeed-469e0f1f2c66";
const STEER_PROMPT: &str =
    "Run the shell command `sleep 8; echo done` and then reply with its output in one short sentence.";
const RESUME_THREAD: &str = "01a15f7e-3005-7ec2-b6ba-71a12406ea84";
const RESUME_OLD_TURN: &str = "01a15f7e-307c-7cc3-b3b6-6eee2b3450e4";
const RESUME_TURN: &str = "01a15f7f-e3c6-79e3-bb54-398df6c7ad2c";

fn params(prompt: &str) -> StartParams {
    let mut config = Map::new();
    config.insert("model_reasoning_effort".into(), json!("low"));
    StartParams {
        prompt: prompt.into(),
        resume: None,
        cwd: "/tmp/probe".into(),
        model: Some("gpt-6-luna".into()),
        approval_policy: "never".into(),
        approvals_reviewer: None,
        sandbox: "workspace-write".into(),
        ephemeral: false,
        config,
    }
}

fn parsed(line: &str) -> Value {
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"))
}

fn methods(writes: &[Value]) -> Vec<&str> {
    writes
        .iter()
        .map(|w| w["method"].as_str().unwrap_or("<response>"))
        .collect()
}

/// A recorded session fed line by line, with everything it produced.
struct Replay {
    session: AppSession,
    lines: Vec<&'static str>,
    at: usize,
    exec: Vec<String>,
    writes: Vec<Value>,
    finished: bool,
}

impl Replay {
    fn new(fixture: &'static str, params: StartParams) -> Self {
        let (session, first) = AppSession::start(params);
        Self {
            session,
            lines: fixture.lines().filter(|l| !l.trim().is_empty()).collect(),
            at: 0,
            exec: Vec::new(),
            writes: first.iter().map(|w| parsed(w)).collect(),
            finished: false,
        }
    }

    fn take(&mut self, step: Step) -> Step {
        self.exec.extend(step.exec_lines.iter().cloned());
        self.writes.extend(step.writes.iter().map(|w| parsed(w)));
        self.finished = step.finished;
        step
    }

    fn feed(&mut self, line: &str) -> Step {
        let step = self.session.on_line(line);
        self.take(step)
    }

    /// Feeds lines up to and including the first `pick` matches.
    fn feed_until(&mut self, pick: impl Fn(&Value) -> bool) -> Step {
        while self.at < self.lines.len() {
            let line = self.lines[self.at];
            self.at += 1;
            let step = self.feed(line);
            if pick(&parsed(line)) {
                return step;
            }
        }
        panic!("no line matched");
    }

    fn feed_rest(&mut self) {
        while self.at < self.lines.len() {
            let line = self.lines[self.at];
            self.at += 1;
            self.feed(line);
        }
    }

    fn steer(&mut self, id: &str, text: &str) -> Vec<Value> {
        let writes: Vec<Value> = self
            .session
            .steer(id, text)
            .expect("steer accepted")
            .iter()
            .map(|w| parsed(w))
            .collect();
        self.writes.extend(writes.iter().cloned());
        writes
    }

    fn exec_values(&self) -> Vec<Value> {
        self.exec.iter().map(|l| parsed(l)).collect()
    }
}

fn is_item(line: &Value, method: &str, kind: &str) -> bool {
    line["method"] == method && line["params"]["item"]["type"] == kind
}

fn command_started(line: &Value) -> bool {
    is_item(line, "item/started", "commandExecution")
}

fn response(id: u64) -> impl Fn(&Value) -> bool {
    move |line: &Value| line.get("method").is_none() && line["id"] == id
}

#[test]
fn handshake_waits_for_each_response() {
    let (mut s, first) = AppSession::start(params(STEER_PROMPT));
    assert_eq!(first.len(), 1);
    let init = parsed(&first[0]);
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["id"], 1);
    assert_eq!(init["method"], "initialize");
    assert_eq!(init["params"]["clientInfo"]["name"], "hollow");

    let lines: Vec<&str> = STEER.lines().collect();
    // A notification before the answer writes nothing.
    let early = s.on_line(r#"{"method":"remoteControl/status/changed","params":{}}"#);
    assert!(early.writes.is_empty() && early.exec_lines.is_empty());

    let step = s.on_line(lines[0]);
    let writes: Vec<Value> = step.writes.iter().map(|w| parsed(w)).collect();
    assert_eq!(methods(&writes), ["initialized", "thread/start"]);
    assert!(
        writes[0].get("id").is_none(),
        "initialized is a notification"
    );
    let start = &writes[1];
    assert_eq!(start["id"], 2);
    assert_eq!(start["params"]["cwd"], "/tmp/probe");
    assert_eq!(start["params"]["model"], "gpt-6-luna");
    assert_eq!(start["params"]["approvalPolicy"], "never");
    assert_eq!(start["params"]["sandbox"], "workspace-write");
    assert_eq!(start["params"]["config"]["model_reasoning_effort"], "low");
    assert!(step.exec_lines.is_empty());

    let step = s.on_line(lines[1]);
    assert_eq!(
        step.exec_lines,
        [json!({"type": "thread.started", "thread_id": STEER_THREAD}).to_string()]
    );
    let writes: Vec<Value> = step.writes.iter().map(|w| parsed(w)).collect();
    assert_eq!(methods(&writes), ["turn/start"]);
    assert_eq!(writes[0]["id"], 3);
    assert_eq!(writes[0]["params"]["threadId"], STEER_THREAD);
    assert_eq!(
        writes[0]["params"]["input"],
        json!([{"type": "text", "text": STEER_PROMPT, "text_elements": []}])
    );
    // thread/started (a notification) and turn/start's answer write nothing.
    assert!(s.on_line(lines[2]).writes.is_empty());
    let step = s.on_line(lines[3]);
    assert!(step.writes.is_empty() && !step.finished);
}

#[test]
fn ephemeral_and_reviewer_reach_thread_start() {
    let mut p = params("x");
    p.ephemeral = true;
    p.approvals_reviewer = Some("auto_review".into());
    p.approval_policy = "on-request".into();
    p.model = None;
    p.config = Map::new();
    let (mut s, _) = AppSession::start(p);
    let step = s.on_line(STEER.lines().next().unwrap());
    let start = parsed(&step.writes[1]);
    assert_eq!(start["params"]["ephemeral"], true);
    assert_eq!(start["params"]["approvalsReviewer"], "auto_review");
    assert_eq!(start["params"]["approvalPolicy"], "on-request");
    assert!(
        start["params"].get("model").is_none(),
        "no model, none sent"
    );
    assert!(
        start["params"].get("config").is_none(),
        "empty config left out"
    );
}

#[test]
fn resume_uses_thread_resume() {
    let mut p = params("Run the shell command `sleep 20; echo second` and reply with its output.");
    p.resume = Some(RESUME_THREAD.into());
    let mut r = Replay::new(RESUME, p);
    r.feed_until(response(1));
    assert_eq!(
        methods(&r.writes),
        ["initialize", "initialized", "thread/resume"]
    );
    let resume = &r.writes[2];
    assert_eq!(resume["id"], 2);
    assert_eq!(resume["params"]["threadId"], RESUME_THREAD);
    assert_eq!(resume["params"]["excludeTurns"], true);
    assert_eq!(resume["params"]["sandbox"], "workspace-write");
    assert_eq!(resume["params"]["approvalPolicy"], "never");

    r.feed_until(response(2));
    assert_eq!(
        r.exec,
        [json!({"type": "thread.started", "thread_id": RESUME_THREAD}).to_string()]
    );
    assert_eq!(r.writes.last().unwrap()["method"], "turn/start");
    // The resumed thread's stale usage line names the old turn: never adopted.
    r.feed_until(response(3));
    assert_eq!(r.session.thread_id(), Some(RESUME_THREAD));
    assert_eq!(r.session.turn_id(), Some(RESUME_TURN));
    let writes = r.session.interrupt();
    let interrupt = parsed(&writes[0]);
    assert_eq!(interrupt["params"]["turnId"], RESUME_TURN);
    assert_ne!(interrupt["params"]["turnId"], RESUME_OLD_TURN);
}

#[test]
fn resumed_turn_usage_is_this_turns_only() {
    let mut p = params("x");
    p.resume = Some(RESUME_THREAD.into());
    let mut r = Replay::new(RESUME, p);
    // Up to the new turn's first usage line (after the command started).
    r.feed_until(command_started);
    r.feed_until(|l| l["method"] == "thread/tokenUsage/updated");
    r.feed(
        &json!({"method": "turn/completed", "params": {"threadId": RESUME_THREAD,
            "turn": {"id": RESUME_TURN, "items": [], "status": "completed", "error": null}}})
        .to_string(),
    );
    assert!(r.finished);
    let last = r.exec_values().pop().unwrap();
    assert_eq!(last["type"], "turn.completed");
    // total 57020/35328/114 minus what the thread held before (28347/7168/57).
    assert_eq!(last["usage"]["input_tokens"], 28673);
    assert_eq!(last["usage"]["cached_input_tokens"], 28160);
    assert_eq!(last["usage"]["output_tokens"], 57);
}

#[test]
fn a_full_turn_reads_as_codex_exec_json() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_rest();
    assert!(r.finished);
    let all = r.exec_values();
    assert_eq!(
        all.iter()
            .filter(|line| line["type"] == "turn.usage")
            .count(),
        3
    );
    let exec: Vec<_> = all
        .into_iter()
        .filter(|line| line["type"] != "turn.usage")
        .collect();
    let kinds: Vec<String> = exec
        .iter()
        .map(|l| match l["item"]["type"].as_str() {
            Some(item) => format!("{} {item}", l["type"].as_str().unwrap()),
            None => l["type"].as_str().unwrap().to_string(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "thread.started",
            "turn.started",
            "item.started command_execution",
            "item.completed command_execution",
            "item.started agent_message",
            "item.completed agent_message",
            "turn.completed",
        ]
    );
    let done = &exec[3]["item"];
    assert_eq!(done["id"], "exec-90708951-bf9f-4bb9-b368-d89d9ad5aa0e");
    assert_eq!(done["command"], "/bin/zsh -lc 'sleep 8; echo done'");
    assert_eq!(done["aggregated_output"], "done\n");
    assert_eq!(done["exit_code"], 0);
    assert_eq!(done["status"], "completed");
    assert_eq!(exec[2]["item"]["status"], "in_progress");
    assert_eq!(
        exec[6]["usage"],
        json!({"input_tokens": 85318, "cached_input_tokens": 68864,
            "cache_write_input_tokens": 0, "output_tokens": 85, "reasoning_output_tokens": 0})
    );
    // No user message ever becomes an exec line.
    assert!(r.exec.iter().all(|l| !l.contains("BANANA in your final")));

    let mut state = ParseState::default();
    let events: Vec<AgentEvent> = r
        .exec
        .iter()
        .flat_map(|l| parse_line(AgentKind::Codex, l, &mut state))
        .collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentEvent::TurnUsage(_)))
            .count(),
        3
    );
    let events: Vec<_> = events
        .into_iter()
        .filter(|event| !matches!(event, AgentEvent::TurnUsage(_)))
        .collect();
    assert_eq!(events.len(), 6, "{events:?}");
    assert!(matches!(&events[0], AgentEvent::Session(id) if id == STEER_THREAD));
    assert!(matches!(&events[1], AgentEvent::ToolUse { id, name, input }
        if id == "exec-90708951-bf9f-4bb9-b368-d89d9ad5aa0e" && name == "Bash"
            && input["command"] == "/bin/zsh -lc 'sleep 8; echo done'"));
    assert!(
        matches!(&events[2], AgentEvent::ToolResult { content, is_error: false, .. }
        if content == "done\n")
    );
    assert!(matches!(&events[3], AgentEvent::TurnBoundary));
    assert!(matches!(&events[4], AgentEvent::AssistantText(t)
        if t == "The output was `done`. BANANA CHERRY MANGO."));
    let AgentEvent::Result {
        is_error, usage, ..
    } = &events[5]
    else {
        panic!("{:?}", events[5]);
    };
    assert!(!is_error);
    let tokens = usage.tokens.expect("usage carried");
    assert_eq!(
        (tokens.input, tokens.cached_input, tokens.output),
        (16454, 68864, 85)
    );
}

#[test]
fn two_quick_steers_share_the_turn_and_both_arrive() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(command_started);
    let a = r.steer("s-1", "Also include the word BANANA in your final reply.");
    let b = r.steer("s-2", "And also include the word CHERRY.");
    for (writes, id, client) in [(&a, 4, "s-1"), (&b, 5, "s-2")] {
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0]["method"], "turn/steer");
        assert_eq!(writes[0]["id"], id);
        assert_eq!(writes[0]["params"]["threadId"], STEER_THREAD);
        assert_eq!(writes[0]["params"]["expectedTurnId"], STEER_TURN);
        assert_eq!(writes[0]["params"]["clientUserMessageId"], client);
        assert_eq!(writes[0]["params"]["input"][0]["type"], "text");
        assert_eq!(writes[0]["params"]["input"][0]["text_elements"], json!([]));
    }
    assert!(
        r.session.steer_events().is_empty(),
        "accepted, not yet seen"
    );
    r.feed_rest();
    assert_eq!(
        r.session.steer_events(),
        [
            SteerUpdate::Delivered {
                client_id: "s-1".into()
            },
            SteerUpdate::Delivered {
                client_id: "s-2".into()
            },
        ]
    );
    assert!(r.session.steer_events().is_empty(), "drained");
    assert!(r.exec.iter().all(|l| !l.contains("CHERRY.\"")));
}

#[test]
fn a_steer_matched_by_its_echoed_client_id() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(command_started);
    r.steer("mine", "words the server rewrote");
    r.feed(
        &json!({"method": "item/completed", "params": {"threadId": STEER_THREAD,
            "turnId": STEER_TURN, "item": {"type": "userMessage", "id": "u-9",
            "clientId": "mine", "content": [{"type": "text", "text": "other words"}]}}})
        .to_string(),
    );
    assert_eq!(
        r.session.steer_events(),
        [SteerUpdate::Delivered {
            client_id: "mine".into()
        }]
    );
}

#[test]
fn a_steer_before_the_turn_id_is_held_then_sent() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(response(2));
    let held = r
        .session
        .steer("early", "Also include the word BANANA in your final reply.");
    assert_eq!(held, Ok(Vec::new()), "nothing to steer yet");
    let step = r.feed_until(response(3));
    assert!(step.writes.is_empty(), "the reader never writes a steer");
    assert!(step.held_steers, "the writer takes them");
    let writes: Vec<Value> = r
        .session
        .take_held_steers()
        .iter()
        .map(|w| parsed(w))
        .collect();
    assert_eq!(methods(&writes), ["turn/steer"]);
    assert!(r.session.take_held_steers().is_empty(), "once");
    assert_eq!(writes[0]["id"], 4);
    assert_eq!(writes[0]["params"]["expectedTurnId"], STEER_TURN);
    assert_eq!(writes[0]["params"]["clientUserMessageId"], "early");
    r.feed_rest();
    assert_eq!(
        r.session.steer_events(),
        [SteerUpdate::Delivered {
            client_id: "early".into()
        }]
    );
}

#[test]
fn a_steer_after_the_turn_ended_is_refused_at_once() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_rest();
    assert!(r.session.finished());
    assert_eq!(
        r.session.steer("late", "late steer"),
        Err(SteerRefusal::Ended)
    );
    assert_eq!(r.session.steer("blank", "  "), Err(SteerRefusal::Ended));
    let (mut fresh, _) = AppSession::start(params("x"));
    assert_eq!(fresh.steer("blank", " \n"), Err(SteerRefusal::Empty));
}

#[test]
fn a_rejected_steer_is_reported_refused() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(command_started);
    r.steer("racing", "late steer");
    r.feed(r#"{"error":{"code":-32600,"message":"no active turn to steer"},"id":4}"#);
    assert_eq!(
        r.session.steer_events(),
        [SteerUpdate::Refused {
            client_id: "racing".into(),
            reason: "no active turn to steer".into()
        }]
    );
    // Its answer settled it: the turn's end reports nothing more for it.
    r.feed_rest();
    assert!(r.session.steer_events().is_empty());
}

#[test]
fn steers_the_turn_never_saw_are_undelivered_at_its_end() {
    let mut r = Replay::new(INTERRUPT, params("x"));
    r.feed_until(command_started);
    r.steer("never-seen", "a steer the turn never takes");
    r.session.interrupt();
    r.feed_rest();
    assert_eq!(
        r.session.steer_events(),
        [SteerUpdate::Undelivered {
            client_id: "never-seen".into()
        }]
    );
}

#[test]
fn an_approval_request_is_declined_and_the_steer_still_arrives() {
    let mut p =
        params("Run the shell command `sleep 2; echo approved-run` and reply with its output.");
    p.approval_policy = "untrusted".into();
    p.sandbox = "read-only".into();
    let mut r = Replay::new(APPROVAL, p);
    let step = r.feed_until(|l| l["method"] == "item/commandExecution/requestApproval");
    assert_eq!(step.writes.len(), 1);
    assert_eq!(step.exec_lines.len(), 1, "the declined request is visible");
    let warning = parsed(&step.exec_lines[0]);
    assert_eq!(warning["type"], "item.completed");
    assert_eq!(warning["item"]["type"], "error");
    assert!(warning["item"]["message"]
        .as_str()
        .unwrap()
        .contains("declined"));
    assert!(!step.exec_lines[0].contains("sleep 2"));
    let events = parse_line(
        AgentKind::Codex,
        &step.exec_lines[0],
        &mut ParseState::default(),
    );
    assert!(events.iter().any(|event| matches!(event,
        AgentEvent::ToolUse { name, input, .. }
        if name == "error" && input["message"].as_str().unwrap().contains("declined"))));
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentEvent::Result { .. })));
    assert_eq!(
        parsed(&step.writes[0]),
        json!({"jsonrpc": "2.0", "id": 0, "result": {"decision": "decline"}})
    );
    let steer = r.steer("kiwi", "Also include the word KIWI in your final reply.");
    assert_eq!(steer[0]["id"], 4);
    r.feed_rest();
    assert_eq!(
        r.session.steer_events(),
        [SteerUpdate::Delivered {
            client_id: "kiwi".into()
        }]
    );
    let last = r.exec_values().pop().unwrap();
    assert_eq!(last["type"], "turn.completed");
    // The probe's second turn (after this one ended) is never translated.
    assert!(r.exec.iter().all(|l| !l.contains("PINEAPPLE")));
}

#[test]
fn a_failed_turn_without_an_error_names_the_next_step() {
    let mut r = Replay::new(STEER, params("x"));
    r.feed_until(command_started);
    let step = r.feed(
        &json!({"method": "turn/completed", "params": {
            "threadId": STEER_THREAD, "turn": {"id": STEER_TURN, "status": "failed"}
        }})
        .to_string(),
    );
    let failed = step
        .exec_lines
        .iter()
        .map(|line| parsed(line))
        .find(|line| line["type"] == "turn.failed")
        .unwrap();
    assert_eq!(
        failed["error"]["message"],
        "Codex ended the turn without saying why. Continue to try again."
    );
    assert_eq!(failed["error"]["status"], "failed");
}

#[test]
fn approval_warnings_are_bounded_and_ignore_foreign_or_ended_turns() {
    let mut r = Replay::new(STEER, params("x"));
    r.feed_until(command_started);
    let request = |turn: &str| {
        json!({"id": 90, "method": "item/fileChange/requestApproval",
        "params": {"threadId": STEER_THREAD, "turnId": turn, "secret": "never show this"}})
        .to_string()
    };
    assert!(r.feed(&request("foreign")).exec_lines.is_empty());
    let first = r.feed(&request(STEER_TURN));
    assert_eq!(first.exec_lines.len(), 1);
    assert!(!first.exec_lines[0].contains("never show this"));
    let second = r.feed(&request(STEER_TURN));
    assert!(second.exec_lines.is_empty());
    assert_eq!(parsed(&second.writes[0])["result"]["decision"], "decline");
    r.feed_rest();
    assert!(r.feed(&request(STEER_TURN)).exec_lines.is_empty());
}

#[test]
fn every_server_request_gets_a_refusal_of_its_shape() {
    let (mut s, _) = AppSession::start(params("x"));
    let cases = [
        (
            "item/commandExecution/requestApproval",
            json!({"decision": "decline"}),
        ),
        (
            "item/fileChange/requestApproval",
            json!({"decision": "decline"}),
        ),
        (
            "item/permissions/requestApproval",
            json!({"permissions": {}, "scope": "turn"}),
        ),
        ("item/tool/requestUserInput", json!({"answers": {}})),
        (
            "mcpServer/elicitation/request",
            json!({"action": "decline", "content": null, "_meta": null}),
        ),
        (
            "item/tool/call",
            json!({"contentItems": [], "success": false}),
        ),
    ];
    for (n, (method, result)) in cases.iter().enumerate() {
        let step = s.on_line(&json!({"id": n, "method": method, "params": {}}).to_string());
        assert_eq!(
            parsed(&step.writes[0]),
            json!({"jsonrpc": "2.0", "id": n, "result": result}),
            "{method}"
        );
    }
    for method in ["execCommandApproval", "applyPatchApproval"] {
        let step = s.on_line(&json!({"id": "r-1", "method": method, "params": {}}).to_string());
        let answer = parsed(&step.writes[0]);
        assert_eq!(answer["id"], "r-1");
        assert!(answer["result"]["decision"]["denied"]["rejection"].is_string());
    }
    let step = s.on_line(r#"{"id":77,"method":"attestation/generate","params":{}}"#);
    let answer = parsed(&step.writes[0]);
    assert_eq!(answer["id"], 77);
    assert_eq!(answer["error"]["code"], -32601);
}

#[test]
fn an_interrupt_is_sent_once_and_ends_with_no_terminal_line() {
    let mut r = Replay::new(INTERRUPT, params("x"));
    r.feed_until(command_started);
    let writes: Vec<Value> = r.session.interrupt().iter().map(|w| parsed(w)).collect();
    assert_eq!(
        writes,
        [
            json!({"jsonrpc": "2.0", "id": 4, "method": "turn/interrupt",
            "params": {"threadId": RESUME_THREAD, "turnId": RESUME_OLD_TURN}})
        ]
    );
    assert!(r.session.interrupt().is_empty(), "never twice");
    r.feed_until(|l| l["method"] == "turn/completed");
    assert!(r.finished);
    assert!(r.session.interrupt().is_empty(), "nor after the end");
    r.feed_rest();
    let exec = r.exec_values();
    assert!(exec
        .iter()
        .all(|l| l["type"] != "turn.completed" && l["type"] != "turn.failed"));
    // The command finished 30 s after the turn did: dropped.
    assert!(exec
        .iter()
        .all(|l| l["type"] != "item.completed" || l["item"]["type"] != "command_execution"));
}

#[test]
fn an_interrupt_waits_for_the_turn_id() {
    let mut r = Replay::new(INTERRUPT, params("x"));
    r.feed_until(response(2));
    assert!(r.session.interrupt().is_empty());
    let step = r.feed_until(response(3));
    let writes: Vec<Value> = step.writes.iter().map(|w| parsed(w)).collect();
    assert_eq!(methods(&writes), ["turn/interrupt"]);
    assert_eq!(writes[0]["params"]["turnId"], RESUME_OLD_TURN);

    // Before turn/start was even written there is nothing to stop.
    let (mut s, _) = AppSession::start(params("x"));
    assert!(s.steer("held", "words").unwrap().is_empty());
    assert!(s.interrupt().is_empty());
    assert!(s.finished());
    assert_eq!(
        s.steer_events(),
        [SteerUpdate::Undelivered {
            client_id: "held".into()
        }]
    );
    let step = s.on_line(STEER.lines().next().unwrap());
    assert!(
        step.writes.is_empty() && step.finished,
        "no thread is started"
    );
}

#[test]
fn an_item_from_another_turn_is_dropped() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(command_started);
    let before = r.exec.len();
    // pB2's orphan: a command an interrupted turn left running, completing
    // under its old turn id.
    let orphan = RESUME
        .lines()
        .find(|l| {
            let v = parsed(l);
            is_item(&v, "item/completed", "commandExecution")
        })
        .unwrap();
    r.feed(orphan);
    let other = orphan.replace(RESUME_THREAD, STEER_THREAD);
    r.feed(&other);
    assert_eq!(r.exec.len(), before, "{:?}", &r.exec[before..]);
    // The same item under the active turn does come through.
    r.feed(&other.replace(RESUME_TURN, STEER_TURN));
    assert_eq!(r.exec.len(), before + 1);
}

#[test]
fn a_failed_turn_reads_as_turn_failed() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(|l| l["method"] == "turn/started");
    let error = json!({"message": "Selected model is at capacity.",
        "codexErrorInfo": "serverOverloaded", "additionalDetails": null});
    r.feed(
        &json!({"method": "error", "params": {"error": error, "willRetry": false,
            "threadId": STEER_THREAD, "turnId": STEER_TURN}})
        .to_string(),
    );
    r.feed(
        &json!({"method": "turn/completed", "params": {"threadId": STEER_THREAD,
            "turn": {"id": STEER_TURN, "items": [], "status": "failed", "error": error}}})
        .to_string(),
    );
    assert!(r.finished);
    let exec = r.exec_values();
    assert_eq!(
        &exec[exec.len() - 2..],
        [
            json!({"type": "error", "message": "Selected model is at capacity."}),
            json!({"type": "turn.failed", "error": {"message": "Selected model is at capacity.",
                "codex_error_info": "server_overloaded", "status": "failed"}}),
        ]
    );
    let mut state = ParseState::default();
    let events = parse_line(AgentKind::Codex, &r.exec[r.exec.len() - 1], &mut state);
    assert!(
        matches!(&events[0], AgentEvent::Result { is_error: true, text: Some(t), .. }
        if t == "Selected model is at capacity.")
    );
}

#[test]
fn a_failed_handshake_ends_the_turn_with_its_message() {
    let mut p = params("x");
    p.resume = Some("gone".into());
    let mut r = Replay::new(STEER, p);
    r.feed_until(response(1));
    r.feed(r#"{"id":2,"error":{"code":-32600,"message":"no rollout found for thread id gone"}}"#);
    assert!(r.finished);
    let last = r.exec_values().pop().unwrap();
    assert_eq!(last["type"], "turn.failed");
    assert!(last["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no rollout found for thread id gone"));
}

#[test]
fn handshake_failures_use_plain_words_and_keep_provider_detail() {
    let (mut session, _) = AppSession::start(params("x"));
    let mut step = Step::default();
    session.fail("turn/start", "The model is unavailable.", &mut step);
    let line = parsed(&step.exec_lines[0]);
    assert_eq!(line["error"]["message"], "Codex could not start the turn. The model is unavailable. Continue to try again, or open Agent updates.");
}

#[test]
fn other_items_take_exec_names_and_fields() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(|l| l["method"] == "turn/started");
    let item = |item: Value| {
        json!({"method": "item/completed", "params": {"threadId": STEER_THREAD,
            "turnId": STEER_TURN, "item": item}})
        .to_string()
    };
    let before = r.exec.len();
    r.feed(&item(
        json!({"type": "fileChange", "id": "f1", "status": "declined",
        "changes": [{"path": "a.md", "kind": {"type": "add"}, "diff": "+x"},
                    {"path": "b.md", "kind": {"type": "update", "move_path": null}, "diff": ""}]}),
    ));
    r.feed(&item(json!({"type": "mcpToolCall", "id": "m1", "server": "hollow_vault",
        "tool": "search", "status": "completed", "arguments": {"q": "x"},
        "result": {"content": [{"type": "text", "text": "hit"}], "structuredContent": null, "_meta": null},
        "error": null, "durationMs": 3})));
    r.feed(&item(
        json!({"type": "webSearch", "id": "w1", "query": "codex",
        "action": {"type": "search", "query": "codex"}, "results": null}),
    ));
    r.feed(&item(
        json!({"type": "reasoning", "id": "r1", "summary": ["a", "b"], "content": []}),
    ));
    r.feed(&item(
        json!({"type": "userMessage", "id": "u1", "clientId": null,
        "content": [{"type": "text", "text": STEER_PROMPT, "text_elements": []}]}),
    ));
    r.feed(&item(json!({"type": "contextCompaction", "id": "c1"})));
    r.feed(r#"{"method":"warning","params":{"threadId":null,"message":"careful"}}"#);
    let exec = r.exec_values();
    let items: Vec<&Value> = exec[before..].iter().map(|l| &l["item"]).collect();
    assert_eq!(
        items,
        [
            &json!({"id": "f1", "type": "file_change", "status": "failed",
                "changes": [{"path": "a.md", "kind": "add"}, {"path": "b.md", "kind": "update"}]}),
            &json!({"id": "m1", "type": "mcp_tool_call", "server": "hollow_vault", "tool": "search",
                "status": "completed", "arguments": {"q": "x"},
                "result": {"content": [{"type": "text", "text": "hit"}], "structured_content": null},
                "error": null}),
            &json!({"id": "w1", "type": "web_search", "query": "codex",
                "action": {"type": "search", "query": "codex"}}),
            &json!({"id": "r1", "type": "reasoning", "text": "a\nb"}),
            &json!({"id": "item_0", "type": "error", "message": "careful"}),
        ]
    );
    let mut state = ParseState::default();
    let events = parse_line(AgentKind::Codex, &r.exec[before + 1], &mut state);
    assert!(
        matches!(&events[0], AgentEvent::ToolUse { name, .. } if name == "mcp__hollow_vault__search")
    );
    assert!(
        matches!(&events[1], AgentEvent::ToolResult { content, is_error: false, .. } if content == "hit")
    );
}

#[test]
fn plan_updates_become_a_todo_list() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(|l| l["method"] == "turn/started");
    let before = r.exec.len();
    let plan = |statuses: [&str; 2]| {
        json!({"method": "turn/plan/updated", "params": {"threadId": STEER_THREAD,
            "turnId": STEER_TURN, "explanation": null, "plan": [
                {"step": "read", "status": statuses[0]},
                {"step": "write", "status": statuses[1]}]}})
        .to_string()
    };
    r.feed(&plan(["inProgress", "pending"]));
    r.feed(&plan(["completed", "inProgress"]));
    r.feed(
        &json!({"method": "turn/completed", "params": {"threadId": STEER_THREAD,
            "turn": {"id": STEER_TURN, "items": [], "status": "completed", "error": null}}})
        .to_string(),
    );
    let exec = r.exec_values();
    let types: Vec<&str> = exec[before..]
        .iter()
        .map(|l| l["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        [
            "item.started",
            "item.updated",
            "item.completed",
            "turn.completed"
        ]
    );
    let done = &exec[before + 2]["item"];
    assert_eq!(done["type"], "todo_list");
    assert_eq!(
        done["items"],
        json!([{"text": "read", "completed": true}, {"text": "write", "completed": false}])
    );
    assert_eq!(done["id"], exec[before]["item"]["id"]);
}

#[test]
fn request_ids_only_grow() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(command_started);
    r.steer("a", "one");
    let stop: Vec<Value> = r.session.interrupt().iter().map(|w| parsed(w)).collect();
    r.writes.extend(stop);
    // A stopping turn takes no more steers.
    assert_eq!(r.session.steer("b", "two"), Err(SteerRefusal::Ended));
    let ids: Vec<u64> = r
        .writes
        .iter()
        .filter(|w| w.get("method").is_some())
        .filter_map(|w| w["id"].as_u64())
        .collect();
    assert_eq!(ids, [1, 2, 3, 4, 5]);
    assert!(r.session.steer_events().is_empty());
}

// ── Review fixes (2026-10-21) ────────────────────────────────────────────

/// Review 1: an `error` the server will retry (`willRetry: true`) is not the
/// turn's end. It reaches exec's parser as nothing, so a turn that recovers
/// and completes reads as a success; one it won't retry still reads as an
/// error.
#[test]
fn a_retryable_error_is_not_terminal_and_the_turn_still_succeeds() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(|l| l["method"] == "turn/started");
    let before = r.exec.len();
    r.feed(
        &json!({"method": "error", "params": {"willRetry": true,
            "error": {"message": "Reconnecting... 1/5", "codexErrorInfo": null, "additionalDetails": null},
            "threadId": STEER_THREAD, "turnId": STEER_TURN}})
        .to_string(),
    );
    assert_eq!(r.exec.len(), before, "{:?}", &r.exec[before..]);
    r.feed_rest();
    let mut state = ParseState::default();
    let failed = r.exec.iter().any(|line| {
        parse_line(AgentKind::Codex, line, &mut state)
            .iter()
            .any(|e| matches!(e, AgentEvent::Result { is_error: true, .. }))
    });
    assert!(!failed, "{:?}", r.exec);
    assert_eq!(r.exec_values().last().unwrap()["type"], "turn.completed");
}

/// The fixture's `turn/start` answer and its `turn/started`, in that order.
fn turn_lines() -> (&'static str, &'static str) {
    let pick = |f: &dyn Fn(&Value) -> bool| STEER.lines().find(|l| f(&parsed(l))).unwrap();
    (
        pick(&|l| l.get("method").is_none() && l["id"] == 3),
        pick(&|l| l["method"] == "turn/started"),
    )
}

/// Review 2: a stop asked while a steer was held (no turn id yet) sends only
/// the interrupt, whichever of `turn/started` and the `turn/start` answer
/// comes first, and the held steer is reported undelivered at once.
#[test]
fn a_held_steer_is_never_sent_after_an_interrupt_in_either_order() {
    let (answer, started) = turn_lines();
    for order in [[answer, started], [started, answer]] {
        let mut r = Replay::new(STEER, params(STEER_PROMPT));
        r.feed_until(response(2));
        assert_eq!(r.session.steer("held", "words"), Ok(Vec::new()));
        assert!(r.session.interrupt().is_empty(), "no turn id yet");
        let before = r.writes.len();
        for line in order {
            r.feed(line);
        }
        let writes = r.writes[before..].to_vec();
        assert_eq!(methods(&writes), ["turn/interrupt"], "{order:?}");
        assert_eq!(
            r.session.steer_events(),
            [SteerUpdate::Undelivered {
                client_id: "held".into()
            }],
            "{order:?}"
        );
    }
}

/// Review 3: a turn Codex itself ended `interrupted` (Hollow asked no stop)
/// is not a success: it reads as exec's `turn.failed`, saying so. A stop
/// Hollow asked still ends with no terminal line
/// (`an_interrupt_is_sent_once_and_ends_with_no_terminal_line`).
#[test]
fn a_turn_codex_interrupted_on_its_own_reads_as_failed() {
    let mut r = Replay::new(INTERRUPT, params("x"));
    r.feed_until(|l| l["method"] == "turn/completed");
    assert!(r.finished);
    let last = r.exec_values().pop().unwrap();
    assert_eq!(last["type"], "turn.failed", "{last}");
    assert!(last["error"]["message"]
        .as_str()
        .unwrap()
        .contains("interrupted"));
    let mut state = ParseState::default();
    let events = parse_line(AgentKind::Codex, &r.exec[r.exec.len() - 1], &mut state);
    assert!(matches!(
        &events[0],
        AgentEvent::Result { is_error: true, .. }
    ));
}

/// Review 4: a completed turn with no usage notification reports no usage,
/// never a measured zero.
#[test]
fn a_turn_with_no_usage_reports_none() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.lines
        .retain(|l| parsed(l)["method"] != "thread/tokenUsage/updated");
    r.feed_rest();
    let last = r.exec_values().pop().unwrap();
    assert_eq!(last["type"], "turn.completed");
    assert!(last.get("usage").is_none(), "{last}");
    let usage = crate::usage::codex_turn_usage(&last);
    assert!(usage.tokens.is_none());
}

/// P1-31: a known delta survives every incomplete ending, without becoming
/// a success or a whole-thread context figure. All inputs are local fixtures.
#[test]
fn partial_usage_survives_stop_failure_and_early_exit() {
    for ending in ["stop", "failed", "early", "error"] {
        let mut r = Replay::new(STEER, params(STEER_PROMPT));
        r.feed_until(|l| l["method"] == "thread/tokenUsage/updated");
        match ending {
            "stop" => r.session.note_stopping(),
            "early" => {
                if let Some(line) = r.session.ended_early() {
                    r.exec.push(line);
                }
            }
            "error" => {
                r.feed(
                    &json!({"method": "error", "params": {
                        "threadId": STEER_THREAD, "turnId": STEER_TURN,
                        "willRetry": false, "error": {"message": "fixture failure"}
                    }})
                    .to_string(),
                );
                assert!(r.session.ended_early().is_none());
            }
            _ => {
                r.feed(
                    &json!({"method": "turn/completed", "params": {
                        "threadId": STEER_THREAD, "turn": {"id": STEER_TURN,
                        "status": "failed", "error": {"message": "fixture failure"}}
                    }})
                    .to_string(),
                );
            }
        }
        let mut state = ParseState::default();
        let mut acc = crate::usage::UsageAccumulator::default();
        let dir = tempfile::tempdir().unwrap();
        let meter = crate::usage_ledger::Meter::for_vault(
            dir.path(),
            crate::usage_ledger::Surface::Chat,
            None,
            None,
        );
        let mut ledger = crate::usage_ledger::UsageRun::begin(meter, Some(AgentKind::Codex), None);
        ledger.mark_spawned();
        for line in &r.exec {
            for event in parse_line(AgentKind::Codex, line, &mut state) {
                assert!(!matches!(
                    event,
                    AgentEvent::Result {
                        is_error: false,
                        ..
                    }
                ));
                acc.observe(&event);
                ledger.observe(&event);
            }
        }
        let usage = crate::usage::resolve(&acc, Some(AgentKind::Codex), false, None, true)
            .unwrap_or_else(|| panic!("{ending}: known tokens lost"));
        assert!(!usage.complete, "{ending}");
        assert!(usage.tokens.unwrap().total() > 0, "{ending}");
        assert_eq!(acc.context_tokens(), None, "a turn total is not context");
        assert_eq!(acc.call_stats().api_calls, None, "not a model call");
        let outcome = if ending == "stop" {
            crate::usage::LedgerOutcome::Cancelled
        } else {
            crate::usage::LedgerOutcome::Error
        };
        let row = ledger.finish(outcome);
        assert_eq!(
            row.cost_status,
            crate::usage::CostStatus::Partial,
            "{ending}"
        );
        assert_eq!(row.tokens, usage.tokens, "{ending}");
    }
}

#[test]
fn every_provider_capability_reports_context() {
    for kind in AgentKind::all() {
        let value = serde_json::to_value(crate::agent::capabilities(kind)).unwrap();
        // Antigravity too since agy 1.5.13 (usage on every step).
        assert_eq!(value["reportsContext"], true, "{kind:?}");
        // AC1: the adapter flags and reasons are on the descriptor's wire.
        for key in [
            "steer",
            "interrupt",
            "fork",
            "rollback",
            "queue",
            "pendingSurvivesRestart",
        ] {
            assert!(value[key].is_boolean(), "{kind:?} {key}");
        }
        assert!(value["unsupportedReasons"].is_object());
    }
}

#[test]
fn malformed_usage_cannot_establish_a_resumed_threads_baseline() {
    let mut p = params("x");
    p.resume = Some(RESUME_THREAD.into());
    let mut r = Replay::new(RESUME, p);
    r.feed_until(command_started);
    let step = r.feed(
        &json!({"method": "thread/tokenUsage/updated", "params": {
            "threadId": RESUME_THREAD, "turnId": RESUME_TURN,
            "tokenUsage": {"total": {}, "last": {}}
        }})
        .to_string(),
    );
    assert!(
        step.exec_lines.is_empty(),
        "malformed snapshots remain unknown"
    );
    r.feed_until(|l| l["method"] == "thread/tokenUsage/updated");
    let last = r.exec_values().pop().unwrap();
    assert_eq!(last["type"], "turn.usage");
    assert_eq!(last["usage"]["input_tokens"], 28673);
    assert_eq!(last["usage"]["cached_input_tokens"], 28160);
    assert_eq!(last["usage"]["output_tokens"], 57);
    let mut acc = crate::usage::UsageAccumulator::default();
    let mut state = ParseState::default();
    // Repeating a snapshot replaces, rather than adding to, the delta.
    for _ in 0..2 {
        for event in parse_line(AgentKind::Codex, &last.to_string(), &mut state) {
            acc.observe(&event);
        }
    }
    let partial = crate::usage::resolve(&acc, Some(AgentKind::Codex), true, None, true).unwrap();
    assert!(!partial.complete);
    assert_eq!(partial.tokens.unwrap().total(), 28730);
}

#[test]
fn rate_limit_notifications_are_forwarded_without_starting_a_request() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(response(2));
    let step = r.feed(
        &json!({"method": "account/rateLimits/updated", "params": {
            "rateLimits": {"primary": {"usedPercent": 23.5, "windowDurationMins": 300,
                "resetsAt": 1800000000}, "secondary": null, "planType": "test"}
        }})
        .to_string(),
    );
    assert!(step.writes.is_empty());
    let line = step.exec_lines.first().expect("rate limits retained");
    assert_eq!(parsed(line)["type"], "account.rate_limits");
    assert_eq!(parsed(line)["limits"]["primary"]["usedPercent"], 23.5);
}

/// Review 5: steers reach the server in the order they were sent. Steer A
/// is held (no turn id yet); when the id arrives nothing is written from the
/// stdout side, and the next steer B (already queued for the writer) writes
/// A first, then B.
#[test]
fn a_held_steer_goes_before_a_later_one() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(response(2));
    assert_eq!(r.session.steer("a", "First."), Ok(Vec::new()));
    let step = r.feed_until(response(3));
    assert!(
        !methods(&step.writes.iter().map(|w| parsed(w)).collect::<Vec<_>>())
            .contains(&"turn/steer"),
        "the reader never writes a steer"
    );
    let writes = r.steer("b", "Second.");
    assert_eq!(methods(&writes), ["turn/steer", "turn/steer"]);
    assert_eq!(writes[0]["params"]["clientUserMessageId"], "a");
    assert_eq!(writes[1]["params"]["clientUserMessageId"], "b");
    assert!(writes[0]["id"].as_u64() < writes[1]["id"].as_u64());
}

/// Round-2 review 1: once Hollow says it is stopping, an `interrupted`
/// completion is that stop (no terminal line, nothing written), and held
/// steers are undelivered.
#[test]
fn a_noted_stop_makes_an_interrupted_completion_the_stop() {
    let mut r = Replay::new(INTERRUPT, params("x"));
    r.feed_until(command_started);
    let before = r.writes.len();
    r.session.note_stopping();
    r.feed_until(|l| l["method"] == "turn/completed");
    assert!(r.finished);
    assert_eq!(r.writes.len(), before, "no interrupt is written");
    assert!(r
        .exec_values()
        .iter()
        .all(|l| l["type"] != "turn.failed" && l["type"] != "turn.completed"));
}

/// Round-2 review 2: output that ends before the turn did is a failed turn
/// carrying the last retried error; once the turn ended there is nothing.
#[test]
fn output_that_ends_before_the_turn_is_a_failed_turn() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(|l| l["method"] == "turn/started");
    r.feed(
        &json!({"method": "error", "params": {"willRetry": true,
            "error": {"message": "Reconnecting... 2/5", "codexErrorInfo": null, "additionalDetails": null},
            "threadId": STEER_THREAD, "turnId": STEER_TURN}})
        .to_string(),
    );
    let line = parsed(&r.session.ended_early().expect("unfinished"));
    assert_eq!(line["type"], "turn.failed");
    let message = line["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("before the turn finished") && message.contains("Reconnecting... 2/5")
    );
    assert!(r.session.finished());
    assert!(r.session.ended_early().is_none());

    let mut done = Replay::new(STEER, params(STEER_PROMPT));
    done.feed_rest();
    assert!(done.session.ended_early().is_none());
}

/// Round-3 review: an error the server won't retry is already the turn's
/// failure, so output that then ends adds no second, generic one.
#[test]
fn an_early_end_after_a_reported_error_adds_nothing() {
    let mut r = Replay::new(STEER, params(STEER_PROMPT));
    r.feed_until(|l| l["method"] == "turn/started");
    r.feed(
        &json!({"method": "error", "params": {"willRetry": false,
            "error": {"message": "Selected model is at capacity.", "codexErrorInfo": null, "additionalDetails": null},
            "threadId": STEER_THREAD, "turnId": STEER_TURN}})
        .to_string(),
    );
    assert_eq!(r.exec_values().last().unwrap()["type"], "error");
    assert!(!r.session.finished());
    assert!(r.session.ended_early().is_none());
    assert!(r.session.finished());
}

/// Live probe (steering PR 3), never in the suite: the installed
/// `codex app-server` driven by [`AppSession`] exactly as a chat turn's
/// attempt drives it, with a steer written while `sleep 6` runs. One model
/// turn on `gpt-6-luna` at low effort, in the folder `HOLLOW_CODEX_LIVE_DIR`
/// names. Checks the translated lines parse, the steer is delivered, and
/// the server exits once stdin closes.
#[test]
#[ignore = "live: runs the installed codex app-server (one model turn)"]
fn live_codex_app_server_takes_a_steer_and_exits_on_eof() {
    use std::io::{BufRead, BufReader, Write};
    let dir = std::env::var("HOLLOW_CODEX_LIVE_DIR").expect("HOLLOW_CODEX_LIVE_DIR");
    let mut p = params("Run the shell command `sleep 6; echo done` and then reply with its output in one short sentence.");
    p.cwd = dir.clone();
    p.config = Map::new();
    // `agent::build_codex_app_server`'s argv, plus no notify program.
    let mut child = std::process::Command::new("codex")
        .args([
            "--disable",
            "apps",
            "--disable",
            "plugins",
            "--disable",
            "remote_plugin",
        ])
        .args([
            "-c",
            "model_reasoning_effort=\"low\"",
            "-c",
            "notify=[]",
            "app-server",
        ])
        .current_dir(&dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("codex app-server");
    let mut stdin = Some(child.stdin.take().unwrap());
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let write = |stdin: &mut Option<std::process::ChildStdin>, lines: Vec<String>| {
        for line in lines {
            eprintln!(">> {line}");
            let pipe = stdin.as_mut().expect("stdin open");
            writeln!(pipe, "{line}").unwrap();
            pipe.flush().unwrap();
        }
    };
    let (mut session, first) = AppSession::start(p);
    write(&mut stdin, first);
    let mut state = ParseState::default();
    let (mut steered, mut text, mut updates, mut tools) = (false, String::new(), Vec::new(), 0);
    let started = std::time::Instant::now();
    for line in stdout.lines() {
        let line = line.unwrap();
        let step = session.on_line(&line);
        write(&mut stdin, step.writes);
        for exec in &step.exec_lines {
            eprintln!("exec {exec}");
            for evt in parse_line(AgentKind::Codex, exec, &mut state) {
                match evt {
                    AgentEvent::AssistantText(t) => text.push_str(&t),
                    AgentEvent::ToolUse { .. } => tools += 1,
                    _ => {}
                }
            }
            if !steered && exec.contains("\"command_execution\"") && exec.contains("item.started") {
                steered = true;
                let lines = session
                    .steer("live-1", "Also end your final reply with the word KIWI.")
                    .expect("steer accepted");
                write(&mut stdin, lines);
            }
        }
        updates.extend(session.steer_events());
        if step.finished && stdin.is_some() {
            eprintln!(
                "-- turn finished after {:?}; closing stdin",
                started.elapsed()
            );
            stdin = None;
        }
    }
    let closed = std::time::Instant::now();
    let status = child.wait().unwrap();
    eprintln!(
        "-- exited {status} {:?} after EOF; answer: {text}",
        closed.elapsed()
    );
    assert!(status.success());
    assert!(steered && tools >= 1);
    assert_eq!(
        updates,
        vec![SteerUpdate::Delivered {
            client_id: "live-1".into()
        }]
    );
    assert!(text.contains("KIWI"), "{text}");
}
