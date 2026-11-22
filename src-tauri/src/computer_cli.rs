//! The shell path to computer control (Cua CU4): `hollow computer`, a hidden
//! subcommand for an agent with no per-run MCP flag (Grok, Antigravity).
//!
//! Design: `outputs/cua-computer-use-design-2026-11-15.md`, "Providers". It is
//! the same server code as `hollow computer-mcp`, driven from a command line:
//! each call builds the JSON-RPC message the MCP server would get and runs it
//! through `computer_mcp::handle_with`, so a call returns the same result for
//! Claude, Codex or a shell, and the broker's cards, modes, limits and audit
//! rows are identical (it is the same socket, with the same run proof).
//!
//! `hollow computer tools` lists what this run may call; `hollow computer
//! call <tool> '<json>'` runs one. The run's identity comes from the
//! environment ([`crate::computer_mcp::broker_env`]): the socket, run, scope,
//! mode and proof. An agent whose sandbox cannot reach the socket gets an
//! answer saying so and nothing runs. Until a test turn shows an agent can
//! reach it, Hollow does not put that environment in its runs
//! ([`desktop_unavailable`]): the paid probe is Alex's call.

use crate::agent::AgentKind;
use crate::computer_mcp::{handle_with, BrokerLink, Context};
use serde_json::{json, Value};
use std::path::PathBuf;

/// The agent's brief for the shell path (what exists and how to call it).
#[allow(dead_code)] // put in a Grok or Antigravity run once a probe shows its sandbox reaches the socket
pub(crate) const CLI_BRIEF: &str = "Desktop tools (see and act) are on the command line: `hollow computer tools` lists what this chat may call, and `hollow computer call <tool> '<json arguments>'` runs one. Nothing else drives the desktop; the sandbox Mac is the default target, and the person is asked on a card where the mode says so.";

/// Why an agent can't use the desktop tools in a chat, or `None` when it can.
/// Claude and Codex have the owned MCP path. Grok and Antigravity would use
/// this module's command, but whether their sandboxes let them reach the
/// broker's socket has not been tried (it takes a paid test turn), so Hollow
/// gives them nothing rather than a path it has not seen work. The agent
/// picker (CU8) shows this beside the agent.
pub(crate) fn desktop_unavailable(kind: AgentKind) -> Option<&'static str> {
    match kind {
        AgentKind::Claude | AgentKind::Codex => None,
        _ => Some("Hollow hasn't confirmed that this agent's sandbox can reach the desktop tools (it takes one test turn). Use Claude or Codex for desktop work."),
    }
}

const NO_LINK: &str = "This run has no desktop link (no HOLLOW_COMPUTER_* environment: the chat is in plan mode, Computer control is off, or Hollow hasn't attached it to this agent); nothing ran.\n";

fn usage() -> String {
    "usage: hollow computer tools\n       hollow computer status\n       hollow computer call <tool> '<json arguments>'\n       hollow computer <tool> '<json arguments>'   (shorthand for call)\nRuns the same tools as the hollow_computer MCP server, through Hollow's broker, in this run's permission mode. It needs the run's HOLLOW_COMPUTER_* environment; without it nothing runs.\n".to_string()
}

/// Render a tool result for a terminal: text as is, an image saved to a PNG
/// whose path is printed. The second value is the exit code (1 for an error).
fn render(response: &Value, save_dir: &std::path::Path) -> (String, i32) {
    if let Some(msg) = response.pointer("/error/message").and_then(Value::as_str) {
        return (format!("{msg}\n"), 1);
    }
    let result = response.get("result").unwrap_or(&Value::Null);
    let mut out = String::new();
    for (n, block) in result
        .get("content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                out.push_str(block.get("text").and_then(Value::as_str).unwrap_or(""));
                out.push('\n');
            }
            Some("image") => out.push_str(&save_image(block, n, save_dir)),
            _ => {}
        }
    }
    let failed = result.get("isError").and_then(Value::as_bool) == Some(true);
    (out, i32::from(failed))
}

fn save_image(block: &Value, n: usize, dir: &std::path::Path) -> String {
    use base64::Engine;
    let bytes = block
        .get("data")
        .and_then(Value::as_str)
        .and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok());
    let Some(bytes) = bytes else {
        return "[an image that could not be read]\n".into();
    };
    // A new file only (`create_new` refuses an existing path or a symlink),
    // with a name that can't be guessed ahead.
    use std::io::Write;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = dir.join(format!(
        "hollow-computer-{}-{nanos:x}-{n}.png",
        std::process::id()
    ));
    let saved = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut f| f.write_all(&bytes));
    match saved {
        Ok(()) => format!("[image saved to {}]\n", path.display()),
        Err(_) => "[an image that could not be saved]\n".into(),
    }
}

/// Words that are subcommands, never tools: a tool of one of these names
/// would be unreachable by its shorthand (a test keeps the broker's tools off
/// this list).
pub(crate) const RESERVED: &[&str] = &["tools", "call", "status", "help"];

/// The tools this run may call, with their descriptions: what the MCP server
/// lists, so the CLI's verbs are generated from the broker's own list (CD5).
async fn listed(ctx: &Context, link: Option<&BrokerLink>) -> Vec<(String, String)> {
    let req = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
    handle_with(&req, ctx, link)
        .await
        .and_then(|resp| {
            resp.pointer("/result/tools")
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default()
        .iter()
        .map(|t| {
            (
                t.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                t.get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

async fn run_tool(
    tool: &str,
    raw: Option<&String>,
    ctx: &Context,
    link: Option<&BrokerLink>,
    save_dir: &std::path::Path,
) -> (String, i32) {
    let arguments = match raw.map(|a| a.trim()).filter(|a| !a.is_empty()) {
        None => json!({}),
        Some(raw) => match serde_json::from_str::<Value>(raw) {
            Ok(v) if v.is_object() => v,
            _ => return ("the arguments must be one JSON object\n".into(), 2),
        },
    };
    let req = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": tool, "arguments": arguments },
    });
    match handle_with(&req, ctx, link).await {
        Some(resp) => render(&resp, save_dir),
        None => ("no answer\n".into(), 1),
    }
}

/// One command. `args` are what follows `hollow computer`: `tools`, `status`,
/// `call <tool> '<json>'`, or a tool's own name as a shorthand for `call`
/// (`hollow computer click '{"x":1}'`).
pub(crate) async fn cli(
    args: &[String],
    ctx: &Context,
    link: Option<&BrokerLink>,
    save_dir: &std::path::Path,
) -> (String, i32) {
    let first = args.first().map(String::as_str);
    let shorthand = first.filter(|f| !RESERVED.contains(f) && !f.starts_with('-') && !f.is_empty());
    if (matches!(first, Some("tools" | "call" | "status")) || shorthand.is_some()) && link.is_none()
    {
        // No run identity, no desktop: the native verbs are not a back door
        // (they would run with no run, mode or proof), and plan runs get no
        // link at all.
        return (NO_LINK.to_string(), 1);
    }
    match first {
        None | Some("--help") | Some("-h") | Some("help") => (usage(), 0),
        Some("tools") => {
            let mut out = String::new();
            for (name, desc) in listed(ctx, link).await {
                out.push_str(&format!("{name}: {desc}\n"));
            }
            (out, 0)
        }
        Some("status") => {
            let names: Vec<String> = listed(ctx, link)
                .await
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            (
                format!(
                    "desktop link: ready\ntools ({}): {}\n",
                    names.len(),
                    names.join(", ")
                ),
                0,
            )
        }
        Some("call") => match args.get(1).map(|t| t.trim()).filter(|t| !t.is_empty()) {
            Some(tool) => run_tool(tool, args.get(2), ctx, link, save_dir).await,
            None => (usage(), 2),
        },
        Some(tool) if shorthand.is_some() => run_tool(tool, args.get(1), ctx, link, save_dir).await,
        Some(_) => (usage(), 2),
    }
}

/// Entry point for the `computer` subcommand.
pub fn run_cli() {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime");
    let ctx = Context::for_cli();
    let link = BrokerLink::from_env();
    let dir = std::env::var("HOLLOW_SCRATCH_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let (out, code) = rt.block_on(cli(&args, &ctx, link.as_ref(), &dir));
    print!("{out}");
    std::process::exit(code);
}

#[cfg(test)]
#[path = "computer_cli_tests.rs"]
mod tests;
