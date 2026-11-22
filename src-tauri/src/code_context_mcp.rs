//! `hollow_code_context` (CC2): the code-context tools (`code_context`) as an
//! owned MCP server for Claude and Codex, over stdio (the hidden
//! `code-context-mcp` subcommand) or the shared endpoint
//! (`shared_mcp_host`), plus its attach entry and Settings toggle. Read
//! only; the checkout root comes from `HOLLOW_CODE_ROOT`.

use crate::code_context::{self, Target, ENV_ROOT, ENV_VAULT};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const DEFAULT_PROTOCOL: &str = "2024-11-05";

/// The server's name in an mcpServers map.
pub(crate) const SERVER_NAME: &str = "hollow_code_context";

/// Settings → "Look up code" (`codeContextMcpEnabled`); on by default.
static CODE_CONTEXT_MCP_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_code_context_mcp_enabled(v: bool) {
    CODE_CONTEXT_MCP_ENABLED.store(v, Ordering::Relaxed);
}

pub fn code_context_mcp_enabled() -> bool {
    CODE_CONTEXT_MCP_ENABLED.load(Ordering::Relaxed)
}

/// The `hollow_code_context` entry for an mcpServers map, or `None` when
/// the toggle is off, `cwd` is not inside a git work tree, or its git root
/// is the home folder or holds or sits inside the vault (the vault has
/// `hollow_vault`; `code_context::root_refusal`). The root is the cwd's git
/// top level; the vault goes along so the server refuses it too.
pub fn server_entry(cwd: &Path, vault_root: Option<&Path>) -> Option<(&'static str, Value)> {
    if !code_context_mcp_enabled() {
        return None;
    }
    let vault = vault_root.filter(|v| !v.as_os_str().is_empty());
    if let Some(vault) = vault {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        if canon(cwd).starts_with(canon(vault)) {
            return None;
        }
    }
    let root = crate::code_index::checkout_root(cwd)?;
    if code_context::root_refusal(&root, vault).is_some() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let mut env = json!({ ENV_ROOT: root.to_string_lossy() });
    if let Some(vault) = vault {
        env[ENV_VAULT] = json!(vault.to_string_lossy());
    }
    Some((
        SERVER_NAME,
        json!({
            "command": exe.to_string_lossy(),
            "args": ["code-context-mcp"],
            "env": env,
        }),
    ))
}

/// The `code-context-mcp` subcommand: JSON-RPC over stdio until EOF.
pub fn serve() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime");
    rt.block_on(async {
        let target = Target::from_env(|key| std::env::var(key).ok());
        let mut stdout = tokio::io::stdout();
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(raw)) = lines.next_line().await {
            let Ok(req) = serde_json::from_str::<Value>(raw.trim()) else {
                continue;
            };
            if let Some(resp) = handle(&req, &target) {
                let mut s = resp.to_string();
                s.push('\n');
                if stdout.write_all(s.as_bytes()).await.is_err() {
                    break;
                }
                let _ = stdout.flush().await;
            }
        }
    });
}

/// One JSON-RPC message; `None` for a notification.
pub(crate) fn handle(req: &Value, target: &Target) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let result = |r: Value| json!({ "jsonrpc": "2.0", "id": id.clone().unwrap_or(Value::Null), "result": r });
    match method {
        "initialize" => {
            let pv = req
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL);
            Some(result(json!({
                "protocolVersion": pv,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "hollow-code-context", "version": env!("CARGO_PKG_VERSION") },
            })))
        }
        "tools/list" => Some(result(json!({ "tools": code_context::specs() }))),
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let args = req.pointer("/params/arguments").cloned().unwrap_or(Value::Null);
            Some(result(code_context::call(name, &args, target)))
        }
        "ping" => Some(result(json!({}))),
        _ => id.map(|id| {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("method not found: {method}") } })
        }),
    }
}

// ── CLI twin (CC1 design 2.5) ───────────────────────────────────────────

/// The `code-context` subcommand's output for `args` (`<tool> '<json>'`,
/// or `--help`/`tools`), and its exit code. The text blocks of the MCP
/// result, one per line, so stdout is byte for byte what `handle` returns.
/// The root is `HOLLOW_CODE_ROOT`, else the git top level of the cwd; it is
/// refused, as the attached server's is, when it is the home folder or
/// holds the vault (`HOLLOW_CODE_VAULT`, else config.json's vault).
pub(crate) fn cli(args: &[String], target: &Target) -> (String, i32) {
    let usage = || {
        let names: Vec<String> = code_context::specs()
            .iter()
            .filter_map(|s| s["name"].as_str().map(str::to_string))
            .collect();
        format!(
            "usage: hollow code-context <tool> '<json arguments>'\ntools: {}\n`hollow code-context tools` prints their JSON schemas.\n`hollow code-context build-base --repo <path> --commit <sha> --out <file>` builds a base index of a commit; `hollow code-context import-base <file>` takes one in.\nThe checkout is the current folder's git root; the home folder and a root that holds the vault are refused.\nSettings > Look up code is app state this command can't read: it turns agents' attached server on or off, not this command.\n",
            names.join(", ")
        )
    };
    match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") | Some("help") => (usage(), 0),
        Some("tools") => (format!("{}\n", json!(code_context::specs())), 0),
        // CC3: a base index built from a commit (on the Mac mini) and taken
        // in here; docs/CODE_CONTEXT.md.
        Some("build-base") => {
            crate::code_index::overlay::cli_build_base(&args[1..], target.vault.as_deref())
        }
        Some("import-base") => crate::code_index::overlay::cli_import_base(
            &args[1..],
            &target.root,
            target.index_dir.as_deref(),
            target.vault.as_deref(),
        ),
        Some(tool) => {
            let parsed = match args.get(1).map(|a| a.trim()).filter(|a| !a.is_empty()) {
                None => Ok(json!({})),
                Some(raw) => serde_json::from_str::<Value>(raw),
            };
            let result = match parsed {
                Ok(v) => code_context::call(tool, &v, target),
                Err(e) => json!({
                    "content": crate::mcp_envelope::refusal_content(
                        crate::mcp_envelope::Server::Code,
                        &format!("{tool}: arguments must be a JSON object ({e})."),
                    ),
                    "isError": true,
                }),
            };
            let mut out = String::new();
            for block in result["content"].as_array().into_iter().flatten() {
                out.push_str(block["text"].as_str().unwrap_or(""));
                out.push('\n');
            }
            (out, if result["isError"] == true { 1 } else { 0 })
        }
    }
}

/// The `code-context` subcommand. Never reads stdin.
pub fn run_cli() {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let mut target = Target::from_env(|key| std::env::var(key).ok());
    if target.root.as_os_str().is_empty() {
        if let Some(root) = std::env::current_dir()
            .ok()
            .and_then(|d| crate::code_index::checkout_root(&d))
        {
            target.root = root;
        }
    }
    if target.vault.is_none() {
        target.vault = crate::config::locate_and_load()
            .ok()
            .map(|(config, _)| config.vault_path)
            .filter(|v| !v.as_os_str().is_empty());
    }
    let (out, code) = cli(&args, &target);
    print!("{out}");
    std::process::exit(code);
}

#[cfg(test)]
#[path = "code_context_mcp_tests.rs"]
mod tests;
