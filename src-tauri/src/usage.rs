//! Usage capture: what each agent CLI reports about tokens and cost, and how
//! Hollow turns that into one honest figure per invocation (Paperclip PR 6).
//!
//! Pure except for the billing probe, which reads a few local config files and
//! keeps only booleans. The ledger that stores the result is
//! [`crate::usage_ledger`]; this module never touches it.
//!
//! **Normalization** ([`TokenCounts`]):
//! - `input` is prompt tokens NOT read from cache, cache writes included.
//!   Claude: `inputTokens + cacheCreationInputTokens`. Codex:
//!   `input_tokens − cached_input_tokens` (Codex's cached count is a subset of
//!   its input count).
//! - `cached_input` is cache reads.
//! - `output` includes reasoning / thinking.
//!
//! **No data is not zero.** A counter set counts only when its input and output
//! fields are numbers (cache fields default to 0); a set whose normalized
//! counters are all zero is "not reported" (Claude error envelopes and Codex's
//! default usage are all zeros); a cost that is not finite and positive is "not
//! reported". Missing values stay `None` and reach the ledger as NULL, never 0.
//!
//! **What the CLIs actually report** (recorded 2026-09-25, see
//! `fixtures/usage/README.md`):
//! - Claude `result.modelUsage` is a per-model map that includes side models
//!   (a background haiku call on every run). Top-level `usage` covers the main
//!   model only, and is all zeros on `error_max_budget_usd`. So the terminal
//!   total is always the `modelUsage` sum, with `usage` only as a fallback.
//! - On `--resume`, `modelUsage` and `total_cost_usd` are SESSION-CUMULATIVE;
//!   top-level `usage` is per-invocation. [`resolve`] therefore subtracts the
//!   previous turn's stored terminal for the same native session, and accepts
//!   the delta only when it reproduces this invocation's own per-message usage.
//!   When it doesn't but the cost rose, the rise is recorded once as a span
//!   (`session_delta_span`, OB24) covering the runs since that terminal.
//!   Nothing here ever inflates: every fallback undercounts or reports unknown.
//! - `assistant` events carry a start-of-message usage SNAPSHOT. The final
//!   per-message figure arrives only in `stream_event` → `message_delta`, which
//!   Claude forwards only with `--include-partial-messages` (chat today). The
//!   accumulator therefore keys messages by id, max-merges snapshots and lets a
//!   final replace them ([`UsageAccumulator`]); PR 8's usage-only partials mode
//!   feeds the same events for non-interactive runs.
//! - Codex `turn.completed.usage` is per turn, including on resume. No cost.
//!   App-server `turn.usage` snapshots are partial until that final report;
//!   they are not per-message or context/call measurements.
//! - Antigravity (agy 1.5.13, probed 2026-11-20) puts `usage` on each finished
//!   step (`input_tokens` = uncached input, `cache_read_tokens`,
//!   `output_tokens` including `thinking_tokens`; `total_tokens` = input +
//!   output). The `result` carries the SAME shape but session-cumulative on
//!   `--conversation`, so a turn is the sum of its own steps
//!   ([`antigravity_turn_usage`]). No cost: Hollow prices it from the
//!   published Gemini rates (`usage_prices`), `estimated`.

use crate::agent::{AgentEvent, AgentKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

// ── Token counts ────────────────────────────────────────────────────────────

/// Normalized token counters for one invocation, message or model. See the
/// module doc for what each field means per agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCounts {
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
}

impl TokenCounts {
    pub fn add(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_add(other.input),
            cached_input: self.cached_input.saturating_add(other.cached_input),
            output: self.output.saturating_add(other.output),
        }
    }

    /// `self − other`, or `None` when any field would go negative.
    pub fn checked_sub(&self, other: &Self) -> Option<Self> {
        Some(Self {
            input: self.input.checked_sub(other.input)?,
            cached_input: self.cached_input.checked_sub(other.cached_input)?,
            output: self.output.checked_sub(other.output)?,
        })
    }

    /// Field-wise maximum: a later snapshot of the same message only grows.
    pub fn max_merge(self, other: Self) -> Self {
        Self {
            input: self.input.max(other.input),
            cached_input: self.cached_input.max(other.cached_input),
            output: self.output.max(other.output),
        }
    }

    pub fn total(&self) -> u64 {
        self.input
            .saturating_add(self.cached_input)
            .saturating_add(self.output)
    }

    pub fn is_zero(&self) -> bool {
        self.total() == 0
    }

    /// `None` for an all-zero set: zero means "not reported", never "free".
    fn reported(self) -> Option<Self> {
        (!self.is_zero()).then_some(self)
    }
}

/// A non-negative integer field. Accepts a whole float too; anything else
/// (missing, negative, a string) is absent.
fn num(value: &Value, key: &str) -> Option<u64> {
    let field = value.get(key)?;
    field.as_u64().or_else(|| {
        field
            .as_f64()
            .filter(|f| f.is_finite() && *f >= 0.0)
            .map(|f| f as u64)
    })
}

/// A cost that means something: finite and positive.
fn cost(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite() && *c > 0.0)
}

/// Claude's snake_case counters (`usage` blocks, message snapshots and
/// finals): input and output must be numbers; cache fields default to 0.
fn claude_counter(usage: &Value) -> Option<TokenCounts> {
    let input = num(usage, "input_tokens")?;
    let output = num(usage, "output_tokens")?;
    let created = num(usage, "cache_creation_input_tokens").unwrap_or(0);
    let read = num(usage, "cache_read_input_tokens").unwrap_or(0);
    TokenCounts {
        input: input.saturating_add(created),
        cached_input: read,
        output,
    }
    .reported()
}

/// One `result.modelUsage` entry (camelCase counters).
fn model_usage_counter(entry: &Value) -> Option<TokenCounts> {
    let input = num(entry, "inputTokens")?;
    let output = num(entry, "outputTokens")?;
    let created = num(entry, "cacheCreationInputTokens").unwrap_or(0);
    let read = num(entry, "cacheReadInputTokens").unwrap_or(0);
    TokenCounts {
        input: input.saturating_add(created),
        cached_input: read,
        output,
    }
    .reported()
}

/// `claude-opus-5-5[1m]` → `claude-opus-5-5`. `modelUsage` keys carry the
/// context-window suffix; `message.model` does not.
pub fn normalize_model(model: &str) -> String {
    let trimmed = model.trim();
    match (trimmed.ends_with(']'), trimmed.rfind('[')) {
        (true, Some(at)) if at > 0 => trimmed[..at].trim().to_string(),
        _ => trimmed.to_string(),
    }
}

// ── What a terminal event reported ──────────────────────────────────────────

/// Usage carried by an agent's end-of-turn event. `Default` means nothing was
/// reported.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReportedUsage {
    /// Every model's normalized total: the `modelUsage` sum for Claude (side
    /// models included), `turn.completed.usage` for Codex.
    pub tokens: Option<TokenCounts>,
    pub cost_usd: Option<f64>,
    /// Normalized model name → tokens, most tokens first.
    pub by_model: Vec<(String, TokenCounts)>,
    /// Claude's top-level `usage` block, normalized: the MAIN model only, and
    /// per invocation even on `--resume` (where `modelUsage` is cumulative).
    pub main_usage: Option<TokenCounts>,
    /// Bounded copy of the raw counters, kept in the ledger's `usage_json`.
    pub raw: Option<Value>,
    /// Claude's `num_turns` (OB05): per invocation, also on `--resume`.
    pub num_turns: Option<u64>,
    /// Claude's `duration_api_ms` (OB05). SESSION-CUMULATIVE on `--resume`
    /// like `total_cost_usd` (the recorded resume reports 6990 ms against a
    /// 2345 ms wall clock), so the ledger keeps it for fresh runs only.
    /// Grok's, through the same parser, is per invocation on `--resume`
    /// (`fixtures/grok/resume.jsonl`: 2164 ms after the one-shot's 3526).
    pub api_ms: Option<u64>,
}

impl ReportedUsage {
    /// Neither tokens nor cost: must never replace per-message estimates.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_none() && self.cost_usd.is_none()
    }
}

/// Hollow's own price for one `modelUsage` entry the CLI couldn't price
/// (`costBasis: "unknown"`): for `claude-haiku-5-5` on Claude Code 2.1.286
/// the CLI's fallback estimate was about 40× too high (2026-10-07). The
/// rates come from the one dated price table (`usage_prices`, master plan
/// F2), one rate per model, never chosen per call: `modelUsage` is
/// cumulative on `--resume` and turns are recorded as differences, so a tier
/// picked per call would reprice the whole session history.
fn unpriced_cost(model: &str, entry: &Value) -> Option<f64> {
    if entry.get("costBasis").and_then(Value::as_str) != Some("unknown") {
        return None;
    }
    let price = crate::usage_prices::lookup(model)?;
    let n = |key: &str| num(entry, key).unwrap_or(0) as f64 / 1_000_000.0;
    Some(
        n("inputTokens") * price.input
            + n("cacheCreationInputTokens") * price.cache_write
            + n("cacheReadInputTokens") * price.cache_read
            + n("outputTokens") * price.output,
    )
}

fn sorted_models(map: BTreeMap<String, TokenCounts>) -> Vec<(String, TokenCounts)> {
    let mut models: Vec<_> = map.into_iter().collect();
    models.sort_by(|a, b| b.1.total().cmp(&a.1.total()).then_with(|| a.0.cmp(&b.0)));
    models
}

/// Claude's `result` record. Tokens are the `modelUsage` sum (side models
/// count); the top-level `usage` block is a fallback only when no `modelUsage`
/// entry survives the no-data rules, because it is main-model-only and all
/// zeros on a budget stop.
pub fn claude_result_usage(evt: &Value) -> ReportedUsage {
    let mut by_model: BTreeMap<String, TokenCounts> = BTreeMap::new();
    let mut raw_models = Map::new();
    let mut summed_cost: Option<f64> = None;
    let mut repriced = false;
    if let Some(models) = evt.get("modelUsage").and_then(Value::as_object) {
        for (name, entry) in models {
            let own = unpriced_cost(&normalize_model(name), entry);
            repriced |= own.is_some();
            let model_cost = own.or_else(|| cost(entry.get("costUSD")));
            if let Some(c) = model_cost {
                summed_cost = Some(summed_cost.unwrap_or(0.0) + c);
            }
            let mut kept = Map::new();
            for key in [
                "inputTokens",
                "outputTokens",
                "cacheReadInputTokens",
                "cacheCreationInputTokens",
                "costUSD",
                "costBasis",
            ] {
                if let Some(v) = entry.get(key) {
                    kept.insert(key.to_string(), v.clone());
                }
            }
            raw_models.insert(name.clone(), Value::Object(kept));
            if let Some(tokens) = model_usage_counter(entry) {
                let slot = by_model.entry(normalize_model(name)).or_default();
                *slot = slot.add(tokens);
            }
        }
    }
    let usage_block = evt.get("usage").unwrap_or(&Value::Null);
    let main_usage = claude_counter(usage_block);
    let model_sum = by_model
        .values()
        .copied()
        .fold(TokenCounts::default(), TokenCounts::add)
        .reported();
    let mut raw_usage = Map::new();
    for key in [
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ] {
        if let Some(v) = usage_block.get(key) {
            raw_usage.insert(key.to_string(), v.clone());
        }
    }
    ReportedUsage {
        tokens: model_sum.or(main_usage),
        // A model Hollow priced itself makes the sum the total: the CLI's
        // own total carries its fallback estimate for it.
        cost_usd: if repriced {
            summed_cost
        } else {
            cost(evt.get("total_cost_usd")).or(summed_cost)
        },
        by_model: sorted_models(by_model),
        main_usage,
        raw: Some(json!({ "modelUsage": raw_models, "usage": raw_usage })),
        num_turns: num(evt, "num_turns"),
        api_ms: num(evt, "duration_api_ms"),
    }
}

/// A Claude message's usage (from an `assistant` event's `message`, or a
/// `message_start`'s `message`): `(id, normalized model, tokens)`. Requires a
/// string id and a usage object that passes the no-data rules.
pub fn claude_message_usage(message: &Value) -> Option<(String, Option<String>, TokenCounts)> {
    let id = message
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let tokens = claude_counter(message.get("usage")?)?;
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty())
        .map(normalize_model);
    Some((id.to_string(), model, tokens))
}

/// The final usage a `message_delta` stream event carries (no id: the caller
/// tracks the current message from `message_start`).
pub fn claude_delta_usage(event: &Value) -> Option<TokenCounts> {
    claude_counter(event.get("usage")?)
}

/// The cache-creation tokens in a Claude message's (or `message_delta`'s)
/// `usage` block, 0 when absent (OB05). [`TokenCounts::input`] already
/// includes them; this keeps them apart for `cacheWriteTokens`.
pub fn claude_cache_write(holder: &Value) -> u64 {
    holder
        .get("usage")
        .and_then(|usage| num(usage, "cache_creation_input_tokens"))
        .unwrap_or(0)
}

/// One Antigravity step's `usage` block, normalized: `input_tokens` is the
/// uncached input, `cache_read_tokens` the cache reads (not part of
/// `total_tokens`), `output_tokens` includes thinking. Input and output must
/// be numbers; all zeros is "not reported".
pub fn antigravity_counter(usage: &Value) -> Option<TokenCounts> {
    let input = num(usage, "input_tokens")?;
    let output = num(usage, "output_tokens")?;
    TokenCounts {
        input,
        cached_input: num(usage, "cache_read_tokens").unwrap_or(0),
        output,
    }
    .reported()
}

/// An Antigravity turn's usage: the sum of this invocation's own steps (the
/// `result` block is cumulative on resume), under the base model the run
/// used. No cost; empty when no step reported tokens.
pub fn antigravity_turn_usage(steps: TokenCounts, model: Option<&str>) -> ReportedUsage {
    let tokens = steps.reported();
    ReportedUsage {
        tokens,
        cost_usd: None,
        by_model: tokens
            .zip(model)
            .map(|(tokens, model)| vec![(model.to_string(), tokens)])
            .unwrap_or_default(),
        main_usage: tokens,
        raw: tokens.map(|t| {
            json!({
                "input_tokens": t.input,
                "cache_read_tokens": t.cached_input,
                "output_tokens": t.output,
            })
        }),
        num_turns: None,
        api_ms: None,
    }
}

/// Codex `turn.completed`: per turn, including on resume. Input and output
/// must be numbers; `cached_input_tokens` is a subset of input.
pub fn codex_turn_usage(evt: &Value) -> ReportedUsage {
    let usage = evt.get("usage").unwrap_or(&Value::Null);
    let tokens = match (num(usage, "input_tokens"), num(usage, "output_tokens")) {
        (Some(input), Some(output)) => {
            let cached = num(usage, "cached_input_tokens").unwrap_or(0);
            TokenCounts {
                input: input.saturating_sub(cached),
                cached_input: cached,
                output,
            }
            .reported()
        }
        _ => None,
    };
    ReportedUsage {
        tokens,
        cost_usd: None,
        by_model: Vec::new(),
        main_usage: None,
        raw: usage.is_object().then(|| usage.clone()),
        num_turns: None,
        api_ms: None,
    }
}

// ── Accumulating one run's events ───────────────────────────────────────────

#[derive(Debug, Clone)]
struct MessageEntry {
    model: Option<String>,
    tokens: TokenCounts,
    /// Set by a `message_delta` final; later snapshots no longer change it.
    is_final: bool,
    /// Cache-creation tokens (OB05), merged like `tokens`.
    cache_write: u64,
    /// The subagent thread (`parent_tool_use_id`) the message belongs to;
    /// `None` on the main thread (OB05).
    thread: Option<String>,
}

/// A run's calls and context (OB05), from its per-message usage and its
/// terminal. Written into the ledger's `usage_json` and onto the run receipt
/// (`apiUsage`). A figure the agent didn't supply stays `None` and absent:
/// only per-message streams (Claude, Grok) fill the message-derived ones,
/// and only a Claude-format `result` (Claude's, and Grok's, which goes
/// through the same parser) the other two.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallStats {
    /// Distinct main-thread assistant message ids: model requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_calls: Option<u64>,
    /// The largest prompt one main-thread message sent: fresh input plus
    /// cache writes plus cache reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_context_tokens: Option<u64>,
    /// Cache-creation tokens over every streamed message, subagents' too;
    /// absent when no message reported any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    /// The results' `duration_api_ms` (Claude, Grok), summed over the run's
    /// terminals. The ledger drops it on a resumed Claude turn, where it is
    /// session-cumulative, and on any other Claude row keeps the last
    /// terminal's (a later attempt resumed the session the row started).
    /// Grok's is per invocation even on `--resume`, so it stays summed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_ms: Option<u64>,
    /// The results' `num_turns` (Claude, Grok), summed over the run's
    /// terminals: per invocation, also on `--resume`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_turns: Option<u64>,
    /// Subagents' messages, kept apart from the main thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagents: Option<SubagentUsage>,
    /// Overnight 9 S31: the Workflow tool's agents, read from their own
    /// transcripts after a Claude run (`usage_ledger::workflow_agent_usage`).
    /// An estimate beside the row, never folded into its tokens or cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_agents: Option<WorkflowAgentUsage>,
}

impl CallStats {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// [`CallStats::subagents`]: distinct subagent message ids, and their tokens
/// (input, cache writes, cache reads and output: [`TokenCounts::total`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentUsage {
    pub calls: u64,
    pub tokens: u64,
}

/// [`CallStats::workflow_agents`] (overnight 9 S31): Workflow-tool agents
/// whose messages fell inside the run's window, summed from the usage
/// blocks of `subagents/workflows/<wf>/*.jsonl` under the run's session.
/// Their usage is often missing from the result's `modelUsage`, so the
/// row's tokens may not hold them. Labelled `estimated`; never in the
/// row's cost.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAgentUsage {
    /// Agent transcripts with at least one message in the window.
    pub count: u64,
    /// Distinct message ids: model requests.
    pub calls: u64,
    /// Input, cache reads and output ([`TokenCounts::total`]).
    pub tokens: u64,
    /// Uncached input, cache writes included ([`TokenCounts::input`]).
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    /// The part of `input_tokens` that wrote the cache.
    pub cache_write_tokens: u64,
    /// List price of those tokens by model (`usage_prices::list_usd`), in
    /// millionths of a dollar so the receipt stays `Eq`; written as
    /// `estimateUsd`. Absent when no model has a price row.
    #[serde(
        default,
        rename = "estimateUsd",
        with = "micro_usd",
        skip_serializing_if = "Option::is_none"
    )]
    pub estimate_micros: Option<u64>,
    /// How many of these tokens the row's own figure can't hold: the
    /// workflow tokens plus what the run's stream showed, less the row's
    /// tokens. Absent when not positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unmetered_tokens: Option<u64>,
    /// Always `"estimated"`: these figures never reach known cost.
    pub basis: String,
    /// The scan stopped at its file or byte limit; the figures are a floor.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// `Option<u64>` millionths of a dollar ⇄ a JSON dollar figure.
mod micro_usd {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(micros) => s.serialize_f64(*micros as f64 / 1_000_000.0),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        let usd = Option::<f64>::deserialize(d)?;
        Ok(usd
            .filter(|u| u.is_finite() && *u >= 0.0)
            .map(|u| (u * 1_000_000.0).round() as u64))
    }
}

/// Everything usage-related one run's stream reported. Messages are keyed by
/// id because Claude repeats a message's snapshot on every content block;
/// snapshots max-merge, and a final (`message_delta`) replaces them.
#[derive(Debug, Default, Clone)]
pub struct UsageAccumulator {
    /// Latest whole-turn snapshot (Codex), separate from message/call counts.
    partial_turn: Option<ReportedUsage>,
    messages: HashMap<String, MessageEntry>,
    /// The last non-empty terminal. One `result` per Claude invocation and
    /// one `turn.completed` per `codex exec`; if a stream ever carried more,
    /// the last wins (an undercount at worst, never an inflation) and
    /// `terminals` says so.
    terminal: Option<ReportedUsage>,
    terminals: u32,
    last_raw: Option<Value>,
    /// The id of the main-thread message whose usage arrived last: its
    /// prompt plus its answer is how full the context window is now. A
    /// subagent's message never sets it (OB05).
    last_message: Option<String>,
    /// `num_turns` summed over every terminal that reported one (OB05):
    /// it is per invocation, also on `--resume`, so two results in one run
    /// add up rather than the last one winning.
    num_turns: Option<u64>,
    /// `duration_api_ms` summed the same way: the run's API time where the
    /// figure is per invocation (every Grok run, a fresh Claude run).
    api_ms: Option<u64>,
    /// The last terminal's own `duration_api_ms`. Claude's is
    /// session-cumulative on `--resume`, so on a Claude row whose later
    /// attempt resumed the session its first attempt started, this, not the
    /// sum, is the row's API time; the ledger picks (`UsageRun::call_stats`).
    last_api_ms: Option<u64>,
}

impl UsageAccumulator {
    /// Fold one event in. True when the running total changed, which is the
    /// hook PR 8's mid-run budget check uses.
    pub fn observe(&mut self, evt: &AgentEvent) -> bool {
        let before = self.running_total();
        match evt {
            AgentEvent::TurnUsage(usage) => {
                if !usage.is_empty() {
                    self.partial_turn = Some(usage.clone());
                    self.last_raw = usage.raw.clone();
                }
            }
            AgentEvent::MessageUsage {
                message_id,
                model,
                tokens,
                is_final,
                cache_write,
                thread,
            } => {
                let entry =
                    self.messages
                        .entry(message_id.clone())
                        .or_insert_with(|| MessageEntry {
                            model: model.clone(),
                            tokens: TokenCounts::default(),
                            is_final: false,
                            cache_write: 0,
                            thread: thread.clone(),
                        });
                if entry.model.is_none() {
                    entry.model = model.clone();
                }
                if entry.thread.is_none() {
                    entry.thread = thread.clone();
                }
                if entry.thread.is_none()
                    && self.last_message.as_deref() != Some(message_id.as_str())
                {
                    self.last_message = Some(message_id.clone());
                }
                if *is_final {
                    entry.tokens = *tokens;
                    entry.cache_write = *cache_write;
                    entry.is_final = true;
                } else if !entry.is_final {
                    entry.tokens = entry.tokens.max_merge(*tokens);
                    entry.cache_write = entry.cache_write.max(*cache_write);
                }
            }
            AgentEvent::Result { usage, .. } => {
                self.terminals = self.terminals.saturating_add(1);
                if let Some(turns) = usage.num_turns {
                    self.num_turns = Some(self.num_turns.unwrap_or(0).saturating_add(turns));
                }
                if let Some(ms) = usage.api_ms {
                    self.api_ms = Some(self.api_ms.unwrap_or(0).saturating_add(ms));
                    self.last_api_ms = Some(ms);
                }
                if usage.raw.is_some() {
                    self.last_raw = usage.raw.clone();
                }
                if !usage.is_empty() {
                    self.terminal = Some(usage.clone());
                }
            }
            _ => return false,
        }
        self.running_total() != before
    }

    /// The terminal's tokens once reported, else the per-message sum.
    pub fn running_total(&self) -> Option<TokenCounts> {
        self.terminal
            .as_ref()
            .and_then(|t| t.tokens)
            .or_else(|| self.partial_turn.as_ref().and_then(|t| t.tokens))
            .or_else(|| self.message_total())
    }

    /// Sum over messages (each counted once), or `None` when nothing usable.
    pub fn message_total(&self) -> Option<TokenCounts> {
        self.messages
            .values()
            .map(|m| m.tokens)
            .fold(TokenCounts::default(), TokenCounts::add)
            .reported()
    }

    /// Whether the stream reported message `id` (overnight 9 S31: a
    /// Workflow-tool agent's message the stream also carried isn't counted
    /// as unmetered twice).
    pub fn has_message(&self, id: &str) -> bool {
        self.messages.contains_key(id)
    }

    /// Per-message sums by normalized model (messages without a model are
    /// grouped under `unknown`, which never matches a terminal model).
    pub fn message_by_model(&self) -> BTreeMap<String, TokenCounts> {
        let mut out: BTreeMap<String, TokenCounts> = BTreeMap::new();
        for entry in self.messages.values() {
            let key = entry.model.clone().unwrap_or_else(|| "unknown".to_string());
            let slot = out.entry(key).or_default();
            *slot = slot.add(entry.tokens);
        }
        out.retain(|_, tokens| !tokens.is_zero());
        out
    }

    pub fn terminal(&self) -> Option<&ReportedUsage> {
        self.terminal.as_ref()
    }

    /// Tokens in the context window after the last message: its prompt
    /// (fresh, cache-written and cache-read) plus its answer. Only streams
    /// with per-message usage (Claude, Grok) report it; a whole-turn total
    /// sums every call in the turn, so it is never used here. Main thread
    /// only: a subagent's message is another context (OB05).
    pub fn context_tokens(&self) -> Option<u64> {
        let id = self.last_message.as_ref()?;
        let total = self.messages.get(id)?.tokens.total();
        (total > 0).then_some(total)
    }

    /// The run's calls and context so far (OB05; see [`CallStats`]). The
    /// message-derived figures are `None` when no message reported usage.
    pub fn call_stats(&self) -> CallStats {
        let mut stats = CallStats {
            num_turns: self.num_turns,
            api_ms: self.api_ms,
            ..CallStats::default()
        };
        if self.messages.is_empty() {
            return stats;
        }
        let (mut calls, mut peak, mut writes) = (0u64, 0u64, 0u64);
        let mut subagents = SubagentUsage::default();
        for entry in self.messages.values() {
            writes = writes.saturating_add(entry.cache_write);
            if entry.thread.is_some() {
                subagents.calls += 1;
                subagents.tokens = subagents.tokens.saturating_add(entry.tokens.total());
            } else {
                calls += 1;
                // `input` already holds the cache writes.
                let tokens = entry.tokens;
                peak = peak.max(tokens.input.saturating_add(tokens.cached_input));
            }
        }
        stats.api_calls = Some(calls);
        stats.peak_context_tokens = (peak > 0).then_some(peak);
        // Absent, never 0: a stream that never reported a cache write (a
        // missing field reads as 0, see `claude_cache_write`) says nothing.
        stats.cache_write_tokens = (writes > 0).then_some(writes);
        stats.subagents = (subagents.calls > 0).then_some(subagents);
        stats
    }

    pub fn terminals(&self) -> u32 {
        self.terminals
    }

    /// The last terminal's own `duration_api_ms` (see the field).
    pub fn last_api_ms(&self) -> Option<u64> {
        self.last_api_ms
    }

    pub fn last_raw(&self) -> Option<&Value> {
        self.last_raw.as_ref()
    }

    /// Compare the per-message sums with the terminal, model by model.
    pub fn self_check(&self) -> SelfCheck {
        let Some(terminal) = &self.terminal else {
            return SelfCheck::Unchecked;
        };
        let messages = self.message_by_model();
        let mut shared = 0;
        for (model, tokens) in &terminal.by_model {
            if let Some(counted) = messages.get(model) {
                shared += 1;
                if counted != tokens {
                    return SelfCheck::Mismatched;
                }
            }
        }
        if shared == 0 {
            SelfCheck::Unchecked
        } else {
            SelfCheck::Matched
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfCheck {
    /// Every model both sides report agrees, and at least one is shared.
    Matched,
    Mismatched,
    /// No terminal, or no model in common.
    Unchecked,
}

// ── Resolving one row's figures ─────────────────────────────────────────────

/// A terminal as the ledger stores it (`usage_json.terminal`), so the next
/// resumed turn of the same native session can subtract it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalRecord {
    pub by_model: BTreeMap<String, TokenCounts>,
    pub tokens: Option<TokenCounts>,
    pub cost_usd: Option<f64>,
}

impl TerminalRecord {
    pub fn from(usage: &ReportedUsage) -> Self {
        let mut by_model: BTreeMap<String, TokenCounts> = BTreeMap::new();
        for (model, tokens) in &usage.by_model {
            let slot = by_model.entry(model.clone()).or_default();
            *slot = slot.add(*tokens);
        }
        Self {
            by_model,
            tokens: usage.tokens,
            cost_usd: usage.cost_usd,
        }
    }
}

/// How a row's figures were obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// The invocation's own report (every non-resumed run, and Codex always).
    PerRun,
    /// A resumed Claude turn whose terminal equals its own messages for every
    /// model it reports. Defensive: the recorded CLI reports cumulative totals
    /// on resume, so this does not occur in practice.
    PerRunVerified,
    /// A resumed Claude turn reduced to this terminal minus the previous
    /// turn's stored terminal, accepted only when it reproduces the turn's own
    /// per-message usage.
    SessionDelta,
    /// OB24: a resumed Claude turn whose token delta does not reproduce its
    /// own messages (a cancelled turn in between stored no terminal, or usage
    /// never streamed), but whose session cost rose since the stored
    /// baseline. The cost is that rise, which can cover the runs in between
    /// (the row's `usage_json.spanRuns`); the tokens are this turn's
    /// main-model usage, as [`Basis::ResumeUnverified`] records them.
    SessionDeltaSpan,
    /// A resumed Claude turn with no verifiable baseline: main-model tokens
    /// only, no cost, marked partial.
    ResumeUnverified,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Basis::PerRun => "per_run",
            Basis::PerRunVerified => "per_run_verified",
            Basis::SessionDelta => "session_delta",
            Basis::SessionDeltaSpan => "session_delta_span",
            Basis::ResumeUnverified => "resume_unverified",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub tokens: Option<TokenCounts>,
    pub cost_usd: Option<f64>,
    /// Normalized model names, most tokens first.
    pub models: Vec<String>,
    /// False when the totals are known to be incomplete (no terminal, or a
    /// resumed turn that could not be verified).
    pub complete: bool,
    pub basis: Basis,
    /// [`Basis::SessionDeltaSpan`] only: tokens the session reported since
    /// the baseline but this turn did not stream (its delta minus this
    /// turn's per-message sum, never below 0). The runs in between are in
    /// it, and so are side-model tokens and any start-of-message snapshot's
    /// shortfall, which never stream.
    pub unstreamed_tokens: Option<u64>,
}

fn model_names(models: &[(String, TokenCounts)]) -> Vec<String> {
    models.iter().map(|(name, _)| name.clone()).collect()
}

/// Decide one invocation's figures. `None` when nothing was observed.
/// `resumed` is true when the run continued an earlier native session;
/// `previous` is that session's newest stored terminal, when known.
/// `may_span` is false when a row between that terminal and this run already
/// has a cost (a turn guard's estimate): then no [`Basis::SessionDeltaSpan`]
/// is recorded, since its cost rise would count that spend again.
pub fn resolve(
    acc: &UsageAccumulator,
    agent: Option<AgentKind>,
    resumed: bool,
    previous: Option<&TerminalRecord>,
    may_span: bool,
) -> Option<Resolved> {
    let Some(terminal) = acc.terminal() else {
        let tokens = acc
            .partial_turn
            .as_ref()
            .and_then(|t| t.tokens)
            .or_else(|| acc.message_total())?;
        return Some(Resolved {
            tokens: Some(tokens),
            cost_usd: None,
            models: model_names(&sorted_models(acc.message_by_model())),
            complete: false,
            basis: Basis::PerRun,
            unstreamed_tokens: None,
        });
    };
    // Only Claude's resume report is cumulative (Codex reports per turn and
    // Antigravity reports nothing).
    if !resumed || agent != Some(AgentKind::Claude) {
        return Some(Resolved {
            tokens: terminal.tokens,
            cost_usd: terminal.cost_usd,
            models: model_names(&terminal.by_model),
            complete: true,
            basis: Basis::PerRun,
            unstreamed_tokens: None,
        });
    }
    if let Some(resolved) = previous.and_then(|prev| session_delta(acc, terminal, prev)) {
        return Some(resolved);
    }
    let messages = acc.message_by_model();
    // Every model both sides report agrees: the terminal is this
    // invocation's own (side models never stream, so they are not compared).
    if acc.self_check() == SelfCheck::Matched {
        return Some(Resolved {
            tokens: terminal.tokens,
            cost_usd: terminal.cost_usd,
            models: model_names(&terminal.by_model),
            complete: true,
            basis: Basis::PerRunVerified,
            unstreamed_tokens: None,
        });
    }
    // The top-level `usage` is exact for the main model even on resume. A
    // budget stop zeroes that block, so fall back to the message sum.
    let own_tokens = terminal.main_usage.or_else(|| acc.message_total());
    // OB24: the delta did not reproduce this turn's messages, but the
    // session's cost rose since the baseline. That rise was spent, by this
    // turn and any run in between that stored no terminal (a cancelled
    // turn), so the row records it once as a span. Not when one of those
    // runs already has a cost of its own (`may_span`).
    let span = previous
        .filter(|_| may_span)
        .and_then(|prev| session_span(acc, terminal, prev));
    if let Some(span) = span {
        return Some(Resolved {
            tokens: own_tokens,
            cost_usd: Some(span.cost_usd),
            models: span.models,
            complete: true,
            basis: Basis::SessionDeltaSpan,
            unstreamed_tokens: Some(span.unstreamed_tokens),
        });
    }
    // No verifiable baseline (the first resumed turn after an upgrade, a
    // pruned or failed previous row) or no cost rise: main-model tokens; the
    // side-model gap is why this is partial.
    let tokens = own_tokens?;
    let main_model = sorted_models(messages)
        .into_iter()
        .map(|(name, _)| name)
        .next()
        .or_else(|| terminal.by_model.first().map(|(name, _)| name.clone()));
    Some(Resolved {
        tokens: Some(tokens),
        cost_usd: None,
        models: main_model.into_iter().collect(),
        complete: false,
        basis: Basis::ResumeUnverified,
        unstreamed_tokens: None,
    })
}

/// `terminal − previous` by model, or `None` when the baseline names a model
/// the terminal lacks or any field decreased: then it is not an earlier
/// state of this cumulative report.
fn cumulative_delta(
    terminal: &ReportedUsage,
    previous: &TerminalRecord,
) -> Option<BTreeMap<String, TokenCounts>> {
    let now = TerminalRecord::from(terminal).by_model;
    if previous
        .by_model
        .keys()
        .any(|model| !now.contains_key(model))
    {
        return None;
    }
    let mut delta: BTreeMap<String, TokenCounts> = BTreeMap::new();
    for (model, tokens) in &now {
        let before = previous.by_model.get(model).copied().unwrap_or_default();
        delta.insert(model.clone(), tokens.checked_sub(&before)?);
    }
    Some(delta)
}

/// A resumed turn's share when the strict check failed (OB24).
struct Span {
    cost_usd: f64,
    models: Vec<String>,
    unstreamed_tokens: u64,
}

/// The cost `terminal − previous` when both report one and it rose, from a
/// baseline [`cumulative_delta`] accepts. `unstreamed_tokens` is that
/// delta's total minus this turn's per-message sum, never below 0: what the
/// session reported but this turn did not stream.
fn session_span(
    acc: &UsageAccumulator,
    terminal: &ReportedUsage,
    previous: &TerminalRecord,
) -> Option<Span> {
    let cost_usd = terminal.cost_usd? - previous.cost_usd?;
    if !cost_usd.is_finite() || cost_usd <= 1e-12 {
        return None;
    }
    let mut delta = cumulative_delta(terminal, previous)?;
    let cumulative = delta
        .values()
        .map(TokenCounts::total)
        .fold(0u64, u64::saturating_add);
    let streamed = acc.message_total().map(|t| t.total()).unwrap_or(0);
    delta.retain(|_, tokens| !tokens.is_zero());
    Some(Span {
        cost_usd,
        models: model_names(&sorted_models(delta)),
        unstreamed_tokens: cumulative.saturating_sub(streamed),
    })
}

/// `terminal − previous`, accepted only when, for every model this turn's
/// messages reported, the delta equals that model's per-message sum, and no
/// field decreased anywhere.
fn session_delta(
    acc: &UsageAccumulator,
    terminal: &ReportedUsage,
    previous: &TerminalRecord,
) -> Option<Resolved> {
    let messages = acc.message_by_model();
    if messages.is_empty() {
        return None;
    }
    let mut delta = cumulative_delta(terminal, previous)?;
    for (model, counted) in &messages {
        if delta.get(model) != Some(counted) {
            return None;
        }
    }
    let tokens = delta
        .values()
        .copied()
        .fold(TokenCounts::default(), TokenCounts::add)
        .reported();
    let (cost_usd, cost_known) = match (terminal.cost_usd, previous.cost_usd) {
        (None, _) => (None, true),
        (Some(after), Some(before)) if after - before > 1e-12 => (Some(after - before), true),
        // A reported cost we cannot reduce is left out, and the row says so.
        _ => (None, false),
    };
    delta.retain(|_, tokens| !tokens.is_zero());
    Some(Resolved {
        tokens,
        cost_usd,
        models: model_names(&sorted_models(delta)),
        complete: cost_known,
        basis: Basis::SessionDelta,
        unstreamed_tokens: None,
    })
}

// ── Row vocabulary ──────────────────────────────────────────────────────────

/// How the account behind a run is billed, decided at the launch point.
/// Subscription only on positive evidence; unknown otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Billing {
    Api,
    Subscription,
    Unknown,
    /// A self-hosted model (Q8, 2026-11-03): billed by the GPU time of the
    /// person's own endpoint, never per token. Its cost is unknown, never $0
    /// and never Claude Code's own estimate for an id it doesn't know.
    SelfHosted,
}

impl Billing {
    pub fn as_str(self) -> &'static str {
        match self {
            Billing::Api => "api",
            Billing::Subscription => "subscription",
            Billing::Unknown => "unknown",
            Billing::SelfHosted => "self-hosted",
        }
    }
}

/// What a row's cost figure means (gate D2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostStatus {
    /// The agent reported a cost and the account bills by API key.
    Reported,
    /// The agent reported a cost, but it is a list-price estimate (a
    /// subscription login, or billing Hollow could not determine).
    Estimated,
    /// Complete token counts, no price (Codex always).
    Unpriced,
    /// Incomplete totals: the run was killed, or a resumed turn could not be
    /// verified.
    Partial,
    /// No usable figures at all.
    Unknown,
}

impl CostStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CostStatus::Reported => "reported",
            CostStatus::Estimated => "estimated",
            CostStatus::Unpriced => "unpriced",
            CostStatus::Partial => "partial",
            CostStatus::Unknown => "unknown",
        }
    }
}

/// How a run ended, as the ledger records it. `budget_stopped` is PR 8b's
/// budget stop, two ways: the budget guard's kill (`Ended::LimitReached`: no
/// result envelope, so its tokens are the partial sum of what streamed and
/// the row is `partial`), or Claude's `--max-budget-usd` backstop exit
/// (`error_max_budget_usd`: a full result, so the row is recorded from
/// `modelUsage` as usual, `reported` or `estimated`). `Interrupted` is
/// written by SQL (the reconcilers), so Rust never constructs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerOutcome {
    Running,
    Success,
    Error,
    Cancelled,
    Timeout,
    /// The agent never started: a validation refusal, a spawn error, or the
    /// build_command "unavailable" sentinel.
    SpawnFailed,
    /// Found still running at startup, or after its parent finished.
    #[allow(dead_code)] // written by SQL (the reconcilers), never by Rust
    Interrupted,
    /// Dropped without a finish (a panic, a dropped future).
    Abandoned,
    /// A usage budget's guard ended the run mid-way (Paperclip PR 8b).
    BudgetStopped,
}

impl LedgerOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            LedgerOutcome::Running => "running",
            LedgerOutcome::Success => "success",
            LedgerOutcome::Error => "error",
            LedgerOutcome::Cancelled => "cancelled",
            LedgerOutcome::Timeout => "timeout",
            LedgerOutcome::SpawnFailed => "spawn_failed",
            LedgerOutcome::Interrupted => "interrupted",
            LedgerOutcome::Abandoned => "abandoned",
            LedgerOutcome::BudgetStopped => "budget_stopped",
        }
    }
}

/// The outcome a run dropped without a finish records: `cancelled` while the
/// process is terminating (team-mcp's SIGTERM handler), else `abandoned`.
pub fn drop_outcome(terminating: bool) -> LedgerOutcome {
    if terminating {
        LedgerOutcome::Cancelled
    } else {
        LedgerOutcome::Abandoned
    }
}

pub fn cost_status(billing: Billing, resolved: Option<&Resolved>) -> CostStatus {
    let Some(r) = resolved else {
        return CostStatus::Unknown;
    };
    if r.tokens.is_none() && r.cost_usd.is_none() {
        return CostStatus::Unknown;
    }
    if !r.complete {
        return CostStatus::Partial;
    }
    match (r.cost_usd, billing) {
        (_, Billing::SelfHosted) => CostStatus::Unpriced,
        (Some(_), Billing::Api) => CostStatus::Reported,
        (Some(_), _) => CostStatus::Estimated,
        (None, _) => CostStatus::Unpriced,
    }
}

// ── Billing probe ───────────────────────────────────────────────────────────

/// Where Claude Code keeps its per-user config, and how an alternate provider
/// shows itself. Shared with PR 8's quota widget, so both agree on
/// `CLAUDE_CONFIG_DIR`.
pub mod claude_env {
    use std::path::{Path, PathBuf};

    pub const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
    pub const PROVIDER_FLAGS: &[&str] = &[
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
    ];
    pub const PROVIDER_VALUES: &[&str] = &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL"];

    /// `CLAUDE_CONFIG_DIR` when set (and then only there), else `~/.claude`.
    pub fn config_dir(configured: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
        configured
            .map(Path::to_path_buf)
            .or_else(|| home.map(|h| h.join(".claude")))
    }

    /// The account files that may carry `oauthAccount`, in lookup order.
    pub fn account_files(configured: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
        match configured {
            Some(dir) => vec![dir.join(".claude.json"), dir.join(".config.json")],
            None => home
                .map(|h| {
                    vec![
                        h.join(".claude.json"),
                        h.join(".claude").join(".config.json"),
                    ]
                })
                .unwrap_or_default(),
        }
    }

    pub fn truthy(value: Option<&str>) -> bool {
        matches!(
            value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
            Some("1" | "true" | "yes" | "on")
        )
    }

    /// Any environment evidence that Claude talks to a provider other than
    /// the Anthropic account login.
    pub fn alt_provider(get: &dyn Fn(&str) -> Option<String>) -> bool {
        PROVIDER_FLAGS.iter().any(|key| truthy(get(key).as_deref()))
            || PROVIDER_VALUES
                .iter()
                .any(|key| get(key).is_some_and(|v| !v.trim().is_empty()))
    }
}

/// The directories the probe reads. Explicit so tests never depend on the
/// developer's machine.
#[derive(Debug, Clone)]
pub struct ProbeRoots {
    pub home: Option<PathBuf>,
    pub claude_config_dir: Option<PathBuf>,
    /// The spawn cwd: Claude applies `.claude/settings{,.local}.json` there.
    pub cwd: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub managed_settings: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CodexAuth {
    /// No readable `auth.json`.
    #[default]
    Missing,
    ApiKey,
    ChatGpt,
    /// Present but not recognizably either.
    Unrecognized,
}

/// Booleans and enums only: no key, token or account value is ever kept, so
/// Debug, a test message or a future log line cannot leak one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BillingProbe {
    pub anthropic_api_key: bool,
    pub claude_alt_provider: bool,
    pub claude_subscription: bool,
    pub codex_api_key_env: bool,
    pub openai_api_key_env: bool,
    pub codex_auth: CodexAuth,
    pub codex_foreign_provider: bool,
    /// `XAI_API_KEY` in the child environment (Grok then bills the API).
    pub grok_api_key_env: bool,
    /// `~/.grok/auth.json` holds an OAuth (SuperGrok) login.
    pub grok_login: bool,
}

/// Credential-bearing files are read only to extract a named field, and never
/// past this size.
const PROBE_FILE_MAX: u64 = 4 * 1024 * 1024;

fn read_capped(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > PROBE_FILE_MAX {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&read_capped(path)?).ok()
}

fn non_empty_str(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty())
}

/// What one Claude settings file says, as booleans. The parsed document never
/// leaves this function.
fn claude_settings_flags(path: &Path) -> (bool, bool) {
    let Some(settings) = read_json(path) else {
        return (false, false);
    };
    let env = settings.get("env").cloned().unwrap_or(Value::Null);
    let api_key = non_empty_str(env.get("ANTHROPIC_API_KEY"));
    let alt = settings.get("apiKeyHelper").is_some_and(|v| !v.is_null())
        || claude_env::PROVIDER_VALUES
            .iter()
            .any(|key| non_empty_str(env.get(*key)))
        || claude_env::PROVIDER_FLAGS.iter().any(|key| {
            env.get(*key)
                .is_some_and(|v| v.as_bool() == Some(true) || claude_env::truthy(v.as_str()))
        });
    (api_key, alt)
}

fn codex_auth(path: &Path) -> CodexAuth {
    let Some(auth) = read_json(path) else {
        return CodexAuth::Missing;
    };
    let mode = auth
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(|m| m.trim().to_ascii_lowercase());
    let has_tokens = auth.get("tokens").is_some_and(Value::is_object);
    match mode.as_deref() {
        Some("apikey" | "api_key" | "api-key") => CodexAuth::ApiKey,
        Some("chatgpt") if has_tokens => CodexAuth::ChatGpt,
        None if non_empty_str(auth.get("OPENAI_API_KEY")) => CodexAuth::ApiKey,
        _ => CodexAuth::Unrecognized,
    }
}

/// A top-level `model_provider` other than `openai`, before the first
/// `[table]` (a provider inside a profile table does not apply by default).
fn codex_foreign_provider(path: &Path) -> bool {
    let Some(text) = read_capped(path) else {
        return false;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return false;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "model_provider" {
            let value = value.split('#').next().unwrap_or("").trim();
            let value = value.trim_matches(|c| c == '"' || c == '\'');
            return !value.is_empty() && value != "openai";
        }
    }
    false
}

/// Read only the named fields, drop every value at once, and keep booleans.
/// `user_config_applies` is false for runs that pass `--ignore-user-config`
/// (the Codex transform); for Claude it is currently always true, because
/// Claude loads project and local settings natively. If Hollow ever passes
/// `--setting-sources user` for unattended Claude runs (a PR 5 backlog item),
/// stop reading the project and local scopes for those runs here.
pub fn probe_from(
    kind: AgentKind,
    env: &dyn Fn(&str) -> Option<String>,
    roots: &ProbeRoots,
    user_config_applies: bool,
) -> BillingProbe {
    let set = |key: &str| env(key).is_some_and(|v| !v.trim().is_empty());
    let mut probe = BillingProbe::default();
    match kind {
        AgentKind::Claude => {
            probe.anthropic_api_key = set("ANTHROPIC_API_KEY");
            probe.claude_alt_provider = claude_env::alt_provider(env);
            let configured = roots.claude_config_dir.as_deref();
            let home = roots.home.as_deref();
            let mut settings = Vec::new();
            if let Some(dir) = claude_env::config_dir(configured, home) {
                settings.push(dir.join("settings.json"));
            }
            if let Some(cwd) = &roots.cwd {
                settings.push(cwd.join(".claude").join("settings.json"));
                settings.push(cwd.join(".claude").join("settings.local.json"));
            }
            settings.push(roots.managed_settings.clone());
            for path in &settings {
                let (api_key, alt) = claude_settings_flags(path);
                probe.anthropic_api_key |= api_key;
                probe.claude_alt_provider |= alt;
            }
            probe.claude_subscription = claude_env::account_files(configured, home)
                .iter()
                .find_map(|path| read_json(path))
                .and_then(|account| {
                    account
                        .get("oauthAccount")
                        .and_then(|o| o.get("billingType"))
                        .and_then(Value::as_str)
                        .map(|billing| billing.trim().ends_with("_subscription"))
                })
                .unwrap_or(false);
            // A `claude setup-token` credential (Hollow's extra accounts)
            // exists only for plan subscriptions.
            probe.claude_subscription |= set(crate::claude_accounts::TOKEN_ENV);
        }
        AgentKind::Codex => {
            probe.codex_api_key_env = set("CODEX_API_KEY");
            probe.openai_api_key_env = set("OPENAI_API_KEY");
            let codex_home = roots
                .codex_home
                .clone()
                .or_else(|| roots.home.as_ref().map(|h| h.join(".codex")));
            if let Some(dir) = codex_home {
                probe.codex_auth = codex_auth(&dir.join("auth.json"));
                probe.codex_foreign_provider =
                    user_config_applies && codex_foreign_provider(&dir.join("config.toml"));
            }
        }
        AgentKind::Antigravity => {}
        AgentKind::Grok => {
            probe.grok_api_key_env = set("XAI_API_KEY");
            probe.grok_login = roots
                .home
                .as_ref()
                .is_some_and(|h| grok_login(&h.join(".grok").join("auth.json")));
        }
    }
    probe
}

/// Whether Grok's auth file holds a signed-in OAuth account: any entry with
/// an `auth_mode` and a non-empty refresh token. Only booleans leave here.
fn grok_login(path: &Path) -> bool {
    read_json(path)
        .and_then(|v| v.as_object().cloned())
        .is_some_and(|entries| {
            entries.values().any(|e| {
                e.get("auth_mode").and_then(Value::as_str).is_some()
                    && e.get("refresh_token")
                        .and_then(Value::as_str)
                        .is_some_and(|t| !t.is_empty())
            })
        })
}

/// Api only on key evidence; subscription only on positive evidence.
pub fn classify_with(kind: AgentKind, probe: &BillingProbe) -> Billing {
    match kind {
        AgentKind::Claude => {
            if probe.anthropic_api_key {
                Billing::Api
            } else if probe.claude_alt_provider {
                Billing::Unknown
            } else if probe.claude_subscription {
                Billing::Subscription
            } else {
                Billing::Unknown
            }
        }
        AgentKind::Codex => {
            if probe.codex_api_key_env {
                return Billing::Api;
            }
            match probe.codex_auth {
                CodexAuth::ApiKey => Billing::Api,
                // With ChatGPT tokens Codex uses the login, but an
                // OPENAI_API_KEY or a foreign provider muddies it.
                CodexAuth::ChatGpt if probe.openai_api_key_env || probe.codex_foreign_provider => {
                    Billing::Unknown
                }
                CodexAuth::ChatGpt => Billing::Subscription,
                CodexAuth::Missing if probe.openai_api_key_env => Billing::Api,
                CodexAuth::Missing | CodexAuth::Unrecognized => Billing::Unknown,
            }
        }
        AgentKind::Antigravity => Billing::Unknown,
        AgentKind::Grok => {
            if probe.grok_api_key_env {
                Billing::Api
            } else if probe.grok_login {
                Billing::Subscription
            } else {
                Billing::Unknown
            }
        }
    }
}

/// The environment a spawned child sees: the command's explicit overrides win
/// (a removal reads as absent), then Hollow's own environment.
pub(crate) fn command_env(cmd: &tokio::process::Command) -> impl Fn(&str) -> Option<String> {
    let overrides: HashMap<String, Option<String>> = cmd
        .as_std()
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();
    move |key: &str| match overrides.get(key) {
        Some(value) => value.clone(),
        None => std::env::var(key).ok(),
    }
}

const MANAGED_SETTINGS: &str = "/Library/Application Support/ClaudeCode/managed-settings.json";

fn roots_from(env: &dyn Fn(&str) -> Option<String>, cwd: Option<PathBuf>) -> ProbeRoots {
    let dir = |key: &str| env(key).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    ProbeRoots {
        home: dir("HOME").or_else(dirs::home_dir),
        claude_config_dir: dir(claude_env::CONFIG_DIR_ENV),
        cwd,
        codex_home: dir("CODEX_HOME"),
        managed_settings: PathBuf::from(MANAGED_SETTINGS),
    }
}

/// Classify the account a built command will use.
pub fn classify(
    kind: AgentKind,
    cmd: &tokio::process::Command,
    user_config_applies: bool,
) -> Billing {
    let env = command_env(cmd);
    // A self-hosted run (Q8): its credential is a Hollow run key, in
    // Claude Code's token variable or (Q9) the one Codex's provider reads.
    let run_key = |name: &str| env(name).is_some_and(|key| crate::self_hosted::is_run_key(&key));
    let self_hosted = match kind {
        AgentKind::Claude => run_key(crate::self_hosted::AUTH_TOKEN_ENV),
        AgentKind::Codex => run_key(crate::self_hosted::CODEX_KEY_ENV),
        AgentKind::Antigravity | AgentKind::Grok => false,
    };
    if self_hosted {
        return Billing::SelfHosted;
    }
    let cwd = cmd.as_std().get_current_dir().map(Path::to_path_buf);
    let roots = roots_from(&env, cwd);
    classify_with(kind, &probe_from(kind, &env, &roots, user_config_applies))
}

/// Classify from Hollow's own environment (a runner whose command is built
/// later, like the Codex transform).
pub fn classify_process_env(kind: AgentKind, user_config_applies: bool) -> Billing {
    let env = |key: &str| std::env::var(key).ok();
    let roots = roots_from(&env, None);
    classify_with(kind, &probe_from(kind, &env, &roots, user_config_applies))
}

#[cfg(test)]
pub(crate) mod tests {
    //! Fixture provenance: `fixtures/usage/README.md`. The Claude and Codex
    //! files are consented stdout recordings (2026-09-25), except the
    //! synthetic ones that cover shapes no capture produced: the two
    //! `codex-*` ones and `claude-subagent.jsonl` (OB05). Expected numbers
    //! are hand-computed from those files.
    use super::*;
    use crate::agent::{parse_line, ParseState};

    pub(crate) const ONESHOT: &str = include_str!("../fixtures/usage/claude-oneshot.jsonl");
    pub(crate) const CHAT_TOOL: &str = include_str!("../fixtures/usage/claude-chat-tool.jsonl");
    pub(crate) const CHAT_RESUME: &str = include_str!("../fixtures/usage/claude-chat-resume.jsonl");
    pub(crate) const MAX_BUDGET: &str = include_str!("../fixtures/usage/claude-max-budget.jsonl");
    /// Synthetic (OB05): a main thread that spawns a two-message subagent.
    pub(crate) const SUBAGENT: &str = include_str!("../fixtures/usage/claude-subagent.jsonl");
    /// Synthetic (OB22): a main thread that compacts between two messages.
    pub(crate) const COMPACTED: &str = include_str!("../fixtures/usage/claude-compacted.jsonl");
    pub(crate) const CODEX_EXEC: &str = include_str!("../fixtures/usage/codex-exec.jsonl");
    pub(crate) const CODEX_RESUME: &str = include_str!("../fixtures/usage/codex-resume.jsonl");
    pub(crate) const CODEX_ZERO: &str = include_str!("../fixtures/usage/codex-zero-usage.jsonl");
    pub(crate) const CODEX_FAILED: &str = include_str!("../fixtures/usage/codex-failed.jsonl");

    fn tc(input: u64, cached_input: u64, output: u64) -> TokenCounts {
        TokenCounts {
            input,
            cached_input,
            output,
        }
    }

    fn events(kind: AgentKind, text: &str) -> Vec<AgentEvent> {
        let mut state = ParseState::default();
        text.lines()
            .flat_map(|line| parse_line(kind, line, &mut state))
            .collect()
    }

    fn accumulate(kind: AgentKind, text: &str) -> UsageAccumulator {
        let mut acc = UsageAccumulator::default();
        for evt in events(kind, text) {
            acc.observe(&evt);
        }
        acc
    }

    /// Everything up to (not including) the `result` line: a run killed
    /// before it reported its totals.
    pub(crate) fn before_result(text: &str) -> String {
        text.lines()
            .take_while(|line| !line.contains("\"type\":\"result\""))
            .map(|line| format!("{line}\n"))
            .collect()
    }

    fn result_usage(text: &str) -> ReportedUsage {
        events(AgentKind::Claude, text)
            .into_iter()
            .find_map(|evt| match evt {
                AgentEvent::Result { usage, .. } => Some(usage),
                _ => None,
            })
            .expect("a result event")
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn claude_result_sums_model_usage_and_counts_cache_creation_as_input() {
        let usage = result_usage(ONESHOT);
        // opus 2 + 19489 + haiku 899 in; opus 4 + haiku 8 out.
        assert_eq!(usage.tokens, Some(tc(20_390, 10_704, 12)));
        assert!(close(usage.cost_usd.unwrap(), 0.1590798));
        assert_eq!(usage.by_model[0].0, "claude-opus-5-5");
        assert_eq!(usage.by_model[0].1, tc(19_491, 10_704, 4));
        assert_eq!(usage.by_model[1].0, "claude-haiku-4-5-20251001");
        // The top-level block is main-model-only: modelUsage wins.
        assert_eq!(usage.main_usage, Some(tc(19_491, 10_704, 4)));
        assert_ne!(usage.tokens, usage.main_usage);

        let chat = result_usage(CHAT_TOOL);
        assert_eq!(chat.tokens, Some(tc(19_034, 42_412, 87)));
        assert!(close(chat.cost_usd.unwrap(), 0.1556084));
    }

    #[test]
    fn a_budget_stop_zeroes_top_level_usage_so_model_usage_decides() {
        let evt = events(AgentKind::Claude, MAX_BUDGET)
            .into_iter()
            .find_map(|evt| match evt {
                AgentEvent::Result {
                    usage,
                    is_error,
                    subtype,
                    ..
                } => Some((usage, is_error, subtype)),
                _ => None,
            })
            .unwrap();
        let (usage, is_error, subtype) = evt;
        assert!(is_error);
        assert_eq!(subtype, "error_max_budget_usd");
        assert_eq!(usage.main_usage, None, "an all-zero block is not reported");
        assert_eq!(usage.tokens, Some(tc(18_931, 12_201, 1_288)));
        assert!(close(usage.cost_usd.unwrap(), 0.1731252));
    }

    #[test]
    fn claude_result_falls_back_to_the_usage_block_with_cache_creation_as_input() {
        let evt = serde_json::json!({
            "type": "result",
            "total_cost_usd": 1.25,
            "usage": {"input_tokens": 10, "cache_creation_input_tokens": 5,
                      "cache_read_input_tokens": 20, "output_tokens": 1800}
        });
        let usage = claude_result_usage(&evt);
        assert_eq!(usage.tokens, Some(tc(15, 20, 1_800)));
        assert_eq!(usage.cost_usd, Some(1.25));
        assert!(usage.by_model.is_empty());
    }

    #[test]
    fn zeroed_or_incomplete_envelopes_report_nothing() {
        let claude_error = serde_json::json!({
            "type": "result", "is_error": true, "subtype": "error_during_execution",
            "result": "Failed to authenticate", "total_cost_usd": 0,
            "usage": {"input_tokens": 0, "cache_creation_input_tokens": 0,
                      "cache_read_input_tokens": 0, "output_tokens": 0},
            "modelUsage": {}
        });
        let usage = claude_result_usage(&claude_error);
        assert_eq!((usage.tokens, usage.cost_usd), (None, None));
        assert!(usage.raw.is_some(), "the raw counters are kept");

        let zero = events(AgentKind::Codex, CODEX_ZERO);
        let codex = zero
            .iter()
            .find_map(|evt| match evt {
                AgentEvent::Result { usage, .. } => Some(usage.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!((codex.tokens, codex.cost_usd), (None, None));
        assert!(codex.raw.is_some());

        let partial = codex_turn_usage(&serde_json::json!({"usage": {"output_tokens": 25}}));
        assert_eq!(
            partial.tokens, None,
            "input missing means no data, not zero"
        );

        let mut acc = UsageAccumulator::default();
        for evt in &zero {
            acc.observe(evt);
        }
        acc.observe(&AgentEvent::Result {
            text: None,
            usage: usage.clone(),
            is_error: true,
            subtype: "error".into(),
            detail: Default::default(),
        });
        assert_eq!(acc.running_total(), None);
        assert_eq!(
            resolve(&acc, Some(AgentKind::Codex), false, None, true),
            None
        );
    }

    #[test]
    fn claude_repeated_message_usage_is_counted_once_by_message_id() {
        // max-budget: two assistant events for msg-005 with the same snapshot.
        let killed = before_result(MAX_BUDGET);
        let evts = events(AgentKind::Claude, &killed);
        let snapshots: Vec<_> = evts
            .iter()
            .filter(|e| matches!(e, AgentEvent::MessageUsage { .. }))
            .collect();
        assert_eq!(snapshots.len(), 2);
        let mut acc = UsageAccumulator::default();
        assert!(acc.observe(snapshots[0]));
        assert!(!acc.observe(snapshots[1]), "a repeat counts once");
        assert_eq!(acc.message_total(), Some(tc(18_026, 12_201, 3)));
        assert!(acc.terminal().is_none());
        // A larger snapshot of the same id raises the total by max-merge.
        assert!(acc.observe(&AgentEvent::MessageUsage {
            message_id: "msg-005".into(),
            model: Some("claude-opus-5-5".into()),
            tokens: tc(18_026, 12_201, 40),
            is_final: false,
            cache_write: 0,
            thread: None,
        }));
        assert_eq!(acc.message_total(), Some(tc(18_026, 12_201, 40)));
    }

    #[test]
    fn message_delta_finals_replace_start_of_message_snapshots() {
        // chat-tool msg-003: snapshot output 16, final 74; msg-004: 3 / 3.
        let acc = accumulate(AgentKind::Claude, &before_result(CHAT_TOOL));
        assert_eq!(acc.message_total(), Some(tc(18_119, 42_412, 77)));
        // A snapshot arriving after the final no longer changes it.
        let mut acc = acc;
        assert!(!acc.observe(&AgentEvent::MessageUsage {
            message_id: "msg-003".into(),
            model: None,
            tokens: tc(1, 1, 500),
            is_final: false,
            cache_write: 0,
            thread: None,
        }));
        // Without partial messages only snapshots exist (known undercount).
        let snapshot_only = accumulate(AgentKind::Claude, &before_result(ONESHOT));
        assert_eq!(snapshot_only.message_total(), Some(tc(19_491, 10_704, 4)));
    }

    #[test]
    fn terminal_usage_replaces_the_per_message_estimate_but_an_empty_terminal_does_not() {
        let mut acc = UsageAccumulator::default();
        let mut changed_on_result = false;
        for evt in events(AgentKind::Claude, ONESHOT) {
            let is_result = matches!(evt, AgentEvent::Result { .. });
            let changed = acc.observe(&evt);
            if is_result {
                changed_on_result = changed;
            }
        }
        assert!(changed_on_result);
        assert_eq!(acc.running_total(), Some(tc(20_390, 10_704, 12)));
        assert_eq!(acc.terminals(), 1);

        let mut killed = accumulate(AgentKind::Claude, &before_result(CHAT_TOOL));
        killed.observe(&AgentEvent::Result {
            text: Some("Failed to authenticate".into()),
            usage: ReportedUsage::default(),
            is_error: true,
            subtype: "error_during_execution".into(),
            detail: Default::default(),
        });
        assert_eq!(killed.running_total(), Some(tc(18_119, 42_412, 77)));
        assert_eq!(killed.terminals(), 1);
    }

    #[test]
    fn a_fresh_chat_turns_finals_match_its_main_model_usage() {
        let acc = accumulate(AgentKind::Claude, CHAT_TOOL);
        assert_eq!(acc.self_check(), SelfCheck::Matched);
        let resolved = resolve(&acc, Some(AgentKind::Claude), false, None, true).unwrap();
        assert_eq!(resolved.basis, Basis::PerRun);
        assert_eq!(resolved.tokens, Some(tc(19_034, 42_412, 87)));
        assert!(resolved.complete);
        assert_eq!(resolved.models[0], "claude-opus-5-5");
    }

    #[test]
    fn context_tokens_are_the_last_message_not_the_turn_total() {
        let acc = accumulate(AgentKind::Claude, CHAT_TOOL);
        let context = acc
            .context_tokens()
            .expect("a chat turn reports its context");
        let turn = acc.message_total().expect("per-message usage").total();
        assert!(context > 0 && context <= turn);
        // A whole-turn total (Codex) never stands in for the context.
        assert_eq!(
            accumulate(AgentKind::Codex, CODEX_EXEC).context_tokens(),
            None
        );
    }

    #[test]
    fn call_stats_count_distinct_messages_and_the_peak_main_prompt() {
        let stats = accumulate(AgentKind::Claude, CHAT_TOOL).call_stats();
        // Six usage events, two distinct ids (msg-003, msg-004).
        assert_eq!(stats.api_calls, Some(2));
        // msg-003: 2 + 18010 cache writes + 12201 cache reads = 30213;
        // msg-004: 2 + 105 + 30211 = 30318, the peak.
        assert_eq!(stats.peak_context_tokens, Some(30_318));
        // 18010 + 105, the result's own cache_creation_input_tokens.
        assert_eq!(stats.cache_write_tokens, Some(18_115));
        assert_eq!((stats.num_turns, stats.api_ms), (Some(2), Some(4_739)));
        assert_eq!(stats.subagents, None);

        // Killed before its result: the message figures, no terminal ones.
        let killed = accumulate(AgentKind::Claude, &before_result(CHAT_TOOL));
        let killed = killed.call_stats();
        assert_eq!(killed.api_calls, Some(2));
        assert_eq!((killed.num_turns, killed.api_ms), (None, None));

        // Codex reports none of them: absent, never 0.
        let codex = accumulate(AgentKind::Codex, CODEX_EXEC).call_stats();
        assert!(codex.is_empty(), "{codex:?}");
        assert_eq!(serde_json::to_value(&codex).unwrap(), json!({}));
    }

    /// #741 OB05 review: two terminals in one run (a fresh attempt, then a
    /// `--resume` of its session) sum `num_turns` (2 + 1) and
    /// `duration_api_ms` (4739 + 6990), where the last one used to win; the
    /// last terminal's own API time stays readable for Claude's ledger rule
    /// (`usage_ledger`: it is session-cumulative there).
    #[test]
    fn call_stats_sum_turns_and_api_time_over_every_terminal() {
        let mut acc = accumulate(AgentKind::Claude, CHAT_TOOL);
        for evt in events(AgentKind::Claude, CHAT_RESUME) {
            acc.observe(&evt);
        }
        assert_eq!(acc.terminals(), 2);
        let stats = acc.call_stats();
        assert_eq!(stats.num_turns, Some(3));
        assert_eq!(stats.api_ms, Some(4_739 + 6_990));
        assert_eq!(acc.last_api_ms(), Some(6_990));

        // A terminal without the figures (a killed run's empty envelope)
        // neither adds nor clears them.
        acc.observe(&AgentEvent::Result {
            text: None,
            usage: ReportedUsage::default(),
            is_error: true,
            subtype: "error_during_execution".into(),
            detail: Default::default(),
        });
        let stats = acc.call_stats();
        assert_eq!((stats.num_turns, stats.api_ms), (Some(3), Some(11_729)));
        assert_eq!(acc.last_api_ms(), Some(6_990));
    }

    /// Absent, never 0: messages that report no cache write leave
    /// `cacheWriteTokens` out, like the other figures.
    #[test]
    fn cache_write_tokens_are_absent_when_no_message_wrote_the_cache() {
        let mut acc = UsageAccumulator::default();
        acc.observe(&AgentEvent::MessageUsage {
            message_id: "msg-nw-1".into(),
            model: Some("claude-opus-5-5".into()),
            tokens: tc(10, 500, 20),
            is_final: true,
            cache_write: 0,
            thread: None,
        });
        let stats = acc.call_stats();
        assert_eq!(stats.api_calls, Some(1));
        assert_eq!(stats.cache_write_tokens, None);
        let doc = serde_json::to_value(&stats).unwrap();
        assert!(doc.get("cacheWriteTokens").is_none(), "{doc}");
    }

    #[test]
    fn subagent_messages_have_their_own_bucket_and_never_set_the_context() {
        let acc = accumulate(AgentKind::Claude, SUBAGENT);
        let stats = acc.call_stats();
        // Main thread msg-sa-1 (3 + 1000 + 20000 = 21003) and msg-sa-2
        // (2 + 400 + 21000 = 21402); the subagent's 5000-token cache write
        // is not the main context.
        assert_eq!(stats.api_calls, Some(2));
        assert_eq!(stats.peak_context_tokens, Some(21_402));
        // msg-sub-1: 10 + 5000 + 0 + 300 = 5310; msg-sub-2: 4 + 200 +
        // 5000 + 50 = 5254.
        assert_eq!(
            stats.subagents,
            Some(SubagentUsage {
                calls: 2,
                tokens: 10_564,
            })
        );
        // Cache writes count every thread: 1000 + 5000 + 200 + 400.
        assert_eq!(stats.cache_write_tokens, Some(6_600));
        assert_eq!((stats.num_turns, stats.api_ms), (Some(2), Some(5_123)));
        // msg-sa-2's prompt plus its 12 output tokens.
        assert_eq!(acc.context_tokens(), Some(21_414));
        // The run's totals still hold every thread, so they match modelUsage.
        assert_eq!(acc.message_total(), Some(tc(6_619, 46_000, 402)));
        assert_eq!(acc.self_check(), SelfCheck::Matched);

        // Cut while the subagent streams (through msg-sub-2's final): the
        // context is still msg-sa-1's 21003 + 40, not msg-sub-2's 5254.
        let mid: String = SUBAGENT
            .lines()
            .take(11)
            .map(|line| format!("{line}\n"))
            .collect();
        let mid = accumulate(AgentKind::Claude, &mid);
        assert_eq!(mid.context_tokens(), Some(21_043));
        assert_eq!(mid.call_stats().peak_context_tokens, Some(21_003));
    }

    /// OB22: after a `compact_boundary` the context is the post-compaction
    /// message's, never the pre-compaction peak, and the drop is no reset:
    /// the stream counts one compaction and the peak keeps the larger prompt.
    #[test]
    fn context_tokens_report_the_post_compaction_value() {
        let mut state = ParseState::default();
        let mut acc = UsageAccumulator::default();
        let mut before = None;
        for line in COMPACTED.lines() {
            if line.contains("compact_boundary") {
                before = acc.context_tokens();
            }
            for evt in parse_line(AgentKind::Claude, line, &mut state) {
                acc.observe(&evt);
            }
        }
        // msg-cp-1: 2 + 1000 cache writes + 160000 cache reads + 50 output.
        assert_eq!(before, Some(161_052));
        // msg-cp-2: 3 + 9000 + 0 + 20.
        assert_eq!(acc.context_tokens(), Some(9_023));
        assert_eq!(state.compactions, 1);
        assert!(state.compacted);
        let stats = acc.call_stats();
        assert_eq!(stats.api_calls, Some(2));
        assert_eq!(stats.peak_context_tokens, Some(161_002));
    }

    #[test]
    fn codex_turn_completed_normalizes_cached_as_a_subset_of_input() {
        let acc = accumulate(AgentKind::Codex, CODEX_EXEC);
        let resolved = resolve(&acc, Some(AgentKind::Codex), false, None, true).unwrap();
        assert_eq!(resolved.tokens, Some(tc(11_849, 11_904, 5)));
        assert_eq!(resolved.cost_usd, None);
        assert_eq!(
            cost_status(Billing::Subscription, Some(&resolved)),
            CostStatus::Unpriced
        );
    }

    #[test]
    fn a_resumed_codex_turn_is_per_turn_and_unpriced() {
        let acc = accumulate(AgentKind::Codex, CODEX_RESUME);
        let resolved = resolve(&acc, Some(AgentKind::Codex), true, None, true).unwrap();
        assert_eq!(resolved.basis, Basis::PerRun);
        assert_eq!(resolved.tokens, Some(tc(219, 23_552, 5)));
        assert!(resolved.complete);
        assert_eq!(
            cost_status(Billing::Unknown, Some(&resolved)),
            CostStatus::Unpriced
        );
    }

    #[test]
    fn failed_codex_turns_and_antigravity_report_no_usage() {
        let acc = accumulate(AgentKind::Codex, CODEX_FAILED);
        assert_eq!(
            resolve(&acc, Some(AgentKind::Codex), false, None, true),
            None
        );
        let agy = r#"{"event":"init","conversation_id":"conversation-one","init":{"cwd":"/tmp"}}
{"event":"step_update","step_update":{"step_index":2,"state":"RUNNING","step_type":"agent_response","text_delta":"line one"}}
{"event":"result","result":{"conversation_id":"conversation-one","status":"SUCCESS","response":"line one"}}"#;
        let acc = accumulate(AgentKind::Antigravity, agy);
        assert_eq!(acc.terminals(), 1);
        assert_eq!(
            resolve(&acc, Some(AgentKind::Antigravity), false, None, true),
            None
        );
        assert_eq!(cost_status(Billing::Unknown, None), CostStatus::Unknown);
    }

    /// agy 1.5.13, recorded 2026-11-20 (gemini-4-argon-low, bypass): a
    /// two-call turn with a shell tool between, then a one-call resume whose
    /// `result` is session-cumulative (36744 input = 18912 + 17832).
    pub(crate) const AGY_TURN: &str = r#"{"event":"init","conversation_id":"835ce2eb","init":{"model":"gemini-4-argon-low","cwd":"/tmp"}}
{"event":"step_update","step_update":{"conversation_id":"835ce2eb","step_index":0,"state":"DONE","step_type":"user_input"}}
{"event":"step_update","step_update":{"conversation_id":"835ce2eb","step_index":1,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":16725,"output_tokens":42,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":16767}}}
{"event":"step_update","step_update":{"conversation_id":"835ce2eb","step_index":2,"state":"DONE","step_type":"tool","tool_name":"run_command"}}
{"event":"step_update","step_update":{"conversation_id":"835ce2eb","step_index":3,"state":"ACTIVE","step_type":"agent_response","text_delta":"done"}}
{"event":"step_update","step_update":{"conversation_id":"835ce2eb","step_index":3,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":2187,"output_tokens":5,"thinking_tokens":0,"cache_read_tokens":14671,"total_tokens":2192}}}
{"event":"step_update","step_update":{"conversation_id":"835ce2eb","step_index":3,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":2187,"output_tokens":5,"thinking_tokens":0,"cache_read_tokens":14671,"total_tokens":2192}}}
{"event":"result","result":{"conversation_id":"835ce2eb","status":"SUCCESS","response":"done","num_turns":1,"usage":{"input_tokens":18912,"output_tokens":47,"thinking_tokens":0,"cache_read_tokens":14671,"total_tokens":18959}}}"#;
    pub(crate) const AGY_RESUME: &str = r#"{"event":"init","conversation_id":"835ce2eb","init":{"model":"gemini-4-argon-low","cwd":"/tmp"}}
{"event":"step_update","step_update":{"conversation_id":"835ce2eb","step_index":6,"state":"DONE","step_type":"agent_response","usage":{"input_tokens":17832,"output_tokens":5,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":17837}}}
{"event":"result","result":{"conversation_id":"835ce2eb","status":"SUCCESS","response":"ok","num_turns":2,"usage":{"input_tokens":36744,"output_tokens":52,"thinking_tokens":0,"cache_read_tokens":14671,"total_tokens":36796}}}"#;

    /// Antigravity reports tokens per finished step; a turn is its own steps
    /// (a repeated step counts once), the cumulative `result` on a resume is
    /// never used, the model is the base (no `-low`), and each step is one
    /// message for the context gauge.
    #[test]
    fn antigravity_turn_usage_is_its_own_steps_even_on_resume() {
        let acc = accumulate(AgentKind::Antigravity, AGY_TURN);
        let turn = resolve(&acc, Some(AgentKind::Antigravity), false, None, true).unwrap();
        assert_eq!(turn.tokens, Some(tc(18_912, 14_671, 47)));
        assert_eq!(turn.models, vec!["gemini-4-argon".to_string()]);
        assert!(turn.complete);
        // The last call's prompt (uncached 2187 + 14671 read) is the context.
        assert_eq!(acc.context_tokens(), Some(2_187 + 14_671 + 5));
        let resumed = accumulate(AgentKind::Antigravity, AGY_RESUME);
        let turn = resolve(&resumed, Some(AgentKind::Antigravity), true, None, true).unwrap();
        assert_eq!(
            turn.tokens,
            Some(tc(17_832, 0, 5)),
            "this invocation's step, not the session's 36744"
        );
        let none = accumulate(
            AgentKind::Antigravity,
            r#"{"event":"result","result":{"conversation_id":"c","status":"SUCCESS","response":"x","usage":{"input_tokens":9,"output_tokens":9}}}"#,
        );
        assert_eq!(
            resolve(&none, Some(AgentKind::Antigravity), false, None, true),
            None,
            "a result with no step usage is not counted"
        );
    }

    #[test]
    fn model_names_drop_the_context_suffix() {
        assert_eq!(normalize_model("claude-opus-5-5[1m]"), "claude-opus-5-5");
        assert_eq!(normalize_model(" gpt-5 "), "gpt-5");
        assert_eq!(normalize_model("[weird]"), "[weird]");
    }

    #[test]
    fn a_resumed_cumulative_claude_turn_is_reduced_to_an_exact_delta() {
        let previous = TerminalRecord::from(&result_usage(CHAT_TOOL));
        let acc = accumulate(AgentKind::Claude, CHAT_RESUME);
        assert_eq!(
            acc.self_check(),
            SelfCheck::Mismatched,
            "the terminal is cumulative"
        );
        let resolved = resolve(&acc, Some(AgentKind::Claude), true, Some(&previous), true).unwrap();
        assert_eq!(resolved.basis, Basis::SessionDelta);
        // Not the cumulative {19182, 72623, 90}: exactly this invocation,
        // which is also what its top-level usage and message_delta say.
        assert_eq!(resolved.tokens, Some(tc(148, 30_211, 3)));
        assert!(close(resolved.cost_usd.unwrap(), 0.0072782));
        assert!(resolved.complete);
        assert_eq!(resolved.models, vec!["claude-opus-5-5".to_string()]);
        assert_eq!(resolved.unstreamed_tokens, None);

        // A baseline that does not reproduce the messages is not trusted as
        // an exact delta; its cost still rose, so it is a span (OB24, below).
        let mut wrong = previous.clone();
        wrong
            .by_model
            .insert("claude-opus-5-5".into(), tc(18_000, 42_412, 77));
        let fallback = resolve(&acc, Some(AgentKind::Claude), true, Some(&wrong), true).unwrap();
        assert_eq!(fallback.basis, Basis::SessionDeltaSpan);
        assert_eq!(fallback.unstreamed_tokens, Some(119));
        // A baseline larger than the terminal (a decrease) is refused too.
        let mut larger = previous;
        larger
            .by_model
            .insert("claude-haiku-4-5-20251001".into(), tc(99_999, 0, 10));
        let refused = resolve(&acc, Some(AgentKind::Claude), true, Some(&larger), true).unwrap();
        assert_eq!(refused.basis, Basis::ResumeUnverified);
    }

    /// OB24: a cancelled turn in between stored no terminal, so the stored
    /// baseline is two runs back and the token delta (8,000 cache reads the
    /// cancelled turn spent) exceeds what this turn streamed. The cost rise
    /// is still real spend: recorded once, as a span.
    #[test]
    fn a_resumed_turn_failing_the_strict_check_is_priced_as_a_span_when_its_cost_rose() {
        let previous = TerminalRecord::from(&result_usage(CHAT_TOOL));
        let resumed = CHAT_RESUME
            .replace(
                "\"total_cost_usd\":0.16288660000000002",
                "\"total_cost_usd\":0.5256084",
            )
            .replace(
                "\"cacheReadInputTokens\":72623",
                "\"cacheReadInputTokens\":80623",
            );
        let acc = accumulate(AgentKind::Claude, &resumed);
        let span = resolve(&acc, Some(AgentKind::Claude), true, Some(&previous), true).unwrap();
        assert_eq!(span.basis, Basis::SessionDeltaSpan);
        assert!(close(span.cost_usd.unwrap(), 0.37), "{:?}", span.cost_usd);
        assert!(span.complete);
        // The row's tokens stay this turn's main-model usage.
        assert_eq!(span.tokens, Some(tc(148, 30_211, 3)));
        assert_eq!(span.unstreamed_tokens, Some(8_000));
        assert_eq!(span.models, vec!["claude-opus-5-5".to_string()]);
        assert_eq!(
            cost_status(Billing::Subscription, Some(&span)),
            CostStatus::Estimated
        );

        // A row in between already has a cost (a turn guard's estimate): the
        // rise would count it again, so no span.
        let guarded = resolve(&acc, Some(AgentKind::Claude), true, Some(&previous), false).unwrap();
        assert_eq!(guarded.basis, Basis::ResumeUnverified);
        assert_eq!((guarded.cost_usd, guarded.unstreamed_tokens), (None, None));

        // No baseline: today's unverified resume.
        let none = resolve(&acc, Some(AgentKind::Claude), true, None, true).unwrap();
        assert_eq!(none.basis, Basis::ResumeUnverified);
        assert_eq!((none.cost_usd, none.unstreamed_tokens), (None, None));
        // A cost that did not rise, or a baseline without one, is no span.
        for cost in [Some(0.5256084), Some(0.6), None] {
            let mut flat = previous.clone();
            flat.cost_usd = cost;
            let kept = resolve(&acc, Some(AgentKind::Claude), true, Some(&flat), true).unwrap();
            assert_eq!(kept.basis, Basis::ResumeUnverified, "{cost:?}");
            assert_eq!(kept.cost_usd, None);
        }
    }

    #[test]
    fn a_resumed_claude_turn_without_a_verifiable_baseline_is_partial_without_cost() {
        let acc = accumulate(AgentKind::Claude, CHAT_RESUME);
        let resolved = resolve(&acc, Some(AgentKind::Claude), true, None, true).unwrap();
        assert_eq!(resolved.basis, Basis::ResumeUnverified);
        // The top-level block: main model, this invocation only.
        assert_eq!(resolved.tokens, Some(tc(148, 30_211, 3)));
        assert_eq!(resolved.cost_usd, None);
        assert!(!resolved.complete);
        assert_eq!(
            cost_status(Billing::Subscription, Some(&resolved)),
            CostStatus::Partial
        );
    }

    #[test]
    fn a_resumed_claude_turn_whose_terminal_equals_its_messages_is_trusted() {
        // Synthetic per-run terminal (not what the CLI does today).
        let mut acc = UsageAccumulator::default();
        acc.observe(&AgentEvent::MessageUsage {
            message_id: "m".into(),
            model: Some("claude-opus-5-5".into()),
            tokens: tc(302, 40_000, 50),
            is_final: true,
            cache_write: 0,
            thread: None,
        });
        acc.observe(&AgentEvent::Result {
            text: None,
            usage: ReportedUsage {
                tokens: Some(tc(302, 40_000, 50)),
                cost_usd: Some(0.0325),
                by_model: vec![("claude-opus-5-5".into(), tc(302, 40_000, 50))],
                main_usage: Some(tc(302, 40_000, 50)),
                raw: None,
                num_turns: None,
                api_ms: None,
            },
            is_error: false,
            subtype: "success".into(),
            detail: Default::default(),
        });
        assert_eq!(acc.self_check(), SelfCheck::Matched);
        let resolved = resolve(&acc, Some(AgentKind::Claude), true, None, true).unwrap();
        assert_eq!(resolved.basis, Basis::PerRunVerified);
        assert_eq!(resolved.cost_usd, Some(0.0325));
        assert!(resolved.complete);
    }

    #[test]
    fn cost_status_follows_billing_and_completeness() {
        let r = |tokens: Option<TokenCounts>, cost: Option<f64>, complete: bool| Resolved {
            tokens,
            cost_usd: cost,
            models: vec![],
            complete,
            basis: Basis::PerRun,
            unstreamed_tokens: None,
        };
        let t = Some(tc(1, 0, 1));
        assert_eq!(
            cost_status(Billing::Api, Some(&r(t, Some(0.1), true))),
            CostStatus::Reported
        );
        assert_eq!(
            cost_status(Billing::Subscription, Some(&r(t, Some(0.1), true))),
            CostStatus::Estimated
        );
        assert_eq!(
            cost_status(Billing::Unknown, Some(&r(t, Some(0.1), true))),
            CostStatus::Estimated
        );
        assert_eq!(
            cost_status(Billing::Api, Some(&r(t, None, true))),
            CostStatus::Unpriced
        );
        assert_eq!(
            cost_status(Billing::Api, Some(&r(t, None, false))),
            CostStatus::Partial
        );
        assert_eq!(cost_status(Billing::Api, None), CostStatus::Unknown);
        assert_eq!(
            cost_status(Billing::Api, Some(&r(None, None, true))),
            CostStatus::Unknown
        );
    }

    fn probe(f: impl FnOnce(&mut BillingProbe)) -> BillingProbe {
        let mut p = BillingProbe::default();
        f(&mut p);
        p
    }

    #[test]
    fn billing_requires_positive_evidence() {
        use AgentKind::*;
        assert_eq!(
            classify_with(Claude, &probe(|p| p.anthropic_api_key = true)),
            Billing::Api
        );
        assert_eq!(
            classify_with(Claude, &probe(|p| p.claude_subscription = true)),
            Billing::Subscription
        );
        assert_eq!(
            classify_with(Claude, &BillingProbe::default()),
            Billing::Unknown
        );
        assert_eq!(
            classify_with(
                Claude,
                &probe(|p| {
                    p.claude_subscription = true;
                    p.claude_alt_provider = true;
                })
            ),
            Billing::Unknown
        );
        assert_eq!(
            classify_with(Codex, &probe(|p| p.codex_api_key_env = true)),
            Billing::Api
        );
        assert_eq!(
            classify_with(Codex, &probe(|p| p.codex_auth = CodexAuth::ApiKey)),
            Billing::Api
        );
        assert_eq!(
            classify_with(Codex, &probe(|p| p.codex_auth = CodexAuth::ChatGpt)),
            Billing::Subscription
        );
        assert_eq!(
            classify_with(
                Codex,
                &probe(|p| {
                    p.codex_auth = CodexAuth::ChatGpt;
                    p.openai_api_key_env = true;
                })
            ),
            Billing::Unknown
        );
        assert_eq!(
            classify_with(
                Codex,
                &probe(|p| {
                    p.codex_auth = CodexAuth::ChatGpt;
                    p.codex_foreign_provider = true;
                })
            ),
            Billing::Unknown
        );
        assert_eq!(
            classify_with(Codex, &probe(|p| p.openai_api_key_env = true)),
            Billing::Api
        );
        assert_eq!(
            classify_with(Codex, &BillingProbe::default()),
            Billing::Unknown
        );
        assert_eq!(
            classify_with(Codex, &probe(|p| p.codex_auth = CodexAuth::Unrecognized)),
            Billing::Unknown
        );
        assert_eq!(
            classify_with(Antigravity, &probe(|p| p.anthropic_api_key = true)),
            Billing::Unknown
        );
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn roots(dir: &Path) -> ProbeRoots {
        ProbeRoots {
            home: Some(dir.join("home")),
            claude_config_dir: None,
            cwd: Some(dir.join("cwd")),
            codex_home: Some(dir.join("codex")),
            managed_settings: dir.join("managed").join("managed-settings.json"),
        }
    }

    #[test]
    fn grok_billing_follows_its_login_and_api_key() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        let none = |_: &str| None::<String>;
        assert_eq!(
            classify_with(
                AgentKind::Grok,
                &probe_from(AgentKind::Grok, &none, &r, true)
            ),
            Billing::Unknown
        );
        write(
            &d.path().join("home/.grok/auth.json"),
            r#"{"https://auth.x.ai::id":{"auth_mode":"oidc","refresh_token":"r","email":"x"}}"#,
        );
        assert_eq!(
            classify_with(
                AgentKind::Grok,
                &probe_from(AgentKind::Grok, &none, &r, true)
            ),
            Billing::Subscription
        );
        let key = |k: &str| (k == "XAI_API_KEY").then(|| "xai-1".to_string());
        assert_eq!(
            classify_with(
                AgentKind::Grok,
                &probe_from(AgentKind::Grok, &key, &r, true)
            ),
            Billing::Api
        );
    }

    #[test]
    fn an_account_token_in_the_child_env_is_a_subscription() {
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        let env = |key: &str| {
            (key == crate::claude_accounts::TOKEN_ENV).then(|| "sk-ant-oat01-x".to_string())
        };
        let probe = probe_from(AgentKind::Claude, &env, &r, true);
        assert!(probe.claude_subscription);
        assert_eq!(
            classify_with(AgentKind::Claude, &probe),
            Billing::Subscription
        );
    }

    #[test]
    fn billing_probe_reads_every_claude_settings_scope_and_only_the_config_dir_when_set() {
        let none = |_: &str| None::<String>;
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        assert_eq!(
            probe_from(AgentKind::Claude, &none, &r, true),
            BillingProbe::default()
        );

        write(
            &d.path().join("cwd/.claude/settings.local.json"),
            r#"{"env":{"ANTHROPIC_API_KEY":"sk-x"}}"#,
        );
        assert!(probe_from(AgentKind::Claude, &none, &r, true).anthropic_api_key);

        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        write(
            &d.path().join("home/.claude/settings.json"),
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://proxy"}}"#,
        );
        assert!(probe_from(AgentKind::Claude, &none, &r, true).claude_alt_provider);

        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        write(
            &r.managed_settings,
            r#"{"apiKeyHelper":"/usr/local/bin/key"}"#,
        );
        assert!(probe_from(AgentKind::Claude, &none, &r, true).claude_alt_provider);

        // CLAUDE_CONFIG_DIR set: only it is read, never ~/.claude.json.
        let d = tempfile::tempdir().unwrap();
        let mut r = roots(d.path());
        write(
            &d.path().join("home/.claude.json"),
            r#"{"oauthAccount":{"billingType":"stripe_subscription"}}"#,
        );
        assert!(probe_from(AgentKind::Claude, &none, &r, true).claude_subscription);
        r.claude_config_dir = Some(d.path().join("config"));
        std::fs::create_dir_all(d.path().join("config")).unwrap();
        assert!(!probe_from(AgentKind::Claude, &none, &r, true).claude_subscription);
        write(
            &d.path().join("config/.claude.json"),
            r#"{"oauthAccount":{"billingType":"stripe_subscription"}}"#,
        );
        assert!(probe_from(AgentKind::Claude, &none, &r, true).claude_subscription);

        // Codex: a provider under a profile table does not count; a top-level
        // one does, and neither counts when user config does not apply.
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        write(
            &d.path().join("codex/auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x"}}"#,
        );
        write(
            &d.path().join("codex/config.toml"),
            "model = \"gpt-5\"\n[profiles.x]\nmodel_provider = \"ollama\"\n",
        );
        let p = probe_from(AgentKind::Codex, &none, &r, true);
        assert_eq!(p.codex_auth, CodexAuth::ChatGpt);
        assert!(!p.codex_foreign_provider);
        write(
            &d.path().join("codex/config.toml"),
            "model_provider = \"ollama\"\n[profiles.x]\n",
        );
        assert!(probe_from(AgentKind::Codex, &none, &r, true).codex_foreign_provider);
        assert!(!probe_from(AgentKind::Codex, &none, &r, false).codex_foreign_provider);
        write(
            &d.path().join("codex/config.toml"),
            "model_provider = \"openai\"\n",
        );
        assert!(!probe_from(AgentKind::Codex, &none, &r, true).codex_foreign_provider);
        write(
            &d.path().join("codex/auth.json"),
            r#"{"auth_mode":"ApiKey"}"#,
        );
        assert_eq!(
            probe_from(AgentKind::Codex, &none, &r, true).codex_auth,
            CodexAuth::ApiKey
        );
    }

    const PROBED_KEYS: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "CODEX_API_KEY",
        "OPENAI_API_KEY",
    ];

    #[test]
    fn command_env_overrides_win_and_removals_hide() {
        let mut cmd = tokio::process::Command::new("/bin/true");
        for key in PROBED_KEYS {
            cmd.env_remove(key);
        }
        cmd.env("ANTHROPIC_API_KEY", "x");
        let env = command_env(&cmd);
        assert_eq!(env("ANTHROPIC_API_KEY").as_deref(), Some("x"));
        assert_eq!(
            env("OPENAI_API_KEY"),
            None,
            "a removal hides the process env"
        );
        cmd.env_remove("ANTHROPIC_API_KEY");
        assert_eq!(command_env(&cmd)("ANTHROPIC_API_KEY"), None);
    }

    #[test]
    fn billing_probe_holds_no_secret_values() {
        let secret = "sk-hollow-secret-test";
        let env = |key: &str| {
            matches!(
                key,
                "ANTHROPIC_API_KEY" | "OPENAI_API_KEY" | "CODEX_API_KEY"
            )
            .then(|| secret.to_string())
        };
        let d = tempfile::tempdir().unwrap();
        let r = roots(d.path());
        for kind in AgentKind::all() {
            let p = probe_from(kind, &env, &r, true);
            assert!(!format!("{p:?}").contains(secret));
        }
        assert!(probe_from(AgentKind::Claude, &env, &r, true).anthropic_api_key);
    }

    #[test]
    fn token_arithmetic_never_goes_negative() {
        assert_eq!(tc(5, 5, 5).checked_sub(&tc(1, 2, 3)), Some(tc(4, 3, 2)));
        assert_eq!(tc(5, 5, 5).checked_sub(&tc(6, 0, 0)), None);
        assert_eq!(tc(1, 9, 3).max_merge(tc(4, 2, 3)), tc(4, 9, 3));
        assert!(TokenCounts::default().is_zero());
    }

    /// 2026-10-07: Claude Code 2.1.286 can't price `claude-haiku-5-5`
    /// (`costBasis: "unknown"`) and estimated about 40× too high; Hollow
    /// prices it from the tokens at one rate, so resumed turns' differences
    /// stay right.
    #[test]
    fn a_model_the_cli_cannot_price_is_priced_from_its_tokens() {
        let evt = |m: u64| {
            json!({
                "total_cost_usd": 0.1971754,
                "usage": {"input_tokens": 2, "output_tokens": 4},
                "modelUsage": {"claude-haiku-5-5": {
                    "inputTokens": m, "outputTokens": m,
                    "cacheReadInputTokens": m, "cacheCreationInputTokens": m,
                    "costUSD": 0.1971754, "costBasis": "unknown"
                }}
            })
        };
        let one = claude_result_usage(&evt(1_000_000)).cost_usd.unwrap();
        assert!(close(one, 0.10 + 0.20 + 0.01 + 0.50), "{one}");
        // Linear in the tokens, so a resumed turn's difference is its own share.
        let two = claude_result_usage(&evt(2_000_000)).cost_usd.unwrap();
        assert!(close(two - one, one));
        // A model the CLI prices keeps the CLI's own total.
        let known = claude_result_usage(&json!({
            "total_cost_usd": 0.42,
            "modelUsage": {"claude-opus-5-5": {"inputTokens": 10, "outputTokens": 10, "costUSD": 0.42, "costBasis": "list"}}
        }));
        assert!(close(known.cost_usd.unwrap(), 0.42));
    }
}
