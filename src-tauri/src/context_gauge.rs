//! The live context gauge (OB11): how full each running chat turn's own
//! context window is, so `sessions.context` can tell the agent before it
//! decides when to hand over. Handover lanes handed over at a median 615K
//! tokens because the trigger was a feeling; Hollow already knew the figure
//! ([`crate::usage::UsageAccumulator::context_tokens`]) and never said it.
//!
//! **Registry.** `chat::drive_turn_attempts` registers its run by history
//! run id ([`register`]) and publishes a reading after every usage event its
//! ledger folds in ([`GaugeGuard::publish_run`]); the guard removes the entry
//! when it drops, so every return (a stop, an error, a failed spawn, a
//! panic's unwind) leaves nothing behind. Readings, not the accumulator: the
//! turn's [`crate::usage_ledger::UsageRun`] owns that, and a failover swaps
//! it for a fresh one, whose first message then publishes again.
//!
//! **Window.** Inferred from the model id the way the composer's context
//! ring does (`src/chatUsage.ts` `contextWindow`): Claude 1M for a `[1m]`
//! model or once the context already passed 200K (only a 1M session can),
//! else 200K; a self-hosted model's window is known exactly (Q3,
//! `self_hosted::remember`) and treated like a reported one. Codex says it
//! itself (overnight 9 A10): its rollout file's
//! `model_context_window` and the last call's prompt tokens
//! (`codex_rollout`), published with [`GaugeGuard::publish_figure`]; other
//! agents don't say. The advisory ([`ADVISORY_TOKENS`]) is offered on a 1M
//! window only. Read-only: nothing here limits a turn.
//!
//! **Growth steps** (repo features R2a). The gauge also tells a turn, once
//! per step, how full it is: 300K, 350K (the advisory), 400K, then every
//! 100K ([`step_of`]); on a window under 1M that the agent reported itself
//! (Codex's 258K), 70% and 85% of it instead ([`step_for`], A10).
//! [`take_note`] hands the broker a short line
//! ([`note_line`]) when the context passed a step the conversation wasn't
//! told about yet, and once after each compaction the stream reported, a
//! pointer at `sessions.task_state` ([`COMPACTED_NOTE`], R1's in-turn
//! half). The broker sends it as a second text block of the next
//! `hollow_app` tool result (`app_broker::answered`), so it costs
//! tool-result bytes only, and only past a step. The told step is the
//! conversation's `derived_state` row of kind [`STEPS_KIND`]
//! (`{announced}`): the broker stores it, and [`register`] seeds the next
//! turn with it ([`announced_step`]), so a resumed turn already past 300K
//! doesn't announce its step again on its first call. A reading lowers it
//! to its own step ([`GaugeGuard::publish_run`]), which re-arms the steps
//! after a compaction. Quiet turns (plan mode, a bare scratch turn, an
//! unattended one) get no note.
//!
//! **The stored figure** (repo features R2b). A live reading dies with its
//! turn, so the end of every chat turn writes its last context figure into
//! the same row ([`store_figure`]: `{tokens, window, announced}`, plus
//! `reported: true` for a window the agent reported, A10; the told
//! step lowered to the figure's own). The next resumed turn's message then
//! starts with a `<hollow-context>` line when that figure passed a step the
//! conversation wasn't told ([`start_note`], which stores the step before
//! the turn registers, so the in-turn channel starts from it: one
//! announcement per step across both channels). That is how an agent
//! without `hollow_app` (Grok) hears of a step, and how a turn that ended
//! just past one does. A fresh native session starts from nothing
//! ([`reset_steps`]).

use crate::agent::AgentKind;
use crate::usage::CallStats;
#[cfg(test)]
use crate::usage::UsageAccumulator;
use crate::usage_ledger::UsageRun;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A 1M-context model's window.
pub const ONE_MILLION: u64 = 1_000_000;
/// A Claude model's window without the `[1m]` suffix.
const CLAUDE_WINDOW: u64 = 200_000;
/// Where a 1M-context turn should hand over (`sessions.handover`) rather
/// than run on: well before the window, where long sessions slow and cost.
pub const ADVISORY_TOKENS: u64 = 350_000;
/// The first growth step: below it a turn is told nothing.
const FIRST_STEP: u64 = 300_000;
/// Past the advisory, a step every this many tokens (400K, 500K, ...).
const STEP_EVERY: u64 = 100_000;

/// The `derived_state` kind of a conversation's told growth step.
pub(crate) const STEPS_KIND: &str = "contextSteps";

/// R1's in-turn half: what the next `hollow_app` result says once after a
/// compaction the turn's stream reported.
pub const COMPACTED_NOTE: &str = "[Hollow: your context was just compacted; run_action sessions.task_state returns this conversation's goal, rules and finished steps.]";

/// One live turn's latest reading.
#[derive(Debug, Clone)]
struct Reading {
    /// Which registration wrote it, so a stale guard never removes a newer one.
    token: u64,
    agent: AgentKind,
    model: Option<String>,
    context_tokens: Option<u64>,
    /// The window the agent reported itself (Codex, A10); else inferred.
    window: Option<u64>,
    calls: CallStats,
    /// No note ever (plan mode, bare, unattended).
    quiet: bool,
    /// The highest step the conversation was told about and still holds.
    announced: u64,
    /// Compactions the turn's stream reported, and how many were pointed out.
    compactions: u32,
    compactions_noted: u32,
}

impl Reading {
    /// The window: what the agent reported, else what its model implies.
    fn window_now(&self) -> Option<u64> {
        self.window
            .or_else(|| context_window(self.agent, self.model.as_deref(), self.context_tokens))
    }
}

/// Live turns by history run id.
static LIVE: Mutex<BTreeMap<String, Reading>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn live() -> std::sync::MutexGuard<'static, BTreeMap<String, Reading>> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// A registered turn; dropping it removes the entry.
#[must_use = "the gauge entry is removed when the guard drops"]
pub struct GaugeGuard {
    run_id: String,
    token: u64,
}

/// Register a live turn with no reading yet. `model` is the id it asked
/// for (`None` for the agent's default). A `quiet` turn is never noted;
/// `announced` is the step its conversation was already told
/// ([`announced_step`]), so the turn starts from it.
pub fn register(
    run_id: &str,
    agent: AgentKind,
    model: Option<&str>,
    quiet: bool,
    announced: u64,
) -> GaugeGuard {
    let token = NEXT.fetch_add(1, Ordering::Relaxed);
    let reading = Reading {
        token,
        agent,
        model: model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
        context_tokens: None,
        // A self-hosted model's window is exact (Q3), like a reported one.
        window: exact_window(agent, model),
        calls: CallStats::default(),
        quiet,
        announced,
        compactions: 0,
        compactions_noted: 0,
    };
    ended().remove(run_id);
    live().insert(run_id.to_string(), reading);
    GaugeGuard {
        run_id: run_id.to_string(),
        token,
    }
}

impl GaugeGuard {
    /// Replace the reading with what the turn's `run` holds now: the main
    /// thread's context (a subagent's messages never move it) and the calls
    /// so far, corrected as its ledger row's are (overnight 10 A8:
    /// [`UsageRun`]'s `call_stats`, so a resumed Claude turn's
    /// session-cumulative `apiMs` is dropped, not shown raw). The told step
    /// drops to the reading's own, so a context that shrank (a compaction)
    /// is told each step again as it grows back.
    pub fn publish_run(&self, run: &UsageRun) {
        self.publish_parts(run.accumulator().context_tokens(), run.call_stats());
    }

    /// Tests: a bare accumulator's figures, its calls as summed.
    #[cfg(test)]
    pub fn publish(&self, acc: &UsageAccumulator) {
        self.publish_parts(acc.context_tokens(), acc.call_stats());
    }

    fn publish_parts(&self, context_tokens: Option<u64>, calls: CallStats) {
        self.with_reading(|reading| {
            reading.context_tokens = context_tokens;
            reading.calls = calls;
            if let Some(tokens) = context_tokens {
                reading.announced = reading.announced.min(step_for(tokens, reading.window));
            }
        });
    }

    /// Overnight 9 A10: a figure the agent reported itself (Codex's rollout:
    /// the last call's prompt tokens and the model's window), for an agent
    /// whose stream carries no per-call usage. Lowers the told step as
    /// [`Self::publish_run`] does.
    pub fn publish_figure(&self, tokens: u64, window: Option<u64>) {
        self.with_reading(|reading| {
            reading.context_tokens = Some(tokens);
            reading.window = window.or(reading.window);
            reading.announced = reading.announced.min(step_for(tokens, reading.window));
        });
    }

    /// The compactions the turn's stream reported so far
    /// (`ParseState::compactions`): each new one is pointed out once.
    pub fn set_compactions(&self, compactions: u32) {
        self.with_reading(|reading| reading.compactions = compactions);
    }

    /// R2b review: a failed resume's fresh retry starts a new session that
    /// was told nothing, so its steps start again from 0 (the stored row is
    /// cleared beside it, `reset_steps`).
    pub fn reset_announced(&self) {
        self.with_reading(|reading| reading.announced = 0);
    }

    fn with_reading(&self, f: impl FnOnce(&mut Reading)) {
        if let Some(reading) = live().get_mut(&self.run_id) {
            if reading.token == self.token {
                f(reading);
            }
        }
    }
}

impl Drop for GaugeGuard {
    fn drop(&mut self) {
        let removed = {
            let mut live = live();
            if live.get(&self.run_id).map(|r| r.token) == Some(self.token) {
                live.remove(&self.run_id)
            } else {
                None
            }
        };
        // R2b review: the step the turn ended holding, for its figure.
        if let Some(reading) = removed {
            ended().insert(self.run_id.clone(), reading.announced);
        }
    }
}

/// R2b review: the step each ended turn's reading held as told, by run id,
/// until [`store_figure`] takes it. The reading is what knows a compaction
/// lowered it (the row learns only from a later note), so the stored
/// figure lowers from it, not from the row.
static ENDED: Mutex<BTreeMap<String, u64>> = Mutex::new(BTreeMap::new());

fn ended() -> std::sync::MutexGuard<'static, BTreeMap<String, u64>> {
    ENDED.lock().unwrap_or_else(|e| e.into_inner())
}

/// The model's context window in tokens, or `None` when Hollow can't tell
/// (see the module doc).
pub fn context_window(agent: AgentKind, model: Option<&str>, used: Option<u64>) -> Option<u64> {
    if agent != AgentKind::Claude {
        return None;
    }
    let id = model.unwrap_or_default().to_ascii_lowercase();
    // A self-hosted model's own window (Q3), exact.
    if let Some(window) = crate::self_hosted::known_window(&id) {
        return Some(window);
    }
    if id.contains("[1m]") || used.is_some_and(|u| u > CLAUDE_WINDOW) {
        Some(ONE_MILLION)
    } else {
        Some(CLAUDE_WINDOW)
    }
}

/// Q3: a window known exactly rather than inferred: a self-hosted model's
/// (`self_hosted::remember`). Treated like a window the agent reported
/// (Codex's): its growth steps are 70% and 85% of it.
pub fn exact_window(agent: AgentKind, model: Option<&str>) -> Option<u64> {
    (agent == AgentKind::Claude)
        .then(|| model.and_then(crate::self_hosted::known_window))
        .flatten()
}

/// The advisory hand-over point for `window`: only a 1M window has one.
pub fn advisory(window: Option<u64>) -> Option<u64> {
    (window == Some(ONE_MILLION)).then_some(ADVISORY_TOKENS)
}

/// The growth step `tokens` has reached: the highest of 300K, 350K, 400K,
/// 500K, 600K, ... at or below it, or 0 below 300K.
pub fn step_of(tokens: u64) -> u64 {
    if tokens < FIRST_STEP {
        0
    } else if tokens < ADVISORY_TOKENS {
        FIRST_STEP
    } else if tokens < ADVISORY_TOKENS + STEP_EVERY / 2 {
        ADVISORY_TOKENS
    } else {
        tokens / STEP_EVERY * STEP_EVERY
    }
}

/// Overnight 9 A10: the growth step for `tokens` in `window`, a window the
/// agent reported itself (Codex's), or `None`. One under 1M never reaches 300K, so its
/// steps are 70% and 85% of it; a 1M window or none keeps [`step_of`]'s.
/// Claude's window is inferred from its model id, which can't tell a 200K
/// session from a 1M one (an alias, the CLI's default), so Claude passes
/// none and keeps R2a's steps (review medium).
pub fn step_for(tokens: u64, window: Option<u64>) -> u64 {
    match window {
        Some(window) if window > 0 && window < ONE_MILLION => {
            let (first, second) = (window * 70 / 100, window * 85 / 100);
            if tokens >= second {
                second
            } else if tokens >= first {
                first
            } else {
                0
            }
        }
        _ => step_of(tokens),
    }
}

/// `412K`, `1M`: a token count as the note says it.
fn short(tokens: u64) -> String {
    if tokens >= ONE_MILLION && tokens.is_multiple_of(ONE_MILLION) {
        format!("{}M", tokens / ONE_MILLION)
    } else {
        format!("{}K", tokens / 1_000)
    }
}

/// `412K of 1M`, or `412K` when the window is unknown.
fn figure(tokens: u64, window: Option<u64>) -> String {
    let of = window
        .map(|w| format!(" of {}", short(w)))
        .unwrap_or_default();
    format!("{}{of}", short(tokens))
}

/// The step note (at most 200 bytes): how full the conversation's context
/// is, of the window when Hollow knows it, and what to do. Addressed to the
/// conversation's main thread: a Claude Code subagent shares the parent's
/// MCP connection, so its `hollow_app` result may be the one that carries
/// the note, and it must not hand the parent's work over.
pub fn note_line(tokens: u64, window: Option<u64>) -> String {
    format!(
        "[Hollow: this conversation's context is at {}. Main thread on a long job: finish the step and hand over (sessions.handover). A subagent: just mention it in your result.]",
        figure(tokens, window),
    )
}

/// R2b: the start-of-turn line, `<hollow-context>` around the step note
/// without its subagent clause (only the main thread reads a turn's
/// message), with the blank line the wrapper's blocks end with. At most
/// 220 bytes.
pub fn start_line(tokens: u64, window: Option<u64>) -> String {
    format!(
        "<hollow-context>This conversation's context is at {}. On a long job, finish the step and hand over (sessions.handover).</hollow-context>\n\n",
        figure(tokens, window),
    )
}

/// What the calling run's next `hollow_app` result should add, if
/// anything: the compaction pointer once per new compaction, and the step
/// note when the context passed a step the conversation wasn't told about.
/// With it, the step now told (for [`store_announced`]). `None` for a
/// quiet turn, a run with no live turn, or nothing new.
pub fn take_note(run_id: &str) -> Option<(String, u64)> {
    let mut live = live();
    let reading = live.get_mut(run_id)?;
    if reading.quiet {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    if reading.compactions > reading.compactions_noted {
        reading.compactions_noted = reading.compactions;
        lines.push(COMPACTED_NOTE.to_string());
    }
    if let Some(tokens) = reading.context_tokens {
        let step = step_for(tokens, reading.window);
        if step > reading.announced {
            reading.announced = step;
            lines.push(note_line(tokens, reading.window_now()));
        }
    }
    (!lines.is_empty()).then(|| (lines.join("\n"), reading.announced))
}

/// The step conversation `key` was last told about (its [`STEPS_KIND`]
/// row's `announced`), or 0.
pub fn announced_step(vault: &Path, key: &str) -> u64 {
    crate::session_store::derived_state(vault, key, STEPS_KIND)
        .ok()
        .flatten()
        .and_then(|row| serde_json::from_str::<Value>(&row.payload).ok())
        .and_then(|payload| payload.get("announced").and_then(Value::as_u64))
        .unwrap_or(0)
}

/// Overnight 9 A9: conversation `key`'s stored context figure in tokens
/// ([`store_figure`]), or `None` (no row, or reset since by a fresh session).
pub fn stored_tokens(vault: &Path, key: &str) -> Option<u64> {
    steps_row(vault, key)
        .1
        .get("tokens")
        .and_then(Value::as_u64)
}

/// K3: conversation `key`'s stored figure as [`step_for`] takes it: the
/// tokens, and the window only when the agent reported it (Codex's).
pub fn stored_figure(vault: &Path, key: &str) -> Option<(u64, Option<u64>)> {
    let payload = steps_row(vault, key).1;
    let tokens = payload.get("tokens").and_then(Value::as_u64)?;
    let reported = payload.get("reported").and_then(Value::as_bool) == Some(true);
    let window = payload
        .get("window")
        .and_then(Value::as_u64)
        .filter(|_| reported);
    Some((tokens, window))
}

/// Store `announced` as the step conversation `key` was told about,
/// keeping the row's other fields. Best effort: a failed write is logged,
/// and the next turn may then repeat the step once.
pub fn store_announced(vault: &Path, key: &str, announced: u64) {
    let (row, mut payload) = steps_row(vault, key);
    payload.insert("announced".into(), json!(announced));
    put_steps(vault, key, row.as_ref(), payload);
}

/// Conversation `key`'s [`STEPS_KIND`] row and its payload's fields (none
/// when there is no row or it doesn't parse).
fn steps_row(
    vault: &Path,
    key: &str,
) -> (Option<crate::session_store::DerivedRow>, Map<String, Value>) {
    let row = crate::session_store::derived_state(vault, key, STEPS_KIND)
        .ok()
        .flatten();
    let payload = row
        .as_ref()
        .and_then(|row| serde_json::from_str::<Map<String, Value>>(&row.payload).ok())
        .unwrap_or_default();
    (row, payload)
}

/// Write `payload` over `row` (the next revision). Best effort: a failed
/// write is logged.
fn put_steps(
    vault: &Path,
    key: &str,
    row: Option<&crate::session_store::DerivedRow>,
    payload: Map<String, Value>,
) {
    let written = crate::session_store::put_derived_state(
        vault,
        key,
        STEPS_KIND,
        &crate::session_store::DerivedRow {
            revision: row.map_or(0, |row| row.revision) + 1,
            through_ord: row.map_or(0, |row| row.through_ord),
            payload: Value::Object(payload).to_string(),
        },
    );
    if let Err(error) = written {
        eprintln!("[context] the conversation's growth step could not be stored: {error}");
    }
}

/// R2b: store run `run_id`'s last context figure (`tokens`, of `window`
/// when known; `None` for an agent that reports none, which stores
/// nothing) for conversation `key`. The told step is what the turn's
/// reading ended holding (the row's, for a run that never registered),
/// lowered to the figure's own step: a compaction lowered the reading, and
/// a context that shrank is told each step again even when no later call
/// carried a note. Called after every chat turn's ledger finishes, quiet
/// turns too: a quiet turn tells nothing, but the next one that isn't
/// quiet reads its figure. `reported`: the agent reported the window itself
/// (Codex, A10), so the steps are window-relative ([`step_for`]); the row
/// keeps it as `reported`.
pub fn store_figure(
    vault: &Path,
    key: &str,
    run_id: &str,
    figure: Option<(u64, Option<u64>)>,
    reported: bool,
) {
    let held = ended().remove(run_id);
    let Some((tokens, window)) = figure else {
        return;
    };
    let _serial = STORING.lock().unwrap_or_else(|e| e.into_inner());
    let held = held.or_else(|| live().get(run_id).map(|reading| reading.announced));
    let (row, mut payload) = steps_row(vault, key);
    let stored = payload.get("announced").and_then(Value::as_u64);
    let step = step_for(tokens, window.filter(|_| reported));
    let announced = held.or(stored).unwrap_or(0).min(step);
    payload.insert("tokens".into(), json!(tokens));
    payload.insert("window".into(), json!(window));
    payload.insert("announced".into(), json!(announced));
    if reported {
        payload.insert("reported".into(), json!(true));
    } else {
        payload.remove("reported");
    }
    put_steps(vault, key, row.as_ref(), payload);
}

/// R2b: the `<hollow-context>` line ([`start_line`]) a resumed turn of
/// conversation `key` starts with, when the stored figure passed a step the
/// conversation wasn't told; the step is then stored as told, before the
/// turn registers its gauge, so neither channel repeats it. `None` without
/// a figure or with nothing new. Best effort: the step is marked told when
/// the prompt is composed, before it is delivered, so a turn stopped or
/// failing before its spawn loses that step's line (the next step still
/// fires), as an in-turn answer the client never reads does.
pub fn start_note(vault: &Path, key: &str) -> Option<String> {
    let _serial = STORING.lock().unwrap_or_else(|e| e.into_inner());
    let (row, mut payload) = steps_row(vault, key);
    let tokens = payload.get("tokens").and_then(Value::as_u64)?;
    let announced = payload
        .get("announced")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let window = payload.get("window").and_then(Value::as_u64);
    let reported = payload.get("reported").and_then(Value::as_bool) == Some(true);
    let step = step_for(tokens, window.filter(|_| reported));
    if step <= announced {
        return None;
    }
    payload.insert("announced".into(), json!(step));
    put_steps(vault, key, row.as_ref(), payload);
    Some(start_line(tokens, window))
}

/// R2b: a fresh native session starts with an empty context, so conversation
/// `key`'s stored figure and told step no longer apply: the row becomes
/// `{announced: 0}`. Nothing to do without a row.
pub fn reset_steps(vault: &Path, key: &str) {
    let _serial = STORING.lock().unwrap_or_else(|e| e.into_inner());
    let (row, _) = steps_row(vault, key);
    if let Some(row) = row {
        let mut payload = Map::new();
        payload.insert("announced".into(), json!(0));
        put_steps(vault, key, Some(&row), payload);
    }
}

/// Serializes the row's read-modify-writes ([`store_told`], and R2b's
/// [`store_figure`], [`start_note`] and [`reset_steps`]).
static STORING: Mutex<()> = Mutex::new(());

/// Store the step run `run_id`'s conversation `key` now holds as told,
/// after a [`take_note`] that told it `told`. Parallel calls of one turn
/// can finish in any order, so this writes the live reading's `announced`
/// as it is at write time, under one lock: a slower call never writes the
/// older step it took, and the row goes down only when the reading did (a
/// compaction). A turn already gone writes `told` only when it is higher
/// than what is stored.
pub fn store_told(vault: &Path, run_id: &str, key: &str, told: u64) {
    let _serial = STORING.lock().unwrap_or_else(|e| e.into_inner());
    let current = live().get(run_id).map(|reading| reading.announced);
    let announced = match current {
        Some(announced) => announced,
        None if told > announced_step(vault, key) => told,
        None => return,
    };
    store_announced(vault, key, announced);
}

/// `sessions.context`'s answer for the calling run: its live reading, or a
/// `live: false` answer (never an error) when the run is no running chat turn.
pub fn reading_json(run_id: &str) -> Value {
    let Some(reading) = live().get(run_id).cloned() else {
        return json!({
            "live": false,
            "contextTokens": null,
            "contextWindow": null,
            "advisoryTokens": null,
            "note": "No live chat turn for this caller: sessions.context reads the calling chat turn's own context while it runs (not an automation's, nor from outside a turn).",
        });
    };
    let window = reading.window_now();
    let hand_over_at = advisory(window);
    let past = hand_over_at
        .zip(reading.context_tokens)
        .map(|(at, used)| used >= at);
    json!({
        "live": true,
        "agent": reading.agent.as_str(),
        "model": reading.model,
        "contextTokens": reading.context_tokens,
        "contextWindow": window,
        "advisoryTokens": hand_over_at,
        "pastAdvisory": past,
        "calls": reading.calls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentEvent;
    use crate::usage::TokenCounts;

    fn message(id: &str, input: u64, cached: u64, thread: Option<&str>) -> AgentEvent {
        AgentEvent::MessageUsage {
            message_id: id.into(),
            model: Some("claude-opus-5-5".into()),
            tokens: TokenCounts {
                input,
                cached_input: cached,
                output: 100,
            },
            is_final: true,
            cache_write: 0,
            thread: thread.map(str::to_string),
        }
    }

    /// The registry holds the main thread's figure: a subagent's message,
    /// however large, doesn't move it.
    #[test]
    fn a_published_reading_is_the_main_threads_context() {
        let guard = register(
            "run-ob11-gauge-main",
            AgentKind::Claude,
            Some("opus[1m]"),
            false,
            0,
        );
        let mut acc = UsageAccumulator::default();
        acc.observe(&message("m1", 1_000, 400_000, None));
        guard.publish(&acc);
        let main = reading_json("run-ob11-gauge-main");
        assert_eq!(main["live"], json!(true));
        assert_eq!(main["contextTokens"], json!(401_100));
        acc.observe(&message("s1", 5_000, 700_000, Some("toolu_task")));
        guard.publish(&acc);
        let after = reading_json("run-ob11-gauge-main");
        assert_eq!(after["contextTokens"], json!(401_100), "{after}");
        assert_eq!(after["calls"]["subagents"]["calls"], json!(1), "{after}");
        assert_eq!(after["pastAdvisory"], json!(true), "{after}");
    }

    /// Every way out of a turn drops its guard, and the entry goes with it;
    /// a stale guard for the same run never removes a newer registration.
    #[test]
    fn the_entry_is_removed_when_its_guard_drops() {
        let run = "run-ob11-gauge-drop";
        let guard = register(run, AgentKind::Claude, None, false, 0);
        assert_eq!(reading_json(run)["live"], json!(true));
        drop(guard);
        assert_eq!(reading_json(run)["live"], json!(false));
        let stale = register(run, AgentKind::Claude, None, false, 0);
        let newer = register(run, AgentKind::Claude, None, false, 0);
        drop(stale);
        assert_eq!(reading_json(run)["live"], json!(true), "newer stays");
        drop(newer);
        assert_eq!(reading_json(run)["live"], json!(false));
    }

    /// 350K is offered on a 1M window (a `[1m]` model, or a Claude turn
    /// already past 200K) and nowhere else.
    #[test]
    fn the_advisory_is_offered_on_a_one_million_window_only() {
        let window = |model, used| context_window(AgentKind::Claude, model, used);
        assert_eq!(window(Some("opus[1m]"), None), Some(ONE_MILLION));
        assert_eq!(window(Some("sonnet[1M]"), None), Some(ONE_MILLION));
        assert_eq!(window(Some("opus"), Some(150_000)), Some(CLAUDE_WINDOW));
        assert_eq!(window(None, Some(250_000)), Some(ONE_MILLION));
        assert_eq!(context_window(AgentKind::Codex, Some("gpt-5"), None), None);
        assert_eq!(advisory(Some(ONE_MILLION)), Some(ADVISORY_TOKENS));
        assert_eq!(advisory(Some(CLAUDE_WINDOW)), None);
        assert_eq!(advisory(None), None);
    }

    /// A turn whose main thread's context is now `tokens` (100 of them
    /// output), as one more message in `acc`.
    fn reach(guard: &GaugeGuard, acc: &mut UsageAccumulator, tokens: u64) {
        let id = format!("m{}", acc.call_stats().api_calls.unwrap_or(0));
        acc.observe(&message(&id, 0, tokens - 100, None));
        guard.publish(acc);
    }

    /// R2a: the steps are 300K, 350K, 400K, then every 100K.
    #[test]
    fn the_growth_steps_are_300k_350k_then_every_100k() {
        let steps: Vec<u64> = [
            0, 299_999, 300_000, 349_999, 350_000, 399_999, 400_000, 499_999, 500_000, 1_234_567,
        ]
        .into_iter()
        .map(step_of)
        .collect();
        assert_eq!(
            steps,
            [0, 0, 300_000, 300_000, 350_000, 350_000, 400_000, 400_000, 500_000, 1_200_000]
        );
        let line = note_line(412_345, Some(ONE_MILLION));
        assert!(
            line.starts_with("[Hollow: this conversation's context is at 412K of 1M. Main thread"),
            "{line}"
        );
        // A subagent sharing the connection may read it: only the main
        // thread is asked to hand over.
        assert!(line.contains("hand over (sessions.handover)"), "{line}");
        assert!(line.contains("A subagent: just mention it"), "{line}");
        let widest = note_line(12_345_678, Some(ONE_MILLION));
        assert!(widest.len() <= 200, "{} bytes", widest.len());
        assert!(!note_line(412_345, None).contains(" of "));
    }

    /// Overnight 9 A10: a 1M window (and an unknown one) keeps R2a's steps
    /// exactly; a window under 1M steps at 70% and 85% of it.
    #[test]
    fn claude_steps_unchanged() {
        for tokens in (0..1_500_000)
            .step_by(12_345)
            .chain([300_000, 350_000, 400_000])
        {
            assert_eq!(
                step_for(tokens, Some(ONE_MILLION)),
                step_of(tokens),
                "{tokens}"
            );
            assert_eq!(step_for(tokens, None), step_of(tokens), "{tokens}");
        }
        let codex = Some(258_400);
        assert_eq!(step_for(180_000, codex), 0);
        assert_eq!(step_for(180_880, codex), 180_880);
        assert_eq!(step_for(219_639, codex), 180_880);
        assert_eq!(step_for(231_046, codex), 219_640);
        // A Claude model without `[1m]` is inferred at 200K, a guess: it
        // reports no window, so it keeps R2a's steps (review medium).
        let run = "run-a10-claude-inferred";
        let guard = register(run, AgentKind::Claude, None, false, 0);
        let mut acc = UsageAccumulator::default();
        reach(&guard, &mut acc, 180_000);
        assert_eq!(take_note(run), None, "no step below 300K");
        assert_eq!(reading_json(run)["contextWindow"], CLAUDE_WINDOW);
    }

    /// Overnight 9 A10: a Codex turn's own figure (its rollout's) drives the
    /// reading, the window-relative steps and `sessions.context`.
    #[test]
    fn a_codex_figure_gets_its_window_and_steps() {
        let run = "run-a10-codex-figure";
        let guard = register(run, AgentKind::Codex, Some("gpt-6-astra"), false, 0);
        guard.publish_figure(150_000, Some(258_400));
        assert_eq!(take_note(run), None, "below 70%");
        let reading = reading_json(run);
        assert_eq!(reading["contextTokens"], 150_000);
        assert_eq!(reading["contextWindow"], 258_400);
        assert_eq!(reading["advisoryTokens"], Value::Null);
        guard.publish_figure(231_046, Some(258_400));
        let (note, step) = take_note(run).expect("past 85%");
        assert_eq!(step, 219_640);
        assert!(note.contains("at 231K of 258K."), "{note}");
        assert_eq!(take_note(run), None, "told once");
        // A compaction shrinks it: the steps re-arm.
        guard.publish_figure(40_000, None);
        guard.set_compactions(1);
        let (note, step) = take_note(run).expect("the compaction pointer");
        assert_eq!((note.as_str(), step), (COMPACTED_NOTE, 0));
        guard.publish_figure(200_000, None);
        assert_eq!(take_note(run).map(|(_, step)| step), Some(180_880));
        drop(guard);

        // The stored figure keeps the reported window's steps for the next
        // turn's `<hollow-context>` line; an inferred window doesn't.
        let dir = tempfile::tempdir().unwrap();
        let (vault, key) = (dir.path(), "s-20261019-a10codex1");
        crate::session_store::start_turn(vault, key, "codex", "/tmp", "run-1", "hi").unwrap();
        store_figure(
            vault,
            key,
            "run-a10-none",
            Some((231_046, Some(258_400))),
            true,
        );
        let line = start_note(vault, key).expect("past 85% of the reported window");
        assert!(line.contains("at 231K of 258K."), "{line}");
        assert_eq!(start_note(vault, key), None, "told once");
        store_figure(
            vault,
            key,
            "run-a10-none",
            Some((190_000, Some(CLAUDE_WINDOW))),
            false,
        );
        assert_eq!(
            start_note(vault, key),
            None,
            "an inferred window keeps R2a's steps"
        );
    }

    /// R2a: each step is noted once; a context that shrank (a compaction)
    /// re-arms the steps as it grows back.
    #[test]
    fn a_step_is_noted_once_and_rearmed_after_a_compaction() {
        let run = "run-r2a-steps";
        let guard = register(run, AgentKind::Claude, Some("opus[1m]"), false, 0);
        let mut acc = UsageAccumulator::default();
        assert_eq!(take_note(run), None, "no reading yet");
        reach(&guard, &mut acc, 290_000);
        assert_eq!(take_note(run), None, "below 300K");
        reach(&guard, &mut acc, 310_000);
        let (note, step) = take_note(run).expect("300K");
        assert!(note.contains("at 310K of 1M"), "{note}");
        assert_eq!(step, 300_000);
        assert_eq!(take_note(run), None, "told once");
        reach(&guard, &mut acc, 320_000);
        assert_eq!(take_note(run), None, "same step");
        reach(&guard, &mut acc, 360_000);
        assert_eq!(take_note(run).map(|(_, s)| s), Some(350_000), "advisory");
        reach(&guard, &mut acc, 40_000);
        assert_eq!(take_note(run), None, "shrank");
        reach(&guard, &mut acc, 410_000);
        let (note, step) = take_note(run).expect("re-armed");
        assert!(note.contains("at 410K"), "{note}");
        assert_eq!(step, 400_000);
    }

    /// R2a: a plan, bare or unattended turn is never noted.
    #[test]
    fn a_quiet_turn_gets_no_note() {
        let run = "run-r2a-quiet";
        let guard = register(run, AgentKind::Claude, Some("opus[1m]"), true, 0);
        let mut acc = UsageAccumulator::default();
        reach(&guard, &mut acc, 410_000);
        guard.set_compactions(1);
        assert_eq!(take_note(run), None);
        assert_eq!(take_note("run-r2a-nobody"), None, "no live turn");
    }

    /// R1's in-turn half: each new compaction is pointed out once, at
    /// `sessions.task_state`.
    #[test]
    fn a_compaction_is_pointed_out_once() {
        let run = "run-r2a-compacted";
        let guard = register(run, AgentKind::Claude, None, false, 0);
        let mut acc = UsageAccumulator::default();
        reach(&guard, &mut acc, 120_000);
        assert_eq!(take_note(run), None);
        guard.set_compactions(1);
        let (note, _) = take_note(run).expect("compacted");
        assert_eq!(note, COMPACTED_NOTE);
        assert!(note.contains("sessions.task_state"));
        assert_eq!(take_note(run), None, "once");
        guard.set_compactions(1);
        assert_eq!(take_note(run), None, "the same compaction");
        // A second one, past a step too: both lines in one note.
        reach(&guard, &mut acc, 305_000);
        guard.set_compactions(2);
        let (note, step) = take_note(run).expect("again");
        assert!(note.starts_with(COMPACTED_NOTE), "{note}");
        assert!(note.contains("at 305K"), "{note}");
        assert_eq!(step, 300_000);
    }

    /// R2a: a resumed turn starts from the step its conversation was
    /// already told, so a turn already past it says nothing at once.
    #[test]
    fn a_turn_seeded_with_its_announced_step_says_nothing_at_once() {
        let run = "run-r2a-seeded";
        let guard = register(run, AgentKind::Claude, Some("opus[1m]"), false, 350_000);
        let mut acc = UsageAccumulator::default();
        reach(&guard, &mut acc, 360_000);
        assert_eq!(take_note(run), None, "already told 350K");
        reach(&guard, &mut acc, 410_000);
        assert_eq!(take_note(run).map(|(_, s)| s), Some(400_000));
    }

    /// The told step round-trips through the conversation's
    /// `contextSteps` row, keeping its other fields.
    #[test]
    fn the_told_step_round_trips_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, key) = (dir.path(), "s-20261018-r2a00001");
        crate::session_store::start_turn(vault, key, "claude", "/tmp", "run-1", "hi").unwrap();
        assert_eq!(announced_step(vault, key), 0, "no row");
        store_announced(vault, key, 300_000);
        assert_eq!(announced_step(vault, key), 300_000);
        crate::session_store::put_derived_state(
            vault,
            key,
            STEPS_KIND,
            &crate::session_store::DerivedRow {
                revision: 5,
                through_ord: 3,
                payload: json!({ "announced": 300_000, "tokens": 320_000 }).to_string(),
            },
        )
        .unwrap();
        store_announced(vault, key, 400_000);
        let row = crate::session_store::derived_state(vault, key, STEPS_KIND)
            .unwrap()
            .unwrap();
        assert_eq!((row.revision, row.through_ord), (6, 3));
        let payload: Value = serde_json::from_str(&row.payload).unwrap();
        assert_eq!(payload, json!({ "announced": 400_000, "tokens": 320_000 }));
    }

    /// R2a review: a slower parallel call that took an older step never
    /// writes it over a newer one; the row goes down only when the live
    /// reading did (a compaction), and a turn already gone only raises it.
    #[test]
    fn a_slower_parallel_call_never_lowers_the_told_step() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, key, run) = (dir.path(), "s-20261018-r2a00003", "run-r2a-store");
        crate::session_store::start_turn(vault, key, "claude", "/tmp", run, "hi").unwrap();
        let guard = register(run, AgentKind::Claude, Some("opus[1m]"), false, 0);
        let mut acc = UsageAccumulator::default();
        reach(&guard, &mut acc, 310_000);
        let (_, older) = take_note(run).unwrap();
        reach(&guard, &mut acc, 360_000);
        let (_, newer) = take_note(run).unwrap();
        store_told(vault, run, key, newer);
        store_told(vault, run, key, older);
        assert_eq!(
            announced_step(vault, key),
            350_000,
            "the older call landed last"
        );
        // A compaction lowers the reading, and the next note's write with it.
        reach(&guard, &mut acc, 40_000);
        guard.set_compactions(1);
        let (_, told) = take_note(run).unwrap();
        store_told(vault, run, key, told);
        assert_eq!(announced_step(vault, key), 0);
        drop(guard);
        store_told(vault, run, key, 0);
        store_told(vault, run, key, 300_000);
        store_told(vault, run, key, 0);
        assert_eq!(announced_step(vault, key), 300_000, "gone: only raised");
    }

    fn steps_payload(vault: &Path, key: &str) -> Value {
        let row = crate::session_store::derived_state(vault, key, STEPS_KIND)
            .unwrap()
            .unwrap();
        serde_json::from_str(&row.payload).unwrap()
    }

    /// R2b: a finished turn's figure lands in the row and lowers the told
    /// step to its own; the next turn's start says a step once, and a fresh
    /// session clears the figure.
    #[test]
    fn a_stored_figure_is_told_once_at_the_start_of_a_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, key) = (dir.path(), "s-20261018-r2b00001");
        crate::session_store::start_turn(vault, key, "claude", "/tmp", "run-1", "hi").unwrap();
        assert_eq!(start_note(vault, key), None, "no row");
        reset_steps(vault, key);
        assert!(
            crate::session_store::derived_state(vault, key, STEPS_KIND)
                .unwrap()
                .is_none(),
            "a reset writes no row"
        );
        // A run that never registered: the row's step, lowered.
        let run = "run-r2b-unregistered";
        let figure =
            |tokens: u64, window| store_figure(vault, key, run, Some((tokens, window)), false);
        store_announced(vault, key, 400_000);
        figure(360_000, Some(ONE_MILLION));
        assert_eq!(
            steps_payload(vault, key),
            json!({ "announced": 350_000, "tokens": 360_000, "window": ONE_MILLION }),
            "lowered to the figure's step"
        );
        assert_eq!(start_note(vault, key), None, "350K already told");
        store_figure(vault, key, run, None, false);
        assert_eq!(
            steps_payload(vault, key)["tokens"],
            json!(360_000),
            "no figure"
        );
        figure(412_345, Some(ONE_MILLION));
        let line = start_note(vault, key).expect("400K");
        assert_eq!(
            line,
            "<hollow-context>This conversation's context is at 412K of 1M. On a long job, finish the step and hand over (sessions.handover).</hollow-context>\n\n"
        );
        assert_eq!(announced_step(vault, key), 400_000, "the turn's gauge seed");
        assert_eq!(start_note(vault, key), None, "once");
        // A turn that compacted ends lower: the steps re-arm.
        figure(120_000, None);
        assert_eq!(announced_step(vault, key), 0);
        assert_eq!(start_note(vault, key), None, "below 300K");
        figure(310_000, None);
        reset_steps(vault, key);
        assert_eq!(steps_payload(vault, key), json!({ "announced": 0 }));
        assert_eq!(start_note(vault, key), None, "a fresh session's row");
    }

    /// R2b review: a turn that compacted and made no call after it stores
    /// the step its reading ended holding, not the row's older one, so the
    /// next turn is told the step it grew back past.
    #[test]
    fn a_figure_lowers_from_the_step_the_turn_ended_holding() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, key, run) = (dir.path(), "s-20261018-r2b00002", "run-r2b-ended");
        crate::session_store::start_turn(vault, key, "claude", "/tmp", run, "hi").unwrap();
        store_announced(vault, key, 400_000);
        let guard = register(run, AgentKind::Claude, Some("opus[1m]"), false, 400_000);
        let mut acc = UsageAccumulator::default();
        reach(&guard, &mut acc, 40_000);
        reach(&guard, &mut acc, 360_000);
        drop(guard);
        store_figure(vault, key, run, Some((360_000, Some(ONE_MILLION))), false);
        assert_eq!(announced_step(vault, key), 0);
        let line = start_note(vault, key).expect("350K, never told since");
        assert!(line.contains("at 360K of 1M"), "{line}");
        // Taken once: a second store for the run reads the row.
        store_figure(vault, key, run, Some((360_000, Some(ONE_MILLION))), false);
        assert_eq!(announced_step(vault, key), 350_000);
    }

    /// R2b: the start-of-turn line stays within its 220-byte budget.
    #[test]
    fn the_start_line_fits_its_budget() {
        let widest = start_line(12_345_678, Some(ONE_MILLION));
        assert!(widest.len() <= 220, "{} bytes", widest.len());
        assert!(!start_line(412_345, None).contains(" of "));
    }
}
