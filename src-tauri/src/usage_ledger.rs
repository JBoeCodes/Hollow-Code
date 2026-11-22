//! The usage ledger: exactly one row per agent invocation, from every launch
//! point (Paperclip PR 6).
//!
//! Rows live in the configured vault's session store
//! (`session_store::usage_ledger`, outside the vault). Every owner of an
//! invocation holds a [`UsageRun`]:
//! - `agent::run_collect` / `collect_command` (one-shots, the heartbeat's
//!   Claude path, team-mcp delegations), `codex_transform::run` (Codex
//!   one-shots), `chat::execute_turn`, `workflows::execute` and ingest's Claude path.
//! - [`UsageRun::begin`] upserts a `running` row keyed by a fresh `run_key`;
//!   [`UsageRun::observe`] folds stream events in and writes a throttled
//!   checkpoint (tokens so far, `partial`) at most every
//!   [`CHECKPOINT_EVERY`]; [`UsageRun::finish`] upserts the final row once.
//!   Dropped unfinished, it records `abandoned` (or `cancelled` while the
//!   process is terminating). The `run_key` UPSERT makes a double row
//!   impossible.
//! - A run whose process died without destructors (a GUI crash, a SIGKILLed
//!   team-mcp) is closed as `interrupted` by [`reconcile_interrupted`] at
//!   startup or by [`close_orphaned_delegates`] when its parent finishes,
//!   keeping any checkpointed tokens as `partial`.
//!
//! **Never fails the run.** `begin`, the setters, `observe` and `finish`
//! return no error the caller could propagate and never unwrap: a failed
//! write is logged and journalled to `health` (subsystem `usage`). This keeps
//! them safe while a run holds its `RunLease` up to `finalize`, where a `?`
//! would drop the lease early.
//!
//! **Lock order.** The ledger takes only `session_store`'s leaf `WRITE_LOCK`
//! (plus SQLite's own lock). Callers finish their row BEFORE
//! `automated_history::finish_run`, and never while holding the history lock,
//! the permission write guard, the review queue lock or `run_guard`'s tracker.
//!
//! **Which store.** Leased surfaces pass their vault ([`Meter::for_vault`]);
//! detached GUI one-shots resolve the root [`set_root`] records at startup
//! ([`Meter::oneshot`]); delegates read `HOLLOW_VAULT_ROOT` from the env
//! team-mcp was spawned with ([`Meter::delegate_from_env`]), never
//! `HOLLOW_VAULT`, which is the run's cwd. No root means no row, logged once.

use crate::agent::{AgentEvent, AgentKind};
use crate::session_store::{self, db_time, UsageRow};
use crate::types::{UsageBucket, UsageDay, UsageSummary};
use crate::usage::{
    self, Basis, Billing, CallStats, CostStatus, LedgerOutcome, Resolved, TerminalRecord,
    TokenCounts, UsageAccumulator, WorkflowAgentUsage,
};
use chrono::{
    DateTime, Datelike, Duration, Local, Months, NaiveDateTime, NaiveTime, Offset, TimeZone, Utc,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// Where a run was launched from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Chat,
    Workflow,
    Heartbeat,
    Oneshot,
    Delegate,
    Ingest,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Chat => "chat",
            Surface::Workflow => "workflow",
            Surface::Heartbeat => "heartbeat",
            Surface::Oneshot => "oneshot",
            Surface::Delegate => "delegate",
            Surface::Ingest => "ingest",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "chat" => Some(Surface::Chat),
            "workflow" => Some(Surface::Workflow),
            "heartbeat" => Some(Surface::Heartbeat),
            "oneshot" => Some(Surface::Oneshot),
            "delegate" => Some(Surface::Delegate),
            "ingest" => Some(Surface::Ingest),
            _ => None,
        }
    }

    /// Settings, Usage label.
    pub fn label(self) -> &'static str {
        match self {
            Surface::Chat => "Chat",
            Surface::Workflow => "Automations",
            Surface::Heartbeat => "Heartbeat",
            Surface::Oneshot => "Assistants",
            Surface::Delegate => "Teammates",
            Surface::Ingest => "Imports",
        }
    }
}

/// The env team-mcp is spawned with (`team_mcp::delegate_env`), read back by
/// the delegate's meter. None of these names contain KEY, SECRET or TOKEN, so
/// Codex's environment filtering passes them through.
pub const ENV_VAULT_ROOT: &str = "HOLLOW_VAULT_ROOT";
pub const ENV_RUN_SURFACE: &str = "HOLLOW_RUN_SURFACE";
pub const ENV_RUN_SCOPE: &str = "HOLLOW_RUN_SCOPE";
pub const ENV_RUN_ID: &str = "HOLLOW_RUN_ID";

/// A checkpoint is written at most this often, and only when the running
/// total changed.
pub const CHECKPOINT_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// `usage_json` cap: the raw counters go first, then everything but the
/// basis, the terminal and a span's fields (see `UsageRun::usage_json`).
const USAGE_JSON_MAX: usize = 4_096;

/// OB24: the most run keys a span row's `spanRuns` keeps, the newest (this
/// row's last); the rest are counted in `spanRunsOmitted`. 64 `u-…` keys are
/// about 2.2 KB, which leaves the terminal room under [`USAGE_JSON_MAX`].
const SPAN_RUNS_MAX: usize = 64;

/// Overnight 9 S31: the most workflow-agent transcripts one run's scan
/// opens, and the most bytes it reads across them; past either the bucket
/// says `truncated` (a floor).
const WORKFLOW_FILES_MAX: usize = 400;
const WORKFLOW_BYTES_MAX: u64 = 256 * 1024 * 1024;

static LEDGER_ROOT: OnceLock<PathBuf> = OnceLock::new();
static TERMINATING: AtomicBool = AtomicBool::new(false);
static NO_ROOT_LOGGED: AtomicBool = AtomicBool::new(false);

/// Record the configured vault as the store for detached one-shots. Called
/// once in setup; a second call is ignored (config is fixed for the process).
pub fn set_root(vault: &Path) {
    let _ = LEDGER_ROOT.set(vault.to_path_buf());
}

/// team-mcp's SIGTERM handler calls this before dropping its in-flight
/// delegation, so that run's Drop records `cancelled`, not `abandoned`.
pub fn note_process_terminating() {
    TERMINATING.store(true, Ordering::SeqCst);
}

/// The parent a team-mcp server's delegations are attributed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegateParent {
    pub surface: Surface,
    /// The chat session key or the workflow slug (bare, never prefixed).
    pub scope: String,
    /// The parent's history run id, used to close its orphaned delegates.
    pub run_id: String,
    /// The parent run's scratch folder (Paperclip PR 15a), passed to the
    /// team server as `HOLLOW_SCRATCH_DIR` (Codex forwards only the
    /// server's explicit environment); a delegate reuses it after
    /// `scratch::inherited` validates it. `None` when the parent has none.
    pub scratch: Option<PathBuf>,
    /// The parent's allowed and denied tools, which its teammates keep
    /// (`team_mcp::delegate_grant`, merged plan slice 1).
    pub tool_limits: crate::team_mcp::ToolLimits,
}

/// What a delegate process knows about its parent, from its environment.
/// Shared with PR 8's delegate preflight, so both read one encoding.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DelegateContext {
    pub vault_root: Option<PathBuf>,
    pub parent_surface: Option<Surface>,
    pub scope: Option<String>,
    pub run_id: Option<String>,
}

impl DelegateContext {
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let value = |key: &str| get(key).filter(|v| !v.trim().is_empty());
        Self {
            vault_root: value(ENV_VAULT_ROOT).map(PathBuf::from),
            parent_surface: value(ENV_RUN_SURFACE).and_then(|s| Surface::parse(&s)),
            scope: value(ENV_RUN_SCOPE),
            run_id: value(ENV_RUN_ID),
        }
    }
}

#[derive(Debug, Clone)]
enum MeterRoot {
    Vault(PathBuf),
    /// Resolved from [`set_root`] at write time.
    Configured,
    /// Nowhere to write (a delegate without `HOLLOW_VAULT_ROOT`).
    Missing,
}

/// Where a run's row goes and how it is attributed.
#[derive(Debug, Clone)]
pub struct Meter {
    root: MeterRoot,
    surface: Surface,
    scope: Option<String>,
    run_id: Option<String>,
    parent_surface: Option<Surface>,
    /// The run's claim-time budget (Paperclip PR 8b): `agent::collect_command`
    /// arms a budget guard from it. Not part of the row.
    budget: Option<crate::budgets::RunBudget>,
}

impl Meter {
    /// A leased surface: the row goes to `vault`'s store.
    pub fn for_vault(
        vault: &Path,
        surface: Surface,
        scope: Option<String>,
        run_id: Option<String>,
    ) -> Self {
        Self {
            root: MeterRoot::Vault(vault.to_path_buf()),
            surface,
            scope,
            run_id,
            parent_surface: None,
            budget: None,
        }
    }

    /// A detached GUI one-shot (`purpose` is its scope), recorded in the
    /// configured vault's store whatever its cwd.
    pub fn oneshot(purpose: &str) -> Self {
        Self {
            root: MeterRoot::Configured,
            surface: Surface::Oneshot,
            scope: Some(purpose.to_string()),
            run_id: None,
            parent_surface: None,
            budget: None,
        }
    }

    /// A detached one-shot run for run `run_id` (F10, D2: a chat turn's
    /// learning check), credited to it: its row carries that run id, so the
    /// run's cost roll-up (`descendants::rollup`) lists it as a helper.
    pub fn oneshot_for_run(purpose: &str, run_id: &str) -> Self {
        Self {
            run_id: Some(run_id.to_string()).filter(|id| !id.trim().is_empty()),
            ..Self::oneshot(purpose)
        }
    }

    /// A team-mcp delegation, from the server's environment. Only
    /// `HOLLOW_VAULT_ROOT` names the store; without it nothing is written.
    pub fn delegate_from_env(get: impl Fn(&str) -> Option<String>) -> Self {
        let context = DelegateContext::from_env(get);
        Self {
            root: context
                .vault_root
                .map(MeterRoot::Vault)
                .unwrap_or(MeterRoot::Missing),
            surface: Surface::Delegate,
            scope: context.scope,
            run_id: context.run_id,
            parent_surface: context.parent_surface,
            budget: None,
        }
    }

    /// Attach the run's budget (Paperclip PR 8b; cross-m2 §4 #2: one runner
    /// signature, the budget rides the meter). `agent::collect_command` then
    /// checks the run's running total against it after every change.
    pub fn with_budget(mut self, budget: crate::budgets::RunBudget) -> Self {
        self.budget = Some(budget);
        self
    }

    /// The budget [`Self::with_budget`] attached.
    pub fn budget(&self) -> Option<&crate::budgets::RunBudget> {
        self.budget.as_ref()
    }

    fn root(&self) -> Option<PathBuf> {
        match &self.root {
            MeterRoot::Vault(path) => Some(path.clone()),
            MeterRoot::Configured => LEDGER_ROOT.get().cloned(),
            MeterRoot::Missing => None,
        }
    }
}

/// What a finished run recorded, for the caller (the chat chip's `usage`
/// event, `Collected.usage`).
#[derive(Debug, Clone, PartialEq)]
pub struct RunUsage {
    pub tokens: Option<TokenCounts>,
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    pub cost_status: CostStatus,
    pub billing: Billing,
    pub outcome: LedgerOutcome,
    pub basis: Basis,
    /// How full the context window was after the run's last message
    /// (`UsageAccumulator::context_tokens`). Never in the ledger row; a
    /// chat turn stores it on its conversation's `contextSteps` row
    /// (`context_gauge::store_figure`, repo features R2b).
    pub context_tokens: Option<u64>,
    /// The window the agent reported with that figure (Codex, overnight 9
    /// A10); `None` when the window is inferred from the model instead.
    pub context_window: Option<u64>,
    /// The run's calls and peak context (OB05), as its `usage_json` stores
    /// them; the chat and workflow receipts carry them as `apiUsage`.
    pub calls: CallStats,
    /// A self-hosted run's endpoint name (Q8), shown with "billed by GPU
    /// time".
    pub endpoint: Option<String>,
}

impl RunUsage {
    /// The chat `usage` event payload (camelCase; missing values are null,
    /// never 0). The frontend's `costChip` renders it.
    pub fn turn_json(&self) -> Value {
        json!({
            "costUsd": self.cost_usd,
            "costStatus": self.cost_status.as_str(),
            "billing": self.billing.as_str(),
            "inputTokens": self.tokens.map(|t| t.input),
            "cachedInputTokens": self.tokens.map(|t| t.cached_input),
            "outputTokens": self.tokens.map(|t| t.output),
            "outcome": self.outcome.as_str(),
            "model": self.model,
            "basis": self.basis.as_str(),
            "contextTokens": self.context_tokens,
            "contextWindow": self.context_window,
            "endpoint": self.endpoint,
        })
    }
}

/// One invocation's row, from `begin` to `finish`. See the module doc.
#[must_use = "a UsageRun records its row when finished (or abandoned when dropped)"]
pub struct UsageRun {
    meter: Meter,
    key: String,
    agent: Option<AgentKind>,
    requested_model: Option<String>,
    billing: Billing,
    /// A self-hosted run's endpoint name (Q8).
    endpoint: Option<String>,
    /// The native session this run resumed, when it resumed one.
    resumed_from: Option<String>,
    /// The native session the stream announced.
    native_session: Option<String>,
    spawned: bool,
    not_started: bool,
    started_at: DateTime<Utc>,
    acc: UsageAccumulator,
    last_checkpoint: Option<(Instant, TokenCounts)>,
    note: Option<String>,
    finished: bool,
    /// The Claude account the command was given (`claude_accounts`), read
    /// back from its environment at launch; rate limits are credited to it.
    claude_account: Option<String>,
    codex_login: Option<crate::provider_accounts::limits::LoginStamp>,
    /// Turn guards (2026-10-09): what a guard estimated this run cost when
    /// it ended the run before the agent reported a cost. Written as the
    /// row's cost (marked partial) only when no reported cost exists.
    cost_estimate: Option<f64>,
    /// Volume figures the caller measured itself ([`Self::set_detail`]),
    /// written into the final row's `usage_json`.
    details: Map<String, Value>,
    /// Overnight 9 A10: the context figure the agent reported outside its
    /// stream (Codex's rollout: prompt tokens, window), for a run whose
    /// messages carry none ([`Self::set_context_figure`]).
    context_figure: Option<(u64, u64)>,
    /// Overnight 9 A13: the model the agent said it ran outside its stream
    /// (Codex's rollout `turn_context`), for a run that asked for none
    /// ([`Self::set_turn_model`]).
    turn_model: Option<String>,
    /// Overnight 9 S31: a launched Claude run's projects folder
    /// (`<CLAUDE_CONFIG_DIR or ~/.claude>/projects`) and working folder,
    /// where its Workflow-tool agents' transcripts live.
    claude_dirs: Option<(PathBuf, PathBuf)>,
    /// What [`workflow_agent_usage`] found when the run finished.
    workflow_agents: Option<WorkflowAgentUsage>,
}

impl UsageRun {
    /// Insert the `running` row. Infallible: a failed write is reported, and
    /// the run proceeds.
    pub fn begin(meter: Meter, agent: Option<AgentKind>, requested_model: Option<String>) -> Self {
        let run = Self {
            meter,
            key: format!(
                "u-{}-{:016x}",
                Utc::now().timestamp_millis(),
                rand::random::<u64>()
            ),
            agent,
            requested_model: requested_model.filter(|m| !m.trim().is_empty()),
            billing: Billing::Unknown,
            endpoint: None,
            resumed_from: None,
            native_session: None,
            spawned: false,
            not_started: false,
            started_at: Utc::now(),
            acc: UsageAccumulator::default(),
            last_checkpoint: None,
            note: None,
            claude_account: None,
            codex_login: None,
            cost_estimate: None,
            details: Map::new(),
            context_figure: None,
            turn_model: None,
            claude_dirs: None,
            workflow_agents: None,
            finished: false,
        };
        run.write_row(run.row(
            LedgerOutcome::Running,
            None,
            None,
            CostStatus::Unknown,
            Basis::PerRun,
            run.requested_model.clone(),
            None,
        ));
        run
    }

    /// A new row for another attempt of the same run (an automation
    /// restarted on another account, #16): the same meter, agent and model,
    /// its own key, so each attempt records its own account and usage.
    pub fn sibling(&self) -> UsageRun {
        UsageRun::begin(self.meter.clone(), self.agent, self.requested_model.clone())
    }

    /// The agent and model, once a caller that began early (a workflow
    /// before preparation) has resolved them.
    pub fn set_agent(&mut self, agent: AgentKind, model: Option<String>) {
        self.agent = Some(agent);
        self.requested_model = model.filter(|m| !m.trim().is_empty());
    }

    /// Q8: a self-hosted run's endpoint, as `(name, served model id)`. The
    /// name rides its row's `usage_json` and the turn's usage event (shown
    /// with "billed by GPU time"); the served id is the row's requested
    /// model, whatever the person picked. `None` (not a self-hosted run)
    /// leaves the run as it is, and so does a run [`Self::set_billing`]
    /// didn't classify as self-hosted (the build failed, and its `/bin/sh`
    /// stand-in carries no run key): call it after `set_billing`.
    pub fn set_self_hosted(&mut self, endpoint: Option<(&str, &str)>) {
        let Some((name, served)) = endpoint else {
            return;
        };
        if self.billing != Billing::SelfHosted {
            return;
        }
        self.endpoint = Some(name.to_string());
        if !served.trim().is_empty() {
            self.requested_model = Some(served.to_string());
        }
    }

    pub fn set_billing(&mut self, billing: Billing) {
        self.billing = billing;
    }

    /// How this run is billed (`usage::classify`, set per attempt).
    pub fn billing(&self) -> Billing {
        self.billing
    }

    /// The night guard's running estimate of what this run has spent so far
    /// (`usage_prices::estimate_usd` over its streamed messages, priced high
    /// on purpose). A self-hosted run (Q8) is billed by GPU time, never per
    /// token: it spends nothing a night budget counts, so 0 (never shown;
    /// its row stays unpriced).
    pub fn night_estimate(&self) -> f64 {
        if self.billing == Billing::SelfHosted {
            return 0.0;
        }
        crate::usage_prices::estimate_usd(&self.acc.message_by_model())
    }

    /// The native session id this run resumes (`None` for a fresh one).
    /// Turn guards: the run was ended at a safe point before the agent
    /// reported its cost; `usd` is the running estimate
    /// (`usage_prices::estimate_usd`), so the row is never cost-unknown.
    pub fn set_cost_estimate(&mut self, usd: f64) {
        self.cost_estimate = Some(usd).filter(|u| u.is_finite() && *u >= 0.0);
    }

    pub fn set_resumed(&mut self, session: Option<String>) {
        self.resumed_from = session.filter(|s| !s.trim().is_empty());
    }

    /// Overnight 9 A10: the run's latest context figure from outside its
    /// stream (`tokens` of `window`). Used only when no message reported
    /// usage, so a stream's own figure always wins.
    pub fn set_context_figure(&mut self, tokens: u64, window: u64) {
        self.context_figure = Some((tokens, window));
    }

    /// Overnight 9 A13: the model the agent reported running outside its
    /// stream (Codex's rollout). The row's model when no message named one
    /// and the run asked for none, so a default-model Codex run can be
    /// priced at the person's row for it.
    pub fn set_turn_model(&mut self, model: &str) {
        let model = crate::usage::normalize_model(model.trim());
        if !model.is_empty() {
            self.turn_model = Some(model);
        }
    }

    pub fn set_native_session(&mut self, session: &str) {
        if !session.trim().is_empty() {
            self.native_session = Some(session.to_string());
        }
    }

    /// Call right after a successful spawn: the build_command stand-in is
    /// spawned too, but is recorded as never started.
    pub fn launched(&mut self, cmd: &tokio::process::Command) {
        self.launched_attributed_to(cmd, cmd.as_std().get_current_dir());
    }

    fn launched_attributed_to(&mut self, cmd: &tokio::process::Command, cwd: Option<&Path>) {
        self.claude_account = crate::claude_accounts::account_of(cmd);
        if self.agent == Some(AgentKind::Codex) {
            self.codex_login = crate::provider_accounts::limits::for_command(cmd);
        }
        if crate::agent::is_unavailable_command(cmd) {
            self.not_started = true;
        } else {
            self.spawned = true;
            self.attribute(cwd);
            self.claude_dirs = claude_dirs_of(cmd);
        }
    }

    /// Run ledger v2 (master plan F2): write who paid for this launch and
    /// what it was for onto the row (`run_attribution`). A failover's new
    /// attempt is a new row, so each row names the account that ran it.
    /// Infallible like everything here.
    fn attribute(&mut self, cwd: Option<&Path>) {
        let Some(root) = self.meter.root() else {
            return;
        };
        let mut attribution = cwd
            .map(|cwd| crate::run_attribution::for_cwd(&root, cwd))
            .unwrap_or_default();
        attribution.account_ref = self.account_ref();
        if attribution.is_empty() {
            return;
        }
        if let Err(error) = session_store::set_usage_attribution(&root, &self.key, &attribution) {
            report_error(Some(root.as_path()), &error);
        }
    }

    /// The hashed label of the account this run used: the account the
    /// command was given, else (a teammate inside `hollow team-mcp`) the one
    /// its parent passed down, else Claude Code's own login. Only Claude runs
    /// have accounts; every other agent's is unknown.
    fn account_ref(&self) -> Option<String> {
        // A self-hosted run (Q8) uses no Claude account at all.
        if self.agent != Some(AgentKind::Claude) || self.billing == Billing::SelfHosted {
            return None;
        }
        let account = self
            .claude_account
            .clone()
            .or_else(|| std::env::var(crate::claude_accounts::ACCOUNT_ENV).ok())
            .unwrap_or_else(|| crate::claude_accounts::LOGIN_ID.to_string());
        Some(crate::run_attribution::account_ref(&account))
    }

    /// For runners whose model turn starts later than their first process
    /// (the Codex transform's version and catalog steps are not turns).
    pub fn mark_spawned(&mut self) {
        self.spawned = true;
    }

    /// Say why a run ended the way it did, on its row (at most 200
    /// characters): the Codex transform's version guard (F9, wave 4 · V1).
    pub fn set_note(&mut self, note: &str) {
        self.note = Some(crate::claude_runner::truncate(note, 200));
    }

    /// A volume figure the caller measured itself, for the final row's
    /// `usage_json` (S28: a Jev request's `use`, `items`, `requestBytes`,
    /// `responseBytes` and `latencyMs`, since TypeSafe reports no tokens).
    /// Numbers, booleans and fixed labels only, never request text. A key
    /// the row already writes (`basis`, `terminal`, the call figures) keeps
    /// the row's own value.
    pub fn set_detail(&mut self, key: &str, value: Value) {
        self.details.insert(key.to_string(), value);
    }

    /// Fold one stream event in; true when [`Self::running_total`] changed
    /// (the hook PR 8's budget check uses). Writes a throttled checkpoint.
    pub fn observe(&mut self, evt: &AgentEvent) -> bool {
        // Plan usage, credited to the account this run used (the login's
        // also feeds the Settings, Usage plan card). A no-op outside the
        // GUI, so inside `hollow team-mcp` too.
        if let AgentEvent::RateLimit(quota) = evt {
            crate::claude_accounts::observe_stream(self.claude_account.as_deref(), quota);
        }
        if let (AgentEvent::ProviderRateLimit(limits), Some(owner)) = (evt, &self.codex_login) {
            crate::provider_accounts::limits::observe(owner, limits);
        }
        let before = self.running_total();
        self.acc.observe(evt);
        let changed = self.running_total() != before;
        if changed {
            self.reserve();
            self.maybe_checkpoint();
        }
        changed
    }

    /// This run's ledger key (`run_key`): a budget guard binds it so its own
    /// reservation is never counted as another run's
    /// (`budgets::BudgetGuard::bind_run`).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Publish the running total to the shared in-flight reservation
    /// (`budgets::reserve`, merged plan slice 3), so every concurrent budget
    /// guard in this process sees it at once, not at the next checkpoint.
    fn reserve(&self) {
        let (Some(root), Some(tokens)) = (self.meter.root(), self.running_total()) else {
            return;
        };
        let run = crate::budgets::InFlightRun {
            surface: self.meter.surface.as_str().to_string(),
            scope: self.meter.scope.clone(),
            parent_surface: self.meter.parent_surface.map(|s| s.as_str().to_string()),
            agent: self.agent.map(|a| a.as_str().to_string()),
            tokens: tokens.input.saturating_add(tokens.output),
            gpu_time: self.billing == Billing::SelfHosted,
        };
        crate::budgets::reserve(&root, &self.key, run);
    }

    /// This run's own tokens so far. A resumed Claude turn's terminal is
    /// session-cumulative (it includes every earlier turn), and only
    /// [`usage::resolve`] at `finish` can reduce it to this turn's share, so
    /// until then such a run counts its own per-message sum. Checkpoints and
    /// PR 8's budget hook read this, never the raw accumulator total, so a
    /// row left `running` by a crash can never hold another turn's tokens.
    pub fn running_total(&self) -> Option<TokenCounts> {
        if self.resumed_from.is_some() && self.agent == Some(AgentKind::Claude) {
            self.acc.message_total()
        } else {
            self.acc.running_total()
        }
    }

    pub fn accumulator(&self) -> &UsageAccumulator {
        &self.acc
    }

    /// The budget its meter carries (Paperclip PR 8b's guard).
    pub fn budget(&self) -> Option<&crate::budgets::RunBudget> {
        self.meter.budget()
    }

    /// The agent this run resolved to, when known (PR 8's `after_run` builds
    /// the finished run's budget intent from it).
    /// The history run id this run is attributed to, when it has one.
    pub fn run_id(&self) -> Option<&str> {
        self.meter.run_id.as_deref()
    }

    pub fn agent(&self) -> Option<AgentKind> {
        self.agent
    }

    /// Write the final row once and return what it recorded.
    pub fn finish(self, outcome: LedgerOutcome) -> RunUsage {
        self.finish_receipt(outcome).0
    }

    /// Finalize once, with the exact final row's identity when it was
    /// persisted (`None` otherwise: an older readable checkpoint or a returned
    /// usage value can't prove the final write). The timestamp comes from the
    /// persisted row, never a later clock read.
    pub(crate) fn finish_receipt(
        mut self,
        outcome: LedgerOutcome,
    ) -> (RunUsage, Option<FinalUsageReceipt>) {
        let result = self.write_final(outcome);
        self.finished = true;
        result
    }

    fn maybe_checkpoint(&mut self) {
        let Some(tokens) = self.running_total() else {
            return;
        };
        let due = match self.last_checkpoint {
            None => true,
            Some((at, last)) => last != tokens && at.elapsed() >= CHECKPOINT_EVERY,
        };
        if !due {
            return;
        }
        self.last_checkpoint = Some((Instant::now(), tokens));
        let mut row = self.row(
            LedgerOutcome::Running,
            Some(tokens),
            None,
            CostStatus::Partial,
            Basis::PerRun,
            self.requested_model.clone(),
            None,
        );
        // OB07: a crash leaves this row to the reconcilers, which keep its
        // tokens as `interrupted`; it keeps their priced floor too.
        row.cost_floor_usd = self.cost_floor(None);
        self.write_row(row);
    }

    fn write_final(&mut self, outcome: LedgerOutcome) -> (RunUsage, Option<FinalUsageReceipt>) {
        let outcome = if self.not_started && outcome != LedgerOutcome::Cancelled {
            LedgerOutcome::SpawnFailed
        } else {
            outcome
        };
        let baseline = self.previous_terminal();
        let resolved = usage::resolve(
            &self.acc,
            self.agent,
            self.resumed_from.is_some(),
            baseline.as_ref().map(|(terminal, _)| terminal),
            !baseline
                .as_ref()
                .is_some_and(|(_, stored)| stored.skipped_priced),
        );
        let skipped = baseline
            .as_ref()
            .map(|(_, stored)| stored.skipped_runs.as_slice())
            .unwrap_or_default();
        let mut cost_status = usage::cost_status(self.billing, resolved.as_ref());
        let basis = resolved.as_ref().map(|r| r.basis).unwrap_or(Basis::PerRun);
        let tokens = resolved.as_ref().and_then(|r| r.tokens);
        let mut cost_usd = resolved.as_ref().and_then(|r| r.cost_usd);
        // Q8: a self-hosted run is billed by GPU time; whatever price the CLI
        // made up for an id it doesn't know is dropped, never shown.
        let self_hosted = self.billing == Billing::SelfHosted;
        if self_hosted {
            cost_usd = None;
        }
        if cost_usd.is_none() && self.cost_estimate.is_some() && !self_hosted {
            cost_usd = self.cost_estimate;
            cost_status = CostStatus::Partial;
        }
        let model = resolved
            .as_ref()
            .and_then(|r| r.models.first().cloned())
            .or_else(|| self.requested_model.clone())
            .or_else(|| self.turn_model.clone());
        // Antigravity's price rows are keyed by base model: a saved or
        // requested level variant (`gemini-4-neon-high`) prices as its base.
        let model = if self.agent == Some(AgentKind::Antigravity) {
            model.map(|m| crate::effort::agy_base(&m).to_string())
        } else {
            model
        };
        // Overnight 9 A13: Codex reports tokens, never a price. With the
        // person's price for its model the row is `estimated` at that price
        // (summed apart from reported costs); without one it stays unpriced.
        // Antigravity (2026-11-20) is the same: tokens from each step, a
        // price from Google's published Gemini rates in the table.
        if matches!(self.agent, Some(AgentKind::Codex | AgentKind::Antigravity))
            && cost_status == CostStatus::Unpriced
        {
            let priced = model
                .as_deref()
                .zip(tokens.as_ref())
                .and_then(|(model, tokens)| crate::usage_prices::list_usd(model, tokens));
            if let Some((usd, price)) = priced {
                cost_usd = Some(usd);
                cost_status = CostStatus::Estimated;
                self.details.insert(
                    "priceSource".into(),
                    json!(format!(
                        "{} ({}, as of {})",
                        price.model, price.source, price.as_of
                    )),
                );
            }
        }
        self.workflow_agents = self.scan_workflow_agents(tokens);
        let usage_json = self.usage_json(basis, resolved.as_ref(), skipped);
        let mut row = self.row(
            outcome,
            tokens,
            cost_usd,
            cost_status,
            basis,
            model.clone(),
            Some(usage_json),
        );
        // OB07: tokens but no cost (killed, interrupted, an unverified
        // resume) carry a priced floor beside the NULL cost, never in it.
        let uncosted = cost_usd.is_none() && cost_status == CostStatus::Partial;
        if uncosted && tokens.is_some() {
            row.cost_floor_usd = self.cost_floor(resolved.as_ref());
        }
        let receipt = FinalUsageReceipt {
            run_key: row.run_key.clone(),
            recorded_at: row.at.clone(),
        };
        let recorded = self.write_row(row).then_some(receipt);
        // Preserve the existing best-effort reservation lifecycle. Callers
        // requiring durable settlement must retain their separate claim when
        // `recorded` is absent; a readable earlier checkpoint is not success.
        if let Some(root) = self.meter.root() {
            let total = tokens.map(|t| t.input.saturating_add(t.output));
            crate::budgets::release(&root, &self.key, total);
        }
        let streamed = self.acc.context_tokens();
        (
            RunUsage {
                tokens,
                cost_usd,
                model,
                cost_status,
                billing: self.billing,
                outcome,
                basis,
                context_tokens: streamed.or(self.context_figure.map(|(tokens, _)| tokens)),
                context_window: streamed
                    .is_none()
                    .then_some(self.context_figure.map(|(_, window)| window))
                    .flatten(),
                calls: self.call_stats(),
                endpoint: self.endpoint.clone(),
            },
            recorded,
        )
    }

    /// OB07: the price table applied to what this run streamed, by model
    /// (`usage_prices::floor_usd`). Its per-message sums are this run's own
    /// even on a resume, and never more than its true total (side models
    /// don't stream). Messages without a model id (grouped as `unknown`,
    /// which has no price row) count at the row's model: its first known
    /// model, else the requested one; that can over-attribute only when a
    /// stream omits model ids and a cheaper model actually answered. With no
    /// messages, the row's own tokens at its one model.
    fn cost_floor(&self, resolved: Option<&Resolved>) -> Option<f64> {
        let mut messages = self.acc.message_by_model();
        let row_model = resolved
            .and_then(|r| r.models.iter().find(|m| m.as_str() != "unknown").cloned())
            .or_else(|| self.requested_model.clone());
        if let Some(model) = row_model {
            if let Some(unknown) = messages.remove("unknown") {
                let slot = messages.entry(model).or_default();
                *slot = slot.add(unknown);
            }
        }
        if !messages.is_empty() {
            return crate::usage_prices::floor_usd(&messages);
        }
        match resolved.map(|r| (r.models.as_slice(), r.tokens)) {
            Some(([model], Some(tokens))) => crate::usage_prices::floor_usd([(model, &tokens)]),
            _ => None,
        }
    }

    /// The accumulator's [`CallStats`] (`numTurns` and `apiMs` summed over
    /// the run's terminals), with Claude's `apiMs` corrected (OB05). Claude's
    /// `duration_api_ms` is session-cumulative on `--resume`: a resumed
    /// Claude turn drops it, since no per-turn figure can be derived from
    /// it, and a Claude row that started its session fresh keeps the last
    /// terminal's, which already covers any earlier result on the row (a
    /// guard: in chat a ceiling move's first attempt ends before any result
    /// and a limit failover starts a new row, so two results on one row are
    /// rare), so summing would count it twice. Grok's is per invocation
    /// even on `--resume` (`fixtures/grok/resume.jsonl`), so its sum stands,
    /// resumed or not. The live gauge reads it too (overnight 10 A8,
    /// `GaugeGuard::publish_run`).
    pub(crate) fn call_stats(&self) -> CallStats {
        let mut stats = self.acc.call_stats();
        if self.agent == Some(AgentKind::Claude) {
            stats.api_ms = if self.resumed_from.is_some() {
                None
            } else {
                self.acc.last_api_ms()
            };
        }
        stats.workflow_agents = self.workflow_agents.clone();
        stats
    }

    /// Overnight 9 S31: a finished Claude run's Workflow-tool agents, from
    /// their transcripts' messages inside the run's window, with
    /// `unmeteredTokens` against the row's `tokens`. `None` for any other
    /// agent, a run that never launched or announced no session, and a run
    /// whose session has no workflow agent in its window.
    fn scan_workflow_agents(&self, tokens: Option<TokenCounts>) -> Option<WorkflowAgentUsage> {
        if self.agent != Some(AgentKind::Claude) {
            return None;
        }
        let (projects, cwd) = self.claude_dirs.as_ref()?;
        let session = self
            .native_session
            .as_deref()
            .or(self.resumed_from.as_deref())?;
        let until = Utc::now();
        let streamed = |id: &str| self.acc.has_message(id);
        // The scan reads up to `WORKFLOW_BYTES_MAX`: `finish` runs on an
        // async worker in a chat turn, so tell the runtime it blocks (a
        // current-thread runtime, in tests, can't be told), as the receipt's
        // cite check does.
        let scan =
            || workflow_agent_usage(projects, cwd, session, self.started_at, until, &streamed);
        let (mut found, unstreamed) =
            match tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()) {
                Ok(tokio::runtime::RuntimeFlavor::MultiThread) => tokio::task::block_in_place(scan),
                _ => scan(),
            }?;
        // What the row can't hold: the workflow messages the stream didn't
        // also carry, plus everything streamed, less the row's tokens.
        let streamed_total = self
            .acc
            .message_by_model()
            .values()
            .fold(0u64, |sum, t| sum.saturating_add(t.total()));
        let unmetered = unstreamed
            .saturating_add(streamed_total)
            .saturating_sub(tokens.map(|t| t.total()).unwrap_or(0));
        found.unmetered_tokens = (unmetered > 0).then_some(unmetered);
        Some(found)
    }

    /// The newest stored terminal of the session a resumed Claude run
    /// continues, the baseline `usage::resolve` subtracts (Claude's resume
    /// totals are session-cumulative), and the stored row: the run keys of
    /// the rows after it that stored none (OB24's `spanRuns`), and whether
    /// any of them has a cost.
    fn previous_terminal(&self) -> Option<(TerminalRecord, session_store::UsageBaseline)> {
        if self.agent != Some(AgentKind::Claude) || self.acc.terminal().is_none() {
            return None;
        }
        let session = self.resumed_from.as_deref()?;
        let root = self.meter.root()?;
        match session_store::previous_terminal(
            &root,
            AgentKind::Claude.as_str(),
            session,
            &db_time(Utc::now()),
            &self.key,
        ) {
            Ok(stored) => {
                let stored = stored?;
                // A terminal that doesn't deserialize (a format from another
                // version) means no baseline: the run is `resume_unverified`.
                // No older row is tried; its delta would span this row too.
                let terminal = serde_json::from_str::<Value>(&stored.usage_json)
                    .ok()
                    .and_then(|doc| doc.get("terminal").cloned())
                    .and_then(|terminal| serde_json::from_value(terminal).ok())?;
                Some((terminal, stored))
            }
            Err(error) => {
                report_error(Some(root.as_path()), &error);
                None
            }
        }
    }

    /// OB24: a span's cost covers the rows its baseline skipped and this
    /// one, oldest first; [`partial_floor`] leaves their floors out. Past
    /// [`SPAN_RUNS_MAX`] only the newest keys are kept and the rest counted
    /// in `spanRunsOmitted`: those rows' floors stay in `partialFloorUsd`,
    /// beside the span that covers them.
    fn span_fields(&self, resolved: Option<&Resolved>, skipped: &[String]) -> Map<String, Value> {
        let mut fields = Map::new();
        let Some(r) = resolved.filter(|r| r.basis == Basis::SessionDeltaSpan) else {
            return fields;
        };
        let mut runs = skipped.to_vec();
        runs.push(self.key.clone());
        let omitted = runs.len().saturating_sub(SPAN_RUNS_MAX);
        if omitted > 0 {
            runs.drain(..omitted);
            fields.insert("spanRunsOmitted".into(), json!(omitted));
        }
        fields.insert("spanRuns".into(), json!(runs));
        fields.insert("unstreamedTokens".into(), json!(r.unstreamed_tokens));
        fields
    }

    fn usage_json(&self, basis: Basis, resolved: Option<&Resolved>, skipped: &[String]) -> String {
        let mut doc = Map::new();
        doc.insert("basis".into(), json!(basis.as_str()));
        doc.insert("resumed".into(), json!(self.resumed_from.is_some()));
        doc.insert(
            "models".into(),
            json!(resolved.map(|r| r.models.clone()).unwrap_or_default()),
        );
        doc.insert("terminals".into(), json!(self.acc.terminals()));
        let terminal = self
            .acc
            .terminal()
            .map(TerminalRecord::from)
            .and_then(|record| serde_json::to_value(record).ok())
            .unwrap_or(Value::Null);
        doc.insert("terminal".into(), terminal.clone());
        if let Some(note) = &self.note {
            doc.insert("note".into(), json!(note));
        }
        if let Some(endpoint) = &self.endpoint {
            doc.insert("endpoint".into(), json!(endpoint));
        }
        let span = self.span_fields(resolved, skipped);
        doc.extend(span.clone());
        // OB05: apiCalls, peakContextTokens, cacheWriteTokens, apiMs,
        // numTurns and subagents, each only when the agent supplied it.
        if let Ok(Value::Object(calls)) = serde_json::to_value(self.call_stats()) {
            doc.extend(calls);
        }
        for (key, value) in &self.details {
            doc.entry(key.clone()).or_insert_with(|| value.clone());
        }
        doc.insert(
            "raw".into(),
            self.acc.last_raw().cloned().unwrap_or(Value::Null),
        );
        let text = Value::Object(doc.clone()).to_string();
        if text.len() <= USAGE_JSON_MAX {
            return text;
        }
        doc.remove("raw");
        let text = Value::Object(doc).to_string();
        if text.len() <= USAGE_JSON_MAX {
            return text;
        }
        // The terminal and the span's fields survive the cap: the next
        // resume subtracts the terminal (without it, that resume would pass
        // over this row and its span would count this row's cost again), and
        // the summary reads the span's fields.
        let mut cut = span;
        cut.insert("basis".into(), json!(basis.as_str()));
        cut.insert("truncated".into(), json!(true));
        // S31: the workflow bucket is small and nowhere else.
        if let Some(found) = self
            .workflow_agents
            .as_ref()
            .and_then(|w| serde_json::to_value(w).ok())
        {
            cut.insert("workflowAgents".into(), found);
        }
        cut.insert("terminal".into(), terminal);
        let text = Value::Object(cut.clone()).to_string();
        if text.len() <= USAGE_JSON_MAX {
            return text;
        }
        // Too large even so (a terminal of many models): the run keys go
        // first, all counted as omitted (their floors stay in), then the
        // terminal. A row with a cost and no terminal is never spanned over:
        // the next resume sees a priced row in between and records no span.
        if cut.remove("spanRuns").is_some() {
            cut.insert("spanRunsOmitted".into(), json!(skipped.len() + 1));
            let text = Value::Object(cut.clone()).to_string();
            if text.len() <= USAGE_JSON_MAX {
                return text;
            }
        }
        cut.remove("terminal");
        Value::Object(cut).to_string()
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        outcome: LedgerOutcome,
        tokens: Option<TokenCounts>,
        cost_usd: Option<f64>,
        cost_status: CostStatus,
        basis: Basis,
        model: Option<String>,
        usage_json: Option<String>,
    ) -> UsageRow {
        let count = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
        UsageRow {
            run_key: self.key.clone(),
            at: db_time(Utc::now()),
            started_at: db_time(self.started_at),
            surface: self.meter.surface.as_str().to_string(),
            scope: self.meter.scope.clone(),
            parent_surface: self.meter.parent_surface.map(|s| s.as_str().to_string()),
            run_id: self.meter.run_id.clone(),
            agent: self.agent.map(|a| a.as_str().to_string()),
            requested_model: self.requested_model.clone(),
            model,
            native_session: self
                .native_session
                .clone()
                .or_else(|| self.resumed_from.clone()),
            billing: self.billing.as_str().to_string(),
            outcome: outcome.as_str().to_string(),
            spawned: self.spawned,
            basis: basis.as_str().to_string(),
            input_tokens: tokens.map(|t| count(t.input)),
            cached_input_tokens: tokens.map(|t| count(t.cached_input)),
            output_tokens: tokens.map(|t| count(t.output)),
            cost_usd,
            cost_status: cost_status.as_str().to_string(),
            usage_json,
            cost_floor_usd: None,
            workspace_id: None,
            account_ref: None,
        }
    }

    fn write_row(&self, row: UsageRow) -> bool {
        let Some(root) = self.meter.root() else {
            if !NO_ROOT_LOGGED.swap(true, Ordering::SeqCst) {
                eprintln!(
                    "[usage] no vault root for {} runs; usage is not recorded",
                    self.meter.surface.as_str()
                );
            }
            return false;
        };
        match session_store::upsert_usage(&root, &row) {
            Ok(()) => true,
            Err(error) => {
                report_error(Some(root.as_path()), &error);
                false
            }
        }
    }
}

/// Internal acknowledgement of the exact final usage row. This is a local
/// accounting receipt, not provider quota or confirmed child-process cleanup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinalUsageReceipt {
    pub(crate) run_key: String,
    pub(crate) recorded_at: String,
}

impl Drop for UsageRun {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            let _ = self.write_final(usage::drop_outcome(TERMINATING.load(Ordering::SeqCst)));
        }
    }
}

/// Log a ledger failure and journal it (redacted, one line) so Settings,
/// Usage can say recording is failing. Never `let _ =`.
fn report_error(root: Option<&Path>, error: &str) {
    let line = crate::claude_runner::truncate(&crate::claude_runner::redact(error), 160);
    eprintln!("[usage] {line}");
    if let Some(root) = root {
        crate::health::record(root, "usage", crate::health::Outcome::Failed, line);
    }
}

/// Overnight 9 S31: where a launched command's Claude Code keeps its
/// transcripts (`<config>/projects`, the config folder from the command's
/// own `CLAUDE_CONFIG_DIR`, else this process's, else `~/.claude`) and the
/// folder it runs in. `None` without a working folder or a home.
fn claude_dirs_of(cmd: &tokio::process::Command) -> Option<(PathBuf, PathBuf)> {
    let std = cmd.as_std();
    let cwd = std.get_current_dir()?.to_path_buf();
    let env = usage::claude_env::CONFIG_DIR_ENV;
    let configured = match std
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new(env))
    {
        Some((_, value)) => value.map(PathBuf::from),
        None => std::env::var_os(env).map(PathBuf::from),
    }
    .filter(|dir| !dir.as_os_str().is_empty());
    let home = dirs::home_dir();
    let config = usage::claude_env::config_dir(configured.as_deref(), home.as_deref())?;
    Some((config.join("projects"), cwd))
}

/// One transcript line, as far as the workflow scan looks: the timestamp
/// and the message's id, model and usage block. Everything else in the line
/// (the message's content, tool inputs and results) is skipped by serde
/// unread into any value.
#[derive(serde::Deserialize)]
struct WorkflowLine {
    timestamp: Option<String>,
    message: Option<WorkflowMessage>,
}

#[derive(serde::Deserialize)]
struct WorkflowMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Value>,
}

/// The `{id, model, usage}` object `usage::claude_message_usage` reads, and
/// nothing else of the line.
fn workflow_holder(message: WorkflowMessage) -> Value {
    json!({ "id": message.id, "model": message.model, "usage": message.usage })
}

/// Overnight 9 S31: the Workflow tool's agents of Claude session `session`
/// run in `cwd`, from `<projects>/<slug of cwd>/<session>/subagents/
/// workflows/<wf>/*.jsonl`: the usage blocks of their messages stamped
/// inside `since..=until`, one figure per message id (a message's repeated
/// snapshots max-merge, like the stream's). Usage blocks only: a line's
/// content is never kept. Also returns the tokens of the messages whose id
/// `streamed` doesn't know (the run's own stream didn't carry them). `None`
/// when no agent has a message in the window. `unmetered_tokens` is the
/// caller's. The folder is `run_manifest::claude_project_slug`'s, which
/// doesn't follow Claude Code's shortening of a name over 200 characters
/// (no folder here comes close).
pub(crate) fn workflow_agent_usage(
    projects: &Path,
    cwd: &Path,
    session: &str,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
    streamed: &dyn Fn(&str) -> bool,
) -> Option<(WorkflowAgentUsage, u64)> {
    use std::io::BufRead;
    let safe = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    if !safe(session) {
        return None;
    }
    // Claude Code names the folder by the real path it runs in.
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let root = projects
        .join(crate::run_manifest::claude_project_slug(&cwd))
        .join(session)
        .join("subagents")
        .join("workflows");
    let mut files = Vec::new();
    for workflow in std::fs::read_dir(&root).ok()?.flatten() {
        let Ok(entries) = std::fs::read_dir(workflow.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // A file last written before the run began holds nothing in it.
            let fresh = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|at| DateTime::<Utc>::from(at) >= since)
                .unwrap_or(true);
            if fresh {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut truncated = files.len() > WORKFLOW_FILES_MAX;
    files.truncate(WORKFLOW_FILES_MAX);
    // id → (model, tokens, cache writes).
    let mut messages: HashMap<String, (Option<String>, TokenCounts, u64)> = HashMap::new();
    let mut agents = 0u64;
    let mut read = 0u64;
    let mut line = Vec::new();
    for path in files {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut reader = std::io::BufReader::new(file);
        let mut counted = false;
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(n) => read = read.saturating_add(n as u64),
            }
            if read > WORKFLOW_BYTES_MAX {
                truncated = true;
                break;
            }
            // Cheap pre-check before parsing: only lines with a usage block.
            if !line.windows(7).any(|w| w == b"\"usage\"") {
                continue;
            }
            let Ok(parsed) = serde_json::from_slice::<WorkflowLine>(&line) else {
                continue;
            };
            let in_window = parsed
                .timestamp
                .as_deref()
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc))
                .is_some_and(|t| t >= since && t <= until);
            let Some(message) = parsed.message.filter(|_| in_window) else {
                continue;
            };
            let holder = workflow_holder(message);
            let Some((id, model, tokens)) = usage::claude_message_usage(&holder) else {
                continue;
            };
            let writes = usage::claude_cache_write(&holder);
            let slot = messages
                .entry(id)
                .or_insert((None, TokenCounts::default(), 0));
            if slot.0.is_none() {
                slot.0 = model;
            }
            slot.1 = slot.1.max_merge(tokens);
            slot.2 = slot.2.max(writes);
            counted = true;
        }
        agents += u64::from(counted);
        if read > WORKFLOW_BYTES_MAX {
            break;
        }
    }
    if messages.is_empty() {
        return None;
    }
    let mut sum = TokenCounts::default();
    let mut writes = 0u64;
    let mut unstreamed = 0u64;
    // model → (tokens, cache writes).
    let mut by_model: BTreeMap<String, (TokenCounts, u64)> = BTreeMap::new();
    for (id, (model, tokens, cache_write)) in &messages {
        sum = sum.add(*tokens);
        writes = writes.saturating_add(*cache_write);
        if !streamed(id) {
            unstreamed = unstreamed.saturating_add(tokens.total());
        }
        if let Some(model) = model {
            let slot = by_model.entry(model.clone()).or_default();
            slot.0 = slot.0.add(*tokens);
            slot.1 = slot.1.saturating_add(*cache_write);
        }
    }
    // `list_usd` prices all uncached input at the input rate; the cache
    // writes inside it cost the write rate (2× input on the built-in rows).
    let priced: Vec<f64> = by_model
        .iter()
        .filter_map(|(model, (tokens, cache_write))| {
            crate::usage_prices::list_usd(model, tokens).map(|(usd, price)| {
                usd + *cache_write as f64 / 1_000_000.0 * (price.cache_write - price.input)
            })
        })
        .collect();
    let estimate = priced.iter().sum::<f64>();
    let found = WorkflowAgentUsage {
        count: agents,
        calls: messages.len() as u64,
        tokens: sum.total(),
        input_tokens: sum.input,
        cached_input_tokens: sum.cached_input,
        output_tokens: sum.output,
        cache_write_tokens: writes,
        estimate_micros: (!priced.is_empty() && estimate.is_finite())
            .then(|| (estimate * 1_000_000.0).round() as u64),
        unmetered_tokens: None,
        basis: "estimated".into(),
        truncated,
    };
    Some((found, unstreamed))
}

/// One `spawn_failed` row for a run refused before it could start (a
/// validation refusal). Separate from [`UsageRun::begin`] so PR 7 can put its
/// permit between validation and the row of a run that does start.
pub fn record_refusal(
    meter: &Meter,
    kind: AgentKind,
    model: Option<String>,
    reason: &str,
) -> RunUsage {
    let mut run = UsageRun::begin(meter.clone(), Some(kind), model);
    run.note = Some(crate::claude_runner::truncate(reason, 200));
    run.finish(LedgerOutcome::SpawnFailed)
}

/// Startup: close every `running` row as `interrupted` (partial when a
/// checkpoint recorded tokens) and prune rows past the retention window.
/// Assumes no other Hollow process is live, like
/// `automated_history::reconcile_interrupted`; PR 7a gates both on the pump
/// owner's lock. `except_run` is the run another live process provably holds
/// the lease for (Paperclip PR 18, `automated_history::foreign_live_owner`,
/// computed once in `lib.rs` and spared by the history reconcile too): its
/// own row and its delegates' rows stay `running`.
pub fn reconcile_interrupted(vault: &Path, except_run: Option<&str>) -> Result<usize, String> {
    let now = Utc::now();
    let closed = session_store::interrupt_running_usage(vault, None, except_run, &db_time(now))?;
    session_store::prune_usage(
        vault,
        &db_time(now - Duration::days(session_store::KEEP_USAGE_DAYS)),
    )?;
    Ok(closed)
}

/// A parent's finish: close its delegates still `running` (their team-mcp
/// server was killed before they could finish).
pub fn close_orphaned_delegates(vault: &Path, parent_run_id: &str) -> Result<usize, String> {
    session_store::interrupt_running_usage(vault, Some(parent_run_id), None, &db_time(Utc::now()))
}

/// Map how a supervised process ended to the row's outcome.
pub fn outcome_for(ended: crate::process_group::Ended, success: bool) -> LedgerOutcome {
    match ended {
        crate::process_group::Ended::Stopped => LedgerOutcome::Cancelled,
        crate::process_group::Ended::TimedOut => LedgerOutcome::Timeout,
        // A watchdog stall (Paperclip PR 15b) is a Hollow-imposed time bound too;
        // the closed outcome vocabulary has no separate value for it.
        crate::process_group::Ended::Stalled => LedgerOutcome::Timeout,
        crate::process_group::Ended::LimitReached => LedgerOutcome::BudgetStopped,
        crate::process_group::Ended::Exited if success => LedgerOutcome::Success,
        crate::process_group::Ended::Exited => LedgerOutcome::Error,
    }
}

// ── Windows ─────────────────────────────────────────────────────────────────

/// Settings, Usage periods, in local time. PR 8's budgets use the same
/// [`window_bounds`], so display and enforcement agree on every edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageWindow {
    /// Today from local midnight.
    Day,
    /// The ISO week from Monday 00:00.
    Week,
    /// The calendar month from the 1st.
    Month,
}

impl UsageWindow {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "day" => Some(UsageWindow::Day),
            "week" => Some(UsageWindow::Week),
            "month" => Some(UsageWindow::Month),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            UsageWindow::Day => "day",
            UsageWindow::Week => "week",
            UsageWindow::Month => "month",
        }
    }
}

/// A window's edges and a stable key (`2026-09-24`, `2026-W39`, `2026-09`).
/// Settings reads `start`; PR 8's budgets read all three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowBounds {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub key: String,
}

/// Infallible, DST-safe window edges: each local midnight resolves through
/// [`first_valid_local`], so a zone whose DST gap swallows midnight still
/// gets an edge (release builds abort on panic, so no unwrap here).
pub fn window_bounds(window: UsageWindow, now: DateTime<Local>) -> WindowBounds {
    window_bounds_in(window, &now)
}

/// [`window_bounds`] in `now`'s own time zone: the one window function
/// (PR 8's budgets evaluate at an explicit instant, so their window and pause
/// tests are exact in any machine time zone).
pub fn window_bounds_in<Tz: TimeZone>(window: UsageWindow, now: &DateTime<Tz>) -> WindowBounds {
    let today = now.date_naive();
    let (first, next, key) = match window {
        UsageWindow::Day => (
            today,
            today.succ_opt(),
            today.format("%Y-%m-%d").to_string(),
        ),
        UsageWindow::Week => {
            let back = i64::from(today.weekday().num_days_from_monday());
            let monday = today
                .checked_sub_signed(Duration::days(back))
                .unwrap_or(today);
            let week = today.iso_week();
            (
                monday,
                monday.checked_add_signed(Duration::days(7)),
                format!("{}-W{:02}", week.year(), week.week()),
            )
        }
        UsageWindow::Month => {
            let first = today.with_day(1).unwrap_or(today);
            (
                first,
                first.checked_add_months(Months::new(1)),
                today.format("%Y-%m").to_string(),
            )
        }
    };
    let tz = now.timezone();
    let offset = now.offset().fix().local_minus_utc();
    let resolve = |naive: NaiveDateTime| {
        tz.from_local_datetime(&naive)
            .earliest()
            .map(|at| at.with_timezone(&Utc))
    };
    let start = first_valid_local(first.and_time(NaiveTime::MIN), resolve, offset);
    let end = next
        .map(|day| first_valid_local(day.and_time(NaiveTime::MIN), resolve, offset))
        .unwrap_or(start + Duration::days(1));
    WindowBounds { start, end, key }
}

/// The first local instant at or after `naive` that exists, stepping 15
/// minutes for up to a day (covers 30- and 45-minute zones), else `naive`
/// read with the current UTC offset. `resolve` is injectable for tests.
pub(crate) fn first_valid_local(
    naive: NaiveDateTime,
    resolve: impl Fn(NaiveDateTime) -> Option<DateTime<Utc>>,
    fallback_offset_secs: i32,
) -> DateTime<Utc> {
    for step in 0..=96i64 {
        let Some(candidate) = naive.checked_add_signed(Duration::minutes(15 * step)) else {
            break;
        };
        if let Some(at) = resolve(candidate) {
            return at;
        }
    }
    naive
        .checked_sub_signed(Duration::seconds(i64::from(fallback_offset_secs)))
        .unwrap_or(naive)
        .and_utc()
}

// ── Settings, Usage ────────────────────────────────────────────────────────

/// Settings, Usage for one window, every agent (the tests' shorthand).
#[cfg(test)]
pub fn summary(
    vault: &Path,
    window: UsageWindow,
    now: DateTime<Local>,
) -> Result<UsageSummary, String> {
    summary_for(vault, UsagePeriod::Window(window), None, now)
}

/// Settings, Usage for one period (Settings PR 8), every agent or one
/// agent tab's (`agent`: `claude`, `codex`, `self-hosted`, ...). `agents`
/// always lists every agent with runs in the period, so the tabs stay put.
pub fn summary_for(
    vault: &Path,
    period: UsagePeriod,
    agent: Option<&str>,
    now: DateTime<Local>,
) -> Result<UsageSummary, String> {
    let start = period_start_in(period, &now);
    let since = db_time(start);
    let mut rows = session_store::usage_rows_since(vault, &since)?;
    // Before the filter on purpose: the tab row lists every agent with runs.
    let agents = detail_buckets(&rows).agents;
    let agent = agent
        .map(str::trim)
        .filter(|a| !a.is_empty() && *a != "all");
    if let Some(agent) = agent {
        rows.retain(|row| agent_family(row) == agent);
    }
    let detail = detail_buckets(&rows);
    let titles: HashMap<String, String> = crate::vault_workflows::list(vault)
        .into_iter()
        .map(|w| (w.slug, w.title))
        .collect();
    let (total, by_surface, by_workflow, by_agent) = aggregate(&rows, &titles);
    let spend = spend_buckets(&rows, &spend_context(vault, &rows));
    let (partial_floor_usd, partial_runs) = partial_floor(&rows);
    let night_runs = night_direct_runs(vault, &since);
    let night_attribution_error = night_runs.as_ref().err().cloned();
    let (daily, undated_runs) = daily_usage(&rows, &night_runs.unwrap_or_default(), &Local);
    let latest_row = rows.iter().map(|row| row.at.as_str()).max();
    // The newest recording failure in the window, unless a row was written
    // after it (recording recovered).
    let ledger_error = crate::health::recent(vault)
        .into_iter()
        .find(|entry| entry.subsystem == "usage" && entry.outcome == crate::health::Outcome::Failed)
        .filter(|entry| entry.at >= start)
        .filter(|entry| latest_row.is_none_or(|at| at <= db_time(entry.at).as_str()))
        .map(|entry| entry.detail);
    let top = top_conversations(&spend.by_lead);
    Ok(UsageSummary {
        window: period.as_str().to_string(),
        agent: agent.map(str::to_string),
        agents,
        by_model: detail.by_model,
        outcomes: detail.outcomes,
        top_conversations: top,
        since,
        generated_at: db_time(now.with_timezone(&Utc)),
        total,
        by_surface,
        by_workflow,
        by_agent,
        by_project: spend.by_project,
        by_lead: spend.by_lead,
        by_subagent: spend.by_subagent,
        by_account: spend.by_account,
        daily,
        undated_runs,
        night_attribution_error,
        ledger_since: session_store::usage_ledger_since(vault)?,
        ledger_error,
        partial_floor_usd,
        partial_runs,
    })
}

/// OB07: the summed priced floor of the rows with tokens but no cost, and
/// how many there are. Kept apart from every bucket's known cost
/// ([`add_row`] reads `cost_usd` only), as budgets keep it out of theirs
/// (`budgets::LedgerRow` has no floor). A live checkpoint (`running`) is in
/// progress, as [`add_row`] counts it, not incomplete: its floor waits for
/// the run's end (an `interrupted` row counts). `None` (never 0) when no row
/// has one. OB24: a row a `session_delta_span` row lists in its `spanRuns`
/// has its cost inside that span's known cost, so its floor is left out.
fn partial_floor(rows: &[UsageRow]) -> (Option<f64>, u32) {
    let spanned = spanned_runs(rows);
    rows.iter()
        .filter(|row| row.outcome != LedgerOutcome::Running.as_str())
        .filter(|row| row.cost_usd.is_none())
        .filter(|row| !spanned.contains(row.run_key.as_str()))
        .filter_map(|row| row.cost_floor_usd)
        .filter(|usd| usd.is_finite() && *usd > 0.0)
        .fold((None::<f64>, 0), |(sum, runs), usd| {
            (Some(sum.unwrap_or(0.0) + usd), runs + 1)
        })
}

/// OB24: the run keys every priced span row in `rows` covers (its
/// `usage_json.spanRuns`, the span row itself included).
fn spanned_runs(rows: &[UsageRow]) -> HashSet<String> {
    rows.iter()
        .filter(|row| row.basis == Basis::SessionDeltaSpan.as_str() && row.cost_usd.is_some())
        .filter_map(|row| row.usage_json.as_deref())
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .filter_map(|doc| doc.get("spanRuns").and_then(Value::as_array).cloned())
        .flatten()
        .filter_map(|key| key.as_str().map(str::to_string))
        .collect()
}

/// Exact queue provenance. UNION prevents a run referenced by multiple night records counting twice.
/// No prompt/name parsing and no classification based on the hour of day.
fn night_direct_runs(vault: &Path, since: &str) -> Result<HashSet<String>, String> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn.prepare(
            "SELECT u.run_id FROM usage_ledger u JOIN run_queue r ON r.history_run_id = u.run_id \
             JOIN night_queue n ON n.queue_id = r.id WHERE u.at >= ?1 AND u.surface = 'chat' \
             UNION SELECT u.run_id FROM usage_ledger u JOIN run_queue r ON r.history_run_id = u.run_id \
             JOIN runbook_steps s ON s.queue_id = r.id JOIN night_queue n ON n.runbook_run = s.run_id \
             WHERE u.at >= ?1 AND u.surface = 'chat'"
        ).map_err(|e| format!("Night attribution: {e}"))?;
        let rows = stmt
            .query_map([since], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Night attribution: {e}"))?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|e| format!("Night attribution: {e}"));
        rows
    })
}

/// Recording day (not start day): checkpoints move with the existing summary's `at` semantics.
/// Invalid dates stay in totals and are disclosed separately; no synthetic zero days.
fn daily_usage<Tz: TimeZone>(
    rows: &[UsageRow],
    night: &HashSet<String>,
    tz: &Tz,
) -> (Vec<UsageDay>, u32)
where
    Tz::Offset: std::fmt::Display,
{
    let mut days = BTreeMap::<String, UsageDay>::new();
    let mut undated = 0;
    for row in rows {
        let Ok(at) = DateTime::parse_from_rfc3339(&row.at) else {
            undated += 1;
            continue;
        };
        let date = at.with_timezone(tz).format("%Y-%m-%d").to_string();
        let day = days.entry(date.clone()).or_insert_with(|| UsageDay {
            date: date.clone(),
            total: UsageBucket {
                key: date.clone(),
                label: date,
                ..Default::default()
            },
            ..Default::default()
        });
        add_row(&mut day.total, row);
        let (agent, label) = agent_bucket(row);
        add_row(bucket_for(&mut day.by_agent, &agent, || label.clone()), row);
        // Delegates carry the parent's run id too: a chat-only condition is essential here.
        if row.surface == "chat" && row.run_id.as_ref().is_some_and(|r| night.contains(r)) {
            add_row(bucket_for(&mut day.night_by_agent, &agent, || label), row);
        }
    }
    (days.into_values().collect(), undated)
}

fn add_cost(slot: &mut Option<f64>, cost: Option<f64>) {
    if let Some(c) = cost.filter(|c| c.is_finite() && *c > 0.0) {
        *slot = Some(slot.unwrap_or(0.0) + c);
    }
}

/// Q8: the "By agent" bucket a row goes in, as `(key, label)`. A
/// self-hosted model's run rides the Claude harness but is not Claude Code's
/// spend, so it has its own bucket, by the id its endpoint served.
fn agent_bucket(row: &UsageRow) -> (String, String) {
    if row.billing == Billing::SelfHosted.as_str() {
        return match row.model.as_deref().or(row.requested_model.as_deref()) {
            Some(model) if !model.trim().is_empty() => (
                format!("self-hosted:{model}"),
                format!("Self-hosted · {model}"),
            ),
            _ => ("self-hosted".into(), "Self-hosted model".into()),
        };
    }
    let agent = row.agent.as_deref().unwrap_or("unknown");
    let label = AgentKind::from_opt(agent)
        .map(|k| k.label().to_string())
        .unwrap_or_else(|| "Unknown agent".to_string());
    (agent.to_string(), label)
}

fn add_row(bucket: &mut UsageBucket, row: &UsageRow) {
    bucket.runs += 1;
    let mut counts_tokens = false;
    // Q8: a self-hosted model's run is billed by GPU time: its tokens count,
    // and it is never "with tokens but no price".
    let gpu_time = row.billing == Billing::SelfHosted.as_str();
    if gpu_time && row.spawned {
        bucket.gpu_time += 1;
    }
    if row.outcome == LedgerOutcome::Running.as_str() {
        bucket.in_progress += 1;
    } else if !row.spawned {
        bucket.not_started += 1;
    } else {
        match row.cost_status.as_str() {
            "unknown" => bucket.unknown += 1,
            "partial" => {
                bucket.partial += 1;
                counts_tokens = true;
            }
            status => {
                bucket.metered += 1;
                if status == CostStatus::Unpriced.as_str() && !gpu_time {
                    bucket.unpriced += 1;
                }
                counts_tokens = true;
            }
        }
    }
    if counts_tokens {
        let n = |value: Option<i64>| value.and_then(|v| u64::try_from(v).ok()).unwrap_or(0);
        bucket.input_tokens += n(row.input_tokens);
        bucket.cached_input_tokens += n(row.cached_input_tokens);
        bucket.output_tokens += n(row.output_tokens);
    }
    match row.cost_status.as_str() {
        "reported" => add_cost(&mut bucket.reported_cost_usd, row.cost_usd),
        "estimated" => add_cost(&mut bucket.estimated_cost_usd, row.cost_usd),
        _ => {}
    }
}

fn bucket_for<'a>(
    buckets: &'a mut Vec<UsageBucket>,
    key: &str,
    label: impl FnOnce() -> String,
) -> &'a mut UsageBucket {
    match buckets.iter().position(|b| b.key == key) {
        Some(at) => &mut buckets[at],
        None => {
            buckets.push(UsageBucket {
                key: key.to_string(),
                label: label(),
                ..UsageBucket::default()
            });
            let last = buckets.len() - 1;
            &mut buckets[last]
        }
    }
}

fn sorted(mut buckets: Vec<UsageBucket>) -> Vec<UsageBucket> {
    buckets.sort_by(|a, b| b.runs.cmp(&a.runs).then_with(|| a.key.cmp(&b.key)));
    buckets
}

/// Group rows by surface, automation and agent. Pure. Delegates run under a
/// workflow are credited to that workflow; reported and estimated costs are
/// never mixed, and stay `None` (never 0) when no such row exists.
pub(crate) fn aggregate(
    rows: &[UsageRow],
    titles: &HashMap<String, String>,
) -> (
    UsageBucket,
    Vec<UsageBucket>,
    Vec<UsageBucket>,
    Vec<UsageBucket>,
) {
    let mut total = UsageBucket {
        key: "total".into(),
        label: "All runs".into(),
        ..UsageBucket::default()
    };
    let mut by_surface = Vec::new();
    let mut by_workflow = Vec::new();
    let mut by_agent = Vec::new();
    for row in rows {
        add_row(&mut total, row);
        let surface = Surface::parse(&row.surface);
        add_row(
            bucket_for(&mut by_surface, &row.surface, || {
                surface
                    .map(|s| s.label().to_string())
                    .unwrap_or_else(|| row.surface.clone())
            }),
            row,
        );
        let workflow = match surface {
            Some(Surface::Workflow) => row.scope.as_deref(),
            Some(Surface::Delegate) if row.parent_surface.as_deref() == Some("workflow") => {
                row.scope.as_deref()
            }
            _ => None,
        };
        if let Some(slug) = workflow.filter(|s| !s.is_empty()) {
            add_row(
                bucket_for(&mut by_workflow, slug, || {
                    titles
                        .get(slug)
                        .cloned()
                        .unwrap_or_else(|| slug.to_string())
                }),
                row,
            );
        }
        let (agent, label) = agent_bucket(row);
        add_row(bucket_for(&mut by_agent, &agent, || label), row);
    }
    (
        total,
        sorted(by_surface),
        sorted(by_workflow),
        sorted(by_agent),
    )
}

/// A conversation as the spend view needs it: `(title, parent_key,
/// fork_kind)` of one `sessions` row.
type SessionInfo = (Option<String>, Option<String>, Option<String>);

/// What the spend buckets look names up in.
#[derive(Default)]
pub(crate) struct SpendContext {
    /// Workspace id to title.
    pub projects: HashMap<String, String>,
    /// Session key to its row.
    pub sessions: HashMap<String, SessionInfo>,
    /// `account_ref` to the account's name.
    pub accounts: HashMap<String, String>,
    /// A handover successor's key to the conversation its chain began as
    /// (L9 review): a successor has no `fork_kind`, so it counts as its
    /// chain's first conversation (a subagent's successor stays that
    /// subagent, under its lead).
    pub handover_origin: HashMap<String, String>,
    /// Old workspace ids to the ones they became (Move Hollow home): a row
    /// recorded under an old id counts under the project's id now.
    pub aliases: crate::workspace_aliases::Aliases,
}

/// The lookups for [`spend_buckets`]. A read that fails leaves its map empty:
/// the buckets then fall back to ids, and still sum.
fn spend_context(vault: &Path, rows: &[UsageRow]) -> SpendContext {
    let projects = crate::workspaces::list(vault)
        .unwrap_or_default()
        .into_iter()
        .map(|w| (w.id, w.title))
        .collect();
    let aliases = session_store::with_read(vault, |conn| {
        Ok(crate::workspace_aliases::Aliases::load(conn))
    })
    .unwrap_or_default();
    let sessions = session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT key, title, parent_key, fork_kind FROM sessions")
            .map_err(|e| format!("Spend sessions: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
            })
            .map_err(|e| format!("Spend sessions: {e}"))?
            .filter_map(Result::ok)
            .collect::<HashMap<_, _>>();
        Ok(rows)
    })
    .unwrap_or_default();
    let accounts = crate::claude_accounts::accounts_view()
        .map(|view| {
            view.accounts
                .into_iter()
                .map(|a| (crate::run_attribution::account_ref(&a.id), a.label))
                .collect()
        })
        .unwrap_or_default();
    let scopes: HashSet<&str> = rows
        .iter()
        .filter_map(|row| row.scope.as_deref())
        .filter(|scope| scope.starts_with("s-"))
        .collect();
    // The handovers are read once, then walked per conversation.
    let pairs = if scopes.is_empty() {
        Vec::new()
    } else {
        crate::app_sessions::handover_pairs(vault)
    };
    let handover_origin = scopes
        .into_iter()
        .filter(|key| !is_subagent(sessions.get(*key)))
        .filter_map(|key| {
            crate::chat_chain::predecessors_in(&pairs, key)
                .last()
                .map(|origin| (key.to_string(), origin.clone()))
        })
        .collect();
    SpendContext {
        projects,
        sessions,
        accounts,
        handover_origin,
        aliases,
    }
}

/// The four spend lists of Settings, Usage.
#[derive(Default)]
pub(crate) struct Spend {
    pub by_project: Vec<UsageBucket>,
    pub by_lead: Vec<UsageBucket>,
    pub by_subagent: Vec<UsageBucket>,
    pub by_account: Vec<UsageBucket>,
}

/// The subagents the by-subagent list names before it sums the rest.
const SUBAGENT_BUCKETS: usize = 50;

fn is_subagent(info: Option<&SessionInfo>) -> bool {
    info.is_some_and(|(_, _, kind)| kind.as_deref() == Some(session_store::FORK_SUBAGENT))
}

/// The conversation a subagent belongs to: follow `parent_key` while the
/// session is a subagent (cycle-safe, at most 32 steps).
fn conversation_root(sessions: &HashMap<String, SessionInfo>, key: &str) -> String {
    let mut current = key.to_string();
    let mut seen = HashSet::new();
    for _ in 0..32 {
        let info = sessions.get(&current);
        if !is_subagent(info) || !seen.insert(current.clone()) {
            break;
        }
        match info.and_then(|(_, parent, _)| parent.clone()) {
            Some(parent) if !parent.is_empty() => current = parent,
            _ => break,
        }
    }
    current
}

fn session_label(sessions: &HashMap<String, SessionInfo>, key: &str) -> String {
    sessions
        .get(key)
        .and_then(|(title, _, _)| title.clone())
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| key.to_string())
}

/// The "By account" bucket of a row, as `(key, label)`: its hashed account
/// label, else its agent's own login.
fn account_bucket(row: &UsageRow, accounts: &HashMap<String, String>) -> (String, String) {
    if let Some(reference) = row.account_ref.as_deref().filter(|r| !r.is_empty()) {
        let label = if reference == crate::claude_accounts::LOGIN_ID {
            "Claude Code login".to_string()
        } else {
            accounts
                .get(reference)
                .cloned()
                .unwrap_or_else(|| "Claude account (removed)".to_string())
        };
        return (reference.to_string(), label);
    }
    let agent = row.agent.as_deref().unwrap_or("unknown");
    let label = match AgentKind::from_opt(agent) {
        Some(AgentKind::Claude) => "Claude (account not recorded)".to_string(),
        Some(kind) => format!("{} (its own login)", kind.label()),
        None => "Unknown agent".to_string(),
    };
    (format!("agent:{agent}"), label)
}

/// By project, lead, subagent and account. Pure. Project and account hold
/// every row; lead and subagent hold the conversation rows (chat, and the
/// delegates a conversation made) only.
pub(crate) fn spend_buckets(rows: &[UsageRow], ctx: &SpendContext) -> Spend {
    let mut spend = Spend::default();
    let mut subagents = Vec::new();
    for row in rows {
        let project = row
            .workspace_id
            .as_deref()
            .filter(|w| !w.is_empty())
            .map(|w| ctx.aliases.current(w));
        let (key, label) = match project.as_deref() {
            Some(id) => (
                id.to_string(),
                ctx.projects
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| id.to_string()),
            ),
            None => ("none".to_string(), "No project".to_string()),
        };
        add_row(bucket_for(&mut spend.by_project, &key, || label), row);
        let (key, label) = account_bucket(row, &ctx.accounts);
        add_row(bucket_for(&mut spend.by_account, &key, || label), row);

        // A delegate under a workflow or a heartbeat carries that run's
        // scope, not a conversation's.
        let conversation = match Surface::parse(&row.surface) {
            Some(Surface::Chat) => true,
            Some(Surface::Delegate) => {
                matches!(row.parent_surface.as_deref(), None | Some("chat"))
            }
            _ => false,
        };
        if !conversation {
            continue;
        }
        let scope = row.scope.as_deref().filter(|s| !s.is_empty());
        let Some(scope) = scope else {
            add_row(
                bucket_for(&mut spend.by_lead, "none", || "No conversation".into()),
                row,
            );
            continue;
        };
        let scope = ctx
            .handover_origin
            .get(scope)
            .map(String::as_str)
            .unwrap_or(scope);
        let root = conversation_root(&ctx.sessions, scope);
        add_row(
            bucket_for(&mut spend.by_lead, &root, || {
                session_label(&ctx.sessions, &root)
            }),
            row,
        );
        if is_subagent(ctx.sessions.get(scope)) {
            add_row(
                bucket_for(&mut subagents, scope, || {
                    session_label(&ctx.sessions, scope)
                }),
                row,
            );
        }
    }
    let tokens = |b: &UsageBucket| b.input_tokens + b.cached_input_tokens + b.output_tokens;
    subagents.sort_by(|a, b| tokens(b).cmp(&tokens(a)).then_with(|| a.key.cmp(&b.key)));
    if subagents.len() > SUBAGENT_BUCKETS {
        let mut other = UsageBucket {
            key: "other".into(),
            label: "Other subagents".into(),
            ..UsageBucket::default()
        };
        for b in subagents.split_off(SUBAGENT_BUCKETS) {
            merge_bucket(&mut other, &b);
        }
        subagents.push(other);
    }
    spend.by_subagent = subagents;
    spend.by_project = sorted(spend.by_project);
    spend.by_lead = sorted(spend.by_lead);
    spend.by_account = sorted(spend.by_account);
    spend
}

/// Add `from`'s counts to `into`.
fn merge_bucket(into: &mut UsageBucket, from: &UsageBucket) {
    into.runs += from.runs;
    into.metered += from.metered;
    into.partial += from.partial;
    into.unknown += from.unknown;
    into.not_started += from.not_started;
    into.in_progress += from.in_progress;
    into.unpriced += from.unpriced;
    into.gpu_time += from.gpu_time;
    into.input_tokens += from.input_tokens;
    into.cached_input_tokens += from.cached_input_tokens;
    into.output_tokens += from.output_tokens;
    for (slot, cost) in [
        (&mut into.reported_cost_usd, from.reported_cost_usd),
        (&mut into.estimated_cost_usd, from.estimated_cost_usd),
    ] {
        add_cost(slot, cost);
    }
}

// ── Settings, Usage in detail (Settings PR 8) ─────────────────────────────

/// Settings, Usage's period (Settings PR 8): a budget window, or the last
/// 30 days. Budgets keep [`UsageWindow`] alone: a rolling 30 days has no
/// window key a pause could wait on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsagePeriod {
    Window(UsageWindow),
    /// Local midnight 29 days before today, so today is the 30th day.
    Last30Days,
}

impl UsagePeriod {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "last30" => Some(UsagePeriod::Last30Days),
            other => UsageWindow::parse(other).map(UsagePeriod::Window),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            UsagePeriod::Window(window) => window.as_str(),
            UsagePeriod::Last30Days => "last30",
        }
    }
}

/// Where a period starts, in `now`'s own time zone (DST-safe, like
/// [`window_bounds_in`]).
pub fn period_start_in<Tz: TimeZone>(period: UsagePeriod, now: &DateTime<Tz>) -> DateTime<Utc> {
    match period {
        UsagePeriod::Window(window) => window_bounds_in(window, now).start,
        UsagePeriod::Last30Days => {
            let today = now.date_naive();
            let first = today
                .checked_sub_signed(Duration::days(29))
                .unwrap_or(today);
            let tz = now.timezone();
            let offset = now.offset().fix().local_minus_utc();
            first_valid_local(
                first.and_time(NaiveTime::MIN),
                |naive| {
                    tz.from_local_datetime(&naive)
                        .earliest()
                        .map(|at| at.with_timezone(&Utc))
                },
                offset,
            )
        }
    }
}

/// The agent tab a row belongs to (Settings PR 8): `self-hosted` for a
/// self-hosted model's run (it rides the Claude harness but is not Claude's
/// spend, as in [`agent_bucket`]), else the row's agent.
pub(crate) fn agent_family(row: &UsageRow) -> String {
    if row.billing == Billing::SelfHosted.as_str() {
        return "self-hosted".into();
    }
    row.agent.clone().unwrap_or_else(|| "unknown".into())
}

fn agent_family_label(key: &str) -> String {
    if key == "self-hosted" {
        return "Self-hosted".into();
    }
    AgentKind::from_opt(key)
        .map(|k| k.label().to_string())
        .unwrap_or_else(|| "Unknown agent".to_string())
}

/// The runs of a failed-over turn that moved on (Settings PR 8): a chat row
/// whose run id has a later chat row on another account. A delegate carries
/// its parent's run id, so only chat rows count.
fn moved_runs(rows: &[UsageRow]) -> HashSet<String> {
    let mut turns: HashMap<&str, Vec<&UsageRow>> = HashMap::new();
    for row in rows {
        let Some(run) = row.run_id.as_deref().filter(|r| !r.is_empty()) else {
            continue;
        };
        if row.surface == "chat" && row.account_ref.as_deref().is_some_and(|a| !a.is_empty()) {
            turns.entry(run).or_default().push(row);
        }
    }
    let mut moved = HashSet::new();
    for mut attempts in turns.into_values().filter(|a| a.len() > 1) {
        attempts.sort_by(|a, b| {
            a.started_at
                .cmp(&b.started_at)
                .then_with(|| a.at.cmp(&b.at))
        });
        for (at, row) in attempts.iter().enumerate() {
            if attempts[at + 1..]
                .iter()
                .any(|later| later.account_ref != row.account_ref)
            {
                moved.insert(row.run_key.clone());
            }
        }
    }
    moved
}

/// A row's outcome as Usage, Outcomes lists it, as `(key, label)`.
fn outcome_bucket(row: &UsageRow, moved: &HashSet<String>) -> (&'static str, &'static str) {
    if row.outcome == LedgerOutcome::Running.as_str() {
        return ("running", "Still running");
    }
    if moved.contains(&row.run_key) {
        return ("moved", "Moved to another account");
    }
    match row.outcome.as_str() {
        "success" => ("finished", "Finished"),
        "cancelled" => ("stopped", "Stopped"),
        "budget_stopped" => ("budget", "Stopped by a budget"),
        "timeout" => ("timeout", "Ran out of time"),
        "spawn_failed" => ("not_started", "Didn't start"),
        "interrupted" | "abandoned" => ("interrupted", "Interrupted"),
        _ => ("failed", "Failed"),
    }
}

/// The model a row ran, as `(key, label)`: the model it reported, else the
/// one asked for.
fn model_bucket(row: &UsageRow) -> (String, String) {
    match row
        .model
        .as_deref()
        .or(row.requested_model.as_deref())
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        Some(model) => (model.to_string(), model.to_string()),
        None => ("unknown".into(), "Model not reported".into()),
    }
}

/// Usage in detail's extra lists (Settings PR 8), from the same rows as
/// [`aggregate`], so each sums to its total.
#[derive(Default)]
pub(crate) struct Detail {
    /// By agent tab (`claude`, `codex`, `self-hosted`, ...).
    pub agents: Vec<UsageBucket>,
    pub by_model: Vec<UsageBucket>,
    pub outcomes: Vec<UsageBucket>,
}

pub(crate) fn detail_buckets(rows: &[UsageRow]) -> Detail {
    let moved = moved_runs(rows);
    let mut detail = Detail::default();
    for row in rows {
        let family = agent_family(row);
        add_row(
            bucket_for(&mut detail.agents, &family, || agent_family_label(&family)),
            row,
        );
        let (key, label) = model_bucket(row);
        add_row(bucket_for(&mut detail.by_model, &key, || label), row);
        let (key, label) = outcome_bucket(row, &moved);
        add_row(
            bucket_for(&mut detail.outcomes, key, || label.to_string()),
            row,
        );
    }
    detail.agents = sorted(detail.agents);
    detail.by_model = sorted(detail.by_model);
    detail.outcomes = sorted(detail.outcomes);
    detail
}

/// The conversations that cost most (Settings PR 8): by known cost, then
/// tokens, at most [`TOP_CONVERSATIONS`]. Each key is the conversation's
/// session key, so Usage can open its chat.
const TOP_CONVERSATIONS: usize = 10;

pub(crate) fn top_conversations(by_lead: &[UsageBucket]) -> Vec<UsageBucket> {
    let cost =
        |b: &UsageBucket| b.reported_cost_usd.unwrap_or(0.0) + b.estimated_cost_usd.unwrap_or(0.0);
    let tokens = |b: &UsageBucket| b.input_tokens + b.cached_input_tokens + b.output_tokens;
    let mut top: Vec<UsageBucket> = by_lead
        .iter()
        .filter(|b| b.key != "none")
        .cloned()
        .collect();
    top.sort_by(|a, b| {
        cost(b)
            .total_cmp(&cost(a))
            .then_with(|| tokens(b).cmp(&tokens(a)))
            .then_with(|| a.key.cmp(&b.key))
    });
    top.truncate(TOP_CONVERSATIONS);
    top
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::tests::usage_row_fixture;
    use crate::session_store::usage_rows_since;
    use crate::usage::tests::{CHAT_RESUME, CHAT_TOOL};

    fn tc(input: u64, cached_input: u64, output: u64) -> TokenCounts {
        TokenCounts {
            input,
            cached_input,
            output,
        }
    }

    fn meter(vault: &Path, surface: Surface) -> Meter {
        Meter::for_vault(vault, surface, Some("test".into()), Some("R0".into()))
    }

    fn message(id: &str, tokens: TokenCounts) -> AgentEvent {
        AgentEvent::MessageUsage {
            message_id: id.into(),
            model: Some("claude-opus-5-5".into()),
            tokens,
            is_final: false,
            cache_write: 0,
            thread: None,
        }
    }

    /// Q8 (self-hosted models): Q0's real Qwen turn, as Claude Code reported
    /// it (its own $0.84 estimate for an id it doesn't know, `costBasis:
    /// unknown`). Billed by GPU time, the row keeps the tokens under the
    /// served id and no price at all, whatever the person picked (`opus`
    /// here); the same report on unknown billing is still the CLI's
    /// estimate.
    const QWEN_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":3,"duration_ms":19885,"total_cost_usd":0.841735,"usage":{"input_tokens":163722,"output_tokens":925,"cache_read_input_tokens":0,"cache_creation_input_tokens":0},"modelUsage":{"qwen3.8-27b":{"inputTokens":163722,"outputTokens":925,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"costUSD":0.841735,"contextWindow":200000,"maxOutputTokens":32000,"canonicalModel":"qwen3.8-27b","costBasis":"unknown"}},"session_id":"s-q8","result":"ok"}"#;

    #[test]
    fn a_self_hosted_run_is_billed_by_gpu_time_never_priced() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = UsageRun::begin(
            meter(dir.path(), Surface::Chat),
            Some(AgentKind::Claude),
            Some("opus".into()),
        );
        run.set_billing(Billing::SelfHosted);
        run.set_self_hosted(Some(("Qwen3.8 27B", "qwen3.8-27b")));
        let qwen_message = AgentEvent::MessageUsage {
            message_id: "m-q8".into(),
            model: Some("qwen3.8-27b".into()),
            tokens: tc(163_722, 0, 925),
            is_final: false,
            cache_write: 0,
            thread: None,
        };
        // The night guard prices nothing for it (Q8 review).
        assert!(run.observe(&qwen_message));
        assert_eq!(run.night_estimate(), 0.0, "no Anthropic spend to estimate");
        feed(&mut run, AgentKind::Claude, QWEN_RESULT);
        let usage = run.finish(LedgerOutcome::Success);
        assert_eq!(
            usage.cost_usd, None,
            "never the CLI's made-up price, never $0"
        );
        assert_eq!(usage.cost_status, CostStatus::Unpriced);
        assert_eq!(usage.model.as_deref(), Some("qwen3.8-27b"));
        let tokens = usage.tokens.expect("tokens");
        assert_eq!((tokens.input, tokens.output), (163_722, 925));
        let json = usage.turn_json();
        assert_eq!(json["billing"], "self-hosted");
        assert_eq!(json["endpoint"], "Qwen3.8 27B");
        assert!(json["costUsd"].is_null());
        let rows = usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cost_usd, None);
        assert_eq!(rows[0].cost_status, "unpriced");
        assert_eq!(rows[0].billing, "self-hosted");
        assert_eq!(rows[0].requested_model.as_deref(), Some("qwen3.8-27b"));
        assert_eq!(rows[0].model.as_deref(), Some("qwen3.8-27b"));
        let doc: Value = serde_json::from_str(rows[0].usage_json.as_deref().unwrap()).unwrap();
        assert_eq!(doc["endpoint"], "Qwen3.8 27B");

        // Unknown billing keeps the CLI's estimate, as before, and the
        // night guard prices an unknown id at the dearest row.
        let dir = tempfile::tempdir().unwrap();
        let mut run = UsageRun::begin(
            meter(dir.path(), Surface::Chat),
            Some(AgentKind::Claude),
            None,
        );
        assert!(run.observe(&qwen_message));
        assert!(run.night_estimate() > 0.0);
        run.set_self_hosted(None);
        feed(&mut run, AgentKind::Claude, QWEN_RESULT);
        let usage = run.finish(LedgerOutcome::Success);
        assert_eq!(usage.cost_status, CostStatus::Estimated);
        assert!(usage.cost_usd.is_some());
        assert_eq!(usage.endpoint, None, "None leaves a run as it is");
        let rows = usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows[0].requested_model, None);

        // Q8 second review: a build failure's stand-in carries no run key, so
        // it isn't billed as self-hosted and takes no endpoint or served id.
        let dir = tempfile::tempdir().unwrap();
        let mut run = UsageRun::begin(
            meter(dir.path(), Surface::Chat),
            Some(AgentKind::Claude),
            Some("opus".into()),
        );
        run.set_billing(Billing::Unknown);
        run.set_self_hosted(Some(("Qwen3.8 27B", "qwen3.8-27b")));
        let usage = run.finish(LedgerOutcome::SpawnFailed);
        assert_eq!(usage.endpoint, None);
        let rows = usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows[0].requested_model.as_deref(), Some("opus"));
    }

    /// Q8 review: Settings, Usage lists a self-hosted model's tokens in its
    /// own "By agent" row, billed by GPU time, never as Claude Code's
    /// "tokens but no price".
    #[test]
    fn settings_usage_lists_self_hosted_runs_as_billed_by_gpu_time() {
        let mut claude = row("chat", "success", "reported");
        claude.cost_usd = Some(0.5);
        claude.input_tokens = Some(10);
        let mut qwen = row("chat", "success", "unpriced");
        qwen.billing = Billing::SelfHosted.as_str().into();
        qwen.model = Some("qwen3.8-27b".into());
        qwen.input_tokens = Some(163_722);
        qwen.output_tokens = Some(925);
        let mut codex = row("chat", "success", "unpriced");
        codex.agent = Some("codex".into());
        codex.input_tokens = Some(5);
        let (total, _, _, by_agent) = aggregate(&[claude, qwen, codex], &HashMap::new());
        assert_eq!(
            (total.unpriced, total.gpu_time),
            (1, 1),
            "only Codex is unpriced"
        );
        assert_eq!(total.reported_cost_usd, Some(0.5));
        assert_eq!(total.input_tokens, 10 + 163_722 + 5);
        let by = |key: &str| by_agent.iter().find(|b| b.key == key).unwrap();
        assert_eq!(
            by("claude").runs,
            1,
            "Claude Code's row holds only its own run"
        );
        assert_eq!(by("claude").input_tokens, 10);
        let gpu = by("self-hosted:qwen3.8-27b");
        assert_eq!(gpu.label, "Self-hosted · qwen3.8-27b");
        assert_eq!((gpu.runs, gpu.gpu_time, gpu.unpriced), (1, 1, 0));
        assert_eq!((gpu.input_tokens, gpu.output_tokens), (163_722, 925));
        assert_eq!(
            (gpu.reported_cost_usd, gpu.estimated_cost_usd),
            (None, None)
        );
        assert_eq!(by("codex").unpriced, 1);

        // Q8 second review: a self-hosted run cut short (partial) or with no
        // usable figures (unknown) is still billed by GPU time, in its own
        // row; partial tokens count, unknown ones can't.
        let mut cut = row("chat", "cancelled", "partial");
        cut.billing = Billing::SelfHosted.as_str().into();
        cut.model = Some("qwen3.8-27b".into());
        cut.input_tokens = Some(700);
        let mut blank = row("chat", "error", "unknown");
        blank.billing = Billing::SelfHosted.as_str().into();
        blank.model = Some("qwen3.8-27b".into());
        let (total, _, _, by_agent) = aggregate(&[cut, blank], &HashMap::new());
        assert_eq!(
            (total.partial, total.unknown, total.unpriced, total.gpu_time),
            (1, 1, 0, 2)
        );
        assert_eq!(total.input_tokens, 700);
        assert_eq!(by_agent.len(), 1, "never under Claude Code");
        assert_eq!(by_agent[0].key, "self-hosted:qwen3.8-27b");
        assert_eq!(by_agent[0].gpu_time, 2);
    }

    fn feed(run: &mut UsageRun, kind: AgentKind, text: &str) {
        let mut state = crate::agent::ParseState::default();
        for line in text.lines() {
            for evt in crate::agent::parse_line(kind, line, &mut state) {
                if let AgentEvent::Session(id) = &evt {
                    run.set_native_session(id);
                }
                run.observe(&evt);
            }
        }
    }

    #[test]
    fn a_run_is_inserted_running_then_finished_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let run = UsageRun::begin(
            meter(dir.path(), Surface::Chat),
            Some(AgentKind::Claude),
            None,
        );
        let rows = usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, "running");
        assert_eq!(
            (rows[0].surface.as_str(), rows[0].scope.as_deref()),
            ("chat", Some("test"))
        );
        let key = rows[0].run_key.clone();
        let started = rows[0].at.clone();
        // `started_at` is taken just before the row's `at` (two clock reads:
        // a millisecond can tick between them under load), so compare it
        // with itself across the finish, not with `at`.
        let started_at = rows[0].started_at.clone();
        assert!(started_at <= started);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let usage = run.finish(LedgerOutcome::Success);
        let rows = usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows.len(), 1, "finish updates the same row");
        assert_eq!(rows[0].run_key, key);
        assert_eq!(rows[0].outcome, "success");
        assert!(rows[0].at > started);
        assert_eq!(rows[0].started_at, started_at, "finish keeps the start");
        assert_eq!(usage.cost_status, CostStatus::Unknown);
        assert_eq!(rows[0].input_tokens, None, "no data is NULL, never 0");
    }

    #[test]
    fn final_recording_acknowledges_the_write_not_a_readable_older_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let run = UsageRun::begin(
            meter(dir.path(), Surface::Chat),
            Some(AgentKind::Claude),
            None,
        );
        let key = run.key().to_owned();
        session_store::with_write(dir.path(), "refuse final usage fixture", |tx| {
            tx.execute_batch("CREATE TRIGGER refuse_final_usage BEFORE UPDATE ON usage_ledger WHEN NEW.outcome != 'running' BEGIN SELECT RAISE(ABORT, 'synthetic final write refusal'); END;")
                .map_err(|e| e.to_string())
        }).unwrap();
        let (_, recorded) = run.finish_receipt(LedgerOutcome::Success);
        assert!(recorded.is_none());
        let rows = usage_rows_since(dir.path(), "").unwrap();
        let previous = rows.iter().find(|row| row.run_key == key).unwrap();
        assert_eq!(
            previous.outcome, "running",
            "the old row is still readable, never final-write proof"
        );
        session_store::with_write(dir.path(), "restore usage fixture", |tx| {
            tx.execute_batch("DROP TRIGGER refuse_final_usage")
                .map_err(|e| e.to_string())
        })
        .unwrap();
        let run = UsageRun::begin(
            meter(dir.path(), Surface::Chat),
            Some(AgentKind::Claude),
            None,
        );
        let key = run.key().to_owned();
        let (_, recorded) = run.finish_receipt(LedgerOutcome::Success);
        let receipt = recorded.expect("exact final row was written");
        let rows = usage_rows_since(dir.path(), "").unwrap();
        let stored = rows.iter().find(|row| row.run_key == key).unwrap();
        assert_eq!(stored.outcome, "success");
        assert_eq!(receipt.run_key, stored.run_key);
        assert_eq!(receipt.recorded_at, stored.at);
    }

    #[test]
    fn final_recording_without_a_store_is_not_confirmed() {
        let dir = tempfile::tempdir().unwrap();
        let meter = Meter {
            root: MeterRoot::Missing,
            ..meter(dir.path(), Surface::Chat)
        };
        let run = UsageRun::begin(meter, Some(AgentKind::Claude), None);
        assert!(run.finish_receipt(LedgerOutcome::Success).1.is_none());
    }

    #[test]
    fn an_unfinished_run_records_abandoned_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut run = UsageRun::begin(
                meter(dir.path(), Surface::Oneshot),
                Some(AgentKind::Claude),
                None,
            );
            run.observe(&message("m1", tc(10, 5, 2)));
        }
        let rows = usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].outcome, "abandoned");
        assert_eq!(rows[0].cost_status, "partial");
        assert_eq!(rows[0].input_tokens, Some(10));
        assert_eq!(usage::drop_outcome(true), LedgerOutcome::Cancelled);
        assert_eq!(usage::drop_outcome(false), LedgerOutcome::Abandoned);
        assert!(
            !TERMINATING.load(Ordering::SeqCst),
            "tests never set the global flag"
        );
    }

    #[test]
    fn checkpoints_survive_a_killed_run_and_reconcile_to_partial() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = UsageRun::begin(
            meter(dir.path(), Surface::Workflow),
            Some(AgentKind::Claude),
            None,
        );
        assert!(run.observe(&message("a", tc(100, 50, 10))));
        let row = &usage_rows_since(dir.path(), "").unwrap()[0];
        assert_eq!(
            (row.outcome.as_str(), row.input_tokens),
            ("running", Some(100))
        );
        assert_eq!(row.cost_status, "partial");
        // The second event lands within the checkpoint interval whatever
        // the wall clock did under load: the mirror of the aging helper in
        // `a_resumed_turns_checkpoint_never_stores_the_cumulative_terminal`.
        // A future instant, so `elapsed()` saturates to zero even if this
        // thread is descheduled for longer than the interval right here.
        let (_, last) = run.last_checkpoint.expect("the first event checkpointed");
        let ahead = Instant::now()
            .checked_add(CHECKPOINT_EVERY * 1000)
            .expect("monotonic clock ahead of the interval");
        run.last_checkpoint = Some((ahead, last));
        assert!(run.observe(&message("b", tc(1, 1, 1))));
        let row = &usage_rows_since(dir.path(), "").unwrap()[0];
        assert_eq!(
            row.input_tokens,
            Some(100),
            "throttled within the checkpoint interval"
        );
        // The process dies without destructors.
        std::mem::forget(run);
        assert_eq!(reconcile_interrupted(dir.path(), None).unwrap(), 1);
        let row = &usage_rows_since(dir.path(), "").unwrap()[0];
        assert_eq!(row.outcome, "interrupted");
        assert_eq!(row.cost_status, "partial");
        assert_eq!(
            (row.input_tokens, row.cached_input_tokens, row.output_tokens),
            (Some(100), Some(50), Some(10))
        );
    }

    /// OB07: a run killed after 1M cache-read and 10K output tokens on Opus
    /// keeps a NULL cost but carries the price table's floor. Opus 5.5
    /// (usage_prices.rs: $4 input, $0.20 cache read, $20 output per
    /// million): 0 × $4 + 1.0 × $0.20 + 0.01 × $20 = $0.40. A crash keeps its
    /// checkpoint's floor through the reconcile; messages without a model id
    /// are priced at the row's model; Codex has no built-in price row, so it
    /// keeps its tokens and has no floor.
    #[test]
    fn a_killed_opus_run_keeps_no_cost_but_a_priced_floor() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let mut killed = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        killed.set_billing(Billing::Subscription);
        killed.observe(&message("k1", tc(0, 1_000_000, 10_000)));
        let killed_key = killed.key().to_string();
        let usage = killed.finish(LedgerOutcome::Cancelled);
        assert_eq!(
            (usage.cost_usd, usage.cost_status),
            (None, CostStatus::Partial)
        );

        let mut crashed =
            UsageRun::begin(meter(v, Surface::Workflow), Some(AgentKind::Claude), None);
        crashed.observe(&message("c1", tc(0, 1_000_000, 10_000)));
        let crashed_key = crashed.key().to_string();
        std::mem::forget(crashed);
        assert_eq!(reconcile_interrupted(v, None).unwrap(), 1);

        let mut codex = UsageRun::begin(meter(v, Surface::Oneshot), Some(AgentKind::Codex), None);
        codex.observe(&AgentEvent::MessageUsage {
            message_id: "x1".into(),
            model: Some("gpt-5.3-codex".into()),
            tokens: tc(0, 1_000_000, 10_000),
            is_final: false,
            cache_write: 0,
            thread: None,
        });
        let codex_key = codex.key().to_string();
        codex.finish(LedgerOutcome::Cancelled);

        let opus = Some("claude-opus-5-5".to_string());
        let mut anonymous =
            UsageRun::begin(meter(v, Surface::Ingest), Some(AgentKind::Claude), opus);
        anonymous.observe(&AgentEvent::MessageUsage {
            message_id: "a1".into(),
            model: None,
            tokens: tc(0, 1_000_000, 10_000),
            is_final: false,
            cache_write: 0,
            thread: None,
        });
        let anonymous_key = anonymous.key().to_string();
        anonymous.finish(LedgerOutcome::Timeout);

        let rows = usage_rows_since(v, "").unwrap();
        let get = |key: &str| rows.iter().find(|r| r.run_key == key).cloned().unwrap();
        for key in [
            killed_key.as_str(),
            crashed_key.as_str(),
            anonymous_key.as_str(),
        ] {
            let row = get(key);
            assert_eq!((row.cost_usd, row.cost_status.as_str()), (None, "partial"));
            let floor = row.cost_floor_usd.expect("a priced floor");
            assert!((floor - 0.40).abs() < 1e-9, "{}: {floor}", row.outcome);
        }
        assert_eq!(get(crashed_key.as_str()).outcome, "interrupted");
        let codex = get(codex_key.as_str());
        assert_eq!(
            (codex.cost_status.as_str(), codex.cost_floor_usd),
            ("partial", None)
        );
        assert_eq!(
            (codex.cached_input_tokens, codex.output_tokens),
            (Some(1_000_000), Some(10_000))
        );
    }

    #[test]
    fn startup_reconcile_marks_running_rows_interrupted_and_prunes_old_rows() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let recent = db_time(now - Duration::minutes(5));
        let mut with_tokens = usage_row_fixture("run-tokens", &recent, "running");
        with_tokens.input_tokens = Some(3);
        with_tokens.output_tokens = Some(1);
        with_tokens.cached_input_tokens = Some(0);
        session_store::upsert_usage(v, &with_tokens).unwrap();
        session_store::upsert_usage(v, &usage_row_fixture("run-empty", &recent, "running"))
            .unwrap();
        let days = |n: i64| db_time(now - Duration::days(n));
        session_store::upsert_usage(v, &usage_row_fixture("old", &days(401), "success")).unwrap();
        session_store::upsert_usage(v, &usage_row_fixture("kept", &days(399), "success")).unwrap();
        assert_eq!(reconcile_interrupted(v, None).unwrap(), 2);
        let rows = usage_rows_since(v, "").unwrap();
        let get = |key: &str| rows.iter().find(|r| r.run_key == key).cloned();
        assert!(get("old").is_none());
        assert!(get("kept").is_some());
        let tokens = get("run-tokens").unwrap();
        assert_eq!(
            (tokens.outcome.as_str(), tokens.cost_status.as_str()),
            ("interrupted", "partial")
        );
        let empty = get("run-empty").unwrap();
        assert_eq!(
            (empty.outcome.as_str(), empty.cost_status.as_str()),
            ("interrupted", "unknown")
        );
    }

    #[test]
    fn a_parent_finish_closes_only_its_own_running_delegates() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let at = db_time(Utc::now());
        for (key, surface, run_id) in [
            ("d1", "delegate", "R1"),
            ("d2", "delegate", "R2"),
            ("c1", "chat", "R1"),
        ] {
            let mut row = usage_row_fixture(key, &at, "running");
            row.surface = surface.into();
            row.run_id = Some(run_id.into());
            session_store::upsert_usage(v, &row).unwrap();
        }
        assert_eq!(close_orphaned_delegates(v, "R1").unwrap(), 1);
        let rows = usage_rows_since(v, "").unwrap();
        let outcome = |key: &str| {
            rows.iter()
                .find(|r| r.run_key == key)
                .unwrap()
                .outcome
                .clone()
        };
        assert_eq!(outcome("d1"), "interrupted");
        assert_eq!(outcome("d2"), "running");
        assert_eq!(outcome("c1"), "running");
    }

    /// The contract's acceptance test: a delegate whose cwd is outside the
    /// vault writes to the vault's store.
    #[test]
    fn a_delegate_whose_cwd_is_outside_the_vault_writes_to_the_vaults_store() {
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let env: HashMap<&str, String> = HashMap::from([
            (
                "HOLLOW_VAULT",
                outside.path().to_string_lossy().into_owned(),
            ),
            (ENV_VAULT_ROOT, vault.path().to_string_lossy().into_owned()),
            (ENV_RUN_SURFACE, "workflow".to_string()),
            (ENV_RUN_SCOPE, "morning-brief".to_string()),
            (ENV_RUN_ID, "R1".to_string()),
        ]);
        let meter = Meter::delegate_from_env(|key| env.get(key).cloned());
        UsageRun::begin(meter, Some(AgentKind::Codex), None).finish(LedgerOutcome::Success);
        let rows = usage_rows_since(vault.path(), "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].surface, "delegate");
        assert_eq!(rows[0].parent_surface.as_deref(), Some("workflow"));
        assert_eq!(rows[0].scope.as_deref(), Some("morning-brief"));
        assert_eq!(rows[0].run_id.as_deref(), Some("R1"));
        assert!(usage_rows_since(outside.path(), "").unwrap().is_empty());

        // Without HOLLOW_VAULT_ROOT nothing is written anywhere, and there is
        // never a fallback to HOLLOW_VAULT (the cwd).
        let other_vault = tempfile::tempdir().unwrap();
        let meter = Meter::delegate_from_env(|key| {
            (key == "HOLLOW_VAULT").then(|| other_vault.path().to_string_lossy().into_owned())
        });
        UsageRun::begin(meter, Some(AgentKind::Codex), None).finish(LedgerOutcome::Success);
        assert!(usage_rows_since(other_vault.path(), "").unwrap().is_empty());
        assert_eq!(usage_rows_since(vault.path(), "").unwrap().len(), 1);
    }

    #[test]
    fn a_resumed_chat_turn_records_only_its_own_usage() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let mut first = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        first.set_billing(Billing::Subscription);
        first.set_resumed(None);
        feed(&mut first, AgentKind::Claude, CHAT_TOOL);
        let first = first.finish(LedgerOutcome::Success);
        assert_eq!(first.tokens, Some(tc(19_034, 42_412, 87)));
        assert_eq!(first.cost_status, CostStatus::Estimated);

        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut second = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        second.set_billing(Billing::Subscription);
        second.set_resumed(Some("id-003".into()));
        feed(&mut second, AgentKind::Claude, CHAT_RESUME);
        let second = second.finish(LedgerOutcome::Success);
        assert_eq!(second.basis, Basis::SessionDelta);
        assert_eq!(second.tokens, Some(tc(148, 30_211, 3)));
        assert!((second.cost_usd.unwrap() - 0.0072782).abs() < 1e-9);
        assert_eq!(second.cost_status, CostStatus::Estimated);

        // With no earlier row for the session: main-model tokens, partial.
        let fresh = tempfile::tempdir().unwrap();
        let mut orphan = UsageRun::begin(
            meter(fresh.path(), Surface::Chat),
            Some(AgentKind::Claude),
            None,
        );
        orphan.set_resumed(Some("id-003".into()));
        feed(&mut orphan, AgentKind::Claude, CHAT_RESUME);
        let orphan = orphan.finish(LedgerOutcome::Success);
        assert_eq!(orphan.basis, Basis::ResumeUnverified);
        assert_eq!(orphan.cost_usd, None);
        assert_eq!(orphan.cost_status, CostStatus::Partial);
        let rows = usage_rows_since(fresh.path(), "").unwrap();
        assert_eq!(rows[0].basis, "resume_unverified");
        assert_eq!(rows[0].native_session.as_deref(), Some("id-003"));
        // OB07: its floor (148 input, 30,211 cache-read and 3 output tokens
        // at the Opus row: $0.000592 + $0.0060422 + $0.00006 = $0.0066942)
        // sits below the same turn's verified cost above, $0.0072782.
        let floor = rows[0]
            .cost_floor_usd
            .expect("an unverified resume has a floor");
        assert!((floor - 0.0066942).abs() < 1e-9, "{floor}");
        assert!(floor < 0.0072782);
    }

    /// OB24: a stored terminal, then a cancelled turn (no terminal), then a
    /// resume whose cumulative cost rose $0.37 and whose token delta carries
    /// the cancelled turn's 8,000 cache reads. Before, the resume found the
    /// cancelled row, which has no terminal to subtract, so it recorded
    /// `resume_unverified` with no cost; now it records
    /// $0.37 as a span over both runs, and the summary's known cost rises by
    /// it once (the cancelled row's floor is not added beside it).
    #[test]
    fn a_resume_after_a_cancelled_turn_records_the_cost_rise_once_as_a_span() {
        crate::test_clock::clear_of_midnight(chrono::Duration::seconds(10));
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let mut first = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        first.set_billing(Billing::Subscription);
        feed(&mut first, AgentKind::Claude, CHAT_TOOL);
        let first = first.finish(LedgerOutcome::Success);
        let first_cost = first.cost_usd.expect("the first turn's cost");

        std::thread::sleep(std::time::Duration::from_millis(5));
        let mut cancelled = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        cancelled.set_billing(Billing::Subscription);
        cancelled.set_resumed(Some("id-003".into()));
        cancelled.observe(&message("c1", tc(0, 8_000, 0)));
        let cancelled_key = cancelled.key().to_string();
        let cancelled = cancelled.finish(LedgerOutcome::Cancelled);
        assert_eq!(
            (cancelled.cost_usd, cancelled.cost_status),
            (None, CostStatus::Partial)
        );
        let middle = summary(v, UsageWindow::Day, Local::now()).unwrap();
        assert_eq!(middle.total.estimated_cost_usd, Some(first_cost));
        let floor = middle.partial_floor_usd.expect("a floor");
        assert_eq!(middle.partial_runs, 1);

        std::thread::sleep(std::time::Duration::from_millis(5));
        let resumed = CHAT_RESUME
            .replace(
                "\"total_cost_usd\":0.16288660000000002",
                "\"total_cost_usd\":0.5256084",
            )
            .replace(
                "\"cacheReadInputTokens\":72623",
                "\"cacheReadInputTokens\":80623",
            );
        let mut third = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        third.set_billing(Billing::Subscription);
        third.set_resumed(Some("id-003".into()));
        feed(&mut third, AgentKind::Claude, &resumed);
        let third_key = third.key().to_string();
        let third = third.finish(LedgerOutcome::Success);
        assert_eq!(third.basis, Basis::SessionDeltaSpan);
        assert!((third.cost_usd.unwrap() - 0.37).abs() < 1e-9);
        assert_eq!(third.cost_status, CostStatus::Estimated);
        assert_eq!(third.tokens, Some(tc(148, 30_211, 3)));

        let rows = usage_rows_since(v, "").unwrap();
        let row = rows.iter().find(|r| r.run_key == third_key).unwrap();
        assert_eq!(row.basis, "session_delta_span");
        assert_eq!(row.cost_floor_usd, None);
        let doc: Value = serde_json::from_str(row.usage_json.as_deref().unwrap()).unwrap();
        assert_eq!(
            doc["spanRuns"],
            json!([&cancelled_key, &third_key]),
            "a span over 2 runs"
        );
        assert_eq!(doc["unstreamedTokens"], json!(8_000));
        assert!(!doc["terminal"].is_null(), "the next resume's baseline");

        let after = summary(v, UsageWindow::Day, Local::now()).unwrap();
        let known = after.total.estimated_cost_usd.unwrap();
        assert!((known - first_cost - 0.37).abs() < 1e-9, "{known}");
        assert_eq!(
            (after.partial_floor_usd, after.partial_runs),
            (None, 0),
            "the spanned row's floor (${floor}) is not added beside the span"
        );
    }

    /// OB24: the first turn of native session `id-003`, which stores a
    /// terminal; its cost.
    fn span_baseline(v: &Path) -> f64 {
        let mut first = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        first.set_billing(Billing::Subscription);
        feed(&mut first, AgentKind::Claude, CHAT_TOOL);
        let first = first.finish(LedgerOutcome::Success);
        std::thread::sleep(std::time::Duration::from_millis(5));
        first.cost_usd.expect("the first turn's cost")
    }

    /// OB24: a cancelled row of `id-003` written straight to the store, with
    /// no terminal and `cost_usd` (a turn guard's estimate, when some).
    fn skipped_row(v: &Path, key: &str, at: &str, cost_usd: Option<f64>) {
        let mut row = usage_row_fixture(key, at, "cancelled");
        row.native_session = Some("id-003".into());
        row.usage_json = Some("{\"terminal\":null}".into());
        row.cost_usd = cost_usd;
        row.cost_status = "partial".into();
        session_store::upsert_usage(v, &row).unwrap();
    }

    /// OB24: a resume of `id-003` whose cumulative cost rose $0.37 since
    /// [`span_baseline`]; its result and its stored row.
    fn resume_with_a_rise(v: &Path) -> (RunUsage, UsageRow) {
        std::thread::sleep(std::time::Duration::from_millis(5));
        let resumed = CHAT_RESUME
            .replace(
                "\"total_cost_usd\":0.16288660000000002",
                "\"total_cost_usd\":0.5256084",
            )
            .replace(
                "\"cacheReadInputTokens\":72623",
                "\"cacheReadInputTokens\":80623",
            );
        let mut run = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        run.set_billing(Billing::Subscription);
        run.set_resumed(Some("id-003".into()));
        feed(&mut run, AgentKind::Claude, &resumed);
        let key = run.key().to_string();
        let usage = run.finish(LedgerOutcome::Success);
        let row = usage_rows_since(v, "")
            .unwrap()
            .into_iter()
            .find(|r| r.run_key == key)
            .unwrap();
        (usage, row)
    }

    /// OB24 review: a span over more runs than [`SPAN_RUNS_MAX`] keeps the
    /// newest keys (its own last) and counts the rest, and its `usage_json`
    /// stays under the cap with the terminal the next resume subtracts.
    #[test]
    fn a_span_over_many_runs_keeps_the_newest_keys_under_the_cap() {
        crate::test_clock::clear_of_midnight(chrono::Duration::seconds(10));
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        span_baseline(v);
        let at = db_time(Utc::now());
        let skipped: Vec<String> = (0..120u64)
            .map(|i| format!("u-{}-{i:016x}", Utc::now().timestamp_millis()))
            .collect();
        for key in &skipped {
            skipped_row(v, key, &at, None);
        }
        let (usage, row) = resume_with_a_rise(v);
        assert_eq!(usage.basis, Basis::SessionDeltaSpan);
        let text = row.usage_json.as_deref().unwrap();
        assert!(text.len() <= USAGE_JSON_MAX, "{}", text.len());
        let doc: Value = serde_json::from_str(text).unwrap();
        let runs = doc["spanRuns"].as_array().expect("spanRuns");
        assert_eq!(runs.len(), SPAN_RUNS_MAX);
        assert_eq!(doc["spanRunsOmitted"], json!(121 - SPAN_RUNS_MAX));
        assert_eq!(runs[0], json!(skipped[121 - SPAN_RUNS_MAX]));
        assert_eq!(runs.last(), Some(&json!(row.run_key)));
        assert!(!doc["terminal"].is_null(), "the next resume's baseline");
    }

    /// OB24 review: a row in between that already has a cost (a turn
    /// guard's estimate) would be counted again inside the span's cost rise,
    /// so the resume stays `resume_unverified`.
    #[test]
    fn a_resume_over_a_row_with_a_cost_records_no_span() {
        crate::test_clock::clear_of_midnight(chrono::Duration::seconds(10));
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        span_baseline(v);
        skipped_row(v, "u-guarded", &db_time(Utc::now()), Some(0.2));
        let (usage, row) = resume_with_a_rise(v);
        assert_eq!(usage.basis, Basis::ResumeUnverified);
        assert_eq!(
            (usage.cost_usd, usage.cost_status),
            (None, CostStatus::Partial)
        );
        assert_eq!(row.basis, "resume_unverified");
        let doc: Value = serde_json::from_str(row.usage_json.as_deref().unwrap()).unwrap();
        assert!(doc.get("spanRuns").is_none());
    }

    /// OB24 review: when even the cut-down `usage_json` is over the cap, the
    /// run keys go (all counted as omitted) before the terminal does.
    #[test]
    fn an_oversized_span_drops_its_run_keys_before_its_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let mut run = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        run.set_resumed(Some("id-003".into()));
        feed(&mut run, AgentKind::Claude, CHAT_RESUME);
        let resolved = Resolved {
            tokens: None,
            cost_usd: Some(0.37),
            models: vec![],
            complete: true,
            basis: Basis::SessionDeltaSpan,
            unstreamed_tokens: Some(0),
        };
        // S31: the workflow bucket survives the cut too.
        run.workflow_agents = Some(WorkflowAgentUsage {
            count: 3,
            calls: 40,
            tokens: 9_000_000,
            estimate_micros: Some(12_345_678),
            unmetered_tokens: Some(8_000_000),
            basis: "estimated".into(),
            ..WorkflowAgentUsage::default()
        });
        // 80 keys of 60 characters: the 64 kept are about 4 KB on their own.
        let skipped: Vec<String> = (0..80).map(|i| format!("{i:0>60}")).collect();
        let text = run.usage_json(Basis::SessionDeltaSpan, Some(&resolved), &skipped);
        assert!(text.len() <= USAGE_JSON_MAX, "{}", text.len());
        let doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["truncated"], json!(true));
        assert!(doc.get("spanRuns").is_none());
        assert_eq!(doc["spanRunsOmitted"], json!(81));
        assert!(!doc["terminal"].is_null(), "the next resume's baseline");
        assert_eq!(doc["workflowAgents"]["estimateUsd"], json!(12.345678));
        assert_eq!(doc["workflowAgents"]["unmeteredTokens"], 8_000_000);
        let _ = run.finish(LedgerOutcome::Success);
    }

    /// OB05: the row's `usage_json` carries the run's calls, peak context,
    /// cache writes, API time and turns, and a subagent bucket; what the
    /// agent didn't supply is absent.
    #[test]
    fn usage_json_records_calls_peak_context_and_subagents() {
        let doc = |dir: &Path| -> Value {
            let rows = usage_rows_since(dir, "").unwrap();
            let text = rows[0].usage_json.clone().unwrap();
            serde_json::from_str(&text).unwrap()
        };
        let run_on = |text: &str, kind: AgentKind, resumed: Option<&str>| {
            let dir = tempfile::tempdir().unwrap();
            let mut run = UsageRun::begin(meter(dir.path(), Surface::Chat), Some(kind), None);
            run.set_resumed(resumed.map(str::to_string));
            feed(&mut run, kind, text);
            let usage = run.finish(LedgerOutcome::Success);
            (doc(dir.path()), usage)
        };

        // claude-chat-tool: msg-003 and msg-004; msg-004's prompt
        // (2 + 105 + 30211) is the peak; cache writes 18010 + 105.
        let (fresh, usage) = run_on(CHAT_TOOL, AgentKind::Claude, None);
        assert_eq!(fresh["apiCalls"], 2);
        assert_eq!(fresh["peakContextTokens"], 30_318);
        assert_eq!(fresh["cacheWriteTokens"], 18_115);
        assert_eq!(fresh["apiMs"], 4_739);
        assert_eq!(fresh["numTurns"], 2);
        assert!(fresh.get("subagents").is_none(), "no subagent ran");
        assert_eq!(usage.calls.peak_context_tokens, Some(30_318));

        let (sub, _) = run_on(crate::usage::tests::SUBAGENT, AgentKind::Claude, None);
        assert_eq!(sub["apiCalls"], 2);
        assert_eq!(sub["peakContextTokens"], 21_402);
        assert_eq!(sub["subagents"], json!({ "calls": 2, "tokens": 10_564 }));

        // On resume `duration_api_ms` is session-cumulative: not stored.
        let (resumed, _) = run_on(CHAT_RESUME, AgentKind::Claude, Some("id-003"));
        assert!(resumed.get("apiMs").is_none(), "{resumed}");
        assert_eq!(resumed["numTurns"], 1);

        // Codex supplies none of them.
        let (codex, _) = run_on(crate::usage::tests::CODEX_EXEC, AgentKind::Codex, None);
        for key in [
            "apiCalls",
            "peakContextTokens",
            "cacheWriteTokens",
            "apiMs",
            "numTurns",
            "subagents",
        ] {
            assert!(codex.get(key).is_none(), "{key} in {codex}");
        }
    }

    /// S31: a temp Claude projects tree for session `sess-1` run in `cwd`,
    /// with `files` (`<workflow>/<name>`, lines) under its workflow folder.
    fn workflow_tree(base: &Path, cwd: &Path, files: &[(&str, Vec<String>)]) -> PathBuf {
        let projects = base.join("projects");
        let cwd = std::fs::canonicalize(cwd).unwrap();
        let root = projects
            .join(crate::run_manifest::claude_project_slug(&cwd))
            .join("sess-1")
            .join("subagents")
            .join("workflows");
        for (name, lines) in files {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, lines.join("\n") + "\n").unwrap();
        }
        // The session's main transcript beside its folder: never counted.
        let main = root
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .with_extension("jsonl");
        let now = Utc::now().to_rfc3339();
        let usage = json!({ "input_tokens": 70_000, "output_tokens": 70_000 });
        std::fs::write(main, workflow_line(&now, "m-main", usage) + "\n").unwrap();
        projects
    }

    /// S31: a transcript line deserializes to exactly the message's id,
    /// model and usage; its content (and every other field) is never kept.
    #[test]
    fn a_workflow_line_keeps_only_id_model_and_usage() {
        let line = workflow_line(
            "2026-10-19T10:00:00Z",
            "m1",
            json!({ "input_tokens": 1, "output_tokens": 2 }),
        );
        let parsed: WorkflowLine = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed.timestamp.as_deref(), Some("2026-10-19T10:00:00Z"));
        let holder = workflow_holder(parsed.message.unwrap());
        let keys: Vec<&String> = holder.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["id", "model", "usage"]);
        assert!(!holder.to_string().contains("S31-CANARY"), "{holder}");
    }

    /// An assistant line of a workflow agent's transcript, stamped `at`,
    /// whose content carries a canary that must never reach the ledger.
    fn workflow_line(at: &str, id: &str, usage: Value) -> String {
        json!({
            "type": "assistant",
            "timestamp": at,
            "message": {
                "id": id,
                "model": "claude-opus-5-5",
                "role": "assistant",
                "content": [{ "type": "text", "text": "S31-CANARY reply text" }],
                "usage": usage,
            },
        })
        .to_string()
    }

    /// A finished Claude run of session `sess-1` in `cwd` whose transcripts
    /// live under `projects`: its `usage_json` and what it returned.
    fn workflow_run(
        vault: &Path,
        kind: AgentKind,
        projects: &Path,
        cwd: &Path,
    ) -> (Value, RunUsage) {
        let mut run = UsageRun::begin(meter(vault, Surface::Chat), Some(kind), None);
        run.claude_dirs = Some((projects.to_path_buf(), cwd.to_path_buf()));
        run.set_native_session("sess-1");
        // The run began a minute ago, so lines stamped "now" fall inside it.
        run.started_at -= Duration::seconds(60);
        let usage = run.finish(LedgerOutcome::Success);
        let rows = usage_rows_since(vault, "").unwrap();
        let text = rows[0].usage_json.clone().unwrap();
        assert!(
            !text.contains("S31-CANARY"),
            "content reached the row: {text}"
        );
        (serde_json::from_str(&text).unwrap(), usage)
    }

    /// S31: two workflow agents' messages inside the run's window give the
    /// hand-summed bucket (a repeated snapshot max-merges; a line before the
    /// run, a user line and a non-jsonl file count nothing), labelled
    /// estimated, never in the row's tokens or cost, and no content.
    #[test]
    fn workflow_agents_bucket_sums_two_agents_inside_the_run_window() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        let now = Utc::now().to_rfc3339();
        let usage = |input: u64, write: u64, read: u64, output: u64| {
            json!({
                "input_tokens": input,
                "cache_creation_input_tokens": write,
                "cache_read_input_tokens": read,
                "output_tokens": output,
            })
        };
        let user = json!({
            "type": "user",
            "timestamp": now,
            "message": { "role": "user", "content": "the \"usage\" S31-CANARY" },
        })
        .to_string();
        let projects = workflow_tree(
            dir.path(),
            &cwd,
            &[
                (
                    "wf-a/agent-1.jsonl",
                    vec![
                        workflow_line("2020-01-01T00:00:00Z", "m0", usage(9_000, 0, 0, 9_000)),
                        workflow_line(&now, "m1", usage(100, 20, 1_000, 5)),
                        user,
                        workflow_line(&now, "m1", usage(100, 20, 1_000, 50)),
                    ],
                ),
                (
                    "wf-b/agent-2.jsonl",
                    vec![workflow_line(&now, "m2", usage(10, 0, 0, 7))],
                ),
                (
                    "wf-b/notes.txt",
                    vec![workflow_line(&now, "m3", usage(1, 0, 0, 1))],
                ),
            ],
        );
        let (doc, run_usage) = workflow_run(dir.path(), AgentKind::Claude, &projects, &cwd);
        // m1: input 100 + 20 written, 1,000 read, 50 out; m2: 10 in, 7 out.
        let bucket = &doc["workflowAgents"];
        assert_eq!(bucket["count"], 2, "{doc}");
        assert_eq!(bucket["calls"], 2);
        assert_eq!(bucket["inputTokens"], 130);
        assert_eq!(bucket["cachedInputTokens"], 1_000);
        assert_eq!(bucket["outputTokens"], 57);
        assert_eq!(bucket["cacheWriteTokens"], 20);
        assert_eq!(bucket["tokens"], 1_187);
        assert_eq!(bucket["basis"], "estimated");
        // At the opus row, cache writes at the write rate.
        let price = crate::usage_prices::table()
            .into_iter()
            .find(|p| p.model == "claude-opus-5-5")
            .unwrap();
        let expected = (130.0 * price.input
            + 1_000.0 * price.cache_read
            + 57.0 * price.output
            + 20.0 * (price.cache_write - price.input))
            / 1_000_000.0;
        let got = bucket["estimateUsd"].as_f64().unwrap();
        assert!((got - expected).abs() < 1e-6, "{got} vs {expected}");
        // Nothing streamed and the row holds no tokens: all of it unmetered.
        assert_eq!(bucket["unmeteredTokens"], 1_187);
        assert!(bucket.get("truncated").is_none());
        // Never folded into the row's own figures.
        assert_eq!(run_usage.tokens, None);
        assert_eq!(run_usage.cost_usd, None);
        let found = run_usage
            .calls
            .workflow_agents
            .clone()
            .expect("bucket on the receipt");
        assert_eq!((found.calls, found.tokens), (2, 1_187));
        // The receipt's apiUsage round-trips it.
        let stats: CallStats =
            serde_json::from_value(serde_json::to_value(&run_usage.calls).unwrap()).unwrap();
        assert_eq!(stats, run_usage.calls);
    }

    /// S31: no bucket for a run with no workflow agents, one whose agents
    /// wrote only before it began, or a non-Claude run.
    #[test]
    fn a_run_without_workflow_agents_in_its_window_writes_no_bucket() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        let empty = dir.path().join("none");
        let (doc, usage) = workflow_run(dir.path(), AgentKind::Claude, &empty, &cwd);
        assert!(doc.get("workflowAgents").is_none(), "{doc}");
        assert!(usage.calls.workflow_agents.is_none());

        let old = workflow_line(
            "2020-01-01T00:00:00Z",
            "m0",
            json!({ "input_tokens": 5, "output_tokens": 5 }),
        );
        let projects = workflow_tree(dir.path(), &cwd, &[("wf/a.jsonl", vec![old])]);
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let vault = tempfile::tempdir().unwrap();
            let (doc, _) = workflow_run(vault.path(), kind, &projects, &cwd);
            assert!(doc.get("workflowAgents").is_none(), "{kind:?}: {doc}");
        }
        let now = Utc::now().to_rfc3339();
        let fresh = workflow_line(&now, "m1", json!({ "input_tokens": 5, "output_tokens": 5 }));
        let projects = workflow_tree(dir.path(), &cwd, &[("wf/b.jsonl", vec![fresh])]);
        let vault = tempfile::tempdir().unwrap();
        let (doc, _) = workflow_run(vault.path(), AgentKind::Codex, &projects, &cwd);
        assert!(doc.get("workflowAgents").is_none(), "Codex: {doc}");
    }

    /// The `usage_json` of a run of `kind` fed `texts` in order: two texts
    /// are two results on one row (rare in chat; the rule is a guard).
    fn attempts_doc(kind: AgentKind, resumed: Option<&str>, texts: &[&str]) -> Value {
        let dir = tempfile::tempdir().unwrap();
        let mut run = UsageRun::begin(meter(dir.path(), Surface::Chat), Some(kind), None);
        run.set_resumed(resumed.map(str::to_string));
        for text in texts {
            feed(&mut run, kind, text);
        }
        let _ = run.finish(LedgerOutcome::Success);
        let rows = usage_rows_since(dir.path(), "").unwrap();
        serde_json::from_str(rows[0].usage_json.as_deref().unwrap()).unwrap()
    }

    /// #741 OB05 review: a Claude row with two attempts (the fresh
    /// claude-chat-tool turn, then a `--resume` of its session id-003) sums
    /// their `num_turns` (2 + 1) and keeps the last `duration_api_ms`, which
    /// is session-cumulative (6990 already holds the first 4739): neither the
    /// second attempt alone for turns nor 4739 + 6990 for API time. A row
    /// that itself resumed still drops the cumulative figure.
    #[test]
    fn a_claude_row_with_two_attempts_sums_turns_and_keeps_the_cumulative_api_time() {
        let fresh = attempts_doc(AgentKind::Claude, None, &[CHAT_TOOL, CHAT_RESUME]);
        assert_eq!(fresh["numTurns"], 3, "{fresh}");
        assert_eq!(fresh["apiMs"], 6_990, "{fresh}");

        let resumed = attempts_doc(AgentKind::Claude, Some("id-003"), &[CHAT_TOOL, CHAT_RESUME]);
        assert_eq!(resumed["numTurns"], 3, "{resumed}");
        assert!(resumed.get("apiMs").is_none(), "{resumed}");
    }

    /// Grok goes through Claude's parser, but its `duration_api_ms` is per
    /// invocation on `--resume` (fixtures/grok/resume.jsonl reports 2164 ms
    /// for the one-shot session it resumed, whose own result said 3526): a
    /// resumed Grok row keeps it, and two attempts sum. Its messages report
    /// `cache_creation_input_tokens: 0`, so `cacheWriteTokens` is absent.
    #[test]
    fn grok_api_time_is_per_invocation_so_a_resume_keeps_it_and_attempts_sum() {
        const GROK_ONESHOT: &str = include_str!("../fixtures/grok/oneshot.jsonl");
        const GROK_RESUME: &str = include_str!("../fixtures/grok/resume.jsonl");
        let session = "01a0df79-39e3-7560-86f9-3b598ba5ff05";

        let resumed = attempts_doc(AgentKind::Grok, Some(session), &[GROK_RESUME]);
        assert_eq!(resumed["apiMs"], 2_164, "{resumed}");
        assert_eq!(resumed["numTurns"], 1, "{resumed}");
        assert!(resumed.get("cacheWriteTokens").is_none(), "{resumed}");

        let both = attempts_doc(AgentKind::Grok, None, &[GROK_ONESHOT, GROK_RESUME]);
        assert_eq!(both["apiMs"], 3_526 + 2_164, "{both}");
        assert_eq!(both["numTurns"], 2, "{both}");
    }

    /// A resumed Claude turn's `result` is session-cumulative. A checkpoint
    /// taken after it, followed by a crash before `finish`, must not leave
    /// the earlier turns' tokens on the row that reconcile keeps.
    #[test]
    fn a_resumed_turns_checkpoint_never_stores_the_cumulative_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let mut run = UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Claude), None);
        run.set_resumed(Some("id-003".into()));
        let mut state = crate::agent::ParseState::default();
        let mut saw_terminal = false;
        for line in CHAT_RESUME.lines() {
            // Every event lands past the checkpoint interval.
            if let Some((_, last)) = run.last_checkpoint {
                let past = Instant::now()
                    .checked_sub(CHECKPOINT_EVERY * 2)
                    .expect("monotonic clock past the interval");
                run.last_checkpoint = Some((past, last));
            }
            for evt in crate::agent::parse_line(AgentKind::Claude, line, &mut state) {
                saw_terminal |= matches!(evt, AgentEvent::Result { .. });
                run.observe(&evt);
            }
        }
        assert!(saw_terminal, "the fixture carries its result line");
        assert_eq!(
            run.accumulator().running_total(),
            Some(tc(19_182, 72_623, 90)),
            "the raw terminal is cumulative"
        );
        assert_eq!(run.running_total(), Some(tc(148, 30_211, 3)));
        std::mem::forget(run);
        assert_eq!(reconcile_interrupted(v, None).unwrap(), 1);
        let row = &usage_rows_since(v, "").unwrap()[0];
        assert_eq!(
            (row.outcome.as_str(), row.cost_status.as_str()),
            ("interrupted", "partial")
        );
        assert_eq!(
            (row.input_tokens, row.cached_input_tokens, row.output_tokens),
            (Some(148), Some(30_211), Some(3))
        );
    }

    #[test]
    fn a_refusal_writes_one_spawn_failed_row_and_the_stand_in_never_counts_as_started() {
        let dir = tempfile::tempdir().unwrap();
        let usage = record_refusal(
            &meter(dir.path(), Surface::Oneshot),
            AgentKind::Antigravity,
            Some("m".into()),
            "Antigravity does not support the tool restrictions",
        );
        assert_eq!(usage.outcome, LedgerOutcome::SpawnFailed);
        let rows = usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].spawned);
        assert!(rows[0]
            .usage_json
            .as_deref()
            .unwrap()
            .contains("tool restrictions"));

        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.args(["-c", "exit 1", crate::agent::UNAVAILABLE_ARGV0, "x"]);
        let mut run = UsageRun::begin(
            meter(dir.path(), Surface::Oneshot),
            Some(AgentKind::Codex),
            None,
        );
        run.launched(&cmd);
        assert_eq!(
            run.finish(LedgerOutcome::Error).outcome,
            LedgerOutcome::SpawnFailed
        );
    }

    /// Run ledger v2 (F2): each attempt of a failed-over turn is its own row
    /// and names the account that ran it (a hashed label, never the id), and
    /// the workspace its process started in with that brief's department.
    #[test]
    fn a_failed_over_turn_records_both_accounts_and_its_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let ws = vault.join("workspaces/payroll");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join("WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Payroll\nstatus: active\ndepartment: Finance\n---\n",
        )
        .unwrap();
        let attempt = |account: &str| {
            let mut cmd = tokio::process::Command::new("/bin/true");
            cmd.current_dir(&ws);
            cmd.env(crate::claude_accounts::ACCOUNT_ENV, account);
            let mut run =
                UsageRun::begin(meter(vault, Surface::Chat), Some(AgentKind::Claude), None);
            run.launched(&cmd);
            let key = run.key().to_string();
            let _ = run.finish(LedgerOutcome::Error);
            key
        };
        let first = attempt("admin-1");
        let second = attempt("tech-2");
        let rows = crate::run_ledger::rows_between(
            vault,
            "2000-01-01T00:00:00.000Z",
            "2100-01-01T00:00:00.000Z",
        )
        .unwrap();
        let of = |key: &str| rows.iter().find(|r| r.run_key == key).unwrap().clone();
        let (a, b) = (of(&first), of(&second));
        assert_eq!(a.account, crate::run_attribution::account_ref("admin-1"));
        assert_eq!(b.account, crate::run_attribution::account_ref("tech-2"));
        assert_ne!(a.account, b.account);
        for r in [&a, &b] {
            assert!(!r.account.contains("admin") && !r.account.contains("tech"));
            assert_eq!((&*r.workspace, &*r.department), ("payroll", "Finance"));
        }
        // Another agent has no account: unknown, never the login.
        let mut cmd = tokio::process::Command::new("/bin/true");
        cmd.current_dir(&ws);
        let mut run = UsageRun::begin(meter(vault, Surface::Chat), Some(AgentKind::Codex), None);
        run.launched(&cmd);
        let key = run.key().to_string();
        let _ = run.finish(LedgerOutcome::Success);
        let rows = crate::run_ledger::rows_between(
            vault,
            "2000-01-01T00:00:00.000Z",
            "2100-01-01T00:00:00.000Z",
        )
        .unwrap();
        let codex = rows.iter().find(|r| r.run_key == key).unwrap();
        assert_eq!(codex.account, crate::run_attribution::UNKNOWN);
        assert_eq!(codex.workspace, "payroll");
    }

    /// Overnight 9 A13: a test-only row (not a real price: lanes enter
    /// none), $2 input, $0.20 cached, $10 output per million.
    fn codex_test_price() -> crate::usage_prices::Price {
        crate::usage_prices::Price {
            model: "gpt-test-codex".into(),
            input: 2.0,
            cache_write: 2.0,
            cache_read: 0.20,
            output: 10.0,
            source: "test row".into(),
            as_of: "2026-10-19".into(),
        }
    }

    /// Overnight 9 A13: codex-exec.jsonl's turn is 23,753 input (11,904
    /// cached) and 5 output. At the test row (per million: $2 input, $0.20
    /// cached, $10 output) that is $0.0261288, `estimated`, summed apart
    /// from reported costs, with the price's source on the row. The model
    /// comes from the request, else from the rollout's `turn_context`.
    #[test]
    fn a_codex_run_with_a_saved_price_is_estimated() {
        crate::usage_prices::with_saved_rows(vec![codex_test_price()], || {
            let dir = tempfile::tempdir().unwrap();
            let v = dir.path();
            let requested = Some("gpt-test-codex".to_string());
            let mut asked =
                UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Codex), requested);
            feed(
                &mut asked,
                AgentKind::Codex,
                crate::usage::tests::CODEX_EXEC,
            );
            let usage = asked.finish(LedgerOutcome::Success);
            assert_eq!(usage.cost_status, CostStatus::Estimated);
            let usd = usage.cost_usd.expect("priced");
            assert!((usd - 0.0261288).abs() < 1e-9, "{usd}");

            // A default-model run: the rollout named the model it ran.
            let mut default =
                UsageRun::begin(meter(v, Surface::Chat), Some(AgentKind::Codex), None);
            default.set_turn_model("gpt-test-codex-2026-10-01");
            feed(
                &mut default,
                AgentKind::Codex,
                crate::usage::tests::CODEX_EXEC,
            );
            let key = default.key().to_string();
            let usage = default.finish(LedgerOutcome::Success);
            assert_eq!(
                (usage.cost_status, usage.model.as_deref()),
                (CostStatus::Estimated, Some("gpt-test-codex-2026-10-01"))
            );

            let rows = usage_rows_since(v, "").unwrap();
            let row = rows.iter().find(|r| r.run_key == key).unwrap();
            assert_eq!(row.cost_status, "estimated");
            let doc: Value = serde_json::from_str(row.usage_json.as_deref().unwrap()).unwrap();
            assert_eq!(
                doc["priceSource"],
                json!("gpt-test-codex (test row, as of 2026-10-19)")
            );
            let mut total = UsageBucket::default();
            for row in &rows {
                add_row(&mut total, row);
            }
            assert_eq!((total.reported_cost_usd, total.unpriced), (None, 0));
            let estimated = total.estimated_cost_usd.expect("estimated apart");
            assert!((estimated - 2.0 * 0.0261288).abs() < 1e-9, "{estimated}");
        });
    }

    /// 2026-11-20: an Antigravity turn is priced at Google's published
    /// Gemini rate in the built-in table (Argon: $2 input, $0.10 cached, $10
    /// output per million), `estimated` with the source on the row; a resume
    /// is its own step only; a model with no row (3.7 Flash) stays unpriced.
    #[test]
    fn an_antigravity_turn_is_estimated_at_the_published_gemini_rate() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let run = |text: &str, model: Option<&str>| {
            let mut run = UsageRun::begin(
                meter(v, Surface::Chat),
                Some(AgentKind::Antigravity),
                model.map(str::to_string),
            );
            feed(&mut run, AgentKind::Antigravity, text);
            let key = run.key().to_string();
            (key, run.finish(LedgerOutcome::Success))
        };
        let (key, turn) = run(crate::usage::tests::AGY_TURN, Some("gemini-4-argon-low"));
        assert_eq!(turn.cost_status, CostStatus::Estimated);
        let usd = turn.cost_usd.expect("priced");
        // 18912 * 2 + 14671 * 0.10 + 47 * 10, per million.
        assert!((usd - 0.0397611).abs() < 1e-9, "{usd}");
        let (_, resumed) = run(crate::usage::tests::AGY_RESUME, Some("gemini-4-argon-low"));
        let usd = resumed.cost_usd.expect("priced");
        assert!((usd - 0.035714).abs() < 1e-9, "{usd}");
        let rows = usage_rows_since(v, "").unwrap();
        let row = rows.iter().find(|r| r.run_key == key).unwrap();
        assert_eq!(
            (row.cost_status.as_str(), row.model.as_deref()),
            ("estimated", Some("gemini-4-argon"))
        );
        let doc: Value = serde_json::from_str(row.usage_json.as_deref().unwrap()).unwrap();
        assert!(doc["priceSource"]
            .as_str()
            .unwrap()
            .starts_with("gemini-4-argon (Google Gemini API"));
        let unlisted =
            crate::usage::tests::AGY_TURN.replace("gemini-4-argon-low", "gemini-3.7-flash-high");
        let (_, flash) = run(&unlisted, None);
        assert_eq!(
            (flash.cost_status, flash.cost_usd),
            (CostStatus::Unpriced, None)
        );
    }

    /// Overnight 9 A13: with no row for its model (or no model at all) a
    /// Codex run keeps its tokens and stays unpriced, never $0; a Claude
    /// run's status is untouched by a saved row.
    #[test]
    fn without_a_row_a_codex_run_stays_unpriced() {
        crate::usage_prices::with_saved_rows(vec![codex_test_price()], || {
            let dir = tempfile::tempdir().unwrap();
            let v = dir.path();
            for model in [Some("gpt-other"), None] {
                let mut run = UsageRun::begin(
                    meter(v, Surface::Chat),
                    Some(AgentKind::Codex),
                    model.map(str::to_string),
                );
                feed(&mut run, AgentKind::Codex, crate::usage::tests::CODEX_EXEC);
                let usage = run.finish(LedgerOutcome::Success);
                assert_eq!(
                    (usage.cost_usd, usage.cost_status),
                    (None, CostStatus::Unpriced),
                    "{model:?}"
                );
                assert!(usage.tokens.is_some());
            }
        });
    }

    fn row(surface: &str, outcome: &str, status: &str) -> UsageRow {
        let mut row = usage_row_fixture(
            &format!("{surface}-{outcome}-{status}-{}", rand::random::<u32>()),
            "2026-09-24T12:00:00.000Z",
            outcome,
        );
        row.surface = surface.into();
        row.cost_status = status.into();
        row
    }

    #[test]
    fn daily_usage_keeps_recording_day_cost_basis_gaps_and_unknowns() {
        let mut a = row("chat", "success", "reported");
        a.at = "2026-10-02T02:00:00.000Z".into();
        a.started_at = "2026-09-29T12:00:00.000Z".into();
        a.cost_usd = Some(2.0);
        let mut b = a.clone();
        b.run_key = "b".into();
        b.cost_status = "estimated".into();
        b.cost_usd = Some(7.0);
        let mut c = a.clone();
        c.run_key = "c".into();
        c.at = "2026-10-04T02:00:00.000Z".into();
        c.cost_status = "unpriced".into();
        c.cost_usd = None;
        c.agent = Some("codex".into());
        let mut d = a.clone();
        d.run_key = "d".into();
        d.at = "broken".into();
        let rows = vec![a, b, c, d];
        let tz = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
        let (days, undated) = daily_usage(&rows, &HashSet::new(), &tz);
        assert_eq!(undated, 1);
        assert_eq!(
            days.iter().map(|d| d.date.as_str()).collect::<Vec<_>>(),
            vec!["2026-10-01", "2026-10-03"]
        );
        assert_eq!(days[0].total.reported_cost_usd, Some(2.0));
        assert_eq!(days[0].total.estimated_cost_usd, Some(7.0));
        assert_eq!(days[1].total.reported_cost_usd, None);
        assert_eq!(days[1].total.unpriced, 1);
        assert_eq!(
            days.iter().map(|d| d.total.runs).sum::<u32>() + undated,
            rows.len() as u32
        );
    }

    #[test]
    fn daily_usage_only_attributes_exact_direct_chat_runs() {
        let mut a = row("chat", "success", "estimated");
        a.run_id = Some("night".into());
        a.cost_usd = Some(3.0);
        let mut delegate = a.clone();
        delegate.surface = "delegate".into();
        let mut manual = a.clone();
        manual.run_id = Some("manual".into());
        manual.scope = Some("night lane".into());
        let mut running = a.clone();
        running.outcome = "running".into();
        running.cost_status = "partial".into();
        running.cost_usd = None;
        let (days, _) = daily_usage(
            &[a, delegate, manual, running],
            &HashSet::from(["night".into()]),
            &Utc,
        );
        assert_eq!(days[0].total.runs, 4);
        assert_eq!(days[0].night_by_agent[0].runs, 2);
        assert_eq!(days[0].night_by_agent[0].estimated_cost_usd, Some(3.0));
        assert_eq!(days[0].night_by_agent[0].in_progress, 1);
    }

    #[test]
    fn night_usage_links_queue_and_runbook_ids_without_duplicate_or_label_inference() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        session_store::with_write(vault, "night usage fixture", |tx| {
            tx.execute_batch("INSERT INTO run_queue(id,source,label,status,priority,requested_at,history_run_id) VALUES
                (1,'chat','manual night lane','done',0,'2026-10-01','direct'),
                (2,'chat','step','done',0,'2026-10-01','step'),
                (3,'chat','overnight lane','done',0,'2026-10-01','manual');
                INSERT INTO night_queue(workspace,prompt,kind,added_by,status,added_at,night,queue_id,runbook_run) VALUES
                ('w','p','chat','user','done','2026-10-01','night',1,8),
                ('w','p','chat','user','done','2026-10-01','night',1,NULL);
                INSERT INTO runbook_steps(run_id,seq,status,evidence,queue_id,at) VALUES (8,1,'done','',2,'2026-10-01');")
                .map_err(|e| e.to_string())?; Ok(())
        }).unwrap();
        for run in ["direct", "step", "manual"] {
            let mut r = row("chat", "success", "estimated");
            r.run_id = Some(run.into());
            session_store::upsert_usage(vault, &r).unwrap();
        }
        assert!(night_direct_runs(vault, "2099").unwrap().is_empty());
        assert_eq!(
            night_direct_runs(vault, "").unwrap(),
            HashSet::from(["direct".into(), "step".into()])
        );
    }

    #[test]
    fn summary_groups_by_surface_workflow_and_agent_and_counts_partial_unknown_separately() {
        let mut reported = row("chat", "success", "reported");
        reported.cost_usd = Some(0.42);
        reported.input_tokens = Some(100);
        reported.output_tokens = Some(10);
        let mut workflow = row("workflow", "success", "estimated");
        workflow.scope = Some("morning-brief".into());
        workflow.cost_usd = Some(3.10);
        workflow.input_tokens = Some(1_000);
        let mut delegate = row("delegate", "success", "unpriced");
        delegate.parent_surface = Some("workflow".into());
        delegate.scope = Some("morning-brief".into());
        delegate.agent = Some("codex".into());
        delegate.input_tokens = Some(50);
        let mut partial = row("chat", "timeout", "partial");
        partial.output_tokens = Some(7);
        let unknown = row("chat", "error", "unknown");
        let mut not_started = row("oneshot", "spawn_failed", "unknown");
        not_started.spawned = false;
        let mut running = row("heartbeat", "running", "partial");
        running.input_tokens = Some(9_999);
        let rows = vec![
            reported,
            workflow,
            delegate,
            partial,
            unknown,
            not_started,
            running,
        ];
        let titles = HashMap::from([("morning-brief".to_string(), "Morning brief".to_string())]);
        let (total, by_surface, by_workflow, by_agent) = aggregate(&rows, &titles);

        assert_eq!(total.runs, 7);
        assert_eq!(
            (
                total.metered,
                total.partial,
                total.unknown,
                total.not_started,
                total.in_progress
            ),
            (3, 1, 1, 1, 1)
        );
        assert_eq!(total.unpriced, 1);
        assert_eq!(
            total.input_tokens, 1_150,
            "metered and partial only, never in-progress"
        );
        assert_eq!(total.output_tokens, 17);
        assert_eq!(total.reported_cost_usd, Some(0.42));
        assert_eq!(total.estimated_cost_usd, Some(3.10));

        assert_eq!(by_workflow.len(), 1);
        assert_eq!(by_workflow[0].label, "Morning brief");
        assert_eq!(
            by_workflow[0].runs, 2,
            "the delegate is credited to its workflow"
        );
        assert_eq!(by_workflow[0].input_tokens, 1_050);
        assert_eq!(
            by_workflow[0].reported_cost_usd, None,
            "never 0 when absent"
        );

        let chat = by_surface.iter().find(|b| b.key == "chat").unwrap();
        assert_eq!((chat.label.as_str(), chat.runs), ("Chat", 3));
        assert_eq!(chat.estimated_cost_usd, None);
        let codex = by_agent.iter().find(|b| b.key == "codex").unwrap();
        assert_eq!((codex.unpriced, codex.input_tokens), (1, 50));
        assert_eq!(by_agent[0].key, "claude", "most runs first");
    }

    /// OB07: Settings, Usage reports the floor apart ("≥ $X more in N
    /// incomplete runs"); no known total ever includes it.
    #[test]
    fn the_summary_reports_the_partial_floor_apart_from_known_cost() {
        crate::test_clock::clear_of_midnight(chrono::Duration::seconds(10));
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let at = db_time(Utc::now());
        let put =
            |key: &str, outcome: &str, status: &str, cost: Option<f64>, floor: Option<f64>| {
                let mut row = usage_row_fixture(key, &at, outcome);
                row.cost_status = status.into();
                row.cost_usd = cost;
                row.cost_floor_usd = floor;
                row.output_tokens = Some(10);
                session_store::upsert_usage(v, &row).unwrap();
            };
        put("known", "success", "estimated", Some(0.42), None);
        put("killed", "cancelled", "partial", None, Some(0.40));
        put("resumed", "success", "partial", None, Some(0.25));
        put("crashed", "interrupted", "partial", None, Some(0.10));
        // A guard's estimate is the row's cost: a floor beside it is ignored.
        put("guarded", "budget_stopped", "partial", Some(1.0), Some(9.0));
        put("codex", "cancelled", "partial", None, None);
        // A live checkpoint is in progress, not incomplete: its floor waits.
        put("live", "running", "partial", None, Some(5.0));
        let s = summary(v, UsageWindow::Day, Local::now()).unwrap();
        let floor = s.partial_floor_usd.expect("the floor is reported");
        assert!((floor - 0.75).abs() < 1e-9, "{floor}");
        assert_eq!(s.partial_runs, 3);
        assert_eq!((s.total.partial, s.total.in_progress), (5, 1));
        assert_eq!(
            s.total.estimated_cost_usd,
            Some(0.42),
            "the floor is never known cost"
        );
        assert_eq!(s.total.reported_cost_usd, None);
        assert_eq!(partial_floor(&[]), (None, 0), "no floor is None, never $0");
    }

    #[test]
    fn ledger_error_is_hidden_once_a_later_row_was_written() {
        // Each write is read back in today's window (overnight D2).
        crate::test_clock::clear_of_midnight(chrono::Duration::seconds(10));
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        report_error(
            Some(v),
            "usage write failed for someone@example.com: disk I/O error",
        );
        let s = summary(v, UsageWindow::Day, Local::now()).unwrap();
        let error = s
            .ledger_error
            .expect("a failure with no later row surfaces");
        assert!(error.contains("disk I/O error"));
        assert!(!error.contains("someone@"), "redacted: {error}");
        std::thread::sleep(std::time::Duration::from_millis(5));
        UsageRun::begin(meter(v, Surface::Chat), None, None).finish(LedgerOutcome::Success);
        let s = summary(v, UsageWindow::Day, Local::now()).unwrap();
        assert_eq!(s.ledger_error, None);
        assert!(s.ledger_since.is_some());
        assert_eq!(s.total.runs, 1);
    }

    #[test]
    fn windows_start_at_local_midnight_monday_and_the_first() {
        let now = Local
            .with_ymd_and_hms(2026, 9, 24, 15, 0, 0)
            .single()
            .expect("a Thursday");
        let local = |at: DateTime<Utc>| at.with_timezone(&Local).naive_local().to_string();
        let day = window_bounds(UsageWindow::Day, now);
        assert_eq!(local(day.start), "2026-09-24 00:00:00");
        assert_eq!(local(day.end), "2026-09-25 00:00:00");
        assert_eq!(day.key, "2026-09-24");
        let week = window_bounds(UsageWindow::Week, now);
        assert_eq!(local(week.start), "2026-09-21 00:00:00");
        assert_eq!(week.key, "2026-W39");
        let month = window_bounds(UsageWindow::Month, now);
        assert_eq!(local(month.start), "2026-09-01 00:00:00");
        assert_eq!(local(month.end), "2026-10-01 00:00:00");
        assert_eq!(month.key, "2026-09");
        let stamp = db_time(day.start);
        assert_eq!(stamp.len(), 24);
        assert!(stamp.ends_with('Z') && stamp.contains('.'));
    }

    #[test]
    fn a_missing_local_midnight_resolves_to_the_first_valid_instant() {
        let midnight = chrono::NaiveDate::from_ymd_opt(2026, 3, 8)
            .unwrap()
            .and_time(NaiveTime::MIN);
        let one_am = midnight + Duration::hours(1);
        let resolve = |n: NaiveDateTime| (n >= one_am).then(|| n.and_utc());
        assert_eq!(first_valid_local(midnight, resolve, 0), one_am.and_utc());
        let never = |_: NaiveDateTime| None;
        assert_eq!(
            first_valid_local(midnight, never, 3_600),
            (midnight - Duration::hours(1)).and_utc()
        );
    }

    #[test]
    fn outcome_for_maps_stop_timeout_and_exit() {
        use crate::process_group::Ended;
        assert_eq!(outcome_for(Ended::Stopped, true), LedgerOutcome::Cancelled);
        assert_eq!(outcome_for(Ended::TimedOut, true), LedgerOutcome::Timeout);
        assert_eq!(outcome_for(Ended::Exited, true), LedgerOutcome::Success);
        assert_eq!(outcome_for(Ended::Exited, false), LedgerOutcome::Error);
        // Paperclip PR 8b: a budget kill is its own outcome, never a cancel.
        assert_eq!(
            outcome_for(Ended::LimitReached, false),
            LedgerOutcome::BudgetStopped
        );
        // Paperclip PR 15b: a watchdog stall records `timeout`.
        assert_eq!(outcome_for(Ended::Stalled, false), LedgerOutcome::Timeout);
        assert_eq!(LedgerOutcome::BudgetStopped.as_str(), "budget_stopped");
    }

    #[test]
    fn turn_json_is_camel_case_and_null_when_missing() {
        let usage = RunUsage {
            tokens: None,
            cost_usd: None,
            model: None,
            cost_status: CostStatus::Unknown,
            billing: Billing::Unknown,
            outcome: LedgerOutcome::Cancelled,
            basis: Basis::PerRun,
            context_tokens: None,
            context_window: None,
            calls: CallStats::default(),
            endpoint: None,
        };
        let json = usage.turn_json();
        assert!(json["inputTokens"].is_null() && json["costUsd"].is_null());
        assert!(json["contextWindow"].is_null());
        assert_eq!(json["costStatus"], "unknown");
    }

    /// Core startup (`core_boot`, from the GUI's setup and `hollow serve`)
    /// starts global services, so this pins its order in source. The
    /// needles are split so this test's own text never matches them.
    #[test]
    fn setup_sets_the_ledger_root_and_reconciles_before_any_scheduler() {
        let source = include_str!("core_boot.rs");
        let at = |needle: &str| {
            source
                .find(needle)
                .unwrap_or_else(|| panic!("core_boot.rs no longer calls {needle}"))
        };
        let set_root = at(concat!("usage_ledger::", "set_root("));
        let reconcile = at(concat!("usage_ledger::", "reconcile_interrupted("));
        assert!(at(concat!("notifications::", "register(")) < set_root);
        assert!(set_root < at(concat!("workflow_permissions::", "initialize(")));
        assert!(set_root < at(concat!("directory_connections::", "start(")));
        // Both reconciles spare the same run (Paperclip PR 18), computed once.
        let spare = at(concat!("automated_history::", "foreign_live_owner("));
        assert!(
            spare
                < at(concat!(
                    "automated_history::",
                    "reconcile_interrupted_sparing("
                ))
        );
        assert!(
            at(concat!(
                "automated_history::",
                "reconcile_interrupted_sparing("
            )) < reconcile
        );
        for later in [
            concat!("workflow_permissions::", "sweep("),
            concat!("heal_orphan", "_running("),
            concat!("workflows::", "start_scheduler("),
            concat!("heartbeat::", "start_timer("),
        ] {
            assert!(
                reconcile < at(later),
                "the usage reconcile precedes {later}"
            );
        }
    }

    /// Overnight 10 A8 (13a): the live reading's calls are the row's own,
    /// corrected as `usage_json` is. A resumed Claude turn's result says the
    /// session's cumulative 6,990 ms; the next message publishes the reading
    /// again, and its `apiMs` stays absent, as the row's does.
    #[test]
    fn the_live_reading_drops_a_resumed_claude_turns_cumulative_api_time() {
        let dir = tempfile::tempdir().unwrap();
        let run_id = "run-o20-a8-resumed-api-ms";
        let gauge = crate::context_gauge::register(run_id, AgentKind::Claude, None, false, 0);
        let mut run = UsageRun::begin(
            meter(dir.path(), Surface::Chat),
            Some(AgentKind::Claude),
            None,
        );
        run.set_resumed(Some("id-003".into()));
        feed(&mut run, AgentKind::Claude, CHAT_RESUME);
        assert_eq!(
            run.accumulator().call_stats().api_ms,
            Some(6_990),
            "the raw figure is session-cumulative"
        );
        run.observe(&message("msg-a8-after", tc(10, 0, 5)));
        gauge.publish_run(&run);
        let reading = crate::context_gauge::reading_json(run_id);
        assert_eq!(reading["live"], json!(true), "{reading}");
        assert_eq!(reading["calls"]["apiMs"], Value::Null, "{reading}");
        assert_eq!(reading["calls"]["numTurns"], json!(1), "{reading}");
        drop(gauge);
        let _ = run.finish(LedgerOutcome::Success);
    }

    fn spend_row(
        key: &str,
        surface: &str,
        scope: Option<&str>,
        tokens: i64,
        cost: f64,
    ) -> UsageRow {
        let mut r = row(surface, "success", "reported");
        r.run_key = key.into();
        r.scope = scope.map(str::to_string);
        r.input_tokens = Some(tokens);
        r.output_tokens = Some(1);
        r.cost_usd = Some(cost);
        r
    }

    fn session(title: &str, parent: Option<&str>, kind: Option<&str>) -> SessionInfo {
        (
            Some(title.into()),
            parent.map(str::to_string),
            kind.map(str::to_string),
        )
    }

    fn sum(buckets: &[UsageBucket]) -> (u32, u64, f64) {
        (
            buckets.iter().map(|b| b.runs).sum(),
            buckets
                .iter()
                .map(|b| b.input_tokens + b.output_tokens)
                .sum(),
            buckets.iter().filter_map(|b| b.reported_cost_usd).sum(),
        )
    }

    #[test]
    fn spend_buckets_sum_to_the_total_and_roll_subagents_up_to_their_lead() {
        let mut rows = vec![
            spend_row("lead", "chat", Some("coo"), 100, 1.0),
            spend_row("sub-a", "chat", Some("sa"), 10, 0.1),
            spend_row("sub-b", "chat", Some("sb"), 20, 0.2),
            spend_row("consult", "delegate", Some("coo"), 5, 0.05),
            spend_row("other-chat", "chat", Some("solo"), 7, 0.07),
            spend_row("wf", "workflow", Some("morning-brief"), 50, 0.5),
            // Sub B handed over: its successor counts as Sub B.
            spend_row("sub-b2", "chat", Some("sb2"), 4, 0.04),
        ];
        let mut wf_delegate = spend_row("wf-del", "delegate", Some("morning-brief"), 3, 0.03);
        wf_delegate.parent_surface = Some("workflow".into());
        rows.push(wf_delegate);
        rows[0].workspace_id = Some("p1".into());
        rows[1].workspace_id = Some("p1".into());
        rows[2].workspace_id = Some("p2".into());
        rows[3].workspace_id = Some("p1".into());
        rows[4].workspace_id = Some("p2".into());
        // The workflow rows have no project.
        rows[0].account_ref = Some("login".into());
        rows[1].account_ref = Some(crate::run_attribution::account_ref("acct-one"));
        rows[2].account_ref = Some(crate::run_attribution::account_ref("acct-one"));
        rows[3].account_ref = Some(crate::run_attribution::account_ref("gone"));
        rows[4].agent = Some("codex".into());
        let ctx = SpendContext {
            projects: HashMap::from([("p1".into(), "Alpha".into())]),
            sessions: HashMap::from([
                ("coo".into(), session("COO", None, None)),
                ("sa".into(), session("Sub A", Some("coo"), Some("subagent"))),
                ("sb".into(), session("Sub B", Some("sa"), Some("subagent"))),
                ("solo".into(), session("Solo", None, None)),
                ("sb2".into(), session("Sub B, continued", None, None)),
            ]),
            accounts: HashMap::from([(
                crate::run_attribution::account_ref("acct-one"),
                "Work".into(),
            )]),
            handover_origin: HashMap::from([("sb2".into(), "sb".into())]),
            ..SpendContext::default()
        };
        let (total, ..) = aggregate(&rows, &HashMap::new());
        let want = (
            total.runs,
            total.input_tokens + total.output_tokens,
            total.reported_cost_usd.unwrap(),
        );
        let spend = spend_buckets(&rows, &ctx);

        let near = |a: (u32, u64, f64), b: (u32, u64, f64)| {
            a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).abs() < 1e-9
        };
        assert!(near(sum(&spend.by_project), want));
        assert!(near(sum(&spend.by_account), want));
        let label = |list: &[UsageBucket], key: &str| {
            list.iter().find(|b| b.key == key).map(|b| b.label.clone())
        };
        assert_eq!(label(&spend.by_project, "p1").as_deref(), Some("Alpha"));
        assert_eq!(label(&spend.by_project, "p2").as_deref(), Some("p2"));
        assert_eq!(
            label(&spend.by_project, "none").as_deref(),
            Some("No project")
        );
        assert_eq!(
            label(&spend.by_account, "login").as_deref(),
            Some("Claude Code login")
        );
        let acct = crate::run_attribution::account_ref("acct-one");
        assert_eq!(label(&spend.by_account, &acct).as_deref(), Some("Work"));
        let gone = crate::run_attribution::account_ref("gone");
        assert_eq!(
            label(&spend.by_account, &gone).as_deref(),
            Some("Claude account (removed)")
        );
        assert_eq!(
            label(&spend.by_account, "agent:codex").as_deref(),
            Some("Codex (its own login)")
        );

        // The lead holds itself, both subagents and the consultation.
        let lead = spend.by_lead.iter().find(|b| b.key == "coo").unwrap();
        assert_eq!(lead.label, "COO");
        assert_eq!((lead.runs, lead.input_tokens), (5, 139));
        assert_eq!(spend.by_lead.len(), 2);
        let (workflow_runs, workflow_tokens, _) = (2, 53 + 2, 0.0);
        let lead_sum = sum(&spend.by_lead);
        assert_eq!(lead_sum.0 + workflow_runs, want.0);
        assert_eq!(lead_sum.1 + workflow_tokens, want.1);

        // Each subagent is listed once, with its own usage.
        assert_eq!(spend.by_subagent.len(), 2);
        let sub_b = spend.by_subagent.iter().find(|b| b.key == "sb").unwrap();
        assert_eq!((sub_b.runs, sub_b.input_tokens), (2, 24));
        let sub_a = spend.by_subagent.iter().find(|b| b.key == "sa").unwrap();
        assert_eq!((sub_a.label.as_str(), sub_a.input_tokens), ("Sub A", 10));
    }

    #[test]
    fn spend_buckets_sum_subagents_past_fifty_into_other_and_survive_a_cycle() {
        let mut rows = Vec::new();
        let mut sessions = HashMap::from([("lead".to_string(), session("Lead", None, None))]);
        for i in 0..51_i64 {
            let key = format!("s{i:02}");
            sessions.insert(key.clone(), session(&key, Some("lead"), Some("subagent")));
            rows.push(spend_row(
                &format!("r{i}"),
                "chat",
                Some(&key),
                100 + i,
                0.0,
            ));
        }
        // The smallest is s00 (100 tokens); a cycle never loops.
        sessions.insert("c1".into(), session("c1", Some("c2"), Some("subagent")));
        sessions.insert("c2".into(), session("c2", Some("c1"), Some("subagent")));
        rows.push(spend_row("cyc", "chat", Some("c1"), 1_000, 0.0));
        let ctx = SpendContext {
            sessions,
            ..SpendContext::default()
        };
        let spend = spend_buckets(&rows, &ctx);
        assert_eq!(spend.by_subagent.len(), 51);
        let other = spend.by_subagent.last().unwrap();
        assert_eq!(
            (other.key.as_str(), other.label.as_str()),
            ("other", "Other subagents")
        );
        assert_eq!(other.runs, 2, "the 51st and 52nd by tokens");
        assert_eq!(spend.by_subagent[0].key, "c1");
        assert!(spend
            .by_lead
            .iter()
            .any(|b| b.key == "lead" && b.runs == 51));
    }

    // ── Settings PR 8: Usage in detail ────────────────────────────────────

    fn detail_row(key: &str, agent: &str, model: &str, cost: Option<f64>, tokens: i64) -> UsageRow {
        let mut r = row(
            "chat",
            "success",
            if cost.is_some() {
                "reported"
            } else {
                "unpriced"
            },
        );
        r.run_key = key.into();
        r.agent = Some(agent.into());
        r.model = Some(model.into());
        r.cost_usd = cost;
        r.input_tokens = Some(tokens);
        r.output_tokens = Some(1);
        r
    }

    fn sums(buckets: &[UsageBucket]) -> (u32, u64, f64) {
        (
            buckets.iter().map(|b| b.runs).sum(),
            buckets
                .iter()
                .map(|b| b.input_tokens + b.cached_input_tokens + b.output_tokens)
                .sum(),
            buckets
                .iter()
                .map(|b| b.reported_cost_usd.unwrap_or(0.0) + b.estimated_cost_usd.unwrap_or(0.0))
                .sum(),
        )
    }

    fn fixture_ledger() -> Vec<UsageRow> {
        let acct = |id: &str| Some(crate::run_attribution::account_ref(id));
        let mut rows = vec![
            detail_row("c1", "claude", "claude-opus-4-7", Some(1.25), 1_000),
            detail_row("c2", "claude", "claude-sonnet-4-6", Some(0.50), 400),
            detail_row("c3", "claude", "claude-opus-4-7", Some(2.00), 2_000),
            detail_row("x1", "codex", "gpt-6-astra", None, 900),
            detail_row("g1", "grok", "grok-5", None, 300),
            detail_row("a1", "antigravity", "gemini-3-pro", None, 0),
            detail_row("q1", "claude", "qwen3.8-27b", None, 700),
        ];
        rows[0].account_ref = acct("work");
        rows[1].account_ref = acct("home");
        rows[2].account_ref = acct("work");
        rows[0].workspace_id = Some("payroll".into());
        rows[3].workspace_id = Some("payroll".into());
        rows[4].workspace_id = Some("hollow".into());
        // Antigravity reports no usage: a run with no tokens, metered as unknown.
        rows[5].input_tokens = None;
        rows[5].output_tokens = None;
        rows[5].cost_status = "unknown".into();
        // A self-hosted model rides the Claude harness, billed by GPU time.
        rows[6].billing = Billing::SelfHosted.as_str().into();
        rows[6].cost_status = "unpriced".into();
        rows
    }

    #[test]
    fn usage_detail_sums_by_model_account_and_workspace_to_each_agent_and_the_total() {
        let rows = fixture_ledger();
        let (total, ..) = aggregate(&rows, &HashMap::new());
        let want = sums(std::slice::from_ref(&total));
        let detail = detail_buckets(&rows);
        let spend = spend_buckets(&rows, &SpendContext::default());
        let near = |a: (u32, u64, f64), b: (u32, u64, f64)| {
            a.0 == b.0 && a.1 == b.1 && (a.2 - b.2).abs() < 1e-9
        };
        for (name, list) in [
            ("agents", &detail.agents),
            ("by_model", &detail.by_model),
            ("outcomes", &detail.outcomes),
            ("by_account", &spend.by_account),
            ("by_project", &spend.by_project),
        ] {
            assert!(near(sums(list), want), "{name} sums to the total");
        }
        // Each agent tab: its model, account and workspace lists sum to its own total.
        for agent in detail.agents.iter().map(|b| b.key.clone()) {
            let mine: Vec<UsageRow> = rows
                .iter()
                .filter(|r| agent_family(r) == agent)
                .cloned()
                .collect();
            let (own, ..) = aggregate(&mine, &HashMap::new());
            let own = sums(std::slice::from_ref(&own));
            let tab = detail.agents.iter().find(|b| b.key == agent).unwrap();
            assert!(near(sums(std::slice::from_ref(tab)), own), "{agent} tab");
            let d = detail_buckets(&mine);
            let s = spend_buckets(&mine, &SpendContext::default());
            for list in [&d.by_model, &d.outcomes, &s.by_account, &s.by_project] {
                assert!(near(sums(list), own), "{agent} lists sum to its total");
            }
        }
        let claude = detail.agents.iter().find(|b| b.key == "claude").unwrap();
        assert_eq!((claude.runs, claude.reported_cost_usd), (3, Some(3.75)));
        let opus = detail
            .by_model
            .iter()
            .find(|b| b.key == "claude-opus-4-7")
            .unwrap();
        assert_eq!((opus.runs, opus.reported_cost_usd), (2, Some(3.25)));
        let work = crate::run_attribution::account_ref("work");
        let work = spend.by_account.iter().find(|b| b.key == work).unwrap();
        assert_eq!(work.runs, 2);
        let payroll = spend
            .by_project
            .iter()
            .find(|b| b.key == "payroll")
            .unwrap();
        assert_eq!(payroll.runs, 2);
    }

    #[test]
    fn antigravity_counts_runs_with_no_tokens_and_self_hosted_stays_out_of_cost() {
        let detail = detail_buckets(&fixture_ledger());
        let anti = detail
            .agents
            .iter()
            .find(|b| b.key == "antigravity")
            .unwrap();
        assert_eq!(anti.label, "Antigravity");
        assert_eq!((anti.runs, anti.unknown), (1, 1));
        assert_eq!(anti.input_tokens + anti.output_tokens, 0);
        assert_eq!(
            (anti.reported_cost_usd, anti.estimated_cost_usd),
            (None, None)
        );

        let own = detail
            .agents
            .iter()
            .find(|b| b.key == "self-hosted")
            .unwrap();
        assert_eq!(
            (own.label.as_str(), own.runs, own.gpu_time),
            ("Self-hosted", 1, 1)
        );
        assert_eq!(own.input_tokens, 700, "its tokens count");
        assert_eq!(
            (own.reported_cost_usd, own.estimated_cost_usd, own.unpriced),
            (None, None, 0)
        );
        let claude = detail.agents.iter().find(|b| b.key == "claude").unwrap();
        assert_eq!(claude.runs, 3, "the self-hosted run is not Claude's");
        let qwen = detail
            .by_model
            .iter()
            .find(|b| b.key == "qwen3.8-27b")
            .unwrap();
        assert_eq!(
            (qwen.reported_cost_usd, qwen.estimated_cost_usd),
            (None, None)
        );
    }

    #[test]
    fn top_conversations_keep_the_ten_costliest_by_session_key() {
        let leads: Vec<UsageBucket> = (0..12)
            .map(|i| UsageBucket {
                key: format!("s-{i:02}"),
                label: format!("Chat {i}"),
                runs: 1,
                reported_cost_usd: Some(f64::from(i)),
                ..UsageBucket::default()
            })
            .chain(std::iter::once(UsageBucket {
                key: "none".into(),
                label: "No conversation".into(),
                reported_cost_usd: Some(99.0),
                ..UsageBucket::default()
            }))
            .collect();
        let top = top_conversations(&leads);
        assert_eq!(top.len(), 10);
        assert_eq!(top[0].key, "s-11");
        assert!(top
            .iter()
            .all(|b| b.key != "none" && b.key != "s-00" && b.key != "s-01"));
    }

    #[test]
    fn last_30_days_starts_at_local_midnight_29_days_back() {
        let now = Utc.with_ymd_and_hms(2026, 11, 12, 15, 30, 0).unwrap();
        let start = period_start_in(UsagePeriod::Last30Days, &now);
        assert_eq!(start, Utc.with_ymd_and_hms(2026, 10, 14, 0, 0, 0).unwrap());
        assert_eq!(UsagePeriod::parse("last30"), Some(UsagePeriod::Last30Days));
        assert_eq!(
            UsagePeriod::parse("week").map(UsagePeriod::as_str),
            Some("week")
        );
        assert_eq!(UsagePeriod::parse("year"), None);
    }

    /// A failed-over turn: two attempts, two rows, two accounts. Each keeps
    /// its account in By account, and the first counts as moved on.
    #[test]
    fn claude_account_attribution_survives_a_failover_run() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let attempt = |account: &str, outcome: LedgerOutcome| {
            let mut cmd = tokio::process::Command::new("/bin/true");
            cmd.env(crate::claude_accounts::ACCOUNT_ENV, account);
            let mut run =
                UsageRun::begin(meter(vault, Surface::Chat), Some(AgentKind::Claude), None);
            run.launched(&cmd);
            let key = run.key().to_string();
            let _ = run.finish(outcome);
            key
        };
        let first = attempt("admin-1", LedgerOutcome::Error);
        let second = attempt("tech-2", LedgerOutcome::Success);
        let rows = session_store::usage_rows_since(vault, "2000-01-01T00:00:00.000Z").unwrap();
        assert_eq!(rows.len(), 2);
        let ctx = SpendContext {
            accounts: HashMap::from([
                (
                    crate::run_attribution::account_ref("admin-1"),
                    "Admin".to_string(),
                ),
                (
                    crate::run_attribution::account_ref("tech-2"),
                    "Tech".to_string(),
                ),
            ]),
            ..SpendContext::default()
        };
        let spend = spend_buckets(&rows, &ctx);
        let labels: Vec<(String, u32)> = spend
            .by_account
            .iter()
            .map(|b| (b.label.clone(), b.runs))
            .collect();
        assert!(labels.contains(&("Admin".to_string(), 1)), "{labels:?}");
        assert!(labels.contains(&("Tech".to_string(), 1)), "{labels:?}");
        let detail = detail_buckets(&rows);
        let outcome = |key: &str| {
            detail
                .outcomes
                .iter()
                .find(|b| b.key == key)
                .map(|b| b.runs)
        };
        assert_eq!(outcome("moved"), Some(1));
        assert_eq!(outcome("finished"), Some(1));
        assert!(moved_runs(&rows).contains(&first));
        assert!(!moved_runs(&rows).contains(&second));
    }

    /// With an agent filter, every breakdown but `agents` holds that agent's
    /// runs only; `agents` still lists every agent with runs (the tab row).
    #[test]
    fn an_agent_filter_narrows_every_list_but_agents() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        for (kind, model) in [
            (AgentKind::Claude, "claude-opus-4-7"),
            (AgentKind::Codex, "gpt-6-astra"),
        ] {
            let mut run =
                UsageRun::begin(meter(vault, Surface::Chat), Some(kind), Some(model.into()));
            run.launched(&tokio::process::Command::new("/bin/true"));
            let _ = run.finish(LedgerOutcome::Success);
        }
        let all = summary_for(vault, UsagePeriod::Last30Days, None, Local::now()).unwrap();
        assert_eq!(all.total.runs, 2);
        let claude =
            summary_for(vault, UsagePeriod::Last30Days, Some("claude"), Local::now()).unwrap();
        assert_eq!(claude.agent.as_deref(), Some("claude"));
        let keys = |list: &[UsageBucket]| list.iter().map(|b| b.key.clone()).collect::<Vec<_>>();
        assert_eq!(
            keys(&claude.agents),
            keys(&all.agents),
            "agents is never filtered"
        );
        assert!(keys(&claude.agents).contains(&"codex".to_string()));
        assert_eq!(claude.total.runs, 1);
        assert_eq!(keys(&claude.by_agent), vec!["claude".to_string()]);
        assert_eq!(keys(&claude.by_model), vec!["claude-opus-4-7".to_string()]);
        for (name, list) in [
            ("by_surface", &claude.by_surface),
            ("by_model", &claude.by_model),
            ("by_account", &claude.by_account),
            ("by_project", &claude.by_project),
            ("outcomes", &claude.outcomes),
            ("by_agent", &claude.by_agent),
        ] {
            assert_eq!(
                list.iter().map(|b| b.runs).sum::<u32>(),
                1,
                "{name} holds only Claude's run"
            );
        }
        assert!(claude.by_account.iter().all(|b| b.key != "agent:codex"));
        let day_runs: u32 = claude.daily.iter().map(|d| d.total.runs).sum();
        assert_eq!(day_runs, 1);
        assert!(claude
            .daily
            .iter()
            .all(|d| d.by_agent.iter().all(|b| b.key == "claude")));
    }
}
