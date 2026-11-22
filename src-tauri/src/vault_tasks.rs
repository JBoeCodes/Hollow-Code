//! Canonical Obsidian task storage. One file per task lives under
//! `vault/tasks/{slug}.md`. Optional `notion_id` fields record import
//! provenance and never change read or write authority.
//! YAML frontmatter holds the structured fields; the body is free-form
//! markdown description.
//!
//! New files are rendered from [`TaskFrontmatter`]; EXISTING files are only
//! ever line-patched ([`patch_task`] over [`crate::frontmatter::patch`]).
//! The frontmatter model is lossy (no list values, no unknown keys, no empty
//! values), so re-rendering an existing file would delete `previous_due`,
//! the legacy Notion import keys and anything the user added.
//!
//! Bodies are section-aware ([`parse_body`]): reserved `## Done when` /
//! `## Activity` sections and closed fenced blocks never become steps or
//! description, and step edits splice lines in place ([`splice_subtasks`]).
//! Creates that must not mint a `slug-2.md` twin go through
//! [`create_idempotent`] / [`create_from_input`] (fingerprint, then an
//! optional title-slug fallback, under one in-process `CREATE_LOCK`).
//!
//! `blocked_by:` (Paperclip PR 11b) is a comma list of `vault:tasks/…md`
//! URLs. [`list`] resolves it through one [`BlockerIndex`] (exact URL, then a
//! unique case-insensitive match) and fills the reverse `blocks` index,
//! `unresolved_blockers`, `ready` and the never-resolving `blocker_warnings`;
//! [`get`] and [`parse`] do not. Writes validate only newly added entries and
//! refuse a blocker loop ([`check_blocker_cycle`]).

use crate::frontmatter::KeyEdit;
use crate::types::{BlockerWarning, CreateTaskInput, SubTask, Task, UpdateTaskInput};
use crate::vault;
use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use walkdir::WalkDir;

const TASKS_DIR: &str = "tasks";

/// Error-message prefix for a stale `update_task` baseline. The frontend
/// matches it (Tauri rejects with the plain string).
pub const TASK_CONFLICT_ERROR: &str = "TASK_CONFLICT";
/// Error-message prefix for an edit refused because a task's tracker issue
/// holds the field.
pub const TASK_LINKED_ERROR: &str = "TASK_LINKED";
/// Error-message prefix for a `blocked_by` edit that would close a loop
/// (`TASK_CYCLE: A → B → A`, by title) or name the task itself.
pub const TASK_CYCLE_ERROR: &str = "TASK_CYCLE";
/// Error-message prefix for a newly added `blocked_by` entry that names no
/// task or can't be stored in the comma list.
pub const TASK_BLOCKER_ERROR: &str = "TASK_BLOCKER";

/// sha256 hex of a task file's exact text: the compare-and-swap baseline
/// carried as `Task.contentHash` and checked against `UpdateTaskInput.expected`.
pub fn content_hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

/// Normalize a priority value to the canonical `P1`..`P4` form. Tolerant of
/// the legacy `High`/`Medium`/`Low` labels (High→P1, Medium→P2, Low→P3) and
/// of any casing. Returns `None` for empty/unrecognized input.
pub fn normalize_priority(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    match s.to_ascii_lowercase().as_str() {
        "p1" | "high" | "urgent" => Some("P1".into()),
        "p2" | "medium" | "med" => Some("P2".into()),
        "p3" | "low" => Some("P3".into()),
        "p4" | "none" => Some("P4".into()),
        _ => None,
    }
}

/// True when a task's status means the work is finished. Case-insensitive and
/// tolerant of vocabulary, so a task marked `Delivered` through the app and one
/// the embedded chat wrote as `Done` (or `Complete`) are both recognized as
/// complete. This is the single source of truth for "is this task done" —
/// `enrich_tasks` uses it to bucket completed tasks out of the active lists, and
/// `set_task_status` uses it for the recurrence roll-forward.
pub fn is_terminal_status(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "delivered" | "done" | "complete" | "completed"
    )
}

pub(crate) fn task_is_done(task: &Task) -> bool {
    task.tracker
        .as_ref()
        .and_then(|m| m.status_type)
        .map_or_else(|| is_terminal_status(&task.status), |kind| kind.is_done())
}

pub(crate) fn task_is_canceled(task: &Task) -> bool {
    task.tracker
        .as_ref()
        .and_then(|m| m.status_type)
        .map_or_else(
            || is_cancelled_status(&task.status),
            |kind| kind == crate::tracker::StateKind::Canceled,
        )
}

/// True when a status means the work was dropped: `cancelled`, `canceled`,
/// `won't do` or `wont do` (case-insensitive, straight or curly apostrophe;
/// covers Notion mirrors). Hollow has no canonical cancelled status, so a
/// blocker with one of these never resolves (a `cancelled` warning).
pub fn is_cancelled_status(status: &str) -> bool {
    let folded: String = status
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if matches!(c, '\u{2019}' | '\u{2018}' | '`') {
                '\''
            } else {
                c
            }
        })
        .collect();
    let words = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    matches!(
        words.as_str(),
        "cancelled" | "canceled" | "won't do" | "wont do"
    )
}

/// True when a status says the task is deliberately parked: any word of it is
/// `blocked`, `waiting`, `hold` or `deferred` (`On hold`, `Waiting on Sam`),
/// the status words `project_next_move` treats as blocked. Such a task is not
/// `ready`, but it can still enter the task-unblocked trigger set.
pub fn is_hold_status(status: &str) -> bool {
    status
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|w| {
            matches!(
                w.to_lowercase().as_str(),
                "blocked" | "waiting" | "hold" | "deferred"
            )
        })
}

/// Split a comma-separated `labels:` scalar into trimmed, de-duplicated,
/// non-empty labels (case-insensitive de-dup, first spelling wins).
pub fn parse_labels(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in raw.split(',') {
        let t = part.trim();
        if !t.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            out.push(t.to_string());
        }
    }
    out
}

// ── Field validators (shared by meeting follow-through and PR 12's agent
// writes). The UI create/update commands keep their own lenient
// normalization; these are for model-produced values.

/// A `YYYY-MM-DD` date, trimmed, or `None`.
pub(crate) fn valid_due(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()
            .map(|_| value.to_string())
    })
}

/// A canonical `P1`..`P4` priority (any case), or `None`. Legacy spellings
/// (`High`, `Medium`, …) are not accepted here; see [`normalize_priority`].
pub(crate) fn valid_priority(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| matches!(value.as_str(), "P1" | "P2" | "P3" | "P4"))
}

/// `value` on one line (whitespace runs collapsed to one space), at most
/// `max` characters.
pub(crate) fn one_line(value: &str, max: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// A 24-hour `HH:MM` clock time (00-23, 00-59), trimmed, or `None`.
pub(crate) fn valid_clock(value: &str) -> Option<String> {
    let value = value.trim();
    let bytes = value.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    let digits = |a: usize, b: usize| {
        bytes[a..b]
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| value[a..b].parse::<u32>().ok())
            .flatten()
    };
    match (digits(0, 2), digits(3, 5)) {
        (Some(h), Some(m)) if h <= 23 && m <= 59 => Some(value.to_string()),
        _ => None,
    }
}

/// Parse a single markdown checklist line (`- [ ]` / `- [x]`, `*` bullets and
/// leading indentation accepted) into a [`SubTask`]. Returns `None` for
/// non-checklist lines or empty item text.
fn parse_checklist_line(line: &str) -> Option<SubTask> {
    let t = line.trim_start();
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("* "))?;
    let rest = rest.trim_start();
    let (done, text) = if let Some(r) = rest.strip_prefix("[ ]") {
        (false, r)
    } else if let Some(r) = rest
        .strip_prefix("[x]")
        .or_else(|| rest.strip_prefix("[X]"))
    {
        (true, r)
    } else {
        return None;
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(SubTask { text, done })
}

/// The task's sub-tasks: the checklist lines of the normal region (outside
/// closed fences and reserved sections). See [`parse_body`].
#[cfg(test)]
pub fn parse_subtasks(body: &str) -> Vec<SubTask> {
    parse_body(body).subtasks
}

/// The prose description: the normal region minus its step lines, with
/// reserved sections left out. See [`parse_body`].
#[cfg(test)]
pub fn strip_subtasks(body: &str) -> String {
    parse_body(body).prose
}

// ── Section-aware task bodies (Paperclip PR 11) ──────────────────────────
//
// A task body has three kinds of region:
// - fenced code blocks: an opening ``` or ~~~ (3+ characters, up to 3
//   leading spaces) CLOSED by the same character repeated at least as many
//   times. An opener with no matching fence line after it is ordinary text,
//   deliberately unlike CommonMark, so a stray ``` at the end of a note
//   doesn't swallow the steps and sections after it. A stray opener still
//   pairs with the next matching fence line, even one that opens a later
//   block, and hides everything in between (as it does in Obsidian). Fenced
//   lines are never headings or steps.
// - reserved sections: an H2 outside a fence whose text (trimmed, trailing
//   `#`s and one trailing `:` removed, case-insensitive) is `Done when` or
//   `Activity`, running to the next H1 or H2 outside a fence. They are never
//   steps or description, and every rewrite keeps them verbatim. Repeated
//   headings are not merged.
// - the normal region: everything else. Its checklist lines (outside fences)
//   are the steps; its other lines are the description prose.
//
// Step-only edits splice the existing step lines in place
// ([`splice_subtasks`]); only a replacement of the prose itself falls back to
// the canonical layout ([`compose_task_body`]), because prose interleaved with
// steps cannot be mapped back.

/// The kind of a reserved H2 section in a task body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservedKind {
    DoneWhen,
    Activity,
}

impl ReservedKind {
    /// The heading line Hollow writes when it creates the section.
    pub fn heading(self) -> &'static str {
        match self {
            ReservedKind::DoneWhen => "## Done when",
            ReservedKind::Activity => "## Activity",
        }
    }
}

/// One reserved section, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservedSection {
    pub kind: ReservedKind,
    /// The heading line as written (no line terminator).
    pub heading: String,
    /// The lines after the heading, leading and trailing blank lines dropped.
    pub content: String,
    /// Heading plus content, trailing blank lines dropped: what a canonical
    /// compose re-emits.
    pub verbatim: String,
    /// Line index of the heading in the parsed body.
    pub start: usize,
    /// Exclusive end line index.
    pub end: usize,
}

/// A parsed task body. `lines` keep their terminators, so joining them
/// reproduces the body byte for byte.
#[derive(Debug, Clone)]
pub struct TaskBody {
    lines: Vec<String>,
    /// (line index, parsed step) for every step line, in order.
    step_slots: Vec<(usize, SubTask)>,
    /// The normal region's non-step lines (fenced blocks included), trimmed.
    pub prose: String,
    pub subtasks: Vec<SubTask>,
    pub sections: Vec<ReservedSection>,
    /// The last non-blank line before the first reserved heading.
    lead_end: Option<usize>,
}

impl TaskBody {
    /// Every `## Done when` section's content, joined by a blank line.
    pub fn done_when(&self) -> Option<String> {
        self.joined(ReservedKind::DoneWhen)
    }

    /// Every `## Activity` section's content, joined by a blank line.
    pub fn activity(&self) -> Option<String> {
        self.joined(ReservedKind::Activity)
    }

    fn joined(&self, kind: ReservedKind) -> Option<String> {
        let parts: Vec<&str> = self
            .sections
            .iter()
            .filter(|s| s.kind == kind && !s.content.is_empty())
            .map(|s| s.content.as_str())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }

    /// True when the body has no non-whitespace text at all.
    fn is_blank(&self) -> bool {
        self.lines.iter().all(|l| l.trim().is_empty())
    }

    /// The file's line terminator: CRLF when any line uses it.
    fn newline(&self) -> &'static str {
        if self.lines.iter().any(|l| l.ends_with("\r\n")) {
            "\r\n"
        } else {
            "\n"
        }
    }
}

/// A raw line without its `\n` / `\r\n` terminator.
fn line_text(raw: &str) -> &str {
    let t = raw.strip_suffix('\n').unwrap_or(raw);
    t.strip_suffix('\r').unwrap_or(t)
}

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// `(fence char, run length)` when `line` can open a fence.
fn fence_open(line: &str) -> Option<(char, usize)> {
    let indent = leading_spaces(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let c = rest.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let n = rest.chars().take_while(|&x| x == c).count();
    if n < 3 {
        return None;
    }
    // A backtick fence's info string may not contain a backtick.
    if c == '`' && rest[n..].contains('`') {
        return None;
    }
    Some((c, n))
}

fn fence_closes(line: &str, c: char, n: usize) -> bool {
    let indent = leading_spaces(line);
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let m = rest.chars().take_while(|&x| x == c).count();
    m >= n && rest[m..].trim().is_empty()
}

/// Inclusive `(open, close)` line spans of every CLOSED fence. An opener
/// with no matching close is ordinary text.
fn fence_spans(lines: &[&str]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((c, n)) = fence_open(lines[i]) {
            if let Some(j) = (i + 1..lines.len()).find(|&j| fence_closes(lines[j], c, n)) {
                spans.push((i, j));
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    spans
}

/// The level of an ATX heading line (`#` .. `######`), if it is one.
fn atx_level(line: &str) -> Option<usize> {
    let indent = leading_spaces(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let n = rest.bytes().take_while(|&b| b == b'#').count();
    if n == 0 || n > 6 {
        return None;
    }
    match rest[n..].chars().next() {
        None | Some(' ') | Some('\t') => Some(n),
        _ => None,
    }
}

/// The reserved kind of an H2 line, if it names one.
fn reserved_kind(line: &str) -> Option<ReservedKind> {
    if atx_level(line) != Some(2) {
        return None;
    }
    let rest = line.trim_start_matches(' ');
    let text = rest[2..].trim();
    let text = text.trim_end_matches('#').trim_end();
    let text = text.strip_suffix(':').unwrap_or(text);
    let normalized = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    match normalized.as_str() {
        "done when" => Some(ReservedKind::DoneWhen),
        "activity" => Some(ReservedKind::Activity),
        _ => None,
    }
}

/// `lines` joined with `\n`, leading and trailing whitespace-only lines dropped.
fn trim_blank_lines(lines: &[&str]) -> String {
    let start = lines.iter().position(|l| !l.trim().is_empty());
    let end = lines.iter().rposition(|l| !l.trim().is_empty());
    match (start, end) {
        (Some(s), Some(e)) => lines[s..=e].join("\n"),
        _ => String::new(),
    }
}

/// Parse a task body into its fenced blocks, reserved sections and normal
/// region (see the grammar above).
pub fn parse_body(body: &str) -> TaskBody {
    let lines: Vec<String> = body.split_inclusive('\n').map(str::to_string).collect();
    let texts: Vec<&str> = lines.iter().map(|l| line_text(l)).collect();
    let n = texts.len();
    let mut in_fence = vec![false; n];
    for (a, b) in fence_spans(&texts) {
        for flag in &mut in_fence[a..=b] {
            *flag = true;
        }
    }

    let mut sections: Vec<ReservedSection> = Vec::new();
    let mut in_section = vec![false; n];
    let mut i = 0;
    while i < n {
        let kind = (!in_fence[i]).then(|| reserved_kind(texts[i])).flatten();
        let Some(kind) = kind else {
            i += 1;
            continue;
        };
        let mut end = i + 1;
        while end < n && (in_fence[end] || !matches!(atx_level(texts[end]), Some(1 | 2))) {
            end += 1;
        }
        for flag in &mut in_section[i..end] {
            *flag = true;
        }
        let body_lines = &texts[i + 1..end];
        let content = trim_blank_lines(body_lines);
        let last = texts[i..end]
            .iter()
            .rposition(|l| !l.trim().is_empty())
            .map_or(i, |k| i + k);
        sections.push(ReservedSection {
            kind,
            heading: texts[i].to_string(),
            content,
            verbatim: texts[i..=last].join("\n"),
            start: i,
            end,
        });
        i = end;
    }

    let mut step_slots: Vec<(usize, SubTask)> = Vec::new();
    let mut prose_lines: Vec<&str> = Vec::new();
    for k in 0..n {
        if in_section[k] {
            continue;
        }
        match (!in_fence[k])
            .then(|| parse_checklist_line(texts[k]))
            .flatten()
        {
            Some(step) => step_slots.push((k, step)),
            None => prose_lines.push(texts[k]),
        }
    }
    let first_section = sections.first().map_or(n, |s| s.start);
    let lead_end = (0..first_section)
        .rev()
        .find(|&k| !texts[k].trim().is_empty());
    TaskBody {
        subtasks: step_slots.iter().map(|(_, s)| s.clone()).collect(),
        prose: prose_lines.join("\n").trim().to_string(),
        step_slots,
        sections,
        lead_end,
        lines,
    }
}

/// Steps as a writer should see them: text trimmed and flattened onto one
/// line, empty items dropped.
fn normalize_steps(steps: &[SubTask]) -> Vec<SubTask> {
    steps
        .iter()
        .map(|s| SubTask {
            text: s
                .text
                .split(['\n', '\r'])
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            done: s.done,
        })
        .filter(|s| !s.text.is_empty())
        .collect()
}

fn step_line(step: &SubTask) -> String {
    let mark = if step.done { "x" } else { " " };
    format!("- [{mark}] {}", step.text)
}

/// Rewrite one existing step line to `step`, keeping its indentation and
/// marker (everything before the checkbox) and its terminator.
fn rewrite_step_line(raw: &str, step: &SubTask) -> String {
    let text = line_text(raw);
    let terminator = &raw[text.len()..];
    let prefix = text.find('[').map_or("- ", |at| &text[..at]);
    let mark = if step.done { "x" } else { " " };
    format!("{prefix}[{mark}] {}{terminator}", step.text)
}

/// Insert `block` (lines without terminators) before line `at`. With
/// `pad_before` a blank line precedes it; with `pad_after` a blank line
/// follows it when the next line is not blank.
fn insert_block(
    lines: &mut Vec<String>,
    at: usize,
    block: &[String],
    nl: &str,
    pad_before: bool,
    pad_after: bool,
) {
    let mut new: Vec<String> = Vec::new();
    if pad_before {
        new.push(String::new());
    }
    new.extend(block.iter().cloned());
    if pad_after && lines.get(at).is_some_and(|l| !l.trim().is_empty()) {
        new.push(String::new());
    }
    if at > 0 && !lines[at - 1].ends_with('\n') {
        lines[at - 1].push_str(nl);
    }
    for (k, line) in new.into_iter().enumerate() {
        lines.insert(at + k, format!("{line}{nl}"));
    }
}

/// Pairs `(old index, new index)` of a longest common subsequence of the two
/// step lists, compared by text only (so a toggled step still pairs).
fn match_steps_by_text(old: &[&str], new: &[&str]) -> Vec<(usize, usize)> {
    let (n, m) = (old.len(), new.len());
    // lcs[i][j] = LCS length of old[i..] and new[j..].
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// The body with its steps replaced by `new`, keeping every other line byte
/// for byte. Old step lines are matched to new steps by text first (a
/// longest common subsequence), so deleting or inserting a step in the
/// middle touches only that step's line:
/// - a matched step whose done flag is unchanged keeps its exact bytes;
/// - a matched step that was toggled, or an unmatched old step paired by
///   position with an unmatched new step between the same matched
///   neighbours (a text edit), keeps its indentation and marker (`- ` / `* `);
/// - an old step left over in such a gap is removed;
/// - a new step left over is inserted as `- [ ] text` after the gap's last
///   old step line, else after the previous matched step, else before the
///   next matched step; with no steps at all, after the last non-blank line
///   before the first reserved heading (separated by a blank line), else just
///   before that heading, else at the end of the body. Nothing is ever
///   inserted ahead of the body's first line, which in a CRLF file (or one
///   whose closing `---` has trailing spaces) is the rest of the frontmatter
///   fence line.
pub fn splice_subtasks(parsed: &TaskBody, new: &[SubTask]) -> String {
    enum Op {
        Replace(String),
        Delete,
        Insert(Vec<String>),
    }
    let nl = parsed.newline();
    let new = normalize_steps(new);
    let slots = &parsed.step_slots;
    let mut lines = parsed.lines.clone();

    if slots.is_empty() {
        if new.is_empty() {
            return lines.concat();
        }
        let extra: Vec<String> = new.iter().map(step_line).collect();
        let first_section = parsed.sections.first().map_or(lines.len(), |s| s.start);
        let at = parsed.lead_end.map_or(first_section, |e| e + 1);
        insert_block(&mut lines, at, &extra, nl, parsed.lead_end.is_some(), true);
        return lines.concat();
    }

    let old_texts: Vec<&str> = slots.iter().map(|(_, s)| s.text.as_str()).collect();
    let new_texts: Vec<&str> = new.iter().map(|s| s.text.as_str()).collect();
    let anchors = match_steps_by_text(&old_texts, &new_texts);

    // (line index, op); an insert goes before its index.
    let mut ops: Vec<(usize, Op)> = Vec::new();
    let mut prev: Option<(usize, usize)> = None;
    for bound in anchors
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None))
    {
        let old_from = prev.map_or(0, |(o, _)| o + 1);
        let new_from = prev.map_or(0, |(_, w)| w + 1);
        let (old_to, new_to) = bound.unwrap_or((slots.len(), new.len()));
        let gap_old = &slots[old_from..old_to];
        let gap_new = &new[new_from..new_to];
        for ((index, _), want) in gap_old.iter().zip(gap_new.iter()) {
            ops.push((*index, Op::Replace(rewrite_step_line(&lines[*index], want))));
        }
        for (index, _) in gap_old.iter().skip(gap_new.len()) {
            ops.push((*index, Op::Delete));
        }
        if gap_new.len() > gap_old.len() {
            let extra: Vec<String> = gap_new[gap_old.len()..].iter().map(step_line).collect();
            let at = match (gap_old.last(), prev, bound) {
                (Some((index, _)), _, _) => index + 1,
                (None, Some((o, _)), _) => slots[o].0 + 1,
                (None, None, Some((o, _))) => slots[o].0,
                // Unreachable: no anchors and no old steps means `slots` is empty.
                (None, None, None) => slots[slots.len() - 1].0 + 1,
            };
            ops.push((at, Op::Insert(extra)));
        }
        if let Some((o, w)) = bound {
            if slots[o].1.done != new[w].done {
                let index = slots[o].0;
                ops.push((
                    index,
                    Op::Replace(rewrite_step_line(&lines[index], &new[w])),
                ));
            }
        }
        prev = bound;
    }

    // Highest index first; at one index, the line's own edit before an
    // insert ahead of it.
    ops.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| matches!(a.1, Op::Insert(_)).cmp(&matches!(b.1, Op::Insert(_))))
    });
    for (index, op) in ops {
        match op {
            Op::Replace(line) => lines[index] = line,
            Op::Delete => {
                lines.remove(index);
            }
            Op::Insert(block) => insert_block(&mut lines, index, &block, nl, false, false),
        }
    }
    lines.concat()
}

/// `text` minus any reserved sections typed into it, plus those sections.
/// A description replacement that contains `## Activity` must not become a
/// section that swallows the steps composed after it.
fn split_reserved(text: &str) -> (String, Vec<ReservedSection>) {
    if !text.contains('#') {
        return (text.to_string(), Vec::new());
    }
    let parsed = parse_body(text);
    if parsed.sections.is_empty() {
        return (text.to_string(), Vec::new());
    }
    let kept: Vec<&str> = parsed
        .lines
        .iter()
        .enumerate()
        .filter(|(k, _)| !parsed.sections.iter().any(|s| (s.start..s.end).contains(k)))
        .map(|(_, l)| line_text(l))
        .collect();
    (kept.join("\n"), parsed.sections)
}

/// The canonical body: the prose, then the steps, then every `## Done when`
/// section verbatim in original order, then every `## Activity` section,
/// joined by blank lines. With no sections this is exactly [`compose_body`].
/// Used only when the prose itself is replaced; step edits splice in place.
pub fn compose_task_body(
    prose: &str,
    subtasks: &[SubTask],
    sections: &[ReservedSection],
) -> String {
    let (prose, typed) = split_reserved(prose);
    let mut out = compose_body(&prose, subtasks);
    for kind in [ReservedKind::DoneWhen, ReservedKind::Activity] {
        for section in sections
            .iter()
            .chain(typed.iter())
            .filter(|s| s.kind == kind)
        {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&section.verbatim);
        }
    }
    out
}

/// Append `line` to the last section of `kind` (after its last non-blank
/// line), or, when there is none, create one (`ReservedKind::heading`, a
/// blank line, then `line`) after the last reserved section, or at the end
/// of the body. Every other byte is kept, and nothing is inserted ahead of
/// the body's first line (see [`splice_subtasks`]). A line of `line` that
/// would read as an H1 or H2 heading (and so end the section) or as a fence
/// line (and so open or close a fenced block that hides later steps and
/// sections) is escaped with `\`. PR 12's `add_comment` is the first caller.
pub fn append_to_section(body: &str, kind: ReservedKind, line: &str) -> String {
    let parsed = parse_body(body);
    let nl = parsed.newline();
    let entry: Vec<String> = line
        .lines()
        .map(|l| {
            let l = l.strip_suffix('\r').unwrap_or(l);
            if matches!(atx_level(l), Some(1 | 2)) || fence_open(l).is_some() {
                format!("\\{}", l.trim_start())
            } else {
                l.to_string()
            }
        })
        .collect();
    if entry.is_empty() {
        return body.to_string();
    }
    let mut lines = parsed.lines.clone();
    let texts: Vec<String> = lines.iter().map(|l| line_text(l).to_string()).collect();
    let last_non_blank =
        |from: usize, to: usize| (from..to).rev().find(|&k| !texts[k].trim().is_empty());
    match parsed.sections.iter().rev().find(|s| s.kind == kind) {
        Some(section) => {
            let last = last_non_blank(section.start, section.end).unwrap_or(section.start);
            let pad_before = last == section.start;
            insert_block(&mut lines, last + 1, &entry, nl, pad_before, true);
        }
        None => {
            let mut block = vec![kind.heading().to_string(), String::new()];
            block.extend(entry);
            let (at, pad_before) = match parsed.sections.last() {
                Some(section) => (
                    last_non_blank(section.start, section.end).unwrap_or(section.start) + 1,
                    true,
                ),
                None => match last_non_blank(0, texts.len()) {
                    Some(k) => (k + 1, true),
                    // A blank body: after its (blank) lines, never before them.
                    None => (texts.len(), false),
                },
            };
            insert_block(&mut lines, at, &block, nl, pad_before, true);
        }
    }
    lines.concat()
}

/// Unchecked checklist items outside closed fences, in order (reserved
/// sections are not special here). Used for `PLAN.md` task candidates.
pub fn open_checklist_items(text: &str) -> Vec<String> {
    let texts: Vec<&str> = text.split_inclusive('\n').map(line_text).collect();
    let mut in_fence = vec![false; texts.len()];
    for (a, b) in fence_spans(&texts) {
        for flag in &mut in_fence[a..=b] {
            *flag = true;
        }
    }
    texts
        .iter()
        .enumerate()
        .filter(|(k, _)| !in_fence[*k])
        .filter_map(|(_, l)| parse_checklist_line(l))
        .filter(|s| !s.done)
        .map(|s| s.text)
        .collect()
}

/// Compose a task body from prose + sub-tasks: prose first, then a markdown
/// checklist (one `- [ ]`/`- [x]` line per sub-task). Either part may be empty.
pub fn compose_body(prose: &str, subtasks: &[SubTask]) -> String {
    let mut out = prose.trim().to_string();
    if !subtasks.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        for st in subtasks {
            let text = st.text.trim();
            if text.is_empty() {
                continue;
            }
            let mark = if st.done { "x" } else { " " };
            out.push_str(&format!("- [{mark}] {text}\n"));
        }
    }
    out.trim_end().to_string()
}

/// Parsed frontmatter for a task file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskFrontmatter {
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub due_time: Option<String>,
    #[serde(default)]
    pub duration_min: Option<u32>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub recurrence: Option<String>,
    /// `vault:tasks/…md` URLs this task waits on (Paperclip PR 11b), stored
    /// as a comma scalar; a YAML block list is read too and rewritten as the
    /// scalar only when the list changes. Hollow-only, never mapped to Notion.
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub last_synced_at: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    /// Where an idempotently created task came from (`heartbeat`,
    /// `meeting:<path>`, `project:<rel>`, `plan:workspaces/<id>/PLAN.md`).
    /// Written once at create; one line, at most 200 characters.
    #[serde(default)]
    pub origin: Option<String>,
    /// sha256 hex of the caller's idempotency key ([`fingerprint_for_key`]).
    /// Written once at create and never by an edit, so a title change does
    /// not break deduplication.
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub notion_id: Option<String>,
    #[serde(default)]
    pub notion_url: Option<String>,
}

/// Stable identifier for a task — a `vault:` URI built from the
/// vault-relative path. The frontend keys drag-drop and selection by
/// this field, so it has to be unique per file.
fn stable_url(_fm: &TaskFrontmatter, rel: &str) -> String {
    format!("vault:{}", rel)
}

fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => c,
        })
        .collect();
    cleaned.chars().take(180).collect()
}

/// Lowercase + hyphenated slug for filename use. Falls back to "task"
/// when the title is empty (shouldn't happen, but defensive).
pub fn slugify(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    let joined: String = cleaned.split_whitespace().collect::<Vec<_>>().join("-");
    if joined.is_empty() {
        "task".into()
    } else {
        joined.chars().take(120).collect()
    }
}

/// Walk `vault/tasks/*.md` and return parsed task entries with their blocker
/// structure resolved ([`link_blockers`]): `blocks`, `unresolved_blockers`,
/// `ready` and `blocker_warnings`. These fields are computed only here; a
/// single [`get`] leaves them at their defaults.
pub fn list(vault: &Path) -> Vec<Task> {
    let mut tasks = list_parsed(vault);
    link_blockers(&mut tasks);
    tasks
}

/// Every task file under `tasks/`, parsed, without the blocker structure.
/// Takes no lock (the write-time cycle check calls it inside
/// `vault::mutate_note`).
pub fn list_parsed(vault: &Path) -> Vec<Task> {
    let root = vault.join(TASKS_DIR);
    if !root.exists() {
        return Vec::new();
    }
    let mut out: Vec<Task> = Vec::new();
    for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !name.ends_with(".md") || name.starts_with('.') {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(vault) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().to_string();
        // Each file read and parsed again only when its stamp moves (P5):
        // the Tasks page, Home's due card and the 30 s tick list every task.
        let stamp = entry.metadata().ok().map(|m| crate::file_stamp::stamp(&m));
        if let Some(cached) = stamp.and_then(|s| TASK_CACHE.get(entry.path(), s)) {
            out.extend(cached);
            continue;
        }
        #[cfg(test)]
        TASK_READS.with(|n| n.set(n.get() + 1));
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let task = parse(&rel_str, &content, vault);
        if let Some(stamp) = stamp {
            TASK_CACHE.put(entry.path(), stamp, task.clone());
        }
        out.extend(task);
    }
    out
}

/// Each task file's parse (`None`: not a task) by absolute path, while its
/// stamp holds ([`crate::file_stamp`]): `parse` depends only on the path and
/// the bytes.
static TASK_CACHE: crate::file_stamp::StampCache<Option<Task>> = crate::file_stamp::StampCache::new(
    if cfg!(test) { 100_000 } else { 20_000 },
    std::time::Duration::from_secs(60),
);
#[cfg(test)]
thread_local! {
    /// Task files read from disk on this thread (cache misses), for tests.
    static TASK_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Read and parse a single task file by vault-relative path. `safe_join`
/// guards against path escape.
pub fn get(vault: &Path, rel: &str) -> Option<Task> {
    let full = vault::safe_join(vault, rel).ok()?;
    let content = fs::read_to_string(&full).ok()?;
    parse(rel, &content, vault)
}

/// Parse a single task file into a `Task`. Returns `None` if the
/// frontmatter has no title.
pub fn parse(rel: &str, content: &str, _vault: &Path) -> Option<Task> {
    let (fm_map, body) = split_frontmatter(content);
    let title = fm_map.get("title")?.clone();
    if title.is_empty() {
        return None;
    }
    let status = fm_map
        .get("status")
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Not started".into());

    let mut priority = fm_map
        .get("priority")
        .map(|s| s.as_str())
        .and_then(normalize_priority);
    let due = fm_map.get("due").cloned().filter(|s| !s.is_empty());
    let due_time = fm_map.get("due_time").cloned().filter(|s| !s.is_empty());
    let duration_min = fm_map
        .get("duration_min")
        .and_then(|s| s.trim().parse::<u32>().ok());
    let effort = fm_map.get("effort").cloned().filter(|s| !s.is_empty());
    let project_name = fm_map.get("project").cloned().filter(|s| !s.is_empty());
    let project_url = fm_map.get("project_url").cloned().filter(|s| !s.is_empty());
    let section = fm_map.get("section").cloned().filter(|s| !s.is_empty());
    let recurrence = fm_map.get("recurrence").cloned().filter(|s| !s.is_empty());
    let labels = labels_from(&fm_map, content);

    // Sub-tasks are the normal-region checklist lines; reserved sections
    // (`## Done when`, `## Activity`) and fenced blocks never contribute.
    let tracker = crate::tracker_sync::seams::task_meta(content);
    if let Some(meta) = &tracker {
        priority = (meta.priority > 0).then(|| format!("P{}", meta.priority));
    }
    let local_body = if tracker.is_some() {
        crate::tracker::mirror::vault_notes(content).unwrap_or_default()
    } else {
        body.clone()
    };
    let pb = parse_body(&local_body);
    let subtasks = pb.subtasks.clone();

    // Description: explicit frontmatter `description:` wins; otherwise the
    // prose part of the body (steps and reserved sections left out).
    let description = if tracker.is_some() {
        crate::tracker::mirror::block(content, "description")
            .ok()
            .flatten()
            .unwrap_or_default()
    } else {
        fm_map
            .get("description")
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| pb.prose.clone())
    };

    let fm = TaskFrontmatter {
        title: title.clone(),
        status: status.clone(),
        priority: priority.clone(),
        due: due.clone(),
        due_time: due_time.clone(),
        duration_min,
        effort: effort.clone(),
        project: project_name.clone(),
        project_url: project_url.clone(),
        section: section.clone(),
        labels: labels.clone(),
        recurrence: recurrence.clone(),
        blocked_by: blocked_by_from(&fm_map, content),
        created: fm_map.get("created").cloned().filter(|s| !s.is_empty()),
        last_synced_at: fm_map
            .get("last_synced_at")
            .cloned()
            .filter(|s| !s.is_empty()),
        source: fm_map.get("source").cloned().filter(|s| !s.is_empty()),
        origin: fm_map.get("origin").cloned().filter(|s| !s.is_empty()),
        fingerprint: fm_map
            .get("fingerprint")
            .and_then(|s| normalize_fingerprint(s)),
        notion_id: fm_map.get("notion_id").cloned().filter(|s| !s.is_empty()),
        notion_url: fm_map.get("notion_url").cloned().filter(|s| !s.is_empty()),
    };
    let url = stable_url(&fm, rel);

    Some(Task {
        url,
        title,
        status,
        priority,
        due,
        effort,
        description,
        project_url,
        project_name,
        days_until_due: None,
        bucket: String::new(),
        vault_path: None,
        labels,
        section,
        due_time,
        duration_min,
        recurrence,
        subtasks,
        content_hash: content_hash(content),
        tracker,
        // Filled by readers that know the settings (`sources_api::label_tasks`).
        source_label: None,
        notion_id: fm.notion_id.clone(),
        fingerprint: fm.fingerprint.clone(),
        origin: fm.origin.clone(),
        done_when: pb.done_when(),
        activity: pb.activity(),
        blocked_by: fm.blocked_by.clone(),
        // Filled by `list` (`link_blockers`) only.
        blocks: Vec::new(),
        ready: false,
        unresolved_blockers: Vec::new(),
        blocker_warnings: Vec::new(),
    })
}

/// `blocked_by` as a comma scalar (Hollow's form) or, when that is empty, as
/// a YAML block list, normalized by [`parse_task_urls`].
fn blocked_by_from(fm_map: &BTreeMap<String, String>, content: &str) -> Vec<String> {
    match fm_map
        .get("blocked_by")
        .filter(|raw| !raw.trim().is_empty())
    {
        Some(raw) => match flow_list_items(raw) {
            // `blocked_by: [a, "b"]` (YAML flow style; `[]` is empty). Read
            // as literal text it would be two unresolvable entries, leaving
            // the task blocked for good with a bogus `missing` warning.
            Some(items) => parse_task_urls(items.iter().map(String::as_str)),
            None => parse_task_urls(raw.split(',')),
        },
        None => crate::frontmatter::list_items(content, "blocked_by")
            .map(|items| parse_task_urls(items.iter().map(String::as_str)))
            .unwrap_or_default(),
    }
}

/// The items of a one-line YAML flow sequence (`[a, "b", 'c']`), each
/// trimmed and unquoted; `[]` is empty. `None` when `raw` isn't bracketed.
/// Commas inside quoted items are not special-cased (task paths have none).
fn flow_list_items(raw: &str) -> Option<Vec<String>> {
    let inner = raw.trim().strip_prefix('[')?.strip_suffix(']')?;
    Some(
        inner
            .split(',')
            .map(|item| crate::frontmatter::unquote(item.trim()))
            .filter(|item| !item.trim().is_empty())
            .collect(),
    )
}

/// Tolerant reader for hand-edited task links: trimmed, empties dropped, a
/// bare `tasks/…` entry read as `vault:tasks/…`, duplicates (compared
/// case-insensitively) dropped with the first spelling kept. Entries that
/// don't resolve stay visible so the UI can warn about them.
pub fn parse_task_urls<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for raw in values {
        let value = raw.trim();
        if value.is_empty() {
            continue;
        }
        let url = if value.starts_with("vault:") {
            value.to_string()
        } else if value.starts_with(&format!("{TASKS_DIR}/")) {
            format!("vault:{value}")
        } else {
            value.to_string()
        };
        if seen.insert(url.to_lowercase()) {
            out.push(url);
        }
    }
    out
}

// ── Blockers and readiness (Paperclip PR 11b) ────────────────────────────

/// Resolves `blocked_by` entries to tasks: the exact `vault:` URL first, then
/// a unique case-insensitive match (an entry typed as `vault:tasks/Foo.md`
/// for `tasks/foo.md`). An ambiguous case-insensitive match resolves to
/// nothing. One resolver serves readiness, the reverse index, the warnings,
/// the trigger set and both cycle checks, so they can never disagree.
pub struct BlockerIndex {
    exact: HashMap<String, usize>,
    /// Lowercased URL → the only task with it, or `None` when several share it.
    folded: HashMap<String, Option<usize>>,
}

impl BlockerIndex {
    pub fn new(tasks: &[Task]) -> Self {
        let mut exact = HashMap::new();
        let mut folded: HashMap<String, Option<usize>> = HashMap::new();
        for (i, task) in tasks.iter().enumerate() {
            exact.entry(task.url.clone()).or_insert(i);
            folded
                .entry(task.url.to_lowercase())
                .and_modify(|slot| *slot = None)
                .or_insert(Some(i));
            if let Some(tracker) = &task.tracker {
                for alias in tracker
                    .key
                    .as_deref()
                    .into_iter()
                    .chain(std::iter::once(tracker.id.as_str()))
                    .filter(|s| !s.is_empty())
                {
                    folded
                        .entry(alias.to_lowercase())
                        .and_modify(|slot| {
                            if *slot != Some(i) {
                                *slot = None;
                            }
                        })
                        .or_insert(Some(i));
                }
            }
        }
        BlockerIndex { exact, folded }
    }

    /// The index (into the `tasks` the index was built from) of the task an
    /// entry names, or `None`.
    pub fn resolve(&self, url: &str) -> Option<usize> {
        let url = url.trim();
        self.exact
            .get(url)
            .copied()
            .or_else(|| self.folded.get(&url.to_lowercase()).copied().flatten())
    }
}

/// Resolved blocker edges: `edges[i]` holds the tasks task `i` waits on.
fn blocker_edges(tasks: &[Task], index: &BlockerIndex) -> Vec<Vec<usize>> {
    tasks
        .iter()
        .map(|t| {
            t.blocked_by
                .iter()
                .filter_map(|url| index.resolve(url))
                .collect()
        })
        .collect()
}

/// A path `from → … → to` along `edges`, found by an iterative depth-first
/// search, or `None` when `to` is unreachable. `from == to` is a path of one.
fn cycle_path(edges: &[Vec<usize>], from: usize, to: usize) -> Option<Vec<usize>> {
    let mut parent: HashMap<usize, usize> = HashMap::new();
    let mut seen: HashSet<usize> = HashSet::from([from]);
    let mut stack = vec![from];
    while let Some(node) = stack.pop() {
        if node == to {
            let mut path = vec![to];
            let mut cur = to;
            while let Some(&prev) = parent.get(&cur) {
                path.push(prev);
                cur = prev;
            }
            path.reverse();
            return Some(path);
        }
        for &next in edges.get(node).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(next) {
                parent.insert(next, node);
                stack.push(next);
            }
        }
    }
    None
}

/// Fill each task's `blocks`, `unresolved_blockers`, `ready` and
/// `blocker_warnings` from the whole list.
///
/// - A blocker resolves when its status is terminal ([`is_terminal_status`]).
///   An entry that names no task, or a task that isn't terminal, is
///   unresolved.
/// - Warnings: `missing` (no task at that path), `cancelled`, `recurring`
///   (Hollow rolls a recurring task forward instead of completing it), and
///   `cycle` for each blocker that waits, directly or through others, on
///   this task (a per-task depth-first search).
/// - `ready` = own status neither terminal, cancelled nor on hold
///   ([`is_hold_status`]), and no unresolved blocker. Readiness looks one
///   level deep only.
pub fn link_blockers(tasks: &mut [Task]) {
    let index = BlockerIndex::new(tasks);
    let edges = blocker_edges(tasks, &index);
    let mut blocks: Vec<Vec<String>> = vec![Vec::new(); tasks.len()];
    let mut computed: Vec<(Vec<String>, Vec<BlockerWarning>)> = Vec::with_capacity(tasks.len());
    for (i, task) in tasks.iter().enumerate() {
        let mut unresolved = Vec::new();
        let mut warnings = Vec::new();
        for url in &task.blocked_by {
            let warn = |kind: &str| BlockerWarning {
                url: url.clone(),
                kind: kind.to_string(),
            };
            let Some(j) = index.resolve(url) else {
                unresolved.push(url.clone());
                warnings.push(warn("missing"));
                continue;
            };
            if !blocks[j].contains(&task.url) {
                blocks[j].push(task.url.clone());
            }
            let blocker = &tasks[j];
            if !task_is_done(blocker) {
                unresolved.push(url.clone());
                if task_is_canceled(blocker) {
                    warnings.push(warn("cancelled"));
                } else if blocker.recurrence.is_some() {
                    warnings.push(warn("recurring"));
                }
            }
            if cycle_path(&edges, j, i).is_some() {
                warnings.push(warn("cycle"));
            }
        }
        computed.push((unresolved, warnings));
    }
    for ((task, (unresolved, warnings)), mut blocked) in tasks.iter_mut().zip(computed).zip(blocks)
    {
        blocked.sort();
        task.ready = !task_is_done(task)
            && !task_is_canceled(task)
            && !is_hold_status(&task.status)
            && unresolved.is_empty();
        task.unresolved_blockers = unresolved;
        task.blocker_warnings = warnings;
        task.blocks = blocked;
    }
}

/// The URL checks a newly added `blocked_by` entry must pass before it can be
/// stored in the comma scalar.
fn blocker_shape_error(url: &str) -> Option<&'static str> {
    if url.contains(',') {
        Some("it contains a comma, which the blocked_by list can't store; rename that task file first")
    } else if url.chars().any(|c| matches!(c, '\n' | '\r' | '\0')) {
        Some("it contains a line break")
    } else if url.contains("..") {
        Some("it contains `..`")
    } else if !url.starts_with(&format!("vault:{TASKS_DIR}/")) {
        Some("blockers must be vault:tasks/… task links")
    } else {
        None
    }
}

/// The `blocked_by` list to store for an edit (or a create, `self_url` =
/// `None`): `requested` in order, deduplicated case-insensitively. An entry
/// already in `previous` is kept exactly as stored, even when it no longer
/// resolves, so other entries stay editable. Every NEWLY added entry must
/// pass the shape checks and resolve through [`BlockerIndex`]
/// (`TASK_BLOCKER: …`) and is stored as the task's canonical URL; naming the
/// task itself is `TASK_CYCLE: …`.
pub fn normalize_blocked_by(
    tasks: &[Task],
    self_url: Option<&str>,
    previous: &[String],
    requested: &[String],
) -> Result<Vec<String>, String> {
    let index = BlockerIndex::new(tasks);
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for entry in parse_task_urls(requested.iter().map(String::as_str)) {
        if let Some(stored) = previous.iter().find(|p| p.eq_ignore_ascii_case(&entry)) {
            if seen.insert(stored.to_lowercase()) {
                out.push(stored.clone());
            }
            continue;
        }
        if let Some(why) = blocker_shape_error(&entry) {
            return Err(format!(
                "{TASK_BLOCKER_ERROR}: {entry} can't be a blocker: {why}. Nothing was saved."
            ));
        }
        let Some(j) = index.resolve(&entry) else {
            return Err(format!(
                "{TASK_BLOCKER_ERROR}: no task exists at {entry}. Nothing was saved; pick an existing task as the blocker."
            ));
        };
        let canonical = tasks[j].url.clone();
        if self_url.is_some_and(|me| me.eq_ignore_ascii_case(&canonical)) {
            return Err(format!(
                "{TASK_CYCLE_ERROR}: a task can't be blocked by itself ({}). Nothing was saved.",
                tasks[j].title
            ));
        }
        if seen.insert(canonical.to_lowercase()) {
            out.push(canonical);
        }
    }
    Ok(out)
}

/// Refuse a `blocked_by` list that would close a loop through `self_url`:
/// with this task's edges replaced by `blockers`, a depth-first search from
/// each entry of `added` (the newly added blockers) must not reach the task
/// again. `self_url` may name a file that doesn't exist yet (a create), and
/// existing entries that already point at it count. Returns
/// `TASK_CYCLE: A → B → A` (titles) on a loop.
pub fn check_blocker_cycle(
    tasks: &[Task],
    self_url: &str,
    self_title: &str,
    blockers: &[String],
    added: &[String],
) -> Result<(), String> {
    let index = BlockerIndex::new(tasks);
    let self_folded = self_url.to_lowercase();
    let existing_self = index.resolve(self_url);
    let me = existing_self.unwrap_or(tasks.len());
    let resolve = |url: &str| -> Option<usize> {
        index
            .resolve(url)
            .or_else(|| (url.trim().to_lowercase() == self_folded).then_some(me))
    };
    let mut edges: Vec<Vec<usize>> = tasks
        .iter()
        .map(|t| {
            t.blocked_by
                .iter()
                .filter_map(|url| resolve(url.as_str()))
                .collect()
        })
        .collect();
    if existing_self.is_none() {
        edges.push(Vec::new());
    }
    edges[me] = blockers
        .iter()
        .filter_map(|url| resolve(url.as_str()))
        .collect();
    for url in added {
        let Some(start) = resolve(url.as_str()) else {
            continue;
        };
        if let Some(path) = cycle_path(&edges, start, me) {
            let names: Vec<String> = std::iter::once(me)
                .chain(path)
                .map(|i| {
                    if i == me {
                        self_title.to_string()
                    } else {
                        tasks.get(i).map(|t| t.title.clone()).unwrap_or_default()
                    }
                })
                .collect();
            return Err(format!(
                "{TASK_CYCLE_ERROR}: {}. Nothing was saved; a task can't wait on work that waits on it.",
                names.join(" → ")
            ));
        }
    }
    Ok(())
}

/// `labels` as a comma scalar (Hollow's form) or, when that is empty, as a
/// YAML block list (the shape Obsidian writes for a List property), which
/// the scalar parser skips. Editing labels in Hollow rewrites either form as
/// the comma scalar.
fn labels_from(fm_map: &BTreeMap<String, String>, content: &str) -> Vec<String> {
    match fm_map.get("labels").filter(|raw| !raw.trim().is_empty()) {
        Some(raw) => parse_labels(raw),
        None => crate::frontmatter::list_items(content, "labels")
            .map(|items| parse_labels(&items.join(",")))
            .unwrap_or_default(),
    }
}

/// TaskModal names of the fields a tracker issue can own.
const SYNCED_MODAL_FIELDS: [&str; 6] = [
    "title",
    "status",
    "priority",
    "due",
    "description",
    "project",
];

/// Fill each task's tracker lock (`Task.tracker`) for a listed batch, so
/// [`held_modal_fields`] and TaskModal see what the tracker holds.
pub fn mark_task_locks(vault: &Path, items: &mut [Task]) {
    crate::tracker_sync::seams::mark_locks(vault, items);
}

/// The fields a writer must not change on `task` (Paperclip PR 12): the
/// fields its tracker lock holds, plus `dueTime` when `due` is held and the
/// due carries a time (`YYYY-MM-DDTHH:MM…`), because a local `due_time`
/// would mask the tracker's time and never sync. Call [`mark_task_locks`]
/// first.
///
/// Keep in step with `lockedTaskFields` in `src/taskEdit.ts`, the same rule
/// for TaskModal.
pub(crate) fn held_modal_fields(task: &Task) -> BTreeSet<&'static str> {
    let mut held: BTreeSet<&'static str> = BTreeSet::new();
    if let Some(tracker) = &task.tracker {
        held.extend(
            SYNCED_MODAL_FIELDS
                .iter()
                .copied()
                .filter(|field| tracker.lock.fields.iter().any(|key| key == field)),
        );
    }
    if held.contains("due") && task.due.as_deref().is_some_and(|d| due_has_time(d.trim())) {
        held.insert("dueTime");
    }
    held
}

/// True for a due value shaped `YYYY-MM-DDTHH:MM…` (the `dueTimePart` test
/// in `src/taskEdit.ts`).
fn due_has_time(raw: &str) -> bool {
    let b = raw.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    b.len() >= 16
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
}

#[cfg(test)]
pub fn notion_id(vault: &Path, rel: &str) -> Option<String> {
    let full = vault::safe_join(vault, rel).ok()?;
    let content = fs::read_to_string(&full).ok()?;
    let (fm_map, _) = split_frontmatter(&content);
    fm_map.get("notion_id").cloned().filter(|s| !s.is_empty())
}

/// Find the task file with the given stable URL. Every task is keyed by
/// its `vault:` URI now.
pub fn find_by_url(vault: &Path, url: &str) -> Option<String> {
    let rel = url.strip_prefix("vault:")?;
    // Resolve through safe_join so a crafted `vault:../…` URL can't
    // escape the vault root — reject it rather than hand back a
    // traversal path that a later mutate/delete would act on.
    let full = vault::safe_join(vault, rel).ok()?;
    if full.exists() {
        Some(rel.to_string())
    } else {
        None
    }
}

/// Resolve a task reference from an agent (`vault:tasks/…md` or
/// `tasks/…md`) to a vault-relative path (Paperclip PR 12). Stricter than
/// [`find_by_url`]: the path must sit under `tasks/`, end in `.md`, have no
/// empty, `.`, `..` or dot-prefixed component, resolve through
/// [`vault::safe_join`], name an existing file that is not a symlink, and
/// resolve inside the resolved `tasks/` folder.
pub fn resolve_task_ref(vault: &Path, reference: &str) -> Result<String, String> {
    let raw = reference.trim();
    let rel = raw.strip_prefix("vault:").unwrap_or(raw);
    let malformed = || {
        format!(
            "Not a task reference: {raw:?}. Use a task's ref as list_tasks returns it (vault:tasks/<name>.md)."
        )
    };
    if !rel.starts_with("tasks/") || !rel.ends_with(".md") || rel.contains('\\') {
        return Err(malformed());
    }
    if rel
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == ".." || part.starts_with('.'))
    {
        return Err(malformed());
    }
    let full = vault::safe_join(vault, rel).map_err(|_| malformed())?;
    if !full.is_file() {
        return Err(format!(
            "No task at {rel}; it may have been moved or deleted. List the tasks again."
        ));
    }
    // `safe_join` only keeps the target inside the vault, and the writers
    // replace the canonical target, so a symlink under `tasks/` (the file
    // itself, or a subfolder) could aim a task write at `me/` or watched
    // dot-state. Refuse a symlinked file and any target that resolves
    // outside the resolved `tasks/` folder.
    let is_link = fs::symlink_metadata(vault.join(rel))
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true);
    let inside_tasks = vault
        .join(TASKS_DIR)
        .canonicalize()
        .is_ok_and(|tasks| full.starts_with(&tasks));
    if is_link || !inside_tasks {
        return Err(format!(
            "{rel} is a link that points outside tasks/; task tools edit only real task files."
        ));
    }
    Ok(rel.to_string())
}

/// Pick a non-colliding filename for a new task. Tries `slug.md`,
/// then `slug-2.md`, `slug-3.md`, etc. Mirrors Obsidian's
/// auto-rename behavior — keeps filenames human-readable.
#[cfg(test)]
fn pick_filename(vault: &Path, title: &str) -> String {
    pick_filename_avoiding(vault, title, &HashSet::new())
}

/// [`pick_filename`], also skipping every candidate whose lowercased
/// vault-relative path is in `avoid`: the paths that some task's `blocked_by`
/// still names although no file is there (a renamed or deleted blocker).
/// Taking such a name would silently make an unrelated new task that task's
/// blocker, so a new task lands at `slug-2.md` instead (Paperclip PR 11b).
fn pick_filename_avoiding(vault: &Path, title: &str, avoid: &HashSet<String>) -> String {
    let base = slugify(title);
    let base = safe_filename(&base);
    let free = |rel: &str| !avoid.contains(&rel.to_lowercase()) && !vault.join(rel).exists();
    let first = format!("{TASKS_DIR}/{base}.md");
    if free(&first) {
        return first;
    }
    for i in 2..1000 {
        let candidate = format!("{TASKS_DIR}/{base}-{i}.md");
        if free(&candidate) {
            return candidate;
        }
    }
    // Pathological — give up and stamp a timestamp.
    let ts = Utc::now().format("%Y%m%d%H%M%S");
    format!("{TASKS_DIR}/{base}-{ts}.md")
}

/// Create a task as a new file at `tasks/{slug}.md` (collision-suffixed),
/// always a new file. Returns the vault-relative path. Production creates go
/// through [`create_idempotent`] / [`create_from_input`]; tests use this.
#[cfg(test)]
pub fn upsert(vault: &Path, fm: TaskFrontmatter, body_md: Option<&str>) -> Result<String, String> {
    let content = render(&fm, body_md.unwrap_or_default());
    // Filename selection is advisory. Obsidian or another task create may
    // publish between the existence check and this write, so publication must
    // be exclusive and retry with a new suffix when that race is lost.
    for _ in 0..1000 {
        let rel = pick_filename(vault, &fm.title);
        match vault::create_note(vault, &rel, &content) {
            Ok(_) => return Ok(rel),
            Err(error) if error.starts_with("Note already exists:") => continue,
            Err(error) => return Err(error),
        }
    }
    Err("too many concurrent task filename collisions".into())
}

// ── Idempotent create (Paperclip PR 11) ──────────────────────────────────

/// Error prefix for an unusable `idempotencyKey` (over [`TASK_KEY_MAX_BYTES`]).
pub const TASK_KEY_ERROR: &str = "TASK_KEY";
/// Error prefix when a key matches a task whose title slug differs from the
/// request under [`ExistingPolicy::RequireSameTitleSlug`].
pub const TASK_IDEMPOTENCY_MISMATCH_ERROR: &str = "IDEMPOTENCY_MISMATCH";
/// IPC hygiene for `CreateTaskInput.idempotencyKey`; the stored fingerprint
/// is a fixed-width digest whatever the key's length.
pub const TASK_KEY_MAX_BYTES: usize = 16 * 1024;
const ORIGIN_MAX_CHARS: usize = 200;

/// Serializes every idempotent create in this process: the scan for an
/// existing fingerprint or slug and the exclusive publish happen as one step.
///
/// Lock order: [`crate::automated_history::with_live_run`]'s `HISTORY_LOCK`
/// (PR 12's agent writes) → `CREATE_LOCK` → {vault INDEX,
/// `vault_search_index`, `vault_embed_index`} (taken inside
/// [`vault::create_note`]). It is never taken while holding
/// `vault::MUTATE_LOCK` or any of those locks, nothing inside an
/// `apply_update` / `mutate_note` closure creates a task, and nothing under
/// it takes `HISTORY_LOCK`. It is in-process only: PR 12's `hollow tasks-mcp`
/// subprocess never writes; it proxies every write to the GUI, which runs it
/// here (there is exactly one `CREATE_LOCK`).
static CREATE_LOCK: Mutex<()> = Mutex::new(());

/// The fingerprint stored for an idempotency key: sha256 hex of the trimmed
/// key, or `None` for a blank key. Callers namespace and bound their keys
/// (`heartbeat-task\n<slug>`, `project-draft:<rel>\n<text>`,
/// `plan:<path>\n<sha>`, a review item's own fingerprint).
pub fn fingerprint_for_key(key: &str) -> Option<String> {
    let key = key.trim();
    (!key.is_empty()).then(|| format!("{:x}", Sha256::digest(key.as_bytes())))
}

/// A stored fingerprint as compared: trimmed, lowercase, non-empty.
fn normalize_fingerprint(raw: &str) -> Option<String> {
    let fp = raw.trim().to_ascii_lowercase();
    (!fp.is_empty()).then_some(fp)
}

/// An `origin:` value: one line, at most 200 characters, `None` when blank.
pub fn task_origin(raw: &str) -> Option<String> {
    let flat: String = raw
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(ORIGIN_MAX_CHARS)
        .collect();
    (!flat.is_empty()).then_some(flat)
}

/// What [`create_idempotent`] does when the fingerprint matches a task.
/// `RequireSameTitleSlug` is for PR 12's MCP create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingPolicy {
    /// Return the existing task.
    Return,
    /// Return it only when its title slug equals the request's; otherwise
    /// fail with `IDEMPOTENCY_MISMATCH: …` (a reused key for different work).
    RequireSameTitleSlug,
}

/// The task a create resolved to: a new file, or (`existing`) the task an
/// earlier create with the same fingerprint (or, with the slug fallback, the
/// same title slug) already made. `content_hash`, `title` and `status` are
/// for PR 12's MCP create result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedTaskFile {
    pub rel: String,
    pub existing: bool,
    /// sha256 of the file text: the `Task.contentHash` baseline.
    pub content_hash: String,
    pub title: String,
    pub status: String,
}

/// One task file as the create scan sees it.
struct ScannedTask {
    rel: String,
    fingerprint: Option<String>,
    slug: String,
    title: String,
    status: String,
    content_hash: String,
    /// Its `blocked_by` entries, for [`dangling_blocker_rels`].
    blocked_by: Vec<String>,
}

impl ScannedTask {
    fn existing(&self) -> CreatedTaskFile {
        CreatedTaskFile {
            rel: self.rel.clone(),
            existing: true,
            content_hash: self.content_hash.clone(),
            title: self.title.clone(),
            status: self.status.clone(),
        }
    }
}

/// Every titled task file under `tasks/`, sorted by path (the same files
/// [`list`] returns).
fn scan_tasks(vault: &Path) -> Vec<ScannedTask> {
    let root = vault.join(TASKS_DIR);
    if !root.exists() {
        return Vec::new();
    }
    let mut out: Vec<ScannedTask> = Vec::new();
    for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !name.ends_with(".md") || name.starts_with('.') {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(vault) else {
            continue;
        };
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let (fm_map, _) = split_frontmatter(&content);
        let Some(title) = fm_map.get("title").filter(|t| !t.is_empty()) else {
            continue;
        };
        out.push(ScannedTask {
            rel: rel.to_string_lossy().to_string(),
            fingerprint: fm_map
                .get("fingerprint")
                .and_then(|f| normalize_fingerprint(f)),
            slug: slugify(title),
            title: title.clone(),
            status: fm_map
                .get("status")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Not started".into()),
            content_hash: content_hash(&content),
            blocked_by: blocked_by_from(&fm_map, &content),
        });
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    out
}

#[cfg(test)]
thread_local! {
    /// Test-only: makes [`create_idempotent`] ignore the dangling-name avoid
    /// set, so a test can reach its otherwise unreachable loop check.
    static IGNORE_DANGLING_AVOID: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The lowercased vault-relative paths some task's `blocked_by` names while
/// no scanned task lives there: the names a new task must not take
/// ([`pick_filename_avoiding`]).
fn dangling_blocker_rels(scanned: &[ScannedTask]) -> HashSet<String> {
    let present: HashSet<String> = scanned.iter().map(|t| t.rel.to_lowercase()).collect();
    scanned
        .iter()
        .flat_map(|t| t.blocked_by.iter())
        .filter_map(|url| url.strip_prefix("vault:"))
        .map(|rel| rel.trim().to_lowercase())
        .filter(|rel| !present.contains(rel))
        .collect()
}

/// `fingerprint → vault:<rel>` for every task that carries one (the
/// smallest path wins a duplicate).
pub fn fingerprint_index(vault: &Path) -> HashMap<String, String> {
    let mut index = HashMap::new();
    for task in scan_tasks(vault) {
        if let Some(fp) = task.fingerprint {
            index
                .entry(fp)
                .or_insert_with(|| format!("vault:{}", task.rel));
        }
    }
    index
}

/// Create a task unless an equivalent one exists, as one step under
/// [`CREATE_LOCK`]:
/// 1. a task whose stored fingerprint equals `fm.fingerprint` (smallest path
///    first) is returned, subject to `policy`;
/// 2. otherwise, with `slug_fallback`, a task whose title slug equals the
///    request's (the identity `pick_filename` uses, and the only one the
///    tasks written before fingerprints existed have) is returned;
/// 3. otherwise `fm` is rendered with `body` and published exclusively at a
///    free `tasks/{slug}.md` (collision-suffixed), retrying a lost race.
///    The name skips any path a dangling `blocked_by` entry still names
///    ([`pick_filename_avoiding`]). A non-empty `fm.blocked_by` is validated
///    and canonicalized first ([`normalize_blocked_by`], `TASK_BLOCKER: …`)
///    and checked for a loop through the picked path
///    ([`check_blocker_cycle`], `TASK_CYCLE: …`); a refusal writes nothing.
pub fn create_idempotent(
    vault: &Path,
    mut fm: TaskFrontmatter,
    body: &str,
    slug_fallback: bool,
    policy: ExistingPolicy,
) -> Result<CreatedTaskFile, String> {
    if fm.title.trim().is_empty() {
        return Err("A task needs a title; nothing was created.".into());
    }
    let _guard = CREATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let scanned = scan_tasks(vault);
    let slug = slugify(&fm.title);
    let wanted = fm.fingerprint.as_deref().and_then(normalize_fingerprint);
    if let Some(fp) = wanted.as_deref() {
        if let Some(hit) = scanned
            .iter()
            .find(|t| t.fingerprint.as_deref() == Some(fp))
        {
            if policy == ExistingPolicy::RequireSameTitleSlug && hit.slug != slug {
                return Err(format!(
                    "{TASK_IDEMPOTENCY_MISMATCH_ERROR}: this idempotency key already created \"{}\" ({}), which is different work from \"{}\". Nothing was created; use a new key for new work.",
                    hit.title, hit.rel, fm.title
                ));
            }
            return Ok(hit.existing());
        }
    }
    if slug_fallback {
        if let Some(hit) = scanned.iter().find(|t| t.slug == slug) {
            return Ok(hit.existing());
        }
    }
    // Blockers are validated only for a genuinely new task (an existing
    // match above is returned as it is).
    let blocker_tasks = (!fm.blocked_by.is_empty()).then(|| list_parsed(vault));
    if let Some(tasks) = &blocker_tasks {
        fm.blocked_by = normalize_blocked_by(tasks, None, &[], &fm.blocked_by)?;
    }
    let avoid = dangling_blocker_rels(&scanned);
    #[cfg(test)]
    let avoid = if IGNORE_DANGLING_AVOID.with(|c| c.get()) {
        HashSet::new()
    } else {
        avoid
    };
    let content = render(&fm, body);
    let status = Some(fm.status.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Not started".into());
    for _ in 0..1000 {
        let rel = pick_filename_avoiding(vault, &fm.title, &avoid);
        if let Some(tasks) = &blocker_tasks {
            // Defense in depth: with dangling names avoided nothing in
            // `tasks` points at a new path, so production can't reach a loop
            // here today; it is still refused (a test proves it by switching
            // the avoid set off, `IGNORE_DANGLING_AVOID`).
            check_blocker_cycle(
                tasks,
                &format!("vault:{rel}"),
                &fm.title,
                &fm.blocked_by,
                &fm.blocked_by,
            )?;
        }
        match vault::create_note(vault, &rel, &content) {
            Ok(_) => {
                return Ok(CreatedTaskFile {
                    rel,
                    existing: false,
                    content_hash: content_hash(&content),
                    title: fm.title.clone(),
                    status,
                })
            }
            Err(error) if error.starts_with("Note already exists:") => continue,
            Err(error) => return Err(error),
        }
    }
    Err("too many concurrent task filename collisions".into())
}

/// The shared create path behind the `create_task` command (and PR 12's
/// MCP create): normalizes a [`CreateTaskInput`] (project link, priority,
/// labels) and creates it through [`create_idempotent`], by fingerprint only
/// (no slug fallback: two tasks may share a title) and
/// [`ExistingPolicy::Return`]. A key over [`TASK_KEY_MAX_BYTES`] returns
/// `TASK_KEY: …`; a blank key means no fingerprint.
pub fn create_from_input(vault: &Path, input: &CreateTaskInput) -> Result<CreatedTaskFile, String> {
    create_from_input_with(vault, input, ExistingPolicy::Return)
}

/// [`create_from_input`] with an explicit [`ExistingPolicy`] for a key hit.
fn create_from_input_with(
    vault: &Path,
    input: &CreateTaskInput,
    policy: ExistingPolicy,
) -> Result<CreatedTaskFile, String> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("A task needs a title; nothing was created.".into());
    }
    let fingerprint = match input.idempotency_key.as_deref() {
        Some(key) if key.len() > TASK_KEY_MAX_BYTES => {
            return Err(format!(
                "{TASK_KEY_ERROR}: the idempotency key is {} bytes; the limit is {TASK_KEY_MAX_BYTES}. Nothing was created.",
                key.len()
            ))
        }
        Some(key) => fingerprint_for_key(key),
        None => None,
    };
    let now = Utc::now().to_rfc3339();
    let selected_project = input
        .project_url
        .as_deref()
        .map(|url| crate::vault_projects::task_project_link(vault, url))
        .transpose()?;
    let fm = TaskFrontmatter {
        title: title.to_string(),
        status: input
            .status
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Not started".into()),
        priority: input.priority.as_deref().and_then(normalize_priority),
        due: input.due.clone().filter(|s| !s.is_empty()),
        due_time: input.due_time.clone().filter(|s| !s.is_empty()),
        duration_min: input.duration_min,
        effort: None,
        project: selected_project
            .as_ref()
            .map(|p| p.0.clone())
            .or_else(|| input.project.clone().filter(|s| !s.is_empty())),
        project_url: selected_project.map(|p| p.1),
        section: input.section.clone().filter(|s| !s.is_empty()),
        labels: parse_labels(&input.labels.join(",")),
        recurrence: input.recurrence.clone().filter(|s| !s.is_empty()),
        // Validated and canonicalized under CREATE_LOCK by create_idempotent.
        blocked_by: parse_task_urls(input.blocked_by.iter().map(String::as_str)),
        created: Some(now.clone()),
        last_synced_at: Some(now),
        source: None,
        origin: input.origin.as_deref().and_then(task_origin),
        fingerprint,
        notion_id: None,
        notion_url: None,
    };
    let body = input.description.clone().unwrap_or_default();
    create_idempotent(vault, fm, &body, false, policy)
}

/// The idempotency key an agent run's create is stored under (Paperclip
/// PR 12): `agent:` + the first 32 hex of sha256(surface ‖ 0 ‖ scope ‖ 0 ‖
/// trimmed key), or `None` for a blank key. `scope` is the chat session key
/// or the workflow slug, so a retried turn or retry row dedups while the same
/// generic key ("task-1") in another conversation or automation never
/// absorbs into an unrelated task. [`create_idempotent`] hashes the result
/// again for the stored fingerprint, which is harmless.
pub(crate) fn scoped_agent_key(surface: &str, scope: &str, key: &str) -> Option<String> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    for part in [surface.trim(), scope.trim(), key] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let digest = format!("{:x}", hasher.finalize());
    Some(format!("agent:{}", &digest[..32]))
}

/// An agent run's create through the shared path (Paperclip PR 12; the
/// `hollow_tasks` broker calls it inside
/// [`crate::automated_history::with_live_run`]). The caller's
/// `idempotency_key` is required and scoped by [`scoped_agent_key`]; the
/// `origin` is `run:<history run id>`. A key that already created a task
/// returns that task (`existing`) when its current title slug equals the
/// request's, and otherwise fails with `IDEMPOTENCY_MISMATCH: …` and writes
/// nothing ([`ExistingPolicy::RequireSameTitleSlug`], decided under the one
/// `CREATE_LOCK`). A retry after the user renamed the task therefore errors,
/// which fails safe. Field validation is the broker's job; this keeps the
/// UI command's normalization.
pub(crate) fn create_agent_task(
    vault: &Path,
    input: &CreateTaskInput,
    surface: &str,
    scope: &str,
    run_id: &str,
) -> Result<CreatedTaskFile, String> {
    let raw_key = input.idempotency_key.as_deref().unwrap_or_default();
    if raw_key.len() > TASK_KEY_MAX_BYTES {
        return Err(format!(
            "{TASK_KEY_ERROR}: the idempotency key is {} bytes; the limit is {TASK_KEY_MAX_BYTES}. Nothing was created.",
            raw_key.len()
        ));
    }
    let Some(key) = scoped_agent_key(surface, scope, raw_key) else {
        return Err(format!(
            "{TASK_KEY_ERROR}: an idempotencyKey is required; reuse the same key when you retry. Nothing was created."
        ));
    };
    let scoped = CreateTaskInput {
        idempotency_key: Some(key),
        origin: Some(format!("run:{}", run_id.trim())),
        ..input.clone()
    };
    create_from_input_with(vault, &scoped, ExistingPolicy::RequireSameTitleSlug)
}

/// A paired companion device's create (`companion_inbox`, through the task
/// broker's validation). The key is `companion:<opId>`, so the same op
/// delivered twice (or re-applied after the applied-op ledger was lost)
/// makes one task; the `origin` is `companion:<deviceId>`. An existing task
/// for the key is returned as is, even if the person renamed it since.
pub(crate) fn create_companion_task(
    vault: &Path,
    input: &CreateTaskInput,
    device_id: &str,
    op_id: &str,
) -> Result<CreatedTaskFile, String> {
    let op_id = op_id.trim();
    if op_id.is_empty() {
        return Err(format!(
            "{TASK_KEY_ERROR}: a companion create needs its op id. Nothing was created."
        ));
    }
    let scoped = CreateTaskInput {
        idempotency_key: Some(format!("companion:{op_id}")),
        origin: Some(format!("companion:{}", device_id.trim())),
        ..input.clone()
    };
    create_from_input_with(vault, &scoped, ExistingPolicy::Return)
}

/// Legacy writers must not normalize provider-owned fields or body blocks.
fn refuse_legacy_tracker_writer(content: &str) -> Result<(), String> {
    if crate::tracker_sync::seams::task_meta(content).is_some() {
        return Err("TASK_LINKED: Use the task tracker editor so its owned fields and comments are preserved.".into());
    }
    Ok(())
}

/// Mutate a local task's frontmatter and return its written content hash.
/// Only changed modeled keys are patched; the other keys and body stay.
#[cfg(test)]
pub fn mutate(
    vault: &Path,
    rel: &str,
    f: impl FnOnce(&mut TaskFrontmatter),
) -> Result<String, String> {
    mutate_checked(vault, rel, None, f)
}

/// [`mutate`] with an optional compare-and-swap baseline: a non-empty
/// `expected` that no longer matches the file returns `TASK_CONFLICT: …` and
/// writes nothing.
fn mutate_checked(
    vault: &Path,
    rel: &str,
    expected: Option<&str>,
    f: impl FnOnce(&mut TaskFrontmatter),
) -> Result<String, String> {
    let expected = expected.filter(|h| !h.is_empty());
    // Read-modify-write under vault::MUTATE_LOCK (data safety Tier 0.3): the
    // parse → apply `f` → patch → write runs in one critical section, so two
    // concurrent mutators can't both read the same base and lose one update
    // (the TOCTOU window the old two-hop read-then-write had). `mutate_note`
    // safe_joins; the write goes through it (atomic).
    let note = vault::mutate_note(vault, rel, |content| {
        refuse_legacy_tracker_writer(content)?;
        if let Some(expected) = expected {
            if content_hash(content) != expected {
                return Err(conflict_error(rel));
            }
        }
        let (fm_map, _) = split_frontmatter(content);
        let title = fm_map.get("title").cloned().unwrap_or_default();
        if title.is_empty() {
            return Err(format!("task file has no title: {rel}"));
        }
        let mut old = frontmatter_from_map(&fm_map, content);
        old.title = title;
        let mut new = old.clone();
        f(&mut new);
        new.last_synced_at = Some(Utc::now().to_rfc3339());
        patch_task(content, &old, &new, Vec::new(), None)
    })?;
    Ok(content_hash(&note.content))
}

/// Write `new_status` verbatim, with no recurrence roll-forward. Production
/// status writes go through [`set_status_with_recurrence`]; tests use this.
#[cfg(test)]
pub fn set_status(vault: &Path, rel: &str, new_status: &str) -> Result<String, String> {
    mutate(vault, rel, |fm| {
        fm.status = new_status.to_string();
    })
}

#[cfg(test)]
pub fn set_priority(vault: &Path, rel: &str, new_priority: Option<&str>) -> Result<String, String> {
    set_priority_checked(vault, rel, new_priority, None)
}

/// Set a task's priority, the writer behind the `set_task_priority` command.
/// With a non-empty `expected` the write is compare-and-swap (review F01,
/// 2026-10-13): a bulk apply sends the previewed `contentHash` and its Undo
/// sends the hash the apply returned, so a newer edit from another surface,
/// an agent, sync or an outside editor is refused with `TASK_CONFLICT: …`
/// and left on disk instead of being overwritten.
pub fn set_priority_checked(
    vault: &Path,
    rel: &str,
    new_priority: Option<&str>,
    expected: Option<&str>,
) -> Result<String, String> {
    mutate_checked(vault, rel, expected, |fm| {
        fm.priority = new_priority.and_then(normalize_priority);
    })
}

/// The `TASK_CONFLICT: …` refusal for a stale compare-and-swap baseline.
fn conflict_error(rel: &str) -> String {
    format!(
        "{TASK_CONFLICT_ERROR}: {rel} changed on disk after it was opened. Nothing was saved; reopen the task to see the latest version, then reapply your edit."
    )
}

/// What [`set_status_with_recurrence`] wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusOutcome {
    /// sha256 of the bytes written (the new `Task.contentHash`).
    pub content_hash: String,
    /// The status now on disk (`Not started` after a roll-forward).
    pub status: String,
    /// The next due date when a recurring task rolled forward instead of
    /// finishing.
    pub rolled_to: Option<String>,
}

/// Set a task's status, the one writer behind the `set_task_status` command
/// and PR 12's `set_status` tool. Completing a recurring task doesn't finish
/// it: when `status` is terminal ([`is_terminal_status`]), the task has a
/// `recurrence` and [`crate::quick_add::next_occurrence`] from `today` (the
/// completion date, so completing early or late still lands the next
/// occurrence and overdue tasks don't stack) gives a date, `due` moves there
/// and the status becomes `Not started`. Otherwise the trimmed `status` is
/// written. An empty status is refused.
///
/// The recurrence read, the decision and the write happen in ONE
/// `vault::mutate_note` critical section. The command used to read the
/// recurrence outside the lock and then mutate, so an edit in between could
/// finish a task that had just become recurring, or roll one that no longer
/// was. With a non-empty `expected` the write is compare-and-swap: a
/// mismatch returns `TASK_CONFLICT: …` and writes nothing.
pub fn set_status_with_recurrence(
    vault: &Path,
    rel: &str,
    status: &str,
    today: NaiveDate,
    expected: Option<&str>,
) -> Result<StatusOutcome, String> {
    set_status_finishing(vault, rel, status, today, expected, &|_, text| Ok(text))
}

/// A last step over a task write's new text, run inside its
/// `vault::mutate_note` critical section after the compare-and-swap check:
/// `(old text, new text) → text to write`, or an error that writes nothing.
pub(crate) type Finish<'a> = &'a dyn Fn(&str, String) -> Result<String, String>;

/// [`set_status_with_recurrence`] with a [`Finish`] step.
pub(crate) fn set_status_finishing(
    vault: &Path,
    rel: &str,
    status: &str,
    today: NaiveDate,
    expected: Option<&str>,
    finish: Finish,
) -> Result<StatusOutcome, String> {
    let requested = status.trim();
    if requested.is_empty() {
        return Err("A task status can't be empty; nothing was saved.".into());
    }
    let expected = expected.filter(|h| !h.is_empty());
    let mut decided: Option<(String, Option<String>)> = None;
    let note = vault::mutate_note(vault, rel, |content| {
        refuse_legacy_tracker_writer(content)?;
        if let Some(expected) = expected {
            if content_hash(content) != expected {
                return Err(conflict_error(rel));
            }
        }
        let (fm_map, _) = split_frontmatter(content);
        let title = fm_map.get("title").cloned().unwrap_or_default();
        if title.is_empty() {
            return Err(format!("task file has no title: {rel}"));
        }
        let mut old = frontmatter_from_map(&fm_map, content);
        old.title = title;
        let mut new = old.clone();
        let next = if is_terminal_status(requested) {
            old.recurrence
                .as_deref()
                .and_then(|recurrence| crate::quick_add::next_occurrence(recurrence, today))
        } else {
            None
        };
        match next {
            Some(next) => {
                let next = next.to_string();
                new.due = Some(next.clone());
                new.status = "Not started".to_string();
                decided = Some((new.status.clone(), Some(next)));
            }
            None => {
                new.status = requested.to_string();
                decided = Some((new.status.clone(), None));
            }
        }
        new.last_synced_at = Some(Utc::now().to_rfc3339());
        finish(content, patch_task(content, &old, &new, Vec::new(), None)?)
    })?;
    let (status, rolled_to) =
        decided.ok_or_else(|| format!("status of {rel} was not decided; nothing reported"))?;
    Ok(StatusOutcome {
        content_hash: content_hash(&note.content),
        status,
        rolled_to,
    })
}

// ── Claims (S09) ─────────────────────────────────────────────────────────

/// Error-message prefix for a claim refused because another run holds the
/// task or its status isn't open, and for a release by a run that doesn't
/// hold the claim.
pub const TASK_CLAIMED_ERROR: &str = "TASK_CLAIMED";
/// Frontmatter key naming the run that claimed a task (`chat:<session key>`
/// or `workflow:<slug>`). Not a modelled field, so every other writer's line
/// patch keeps it, and Notion sync maps no such field; but a status change
/// that ends the claim ([`status_change_ends_claim`]) drops it, in
/// [`patch_task`] and in Notion sync's pull alike.
pub const CLAIMED_BY_KEY: &str = "claimed_by";
/// RFC 3339 time of the claim, written beside [`CLAIMED_BY_KEY`].
pub const CLAIMED_AT_KEY: &str = "claimed_at";
/// The status a claim writes.
pub const CLAIMED_STATUS: &str = "In progress";
/// The status a release writes back over an in-progress status.
pub const RELEASED_STATUS: &str = "Not started";

/// True for the in-progress vocabulary (the `In progress` aliases in
/// `src/taskEdit.ts`), case-insensitive.
pub fn is_in_progress_status(status: &str) -> bool {
    matches!(
        status.trim().to_lowercase().as_str(),
        "in progress" | "in-progress" | "doing" | "started" | "active" | "wip" | "working"
    )
}

/// True when a task with this status is open to a claim: not finished,
/// cancelled, on hold or already in progress. An empty status reads as
/// `Not started`.
pub fn is_claimable_status(status: &str) -> bool {
    !is_terminal_status(status)
        && !is_cancelled_status(status)
        && !is_hold_status(status)
        && !is_in_progress_status(status)
}

/// The `(claimed_by, claimed_at)` a task file's text records, or `None` when
/// nobody holds it.
pub fn claim_of(content: &str) -> Option<(String, Option<String>)> {
    let (fm, _) = split_frontmatter(content);
    let value = |key: &str| {
        fm.get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    Some((value(CLAIMED_BY_KEY)?, value(CLAIMED_AT_KEY)))
}

/// [`claim_of`] for a task whose status is `status`, counting only a live
/// claim: local tasks require in-progress status (S24); tracker ownership
/// is local on any nonclosed active issue. Every reader uses this: [`claim_task`],
/// [`release_task`] and the `get_task` tool.
pub fn live_claim_of(content: &str, status: &str) -> Option<(String, Option<String>)> {
    claim_of(content).filter(|_| match crate::tracker_sync::seams::task_meta(content) {
        Some(meta) => {
            meta.status_type.is_some_and(|kind| !kind.is_closed())
                && crate::frontmatter::parse(content).get("tracker_state") == Some("active")
        }
        None => is_in_progress_status(status),
    })
}

/// True when a status write from `old` to `new` ends the task's claim, so
/// `claimed_by` / `claimed_at` are dropped: the status changes, and it isn't
/// in progress both before and after. Leaving in progress (finish, roll
/// forward, reopen) ends a live claim; entering it from another status finds
/// only a stale key, which must not come back to life as a claim. The claim
/// write itself sets the keys explicitly and keeps them. Shared by
/// [`patch_task`] and Notion sync's pull (`notion_sync::local_text`).
pub(crate) fn status_change_ends_claim(old: &str, new: &str) -> bool {
    old != new && (!is_in_progress_status(old) || !is_in_progress_status(new))
}

/// What [`claim_task`] or [`release_task`] left on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimOutcome {
    /// sha256 of the file as it now is (the new `Task.contentHash`).
    pub content_hash: String,
    pub status: String,
    /// The claim's owner (`None` after a release).
    pub owner: Option<String>,
    pub claimed_at: Option<String>,
    /// The claim's attempt id ([`ATTEMPTS_KEY`] at claim time, OC5): a
    /// completion or failure report must carry it. `None` after a release
    /// and for a claim made before attempt ids existed.
    pub attempt: Option<u32>,
    /// The caller already held the claim; the file kept its bytes.
    pub already: bool,
}

/// A claim step's decision over a task's current text: the frontmatter and
/// extra key edits to write (`None` keeps the file as it is), and the
/// outcome to report.
struct ClaimStep {
    write: Option<(TaskFrontmatter, Vec<(&'static str, KeyEdit)>)>,
    outcome: ClaimOutcome,
}

fn since(at: Option<&str>) -> String {
    at.map(|a| format!(" since {a}")).unwrap_or_default()
}

/// The shared compare-and-swap behind [`claim_task`] and [`release_task`]:
/// the baseline check, `step` and the write run in ONE `vault::mutate_note`
/// critical section, so of two claims made from the same read exactly one
/// lands and the other is `TASK_CONFLICT: …`.
fn claim_write(
    vault: &Path,
    rel: &str,
    expected: Option<&str>,
    step: impl FnOnce(&str, &TaskFrontmatter) -> Result<ClaimStep, String>,
) -> Result<ClaimOutcome, String> {
    let expected = expected.map(str::trim).filter(|h| !h.is_empty());
    let mut decided: Option<ClaimOutcome> = None;
    let note = vault::mutate_note(vault, rel, |content| {
        if let Some(expected) = expected {
            if content_hash(content) != expected {
                return Err(conflict_error(rel));
            }
        }
        let (fm_map, _) = split_frontmatter(content);
        let title = fm_map.get("title").cloned().unwrap_or_default();
        if title.is_empty() {
            return Err(format!("task file has no title: {rel}"));
        }
        let mut old = frontmatter_from_map(&fm_map, content);
        old.title = title;
        let ClaimStep { write, outcome } = step(content, &old)?;
        decided = Some(outcome);
        match write {
            None => Ok(content.to_string()),
            Some((mut new, extra)) => {
                if crate::tracker_sync::seams::task_meta(content).is_some() {
                    // Claims are local ownership only. Never use the legacy
                    // task renderer for a provider's fields or body blocks.
                    return crate::frontmatter::patch(content, &extra, None);
                }
                new.last_synced_at = Some(Utc::now().to_rfc3339());
                patch_task(content, &old, &new, extra, None)
            }
        }
    })?;
    let mut outcome = decided.ok_or_else(|| format!("the claim on {rel} was not decided"))?;
    outcome.content_hash = content_hash(&note.content);
    Ok(outcome)
}

/// Claim the task at `rel` for `owner` (S09, the `claim_task` tool). With a
/// non-empty `expected` the write is compare-and-swap: a file that changed
/// since that read (another claim won) is `TASK_CONFLICT: …`. A task another
/// owner holds is `TASK_CLAIMED: …` naming them, as is one whose status isn't
/// open ([`is_claimable_status`]); otherwise the status becomes
/// [`CLAIMED_STATUS`] and `claimed_by` / `claimed_at` are written. The
/// owner's own repeat claim reports `already` and changes nothing. Nothing
/// is written on a refusal.
pub fn claim_task(
    vault: &Path,
    rel: &str,
    owner: &str,
    now: DateTime<Local>,
    expected: Option<&str>,
) -> Result<ClaimOutcome, String> {
    let owner = owner.trim();
    if owner.is_empty() || owner.contains(['\n', '\r']) {
        return Err("A claim needs the claiming run's identity; nothing was saved.".into());
    }
    let at = now.to_rfc3339();
    claim_write(vault, rel, expected, |content, old| {
        // Local claims require in-progress status; tracker claims use the
        // provider's active state kind without changing its status.
        if let Some((holder, held_since)) = live_claim_of(content, &old.status) {
            if holder != owner {
                return Err(format!(
                    "{TASK_CLAIMED_ERROR}: {rel} is already claimed by {holder}{}. Nothing was saved; pick another task (next_claimable), or ask its owner to release it.",
                    since(held_since.as_deref())
                ));
            }
            return Ok(ClaimStep {
                write: None,
                outcome: ClaimOutcome {
                    content_hash: String::new(),
                    status: old.status.clone(),
                    owner: Some(holder),
                    claimed_at: held_since,
                    attempt: attempts_of(content),
                    already: true,
                },
            });
        }
        let tracked = crate::tracker_sync::seams::task_meta(content);
        // OC5: a broken circuit refuses with its reason before the status
        // check, so the refusal says why the row stopped.
        if let Some(reason) = live_circuit_of(content, &old.status) {
            return Err(format!(
                "{TASK_CIRCUIT_BROKEN_ERROR}: {rel} stopped retrying: {reason}. Nothing was saved; a person reopens it (status Not started) to try again."
            ));
        }
        let open = match &tracked {
            Some(meta) => {
                meta.status_type.is_some_and(|kind| !kind.is_closed())
                    && crate::frontmatter::parse(content).get("tracker_state") == Some("active")
            }
            None => is_claimable_status(&old.status),
        };
        if !open {
            return Err(format!(
                "{TASK_CLAIMED_ERROR}: {rel} is \"{}\", not open, so it can't be claimed. Nothing was saved.",
                old.status
            ));
        }
        let mut new = old.clone();
        if tracked.is_none() {
            new.status = CLAIMED_STATUS.to_string();
        }
        let status = new.status.clone();
        // An unreadable count would restart at 1 and could reuse an old id.
        if fm_value(content, ATTEMPTS_KEY).is_some() && attempts_of(content).is_none() {
            return Err(format!(
                "{TASK_CLAIMED_ERROR}: {rel} has an unreadable {ATTEMPTS_KEY}, so a new attempt id can't be issued. Nothing was saved; set it to the last attempt number (or remove it if the task was never claimed)."
            ));
        }
        let attempt = attempts_of(content).unwrap_or(0).saturating_add(1);
        let mut extra = vec![
            (CLAIMED_BY_KEY, KeyEdit::Quoted(owner.to_string())),
            (CLAIMED_AT_KEY, KeyEdit::Quoted(at.clone())),
            (ATTEMPTS_KEY, KeyEdit::Set(attempt.to_string())),
        ];
        // A breaker key on an open task is stale: a person reopened it, which
        // resets the breaker for a fresh run of attempts. Failures below the
        // limit carry over from claim to claim.
        if circuit_of(content).is_some() {
            extra.extend(breaker_removals());
        }
        Ok(ClaimStep {
            write: Some((new, extra)),
            outcome: ClaimOutcome {
                content_hash: String::new(),
                status,
                owner: Some(owner.to_string()),
                claimed_at: Some(at.clone()),
                attempt: Some(attempt),
                already: false,
            },
        })
    })
}

/// Release `owner`'s claim on the task at `rel` (the `release_task` tool):
/// `claimed_by` / `claimed_at` are removed and the in-progress status goes
/// back to [`RELEASED_STATUS`]. An unclaimed task, or one another owner
/// holds, is `TASK_CLAIMED: …` and nothing is written; a stale key on a task
/// that isn't in progress counts as no claim ([`live_claim_of`]). `expected`
/// is compare-and-swap as in [`claim_task`].
pub fn release_task(
    vault: &Path,
    rel: &str,
    owner: &str,
    expected: Option<&str>,
) -> Result<ClaimOutcome, String> {
    let owner = owner.trim();
    claim_write(vault, rel, expected, |content, old| {
        let Some((holder, held_since)) = live_claim_of(content, &old.status) else {
            return Err(format!(
                "{TASK_CLAIMED_ERROR}: {rel} isn't claimed, so there is nothing to release. Nothing was saved."
            ));
        };
        if holder != owner {
            return Err(format!(
                "{TASK_CLAIMED_ERROR}: {rel} is claimed by {holder}{}, not by this run; only its owner can release it. Nothing was saved.",
                since(held_since.as_deref())
            ));
        }
        // Only legacy local claims change status when released.
        let mut new = old.clone();
        if crate::tracker_sync::seams::task_meta(content).is_none() {
            new.status = RELEASED_STATUS.to_string();
        }
        let status = new.status.clone();
        let extra = vec![
            (CLAIMED_BY_KEY, KeyEdit::Remove),
            (CLAIMED_AT_KEY, KeyEdit::Remove),
        ];
        Ok(ClaimStep {
            write: Some((new, extra)),
            outcome: ClaimOutcome {
                content_hash: String::new(),
                status,
                owner: None,
                claimed_at: None,
                attempt: None,
                already: false,
            },
        })
    })
}

// ── Attempts and the circuit breaker (OC5) ───────────────────────────────

/// Error-message prefix for a completion or failure report whose attempt id
/// isn't the task's live claim: a stale retry from a replaced run, another
/// run, or a claim that already ended.
#[cfg_attr(not(test), allow(dead_code))] // OC5 core; the hollow_tasks tools (lane M) call it next
pub const TASK_STALE_ATTEMPT_ERROR: &str = "TASK_STALE_ATTEMPT";
/// Error-message prefix for a claim refused because the task's circuit broke.
pub const TASK_CIRCUIT_BROKEN_ERROR: &str = "TASK_CIRCUIT_BROKEN";
/// Frontmatter key counting every claim the task has had. The live claim's
/// attempt id is this number; it outlives the claim, so a later claim always
/// gets a new id and an old one can't close it.
pub const ATTEMPTS_KEY: &str = "claim_attempts";
/// Frontmatter key counting failed attempts since the breaker last reset.
pub const FAILED_ATTEMPTS_KEY: &str = "failed_attempts";
/// Frontmatter key with the last failed attempt's reason, one line.
pub const LAST_FAILURE_KEY: &str = "last_failure";
/// Frontmatter key set when the breaker trips: why the task stopped retrying.
pub const CIRCUIT_BROKEN_KEY: &str = "circuit_broken";
/// The status the breaker left the task in; the circuit is live only while
/// the task still has it, so any status change (a local reopen, or the
/// provider moving a tracker issue) makes the breaker stale.
pub const CIRCUIT_STATUS_KEY: &str = "circuit_status";
/// Failed attempts after which a task stops retrying (Orca's number; Hollow
/// has no data of its own on it yet).
#[cfg_attr(not(test), allow(dead_code))] // OC5 core; the hollow_tasks tools (lane M) call it next
pub const CIRCUIT_BREAK_AFTER: u32 = 3;
/// The status a broken circuit writes on a local task ([`is_hold_status`],
/// so [`next_claimable`] skips it).
pub const CIRCUIT_BROKEN_STATUS: &str = "On hold";

fn fm_value(content: &str, key: &str) -> Option<String> {
    split_frontmatter(content)
        .0
        .get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The task's claim count ([`ATTEMPTS_KEY`]), `None` when it has never been
/// claimed with attempt ids.
pub fn attempts_of(content: &str) -> Option<u32> {
    fm_value(content, ATTEMPTS_KEY).and_then(|v| v.parse().ok())
}

/// Failed attempts since the breaker last reset.
#[cfg_attr(not(test), allow(dead_code))] // OC5 core; the hollow_tasks tools (lane M) call it next
pub fn failures_of(content: &str) -> u32 {
    fm_value(content, FAILED_ATTEMPTS_KEY)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// The recorded breaker reason, live or stale.
pub fn circuit_of(content: &str) -> Option<String> {
    fm_value(content, CIRCUIT_BROKEN_KEY)
}

/// The breaker reason when the circuit is live: only while `status` is
/// still the [`CIRCUIT_STATUS_KEY`] the breaker recorded (a local task's
/// `On hold`, a tracker issue's provider status at the break), so a reopen,
/// a later hold for another reason or a provider status change never cites
/// it. A key without a recorded status counts only on a local `On hold`.
pub fn live_circuit_of(content: &str, status: &str) -> Option<String> {
    let reason = circuit_of(content)?;
    let same = |s: &str| s.trim().eq_ignore_ascii_case(status.trim());
    let live = match fm_value(content, CIRCUIT_STATUS_KEY) {
        Some(at_break) => same(&at_break),
        None => {
            crate::tracker_sync::seams::task_meta(content).is_none() && same(CIRCUIT_BROKEN_STATUS)
        }
    };
    live.then_some(reason)
}

/// Key edits that drop the breaker and its failure count.
fn breaker_removals() -> [(&'static str, KeyEdit); 4] {
    [
        (CIRCUIT_BROKEN_KEY, KeyEdit::Remove),
        (CIRCUIT_STATUS_KEY, KeyEdit::Remove),
        (FAILED_ATTEMPTS_KEY, KeyEdit::Remove),
        (LAST_FAILURE_KEY, KeyEdit::Remove),
    ]
}

/// Refuse unless `owner` holds the live claim on `content` with attempt id
/// `attempt`.
#[cfg_attr(not(test), allow(dead_code))] // OC5 core; the hollow_tasks tools (lane M) call it next
fn check_attempt(
    rel: &str,
    content: &str,
    status: &str,
    owner: &str,
    attempt: u32,
) -> Result<(), String> {
    let live = attempts_of(content);
    match live_claim_of(content, status) {
        Some((holder, _)) if holder == owner.trim() && live == Some(attempt) => Ok(()),
        Some((holder, _)) => Err(format!(
            "{TASK_STALE_ATTEMPT_ERROR}: {rel} is held by {holder} on attempt {}, not by {} on attempt {attempt}. Nothing was saved; this report is from a replaced or other run.",
            live.map(|n| n.to_string()).unwrap_or_else(|| "(none)".into()),
            owner.trim()
        )),
        None => Err(format!(
            "{TASK_STALE_ATTEMPT_ERROR}: {rel} has no live claim, so attempt {attempt} has already ended. Nothing was saved."
        )),
    }
}

/// Finish (or reopen) a claimed task as the claim's own attempt (OC5, the
/// completion a lane or worker sends when it is done): [`set_status_finishing`]
/// with a step that checks, inside the same critical section, that `owner`
/// holds the live claim and that `attempt` is its id ([`ATTEMPTS_KEY`]). A
/// stale retry is `TASK_STALE_ATTEMPT: …` and writes nothing, so it can't
/// close a newer claim.
#[cfg_attr(not(test), allow(dead_code))] // OC5 core; the hollow_tasks tools (lane M) call it next
pub fn complete_claim(
    vault: &Path,
    rel: &str,
    owner: &str,
    attempt: u32,
    status: &str,
    today: NaiveDate,
    expected: Option<&str>,
) -> Result<StatusOutcome, String> {
    set_status_finishing(vault, rel, status, today, expected, &|old, text| {
        let (fm, _) = split_frontmatter(old);
        let current = fm
            .get("status")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        check_attempt(rel, old, &current, owner, attempt)?;
        // A success ends the failure count: nothing carries past it, a
        // recurring task's next occurrence included.
        if is_terminal_status(status) {
            return crate::frontmatter::patch(&text, &breaker_removals(), None);
        }
        Ok(text)
    })
}

/// What [`fail_claim`] left on disk.
#[cfg_attr(not(test), allow(dead_code))] // OC5 core; the hollow_tasks tools (lane M) call it next
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailOutcome {
    pub content_hash: String,
    pub status: String,
    /// Failed attempts since the breaker last reset, this one included.
    pub failures: u32,
    /// Why the task stopped retrying, when this failure broke the circuit.
    pub circuit_broken: Option<String>,
}

/// Report that attempt `attempt` of `owner`'s claim failed (OC5). The
/// attempt is checked as in [`complete_claim`]; then the claim ends,
/// [`FAILED_ATTEMPTS_KEY`] counts up and [`LAST_FAILURE_KEY`] keeps `reason`.
/// Below [`CIRCUIT_BREAK_AFTER`] the task goes back to [`RELEASED_STATUS`]
/// for a retry; at it the circuit breaks: [`CIRCUIT_BROKEN_KEY`] says why and
/// a local task goes [`CIRCUIT_BROKEN_STATUS`], so it is no longer offered
/// and a claim is refused with that reason until a person reopens it.
#[cfg_attr(not(test), allow(dead_code))] // OC5 core; the hollow_tasks tools (lane M) call it next
pub fn fail_claim(
    vault: &Path,
    rel: &str,
    owner: &str,
    attempt: u32,
    reason: &str,
    expected: Option<&str>,
) -> Result<FailOutcome, String> {
    let reason = one_line(reason, 300);
    let reason = if reason.is_empty() {
        "no reason given".to_string()
    } else {
        reason
    };
    let mut report: Option<(u32, Option<String>)> = None;
    let outcome = claim_write(vault, rel, expected, |content, old| {
        check_attempt(rel, content, &old.status, owner, attempt)?;
        let failures = failures_of(content).saturating_add(1);
        let tracked = crate::tracker_sync::seams::task_meta(content).is_some();
        let broken = (failures >= CIRCUIT_BREAK_AFTER)
            .then(|| format!("{failures} failed attempts; the last: {reason}"));
        let mut new = old.clone();
        if !tracked {
            new.status = if broken.is_some() {
                CIRCUIT_BROKEN_STATUS.to_string()
            } else {
                RELEASED_STATUS.to_string()
            };
        }
        let mut extra = vec![
            (CLAIMED_BY_KEY, KeyEdit::Remove),
            (CLAIMED_AT_KEY, KeyEdit::Remove),
            (FAILED_ATTEMPTS_KEY, KeyEdit::Set(failures.to_string())),
            (LAST_FAILURE_KEY, KeyEdit::Quoted(reason.clone())),
        ];
        if let Some(why) = &broken {
            extra.push((CIRCUIT_BROKEN_KEY, KeyEdit::Quoted(why.clone())));
            extra.push((CIRCUIT_STATUS_KEY, KeyEdit::Quoted(new.status.clone())));
        }
        report = Some((failures, broken));
        Ok(ClaimStep {
            outcome: ClaimOutcome {
                content_hash: String::new(),
                status: new.status.clone(),
                owner: None,
                claimed_at: None,
                attempt: None,
                already: false,
            },
            write: Some((new, extra)),
        })
    })?;
    let (failures, circuit_broken) =
        report.ok_or_else(|| format!("the failure on {rel} was not decided"))?;
    Ok(FailOutcome {
        content_hash: outcome.content_hash,
        status: outcome.status,
        failures,
        circuit_broken,
    })
}

/// The task to claim next among `tasks` (from [`list`], which fills `ready`;
/// run [`mark_task_locks`] first so a status the tracker owns is skipped): ready
/// (every blocker finished) and an open status ([`is_claimable_status`], so
/// never a live claim). Highest priority first (unprioritized counts as P4),
/// then the oldest `created` (a task without one sorts after), then the ref.
pub fn next_claimable(vault: &Path, tasks: Vec<Task>) -> Option<Task> {
    tasks
        .into_iter()
        .filter(|t| {
            t.ready
                && match &t.tracker {
                    Some(meta) => {
                        meta.lock.kind == crate::tracker::ipc::TrackerLockKind::Linked
                            && meta.status_type.is_some_and(|kind| !kind.is_closed())
                    }
                    None => {
                        is_claimable_status(&t.status) && !held_modal_fields(t).contains("status")
                    }
                }
        })
        .filter_map(|t| {
            let rel = t.url.strip_prefix("vault:")?;
            // A tracker claim need not change the provider's open status.
            let text = fs::read_to_string(vault.join(rel)).ok()?;
            if live_claim_of(&text, &t.status).is_some() {
                return None;
            }
            // OC5: a broken circuit is not offered (a tracker issue's open
            // status alone would let it through).
            if live_circuit_of(&text, &t.status).is_some() {
                return None;
            }
            let created = split_frontmatter(&text)
                .0
                .get("created")
                .map(|c| c.trim().to_string())
                .unwrap_or_default();
            let rank = t.priority.clone().unwrap_or_else(|| "P4".into());
            Some((rank, (created.is_empty(), created), t))
        })
        .min_by(|a, b| (&a.0, &a.1, &a.2.url).cmp(&(&b.0, &b.1, &b.2.url)))
        .map(|(_, _, t)| t)
}

/// Apply a full-edit [`UpdateTaskInput`] to a task file and return the
/// sha256 of the bytes written. `None` fields are left unchanged; an empty
/// string clears an optional scalar; `labels` / `subtasks` replace when
/// present.
///
/// - **Line patch, not re-render.** Only the modelled keys whose value
///   changed are rewritten ([`patch_task`]); `previous_due`, the legacy
///   Notion import keys (`description`, `notion_project_ids`, `due_end`,
///   `due_time_zone`),
///   list-valued and custom keys survive byte-for-byte. Moving `due` drops
///   the stale `due_end` / `due_time_zone` range.
/// - **Compare-and-swap.** When `input.expected` is a non-empty hash, it is
///   compared with the file under `vault::MUTATE_LOCK`; a mismatch returns
///   `TASK_CONFLICT: …` and writes nothing.
/// - **Description routing.** A description equal (trimmed) to what
///   [`parse`] shows is unchanged. Otherwise, when the file has a
///   `description` key (an imported summary; present even when empty), the
///   edit writes that key; without the key it replaces the body prose.
/// - **Body bytes.** A steps-only change splices the existing step lines in
///   place ([`splice_subtasks`]): every other line, reserved sections and
///   fenced blocks included, keeps its bytes. Only a body-routed description
///   replacement recomposes the canonical layout ([`compose_task_body`]),
///   re-emitting `## Done when` and `## Activity` verbatim after the steps.
///   Nothing is rewritten when neither changes.
/// - **Blockers.** A `blocked_by` list replaces the stored one when it
///   differs. Only newly added entries are validated ([`normalize_blocked_by`]:
///   `TASK_BLOCKER: …` for a link that names no task, `TASK_CYCLE: …` for the
///   task itself), then [`check_blocker_cycle`] refuses a loop. Either
///   refusal writes nothing. Stored entries that no longer resolve are kept.
pub fn apply_update(vault: &Path, rel: &str, input: &UpdateTaskInput) -> Result<String, String> {
    apply_update_finishing(vault, rel, input, &|_, text| Ok(text))
}

/// [`apply_update`] with a [`Finish`] step.
pub(crate) fn apply_update_finishing(
    vault: &Path,
    rel: &str,
    input: &UpdateTaskInput,
    finish: Finish,
) -> Result<String, String> {
    let expected = input.expected.as_deref().filter(|h| !h.is_empty());
    let note = vault::mutate_note(vault, rel, |content| {
        refuse_legacy_tracker_writer(content)?;
        if let Some(expected) = expected {
            if content_hash(content) != expected {
                return Err(conflict_error(rel));
            }
        }
        let (fm_map, body) = split_frontmatter(content);
        let existing_title = fm_map.get("title").cloned().unwrap_or_default();
        if existing_title.is_empty() {
            return Err(format!("task file has no title: {rel}"));
        }
        let mut old = frontmatter_from_map(&fm_map, content);
        old.title = existing_title;
        let mut new = old.clone();

        // Scalar setter: Some("") clears, Some(v) sets, None leaves unchanged.
        fn set_opt(cur: &mut Option<String>, v: &Option<String>) {
            if let Some(s) = v {
                let t = s.trim();
                *cur = if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                };
            }
        }
        if let Some(t) = &input.title {
            if !t.trim().is_empty() {
                new.title = t.trim().to_string();
            }
        }
        if let Some(s) = &input.status {
            if !s.trim().is_empty() {
                new.status = s.trim().to_string();
            }
        }
        if let Some(p) = &input.priority {
            new.priority = normalize_priority(p);
        }
        set_opt(&mut new.due, &input.due);
        set_opt(&mut new.due_time, &input.due_time);
        set_opt(&mut new.recurrence, &input.recurrence);
        set_opt(&mut new.section, &input.section);
        set_opt(&mut new.project, &input.project);
        if new.project != old.project {
            // A re-filed task's stored `project_url` still points at the old
            // project, and nothing here can resolve the new name to one — so
            // drop it rather than leave the task attached to a project it no
            // longer names.
            new.project_url = None;
        }
        if let Some(d) = input.duration_min {
            new.duration_min = if d == 0 { None } else { Some(d) };
        }
        if let Some(labels) = &input.labels {
            // Trim, drop empties, de-dup (case-insensitive) via the same parser
            // used on read, so the stored scalar stays clean.
            new.labels = parse_labels(&labels.join(","));
        }
        if let Some(requested) = &input.blocked_by {
            // Every task, read inside the mutate lock (list_parsed takes no
            // lock), so the cycle check sees the graph this write joins.
            let tasks = list_parsed(vault);
            let self_url = format!("vault:{rel}");
            let normalized =
                normalize_blocked_by(&tasks, Some(self_url.as_str()), &old.blocked_by, requested)?;
            if normalized != old.blocked_by {
                let added: Vec<String> = normalized
                    .iter()
                    .filter(|url| !old.blocked_by.iter().any(|o| o.eq_ignore_ascii_case(url)))
                    .cloned()
                    .collect();
                check_blocker_cycle(&tasks, &self_url, &new.title, &normalized, &added)?;
                new.blocked_by = normalized;
            }
        }

        let pb = parse_body(&body);
        let mut extra: Vec<(&'static str, KeyEdit)> = Vec::new();
        let mut prose_override: Option<String> = None;
        if let Some(d) = &input.description {
            // The same rule `parse` uses to show a description.
            let shown = fm_map
                .get("description")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| pb.prose.clone());
            if d.trim() != shown.trim() {
                if fm_map.contains_key("description") {
                    extra.push(("description", KeyEdit::Set(d.trim().to_string())));
                } else {
                    prose_override = Some(d.trim().to_string());
                }
            }
        }
        let requested_steps = input.subtasks.as_deref().map(normalize_steps);
        let body_edit = match (&prose_override, requested_steps) {
            // The prose itself was replaced: interleaved prose can't be mapped
            // back, so the body takes the canonical layout (reserved sections
            // re-emitted verbatim after the steps).
            (Some(prose), steps) => {
                let steps = steps.unwrap_or_else(|| pb.subtasks.clone());
                let composed = compose_task_body(prose, &steps, &pb.sections);
                if composed.trim() == body.trim() {
                    BodyEdit::Keep
                } else {
                    BodyEdit::Canonical(composed)
                }
            }
            (None, Some(steps)) if steps != pb.subtasks => {
                if pb.is_blank() {
                    BodyEdit::Canonical(compose_body("", &steps))
                } else {
                    BodyEdit::Exact(splice_subtasks(&pb, &steps))
                }
            }
            _ => BodyEdit::Keep,
        };

        new.last_synced_at = Some(Utc::now().to_rfc3339());
        let text = match body_edit {
            BodyEdit::Keep => patch_task(content, &old, &new, extra, None),
            BodyEdit::Canonical(b) => patch_task(content, &old, &new, extra, Some(&b)),
            BodyEdit::Exact(b) => {
                // `patch(…, None)` keeps everything after the frontmatter
                // block byte for byte, and the parsed body is a suffix of the
                // file, so replacing that suffix changes only the body.
                let patched = patch_task(content, &old, &new, extra, None)?;
                match patched.strip_suffix(body.as_str()) {
                    Some(head) => Ok(format!("{head}{b}")),
                    None => Err(format!(
                        "task body of {rel} could not be located after the frontmatter; file preserved"
                    )),
                }
            }
        }?;
        finish(content, text)
    })?;
    Ok(content_hash(&note.content))
}

/// How [`apply_update`] rewrites a task body.
enum BodyEdit {
    /// Keep the body bytes.
    Keep,
    /// Replace the body with this canonical text (written in render shape).
    Canonical(String),
    /// Replace the body with these exact bytes (a step splice).
    Exact(String),
}

// ── Activity lines (Paperclip PR 12's add_comment) ───────────────────────

/// Longest activity entry body, in characters.
pub(crate) const ACTIVITY_MAX_CHARS: usize = 4_000;
/// Most lines in one activity entry body.
pub(crate) const ACTIVITY_MAX_LINES: usize = 40;
/// Longest author label on an activity line.
const ACTIVITY_AUTHOR_MAX_CHARS: usize = 60;

/// The hidden marker text for an activity entry keyed by `key`: the first 16
/// hex of sha256(surface ‖ 0 ‖ scope ‖ 0 ‖ trimmed key), or `None` for a
/// blank key. A retried `add_comment` with the same key finds it and writes
/// nothing.
pub(crate) fn activity_marker(surface: &str, scope: &str, key: &str) -> Option<String> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let mut hasher = Sha256::new();
    for part in [surface.trim(), scope.trim(), key] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    Some(format!("{:x}", hasher.finalize())[..16].to_string())
}

/// The HTML comment that carries an activity marker at the end of its entry
/// (the same hidden end-of-line style as the workflow run log's run ids).
fn activity_marker_comment(marker: &str) -> String {
    format!("<!-- hollow-activity: {marker} -->")
}

/// Escape one line of an activity entry so it can't read as structure: a
/// leading checklist item (`- [`, `* [`, `+ [`) gets `\[`, and a leading
/// `#` becomes `\#` (an `## ` line would otherwise end the section).
fn escape_activity_line(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    for bullet in ["- [", "* [", "+ ["] {
        if let Some(rest) = trimmed.strip_prefix(bullet) {
            return format!("{indent}{}\\[{rest}", &bullet[..2]);
        }
    }
    if trimmed.starts_with('#') {
        return format!("{indent}\\{trimmed}");
    }
    line.to_string()
}

/// One `## Activity` entry: `- {YYYY-MM-DD HH:MM} {author}: {first line}`,
/// continuation lines indented four spaces, carriage returns stripped, each
/// line escaped ([`escape_activity_line`]), and an optional trailing hidden
/// marker comment. An empty body, one over [`ACTIVITY_MAX_CHARS`] characters
/// or [`ACTIVITY_MAX_LINES`] lines, or a marker that is not lowercase hex is
/// refused.
pub(crate) fn activity_line(
    author: &str,
    body: &str,
    at: DateTime<Local>,
    marker: Option<&str>,
) -> Result<String, String> {
    let cleaned = body.replace('\r', "");
    let text = cleaned.trim();
    if text.is_empty() {
        return Err("An activity entry needs some text; nothing was written.".into());
    }
    let chars = text.chars().count();
    if chars > ACTIVITY_MAX_CHARS {
        return Err(format!(
            "An activity entry is at most {ACTIVITY_MAX_CHARS} characters (this one is {chars}); nothing was written."
        ));
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() > ACTIVITY_MAX_LINES {
        return Err(format!(
            "An activity entry is at most {ACTIVITY_MAX_LINES} lines (this one is {}); nothing was written.",
            lines.len()
        ));
    }
    if let Some(marker) = marker {
        if marker.is_empty()
            || !marker
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("activity marker must be lowercase hex".into());
        }
    }
    let author = Some(one_line(author, ACTIVITY_AUTHOR_MAX_CHARS))
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| "agent".into());
    let mut out = format!(
        "- {} {author}: {}",
        at.format("%Y-%m-%d %H:%M"),
        escape_activity_line(lines[0].trim())
    );
    for line in &lines[1..] {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str("    ");
            out.push_str(&escape_activity_line(line.trim_end()));
        }
    }
    if let Some(marker) = marker {
        out.push(' ');
        out.push_str(&activity_marker_comment(marker));
    }
    Ok(out)
}

/// What [`append_activity`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityOutcome {
    /// sha256 of the file after the call (unchanged on a duplicate).
    pub content_hash: String,
    /// True when an entry with the same marker was already present, so
    /// nothing was written.
    pub duplicate: bool,
}

/// Append one entry ([`activity_line`]) to the task's `## Activity` section
/// through [`append_to_section`], which splices it in place (creating the
/// section after the last reserved section when there is none). The
/// frontmatter, prose, steps, `## Done when` and every other byte are kept;
/// the file is replaced only by that splice. Runs under `vault::MUTATE_LOCK`:
/// with a `marker` already present in an `## Activity` section nothing is
/// written (`duplicate`), whatever `expected` says; otherwise a non-empty
/// `expected` makes the write compare-and-swap (`TASK_CONFLICT: …` on a
/// mismatch).
pub fn append_activity(
    vault: &Path,
    rel: &str,
    author: &str,
    body: &str,
    at: DateTime<Local>,
    expected: Option<&str>,
    marker: Option<&str>,
) -> Result<ActivityOutcome, String> {
    let entry = activity_line(author, body, at, marker)?;
    let expected = expected.filter(|h| !h.is_empty());
    let mut already: Option<String> = None;
    let written = vault::mutate_note(vault, rel, |content| {
        let (fm_map, body) = split_frontmatter(content);
        let tracked = crate::tracker_sync::seams::task_meta(content).is_some();
        let activity_body = if tracked {
            crate::tracker::mirror::vault_notes(content)?
        } else {
            body.clone()
        };
        if fm_map.get("title").is_none_or(|t| t.is_empty()) {
            return Err(format!("task file has no title: {rel}"));
        }
        // The marker is checked BEFORE the baseline: a retry whose first
        // attempt landed (its response lost) carries the pre-write hash,
        // which is stale by construction, and must still read as the
        // duplicate it is rather than a conflict.
        if let Some(marker) = marker {
            let tag = activity_marker_comment(marker);
            let seen = parse_body(&activity_body)
                .sections
                .iter()
                .any(|s| s.kind == ReservedKind::Activity && s.content.contains(&tag));
            if seen {
                // Abort the write; the outcome is reported below.
                already = Some(content_hash(content));
                return Err(String::new());
            }
        }
        if let Some(expected) = expected {
            if content_hash(content) != expected {
                return Err(conflict_error(rel));
            }
        }
        let new_body = if tracked {
            // The writable tail starts after the final owned block. Never
            // treat a provider comment's headings as local activity.
            let start = [
                "<!-- /tracker:description -->",
                "<!-- /tracker:comments -->",
                "<!-- /tracker:conflicts -->",
            ]
            .iter()
            .filter_map(|marker| body.rfind(marker).map(|at| at + marker.len()))
            .max()
            .unwrap_or(0);
            format!(
                "{}{}",
                &body[..start],
                append_to_section(&body[start..], ReservedKind::Activity, &entry)
            )
        } else {
            append_to_section(&body, ReservedKind::Activity, &entry)
        };
        match content.strip_suffix(body.as_str()) {
            // A file that ends right at its closing `---` has an empty body
            // and no newline after the fence; without one the section
            // heading would join the fence (`---## Activity`) and Obsidian
            // would lose the whole frontmatter.
            Some(head) if !head.is_empty() && !head.ends_with('\n') => {
                let newline = if head.contains("\r\n") { "\r\n" } else { "\n" };
                Ok(format!("{head}{newline}{new_body}"))
            }
            Some(head) => Ok(format!("{head}{new_body}")),
            None => Err(format!(
                "task body of {rel} could not be located after the frontmatter; file preserved"
            )),
        }
    });
    match (written, already) {
        (_, Some(unchanged)) => Ok(ActivityOutcome {
            content_hash: unchanged,
            duplicate: true,
        }),
        (Ok(note), None) => Ok(ActivityOutcome {
            content_hash: content_hash(&note.content),
            duplicate: false,
        }),
        (Err(error), None) => Err(error),
    }
}

/// The migration patch for one file, or `None` when it needs none. Only a
/// recognised, non-canonical `priority` (`High` → `P1`) is rewritten, and
/// the tags block only alongside it. Notion-linked files are skipped: their
/// priority is a Notion option name, and rewriting it would either flip back
/// on every sync or map to a different option at the same level.
fn migrate_one(content: &str) -> Option<String> {
    let (fm_map, _) = split_frontmatter(content);
    if fm_map.get("title").is_none_or(|t| t.is_empty()) {
        return None;
    }
    if fm_map.get("notion_id").is_some_and(|id| !id.is_empty())
        || crate::tracker_sync::seams::task_meta(content).is_some()
    {
        return None;
    }
    let raw = fm_map.get("priority")?;
    let normalized = normalize_priority(raw)?;
    if *raw == normalized {
        return None;
    }
    let fm = frontmatter_from_map(&fm_map, content);
    crate::frontmatter::patch(
        content,
        &[
            ("priority", KeyEdit::Set(normalized)),
            ("tags", KeyEdit::Block(tag_lines(&fm))),
        ],
        None,
    )
    .ok()
}

/// One-shot migration: rewrite `priority` to canonical `P1`..`P4` form in
/// every local task file that uses a legacy spelling (`High`/`Medium`/`Low`),
/// refreshing its tags. Returns the number of files changed. Idempotent — a
/// second run rewrites 0. Skips Notion-linked files (any `notion_id`) and
/// leaves unrecognised priorities, bodies and every other key alone. Each
/// write is compare-and-swap against the bytes read; a file that changed in
/// between is skipped.
pub fn migrate_priorities(vault: &Path) -> Result<usize, String> {
    let root = vault.join(TASKS_DIR);
    if !root.exists() {
        return Ok(0);
    }
    let mut count = 0usize;
    for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !name.ends_with(".md") || name.starts_with('.') {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(vault) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().to_string();
        let Ok(content) = fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(patched) = migrate_one(&content) else {
            continue;
        };
        match vault::write_note_cas(vault, &rel_str, &patched, &content)? {
            vault::CasOutcome::Written(_) => count += 1,
            // Edited since it was read: leave it for the next run.
            vault::CasOutcome::Conflict(_) => {}
        }
    }
    Ok(count)
}

pub fn delete(vault: &Path, rel: &str) -> Result<(), String> {
    if crate::tracker_sync::seams::tracked(vault, rel) {
        return Err("TASK_LINKED: Use Archive on the tracker issue; removing its mirror would be undone by sync.".into());
    }
    vault::delete_note(vault, rel)
}

/// Restore deleted task bytes only if the original path is still free. Undo
/// must not replace a newly created task with the same title/slug.
///
/// A restore is never refused for its blockers: Undo returns the user's
/// bytes. When re-binding the path makes dangling `blocked_by` links resolve
/// into a loop, [`list`] flags every task on it with a `cycle` warning.
pub fn restore(vault: &Path, rel: &str, content: &str) -> Result<(), String> {
    if !rel.starts_with("tasks/") {
        return Err(format!("refusing to restore outside tasks/: {rel}"));
    }
    vault::create_note(vault, rel, content).map(|_| ()).map_err(|error| {
        if error.starts_with("Note already exists:") {
            format!("Cannot undo: another task now exists at {rel}. It was preserved; the deleted original remains recoverable in the vault's .trash folder.")
        } else { error }
    })
}

/// The modelled fields of a task file; `content` supplies a YAML-list `labels:`.
fn frontmatter_from_map(m: &BTreeMap<String, String>, content: &str) -> TaskFrontmatter {
    TaskFrontmatter {
        title: m.get("title").cloned().unwrap_or_default(),
        status: m.get("status").cloned().unwrap_or_default(),
        priority: m
            .get("priority")
            .map(|s| s.as_str())
            .and_then(normalize_priority),
        due: m.get("due").cloned().filter(|s| !s.is_empty()),
        due_time: m.get("due_time").cloned().filter(|s| !s.is_empty()),
        duration_min: m
            .get("duration_min")
            .and_then(|s| s.trim().parse::<u32>().ok()),
        effort: m.get("effort").cloned().filter(|s| !s.is_empty()),
        project: m.get("project").cloned().filter(|s| !s.is_empty()),
        project_url: m.get("project_url").cloned().filter(|s| !s.is_empty()),
        section: m.get("section").cloned().filter(|s| !s.is_empty()),
        labels: labels_from(m, content),
        recurrence: m.get("recurrence").cloned().filter(|s| !s.is_empty()),
        blocked_by: blocked_by_from(m, content),
        created: m.get("created").cloned().filter(|s| !s.is_empty()),
        last_synced_at: m.get("last_synced_at").cloned().filter(|s| !s.is_empty()),
        source: m.get("source").cloned().filter(|s| !s.is_empty()),
        origin: m.get("origin").cloned().filter(|s| !s.is_empty()),
        fingerprint: m.get("fingerprint").cloned().filter(|s| !s.is_empty()),
        notion_id: m.get("notion_id").cloned().filter(|s| !s.is_empty()),
        notion_url: m.get("notion_url").cloned().filter(|s| !s.is_empty()),
    }
}

/// The modelled scalar keys in render order, with the value each would be
/// written as (`None` = absent). One table drives both [`render`] (new files)
/// and [`scalar_edits`] (existing files), so a new field is added once.
fn scalar_fields(fm: &TaskFrontmatter) -> Vec<(&'static str, Option<String>)> {
    vec![
        ("title", Some(fm.title.clone()).filter(|s| !s.is_empty())),
        ("status", Some(fm.status.clone()).filter(|s| !s.is_empty())),
        ("priority", fm.priority.clone()),
        ("due", fm.due.clone()),
        ("due_time", fm.due_time.clone()),
        ("duration_min", fm.duration_min.map(|n| n.to_string())),
        ("effort", fm.effort.clone()),
        ("project", fm.project.clone()),
        ("project_url", fm.project_url.clone()),
        ("section", fm.section.clone()),
        (
            "labels",
            (!fm.labels.is_empty()).then(|| fm.labels.join(", ")),
        ),
        ("recurrence", fm.recurrence.clone()),
        (
            "blocked_by",
            (!fm.blocked_by.is_empty()).then(|| fm.blocked_by.join(", ")),
        ),
        ("created", fm.created.clone()),
        ("source", fm.source.clone()),
        ("origin", fm.origin.clone()),
        ("fingerprint", fm.fingerprint.clone()),
        ("notion_id", fm.notion_id.clone()),
        ("notion_url", fm.notion_url.clone()),
        ("last_synced_at", fm.last_synced_at.clone()),
    ]
}

/// A tag segment: lowercased, with every whitespace or control character
/// (a space as before, and also a newline or CR) turned into `-`. The tag
/// lines are written raw, so a newline in a status or label would otherwise
/// start a new frontmatter line, even a bare `---` that ends the block early.
fn tag_slug(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect()
}

/// The derived `tags:` block. Hollow owns it and regenerates it from
/// status, priority and labels on every write.
fn tag_lines(fm: &TaskFrontmatter) -> Vec<String> {
    let mut lines = vec!["tags:".to_string(), "  - tasks".to_string()];
    let status_slug = tag_slug(&fm.status);
    if !status_slug.is_empty() {
        lines.push(format!("  - status/{status_slug}"));
    }
    if let Some(p) = fm.priority.as_deref() {
        lines.push(format!("  - priority/{}", tag_slug(p)));
    }
    for label in &fm.labels {
        let lslug = tag_slug(label);
        if !lslug.is_empty() {
            lines.push(format!("  - label/{lslug}"));
        }
    }
    lines
}

/// `Set` / `Remove` edits for exactly the modelled keys whose value differs
/// between `old` (parsed from the file) and `new`. An unchanged key keeps its
/// original line, so legacy spellings (`priority: High`), quoting and values
/// the model cannot parse survive.
fn scalar_edits(old: &TaskFrontmatter, new: &TaskFrontmatter) -> Vec<(&'static str, KeyEdit)> {
    scalar_fields(old)
        .into_iter()
        .zip(scalar_fields(new))
        .filter(|((_, before), (_, after))| before != after)
        .map(|(_, (key, after))| match after {
            Some(value) => (key, KeyEdit::Set(value)),
            None => (key, KeyEdit::Remove),
        })
        .collect()
}

/// Line-patch an EXISTING task file from `old` (what was parsed from
/// `content`) to `new`: changed modelled keys, then `extra`, then — when
/// `due` moved — removal of the stale `due_end` / `due_time_zone` range
/// (Notion sync keeps a stored range only while the start is unchanged, so
/// removing them makes the next push clear it instead of sending a stale or
/// inverted range), then the regenerated `tags:` block. Every other line is
/// kept byte-for-byte; the body is replaced only when `body` is `Some`.
/// This is the only writer for existing task files; [`render`] is for new ones.
fn patch_task(
    content: &str,
    old: &TaskFrontmatter,
    new: &TaskFrontmatter,
    extra: Vec<(&'static str, KeyEdit)>,
    body: Option<&str>,
) -> Result<String, String> {
    let mut edits = scalar_edits(old, new);
    // A claim lives only while its task is in progress (S09): any write that
    // moves the status out of it (finish, roll-forward, reopen) drops it, and
    // so does one moving a task with a stale key into it (S24).
    let ends_claim = status_change_ends_claim(&old.status, &new.status);
    for key in [CLAIMED_BY_KEY, CLAIMED_AT_KEY] {
        if ends_claim && !extra.iter().any(|(k, _)| *k == key) {
            edits.push((key, KeyEdit::Remove));
        }
    }
    // OC5: a status change drops a breaker (a person's reopen resets it), so
    // a later hold for another reason never cites the old failures.
    if old.status != new.status && circuit_of(content).is_some() {
        for (key, edit) in breaker_removals() {
            if !extra.iter().any(|(k, _)| *k == key) {
                edits.push((key, edit));
            }
        }
    }
    edits.extend(extra);
    if old.due != new.due {
        edits.push(("due_end", KeyEdit::Remove));
        edits.push(("due_time_zone", KeyEdit::Remove));
    }
    edits.push(("tags", KeyEdit::Block(tag_lines(new))));
    crate::frontmatter::patch(content, &edits, body)
}

/// Render a NEW task file. Existing files go through [`patch_task`].
pub fn render(fm: &TaskFrontmatter, body: &str) -> String {
    let mut lines = vec!["---".to_string()];
    for (key, value) in scalar_fields(fm) {
        let value = match (key, value) {
            ("last_synced_at", None) => Some(Utc::now().to_rfc3339()),
            (_, value) => value,
        };
        if let Some(value) = value {
            push_kv(&mut lines, key, &value);
        }
    }
    lines.extend(tag_lines(fm));
    lines.push("---".into());
    lines.push(String::new());
    let trimmed = body.trim();
    if !trimmed.is_empty() {
        lines.push(trimmed.to_string());
        lines.push(String::new());
    }
    lines.join("\n")
}

fn push_kv(lines: &mut Vec<String>, key: &str, value: &str) {
    crate::frontmatter::push_kv(lines, key, value);
}

/// Split a task markdown file into (frontmatter map, body). Thin wrapper
/// over the shared [`crate::frontmatter`] parser — kept as a `pub` helper
/// because the rest of this module and its tests reference it by name.
pub fn split_frontmatter(content: &str) -> (BTreeMap<String, String>, String) {
    crate::frontmatter::parse_map(content)
}

#[cfg(test)]
#[path = "vault_tasks_tests.rs"]
mod tests;
