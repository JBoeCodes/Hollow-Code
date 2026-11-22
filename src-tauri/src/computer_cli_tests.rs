//! `hollow computer` (Cua CU4): the same result as the MCP server for the
//! same call, images to files, exit codes, and the agent picker's note.

use super::*;
use crate::computer_mcp::broker_tests::{ctx, fake_broker, link};

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn a_shell_call_returns_what_the_mcp_server_returns() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, seen) = fake_broker(dir.path());
    let link = link(sock);
    let ctx = ctx(None);
    let out = tempfile::tempdir().unwrap();

    let mcp = handle_with(
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                 "params": { "name": "click", "arguments": { "x": 4 } } }),
        &ctx,
        Some(&link),
    )
    .await
    .unwrap();
    let want = mcp["result"]["content"][0]["text"].as_str().unwrap();

    let (text, code) = cli(
        &args(&["call", "click", r#"{"x":4}"#]),
        &ctx,
        Some(&link),
        out.path(),
    )
    .await;
    assert_eq!((text.trim_end(), code), (want, 0));
    // Both went to the broker with the run's identity and the same arguments.
    {
        let seen = seen.lock().unwrap();
        let calls: Vec<&Value> = seen.iter().filter(|r| r["op"] == "call").collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["arguments"], calls[1]["arguments"]);
        assert_eq!(calls[0]["proof"], calls[1]["proof"]);
    }

    let (listed, code) = cli(&args(&["tools"]), &ctx, Some(&link), out.path()).await;
    assert_eq!(code, 0);
    assert!(
        listed.contains("open_url:") && listed.contains("click:"),
        "{listed}"
    );
}

#[tokio::test]
async fn refusals_exit_nonzero_and_bad_input_exits_two() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, _) = fake_broker(dir.path());
    let link = link(sock);
    let ctx = ctx(None);
    let out = tempfile::tempdir().unwrap();
    let (text, code) = cli(&args(&["call", "hotkey"]), &ctx, Some(&link), out.path()).await;
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("nothing ran"), "{text}");
    for bad in [
        args(&["call"]),
        args(&["call", "click", "[1]"]),
        args(&["call", "click", "not json"]),
        args(&["--frobnicate"]),
    ] {
        assert_eq!(
            cli(&bad, &ctx, Some(&link), out.path()).await.1,
            2,
            "{bad:?}"
        );
    }
    assert_eq!(cli(&[], &ctx, Some(&link), out.path()).await.1, 0, "help");
    // No run environment: nothing runs, not even a native verb.
    for verb in ["click", "quit_app", "open_url"] {
        let (text, code) = cli(&args(&["call", verb]), &ctx, None, out.path()).await;
        assert_eq!(code, 1, "{verb}");
        assert!(text.contains("no desktop link"), "{text}");
    }
    assert_eq!(cli(&args(&["tools"]), &ctx, None, out.path()).await.1, 1);
    // Help needs no link.
    assert_eq!(cli(&args(&["help"]), &ctx, None, out.path()).await.1, 0);
}

#[test]
fn an_image_is_saved_to_a_file_and_its_path_printed() {
    use base64::Engine;
    let dir = tempfile::tempdir().unwrap();
    let png = base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]);
    let resp = json!({ "result": { "isError": false, "content": [
        { "type": "text", "text": "a window" },
        { "type": "image", "data": png, "mimeType": "image/png" },
        { "type": "image", "data": "!!not base64!!" },
    ] } });
    let (text, code) = render(&resp, dir.path());
    assert_eq!(code, 0);
    assert!(text.starts_with("a window\n[image saved to "), "{text}");
    assert!(text.contains("could not be read"));
    let saved: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(saved.len(), 1);
    assert_eq!(
        std::fs::read(saved[0].as_ref().unwrap().path()).unwrap(),
        [1, 2, 3]
    );
}

#[test]
fn the_shell_context_is_never_bypass_whatever_the_environment_says() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // The toggles lock serialises tests that touch this variable.
    std::env::set_var(crate::computer_mcp::ENV_BYPASS, "1");
    let from_env = crate::computer_mcp::Context::from_env().bypass;
    let for_cli = crate::computer_mcp::Context::for_cli().bypass;
    std::env::remove_var(crate::computer_mcp::ENV_BYPASS);
    assert!(from_env, "the MCP server's context reads Hollow's flag");
    assert!(
        !for_cli,
        "the shell's never does: an agent sets its own env"
    );
}

#[test]
fn the_picker_note_names_who_cannot_use_the_desktop_yet() {
    assert!(desktop_unavailable(AgentKind::Claude).is_none());
    assert!(desktop_unavailable(AgentKind::Codex).is_none());
    for kind in [AgentKind::Grok, AgentKind::Antigravity] {
        let why = desktop_unavailable(kind).unwrap();
        assert!(why.contains("Use Claude or Codex"), "{kind:?}");
        let caps = crate::agent::capabilities(kind);
        assert_eq!(caps.desktop_unavailable.as_deref(), Some(why));
    }
    assert!(crate::agent::capabilities(AgentKind::Claude)
        .desktop_unavailable
        .is_none());
}

#[test]
fn the_run_environment_is_the_servers_and_empty_in_plan() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::computer_broker::set_socket_path_for_test(Some(PathBuf::from("/tmp/computer-1.sock")));
    let env = crate::computer_mcp::broker_env("acceptEdits", "run-1", "chat-1");
    let keys: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys.len(), 5);
    assert!(keys.contains(&crate::computer_mcp::ENV_BROKER_SOCK));
    assert!(crate::computer_mcp::broker_env("plan", "run-1", "chat-1").is_empty());
    assert!(crate::computer_mcp::broker_env("acceptEdits", " ", "chat-1").is_empty());
    crate::computer_broker::set_socket_path_for_test(None);
    assert!(crate::computer_mcp::broker_env("acceptEdits", "run-1", "chat-1").is_empty());
    assert!(CLI_BRIEF.contains("hollow computer call"));
}

#[tokio::test]
async fn the_verbs_are_the_brokers_tools_and_a_tool_name_is_a_shorthand_for_call() {
    let dir = tempfile::tempdir().unwrap();
    let (sock, seen) = fake_broker(dir.path());
    let link = link(sock);
    let ctx = ctx(None);
    let out = tempfile::tempdir().unwrap();

    // Parity: the CLI's list is the MCP server's list, name for name.
    let mcp = handle_with(
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        &ctx,
        Some(&link),
    )
    .await
    .unwrap();
    let want: Vec<String> = mcp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    let (listed, _) = cli(&args(&["tools"]), &ctx, Some(&link), out.path()).await;
    let got: Vec<&str> = listed
        .lines()
        .filter_map(|l| l.split_once(':').map(|(n, _)| n))
        .collect();
    assert_eq!(got, want);
    // No tool is hidden behind a subcommand's name.
    for name in &want {
        assert!(
            !RESERVED.contains(&name.as_str()),
            "{name} would shadow a subcommand"
        );
    }
    // status says the link and the count.
    let (status, code) = cli(&args(&["status"]), &ctx, Some(&link), out.path()).await;
    assert_eq!(code, 0);
    assert!(
        status.contains(&format!("tools ({})", want.len())),
        "{status}"
    );
    assert_eq!(cli(&args(&["status"]), &ctx, None, out.path()).await.1, 1);

    // Each listed tool is callable by its own name, the same as `call`.
    let (a, ca) = cli(
        &args(&["click", r#"{"x":2}"#]),
        &ctx,
        Some(&link),
        out.path(),
    )
    .await;
    let (b, cb) = cli(
        &args(&["call", "click", r#"{"x":2}"#]),
        &ctx,
        Some(&link),
        out.path(),
    )
    .await;
    assert_eq!((a, ca), (b, cb));
    let sent = seen.lock().unwrap();
    let calls: Vec<&Value> = sent.iter().filter(|r| r["op"] == "call").collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["arguments"], calls[1]["arguments"]);
    assert_eq!(calls[0]["tool"], "click");
}
