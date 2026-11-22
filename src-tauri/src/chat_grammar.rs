//! The chat event stream grammar (AG1): Hollow's own rules for what one
//! turn's `chat-event` stream may look like, as a pure checker.
//!
//! The rules are written as MUST statements in
//! `docs/reference/chat-stream-grammar.md`; the rule ids here (`G01`..) are the
//! same ids. The checker reads a recorded stream (one JSON event per line, as
//! `chat::emit_event` payloads) and names the first rule broken and the
//! 1-based line it broke on. The corpus under `tests/fixtures/chat-streams/`
//! holds a stream per adapter (Claude, Codex, Antigravity, Grok) plus planted
//! bad ones, each with its expected verdict in `manifest.json`.
//!
//! Hollow has no message end event, so an "open" text segment runs from a
//! `turnBoundary` (or the turn's start) to the next one; an "open" tool is a
//! `toolUse` with an id and no `toolResult` yet. The ordering rules are the
//! author's reading of `chat.rs` and the adapters, not a recording of a live
//! run: a fixture that fails for a real turn means the rule or the code needs
//! a decision, and the doc says which.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Every event type `ChatEventPayload` knows: the AG2 schema's variants.
const KNOWN: &[&str] = crate::chat_event::TYPES;

/// Events that draw conversation content; none may follow an ending event.
const CONTENT: &[&str] = &[
    "assistantText",
    "assistantDelta",
    "turnBoundary",
    "toolUse",
    "toolResult",
    "toolImages",
];

/// A broken rule: the id, the 1-based line (0 for a whole-stream rule that has
/// no single line, reported on the last line) and what was wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Violation {
    pub rule: &'static str,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at line {}: {}", self.rule, self.line, self.message)
    }
}

fn bad(rule: &'static str, line: usize, message: impl Into<String>) -> Violation {
    Violation {
        rule,
        line,
        message: message.into(),
    }
}

/// Checks a JSON Lines stream. Blank lines are skipped but still counted.
pub(crate) fn check_jsonl(text: &str) -> Result<(), Violation> {
    let mut events = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(raw) {
            Ok(value) => events.push((index + 1, value)),
            Err(error) => return Err(bad("G01", index + 1, format!("not JSON: {error}"))),
        }
    }
    check_events(&events)
}

fn str_of<'a>(event: &'a Value, key: &str) -> Option<&'a str> {
    event.get(key).and_then(Value::as_str)
}

/// Checks `(line, event)` pairs in order.
pub(crate) fn check_events(events: &[(usize, Value)]) -> Result<(), Violation> {
    let last_line = events.last().map(|(l, _)| *l).unwrap_or(0);
    let Some((first_line, first)) = events.first() else {
        return Err(bad("G02", 0, "the stream is empty"));
    };
    let first_type = str_of(first, "type").unwrap_or("");
    // G02: a started turn opens with runStarted; a turn that never started
    // (taken out of the queue, or recovered) is only cancelled/error then done.
    let never_started = matches!(first_type, "cancelled" | "error");
    if first_type != "runStarted" && !never_started {
        return Err(bad(
            "G02",
            *first_line,
            format!("first event is `{first_type}`, not runStarted"),
        ));
    }

    let mut run_id: Option<String> = None;
    let mut conversation: Option<String> = None;
    let mut open_tools: HashMap<String, usize> = HashMap::new();
    let mut seen_tools: HashSet<String> = HashSet::new();
    let mut resulted: HashSet<String> = HashSet::new();
    let mut ended_at: Option<(usize, String)> = None; // error / cancelled / paused
    let mut interrupted = false; // the turn ended without finishing its tools
    let mut saw_usage = false;
    let mut segment_has_delta = false;
    let mut started = false;
    let mut done_line = None;

    for (line, event) in events {
        let line = *line;
        let Some(kind) = str_of(event, "type") else {
            return Err(bad("G01", line, "event has no string `type`"));
        };
        if !KNOWN.contains(&kind) {
            return Err(bad("G01", line, format!("unknown event type `{kind}`")));
        }
        if let Some(done) = done_line {
            return Err(bad(
                "G03",
                line,
                format!("`{kind}` after done (line {done})"),
            ));
        }
        // G09: one conversation per stream, when events say which.
        if let Some(key) = str_of(event, "conversationKey") {
            match &conversation {
                Some(seen) if seen != key => {
                    return Err(bad(
                        "G09",
                        line,
                        format!("conversationKey `{key}` differs from `{seen}`"),
                    ));
                }
                None => conversation = Some(key.to_string()),
                _ => {}
            }
        }
        if kind == "runStarted" {
            if started || never_started {
                return Err(bad(
                    "G02",
                    line,
                    "runStarted is not the first and only start",
                ));
            }
            started = true;
            run_id = str_of(event, "runId").map(str::to_string);
        }
        if never_started && !matches!(kind, "cancelled" | "error" | "done") {
            return Err(bad(
                "G02",
                line,
                format!("`{kind}` in a turn that never started"),
            ));
        }
        if let Some((end_line, end_kind)) = &ended_at {
            if CONTENT.contains(&kind) {
                return Err(bad(
                    "G04",
                    line,
                    format!("`{kind}` after `{end_kind}` (line {end_line}); only closing events may follow"),
                ));
            }
        }
        match kind {
            "session" => {
                if str_of(event, "sessionId").is_none_or(str::is_empty) {
                    return Err(bad("G08", line, "session has no sessionId"));
                }
            }
            "assistantText" | "assistantDelta" => {
                if str_of(event, "text").is_none() {
                    return Err(bad("G08", line, format!("{kind} has no string `text`")));
                }
                if kind == "assistantDelta" {
                    segment_has_delta = true;
                } else if segment_has_delta {
                    return Err(bad(
                        "G07",
                        line,
                        "assistantText in a segment that already streamed deltas would draw the text twice",
                    ));
                }
            }
            "turnBoundary" => segment_has_delta = false,
            "toolUse" => {
                if str_of(event, "name").is_none() {
                    return Err(bad("G08", line, "toolUse has no string `name`"));
                }
                if let Some(id) = str_of(event, "id").filter(|id| !id.is_empty()) {
                    if !seen_tools.insert(id.to_string()) {
                        return Err(bad("G05", line, format!("toolUse id `{id}` already used")));
                    }
                    open_tools.insert(id.to_string(), line);
                }
            }
            "toolResult" | "toolImages" => {
                let Some(id) = str_of(event, "id") else {
                    return Err(bad("G08", line, format!("{kind} has no string `id`")));
                };
                if kind == "toolResult"
                    && (str_of(event, "content").is_none()
                        || event.get("isError").and_then(Value::as_bool).is_none())
                {
                    return Err(bad(
                        "G08",
                        line,
                        "toolResult needs string `content` and bool `isError`",
                    ));
                }
                if !id.is_empty() {
                    if !seen_tools.contains(id) {
                        return Err(bad(
                            "G06",
                            line,
                            format!("{kind} `{id}` matches no earlier toolUse"),
                        ));
                    }
                    if kind == "toolResult" {
                        open_tools.remove(id);
                        if !resulted.insert(id.to_string()) {
                            return Err(bad("G06", line, format!("second toolResult for `{id}`")));
                        }
                    } else if !resulted.contains(id) {
                        return Err(bad(
                            "G06",
                            line,
                            format!("toolImages `{id}` before its toolResult"),
                        ));
                    }
                }
            }
            "result" => {
                if event.get("isError").and_then(Value::as_bool).is_none() {
                    return Err(bad("G08", line, "result needs bool `isError`"));
                }
                if event.get("isError").and_then(Value::as_bool) == Some(true) {
                    interrupted = true;
                }
            }
            "usage" => {
                if saw_usage {
                    return Err(bad("G10", line, "a second usage event"));
                }
                saw_usage = true;
            }
            "activity" => {
                if let (Some(want), Some(got)) = (&run_id, str_of(event, "runId")) {
                    if want != got {
                        return Err(bad(
                            "G09",
                            line,
                            format!("activity runId `{got}` is not the run's `{want}`"),
                        ));
                    }
                }
            }
            "error" | "cancelled" | "paused" => {
                interrupted = true;
                if ended_at.is_none() {
                    ended_at = Some((line, kind.to_string()));
                }
            }
            "done" => done_line = Some(line),
            _ => {}
        }
        // G10: usage closes the turn's content; bookkeeping (a remoteRun notice, the
        // evidence receipt, an ending notice) may still follow it before done.
        if saw_usage && CONTENT.contains(&kind) {
            return Err(bad(
                "G10",
                line,
                format!("`{kind}` after usage; usage closes the turn"),
            ));
        }
    }

    // G03: the stream ends with done.
    if done_line.is_none() {
        return Err(bad(
            "G03",
            last_line,
            "the stream ends without a done event",
        ));
    }
    // G05: everything open closes before the terminal event, unless the turn
    // was stopped, paused, failed or came back as an error result.
    if !interrupted {
        if let Some((id, line)) = open_tools.iter().min_by_key(|(_, l)| **l) {
            return Err(bad(
                "G05",
                *line,
                format!("toolUse `{id}` never got a toolResult before done"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "chat_grammar_tests.rs"]
mod tests;
