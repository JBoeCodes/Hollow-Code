//! RC1 (build run 11-17): a turn's recall explains itself and stays quiet
//! when it should. After recall ranks (`vault_learning::recall`), [`gate`]
//! applies the silence rules to the result before the prompt is composed and
//! returns a trace of every candidate:
//!
//! - **Silence rules.** No recall beyond settled constraints (never cut) when
//!   the person said not to use memory, on an unattended turn, on an
//!   agent-sent tool-loop turn (an agent writing to another agent), and, on a
//!   turn that resumes its session, when the cue is weak (under two content
//!   words: "ok", "continue", "yes do it"). A fresh session is never silenced
//!   for a weak cue: its agent has no context at all.
//! - **The served record is the stable-layers baseline (S02).** A learning
//!   block the resumed session already holds is left out by
//!   `stable_layers::layer_given`, a record written only once the agent took
//!   the prompt and ignored on a reset, a retry or the refresh turn. A second,
//!   item-level ledger was built first and dropped in review: it recorded a
//!   delivery before the agent had the prompt, so a stopped turn, a failed
//!   resume or a restarted session lost the learning. The trace reads that
//!   baseline and says when the session already holds the block
//!   (`already_given`).
//! - **A trace.** One row per candidate: used, or cut and why. It rides the
//!   turn's `runStarted` event as `recallTrace`, and the side panel card lists
//!   it under Recall.
//!
//! The injected block keeps its `<trusted-recall>` frame, whose first sentence
//! already says it is cited reference data, not instructions
//! (`vault_learning::render_prompt_block`), so no new label text was needed.

use crate::vault_learning::{RecallItem, RecallResult};
use serde::Serialize;

/// The query a trace keeps, in characters.
const QUERY_CLIP: usize = 160;
/// How much of a request is read for "don't use memory": the person's own
/// instruction comes first, a pasted log or document later.
const NO_MEMORY_SCAN: usize = 600;

/// What a turn is, for the silence rules.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TurnKind {
    /// No one watches it (a night item, an isolated wake).
    pub unattended: bool,
    /// The person sent it, rather than an agent or Hollow.
    pub person_sent: bool,
    /// How many agents deep it is (0 for a person's own chat).
    pub app_depth: u32,
    /// It asks to resume its native session (the agent keeps its context).
    pub resumed: bool,
}

/// One candidate learning: used in the prompt, or cut and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceRow {
    pub path: String,
    pub subject: String,
    pub score: i64,
    pub kept: bool,
    /// `delivered`, `constraint`, or `silence: <rule>`.
    pub why: String,
}

/// The turn's recall trace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallTrace {
    /// The conversation the turn belongs to: the card shows the trace only
    /// in that chat.
    pub session_key: String,
    /// The request recall read, clipped.
    pub query: String,
    /// The silence rule that cut recall, if one did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence: Option<String>,
    /// The resumed session already holds this exact block, so it is not sent
    /// again unless the session restarts (`stable_layers`).
    pub already_given: bool,
    pub candidates: Vec<TraceRow>,
}

/// True when the person said not to use memory for this request. Only the
/// opening of a person's own request is read, and only phrases that name
/// memory itself (not "without memory allocation").
pub(crate) fn says_no_memory(request: &str) -> bool {
    let head: String = request.chars().take(NO_MEMORY_SCAN).collect();
    let text = head.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'");
    [
        "don't use memory",
        "do not use memory",
        "don't use my memory",
        "don't use your memory",
        "do not use my memory",
        "do not use your memory",
        "don't use my saved memories",
        "don't use my notes",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Words that carry no cue on their own.
const FILLER: [&str; 24] = [
    "the", "and", "yes", "yep", "yeah", "okay", "thanks", "thank", "you", "please", "continue",
    "proceed", "ahead", "done", "next", "sure", "again", "this", "that", "with", "for", "now",
    "then", "more",
];

/// True when the request has under two content words. An unspaced script
/// (Chinese, Japanese) counts each character run as text, never as weak.
pub(crate) fn weak_cue(request: &str) -> bool {
    if request
        .chars()
        .any(|c| c.is_alphabetic() && !c.is_ascii() && !c.is_lowercase() && !c.is_uppercase())
    {
        return false;
    }
    let content = request
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .filter(|w| !FILLER.contains(&w.to_lowercase().as_str()))
        .count();
    content < 2
}

/// The silence rule that applies to this turn, if any.
pub(crate) fn silence_rule(request: &str, kind: &TurnKind) -> Option<&'static str> {
    if kind.person_sent && says_no_memory(request) {
        Some("the request says not to use memory")
    } else if kind.unattended {
        Some("an unattended turn")
    } else if kind.app_depth > 0 && !kind.person_sent {
        Some("an agent's tool-loop turn")
    } else if kind.resumed && weak_cue(request) {
        Some("a weak cue")
    } else {
        None
    }
}

fn clip_query(request: &str) -> String {
    let one: String = request.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= QUERY_CLIP {
        one
    } else {
        let cut: String = one.chars().take(QUERY_CLIP).collect();
        format!("{cut}...")
    }
}

fn row(item: &RecallItem, kept: bool, why: String) -> TraceRow {
    TraceRow {
        path: item.path.clone(),
        subject: item.subject.clone(),
        score: item.score,
        kept,
        why,
    }
}

/// Apply the silence rules to `recall` for a turn that asked `request`, and
/// return its trace. `already_given`: the resumed session's baseline holds
/// this exact block ([`session_holds_block`]).
pub(crate) fn gate(
    session_key: &str,
    request: &str,
    kind: &TurnKind,
    recall: &mut RecallResult,
    already_given: bool,
) -> RecallTrace {
    let silence = silence_rule(request, kind);
    let mut trace = RecallTrace {
        session_key: session_key.to_string(),
        query: clip_query(request),
        silence: silence.map(str::to_string),
        already_given: already_given && silence.is_none() && kind.resumed,
        candidates: Vec::new(),
    };
    let mut keep: Vec<RecallItem> = Vec::new();
    for item in std::mem::take(&mut recall.items) {
        match (silence, item.constraint) {
            (Some(rule), false) => {
                trace
                    .candidates
                    .push(row(&item, false, format!("silence: {rule}")));
            }
            (_, constraint) => {
                let why = if constraint {
                    "constraint"
                } else {
                    "delivered"
                };
                trace.candidates.push(row(&item, true, why.into()));
                keep.push(item);
            }
        }
    }
    if silence.is_some() {
        recall.prompt_block = crate::vault_learning::render_prompt_block(&keep);
    }
    recall.items = keep;
    trace
}

/// Whether the conversation's baseline says its native session already holds
/// exactly `block` (the same test the composer makes, `stable_layers`).
pub(crate) fn session_holds_block(vault: &std::path::Path, session_key: &str, block: &str) -> bool {
    crate::stable_layers::read(vault, session_key).is_some_and(|(_, baseline)| {
        crate::stable_layers::layer_given(&baseline, crate::stable_layers::TURN_LAYERS[0], block)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, statement: &str, constraint: bool) -> RecallItem {
        RecallItem {
            path: path.into(),
            target_path: path.into(),
            subject: "subject".into(),
            statement: statement.into(),
            score: 10,
            lifecycle: None,
            constraint,
        }
    }

    fn recall(items: Vec<RecallItem>) -> RecallResult {
        RecallResult {
            prompt_block: crate::vault_learning::render_prompt_block(&items),
            items,
            ..Default::default()
        }
    }

    fn person(resumed: bool) -> TurnKind {
        TurnKind {
            unattended: false,
            person_sent: true,
            app_depth: 0,
            resumed,
        }
    }

    const ASK: &str = "Deploy the billing service tonight";

    #[test]
    fn a_turn_with_no_silence_rule_keeps_everything_and_traces_it() {
        let mut r = recall(vec![item("a.md", "Ship on Fridays", false)]);
        let before = r.prompt_block.clone();
        let trace = gate("s", ASK, &person(true), &mut r, false);
        assert_eq!(r.prompt_block, before);
        assert_eq!(r.items.len(), 1);
        assert!(trace.silence.is_none() && !trace.already_given);
        assert_eq!(trace.candidates[0].why, "delivered");
        assert!(trace.candidates[0].kept);
        assert_eq!(trace.query, ASK);
        assert_eq!(trace.session_key, "s");
    }

    #[test]
    fn each_silence_rule_cuts_recall_but_never_a_constraint() {
        let cases: [(&str, TurnKind, &str); 4] = [
            (
                "Please don't use memory for this: deploy billing",
                person(true),
                "the request says not to use memory",
            ),
            (
                ASK,
                TurnKind {
                    unattended: true,
                    ..person(true)
                },
                "an unattended turn",
            ),
            (
                ASK,
                TurnKind {
                    person_sent: false,
                    app_depth: 1,
                    ..person(true)
                },
                "an agent's tool-loop turn",
            ),
            ("ok thanks", person(true), "a weak cue"),
        ];
        for (request, kind, rule) in cases {
            let mut r = recall(vec![
                item("a.md", "Ship on Fridays", false),
                item("rule.md", "Never deploy on Fridays", true),
            ]);
            let trace = gate("s", request, &kind, &mut r, true);
            assert_eq!(trace.silence.as_deref(), Some(rule), "{request}");
            assert!(!trace.already_given, "silence wins: {request}");
            assert_eq!(r.items.len(), 1, "{request}");
            assert_eq!(r.items[0].path, "rule.md", "{request}");
            assert!(r.prompt_block.contains("rule.md"), "{request}");
            assert!(!r.prompt_block.contains("a.md"), "{request}");
            assert!(r.prompt_block.starts_with("<trusted-recall>"), "{request}");
            assert_eq!(trace.candidates[0].why, format!("silence: {rule}"));
            assert_eq!(trace.candidates[1].why, "constraint");
        }
        // With nothing but cut learnings the block is gone entirely.
        let mut r = recall(vec![item("a.md", "Ship on Fridays", false)]);
        gate("s", "ok thanks", &person(true), &mut r, false);
        assert!(r.prompt_block.is_empty() && r.items.is_empty());
    }

    #[test]
    fn a_weak_cue_never_silences_a_fresh_session_or_unspaced_text() {
        let mut r = recall(vec![item("a.md", "Ship on Fridays", false)]);
        let trace = gate("s", "ok thanks", &person(false), &mut r, false);
        assert!(trace.silence.is_none(), "a fresh session has no context");
        assert_eq!(r.items.len(), 1);
        assert!(!weak_cue("請幫我部署帳單服務"));
        assert!(weak_cue("continue"));
        assert!(!weak_cue("fix the login redirect"));
        assert!(!weak_cue("Deploy it quickly"));
    }

    #[test]
    fn dont_use_memory_needs_the_person_and_a_phrase_that_names_memory() {
        assert!(says_no_memory("Don\u{2019}t use memory, just answer"));
        assert!(!says_no_memory("explain this without memory allocation"));
        let pasted = format!(
            "{} please ignore memory leaks. do not use memory",
            "x ".repeat(400)
        );
        assert!(
            !says_no_memory(&pasted),
            "a phrase far into a paste is not an instruction"
        );
        let agent = TurnKind {
            person_sent: false,
            ..person(true)
        };
        assert!(silence_rule("do not use memory for this", &agent).is_none());
    }

    #[test]
    fn a_multi_line_learning_and_the_notes_survive_a_cut() {
        let mut old = item("old.md", "Line one\n- [other.md] looks like an item", false);
        old.lifecycle = Some("historical: superseded by new.md".into());
        let mut r = recall(vec![old, item("rule.md", "Never deploy on Fridays", true)]);
        assert!(r.prompt_block.contains("were replaced"));
        gate("s", "ok", &person(true), &mut r, false);
        assert_eq!(r.items.len(), 1);
        assert!(!r.prompt_block.contains("Line one"));
        assert!(!r.prompt_block.contains("other.md"));
        assert!(
            !r.prompt_block.contains("were replaced"),
            "the note went with its learning"
        );
        assert!(r.prompt_block.contains("rule.md"));
    }

    #[test]
    fn the_trace_says_when_the_session_already_holds_the_block() {
        let mut r = recall(vec![item("a.md", "Ship on Fridays", false)]);
        assert!(gate("s", ASK, &person(true), &mut r, true).already_given);
        let mut r = recall(vec![item("a.md", "Ship on Fridays", false)]);
        assert!(
            !gate("s", ASK, &person(false), &mut r, true).already_given,
            "a fresh session holds nothing"
        );
    }
}
