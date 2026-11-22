//! Friction → fix loop (master plan #13, Phase 4): where Hollow keeps hitting
//! the same wall, counted across runs, and a way to draft a fix that only a
//! person starts.
//!
//! **Signals** ([`collect`]), the last [`WINDOW_DAYS`] days:
//! - a failed step of a chat turn (the F6 step spine's `error` steps: the
//!   tool's name and its result), and a shell step that exited cleanly but
//!   whose output shows a check's failure (OF16: `npx vitest run | tail`
//!   printing `Tests  2 failed`; the spine's `checkFailed`);
//! - a Hollow app action that failed or was refused (`.app-actions.jsonl` and
//!   its rotated copy; never the person's own Deny, which is a decision, not
//!   friction; a card that timed out is friction);
//! - the person's explicit Not useful vote on a turn (`turn_feedback`), with
//!   its reason. Nothing is read from their notes or chats: a correction in
//!   chat counts only through that vote (decision: the plan's
//!   recommendation).
//!
//! **Signatures** ([`signature`]): tool, error class and the message
//! normalised (secrets redacted, then paths, ids, hashes and numbers replaced,
//! case and spacing folded). A shell command that exited non-zero is signed
//! by its exit code and command head instead ([`signed_as`]). A different
//! tool, class or message is a different pattern; nothing is merged by
//! similarity.
//!
//! **Stored** in the session store's `friction` table ([`SCHEMA`]): one row
//! per signature with its counts, chats, first and last time, up to
//! [`EXAMPLES`] redacted examples, and its status (`open`, `dismissed`,
//! `drafted`). [`refresh`] recomputes the counts (the nightly window job,
//! and every read) and keeps the statuses.
//!
//! **Noise** ([`routine_exits_dropped`], G3): a shell exit code alone
//! (class `exit-code`) is a pattern only when its command was a check
//! (`checkScopes`: the exit is a check failing) or it recurs in
//! [`EXIT_MIN_CHATS`] or more chats on [`EXIT_MIN_DAYS`] or more days. A
//! failed check read from the output (`check-failed`) always counts.
//! [`list_view`] puts new and growing patterns first ([`trend`]).
//!
//! **Draft a fix only proposes.** `friction.draft_fix` asks on a card in
//! every permission mode and is refused from automations and Main agent
//! check-ins (`app_actions::ALWAYS_ASK`). Its one visible home is the weekly
//! review's Do it card (the Settings, Usage, Friction page and its
//! user-only commands were cut, G2). It creates a build workspace on a new
//! local branch and starts one ordinary chat there with a task packet: the
//! pattern, its examples, red-first, the gate, the two-slot rule, and never
//! push, open a pull request or install. At most [`WEEKLY_DRAFTS`] drafts in
//! 7 days. Nothing here pushes, opens a pull request, runs the bed or
//! installs.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS friction( \
   signature TEXT PRIMARY KEY, \
   tool TEXT NOT NULL, \
   error_class TEXT NOT NULL, \
   message TEXT NOT NULL, \
   count INTEGER NOT NULL, \
   chats INTEGER NOT NULL, \
   week_count INTEGER NOT NULL DEFAULT 0, \
   first_seen TEXT NOT NULL, \
   last_seen TEXT NOT NULL, \
   examples TEXT NOT NULL, \
   sources TEXT NOT NULL, \
   status TEXT NOT NULL DEFAULT 'open', \
   drafted_session TEXT, \
   drafted_at TEXT, \
   updated_at TEXT NOT NULL \
 );";

/// How far back the counts look.
pub const WINDOW_DAYS: i64 = 30;
/// Examples kept per pattern.
pub const EXAMPLES: usize = 5;
/// Drafts a week (decision: conservative; the plan left it open).
pub const WEEKLY_DRAFTS: usize = 3;
/// A pattern the weekly view headlines: at least this many hits in 7 days.
pub const WEEKLY_MIN: i64 = 3;
/// A shell exit code alone counts only when it recurs in at least this many
/// chats...
pub const EXIT_MIN_CHATS: usize = 3;
/// ...on at least this many days (G3).
pub const EXIT_MIN_DAYS: usize = 2;
/// Patterns `friction.list` returns.
pub const LIST_CAP: usize = 200;
const MESSAGE_CHARS: usize = 160;
const EXCERPT_CHARS: usize = 200;

/// One observed failure.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub at: String,
    /// `step`, `action` or `feedback`.
    pub source: &'static str,
    pub tool: String,
    pub message: String,
    pub session_key: Option<String>,
    pub run_id: Option<String>,
    /// A step's shell command (from its checkpoint's input), so an exit is
    /// signed by the command's head ([`signed_as`]).
    pub command: Option<String>,
    /// A shell step that exited cleanly but whose output shows a check's
    /// failure (OF16: the step spine's `checkFailed`, a test run piped into
    /// `tail` or behind `|| true`).
    pub check_failed: bool,
    /// A shell step whose command held a check (the checkpoint's
    /// `checkScopes`: a test run, a typecheck, a lint): its non-zero exit is
    /// that check failing, so [`routine_exits_dropped`] keeps it.
    pub check_run: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Example {
    pub at: String,
    pub source: String,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pattern {
    pub signature: String,
    pub tool: String,
    pub error_class: String,
    pub message: String,
    pub count: i64,
    pub chats: i64,
    pub week_count: i64,
    pub first_seen: String,
    pub last_seen: String,
    pub examples: Vec<Example>,
    pub sources: Vec<String>,
    pub status: String,
    pub drafted_session: Option<String>,
    pub drafted_at: Option<String>,
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// The message with secrets redacted and the parts that vary between
/// occurrences of the same wall replaced: paths, session and run ids, UUIDs,
/// long hex, numbers. Quoted values stay (`priority 'urgent'` is the wall).
pub fn normalize(message: &str) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static PATH: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"(?:~|\.{0,2})?/[^\s'"`,;:)\]]+"#).unwrap());
    static UUID: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap()
    });
    static IDS: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\b(?:s|run|turn|bakeoff|swarm)-[0-9a-z-]{4,}\b").unwrap());
    static HEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[0-9a-f]{8,}\b").unwrap());
    static NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d+(?:[.,]\d+)*\b").unwrap());
    static SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
    let first = message
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let text = crate::claude_runner::redact(first).to_lowercase();
    let text = UUID.replace_all(&text, "<id>");
    let text = PATH.replace_all(&text, "<path>");
    let text = IDS.replace_all(&text, "<id>");
    let text = HEX.replace_all(&text, "<hex>");
    let text = NUM.replace_all(&text, "<n>");
    let text = SPACE.replace_all(&text, " ");
    cut(text.trim(), MESSAGE_CHARS)
}

/// The error's class: an UPPER_SNAKE code it starts with, else the first
/// matching family, else `error`.
pub fn error_class(message: &str) -> String {
    let first = message.trim_start();
    let code: String = first
        .chars()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
        .collect();
    if code.len() >= 4 && code.contains('_') {
        return code.to_lowercase();
    }
    let lower = first.to_lowercase();
    const FAMILIES: [(&str, &[&str]); 7] = [
        (
            "permission",
            &[
                "permission",
                "denied",
                "not allowed",
                "refused",
                "isn't allowed",
                // Claude Code's approval refusals (HF02): a wall of the
                // permission setup, not a failed command.
                "requires approval",
                "require approval",
                "requires explicit approval",
                // Claude Code's wording; a bare "was blocked" is too common
                // ("the main thread was blocked on IO").
                "was blocked. for security",
                "simple_expansion",
                "changes directory before running",
                "haven't granted",
            ],
        ),
        ("timeout", &["timed out", "timeout", "timed_out"]),
        (
            "conflict",
            &["conflict", "no longer matches", "already exists"],
        ),
        (
            "not-found",
            &[
                "not found",
                "no such",
                "doesn't exist",
                "does not exist",
                "unknown",
            ],
        ),
        (
            "invalid",
            &["invalid", "must be", "is not a", "malformed", "expected"],
        ),
        (
            "limit",
            &["limit", "quota", "rate", "cap reached", "too large"],
        ),
        ("network", &["network", "connection", "unreachable", "http"]),
    ];
    FAMILIES
        .iter()
        .find(|(_, words)| words.iter().any(|w| lower.contains(w)))
        .map(|(class, _)| class.to_string())
        .unwrap_or_else(|| "error".into())
}

/// The command a shell step ran, from its checkpoint's `input`: the tool
/// input as JSON text, cut at 300 characters, so a long one is unterminated
/// and its `command` is read as far as it goes.
pub fn shell_command(input: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<Value>(input) {
        return v.get("command").and_then(Value::as_str).map(str::to_string);
    }
    const KEY: &str = "\"command\":\"";
    let rest = &input[input.find(KEY)? + KEY.len()..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next() {
                Some('n' | 't' | 'r') => out.push(' '),
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

/// Launchers whose subcommand is part of a command's head.
const LAUNCHERS: &[&str] = &[
    "npx", "npm", "pnpm", "yarn", "bun", "bunx", "cargo", "git", "gh", "uv", "uvx",
];

/// `NAME=value`: an environment assignment before a command.
fn is_assignment(word: &str) -> bool {
    let name = word.split_once('=').map_or("", |(name, _)| name);
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A shell command's head, lowercased: its first word past leading
/// `NAME=value` assignments and a leading `cd <dir> &&`, with a launcher's
/// subcommand (`npx vitest`, `cargo test`; never a flag).
/// Words that run the command after them (`env`, `time`, …), skipped.
const WRAPPERS: &[&str] = &["env", "time", "sudo", "nohup", "exec"];

pub fn command_head(command: &str) -> Option<String> {
    // `zsh -lc 'git push'` signs as the script it runs, not as `zsh` (OF13).
    let command = crate::run_effects::unwrap_shell_wrapper(command);
    let words: Vec<&str> = command.split_whitespace().collect();
    let mut i = 0;
    while let Some(w) = words.get(i) {
        if is_assignment(w) || WRAPPERS.contains(w) {
            i += 1;
        } else if *w == "timeout" {
            // `timeout 60 cmd`: the duration, then the command.
            i += 2;
        } else if *w == "cd" && words.get(i + 2).is_some_and(|w| ["&&", ";"].contains(w)) {
            i += 3;
        } else if *w == "cd" && words.get(i + 1).is_some_and(|w| w.ends_with(';')) {
            // `cd dir; cmd`
            i += 2;
        } else if ["set", "source", "."].contains(w) {
            // `set -euo pipefail; cmd`, `source env.sh && cmd`: through the `;`.
            let rest = &words[i..];
            match rest.iter().position(|x| x.ends_with(';') || *x == "&&") {
                Some(end) => i += end + 1,
                None => break,
            }
        } else {
            break;
        }
    }
    // A script named by its path signs by its file name: no home folder in
    // the pattern, one pattern per script wherever it is called from.
    let word = words.get(i)?.trim_end_matches(';');
    let first = word.rsplit('/').next().unwrap_or(word).to_lowercase();
    if first.is_empty() {
        return None;
    }
    // `git -C dir push`: the subcommand comes after the folder.
    let at = if first == "git" && words.get(i + 1) == Some(&"-C") {
        i + 3
    } else {
        i + 1
    };
    let sub = words.get(at).filter(|w| {
        LAUNCHERS.contains(&first.as_str())
            && w.starts_with(|c: char| c.is_ascii_alphabetic())
            && !w.contains(['/', ':'])
    });
    Some(match sub {
        Some(s) => format!("{first} {}", s.trim_end_matches(';').to_lowercase()),
        None => first,
    })
}

/// A line that reads `Exit code N`, optionally followed by a parenthesised
/// note (Codex's `Exit code N (no output)`, OF18,
/// `agent::codex_command_result`): (the code, the note, lowercased). The
/// code may be negative (`Exit code -1`, a process killed or never
/// started), so such a failure signs and drops like any other (OF18
/// review).
fn exit_line(line: &str) -> Option<(String, String)> {
    let rest = line.trim().to_ascii_lowercase();
    let rest = rest.strip_prefix("exit code ")?.trim();
    let (code, note) = rest.split_once(' ').unwrap_or((rest, ""));
    let note = note.trim();
    let digits = code.strip_prefix('-').unwrap_or(code);
    let digits = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());
    let note_ok = note.is_empty() || note.starts_with('(');
    (digits && note_ok).then(|| (code.to_string(), note.to_string()))
}

/// The code of a result that starts with `Exit code N` ([`exit_line`]).
fn exit_code(message: &str) -> Option<String> {
    let first = message.lines().map(str::trim).find(|l| !l.is_empty())?;
    exit_line(first).map(|(code, _)| code)
}

/// Whether a shell result's first line is its exit code alone: Claude's
/// `Exit code N`, or Codex's `Exit code N (no output)` (OF18), any code.
fn bare_exit_line(line: &str) -> bool {
    exit_line(line).is_some_and(|(_, note)| note.is_empty() || note == "(no output)")
}

/// A shell step that exited non-zero with its command known: (exit code,
/// command head).
fn shell_exit(s: &Signal) -> Option<(String, String)> {
    let tool = s.tool.trim().to_ascii_lowercase();
    if !SHELL_TOOLS.contains(&tool.as_str()) {
        return None;
    }
    let code = exit_code(&s.message)?;
    Some((code, s.command.as_deref().and_then(command_head)?))
}

/// What a signal is signed by: its error class and normalised message. A
/// shell step that exited non-zero, with its command known, is signed by the
/// exit code and the command's head instead (class `exit-code`, message
/// `exit code 1 from npx vitest`): its first output line was a test runner's
/// banner, a warning or a heading as often as the error, and each became a
/// pattern of its own (HF02). A clean exit whose output failed a check
/// (OF16) is signed alike: class `check-failed`, `failed check from cargo
/// test`.
pub fn signed_as(s: &Signal) -> (String, String) {
    if s.check_failed {
        if let Some(head) = s.command.as_deref().and_then(command_head) {
            return ("check-failed".into(), format!("failed check from {head}"));
        }
    }
    if let Some((code, head)) = shell_exit(s) {
        return ("exit-code".into(), format!("exit code {code} from {head}"));
    }
    (error_class(&s.message), normalize(&s.message))
}

/// A pattern's signature: 16 hex digits of SHA-256 over tool, class and
/// normalised message.
pub fn signature(tool: &str, class: &str, message: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{tool}\n{class}\n{message}").as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Group signals into patterns, most frequent first. `week_from`: the start
/// of the weekly count.
pub fn aggregate(signals: &[Signal], week_from: &str) -> Vec<Pattern> {
    struct Acc {
        p: Pattern,
        chats: BTreeSet<String>,
        sources: BTreeSet<String>,
    }
    let mut by: BTreeMap<String, Acc> = BTreeMap::new();
    for s in signals {
        let (class, message) = signed_as(s);
        let tool = s.tool.trim().to_string();
        let sig = signature(&tool, &class, &message);
        let acc = by.entry(sig.clone()).or_insert_with(|| Acc {
            p: Pattern {
                signature: sig,
                tool: tool.clone(),
                error_class: class.clone(),
                message: message.clone(),
                count: 0,
                chats: 0,
                week_count: 0,
                first_seen: s.at.clone(),
                last_seen: s.at.clone(),
                examples: Vec::new(),
                sources: Vec::new(),
                status: "open".into(),
                drafted_session: None,
                drafted_at: None,
            },
            chats: BTreeSet::new(),
            sources: BTreeSet::new(),
        });
        acc.p.count += 1;
        if s.at.as_str() >= week_from {
            acc.p.week_count += 1;
        }
        if s.at < acc.p.first_seen {
            acc.p.first_seen = s.at.clone();
        }
        if s.at > acc.p.last_seen {
            acc.p.last_seen = s.at.clone();
        }
        if let Some(k) = &s.session_key {
            acc.chats.insert(k.clone());
        }
        acc.sources.insert(s.source.to_string());
        acc.p.examples.push(Example {
            at: s.at.clone(),
            source: s.source.to_string(),
            session_key: s.session_key.clone(),
            run_id: s.run_id.clone(),
            excerpt: cut(
                &crate::claude_runner::redact(s.message.trim()),
                EXCERPT_CHARS,
            ),
        });
    }
    let mut out: Vec<Pattern> = by
        .into_values()
        .map(|mut acc| {
            acc.p.examples.sort_by(|a, b| b.at.cmp(&a.at));
            acc.p.examples.truncate(EXAMPLES);
            acc.p.chats = acc.chats.len() as i64;
            acc.p.sources = acc.sources.into_iter().collect();
            acc.p
        })
        .collect();
    out.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(b.last_seen.cmp(&a.last_seen))
            .then(a.signature.cmp(&b.signature))
    });
    out
}

/// The audit log's failed or refused actions since `since` (never the
/// person's own Deny).
pub fn audit_signals(text: &str, since: &str) -> Vec<Signal> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("ok").and_then(Value::as_bool) == Some(false))
        .filter(|v| v.get("card").and_then(Value::as_str) != Some("denied"))
        .filter_map(|v| {
            let at = v.get("ts")?.as_str()?.to_string();
            if at.as_str() < since {
                return None;
            }
            let error = v
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    v.get("card")
                        .and_then(Value::as_str)
                        .map(|c| format!("card {c}"))
                })?;
            Some(Signal {
                at,
                source: "action",
                tool: v.get("action")?.as_str()?.to_string(),
                message: error,
                session_key: v
                    .get("scope")
                    .and_then(Value::as_str)
                    .filter(|s| s.starts_with("s-"))
                    .map(str::to_string),
                run_id: v.get("runId").and_then(Value::as_str).map(str::to_string),
                command: None,
                check_failed: false,
                check_run: false,
            })
        })
        .collect()
}

/// Every signal since `since`.
pub fn collect(vault: &Path, since: &str) -> Vec<Signal> {
    let mut out = Vec::new();
    // Failed steps (F6), and shell steps that exited cleanly with a check's
    // failure in their output (OF16, the step spine's `checkFailed`).
    let steps: Vec<Signal> = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT started_at, COALESCE(name, kind), checkpoint, session_key, run_id, status FROM steps WHERE (status = 'error' OR (status = 'ok' AND checkpoint LIKE '%\"checkFailed\":true%')) AND started_at >= ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![since], |r| {
                let checkpoint: Option<Value> = r
                    .get::<_, Option<String>>(2)?
                    .and_then(|c| serde_json::from_str(&c).ok());
                let field = |key: &str| {
                    checkpoint
                        .as_ref()
                        .and_then(|c| c.get(key).and_then(Value::as_str).map(str::to_string))
                };
                // An empty result: a tool that said nothing, or a Codex shell
                // step recorded before OF18 gave its results an exit code.
                let result = field("result").unwrap_or_default();
                let status: String = r.get(5)?;
                Ok(Signal {
                    at: r.get(0)?,
                    source: "step",
                    tool: r.get(1)?,
                    message: if result.trim().is_empty() { "failed with no output".into() } else { result },
                    session_key: r.get(3)?,
                    run_id: r.get(4)?,
                    command: field("input").as_deref().and_then(shell_command),
                    check_failed: status == "ok",
                    check_run: checkpoint
                        .as_ref()
                        .and_then(|c| c.get("checkScopes"))
                        .and_then(Value::as_array)
                        .is_some_and(|scopes| !scopes.is_empty()),
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    })
    .unwrap_or_default();
    out.extend(steps);
    // Refused or failed Hollow app actions (the current log and the rotated one).
    let mut denied = Vec::new();
    for name in [crate::app_broker::AUDIT_FILE, ".app-actions.1.jsonl"] {
        if let Ok(text) = std::fs::read_to_string(vault.join(name)) {
            out.extend(audit_signals(&text, since));
            denied.extend(denied_keys(&text, since));
        }
    }
    // The person's explicit Not useful votes.
    let votes: Vec<Signal> = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT updated_at, COALESCE(reason, ''), session_key, run_id FROM turn_feedback WHERE vote = 'notUseful' AND updated_at >= ?1")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![since], |r| {
                let reason: String = r.get(1)?;
                Ok(Signal {
                    at: r.get(0)?,
                    source: "feedback",
                    tool: "turn".into(),
                    message: if reason.trim().is_empty() { "not useful (no reason given)".into() } else { format!("not useful: {reason}") },
                    session_key: r.get(2)?,
                    run_id: r.get(3)?,
                    command: None,
                    check_failed: false,
                    check_run: false,
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    })
    .unwrap_or_default();
    out.extend(votes);
    routine_exits_dropped(without_noise(out, &denied))
}

/// The person's Deny on a card, as (run id, normalised message): not
/// friction itself ([`audit_signals`] leaves it out), and neither is the
/// failed tool step that carried it back to the agent.
pub fn denied_keys(text: &str, since: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v.get("card").and_then(Value::as_str) == Some("denied"))
        .filter(|v| {
            v.get("ts")
                .and_then(Value::as_str)
                .is_some_and(|t| t >= since)
        })
        .filter_map(|v| {
            Some((
                v.get("runId")?.as_str()?.to_string(),
                normalize(v.get("error")?.as_str()?),
            ))
        })
        .collect()
}

/// The shell tools steps record: Claude's and Codex's command steps are both
/// `Bash`; Grok's is `run_terminal_command`.
const SHELL_TOOLS: &[&str] = &["bash", "run_terminal_command"];

/// Whether two normalised messages are the same failure: equal, or one a
/// prefix of the other (the audit log keeps 200 characters of an error, a
/// step 300, so a long first line can be cut at different points).
fn same_message(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim_end_matches('…'), b.trim_end_matches('…'));
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

/// Signals that aren't a wall of Hollow's, and what's left of the rest
/// (overnight S3, from the friction list itself):
/// - an agent's own shell command that failed with nothing to say beyond its
///   exit code (`Exit code 1` and no output; Codex's `Exit code 1 (no
///   output)` since OF18, and its `failed with no output` from before, when
///   its results carried no exit code) is dropped. Such steps all normalised
///   to one `exit code <n>` pattern that couldn't point at anything and drowned the
///   real ones (17 of the week's 31 hits on 2026-10-10). A failure with
///   output is signed by its exit code and command head ([`signed_as`]);
///   when the command isn't known it loses the `Exit code N` line, so its
///   signature comes from what the command said;
/// - a failed Hollow app tool step that the action audit log already
///   records for the same run with the same message (one failure, counted
///   once, under the action's name), or that only carried back the
///   person's Deny (`denied`, run id and normalised message: a decision,
///   never friction).
pub fn without_noise(signals: Vec<Signal>, denied: &[(String, String)]) -> Vec<Signal> {
    let mut audited: Vec<(String, String)> = signals
        .iter()
        .filter(|s| s.source == "action")
        .filter_map(|s| s.run_id.clone().map(|r| (r, normalize(&s.message))))
        .collect();
    audited.extend(denied.iter().cloned());
    signals
        .into_iter()
        .filter_map(|mut s| {
            if s.source != "step" {
                return Some(s);
            }
            let tool = s.tool.to_ascii_lowercase();
            if SHELL_TOOLS.contains(&tool.as_str()) {
                // A Codex step recorded before OF18 kept no exit code.
                if s.message.trim() == "failed with no output" {
                    return None;
                }
                let mut lines = s.message.lines().skip_while(|l| l.trim().is_empty());
                let first = lines.next().unwrap_or("").trim();
                if bare_exit_line(first) {
                    let rest: Vec<&str> = lines.skip_while(|l| l.trim().is_empty()).collect();
                    if rest.is_empty() {
                        return None;
                    }
                    if shell_exit(&s).is_none() {
                        s.message = rest.join("\n");
                    }
                }
            }
            let app_tool = tool.starts_with("mcp__hollow_app__") || tool.starts_with("hollow_app");
            let message = normalize(&s.message);
            let echoed = app_tool
                && s.run_id.as_ref().is_some_and(|r| {
                    audited
                        .iter()
                        .any(|(run, m)| run == r && same_message(m, &message))
                });
            (!echoed).then_some(s)
        })
        .collect()
}

/// G3: a shell exit code alone (class `exit-code`, [`signed_as`]) is routine
/// noise unless a check failed (`check_run`: its command held a test run, a
/// typecheck or a lint, so the exit is that check failing) or the same
/// command and code recur in [`EXIT_MIN_CHATS`] or more chats on
/// [`EXIT_MIN_DAYS`] or more days. Every other class stays: a failed check
/// read from the output (`check-failed`), an error with its own words, a
/// refused or failed Hollow action, a Not useful vote. One hit that was a
/// check keeps every hit of its signature (the same command head and code),
/// even a `cargo build` hit signed alike. A dropped pattern loses all its hits; it comes back whole once it crosses the line (the
/// counts are recomputed from the steps on every refresh).
pub fn routine_exits_dropped(signals: Vec<Signal>) -> Vec<Signal> {
    struct Seen {
        check: bool,
        chats: BTreeSet<String>,
        days: BTreeSet<String>,
    }
    let key = |s: &Signal| -> Option<String> {
        let (class, message) = signed_as(s);
        (class == "exit-code").then(|| signature(s.tool.trim(), &class, &message))
    };
    let mut by: BTreeMap<String, Seen> = BTreeMap::new();
    for s in &signals {
        let Some(k) = key(s) else { continue };
        let seen = by.entry(k).or_insert_with(|| Seen {
            check: false,
            chats: BTreeSet::new(),
            days: BTreeSet::new(),
        });
        seen.check |= s.check_run;
        if let Some(chat) = &s.session_key {
            seen.chats.insert(chat.clone());
        }
        seen.days.insert(s.at.chars().take(10).collect());
    }
    signals
        .into_iter()
        .filter(|s| match key(s) {
            None => true,
            Some(k) => by.get(&k).is_some_and(|seen| {
                seen.check
                    || (seen.chats.len() >= EXIT_MIN_CHATS && seen.days.len() >= EXIT_MIN_DAYS)
            }),
        })
        .collect()
}

fn row_to_pattern(r: &rusqlite::Row<'_>) -> rusqlite::Result<Pattern> {
    let examples: String = r.get(9)?;
    let sources: String = r.get(10)?;
    Ok(Pattern {
        signature: r.get(0)?,
        tool: r.get(1)?,
        error_class: r.get(2)?,
        message: r.get(3)?,
        count: r.get(4)?,
        chats: r.get(5)?,
        week_count: r.get(6)?,
        first_seen: r.get(7)?,
        last_seen: r.get(8)?,
        examples: serde_json::from_str(&examples).unwrap_or_default(),
        sources: serde_json::from_str(&sources).unwrap_or_default(),
        status: r.get(11)?,
        drafted_session: r.get(12)?,
        drafted_at: r.get(13)?,
    })
}

const COLUMNS: &str = "signature, tool, error_class, message, count, chats, week_count, first_seen, last_seen, examples, sources, status, drafted_session, drafted_at";

/// Recompute every pattern of the last [`WINDOW_DAYS`] days into the table,
/// keeping each one's status; a pattern no longer seen drops to zero.
pub fn refresh(vault: &Path, now: DateTime<Utc>) -> Result<usize, String> {
    let since = (now - Duration::days(WINDOW_DAYS)).to_rfc3339();
    let week = (now - Duration::days(7)).to_rfc3339();
    let patterns = aggregate(&collect(vault, &since), &week);
    let at = now.to_rfc3339();
    crate::session_store::with_write(vault, "friction", |tx| {
        tx.execute(
            "UPDATE friction SET count = 0, chats = 0, week_count = 0, updated_at = ?1",
            params![at],
        )
        .map_err(|e| e.to_string())?;
        for p in &patterns {
            tx.execute(
                "INSERT INTO friction(signature, tool, error_class, message, count, chats, week_count, first_seen, last_seen, examples, sources, status, updated_at) \
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'open', ?12) \
                 ON CONFLICT(signature) DO UPDATE SET count = excluded.count, chats = excluded.chats, week_count = excluded.week_count, \
                 first_seen = MIN(friction.first_seen, excluded.first_seen), last_seen = excluded.last_seen, examples = excluded.examples, \
                 sources = excluded.sources, updated_at = excluded.updated_at",
                params![
                    p.signature,
                    p.tool,
                    p.error_class,
                    p.message,
                    p.count,
                    p.chats,
                    p.week_count,
                    p.first_seen,
                    p.last_seen,
                    serde_json::to_string(&p.examples).unwrap_or_default(),
                    serde_json::to_string(&p.sources).unwrap_or_default(),
                    at
                ],
            )
            .map_err(|e| format!("friction upsert: {e}"))?;
        }
        Ok(patterns.len())
    })
}

/// The stored patterns still seen, most frequent first (all of them:
/// [`list_view`] cuts to [`LIST_CAP`] after it orders by trend, so a new
/// one-hit failure isn't pushed out by frequent steady ones).
pub fn stored(vault: &Path) -> Result<Vec<Pattern>, String> {
    crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM friction WHERE count > 0 ORDER BY count DESC, last_seen DESC"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_pattern)
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    })
}

fn get(vault: &Path, sig: &str) -> Result<Option<Pattern>, String> {
    crate::session_store::with_read(vault, |conn| {
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM friction WHERE signature = ?1"),
            params![sig],
            row_to_pattern,
        )
        .optional()
        .map_err(|e| e.to_string())
    })
}

/// Drafts started in the 7 days before `now`.
pub fn drafts_this_week(vault: &Path, now: DateTime<Utc>) -> Result<usize, String> {
    let since = (now - Duration::days(7)).to_rfc3339();
    crate::session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM friction WHERE drafted_at IS NOT NULL AND drafted_at >= ?1",
            params![since],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n as usize)
        .map_err(|e| e.to_string())
    })
}

/// How a pattern is moving (G3), for the order of [`list_view`]: `new` (first
/// seen in the last 7 days), `growing` (the last 7 days hold more hits per
/// day than the days before them) or `steady`.
pub fn trend(p: &Pattern, now: DateTime<Utc>) -> &'static str {
    // Parsed, not string-compared: the sources write different precisions
    // and offsets (`Z` and `+00:00`, with and without fractions).
    let week_from = now - Duration::days(7);
    let new = match DateTime::parse_from_rfc3339(&p.first_seen) {
        Ok(t) => t >= week_from,
        Err(_) => p.first_seen >= week_from.to_rfc3339(),
    };
    if new {
        return "new";
    }
    let before = p.count - p.week_count;
    if p.week_count > 0 && p.week_count * (WINDOW_DAYS - 7) > before * 7 {
        "growing"
    } else {
        "steady"
    }
}

/// `friction.list`: refreshed patterns with new and growing ones first (each
/// carries its `trend`), the weekly headline and the cap.
pub fn list_view(vault: &Path, now: DateTime<Utc>) -> Result<Value, String> {
    refresh(vault, now)?;
    let rank = |t: &str| match t {
        "new" => 0,
        "growing" => 1,
        _ => 2,
    };
    let mut all: Vec<(Pattern, &'static str)> = stored(vault)?
        .into_iter()
        .map(|p| {
            let t = trend(&p, now);
            (p, t)
        })
        .collect();
    all.sort_by(|(a, ta), (b, tb)| {
        rank(ta)
            .cmp(&rank(tb))
            .then(b.week_count.cmp(&a.week_count))
            .then(b.count.cmp(&a.count))
            .then(b.last_seen.cmp(&a.last_seen))
            .then(a.signature.cmp(&b.signature))
    });
    all.truncate(LIST_CAP);
    let weekly: Vec<String> = all
        .iter()
        .map(|(p, _)| p)
        .filter(|p| p.status == "open" && p.week_count >= WEEKLY_MIN)
        .take(5)
        .map(headline)
        .collect();
    let patterns: Vec<Value> = all
        .iter()
        .map(|(p, t)| {
            let mut v = serde_json::to_value(p).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                o.insert("trend".into(), json!(t));
            }
            v
        })
        .collect();
    Ok(json!({
        "patterns": patterns,
        "weekly": weekly,
        "draftsThisWeek": drafts_this_week(vault, now)?,
        "weeklyDraftCap": WEEKLY_DRAFTS,
        "windowDays": WINDOW_DAYS,
        "note": "Counted from failed steps, failed or refused Hollow app actions (never the person's Deny) and explicit Not useful votes. A shell exit code alone counts only when a check failed or it recurs in 3 or more chats on 2 or more days. New and growing patterns come first (trend: new, growing, steady). Only the person starts a fix draft (on a card); nothing is pushed, opened as a pull request or installed.",
    }))
}

/// "Hollow hit the same wall 7 times: …".
pub fn headline(p: &Pattern) -> String {
    format!(
        "Hollow hit the same wall {} time{} this week: {} on {} ({} chat{})",
        p.week_count,
        if p.week_count == 1 { "" } else { "s" },
        p.error_class,
        p.tool,
        p.chats,
        if p.chats == 1 { "" } else { "s" }
    )
}

/// Set a pattern's status: Dismiss (or reopen). The page that called it was
/// cut (G2); the weekly review's Never answer on a fix card is its next
/// caller.
#[cfg_attr(not(test), allow(dead_code))] // the weekly review's Never answer
pub fn set_status(vault: &Path, sig: &str, status: &str) -> Result<(), String> {
    if !matches!(status, "open" | "dismissed") {
        return Err("A pattern is open or dismissed.".into());
    }
    // A drafted pattern stays drafted: reopening it would let a second
    // draft overwrite `drafted_at` and slip past the weekly cap.
    let n = crate::session_store::with_write(vault, "friction", |tx| {
        tx.execute(
            "UPDATE friction SET status = ?2 WHERE signature = ?1 AND status != 'drafted'",
            params![sig, status],
        )
        .map_err(|e| e.to_string())
    })?;
    if n == 0 {
        return Err(match get(vault, sig)? {
            Some(p) if p.status == "drafted" => format!(
                "Pattern {sig} already has a fix draft{}; it stays drafted.",
                p.drafted_session
                    .as_deref()
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default()
            ),
            _ => format!("No friction pattern {sig}."),
        });
    }
    Ok(())
}

/// Check a draft can start: the pattern exists and is open, and the week's
/// cap has room. Returns the pattern.
pub fn check_draft(vault: &Path, sig: &str, now: DateTime<Utc>) -> Result<Pattern, String> {
    let p =
        get(vault, sig)?.ok_or_else(|| format!("No friction pattern {sig}. Nothing started."))?;
    if p.status != "open" {
        return Err(format!(
            "Pattern {sig} is {}{}. Nothing started.",
            p.status,
            p.drafted_session
                .as_deref()
                .map(|s| format!(" (draft in {s})"))
                .unwrap_or_default()
        ));
    }
    let used = drafts_this_week(vault, now)?;
    if used >= WEEKLY_DRAFTS {
        return Err(format!(
            "{used} fix drafts started in the last 7 days; the cap is {WEEKLY_DRAFTS}. Nothing started."
        ));
    }
    Ok(p)
}

/// The draft session's branch for a pattern.
pub fn branch_for(sig: &str) -> String {
    format!("fix/friction-{}", &sig[..sig.len().min(8)])
}

/// How a friction fix's brief opens: a `main_agent::WAKE_PREFIX` message
/// that is the new conversation's task, not a check-in
/// (`main_agent::is_wake_message`).
pub(crate) const FIX_OPEN: &str = "⟲ Hollow: draft a fix for a friction pattern";

/// The task packet the draft session gets: the same for any agent.
pub fn packet(p: &Pattern) -> String {
    let examples: Vec<String> = p
        .examples
        .iter()
        .map(|e| {
            format!(
                "- {} ({}{}): {}",
                e.at,
                e.source,
                e.session_key
                    .as_deref()
                    .map(|k| format!(", session {k}"))
                    .unwrap_or_default(),
                e.excerpt
            )
        })
        .collect();
    format!(
        "{FIX_OPEN} the person chose (master plan #13). You are on a new local branch in a build workspace.\n\nPattern {sig}: {class} on `{tool}`, {count} times in {days} days across {chats} chats.\nNormalised message: {msg}\nExamples (newest first):\n{examples}\n\nHow to work:\n1. Find the cause. Read the examples' sessions with hollow_app read_session if you need more.\n2. Red first: add a regression test that fails on this branch as it is, and show it failing.\n3. Make the smallest fix; show the test passing.\n4. Run the gate one step at a time (never two at once; this machine allows two toolchain slots in all, and another may be in use): cargo fmt, cargo clippy -D warnings, TMPDIR=/tmp cargo test --lib, tsc --noEmit, vitest (VITEST_MIN_FORKS=1 VITEST_MIN_THREADS=1), node --test scripts/*.test.mjs.\n5. Update CLAUDE.md and docs/BUILD_LOG.md as the repo requires, and commit locally.\n6. End with \"Ready to install?\", the diffstat and the gate results.\n\nNever push, open a pull request, run the test bed, build a candidate or install: each of those is the person's own decision, made separately.",
        sig = p.signature,
        class = p.error_class,
        tool = p.tool,
        count = p.count,
        days = WINDOW_DAYS,
        chats = p.chats,
        msg = p.message,
        examples = if examples.is_empty() { "(none kept)".into() } else { examples.join("\n") },
    )
}

/// The draft's mode: `ceiling` (the asking agent's mode) or `auto` for the
/// person, never above either.
pub fn draft_mode(ceiling: Option<&str>) -> String {
    let base = "auto";
    match ceiling {
        Some(c) if crate::app_mcp::perm_rank(c) < crate::app_mcp::perm_rank(base) => c.to_string(),
        _ => base.to_string(),
    }
}

/// Record that a draft started.
pub fn mark_drafted(
    vault: &Path,
    sig: &str,
    session: &str,
    now: DateTime<Utc>,
) -> Result<(), String> {
    crate::session_store::with_write(vault, "friction", |tx| {
        tx.execute(
            "UPDATE friction SET status = 'drafted', drafted_session = ?2, drafted_at = ?3 WHERE signature = ?1",
            params![sig, session, now.to_rfc3339()],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftInput {
    pub signature: String,
    /// A repository path from `workspaces.build_repo_candidates`.
    pub repo: String,
    #[serde(default)]
    pub base: Option<String>,
}

/// Start a fix draft: a build workspace on a new local branch, then one
/// chat there with the packet. `ceiling`: the asking agent's mode (`None`
/// for the person). Only reached through a confirmed card or the person's
/// own command.
pub(crate) async fn draft_fix(
    app: Option<&tauri::AppHandle>,
    state: &crate::commands::AppState,
    input: DraftInput,
    ceiling: Option<&str>,
) -> Result<Value, String> {
    // One draft at a time, so two can't both pass the weekly cap.
    static DRAFTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = DRAFTING.lock().await;
    let vault = state.config.vault_path.clone();
    let now = Utc::now();
    let p = {
        let vault = vault.clone();
        let sig = input.signature.trim().to_string();
        tauri::async_runtime::spawn_blocking(move || {
            refresh(&vault, now)?;
            check_draft(&vault, &sig, now)
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let create = crate::build_workspaces::CreateInput {
        title: format!("Fix friction {}", &p.signature[..8]),
        goal: format!(
            "Draft a fix for friction pattern {} ({} on {}).",
            p.signature, p.error_class, p.tool
        ),
        repo: input.repo.clone(),
        branch: Some(branch_for(&p.signature)),
        base: input.base.clone(),
    };
    let created =
        crate::commands::workspaces_api::create_build_workspace_with(create, state).await?;
    let target = crate::app_sessions::resolve_workspace(&vault, &created.workspace.id)?;
    let mode = draft_mode(ceiling);
    let chat = crate::commands::ChatSendInput {
        prompt: packet(&p),
        session_id: None,
        session_key: None,
        cwd: Some(target.cwd.clone()),
        model: None,
        permission_mode: Some(mode.clone()),
        agent: None,
        client_turn_id: None,
        app_depth: 0,
        ultracode: false,
        canvases: None,
        continued_from: Vec::new(),
        unattended: false,
        typed: false,
        host_asker: None,
        canned: None,
        worker: false,
    };
    let accepted = crate::commands::send_chat(app, &state.config, chat).await?;
    mark_drafted(&vault, &p.signature, &accepted.session_key, now)?;
    Ok(json!({
        "workspace": created.workspace.id,
        "branch": branch_for(&p.signature),
        "sessionKey": accepted.session_key,
        "mode": mode,
        "warnings": created.warnings,
        "note": "A local branch and one draft chat. Nothing is pushed, opened as a pull request, bed-tested or installed without the person.",
    }))
}

/// The nightly window job: recount (it never starts anything).
pub(crate) fn nightly_job(
    _app: tauri::AppHandle,
    config: crate::config::Config,
    _key: String,
    _late: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
    Box::pin(async move {
        tauri::async_runtime::spawn_blocking(move || {
            refresh(&config.vault_path, Utc::now()).map(|_| ())
        })
        .await
        .map_err(|e| e.to_string())?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(at: &str, tool: &str, message: &str, key: &str) -> Signal {
        Signal {
            at: at.into(),
            source: "step",
            tool: tool.into(),
            message: message.into(),
            session_key: Some(key.into()),
            run_id: None,
            command: None,
            check_failed: false,
            check_run: false,
        }
    }

    #[test]
    fn bare_shell_exits_and_audited_app_tool_steps_are_not_counted() {
        let mut action = sig(
            "2026-10-10T03:26:16Z",
            "sessions.start",
            "This session was itself started by an agent 2 levels deep; nothing started.",
            "s-a1b2c3d4",
        );
        action.source = "action";
        action.run_id = Some("run-1".into());
        let mut step = sig(
            "2026-10-10T03:26:16Z",
            "mcp__hollow_app__run_action",
            "This session was itself started by an agent 2 levels deep; nothing started.",
            "s-a1b2c3d4",
        );
        step.run_id = Some("run-1".into());
        let mut other_run = step.clone();
        other_run.run_id = Some("run-2".into());
        let mut unaudited = sig(
            "2026-10-10T03:49:11Z",
            "mcp__hollow_app__describe_action",
            "unknown action `sessions.start_session`",
            "s-a1b2c3d4",
        );
        unaudited.run_id = Some("run-1".into());
        let mut deny = sig(
            "2026-10-10T03:50:00Z",
            "mcp__hollow_app__run_action",
            "The user declined gardener.apply; nothing changed.",
            "s-a1b2c3d4",
        );
        deny.run_id = Some("run-3".into());
        let denied = vec![(
            "run-3".to_string(),
            normalize("The user declined gardener.apply; nothing changed."),
        )];
        let kept = without_noise(
            vec![
                sig("2026-10-10T04:07:09Z", "Bash", "Exit code 1", "s-a1b2c3d4"),
                sig("2026-10-10T04:07:10Z", "Bash", "failed with no output", "s-a1b2c3d4"),
                // Review: a failure with output keeps it, without the exit line.
                sig("2026-10-10T04:07:11Z", "Bash", "Exit code 101\n\nerror: CARGO_TARGET_DIR is \"~/x\": a \"~\" there is a literal folder", "s-a1b2c3d4"),
                sig("2026-10-10T04:07:12Z", "Bash", "PreToolUse:Bash hook denied: toolchain guard", "s-a1b2c3d4"),
                sig("2026-10-10T04:07:13Z", "run_terminal_command", "Exit code 2", "s-a1b2c3d4"),
                action.clone(),
                step,
                other_run.clone(),
                unaudited.clone(),
                deny,
            ],
            &denied,
        );
        let tools: Vec<&str> = kept.iter().map(|s| s.tool.as_str()).collect();
        assert_eq!(
            tools,
            [
                "Bash",
                "Bash",
                "sessions.start",
                "mcp__hollow_app__run_action",
                "mcp__hollow_app__describe_action"
            ],
            "{kept:?}"
        );
        assert!(
            kept[0].message.starts_with("error: CARGO_TARGET_DIR"),
            "{:?}",
            kept[0].message
        );
        assert_eq!(
            normalize(&kept[0].message),
            normalize("error: CARGO_TARGET_DIR is \"~/x\": a \"~\" there is a literal folder")
        );
        assert!(kept[1].message.contains("hook denied"));
        assert_eq!(
            kept[3].run_id.as_deref(),
            Some("run-2"),
            "a step another run logged no action for stays"
        );
    }

    #[test]
    fn audit_denials_are_keys_and_long_messages_still_match() {
        let log = concat!(
            r#"{"ts":"2026-10-10T03:50:00Z","action":"gardener.apply","ok":false,"card":"denied","runId":"run-3","error":"The user declined gardener.apply; nothing changed."}"#,
            "\n",
            r#"{"ts":"2026-10-01T03:50:00Z","action":"x.y","ok":false,"card":"denied","runId":"run-old","error":"old"}"#,
        );
        assert_eq!(
            denied_keys(log, "2026-10-05T00:00:00Z"),
            vec![(
                "run-3".to_string(),
                normalize("The user declined gardener.apply; nothing changed.")
            )]
        );
        assert!(same_message(
            "a long error cut here…",
            "a long error cut here and then more"
        ));
        assert!(!same_message("", "anything"));
        assert!(!same_message("one error", "another error"));
    }

    #[test]
    fn the_same_wall_groups_and_different_walls_never_merge() {
        let signals = vec![
            sig(
                "2026-10-05T10:00:00Z",
                "tasks.update_task",
                "TASK_INVALID: priority 'urgent' is not one of low, medium, high",
                "s-a1b2c3d4",
            ),
            sig(
                "2026-10-06T10:00:00Z",
                "tasks.update_task",
                "TASK_INVALID: priority 'urgent' is not one of low, medium, high",
                "s-e5f6a7b8",
            ),
            sig(
                "2026-10-07T10:00:00Z",
                "tasks.update_task",
                "TASK_INVALID: priority 'urgent' is not one of low, medium, high",
                "s-e5f6a7b8",
            ),
            // Same tool and class, another value: another wall.
            sig(
                "2026-10-07T11:00:00Z",
                "tasks.update_task",
                "TASK_INVALID: priority 'p0' is not one of low, medium, high",
                "s-a1b2c3d4",
            ),
            // Same message, another tool: another wall.
            sig(
                "2026-10-07T12:00:00Z",
                "tasks.create_task",
                "TASK_INVALID: priority 'urgent' is not one of low, medium, high",
                "s-a1b2c3d4",
            ),
            // Paths, ids and numbers vary between occurrences of one wall.
            sig(
                "2026-10-07T13:00:00Z",
                "Bash",
                "cat: /Users/x/vault/a.md: No such file or directory",
                "s-1",
            ),
            sig(
                "2026-10-08T13:00:00Z",
                "Bash",
                "cat: /tmp/b-123.md: No such file or directory",
                "s-2",
            ),
        ];
        let p = aggregate(&signals, "2026-10-06T00:00:00Z");
        assert_eq!(p.len(), 4, "{p:#?}");
        assert_eq!(p[0].count, 3);
        assert_eq!(p[0].chats, 2);
        assert_eq!(p[0].week_count, 2);
        assert_eq!(p[0].error_class, "task_invalid");
        assert_eq!(p[0].first_seen, "2026-10-05T10:00:00Z");
        assert_eq!(p[0].examples[0].at, "2026-10-07T10:00:00Z", "newest first");
        let bash = p.iter().find(|x| x.tool == "Bash").unwrap();
        assert_eq!(bash.count, 2);
        assert_eq!(bash.error_class, "not-found");
        assert_eq!(bash.message, "cat: <path>: no such file or directory");
        assert_eq!(
            headline(&p[0]),
            "Hollow hit the same wall 2 times this week: task_invalid on tasks.update_task (2 chats)"
        );
    }

    #[test]
    fn normalising_redacts_secrets_and_keeps_quoted_values() {
        let n = normalize(
            "Auth failed for sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcd at 12:30",
        );
        assert!(!n.contains("ABCDEFGHIJ"), "{n}");
        assert!(normalize("bad value 'urgent' in run-20261009-abcdef").contains("'urgent'"));
        assert!(normalize("bad value 'urgent' in run-20261009-abcdef").contains("<id>"));
        assert_eq!(normalize("  \n second line only"), "second line only");
        assert_eq!(error_class("HTTP 503 from upstream"), "network");
        assert_eq!(
            error_class("This is an unattended Main agent turn ... nothing changed (refused)"),
            "permission"
        );
        assert_eq!(error_class("something odd"), "error");
    }

    /// HF02 (red first): Claude Code's approval refusals were `error`.
    #[test]
    fn approval_refusals_are_permission() {
        for message in [
            "This command requires approval",
            "Compound commands require approval",
            "rm -rf requires explicit approval",
            "ls of the vault folder was blocked. For security, Claude Code may only list files in the allowed working directories",
            "Command contains simple_expansion",
            "This command changes directory before running a version-control command",
            "You haven't granted Bash yet",
        ] {
            assert_eq!(error_class(message), "permission", "{message}");
        }
        assert_eq!(error_class("the main thread was blocked on IO"), "error");
    }

    /// HF02 (red first): a shell failure's first output line was a runner's
    /// banner, a NO_COLOR warning or a heading as often as the error, and
    /// each made its own pattern; one command head's exits are one.
    #[test]
    fn shell_exits_are_signed_by_exit_code_and_command_head() {
        let shell = |at: &str, command: &str, message: &str| Signal {
            command: Some(command.into()),
            ..sig(at, "Bash", message, "s-1")
        };
        let read = sig("2026-10-15T14:00:00Z", "Read", "File not found", "s-1");
        let kept = without_noise(
            vec![
                shell(
                    "2026-10-15T10:00:00Z",
                    "npx vitest run a",
                    "Exit code 1\n\n RUN  v3.2.4 /Users/x/app\n\n FAIL  a.test.ts",
                ),
                shell(
                    "2026-10-15T11:00:00Z",
                    "npx vitest run b",
                    "Exit code 1\nWarning: The 'NO_COLOR' env is ignored due to the 'FORCE_COLOR' env being set.",
                ),
                shell(
                    "2026-10-15T12:00:00Z",
                    "cd app && CI=1 npx vitest run c",
                    "Exit code 1\n# Results",
                ),
                // The same first output line as the banner, another command.
                shell(
                    "2026-10-15T13:00:00Z",
                    "cargo test --lib",
                    "Exit code 1\n RUN  v3.2.4 /Users/x/app",
                ),
                read.clone(),
            ],
            &[],
        );
        assert_eq!(kept.len(), 5, "{kept:?}");
        assert!(
            kept[0].message.starts_with("Exit code 1"),
            "the example keeps the exit line"
        );
        let p = aggregate(&kept, "2026-10-10T00:00:00Z");
        assert_eq!(p.len(), 3, "{p:#?}");
        let vitest = &p[0];
        assert_eq!(vitest.count, 3);
        assert_eq!(vitest.error_class, "exit-code");
        assert_eq!(vitest.message, "exit code 1 from npx vitest");
        let cargo = p
            .iter()
            .find(|x| x.message == "exit code 1 from cargo test")
            .unwrap();
        assert_ne!(cargo.signature, vitest.signature);
        // Any other tool signs as before, even with an exit line.
        let read_p = p.iter().find(|x| x.tool == "Read").unwrap();
        assert_eq!(read_p.error_class, error_class(&read.message));
        assert_eq!(read_p.message, normalize(&read.message));
        let other = Signal {
            command: Some("npx vitest run".into()),
            ..sig("2026-10-15T15:00:00Z", "mcp__x", "Exit code 1\nx", "s-1")
        };
        assert_eq!(
            signed_as(&other),
            (error_class(&other.message), normalize(&other.message))
        );
    }

    /// OF18 (red first): a Codex failure's result was its output alone, so
    /// each runner banner made its own pattern; with its exit code it signs
    /// by code and head like Claude's, and a quiet one is noise like a bare
    /// `Exit code N`. Steps recorded before (`failed with no output`) still
    /// drop.
    #[test]
    fn codex_shell_failures_sign_by_exit_code_and_head() {
        let codex = |at: &str, command: &str, output: &str, exit: i64| {
            let (message, is_error) =
                crate::agent::codex_command_result(command, output, Some(exit));
            assert!(is_error, "{command}");
            Signal {
                command: Some(command.into()),
                ..sig(at, "Bash", &message, "s-1")
            }
        };
        let kept = without_noise(
            vec![
                codex(
                    "2026-10-17T01:00:00Z",
                    "/bin/zsh -lc 'npx vitest run a'",
                    "\n RUN  v3.2.4 /Users/x/app\n\n FAIL  a.test.ts\n",
                    1,
                ),
                codex(
                    "2026-10-17T02:00:00Z",
                    "bash -lc 'cd app && npx vitest run b'",
                    "Warning: The 'NO_COLOR' env is ignored due to the 'FORCE_COLOR' env being set.\n",
                    1,
                ),
                codex(
                    "2026-10-17T03:00:00Z",
                    "/bin/zsh -lc 'npm run build > /tmp/b.log 2>&1'",
                    "",
                    2,
                ),
                sig("2026-10-10T04:07:10Z", "Bash", "failed with no output", "s-1"),
            ],
            &[],
        );
        assert_eq!(kept.len(), 2, "{kept:?}");
        let p = aggregate(&kept, "2026-10-10T00:00:00Z");
        assert_eq!(p.len(), 1, "{p:#?}");
        assert_eq!(p[0].count, 2);
        assert_eq!(p[0].error_class, "exit-code");
        assert_eq!(p[0].message, "exit code 1 from npx vitest");
        assert_eq!(exit_code("Exit code 2 (no output)").as_deref(), Some("2"));
        assert_eq!(exit_code("Exit code 2 of 3"), None);
    }

    /// OF18 review (red first): `Exit code -1` (a killed or unstarted
    /// process) failed the digits check, so it didn't sign by code and head,
    /// and a quiet one wasn't dropped as a bare exit line. Any
    /// parenthesised note is accepted after the code; only `(no output)`
    /// makes the line bare.
    #[test]
    fn negative_exit_codes_sign_and_drop_like_others() {
        assert_eq!(exit_code("Exit code -1").as_deref(), Some("-1"));
        assert_eq!(exit_code("Exit code -1 (no output)").as_deref(), Some("-1"));
        assert_eq!(exit_code("Exit code -"), None);
        assert_eq!(exit_code("Exit code 1-2"), None);
        assert!(bare_exit_line("Exit code -1"));
        assert!(bare_exit_line("Exit code -1 (no output)"));
        assert!(bare_exit_line("Exit code 1"));
        assert!(!bare_exit_line("Exit code 1 (no match)"));
        assert!(!bare_exit_line("Exit code 1 of 2"));

        let quiet = Signal {
            command: Some("/bin/zsh -lc 'npm run build'".into()),
            ..sig(
                "2026-10-17T01:00:00Z",
                "Bash",
                "Exit code -1 (no output)",
                "s-1",
            )
        };
        let loud = Signal {
            command: Some("/bin/zsh -lc 'npm run build'".into()),
            ..sig(
                "2026-10-17T02:00:00Z",
                "Bash",
                "Exit code -1\nKilled",
                "s-1",
            )
        };
        let kept = without_noise(vec![quiet, loud], &[]);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            signed_as(&kept[0]),
            (
                "exit-code".to_string(),
                "exit code -1 from npm run".to_string()
            )
        );
    }

    /// OF16: a shell step that exited cleanly but whose output shows a
    /// check's failure (a test run piped into `tail`) is friction, signed by
    /// its command's head; a passing one isn't. The step spine decides it
    /// from the full output and keeps only the verdict.
    #[test]
    fn a_clean_exit_that_failed_a_check_by_its_output_counts() {
        use crate::step_spine::{steps_for_turn, tool_finished, tool_started};
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-of16-friction";
        let ord =
            crate::session_store::start_turn(v, key, "claude", "/vault", "run-1", "Test").unwrap();
        let piped = json!({ "command": "cd /r && npx vitest run 2>&1 | tail -20" });
        // The summary comes after the checkpoint's 300-character cut.
        let progress = "✓ a\n".repeat(200);
        let red = format!("{progress} Test Files  1 failed (1)\n Tests  2 failed");
        let green = " Tests  10 passed (10)".to_string();
        for (id, output) in [("t1", red), ("t2", green)] {
            tool_started(v, key, ord, "run-1", id, "Bash", &piped).unwrap();
            tool_finished(v, v, key, ord, "run-1", id, &output, false, None).unwrap();
        }
        let steps = steps_for_turn(v, key, ord, true).unwrap();
        let shell: Vec<&Value> = steps
            .iter()
            .filter_map(|s| s.checkpoint.as_ref())
            .filter(|c| c["tool"] == "Bash")
            .collect();
        assert_eq!(shell.len(), 2);
        assert_eq!(shell[0]["checkScopes"], json!(["vitest"]));
        assert_eq!(shell[0]["checkFailed"], true);
        assert!(shell[1].get("checkFailed").is_none());

        let signals = collect(v, "2000-01-01T00:00:00Z");
        assert_eq!(signals.len(), 1, "{signals:?}");
        assert!(signals[0].check_failed);
        let signed = signed_as(&signals[0]);
        assert_eq!(signed.0, "check-failed");
        assert_eq!(signed.1, "failed check from npx vitest");
    }

    #[test]
    fn the_command_head_skips_assignments_and_a_leading_cd() {
        let head = |c: &str| command_head(c).unwrap_or_default();
        assert_eq!(head("npx vitest run a"), "npx vitest");
        assert_eq!(
            head("cd src-tauri && CARGO_TARGET_DIR=/tmp/t cargo test --lib"),
            "cargo test"
        );
        assert_eq!(head("FOO=1 grep -rn x src"), "grep");
        assert_eq!(head("npx -y foo"), "npx");
        // Review: wrappers are skipped, a script's path signs by its name, a
        // URL is never a subcommand.
        assert_eq!(head("timeout 60 npm test"), "npm test");
        assert_eq!(head("env X=1 cargo build"), "cargo build");
        assert_eq!(head("/Users/someone/projects/x/run.sh --fast"), "run.sh");
        assert_eq!(head("npx https://example.com/pkg"), "npx");
        // Review follow-ups: `cd dir; cmd`, `cd dir ; cmd` and `git -C dir`.
        assert_eq!(head("cd app; npm test"), "npm test");
        assert_eq!(head("cd app ; cargo build"), "cargo build");
        assert_eq!(head("git -C ~/repo push origin main"), "git push");
        // OF13: a `<shell> -c`/`-lc` wrapper signs by the script it runs.
        assert_eq!(head("/bin/zsh -lc 'git push'"), "git push");
        assert_eq!(head("bash -c \"cd app && npm test\""), "npm test");
        assert_eq!(head("sh -c 'FOO=1 cargo build'"), "cargo build");
        assert_eq!(head("bash script.sh"), "bash");
        // Review: a leading `set …;`, `source …;` or `. …` is skipped too.
        assert_eq!(head("bash -c 'set -euo pipefail; npm test'"), "npm test");
        assert_eq!(
            head("zsh -lc 'source ~/.zshrc; cargo build'"),
            "cargo build"
        );
        assert_eq!(head(". ./env.sh && npx vitest run"), "npx vitest");
        assert_eq!(head("set -x"), "set");
        assert_eq!(command_head("  "), None);
        let cmd = |input: &str| shell_command(input).unwrap_or_default();
        assert_eq!(
            cmd(r#"{"command":"npx vitest run a","description":"Run tests"}"#),
            "npx vitest run a"
        );
        // The checkpoint's input is cut at 300 characters: read what is there.
        assert_eq!(
            cmd("{\"command\":\"cd app && npx vitest run \\\"a b\\\" --bail…"),
            "cd app && npx vitest run \"a b\" --bail…"
        );
        assert_eq!(shell_command("npm test"), None);
    }

    #[test]
    fn audit_lines_count_failures_and_timeouts_but_never_the_persons_deny() {
        let log = [
            json!({"ts": "2026-10-08T10:00:00Z", "action": "build.start_app_build", "ok": false, "card": "timed_out", "scope": "s-1", "runId": "r1"}),
            json!({"ts": "2026-10-08T10:01:00Z", "action": "tasks.delete_task", "ok": false, "card": "denied", "error": "denied by the user"}),
            json!({"ts": "2026-10-08T10:02:00Z", "action": "tasks.update_task", "ok": false, "error": "TASK_INVALID: priority"}),
            json!({"ts": "2026-10-08T10:03:00Z", "action": "tasks.update_task", "ok": true}),
            json!({"ts": "2026-09-01T10:00:00Z", "action": "old.thing", "ok": false, "error": "x"}),
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let s = audit_signals(&format!("{log}\nnot json"), "2026-10-01T00:00:00Z");
        assert_eq!(s.len(), 2, "{s:#?}");
        assert_eq!(s[0].message, "card timed_out");
        assert_eq!(s[0].session_key.as_deref(), Some("s-1"));
        assert_eq!(s[1].tool, "tasks.update_task");
    }

    #[test]
    fn the_packet_says_red_first_the_gate_and_never_push() {
        let p = &aggregate(
            &[sig(
                "2026-10-08T10:00:00Z",
                "tasks.update_task",
                "TASK_INVALID: priority 'urgent'",
                "s-9",
            )],
            "2026-10-01T00:00:00Z",
        )[0];
        let text = packet(p);
        assert!(text.contains(&p.signature));
        assert!(text.contains("Red first"));
        assert!(text.contains("cargo clippy -D warnings"));
        assert!(text.contains("one step at a time"));
        assert!(text.contains("Never push, open a pull request"));
        assert!(text.contains("s-9"));
        assert_eq!(
            branch_for(&p.signature),
            format!("fix/friction-{}", &p.signature[..8])
        );
        assert_eq!(draft_mode(None), "auto");
        assert_eq!(draft_mode(Some("acceptEdits")), "acceptEdits");
        assert_eq!(draft_mode(Some("bypassPermissions")), "auto");
    }

    #[test]
    fn refresh_keeps_status_and_the_weekly_cap_holds() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-10-09T06:00:00Z")
            .unwrap()
            .into();
        let line = |at: &str| {
            json!({"ts": at, "action": "tasks.update_task", "ok": false, "error": "TASK_INVALID: priority 'urgent'", "scope": "s-1"}).to_string()
        };
        std::fs::write(
            v.join(crate::app_broker::AUDIT_FILE),
            [line("2026-10-08T10:00:00Z"), line("2026-10-08T11:00:00Z")].join("\n"),
        )
        .unwrap();
        assert_eq!(refresh(v, now).unwrap(), 1);
        let p = stored(v).unwrap().remove(0);
        assert_eq!(p.count, 2);
        set_status(v, &p.signature, "dismissed").unwrap();
        refresh(v, now).unwrap();
        assert_eq!(
            stored(v).unwrap()[0].status,
            "dismissed",
            "a refresh keeps the status"
        );
        assert!(check_draft(v, &p.signature, now)
            .unwrap_err()
            .contains("dismissed"));
        set_status(v, &p.signature, "open").unwrap();
        assert!(check_draft(v, &p.signature, now).is_ok());
        assert!(check_draft(v, "nope", now).is_err());
        assert!(
            set_status(v, &p.signature, "drafted").is_err(),
            "only the draft path marks drafted"
        );
        // The week's cap: three drafts, then refused.
        for i in 0..WEEKLY_DRAFTS {
            crate::session_store::with_write(v, "t", |tx| {
                tx.execute(
                    "INSERT INTO friction(signature, tool, error_class, message, count, chats, first_seen, last_seen, examples, sources, status, drafted_session, drafted_at, updated_at) VALUES(?1, 't', 'c', 'm', 1, 1, ?2, ?2, '[]', '[]', 'drafted', 's-x', ?2, ?2)",
                    params![format!("d{i}"), "2026-10-08T00:00:00Z"],
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            })
            .unwrap();
        }
        assert!(check_draft(v, &p.signature, now)
            .unwrap_err()
            .contains("cap is 3"));
        // A week later the cap has room again.
        assert!(check_draft(v, &p.signature, now + Duration::days(8)).is_ok());
        // The draft path marks it drafted.
        mark_drafted(v, &p.signature, "s-draft", now).unwrap();
        // Review finding: a drafted pattern can't be reopened and drafted again.
        assert!(set_status(v, &p.signature, "open")
            .unwrap_err()
            .contains("stays drafted"));
        assert!(check_draft(v, &p.signature, now + Duration::days(8))
            .unwrap_err()
            .contains("s-draft"));
    }

    /// G3 (red first): a bare shell exit code is noise. It counts only when
    /// its command was a check, or it recurs in 3 or more chats on 2 or more
    /// days; every other class of failure is never filtered.
    #[test]
    fn a_shell_exit_code_counts_only_when_a_check_failed_or_it_recurs() {
        let exit = |at: &str, command: &str, chat: &str, check: bool| Signal {
            command: Some(command.into()),
            check_run: check,
            ..sig(at, "Bash", "Exit code 1\n\nsome output", chat)
        };
        let mut signals = vec![
            // One chat, one day: dropped.
            exit("2026-10-10T10:00:00Z", "grep foo x", "s-1", false),
            // Three chats on one day: dropped (days < 2).
            exit("2026-10-10T10:00:00Z", "git diff", "s-1", false),
            exit("2026-10-10T11:00:00Z", "git diff", "s-2", false),
            exit("2026-10-10T12:00:00Z", "git diff", "s-3", false),
            // Two chats on two days: dropped (chats < 3).
            exit("2026-10-10T10:00:00Z", "ls nope", "s-1", false),
            exit("2026-10-11T10:00:00Z", "ls nope", "s-2", false),
            // Three chats on two days: kept, every hit.
            exit("2026-10-10T10:00:00Z", "rg needle", "s-1", false),
            exit("2026-10-10T11:00:00Z", "rg needle", "s-2", false),
            exit("2026-10-11T10:00:00Z", "rg needle", "s-3", false),
            // A check that failed: kept at its first hit.
            exit("2026-10-10T10:00:00Z", "npx vitest run", "s-1", true),
        ];
        // Never filtered: a failed check read from the output, an error with
        // its own words, and a person's vote.
        let mut masked = exit("2026-10-10T10:00:00Z", "cargo test", "s-1", true);
        masked.check_failed = true;
        let mut vote = sig("2026-10-10T10:00:00Z", "turn", "not useful: slow", "s-1");
        vote.source = "feedback";
        signals.extend([
            masked,
            sig("2026-10-10T10:00:00Z", "Read", "File not found", "s-1"),
            vote,
        ]);
        let kept = routine_exits_dropped(signals);
        let heads: Vec<String> = kept.iter().map(|s| signed_as(s).1).collect();
        assert_eq!(
            heads,
            [
                "exit code 1 from rg",
                "exit code 1 from rg",
                "exit code 1 from rg",
                "exit code 1 from npx vitest",
                "failed check from cargo test",
                "file not found",
                "not useful: slow",
            ],
            "{kept:?}"
        );
    }

    /// G3 through the step spine: a shell step's `checkScopes` marks its
    /// command as a check, so its exit survives where `grep`'s is dropped.
    #[test]
    fn collect_reads_whether_the_command_was_a_check() {
        use crate::step_spine::{tool_finished, tool_started};
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-g3-friction";
        let ord =
            crate::session_store::start_turn(v, key, "claude", "/vault", "run-1", "Test").unwrap();
        for (id, command) in [("t1", "npx vitest run"), ("t2", "grep needle haystack")] {
            tool_started(
                v,
                key,
                ord,
                "run-1",
                id,
                "Bash",
                &json!({ "command": command }),
            )
            .unwrap();
            tool_finished(
                v,
                v,
                key,
                ord,
                "run-1",
                id,
                "Exit code 1\n\nsome output",
                true,
                None,
            )
            .unwrap();
        }
        let signals = collect(v, "2000-01-01T00:00:00Z");
        assert_eq!(signals.len(), 1, "{signals:?}");
        assert!(signals[0].check_run);
        assert_eq!(signed_as(&signals[0]).1, "exit code 1 from npx vitest");
    }

    /// G3: `friction.list` puts new patterns, then growing ones, before the
    /// steady ones, however large the steady counts are; each pattern says
    /// which it is.
    #[test]
    fn list_view_puts_new_and_growing_patterns_first() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-10-20T06:00:00Z")
            .unwrap()
            .into();
        let mut lines = Vec::new();
        let mut hit = |action: &str, at: &str| {
            lines.push(json!({"ts": at, "action": action, "ok": false, "error": "TASK_INVALID: priority 'urgent'", "scope": "s-1"}).to_string());
        };
        // Steady and large: 20 hits before this week, none since.
        for d in 1..=20 {
            hit("tasks.big", &format!("2026-10-{:02}T10:00:00Z", d.min(12)));
        }
        // Steady: 6 hits before, 1 this week (a flat rate).
        for d in 1..=6 {
            hit("tasks.flat", &format!("2026-10-{d:02}T10:00:00Z"));
        }
        hit("tasks.flat", "2026-10-15T10:00:00Z");
        // Growing: 1 hit before, 4 this week.
        hit("tasks.rising", "2026-10-02T10:00:00Z");
        for d in 14..=17 {
            hit("tasks.rising", &format!("2026-10-{d}T10:00:00Z"));
        }
        // New: first seen this week.
        hit("tasks.fresh", "2026-10-16T10:00:00Z");
        hit("tasks.fresh", "2026-10-17T10:00:00Z");
        std::fs::write(v.join(crate::app_broker::AUDIT_FILE), lines.join("\n")).unwrap();
        let view = list_view(v, now).unwrap();
        let order: Vec<(String, String)> = view["patterns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["tool"].as_str().unwrap().to_string(),
                    p["trend"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let want = [
            ("tasks.fresh", "new"),
            ("tasks.rising", "growing"),
            ("tasks.flat", "steady"),
            ("tasks.big", "steady"),
        ];
        assert_eq!(
            order,
            want.map(|(a, b)| (a.to_string(), b.to_string())),
            "{view}"
        );
    }
}
