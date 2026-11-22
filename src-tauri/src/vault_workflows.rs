//! Workflows are Claude-driven automations stored as markdown files
//! under `vault/workflows/{slug}.md`. YAML frontmatter holds the
//! schedule + run metadata; the body is the prompt template (free-form
//! markdown). A `## Run log` section at the bottom records the most
//! recent runs in human-readable form so the user can audit history
//! straight from Obsidian.
//!
//! Schedule values are either the literal string `manual` or a 7-field
//! cron expression (the format the `cron` crate consumes — seconds,
//! minutes, hours, day-of-month, month, day-of-week, year).
//!
//! **Writes to an existing file are line patches** (Paperclip PR 9, 9a-1).
//! [`render`] builds NEW files only. Every status, run-log, schedule and
//! editor write to a file that already exists goes through `patch_workflow`:
//! PR 3's `frontmatter::patch` rewrites only the modelled keys whose parsed
//! value changed, the body is spliced at `split_body`'s own run-log index,
//! and the result is verified parse-equivalent before it is written. Unknown
//! keys, YAML lists, comments, empty values, key order, quoting, CRLF, user
//! tags and the prompt bytes all survive. When a patch is refused (a
//! hand-written indented line would shadow or hide a key), status and
//! run-log writes (`WritePolicy::Bookkeeping`) fall back to the canonical
//! render and log it; owned writers (`set_schedule`, the
//! editor's save) return the error and write nothing.

use crate::frontmatter::KeyEdit;
use crate::types::{RunLogEntry, Workflow, WorkflowInput, WorkflowOutput};
use crate::vault;
use chrono::{DateTime, Utc};
use cron::Schedule;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::str::FromStr;
use walkdir::WalkDir;

pub const WORKFLOWS_DIR: &str = "workflows";
const RUN_LOG_HEADER: &str = "## Run log";
const RUN_ID_MARKER_PREFIX: &str = "<!-- hollow-run-id:";
const RUN_ID_MARKER_SUFFIX: &str = "-->";
const STDERR_MARKER_PREFIX: &str = "<!-- hollow-stderr:";
/// Hidden exit-kind marker (Paperclip PR 8b), written last on a run-log line
/// (after the run-id and stderr markers) and parsed first. Only the values in
/// [`EXIT_KINDS`] are ever written or accepted.
const EXIT_MARKER_PREFIX: &str = "<!-- hollow-exit:";
/// The exit kind of a run a usage budget ended (`RunLogEntry.exit_kind`).
pub const EXIT_KIND_BUDGET: &str = "budget";
/// Every exit kind the marker may carry; anything else is not a marker.
const EXIT_KINDS: &[&str] = &[EXIT_KIND_BUDGET];
/// Default misfire grace window (minutes). A scheduled run whose
/// `next_run_at` is further past than this fires as a skip + reschedule
/// instead of a late catch-up run. Override per workflow with the
/// `misfire_grace_min` frontmatter key.
pub const DEFAULT_MISFIRE_GRACE_MIN: u64 = 120;
/// Maximum number of run-log entries persisted in each workflow file.
/// Older entries roll off the bottom — this keeps the file readable in
/// Obsidian without losing recent context.
const RUN_LOG_KEEP: usize = 50;

/// The one run-log note for a genuine misfire: the occurrence passed its
/// grace window while Hollow was closed or the machine slept. Never used for
/// lease contention, which has its own notes below.
pub const MISSED_WHILE_CLOSED_NOTE: &str = "missed while Hollow was closed";
/// Run-log status of a run that started after waiting for the vault lease.
/// Informational: that run's own terminal row follows it.
pub const DEFERRED_STATUS: &str = "deferred";
/// Run-log status of a run (or a broken precheck) that failed.
pub const ERROR_STATUS: &str = "error";
/// Run-log status of an occurrence that did not run.
pub const SKIPPED_STATUS: &str = "skipped";
/// The reason in a past-grace `not run: <reason>` row
/// ([`refused_skip_entry`]) for an automation whose settings changed outside
/// Hollow's editor and await (or were denied) approval (Paperclip PR 5). Like
/// every run-log note: one line, no em dash (the row separator), no `<`/`>`.
pub const PERMISSION_REFUSED_REASON: &str =
    "automation settings changed; review them in Automations → Permission changes";
/// The same row when the permission records themselves could not be read.
pub const PERMISSION_RECORDS_UNAVAILABLE_REASON: &str =
    "automation permission records could not be read; repair or move them, then restart Hollow";
/// Longest holder name or reason carried into a run-log note.
const NOTE_FRAGMENT_MAX_CHARS: usize = 120;
/// Every editor-save refusal because the file changed outside the editor
/// starts with this, like PR 3's `TASK_CONFLICT` (mirrored in
/// `src/pages/Workflows.tsx`).
pub const EDIT_CONFLICT_ERROR: &str = "WORKFLOW_EDIT_CONFLICT";

/// An editor save whose prompt contains [`RUN_LOG_HEADER`].
const RUN_LOG_IN_PROMPT_ERROR: &str = "The prompt contains `## Run log`, which starts the run log, so everything after it would be read as log. Reword it and save again.";
/// How many times an editor save re-reads and retries when a status or
/// run-log write lands between its read and its write.
const SAVE_ATTEMPTS: usize = 3;
/// The one way `skip_if_quiet` is written: a bare YAML boolean.
const SKIP_IF_QUIET_LINE: &str = "skip_if_quiet: true";

/// What a read-modify-write of an existing workflow file does when its line
/// patch is refused (a hand-written indented line would shadow or hide a
/// key, see `verify_patch`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WritePolicy {
    /// Status and run-log writes: fall back to the canonical render (logged).
    /// A stuck `running` status or an un-advanced `next_run_at` would be
    /// worse than the lossy render of a pathological file, and the render
    /// re-emits every parsed value, so no permission stamp moves.
    Bookkeeping,
    /// Explicit user actions (`set_schedule`): return the
    /// error and write nothing.
    Owned,
}

/// Parsed frontmatter for a workflow file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkflowFrontmatter {
    pub title: String,
    pub description: String,
    pub schedule: String,
    /// `claude` | `codex` | `antigravity`. None ⇒ Claude (back-compat).
    pub agent: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    /// Optional glob (vault-relative) of files this workflow produces,
    /// e.g. `briefings/*.html`. Powers the outputs view in the UI.
    pub output_glob: Option<String>,
    /// Built-in routine marker (`morning-brief` | `plan-day` | `close-day` |
    /// `close-week`). Set only by the scaffolded routine templates; tags a
    /// workflow as one of Hollow's managed routines so the Settings,
    /// Automation UI can surface it (with a friendly enable + time control)
    /// rather than leaving it only in the Workflows tab. None ⇒ a normal
    /// user-authored workflow.
    pub routine: Option<String>,
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    /// Event triggers (the `triggers:` comma-list). Empty ⇒ schedule-only.
    /// Grammar in `trigger_evaluator::TriggerSpec`: `file-changed:<glob>`,
    /// `task-overdue`, `task-unblocked` (Paperclip PR 11b), `meeting-in:<dur>`.
    /// Retired email-from tokens are ignored.
    pub triggers: Vec<String>,
    /// `in_chat:` the conversation each run is a new turn in (overhaul F1,
    /// `automation_in_chat`). None: a new chat each run.
    pub in_chat: Option<String>,
    /// `runs_on:` the owner Mac (M4, `crate::runs_on`). None: no owner.
    pub runs_on: Option<String>,
    /// Misfire grace window (minutes). None ⇒ [`DEFAULT_MISFIRE_GRACE_MIN`].
    pub misfire_grace_min: Option<u64>,
    /// `skip_if_quiet: true` (Paperclip PR 9). Parsed, rendered and preserved;
    /// nothing reads it yet (9b adds the quiet-skip).
    pub skip_if_quiet: bool,
    /// `watch:` comma-list of vault-relative globs for the quiet-skip
    /// (Paperclip PR 9). Parsed and preserved; nothing reads it yet.
    pub watch: Vec<String>,
    /// `precheck:` (OC13) a one-line shell command whose non-zero exit skips
    /// a scheduled occurrence before any agent turn
    /// (`workflow_precheck`). A stamped permission field.
    pub precheck: Option<String>,
    /// `notify:` `always|failures|never` (Paperclip PR 9): filters the
    /// per-run notification and skip banners
    /// (`workflow_hygiene::NotifyPolicy`). Not stamped by PR 5.
    pub notify: Option<String>,
    /// When the last dispatched run started (Paperclip PR 9), written by
    /// [`mark_started`].
    pub last_started_at: Option<String>,
    pub created: Option<String>,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_duration_sec: Option<u64>,
    pub next_run_at: Option<String>,
    /// `ends:` (G2) an RFC 3339 instant; the scheduler ends the automation
    /// once it passes ([`end_if_due`]). Cleared when it ends.
    pub ends: Option<String>,
    /// `after_runs:` (G2) end after this many finished runs.
    pub after_runs: Option<u64>,
    /// `runs_counted:` (G2) runs finished since `after_runs` was set.
    pub runs_counted: Option<u64>,
    /// `ended_note:` (G2) why and when an end rule stopped it. Kept until a
    /// save sets a new end rule or a schedule.
    pub ended_note: Option<String>,
}

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
        "workflow".into()
    } else {
        joined.chars().take(120).collect()
    }
}

fn pick_filename(vault: &Path, title: &str) -> String {
    let base = slugify(title);
    let first = format!("{WORKFLOWS_DIR}/{base}.md");
    if !vault.join(&first).exists() {
        return first;
    }
    for i in 2..1000 {
        let candidate = format!("{WORKFLOWS_DIR}/{base}-{i}.md");
        if !vault.join(&candidate).exists() {
            return candidate;
        }
    }
    let ts = Utc::now().format("%Y%m%d%H%M%S");
    format!("{WORKFLOWS_DIR}/{base}-{ts}.md")
}

/// Parse a frontmatter map plus body into a `WorkflowFrontmatter`.
fn frontmatter_from_map(m: &BTreeMap<String, String>) -> WorkflowFrontmatter {
    WorkflowFrontmatter {
        title: m.get("title").cloned().unwrap_or_default(),
        description: m.get("description").cloned().unwrap_or_default(),
        schedule: m
            .get("schedule")
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "manual".into()),
        agent: m.get("agent").cloned().filter(|s| !s.is_empty()),
        model: m.get("model").cloned().filter(|s| !s.is_empty()),
        cwd: m.get("cwd").cloned().filter(|s| !s.is_empty()),
        output_glob: m.get("output_glob").cloned().filter(|s| !s.is_empty()),
        routine: m.get("routine").cloned().filter(|s| !s.is_empty()),
        permission_mode: m
            .get("permission_mode")
            .cloned()
            .filter(|s| !s.is_empty())
            // `acceptEdits` in the app; serve's own default there.
            .unwrap_or_else(|| crate::app_mcp::unrecorded_mode().into()),
        allowed_tools: parse_list(m.get("allowed_tools").map(|s| s.as_str())),
        disallowed_tools: parse_list(m.get("disallowed_tools").map(|s| s.as_str())),
        triggers: parse_list(m.get("triggers").map(|s| s.as_str())),
        in_chat: m
            .get("in_chat")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        runs_on: m
            .get(crate::runs_on::KEY)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        misfire_grace_min: m
            .get("misfire_grace_min")
            .and_then(|s| s.parse::<u64>().ok()),
        skip_if_quiet: parse_bool(m.get("skip_if_quiet").map(|s| s.as_str())),
        watch: parse_list(m.get("watch").map(|s| s.as_str())),
        precheck: m
            .get("precheck")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        notify: m
            .get("notify")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        last_started_at: m.get("last_started_at").cloned().filter(|s| !s.is_empty()),
        created: m.get("created").cloned().filter(|s| !s.is_empty()),
        last_run_at: m.get("last_run_at").cloned().filter(|s| !s.is_empty()),
        last_status: m.get("last_status").cloned().filter(|s| !s.is_empty()),
        last_error: m.get("last_error").cloned().filter(|s| !s.is_empty()),
        last_duration_sec: m
            .get("last_duration_sec")
            .and_then(|s| s.parse::<u64>().ok()),
        next_run_at: m.get("next_run_at").cloned().filter(|s| !s.is_empty()),
        ends: m
            .get("ends")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        after_runs: m
            .get("after_runs")
            .and_then(|s| s.trim().parse::<u64>().ok())
            .filter(|n| *n > 0),
        runs_counted: m
            .get("runs_counted")
            .and_then(|s| s.trim().parse::<u64>().ok()),
        ended_note: m
            .get("ended_note")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    }
}

fn parse_list(s: Option<&str>) -> Vec<String> {
    let raw = match s {
        Some(v) if !v.is_empty() => v,
        _ => return Vec::new(),
    };
    raw.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// A YAML-ish boolean: `true`, `yes`, `on` or `1`, case-insensitive (the
/// value is already unquoted by the parser, so `"true"` counts too).
fn parse_bool(s: Option<&str>) -> bool {
    s.map(str::trim)
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1"))
}

fn render_list(items: &[String]) -> String {
    items.join(", ")
}

/// Render a workflow file from frontmatter + prompt body + run log.
pub fn render(fm: &WorkflowFrontmatter, prompt: &str, run_log: &[RunLogEntry]) -> String {
    let mut lines: Vec<String> = vec!["---".into()];
    push_kv(&mut lines, "title", &fm.title);
    push_kv(&mut lines, "description", &fm.description);
    push_kv(&mut lines, "schedule", &fm.schedule);
    if let Some(a) = fm.agent.as_deref() {
        push_kv(&mut lines, "agent", a);
    }
    if let Some(m) = fm.model.as_deref() {
        push_kv(&mut lines, "model", m);
    }
    if let Some(c) = fm.cwd.as_deref() {
        push_kv(&mut lines, "cwd", c);
    }
    if let Some(g) = fm.output_glob.as_deref() {
        push_kv(&mut lines, "output_glob", g);
    }
    if let Some(r) = fm.routine.as_deref() {
        push_kv(&mut lines, "routine", r);
    }
    push_kv(&mut lines, "permission_mode", &fm.permission_mode);
    let allowed = render_list(&fm.allowed_tools);
    if !allowed.is_empty() {
        push_kv(&mut lines, "allowed_tools", &allowed);
    }
    let disallowed = render_list(&fm.disallowed_tools);
    if !disallowed.is_empty() {
        push_kv(&mut lines, "disallowed_tools", &disallowed);
    }
    let triggers = render_list(&fm.triggers);
    if !triggers.is_empty() {
        push_kv(&mut lines, "triggers", &triggers);
    }
    if let Some(key) = fm.in_chat.as_deref() {
        push_kv(&mut lines, "in_chat", key);
    }
    if let Some(owner) = fm.runs_on.as_deref() {
        push_kv(&mut lines, crate::runs_on::KEY, owner);
    }
    if let Some(g) = fm.misfire_grace_min {
        lines.push(format!("misfire_grace_min: {g}"));
    }
    if fm.skip_if_quiet {
        // A bare YAML boolean, never the quoted string `Set("true")` writes.
        lines.push(SKIP_IF_QUIET_LINE.into());
    }
    let watch = render_list(&fm.watch);
    if !watch.is_empty() {
        push_kv(&mut lines, "watch", &watch);
    }
    if let Some(command) = fm.precheck.as_deref() {
        push_kv(&mut lines, "precheck", command);
    }
    if let Some(n) = fm.notify.as_deref() {
        push_kv(&mut lines, "notify", n);
    }
    if let Some(e) = fm.ends.as_deref() {
        push_kv(&mut lines, "ends", e);
    }
    if let Some(n) = fm.after_runs {
        lines.push(format!("after_runs: {n}"));
    }
    if let Some(n) = fm.runs_counted {
        lines.push(format!("runs_counted: {n}"));
    }
    if let Some(n) = fm.ended_note.as_deref() {
        push_kv(&mut lines, "ended_note", n);
    }
    if let Some(c) = fm.created.as_deref() {
        push_kv(&mut lines, "created", c);
    }
    if let Some(at) = fm.last_started_at.as_deref() {
        push_kv(&mut lines, "last_started_at", at);
    }
    if let Some(at) = fm.last_run_at.as_deref() {
        push_kv(&mut lines, "last_run_at", at);
    }
    if let Some(s) = fm.last_status.as_deref() {
        push_kv(&mut lines, "last_status", s);
    }
    if let Some(e) = fm.last_error.as_deref() {
        push_kv(&mut lines, "last_error", e);
    }
    if let Some(d) = fm.last_duration_sec {
        lines.push(format!("last_duration_sec: {d}"));
    }
    if let Some(n) = fm.next_run_at.as_deref() {
        push_kv(&mut lines, "next_run_at", n);
    }
    lines.push("tags:".into());
    lines.push("  - workflows".into());
    if fm.schedule != "manual" {
        lines.push("  - schedule/scheduled".into());
    } else {
        lines.push("  - schedule/manual".into());
    }
    lines.push("---".into());
    lines.push(String::new());

    let trimmed = prompt.trim();
    if !trimmed.is_empty() {
        lines.push(trimmed.to_string());
        lines.push(String::new());
    }

    let mut out = lines.join("\n");
    out.push('\n');
    out.push_str(&render_run_log(run_log));
    out
}

/// The `## Run log` section exactly as [`render`] writes it, newest first,
/// ending with a newline. Also what a status write splices over the old
/// section of an existing file (`splice_body`).
fn render_run_log(run_log: &[RunLogEntry]) -> String {
    let mut lines: Vec<String> = vec![RUN_LOG_HEADER.into(), String::new()];
    if run_log.is_empty() {
        lines.push("_no runs yet_".into());
    } else {
        for entry in run_log.iter().rev() {
            lines.push(format_run_log_line(entry));
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn format_run_log_line(entry: &RunLogEntry) -> String {
    let note = if entry.note.is_empty() {
        String::new()
    } else {
        format!(" — {}", entry.note)
    };
    let mut line = format!(
        "- {} — {} ({}s){}",
        entry.at, entry.status, entry.duration_sec, note,
    );
    if let Some(run_id) = entry
        .run_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        line = format!("{line} {RUN_ID_MARKER_PREFIX} {run_id} {RUN_ID_MARKER_SUFFIX}");
    }
    if let Some(tail) = entry.error_tail.as_deref().filter(|t| !t.trim().is_empty()) {
        line = format!(
            "{line} {STDERR_MARKER_PREFIX} {} {RUN_ID_MARKER_SUFFIX}",
            encode_stderr_tail(tail)
        );
    }
    if let Some(kind) = entry
        .exit_kind
        .as_deref()
        .map(str::trim)
        .filter(|kind| EXIT_KINDS.contains(kind))
    {
        line = format!("{line} {EXIT_MARKER_PREFIX} {kind} {RUN_ID_MARKER_SUFFIX}");
    }
    line
}

/// Escape a multi-line stderr tail into a single-line, comment-safe token:
/// backslashes double, newlines become `\n`, `-->` (which would close the
/// surrounding HTML comment early) becomes `--\>`, and `<` becomes `\l`, so
/// an agent's stderr can never carry the opening of another hidden marker
/// (Paperclip PR 8b's exit marker is written right after this one). Older
/// encodings never contain a lone `\l` (their backslashes were doubled), so
/// they decode unchanged.
fn encode_stderr_tail(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\r', "")
        .replace('\n', "\\n")
        .replace("-->", "--\\>")
        .replace('<', "\\l")
}

fn decode_stderr_tail(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some('>') => out.push('>'),
            Some('l') => out.push('<'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn push_kv(lines: &mut Vec<String>, key: &str, value: &str) {
    crate::frontmatter::push_kv(lines, key, value);
}

/// Slice a file's content into (frontmatter map, body). Thin wrapper over
/// the shared [`crate::frontmatter`] parser. Every consumer here either
/// filters empty values or reads keys (`title`/`schedule`/`permission_mode`)
/// that are never empty, so routing through the shared parser — which keeps
/// empty-valued keys rather than dropping them — is behavior-preserving.
pub fn split_frontmatter(content: &str) -> (BTreeMap<String, String>, String) {
    crate::frontmatter::parse_map(content)
}

/// Pull the prompt body and run log out of a workflow file's markdown
/// body. The `## Run log` section is everything below that header.
pub fn split_body(body: &str) -> (String, Vec<RunLogEntry>) {
    let body = body.trim_end();
    if let Some(idx) = body.find(RUN_LOG_HEADER) {
        let prompt = body[..idx].trim_end().to_string();
        let log_block = body[idx + RUN_LOG_HEADER.len()..].trim();
        let entries = parse_run_log(log_block);
        (prompt, entries)
    } else {
        (body.to_string(), Vec::new())
    }
}

fn parse_run_log(s: &str) -> Vec<RunLogEntry> {
    let mut out: Vec<RunLogEntry> = Vec::new();
    for raw in s.lines() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("- ") else {
            continue;
        };
        // Markers come off the end in reverse write order: exit, stderr,
        // run id.
        let (rest, exit_kind) = split_exit_marker(rest);
        let (rest, error_tail) = split_stderr_marker(rest);
        let (rest, run_id) = split_run_id_marker(rest);
        // Format: "{at} — {status} ({duration}s)[ — {note}]"
        let parts: Vec<&str> = rest.splitn(3, " — ").collect();
        if parts.len() < 2 {
            continue;
        }
        let at = parts[0].trim().to_string();
        let (status, duration_sec) = match parse_status_and_duration(parts[1]) {
            Some(p) => p,
            None => continue,
        };
        let note = parts
            .get(2)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        out.push(RunLogEntry {
            at,
            status,
            duration_sec,
            note,
            run_id,
            error_tail,
            exit_kind,
        });
    }
    // The file lists newest-first; reverse so callers get chronological order.
    out.reverse();
    out
}

/// Separate the hidden exit-kind marker (Paperclip PR 8b) from the end of a
/// run-log line. It is written after every other marker, so it is stripped
/// first, and only when it is the very end of the line and names a known
/// kind ([`EXIT_KINDS`]). A forged marker in an agent's note is always
/// followed by the run-id marker and so is never at the end; a stderr tail
/// cannot hold one at all (its `<` is escaped).
fn split_exit_marker(rest: &str) -> (&str, Option<String>) {
    let Some(start) = rest.rfind(EXIT_MARKER_PREFIX) else {
        return (rest, None);
    };
    let marker = &rest[start + EXIT_MARKER_PREFIX.len()..];
    let Some(end) = marker.find(RUN_ID_MARKER_SUFFIX) else {
        return (rest, None);
    };
    if !marker[end + RUN_ID_MARKER_SUFFIX.len()..].trim().is_empty() {
        return (rest, None);
    }
    let kind = marker[..end].trim();
    if !EXIT_KINDS.contains(&kind) {
        return (rest, None);
    }
    (rest[..start].trim_end(), Some(kind.to_string()))
}

/// Separate the hidden stderr-tail marker from the end of a run-log line.
/// Written after the run-id marker, so it must be stripped first. Legacy
/// rows (and successes) simply return `None`.
fn split_stderr_marker(rest: &str) -> (&str, Option<String>) {
    let Some(start) = rest.rfind(STDERR_MARKER_PREFIX) else {
        return (rest, None);
    };
    let marker = &rest[start + STDERR_MARKER_PREFIX.len()..];
    let Some(end) = marker.find(RUN_ID_MARKER_SUFFIX) else {
        return (rest, None);
    };
    if !marker[end + RUN_ID_MARKER_SUFFIX.len()..].trim().is_empty() {
        return (rest, None);
    }
    let encoded = marker[..end].trim();
    if encoded.is_empty() {
        return (rest, None);
    }
    (rest[..start].trim_end(), Some(decode_stderr_tail(encoded)))
}

/// Separate the hidden durable-run marker from the human-readable row. The
/// marker is recognized only at the end of a line, so a note that happens to
/// mention similar text remains untouched. Legacy rows simply return `None`.
fn split_run_id_marker(rest: &str) -> (&str, Option<String>) {
    let Some(start) = rest.rfind(RUN_ID_MARKER_PREFIX) else {
        return (rest, None);
    };
    let marker = &rest[start + RUN_ID_MARKER_PREFIX.len()..];
    let Some(end) = marker.find(RUN_ID_MARKER_SUFFIX) else {
        return (rest, None);
    };
    if !marker[end + RUN_ID_MARKER_SUFFIX.len()..].trim().is_empty() {
        return (rest, None);
    }
    let run_id = marker[..end].trim();
    if run_id.is_empty() {
        return (rest, None);
    }
    (rest[..start].trim_end(), Some(run_id.to_string()))
}

fn parse_status_and_duration(seg: &str) -> Option<(String, u64)> {
    // "success (14s)" → ("success", 14)
    let open = seg.find('(')?;
    let close = seg.rfind(')')?;
    if close <= open {
        return None;
    }
    let status = seg[..open].trim().to_string();
    let dur_raw = seg[open + 1..close].trim_end_matches('s');
    let duration = dur_raw.parse::<u64>().ok()?;
    Some((status, duration))
}

/// Build a `Workflow` summary from a parsed file.
fn build_workflow(rel: &str, fm: &WorkflowFrontmatter) -> Workflow {
    let slug = slug_from_rel(rel);
    Workflow {
        slug,
        title: fm.title.clone(),
        description: fm.description.clone(),
        schedule: fm.schedule.clone(),
        agent: fm.agent.clone(),
        model: fm.model.clone(),
        cwd: fm.cwd.clone(),
        output_glob: fm.output_glob.clone(),
        routine: fm.routine.clone(),
        permission_mode: fm.permission_mode.clone(),
        allowed_tools: fm.allowed_tools.clone(),
        disallowed_tools: fm.disallowed_tools.clone(),
        triggers: fm.triggers.clone(),
        in_chat: fm.in_chat.clone(),
        runs_on: fm.runs_on.clone(),
        misfire_grace_min: fm.misfire_grace_min,
        catch_up: crate::schedule_windows::effective_catch_up(&fm.schedule)
            .map(|p| p.as_str().to_string()),
        schedule_problem: validate_schedule(&fm.schedule).err(),
        skip_if_quiet: fm.skip_if_quiet,
        watch: fm.watch.clone(),
        precheck: fm.precheck.clone(),
        notify: fm.notify.clone(),
        last_started_at: fm.last_started_at.clone(),
        ends: fm.ends.clone(),
        after_runs: fm.after_runs,
        runs_counted: fm.runs_counted,
        ended_note: fm.ended_note.clone(),
        created: fm.created.clone(),
        last_run_at: fm.last_run_at.clone(),
        last_status: fm.last_status.clone(),
        last_error: fm.last_error.clone(),
        last_duration_sec: fm.last_duration_sec,
        next_run_at: fm.next_run_at.clone(),
        path: rel.to_string(),
        permission_hold: None,
        budget_hold: None,
    }
}

pub(crate) fn slug_from_rel(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.trim_end_matches(".md").to_string()
}

/// Walk `vault/workflows/*.md` and return parsed entries.
pub fn list(vault: &Path) -> Vec<Workflow> {
    let root = vault.join(WORKFLOWS_DIR);
    if !root.exists() {
        return Vec::new();
    }
    let mut out: Vec<Workflow> = Vec::new();
    for entry in WalkDir::new(&root)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
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
        let (map, _) = split_frontmatter(&content);
        if map.get("title").map(|s| s.is_empty()).unwrap_or(true) {
            continue;
        }
        let fm = frontmatter_from_map(&map);
        out.push(build_workflow(&rel_str, &fm));
    }
    // Stable ordering: title, case-insensitive. Title is required so it
    // exists for every entry that reaches this point.
    out.sort_by_key(|a| a.title.to_lowercase());
    out
}

/// List the files a workflow's `output_glob` matches, newest-first. The
/// glob is vault-relative; an optional directory prefix plus a single
/// `*` / `?` wildcard in the filename is supported (e.g. `briefings/*.html`,
/// `briefings/*`, `reports/weekly-*.md`). Returns empty when the workflow
/// has no `output_glob` or the target directory is missing.
pub fn list_outputs(vault: &Path, slug: &str) -> Vec<WorkflowOutput> {
    let glob = match read_full(vault, slug) {
        Ok((w, _, _)) => match w.output_glob {
            Some(g) if !g.trim().is_empty() => g.trim().trim_start_matches('/').to_string(),
            _ => return Vec::new(),
        },
        Err(_) => return Vec::new(),
    };
    let (dir_rel, pat) = match glob.rsplit_once('/') {
        Some((d, p)) => (d.to_string(), p.to_string()),
        None => (String::new(), glob.clone()),
    };
    // The glob is frontmatter anyone can edit: its folder is confined to
    // the vault (no `..`, absolute or symlinked escape), or nothing is listed.
    let dir = if dir_rel.is_empty() {
        vault.to_path_buf()
    } else {
        match vault::safe_join(vault, &dir_rel) {
            Ok(dir) => dir,
            Err(_) => return Vec::new(),
        }
    };
    if !dir.exists() {
        return Vec::new();
    }
    let re = wildcard_to_regex(&pat);
    // `safe_join` returns a canonical path, so relative paths are taken
    // against the canonical vault too (a vault reached through a symlink,
    // like `/var` → `/private/var`, would otherwise list nothing).
    let root = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    let mut out: Vec<WorkflowOutput> = Vec::new();
    for entry in WalkDir::new(&dir)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || !re.is_match(&name) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&root) else {
            continue;
        };
        let modified = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| DateTime::<Utc>::from(t).to_rfc3339())
            .unwrap_or_default();
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format = match ext.as_str() {
            "html" | "htm" => "html",
            "md" | "markdown" => "md",
            other => other,
        }
        .to_string();
        let stem = entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&name)
            .to_string();
        out.push(WorkflowOutput {
            path: rel.to_string_lossy().to_string(),
            name: stem,
            format,
            modified,
        });
    }
    // Newest first (RFC3339 timestamps sort lexicographically by time).
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// Translate a shell-style filename wildcard (`*`, `?`) into an anchored
/// regex; every other character is matched literally.
fn wildcard_to_regex(pat: &str) -> Regex {
    let mut re = String::from("^");
    for ch in pat.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    Regex::new(&re).unwrap_or_else(|_| Regex::new("^$").unwrap())
}

/// Read the full workflow (frontmatter + prompt body + parsed run log).
pub fn read_full(vault: &Path, slug: &str) -> Result<(Workflow, String, Vec<RunLogEntry>), String> {
    let rel = format!("{WORKFLOWS_DIR}/{slug}.md");
    let full = vault::safe_join(vault, &rel)?;
    let content = fs::read_to_string(&full).map_err(|e| format!("read {rel}: {e}"))?;
    parse_content(slug, &content)
}

/// Parse a workflow file's text into (workflow, prompt body, run log).
///
/// This is THE parser for everything that decides what a workflow does:
/// execution, permission stamping and post-run flagging all go through it,
/// so duplicate keys, dropped YAML-list lines or a fake `## Run log` header
/// can never make the stamped view differ from what runs
/// ([`crate::workflow_permissions`]).
pub(crate) fn parse_content(
    slug: &str,
    content: &str,
) -> Result<(Workflow, String, Vec<RunLogEntry>), String> {
    let rel = format!("{WORKFLOWS_DIR}/{slug}.md");
    let (map, body) = split_frontmatter(content);
    let title = map
        .get("title")
        .cloned()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("workflow {slug} has no title"))?;
    let mut fm = frontmatter_from_map(&map);
    fm.title = title;
    let (prompt, run_log) = split_body(&body);
    Ok((build_workflow(&rel, &fm), prompt, run_log))
}

/// Validate a schedule string. Returns Ok for `manual` or any cron
/// expression parseable by the `cron` crate. Otherwise returns the
/// parse error so the UI can surface it.
pub fn validate_schedule(schedule: &str) -> Result<(), String> {
    if schedule == "manual" {
        return Ok(());
    }
    if crate::schedule_windows::schedule_window(schedule).is_some() {
        return crate::schedule_windows::validate_window_schedule(schedule);
    }
    Schedule::from_str(schedule)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Compute the next time a cron expression should fire after `now`.
/// Returns None for `manual` or invalid expressions.
///
/// The cron hour field is interpreted in the machine's **local** timezone, not
/// UTC: the UI (and the seed) author these schedules as local wall-clock times
/// (e.g. close-day at hour 18 means 6pm where the user sits). We evaluate
/// against `Local` so a routine fires at its intended local hour year-round
/// (the offset, incl. DST, is resolved at compute time), then convert the
/// resulting instant back to UTC for storage + the scheduler's `Utc::now()`
/// comparison. Evaluating against UTC was the bug that fired every routine
/// 4–5h early (e.g. close-day at 2pm EDT instead of 6pm).
pub fn next_run_after(schedule: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if schedule == "manual" {
        return None;
    }
    if crate::schedule_windows::schedule_window(schedule).is_some() {
        return crate::schedule_windows::next_window_run(schedule, now);
    }
    let sched = Schedule::from_str(schedule).ok()?;
    let local_now = now.with_timezone(&chrono::Local);
    sched
        .after(&local_now)
        .next()
        .map(|d| d.with_timezone(&Utc))
}

/// Where `next_run_at` goes once a run of `w`'s stored occurrence finished
/// at `now` (Scheduler v2): under a `/run-all` window the next missed
/// occurrence (bounded), otherwise [`next_run_after`] `now`.
pub fn next_after_run(w: &Workflow, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    use crate::schedule_windows::{effective_catch_up, next_after_run as walk, CatchUp};
    let policy = effective_catch_up(&w.schedule);
    if policy != Some(CatchUp::RunAll) {
        return next_run_after(&w.schedule, now);
    }
    let due = w
        .next_run_at
        .as_deref()
        .and_then(|d| DateTime::parse_from_rfc3339(d).ok())
        .map(|d| d.with_timezone(&Utc));
    let schedule = w.schedule.clone();
    let after = move |t: DateTime<Utc>| {
        let mut out = Vec::new();
        let mut at = t;
        // Enough to see past `now` behind RUN_ALL_MAX missed occurrences of
        // even a minutely cron over a closed week.
        for _ in 0..20_000 {
            match next_run_after(&schedule, at) {
                Some(next) if next > at => {
                    out.push(next);
                    if next > now {
                        break;
                    }
                    at = next;
                }
                _ => break,
            }
        }
        out
    };
    walk(policy, due, now, &after)
}

/// Create or update a workflow from positional arguments, with no editor
/// baseline. A thin wrapper over [`upsert_input`], kept for tests and the
/// shared fixture ([`seed_workflow_with`]); the IPC calls `upsert_input`.
#[cfg(test)]
pub fn upsert(
    vault: &Path,
    original_slug: Option<&str>,
    input_title: &str,
    input_description: &str,
    schedule: &str,
    agent: Option<&str>,
    model: Option<&str>,
    cwd: Option<&str>,
    permission_mode: &str,
    allowed_tools: &[String],
    disallowed_tools: &[String],
    output_glob: Option<&str>,
    prompt: &str,
) -> Result<Workflow, String> {
    upsert_input(
        vault,
        original_slug,
        &WorkflowInput {
            title: input_title.to_string(),
            description: input_description.to_string(),
            schedule: schedule.to_string(),
            agent: agent.map(str::to_string),
            model: model.map(str::to_string),
            cwd: cwd.map(str::to_string),
            output_glob: output_glob.map(str::to_string),
            permission_mode: permission_mode.to_string(),
            allowed_tools: allowed_tools.to_vec(),
            disallowed_tools: disallowed_tools.to_vec(),
            prompt: prompt.to_string(),
            edit_base: None,
            notify: None,
            skip_if_quiet: None,
            watch: None,
            precheck: None,
            triggers: None,
            in_chat: None,
            ends: None,
            after_runs: None,
            runs_on: None,
        },
    )
}

/// The automation editor's save (`write_workflow`): create a workflow, or
/// update `original_slug` in place, or rename it when the title's slug
/// changes. Returns the saved `Workflow`.
///
/// Updating an existing file is a verified line patch of only the fields
/// the editor owns (title, description, schedule and its `next_run_at`,
/// agent, model, cwd, output_glob, permission_mode, the tool lists, and the
/// prompt, spliced so the run log keeps its bytes). Everything else in the
/// file survives: unknown keys, lists, comments, `routine`, `triggers`,
/// `misfire_grace_min`, `skip_if_quiet`/`watch`/`notify`, `created`, every
/// `last_*` key and the run log. A refused patch is returned as the error
/// and nothing is written; an editor save never falls back to a render.
/// A prior file that no longer parses (no title) keeps PR 5's render path,
/// written with compare-and-swap.
///
/// Race safety, all under the permission write guard (held across every
/// attempt and the re-stamp, so a scheduler verify never sees a half-done
/// save):
/// - `input.edit_base` (the editor's [`edit_digest`] from when it loaded the
///   file) is compared with each attempt's prior bytes: a file whose
///   editor-owned fields changed since returns `WORKFLOW_EDIT_CONFLICT: …`.
///   A run finishing meanwhile (status, run log, `next_run_at`) never
///   conflicts, because none of that is in the digest.
/// - In place, the write is a compare-and-swap against the prior bytes; a
///   status or run-log write landing in between retries (at most
///   [`SAVE_ATTEMPTS`] times) on the fresh bytes.
/// - A rename is refused while the workflow runs or holds the vault lease.
///   Otherwise, under `vault::MUTATE_LOCK`, it re-reads the old file (a
///   bookkeeping write since the read retries), creates the new file
///   exclusively, then moves the old one to `.trash`. The new file is
///   written first, so a failure never loses the automation; if the old file
///   cannot be trashed the new one is, and the save fails.
///
/// A prior file that disappeared while the editor was open is an error
/// (`WORKFLOW_EDIT_CONFLICT: …` when an `edit_base` was sent), never a
/// silent recreation. The re-stamp gets the prior bytes of the attempt that
/// actually wrote ([`crate::workflow_permissions::after_upsert_locked`]).
/// The editor-input checks every save makes before touching a file.
fn check_input(input: &WorkflowInput) -> Result<(), String> {
    if input.title.trim().is_empty() {
        return Err("title is required".into());
    }
    // Self-hosted models Q5 review: an automation isn't a chat.
    if let Some(model) = input
        .model
        .as_deref()
        .filter(|m| crate::self_hosted::entry_id(m).is_some())
    {
        return Err(crate::self_hosted::not_for_this(model));
    }
    validate_schedule(&input.schedule)?;
    check_end_input(input, Utc::now())?;
    if input
        .triggers
        .iter()
        .flatten()
        .any(|token| token.trim().is_empty() || token.contains([',', '\r', '\n']))
    {
        return Err("A trigger must be nonempty and contain no commas or line breaks".into());
    }
    if let Some(value) = input.notify.as_deref() {
        notify_for_file(value)?;
    }
    // M4: an owner must name a Mac (`this` needs this Mac's name).
    if let Some(raw) = input.runs_on.as_deref() {
        crate::runs_on::for_file(raw, &crate::runs_on::Seat::current())?;
    }
    // `watch` is stored as a comma-list, so a glob may not contain a comma.
    if let Some(glob) = input.watch.iter().flatten().find(|glob| glob.contains(',')) {
        return Err(format!("A watch glob may not contain a comma: {glob:?}"));
    }
    if let Some(command) = &input.precheck {
        crate::workflow_precheck::check_command(command)?;
    }
    // `split_body` reads everything from the first `## Run log` as the log,
    // so such a prompt would lose its tail on the next read. Refused for a
    // create, an in-place save and a rename alike.
    if input.prompt.contains(RUN_LOG_HEADER) {
        return Err(RUN_LOG_IN_PROMPT_ERROR.into());
    }
    Ok(())
}

/// What [`upsert_input`] would write for `input` now, without writing: the
/// slug it lands at, and the permission fields before (none for a create)
/// and after. Hollow app tools' `save_automation` card shows these, and the
/// approval it records is for exactly `after` (PR A3). Same checks and the
/// same content builders as the save.
pub(crate) fn preview_input(
    vault: &Path,
    original_slug: Option<&str>,
    input: &WorkflowInput,
) -> Result<SavePreview, String> {
    use crate::workflow_permissions::PermissionFields;
    check_input(input)?;
    let (slug, before, content) = match original_slug.filter(|s| !s.is_empty()) {
        None => {
            let slug = slug_from_rel(&pick_filename(vault, &input.title));
            let fm = new_frontmatter(vault, input);
            bound_tools_check(&fm)?;
            (slug, None, render(&fm, &input.prompt, &[]))
        }
        Some(prev) => {
            let prev_rel = format!("{WORKFLOWS_DIR}/{prev}.md");
            let prior = match fs::read_to_string(vault::safe_join(vault, &prev_rel)?) {
                Ok(prior) => prior,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(missing_prior_error(input))
                }
                Err(e) => return Err(format!("read {prev_rel}: {e}")),
            };
            let slug = slug_from_rel(&pick_target_filename(vault, &input.title, prev));
            let before = PermissionFields::from_content(prev, &prior).map(|(fields, _)| fields);
            (slug, before, saved_content(prev, &prior, input)?)
        }
    };
    let (after, prompt) = PermissionFields::from_content(&slug, &content)
        .ok_or("The automation would not parse as written; nothing was saved.")?;
    Ok(SavePreview {
        slug,
        before,
        after,
        prompt,
    })
}

/// [`preview_input`]'s answer.
#[derive(Debug, Clone)]
pub(crate) struct SavePreview {
    pub slug: String,
    pub before: Option<crate::workflow_permissions::PermissionFields>,
    pub after: crate::workflow_permissions::PermissionFields,
    /// The trimmed prompt as it would be saved.
    pub prompt: String,
}

/// The person's own editor save (`write_workflow`, called by the UI): a save
/// that re-stamps also pins the precheck script as it is now.
pub fn upsert_input(
    vault: &Path,
    original_slug: Option<&str>,
    input: &WorkflowInput,
) -> Result<Workflow, String> {
    upsert_input_by(vault, original_slug, input, false)
}

/// [`upsert_input`] for a save an AGENT drives (`automations.save`): the same
/// write, but its re-stamp keeps the old precheck script pin instead of
/// hashing whatever the script is now, so an agent that rewrote the script
/// cannot have it approved by a card that shows no script bytes.
pub fn upsert_input_by(
    vault: &Path,
    original_slug: Option<&str>,
    input: &WorkflowInput,
    by_agent: bool,
) -> Result<Workflow, String> {
    let title = input.title.as_str();
    check_input(input)?;
    // Overhaul F1: a new binding must name a live conversation.
    if let Some(key) = input
        .in_chat
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        crate::automation_in_chat::check_binding(vault, key)?;
    }

    // Hollow-owned workflow writer: hold the permission write guard across
    // the file write AND its re-stamp, so a scheduler verify can never land
    // between the two and queue a spurious review. Lock order: this guard →
    // { workflows' running registry, `vault::MUTATE_LOCK` } → the stamp
    // store and review queue; `lease_holder` takes no lock.
    let _guard = crate::workflow_permissions::write_guard();

    let Some(prev) = original_slug.filter(|s| !s.is_empty()) else {
        return create_locked(vault, input, by_agent);
    };
    let prev_rel = format!("{WORKFLOWS_DIR}/{prev}.md");
    let prev_path = vault::safe_join(vault, &prev_rel)?;
    let new_rel = pick_target_filename(vault, title, prev);
    let new_slug = slug_from_rel(&new_rel);
    let renamed = new_rel != prev_rel;
    // An admission registers its running slot and checks the file under this
    // same guard, so a run is either refused here or finds the file gone.
    if renamed && workflow_is_busy(vault, prev) {
        return Err(format!(
            "`{prev}` is running. Wait for its run to finish, then rename it."
        ));
    }

    for _ in 0..SAVE_ATTEMPTS {
        let prior = match fs::read_to_string(&prev_path) {
            Ok(prior) => prior,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(missing_prior_error(input));
            }
            Err(e) => return Err(format!("read {prev_rel}: {e}")),
        };
        if let Some(base) = input.edit_base.as_deref() {
            let current = parse_content(prev, &prior)
                .ok()
                .map(|(w, prompt, _)| edit_digest(&w, &prompt));
            if current.as_deref() != Some(base) {
                return Err(format!(
                    "{EDIT_CONFLICT_ERROR}: This automation changed outside the editor since you opened it, so nothing was saved. Your edits are still in the form: copy what you want to keep, then reopen the automation, which loads the current file and discards the unsaved edits."
                ));
            }
        }
        #[cfg(test)]
        run_before_write_hook();
        let content = saved_content(prev, &prior, input)?;
        validate_quiet(vault, &new_slug, Some(&prior), &content, input)?;

        if renamed {
            let _mutate = vault::lock_mutate();
            match fs::read_to_string(&prev_path) {
                Ok(current) if current == prior => {}
                // A status or run-log write landed since the read: redo the
                // save on the fresh bytes so that run's row is not lost.
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(missing_prior_error(input));
                }
                Err(e) => return Err(format!("read {prev_rel}: {e}")),
            }
            vault::create_note(vault, &new_rel, &content)?;
            if let Err(e) = vault::delete_note(vault, &prev_rel) {
                let _ = vault::delete_note(vault, &new_rel);
                return Err(format!(
                    "Could not move {prev_rel} to the trash ({e}); nothing was renamed."
                ));
            }
        } else {
            match vault::write_note_cas(vault, &prev_rel, &content, &prior)? {
                vault::CasOutcome::Written(_) => {}
                vault::CasOutcome::Conflict(_) => continue,
            }
        }
        // Paperclip PR 9 (9a-2), with MUTATE_LOCK released (it and the
        // review queue's lock are never nested): a renamed automation's
        // failure streak spoke for the old file, so close it; and tell the
        // run guard what the owner wrote, so a run in flight is not flagged
        // for this save's notify change (or the old file's disappearance).
        if renamed {
            crate::workflow_hygiene::close_streak(
                vault,
                prev,
                crate::workflow_hygiene::STREAK_RENAMED_REASON,
            );
            crate::run_guard::note_owned_workflow_settings(vault, prev, None, None);
        }
        let saved = parse_content(&new_slug, &content).map(|(w, _, _)| w);
        if let Ok(workflow) = &saved {
            // Only what this save changed relative to the file it replaced
            // is the owner's: a save that carries an agent's `notify`,
            // `skip_if_quiet` or `watch` through unchanged (the editor sends
            // each only when the user changed it) must not excuse it.
            // An unparseable prior records nothing (conservative: whoever
            // broke it may have been a run).
            if let Ok((before, _, _)) = parse_content(prev, &prior) {
                crate::run_guard::note_owned_workflow_settings(
                    vault,
                    &new_slug,
                    Some(&crate::run_guard::QuietSettings::of(&before)),
                    Some(&crate::run_guard::QuietSettings::of(workflow)),
                );
            }
        }
        // Re-stamp by the owned-writer rule, from the bytes this attempt
        // actually replaced: a save re-stamps only when they still matched
        // the stamp, and a rename moves the stamp first.
        crate::workflow_permissions::after_upsert_locked(
            vault,
            Some(prev),
            &new_slug,
            Some(&prior),
            &content,
            by_agent,
        );
        // A rename carries its usage budgets and pauses along (Paperclip PR
        // 8a), still under the write guard, so a claim can never see the new
        // slug without its budget. Logged, never fails the save.
        if renamed {
            crate::budgets::rename_workflow_scope(
                &crate::budgets::Store::for_app(vault),
                vault,
                prev,
                &new_slug,
            );
        }
        return saved;
    }
    Err("The automation kept changing while it was being saved, so nothing was saved. Reopen it and save again.".into())
}

/// A new automation from the editor's create form. The caller holds the
/// permission write guard. The file is created exclusively (never replacing
/// one that appeared meanwhile) and stamped as Hollow-created.
fn create_locked(vault: &Path, input: &WorkflowInput, by_agent: bool) -> Result<Workflow, String> {
    let rel = pick_filename(vault, &input.title);
    let slug = slug_from_rel(&rel);
    let fm = new_frontmatter(vault, input);
    bound_tools_check(&fm)?;
    end_rule_check(&fm)?;
    let content = render(&fm, &input.prompt, &[]);
    validate_quiet(vault, &slug, None, &content, input)?;
    vault::create_note(vault, &rel, &content)?;
    let workflow = build_workflow(&rel, &fm);
    crate::run_guard::note_owned_workflow_settings(
        vault,
        &slug,
        None,
        Some(&crate::run_guard::QuietSettings::of(&workflow)),
    );
    crate::workflow_permissions::after_upsert_locked(vault, None, &slug, None, &content, by_agent);
    Ok(workflow)
}

/// A new automation's frontmatter from `input`: with no `runsOn` sent it
/// gets this Mac as its owner only once another Mac shares the vault (M4,
/// `runs_on::default_for_new`); alone it has none, as before.
fn new_frontmatter(vault: &Path, input: &WorkflowInput) -> WorkflowFrontmatter {
    let mut fm = WorkflowFrontmatter::default();
    apply_input(&mut fm, input);
    if input.runs_on.is_none() {
        fm.runs_on = crate::runs_on::default_for_new(vault, &crate::runs_on::Seat::current());
    }
    fm
}

/// Overhaul F1: an automation that runs in a chat can't keep a tool
/// allowlist or denylist, because a chat turn doesn't carry them
/// (`automation_in_chat::TOOL_LISTS_REFUSED`).
fn bound_tools_check(fm: &WorkflowFrontmatter) -> Result<(), String> {
    if fm.in_chat.is_some() && (!fm.allowed_tools.is_empty() || !fm.disallowed_tools.is_empty()) {
        return Err(crate::automation_in_chat::TOOL_LISTS_REFUSED.into());
    }
    Ok(())
}

// ── End rules (G2) ──────────────────────────────────────────────────────────

/// The most runs an `after_runs` rule may count.
const MAX_AFTER_RUNS: u64 = 10_000;

/// Read an end time: an RFC 3339 instant, or a local `YYYY-MM-DDTHH:MM[:SS]`
/// (a space may stand for the `T`).
pub fn parse_end(raw: &str) -> Result<DateTime<Utc>, String> {
    use chrono::{Local, NaiveDateTime, TimeZone};
    let raw = raw.trim();
    if let Ok(at) = DateTime::parse_from_rfc3339(raw) {
        return Ok(at.with_timezone(&Utc));
    }
    let unspaced = raw.replacen(' ', "T", 1);
    for format in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&unspaced, format) {
            return Local
                .from_local_datetime(&naive)
                .earliest()
                .map(|at| at.with_timezone(&Utc))
                .ok_or_else(|| format!("{raw:?} does not exist in this time zone"));
        }
    }
    Err(format!(
        "The end time {raw:?} is not a date and time (use 2026-11-20T18:00:00-05:00 or 2026-11-20T18:00)."
    ))
}

/// The save-time checks on an end rule the input carries: a readable future
/// time, a sane run count, and not both. Nothing here reads the saved file,
/// so a save that omits both keys never fails on an end time that has passed.
pub(crate) fn check_end_input(input: &WorkflowInput, now: DateTime<Utc>) -> Result<(), String> {
    let ends = input
        .ends
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let runs = input.after_runs.filter(|n| *n > 0);
    if ends.is_some() && runs.is_some() {
        return Err("Set an end time or an end after a number of runs, not both.".into());
    }
    if let Some(raw) = ends {
        if parse_end(raw)? <= now {
            return Err(format!("The end time {raw:?} has already passed."));
        }
    }
    if runs.is_some_and(|n| n > MAX_AFTER_RUNS) {
        return Err(format!(
            "An end after runs counts at most {MAX_AFTER_RUNS}."
        ));
    }
    Ok(())
}

/// What the saved automation may carry: one end rule, never two (a hand edit
/// can write both; a save refuses to keep them).
fn end_rule_check(fm: &WorkflowFrontmatter) -> Result<(), String> {
    if fm.ends.is_some() && fm.after_runs.is_some() {
        return Err("The file has both `ends` and `after_runs`; keep one end rule.".into());
    }
    Ok(())
}

/// Apply the input's end rule (already validated). Omitted keys change
/// nothing. A new rule replaces the other kind, restarts the run count and
/// clears an old `ended_note`, and saving a schedule clears it too, because
/// the person has started the automation again.
fn apply_end_input(fm: &mut WorkflowFrontmatter, input: &WorkflowInput) {
    // Any end field the save carries (a cleared one too) is the person
    // handling the end rule: the note goes, so a trigger-only automation that
    // ended can be restarted.
    if input.ends.is_some() || input.after_runs.is_some() {
        fm.ended_note = None;
    }
    if let Some(triggers) = &input.triggers {
        let new: Vec<String> = triggers.iter().map(|t| t.trim().to_string()).collect();
        if new != fm.triggers {
            fm.ended_note = None;
        }
    }
    if let Some(raw) = &input.ends {
        let raw = raw.trim();
        let new = if raw.is_empty() {
            None
        } else {
            parse_end(raw).ok().map(|at| at.to_rfc3339())
        };
        if new != fm.ends {
            fm.ended_note = None;
        }
        if new.is_some() {
            fm.after_runs = None;
            fm.runs_counted = None;
        }
        fm.ends = new;
    }
    if let Some(n) = input.after_runs {
        let new = (n > 0).then_some(n);
        if new != fm.after_runs {
            fm.runs_counted = None;
            fm.ended_note = None;
        }
        if new.is_some() {
            fm.ends = None;
        }
        fm.after_runs = new;
    }
    if input.schedule != "manual" {
        fm.ended_note = None;
    }
}

/// Why `fm`'s end rule has fired at `now`, or None while it has not.
fn end_reason(fm: &WorkflowFrontmatter, now: DateTime<Utc>) -> Option<String> {
    if let Some(at) = fm.ends.as_deref().and_then(|raw| parse_end(raw).ok()) {
        if at <= now {
            return Some(format!(
                "its end time ({}) passed",
                at.format("%Y-%m-%d %H:%M UTC")
            ));
        }
    }
    match (fm.after_runs, fm.runs_counted) {
        (Some(n), Some(done)) if done >= n => Some(format!("it finished its {n} runs")),
        _ => None,
    }
}

/// Stop `fm` on its own: manual, no next run, the rule cleared, and a note
/// saying when and why. Triggers are not touched (they are a stamped
/// permission field); `ended_note` is what keeps the scheduler from firing
/// them ([`Workflow::ended_note`]).
fn apply_end(fm: &mut WorkflowFrontmatter, reason: &str, now: DateTime<Utc>) {
    fm.schedule = "manual".into();
    fm.next_run_at = None;
    fm.ends = None;
    fm.after_runs = None;
    fm.runs_counted = None;
    fm.ended_note = Some(format!(
        "Ended {} because {reason}. It now runs only when you start it.",
        now.format("%Y-%m-%d %H:%M UTC")
    ));
}

/// Has `w`'s end time passed at `now` without the scheduler ending it yet
/// (a busy vault, a failed write)? Such an automation is not enqueued.
pub fn end_passed(w: &Workflow, now: DateTime<Utc>) -> bool {
    w.ends
        .as_deref()
        .and_then(|raw| parse_end(raw).ok())
        .is_some_and(|at| at <= now)
}

/// End `slug` if its end rule has fired at `now` (the scheduler's pass, under
/// its bookkeeping lease). An owned write like [`set_schedule`]: the schedule
/// change and its stamp update are one step under the permission write guard,
/// so the ended automation can still be run by hand. Returns whether it ended.
pub(crate) fn end_if_due(vault: &Path, slug: &str, now: DateTime<Utc>) -> Result<bool, String> {
    let _guard = crate::workflow_permissions::write_guard();
    let mut ended = false;
    let (_, before, after) = mutate_capturing(vault, slug, WritePolicy::Bookkeeping, |fm, _| {
        if let Some(reason) = end_reason(fm, now) {
            apply_end(fm, &reason, now);
            ended = true;
        }
    })?;
    if ended {
        crate::workflow_permissions::after_schedule_write_locked(vault, slug, &before, &after);
    }
    Ok(ended)
}

/// Copy the fields the automation editor owns from `input` onto `fm`.
/// `next_run_at` follows the schedule (recomputed on every save, as before);
/// `created` is stamped only when the file has none. Nothing else changes.
fn apply_input(fm: &mut WorkflowFrontmatter, input: &WorkflowInput) {
    fn given(value: &Option<String>) -> Option<String> {
        value
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
    fm.title = input.title.clone();
    fm.description = input.description.clone();
    fm.schedule = input.schedule.clone();
    fm.agent = given(&input.agent);
    fm.model = given(&input.model);
    fm.cwd = given(&input.cwd);
    fm.output_glob = given(&input.output_glob);
    // An empty mode reads back as the runner's default, so write that.
    fm.permission_mode = if input.permission_mode.is_empty() {
        "acceptEdits".into()
    } else {
        input.permission_mode.clone()
    };
    fm.allowed_tools = input.allowed_tools.clone();
    fm.disallowed_tools = input.disallowed_tools.clone();
    if let Some(triggers) = &input.triggers {
        fm.triggers = triggers
            .iter()
            .map(|token| token.trim().to_string())
            .collect();
    }
    // Overhaul F1: only when sent; empty unbinds (checked in `upsert_input`).
    if let Some(key) = &input.in_chat {
        let key = key.trim();
        fm.in_chat = (!key.is_empty()).then(|| key.to_string());
    }
    // Only when the editor sent it (validated by `upsert_input` first): a
    // caller that doesn't know the key never clears it.
    if let Some(Ok(notify)) = input.notify.as_deref().map(notify_for_file) {
        fm.notify = notify;
    }
    // Paperclip PR 9b: the quiet-skip settings, again only when sent.
    if let Some(skip) = input.skip_if_quiet {
        fm.skip_if_quiet = skip;
    }
    if let Some(watch) = &input.watch {
        fm.watch = watch
            .iter()
            .map(|glob| glob.trim().to_string())
            .filter(|glob| !glob.is_empty())
            .collect();
    }
    // OC13: only when sent (validated by `check_input` first); empty removes.
    // Only the editor's own save sends it; the agent action strips it.
    if let Some(command) = &input.precheck {
        let command = command.trim();
        fm.precheck = (!command.is_empty()).then(|| command.to_string());
    }
    // M4: only when sent (validated by `check_input` first); empty removes.
    if let Some(Ok(owner)) = input
        .runs_on
        .as_deref()
        .map(|raw| crate::runs_on::for_file(raw, &crate::runs_on::Seat::current()))
    {
        fm.runs_on = owner;
    }
    apply_end_input(fm, input);
    fm.next_run_at = next_run_after(&input.schedule, Utc::now()).map(|d| d.to_rfc3339());
    if fm.created.is_none() {
        fm.created = Some(Utc::now().to_rfc3339());
    }
}

/// The file an editor save of `slug` writes over the prior bytes `prior`: a
/// verified line patch when the prior parses, else PR 5's render (a file
/// with no title cannot be patched; the render keeps every value the parser
/// still reads, and its run log). A refused patch is the error.
fn saved_content(slug: &str, prior: &str, input: &WorkflowInput) -> Result<String, String> {
    let (map, body) = split_frontmatter(prior);
    let old = frontmatter_from_map(&map);
    let mut new = old.clone();
    apply_input(&mut new, input);
    bound_tools_check(&new)?;
    end_rule_check(&new)?;
    if !old.title.is_empty() {
        // An unedited prompt keeps its bytes (leading indentation, trailing
        // whitespace, CRLF), so a description-only save is idempotent.
        let (prior_prompt, _) = split_body(&body);
        let prompt = (!same_prompt(&prior_prompt, &input.prompt)).then_some(input.prompt.as_str());
        patch_workflow(prior, &old, &new, prompt, None)
            .map_err(|e| format!("Could not save {WORKFLOWS_DIR}/{slug}.md: {e}"))
    } else {
        let (_, log) = split_body(&body);
        Ok(render(&new, &input.prompt, &log))
    }
}

/// Refuse a save whose quiet-skip configuration cannot work (Paperclip PR
/// 9b): a workflow that skips when quiet needs watch globs its check can walk
/// (`workflow_hygiene::quiet_config`). Checked when the editor sent
/// `skip_if_quiet` or `watch`, on a create, and whenever the save changes
/// what the check would watch (review: changing only `cwd` moves the default
/// watch or the base the globs must sit under). A save that leaves an
/// already-broken configuration exactly as it was is not refused, so an
/// unrelated edit still saves. Runs under the permission write guard, so it
/// reads only paths and the workspace brief: never the history lock or the
/// receipt store.
fn validate_quiet(
    vault: &Path,
    slug: &str,
    prior: Option<&str>,
    content: &str,
    input: &WorkflowInput,
) -> Result<(), String> {
    let Ok((workflow, _, _)) = parse_content(slug, content) else {
        return Ok(());
    };
    if !workflow.skip_if_quiet && input.skip_if_quiet.is_none() && input.watch.is_none() {
        return Ok(());
    }
    let saved = crate::workflow_hygiene::quiet_config(vault, &workflow);
    let sent = input.skip_if_quiet.is_some() || input.watch.is_some();
    let changed = match prior.map(|prior| parse_content(slug, prior)) {
        None => true,
        Some(Ok((before, _, _))) => crate::workflow_hygiene::quiet_config(vault, &before) != saved,
        Some(Err(_)) => true,
    };
    if !sent && !changed {
        return Ok(());
    }
    saved
        .map(|_| ())
        .map_err(|error| format!("Skip when nothing changed: {error}"))
}

/// The `notify` values the automation editor may send (Paperclip PR 9,
/// 9a-2), in the order its select lists them. `always` is the default.
pub const NOTIFY_VALUES: [&str; 3] = ["always", "failures", "never"];

/// What an editor-sent `notify` writes to the file: None (the key removed,
/// or kept absent) for `always`, the default; `failures` or `never`
/// otherwise. Trimmed and case-insensitive; any other value is refused, so
/// the editor can never write a value the runner would read as `always`
/// with a warning.
fn notify_for_file(value: &str) -> Result<Option<String>, String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "always" => Ok(None),
        "failures" | "never" => Ok(Some(normalized)),
        _ => Err(format!(
            "Notifications must be one of {} (got {value:?}).",
            NOTIFY_VALUES.join(", ")
        )),
    }
}

/// The editor's textarea normalizes line endings and may drop trailing
/// whitespace, so compare modulo both.
fn same_prompt(on_disk: &str, edited: &str) -> bool {
    on_disk.replace("\r\n", "\n").trim_end() == edited.replace("\r\n", "\n").trim_end()
}

/// A saved prompt as written into the body: trailing whitespace and leading
/// blank lines dropped (the parser drops those anyway), the first line's
/// indentation kept, and line breaks in the file's own style.
fn prompt_for_body(prompt: &str, nl: &str) -> String {
    let p = prompt.replace("\r\n", "\n");
    let p = p.trim_end();
    let ws = p.len() - p.trim_start().len();
    let start = p[..ws].rfind('\n').map_or(0, |i| i + 1);
    p[start..].replace('\n', nl)
}

/// True while `slug` has a live run: its stop slot is registered, or the
/// vault lease names its run. Neither takes a lock the caller could invert.
fn workflow_is_busy(vault: &Path, slug: &str) -> bool {
    crate::workflows::is_running(slug)
        || crate::automated_history::lease_holder(vault).is_some_and(|h| h.is_workflow(slug))
}

fn missing_prior_error(input: &WorkflowInput) -> String {
    let message = "This automation no longer exists, so nothing was saved. Reopen Automations.";
    if input.edit_base.is_some() {
        format!("{EDIT_CONFLICT_ERROR}: {message}")
    } else {
        message.into()
    }
}

/// The automation editor's baseline for a loaded workflow: sha256 hex of
/// `v1:` + the JSON of exactly the fields the editor sends (title,
/// description, schedule, agent, model, cwd, output_glob, permission_mode,
/// the tool lists, `notify`, `skip_if_quiet`, `watch` and the trimmed
/// prompt). Status, run-log and schedule bookkeeping (`last_*`,
/// `next_run_at`) and keys the editor never sends (`routine`, `triggers`, …)
/// are left out, so only an edit the save would overwrite can conflict. Compared only within one
/// editor session and never persisted, so its composition may change (9a-2
/// added `notify`, which the editor now sends when the user changes it).
pub fn edit_digest(w: &Workflow, prompt: &str) -> String {
    let owned = serde_json::json!({
        "title": w.title,
        "description": w.description,
        "schedule": w.schedule,
        "agent": w.agent,
        "model": w.model,
        "cwd": w.cwd,
        "outputGlob": w.output_glob,
        "permissionMode": w.permission_mode,
        "allowedTools": w.allowed_tools,
        "disallowedTools": w.disallowed_tools,
        "notify": w.notify,
        "skipIfQuiet": w.skip_if_quiet,
        "watch": w.watch,
        "precheck": w.precheck,
        "triggers": w.triggers,
        "inChat": w.in_chat,
        "ends": w.ends,
        "afterRuns": w.after_runs,
        "prompt": prompt.trim(),
    });
    let mut hasher = Sha256::new();
    hasher.update(b"v1:");
    hasher.update(serde_json::to_vec(&owned).unwrap_or_default());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
thread_local! {
    /// Runs once between an editor save's read and its write, on the saving
    /// thread only (race tests). It must not call `upsert*`/`set_schedule`,
    /// which take the write guard the save already holds.
    static BEFORE_WRITE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn set_before_write_hook(hook: impl FnOnce() + 'static) {
    BEFORE_WRITE_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_before_write_hook() {
    let hook = BEFORE_WRITE_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

// ── Line patches for existing files (Paperclip PR 9, 9a-1) ──────────────────

/// Every modelled key in render order, with the value a reader parses: None
/// when absent or empty, lists joined the way they round-trip through
/// `parse_list`, numbers in decimal, `skip_if_quiet` only when true, and the
/// parser's defaults for `schedule` and `permission_mode`.
fn modeled_fields(fm: &WorkflowFrontmatter) -> Vec<(&'static str, Option<String>)> {
    fn text(value: &str) -> Option<String> {
        (!value.is_empty()).then(|| value.to_string())
    }
    fn opt(value: &Option<String>) -> Option<String> {
        value.as_deref().and_then(text)
    }
    fn list(items: &[String]) -> Option<String> {
        text(&render_list(&parse_list(Some(render_list(items).as_str()))))
    }
    let schedule = if fm.schedule.is_empty() {
        "manual"
    } else {
        fm.schedule.as_str()
    };
    let mode = if fm.permission_mode.is_empty() {
        "acceptEdits"
    } else {
        fm.permission_mode.as_str()
    };
    vec![
        ("title", text(&fm.title)),
        ("description", text(&fm.description)),
        ("schedule", text(schedule)),
        ("agent", opt(&fm.agent)),
        ("model", opt(&fm.model)),
        ("cwd", opt(&fm.cwd)),
        ("output_glob", opt(&fm.output_glob)),
        ("routine", opt(&fm.routine)),
        ("permission_mode", text(mode)),
        ("allowed_tools", list(&fm.allowed_tools)),
        ("disallowed_tools", list(&fm.disallowed_tools)),
        ("triggers", list(&fm.triggers)),
        ("in_chat", opt(&fm.in_chat)),
        (crate::runs_on::KEY, opt(&fm.runs_on)),
        (
            "misfire_grace_min",
            fm.misfire_grace_min.map(|g| g.to_string()),
        ),
        (
            "skip_if_quiet",
            fm.skip_if_quiet.then(|| "true".to_string()),
        ),
        ("watch", list(&fm.watch)),
        ("precheck", opt(&fm.precheck)),
        ("notify", opt(&fm.notify)),
        ("ends", opt(&fm.ends)),
        ("after_runs", fm.after_runs.map(|n| n.to_string())),
        ("runs_counted", fm.runs_counted.map(|n| n.to_string())),
        ("ended_note", opt(&fm.ended_note)),
        ("created", opt(&fm.created)),
        ("last_started_at", opt(&fm.last_started_at)),
        ("last_run_at", opt(&fm.last_run_at)),
        ("last_status", opt(&fm.last_status)),
        ("last_error", opt(&fm.last_error)),
        (
            "last_duration_sec",
            fm.last_duration_sec.map(|d| d.to_string()),
        ),
        ("next_run_at", opt(&fm.next_run_at)),
    ]
}

/// The patch edits that turn `old` (parsed from `content`) into `new`: an
/// edit only for a key whose parsed value changed (`Set`, or `Remove` when
/// it becomes empty; `skip_if_quiet: true` as a bare-boolean `Block`).
/// `tags` is rewritten only when the schedule flips between manual and
/// scheduled; the second value is the tag list that edit writes.
fn frontmatter_edits(
    content: &str,
    old: &WorkflowFrontmatter,
    new: &WorkflowFrontmatter,
) -> (Vec<(&'static str, KeyEdit)>, Option<Vec<String>>) {
    let mut edits: Vec<(&'static str, KeyEdit)> = Vec::new();
    for ((key, before), (_, after)) in modeled_fields(old).into_iter().zip(modeled_fields(new)) {
        if before == after {
            continue;
        }
        let edit = match (key, after) {
            ("skip_if_quiet", Some(_)) => KeyEdit::Block(vec![SKIP_IF_QUIET_LINE.to_string()]),
            (_, Some(value)) => KeyEdit::Set(value),
            (_, None) => KeyEdit::Remove,
        };
        edits.push((key, edit));
    }
    let scheduled = |fm: &WorkflowFrontmatter| fm.schedule != "manual";
    let mut tags = None;
    if scheduled(old) != scheduled(new) {
        let items = schedule_tags(content, scheduled(new));
        edits.push(("tags", KeyEdit::Block(tag_block(&items))));
        tags = Some(items);
    }
    (edits, tags)
}

/// The tag list after a manual ↔ scheduled flip, keeping every user tag.
/// Reads the YAML list, else a scalar or flow value (`a, b` or `[a, b]`),
/// else nothing; swaps `schedule/manual` ↔ `schedule/scheduled` where the
/// old one was (right after `workflows` when neither was there) and makes
/// sure `workflows` is present.
fn schedule_tags(content: &str, scheduled: bool) -> Vec<String> {
    let current = crate::frontmatter::list_items(content, "tags").unwrap_or_else(|| {
        let (map, _) = split_frontmatter(content);
        let raw = map
            .get("tags")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let inner = raw
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .unwrap_or(raw.as_str());
        inner
            .split(',')
            .map(|t| crate::frontmatter::unquote(t.trim()))
            .filter(|t| !t.is_empty())
            .collect()
    });
    let wanted = if scheduled {
        "schedule/scheduled"
    } else {
        "schedule/manual"
    };
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    for tag in current {
        if tag == "schedule/manual" || tag == "schedule/scheduled" {
            if !placed {
                out.push(wanted.to_string());
                placed = true;
            }
            continue;
        }
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    if !out.iter().any(|t| t == "workflows") {
        out.insert(0, "workflows".into());
    }
    if !placed {
        let at = out
            .iter()
            .position(|t| t == "workflows")
            .map_or(0, |i| i + 1);
        out.insert(at, wanted.to_string());
    }
    out
}

/// A `tags:` YAML block list, items quoted like scalars when they need it.
fn tag_block(items: &[String]) -> Vec<String> {
    let mut lines = vec!["tags:".to_string()];
    for item in items {
        if crate::frontmatter::needs_quoting(item) {
            lines.push(format!("  - {}", crate::frontmatter::quote(item)));
        } else {
            lines.push(format!("  - {item}"));
        }
    }
    lines
}

/// Replace the prompt and/or the run log of an existing file and keep every
/// other byte.
///
/// The body starts where [`crate::frontmatter::parse`] says it does. Its
/// leading whitespace up to the last line break (`lead`) is kept, so a CRLF
/// file or a `--- ` fence with trailing spaces keeps its fence line (the
/// first prompt line's indentation is part of the prompt, not the lead). A
/// normal LF fence needs nothing: `parse` has already consumed its blank
/// lines, so the byte before the body is a newline. Only a fence glued to
/// text (`\n---X`, which `parse` accepts) gets a line break and a blank line.
/// The run log is found at [`split_body`]'s own index (the first
/// `## Run log`), so a prompt that mentions the header can never split
/// differently from what the parser reads. `run_log: Some` replaces
/// everything from that index with [`render_run_log`] (appended after the
/// prompt and a blank line when there is no header yet). `prompt: Some`
/// replaces only the text between `lead` and the index with the prompt
/// ([`prompt_for_body`]: trailing whitespace dropped, first-line indentation
/// kept, the file's line breaks) and a blank line. `None` keeps those bytes exactly, so a status
/// write never moves PR 5's `sha256(prompt.trim())`.
fn splice_body(content: &str, prompt: Option<&str>, run_log: Option<&[RunLogEntry]>) -> String {
    if prompt.is_none() && run_log.is_none() {
        return content.to_string();
    }
    let body_len = crate::frontmatter::parse(content).body.len();
    let (head, body) = content.split_at(content.len() - body_len);
    let nl = if head.contains("\r\n") { "\r\n" } else { "\n" };
    // The leading whitespace up to and including its last line break: the
    // fence line's own `\r\n` or trailing spaces, and blank lines. The first
    // prompt line's indentation after it belongs to the prompt.
    let ws_len = body.len() - body.trim_start_matches([' ', '\t', '\r', '\n']).len();
    let lead_len = body[..ws_len].rfind('\n').map_or(0, |i| i + 1);
    let (lead, rest) = body.split_at(lead_len);
    // `parse` has already consumed the blank lines after a normal fence, so
    // the byte before the body is a newline; only a glued fence needs one.
    // Plain `\n\n` even in a CRLF file: `parse` strips exactly that, so the
    // body (and the prompt a status write must not move) reads unchanged.
    let glue = if lead.is_empty() && !head.ends_with('\n') {
        "\n\n"
    } else {
        ""
    };
    let idx = rest.find(RUN_LOG_HEADER);
    let (old_prompt, old_log) = match idx {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let log = match run_log {
        Some(log) => render_run_log(log).replace('\n', nl),
        None => old_log.to_string(),
    };
    let prompt_part = match prompt {
        Some(p) => {
            let p = prompt_for_body(p, nl);
            if p.is_empty() {
                String::new()
            } else if log.is_empty() {
                format!("{p}{nl}")
            } else {
                format!("{p}{nl}{nl}")
            }
        }
        // No header yet: the new log goes after the prompt, a blank line apart.
        None if idx.is_none() && !log.is_empty() => {
            let p = old_prompt.trim_end();
            if p.is_empty() {
                String::new()
            } else {
                format!("{p}{nl}{nl}")
            }
        }
        None => old_prompt.to_string(),
    };
    format!("{head}{glue}{lead}{prompt_part}{log}")
}

/// Prove a patch says exactly what was meant, by re-parsing it:
/// - every key the write does NOT edit parses to the same value as before
///   (so removing a key whose span hid an indented `permission_mode:` line,
///   which would change what the runner reads, is refused);
/// - every modelled field parses to `new` (so a `Block` shadowed by a later
///   indented line of the same key is refused);
/// - the tag list is the intended one when tags were edited;
/// - the prompt is byte-identical when not edited, else the saved text.
///
/// The error names the key.
fn verify_patch(
    before: &str,
    after: &str,
    edited: &[&str],
    new: &WorkflowFrontmatter,
    tags: Option<&[String]>,
    prompt: Option<&str>,
) -> Result<(), String> {
    let (before_map, before_body) = split_frontmatter(before);
    let (after_map, after_body) = split_frontmatter(after);
    let keys: BTreeSet<&String> = before_map.keys().chain(after_map.keys()).collect();
    for key in keys {
        if !edited.contains(&key.as_str()) && before_map.get(key) != after_map.get(key) {
            return Err(format!(
                "`{key}` would read back changed although this write does not edit it (an indented line inside another frontmatter key is probably hiding it); file preserved"
            ));
        }
    }
    let have = modeled_fields(&frontmatter_from_map(&after_map));
    for ((key, want), (_, got)) in modeled_fields(new).iter().zip(have.iter()) {
        if want != got {
            return Err(format!(
                "`{key}` would read back as {got:?} instead of {want:?} (an indented line inside another frontmatter key is probably shadowing it); file preserved"
            ));
        }
    }
    if let Some(tags) = tags {
        if crate::frontmatter::list_items(after, "tags").as_deref() != Some(tags) {
            return Err("`tags` would not read back as the intended list; file preserved".into());
        }
    }
    let (after_prompt, _) = split_body(&after_body);
    match prompt {
        None if after_prompt != split_body(&before_body).0 => {
            Err("the prompt would change; file preserved".into())
        }
        Some(p) if !same_prompt(after_prompt.trim_start(), p.trim_start()) => {
            Err(if p.contains(RUN_LOG_HEADER) {
                RUN_LOG_IN_PROMPT_ERROR.into()
            } else {
                "the prompt would not read back as saved; file preserved".into()
            })
        }
        _ => Ok(()),
    }
}

/// Patch an existing workflow file from `old` (parsed from `content`) to
/// `new`: the frontmatter edits (none when nothing changed), then the body
/// splice, then [`verify_patch`]. Never re-renders.
fn patch_workflow(
    content: &str,
    old: &WorkflowFrontmatter,
    new: &WorkflowFrontmatter,
    prompt: Option<&str>,
    run_log: Option<&[RunLogEntry]>,
) -> Result<String, String> {
    let (edits, tags) = frontmatter_edits(content, old, new);
    let patched = if edits.is_empty() {
        content.to_string()
    } else {
        crate::frontmatter::patch(content, &edits, None)?
    };
    let out = splice_body(&patched, prompt, run_log);
    let edited: Vec<&str> = edits.iter().map(|(key, _)| *key).collect();
    verify_patch(content, &out, &edited, new, tags.as_deref(), prompt)?;
    Ok(out)
}

fn pick_target_filename(vault: &Path, title: &str, current_slug: &str) -> String {
    let desired = slugify(title);
    if desired == current_slug {
        return format!("{WORKFLOWS_DIR}/{current_slug}.md");
    }
    pick_filename(vault, title)
}

pub fn delete(vault: &Path, slug: &str) -> Result<(), String> {
    let _guard = crate::workflow_permissions::write_guard();
    let rel = format!("{WORKFLOWS_DIR}/{slug}.md");
    vault::delete_note(vault, &rel)?;
    // A deleted automation keeps no approval: re-creating the same slug by
    // hand needs a fresh review.
    crate::workflow_permissions::forget_locked(vault, slug);
    // Paperclip PR 9 (9a-2), still under the write guard (write guard →
    // review queue lock is the allowed order): its failure streak spoke for
    // a file that is gone, and a run in flight is not flagged for the
    // owner's own delete.
    crate::workflow_hygiene::close_streak(
        vault,
        slug,
        crate::workflow_hygiene::STREAK_DELETED_REASON,
    );
    crate::run_guard::note_owned_workflow_settings(vault, slug, None, None);
    // Its usage budgets go with it (Paperclip PR 8a); logged, never fails.
    crate::budgets::forget_workflow_scope(&crate::budgets::Store::for_app(vault), vault, slug);
    Ok(())
}

/// Mutate the run-status block of a workflow's frontmatter. Returns
/// the path that was written.
pub fn mark_status(
    vault: &Path,
    slug: &str,
    status: &str,
    error: Option<&str>,
    duration_sec: Option<u64>,
    next: Option<DateTime<Utc>>,
) -> Result<String, String> {
    mutate(vault, slug, WritePolicy::Bookkeeping, |fm, _log| {
        fm.last_run_at = Some(Utc::now().to_rfc3339());
        fm.last_status = Some(status.to_string());
        fm.last_error = error.map(|s| s.to_string());
        fm.last_duration_sec = duration_sec;
        if let Some(d) = next {
            fm.next_run_at = Some(d.to_rfc3339());
        }
    })
}

/// Stamp a dispatched run's start (Paperclip PR 9, 9a-2), called by
/// `workflows::execute` right before the agent spawns (after preparation and
/// the stop check), so a run that failed in preparation never advances it:
/// `last_started_at` and `last_run_at` = `at`, `last_status` = `running`,
/// `last_error` and `last_duration_sec` cleared. `last_run_at` keeps its
/// start-stamp meaning for the run queue's `WaitEpisode::ran_since` until
/// `finalize_run` stamps the completion time. A bookkeeping write: it never
/// moves a stamped permission field or a quiet setting, so the run's own
/// start is never flagged. Unlike the old `mark_status("running")`, the
/// caller treats an error as the run's failure (a file gone here means a
/// rename or delete slipped past admission).
pub fn mark_started(vault: &Path, slug: &str, at: DateTime<Utc>) -> Result<String, String> {
    let at = at.to_rfc3339();
    mutate(vault, slug, WritePolicy::Bookkeeping, |fm, _log| {
        fm.last_started_at = Some(at.clone());
        fm.last_run_at = Some(at);
        fm.last_status = Some("running".to_string());
        fm.last_error = None;
        fm.last_duration_sec = None;
    })
}

/// Heal workflows left stuck in `running` by a crash, a force-quit, or a
/// killed process.
///
/// `last_status` is only ever cleared by `finalize`, so any process that dies
/// mid-run leaves its workflow claiming to be running forever — and because the
/// UI reads `last_status`, that workflow shows a spinner that never resolves and
/// the failure-debugger never opens. `close-my-day.md` is in exactly that state
/// today.
///
/// Called at startup, after `automated_history::reconcile_interrupted`, and only
/// for slugs the in-memory running map does not know about — a genuinely live
/// run must not be marked interrupted underneath itself.
pub fn heal_orphan_running(vault: &Path, live: &[String]) -> Vec<String> {
    let mut healed = vec![];
    for w in list(vault) {
        if w.last_status.as_deref() != Some("running") || live.contains(&w.slug) {
            continue;
        }
        // Order matters: stamp the status first so a crash between the two
        // writes leaves the workflow correct rather than still "running".
        if mark_status(
            vault,
            &w.slug,
            "interrupted",
            Some("Hollow stopped before this run finished"),
            None,
            None,
        )
        .is_err()
        {
            continue;
        }
        let _ = append_run_log(
            vault,
            &w.slug,
            crate::types::RunLogEntry {
                at: Utc::now().to_rfc3339(),
                status: "interrupted".into(),
                duration_sec: 0,
                note: "stopped before it finished".into(),
                run_id: None,
                error_tail: None,
                exit_kind: None,
            },
        );
        healed.push(w.slug);
    }
    healed
}

/// Append a run-log entry, capping at `RUN_LOG_KEEP` newest entries.
pub fn append_run_log(vault: &Path, slug: &str, entry: RunLogEntry) -> Result<String, String> {
    mutate(vault, slug, WritePolicy::Bookkeeping, |_, log| {
        log.push(entry);
        if log.len() > RUN_LOG_KEEP {
            let drop = log.len() - RUN_LOG_KEEP;
            log.drain(0..drop);
        }
    })
}

/// Atomically persist the terminal status and matching run-log entry in one
/// read-modify-write. Keeping these together prevents a concurrent editor or
/// second finalizer from observing a terminal status without its audit line
/// (or vice versa).
///
/// `next` replaces `next_run_at` outright. `None` (a manual schedule, or a
/// cron that is exhausted or no longer parses) clears it: keeping the
/// occurrence that just ran would leave it due, and the scheduler would start
/// it again every tick until its grace ran out.
pub fn finalize_run(
    vault: &Path,
    slug: &str,
    status: &str,
    error: Option<&str>,
    duration_sec: u64,
    next: Option<DateTime<Utc>>,
    entry: RunLogEntry,
) -> Result<String, String> {
    finalize_run_schedule(
        vault,
        slug,
        status,
        error,
        duration_sec,
        Some((None, next)),
        entry,
    )
}

/// Record a run without consuming an independent schedule occurrence. Preserve
/// the stored next_run_at byte-for-byte, including an absent value.
pub(crate) fn finalize_run_preserving_schedule(
    vault: &Path,
    slug: &str,
    status: &str,
    error: Option<&str>,
    duration_sec: u64,
    entry: RunLogEntry,
) -> Result<String, String> {
    finalize_run_schedule(vault, slug, status, error, duration_sec, None, entry)
}

/// Advance only the exact owned occurrence, comparing the currently saved
/// value inside the workflow mutation owner. A newer occurrence is preserved.
pub(crate) fn finalize_run_for_occurrence(
    vault: &Path,
    slug: &str,
    status: &str,
    error: Option<&str>,
    duration_sec: u64,
    schedule: (&str, Option<DateTime<Utc>>),
    entry: RunLogEntry,
) -> Result<String, String> {
    finalize_run_schedule(
        vault,
        slug,
        status,
        error,
        duration_sec,
        Some((Some(schedule.0), schedule.1)),
        entry,
    )
}

fn finalize_run_schedule(
    vault: &Path,
    slug: &str,
    status: &str,
    error: Option<&str>,
    duration_sec: u64,
    next: Option<(Option<&str>, Option<DateTime<Utc>>)>,
    entry: RunLogEntry,
) -> Result<String, String> {
    // G2: a run that finishes an `after_runs` automation's last run ends it,
    // an owned write under the permission guard. The guard is taken up front
    // only when an unlocked peek sees the key (so an ordinary finalize never
    // touches it); the count itself is decided inside the locked closure from
    // the bytes it reads, and a file that gained the key after the peek is
    // counted by a second, guarded write below.
    let counts = fs::read_to_string(vault.join(format!("{WORKFLOWS_DIR}/{slug}.md")))
        .is_ok_and(|text| text.contains("after_runs:"));
    let guard = counts.then(crate::workflow_permissions::write_guard);
    let held = guard.is_some();
    let counts_run = status != "cancelled";
    let mut ended = false;
    let mut missed = false;
    let (rel, before, after) =
        mutate_capturing(vault, slug, WritePolicy::Bookkeeping, |fm, log| {
            fm.last_run_at = Some(entry.at.clone());
            fm.last_status = Some(status.to_string());
            fm.last_error = error.map(str::to_string);
            fm.last_duration_sec = Some(duration_sec);
            if let Some((expected, next)) = next {
                if expected.is_none() || fm.next_run_at.as_deref() == expected {
                    fm.next_run_at = next.map(|next| next.to_rfc3339());
                }
            }
            if fm.after_runs.is_some() && counts_run {
                if held {
                    let at = DateTime::parse_from_rfc3339(&entry.at)
                        .map(|at| at.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now());
                    ended = count_run(fm, at);
                } else {
                    missed = true;
                }
            }
            log.push(entry);
            if log.len() > RUN_LOG_KEEP {
                let drop = log.len() - RUN_LOG_KEEP;
                log.drain(0..drop);
            }
        })?;
    if ended {
        crate::workflow_permissions::after_schedule_write_locked(vault, slug, &before, &after);
    }
    drop(guard);
    if missed {
        let _guard = crate::workflow_permissions::write_guard();
        let mut ended = false;
        let (_, before, after) =
            mutate_capturing(vault, slug, WritePolicy::Bookkeeping, |fm, _| {
                if fm.after_runs.is_some() {
                    ended = count_run(fm, Utc::now());
                }
            })?;
        if ended {
            crate::workflow_permissions::after_schedule_write_locked(vault, slug, &before, &after);
        }
    }
    Ok(rel)
}

/// Count one finished run against `fm`'s `after_runs`; when the count is
/// reached, end it. Returns whether it ended. The caller holds the permission
/// write guard and re-stamps the schedule when this returns true.
fn count_run(fm: &mut WorkflowFrontmatter, at: DateTime<Utc>) -> bool {
    let Some(n) = fm.after_runs else {
        return false;
    };
    let done = fm.runs_counted.unwrap_or(0) + 1;
    fm.runs_counted = Some(done);
    if done >= n {
        apply_end(fm, &format!("it finished its {n} runs"), at);
        return true;
    }
    false
}

/// Replace `next_run_at` on a workflow without touching last_run_*. Used
/// by the scheduler to repair a stale or missing nextRunAt at start-up.
pub fn refresh_next_run(vault: &Path, slug: &str) -> Result<String, String> {
    mutate(vault, slug, WritePolicy::Bookkeeping, |fm, _| {
        fm.next_run_at = next_run_after(&fm.schedule, Utc::now()).map(|d| d.to_rfc3339());
    })
}

/// Skip one scheduled occurrence: advance `next_run_at` to the schedule's
/// next occurrence after `now` and record `entry` in the run log, in one
/// mutation. last_run_* is untouched because no run happened. The caller
/// chooses the entry, so a genuine misfire and a lease-contention skip are
/// told apart in the log; the caller must also hold a lease for the write
/// (the scheduler's bookkeeping lease), or it lands in another run's capture.
///
/// A budget skip ([`budget_skip_entry`], Paperclip PR 8a) directly after
/// another budget skip of the SAME pause episode only advances: consecutive
/// occurrences blocked by one episode read as ONE row (the first), so a long
/// pause never floods the 50-row log. A new episode (the pause lapsed and the
/// budget tripped again) or anything else in between (a run, another skip)
/// starts a new row. Decided here, under the caller's lease, so a busy lease
/// never loses the first row.
pub fn skip_scheduled_run(
    vault: &Path,
    slug: &str,
    entry: RunLogEntry,
    now: DateTime<Utc>,
) -> Result<String, String> {
    mutate(vault, slug, WritePolicy::Bookkeeping, |fm, log| {
        fm.next_run_at = next_run_after(&fm.schedule, now).map(|d| d.to_rfc3339());
        if is_budget_skip(&entry)
            && log
                .last()
                .is_some_and(|last| is_budget_skip(last) && same_budget_episode(last, &entry))
        {
            return;
        }
        log.push(entry);
        if log.len() > RUN_LOG_KEEP {
            let drop = log.len() - RUN_LOG_KEEP;
            log.drain(0..drop);
        }
    })
}

/// The plain misfire row, for an occurrence missed while Hollow was closed or
/// the machine slept, when no contention episode explains the skip. Used ONLY
/// for genuine misfires, never for lease contention (which names the holder).
/// It replaced `skip_missed_run`, whose unconditional note mislabelled
/// contention losers.
pub fn missed_while_closed_entry(now: DateTime<Utc>) -> RunLogEntry {
    note_entry(SKIPPED_STATUS, MISSED_WHILE_CLOSED_NOTE.to_string(), now)
}

/// A deferred occurrence finally started: it waited `waited_min` minutes over
/// `attempts` refused admissions for `holder`. Written inside the run's own
/// lease, so the row is attributed to the run that it describes.
pub fn deferred_start_entry(
    holder: &str,
    waited_min: i64,
    attempts: u32,
    now: DateTime<Utc>,
) -> RunLogEntry {
    note_entry(
        DEFERRED_STATUS,
        format!(
            "started after waiting {} min for {} ({})",
            waited_min.max(0),
            note_holder(holder),
            deferral_count(attempts)
        ),
        now,
    )
}

/// A deferred occurrence passed its grace window while Hollow stayed open:
/// another writer held the vault the whole time.
pub fn busy_skip_entry(
    holder: &str,
    grace_min: u64,
    attempts: u32,
    now: DateTime<Utc>,
) -> RunLogEntry {
    note_entry(
        SKIPPED_STATUS,
        format!(
            "not run: {} held the vault past the {grace_min}-minute grace ({})",
            note_holder(holder),
            deferral_count(attempts)
        ),
        now,
    )
}

/// A deferred occurrence stopped being due (its schedule or trigger changed)
/// before the vault became free.
pub fn deferral_cleared_entry(holder: &str, now: DateTime<Utc>) -> RunLogEntry {
    note_entry(
        SKIPPED_STATUS,
        format!(
            "not run: no longer due after waiting for {}",
            note_holder(holder)
        ),
        now,
    )
}

/// A deferred occurrence whose episode went quiet, meaning Hollow closed or
/// the machine slept while it waited: both facts are stated.
pub fn abandoned_deferral_entry(holder: &str, now: DateTime<Utc>) -> RunLogEntry {
    note_entry(
        SKIPPED_STATUS,
        format!(
            "{MISSED_WHILE_CLOSED_NOTE} (it had been waiting for {})",
            note_holder(holder)
        ),
        now,
    )
}

fn note_entry(status: &str, note: String, now: DateTime<Utc>) -> RunLogEntry {
    RunLogEntry {
        at: now.to_rfc3339(),
        status: status.to_string(),
        duration_sec: 0,
        note,
        run_id: None,
        error_tail: None,
        exit_kind: None,
    }
}

fn deferral_count(attempts: u32) -> String {
    let attempts = attempts.max(1);
    if attempts == 1 {
        "1 deferral".into()
    } else {
        format!("{attempts} deferrals")
    }
}

/// A due occurrence skipped past its grace because the admission precheck
/// refused the workflow as written. A refusal outranks both the contention
/// notes and the misfire note, so neither is used. The producer is Paperclip
/// PR 5's permission verification, whose reason is
/// [`PERMISSION_REFUSED_REASON`] (or [`PERMISSION_RECORDS_UNAVAILABLE_REASON`]).
pub fn refused_skip_entry(reason: &str, now: DateTime<Utc>) -> RunLogEntry {
    note_entry(
        SKIPPED_STATUS,
        format!(
            "not run: {}",
            note_fragment(reason, "the workflow did not pass its checks")
        ),
        now,
    )
}

/// A due occurrence skipped past its grace because the workflow could not
/// start as written (an empty prompt, say). Hollow was open the whole time,
/// so this is never the misfire note, and nothing waited on the lease.
pub fn invalid_skip_entry(error: &str, now: DateTime<Utc>) -> RunLogEntry {
    note_entry(
        SKIPPED_STATUS,
        format!(
            "not run: {}",
            note_fragment(error, "the workflow could not start")
        ),
        now,
    )
}

/// Every budget skip note starts with this (after `not run: `).
pub const BUDGET_SKIP_NOTE_PREFIX: &str = "not run: budget paused";

/// A due occurrence a usage budget blocked at claim (Paperclip PR 8a): a
/// hard-stop pause or a hard-stop budget at its limit holds the automation.
/// Consumed (never run late) and logged once per pause episode
/// ([`skip_scheduled_run`]). `detail` is the budget's short summary (scope
/// and amounts); `episode` is when the pause began (RFC 3339), named in the
/// note as `since <local time>` so each episode gets its own row. Sanitized
/// like every note.
pub fn budget_skip_entry(detail: &str, episode: Option<&str>, now: DateTime<Utc>) -> RunLogEntry {
    let since = episode
        .and_then(|at| DateTime::parse_from_rfc3339(at.trim()).ok())
        .map(|at| {
            format!(
                " since {}",
                at.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M")
            )
        })
        .unwrap_or_default();
    note_entry(
        SKIPPED_STATUS,
        format!(
            "{BUDGET_SKIP_NOTE_PREFIX}{since} ({})",
            note_fragment(detail, "a usage budget holds this automation")
        ),
        now,
    )
}

fn is_budget_skip(entry: &RunLogEntry) -> bool {
    entry.status == SKIPPED_STATUS && entry.note.starts_with(BUDGET_SKIP_NOTE_PREFIX)
}

/// Two budget skip rows name the same pause episode: the note up to the
/// detail (`not run: budget paused since …`) is equal.
fn same_budget_episode(a: &RunLogEntry, b: &RunLogEntry) -> bool {
    fn key(note: &str) -> &str {
        note.split_once(" (").map_or(note, |(head, _)| head)
    }
    key(&a.note) == key(&b.note)
}

/// The note of a scheduled occurrence skipped because nothing it watches
/// changed since its last run (Paperclip PR 9b, `skip_if_quiet`). A benign
/// skip ([`is_benign_skip_note`]): not a failure in the activity ledger.
/// Never starts with `not run:` and never mentions a misfire.
pub const QUIET_SKIP_NOTE: &str = "no changes since last run";
/// Every inactive-workspace skip note starts with this (Paperclip PR 9b): the
/// workflow works in a workspace whose status is not current.
pub const INACTIVE_SKIP_PREFIX: &str = "workspace inactive:";
/// Separates an idle-skip note from its coalesced count (`· 3 skips`).
const IDLE_SKIP_COUNT_SEP: &str = " · ";

/// Why a scheduled occurrence was skipped with nothing to do (Paperclip PR
/// 9b). Written by [`skip_idle_run`], which coalesces consecutive skips of
/// the same kind into one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdleSkip {
    /// `skip_if_quiet`: nothing under the watch globs changed since the last
    /// successful run started.
    Quiet,
    /// The workflow's `cwd` is a workspace whose status is not current.
    Inactive { workspace: String, status: String },
    /// The automation's `precheck` command said not to run (OC13,
    /// `workflow_precheck`). `None`: it ran and exited with a clean non-zero
    /// status (a benign skip). `Some(why)`: it is broken (could not start,
    /// exit 126 or 127, a signal, timeout, stop, changed script): fail closed,
    /// recorded as a failure with the reason.
    Precheck { unfinished: Option<String> },
}

impl IdleSkip {
    /// The row's note (without a coalesced count). One line, no em dash, no
    /// `<`/`>`, like every run-log note.
    pub fn note(&self) -> String {
        match self {
            IdleSkip::Quiet => QUIET_SKIP_NOTE.to_string(),
            IdleSkip::Inactive { workspace, status } => format!(
                "{INACTIVE_SKIP_PREFIX} {} is {}",
                note_fragment(workspace, "its workspace"),
                note_fragment(status, "not current")
            ),
            IdleSkip::Precheck { unfinished: None } => {
                crate::workflow_precheck::FAILED_NOTE.to_string()
            }
            IdleSkip::Precheck {
                unfinished: Some(why),
            } => format!(
                "{} ({})",
                crate::workflow_precheck::UNRUNNABLE_NOTE_PREFIX,
                note_fragment(why, "no reason given")
            ),
        }
    }

    /// The note after `count` consecutive skips of this kind.
    pub fn coalesced_note(&self, count: u32) -> String {
        if count <= 1 {
            self.note()
        } else {
            format!("{}{IDLE_SKIP_COUNT_SEP}{count} skips", self.note())
        }
    }

    /// A broken precheck (OC13): recorded as an `error` row and a failed
    /// queue row, not a skip. Never coalesced, so each one counts toward the
    /// failure streak that opens the Workflows warning.
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            IdleSkip::Precheck {
                unfinished: Some(_)
            }
        )
    }

    /// A fresh row for this skip: `skipped`, or `error` for a failure.
    pub fn entry(&self, now: DateTime<Utc>) -> RunLogEntry {
        let status = if self.is_failure() {
            ERROR_STATUS
        } else {
            SKIPPED_STATUS
        };
        note_entry(status, self.note(), now)
    }

    /// How many skips `entry` already stands for when it is a row of this
    /// same skip (same kind, and for an inactive skip the same workspace and
    /// status), else None. A row with a run id is a run, never a skip.
    fn count_in(&self, entry: &RunLogEntry) -> Option<u32> {
        if self.is_failure() {
            return None;
        }
        if entry.status != SKIPPED_STATUS || entry.run_id.is_some() {
            return None;
        }
        let base = self.note();
        let rest = entry.note.strip_prefix(base.as_str())?;
        if rest.is_empty() {
            return Some(1);
        }
        rest.strip_prefix(IDLE_SKIP_COUNT_SEP)?
            .strip_suffix(" skips")?
            .parse::<u32>()
            .ok()
            .filter(|count| *count >= 2)
    }
}

/// What [`skip_idle_run`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleSkipWrite {
    /// The skip was folded into the newest row (its `at` and count moved).
    pub coalesced: bool,
    /// False when the occurrence was no longer the workflow's `next_run_at`
    /// (it ran or was rescheduled meanwhile): nothing was written.
    pub written: bool,
}

/// Skip one scheduled occurrence with nothing to do (Paperclip PR 9b): the
/// same `next_run_at` advance as [`skip_scheduled_run`], and one run-log row
/// per streak of the same idle skip: when the newest row is already this
/// skip, its `at` moves to `now` and its note counts the skips
/// (`no changes since last run · 3 skips`); anything else in between (a run,
/// another kind of skip, an inactive skip for another status) starts a new
/// row. Coalescing keeps the 50-row log from filling with skips, which would
/// push out the last run's row and with it the run id the quiet check reads.
///
/// `expect_due` is the claimed row's occurrence: when the file's
/// `next_run_at` no longer equals it (a run consumed it, the schedule
/// changed), nothing is written. The caller holds the scheduler's
/// manifest-free bookkeeping lease, so no run is capturing the vault and the
/// write creates no history run. A bookkeeping write: it never touches a
/// stamped permission field or a quiet setting, so it neither re-stamps nor
/// needs the permission write guard.
pub fn skip_idle_run(
    vault: &Path,
    slug: &str,
    skip: &IdleSkip,
    expect_due: Option<&str>,
    now: DateTime<Utc>,
) -> Result<IdleSkipWrite, String> {
    if let Some(due) = expect_due {
        let (workflow, _, _) = read_full(vault, slug)?;
        if workflow.next_run_at.as_deref() != Some(due) {
            return Ok(IdleSkipWrite {
                coalesced: false,
                written: false,
            });
        }
    }
    let mut result = IdleSkipWrite {
        coalesced: false,
        written: false,
    };
    mutate(vault, slug, WritePolicy::Bookkeeping, |fm, log| {
        // Re-checked under MUTATE_LOCK: an editor save may have moved it.
        if expect_due.is_some_and(|due| fm.next_run_at.as_deref() != Some(due)) {
            return;
        }
        fm.next_run_at = next_run_after(&fm.schedule, now).map(|d| d.to_rfc3339());
        result.written = true;
        if let Some(last) = log.last_mut() {
            if let Some(count) = skip.count_in(last) {
                last.at = now.to_rfc3339();
                last.note = skip.coalesced_note(count.saturating_add(1));
                result.coalesced = true;
                return;
            }
        }
        log.push(skip.entry(now));
        if log.len() > RUN_LOG_KEEP {
            let drop = log.len() - RUN_LOG_KEEP;
            log.drain(0..drop);
        }
    })?;
    Ok(result)
}

/// Whether a `skipped` row's note is a benign idle skip (a quiet or an
/// inactive-workspace skip), which the activity ledger does not count as a
/// failure. A precheck that ran and exited with a clean non-zero status is
/// benign too (the person's own condition said there was nothing to do); a
/// broken one (not found, not executable, signal, timeout, stop, changed
/// script) is an `error` row and never reaches this. Refusals (`not run: …`) and misfires never match.
pub fn is_benign_skip_note(note: &str) -> bool {
    note.starts_with(QUIET_SKIP_NOTE)
        || note.starts_with(INACTIVE_SKIP_PREFIX)
        || note.starts_with(crate::workflow_precheck::FAILED_NOTE)
}

/// A queued cron occurrence the user skipped from the run queue (Paperclip
/// PR 7: Skip on a queued scheduled row). Written at claim, under the
/// bookkeeping lease, when the occurrence is still the workflow's next run.
pub fn queue_cancelled_entry(now: DateTime<Utc>) -> RunLogEntry {
    note_entry(
        SKIPPED_STATUS,
        "not run: skipped from the run queue".to_string(),
        now,
    )
}

/// The holder name inside a contention note.
fn note_holder(holder: &str) -> String {
    note_fragment(holder, "another automated run")
}

/// Keep text safe inside a run-log line: one line, no em dash (the row
/// separator), no `<`/`>` (so it can never open or close a hidden marker),
/// bounded; `fallback` when nothing printable is left. Holder text normally
/// comes from `LeaseHolder::describe`, but the deferral file is
/// hand-editable, so it is sanitized here as well.
fn note_fragment(text: &str, fallback: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '—' | '<' | '>') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let bounded: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(NOTE_FRAGMENT_MAX_CHARS)
        .collect();
    if bounded.is_empty() {
        fallback.into()
    } else {
        bounded
    }
}

/// Set a workflow's `schedule` (and recompute `next_run_at`) without
/// disturbing the prompt body, run log, or any other frontmatter: a line
/// patch of `schedule`, `next_run_at` and, on a manual ↔ scheduled flip, the
/// `schedule/*` tag (user tags survive). Used by the Automations page
/// routine toggles to flip a routine between `manual` and a cron. `schedule`
/// is validated first (accepts `manual` or a 7-field cron). An owned write:
/// when the patch is refused (a hand-written indented `schedule:` line would
/// shadow it) the error is returned and the file is left untouched. Returns
/// the updated `Workflow` summary.
pub fn set_schedule(vault: &Path, slug: &str, schedule: &str) -> Result<Workflow, String> {
    validate_schedule(schedule)?;
    // Owned writer: the schedule write and its stamp update are one step for
    // any concurrent verify (see `workflow_permissions::write_guard`).
    let _guard = crate::workflow_permissions::write_guard();
    let (_, before, after) = mutate_capturing(vault, slug, WritePolicy::Owned, |fm, _| {
        fm.schedule = schedule.to_string();
        fm.next_run_at = next_run_after(schedule, Utc::now()).map(|d| d.to_rfc3339());
    })?;
    // Updates only the schedule component of an otherwise-matching stamp; a
    // routine toggle never approves any other change.
    crate::workflow_permissions::after_schedule_write_locked(vault, slug, &before, &after);
    read_full(vault, slug).map(|(w, _, _)| w)
}

/// M4: set `slug`'s owner Mac (`runs_on`; None removes it) as an owned
/// write, moving only the owner of an otherwise-matching permission stamp
/// (`after_owner_write_locked`). For this Mac's slug rename
/// (`runs_on::after_slug_change`); a person's change goes through the save.
pub fn set_owner(vault: &Path, slug: &str, owner: Option<&str>) -> Result<(), String> {
    let _guard = crate::workflow_permissions::write_guard();
    let (_, before, after) = mutate_capturing(vault, slug, WritePolicy::Owned, |fm, _| {
        fm.runs_on = owner.map(str::to_string);
    })?;
    crate::workflow_permissions::after_owner_write_locked(vault, slug, &before, &after);
    Ok(())
}

/// Status, run-log and schedule writers: a read-modify-write of an existing
/// workflow file under `vault::MUTATE_LOCK`, written as a verified line patch
/// (`patch_workflow`). Only the modelled keys whose parsed value `f` changed
/// are rewritten, the run log is spliced only when `f` changed it, and the
/// prompt bytes are never touched, so no write through here can move a
/// stamped permission field (the status and run-log writers never re-stamp).
/// When the patch is refused, [`WritePolicy::Bookkeeping`] logs it and writes
/// the canonical render instead, which re-emits every parsed value (so the
/// stamp still holds); [`WritePolicy::Owned`] returns the error and writes
/// nothing.
fn mutate(
    vault: &Path,
    slug: &str,
    policy: WritePolicy,
    f: impl FnOnce(&mut WorkflowFrontmatter, &mut Vec<RunLogEntry>),
) -> Result<String, String> {
    mutate_capturing(vault, slug, policy, f).map(|(rel, _, _)| rel)
}

/// [`mutate`] that also returns the exact (before, after) file text of the
/// read-modify-write, for the owned writers that must re-stamp.
fn mutate_capturing(
    vault: &Path,
    slug: &str,
    policy: WritePolicy,
    f: impl FnOnce(&mut WorkflowFrontmatter, &mut Vec<RunLogEntry>),
) -> Result<(String, String, String), String> {
    let rel = format!("{WORKFLOWS_DIR}/{slug}.md");
    let mut before = String::new();
    let note = vault::mutate_note(vault, &rel, |content| {
        before = content.to_string();
        let (map, body) = split_frontmatter(content);
        let title = map
            .get("title")
            .cloned()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("workflow {slug} has no title"))?;
        let mut old = frontmatter_from_map(&map);
        old.title = title;
        let (prompt, old_log) = split_body(&body);
        let mut new = old.clone();
        let mut log = old_log.clone();
        f(&mut new, &mut log);
        let changed_log = (log != old_log).then_some(log.as_slice());
        match patch_workflow(content, &old, &new, None, changed_log) {
            Ok(patched) => Ok(patched),
            Err(e) => match policy {
                WritePolicy::Bookkeeping => {
                    eprintln!(
                        "[workflows] {slug}: frontmatter patch refused ({e}); writing canonical form"
                    );
                    Ok(render(&new, &prompt, &log))
                }
                WritePolicy::Owned => Err(format!("Could not update {rel}: {e}")),
            },
        }
    })?;
    Ok((rel, before, note.content))
}

/// The shared workflow test fixture for scheduler, admission and deferral
/// tests in every module. It writes through the normal writers (`upsert`,
/// then one `WritePolicy::Bookkeeping` line patch) rather than a raw
/// `fs::write`, so a later writer-side
/// invariant is added here once instead of in each test. Seeded `manual`,
/// then given `schedule` verbatim: that lets a test seed an unparseable cron,
/// which `upsert` rightly rejects. `next_run_at` is computed from the real
/// clock; `edit` may override any field.
///
/// Permission stamps (Paperclip PR 5): the vault is trusted first, as at
/// startup, and the finished file is stamped as a Hollow-created automation,
/// because `edit` (triggers, schedule) goes through `mutate`, which never
/// re-stamps. Without that every seeded workflow would be refused by
/// `workflow_permissions::verify`. A test that wants a tampered workflow
/// writes the file directly afterwards.
#[cfg(test)]
pub(crate) fn seed_workflow_with(
    vault: &Path,
    title: &str,
    schedule: &str,
    prompt: &str,
    edit: impl FnOnce(&mut WorkflowFrontmatter),
) -> Workflow {
    crate::workflow_permissions::initialize(vault).expect("trust the test vault");
    let seeded = upsert(
        vault,
        None,
        title,
        "",
        "manual",
        None,
        None,
        None,
        "acceptEdits",
        &[],
        &[],
        None,
        prompt,
    )
    .expect("seed workflow");
    {
        let _guard = crate::workflow_permissions::write_guard();
        let (_, _, content) =
            mutate_capturing(vault, &seeded.slug, WritePolicy::Bookkeeping, |fm, _| {
                fm.schedule = schedule.to_string();
                fm.next_run_at = next_run_after(schedule, Utc::now()).map(|d| d.to_rfc3339());
                edit(fm);
            })
            .expect("configure seeded workflow");
        crate::workflow_permissions::stamp_created_locked(vault, &seeded.slug, &content, false);
    }
    read_full(vault, &seeded.slug)
        .map(|(workflow, _, _)| workflow)
        .expect("read seeded workflow")
}

/// [`seed_workflow_with`] with no further edits.
#[cfg(test)]
pub(crate) fn seed_workflow(vault: &Path, title: &str, schedule: &str, prompt: &str) -> Workflow {
    seed_workflow_with(vault, title, schedule, prompt, |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_vault() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn slugify_handles_punctuation_and_unicode() {
        assert_eq!(slugify("Daily 7am Brief"), "daily-7am-brief");
        assert_eq!(slugify("Morning, Brief!"), "morning-brief");
        assert_eq!(slugify(""), "workflow");
    }

    #[test]
    fn validate_schedule_accepts_manual_and_cron() {
        assert!(validate_schedule("manual").is_ok());
        assert!(validate_schedule("0 0 7 * * * *").is_ok());
        assert!(validate_schedule("not-cron").is_err());
    }

    /// B04: a schedule `validate_schedule` refuses never fires, so the
    /// workflow carries the reason (`schedule_problem`) for the Automations
    /// row and detail pane. `list` and `read_full` both build it in
    /// `build_workflow`, so they agree.
    #[test]
    fn a_schedule_that_cannot_be_read_is_reported_by_list_and_read_full() {
        let dir = temp_vault();
        let v = dir.path();
        fs::create_dir_all(v.join(WORKFLOWS_DIR)).unwrap();
        let cases = [
            ("broken", "not-cron", true),
            ("unknown-window", "window:no-such-window-b04", true),
            ("daily", "0 0 7 * * * *", false),
            ("by-hand", "manual", false),
        ];
        for (slug, schedule, _) in cases {
            fs::write(
                v.join(format!("{WORKFLOWS_DIR}/{slug}.md")),
                format!("---\ntitle: {slug}\nschedule: {schedule}\n---\nBODY\n"),
            )
            .unwrap();
        }
        let listed = list(v);
        assert_eq!(listed.len(), cases.len());
        for (slug, schedule, broken) in cases {
            let from_list = listed.iter().find(|w| w.slug == slug).unwrap();
            let (from_read, _, _) = read_full(v, slug).unwrap();
            assert_eq!(from_list.schedule, schedule);
            assert_eq!(from_list.schedule_problem.is_some(), broken, "{slug}");
            assert_eq!(
                from_list.schedule_problem, from_read.schedule_problem,
                "{slug}"
            );
            assert_eq!(
                from_list.schedule_problem,
                validate_schedule(schedule).err(),
                "{slug}"
            );
        }
    }

    #[test]
    fn next_run_after_is_some_for_cron_and_none_for_manual() {
        let now = Utc::now();
        assert!(next_run_after("manual", now).is_none());
        let next = next_run_after("0 0 7 * * * *", now).unwrap();
        assert!(next > now);
    }

    #[test]
    fn next_run_after_interprets_hour_in_local_time() {
        use chrono::Timelike;
        // Cron hour is a LOCAL wall-clock hour. Whatever the test machine's tz,
        // the next fire converted back to Local must land on the cron's hour
        // (regression guard: evaluating against UTC fired routines hours early).
        let now = Utc::now();
        for h in [7u32, 8, 16, 18] {
            let next = next_run_after(&format!("0 0 {h} * * * *"), now).unwrap();
            let local = next.with_timezone(&chrono::Local);
            assert_eq!(local.hour(), h, "cron hour {h} should fire at {h}:00 local");
            assert_eq!(local.minute(), 0);
            assert!(next > now);
        }
    }

    /// next-level P17: the editor's "next three runs" (`src/workflowSchedule.ts`
    /// `nextRuns`) mirrors how the `cron` crate resolves a schedule in a zone
    /// with daylight saving: a local time that does not exist (spring
    /// forward) or that exists twice (fall back) is skipped for that day,
    /// because each candidate is built with chrono's `.single()`. Pinned on a
    /// toy US-Eastern-like zone (UTC-5, UTC-4 between 2027-03-14 07:00Z and
    /// 2027-11-07 06:00Z) so the test never depends on the machine's zone or
    /// touches `TZ`; a crate upgrade that changes this breaks it, and the TS
    /// mirror (`workflowNextRuns.test.ts`) must change with it.
    #[test]
    fn cron_skips_times_a_daylight_saving_change_removes_or_repeats() {
        use chrono::{
            Datelike, FixedOffset, MappedLocalTime, NaiveDate, NaiveDateTime, TimeZone, Timelike,
        };
        #[derive(Clone, Copy, Debug)]
        struct ToyEastern;
        fn dst_window() -> (i64, i64) {
            (
                Utc.with_ymd_and_hms(2027, 3, 14, 7, 0, 0)
                    .unwrap()
                    .timestamp(),
                Utc.with_ymd_and_hms(2027, 11, 7, 6, 0, 0)
                    .unwrap()
                    .timestamp(),
            )
        }
        fn in_dst(ts: i64) -> bool {
            let (start, end) = dst_window();
            ts >= start && ts < end
        }
        fn std_off() -> FixedOffset {
            FixedOffset::west_opt(5 * 3600).unwrap()
        }
        fn dst_off() -> FixedOffset {
            FixedOffset::west_opt(4 * 3600).unwrap()
        }
        impl TimeZone for ToyEastern {
            type Offset = FixedOffset;
            fn from_offset(_: &FixedOffset) -> Self {
                ToyEastern
            }
            fn offset_from_local_date(&self, local: &NaiveDate) -> MappedLocalTime<FixedOffset> {
                self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
            }
            fn offset_from_local_datetime(
                &self,
                local: &NaiveDateTime,
            ) -> MappedLocalTime<FixedOffset> {
                let wall = local.and_utc().timestamp();
                let as_std = !in_dst(wall + 5 * 3600);
                let as_dst = in_dst(wall + 4 * 3600);
                match (as_std, as_dst) {
                    (true, true) => MappedLocalTime::Ambiguous(dst_off(), std_off()),
                    (true, false) => MappedLocalTime::Single(std_off()),
                    (false, true) => MappedLocalTime::Single(dst_off()),
                    (false, false) => MappedLocalTime::None,
                }
            }
            fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
                self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
            }
            fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
                if in_dst(utc.and_utc().timestamp()) {
                    dst_off()
                } else {
                    std_off()
                }
            }
        }
        let days = |schedule: &str, from: chrono::DateTime<ToyEastern>| -> Vec<(u32, u32, u32)> {
            Schedule::from_str(schedule)
                .unwrap()
                .after(&from)
                .take(3)
                .map(|d| (d.day(), d.hour(), d.minute()))
                .collect()
        };
        // Spring forward: 02:30 does not exist on Mar 14.
        let spring = ToyEastern.with_ymd_and_hms(2027, 3, 12, 12, 0, 0).unwrap();
        assert_eq!(
            days("0 30 2 * * * *", spring),
            vec![(13, 2, 30), (15, 2, 30), (16, 2, 30)]
        );
        // Fall back: 01:30 happens twice on Nov 7, so it is skipped too.
        let fall = ToyEastern.with_ymd_and_hms(2027, 11, 5, 12, 0, 0).unwrap();
        assert_eq!(
            days("0 30 1 * * * *", fall),
            vec![(6, 1, 30), (8, 1, 30), (9, 1, 30)]
        );
        // Times clear of the change are unaffected, and keep their wall hour.
        assert_eq!(
            days("0 0 7 * * * *", spring),
            vec![(13, 7, 0), (14, 7, 0), (15, 7, 0)]
        );
        // Strictly after: a run due exactly now is not the next one.
        let seven = ToyEastern.with_ymd_and_hms(2027, 3, 13, 7, 0, 0).unwrap();
        assert_eq!(days("0 0 7 * * * *", seven)[0], (14, 7, 0));
    }

    #[test]
    fn set_schedule_flips_cron_and_preserves_body() {
        let dir = temp_vault();
        let v = dir.path();
        let w = upsert(
            v,
            None,
            "Routine X",
            "desc",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "PROMPT BODY HERE",
        )
        .unwrap();
        assert_eq!(w.schedule, "manual");
        assert!(w.next_run_at.is_none());

        let w2 = set_schedule(v, &w.slug, "0 0 7 * * * *").unwrap();
        assert_eq!(w2.schedule, "0 0 7 * * * *");
        assert!(w2.next_run_at.is_some());
        // Body survives the schedule flip.
        let (_, prompt, _) = read_full(v, &w.slug).unwrap();
        assert!(prompt.contains("PROMPT BODY HERE"));

        // Back to manual clears next_run_at.
        let w3 = set_schedule(v, &w.slug, "manual").unwrap();
        assert_eq!(w3.schedule, "manual");
        assert!(w3.next_run_at.is_none());

        // Invalid cron is rejected.
        assert!(set_schedule(v, &w.slug, "not-cron").is_err());
    }

    #[test]
    fn routine_marker_survives_an_edit() {
        let dir = temp_vault();
        let v = dir.path();
        let w = upsert(
            v,
            None,
            "Brief",
            "d",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        // Mark it a routine by hand (the retired seed used to do this).
        let file = v.join(&w.path);
        let text = fs::read_to_string(&file).unwrap();
        fs::write(
            &file,
            text.replacen("---\n", "---\nroutine: morning-brief\n", 1),
        )
        .unwrap();
        // Edit the workflow in place (Workflows-tab editor path) — the routine
        // marker must be preserved even though upsert never takes it as input.
        let edited = upsert(
            v,
            Some(&w.slug),
            "Brief",
            "new desc",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "NEW BODY",
        )
        .unwrap();
        assert_eq!(edited.routine.as_deref(), Some("morning-brief"));
    }

    #[test]
    fn triggers_parse_render_and_survive_an_edit() {
        let dir = temp_vault();
        let v = dir.path();
        // Hand-write a workflow with a triggers list (Obsidian / hand-edit path).
        fs::create_dir_all(v.join(WORKFLOWS_DIR)).unwrap();
        fs::write(
            v.join(format!("{WORKFLOWS_DIR}/watcher.md")),
            "---\ntitle: Watcher\nschedule: manual\ntriggers: email-from:boss@acme.com, task-overdue\n---\nBODY\n",
        )
        .unwrap();
        let (w, _, _) = read_full(v, "watcher").unwrap();
        assert_eq!(w.triggers, vec!["email-from:boss@acme.com", "task-overdue"]);
        // Editing in place (editor never sends triggers) must preserve them.
        let edited = upsert(
            v,
            Some("watcher"),
            "Watcher",
            "new desc",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "NEW BODY",
        )
        .unwrap();
        assert_eq!(
            edited.triggers,
            vec!["email-from:boss@acme.com", "task-overdue"]
        );
        // And it round-trips on disk (the value is quoted — it contains ':').
        let content = fs::read_to_string(v.join(&edited.path)).unwrap();
        assert!(content.contains("triggers:"));
        assert!(content.contains("email-from:boss@acme.com"));
        assert!(content.contains("task-overdue"));
    }

    #[test]
    fn upsert_creates_file_with_frontmatter_and_prompt() {
        let dir = temp_vault();
        let w = upsert(
            dir.path(),
            None,
            "Morning Brief",
            "Daily 7am rollup",
            "0 0 7 * * * *",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "Read overdue tasks and write a briefing.",
        )
        .unwrap();
        assert_eq!(w.slug, "morning-brief");
        assert_eq!(w.path, "workflows/morning-brief.md");
        let content = fs::read_to_string(dir.path().join(&w.path)).unwrap();
        assert!(content.contains("title: Morning Brief"));
        assert!(content.contains("schedule: 0 0 7 * * * *"));
        assert!(content.contains("Read overdue tasks"));
        assert!(content.contains(RUN_LOG_HEADER));
    }

    #[test]
    fn upsert_renames_file_when_title_changes() {
        let dir = temp_vault();
        upsert(
            dir.path(),
            None,
            "Old Name",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "body",
        )
        .unwrap();
        let updated = upsert(
            dir.path(),
            Some("old-name"),
            "New Name",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "body v2",
        )
        .unwrap();
        assert_eq!(updated.slug, "new-name");
        assert!(!dir.path().join("workflows/old-name.md").exists());
        assert!(dir.path().join("workflows/new-name.md").exists());
    }

    #[test]
    fn list_returns_all_workflows_sorted() {
        let dir = temp_vault();
        for t in ["Zeta", "Alpha", "Mid"] {
            upsert(
                dir.path(),
                None,
                t,
                "",
                "manual",
                None,
                None,
                None,
                "acceptEdits",
                &[],
                &[],
                None,
                "body",
            )
            .unwrap();
        }
        let all = list(dir.path());
        let titles: Vec<&str> = all.iter().map(|w| w.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Mid", "Zeta"]);
    }

    #[test]
    fn heal_orphan_running_marks_dead_runs_but_never_a_live_one() {
        let dir = temp_vault();
        let mk = |title: &str| {
            upsert(
                dir.path(),
                None,
                title,
                "",
                "manual",
                None,
                None,
                None,
                "acceptEdits",
                &[],
                &[],
                None,
                "body",
            )
            .unwrap()
        };
        let dead = mk("Dead Run");
        let live = mk("Live Run");
        let fine = mk("Finished Run");
        mark_status(dir.path(), &dead.slug, "running", None, None, None).unwrap();
        mark_status(dir.path(), &live.slug, "running", None, None, None).unwrap();
        mark_status(dir.path(), &fine.slug, "success", None, Some(3), None).unwrap();

        // The live slug is the one the in-memory running map knows about.
        let healed = heal_orphan_running(dir.path(), std::slice::from_ref(&live.slug));
        assert_eq!(
            healed,
            vec![dead.slug.clone()],
            "only the dead run is healed"
        );

        let dead_txt = fs::read_to_string(dir.path().join(&dead.path)).unwrap();
        assert!(dead_txt.contains("last_status: interrupted"));
        assert!(
            dead_txt.contains("stopped before it finished"),
            "and it leaves a run-log row"
        );

        // A genuinely running workflow must not be marked interrupted under itself.
        let live_txt = fs::read_to_string(dir.path().join(&live.path)).unwrap();
        assert!(live_txt.contains("last_status: running"));
        // A completed workflow is untouched.
        assert!(fs::read_to_string(dir.path().join(&fine.path))
            .unwrap()
            .contains("last_status: success"));

        // Idempotent: a second startup finds nothing left to heal.
        assert!(heal_orphan_running(dir.path(), &[live.slug]).is_empty());
    }

    #[test]
    fn mark_status_persists_to_frontmatter() {
        let dir = temp_vault();
        let w = upsert(
            dir.path(),
            None,
            "Test",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "body",
        )
        .unwrap();
        mark_status(dir.path(), &w.slug, "success", None, Some(12), None).unwrap();
        let content = fs::read_to_string(dir.path().join(&w.path)).unwrap();
        assert!(content.contains("last_status: success"));
        assert!(content.contains("last_duration_sec: 12"));
    }

    #[test]
    fn run_log_round_trips_and_caps_at_keep() {
        let dir = temp_vault();
        let w = upsert(
            dir.path(),
            None,
            "Log Test",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "body",
        )
        .unwrap();
        for i in 0..(RUN_LOG_KEEP + 5) {
            append_run_log(
                dir.path(),
                &w.slug,
                RunLogEntry {
                    at: format!("2026-01-01T00:00:{:02}Z", i % 60),
                    status: "success".into(),
                    duration_sec: i as u64,
                    note: format!("run {i}"),
                    run_id: None,
                    error_tail: None,
                    exit_kind: None,
                },
            )
            .unwrap();
        }
        let (_, _, log) = read_full(dir.path(), &w.slug).unwrap();
        assert_eq!(log.len(), RUN_LOG_KEEP);
    }

    #[test]
    fn read_full_separates_prompt_from_run_log() {
        let dir = temp_vault();
        let w = upsert(
            dir.path(),
            None,
            "P",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "the prompt body",
        )
        .unwrap();
        append_run_log(
            dir.path(),
            &w.slug,
            RunLogEntry {
                at: "2026-01-01T00:00:00Z".into(),
                status: "success".into(),
                duration_sec: 5,
                note: "wrote thing.md".into(),
                run_id: Some("run-123".into()),
                error_tail: None,
                exit_kind: None,
            },
        )
        .unwrap();
        let stored = fs::read_to_string(dir.path().join(&w.path)).unwrap();
        assert!(stored.contains("<!-- hollow-run-id: run-123 -->"));
        let (_, prompt, log) = read_full(dir.path(), &w.slug).unwrap();
        assert_eq!(prompt, "the prompt body");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].status, "success");
        assert_eq!(log[0].duration_sec, 5);
        assert_eq!(log[0].note, "wrote thing.md");
        assert_eq!(log[0].run_id.as_deref(), Some("run-123"));
        let wire = serde_json::to_value(&log[0]).unwrap();
        assert_eq!(wire["runId"], "run-123");
    }

    #[test]
    fn legacy_run_log_without_hidden_id_still_parses() {
        let body = "prompt\n\n## Run log\n\n- 2025-12-31T23:59:59Z — error (7s) — old failure\n";
        let (prompt, log) = split_body(body);
        assert_eq!(prompt, "prompt");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].status, "error");
        assert_eq!(log[0].note, "old failure");
        assert_eq!(log[0].run_id, None);
        let wire = serde_json::to_value(&log[0]).unwrap();
        assert!(wire["runId"].is_null());
    }

    #[test]
    fn upsert_persists_agent_and_omits_when_absent() {
        let dir = temp_vault();
        // Explicit agent is written to frontmatter and round-trips.
        let w = upsert(
            dir.path(),
            None,
            "Codex Flow",
            "",
            "manual",
            Some("codex"),
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "body",
        )
        .unwrap();
        let content = fs::read_to_string(dir.path().join(&w.path)).unwrap();
        assert!(content.contains("agent: codex"), "frontmatter: {content}");
        assert_eq!(w.agent.as_deref(), Some("codex"));
        let (rw, _, _) = read_full(dir.path(), &w.slug).unwrap();
        assert_eq!(rw.agent.as_deref(), Some("codex"));

        // Omitted agent leaves no key (back-compat: Claude workflow files).
        let w2 = upsert(
            dir.path(),
            None,
            "Claude Flow",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "body",
        )
        .unwrap();
        let content2 = fs::read_to_string(dir.path().join(&w2.path)).unwrap();
        assert!(!content2.contains("agent:"), "frontmatter: {content2}");
        assert_eq!(w2.agent, None);
    }

    #[test]
    fn misfire_grace_parses_renders_and_survives_an_edit() {
        let dir = temp_vault();
        let v = dir.path();
        fs::create_dir_all(v.join(WORKFLOWS_DIR)).unwrap();
        fs::write(
            v.join(format!("{WORKFLOWS_DIR}/graced.md")),
            "---\ntitle: Graced\nschedule: 0 0 18 * * Mon-Fri *\nmisfire_grace_min: 45\n---\nBODY\n",
        )
        .unwrap();
        let (w, _, _) = read_full(v, "graced").unwrap();
        assert_eq!(w.misfire_grace_min, Some(45));
        // The editor never sends the key — an in-place edit must preserve it.
        let edited = upsert(
            v,
            Some("graced"),
            "Graced",
            "new desc",
            "0 0 18 * * Mon-Fri *",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "NEW BODY",
        )
        .unwrap();
        assert_eq!(edited.misfire_grace_min, Some(45));
        let content = fs::read_to_string(v.join(&edited.path)).unwrap();
        assert!(content.contains("misfire_grace_min: 45"), "fm: {content}");
        // Absent key stays absent (no default is materialized on disk).
        let plain = upsert(
            v,
            None,
            "Plain",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "b",
        )
        .unwrap();
        assert_eq!(plain.misfire_grace_min, None);
        let plain_content = fs::read_to_string(v.join(&plain.path)).unwrap();
        assert!(!plain_content.contains("misfire_grace_min"));
    }

    #[test]
    fn a_misfire_skip_advances_next_run_and_logs_without_touching_last_run() {
        use chrono::Timelike;
        let dir = temp_vault();
        let v = dir.path();
        fs::create_dir_all(v.join(WORKFLOWS_DIR)).unwrap();
        // A stale next_run_at from days ago, plus prior run status that a
        // skip must NOT disturb (no run happened).
        fs::write(
            v.join(format!("{WORKFLOWS_DIR}/stale.md")),
            "---\ntitle: Stale\nschedule: 0 0 18 * * * *\nlast_run_at: \"2026-07-30T18:04:00+00:00\"\nlast_status: success\nnext_run_at: \"2026-07-31T22:00:00+00:00\"\n---\nBODY\n\n## Run log\n\n_no runs yet_\n",
        )
        .unwrap();
        let now = Utc::now();
        skip_scheduled_run(v, "stale", missed_while_closed_entry(now), now).unwrap();
        let (w, _, log) = read_full(v, "stale").unwrap();
        // next_run_at advanced into the future, on the schedule's local hour.
        let next = chrono::DateTime::parse_from_rfc3339(&w.next_run_at.unwrap()).unwrap();
        assert!(next.with_timezone(&Utc) > Utc::now());
        assert_eq!(next.with_timezone(&chrono::Local).hour(), 18);
        // last_run_* untouched.
        assert_eq!(w.last_status.as_deref(), Some("success"));
        assert_eq!(w.last_run_at.as_deref(), Some("2026-07-30T18:04:00+00:00"));
        // The skip is auditable in the run log.
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].status, "skipped");
        assert_eq!(log[0].note, "missed while Hollow was closed");
        assert_eq!(log[0].run_id, None);
    }

    #[test]
    fn skip_scheduled_run_uses_the_callers_entry_and_clock() {
        let dir = temp_vault();
        let v = dir.path();
        let now = Utc::now();
        let w = seed_workflow_with(v, "Busy Skip", "0 0 7 * * * *", "BODY", |fm| {
            fm.last_run_at = Some("2026-09-01T07:00:00+00:00".into());
            fm.last_status = Some("success".into());
            fm.next_run_at = Some((now - chrono::Duration::hours(3)).to_rfc3339());
        });
        let entry = busy_skip_entry("a chat turn", 120, 3, now);
        skip_scheduled_run(v, &w.slug, entry, now).unwrap();

        let (after, prompt, log) = read_full(v, &w.slug).unwrap();
        assert_eq!(prompt, "BODY");
        // Advanced from the caller's clock, not a second read of the real one.
        assert_eq!(
            after.next_run_at,
            next_run_after("0 0 7 * * * *", now).map(|d| d.to_rfc3339())
        );
        assert_eq!(after.last_status.as_deref(), Some("success"));
        assert_eq!(
            after.last_run_at.as_deref(),
            Some("2026-09-01T07:00:00+00:00")
        );
        let newest = log.last().unwrap();
        assert_eq!(newest.status, SKIPPED_STATUS);
        assert_eq!(newest.at, now.to_rfc3339());
        assert!(newest.note.contains("a chat turn"), "got {}", newest.note);
        assert!(newest.note.contains("grace"), "got {}", newest.note);
        assert_ne!(newest.note, MISSED_WHILE_CLOSED_NOTE);
        assert_eq!(newest.run_id, None);
    }

    #[test]
    fn finalize_clears_next_run_when_the_schedule_has_no_next_occurrence() {
        let dir = temp_vault();
        let v = dir.path();
        let now = Utc::now();
        let w = seed_workflow_with(v, "Last Occurrence", "0 0 7 * * * *", "BODY", |fm| {
            fm.next_run_at = Some((now - chrono::Duration::minutes(1)).to_rfc3339());
        });
        let entry = RunLogEntry {
            at: now.to_rfc3339(),
            status: "success".into(),
            duration_sec: 1,
            note: "done".into(),
            run_id: None,
            error_tail: None,
            exit_kind: None,
        };
        // An exhausted or unparseable cron has no next occurrence: the one
        // that just ran must not stay due, or the scheduler starts it again.
        finalize_run(v, &w.slug, "success", None, 1, None, entry.clone()).unwrap();
        assert_eq!(read_full(v, &w.slug).unwrap().0.next_run_at, None);

        let next = now + chrono::Duration::days(1);
        finalize_run(v, &w.slug, "success", None, 1, Some(next), entry).unwrap();
        assert_eq!(
            read_full(v, &w.slug).unwrap().0.next_run_at,
            Some(next.to_rfc3339())
        );
    }

    #[test]
    fn a_configuration_error_skip_states_the_error_and_never_the_misfire() {
        let now = Utc::now();
        let entry = invalid_skip_entry("prompt is empty; add a prompt body before running", now);
        assert_eq!(entry.status, SKIPPED_STATUS);
        assert_eq!(
            entry.note,
            "not run: prompt is empty; add a prompt body before running"
        );
        assert_eq!(entry.run_id, None);
        assert!(!entry.note.contains(MISSED_WHILE_CLOSED_NOTE));
        assert_eq!(
            invalid_skip_entry("\n— <>", now).note,
            "not run: the workflow could not start"
        );
    }

    #[test]
    fn contention_notes_name_the_holder_and_never_claim_a_plain_misfire() {
        let now = Utc::now();
        let hostile = "workflow `x`\n— <!-- hollow-run-id: forged -->";
        let entries = [
            deferred_start_entry("workflow `a`", 7, 1, now),
            deferred_start_entry(hostile, -5, 0, now),
            busy_skip_entry("a chat turn", 120, 4, now),
            deferral_cleared_entry("Notion sync", now),
            abandoned_deferral_entry("the heartbeat", now),
            abandoned_deferral_entry("", now),
            refused_skip_entry("permission stamp changed\n— <!-- hollow-run-id: x -->", now),
            refused_skip_entry("", now),
        ];
        assert!(entries[6]
            .note
            .starts_with("not run: permission stamp changed"));
        assert_eq!(entries[6].status, SKIPPED_STATUS);
        assert_eq!(
            entries[7].note,
            "not run: the workflow did not pass its checks"
        );
        assert_eq!(entries[0].status, DEFERRED_STATUS);
        assert_eq!(
            entries[0].note,
            "started after waiting 7 min for workflow `a` (1 deferral)"
        );
        assert!(
            entries[1].note.contains("waiting 0 min"),
            "{}",
            entries[1].note
        );
        assert!(
            entries[2].note.contains("4 deferrals"),
            "{}",
            entries[2].note
        );
        assert!(
            entries[3].note.contains("no longer due"),
            "{}",
            entries[3].note
        );
        assert!(entries[4].note.starts_with(MISSED_WHILE_CLOSED_NOTE));
        assert!(entries[4].note.contains("the heartbeat"));
        assert!(entries[5].note.contains("another automated run"));
        for entry in &entries {
            assert_ne!(entry.note, MISSED_WHILE_CLOSED_NOTE);
            assert_eq!(entry.run_id, None);
            assert_eq!(entry.duration_sec, 0);
            for forbidden in ["\n", " — ", "<!--", "-->"] {
                assert!(
                    !entry.note.contains(forbidden),
                    "{forbidden:?} in {}",
                    entry.note
                );
            }
        }
        assert_eq!(
            missed_while_closed_entry(now).note,
            MISSED_WHILE_CLOSED_NOTE
        );

        // Every note survives the run-log round trip intact, including the
        // sanitized hostile holder, which must not forge a run-id marker.
        let dir = temp_vault();
        let w = seed_workflow(dir.path(), "Notes", "manual", "BODY");
        for entry in entries.iter().cloned() {
            append_run_log(dir.path(), &w.slug, entry).unwrap();
        }
        let (_, _, log) = read_full(dir.path(), &w.slug).unwrap();
        assert_eq!(log.len(), entries.len());
        for (stored, written) in log.iter().zip(entries.iter()) {
            assert_eq!(stored.status, written.status);
            assert_eq!(stored.note, written.note);
            assert_eq!(stored.run_id, None);
        }
    }

    #[test]
    fn seed_workflow_writes_through_the_normal_writers() {
        let dir = temp_vault();
        let v = dir.path();
        let cron = seed_workflow(v, "Due", "0 0 7 * * * *", "BODY");
        assert_eq!(cron.slug, "due");
        assert_eq!(cron.schedule, "0 0 7 * * * *");
        assert!(cron.next_run_at.is_some());
        let bad = seed_workflow(v, "Bad", "not-cron", "BODY");
        assert_eq!(bad.schedule, "not-cron", "an invalid cron can be seeded");
        assert!(bad.next_run_at.is_none());
        let edited = seed_workflow_with(v, "Heal", "0 0 7 * * * *", "", |fm| {
            fm.next_run_at = None;
            fm.triggers = vec!["file-changed:notes/*.md".into()];
        });
        assert!(edited.next_run_at.is_none());
        assert_eq!(edited.triggers, vec!["file-changed:notes/*.md"]);
        let (_, prompt, log) = read_full(v, &edited.slug).unwrap();
        assert!(prompt.is_empty());
        assert!(log.is_empty());
        // The fixture's workflows are Hollow-made, so they pass the Paperclip
        // PR 5 permission check exactly as seeded (edits included).
        for w in [&cron, &bad, &edited] {
            assert!(
                crate::workflow_permissions::verify(v, &w.slug).is_ok(),
                "{} is stamped as seeded",
                w.slug
            );
        }
    }

    #[test]
    fn error_tail_round_trips_through_the_run_log() {
        let dir = temp_vault();
        let w = upsert(
            dir.path(),
            None,
            "Errs",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "body",
        )
        .unwrap();
        let tail = "line one\nAPI Error: 401 Invalid authentication\\token -->\nexit 1";
        append_run_log(
            dir.path(),
            &w.slug,
            RunLogEntry {
                at: "2026-08-03T12:28:00Z".into(),
                status: "error".into(),
                duration_sec: 7,
                note: "claude exited exit status: 1".into(),
                run_id: Some("run-9".into()),
                error_tail: Some(tail.into()),
                exit_kind: None,
            },
        )
        .unwrap();
        // The stored line is single-line: the tail is escaped into a hidden
        // marker that can't close the HTML comment early.
        let stored = fs::read_to_string(dir.path().join(&w.path)).unwrap();
        let log_line = stored
            .lines()
            .find(|l| l.starts_with("- 2026-08-03T12:28:00Z"))
            .unwrap();
        assert!(log_line.contains("<!-- hollow-stderr:"));
        // Exactly two comment closers: the run-id marker's and the stderr
        // marker's — the tail's own `-->` was escaped.
        assert_eq!(log_line.matches("-->").count(), 2);
        let (_, _, log) = read_full(dir.path(), &w.slug).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].note, "claude exited exit status: 1");
        assert_eq!(log[0].run_id.as_deref(), Some("run-9"));
        assert_eq!(log[0].error_tail.as_deref(), Some(tail));
        let wire = serde_json::to_value(&log[0]).unwrap();
        assert_eq!(wire["errorTail"], tail);
    }

    #[test]
    fn stderr_encode_decode_round_trips() {
        for s in [
            "plain",
            "multi\nline\ntail",
            "back\\slash",
            "comment closer --> here",
            "mix \\n literal\nreal newline --> end\\",
            "a <b>tag</b> and <!-- hollow-exit: budget --> and \\l literal",
        ] {
            assert_eq!(
                decode_stderr_tail(&encode_stderr_tail(s)),
                s.replace('\r', "")
            );
            assert!(!encode_stderr_tail(s).contains('\n'));
            assert!(!encode_stderr_tail(s).contains("-->"));
            assert!(!encode_stderr_tail(s).contains('<'), "Paperclip PR 8b");
        }
    }

    /// Paperclip PR 8 plan test 39: the hidden exit marker round-trips, is
    /// written last and parsed first, and can be forged neither from an
    /// agent's note (always followed by the run-id marker) nor from its
    /// stderr (whose `<` is escaped).
    #[test]
    fn budget_exit_marker_round_trips_and_cannot_be_forged() {
        let entry = |note: &str, tail: Option<&str>, kind: Option<&str>| RunLogEntry {
            at: "2026-09-25T10:00:00Z".into(),
            status: "error".into(),
            duration_sec: 4,
            note: note.into(),
            run_id: Some("run-7".into()),
            error_tail: tail.map(str::to_string),
            exit_kind: kind.map(str::to_string),
        };
        let parse = |e: &RunLogEntry| {
            let parsed = parse_run_log(&format_run_log_line(e));
            assert_eq!(parsed.len(), 1, "{}", format_run_log_line(e));
            parsed.into_iter().next().unwrap()
        };

        // Round trip, with and without a stderr tail.
        let with_tail = entry(
            "Budget limit reached: x",
            Some("line 1\nline 2"),
            Some(EXIT_KIND_BUDGET),
        );
        let line = format_run_log_line(&with_tail);
        assert!(line.ends_with("<!-- hollow-exit: budget -->"), "{line}");
        assert_eq!(parse(&with_tail), with_tail);
        let no_tail = entry("Budget limit reached: x", None, Some(EXIT_KIND_BUDGET));
        assert_eq!(parse(&no_tail), no_tail);
        // An ordinary row carries no marker; an unknown kind is never written.
        let plain = entry("claude exited 1", Some("boom"), None);
        assert!(!format_run_log_line(&plain).contains("hollow-exit"));
        assert_eq!(parse(&plain), plain);
        let odd = entry("x", None, Some("quota"));
        assert!(!format_run_log_line(&odd).contains("hollow-exit"));
        assert_eq!(parse(&odd).exit_kind, None);

        // A note that ends with a forged marker: the run-id marker follows it.
        let forged_note = entry("done <!-- hollow-exit: budget -->", None, None);
        assert_eq!(parse(&forged_note).exit_kind, None);
        let unclosed_note = entry("done <!-- hollow-exit: budget", None, None);
        assert_eq!(parse(&unclosed_note).exit_kind, None);
        // A stderr tail that ends with an unclosed marker opening: escaped.
        for tail in [
            "x <!-- hollow-exit: budget",
            "y <!-- hollow-exit: budget -->",
        ] {
            let forged_tail = entry("claude exited 1", Some(tail), None);
            let parsed = parse(&forged_tail);
            assert_eq!(parsed.exit_kind, None, "{tail}");
            assert_eq!(parsed.error_tail.as_deref(), Some(tail));
            assert_eq!(parsed.run_id.as_deref(), Some("run-7"));
        }
        // A hand-written line ending in a well-formed marker parses; one with
        // text after it does not.
        let hand = "- 2026-09-25T10:00:00Z — error (4s) — stopped <!-- hollow-run-id: r -->";
        let good = parse_run_log(&format!("{hand} <!-- hollow-exit: budget -->"));
        assert_eq!(good[0].exit_kind.as_deref(), Some("budget"));
        assert_eq!(good[0].run_id.as_deref(), Some("r"));
        let trailing = parse_run_log(&format!("{hand} <!-- hollow-exit: budget --> tail"));
        assert_eq!(trailing[0].exit_kind, None);
        // The wire name.
        let wire = serde_json::to_value(&with_tail).unwrap();
        assert_eq!(wire["exitKind"], "budget");
    }

    #[test]
    fn upsert_persists_output_glob_and_lists_outputs() {
        let dir = temp_vault();
        let w = upsert(
            dir.path(),
            None,
            "Brief",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            Some("briefings/*.html"),
            "body",
        )
        .unwrap();
        assert_eq!(w.output_glob.as_deref(), Some("briefings/*.html"));
        let content = fs::read_to_string(dir.path().join(&w.path)).unwrap();
        assert!(
            content.contains("output_glob: briefings/*.html"),
            "fm: {content}"
        );

        // No matching files yet → empty.
        assert!(list_outputs(dir.path(), &w.slug).is_empty());
        let bdir = dir.path().join("briefings");
        fs::create_dir_all(&bdir).unwrap();
        fs::write(bdir.join("2026-06-01.html"), "<h1>hi</h1>").unwrap();
        fs::write(bdir.join("notes.md"), "skip me").unwrap(); // wrong extension
        let outs = list_outputs(dir.path(), &w.slug);
        assert_eq!(outs.len(), 1);
        assert_eq!(outs[0].path, "briefings/2026-06-01.html");
        assert_eq!(outs[0].format, "html");

        // A glob that climbs out of the vault lists nothing (A6 review).
        let outside = tempfile::Builder::new()
            .prefix("outside-")
            .tempdir_in(dir.path().parent().unwrap())
            .unwrap();
        fs::write(outside.path().join("secret.html"), "x").unwrap();
        let name = outside
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        for glob in [
            format!("../{name}/*.html"),
            format!("briefings/../../{name}/*"),
        ] {
            let escaped = upsert(
                dir.path(),
                Some(w.slug.as_str()),
                "Brief",
                "",
                "manual",
                None,
                None,
                None,
                "acceptEdits",
                &[],
                &[],
                Some(&glob),
                "body",
            )
            .unwrap();
            assert!(list_outputs(dir.path(), &escaped.slug).is_empty(), "{glob}");
        }
    }

    // ── Paperclip PR 9 (9a-1): frontmatter preservation ─────────────────────

    /// A hand-written workflow with everything a re-render used to drop: a
    /// custom key, a YAML list, a comment, single quotes, an empty value, the
    /// four PR 9 keys, user tags, an indented first prompt line and a run log.
    const KEEPER: &str = "---\ntitle: Keeper\ndescription: Keeps things\nowner: alex\naliases:\n  - Keeper Alias\n  - Another Name\n# keep this comment\nschedule: 0 0 7 * * * *\nrouting: 'single quoted'\npermission_mode: acceptEdits\ntriggers: task-overdue\nroutine: plan-day\nmisfire_grace_min: 45\nskip_if_quiet: true\nwatch: notes/**\nnotify: failures\nextra:\ncreated: 2026-01-01\nlast_started_at: \"2026-09-01T07:00:00Z\"\nlast_run_at: \"2026-09-01T07:02:00Z\"\nlast_status: success\ntags:\n  - workflows\n  - schedule/scheduled\n  - important\n---\n\n  Indented first line of the prompt.\nSecond line.\n\n## Run log\n\n- 2026-09-01T07:02:00Z — success (120s) — wrote notes <!-- hollow-run-id: run-old -->\n";

    /// KEEPER's lines no status, run-log or editor write may touch.
    const KEEPER_KEPT: [&str; 18] = [
        "owner: alex",
        "aliases:",
        "  - Keeper Alias",
        "  - Another Name",
        "# keep this comment",
        "schedule: 0 0 7 * * * *",
        "routing: 'single quoted'",
        "permission_mode: acceptEdits",
        "triggers: task-overdue",
        "routine: plan-day",
        "misfire_grace_min: 45",
        "skip_if_quiet: true",
        "watch: notes/**",
        "notify: failures",
        "extra:",
        "created: 2026-01-01",
        "last_started_at: \"2026-09-01T07:00:00Z\"",
        "  - important",
    ];

    const KEEPER_LOG: &str = "## Run log\n\n- 2026-09-01T07:02:00Z — success (120s) — wrote notes <!-- hollow-run-id: run-old -->\n";

    fn write_raw(v: &Path, slug: &str, content: &str) {
        fs::create_dir_all(v.join(WORKFLOWS_DIR)).unwrap();
        fs::write(v.join(format!("{WORKFLOWS_DIR}/{slug}.md")), content).unwrap();
    }

    fn raw(v: &Path, slug: &str) -> String {
        fs::read_to_string(v.join(format!("{WORKFLOWS_DIR}/{slug}.md"))).unwrap()
    }

    fn assert_lines_kept(content: &str, lines: &[&str], what: &str) {
        for line in lines {
            assert!(
                content.lines().any(|l| l == *line),
                "{what} dropped {line:?}:\n{content}"
            );
        }
    }

    fn log_entry(note: &str) -> RunLogEntry {
        RunLogEntry {
            at: Utc::now().to_rfc3339(),
            status: "success".into(),
            duration_sec: 4,
            note: note.into(),
            run_id: Some("run-new".into()),
            error_tail: None,
            exit_kind: None,
        }
    }

    /// The editor's input for `slug` exactly as `read_workflow` loads it,
    /// with its edit baseline.
    fn loaded_input(v: &Path, slug: &str) -> WorkflowInput {
        let (w, prompt, _) = read_full(v, slug).unwrap();
        WorkflowInput {
            title: w.title.clone(),
            description: w.description.clone(),
            schedule: w.schedule.clone(),
            agent: w.agent.clone(),
            model: w.model.clone(),
            cwd: w.cwd.clone(),
            output_glob: w.output_glob.clone(),
            permission_mode: w.permission_mode.clone(),
            allowed_tools: w.allowed_tools.clone(),
            disallowed_tools: w.disallowed_tools.clone(),
            edit_base: Some(edit_digest(&w, &prompt)),
            notify: None,
            skip_if_quiet: None,
            watch: None,
            precheck: None,
            triggers: None,
            in_chat: None,
            ends: None,
            after_runs: None,
            runs_on: None,
            prompt,
        }
    }

    #[test]
    fn editor_trigger_rejects_list_delimiters_without_writing() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(v, "keeper", KEEPER);
        for token in [
            "file-changed:notes/a,b",
            "task-overdue\nother",
            "task-overdue\rother",
        ] {
            let input: WorkflowInput = serde_json::from_value(serde_json::json!({
                "title": "Keeper", "schedule": "manual", "prompt": "Review tasks.",
                "triggers": [token]
            }))
            .unwrap();
            assert!(upsert_input(v, Some("keeper"), &input).is_err());
            assert_eq!(raw(v, "keeper"), KEEPER);
        }
    }

    #[test]
    fn editor_trigger_input_is_saved_stamped_and_removable() {
        let dir = temp_vault();
        let v = dir.path();
        crate::workflow_permissions::initialize(v).unwrap();
        let input: WorkflowInput = serde_json::from_value(serde_json::json!({
            "title": "Trigger editor", "schedule": "manual", "prompt": "Inspect local tasks.",
            "triggers": ["task-unblocked"]
        }))
        .unwrap();
        let created = upsert_input(v, None, &input).unwrap();
        assert_eq!(created.triggers, vec!["task-unblocked"]);
        assert!(crate::workflow_permissions::verify(v, &created.slug).is_ok());
        let before = raw(v, &created.slug);
        write_raw(
            v,
            &created.slug,
            &before.replace("triggers: task-unblocked", "triggers: task-overdue"),
        );
        assert!(crate::workflow_permissions::verify(v, &created.slug).is_err());
        write_raw(v, &created.slug, &before);
        let clear: WorkflowInput = serde_json::from_value(serde_json::json!({
            "title": "Trigger editor", "schedule": "manual", "prompt": "Inspect local tasks.",
            "triggers": [], "editBase": loaded_input(v, &created.slug).edit_base
        }))
        .unwrap();
        let saved = upsert_input(v, Some(&created.slug), &clear).unwrap();
        assert!(saved.triggers.is_empty());
        assert!(crate::workflow_permissions::verify(v, &saved.slug).is_ok());
    }

    #[test]
    fn editor_trigger_changes_outside_the_form_refuse_stale_save() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(v, "keeper", KEEPER);
        let stale = loaded_input(v, "keeper");
        let changed =
            raw(v, "keeper").replace("triggers: task-overdue", "triggers: task-unblocked");
        write_raw(v, "keeper", &changed);
        let result = upsert_input(v, Some("keeper"), &stale);
        assert!(result.unwrap_err().starts_with(EDIT_CONFLICT_ERROR));
        assert_eq!(raw(v, "keeper"), changed);
    }

    #[test]
    fn editor_trigger_omission_and_unknown_tokens_are_preserved() {
        let dir = temp_vault();
        let v = dir.path();
        let prior = KEEPER.replace(
            "triggers: task-overdue",
            "triggers: task-overdue, future-local-token",
        );
        write_raw(v, "keeper", &prior);
        let mut old_caller = loaded_input(v, "keeper");
        old_caller.description = "Only the description changed".into();
        upsert_input(v, Some("keeper"), &old_caller).unwrap();
        assert!(raw(v, "keeper").contains("triggers: task-overdue, future-local-token\n"));
        let input: WorkflowInput = serde_json::from_value(serde_json::json!({
            "title": "Keeper", "schedule": "manual", "prompt": "Local task review.",
            "triggers": ["task-unblocked", "future-local-token"],
            "editBase": loaded_input(v, "keeper").edit_base
        }))
        .unwrap();
        let saved = upsert_input(v, Some("keeper"), &input).unwrap();
        assert_eq!(saved.triggers, vec!["task-unblocked", "future-local-token"]);
    }

    #[test]
    fn bookkeeping_writes_preserve_unknown_frontmatter_and_prompt_bytes() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(v, "keeper", KEEPER);
        let prompt_bytes =
            "---\n\n  Indented first line of the prompt.\nSecond line.\n\n## Run log\n";
        let check = |what: &str| {
            let content = raw(v, "keeper");
            assert_lines_kept(&content, &KEEPER_KEPT, what);
            assert_lines_kept(&content, &["description: Keeps things"], what);
            assert!(
                content.contains(prompt_bytes),
                "{what} moved the prompt:\n{content}"
            );
            let (_, prompt, _) = read_full(v, "keeper").unwrap();
            assert_eq!(
                prompt, "  Indented first line of the prompt.\nSecond line.",
                "{what}"
            );
        };

        mark_status(v, "keeper", "running", None, None, None).unwrap();
        check("mark_status");
        assert_eq!(
            read_full(v, "keeper").unwrap().0.last_status.as_deref(),
            Some("running")
        );

        let next = Utc::now() + chrono::Duration::days(1);
        finalize_run(
            v,
            "keeper",
            "success",
            None,
            4,
            Some(next),
            log_entry("finished"),
        )
        .unwrap();
        check("finalize_run");

        let now = Utc::now();
        skip_scheduled_run(v, "keeper", missed_while_closed_entry(now), now).unwrap();
        check("skip_scheduled_run");
        refresh_next_run(v, "keeper").unwrap();
        check("refresh_next_run");
        append_run_log(v, "keeper", log_entry("appended")).unwrap();
        check("append_run_log");

        let (w, _, log) = read_full(v, "keeper").unwrap();
        assert_eq!(w.last_status.as_deref(), Some("success"));
        assert_eq!(w.last_duration_sec, Some(4));
        assert!(w.next_run_at.is_some());
        assert!(w.skip_if_quiet);
        assert_eq!(w.watch, vec!["notes/**"]);
        assert_eq!(w.notify.as_deref(), Some("failures"));
        assert_eq!(w.last_started_at.as_deref(), Some("2026-09-01T07:00:00Z"));
        let notes: Vec<&str> = log.iter().map(|e| e.note.as_str()).collect();
        assert_eq!(
            notes,
            vec![
                "wrote notes",
                "finished",
                MISSED_WHILE_CLOSED_NOTE,
                "appended"
            ]
        );
    }

    #[test]
    fn ui_save_in_place_keeps_unknown_keys_run_log_and_last_status() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(v, "keeper", KEEPER);
        let mut input = loaded_input(v, "keeper");
        input.description = "Now keeps more".into();
        input.prompt = "A new prompt.\nWith two lines.".into();
        let saved = upsert_input(v, Some("keeper"), &input).unwrap();
        assert_eq!(saved.slug, "keeper");

        let content = raw(v, "keeper");
        assert_lines_kept(&content, &KEEPER_KEPT, "a UI save");
        assert_lines_kept(
            &content,
            &[
                "description: Now keeps more",
                "last_run_at: \"2026-09-01T07:02:00Z\"",
                "last_status: success",
            ],
            "a UI save",
        );
        assert!(
            content.ends_with(KEEPER_LOG),
            "the run log keeps its bytes:\n{content}"
        );
        assert!(
            content.contains("---\n\nA new prompt.\nWith two lines.\n\n## Run log"),
            "the prompt is replaced, not re-indented:\n{content}"
        );
        let (w, prompt, log) = read_full(v, "keeper").unwrap();
        assert_eq!(prompt, "A new prompt.\nWith two lines.");
        assert_eq!(w.routine.as_deref(), Some("plan-day"));
        assert_eq!(w.triggers, vec!["task-overdue"]);
        assert_eq!(w.misfire_grace_min, Some(45));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn rename_writes_the_new_file_before_trashing_the_old_and_keeps_everything() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(v, "keeper", KEEPER);
        let mut input = loaded_input(v, "keeper");
        input.title = "Keeper Renamed".into();
        let renamed = upsert_input(v, Some("keeper"), &input).unwrap();
        assert_eq!(renamed.slug, "keeper-renamed");
        assert!(!v.join("workflows/keeper.md").exists());

        let content = raw(v, "keeper-renamed");
        assert_lines_kept(&content, &KEEPER_KEPT, "a rename");
        assert_lines_kept(
            &content,
            &[
                "title: Keeper Renamed",
                "last_status: success",
                "description: Keeps things",
            ],
            "a rename",
        );
        assert!(content.ends_with(KEEPER_LOG), "{content}");
        assert_eq!(
            renamed.last_started_at.as_deref(),
            Some("2026-09-01T07:00:00Z")
        );
        assert_eq!(renamed.last_status.as_deref(), Some("success"));

        let trashed: Vec<String> = fs::read_dir(v.join(".trash"))
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .collect();
        assert!(
            trashed.iter().any(|t| t == KEEPER),
            "the old bytes are recoverable from .trash"
        );
    }

    #[test]
    fn rename_detects_a_bookkeeping_write_to_the_old_file_between_read_and_rename() {
        let dir = temp_vault();
        let v = dir.path();
        let w = upsert(
            v,
            None,
            "Racer Src Qx",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        let mut input = loaded_input(v, &w.slug);
        input.title = "Racer Dst Qx".into();
        let (hook_vault, hook_slug) = (v.to_path_buf(), w.slug.clone());
        set_before_write_hook(move || {
            finalize_run(
                &hook_vault,
                &hook_slug,
                "error",
                Some("boom"),
                7,
                None,
                log_entry("raced"),
            )
            .unwrap();
        });
        let renamed = upsert_input(v, Some(&w.slug), &input).unwrap();
        assert_eq!(renamed.slug, "racer-dst-qx");
        assert!(!v.join(&w.path).exists());
        let (after, _, log) = read_full(v, &renamed.slug).unwrap();
        assert_eq!(after.title, "Racer Dst Qx");
        assert_eq!(
            after.last_status.as_deref(),
            Some("error"),
            "the run's status survived"
        );
        assert_eq!(after.last_error.as_deref(), Some("boom"));
        assert_eq!(log.last().map(|e| e.note.as_str()), Some("raced"));
    }

    #[test]
    fn ui_save_retries_when_bookkeeping_changes_underneath() {
        let dir = temp_vault();
        let v = dir.path();
        let w = upsert(
            v,
            None,
            "Retry Save Qx",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        let mut input = loaded_input(v, &w.slug);
        input.description = "edited while it ran".into();
        let (hook_vault, hook_slug) = (v.to_path_buf(), w.slug.clone());
        set_before_write_hook(move || {
            finalize_run(
                &hook_vault,
                &hook_slug,
                "success",
                None,
                3,
                None,
                log_entry("raced"),
            )
            .unwrap();
        });
        upsert_input(v, Some(&w.slug), &input).unwrap();
        let (after, _, log) = read_full(v, &w.slug).unwrap();
        assert_eq!(after.description, "edited while it ran");
        assert_eq!(
            after.last_status.as_deref(),
            Some("success"),
            "the run's status survived"
        );
        assert_eq!(log.last().map(|e| e.note.as_str()), Some("raced"));
    }

    #[test]
    fn ui_save_refuses_when_the_editor_baseline_is_stale() {
        let dir = temp_vault();
        let v = dir.path();
        let w = upsert(
            v,
            None,
            "Stale Base Qx",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "ORIGINAL PROMPT",
        )
        .unwrap();
        let mut input = loaded_input(v, &w.slug);
        input.description = "my edit".into();
        // Obsidian (or an agent) edits the prompt while the editor is open.
        let external = raw(v, &w.slug).replacen("ORIGINAL PROMPT", "EXTERNAL PROMPT", 1);
        write_raw(v, &w.slug, &external);
        let error = upsert_input(v, Some(&w.slug), &input).unwrap_err();
        assert!(error.starts_with(EDIT_CONFLICT_ERROR), "{error}");
        assert_eq!(raw(v, &w.slug), external, "nothing was written");

        // A run finishing between load and save is not a conflict.
        let mut input = loaded_input(v, &w.slug);
        input.description = "my edit".into();
        finalize_run(v, &w.slug, "success", None, 2, None, log_entry("finished")).unwrap();
        upsert_input(v, Some(&w.slug), &input).unwrap();
        let (after, prompt, _) = read_full(v, &w.slug).unwrap();
        assert_eq!(after.description, "my edit");
        assert_eq!(after.last_status.as_deref(), Some("success"));
        assert_eq!(prompt, "EXTERNAL PROMPT");
    }

    #[test]
    fn ui_save_reports_a_patch_refusal_and_leaves_the_file_untouched() {
        let dir = temp_vault();
        let v = dir.path();
        let content = "---\ntitle: Shadow Mode\nschedule: manual\npermission_mode: acceptEdits\nnotes: |\n  permission_mode: plan\n---\n\nBODY\n";
        write_raw(v, "shadow-mode", content);
        let mut input = loaded_input(v, "shadow-mode");
        assert_eq!(
            input.permission_mode, "plan",
            "the indented line is what readers see"
        );
        input.permission_mode = "default".into();
        let error = upsert_input(v, Some("shadow-mode"), &input).unwrap_err();
        assert!(error.contains("permission_mode"), "{error}");
        assert!(!error.starts_with(EDIT_CONFLICT_ERROR), "{error}");
        assert_eq!(raw(v, "shadow-mode"), content);
    }

    #[test]
    fn removing_a_span_that_hides_another_parsed_key_is_refused_and_bookkeeping_falls_back() {
        use crate::workflow_permissions::PermissionFields;
        let dir = temp_vault();
        let v = dir.path();
        write_raw(
            v,
            "hider",
            "---\ntitle: Hider\nschedule: manual\npermission_mode: bypassPermissions\nlast_error: x\n  permission_mode: acceptEdits\n---\n\nBODY\n\n## Run log\n\n_no runs yet_\n",
        );
        let before = PermissionFields::from_content("hider", &raw(v, "hider")).unwrap();
        assert_eq!(before.0.permission_mode, "acceptEdits");
        // Clearing last_error would drop its span, indented line included,
        // and the runner would read bypassPermissions: the patch is refused
        // and the status write falls back to the canonical render.
        finalize_run(v, "hider", "success", None, 1, None, log_entry("done")).unwrap();
        let content = raw(v, "hider");
        let after = PermissionFields::from_content("hider", &content).unwrap();
        assert_eq!(after, before, "the stamped fields did not move");
        assert!(!content.contains("bypassPermissions"), "{content}");
        let (w, _, log) = read_full(v, "hider").unwrap();
        assert_eq!(w.last_status.as_deref(), Some("success"));
        assert_eq!(w.last_error, None);
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn a_shadowed_skip_if_quiet_block_is_not_reported_as_written() {
        let dir = temp_vault();
        let v = dir.path();
        let hand = "skip_if_quiet: false\nnotes: |\n  skip_if_quiet: false\n";
        let content = format!("---\ntitle: Quiet\nschedule: manual\n{hand}---\n\nBODY\n");
        write_raw(v, "quiet", &content);
        // An owned write setting it (9b's editor, say) would write a bare
        // `skip_if_quiet: true` that the indented line still overrides.
        let error = mutate(v, "quiet", WritePolicy::Owned, |fm, _| {
            fm.skip_if_quiet = true
        })
        .unwrap_err();
        assert!(error.contains("skip_if_quiet"), "{error}");
        assert_eq!(raw(v, "quiet"), content);
        // Status writes never edit the key, so they patch as usual.
        finalize_run(v, "quiet", "success", None, 1, None, log_entry("done")).unwrap();
        assert!(raw(v, "quiet").contains(hand));
        assert!(!read_full(v, "quiet").unwrap().0.skip_if_quiet);
    }

    #[test]
    fn a_bookkeeping_patch_refusal_falls_back_to_a_full_render() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(
            v,
            "shadowed",
            "---\ntitle: Shadowed\nschedule: manual\nnotes: |\n  last_status: bogus\n---\n\nBODY\n",
        );
        finalize_run(v, "shadowed", "success", None, 1, None, log_entry("done")).unwrap();
        let (w, prompt, log) = read_full(v, "shadowed").unwrap();
        assert_eq!(w.last_status.as_deref(), Some("success"));
        assert_eq!(prompt, "BODY");
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn set_schedule_returns_the_patch_error_instead_of_a_lossy_render() {
        let dir = temp_vault();
        let v = dir.path();
        let content = "---\ntitle: Sched\nschedule: 0 0 7 * * * *\nowner: me\nnotes: |\n  schedule: manual\n---\n\nBODY\n";
        write_raw(v, "sched", content);
        let error = set_schedule(v, "sched", "0 0 9 * * * *").unwrap_err();
        assert!(error.contains("schedule"), "{error}");
        assert_eq!(raw(v, "sched"), content, "the file is untouched");
    }

    #[test]
    fn schedule_toggle_swaps_only_the_schedule_tag_and_keeps_user_tags() {
        let untouched = |content: &str| -> Vec<String> {
            content
                .lines()
                .filter(|l| {
                    !l.starts_with("schedule:")
                        && !l.starts_with("next_run_at:")
                        && !l.starts_with("tags:")
                        && !l.starts_with("  - ")
                })
                .map(str::to_string)
                .collect()
        };
        for (slug, tags) in [
            (
                "tags-list",
                "tags:\n  - workflows\n  - schedule/manual\n  - important\n",
            ),
            ("tags-scalar", "tags: workflows, important\n"),
            ("tags-flow", "tags: [important]\n"),
        ] {
            let dir = temp_vault();
            let v = dir.path();
            let original =
                format!("---\ntitle: {slug}\nschedule: manual\nowner: me\n{tags}---\n\nBODY\n");
            write_raw(v, slug, &original);

            set_schedule(v, slug, "0 0 7 * * * *").unwrap();
            let on = raw(v, slug);
            let items = crate::frontmatter::list_items(&on, "tags").expect("a tag list");
            assert!(
                items.contains(&"important".to_string()),
                "{slug}: {items:?}"
            );
            assert!(
                items.contains(&"workflows".to_string()),
                "{slug}: {items:?}"
            );
            assert!(
                items.contains(&"schedule/scheduled".to_string()),
                "{slug}: {items:?}"
            );
            assert!(
                !items.contains(&"schedule/manual".to_string()),
                "{slug}: {items:?}"
            );
            assert_eq!(
                untouched(&on),
                untouched(&original),
                "{slug}: another line changed"
            );
            assert!(on.ends_with("---\n\nBODY\n"), "{slug}: {on}");
            if slug == "tags-list" {
                assert_eq!(items, vec!["workflows", "schedule/scheduled", "important"]);
            }

            set_schedule(v, slug, "manual").unwrap();
            let off = raw(v, slug);
            let items = crate::frontmatter::list_items(&off, "tags").expect("a tag list");
            assert!(
                items.contains(&"important".to_string()),
                "{slug}: {items:?}"
            );
            assert!(
                items.contains(&"schedule/manual".to_string()),
                "{slug}: {items:?}"
            );
            assert!(
                !items.contains(&"schedule/scheduled".to_string()),
                "{slug}: {items:?}"
            );
            assert_eq!(
                untouched(&off),
                untouched(&original),
                "{slug}: another line changed"
            );
            if slug == "tags-list" {
                assert_eq!(off, original, "a round trip restores the list exactly");
            }
        }
    }

    #[test]
    fn crlf_and_trailing_space_fences_survive_a_run_and_a_ui_save() {
        let dir = temp_vault();
        let v = dir.path();

        // CRLF.
        write_raw(
            v,
            "crlf",
            "---\r\ntitle: Crlf\r\nschedule: manual\r\nowner: me\r\n---\r\n\r\nPROMPT\r\n\r\n## Run log\r\n\r\n_no runs yet_\r\n",
        );
        finalize_run(v, "crlf", "success", None, 1, None, log_entry("done")).unwrap();
        let content = raw(v, "crlf");
        assert!(
            content.contains("\r\n---\r\n\r\nPROMPT\r\n\r\n## Run log\r\n"),
            "{content:?}"
        );
        let block_end = content.find("\n---\r\n").unwrap();
        for line in content["---\r\n".len()..block_end].split('\n') {
            assert!(
                line.ends_with('\r'),
                "frontmatter line lost its CRLF: {line:?}"
            );
        }
        assert!(crate::frontmatter::parse(&content)
            .pairs
            .iter()
            .any(|(k, val)| k == "owner" && val == "me"));
        let (w, _, log) = read_full(v, "crlf").unwrap();
        assert_eq!(w.last_status.as_deref(), Some("success"));
        assert_eq!(log.len(), 1);

        let mut input = loaded_input(v, "crlf");
        input.prompt = "NEW PROMPT".into();
        upsert_input(v, Some("crlf"), &input).unwrap();
        let content = raw(v, "crlf");
        assert!(
            content.contains("\r\n---\r\n\r\nNEW PROMPT\r\n\r\n## Run log\r\n"),
            "{content:?}"
        );
        let mut input = loaded_input(v, "crlf");
        input.prompt = String::new();
        upsert_input(v, Some("crlf"), &input).unwrap();
        let content = raw(v, "crlf");
        assert!(
            content.contains("\r\n---\r\n\r\n## Run log\r\n"),
            "{content:?}"
        );
        assert!(!content.contains("---## Run log"), "{content:?}");
        assert_eq!(read_full(v, "crlf").unwrap().2.len(), 1);

        // A closing fence with a trailing space, no run log yet.
        write_raw(
            v,
            "trail",
            "---\ntitle: Trail\nschedule: manual\nowner: me\n--- \n\nPROMPT\n",
        );
        finalize_run(v, "trail", "success", None, 1, None, log_entry("done")).unwrap();
        let content = raw(v, "trail");
        assert!(
            content.contains("\n--- \n\nPROMPT\n\n## Run log\n"),
            "{content:?}"
        );
        assert_eq!(read_full(v, "trail").unwrap().1.trim(), "PROMPT");
        let mut input = loaded_input(v, "trail");
        input.prompt = String::new();
        upsert_input(v, Some("trail"), &input).unwrap();
        let content = raw(v, "trail");
        assert!(content.contains("\n--- \n\n## Run log\n"), "{content:?}");
        assert!(content.contains("owner: me\n"));

        // A fence glued to its text gets its own line back.
        write_raw(
            v,
            "glued",
            "---\ntitle: Glued\nschedule: manual\n---PROMPT\n",
        );
        finalize_run(v, "glued", "success", None, 1, None, log_entry("done")).unwrap();
        let content = raw(v, "glued");
        assert!(
            content.contains("\n---\n\nPROMPT\n\n## Run log\n"),
            "{content:?}"
        );
        assert_eq!(read_full(v, "glued").unwrap().1, "PROMPT");

        // An LF file saved with an unchanged prompt is byte-identical, twice.
        let w = upsert(
            v,
            None,
            "Idem Qx",
            "desc",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "The prompt.",
        )
        .unwrap();
        let created = raw(v, &w.slug);
        upsert_input(v, Some(&w.slug), &loaded_input(v, &w.slug)).unwrap();
        assert_eq!(raw(v, &w.slug), created, "the first save changes nothing");
        upsert_input(v, Some(&w.slug), &loaded_input(v, &w.slug)).unwrap();
        assert_eq!(raw(v, &w.slug), created, "nor does the second");
    }

    #[test]
    fn new_keys_parse_render_and_round_trip() {
        let fm = WorkflowFrontmatter {
            title: "Keys".into(),
            schedule: "manual".into(),
            permission_mode: "acceptEdits".into(),
            skip_if_quiet: true,
            watch: vec!["notes/**".into(), "projects/*.md".into()],
            notify: Some("failures".into()),
            last_started_at: Some("2026-09-01T07:00:00Z".into()),
            ..Default::default()
        };
        let content = render(&fm, "BODY", &[]);
        assert!(
            content.contains("\nskip_if_quiet: true\n"),
            "a bare boolean:\n{content}"
        );
        assert!(
            content.contains("\nwatch: notes/**, projects/*.md\n"),
            "{content}"
        );
        assert!(content.contains("\nnotify: failures\n"), "{content}");
        assert!(
            content.contains("\nlast_started_at: \"2026-09-01T07:00:00Z\"\n"),
            "{content}"
        );
        let (w, _, _) = parse_content("keys", &content).unwrap();
        assert!(w.skip_if_quiet);
        assert_eq!(w.watch, vec!["notes/**", "projects/*.md"]);
        assert_eq!(w.notify.as_deref(), Some("failures"));
        assert_eq!(w.last_started_at.as_deref(), Some("2026-09-01T07:00:00Z"));
        let (map, _) = split_frontmatter(&content);
        assert_eq!(frontmatter_from_map(&map), fm);

        for (value, expected) in [
            ("true", true),
            ("True", true),
            ("\"true\"", true),
            ("yes", true),
            ("on", true),
            ("1", true),
            ("false", false),
            ("no", false),
            ("", false),
        ] {
            let text = format!("---\ntitle: B\nskip_if_quiet: {value}\n---\n\nBODY\n");
            assert_eq!(
                parse_content("b", &text).unwrap().0.skip_if_quiet,
                expected,
                "{value:?}"
            );
        }

        // A workflow without the new keys renders exactly as before PR 9.
        let plain = WorkflowFrontmatter {
            title: "T".into(),
            schedule: "manual".into(),
            permission_mode: "acceptEdits".into(),
            ..Default::default()
        };
        assert_eq!(
            render(&plain, "P", &[]),
            "---\ntitle: T\nschedule: manual\npermission_mode: acceptEdits\ntags:\n  - workflows\n  - schedule/manual\n---\n\nP\n\n## Run log\n\n_no runs yet_\n"
        );
    }

    #[test]
    fn ui_save_never_clears_keys_the_caller_did_not_send() {
        let dir = temp_vault();
        let v = dir.path();
        let hand = [
            "notify: failures",
            "skip_if_quiet: true",
            "watch: notes/**, work/*.md",
        ];
        write_raw(
            v,
            "sent",
            &format!(
                "---\ntitle: Sent\nschedule: manual\n{}\n---\n\nBODY\n",
                hand.join("\n")
            ),
        );
        let mut input = loaded_input(v, "sent");
        input.description = "edited".into();
        let saved = upsert_input(v, Some("sent"), &input).unwrap();
        assert_lines_kept(&raw(v, "sent"), &hand, "a UI save");
        assert_eq!(saved.notify.as_deref(), Some("failures"));
        assert!(saved.skip_if_quiet);
        assert_eq!(saved.watch, vec!["notes/**", "work/*.md"]);
    }

    #[test]
    fn ui_save_of_a_deleted_automation_is_an_error_not_a_recreation() {
        let dir = temp_vault();
        let v = dir.path();
        let w = upsert(
            v,
            None,
            "Gone",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        let input = loaded_input(v, &w.slug);
        fs::remove_file(v.join(&w.path)).unwrap();
        let error = upsert_input(v, Some(&w.slug), &input).unwrap_err();
        assert!(error.starts_with(EDIT_CONFLICT_ERROR), "{error}");
        assert!(error.contains("no longer exists"), "{error}");
        let error = upsert_input(
            v,
            Some(&w.slug),
            &WorkflowInput {
                edit_base: None,
                ..input
            },
        )
        .unwrap_err();
        assert!(error.contains("no longer exists"), "{error}");
        assert!(
            !v.join(&w.path).exists(),
            "the automation is not silently recreated"
        );
    }

    /// `split_body` reads everything from the first `## Run log` as the log,
    /// so a prompt containing it used to lose its tail silently on the next
    /// read. The save now refuses and says why.
    #[test]
    fn a_prompt_containing_the_run_log_header_is_refused_not_truncated() {
        let dir = temp_vault();
        let v = dir.path();
        let w = upsert(
            v,
            None,
            "Header Prompt Qx",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        let before = raw(v, &w.slug);
        let mut input = loaded_input(v, &w.slug);
        input.prompt = "Summarise the week.\n## Run log\nthen stop".into();
        let error = upsert_input(v, Some(&w.slug), &input).unwrap_err();
        assert!(error.contains("## Run log"), "{error}");
        assert_eq!(raw(v, &w.slug), before, "nothing was written");

        // A rename with such a prompt is refused the same way.
        input.title = "Header Prompt Renamed Qx".into();
        let error = upsert_input(v, Some(&w.slug), &input).unwrap_err();
        assert!(error.contains("## Run log"), "{error}");
        assert_eq!(raw(v, &w.slug), before, "nothing was written");
        assert!(!v
            .join(WORKFLOWS_DIR)
            .join("header-prompt-renamed-qx.md")
            .exists());

        // And so is creating a new automation with one.
        let error = upsert(
            v,
            None,
            "Header Create Qx",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "Plan the day.\n## Run log\nthen stop",
        )
        .unwrap_err();
        assert!(error.contains("## Run log"), "{error}");
        assert!(!v.join(WORKFLOWS_DIR).join("header-create-qx.md").exists());
    }

    /// The textarea hands back the prompt as loaded (LF line breaks, maybe
    /// without trailing whitespace). A save that changes only another field
    /// must keep the prompt's bytes, including a first line's indentation.
    #[test]
    fn a_save_with_the_prompt_unchanged_keeps_its_bytes() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(v, "keeper", KEEPER);
        let mut input = loaded_input(v, "keeper");
        input.description = "Only the description changed".into();
        upsert_input(v, Some("keeper"), &input).unwrap();
        let content = raw(v, "keeper");
        assert!(
            content.contains(
                "---\n\n  Indented first line of the prompt.\nSecond line.\n\n## Run log"
            ),
            "the prompt kept its bytes:\n{content}"
        );
        assert_lines_kept(
            &content,
            &["description: Only the description changed"],
            "a save",
        );

        // A CRLF prompt handed back with LF breaks is unchanged too.
        write_raw(
            v,
            "crlfp",
            "---\r\ntitle: Crlfp\r\nschedule: manual\r\n---\r\n\r\n  First.\r\nSecond.  \r\n",
        );
        let mut input = loaded_input(v, "crlfp");
        input.prompt = input.prompt.replace("\r\n", "\n").trim_end().to_string();
        input.description = "Described".into();
        upsert_input(v, Some("crlfp"), &input).unwrap();
        let content = raw(v, "crlfp");
        assert!(
            content.ends_with("  First.\r\nSecond.  \r\n"),
            "{content:?}"
        );

        // An edited prompt keeps its first-line indentation and the file's
        // line breaks.
        let mut input = loaded_input(v, "crlfp");
        input.prompt = "  Edited.\nSecond line.".into();
        upsert_input(v, Some("crlfp"), &input).unwrap();
        let content = raw(v, "crlfp");
        assert!(
            content.contains("\r\n  Edited.\r\nSecond line.\r\n"),
            "{content:?}"
        );
        assert!(
            !content.replace("\r\n", "").contains('\n'),
            "no bare LF: {content:?}"
        );
        // The parser leaves a CRLF file's blank lines after the fence in the
        // body (pre-existing), so compare from the first prompt line.
        let (_, read, _) = read_full(v, "crlfp").unwrap();
        assert_eq!(
            read.replace("\r\n", "\n").trim_start_matches('\n'),
            "  Edited.\nSecond line.",
            "{content:?}"
        );
    }

    #[test]
    fn created_is_stamped_once_when_a_saved_file_has_none() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(
            v,
            "undated",
            "---\ntitle: Undated\nschedule: manual\n---\n\nBODY\n",
        );
        upsert_input(v, Some("undated"), &loaded_input(v, "undated")).unwrap();
        let created = read_full(v, "undated")
            .unwrap()
            .0
            .created
            .expect("created is stamped");
        upsert_input(v, Some("undated"), &loaded_input(v, "undated")).unwrap();
        assert_eq!(read_full(v, "undated").unwrap().0.created, Some(created));
    }

    #[test]
    fn a_rename_during_another_runs_tracked_window_is_not_flagged() {
        let dir = tempfile::Builder::new()
            .prefix("hollow-rename-tracked-")
            .tempdir()
            .unwrap();
        let v = dir.path();
        crate::workflow_permissions::initialize(v).unwrap();
        let w = upsert(
            v,
            None,
            "Tracked Src Qx",
            "",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        let run = crate::run_guard::begin_for_test(v, "chat", &[]);
        let mut input = loaded_input(v, &w.slug);
        input.title = "Tracked Dst Qx".into();
        let renamed = upsert_input(v, Some(&w.slug), &input).unwrap();
        // The stamp moved with the file (the old key is gone), so the old
        // path's disappearance is the editor's rename, not the run's delete.
        let flags = crate::run_guard::finish_for_test(v, &run);
        assert!(flags.is_empty(), "{flags:#?}");
        assert!(crate::workflow_permissions::verify(v, &renamed.slug).is_ok());
    }
    #[test]
    fn mark_started_stamps_both_start_fields() {
        use crate::workflow_permissions::PermissionFields;
        let dir = temp_vault();
        let v = dir.path();
        write_raw(
            v,
            "starter",
            "---\ntitle: Starter\nschedule: manual\nlast_status: error\nlast_error: boom\nlast_duration_sec: 12\nnotify: never\nowner: me\n---\n\nBODY\n",
        );
        let before = raw(v, "starter");
        let at = DateTime::parse_from_rfc3339("2026-09-25T08:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        mark_started(v, "starter", at).unwrap();
        let after = raw(v, "starter");
        let (w, prompt, log) = read_full(v, "starter").unwrap();
        assert_eq!(w.last_started_at.as_deref(), Some(at.to_rfc3339().as_str()));
        assert_eq!(w.last_run_at.as_deref(), Some(at.to_rfc3339().as_str()));
        assert_eq!(w.last_status.as_deref(), Some("running"));
        assert_eq!((w.last_error.as_deref(), w.last_duration_sec), (None, None));
        assert_eq!(prompt, "BODY");
        assert!(log.is_empty(), "a start writes no run-log row");
        assert_lines_kept(&after, &["notify: never", "owner: me"], "mark_started");
        // A bookkeeping write never moves a stamped permission field.
        assert_eq!(
            PermissionFields::from_content("starter", &before).unwrap(),
            PermissionFields::from_content("starter", &after).unwrap()
        );
        // A file gone since admission is an error the run reports.
        assert!(mark_started(v, "missing", at).is_err());
    }

    #[test]
    fn editor_notify_is_validated_and_always_means_the_default() {
        let dir = temp_vault();
        let v = dir.path();
        write_raw(
            v,
            "loud",
            "---\ntitle: Loud\nschedule: manual\nnotify: failures\n---\n\nBODY\n",
        );
        // None: the key is kept exactly (older callers, the create form).
        upsert_input(v, Some("loud"), &loaded_input(v, "loud")).unwrap();
        assert_lines_kept(
            &raw(v, "loud"),
            &["notify: failures"],
            "a save without notify",
        );
        // A value outside the three is refused and nothing is written.
        let untouched = raw(v, "loud");
        let mut input = loaded_input(v, "loud");
        input.notify = Some("sometimes".into());
        let error = upsert_input(v, Some("loud"), &input).unwrap_err();
        assert!(error.contains("always, failures, never"), "{error}");
        assert_eq!(raw(v, "loud"), untouched);
        // Trimmed and case-insensitive.
        let mut input = loaded_input(v, "loud");
        input.notify = Some(" Never ".into());
        assert_eq!(
            upsert_input(v, Some("loud"), &input)
                .unwrap()
                .notify
                .as_deref(),
            Some("never")
        );
        assert_lines_kept(&raw(v, "loud"), &["notify: never"], "a notify save");
        // `always` is the default: the key goes.
        let mut input = loaded_input(v, "loud");
        input.notify = Some("always".into());
        assert_eq!(upsert_input(v, Some("loud"), &input).unwrap().notify, None);
        assert!(!raw(v, "loud").contains("notify:"), "{}", raw(v, "loud"));
        // ...and an absent key stays absent: that save changes nothing.
        let quiet = raw(v, "loud");
        let mut input = loaded_input(v, "loud");
        input.notify = Some("always".into());
        upsert_input(v, Some("loud"), &input).unwrap();
        assert_eq!(raw(v, "loud"), quiet);
        // The editor baseline covers notify: a hand edit of it since the
        // editor loaded the file conflicts instead of being overwritten.
        let stale = loaded_input(v, "loud");
        write_raw(
            v,
            "loud",
            &quiet.replace("schedule: manual\n", "schedule: manual\nnotify: never\n"),
        );
        let error = upsert_input(v, Some("loud"), &stale).unwrap_err();
        assert!(error.starts_with(EDIT_CONFLICT_ERROR), "{error}");
        // A new automation can be created with a policy.
        let created = upsert_input(
            v,
            None,
            &WorkflowInput {
                title: "Created Quiet Qx".into(),
                notify: Some("failures".into()),
                edit_base: None,
                ..loaded_input(v, "loud")
            },
        )
        .unwrap();
        assert_eq!(created.notify.as_deref(), Some("failures"));
    }

    /// Paperclip PR 8a: consecutive budget skips of one pause episode read as
    /// ONE row (the first), each still consuming its occurrence; a new episode
    /// or anything in between starts anew.
    #[test]
    fn consecutive_budget_skips_log_once_and_every_occurrence_advances() {
        let dir = temp_vault();
        let v = dir.path();
        let w = seed_workflow(v, "Budget Skip Coalesce", "0 0 * * * * *", "BODY");
        let now = Utc::now();
        let episode = "2026-09-25T10:00:00.000Z";
        let ep = Some(episode);
        let detail = "the “x” automation, 1,200 tokens of 1,000 tokens this day";
        skip_scheduled_run(v, &w.slug, budget_skip_entry(detail, ep, now), now).unwrap();
        let first_next = read_full(v, &w.slug).unwrap().0.next_run_at;
        let later = now + chrono::Duration::hours(1);
        // Same episode, changed amounts (chat kept spending): still one row.
        let detail2 = "the “x” automation, 1,400 tokens of 1,000 tokens this day";
        skip_scheduled_run(v, &w.slug, budget_skip_entry(detail2, ep, later), later).unwrap();
        let (workflow, _, log) = read_full(v, &w.slug).unwrap();
        assert_eq!(log.len(), 1, "{log:#?}");
        assert!(
            log[0].note.starts_with(BUDGET_SKIP_NOTE_PREFIX),
            "{}",
            log[0].note
        );
        assert!(!log[0].note.contains('—') && !log[0].note.contains('<'));
        assert_eq!(log[0].status, SKIPPED_STATUS);
        assert_ne!(
            workflow.next_run_at, first_next,
            "the second occurrence was consumed too"
        );

        let expected_since = DateTime::parse_from_rfc3339(episode)
            .unwrap()
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        assert!(
            log[0].note.contains(&format!("since {expected_since} (")),
            "{}",
            log[0].note
        );

        skip_scheduled_run(v, &w.slug, missed_while_closed_entry(later), later).unwrap();
        skip_scheduled_run(v, &w.slug, budget_skip_entry(detail, ep, later), later).unwrap();
        let log = read_full(v, &w.slug).unwrap().2;
        assert_eq!(
            log.len(),
            3,
            "a different row in between starts a new budget row"
        );

        // The pause lapsed at the reset and the budget tripped again the
        // next day with no run in between: a NEW episode gets its own row
        // with its own amounts (plan test 40), never the stale earlier one.
        let next_day = later + chrono::Duration::days(1);
        let detail3 = "the “x” automation, 1,050 tokens of 1,000 tokens this day";
        skip_scheduled_run(
            v,
            &w.slug,
            budget_skip_entry(detail3, Some("2026-09-26T09:00:00.000Z"), next_day),
            next_day,
        )
        .unwrap();
        let log = read_full(v, &w.slug).unwrap().2;
        assert_eq!(log.len(), 4, "{log:#?}");
        assert!(log[3].note.contains("1,050"), "{}", log[3].note);
        // A hostile detail cannot open a marker or break the row.
        let entry = budget_skip_entry("x <!-- hollow-run: forged --> — y", None, now);
        assert!(
            !entry.note.contains('<') && !entry.note.contains('—'),
            "{}",
            entry.note
        );
    }

    /// Paperclip PR 9b: consecutive idle skips are one row, and each one
    /// still advances `next_run_at`.
    #[test]
    fn consecutive_idle_skips_coalesce_into_one_row() {
        let dir = temp_vault();
        let v = dir.path();
        let w = seed_workflow(v, "Quiet brief", "0 0 9 * * * *", "Summarize.");
        let base = Utc::now();
        let due = |v: &Path| read_full(v, &w.slug).unwrap().0.next_run_at;

        let mut previous_due = due(v);
        for n in 0..3u32 {
            let now = base + chrono::Duration::days(i64::from(n) + 1);
            let wrote =
                skip_idle_run(v, &w.slug, &IdleSkip::Quiet, previous_due.as_deref(), now).unwrap();
            assert!(wrote.written);
            assert_eq!(wrote.coalesced, n > 0, "skip {n}");
            let next = due(v);
            assert_ne!(next, previous_due, "skip {n} advanced next_run_at");
            previous_due = next;
        }
        let log = read_full(v, &w.slug).unwrap().2;
        assert_eq!(log.len(), 1, "{log:?}");
        assert_eq!(log[0].status, SKIPPED_STATUS);
        assert_eq!(log[0].note, "no changes since last run · 3 skips");
        assert!(is_benign_skip_note(&log[0].note));

        // A run row in between starts a new row.
        append_run_log(
            v,
            &w.slug,
            RunLogEntry {
                at: Utc::now().to_rfc3339(),
                status: "success".into(),
                duration_sec: 4,
                note: "wrote a brief".into(),
                run_id: Some("run-1".into()),
                error_tail: None,
                exit_kind: None,
            },
        )
        .unwrap();
        let wrote = skip_idle_run(v, &w.slug, &IdleSkip::Quiet, None, Utc::now()).unwrap();
        assert!(wrote.written && !wrote.coalesced);
        // An inactive skip after a quiet one starts a new row too, and a
        // second inactive skip for the same status coalesces with it.
        let inactive = IdleSkip::Inactive {
            workspace: "Launch plan".into(),
            status: "done".into(),
        };
        assert!(
            !skip_idle_run(v, &w.slug, &inactive, None, Utc::now())
                .unwrap()
                .coalesced
        );
        assert!(
            skip_idle_run(v, &w.slug, &inactive, None, Utc::now())
                .unwrap()
                .coalesced
        );
        let log = read_full(v, &w.slug).unwrap().2;
        let notes: Vec<&str> = log.iter().map(|e| e.note.as_str()).collect();
        assert_eq!(
            notes,
            vec![
                "no changes since last run · 3 skips",
                "wrote a brief",
                "no changes since last run",
                "workspace inactive: Launch plan is done · 2 skips",
            ]
        );
        for note in &notes[2..] {
            assert!(
                !note.starts_with("not run:") && !note.contains("missed while"),
                "{note}"
            );
        }

        // An occurrence that is no longer the next run writes nothing.
        let before = raw(v, &w.slug);
        let wrote = skip_idle_run(
            v,
            &w.slug,
            &IdleSkip::Quiet,
            Some("2020-01-01T09:00:00+00:00"),
            Utc::now(),
        )
        .unwrap();
        assert!(!wrote.written);
        assert_eq!(raw(v, &w.slug), before);
    }

    /// Paperclip PR 9b (refresh C5): idle skips are bookkeeping. They move
    /// `next_run_at` and the run log only, so the permission fields stay put
    /// and the workflow still verifies.
    #[test]
    fn idle_skips_keep_the_permission_fields_and_verify() {
        use crate::workflow_permissions::PermissionFields;
        let dir = temp_vault();
        let v = dir.path();
        let w = seed_workflow_with(v, "Quiet guard", "0 0 9 * * * *", "Check it.", |fm| {
            fm.skip_if_quiet = true;
            fm.watch = vec!["notes/**".into()];
        });
        let before = PermissionFields::from_content(&w.slug, &raw(v, &w.slug)).unwrap();
        for day in 1..=3 {
            skip_idle_run(
                v,
                &w.slug,
                &IdleSkip::Quiet,
                None,
                Utc::now() + chrono::Duration::days(day),
            )
            .unwrap();
        }
        let inactive = IdleSkip::Inactive {
            workspace: "Old work".into(),
            status: "archived".into(),
        };
        skip_idle_run(v, &w.slug, &inactive, None, Utc::now()).unwrap();
        let after = PermissionFields::from_content(&w.slug, &raw(v, &w.slug)).unwrap();
        assert_eq!(before, after);
        let (parsed, _, _) = read_full(v, &w.slug).unwrap();
        assert!(parsed.skip_if_quiet);
        assert_eq!(parsed.watch, vec!["notes/**"]);
        assert!(crate::workflow_permissions::verify(v, &w.slug).is_ok());
    }

    /// Paperclip PR 9b: the editor may turn quiet-skip on only with a watch
    /// its check can walk, and a watch glob may not contain a comma.
    #[test]
    fn a_quiet_skip_save_is_validated() {
        let dir = temp_vault();
        let v = dir.path();
        let w = seed_workflow(v, "Quiet Save Qx", "manual", "BODY");
        let before = raw(v, &w.slug);
        let mut input = loaded_input(v, &w.slug);
        input.skip_if_quiet = Some(true);
        let error = upsert_input(v, Some(&w.slug), &input).unwrap_err();
        assert!(error.starts_with("Skip when nothing changed:"), "{error}");
        assert_eq!(raw(v, &w.slug), before, "nothing was written");

        input.watch = Some(vec!["notes/**".into(), "a,b/*".into()]);
        let error = upsert_input(v, Some(&w.slug), &input).unwrap_err();
        assert!(error.contains("comma"), "{error}");

        input.watch = Some(vec!["_inbox/**".into()]);
        assert!(upsert_input(v, Some(&w.slug), &input).is_err());

        input.watch = Some(vec![" notes/** ".into(), "".into()]);
        let saved = upsert_input(v, Some(&w.slug), &input).unwrap();
        assert!(saved.skip_if_quiet);
        assert_eq!(saved.watch, vec!["notes/**"]);
        assert!(raw(v, &w.slug).contains("\nskip_if_quiet: true\n"));

        // Turning it off again removes the key; the watch stays unless sent.
        let mut off = loaded_input(v, &w.slug);
        off.skip_if_quiet = Some(false);
        let saved = upsert_input(v, Some(&w.slug), &off).unwrap();
        assert!(!saved.skip_if_quiet);
        assert_eq!(saved.watch, vec!["notes/**"]);
        assert!(!raw(v, &w.slug).contains("skip_if_quiet"));

        // Review: moving a quiet workspace automation out of its workspace
        // (only `cwd` sent changed) loses the default watch, so it is refused.
        let workspace = crate::workspaces::create(v, "Quiet Home", "Goal").unwrap();
        let cwd = workspace.cwd.clone();
        let ws = seed_workflow_with(v, "Quiet Save Cwd", "manual", "BODY", |fm| {
            fm.cwd = Some(cwd);
            fm.skip_if_quiet = true;
        });
        let before = raw(v, &ws.slug);
        let mut moved = loaded_input(v, &ws.slug);
        assert!(
            moved.skip_if_quiet.is_none() && moved.watch.is_none(),
            "quiet fields not sent"
        );
        moved.cwd = Some(v.to_string_lossy().into_owned());
        let error = upsert_input(v, Some(&ws.slug), &moved).unwrap_err();
        assert!(error.starts_with("Skip when nothing changed:"), "{error}");
        assert_eq!(raw(v, &ws.slug), before, "nothing was written");

        // A save that leaves an already-broken config as it was still saves.
        let broken = seed_workflow_with(v, "Quiet Save Broken", "manual", "BODY", |fm| {
            fm.skip_if_quiet = true;
        });
        let mut edit = loaded_input(v, &broken.slug);
        edit.description = "a new description".into();
        upsert_input(v, Some(&broken.slug), &edit).unwrap();
    }

    // ── End rules (G2) ──────────────────────────────────────────────────────

    #[test]
    fn an_end_rule_round_trips_and_old_files_are_unchanged() {
        let dir = temp_vault();
        let v = dir.path();
        let plain = seed_workflow(v, "No End N", "manual", "BODY");
        assert!(plain.ends.is_none() && plain.after_runs.is_none() && plain.ended_note.is_none());
        let before = raw(v, &plain.slug);
        // A save that sends no end key leaves the file's bytes alone.
        upsert_input(v, Some(&plain.slug), &loaded_input(v, &plain.slug)).unwrap();
        let after = raw(v, &plain.slug);
        for key in ["ends", "after_runs", "runs_counted", "ended_note"] {
            assert!(!after.contains(&format!("\n{key}:")), "{after}");
        }
        assert_eq!(
            before.lines().filter(|l| l.starts_with("title:")).count(),
            after.lines().filter(|l| l.starts_with("title:")).count()
        );

        let at = (Utc::now() + chrono::Duration::days(2)).to_rfc3339();
        let mut input = loaded_input(v, &plain.slug);
        input.ends = Some(at.clone());
        let saved = upsert_input(v, Some(&plain.slug), &input).unwrap();
        assert_eq!(saved.ends.as_deref(), Some(at.as_str()));
        assert!(raw(v, &plain.slug).contains("\nends: "));
        // Omitted keeps it; empty clears it.
        let kept = upsert_input(v, Some(&plain.slug), &loaded_input(v, &plain.slug)).unwrap();
        assert_eq!(kept.ends.as_deref(), Some(at.as_str()));
        let mut clear = loaded_input(v, &plain.slug);
        clear.ends = Some(String::new());
        assert!(upsert_input(v, Some(&plain.slug), &clear)
            .unwrap()
            .ends
            .is_none());
        assert!(!raw(v, &plain.slug).contains("\nends:"));
    }

    #[test]
    fn a_save_refuses_a_bad_past_or_doubled_end_rule() {
        let dir = temp_vault();
        let v = dir.path();
        let w = seed_workflow(v, "End Rules R", "manual", "BODY");
        let before = raw(v, &w.slug);
        let try_save = |edit: &dyn Fn(&mut WorkflowInput)| {
            let mut input = loaded_input(v, &w.slug);
            edit(&mut input);
            upsert_input(v, Some(&w.slug), &input).unwrap_err()
        };
        let bad = try_save(&|i| i.ends = Some("next friday".into()));
        assert!(bad.contains("not a date and time"), "{bad}");
        let past =
            try_save(&|i| i.ends = Some((Utc::now() - chrono::Duration::hours(1)).to_rfc3339()));
        assert!(past.contains("already passed"), "{past}");
        let both = try_save(&|i| {
            i.ends = Some((Utc::now() + chrono::Duration::days(1)).to_rfc3339());
            i.after_runs = Some(3);
        });
        assert!(both.contains("not both"), "{both}");
        let huge = try_save(&|i| i.after_runs = Some(MAX_AFTER_RUNS + 1));
        assert!(huge.contains("at most"), "{huge}");
        assert_eq!(raw(v, &w.slug), before, "nothing was written");

        // A local date-time without an offset reads in this Mac's zone.
        let local = (chrono::Local::now() + chrono::Duration::days(3)).format("%Y-%m-%dT%H:%M");
        assert!(parse_end(&local.to_string()).is_ok());
        let mut input = loaded_input(v, &w.slug);
        input.after_runs = Some(3);
        let saved = upsert_input(v, Some(&w.slug), &input).unwrap();
        assert_eq!(saved.after_runs, Some(3));
        assert_eq!(saved.runs_counted, None);
    }

    #[test]
    fn the_nth_finished_run_ends_the_automation_and_it_stays_runnable_by_hand() {
        let dir = temp_vault();
        let v = dir.path();
        let w = seed_workflow_with(v, "Third Run T", "0 0 7 * * * *", "BODY", |fm| {
            fm.after_runs = Some(2);
        });
        let next = next_run_after(&w.schedule, Utc::now());
        finalize_run(v, &w.slug, "success", None, 1, next, log_entry("one")).unwrap();
        let (mid, _, _) = read_full(v, &w.slug).unwrap();
        assert_eq!(mid.runs_counted, Some(1));
        assert_eq!(mid.schedule, "0 0 7 * * * *");
        assert!(mid.ended_note.is_none());

        finalize_run(v, &w.slug, "error", Some("boom"), 1, next, log_entry("two")).unwrap();
        let (done, _, log) = read_full(v, &w.slug).unwrap();
        assert_eq!(done.schedule, "manual");
        assert!(done.next_run_at.is_none());
        assert!(done.after_runs.is_none() && done.runs_counted.is_none());
        let note = done.ended_note.expect("the end is noted");
        assert!(note.contains("it finished its 2 runs"), "{note}");
        assert_eq!(log.len(), 2, "the run log keeps both runs");
        let text = raw(v, &w.slug);
        assert!(text.contains("schedule/manual"), "{text}");
        assert!(
            crate::workflows::precheck(v, &w.slug).is_ok(),
            "the schedule change was re-stamped, so Run now still works"
        );
    }

    #[test]
    fn clearing_the_end_fields_restarts_an_ended_trigger_only_automation() {
        let dir = temp_vault();
        let v = dir.path();
        let note = "Ended earlier because its end time passed.";
        let w = seed_workflow_with(v, "Ended Restart R", "manual", "BODY", |fm| {
            fm.triggers = vec!["task-overdue".into()];
            fm.ended_note = Some(note.into());
        });
        // A plain save (nothing about the end sent, still manual) keeps it.
        let kept = upsert_input(v, Some(&w.slug), &loaded_input(v, &w.slug)).unwrap();
        assert_eq!(kept.ended_note.as_deref(), Some(note));
        // Cleared end fields clear the note though no rule remains.
        let mut cleared = loaded_input(v, &w.slug);
        cleared.ends = Some(String::new());
        assert!(upsert_input(v, Some(&w.slug), &cleared)
            .unwrap()
            .ended_note
            .is_none());

        let again = seed_workflow_with(v, "Ended Restart Two", "manual", "BODY", |fm| {
            fm.triggers = vec!["task-overdue".into()];
            fm.ended_note = Some(note.into());
        });
        let mut runs = loaded_input(v, &again.slug);
        runs.after_runs = Some(0);
        assert!(upsert_input(v, Some(&again.slug), &runs)
            .unwrap()
            .ended_note
            .is_none());
        assert!(!raw(v, &again.slug).contains("ended_note"));
    }
}
