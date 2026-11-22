//! AG2 schema tests: one golden fixture per variant, the variant list, the
//! AG1 corpus read with the schema, and the funnel check's refusals.

use super::*;
use crate::ts_bindings::{check_or_write, repo_root};
use serde_json::json;
use std::collections::BTreeSet;

fn s(text: &str) -> String {
    text.to_string()
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

/// One representative envelope per variant, every optional field set.
fn samples() -> Vec<ChatEventEnvelope> {
    let tag = ChatTurnTag {
        client_turn_id: some("ct-1"),
        queue_id: Some(42),
        conversation_key: some("s-20261118-abc"),
    };
    let text = AssistantTextEvent {
        text: s("Hello"),
        parent_tool_use_id: some("toolu_parent"),
    };
    let events = vec![
        ChatEvent::Session(SessionEvent {
            session_id: s("sess-1"),
        }),
        ChatEvent::RunStarted(RunStartedEvent {
            run_id: s("run-1"),
            session_key: some("s-20261118-abc"),
            recall: json!([{ "path": "notes/a.md", "targetPath": "notes/a.md", "subject": "A", "statement": "B", "score": 0.5 }]),
            asked: some("What changed?"),
            steer_support: Some(
                json!({ "agent": "claude", "support": "native", "note": "Steer goes in now." }),
            ),
            permission_mode: some("default"),
            rt: Some(json!({ "sessionKey": "s-20261118-abc", "query": "q", "candidates": [] })),
            unattended: Some(false),
            attended: Some(true),
        }),
        ChatEvent::AssistantText(text.clone()),
        ChatEvent::AssistantDelta(text),
        ChatEvent::TurnBoundary(TurnBoundaryEvent {}),
        ChatEvent::ToolUse(ToolUseEvent {
            id: some("toolu_1"),
            name: s("Read"),
            input: json!({ "file_path": "/tmp/a" }),
            parent_tool_use_id: some("toolu_parent"),
        }),
        ChatEvent::ToolResult(ToolResultEvent {
            id: s("toolu_1"),
            content: s("ok"),
            is_error: false,
            parent_tool_use_id: some("toolu_parent"),
        }),
        ChatEvent::ToolImages(ToolImagesEvent {
            id: s("toolu_1"),
            images: vec![s("data:image/png;base64,AA==")],
            parent_tool_use_id: some("toolu_parent"),
        }),
        ChatEvent::EvidenceReceipt(EvidenceReceiptEvent {
            receipt: json!({ "runId": "run-1", "files": [] }),
        }),
        ChatEvent::Result(ResultEvent {
            result: some("Done."),
            cost_usd: Some(0.25),
            is_error: false,
            subtype: s("success"),
            session_id: some("sess-1"),
            result_kind: some("answer"),
        }),
        ChatEvent::Usage(UsageEvent {
            usage: json!({ "inputTokens": 10, "outputTokens": 5 }),
            session_key: some("s-20261118-abc"),
        }),
        ChatEvent::Activity(ActivityEvent {
            run_id: s("run-1"),
            session_key: some("s-20261118-abc"),
            last_event_at: some("2026-11-18T10:00:00Z"),
            progress: json!("working"),
        }),
        ChatEvent::BudgetWarning(BudgetWarningEvent {
            session_key: s("s-20261118-abc"),
            notice: json!({ "level": "warn" }),
        }),
        ChatEvent::LoginLow(SessionMessageEvent {
            session_key: s("s-20261118-abc"),
            message: s("Claude login is low."),
        }),
        ChatEvent::Panel(PanelEvent {
            session_key: s("s-20261118-abc"),
            panel: s("hollow:context"),
            snapshot: json!({ "message": "Started fresh." }),
        }),
        ChatEvent::Interaction(InteractionEvent {
            interaction: json!({ "id": "q-1", "status": "open" }),
            session_key: some("s-20261118-abc"),
        }),
        ChatEvent::Cancelled(CancelledEvent {
            stop: some("verified"),
            queued: Some(false),
            before_spawn: Some(false),
        }),
        ChatEvent::Paused(PausedEvent {
            session_key: s("s-20261118-abc"),
            note: s("Paused."),
            at: 1_763_460_000,
            reason: some("all"),
            resume_at: Some(1_763_463_600),
        }),
        ChatEvent::SessionReset(SessionResetEvent {
            reason: s("configChanged"),
            agent: s("claude"),
            session_key: some("s-20261118-abc"),
            changed: Some(vec![s("model")]),
            away: some("workspace"),
        }),
        ChatEvent::AccountSwitched(AccountSwitchedEvent {
            from: s("Work"),
            to: s("Personal"),
            reason: some("limit"),
            session_key: some("s-20261118-abc"),
        }),
        ChatEvent::RemoteRun(
            json!({ "phase": "start", "host": "runner-host", "sessionKey": "s-20261118-abc" }),
        ),
        ChatEvent::Error(ErrorEvent {
            message: s("It failed."),
            result_kind: some("error"),
        }),
        ChatEvent::Done(DoneEvent {
            session_id: some("sess-1"),
            session_key: some("s-20261118-abc"),
            recovered: Some(false),
        }),
        ChatEvent::Steer(SteerEvent {
            session_key: s("s-20261118-abc"),
            steer: json!({ "id": "st-1", "text": "also", "at": "2026-11-18T10:00:00Z", "afterTools": 1, "state": "sent" }),
        }),
        ChatEvent::HeldSent(HeldSentEvent {
            held_id: s("h-1"),
            after: s("queue"),
            session_key: s("s-20261118-abc"),
        }),
        ChatEvent::AgentContinued(AgentContinuedEvent {
            from: s("claude"),
            to: s("codex"),
            prompt: s("Go on."),
            session_key: s("s-20261118-abc"),
        }),
        ChatEvent::HeldFailed(HeldFailedEvent {
            held_id: s("h-1"),
            error: s("The queue was closed."),
            prompt: s("Later."),
            session_key: s("s-20261118-abc"),
        }),
    ];
    events
        .into_iter()
        .map(|event| ChatEventEnvelope {
            tag: tag.clone(),
            event,
        })
        .collect()
}

fn type_of(envelope: &ChatEventEnvelope) -> String {
    serde_json::to_value(envelope).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn every_variant_has_a_golden_fixture_and_round_trips() {
    let samples = samples();
    let types: Vec<String> = samples.iter().map(type_of).collect();
    assert_eq!(
        types,
        TYPES.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
        "one sample per variant, in order"
    );
    let dir = repo_root().join("tests/fixtures/chat-events");
    for (sample, kind) in samples.iter().zip(&types) {
        let value = serde_json::to_value(sample).unwrap();
        assert_eq!(conformance_error(&value), None, "{kind}");
        let back: ChatEventEnvelope = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(&back, sample, "{kind} round-trips");
        let mut text = serde_json::to_string_pretty(&value).unwrap();
        text.push('\n');
        check_or_write(&dir.join(format!("{kind}.json")), &text);
    }
    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            e.ok()?
                .file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(str::to_string)
        })
        .collect();
    assert_eq!(
        on_disk,
        types.into_iter().collect(),
        "no stale fixture in tests/fixtures/chat-events"
    );
}

/// Every stream the AG1 corpus expects to pass is in the schema, line by line.
#[test]
fn passing_corpus_streams_conform() {
    let dir = repo_root().join("tests/fixtures/chat-streams");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let mut checked = 0;
    for stream in manifest["streams"].as_array().unwrap() {
        if stream["expect"] != "pass" {
            continue;
        }
        let file = stream["file"].as_str().unwrap();
        for (n, line) in std::fs::read_to_string(dir.join(file))
            .unwrap()
            .lines()
            .enumerate()
        {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(conformance_error(&value), None, "{file}:{}", n + 1);
            checked += 1;
        }
    }
    assert!(checked > 20, "the corpus was read ({checked} lines)");
}

#[test]
fn an_unknown_type_or_field_does_not_conform() {
    assert!(conformance_error(&json!({ "type": "mystery" }))
        .unwrap()
        .starts_with("does not parse"));
    let extra = json!({ "type": "session", "sessionId": "s", "surprise": 1 });
    assert!(conformance_error(&extra).unwrap().contains("surprise"));
    // A held send's outcome and a continued turn name their queue row.
    let held = json!({ "type": "heldSent", "heldId": "h", "after": "queue", "sessionKey": "s" });
    assert!(conformance_error(&held).unwrap().contains("queueId"));
    let mut held = held;
    held["queueId"] = json!(3);
    assert_eq!(conformance_error(&held), None);
    // A required field missing.
    assert!(conformance_error(&json!({ "type": "toolResult", "id": "t" })).is_some());
    // An explicit null reads as an absent optional.
    let nulls =
        json!({ "type": "done", "sessionId": null, "sessionKey": null, "clientTurnId": null });
    assert_eq!(conformance_error(&nulls), None);
}

/// The typed builders write the same bytes the `json!` sites they replaced
/// did (AG2 follow-up), with the optional fields both present and empty.
#[test]
fn builders_write_the_old_bytes() {
    use super::build;
    let same = |old: Value, new: Value| {
        assert_eq!(old.to_string(), new.to_string());
        assert_eq!(conformance_error(&new), None, "{new}");
    };
    same(
        json!({ "type": "session", "sessionId": "s1" }),
        build::session("s1"),
    );
    same(json!({ "type": "turnBoundary" }), build::turn_boundary());
    same(
        json!({ "type": "assistantDelta", "text": "a" }),
        build::assistant_delta("a"),
    );
    same(
        json!({ "type": "assistantText", "text": "a" }),
        build::assistant_text("a"),
    );
    same(
        json!({ "type": "toolUse", "id": "t1", "name": "Read", "input": { "p": 1 } }),
        build::tool_use("t1", "Read", json!({ "p": 1 })),
    );
    same(
        json!({ "type": "toolUse", "id": "", "name": "Read", "input": null }),
        build::tool_use("", "Read", Value::Null),
    );
    same(
        json!({ "type": "toolResult", "id": "t1", "content": "ok", "isError": true }),
        build::tool_result("t1", "ok", true),
    );
    same(
        json!({ "type": "toolImages", "id": "t1", "images": ["data:a"] }),
        build::tool_images("t1", vec!["data:a".into()]),
    );
    same(
        json!({ "type": "evidenceReceipt", "receipt": { "a": 1 } }),
        build::evidence_receipt(json!({ "a": 1 })),
    );
    for (text, cost, sid) in [(Some("r"), Some(0.5), Some("s")), (None, None, None)] {
        same(
            json!({ "type": "result", "result": text, "costUsd": cost, "isError": false,
                "subtype": "success", "resultKind": "ok", "sessionId": sid }),
            build::result(
                text.map(str::to_string),
                cost,
                false,
                "success",
                "ok",
                sid.map(str::to_string),
            ),
        );
    }
    same(
        json!({ "type": "usage", "usage": { "n": 1 }, "sessionKey": "k" }),
        build::usage(json!({ "n": 1 }), "k"),
    );
    same(
        json!({ "type": "error", "message": "m" }),
        build::error("m"),
    );
    same(
        json!({ "type": "error", "message": "m", "resultKind": "limit" }),
        build::error_kind("m", "limit"),
    );
    same(
        json!({ "type": "budgetWarning", "sessionKey": "k", "notice": { "w": 1 } }),
        build::budget_warning("k", json!({ "w": 1 })),
    );
    same(
        json!({ "type": "loginLow", "message": "m", "sessionKey": "k" }),
        build::login_low("k", "m"),
    );
    same(
        json!({ "type": "panel", "sessionKey": "k", "panel": "hollow:context",
            "snapshot": { "message": "m" } }),
        build::panel("k", "hollow:context", json!({ "message": "m" })),
    );
    same(
        json!({ "type": "cancelled", "stop": "verified" }),
        build::cancelled(Some("verified"), false),
    );
    same(
        json!({ "type": "cancelled", "stop": Value::Null }),
        build::cancelled(None, false),
    );
    same(
        json!({ "type": "cancelled", "stop": Value::Null, "beforeSpawn": true }),
        build::cancelled(None, true),
    );
    same(
        json!({ "type": "paused", "sessionKey": "k", "note": "n", "at": 5,
            "reason": "user", "resumeAt": Value::Null }),
        build::paused("k", "n", 5, "user"),
    );
    same(
        json!({ "type": "activity", "runId": "r", "sessionKey": "k",
            "lastEventAt": "t", "progress": { "p": 1 } }),
        build::activity("r", "k", Some("t".into()), json!({ "p": 1 })),
    );
    same(
        json!({ "type": "activity", "runId": "r", "sessionKey": "k",
            "lastEventAt": Value::Null, "progress": Value::Null }),
        build::activity("r", "k", None, Value::Null),
    );
    same(
        json!({ "type": "accountSwitched", "from": "a", "to": "b", "reason": "limit",
            "sessionKey": "k" }),
        build::account_switched("a", "b", "limit", "k"),
    );
    same(
        json!({ "type": "sessionReset", "reason": "x", "agent": "claude", "sessionKey": "k" }),
        build::session_reset("x", "claude", "k", None, None),
    );
    same(
        json!({ "type": "sessionReset", "reason": "x", "agent": "claude", "sessionKey": "k",
            "changed": ["a"], "away": "3 h" }),
        build::session_reset(
            "x",
            "claude",
            "k",
            Some(vec!["a".into()]),
            Some("3 h".into()),
        ),
    );
    // runStarted: nulls stay explicit when a field is empty.
    same(
        json!({ "type": "runStarted", "steerSupport": { "a": 1 }, "runId": "r",
            "sessionKey": "k", "recall": [1], "rt": { "q": 1 }, "asked": "q",
            "permissionMode": "default", "unattended": false, "attended": true }),
        build::run_started(
            "r",
            "k",
            json!([1]),
            Some("q".into()),
            Some(json!({ "a": 1 })),
            Some("default".into()),
            Some(json!({ "q": 1 })),
            Some(false),
            Some(true),
        ),
    );
    same(
        json!({ "type": "runStarted", "steerSupport": Value::Null, "runId": "r",
            "sessionKey": "k", "recall": [], "rt": Value::Null, "asked": Value::Null,
            "permissionMode": Value::Null, "unattended": Value::Null, "attended": Value::Null }),
        build::run_started(
            "r",
            "k",
            json!([]),
            None,
            None::<Value>,
            None,
            None::<Value>,
            None,
            None,
        ),
    );
}

/// The second batch (`chat_queue.rs`, `interactions.rs`, `steering.rs`,
/// `commands.rs`): same bytes as the `json!` sites they replaced.
#[test]
fn queue_and_held_builders_write_the_old_bytes() {
    use super::build;
    let same = |old: Value, new: Value| {
        assert_eq!(old.to_string(), new.to_string());
        assert_eq!(
            conformance_error(&new).filter(|e| !e.contains("without")),
            None,
            "{new}"
        );
    };
    same(
        json!({ "type": "done", "sessionId": "n", "sessionKey": "k" }),
        build::done(Some("n".into()), Some("k")),
    );
    same(
        json!({ "type": "done", "sessionId": Value::Null, "sessionKey": "k" }),
        build::done(None, Some("k")),
    );
    same(
        json!({ "type": "done", "sessionId": Value::Null, "sessionKey": Value::Null }),
        build::done(None, None),
    );
    same(
        json!({ "type": "done", "recovered": true }),
        build::done_recovered(),
    );
    same(
        json!({ "type": "cancelled", "stop": Value::Null, "queued": true }),
        build::cancelled_queued(),
    );
    same(
        json!({ "type": "interaction", "interaction": { "id": "i" }, "sessionKey": "k" }),
        build::interaction(json!({ "id": "i" }), "k"),
    );
    same(
        json!({ "type": "steer", "sessionKey": "k", "steer": { "id": "s" } }),
        build::steer("k", json!({ "id": "s" })),
    );
    same(
        json!({ "type": "heldFailed", "heldId": "h", "error": "e", "prompt": "p",
            "sessionKey": "k", "conversationKey": "k" }),
        build::held_failed("h", "e", "p", "k", "k"),
    );
    for turn in [Some("c"), None] {
        same(
            json!({ "type": "heldSent", "heldId": "h", "after": "queue", "sessionKey": "k2",
                "queueId": 7, "clientTurnId": turn, "conversationKey": "k" }),
            build::held_sent(
                "h",
                crate::steering::After::Queue,
                "k2",
                7,
                turn.map(str::to_string),
                "k",
            ),
        );
    }
    same(
        json!({ "type": "agentContinued", "from": "a", "to": "b", "prompt": "p",
            "sessionKey": "k2", "queueId": 7, "clientTurnId": "c", "conversationKey": "k" }),
        build::agent_continued("a", "b", "p", "k2", 7, "c", "k"),
    );
}
