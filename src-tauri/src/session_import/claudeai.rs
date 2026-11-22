//! claude.ai's data export (H5b): a zip (or the extracted folder) holding
//! `conversations.json`, a JSON array of conversations, plus `projects.json`,
//! `users.json` and the like. Only `conversations.json` is read: the other
//! files are never opened (no profile or account data is imported) and are
//! listed by name as skipped files, as attachments are (N28).
//!
//! **Staging** reuses [`archive::stage_conversations`] unchanged: each
//! conversation becomes one staging file named by the hash of its `uuid`
//! (never the raw id), so the shared inspect, preview and commit code reads
//! it like any other file ([`read`]).
//!
//! **One conversation:** `uuid`, `name`, `created_at`, `updated_at` and
//! `chat_messages`, each with a `sender` (`human` or `assistant`), a `text`,
//! often `content` blocks, and `attachments` and `files` that are named and
//! never copied (their `extracted_content` is ignored). Content blocks win
//! over `text` when present: `text` blocks are the words, `tool_use` and
//! `tool_result` blocks become short labelled excerpts (never rerun,
//! [`model::NOT_RERUN`]), `thinking` is skipped, an image is named, and a
//! block type this reader doesn't know is a one-line label. Unknown fields
//! are ignored; a message or block of the wrong shape is skipped and counted
//! in `malformed`.

use super::archive::{self, Export, Staged};
use super::model::{Builder, Parsed};
use super::walker::{Entry, Kind};
use chrono::DateTime;
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// A staged conversation file bigger than this isn't read.
const MAX_CONVERSATION_FILE: u64 = 256 * 1024 * 1024;

/// Whether an export file name is the conversations file (at most one
/// wrapping folder level).
pub(crate) fn is_shard(name: &str) -> bool {
    if name.matches('/').count() > 1 {
        return false;
    }
    name.rsplit('/')
        .next()
        .unwrap_or(name)
        .eq_ignore_ascii_case("conversations.json")
}

/// A staged conversation file, as the walker lists it.
pub(crate) fn is_transcript(entry: &Entry) -> bool {
    entry.kind == Kind::Json && entry.rel.len() == 1
}

fn id_of(value: &Value) -> Option<String> {
    value
        .as_object()?
        .get("uuid")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .map(str::to_string)
}

/// Split `export`'s `conversations.json` into `dir`, one file per
/// conversation (module docs).
pub(crate) fn stage(
    export: &mut Export,
    dir: &Path,
    cancel: &AtomicBool,
) -> Result<Staged, String> {
    archive::stage_conversations(export, dir, is_shard, id_of, cancel)
}

/// An RFC 3339 time as given, or `None` when it doesn't parse.
fn time_of(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.to_rfc3339())
}

fn label(kind: &str) -> String {
    let short: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(40)
        .collect();
    format!(
        "[{} content, not imported]",
        if short.is_empty() { "unknown" } else { &short }
    )
}

/// The names of a message's `attachments` and `files` (the text inside an
/// attachment is never read, N28).
fn file_names(msg: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for key in ["attachments", "files"] {
        let Some(list) = msg.get(key).and_then(Value::as_array) else {
            continue;
        };
        for item in list {
            let name = item
                .get("file_name")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .map(|n| n.chars().take(200).collect::<String>())
                .filter(|n| !n.trim().is_empty());
            if let Some(name) = name {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

/// A `tool_result` block's content as text: a string, or a list of text
/// parts (other parts are labelled).
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| match p {
                Value::String(s) => Some(s.clone()),
                Value::Object(o) => match o.get("type").and_then(Value::as_str) {
                    Some("text") | None => o.get("text").and_then(Value::as_str).map(String::from),
                    Some(kind) => Some(label(kind)),
                },
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Parse one conversation JSON value.
pub(crate) fn parse(conv: &Value, cap: usize) -> Parsed {
    let mut builder = Builder::with_cap(cap);
    let Some(obj) = conv.as_object() else {
        return builder.finish();
    };
    let Some(messages) = obj.get("chat_messages").and_then(Value::as_array) else {
        return builder.finish();
    };
    let id = id_of(conv);
    {
        let meta = builder.meta();
        meta.recognized = id.is_some();
        meta.id = id;
        meta.title = obj
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty());
    }
    for msg in messages {
        let Some(m) = msg.as_object() else {
            builder.meta().malformed += 1;
            continue;
        };
        let at = time_of(m.get("created_at"));
        let at = at.as_deref();
        builder.saw_time(at);
        let sender = m.get("sender").and_then(Value::as_str).unwrap_or("");
        let human = match sender {
            "human" | "user" => true,
            "assistant" => false,
            _ => continue,
        };
        // Words, tool calls and results, and the names of images.
        let mut words: Vec<String> = Vec::new();
        let mut files = file_names(msg);
        let mut calls: Vec<(String, String, Value)> = Vec::new();
        let mut results: Vec<(String, String, bool)> = Vec::new();
        match m.get("content").and_then(Value::as_array) {
            Some(blocks) if !blocks.is_empty() => {
                for block in blocks {
                    let Some(b) = block.as_object() else {
                        builder.meta().malformed += 1;
                        continue;
                    };
                    match b.get("type").and_then(Value::as_str).unwrap_or("") {
                        "text" => {
                            if let Some(t) = b.get("text").and_then(Value::as_str) {
                                words.push(t.to_string());
                            }
                        }
                        "tool_use" => {
                            let id = b.get("id").and_then(Value::as_str).unwrap_or("");
                            let name = b.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let input = b.get("input").cloned().unwrap_or_else(|| json!({}));
                            calls.push((id.to_string(), name.to_string(), input));
                        }
                        "tool_result" => {
                            let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                            let error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                            results.push((id.to_string(), result_text(b.get("content")), error));
                        }
                        "thinking" | "redacted_thinking" => {}
                        "image" => files.push(format!("image {}", files.len() + 1)),
                        "" => builder.meta().malformed += 1,
                        other => words.push(label(other)),
                    }
                }
            }
            _ => {
                if let Some(t) = m.get("text").and_then(Value::as_str) {
                    words.push(t.to_string());
                }
            }
        }
        let text = words.join("\n");
        if human {
            builder.user(&text, files, at);
        } else {
            let inline = if files.is_empty() {
                text
            } else {
                format!("{text}\n[{}]", files.join(", "))
            };
            builder.assistant(&inline, at);
        }
        for (id, name, input) in calls {
            builder.tool_call(&id, &name, json!({ "input": input }), at);
        }
        for (id, output, error) in results {
            builder.tool_result(&id, &output, error);
        }
    }
    let mut parsed = builder.finish();
    if let Some(created) = time_of(obj.get("created_at")) {
        parsed.created_at = Some(created);
    }
    if let Some(updated) = time_of(obj.get("updated_at")) {
        parsed.last_at = Some(updated);
    }
    parsed
}

/// Read one staged conversation file (or any file holding one conversation
/// object).
pub(crate) fn read(path: &Path, max_bytes: usize) -> Result<Parsed, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("conversation file: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_CONVERSATION_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("conversation file: {e}"))?;
    if bytes.len() as u64 > MAX_CONVERSATION_FILE {
        return Err("That conversation is too large to read.".into());
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("conversation file: {e}"))?;
    Ok(parse(&value, max_bytes))
}
