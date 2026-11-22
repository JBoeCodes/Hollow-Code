//! AG9: every AG1 corpus stream translates to a valid AG-UI sequence, and
//! the mapping is pinned on one stream.

use super::*;
use std::collections::HashSet;
use std::path::Path;

/// AG-UI's ordering and field rules as the published event list states them
/// (our own reading): one `RUN_STARTED` first, one terminal (`RUN_FINISHED` or
/// `RUN_ERROR`) last; text messages start, get non-empty content, end, and
/// none is open at the end; tool calls start once per id, args and end match
/// an open call, a result follows its call's end; required fields present.
fn check(events: &[Value]) -> Result<(), String> {
    let kind = |e: &Value| e["type"].as_str().unwrap_or("").to_string();
    let field = |e: &Value, k: &str| -> Result<String, String> {
        e.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("{} without {k}: {e}", kind(e)))
    };
    let (first, last) = (events.first().ok_or("no events")?, events.last().unwrap());
    if kind(first) != "RUN_STARTED" {
        return Err(format!("first is {}", kind(first)));
    }
    if !matches!(kind(last).as_str(), "RUN_FINISHED" | "RUN_ERROR") {
        return Err(format!("last is {}", kind(last)));
    }
    let run_id = field(first, "runId")?;
    field(first, "threadId")?;
    let (mut open_text, mut seen_text): (Option<String>, HashSet<String>) = (None, HashSet::new());
    let (mut open_tool, mut started, mut ended, mut results) = (
        None::<String>,
        HashSet::new(),
        HashSet::new(),
        HashSet::new(),
    );
    for (i, e) in events.iter().enumerate() {
        let k = kind(e);
        let at = |m: String| format!("event {}: {m}", i + 1);
        match k.as_str() {
            "RUN_STARTED" if i > 0 => return Err(at("a second RUN_STARTED".into())),
            "RUN_STARTED" => {}
            "RUN_FINISHED" | "RUN_ERROR" => {
                if i + 1 != events.len() {
                    return Err(at(format!("{k} is not last")));
                }
                if open_text.is_some() || open_tool.is_some() {
                    return Err(at("a message or tool call is still open".into()));
                }
                if k == "RUN_FINISHED" && field(e, "runId")? != run_id {
                    return Err(at("RUN_FINISHED names another run".into()));
                }
                if k == "RUN_ERROR" {
                    field(e, "message")?;
                }
            }
            "TEXT_MESSAGE_START" => {
                let id = field(e, "messageId")?;
                if open_text.is_some() || open_tool.is_some() || !seen_text.insert(id.clone()) {
                    return Err(at(format!(
                        "message {id} starts while another is open or again"
                    )));
                }
                if e["role"] != "assistant" {
                    return Err(at("role is not assistant".into()));
                }
                open_text = Some(id);
            }
            "TEXT_MESSAGE_CONTENT" => {
                if open_text.as_deref() != Some(field(e, "messageId")?.as_str()) {
                    return Err(at("content for a message not open".into()));
                }
                field(e, "delta")?;
            }
            "TEXT_MESSAGE_END" => {
                if open_text.take().as_deref() != Some(field(e, "messageId")?.as_str()) {
                    return Err(at("end for a message not open".into()));
                }
            }
            "TOOL_CALL_START" => {
                let id = field(e, "toolCallId")?;
                field(e, "toolCallName")?;
                if open_text.is_some() || open_tool.is_some() || !started.insert(id.clone()) {
                    return Err(at(format!("tool call {id} starts while open or again")));
                }
                open_tool = Some(id);
            }
            "TOOL_CALL_ARGS" => {
                if open_tool.as_deref() != Some(field(e, "toolCallId")?.as_str()) {
                    return Err(at("args for a call not open".into()));
                }
                field(e, "delta")?;
            }
            "TOOL_CALL_END" => {
                let id = field(e, "toolCallId")?;
                if open_tool.take().as_deref() != Some(id.as_str()) {
                    return Err(at("end for a call not open".into()));
                }
                ended.insert(id);
            }
            "TOOL_CALL_RESULT" => {
                let id = field(e, "toolCallId")?;
                field(e, "messageId")?;
                if !ended.contains(&id) || !results.insert(id.clone()) || e.get("content").is_none()
                {
                    return Err(at(format!(
                        "result for {id} before its end, twice or without content"
                    )));
                }
            }
            "CUSTOM" => {
                field(e, "name")?;
            }
            other => return Err(at(format!("unexpected type {other}"))),
        }
    }
    Ok(())
}

fn corpus() -> Vec<(String, Vec<Value>)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/chat-streams");
    let mut streams: Vec<(String, Vec<Value>)> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let path = e.ok()?.path();
            (path.extension()? == "jsonl").then_some(path)
        })
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            // A line that is not JSON (the G01 fixture) never reaches a client.
            let events = text
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                events,
            )
        })
        .collect();
    streams.sort_by(|a, b| a.0.cmp(&b.0));
    streams
}

/// Every corpus stream, passing and failing alike: the translator keeps the
/// AG-UI sequence valid even when Hollow's own stream breaks a G rule.
#[test]
fn every_corpus_stream_translates_to_a_valid_ag_ui_sequence() {
    let streams = corpus();
    assert!(
        streams.len() >= 20,
        "the corpus was read ({})",
        streams.len()
    );
    for (file, events) in &streams {
        let out = translate("s1", events);
        if let Err(error) = check(&out) {
            panic!(
                "{file}: {error}\n{}",
                serde_json::to_string_pretty(&out).unwrap()
            );
        }
    }
}

#[test]
fn a_text_and_tool_turn_maps_event_for_event() {
    let (_, events) = corpus()
        .into_iter()
        .find(|(f, _)| f == "claude-text-tool.jsonl")
        .unwrap();
    let out = translate("s1", &events);
    let kinds: Vec<&str> = out.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "RUN_STARTED",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "TOOL_CALL_START",
            "TOOL_CALL_ARGS",
            "TOOL_CALL_END",
            "TOOL_CALL_RESULT",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED",
        ]
    );
    assert_eq!(
        out[0],
        json!({ "type": "RUN_STARTED", "threadId": "s1", "runId": "r1" })
    );
    assert_eq!(out[2]["delta"], "Looking");
    assert_eq!(out[6]["delta"], r#"{"file_path":"a.md"}"#);
    assert_eq!(out[8]["content"], "ok");
    assert_eq!(out.last().unwrap()["result"], "Done.");
}

/// Hollow's own state never crosses: its events produce nothing, and an
/// error, pause or error result ends the run with a fixed message only.
#[test]
fn hollow_state_and_error_text_never_cross() {
    let secret = "Budget limit reached: Work account ($40) at /Users/someone/vault";
    let quiet = [
        json!({ "type": "session", "sessionId": "c-1" }),
        json!({ "type": "usage", "usage": { "costUsd": 1.5 }, "sessionKey": "s1" }),
        json!({ "type": "activity", "runId": "r1", "lastEventAt": null, "progress": null }),
        json!({ "type": "budgetWarning", "sessionKey": "s1", "notice": { "message": secret } }),
        json!({ "type": "evidenceReceipt", "receipt": { "files": [secret] } }),
    ];
    for ending in [
        json!({ "type": "error", "message": secret }),
        json!({ "type": "paused", "sessionKey": "s1", "note": secret, "at": 1 }),
        json!({ "type": "result", "result": secret, "costUsd": null, "isError": true, "subtype": "error", "sessionId": null }),
    ] {
        let mut events = vec![json!({ "type": "runStarted", "runId": "r1" })];
        events.extend(quiet.iter().cloned());
        events.push(ending);
        events.push(json!({ "type": "done", "sessionId": null }));
        let out = translate("s1", &events);
        assert!(check(&out).is_ok(), "{out:?}");
        assert_eq!(out.len(), 2, "only RUN_STARTED and RUN_ERROR: {out:?}");
        let text = serde_json::to_string(&out).unwrap();
        assert!(
            !text.contains("Budget") && !text.contains("/Users"),
            "{text}"
        );
    }
}

#[test]
fn a_queued_cancel_takes_its_send_id_as_run_id() {
    let events = [
        json!({ "type": "cancelled", "queued": true, "clientTurnId": "ct-9" }),
        json!({ "type": "done" }),
    ];
    let out = translate("s1", &events);
    assert_eq!(out[0]["runId"], "unstarted-ct-9");
    assert_eq!(out[1]["code"], "cancelled");
    assert_eq!(out.len(), 2);
}
