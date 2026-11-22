//! What a conversation's context is spent on (repo features R6), for the
//! usage ring's Context section ("What's using it" and "Cache") and
//! `usage.conversation_context_use`. Sizes and counts only: never a result's
//! text, never a prompt. Read when the section opens, never on the
//! always-on path.
//!
//! **Window.** The current agent session: from the open stint's first turn
//! on (a new agent, a handover or a reset opens one), else every turn. A
//! stint's first turn is the one running when it started (a reset calls
//! `restart_stint` after `start_turn`, mid-turn), else the first
//! that started at or after it ([`first_of_stint`]).
//!
//! **Tool results.** The step spine's non-model steps with no
//! `parent_seq` (a subagent's results never enter the main context), sized
//! by the full result's UTF-8 bytes (`resultBytes`, written by
//! `step_spine::tool_finished` before its 300-character cut). Steps written
//! before that have a `result` but no `resultBytes`: their turns count as
//! unmeasured (`measuredTurns` against `windowTurns`). A turn that hit
//! `step_spine::MAX_STEPS` stopped recording, so the figures are a floor
//! (`atLeast`). The UI shows bytes/4 as an estimate of tokens.
//!
//! **Cache.** The conversation's last [`CACHE_TURNS`] turns, each from its
//! own `chat` usage rows (every attempt): `cachedShare` = cache reads ÷
//! (input + cache reads), where input already excludes cache reads and
//! includes cache writes (`usage.rs`). A turn is cold below [`COLD_SHARE`]
//! with at least [`COLD_MIN_INPUT`] input; its cause, first match wins:
//! `newSession` (a stint's first turn, as above), `idle` (more than an hour
//! after the previous turn ended: scorecard.mjs `gapBucket`'s `gt60m`),
//! `model` (the model changed), `compacted` (the previous turn's receipt
//! counted compactions), else `unknown`. `compactions` sums the window's
//! receipts.

use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Tools listed by name; the rest are summed as "other".
const BY_TOOL: usize = 6;
/// The largest single results listed.
const LARGEST: usize = 3;
/// The turns the Cache section lists.
const CACHE_TURNS: usize = 10;
/// Below this cached share a turn with enough input is cold.
const COLD_SHARE: f64 = 0.2;
/// The least input a cold turn has (a small fresh prompt isn't a miss).
const COLD_MIN_INPUT: u64 = 20_000;
/// A gap longer than this after the previous turn reads as idle.
const IDLE_MINUTES: i64 = 60;
/// A target is cut to this many characters.
const TARGET_CHARS: usize = 80;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextUse {
    /// The open stint's start (RFC 3339), or null when no stint is open.
    pub window_started_at: Option<String>,
    /// Turns in the window.
    pub window_turns: u32,
    /// Window turns whose tool results all carry a size.
    pub measured_turns: u32,
    /// A window turn stopped recording steps: the figures are a floor.
    pub at_least: bool,
    /// The [`BY_TOOL`] largest tools by bytes, then "other".
    pub by_tool: Vec<ToolBytes>,
    /// The [`LARGEST`] largest single results, largest first.
    pub largest: Vec<LargeResult>,
    /// The last [`CACHE_TURNS`] turns, oldest first.
    pub cache: Vec<CacheTurn>,
    /// Compactions the window's receipts counted.
    pub compactions: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolBytes {
    pub tool: String,
    pub bytes: u64,
    pub calls: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LargeResult {
    pub turn: i64,
    pub tool: String,
    pub bytes: u64,
    /// A file tool's file name or a shell call's program, from the step's
    /// input; null otherwise. Never result text.
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CacheTurn {
    pub turn: i64,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cached_share: Option<f64>,
    pub cold: bool,
    /// `newSession` | `idle` | `model` | `compacted` | `unknown`; cold
    /// turns only.
    pub cause: Option<&'static str>,
}

struct Turn {
    ord: i64,
    run_id: Option<String>,
    started: Option<DateTime<Utc>>,
    ended: Option<DateTime<Utc>>,
}

struct Usage {
    input: Option<u64>,
    cached: Option<u64>,
    model: Option<String>,
}

/// A step row: turn, seq, kind, name, parent_seq, checkpoint.
type StepRow = (
    i64,
    i64,
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
);

struct Measured {
    ord: i64,
    seq: i64,
    tool: String,
    bytes: u64,
    input: Option<String>,
    /// A write step's first recorded path.
    path: Option<String>,
}

fn time(raw: Option<String>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.as_deref()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn add(sum: Option<u64>, value: Option<i64>) -> Option<u64> {
    match value {
        Some(v) => Some(sum.unwrap_or(0).saturating_add(v.max(0) as u64)),
        None => sum,
    }
}

/// One conversation's context use (see the module doc).
pub(crate) fn for_session(vault: &Path, key: &str) -> Result<ContextUse, String> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    let (stints, turns, steps, usage) = crate::session_store::with_read(vault, |conn| {
        let fail = |what: &str| {
            let what = what.to_string();
            move |e: rusqlite::Error| format!("context use {what}: {e}")
        };
        let mut stmt = conn
            .prepare("SELECT started_at, ended_at IS NULL FROM stints WHERE session_key = ?1 ORDER BY id")
            .map_err(fail("stints"))?;
        let stints: Vec<(Option<DateTime<Utc>>, bool)> = stmt
            .query_map(params![key], |r| Ok((time(r.get(0)?), r.get(1)?)))
            .and_then(|rows| rows.collect())
            .map_err(fail("stints"))?;
        let mut stmt = conn
            .prepare("SELECT ord, run_id, started_at, ended_at FROM turns WHERE session_key = ?1 ORDER BY ord")
            .map_err(fail("turns"))?;
        let turns: Vec<Turn> = stmt
            .query_map(params![key], |r| {
                Ok(Turn {
                    ord: r.get(0)?,
                    run_id: r.get(1)?,
                    started: time(r.get(2)?),
                    ended: time(r.get(3)?),
                })
            })
            .and_then(|rows| rows.collect())
            .map_err(fail("turns"))?;
        let window_start = stints.iter().rev().find(|s| s.1).and_then(|s| s.0);
        let first = window_first(&turns, window_start).map_or(i64::MAX, |i| turns[i].ord);
        // Bounded by the steps primary key (session_key, turn_ord, …).
        let mut stmt = conn
            .prepare("SELECT turn_ord, seq, kind, name, parent_seq, checkpoint FROM steps WHERE session_key = ?1 AND turn_ord >= ?2 ORDER BY turn_ord, seq")
            .map_err(fail("steps"))?;
        let steps: Vec<StepRow> = stmt
            .query_map(params![key, first], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .and_then(|rows| rows.collect())
            .map_err(fail("steps"))?;
        // The cache turns and the one before the first of them (its model).
        let mut usage: HashMap<String, Usage> = HashMap::new();
        let mut stmt = conn
            .prepare("SELECT input_tokens, cached_input_tokens, COALESCE(model, requested_model) FROM usage_ledger WHERE run_id = ?1 AND surface = 'chat' ORDER BY id")
            .map_err(fail("usage"))?;
        for turn in turns.iter().rev().take(CACHE_TURNS + 1) {
            let Some(run) = turn.run_id.as_deref() else {
                continue;
            };
            let rows: Vec<(Option<i64>, Option<i64>, Option<String>)> = stmt
                .query_map(params![run], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .and_then(|rows| rows.collect())
                .map_err(fail("usage"))?;
            let mut u = Usage {
                input: None,
                cached: None,
                model: None,
            };
            for (input, cached, model) in rows {
                u.input = add(u.input, input);
                u.cached = add(u.cached, cached);
                if model.is_some() {
                    u.model = model;
                }
            }
            usage.insert(run.to_string(), u);
        }
        Ok((stints, turns, steps, usage))
    })?;

    let window_start = stints.iter().rev().find(|s| s.1).and_then(|s| s.0);
    let first = window_first(&turns, window_start);
    let window: Vec<&Turn> = turns
        .iter()
        .enumerate()
        .filter(|(i, t)| {
            first.is_some_and(|f| *i >= f) && (window_start.is_none() || t.started.is_some())
        })
        .map(|(_, t)| t)
        .collect();
    let window_ords: HashSet<i64> = window.iter().map(|t| t.ord).collect();

    // Tool results.
    let mut with_steps = HashSet::new();
    let mut unmeasured = HashSet::new();
    let mut at_least = false;
    let mut measured = Vec::new();
    for (ord, seq, kind, name, parent, checkpoint) in steps {
        if !window_ords.contains(&ord) {
            continue;
        }
        with_steps.insert(ord);
        at_least |= seq >= crate::step_spine::MAX_STEPS;
        if kind == "model" || parent.is_some() {
            continue;
        }
        let checkpoint: Value = checkpoint
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or(Value::Null);
        match checkpoint.get("resultBytes").and_then(Value::as_u64) {
            Some(bytes) => measured.push(Measured {
                ord,
                seq,
                tool: name
                    .or_else(|| {
                        checkpoint
                            .get("tool")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or(kind),
                bytes,
                input: checkpoint
                    .get("input")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                path: checkpoint
                    .pointer("/paths/0")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            }),
            // Finished before results were sized.
            None if checkpoint.get("result").is_some() => {
                unmeasured.insert(ord);
            }
            // Never finished (interrupted): no result entered the context.
            None => {}
        }
    }
    let measured_turns = window
        .iter()
        .filter(|t| with_steps.contains(&t.ord) && !unmeasured.contains(&t.ord))
        .count() as u32;

    let mut tools: HashMap<&str, (u64, u32)> = HashMap::new();
    for m in &measured {
        let e = tools.entry(m.tool.as_str()).or_default();
        e.0 = e.0.saturating_add(m.bytes);
        e.1 += 1;
    }
    let mut tools: Vec<ToolBytes> = tools
        .into_iter()
        .map(|(tool, (bytes, calls))| ToolBytes {
            tool: tool.to_string(),
            bytes,
            calls,
        })
        .collect();
    tools.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.tool.cmp(&b.tool)));
    if tools.len() > BY_TOOL {
        let rest = tools.split_off(BY_TOOL);
        tools.push(ToolBytes {
            tool: "other".into(),
            bytes: rest.iter().map(|t| t.bytes).sum(),
            calls: rest.iter().map(|t| t.calls).sum(),
        });
    }

    measured.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then(a.ord.cmp(&b.ord))
            .then(a.seq.cmp(&b.seq))
    });
    let largest = measured
        .iter()
        .take(LARGEST)
        .map(|m| LargeResult {
            turn: m.ord,
            tool: m.tool.clone(),
            bytes: m.bytes,
            target: target(&m.tool, m.input.as_deref().unwrap_or(""), m.path.as_deref()),
        })
        .collect();

    // Compactions, from the receipts of the window and of the cache turns.
    let tail = turns.len().saturating_sub(CACHE_TURNS + 1);
    let wanted: HashSet<&str> = window
        .iter()
        .copied()
        .chain(turns[tail..].iter())
        .filter_map(|t| t.run_id.as_deref())
        .collect();
    let compacted: HashMap<String, u32> = crate::evidence::with_receipts(vault, |receipts| {
        receipts
            .iter()
            .filter(|r| {
                r.compactions.unwrap_or(0) > 0 && wanted.contains(r.history_run_id.as_str())
            })
            .map(|r| (r.history_run_id.clone(), r.compactions.unwrap_or(0)))
            .collect()
    })
    .unwrap_or_default();
    let compactions_of = |t: &Turn| {
        t.run_id
            .as_deref()
            .and_then(|r| compacted.get(r).copied())
            .unwrap_or(0)
    };
    let compactions = window.iter().map(|t| compactions_of(t)).sum();

    // Cache.
    let stint_firsts: HashSet<usize> = stints
        .iter()
        .filter_map(|s| first_of_stint(&turns, s.0?))
        .collect();
    let start = turns.len().saturating_sub(CACHE_TURNS);
    let cache = (start..turns.len())
        .map(|i| {
            let turn = &turns[i];
            let prev = i.checked_sub(1).map(|p| &turns[p]);
            let none = Usage {
                input: None,
                cached: None,
                model: None,
            };
            let of = |t: &Turn| t.run_id.as_deref().and_then(|r| usage.get(r));
            let u = of(turn).unwrap_or(&none);
            let total = u.input.unwrap_or(0).saturating_add(u.cached.unwrap_or(0));
            let share = (total > 0 && (u.input.is_some() || u.cached.is_some()))
                .then(|| u.cached.unwrap_or(0) as f64 / total as f64);
            let cold =
                share.is_some_and(|s| s < COLD_SHARE) && u.input.unwrap_or(0) >= COLD_MIN_INPUT;
            let cause = cold.then(|| {
                let Some(prev) = prev else {
                    return "newSession";
                };
                if stint_firsts.contains(&i) {
                    return "newSession";
                }
                if let (Some(end), Some(start)) = (prev.ended, turn.started) {
                    if (start - end).num_seconds() > IDLE_MINUTES * 60 {
                        return "idle";
                    }
                }
                let prev_model = of(prev).and_then(|p| p.model.as_deref());
                if let (Some(a), Some(b)) = (prev_model, u.model.as_deref()) {
                    if a != b {
                        return "model";
                    }
                }
                if compactions_of(prev) > 0 {
                    return "compacted";
                }
                "unknown"
            });
            CacheTurn {
                turn: turn.ord,
                input_tokens: u.input,
                cached_input_tokens: u.cached,
                cached_share: share,
                cold,
                cause,
            }
        })
        .collect();

    Ok(ContextUse {
        window_started_at: window_start.map(|t| t.to_rfc3339()),
        window_turns: window.len() as u32,
        measured_turns,
        at_least,
        by_tool: tools,
        largest,
        cache,
        compactions,
    })
}

/// The index of the window's first turn: the open stint's first turn, or
/// the first turn when no stint is open.
fn window_first(turns: &[Turn], start: Option<DateTime<Utc>>) -> Option<usize> {
    match start {
        Some(s) => first_of_stint(turns, s),
        None => (!turns.is_empty()).then_some(0),
    }
}

/// The index of the first turn of the stint that started at `s`: a turn
/// that started at exactly `s` (`start_turn` opened it), else the turn
/// running then (the last to start before it, if it hadn't ended, since a
/// reset restarts the stint mid-turn), else the first turn that started at
/// or after it. A crashed turn keeps no end, so the turn just before a reset
/// counts even if a crash, not the reset, ended it: at most one extra turn.
fn first_of_stint(turns: &[Turn], s: DateTime<Utc>) -> Option<usize> {
    // `start_turn` opens a stint (an agent change) with the same timestamp
    // as its turn; that turn wins over an earlier one a crash left without
    // an end. `restart_stint` takes its own clock, so a reset never matches.
    if let Some(j) = turns.iter().position(|t| t.started == Some(s)) {
        return Some(j);
    }
    let before = turns.iter().rposition(|t| t.started.is_some_and(|t| t < s));
    if let Some(i) = before {
        if turns[i].ended.is_none_or(|e| e >= s) {
            return Some(i);
        }
    }
    let from = before.map_or(0, |i| i + 1);
    turns[from..]
        .iter()
        .position(|t| t.started.is_some_and(|t| t >= s))
        .map(|p| from + p)
}

/// What a large result was about, from the step's input (the tool input as
/// JSON text, cut at 300 characters): the file name for a file tool, the
/// program for a shell call, else none. A write step's `paths` (read from
/// its full input) come first: its `content` sorts before `file_path`, so
/// the cut drops the path.
fn target(tool: &str, input: &str, paths: Option<&str>) -> Option<String> {
    let parsed: Option<Value> = serde_json::from_str(input).ok();
    let short = tool.rsplit("__").next().unwrap_or(tool);
    let found = if matches!(
        short,
        "Read" | "read_source" | "Edit" | "MultiEdit" | "Write" | "NotebookEdit"
    ) {
        let path = match (paths, &parsed) {
            (Some(p), _) => Some(p.to_string()),
            (None, Some(v)) => crate::step_spine::named_paths(v).into_iter().next(),
            (None, None) => ["file_path", "path", "notebook_path"]
                .iter()
                .find_map(|k| closed_field(input, k)),
        }?;
        path.trim_end_matches('/').rsplit('/').next()?.to_string()
    } else {
        // A shell tool (the name alone decides).
        crate::run_effects::shell_command(tool, &Value::Null)?;
        let script = match &parsed {
            Some(v) => crate::run_effects::shell_command(tool, v)?,
            None => {
                crate::run_effects::unwrap_shell_wrapper(&crate::friction::shell_command(input)?)
            }
        };
        program(&script)?
    };
    let found = found.trim();
    (!found.is_empty()).then(|| found.chars().take(TARGET_CHARS).collect())
}

/// A string field of cut JSON text, only when its closing quote survived
/// (half a path names the wrong file).
fn closed_field(input: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\":\"");
    let rest = &input[input.find(&pattern)? + pattern.len()..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            other => out.push(other),
        }
    }
    None
}

/// A script's program word: the first simple command's, past assignments
/// and `cd` steps, without its folder.
fn program(script: &str) -> Option<String> {
    let segments: Vec<String> = match crate::run_effects::split_joined(script, true) {
        Some(segments) => segments.into_iter().map(|s| s.text).collect(),
        None => vec![script.to_string()],
    };
    segments.iter().find_map(|segment| {
        let words = crate::run_effects::shell_words(segment);
        let mut words = words
            .iter()
            .skip_while(|w| crate::evidence::is_assignment(w))
            .map(|w| w.rsplit('/').next().unwrap_or(w));
        let mut name = words.next()?;
        // An argv joined back into words (Codex's `["bash", "-lc", "git
        // status"]`): the wrapped script's program.
        if matches!(name, "bash" | "sh" | "zsh") {
            let mut rest = words.clone();
            if matches!(rest.next(), Some("-c" | "-lc")) {
                name = rest.next()?;
            }
        }
        (name != "cd" && !name.is_empty()).then(|| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step_spine::{tool_finished, tool_started, turn_started};
    use serde_json::json;

    /// A turn of `key` on `agent`, its times set to `start`..`end` (10-01
    /// clock times, "HH:MM").
    fn turn(v: &Path, key: &str, agent: &str, run: &str, start: &str, end: &str) -> i64 {
        let ord = crate::session_store::start_turn(v, key, agent, "/vault", run, "Do it").unwrap();
        crate::session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE turns SET started_at = ?3, ended_at = ?4 WHERE session_key = ?1 AND ord = ?2",
                params![key, ord, at(start), at(end)],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        turn_started(v, key, ord, run).unwrap();
        ord
    }

    fn at(hm: &str) -> String {
        format!("2026-10-01T{hm}:00+00:00")
    }

    /// The conversation's stints start at these times, in order.
    fn stints_at(v: &Path, key: &str, starts: &[&str]) {
        crate::session_store::with_write(v, "test", |tx| {
            let ids: Vec<i64> = tx
                .prepare("SELECT id FROM stints WHERE session_key = ?1 ORDER BY id")
                .unwrap()
                .query_map(params![key], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(ids.len(), starts.len());
            for (id, start) in ids.iter().zip(starts) {
                tx.execute(
                    "UPDATE stints SET started_at = ?2 WHERE id = ?1",
                    params![id, at(start)],
                )
                .unwrap();
            }
            Ok(())
        })
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn tool(
        v: &Path,
        key: &str,
        ord: i64,
        run: &str,
        id: &str,
        name: &str,
        input: Value,
        bytes: usize,
    ) {
        tool_started(v, key, ord, run, id, name, &input).unwrap();
        tool_finished(v, v, key, ord, run, id, &"x".repeat(bytes), false, None).unwrap();
    }

    fn usage(v: &Path, run: &str, model: &str, input: i64, cached: i64) {
        crate::session_store::with_write(v, "test", |tx| {
            tx.execute(
                "INSERT INTO usage_ledger(run_key, at, started_at, surface, run_id, model, billing, outcome, spawned, basis, input_tokens, cached_input_tokens, cost_status) \
                 VALUES(?1, ?2, ?2, 'chat', ?3, ?4, 'subscription', 'success', 1, 'per_run', ?5, ?6, 'unknown')",
                params![format!("k-{run}"), at("09:00"), run, model, input, cached],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    }

    fn compacted(v: &Path, run: &str, n: u32) {
        crate::evidence::persist_for_test(
            v,
            crate::evidence::EvidenceReceipt {
                id: format!("r-{run}"),
                run_id: run.into(),
                history_run_id: run.into(),
                source: "chat".into(),
                finished_at: at("09:00"),
                compactions: Some(n),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn the_window_is_the_open_stint_and_subagent_results_stay_out() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-ctxuse-1";
        let t1 = turn(v, key, "claude", "run-1", "10:00", "10:01");
        tool(
            v,
            key,
            t1,
            "run-1",
            "a",
            "Read",
            json!({"file_path": "/w/old.md"}),
            50_000,
        );
        crate::session_store::restart_stint(v, key, "claude", "handover").unwrap();
        let t2 = turn(v, key, "claude", "run-2", "10:31", "10:40");
        stints_at(v, key, &["10:00", "10:30"]);
        // A subagent's Read nests in its Task: only the Task's own result
        // reaches the main context.
        tool_started(
            v,
            key,
            t2,
            "run-2",
            "task",
            "Task",
            &json!({"prompt": "look"}),
        )
        .unwrap();
        tool(
            v,
            key,
            t2,
            "run-2",
            "sub",
            "Read",
            json!({"file_path": "/w/sub.md"}),
            90_000,
        );
        tool_finished(
            v,
            v,
            key,
            t2,
            "run-2",
            "task",
            &"y".repeat(100),
            false,
            None,
        )
        .unwrap();
        tool(
            v,
            key,
            t2,
            "run-2",
            "b",
            "Read",
            json!({"file_path": "/w/small.md"}),
            300,
        );
        tool(
            v,
            key,
            t2,
            "run-2",
            "c",
            "Bash",
            json!({"command": "cd /w && LC_ALL=C cargo test --lib"}),
            4_000,
        );
        tool(
            v,
            key,
            t2,
            "run-2",
            "d",
            "Read",
            json!({"file_path": "/w/notes/big.md"}),
            7_000,
        );
        // A Write whose stored input the 300-character cut leaves without
        // its path (`content` sorts first): the step's `paths` name it.
        tool(
            v,
            key,
            t2,
            "run-2",
            "e",
            "Write",
            json!({"file_path": "/w/out.md", "content": "z".repeat(400)}),
            5_000,
        );

        let used = for_session(v, key).unwrap();
        assert_eq!(
            used.window_started_at.as_deref(),
            Some("2026-10-01T10:30:00+00:00")
        );
        assert_eq!(used.window_turns, 1);
        assert_eq!(used.measured_turns, 1);
        assert!(!used.at_least);
        let by: Vec<(&str, u64, u32)> = used
            .by_tool
            .iter()
            .map(|t| (t.tool.as_str(), t.bytes, t.calls))
            .collect();
        assert_eq!(
            by,
            vec![
                ("Read", 7_300, 2),
                ("Write", 5_000, 1),
                ("Bash", 4_000, 1),
                ("Task", 100, 1)
            ]
        );
        let largest: Vec<(i64, &str, u64, Option<&str>)> = used
            .largest
            .iter()
            .map(|l| (l.turn, l.tool.as_str(), l.bytes, l.target.as_deref()))
            .collect();
        assert_eq!(
            largest,
            vec![
                (t2, "Read", 7_000, Some("big.md")),
                (t2, "Write", 5_000, Some("out.md")),
                (t2, "Bash", 4_000, Some("cargo"))
            ]
        );
    }

    /// Production order: a reset (`chat::announce_reset`) calls
    /// `restart_stint` after `start_turn`, so the turn that opened the stint
    /// started before it.
    #[test]
    fn the_turn_running_when_a_stint_opens_is_its_first() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-ctxuse-6";
        let opus = "claude-opus-5-5";
        let t1 = turn(v, key, "claude", "run-1", "10:00", "10:01");
        usage(v, "run-1", opus, 1_000, 50_000);
        tool(
            v,
            key,
            t1,
            "run-1",
            "a",
            "Read",
            json!({"file_path": "/w/old.md"}),
            50_000,
        );
        // Turn 2 starts at 10:30, then hands over at 10:35 and ends at 10:40.
        let t2 = turn(v, key, "claude", "run-2", "10:30", "10:40");
        crate::session_store::restart_stint(v, key, "claude", "handover").unwrap();
        usage(v, "run-2", opus, 60_000, 0);
        tool(
            v,
            key,
            t2,
            "run-2",
            "b",
            "Read",
            json!({"file_path": "/w/big.md"}),
            40_000,
        );
        // Turn 3: cold with nothing to blame.
        let t3 = turn(v, key, "claude", "run-3", "10:41", "10:42");
        usage(v, "run-3", opus, 30_000, 0);
        tool(
            v,
            key,
            t3,
            "run-3",
            "c",
            "Bash",
            json!({"command": "ls"}),
            2_000,
        );
        stints_at(v, key, &["10:00", "10:35"]);

        let used = for_session(v, key).unwrap();
        assert_eq!(used.window_turns, 2, "turns 2 and 3");
        let by: Vec<(&str, u64)> = used
            .by_tool
            .iter()
            .map(|t| (t.tool.as_str(), t.bytes))
            .collect();
        assert_eq!(by, vec![("Read", 40_000), ("Bash", 2_000)]);
        let causes: Vec<(i64, Option<&str>)> =
            used.cache.iter().map(|c| (c.turn, c.cause)).collect();
        assert_eq!(
            causes,
            vec![(1, None), (2, Some("newSession")), (3, Some("unknown"))]
        );

        // Turn 4 is still running when a reset opens a third stint.
        let t4 = turn(v, key, "claude", "run-4", "10:50", "10:51");
        crate::session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE turns SET ended_at = NULL WHERE session_key = ?1 AND ord = ?2",
                params![key, t4],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        crate::session_store::restart_stint(v, key, "claude", "reset").unwrap();
        usage(v, "run-4", opus, 45_000, 0);
        stints_at(v, key, &["10:00", "10:35", "10:55"]);
        let used = for_session(v, key).unwrap();
        assert_eq!(used.window_turns, 1, "turn 4 only");
        assert_eq!(
            used.cache.last().map(|c| (c.turn, c.cause)),
            Some((t4, Some("newSession")))
        );
    }

    /// An agent change: `start_turn` opens the stint with its turn's own
    /// timestamp, while an earlier turn a crash killed still has no end.
    #[test]
    fn a_crashed_turn_never_opens_the_next_agents_stint() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-ctxuse-7";
        let model = "gpt-5";
        let t1 = turn(v, key, "claude", "run-1", "10:00", "10:01");
        usage(v, "run-1", model, 1_000, 50_000);
        crate::session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE turns SET ended_at = NULL WHERE session_key = ?1 AND ord = ?2",
                params![key, t1],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        let t2 = turn(v, key, "codex", "run-2", "10:30", "10:31");
        usage(v, "run-2", model, 50_000, 0);
        stints_at(v, key, &["10:00", "10:30"]);

        let used = for_session(v, key).unwrap();
        assert_eq!(used.window_turns, 1, "turn 2 only");
        assert_eq!(
            used.cache.last().map(|c| (c.turn, c.cause)),
            Some((t2, Some("newSession")))
        );
    }

    #[test]
    fn targets_name_a_file_or_a_program_never_more() {
        assert_eq!(
            target("Read", r#"{"file_path":"/w/a/b.md","limit":10}"#, None),
            Some("b.md".into())
        );
        assert_eq!(
            target(
                "mcp__hollow_vault__read_source",
                r#"{"path":"sources/a b.md"}"#,
                None
            ),
            Some("a b.md".into())
        );
        // Cut JSON: a path whose closing quote survived, a command's program.
        assert_eq!(
            target("Read", r#"{"file_path":"/w/c.md","offset":1…"#, None),
            Some("c.md".into())
        );
        assert_eq!(
            target("Read", r#"{"file_path":"/w/very-long-na…"#, None),
            None
        );
        assert_eq!(
            target(
                "Bash",
                r#"{"command":"cd /w && npm run gate 2>&1 | tail -40 && echo do…"#,
                None
            ),
            Some("npm".into())
        );
        assert_eq!(
            target(
                "exec_command",
                r#"{"cmd":["bash","-lc","/usr/bin/git status"]}"#,
                None
            ),
            Some("git".into())
        );
        assert_eq!(target("Grep", r#"{"pattern":"secret"}"#, None), None);
        assert_eq!(target("Bash", r#"{"command":""}"#, None), None);
    }

    #[test]
    fn more_than_six_tools_fold_into_other() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-ctxuse-2";
        let t1 = turn(v, key, "codex", "run-1", "10:00", "10:01");
        for (i, name) in ["t1", "t2", "t3", "t4", "t5", "t6", "t7", "t8"]
            .iter()
            .enumerate()
        {
            tool(v, key, t1, "run-1", name, name, json!({}), 1_000 - i * 10);
        }
        stints_at(v, key, &["10:00"]);
        let used = for_session(v, key).unwrap();
        assert_eq!(used.by_tool.len(), 7);
        let other = used.by_tool.last().unwrap();
        assert_eq!(
            (other.tool.as_str(), other.bytes, other.calls),
            ("other", 930 + 940, 2)
        );
    }

    #[test]
    fn older_steps_count_as_unmeasured() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-ctxuse-3";
        // A turn from before the step spine: no steps at all.
        let t1 =
            crate::session_store::start_turn(v, key, "claude", "/vault", "run-1", "Hi").unwrap();
        // A turn whose result was recorded before results were sized.
        let t2 = turn(v, key, "claude", "run-2", "10:10", "10:11");
        tool(
            v,
            key,
            t2,
            "run-2",
            "a",
            "Read",
            json!({"file_path": "/w/a.md"}),
            800,
        );
        crate::session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE steps SET checkpoint = json_remove(checkpoint, '$.resultBytes') WHERE session_key = ?1 AND turn_ord = ?2",
                params![key, t2],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        // A turn with a result that never finished, and one measured.
        let t3 = turn(v, key, "claude", "run-3", "10:20", "10:21");
        tool_started(
            v,
            key,
            t3,
            "run-3",
            "b",
            "Bash",
            &json!({"command": "sleep 9"}),
        )
        .unwrap();
        tool(
            v,
            key,
            t3,
            "run-3",
            "c",
            "Read",
            json!({"file_path": "/w/c.md"}),
            500,
        );
        crate::session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE turns SET started_at = ?2 WHERE session_key = ?1 AND ord = ?3",
                params![key, at("10:00"), t1],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        stints_at(v, key, &["10:00"]);
        let used = for_session(v, key).unwrap();
        assert_eq!(used.window_turns, 3);
        assert_eq!(used.measured_turns, 1, "only the third turn is measured");
        assert_eq!(
            used.by_tool,
            vec![ToolBytes {
                tool: "Read".into(),
                bytes: 500,
                calls: 1
            }]
        );
    }

    #[test]
    fn cold_turns_say_why() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-ctxuse-4";
        let opus = "claude-opus-5-5";
        // 1: the stint's first turn, cold.
        turn(v, key, "claude", "run-1", "10:00", "10:05");
        usage(v, "run-1", opus, 50_000, 0);
        // 2: warm.
        turn(v, key, "claude", "run-2", "10:06", "10:10");
        usage(v, "run-2", opus, 1_000, 50_000);
        // 3: 61 minutes after turn 2 ended.
        turn(v, key, "claude", "run-3", "11:11", "11:12");
        usage(v, "run-3", opus, 60_000, 1_000);
        // 4: another model.
        turn(v, key, "claude", "run-4", "11:13", "11:14");
        usage(v, "run-4", "claude-sonnet-5", 40_000, 2_000);
        compacted(v, "run-4", 1);
        // 5: after turn 4 compacted.
        turn(v, key, "claude", "run-5", "11:15", "11:16");
        usage(v, "run-5", "claude-sonnet-5", 30_000, 0);
        // 6: cold with nothing to blame; 7: small, so not cold.
        turn(v, key, "claude", "run-6", "11:17", "11:18");
        usage(v, "run-6", "claude-sonnet-5", 30_000, 0);
        turn(v, key, "claude", "run-7", "11:19", "11:20");
        usage(v, "run-7", "claude-sonnet-5", 5_000, 0);
        // 8: a new stint (a reset), cold.
        crate::session_store::restart_stint(v, key, "claude", "reset").unwrap();
        turn(v, key, "claude", "run-8", "11:31", "11:32");
        usage(v, "run-8", "claude-sonnet-5", 45_000, 0);
        compacted(v, "run-8", 2);
        stints_at(v, key, &["10:00", "11:30"]);

        let used = for_session(v, key).unwrap();
        let causes: Vec<(i64, bool, Option<&str>)> = used
            .cache
            .iter()
            .map(|c| (c.turn, c.cold, c.cause))
            .collect();
        assert_eq!(
            causes,
            vec![
                (1, true, Some("newSession")),
                (2, false, None),
                (3, true, Some("idle")),
                (4, true, Some("model")),
                (5, true, Some("compacted")),
                (6, true, Some("unknown")),
                (7, false, None),
                (8, true, Some("newSession")),
            ]
        );
        let warm = &used.cache[1];
        assert!((warm.cached_share.unwrap() - 50.0 / 51.0).abs() < 1e-9);
        assert_eq!(
            used.compactions, 2,
            "the window is the open stint: turn 8 only"
        );
        assert_eq!(used.window_turns, 1);
    }

    #[test]
    fn the_cache_lists_the_last_ten_turns() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = "s-ctxuse-5";
        for i in 0..12 {
            let run = format!("run-{i}");
            turn(
                v,
                key,
                "codex",
                &run,
                &format!("10:{:02}", i * 2),
                &format!("10:{:02}", i * 2 + 1),
            );
        }
        stints_at(v, key, &["10:00"]);
        let used = for_session(v, key).unwrap();
        assert_eq!(
            used.cache.iter().map(|c| c.turn).collect::<Vec<_>>(),
            (3..=12).collect::<Vec<_>>()
        );
        assert!(used
            .cache
            .iter()
            .all(|c| c.cached_share.is_none() && !c.cold));
    }

    #[test]
    fn an_unsafe_key_is_refused() {
        let vault = tempfile::tempdir().unwrap();
        assert!(for_session(vault.path(), "../x").is_err());
    }
}
