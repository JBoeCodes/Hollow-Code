//! What a run did beyond the vault and its repositories, and how a retry
//! avoids doing it twice (plan completion L2, 2026-10-13; assignment item 3).
//!
//! The evidence receipt already proves vault changes (History's diff),
//! repository changes, checks, pull requests and started services. This
//! module adds the rest of the picture from the run's own tool calls, which
//! [`crate::evidence::EvidenceCollector`] observes for every surface:
//!
//! - **Recorded outside effects** ([`OutsideEffect`]): calls Hollow can name
//!   as acting outside the run's files: built-in connector writes (none
//!   write since Hollow's Notion connector was removed on 2026-11-10),
//!   Hollow app and task changes, `git push`, `gh pr|issue|release …`,
//!   mutating `curl`, publishes, and write-like tools of other MCP servers.
//!   Each is `done`, `failed` or `uncertain` (no reply, a lost reply, a
//!   partial write). A call Hollow itself executes carries a `key`: the
//!   same call in a guarded retry is refused ([`refusal`]).
//! - **Unknown coverage** ([`UnknownCalls`]): calls whose effects Hollow
//!   can't see: shell commands it can't classify as read-only or a check,
//!   other MCP tools it can't classify, and helper agents (their own tool
//!   calls are not in the parent's stream). Never read as "nothing happened".
//!
//! Classification is deliberately conservative: anything it can't place is
//! unknown, never read-only. A wording change in a tool means more unknowns,
//! never fewer.
//!
//! **Guarded retry.** "Run again without repeating" (the run review's retry,
//! `workflows::execute` with `RunContext.guarded_retry_of`) writes a
//! [`RetryGuard`] for the new run with the keys of the failed run's `done` and
//! `uncertain` keyed effects. While it exists, the Hollow app broker
//! (`app_broker::execute`) and the task broker (`task_broker::dispatch_at`)
//! refuse an identical call (same server, tool and arguments) with
//! [`refusal`]. Guards live in app data, outside the vault, so the run's own
//! agent can't edit them, and are removed when the run finalizes
//! ([`remove_guard`]). Effects Hollow doesn't execute (shell,
//! other servers) can't be blocked; [`retry_note`] lists them in the retry's
//! prompt and the run review says so.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Most recorded effects kept per receipt.
const MAX_RECORDED: usize = 50;
/// Example commands kept per unknown group.
const MAX_EXAMPLES: usize = 3;
/// A label's longest target text.
const TARGET_CHARS: usize = 60;
/// A shell command's longest shown text.
const COMMAND_CHARS: usize = 120;
/// Folder (in app data) holding the live retry guards.
const GUARD_DIR: &str = "RetryGuards";
/// Guards older than this are pruned when a new one is written (a crash can
/// leave one behind; run ids are never reused, so a stale one is inert).
const GUARD_KEEP_SECS: u64 = 2 * 24 * 3600;

/// Everything this module adds to a receipt. Absent from the receipt when
/// both lists are empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutsideEffects {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recorded: Vec<OutsideEffect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknown: Vec<UnknownCalls>,
    /// Recorded effects beyond [`MAX_RECORDED`] that were not kept.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more_recorded: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl OutsideEffects {
    pub fn is_empty(&self) -> bool {
        self.recorded.is_empty() && self.unknown.is_empty()
    }

    /// An effect that happened or may have happened (`done`, `uncertain`).
    pub fn has_completed_or_uncertain(&self) -> bool {
        self.recorded.iter().any(OutsideEffect::may_have_happened) || self.more_recorded > 0
    }

    /// Whether a run with these effects may be retried automatically: only
    /// when nothing outside may have happened and nothing is unknown.
    pub fn allows_automatic_retry(&self) -> bool {
        !self.has_completed_or_uncertain() && self.unknown.is_empty()
    }
}

/// One recorded effect.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutsideEffect {
    /// `connector` (a built-in connector write), `hollow` (a Hollow app or
    /// task change), `shell` (a recognized command), `tool` (another MCP
    /// server's write-like tool) or `mac` (opening or quitting an app or URL).
    pub kind: String,
    /// What it was, for people: `Tasks · create task “Weekly notes”`.
    pub label: String,
    /// The agent's tool name.
    pub tool: String,
    /// `done` | `failed` | `uncertain`.
    pub outcome: String,
    /// The guard key when Hollow executes this call itself and can refuse a
    /// repeat; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub sequence: u64,
}

impl OutsideEffect {
    pub fn may_have_happened(&self) -> bool {
        self.outcome != "failed"
    }
}

/// Calls whose effects Hollow can't see, grouped.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnknownCalls {
    /// `shell` | `tool` | `helper`.
    pub kind: String,
    /// `Shell commands`, `Linear tools`, `Helper agents`.
    pub label: String,
    pub count: u32,
    /// Up to three examples (commands are redacted and shortened).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

/// One observed tool call, as the classifier needs it.
pub struct Call<'a> {
    pub name: &'a str,
    pub input: &'a Value,
    /// The result text; None when the call never returned.
    pub result: Option<&'a str>,
    pub is_error: bool,
    pub sequence: u64,
}

/// The guard key of one call: server, tool and arguments. `serde_json`
/// (without `preserve_order`) renders object keys sorted, so the same
/// arguments give the same key whichever order the agent wrote them in.
pub fn effect_key(server: &str, tool: &str, args: &Value) -> String {
    let mut args = if args.is_null() {
        Value::Object(Default::default())
    } else {
        args.clone()
    };
    // A task write's idempotency key is the agent's own pick, and a fresh
    // retry run picks a new one: the same write must still match.
    if server == "hollow_tasks" {
        if let Some(map) = args.as_object_mut() {
            map.remove("idempotencyKey");
        }
    }
    let digest = Sha256::digest(format!("{server}/{tool}\n{args}").as_bytes());
    format!("{digest:x}")[..16].to_string()
}

/// `mcp__<server>__<tool>` → (server, tool). Codex calls are normalized to
/// this name before they are observed (`agent::codex_mcp_name`).
pub fn split_mcp(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    (!server.is_empty() && !tool.is_empty()).then_some((server, tool))
}

/// The run's outside effects, or None when it had none and nothing unknown.
pub fn classify(calls: &[Call<'_>]) -> Option<OutsideEffects> {
    let mut out = OutsideEffects::default();
    let add_unknown =
        |out: &mut OutsideEffects, kind: &str, label: String, example: Option<String>| match out
            .unknown
            .iter_mut()
            .find(|u| u.kind == kind && u.label == label)
        {
            Some(group) => {
                group.count += 1;
                if let Some(e) = example {
                    if group.examples.len() < MAX_EXAMPLES && !group.examples.contains(&e) {
                        group.examples.push(e);
                    }
                }
            }
            None => out.unknown.push(UnknownCalls {
                kind: kind.into(),
                label,
                count: 1,
                examples: example.into_iter().collect(),
            }),
        };
    for call in calls {
        match classify_call(call) {
            Class::Ignore => {}
            Class::Effect(effect) => {
                if out.recorded.len() < MAX_RECORDED {
                    out.recorded.push(effect);
                } else if effect.may_have_happened() {
                    out.more_recorded += 1;
                }
            }
            Class::Unknown {
                kind,
                label,
                example,
            } => add_unknown(&mut out, kind, label, example),
        }
    }
    (!out.is_empty()).then_some(out)
}

enum Class {
    Ignore,
    Effect(OutsideEffect),
    Unknown {
        kind: &'static str,
        label: String,
        example: Option<String>,
    },
}

/// What a `run_script` answer changed: `None` when its `calls` list names
/// only reads, `Some(tools)` with the non-read tools it named (empty when the
/// answer can't be read, so what it did is unknown).
fn script_changes(result: Option<&str>) -> Option<Vec<String>> {
    let Some(report) = result.and_then(|text| serde_json::from_str::<Value>(text).ok()) else {
        return Some(Vec::new());
    };
    let Some(calls) = report.get("calls").and_then(Value::as_array) else {
        return Some(Vec::new());
    };
    let mut changed: Vec<String> = Vec::new();
    for call in calls {
        let tool = call
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let read = matches!(
            crate::app_actions::find(tool).map(|a| a.tier),
            Some(crate::app_actions::Tier::Read)
        );
        if !read && !changed.iter().any(|known| known == tool) {
            changed.push(tool.to_string());
        }
    }
    (!changed.is_empty()).then_some(changed)
}

fn outcome(call: &Call<'_>) -> &'static str {
    match call.result {
        None => "uncertain",
        Some(text) if call.is_error => {
            let lower = text.to_ascii_lowercase();
            if lower.contains("uncertain")
                || lower.contains("\"partial\"")
                || lower.contains("outcome: partial")
                || lower.contains("partially")
                || lower.contains("may have completed")
                || lower.contains("did not answer")
                || lower.contains("timed out")
                || lower.contains("timeout")
            {
                "uncertain"
            } else {
                "failed"
            }
        }
        Some(_) => "done",
    }
}

const SHELL_TOOLS: &[&str] = &[
    "Bash",
    "bash",
    "shell",
    "Shell",
    "exec_command",
    "local_shell",
    "run_shell_command",
];
const HELPER_TOOLS: &[&str] = &["Task", "Agent", "task", "agent", "spawn_agent"];
/// Tool-name words that read (the first word of a snake/kebab/camel name).
const READ_VERBS: &[&str] = &[
    "search", "fetch", "get", "list", "read", "query", "find", "describe", "view", "lookup",
    "show", "browse", "count", "check", "status", "whoami", "ping", "download", "export",
];
/// Tool-name words that change something.
const WRITE_VERBS: &[&str] = &[
    "create", "update", "delete", "remove", "send", "post", "add", "append", "move", "set",
    "write", "publish", "merge", "insert", "upsert", "edit", "archive", "trash", "close", "reply",
    "comment", "assign", "schedule", "cancel", "invite", "upload", "share", "transfer", "pay",
    "submit", "push",
];

fn first_word(tool: &str) -> String {
    let mut word = String::new();
    for (i, c) in tool.chars().enumerate() {
        if c == '_' || c == '-' || c == '.' || (i > 0 && c.is_ascii_uppercase()) {
            if !word.is_empty() {
                break;
            }
            if c == '_' || c == '-' || c == '.' {
                continue;
            }
        }
        word.push(c.to_ascii_lowercase());
    }
    word
}

fn target_of(args: &Value) -> Option<String> {
    for key in [
        "title", "name", "page_id", "pageId", "id", "ref", "slug", "url",
    ] {
        if let Some(s) = args
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(shorten(s, TARGET_CHARS));
        }
    }
    None
}

fn shorten(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    }
}

fn words(tool: &str) -> String {
    tool.replace(['_', '-', '.'], " ")
}

fn labelled(service: &str, tool: &str, args: &Value) -> String {
    match target_of(args) {
        Some(t) => format!("{service} · {} “{t}”", words(tool)),
        None => format!("{service} · {}", words(tool)),
    }
}

/// The start of every guard refusal ([`refusal_in`]): the call was not
/// sent, and the earlier run's effect is carried on this run's receipt.
pub const REFUSAL_PREFIX: &str = "Not repeated:";

fn classify_call(call: &Call<'_>) -> Class {
    if SHELL_TOOLS.contains(&call.name) {
        return classify_shell_call(call);
    }
    if HELPER_TOOLS.contains(&call.name) {
        return Class::Unknown {
            kind: "helper",
            label: "Helper agents".into(),
            example: call
                .input
                .get("description")
                .and_then(Value::as_str)
                .map(|d| shorten(d, TARGET_CHARS)),
        };
    }
    let Some((server, tool)) = split_mcp(call.name) else {
        // The agent's own file, search and web-read tools: vault and repo
        // changes are on the receipt already; reads change nothing.
        return Class::Ignore;
    };
    let effect = |kind: &str, label: String, keyed: bool| {
        Class::Effect(OutsideEffect {
            kind: kind.into(),
            label,
            tool: call.name.to_string(),
            outcome: outcome(call).into(),
            key: keyed.then(|| effect_key(server, tool, call.input)),
            sequence: call.sequence,
        })
    };
    // A guard's refusal from one of Hollow's own executors: nothing was sent,
    // and the earlier run's effect is carried on this receipt.
    if matches!(server, "hollow_tasks" | "hollow_app")
        && call.is_error
        && call
            .result
            .is_some_and(|r| r.trim_start().starts_with(REFUSAL_PREFIX))
    {
        return Class::Ignore;
    }
    match server {
        // A teammate runs with this run's permissions and tools; what it did
        // isn't in this stream.
        "hollow_team" => Class::Unknown {
            kind: "helper",
            label: "Teammate consultations".into(),
            example: call
                .input
                .get("agent")
                .and_then(Value::as_str)
                .map(|a| shorten(a, TARGET_CHARS)),
        },
        "hollow_tasks" => match tool {
            "list_tasks" | "get_task" | "list_projects" | "get_project" | "next_claimable" => {
                Class::Ignore
            }
            _ => effect("hollow", labelled("Tasks", tool, call.input), true),
        },
        "hollow_app" => {
            let action = match tool {
                "list_actions" | "describe_action" => return Class::Ignore,
                "run_action" => call
                    .input
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                named => crate::app_mcp::FIRST_CLASS
                    .iter()
                    .find(|(t, _)| *t == named)
                    .map(|(_, id)| id.to_string())
                    .unwrap_or_default(),
            };
            // CM1c seam note (a): a code-mode script's own tier is read, but
            // its inner calls may have changed things. The report lists the
            // tools it called, so any that isn't a read (or a report that
            // can't be read) is an effect, keyed on the script like any call
            // so a guarded retry refuses the same script again.
            if action == "codemode.run" {
                return match script_changes(call.result) {
                    None => Class::Ignore,
                    Some(tools) => {
                        let label = if tools.is_empty() {
                            "Hollow · code-mode script (what it changed is unknown)".to_string()
                        } else {
                            format!("Hollow · code-mode script changed: {}", tools.join(", "))
                        };
                        effect("hollow", label, true)
                    }
                };
            }
            match crate::app_actions::find(&action).map(|a| a.tier) {
                Some(crate::app_actions::Tier::Read) => Class::Ignore,
                Some(_) => {
                    let args = if tool == "run_action" {
                        call.input.get("args").cloned().unwrap_or(Value::Null)
                    } else {
                        call.input.clone()
                    };
                    effect("hollow", labelled("Hollow", &action, &args), true)
                }
                // An unknown action never ran (the server refuses it).
                None => Class::Ignore,
            }
        }
        "hollow_computer" => match tool {
            "open_app" | "open_url" | "quit_app" => {
                effect("mac", labelled("This Mac", tool, call.input), false)
            }
            _ => Class::Ignore,
        },
        // Reads, cards, consultations, the read-only connectors; services
        // are on the receipt already.
        "hollow_vault"
        | "hollow_code_context"
        | "hollow_questions"
        | "hollow_services"
        | "hollow_m365"
        | "hollow_plaud" => Class::Ignore,
        other => {
            let verb = first_word(tool);
            if READ_VERBS.contains(&verb.as_str()) {
                Class::Ignore
            } else if WRITE_VERBS.contains(&verb.as_str()) {
                // Another server's error isn't proof nothing happened.
                match effect(
                    "tool",
                    labelled(&service_name(other), tool, call.input),
                    false,
                ) {
                    Class::Effect(mut e) if e.outcome == "failed" => {
                        e.outcome = "uncertain".into();
                        Class::Effect(e)
                    }
                    class => class,
                }
            } else {
                Class::Unknown {
                    kind: "tool",
                    label: format!("{} tools", service_name(other)),
                    example: Some(words(tool)),
                }
            }
        }
    }
}

/// `claude_ai_Linear` → `Linear`; `my-server` stays.
fn service_name(server: &str) -> String {
    server
        .strip_prefix("claude_ai_")
        .unwrap_or(server)
        .replace('_', " ")
}

fn classify_shell_call(call: &Call<'_>) -> Class {
    let command = shell_script(call.input);
    let shown = shorten(&crate::claude_runner::redact(&command), COMMAND_CHARS);
    match classify_shell(&command) {
        Shell::ReadOnly => Class::Ignore,
        Shell::Effect(what) => Class::Effect(OutsideEffect {
            kind: "shell".into(),
            label: format!("{what} · {shown}"),
            tool: call.name.to_string(),
            // A failing exit status doesn't prove the effect didn't happen
            // (`git push && npm test`): never "failed", at worst uncertain.
            outcome: match outcome(call) {
                "failed" => "uncertain",
                other => other,
            }
            .into(),
            key: None,
            sequence: call.sequence,
        }),
        Shell::Unknown => Class::Unknown {
            kind: "shell",
            label: "Shell commands".into(),
            example: (!shown.is_empty()).then_some(shown),
        },
    }
}

/// The script a shell tool call (`Bash`, Codex's `exec_command`, …) ran,
/// when `name` is one: its `command` (or `cmd`; an argv array joined by
/// spaces), with Codex's `<shell> -lc '<script>'` wrapper taken off. Also
/// the task-state record's read-only shell test (`task_state::fold`).
pub(crate) fn shell_command(name: &str, input: &Value) -> Option<String> {
    SHELL_TOOLS.contains(&name).then(|| shell_script(input))
}

/// The `.md` files a shell tool call reads with `cat`, `sed`, `head` or
/// `tail` (S26, the manifest's `agentReads.notesOpened`), as the script
/// names them (a relative one is the shell's folder's). Not a redirect's
/// target nor an in-place `sed`'s file; nothing when `name` isn't a shell
/// tool or the script can't be read.
pub(crate) fn shell_note_reads(name: &str, input: &Value) -> Vec<String> {
    shell_file_reads(name, input)
        .into_iter()
        .map(|(path, _)| path)
        .filter(|path| path.to_ascii_lowercase().ends_with(".md"))
        .collect()
}

/// The files a shell tool call reads with `cat`, `sed`, `head`, `tail` or
/// `nl` (repo features R5, the receipt's citation check; S26's
/// [`shell_note_reads`] is its `.md` part), any extension, as the script
/// names them, each with the lines it prints: `sed -n 'a,bp'` / `'ap'` and
/// `head -n N` (`-N`, default 10) give a window; `cat`, `tail`, `nl` and any
/// other script read the whole file (`None`). A window piped after a whole
/// read narrows it (`nl -ba f | sed -n '5,9p'`). Not a redirect's target nor
/// an in-place `sed`'s file; nothing when `name` isn't a shell tool or the
/// script can't be read.
pub(crate) fn shell_file_reads(name: &str, input: &Value) -> Vec<(String, Option<(u32, u32)>)> {
    let Some(script) = shell_read_script(name, input) else {
        return Vec::new();
    };
    let Some(segments) = split_joined(&script, true) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Option<(u32, u32)>)> = Vec::new();
    // The reads a `|` feeds into the next command.
    let mut piped: Vec<usize> = Vec::new();
    for segment in segments {
        let words = shell_words(&segment.text);
        // `LC_ALL=C cat …`: past the assignments.
        let mut words = words
            .iter()
            .skip_while(|w| crate::evidence::is_assignment(w));
        let program = words
            .next()
            .map(|p| p.rsplit('/').next().unwrap_or(""))
            .unwrap_or("");
        let args: Vec<&str> = words.map(String::as_str).collect();
        match reader_operands(program, &args) {
            Some((files, window)) if files.is_empty() => {
                // A filter on its input: it narrows what was piped in.
                if let Some((from, to)) = window {
                    for &i in &piped {
                        out[i].1 = Some(match out[i].1 {
                            None => (from, to),
                            Some((start, end)) => (
                                start.saturating_add(from - 1),
                                end.min(start.saturating_add(to - 1)),
                            ),
                        });
                    }
                }
            }
            Some((files, window)) => {
                let start = out.len();
                out.extend(files.into_iter().map(|file| (file, window)));
                piped = (start..out.len()).collect();
            }
            None => piped.clear(),
        }
        if segment.joint != Joint::Pipe {
            piped.clear();
        }
    }
    out
}

/// The script a shell tool call ran, for reading what it read (R5): Codex's
/// `["bash", "-lc", "<script>"]` argv kept whole ([`wrapped_script`]), else
/// [`shell_command`].
pub(crate) fn shell_read_script(name: &str, input: &Value) -> Option<String> {
    wrapped_script(name, input).or_else(|| shell_command(name, input))
}

/// Codex's `["bash", "-lc", "<script>"]` argv: its script (joining it would
/// lose the quoting).
fn wrapped_script(name: &str, input: &Value) -> Option<String> {
    let Some(Value::Array(parts)) = input.get("command").or_else(|| input.get("cmd")) else {
        return None;
    };
    let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
    (SHELL_TOOLS.contains(&name)
        && parts.len() == 3
        && matches!(parts[0].rsplit('/').next(), Some("bash" | "sh" | "zsh"))
        && matches!(parts[1], "-lc" | "-c"))
    .then(|| parts[2].to_string())
}

/// What one simple command of [`shell_file_reads`] reads: its file operands
/// and the window it prints, or `None` when it isn't a reader (or is an
/// in-place `sed`). Empty operands: it reads its input.
fn reader_operands(program: &str, args: &[&str]) -> Option<(Vec<String>, Option<(u32, u32)>)> {
    if !matches!(program, "cat" | "sed" | "head" | "tail" | "nl") {
        return None;
    }
    // `-i`, `-i.bak`, `-Ei`, `-ni` (BSD's `-I` too), `--in-place[=…]`.
    let in_place = |w: &&str| match w.strip_prefix("--") {
        Some(long) => long == "in-place" || long.starts_with("in-place="),
        None => w
            .strip_prefix('-')
            .is_some_and(|flags| flags.contains(['i', 'I'])),
    };
    if program == "sed" && args.iter().any(in_place) {
        return None;
    }
    // Options whose value is the next word.
    let takes_value = |flag: &str| match program {
        "head" | "tail" => matches!(flag, "-n" | "-c"),
        "nl" => matches!(
            flag,
            "-b" | "-d" | "-f" | "-h" | "-i" | "-l" | "-n" | "-s" | "-v" | "-w"
        ),
        _ => false,
    };
    let mut files = Vec::new();
    let mut scripts: Vec<String> = Vec::new();
    let mut quiet = false;
    let mut count: Option<u32> = None;
    // `head -c N`: bytes, not lines, so no line window.
    let mut bytes = false;
    let mut target = false;
    let mut value_of: Option<String> = None;
    for word in args {
        if std::mem::take(&mut target) {
            continue;
        }
        if let Some(flag) = value_of.take() {
            match flag.as_str() {
                "-e" => scripts.push(word.to_string()),
                // A script file: a script, but not one read here.
                "-f" => scripts.push(String::new()),
                "-n" if program == "head" => count = word.parse().ok(),
                "-c" if program == "head" => bytes = true,
                _ => {}
            }
            continue;
        }
        let word = match word.split_once('>') {
            Some((before, after)) => {
                // `> out.md` (`>>`, `2>`, `&>`): the next word is the
                // file written; `>out.md` is this one; `2>&1` writes none.
                target = after.trim_start_matches('>').is_empty();
                // `a.md>b.md`: the word before the redirect is still read.
                if before.chars().all(|c| c.is_ascii_digit() || c == '&') {
                    continue;
                }
                before
            }
            None => word,
        };
        let word = word.strip_prefix('<').unwrap_or(word);
        if word.is_empty() || word == "-" {
            continue;
        }
        if let Some(flags) = word.strip_prefix('-') {
            if program == "sed" {
                if let Some(long) = flags.strip_prefix('-') {
                    quiet |= matches!(long, "quiet" | "silent");
                    if let Some(script) = long.strip_prefix("expression=") {
                        scripts.push(script.to_string());
                    }
                    continue;
                }
                quiet |= flags.contains('n');
                // `-e SCRIPT` / `-eSCRIPT` / `-ne SCRIPT`; `-f FILE`.
                if let Some(at) = flags.find(['e', 'f']) {
                    let rest = &flags[at + 1..];
                    if rest.is_empty() {
                        value_of = Some(format!("-{}", &flags[at..=at]));
                    } else if flags.as_bytes()[at] == b'e' {
                        scripts.push(rest.to_string());
                    }
                }
            } else if program == "head" {
                if let Some(n) = flags.strip_prefix("n").filter(|n| !n.is_empty()) {
                    count = n.parse().ok();
                } else if let Some(n) = flags.strip_prefix("-lines=") {
                    count = n.parse().ok();
                } else if flags.starts_with("-bytes") || (flags.starts_with('c') && flags.len() > 1)
                {
                    bytes = true;
                } else if flags.bytes().all(|b| b.is_ascii_digit()) {
                    count = flags.parse().ok();
                } else if takes_value(word) {
                    value_of = Some(word.to_string());
                }
            } else if takes_value(word) {
                value_of = Some(word.to_string());
            }
            continue;
        }
        // sed's first operand is its script, unless `-e` gave one.
        if program == "sed" && scripts.is_empty() {
            scripts.push(word.to_string());
            continue;
        }
        if program == "tail" && word.starts_with('+') {
            continue;
        }
        files.push(word.to_string());
    }
    let window = match program {
        "head" if bytes => None,
        "head" => Some((1, count.unwrap_or(10).max(1))),
        "sed" if quiet && scripts.len() == 1 => sed_window(&scripts[0]),
        _ => None,
    };
    Some((files, window))
}

/// `a,bp` → `(a, b)`, `ap` → `(a, a)`; anything else prints more.
fn sed_window(script: &str) -> Option<(u32, u32)> {
    let range = script.trim().strip_suffix('p')?;
    let (from, to) = range.split_once(',').unwrap_or((range, range));
    let (from, to) = (
        from.trim().parse::<u32>().ok()?,
        to.trim().parse::<u32>().ok()?,
    );
    (from >= 1 && to >= from).then_some((from, to))
}

fn shell_script(input: &Value) -> String {
    let command = match input.get("command").or_else(|| input.get("cmd")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    // Codex's `bash -lc "<script>"` form: classify the script.
    unwrap_shell_wrapper(&command)
}

/// `<shell> -lc '<script>'` (or `-c`) → the script; anything else as given.
pub(crate) fn unwrap_shell_wrapper(command: &str) -> String {
    let words = shell_words(command);
    if words.len() == 3
        && matches!(
            words[0].as_str(),
            "bash" | "sh" | "zsh" | "/bin/bash" | "/bin/sh" | "/bin/zsh"
        )
        && matches!(words[1].as_str(), "-lc" | "-c")
    {
        return words[2].clone();
    }
    command.to_string()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Shell {
    ReadOnly,
    /// A recognized outside effect, named.
    Effect(&'static str),
    Unknown,
}

/// A command's segments (split on `&&`, `||`, `;`, `|` and newlines outside
/// quotes), each classified; an effect anywhere wins, then unknown.
pub(crate) fn classify_shell(command: &str) -> Shell {
    if command.trim().is_empty() {
        return Shell::Unknown;
    }
    // A bounded test / build / typecheck / lint invocation (the receipt's own
    // check rule): local work, not an outside effect.
    if crate::evidence::check_kind(command).is_some() {
        return Shell::ReadOnly;
    }
    let Some(segments) = split_segments(command) else {
        return Shell::Unknown;
    };
    let mut unknown = false;
    for segment in segments {
        match classify_segment(&segment) {
            Shell::Effect(what) => return Shell::Effect(what),
            Shell::Unknown => unknown = true,
            Shell::ReadOnly => {}
        }
    }
    if unknown {
        Shell::Unknown
    } else {
        Shell::ReadOnly
    }
}

/// Quote-aware split into simple commands. None for command substitution,
/// subshells or an unclosed quote (unknown, never read-only). A redirect to
/// a file marks its segment with a leading `>`.
fn split_segments(command: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for segment in split_joined(command, false)? {
        out.push(mark(segment.text, segment.redirect));
    }
    Some(out)
}

/// How a simple command is joined to the one after it ([`split_joined`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Joint {
    /// The script's last command (a trailing `;` or newline reads the same).
    End,
    /// `;` or a newline.
    Then,
    /// `&&`.
    And,
    /// `||`.
    Or,
    /// `|` (and `|&`).
    Pipe,
    /// A single `&`: run in the background (not `&>file`, a redirect).
    Background,
}

/// One simple command of [`split_joined`]: its text, whether it redirects
/// to a file (not `2>&1` or `/dev/null`), and how it joins the next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub text: String,
    pub redirect: bool,
    pub joint: Joint,
}

/// [`split_segments`] keeping each command's [`Joint`] (OF16: a receipt
/// check's exit status only speaks for it when nothing after it masks it).
/// `comments`: drop `# …` to the end of the line (a `#` that starts a word
/// outside quotes), so an apostrophe or a parenthesis in a comment doesn't
/// make the whole script unreadable. Empty commands are left out.
pub(crate) fn split_joined(command: &str, comments: bool) -> Option<Vec<Segment>> {
    fn end(out: &mut Vec<Segment>, cur: &mut String, redirect: &mut bool, joint: Joint) {
        out.push(Segment {
            text: std::mem::take(cur),
            redirect: std::mem::take(redirect),
            joint,
        });
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    let mut redirect = false;
    let mut word_start = true;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if q == '"' && c == '\\' {
                // `\"` inside double quotes doesn't close them.
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
                continue;
            } else if q == '"' && (c == '`' || (c == '$' && chars.peek() == Some(&'('))) {
                return None;
            }
            cur.push(c);
            continue;
        }
        // OF16 re-review: `\` outside quotes escapes the next character. A
        // line continuation (`\` + newline) is whitespace, so `cargo test \⏎
        // | head` stays one pipeline; any other escaped character is kept
        // with its `\` for [`shell_words`] and is never a separator (`\;`,
        // `\&`, `\|`).
        if c == '\\' {
            match chars.next() {
                Some('\n') => {
                    cur.push(' ');
                    word_start = true;
                }
                Some(n) => {
                    cur.push(c);
                    cur.push(n);
                    word_start = false;
                }
                None => cur.push(c),
            }
            continue;
        }
        let starts_word = word_start;
        word_start = c.is_whitespace() || matches!(c, ';' | '&' | '|');
        match c {
            '#' if comments && starts_word => {
                while chars.peek().is_some_and(|n| *n != '\n') {
                    chars.next();
                }
            }
            '\'' | '"' => {
                quote = Some(c);
                cur.push(c);
            }
            '`' | '(' | ')' => return None,
            '$' if chars.peek() == Some(&'(') => return None,
            ';' | '\n' => end(&mut out, &mut cur, &mut redirect, Joint::Then),
            // `&>file`: stdout and stderr to a file, not a background run.
            '&' if chars.peek() == Some(&'>') => cur.push(c),
            '&' | '|' => {
                let doubled = chars.peek() == Some(&c);
                if doubled {
                    chars.next();
                } else if c == '|' && chars.peek() == Some(&'&') {
                    // `|&`: a pipe of stdout and stderr.
                    chars.next();
                }
                let joint = match (c, doubled) {
                    ('&', true) => Joint::And,
                    ('&', false) => Joint::Background,
                    (_, true) => Joint::Or,
                    _ => Joint::Pipe,
                };
                end(&mut out, &mut cur, &mut redirect, joint);
            }
            '>' => {
                cur.push(c);
                if chars.peek() == Some(&'>') {
                    cur.push('>');
                    chars.next();
                }
                // `2>&1`: a descriptor copy, not a file.
                if chars.peek() == Some(&'&') {
                    cur.push('&');
                    chars.next();
                    while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit() || **d == '-') {
                        cur.push(d);
                        chars.next();
                    }
                    continue;
                }
                // `>/dev/null` changes no file worth naming.
                let rest: String = chars.clone().collect();
                if !rest.trim_start().starts_with("/dev/null") {
                    redirect = true;
                }
            }
            _ => cur.push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    end(&mut out, &mut cur, &mut redirect, Joint::End);
    // An empty command is left out, but a `|`, `||`, `&&` or `&` it carries
    // joins the command before it (`cargo test ;⏎ | head` is still a pipe
    // into `head`): dropping it with the empty command would make the
    // filter's input look like the script's last command. Nothing before it
    // to join, or a joint already there (`a && | b`): unreadable.
    let mut kept: Vec<Segment> = Vec::with_capacity(out.len());
    for segment in out {
        if !segment.text.trim().is_empty() {
            kept.push(segment);
            continue;
        }
        if matches!(segment.joint, Joint::Then | Joint::End) {
            continue;
        }
        match kept.last_mut() {
            Some(prev) if prev.joint == Joint::Then => prev.joint = segment.joint,
            _ => return None,
        }
    }
    let mut out = kept;
    if let Some(last) = out.last_mut().filter(|s| s.joint == Joint::Then) {
        last.joint = Joint::End;
    }
    Some(out)
}

fn mark(segment: String, redirect: bool) -> String {
    if redirect {
        format!(">{segment}")
    } else {
        segment
    }
}

pub(crate) fn shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = segment.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            // `\"`, `\\`, `\$` and `` \` `` inside double quotes: the
            // character itself; any other `\` stays.
            Some('"') if c == '\\' => match chars.next() {
                Some(n @ ('"' | '\\' | '$' | '`')) => cur.push(n),
                Some(n) => {
                    cur.push(c);
                    cur.push(n);
                }
                None => cur.push(c),
            },
            Some(_) => cur.push(c),
            // `\x` outside quotes: a literal `x` (an escaped space or quote
            // too), never a word break; a line continuation (`\` + newline)
            // is nothing (OF16 re-review).
            None if c == '\\' => match chars.next() {
                Some('\n') => {}
                Some(n) => cur.push(n),
                None => cur.push(c),
            },
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => cur.push(c),
        }
    }
    if started || !cur.is_empty() {
        words.push(cur);
    }
    words
}

const READ_ONLY_COMMANDS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "grep",
    "egrep",
    "rg",
    "wc",
    "pwd",
    "echo",
    "printf",
    "which",
    "whereis",
    "type",
    "date",
    "stat",
    "file",
    "du",
    "df",
    "tree",
    "jq",
    "uniq",
    "cut",
    "tr",
    "diff",
    "cmp",
    "sha256sum",
    "shasum",
    "md5",
    "md5sum",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "printenv",
    "true",
    "false",
    "test",
    "[",
    "sleep",
    "column",
    "nl",
    "less",
    "more",
    "bat",
    "eza",
    "xxd",
    "hexdump",
    "strings",
    "uname",
    "whoami",
    "id",
    "ps",
    "pgrep",
    "lsof",
    "cd",
    "pushd",
    "popd",
    "export",
    "set",
    "unset",
    "exit",
    "mdls",
    "sw_vers",
    "uptime",
    "nproc",
];

fn classify_segment(segment: &str) -> Shell {
    let redirect = segment.starts_with('>');
    let segment = segment.trim_start_matches('>');
    let mut words = shell_words(segment);
    // Leading `VAR=value` assignments and `sudo`/`time`/`nohup` wrappers.
    while words.first().is_some_and(|w| {
        (w.contains('=') && !w.starts_with('=') && !w.starts_with('-'))
            || matches!(
                w.as_str(),
                "sudo" | "time" | "nohup" | "command" | "exec" | "env" | "nice" | "caffeinate"
            )
    }) {
        words.remove(0);
    }
    let Some(program) = words
        .first()
        .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
    else {
        return Shell::ReadOnly;
    };
    let arg = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
    let has = |flags: &[&str]| {
        words.iter().any(|w| {
            flags
                .iter()
                .any(|f| w == f || w.starts_with(&format!("{f}=")))
        })
    };
    // A short-flag cluster holding `flag`: `-i`, `-ri`, `-ni`, `-uo`.
    let clustered = |flag: char| {
        words.iter().skip(1).any(|w| {
            w.len() > 1 && w.starts_with('-') && !w.starts_with("--") && w[1..].contains(flag)
        })
    };
    match program.as_str() {
        "git" => {
            // Skip `-C <dir>` and `-c k=v` global options.
            let mut i = 1;
            while matches!(arg(i), "-C" | "-c") {
                i += 2;
            }
            match arg(i) {
                "push" => Shell::Effect("git push"),
                // Listing branches reads; `branch -D x` / `branch new` change refs.
                "branch"
                    if words[i + 1..].iter().all(|w| {
                        matches!(
                            w.as_str(),
                            "-a" | "-r"
                                | "-l"
                                | "--list"
                                | "-v"
                                | "-vv"
                                | "--show-current"
                                | "--all"
                        )
                    }) && !redirect =>
                {
                    Shell::ReadOnly
                }
                "status" | "log" | "diff" | "show" | "rev-parse" | "ls-files" | "describe"
                | "blame" | "grep" | "reflog" | "shortlog" | "rev-list" | "cat-file"
                | "ls-remote" | "merge-base" | "name-rev" | "for-each-ref" | "show-ref"
                | "version" | "help" => {
                    // `git diff --output=f` writes a file.
                    if redirect || has(&["--output"]) {
                        Shell::Unknown
                    } else {
                        Shell::ReadOnly
                    }
                }
                "remote" if matches!(arg(i + 1), "" | "-v" | "show" | "get-url") => Shell::ReadOnly,
                "config" if has(&["--get", "--list", "-l", "--get-all"]) => Shell::ReadOnly,
                "worktree" | "stash" if arg(i + 1) == "list" => Shell::ReadOnly,
                // Local repository changes are the receipt's repo diff.
                "add" | "commit" | "checkout" | "switch" | "restore" | "fetch" | "pull"
                | "merge" | "rebase" | "reset" | "stash" | "tag" | "worktree" | "cherry-pick"
                | "revert" | "mv" | "rm" | "init" | "clone" | "apply" | "am" | "clean" => {
                    Shell::Unknown
                }
                _ => Shell::Unknown,
            }
        }
        "gh" => match (arg(1), arg(2)) {
            (
                "pr",
                "create" | "merge" | "close" | "comment" | "edit" | "review" | "ready" | "reopen",
            ) => Shell::Effect("GitHub pull request"),
            (
                "issue",
                "create" | "comment" | "close" | "edit" | "reopen" | "delete" | "transfer",
            ) => Shell::Effect("GitHub issue"),
            ("release", "create" | "upload" | "delete" | "edit") => Shell::Effect("GitHub release"),
            ("repo", "create" | "delete" | "edit" | "rename" | "archive" | "fork") => {
                Shell::Effect("GitHub repository")
            }
            ("workflow", "run" | "enable" | "disable") | ("run", "rerun" | "cancel") => {
                Shell::Effect("GitHub Actions")
            }
            ("secret" | "variable", "set" | "delete") => Shell::Effect("GitHub settings"),
            ("api", _) => {
                let method = words
                    .iter()
                    .position(|w| w == "-X" || w == "--method")
                    .and_then(|i| words.get(i + 1))
                    .map(|m| m.to_ascii_uppercase());
                if method.as_deref().is_some_and(|m| m != "GET")
                    || has(&["-f", "-F", "--field", "--raw-field", "--input"])
                {
                    Shell::Effect("GitHub API change")
                } else {
                    Shell::ReadOnly
                }
            }
            (
                "pr" | "issue" | "release" | "repo" | "run" | "workflow",
                "list" | "view" | "status" | "checks" | "diff",
            ) => Shell::ReadOnly,
            ("auth", "status") | ("--version", _) => Shell::ReadOnly,
            _ => Shell::Unknown,
        },
        "curl" | "wget" | "http" | "https" | "xh" => {
            let method = words
                .iter()
                .position(|w| w == "-X" || w == "--request")
                .and_then(|i| words.get(i + 1))
                .map(|m| m.to_ascii_uppercase());
            let sends = has(&[
                "-d",
                "--data",
                "--data-raw",
                "--data-binary",
                "--data-urlencode",
                "-F",
                "--form",
                "--json",
                "-T",
                "--upload-file",
                "--post-data",
                "--post-file",
            ]) || words
                .iter()
                .any(|w| w.starts_with("-d") && w.len() > 2 && program == "curl");
            if method.as_deref().is_some_and(|m| m != "GET" && m != "HEAD") || sends {
                Shell::Effect("Web request that sends data")
            } else if redirect || has(&["-o", "--output", "-O", "--remote-name"]) {
                Shell::Unknown
            } else {
                Shell::ReadOnly
            }
        }
        "npm" | "pnpm" | "yarn" if arg(1) == "publish" => Shell::Effect("Package publish"),
        "cargo" if arg(1) == "publish" => Shell::Effect("Package publish"),
        "twine" if arg(1) == "upload" => Shell::Effect("Package publish"),
        "docker" | "podman" if arg(1) == "push" => Shell::Effect("Container push"),
        "ssh" | "scp" | "sftp" => Shell::Effect("Remote machine"),
        "rsync"
            if words
                .iter()
                .skip(1)
                .any(|w| !w.starts_with('-') && w.contains(':')) =>
        {
            Shell::Effect("Remote machine")
        }
        "osascript" | "open" | "shortcuts" => Shell::Unknown,
        // A search reads, unless it runs or deletes what it finds.
        // `-fprint`, `-fprint0`, `-fprintf` and `-fls` write their list to a file.
        "find"
            if !has(&["-exec", "-execdir", "-ok", "-okdir", "-delete", "-fls"])
                && !words.iter().any(|w| w.starts_with("-fprint"))
                && !redirect =>
        {
            Shell::ReadOnly
        }
        "fd" if !has(&["-x", "--exec", "-X", "--exec-batch"]) && !redirect => Shell::ReadOnly,
        "sort" if !clustered('o') && !has(&["--output"]) && !redirect => Shell::ReadOnly,
        // In place: `-i`, `-i.bak`, `-ri`, `-Ei`, `-ni`, `--in-place[=suffix]`.
        "sed"
            if !clustered('i')
                && !words.iter().any(|w| w.starts_with("--in-place"))
                && !redirect =>
        {
            Shell::ReadOnly
        }
        // `tree -o file` writes its listing.
        "tree" if clustered('o') => Shell::Unknown,
        p if READ_ONLY_COMMANDS.contains(&p) && !redirect => Shell::ReadOnly,
        _ => Shell::Unknown,
    }
}

// ── Retry guards ──────────────────────────────────────────────────────────

/// The live guard of one guarded retry run.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetryGuard {
    pub run_id: String,
    /// The history run this one retries.
    pub retry_of: String,
    pub created: String,
    pub keys: Vec<GuardKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuardKey {
    pub key: String,
    pub label: String,
    pub outcome: String,
}

fn guard_root() -> Result<PathBuf, String> {
    crate::agent_settings::app_data_dir().map(|d| d.join(GUARD_DIR))
}

fn safe_run_id(run_id: &str) -> Option<&str> {
    let id = run_id.trim();
    (!id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.starts_with('.'))
    .then_some(id)
}

fn guard_path(root: &Path, run_id: &str) -> Option<PathBuf> {
    safe_run_id(run_id).map(|id| root.join(format!("{id}.json")))
}

/// The guard a retry of a run with `effects` needs: the keys of its `done`
/// and `uncertain` keyed effects. None when there is nothing Hollow can block.
pub fn guard_for(run_id: &str, retry_of: &str, effects: &OutsideEffects) -> Option<RetryGuard> {
    let keys: Vec<GuardKey> = effects
        .recorded
        .iter()
        .filter(|e| e.may_have_happened())
        .filter_map(|e| {
            e.key.as_ref().map(|key| GuardKey {
                key: key.clone(),
                label: e.label.clone(),
                outcome: e.outcome.clone(),
            })
        })
        .collect();
    (!keys.is_empty()).then(|| RetryGuard {
        run_id: run_id.to_string(),
        retry_of: retry_of.to_string(),
        created: chrono::Utc::now().to_rfc3339(),
        keys,
    })
}

/// Write `guard` before the run spawns. An error means the guard isn't in
/// place, so the caller must not start the retry.
pub fn write_guard(guard: &RetryGuard) -> Result<(), String> {
    write_guard_in(&guard_root()?, guard)
}

pub(crate) fn write_guard_in(root: &Path, guard: &RetryGuard) -> Result<(), String> {
    let path = guard_path(root, &guard.run_id).ok_or("invalid run id for a retry guard")?;
    std::fs::create_dir_all(root).map_err(|e| format!("couldn't prepare the retry guard: {e}"))?;
    prune(root);
    let bytes = serde_json::to_vec_pretty(guard).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, &bytes)
        .map_err(|e| format!("couldn't write the retry guard: {e}"))
}

/// Remove `run_id`'s guard (its run finalized). Best effort.
pub fn remove_guard(run_id: &str) {
    if let Ok(root) = guard_root() {
        remove_guard_in(&root, run_id);
    }
}

pub(crate) fn remove_guard_in(root: &Path, run_id: &str) {
    if let Some(path) = guard_path(root, run_id) {
        let _ = std::fs::remove_file(path);
    }
}

fn prune(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > GUARD_KEEP_SECS);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn read_guard_in(root: &Path, run_id: &str) -> Option<RetryGuard> {
    let path = guard_path(root, run_id)?;
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The refusal for a call `run_id` makes when its guard names the same call
/// (server, tool, arguments), or None.
pub fn refusal(run_id: &str, server: &str, tool: &str, args: &Value) -> Option<String> {
    refusal_in(&guard_root().ok()?, run_id, server, tool, args)
}

pub(crate) fn refusal_in(
    root: &Path,
    run_id: &str,
    server: &str,
    tool: &str,
    args: &Value,
) -> Option<String> {
    let guard = read_guard_in(root, run_id)?;
    let key = effect_key(server, tool, args);
    let hit = guard.keys.iter().find(|k| k.key == key)?;
    let how = if hit.outcome == "uncertain" {
        "may already have happened (its reply was lost or it was partial)"
    } else {
        "already happened"
    };
    Some(format!(
        "{REFUSAL_PREFIX} this exact call ({}) {how} in the run this one retries ({}). Hollow blocks repeating it in a retry, so nothing was sent. Check its result there, or ask the person.",
        hit.label, guard.retry_of
    ))
}

/// `Shell commands` → `shell command` when there is one.
pub fn unknown_label(label: &str, count: u32) -> String {
    let lower = label.to_lowercase();
    if count == 1 {
        if let Some((first, rest)) = lower.split_once(' ') {
            if let Some(stem) = rest.strip_suffix('s') {
                return format!("{first} {stem}");
            }
        }
    }
    lower
}

/// What a guarded retry's own receipt carries from the run it retries: the
/// effects that happened or may have happened there and the calls Hollow
/// couldn't see into, marked as earlier, so a retry of the retry still guards
/// them (and its review lists them) even when this run never attempted them.
pub fn carried(retry_of: &str, effects: &OutsideEffects) -> OutsideEffects {
    let earlier = |label: &str| {
        if label.contains("(earlier run ") {
            label.to_string()
        } else {
            format!("{label} (earlier run {retry_of})")
        }
    };
    OutsideEffects {
        recorded: effects
            .recorded
            .iter()
            .filter(|e| e.may_have_happened())
            .map(|e| OutsideEffect {
                label: earlier(&e.label),
                sequence: 0,
                ..e.clone()
            })
            .collect(),
        unknown: effects
            .unknown
            .iter()
            .map(|u| UnknownCalls {
                label: earlier(&u.label),
                ..u.clone()
            })
            .collect(),
        more_recorded: effects.more_recorded,
    }
}

/// Merge `carried` in front of a run's own effects, one recorded effect per
/// key (a keyed effect this run also made keeps the earlier entry).
pub fn with_carried(
    own: Option<OutsideEffects>,
    carried: &OutsideEffects,
) -> Option<OutsideEffects> {
    if carried.is_empty() && carried.more_recorded == 0 {
        return own;
    }
    let mut out = own.unwrap_or_default();
    let mut merged: Vec<OutsideEffect> = carried.recorded.clone();
    for e in out.recorded.drain(..) {
        if e.key.is_some() && merged.iter().any(|c| c.key == e.key) {
            continue;
        }
        merged.push(e);
    }
    out.more_recorded += carried.more_recorded + merged.len().saturating_sub(MAX_RECORDED) as u32;
    merged.truncate(MAX_RECORDED);
    out.recorded = merged;
    let mut unknown = carried.unknown.clone();
    unknown.append(&mut out.unknown);
    out.unknown = unknown;
    Some(out)
}

/// The unknown group a run gets when its agent doesn't report its tool calls
/// (Antigravity): nothing it did is visible to Hollow.
pub fn unreported_calls() -> UnknownCalls {
    UnknownCalls {
        kind: "agent".into(),
        label: "Unreported agent calls".into(),
        count: 1,
        examples: Vec::new(),
    }
}

/// The note a guarded retry's prompt gets: what the failed run already did,
/// what Hollow will block, and what it can't.
pub fn retry_note(retry_of: &str, effects: &OutsideEffects) -> Option<String> {
    let happened: Vec<&OutsideEffect> = effects
        .recorded
        .iter()
        .filter(|e| e.may_have_happened())
        .collect();
    if happened.is_empty() && effects.unknown.is_empty() {
        return None;
    }
    let mut note = format!(
        "<retry-of-failed-run run=\"{retry_of}\">\nThis run retries a failed run of this automation. That run already acted outside its files; do not repeat what it did.\n"
    );
    let blocked: Vec<&&OutsideEffect> = happened.iter().filter(|e| e.key.is_some()).collect();
    let open: Vec<&&OutsideEffect> = happened.iter().filter(|e| e.key.is_none()).collect();
    if !blocked.is_empty() {
        note.push_str("Hollow refuses an identical call to these in this run:\n");
        for e in blocked {
            note.push_str(&format!("- {} ({})\n", e.label, e.outcome));
        }
    }
    if !open.is_empty() {
        note.push_str(
            "Hollow can't block these; check their result before doing any of them again:\n",
        );
        for e in open {
            note.push_str(&format!("- {} ({})\n", e.label, e.outcome));
        }
    }
    for u in &effects.unknown {
        note.push_str(&format!(
            "It also ran {} {} whose effects Hollow couldn't see{}.\n",
            u.count,
            unknown_label(&u.label, u.count),
            if u.examples.is_empty() {
                String::new()
            } else {
                format!(" (for example: {})", u.examples.join("; "))
            }
        ));
    }
    note.push_str("If you are unsure whether something already happened, check first or say so in your reply.\n</retry-of-failed-run>");
    Some(note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call<'a>(
        name: &'a str,
        input: &'a Value,
        result: Option<&'a str>,
        is_error: bool,
    ) -> Call<'a> {
        Call {
            name,
            input,
            result,
            is_error,
            sequence: 1,
        }
    }

    /// CM1c: a `run_script` that only read is no effect; one whose report
    /// lists a change, or can't be read, is one, keyed on the script so a
    /// guarded retry refuses the same script and names what changed.
    #[test]
    fn a_code_mode_script_is_an_effect_only_when_it_changed_something() {
        let input = json!({"script": "return 1"});
        let name = "mcp__hollow_app__run_script";
        let reads = r#"{"ok":true,"calls":[{"tool":"vault.vault_note","ok":true}]}"#;
        assert!(classify(&[call(name, &input, Some(reads), false)]).is_none());
        let writes = r#"{"ok":true,"calls":[{"tool":"vault.vault_note","ok":true},{"tool":"goals.create_goal","ok":true},{"tool":"goals.create_goal","ok":true}]}"#;
        let effects = classify(&[call(name, &input, Some(writes), false)]).expect("an effect");
        assert_eq!(effects.recorded.len(), 1);
        assert_eq!(
            effects.recorded[0].label,
            "Hollow · code-mode script changed: goals.create_goal"
        );
        assert!(effects.recorded[0].key.is_some(), "keyed on the script");
        let unreadable = classify(&[call(name, &input, Some("not json"), false)]).expect("unknown");
        assert!(unreadable.recorded[0].label.contains("unknown"));
        let no_answer = classify(&[call(name, &input, None, false)]).expect("uncertain");
        assert_eq!(no_answer.recorded.len(), 1);
    }

    #[test]
    fn task_writes_are_recorded_with_a_key_and_reads_are_not() {
        let write = json!({"title": "Weekly notes", "parent_id": "abc"});
        let read = json!({"query": "x"});
        let effects = classify(&[
            call(
                "mcp__hollow_tasks__create_task",
                &write,
                Some("{\"outcome\":\"complete\"}"),
                false,
            ),
            call("mcp__hollow_tasks__list_tasks", &read, Some("[]"), false),
        ])
        .expect("one effect");
        assert_eq!(effects.recorded.len(), 1);
        let e = &effects.recorded[0];
        assert_eq!(e.kind, "hollow");
        assert_eq!(e.outcome, "done");
        assert_eq!(e.label, "Tasks · create task “Weekly notes”");
        assert_eq!(
            e.key.as_deref(),
            Some(effect_key("hollow_tasks", "create_task", &write).as_str())
        );
        assert!(effects.unknown.is_empty());
        assert!(!effects.allows_automatic_retry());
    }

    #[test]
    fn a_lost_or_partial_reply_is_uncertain_and_a_refusal_failed() {
        let args = json!({"page_id": "p"});
        let lost = classify(&[call("mcp__hollow_tasks__add_comment", &args, None, false)]).unwrap();
        assert_eq!(lost.recorded[0].outcome, "uncertain");
        let partial = classify(&[call(
            "mcp__hollow_tasks__add_comment",
            &args,
            Some("{\"outcome\":\"partial\"}"),
            true,
        )])
        .unwrap();
        assert_eq!(partial.recorded[0].outcome, "uncertain");
        let refused = classify(&[call(
            "mcp__hollow_tasks__add_comment",
            &args,
            Some("TASK_INVALID: no title"),
            true,
        )])
        .unwrap();
        assert_eq!(refused.recorded[0].outcome, "failed");
        // A failed effect alone does not block an automatic retry.
        assert!(refused.allows_automatic_retry());
        assert!(!partial.allows_automatic_retry());
    }

    #[test]
    fn the_key_ignores_argument_order() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":{"y":2,"x":1}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"x":1,"y":2},"b":1}"#).unwrap();
        assert_eq!(effect_key("s", "t", &a), effect_key("s", "t", &b));
        assert_ne!(effect_key("s", "t", &a), effect_key("s", "u", &a));
        assert_eq!(
            effect_key("s", "t", &Value::Null),
            effect_key("s", "t", &json!({}))
        );
    }

    #[test]
    fn shell_commands_split_into_effects_reads_and_unknowns() {
        assert_eq!(
            classify_shell("git status && git diff --stat"),
            Shell::ReadOnly
        );
        assert_eq!(
            classify_shell("cd repo && git push origin main"),
            Shell::Effect("git push")
        );
        assert_eq!(classify_shell("git -C /x push"), Shell::Effect("git push"));
        assert_eq!(
            classify_shell("gh pr create --title 'a; b'"),
            Shell::Effect("GitHub pull request")
        );
        assert_eq!(classify_shell("gh pr view 12"), Shell::ReadOnly);
        assert_eq!(
            classify_shell("gh api repos/a/b/issues -f title=x"),
            Shell::Effect("GitHub API change")
        );
        assert_eq!(classify_shell("gh api repos/a/b"), Shell::ReadOnly);
        assert_eq!(
            classify_shell("curl -s https://x.test/a | jq ."),
            Shell::ReadOnly
        );
        assert_eq!(
            classify_shell("curl -X POST https://x.test -d '{}'"),
            Shell::Effect("Web request that sends data")
        );
        assert_eq!(
            classify_shell("curl https://x.test --json '{}'"),
            Shell::Effect("Web request that sends data")
        );
        assert_eq!(classify_shell("ls -la > out.txt"), Shell::Unknown);
        assert_eq!(classify_shell("ls 2>&1"), Shell::ReadOnly);
        assert_eq!(classify_shell("grep -r x . 2>/dev/null"), Shell::ReadOnly);
        assert_eq!(classify_shell("python3 script.py"), Shell::Unknown);
        assert_eq!(classify_shell("echo $(whoami)"), Shell::Unknown);
        assert_eq!(classify_shell("sed -i '' s/a/b/ f"), Shell::Unknown);
        assert_eq!(classify_shell("sed -n 1,5p f"), Shell::ReadOnly);
        assert_eq!(
            classify_shell("FOO=1 rsync -a dist/ host:/srv"),
            Shell::Effect("Remote machine")
        );
        assert_eq!(
            classify_shell("npm publish"),
            Shell::Effect("Package publish")
        );
        assert_eq!(classify_shell("'unclosed"), Shell::Unknown);
        // Wrappers and commands that only look read-only.
        assert_eq!(
            classify_shell("env GIT_SSH_COMMAND=x git push"),
            Shell::Effect("git push")
        );
        assert_eq!(
            classify_shell("find . -name '*.tmp' -delete"),
            Shell::Unknown
        );
        assert_eq!(
            classify_shell("find . -name x -exec rm {} ;"),
            Shell::Unknown
        );
        assert_eq!(classify_shell("find . -name x"), Shell::ReadOnly);
        assert_eq!(classify_shell("source ./deploy.sh"), Shell::Unknown);
        assert_eq!(
            classify_shell("awk 'BEGIN{system(\"rm x\")}'"),
            Shell::Unknown
        );
        assert_eq!(classify_shell("sort -o out.txt in.txt"), Shell::Unknown);
        // Combined flag clusters and long forms that write a file.
        for command in [
            "sed -ri s/a/b/ f",
            "sed -Ei s/a/b/ f",
            "sed -ni 1p f",
            "sed --in-place=.bak s/a/b/ f",
            "sed --in-place s/a/b/ f",
            "find . -name x -fprint out.txt",
            "find . -fprint0 out.txt",
            "find . -fprintf out.txt %p",
            "find . -fls out.txt",
            "sort -uo out.txt in.txt",
            "sort --output=out.txt in.txt",
            "git diff --output=out.patch",
            "git diff --output out.patch",
            "tree -o out.txt",
            "tree -ao out.txt",
        ] {
            assert_ne!(classify_shell(command), Shell::ReadOnly, "{command}");
        }
        assert_eq!(classify_shell("sed -nE 1,5p f"), Shell::ReadOnly);
        assert_eq!(classify_shell("sort -u in.txt"), Shell::ReadOnly);
        assert_eq!(classify_shell("tree -a"), Shell::ReadOnly);
        assert_eq!(classify_shell("git branch -D old"), Shell::Unknown);
        assert_eq!(classify_shell("git branch --show-current"), Shell::ReadOnly);
        assert_eq!(classify_shell("git branch"), Shell::ReadOnly);
    }

    /// OF16: the split keeps how each command joins the next (a trailing
    /// newline is the end), and drops comments only when asked.
    #[test]
    fn segments_keep_their_joints() {
        use Joint::*;
        let script = "cd /r && npm test 2>&1 | tail -3 || true; x &\ny\n";
        let segments = split_joined(script, false).unwrap();
        let joints: Vec<Joint> = segments.iter().map(|s| s.joint).collect();
        assert_eq!(joints, vec![And, Pipe, Or, Then, Background, End]);
        assert_eq!(segments[1].text.trim(), "npm test 2>&1");
        // A comment's apostrophe or parenthesis reads only when comments go.
        assert_eq!(split_joined("# it's (slow)\nnpm test", false), None);
        let kept = split_joined("# it's (slow)\nnpm test", true).unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].text, "npm test");
        assert_eq!(kept[0].joint, End);
        // `&>file` is a redirect, `|&` a pipe: neither runs in the background.
        let both = split_joined("npm test &> t.log; tail t.log", false).unwrap();
        assert_eq!(both.len(), 2);
        assert!(both[0].redirect);
        assert_eq!(both[0].joint, Then);
        let piped = split_joined("cargo test |& tail -3", false).unwrap();
        let joints: Vec<Joint> = piped.iter().map(|s| s.joint).collect();
        assert_eq!(joints, vec![Pipe, End]);
    }

    /// OF16 re-review: a line continuation is whitespace and an escaped
    /// character never a separator, so the pipe or `&` after a `\`⏎ stays
    /// on the command before it; an empty command's `|`/`&` moves onto the
    /// command before it instead of vanishing with it.
    #[test]
    fn line_continuations_and_escapes_keep_the_joint() {
        use Joint::*;
        let joints = |script: &str| -> Option<Vec<Joint>> {
            split_joined(script, true).map(|s| s.iter().map(|s| s.joint).collect())
        };
        let head = "cargo test --workspace 2>&1 \\\n  | head -40";
        let segments = split_joined(head, true).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].joint, Pipe);
        assert_eq!(
            shell_words(&segments[0].text),
            vec!["cargo", "test", "--workspace", "2>&1"]
        );
        assert_eq!(joints("cargo test \\\n  &"), Some(vec![Background]));
        assert_eq!(joints("cargo test \\\n  && echo ok"), Some(vec![And, End]));
        // `\;`, `\|` and `\&` are words, not separators.
        assert_eq!(joints("find . -exec rm {} \\; ; ls"), Some(vec![Then, End]));
        assert_eq!(joints("echo a\\|b \\& c"), Some(vec![End]));
        assert_eq!(
            shell_words("echo a\\|b \\& \"q \\\" r\""),
            vec!["echo", "a|b", "&", "q \" r"]
        );
        // An empty command's pipe joins the one before it; with nothing
        // before it, or a joint already there, the script is unreadable.
        assert_eq!(joints("cargo test;\n| head -40"), Some(vec![Pipe, End]));
        assert_eq!(joints("| head"), None);
        assert_eq!(joints("cargo test && | head"), None);
    }

    /// S26: the notes a shell command reads (`cat`, `sed`, `head`, `tail`),
    /// never a redirect's target, an in-place edit or another program's.
    #[test]
    fn shell_note_reads_name_the_markdown_a_command_reads() {
        let reads = |command: &str| shell_note_reads("Bash", &json!({ "command": command }));
        assert_eq!(
            reads("sed -n 1,40p /v/notes/a.md && LC_ALL=C head -5 '../lib/B.MD' 2>&1 | tail"),
            ["/v/notes/a.md", "../lib/B.MD"]
        );
        assert_eq!(reads("cat < notes/c.md > notes/out.md"), ["notes/c.md"]);
        assert_eq!(
            reads("/bin/cat notes/d.md >>log.md notes/e.txt"),
            ["notes/d.md"]
        );
        assert!(reads("sed -i '' 's/a/b/' notes/a.md").is_empty());
        // S26 review: a redirect glued to the read keeps the read; any
        // in-place flag cluster is an edit, not a read.
        assert_eq!(reads("cat a.md>b.md"), ["a.md"]);
        assert_eq!(reads("cat a.md>>b.md c.md"), ["a.md", "c.md"]);
        assert_eq!(reads("cat a.md> b.md"), ["a.md"]);
        assert!(reads("sed -Ei 's/a/b/' notes/a.md").is_empty());
        assert!(reads("sed -ni '1p' notes/a.md").is_empty());
        assert!(reads("sed --in-place 's/a/b/' notes/a.md").is_empty());
        assert!(reads("sed --in-place=.bak 's/a/b/' notes/a.md").is_empty());
        assert_eq!(reads("sed -En '1p' notes/a.md"), ["notes/a.md"]);
        assert!(reads("rg x notes/a.md; grep -n y notes/b.md").is_empty());
        assert!(shell_note_reads("Read", &json!({ "command": "cat notes/a.md" })).is_empty());
    }

    /// R5: every file a shell command reads, any extension, with the lines
    /// it prints (`sed -n`, `head`, a window piped after a whole read).
    #[test]
    fn shell_file_reads_name_each_file_and_its_lines() {
        let reads = |command: &str| shell_file_reads("Bash", &json!({ "command": command }));
        let s = |p: &str, w: Option<(u32, u32)>| (p.to_string(), w);
        assert_eq!(
            reads("sed -n '10,40p' src/a.rs; head -n 5 b.ts; head -3 c.py; cat d.toml e.json"),
            [
                s("src/a.rs", Some((10, 40))),
                s("b.ts", Some((1, 5))),
                s("c.py", Some((1, 3))),
                s("d.toml", None),
                s("e.json", None),
            ]
        );
        assert_eq!(
            reads("nl -ba src/x.rs | sed -n '120,180p'"),
            [s("src/x.rs", Some((120, 180)))]
        );
        assert_eq!(reads("cat -n f.rs | head -20"), [s("f.rs", Some((1, 20)))]);
        assert_eq!(reads("sed -n -e 7p g.rs"), [s("g.rs", Some((7, 7)))]);
        assert_eq!(reads("tail -n +10 h.rs"), [s("h.rs", None)]);
        // R5 review: `head -c` counts bytes, so it's the whole file.
        assert_eq!(
            reads("head -c 200 h2.rs; head -c200 h3.rs"),
            [s("h2.rs", None), s("h3.rs", None)]
        );
        assert_eq!(reads("sed 's/a/b/' i.rs"), [s("i.rs", None)]);
        assert_eq!(reads("sed -n '1,5p;9p' j.rs"), [s("j.rs", None)]);
        assert!(reads("sed -i 's/a/b/' k.rs; grep -n x l.rs").is_empty());
        // Codex's argv form keeps its script's quoting.
        assert_eq!(
            shell_file_reads(
                "exec_command",
                &json!({ "cmd": ["bash", "-lc", "sed -n '3,4p' m.rs"] })
            ),
            [s("m.rs", Some((3, 4)))]
        );
    }

    #[test]
    fn unknown_calls_are_grouped_with_redacted_examples() {
        let a = json!({"command": "python3 a.py"});
        let b = json!({"command": "python3 b.py"});
        let c = json!({"command": "ls"});
        let helper = json!({"description": "Research"});
        let other = json!({"x": 1});
        let effects = classify(&[
            call("Bash", &a, Some("ok"), false),
            call("Bash", &b, Some("ok"), false),
            call("Bash", &c, Some("ok"), false),
            call("Task", &helper, Some("done"), false),
            call(
                "mcp__claude_ai_Linear__triage_issue",
                &other,
                Some("ok"),
                false,
            ),
            call(
                "mcp__claude_ai_Linear__get_issue",
                &other,
                Some("ok"),
                false,
            ),
        ])
        .unwrap();
        assert!(effects.recorded.is_empty());
        let shell = effects.unknown.iter().find(|u| u.kind == "shell").unwrap();
        assert_eq!(shell.count, 2);
        assert_eq!(shell.examples, vec!["python3 a.py", "python3 b.py"]);
        assert!(effects
            .unknown
            .iter()
            .any(|u| u.kind == "helper" && u.count == 1));
        assert!(effects
            .unknown
            .iter()
            .any(|u| u.label == "Linear tools" && u.count == 1));
        assert!(
            !effects.allows_automatic_retry(),
            "unknown coverage is never nothing"
        );
    }

    #[test]
    fn other_servers_write_verbs_are_recorded_without_a_key() {
        let args = json!({"title": "Bug"});
        let effects = classify(&[call(
            "mcp__claude_ai_Linear__create_issue",
            &args,
            Some("ok"),
            false,
        )])
        .unwrap();
        assert_eq!(effects.recorded[0].kind, "tool");
        assert_eq!(effects.recorded[0].label, "Linear · create issue “Bug”");
        assert!(effects.recorded[0].key.is_none());
    }

    #[test]
    fn hollow_app_changes_are_keyed_and_reads_ignored() {
        let start = json!({"id": "sessions.start", "args": {"workspace": "w", "prompt": "p"}});
        let tree = json!({"id": "vault.vault_tree"});
        let effects = classify(&[
            call("mcp__hollow_app__run_action", &start, Some("{}"), false),
            call("mcp__hollow_app__run_action", &tree, Some("{}"), false),
            call("mcp__hollow_app__list_actions", &tree, Some("{}"), false),
        ])
        .unwrap();
        assert_eq!(effects.recorded.len(), 1);
        assert_eq!(effects.recorded[0].kind, "hollow");
        assert_eq!(
            effects.recorded[0].key.as_deref(),
            Some(effect_key("hollow_app", "run_action", &start).as_str())
        );
    }

    #[test]
    fn nothing_outside_is_none() {
        let read = json!({"file_path": "/v/a.md"});
        assert!(classify(&[
            call("Read", &read, Some("x"), false),
            call("Write", &read, Some("ok"), false)
        ])
        .is_none());
    }

    #[test]
    fn a_guard_refuses_only_the_same_call_and_goes_away() {
        let root = tempfile::tempdir().unwrap();
        let args = json!({"title": "Weekly notes"});
        let effects = classify(&[
            call("mcp__hollow_tasks__create_task", &args, Some("{}"), false),
            call("Bash", &json!({"command": "git push"}), Some("ok"), false),
            call(
                "mcp__hollow_tasks__add_comment",
                &json!({"page_id": "p"}),
                Some("TASK_INVALID"),
                true,
            ),
        ])
        .unwrap();
        let guard = guard_for("run-2", "run-1", &effects).expect("one keyed effect");
        assert_eq!(
            guard.keys.len(),
            1,
            "a failed call and an unkeyed one aren't guarded"
        );
        write_guard_in(root.path(), &guard).unwrap();
        let refusal = refusal_in(root.path(), "run-2", "hollow_tasks", "create_task", &args)
            .expect("refused");
        assert!(refusal.contains("already happened") && refusal.contains("run-1"));
        assert!(refusal_in(
            root.path(),
            "run-2",
            "hollow_tasks",
            "create_task",
            &json!({"title": "Other"})
        )
        .is_none());
        assert!(refusal_in(root.path(), "run-3", "hollow_tasks", "create_task", &args).is_none());
        remove_guard_in(root.path(), "run-2");
        assert!(refusal_in(root.path(), "run-2", "hollow_tasks", "create_task", &args).is_none());
        assert!(write_guard_in(
            root.path(),
            &RetryGuard {
                run_id: "../x".into(),
                ..guard.clone()
            }
        )
        .is_err());
    }

    /// A retry of a retry still guards the first run's effects: the guarded
    /// run carries them on its own receipt, and the refused repeat isn't
    /// recorded as a fresh failed call.
    #[test]
    fn a_guarded_runs_receipt_carries_the_earlier_effects() {
        let args = json!({"title": "A"});
        let first = classify(&[call(
            "mcp__hollow_tasks__create_task",
            &args,
            Some("{}"),
            false,
        )])
        .unwrap();
        let refused = format!("{REFUSAL_PREFIX} this exact call …");
        let push = json!({"command": "git push"});
        let own = classify(&[
            call(
                "mcp__hollow_tasks__create_task",
                &args,
                Some(&refused),
                true,
            ),
            call("Bash", &push, Some("rejected"), true),
        ]);
        let second = with_carried(own, &carried("run-1", &first)).unwrap();
        let unknown_only = OutsideEffects {
            unknown: vec![UnknownCalls {
                kind: "shell".into(),
                label: "Shell commands".into(),
                count: 2,
                examples: vec![],
            }],
            ..Default::default()
        };
        let carried_unknown = with_carried(None, &carried("run-1", &unknown_only)).unwrap();
        assert_eq!(
            carried_unknown.unknown[0].label,
            "Shell commands (earlier run run-1)"
        );
        assert!(
            !carried_unknown.allows_automatic_retry(),
            "unknown calls stay unknown on the retry"
        );
        assert_eq!(
            second.recorded[0].label,
            "Tasks · create task “A” (earlier run run-1)"
        );
        assert_eq!(second.recorded[0].outcome, "done");
        assert_eq!(second.recorded[0].key, first.recorded[0].key);
        assert!(
            !second.recorded.iter().any(|e| e.outcome == "failed"),
            "{second:?}"
        );
        assert_eq!(
            second.recorded[1].outcome, "uncertain",
            "an errored push may still have pushed"
        );
        assert!(!second.allows_automatic_retry());
        let guard = guard_for("run-3", "run-2", &second).unwrap();
        assert_eq!(guard.keys.len(), 1);
        assert_eq!(
            carried("run-2", &second).recorded[0].label,
            second.recorded[0].label,
            "labelled once"
        );
    }

    /// Review fixes: an agent-chosen idempotency key doesn't defeat the task
    /// guard; a timeout may have happened; a shell line can't hide an effect
    /// behind the refusal text; teammates are unknown; checks are local.
    #[test]
    fn keys_timeouts_teammates_and_checks() {
        assert_eq!(
            effect_key(
                "hollow_tasks",
                "create_task",
                &json!({"title": "A", "idempotencyKey": "k1"})
            ),
            effect_key(
                "hollow_tasks",
                "create_task",
                &json!({"title": "A", "idempotencyKey": "k2"})
            )
        );
        assert_ne!(
            effect_key(
                "hollow_app",
                "create_page",
                &json!({"title": "A", "idempotencyKey": "k1"})
            ),
            effect_key(
                "hollow_app",
                "create_page",
                &json!({"title": "A", "idempotencyKey": "k2"})
            )
        );
        let args = json!({"title": "A"});
        let timed = classify(&[call(
            "mcp__hollow_tasks__create_task",
            &args,
            Some("Hollow did not answer within 30 seconds; the write may have completed."),
            true,
        )])
        .unwrap();
        assert_eq!(timed.recorded[0].outcome, "uncertain");
        let push = json!({"command": "echo 'Not repeated: x'; git push"});
        let hidden = classify(&[call("Bash", &push, Some("Not repeated: x"), false)]).unwrap();
        assert_eq!(
            hidden.recorded.len(),
            1,
            "a shell effect is never hidden by the refusal text"
        );
        let team = json!({"agent": "codex", "task": "x"});
        let teammate = classify(&[call(
            "mcp__hollow_team__ask_agent",
            &team,
            Some("ok"),
            false,
        )])
        .unwrap();
        assert_eq!(teammate.unknown[0].label, "Teammate consultations");
        assert_eq!(classify_shell("npm test"), Shell::ReadOnly);
        assert_eq!(
            classify_shell("cd app && cargo test --lib"),
            Shell::ReadOnly
        );
        assert!(
            classify(&[call("mcp__x__resolve_issue", &args, Some("ok"), false)])
                .unwrap()
                .unknown
                .len()
                == 1
        );
    }

    #[test]
    fn unknown_labels_are_singular_for_one() {
        assert_eq!(unknown_label("Shell commands", 1), "shell command");
        assert_eq!(unknown_label("Shell commands", 2), "shell commands");
        assert_eq!(unknown_label("Linear tools", 1), "linear tool");
    }

    #[test]
    fn the_retry_note_separates_blocked_open_and_unknown() {
        let effects = classify(&[
            call(
                "mcp__hollow_tasks__create_task",
                &json!({"title": "A"}),
                Some("{}"),
                false,
            ),
            call("Bash", &json!({"command": "git push"}), Some("ok"), false),
            call(
                "Bash",
                &json!({"command": "python3 x.py"}),
                Some("ok"),
                false,
            ),
        ])
        .unwrap();
        let note = retry_note("run-1", &effects).unwrap();
        assert!(note.contains("Hollow refuses an identical call to these in this run:\n- Tasks · create task “A” (done)"));
        assert!(note.contains("Hollow can't block these"));
        assert!(note.contains("git push · git push (done)"));
        assert!(note.contains(
            "1 shell command whose effects Hollow couldn't see (for example: python3 x.py)"
        ));
        assert!(retry_note("run-1", &OutsideEffects::default()).is_none());
    }
}
