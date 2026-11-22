//! Blocked hotkeys for the computer broker (OC3, from Orca's blocked key
//! combos): key combinations refused before they are sent, because they end
//! the session or the work: lock the screen, log out, force quit, empty the
//! Trash, quit the front app, shut down or restart. The agent's act tier
//! otherwise allows any hotkey, and a call that logs the sandbox out (or the
//! person on This Mac) is never what a task meant.
//!
//! The check reads the call's own key arguments (`key`, `keys`, `combo`,
//! `hotkey`, `modifiers`, strings or arrays of strings), so it is a
//! heuristic over names (the Driver's real argument shapes are for CU0 to
//! confirm); a combination typed as text with `type_text` is not a hotkey.

use serde_json::Value;
use std::collections::BTreeSet;

/// One blocked combination: the tokens that must all be present, and why.
struct Rule {
    keys: &'static [&'static str],
    why: &'static str,
}

const RULES: &[Rule] = &[
    Rule {
        keys: &["cmd", "ctrl", "q"],
        why: "locks the screen",
    },
    Rule {
        keys: &["cmd", "shift", "q"],
        why: "logs out the session",
    },
    Rule {
        keys: &["cmd", "opt", "esc"],
        why: "opens Force Quit",
    },
    Rule {
        keys: &["cmd", "shift", "delete"],
        why: "empties the Trash",
    },
    Rule {
        keys: &["cmd", "q"],
        why: "quits the front app",
    },
    Rule {
        keys: &["ctrl", "cmd", "eject"],
        why: "shuts down or restarts",
    },
    Rule {
        keys: &["ctrl", "cmd", "power"],
        why: "shuts down or restarts",
    },
    Rule {
        keys: &["ctrl", "opt", "cmd", "eject"],
        why: "shuts down",
    },
];

fn canonical(token: &str) -> Option<&'static str> {
    Some(match token {
        "cmd" | "command" | "meta" | "super" | "win" => "cmd",
        "ctrl" | "control" | "ctl" => "ctrl",
        "opt" | "option" | "alt" => "opt",
        "shift" => "shift",
        "esc" | "escape" => "esc",
        "delete" | "del" | "backspace" | "forwarddelete" => "delete",
        "q" => "q",
        "eject" => "eject",
        "power" => "power",
        _ => return None,
    })
}

/// Glyphs people and drivers write for modifiers.
fn glyph(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{2318}' => "cmd",
        '\u{2303}' => "ctrl",
        '\u{2325}' => "opt",
        '\u{21e7}' => "shift",
        '\u{238b}' => "esc",
        '\u{232b}' | '\u{2326}' => "delete",
        '\u{23cf}' => "eject",
        _ => return None,
    })
}

fn push_text(s: &str, out: &mut BTreeSet<&'static str>) {
    let lower = s.to_lowercase();
    for c in lower.chars() {
        if let Some(t) = glyph(c) {
            out.insert(t);
        }
    }
    // Any non-alphanumeric character separates; `left_command`, `cmd_q` and
    // `Control+Command+Q` all split into words.
    for part in lower.split(|c: char| !c.is_ascii_alphanumeric()) {
        let part = part
            .strip_prefix("left")
            .or_else(|| part.strip_prefix("right"))
            .unwrap_or(part);
        if let Some(t) = canonical(part) {
            out.insert(t);
        }
    }
}

/// Every string leaf and every object key anywhere in the arguments, so a
/// differently named argument (`chord`, `keystroke`, `{"command":true,"key":"q"}`)
/// still counts. Numeric key codes 12 (Q), 53 (Esc), 51 and 117 (delete) count.
fn collect(v: &Value, out: &mut BTreeSet<&'static str>) {
    match v {
        Value::String(s) => push_text(s, out),
        Value::Array(a) => a.iter().for_each(|x| collect(x, out)),
        Value::Object(o) => {
            for (k, x) in o {
                // A modifier flag is a key set to true; a false flag is not.
                if x.as_bool() != Some(false) {
                    push_text(k, out);
                }
                collect(x, out);
            }
        }
        Value::Number(n) => match n.as_u64() {
            Some(12) => {
                out.insert("q");
            }
            Some(53) => {
                out.insert("esc");
            }
            Some(51 | 117) => {
                out.insert("delete");
            }
            _ => {}
        },
        _ => {}
    }
}

fn tokens(args: &Value) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    collect(args, &mut out);
    out
}

fn blocked_in(tool: &str, args: &Value) -> Option<&'static str> {
    if !matches!(tool, "press_key" | "hotkey") {
        return None;
    }
    let t = tokens(args);
    RULES
        .iter()
        .find(|r| r.keys.iter().all(|k| t.contains(k)))
        .map(|r| r.why)
}

/// Why this call (or a step of it) is a blocked hotkey, or `None`.
pub(crate) fn blocked_hotkey(tool: &str, args: &Value) -> Option<&'static str> {
    if tool != "run_steps" {
        return blocked_in(tool, args);
    }
    args.get("steps")?.as_array()?.iter().find_map(|s| {
        blocked_in(
            s.get("tool").and_then(Value::as_str).unwrap_or(""),
            s.get("arguments").unwrap_or(&Value::Null),
        )
    })
}

#[cfg(test)]
#[path = "computer_keys_tests.rs"]
mod tests;
