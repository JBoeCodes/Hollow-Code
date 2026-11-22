//! Fresh chats with a brief (overhaul K3, overnight 11-12 C7): a small
//! follow-up to a subagent whose context is already large starts a fresh
//! subagent with a short brief instead of resuming the long chat, so the
//! follow-up doesn't pay for (and wade through) the whole earlier context.
//!
//! **When.** [`recommend`]: the conversation's stored context figure
//! (`context_gauge::stored_figure`, the one the turn footer and the growth
//! steps read) has reached the gauge's first growth step
//! (`context_gauge::step_for`: 300K, or 70% of a window the agent reported
//! itself) and the follow-up is at most [`SMALL_PROMPT_CHARS`] characters.
//! [`decide`] adds who asked for what: `send_to_session` with `fresh` unset
//! goes fresh on that recommendation for a subagent only (a project lead
//! and the COO keep their one conversation, and an ordinary chat is the
//! person's); `fresh: true` asks for it on a subagent even below the
//! threshold (refused on any other conversation); `fresh: false` always
//! continues. Both fresh paths need the caller to be the subagent's parent
//! and the subagent to have no turn queued or running (review M1, M2,
//! 2026-11-12): otherwise the automatic path continues (an ordinary send
//! waits behind a running turn) and `fresh: true` is refused, so two
//! agents never edit one folder at once and no one else's subagent is
//! replaced. A fresh follow-up runs in the current conversation's own mode
//! (`app_sessions::send_mode` reads the chain's current conversation, so a
//! bypass predecessor with a read-only successor gives a read-only child),
//! lowered only as the caller asks, and every field copied from the earlier
//! chat is redacted before it is cut.
//!
//! **The brief** ([`build`]): deterministic, no model call, at most about
//! [`MAX_BRIEF_CHARS`] characters after the follow-up, which comes first
//! verbatim (so the new chat's title is its own). Where the earlier chat
//! stands (its title, its goal and its last answer, clipped),
//! the settled constraints and corrections from its task-state record
//! (`task_state.rs`), the files its turns changed, how to read the rest
//! (`read_session`) and the batching rule ([`BATCH_EDITS`]). The new subagent's queue row is an ordinary `start_session`
//! one in the earlier chat's folder, on its agent and model.
//!
//! **Trace.** The new conversation's `derived_state` row of kind [`KIND`]
//! (`{from, tokens}`) is what the Subagents tab reads
//! (`subagents::SubagentRow::fresh_from`) to say [`TRACE`] on its row and
//! in its header, so the person sees it happened and where it came from.

use crate::task_state::Record;
use serde_json::{json, Value};
use std::path::Path;

/// The longest follow-up (characters) that counts as small.
pub(crate) const SMALL_PROMPT_CHARS: usize = 1_500;
/// What the Subagents tab says on a fresh chat's row and header.
pub(crate) const TRACE: &str = "Fresh chat with a brief";
/// The `derived_state` kind on the fresh conversation.
pub(crate) const KIND: &str = "freshBrief";
/// The brief's bound before the follow-up (characters).
pub(crate) const MAX_BRIEF_CHARS: usize = 3_000;
/// The earlier chat's last answer, clipped to this many characters.
const ANSWER_CHARS: usize = 700;
/// Settled items (constraints, then corrections) the brief carries at most.
const MAX_SETTLED: usize = 8;
/// Changed files the brief lists at most (the newest).
const MAX_FILES: usize = 12;
/// What every brief tells the fresh subagent about edits (K3's second half).
pub(crate) const BATCH_EDITS: &str =
    "Batch your file edits: one edit pass per file, not many small writes.";

/// The delegating agents' one line (the COO's and every lead's role): K3's
/// fresh follow-ups and batched edits.
pub(crate) const FOLLOW_UP_RULE: &str = "A small follow-up to a long subagent goes fresh with a brief (send_to_session does it); ask for one edit pass per file.";

/// Whether a follow-up of `prompt` to a conversation whose stored context
/// figure is `tokens` (of `window`, when the agent reported it) should go
/// to a fresh chat with a brief.
pub(crate) fn recommend(tokens: Option<u64>, window: Option<u64>, prompt: &str) -> bool {
    let large = tokens.is_some_and(|t| crate::context_gauge::step_for(t, window) > 0);
    large && is_small(prompt)
}

/// Whether `prompt` is a small follow-up.
pub(crate) fn is_small(prompt: &str) -> bool {
    let text = prompt.trim();
    !text.is_empty() && crate::context_compiler::char_count(text) <= SMALL_PROMPT_CHARS
}

/// What a `send_to_session` does with its follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Resume the conversation, as before K3.
    Continue,
    /// Start a fresh subagent with a brief: `asked` when the caller passed
    /// `fresh: true`, else Hollow's recommendation.
    Fresh { asked: bool },
}

/// The conversation a follow-up is for, as [`decide`] needs it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Target {
    /// A subagent (`fork_kind = 'subagent'`).
    pub subagent: bool,
    /// A project lead's conversation or the COO's: never replaced.
    pub keeps_one_chat: bool,
    /// The conversation has a turn queued or running (review M1): a fresh
    /// chat in its folder would edit the same files at once.
    pub busy: bool,
    /// The caller is the subagent's parent (review M2): only it replaces
    /// its own subagent.
    pub from_parent: bool,
}

/// `fresh: true` from a caller that isn't the subagent's parent.
pub(crate) const NOT_PARENT: &str = "fresh is for the subagent's own parent: only the chat that started it can replace it with a fresh one. Send without fresh. Nothing was sent.";
/// `fresh: true` to a subagent with a turn queued or running.
pub(crate) const BUSY: &str = "fresh needs the subagent idle: it has a turn queued or running, and a fresh chat in its folder would edit the same files at once. Send without fresh to wait behind that turn, or call wait_for_session and send once it has ended. Nothing was sent.";

/// See the module doc. `Err` for `fresh: true` on a lead's or the COO's
/// conversation, or on one that isn't a subagent (a person's own chat is
/// never turned into a subagent of another, review M1), from a caller that
/// isn't its parent, or while it has a turn queued or running.
pub(crate) fn decide(
    fresh: Option<bool>,
    target: Target,
    recommended: bool,
) -> Result<Decision, String> {
    match fresh {
        Some(false) => Ok(Decision::Continue),
        Some(true) if target.keeps_one_chat || !target.subagent => Err("fresh is for subagents only: a project lead, the COO and a person's own chat keep their conversation. Send without fresh. Nothing was sent.".into()),
        Some(true) if !target.from_parent => Err(NOT_PARENT.into()),
        Some(true) if target.busy => Err(BUSY.into()),
        Some(true) => Ok(Decision::Fresh { asked: true }),
        None if target.subagent
            && !target.keeps_one_chat
            && target.from_parent
            && !target.busy
            && recommended =>
        {
            Ok(Decision::Fresh { asked: false })
        }
        None => Ok(Decision::Continue),
    }
}

/// What [`build`] reads about the earlier chat.
#[derive(Debug, Clone, Default)]
pub(crate) struct Earlier<'a> {
    pub key: &'a str,
    pub title: Option<&'a str>,
    /// Its stored context figure, when known.
    pub tokens: Option<u64>,
    /// Its newest answer.
    pub answer: Option<&'a str>,
    /// Its task-state record (goal, constraints, corrections, files).
    pub record: Option<&'a Record>,
}

/// `text` with secrets masked (`claude_runner::redact`, review B1), then on
/// one line, cut to `max` characters. Redacted before the cut, so a cut
/// never leaves half a token unmasked.
fn clip(text: &str, max: usize) -> String {
    let safe = crate::claude_runner::redact(text);
    let one_line = safe.split_whitespace().collect::<Vec<_>>().join(" ");
    if crate::context_compiler::char_count(&one_line) <= max {
        one_line
    } else {
        let mut cut: String = one_line.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

/// The fresh subagent's first message: the follow-up first (so the new
/// chat's title is the follow-up's own), then the brief.
pub(crate) fn build(earlier: &Earlier<'_>, prompt: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let size = earlier
        .tokens
        .map(|t| format!(" (its context reached {}K tokens)", t / 1_000))
        .unwrap_or_default();
    lines.push(format!(
        "{TRACE}: the request above follows up the earlier chat {}{size}, so it starts here fresh instead of resuming that long context.",
        earlier.key
    ));
    if let Some(title) = earlier.title.map(str::trim).filter(|t| !t.is_empty()) {
        lines.push(format!("Earlier chat: {}", clip(title, 120)));
    }
    let record = earlier.record;
    if let Some(goal) = record.and_then(|r| r.goal.as_ref()) {
        lines.push(format!("Its goal: {}", clip(&goal.text, 300)));
    }
    if let Some(answer) = earlier.answer.map(str::trim).filter(|a| !a.is_empty()) {
        lines.push(format!("Where it left off: {}", clip(answer, ANSWER_CHARS)));
    }
    let settled: Vec<String> = record
        .map(|r| {
            r.constraints
                .iter()
                .chain(r.corrections.iter())
                .take(MAX_SETTLED)
                .map(|item| format!("- {}", clip(&item.text, 200)))
                .collect()
        })
        .unwrap_or_default();
    if !settled.is_empty() {
        lines.push("Settled (still applies):".into());
        lines.extend(settled);
    }
    let files: Vec<String> = record
        .map(|r| {
            let skip = r.artifacts.len().saturating_sub(MAX_FILES);
            r.artifacts
                .iter()
                .skip(skip)
                .map(|a| format!("{} ({})", clip(&a.path, 200), a.change))
                .collect()
        })
        .unwrap_or_default();
    if !files.is_empty() {
        lines.push(format!("Files it changed: {}", files.join(", ")));
    }
    // The footer always stays whole (review L1): the body is cut to leave
    // room for it.
    let footer = format!(
        "Read more of it only if you need to: read_session {{\"key\": \"{}\"}}.\n{BATCH_EDITS}",
        earlier.key
    );
    let room = MAX_BRIEF_CHARS.saturating_sub(crate::context_compiler::char_count(&footer) + 1);
    let mut body = lines.join("\n");
    if crate::context_compiler::char_count(&body) > room {
        body = body.chars().take(room.saturating_sub(1)).collect();
        body.push('…');
    }
    format!("{}\n\n{body}\n{footer}", prompt.trim())
}

/// Mark conversation `key` as a fresh chat with a brief from `from`. Best
/// effort: a failed write only loses the trace.
pub(crate) fn mark(vault: &Path, key: &str, from: &str, tokens: Option<u64>) {
    let payload = json!({ "from": from, "tokens": tokens }).to_string();
    let row = crate::session_store::DerivedRow {
        revision: 1,
        through_ord: 0,
        payload,
    };
    if let Err(error) = crate::session_store::put_derived_state(vault, key, KIND, &row) {
        eprintln!("[fresh-brief] the trace could not be stored: {error}");
    }
}

/// The conversation `key` was started fresh from, when it was.
pub(crate) fn from_of(vault: &Path, key: &str) -> Option<String> {
    let row = crate::session_store::derived_state(vault, key, KIND).ok()??;
    from_payload(&row.payload)
}

/// [`from_of`] on an open session store connection (the Subagents tab's
/// read, one query per row on the connection it already holds).
pub(crate) fn from_in(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    let payload: String = conn
        .query_row(
            "SELECT payload FROM derived_state WHERE session_key = ?1 AND kind = ?2",
            rusqlite::params![key, KIND],
            |row| row.get(0),
        )
        .ok()?;
    from_payload(&payload)
}

fn from_payload(payload: &str) -> Option<String> {
    serde_json::from_str::<Value>(payload)
        .ok()?
        .get("from")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_state::{Artifact, Item};

    fn item(text: &str) -> Item {
        Item {
            text: text.into(),
            turn: 1,
            ..Default::default()
        }
    }

    #[test]
    fn a_small_follow_up_to_a_large_context_is_recommended_fresh() {
        assert!(recommend(Some(412_000), None, "Also rename the flag."));
        // Below the first growth step: resume.
        assert!(!recommend(Some(120_000), None, "Also rename the flag."));
        // No figure stored: resume.
        assert!(!recommend(None, None, "Also rename the flag."));
        // A large prompt is real new work, not a small follow-up.
        let long = "word ".repeat(SMALL_PROMPT_CHARS / 4);
        assert!(!recommend(Some(412_000), None, &long));
        assert!(!recommend(Some(412_000), None, "   "));
        // A window the agent reported (Codex's 258K): 70% of it is large.
        assert!(recommend(Some(190_000), Some(258_000), "Fix the typo."));
        assert!(!recommend(Some(150_000), Some(258_000), "Fix the typo."));
    }

    #[test]
    fn only_subagents_go_fresh_on_their_own_and_leads_never() {
        let sub = Target {
            subagent: true,
            from_parent: true,
            ..Target::default()
        };
        let chat = Target {
            from_parent: true,
            ..Target::default()
        };
        let lead = Target {
            keeps_one_chat: true,
            from_parent: true,
            ..Target::default()
        };
        assert_eq!(
            decide(None, sub, true),
            Ok(Decision::Fresh { asked: false })
        );
        assert_eq!(decide(None, sub, false), Ok(Decision::Continue));
        assert_eq!(decide(None, chat, true), Ok(Decision::Continue));
        assert_eq!(decide(None, lead, true), Ok(Decision::Continue));
        assert_eq!(decide(Some(false), sub, true), Ok(Decision::Continue));
        assert_eq!(
            decide(Some(true), sub, false),
            Ok(Decision::Fresh { asked: true })
        );
        // Review M1: a person's own chat is never made a subagent, even
        // when asked.
        assert!(decide(Some(true), chat, false).is_err());
        assert!(decide(Some(true), chat, true).is_err());
        assert!(decide(Some(true), lead, true).is_err());
        let sub_lead = Target {
            subagent: true,
            keeps_one_chat: true,
            from_parent: true,
            ..Target::default()
        };
        assert!(decide(Some(true), sub_lead, true).is_err());
    }

    #[test]
    fn a_busy_subagent_continues_and_refuses_fresh() {
        // Review M1 (2026-11-12): a subagent with a turn queued or running
        // takes the ordinary send (it waits behind that turn); a fresh chat
        // would edit its folder at the same time.
        let busy = Target {
            subagent: true,
            from_parent: true,
            busy: true,
            ..Target::default()
        };
        assert_eq!(decide(None, busy, true), Ok(Decision::Continue));
        assert_eq!(decide(Some(false), busy, true), Ok(Decision::Continue));
        assert_eq!(decide(Some(true), busy, true), Err(BUSY.to_string()));
        assert_eq!(decide(Some(true), busy, false), Err(BUSY.to_string()));
    }

    #[test]
    fn only_the_parent_replaces_its_subagent() {
        // Review M2 (2026-11-12): the COO, a sibling or an automation gets
        // the ordinary send, and is refused when it asks for fresh.
        let other = Target {
            subagent: true,
            ..Target::default()
        };
        assert_eq!(decide(None, other, true), Ok(Decision::Continue));
        assert_eq!(decide(Some(false), other, true), Ok(Decision::Continue));
        assert_eq!(decide(Some(true), other, true), Err(NOT_PARENT.to_string()));
        let other_busy = Target {
            busy: true,
            ..other
        };
        assert_eq!(
            decide(Some(true), other_busy, false),
            Err(NOT_PARENT.to_string())
        );
    }

    #[test]
    fn the_brief_is_short_and_carries_summary_constraints_files_and_the_prompt() {
        let record = Record {
            goal: Some(item("Add the export button to the report page")),
            constraints: vec![
                item("Never touch the billing module."),
                item("Keep the API stable."),
            ],
            corrections: vec![item("No, the button goes on the right.")],
            artifacts: vec![Artifact {
                path: "src/report.ts".into(),
                change: "modify".into(),
                turn: 2,
                ..Default::default()
            }],
            ..Default::default()
        };
        let answer = "Done: the button exports CSV. ".repeat(100);
        let brief = build(
            &Earlier {
                key: "s-20261112-abc",
                title: Some("Report export"),
                tokens: Some(412_345),
                answer: Some(&answer),
                record: Some(&record),
            },
            "  Also export as XLSX.  ",
        );
        assert!(
            brief.starts_with("Also export as XLSX.\n\nFresh chat with a brief: "),
            "{brief}"
        );
        assert!(brief.contains("s-20261112-abc (its context reached 412K tokens)"));
        assert!(brief.contains("Earlier chat: Report export"));
        assert!(brief.contains("Its goal: Add the export button"));
        assert!(brief.contains("Where it left off: Done: the button exports CSV."));
        assert!(brief.contains("- Never touch the billing module."));
        assert!(brief.contains("- No, the button goes on the right."));
        assert!(brief.contains("Files it changed: src/report.ts (modify)"));
        assert!(brief.contains("read_session {\"key\": \"s-20261112-abc\"}"));
        assert!(brief.contains(BATCH_EDITS));
        assert!(brief.ends_with(BATCH_EDITS), "{brief}");
        // The 3,000-character answer was clipped: the brief stays short.
        let before = brief.split_once("\n\n").unwrap().1;
        assert!(crate::context_compiler::char_count(before) <= MAX_BRIEF_CHARS);
        assert!(
            crate::context_compiler::char_count(before) < 1_400,
            "{}",
            before.len()
        );
        assert!(!brief.contains('\u{2014}') && !brief.contains('\u{2013}'));
    }

    #[test]
    fn the_brief_caps_settled_items_and_lists_the_newest_files() {
        let record = Record {
            constraints: (0..20).map(|i| item(&format!("Rule {i}."))).collect(),
            artifacts: (0..30)
                .map(|i| Artifact {
                    path: format!("f{i}.rs"),
                    change: "create".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let brief = build(
            &Earlier {
                key: "s-x",
                record: Some(&record),
                ..Default::default()
            },
            "Go.",
        );
        assert_eq!(brief.matches("\n- Rule ").count(), MAX_SETTLED);
        assert!(brief.contains("f29.rs (create)") && brief.contains("f18.rs"));
        assert!(!brief.contains("f17.rs"));
        // Nothing known but the key: still a usable brief.
        let bare = build(
            &Earlier {
                key: "s-y",
                ..Default::default()
            },
            "Go.",
        );
        assert!(
            bare.contains("earlier chat s-y, so it starts here fresh"),
            "{bare}"
        );
        assert!(!bare.contains("Settled") && !bare.contains("Files it changed"));
    }

    #[test]
    fn copied_fields_are_redacted_before_they_are_cut() {
        let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let record = Record {
            goal: Some(item(&format!("Use key {secret} for the deploy"))),
            constraints: vec![item(&format!("Never print {secret}."))],
            artifacts: vec![Artifact {
                path: format!("notes/{secret}.md"),
                change: "create".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        // The answer ends with the secret right at the clip point, so a cut
        // before redaction would leave its head unmasked.
        let answer = format!("{} {secret}", "x".repeat(ANSWER_CHARS - 20));
        let title = format!("Deploy with {secret}");
        let brief = build(
            &Earlier {
                key: "s-a",
                title: Some(&title),
                answer: Some(&answer),
                record: Some(&record),
                ..Default::default()
            },
            "Go.",
        );
        assert!(!brief.contains("abcdefghijklmnop"), "{brief}");
        assert!(!brief.contains("sk-ant-api03-abcdefg"), "{brief}");
    }

    #[test]
    fn the_footer_survives_long_settled_items_and_paths() {
        let long = "word ".repeat(400);
        let record = Record {
            goal: Some(item(&long)),
            constraints: (0..20).map(|_| item(&long)).collect(),
            corrections: (0..6).map(|_| item(&long)).collect(),
            artifacts: (0..30)
                .map(|i| Artifact {
                    path: format!("{i}/{}", "dir/".repeat(200)),
                    change: "modify".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let brief = build(
            &Earlier {
                key: "s-long",
                title: Some(&long),
                answer: Some(&long),
                record: Some(&record),
                ..Default::default()
            },
            "Go.",
        );
        let footer = format!("read_session {{\"key\": \"s-long\"}}.\n{BATCH_EDITS}");
        assert!(brief.ends_with(&footer), "{brief}");
        let after = brief.split_once("\n\n").unwrap().1;
        assert!(crate::context_compiler::char_count(after) <= MAX_BRIEF_CHARS);
    }

    #[test]
    fn the_trace_round_trips_through_derived_state() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        assert_eq!(from_of(vault, "s-new"), None);
        mark(vault, "s-new", "s-old", Some(400_000));
        assert_eq!(from_of(vault, "s-new").as_deref(), Some("s-old"));
    }
}
