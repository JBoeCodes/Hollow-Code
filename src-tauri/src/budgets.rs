//! Usage budgets (Paperclip PR 8a, semantics per gate D2).
//!
//! **What this file holds.** The store (`budgets.json`), its schema and
//! validation, the per-run evaluation over PR 6's usage ledger, the claim-time
//! [`preflight`] (plus the read-only [`is_blocked`] and [`preflight_readonly`]),
//! the run-end [`after_run`], the pause lifecycle (a `budget_pause` review
//! item per pause episode; [`accept_pause`] "Raise and resume",
//! [`hold_pause_and_reject`] "Keep paused", [`resume`] from Settings), the
//! workflow rename and delete hooks, suggestions, the Settings [`view`], and
//! the tamper and corrupt-file handling, and PR 8b's mid-run enforcement
//! ([`BudgetGuard`], see below).
//!
//! **Store.** One global `budgets.json` beside `agent-settings.json`
//! ([`crate::agent_settings::app_data_dir`], outside the vault). Schema v1,
//! camelCase: `{schemaVersion:1, budgets:[BudgetDef], pauses:{<budgetId>:
//! BudgetPause}, notices:{<key>: rfc3339}}`. Writes are a private temp file
//! (0600, exclusive create) → fsync → rename, under [`BUDGETS_LOCK`], and only
//! by the GUI process; `hollow team-mcp` only reads. Every API takes an
//! explicit [`Store`]: production call sites use [`Store::for_app`], which
//! ignores the vault; test builds key the file by vault
//! (`{temp}/hollow-test-budgets-{pid}/{vault stem}/budgets.json`) so parallel
//! tests never share a store.
//!
//! **Failure handling.** A missing file means no budgets. An unreadable,
//! corrupt, unknown-schema or invalid file is never overwritten ([`Store::save`]
//! refuses to replace it), fails closed for enforced intents only
//! ([`load_failure_notice`]), and is reported by [`report_load_error`]: one
//! `budgets` health failure and one error notification per distinct error per
//! hour (PR 5's permission store uses 10 minutes per vault; an hour per error
//! is this plan's choice). Hollow rereads the file before every run, so a
//! repaired file takes effect without a restart, and [`report_load_ok`]
//! records the recovery.
//!
//! **Scopes and metrics.** Scopes `global`, `agent:<id>`, `workflow:<slug>`,
//! `heartbeat`. `tokens` = input (including cache creation) + output; cache
//! reads are shown, never counted. `costUsd` = the sum of reported and
//! estimated (Claude subscription, list-price; Codex at a price the person
//! entered, overnight 9 A13) costs; Codex and Antigravity report no price of
//! their own (Antigravity's is Hollow's estimate at Google's list rates), so
//! spending budgets that could never trip are refused
//! by [`validate_def`]. Windows are local day, week (Monday, `YYYY-Www`) and
//! month, from PR 6's one window function (`usage_ledger::window_bounds_in`).
//! Ledger rows that never spawned (validation refusals) are not counted.
//!
//! **Enforcement (D2).** Workflows from every source, the heartbeat, and a
//! delegate whose parent is an automation or unknown are enforced: a hard-stop
//! budget over its limit blocks them.
//! Ordinary chat, a delegate under chat, ingest and
//! one-shots only warn. Usage from every surface counts toward global and
//! agent totals, and a warn-only run that pushes a hard-stop budget over its
//! limit pauses the automations under it ([`after_run`]), never chat.
//!
//! **Where it runs.** [`preflight`] runs at the run-queue claim, LAST among the
//! claim gates (after PR 5's permission verify: a refused workflow never gets
//! budget bookkeeping), in the heartbeat's gates after preparation and before
//! the budget charge, and in chat's `execute_turn` after `runStarted` (warn
//! only). [`after_run`] runs in every lease-releasing funnel (workflow
//! `finalize`, chat `finish_observed_run`, heartbeat
//! `finish_heartbeat_history`) after the run's usage row and before
//! `finish_run` (cross-m2 §4 #6), so the next claim sees the finished run, and
//! while the run is still tracked, so its review-queue writes are attributed.
//! Every run-end step is infallible: errors are logged, never propagated.
//!
//! **Mid-run (Paperclip PR 8b).** The claim's [`RunBudget`] carries every
//! applicable token budget with its claim-time baseline ([`GuardCheck`]). A
//! [`BudgetGuard`] built from it checks the run's own running total (PR 6's
//! `UsageRun::running_total`) after every change: an enforced check crossed
//! mid-run stops the run (the caller requests `Supervision::limit`, the run
//! ends as `Ended::LimitReached`, its usage row records `budget_stopped` with
//! the partial sum of what streamed); every other crossing is a warning.
//! Totals are a lower bound (side models report only in the final result;
//! delegations write their own rows), and `after_run` catches up from the
//! ledger. Dollars are never checked mid-run (a stream reports no cost):
//! Claude's `--max-budget-usd` ([`RunBudget::max_budget_usd`], exit subtype
//! [`BACKSTOP_SUBTYPE`]) is the spending backstop, enforced by Claude only
//! after a message completes. Codex and Antigravity are checked at the claim
//! only ([`BudgetGuard::for_agent`]): their usage arrives once, when the work
//! is done. A result envelope is never checked ([`BudgetGuard::on_event`]),
//! so a run that finished its work is never turned into a budget stop by its
//! terminal total. team-mcp delegates call only [`preflight_readonly`].
//!
//! **One reservation (merged plan slice 3, review A·R6, decision 6).** A
//! guard's total is no longer just its own run: every usage row publishes
//! its running total to a per-vault in-flight reservation ([`reserve`],
//! [`release`], from `usage_ledger::UsageRun`), and a guard adds what every
//! other run under the same scope used since its claim, in-process runs at
//! once and other processes' (parent and team-mcp delegates) from the
//! ledger's checkpoints ([`RESERVATION_REFRESH`]). Dollars, which no stream
//! reports mid-run, are held at the claim instead: each live Claude claim
//! holds the `--max-budget-usd` backstop it was given ([`SpendGrant`]), and a
//! concurrent claim's backstop is what is left after every hold under the
//! same budget, with a one-cent floor. Holds are per process: a team-mcp
//! delegate sees the GUI's runs only through ledger checkpoints, not its holds.
//! At the claim, [`preflight_with_fallback`] moves a workflow whose own
//! agent is at its `agent:` ceiling to another agent with room
//! ([`RunBudget::fallback`], recorded on the receipt); with none the block
//! stands and ends with [`NO_ROOM_NOTE`]. Budgets are per scope, never per
//! Claude account, so another account never has room under one; running
//! out of an account's allowance is R1's chat failover, also on the receipt.
//!
//! **Pauses.** A hard-stop budget that trips opens one pause EPISODE (named
//! by its start, [`pause_fingerprint`]) and one sticky `budget_pause` review
//! item. An unheld pause lapses at its window reset (its item superseded); a
//! held one ("Keep paused") lasts until Resume. Accept, reject and resume
//! re-derive everything from `budgets.json` (the review payload is in the
//! vault and agent-writable) and refuse an item id the queue holds twice.
//!
//! **Tamper.** [`Store`] remembers the hash of the last file it wrote or first
//! trusted (per process). A host read-for-write that finds a different file
//! flags it once (a notification) and proceeds on the current file: a user's
//! hand edit is legitimate, so it is flagged, not refused. Not a sandbox: an
//! agent with Bash or bypassPermissions can edit the file.
//!
//! **Deviation (8a, resolved in 8b): a mid-run edit was flagged, not enforced at run end.**
//! The plan's run-end tamper case (test 34 part 1: an external rewrite of
//! `budgets.json` during a run pauses at the limit the CLAIM saw) is not
//! implemented. The claim's [`RunBudget`] (its `file_hash` and limits) is
//! dropped after the claim, so [`after_run`] evaluates the file as it is now:
//! an agent that raises a limit mid-run gets the new limit at run end, and the
//! only effect is the one "Budgets changed outside Hollow" notice from
//! [`Store::load_for_write`]. PR 8b carries the claim's `RunBudget` into the
//! run: a Claude run's mid-run [`BudgetGuard`] checks the CLAIM's token limits
//! in memory and never re-reads the file, so a mid-run edit cannot loosen the
//! kill, and workflow and heartbeat funnels call [`after_run_claimed`], which
//! judges a limit raised during the run at the claim's limit (test 34 part 1,
//! resolved).
//!
//! **Lock order (Milestone 2):** `automated_history` HISTORY_LOCK →
//! `workflow_permissions::write_guard` → (`vault::MUTATE_LOCK`, released
//! first) → [`BUDGETS_LOCK`] → `review_queue` QUEUE_LOCK → `run_guard`
//! TRACKED. Nothing under [`BUDGETS_LOCK`] may take the write guard or
//! HISTORY_LOCK, and the permission store's lock is never nested with it.
//! [`workflow_holds`] is computed apart from `permission_holds`, which takes
//! the write guard. The in-flight reservation's lock (`RESERVATIONS`) is a
//! leaf: nothing else is taken while it is held.

use crate::agent::AgentKind;
use crate::notifications::{self, NotificationInput};
use chrono::{DateTime, Local, SecondsFormat, TimeZone, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::Emitter;

pub(crate) use crate::usage_ledger::{window_bounds_in, Surface, UsageWindow, WindowBounds};

/// File name inside [`crate::agent_settings::app_data_dir`].
pub const FILE_NAME: &str = "budgets.json";
/// Review-queue kind of a hard-stop pause.
pub const REVIEW_KIND: &str = "budget_pause";
/// Review-queue source of a hard-stop pause.
pub const REVIEW_SOURCE: &str = "budget";
/// Tauri event emitted whenever budgets or pauses change.
pub const EVENT: &str = "budget-changed";
/// Every message that blocks a run starts with this.
pub const BUDGET_PAUSED_PREFIX: &str = "Budget paused:";
/// A run stopped mid-way (PR 8b) reports this.
pub const BUDGET_STOP_PREFIX: &str = "Budget limit reached:";
/// The run-queue hold note of a scheduled or trigger row a hard-stop budget
/// holds because usage or budget settings could not be read: the row waits
/// (like PR 5's unavailable permission records) instead of being consumed.
pub const UNAVAILABLE_HELD_NOTE: &str = "usage for a hard-stop budget could not be read";
/// Default warning threshold, percent of the limit.
pub const DEFAULT_WARN_PERCENT: u8 = 80;
/// "Suggest a limit" needs this much ledger history (D2: no default caps
/// until a week of data exists).
pub const SUGGESTION_MIN_DAYS: i64 = 7;
/// `.hollow-health.jsonl` subsystem id for store failures.
pub const HEALTH_SUBSYSTEM: &str = "budgets";
const SCHEMA_VERSION: u32 = 1;
/// One store-error notice per distinct error per this interval.
const LOAD_ERROR_INTERVAL: Duration = Duration::from_secs(60 * 60);
const REPAIR_HINT: &str =
    "Nothing was overwritten; move or repair the file. Hollow reads it again before each run.";
const ID_MAX: usize = 64;
const SLUG_MAX: usize = 200;
const HEALTH_DETAIL_CAP: usize = 300;

/// Serializes every host load → modify → save. Poison-tolerant.
pub(crate) static BUDGETS_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
/// Store path → sha256 of the file Hollow last wrote, or first trusted
/// (`None` = the file was absent). Trust on first use, per process.
static HOST_HASH: Lazy<Mutex<HashMap<PathBuf, Option<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// (store path, foreign hash) pairs already announced.
static FOREIGN_SEEN: Lazy<Mutex<HashSet<(PathBuf, Option<String>)>>> =
    Lazy::new(|| Mutex::new(HashSet::new()));
/// Store-error text → when it was last announced.
static LOAD_ERRORS: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// Canonical vaults whose store failure was reported in this process, so
/// [`report_load_ok`] records the recovery once.
static FAILURE_REPORTED: Lazy<Mutex<HashSet<PathBuf>>> = Lazy::new(|| Mutex::new(HashSet::new()));

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn rfc3339_millis<Tz: TimeZone>(at: &DateTime<Tz>) -> String {
    at.with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Serializes a [`UsageWindow`] as `"day"` / `"week"` / `"month"`.
mod window_serde {
    use super::UsageWindow;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(window: &UsageWindow, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(window.as_str())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<UsageWindow, D::Error> {
        let raw = String::deserialize(d)?;
        UsageWindow::parse(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown window '{raw}'")))
    }
}

fn window_noun(window: UsageWindow) -> &'static str {
    window.as_str()
}

fn window_adjective(window: UsageWindow) -> &'static str {
    match window {
        UsageWindow::Day => "today's",
        UsageWindow::Week => "this week's",
        UsageWindow::Month => "this month's",
    }
}

// ---------------------------------------------------------------------------
// Scopes, metrics, intents
// ---------------------------------------------------------------------------

/// What a budget counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Metric {
    /// Input (including cache creation) + output tokens.
    Tokens,
    /// Reported or estimated dollars.
    CostUsd,
}

impl Metric {
    pub fn as_str(self) -> &'static str {
        match self {
            Metric::Tokens => "tokens",
            Metric::CostUsd => "costUsd",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Metric::Tokens => "token",
            Metric::CostUsd => "spending",
        }
    }
}

/// Which usage a budget covers. Serialized as its [`Scope::key`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Scope {
    Global,
    Agent(AgentKind),
    Workflow(String),
    Heartbeat,
}

impl Scope {
    /// Parse `global`, `heartbeat`, `agent:<claude|codex|antigravity>` or
    /// `workflow:<slug>`. Agent aliases (`agy`) are canonicalized.
    pub fn parse(raw: &str) -> Result<Scope, String> {
        let raw = raw.trim();
        match raw {
            "global" => return Ok(Scope::Global),
            "heartbeat" => return Ok(Scope::Heartbeat),
            _ => {}
        }
        if let Some(agent) = raw.strip_prefix("agent:") {
            return AgentKind::from_opt(agent)
                .map(Scope::Agent)
                .ok_or_else(|| format!("Unknown agent '{agent}' in budget scope."));
        }
        if let Some(slug) = raw.strip_prefix("workflow:") {
            return if valid_workflow_slug(slug) {
                Ok(Scope::Workflow(slug.to_string()))
            } else {
                Err(format!("Invalid automation name '{slug}' in budget scope."))
            };
        }
        Err(format!("Unknown budget scope '{raw}'."))
    }

    pub fn key(&self) -> String {
        match self {
            Scope::Global => "global".into(),
            Scope::Agent(kind) => format!("agent:{}", kind.as_str()),
            Scope::Workflow(slug) => format!("workflow:{slug}"),
            Scope::Heartbeat => "heartbeat".into(),
        }
    }

    /// Lower-case label for sentences ("all usage", "the “brief” automation").
    pub fn label(&self) -> String {
        match self {
            Scope::Global => "all usage".into(),
            Scope::Agent(kind) => format!("{} usage", kind.label()),
            Scope::Workflow(slug) => format!("the “{slug}” automation"),
            Scope::Heartbeat => "the heartbeat".into(),
        }
    }

    /// Whether a usage-ledger row counts toward this scope. PR 6 rows carry a
    /// bare `scope` (a workflow slug or a session key) and, for delegates,
    /// the parent's `parent_surface` (cross-m2 H5).
    pub fn matches_row(&self, row: &LedgerRow<'_>) -> bool {
        let workflow = Surface::Workflow.as_str();
        let heartbeat = Surface::Heartbeat.as_str();
        let delegate = Surface::Delegate.as_str();
        match self {
            Scope::Global => true,
            // Q8: a self-hosted model's run rides the Claude harness (agent
            // `claude`) but is neither Claude's tokens nor its spend; it
            // counts under Global only.
            Scope::Agent(kind) => row.agent == Some(kind.as_str()) && !row.gpu_time,
            Scope::Workflow(slug) => {
                row.scope == Some(slug.as_str())
                    && (row.surface == workflow
                        || (row.surface == delegate && row.parent_surface == Some(workflow)))
            }
            Scope::Heartbeat => {
                row.surface == heartbeat
                    || (row.surface == delegate && row.parent_surface == Some(heartbeat))
            }
        }
    }
}

impl TryFrom<String> for Scope {
    type Error = String;
    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Scope::parse(&raw)
    }
}

impl From<Scope> for String {
    fn from(scope: Scope) -> Self {
        scope.key()
    }
}

/// A workflow slug as it appears in `workflows/{slug}.md`: non-empty, no path
/// separators, no leading dot, no control characters.
pub fn valid_workflow_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.chars().count() <= SLUG_MAX
        && !slug.starts_with('.')
        && !slug
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Who started a delegate (team-mcp `ask_agent`), from its environment.
/// `surface: None` means unknown or missing, which is enforced (fail closed).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParentRun {
    pub surface: Option<Surface>,
    /// The parent's bare scope: a workflow slug, or a chat session key.
    pub scope: Option<String>,
}

/// The run about to start (or finishing), as budgets see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunIntent {
    pub surface: Surface,
    /// The resolved agent, when known.
    pub agent: Option<AgentKind>,
    /// The workflow slug of a workflow run.
    pub workflow: Option<String>,
    /// Only meaningful for delegates.
    pub parent: ParentRun,
}

impl RunIntent {
    fn new(surface: Surface, agent: Option<AgentKind>) -> Self {
        RunIntent {
            surface,
            agent,
            workflow: None,
            parent: ParentRun::default(),
        }
    }

    pub fn chat(agent: Option<AgentKind>) -> Self {
        Self::new(Surface::Chat, agent)
    }

    pub fn workflow(slug: &str, agent: Option<AgentKind>) -> Self {
        RunIntent {
            workflow: Some(slug.to_string()),
            ..Self::new(Surface::Workflow, agent)
        }
    }

    pub fn heartbeat(agent: Option<AgentKind>) -> Self {
        Self::new(Surface::Heartbeat, agent)
    }

    #[cfg(test)]
    pub fn oneshot(agent: Option<AgentKind>) -> Self {
        Self::new(Surface::Oneshot, agent)
    }

    #[cfg(test)]
    pub fn ingest(agent: Option<AgentKind>) -> Self {
        Self::new(Surface::Ingest, agent)
    }

    pub fn delegate(agent: Option<AgentKind>, parent: ParentRun) -> Self {
        RunIntent {
            parent,
            ..Self::new(Surface::Delegate, agent)
        }
    }

    /// Hard stops apply: every workflow run, the heartbeat, and a delegate
    /// whose parent is an automation or unknown.
    pub fn enforced(&self) -> bool {
        match self.surface {
            Surface::Workflow | Surface::Heartbeat => true,
            Surface::Delegate => matches!(
                self.parent.surface,
                None | Some(Surface::Workflow) | Some(Surface::Heartbeat)
            ),
            Surface::Chat | Surface::Oneshot | Surface::Ingest => false,
        }
    }

    /// The workflow whose budgets this run counts toward.
    fn workflow_scope(&self) -> Option<&str> {
        match self.surface {
            Surface::Workflow => self.workflow.as_deref(),
            Surface::Delegate if self.parent.surface == Some(Surface::Workflow) => {
                self.parent.scope.as_deref()
            }
            _ => None,
        }
    }

    /// Whether a budget with this scope covers this run.
    pub fn applies(&self, scope: &Scope) -> bool {
        match scope {
            Scope::Global => true,
            Scope::Agent(kind) => self.agent == Some(*kind),
            Scope::Workflow(slug) => self.workflow_scope() == Some(slug.as_str()),
            Scope::Heartbeat => {
                self.surface == Surface::Heartbeat
                    || (self.surface == Surface::Delegate
                        && self.parent.surface == Some(Surface::Heartbeat))
            }
        }
    }
}

/// Whether `agent`'s runs report `metric` at all: Claude and Grok report
/// both, Codex tokens only (no price of its own; A13's estimate at the
/// person's price is too soft to hard-stop on), Antigravity the same as Codex
/// since 2026-11-20 (per-step tokens; its price is Hollow's estimate at
/// Google's list rates, never the CLI's own).
pub fn can_report(agent: AgentKind, metric: Metric) -> bool {
    match (agent, metric) {
        (AgentKind::Claude, _) => true,
        (AgentKind::Codex | AgentKind::Antigravity, Metric::Tokens) => true,
        (AgentKind::Codex | AgentKind::Antigravity, Metric::CostUsd) => false,
        // Grok's result carries Claude-shaped `modelUsage` with `costUSD`.
        (AgentKind::Grok, _) => true,
    }
}

fn unreportable_reason(agent: AgentKind, metric: Metric) -> String {
    match (agent, metric) {
        (AgentKind::Codex, Metric::CostUsd) => {
            "Codex reports no price of its own (any cost is an estimate at a price you entered), so a spending budget on Codex alone isn't offered. Use a token budget instead.".into()
        }
        (AgentKind::Antigravity, Metric::CostUsd) => {
            "Antigravity reports no price of its own (any cost is Hollow's estimate at Google's list rates), so a spending budget on Antigravity alone isn't offered. Use a token budget instead.".into()
        }
        (kind, metric) => format!("{} runs report no {}.", kind.label(), metric.noun()),
    }
}

// ---------------------------------------------------------------------------
// Ledger rows and totals
// ---------------------------------------------------------------------------

/// The columns of a PR 6 `usage_ledger` row that budgets read (TEXT columns
/// borrowed as they come from SQLite).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LedgerRow<'a> {
    pub surface: &'a str,
    pub scope: Option<&'a str>,
    pub parent_surface: Option<&'a str>,
    pub agent: Option<&'a str>,
    /// Includes cache creation (PR 6 normalization).
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    /// `reported` | `estimated` | `unpriced` | `partial` | `unknown`.
    pub cost_status: &'a str,
    /// A self-hosted model's run (Q8, `billing` `self-hosted`): billed by
    /// GPU time, so no Anthropic spend and never missing from a spending
    /// figure. Its tokens count like any run's.
    pub gpu_time: bool,
}

/// Observed usage for one budget's scope and window.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    /// Input (including cache creation) + output.
    pub tokens: u64,
    /// Shown, never counted.
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub runs: u32,
    /// Runs with no usable figures.
    pub unknown_runs: u32,
    /// Runs killed or interrupted mid-way (tokens are a lower bound).
    pub partial_runs: u32,
    /// Runs with tokens but no price (Codex).
    pub unpriced_runs: u32,
    /// Partial runs that carry tokens but no cost (a killed, interrupted or
    /// unverified-resume Claude run): missing, never $0, for spending budgets.
    pub uncosted_partial_runs: u32,
    /// Runs on a self-hosted model (Q8): billed by GPU time, never priced.
    /// They add nothing to `cost_usd` and are never missing from it, so they
    /// never make a spending budget a lower bound.
    pub gpu_time_runs: u32,
    /// Of `unknown_runs`, those on a self-hosted model: missing for tokens,
    /// never for spending.
    #[serde(skip)]
    pub gpu_unknown_runs: u32,
    /// True when any counted cost is a list-price estimate.
    pub estimated: bool,
}

fn non_negative(v: Option<i64>) -> u64 {
    v.map(|n| n.max(0) as u64).unwrap_or(0)
}

impl UsageTotals {
    /// Add one row (no scope check).
    pub fn add(&mut self, row: &LedgerRow<'_>) {
        self.runs += 1;
        self.tokens = self
            .tokens
            .saturating_add(non_negative(row.input_tokens))
            .saturating_add(non_negative(row.output_tokens));
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(non_negative(row.cached_input_tokens));
        let cost = row.cost_usd.filter(|c| c.is_finite() && *c > 0.0);
        if let Some(cost) = cost {
            self.cost_usd += cost;
        }
        // Q8: a self-hosted model's missing price is GPU time, not a gap.
        if row.gpu_time {
            self.gpu_time_runs += 1;
        }
        match row.cost_status {
            "unknown" => {
                self.unknown_runs += 1;
                if row.gpu_time {
                    self.gpu_unknown_runs += 1;
                }
            }
            "partial" => {
                self.partial_runs += 1;
                if cost.is_none() && !row.gpu_time {
                    self.uncosted_partial_runs += 1;
                }
            }
            "unpriced" if row.gpu_time => {}
            "unpriced" => self.unpriced_runs += 1,
            "estimated" => self.estimated = true,
            _ => {}
        }
    }

    /// Totals of the rows `scope` covers.
    pub fn from_rows(rows: &[LedgerRow<'_>], scope: &Scope) -> Self {
        let mut totals = UsageTotals::default();
        for row in rows.iter().filter(|row| scope.matches_row(row)) {
            totals.add(row);
        }
        totals
    }

    pub fn value(&self, metric: Metric) -> f64 {
        match metric {
            Metric::Tokens => self.tokens as f64,
            Metric::CostUsd => self.cost_usd,
        }
    }

    /// Runs that reported no figure for `metric`. For spending that includes
    /// Codex's unpriced runs and partial Claude rows without a cost (D2: a
    /// missing figure is never counted as zero), but never a self-hosted
    /// model's run (Q8): billed by GPU time, it spends no Anthropic money.
    pub fn missing(&self, metric: Metric) -> u32 {
        match metric {
            Metric::Tokens => self.unknown_runs,
            Metric::CostUsd => {
                self.unknown_runs.saturating_sub(self.gpu_unknown_runs)
                    + self.unpriced_runs
                    + self.uncosted_partial_runs
            }
        }
    }

    /// Runs a budget card lists as having no usage figures: for spending,
    /// never a self-hosted model's run (Q8, listed as GPU time instead).
    pub fn shown_unknown(&self, metric: Metric) -> u32 {
        match metric {
            Metric::Tokens => self.unknown_runs,
            Metric::CostUsd => self.unknown_runs.saturating_sub(self.gpu_unknown_runs),
        }
    }

    /// The observed figure, or None when every counted run reported nothing
    /// for `metric` (shown as a dash, never as 0).
    pub fn observed(&self, metric: Metric) -> Option<f64> {
        if self.runs > 0 && self.missing(metric) >= self.runs {
            None
        } else {
            Some(self.value(metric))
        }
    }
}

// ---------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------

fn default_warn_percent() -> u8 {
    DEFAULT_WARN_PERCENT
}

fn default_hard_stop() -> bool {
    true
}

/// One budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetDef {
    pub id: String,
    pub scope: Scope,
    pub metric: Metric,
    #[serde(with = "window_serde")]
    pub window: UsageWindow,
    pub limit: f64,
    #[serde(default = "default_warn_percent")]
    pub warn_percent: u8,
    /// A missing value is a hard stop: failing safe is the D2 default for
    /// automations.
    #[serde(default = "default_hard_stop")]
    pub hard_stop: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

impl BudgetDef {
    pub fn warn_at(&self) -> f64 {
        self.limit * f64::from(self.warn_percent) / 100.0
    }
}

/// A hard-stop budget's pause, keyed by budget id in [`BudgetsFile::pauses`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetPause {
    /// The window the pause began in.
    pub window_key: String,
    /// When that window ends (an unheld pause lapses then).
    pub window_end: String,
    /// `pausedAt` in RFC 3339 milliseconds; names the review item's
    /// fingerprint ([`pause_fingerprint`]).
    pub episode: String,
    pub paused_at: String,
    pub observed: f64,
    pub limit: f64,
    /// "Keep paused": survives the window reset until Resume.
    #[serde(default)]
    pub held: bool,
    #[serde(default)]
    pub review_item_id: Option<String>,
}

/// `budgets.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub budgets: Vec<BudgetDef>,
    #[serde(default)]
    pub pauses: BTreeMap<String, BudgetPause>,
    /// Notices already delivered: [`notice_key`] → when.
    #[serde(default)]
    pub notices: BTreeMap<String, String>,
}

impl Default for BudgetsFile {
    fn default() -> Self {
        BudgetsFile {
            schema_version: SCHEMA_VERSION,
            budgets: Vec::new(),
            pauses: BTreeMap::new(),
            notices: BTreeMap::new(),
        }
    }
}

/// Structural rules every stored file must meet. A file breaking them is
/// treated like a corrupt one (fail closed, never overwritten), so a
/// hand-edited `limit: -1` or duplicate id cannot silently change what trips.
pub fn validate_file(file: &BudgetsFile) -> Result<(), String> {
    let mut ids = HashSet::new();
    let mut shapes = HashSet::new();
    for def in &file.budgets {
        validate_shape(def)?;
        if !ids.insert(def.id.as_str()) {
            return Err(format!("duplicate budget id '{}'", def.id));
        }
        if !shapes.insert((def.scope.key(), def.metric, def.window.as_str())) {
            return Err(format!(
                "two {} {} budgets for {}",
                def.window.as_str(),
                def.metric.as_str(),
                def.scope.key()
            ));
        }
    }
    if let Some(bad) = file.pauses.keys().find(|id| !valid_id(id)) {
        return Err(format!("invalid pause key '{bad}'"));
    }
    Ok(())
}

fn validate_shape(def: &BudgetDef) -> Result<(), String> {
    if !valid_id(&def.id) {
        return Err(format!("invalid budget id '{}'", def.id));
    }
    if !def.limit.is_finite() || def.limit <= 0.0 {
        return Err("a budget limit must be a number above zero".into());
    }
    if !(1..=100).contains(&def.warn_percent) {
        return Err("a warning threshold must be between 1 and 100 percent".into());
    }
    Ok(())
}

/// The rules a budget must meet before Settings saves it. `scope_agent`
/// resolves the agent a `workflow:` or `heartbeat` scope runs on now (None
/// when it cannot be resolved, which is allowed); `others` are the stored
/// budgets (the one being edited, if any, shares `def.id`).
pub fn validate_def(
    def: &BudgetDef,
    others: &[BudgetDef],
    scope_agent: &dyn Fn(&Scope) -> Option<AgentKind>,
) -> Result<(), String> {
    validate_shape(def).map_err(|e| format!("{}.", sentence(&e)))?;
    let agent = match &def.scope {
        Scope::Global => None,
        Scope::Agent(kind) => Some(*kind),
        Scope::Workflow(_) | Scope::Heartbeat => scope_agent(&def.scope),
    };
    if let Some(kind) = agent {
        if !can_report(kind, def.metric) {
            return Err(match &def.scope {
                Scope::Agent(_) => unreportable_reason(kind, def.metric),
                scope => format!(
                    "{} runs on {}. {}",
                    sentence(&scope.label()),
                    kind.label(),
                    unreportable_reason(kind, def.metric)
                ),
            });
        }
    }
    if others.iter().any(|o| {
        o.id != def.id && o.scope == def.scope && o.metric == def.metric && o.window == def.window
    }) {
        return Err(format!(
            "There is already a {} {} budget for {}.",
            window_noun(def.window),
            def.metric.noun(),
            def.scope.label()
        ));
    }
    Ok(())
}

/// How far a budget can actually be enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Enforceability {
    #[serde(rename = "ok")]
    Enforced,
    /// Some in-window runs in scope reported no usable figure.
    #[serde(rename = "partial")]
    Partial,
    /// The scope's agent cannot report this metric (it changed after save).
    #[serde(rename = "none")]
    Never,
}

/// Enforceability of a budget whose scope currently runs on `scope_agent`
/// (the agent itself for `agent:` scopes), given this window's `totals`.
pub fn enforceability(
    scope: &Scope,
    metric: Metric,
    scope_agent: Option<AgentKind>,
    totals: Option<&UsageTotals>,
) -> (Enforceability, Option<String>) {
    let agent = match scope {
        Scope::Agent(kind) => Some(*kind),
        _ => scope_agent,
    };
    if let Some(kind) = agent {
        if !can_report(kind, metric) {
            return (
                Enforceability::Never,
                Some(unreportable_reason(kind, metric)),
            );
        }
    }
    // Partial rows (a killed or unverified run) still carry their tokens as a
    // lower bound, so they do not make a token budget "partial". A partial
    // Claude row usually has NO cost, though, so for spending it is missing,
    // like an unpriced or unknown run: the total is then a lower bound.
    if let Some(t) = totals {
        let missing = t.missing(metric);
        if missing > 0 {
            let runs = if missing == 1 { "run" } else { "runs" };
            return (
                Enforceability::Partial,
                Some(format!(
                    "{missing} {runs} in this window reported no {} figure, so the total shown is a lower bound.",
                    metric.noun()
                )),
            );
        }
    }
    (Enforceability::Enforced, None)
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Handle on one `budgets.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// The store production code uses. Test builds key it by vault so every
    /// test's temp vault gets its own file, whichever thread or task asks.
    pub fn for_app(vault: &Path) -> Result<Store, String> {
        #[cfg(not(test))]
        {
            let _ = vault;
            Ok(Store {
                path: crate::agent_settings::app_data_dir()?.join(FILE_NAME),
            })
        }
        #[cfg(test)]
        {
            Ok(Store {
                path: crate::test_temp::process_dir("hollow-test-budgets-")
                    .join(crate::session_store::vault_file_stem(vault))
                    .join(FILE_NAME),
            })
        }
    }

    /// A store at an explicit path (unit tests).
    #[cfg(test)]
    pub fn at(path: PathBuf) -> Store {
        Store { path }
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!(
                "budget settings at {} could not be read ({e}). {REPAIR_HINT}",
                self.path.display()
            )),
        }
    }

    fn parse(&self, bytes: Option<&[u8]>) -> Result<BudgetsFile, String> {
        let Some(bytes) = bytes else {
            return Ok(BudgetsFile::default());
        };
        let shown = self.path.display();
        let value: Value = serde_json::from_slice(bytes).map_err(|e| {
            format!("budget settings at {shown} are unreadable ({e}). {REPAIR_HINT}")
        })?;
        let version = value.get("schemaVersion").and_then(Value::as_u64);
        if version != Some(u64::from(SCHEMA_VERSION)) {
            let found = version
                .map(|v| v.to_string())
                .unwrap_or_else(|| "missing".into());
            return Err(format!(
                "budget settings at {shown} have an unsupported schema version ({found}; \
                 expected {SCHEMA_VERSION}). {REPAIR_HINT}"
            ));
        }
        let file: BudgetsFile = serde_json::from_value(value).map_err(|e| {
            format!("budget settings at {shown} are unreadable ({e}). {REPAIR_HINT}")
        })?;
        validate_file(&file)
            .map_err(|e| format!("budget settings at {shown} are invalid ({e}). {REPAIR_HINT}"))?;
        Ok(file)
    }

    /// Read and parse, plus the hash of the bytes read (None = no file).
    /// The first successful read of a store in this process becomes its
    /// trusted hash.
    fn read(&self) -> Result<(BudgetsFile, Option<String>), String> {
        let bytes = self.read_bytes()?;
        let file = self.parse(bytes.as_deref())?;
        let hash = bytes.as_deref().map(sha256_hex);
        lock(&HOST_HASH)
            .entry(self.path.clone())
            .or_insert_with(|| hash.clone());
        Ok((file, hash))
    }

    /// Read-only load. A missing file is the default (no budgets); any other
    /// failure is an `Err` naming the file, and nothing is written.
    pub fn load(&self) -> Result<BudgetsFile, String> {
        self.read().map(|(file, _)| file)
    }

    /// Load plus the file's hash, for a claim-time snapshot that a run's end
    /// compares with [`Store::is_host_hash`].
    pub fn snapshot(&self) -> Result<(BudgetsFile, Option<String>), String> {
        self.read()
    }

    /// True when `hash` is what Hollow last wrote or trusted for this store.
    #[cfg(test)]
    pub fn is_host_hash(&self, hash: Option<&str>) -> bool {
        match lock(&HOST_HASH).get(&self.path) {
            Some(known) => known.as_deref() == hash,
            None => false,
        }
    }

    /// True once this process has seen the file change outside Hollow.
    pub fn foreign_edit_seen(&self) -> bool {
        lock(&FOREIGN_SEEN)
            .iter()
            .any(|(path, _)| *path == self.path)
    }

    /// [`Store::load`] for a host write: a file that differs from the one
    /// Hollow last wrote or trusted is flagged (one warning notification per
    /// distinct foreign file) and then used as-is, so the caller's save
    /// merges on top of it rather than silently discarding a hand edit.
    pub fn load_for_write(&self, vault: &Path) -> Result<BudgetsFile, String> {
        self.load_for_write_hashed(vault).map(|(file, _)| file)
    }

    /// [`Store::load_for_write`] plus the hash of the bytes read (None = no
    /// file), for a claim-time snapshot.
    pub fn load_for_write_hashed(
        &self,
        vault: &Path,
    ) -> Result<(BudgetsFile, Option<String>), String> {
        let known = lock(&HOST_HASH).get(&self.path).cloned();
        let (file, hash) = self.read()?;
        if let Some(known) = known {
            if known != hash {
                self.flag_foreign(vault, hash.clone());
            }
        }
        Ok((file, hash))
    }

    fn flag_foreign(&self, vault: &Path, hash: Option<String>) {
        if !lock(&FOREIGN_SEEN).insert((self.path.clone(), hash)) {
            return;
        }
        eprintln!(
            "[budgets] {} changed outside Hollow; using the current file",
            self.path.display()
        );
        send(
            vault,
            NotificationInput {
                title: "Budgets changed outside Hollow".into(),
                body: format!(
                    "{} was edited by something other than Hollow. Review your budgets in \
                     Settings → Usage.",
                    self.path.display()
                ),
                kind: "warning".into(),
                source: Some("budgets".into()),
                action_label: Some("Review".into()),
                action_url: Some("settings:usage".into()),
                notify_native: true,
            },
        );
    }

    /// Owner-only atomic write ([`crate::vault::atomic_write_private`]: synced
    /// 0600 temp file → rename → directory sync). Refuses to replace a file
    /// it cannot read, and to write an invalid file.
    pub fn save(&self, file: &BudgetsFile) -> Result<(), String> {
        if let Some(bytes) = self.read_bytes()? {
            self.parse(Some(bytes.as_slice()))?;
        }
        validate_file(file).map_err(|e| format!("Budget settings not saved: {e}."))?;
        let body =
            serde_json::to_vec_pretty(file).map_err(|e| format!("encode budget settings: {e}"))?;
        crate::vault::atomic_write_private(&self.path, &body)
            .map_err(|e| format!("replace {}: {e}", self.path.display()))?;
        lock(&HOST_HASH).insert(self.path.clone(), Some(sha256_hex(&body)));
        Ok(())
    }

    /// Serialized load-for-write → modify → save (only when `f` reports a
    /// change). A load error aborts before `f` runs, so an unreadable file is
    /// never overwritten.
    pub fn with_file<T>(
        &self,
        vault: &Path,
        f: impl FnOnce(&mut BudgetsFile) -> Result<(T, bool), String>,
    ) -> Result<T, String> {
        let _guard = lock(&BUDGETS_LOCK);
        let mut file = self.load_for_write(vault)?;
        let (value, changed) = f(&mut file)?;
        if changed {
            self.save(&file)?;
        }
        Ok(value)
    }
}

/// Send through the registered app handle (none under test, where the title
/// is recorded for assertions instead).
fn send(vault: &Path, input: NotificationInput) {
    #[cfg(test)]
    lock(&SENT_FOR_TEST).push((canonical(vault), input.title.clone()));
    let _ = notifications::create_registered(vault, input);
}

fn canonical(vault: &Path) -> PathBuf {
    fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf())
}

#[cfg(test)]
static SENT_FOR_TEST: Lazy<Mutex<Vec<(PathBuf, String)>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Titles of the notifications this module sent for `vault` (tests only).
#[cfg(test)]
pub(crate) fn sent_for_test(vault: &Path) -> Vec<String> {
    let root = canonical(vault);
    lock(&SENT_FOR_TEST)
        .iter()
        .filter(|(v, _)| *v == root)
        .map(|(_, title)| title.clone())
        .collect()
}

fn load_error_due(error: &str) -> bool {
    let mut seen = lock(&LOAD_ERRORS);
    let now = Instant::now();
    match seen.get(error) {
        Some(at) if now.duration_since(*at) < LOAD_ERROR_INTERVAL => false,
        _ => {
            seen.insert(error.to_string(), now);
            true
        }
    }
}

/// Tell the user budgets are holding automations because the store (or its
/// location) is unusable: a `budgets` health failure and one error
/// notification, at most once per distinct error per hour (in process
/// memory). Callers are the notify-capable paths: preflight, `after_run`
/// and the Settings view.
pub(crate) fn report_load_error(vault: &Path, error: &str) {
    lock(&FAILURE_REPORTED).insert(canonical(vault));
    if !load_error_due(error) {
        return;
    }
    let detail: String = error.chars().take(HEALTH_DETAIL_CAP).collect();
    crate::health::record(
        vault,
        HEALTH_SUBSYSTEM,
        crate::health::Outcome::Failed,
        detail,
    );
    send(
        vault,
        NotificationInput {
            title: "Automations are paused: budget settings unreadable".into(),
            body: error.to_string(),
            kind: "error".into(),
            source: Some("budgets".into()),
            action_label: None,
            action_url: Some("settings:usage".into()),
            notify_native: true,
        },
    );
}

/// The store read cleanly after [`report_load_error`] reported a failure for
/// `vault` in this process: record `budgets` as healthy again (otherwise the
/// failure would stay the subsystem's last word after the user repaired the
/// file, since Hollow never needs a restart to pick it up), and re-arm the
/// hourly notice for this store's errors so a later breakage is announced
/// at once. A no-op when nothing was reported. Callers are the same paths
/// as [`report_load_error`], after a successful [`Store::load`].
pub(crate) fn report_load_ok(vault: &Path, store: &Store) {
    if !lock(&FAILURE_REPORTED).remove(&canonical(vault)) {
        return;
    }
    let shown = store.path.display().to_string();
    lock(&LOAD_ERRORS).retain(|error, _| !error.contains(&shown));
    crate::health::record(
        vault,
        HEALTH_SUBSYSTEM,
        crate::health::Outcome::Ok,
        "budget settings are readable again",
    );
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Why a notice exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeLevel {
    /// Past `warn_percent`.
    Warn,
    /// At or over the limit (not blocking this run).
    Limit,
    /// A hard stop holds this scope.
    Paused,
    /// Budgets could not be read or observed; fail-closed.
    Unavailable,
}

impl NoticeLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            NoticeLevel::Warn => "warn",
            NoticeLevel::Limit => "limit",
            NoticeLevel::Paused => "paused",
            NoticeLevel::Unavailable => "unavailable",
        }
    }
}

/// A user-facing budget message. `blocking` notices stop the run; every
/// blocking message starts with [`BUDGET_PAUSED_PREFIX`].
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetNotice {
    pub budget_id: Option<String>,
    pub scope: Option<String>,
    pub scope_label: Option<String>,
    pub level: NoticeLevel,
    pub metric: Option<Metric>,
    pub window: Option<String>,
    pub window_key: Option<String>,
    pub observed: Option<f64>,
    pub limit: Option<f64>,
    pub blocking: bool,
    pub estimated: bool,
    pub message: String,
    /// When the pause episode behind a blocking notice began (`pausedAt`),
    /// set by [`preflight`]; keys the run-log skip row so each episode gets
    /// its own row. Never sent to the webview.
    #[serde(skip)]
    pub episode: Option<String>,
}

/// Where one applicable budget stands for this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BudgetState {
    Ok,
    Warning,
    Over,
    Paused,
    /// The ledger could not be read.
    Unknown,
}

/// One applicable budget, evaluated.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetCheck {
    pub budget_id: String,
    pub scope: Scope,
    pub metric: Metric,
    pub window: UsageWindow,
    pub bounds: WindowBounds,
    pub limit: f64,
    pub warn_at: f64,
    pub hard_stop: bool,
    pub totals: Option<UsageTotals>,
    pub state: BudgetState,
    /// This check would stop this run (enforced intent and a hard stop).
    pub enforced: bool,
}

impl BudgetCheck {
    pub fn observed(&self) -> Option<f64> {
        self.totals.as_ref().map(|t| t.value(self.metric))
    }
}

/// A hard-stop budget an enforced run found at or over its limit with no
/// current pause: the caller records a new pause episode ([`new_pause`]).
#[derive(Debug, Clone, PartialEq)]
pub struct PauseRequest {
    pub budget_id: String,
    pub bounds: WindowBounds,
    pub observed: f64,
    pub limit: f64,
}

/// The result of [`evaluate`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Evaluation {
    /// The first blocking notice, in budget order.
    pub blocked: Option<BudgetNotice>,
    /// Non-blocking notices (not yet de-duplicated against `notices`).
    pub warnings: Vec<BudgetNotice>,
    pub to_pause: Vec<PauseRequest>,
    /// Budget ids whose pause has ended: an unheld pause from an earlier
    /// window, a pause whose budget is gone, or one whose budget no longer
    /// hard-stops.
    pub lapsed: Vec<String>,
    pub checks: Vec<BudgetCheck>,
    /// Levels the observed baseline already meets (`{budgetId}:{level}`), so
    /// a mid-run guard (PR 8b) fires only on a crossing inside the run.
    pub seed_levels: BTreeSet<String>,
    /// For an enforced run: the smallest dollar headroom left under an
    /// applicable hard-stop spending budget, floored at $0.01 (PR 8b's
    /// `--max-budget-usd` backstop).
    pub max_budget_usd: Option<f64>,
}

/// `{budgetId}:{level}` for [`Evaluation::seed_levels`].
pub fn level_key(budget_id: &str, level: NoticeLevel) -> String {
    format!("{budget_id}:{}", level.as_str())
}

/// Observed usage for one budget in one window.
pub type ObservedFn<'a> = dyn Fn(&BudgetDef, &WindowBounds) -> Result<UsageTotals, String> + 'a;

/// Evaluate every budget that applies to `intent` at `now` (pure: the ledger
/// arrives through `observed`, which is never called when no budget
/// applies). A current pause, or a hard-stop budget at or over its limit,
/// blocks only an enforced intent; an unreadable ledger blocks only an
/// enforced intent under a hard-stop budget (fail closed).
pub fn evaluate<Tz: TimeZone>(
    file: &BudgetsFile,
    intent: &RunIntent,
    observed: &ObservedFn<'_>,
    now: &DateTime<Tz>,
) -> Evaluation {
    let mut out = Evaluation::default();
    for (id, pause) in &file.pauses {
        let ended = match file.budgets.iter().find(|b| &b.id == id) {
            None => true,
            Some(def) if !def.hard_stop => true,
            Some(def) => !pause.held && pause.window_key != window_bounds_in(def.window, now).key,
        };
        if ended {
            out.lapsed.push(id.clone());
        }
    }
    let enforced = intent.enforced();
    for def in &file.budgets {
        if !intent.applies(&def.scope) {
            continue;
        }
        let bounds = window_bounds_in(def.window, now);
        let pause = file
            .pauses
            .get(&def.id)
            .filter(|_| !out.lapsed.contains(&def.id));
        let blocks = enforced && def.hard_stop;
        let (totals, ledger_error) = match observed(def, &bounds) {
            Ok(t) => (Some(t), None),
            Err(e) => (None, Some(e)),
        };
        let value = totals.as_ref().map(|t| t.value(def.metric));
        let estimated =
            def.metric == Metric::CostUsd && totals.as_ref().map(|t| t.estimated).unwrap_or(false);
        let warn_at = def.warn_at();
        if let Some(v) = value {
            if v >= warn_at {
                out.seed_levels
                    .insert(level_key(&def.id, NoticeLevel::Warn));
            }
            if v >= def.limit {
                out.seed_levels
                    .insert(level_key(&def.id, NoticeLevel::Limit));
            }
        }
        let state = if let Some(pause) = pause {
            let notice = paused_notice(def, pause, &bounds, value, estimated, blocks);
            push_notice(&mut out, notice);
            BudgetState::Paused
        } else if let Some(error) = &ledger_error {
            if blocks {
                push_notice(&mut out, unavailable_notice(def, &bounds, error));
            }
            BudgetState::Unknown
        } else if let Some(v) = value.filter(|v| *v >= def.limit) {
            if blocks {
                out.to_pause.push(PauseRequest {
                    budget_id: def.id.clone(),
                    bounds: bounds.clone(),
                    observed: v,
                    limit: def.limit,
                });
            }
            push_notice(&mut out, limit_notice(def, &bounds, v, estimated, blocks));
            BudgetState::Over
        } else if let Some(v) = value.filter(|v| *v >= warn_at) {
            push_notice(&mut out, warn_notice(def, &bounds, v, estimated));
            BudgetState::Warning
        } else {
            BudgetState::Ok
        };
        out.checks.push(BudgetCheck {
            budget_id: def.id.clone(),
            scope: def.scope.clone(),
            metric: def.metric,
            window: def.window,
            bounds,
            limit: def.limit,
            warn_at,
            hard_stop: def.hard_stop,
            totals,
            state,
            enforced: blocks,
        });
    }
    out.max_budget_usd = spend_headroom(&out.checks, &|_| 0.0, 0.01);
    out
}

/// The smallest dollar headroom left under an enforced hard-stop spending
/// budget that is not paused and not yet at its limit, less what `granted`
/// says other live claims already hold under that budget's scope (merged
/// plan slice 3), bounded below by the caller's floor ($0.01). None when no
/// such budget applies.
fn spend_headroom(
    checks: &[BudgetCheck],
    granted: &dyn Fn(&Scope) -> f64,
    floor: f64,
) -> Option<f64> {
    checks
        .iter()
        .filter(|check| {
            check.enforced && check.metric == Metric::CostUsd && check.state != BudgetState::Paused
        })
        .filter_map(|check| {
            let v = check.observed().filter(|v| *v < check.limit)?;
            Some((check.limit - v - granted(&check.scope)).max(floor))
        })
        .reduce(f64::min)
}

fn push_notice(out: &mut Evaluation, notice: BudgetNotice) {
    if notice.blocking {
        if out.blocked.is_none() {
            out.blocked = Some(notice);
        }
    } else {
        out.warnings.push(notice);
    }
}

/// The blocking notice for an enforced intent when the store itself failed
/// ([`Store::for_app`] or [`Store::load`]); None for warn-only intents, which
/// run on as if there were no budgets.
pub fn load_failure_notice(intent: &RunIntent, error: &str) -> Option<BudgetNotice> {
    intent.enforced().then(|| BudgetNotice {
        budget_id: None,
        scope: None,
        scope_label: None,
        level: NoticeLevel::Unavailable,
        metric: None,
        window: None,
        window_key: None,
        observed: None,
        limit: None,
        blocking: true,
        estimated: false,
        message: format!("{BUDGET_PAUSED_PREFIX} automations are held because {error}"),
        episode: None,
    })
}

fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `12,345 tokens` / `$1.23` (`$1.23 estimated`).
pub fn format_amount(metric: Metric, value: f64, estimated: bool) -> String {
    match metric {
        Metric::Tokens => format!("{} tokens", group_digits(value.max(0.0).round() as u64)),
        Metric::CostUsd if estimated => format!("${value:.2} estimated"),
        Metric::CostUsd => format!("${value:.2}"),
    }
}

fn notice(
    def: &BudgetDef,
    bounds: &WindowBounds,
    level: NoticeLevel,
    observed: Option<f64>,
    estimated: bool,
    blocking: bool,
    message: String,
) -> BudgetNotice {
    BudgetNotice {
        budget_id: Some(def.id.clone()),
        scope: Some(def.scope.key()),
        scope_label: Some(def.scope.label()),
        level,
        metric: Some(def.metric),
        window: Some(def.window.as_str().to_string()),
        window_key: Some(bounds.key.clone()),
        observed,
        limit: Some(def.limit),
        blocking,
        estimated,
        message,
        episode: None,
    }
}

fn of_limit(def: &BudgetDef, value: f64, estimated: bool) -> String {
    format!(
        "{} of {}",
        format_amount(def.metric, value, estimated),
        format_amount(def.metric, def.limit, false)
    )
}

fn paused_notice(
    def: &BudgetDef,
    pause: &BudgetPause,
    bounds: &WindowBounds,
    value: Option<f64>,
    estimated: bool,
    blocking: bool,
) -> BudgetNotice {
    let label = def.scope.label();
    let limit_text = format!(
        "{} {} limit",
        window_adjective(def.window),
        def.metric.noun()
    );
    let message = match (blocking, pause.held) {
        (true, true) => format!(
            "{BUDGET_PAUSED_PREFIX} {label} reached {limit_text} and is kept paused until you \
             resume it in Automations."
        ),
        (true, false) => format!(
            "{BUDGET_PAUSED_PREFIX} {label} reached {limit_text} ({}). Raise and resume it in \
             Automations, or wait for the {} to reset.",
            of_limit(def, value.unwrap_or(pause.observed), estimated),
            window_noun(def.window)
        ),
        (false, _) => format!(
            "{} is paused at {limit_text}. Chat keeps working; automations under this budget \
             wait until it is resumed.",
            sentence(&label)
        ),
    };
    notice(
        def,
        bounds,
        NoticeLevel::Paused,
        value,
        estimated,
        blocking,
        message,
    )
}

fn limit_notice(
    def: &BudgetDef,
    bounds: &WindowBounds,
    value: f64,
    estimated: bool,
    blocking: bool,
) -> BudgetNotice {
    let label = def.scope.label();
    let limit_text = format!(
        "{} {} limit",
        window_adjective(def.window),
        def.metric.noun()
    );
    let amounts = of_limit(def, value, estimated);
    let (level, message) = if blocking {
        (
            NoticeLevel::Paused,
            format!(
                "{BUDGET_PAUSED_PREFIX} {label} reached {limit_text} ({amounts}). Raise and \
                 resume it in Automations or Settings → Usage, or wait for the {} to reset.",
                window_noun(def.window)
            ),
        )
    } else if def.hard_stop {
        (
            NoticeLevel::Limit,
            format!(
                "{} is over {limit_text} ({amounts}). Chat keeps working; automations under \
                 this budget are paused.",
                sentence(&label)
            ),
        )
    } else {
        (
            NoticeLevel::Limit,
            format!("{} is over {limit_text} ({amounts}).", sentence(&label)),
        )
    };
    notice(
        def,
        bounds,
        level,
        Some(value),
        estimated,
        blocking,
        message,
    )
}

fn warn_notice(
    def: &BudgetDef,
    bounds: &WindowBounds,
    value: f64,
    estimated: bool,
) -> BudgetNotice {
    let percent = (value / def.limit * 100.0).floor().max(0.0) as u64;
    let message = format!(
        "{} has used {percent}% of {} {} limit ({}).",
        sentence(&def.scope.label()),
        window_adjective(def.window),
        def.metric.noun(),
        of_limit(def, value, estimated)
    );
    notice(
        def,
        bounds,
        NoticeLevel::Warn,
        Some(value),
        estimated,
        false,
        message,
    )
}

fn unavailable_notice(def: &BudgetDef, bounds: &WindowBounds, error: &str) -> BudgetNotice {
    let message = format!(
        "{BUDGET_PAUSED_PREFIX} {} has a hard-stop budget and Hollow could not read the usage \
         ledger ({error}), so automations under it are held.",
        def.scope.label()
    );
    notice(
        def,
        bounds,
        NoticeLevel::Unavailable,
        None,
        false,
        true,
        message,
    )
}

// ---------------------------------------------------------------------------
// Pause episodes and notice bookkeeping (pure file edits)
// ---------------------------------------------------------------------------

/// The review item fingerprint of one pause episode, lower-cased as the
/// review queue stores every fingerprint.
pub fn pause_fingerprint(budget_id: &str, episode: &str) -> String {
    format!("budget-pause:{budget_id}:{episode}").to_lowercase()
}

/// A new pause episode for `request`, starting `now`.
pub fn new_pause<Tz: TimeZone>(request: &PauseRequest, now: &DateTime<Tz>) -> BudgetPause {
    let at = rfc3339_millis(now);
    BudgetPause {
        window_key: request.bounds.key.clone(),
        window_end: rfc3339_millis(&request.bounds.end),
        episode: at.clone(),
        paused_at: at,
        observed: request.observed,
        limit: request.limit,
        held: false,
        review_item_id: None,
    }
}

/// Remove the lapsed pauses; returns `(budget id, linked review item id)` for
/// each removed pause so the caller can supersede its item.
pub fn remove_lapsed(file: &mut BudgetsFile, lapsed: &[String]) -> Vec<(String, Option<String>)> {
    lapsed
        .iter()
        .filter_map(|id| {
            file.pauses
                .remove(id)
                .map(|pause| (id.clone(), pause.review_item_id))
        })
        .collect()
}

/// `{budgetId}:{windowKey}:{level}`, prefixed `chat:` for notices delivered
/// into a chat conversation (a separate channel from notifications).
pub fn notice_key(chat: bool, budget_id: &str, window_key: &str, level: NoticeLevel) -> String {
    let key = format!("{budget_id}:{window_key}:{}", level.as_str());
    if chat {
        format!("chat:{key}")
    } else {
        key
    }
}

/// Drop recorded notices whose budget is gone or whose window has passed.
/// Returns whether anything was removed.
pub fn prune_notices<Tz: TimeZone>(file: &mut BudgetsFile, now: &DateTime<Tz>) -> bool {
    let current: HashMap<&str, String> = file
        .budgets
        .iter()
        .map(|def| (def.id.as_str(), window_bounds_in(def.window, now).key))
        .collect();
    let before = file.notices.len();
    file.notices.retain(|key, _| {
        let rest = key.strip_prefix("chat:").unwrap_or(key);
        let mut parts = rest.splitn(3, ':');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(id), Some(window_key), Some(_level)) => {
                current.get(id).map(String::as_str) == Some(window_key)
            }
            _ => false,
        }
    });
    file.notices.len() != before
}

// ---------------------------------------------------------------------------
// Suggestions (D2: never a default cap)
// ---------------------------------------------------------------------------

/// A starting limit for the Settings form: the largest per-window total ×
/// 1.5, rounded up to two significant figures. None without data. Only ever
/// prefills the form.
pub fn suggest_limit(series: &[f64]) -> Option<f64> {
    let max = series
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(None, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |a| a.max(v)))
        })?;
    let target = max * 1.5;
    if !target.is_finite() || target <= 0.0 {
        return None;
    }
    let exponent = target.log10().floor() as i32 - 1;
    let scale = |n: f64| {
        if exponent >= 0 {
            n * 10f64.powi(exponent)
        } else {
            n / 10f64.powi(-exponent)
        }
    };
    let unit = scale(1.0);
    // The tiny shave keeps an exact product (600 = 60 × 10) from rounding up
    // on floating-point noise.
    let steps = (target / unit * (1.0 - 1e-12)).ceil();
    Some(scale(steps))
}

/// Whole days of ledger history before `now`.
pub fn ledger_days(first_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> i64 {
    first_at
        .map(|first| (now - first).num_days().max(0))
        .unwrap_or(0)
}

/// "Suggest a limit" is offered only after [`SUGGESTION_MIN_DAYS`] of data.
pub fn suggestions_available(first_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    ledger_days(first_at, now) >= SUGGESTION_MIN_DAYS
}

// ---------------------------------------------------------------------------
// Reading the usage ledger
// ---------------------------------------------------------------------------

/// Where the Automations pause panel opens (notifications, Since last visit).
pub const PAUSES_ACTION: &str = "budget-pauses:open";
/// Where Settings, Usage opens.
pub const USAGE_ACTION: &str = "settings:usage";
const ENDED_MESSAGE: &str = "This pause has ended; the budget is active again.";
const DUPLICATE_ITEM_MESSAGE: &str = "This pause can't be resolved from its card: more than one \
     review item has its id. Resume the budget from Settings → Usage instead.";

impl<'a> LedgerRow<'a> {
    /// Borrow the columns budgets read from one PR 6 ledger row.
    pub fn from_usage(row: &'a crate::session_store::UsageRow) -> Self {
        LedgerRow {
            surface: row.surface.as_str(),
            scope: row.scope.as_deref(),
            parent_surface: row.parent_surface.as_deref(),
            agent: row.agent.as_deref(),
            input_tokens: row.input_tokens,
            cached_input_tokens: row.cached_input_tokens,
            output_tokens: row.output_tokens,
            cost_usd: row.cost_usd,
            cost_status: row.cost_status.as_str(),
            gpu_time: row.billing == crate::usage::Billing::SelfHosted.as_str(),
        }
    }
}

/// Observed usage per budget, read from the vault's usage ledger with each
/// distinct window queried once (budgets sharing a window share the rows).
/// Rows that never spawned are skipped: a validation refusal used nothing.
/// Running rows count with their last checkpoint.
pub struct LedgerReader<'v> {
    vault: &'v Path,
    cache: RefCell<
        HashMap<
            (DateTime<Utc>, DateTime<Utc>),
            Result<Vec<crate::session_store::UsageRow>, String>,
        >,
    >,
}

impl<'v> LedgerReader<'v> {
    pub fn new(vault: &'v Path) -> Self {
        LedgerReader {
            vault,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// The totals `def`'s scope covers in `bounds`.
    pub fn totals(&self, def: &BudgetDef, bounds: &WindowBounds) -> Result<UsageTotals, String> {
        let mut cache = self.cache.borrow_mut();
        let rows = cache.entry((bounds.start, bounds.end)).or_insert_with(|| {
            crate::session_store::usage_rows_between(
                self.vault,
                &crate::session_store::db_time(bounds.start),
                &crate::session_store::db_time(bounds.end),
            )
        });
        match rows {
            Ok(rows) => {
                let ledger = scope_rows(self.vault, rows, &def.scope);
                Ok(UsageTotals::from_rows(&ledger, &def.scope))
            }
            Err(error) => Err(error.clone()),
        }
    }

    /// Run key → tokens of every row behind [`Self::totals`]'s token figure
    /// for `def` in `bounds` (empty when the ledger could not be read): what
    /// a guard's claim-time baseline already counts of each run (merged plan
    /// slice 3), from the same cached rows.
    pub fn counted(&self, def: &BudgetDef, bounds: &WindowBounds) -> BTreeMap<String, u64> {
        let mut cache = self.cache.borrow_mut();
        let rows = cache.entry((bounds.start, bounds.end)).or_insert_with(|| {
            crate::session_store::usage_rows_between(
                self.vault,
                &crate::session_store::db_time(bounds.start),
                &crate::session_store::db_time(bounds.end),
            )
        });
        match rows {
            Ok(rows) => scope_runs(self.vault, rows, &def.scope),
            Err(_) => BTreeMap::new(),
        }
    }
}

/// Run key → tokens (input with cache writes, plus output) of every spawned
/// ledger row `scope` covers: the rows, and the figure, [`UsageTotals`]
/// counts for a token budget.
fn scope_runs(
    vault: &Path,
    rows: &[crate::session_store::UsageRow],
    scope: &Scope,
) -> BTreeMap<String, u64> {
    let chat_turns = automation_chat_turns(vault, scope);
    rows.iter()
        .filter(|row| row.spawned)
        .filter(|row| scope.matches_row(&scoped_row(row, scope, &chat_turns)))
        .map(|row| {
            let tokens =
                non_negative(row.input_tokens).saturating_add(non_negative(row.output_tokens));
            (row.run_key.clone(), tokens)
        })
        .collect()
}

/// Overhaul F1: the chat turns an automation that runs in a chat queued
/// (`automation_in_chat::turn_runs`), by history run id; empty for any other
/// scope.
fn automation_chat_turns(vault: &Path, scope: &Scope) -> HashSet<String> {
    match scope {
        Scope::Workflow(slug) => crate::automation_in_chat::turn_runs(vault, slug),
        _ => HashSet::new(),
    }
}

/// `row` as `scope` reads it: a chat row of one of the automation's own chat
/// turns counts as that automation's (its row stays the chat's for every
/// other scope and figure).
fn scoped_row<'a>(
    row: &'a crate::session_store::UsageRow,
    scope: &'a Scope,
    chat_turns: &HashSet<String>,
) -> LedgerRow<'a> {
    let mut ledger = LedgerRow::from_usage(row);
    if let Scope::Workflow(slug) = scope {
        let ours = row.surface == Surface::Chat.as_str()
            && row
                .run_id
                .as_deref()
                .is_some_and(|run| chat_turns.contains(run));
        if ours {
            ledger.surface = Surface::Workflow.as_str();
            ledger.scope = Some(slug.as_str());
            ledger.parent_surface = None;
        }
    }
    ledger
}

/// The spawned rows `scope` reads ([`scoped_row`]).
pub(crate) fn scope_rows<'a>(
    vault: &Path,
    rows: &'a [crate::session_store::UsageRow],
    scope: &'a Scope,
) -> Vec<LedgerRow<'a>> {
    let chat_turns = automation_chat_turns(vault, scope);
    rows.iter()
        .filter(|row| row.spawned)
        .map(|row| scoped_row(row, scope, &chat_turns))
        .collect()
}

fn local(now: DateTime<Utc>) -> DateTime<Local> {
    now.with_timezone(&Local)
}

// ---------------------------------------------------------------------------
// Agents behind an intent
// ---------------------------------------------------------------------------

/// The app default agent as a run would resolve it without an explicit
/// choice: the saved preference, else the first installed CLI. Unlike
/// `agent_settings::resolve` it never errors (a budget intent with no agent
/// simply skips `agent:` budgets).
pub fn default_agent() -> Option<AgentKind> {
    crate::agent_settings::get_settings()
        .ok()
        .and_then(|settings| settings.default_agent)
        .and_then(|name| AgentKind::from_opt(&name))
        .or_else(|| crate::agent::available().first().copied())
}

/// The agent a run with this explicit selection (a workflow's stamped
/// `agent:`) resolves to, `default` otherwise.
pub fn intent_agent_with(explicit: Option<&str>, default: Option<AgentKind>) -> Option<AgentKind> {
    explicit
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .and_then(AgentKind::from_opt)
        .or(default)
}

/// [`intent_agent_with`] against the live default.
pub fn intent_agent(explicit: Option<&str>) -> Option<AgentKind> {
    intent_agent_with(explicit, default_agent())
}

/// The agent each scope runs on now, for Settings validation and the view:
/// an `agent:` scope is its agent, a workflow its stamped (or the default)
/// agent, the heartbeat the default; `global` and unknown workflows None.
pub fn production_scope_agent(vault: &Path) -> impl Fn(&Scope) -> Option<AgentKind> {
    let default = default_agent();
    let agents: HashMap<String, Option<String>> = crate::vault_workflows::list(vault)
        .into_iter()
        .map(|workflow| (workflow.slug, workflow.agent))
        .collect();
    move |scope| match scope {
        Scope::Global => None,
        Scope::Agent(kind) => Some(*kind),
        Scope::Heartbeat => default,
        Scope::Workflow(slug) => agents
            .get(slug)
            .and_then(|agent| intent_agent_with(agent.as_deref(), default)),
    }
}

// ---------------------------------------------------------------------------
// Preflight
// ---------------------------------------------------------------------------

/// What the claim hands the run (PR 8b's run guard reads it).
#[derive(Debug, Clone, PartialEq)]
pub struct RunBudget {
    pub intent: RunIntent,
    /// Levels the claim-time baseline already met, so a mid-run guard fires
    /// only on a crossing inside the run.
    pub seed_levels: BTreeSet<String>,
    /// Smallest dollar headroom under an enforced hard-stop spending budget,
    /// less what other live claims hold of it ([`Self::spend_grant`]).
    pub max_budget_usd: Option<f64>,
    /// Merged plan slice 3: this Claude claim's hold on
    /// [`Self::max_budget_usd`] in the vault's shared reservation, so a
    /// concurrent claim's backstop gets only what is left. Released when the
    /// last clone of this budget is dropped (the run is over).
    pub spend_grant: Option<Arc<SpendGrant>>,
    /// Hash of `budgets.json` at the claim (None = no file).
    pub file_hash: Option<String>,
    /// Token budgets a mid-run [`BudgetGuard`] watches (Paperclip PR 8b),
    /// each with its claim-time baseline. Spending budgets are not here: a
    /// stream reports no cost until the run ends, so dollars are stopped only
    /// by Claude's `--max-budget-usd` backstop ([`Self::max_budget_usd`]).
    pub checks: Vec<GuardCheck>,
    /// Paperclip PR 8b: budget id → limit of every hard-stop budget that
    /// would stop this run, as the claim read them. [`after_run_claimed`]
    /// holds a run that changed `budgets.json` mid-run to these limits.
    pub claimed_limits: BTreeMap<String, f64>,
    /// The vault whose usage ledger the claim read (merged plan slice 3,
    /// decision 6): a [`BudgetGuard`] shares one in-flight reservation with
    /// every other run in it. None (no ledger, or no budget applied) means
    /// the guard sees only this run.
    pub vault: Option<PathBuf>,
    /// Decision 6: the claim found this run's own agent at an `agent:`
    /// hard-stop ceiling and moved it to another agent with room
    /// ([`preflight_with_fallback`]). The run spawns on `fallback.to`, and
    /// the switch goes on its receipt.
    pub fallback: Option<BudgetFallback>,
}

impl RunBudget {
    pub fn none(intent: RunIntent) -> Self {
        RunBudget {
            intent,
            seed_levels: BTreeSet::new(),
            max_budget_usd: None,
            spend_grant: None,
            file_hash: None,
            checks: Vec::new(),
            claimed_limits: BTreeMap::new(),
            vault: None,
            fallback: None,
        }
    }

    /// Whether a Claude run should stream partial messages for its usage
    /// (`RunSpec::usage_stream`): only when a token budget watches it. With
    /// no budgets (the D2 default) nothing about any run changes.
    pub fn usage_stream(&self) -> bool {
        !self.checks.is_empty()
    }
}

/// The claim-time verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum Preflight {
    Clear(RunBudget),
    /// Runs, with notices for the conversation it was claimed for (chat
    /// only: automations' warnings go to the notification center).
    Warn(RunBudget, Vec<BudgetNotice>),
    /// A hard-stop budget holds this enforced run.
    Blocked(BudgetNotice),
}

impl Preflight {
    #[cfg(test)]
    pub fn blocked(&self) -> Option<&BudgetNotice> {
        match self {
            Preflight::Blocked(notice) => Some(notice),
            _ => None,
        }
    }
}

/// What [`after_run`] (and the lifecycle) did, for tests and logs.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetEffect {
    Paused {
        budget_id: String,
        review_item_id: Option<String>,
    },
    Warned {
        key: String,
    },
    Lapsed {
        budget_id: String,
    },
    /// Paperclip PR 8b: `budgets.json` changed during the run and raised this
    /// budget's limit; the run is held to the limit its claim read (a pause
    /// at that limit when the usage reached it).
    ClaimLimitKept {
        budget_id: String,
    },
}

/// A notification to send once the budgets lock is released.
type Outbox = Vec<NotificationInput>;

fn stamp(now: DateTime<Utc>) -> String {
    rfc3339_millis(&now)
}

/// Supersede a pause's review item, logging (never propagating) failures.
fn supersede_item(vault: &Path, item_id: &str, reason: &str) {
    if let Err(error) = crate::review_queue::supersede(vault, item_id, reason) {
        eprintln!("[budgets] could not supersede review item {item_id}: {error}");
    }
}

fn lapse_reason(file: &BudgetsFile, budget_id: &str) -> String {
    match file.budgets.iter().find(|def| def.id == budget_id) {
        None => "Superseded: the budget was removed.".into(),
        Some(def) if !def.hard_stop => "Superseded: the budget no longer stops automations.".into(),
        Some(def) => format!("Superseded: the {} reset.", window_noun(def.window)),
    }
}

/// Remove `lapsed` pauses from `file`, superseding their items. Must run
/// before the lapsed budgets' definitions change (the reason reads them).
fn apply_lapses(
    vault: &Path,
    file: &mut BudgetsFile,
    lapsed: &[String],
    effects: &mut Vec<BudgetEffect>,
) -> bool {
    let reasons: Vec<(String, String)> = lapsed
        .iter()
        .map(|id| (id.clone(), lapse_reason(file, id)))
        .collect();
    let removed = remove_lapsed(file, lapsed);
    for (budget_id, item) in &removed {
        if let Some(item) = item {
            let reason = reasons
                .iter()
                .find(|(id, _)| id == budget_id)
                .map(|(_, reason)| reason.as_str())
                .unwrap_or("Superseded: the pause ended.");
            supersede_item(vault, item, reason);
        }
        effects.push(BudgetEffect::Lapsed {
            budget_id: budget_id.clone(),
        });
    }
    !removed.is_empty()
}

/// The review payload of a pause (display only: accept, reject and resume
/// re-derive everything from `budgets.json`).
pub fn pause_payload(def: &BudgetDef, pause: &BudgetPause, estimated: bool) -> Value {
    json!({
        "budgetId": def.id,
        "episode": pause.episode,
        "scope": def.scope.key(),
        "scopeLabel": def.scope.label(),
        "metric": def.metric.as_str(),
        "window": def.window.as_str(),
        "windowKey": pause.window_key,
        "windowEnd": pause.window_end,
        "limit": pause.limit,
        "observed": pause.observed,
        "pausedAt": pause.paused_at,
        "suggestedLimit": suggest_limit(&[pause.observed.max(pause.limit)]),
        "estimated": estimated,
        "enforcement": ENFORCEMENT_COPY,
    })
}

/// What Settings and the pause card say about enforcement (mirrored by
/// `ENFORCEMENT_COPY` in `src/budgets.ts`): 8a's claim-time check, 8b's
/// mid-run stop for Claude token limits, the spending backstop's one-message
/// overshoot, and the Codex/Antigravity distinction.
pub const ENFORCEMENT_COPY: &str = "Hard stop applies to automations: every run is checked before \
     it starts, and a Claude run under these policies that crosses a token limit is stopped mid-run. A spending \
     limit is a backstop Claude applies after each message, so a run can go over by up to one \
     message. Codex, Antigravity and Grok runs are checked only before they start. Ordinary chat only warns.";

/// The notification a new pause raises.
pub fn pause_notification(def: &BudgetDef, pause: &BudgetPause) -> NotificationInput {
    NotificationInput {
        title: format!("{BUDGET_PAUSED_PREFIX} {}", def.scope.label()),
        body: format!(
            "{} reached {} {} limit ({}). Automations under it wait until you raise and resume \
             it in Automations, or until the {} resets. Chat keeps working.",
            sentence(&def.scope.label()),
            window_adjective(def.window),
            def.metric.noun(),
            of_limit(def, pause.observed, false),
            window_noun(def.window)
        ),
        kind: "warning".into(),
        source: Some("budgets".into()),
        action_label: Some("Review".into()),
        action_url: Some(PAUSES_ACTION.into()),
        notify_native: true,
    }
}

/// The once-per-window notification of a non-blocking notice: a warn crossing
/// (info, in-app only) or a limit reached (a warning, also native).
fn warning_notification(notice: &BudgetNotice) -> NotificationInput {
    let limit = notice.level == NoticeLevel::Limit;
    NotificationInput {
        title: if limit {
            "Budget limit reached".into()
        } else {
            "Budget warning".into()
        },
        body: notice.message.clone(),
        kind: if limit { "warning" } else { "info" }.into(),
        source: Some("budgets".into()),
        action_label: Some("Open".into()),
        action_url: Some(USAGE_ACTION.into()),
        notify_native: limit,
    }
}

/// Open a pause episode for `request`: enqueue its sticky review item (a
/// non-pending result leaves the pause unlinked; Settings, Resume still
/// works), record it and queue its notification. The caller holds
/// [`BUDGETS_LOCK`] and saves the file.
fn open_pause(
    vault: &Path,
    file: &mut BudgetsFile,
    request: &PauseRequest,
    estimated: bool,
    now: DateTime<Utc>,
    outbox: &mut Outbox,
) -> Option<BudgetEffect> {
    if file.pauses.contains_key(&request.budget_id) {
        return None;
    }
    let def = file
        .budgets
        .iter()
        .find(|def| def.id == request.budget_id)?
        .clone();
    let mut pause = new_pause(request, &now);
    let fingerprint = pause_fingerprint(&def.id, &pause.episode);
    match crate::review_queue::enqueue(
        vault,
        crate::review_queue::NewReviewItem {
            kind: REVIEW_KIND.into(),
            fingerprint,
            payload: pause_payload(&def, &pause, estimated),
            risk: "local_write".into(),
            source: REVIEW_SOURCE.into(),
            source_path: None,
        },
    ) {
        Ok(item) if item.status == "pending" => pause.review_item_id = Some(item.id),
        Ok(item) => eprintln!(
            "[budgets] the pause item for {} is already {}; resume it from Settings",
            def.id, item.status
        ),
        Err(error) => eprintln!(
            "[budgets] could not queue the pause for {}: {error}",
            def.id
        ),
    }
    outbox.push(pause_notification(&def, &pause));
    let effect = BudgetEffect::Paused {
        budget_id: def.id.clone(),
        review_item_id: pause.review_item_id.clone(),
    };
    file.pauses.insert(def.id.clone(), pause);
    Some(effect)
}

/// Record a notification once per budget, window and level; returns whether
/// it is new.
fn note_once(file: &mut BudgetsFile, key: String, now: DateTime<Utc>) -> bool {
    if file.notices.contains_key(&key) {
        return false;
    }
    file.notices.insert(key, stamp(now));
    true
}

fn notice_keys(notice: &BudgetNotice, chat: bool) -> Option<String> {
    Some(notice_key(
        chat,
        notice.budget_id.as_deref()?,
        notice.window_key.as_deref()?,
        notice.level,
    ))
}

/// Tell the Settings and Automations views that budgets or pauses changed.
/// A no-op without a registered GUI (tests, MCP subprocesses).
pub fn emit_changed() {
    if let Some(app) = notifications::registered_app() {
        let _ = app.emit(EVENT, ());
    }
}

/// THE claim-time check (GUI only). Loads the store under [`BUDGETS_LOCK`]
/// (a failure blocks enforced intents only), evaluates every applicable
/// budget against the ledger (never read when no budget applies), then:
/// removes lapsed pauses (superseding their items), opens a pause episode for
/// every hard-stop budget an enforced run finds at its limit, notifies each
/// warning once per window and level, and returns the verdict. A chat intent
/// also gets its not-yet-delivered conversation notices (keyed `chat:…`).
pub fn preflight(
    store: &Result<Store, String>,
    vault: &Path,
    intent: &RunIntent,
    now: DateTime<Utc>,
) -> Preflight {
    let fail = |error: &str| {
        report_load_error(vault, error);
        match load_failure_notice(intent, error) {
            Some(notice) => Preflight::Blocked(notice),
            None => Preflight::Clear(RunBudget::none(intent.clone())),
        }
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => return fail(error),
    };
    let guard = lock(&BUDGETS_LOCK);
    let (mut file, hash) = match store.load_for_write_hashed(vault) {
        Ok(loaded) => loaded,
        Err(error) => {
            drop(guard);
            return fail(&error);
        }
    };
    report_load_ok(vault, store);
    if file.budgets.is_empty() && file.pauses.is_empty() {
        return Preflight::Clear(RunBudget {
            file_hash: hash,
            ..RunBudget::none(intent.clone())
        });
    }
    let local_now = local(now);
    let reader = LedgerReader::new(vault);
    let observed = |def: &BudgetDef, bounds: &WindowBounds| reader.totals(def, bounds);
    let eval = evaluate(&file, intent, &observed, &local_now);
    let checks = guard_checks(&file, &eval, &|def, bounds| reader.counted(def, bounds));
    let claimed = claimed_limits(&eval);
    // Under BUDGETS_LOCK, so two claims never grant the same dollars.
    let (max_budget_usd, spend_grant) = match grant_spend(vault, intent, &eval) {
        Ok(grant) => grant,
        Err(notice) => return Preflight::Blocked(notice),
    };

    let mut effects = Vec::new();
    let mut outbox: Outbox = Vec::new();
    let mut changed = apply_lapses(vault, &mut file, &eval.lapsed, &mut effects);
    for request in &eval.to_pause {
        let estimated = eval
            .checks
            .iter()
            .find(|check| check.budget_id == request.budget_id)
            .and_then(|check| check.totals.as_ref())
            .is_some_and(|totals| totals.estimated);
        if open_pause(vault, &mut file, request, estimated, now, &mut outbox).is_some() {
            changed = true;
        }
    }
    let chat = intent.surface == Surface::Chat;
    let mut conversation = Vec::new();
    for notice in &eval.warnings {
        // A pause already notified when it opened; only warn and limit
        // crossings reach the notification center.
        let notify = matches!(notice.level, NoticeLevel::Warn | NoticeLevel::Limit);
        if let Some(key) = notice_keys(notice, false).filter(|_| notify) {
            if note_once(&mut file, key, now) {
                outbox.push(warning_notification(notice));
                changed = true;
            }
        }
        if chat {
            if let Some(key) = notice_keys(notice, true) {
                if note_once(&mut file, key, now) {
                    conversation.push(notice.clone());
                    changed = true;
                }
            }
        }
    }
    if prune_notices(&mut file, &local_now) {
        changed = true;
    }
    if changed {
        if let Err(error) = store.save(&file) {
            eprintln!("[budgets] could not save budget state: {error}");
        }
    }
    drop(guard);
    for input in outbox {
        send(vault, input);
    }
    if changed {
        emit_changed();
    }
    let budget = RunBudget {
        intent: intent.clone(),
        seed_levels: eval.seed_levels,
        max_budget_usd,
        spend_grant,
        file_hash: hash,
        checks,
        claimed_limits: claimed,
        vault: Some(vault.to_path_buf()),
        fallback: None,
    };
    match eval.blocked {
        Some(mut notice) => {
            notice.episode = notice
                .budget_id
                .as_deref()
                .and_then(|id| file.pauses.get(id))
                .map(|pause| pause.paused_at.clone());
            Preflight::Blocked(notice)
        }
        None if conversation.is_empty() => Preflight::Clear(budget),
        None => Preflight::Warn(budget, conversation),
    }
}

/// Read-only: whether a hard-stop budget holds this enforced run right now
/// (an existing pause, a budget at its limit, an unreadable store or ledger).
/// Opens nothing and notifies nothing (Run now's up-front check, PR 9).
pub fn is_blocked(
    store: &Result<Store, String>,
    vault: &Path,
    intent: &RunIntent,
    now: DateTime<Utc>,
) -> Option<BudgetNotice> {
    let store = match store {
        Ok(store) => store,
        Err(error) => return load_failure_notice(intent, error),
    };
    let file = match store.load() {
        Ok(file) => file,
        Err(error) => return load_failure_notice(intent, &error),
    };
    if !intent.enforced() || file.budgets.is_empty() {
        return None;
    }
    let reader = LedgerReader::new(vault);
    let observed = |def: &BudgetDef, bounds: &WindowBounds| reader.totals(def, bounds);
    evaluate(&file, intent, &observed, &local(now)).blocked
}

/// Read-only preflight for a process that must not write (PR 8b's team-mcp
/// delegates): the same verdict with no pause, notice or save. Without a
/// vault root (no ledger to read) an enforced intent under any applicable
/// hard-stop budget is blocked (fail closed).
pub fn preflight_readonly(
    store: &Result<Store, String>,
    vault_root: Option<&Path>,
    intent: &RunIntent,
    now: DateTime<Utc>,
) -> Preflight {
    let none = || Preflight::Clear(RunBudget::none(intent.clone()));
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            return load_failure_notice(intent, error).map_or_else(none, Preflight::Blocked)
        }
    };
    let (file, hash) = match store.snapshot() {
        Ok(loaded) => loaded,
        Err(error) => {
            return load_failure_notice(intent, &error).map_or_else(none, Preflight::Blocked)
        }
    };
    let Some(vault) = vault_root else {
        let held = intent.enforced()
            && file
                .budgets
                .iter()
                .any(|def| def.hard_stop && intent.applies(&def.scope));
        return if held {
            load_failure_notice(
                intent,
                "the usage ledger's vault is unknown to this process",
            )
            .map_or_else(none, Preflight::Blocked)
        } else {
            none()
        };
    };
    let reader = LedgerReader::new(vault);
    let observed = |def: &BudgetDef, bounds: &WindowBounds| reader.totals(def, bounds);
    let eval = evaluate(&file, intent, &observed, &local(now));
    let checks = guard_checks(&file, &eval, &|def, bounds| reader.counted(def, bounds));
    let claimed = claimed_limits(&eval);
    let (max_budget_usd, spend_grant) = match grant_spend(vault, intent, &eval) {
        Ok(grant) => grant,
        Err(notice) => return Preflight::Blocked(notice),
    };
    let budget = RunBudget {
        intent: intent.clone(),
        seed_levels: eval.seed_levels,
        max_budget_usd,
        spend_grant,
        file_hash: hash,
        checks,
        claimed_limits: claimed,
        vault: Some(vault.to_path_buf()),
        fallback: None,
    };
    match eval.blocked {
        Some(notice) => Preflight::Blocked(notice),
        None if eval.warnings.is_empty() => Preflight::Clear(budget),
        None => Preflight::Warn(budget, eval.warnings),
    }
}

// ---------------------------------------------------------------------------
// Ceiling fallback (merged plan slice 3, decision 6)
// ---------------------------------------------------------------------------

/// Appended to the blocking notice when an `agent:` ceiling stops a run and
/// no other agent could take it over, so the stop says why it is a stop.
pub const NO_ROOM_NOTE: &str = "No other agent has room under its budgets to take it over.";

/// Decision 6: an enforced run its own agent's `agent:` hard-stop budget
/// held, moved at the claim to another agent with room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetFallback {
    pub from: AgentKind,
    pub to: AgentKind,
    /// The blocking notice's message: why `from` could not run.
    pub reason: String,
}

impl BudgetFallback {
    /// The receipt's record of the switch.
    pub fn receipt_switch(&self) -> crate::evidence::ReceiptSwitch {
        crate::evidence::ReceiptSwitch {
            kind: crate::evidence::SWITCH_AGENT.into(),
            from: self.from.label().into(),
            to: self.to.label().into(),
            reason: self.reason.clone(),
        }
    }
}

/// The agent whose own `agent:` budget blocks `intent`, the only ceiling
/// another agent can have room under: a global, automation or heartbeat
/// budget holds every agent alike, and usage that could not be read is not
/// a ceiling at all.
fn agent_ceiling(intent: &RunIntent, notice: &BudgetNotice) -> Option<AgentKind> {
    let agent = intent.agent?;
    let own = Scope::Agent(agent).key();
    (notice.level != NoticeLevel::Unavailable && notice.scope.as_deref() == Some(own.as_str()))
        .then_some(agent)
}

/// Whether every hard-stop budget over `intent` can measure its agent, so a
/// fallback never moves work where a ceiling could not see it (Antigravity
/// reports no usage at all).
fn measurable(file: &BudgetsFile, intent: &RunIntent) -> bool {
    let Some(agent) = intent.agent else {
        return false;
    };
    file.budgets
        .iter()
        .filter(|def| def.hard_stop && intent.applies(&def.scope))
        .all(|def| can_report(agent, def.metric))
}

/// [`preflight`] with decision 6's fallback: when the run's own agent is at
/// an `agent:` hard-stop ceiling, each of `candidates()` (the caller's
/// agents able to run this work, in order; only asked then) is tried with
/// the full preflight, and the first with room runs it: the verdict carries
/// [`RunBudget::fallback`]. With none, the original block stands, its
/// message ending in [`NO_ROOM_NOTE`]. Any other verdict is returned as is.
/// A candidate's preflight is real (it may open that agent's own pause),
/// because each is a claim for that agent.
pub fn preflight_with_fallback(
    store: &Result<Store, String>,
    vault: &Path,
    intent: &RunIntent,
    candidates: &dyn Fn() -> Vec<AgentKind>,
    now: DateTime<Utc>,
) -> Preflight {
    let mut notice = match preflight(store, vault, intent, now) {
        Preflight::Blocked(notice) => notice,
        verdict => return verdict,
    };
    let Some(from) = agent_ceiling(intent, &notice) else {
        return Preflight::Blocked(notice);
    };
    let file = store.as_ref().ok().and_then(|store| store.load().ok());
    for to in candidates().into_iter().filter(|to| *to != from) {
        let alternate = RunIntent {
            agent: Some(to),
            ..intent.clone()
        };
        if !file
            .as_ref()
            .is_some_and(|file| measurable(file, &alternate))
        {
            continue;
        }
        let fallback = BudgetFallback {
            from,
            to,
            reason: notice.message.clone(),
        };
        match preflight(store, vault, &alternate, now) {
            Preflight::Blocked(_) => continue,
            Preflight::Clear(mut budget) => {
                budget.fallback = Some(fallback);
                return Preflight::Clear(budget);
            }
            Preflight::Warn(mut budget, notices) => {
                budget.fallback = Some(fallback);
                return Preflight::Warn(budget, notices);
            }
        }
    }
    notice.message = format!("{} {NO_ROOM_NOTE}", notice.message);
    Preflight::Blocked(notice)
}

// ---------------------------------------------------------------------------
// After a run
// ---------------------------------------------------------------------------

/// Bring budgets up to date after a run's usage row was written and BEFORE
/// its lease is released (every lease-releasing funnel). Infallible: every
/// error is logged. It re-evaluates the budgets the finished run counted
/// toward and opens a pause for every hard-stop budget now at its limit,
/// EVEN when the finished run was chat (chat is never stopped, but the
/// automations under that budget are), notifies warnings once per window,
/// and removes lapsed pauses.
pub fn after_run(
    store: &Result<Store, String>,
    vault: &Path,
    intent: &RunIntent,
    now: DateTime<Utc>,
) -> Vec<BudgetEffect> {
    after_run_claimed(store, vault, intent, None, now)
}

/// [`after_run`] for a run that carries its claim's [`RunBudget`] (a workflow
/// or heartbeat run, Paperclip PR 8b). When `budgets.json` changed during the
/// run (its hash differs from the claim's), every hard-stop budget the claim
/// would have stopped this run under is judged at the LIMIT THE CLAIM READ,
/// not a raised one: at or over it, a pause opens at that limit
/// ([`BudgetEffect::ClaimLimitKept`] + [`BudgetEffect::Paused`]), so an agent
/// that raises its own limit mid-run gets no extra room. A budget the edit
/// deleted or turned soft cannot hold a pause (it lapses at once); that edit
/// keeps only the "Budgets changed outside Hollow" notice. A host save
/// during the run (Settings) changes the hash too, and is treated the same
/// way: the next claim reads the new limit anyway.
pub fn after_run_claimed(
    store: &Result<Store, String>,
    vault: &Path,
    intent: &RunIntent,
    claim: Option<&RunBudget>,
    now: DateTime<Utc>,
) -> Vec<BudgetEffect> {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            report_load_error(vault, error);
            return Vec::new();
        }
    };
    let guard = lock(&BUDGETS_LOCK);
    let (mut file, hash) = match store.load_for_write_hashed(vault) {
        Ok(loaded) => loaded,
        Err(error) => {
            drop(guard);
            report_load_error(vault, &error);
            return Vec::new();
        }
    };
    report_load_ok(vault, store);
    if file.budgets.is_empty() && file.pauses.is_empty() {
        return Vec::new();
    }
    // The claim's limits, only when the file changed during the run.
    let kept_limits = claim
        .filter(|claim| claim.file_hash.is_some() && claim.file_hash != hash)
        .map(|claim| &claim.claimed_limits);
    let local_now = local(now);
    let reader = LedgerReader::new(vault);
    let observed = |def: &BudgetDef, bounds: &WindowBounds| reader.totals(def, bounds);
    let eval = evaluate(&file, intent, &observed, &local_now);

    let mut effects = Vec::new();
    let mut outbox: Outbox = Vec::new();
    let mut changed = apply_lapses(vault, &mut file, &eval.lapsed, &mut effects);
    let mut paused: HashSet<String> = HashSet::new();
    for check in &eval.checks {
        if !check.hard_stop {
            continue;
        }
        let Some(value) = check.observed() else {
            continue;
        };
        // A limit raised mid-run is held to the claim's (never loosened).
        let kept = kept_limits
            .and_then(|limits| limits.get(&check.budget_id).copied())
            .filter(|claimed| *claimed < check.limit && value >= *claimed);
        let limit = match (check.state, kept) {
            (_, Some(claimed)) => claimed,
            (BudgetState::Over, None) => check.limit,
            _ => continue,
        };
        let request = PauseRequest {
            budget_id: check.budget_id.clone(),
            bounds: check.bounds.clone(),
            observed: value,
            limit,
        };
        let estimated = check.totals.as_ref().is_some_and(|t| t.estimated);
        if let Some(effect) = open_pause(vault, &mut file, &request, estimated, now, &mut outbox) {
            paused.insert(check.budget_id.clone());
            if kept.is_some() {
                effects.push(BudgetEffect::ClaimLimitKept {
                    budget_id: check.budget_id.clone(),
                });
            }
            effects.push(effect);
            changed = true;
        }
    }
    for notice in &eval.warnings {
        let id = notice.budget_id.clone().unwrap_or_default();
        let notify = matches!(notice.level, NoticeLevel::Warn | NoticeLevel::Limit);
        if !notify || (notice.level == NoticeLevel::Limit && paused.contains(&id)) {
            continue;
        }
        if let Some(key) = notice_keys(notice, false) {
            if note_once(&mut file, key.clone(), now) {
                outbox.push(warning_notification(notice));
                effects.push(BudgetEffect::Warned { key });
                changed = true;
            }
        }
    }
    if prune_notices(&mut file, &local_now) {
        changed = true;
    }
    if changed {
        if let Err(error) = store.save(&file) {
            eprintln!("[budgets] could not save budget state after a run: {error}");
        }
    }
    drop(guard);
    for input in outbox {
        send(vault, input);
    }
    if changed {
        emit_changed();
    }
    effects
}

/// The short run-log summary of a blocking notice (the note is sanitized by
/// `vault_workflows::budget_skip_entry`).
pub fn skip_detail(notice: &BudgetNotice) -> String {
    if notice.level == NoticeLevel::Unavailable {
        // Never "over its limit": nothing says the scope reached it.
        return match notice.scope_label.as_deref() {
            Some(label) => format!("{label}, usage could not be read"),
            None => "budget settings or the usage ledger could not be read".into(),
        };
    }
    match (
        notice.scope_label.as_deref(),
        notice.metric,
        notice.observed,
        notice.limit,
        notice.window.as_deref(),
    ) {
        (Some(label), Some(metric), Some(observed), Some(limit), Some(window)) => format!(
            "{label}, {} of {} this {window}",
            format_amount(metric, observed, notice.estimated),
            format_amount(metric, limit, false)
        ),
        (Some(label), _, _, _, Some(window)) => format!("{label}, over its {window} limit"),
        _ => "budget settings or the usage ledger could not be read".into(),
    }
}

// ---------------------------------------------------------------------------
// Mid-run enforcement (Paperclip PR 8b)
// ---------------------------------------------------------------------------

/// Claude's result subtype when its `--max-budget-usd` backstop ended a run
/// (recorded 2026-09-25: exit status 1, no result text, full `modelUsage`
/// with an all-zero top-level `usage`).
pub const BACKSTOP_SUBTYPE: &str = "error_max_budget_usd";

/// One token budget a [`BudgetGuard`] watches: the definition and window as
/// the claim evaluated them, and the usage observed then (before this run).
#[derive(Debug, Clone, PartialEq)]
pub struct GuardCheck {
    pub def: BudgetDef,
    pub bounds: WindowBounds,
    /// Observed in the budget's window at the claim, before this run.
    pub baseline: f64,
    /// Crossing the limit stops this run (an enforced intent and a hard
    /// stop); otherwise the guard only warns.
    pub enforced: bool,
    /// Merged plan slice 3: run key → the tokens `baseline` already counts
    /// of each ledger row under this scope, so the shared reservation adds
    /// only what each other run used since the claim. Empty when the claim
    /// read no rows (or a test built the check by hand).
    pub counted: BTreeMap<String, u64>,
}

/// The token budgets an evaluation leaves a mid-run guard to watch: every
/// applicable token budget whose baseline was read (a paused or unreadable
/// one either blocked the run already or is left to `after_run`).
/// The claim-time limit of every applicable budget that would stop the run
/// (an enforced intent and a hard stop), for [`after_run_claimed`].
fn claimed_limits(eval: &Evaluation) -> BTreeMap<String, f64> {
    eval.checks
        .iter()
        .filter(|check| check.enforced)
        .map(|check| (check.budget_id.clone(), check.limit))
        .collect()
}

fn guard_checks(
    file: &BudgetsFile,
    eval: &Evaluation,
    counted: &dyn Fn(&BudgetDef, &WindowBounds) -> BTreeMap<String, u64>,
) -> Vec<GuardCheck> {
    eval.checks
        .iter()
        .filter(|check| check.metric == Metric::Tokens)
        .filter(|check| {
            matches!(
                check.state,
                BudgetState::Ok | BudgetState::Warning | BudgetState::Over
            )
        })
        .filter_map(|check| {
            let def = file.budgets.iter().find(|def| def.id == check.budget_id)?;
            Some(GuardCheck {
                def: def.clone(),
                bounds: check.bounds.clone(),
                baseline: check.observed()?,
                enforced: check.enforced,
                counted: counted(def, &check.bounds),
            })
        })
        .collect()
}

/// Keep a notice safe inside a workflow run-log note and a tool error: one
/// line, no em dash (the run-log separator), no `<`/`>` (so it can never
/// open or close a hidden marker). Scope labels carry workflow slugs, which
/// may hold any of these.
fn note_safe(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '—' => '-',
            '<' | '>' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The notice of a run its guard stopped at an enforced token limit.
pub fn stop_notice(def: &BudgetDef, bounds: &WindowBounds, value: f64) -> BudgetNotice {
    let message = note_safe(&format!(
        "{BUDGET_STOP_PREFIX} {} reached {} {} limit ({}) during this run, so Hollow stopped \
         it. Raise and resume the budget in Automations, or wait for the {} to reset.",
        def.scope.label(),
        window_adjective(def.window),
        def.metric.noun(),
        of_limit(def, value, false),
        window_noun(def.window)
    ));
    notice(
        def,
        bounds,
        NoticeLevel::Limit,
        Some(value),
        false,
        true,
        message,
    )
}

/// The notice of a run Claude's own spending backstop ended.
pub fn backstop_notice(kind: AgentKind) -> BudgetNotice {
    BudgetNotice {
        budget_id: None,
        scope: None,
        scope_label: None,
        level: NoticeLevel::Limit,
        metric: Some(Metric::CostUsd),
        window: None,
        window_key: None,
        observed: None,
        limit: None,
        blocking: true,
        estimated: false,
        message: format!(
            "{BUDGET_STOP_PREFIX} {} stopped at this run's spending backstop.",
            kind.label()
        ),
        episode: None,
    }
}

/// A budget stop with no recorded notice (never expected: the guard records
/// one before it requests the stop).
pub fn generic_stop_notice(kind: AgentKind) -> BudgetNotice {
    BudgetNotice {
        message: format!(
            "{BUDGET_STOP_PREFIX} {} passed a token limit during this run.",
            kind.label()
        ),
        metric: Some(Metric::Tokens),
        ..backstop_notice(kind)
    }
}

/// What one [`BudgetGuard::observe`] found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GuardOutcome {
    /// An enforced limit was crossed: end the run (set once; later calls
    /// return None).
    pub stop: Option<BudgetNotice>,
    /// Warn or limit levels crossed inside the run, each once.
    pub warnings: Vec<BudgetNotice>,
}

/// Checks a run's tokens so far against its claim-time token budgets
/// (Paperclip PR 8b), measured honestly across concurrent runs (merged plan
/// slice 3, decision 6). A check's total is the claim-time baseline, plus
/// this run's own running total, plus what every OTHER run under the same
/// scope used since the claim: in-process runs from the shared in-flight
/// reservation ([`reserve`]), exact and immediate; runs in other processes
/// (team-mcp delegates, and a delegate's view of its parent) from the
/// ledger's checkpoints, reread at most every [`RESERVATION_REFRESH`]. Before
/// this, each guard saw only its own run, so N concurrent runs could each
/// spend the whole remaining budget. It is still a lower bound (side models
/// report only in the final result; another process's run is up to one
/// checkpoint behind). It is seeded with the levels the baseline already
/// met, so it fires only on a crossing inside the run. An enforced check
/// stops the run; every other crossing is a warning. The finished run's
/// `after_run` catches up from the ledger.
#[derive(Debug, Clone)]
pub struct BudgetGuard {
    checks: Vec<GuardCheck>,
    fired: BTreeSet<String>,
    stop: Option<BudgetNotice>,
    /// A result envelope arrived: the agent has finished its work.
    finished: bool,
    /// The vault whose runs share this guard's budgets (None: only this run
    /// is seen, as before slice 3).
    vault: Option<PathBuf>,
    /// This run's own ledger key ([`Self::bind_run`]): its tokens arrive as
    /// `observe`'s argument, so its reservation is never counted again.
    own: Option<String>,
    /// Per check: run key → tokens of every ledger row its scope covers, as
    /// last read (starts as the claim's [`GuardCheck::counted`]).
    seen: Vec<BTreeMap<String, u64>>,
    refreshed: Option<Instant>,
    /// Keeps released runs in the reservation while this guard may still
    /// need their last figure.
    #[allow(dead_code)] // held, not read: its Drop releases the runs
    ticket: Option<Arc<GuardTicket>>,
}

impl BudgetGuard {
    /// A guard for `budget`, or None when no token budget watches the run.
    pub fn new(budget: &RunBudget) -> Option<Self> {
        budget.usage_stream().then(|| BudgetGuard {
            checks: budget.checks.clone(),
            fired: budget.seed_levels.clone(),
            stop: None,
            finished: false,
            vault: budget.vault.clone(),
            own: None,
            seen: budget
                .checks
                .iter()
                .map(|check| check.counted.clone())
                .collect(),
            refreshed: None,
            ticket: budget.vault.as_deref().map(GuardTicket::open),
        })
    }

    /// The guard for a run of `kind`: Claude only. Codex and Antigravity are
    /// checked at the claim only (their usage arrives once, in the final
    /// `turn.completed`, after the work is done), so they get no guard.
    pub fn for_agent(budget: &RunBudget, kind: AgentKind) -> Option<Self> {
        if kind == AgentKind::Claude {
            Self::new(budget)
        } else {
            None
        }
    }

    /// Name the ledger row whose tokens this guard is fed
    /// (`UsageRun::key`). Every caller binds before each check, because a
    /// chat turn that fails over to another account (R1) continues on a new
    /// row: the old row then counts as another run's usage since the claim,
    /// which it is.
    pub fn bind_run(&mut self, run_key: &str) {
        if self.own.as_deref() != Some(run_key) {
            self.own = Some(run_key.to_string());
        }
    }

    /// Check one parsed event of the run. `grew` is what
    /// `UsageRun::observe` returned for it and `running` the run's
    /// `running_total` after it. A result envelope, and everything after
    /// one, is never checked: the agent has already finished, and its
    /// terminal total (side models included) must not turn a completed run
    /// into a budget stop. The finished run's `after_run` catches up from
    /// the ledger instead.
    pub fn on_event(
        &mut self,
        evt: &crate::agent::AgentEvent,
        grew: bool,
        running: Option<crate::usage::TokenCounts>,
    ) -> GuardOutcome {
        if matches!(evt, crate::agent::AgentEvent::Result { .. }) {
            self.finished = true;
        }
        if self.finished || !grew {
            return GuardOutcome::default();
        }
        self.observe(running)
    }

    /// Check the run's own tokens so far (`UsageRun::running_total`). Tokens
    /// are input (cache creation included) plus output, as the ledger counts
    /// them; cache reads never count. Every other run's usage since the
    /// claim is added (see the type's doc).
    pub fn observe(&mut self, run: Option<crate::usage::TokenCounts>) -> GuardOutcome {
        let mut out = GuardOutcome::default();
        let Some(run) = run else {
            return out;
        };
        if self.stop.is_some() {
            return out;
        }
        let used = run.input.saturating_add(run.output);
        self.refresh();
        let totals = self.totals(used);
        if let Some((check, value)) = self.checks.iter().zip(&totals).find(|(check, value)| {
            check.enforced && check.def.hard_stop && **value >= check.def.limit
        }) {
            let notice = stop_notice(&check.def, &check.bounds, *value);
            self.stop = Some(notice.clone());
            out.stop = Some(notice);
            return out;
        }
        for (check, value) in self.checks.iter().zip(totals) {
            let def = &check.def;
            let warn_key = level_key(&def.id, NoticeLevel::Warn);
            let limit_key = level_key(&def.id, NoticeLevel::Limit);
            if value >= def.limit {
                if self.fired.insert(limit_key) {
                    self.fired.insert(warn_key);
                    out.warnings
                        .push(limit_notice(def, &check.bounds, value, false, false));
                }
            } else if value >= def.warn_at() && self.fired.insert(warn_key) {
                out.warnings
                    .push(warn_notice(def, &check.bounds, value, false));
            }
        }
        out
    }

    /// The notice of the stop this guard requested, if any.
    pub fn stop_notice(&self) -> Option<&BudgetNotice> {
        self.stop.as_ref()
    }

    /// Each check's total with `used` as this run's own tokens.
    fn totals(&self, used: u64) -> Vec<f64> {
        let reserved = self.vault.as_deref().map(reserved_runs).unwrap_or_default();
        self.checks
            .iter()
            .zip(&self.seen)
            .map(|(check, seen)| {
                let others = shared_growth(check, seen, &reserved, self.own.as_deref());
                check.baseline + used.saturating_add(others) as f64
            })
            .collect()
    }

    /// Reread the ledger rows under each check's scope, at most every
    /// [`RESERVATION_REFRESH`] (and on the first check). A read that fails
    /// keeps the last rows.
    fn refresh(&mut self) {
        let Some(vault) = self.vault.clone() else {
            return;
        };
        if self
            .refreshed
            .is_some_and(|at| at.elapsed() < RESERVATION_REFRESH)
        {
            return;
        }
        self.refreshed = Some(Instant::now());
        for (check, seen) in self.checks.iter().zip(self.seen.iter_mut()) {
            let rows = crate::session_store::usage_rows_between(
                &vault,
                &crate::session_store::db_time(check.bounds.start),
                &crate::session_store::db_time(check.bounds.end),
            );
            if let Ok(rows) = rows {
                *seen = scope_runs(&vault, &rows, &check.def.scope);
            }
        }
    }
}

/// What every run other than `own` under `check`'s scope used since the
/// claim: for each run the ledger rows (`seen`) or the in-process
/// reservation know, its larger current figure less what the claim's
/// baseline already counted of it.
fn shared_growth(
    check: &GuardCheck,
    seen: &BTreeMap<String, u64>,
    reserved: &HashMap<String, InFlightRun>,
    own: Option<&str>,
) -> u64 {
    let scope = &check.def.scope;
    let live = |key: &str| {
        reserved
            .get(key)
            .filter(|run| scope.matches_row(&run.row()))
            .map(|run| run.tokens)
    };
    let mut keys: BTreeSet<&str> = seen.keys().map(String::as_str).collect();
    keys.extend(
        reserved
            .keys()
            .map(String::as_str)
            .filter(|&key| live(key).is_some()),
    );
    keys.into_iter()
        .filter(|key| Some(*key) != own)
        .map(|key| {
            let ledger = seen.get(key).copied().unwrap_or(0);
            let now = ledger.max(live(key).unwrap_or(0));
            now.saturating_sub(check.counted.get(key).copied().unwrap_or(0))
        })
        .fold(0, u64::saturating_add)
}

// ---------------------------------------------------------------------------
// The shared in-flight reservation (merged plan slice 3, decision 6)
// ---------------------------------------------------------------------------

/// How often a [`BudgetGuard`] rereads the ledger for runs outside this
/// process: the ledger's own checkpoint interval, so a reread can see news.
pub const RESERVATION_REFRESH: Duration = crate::usage_ledger::CHECKPOINT_EVERY;

/// One live run as the shared reservation holds it: the ledger columns
/// [`Scope::matches_row`] reads, and its tokens so far (input with cache
/// writes, plus output).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InFlightRun {
    pub surface: String,
    pub scope: Option<String>,
    pub parent_surface: Option<String>,
    pub agent: Option<String>,
    pub tokens: u64,
    /// A self-hosted model's run (Q8, [`LedgerRow::gpu_time`]).
    pub gpu_time: bool,
}

impl InFlightRun {
    fn row(&self) -> LedgerRow<'_> {
        LedgerRow {
            surface: &self.surface,
            scope: self.scope.as_deref(),
            parent_surface: self.parent_surface.as_deref(),
            agent: self.agent.as_deref(),
            gpu_time: self.gpu_time,
            ..LedgerRow::default()
        }
    }
}

#[derive(Debug)]
struct Reserved {
    run: InFlightRun,
    /// The reservation sequence number at its release; None while live.
    released: Option<u64>,
}

/// One vault's live runs and the guards watching them.
#[derive(Debug, Default)]
struct VaultReservations {
    runs: HashMap<String, Reserved>,
    /// Live guards, by the sequence number they opened at.
    guards: BTreeSet<u64>,
    /// Live Claude claims' spending backstops ([`SpendGrant`]), by id.
    grants: HashMap<u64, Granted>,
}

/// One live claim's spending backstop: the run it is for (which budgets
/// cover it) and the dollars its `--max-budget-usd` may spend.
#[derive(Debug)]
struct Granted {
    intent: RunIntent,
    usd: f64,
}

impl VaultReservations {
    /// Forget released runs no live guard can still need. A guard opened
    /// after a release reads that run's final row from the ledger instead.
    fn prune(&mut self) {
        let oldest = self.guards.first().copied();
        self.runs
            .retain(|_, reserved| match (reserved.released, oldest) {
                (None, _) => true,
                (Some(at), Some(oldest)) => at > oldest,
                (Some(_), None) => false,
            });
    }

    fn is_empty(&self) -> bool {
        self.runs.is_empty() && self.guards.is_empty() && self.grants.is_empty()
    }
}

/// Every vault's in-flight runs (per vault: a budget counts one vault's
/// ledger). A leaf lock: nothing else is taken while it is held.
static RESERVATIONS: Lazy<Mutex<HashMap<PathBuf, VaultReservations>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static RESERVATION_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_reservation_seq() -> u64 {
    RESERVATION_SEQ.fetch_add(1, Ordering::SeqCst)
}

/// Publish a live run's tokens so far (`usage_ledger::UsageRun`, on every
/// change of its running total, from every surface). Infallible.
pub fn reserve(vault: &Path, run_key: &str, run: InFlightRun) {
    let reserved = Reserved {
        run,
        released: None,
    };
    let mut all = lock(&RESERVATIONS);
    let entry = all.entry(vault.to_path_buf()).or_default();
    entry.runs.insert(run_key.to_string(), reserved);
}

/// A run wrote its final ledger row: its figure becomes `final_tokens` when
/// it has one, and it stays only while a guard opened before now may still
/// need it. Infallible.
pub fn release(vault: &Path, run_key: &str, final_tokens: Option<u64>) {
    let mut all = lock(&RESERVATIONS);
    let Some(entry) = all.get_mut(vault) else {
        return;
    };
    if let Some(reserved) = entry.runs.get_mut(run_key) {
        if let Some(tokens) = final_tokens {
            reserved.run.tokens = tokens;
        }
        reserved.released = Some(next_reservation_seq());
    }
    entry.prune();
    if entry.is_empty() {
        all.remove(vault);
    }
}

/// A snapshot of `vault`'s reserved runs.
fn reserved_runs(vault: &Path) -> HashMap<String, InFlightRun> {
    lock(&RESERVATIONS)
        .get(vault)
        .map(|entry| {
            entry
                .runs
                .iter()
                .map(|(key, reserved)| (key.clone(), reserved.run.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// A claim's hold on part of every spending ceiling over its run (merged
/// plan slice 3, review A·R6): the dollars [`RunBudget::max_budget_usd`]
/// lets its `--max-budget-usd` backstop spend. Dropped with the last clone of
/// the claim's [`RunBudget`], which outlives the run.
#[derive(Debug, PartialEq, Eq)]
pub struct SpendGrant {
    vault: PathBuf,
    id: u64,
}

impl Drop for SpendGrant {
    fn drop(&mut self) {
        let mut all = lock(&RESERVATIONS);
        if let Some(entry) = all.get_mut(&self.vault) {
            entry.grants.remove(&self.id);
            if entry.is_empty() {
                all.remove(&self.vault);
            }
        }
    }
}

/// The claim's spending backstop and its hold on it. A stream reports no
/// cost, so dollars cannot be checked mid-run; instead each live claim
/// holds the backstop it was given, and a later claim's headroom is what is
/// left after every hold under the same budget, with a one-cent floor. Only a
/// Claude claim holds dollars: no other agent gets the backstop. Holds are
/// per process: a team-mcp delegate's claim does not see the GUI's.
fn grant_spend(
    vault: &Path,
    intent: &RunIntent,
    eval: &Evaluation,
) -> Result<(Option<f64>, Option<Arc<SpendGrant>>), BudgetNotice> {
    if eval.blocked.is_some() || eval.max_budget_usd.is_none() {
        return Ok((eval.max_budget_usd, None));
    }
    let mut all = lock(&RESERVATIONS);
    let headroom = {
        let grants = all.get(vault).map(|entry| &entry.grants);
        let granted = |scope: &Scope| -> f64 {
            grants
                .into_iter()
                .flat_map(|grants| grants.values())
                .filter(|grant| grant.intent.applies(scope))
                .map(|grant| grant.usd)
                .sum()
        };
        spend_headroom(&eval.checks, &granted, 0.01)
    };
    let Some(usd) = headroom.filter(|_| intent.agent == Some(AgentKind::Claude)) else {
        return Ok((headroom, None));
    };
    let id = next_reservation_seq();
    all.entry(vault.to_path_buf()).or_default().grants.insert(
        id,
        Granted {
            intent: intent.clone(),
            usd,
        },
    );
    let grant = SpendGrant {
        vault: vault.to_path_buf(),
        id,
    };
    Ok((Some(usd), Some(Arc::new(grant))))
}

/// Tests: the dollars `vault`'s live claims hold.
#[cfg(test)]
fn granted_usd(vault: &Path) -> Vec<f64> {
    lock(&RESERVATIONS)
        .get(vault)
        .map(|entry| entry.grants.values().map(|grant| grant.usd).collect())
        .unwrap_or_default()
}

/// A live guard's place in its vault's reservation (dropped with the last
/// clone of the guard).
#[derive(Debug)]
struct GuardTicket {
    vault: PathBuf,
    seq: u64,
}

impl GuardTicket {
    fn open(vault: &Path) -> Arc<GuardTicket> {
        let seq = next_reservation_seq();
        lock(&RESERVATIONS)
            .entry(vault.to_path_buf())
            .or_default()
            .guards
            .insert(seq);
        Arc::new(GuardTicket {
            vault: vault.to_path_buf(),
            seq,
        })
    }
}

impl Drop for GuardTicket {
    fn drop(&mut self) {
        let mut all = lock(&RESERVATIONS);
        if let Some(entry) = all.get_mut(&self.vault) {
            entry.guards.remove(&self.seq);
            entry.prune();
            if entry.is_empty() {
                all.remove(&self.vault);
            }
        }
    }
}

/// Record budget notices a chat turn's guard delivered into its conversation
/// mid-turn (Paperclip PR 8b), under their `chat:` notice keys, so the next
/// turn's preflight does not deliver them again. Infallible: errors are
/// logged. GUI only (it saves `budgets.json`).
pub fn record_conversation_notices(
    store: &Result<Store, String>,
    vault: &Path,
    notices: &[BudgetNotice],
    now: DateTime<Utc>,
) {
    if notices.is_empty() {
        return;
    }
    let Ok(store) = store else {
        return;
    };
    let guard = lock(&BUDGETS_LOCK);
    let mut file = match store.load_for_write(vault) {
        Ok(file) => file,
        Err(error) => {
            drop(guard);
            eprintln!("[budgets] could not record delivered warnings: {error}");
            return;
        }
    };
    let mut changed = false;
    for notice in notices {
        if let Some(key) = notice_keys(notice, true) {
            if note_once(&mut file, key, now) {
                changed = true;
            }
        }
    }
    if changed {
        if let Err(error) = store.save(&file) {
            eprintln!("[budgets] could not record delivered warnings: {error}");
        }
    }
    drop(guard);
}

/// Tests: a run budget with one global daily token budget (`limit`, observed
/// `baseline` at the claim), enforced when `intent` is and `hard_stop` holds.
#[cfg(test)]
pub(crate) fn token_budget_for_test(
    intent: RunIntent,
    limit: f64,
    baseline: f64,
    hard_stop: bool,
) -> RunBudget {
    let def: BudgetDef = serde_json::from_value(json!({
        "id": "b-test",
        "scope": "global",
        "metric": "tokens",
        "window": "day",
        "limit": limit,
        "hardStop": hard_stop,
    }))
    .expect("a test budget");
    let bounds = window_bounds_in(def.window, &Utc::now());
    let enforced = intent.enforced() && hard_stop;
    RunBudget {
        checks: vec![GuardCheck {
            def,
            bounds,
            baseline,
            enforced,
            counted: BTreeMap::new(),
        }],
        ..RunBudget::none(intent)
    }
}

// ---------------------------------------------------------------------------
// Holds (Automations list) and the Settings view
// ---------------------------------------------------------------------------

/// A pause that no longer holds anything at `bounds` (its window).
fn pause_lapsed(def: &BudgetDef, pause: &BudgetPause, bounds: &WindowBounds) -> bool {
    !def.hard_stop || (!pause.held && pause.window_key != bounds.key)
}

/// Slug → blocking message for every listed workflow a usage budget holds
/// right now. Read-only (no pause, no notice); `Workflow.budgetHold`.
pub fn workflow_holds(
    vault: &Path,
    workflows: &[crate::types::Workflow],
    now: DateTime<Utc>,
) -> BTreeMap<String, String> {
    let store = Store::for_app(vault);
    let file = match store.as_ref().map_err(|e| e.clone()).and_then(|s| s.load()) {
        Ok(file) => file,
        Err(error) => {
            return workflows
                .iter()
                .filter_map(|w| {
                    load_failure_notice(&RunIntent::workflow(&w.slug, None), &error)
                        .map(|notice| (w.slug.clone(), notice.message))
                })
                .collect()
        }
    };
    if file.budgets.is_empty() {
        return BTreeMap::new();
    }
    let reader = LedgerReader::new(vault);
    let observed = |def: &BudgetDef, bounds: &WindowBounds| reader.totals(def, bounds);
    let local_now = local(now);
    let default = default_agent();
    workflows
        .iter()
        .filter_map(|w| {
            let intent =
                RunIntent::workflow(&w.slug, intent_agent_with(w.agent.as_deref(), default));
            evaluate(&file, &intent, &observed, &local_now)
                .blocked
                .map(|notice| (w.slug.clone(), notice.message))
        })
        .collect()
}

/// What Settings, Usage shows for one budget.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetStatus {
    pub id: String,
    pub scope: String,
    pub scope_label: String,
    pub metric: Metric,
    pub window: String,
    pub limit: f64,
    pub warn_percent: u8,
    pub hard_stop: bool,
    pub window_key: String,
    pub window_start: String,
    pub window_end: String,
    /// None when the ledger could not be read.
    pub observed: Option<f64>,
    pub percent: Option<f64>,
    pub state: BudgetState,
    pub pause: Option<BudgetPause>,
    pub runs: u32,
    /// For a spending budget, never a self-hosted model's run (Q8): those
    /// are `gpu_time_runs`.
    pub unknown_runs: u32,
    pub partial_runs: u32,
    pub unpriced_runs: u32,
    /// A spending budget's runs on a self-hosted model (Q8): billed by GPU
    /// time, no spend; 0 for a token budget.
    pub gpu_time_runs: u32,
    pub cached_input_tokens: u64,
    pub estimated: bool,
    pub enforceability: Enforceability,
    pub enforceability_reason: Option<String>,
    /// A `workflow:` scope whose automation no longer exists.
    pub scope_missing: bool,
    /// A starting limit (never saved automatically), when suggestions are on.
    pub suggested_limit: Option<f64>,
    pub ledger_error: Option<String>,
}

/// Which automations budgets hold right now.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedAutomations {
    pub all_automations: bool,
    pub heartbeat: bool,
    pub workflows: Vec<String>,
}

/// Settings, Usage's budgets panel.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetsView {
    pub budgets: Vec<BudgetStatus>,
    pub ledger_days: i64,
    pub suggestions_available: bool,
    pub blocked: BlockedAutomations,
    /// The store could not be read: automations are held and nothing is
    /// shown or saved until it is repaired.
    pub store_error: Option<String>,
    pub foreign_edit: bool,
    pub enforcement: String,
}

/// A budget from the Settings form (`id` absent = create).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetInput {
    #[serde(default)]
    pub id: Option<String>,
    pub scope: String,
    pub metric: Metric,
    pub window: String,
    pub limit: f64,
    #[serde(default)]
    pub warn_percent: Option<u8>,
    #[serde(default)]
    pub hard_stop: Option<bool>,
}

/// Totals of up to `count` completed windows before the current one.
fn past_series(
    reader: &LedgerReader<'_>,
    def: &BudgetDef,
    current: &WindowBounds,
    count: usize,
) -> Vec<f64> {
    let mut series = Vec::new();
    let mut start = current.start;
    for _ in 0..count {
        let Some(before) = start.checked_sub_signed(chrono::Duration::seconds(1)) else {
            break;
        };
        let bounds = window_bounds_in(def.window, &local(before));
        if let Ok(totals) = reader.totals(def, &bounds) {
            if totals.runs > 0 {
                series.push(totals.value(def.metric));
            }
        }
        start = bounds.start;
    }
    series
}

/// Settings, Usage. Never fails: a store that cannot be read is reported in
/// `storeError` (and through [`report_load_error`]).
pub fn view(
    store: &Result<Store, String>,
    vault: &Path,
    scope_agent: &dyn Fn(&Scope) -> Option<AgentKind>,
    now: DateTime<Utc>,
) -> BudgetsView {
    let first_at = crate::session_store::usage_ledger_since(vault)
        .ok()
        .flatten()
        .and_then(|at| crate::session_store::parse_iso(&at));
    let ledger_days = ledger_days(first_at, now);
    let suggestions = suggestions_available(first_at, now);
    let mut out = BudgetsView {
        budgets: Vec::new(),
        ledger_days,
        suggestions_available: suggestions,
        blocked: BlockedAutomations::default(),
        store_error: None,
        foreign_edit: false,
        enforcement: ENFORCEMENT_COPY.into(),
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            report_load_error(vault, error);
            out.store_error = Some(error.clone());
            out.blocked.all_automations = true;
            return out;
        }
    };
    let file = match store.load() {
        Ok(file) => file,
        Err(error) => {
            report_load_error(vault, &error);
            out.store_error = Some(error);
            out.blocked.all_automations = true;
            return out;
        }
    };
    report_load_ok(vault, store);
    out.foreign_edit = store.foreign_edit_seen();
    let local_now = local(now);
    let reader = LedgerReader::new(vault);
    let workflows = crate::vault_workflows::list(vault);
    for def in &file.budgets {
        let bounds = window_bounds_in(def.window, &local_now);
        let totals = reader.totals(def, &bounds);
        let pause = file
            .pauses
            .get(&def.id)
            .filter(|pause| !pause_lapsed(def, pause, &bounds))
            .cloned();
        let value = totals.as_ref().ok().and_then(|t| t.observed(def.metric));
        let state = if pause.is_some() {
            BudgetState::Paused
        } else {
            match value {
                None => BudgetState::Unknown,
                Some(v) if v >= def.limit => BudgetState::Over,
                Some(v) if v >= def.warn_at() => BudgetState::Warning,
                Some(_) => BudgetState::Ok,
            }
        };
        let (enforceability, reason) = enforceability(
            &def.scope,
            def.metric,
            scope_agent(&def.scope),
            totals.as_ref().ok(),
        );
        let scope_missing = match &def.scope {
            Scope::Workflow(slug) => !workflows.iter().any(|w| &w.slug == slug),
            _ => false,
        };
        let suggested = if suggestions {
            let mut series = past_series(&reader, def, &bounds, 4);
            if let Some(v) = value {
                series.push(v);
            }
            suggest_limit(&series)
        } else {
            None
        };
        let t = totals.as_ref().ok();
        out.budgets.push(BudgetStatus {
            id: def.id.clone(),
            scope: def.scope.key(),
            scope_label: def.scope.label(),
            metric: def.metric,
            window: def.window.as_str().to_string(),
            limit: def.limit,
            warn_percent: def.warn_percent,
            hard_stop: def.hard_stop,
            window_key: bounds.key.clone(),
            window_start: rfc3339_millis(&bounds.start),
            window_end: rfc3339_millis(&bounds.end),
            observed: value,
            percent: value.map(|v| v / def.limit * 100.0),
            state,
            pause,
            runs: t.map_or(0, |t| t.runs),
            unknown_runs: t.map_or(0, |t| t.shown_unknown(def.metric)),
            partial_runs: t.map_or(0, |t| t.partial_runs),
            unpriced_runs: t.map_or(0, |t| t.unpriced_runs),
            gpu_time_runs: t
                .filter(|_| def.metric == Metric::CostUsd)
                .map_or(0, |t| t.gpu_time_runs),
            cached_input_tokens: t.map_or(0, |t| t.cached_input_tokens),
            estimated: def.metric == Metric::CostUsd && t.is_some_and(|t| t.estimated),
            enforceability,
            enforceability_reason: reason,
            scope_missing,
            suggested_limit: suggested,
            ledger_error: totals.err(),
        });
    }
    let observed = |def: &BudgetDef, bounds: &WindowBounds| reader.totals(def, bounds);
    let default = scope_agent(&Scope::Heartbeat);
    out.blocked.heartbeat = evaluate(&file, &RunIntent::heartbeat(default), &observed, &local_now)
        .blocked
        .is_some();
    out.blocked.all_automations = file.budgets.iter().any(|def| {
        def.scope == Scope::Global
            && def.hard_stop
            && out.budgets.iter().any(|b| {
                b.id == def.id && matches!(b.state, BudgetState::Paused | BudgetState::Over)
            })
    });
    out.blocked.workflows = workflows
        .iter()
        .filter(|w| {
            let intent =
                RunIntent::workflow(&w.slug, scope_agent(&Scope::Workflow(w.slug.clone())));
            evaluate(&file, &intent, &observed, &local_now)
                .blocked
                .is_some()
        })
        .map(|w| w.slug.clone())
        .collect();
    out
}

// ---------------------------------------------------------------------------
// Settings writes and the pause lifecycle
// ---------------------------------------------------------------------------

fn new_budget_id(file: &BudgetsFile) -> String {
    loop {
        let id = format!("b-{:08x}", rand::random::<u32>());
        if !file.budgets.iter().any(|def| def.id == id) {
            return id;
        }
    }
}

fn store_of(store: &Result<Store, String>) -> Result<&Store, String> {
    store.as_ref().map_err(|error| error.clone())
}

/// This window's observed value for `def`, or an error naming the window.
fn observed_now(vault: &Path, def: &BudgetDef, now: DateTime<Utc>) -> Result<f64, String> {
    let bounds = window_bounds_in(def.window, &local(now));
    LedgerReader::new(vault)
        .totals(def, &bounds)
        .map(|totals| totals.value(def.metric))
        .map_err(|error| {
            format!(
                "Hollow could not read this {}'s usage ({error}), so nothing was changed.",
                window_noun(def.window)
            )
        })
}

fn above_observed(def: &BudgetDef, new_limit: f64, observed: f64) -> Result<(), String> {
    if !new_limit.is_finite() || new_limit <= observed {
        return Err(format!(
            "New limit must be above this {}'s usage ({}).",
            window_noun(def.window),
            format_amount(def.metric, observed, false)
        ));
    }
    Ok(())
}

/// Create or update a budget from Settings. Refuses budgets that could never
/// trip (`scope_agent` resolves what a workflow or the heartbeat runs on) and
/// duplicate (scope, metric, window). An edit that changes the budget's
/// identity, turns off its hard stop, or raises its limit above this window's
/// usage clears its pause and supersedes the pause's item.
pub fn save(
    store: &Result<Store, String>,
    vault: &Path,
    input: BudgetInput,
    scope_agent: &dyn Fn(&Scope) -> Option<AgentKind>,
    now: DateTime<Utc>,
) -> Result<BudgetDef, String> {
    let scope = Scope::parse(&input.scope)?;
    let window = UsageWindow::parse(&input.window)
        .ok_or_else(|| format!("Unknown budget window '{}'.", input.window))?;
    let store = store_of(store)?;
    let _guard = lock(&BUDGETS_LOCK);
    let mut file = store.load_for_write(vault)?;
    let existing = match input.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => Some(
            file.budgets
                .iter()
                .position(|def| def.id == id)
                .ok_or_else(|| "That budget no longer exists.".to_string())?,
        ),
        None => None,
    };
    let at = stamp(now);
    let def = BudgetDef {
        id: existing
            .map(|i| file.budgets[i].id.clone())
            .unwrap_or_else(|| new_budget_id(&file)),
        scope,
        metric: input.metric,
        window,
        limit: input.limit,
        warn_percent: input.warn_percent.unwrap_or(DEFAULT_WARN_PERCENT),
        hard_stop: input.hard_stop.unwrap_or(true),
        created_at: existing
            .map(|i| file.budgets[i].created_at.clone())
            .filter(|created| !created.is_empty())
            .unwrap_or_else(|| at.clone()),
        updated_at: at,
    };
    validate_def(&def, &file.budgets, scope_agent)?;
    let mut retire: Option<String> = None;
    if let Some(i) = existing {
        let old = file.budgets[i].clone();
        if let Some(pause) = file.pauses.get(&def.id).cloned() {
            let identity = old.scope != def.scope
                || old.metric != def.metric
                || old.window != def.window
                || !def.hard_stop;
            let clears = identity
                || observed_now(vault, &def, now).is_ok_and(|observed| def.limit > observed);
            if clears {
                file.pauses.remove(&def.id);
                retire = pause.review_item_id;
            }
        }
        file.budgets[i] = def.clone();
    } else {
        file.budgets.push(def.clone());
    }
    prune_notices(&mut file, &local(now));
    store.save(&file)?;
    if let Some(item) = retire {
        supersede_item(
            vault,
            &item,
            "Superseded: the budget was changed in Settings.",
        );
    }
    Ok(def)
}

/// Remove a budget, its pause (superseding the pause's item) and its notices.
/// A missing budget is not an error.
pub fn delete(store: &Result<Store, String>, vault: &Path, id: &str) -> Result<(), String> {
    let store = store_of(store)?;
    let _guard = lock(&BUDGETS_LOCK);
    let mut file = store.load_for_write(vault)?;
    let before = file.budgets.len();
    file.budgets.retain(|def| def.id != id);
    let pause = file.pauses.remove(id);
    if before == file.budgets.len() && pause.is_none() {
        return Ok(());
    }
    prune_notices(&mut file, &local(Utc::now()));
    store.save(&file)?;
    if let Some(item) = pause.and_then(|pause| pause.review_item_id) {
        supersede_item(vault, &item, "Superseded: the budget was deleted.");
    }
    Ok(())
}

/// The id of the pause `item` belongs to: the one whose linked item id AND
/// episode fingerprint match (both from `budgets.json`, never the payload).
fn pause_for_item(file: &BudgetsFile, item: &crate::review_queue::ReviewItem) -> Option<String> {
    let fingerprint = item.fingerprint.trim().to_lowercase();
    file.pauses
        .iter()
        .find(|(budget_id, pause)| {
            pause.review_item_id.as_deref() == Some(item.id.as_str())
                && pause_fingerprint(budget_id, &pause.episode) == fingerprint
        })
        .map(|(budget_id, _)| budget_id.clone())
}

/// The queue holds exactly one item with this id (it does not enforce unique
/// ids, so a planted duplicate is refused rather than acted on).
fn unique_item(vault: &Path, id: &str) -> Result<(), String> {
    let count = crate::review_queue::list(vault, None)?
        .iter()
        .filter(|item| item.id == id)
        .count();
    if count > 1 {
        return Err(DUPLICATE_ITEM_MESSAGE.into());
    }
    Ok(())
}

/// Apply a raise: the new limit, the pause cleared.
fn apply_raise(file: &mut BudgetsFile, budget_id: &str, new_limit: f64, now: DateTime<Utc>) {
    if let Some(def) = file.budgets.iter_mut().find(|def| def.id == budget_id) {
        def.limit = new_limit;
        def.updated_at = stamp(now);
    }
    file.pauses.remove(budget_id);
    prune_notices(file, &local(now));
}

/// Claim → apply → save → accept, calling `review_queue::fail` on any error
/// after the claim. The caller holds [`BUDGETS_LOCK`].
fn raise_through_item(
    store: &Store,
    vault: &Path,
    file: &mut BudgetsFile,
    item: &crate::review_queue::ReviewItem,
    budget_id: &str,
    new_limit: f64,
    now: DateTime<Utc>,
) -> Result<crate::review_queue::ReviewItem, String> {
    let mut payload = item.payload.clone();
    match payload.as_object_mut() {
        Some(object) => {
            object.insert("newLimit".into(), json!(new_limit));
        }
        None => payload = json!({ "newLimit": new_limit }),
    }
    crate::review_queue::claim(vault, &item.id, Some(payload))?;
    apply_raise(file, budget_id, new_limit, now);
    if let Err(error) = store.save(file) {
        let _ = crate::review_queue::fail(vault, &item.id, &error);
        return Err(error);
    }
    crate::review_queue::accept(vault, &item.id)
}

/// "Raise and resume" on a pause card. The new limit (the only value taken
/// from the edited payload) must be above this window's usage, checked before
/// anything is claimed. Everything else comes from `budgets.json`: the item
/// must be the one its pause links, by id and episode fingerprint. A pause
/// that already ended supersedes the card.
pub fn accept_pause(
    store: &Result<Store, String>,
    vault: &Path,
    item: &crate::review_queue::ReviewItem,
    edited: Option<&Value>,
    now: DateTime<Utc>,
) -> Result<crate::review_queue::ReviewItem, String> {
    let new_limit = edited
        .and_then(|payload| payload.get("newLimit"))
        .and_then(Value::as_f64)
        .ok_or_else(|| "Enter a new limit to raise and resume.".to_string())?;
    let store = store_of(store)?;
    let _guard = lock(&BUDGETS_LOCK);
    let mut file = store.load_for_write(vault)?;
    unique_item(vault, &item.id)?;
    let Some(budget_id) = pause_for_item(&file, item) else {
        supersede_item(vault, &item.id, "Superseded: this pause has ended.");
        return Err(ENDED_MESSAGE.into());
    };
    let Some(def) = file.budgets.iter().find(|def| def.id == budget_id).cloned() else {
        file.pauses.remove(&budget_id);
        let _ = store.save(&file);
        supersede_item(vault, &item.id, "Superseded: the budget was removed.");
        return Err(ENDED_MESSAGE.into());
    };
    let bounds = window_bounds_in(def.window, &local(now));
    let lapsed = file
        .pauses
        .get(&budget_id)
        .is_some_and(|pause| pause_lapsed(&def, pause, &bounds));
    if lapsed {
        file.pauses.remove(&budget_id);
        let _ = store.save(&file);
        supersede_item(
            vault,
            &item.id,
            &format!("Superseded: the {} reset.", window_noun(def.window)),
        );
        return Err(ENDED_MESSAGE.into());
    }
    let observed = observed_now(vault, &def, now)?;
    above_observed(&def, new_limit, observed)?;
    raise_through_item(store, vault, &mut file, item, &budget_id, new_limit, now)
}

/// "Keep paused": hold the pause across its window reset, then reject the
/// card. When the rejection fails the hold is rolled back. A card whose pause
/// already ended is superseded instead.
pub fn hold_pause_and_reject(
    store: &Result<Store, String>,
    vault: &Path,
    item: &crate::review_queue::ReviewItem,
) -> Result<crate::review_queue::ReviewItem, String> {
    let store = store_of(store)?;
    let _guard = lock(&BUDGETS_LOCK);
    let mut file = store.load_for_write(vault)?;
    unique_item(vault, &item.id)?;
    let Some(budget_id) = pause_for_item(&file, item) else {
        supersede_item(vault, &item.id, "Superseded: this pause has ended.");
        return Err(ENDED_MESSAGE.into());
    };
    let was_held = file.pauses.get(&budget_id).is_some_and(|pause| pause.held);
    if let Some(pause) = file.pauses.get_mut(&budget_id) {
        pause.held = true;
    }
    store.save(&file)?;
    match crate::review_queue::reject(vault, &item.id) {
        Ok(rejected) => Ok(rejected),
        Err(error) => {
            if let Some(pause) = file.pauses.get_mut(&budget_id) {
                pause.held = was_held;
            }
            if let Err(save_error) = store.save(&file) {
                eprintln!("[budgets] could not roll back the hold on {budget_id}: {save_error}");
            }
            Err(error)
        }
    }
}

/// `resolve_review_item` for a `budget_pause` card (the IPC delegates here so
/// the routing is testable): the confirmed fingerprint must match when
/// supplied (checked before anything is claimed or written); "reject" is
/// "Keep paused" ([`hold_pause_and_reject`], never the plain queue rejection,
/// which would let the pause lapse at the reset); "accept" is "Raise and
/// resume" ([`accept_pause`]). Both re-derive the pause from `budgets.json`.
pub fn resolve_pause(
    store: &Result<Store, String>,
    vault: &Path,
    item: &crate::review_queue::ReviewItem,
    action: &str,
    expected_fingerprint: Option<&str>,
    edited: Option<&Value>,
    now: DateTime<Utc>,
) -> Result<crate::review_queue::ReviewItem, String> {
    if let Some(expected) = expected_fingerprint {
        if !item.fingerprint.eq_ignore_ascii_case(expected.trim()) {
            return Err("This pause changed after you opened it. Review it again.".into());
        }
    }
    let resolved = match action.trim().to_lowercase().as_str() {
        "reject" => hold_pause_and_reject(store, vault, item),
        "accept" => accept_pause(store, vault, item, edited, now),
        other => Err(format!("unsupported review resolution: {other}")),
    };
    emit_changed();
    resolved
}

/// `snooze_review_item`'s check: a budget pause is resolved (Raise and resume
/// or Keep paused), never snoozed, because a snoozed pause would still hold
/// every automation under it.
pub fn refuse_pause_snooze(vault: &Path, id: &str) -> Result<(), String> {
    let is_pause = crate::review_queue::list(vault, Some(REVIEW_KIND))?
        .iter()
        .any(|item| item.id == id);
    if is_pause {
        return Err(
            "A budget pause can't be snoozed. Raise and resume it, or keep it paused.".into(),
        );
    }
    Ok(())
}

/// Settings, Usage "Resume…": raise a paused (or held) budget above this
/// window's usage and clear its pause. A still-pending card is claimed and
/// accepted; a live card that cannot be claimed is superseded; a rejected
/// ("Keep paused") card stays as the history of that choice.
pub fn resume(
    store: &Result<Store, String>,
    vault: &Path,
    id: &str,
    new_limit: f64,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let store = store_of(store)?;
    let _guard = lock(&BUDGETS_LOCK);
    let mut file = store.load_for_write(vault)?;
    let def = file
        .budgets
        .iter()
        .find(|def| def.id == id)
        .cloned()
        .ok_or_else(|| "That budget no longer exists.".to_string())?;
    let observed = observed_now(vault, &def, now)?;
    above_observed(&def, new_limit, observed)?;
    let linked = file
        .pauses
        .get(id)
        .and_then(|pause| pause.review_item_id.clone());
    let item = match &linked {
        Some(item_id) => {
            let items: Vec<crate::review_queue::ReviewItem> =
                crate::review_queue::list(vault, None)?
                    .into_iter()
                    .filter(|item| &item.id == item_id)
                    .collect();
            match items.as_slice() {
                [only] if pause_for_item(&file, only).as_deref() == Some(id) => Some(only.clone()),
                _ => None,
            }
        }
        None => None,
    };
    match item {
        Some(item) if item.status == "pending" => {
            raise_through_item(store, vault, &mut file, &item, id, new_limit, now).map(|_| ())
        }
        other => {
            apply_raise(&mut file, id, new_limit, now);
            store.save(&file)?;
            if let Some(item_id) = linked {
                let live = other
                    .as_ref()
                    .is_none_or(|item| matches!(item.status.as_str(), "pending" | "snoozed"));
                if live {
                    supersede_item(vault, &item_id, "Superseded: resumed from Settings.");
                }
            }
            Ok(())
        }
    }
}

/// A workflow was renamed in the automation editor: its `workflow:` budgets
/// follow it (pauses are keyed by budget id, so they follow too). A budget
/// that would duplicate one already set for the new name stays behind and is
/// logged. Called under PR 5's write guard; logged, never fails the rename.
pub fn rename_workflow_scope(store: &Result<Store, String>, vault: &Path, old: &str, new: &str) {
    if old == new {
        return;
    }
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            eprintln!("[budgets] rename {old} → {new}: {error}");
            return;
        }
    };
    let result = store.with_file(vault, |file| {
        let mut changed = false;
        for i in 0..file.budgets.len() {
            if file.budgets[i].scope != Scope::Workflow(old.to_string()) {
                continue;
            }
            let (metric, window) = (file.budgets[i].metric, file.budgets[i].window);
            let taken = file.budgets.iter().any(|def| {
                def.scope == Scope::Workflow(new.to_string())
                    && def.metric == metric
                    && def.window == window
            });
            if taken {
                eprintln!(
                    "[budgets] {} stays on {old}: {new} already has that budget",
                    file.budgets[i].id
                );
                continue;
            }
            file.budgets[i].scope = Scope::Workflow(new.to_string());
            changed = true;
        }
        Ok(((), changed))
    });
    match result {
        Ok(()) => emit_changed(),
        Err(error) => eprintln!("[budgets] rename {old} → {new}: {error}"),
    }
}

/// A workflow was deleted in the automation editor: its `workflow:` budgets,
/// their pauses (items superseded) and notices go. Logged, never fails the
/// delete.
pub fn forget_workflow_scope(store: &Result<Store, String>, vault: &Path, slug: &str) {
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            eprintln!("[budgets] forget {slug}: {error}");
            return;
        }
    };
    let scope = Scope::Workflow(slug.to_string());
    let result = store.with_file(vault, |file| {
        let gone: Vec<String> = file
            .budgets
            .iter()
            .filter(|def| def.scope == scope)
            .map(|def| def.id.clone())
            .collect();
        if gone.is_empty() {
            return Ok((Vec::new(), false));
        }
        file.budgets.retain(|def| def.scope != scope);
        let items: Vec<String> = gone
            .iter()
            .filter_map(|id| file.pauses.remove(id))
            .filter_map(|pause| pause.review_item_id)
            .collect();
        prune_notices(file, &local(Utc::now()));
        Ok((items, true))
    });
    match result {
        Ok(items) => {
            for item in items {
                supersede_item(vault, &item, "Superseded: the automation was deleted.");
            }
            emit_changed();
        }
        Err(error) => eprintln!("[budgets] forget {slug}: {error}"),
    }
}

// ---------------------------------------------------------------------------
// Named spending scopes: `main-agent` and `overnight` (run ledger v2, F2)
// ---------------------------------------------------------------------------
//
// Two scopes whose limit is not a `budgets.json` row. `main-agent` is the
// Main agent's daily hard stop (its number stays in Settings, Main agent,
// `main_agent::MainAgentSettings::daily_cap_usd`; the evaluation lives here
// since F2). `overnight` is decision 7's separate overnight scope: unattended
// runs started in the overnight window, with a $40-a-night hard stop. The
// night queue (master plan F1) checks it before it starts each item
// (`night_queue::night_budget`: its own turns are chat rows, added on top),
// and a due wake-up isn't sent past it (`wake_me::run_due`, morning M1);
// nothing else is stopped by it yet. Woken turns count as unattended. Known costs count; unknown costs are
// reported apart, never as $0.

/// The `main-agent` scope's key.
pub const MAIN_AGENT_SCOPE: &str = "main-agent";
/// The `overnight` scope's key.
#[cfg(test)]
pub const OVERNIGHT_SCOPE: &str = "overnight";
/// Decision 7 (2026-10-08): the overnight hard stop.
pub const OVERNIGHT_LIMIT_USD: f64 = 40.0;
/// The overnight window: 18:00 to 07:00 the next morning (local time),
/// for nights that start Monday to Friday evening.
pub const OVERNIGHT_START_HOUR: u32 = 18;
pub const OVERNIGHT_END_HOUR: u32 = 7;

/// One named scope's spend over its current window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSpend {
    pub scope: &'static str,
    /// RFC 3339; `None` outside the scope's window (daytime, for overnight).
    pub from: Option<String>,
    pub to: Option<String>,
    pub spent_usd: f64,
    /// Started runs with no cost (unpriced), never counted as $0.
    pub unknown_runs: u32,
    pub limit_usd: f64,
    /// The hard stop is reached.
    pub over: bool,
}

/// What the Main agent spent today on its own ([`main_agent_split`]), with
/// consultations and helpers: what the daily cap stops. Unknown costs add
/// nothing.
pub(crate) fn main_agent_spent(vault: &Path, now: DateTime<Utc>) -> f64 {
    main_agent_split(vault, now).0
}

/// The Main agent's day as `(on its own, asked for by the person)`
/// (overnight 10-11 A1). On its own: its wake and check-in turns (their
/// message is Hollow's), chats such turns started, swarm parts and pressure
/// tests; the cap stops these. Asked for: the person's own turns and the
/// chats those started, shown apart and never capped (on 10-10 two lanes the
/// person asked for spent past $100 and silenced every wake after 17:51).
pub(crate) fn main_agent_split(vault: &Path, now: DateTime<Utc>) -> (f64, f64) {
    use crate::main_agent::{budget_children, day_start, main_cwd, swarm_rows, WAKE_PREFIX};
    let start = day_start(now);
    // Pressure tests spend within the Main agent's day (master plan #8),
    // whether or not its workspace exists yet.
    let pressure = crate::stakeholder_sim::spent_since(vault, start);
    let Some(cwd) = main_cwd(vault) else {
        return (pressure, 0.0);
    };
    let swarm = swarm_rows(vault);
    let (mut on_its_own, mut person) = (pressure, 0.0);
    let mut counted = std::collections::HashSet::new();
    let since = crate::session_store::db_time(start);
    // Older roots first: an ancestor owns classification if a child in the
    // Main folder itself starts another child. Retain pre-midnight roots;
    // filter each ledger row by its own start, not the root's request day.
    for c in budget_children(vault, now).iter().rev() {
        // Complete receipt cost is intentionally null while a successor
        // runs. Charge recorded rows as they arrive, without changing that
        // nullable UI contract or counting a shared run twice.
        let cost = crate::run_queue::get(vault, c.queue_id)
            .ok()
            .flatten()
            .filter(|r| {
                r.source == crate::run_queue::RunSource::Chat
                    && r.source_id.as_deref() == Some(c.child_session.as_str())
            })
            .and_then(|r| r.history_run_id)
            .map_or(0.0, |run| {
                crate::descendants::recorded_chain_cost(vault, &run, &since, &mut counted)
            });
        if c.by_hollow || swarm.contains(&c.queue_id) {
            on_its_own += cost;
        } else {
            person += cost;
        }
    }
    // Main-folder descendants have already been assigned to their root.
    // Exclude them from the folder aggregate to avoid both double counting
    // and classifying an agent-authored child prompt as a person's request.
    let (own, asked) =
        crate::session_store::cost_for_cwd_since_split(vault, &cwd, start, WAKE_PREFIX, &counted)
            .unwrap_or((0.0, 0.0));
    on_its_own += own;
    person += asked;
    (on_its_own, person)
}

/// The `main-agent` scope against its daily `limit_usd`.
pub(crate) fn main_agent_spend(vault: &Path, now: DateTime<Utc>, limit_usd: f64) -> ScopeSpend {
    let spent = main_agent_spent(vault, now);
    let start = crate::main_agent::day_start(now);
    ScopeSpend {
        scope: MAIN_AGENT_SCOPE,
        from: Some(start.to_rfc3339_opts(SecondsFormat::Secs, true)),
        to: Some(now.to_rfc3339_opts(SecondsFormat::Secs, true)),
        spent_usd: spent,
        unknown_runs: 0,
        limit_usd,
        over: spent >= limit_usd,
    }
}

/// The overnight window `now` falls in, if any: `[18:00, 07:00 next day)`
/// local time, for nights starting Monday to Friday.
pub fn overnight_window(now: DateTime<Local>) -> Option<(DateTime<Local>, DateTime<Local>)> {
    use chrono::{Datelike, Duration as Days, Timelike, Weekday};
    let night = if now.hour() >= OVERNIGHT_START_HOUR {
        now.date_naive()
    } else if now.hour() < OVERNIGHT_END_HOUR {
        now.date_naive() - Days::days(1)
    } else {
        return None;
    };
    if matches!(night.weekday(), Weekday::Sat | Weekday::Sun) {
        return None;
    }
    let at = |date: chrono::NaiveDate, hour: u32| {
        Local
            .from_local_datetime(&date.and_hms_opt(hour, 0, 0)?)
            .earliest()
    };
    Some((
        at(night, OVERNIGHT_START_HOUR)?,
        at(night + Days::days(1), OVERNIGHT_END_HOUR)?,
    ))
}

/// Whether a ledger row counts toward the overnight scope: unattended runs
/// only (automations, the heartbeat, and teammates under either). A person's
/// own evening chat never uses up the night's budget.
fn overnight_row(surface: &str, parent_surface: Option<&str>) -> bool {
    let unattended = |s: &str| s == Surface::Workflow.as_str() || s == Surface::Heartbeat.as_str();
    unattended(surface)
        || (surface == Surface::Delegate.as_str() && parent_surface.is_some_and(unattended))
}

/// What unattended runs (automations, the heartbeat and their teammates,
/// and the turns `sessions.wake_me` woke, with their helpers) started in
/// `[from, to)` spent: known cost, and runs with no cost.
pub fn unattended_spend(
    vault: &Path,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<(f64, u32), String> {
    let (from, to) = (
        crate::session_store::db_time(from),
        crate::session_store::db_time(to),
    );
    let rows = crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT surface, parent_surface, cost_usd, spawned, run_id, billing FROM usage_ledger \
                 WHERE started_at >= ?1 AND started_at < ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![from, to], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)? == crate::usage::Billing::SelfHosted.as_str(),
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    })?;
    // A woken turn is a chat row, but nobody is watching it (morning M1).
    let woken = crate::wake_me::woken_runs(vault);
    let (mut spent, mut unknown) = (0.0, 0u32);
    for (surface, parent, cost, spawned, run, gpu_time) in rows {
        let wake = run.as_deref().is_some_and(|r| woken.contains(r));
        if !wake && !overnight_row(&surface, parent.as_deref()) {
            continue;
        }
        match cost {
            Some(cost) => spent += cost,
            // Q8: GPU time (a self-hosted model) is no spend, not unknown.
            None if spawned && !gpu_time => unknown += 1,
            None => {}
        }
    }
    Ok((spent, unknown))
}

/// The `overnight` scope now: what unattended runs started in the current
/// window spent, against [`OVERNIGHT_LIMIT_USD`]. Outside the window nothing
/// is spent and nothing is over.
#[cfg(test)]
pub fn overnight_spend(vault: &Path, now: DateTime<Local>) -> Result<ScopeSpend, String> {
    let mut spend = ScopeSpend {
        scope: OVERNIGHT_SCOPE,
        from: None,
        to: None,
        spent_usd: 0.0,
        unknown_runs: 0,
        limit_usd: OVERNIGHT_LIMIT_USD,
        over: false,
    };
    let Some((start, end)) = overnight_window(now) else {
        return Ok(spend);
    };
    let (spent, unknown) =
        unattended_spend(vault, start.with_timezone(&Utc), end.with_timezone(&Utc))?;
    spend.spent_usd += spent;
    spend.unknown_runs += unknown;
    // Master plan #1: the night queue's own turns (chat rows) count too, so
    // automations and the queue share one $40.
    let night = start.format("%Y-%m-%d").to_string();
    let (queued, unknown) = crate::night_queue::spent_tonight(vault, &night)?;
    spend.spent_usd += queued;
    spend.unknown_runs += unknown;
    spend.from = Some(start.to_rfc3339_opts(SecondsFormat::Secs, false));
    spend.to = Some(end.to_rfc3339_opts(SecondsFormat::Secs, false));
    spend.over = spend.spent_usd >= OVERNIGHT_LIMIT_USD;
    Ok(spend)
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use crate::session_store::tests::usage_row_fixture;
    use chrono::Timelike;

    fn local(y: i32, m: u32, d: u32, h: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, 30, 0).single().unwrap()
    }

    #[test]
    fn the_overnight_window_is_weeknights_six_to_seven() {
        // 2026-10-05 is a Monday.
        assert!(
            overnight_window(local(2026, 10, 5, 12)).is_none(),
            "daytime"
        );
        let (start, end) = overnight_window(local(2026, 10, 5, 19)).unwrap();
        assert_eq!((start.hour(), end.hour()), (18, 7));
        assert_eq!(end.date_naive(), local(2026, 10, 6, 0).date_naive());
        // 03:30 Tuesday belongs to Monday's night.
        assert_eq!(overnight_window(local(2026, 10, 6, 3)).unwrap().0, start);
        // Friday night counts; Saturday and Sunday nights don't.
        assert!(overnight_window(local(2026, 10, 9, 22)).is_some());
        assert!(overnight_window(local(2026, 10, 10, 22)).is_none());
        assert!(overnight_window(local(2026, 10, 11, 22)).is_none());
        assert!(
            overnight_window(local(2026, 10, 12, 6)).is_none(),
            "Sunday night"
        );
    }

    #[test]
    fn overnight_counts_unattended_runs_only_and_stops_at_forty_dollars() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let night = local(2026, 10, 5, 20);
        let at = |h: u32| crate::session_store::db_time(local(2026, 10, 5, h).with_timezone(&Utc));
        let add = |key: &str, h: u32, surface: &str, parent: Option<&str>, cost: Option<f64>| {
            let mut r = usage_row_fixture(key, &at(h), "success");
            r.started_at = at(h);
            r.surface = surface.into();
            r.parent_surface = parent.map(str::to_string);
            r.cost_usd = cost;
            r.spawned = true;
            crate::session_store::upsert_usage(vault, &r).unwrap();
        };
        add("w1", 18, "workflow", None, Some(25.0));
        add("d1", 19, "delegate", Some("workflow"), Some(10.0));
        add("c1", 19, "chat", None, Some(30.0)); // the person's own chat
        add("w0", 17, "workflow", None, Some(30.0)); // before the window
        add("h1", 19, "heartbeat", None, None); // unpriced
        let spend = overnight_spend(vault, night).unwrap();
        assert_eq!(spend.spent_usd, 35.0);
        assert_eq!(spend.unknown_runs, 1, "unknown, never $0");
        assert!(!spend.over);
        add("w2", 20, "workflow", None, Some(5.0));
        assert!(overnight_spend(vault, night).unwrap().over, "$40 hard stop");
        // Daytime: no window.
        let day = overnight_spend(vault, local(2026, 10, 6, 12)).unwrap();
        assert!(!day.over && day.from.is_none());
    }
}

#[cfg(test)]
#[path = "budgets_tests.rs"]
mod tests;
