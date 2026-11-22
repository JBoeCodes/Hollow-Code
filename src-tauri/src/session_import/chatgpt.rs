//! ChatGPT's data export (H5b): a zip (or the extracted folder) holding
//! `conversations.json`, or `conversations-NNN.json` shards in newer
//! exports, each a JSON array of conversations, plus attachments that are
//! listed by name only (N28).
//!
//! **Staging.** [`stage`] streams each shard through [`archive`] and writes
//! one JSON file per conversation into the import's staging folder, so the
//! shared inspect, preview and commit code reads a conversation like any
//! other file ([`read`]); a shard of hundreds of MB is never held whole.
//!
//! **One conversation.** A conversation is a tree: `mapping` holds nodes
//! (`parent`, `children`, `message`) and `current_node` is the leaf the
//! person was last on. The transcript is the path from `current_node` up to
//! the root, oldest first; other branches (an edited prompt, a regenerated
//! answer) are counted in [`Parsed::other_branches`] and not imported. When
//! `current_node` is missing, the newest leaf stands in. System and
//! visually hidden messages, Custom Instructions context and reasoning
//! recaps are skipped. User and assistant text become prompts and replies;
//! images and uploads are named; code and tool calls (`recipient` other than
//! `all`, `code`, and the `tool` role's output) become short labelled
//! excerpts (never rerun, [`model::NOT_RERUN`]); a content type this reader
//! doesn't know is a one-line label. Unknown fields are ignored; a node or
//! part of the wrong shape is skipped and counted in `malformed`.

use super::archive::{self, Export, Staged};
use super::model::{Builder, Parsed};
use super::walker::{Entry, Kind};
use chrono::DateTime;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// A staged conversation file bigger than this isn't read.
const MAX_CONVERSATION_FILE: u64 = archive::MAX_ELEMENT_BYTES;
/// Nodes followed along one branch (a loop in a damaged mapping stops).
const MAX_PATH: usize = 200_000;

/// Whether an export file name is a conversations shard.
pub(crate) fn is_shard(name: &str) -> bool {
    let file = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    // At most one folder level (some zips wrap the export in a folder).
    if name.matches('/').count() > 1 {
        return false;
    }
    if file == "conversations.json" {
        return true;
    }
    file.strip_prefix("conversations-")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// A staged conversation file, as the walker lists it.
pub(crate) fn is_transcript(entry: &Entry) -> bool {
    entry.kind == Kind::Json && entry.rel.len() == 1
}

fn id_of(value: &Value) -> Option<String> {
    let obj = value.as_object()?;
    obj.get("id")
        .or_else(|| obj.get("conversation_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .map(str::to_string)
}

/// Split `export`'s conversation shards into `dir`, one file per
/// conversation (module docs).
pub(crate) fn stage(
    export: &mut Export,
    dir: &Path,
    cancel: &AtomicBool,
) -> Result<Staged, String> {
    archive::stage_conversations(export, dir, is_shard, id_of, cancel)
}

/// A node's time (`create_time` is seconds since the epoch, a float) as
/// RFC 3339.
fn time_of(value: Option<&Value>) -> Option<String> {
    let secs = value?.as_f64()?;
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    DateTime::from_timestamp(secs.floor() as i64, ((secs.fract()) * 1e9) as u32)
        .map(|t| t.to_rfc3339())
}

fn label(content_type: &str) -> String {
    let short: String = content_type
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(40)
        .collect();
    format!(
        "[{} content, not imported]",
        if short.is_empty() { "unknown" } else { &short }
    )
}

/// One message's words, and the images and files it carried by name.
struct Said {
    text: String,
    files: Vec<String>,
}

fn said(content: &Value, attachments: &[String]) -> Said {
    let content_type = content
        .get("content_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut text: Vec<String> = Vec::new();
    let mut files: Vec<String> = attachments.to_vec();
    match content_type {
        "code" | "execution_output" => {
            if let Some(t) = content.get("text").and_then(Value::as_str) {
                text.push(t.to_string());
            }
        }
        "tether_quote" => {
            let title = content.get("title").and_then(Value::as_str).unwrap_or("");
            let url = content.get("url").and_then(Value::as_str).unwrap_or("");
            if !title.is_empty() || !url.is_empty() {
                text.push(format!("[Web quote: {} {}]", title, url).replace("  ", " "));
            }
        }
        _ => {
            if let Some(parts) = content.get("parts").and_then(Value::as_array) {
                for part in parts {
                    match part {
                        Value::String(s) => text.push(s.clone()),
                        Value::Object(o) => {
                            let kind = o.get("content_type").and_then(Value::as_str).unwrap_or("");
                            match kind {
                                "image_asset_pointer" => {
                                    files.push(format!("image {}", files.len() + 1))
                                }
                                "audio_asset_pointer"
                                | "real_time_user_audio_video_asset_pointer" => {
                                    files.push(format!("audio {}", files.len() + 1))
                                }
                                _ => {
                                    if let Some(t) = o.get("text").and_then(Value::as_str) {
                                        text.push(t.to_string());
                                    } else if !kind.is_empty() {
                                        text.push(label(kind));
                                    }
                                }
                            }
                        }
                        // null parts and bare numbers carry nothing.
                        _ => {}
                    }
                }
            } else if let Some(t) = content.get("text").and_then(Value::as_str) {
                text.push(t.to_string());
            } else if !content_type.is_empty() && content_type != "text" {
                text.push(label(content_type));
            }
        }
    }
    Said {
        text: text.join("\n"),
        files,
    }
}

fn attachment_names(msg: &Value) -> Vec<String> {
    msg.get("metadata")
        .and_then(|m| m.get("attachments"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| a.get("name").and_then(Value::as_str))
                .map(|n| n.chars().take(200).collect::<String>())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Content types that aren't the conversation: Custom Instructions context
/// and the model's reasoning.
fn hidden_type(content_type: &str) -> bool {
    matches!(
        content_type,
        "user_editable_context" | "model_editable_context" | "thoughts" | "reasoning_recap"
    )
}

fn is_hidden(msg: &Value) -> bool {
    let meta = msg.get("metadata");
    let flag = |key: &str| {
        meta.and_then(|m| m.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    flag("is_visually_hidden_from_conversation") || flag("is_user_system_message")
}

/// The node ids from the root to the transcript's last node, and how many
/// other branches the tree has. `None` when the mapping holds no node.
fn linearize(
    mapping: &serde_json::Map<String, Value>,
    current: Option<&str>,
) -> Option<(Vec<String>, usize, bool)> {
    let mut recovered = false;
    let start = match current.filter(|c| mapping.contains_key(*c)) {
        Some(c) => c.to_string(),
        None => {
            recovered = true;
            // The newest leaf stands in for a missing current_node.
            mapping
                .iter()
                .filter(|(_, n)| {
                    n.get("children")
                        .and_then(Value::as_array)
                        .is_none_or(Vec::is_empty)
                })
                .max_by(|(ia, a), (ib, b)| {
                    let t = |n: &Value| {
                        n.get("message")
                            .and_then(|m| m.get("create_time"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0)
                    };
                    t(a).total_cmp(&t(b)).then(ib.cmp(ia))
                })
                .map(|(id, _)| id.clone())?
        }
    };
    let mut path: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut at = Some(start.clone());
    while let Some(id) = at {
        if path.len() >= MAX_PATH || !seen.insert(id.clone()) {
            break;
        }
        let Some(node) = mapping.get(&id) else { break };
        path.push(id);
        at = node
            .get("parent")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    path.reverse();
    // Leaves with a message that aren't the last node: other branches.
    let other = mapping
        .iter()
        .filter(|(id, n)| {
            **id != start
                && !seen.contains(*id)
                && n.get("message").is_some_and(Value::is_object)
                && n.get("children")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty)
        })
        .count();
    Some((path, other, recovered))
}

/// Parse one conversation JSON value.
pub(crate) fn parse(conv: &Value, cap: usize) -> Parsed {
    let mut builder = Builder::with_cap(cap);
    let Some(obj) = conv.as_object() else {
        return builder.finish();
    };
    let Some(mapping) = obj.get("mapping").and_then(Value::as_object) else {
        return builder.finish();
    };
    let id = id_of(conv);
    {
        let meta = builder.meta();
        meta.recognized = id.is_some();
        meta.id = id;
        meta.title = obj
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty());
    }
    let current = obj.get("current_node").and_then(Value::as_str);
    let Some((path, other, recovered)) = linearize(mapping, current) else {
        return builder.finish();
    };
    if recovered && !mapping.is_empty() {
        builder.meta().malformed += 1;
    }
    builder.meta().other_branches = other;
    // The tool call a following `tool` message answers.
    let mut pending: Option<String> = None;
    for node_id in &path {
        let Some(node) = mapping.get(node_id) else {
            continue;
        };
        let Some(msg) = node.get("message") else {
            continue;
        };
        if msg.is_null() {
            continue;
        }
        if !msg.is_object() {
            builder.meta().malformed += 1;
            continue;
        }
        let Some(content) = msg.get("content").filter(|c| c.is_object()) else {
            continue;
        };
        let content_type = content
            .get("content_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        if is_hidden(msg) || hidden_type(content_type) {
            continue;
        }
        let role = msg
            .get("author")
            .and_then(|a| a.get("role"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let at = time_of(msg.get("create_time"));
        let at = at.as_deref();
        builder.saw_time(at);
        match role {
            "user" => {
                let s = said(content, &attachment_names(msg));
                builder.user(&s.text, s.files, at);
            }
            "assistant" => {
                let recipient = msg
                    .get("recipient")
                    .and_then(Value::as_str)
                    .unwrap_or("all");
                let s = said(content, &[]);
                let inline = if s.files.is_empty() {
                    s.text.clone()
                } else {
                    format!("{}\n[{}]", s.text, s.files.join(", "))
                };
                if recipient != "all" || content_type == "code" {
                    let name = if recipient == "all" {
                        "code"
                    } else {
                        recipient
                    };
                    builder.tool_call(node_id, name, json!({ "input": inline }), at);
                    pending = Some(node_id.clone());
                } else {
                    builder.assistant(&inline, at);
                }
            }
            "tool" => {
                let s = said(content, &[]);
                let text = if s.files.is_empty() {
                    s.text
                } else {
                    format!("{}\n[{}]", s.text, s.files.join(", "))
                };
                match pending.take() {
                    Some(call) => builder.tool_result(&call, &text, false),
                    None => {
                        let name = msg
                            .get("author")
                            .and_then(|a| a.get("name"))
                            .and_then(Value::as_str)
                            .filter(|n| !n.is_empty())
                            .unwrap_or("tool");
                        builder.tool_call(node_id, name, json!({}), at);
                        builder.tool_result(node_id, &text, false);
                    }
                }
            }
            // system, and any role this reader doesn't know.
            _ => {}
        }
    }
    let mut parsed = builder.finish();
    if let Some(created) = time_of(obj.get("create_time")) {
        parsed.created_at = Some(created);
    }
    if let Some(updated) = time_of(obj.get("update_time")) {
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
