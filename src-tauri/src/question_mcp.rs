//! The `hollow_questions` MCP stdio server (Paperclip PR 13): one tool,
//! `ask_user_questions`, that lets the chat agent post a structured question
//! card instead of asking in prose.
//!
//! STATELESS by design (contract rule 12: a Hollow MCP subprocess cannot share
//! the GUI's locks or state). It knows neither the session nor the vault: it
//! validates the request with `interaction_schema::validate_request` and
//! answers `Posted N question(s) … End your turn now`, or a tool error with the
//! reason. The GUI records the card itself, from the COMPLETED structured tool
//! event the chat turn already streams, and validates again; nothing here is
//! trusted. The success text starts with `interaction_schema::POSTED_PREFIX`
//! so the GUI can tell a real success from the MCP's own rejection when the
//! transport's success flag cannot be trusted.
//!
//! Launched as the hidden `hollow questions-mcp` subcommand (see `main.rs`).
//! Attached to every chat turn whose agent supports owned MCP (Claude,
//! Codex), with no Settings toggle; never to workflows, the heartbeat or
//! teammate delegations, where no user can answer. Its tools are preapproved
//! (`agent::owned_preapprovals`) because a headless CLI never prompts.
//! Modeled on the `vault_mcp` JSON-RPC skeleton.

use crate::interaction_schema::{self, POSTED_PREFIX};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Default MCP protocol version echoed when a client doesn't send one.
const DEFAULT_PROTOCOL: &str = "2024-11-05";

/// The `hollow_questions` entry for an `--mcp-config` mcpServers map. No
/// environment: the server needs none (the GUI still injects its run tag).
pub fn server_entry() -> Option<(&'static str, Value)> {
    let exe = std::env::current_exe().ok()?;
    Some((
        interaction_schema::SERVER,
        json!({
            "command": exe.to_string_lossy(),
            "args": ["questions-mcp"]
        }),
    ))
}

/// Entry point for the `questions-mcp` subcommand. Blocks on a current-thread
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
            Err(_) => continue,
        };
        if let Some(resp) = handle(&req) {
            let mut s = resp.to_string();
            s.push('\n');
            if stdout.write_all(s.as_bytes()).await.is_err() {
                break;
            }
            let _ = stdout.flush().await;
        }
    }
}

/// Dispatch one JSON-RPC message: `Some(response)` for requests, `None` for
/// notifications.
pub(crate) fn handle(req: &Value) -> Option<Value> {
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
                    "serverInfo": { "name": "hollow-questions", "version": env!("CARGO_PKG_VERSION") },
                }),
            ))
        }
        "tools/list" => Some(result(
            id,
            json!({ "tools": [interaction_schema::tool_spec()] }),
        )),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            Some(call_tool(id, name, &args))
        }
        "ping" => Some(result(id, json!({}))),
        _ => id.map(|id| error(Some(id), -32601, &format!("method not found: {method}"))),
    }
}

fn call_tool(id: Option<Value>, name: &str, args: &Value) -> Value {
    if name != interaction_schema::TOOL {
        return tool_error(id, &format!("unknown tool: {name}"));
    }
    match interaction_schema::validate_request(args) {
        Ok(request) => {
            let n = request.question_count();
            let noun = if n == 1 { "question" } else { "questions" };
            tool_text(
                id,
                &format!(
                    "{POSTED_PREFIX}{n} {noun} to the user as a card. End your turn now: do not call more tools or continue this work. The answers will arrive as the user's next message, beginning \"{}\".",
                    interaction_schema::ANSWER_HEADER
                ),
            )
        }
        Err(reason) => tool_error(id, &reason),
    }
}

// ── JSON-RPC helpers (same shapes as vault_mcp / team_mcp / computer_mcp) ────

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
mod tests {
    use super::*;

    fn req(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn text_of(resp: &Value) -> String {
        resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn lists_one_read_only_tool_and_posts_or_errors() {
        let init = handle(&req(
            1,
            "initialize",
            json!({ "protocolVersion": "2025-06-18" }),
        ))
        .unwrap();
        assert_eq!(
            init["result"]["serverInfo"]["name"],
            json!("hollow-questions")
        );
        assert_eq!(init["result"]["protocolVersion"], json!("2025-06-18"));

        let list = handle(&req(2, "tools/list", Value::Null)).unwrap();
        let tools = list["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], json!("ask_user_questions"));
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], json!(true));
        assert!(tools[0]["description"].as_str().unwrap().contains("COO"));

        let posted = handle(&req(
            3,
            "tools/call",
            json!({ "name": "ask_user_questions", "arguments": {
                "questions": [{ "id": "layout", "prompt": "Which layout?", "options": [{ "id": "grid", "label": "Grid" }] }]
            }}),
        ))
        .unwrap();
        assert_eq!(posted["result"]["isError"], json!(false));
        let text = text_of(&posted);
        assert!(text.starts_with(POSTED_PREFIX), "{text}");
        assert!(text.starts_with("Posted 1 question to the user"), "{text}");
        assert!(text.contains("End your turn now"));
        assert!(text.contains("Answered questions"));

        let two = handle(&req(
            4,
            "tools/call",
            json!({ "name": "ask_user_questions", "arguments": {
                "questions": [
                    { "id": "a", "prompt": "A?", "options": [{ "id": "x", "label": "X" }] },
                    { "id": "b", "prompt": "B?", "answerMode": "text" }
                ]
            }}),
        ))
        .unwrap();
        assert!(text_of(&two).starts_with("Posted 2 questions"));

        let rejected = handle(&req(
            5,
            "tools/call",
            json!({ "name": "ask_user_questions", "arguments": { "questions": [] } }),
        ))
        .unwrap();
        assert_eq!(rejected["result"]["isError"], json!(true));
        assert!(
            !text_of(&rejected).starts_with(POSTED_PREFIX),
            "a rejection never looks posted"
        );

        let unknown = handle(&req(
            6,
            "tools/call",
            json!({ "name": "nope", "arguments": {} }),
        ))
        .unwrap();
        assert_eq!(unknown["result"]["isError"], json!(true));

        assert!(
            handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none()
        );
        let bad =
            handle(&json!({ "jsonrpc": "2.0", "id": 9, "method": "does/not/exist" })).unwrap();
        assert_eq!(bad["error"]["code"], json!(-32601));
    }

    /// The server is stateless: no environment, and none of the modules that
    /// hold Hollow's state. The needles are built with `concat!` so this test's
    /// own text never matches them (agent.rs's `OWNED_MCP_SOURCES` scans this
    /// file for the same substrings).
    #[test]
    fn server_entry_is_stateless_and_never_touches_the_store() {
        let (name, entry) = server_entry().expect("current exe");
        assert_eq!(name, "hollow_questions");
        assert_eq!(entry["args"], json!(["questions-mcp"]));
        assert!(entry.get("env").is_none(), "{entry}");
        let source = include_str!("question_mcp.rs");
        for needle in [
            concat!("session_", "store::"),
            concat!("interactions", "::"),
            concat!("run_", "queue::"),
            concat!("automated_", "history::"),
            concat!("HOLLOW_", "VAULT"),
            concat!("claude_", "quota::"),
        ] {
            assert!(
                !source.contains(needle),
                "question_mcp.rs must not contain `{needle}`"
            );
        }
    }
}
