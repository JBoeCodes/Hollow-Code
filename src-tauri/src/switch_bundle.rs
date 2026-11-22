//! Budgeted handoff on a provider switch (repo inspiration PS1): what the
//! conversation bundle of a fresh native session may hold, measured against
//! the target agent's context window, and the pointers to what it left out.
//!
//! **What a switch carries.** The composer drops the native id on an agent
//! or mode switch, so the next turn is a fresh session whose body leads with
//! the `<conversation-context>` bundle (`context_compiler`) and the
//! `<task-state>` record (`task_state.rs`: goal, settled constraints,
//! corrections, steps done, open questions, changed files, each citing its
//! turn). The task-state record is the host-derived handoff body; the OR1
//! `<hollow-handoff-record>` (`delegation.rs`) is something an agent writes
//! for a successor or child and arrives in a task packet, so a person's own
//! switch has none of it.
//!
//! **The rule (ours).**
//! 1. The bundle gets at most [`SHARE_PERCENT`] percent of the target's
//!    window ([`window_tokens`], a conservative table), at
//!    [`BYTES_PER_TOKEN`] bytes a token ([`byte_limit`]), on top of the turn
//!    budget's grant (`context_budget`, G2); the smaller limit wins.
//! 2. The most recent turns are kept first; entries drop from the front.
//! 3. Every stretch left out is replaced by one pointer entry per
//!    conversation naming its session key and turn range, with the
//!    `read_session` arguments that fetch it ([`pointer_text`]); tool
//!    results are not in the bundle, and `read_session` excerpts them.
//! 4. A limit under [`MIN_BUNDLE_BYTES`], or one the newest entry and its
//!    pointer cannot fit in, starts the agent fresh with no bundle at all,
//!    and the person sees a notice ([`fresh_notice`]) in the chat.
//!
//! **Windows.** `context_gauge::context_window` knows only Claude's (200K,
//! or 1M for a `[1m]` model). The rest are our own conservative floors:
//! Codex reports 258,400 in its rollouts and Antigravity and Grok publish
//! larger ones, but Hollow can't read them per model, so each gets 128,000
//! tokens, under every window seen. A self-hosted model's exact window cuts
//! its own prompt (`self_hosted.rs`).

use crate::agent::AgentKind;
use crate::session_store::ContextEntry;

/// The bundle's share of the target's window, in percent.
pub(crate) const SHARE_PERCENT: u64 = 25;
/// Bytes counted for one token: lower than prose's four, because the
/// bundle is JSON-escaped and tokenizes denser.
pub(crate) const BYTES_PER_TOKEN: u64 = 3;
/// A limit below this carries no bundle: the block's frame and the newest
/// turn alone would not fit.
pub(crate) const MIN_BUNDLE_BYTES: usize = 4_000;
/// The most turns one `read_session` call returns in full.
const POINTER_TURNS: i64 = 20;

/// The target's context window in tokens (see the module doc).
pub(crate) fn window_tokens(agent: AgentKind) -> u64 {
    match agent {
        AgentKind::Claude => 200_000,
        AgentKind::Codex | AgentKind::Antigravity | AgentKind::Grok => 128_000,
    }
}

/// The bundle's byte limit for a window of `tokens`.
pub(crate) fn limit_for_window(tokens: u64) -> usize {
    usize::try_from(tokens * SHARE_PERCENT / 100 * BYTES_PER_TOKEN).unwrap_or(usize::MAX)
}

/// The bundle's byte limit for a turn with `agent`.
pub(crate) fn byte_limit(agent: AgentKind) -> usize {
    limit_for_window(window_tokens(agent))
}

/// One left-out stretch: a conversation and its turns, inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stretch {
    pub key: String,
    pub from: i64,
    pub to: i64,
}

/// The stretches `omitted` (a prefix of the history) leaves out, one per
/// conversation in reading order. An entry with no key reads as nothing.
pub(crate) fn stretches(omitted: &[&ContextEntry]) -> Vec<Stretch> {
    let mut out: Vec<Stretch> = Vec::new();
    for entry in omitted {
        if entry.key.is_empty() || entry.turn < 1 {
            continue;
        }
        match out.iter_mut().find(|s| s.key == entry.key) {
            Some(s) => {
                s.from = s.from.min(entry.turn);
                s.to = s.to.max(entry.turn);
            }
            None => out.push(Stretch {
                key: entry.key.clone(),
                from: entry.turn,
                to: entry.turn,
            }),
        }
    }
    out
}

/// The pointer line for a stretch: the key, the turn range and the
/// `read_session` arguments, in as many calls as the range needs.
pub(crate) fn pointer_text(s: &Stretch) -> String {
    let range = if s.from == s.to {
        format!("Turn {}", s.from)
    } else {
        format!("Turns {} to {}", s.from, s.to)
    };
    let mut calls = Vec::new();
    let mut from = s.from;
    while from <= s.to {
        let to = (from + POINTER_TURNS - 1).min(s.to);
        calls.push(format!(
            "{{\"key\":{},\"fromTurn\":{from},\"toTurn\":{to}}}",
            serde_json::to_string(&s.key).unwrap_or_else(|_| "\"\"".into())
        ));
        from = to + 1;
    }
    format!(
        "{range} of session {} were left out to fit. Read them with hollow_app read_session: {} (tools are excerpted unless \"tools\":\"full\").",
        s.key,
        calls.join(" then ")
    )
}

/// The notice the person sees when a fresh session starts with no bundle
/// though the conversation has history.
pub(crate) fn fresh_notice(turns_available: usize) -> String {
    format!(
        "Hollow started this agent fresh: the earlier conversation ({turns_available} entries) does not fit the room it has for it, so none of it was sent. This chat's history is still here; paste anything the agent needs."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(key: &str, turn: i64) -> ContextEntry {
        ContextEntry {
            role: "user".into(),
            agent: "claude".into(),
            text: String::new(),
            key: key.into(),
            turn,
        }
    }

    #[test]
    fn the_limit_is_a_quarter_of_the_window() {
        assert_eq!(limit_for_window(128_000), 96_000);
        assert_eq!(byte_limit(AgentKind::Claude), 150_000);
        assert_eq!(limit_for_window(8_000), 6_000);
    }

    #[test]
    fn stretches_group_by_conversation_and_skip_unknowns() {
        let all = [
            e("a", 3),
            e("a", 4),
            e("b", 1),
            e("b", 2),
            e("", 0),
            e("a", 5),
        ];
        let refs: Vec<&ContextEntry> = all.iter().collect();
        let got = stretches(&refs);
        assert_eq!(
            got,
            vec![
                Stretch {
                    key: "a".into(),
                    from: 3,
                    to: 5
                },
                Stretch {
                    key: "b".into(),
                    from: 1,
                    to: 2
                },
            ]
        );
    }

    #[test]
    fn a_long_range_splits_into_calls_of_twenty_turns() {
        let text = pointer_text(&Stretch {
            key: "k".into(),
            from: 1,
            to: 45,
        });
        assert!(text.contains(r#""fromTurn":1,"toTurn":20"#));
        assert!(text.contains(r#""fromTurn":21,"toTurn":40"#));
        assert!(text.contains(r#""fromTurn":41,"toTurn":45"#));
        assert!(pointer_text(&Stretch {
            key: "k".into(),
            from: 7,
            to: 7
        })
        .starts_with("Turn 7 of"));
    }
}
