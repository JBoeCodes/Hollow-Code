//! The chat card's state (CU8): bounded, per chat, never persisted.

use super::*;
use serde_json::json;

fn entry(tool: &str) -> Entry {
    Entry {
        id: 0,
        at_ms: 0,
        tool: tool.into(),
        tier: "act".into(),
        target: "sandboxVm".into(),
        outcome: "ok".into(),
        detail: "ran".into(),
        app: None,
        has_frame: false,
    }
}

fn frame(n: u8) -> Option<String> {
    Some(format!("data:image/png;base64,AAA{n}"))
}

#[test]
fn entries_come_back_newest_first_and_only_the_newest_few_keep_a_frame() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let scope = "act-t1";
    for i in 0..8u8 {
        record(scope, entry(&format!("click{i}")), frame(i));
    }
    let v = view(scope);
    assert_eq!(v.entries.len(), 8);
    assert_eq!(v.entries[0].tool, "click7", "newest first");
    let with: Vec<_> = v
        .entries
        .iter()
        .filter(|e| e.has_frame)
        .map(|e| e.tool.clone())
        .collect();
    assert_eq!(with.len(), MAX_THUMBS, "{with:?}");
    assert!(with.contains(&"click7".to_string()) && !with.contains(&"click0".to_string()));
    assert_eq!(v.frame_id, Some(v.entries[0].id));
    assert_eq!(
        frame_of(scope, None).as_deref(),
        Some("data:image/png;base64,AAA7")
    );
    assert_eq!(
        frame_of(scope, Some(v.entries[7].id)),
        None,
        "an old frame is gone"
    );
    assert!(!v.paused && !v.taken_over);
}

fn frame_of(scope: &str, id: Option<u64>) -> Option<String> {
    super::frame(scope, id)
}

#[test]
fn an_entry_list_is_bounded_and_a_scope_is_remembered_separately() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    for i in 0..(MAX_ENTRIES + 10) {
        record("act-t2", entry(&format!("t{i}")), None);
    }
    assert_eq!(view("act-t2").entries.len(), MAX_ENTRIES);
    assert!(view("act-t3").entries.is_empty());
    record("", entry("x"), None);
    assert!(view("").entries.is_empty(), "no chat, no state");
}

#[test]
fn pause_take_over_and_forget() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let scope = "act-t4";
    assert_eq!(pause_state(scope), None);
    set_paused(scope, true, false);
    assert_eq!(pause_state(scope), Some(false));
    set_paused(scope, true, true);
    assert_eq!(pause_state(scope), Some(true));
    assert!(view(scope).paused && view(scope).taken_over);
    set_paused(scope, false, true);
    assert_eq!(pause_state(scope), None, "resuming clears take over too");
    set_paused(scope, true, false);
    forget(scope);
    assert_eq!(pause_state(scope), None);
    set_paused("act-t5", true, false);
    clear();
    assert_eq!(pause_state("act-t5"), None);
}

#[test]
fn the_first_image_of_a_result_becomes_a_data_url_and_odd_types_are_png() {
    let r = json!({ "content": [
        { "type": "text", "text": "x" },
        { "type": "image", "data": "QUJD", "mimeType": "image/jpeg" },
        { "type": "image", "data": "ZZZ", "mimeType": "image/png" },
    ] });
    assert_eq!(
        first_frame(&r).as_deref(),
        Some("data:image/jpeg;base64,QUJD")
    );
    let odd = json!({ "content": [{ "type": "image", "data": "QUJD", "mimeType": "text/html" }] });
    assert_eq!(
        first_frame(&odd).as_deref(),
        Some("data:image/png;base64,QUJD")
    );
    assert_eq!(first_frame(&json!({ "content": [] })), None);
    assert_eq!(first_frame(&json!({})), None);
}

#[test]
fn a_chat_the_person_holds_is_never_dropped_to_make_room() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    clear();
    set_paused("held-chat", true, false);
    for i in 0..(MAX_CHATS + 5) {
        record(&format!("busy-{i}"), entry("click"), None);
    }
    assert_eq!(
        pause_state("held-chat"),
        Some(false),
        "still held after 37 other chats"
    );
    clear();
}
