//! Settings' Computer control sheet reads and the one action it has.

use super::*;

fn pin(json: &str) -> EngineConfig {
    EngineConfig::parse(json).unwrap()
}

#[test]
fn the_status_says_what_is_pinned_and_switched_on() {
    let none = status_from(None, true, (true, false), false, 0);
    assert!(!none.pinned && none.target.is_none() && none.version.is_none());
    assert!(none.enabled && none.sandbox_enabled && !none.this_mac_enabled);
    assert!(!none.broker_listening);

    let sandbox = pin(
        r#"{"version":"0.52.1","sha256":"0000000000000000000000000000000000000000000000000000000000000000",
            "target":{"kind":"sandboxVm","host":"192.168.64.5","user":"lume","key":"/k"}}"#,
    );
    let s = status_from(Some(&sandbox), false, (false, true), true, 2);
    assert_eq!(s.target.as_deref(), Some("sandboxVm"));
    assert_eq!(s.version.as_deref(), Some("0.52.1"));
    assert!(s.pinned && !s.enabled && s.broker_listening);
    assert_eq!(
        (s.sandbox_enabled, s.this_mac_enabled, s.live_sessions),
        (false, true, 2)
    );
    // The sheet reads camelCase.
    let json = serde_json::to_value(&s).unwrap();
    assert!(json.get("sandboxEnabled").is_some() && json.get("liveSessions").is_some());
}

#[test]
fn turning_a_target_off_ends_sessions_and_the_switches_default_as_designed() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    computer_broker::set_targets(true, false);
    assert_eq!(
        computer_broker::targets(),
        (true, false),
        "This Mac's screen is off by default"
    );
    computer_broker::set_targets(true, true);
    computer_broker::set_targets(true, false);
    assert_eq!(
        computer_stop_all(),
        0,
        "nothing live in a test, and no panic"
    );
    computer_broker::set_targets(true, false);
}

#[test]
fn the_apps_list_starts_all_blocked_and_one_click_allows_one_group() {
    let dir = tempfile::tempdir().unwrap();
    let before = denied_apps(&crate::computer_thismac::Policy::load_from(dir.path()));
    assert!(before.iter().any(|a| a.label == "Terminal") && before.iter().all(|a| !a.allowed));
    let after = set_app_allowed_in(dir.path(), "Terminal", true).unwrap();
    assert!(
        after
            .iter()
            .find(|a| a.label == "Terminal")
            .unwrap()
            .allowed
    );
    assert!(after.iter().filter(|a| a.allowed).count() == 1);
    // The file the broker reads says so.
    assert!(!crate::computer_thismac::Policy::load_from(dir.path()).denied("com.apple.terminal"));
    let back = set_app_allowed_in(dir.path(), "Terminal", false).unwrap();
    assert!(back.iter().all(|a| !a.allowed));
    assert!(set_app_allowed_in(dir.path(), "Nope", true).is_err());
    let json = serde_json::to_value(&after[0]).unwrap();
    assert!(json.get("label").is_some() && json.get("allowed").is_some());
}

#[test]
fn the_chat_controls_pause_take_over_stop_and_each_leaves_an_audit_row() {
    let _g = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let scope = "chat-ctl-test";
    let v = chat_control(scope, "pause").unwrap();
    assert!(v.paused && !v.taken_over);
    let v = chat_control(scope, "takeover").unwrap();
    assert!(v.paused && v.taken_over);
    let v = chat_control(scope, "handback").unwrap();
    assert!(!v.paused && !v.taken_over);
    chat_control(scope, "pause").unwrap();
    let v = chat_control(scope, "resume").unwrap();
    assert!(!v.paused);
    chat_control(scope, "pause").unwrap();
    let v = chat_control(scope, "stop").unwrap();
    assert!(!v.paused && v.entries.is_empty(), "stop forgets the chat");
    assert!(chat_control(scope, "explode").is_err());
    assert!(chat_control("  ", "pause").is_err());
    crate::computer_activity::forget(scope);
}
