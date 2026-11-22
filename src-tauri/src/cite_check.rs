//! The citation check on Run records (repo features R5): every `path:line`
//! a run's final reply names, checked when the run ends for whether the file
//! exists, whether the line is in it and whether the run read it.
//!
//! [`extract`] reads the cites out of a reply (outside fenced blocks, never
//! from a URL), and [`check`] gives each one status against the run's reads
//! ([`read_windows`], [`grep_hits`]). The result is a note on the receipt
//! ([`ReceiptCites`]): it never fails a run, never changes its headline state,
//! and nothing of it is sent to an agent. Only paths and line numbers are
//! kept, never text. Separate from S26's `run_manifest::cites`, which marks
//! the retrieval passages a reply names.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

/// The most cites one reply is checked for.
pub(crate) const MAX_CITES: usize = 64;
/// The most problem cites a receipt lists ([`ReceiptCites::items`]).
pub(crate) const MAX_ITEMS: usize = 16;
/// The most grep hits one run's reads take in.
pub(crate) const MAX_HITS: usize = 256;
/// A grep hit counts as seeing the lines this far either side of it.
const HIT_SLACK: u32 = 3;
/// Claude Code's `Read` returns this many lines when given no `limit`.
const READ_DEFAULT_LINES: u32 = 2_000;
/// Files over this size aren't counted (their range is not checked).
const FILE_MAX: u64 = 16 * 1024 * 1024;
/// What one check reads, at most, counting lines across all its files.
const TOTAL_MAX: u64 = 64 * 1024 * 1024;

/// One `path:line` a reply names: `end` is `line` unless it named a range
/// (`path:10-20`, `path#L10-L20`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cite {
    pub path: String,
    pub line: u32,
    pub end: u32,
}

/// A path ending in a file name whose extension starts with a letter, then a
/// line: `:N`, `:N-M`, `:N:C`, `#LN` or `#LN-LM`. A git-ref prefix
/// (`origin/main:`, `HEAD:`, a hash and `:`) never matches, since a path
/// holds no `:`. Known misses, on purpose: a path with a space is cut at the
/// space (its last word is a bare name, so `unresolved` at worst), and a
/// Windows path (`C:\…`, backslashes) is never matched.
static CITE: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(
        r"(?:~/|/)?(?:[A-Za-z0-9_.@+\-]+/)*[A-Za-z0-9_.@+\-]*[A-Za-z0-9_@+\-]\.[A-Za-z][A-Za-z0-9]*(?::(\d+)(?:-(\d+)|:\d+)?|#L(\d+)(?:-L?(\d+))?)",
    )
    .expect("CITE compiles")
});

/// A URL, whose `host.tld:port` or `/file.ext#L3` is no cite.
static URL: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"[A-Za-z][A-Za-z0-9+.\-]*://[^\s)>\]]*").expect("URL compiles")
});

/// The cites in `reply`, in order, deduplicated, at most [`MAX_CITES`].
/// Fenced blocks (``` or ~~~, `canvas_fence`'s fence rules) are skipped, and
/// so is anything inside a URL.
pub(crate) fn extract(reply: &str) -> Vec<Cite> {
    let mut out: Vec<Cite> = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    for line in reply.split('\n') {
        if let Some((ch, run)) = fence {
            if crate::canvas_fence::fence_close(line, ch, run) {
                fence = None;
            }
            continue;
        }
        if let Some((ch, run, _)) = crate::canvas_fence::fence_open(line) {
            fence = Some((ch, run));
            continue;
        }
        let urls: Vec<(usize, usize)> = URL.find_iter(line).map(|m| (m.start(), m.end())).collect();
        for caps in CITE.captures_iter(line) {
            let whole = caps.get(0).expect("group 0");
            if urls
                .iter()
                .any(|&(a, b)| whole.start() < b && whole.end() > a)
            {
                continue;
            }
            // `a.rs:12abc` is no cite; `a.rs:12.` ends a sentence.
            if line[whole.end()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                continue;
            }
            let number = |i: usize| caps.get(i).map(|m| m.as_str().parse::<u32>());
            let (line_no, end) = match (number(1), number(3)) {
                (Some(Ok(n)), _) => (n, number(2)),
                (_, Some(Ok(n))) => (n, number(4)),
                _ => continue,
            };
            if line_no == 0 {
                continue;
            }
            let end = match end {
                Some(Ok(e)) if e >= line_no => e,
                Some(Err(_)) => continue,
                _ => line_no,
            };
            let first = caps
                .get(1)
                .or_else(|| caps.get(3))
                .expect("a line group matched");
            // The path is everything before the `:` or `#L`.
            let cut = first.start() - if caps.get(1).is_some() { 1 } else { 2 };
            let cite = Cite {
                path: line[whole.start()..cut].to_string(),
                line: line_no,
                end,
            };
            if !out.contains(&cite) {
                out.push(cite);
                if out.len() >= MAX_CITES {
                    return out;
                }
            }
        }
    }
    out
}

/// Where a read's relative path is read from first (the other is tried when
/// it isn't there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Base {
    /// The run's folder: `Read`, a shell command, a grep hit.
    Cwd,
    /// The vault: `read_source`'s vault-relative paths.
    Vault,
}

/// One file a run read and which of its lines (`None`: all of it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Named {
    pub path: String,
    pub base: Base,
    pub window: Option<(u32, u32)>,
}

/// Grok's shell tool (`friction.rs` reads it as one too). `run_effects`
/// doesn't list it, so its calls are read here as `Bash` calls.
const GROK_SHELL: &str = "run_terminal_command";

/// `name` as the shell tools `run_effects` knows it.
fn shell_name(name: &str) -> &str {
    if name == GROK_SHELL {
        "Bash"
    } else {
        name
    }
}

/// Whether tool call `name` is one [`read_windows`] reads: Claude Code's
/// `Read`, Grok's `read_file`, `read_source`, or a shell tool (Grok's
/// `run_terminal_command` too). The one list both this run's reads and
/// `step_spine::earlier_reads` keep.
pub(crate) fn is_reader(name: &str) -> bool {
    matches!(name, "Read" | "read_file")
        || name == crate::run_manifest::READ_SOURCE_TOOL
        || crate::run_effects::shell_command(shell_name(name), &Value::Null).is_some()
}

/// `path` under `base` when relative (and `base` is known); `~` is `~/`.
fn join_rel(base: Option<&str>, path: &str) -> String {
    if path == "~" {
        return "~/".into();
    }
    match base {
        Some(base) if !path.starts_with('/') && !path.starts_with("~/") => {
            format!("{}/{path}", base.trim_end_matches('/'))
        }
        _ => path.to_string(),
    }
}

/// The folder a shell call's relative paths are read from, itself relative
/// to the run's folder when relative: Codex's `exec_command` `workdir`, then
/// each leading `cd DIR &&` / `cd DIR;` (a bare `cd` is `~`, `cd -` the
/// folder before the last `cd`). `None` when there is neither. A `cd` later
/// in the script, or a `cd -` with no `cd` before it, stops the walk.
fn shell_base(name: &str, input: &Value) -> Option<String> {
    let mut base = input
        .get("workdir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .map(str::to_string);
    // The folder before the last `cd` (`cd -` goes back to it), once one ran.
    let mut previous: Option<Option<String>> = None;
    let script = crate::run_effects::shell_read_script(shell_name(name), input)?;
    for segment in crate::run_effects::split_joined(&script, true).unwrap_or_default() {
        use crate::run_effects::Joint;
        let words = crate::run_effects::shell_words(&segment.text);
        if words.first().map(String::as_str) != Some("cd")
            || words.len() > 2
            || !matches!(segment.joint, Joint::And | Joint::Then)
        {
            break;
        }
        let next = match words.get(1).map(String::as_str) {
            None => Some("~/".to_string()),
            Some("-") => match previous.take() {
                Some(before) => before,
                None => break,
            },
            // An option (`cd -P`, `cd --`): not a folder this walk follows.
            Some(flag) if flag.starts_with('-') => break,
            Some(dir) => Some(join_rel(base.as_deref(), dir)),
        };
        previous = Some(std::mem::replace(&mut base, next));
    }
    base
}

/// The files tool call `name` reads with `input`, each with the lines it
/// returns: Claude Code's `Read` (`offset`/`limit`, else lines 1–2000),
/// Grok's `read_file` (`target_file`; `offset`/`limit`, else the whole
/// file), `read_source` (`from_line`–`to_line`), and a shell command's reads
/// (`run_effects::shell_file_reads`; Grok's `run_terminal_command` too),
/// relative ones under its [`shell_base`]. Nothing for any other call
/// ([`is_reader`]). The caller keeps only calls that succeeded.
pub(crate) fn read_windows(name: &str, input: &Value) -> Vec<Named> {
    if !is_reader(name) {
        return Vec::new();
    }
    let text = |key: &str| input.get(key).and_then(Value::as_str);
    let number = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_u64)
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
    };
    if name == "Read" {
        let Some(path) = text("file_path") else {
            return Vec::new();
        };
        let start = number("offset").unwrap_or(1).max(1);
        let limit = number("limit").unwrap_or(READ_DEFAULT_LINES).max(1);
        return vec![Named {
            path: path.to_string(),
            base: Base::Cwd,
            window: Some((start, start.saturating_add(limit - 1))),
        }];
    }
    if name == "read_file" {
        let Some(path) = text("target_file").or_else(|| text("path")) else {
            return Vec::new();
        };
        let start = number("offset").unwrap_or(1).max(1);
        let window = match number("limit") {
            Some(limit) => Some((start, start.saturating_add(limit.max(1) - 1))),
            None if start > 1 => Some((start, u32::MAX)),
            None => None,
        };
        return vec![Named {
            path: path.to_string(),
            base: Base::Cwd,
            window,
        }];
    }
    if name == crate::run_manifest::READ_SOURCE_TOOL {
        let Some(path) = text("path") else {
            return Vec::new();
        };
        let from = number("from_line").unwrap_or(1).max(1);
        let lines = u32::try_from(crate::source_evidence::READ_MAX_LINES).unwrap_or(u32::MAX);
        let to = number("to_line")
            .filter(|&to| to >= from)
            .unwrap_or_else(|| from.saturating_add(lines - 1));
        return vec![Named {
            path: path.to_string(),
            base: Base::Vault,
            window: Some((from, to)),
        }];
    }
    let reads = crate::run_effects::shell_file_reads(shell_name(name), input);
    if reads.is_empty() {
        return Vec::new();
    }
    let base = shell_base(name, input);
    reads
        .into_iter()
        .map(|(path, window)| Named {
            path: join_rel(base.as_deref(), &path),
            base: Base::Cwd,
            window,
        })
        .collect()
}

/// `path:12:` or `path:12-` at a line's start: a grep line with its number.
static HIT: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"^([^\s:][^:\n]*):(\d+)[:\-]").expect("HIT compiles"));

/// `12:` or `12-` at a line's start: a single-file grep's line, no path.
static BARE_HIT: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"^(\d+)[:\-]").expect("BARE_HIT compiles"));

/// The `(path, line)` hits in the full result of a `Grep` call (Claude's,
/// or Grok's lowercase `grep`), or of a shell `grep`/`egrep`/`rg`/`git grep`
/// given line numbers (`-n`, `--line-number`, `--vimgrep`), at most
/// [`MAX_HITS`]. A grep of exactly one file prints `12:text` with no path,
/// so such lines are that file's (the tool's `path`, or the command's one
/// file operand). Claude's `Grep` prints numbers only in its `content` mode
/// with `-n` (the default there): with `-n: false` it gives no hits, and in
/// any other mode no bare ones. A shell grep reading another numbered
/// grep's output numbers that stream, not a file, so such a script gives
/// none ([`numbered_grep_operands`]). Shell paths are under the call's
/// [`shell_base`]. A result that is JSON (Grok's tools answer in JSON) is
/// read through its strings. Nothing for any other call.
pub(crate) fn grep_hits(name: &str, input: &Value, result: &str) -> Vec<(String, u32)> {
    let (single, base) = if name == "Grep" || name == "grep" {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string);
        if name == "Grep" {
            if input.get("-n").and_then(Value::as_bool) == Some(false) {
                return Vec::new();
            }
            let content = input.get("output_mode").and_then(Value::as_str) == Some("content");
            (path.filter(|_| content), None)
        } else {
            (path, None)
        }
    } else {
        let Some(script) = crate::run_effects::shell_read_script(shell_name(name), input) else {
            return Vec::new();
        };
        let Some(operands) = numbered_grep_operands(&script) else {
            return Vec::new();
        };
        let base = shell_base(name, input);
        let single = match operands.as_slice() {
            [only] => Some(join_rel(base.as_deref(), only)),
            _ => None,
        };
        (single, base)
    };
    let hit = |line: &str| -> Option<(String, u32)> {
        // `path:12:` unless its "path" is all digits: that's a single-file
        // line whose text holds `:34:`.
        if let Some(caps) = HIT.captures(line) {
            if !caps[1].bytes().all(|b| b.is_ascii_digit()) {
                let line_no = caps[2].parse::<u32>().ok().filter(|&n| n > 0)?;
                return Some((join_rel(base.as_deref(), &caps[1]), line_no));
            }
        }
        // A directory's "single" operand only keys hits to the folder,
        // which no cite names, so it does no harm.
        let caps = BARE_HIT.captures(line)?;
        let line_no = caps[1].parse::<u32>().ok().filter(|&n| n > 0)?;
        Some((single.clone()?, line_no))
    };
    let mut hits = Vec::new();
    each_result_line(result, |line| {
        hits.extend(hit(line));
        hits.len() < MAX_HITS
    });
    hits
}

/// The path operands of the script's line-numbered greps, all of them
/// together; `None` when it runs none. A pattern given with `-e`/`-f` makes
/// every operand a path; otherwise the first is the pattern. Also `None`
/// when a numbered grep reads a pipe (`grep -n x a.rs | grep -n y`): its
/// numbers count the stream's lines, and the script's output can't be told
/// apart by grep. `git grep` never reads stdin (with no path it searches
/// the work tree), so a pipe into it changes nothing.
fn numbered_grep_operands(script: &str) -> Option<Vec<String>> {
    let mut found: Option<Vec<String>> = None;
    let mut piped = false;
    for segment in crate::run_effects::split_joined(script, true)? {
        let reads_pipe =
            std::mem::replace(&mut piped, segment.joint == crate::run_effects::Joint::Pipe);
        let words = crate::run_effects::shell_words(&segment.text);
        let mut words = words
            .iter()
            .skip_while(|w| crate::evidence::is_assignment(w))
            .map(String::as_str)
            .peekable();
        let mut program = words
            .next()
            .map_or("", |p| p.rsplit('/').next().unwrap_or(""));
        let is_git = program == "git" && words.peek() == Some(&"grep");
        if is_git {
            words.next();
            program = "grep";
        }
        if !matches!(program, "grep" | "egrep" | "rg") {
            continue;
        }
        // Options whose value is the next word.
        let takes_value: &[&str] = if program == "rg" {
            &[
                "-e", "-f", "-g", "-t", "-T", "-A", "-B", "-C", "-m", "-M", "-j", "-E", "-r", "-d",
            ]
        } else {
            &["-e", "-f", "-A", "-B", "-C", "-m", "-d", "-D"]
        };
        let (mut numbered, mut pattern_given) = (false, false);
        let mut operands = Vec::new();
        let mut skip = false;
        for word in words {
            if std::mem::take(&mut skip) {
                continue;
            }
            if word.contains('>') || word.starts_with('<') {
                // A redirect: `> out` writes the next word.
                skip = word.ends_with('>');
                continue;
            }
            if let Some(long) = word.strip_prefix("--") {
                numbered |= matches!(long, "line-number" | "vimgrep");
                pattern_given |= long.starts_with("regexp") || long.starts_with("file");
                continue;
            }
            if let Some(flags) = word.strip_prefix('-').filter(|f| !f.is_empty()) {
                numbered |= flags.bytes().all(|b| b.is_ascii_alphabetic()) && flags.contains('n');
                if takes_value.contains(&word) {
                    skip = true;
                    pattern_given |= matches!(word, "-e" | "-f");
                }
                continue;
            }
            operands.push(word.to_string());
        }
        if !numbered {
            continue;
        }
        if !pattern_given && !operands.is_empty() {
            operands.remove(0);
        }
        if reads_pipe && !is_git && operands.is_empty() {
            return None;
        }
        found.get_or_insert_with(Vec::new).extend(operands);
    }
    found
}

/// Hands `each` a tool result's lines in order (a JSON result's: its
/// strings' lines), until it returns `false`. A text result's lines are
/// borrowed, never collected; a JSON one is parsed into a `Value` first.
fn each_result_line(result: &str, mut each: impl FnMut(&str) -> bool) {
    fn walk(value: &Value, each: &mut dyn FnMut(&str) -> bool) -> bool {
        match value {
            Value::String(s) => s.lines().all(&mut *each),
            Value::Array(items) => items.iter().all(|v| walk(v, each)),
            Value::Object(map) => map.values().all(|v| walk(v, each)),
            _ => true,
        }
    }
    let trimmed = result.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            walk(&value, &mut each);
            return;
        }
    }
    for line in result.lines() {
        if !each(line) {
            break;
        }
    }
}

/// A grep hit at `line` as a read window ([`HIT_SLACK`] either side).
pub(crate) fn hit_window(path: String, line: u32) -> Named {
    Named {
        path,
        base: Base::Cwd,
        window: Some((
            line.saturating_sub(HIT_SLACK).max(1),
            line.saturating_add(HIT_SLACK),
        )),
    }
}

/// What [`check`] holds the cites against.
pub(crate) struct Ctx<'a> {
    /// The run's folder (`manifest.run_cwd`), when known.
    pub cwd: Option<&'a Path>,
    pub vault: &'a Path,
    /// This run's successful reads, grep hits included.
    pub reads: Vec<Named>,
    /// What the conversation's earlier turns read (chat only).
    pub earlier: Vec<Named>,
    /// The run handed work to another agent (a subagent, a Workflow run, a
    /// consultation or child task), whose reads may not be seen here.
    pub delegated: bool,
    /// The agent reports no tool calls (Antigravity).
    pub unobserved: bool,
}

/// [`crate::evidence::EvidenceReceipt::cites`]: how the final reply's cites
/// checked out when the run ended. Each count is left out when 0; `items`
/// lists the problem cites (any status but `seen`), at most [`MAX_ITEMS`],
/// leaving out (but counting) any under a `restricted` folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptCites {
    pub checked: u32,
    /// Its line was in what the run read.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub seen: u32,
    /// The run read the file, but not the lines around that one.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub file_only: u32,
    /// An earlier turn of the conversation read the file.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub earlier: u32,
    /// Nothing seen here opened the file.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub not_opened: u32,
    /// Not opened as far as Hollow can see, but a delegation ran or the agent
    /// reports no tool calls, so it may have been read.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unknown: u32,
    /// The line is past the file's end.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub past_end: u32,
    /// No such file.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub missing: u32,
    /// A bare file name the run's reads don't settle (none or several).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub unresolved: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ReceiptCite>,
}

/// One problem cite: its path (relative to the run's folder or the vault
/// when under one, else as written), its line(s) and its status
/// (`fileOnly`, `earlier`, `notOpened`, `unknown`, `pastEnd`, `missing`,
/// `unresolved`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptCite {
    pub path: String,
    /// Exact resolved file; added only after the restricted-path filter.
    /// Legacy or unresolved records remain non-navigable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u32>,
    pub status: String,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// A read with its file's real path, once.
struct Resolved {
    key: PathBuf,
    window: Option<(u32, u32)>,
}

/// Check `cites` against `ctx` (see the module doc); `None` when there are
/// none.
pub(crate) fn check(cites: &[Cite], ctx: &Ctx<'_>) -> Option<ReceiptCites> {
    if cites.is_empty() {
        return None;
    }
    let resolve_all = |reads: &[Named]| -> Vec<Resolved> {
        reads
            .iter()
            .map(|read| Resolved {
                key: key_of(&resolve_read(ctx, read)),
                window: read.window,
            })
            .collect()
    };
    let reads = resolve_all(&ctx.reads);
    let earlier = resolve_all(&ctx.earlier);
    let mut budget = TOTAL_MAX;
    let mut lines_of: HashMap<PathBuf, Option<u32>> = HashMap::new();
    let mut out = ReceiptCites {
        checked: u32::try_from(cites.len()).unwrap_or(u32::MAX),
        ..ReceiptCites::default()
    };
    for cite in cites {
        let (status, shown) = match resolve_cite(ctx, &cite.path, &reads, &earlier) {
            Err(status) => (status, None),
            Ok(file) => {
                let key = key_of(&file);
                let lines = *lines_of
                    .entry(key.clone())
                    .or_insert_with(|| count_lines(&file, &mut budget));
                let status = if lines.is_some_and(|n| cite.end > n) {
                    "pastEnd"
                } else {
                    read_status(cite, &key, &reads, &earlier, ctx)
                };
                (status, Some(file))
            }
        };
        match status {
            "seen" => out.seen += 1,
            "fileOnly" => out.file_only += 1,
            "earlier" => out.earlier += 1,
            "notOpened" => out.not_opened += 1,
            "unknown" => out.unknown += 1,
            "pastEnd" => out.past_end += 1,
            "missing" => out.missing += 1,
            _ => out.unresolved += 1,
        }
        if status == "seen" || out.items.len() >= MAX_ITEMS {
            continue;
        }
        if restricted(Path::new(&cite.path))
            || shown
                .as_ref()
                .is_some_and(|file| restricted(file) || restricted(&key_of(file)))
        {
            continue;
        }
        let path = shown
            .as_deref()
            .and_then(|file| shown_path(ctx, file))
            .unwrap_or_else(|| cite.path.clone());
        out.items.push(ReceiptCite {
            path,
            target: shown
                .as_ref()
                .map(|file| key_of(file).to_string_lossy().into_owned()),
            line: cite.line,
            end: (cite.end != cite.line).then_some(cite.end),
            status: status.to_string(),
        });
    }
    Some(out)
}

/// `seen`, `fileOnly`, `earlier`, or not opened (`unknown` when the run
/// couldn't see every read).
fn read_status(
    cite: &Cite,
    key: &Path,
    reads: &[Resolved],
    earlier: &[Resolved],
    ctx: &Ctx<'_>,
) -> &'static str {
    let covers = |window: Option<(u32, u32)>| {
        window.is_none_or(|(from, to)| from <= cite.end && cite.line <= to)
    };
    let mine: Vec<&Resolved> = reads.iter().filter(|r| r.key == key).collect();
    if mine.iter().any(|r| covers(r.window)) {
        "seen"
    } else if !mine.is_empty() {
        "fileOnly"
    } else if earlier.iter().any(|r| r.key == key) {
        "earlier"
    } else if ctx.delegated || ctx.unobserved {
        "unknown"
    } else {
        "notOpened"
    }
}

/// The file a cite names: an absolute or `~/` path as given, else under the
/// run's folder, else under the vault. A bare file name only names a file the
/// run read by that name, and only when it read exactly one. `Err` is the
/// status when there is no file (`missing`, `unresolved`).
fn resolve_cite(
    ctx: &Ctx<'_>,
    path: &str,
    reads: &[Resolved],
    earlier: &[Resolved],
) -> Result<PathBuf, &'static str> {
    if !path.contains('/') {
        let mut found: Vec<&PathBuf> = reads
            .iter()
            .chain(earlier)
            .map(|r| &r.key)
            .filter(|key| key.file_name().is_some_and(|name| name == path))
            .collect();
        found.sort();
        found.dedup();
        return match found.as_slice() {
            [one] if one.is_file() => Ok((*one).clone()),
            _ => Err("unresolved"),
        };
    }
    let absolute = path.starts_with('/') || path.starts_with("~/");
    let candidates: Vec<PathBuf> = if absolute {
        vec![lexical(Path::new("/"), path)]
    } else {
        ctx.cwd
            .into_iter()
            .chain(std::iter::once(ctx.vault))
            .map(|base| lexical(base, path))
            .collect()
    };
    candidates
        .into_iter()
        .find(|file| file.is_file())
        .ok_or("missing")
}

/// A read's file: as given when absolute, else under its base, else under
/// the other one, else under its base (it may be gone).
fn resolve_read(ctx: &Ctx<'_>, read: &Named) -> PathBuf {
    let path = read.path.trim();
    if path.starts_with('/') || path.starts_with("~/") {
        return lexical(Path::new("/"), path);
    }
    let cwd = ctx.cwd.unwrap_or(ctx.vault);
    let (first, second) = match read.base {
        Base::Cwd => (cwd, ctx.vault),
        Base::Vault => (ctx.vault, cwd),
    };
    let primary = lexical(first, path);
    if primary.exists() {
        return primary;
    }
    let other = lexical(second, path);
    if other.exists() {
        other
    } else {
        primary
    }
}

/// `path` read from `base` when relative (`~/` from the home folder), with
/// `.` and `..` taken out lexically.
fn lexical(base: &Path, path: &str) -> PathBuf {
    let joined = match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => base.join(path),
    };
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The real path when there is one (`/var` is `/private/var`), else as given.
fn key_of(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `path` relative to `dir` when under it.
fn under<'p>(path: &'p Path, dir: &Path) -> Option<&'p Path> {
    path.strip_prefix(dir).ok()
}

/// Any folder named `restricted`, in any case (as `notes_opened` reads it).
fn restricted(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case("restricted"))
}

/// A listed cite's path relative to the run's folder, else the vault; `None`
/// outside both, so the caller keeps the path as the reply wrote it (`~/…`
/// included).
fn shown_path(ctx: &Ctx<'_>, file: &Path) -> Option<String> {
    let key = key_of(file);
    let rel = ctx
        .cwd
        .into_iter()
        .chain(std::iter::once(ctx.vault))
        .find_map(|dir| {
            under(file, dir)
                .or_else(|| under(&key, &key_of(dir)))
                .map(Path::to_path_buf)
        })?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// `file`'s line count, streamed; `None` for a binary file, one over
/// [`FILE_MAX`], one that can't be read, one that isn't a regular file or
/// whose size changes while read, or once `budget` is spent. A PDF
/// or Office file is cited by its extracted text's lines (R14a), so it
/// counts those when `doc_text` already holds them and is unknown
/// otherwise: never parsed here.
fn count_lines(file: &Path, budget: &mut u64) -> Option<u32> {
    count_lines_with(file, budget, &|| {})
}

/// [`count_lines`] with a hook run between the size check and the read
/// (tests use it to grow the file in between).
fn count_lines_with(file: &Path, budget: &mut u64, after_stat: &dyn Fn()) -> Option<u32> {
    if crate::doc_text::kind(file).is_some() {
        // Readers cache by the canonical path (a symlinked vault's cite
        // names the link).
        let text = crate::doc_text::peek(file)
            .or_else(|| crate::doc_text::peek(&key_of(file)))?
            .ok()?;
        let lines = text.bytes().filter(|&b| b == b'\n').count()
            + usize::from(!text.is_empty() && !text.ends_with('\n'));
        return Some(u32::try_from(lines).unwrap_or(u32::MAX));
    }
    // `resolve_cite` only passes regular files, but the path can be swapped
    // since (RF-788): a FIFO's blocking `open` waits for a writer.
    let meta = fs::metadata(file).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let allowed = FILE_MAX.min(*budget);
    if size > allowed {
        return None;
    }
    after_stat();
    let mut options = fs::OpenOptions::new();
    options.read(true);
    // Swapped between the stat and here: a non-blocking open of a FIFO
    // returns at once, and the opened file is checked again. On a regular
    // file `O_NONBLOCK` changes nothing.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NONBLOCK);
    let opened = options.open(file).ok()?;
    if !opened.metadata().ok()?.is_file() {
        return None;
    }
    let mut handle = opened.take(allowed + 1);
    let mut buf = vec![0u8; 64 * 1024];
    let (mut lines, mut last, mut total) = (0u64, b'\n', 0u64);
    let read_all = loop {
        let Ok(n) = handle.read(&mut buf) else {
            break false;
        };
        if n == 0 {
            break true;
        }
        let chunk = &buf[..n];
        if total == 0 && chunk.contains(&0) {
            total = n as u64;
            break false;
        }
        total += n as u64;
        lines += chunk.iter().filter(|&&b| b == b'\n').count() as u64;
        last = chunk[n - 1];
    };
    // Charge what was read, not the stat's size: a file that grew since is
    // read only to the budget's end, and then gets no count (RF-788).
    *budget -= total.min(*budget);
    if !read_all || total > allowed || total != size {
        return None;
    }
    if last != b'\n' {
        lines += 1;
    }
    Some(u32::try_from(lines).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cite(path: &str, line: u32, end: u32) -> Cite {
        Cite {
            path: path.into(),
            line,
            end,
        }
    }

    /// The extraction golden: fences skipped; URLs, times and versions
    /// skipped; every suffix form; a git-ref prefix taken off.
    #[test]
    fn extract_reads_cites_outside_fences_and_urls() {
        let reply = "See `src/chat.rs:12` and src/lib.rs:40-52, also docs/a.md#L12-L20.\n\
            Column form src/x.ts:7:3 and [link](/abs/p/File.tsx#L9).\n\
            Ref forms origin/main:src/y.rs:5, HEAD:src/z.rs:6 and 1a2b3c4d:src/w.rs:8.\n\
            Not cites: https://example.com/a.rs:12, http://localhost.dev:3000/b.md#L3,\n\
            at 10:30, v1.2:3, the a.rs:12abc and line 0 a.rs:0.\n\
            Bare name Cargo.toml:3 and README.md#L4. Again src/chat.rs:12.\n\
            ```rust\n\
            in/fence.rs:1\n\
            ```\n\
            ~~~~\n\
            in/tilde.rs:2\n\
            ~~~~\n\
            after/fence.rs:99";
        assert_eq!(
            extract(reply),
            vec![
                cite("src/chat.rs", 12, 12),
                cite("src/lib.rs", 40, 52),
                cite("docs/a.md", 12, 20),
                cite("src/x.ts", 7, 7),
                cite("/abs/p/File.tsx", 9, 9),
                cite("src/y.rs", 5, 5),
                cite("src/z.rs", 6, 6),
                cite("src/w.rs", 8, 8),
                cite("Cargo.toml", 3, 3),
                cite("README.md", 4, 4),
                cite("after/fence.rs", 99, 99),
            ]
        );
        let many: String = (1..=80).map(|n| format!("a/b.rs:{n} ")).collect();
        assert_eq!(extract(&many).len(), MAX_CITES);
    }

    #[test]
    fn shell_reads_and_grep_hits_become_windows() {
        let codex = json!({ "cmd": ["bash", "-lc", "sed -n '10,40p' src/a.rs"] });
        assert_eq!(
            read_windows("exec_command", &codex),
            vec![Named {
                path: "src/a.rs".into(),
                base: Base::Cwd,
                window: Some((10, 40)),
            }]
        );
        let read = json!({ "file_path": "/p/a.rs", "offset": 100, "limit": 50 });
        assert_eq!(read_windows("Read", &read)[0].window, Some((100, 149)));
        assert_eq!(
            read_windows("Read", &json!({ "file_path": "/p/a.rs" }))[0].window,
            Some((1, 2000))
        );
        let rg = json!({ "command": "rg -n check_outcome src" });
        assert_eq!(
            grep_hits(
                "Bash",
                &rg,
                "src/evidence.rs:1041:    pub fn check_outcome(\nsrc/x.rs-12-context line"
            ),
            [("src/evidence.rs".to_string(), 1041)]
        );
        // Without line numbers, or from another program, nothing.
        assert!(grep_hits("Bash", &json!({ "command": "rg check src" }), "a.rs:1:x").is_empty());
        assert!(grep_hits("Bash", &json!({ "command": "cargo test -n" }), "a.rs:1:x").is_empty());
        assert_eq!(
            grep_hits("Grep", &json!({}), "/p/a.rs:7:x"),
            [("/p/a.rs".into(), 7)]
        );
    }

    fn numbered(lines: usize) -> String {
        (1..=lines).map(|n| format!("line {n}\n")).collect()
    }

    #[test]
    fn receipt_keeps_each_resolved_target_after_restricted_filtering() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let cwd = v.join("repo");
        fs::create_dir_all(cwd.join("notes")).unwrap();
        fs::create_dir_all(v.join("notes")).unwrap();
        fs::create_dir_all(cwd.join("restricted")).unwrap();
        fs::write(cwd.join("notes/same.md"), numbered(60)).unwrap();
        fs::write(v.join("notes/same.md"), numbered(60)).unwrap();
        fs::write(v.join("notes/fallback.md"), numbered(60)).unwrap();
        fs::write(cwd.join("restricted/secret.md"), numbered(60)).unwrap();
        let ctx = Ctx {
            cwd: Some(&cwd),
            vault: v,
            reads: vec![],
            earlier: vec![],
            delegated: false,
            unobserved: false,
        };
        let cites = extract(&format!(
            "notes/same.md:42 notes/fallback.md:43 {}:44 notes/missing.md:2 restricted/secret.md:1",
            v.join("notes/same.md").display()
        ));
        let r = check(&cites, &ctx).unwrap();
        let items = serde_json::to_value(&r).unwrap()["items"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(items.len(), 4, "restricted paths remain absent");
        for (line, target) in [
            (42, cwd.join("notes/same.md")),
            (43, v.join("notes/fallback.md")),
            (44, v.join("notes/same.md")),
        ] {
            let item = items.iter().find(|i| i["line"] == line).unwrap();
            assert_eq!(item["target"], key_of(&target).to_string_lossy().as_ref());
        }
        assert!(items
            .iter()
            .find(|i| i["line"] == 2)
            .unwrap()
            .get("target")
            .is_none());
    }

    fn status_of(r: &ReceiptCites, path: &str) -> String {
        r.items
            .iter()
            .find(|i| i.path == path)
            .map(|i| i.status.clone())
            .unwrap_or_else(|| "seen or unlisted".into())
    }

    #[test]
    #[cfg(unix)]
    fn a_cite_symlink_outside_the_vault_never_lists_a_restricted_target() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let restricted_dir = dir.path().join("outside/restricted");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&restricted_dir).unwrap();
        fs::write(restricted_dir.join("private.md"), numbered(4)).unwrap();
        std::os::unix::fs::symlink(restricted_dir.join("private.md"), vault.join("linked.md"))
            .unwrap();
        let ctx = Ctx {
            cwd: Some(&vault),
            vault: &vault,
            reads: vec![],
            earlier: vec![],
            delegated: false,
            unobserved: false,
        };
        let receipt = check(&extract("./linked.md:2"), &ctx).unwrap();
        assert_eq!(receipt.checked, 1);
        assert_eq!(receipt.not_opened, 1);
        assert!(
            receipt.items.is_empty(),
            "a restricted real target must never become a clickable cite"
        );
    }

    /// Each status: a windowed Read gives `fileOnly`; a Codex `sed -n` read
    /// and an `rg -n` hit give `seen`; a file never opened is `notOpened`,
    /// or `unknown` once a delegation ran; past the end, missing, an
    /// ambiguous bare name; a restricted path counted but never listed.
    #[test]
    fn statuses_follow_the_runs_reads() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let cwd = v.join("repo");
        for (rel, n) in [
            ("repo/src/a.rs", 300),
            ("repo/src/b.rs", 100),
            ("repo/src/c.rs", 50),
            ("repo/src/d.rs", 20),
            ("repo/lib/mod.rs", 5),
            ("repo/tests/mod.rs", 5),
            ("repo/restricted/secret.md", 10),
            ("notes/n.md", 3),
        ] {
            fs::create_dir_all(v.join(rel).parent().unwrap()).unwrap();
            fs::write(v.join(rel), numbered(n)).unwrap();
        }
        let a = v.join("repo/src/a.rs").to_string_lossy().to_string();
        let mut reads = read_windows("Read", &json!({ "file_path": a, "offset": 1, "limit": 50 }));
        reads.extend(read_windows(
            "exec_command",
            &json!({ "cmd": "bash -lc \"sed -n '10,40p' src/b.rs\"" }),
        ));
        reads.push(hit_window("src/c.rs".into(), 30));
        reads.extend(read_windows("Read", &json!({ "file_path": "lib/mod.rs" })));
        reads.extend(read_windows(
            "Read",
            &json!({ "file_path": "tests/mod.rs" }),
        ));
        let cites = extract(
            "src/a.rs:10 src/a.rs:200 src/b.rs:35 src/c.rs:33 src/c.rs:40 \
             src/d.rs:5 src/d.rs:21 src/gone.rs:1 mod.rs:2 restricted/secret.md:1 \
             notes/n.md:2",
        );
        let mut ctx = Ctx {
            cwd: Some(&cwd),
            vault: v,
            reads,
            earlier: read_windows(
                crate::run_manifest::READ_SOURCE_TOOL,
                &json!({ "path": "notes/n.md" }),
            ),
            delegated: false,
            unobserved: false,
        };
        let r = check(&cites, &ctx).unwrap();
        assert_eq!(r.checked, 11);
        assert_eq!(r.seen, 3, "{r:?}"); // a:10, b:35, c:33
        assert_eq!(r.file_only, 2, "{r:?}"); // a:200, c:40
        assert_eq!(status_of(&r, "src/a.rs"), "fileOnly");
        assert_eq!(r.earlier, 1);
        assert_eq!(status_of(&r, "notes/n.md"), "earlier");
        assert_eq!(r.not_opened, 2, "{r:?}"); // d:5 and the restricted note
        assert_eq!(status_of(&r, "src/d.rs"), "notOpened");
        assert_eq!(r.past_end, 1);
        assert!(r
            .items
            .iter()
            .any(|i| i.path == "src/d.rs" && i.line == 21 && i.status == "pastEnd"));
        assert_eq!(r.missing, 1);
        assert_eq!(status_of(&r, "src/gone.rs"), "missing");
        assert_eq!(r.unresolved, 1);
        assert_eq!(status_of(&r, "mod.rs"), "unresolved");
        assert!(
            !r.items.iter().any(|i| i.path.contains("restricted")),
            "{r:?}"
        );
        assert_eq!(r.items.len(), 7, "{r:?}");

        // A delegation: not opened becomes unknown (so does an agent that
        // reports no tool calls).
        ctx.delegated = true;
        let r = check(&cites, &ctx).unwrap();
        assert_eq!((r.not_opened, r.unknown), (0, 2));
        assert_eq!(status_of(&r, "src/d.rs"), "unknown");
        ctx.delegated = false;
        ctx.unobserved = true;
        assert_eq!(check(&cites, &ctx).unwrap().unknown, 2);

        // No cites, no record.
        assert!(check(&[], &ctx).is_none());
    }

    /// R5 review 1 and 2: Grok's `read_file` and lowercase `grep` (a JSON
    /// result) and its shell tool count; a grep of one file prints no path,
    /// so its bare lines are that file's.
    #[test]
    fn grok_tools_and_single_file_greps_are_reads() {
        assert_eq!(
            read_windows("read_file", &json!({ "target_file": "/p/a.rs" }))[0].window,
            None
        );
        assert_eq!(
            read_windows(
                "read_file",
                &json!({ "target_file": "/p/a.rs", "offset": 10, "limit": 5 })
            )[0]
            .window,
            Some((10, 14))
        );
        assert_eq!(
            read_windows(
                "run_terminal_command",
                &json!({ "command": "sed -n '1,5p' a.rs" })
            ),
            [Named {
                path: "a.rs".into(),
                base: Base::Cwd,
                window: Some((1, 5)),
            }]
        );
        let hits = |name: &str, input: Value, result: &str| grep_hits(name, &input, result);
        let p = |path: &str, n: u32| (path.to_string(), n);
        assert_eq!(
            hits(
                "grep",
                json!({ "pattern": "fn", "path": "src/a.rs" }),
                "{\"type\":\"Grep\",\"content\":\"12:fn x\\n13-ctx\"}"
            ),
            [p("src/a.rs", 12), p("src/a.rs", 13)]
        );
        assert_eq!(
            hits(
                "Grep",
                json!({ "path": "/p/a.rs", "output_mode": "content" }),
                "5:x"
            ),
            [p("/p/a.rs", 5)]
        );
        // Text holding `:34:` is still the one file's line 40.
        for command in [
            "grep -n foo src/a.rs",
            "rg -n foo src/a.rs",
            "git grep -n -e foo src/a.rs",
        ] {
            assert_eq!(
                hits("Bash", json!({ "command": command }), "12:text\n40:x:34:y"),
                [p("src/a.rs", 12), p("src/a.rs", 40)],
                "{command}"
            );
        }
        // Two files: their lines carry paths; a bare line is no one's.
        assert_eq!(
            hits(
                "Bash",
                json!({ "command": "rg -n foo src/a.rs src/b.rs" }),
                "src/b.rs:3:x\n9:y"
            ),
            [p("src/b.rs", 3)]
        );
        assert_eq!(
            hits(
                "Bash",
                json!({ "command": "grep -rn foo notes" }),
                "notes/2026-10-18.md:3:x"
            ),
            [p("notes/2026-10-18.md", 3)]
        );
    }

    /// R5 review 3: a leading `cd DIR &&` / `cd DIR;` and Codex's `workdir`
    /// move the folder a shell call's relative paths are read from.
    #[test]
    fn a_leading_cd_and_codex_workdir_set_the_shell_base() {
        let paths = |name: &str, input: Value| -> Vec<String> {
            read_windows(name, &input)
                .into_iter()
                .map(|r| r.path)
                .collect()
        };
        assert_eq!(
            paths("Bash", json!({ "command": "cd sub && sed -n '1,5p' a.rs" })),
            ["sub/a.rs"]
        );
        assert_eq!(
            paths("Bash", json!({ "command": "cd /abs; cat b.rs /c/d.rs" })),
            ["/abs/b.rs", "/c/d.rs"]
        );
        assert_eq!(
            paths(
                "exec_command",
                json!({ "cmd": "cat c.rs", "workdir": "/w" })
            ),
            ["/w/c.rs"]
        );
        assert_eq!(
            paths(
                "exec_command",
                json!({ "cmd": "cd x && cd y && cat d.rs", "workdir": "/w" })
            ),
            ["/w/x/y/d.rs"]
        );
        assert_eq!(
            paths("Bash", json!({ "command": "cd ~ && cat e.rs" })),
            ["~/e.rs"]
        );
        // Only a leading `cd` counts.
        assert_eq!(
            paths("Bash", json!({ "command": "cat e.rs && cd z && cat f.rs" })),
            ["e.rs", "f.rs"]
        );
        assert_eq!(
            grep_hits(
                "Bash",
                &json!({ "command": "cd sub && grep -n foo a.rs" }),
                "3:x"
            ),
            [("sub/a.rs".to_string(), 3)]
        );

        // End to end: the read resolves under the run's folder.
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join("sub")).unwrap();
        fs::write(cwd.join("sub/a.rs"), numbered(10)).unwrap();
        let ctx = Ctx {
            cwd: Some(&cwd),
            vault: dir.path(),
            reads: read_windows(
                "Bash",
                &json!({ "command": "cd sub && sed -n '1,5p' a.rs" }),
            ),
            earlier: Vec::new(),
            delegated: false,
            unobserved: false,
        };
        let r = check(&extract("sub/a.rs:3 and sub/a.rs:8"), &ctx).unwrap();
        assert_eq!((r.seen, r.file_only), (1, 1), "{r:?}");
    }

    /// R5 delta review: `cd -` goes back to the folder before the last `cd`
    /// (the run's folder here), a bare `cd` to `~`.
    #[test]
    fn cd_dash_goes_back_and_a_bare_cd_goes_home() {
        let paths = |name: &str, input: Value| -> Vec<String> {
            read_windows(name, &input)
                .into_iter()
                .map(|r| r.path)
                .collect()
        };
        assert_eq!(
            paths("Bash", json!({ "command": "cd sub && cd - && cat a.rs" })),
            ["a.rs"]
        );
        assert_eq!(
            paths(
                "exec_command",
                json!({ "cmd": "cd x && cd y && cd - && cat a.rs", "workdir": "/w" })
            ),
            ["/w/x/a.rs"]
        );
        assert_eq!(
            paths("Bash", json!({ "command": "cd sub; cd; cat a.rs" })),
            ["~/a.rs"]
        );
        // `cd -` with no `cd` before it: the walk stops where it was.
        assert_eq!(
            paths(
                "exec_command",
                json!({ "cmd": "cd - && cat a.rs", "workdir": "/w" })
            ),
            ["/w/a.rs"]
        );

        // End to end: `cd sub && cd - && cat a.rs` read the run folder's a.rs.
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join("sub")).unwrap();
        fs::write(cwd.join("a.rs"), numbered(10)).unwrap();
        fs::write(cwd.join("sub/a.rs"), numbered(10)).unwrap();
        let ctx = Ctx {
            cwd: Some(&cwd),
            vault: dir.path(),
            reads: read_windows("Bash", &json!({ "command": "cd sub && cd - && cat a.rs" })),
            earlier: Vec::new(),
            delegated: false,
            unobserved: false,
        };
        let r = check(&extract("a.rs:3 and sub/a.rs:3"), &ctx).unwrap();
        assert_eq!((r.seen, r.not_opened), (1, 1), "{r:?}");
        assert_eq!(status_of(&r, "sub/a.rs"), "notOpened");
    }

    /// R5 delta review: Claude's `Grep` prints line numbers only in its
    /// `content` mode with `-n`; a numbered grep reading a pipe numbers the
    /// stream, so it counts against no file.
    #[test]
    fn unnumbered_greps_and_piped_numbered_greps_give_no_hits() {
        let hits = |name: &str, input: Value, result: &str| grep_hits(name, &input, result);
        let p = |path: &str, n: u32| (path.to_string(), n);
        assert!(hits(
            "Grep",
            json!({ "path": "/p/a.rs", "output_mode": "content", "-n": false }),
            "12:30 meeting\n/p/b.rs:4:x"
        )
        .is_empty());
        assert!(hits("Grep", json!({ "path": "/p/a.rs" }), "5").is_empty());
        assert!(hits(
            "Grep",
            json!({ "path": "/p/a.rs", "output_mode": "count" }),
            "5:x"
        )
        .is_empty());
        assert_eq!(
            hits(
                "Grep",
                json!({ "path": "/p/a.rs", "output_mode": "content", "-n": true }),
                "5:x"
            ),
            [p("/p/a.rs", 5)]
        );
        // `grep -n y` numbers the first grep's output: line 1 isn't a.rs's.
        for command in [
            "grep -n x a.rs | grep -n y",
            "rg -n x a.rs | rg -n y",
            "cat b.rs | grep -n y; grep -n x a.rs",
        ] {
            assert!(
                hits("Bash", json!({ "command": command }), "1:12:x y\n3:z").is_empty(),
                "{command}"
            );
        }
        // An unnumbered filter keeps the first grep's numbers.
        assert_eq!(
            hits(
                "Bash",
                json!({ "command": "grep -n x a.rs | grep -v test" }),
                "12:x"
            ),
            [p("a.rs", 12)]
        );
        // Second delta review: a numbered grep piped into a filter keeps its
        // hits, so the pipe rule can't widen unnoticed.
        assert_eq!(
            hits(
                "Bash",
                json!({ "command": "rg -n foo src | head -50" }),
                "src/a.rs:3:x"
            ),
            [p("src/a.rs", 3)]
        );
        assert_eq!(
            hits(
                "Bash",
                json!({ "command": "grep -rn x . | head" }),
                "./src/a.rs:7:x"
            ),
            [p("./src/a.rs", 7)]
        );
        // `git grep` ignores stdin: a pipe into it still searches files.
        assert_eq!(
            hits(
                "Bash",
                json!({ "command": "echo | git grep -n foo" }),
                "src/a.rs:3:x"
            ),
            [p("src/a.rs", 3)]
        );
    }

    /// Second delta review: `cd` with an option (`-P`, `-L`, `--`) stops the
    /// walk, so its word is never taken as a folder.
    #[test]
    fn cd_with_an_option_stops_the_walk() {
        for command in [
            "cd -P && cat a.rs",
            "cd -- && cat a.rs",
            "cd -L sub && cat a.rs",
        ] {
            let paths: Vec<String> =
                read_windows("exec_command", &json!({ "cmd": command, "workdir": "/w" }))
                    .into_iter()
                    .map(|r| r.path)
                    .collect();
            assert_eq!(paths, ["/w/a.rs"], "{command}");
        }
    }

    /// R5 delta review: a result's lines are walked lazily, so a long grep
    /// result stops at [`MAX_HITS`] without copying the rest.
    #[test]
    fn result_lines_stop_when_told() {
        let mut seen = Vec::new();
        each_result_line("a\nb\nc\nd", |line| {
            seen.push(line.to_string());
            seen.len() < 2
        });
        assert_eq!(seen, ["a", "b"]);
        seen.clear();
        each_result_line("{\"x\":[\"a\\nb\",\"c\"],\"y\":1}", |line| {
            seen.push(line.to_string());
            true
        });
        assert_eq!(seen, ["a", "b", "c"]);
        // A JSON result stops early too: inside a string, then across items.
        seen.clear();
        each_result_line("[\"a\\nb\\nc\",\"d\"]", |line| {
            seen.push(line.to_string());
            seen.len() < 2
        });
        assert_eq!(seen, ["a", "b"]);
        seen.clear();
        each_result_line("{\"x\":[\"a\",\"b\",\"c\"]}", |line| {
            seen.push(line.to_string());
            seen.len() < 2
        });
        assert_eq!(seen, ["a", "b"]);
        // Not JSON after all: read as text.
        seen.clear();
        each_result_line("[1] a.rs:3:x", |line| {
            seen.push(line.to_string());
            true
        });
        assert_eq!(seen, ["[1] a.rs:3:x"]);
        let long: String = (1..=1000).map(|n| format!("a.rs:{n}:x\n")).collect();
        let got = grep_hits("Bash", &json!({ "command": "grep -n x a.rs b.rs" }), &long);
        assert_eq!(got.len(), MAX_HITS);
    }

    /// R5 review nits: a cite outside the folder and the vault is listed as
    /// the reply wrote it (`~` kept); a file over 16 MiB, a binary one or one
    /// past the read budget gets no range check, so never `pastEnd`.
    #[test]
    fn outside_paths_stay_as_written_and_big_or_binary_files_skip_the_range() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("repo");
        fs::create_dir_all(cwd.join("data")).unwrap();
        fs::create_dir_all(outside.path().join("sub")).unwrap();
        fs::write(outside.path().join("x.rs"), numbered(5)).unwrap();
        fs::write(cwd.join("data/bin.dat"), b"a\0b\nc\n").unwrap();
        let big = fs::File::create(cwd.join("data/big.log")).unwrap();
        big.set_len(FILE_MAX + 1).unwrap();
        let written = format!("{}/sub/../x.rs", outside.path().display());
        let ctx = Ctx {
            cwd: Some(&cwd),
            vault: dir.path(),
            reads: Vec::new(),
            earlier: Vec::new(),
            delegated: false,
            unobserved: false,
        };
        let reply = format!(
            "{written}:3, ~/no-such-folder-r5/a.rs:2, data/bin.dat:999 and data/big.log:999999"
        );
        let r = check(&extract(&reply), &ctx).unwrap();
        assert_eq!((r.not_opened, r.missing, r.past_end), (3, 1, 0), "{r:?}");
        let listed: Vec<&str> = r.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(
            listed,
            [
                written.as_str(),
                "~/no-such-folder-r5/a.rs",
                "data/bin.dat",
                "data/big.log"
            ]
        );
        // The budget: a file bigger than what's left isn't counted.
        let small = outside.path().join("x.rs");
        assert_eq!(count_lines(&small, &mut 1_000), Some(5));
        assert_eq!(count_lines(&small, &mut 3), None);
        assert_eq!(count_lines(&cwd.join("data/bin.dat"), &mut 1_000), None);
        let mut budget = TOTAL_MAX;
        assert_eq!(count_lines(&cwd.join("data/big.log"), &mut budget), None);
    }

    /// RF-788, red first: a file that grows between its size check and the
    /// read gets no count (so never a confident `pastEnd`), and the read
    /// stops at what the budget has left, which is all it charges.
    #[test]
    fn a_file_growing_while_counted_gets_no_count_and_keeps_the_budget() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("grow.log");
        fs::write(&file, "x\n".repeat(50)).unwrap();
        let grow = || {
            let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
            f.write_all(&[b'y'; 5 * 1024]).unwrap();
        };
        let mut budget = 1_000;
        assert_eq!(count_lines_with(&file, &mut budget, &grow), None);
        assert_eq!(budget, 0, "charged what it read, capped at what was left");
        // Grown again, now within a larger budget: still no count, and it
        // charges the bytes it actually read.
        let mut budget = 100_000;
        assert_eq!(count_lines_with(&file, &mut budget, &grow), None);
        assert_eq!(budget, 100_000 - (100 + 10 * 1024));
        // An unchanged file still counts and is charged its size.
        let still = dir.path().join("still.log");
        fs::write(&still, "x\n".repeat(50)).unwrap();
        let mut budget = 1_000;
        assert_eq!(count_lines_with(&still, &mut budget, &|| {}), Some(50));
        assert_eq!(budget, 900);
    }

    /// RF-788: `resolve_cite` only passes regular files, but a path swapped
    /// for a FIFO since (before the stat, or between the stat and the open)
    /// would block `open` until a writer came: it gets no count, uncharged.
    #[cfg(unix)]
    #[test]
    fn a_path_swapped_for_a_fifo_is_not_waited_on() {
        use std::os::unix::fs::OpenOptionsExt as _;
        use std::time::Duration;
        fn make_fifo(path: &Path) {
            let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
            // SAFETY: a valid NUL-terminated path; mkfifo does not retain it.
            assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        }
        fn count_guarded(path: &Path, swap_after_stat: bool) -> (Option<u32>, u64) {
            let (tx, rx) = std::sync::mpsc::channel();
            let counted = path.to_path_buf();
            std::thread::spawn(move || {
                let swap = || {
                    if swap_after_stat {
                        fs::remove_file(&counted).unwrap();
                        make_fifo(&counted);
                    }
                };
                let mut budget = 1_000;
                let lines = count_lines_with(&counted, &mut budget, &swap);
                tx.send((lines, budget)).unwrap();
            });
            rx.recv_timeout(Duration::from_secs(5))
                .or_else(|_| {
                    // Stuck in `open`: a writer releases it (a non-blocking
                    // open fails harmlessly when nothing is reading) and
                    // feeds it the swapped file's bytes, so a count that
                    // waited reads them and fails below instead of hanging.
                    if let Ok(mut writer) = fs::OpenOptions::new()
                        .write(true)
                        .custom_flags(libc::O_NONBLOCK)
                        .open(path)
                    {
                        let _ = std::io::Write::write_all(&mut writer, b"x\n");
                    }
                    rx.recv_timeout(Duration::from_secs(60))
                })
                .unwrap()
        }
        let dir = tempfile::tempdir().unwrap();
        let pipe = dir.path().join("pipe.log");
        make_fifo(&pipe);
        assert_eq!(count_guarded(&pipe, false), (None, 1_000));
        let swapped = dir.path().join("swapped.log");
        fs::write(&swapped, "x\n").unwrap();
        assert_eq!(count_guarded(&swapped, true), (None, 1_000));
    }

    /// R14a review, red first: a PDF's raw bytes (31 lines in the fixture)
    /// are not what a cite into it counts; its extracted text's lines are,
    /// and only once Hollow has read it (never parsed here).
    #[test]
    fn a_document_counts_its_extracted_lines_or_is_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("terms.pdf");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/doc_text_one_page.pdf"),
            &pdf,
        )
        .unwrap();
        assert_eq!(count_lines(&pdf, &mut 1_000_000), None, "not read yet");
        let docx = dir.path().join("plan.docx");
        fs::write(
            &docx,
            crate::doc_text::tests::docx_with(&["one", "two", "three"]),
        )
        .unwrap();
        assert_eq!(count_lines(&docx, &mut 1_000_000), None);
        crate::doc_text::cached(&docx).unwrap();
        assert_eq!(count_lines(&docx, &mut 1_000_000), Some(3));
        // Cached under its canonical path, cited through a link.
        #[cfg(unix)]
        {
            let real = dir.path().canonicalize().unwrap().join("linked.docx");
            fs::write(&real, crate::doc_text::tests::docx_with(&["a", "b"])).unwrap();
            crate::doc_text::cached(&real).unwrap();
            let alias = dir.path().join("alias");
            std::os::unix::fs::symlink(dir.path().canonicalize().unwrap(), &alias).unwrap();
            assert_eq!(
                count_lines(&alias.join("linked.docx"), &mut 1_000_000),
                Some(2)
            );
        }
        #[cfg(target_os = "macos")]
        {
            let text = crate::doc_text::cached(&pdf).unwrap();
            assert_eq!(
                count_lines(&pdf, &mut 1_000_000),
                Some(text.lines().count() as u32)
            );
            assert!(text.lines().count() < 10, "{text}");
        }
    }
}
