//! The `hollow_services` MCP stdio server (Hollow build programme, PR B6):
//! long-running processes (dev servers, watchers) that outlive the chat turn
//! that started them. Tools: `start_service`, `list_services`,
//! `stop_service`, `service_logs`.
//!
//! **This process starts, signals and waits for nothing itself** (the rule
//! for every Hollow MCP subprocess: the GUI owns process lifetimes). Every
//! call is forwarded as one JSON line over the per-pid Unix socket named by
//! `HOLLOW_SERVICES_SOCK` to the GUI's service broker (`services.rs`), which
//! checks the GUI-minted `HOLLOW_SERVICES_PROOF` over the run's [`Identity`]:
//! the scope (`HOLLOW_SERVICES_SCOPE`, the build worktree the chat runs in),
//! the turn's history run id (`HOLLOW_SERVICES_RUN`), the chat's session key
//! (`HOLLOW_SERVICES_CHAT`) and its permission mode
//! (`HOLLOW_SERVICES_PERM`). The proof expires when that run ends. The broker
//! validates the arguments and does the work. None of these names contain
//! KEY, SECRET or TOKEN, which Codex strips from an MCP server's environment.
//!
//! Attached by `mcp_attach` to Claude and Codex chats whose working folder is
//! a live build worktree, never in `plan` mode, never to automations.
//! Preapproved in `auto` and `bypassPermissions` only
//! (`agent::OWNED_MODE_PREAPPROVED`); elsewhere a headless CLI denies the
//! call. Merged plan decision 1 (2026-10-02): in every mode but bypass a
//! service is **bounded** (its folder resolves inside the worktree, it stops
//! when its chat is archived, on an explicit stop, when the worktree is
//! removed or when Hollow quits, and its command and folder go on the
//! starting turn's receipt). Only bypass starts an unrestricted service, one
//! not tied to its chat (`bounded` in `services.rs`).
//!
//! Launched as the hidden `hollow services-mcp` subcommand (see `main.rs`).

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The server key in an mcpServers map.
pub const SERVER_NAME: &str = "hollow_services";
pub const ENV_SOCK: &str = "HOLLOW_SERVICES_SOCK";
pub const ENV_SCOPE: &str = "HOLLOW_SERVICES_SCOPE";
pub const ENV_PROOF: &str = "HOLLOW_SERVICES_PROOF";
pub const ENV_RUN: &str = "HOLLOW_SERVICES_RUN";
pub const ENV_CHAT: &str = "HOLLOW_SERVICES_CHAT";
pub const ENV_PERM: &str = "HOLLOW_SERVICES_PERM";

/// Who a services server acts for, every part covered by its proof: the
/// build worktree (`scope`, canonical), the turn's history run id, the
/// chat's session key and the run's canonical permission mode.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Identity {
    pub scope: String,
    pub run_id: String,
    pub chat: String,
    pub perm: String,
}

const DEFAULT_PROTOCOL: &str = "2024-11-05";
/// How long a call waits for the GUI's answer (a stop can take ~10 s).
const BROKER_TIMEOUT_SECS: u64 = 30;

/// Chat guidance when the server is attached. Byte-stable and path-free: it
/// is part of the appended system prompt the resume fingerprint hashes.
pub const PROMPT_GUIDANCE: &str = "Services (hollow_services): Hollow ends every process a turn started when the turn finishes. To leave a dev server, watcher or other long-running command running for the user, start it with start_service (a short name, the command, and the port it listens on) instead of running it in the background with & in a shell; it keeps running after your turn, in a folder inside this worktree, until someone stops it (outside Bypass mode it also stops when this conversation is archived). list_services shows what is running, service_logs reads a service's recent output, and stop_service stops one. Only one dev server can run at a time on this machine; stop the old one first. Tell the user the name and port of anything you start.";

/// The `hollow_services` entry for a chat's mcpServers map (pure builder;
/// the GUI broker supplies the socket and proof).
pub(crate) fn entry_for(
    exe: &Path,
    sock: &Path,
    id: &Identity,
    proof: &str,
) -> (&'static str, Value) {
    (
        SERVER_NAME,
        json!({
            "command": exe.to_string_lossy(),
            "args": ["services-mcp"],
            "env": {
                ENV_SOCK: sock.to_string_lossy(),
                ENV_SCOPE: id.scope,
                ENV_RUN: id.run_id,
                ENV_CHAT: id.chat,
                ENV_PERM: id.perm,
                ENV_PROOF: proof,
            },
        }),
    )
}

/// Forwards one call to the GUI's broker. Tests plug in an in-process one.
pub(crate) trait Broker {
    fn call(&self, op: &str, args: &Value) -> Result<Value, String>;
}

#[cfg(unix)]
pub(crate) struct SocketBroker {
    pub path: PathBuf,
    pub id: Identity,
    pub proof: String,
}

/// The one-line request the broker reads (version 2 since merged plan
/// decision 1 added the run, chat and mode).
pub(crate) fn request(op: &str, id: &Identity, proof: &str, args: &Value) -> Value {
    json!({ "v": 2, "op": op, "scope": id.scope, "runId": id.run_id, "chat": id.chat, "perm": id.perm, "proof": proof, "args": args })
}

#[cfg(unix)]
impl Broker for SocketBroker {
    fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;
        use std::time::Duration;
        let mut stream = UnixStream::connect(&self.path).map_err(|e| {
            format!("Hollow's service broker is unavailable ({e}). Hollow may have quit.")
        })?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(BROKER_TIMEOUT_SECS)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let mut line = request(op, &self.id, &self.proof, args).to_string();
        line.push('\n');
        stream
            .write_all(line.as_bytes())
            .and_then(|_| stream.flush())
            .map_err(|e| format!("Could not reach Hollow's service broker ({e})."))?;
        let mut answer = String::new();
        match BufReader::new(stream).read_line(&mut answer) {
            Ok(0) => Err("Hollow closed the connection before answering; call list_services to see what happened.".into()),
            Ok(_) => parse_answer(&answer),
            Err(e) => Err(format!("Hollow did not answer ({e}); call list_services to see what happened.")),
        }
    }
}

/// `{ok: true, result}` → the result; `{ok: false, error}` → the error.
pub(crate) fn parse_answer(line: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("Hollow's service broker sent an unreadable answer ({e})."))?;
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    } else {
        Err(value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Hollow refused the request.")
            .to_string())
    }
}

/// Entry point for `hollow services-mcp`: a JSON-RPC loop over stdio until
/// stdin closes.
pub fn serve() {
    use std::io::{BufRead, Write};
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    #[cfg(unix)]
    let socket = match (
        var(ENV_SOCK),
        var(ENV_SCOPE),
        var(ENV_RUN),
        var(ENV_CHAT),
        var(ENV_PERM),
        var(ENV_PROOF),
    ) {
        (Some(sock), Some(scope), Some(run_id), Some(chat), Some(perm), Some(proof)) => {
            Some(SocketBroker {
                path: PathBuf::from(sock),
                id: Identity {
                    scope,
                    run_id,
                    chat,
                    perm,
                },
                proof,
            })
        }
        _ => None,
    };
    #[cfg(unix)]
    let broker: Option<&dyn Broker> = socket.as_ref().map(|b| b as &dyn Broker);
    #[cfg(not(unix))]
    let broker: Option<&dyn Broker> = None;
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
        if let Some(resp) = handle(&req, broker) {
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

pub(crate) const TOOLS: [&str; 4] = [
    "start_service",
    "list_services",
    "stop_service",
    "service_logs",
];

/// Dispatch one JSON-RPC message: `Some(response)` for requests.
pub(crate) fn handle(req: &Value, broker: Option<&dyn Broker>) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    match method {
        "initialize" => {
            let pv = req.get("params").and_then(|p| p.get("protocolVersion")).and_then(Value::as_str).unwrap_or(DEFAULT_PROTOCOL);
            Some(result(id, json!({
                "protocolVersion": pv,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "hollow-services", "version": env!("CARGO_PKG_VERSION") },
            })))
        }
        "tools/list" => Some(result(id, json!({ "tools": tool_specs() }))),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or(Value::Null);
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let outcome = if !TOOLS.contains(&name) {
                Err(format!("unknown tool: {name}"))
            } else if !args.is_object() {
                Err(format!("{name}: arguments must be an object."))
            } else {
                match broker {
                    Some(b) => b.call(name, &args),
                    None => Err("Services are unavailable in this run: Hollow's service broker isn't connected.".into()),
                }
            };
            Some(match outcome {
                Ok(value) => tool_result(id, &serde_json::to_string_pretty(&value).unwrap_or_default(), false),
                Err(e) => tool_result(id, &e, true),
            })
        }
        "ping" => Some(result(id, json!({}))),
        _ => id.map(|id| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {method}") } })),
    }
}

fn tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "start_service",
            "description": "Start a long-running command (a dev server, a watcher) that keeps running after this turn ends, in a folder inside this worktree. Outside Bypass mode it stops when this conversation is archived. Use this instead of running it in the background with & (Hollow ends background processes when a turn finishes). Its output goes to a log you can read with service_logs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short name: letters, digits, - and _ (e.g. web, api, watch)." },
                    "command": { "type": "string", "description": "The shell command, e.g. `npm run dev -- --port 5173`." },
                    "cwd": { "type": "string", "description": "Folder to run in, relative to the worktree (default: the worktree)." },
                    "port": { "type": "integer", "description": "The port it listens on, so the user gets an Open link." },
                    "kind": { "type": "string", "enum": ["dev-server", "other"], "description": "dev-server (only one may run on this machine at a time) or other. Default other." }
                },
                "required": ["name", "command"]
            }
        }),
        json!({
            "name": "list_services",
            "description": "The services started in this worktree: name, command, port, running or exited.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "stop_service",
            "description": "Stop a service (and everything it started).",
            "inputSchema": { "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }
        }),
        json!({
            "name": "service_logs",
            "description": "The newest lines of a service's output.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string" }, "lines": { "type": "integer", "description": "How many lines (default 80, at most 400)." } },
                "required": ["name"]
            }
        }),
    ]
}

fn result(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn tool_result(id: Option<Value>, text: &str, is_error: bool) -> Value {
    result(
        id,
        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Recorder(RefCell<Vec<(String, Value)>>);
    impl Broker for Recorder {
        fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
            self.0.borrow_mut().push((op.into(), args.clone()));
            if op == "stop_service" {
                Err("no such service".into())
            } else {
                Ok(json!({ "ok": op }))
            }
        }
    }

    fn call(name: &str, args: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } })
    }

    #[test]
    fn every_call_is_forwarded_and_nothing_runs_here() {
        let rec = Recorder(RefCell::new(vec![]));
        let list = handle(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
            Some(&rec),
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, TOOLS);
        let ok = handle(
            &call(
                "start_service",
                json!({ "name": "web", "command": "npm run dev" }),
            ),
            Some(&rec),
        )
        .unwrap();
        assert_eq!(ok["result"]["isError"], json!(false));
        let err = handle(&call("stop_service", json!({ "name": "x" })), Some(&rec)).unwrap();
        assert_eq!(err["result"]["isError"], json!(true));
        assert_eq!(
            err["result"]["content"][0]["text"],
            json!("no such service")
        );
        assert!(
            handle(&call("rm_rf", json!({})), Some(&rec)).unwrap()["result"]["isError"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(
            rec.0
                .borrow()
                .iter()
                .map(|(o, _)| o.as_str())
                .collect::<Vec<_>>(),
            ["start_service", "stop_service"]
        );
        let none = handle(&call("list_services", json!({})), None).unwrap();
        assert!(none["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("isn't connected"));
    }

    #[test]
    fn entries_and_answers() {
        let id = Identity {
            scope: "/w".into(),
            run_id: "R1".into(),
            chat: "s-1".into(),
            perm: "auto".into(),
        };
        let (name, entry) = entry_for(
            Path::new("/A/hollow"),
            Path::new("/c/services-1.sock"),
            &id,
            "p",
        );
        assert_eq!(name, "hollow_services");
        assert_eq!(entry["args"], json!(["services-mcp"]));
        assert_eq!(entry["env"][ENV_SCOPE], json!("/w"));
        assert_eq!(
            (
                &entry["env"][ENV_RUN],
                &entry["env"][ENV_CHAT],
                &entry["env"][ENV_PERM]
            ),
            (&json!("R1"), &json!("s-1"), &json!("auto"))
        );
        for key in entry["env"].as_object().unwrap().keys() {
            assert!(
                !["KEY", "SECRET", "TOKEN"].iter().any(|w| key.contains(w)),
                "Codex strips {key}"
            );
        }
        let line = request("list_services", &id, "p", &json!({}));
        assert_eq!(
            (
                line["v"].clone(),
                line["runId"].clone(),
                line["chat"].clone()
            ),
            (json!(2), json!("R1"), json!("s-1"))
        );
        assert_eq!(
            parse_answer(r#"{"ok":true,"result":{"a":1}}"#).unwrap(),
            json!({ "a": 1 })
        );
        assert_eq!(
            parse_answer(r#"{"ok":false,"error":"nope"}"#).unwrap_err(),
            "nope"
        );
        assert!(!PROMPT_GUIDANCE.contains('/'), "path-free guidance");
    }
}
