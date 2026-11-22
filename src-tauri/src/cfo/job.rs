//! What kind of job a prompt is (COO rebuild L7): deterministic heuristics
//! over the prompt's first [`SCAN_BYTES`] and who asked. No model call, so it
//! adds no time to a turn. The kind sets the effort (`effort.rs`) and the
//! reason the turn footer shows.

use serde::{Deserialize, Serialize};

/// How much of a prompt is read.
const SCAN_BYTES: usize = 2048;
/// A question at most this long (and ending in `?`) is a short question.
const SHORT_QUESTION_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoJobKind"))]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Conversation,
    Question,
    Plan,
    Research,
    Code,
    Review,
    Report,
}

/// Who asked for the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The person typed it (attended).
    Person,
    /// Another agent started it (a subagent with a task packet, a teammate).
    Subagent,
    /// An automation, a night item, an unattended runbook step.
    Automation,
    /// A check-in, a wake or the heartbeat.
    CheckIn,
}

impl Origin {
    /// The person's own prompts: never held, never moved off their account.
    pub fn attended(self) -> bool {
        self == Self::Person
    }
}

/// The words each kind leans on, checked in this order (the first match
/// wins). Matched on whole words of the lowercased prompt.
const CODE_WORDS: &[&str] = &[
    "hollow-slot",
    "pr",
    "pull request",
    "merge",
    "fix",
    "bug",
    "implement",
    "refactor",
    "compile",
    "build",
    "commit",
    "branch",
    "test",
    "tests",
    "cargo",
    "npm",
    "typecheck",
    "lint",
];
const REVIEW_WORDS: &[&str] = &["review", "check this diff", "code review", "audit this"];
const PLAN_WORDS: &[&str] = &["plan", "design", "roadmap", "architecture", "spec"];
const RESEARCH_WORDS: &[&str] = &[
    "research",
    "find out",
    "compare",
    "investigate",
    "look into",
];
const REPORT_WORDS: &[&str] = &[
    "report",
    "summary",
    "summarize",
    "summarise",
    "digest",
    "status update",
    "recap",
];

/// `text` with every non-alphanumeric run (except `-`) as one space, padded,
/// so a phrase matches only on word edges.
fn words(prompt: &str) -> String {
    let mut end = prompt.len().min(SCAN_BYTES);
    while !prompt.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + 2);
    out.push(' ');
    let mut space = true;
    for c in prompt[..end].chars() {
        if c.is_alphanumeric() || c == '-' {
            out.extend(c.to_lowercase());
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    if !space {
        out.push(' ');
    }
    out
}

fn has_any(text: &str, list: &[&str]) -> bool {
    list.iter().any(|w| text.contains(&format!(" {w} ")))
}

/// The job kind and a short reason for the footer. `build` is a build
/// workspace (a worktree): its turns lean Code.
pub fn classify(prompt: &str, origin: Origin, build: bool) -> (JobKind, &'static str) {
    if origin == Origin::CheckIn {
        return (JobKind::Conversation, "check-in");
    }
    let text = words(prompt);
    if has_any(&text, REVIEW_WORDS) {
        return (JobKind::Review, "review");
    }
    if has_any(&text, CODE_WORDS) {
        return (JobKind::Code, "code change");
    }
    if has_any(&text, PLAN_WORDS) {
        return (JobKind::Plan, "planning");
    }
    if has_any(&text, RESEARCH_WORDS) {
        return (JobKind::Research, "research");
    }
    if has_any(&text, REPORT_WORDS) {
        return (JobKind::Report, "report");
    }
    if build {
        return (JobKind::Code, "build workspace");
    }
    let trimmed = prompt.trim();
    if trimmed.ends_with('?') && trimmed.chars().count() <= SHORT_QUESTION_CHARS {
        return (JobKind::Question, "short question");
    }
    match origin {
        // An automation is a report unless its prompt says otherwise.
        Origin::Automation => (JobKind::Report, "automation"),
        _ => (JobKind::Conversation, "conversation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_prompts_get_their_kind() {
        let person = |p: &str| classify(p, Origin::Person, false).0;
        assert_eq!(person("Fix the failing test in chat.rs"), JobKind::Code);
        assert_eq!(person("Open a PR for this and merge it"), JobKind::Code);
        assert_eq!(person("Please review PR #1262"), JobKind::Review);
        assert_eq!(person("Check this diff for races"), JobKind::Review);
        assert_eq!(person("Draft a roadmap for Q1"), JobKind::Plan);
        assert_eq!(
            person("Research how other apps price seats and compare them"),
            JobKind::Research
        );
        assert_eq!(
            person("Write a status update for the team"),
            JobKind::Report
        );
        assert_eq!(person("What time is it in Tokyo?"), JobKind::Question);
        assert_eq!(person("thanks, that helps"), JobKind::Conversation);
        // Words match on their edges only: "prefix" is not "fix", "spr" not "pr".
        assert_eq!(person("the prefix of a sprite"), JobKind::Conversation);
        // A long question is a conversation, not a short question.
        let long = format!("{}?", "why ".repeat(80));
        assert_eq!(person(&long), JobKind::Conversation);
    }

    #[test]
    fn origin_and_workspace_lean_the_kind() {
        assert_eq!(
            classify("Good morning", Origin::CheckIn, true),
            (JobKind::Conversation, "check-in")
        );
        assert_eq!(
            classify("Make the header sticky", Origin::Person, true).0,
            JobKind::Code
        );
        assert_eq!(
            classify("Collect yesterday's calendar", Origin::Automation, false).0,
            JobKind::Report
        );
        assert_eq!(
            classify("Collect yesterday's calendar", Origin::Subagent, false).0,
            JobKind::Conversation
        );
        assert!(Origin::Person.attended() && !Origin::Subagent.attended());
    }

    #[test]
    fn only_the_first_part_of_a_long_prompt_is_read() {
        let prompt = format!("{} merge", "é".repeat(SCAN_BYTES));
        assert_eq!(
            classify(&prompt, Origin::Person, false).0,
            JobKind::Conversation
        );
    }
}
