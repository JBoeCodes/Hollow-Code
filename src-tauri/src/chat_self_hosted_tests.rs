//! Self-hosted models Q5 (2026-11-03): a chat turn on a self-hosted model,
//! through the attempt loop, the real command builder, the registry and the
//! real forwarder, against a stub endpoint that speaks Anthropic's
//! streaming Messages API. Attached from `chat_tests.rs` as `#[path] mod
//! self_hosted`, so `super::` is the chat tests (and through them
//! `crate::chat`).
//!
//! Claude Code itself is stood in for by a shell script
//! ([`STAND_IN`]): it is given exactly the argv, environment and folder the
//! real builder made for the turn, checks the self-hosted parts of them,
//! makes Claude Code's two Messages calls through the forwarder with the
//! run key (sent on stdin to curl, never on argv), and prints Claude Code's
//! stream-json for one tool call and the answer. What it does not cover:
//! Claude Code's own request shapes and SSE parsing, and the Hollow MCP
//! server answering the tool call (the stand-in reports the call and its
//! result the way Claude Code does). The opt-in
//! [`real_claude_code_calls_hollow_tasks_through_the_forwarder`] covers both
//! with the installed `claude` and the built `hollow` binary.
use super::*;
use crate::self_hosted_registry::{preset, SelfHostedAuth, SelfHostedEntry};

/// Claude Code's stand-in: `$0` is this script in a folder holding the
/// two request bodies; the rest is the built command's argv.
const STAND_IN: &str = r#"D=$(dirname "$0")
fail() { printf 'stand-in: %s\n' "$1" >&2; exit 4; }
read -r first || exit 3
printf '%s\n' "$first" > "$D/first.json"
case "${ANTHROPIC_BASE_URL:-}" in http://127.0.0.1:*) ;; *) fail "ANTHROPIC_BASE_URL is ${ANTHROPIC_BASE_URL:-unset}" ;; esac
case "${ANTHROPIC_AUTH_TOKEN:-}" in hsr_*) ;; *) fail "no run key" ;; esac
[ -z "${CLAUDE_CODE_OAUTH_TOKEN:-}" ] || fail "an OAuth token reached the run"
[ -z "${ANTHROPIC_API_KEY:-}" ] || fail "an API key reached the run"
[ "${ANTHROPIC_MODEL:-}" = qwen3.8-27b ] || fail "ANTHROPIC_MODEL is ${ANTHROPIC_MODEL:-unset}"
model=; prev=
for arg in "$@"; do
  [ "$prev" = --model ] && model=$arg
  prev=$arg
done
[ "$model" = qwen3.8-27b ] || fail "--model is $model"
case "$*" in *fake-*) fail "a credential reached argv" ;; esac
call() {
  printf 'Authorization: Bearer %s\n' "$ANTHROPIC_AUTH_TOKEN" | curl -sS -N -o "$D/$2" -w '%{http_code}' -H @- -H 'content-type: application/json' -H 'anthropic-version: 2023-06-01' --data-binary @"$D/$1" "$ANTHROPIC_BASE_URL/v1/messages"
}
echo '{"type":"system","subtype":"init","session_id":"sess-q5","model":"qwen3.8-27b","tools":["mcp__hollow_tasks__list_tasks"]}'
status=$(call one.json one.sse) || fail "the first call failed"
[ "$status" = 200 ] || fail "the first call answered $status"
grep -q '"name":"mcp__hollow_tasks__list_tasks"' "$D/one.sse" || fail "no tool call came back"
echo '{"type":"assistant","message":{"id":"msg-q1","type":"message","role":"assistant","model":"qwen3.8-27b","content":[{"type":"tool_use","id":"toolu-q1","name":"mcp__hollow_tasks__list_tasks","input":{"status":"open"}}]},"session_id":"sess-q5"}'
echo '{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu-q1","type":"tool_result","content":"2 open tasks: Ship Q5; Write the build log."}]},"session_id":"sess-q5"}'
status=$(call two.json two.sse) || fail "the second call failed"
[ "$status" = 200 ] || fail "the second call answered $status"
text=$(sed -n 's/.*"text_delta","text":"\([^"]*\)".*/\1/p' "$D/two.sse")
[ -n "$text" ] || fail "no answer text came back"
printf '{"type":"assistant","message":{"id":"msg-q2","type":"message","role":"assistant","model":"qwen3.8-27b","content":[{"type":"text","text":"%s"}]},"session_id":"sess-q5"}\n' "$text"
printf '{"type":"result","subtype":"success","is_error":false,"result":"%s","session_id":"sess-q5","total_cost_usd":0.4321,"usage":{"input_tokens":40,"output_tokens":12}}\n' "$text"
cat > /dev/null
"#;

/// The stand-in's first call: the question and the one Hollow tool.
const FIRST_CALL: &str = r#"{"model":"qwen3.8-27b","max_tokens":1024,"stream":true,"messages":[{"role":"user","content":"Which tasks are open?"}],"tools":[{"name":"mcp__hollow_tasks__list_tasks","description":"List tasks","input_schema":{"type":"object"}}]}"#;
/// Its second: the tool call and its result.
const SECOND_CALL: &str = r#"{"model":"qwen3.8-27b","max_tokens":1024,"stream":true,"messages":[{"role":"user","content":"Which tasks are open?"},{"role":"assistant","content":[{"type":"tool_use","id":"toolu-q1","name":"mcp__hollow_tasks__list_tasks","input":{"status":"open"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu-q1","content":"2 open tasks: Ship Q5; Write the build log."}]}],"tools":[{"name":"mcp__hollow_tasks__list_tasks","description":"List tasks","input_schema":{"type":"object"}}]}"#;

/// The endpoint's streamed answer to the first call: one tool call.
const TOOL_SSE: &str = concat!(
    "event: message_start\n",
    r#"data: {"type":"message_start","message":{"id":"msg_q1","type":"message","role":"assistant","model":"qwen3.8-27b","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":40,"output_tokens":0}}}"#,
    "\n\nevent: content_block_start\n",
    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_q1","name":"mcp__hollow_tasks__list_tasks","input":{}}}"#,
    "\n\nevent: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"status\":\"open\"}"}}"#,
    "\n\nevent: content_block_stop\n",
    r#"data: {"type":"content_block_stop","index":0}"#,
    "\n\nevent: message_delta\n",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":9}}"#,
    "\n\nevent: message_stop\n",
    r#"data: {"type":"message_stop"}"#,
    "\n\n",
);

/// The endpoint's streamed answer to the second call: the reply.
const ANSWER: &str = "Two tasks are open: Ship Q5 and Write the build log.";
const ANSWER_SSE: &str = concat!(
    "event: message_start\n",
    r#"data: {"type":"message_start","message":{"id":"msg_q2","type":"message","role":"assistant","model":"qwen3.8-27b","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":60,"output_tokens":0}}}"#,
    "\n\nevent: content_block_start\n",
    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    "\n\nevent: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Two tasks are open: Ship Q5 and Write the build log."}}"#,
    "\n\nevent: content_block_stop\n",
    r#"data: {"type":"content_block_stop","index":0}"#,
    "\n\nevent: message_delta\n",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":12}}"#,
    "\n\nevent: message_stop\n",
    r#"data: {"type":"message_stop"}"#,
    "\n\n",
);

/// One call the stub endpoint received.
#[derive(Clone, Debug)]
struct Call {
    path: String,
    head: String,
    body: String,
}

struct Endpoint {
    base: String,
    calls: Arc<Mutex<Vec<Call>>>,
    /// Q5 review: set when the forwarder closed a call the endpoint was
    /// still holding (with `hang`): the client's Stop reached it.
    closed: Arc<std::sync::atomic::AtomicBool>,
}

/// A stub of the self-hosted endpoint on loopback: each `POST` is recorded,
/// then answered with [`TOOL_SSE`], or [`ANSWER_SSE`] once the request
/// carries a tool result. With `hang`, it never answers (a cold start that
/// doesn't end).
async fn endpoint(hang: bool) -> Endpoint {
    endpoint_with(hang, |body| {
        if body.contains("tool_result") {
            ANSWER_SSE
        } else {
            TOOL_SSE
        }
    })
    .await
}

/// [`endpoint`] answering each request body with what `pick` gives (Q9:
/// OpenAI's Responses events for Codex).
async fn endpoint_with(hang: bool, pick: fn(&str) -> &'static str) -> Endpoint {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let log = calls.clone();
    let seen_close = closed.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let log = log.clone();
            let seen_close = seen_close.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                let end = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let head = String::from_utf8_lossy(&raw[..end]).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.trim()
                            .eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while raw.len() < end + length {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                }
                let body = String::from_utf8_lossy(&raw[end..]).into_owned();
                let path = head
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                let answer = pick(&body);
                log.lock().unwrap().push(Call { path, head, body });
                if hang {
                    // Hold the call until the forwarder lets go of it.
                    while stream.read(&mut buf).await.unwrap_or(0) > 0 {}
                    seen_close.store(true, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    Endpoint {
        base,
        calls,
        closed,
    }
}

/// A saved, switched-on entry `id` for the stub at `base` (no sign-in).
fn save_entry(id: &str, base: &str) {
    save_entry_as(id, base, crate::self_hosted_registry::API_ANTHROPIC);
}

/// [`save_entry`] with its API style (Q9: `openai` runs on Codex).
fn save_entry_as(id: &str, base: &str, api_style: &str) {
    crate::self_hosted_registry::save(
        SelfHostedEntry {
            id: id.into(),
            base_url: base.into(),
            api_style: api_style.into(),
            auth: SelfHostedAuth::None,
            ..preset()
        },
        false,
    )
    .unwrap();
}

/// The stand-in script and its two request bodies, in a folder of its own.
fn stand_in_folder() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("claude-stand-in.sh");
    std::fs::write(&script, STAND_IN).unwrap();
    std::fs::write(dir.path().join("one.json"), FIRST_CALL).unwrap();
    std::fs::write(dir.path().join("two.json"), SECOND_CALL).unwrap();
    (dir, script)
}

/// The stand-in run with `real`'s argv, environment (removals included)
/// and folder, leading its own group like a built command.
fn stand_in(real: &Command, script: &Path) -> Command {
    stand_in_as(real, script, "claude")
}

/// [`stand_in`] named `name` (`$0` of the shell; Q9: `codex`).
fn stand_in_as(real: &Command, script: &Path, name: &str) -> Command {
    let mut cmd = crate::process_group::test_support::isolated_shell(
        &format!("exec /bin/sh '{}' \"$@\"", script.display()),
        &[],
    );
    cmd.arg(name).args(real.as_std().get_args());
    for (name, value) in real.as_std().get_envs() {
        match value {
            Some(value) => {
                cmd.env(name, value);
            }
            None => {
                cmd.env_remove(name);
            }
        }
    }
    if let Some(dir) = real.as_std().get_current_dir() {
        cmd.current_dir(dir);
    }
    cmd.stderr(Stdio::piped());
    cmd
}

/// The real Claude command for a self-hosted turn's `spec`, checked: the
/// served model, the pinned settings, and no run key anywhere on argv.
fn built_for(spec: &agent::RunSpec, key: &str) -> Command {
    assert_eq!(spec.model.as_deref(), Some("qwen3.8-27b"));
    assert!(spec.extras.self_hosted.is_some(), "the turn has its target");
    // The fake credentials the stub turn's extra env carries (chat_tests'
    // harness) are there to be removed.
    assert!(spec
        .extras
        .env
        .iter()
        .any(|(k, _)| k == "ANTHROPIC_API_KEY"));
    let real = agent::command_without_codex_listing(AgentKind::Claude, spec);
    assert!(!agent::is_unavailable_command(&real), "the run may start");
    for (name, value) in real.as_std().get_envs() {
        if let Some(value) = value {
            assert!(
                !value.to_string_lossy().contains("fake-"),
                "{name:?} carries a credential"
            );
        }
    }
    let argv: Vec<String> = real
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(argv.iter().any(|a| a == "--settings"), "{argv:?}");
    assert!(
        !argv.iter().any(|a| a.contains(key)),
        "the key is never on argv"
    );
    real
}

/// Runs the next stub turn on `target`, until dropped.
struct OnTarget;

impl OnTarget {
    fn set(target: &crate::self_hosted::RunTarget) -> OnTarget {
        SELF_HOSTED_STUB.with(|stub| *stub.borrow_mut() = Some(target.clone()));
        OnTarget
    }
}

impl Drop for OnTarget {
    fn drop(&mut self) {
        SELF_HOSTED_STUB.with(|stub| *stub.borrow_mut() = None);
    }
}

/// Whether the forwarder at `base` still takes `key` (its status for a call).
async fn status_with(base: &str, key: &str) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/v1/messages"))
        .bearer_auth(key)
        .header("content-type", "application/json")
        .body(FIRST_CALL)
        .send()
        .await
        .map(|resp| resp.status().as_u16())
        .unwrap_or(0)
}

/// Q5's verify: a chat turn on `self-hosted:<id>` resolves the saved entry,
/// runs the real Claude command (stood in for) through the forwarder, gets
/// a Hollow tool call and then the answer from the endpoint, and completes
/// through Hollow's own stream parsing. The endpoint never sees the run
/// key, and the key is refused once the turn lets go of it.
#[tokio::test]
async fn a_self_hosted_chat_turn_calls_a_hollow_tool_and_answers() {
    let endpoint = endpoint(false).await;
    save_entry("q5-turn", &endpoint.base);
    let run = crate::self_hosted::turn_route(AgentKind::Claude, Some("self-hosted:q5-turn"))
        .await
        .unwrap()
        .expect("a self-hosted turn");
    let forwarder = run.forwarder().clone();
    let key = run.target.run_key.as_str().to_string();
    assert_eq!(forwarder.live_keys(), 1);
    assert_eq!(
        status_with(
            &forwarder.base_url(),
            "hsr_00000000000000000000000000000000"
        )
        .await,
        401
    );

    let dir = chat_vault();
    let vault = dir.path();
    let session = "s-20261103-5e1f0001";
    let ord = conversation_with_prior_turn(vault, session);
    let (_bin, script) = stand_in_folder();
    let stop = StopSignal::new();
    let builds = std::cell::Cell::new(0);
    let attempted = {
        let _on = OnTarget::set(&run.target);
        drive_live(
            "run-q5-turn",
            vault,
            session,
            ord,
            &ResumePlan::New,
            &stop,
            |spec: &agent::RunSpec| {
                builds.set(builds.get() + 1);
                stand_in(&built_for(spec, &key), &script)
            },
        )
        .await
    };
    assert_eq!(builds.get(), 1);
    let AttemptsEnd::Driven {
        driven,
        stream,
        retried,
        failover_stop,
        ..
    } = attempted.end
    else {
        panic!("the turn ran");
    };
    assert!(
        matches!(driven.status, Some(Ok(ref status)) if status.success()),
        "{:?} {:?}",
        driven.status,
        driven.stderr_tail
    );
    assert!(!retried);
    assert_eq!(failover_stop, None);
    assert_eq!(stream.last_session_id.as_deref(), Some("sess-q5"));
    assert!(stream.assistant.contains(ANSWER), "{}", stream.assistant);
    let tools = attempted.sink.of_type("toolUse");
    assert_eq!(tools.len(), 1, "{}", attempted.sink.text());
    assert_eq!(tools[0]["name"], "mcp__hollow_tasks__list_tasks");
    assert_eq!(attempted.sink.of_type("toolResult").len(), 1);
    assert_eq!(attempted.evidence.tool_calls_seen(), Some(1));
    assert!(attempted.sink.text().contains(ANSWER));

    // The endpoint got both calls, without the run key.
    let calls = endpoint.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2, "{calls:?}");
    for call in &calls {
        assert_eq!(call.path, "/v1/messages");
        assert!(!call.head.contains("hsr_"), "{}", call.head);
        assert!(call
            .head
            .to_ascii_lowercase()
            .contains("anthropic-version: 2023-06-01"));
    }
    assert!(calls[1].body.contains("tool_result"));
    let _ = attempted.ledger.finish(LedgerOutcome::Success);

    // The turn ends: its key goes with it.
    drop(run);
    assert_eq!(forwarder.live_keys(), 0);
    assert_eq!(status_with(&forwarder.base_url(), &key).await, 401);
    assert_eq!(endpoint.calls.lock().unwrap().len(), 2, "never forwarded");
    crate::self_hosted_forwarder::stop("q5-turn");
}

/// Q5's verify: Stop ends a self-hosted turn whose endpoint is still
/// starting (the stand-in waiting on its first call), with no retry, and
/// the turn's key is refused once the turn lets go of it.
#[tokio::test]
async fn stop_ends_a_self_hosted_turn_and_its_key() {
    let endpoint = endpoint(true).await;
    save_entry("q5-stop", &endpoint.base);
    let run = crate::self_hosted::turn_route(AgentKind::Claude, Some("self-hosted:q5-stop"))
        .await
        .unwrap()
        .expect("a self-hosted turn");
    let forwarder = run.forwarder().clone();
    let key = run.target.run_key.as_str().to_string();

    let dir = chat_vault();
    let vault = dir.path();
    let session = "s-20261103-5e1f0002";
    let ord = conversation_with_prior_turn(vault, session);
    let (_bin, script) = stand_in_folder();
    let stop = Arc::new(StopSignal::new());
    let stopper = Arc::clone(&stop);
    let calls = endpoint.calls.clone();
    // Stop once the endpoint holds the turn's first call.
    tokio::spawn(async move {
        for _ in 0..600 {
            if !calls.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        stopper.request();
    });
    let builds = std::cell::Cell::new(0);
    let attempted = {
        let _on = OnTarget::set(&run.target);
        drive_live(
            "run-q5-stop",
            vault,
            session,
            ord,
            &ResumePlan::New,
            &stop,
            |spec: &agent::RunSpec| {
                builds.set(builds.get() + 1);
                stand_in(&built_for(spec, &key), &script)
            },
        )
        .await
    };
    assert_eq!(builds.get(), 1, "never retried");
    let AttemptsEnd::Driven {
        driven, retried, ..
    } = attempted.end
    else {
        panic!("the turn ran");
    };
    assert_eq!(driven.ended, Ended::Stopped, "{:?}", driven.stderr_tail);
    assert!(!retried);
    assert_eq!(endpoint.calls.lock().unwrap().len(), 1);
    assert!(!attempted.sink.text().contains(ANSWER));
    let _ = attempted.ledger.finish(LedgerOutcome::Cancelled);
    // The Stop reached the endpoint: the forwarder closed the call it held.
    let mut waited = 0;
    while !endpoint.closed.load(std::sync::atomic::Ordering::SeqCst) && waited < 200 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        waited += 1;
    }
    assert!(
        endpoint.closed.load(std::sync::atomic::Ordering::SeqCst),
        "the forwarder let go of the endpoint's call"
    );

    // While the turn holds it the key is live; once it lets go, refused.
    assert_eq!(forwarder.live_keys(), 1);
    drop(run);
    assert_eq!(forwarder.live_keys(), 0);
    assert_eq!(status_with(&forwarder.base_url(), &key).await, 401);
    crate::self_hosted_forwarder::stop("q5-stop");
}

/// Q5: a self-hosted turn whose stream says it hit a limit never moves to
/// another Claude account, even with accounts that would take it: it has
/// no account, so nothing fails over or switches at the ceiling.
#[tokio::test]
async fn a_self_hosted_turn_never_moves_to_another_claude_account() {
    let target = crate::self_hosted::RunTarget {
        entry: "qwen".into(),
        name: "Qwen3.8 27B".into(),
        base_url: "http://127.0.0.1:41234".into(),
        run_key: crate::self_hosted::RunKey::from("hsr_0123456789abcdef0123456789abcdef"),
        served_model: "qwen3.8-27b".into(),
        context_window: 131_072,
        effort_levels: vec!["low".into(), "medium".into(), "xhigh".into()],
        api_style: crate::self_hosted_registry::API_ANTHROPIC.into(),
        codex_home: None,
    };
    let accounts = StubAccounts::two();
    let first = lines(&[FO_INIT, FO_WORKING, FO_REJECTED, FO_LIMITED]);
    let second = lines(&[FO_INIT, FO_DONE_TEXT, FO_DONE]);
    let (_dir, _ord, run, spawns) = {
        let _on = OnTarget::set(&target);
        drive_failover("s-20261103-5e1f0003", &first, &second, 0, &accounts).await
    };
    assert_eq!(spawns.len(), 1, "no second attempt on another account");
    assert_eq!(spawns[0].2, None, "no account was ever forced");
    assert!(run.sink.of_type("accountSwitched").is_empty());
    assert_eq!(accounts.probes.get(), 0, "no account was even probed");
    let _ = run.ledger.finish(LedgerOutcome::Error);
}

/// Q5's model switch hazard: Claude's thinking blocks carry Anthropic
/// signatures and a self-hosted model's don't, so a conversation that moves
/// between the two must not resume the same Claude Code session. The
/// chat's own model id is part of the stint fingerprint (`chat_resume`
/// `PARTS`, `execute_turn`'s `input_at`), so either direction resets with
/// `model` named; the same model resumes.
#[test]
fn switching_between_self_hosted_and_anthropic_models_starts_a_fresh_session() {
    use crate::chat_resume::{config_fingerprint, config_parts, plan_resume_with, reset_parts};
    use crate::session_store::{OpenStint, ResumeState};
    let cwd = Path::new("/Users/x/vault/workspaces/main");
    let input = |model: Option<&'static str>| crate::chat_resume::FingerprintInput {
        agent: AgentKind::Claude,
        model,
        perm: "acceptEdits",
        cwd,
        mcp_servers: &[],
        caps: crate::mcp_attach::AttachedCaps::default(),
        canvases: crate::visual_prompt::CanvasFrequency::Helps,
        workspace: "workspace",
        skills: &[],
    };
    let held_by = |model: Option<&'static str>| ResumeState {
        open: Some(OpenStint {
            agent: "claude".into(),
            harness_session_id: Some("sess-q5".into()),
            config_fingerprint: Some(config_fingerprint(&input(model))),
            config_parts: Some(config_parts(&input(model))),
        }),
        last_closed: None,
    };
    let qwen = Some("self-hosted:qwen");
    for (from, to) in [
        (qwen, Some("opus")),
        (Some("opus"), qwen),
        (qwen, Some("claude-opus-5-5")),
        (qwen, None),
        (None, qwen),
        (qwen, Some("self-hosted:other")),
    ] {
        let state = held_by(from);
        let now = input(to);
        assert_eq!(
            plan_resume_with(Some("sess-q5"), &state, &config_fingerprint(&now), false),
            ResumePlan::Reset(ResetReason::ConfigChanged),
            "{from:?} to {to:?}"
        );
        assert_eq!(reset_parts(&state, &config_parts(&now)), vec!["model"]);
    }
    assert_eq!(
        plan_resume_with(
            Some("sess-q5"),
            &held_by(qwen),
            &config_fingerprint(&input(qwen)),
            false
        ),
        ResumePlan::Resume("sess-q5".into())
    );
}

/// Q5 review: `execute_turn`'s template takes its model and its
/// self-hosted target from one call (`self_hosted::turn_spec_model`), so
/// the served id never goes without its target, and nothing in the chat
/// sets either afterwards. With `validate_run_spec` refusing a known served
/// id without a target, a slip would fail closed rather than reach
/// Anthropic. A self-hosted chat's claude.ai connectors, retitle and
/// learning follow from the same check of the chat's model.
#[test]
fn the_template_takes_its_model_and_target_together() {
    let source = chat_source();
    let turn = fn_body(source, concat!("pub(crate) async fn ", "execute_turn("));
    let call = concat!(
        "self_hosted::turn_spec_model(",
        "model.as_deref(), self_hosted_run.as_ref())"
    );
    assert_eq!(turn.matches(call).count(), 1);
    let at = turn.find(call).unwrap();
    let start = turn
        .find(concat!("let template = ", "agent::RunSpec {"))
        .unwrap();
    assert!(at < start, "decided before the template");
    let template = &turn[start..];
    let template = &template[..template.find("\n    };\n").unwrap()];
    assert!(template.contains(concat!("model: ", "spec_model,")));
    assert!(template.contains(concat!("self_hosted: ", "self_hosted_target,")));
    // Q5 delta review: only the turn's own spec and template, inside
    // `execute_turn`, so an unrelated `.model = ` elsewhere doesn't trip it.
    for setter in [
        concat!("spec.model", " ="),
        concat!("template.model", " ="),
        concat!("spec.extras.self_hosted", " ="),
        concat!("template.extras.self_hosted", " ="),
    ] {
        assert!(!turn.contains(setter), "execute_turn sets {setter}");
    }
    // The chat's own model decides the rest, before connectors are merged.
    let check = concat!(
        "self_hosted::chat_on_self_hosted(",
        "agent, model.as_deref())"
    );
    assert_eq!(turn.matches(check).count(), 1);
    // Q10: its retitle and learning run on its own entry when Background
    // jobs sends them there, and only there; skipped otherwise.
    assert!(turn.contains(concat!("&& !unattended && !bare && ", "titles_ok,")));
    assert!(turn.contains(concat!(
        "turn_learns(typed, app_depth, unattended, bare)\n",
        "        && on_own(crate::self_hosted_jobs::JobKind::Learning);"
    )));
    assert!(turn.contains(concat!(
        ".is_none_or(|own| crate::self_hosted_jobs::",
        "routes_to(kind, own))"
    )));
    assert!(turn.contains(concat!("learn_from.self_hosted = ", "own_entry.clone();")));
    let filtered = turn
        .find(concat!("connectors::", "without_claude_login(shared)"))
        .unwrap();
    let merged = turn
        .find(concat!("mcp_attach::", "merge(servers)"))
        .unwrap();
    assert!(filtered < merged);
}

/// Q7: `execute_turn` places a turn once (`remote_session::turn_place`),
/// with the chat's own self-hosted check, before anything reads where it
/// runs (a stale turn's choice, the fingerprint's folder, the runner's
/// preparation and `begin_turn`), and never asks Runs on or the unattended
/// default itself, so a self-hosted turn kept here sends nothing to the
/// runner. Its note is recorded once its route resolved and before
/// `wanted`, so a refused route says only why it was refused. Q7 review:
/// the route resolves before a stale remote turn is settled, and what to
/// record after the settle comes from one function, `settled_notices` (the
/// note first, then that earlier turn's return; tested below), so the note
/// is the turn's first line even after a quit during a remote turn.
#[test]
fn a_self_hosted_turn_is_placed_before_anything_reads_where_it_runs() {
    let source = chat_source();
    let turn = fn_body(source, concat!("pub(crate) async fn ", "execute_turn("));
    let place = concat!("remote_session::", "turn_place(");
    assert_eq!(turn.matches(place).count(), 1);
    let at = turn.find(place).unwrap();
    let call = &turn[at..at + turn[at..].find(");").unwrap()];
    assert!(call.contains(concat!("self_hosted", "_chat,")), "{call}");
    assert!(call.contains(concat!("PlaceReads::", "LIVE")), "{call}");
    for direct in [
        concat!("remote_session::", "runs_on("),
        concat!("remote_session::", "unattended_runner("),
    ] {
        assert!(!turn.contains(direct), "execute_turn calls {direct}");
    }
    for later in [
        concat!("remote_session::", "stale_turn("),
        concat!("let fingerprint", "_cwd"),
        concat!("remote_session::", "prepare("),
        concat!("remote_session::", "prepare_folder("),
        concat!("remote_session::", "prepare_with("),
        concat!("remote_session::", "begin_turn("),
    ] {
        let found = turn.find(later).unwrap_or_else(|| panic!("{later}"));
        assert!(at < found, "{later} comes after the place is decided");
    }
    // Q7 review: the route, the stale turn's settle (its host from the
    // place, `stale_host`), then what to record (`settled_notices`: the note
    // first, then the earlier turn's return; tested below), all before the
    // fingerprint's folder and `wanted`, so a refused route says only why.
    let find = |needle: &str| turn.find(needle).unwrap_or_else(|| panic!("{needle}"));
    let route = find(concat!("self_hosted::", "turn_route("));
    let stale = find(concat!("remote_session::", "stale_turn("));
    let settle = find(concat!("remote_session::", "settle_stale("));
    let settled = concat!("settled_", "notices(");
    assert_eq!(turn.matches(settled).count(), 1);
    let noted = find(settled);
    let fingerprint = find(concat!("let fingerprint", "_cwd"));
    let wanted = find(concat!("let wanted = ", "match (runs_on, preferred)"));
    assert!(
        route < stale,
        "the route resolves before a stale turn is settled"
    );
    assert!(
        stale < settle && settle < noted,
        "the note counts the settle"
    );
    assert!(noted < fingerprint && fingerprint < wanted);
    assert!(turn[stale..settle].contains(concat!("stale_host(", "&runs_on, &preferred)")));
    // Q7 follow-up: whether the stale turn's record is still there is read
    // again between the settle and `settled_notices`, and that is what the
    // call is given, so a runner that didn't answer says `pending` rather
    // than that only a turn there on a model that isn't self-hosted brings
    // the work back.
    // One binding, read once: a hardcoded `false` fails here.
    let recheck = concat!(
        "let still_stale = stay.is_some() && crate::remote_session::",
        "stale_turn(&session_key).is_some();"
    );
    let rechecked = find(recheck);
    assert!(
        settle < rechecked && rechecked < noted,
        "re-read after the settle"
    );
    assert_eq!(turn.matches(concat!("still", "_stale")).count(), 2);
    let call = &turn[noted..noted + turn[noted..].find(") {").unwrap()];
    assert!(
        call.contains(concat!(
            "earlier_back.as_ref(),\n        ",
            "still_stale,\n"
        )),
        "{call}"
    );
    // Every notice it returns is recorded, in order, and nothing else in the
    // turn records the earlier turn's return or a self-hosted note.
    let recorded = &turn[noted..fingerprint];
    assert!(recorded.contains(concat!(
        "record_remote",
        "_notice(&out, &vault, &session_key, turn_ord, notice)"
    )));
    assert!(recorded.contains(concat!("remote_run", "_trail(&vault, &session_key, 40)")));
    assert!(!turn.contains(concat!("record_remote", "_back(")));
    assert!(!turn.contains(concat!("self_hosted::", "runner_note(")));
    let body = fn_body(source, concat!("fn settled_", "notices("));
    assert!(body.contains(concat!("self_hosted::", "runner_note(")));
    assert!(body.contains(concat!("map(", "remote_back_notices)")));
    // The old Q5 arm (a self-hosted turn the unattended default sent) is gone:
    // such a turn never reaches the runner's choice.
    assert!(!turn[wanted..].contains(concat!("self_hosted_run", ".is_some()")));
}

/// Q7 review: a self-hosted turn whose Runs on is the runner gives a stale
/// remote turn's choice no host (`stale_host` from its `turn_place`), so
/// even a Continue brings that turn back rather than reattaching there; a
/// Claude model's turn with the same settings gives the runner's.
#[test]
fn a_self_hosted_turn_gives_a_stale_turn_no_host() {
    use crate::remote_session::{stale_choice, turn_place, PlaceReads, StaleChoice, StaleTurn};
    let vault = tempfile::tempdir().unwrap();
    let tree = vault.path().join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    crate::build_workspaces::bind_repo_for_test(
        vault.path(),
        "bw",
        &vault.path().join("repo"),
        &tree,
    );
    crate::build_workspaces::set_runs_on(vault.path(), "bw", Some("mac-mini")).unwrap();
    let reads = PlaceReads {
        settings: || Some(crate::agent_settings::AgentSettings::default()),
        runner: || Some("runner-host".into()),
    };
    let stale = StaleTurn {
        target: crate::remote_session::RemoteTarget {
            host: "runner-host".into(),
            worktree: tree.clone(),
            git_worktree: true,
            ..Default::default()
        },
        agent: "claude".into(),
        reattach: Some(Default::default()),
    };
    let go = crate::restart_resume::CONTINUE_PROMPT;
    for prefers in [false, true] {
        let place = turn_place(vault.path(), &tree, AgentKind::Claude, prefers, true, reads);
        let host = stale_host(&place.chosen, &place.preferred);
        assert_eq!(host, None, "a self-hosted turn names no host");
        assert_eq!(
            stale_choice(&stale, go, AgentKind::Claude, host.as_deref(), &tree),
            StaleChoice::BringBack
        );
        let claude = turn_place(
            vault.path(),
            &tree,
            AgentKind::Claude,
            prefers,
            false,
            reads,
        );
        let host = stale_host(&claude.chosen, &claude.preferred);
        assert_eq!(host.as_deref(), Some("runner-host"));
        assert_eq!(
            stale_choice(&stale, go, AgentKind::Claude, host.as_deref(), &tree),
            StaleChoice::Reattach
        );
    }
}

/// Q7 review: what a turn records once a stale remote turn was settled
/// (`settled_notices`), for each outcome: a self-hosted turn whose Runs on
/// is the runner gets its note first, then the earlier turn's return, and
/// the note says what of that turn is still on the runner and who brings it
/// back. The runner isn't asked: the settle's outcome, whether the stale
/// record is still there, and the chat's start and back notices decide.
#[tokio::test]
async fn a_settled_stale_turn_records_the_note_first_and_says_what_is_left() {
    use crate::remote_session::Collected;
    let entry = SelfHostedEntry {
        base_url: "http://127.0.0.1:9".into(),
        ..preset()
    };
    let forwarder = std::sync::Arc::new(
        crate::self_hosted_forwarder::Forwarder::start(entry.upstream())
            .await
            .unwrap(),
    );
    let run = crate::self_hosted::turn_run(&entry, forwarder);
    let stay = Some(("runner-host", &run));
    let start = serde_json::json!({ "type": "remoteRun", "phase": "start", "host": "runner-host" })
        .to_string();
    let trail = || vec![start.clone()];
    let worktree = |collected: Result<Collected, String>, still_there: bool| RemoteBack {
        host: "runner-host",
        worktree: Some((collected, still_there)),
        folder: None,
        local_folder: None,
        earlier: true,
    };
    let left = |notices: &[Value]| notices[0]["selfHosted"]["leftOnRunner"].clone();
    let key = "s-20261103-q7settle";

    // The settle brought the earlier turn back: the note, then its return.
    for came in [
        Ok(Collected::Applied { head: "abc".into() }),
        Ok(Collected::Unchanged),
        Ok(Collected::KeptAside {
            branch: "w-mini-1".into(),
        }),
        // Came back, but couldn't be applied: on a branch here.
        Err("couldn't apply it here; it is on w-mini-1".into()),
    ] {
        let back = worktree(came.clone(), false);
        let notices = settled_notices(stay, Some(&back), false, trail, key);
        assert_eq!(notices.len(), 2, "{came:?}");
        assert_eq!(notices[0]["phase"], "stayed");
        assert_eq!(notices[0]["host"], "runner-host");
        assert_eq!(notices[1]["phase"], "back");
        assert_eq!(notices[1]["earlier"], true);
        assert_eq!(left(&notices), Value::Null, "{came:?}");
    }
    // A failed return that left the work there, worktree or folder: only a
    // turn there brings it back (`runner`).
    let failed = worktree(Err("no route".into()), true);
    let notices = settled_notices(stay, Some(&failed), false, trail, key);
    assert_eq!(notices[1]["on"], "runner");
    assert_eq!(left(&notices), "runner");
    let folder = RemoteBack {
        host: "runner-host",
        worktree: None,
        folder: Some(Err("later".into())),
        local_folder: None,
        earlier: true,
    };
    let notices = settled_notices(stay, Some(&folder), false, trail, key);
    assert_eq!(notices[1]["scope"], "folder");
    assert_eq!(left(&notices), "runner");
    // Nothing came back and the record went (its run was gone): what it
    // left there waits for a turn there (`runner`).
    let notices = settled_notices(stay, None, false, trail, key);
    assert_eq!(notices.len(), 1);
    assert_eq!(left(&notices), "runner");
    // Nothing came back and the record stayed (the runner didn't answer):
    // a later message tries again (`pending`), whatever the notices say,
    // and the trail isn't read.
    let notices = settled_notices(stay, None, true, || panic!("read"), key);
    assert_eq!(notices.len(), 1);
    assert_eq!(left(&notices), "pending");
    // No remote turn ever: nothing left.
    assert_eq!(
        left(&settled_notices(stay, None, false, Vec::new, key)),
        Value::Null
    );
    // An earlier failure, recorded, with self-hosted notes since (the trail
    // holds only start and back): still there.
    let recorded = remote_back_notices(&failed)[0].to_string();
    let notices = settled_notices(stay, None, false, || vec![start.clone(), recorded], key);
    assert_eq!(left(&notices), "runner");
    // Not a self-hosted turn kept here: only the earlier turn's return, as
    // `record_remote_back` would record it, and no trail is read.
    let notices = settled_notices(None, Some(&failed), false, || panic!("read"), key);
    assert_eq!(notices, remote_back_notices(&failed));
    assert!(settled_notices(None, None, true, || panic!("read"), key).is_empty());
}

/// Opt-in (`cargo build --bin hollow && cargo test --lib real_claude_code
/// -- --ignored`): the installed `claude` itself runs a self-hosted turn
/// against the stub endpoint through the real forwarder, with Hollow's
/// `hollow_tasks` server (the built `hollow` binary, read-only on the test
/// vault) attached: it calls `mcp__hollow_tasks__list_tasks`, sends the
/// server's answer back and finishes with the endpoint's reply. Costs
/// nothing: no call leaves this Mac. Ignored by default because it needs
/// both binaries.
#[tokio::test]
#[ignore = "needs the installed claude and the built hollow binary; cargo build --bin hollow, then cargo test --lib real_claude_code -- --ignored"]
async fn real_claude_code_calls_hollow_tasks_through_the_forwarder() {
    let claude = crate::agent::installed_binary(AgentKind::Claude)
        .expect("claude isn't installed where Hollow looks for it");
    let exe = std::env::current_exe().unwrap();
    let hollow = exe
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("hollow"))
        .filter(|bin| bin.is_file())
        .expect("build the app binary first: cargo build --bin hollow");
    eprintln!("real claude: {}", claude.display());
    let endpoint = endpoint(false).await;
    save_entry("q5-real", &endpoint.base);
    let run = crate::self_hosted::turn_route(AgentKind::Claude, Some("self-hosted:q5-real"))
        .await
        .unwrap()
        .expect("a self-hosted turn");
    let forwarder = run.forwarder().clone();
    let key = run.target.run_key.as_str().to_string();

    let dir = chat_vault();
    let vault = dir.path();
    let session = "s-20261103-5e1f0004";
    let ord = conversation_with_prior_turn(vault, session);
    let parent = crate::usage_ledger::DelegateParent {
        surface: crate::usage_ledger::Surface::Chat,
        scope: session.into(),
        run_id: "run-q5-real".into(),
        scratch: None,
        tool_limits: Default::default(),
    };
    let (name, tasks) = crate::tasks_mcp::entry_for(
        &hollow,
        &vault.join("no-broker.sock"),
        vault,
        &parent,
        crate::tasks_mcp::TasksAccess::ReadOnly,
        "no-proof",
    );
    let mut servers = serde_json::Map::new();
    servers.insert(name.to_string(), tasks);
    let config = json!({ "mcpServers": servers }).to_string();
    let stop = StopSignal::new();
    let attempted = {
        let _on = OnTarget::set(&run.target);
        drive_live(
            "run-q5-real",
            vault,
            session,
            ord,
            &ResumePlan::New,
            &stop,
            |spec: &agent::RunSpec| {
                let mut spec = spec.clone();
                spec.mcp_config = Some(config.clone());
                let cmd = agent::build_command(AgentKind::Claude, &spec);
                assert!(!agent::is_unavailable_command(&cmd));
                let argv: Vec<String> = cmd
                    .as_std()
                    .get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                assert!(!argv.iter().any(|a| a.contains(&key)));
                cmd
            },
        )
        .await
    };
    let AttemptsEnd::Driven { driven, stream, .. } = attempted.end else {
        panic!("the turn ran");
    };
    let calls = endpoint.calls.lock().unwrap().clone();
    assert!(
        matches!(driven.status, Some(Ok(ref status)) if status.success()),
        "{:?} {:?} calls: {calls:?}",
        driven.status,
        driven.stderr_tail
    );
    let tools = attempted.sink.of_type("toolUse");
    assert!(
        tools
            .iter()
            .any(|t| t["name"] == "mcp__hollow_tasks__list_tasks"),
        "{}",
        attempted.sink.text()
    );
    assert!(stream.assistant.contains(ANSWER), "{}", stream.assistant);
    assert!(
        calls.iter().all(|c| c.path.starts_with("/v1/messages")),
        "{calls:?}"
    );
    assert!(calls.iter().all(|c| !c.head.contains("hsr_")));
    let answered = calls
        .iter()
        .find(|c| c.body.contains("tool_result"))
        .expect("the tool's result went back to the endpoint");
    assert!(answered.body.contains("toolu_q1"));
    assert!(
        !answered.body.contains("No such tool"),
        "hollow_tasks answered: {}",
        answered.body
    );
    let _ = attempted.ledger.finish(LedgerOutcome::Success);
    drop(run);
    assert_eq!(forwarder.live_keys(), 0);
    crate::self_hosted_forwarder::stop("q5-real");
}

// ── Q9: a Codex chat turn on a self-hosted model ────────────────────────

/// Codex's stand-in (`codex exec --json`): `$0` is this script in a folder
/// holding the two Responses request bodies and `home.txt` (the entry's
/// Codex home); the rest is the built command's argv. It checks the
/// self-hosted parts of its argv and environment, makes Codex's two
/// `POST /v1/responses` calls through the forwarder at the provider's
/// `base_url` with the run key from `HOLLOW_SELF_HOSTED_KEY` (on stdin to
/// curl, never argv), and prints Codex's `--json` lines for one Hollow tool
/// call and the answer.
const CODEX_STAND_IN: &str = r#"D=$(dirname "$0")
fail() { printf 'stand-in: %s\n' "$1" >&2; exit 4; }
case "${HOLLOW_SELF_HOSTED_KEY:-}" in hsr_*) ;; *) fail "no run key" ;; esac
[ "${CODEX_HOME:-}" = "$(cat "$D/home.txt")" ] || fail "CODEX_HOME is ${CODEX_HOME:-unset}"
[ -d "$CODEX_HOME" ] || fail "the Codex home wasn't made"
[ ! -e "$CODEX_HOME/auth.json" ] || fail "a sign-in is in the Codex home"
for v in OPENAI_API_KEY CODEX_API_KEY OPENAI_BASE_URL ANTHROPIC_API_KEY CLAUDE_CODE_OAUTH_TOKEN ANTHROPIC_AUTH_TOKEN; do
  eval "val=\${$v:-}"
  [ -z "$val" ] || fail "$v reached the run"
done
model=; prev=; base=; provider=; apps=
for arg in "$@"; do
  [ "$prev" = -m ] && model=$arg
  [ "$prev" = --disable ] && [ "$arg" = apps ] && apps=1
  case "$arg" in
    'model_provider="hollow-self-hosted"') provider=1 ;;
    model_providers.hollow-self-hosted=*) base=$(printf '%s' "$arg" | sed -n 's/.*base_url="\([^"]*\)".*/\1/p') ;;
  esac
  prev=$arg
done
[ "$model" = qwen3.8-27b ] || fail "-m is $model"
[ -n "$provider" ] || fail "no self-hosted provider"
[ -n "$apps" ] || fail "apps weren't disabled"
case "$base" in http://127.0.0.1:*/v1) ;; *) fail "base_url is $base" ;; esac
case "$*" in *fake-*) fail "a credential reached argv" ;; *hsr_*) fail "the run key reached argv" ;; esac
call() {
  printf 'Authorization: Bearer %s\n' "$HOLLOW_SELF_HOSTED_KEY" | curl -sS -N -o "$D/$2" -w '%{http_code}' -H @- -H 'content-type: application/json' -H 'accept: text/event-stream' --data-binary @"$D/$1" "$base/responses"
}
echo '{"type":"thread.started","thread_id":"0199b9a0-9c0d-7e00-8000-00000000c0de"}'
echo '{"type":"turn.started"}'
status=$(call one.json one.sse) || fail "the first call failed"
[ "$status" = 200 ] || fail "the first call answered $status"
grep -q '"namespace":"mcp__hollow_tasks","name":"list_tasks"' "$D/one.sse" || fail "no tool call came back"
echo '{"type":"item.started","item":{"id":"item_0","type":"mcp_tool_call","server":"hollow_tasks","tool":"list_tasks","arguments":{"status":"open"},"result":null,"error":null,"status":"in_progress"}}'
echo '{"type":"item.completed","item":{"id":"item_0","type":"mcp_tool_call","server":"hollow_tasks","tool":"list_tasks","arguments":{"status":"open"},"result":{"content":[{"type":"text","text":"2 open tasks: Ship Q9; Write the build log."}],"structured_content":null},"error":null,"status":"completed"}}'
status=$(call two.json two.sse) || fail "the second call failed"
[ "$status" = 200 ] || fail "the second call answered $status"
text=$(sed -n 's/.*"type":"response.output_text.delta".*"delta":"\([^"]*\)".*/\1/p' "$D/two.sse")
[ -n "$text" ] || fail "no answer text came back"
printf '{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"%s"}}\n' "$text"
echo '{"type":"turn.completed","usage":{"input_tokens":40,"cached_input_tokens":0,"output_tokens":12,"reasoning_output_tokens":0}}'
"#;

/// Codex's first Responses call: the question and the one Hollow tool, in
/// the `namespace` shape Codex 0.164 sends an MCP server's tools in
/// (probed 2026-11-03 against a loopback stub).
const CODEX_FIRST_CALL: &str = r#"{"model":"qwen3.8-27b","stream":true,"store":false,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"Which tasks are open?"}]}],"tools":[{"type":"namespace","name":"mcp__hollow_tasks","description":"Tools in the mcp__hollow_tasks namespace.","tools":[{"type":"function","name":"list_tasks","description":"List tasks","strict":false,"parameters":{"type":"object"}}]}],"tool_choice":"auto","parallel_tool_calls":true}"#;
/// Its second: the call and its output, the whole history again
/// (`store: false`).
const CODEX_SECOND_CALL: &str = r#"{"model":"qwen3.8-27b","stream":true,"store":false,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"Which tasks are open?"}]},{"type":"function_call","call_id":"call_q9","namespace":"mcp__hollow_tasks","name":"list_tasks","arguments":"{\"status\":\"open\"}"},{"type":"function_call_output","call_id":"call_q9","output":[{"type":"input_text","text":"2 open tasks: Ship Q9; Write the build log."}]}],"tools":[{"type":"namespace","name":"mcp__hollow_tasks","description":"Tools in the mcp__hollow_tasks namespace.","tools":[{"type":"function","name":"list_tasks","description":"List tasks","strict":false,"parameters":{"type":"object"}}]}],"tool_choice":"auto","parallel_tool_calls":true}"#;

/// The endpoint's Responses stream for the first call: one tool call.
const CODEX_TOOL_SSE: &str = concat!(
    "event: response.created\n",
    r#"data: {"type":"response.created","response":{"id":"resp_q9a"}}"#,
    "\n\nevent: response.output_item.done\n",
    r#"data: {"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_q9","call_id":"call_q9","namespace":"mcp__hollow_tasks","name":"list_tasks","arguments":"{\"status\":\"open\"}"}}"#,
    "\n\nevent: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_q9a","usage":{"input_tokens":40,"output_tokens":9,"total_tokens":49}}}"#,
    "\n\n",
);

/// The answer Codex's second call gets.
const CODEX_ANSWER: &str = "Two tasks are open: Ship Q9 and Write the build log.";
const CODEX_ANSWER_SSE: &str = concat!(
    "event: response.created\n",
    r#"data: {"type":"response.created","response":{"id":"resp_q9b"}}"#,
    "\n\nevent: response.output_item.added\n",
    r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","id":"msg_q9","content":[]}}"#,
    "\n\nevent: response.output_text.delta\n",
    r#"data: {"type":"response.output_text.delta","item_id":"msg_q9","output_index":0,"content_index":0,"delta":"Two tasks are open: Ship Q9 and Write the build log."}"#,
    "\n\nevent: response.output_item.done\n",
    r#"data: {"type":"response.output_item.done","output_index":0,"item":{"type":"message","role":"assistant","id":"msg_q9","content":[{"type":"output_text","text":"Two tasks are open: Ship Q9 and Write the build log."}]}}"#,
    "\n\nevent: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_q9b","usage":{"input_tokens":60,"output_tokens":12,"total_tokens":72}}}"#,
    "\n\n",
);

/// Codex's stand-in and its request bodies, with the home it must get.
fn codex_stand_in_folder(home: &Path) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("codex-stand-in.sh");
    std::fs::write(&script, CODEX_STAND_IN).unwrap();
    std::fs::write(dir.path().join("one.json"), CODEX_FIRST_CALL).unwrap();
    std::fs::write(dir.path().join("two.json"), CODEX_SECOND_CALL).unwrap();
    std::fs::write(
        dir.path().join("home.txt"),
        home.to_string_lossy().as_bytes(),
    )
    .unwrap();
    (dir, script)
}

/// The real Codex command for a self-hosted turn's `spec`, checked: the
/// served model, the provider, and no run key or credential anywhere on
/// argv or in the environment.
fn built_for_codex(spec: &agent::RunSpec, key: &str) -> Command {
    assert_eq!(spec.model.as_deref(), Some("qwen3.8-27b"));
    assert!(spec.extras.self_hosted.is_some(), "the turn has its target");
    // The fake credentials the stub turn's extra env carries are there to
    // be removed.
    assert!(spec.extras.env.iter().any(|(k, _)| k == "OPENAI_API_KEY"));
    let real = agent::command_without_codex_listing(AgentKind::Codex, spec);
    assert!(!agent::is_unavailable_command(&real), "the run may start");
    for (name, value) in real.as_std().get_envs() {
        if let Some(value) = value {
            assert!(
                !value.to_string_lossy().contains("fake-"),
                "{name:?} carries a credential"
            );
        }
    }
    let argv: Vec<String> = real
        .as_std()
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(argv
        .iter()
        .any(|a| a == r#"model_provider="hollow-self-hosted""#));
    assert!(
        !argv.iter().any(|a| a.contains(key)),
        "the key is never on argv"
    );
    real
}

/// Q9's verify: a Codex chat turn on an `openai` entry resolves it (Claude
/// refuses it with where to choose it), runs the real Codex command (stood
/// in for) through the forwarder in the entry's own Codex home, gets a
/// Hollow tool call and then the answer from the endpoint's Responses
/// stream, and completes through Hollow's own Codex parsing. The endpoint
/// never sees the run key, and the key is refused once the turn lets go.
#[tokio::test]
async fn a_self_hosted_codex_turn_calls_a_hollow_tool_and_answers() {
    let endpoint = codex_endpoint().await;
    save_entry_as(
        "q9-turn",
        &endpoint.base,
        crate::self_hosted_registry::API_OPENAI,
    );
    let why = crate::self_hosted::turn_route(AgentKind::Claude, Some("self-hosted:q9-turn"))
        .await
        .unwrap_err();
    assert!(why.contains("runs on Codex, not Claude"), "{why}");
    let run = crate::self_hosted::turn_route(AgentKind::Codex, Some("self-hosted:q9-turn"))
        .await
        .unwrap()
        .expect("a self-hosted turn");
    let forwarder = run.forwarder().clone();
    let key = run.target.run_key.as_str().to_string();
    let home = run.target.codex_home.clone().expect("its own Codex home");
    assert!(crate::self_hosted::is_self_hosted_codex_home(&home));

    let dir = chat_vault();
    let vault = dir.path();
    let session = "s-20261103-9c0d0001";
    let ord = conversation_with_prior_turn(vault, session);
    let (_bin, script) = codex_stand_in_folder(&home);
    let stop = StopSignal::new();
    let builds = std::cell::Cell::new(0);
    let attempted = {
        let _on = OnTarget::set(&run.target);
        drive_stub_agent(
            AgentKind::Codex,
            "run-q9-turn",
            vault,
            session,
            ord,
            &ResumePlan::New,
            &stop,
            |spec: &agent::RunSpec| {
                builds.set(builds.get() + 1);
                stand_in_as(&built_for_codex(spec, &key), &script, "codex")
            },
            &NoAccounts,
            None,
            None,
            None,
            &[],
        )
        .await
    };
    assert_eq!(builds.get(), 1);
    let AttemptsEnd::Driven {
        driven,
        stream,
        retried,
        ..
    } = attempted.end
    else {
        panic!("the turn ran");
    };
    assert!(
        matches!(driven.status, Some(Ok(ref status)) if status.success()),
        "{:?} {:?}",
        driven.status,
        driven.stderr_tail
    );
    assert!(!retried);
    assert_eq!(
        stream.last_session_id.as_deref(),
        Some("0199b9a0-9c0d-7e00-8000-00000000c0de")
    );
    assert!(
        stream.assistant.contains(CODEX_ANSWER),
        "{}",
        stream.assistant
    );
    let tools = attempted.sink.of_type("toolUse");
    assert_eq!(tools.len(), 1, "{}", attempted.sink.text());
    assert_eq!(tools[0]["name"], "mcp__hollow_tasks__list_tasks");
    assert_eq!(attempted.sink.of_type("toolResult").len(), 1);
    assert!(attempted.sink.text().contains(CODEX_ANSWER));

    // The endpoint got both Responses calls, without the run key.
    let calls = endpoint.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 2, "{calls:?}");
    for call in &calls {
        assert_eq!(call.path, "/v1/responses");
        assert!(!call.head.contains("hsr_"), "{}", call.head);
        assert!(!call.head.to_ascii_lowercase().contains("authorization"));
    }
    assert!(calls[1].body.contains("function_call_output"));
    // Q8 for Codex (Q9 review): billed by GPU time on the endpoint, under
    // the served id, never priced.
    let usage = attempted.ledger.finish(LedgerOutcome::Success);
    assert_eq!(usage.billing, crate::usage::Billing::SelfHosted);
    assert_eq!(usage.endpoint.as_deref(), Some("Qwen3.8 27B"));
    assert_eq!(usage.model.as_deref(), Some("qwen3.8-27b"));
    assert_eq!(usage.cost_usd, None);
    assert!(usage.tokens.is_some(), "tokens are kept");

    // The turn ends: its key goes with it.
    drop(run);
    assert_eq!(forwarder.live_keys(), 0);
    let refused = reqwest::Client::new()
        .post(format!("{}/v1/responses", forwarder.base_url()))
        .bearer_auth(&key)
        .body(CODEX_FIRST_CALL)
        .send()
        .await
        .map(|resp| resp.status().as_u16())
        .unwrap_or(0);
    assert_eq!(refused, 401);
    assert_eq!(endpoint.calls.lock().unwrap().len(), 2, "never forwarded");
    crate::self_hosted_forwarder::stop("q9-turn");
}

/// A stub Responses endpoint that answers a Hollow tool call the way Codex
/// asks for it: a `function_call` in the tool's `namespace` for the first
/// call, then the answer once the call's output comes back.
async fn codex_endpoint() -> Endpoint {
    endpoint_with(false, |body| {
        if body.contains("function_call_output") {
            CODEX_ANSWER_SSE
        } else {
            CODEX_TOOL_SSE
        }
    })
    .await
}

/// `real` started through `program` instead of its own binary, with every
/// argument mapped by `map`, the same environment (removals included) and
/// folder, leading its own group like a built command.
fn relaunch(real: &Command, program: &Path, map: impl Fn(String) -> String) -> Command {
    let mut cmd = crate::process_group::test_support::isolated_shell(
        &format!("exec '{}' \"$@\"", program.display()),
        &[],
    );
    cmd.arg("codex");
    for arg in real.as_std().get_args() {
        cmd.arg(map(arg.to_string_lossy().into_owned()));
    }
    for (name, value) in real.as_std().get_envs() {
        match value {
            Some(value) => {
                cmd.env(name, value);
            }
            None => {
                cmd.env_remove(name);
            }
        }
    }
    if let Some(dir) = real.as_std().get_current_dir() {
        cmd.current_dir(dir);
    }
    // As the built command: `codex exec` reads a stdin that isn't a TTY.
    cmd.stdin(Stdio::null()).stderr(Stdio::piped());
    cmd
}

/// Opt-in (`cargo build --bin hollow && cargo test --lib real_codex --
/// --ignored`): the installed `codex` itself runs a self-hosted turn
/// against a stub Responses endpoint through the real forwarder, in the
/// entry's own Codex home, with Hollow's `hollow_tasks` server (the built
/// `hollow` binary, read-only on the test vault) attached: it calls
/// `list_tasks`, sends the server's answer back and finishes with the
/// endpoint's reply. The endpoint never sees the run key, and Codex's
/// request carries its built-in tools and Hollow's namespace only, no
/// connector tools. Costs nothing: no call leaves this Mac. Skips (with a
/// line saying why) when either binary is missing.
#[tokio::test]
#[ignore = "needs the installed codex and the built hollow binary; cargo build --bin hollow, then cargo test --lib real_codex -- --ignored"]
async fn real_codex_calls_hollow_tasks_through_the_forwarder() {
    let Some(codex) = crate::agent::installed_binary(AgentKind::Codex) else {
        eprintln!("skipped: codex isn't installed where Hollow looks for it");
        return;
    };
    let exe = std::env::current_exe().unwrap();
    let Some(hollow) = exe
        .parent()
        .and_then(Path::parent)
        .map(|dir| dir.join("hollow"))
        .filter(|bin| bin.is_file())
    else {
        eprintln!("skipped: build the app binary first (cargo build --bin hollow)");
        return;
    };
    eprintln!("real codex: {}", codex.display());
    // A freshly linked `hollow` starts slowly the first time (about a
    // second on the Mac mini), and Codex 0.164.0 sends its first request
    // without waiting for an MCP server that isn't up, so the first run
    // after a build would miss Hollow's tools. One start first (`mcp-launch`
    // with no arguments prints its usage and exits).
    let _ = std::process::Command::new(&hollow)
        .arg(crate::run_auth::LAUNCH_SUBCOMMAND)
        .output();
    let endpoint = codex_endpoint().await;
    save_entry_as(
        "q9-real",
        &endpoint.base,
        crate::self_hosted_registry::API_OPENAI,
    );
    let run = crate::self_hosted::turn_route(AgentKind::Codex, Some("self-hosted:q9-real"))
        .await
        .unwrap()
        .expect("a self-hosted turn");
    let forwarder = run.forwarder().clone();
    let key = run.target.run_key.as_str().to_string();
    let home = run.target.codex_home.clone().expect("its own Codex home");

    let dir = chat_vault();
    let vault = dir.path();
    let session = "s-20261103-9c0d0002";
    let ord = conversation_with_prior_turn(vault, session);
    let parent = crate::usage_ledger::DelegateParent {
        surface: crate::usage_ledger::Surface::Chat,
        scope: session.into(),
        run_id: "run-q9-real".into(),
        scratch: None,
        tool_limits: Default::default(),
    };
    let (name, tasks) = crate::tasks_mcp::entry_for(
        &hollow,
        &vault.join("no-broker.sock"),
        vault,
        &parent,
        crate::tasks_mcp::TasksAccess::ReadOnly,
        "no-proof",
    );
    let mut servers = serde_json::Map::new();
    servers.insert(name.to_string(), tasks);
    let config = json!({ "mcpServers": servers }).to_string();
    let stop = StopSignal::new();
    let test_exe = exe.to_string_lossy().into_owned();
    let attempted = {
        let _on = OnTarget::set(&run.target);
        drive_stub_agent(
            AgentKind::Codex,
            "run-q9-real",
            vault,
            session,
            ord,
            &ResumePlan::New,
            &stop,
            |spec: &agent::RunSpec| {
                let mut spec = spec.clone();
                spec.mcp_config = Some(config.clone());
                let cmd = agent::build_command(AgentKind::Codex, &spec);
                assert!(
                    !agent::is_unavailable_command(&cmd),
                    "{:?}",
                    cmd.as_std().get_args().collect::<Vec<_>>()
                );
                let argv: Vec<String> = cmd
                    .as_std()
                    .get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                assert!(!argv.iter().any(|a| a.contains(&key)));
                // Codex starts Hollow's servers through `<this program>
                // mcp-launch`; under test this program is the test binary,
                // so the built app binary stands in for it.
                relaunch(&cmd, &codex, |arg| {
                    arg.replace(&test_exe, &hollow.to_string_lossy())
                })
            },
            &NoAccounts,
            None,
            None,
            None,
            &[],
        )
        .await
    };
    let AttemptsEnd::Driven { driven, stream, .. } = attempted.end else {
        panic!("the turn ran");
    };
    let calls = endpoint.calls.lock().unwrap().clone();
    assert!(
        matches!(driven.status, Some(Ok(ref status)) if status.success()),
        "{:?} {:?} calls: {calls:?}",
        driven.status,
        driven.stderr_tail
    );
    let tools = attempted.sink.of_type("toolUse");
    assert!(
        tools
            .iter()
            .any(|t| t["name"] == "mcp__hollow_tasks__list_tasks"),
        "{}\ncalls: {calls:?}\nstderr: {:?}",
        attempted.sink.text(),
        driven.stderr_tail
    );
    assert!(
        stream.assistant.contains(CODEX_ANSWER),
        "{}\nstderr: {:?}",
        stream.assistant,
        driven.stderr_tail
    );
    assert!(calls.iter().all(|c| c.path == "/v1/responses"), "{calls:?}");
    assert!(calls.iter().all(|c| !c.head.contains("hsr_")));
    // Only Codex's own tools and Hollow's namespace: no connector tools.
    let first: serde_json::Value = serde_json::from_str(&calls[0].body).unwrap();
    let names: Vec<String> = first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .unwrap_or(t["type"].as_str().unwrap_or(""))
                .to_string()
        })
        .collect();
    assert!(names.len() < 30, "{names:?}");
    assert!(
        names
            .iter()
            .filter(|n| n.starts_with("mcp__"))
            .all(|n| n == "mcp__hollow_tasks"),
        "{names:?}"
    );
    let answered = calls
        .iter()
        .find(|c| c.body.contains("function_call_output"))
        .expect("the tool's result went back to the endpoint");
    assert!(answered.body.contains("call_q9"));
    // The rollout is in the entry's own home.
    let sessions = home.join("sessions");
    assert!(sessions.is_dir(), "{}", sessions.display());
    let _ = attempted.ledger.finish(LedgerOutcome::Success);
    drop(run);
    assert_eq!(forwarder.live_keys(), 0);
    crate::self_hosted_forwarder::stop("q9-real");
}
