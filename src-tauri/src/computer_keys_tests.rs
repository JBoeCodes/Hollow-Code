//! Blocked hotkeys (OC3).

use super::*;
use serde_json::json;

#[test]
fn the_session_ending_combinations_are_blocked_in_any_spelling() {
    let cases = [
        (json!({ "keys": "cmd+shift+q" }), "logs out the session"),
        (
            json!({ "keys": ["command", "shift", "Q"] }),
            "logs out the session",
        ),
        (
            json!({ "key": "q", "modifiers": ["cmd", "ctrl"] }),
            "locks the screen",
        ),
        (json!({ "combo": "Control+Command+Q" }), "locks the screen"),
        (
            json!({ "key": "escape", "modifiers": ["command", "option"] }),
            "opens Force Quit",
        ),
        (json!({ "hotkey": "cmd+opt+esc" }), "opens Force Quit"),
        (
            json!({ "keys": "cmd+shift+backspace" }),
            "empties the Trash",
        ),
        (
            json!({ "key": "q", "modifiers": "cmd" }),
            "quits the front app",
        ),
        (
            json!({ "keys": "ctrl+cmd+eject" }),
            "shuts down or restarts",
        ),
    ];
    for (args, why) in cases {
        assert_eq!(blocked_hotkey("hotkey", &args), Some(why), "{args}");
        assert_eq!(blocked_hotkey("press_key", &args), Some(why), "{args}");
    }
}

#[test]
fn ordinary_keys_and_other_tools_are_not_blocked() {
    for args in [
        json!({ "keys": "cmd+c" }),
        json!({ "keys": "cmd+shift+4" }),
        json!({ "key": "return" }),
        json!({ "keys": ["cmd", "w"] }),
        json!({ "key": "q" }),
        json!({ "keys": "shift+q" }),
        json!({}),
        json!({ "keys": 7 }),
    ] {
        assert_eq!(blocked_hotkey("hotkey", &args), None, "{args}");
    }
    // Typed text is not a hotkey, and only the two key tools are checked.
    assert_eq!(
        blocked_hotkey("type_text", &json!({ "text": "cmd+q", "keys": "cmd+q" })),
        None
    );
    assert_eq!(blocked_hotkey("click", &json!({ "keys": "cmd+q" })), None);
}

#[test]
fn a_blocked_step_inside_run_steps_blocks_the_call() {
    let steps = json!({ "steps": [
        { "tool": "click", "arguments": { "x": 1 } },
        { "tool": "hotkey", "arguments": { "keys": "cmd+shift+q" } },
    ] });
    assert_eq!(
        blocked_hotkey("run_steps", &steps),
        Some("logs out the session")
    );
    let fine = json!({ "steps": [{ "tool": "hotkey", "arguments": { "keys": "cmd+c" } }] });
    assert_eq!(blocked_hotkey("run_steps", &fine), None);
    assert_eq!(blocked_hotkey("run_steps", &json!({})), None);
}

#[test]
fn other_argument_names_glyphs_objects_and_key_codes_are_still_caught() {
    for args in [
        json!({ "chord": "cmd+q" }),
        json!({ "keystroke": "Command-Shift-Q" }),
        json!({ "combination": ["left_command", "q"] }),
        json!({ "command": true, "shift": true, "key": "q" }),
        json!({ "key": 12, "modifiers": ["cmd"] }),
        json!({ "keys": "\u{2318}\u{21e7}Q" }),
        json!({ "keys": "cmd_q" }),
        json!({ "nested": { "combo": "cmd+opt+esc" } }),
    ] {
        assert!(blocked_hotkey("hotkey", &args).is_some(), "{args}");
    }
    assert_eq!(
        blocked_hotkey("hotkey", &json!({ "command": false, "key": "q" })),
        None
    );
    assert_eq!(blocked_hotkey("hotkey", &json!({ "keys": "cmd+c" })), None);
}
