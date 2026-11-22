//! An approval is for one exact action (ND6, 2026-11-20; Hollow's own design).
//!
//! A card's Allow is bound to a fingerprint of the call it showed: the tool
//! (or action id) and its canonical arguments. The broker recomputes the
//! fingerprint from the arguments it is about to run, after the wait and
//! every check, and a mismatch voids the approval: nothing runs and the
//! agent is told to ask again. The consent text also lists what a yes does
//! not cover, so the person never reads one card as a standing permission.
//!
//! Pure functions only: this module stores and shows nothing. The grant
//! itself is held by `approvals.rs` (per asking thread, like the Decline
//! reason) and consumed once by [`verify`].

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// What one approval was given for: the card and the call it showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grant {
    pub card_id: String,
    /// [`fingerprint`] of the call the card showed.
    pub fingerprint: String,
}

/// Arguments in one form: a missing `arguments` and `{}` are the same call
/// (Claude emits `{}` for a tool with no arguments, an MCP client may omit
/// the field), and object keys are sorted whatever the JSON library keeps.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: std::collections::BTreeMap<&String, Value> =
                map.iter().map(|(k, v)| (k, canonical(v))).collect();
            let mut out = Map::new();
            for (k, v) in sorted {
                out.insert(k.clone(), v);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// SHA-256 (hex) of the tool name and its canonical arguments. Stable across
/// key order and `null` against `{}`; any other change of any argument
/// changes it.
pub(crate) fn fingerprint(tool: &str, args: &Value) -> String {
    let args = if args.is_null() {
        Value::Object(Map::new())
    } else {
        canonical(args)
    };
    let mut hasher = Sha256::new();
    hasher.update(tool.as_bytes());
    hasher.update([0u8]);
    hasher.update(serde_json::to_vec(&args).unwrap_or_default());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `grant` covers this call. The error is the refusal the agent
/// hears; the approval is spent either way (the caller took it).
pub(crate) fn verify(grant: &Grant, tool: &str, args: &Value) -> Result<(), String> {
    if grant.fingerprint == fingerprint(tool, args) {
        Ok(())
    } else {
        Err(format!(
            "The arguments of {tool} changed after the person approved it, so that approval is void and nothing ran. Ask again with the arguments you mean."
        ))
    }
}

/// What a yes on this kind of card does not cover, one line each (shown on
/// the card and the phone). `external` is a card for an action that leaves
/// this Mac.
pub(crate) fn not_covered(external: bool) -> Vec<&'static str> {
    let mut lines = vec![
        "Only this exact call: a change to any argument needs a new yes.",
        "One use: it does not allow repeating the call, now or in a retry.",
        "Nothing else in this chat: other tools, other cards and later turns ask again.",
    ];
    if external {
        lines.push("It does not undo or check anything already sent: a call interrupted before its reply may have happened.");
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_and_empty_arguments_do_not_change_the_fingerprint() {
        let a = fingerprint(
            "t",
            &json!({"a": 1, "b": {"y": 2, "x": [1, {"q": 1, "p": 2}]}}),
        );
        let b = fingerprint(
            "t",
            &json!({"b": {"x": [1, {"p": 2, "q": 1}], "y": 2}, "a": 1}),
        );
        assert_eq!(a, b);
        assert_eq!(fingerprint("t", &Value::Null), fingerprint("t", &json!({})));
    }

    #[test]
    fn any_changed_argument_or_tool_changes_it() {
        let base = fingerprint("send", &json!({"to": "a@x.com", "amount": 10}));
        assert_ne!(
            base,
            fingerprint("send", &json!({"to": "b@x.com", "amount": 10}))
        );
        assert_ne!(
            base,
            fingerprint("send", &json!({"to": "a@x.com", "amount": 11}))
        );
        assert_ne!(
            base,
            fingerprint("send", &json!({"to": "a@x.com", "amount": 10, "x": 1}))
        );
        assert_ne!(
            base,
            fingerprint("send2", &json!({"to": "a@x.com", "amount": 10}))
        );
        // 1 and "1" are different arguments.
        assert_ne!(
            fingerprint("t", &json!({"n": 1})),
            fingerprint("t", &json!({"n": "1"}))
        );
        // The tool/args boundary is unambiguous.
        assert_ne!(
            fingerprint("ab", &json!({})),
            fingerprint("a", &json!({"b": 1}))
        );
    }

    #[test]
    fn verify_accepts_the_shown_call_and_voids_a_changed_one() {
        let args = json!({"to": "a@x.com"});
        let grant = Grant {
            card_id: "apr-1".into(),
            fingerprint: fingerprint("send", &args),
        };
        assert!(verify(&grant, "send", &args).is_ok());
        let error = verify(&grant, "send", &json!({"to": "evil@x.com"})).unwrap_err();
        assert!(error.contains("void") && error.contains("nothing ran"));
        assert!(verify(&grant, "other", &args).is_err());
    }

    #[test]
    fn the_consent_text_lists_what_the_yes_does_not_cover() {
        assert_eq!(not_covered(false).len(), 3);
        assert_eq!(not_covered(true).len(), 4);
        assert!(not_covered(true).iter().any(|l| l.contains("interrupted")));
        for line in not_covered(true) {
            assert!(!line.contains('\u{2014}') && !line.contains('\u{2013}'));
        }
    }
}
