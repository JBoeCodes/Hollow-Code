//! Claude Code transcripts: `<projects folder>/<cwd slug>/<session id>.jsonl`.
//!
//! One JSON object per line with `type`, `uuid`, `parentUuid`, `sessionId`,
//! `cwd`, `timestamp`, `message` and `isSidechain`. `user` lines carry the
//! person's prompts and, as `tool_result` parts, the outputs of the tool
//! calls on earlier `assistant` lines (`tool_use` parts). Sidechain lines
//! (subagent runs inside the session) become a count, never turns. A
//! `summary` line is Claude Code's own title. Meta lines, compaction
//! summaries and local slash-command echoes are not the person's words and
//! are left out; the composer blocks Hollow itself injects are peeled with
//! `sessions::strip_injected_context`.

use super::lines::{self, Line};
use super::model::{Builder, Parsed};
use super::walker::{Entry, Kind};
use serde_json::Value;
use std::path::Path;

/// Whether `entry` is a Claude Code transcript: a `.jsonl` file directly in
/// a project folder (`<slug>/<id>.jsonl`). Deeper files (a session's own
/// `subagents/` folder) are subagent runs, not conversations.
pub(crate) fn is_transcript(entry: &Entry) -> bool {
    entry.kind == Kind::JsonLines && entry.rel.len() == 2
}

fn text_of(content: Option<&Value>, files: &mut Vec<String>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let mut out: Vec<String> = Vec::new();
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = part.get("text").and_then(Value::as_str) {
                            out.push(t.to_string());
                        }
                    }
                    Some("image") => files.push(format!("image {}", files.len() + 1)),
                    Some("document") => files.push(
                        part.get("title")
                            .or_else(|| part.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("document {}", files.len() + 1)),
                    ),
                    _ => {}
                }
            }
            out.join("\n")
        }
        _ => String::new(),
    }
}

/// The person's words on a `user` line, or `None` for a line that isn't a
/// prompt (tool results only, meta, a command echo).
fn prompt(evt: &Value, files: &mut Vec<String>) -> Option<String> {
    if evt.get("isMeta").and_then(Value::as_bool) == Some(true)
        || evt.get("isCompactSummary").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let raw = text_of(evt.get("message").and_then(|m| m.get("content")), files);
    let text = crate::sessions::strip_injected_context(&raw);
    let trimmed = text.trim_start();
    if trimmed.starts_with("<local-command-")
        || trimmed.starts_with("<command-message>")
        || trimmed.starts_with("<system-reminder>")
    {
        return None;
    }
    // `/model` and friends arrive as `<command-name>/model</command-name>...`.
    if let Some(rest) = trimmed.strip_prefix("<command-name>") {
        let name = rest.split("</command-name>").next().unwrap_or("").trim();
        return (!name.is_empty()).then(|| name.to_string());
    }
    Some(text)
}

/// Read one Claude Code transcript, streamed line by line.
pub(crate) fn read(path: &Path, max_bytes: usize) -> Result<Parsed, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut b = Builder::with_cap(max_bytes);
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
        let kind = evt.get("type").and_then(Value::as_str).unwrap_or("");
        let at = evt.get("timestamp").and_then(Value::as_str);
        if kind == "summary" {
            if let Some(s) = evt.get("summary").and_then(Value::as_str) {
                if !s.trim().is_empty() {
                    b.meta().title = Some(s.trim().to_string());
                }
            }
            return;
        }
        if kind != "user" && kind != "assistant" {
            return;
        }
        let Some(message) = evt.get("message").filter(|m| m.is_object()) else {
            return;
        };
        let meta = b.meta();
        meta.recognized = true;
        if meta.id.is_none() {
            meta.id = evt
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if meta.cwd.is_none() {
            meta.cwd = evt.get("cwd").and_then(Value::as_str).map(str::to_string);
        }
        if evt.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            meta.sidechain += 1;
            return;
        }
        b.saw_time(at);
        if kind == "user" {
            if let Some(parts) = message.get("content").and_then(Value::as_array) {
                for part in parts {
                    if part.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(id) = part.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    let output = crate::agent::extract_tool_result_text(part.get("content"));
                    let is_error = part
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    b.tool_result(id, &output, is_error);
                }
            }
            let mut files = Vec::new();
            if let Some(text) = prompt(&evt, &mut files) {
                b.user(&text, files, at);
            }
            return;
        }
        match message.get("content") {
            Some(Value::Array(parts)) => {
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = part.get("text").and_then(Value::as_str) {
                                b.assistant(t, at);
                            }
                        }
                        Some("tool_use") => b.tool_call(
                            part.get("id").and_then(Value::as_str).unwrap_or(""),
                            part.get("name").and_then(Value::as_str).unwrap_or("tool"),
                            part.get("input").cloned().unwrap_or(Value::Null),
                            at,
                        ),
                        _ => {}
                    }
                }
            }
            Some(Value::String(t)) => b.assistant(t, at),
            _ => {}
        }
    });
    let mut parsed = b.finish();
    if parsed.id.is_none() {
        parsed.id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string);
    }
    Ok(parsed)
}
