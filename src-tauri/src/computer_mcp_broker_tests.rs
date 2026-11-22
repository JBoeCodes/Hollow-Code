//! `hollow_computer`'s see-and-act link (Cua CU1 part 2) against a fake
//! computer broker on a Unix socket: the seven verbs stay as they were and
//! are never forwarded; other names go to the broker; typed text never
//! reaches the vault log.

use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncBufReadExt;

const SEVEN: [&str; 7] = [
    "open_url",
    "search_bookmarks",
    "open_app",
    "list_apps",
    "browser_current_tab",
    "browser_list_tabs",
    "quit_app",
];

pub(crate) type Seen = Arc<Mutex<Vec<Value>>>;

/// A fake broker: lists `click` (and `open_url`, which must be dropped),
/// runs `click` and `type_text`, refuses everything else.
pub(crate) fn fake_broker(dir: &Path) -> (PathBuf, Seen) {
    let sock = dir.join("computer-test.sock");
    let listener = tokio::net::UnixListener::bind(&sock).unwrap();
    let seen: Seen = Arc::default();
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut line = String::new();
                if BufReader::new(read).read_line(&mut line).await.is_err() {
                    return;
                }
                let req: Value = serde_json::from_str(&line).unwrap();
                log.lock().unwrap().push(req.clone());
                let answer = match (req["op"].as_str(), req["tool"].as_str()) {
                    (Some("tools"), _) => json!({ "ok": true, "tools": [
                        { "name": "click", "description": "Click. (Hollow: act tier, in the sandbox Mac.)", "inputSchema": { "type": "object" } },
                        { "name": "open_url", "description": "a clash", "inputSchema": { "type": "object" } }
                    ] }),
                    (Some("call"), Some("click" | "type_text")) => {
                        json!({ "ok": true, "tier": "act", "result": {
                        "content": [{ "type": "text", "text": "clicked" }], "isError": false
                    } })
                    }
                    (_, tool) => {
                        json!({ "ok": false, "error": format!("{} is not offered to agents in Hollow; nothing ran.", tool.unwrap_or("")) })
                    }
                };
                let mut out = answer.to_string();
                out.push('\n');
                let _ = write.write_all(out.as_bytes()).await;
            });
        }
    });
    (sock, seen)
}

pub(crate) fn link(sock: PathBuf) -> BrokerLink {
    BrokerLink {
        sock,
        run_id: "run-1".into(),
        scope: "chat-1".into(),
        perm: "acceptEdits".into(),
        proof: crate::computer_broker::proof_for("run-1", "chat-1", "acceptEdits"),
    }
}

pub(crate) fn ctx(audit: Option<&Path>) -> Context {
    Context {
        bypass: false,
        audit_root: audit.map(Path::to_path_buf),
        approval: None,
    }
}

fn list_req() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })
}

fn call_req(name: &str, args: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": name, "arguments": args } })
}

fn tool_names(resp: &Value) -> Vec<String> {
    resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn text(resp: &Value) -> &str {
    resp["result"]["content"][0]["text"].as_str().unwrap()
}

#[test]
fn the_seven_verbs_are_unchanged_and_named_as_the_broker_skips_them() {
    let names: Vec<String> = tool_specs(false)
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, SEVEN);
    assert_eq!(crate::computer_broker::NATIVE_VERBS, SEVEN);
}

#[tokio::test]
async fn without_a_link_only_the_seven_verbs_exist() {
    let resp = handle_in(&list_req(), &ctx(None)).await.unwrap();
    assert_eq!(tool_names(&resp), SEVEN);
    let resp = handle_in(&call_req("click", json!({})), &ctx(None))
        .await
        .unwrap();
    assert_eq!(resp["result"]["isError"], json!(true));
    assert!(text(&resp).contains("unknown tool"));
}

#[tokio::test]
async fn the_broker_lists_after_the_verbs_and_runs_other_names() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, seen) = fake_broker(dir.path());
    let link = link(sock);
    let vault = tempfile::tempdir().unwrap();
    let ctx = ctx(Some(vault.path()));

    let resp = handle_with(&list_req(), &ctx, Some(&link)).await.unwrap();
    let mut want: Vec<&str> = SEVEN.to_vec();
    want.push("click");
    assert_eq!(
        tool_names(&resp),
        want,
        "a broker tool named like a verb is dropped"
    );

    let resp = handle_with(&call_req("click", json!({ "x": 1 })), &ctx, Some(&link))
        .await
        .unwrap();
    assert_eq!(resp["result"]["isError"], json!(false));
    assert_eq!(text(&resp), "clicked");
    let sent = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(sent["op"], "call");
    assert_eq!(sent["tool"], "click");
    assert_eq!(sent["arguments"], json!({ "x": 1 }));
    assert_eq!(sent["run_id"], "run-1");
    assert_eq!(sent["perm"], "acceptEdits");
    assert_eq!(sent["proof"], json!(link.proof));

    let resp = handle_with(&call_req("set_config", json!({})), &ctx, Some(&link))
        .await
        .unwrap();
    assert_eq!(resp["result"]["isError"], json!(true));
    assert!(text(&resp).contains("not offered"), "{resp}");

    // A native verb never goes to the broker, even with a link.
    let before = seen.lock().unwrap().len();
    let resp = handle_with(
        &call_req("open_url", json!({ "url": "file:///etc/passwd" })),
        &ctx,
        Some(&link),
    )
    .await
    .unwrap();
    assert_eq!(resp["result"]["isError"], json!(true));
    assert_eq!(seen.lock().unwrap().len(), before);

    // Typed text is logged as a digest only.
    handle_with(
        &call_req("type_text", json!({ "text": "hunter2-secret" })),
        &ctx,
        Some(&link),
    )
    .await
    .unwrap();
    let log = std::fs::read_to_string(vault.path().join(".computer-actions.jsonl")).unwrap();
    assert!(log.contains("\"tool\":\"type_text\""), "{log}");
    assert!(log.contains("\"keys\":[\"text\"]"), "{log}");
    assert!(
        !log.contains("not offered"),
        "the broker's words are not logged: {log}"
    );
    assert!(!log.contains("hunter2-secret"), "{log}");
    assert!(!log.contains("clicked"), "results are not logged");
}

#[tokio::test]
async fn an_unreachable_broker_lists_the_verbs_and_refuses_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let link = link(dir.path().join("gone.sock"));
    let resp = handle_with(&list_req(), &ctx(None), Some(&link))
        .await
        .unwrap();
    assert_eq!(tool_names(&resp), SEVEN);
    let resp = handle_with(&call_req("click", json!({})), &ctx(None), Some(&link))
        .await
        .unwrap();
    assert_eq!(resp["result"]["isError"], json!(true));
    assert!(text(&resp).contains("isn't reachable"), "{resp}");
}

#[test]
fn the_entry_carries_the_link_only_with_a_broker_and_never_in_plan() {
    let _toggles = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_computer_control_enabled(true);
    let vault = PathBuf::from("/tmp/vault");
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            crate::computer_broker::set_socket_path_for_test(None);
        }
    }
    let _reset = Reset;
    crate::computer_broker::set_socket_path_for_test(None);
    let (_, entry) = server_entry(&vault, "acceptEdits", "run-1", "chat-1").unwrap();
    assert!(entry["env"].get(ENV_BROKER_SOCK).is_none());

    crate::computer_broker::set_socket_path_for_test(Some(PathBuf::from("/tmp/computer-1.sock")));
    let (_, entry) = server_entry(&vault, "acceptEdits", "run-1", "chat-1").unwrap();
    let env = &entry["env"];
    assert_eq!(env[ENV_BROKER_SOCK], "/tmp/computer-1.sock");
    assert_eq!(env["HOLLOW_RUN_ID"], "run-1", "the key run_auth registers");
    assert_eq!(env[ENV_BROKER_SCOPE], "chat-1");
    assert_eq!(env[ENV_BROKER_PERM], "acceptEdits");
    assert_eq!(
        env[ENV_BROKER_PROOF],
        json!(crate::computer_broker::proof_for(
            "run-1",
            "chat-1",
            "acceptEdits"
        ))
    );
    let (_, plan) = server_entry(&vault, "plan", "run-1", "chat-1").unwrap();
    assert!(
        plan["env"].get(ENV_BROKER_SOCK).is_none(),
        "plan gets no link"
    );
    let (_, anon) = server_entry(&vault, "acceptEdits", " ", "chat-1").unwrap();
    assert!(anon["env"].get(ENV_BROKER_PROOF).is_none());
}
