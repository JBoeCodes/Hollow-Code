//! Read-only Pi JSONL history. Versions 2 and 3 keep only the path to the
//! last entry, following parentId; version 1 is linear. Configuration,
//! reasoning and extension state are not conversation messages.

use super::lines::{self, Line};
use super::model::{Builder, Parsed};
use super::walker::{Entry, Kind};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

const FILE_MAX: u64 = 64 * 1024 * 1024;
const ENTRY_MAX: usize = 100_000;

pub(crate) fn is_transcript(entry: &Entry) -> bool {
    entry.kind == Kind::JsonLines && entry.rel.len() <= 2
}

fn string<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn text(content: &Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| string(part, "type") == Some("text"))
        .filter_map(|part| string(part, "text"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn read(path: &Path, max_bytes: usize) -> Result<Parsed, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Pi history: {e}"))?;
    let mut bytes = Vec::new();
    file.take(FILE_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Pi history: {e}"))?;
    if bytes.len() as u64 > FILE_MAX {
        return Err("Pi history exceeds the 64 MB file limit.".into());
    }
    parse(&bytes, max_bytes)
}

fn parse(bytes: &[u8], max_bytes: usize) -> Result<Parsed, String> {
    let mut builder = Builder::with_cap(max_bytes);
    let mut version = 0;
    let mut seen_header = false;
    let mut bad_tree = false;
    let mut parents: HashMap<String, Option<String>> = HashMap::new();
    let mut leaf: Option<String> = None;
    // Both passes use the same bounded snapshot, so a live append cannot
    // change ancestry between discovery and message extraction.
    lines::for_each(bytes, |line| {
        let value = match line {
            Line::Json(v) => v,
            Line::Malformed { last } => {
                builder.meta().malformed += 1;
                builder.meta().truncated |= last;
                return;
            }
            Line::TooLong => {
                builder.meta().malformed += 1;
                return;
            }
        };
        if !seen_header {
            seen_header = true;
            if string(&value, "type") != Some("session") {
                return;
            }
            version = value.get("version").and_then(Value::as_u64).unwrap_or(1);
            if !(1..=3).contains(&version) {
                version = 0;
                return;
            }
            builder.meta().id = string(&value, "id")
                .filter(|id| !id.is_empty())
                .map(str::to_owned);
            builder.meta().cwd = string(&value, "cwd").map(str::to_owned);
            builder.meta().recognized = builder.meta().id.is_some();
            builder.saw_time(string(&value, "timestamp"));
            return;
        }
        if version == 0 || bad_tree {
            return;
        }
        if string(&value, "type") == Some("session") {
            bad_tree = true;
            return;
        }
        if string(&value, "type") == Some("session_info") {
            builder.meta().title = string(&value, "name").map(str::to_owned);
        }
        if version == 1 {
            return;
        }
        let Some(id) = string(&value, "id").filter(|id| !id.is_empty() && id.len() <= 256) else {
            bad_tree = true;
            return;
        };
        let parent = match value.get("parentId") {
            Some(Value::Null) => None,
            Some(Value::String(p)) if parents.contains_key(p) => Some(p.clone()),
            _ => {
                bad_tree = true;
                return;
            }
        };
        if parents.len() >= ENTRY_MAX || parents.contains_key(id) {
            bad_tree = true;
            return;
        }
        parents.insert(id.to_owned(), parent);
        leaf = Some(id.to_owned());
    });
    if bad_tree {
        return Err("Pi history has an invalid or oversized entry tree.".into());
    }
    if version == 0 {
        return Ok(builder.finish());
    }
    let mut active = HashSet::new();
    let mut next = leaf.clone();
    while let Some(id) = next {
        if !active.insert(id.clone()) {
            return Err("Pi history contains an entry cycle.".into());
        }
        next = parents.get(&id).cloned().flatten();
    }
    let referenced: HashSet<&str> = parents.values().filter_map(|p| p.as_deref()).collect();
    builder.meta().other_branches = parents
        .keys()
        .filter(|id| !referenced.contains(id.as_str()) && Some(id.as_str()) != leaf.as_deref())
        .count();
    lines::for_each(bytes, |line| {
        let Line::Json(value) = line else { return };
        if version >= 2 && !string(&value, "id").is_some_and(|id| active.contains(id)) {
            return;
        }
        if string(&value, "type") != Some("message") {
            return;
        }
        let Some(message) = value.get("message") else {
            return;
        };
        let at = string(&value, "timestamp");
        let content = message.get("content").unwrap_or(&Value::Null);
        match string(message, "role") {
            Some("user") => {
                builder.saw_time(at);
                let files = content
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|part| string(part, "type") == Some("image"))
                    .map(|_| "Image from Pi history".to_string())
                    .collect();
                builder.user(&text(content), files, at);
            }
            Some("assistant") => {
                builder.saw_time(at);
                builder.assistant(&text(content), at);
                for part in content.as_array().into_iter().flatten() {
                    if string(part, "type") == Some("toolCall") {
                        builder.tool_call(
                            string(part, "id").unwrap_or(""),
                            string(part, "name").unwrap_or("tool"),
                            part.get("arguments").cloned().unwrap_or(Value::Null),
                            at,
                        );
                    }
                }
            }
            Some("toolResult") => {
                builder.saw_time(at);
                builder.tool_result(
                    string(message, "toolCallId").unwrap_or(""),
                    &text(content),
                    message
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                );
            }
            _ => {}
        }
    });
    Ok(builder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE: &str = include_str!("../../fixtures/session_import/pi_session.jsonl");

    #[test]
    fn pi_uses_the_latest_branch_without_reasoning_or_extension_state() {
        let p = parse(FIXTURE.as_bytes(), 8 * 1024 * 1024).unwrap();
        assert!(p.recognized);
        assert_eq!(p.id.as_deref(), Some("pi-sample"));
        assert_eq!(p.title.as_deref(), Some("Pi import example"));
        assert_eq!(p.other_branches, 1);
        assert_eq!(p.turns.len(), 1);
        assert_eq!(p.turns[0].asked, "Inspect the example");
        assert_eq!(p.turns[0].reply, "The selected answer");
        assert_eq!(p.turns[0].tools[0].name, "read");
        assert_eq!(
            p.turns[0].tools[0].output.as_deref(),
            Some("example content")
        );
        assert!(!format!("{p:?}").contains("SECRET_REASONING"));
        assert!(!format!("{p:?}").contains("abandoned answer"));
    }

    #[test]
    fn pi_refuses_broken_ancestry_and_duplicate_ids() {
        assert!(parse(
            FIXTURE
                .replace("\"parentId\":\"u1\"", "\"parentId\":\"missing\"")
                .as_bytes(),
            4096
        )
        .is_err());
        assert!(parse(
            format!(
                "{FIXTURE}{}\n",
                r#"{"type":"label","id":"u1","parentId":null}"#
            )
            .as_bytes(),
            4096
        )
        .is_err());
    }

    #[test]
    fn pi_supports_legacy_linear_history_and_records_a_cut_final_line() {
        let s = concat!(
            r#"{"type":"session","id":"legacy"}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":"Hi"}}"#,
            "\n",
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]}}"#,
            "\n{\"type\":"
        );
        let p = parse(s.as_bytes(), 4096).unwrap();
        assert_eq!(p.turns[0].asked, "Hi");
        assert_eq!(p.turns[0].reply, "Hello");
        assert!(p.truncated);
        assert_eq!(p.malformed, 1);
    }

    #[test]
    fn pi_unknown_format_is_not_importable() {
        assert!(
            !parse(
                b"{\"type\":\"session\",\"version\":99,\"id\":\"future\"}\n",
                4096
            )
            .unwrap()
            .recognized
        );
        assert!(
            !parse(
                b"{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"Hi\"}}\n",
                4096
            )
            .unwrap()
            .recognized
        );
    }
}
