//! AG9: a pure translator from one turn's `chat-event` stream to AG-UI
//! events, so an outside AG-UI client could follow a Hollow chat. Written
//! from AG-UI's published event list (docs.ag-ui.com, "Events"); no AG-UI
//! code was copied. Rules and the mapping table:
//! `docs/reference/chat-stream-grammar.md`, "AG-UI translation (AG9)".
//!
//! Only conversation content crosses: the run's start and end, text, tool
//! calls and their results, and a card's id, mode and status. A run's end
//! carries a fixed message per code, never Hollow's own error, pause or
//! limit text (it can name budgets, accounts or paths). Hollow's own state
//! (usage, receipts, recall, budgets, accounts, the runner, steering, held
//! sends) never does. There is no HTTP route yet: the row's second step
//! (a `POST` returning SSE behind the phone API's bearer token and tailnet
//! check) waits until a client needs it. Tests only until then.

use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// One turn's translation state. Feed it the turn's events in order with
/// [`Translator::push`], then [`Translator::finish`].
pub(crate) struct Translator {
    thread_id: String,
    run_id: Option<String>,
    /// The assistant message open now (between deltas and the next boundary).
    open_message: Option<String>,
    messages: usize,
    /// Tool call ids started so far (a repeated id is dropped).
    tools: HashSet<String>,
    /// Ids whose result went out (a second result is dropped).
    results: HashSet<String>,
    unnamed_tools: usize,
    /// The agent's result text, for `RUN_FINISHED.result`.
    result: Option<String>,
    /// `RUN_FINISHED` or `RUN_ERROR` went out: nothing more does.
    ended: bool,
}

fn event(kind: &str, fields: Value) -> Value {
    let mut out = Map::new();
    out.insert("type".into(), json!(kind));
    if let Value::Object(fields) = fields {
        out.extend(fields);
    }
    Value::Object(out)
}

fn text(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).map(str::to_string)
}

impl Translator {
    /// `thread_id` is the conversation (Hollow's session key).
    pub(crate) fn new(thread_id: &str) -> Translator {
        Translator {
            thread_id: thread_id.to_string(),
            run_id: None,
            open_message: None,
            messages: 0,
            tools: HashSet::new(),
            results: HashSet::new(),
            unnamed_tools: 0,
            result: None,
            ended: false,
        }
    }

    /// `RUN_STARTED` once, before anything else. A turn that never started
    /// (taken out of the queue) has no `runStarted`; it gets
    /// `unstarted-<clientTurnId>`, or `unstarted` without one.
    fn ensure_started(&mut self, run_id: Option<String>, out: &mut Vec<Value>) {
        if self.run_id.is_some() {
            return;
        }
        let run_id = run_id.unwrap_or_else(|| "unstarted".to_string());
        out.push(event(
            "RUN_STARTED",
            json!({ "threadId": self.thread_id, "runId": run_id }),
        ));
        self.run_id = Some(run_id);
    }

    fn close_message(&mut self, out: &mut Vec<Value>) {
        if let Some(id) = self.open_message.take() {
            out.push(event("TEXT_MESSAGE_END", json!({ "messageId": id })));
        }
    }

    fn content(&mut self, delta: &str, out: &mut Vec<Value>) {
        if delta.is_empty() {
            return;
        }
        let id = match &self.open_message {
            Some(id) => id.clone(),
            None => {
                self.messages += 1;
                let id = format!(
                    "{}-m{}",
                    self.run_id.as_deref().unwrap_or("run"),
                    self.messages
                );
                out.push(event(
                    "TEXT_MESSAGE_START",
                    json!({ "messageId": id, "role": "assistant" }),
                ));
                self.open_message = Some(id.clone());
                id
            }
        };
        out.push(event(
            "TEXT_MESSAGE_CONTENT",
            json!({ "messageId": id, "delta": delta }),
        ));
    }

    fn end(&mut self, terminal: Value, out: &mut Vec<Value>) {
        self.close_message(out);
        out.push(terminal);
        self.ended = true;
    }

    fn run_error(&self, message: &str, code: Option<&str>) -> Value {
        let mut fields = json!({ "message": message });
        if let Some(code) = code {
            fields["code"] = json!(code);
        }
        event("RUN_ERROR", fields)
    }

    /// The AG-UI events for one `chat-event` payload (often none).
    pub(crate) fn push(&mut self, payload: &Value) -> Vec<Value> {
        let mut out = Vec::new();
        if self.ended {
            return out;
        }
        let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "runStarted" {
            self.ensure_started(text(payload, "runId").filter(|id| !id.is_empty()), &mut out);
            return out;
        }
        // A turn that never started has no run id; its send's id stands in.
        let unstarted = text(payload, "clientTurnId").map(|id| format!("unstarted-{id}"));
        // Hollow-only events never start a run or cross.
        let crosses = [
            "assistantDelta",
            "assistantText",
            "turnBoundary",
            "toolUse",
            "toolResult",
            "toolImages",
            "interaction",
            "result",
            "error",
            "cancelled",
            "paused",
            "done",
        ];
        if !crosses.contains(&kind) {
            return out;
        }
        self.ensure_started(unstarted, &mut out);
        match kind {
            "assistantDelta" => {
                if let Some(delta) = text(payload, "text") {
                    self.content(&delta, &mut out);
                }
            }
            // A whole block (no deltas streamed it): a message of its own.
            "assistantText" => {
                self.close_message(&mut out);
                if let Some(block) = text(payload, "text") {
                    self.content(&block, &mut out);
                }
                self.close_message(&mut out);
            }
            "turnBoundary" => self.close_message(&mut out),
            "toolUse" => {
                self.close_message(&mut out);
                let id = match text(payload, "id").filter(|id| !id.is_empty()) {
                    Some(id) => id,
                    None => {
                        self.unnamed_tools += 1;
                        format!(
                            "{}-t{}",
                            self.run_id.as_deref().unwrap_or("run"),
                            self.unnamed_tools
                        )
                    }
                };
                if !self.tools.insert(id.clone()) {
                    return out;
                }
                let name = text(payload, "name")
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "tool".to_string());
                out.push(event(
                    "TOOL_CALL_START",
                    json!({ "toolCallId": id, "toolCallName": name }),
                ));
                let args = payload.get("input").cloned().unwrap_or(Value::Null);
                let args = if args.is_null() {
                    "{}".to_string()
                } else {
                    args.to_string()
                };
                out.push(event(
                    "TOOL_CALL_ARGS",
                    json!({ "toolCallId": id, "delta": args }),
                ));
                out.push(event("TOOL_CALL_END", json!({ "toolCallId": id })));
            }
            "toolResult" => {
                let Some(id) = text(payload, "id") else {
                    return out;
                };
                if !self.tools.contains(&id) || !self.results.insert(id.clone()) {
                    return out;
                }
                self.close_message(&mut out);
                let content = match payload.get("content") {
                    Some(Value::String(content)) => content.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                };
                out.push(event(
                    "TOOL_CALL_RESULT",
                    json!({ "messageId": format!("{id}-result"), "toolCallId": id, "content": content, "role": "tool" }),
                ));
            }
            // Images stay on this Mac; the client learns how many came back.
            "toolImages" => {
                let Some(id) = text(payload, "id") else {
                    return out;
                };
                if !self.results.contains(&id) {
                    return out;
                }
                let count = payload
                    .get("images")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                out.push(event("CUSTOM", json!({ "name": "hollow.toolImages", "value": { "toolCallId": id, "count": count } })));
            }
            // A question or approval card: its id, mode and status only.
            "interaction" => {
                let card = payload.get("interaction").cloned().unwrap_or(Value::Null);
                let pick = |key: &str| card.get(key).cloned().unwrap_or(Value::Null);
                out.push(event(
                    "CUSTOM",
                    json!({ "name": "hollow.interaction", "value": { "id": pick("id"), "mode": pick("mode"), "status": pick("status") } }),
                ));
            }
            "result" => {
                if payload.get("isError").and_then(Value::as_bool) == Some(true) {
                    let error = self.run_error("The agent reported an error.", Some("agent_error"));
                    self.end(error, &mut out);
                } else {
                    self.result = text(payload, "result");
                }
            }
            "error" => {
                let error = self.run_error("The turn failed.", Some("error"));
                self.end(error, &mut out);
            }
            "cancelled" => {
                let error = self.run_error("The turn was stopped.", Some("cancelled"));
                self.end(error, &mut out);
            }
            "paused" => {
                let error = self.run_error("The turn paused.", Some("paused"));
                self.end(error, &mut out);
            }
            "done" => self.finish_into(&mut out),
            _ => {}
        }
        out
    }

    fn finish_into(&mut self, out: &mut Vec<Value>) {
        if self.ended {
            return;
        }
        self.ensure_started(None, out);
        let mut fields = json!({ "threadId": self.thread_id, "runId": self.run_id });
        if let Some(result) = self.result.take() {
            fields["result"] = json!(result);
        }
        self.end(event("RUN_FINISHED", fields), out);
    }

    /// Closes the run when the stream ended without `done` (a dropped feed).
    pub(crate) fn finish(&mut self) -> Vec<Value> {
        let mut out = Vec::new();
        self.finish_into(&mut out);
        out
    }
}

/// One whole turn's AG-UI events.
pub(crate) fn translate(thread_id: &str, events: &[Value]) -> Vec<Value> {
    let mut translator = Translator::new(thread_id);
    let mut out: Vec<Value> = events.iter().flat_map(|e| translator.push(e)).collect();
    out.extend(translator.finish());
    out
}

#[cfg(test)]
#[path = "ag_ui_tests.rs"]
mod ag_ui_tests;
