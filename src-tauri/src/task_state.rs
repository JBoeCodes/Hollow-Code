//! The task-state record (agent effectiveness F4, wave 2 · lane K2): a
//! compact, attributable record of where a conversation's work stands, so a
//! fresh native session (a reset, a provider or mode switch, a failed
//! resume) gets more than the recent-prose tail.
//!
//! **Derived, never a source.** Built deterministically from the
//! conversation's transcript (`session_store` turns: the request, the reply,
//! the tool calls), its evidence receipts and History's after-hashes. No
//! model call. Stored per conversation outside the vault, in the session
//! store's `derived_state` table ([`KIND`]), and folded forward one finished
//! turn at a time ([`refresh`]); a missing row or an older [`SCHEMA`] is
//! rebuilt from the transcript. The transcript, the receipts and History stay
//! the source of truth.
//!
//! **What it holds** ([`Record`]), each item citing the turn it came from
//! (and the run, whose receipt holds the evidence):
//! - `goal`: the conversation's first request (a Main agent wake or
//!   check-in, [`crate::main_agent::is_wake_message`], only when every turn
//!   is one; a step retry's or friction fix's opening is a request);
//! - `constraints`: sentences of the user's requests that settle how the
//!   work is done ("always", "never", "must", "don't", "make sure", "we
//!   decided", …; never a question, never a host wake's) ([`classify`]),
//!   and every constraint a child turn's task packet carried, attributed
//!   as the packet did (RF-797-F3);
//! - `corrections`: sentences that correct the agent ("no,", "actually",
//!   "that's wrong", "I said", "you forgot", …);
//! - `steps`: the tool calls that acted (not reads, searches, question
//!   cards or read-only shell commands: [`is_read_only_shell`]), each
//!   `done`, `failed` or `unconfirmed` (no result recorded), so a fresh
//!   session doesn't repeat finished work (a check run takes its receipt's
//!   outcome: piped into `tail` or behind `|| true`, it is `failed` when its
//!   output shows a failure and `unconfirmed` when nothing says it passed:
//!   [`outcome_of`]);
//! - `open`: the questions the latest turn left (its question card's
//!   prompts and the reply's closing questions);
//! - `artifacts`: the files the turns changed, from their receipts, with
//!   History's after-hash as the revision (newest change per path).
//!
//! **Bounds.** Each text ≤ [`ITEM_CHARS`] characters (the goal
//! [`GOAL_CHARS`]), at most [`MAX_CONSTRAINTS`] constraints (the first
//! [`KEEP_FIRST_CONSTRAINTS`] stay; newer ones replace the ones after them),
//! [`MAX_CORRECTIONS`] corrections, [`MAX_STEPS`] steps, [`MAX_OPEN`] open
//! questions and [`MAX_ARTIFACTS`] artifacts (the newest), and the rendered
//! block ≤ [`MAX_BLOCK_CHARS`] characters ([`render`] drops the oldest steps,
//! artifacts and corrections first). Characters are Unicode scalar values.
//!
//! **Sent** by the context compiler on a fresh native session only, as the
//! `<task-state>` block after the conversation bundle, with the transcript
//! reference (how to read every earlier turn: `hollow_app`'s
//! `read_session`, or a truthful "can't from here" where the run has no app
//! tools). A resumed native session holds the work already and gets none.

use crate::session_store::{ConversationTool, DerivedRow, TaskTurn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The `derived_state` kind.
pub(crate) const KIND: &str = "taskState";
/// The stored payload's schema; any other is rebuilt. 2 (2026-10-07):
/// items say who asked, and a card's open questions name their card. 3
/// (2026-10-16, HB03): read-only shell commands aren't steps, and a host
/// wake isn't the goal or a constraint. 4 (2026-10-19, overnight 9 A1): a
/// request Hollow wrote ([`is_canned`]) and quoted or fenced text settle
/// nothing (`settling_text`). 5 (2026-10-20, RF-797-F2): a rule keeps the
/// long quoted path or command it names ([`settling_sentences`]).
pub(crate) const SCHEMA: u32 = 5;

/// The requests Hollow writes for the person (a canvas view, its Refresh,
/// Explain visually): `chat_send`'s `canned` argument, and the turn's
/// `asked_by` tag. Anything else sent as `canned` is the person's own words.
const CANNED: [(&str, &str); 3] = [
    ("canvas", "hollow:canvas"),
    ("canvasRefresh", "hollow:canvasRefresh"),
    ("explain", "hollow:explain"),
];

/// The `asked_by` tag of `chat_send`'s `canned` argument; `None` for an
/// absent or unknown one.
pub(crate) fn canned_asker(canned: Option<&str>) -> Option<&'static str> {
    let canned = canned?;
    CANNED
        .iter()
        .find(|(arg, _)| *arg == canned)
        .map(|(_, tag)| *tag)
}

/// Whether a turn's `asked_by` marks a request Hollow wrote ([`CANNED`]):
/// its words ("Do not change any files.") are Hollow's, never a constraint
/// or a correction.
pub(crate) fn is_canned(asked_by: Option<&str>) -> bool {
    asked_by.is_some_and(|by| CANNED.iter().any(|(_, tag)| *tag == by))
}

/// Quoted spans longer than this many characters are someone else's words.
const QUOTE_CHARS: usize = 40;

/// The part of a request whose sentences can settle anything: without
/// fenced blocks, `>` quote lines, and quoted spans (`"…"`, `“…”`, `` `…` ``,
/// a JSON string) over [`QUOTE_CHARS`] characters. A pasted review, a
/// quoted reply or a passage to explain says what someone else wrote. The
/// text [`settling_sentences`] classifies.
#[cfg(test)]
pub(crate) fn settling_text(text: &str) -> String {
    mark_long_quotes(&settling_lines(text), None)
}

/// Marks a long quote cut out of [`settling_lines`]' text: a space, then
/// `OPEN_MARK`, its index in the cut quotes, `CLOSE_MARK` (private-use
/// characters; the space ends a sentence before it as the old one did).
const OPEN_MARK: char = '\u{E000}';
const CLOSE_MARK: char = '\u{E001}';

/// The sentences of a request that can settle anything, each as it is
/// classified (`settling_text`: long quotes out, as since overnight 9 A1)
/// and as it is kept. A sentence that settles something without its long
/// quotes gets back each one that is a single phrase settling nothing itself
/// ([`restorable`]), so "Never write to `<long path>`." keeps its object
/// (RF-797-F2). A quoted passage or a quoted rule stays out.
pub(crate) fn settling_sentences(text: &str) -> Vec<(String, String)> {
    let lines = settling_lines(text);
    // Text holding the marks' own characters (a pasted icon-font glyph)
    // keeps no long quote, as before.
    if lines.contains([OPEN_MARK, CLOSE_MARK]) {
        return sentences(&mark_long_quotes(&lines, None))
            .into_iter()
            .map(|s| (s.clone(), s))
            .collect();
    }
    let mut quotes = Vec::new();
    let marked = mark_long_quotes(&lines, Some(&mut quotes));
    sentences(&marked)
        .into_iter()
        .map(|sentence| {
            let bare = unmark(&sentence, &[]);
            if !sentence.contains(OPEN_MARK) || classify(&bare) == Kind::Other {
                return (bare.clone(), bare);
            }
            let kept = unmark(&sentence, &quotes);
            (bare, kept)
        })
        .collect()
}

/// Whether a long quote goes back into a sentence that settles something
/// without it: one phrase (no line break, no sentence end inside), and a
/// `` `code` `` span, or a quote whose words settle nothing themselves. A
/// path, a command (`--prefer-offline` included) or a commit trailer comes
/// back; a quoted "always deploy on Fridays…" or "no, push to main…" doesn't.
fn restorable(quote: &str) -> bool {
    let inner = quote.trim_matches(|c| matches!(c, '"' | '`' | '\u{201C}' | '\u{201D}'));
    !quote.contains('\n')
        && sentences(inner).len() <= 1
        && (quote.starts_with('`') || classify(inner) == Kind::Other)
}

/// `text` with each mark (and the space before it) put back as its quote
/// when [`restorable`], else as one space.
fn unmark(text: &str, quotes: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != OPEN_MARK {
            out.push(c);
            continue;
        }
        if out.ends_with(' ') {
            out.pop();
        }
        let index: String = chars.by_ref().take_while(|&c| c != CLOSE_MARK).collect();
        match index.parse::<usize>().ok().and_then(|i| quotes.get(i)) {
            Some(quote) if restorable(quote) => out.push_str(quote),
            _ => out.push(' '),
        }
    }
    out
}

/// A request without fenced blocks and `>` quote lines.
fn settling_lines(text: &str) -> String {
    let mut kept = Vec::new();
    let mut fence: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            }
            continue;
        }
        // A fence opens on its own line: ```` ```npm ci``` ```` is inline code.
        if (trimmed.starts_with("```") || trimmed.starts_with("~~~"))
            && !trimmed[3..].contains(&trimmed[..3])
        {
            fence = Some(&trimmed[..3]);
            continue;
        }
        if trimmed.starts_with('>') {
            continue;
        }
        kept.push(line);
    }
    kept.join("\n")
}

/// `text` with each quoted span over [`QUOTE_CHARS`] characters replaced by
/// one space, or, given `quotes`, by a space and a mark ([`OPEN_MARK`]),
/// the span (its quote marks included) pushed onto `quotes`. A span runs
/// from an opening mark to its closing one (a backslash escapes a straight
/// `"`, as in JSON); an unclosed mark is kept as text.
fn mark_long_quotes(text: &str, mut quotes: Option<&mut Vec<String>>) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let close = match chars[i] {
            '"' => Some('"'),
            '\u{201C}' => Some('\u{201D}'),
            '`' => Some('`'),
            _ => None,
        };
        if let Some(close) = close {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != close {
                if chars[j] == '\\' && close == '"' {
                    j += 1;
                }
                j += 1;
            }
            if j < chars.len() {
                // A short quote stays whole, so its closing mark never opens
                // the next span.
                if j - i - 1 > QUOTE_CHARS {
                    out.push(' ');
                    if let Some(quotes) = quotes.as_deref_mut() {
                        out.push(OPEN_MARK);
                        out.push_str(&quotes.len().to_string());
                        out.push(CLOSE_MARK);
                        quotes.push(chars[i..=j].iter().collect());
                    }
                } else {
                    out.extend(&chars[i..=j]);
                }
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub(crate) const OPEN: &str = "<task-state>";
pub(crate) const CLOSE: &str = "</task-state>";

/// The block's explanatory sentence, when the user asked every turn the
/// record cites.
pub(crate) const HEADER: &str = "This is Hollow's task-state record for this conversation, derived from its transcript, tool receipts and History (they stay the source of truth). Each item cites the turn it came from (and the run, whose receipt holds the evidence). Constraints and corrections are what the user said in those turns and still apply unless the user changes them. Steps marked done are finished: don't repeat them unless the user asks; check a step marked unconfirmed before redoing it.";

/// The header when another agent asked a cited turn (it started or messaged
/// this conversation through `hollow_app`): who asked is said per item
/// ("parent": that agent added it to its task packet, RF-797-F3; the
/// shorter "cites its turn" paid for the added bytes).
pub(crate) const HEADER_AGENT: &str = "This is Hollow's task-state record for this conversation, derived from its transcript, tool receipts and History (they stay the source of truth). Each item cites its turn (and the run, whose receipt holds the evidence). Constraints and corrections are what was asked in those turns: by the user, or, for an item marked \"by\":\"agent\" or \"parent\", by another agent that started or messaged this conversation through Hollow's app tools. They still apply unless whoever asked changes them. Steps marked done are finished: don't repeat them unless asked; check a step marked unconfirmed before redoing it.";

/// Characters kept of each item's text.
pub(crate) const ITEM_CHARS: usize = 280;
/// Characters kept of the goal.
pub(crate) const GOAL_CHARS: usize = 400;
/// Characters kept of a step's target.
const TARGET_CHARS: usize = 160;
pub(crate) const MAX_CONSTRAINTS: usize = 12;
/// The earliest constraints are kept when newer ones overflow the cap.
pub(crate) const KEEP_FIRST_CONSTRAINTS: usize = 4;
pub(crate) const MAX_CORRECTIONS: usize = 6;
pub(crate) const MAX_STEPS: usize = 16;
pub(crate) const MAX_OPEN: usize = 4;
pub(crate) const MAX_ARTIFACTS: usize = 12;
/// The rendered block's ceiling, in characters.
pub(crate) const MAX_BLOCK_CHARS: usize = 8_000;

/// One cited statement.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    pub text: String,
    /// The turn (`turns.ord`) it came from.
    pub turn: i64,
    /// The earlier conversation it came from, when not this one (a
    /// continued conversation's merged record).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// `agent` when another agent asked the turn (`hollow_app`
    /// `start_session` / `send_to_session`); absent when the user did. A
    /// constraint the turn's task packet carried keeps the packet's `from`
    /// (RF-797-F3): absent for `user`, else `agent` or `parent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// An open question's card (`<run id>/<tool use id>`), so a card the
    /// user has answered is no longer listed ([`retain_open_cards`]). Never
    /// rendered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<String>,
}

/// One tool call that acted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Step {
    pub turn: i64,
    pub tool: String,
    /// What it acted on: a path, a command, a title (the first of a fixed
    /// list of input fields), cut to 160 characters.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    /// `done`, `failed` or `unconfirmed`.
    pub outcome: String,
    /// The run (its evidence receipt and History run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// One file a turn changed, as its receipt lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Artifact {
    pub path: String,
    /// `create`, `modify` or `delete`.
    pub change: String,
    /// History's SHA-256 of the file after the turn: its revision. Absent
    /// for a delete, a repository file and an uncaptured version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub turn: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

/// See the module doc. Field order is the rendered order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
}

impl Record {
    /// Every cited item.
    pub(crate) fn items(&self) -> usize {
        usize::from(self.goal.is_some())
            + self.constraints.len()
            + self.corrections.len()
            + self.steps.len()
            + self.open.len()
            + self.artifacts.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items() == 0
    }
}

/// The stored payload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    schema: u32,
    record: Record,
}

/// A conversation's record as of its last folded turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct State {
    pub record: Record,
    /// Bumped each time the record's content changes; 0 before any.
    pub revision: i64,
    /// The last turn folded in; 0 before any.
    pub through_turn: i64,
}

// ── Reading the user's words ──────────────────────────────────────────────

/// What a sentence of the user's request is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Constraint,
    Correction,
    Other,
}

/// Openings that correct the agent.
const CORRECTION_OPENINGS: [&str; 17] = [
    "no,",
    "no.",
    "nope",
    "actually",
    "wrong",
    "that's wrong",
    "that is wrong",
    "that's not",
    "that is not",
    "not what i",
    "i said",
    "i meant",
    "i asked",
    "instead,",
    "undo",
    "revert",
    "stop ",
];

/// Phrases anywhere in a sentence that correct the agent.
const CORRECTION_PHRASES: [&str; 7] = [
    "you forgot",
    "you missed",
    "should have",
    "shouldn't have",
    "should not have",
    "not what i asked",
    "i told you",
];

/// Whole words or phrases that settle how the work is done.
const CONSTRAINT_PHRASES: [&str; 24] = [
    "always",
    "never",
    "must",
    "mustn't",
    "must not",
    "required",
    "requirement",
    "constraint",
    "avoid",
    "do not",
    "don't",
    "make sure",
    "be sure",
    "no more than",
    "at most",
    "at least",
    "we decided",
    "we agreed",
    "let's use",
    "stick to",
    "keep it",
    "only use",
    "prefer",
    "settled",
];

/// `text` lowercased with every character that isn't a letter, digit or
/// apostrophe as a space, padded with spaces, so a phrase matches whole
/// words: `" {phrase} "`.
fn word_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push(' ');
    for c in text.chars() {
        let c = if c == '\u{2019}' { '\'' } else { c };
        if c.is_alphanumeric() || c == '\'' {
            out.extend(c.to_lowercase());
        } else {
            out.push(' ');
        }
    }
    out.push(' ');
    // One space between words.
    let mut squeezed = String::with_capacity(out.len());
    let mut last_space = false;
    for c in out.chars() {
        if c == ' ' {
            if !last_space {
                squeezed.push(c);
            }
            last_space = true;
        } else {
            squeezed.push(c);
            last_space = false;
        }
    }
    squeezed
}

/// Classify one sentence of the user's request. A question is never a
/// constraint or a correction; a correction wins over a constraint.
pub(crate) fn classify(sentence: &str) -> Kind {
    let trimmed = sentence.trim();
    if trimmed.ends_with('?') || crate::context_compiler::char_count(trimmed) < 8 {
        return Kind::Other;
    }
    let lower = trimmed.to_lowercase().replace('\u{2019}', "'");
    let words = word_text(trimmed);
    if CORRECTION_OPENINGS
        .iter()
        .any(|opening| lower.starts_with(opening))
        || CORRECTION_PHRASES
            .iter()
            .any(|phrase| words.contains(&format!(" {phrase} ")))
    {
        return Kind::Correction;
    }
    if CONSTRAINT_PHRASES
        .iter()
        .any(|phrase| words.contains(&format!(" {phrase} ")))
    {
        return Kind::Constraint;
    }
    Kind::Other
}

/// The sentences of `text`: each line, split after `.`, `!` or `?` followed
/// by whitespace. Trimmed, empty ones dropped.
pub(crate) fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        let mut current = String::new();
        for (i, c) in chars.iter().enumerate() {
            current.push(*c);
            let ends = matches!(c, '.' | '!' | '?')
                && chars.get(i + 1).is_none_or(|next| next.is_whitespace());
            if ends {
                let sentence = current.trim().to_string();
                if !sentence.is_empty() {
                    out.push(sentence);
                }
                current.clear();
            }
        }
        let sentence = current.trim().to_string();
        if !sentence.is_empty() {
            out.push(sentence);
        }
    }
    out
}

/// `text` on one line (whitespace runs as one space), cut to `max`
/// characters with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if crate::context_compiler::char_count(&one_line) <= max {
        one_line
    } else {
        let mut cut: String = one_line.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    }
}

fn normalized(text: &str) -> String {
    word_text(text).trim().to_string()
}

// ── Tool calls ────────────────────────────────────────────────────────────

/// Name prefixes (of the part after the last `__`, lowercased) of tools that
/// only read: reads, listings, searches, fetches and planning notes.
const READ_PREFIXES: [&str; 15] = [
    "read",
    "list",
    "get",
    "search",
    "describe",
    "find",
    "view",
    "glob",
    "grep",
    "ls",
    "web",
    "todo",
    "toolsearch",
    "notebookread",
    "status",
];

/// Whether `name` is a tool that didn't act: a read (see
/// [`READ_PREFIXES`]) or a question card (an open decision instead).
pub(crate) fn is_read_only(name: &str) -> bool {
    if name.contains("hollow_questions") || name.contains("ask_user_questions") {
        return true;
    }
    let base = name.rsplit("__").next().unwrap_or(name).to_lowercase();
    READ_PREFIXES.iter().any(|prefix| base.starts_with(prefix))
}

/// Whether `tool` is a shell call (`Bash`, Codex's `exec_command`, …; a
/// `<shell> -lc '<script>'` wrapper taken off) that only read: `grep`,
/// `cat`, `sed -n`, `ls`, `git status`, … ([`crate::run_effects::classify_shell`]).
/// A test, build, typecheck or lint run, which that classifier also calls
/// read-only (it has no outside effect), is still a step: its result is
/// work done. An unknown command (`python3 …`) is a step.
pub(crate) fn is_read_only_shell(tool: &ConversationTool) -> bool {
    let Some(command) = crate::run_effects::shell_command(&tool.name, &tool.input) else {
        return false;
    };
    crate::evidence::check_kind(&command).is_none()
        && crate::run_effects::classify_shell(&command) == crate::run_effects::Shell::ReadOnly
}

/// Whether `asked` is a Main agent wake or check-in, not a request. A step
/// retry's or a friction fix's opening also starts "⟲ Hollow:" but is the
/// conversation's task, so it counts as a request.
fn is_host(asked: &str) -> bool {
    crate::main_agent::is_wake_message(asked)
}

/// Whether goal `next` takes the place of `kept`: the first goal stays,
/// unless it is a Main agent wake and `next` isn't.
fn replaces_goal(kept: Option<&Item>, next: &Item) -> bool {
    kept.is_none_or(|kept| is_host(&kept.text) && !is_host(&next.text))
}

/// Input fields that name what a tool acted on, in order of preference.
const TARGET_FIELDS: [&str; 12] = [
    "file_path",
    "path",
    "notebook_path",
    "command",
    "cmd",
    "url",
    "title",
    "name",
    "key",
    "slug",
    "id",
    "description",
];

/// What a tool call acted on, through `claude_runner::redact` (as
/// `hollow_app`'s `read_session` hands tool inputs to an agent): a key a
/// command once carried doesn't travel on in the handoff.
fn target_of(input: &serde_json::Value) -> String {
    clip(
        &crate::claude_runner::redact(&raw_target(input)),
        TARGET_CHARS,
    )
}

fn raw_target(input: &serde_json::Value) -> String {
    if let Some(text) = input.as_str() {
        return clip(text, TARGET_CHARS);
    }
    for field in TARGET_FIELDS {
        match input.get(field) {
            Some(serde_json::Value::String(text)) if !text.trim().is_empty() => {
                return clip(text, TARGET_CHARS);
            }
            Some(serde_json::Value::Array(parts)) if !parts.is_empty() => {
                let joined = parts
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ");
                if !joined.trim().is_empty() {
                    return clip(&joined, TARGET_CHARS);
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// `failed`, `done` or `unconfirmed` (no result recorded, or a check run
/// whose outcome nothing confirms). A shell step that ran a check takes the
/// receipt's check outcome (OF16, `evidence::shell_checks`): `failed` when a
/// check failed (its runner's failure text, though the call itself didn't
/// error: a pipe into `tail`, `|| true`), `done` when every check passed,
/// and `unconfirmed` when one's outcome is unknown (`npx vitest run | tail`
/// with no summary): the exit status of `tail` isn't the tests'.
///
/// The outcome is the step spine's `checkOutcome` (`checked`, by tool-use
/// id), read from the call's full result (OF16 re-review). A turn recorded
/// before it has none: the transcript's stored result is read instead, the
/// first ~8,000 characters of the tool-result event (`session_store`'s
/// event cap, marked `… (truncated)`, so a cut result never passes on its
/// text), and a summary past that is missed.
fn outcome_of(tool: &ConversationTool, checked: Option<&str>) -> &'static str {
    if tool.is_error || tool.status == "error" {
        return "failed";
    }
    if tool.status != "done" {
        return "unconfirmed";
    }
    match checked.or_else(|| stored_check_outcome(tool)) {
        Some("failed") => "failed",
        Some("passed") | None => "done",
        Some(_) => "unconfirmed",
    }
}

/// The receipt's outcome of finished shell step `tool`'s checks, read from
/// the transcript's stored result; None for a call that holds no check.
fn stored_check_outcome(tool: &ConversationTool) -> Option<&'static str> {
    let command = crate::run_effects::shell_command(&tool.name, &tool.input)?;
    let output = tool.result.as_deref().unwrap_or("");
    crate::evidence::outcome_of_output(&command, output, tool.is_error)
}

/// The question prompts of the turn's question-card calls, each with its
/// call's tool-use id.
fn card_questions(tools: &[ConversationTool]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for tool in tools {
        if !(tool.name.contains("hollow_questions") || tool.name.contains("ask_user_questions")) {
            continue;
        }
        for question in tool
            .input
            .get("questions")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(prompt) = question.get("prompt").and_then(serde_json::Value::as_str) {
                out.push((tool.id.clone(), prompt.to_string()));
            }
        }
    }
    out
}

// ── Receipts ──────────────────────────────────────────────────────────────

/// What one run's receipt says it changed: (path, change, after-hash).
type Changes = Vec<(String, String, Option<String>)>;

/// The chat receipts' changes, by run id, read once per [`refresh`] that
/// has turns to fold.
pub(crate) struct Receipts {
    by_run: HashMap<String, Changes>,
}

impl Receipts {
    /// The chat receipts of `runs` and, for each vault file one lists, the
    /// after-hash its History run recorded (one History read per receipt
    /// that changed a vault file). A store that can't be read reads as no
    /// receipts.
    pub(crate) fn load(vault: &Path, runs: &HashSet<&str>) -> Self {
        let mut by_run = HashMap::new();
        if runs.is_empty() {
            return Receipts { by_run };
        }
        let receipts = crate::evidence::list(vault, Some("chat"), None, crate::evidence::KEEP)
            .unwrap_or_default();
        for receipt in receipts {
            if !runs.contains(receipt.run_id.as_str())
                || by_run.contains_key(&receipt.run_id)
                || (receipt.files.is_empty() && receipt.repo_changes.is_empty())
            {
                continue;
            }
            let hashes: HashMap<String, Option<String>> = if receipt.files.is_empty() {
                HashMap::new()
            } else {
                crate::automated_history::get_run(vault, &receipt.history_run_id)
                    .map(|run| {
                        run.changes
                            .into_iter()
                            .map(|change| (change.vault_path, change.after_hash))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            // A file another chat in the folder changed is not this
            // conversation's artifact (chat concurrency F1, 2026-10-20).
            let mut changes: Changes = receipt
                .files
                .iter()
                .filter(|file| file.is_own())
                .map(|file| {
                    (
                        file.path.clone(),
                        file.change_type.clone(),
                        hashes.get(&file.path).cloned().flatten(),
                    )
                })
                .collect();
            for repo in &receipt.repo_changes {
                for file in &repo.files {
                    changes.push((
                        format!("{}/{}", repo.root.trim_end_matches('/'), file.path),
                        file.change_type.clone(),
                        None,
                    ));
                }
            }
            by_run.insert(receipt.run_id.clone(), changes);
        }
        Receipts { by_run }
    }

    /// No receipts: a fold that reads only the turns' own text and tools
    /// (`delegation::Parent::of_conversation`, F10 D1).
    pub(crate) fn none() -> Self {
        Receipts {
            by_run: HashMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn of(by_run: HashMap<String, Changes>) -> Self {
        Receipts { by_run }
    }

    fn changes(&self, run_id: Option<&str>) -> &[(String, String, Option<String>)] {
        run_id
            .and_then(|id| self.by_run.get(id))
            .map_or(&[], Vec::as_slice)
    }
}

// ── Folding ───────────────────────────────────────────────────────────────

fn push_capped<T>(list: &mut Vec<T>, item: T, cap: usize) {
    list.push(item);
    let excess = list.len().saturating_sub(cap);
    list.drain(..excess);
}

/// Add constraint `item` (its text already clipped) unless the record
/// holds it; past [`MAX_CONSTRAINTS`] the first [`KEEP_FIRST_CONSTRAINTS`]
/// stay and the oldest after them goes. A rule already held is renewed
/// (RF-R2m-F1, D1): it moves to `item`'s turn, so the ask window counts
/// from the newest packet or request that said it, and becomes the
/// person's when the person said it (`item` unmarked). Past the kept
/// first ones it also moves to the end (D2), so the list stays in turn
/// order for the cap here and `render`'s drops.
fn push_constraint(record: &mut Record, item: Item) {
    // As stored (clipped), so a long rule said again matches itself.
    let key = normalized(&item.text);
    if let Some(at) = record
        .constraints
        .iter()
        .position(|kept| normalized(&kept.text) == key)
    {
        let kept = &mut record.constraints[at];
        kept.turn = kept.turn.max(item.turn);
        if item.by.is_none() {
            kept.by = None;
        }
        if at >= KEEP_FIRST_CONSTRAINTS {
            let kept = record.constraints.remove(at);
            record.constraints.push(kept);
        }
        return;
    }
    record.constraints.push(item);
    if record.constraints.len() > MAX_CONSTRAINTS {
        record.constraints.remove(KEEP_FIRST_CONSTRAINTS);
    }
}

/// Fold one turn into `record`: a finished one ([`refresh`]), or one still
/// `running` ([`peek`]), whose calls still waiting on a result aren't steps
/// yet and whose finished ones are `unconfirmed` (the turn may still fail).
pub(crate) fn fold(record: &mut Record, turn: &TaskTurn, receipts: &Receipts) {
    let asked = crate::next_step::strip(&turn.asked);
    // Who asked: the user, or another agent through `hollow_app`.
    let by = turn.asked_by.clone().filter(|by| by == "agent");
    let cite = |text: &str, max: usize| Item {
        text: clip(text, max),
        turn: turn.ord,
        by: by.clone(),
        ..Item::default()
    };
    // A Main agent wake is the goal only until a request comes: when every
    // turn is one, the first stays.
    if !asked.trim().is_empty() {
        let goal = cite(&asked, GOAL_CHARS);
        if replaces_goal(record.goal.as_ref(), &goal) {
            record.goal = Some(goal);
        }
    }
    // A wake's instructions ("don't start new work…") and a request Hollow
    // wrote (a canvas view's "Do not change any files.") are Hollow's, not
    // the requester's: never a constraint or a correction. Nor is what the
    // request quotes.
    let said = if is_host(&asked) || is_canned(turn.asked_by.as_deref()) {
        Vec::new()
    } else {
        settling_sentences(&asked)
    };
    // The turn's task packet's constraints first (RF-797-F3): every one is
    // a rule, attributed as the packet did, and cites this turn, so the
    // packets of this conversation's own children carry them under the
    // same age window as the rest.
    for (text, from) in &turn.packet_constraints {
        let item = Item {
            by: (from.as_str() != "user").then(|| from.clone()),
            ..cite(text, ITEM_CHARS)
        };
        push_constraint(record, item);
    }
    for (bare, sentence) in said {
        match classify(&bare) {
            Kind::Constraint => push_constraint(record, cite(&sentence, ITEM_CHARS)),
            Kind::Correction => {
                push_capped(
                    &mut record.corrections,
                    cite(&sentence, ITEM_CHARS),
                    MAX_CORRECTIONS,
                );
            }
            Kind::Other => {}
        }
    }
    // Mid-turn steering: what the person said to the running turn is their
    // direction too, settled the same way, and always theirs (never the
    // agent that asked the turn).
    for steer in &turn.steers {
        let steer = crate::next_step::strip(steer);
        if is_host(&steer) {
            continue;
        }
        let theirs = |text: &str| Item {
            text: clip(text, ITEM_CHARS),
            turn: turn.ord,
            ..Item::default()
        };
        for (bare, sentence) in settling_sentences(&steer) {
            match classify(&bare) {
                Kind::Constraint => push_constraint(record, theirs(&sentence)),
                Kind::Correction => {
                    push_capped(&mut record.corrections, theirs(&sentence), MAX_CORRECTIONS);
                }
                Kind::Other => {}
            }
        }
    }
    // The COO's steer of a chat it started (`sessions.steer_session`, L13):
    // the asking agent's direction, attributed to it like a turn it asked,
    // never the person's.
    for steer in &turn.agent_steers {
        let steer = crate::next_step::strip(steer);
        if is_host(&steer) {
            continue;
        }
        let agents = |text: &str| Item {
            text: clip(text, ITEM_CHARS),
            turn: turn.ord,
            by: Some("agent".to_string()),
            ..Item::default()
        };
        for (bare, sentence) in settling_sentences(&steer) {
            match classify(&bare) {
                Kind::Constraint => push_constraint(record, agents(&sentence)),
                Kind::Correction => {
                    push_capped(&mut record.corrections, agents(&sentence), MAX_CORRECTIONS);
                }
                Kind::Other => {}
            }
        }
    }
    let running = turn.status.as_deref() == Some("running");
    for tool in &turn.tools {
        if is_read_only(&tool.name)
            || is_read_only_shell(tool)
            || (running && tool.status == "running")
        {
            continue;
        }
        let check = turn.check_outcomes.get(&tool.id).map(String::as_str);
        let outcome = match outcome_of(tool, check) {
            "done" if running => "unconfirmed",
            outcome => outcome,
        };
        push_capped(
            &mut record.steps,
            Step {
                turn: turn.ord,
                tool: tool.name.clone(),
                target: target_of(&tool.input),
                outcome: outcome.to_string(),
                run: turn.run_id.clone(),
                session: None,
            },
            MAX_STEPS,
        );
    }
    // The latest turn's open questions replace the last ones: a later turn
    // answered them or moved on.
    let reply = crate::next_step::strip(&turn.reply);
    // The agent of this conversation asked these, never the requester.
    let asks = |text: &str, card: Option<String>| Item {
        text: clip(text, ITEM_CHARS),
        turn: turn.ord,
        card,
        ..Item::default()
    };
    let mut open: Vec<Item> = card_questions(&turn.tools)
        .iter()
        .map(|(tool, question)| {
            let card = turn.run_id.as_ref().map(|run| format!("{run}/{tool}"));
            asks(question, card)
        })
        .collect();
    let closing: Vec<String> = sentences(&reply)
        .into_iter()
        .rev()
        .take(3)
        .filter(|sentence| sentence.ends_with('?'))
        .collect();
    for question in closing.into_iter().rev() {
        open.push(asks(&question, None));
    }
    open.truncate(MAX_OPEN);
    record.open = open;
    for (path, change, sha256) in receipts.changes(turn.run_id.as_deref()) {
        record.artifacts.retain(|artifact| &artifact.path != path);
        push_capped(
            &mut record.artifacts,
            Artifact {
                path: path.clone(),
                change: change.clone(),
                sha256: sha256.clone(),
                turn: turn.ord,
                run: turn.run_id.clone(),
                session: None,
            },
            MAX_ARTIFACTS,
        );
    }
}

/// Conversation `key`'s record, folded forward through every finished turn
/// before `before_ord` and stored. Folding stops at a turn still `running`
/// (it is folded once it ends; [`peek`] reads it before then). The revision is bumped only when the
/// record's content changed. A row from another schema is rebuilt from the
/// transcript.
pub(crate) fn refresh(vault: &Path, key: &str, before_ord: i64) -> Result<State, String> {
    let row = crate::session_store::derived_state(vault, key, KIND)?;
    let (mut state, rebuilt) = match row.as_ref().map(|row| {
        (
            row,
            serde_json::from_str::<Stored>(&row.payload)
                .ok()
                .filter(|stored| stored.schema == SCHEMA),
        )
    }) {
        Some((row, Some(stored))) => (
            State {
                record: stored.record,
                revision: row.revision,
                through_turn: row.through_ord,
            },
            false,
        ),
        // Rebuilt from the transcript, counting on from the old revision.
        Some((row, None)) => (
            State {
                revision: row.revision,
                ..State::default()
            },
            true,
        ),
        None => (State::default(), false),
    };
    let turns = crate::session_store::task_turns(vault, key, state.through_turn, before_ord)?;
    let ready: Vec<&TaskTurn> = turns
        .iter()
        .take_while(|turn| turn.status.as_deref() != Some("running"))
        .collect();
    if ready.is_empty() && !rebuilt {
        return Ok(state);
    }
    let before = state.record.clone();
    let runs: HashSet<&str> = ready
        .iter()
        .filter_map(|turn| turn.run_id.as_deref())
        .collect();
    let receipts = Receipts::load(vault, &runs);
    for turn in ready {
        fold(&mut state.record, turn, &receipts);
        state.through_turn = turn.ord;
    }
    if state.record != before || (rebuilt && !state.record.is_empty()) {
        state.revision += 1;
    }
    let payload = serde_json::to_string(&Stored {
        schema: SCHEMA,
        record: state.record.clone(),
    })
    .map_err(|e| format!("task state encode: {e}"))?;
    crate::session_store::put_derived_state(
        vault,
        key,
        KIND,
        &DerivedRow {
            revision: state.revision,
            through_ord: state.through_turn,
            payload,
        },
    )?;
    Ok(state)
}

/// The stored record's revision, last folded turn and item count, without
/// folding anything (`TurnRequest`'s admission-time reference).
pub(crate) fn stored(vault: &Path, key: &str) -> Option<State> {
    let row = crate::session_store::derived_state(vault, key, KIND).ok()??;
    let stored = serde_json::from_str::<Stored>(&row.payload)
        .ok()
        .filter(|stored| stored.schema == SCHEMA)?;
    Some(State {
        record: stored.record,
        revision: row.revision,
        through_turn: row.through_ord,
    })
}

/// Conversation `key`'s record as it stands now, read from another
/// conversation (a handover's successor or a Continue, OB14): the stored
/// record with every turn not yet folded into it, the running one included
/// ([`fold`]), folded into a copy, with their receipts. A handover is its
/// predecessor's last action inside its running turn, and the successor
/// starts at once, so [`refresh`] (which stops at that turn) would give it
/// nothing. Read only: nothing is stored and `through_turn` stays the
/// stored one, so the conversation's own [`refresh`] folds the turn once it
/// ends. A missing row or another schema folds from the first turn.
pub(crate) fn peek(vault: &Path, key: &str) -> Result<State, String> {
    peek_turns(vault, key, usize::MAX, true).map(|(state, _)| state)
}

/// [`peek`] folding only the `recent` newest pending turns, with their
/// receipts only when `with_receipts`, and returning those turns
/// (`delegation::Parent::of_conversation`, F10 D1, reads their newest
/// request).
pub(crate) fn peek_turns(
    vault: &Path,
    key: &str,
    recent: usize,
    with_receipts: bool,
) -> Result<(State, Vec<TaskTurn>), String> {
    let mut state = stored(vault, key).unwrap_or_default();
    let mut pending = crate::session_store::task_turns(vault, key, state.through_turn, i64::MAX)?;
    let skip = pending.len().saturating_sub(recent);
    pending.drain(..skip);
    let receipts = if with_receipts {
        let runs: HashSet<&str> = pending
            .iter()
            .filter_map(|turn| turn.run_id.as_deref())
            .collect();
        Receipts::load(vault, &runs)
    } else {
        Receipts::none()
    };
    for turn in &pending {
        fold(&mut state.record, turn, &receipts);
    }
    Ok((state, pending))
}

/// One record from the continued conversations' records (`sources`, oldest
/// first, each with its key) and this conversation's own (`own`): the
/// earliest goal; the sources' items before the own ones, each tagged with
/// its conversation; the own open questions, else the newest source's; then
/// the caps again.
pub(crate) fn merged(sources: &[(String, Record)], own: &Record) -> Record {
    if sources.is_empty() {
        return own.clone();
    }
    let tag_item = |item: &Item, key: &str| Item {
        session: Some(key.to_string()),
        ..item.clone()
    };
    let mut out = Record::default();
    for (key, record) in sources {
        // The same goal rule as a fold: a wake gives way to a request.
        let goal = record.goal.as_ref();
        if let Some(goal) = goal.filter(|goal| replaces_goal(out.goal.as_ref(), goal)) {
            out.goal = Some(tag_item(goal, key));
        }
        out.constraints
            .extend(record.constraints.iter().map(|item| tag_item(item, key)));
        out.corrections
            .extend(record.corrections.iter().map(|item| tag_item(item, key)));
        out.steps.extend(record.steps.iter().map(|step| Step {
            session: Some(key.clone()),
            ..step.clone()
        }));
        out.artifacts
            .extend(record.artifacts.iter().map(|artifact| Artifact {
                session: Some(key.clone()),
                ..artifact.clone()
            }));
        if !record.open.is_empty() {
            out.open = record.open.iter().map(|item| tag_item(item, key)).collect();
        }
    }
    let goal = own.goal.as_ref();
    if let Some(goal) = goal.filter(|goal| replaces_goal(out.goal.as_ref(), goal)) {
        out.goal = Some(goal.clone());
    }
    out.constraints.extend(own.constraints.iter().cloned());
    out.corrections.extend(own.corrections.iter().cloned());
    out.steps.extend(own.steps.iter().cloned());
    for artifact in &own.artifacts {
        out.artifacts.retain(|kept| kept.path != artifact.path);
        out.artifacts.push(artifact.clone());
    }
    if !own.open.is_empty() || own.goal.is_some() {
        out.open = own.open.clone();
    }
    while out.constraints.len() > MAX_CONSTRAINTS {
        out.constraints.remove(KEEP_FIRST_CONSTRAINTS);
    }
    let trim = |len: usize, cap: usize| len.saturating_sub(cap);
    let excess = trim(out.corrections.len(), MAX_CORRECTIONS);
    out.corrections.drain(..excess);
    let excess = trim(out.steps.len(), MAX_STEPS);
    out.steps.drain(..excess);
    let excess = trim(out.artifacts.len(), MAX_ARTIFACTS);
    out.artifacts.drain(..excess);
    out.open.truncate(MAX_OPEN);
    out
}

// ── Rendering ─────────────────────────────────────────────────────────────

/// How the agent can read the earlier turns this record and the bundle
/// summarize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reader {
    /// `hollow_app` is attached: its `read_session` tool reads them.
    AppTools,
    /// No Hollow app tools in this run (an agent without owned MCP, or app
    /// tools switched off).
    None,
}

/// The transcript reference the block ends with.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Transcript<'a> {
    session: &'a str,
    turns: i64,
    read: String,
}

/// The block's JSON: the record's fields, then `transcript`.
#[derive(Serialize)]
struct Block<'a> {
    #[serde(flatten)]
    record: &'a Record,
    transcript: &'a Transcript<'a>,
}

/// A rendered `<task-state>` block and what went into it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Rendered {
    pub text: String,
    pub items: usize,
}

/// The most turns `hollow_app`'s `read_session` (`sessions.conversation`)
/// returns: its `lastTurns` is `Kind::Int(100)`, and a larger value is
/// rejected, not clamped.
pub(crate) const READ_SESSION_MAX_TURNS: i64 = 100;

/// Leave out the open questions whose card is no longer open: a card the
/// user answered (its answer is this turn, or an earlier one), dismissed,
/// or that was superseded or expired, and a question-card call that never
/// became a card. A card still waiting (`pending`, `stale`) stays. Each
/// item's conversation is its `session` (a continued conversation's merged
/// item) or `key`. A store that can't be read leaves the list as it is.
pub(crate) fn retain_open_cards(vault: &Path, key: &str, record: &mut Record) {
    if record.open.iter().all(|item| item.card.is_none()) {
        return;
    }
    let mut open: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    record.open.retain(|item| {
        let Some(card) = &item.card else {
            return true;
        };
        let conversation = item.session.as_deref().unwrap_or(key);
        let cards = open.entry(conversation.to_string()).or_insert_with(|| {
            crate::interactions::list_pending(vault, conversation)
                .ok()
                .map(|cards| {
                    cards
                        .into_iter()
                        .map(|card| format!("{}/{}", card.run_id, card.tool_use_id))
                        .collect()
                })
        });
        cards.as_ref().is_none_or(|cards| cards.contains(card))
    });
}

/// The header for `record`: [`HEADER_AGENT`] when another agent asked any
/// item it cites, else [`HEADER`].
fn header_for(record: &Record) -> &'static str {
    let agent = |item: &Item| item.by.is_some();
    if record.goal.iter().any(agent)
        || record.constraints.iter().any(agent)
        || record.corrections.iter().any(agent)
    {
        HEADER_AGENT
    } else {
        HEADER
    }
}

/// Every item's text cut to `max` characters (the goal too, and a step's
/// target); an artifact's path keeps its last `max` characters.
fn shrink(record: &mut Record, max: usize) {
    for item in record
        .goal
        .iter_mut()
        .chain(&mut record.constraints)
        .chain(&mut record.corrections)
        .chain(&mut record.open)
    {
        item.text = clip(&item.text, max);
    }
    for step in &mut record.steps {
        step.target = clip(&step.target, max);
    }
    for artifact in &mut record.artifacts {
        if crate::context_compiler::char_count(&artifact.path) > max {
            artifact.path = format!(
                "…{}",
                crate::context_compiler::last_chars(&artifact.path, max.saturating_sub(1))
            );
        }
    }
}

/// The last resort past [`shrink`]: drop one item, least needed first
/// (open questions, steps, artifacts, corrections, the newest constraints,
/// then the goal). `false` when nothing is left to drop.
fn drop_last_resort(record: &mut Record) -> bool {
    if record.open.pop().is_some()
        || (!record.steps.is_empty() && {
            record.steps.remove(0);
            true
        })
        || (!record.artifacts.is_empty() && {
            record.artifacts.remove(0);
            true
        })
        || (!record.corrections.is_empty() && {
            record.corrections.remove(0);
            true
        })
        || record.constraints.pop().is_some()
    {
        return true;
    }
    record.goal.take().is_some()
}

/// The `<task-state>` block for `record` (empty for an empty record): the
/// header (who asked: [`header_for`]), then one JSON object (the record,
/// then `transcript`: the session key, the earlier turns' count and how to
/// read them), `<` written as `\u003c` so nothing inside can close the
/// block.
///
/// [`MAX_BLOCK_CHARS`] is a hard ceiling on the rendered text, escaping
/// included. Over it, the oldest steps, then artifacts, then corrections,
/// then the newer constraints are dropped (a few of each are kept); then
/// every item's text is cut shorter (to 140, 70, 35 and 16 characters);
/// then, as a last resort, items go one by one ([`drop_last_resort`]). A
/// card's reference is never rendered.
///
/// `stored` is the turns the session store holds for `key`, the current
/// one included: its request is stored before the body is compiled, and
/// `read_session` counts it, so `lastTurns` = `stored` is what reaches
/// turn 1. Past [`READ_SESSION_MAX_TURNS`] the reference asks for the
/// latest 100 and says the older ones are covered only by this record.
pub(crate) fn render(record: &Record, key: &str, stored: i64, reader: Reader) -> Rendered {
    if record.is_empty() {
        return Rendered::default();
    }
    let stored = stored.max(1);
    let turns = stored - 1;
    let read = match reader {
        Reader::AppTools if stored <= READ_SESSION_MAX_TURNS => format!(
            "hollow_app read_session with {{\"key\":\"{key}\",\"lastTurns\":{stored},\"tools\":\"full\"}} returns every earlier turn (and this one): the request, the reply, the tool calls and each run's receipt"
        ),
        Reader::AppTools => format!(
            "hollow_app read_session with {{\"key\":\"{key}\",\"lastTurns\":{max},\"tools\":\"full\"}} returns the latest {max} turns (this one included): the request, the reply, the tool calls and each run's receipt. It reads no further back, so the {older} turns before those are covered only by this record",
            max = READ_SESSION_MAX_TURNS,
            older = stored - READ_SESSION_MAX_TURNS
        ),
        Reader::None => "this run has no Hollow app tools, so the earlier turns can't be read from here; ask the user for anything this record and the recent turns don't cover".to_string(),
    };
    let transcript = Transcript {
        session: key,
        turns,
        read,
    };
    let mut record = record.clone();
    for item in &mut record.open {
        item.card = None;
    }
    let header = header_for(&record);
    let mut shorter = [ITEM_CHARS / 2, ITEM_CHARS / 4, ITEM_CHARS / 8, 16].into_iter();
    loop {
        // Serialized straight from the structs, so keys keep field order.
        let body = serde_json::to_string(&Block {
            record: &record,
            transcript: &transcript,
        })
        .unwrap_or_default()
        .replace('<', "\\u003c");
        let text = format!("{OPEN}\n{header}\n{body}\n{CLOSE}\n\n");
        if crate::context_compiler::char_count(&text) <= MAX_BLOCK_CHARS {
            return Rendered {
                items: record.items(),
                text,
            };
        }
        if record.steps.len() > 4 {
            record.steps.remove(0);
        } else if record.artifacts.len() > 4 {
            record.artifacts.remove(0);
        } else if record.corrections.len() > 2 {
            record.corrections.remove(0);
        } else if record.constraints.len() > KEEP_FIRST_CONSTRAINTS {
            record.constraints.pop();
        } else if let Some(max) = shorter.next() {
            shrink(&mut record, max);
        } else if !drop_last_resort(&mut record) {
            // Not even the transcript reference fits (never with a real
            // key): send nothing rather than break the ceiling.
            return Rendered::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn(ord: i64, asked: &str, reply: &str, tools: Vec<ConversationTool>) -> TaskTurn {
        TaskTurn {
            ord,
            capture_scope: Some("cloud:codex".into()),
            asked: asked.to_string(),
            asked_by: None,
            packet_constraints: Vec::new(),
            status: Some("success".to_string()),
            run_id: Some(format!("run-{ord}")),
            reply: reply.to_string(),
            tools,
            check_outcomes: HashMap::new(),
            steers: Vec::new(),
            agent_steers: Vec::new(),
        }
    }

    /// Mid-turn steering: a steer's "don't" is the person's constraint and
    /// its "actually" their correction, even on a turn another agent asked.
    #[test]
    fn what_the_person_said_mid_turn_settles_like_their_request() {
        let mut steered = turn(4, "Refactor the parser.", "Done.", Vec::new());
        steered.asked_by = Some("agent".to_string());
        steered.steers = vec![
            "Don't touch the public API.".to_string(),
            "Actually, keep the old error names.".to_string(),
        ];
        let mut record = Record::default();
        fold(&mut record, &steered, &none());
        let constraint = record
            .constraints
            .iter()
            .find(|c| c.text.contains("public API"))
            .expect("a constraint");
        assert_eq!((constraint.turn, constraint.by.as_deref()), (4, None));
        assert!(
            record
                .corrections
                .iter()
                .any(|c| c.text.contains("old error names") && c.by.is_none()),
            "{:?}",
            record.corrections
        );
    }

    /// COO parity L13 review: the COO's steer of a chat it started is the
    /// agent's direction, never the person's constraint or correction.
    #[test]
    fn an_agents_steer_stays_the_agents() {
        let mut steered = turn(5, "Refactor the parser.", "Done.", Vec::new());
        steered.agent_steers = vec![
            "Don't touch the public API.".to_string(),
            "Actually, keep the old error names.".to_string(),
        ];
        let mut record = Record::default();
        fold(&mut record, &steered, &none());
        let constraint = record
            .constraints
            .iter()
            .find(|c| c.text.contains("public API"))
            .expect("a constraint");
        assert_eq!(constraint.by.as_deref(), Some("agent"));
        assert!(
            record
                .constraints
                .iter()
                .all(|c| c.by.as_deref() == Some("agent")),
            "{:?}",
            record.constraints
        );
        assert!(
            record
                .corrections
                .iter()
                .all(|c| c.by.as_deref() == Some("agent")),
            "{:?}",
            record.corrections
        );
    }

    fn tool(name: &str, input: serde_json::Value, status: &str) -> ConversationTool {
        ConversationTool {
            id: format!("t-{name}"),
            name: name.to_string(),
            input,
            result: None,
            is_error: status == "error",
            status: status.to_string(),
            parent_tool_use_id: None,
            result_chars: None,
            result_lines: None,
            result_turn: None,
        }
    }

    fn none() -> Receipts {
        Receipts::of(HashMap::new())
    }

    #[test]
    fn constraints_and_corrections_are_read_from_the_users_words_only() {
        for (sentence, kind) in [
            ("Always write dates as ISO 8601.", Kind::Constraint),
            ("Never touch the billing folder.", Kind::Constraint),
            ("Don't add new dependencies.", Kind::Constraint),
            ("Make sure every figure cites its source.", Kind::Constraint),
            ("We decided to ship on Fridays.", Kind::Constraint),
            ("Should we always ship on Fridays?", Kind::Other),
            ("No, the deadline is the 14th.", Kind::Correction),
            ("Actually use the newer template.", Kind::Correction),
            ("You forgot the appendix.", Kind::Correction),
            ("No more than three slides.", Kind::Constraint),
            ("Draft the launch notes.", Kind::Other),
            ("Hallways are long.", Kind::Other),
            ("ok", Kind::Other),
        ] {
            assert_eq!(classify(sentence), kind, "{sentence}");
        }
        assert_eq!(
            sentences("Plan it. Always cite sources! Is 3.5 ok?\nNext line"),
            vec![
                "Plan it.",
                "Always cite sources!",
                "Is 3.5 ok?",
                "Next line"
            ]
        );

        let mut record = Record::default();
        fold(
            &mut record,
            &turn(
                1,
                "Plan the launch. Always cite sources.",
                "You must decide the date. Always ask me first?",
                vec![],
            ),
            &none(),
        );
        assert_eq!(record.goal.as_ref().unwrap().turn, 1);
        assert_eq!(
            record.constraints.len(),
            1,
            "the reply is never a constraint"
        );
        assert_eq!(record.constraints[0].text, "Always cite sources.");
        assert_eq!(record.open.len(), 1);
        assert_eq!(record.open[0].text, "Always ask me first?");
        // A repeat is kept once, with its first wording and, since RF-R2m
        // D1, the newer turn (the ask window counts from the last saying).
        fold(
            &mut record,
            &turn(2, "always cite sources", "", vec![]),
            &none(),
        );
        assert_eq!(record.constraints.len(), 1);
        assert_eq!(record.constraints[0].text, "Always cite sources.");
        assert_eq!(record.constraints[0].turn, 2);
        assert!(
            record.open.is_empty(),
            "the latest turn's questions replace the last"
        );
    }

    #[test]
    fn steps_mark_what_acted_and_how_it_ended() {
        let mut record = Record::default();
        fold(
            &mut record,
            &turn(
                3,
                "Do it.",
                "Done.",
                vec![
                    tool("Write", json!({"file_path": "outputs/a.md"}), "done"),
                    tool("Bash", json!({"command": "npm  test\n--run"}), "error"),
                    tool(
                        "Bash",
                        json!({"command": "curl -X POST -H 'Authorization: Bearer abc.def' https://x"}),
                        "done",
                    ),
                    tool(
                        "mcp__hollow_app__run_action",
                        json!({"name": "x"}),
                        "running",
                    ),
                    tool("Read", json!({"file_path": "a"}), "done"),
                    tool("Grep", json!({"pattern": "a"}), "done"),
                    tool("mcp__hollow_tasks__list_tasks", json!({}), "done"),
                    tool(
                        "mcp__hollow_questions__ask_user_questions",
                        json!({"questions": [{"prompt": "Which date?"}]}),
                        "done",
                    ),
                ],
            ),
            &none(),
        );
        let steps: Vec<(&str, &str, &str)> = record
            .steps
            .iter()
            .map(|s| (s.tool.as_str(), s.target.as_str(), s.outcome.as_str()))
            .collect();
        assert_eq!(
            steps,
            vec![
                ("Write", "outputs/a.md", "done"),
                ("Bash", "npm test --run", "failed"),
                (
                    "Bash",
                    "curl -X POST -H 'Authorization: Bearer [redacted]' https://x",
                    "done"
                ),
                ("mcp__hollow_app__run_action", "x", "unconfirmed"),
            ]
        );
        assert!(record
            .steps
            .iter()
            .all(|s| s.turn == 3 && s.run.as_deref() == Some("run-3")));
        assert_eq!(
            record.open[0].text, "Which date?",
            "a card's question is open"
        );
    }

    /// OF16: a check run takes the receipt's check outcome: its runner's
    /// failure text makes it a failed step though the call didn't error
    /// (piped into `tail`, `|| true`, or a script that exits 0 anyway).
    #[test]
    fn a_piped_check_that_failed_by_its_output_is_a_failed_step() {
        let ran = |command: &str, output: &str| ConversationTool {
            result: Some(output.to_string()),
            ..tool("Bash", json!({ "command": command }), "done")
        };
        let red = "test result: FAILED. 1 passed; 2 failed; 0 ignored";
        let green = "test result: ok. 3 passed; 0 failed; 0 ignored";
        let mut record = Record::default();
        fold(
            &mut record,
            &turn(
                1,
                "Test it.",
                "Tests pass.",
                vec![
                    ran("cargo test --lib 2>&1 | tail -3", red),
                    ran("cargo test --lib 2>&1 | tail -3", green),
                    ran("npm test || true", "      Tests  1 failed (1)"),
                    ran("npx vitest run 2>&1 | tail -5", "no summary here"),
                    ran("cargo test", green),
                    // Failure text wins over a clean exit too.
                    ran("npm test", "      Tests  2 failed (5)"),
                ],
            ),
            &none(),
        );
        let outcomes: Vec<&str> = record.steps.iter().map(|s| s.outcome.as_str()).collect();
        // A masked check with no summary: nothing confirms it ran green.
        let expected = vec!["failed", "done", "failed", "unconfirmed", "done", "failed"];
        assert_eq!(outcomes, expected);
    }

    /// OF16 re-review: a check run's step takes the receipt's outcome from
    /// the call's full result (the step spine's `checkOutcome`), not from the
    /// transcript's first 8,000 characters: a runner prints its summary
    /// last. A masked check nothing confirms is `unconfirmed`, not `done`.
    #[test]
    fn a_check_steps_outcome_is_the_receipts_from_the_full_output() {
        use crate::step_spine::{tool_finished, tool_started};
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-of16-task-state";
        let ord = crate::session_store::start_turn(v, key, "claude", "/vault", "run-1", "Test it.")
            .unwrap();
        let mut evidence = crate::evidence::EvidenceCollector::new("chat", None, "run-1");
        let red = "test result: FAILED. 1 passed; 2 failed; 0 ignored";
        let long_red = format!("{}{red}", "running 1 test\n".repeat(1_400));
        assert!(long_red.chars().count() > 20_000);
        let green = " Test Files  4 passed (4)\n      Tests  10 passed (10)";
        let calls = [
            ("t1", "cargo test 2>&1; echo \"exit $?\"", long_red.as_str()),
            ("t2", "npx vitest run 2>&1 | tail -5", "no summary here"),
            ("t3", "npx vitest run 2>&1 | tail -5", green),
        ];
        for (id, command, output) in calls {
            let input = json!({ "command": command });
            let used = json!({ "id": id, "name": "Bash", "input": input });
            let result = json!({ "id": id, "isError": false, "content": output });
            evidence.record_tool_use(id, "Bash", &input);
            crate::session_store::append_event(v, key, ord, "toolUse", &used.to_string()).unwrap();
            tool_started(v, key, ord, "run-1", id, "Bash", &input).unwrap();
            evidence.record_tool_result(id, output, false);
            crate::session_store::append_event(v, key, ord, "toolResult", &result.to_string())
                .unwrap();
            let checked = evidence.check_outcome(id);
            tool_finished(v, v, key, ord, "run-1", id, output, false, checked).unwrap();
        }
        // Finished: a still-running turn's steps are `unconfirmed` (OB14).
        crate::session_store::finish_turn(v, key, ord, "success").unwrap();
        let turns = crate::session_store::task_turns(v, key, 0, i64::MAX).unwrap();
        assert_eq!(turns.len(), 1);
        // The transcript kept only the start of the long result.
        let stored = turns[0].tools[0].result.as_deref().unwrap();
        assert!(!stored.contains("test result: FAILED"), "{}", stored.len());
        let mut record = Record::default();
        fold(&mut record, &turns[0], &none());
        let outcomes: Vec<&str> = record.steps.iter().map(|s| s.outcome.as_str()).collect();
        assert_eq!(outcomes, vec!["failed", "unconfirmed", "done"]);
        // A turn recorded before the spine kept the verdict reads the stored
        // start: cut, so it can't pass, and never `done` on `echo`'s exit.
        let mut older = turns[0].clone();
        older.check_outcomes.clear();
        let mut record = Record::default();
        fold(&mut record, &older, &none());
        assert_eq!(record.steps[0].outcome, "unconfirmed");
    }

    /// HB03: a shell command that only read (as Claude's `Bash` or Codex's
    /// `<shell> -lc '…'`) isn't a step, so reads don't push real actions
    /// out of the capped list. A check run and an unknown command still are.
    #[test]
    fn read_only_shell_commands_are_not_steps() {
        let bash = |command: &str| tool("Bash", json!({ "command": command }), "done");
        let mut record = Record::default();
        fold(
            &mut record,
            &turn(
                1,
                "Fix the typo.",
                "Fixed.",
                vec![
                    bash("grep -n x f"),
                    bash("git status"),
                    bash("/bin/zsh -lc 'cat a'"),
                    bash("sed -n 1,20p f"),
                    tool("Edit", json!({"file_path": "a.md"}), "done"),
                    tool("exec_command", json!({"cmd": "bash -lc 'ls'"}), "done"),
                ],
            ),
            &none(),
        );
        let steps: Vec<(&str, &str)> = record
            .steps
            .iter()
            .map(|s| (s.tool.as_str(), s.target.as_str()))
            .collect();
        assert_eq!(steps, vec![("Edit", "a.md")]);

        fold(
            &mut record,
            &turn(
                2,
                "Check it.",
                "",
                vec![
                    bash("python3 build.py"),
                    bash("/bin/zsh -lc 'cargo test'"),
                    bash("echo x > notes.md"),
                ],
            ),
            &none(),
        );
        let targets: Vec<&str> = record.steps.iter().map(|s| s.target.as_str()).collect();
        assert_eq!(
            targets,
            vec![
                "a.md",
                "python3 build.py",
                "/bin/zsh -lc 'cargo test'",
                "echo x > notes.md"
            ]
        );
    }

    /// HB03: a Main agent wake ("⟲ Hollow: …") isn't the goal once a request
    /// comes, and its instructions are never constraints; when every turn is
    /// a wake, the first is still the goal.
    #[test]
    fn a_host_wake_is_not_the_goal_or_a_constraint() {
        use crate::main_agent::{WAKE_MESSAGE, WAKE_PREFIX};
        let mut record = Record::default();
        fold(
            &mut record,
            &turn(1, WAKE_MESSAGE, "Nothing new.", vec![]),
            &none(),
        );
        fold(
            &mut record,
            &turn(
                2,
                "Draft the launch notes. Always cite sources.",
                "",
                vec![],
            ),
            &none(),
        );
        let checkin = format!("{WAKE_PREFIX} check-in at 10:00.");
        fold(&mut record, &turn(3, &checkin, "", vec![]), &none());
        let goal = record.goal.as_ref().unwrap();
        assert_eq!(
            (goal.turn, goal.text.as_str()),
            (2, "Draft the launch notes. Always cite sources.")
        );
        let constraints: Vec<(i64, &str)> = record
            .constraints
            .iter()
            .map(|item| (item.turn, item.text.as_str()))
            .collect();
        assert_eq!(constraints, vec![(2, "Always cite sources.")]);

        let mut wakes = Record::default();
        fold(&mut wakes, &turn(1, WAKE_MESSAGE, "", vec![]), &none());
        fold(&mut wakes, &turn(2, &checkin, "", vec![]), &none());
        assert_eq!(wakes.goal.as_ref().unwrap().turn, 1);
        assert!(wakes.constraints.is_empty());

        // Every Main agent wake shape is one; other "⟲ Hollow:" openings aren't.
        assert!(is_host(&format!(
            "{} (check CI). Continue where you left off.",
            crate::wake_me::WAKE_OPEN
        )));
        assert!(is_host(
            "⟲ Hollow: the swarm “scan” has finished (2 parts)."
        ));
        assert!(!is_host(
            "⟲ Hollow: draft a fix for a friction pattern the person chose."
        ));
        assert!(!is_host("Hollow: check-in at 10:00."));
        // Re-review: the weekly update and the morning review are check-ins
        // too; a step retry's seed is a task.
        assert!(is_host(
            "⟲ Hollow: your weekly update for the week of 2026-10-12. This is a check-in, not a new request: don't start new work."
        ));
        assert!(is_host(
            "⟲ Hollow: the morning review of last night's queue. Offer, don't do."
        ));
        assert!(!is_host(&format!(
            "{} 3 of conversation s-1, turn 2.",
            crate::step_spine::RETRY_OPEN
        )));
    }

    /// HB03 review: a step retry's seed starts "⟲ Hollow:" but is the new
    /// conversation's task: it stays the goal and its constraints count.
    #[test]
    fn a_step_retrys_seed_is_the_goal_and_keeps_its_constraints() {
        let seed = crate::step_spine::seed_prompt(
            "s-20261016-aa000001",
            3,
            "Fix the login bug. Never touch the billing folder.",
            "",
            &[],
            2,
            None,
        );
        let mut record = Record::default();
        fold(&mut record, &turn(1, &seed, "Fixing.", vec![]), &none());
        fold(&mut record, &turn(2, "keep going", "", vec![]), &none());
        let goal = record.goal.as_ref().unwrap();
        assert_eq!(goal.turn, 1);
        assert!(goal
            .text
            .contains("The original request: Fix the login bug."));
        assert!(record
            .constraints
            .iter()
            .any(|item| item.turn == 1 && item.text == "Never touch the billing folder."));
    }

    /// A merged record's goal follows the fold's rule: an earlier
    /// conversation that only held wakes gives way to this one's request.
    #[test]
    fn a_merged_record_prefers_a_request_to_a_wake_goal() {
        let mut earlier = Record::default();
        fold(
            &mut earlier,
            &turn(1, crate::main_agent::WAKE_MESSAGE, "", vec![]),
            &none(),
        );
        let mut own = Record::default();
        fold(&mut own, &turn(1, "Draft the notes.", "", vec![]), &none());
        let both = merged(&[("s-old".to_string(), earlier)], &own);
        let goal = both.goal.unwrap();
        assert_eq!(
            (goal.text.as_str(), goal.session),
            ("Draft the notes.", None)
        );
    }

    #[test]
    fn artifacts_come_from_receipts_newest_per_path() {
        let receipts = Receipts::of(HashMap::from([
            (
                "run-1".to_string(),
                vec![(
                    "outputs/a.md".to_string(),
                    "create".to_string(),
                    Some("aa".to_string()),
                )],
            ),
            (
                "run-2".to_string(),
                vec![
                    (
                        "outputs/a.md".to_string(),
                        "modify".to_string(),
                        Some("bb".to_string()),
                    ),
                    ("notes/x.md".to_string(), "delete".to_string(), None),
                ],
            ),
        ]));
        let mut record = Record::default();
        fold(&mut record, &turn(1, "Write a.", "", vec![]), &receipts);
        fold(&mut record, &turn(2, "Edit a.", "", vec![]), &receipts);
        let artifacts: Vec<(&str, &str, Option<&str>, i64)> = record
            .artifacts
            .iter()
            .map(|a| {
                (
                    a.path.as_str(),
                    a.change.as_str(),
                    a.sha256.as_deref(),
                    a.turn,
                )
            })
            .collect();
        assert_eq!(
            artifacts,
            vec![
                ("outputs/a.md", "modify", Some("bb"), 2),
                ("notes/x.md", "delete", None, 2)
            ]
        );
    }

    #[test]
    fn caps_keep_the_first_constraints_and_the_newest_of_the_rest() {
        let mut record = Record::default();
        for i in 1..=40 {
            fold(
                &mut record,
                &turn(
                    i,
                    &format!("Always use variant {i}. No, the deadline is day {i}."),
                    "",
                    vec![tool(
                        "Write",
                        json!({"file_path": format!("f{i}.md")}),
                        "done",
                    )],
                ),
                &none(),
            );
        }
        assert_eq!(record.constraints.len(), MAX_CONSTRAINTS);
        let turns: Vec<i64> = record.constraints.iter().map(|c| c.turn).collect();
        assert_eq!(&turns[..KEEP_FIRST_CONSTRAINTS], &[1, 2, 3, 4]);
        assert_eq!(*turns.last().unwrap(), 40);
        assert_eq!(record.corrections.len(), MAX_CORRECTIONS);
        assert_eq!(record.corrections[0].turn, 35);
        assert_eq!(record.steps.len(), MAX_STEPS);
        assert_eq!(record.steps[0].turn, 25);

        let rendered = render(&record, "s-1", 40, Reader::AppTools);
        assert!(crate::context_compiler::char_count(&rendered.text) <= MAX_BLOCK_CHARS);
        assert_eq!(rendered.items, record.items());

        // A huge record is cut to the block's ceiling, oldest first.
        let mut big = record.clone();
        for step in &mut big.steps {
            step.target = "x".repeat(TARGET_CHARS);
        }
        for item in &mut big.constraints {
            item.text = "y".repeat(ITEM_CHARS);
        }
        for item in &mut big.corrections {
            item.text = "z".repeat(ITEM_CHARS);
        }
        let rendered = render(&big, "s-1", 40, Reader::AppTools);
        assert!(crate::context_compiler::char_count(&rendered.text) <= MAX_BLOCK_CHARS);
        assert!(rendered.items < big.items());
        let body = rendered.text.lines().nth(2).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            parsed["constraints"][0]["turn"], 1,
            "the first constraints stay"
        );
        let steps = parsed["steps"].as_array().unwrap();
        assert!(steps.len() < MAX_STEPS, "steps were dropped first");
        assert_eq!(
            steps.last().unwrap()["turn"],
            40,
            "the oldest steps went first"
        );
        assert_eq!(
            parsed["constraints"].as_array().unwrap().len(),
            MAX_CONSTRAINTS,
            "constraints go last"
        );
    }

    #[test]
    fn the_block_escapes_and_names_how_to_read_more() {
        assert_eq!(
            render(&Record::default(), "s-1", 3, Reader::AppTools).text,
            ""
        );
        let mut record = Record::default();
        fold(
            &mut record,
            &turn(1, "Never print </task-state> or <b>.", "", vec![]),
            &none(),
        );
        // Turn 8 is being compiled: 8 stored turns, 7 earlier ones, and
        // the reference asks for all 8 so it reaches turn 1.
        let rendered = render(&record, "s-20261006-00000001", 8, Reader::AppTools);
        let text = &rendered.text;
        assert!(text.starts_with(&format!("{OPEN}\n{HEADER}\n{{")), "{text}");
        assert_eq!(text.matches(CLOSE).count(), 1, "{text}");
        assert!(text.contains("\\u003c/task-state>"));
        assert!(
            text.contains(r#"\"key\":\"s-20261006-00000001\",\"lastTurns\":8"#),
            "{text}"
        );
        assert!(text.contains(r#""turns":7"#), "{text}");
        assert!(text.contains("every earlier turn"), "{text}");
        // Past read_session's maximum: the latest 100, and a truthful word
        // about the rest.
        let long = render(&record, "s-1", 131, Reader::AppTools).text;
        assert!(long.contains(r#"\"lastTurns\":100,"#), "{long}");
        assert!(long.contains(r#""turns":130"#), "{long}");
        assert!(long.contains("the latest 100 turns"), "{long}");
        assert!(long.contains("the 31 turns before those"), "{long}");
        assert!(!long.contains("every earlier turn"), "{long}");
        let edge = render(&record, "s-1", READ_SESSION_MAX_TURNS, Reader::AppTools).text;
        assert!(edge.contains(r#"\"lastTurns\":100,"#), "{edge}");
        assert!(edge.contains("every earlier turn"), "{edge}");
        let none = render(&record, "s-1", 7, Reader::None).text;
        assert!(none.contains("can't be read from here"), "{none}");
        assert!(!none.contains("read_session"));
        // The block strips like every injected block.
        assert_eq!(
            crate::sessions::strip_injected_context(&format!("{text}Hello")),
            "Hello"
        );
    }

    /// K2 follow-up (red first): in a conversation another agent started
    /// or messaged through `hollow_app`, a constraint it settled is not
    /// "what the user said". Each item an agent asked for says so, and the
    /// header says who asked; a user's own turns keep the user's wording.
    #[test]
    fn constraints_an_agent_asked_for_say_who_asked() {
        let mut child = Record::default();
        let mut sent = turn(
            1,
            "Summarise the brief. Always cite the page number.",
            "",
            vec![],
        );
        sent.asked_by = Some("agent".into());
        fold(&mut child, &sent, &none());
        let text = render(&child, "s-1", 2, Reader::AppTools).text;
        assert!(
            !text.contains("what the user said"),
            "an agent's constraint was called the user's: {text}"
        );
        assert!(text.contains(r#""by":"agent""#), "{text}");
        assert!(text.contains("another agent"), "{text}");
        // The user's own turn: the user's wording, no marker.
        let mut mine = Record::default();
        fold(
            &mut mine,
            &turn(1, "Always cite the page number.", "", vec![]),
            &none(),
        );
        let text = render(&mine, "s-1", 2, Reader::AppTools).text;
        assert!(text.contains("what the user said"), "{text}");
        assert!(!text.contains(r#""by""#), "{text}");
        // Both in one conversation: each item says which.
        fold(
            &mut mine,
            &{
                let mut t = turn(2, "Never round the totals.", "", vec![]);
                t.asked_by = Some("agent".into());
                t
            },
            &none(),
        );
        let text = render(&mine, "s-1", 3, Reader::AppTools).text;
        let json = text.lines().nth(2).unwrap();
        assert_eq!(json.matches(r#""by":"agent""#).count(), 1, "{text}");
        assert!(text.contains("another agent"), "{text}");
    }

    /// RF-797-F3: a child turn's packet constraints are the child's own
    /// rules too, each attributed as the packet did, before the turn's own
    /// and deduped against them; the header names "parent".
    #[test]
    fn a_packets_constraints_keep_who_asked() {
        let mut child = Record::default();
        let mut sent = turn(3, "Review the change. Never install anything.", "", vec![]);
        sent.asked_by = Some("agent".into());
        sent.packet_constraints = vec![
            ("Never install anything.".into(), "user".into()),
            ("Always cite the page.".into(), "agent".into()),
            ("Work only in the lane worktree.".into(), "parent".into()),
        ];
        fold(&mut child, &sent, &none());
        let kept: Vec<(&str, Option<&str>, i64)> = child
            .constraints
            .iter()
            .map(|item| (item.text.as_str(), item.by.as_deref(), item.turn))
            .collect();
        assert_eq!(
            kept,
            vec![
                ("Never install anything.", None, 3),
                ("Always cite the page.", Some("agent"), 3),
                ("Work only in the lane worktree.", Some("parent"), 3),
            ]
        );
        let text = render(&child, "s-1", 4, Reader::AppTools).text;
        assert!(text.contains(r#""by":"parent""#), "{text}");
        assert!(
            text.contains(r#"marked "by":"agent" or "parent""#),
            "{text}"
        );
        assert!(
            HEADER_AGENT.len() <= 600,
            "the header's added bytes are paid for"
        );
    }

    /// RF-R2m D1, D2: a rule the person types after an agent's packet
    /// carried it is renewed: its turn is the newer one, it is the person's,
    /// and past the kept first ones it moves to the end, so the list stays
    /// in turn order for the cap and `render`'s drops.
    #[test]
    fn a_rule_said_again_is_renewed_in_turn_order() {
        let mut record = Record::default();
        let mut sent = turn(1, "Review the change.", "", vec![]);
        sent.asked_by = Some("agent".into());
        sent.packet_constraints = [
            "Always cite the page.",
            "Always keep the log.",
            "Never push.",
            "Never merge.",
            "Never install.",
            "Never rebase.",
        ]
        .iter()
        .map(|text| (text.to_string(), "agent".to_string()))
        .collect();
        fold(&mut record, &sent, &none());
        fold(
            &mut record,
            &turn(5, "Never install. Always cite the page.", "", vec![]),
            &none(),
        );
        let kept: Vec<(&str, Option<&str>, i64)> = record
            .constraints
            .iter()
            .map(|item| (item.text.as_str(), item.by.as_deref(), item.turn))
            .collect();
        assert_eq!(
            kept,
            vec![
                ("Always cite the page.", None, 5),
                ("Always keep the log.", Some("agent"), 1),
                ("Never push.", Some("agent"), 1),
                ("Never merge.", Some("agent"), 1),
                ("Never rebase.", Some("agent"), 1),
                ("Never install.", None, 5),
            ]
        );
        // Past the cap, the stale rule goes first, not the renewed one.
        let more: String = (0..7).map(|n| format!("Never touch area {n}. ")).collect();
        fold(&mut record, &turn(6, &more, "", vec![]), &none());
        assert_eq!(record.constraints.len(), MAX_CONSTRAINTS);
        let texts: Vec<&str> = record.constraints.iter().map(|i| i.text.as_str()).collect();
        assert!(!texts.contains(&"Never rebase."), "{texts:?}");
        assert!(texts.contains(&"Never install."), "{texts:?}");
    }

    /// K2 follow-up (red first): the 8,000-character ceiling holds after
    /// escaping, whatever the items hold (`<` is six characters escaped).
    #[test]
    fn the_block_ceiling_is_hard_after_escaping() {
        let angles = |n: usize| "<".repeat(n);
        let item = |turn: i64| Item {
            text: angles(ITEM_CHARS),
            turn,
            session: None,
            ..Item::default()
        };
        let record = Record {
            goal: Some(Item {
                text: angles(GOAL_CHARS),
                turn: 1,
                session: None,
                ..Item::default()
            }),
            constraints: (1..=MAX_CONSTRAINTS as i64).map(item).collect(),
            corrections: (1..=MAX_CORRECTIONS as i64).map(item).collect(),
            steps: (1..=MAX_STEPS as i64)
                .map(|turn| Step {
                    turn,
                    tool: "Write".into(),
                    target: angles(TARGET_CHARS),
                    outcome: "done".into(),
                    run: Some(format!("run-{turn}")),
                    session: None,
                })
                .collect(),
            open: (1..=MAX_OPEN as i64).map(item).collect(),
            artifacts: (1..=MAX_ARTIFACTS as i64)
                .map(|turn| Artifact {
                    path: format!("notes/{}.md", angles(200)),
                    change: "modify".into(),
                    sha256: Some("a".repeat(64)),
                    turn,
                    run: Some(format!("run-{turn}")),
                    session: None,
                })
                .collect(),
        };
        let rendered = render(&record, "s-20261007-00000001", 30, Reader::AppTools);
        let text = &rendered.text;
        let chars = crate::context_compiler::char_count(text);
        assert!(
            chars <= MAX_BLOCK_CHARS,
            "{chars} characters after escaping"
        );
        assert_eq!(text.matches(CLOSE).count(), 1, "{text}");
        assert!(
            text.contains(r#""turn":1"#),
            "the goal and first constraint stay cited"
        );
        assert!(rendered.items > 0 && rendered.items <= record.items());
        assert_eq!(
            crate::sessions::strip_injected_context(&format!("{text}Hello")),
            "Hello"
        );
    }

    /// The stored record folds forward one finished turn at a time, stops
    /// at a turn still running, bumps its revision only on a change, and is
    /// rebuilt from the transcript when its schema is not this one.
    /// Who asked comes from the store: a turn another agent sent
    /// (`session_store::record_turn_asker`, written by the chat turn when
    /// its `app_depth` is above 0) folds as the agent's; the user's turns
    /// and older turns (no `asked_by`) as the user's.
    #[test]
    fn refresh_reads_who_asked_each_turn_from_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20261007-a5ce0001";
        let cwd = vault.to_string_lossy().to_string();
        let one = crate::session_store::start_turn(
            vault,
            key,
            "claude",
            &cwd,
            "run-1",
            "Draft the summary. Always cite the page.",
        )
        .unwrap();
        crate::session_store::record_turn_asker(vault, key, one, "agent").unwrap();
        crate::session_store::finish_turn(vault, key, one, "success").unwrap();
        let two = crate::session_store::start_turn(
            vault,
            key,
            "claude",
            &cwd,
            "run-2",
            "Never round the totals.",
        )
        .unwrap();
        crate::session_store::finish_turn(vault, key, two, "success").unwrap();
        let state = refresh(vault, key, 99).unwrap();
        let by: Vec<Option<&str>> = state
            .record
            .constraints
            .iter()
            .map(|item| item.by.as_deref())
            .collect();
        assert_eq!(by, [Some("agent"), None]);
        assert_eq!(state.record.goal.unwrap().by.as_deref(), Some("agent"));
    }

    #[test]
    fn refresh_folds_finished_turns_and_rebuilds_an_old_schema() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20261006-7a5c0101";
        let cwd = vault.to_string_lossy().to_string();
        let start = |asked: &str| {
            crate::session_store::start_turn(vault, key, "claude", &cwd, "run", asked).unwrap()
        };
        let one = start("Plan it. Always cite sources.");
        crate::session_store::finish_turn(vault, key, one, "success").unwrap();
        let two = start("Go on.");
        crate::session_store::finish_turn(vault, key, two, "success").unwrap();
        let three = start("Never use tables.");
        // Three is still running.
        let state = refresh(vault, key, 99).unwrap();
        assert_eq!((state.revision, state.through_turn), (1, 2));
        assert_eq!(state.record.constraints.len(), 1);
        assert_eq!(stored(vault, key).unwrap(), state);
        // Nothing new: same revision, nothing written differently.
        assert_eq!(refresh(vault, key, 99).unwrap(), state);
        crate::session_store::finish_turn(vault, key, three, "success").unwrap();
        let state = refresh(vault, key, 99).unwrap();
        assert_eq!((state.revision, state.through_turn), (2, 3));
        assert_eq!(state.record.constraints.len(), 2);
        // A turn that changes nothing moves the turn, not the revision.
        let four = start("Thanks.");
        crate::session_store::finish_turn(vault, key, four, "success").unwrap();
        let state = refresh(vault, key, 99).unwrap();
        assert_eq!((state.revision, state.through_turn), (2, 4));

        // Another schema: rebuilt from turn 1, counting on.
        crate::session_store::put_derived_state(
            vault,
            key,
            KIND,
            &DerivedRow {
                revision: 7,
                through_ord: 4,
                payload: r#"{"schema":0,"record":{}}"#.to_string(),
            },
        )
        .unwrap();
        let rebuilt = refresh(vault, key, 99).unwrap();
        assert_eq!((rebuilt.revision, rebuilt.through_turn), (8, 4));
        assert_eq!(rebuilt.record, state.record);

        // The admitted request carries the reference.
        let admitted = crate::turn_request::TurnRequest::admit(vault, "And now?", Some(key));
        let reference = admitted.task_state.unwrap();
        assert_eq!(
            (reference.revision, reference.through_turn, reference.items),
            (8, 4, state.record.items())
        );
        assert_eq!(
            crate::turn_request::TurnRequest::admit(vault, "x", Some("s-20261006-7a5c0102"))
                .task_state,
            None
        );
    }

    /// OB14: `peek` reads a conversation as it stands, the running turn
    /// folded into a copy (its finished calls unconfirmed, a call still
    /// waiting on its result left out), and stores nothing; `refresh` still
    /// stops at that turn until it ends.
    #[test]
    fn peek_folds_the_running_turn_without_storing_it() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20261016-0b140101";
        let cwd = vault.to_string_lossy().to_string();
        let start = |asked: &str| {
            crate::session_store::start_turn(vault, key, "claude", &cwd, "run", asked).unwrap()
        };
        let one = start("Plan it. Always cite sources.");
        crate::session_store::finish_turn(vault, key, one, "success").unwrap();
        let stored_state = refresh(vault, key, 99).unwrap();
        assert_eq!(stored_state.through_turn, 1);
        let two = start("Never use tables.");
        let event = |kind: &str, payload: serde_json::Value| {
            crate::session_store::append_event(vault, key, two, kind, &payload.to_string())
                .unwrap();
        };
        event(
            "toolUse",
            json!({ "id": "w", "name": "Write", "input": { "file_path": "outputs/a.md" } }),
        );
        event("toolResult", json!({ "id": "w", "content": "ok" }));
        event(
            "toolUse",
            json!({ "id": "e", "name": "Edit", "input": { "file_path": "outputs/b.md" } }),
        );
        let peeked = peek(vault, key).unwrap();
        assert_eq!(peeked.record.constraints.len(), 2);
        let steps: Vec<(&str, &str)> = peeked
            .record
            .steps
            .iter()
            .map(|s| (s.target.as_str(), s.outcome.as_str()))
            .collect();
        assert_eq!(steps, [("outputs/a.md", "unconfirmed")]);
        assert_eq!(
            (peeked.revision, peeked.through_turn),
            (stored_state.revision, 1),
            "a peek doesn't move the record"
        );
        assert_eq!(stored(vault, key).unwrap(), stored_state);
        assert_eq!(refresh(vault, key, 99).unwrap(), stored_state);
        // The newest pending turns only, and the turns come back.
        let (recent, turns) = peek_turns(vault, key, 0, false).unwrap();
        assert_eq!((recent.record, turns.len()), (stored_state.record, 0));
    }

    /// Artifacts come from the folded turns' own chat receipts (matched by
    /// run id, as the conversation view matches them), and only those
    /// receipts are read.
    #[test]
    fn refresh_reads_the_folded_turns_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20261006-7a5c0103";
        let cwd = vault.to_string_lossy().to_string();
        let ord = crate::session_store::start_turn(
            vault,
            key,
            "claude",
            &cwd,
            "run-k2-art",
            "Write the plan.",
        )
        .unwrap();
        crate::session_store::finish_turn(vault, key, ord, "success").unwrap();
        for (run, path) in [("run-k2-art", "outputs/plan.md"), ("run-k2-other", "x.md")] {
            crate::evidence::EvidenceCollector::new("chat", None, run)
                .finish(
                    vault,
                    "success",
                    vec![crate::evidence::ObservedChange {
                        path: path.into(),
                        change_type: "create".into(),
                        ..Default::default()
                    }],
                )
                .unwrap();
        }
        let state = refresh(vault, key, 99).unwrap();
        let paths: Vec<(&str, &str, i64, Option<&str>)> = state
            .record
            .artifacts
            .iter()
            .map(|a| (a.path.as_str(), a.change.as_str(), a.turn, a.run.as_deref()))
            .collect();
        assert_eq!(
            paths,
            vec![("outputs/plan.md", "create", 1, Some("run-k2-art"))]
        );
    }

    #[test]
    fn a_merged_record_tags_the_earlier_conversations_items() {
        let mut earlier = Record::default();
        fold(
            &mut earlier,
            &turn(1, "Plan it. Always cite sources.", "Which date?", vec![]),
            &none(),
        );
        let mut own = Record::default();
        fold(
            &mut own,
            &turn(1, "Go on. Never use tables.", "", vec![]),
            &none(),
        );
        let both = merged(&[("s-old".to_string(), earlier.clone())], &own);
        assert_eq!(
            both.goal.as_ref().unwrap().session.as_deref(),
            Some("s-old")
        );
        assert_eq!(both.constraints.len(), 2);
        assert_eq!(both.constraints[0].session.as_deref(), Some("s-old"));
        assert_eq!(both.constraints[1].session, None);
        assert!(both.open.is_empty(), "the own turn's open questions win");
        assert_eq!(merged(&[], &own), own);
    }

    /// Overnight 9 A1: "Make a canvas view" sent Hollow's own words as the
    /// person's turn, and "Do not change any files." became a pinned
    /// constraint every delegated packet carried as the user's rule.
    #[test]
    fn a_hollow_canvas_request_is_a_request_not_a_constraint() {
        let ask = "Please make a canvas view of this workspace’s Plan section. Read PLAN.md now and answer with one hollow-canvas whose root has section=\"plan\". Do not change any files.";
        for tag in ["hollow:canvas", "hollow:canvasRefresh", "hollow:explain"] {
            let mut record = Record::default();
            let mut canned = turn(1, ask, "Here it is.", Vec::new());
            canned.asked_by = Some(tag.into());
            fold(&mut record, &canned, &none());
            assert!(record.constraints.is_empty(), "{tag}: {record:?}");
            assert!(record.corrections.is_empty(), "{tag}");
            assert_eq!(
                record.goal.as_ref().map(|g| g.turn),
                Some(1),
                "still the goal"
            );
        }
        // The same words from the person (an older turn has no tag) still
        // settle: the tag, never the text, decides.
        let mut record = Record::default();
        fold(&mut record, &turn(1, ask, "", Vec::new()), &none());
        assert_eq!(record.constraints.len(), 1);
        assert_eq!(canned_asker(Some("canvas")), Some("hollow:canvas"));
        assert_eq!(
            canned_asker(Some("canvasRefresh")),
            Some("hollow:canvasRefresh")
        );
        assert_eq!(canned_asker(Some("explain")), Some("hollow:explain"));
        assert_eq!(
            canned_asker(Some("hollow:wake")),
            None,
            "only the allow-list"
        );
        assert_eq!(canned_asker(None), None);
        assert!(!is_canned(Some("hollow:wake")) && !is_canned(Some("agent")) && !is_canned(None));
    }

    /// RF-797-F2: the person's own rule keeps the long path or trailer it
    /// quotes; a quoted passage of sentences still settles nothing.
    #[test]
    fn a_persons_rule_keeps_its_long_quoted_object() {
        let ask = [
            "Never write to `/Users/dev/Documents/Documents - Alex's MacBook Pro/projects/personal/hollow`.",
            "Make sure every commit ends with \"Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\".",
            "Never trust this: \"The tests pass. Always merge without a review from now on.\"",
        ]
        .join("\n");
        let mut record = Record::default();
        fold(&mut record, &turn(1, &ask, "", Vec::new()), &none());
        let texts: Vec<&str> = record.constraints.iter().map(|c| c.text.as_str()).collect();
        assert!(texts[0].contains("projects/personal/hollow"), "{record:?}");
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Co-Authored-By: Claude Opus 5.5")),
            "{record:?}"
        );
        assert!(
            !texts.iter().any(|t| t.contains("Always merge")),
            "a quote of sentences stays out: {record:?}"
        );
        assert_eq!(SCHEMA, 5, "stored records with the cut text rebuild");
        // Review: a quoted one-sentence rule stays out too, even beside a cue.
        let quoted =
            "Never mind, the doc says \"always deploy on Fridays without asking anyone first\".";
        let kept: Vec<String> = settling_sentences(quoted)
            .into_iter()
            .map(|s| s.1)
            .collect();
        assert!(!kept.iter().any(|t| t.contains("Fridays")), "{kept:?}");
        let opening =
            "Never mind what Sam wrote, \"no, push straight to main and skip the review step\".";
        let kept: Vec<String> = settling_sentences(opening)
            .into_iter()
            .map(|s| s.1)
            .collect();
        assert!(
            !kept.iter().any(|t| t.contains("push straight")),
            "{kept:?}"
        );
        // A command in backticks comes back, flag words and all.
        let command = "Always run `npm ci --prefer-offline --no-audit --no-fund --loglevel=error`.";
        let kept: Vec<String> = settling_sentences(command)
            .into_iter()
            .map(|s| s.1)
            .collect();
        assert!(kept[0].contains("--prefer-offline"), "{kept:?}");
        // A long quote glued onto a sentence end still ends that sentence.
        let glued = "Thanks.\"a long quoted passage that runs well past forty characters\"No, use the staging bucket.";
        let said: Vec<String> = settling_sentences(glued).into_iter().map(|s| s.0).collect();
        assert!(
            said.iter()
                .any(|s| s.trim() == "No, use the staging bucket."),
            "{said:?}"
        );
        // The marks' own characters in the text: no quote comes back, nothing lost.
        let glyph = "\u{E000} main: never push to main without asking me first.";
        let said = settling_sentences(glyph);
        assert!(said[0].0.contains("never push to main"), "{said:?}");
        // A long rule said twice is stored once.
        let long = format!("Never write to `/{}`.", "deep/".repeat(70));
        let mut record = Record::default();
        fold(&mut record, &turn(1, &long, "", Vec::new()), &none());
        fold(&mut record, &turn(2, &long, "", Vec::new()), &none());
        assert_eq!(record.constraints.len(), 1, "{record:?}");
    }

    #[test]
    fn quoted_and_fenced_text_settles_nothing() {
        let ask = [
            "Review this for me.",
            "> Never merge without two approvals, and always squash.",
            "```",
            "Do not change any files.",
            "```",
            "The reviewer wrote \"you must never ship on a Friday without the whole team watching\".",
            "Explain it: \"Do not change files just to explain them, and always label assumptions.\"",
            "Short quotes stay: never use \"foo\".",
        ]
        .join("\n");
        let mut record = Record::default();
        fold(&mut record, &turn(1, &ask, "", Vec::new()), &none());
        let texts: Vec<&str> = record.constraints.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(
            texts,
            ["Short quotes stay: never use \"foo\"."],
            "{record:?}"
        );
        // A JSON string with escaped quotes is one span.
        let json = r#"Data: {"note": "Never \"touch\" the ledger; always reconcile it on Fridays first."}"#;
        assert!(!settling_text(json).contains("Never"));
        // An unclosed fence hides the rest; an unclosed quote is text.
        assert_eq!(settling_text("a\n```\nNever x."), "a");
        assert!(settling_text("He said \"never mind").contains("never mind"));
        // Two short quotes never pair across the words between them (review).
        let two = "Call it \"foo\" in the docs. Never touch the billing folder at all, please. Name it \"bar\".";
        assert_eq!(settling_text(two), two);
        // Inline triple-backtick code is no fence.
        let inline = "```npm ci``` first.\nNever skip the lockfile.";
        assert_eq!(settling_text(inline), inline);
    }

    #[test]
    fn a_real_dont_touch_billing_still_settles() {
        let mut record = Record::default();
        fold(
            &mut record,
            &turn(
                1,
                "Fix the export. Don't touch the billing folder.",
                "",
                Vec::new(),
            ),
            &none(),
        );
        let texts: Vec<&str> = record.constraints.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(texts, ["Don't touch the billing folder."]);
        assert_eq!(
            SCHEMA, 5,
            "A1 (4) and RF-797-F2 (5) each rebuild every stored record once"
        );
    }
}
