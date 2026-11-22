//! Codex rollouts: `<sessions folder>/YYYY/MM/DD/rollout-*.jsonl`.
//!
//! The first line is `session_meta` (its `payload` has the session `id`,
//! `cwd` and `timestamp`; the oldest rollouts have an untyped
//! `{id, timestamp}` line instead), then `response_item` lines (messages,
//! tool calls and their outputs) and `event_msg` lines, which repeat the
//! same conversation for the TUI and are skipped. Codex's own context items
//! sent as user messages (environment context, AGENTS.md, instructions) are
//! not the person's words and are dropped. A rollout whose `source` is an
//! object is a subagent's own thread ([`Parsed::subagent`]).

use super::lines::{self, Line};
use super::model::{Builder, Parsed};
use super::walker::{Entry, Kind};
use serde_json::{json, Value};
use std::path::Path;

/// Codex's own context items, sent as user messages: never the user's words.
const INJECTED: [&str; 12] = [
    "<environment_context>",
    "<user_instructions>",
    "# AGENTS.md instructions",
    "<recommended_plugins>",
    "<system_instruction>",
    "<INSTRUCTIONS>",
    "<permissions",
    "<collaboration_mode",
    "<in-app-browser-context",
    "<skills_instructions>",
    "<turn_aborted>",
    "</image>",
];

/// Whether `entry` is a Codex rollout (`rollout-*.jsonl`, at most four
/// levels down: `YYYY/MM/DD/`).
pub(crate) fn is_transcript(entry: &Entry) -> bool {
    entry.kind == Kind::JsonLines && entry.name().starts_with("rollout-") && entry.rel.len() <= 4
}

fn user_text(payload: &Value, files: &mut Vec<String>) -> Option<String> {
    let parts = payload.get("content")?.as_array()?;
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("input_text") | Some("text") => {
                let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                let trimmed = text.trim_start();
                if trimmed.starts_with("<image") {
                    continue;
                }
                if INJECTED.iter().any(|marker| trimmed.starts_with(marker)) {
                    continue;
                }
                let stripped = crate::sessions::strip_injected_context(text);
                if !stripped.trim().is_empty() {
                    out.push(stripped);
                }
            }
            Some("input_image") => files.push(format!("image {}", files.len() + 1)),
            Some("input_file") => files.push(
                part.get("filename")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("file {}", files.len() + 1)),
            ),
            _ => {}
        }
    }
    let joined = out.join("\n");
    (!joined.trim().is_empty() || !files.is_empty()).then_some(joined)
}

fn assistant_text(payload: &Value) -> String {
    let Some(parts) = payload.get("content").and_then(Value::as_array) else {
        return String::new();
    };
    parts
        .iter()
        .filter(|p| {
            matches!(
                p.get("type").and_then(Value::as_str),
                Some("output_text") | Some("text")
            )
        })
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tool output in any of Codex's shapes: a string (sometimes itself JSON
/// with an `output` field), `{content|output}`, or an array of text parts.
fn output_text(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(s)) => {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(s) {
                if let Some(inner) = map.get("output").and_then(Value::as_str) {
                    return inner.to_string();
                }
            }
            s.clone()
        }
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Object(map)) => map
            .get("content")
            .or_else(|| map.get("output"))
            .map(|v| output_text(Some(v)))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// The meta payload: a typed `session_meta` line's, or the oldest untyped
/// `{id, timestamp}` first line.
fn meta_payload(evt: &Value) -> Option<&Value> {
    match evt.get("type").and_then(Value::as_str) {
        Some("session_meta") => evt.get("payload"),
        None if evt.get("id").is_some() && evt.get("timestamp").is_some() => Some(evt),
        _ => None,
    }
}

/// The item of a line: a `response_item`'s payload, or (oldest rollouts)
/// the line itself.
fn item(evt: &Value) -> Option<&Value> {
    match evt.get("type").and_then(Value::as_str) {
        Some("response_item") => evt.get("payload"),
        Some("message")
        | Some("function_call")
        | Some("function_call_output")
        | Some("custom_tool_call")
        | Some("custom_tool_call_output")
        | Some("local_shell_call")
        | Some("web_search_call") => Some(evt),
        _ => None,
    }
}

/// Read one Codex rollout, streamed line by line. A file whose first JSON
/// line is no session meta is not a rollout ([`Parsed::recognized`] false).
pub(crate) fn read(path: &Path, max_bytes: usize) -> Result<Parsed, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut b = Builder::with_cap(max_bytes);
    let mut first = true;
    lines::for_each(file, |line| {
        let evt = match line {
            Line::Json(evt) => evt,
            Line::Malformed { last } => {
                b.meta().malformed += 1;
                b.meta().truncated |= last;
                return;
            }
            Line::TooLong => {
                b.meta().malformed += 1;
                return;
            }
        };
        if first {
            first = false;
            if let Some(payload) = meta_payload(&evt) {
                let string = |key: &str| {
                    payload
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                };
                let meta = b.meta();
                meta.recognized = true;
                // `id` first: a subagent rollout carries its parent's
                // `session_id`.
                meta.id = string("id").or_else(|| string("session_id"));
                meta.cwd = string("cwd");
                meta.subagent = payload.get("source").is_some_and(Value::is_object);
                let at = string("timestamp");
                b.saw_time(at.as_deref());
                return;
            }
        }
        if !b.meta().recognized {
            return;
        }
        let at = evt.get("timestamp").and_then(Value::as_str);
        if evt.get("type").and_then(Value::as_str) == Some("turn_context") {
            if b.meta().cwd.is_none() {
                b.meta().cwd = evt
                    .get("payload")
                    .and_then(|p| p.get("cwd"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            return;
        }
        let Some(item) = item(&evt) else { return };
        b.saw_time(at);
        let string = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            "message" => match item.get("role").and_then(Value::as_str) {
                Some("user") => {
                    let mut files = Vec::new();
                    if let Some(text) = user_text(item, &mut files) {
                        b.user(&text, files, at);
                    }
                }
                Some("assistant") => b.assistant(&assistant_text(item), at),
                _ => {}
            },
            "function_call" => {
                let args = string("arguments");
                let input = serde_json::from_str::<Value>(&args).unwrap_or(Value::String(args));
                b.tool_call(&string("call_id"), &string("name"), input, at);
            }
            "custom_tool_call" => b.tool_call(
                &string("call_id"),
                &string("name"),
                json!({ "input": string("input") }),
                at,
            ),
            "local_shell_call" => {
                let command = item
                    .get("action")
                    .and_then(|a| a.get("command"))
                    .cloned()
                    .unwrap_or(Value::Null);
                b.tool_call(
                    &string("call_id"),
                    "shell",
                    json!({ "command": command }),
                    at,
                );
            }
            "web_search_call" => {
                let query = item
                    .get("action")
                    .and_then(|a| a.get("query"))
                    .cloned()
                    .unwrap_or(Value::Null);
                b.tool_call(&string("id"), "web_search", json!({ "query": query }), at);
            }
            "function_call_output" | "custom_tool_call_output" => {
                let text = output_text(item.get("output"));
                let is_error = item
                    .get("output")
                    .and_then(|o| o.get("success"))
                    .and_then(Value::as_bool)
                    == Some(false);
                b.tool_result(&string("call_id"), &text, is_error);
            }
            _ => {}
        }
    });
    Ok(b.finish())
}
