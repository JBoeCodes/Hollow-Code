//! The overnight summary card (overnight C8): what happened last night, for
//! a morning card on Home. Read-only.
//!
//! **The night** is the overnight window the budgets use
//! ([`crate::budgets::OVERNIGHT_START_HOUR`] local yesterday evening to
//! [`crate::budgets::OVERNIGHT_END_HOUR`] this morning, or now if earlier),
//! every day of the week. It covers the night queue's `tonight` window and
//! the overnight chats around it. All day it reports the night that ended
//! this morning; the next night is tomorrow morning's card.
//!
//! **Chains run past the night** (S06 follow-up): a lane that started at
//! 11 PM and handed over until 11 AM is the night's work, all of it. So
//! after the night ends, what the chats of the handover chains that
//! *started* in the night did (their first chat's first turn was in the
//! night; every chat they handed along joins, whenever it started) still
//! counts, until now but never past the next night's start: their
//! receipts, PRs, usage rows and stints. Nothing else after the night
//! does. `to` is the last such thing's end (now while a row still runs),
//! at least the night's end. A chain that started before the night counts
//! only inside it. The action's `from` / `to` set an exact window instead
//! (everything in it, no chain followed past it).
//!
//! **What it reads**, all local and all already recorded:
//! - **Runs:** evidence receipts that started in the night. `failed` lists
//!   those whose status is neither `success` nor `cancelled`, or whose last
//!   run of some check failed (a check that failed and then passed doesn't
//!   count), newest first, with their conversation's title.
//! - **Pull requests:** the night's shell tool calls (`toolUse` events,
//!   paired with their `toolResult` by conversation, turn and id) running
//!   `gh pr merge <n>` or `gh pr create` as a command of their own (also
//!   inside `bash -c '…'`; a here-document's body is skipped, the commands
//!   after it are read), counted only
//!   when the result is not an error. Skipped: `--auto` (queued, not merged)
//!   and background calls (their result says nothing). A created PR's number
//!   comes from the URL `gh` printed; a merge with no repository takes the
//!   one its PR was opened in, else the one its chat's receipt recorded for
//!   that PR (`evidence::fill_pr_repos`). This is what the agents did, not GitHub's
//!   view: a merge done by hand on github.com isn't here.
//! - **Commits:** receipts whose repository head moved, one row per
//!   repository (first head before → last head after, and how many runs).
//!   A head move can also be a checkout or a pull.
//! - **Waiting:** `needs_you` now, without the approval cards on screen (the
//!   window's own; Home's card shows the live count instead).
//! - **Report:** the night queue's morning report for that evening
//!   (`workspaces/main/outputs/night/<evening>.md`), else the newest written
//!   since the night began.
//! - **Chains and cost (S06):** the usage ledger's rows that finished in the
//!   night, each by its run's chat (`turns.run_id`), grouped by handover
//!   chain: a chat a `handover` delegation started joins its predecessor's
//!   chain, followed back to the first chat (a chat that never handed over
//!   is a chain of its own). A row's cost covers the main thread and its
//!   subagents; consultations and helpers share the turn's run id, so they
//!   land in the same chain. A child chat a turn *started* is its own chain,
//!   so each dollar is counted once (not `descendants::rollup`, which folds
//!   a child chat's cost into its parent's). As in the usage ledger's
//!   buckets (`usage_ledger::add_row`), a row still running counts as in
//!   progress and a row whose agent never started is left out; any other
//!   NULL-cost row adds nothing and its run counts as unknown, once however
//!   many such rows it has (`unknownCost`), so a cost with unknowns is a
//!   lower bound. A NULL-cost row with no run id is not a run (mostly Jev
//!   ranking requests, several per turn): it counts in `unknownCostCalls`
//!   instead. Peak context is the highest `peakContextTokens` of the
//!   chain's runs, from their rows' `usage_json` or their receipts'
//!   `apiUsage` (absent on runs before OB05). A cost adds every row that carries one,
//!   finite and positive, whatever its status: reported and estimated
//!   together, and partial (killed) runs too, so it can exceed Usage's
//!   reported/estimated figure; a Codex `unpriced` row (NULL) is unknown.
//!   Predecessors come from the `handover` delegation records, read once
//!   per summary; a chat whose record is missing (a run records at most
//!   `delegation::MAX_RECORDED_PER_RUN`) falls back to the handover
//!   registry (`app_sessions::handover_pairs`, its newest 500, and only
//!   handovers recorded with their `from`). A chain's label is its first
//!   chat, which may have run before the night. Stints are the `stints`
//!   rows of the chain's chats that started in the night.
//!   $/merged PR uses the PR heuristic above, so it is only as good as that.
//!
//! `ran` is false when nothing started in the night; the card stays hidden.

use chrono::{DateTime, Duration, Local, NaiveTime, SecondsFormat, TimeZone, Timelike, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

/// Rows kept per list.
const LIST_CAP: usize = 20;
/// Handovers followed back from a chat to its chain's first chat, at most.
const CHAIN_HOPS: usize = 64;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    /// `owner/repo` when the command or the URL named one.
    pub repo: Option<String>,
    pub number: u64,
    /// `merged` | `opened`.
    pub action: String,
    pub at: String,
    pub session_key: String,
    pub session_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub repo: String,
    pub from: String,
    pub to: String,
    /// Runs that moved this repository's head.
    pub runs: usize,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailedRun {
    pub status: String,
    pub result_kind: Option<String>,
    pub failed_checks: usize,
    pub at: String,
    pub session_key: Option<String>,
    pub session_title: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub total: usize,
    pub titles: Vec<String>,
}

/// One handover chain's night (S06): the chats one lane handed along.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Chain {
    /// The chain's first chat: its label (by title, else this key). It may
    /// be a predecessor that ran before the night and has no row in it.
    pub session_key: String,
    pub session_title: Option<String>,
    /// The chain's chats with usage or a PR in the night.
    pub chats: usize,
    /// Their turns (runs) with a usage row in the night.
    pub turns: usize,
    /// Their stints (the `stints` table) that started in the night.
    pub stints: usize,
    pub merged: usize,
    pub opened: usize,
    /// Known cost, reported and estimated added together: main threads,
    /// subagents, consultations and helpers.
    pub cost_usd: f64,
    /// Runs with a finished NULL-cost usage row, each once (a turn's chat
    /// row and its consultation's are one run); `costUsd` is a lower bound
    /// when > 0.
    pub unknown_cost: usize,
    /// Usage rows still running (no cost yet; not unknown).
    pub in_progress: usize,
    /// The highest `peakContextTokens` (OB05) of its runs: their usage rows'
    /// `usage_json` or their receipts' `apiUsage`. Null when none recorded
    /// one (runs before OB05's build, Codex, Antigravity).
    pub peak_context_tokens: Option<u64>,
    /// `costUsd / merged`; null with no merged PR.
    pub usd_per_merged_pr: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// The evening's local date (`YYYY-MM-DD`): the dismissal key.
    pub night: String,
    pub from: String,
    pub to: String,
    pub ran: bool,
    pub runs: usize,
    pub conversations: usize,
    pub succeeded: usize,
    pub failed: Vec<FailedRun>,
    pub pull_requests: Vec<PullRequest>,
    pub commits: Vec<Commit>,
    pub waiting: Waiting,
    /// Vault-relative morning report, when the night queue wrote one.
    pub report: Option<String>,
    /// The night's handover chains (S06), costliest first.
    pub chains: Vec<Chain>,
    /// Known cost of every usage row of the night, in a chat or not
    /// (any row carrying a cost, partial runs included; see the module doc).
    pub cost_usd: f64,
    /// Runs of the night with a finished NULL-cost usage row, each once.
    pub unknown_cost: usize,
    /// Finished NULL-cost usage rows with no run id (helpers outside a turn,
    /// mostly Jev ranking requests, which report no cost): not runs, so not
    /// in `unknownCost`.
    pub unknown_cost_calls: usize,
    /// Usage rows of the night still running.
    pub in_progress: usize,
    /// PRs merged in the night (all of them, not only the listed 20).
    pub merged_prs: usize,
    /// `costUsd / mergedPrs`; null with no merged PR.
    pub usd_per_merged_pr: Option<f64>,
}

/// The night to report at `now`: `(evening date, start, end)`.
pub fn night_at(now: DateTime<Local>) -> (chrono::NaiveDate, DateTime<Utc>, DateTime<Utc>) {
    let start_t = NaiveTime::from_hms_opt(crate::budgets::OVERNIGHT_START_HOUR, 0, 0).unwrap();
    let end_t = NaiveTime::from_hms_opt(crate::budgets::OVERNIGHT_END_HOUR, 0, 0).unwrap();
    let evening = now.date_naive() - Duration::days(1);
    let local = |d: chrono::NaiveDate, t: NaiveTime| {
        Local
            .from_local_datetime(&d.and_time(t))
            .earliest()
            .unwrap_or(now)
            .with_timezone(&Utc)
    };
    let start = local(evening, start_t);
    let end = local(evening + Duration::days(1), end_t).min(now.with_timezone(&Utc));
    (evening, start, end)
}

fn parse(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// Split a shell command into simple commands (at `;`, `&` and `|` outside
/// quotes), each a list of words with quotes removed. A here-document's
/// body (`cat > f <<'EOF'` … `EOF`) is text, not commands: it is dropped
/// through its closing line and reading goes on after it (a `gh pr create`
/// after the body that `cat` wrote is a command). One still open at the end
/// drops the rest. Only an unquoted `<<` opens one (not a `<<<`
/// here-string, nor one in a comment), so a quoted script (`bash -c '…'`)
/// keeps its here-documents whole for its own pass; the exception is a
/// command substitution inside double quotes (`-m "$(cat <<'EOF'` … `EOF`
/// `)"`, how agents write commit and PR bodies), whose body is dropped too,
/// so a lone `"` or `'` in it can't swallow the commands after it; there the
/// `)` may follow the delimiter on its closing line (`EOF)"`), and `$((`
/// arithmetic or text in `'…'` inside it opens nothing. Otherwise the
/// closing line must be the delimiter exactly (`<<-` strips leading tabs
/// only). A `#` starting a command, or followed by a space, starts a
/// comment; `gh pr merge #451` keeps its number on purpose (bash would read
/// a comment there, but an argv array joined by `shell_command` has none).
pub(crate) fn simple_commands(command: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    let mut out = vec![Vec::new()];
    let mut word = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    // `$(` opened inside the current double-quoted word and not yet closed
    // (not `$((`), and a `'` open inside one, whose text is literal.
    let mut comsub = 0usize;
    let mut comsub_quote = false;
    // Here-documents opened on this line, in order.
    let mut pending: Vec<Heredoc> = Vec::new();
    let end_word = |out: &mut Vec<Vec<String>>, word: &mut String, in_word: &mut bool| {
        if *in_word {
            out.last_mut().unwrap().push(std::mem::take(word));
            *in_word = false;
        }
    };
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if c == '\n'
            && !pending.is_empty()
            && (quote.is_none() || pending.iter().any(|h| h.in_comsub))
        {
            // Skip each body through its closing line; one never closed
            // takes the rest.
            let mut queue = std::mem::take(&mut pending).into_iter();
            'bodies: while let Some(h) = queue.next() {
                loop {
                    if i >= chars.len() {
                        end_word(&mut out, &mut word, &mut in_word);
                        out.retain(|w| !w.is_empty());
                        return out;
                    }
                    let end = chars[i..]
                        .iter()
                        .position(|c| *c == '\n')
                        .map_or(chars.len(), |n| i + n);
                    let line = &chars[i..end];
                    let tabs = if h.dash {
                        line.iter().take_while(|c| **c == '\t').count()
                    } else {
                        0
                    };
                    let text = &line[tabs..];
                    let text = text.strip_suffix(&['\r']).unwrap_or(text);
                    if text == h.delimiter.as_slice() {
                        i = (end + 1).min(chars.len());
                        break;
                    }
                    // Inside `$( … )` bash 5 also ends it at the delimiter
                    // and the `)` closing it (`EOF)"`, `EOF )"`): reading
                    // goes on from that `)`.
                    if h.in_comsub && text.starts_with(&h.delimiter) {
                        let after = &text[h.delimiter.len()..];
                        let blanks = after
                            .iter()
                            .take_while(|c| **c == ' ' || **c == '\t')
                            .count();
                        if after.get(blanks) == Some(&')') {
                            i += tabs + h.delimiter.len() + blanks;
                            pending = queue.by_ref().collect();
                            break 'bodies;
                        }
                    }
                    i = (end + 1).min(chars.len());
                }
            }
        }
        match (quote, c) {
            (Some(q), c) if c == q && !comsub_quote => {
                quote = None;
                comsub = 0;
                // One opened inside it that never reached a line end goes too.
                pending.retain(|h| !h.in_comsub);
            }
            // A comment inside one (`<<'EOF'  # don't indent`): its `'`
            // opens nothing.
            (Some('"'), '#')
                if comsub > 0
                    && !comsub_quote
                    && i.checked_sub(2)
                        .and_then(|p| chars.get(p))
                        .is_some_and(|p| {
                            p.is_whitespace() || matches!(p, '(' | ';' | '|' | '&')
                        }) =>
            {
                while chars.get(i).is_some_and(|n| *n != '\n') {
                    i += 1;
                }
            }
            (Some('"'), '\'') if comsub > 0 => {
                comsub_quote = !comsub_quote;
                word.push(c);
            }
            (Some('"'), c) if comsub_quote => word.push(c),
            (Some('"'), '\\') => {
                if let Some(&n) = chars.get(i) {
                    word.push(n);
                    i += 1;
                }
            }
            (Some('"'), '$') if chars.get(i) == Some(&'(') && chars.get(i + 1) != Some(&'(') => {
                comsub += 1;
                word.push(c);
            }
            (Some('"'), ')') if comsub > 0 => {
                comsub -= 1;
                word.push(c);
            }
            (Some('"'), '<') if comsub > 0 && chars.get(i) == Some(&'<') => {
                if let Some(h) = heredoc_opener(&chars, &mut i, true) {
                    pending.push(h);
                }
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                if let Some(&n) = chars.get(i) {
                    i += 1;
                    if n != '\n' {
                        word.push(n);
                        in_word = true;
                    }
                }
            }
            (None, '<') if chars.get(i) == Some(&'<') => {
                // The opener's delimiter is not a word; the body goes at the
                // line's end. A `<<<` here-string's word is an ordinary word.
                end_word(&mut out, &mut word, &mut in_word);
                if let Some(h) = heredoc_opener(&chars, &mut i, false) {
                    pending.push(h);
                }
            }
            (None, '#')
                if !in_word
                    && (out.last().unwrap().is_empty()
                        || chars.get(i).is_none_or(|n| n.is_whitespace())) =>
            {
                while chars.get(i).is_some_and(|n| *n != '\n') {
                    i += 1;
                }
            }
            (None, ';' | '&' | '|' | '\n') => {
                end_word(&mut out, &mut word, &mut in_word);
                if !out.last().unwrap().is_empty() {
                    out.push(Vec::new());
                }
            }
            (None, c) if c.is_whitespace() => end_word(&mut out, &mut word, &mut in_word),
            (None, c) => {
                word.push(c);
                in_word = true;
            }
        }
    }
    end_word(&mut out, &mut word, &mut in_word);
    out.retain(|w| !w.is_empty());
    out
}

/// A here-document waiting for its body: its delimiter (quotes removed),
/// whether it was `<<-`, and whether it opened inside a quoted `"$( … )"`.
struct Heredoc {
    delimiter: Vec<char>,
    dash: bool,
    in_comsub: bool,
}

/// A here-document opener whose first `<` is just before `chars[*i]`
/// (`<<EOF`, `<<-'EOF'`, `<< "EOF"`), read up to the end of its delimiter.
/// `None` for a `<<<` here-string (consumed) or an opener with no delimiter.
fn heredoc_opener(chars: &[char], i: &mut usize, in_comsub: bool) -> Option<Heredoc> {
    *i += 1;
    if chars.get(*i) == Some(&'<') {
        *i += 1;
        return None;
    }
    let dash = chars.get(*i) == Some(&'-');
    if dash {
        *i += 1;
    }
    while chars.get(*i).is_some_and(|c| *c == ' ' || *c == '\t') {
        *i += 1;
    }
    let mut delimiter = Vec::new();
    let mut q: Option<char> = None;
    while let Some(&d) = chars.get(*i) {
        match (q, d) {
            // Never past its line, even with a quote left open.
            (_, '\n') => break,
            (Some(open), d) if d == open => q = None,
            (Some(_), d) => delimiter.push(d),
            (None, '\'' | '"') => q = Some(d),
            (None, '\\') => {}
            (None, d) if d.is_whitespace() || matches!(d, ';' | '&' | '|' | ')' | '<' | '>') => {
                break
            }
            (None, d) => delimiter.push(d),
        }
        *i += 1;
    }
    (!delimiter.is_empty()).then_some(Heredoc {
        delimiter,
        dash,
        in_comsub,
    })
}

/// Flags of `gh pr merge` / `gh pr create` that take a value.
const VALUE_FLAGS: &[&str] = &[
    "-R",
    "--repo",
    "-t",
    "--subject",
    "--title",
    "-b",
    "--body",
    "-F",
    "--body-file",
    "--match-head-commit",
    "-A",
    "--author-email",
    "-B",
    "--base",
    "-H",
    "--head",
    "-l",
    "--label",
    "-a",
    "--assignee",
    "-r",
    "--reviewer",
    "-m",
    "--milestone",
    "-p",
    "--project",
    "-T",
    "--template",
];

/// `gh pr merge <n>` / `gh pr create` in a shell command: `(action, number,
/// repo)`; a create's number comes from its result. Only a simple command
/// of its own counts (or one inside `bash -c '…'`); the number is the first
/// positional `<n>`, `#<n>` or PR URL, skipping flag values. A merge with
/// `--auto`, or with no number, is not claimed.
pub(crate) fn pr_command(command: &str) -> Option<(&'static str, Option<u64>, Option<String>)> {
    simple_commands(command)
        .iter()
        .find_map(|words| pr_words(words))
        .flatten()
}

/// One simple command's words (quotes already removed), as `pr_command`
/// reads them: `None` when it is no `gh pr merge`/`create` (reading goes
/// on), else what it claims (`Some(None)`: an `--auto` merge or one with no
/// number). `evidence::merge_places` passes its own words here rather than
/// a re-joined line, which would be split again without its quotes.
pub(crate) fn pr_words(
    words: &[String],
) -> Option<Option<(&'static str, Option<u64>, Option<String>)>> {
    let mut w = words;
    while w
        .first()
        .is_some_and(|x| x.contains('=') && !x.starts_with('-'))
    {
        w = &w[1..];
    }
    if let [shell, flag, script, ..] = w {
        if matches!(
            shell.as_str(),
            "bash" | "sh" | "zsh" | "/bin/bash" | "/bin/zsh" | "/bin/sh"
        ) && flag.starts_with('-')
            && flag.contains('c')
        {
            return pr_command(script).map(Some);
        }
    }
    let [gh, pr, verb, args @ ..] = w else {
        return None;
    };
    if gh != "gh" || pr != "pr" || !matches!(verb.as_str(), "merge" | "create") {
        return None;
    }
    let mut repo = None;
    let mut number = None;
    let mut auto = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some((flag, value)) = a.split_once('=').filter(|(f, _)| f.starts_with('-')) {
            if matches!(flag, "-R" | "--repo") {
                repo = Some(value.to_string());
            }
        } else if VALUE_FLAGS.contains(&a) {
            if matches!(a, "-R" | "--repo") {
                repo = args.get(i + 1).cloned();
            }
            i += 1;
        } else if a == "--auto" {
            auto = true;
        } else if !a.starts_with('-') && number.is_none() {
            number = a.trim_start_matches('#').parse::<u64>().ok().or_else(|| {
                pr_url(a).map(|(r, n)| {
                    repo.get_or_insert(r);
                    n
                })
            });
        }
        i += 1;
    }
    if verb == "create" {
        return Some(Some(("opened", None, repo)));
    }
    if auto {
        return Some(None);
    }
    Some(number.map(|n| ("merged", Some(n), repo)))
}

/// The first `https://github.com/<owner>/<repo>/pull/<n>` in a text.
pub(crate) fn pr_url(text: &str) -> Option<(String, u64)> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static URL: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"https://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/pull/(\d+)").unwrap()
    });
    let c = URL.captures(text)?;
    Some((c[1].to_string(), c[2].parse().ok()?))
}

/// The command a truncated `toolUse` kept: its input became
/// `{"truncated":true,"preview":"{\"command\":\"…"}` (`bounded_event`).
fn preview_command(preview: &str) -> Option<String> {
    let at = preview.find("\"command\":\"")? + "\"command\":\"".len();
    let mut out = String::new();
    let mut chars = preview[at..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    Some(out)
}

/// A shell tool's command text, across agents' tool shapes, or `None` for
/// any other tool and for a background call (its result says nothing).
pub(crate) fn shell_command(name: &str, input: &Value) -> Option<String> {
    if !matches!(
        name,
        "Bash" | "bash" | "shell" | "Shell" | "exec_command" | "local_shell" | "run_shell_command"
    ) {
        return None;
    }
    if input.get("run_in_background").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    if let Some(preview) = input.get("preview").and_then(Value::as_str) {
        return preview_command(preview);
    }
    match input.get("command").or_else(|| input.get("cmd"))? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(Value::as_str)
                .map(|p| {
                    if p.contains(char::is_whitespace) {
                        format!("'{}'", p.replace('\'', ""))
                    } else {
                        p.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// The PRs agents merged or opened between `start` and `end` (the weekly
/// review, overnight 10-11 A6/A7), from the same tool calls as the night's.
pub(crate) fn pull_requests_between(
    vault: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<PullRequest> {
    let receipts = crate::evidence::list(vault, None, None, usize::MAX).unwrap_or_default();
    let mut out = pull_requests(vault, start, end, &receipts, &mut HashMap::new());
    out.truncate(LIST_CAP);
    out
}

/// The night's PRs from its shell tool calls, all of them (callers keep
/// [`LIST_CAP`]; the chains count every one). A merge with no repository
/// takes the one its PR was opened in, else the one its chat's receipt
/// recorded for that PR (`evidence::fill_pr_repos`: `gh`'s remote of the
/// folder the merge ran in).
fn pull_requests(
    vault: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    receipts: &[crate::evidence::EvidenceReceipt],
    titles: &mut HashMap<String, Option<String>>,
) -> Vec<PullRequest> {
    // `events.at` is `to_rfc3339()` (`+00:00`, any fraction): bounds in the
    // same shape compare correctly as text to the second; each row's time is
    // then checked exactly.
    let bound = |t: DateTime<Utc>| t.to_rfc3339_opts(SecondsFormat::Secs, false);
    type Row = (String, Option<i64>, String, String, String);
    let rows: Vec<Row> = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT session_key, turn_ord, at, kind, payload FROM events \
                 WHERE at >= ?1 AND at < ?2 AND kind IN ('toolUse', 'toolResult') \
                 ORDER BY id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([bound(start), bound(end + Duration::seconds(1))], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    })
    .unwrap_or_default();
    type Asked = (&'static str, Option<u64>, Option<String>, String);
    let mut asked: HashMap<(String, Option<i64>, String), Asked> = HashMap::new();
    let mut found: Vec<PullRequest> = Vec::new();
    for (key, turn, at, kind, payload) in rows {
        if !parse(&at).is_some_and(|t| t >= start && t < end) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&payload) else {
            continue;
        };
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if kind == "toolUse" {
            let name = v.get("name").and_then(Value::as_str).unwrap_or("");
            let input = v.get("input").cloned().unwrap_or(Value::Null);
            if let Some((action, number, repo)) =
                shell_command(name, &input).as_deref().and_then(pr_command)
            {
                asked.insert((key, turn, id), (action, number, repo, at));
            }
            continue;
        }
        let Some((action, number, repo, at)) = asked.remove(&(key.clone(), turn, id)) else {
            continue;
        };
        if v.get("isError").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let url = pr_url(&text_of(v.get("content")));
        let (repo, number) = match (action, number, url) {
            ("merged", Some(n), url) => (repo.or(url.map(|(r, _)| r)), n),
            ("opened", None, Some((r, n))) => (repo.or(Some(r)), n),
            _ => continue,
        };
        let title = titles
            .entry(key.clone())
            .or_insert_with(|| session_title(vault, &key))
            .clone();
        found.push(PullRequest {
            repo,
            number,
            action: action.to_string(),
            at,
            session_key: key,
            session_title: title,
        });
    }
    // A merge with no repository takes the one its PR was opened in.
    let opened: HashMap<u64, Option<String>> = found
        .iter()
        .filter(|p| p.action == "opened")
        .map(|p| (p.number, p.repo.clone()))
        .collect();
    let recorded: HashMap<(&str, u64), &str> = receipts
        .iter()
        .filter_map(|r| Some((r.session_key.as_deref()?, &r.pull_requests)))
        .flat_map(|(key, prs)| {
            prs.iter()
                .filter_map(move |p| Some(((key, p.number), p.repo.as_deref()?)))
        })
        .collect();
    for p in found
        .iter_mut()
        .filter(|p| p.action == "merged" && p.repo.is_none())
    {
        p.repo = opened.get(&p.number).cloned().flatten().or_else(|| {
            recorded
                .get(&(p.session_key.as_str(), p.number))
                .map(|r| r.to_string())
        });
    }
    // One row per PR: merged wins over opened; the first of each.
    let mut by_pr: BTreeMap<(Option<String>, u64), PullRequest> = BTreeMap::new();
    for p in found {
        let k = (p.repo.clone(), p.number);
        match by_pr.get(&k) {
            Some(old) if old.action == "merged" || p.action == "opened" => {}
            _ => {
                by_pr.insert(k, p);
            }
        }
    }
    let mut out: Vec<PullRequest> = by_pr.into_values().collect();
    out.sort_by(|a, b| a.at.cmp(&b.at));
    out
}

/// Each handed-over chat's predecessor (`successor → from`), read once:
/// every `handover` delegation record (the newest per successor wins), then
/// the handover registry for successors with no record (a run records at
/// most `delegation::MAX_RECORDED_PER_RUN` delegations).
fn handover_parents(vault: &Path) -> HashMap<String, String> {
    let mut parents: HashMap<String, String> = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT json_extract(d.record_json, '$.childSession'), t.session_key \
                 FROM delegations d JOIN turns t ON t.run_id = d.run_id \
                 WHERE json_extract(d.record_json, '$.kind') = 'childTask' \
                 AND json_extract(d.record_json, '$.tool') = ?1 \
                 ORDER BY d.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([crate::app_sessions::HANDOVER_TOOL], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        let out: HashMap<String, String> = rows
            .flatten()
            .filter_map(|(child, parent)| child.map(|c| (c, parent)))
            .collect();
        Ok(out)
    })
    .unwrap_or_default();
    for (key, from) in crate::app_sessions::handover_pairs(vault) {
        parents.entry(key).or_insert(from);
    }
    parents.retain(|child, parent| child != parent);
    parents
}

/// `key`'s chain's first chat: its handover predecessors followed back, at
/// most [`CHAIN_HOPS`]. On a cycle every chat in or leading into it gets
/// the cycle's smallest key, so the cycle is one chain.
fn chain_root(key: &str, parents: &HashMap<String, String>) -> String {
    let mut path = vec![key.to_string()];
    for _ in 0..CHAIN_HOPS {
        let Some(parent) = parents.get(path.last().unwrap()) else {
            break;
        };
        if let Some(at) = path.iter().position(|k| k == parent) {
            return path[at..].iter().min().unwrap().clone();
        }
        path.push(parent.clone());
    }
    path.pop().unwrap()
}

/// What a summary covers: everything in `[start, end)`, and after it, until
/// `limit`, only what `chats` did (the chats of the handover chains that
/// started in the night, followed to their end).
struct Span {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    limit: DateTime<Utc>,
    chats: HashSet<String>,
}

impl Span {
    fn holds(&self, at: DateTime<Utc>, chat: Option<&str>) -> bool {
        at >= self.start
            && (at < self.end || (at < self.limit && chat.is_some_and(|c| self.chats.contains(c))))
    }
}

/// The chats of the handover chains that started in `[start, end)`: the
/// chain's first chat had its first turn then (a chat that never handed
/// over, or a child task, is a chain of its own); every chat it handed
/// along joins, whenever it started.
fn chains_started_between(
    vault: &Path,
    parents: &HashMap<String, String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> HashSet<String> {
    let firsts: Vec<(String, String)> = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT session_key, MIN(started_at) FROM turns GROUP BY session_key")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        Ok(rows)
    })
    .unwrap_or_default();
    let mut chats: HashSet<String> = firsts
        .into_iter()
        .filter(|(key, at)| {
            parse(at).is_some_and(|at| at >= start && at < end) && chain_root(key, parents) == *key
        })
        .map(|(key, _)| key)
        .collect();
    if chats.is_empty() {
        return chats;
    }
    let joined: Vec<String> = parents
        .keys()
        .filter(|child| chats.contains(&chain_root(child, parents)))
        .cloned()
        .collect();
    chats.extend(joined);
    chats
}

/// A `from` / `to` argument: an RFC 3339 time, a local date and time
/// (`YYYY-MM-DD HH:MM[:SS]`, or with a `T`), or a date (local midnight).
fn parse_arg(raw: &str) -> Result<DateTime<Utc>, String> {
    let raw = raw.trim();
    if let Some(at) = parse(raw) {
        return Ok(at);
    }
    let naive = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ]
    .iter()
    .find_map(|f| chrono::NaiveDateTime::parse_from_str(raw, f).ok())
    .or_else(|| {
        chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .ok()
            .map(|d| d.and_time(NaiveTime::MIN))
    })
    .ok_or_else(|| {
        format!("'{raw}' is not an RFC 3339 time, a local YYYY-MM-DD HH:MM or a date.")
    })?;
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|at| at.with_timezone(&Utc))
        .ok_or_else(|| format!("'{raw}' is not a local time (a clock change skips it)."))
}

/// Each of `keys`' stints that started in the span, counted per chat (one
/// connection; `started_at` is parsed, not compared as text).
fn stints_between<'a>(
    vault: &Path,
    keys: impl Iterator<Item = &'a String>,
    span: &Span,
) -> HashMap<String, usize> {
    crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT started_at FROM stints WHERE session_key = ?1")
            .map_err(|e| e.to_string())?;
        let mut out = HashMap::new();
        for key in keys {
            let rows = stmt
                .query_map([key], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            let n = rows
                .flatten()
                .filter(|at| parse(at).is_some_and(|at| span.holds(at, Some(key))))
                .count();
            out.insert(key.clone(), n);
        }
        Ok(out)
    })
    .unwrap_or_default()
}

/// The chat each of `runs` was a turn of (`turns.run_id`); others are absent.
fn sessions_of_runs(vault: &Path, runs: &[String]) -> HashMap<String, String> {
    crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT session_key FROM turns WHERE run_id = ?1 LIMIT 1")
            .map_err(|e| e.to_string())?;
        let mut out = HashMap::new();
        for run in runs {
            if let Ok(key) = stmt.query_row([run], |r| r.get::<_, String>(0)) {
                out.insert(run.clone(), key);
            }
        }
        Ok(out)
    })
    .unwrap_or_default()
}

/// The whole span's figures beside its chains.
#[derive(Default)]
struct Totals {
    cost: f64,
    /// Runs with a NULL-cost row, each once.
    unknown_runs: HashSet<String>,
    /// NULL-cost rows with no run (Jev requests).
    unknown_calls: usize,
    in_progress: usize,
    /// The last kept row's end after the night (`limit` while one runs).
    last: Option<DateTime<Utc>>,
}

/// The span's chains (S06, see the module doc) from its usage rows and
/// every PR, with the whole span's totals. `peaks` is each run's
/// `apiUsage.peakContextTokens` from its receipt.
fn chains(
    vault: &Path,
    span: &Span,
    prs: &[PullRequest],
    parents: &HashMap<String, String>,
    peaks: &HashMap<String, u64>,
    titles: &mut HashMap<String, Option<String>>,
) -> (Vec<Chain>, Totals) {
    #[derive(Default)]
    struct Acc {
        chats: HashSet<String>,
        turns: HashSet<String>,
        merged: usize,
        opened: usize,
        cost: f64,
        unknown: HashSet<String>,
        in_progress: usize,
        peak: Option<u64>,
    }
    /// What a row adds, as `usage_ledger::add_row` buckets it (`cost_of`
    /// gives `None` for a row whose agent never started: it is left out).
    /// A known cost counts only when finite and positive (`add_cost`).
    #[derive(Clone, Copy)]
    enum Cost {
        Known(f64),
        Unknown,
        InProgress,
    }
    let cost_of = |row: &crate::session_store::UsageRow| {
        if row.outcome == crate::usage::LedgerOutcome::Running.as_str() {
            Some(Cost::InProgress)
        } else if !row.spawned {
            None
        } else {
            Some(match row.cost_usd {
                Some(c) => Cost::Known(if c.is_finite() && c > 0.0 { c } else { 0.0 }),
                None => Cost::Unknown,
            })
        }
    };
    let rows = crate::session_store::usage_rows_between(
        vault,
        &crate::session_store::db_time(span.start),
        &crate::session_store::db_time(span.limit),
    )
    .unwrap_or_default();
    let runs: Vec<String> = rows
        .iter()
        .filter_map(|r| r.run_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let session_of = sessions_of_runs(vault, &runs);
    let mut acc: HashMap<String, Acc> = HashMap::new();
    let mut totals = Totals::default();
    for row in &rows {
        let Some(cost) = cost_of(row) else {
            continue;
        };
        let key = row.run_id.as_ref().and_then(|run| session_of.get(run));
        let Some(at) = parse(&row.at) else {
            continue;
        };
        if !span.holds(at, key.map(String::as_str)) {
            continue;
        }
        if at >= span.end {
            let end = if matches!(cost, Cost::InProgress) {
                span.limit
            } else {
                at
            };
            totals.last = totals.last.max(Some(end));
        }
        match (cost, &row.run_id) {
            (Cost::Known(c), _) => totals.cost += c,
            (Cost::Unknown, Some(run)) => {
                totals.unknown_runs.insert(run.clone());
            }
            (Cost::Unknown, None) => totals.unknown_calls += 1,
            (Cost::InProgress, _) => totals.in_progress += 1,
        }
        let (Some(run), Some(key)) = (row.run_id.as_ref(), key) else {
            continue;
        };
        let root = chain_root(key, parents);
        let a = acc.entry(root).or_default();
        a.chats.insert(key.clone());
        a.turns.insert(run.clone());
        match cost {
            Cost::Known(c) => a.cost += c,
            Cost::Unknown => {
                a.unknown.insert(run.clone());
            }
            Cost::InProgress => a.in_progress += 1,
        }
        let peak = row
            .usage_json
            .as_deref()
            .and_then(|j| serde_json::from_str::<Value>(j).ok())
            .and_then(|v| v.get("peakContextTokens").and_then(Value::as_u64));
        a.peak = a.peak.max(peak).max(peaks.get(run).copied());
    }
    for p in prs {
        let root = chain_root(&p.session_key, parents);
        let a = acc.entry(root).or_default();
        a.chats.insert(p.session_key.clone());
        if p.action == "merged" {
            a.merged += 1;
        } else {
            a.opened += 1;
        }
    }
    let stints = stints_between(vault, acc.values().flat_map(|a| a.chats.iter()), span);
    let mut out: Vec<Chain> = acc
        .into_iter()
        .map(|(root, a)| Chain {
            session_title: titles
                .entry(root.clone())
                .or_insert_with(|| session_title(vault, &root))
                .clone(),
            session_key: root,
            chats: a.chats.len(),
            turns: a.turns.len(),
            stints: a.chats.iter().filter_map(|k| stints.get(k)).sum(),
            merged: a.merged,
            opened: a.opened,
            cost_usd: a.cost,
            unknown_cost: a.unknown.len(),
            in_progress: a.in_progress,
            peak_context_tokens: a.peak,
            usd_per_merged_pr: per_merged_pr(a.cost, a.merged),
        })
        .collect();
    out.sort_by(|a, b| {
        b.cost_usd
            .total_cmp(&a.cost_usd)
            .then_with(|| a.session_key.cmp(&b.session_key))
    });
    out.truncate(LIST_CAP);
    (out, totals)
}

/// `cost / merged`, `None` with no merged PR.
fn per_merged_pr(cost: f64, merged: usize) -> Option<f64> {
    (merged > 0).then(|| cost / merged as f64)
}

fn session_title(vault: &Path, key: &str) -> Option<String> {
    crate::session_store::with_read(vault, |conn| {
        use rusqlite::OptionalExtension;
        conn.query_row("SELECT title FROM sessions WHERE key = ?1", [key], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()
        .map(Option::flatten)
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .filter(|t| !t.trim().is_empty())
}

/// The evening's morning report, else the newest written since `from`.
fn report_for(vault: &Path, evening: chrono::NaiveDate, from: DateTime<Utc>) -> Option<String> {
    let rel_dir = format!(
        "workspaces/{}/outputs/night",
        crate::workspaces::MAIN_AGENT_ID
    );
    let named = format!("{rel_dir}/{evening}.md");
    if vault.join(&named).is_file() {
        return Some(named);
    }
    let newest = std::fs::read_dir(vault.join(&rel_dir))
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .filter_map(|e| {
            let modified: DateTime<Utc> = e.metadata().ok()?.modified().ok()?.into();
            (modified >= from).then(|| (modified, e.file_name().to_string_lossy().to_string()))
        })
        .max()?;
    Some(format!("{rel_dir}/{}", newest.1))
}

/// Checks whose last run (by sequence) of the same command failed.
fn failed_checks(checks: &[crate::evidence::ReceiptCheck]) -> usize {
    let mut last: HashMap<&str, (u64, bool)> = HashMap::new();
    for c in checks {
        let entry = last
            .entry(c.command.as_str())
            .or_insert((c.sequence, false));
        if c.sequence >= entry.0 {
            *entry = (c.sequence, c.outcome == "failed");
        }
    }
    last.values().filter(|(_, failed)| *failed).count()
}

/// `overnight_summary` (Home's card): last night, with the handover chains
/// that started in it followed to their end.
pub fn summary(vault: &Path, now: DateTime<Local>) -> Summary {
    let (evening, start, end) = night_at(now);
    let parents = handover_parents(vault);
    let start_t = NaiveTime::from_hms_opt(crate::budgets::OVERNIGHT_START_HOUR, 0, 0).unwrap();
    let next_night = Local
        .from_local_datetime(&(evening + Duration::days(1)).and_time(start_t))
        .earliest()
        .map_or(start + Duration::days(1), |at| at.with_timezone(&Utc));
    let span = Span {
        start,
        end,
        limit: now.with_timezone(&Utc).min(next_night).max(end),
        chats: chains_started_between(vault, &parents, start, end),
    };
    summarize(vault, evening, span, parents)
}

/// The `system.overnight_summary` action: [`summary`], or with `from` /
/// `to` (either; [`parse_arg`]) exactly that window, `from` defaulting to
/// the night's start and `to` to now, with no chain followed past it.
pub fn summary_for(
    vault: &Path,
    now: DateTime<Local>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Summary, String> {
    if from.is_none() && to.is_none() {
        return Ok(summary(vault, now));
    }
    let start = match from {
        Some(raw) => parse_arg(raw)?,
        None => night_at(now).1,
    };
    let end = match to {
        Some(raw) => parse_arg(raw)?,
        None => now.with_timezone(&Utc),
    };
    if start >= end {
        return Err(match (from, to) {
            (None, _) => format!(
                "`to` must be after last night's start ({}), or give a `from` too.",
                crate::session_store::db_time(start)
            ),
            (_, None) => "`from` must be in the past.".into(),
            _ => "`from` must be before `to`.".into(),
        });
    }
    let span = Span {
        start,
        end,
        limit: end,
        chats: HashSet::new(),
    };
    // Overnight 10 A8: a start before noon is the morning after a night (a
    // bare date is local midnight), so it names the evening before, as
    // `night_at` does; from noon on, its own day.
    let local_start = start.with_timezone(&Local);
    let evening = local_start.date_naive() - Duration::days(i64::from(local_start.hour() < 12));
    Ok(summarize(vault, evening, span, handover_parents(vault)))
}

fn summarize(
    vault: &Path,
    evening: chrono::NaiveDate,
    span: Span,
    parents: HashMap<String, String>,
) -> Summary {
    let all_receipts = crate::evidence::list(vault, None, None, usize::MAX).unwrap_or_default();
    let peaks: HashMap<String, u64> = all_receipts
        .iter()
        .filter_map(|r| {
            let peak = r.api_usage.as_ref()?.peak_context_tokens?;
            Some((r.run_id.clone(), peak))
        })
        .collect();
    let mut last = None;
    let mut receipts: Vec<_> = all_receipts
        .iter()
        .filter(|r| {
            let Some(at) = parse(&r.started_at).or_else(|| parse(&r.finished_at)) else {
                return false;
            };
            let held = span.holds(at, r.session_key.as_deref());
            if held && at >= span.end {
                last = last.max(parse(&r.finished_at).or(Some(at)));
            }
            held
        })
        .collect();
    receipts.sort_by(|a, b| a.finished_at.cmp(&b.finished_at));
    let mut titles: HashMap<String, Option<String>> = HashMap::new();
    let mut failed = Vec::new();
    let mut commits: Vec<Commit> = Vec::new();
    let mut succeeded = 0;
    let mut conversations = std::collections::HashSet::new();
    for r in &receipts {
        if let Some(k) = &r.session_key {
            conversations.insert(k.clone());
        }
        let failed_checks = failed_checks(&r.checks);
        if matches!(r.status.as_str(), "success" | "cancelled") && failed_checks == 0 {
            succeeded += usize::from(r.status == "success");
        } else {
            let session_title = r.session_key.as_ref().and_then(|k| {
                titles
                    .entry(k.clone())
                    .or_insert_with(|| session_title(vault, k))
                    .clone()
            });
            failed.push(FailedRun {
                status: r.status.clone(),
                result_kind: r.result_kind.clone(),
                failed_checks,
                at: r.finished_at.clone(),
                session_key: r.session_key.clone(),
                session_title,
                source: r.source.clone(),
            });
        }
        for repo in &r.repo_changes {
            let (Some(a), Some(b)) = (&repo.head_before, &repo.head_after) else {
                continue;
            };
            if a == b {
                continue;
            }
            let short = |s: &str| s.chars().take(8).collect::<String>();
            match commits.iter_mut().find(|c| c.repo == repo.label) {
                Some(c) => {
                    c.to = short(b);
                    c.runs += 1;
                    c.at = r.finished_at.clone();
                }
                None => commits.push(Commit {
                    repo: repo.label.clone(),
                    from: short(a),
                    to: short(b),
                    runs: 1,
                    at: r.finished_at.clone(),
                }),
            }
        }
    }
    failed.sort_by(|a, b| b.at.cmp(&a.at));
    failed.truncate(LIST_CAP);
    commits.truncate(LIST_CAP);
    let mut pull_requests =
        pull_requests(vault, span.start, span.limit, &all_receipts, &mut titles);
    pull_requests.retain(|p| {
        let Some(at) = parse(&p.at) else {
            return false;
        };
        let held = span.holds(at, Some(&p.session_key));
        if held && at >= span.end {
            last = last.max(Some(at));
        }
        held
    });
    let merged_prs = pull_requests
        .iter()
        .filter(|p| p.action == "merged")
        .count();
    let (chains, totals) = chains(vault, &span, &pull_requests, &parents, &peaks, &mut titles);
    let to = last
        .max(totals.last)
        .map_or(span.end, |at| at.clamp(span.end, span.limit));
    pull_requests.truncate(LIST_CAP);
    let waiting = crate::needs_you::list(vault, Vec::new(), None, None, Some(3))
        .map(|page| Waiting {
            total: page.total,
            titles: page.items.into_iter().map(|i| i.title).collect(),
        })
        .unwrap_or(Waiting {
            total: 0,
            titles: Vec::new(),
        });
    Summary {
        night: evening.to_string(),
        from: crate::session_store::db_time(span.start),
        to: crate::session_store::db_time(to),
        ran: !receipts.is_empty() || !pull_requests.is_empty(),
        runs: receipts.len(),
        conversations: conversations.len(),
        succeeded,
        failed,
        pull_requests,
        commits,
        waiting,
        report: report_for(vault, evening, span.start),
        chains,
        cost_usd: totals.cost,
        unknown_cost: totals.unknown_runs.len(),
        unknown_cost_calls: totals.unknown_calls,
        in_progress: totals.in_progress,
        merged_prs,
        usd_per_merged_pr: per_merged_pr(totals.cost, merged_prs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::{append_event, start_turn};

    fn local(s: &str) -> DateTime<Local> {
        Local
            .from_local_datetime(
                &chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap(),
            )
            .earliest()
            .unwrap()
    }

    #[test]
    fn the_night_is_the_overnight_window_that_ended_this_morning() {
        let (evening, start, end) = night_at(local("2026-10-10 06:00"));
        assert_eq!(evening.to_string(), "2026-10-09");
        assert_eq!(start, local("2026-10-09 18:00").with_timezone(&Utc));
        assert_eq!(end, local("2026-10-10 06:00").with_timezone(&Utc), "so far");
        let (evening, _, end) = night_at(local("2026-10-10 11:30"));
        assert_eq!(evening.to_string(), "2026-10-09");
        assert_eq!(end, local("2026-10-10 07:00").with_timezone(&Utc));
        // All evening it is still last night's card, never tonight's.
        let (evening, start, _) = night_at(local("2026-10-10 21:00"));
        assert_eq!(evening.to_string(), "2026-10-09");
        assert_eq!(start, local("2026-10-09 18:00").with_timezone(&Utc));
    }

    #[test]
    fn pr_commands_are_read_as_commands_with_their_own_number() {
        let merged =
            |n: u64, repo: Option<&str>| Some(("merged", Some(n), repo.map(str::to_string)));
        assert_eq!(
            pr_command(
                "gh pr merge 451 -R example-org/hollow-code --merge --subject \"Merge #451: x\""
            ),
            merged(451, Some("example-org/hollow-code"))
        );
        assert_eq!(
            pr_command("cd x && gh pr merge --merge 12"),
            merged(12, None)
        );
        assert_eq!(
            pr_command("gh pr merge --subject \"fixes 3 bugs\" 451 --repo=o/r"),
            merged(451, Some("o/r")),
            "a flag's value is not the number"
        );
        assert_eq!(
            pr_command("gh pr merge https://github.com/o/r/pull/7 --merge && gh pr view 8"),
            merged(7, Some("o/r"))
        );
        assert_eq!(
            pr_command("bash -lc 'gh pr merge 460 --merge'"),
            merged(460, None)
        );
        assert_eq!(
            pr_command("gh pr create -R o/r --title t --body-file b"),
            Some(("opened", None, Some("o/r".into())))
        );
        for not_a_merge in [
            "gh pr view 451",
            "echo gh pr merge 5",
            "git commit -m \"then gh pr merge 9\"",
            "git commit -F - <<'EOF'\ngh pr merge 9\nEOF",
            "gh pr merge --auto 451",
            "gh pr merge --merge",
        ] {
            assert_eq!(pr_command(not_a_merge), None, "{not_a_merge}");
        }
    }

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn sql(v: &Path, statement: &str) {
        crate::session_store::with_write(v, "test", |conn| {
            conn.execute(statement, []).map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
    }

    fn receipt(
        v: &Path,
        id: &str,
        status: &str,
        at: &str,
        checks: &[(&str, u64, &str)],
        heads: Option<(&str, &str)>,
    ) {
        crate::evidence::persist_for_test(
            v,
            crate::evidence::EvidenceReceipt {
                id: id.into(),
                run_id: id.into(),
                source: "chat".into(),
                status: status.into(),
                started_at: at.into(),
                finished_at: at.into(),
                session_key: Some("s-night".into()),
                checks: checks
                    .iter()
                    .map(|(cmd, seq, outcome)| crate::evidence::ReceiptCheck {
                        command: (*cmd).into(),
                        kind: "test".into(),
                        outcome: (*outcome).into(),
                        sequence: *seq,
                        excerpt: None,
                    })
                    .collect(),
                repo_changes: heads
                    .map(|(a, b)| {
                        vec![serde_json::from_value::<crate::repo_diff::RepoChanges>(
                            serde_json::json!({
                                "root": "/r", "label": "Hollow", "target": "cwd",
                                "status": "captured", "headBefore": a, "headAfter": b,
                            }),
                        )
                        .unwrap()]
                    })
                    .unwrap_or_default(),
                ..Default::default()
            },
        )
        .unwrap();
    }

    /// 09:00 ET on 10-10: the night is 10-09 18:00 to 10-10 07:00 ET.
    fn morning() -> DateTime<Local> {
        DateTime::parse_from_rfc3339("2026-10-10T13:00:00Z")
            .unwrap()
            .with_timezone(&Local)
    }

    #[test]
    fn the_nights_merges_failures_and_commits_are_listed_and_the_rest_is_not() {
        let dir = vault();
        let v = dir.path();
        let key = "s-night";
        let ord = start_turn(v, key, "claude", "/vault", "run-1", "Overnight lane B").unwrap();
        let tool = |id: &str, name: &str, input: Value| {
            let payload = serde_json::json!({ "id": id, "name": name, "input": input }).to_string();
            append_event(v, key, ord, "toolUse", &payload).unwrap();
        };
        let result = |id: &str, content: &str, error: bool| {
            let payload =
                serde_json::json!({ "id": id, "content": content, "isError": error }).to_string();
            append_event(v, key, ord, "toolResult", &payload).unwrap();
        };
        let bash = |cmd: &str| serde_json::json!({ "command": cmd });
        // Opened without -R, merged without -R: one row, merged, in the
        // repository the URL named.
        tool(
            "t1",
            "Bash",
            bash("gh pr create --title C6 --body-file /tmp/b"),
        );
        result(
            "t1",
            "https://github.com/example-org/hollow-code/pull/451\n",
            false,
        );
        tool("t2", "Bash", bash("gh pr merge 451 --merge"));
        result("t2", "", false);
        // A long body: the stored event keeps only a preview of the input.
        let body = "x".repeat(9_000);
        tool(
            "t3",
            "Bash",
            bash(&format!(
                "gh pr create -R example-org/hollow-code --title C8 --body \"{body}\""
            )),
        );
        result(
            "t3",
            "https://github.com/example-org/hollow-code/pull/452\n",
            false,
        );
        // Codex: an argv array through bash -lc.
        tool(
            "t4",
            "exec_command",
            serde_json::json!({ "cmd": ["bash", "-lc", "gh pr merge 460 -R o/r --merge"] }),
        );
        result("t4", "", false);
        // Not merges: an error, a background call, a view.
        tool("t5", "Bash", bash("gh pr merge 999 --merge"));
        result("t5", "GraphQL: Pull request is not mergeable", true);
        tool(
            "t6",
            "Bash",
            serde_json::json!({ "command": "gh pr merge 777 --merge", "run_in_background": true }),
        );
        result("t6", "Command running in background", false);
        tool("t7", "Bash", bash("gh pr view 451"));
        result("t7", "MERGED", false);
        // The stored shape: `to_rfc3339()`, with a fraction.
        sql(
            v,
            "UPDATE events SET at = '2026-10-10T06:00:00.123456789+00:00'",
        );
        sql(v, "UPDATE sessions SET title = 'Overnight lane B'");
        receipt(
            v,
            "ok",
            "success",
            "2026-10-10T05:00:00Z",
            &[("npm test", 1, "failed"), ("npm test", 2, "passed")],
            Some(("aaaaaaaaaa", "bbbbbbbbbb")),
        );
        receipt(
            v,
            "ok2",
            "success",
            "2026-10-10T05:30:00Z",
            &[],
            Some(("bbbbbbbbbb", "cccccccccc")),
        );
        receipt(v, "err", "error", "2026-10-10T06:00:00Z", &[], None);
        receipt(
            v,
            "red",
            "success",
            "2026-10-10T06:30:00Z",
            &[("npm test", 1, "passed"), ("npm test", 3, "failed")],
            None,
        );
        receipt(v, "stop", "cancelled", "2026-10-10T07:00:00Z", &[], None);
        receipt(v, "day", "error", "2026-10-09T15:00:00Z", &[], None);
        let s = summary(v, morning());
        assert!(s.ran);
        assert_eq!(s.runs, 5, "the daytime receipt is not the night's");
        assert_eq!(s.succeeded, 2, "failed then passed is a success");
        assert_eq!(s.conversations, 1);
        let failed: Vec<(&str, usize)> = s
            .failed
            .iter()
            .map(|f| (f.status.as_str(), f.failed_checks))
            .collect();
        assert_eq!(failed, [("success", 1), ("error", 0)]);
        assert_eq!(
            s.failed[0].session_title.as_deref(),
            Some("Overnight lane B")
        );
        let prs: Vec<(Option<&str>, u64, &str)> = s
            .pull_requests
            .iter()
            .map(|p| (p.repo.as_deref(), p.number, p.action.as_str()))
            .collect();
        assert_eq!(
            prs,
            [
                (Some("example-org/hollow-code"), 451, "merged"),
                (Some("example-org/hollow-code"), 452, "opened"),
                (Some("o/r"), 460, "merged"),
            ],
            "{:?}",
            s.pull_requests
        );
        assert_eq!(s.commits.len(), 1, "one row per repository");
        let c = &s.commits[0];
        assert_eq!(
            (c.from.as_str(), c.to.as_str(), c.runs),
            ("aaaaaaaa", "cccccccc", 2)
        );
        assert_eq!(s.night, "2026-10-09");
    }

    /// S06: lane A handed over from s-a1 to s-a2; lane B (s-b) never did.
    #[test]
    fn s06_the_nights_cost_is_grouped_by_handover_chain_with_unknowns_and_dollars_per_pr() {
        let dir = vault();
        let v = dir.path();
        let a1 = start_turn(v, "s-a1", "claude", "/vault", "run-a1", "Lane A").unwrap();
        let a2 = start_turn(v, "s-a2", "claude", "/vault", "run-a2", "Lane A, next").unwrap();
        start_turn(v, "s-b", "claude", "/vault", "run-b1", "Lane B").unwrap();
        start_turn(v, "s-b", "claude", "/vault", "run-b2", "Lane B again").unwrap();
        start_turn(
            v,
            "s-b",
            "claude",
            "/vault",
            "run-b3",
            "Lane B, still going",
        )
        .unwrap();
        let now = crate::session_store::db_time(Utc::now());
        let handover = "{\"kind\":\"childTask\",\"tool\":\"handover\",\"childSession\":\"s-a2\"}";
        crate::session_store::record_delegation(v, "run-a1", &now, handover, 10).unwrap();
        let pr = |key: &str, ord: i64, id: &str, cmd: &str, out: &str| {
            let input = serde_json::json!({ "command": cmd });
            let call = serde_json::json!({ "id": id, "name": "Bash", "input": input });
            append_event(v, key, ord, "toolUse", &call.to_string()).unwrap();
            let result = serde_json::json!({ "id": id, "content": out, "isError": false });
            append_event(v, key, ord, "toolResult", &result.to_string()).unwrap();
        };
        pr("s-a1", a1, "p1", "gh pr merge 10 -R o/r --merge", "");
        pr("s-a2", a2, "p2", "gh pr merge 11 -R o/r --merge", "");
        pr(
            "s-a2",
            a2,
            "p3",
            "gh pr create --title t --body b",
            "https://github.com/o/r/pull/12\n",
        );
        sql(v, "UPDATE events SET at = '2026-10-10T06:00:00+00:00'");
        sql(v, "UPDATE sessions SET title = 'Lane ' || key");
        // One stint per chat in the night (s-b's turns share one), and s-b
        // had another the day before.
        sql(
            v,
            "UPDATE stints SET started_at = '2026-10-10T06:00:00+00:00'",
        );
        sql(
            v,
            "INSERT INTO stints(session_key, agent, started_at) \
             VALUES('s-b', 'codex', '2026-10-09T15:00:00+00:00')",
        );
        let usage = |key: &str, at: &str, kind: &str, run: &str, cost: Option<f64>, json: &str| {
            let mut row = crate::session_store::tests::usage_row_fixture(key, at, "success");
            row.surface = kind.into();
            row.run_id = Some(run.into());
            row.cost_usd = cost;
            row.usage_json = (!json.is_empty()).then(|| json.to_string());
            crate::session_store::upsert_usage(v, &row).unwrap();
        };
        let night = "2026-10-10T06:00:00.000Z";
        usage("u-a1", night, "chat", "run-a1", Some(2.0), "");
        let peak = "{\"peakContextTokens\":50000}";
        usage("u-a2", night, "chat", "run-a2", Some(3.0), peak);
        // A consultation shares its turn's run id: the same chain.
        usage("u-a2-ask", night, "delegate", "run-a2", Some(1.0), "");
        usage("u-b1", night, "chat", "run-b1", None, "");
        usage("u-b2", night, "chat", "run-b2", Some(1.5), "");
        // Still running: in progress, not unknown.
        let mut running = crate::session_store::tests::usage_row_fixture("u-b3", night, "running");
        running.run_id = Some("run-b3".into());
        crate::session_store::upsert_usage(v, &running).unwrap();
        // A helper whose agent never started: left out, not unknown.
        let mut unspawned =
            crate::session_store::tests::usage_row_fixture("u-a2-x", night, "spawn_failed");
        unspawned.surface = "delegate".into();
        unspawned.run_id = Some("run-a2".into());
        unspawned.spawned = false;
        crate::session_store::upsert_usage(v, &unspawned).unwrap();
        // A run in no chat: in the night's total, in no chain.
        usage("u-loose", night, "workflow", "run-gone", Some(0.5), "");
        // The day before: not the night's.
        let day = "2026-10-09T15:00:00.000Z";
        usage("u-day", day, "chat", "run-b1", Some(9.0), "");
        let s = summary(v, morning());
        let lane_a = Chain {
            session_key: "s-a1".into(),
            session_title: Some("Lane s-a1".into()),
            chats: 2,
            turns: 2,
            stints: 2,
            merged: 2,
            opened: 1,
            cost_usd: 6.0,
            unknown_cost: 0,
            in_progress: 0,
            peak_context_tokens: Some(50_000),
            usd_per_merged_pr: Some(3.0),
        };
        let lane_b = Chain {
            session_key: "s-b".into(),
            session_title: Some("Lane s-b".into()),
            chats: 1,
            turns: 3,
            stints: 1,
            merged: 0,
            opened: 0,
            cost_usd: 1.5,
            unknown_cost: 1,
            in_progress: 1,
            peak_context_tokens: None,
            usd_per_merged_pr: None,
        };
        assert_eq!(s.chains, [lane_a, lane_b]);
        let in_chains: f64 = s.chains.iter().map(|c| c.cost_usd).sum();
        assert!(
            s.cost_usd > in_chains,
            "the loose run counts in the night only"
        );
        assert_eq!(
            (s.cost_usd, s.unknown_cost, s.in_progress, s.merged_prs),
            (8.0, 1, 1, 2)
        );
        assert_eq!(s.usd_per_merged_pr, Some(4.0));
    }

    /// S06: a handover whose delegation record is missing (past the
    /// per-run cap) is found in the handover registry, a delegation record
    /// wins over the registry, and a handover cycle is one chain.
    #[test]
    fn s06_a_handover_with_no_record_comes_from_the_registry_and_a_cycle_is_one_chain() {
        let dir = vault();
        let v = dir.path();
        for (key, run) in [
            ("s-d1", "run-d1"),
            ("s-d2", "run-d2"),
            ("s-x", "run-x"),
            ("s-c1", "run-c1"),
            ("s-c2", "run-c2"),
        ] {
            start_turn(v, key, "claude", "/vault", run, "Work").unwrap();
        }
        crate::app_sessions::record_handover(v, "s-d1", "s-d2", 1).unwrap();
        crate::app_sessions::record_handover(v, "s-x", "s-c1", 1).unwrap();
        let now = crate::session_store::db_time(Utc::now());
        let handover = |run: &str, to: &str| {
            let tool = crate::app_sessions::HANDOVER_TOOL;
            let json = serde_json::json!({ "kind": "childTask", "tool": tool, "childSession": to });
            crate::session_store::record_delegation(v, run, &now, &json.to_string(), 10).unwrap();
        };
        // s-c1 and s-c2 handed over to each other; the registry's s-x
        // record for s-c1 loses to the delegation record.
        handover("run-c1", "s-c2");
        handover("run-c2", "s-c1");
        let night = "2026-10-10T06:00:00.000Z";
        for run in ["run-d1", "run-d2", "run-x", "run-c1", "run-c2"] {
            let mut row = crate::session_store::tests::usage_row_fixture(run, night, "success");
            row.run_id = Some(run.into());
            row.cost_usd = Some(1.0);
            crate::session_store::upsert_usage(v, &row).unwrap();
        }
        let s = summary(v, morning());
        let chains: Vec<_> = s
            .chains
            .iter()
            .map(|c| (c.session_key.as_str(), c.chats, c.cost_usd))
            .collect();
        assert_eq!(
            chains,
            [("s-c1", 2, 2.0), ("s-d1", 2, 2.0), ("s-x", 1, 1.0)]
        );
    }

    fn receipt_in(run: &str, key: &str, at: &str) -> crate::evidence::EvidenceReceipt {
        crate::evidence::EvidenceReceipt {
            id: run.into(),
            run_id: run.into(),
            source: "chat".into(),
            status: "success".into(),
            started_at: at.into(),
            finished_at: at.into(),
            session_key: Some(key.into()),
            ..Default::default()
        }
    }

    fn persist(v: &Path, r: crate::evidence::EvidenceReceipt) {
        crate::evidence::persist_for_test(v, r).unwrap();
    }

    /// S06 follow-up: a body `cat` writes through a here-document is not
    /// commands, but what follows its closing line is (the lanes' `cat >
    /// pr.md <<'EOF'` … `EOF` then `gh pr create --body-file pr.md`).
    #[test]
    fn s06_a_pr_command_after_a_heredoc_is_read() {
        let opened = Some(("opened", None, None));
        assert_eq!(
            pr_command(
                "cat > /tmp/pr.md <<'EOF'\nIt's the body\ngh pr merge 9\nEOF\n\
                 gh pr create --base main --title t --body-file /tmp/pr.md 2>&1 | tail -1"
            ),
            opened
        );
        assert_eq!(
            pr_command("gh pr create --title t --body \"$(cat <<'EOF'\nIt's \"x\"\nEOF\n)\""),
            opened,
            "a body inside the quotes"
        );
        assert_eq!(
            pr_command("cat <<-EOF > b\n\tgh pr merge 9\n\tEOF\ngh pr merge 10 --merge"),
            Some(("merged", Some(10), None))
        );
        assert_eq!(
            pr_command("cat <<EOF\ngh pr merge 9\nEOF\necho done"),
            None,
            "the body's line is text"
        );
    }

    /// S06 review: only an unquoted `<<` outside a comment opens a
    /// here-document, a `<<<` here-string never does, and a quoted script's
    /// here-document stays whole for its own pass.
    #[test]
    fn s06_a_quoted_or_commented_heredoc_marker_hides_nothing() {
        let merged = |n: u64| Some(("merged", Some(n), None));
        for (command, expected) in [
            (
                "git commit -m \"shift a << b\"\ngh pr merge 5 --squash",
                merged(5),
            ),
            ("rg -n '<<EOF' src\ngh pr merge 6 --merge", merged(6)),
            ("# note << x\ngh pr merge 7 --merge", merged(7)),
            (
                "cat <<< \"$x\" >a; cat <<EOF >b\ngh pr merge 9\nEOF\ngh pr merge 8 --merge",
                merged(8),
            ),
            ("gh pr merge #451 --merge", merged(451)),
            (
                "bash -c 'cat > f <<EOF\nbody\nEOF\ngh pr create --body-file f'",
                Some(("opened", None, None)),
            ),
            // Mid-command, and at a command's start with no space after `#`.
            ("echo hi # see << x\ngh pr merge 7 --merge", merged(7)),
            ("#x << y\ngh pr merge 8 --merge", merged(8)),
        ] {
            assert_eq!(pr_command(command), expected, "{command:?}");
        }
    }

    /// S06 delta review: a here-document inside `"$( … )"` (how agents write
    /// commit and PR bodies) is skipped too, so an odd `"` or a `'` in its
    /// body can neither hide a later command nor make one of a body line;
    /// and a body ends only at its exact delimiter line (`<<-`: leading tabs).
    #[test]
    fn s06_a_heredoc_in_a_quoted_command_substitution_and_exact_delimiters() {
        let merged = |n: u64| Some(("merged", Some(n), None));
        for (command, expected) in [
            (
                "git commit -m \"$(cat <<'EOF'\nUse a 2\" gap\nEOF\n)\" && git push && gh pr merge 5 --squash",
                merged(5),
            ),
            (
                "git commit -m \"$(cat <<'EOF'\n3\" margin\ngh pr merge 12\nEOF\n)\"",
                None,
            ),
            (
                "gh pr create --title t --body \"$(cat <<'EOF'\nRename \"Bob's\" field\nEOF\n)\"\ngh pr merge 6 --merge",
                Some(("opened", None, None)),
            ),
            (
                "git commit -m \"$(cat <<'EOF'\nRename \"Bob's\" field\nEOF\n)\" && gh pr merge 6 --merge",
                merged(6),
            ),
            (
                "cat > /tmp/pr.md <<'EOF'\nExample:\n    EOF\n    gh pr merge 5 --merge\nEOF\ngh pr create --title t --body-file /tmp/pr.md",
                Some(("opened", None, None)),
            ),
            ("cat <<-EOF > f\n\tbody\n\tEOF\ngh pr merge 6 --merge", merged(6)),
            ("cat <<EOF > f\r\nbody\r\nEOF\r\ngh pr merge 7 --merge", merged(7)),
            // Second delta review: `EOF)"` closes it (bash accepts that);
            // `$((…<<…))` and `'…<<…'` inside open nothing; a `)` on the
            // opener's line doesn't strand the body.
            (
                "bash -lc 'git commit -m \"$(cat <<EOF\nmsg\nEOF)\" && gh pr merge 5 --squash'",
                merged(5),
            ),
            ("echo \"$((1<<2))\"\ngh pr merge 5 --merge", merged(5)),
            (
                "git commit -m \"$(printf '%s' 'a<<b')\" && gh pr merge 5",
                merged(5),
            ),
            (
                "git commit -m \"$(cat <<'EOF' | sed 's/(wip)//'\nfix: x\nEOF\n)\" && git push\ngh pr merge 5 --squash",
                merged(5),
            ),
            // Final review: each guard on its own (a `$((`, an unquoted `)`
            // on the opener's line, an opener whose quote closes on its
            // line), a comment's `'`, and `EOF )`.
            ("echo \"$((1<<2))\ntext\"\ngh pr merge 5", merged(5)),
            (
                "git commit -m \"$(cat <<EOF | (cat)\nit's 2\" wide\nEOF\n)\" && gh pr merge 5",
                merged(5),
            ),
            ("git commit -m \"$(cat <<EOF)\" && x\ngh pr merge 5", merged(5)),
            (
                "git commit -m \"$(cat <<'EOF'  # don't indent\nfix: x\nEOF\n)\" && gh pr merge 5 --squash",
                merged(5),
            ),
            ("x=\"$(cat <<EOF\nmsg\nEOF )\" && gh pr merge 9", merged(9)),
        ] {
            assert_eq!(pr_command(command), expected, "{command:?}");
        }
    }

    /// S06 follow-up: a merge without `-R` gets its repository from a create
    /// written after a here-document, else from its chat's receipt.
    #[test]
    fn s06_a_merge_without_a_repository_takes_its_create_or_its_receipt() {
        let dir = vault();
        let v = dir.path();
        let key = "s-r";
        let ord = start_turn(v, key, "claude", "/vault", "run-r", "Lane R").unwrap();
        let call = |id: &str, cmd: &str, out: &str| {
            let input = serde_json::json!({ "command": cmd });
            let use_ = serde_json::json!({ "id": id, "name": "Bash", "input": input });
            append_event(v, key, ord, "toolUse", &use_.to_string()).unwrap();
            let result = serde_json::json!({ "id": id, "content": out, "isError": false });
            append_event(v, key, ord, "toolResult", &result.to_string()).unwrap();
        };
        call(
            "c1",
            "cd /w && cat > /tmp/pr.md <<'EOF'\nbody\nEOF\ngh pr create --title t --body-file /tmp/pr.md",
            "https://github.com/o/r/pull/30\n",
        );
        call("c2", "gh pr merge 30 --merge", "");
        call("c3", "gh pr merge 31 --merge", "");
        call("c4", "gh pr merge 32 --merge", "");
        sql(v, "UPDATE events SET at = '2026-10-10T06:00:00+00:00'");
        let mut r = receipt_in("run-r", key, "2026-10-10T06:00:00Z");
        r.pull_requests = vec![crate::evidence::ReceiptPullRequest {
            number: 31,
            repo: Some("o/r2".into()),
            action: "merged".into(),
            url: None,
        }];
        persist(v, r);
        let s = summary(v, morning());
        let prs: Vec<(Option<&str>, u64, &str)> = s
            .pull_requests
            .iter()
            .map(|p| (p.repo.as_deref(), p.number, p.action.as_str()))
            .collect();
        assert_eq!(
            prs,
            [
                (None, 32, "merged"),
                (Some("o/r"), 30, "merged"),
                (Some("o/r2"), 31, "merged"),
            ],
            "one row for 30, merged; nothing to go on for 32"
        );
    }

    /// S06 follow-up: a run counts once in `unknownCost` however many
    /// NULL-cost rows it has; a row with no run (a Jev request) is not a
    /// run; peak context comes from the rows or the receipts.
    #[test]
    fn s06_an_unknown_cost_run_counts_once_and_peak_context_comes_from_receipts() {
        let dir = vault();
        let v = dir.path();
        start_turn(v, "s-u", "claude", "/vault", "run-u1", "Lane U").unwrap();
        start_turn(v, "s-u", "claude", "/vault", "run-u2", "Lane U again").unwrap();
        let night = "2026-10-10T06:00:00.000Z";
        let usage = |key: &str, surface: &str, run: Option<&str>, cost: Option<f64>, json: &str| {
            let mut row = crate::session_store::tests::usage_row_fixture(key, night, "success");
            row.surface = surface.into();
            row.run_id = run.map(str::to_string);
            row.cost_usd = cost;
            row.usage_json = (!json.is_empty()).then(|| json.to_string());
            crate::session_store::upsert_usage(v, &row).unwrap();
        };
        // run-u1: its chat row and its consultation both have no cost.
        usage("u-1", "chat", Some("run-u1"), None, "");
        usage("u-1-ask", "delegate", Some("run-u1"), None, "");
        usage(
            "u-2",
            "chat",
            Some("run-u2"),
            Some(2.0),
            "{\"peakContextTokens\":50000}",
        );
        // Jev ranking requests: no run id, no cost.
        usage("u-jev1", "oneshot", None, None, "");
        usage("u-jev2", "oneshot", None, None, "");
        let peak = |n: u64| crate::usage::CallStats {
            peak_context_tokens: Some(n),
            ..Default::default()
        };
        let mut r1 = receipt_in("run-u1", "s-u", "2026-10-10T05:00:00Z");
        r1.api_usage = Some(peak(120_000));
        persist(v, r1);
        let mut r2 = receipt_in("run-u2", "s-u", "2026-10-10T05:30:00Z");
        r2.api_usage = Some(peak(70_000));
        persist(v, r2);
        let s = summary(v, morning());
        assert_eq!(s.chains.len(), 1);
        let c = &s.chains[0];
        assert_eq!((c.turns, c.unknown_cost, c.cost_usd), (2, 1, 2.0));
        assert_eq!(c.peak_context_tokens, Some(120_000));
        assert_eq!((s.unknown_cost, s.unknown_cost_calls), (1, 2));
    }

    /// S06 follow-up: a handover chain that started in the night is followed
    /// to its end (a successor that started after 7 AM included), but not
    /// past the next night's start; a chat that started after the night and
    /// a chain that started before it count only inside the night. `from` /
    /// `to` set an exact window.
    #[test]
    fn s06_a_chain_that_started_in_the_night_is_followed_to_its_end() {
        let dir = vault();
        let v = dir.path();
        for (key, run) in [
            ("s-l1", "run-l1"),
            ("s-l2", "run-l2"),
            ("s-l2", "run-l3"),
            ("s-day", "run-day"),
            ("s-old", "run-old"),
            ("s-old", "run-old2"),
        ] {
            start_turn(v, key, "claude", "/vault", run, "Work").unwrap();
        }
        for (key, at) in [
            ("s-l1", "2026-10-10T03:00:00+00:00"),
            ("s-l2", "2026-10-10T13:00:00+00:00"),
            ("s-day", "2026-10-10T12:30:00+00:00"),
            ("s-old", "2026-10-09T20:00:00+00:00"),
        ] {
            sql(
                v,
                &format!("UPDATE turns SET started_at = '{at}' WHERE session_key = '{key}'"),
            );
            sql(
                v,
                &format!("UPDATE stints SET started_at = '{at}' WHERE session_key = '{key}'"),
            );
        }
        let now = crate::session_store::db_time(Utc::now());
        let handover = "{\"kind\":\"childTask\",\"tool\":\"handover\",\"childSession\":\"s-l2\"}";
        crate::session_store::record_delegation(v, "run-l1", &now, handover, 10).unwrap();
        let usage = |key: &str, at: &str, run: Option<&str>, cost: Option<f64>| {
            let mut row = crate::session_store::tests::usage_row_fixture(key, at, "success");
            row.run_id = run.map(str::to_string);
            row.cost_usd = cost;
            crate::session_store::upsert_usage(v, &row).unwrap();
        };
        usage(
            "u-l1",
            "2026-10-10T05:00:00.000Z",
            Some("run-l1"),
            Some(2.0),
        );
        usage(
            "u-l2",
            "2026-10-10T15:00:00.000Z",
            Some("run-l2"),
            Some(3.0),
        );
        // After the next night began: not this night's.
        usage(
            "u-l3",
            "2026-10-10T23:00:00.000Z",
            Some("run-l3"),
            Some(11.0),
        );
        usage(
            "u-day",
            "2026-10-10T14:00:00.000Z",
            Some("run-day"),
            Some(5.0),
        );
        usage(
            "u-old",
            "2026-10-10T06:00:00.000Z",
            Some("run-old"),
            Some(1.0),
        );
        usage(
            "u-old2",
            "2026-10-10T13:00:00.000Z",
            Some("run-old2"),
            Some(7.0),
        );
        usage("u-jev", "2026-10-10T14:00:00.000Z", None, None);
        let merge = |key: &str, id: &str, n: u64, at: &str| {
            let ord: i64 = crate::session_store::with_read(v, |conn| {
                conn.query_row(
                    "SELECT MAX(ord) FROM turns WHERE session_key = ?1",
                    [key],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())
            })
            .unwrap();
            let cmd = format!("gh pr merge {n} -R o/r --merge");
            let input = serde_json::json!({ "command": cmd });
            let call = serde_json::json!({ "id": id, "name": "Bash", "input": input });
            append_event(v, key, ord, "toolUse", &call.to_string()).unwrap();
            let result = serde_json::json!({ "id": id, "content": "", "isError": false });
            append_event(v, key, ord, "toolResult", &result.to_string()).unwrap();
            sql(
                v,
                &format!("UPDATE events SET at = '{at}' WHERE payload LIKE '%\"{id}\"%'"),
            );
        };
        merge("s-l2", "m20", 20, "2026-10-10T14:30:00+00:00");
        merge("s-day", "m21", 21, "2026-10-10T14:00:00+00:00");
        persist(v, receipt_in("run-l2", "s-l2", "2026-10-10T14:00:00Z"));
        persist(v, receipt_in("run-day", "s-day", "2026-10-10T14:00:00Z"));
        // 8 PM ET: the next night (from 6 PM) has begun.
        let evening = DateTime::parse_from_rfc3339("2026-10-11T00:00:00Z")
            .unwrap()
            .with_timezone(&Local);
        let s = summary(v, evening);
        assert_eq!(s.night, "2026-10-09");
        let db = |s: &str| crate::session_store::db_time(local(s).with_timezone(&Utc));
        assert_eq!(s.from, db("2026-10-09 18:00"));
        assert_eq!(s.to, "2026-10-10T15:00:00.000Z", "the chain's last row");
        let chains: Vec<_> = s
            .chains
            .iter()
            .map(|c| {
                (
                    c.session_key.as_str(),
                    c.chats,
                    c.turns,
                    c.stints,
                    c.merged,
                    c.cost_usd,
                )
            })
            .collect();
        assert_eq!(
            chains,
            [("s-l1", 2, 2, 2, 1, 5.0), ("s-old", 1, 1, 0, 0, 1.0)]
        );
        assert_eq!((s.cost_usd, s.merged_prs, s.runs), (6.0, 1, 1));
        assert_eq!(
            s.unknown_cost_calls, 0,
            "a call after the night is no chain's"
        );
        // An exact window: everything in it, nothing followed past it.
        let w = summary_for(
            v,
            evening,
            Some("2026-10-10 09:30"),
            Some("2026-10-10T16:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            (w.from.clone(), w.to.as_str()),
            (db("2026-10-10 09:30"), "2026-10-10T16:00:00.000Z")
        );
        assert_eq!((w.cost_usd, w.merged_prs, w.runs), (8.0, 2, 2));
        assert_eq!(w.unknown_cost_calls, 1);
        assert!(summary_for(v, evening, Some("yesterday"), None).is_err());
        assert!(summary_for(v, evening, Some("2026-10-10"), Some("2026-10-09")).is_err());
        let early = summary_for(v, evening, None, Some("2026-10-01")).unwrap_err();
        assert!(
            early.starts_with("`to` must be after last night's start"),
            "{early}"
        );
        assert_eq!(
            summary_for(v, evening, Some("2026-10-10 21:00"), None).unwrap_err(),
            "`from` must be in the past."
        );
    }

    /// Overnight 10 A8 (13c): a `from` before noon is the morning after a
    /// night, so the window names the evening before it, as [`night_at`]
    /// does; a bare date is local midnight. From noon on, its own day.
    #[test]
    fn a_morning_from_names_the_evening_before() {
        let dir = vault();
        let v = dir.path();
        let now = local("2026-10-12 10:00");
        let night = |from: &str| summary_for(v, now, Some(from), None).unwrap().night;
        assert_eq!(night("2026-10-10 09:00"), "2026-10-09");
        assert_eq!(night("2026-10-10"), "2026-10-09");
        assert_eq!(night("2026-10-10 11:59"), "2026-10-09");
        assert_eq!(night("2026-10-10 12:00"), "2026-10-10");
        assert_eq!(night("2026-10-10 18:00"), "2026-10-10");
    }

    #[test]
    fn a_quiet_night_did_not_run_and_the_evenings_report_is_found_by_name() {
        let dir = vault();
        let v = dir.path();
        let s = summary(v, morning());
        assert!(!s.ran);
        assert_eq!(s.runs, 0);
        assert!(s.report.is_none());
        let night = v.join("workspaces/main/outputs/night");
        std::fs::create_dir_all(&night).unwrap();
        std::fs::write(night.join("2026-10-09.md"), "# Night").unwrap();
        std::fs::write(night.join("2026-10-01.md"), "# Older, touched later").unwrap();
        assert_eq!(
            summary(v, morning()).report.as_deref(),
            Some("workspaces/main/outputs/night/2026-10-09.md")
        );
    }
}
