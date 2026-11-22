//! OC1: the ACP session against frames shaped like the 2026-11-17 probe of
//! `grok agent stdio` (grok 1.0.88), a scripted stand-in agent over real
//! pipes, the per-mode transport table and the headless fallback.

use super::*;
use crate::agent::{parse_line, AgentEvent, AgentKind, ParseState, RunExtras, RunSpec};
use serde_json::json;
use std::path::PathBuf;

const SID: &str = "01a1ee84-e280-75a0-bf68-34429c929886";

fn params(resume: Option<&str>, model: Option<&str>) -> StartParams {
    StartParams {
        cwd: "/tmp/acp".into(),
        prompt: "Reply with the single word ok.".into(),
        resume: resume.map(str::to_string),
        model: model.map(str::to_string),
        effort: None,
        session_meta: json!({"yoloMode": true}),
        perm: "bypassPermissions".into(),
        agent: AgentKind::Grok,
        mode: None,
        ask: false,
        mcp_servers: Vec::new(),
    }
}

fn parsed(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

fn response(id: u64, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn update(update: Value) -> String {
    json!({"jsonrpc": "2.0", "method": "session/update",
        "params": {"sessionId": SID, "update": update}})
    .to_string()
}

fn init_result() -> Value {
    json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true},
        "authMethods": [{"id": "cached_token"}], "_meta": {"x.ai/hooks": {}}})
}

/// A session past the handshake, prompt written; its lines so far.
fn prompted(model: Option<&str>) -> (AcpSession, Vec<String>) {
    let (mut session, first) = AcpSession::start(params(None, model));
    let init = parsed(&first[0]);
    assert_eq!(init["method"], "initialize");
    assert_eq!(init["params"]["protocolVersion"], 1);
    assert_eq!(
        init["params"]["clientCapabilities"],
        json!({"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false})
    );
    let step = session.on_line(&response(1, init_result()));
    let new = parsed(&step.writes[0]);
    assert_eq!(new["method"], "session/new");
    assert_eq!(new["params"]["cwd"], "/tmp/acp");
    assert_eq!(new["params"]["mcpServers"], json!([]));
    assert_eq!(new["params"]["_meta"], json!({"yoloMode": true}));
    // The setup burst before session/new answers is ignored.
    let setup = json!({"jsonrpc": "2.0", "method": "_x.ai/session/setup",
        "params": {"phase": "folder_trust"}})
    .to_string();
    assert_eq!(session.on_line(&setup), Step::default());
    let mut lines = Vec::new();
    let step = session.on_line(&response(2, json!({"sessionId": SID, "_meta": {}})));
    lines.extend(step.lines);
    let mut next = 3;
    let mut write = parsed(&step.writes[0]);
    if let Some(model) = model {
        assert_eq!(write["method"], "session/set_config_option");
        assert_eq!(
            write["params"],
            json!({"sessionId": SID, "configId": "model", "value": model})
        );
        let step = session.on_line(&response(next, json!({"configOptions": []})));
        next += 1;
        write = parsed(&step.writes[0]);
    }
    assert_eq!(write["method"], "session/prompt");
    assert_eq!(write["id"], next);
    assert_eq!(
        write["params"],
        json!({"sessionId": SID, "prompt": [{"type": "text", "text": "Reply with the single word ok."}]})
    );
    (session, lines)
}

fn events(lines: &[String]) -> Vec<AgentEvent> {
    let mut state = ParseState::default();
    lines
        .iter()
        .flat_map(|l| parse_line(AgentKind::Grok, l, &mut state))
        .collect()
}

fn feed(session: &mut AcpSession, frames: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut lines, mut writes) = (Vec::new(), Vec::new());
    for frame in frames {
        let step = session.on_line(frame);
        lines.extend(step.lines);
        writes.extend(step.writes);
    }
    (lines, writes)
}

fn probe_turn() -> Vec<String> {
    vec![
        update(json!({"sessionUpdate": "available_commands_update", "availableCommands": []})),
        update(json!({"sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": "Run it."}})),
        update(json!({"sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": " Then answer."}})),
        update(json!({"sessionUpdate": "tool_call", "toolCallId": "call-1",
            "title": "run_terminal_command", "kind": "execute", "status": "pending",
            "rawInput": {"command": "echo hi > out.txt"}})),
        json!({"jsonrpc": "2.0", "method": "_x.ai/session/pending_interaction",
            "params": {"kind": "permission"}})
        .to_string(),
        update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "call-1",
            "status": "in_progress",
            "content": [{"type": "content", "content": {"type": "text", "text": "partial"}}]}),
        ),
        update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "call-1",
            "status": "completed",
            "content": [{"type": "content", "content": {"type": "text", "text": "done"}}]}),
        ),
        update(json!({"sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "o"}})),
        update(json!({"sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "k"}})),
        response(
            3,
            json!({"stopReason": "end_turn", "_meta": {"usage": {
                "inputTokens": 25000, "outputTokens": 40, "cachedReadTokens": 1000,
                "cacheCreationTokens": 0, "reasoningTokens": 12, "totalTokens": 25040,
                "costUsdTicks": 158_861_600_u64}}}),
        ),
    ]
}

#[test]
fn a_probe_shaped_turn_reads_as_the_headless_stream() {
    let (mut session, mut lines) = prompted(None);
    let (more, writes) = feed(&mut session, &probe_turn());
    lines.extend(more);
    assert!(writes.is_empty());
    assert!(session.finished());
    let got = events(&lines);
    assert!(matches!(&got[0], AgentEvent::Session(s) if s == SID));
    assert!(got.iter().any(|e| matches!(e, AgentEvent::TurnBoundary)));
    assert!(got.iter().any(|e| matches!(e, AgentEvent::Thinking)));
    let tool = got
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolUse { id, name, input } => Some((id, name, input)),
            _ => None,
        })
        .unwrap();
    assert_eq!(tool.0, "call-1");
    assert_eq!(tool.1, "run_terminal_command");
    assert_eq!(tool.2, &json!({"command": "echo hi > out.txt"}));
    let result = got
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolResult {
                id,
                content,
                is_error,
            } => Some((id, content, *is_error)),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (result.0.as_str(), result.1.as_str(), result.2),
        ("call-1", "done", false)
    );
    let deltas: String = got
        .iter()
        .filter_map(|e| match e {
            AgentEvent::AssistantDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, "ok");
    // The thinking never reaches the text.
    assert!(!lines.iter().any(|l| l.contains("Run it.")));
    let AgentEvent::Result {
        text,
        usage,
        is_error,
        subtype,
        detail,
    } = got.last().unwrap()
    else {
        panic!("the turn ends in a result: {got:?}");
    };
    assert_eq!(text.as_deref(), Some("ok"));
    assert!(!is_error);
    assert_eq!(subtype, "success");
    assert_eq!(detail.stop_reason.as_deref(), Some("end_turn"));
    // ACP's full-prompt inputTokens become headless's uncached input.
    let headless = json!({"type": "result", "subtype": "success", "result": "ok",
        "usage": {"input_tokens": 24000, "output_tokens": 40,
            "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0}});
    let want = crate::usage::claude_result_usage(&headless);
    assert_eq!(usage.tokens, want.tokens);
    assert!(usage.tokens.is_some());
    // Ticks are 1e-10 dollars (Grok's docs); only a complete cost is kept.
    assert!((usage.cost_usd.unwrap() - 0.015_886_16).abs() < 1e-12);
    let partial = json!({"_meta": {"usage": {"costUsdTicks": 5, "costIsPartial": true}}});
    assert_eq!(prompt_cost(&partial), None);
}

#[test]
fn a_model_goes_as_a_config_option_and_names_the_init_line() {
    let (_, lines) = prompted(Some("grok-4.6"));
    assert_eq!(parsed(&lines[0])["model"], "grok-4.6");
}

#[test]
fn no_usage_is_unknown_never_zero() {
    let (mut session, _) = prompted(None);
    let step = session.on_line(&response(3, json!({"stopReason": "end_turn"})));
    let result = parsed(step.lines.last().unwrap());
    assert!(result.get("usage").is_none());
    assert!(step.finished);
}

#[test]
fn a_permission_request_is_answered_with_its_reject_option() {
    let (mut session, _) = prompted(None);
    session.on_line(&update(
        json!({"sessionUpdate": "tool_call", "toolCallId": "call-9",
        "title": "run_terminal_command", "status": "pending", "rawInput": {"command": "rm x"}}),
    ));
    let ask = json!({"jsonrpc": "2.0", "id": 77, "method": "session/request_permission",
        "params": {"sessionId": SID, "toolCall": {"toolCallId": "call-9"}, "options": [
            {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
            {"optionId": "always", "name": "Always", "kind": "allow_always"},
            {"optionId": "no", "name": "Reject", "kind": "reject_once"}]}})
    .to_string();
    let step = session.on_line(&ask);
    assert_eq!(
        parsed(&step.writes[0]),
        json!({"jsonrpc": "2.0", "id": 77, "result": {"outcome": {"outcome": "selected", "optionId": "no"}}})
    );
    // Only reject_always: that one. No reject option: cancelled, never allow.
    let only_always = ask.replace("reject_once", "reject_always");
    let step = session.on_line(&only_always);
    assert_eq!(
        parsed(&step.writes[0])["result"]["outcome"]["optionId"],
        "no"
    );
    let none = json!({"jsonrpc": "2.0", "id": 78, "method": "session/request_permission",
        "params": {"options": [{"optionId": "allow", "kind": "allow_once"}]}})
    .to_string();
    let step = session.on_line(&none);
    assert_eq!(
        parsed(&step.writes[0])["result"],
        json!({"outcome": {"outcome": "cancelled"}})
    );
    // The declined tool reads as a denial, closed by Grok or not.
    let step = session.on_line(&response(3, json!({"stopReason": "end_turn"})));
    let got = events(&step.lines);
    assert!(got.iter().any(|e| matches!(e,
        AgentEvent::ToolResult { id, is_error: true, content } if id == "call-9" && content.contains("declined"))));
}

#[test]
fn fs_terminal_and_extension_requests_get_method_not_found() {
    let (mut session, _) = prompted(None);
    for (id, method) in [
        (40, "fs/read_text_file"),
        (41, "fs/write_text_file"),
        (42, "terminal/create"),
        (43, "_x.ai/fs/list"),
    ] {
        let ask = json!({"jsonrpc": "2.0", "id": id, "method": method,
            "params": {"sessionId": SID, "path": "/etc/hosts"}})
        .to_string();
        let step = session.on_line(&ask);
        let answer = parsed(&step.writes[0]);
        assert_eq!(answer["id"], id);
        assert_eq!(answer["error"]["code"], -32601, "{method}");
        assert!(answer.get("result").is_none());
        assert!(step.lines.is_empty());
    }
    assert!(!session.finished());
}

#[test]
fn extension_notifications_malformed_and_dropped_lines_are_ignored() {
    let (mut session, _) = prompted(None);
    for line in [
        json!({"jsonrpc": "2.0", "method": "_x.ai/queue/changed", "params": {"queue": []}})
            .to_string(),
        json!({"jsonrpc": "2.0", "method": "_x.ai/settings/update", "params": {}}).to_string(),
        "not json at all".to_string(),
        "{\"jsonrpc\": \"2.0\", \"method\": \"session/upd".to_string(),
        "[1, 2, 3]".to_string(),
        crate::process_group::dropped_line_event(20_000_000, crate::process_group::STDOUT_LINE_MAX),
        // Another session's update, and an answer to nothing Hollow asked.
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "other",
            "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "x"}}}})
        .to_string(),
        response(999, json!({})),
    ] {
        assert_eq!(session.on_line(&line), Step::default(), "{line}");
    }
    assert!(!session.finished());
}

#[test]
fn an_agent_that_exits_mid_turn_is_a_failed_turn() {
    let (mut session, _) = prompted(None);
    session.on_line(&update(json!({"sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": "half"}})));
    let line = session.ended_early().expect("a failure");
    let got = events(&[line]);
    assert!(matches!(
        got.last(),
        Some(AgentEvent::Result { is_error: true, .. })
    ));
    assert!(session.ended_early().is_none(), "once");
    // After a stop Hollow asked, the end is that stop.
    let (mut stopped, _) = prompted(None);
    stopped.note_stopping();
    assert!(stopped.ended_early().is_none());
    let step = stopped.on_line(&response(3, json!({"stopReason": "cancelled"})));
    assert!(step.lines.iter().all(|l| !l.contains("\"result\"")));
    assert!(step.finished);
}

#[test]
fn stop_reasons_map_to_headless_subtypes() {
    for (stop, subtype, is_error) in [
        ("end_turn", "success", false),
        ("max_tokens", "success", false),
        ("refusal", "success", false),
        ("max_turn_requests", "error_max_turns", true),
        ("cancelled", "error_during_execution", true),
    ] {
        let (mut session, _) = prompted(None);
        let step = session.on_line(&response(3, json!({"stopReason": stop})));
        let result = parsed(step.lines.last().unwrap());
        assert_eq!(result["subtype"], subtype, "{stop}");
        assert_eq!(result["is_error"], is_error, "{stop}");
        assert_eq!(result["stop_reason"], stop);
    }
}

#[test]
fn refusals_end_the_turn_with_the_agents_words() {
    // Another protocol version.
    let (mut session, _) = AcpSession::start(params(None, None));
    let step = session.on_line(&response(1, json!({"protocolVersion": 2})));
    assert!(step.finished && step.writes.is_empty());
    assert!(parsed(&step.lines[0])["result"]
        .as_str()
        .unwrap()
        .contains("protocol version 2"));
    // A refused prompt (a sign-in failure reads as one).
    let (mut session, _) = prompted(None);
    let error = json!({"jsonrpc": "2.0", "id": 3,
        "error": {"code": -32000, "message": "Authentication required", "data": "Not signed in"}})
    .to_string();
    let step = session.on_line(&error);
    let result = parsed(step.lines.last().unwrap());
    assert_eq!(result["is_error"], true);
    assert_eq!(result["result"], "Authentication required (Not signed in)");
}

#[test]
fn a_resumed_turn_loads_its_session_and_drops_the_replay() {
    let (mut session, _) = AcpSession::start(params(Some(SID), None));
    let step = session.on_line(&response(1, init_result()));
    let load = parsed(&step.writes[0]);
    assert_eq!(load["method"], "session/load");
    assert_eq!(load["params"]["sessionId"], SID);
    assert_eq!(load["params"]["_meta"], json!({"yoloMode": true}));
    // The history replay before the load answers is not this turn.
    let replay = update(json!({"sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": "old answer"}}));
    assert!(session.on_line(&replay).lines.is_empty());
    let step = session.on_line(&response(2, json!(null)));
    assert!(matches!(&events(&step.lines)[0], AgentEvent::Session(s) if s == SID));
    assert_eq!(parsed(&step.writes[0])["method"], "session/prompt");
    // A refused load is a failed resume, so the chat retries fresh.
    let (mut session, _) = AcpSession::start(params(Some(SID), None));
    session.on_line(&response(1, init_result()));
    let error = json!({"jsonrpc": "2.0", "id": 2,
        "error": {"code": -32002, "message": "Resource not found"}})
    .to_string();
    let step = session.on_line(&error);
    let result = parsed(&step.lines[0]);
    let text = result["result"].as_str().unwrap();
    assert!(text.starts_with(LOAD_FAILED), "{text}");
    assert_eq!(
        crate::result_kind::envelope_class(AgentKind::Grok, &[text], None),
        crate::result_kind::ResultKind::ResumeFailed
    );
}

#[test]
fn only_a_missing_session_makes_a_refused_load_a_fresh_retry() {
    let refused = |error: Value| {
        let (mut session, _) = AcpSession::start(params(Some(SID), None));
        session.on_line(&response(1, init_result()));
        let line = json!({"jsonrpc": "2.0", "id": 2, "error": error}).to_string();
        let step = session.on_line(&line);
        assert!(step.finished);
        let result = parsed(&step.lines[0]);
        let text = result["result"].as_str().unwrap().to_string();
        crate::result_kind::envelope_class(AgentKind::Grok, &[&text], None)
    };
    use crate::result_kind::ResultKind;
    for missing in [
        json!({"code": -32603, "message": "Session \"x\" not found locally"}),
        json!({"code": -32603, "message": "Unknown session id"}),
        json!({"code": -32603, "message": "cannot resume this session under sandbox profile off"}),
    ] {
        assert_eq!(
            refused(missing.clone()),
            ResultKind::ResumeFailed,
            "{missing}"
        );
    }
    // Sign-in, network and limits fail the turn and keep the history.
    for other in [
        json!({"code": -32000, "message": "Authentication required"}),
        json!({"code": -32603, "message": "connection reset by peer"}),
        json!({"code": -32603, "message": "Rate limit exceeded, retry later"}),
    ] {
        assert_ne!(refused(other.clone()), ResultKind::ResumeFailed, "{other}");
    }
}

#[test]
fn an_error_with_no_request_id_ends_a_waiting_turn() {
    let (mut session, _) = prompted(None);
    let step = session.on_line(
        &json!({"jsonrpc": "2.0", "id": null,
            "error": {"code": -32700, "message": "Parse error"}})
        .to_string(),
    );
    assert!(step.finished);
    assert_eq!(parsed(&step.lines[0])["is_error"], true);
    assert!(parsed(&step.lines[0])["result"]
        .as_str()
        .unwrap()
        .contains("Parse error"));
}

// ── Transport choice and argv ──────────────────────────────────────────────

fn run(perm: &str, acp: bool) -> RunSpec {
    RunSpec {
        prompt: "hello".into(),
        resume: None,
        model: None,
        perm: perm.into(),
        cwd: PathBuf::from("/tmp"),
        allowed_tools: vec![],
        disallowed_tools: vec![],
        interactive: true,
        mcp_config: None,
        append_system_prompt: None,
        process_scope: crate::agent::ProcessScope::Inherited,
        usage_stream: false,
        max_budget_usd: None,
        extras: RunExtras {
            grok_acp: acp,
            ..RunExtras::default()
        },
    }
}

fn argv(spec: &RunSpec) -> Vec<String> {
    crate::agent::build_command(AgentKind::Grok, spec)
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn only_bypass_maps_exactly_and_every_other_mode_falls_back() {
    assert_eq!(
        grok_mode("bypassPermissions"),
        Ok(GrokAcpMode {
            sandbox: "off",
            always_approve: true
        })
    );
    for perm in ["plan", "default", "acceptEdits", "auto", "", "weird"] {
        assert!(grok_mode(perm).is_err(), "{perm}");
        assert!(!crate::agent::grok_acp(&run(perm, true)), "{perm}");
        // The switch on changes nothing about a mode that falls back.
        assert_eq!(argv(&run(perm, true)), argv(&run(perm, false)), "{perm}");
    }
    assert!(crate::agent::grok_acp(&run("bypassPermissions", true)));
    // Matched untrimmed, as headless matches it (a padded mode runs as
    // acceptEdits with the workspace sandbox there), so it stays headless.
    for padded in [
        "bypassPermissions ",
        " bypassPermissions",
        "bypassPermissions\n",
    ] {
        assert!(grok_mode(padded).is_err(), "{padded:?}");
        assert!(!crate::agent::grok_acp(&run(padded, true)), "{padded:?}");
        assert_eq!(argv(&run(padded, true)), argv(&run(padded, false)));
    }
    // Antigravity's dialect is probed (2026-11-20) but never in the switch.
    assert!(selectable(AgentKind::Grok));
    assert!(!selectable(AgentKind::Antigravity));
    assert_eq!(dialect(AgentKind::Antigravity).unwrap().server, &["acp"]);
    assert!(dialect(AgentKind::Antigravity).unwrap().probed);
}

#[test]
fn the_switch_off_keeps_the_headless_argv() {
    let off = argv(&run("bypassPermissions", false));
    assert!(off.contains(&"--prompt-file".to_string()));
    assert!(off.contains(&"streaming-messages-json".to_string()));
    assert!(!off.contains(&"stdio".to_string()));
}

#[test]
fn a_bypass_turn_runs_the_stdio_server_with_the_headless_sandbox() {
    let mut spec = run("bypassPermissions", true);
    spec.model = Some("grok-4.6".into());
    let args = argv(&spec);
    assert_eq!(
        args,
        [
            "--sandbox",
            "off",
            "agent",
            "--always-approve",
            "--no-leader",
            "stdio"
        ]
    );
    // No prompt, model or session in argv: they go over JSON-RPC.
    assert!(crate::agent::stdin_prompt(AgentKind::Grok, &spec).is_none());
    let start = crate::agent::grok_acp_start(&spec);
    assert_eq!(start.model.as_deref(), Some("grok-4.6"));
    assert_eq!(start.session_meta, json!({"yoloMode": true}));
    assert!(start.prompt.contains("hello"));
}

#[test]
fn a_deny_rule_a_one_shot_or_a_runner_falls_back() {
    let mut denied = run("bypassPermissions", true);
    denied.disallowed_tools = vec!["Bash(osascript:*)".into()];
    assert!(!crate::agent::grok_acp(&denied));
    assert!(argv(&denied).contains(&"--deny".to_string()));
    // A rule Grok holds no flag for doesn't decide the transport.
    let mut inert = run("bypassPermissions", true);
    inert.disallowed_tools = vec!["AskUserQuestion".into()];
    assert!(crate::agent::grok_acp(&inert));
    let mut oneshot = run("bypassPermissions", true);
    oneshot.interactive = false;
    assert!(!crate::agent::grok_acp(&oneshot));
}

#[test]
fn the_setting_keeps_only_an_on_grok_switch() {
    let settings = crate::agent_settings::AgentSettings {
        acp: std::collections::BTreeMap::from([
            ("grok".to_string(), true),
            ("antigravity".to_string(), true),
        ]),
        ..Default::default()
    };
    let saved = crate::agent_settings::normalize(settings).unwrap();
    assert_eq!(
        saved.acp,
        std::collections::BTreeMap::from([("grok".to_string(), true)])
    );
    assert!(crate::agent_settings::acp_enabled_with(
        &saved,
        AgentKind::Grok
    ));
    assert!(!crate::agent_settings::acp_enabled_with(
        &saved,
        AgentKind::Antigravity
    ));
    let mut off = saved.clone();
    off.acp.insert("grok".into(), false);
    let off = crate::agent_settings::normalize(off).unwrap();
    assert!(off.acp.is_empty());
    assert!(!crate::agent_settings::acp_enabled_with(
        &crate::agent_settings::AgentSettings::default(),
        AgentKind::Grok
    ));
}

// ── A scripted stand-in agent over real pipes ──────────────────────────────

/// Drive `session` against `script` (a `/bin/sh` stand-in that answers each
/// request line in order), the way a chat turn does: writes to its stdin,
/// each stdout line through the session, stdin closed at the turn's end.
async fn drive(script: &str, mut session: AcpSession, first: Vec<String>) -> Vec<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut child = tokio::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    for line in first {
        let w = stdin.as_mut().unwrap();
        w.write_all(format!("{line}\n").as_bytes()).await.unwrap();
    }
    let mut lines = Vec::new();
    while let Ok(Some(raw)) = out.next_line().await {
        let step = session.on_line(&raw);
        if let Some(w) = stdin.as_mut() {
            for write in step.writes {
                let _ = w.write_all(format!("{write}\n").as_bytes()).await;
            }
        }
        lines.extend(step.lines);
        if step.finished {
            stdin = None;
        }
    }
    let _ = child.wait().await;
    if let Some(line) = session.ended_early() {
        lines.push(line);
    }
    lines
}

/// A stand-in that reads one request per answer and prints the probe's
/// frames; `exit_mid_turn` quits after the first chunk without answering.
fn stand_in(exit_mid_turn: bool) -> String {
    let say = |frame: String| format!("printf '%s\\n' '{frame}'\n");
    let mut script = String::new();
    script += "read -r _\n";
    script += &say(response(1, init_result()));
    script += "read -r _\n";
    script += &say(response(2, json!({"sessionId": SID})));
    script += "read -r _\n";
    script += &say(update(json!({"sessionUpdate": "agent_message_chunk",
        "content": {"type": "text", "text": "ok"}})));
    if exit_mid_turn {
        script += "exit 0\n";
        return script;
    }
    script += &say(response(3, json!({"stopReason": "end_turn"})));
    // The turn's end closes stdin: the stand-in exits on EOF, as grok does.
    script += "while read -r _; do :; done\n";
    script
}

#[tokio::test]
async fn a_stand_in_agent_runs_a_turn_over_real_pipes() {
    let (session, first) = AcpSession::start(params(None, None));
    let lines = drive(&stand_in(false), session, first).await;
    let got = events(&lines);
    assert!(matches!(&got[0], AgentEvent::Session(s) if s == SID));
    assert!(matches!(got.last(),
        Some(AgentEvent::Result { is_error: false, text: Some(t), .. }) if t == "ok"));
}

#[tokio::test]
async fn a_stand_in_agent_that_exits_mid_turn_fails_the_turn() {
    let (session, first) = AcpSession::start(params(None, None));
    let lines = drive(&stand_in(true), session, first).await;
    let got = events(&lines);
    let Some(AgentEvent::Result { is_error, text, .. }) = got.last() else {
        panic!("{got:?}");
    };
    assert!(*is_error);
    assert!(text.as_deref().unwrap().contains("ended during its turn"));
}

// ── Live smoke (one short turn on the person's cached Grok login) ──────────

/// `HOLLOW_GROK_ACP_SMOKE=1 cargo test --lib acp_client::tests::live -- --ignored --nocapture`:
/// one bypass turn in an empty temp folder, "Reply with the single word ok.",
/// printing every raw frame and mapped line. Skipped when `grok` isn't found.
#[tokio::test]
#[ignore = "live: one real Grok turn on the person's login, run by hand with HOLLOW_GROK_ACP_SMOKE=1 and --ignored"]
async fn live_grok_smoke() {
    if std::env::var_os("HOLLOW_GROK_ACP_SMOKE").is_none() {
        return;
    }
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let dir = tempfile::tempdir().unwrap();
    let mut spec = run("bypassPermissions", true);
    spec.cwd = dir.path().to_path_buf();
    spec.prompt = "Reply with the single word ok. Use no tools.".into();
    let mut cmd = crate::agent::build_command(AgentKind::Grok, &spec);
    eprintln!("argv: {:?}", cmd.as_std().get_args().collect::<Vec<_>>());
    let Ok(mut child) = cmd.spawn() else {
        eprintln!("grok not found: skipped");
        return;
    };
    let mut start = crate::agent::grok_acp_start(&spec);
    start.prompt = spec.prompt.clone();
    let (mut session, first) = AcpSession::start(start);
    let mut stdin = child.stdin.take();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    for line in first {
        stdin
            .as_mut()
            .unwrap()
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }
    let mut lines = Vec::new();
    let deadline = std::time::Duration::from_secs(180);
    while let Ok(Ok(Some(raw))) = tokio::time::timeout(deadline, out.next_line()).await {
        eprintln!("ACP< {}", raw.chars().take(600).collect::<String>());
        let step = session.on_line(&raw);
        for write in step.writes {
            eprintln!("ACP> {write}");
            if let Some(w) = stdin.as_mut() {
                let _ = w.write_all(format!("{write}\n").as_bytes()).await;
            }
        }
        for line in &step.lines {
            eprintln!("MAPPED {line}");
        }
        lines.extend(step.lines);
        if step.finished {
            stdin = None;
            break;
        }
    }
    drop(stdin);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait()).await;
    let got = events(&lines);
    eprintln!("EVENTS {got:?}");
    assert!(matches!(
        got.last(),
        Some(AgentEvent::Result {
            is_error: false,
            ..
        })
    ));
}

// ── Antigravity over `agy acp` (2026-11-20, agy 1.5.13) ─────────────────────

fn agy_params() -> StartParams {
    StartParams {
        cwd: "/tmp/acp".into(),
        prompt: "Run echo hi.".into(),
        resume: None,
        model: Some("gemini-4-neon".into()),
        effort: Some("high".into()),
        session_meta: json!({}),
        perm: "acceptEdits".into(),
        agent: AgentKind::Antigravity,
        mode: Some("accept-edits".into()),
        ask: true,
        mcp_servers: vec![json!({"name": "hollow_app", "command": "/bin/hollow",
            "args": ["app-mcp"], "env": []})],
    }
}

/// A recorded permission request (agy 1.5.13, `run_command` in accept-edits).
fn agy_request(rpc_id: u64) -> String {
    json!({"jsonrpc": "2.0", "id": rpc_id, "method": "session/request_permission",
        "params": {"sessionId": SID,
            "toolCall": {"sessionUpdate": "tool_call", "toolCallId": "call_1",
                "name": "run_command", "title": "echo hi", "kind": "execute",
                "status": "pending", "rawInput": {"CommandLine": "echo hi"}},
            "options": [{"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                {"optionId": "deny", "name": "Deny", "kind": "reject_once"}]}})
    .to_string()
}

/// The handshake: servers per session, then model, level and mode as config
/// options, then the prompt; a refused thinking level is not a failure.
fn agy_prompted() -> AcpSession {
    let (mut session, first) = AcpSession::start(agy_params());
    assert_eq!(parsed(&first[0])["method"], "initialize");
    let step = session.on_line(&response(1, init_result()));
    let new = parsed(&step.writes[0]);
    assert_eq!(new["method"], "session/new");
    assert_eq!(new["params"]["mcpServers"][0]["name"], "hollow_app");
    let step = session.on_line(&response(2, json!({"sessionId": SID})));
    assert!(step.lines[0].contains("\"transport\":\"acp\""));
    let mut write = parsed(&step.writes[0]);
    for (next, (id, value)) in (3u64..).zip([
        ("model", "gemini-4-neon"),
        ("thought_level", "high"),
        ("mode", "accept-edits"),
    ]) {
        assert_eq!(write["method"], "session/set_config_option");
        assert_eq!(
            write["params"],
            json!({"sessionId": SID, "configId": id, "value": value})
        );
        let answer = if id == "thought_level" {
            json!({"jsonrpc": "2.0", "id": next, "error": {"code": -32602, "message": "no such level"}})
                .to_string()
        } else {
            response(next, json!({"configOptions": []}))
        };
        let step = session.on_line(&answer);
        assert!(
            !step.finished,
            "a refused thinking level must not end the turn"
        );
        write = parsed(&step.writes[0]);
    }
    assert_eq!(write["method"], "session/prompt");
    session
}

#[test]
fn antigravity_permission_requests_wait_for_the_person_and_answer_by_option() {
    let mut session = agy_prompted();
    let step = session.on_line(&agy_request(41));
    assert!(step.writes.is_empty(), "nothing is answered for the person");
    assert_eq!(step.asks.len(), 1);
    let ask = &step.asks[0];
    assert_eq!(ask.tool, "run_command");
    assert_eq!(ask.action_text(), "Run the command: echo hi");
    let allow = parsed(&ask.reply(true));
    assert_eq!(allow["id"], 41);
    assert_eq!(
        allow["result"]["outcome"],
        json!({"outcome": "selected", "optionId": "allow"})
    );
    let deny = parsed(&ask.reply(false));
    assert_eq!(deny["result"]["outcome"]["optionId"], "deny");
    // No option of the kind asked for: cancelled, never a guess.
    let mut bare = ask.clone();
    bare.options.clear();
    assert_eq!(
        parsed(&bare.reply(true))["result"]["outcome"],
        json!({"outcome": "cancelled"})
    );
    // A denied tool reads as the denial it was when the turn ends.
    session.note_declined("call_1");
    let (lines, _) = feed(
        &mut session,
        &[
            update(json!({"sessionUpdate": "tool_call", "toolCallId": "call_1",
                "name": "run_command", "title": "echo hi", "status": "pending",
                "rawInput": {"CommandLine": "echo hi"}})),
            response(6, json!({"stopReason": "end_turn"})),
        ],
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("tool_result") && l.contains("is_error\":true")),
        "{lines:?}"
    );
}

#[test]
fn a_session_that_does_not_ask_still_declines_as_grok_does() {
    let mut params = agy_params();
    params.ask = false;
    let (mut session, _) = AcpSession::start(params);
    session.on_line(&response(1, init_result()));
    let step = session.on_line(&agy_request(5));
    assert!(step.asks.is_empty());
    assert_eq!(
        parsed(&step.writes[0])["result"]["outcome"]["optionId"],
        "deny"
    );
}

#[test]
fn antigravity_context_usage_and_tool_names_reach_the_parser() {
    let mut session = agy_prompted();
    let (lines, _) = feed(
        &mut session,
        &[
            update(json!({"sessionUpdate": "usage_update", "used": 0, "size": 1_048_576})),
            update(json!({"sessionUpdate": "usage_update", "used": 16860, "size": 1_048_576})),
            update(json!({"sessionUpdate": "tool_call", "toolCallId": "call_1",
                "name": "run_command", "title": "echo hi", "kind": "execute",
                "status": "in_progress", "rawInput": {"CommandLine": "echo hi"}})),
            update(json!({"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "done"}})),
            response(
                6,
                json!({"stopReason": "end_turn", "usage": {"inputTokens": 33577,
                    "outputTokens": 47, "cachedReadTokens": 1000, "cachedWriteTokens": 0,
                    "thoughtTokens": 0, "totalTokens": 33624}}),
            ),
        ],
    );
    let mut state = ParseState::default();
    let events: Vec<AgentEvent> = lines
        .iter()
        .flat_map(|l| parse_line(AgentKind::Antigravity, l, &mut state))
        .collect();
    let figures: Vec<(u64, u64)> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ContextFigure { tokens, window } => Some((*tokens, *window)),
            _ => None,
        })
        .collect();
    assert_eq!(figures, vec![(16860, 1_048_576)], "zero is not a reading");
    assert!(
        events.iter().any(|e| matches!(e,
        AgentEvent::ToolUse { name, .. } if name == "run_command")),
        "{events:?}"
    );
    let usage = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::Result { usage, .. } => Some(usage.clone()),
            _ => None,
        })
        .expect("a result");
    let tokens = usage.tokens.expect("tokens");
    assert_eq!(
        (tokens.input, tokens.cached_input, tokens.output),
        (32_577, 1_000, 47),
        "the full prompt less its cached read"
    );
}

#[test]
fn antigravity_failures_name_antigravity() {
    let (mut session, _) = AcpSession::start(agy_params());
    let step = session.on_line(
        &json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -1, "message": "no login"}})
            .to_string(),
    );
    assert!(
        step.lines[0].contains("Antigravity's ACP server refused to start"),
        "{:?}",
        step.lines
    );
}

// ── Hollow's servers for an Antigravity chat (2026-11-20) ───────────────────

/// The `{"mcpServers": ...}` config Hollow builds for Claude and Codex becomes
/// ACP's per-session `mcpServers`: stdio entries with `env` as name/value
/// pairs, `http` entries with `headers` likewise, claude.ai's proxy dropped.
#[test]
fn hollow_servers_and_connectors_become_acp_mcp_servers() {
    let config = json!({"mcpServers": {
        "hollow_app": {"type": "http", "url": "http://127.0.0.1:7/mcp",
            "headers": {"Authorization": "Bearer x"}},
        "plaud": {"command": "/opt/bin/plaud", "args": ["mcp"], "env": {"PATH": "/opt/bin"}},
        "slack": {"type": "claudeai-proxy", "url": "https://x", "id": "s"},
        "broken": {"type": "http"},
    }})
    .to_string();
    let servers = mcp_servers_from_config(Some(&config));
    assert_eq!(
        servers,
        vec![
            json!({"type": "http", "name": "hollow_app", "url": "http://127.0.0.1:7/mcp",
                "headers": [{"name": "Authorization", "value": "Bearer x"}]}),
            json!({"name": "plaud", "command": "/opt/bin/plaud", "args": ["mcp"],
                "env": [{"name": "PATH", "value": "/opt/bin"}]}),
        ]
    );
    assert!(mcp_servers_from_config(None).is_empty());
    assert!(mcp_servers_from_config(Some("")).is_empty());
    assert!(mcp_servers_from_config(Some("not json")).is_empty());
    assert!(mcp_servers_from_config(Some(r#"{"mcpServers": {}}"#)).is_empty());
}

/// agy spells an MCP tool `mcp_<server>_<tool>` (probed: `mcp_hollow_app_list_sessions`,
/// `mcp_linear_get_user`); Hollow's cards and chips read `mcp__<server>__<tool>`.
/// Only a name that starts with one of the session's own servers is rewritten.
#[test]
fn antigravity_mcp_tool_names_read_as_claudes() {
    let mut params = agy_params();
    params.mcp_servers = vec![
        json!({"name": "hollow_app", "command": "x", "args": [], "env": []}),
        json!({"name": "hollow", "command": "x", "args": [], "env": []}),
        json!({"name": "linear", "type": "http", "url": "https://x", "headers": []}),
    ];
    let (mut session, _) = AcpSession::start(params);
    let name = |session: &AcpSession, raw: &str| {
        session.named(&json!({"name": raw, "title": "Run it", "kind": "other"}))
    };
    assert_eq!(
        name(&session, "mcp_hollow_app_list_sessions"),
        "mcp__hollow_app__list_sessions",
        "the longest matching server wins"
    );
    assert_eq!(
        name(&session, "mcp_linear_get_user"),
        "mcp__linear__get_user"
    );
    assert_eq!(name(&session, "run_command"), "run_command");
    assert_eq!(name(&session, "mcp_unknown_tool"), "mcp_unknown_tool");
    session.on_line(&response(1, init_result()));
}
