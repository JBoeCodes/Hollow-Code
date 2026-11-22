//! Agent abstraction for the headless coding CLIs Hollow can drive:
//! Claude Code (`claude`), OpenAI Codex (`codex`), and Antigravity (`agy`).
//!
//! This is the single place that knows, per backend, how to
//!   1. locate the binary ([`find_binary`] / [`is_available`]),
//!   2. build the headless command for a prompt ([`build_command`]), and
//!   3. translate that binary's stdout into the unified [`AgentEvent`]
//!      vocabulary ([`parse_line`]).
//!
//! `chat.rs` and `workflows.rs` keep owning their run lifecycle (lease,
//! cancel registration, persistence) and just map [`AgentEvent`] onto the
//! `chat-event` / `workflow-event` Tauri payloads they already emit. Waiting
//! on the process, stderr, deadlines and ending the whole process tree belong
//! to [`crate::process_group::drive`].
//!
//! The three CLIs diverge in three ways this module hides:
//!   * **Invocation** — `claude -p`, `codex exec`, `agy -p`.
//!   * **Output** — every adapter translates its own structured NDJSON protocol.
//!   * **Permissions** — the UI speaks Claude's four canonical modes
//!     (`acceptEdits` / `auto` / `plan` / `bypassPermissions`); each
//!     adapter maps them to its own sandbox/approval flags.

use crate::process_group;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Which agent CLI backs a chat session or workflow run. Serialized
/// camelCase so the frontend and workflow frontmatter use the plain
/// strings `claude` / `codex` / `antigravity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    Claude,
    Codex,
    Antigravity,
    /// xAI's Grok Build CLI (`grok`), signed in with the user's SuperGrok
    /// login. Its headless stream is Claude Code's stream-json format.
    Grok,
}

impl AgentKind {
    /// The persisted / wire string (`claude` / `codex` / `antigravity` / `grok`).
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Antigravity => "antigravity",
            AgentKind::Grok => "grok",
        }
    }

    /// Human label for UI surfaces, run logs, and teammate result messages.
    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::Antigravity => "Antigravity",
            AgentKind::Grok => "Grok",
        }
    }

    /// Strict parse — `None` for anything unrecognized.
    pub fn from_opt(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(AgentKind::Claude),
            "codex" => Some(AgentKind::Codex),
            // `agy` accepted as a friendly alias for Antigravity.
            "antigravity" | "agy" => Some(AgentKind::Antigravity),
            "grok" => Some(AgentKind::Grok),
            _ => None,
        }
    }

    pub fn all() -> [AgentKind; 4] {
        [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Antigravity,
            AgentKind::Grok,
        ]
    }
}

/// Bare command name used for the `$PATH` fallback at spawn time.
fn bin_name(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        AgentKind::Antigravity => "agy",
        AgentKind::Grok => "grok",
    }
}

/// Concrete install locations to probe before falling back to `$PATH`.
/// A macOS `.app` launched from Finder doesn't inherit the user's shell
/// PATH, so we check the usual spots explicitly (mirrors the original
/// `chat::find_claude_binary`).
fn candidates(kind: AgentKind) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let home = dirs::home_dir();
    match kind {
        AgentKind::Claude => {
            if let Some(h) = &home {
                out.push(h.join(".local/bin/claude"));
                out.push(h.join(".claude/local/claude"));
            }
            out.push(PathBuf::from("/opt/homebrew/bin/claude"));
            out.push(PathBuf::from("/usr/local/bin/claude"));
            out.push(PathBuf::from("/usr/bin/claude"));
        }
        AgentKind::Codex => {
            if let Some(h) = &home {
                out.push(h.join(".local/bin/codex"));
            }
            out.push(PathBuf::from("/opt/homebrew/bin/codex"));
            out.push(PathBuf::from("/usr/local/bin/codex"));
            out.push(PathBuf::from("/usr/bin/codex"));
        }
        AgentKind::Antigravity => {
            if let Some(h) = &home {
                out.push(h.join(".local/bin/agy"));
            }
            out.push(PathBuf::from("/opt/homebrew/bin/agy"));
            out.push(PathBuf::from("/usr/local/bin/agy"));
            out.push(PathBuf::from("/usr/bin/agy"));
        }
        AgentKind::Grok => {
            if let Some(h) = &home {
                // The installer's own location.
                out.push(h.join(".grok/bin/grok"));
                out.push(h.join(".local/bin/grok"));
            }
            out.push(PathBuf::from("/opt/homebrew/bin/grok"));
            out.push(PathBuf::from("/usr/local/bin/grok"));
        }
    }
    out
}

/// Scan the agents' PATH ([`cli_path`]) for `name` so availability
/// detection is honest even when the binary lives somewhere we don't
/// hard-code, including a folder only the login shell adds.
fn path_lookup(name: &str) -> Option<PathBuf> {
    let path = cli_path();
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// Resolve a real, existing binary for `kind` — concrete candidates first,
/// then a `$PATH` scan. `None` means the CLI isn't installed. A test-bed
/// candidate launched with a fake-agent folder resolves from it first
/// ([`crate::instance::candidate_agent_bin`]).
fn concrete_binary(kind: AgentKind) -> Option<PathBuf> {
    crate::instance::candidate_agent_bin(bin_name(kind))
        .or_else(|| candidates(kind).into_iter().find(|p| p.exists()))
        .or_else(|| path_lookup(bin_name(kind)))
}

/// The installed binary for `kind`, or `None` when the CLI isn't installed
/// (no bare-name fallback): the Codex helper probe (`codex_probe`).
pub(crate) fn installed_binary(kind: AgentKind) -> Option<PathBuf> {
    concrete_binary(kind)
}

/// Locate the binary for `kind`, falling back to the bare command name so
/// a `$PATH`-only setup (e.g. under `tauri dev`) still spawns.
pub fn find_binary(kind: AgentKind) -> Option<PathBuf> {
    concrete_binary(kind).or_else(|| Some(PathBuf::from(bin_name(kind))))
}

/// The PATH every agent CLI Hollow starts gets. A Finder- or Dock-launched
/// Hollow inherits the bare system PATH, and `codex` is a `#!/usr/bin/env
/// node` script, so without the user's tool folders it exits 127 ("env:
/// node: No such file or directory") before doing anything. Once the login
/// shell's PATH has been read ([`crate::shell_path`]), its folders (gcloud,
/// bun, anything `~/.zshrc` adds) come after Hollow's tool folders and before
/// the inherited and system ones; until then, or when it couldn't be read,
/// this is exactly [`crate::self_install::tool_path`]. Joined from the same
/// tagged merge Settings, Advanced lists ([`crate::shell_path::agent_tool_path`]).
pub fn cli_path() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    let inherited = std::env::var("PATH").ok();
    let shell = crate::shell_path::folders().unwrap_or_default();
    crate::shell_path::join(&cli_path_dirs_with(shell, inherited.as_deref(), &home))
}

/// [`cli_path`] folder by folder, each with where it came from, given the
/// login shell's folders (none until it has answered).
pub(crate) fn cli_path_dirs_with(
    shell: &[String],
    inherited: Option<&str>,
    home: &std::path::Path,
) -> Vec<crate::shell_path::PathDir> {
    let split = |path: &str| -> Vec<String> {
        path.split(':')
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .collect()
    };
    let system = split(crate::self_install::SYSTEM_PATH);
    // tool_path with nothing inherited is Hollow's own folders + the system ones.
    let hollow: Vec<String> = split(&crate::self_install::tool_path(None, home))
        .into_iter()
        .filter(|d| !system.contains(d))
        .collect();
    crate::shell_path::merge(
        &hollow,
        shell,
        &split(inherited.unwrap_or_default()),
        &system,
    )
}

/// True when the CLI for `kind` actually resolves on this machine.
pub fn is_available(kind: AgentKind) -> bool {
    concrete_binary(kind).is_some()
}

/// Track U: whether the Claude CLI takes `--effort ultracode`, keyed like
/// [`MAX_BUDGET_PROBES`] so an upgrade is probed again.
static ULTRACODE_PROBES: once_cell::sync::Lazy<
    tokio::sync::Mutex<std::collections::HashMap<(PathBuf, Option<std::time::SystemTime>), bool>>,
> = once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

/// Longest the ultracode probe may take.
const ULTRACODE_PROBE_LIMIT: Duration = Duration::from_secs(20);

/// What Claude Code 2.1.286 prints for an effort it doesn't know, before
/// ignoring it and running at its default.
const UNKNOWN_EFFORT: &str = "Unknown --effort value";

/// Whether the installed Claude CLI takes `--effort ultracode` (merged plan
/// Track U), recorded for `effort::resolve` ([`crate::effort::set_claude_ultracode`]).
/// `--help` doesn't list it (2.1.286 shows five levels), so this asks the
/// CLI itself, for free; see [`claude_ultracode_supported_at`]. Only when a
/// chat asks for ultracode; GUI async contexts only.
pub async fn claude_ultracode_supported() -> bool {
    let supported = match find_binary(AgentKind::Claude) {
        Some(bin) => claude_ultracode_supported_at(&bin).await,
        None => false,
    };
    crate::effort::set_claude_ultracode(supported);
    supported
}

/// [`claude_ultracode_supported`] against an explicit binary (tests pass a
/// shell script). Runs `-p --effort ultracode` with a model id no CLI knows,
/// which 2.1.286 rejects before any request (`duration_api_ms: 0`, cost 0;
/// probed 2026-10-02); an API that saw it would answer 404 before any
/// inference. (Pointing `ANTHROPIC_BASE_URL` at a closed port instead made
/// the CLI hang retrying, so the probe could never pass.) It prints
/// [`UNKNOWN_EFFORT`] for a level it doesn't take, so: supported when the
/// CLI got as far as the model and said nothing about the effort. Anything
/// else (no output, a timeout, an error) counts as unsupported, and the run
/// degrades to xhigh with a note rather than sending a flag the CLI may not
/// take. In the system temp directory, outside the vault.
pub(crate) async fn claude_ultracode_supported_at(bin: &std::path::Path) -> bool {
    let canonical = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let modified = std::fs::metadata(&canonical)
        .and_then(|meta| meta.modified())
        .ok();
    let key = (canonical.clone(), modified);
    let mut probes = ULTRACODE_PROBES.lock().await;
    if let Some(known) = probes.get(&key) {
        return *known;
    }
    let mut cmd = Command::new(&canonical);
    cmd.args([
        "-p",
        "probe",
        "--effort",
        crate::effort::ULTRACODE,
        "--model",
        "hollow-probe-no-such-model",
        "--tools",
        "",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--output-format",
        "json",
    ])
    .current_dir(std::env::temp_dir());
    let supported =
        match process_group::run_to_completion(&mut cmd, None, ULTRACODE_PROBE_LIMIT, None).await {
            Ok(out) if !out.timed_out => {
                ultracode_probe_says_supported(&out.stdout, out.stderr.as_bytes())
            }
            Ok(_) => false,
            Err(error) => {
                eprintln!(
                    "[effort] ultracode probe failed: {}",
                    crate::claude_runner::redact(&error.to_string())
                );
                false
            }
        };
    if !supported {
        eprintln!("[effort] this Claude CLI doesn't take --effort ultracode; ultracode chats run at xhigh");
    }
    probes.insert(key, supported);
    supported
}

/// The probe's verdict from its output: the CLI reached the model check
/// (it named the unknown model, or wrote a result) and didn't reject the
/// effort.
fn ultracode_probe_says_supported(stdout: &[u8], stderr: &[u8]) -> bool {
    let out = String::from_utf8_lossy(stdout);
    let err = String::from_utf8_lossy(stderr);
    let rejected = out.contains(UNKNOWN_EFFORT) || err.contains(UNKNOWN_EFFORT);
    let reached = [&out, &err]
        .iter()
        .any(|text| text.contains("hollow-probe-no-such-model") || text.contains("\"session_id\""));
    reached && !rejected
}

/// Paperclip PR 8b: probe results for Claude's `--max-budget-usd`, keyed by
/// the binary's canonical path and modification time, so an update is
/// probed again. The async mutex makes the probe single-flight.
static MAX_BUDGET_PROBES: once_cell::sync::Lazy<
    tokio::sync::Mutex<std::collections::HashMap<(PathBuf, Option<std::time::SystemTime>), bool>>,
> = once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));

/// Longest the `--help` probe may take.
const MAX_BUDGET_PROBE_LIMIT: Duration = Duration::from_secs(5);

/// Whether the installed Claude CLI supports `--max-budget-usd` (Paperclip PR
/// 8b's spending backstop). GUI async contexts only (a workflow's
/// `prepare_spawn`, the heartbeat's `run_beat`), and only when a run has an
/// enforced spending budget. Never `block_on` it. An unavailable backstop is
/// logged, never silent; the run then goes ahead on its token checks and the
/// claim-time preflight.
pub async fn claude_max_budget_supported() -> bool {
    match find_binary(AgentKind::Claude) {
        Some(bin) => claude_max_budget_supported_at(&bin).await,
        None => {
            eprintln!("[budgets] Claude spending backstop unavailable: claude is not installed");
            false
        }
    }
}

/// [`claude_max_budget_supported`] against an explicit binary (tests pass a
/// shell script, so no test ever runs the installed CLI). Runs `<bin>
/// --help` in the system temp directory, outside the vault, so no CLI side
/// file can land in a vault `.claude/` folder while the run's lease is held
/// (PR 5's run guard watches those).
pub(crate) async fn claude_max_budget_supported_at(bin: &std::path::Path) -> bool {
    let canonical = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    let modified = std::fs::metadata(&canonical)
        .and_then(|meta| meta.modified())
        .ok();
    let key = (canonical.clone(), modified);
    let mut probes = MAX_BUDGET_PROBES.lock().await;
    if let Some(known) = probes.get(&key) {
        return *known;
    }
    let mut cmd = Command::new(&canonical);
    cmd.arg("--help").current_dir(std::env::temp_dir());
    let supported = match process_group::run_to_completion(
        &mut cmd,
        None,
        MAX_BUDGET_PROBE_LIMIT,
        None,
    )
    .await
    {
        Ok(out) if out.timed_out => {
            eprintln!("[budgets] Claude spending backstop unavailable: `claude --help` timed out");
            false
        }
        Ok(out) => {
            let help = String::from_utf8_lossy(&out.stdout);
            let found = help.contains("--max-budget-usd");
            if !found {
                eprintln!(
                    "[budgets] Claude spending backstop unavailable: this Claude CLI has no --max-budget-usd"
                );
            }
            found
        }
        Err(error) => {
            eprintln!(
                "[budgets] Claude spending backstop unavailable: {}",
                crate::claude_runner::redact(&error.to_string())
            );
            false
        }
    };
    probes.insert(key, supported);
    supported
}

/// Apply a run's claim-time usage budget to its spec (Paperclip PR 8b),
/// Claude only: partial messages (`usage_stream`) when a token budget
/// watches the run, so its guard reads final per-message usage, and the
/// `--max-budget-usd` spending backstop under an enforced hard-stop spending
/// budget on a fresh non-interactive run, when `probe` (normally
/// [`claude_max_budget_supported`]) says the installed CLI supports it. The
/// probe runs only then. Other agents are left unchanged: they are checked
/// at the claim only. Neither field is a stamped workflow field.
pub async fn apply_run_budget<F, Fut>(
    kind: AgentKind,
    spec: &mut RunSpec,
    budget: &crate::budgets::RunBudget,
    probe: F,
) where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    if kind != AgentKind::Claude {
        return;
    }
    spec.usage_stream = budget.usage_stream();
    let fresh = !spec.interactive && spec.resume.as_deref().is_none_or(str::is_empty);
    spec.max_budget_usd = match budget.max_budget_usd {
        Some(usd) if fresh && probe().await => Some(usd),
        _ => None,
    };
}

/// The agents installed on this machine, in canonical order. Drives the
/// chat agent picker (only installed CLIs are offered).
pub fn available() -> Vec<AgentKind> {
    AgentKind::all()
        .into_iter()
        .filter(|k| is_available(*k))
        .collect()
}

/// What an agent's adapter can do. AC1 (2026-11-17): `steer`, `interrupt`,
/// `fork`, `rollback`, `queue` and `pending_survives_restart` come from one
/// row per agent in [`flags`], the only place that names an agent. A feature
/// asks [`require`] or [`supports`], never the agent's name, and a new agent
/// fits by filling one row. A flag is true only when Hollow's adapter does it
/// today; a protocol that could is written on [`Flags`], not set.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Adapter reports an explicit plan proposal through ExitPlanMode.
    pub plan_proposals: bool,
    /// Adapter reports completed, non-redacted reasoning text.
    pub reasoning: bool,
    /// Adapter can multiplex turns on an account-scoped, supervised server.
    pub shared_server: bool,
    pub reports_context: bool,
    pub model_selection: bool,
    pub session_resume: bool,
    pub tool_restrictions: bool,
    pub owned_mcp: bool,
    pub structured_output: bool,
    /// Paperclip PR 14a: [`effort_levels`] (serialized `effortLevels`);
    /// empty when the agent has no effort setting.
    pub effort_levels: Vec<String>,
    /// Permission modes this agent can't run, each with the reason the mode
    /// pickers show ([`unavailable_mode`]; serialized `unavailableModes`).
    pub unavailable_modes: std::collections::BTreeMap<String, String>,
    /// Why this agent can't use the desktop (see-and-act) tools in a chat,
    /// shown beside it in the picker (CU4; `None` when it can).
    pub desktop_unavailable: Option<String>,
    /// Adapter can take a message into a running turn (`steering::support_for`
    /// still gates a Codex version below its qualified floor on top of this).
    pub steer: bool,
    /// Adapter can stop a running turn and keep its session resumable.
    pub interrupt: bool,
    /// Adapter can start a branch of a session from some point in it.
    pub fork: bool,
    /// Adapter can rewind a session to an earlier turn.
    pub rollback: bool,
    /// Hollow can hold a message for this agent until its turn ends
    /// (Queue for after, Stop and send).
    pub queue: bool,
    /// Messages held for this agent are saved and sent after a Hollow restart.
    pub pending_survives_restart: bool,
    /// Why each false flag above is false, in a plain sentence naming the
    /// feature and the agent ([`unsupported_reason`]; serialized
    /// `unsupportedReasons`, keyed by the flag's name). Empty when every flag
    /// is true. A feature refuses with this text before any side effect.
    pub unsupported_reasons: std::collections::BTreeMap<String, String>,
}

/// The adapter features AC1 flags. A feature that depends on one asks
/// [`require`] with it, so an agent that lacks it is refused with a reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Steer,
    Interrupt,
    Fork,
    Rollback,
    Queue,
    PendingSurvivesRestart,
}

impl Feature {
    /// Every feature, in the order the wire lists them.
    pub const ALL: [Feature; 6] = [
        Feature::Steer,
        Feature::Interrupt,
        Feature::Fork,
        Feature::Rollback,
        Feature::Queue,
        Feature::PendingSurvivesRestart,
    ];

    /// The capability's wire name (its key in `unsupportedReasons`).
    pub fn key(self) -> &'static str {
        match self {
            Feature::Steer => "steer",
            Feature::Interrupt => "interrupt",
            Feature::Fork => "fork",
            Feature::Rollback => "rollback",
            Feature::Queue => "queue",
            Feature::PendingSurvivesRestart => "pendingSurvivesRestart",
        }
    }

    /// The plain sentence for an agent that lacks this feature.
    fn refusal(self, label: &str) -> String {
        match self {
            Feature::Steer => format!(
                "{label} can't take a message mid-turn. Queue it for after, or Stop and send restarts it with your message."
            ),
            Feature::Interrupt => format!(
                "Hollow can't stop a running {label} turn and keep its session; it would have to end the whole conversation."
            ),
            Feature::Fork => format!(
                "Hollow can't branch a {label} conversation yet: the adapter doesn't wire a session fork."
            ),
            Feature::Rollback => format!(
                "Hollow can't rewind a {label} conversation yet: the adapter doesn't wire a session rollback."
            ),
            Feature::Queue => format!(
                "Hollow can't hold a message for {label} until its turn ends; send it when the turn is done."
            ),
            Feature::PendingSurvivesRestart => format!(
                "A message waiting for {label} would be lost if Hollow restarts; send it before quitting."
            ),
        }
    }
}

/// One agent's row of the AC1 capability table. Every agent states every
/// flag; there is no default, so a new agent can't be added without deciding.
///
/// Protocol facts that are not flags yet (the adapter doesn't wire them), for
/// the rows that will flip them (CP1 turn checkpoints, CH1 branch a chat):
/// Claude Code has `--resume <id> --fork-session` (fork) and no rewind;
/// Codex app-server has `thread/fork` (fork) and `thread/rollback` (rollback)
/// and its `turn/interrupt` ([`crate::codex_app_server`] writes it, but a
/// chat's Stop ends the process group instead, as for every agent); Grok has
/// ACP (`_x.ai/interject`, which `grok -p` doesn't use); Antigravity has
/// nothing for either. Hollow's own chat forks (offline copies, subagents) are
/// Hollow-side copies that start a fresh native session, so they don't count.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Flags {
    /// Claude: stream-json input on stdin. Codex: `turn/steer` over
    /// `codex app-server`. Grok: `grok -p` has no input channel. Antigravity:
    /// its stream input only queues a whole next turn
    /// (docs/reference/agents.md, "Steer a running turn").
    pub(crate) steer: bool,
    /// Hollow's Stop ends the turn's process group for every agent and the
    /// session id stays resumable (`session_resume`).
    pub(crate) interrupt: bool,
    pub(crate) fork: bool,
    pub(crate) rollback: bool,
    /// Queue for after and Stop and send are Hollow-side held sends
    /// (`steering::hold`), independent of the agent.
    pub(crate) queue: bool,
    /// Held sends are saved to `held-sends.json` after every change and the
    /// next launch sends them (`steering::with_held`), independent of the agent.
    pub(crate) pending_survives_restart: bool,
}

/// The one table: the only place the flags name an agent.
const fn flags(kind: AgentKind) -> Flags {
    match kind {
        AgentKind::Claude => Flags {
            steer: true,
            interrupt: true,
            fork: false,
            rollback: false,
            queue: true,
            pending_survives_restart: true,
        },
        AgentKind::Codex => Flags {
            steer: true,
            interrupt: true,
            fork: false,
            rollback: false,
            queue: true,
            pending_survives_restart: true,
        },
        AgentKind::Grok => Flags {
            steer: false,
            interrupt: true,
            fork: false,
            rollback: false,
            queue: true,
            pending_survives_restart: true,
        },
        AgentKind::Antigravity => Flags {
            steer: false,
            interrupt: true,
            fork: false,
            rollback: false,
            queue: true,
            pending_survives_restart: true,
        },
    }
}

fn row_supports(row: Flags, feature: Feature) -> bool {
    match feature {
        Feature::Steer => row.steer,
        Feature::Interrupt => row.interrupt,
        Feature::Fork => row.fork,
        Feature::Rollback => row.rollback,
        Feature::Queue => row.queue,
        Feature::PendingSurvivesRestart => row.pending_survives_restart,
    }
}

/// Whether `kind`'s adapter has `feature` today.
pub fn supports(kind: AgentKind, feature: Feature) -> bool {
    row_supports(flags(kind), feature)
}

/// The plain-sentence reason `kind` lacks `feature`, or `None` when it has it.
pub fn unsupported_reason(kind: AgentKind, feature: Feature) -> Option<String> {
    reason_in(flags(kind), kind.label(), feature)
}

pub(crate) fn reason_in(row: Flags, label: &str, feature: Feature) -> Option<String> {
    (!row_supports(row, feature)).then(|| feature.refusal(label))
}

/// Refuse `feature` for `kind` with its reason. Pure: call it before any
/// side effect (a stored row, a spawned process, a held send) so a refusal
/// leaves nothing behind.
pub fn require(kind: AgentKind, feature: Feature) -> Result<(), String> {
    match unsupported_reason(kind, feature) {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub capabilities: AgentCapabilities,
}

pub fn capabilities(kind: AgentKind) -> AgentCapabilities {
    AgentCapabilities {
        plan_proposals: matches!(kind, AgentKind::Claude | AgentKind::Grok),
        reasoning: matches!(kind, AgentKind::Claude | AgentKind::Codex | AgentKind::Grok),
        shared_server: kind == AgentKind::Codex,
        reports_context: true,
        model_selection: true,
        session_resume: true,
        tool_restrictions: kind == AgentKind::Claude,
        // Grok has no per-run MCP config flag. Antigravity takes servers in
        // `session/new` over ACP (2026-11-20), per chat; its one-shots stay
        // on `agy -p` and get none ([`owned_mcp_for`]).
        owned_mcp: match kind {
            AgentKind::Claude | AgentKind::Codex => true,
            AgentKind::Antigravity => !antigravity_headless_forced(),
            AgentKind::Grok => false,
        },
        structured_output: true,
        steer: supports(kind, Feature::Steer),
        interrupt: supports(kind, Feature::Interrupt),
        fork: supports(kind, Feature::Fork),
        rollback: supports(kind, Feature::Rollback),
        queue: supports(kind, Feature::Queue),
        pending_survives_restart: supports(kind, Feature::PendingSurvivesRestart),
        unsupported_reasons: Feature::ALL
            .into_iter()
            .filter_map(|f| unsupported_reason(kind, f).map(|why| (f.key().to_string(), why)))
            .collect(),
        effort_levels: effort_levels(kind).iter().map(|s| s.to_string()).collect(),
        desktop_unavailable: crate::computer_cli::desktop_unavailable(kind).map(str::to_string),
        unavailable_modes: [
            "plan",
            "default",
            "acceptEdits",
            "auto",
            "bypassPermissions",
        ]
        .into_iter()
        .filter_map(|perm| {
            unavailable_mode(kind, perm).map(|why| (perm.to_string(), why.to_string()))
        })
        .collect(),
    }
}

/// Conservative physical footprint estimate including owned MCP helpers.
/// The pump consumes this adapter estimate without choosing a provider.
pub(crate) fn turn_memory_bytes(kind: Option<AgentKind>) -> u64 {
    match kind {
        Some(AgentKind::Codex) => 180_000_000,
        _ => 250_000_000,
    }
}

// Antigravity's `auto` (re-enabled 2026-11-20 on agy 1.5.13). It was refused
// from merged plan slice 2 because agy 1.2.15's `--sandbox
// --dangerously-skip-permissions` pair confined only its shell: its file
// tools wrote outside the working folder (probe matrix in
// docs/history/CLAUDE-archive-2026-10-agents.md). agy 1.5.13 has its own
// `--mode auto` (edits in the workspace and safe commands approved, the rest
// asked) and Hollow's probe, rerun against it, held: with the workspace and
// the oracle folder outside the system temp tree, a shell write, the write
// tool, an edit of an existing file, `../` and a symlink out all failed
// (`operation not permitted`, or `denied_actions: [write_file]`), while a
// write inside the folder worked, headless and over ACP. System temp folders
// stay writable, as for Claude's and Codex's sandboxes. A run still never
// gets the `--dangerously-skip-permissions` flag outside bypass.
/// The reason `kind` can't run in permission mode `perm`, or `None` when it
/// can. [`validate_run_spec`] refuses such a run with this text (a chat,
/// automation or teammate fails closed and says why; never a wider or a
/// silently narrower mode), and [`capabilities`] hands it to the pickers.
pub fn unavailable_mode(_kind: AgentKind, _perm: &str) -> Option<&'static str> {
    // Every agent runs every mode since Antigravity's Auto came back
    // (the Auto probe above); the hook stays for the next one that can't.
    None
}

/// Whether `kind` runs `perm` with no sandbox at all, as it runs bypass:
/// Grok's `auto` turns its sandbox `off` ([`grok_sandbox`]), so Hollow ranks
/// a request for it as bypass when it decides whether a start needs the
/// person's card (`app_sessions::escalation_for`).
pub fn auto_is_unsandboxed(kind: AgentKind, perm: &str) -> bool {
    kind == AgentKind::Grok && perm.trim() == "auto"
}

/// How far a Hollow run of an agent reaches in a permission mode, weakest
/// first (Q-R3, 2026-11-13): one axis of [`ModeCapability`]. Mode names mean
/// different things across agents: Claude's headless `default` declines
/// every edit and command, while Codex's `default` runs the workspace-write
/// sandbox. The levels follow the teammate cap's capability model
/// (`team_mcp` tests, `Effective`: writes in the working folder, writes
/// outside it, no check at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModeReach {
    /// Reads and answers; changes nothing beyond the agent's own temp folders.
    ReadOnly,
    /// Changes files in its working folder: edits, or commands in a sandbox
    /// held to that folder.
    Folder,
    /// Also beyond the working folder, each action judged by the agent's own
    /// classifier or reviewer.
    Checked,
    /// Anything on this Mac, with no sandbox and no check.
    Unchecked,
}

/// What a Hollow run of an agent can do in a permission mode, on two axes
/// compared each on its own (Q-R3 round 2): how far it reaches, and whether
/// it runs commands that can change things. Claude's acceptEdits edits the
/// folder but declines every command that needs permission; Codex's
/// acceptEdits runs shell commands in the folder's sandbox: same reach, but
/// Codex's also runs commands, so Claude's is within Codex's and Codex's is
/// not within Claude's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeCapability {
    pub reach: ModeReach,
    /// Runs commands that can change things (in a sandbox or not). A command
    /// in a read-only sandbox can't, so it doesn't count.
    pub commands: bool,
}

impl ModeCapability {
    /// Whether this can do nothing `other` can't: both axes at most.
    pub fn within(self, other: ModeCapability) -> bool {
        self.reach <= other.reach && self.commands <= other.commands
    }

    /// What both can do: the lower of each axis.
    pub fn meet(self, other: ModeCapability) -> ModeCapability {
        ModeCapability {
            reach: self.reach.min(other.reach),
            commands: self.commands && other.commands,
        }
    }

    /// What either can do: the higher of each axis.
    pub fn join(self, other: ModeCapability) -> ModeCapability {
        ModeCapability {
            reach: self.reach.max(other.reach),
            commands: self.commands || other.commands,
        }
    }
}

/// [`ModeCapability`] of `kind` in `perm` (normalized), from the argv each
/// mode gets (docs/reference/agent-tools.md "Permission modes across
/// agents"): Claude edits from acceptEdits and runs commands from auto;
/// Codex writes and runs commands in the workspace sandbox from `default`
/// (`codex_sandbox`) and has a reviewer in auto; Grok runs every command,
/// held by its sandbox profile (`grok_sandbox`: read-only, workspace, then
/// off for auto and bypass); Antigravity's own auto holds writes to the
/// working folder (the Auto probe above); its other modes are listed
/// read only, as before (agy 1.5.13's headless accept-edits and no-mode runs
/// write inside the folder: a conservative reading, noted in the build log
/// 2026-11-20). Bypass is unchecked everywhere.
pub fn mode_capability(kind: AgentKind, perm: &str) -> ModeCapability {
    use ModeReach::{Checked, Folder, ReadOnly, Unchecked};
    let perm = crate::app_mcp::normalize_perm(perm);
    let (reach, commands) = match (kind, perm) {
        (_, "bypassPermissions") => (Unchecked, true),
        (_, "plan") => (ReadOnly, false),
        (AgentKind::Claude, "acceptEdits") => (Folder, false),
        (AgentKind::Claude | AgentKind::Codex | AgentKind::Antigravity, "auto") => (Checked, true),
        (AgentKind::Claude, _) => (ReadOnly, false),
        (AgentKind::Codex, _) => (Folder, true),
        (AgentKind::Grok, "auto") => (Unchecked, true),
        (AgentKind::Grok, "acceptEdits") => (Folder, true),
        (AgentKind::Grok, _) => (ReadOnly, false),
        (AgentKind::Antigravity, _) => (ReadOnly, false),
    };
    ModeCapability { reach, commands }
}

/// What Hollow's own layer allows in `perm`, whatever the agent: every card
/// in plan, default and acceptEdits (`app_actions::asks_in_mode`), plan's
/// read-only tools below them, fewer cards and no preapproved connector in
/// auto, none of either in bypass.
fn hollow_layer(perm: &str) -> u8 {
    match crate::app_mcp::normalize_perm(perm) {
        "plan" => 0,
        "auto" => 2,
        "bypassPermissions" => 3,
        _ => 1,
    }
}

/// What a parent's `mode` can do on its agent `from`. When the agent is
/// unknown, or can't run the mode, the most restrictive reading: what every
/// agent that can run a mode of that name can do ([`ModeCapability::meet`]),
/// so `acceptEdits` from an unknown agent reads as Antigravity's (read only,
/// no commands).
pub fn parent_capability(from: Option<AgentKind>, mode: &str) -> ModeCapability {
    let mode = crate::app_mcp::normalize_perm(mode);
    match from.filter(|kind| unavailable_mode(*kind, mode).is_none()) {
        Some(kind) => mode_capability(kind, mode),
        None => AgentKind::all()
            .into_iter()
            .filter(|kind| unavailable_mode(*kind, mode).is_none())
            .map(|kind| mode_capability(kind, mode))
            .reduce(ModeCapability::meet)
            .unwrap_or(mode_capability(AgentKind::Claude, "plan")),
    }
}

/// The best mode of `kind` it can run within `cap`, at most `max_rank`
/// (`app_mcp::perm_rank`) and no looser in Hollow's layer than `mode`: the
/// one that reaches furthest, then runs commands, then `mode` itself, then
/// the strongest. Every agent's `plan` is read-only with no commands, so
/// there is always one.
fn best_mode_within(
    kind: AgentKind,
    mode: &'static str,
    cap: ModeCapability,
    max_rank: usize,
) -> &'static str {
    crate::app_mcp::PERMISSION_MODES
        .into_iter()
        .filter(|m| {
            unavailable_mode(kind, m).is_none()
                && mode_capability(kind, m).within(cap)
                && hollow_layer(m) <= hollow_layer(mode)
                && crate::app_mcp::perm_rank(m) <= max_rank
        })
        .max_by_key(|m| {
            let can = mode_capability(kind, m);
            (
                can.reach,
                can.commands,
                *m == mode,
                crate::app_mcp::perm_rank(m),
            )
        })
        .unwrap_or("plan")
}

/// The mode `kind` runs in place of an inherited `mode` when it may do at
/// most `cap`: among `kind`'s modes it can run within `cap` whose Hollow
/// layer is no looser than `mode`'s, the one that does most
/// ([`best_mode_within`]). It may be stronger by rank than `mode` (Codex's
/// `default` writes the folder, so Claude gets acceptEdits from it).
pub fn capped_mode(kind: AgentKind, mode: &str, cap: ModeCapability) -> &'static str {
    let mode = crate::app_mcp::normalize_perm(mode);
    best_mode_within(
        kind,
        mode,
        cap,
        crate::app_mcp::perm_rank("bypassPermissions"),
    )
}

/// A mode someone named for `kind`, held to `cap`: `mode` exactly when `kind`
/// can run it within `cap`, else the best mode below it that fits. Never
/// raised: a caller asking for less gets what it asked for.
pub fn named_mode_within(kind: AgentKind, mode: &str, cap: ModeCapability) -> &'static str {
    let mode = crate::app_mcp::normalize_perm(mode);
    if unavailable_mode(kind, mode).is_none() && mode_capability(kind, mode).within(cap) {
        return mode;
    }
    best_mode_within(kind, mode, cap, crate::app_mcp::perm_rank(mode))
}

/// The mode `kind` runs when `mode` was inherited from a parent on agent
/// `from` rather than picked for it (a started chat taking its parent's
/// mode, a handover successor on another agent, the phone's new chat taking
/// the window's new-chat mode, a teammate): never more than the parent's
/// mode could do on either axis (Q-R3, [`capped_mode`] under
/// [`parent_capability`]). So Claude's `default` and `acceptEdits` (no
/// commands) give Codex `plan`; an `auto` gives Grok `acceptEdits` (Grok's
/// auto has its sandbox off, [`auto_is_unsandboxed`]) and Antigravity its own
/// `auto` (held to the working folder, the Auto probe above); and the
/// same agent keeps the mode. A mode someone named for this agent is not inherited and
/// doesn't come here.
pub fn inherited_mode(from: Option<AgentKind>, kind: AgentKind, mode: &str) -> &'static str {
    capped_mode(kind, mode, parent_capability(from, mode))
}

pub fn descriptors() -> Vec<AgentDescriptor> {
    AgentKind::all()
        .into_iter()
        .map(|kind| AgentDescriptor {
            id: kind.as_str().into(),
            label: kind.label().into(),
            available: is_available(kind),
            capabilities: capabilities(kind),
        })
        .collect()
}

/// Who spawns a run, which decides whether it gets its own process group.
/// There is deliberately no default: every call site states its choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessScope {
    /// Spawned by the GUI process: the CLI leads a new process group and
    /// carries a fresh `HOLLOW_RUN_TAG`, so stop and finish can end every
    /// process the run started (see `process_group`).
    GuiOwned,
    /// Spawned inside a Hollow MCP subprocess (`hollow team-mcp`): keep the
    /// caller's group and inherited run tag so the GUI's sweep reaches it.
    /// Never create or signal a group from there.
    Inherited,
}

/// Everything needed to construct a single headless invocation. The same
/// shape serves chat (`interactive = true`: stream token deltas + keep a
/// resumable session) and workflows / batch runs (`interactive = false`:
/// no partial messages, no session persistence).
#[derive(Clone)]
pub struct RunSpec {
    pub prompt: String,
    /// Prior session/thread id to continue. Only honored when `interactive`.
    pub resume: Option<String>,
    pub model: Option<String>,
    /// Canonical Claude permission mode; mapped per-agent in `build_command`.
    pub perm: String,
    pub cwd: PathBuf,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub interactive: bool,
    /// Hollow-owned MCP servers in {mcpServers:{...}} format. The adapter
    /// translates this to its own isolated per-invocation configuration.
    pub mcp_config: Option<String>,
    /// Additional environment/context instructions. Claude appends a system
    /// prompt; other adapters include the same context before the user prompt.
    pub append_system_prompt: Option<String>,
    /// Whether the spawn leads its own process group (GUI) or inherits its
    /// caller's (inside an MCP subprocess).
    pub process_scope: ProcessScope,
    /// Paperclip PR 8b: stream Claude's partial messages on a
    /// non-interactive run (`--include-partial-messages`) so its usage
    /// arrives as each message's final `message_delta` figure rather than a
    /// start-of-message snapshot. Set only when a token budget is watching
    /// the run; [`ParseState::for_spec`] then parses in usage-only mode, so
    /// the run's text events are unchanged. Claude only.
    pub usage_stream: bool,
    /// Paperclip PR 8b: Claude's `--max-budget-usd` spending backstop for a
    /// non-interactive run, from the smallest headroom under an enforced
    /// hard-stop spending budget. Claude enforces it only after a message
    /// completes, so a run can overshoot by up to one message. Claude only;
    /// never with `interactive` or `resume` (a resumed session's cost is
    /// session-cumulative).
    pub max_budget_usd: Option<f64>,
    /// Optional per-run settings, grouped (Paperclip PR 14a, cross-m34 M9)
    /// so a new one touches only the sites that set it: most literals say
    /// `extras: RunExtras::default()`. `process_scope` stays a required
    /// field above, because PR 1's source lint counts it per literal.
    pub extras: RunExtras,
}

/// The optional part of a [`RunSpec`]. Every field defaults to "not set".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunExtras {
    /// Host-owned private config identity, independent of stdio server env.
    pub mcp_run_tag: Option<String>,
    /// Paperclip PR 14a: the reasoning effort from Settings, Models
    /// (`agent_settings::effort`), mapped per model to the nearest
    /// level it takes ([`crate::effort::resolve`]): Claude `--effort`, Codex
    /// a top-level `-c model_reasoning_effort`, Grok `--reasoning-effort`,
    /// Antigravity the model variant. `None` keeps the runtime default; an
    /// effort never fails a run.
    pub effort: Option<String>,
    /// Paperclip PR 15a: the run's scratch folder ([`crate::scratch`]),
    /// set by a launch point only once it holds the run's lease. A folder
    /// sets `TMPDIR`/`TEMP`/`TMP` and `HOLLOW_SCRATCH_DIR` on the child (and
    /// `--add-dir` for Claude and Antigravity); `Unavailable` sets only
    /// `HOLLOW_SCRATCH_DIR`, to a path that can never exist. `None` (tool-free
    /// one-shots) leaves the inherited environment alone. Never part of the
    /// stint fingerprint (v3, OB04, hashes what selects the appended prompt
    /// and the server names, never the argv or the environment).
    pub scratch: Option<crate::scratch::ScratchSpec>,
    /// Connectors from the user's library (`crate::connectors`) attached to
    /// this run's MCP config beside Hollow's own servers. Codex accepts a
    /// non-`hollow_` server only when it is listed here; which ones are
    /// preapproved depends on the mode ([`connector_preapproved`]: every one
    /// in bypass, none in auto or plan, the auto-approve ones otherwise).
    pub shared_connectors: Vec<SharedConnector>,
    /// Keys a conversation for Claude account routing (`claude_accounts`):
    /// its turns stay on one account while that account has room, so its
    /// prompt cache is reused. `None` for runs with no conversation.
    pub account_affinity: Option<String>,
    /// The Claude account this run must use: a chat turn resumed on another
    /// account after its own ran out (R1, `claude_accounts::apply`). `None`
    /// routes as usual.
    pub account: Option<String>,
    /// Long unattended work (an automation, #16): its account is chosen
    /// away from one at or above `claude_accounts::long_work_ceiling` (the
    /// CFO's `100 - reserve`).
    pub long_work: bool,
    /// More folders the agent may write besides its cwd: a build
    /// workspace's vault folder (`crate::build_workspaces`), whose cwd is
    /// the worktree. Claude and Antigravity get `--add-dir` for each (beside
    /// the scratch folder). Codex gets them as writable roots in auto only
    /// ([`codex_auto_sandbox_overrides`]); in its other sandboxed modes the
    /// folder is readable but not writable.
    pub extra_dirs: Vec<std::path::PathBuf>,
    /// Extra environment for the agent and everything it starts (build
    /// programme PR B7: a build workspace's own `CARGO_TARGET_DIR`, port,
    /// toolchain caps and `hollow-slot` on `PATH`; since OF07 the caps and
    /// `hollow-slot` for every chat turn that can take a slot,
    /// `build_slots::chat_env`).
    /// Never on the stand-in.
    /// Also the compaction window (OB22, `agent_settings::compaction_env`):
    /// Claude's own variable, or [`CODEX_COMPACT_LIMIT_ENV`], which becomes
    /// a Codex `-c` override and never reaches the child.
    pub env: Vec<(String, String)>,
    /// The resumed native session already holds this run's appended prompt
    /// (agent effectiveness F6, wave 3 · G2; `stable_layers::holds`): Codex,
    /// Antigravity and Grok then get [`CONFIGURED_HELD`] in its place
    /// ([`contextual_prompt`]), and only while the spec resumes, so a fresh
    /// retry gets it whole. Claude's appended prompt is its system prompt,
    /// which every process needs, so this never changes Claude's command.
    pub configured_held: bool,
    /// The run's message carries its permission mode as a `<hollow-mode>`
    /// line (a chat turn, HB01: `mode_contract::turn_line`), so the policy
    /// after the appended prompt is the mode-free [`chat_policy`] instead of
    /// [`workspace_policy`]: the configured half then stays the same when
    /// the mode flips, and so do the native session and Claude's prompt
    /// cache. Enforcement is the per-process mode flag either way.
    pub mode_per_turn: bool,
    /// Mid-turn steering (2026-10-21, `crate::steering`): a chat turn whose
    /// agent takes messages while it runs. Claude Code then gets
    /// `--input-format stream-json` and no prompt on its command line; the
    /// turn writes the prompt as its first stdin line and holds stdin open
    /// for the person's steers until the answer's `result`. Set only by a
    /// chat turn, whose resolved `steering::resolve_support` snapshot is native.
    pub live_input: bool,
    /// The Mac mini runner (2026-11-01, Phase 2, `crate::remote_session`):
    /// the run's agent process runs on that host, in the folder
    /// [`crate::remote_session::prepare`] readied, while Hollow keeps the
    /// rest here. The command is built for the host's paths (scratch,
    /// working folder; no extra folders: the vault stays here) and then
    /// becomes the run's local leader ([`crate::remote_session::into_leader`]).
    /// Claude and Codex only. Never part of the stint fingerprint.
    pub remote: Option<crate::remote_session::RemoteTarget>,
    /// A self-hosted model (2026-11-03, Q1, `crate::self_hosted`): the run
    /// drives the endpoint through Hollow's loopback forwarder with a
    /// per-run key, never with an Anthropic credential, and no Claude
    /// account is chosen for it. Claude for an `anthropic` entry, Codex
    /// for an `openai` one (Q9: its own `CODEX_HOME`, no OpenAI
    /// credential). Never part of the stint fingerprint (argv and env are
    /// not hashed).
    pub self_hosted: Option<crate::self_hosted::RunTarget>,
    /// OC1 (2026-11-18): Settings' ACP switch for Grok (`agents.acp.grok`)
    /// was on when the chat built this turn. The turn runs over ACP only
    /// when [`grok_acp`] also finds its mode mapped exactly; set only by a
    /// chat turn.
    pub grok_acp: bool,
    /// 2026-11-20: this is an Antigravity chat turn, which runs over `agy acp`
    /// ([`antigravity_acp`]). Set only by a chat turn, so one-shots and every
    /// other caller keep `agy -p`.
    pub antigravity_acp: bool,
    /// AG4b: the answers file of an ask-first Claude chat turn whose gated
    /// tool calls defer to an approval card (`crate::claude_defer`). Set only
    /// by a chat turn that holds a scratch folder; [`claude_defers`] decides
    /// whether the run carries the hook. Never part of the fingerprint.
    pub defer_answers: Option<PathBuf>,
    /// H8: this Claude turn resumes an imported chat's original session and
    /// must fork it (`--fork-session`, `session_import::native_resume`), so
    /// the source file is never appended to. Only while the spec resumes.
    pub fork_resume: bool,
}

/// What Codex, Antigravity and Grok get instead of Hollow's appended prompt
/// on a resumed turn whose native session already holds it (G2). The
/// workspace policy and permission paragraph still go every turn.
pub const CONFIGURED_HELD: &str = "(Hollow's standing instructions for this conversation (visual answers, the capabilities in this run, the brief and the guidance) are unchanged since they were given earlier in this conversation; follow them as given.)";

/// A library connector as the command builder needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedConnector {
    pub id: String,
    pub auto_approve: bool,
    /// A claude.ai connector: it belongs to Claude Code's own login, so the
    /// run skips account spreading (`claude_accounts::apply_login`).
    pub claude_login: bool,
    /// Its display name, for the receipt line saying why a run went on the
    /// login (login override, 2026-10-09).
    pub name: String,
    /// What would keep the run off the login (`connectors::remedy_for`),
    /// for the receipt (five asks B3).
    pub remedy: Option<String>,
}

/// Login override (2026-10-09): the names of the claude.ai connectors that
/// put this Claude run on Claude Code's own login ([`build_command`] calls
/// `claude_accounts::apply_login` for any of them). Empty for any other
/// agent or an inherited (team-mcp) spec, which never routes.
pub fn login_connectors(kind: AgentKind, spec: &RunSpec) -> Vec<String> {
    if kind != AgentKind::Claude || spec.process_scope != ProcessScope::GuiOwned {
        return Vec::new();
    }
    spec.extras
        .shared_connectors
        .iter()
        .filter(|c| c.claude_login)
        .map(|c| {
            if c.name.is_empty() {
                c.id.clone()
            } else {
                c.name.clone()
            }
        })
        .collect()
}

/// What would keep a Claude run with claude.ai connectors off the login
/// (`connectors::login_remedy`), for its receipt row.
pub fn login_remedy(kind: AgentKind, spec: &RunSpec) -> Option<String> {
    if kind != AgentKind::Claude || spec.process_scope != ProcessScope::GuiOwned {
        return None;
    }
    crate::connectors::login_remedy(
        spec.extras
            .shared_connectors
            .iter()
            .filter(|c| c.claude_login)
            .filter_map(|c| c.remedy.as_deref()),
    )
}

/// The reasoning-effort levels Hollow offers for an agent: the same five
/// for every agent (user decision, 2026-10-02). A run maps the saved level
/// to the nearest one its model takes ([`crate::effort::resolve`]).
pub fn effort_levels(_kind: AgentKind) -> &'static [&'static str] {
    &crate::effort::LEVELS
}

/// The workspace boundary every adapter receives, as a system prompt or as
/// context ahead of the user prompt, followed by the run's permission
/// paragraph ([`workspace_policy`]).
pub const WORKSPACE_POLICY: &str = "Hollow is a local workspace for projects, tasks, notes, recordings and Calendar. Hollow itself has no built-in email, Slack or Microsoft Teams features: never read or sync retired message folders, create message approvals, or rebuild those features with shell, browser, plugins or delegated agents. Email, Slack and Teams go only through a connector shared with you in Hollow's Connectors library (named in a <hollow-connectors> note); CLIs like git and gh are not connectors. Calendar events and local task extraction from meetings remain supported.";

/// [`WORKSPACE_POLICY`] and the permission paragraph for a `kind` run in
/// canonical mode `perm`, generated from the mode contract
/// ([`crate::mode_contract::policy`], F6/Q1). Fixed text per (agent
/// capability, mode); the mode is in the resume fingerprint.
pub fn workspace_policy(kind: AgentKind, perm: &str) -> String {
    format!(
        "{WORKSPACE_POLICY}\n\n{}",
        crate::mode_contract::policy(perm, capabilities(kind).owned_mcp)
    )
}

/// Whether a run of `kind` carries Hollow-owned MCP servers: the agent takes
/// them ([`capabilities`]) and, for Antigravity, this run is an ACP chat turn
/// ([`antigravity_acp`]), not a one-shot `agy -p`.
pub fn owned_mcp_for(kind: AgentKind, spec: &RunSpec) -> bool {
    capabilities(kind).owned_mcp && (kind != AgentKind::Antigravity || antigravity_acp(spec))
}

/// A chat's policy (HB01): [`WORKSPACE_POLICY`] and the mode-free
/// paragraph ([`crate::mode_contract::chat_policy`]); the mode itself is
/// the turn's `<hollow-mode>` line. The same text in every mode but plan.
pub fn chat_policy(kind: AgentKind, perm: &str) -> String {
    format!(
        "{WORKSPACE_POLICY}\n\n{}",
        crate::mode_contract::chat_policy(perm, capabilities(kind).owned_mcp)
    )
}

/// The policy `spec`'s command carries: [`chat_policy`] when its message
/// names the mode ([`RunExtras::mode_per_turn`]), else [`workspace_policy`].
pub fn run_policy(kind: AgentKind, spec: &RunSpec) -> String {
    let owned = owned_mcp_for(kind, spec);
    let policy = if spec.extras.mode_per_turn {
        crate::mode_contract::chat_policy(&spec.perm, owned)
    } else {
        crate::mode_contract::policy(&spec.perm, owned)
    };
    format!("{WORKSPACE_POLICY}\n\n{policy}")
}

/// Configure every Hollow-spawned Claude process with only the supplied owned
/// servers. Strict mode excludes user, project, plugin, and remote MCP config.
/// `config_file` is the run's private MCP config file (`run_auth`, merged
/// plan slice 6); without one Claude gets the inline empty config. A config
/// with servers is never inline: argv is readable by every process.
/// `policy` closes the appended system prompt ([`workspace_policy`]).
pub fn isolate_claude(
    cmd: &mut Command,
    config_file: Option<&Path>,
    append: Option<&str>,
    policy: &str,
) {
    isolate_claude_with(cmd, config_file, append, policy, AppendTo::Argv);
}

/// Where [`isolate_claude_with`] puts Claude's appended system prompt.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AppendTo {
    /// `--append-system-prompt <text>` (probes with a fixed policy only).
    Argv,
    /// An unlinked private file on a descriptor of ours (`secret_fds::pass`).
    Descriptor,
    /// Left out and returned: a runner run carries it in the leader's header.
    Withheld,
}

/// [`isolate_claude`], with the appended system prompt's route `to` (R5b,
/// Q-R33: it carries the person's context, and argv is readable by every
/// process): [`AppendTo::Descriptor`] is `--append-system-prompt-file
/// /dev/fd/<n>`, falling back to the argument only when the file can't be
/// made; [`AppendTo::Withheld`] returns the text for the caller to carry.
fn isolate_claude_with(
    cmd: &mut Command,
    config_file: Option<&Path>,
    append: Option<&str>,
    policy: &str,
    to: AppendTo,
) -> Option<String> {
    let config = match config_file {
        Some(file) => file.as_os_str(),
        None => std::ffi::OsStr::new(r#"{"mcpServers":{}}"#),
    };
    cmd.arg("--strict-mcp-config")
        .arg("--mcp-config")
        .arg(config);
    let prompt = match append.filter(|s| !s.is_empty()) {
        Some(extra) => format!("{extra}\n\n{policy}"),
        None => policy.to_string(),
    };
    if to == AppendTo::Withheld {
        return Some(prompt);
    }
    let fd = if to == AppendTo::Descriptor {
        crate::secret_fds::pass(cmd, &[prompt.as_str()])
            .ok()
            .and_then(|fds| fds.first().copied())
    } else {
        None
    };
    if let Some(fd) = fd {
        cmd.arg("--append-system-prompt-file")
            .arg(format!("/dev/fd/{fd}"));
    } else {
        cmd.arg("--append-system-prompt").arg(prompt);
    }
    None
}

/// Build the fully-configured process (args + cwd + piped stdio) for `kind`.
/// Callers spawn it, adopt a `process_group::GroupGuard` for the child, and
/// wait only through `process_group::drive`, which ends the whole process
/// tree before returning. A GUI-owned spec also leads its own process group
/// and carries a fresh run tag, which is injected into every owned MCP
/// server's environment too.
pub fn build_command(kind: AgentKind, spec: &RunSpec) -> Command {
    build_command_timed(kind, spec).0
}

/// [`build_command`], and Codex's native MCP inventory inside it (agent
/// effectiveness F8): how long it took and whether `codex mcp list` ran or
/// a cached listing served it (`codex_inventory`). `None` for every other
/// agent, which runs none.
pub(crate) fn build_command_timed(
    kind: AgentKind,
    spec: &RunSpec,
) -> (Command, Option<crate::codex_inventory::Inventory>) {
    build_command_via(kind, spec, crate::codex_inventory::installed)
}

/// [`build_command_timed`] with Codex's inventory from `inventory` (the
/// installed CLI through the app's cache in production; a fake binary in
/// tests and the preparation bench).
pub(crate) fn build_command_via(
    kind: AgentKind,
    spec: &RunSpec,
    inventory: impl FnOnce(&RunSpec) -> crate::codex_inventory::Listing,
) -> (Command, Option<crate::codex_inventory::Inventory>) {
    let (servers, inventory) = if kind == AgentKind::Codex {
        let started = std::time::Instant::now();
        let listing = inventory(spec);
        let timed = crate::codex_inventory::Inventory {
            mark: crate::prep_timing::Mark::since(started),
            served: listing.served,
        };
        (listing.servers, Some(timed))
    } else {
        (Ok(Vec::new()), None)
    };
    (build_scoped(kind, spec, servers), inventory)
}

/// [`build_command`] without asking Codex for its configured servers, so a
/// test outside this module can read a command's argv.
#[cfg(test)]
pub(crate) fn command_without_codex_listing(kind: AgentKind, spec: &RunSpec) -> Command {
    build_with_codex_servers(kind, spec, Ok(Vec::new()))
}

fn build_scoped(kind: AgentKind, spec: &RunSpec, servers: Result<Vec<String>, String>) -> Command {
    // Its CLI is being replaced (Settings, Agent updates): say so plainly
    // rather than start a half-installed binary. Otherwise the run counts as
    // in use until its process group settles, so an update waits for it.
    if spec.process_scope == ProcessScope::Inherited {
        if let Some(why) = crate::agent_cli::admit(kind, None) {
            return failed_command(spec, &why);
        }
        return build_with_codex_servers(kind, spec, servers);
    }
    let tag = process_group::RunTag::new();
    if let Some(why) = crate::agent_cli::admit(kind, Some(tag.as_str())) {
        return failed_command(spec, &why);
    }
    let mut cmd = match spec.mcp_config.as_deref().filter(|c| !c.is_empty()) {
        Some(config) => {
            let config = process_group::with_run_tag(config, &tag);
            // Self-hosted models Q5 review: Hollow's own servers learn they
            // run under a self-hosted run from their config, not from what
            // the CLI passes down (`hollow team-mcp` refuses a Claude
            // teammate there).
            let config = match spec.extras.self_hosted.as_ref() {
                Some(target) => crate::self_hosted::with_marker(&config, &target.entry),
                None => config,
            };
            // The run ids its servers' proofs name, for the brokers' peer
            // check (`run_auth`): the guard that adopts the child registers
            // them for its process tree.
            let bound = crate::run_auth::bound_runs_value(&config);
            #[cfg(unix)]
            let (config, shared) = if spec.extras.remote.is_none()
                && matches!(kind, AgentKind::Claude | AgentKind::Codex)
            {
                crate::shared_mcp_host::config(&config, tag.as_str())
            } else {
                (config, None)
            };
            let tagged = RunSpec {
                mcp_config: Some(config),
                extras: RunExtras {
                    mcp_run_tag: Some(tag.as_str().into()),
                    ..spec.extras.clone()
                },
                ..spec.clone()
            };
            let mut cmd = build_with_codex_servers(kind, &tagged, servers);
            if !is_unavailable_command(&cmd) {
                cmd.env(crate::run_auth::BOUND_RUNS_ENV, bound);
                #[cfg(unix)]
                if let Some(shared) = shared {
                    crate::shared_mcp_host::retain(&mut cmd, shared);
                }
            }
            cmd
        }
        None => build_with_codex_servers(kind, spec, servers),
    };
    process_group::isolate_with(&mut cmd, &tag);
    cmd
}

/// Which Claude account step a command gets (Q1 review: a pure decision,
/// so a test can prove a self-hosted run gets none).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccountRoute {
    /// No account step: another agent, an inherited spec (team-mcp
    /// delegates keep what their parent passed down), or a self-hosted run.
    None,
    /// Claude Code's own login (a claude.ai connector is attached).
    Login,
    /// Long unattended work (#16).
    LongWork,
    /// The usual routing (affinity, R1's forced account).
    Routed,
}

/// The account step for `kind` running `spec`.
pub(crate) fn account_route(kind: AgentKind, spec: &RunSpec) -> AccountRoute {
    if kind != AgentKind::Claude
        || spec.process_scope != ProcessScope::GuiOwned
        || spec.extras.self_hosted.is_some()
    {
        AccountRoute::None
    } else if spec.extras.shared_connectors.iter().any(|c| c.claude_login) {
        AccountRoute::Login
    } else if spec.extras.long_work {
        AccountRoute::LongWork
    } else {
        AccountRoute::Routed
    }
}

pub(crate) async fn refresh_accounts(kind: AgentKind, spec: &RunSpec) {
    if matches!(
        account_route(kind, spec),
        AccountRoute::Routed | AccountRoute::LongWork
    ) {
        crate::claude_accounts::refresh_before_pick().await;
    }
}

fn build_with_codex_servers(
    kind: AgentKind,
    spec: &RunSpec,
    servers: Result<Vec<String>, String>,
) -> Command {
    #[cfg(test)]
    crate::secret_fds::reset();
    if let Err(error) = validate_run_spec(kind, spec) {
        return failed_command(spec, &error);
    }
    // A remote run (the Mac mini runner): built for the host's scratch folder,
    // with no extra folders but the copy of a build
    // workspace's vault folder there (Phase 3; the vault stays here), then
    // turned into the run's local leader at the end.
    let remote_spec;
    let spec = match spec.extras.remote.as_ref() {
        Some(target) => {
            if !matches!(kind, AgentKind::Claude | AgentKind::Codex) {
                return failed_command(spec, "Only Claude and Codex can run on the runner.");
            }
            let mut host = spec.clone();
            host.extras.scratch = Some(crate::scratch::ScratchSpec::Dir(PathBuf::from(
                &target.scratch,
            )));
            host.extras.extra_dirs = remote_extra_dirs(target);
            remote_spec = host;
            &remote_spec
        }
        None => spec,
    };
    let remote = spec.extras.remote.as_ref();
    // A local Codex auto run may write its worktree's git folder: its
    // `commondir` and `gitdir` are snapshotted and guarded from here on.
    if kind == AgentKind::Codex && spec.perm == "auto" && remote.is_none() {
        arm_codex_worktree_guard(&spec.cwd, &spec.extras.extra_dirs);
    }
    // A self-hosted model (Q1) rides the harness its API style names
    // (Claude, or Codex since Q9), from this Mac only, and only through a
    // loopback forwarder with a run key.
    if let Some(target) = spec.extras.self_hosted.as_ref() {
        if let Some(why) = crate::self_hosted::wrong_harness(target, kind) {
            return failed_command(spec, &why);
        }
        if remote.is_some() {
            return failed_command(
                spec,
                &format!(
                    "{} is a self-hosted model, which runs on this Mac only.",
                    target.name
                ),
            );
        }
        if let Some(why) = crate::self_hosted::invalid(target) {
            return failed_command(spec, &why);
        }
        // Q9: Codex's own home for the entry, made and checked (no sign-in).
        if let Some(home) = target
            .codex_home
            .as_deref()
            .filter(|_| kind == AgentKind::Codex)
        {
            if let Err(why) = crate::self_hosted::ensure_codex_home(home) {
                return failed_command(spec, &why);
            }
        }
        // Q5 review: a claude.ai connector runs on Claude Code's own login,
        // which a self-hosted run never uses (the chat leaves them out).
        if crate::self_hosted::claude_login_servers(spec) {
            return failed_command(
                spec,
                &format!(
                    "{} is a self-hosted model, and a claude.ai connector can't go with it (it runs on your Claude login), so the run didn't start.",
                    target.name
                ),
            );
        }
    }
    // The run's private config file, which a remote run's leader starts the
    // tools from, and Claude's tools as the host sees them (the bridges).
    let mut relay_config: Option<PathBuf> = None;
    let mut claude_bridges: Option<String> = None;
    // R5b: what a runner run's argv would carry but must not.
    let mut handover = crate::remote_session::Handover::default();
    let bin = find_binary(kind).unwrap_or_else(|| PathBuf::from(bin_name(kind)));
    let mut cmd = Command::new(&bin);
    // HTTP connectors' header values for Codex: its environment, not argv.
    let mut codex_env: Vec<(String, String)> = Vec::new();
    match (kind, servers) {
        (AgentKind::Claude, _) => {
            // Merged plan slice 6: the config (broker proofs, connector
            // secrets) goes to the run's private file, never onto argv.
            let file = match spec.mcp_config.as_deref().filter(|s| !s.is_empty()) {
                Some(config) => match crate::run_auth::write_config_as(
                    config,
                    spec.extras.mcp_run_tag.as_deref(),
                ) {
                    Ok(file) => Some(file),
                    Err(error) => return failed_command(spec, &error),
                },
                None => None,
            };
            if let (Some(target), Some(config)) =
                (remote, spec.mcp_config.as_deref().filter(|s| !s.is_empty()))
            {
                match crate::remote_session::bridge_config(config, &target.home) {
                    Ok(bridges) => claude_bridges = Some(bridges),
                    Err(error) => return failed_command(spec, &error),
                }
            }
            relay_config = file.clone();
            handover.append = build_claude(&mut cmd, spec, file.as_deref());
        }
        (AgentKind::Codex, Ok(names)) => {
            // An empty table override deep-merges with Codex config; disable each
            // configured server explicitly, without changing any external config.
            let owned_names: Vec<String> = spec
                .mcp_config
                .as_deref()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|v| {
                    v.get("mcpServers")
                        .and_then(Value::as_object)
                        .map(|m| m.keys().cloned().collect())
                })
                .unwrap_or_default();
            let mut overrides = names
                .iter()
                .filter(|name| !owned_names.contains(name))
                .map(|name| {
                    format!(
                        "{}={{enabled=false}}",
                        serde_json::to_string(name).expect("server name")
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            if let Some(config) = spec.mcp_config.as_deref().filter(|s| !s.is_empty()) {
                // Merged plan slice 6: each stdio server starts through
                // `hollow mcp-launch`, which reads it from the run's private
                // file, so no proof or secret reaches Codex's argv.
                let launch = match CodexLaunch::write(config, spec.extras.mcp_run_tag.as_deref()) {
                    Ok(mut launch) => {
                        launch.remote_home = remote.map(|t| t.home.clone());
                        relay_config = Some(launch.file.clone());
                        launch
                    }
                    Err(error) => return failed_command(spec, &error),
                };
                let shared = &spec.extras.shared_connectors;
                match codex_server_overrides(config, &launch, &spec.perm, shared) {
                    Ok(owned) => {
                        if !owned.overrides.is_empty() {
                            if !overrides.is_empty() {
                                overrides.push(',');
                            }
                            overrides.push_str(&owned.overrides);
                        }
                        codex_env = owned.env;
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(&launch.file);
                        return failed_command(spec, &error);
                    }
                }
            }
            // Codex splits override keys on dots without TOML key parsing;
            // quoting belongs in the inline table, not the dotted -c key.
            cmd.arg("-c").arg(format!("mcp_servers={{{overrides}}}"));
            // Steering PR 3: a steerable chat turn runs `codex app-server`
            // (the turn speaks JSON-RPC on its stdin); every other run exec.
            if live_codex_input(spec) {
                build_codex_app_server(&mut cmd, spec);
            } else {
                build_codex(&mut cmd, spec);
            }
        }
        (AgentKind::Codex, Err(error)) => {
            // Fail closed if the local CLI cannot enumerate its configuration.
            // The error is an argument, never interpolated shell code.
            cmd = Command::new("/bin/sh");
            cmd.arg("-c")
                .arg("printf '%s\n' \"$1\" >&2; exit 1")
                .arg(UNAVAILABLE_ARGV0)
                .arg(error);
        }
        // 2026-11-20: a chat turn runs over `agy acp` ([`antigravity_acp`]).
        (AgentKind::Antigravity, _) if antigravity_acp(spec) => {
            build_antigravity_acp(&mut cmd, spec)
        }
        (AgentKind::Antigravity, _) => build_antigravity(&mut cmd, spec),
        // OC1: a chat turn with the ACP switch on and an exactly mapped mode.
        (AgentKind::Grok, _) if grok_acp(spec) => build_grok_acp(&mut cmd, spec),
        (AgentKind::Grok, _) => build_grok(&mut cmd, spec),
    }
    if !is_unavailable_command(&cmd) {
        for (key, value) in &codex_env {
            cmd.env(key, value);
        }
        // Before the extras, so a build workspace's own PATH still wins.
        cmd.env("PATH", cli_path());
        // Self-hosted models Q5 review: a run with no self-hosted target
        // never inherits one (a teammate of a self-hosted chat, whose
        // process env holds the endpoint and the run key). Before the
        // extras, so the run's own compaction window still applies.
        if spec.extras.self_hosted.is_none() {
            crate::self_hosted::strip_inherited(&mut cmd);
        }
        for (key, value) in &spec.extras.env {
            // OB22: Hollow's carrier for Codex's `-c` override, not the child's.
            if key == CODEX_COMPACT_LIMIT_ENV {
                continue;
            }
            cmd.env(key, value);
        }
        // NV9: after the extras, so nothing can turn them back on.
        if kind == AgentKind::Claude {
            apply_claude_run_settings(&mut cmd);
        }
    }
    // Never on the `/bin/sh` stand-in: it is not an agent.
    if let Some(scratch) = spec.extras.scratch.as_ref() {
        if !is_unavailable_command(&cmd) {
            apply_scratch(&mut cmd, scratch);
        }
    }
    // Which Claude account pays for the run (a no-op until a second account
    // is saved, and outside the GUI). Inherited specs (team-mcp delegates)
    // keep whatever their parent passed down.
    if !is_unavailable_command(&cmd) {
        match account_route(kind, spec) {
            AccountRoute::None => {}
            AccountRoute::Login => crate::claude_accounts::apply_login(&mut cmd),
            AccountRoute::LongWork => {
                if let Some(message) = crate::claude_accounts::waiting_for_room() {
                    return failed_command(spec, &message);
                }
                crate::claude_accounts::apply_long_work(&mut cmd, spec.extras.account.as_deref())
            }
            AccountRoute::Routed => {
                if let Some(message) = crate::claude_accounts::waiting_for_room() {
                    return failed_command(spec, &message);
                }
                crate::claude_accounts::apply(
                    &mut cmd,
                    spec.extras.account_affinity.as_deref(),
                    spec.extras.account.as_deref(),
                )
            }
        }
        // A self-hosted run (Q1) gets no account at all: its credential is
        // the run key, set after the account step so nothing can put an
        // Anthropic one back. Q9: a Codex run's key, home and no OpenAI
        // credential.
        if let Some(target) = spec.extras.self_hosted.as_ref() {
            match kind {
                AgentKind::Codex => crate::self_hosted::apply_codex_env(&mut cmd, target),
                _ => crate::self_hosted::apply_env(&mut cmd, target),
            }
        }
    }
    // A steerable Claude or Codex chat turn reads its messages from stdin,
    // which the turn writes and closes (`chat::drive_turn_attempts`); every
    // other run gets none.
    let live = match kind {
        AgentKind::Claude => live_claude_input(spec),
        AgentKind::Codex => live_codex_input(spec),
        // OC1: a Grok ACP turn speaks JSON-RPC on its stdin.
        AgentKind::Grok => grok_acp(spec),
        AgentKind::Antigravity => false,
    };
    #[cfg(test)]
    LAST_STDIN_PROMPT.with(|last| *last.borrow_mut() = None);
    let stdin = if live {
        Stdio::piped()
    } else if is_unavailable_command(&cmd) {
        Stdio::null()
    } else if remote.is_some() {
        // A runner run's prompt goes in the leader's header, which the runner
        // hands the agent as its stdin (R5b); the leader's own stdin is empty.
        handover.stdin = stdin_prompt(kind, spec);
        Stdio::null()
    } else if let Some(prompt) = stdin_prompt(kind, spec) {
        // Tests read the prompt text, not Antigravity's JSON wrapper.
        #[cfg(test)]
        LAST_STDIN_PROMPT.with(|last| {
            let text = match kind {
                AgentKind::Antigravity => serde_json::from_str::<serde_json::Value>(&prompt)
                    .ok()
                    .and_then(|v| v["message"]["content"].as_str().map(str::to_string))
                    .unwrap_or_else(|| prompt.clone()),
                _ => prompt.clone(),
            };
            *last.borrow_mut() = Some(text)
        });
        match prompt_stdin_file(&prompt) {
            Ok(file) => Stdio::from(file),
            Err(error) => {
                return failed_command(
                    spec,
                    &format!("Hollow couldn't hand the run its prompt: {error}"),
                )
            }
        }
    } else {
        Stdio::null()
    };
    cmd.current_dir(&spec.cwd)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(target) = remote {
        if is_unavailable_command(&cmd) {
            return cmd;
        }
        return match crate::remote_session::into_leader_with(
            &cmd,
            kind,
            target,
            relay_config.as_deref(),
            claude_bridges.as_deref(),
            handover,
        ) {
            Ok(leader) => leader,
            Err(error) => failed_command(spec, &error),
        };
    }
    cmd
}

/// Paperclip PR 15a: the scratch environment of an agent child. A folder
/// sets `TMPDIR`, `TEMP` and `TMP` (with the trailing slash macOS uses) and
/// `HOLLOW_SCRATCH_DIR`; `Unavailable` sets only `HOLLOW_SCRATCH_DIR`, to
/// [`crate::scratch::UNAVAILABLE_PATH`], so the always-present guidance never
/// expands to an empty path. Hollow always overrides `TMPDIR` here: a launchd
/// app inherits one, so "only if unset" would never apply.
fn apply_scratch(cmd: &mut Command, scratch: &crate::scratch::ScratchSpec) {
    match scratch {
        crate::scratch::ScratchSpec::Dir(dir) => {
            let tmp = format!("{}/", dir.to_string_lossy().trim_end_matches('/'));
            cmd.env("TMPDIR", &tmp)
                .env("TEMP", &tmp)
                .env("TMP", &tmp)
                .env(crate::scratch::SCRATCH_ENV, dir);
        }
        crate::scratch::ScratchSpec::Unavailable => {
            cmd.env(
                crate::scratch::SCRATCH_ENV,
                crate::scratch::UNAVAILABLE_PATH,
            );
        }
    }
}

/// `$0` of the `/bin/sh` stand-in [`build_command`] returns when an agent
/// cannot run (a refused spec, or a Codex configuration it cannot isolate).
/// The stand-in still runs, so callers keep printing its stderr, but it is not
/// an agent: [`is_unavailable_command`] lets the usage ledger record it as
/// never started.
pub const UNAVAILABLE_ARGV0: &str = "hollow-agent-unavailable";

/// True for the stand-in command above.
pub fn is_unavailable_command(cmd: &Command) -> bool {
    let std = cmd.as_std();
    std.get_program() == std::ffi::OsStr::new("/bin/sh")
        && std.get_args().nth(2) == Some(std::ffi::OsStr::new(UNAVAILABLE_ARGV0))
}

fn failed_command(spec: &RunSpec, error: &str) -> Command {
    let mut cmd = Command::new("/bin/sh");
    cmd.args([
        "-c",
        "printf '%s\\n' \"$1\" >&2; exit 1",
        UNAVAILABLE_ARGV0,
        error,
    ]);
    cmd.current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// The prefix of the refusal an agent run gets in an unsafe working folder
/// (Paperclip PR 19). Deliberately distinct from chat's
/// `CHAT_CONTEXT_RECOVERED` (a folder that vanished): this folder exists,
/// and Hollow will not run an agent there.
pub(crate) const UNSAFE_CWD_ERROR: &str = "UNSAFE_WORKING_DIRECTORY";

/// Why `cwd` is no place for an agent run, or `None` when it is fine
/// (Paperclip PR 19). After canonicalization (so `/tmp` and `/private/tmp`
/// are one folder, and a symlink is judged by where it leads) the folder must
/// not be exactly `/`, `/tmp`, `/private/tmp`, `/usr`, `/System`, `/Library`
/// or the bare home folder. Only those roots: `/tmp/x` or `~/Documents` are
/// fine. The configured vault is never refused, even when it IS one of them
/// (a vault at `~`). A folder that cannot be canonicalized (missing) is not
/// judged here: the callers' own existence checks handle it. Pure apart from
/// the filesystem reads.
pub(crate) fn unsafe_working_dir(cwd: &Path, vault: Option<&Path>) -> Option<String> {
    let canonical = std::fs::canonicalize(cwd).ok()?;
    if let Some(vault) = vault {
        let vault = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
        if canonical == vault {
            return None;
        }
    }
    let mut roots: Vec<PathBuf> = ["/", "/tmp", "/private/tmp", "/usr", "/System", "/Library"]
        .iter()
        .map(PathBuf::from)
        .collect();
    if let Some(home) = dirs::home_dir() {
        roots.push(home);
    }
    let denied = roots
        .iter()
        .any(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()) == canonical);
    denied.then(|| format!("{} is a system or shared folder", canonical.display()))
}

/// The refusal for [`unsafe_working_dir`]'s `reason`, starting with
/// [`UNSAFE_CWD_ERROR`]. One line, no em dash, no angle brackets.
pub(crate) fn unsafe_cwd_error(reason: &str) -> String {
    format!(
        "{UNSAFE_CWD_ERROR}: Hollow won't run an agent in {reason}. Choose a project folder, or use the vault."
    )
}

/// A permission sandbox is not a tool allowlist: a read-only CLI may still
/// read private files or reach remote services. Reject unsupported restrictions
/// instead of quietly executing with more authority than the caller requested.
/// Whether `kind` enforces the deny rule `tool` as a flag. Claude takes any
/// rule. Grok takes only the computer-control pair
/// ([`crate::computer_mcp::BASH_DENY`]) as `--deny`: probed 2026-10-02 with
/// grok 1.0.46 `-p --permission-mode bypassPermissions`, both were "Denied
/// by permission policy" while `echo` ran. Other rule shapes are unprobed for
/// Grok (it skips a rule naming a tool it doesn't know), so they still refuse.
pub fn holds_deny(kind: AgentKind, tool: &str) -> bool {
    match kind {
        AgentKind::Claude => true,
        AgentKind::Grok => crate::computer_mcp::BASH_DENY.contains(&tool),
        AgentKind::Codex | AgentKind::Antigravity => false,
    }
}

pub fn validate_run_spec(kind: AgentKind, spec: &RunSpec) -> Result<(), String> {
    if let Some(reason) = unavailable_mode(kind, spec.perm.trim()) {
        return Err(reason.to_string());
    }
    // Self-hosted models Q5: `self-hosted:<id>` names a registry entry, which
    // only a chat turn resolves to its endpoint (`RunExtras::self_hosted`).
    // Anything else would send the id to the agent's own provider with the
    // person's login, so it is refused here instead; so is a Claude run on
    // a known endpoint's served id without its target (Q5 review), and a
    // Codex run on an `openai` entry's (Q9).
    if spec.extras.self_hosted.is_none() {
        if let Some(model) = spec
            .model
            .as_deref()
            .filter(|m| crate::self_hosted::needs_target_on(kind, m))
        {
            return Err(crate::self_hosted::chats_only(model));
        }
    }
    let forbidden = spec.disallowed_tools.iter().any(|tool| {
        !matches!(
            tool.as_str(),
            "SendUserFile" | "AskUserQuestion" | "SendUserMessage"
        ) && !holds_deny(kind, tool)
    });
    if !capabilities(kind).tool_restrictions && (!spec.allowed_tools.is_empty() || forbidden) {
        return Err(format!("{} does not support the tool restrictions required by this operation. Choose an agent with restricted-tool support in Settings → Models, or remove an explicit workflow restriction after reviewing it.", kind.label()));
    }
    if !owned_mcp_for(kind, spec) && spec.mcp_config.as_deref().is_some_and(|s| !s.is_empty()) {
        return Err(format!(
            "{} does not support Hollow-owned connectors.",
            kind.label()
        ));
    }
    // Effort never fails a run: it maps to the nearest level the model
    // takes, or is left out (`effort::resolve`, 2026-10-02).
    // Paperclip PR 8b: mid-run budget metering is Claude's alone.
    if kind != AgentKind::Claude && (spec.usage_stream || spec.max_budget_usd.is_some()) {
        return Err(format!(
            "{} does not support budget metering.",
            kind.label()
        ));
    }
    if let Some(usd) = spec.max_budget_usd {
        if !(usd.is_finite() && usd > 0.0) {
            return Err("The spending backstop must be a positive amount.".into());
        }
        // Per invocation only: a resumed session reports cumulative cost.
        if spec.interactive || spec.resume.as_deref().is_some_and(|s| !s.is_empty()) {
            return Err(
                "The spending backstop applies only to a fresh non-interactive run.".into(),
            );
        }
    }
    Ok(())
}

/// Hollow-owned MCP servers whose tools run without a prompt, and the Claude
/// `--allowed-tools` token that preapproves each (the whole-server
/// `mcp__<server>` form). Paperclip PR 13, decided with the user on
/// 2026-09-25: question cards, task tools (PR 12), read-only vault search, and
/// teamwork (bounded by the run permit and budgets). NOT `hollow_computer`:
/// its desktop side effects stay behind the permission mode (preapproved in
/// `auto` and `bypassPermissions` only, [`OWNED_MODE_PREAPPROVED`]), and
/// tier 2 behind the Allow/Deny card.
///
/// Headless CLIs never prompt: Claude `-p` DENIES an un-preapproved MCP call
/// in `acceptEdits`/`plan` (a `permission_denied` event, the run still ends
/// `success`), and `codex exec` (approval policy `never`) FAILS it. Recorded
/// 2026-09-25 (fixtures/interactions/README.md). Codex gets
/// `default_tools_approval_mode="approve"` for the same servers
/// ([`codex_server_overrides`]). Order is fixed so argv stays byte-stable.
///
/// Teamwork is never preapproved for a `plan` (read-only) run: a teammate
/// would be a way to make changes (`team_mcp::delegate_grant` refuses a plan
/// parent too), so preapproving it would let a read-only chat or workflow
/// write through a teammate ([`preapproves`]).
const OWNED_PREAPPROVED: &[(&str, &str)] = &[
    ("hollow_questions", "mcp__hollow_questions"),
    ("hollow_tasks", "mcp__hollow_tasks"),
    // PR A1: the broker, not the CLI prompt, decides what needs the user.
    ("hollow_app", crate::app_mcp::APP_TOOL_ALLOW),
    ("hollow_vault", "mcp__hollow_vault"),
    // CC2: read only, so preapproved in every mode.
    ("hollow_code_context", "mcp__hollow_code_context"),
    ("hollow_team", crate::team_mcp::TEAM_TOOL_ALLOW),
];

/// Hollow-owned servers preapproved only in `auto` and `bypassPermissions`
/// (user decision, 2026-10-02: auto approves everything that doesn't truly
/// need the user, bypass everything). Computer control's tier-2 verbs keep
/// their own Allow/Deny card in auto; in bypass that card is skipped too
/// (`computer_mcp::confirm`). Services are preapproved in auto only because
/// the broker bounds them there (merged plan decision 1, `services::bounded`):
/// folder inside the worktree, stopped with the chat, command on the receipt,
/// grant tied to the run. Only bypass starts an unrestricted one.
const OWNED_MODE_PREAPPROVED: &[(&str, &str)] = &[
    ("hollow_computer", "mcp__hollow_computer"),
    ("hollow_services", "mcp__hollow_services"),
];

/// Whether `perm` approves every Hollow-owned tool up front.
pub(crate) fn mode_approves_all(perm: &str) -> bool {
    matches!(perm, "auto" | "bypassPermissions")
}

/// Whether `server` is a Hollow-owned server whose tools are preapproved for
/// a run in canonical permission mode `perm`.
fn preapproves(server: &str, perm: &str) -> bool {
    let always = OWNED_PREAPPROVED.iter().any(|(name, _)| *name == server)
        && !(perm == "plan" && server == "hollow_team");
    always
        || (mode_approves_all(perm)
            && OWNED_MODE_PREAPPROVED
                .iter()
                .any(|(name, _)| *name == server))
}

/// The Claude preapproval tokens for the preapproved servers present in an
/// `{mcpServers:{…}}` config, in [`OWNED_PREAPPROVED`] order. Empty for no
/// config or an unreadable one (the build then fails on the config itself).
/// `perm` is the run's canonical permission mode ([`preapproves`]).
pub(crate) fn owned_preapprovals(mcp_config: Option<&str>, perm: &str) -> Vec<&'static str> {
    let Some(servers) = mcp_config
        .filter(|s| !s.is_empty())
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|v| v.get("mcpServers").and_then(Value::as_object).cloned())
    else {
        return Vec::new();
    };
    OWNED_PREAPPROVED
        .iter()
        .chain(OWNED_MODE_PREAPPROVED)
        .filter(|(name, _)| servers.contains_key(*name) && preapproves(name, perm))
        .map(|(_, token)| *token)
        .collect()
}

/// `mcp__<server>__<tool>` for every tool an attached server keeps from
/// agents ([`crate::builtin_connectors::hidden_tools`]).
fn hidden_mcp_tools(mcp_config: Option<&str>) -> Vec<String> {
    let Some(servers) = mcp_config
        .filter(|s| !s.is_empty())
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|v| v.get("mcpServers").and_then(Value::as_object).cloned())
    else {
        return Vec::new();
    };
    servers
        .keys()
        .flat_map(|name| {
            crate::builtin_connectors::hidden_tools(name)
                .iter()
                .map(move |tool| format!("mcp__{name}__{tool}"))
        })
        .collect()
}

/// Whether the config attaches at least one MCP server.
fn has_mcp_servers(mcp_config: Option<&str>) -> bool {
    mcp_config
        .filter(|s| !s.is_empty())
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|v| {
            v.get("mcpServers")
                .and_then(Value::as_object)
                .map(|m| !m.is_empty())
        })
        .unwrap_or(false)
}

/// Whether a shared library connector's tools run without a prompt
/// ([`crate::mode_contract::connector_preapproved`], the rule the
/// `<hollow-connectors>` note describes).
fn connector_preapproved(connector: &SharedConnector, perm: &str) -> bool {
    crate::mode_contract::connector_preapproved(connector.auto_approve, perm)
}

/// How a Codex run starts its stdio MCP servers (merged plan slice 6): this
/// binary's `mcp-launch` subcommand, pointed at the run's private config
/// file, so a server's command, arguments and environment stay off argv.
#[derive(Debug, Clone)]
struct CodexLaunch {
    exe: PathBuf,
    file: PathBuf,
    /// `run_auth::config_digest` of what was written, so `mcp-launch`
    /// refuses a file rewritten before Codex starts the server.
    digest: String,
    /// A remote run's host home: each server then starts there as the
    /// bridge to the same server here (`crate::remote_session`).
    remote_home: Option<String>,
}

impl CodexLaunch {
    /// Write the run's config file and find this binary.
    fn write(config: &str, tag: Option<&str>) -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| {
            format!("Hollow couldn't find its own program to start Codex's tools ({e}).")
        })?;
        let file = crate::run_auth::write_config_as(config, tag)?;
        let digest = crate::run_auth::config_digest(config);
        Ok(Self {
            exe,
            file,
            digest,
            remote_home: None,
        })
    }

    /// The `command=…,args=[…]` fields that launch `name` (on a remote
    /// run's host: the bridge, with the relay variables it needs).
    fn fields(&self, name: &str) -> Vec<String> {
        if let Some(home) = self.remote_home.as_deref() {
            return crate::remote_session::codex_bridge_fields(home, name).to_vec();
        }
        let args = json!([
            crate::run_auth::LAUNCH_SUBCOMMAND,
            self.file.to_string_lossy(),
            name,
            self.digest
        ]);
        vec![
            format!("command={}", toml_string(&self.exe.to_string_lossy())),
            format!("args={args}"),
        ]
    }
}

/// A JSON string literal, which is also a valid TOML basic string.
fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("a string serializes")
}

/// The Codex `mcp_servers` overrides for a run's owned and library servers,
/// and the environment Codex itself must carry for them (HTTP header
/// values, read through `env_http_headers`).
#[derive(Debug, Default)]
struct CodexServers {
    overrides: String,
    env: Vec<(String, String)>,
}

/// Codex's own MCP tool-call timeout (`tool_timeout_sec`; its default is
/// [`CODEX_DEFAULT_TOOL_TIMEOUT_SECS`]) Hollow sets for owned server `name`:
/// longer than every wait a tool of that server can make
/// (`delegation::WAITING_TOOLS`, F10 D2). `None` keeps Codex's default.
pub(crate) fn codex_tool_timeout_secs(name: &str) -> Option<u64> {
    match name {
        crate::app_mcp::SERVER_NAME => Some(crate::app_mcp::CODEX_TOOL_TIMEOUT_SECS),
        crate::team_mcp::SERVER_NAME => Some(crate::team_mcp::CODEX_TOOL_TIMEOUT_SECS),
        _ => None,
    }
}

/// Codex's tool timeout for an approved mail connector: past the approval
/// card's 120 s wait plus the server's own reply.
pub(crate) const MAIL_TOOL_TIMEOUT_SECS: u64 = 180;

/// Codex's MCP tool-call timeout when a server sets none.
#[cfg(test)]
pub(crate) const CODEX_DEFAULT_TOOL_TIMEOUT_SECS: u64 = 60;

fn codex_server_overrides(
    config: &str,
    launch: &CodexLaunch,
    perm: &str,
    shared: &[SharedConnector],
) -> Result<CodexServers, String> {
    let value: Value = serde_json::from_str(config)
        .map_err(|e| format!("Invalid Hollow connector configuration: {e}"))?;
    let servers = value
        .get("mcpServers")
        .and_then(Value::as_object)
        .ok_or("Invalid Hollow connector configuration: missing mcpServers")?;
    let mut entries = Vec::new();
    let mut env_out: Vec<(String, String)> = Vec::new();
    for (name, value) in servers {
        let library = shared.iter().find(|c| &c.id == name);
        if !name.starts_with("hollow_") && library.is_none() {
            return Err(
                "Only Hollow-owned or library connectors may be attached to an agent.".into(),
            );
        }
        let object = value
            .as_object()
            .ok_or("Invalid Hollow connector configuration: server must be an object")?;
        if let Some(url) = object.get("url").and_then(Value::as_str) {
            if library.is_none() && !name.starts_with("hollow_") {
                return Err("Unselected HTTP MCP server".into());
            }
            // A remote run (the Mac mini runner) gets no HTTP connector with
            // headers: they are often credentials, and would go there.
            if launch.remote_home.is_some() && object.get("headers").is_some() {
                continue;
            }
            // A streamable-HTTP library connector.
            let mut fields = vec![
                format!("url={}", serde_json::to_string(url).unwrap()),
                "enabled=true".into(),
            ];
            if preapproves(name, perm) || library.is_some_and(|c| connector_preapproved(c, perm)) {
                fields.push("default_tools_approval_mode=\"approve\"".into());
            }
            if let Some(headers) = object.get("headers").and_then(Value::as_object) {
                // Header values are often credentials: Codex reads each from
                // its own environment, so only the variable's name is on argv.
                let prefix = crate::run_auth::CODEX_HEADER_ENV_PREFIX;
                let mut vars = Vec::new();
                for (key, value) in headers {
                    let value = value.as_str().ok_or("Invalid connector header value")?;
                    let var = format!("{prefix}{}", env_out.len());
                    vars.push(format!("{}={}", toml_string(key), toml_string(&var)));
                    env_out.push((var, value.to_string()));
                }
                fields.push(format!("env_http_headers={{{}}}", vars.join(",")));
            }
            if let Some(seconds) = codex_tool_timeout_secs(name) {
                fields.push(format!("tool_timeout_sec={seconds}"));
            }
            entries.push(format!(
                "{}={{{}}}",
                serde_json::to_string(name).unwrap(),
                fields.join(",")
            ));
            continue;
        }
        // Validated here; started from the run's file by `mcp-launch`.
        object
            .get("command")
            .and_then(Value::as_str)
            .ok_or("Hollow connector requires a stdio command")?;
        let mut fields = launch.fields(name).to_vec();
        fields.push("enabled=true".into());
        // `codex exec` runs with approval policy `never`, which FAILS an MCP
        // call that needs approval ("MCP tool call requires approval, but
        // approval policy is never"). Before Paperclip PR 13 that left every
        // Hollow-owned tool uncallable from Codex. Preapprove the same servers
        // Claude gets through `--allowed-tools`, never hollow_computer (nor
        // hollow_team in plan mode).
        if preapproves(name, perm) || library.is_some_and(|c| connector_preapproved(c, perm)) {
            fields.push("default_tools_approval_mode=\"approve\"".into());
        }
        // Tools no agent gets (the built-in Plaud's login/logout; S8 review).
        let hidden = crate::builtin_connectors::hidden_tools(name);
        if !hidden.is_empty() {
            let list: Vec<String> = hidden.iter().map(|t| toml_string(t)).collect();
            fields.push(format!("disabled_tools=[{}]", list.join(",")));
        }
        // A Hollow app call may wait on the user's card, then a browser
        // sign-in (PR A5), and a bounded consultation (F10, D1) waits for a
        // run permit, then for its teammate up to its deadline: both past
        // Codex's 60 s default tool timeout ([`codex_tool_timeout_secs`]).
        // An approved mail connector's write waits on the person's card
        // (up to 120 s): Codex must wait longer, or a late Allow would send
        // what Codex already gave up on (10-11 B4).
        let mail = object
            .get("args")
            .and_then(Value::as_array)
            .and_then(|a| {
                a.iter()
                    .position(|x| x == "--url")
                    .and_then(|i| a.get(i + 1))
            })
            .and_then(Value::as_str)
            .is_some_and(crate::remote_auth::approved_mail);
        if let Some(seconds) =
            codex_tool_timeout_secs(name).or(mail.then_some(MAIL_TOOL_TIMEOUT_SECS))
        {
            fields.push(format!("tool_timeout_sec={seconds}"));
        }
        if let Some(args) = object.get("args") {
            if !args
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string))
            {
                return Err("Invalid Hollow connector arguments".into());
            }
        }
        if let Some(env) = object.get("env") {
            let env = env
                .as_object()
                .ok_or("Invalid Hollow connector environment")?;
            if !env.values().all(Value::is_string) {
                return Err("Invalid Hollow connector environment value".into());
            }
        }
        entries.push(format!(
            "{}={{{}}}",
            serde_json::to_string(name).unwrap(),
            fields.join(",")
        ));
    }
    Ok(CodexServers {
        overrides: entries.join(","),
        env: env_out,
    })
}

pub(crate) fn codex_server_names(raw: &[u8]) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_slice(raw).map_err(|_| {
        "Could not isolate Codex connectors: invalid local configuration listing.".to_string()
    })?;
    let entries = value.as_array().ok_or_else(|| {
        "Could not isolate Codex connectors: unexpected local configuration listing.".to_string()
    })?;
    entries
        .iter()
        .map(|entry| {
            entry
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    "Could not isolate Codex connectors: missing server name.".to_string()
                })
        })
        .collect()
}

/// The longest native session id a builder passes on. Real ids are UUIDs
/// (36 characters); this is headroom, not a guess at a provider's format.
pub(crate) const MAX_RESUME_ID_LEN: usize = 128;

/// Whether `id` is safe to place right after a resume flag (`--resume`,
/// Codex's `exec resume`, Antigravity's `--conversation`): 1 to
/// [`MAX_RESUME_ID_LEN`] ASCII letters, digits, `-`, `_` or `.`, and no
/// leading `-`, so it can never parse as a flag such as
/// `--dangerously-skip-permissions` (review finding A·R10).
pub(crate) fn is_safe_resume_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_RESUME_ID_LEN
        && !id.starts_with('-')
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The resume id every builder passes: the spec's id only when
/// [`is_safe_resume_id`] accepts it; anything else runs a fresh session. The
/// chat path refuses an unsafe id earlier, with a visible note
/// (`chat_resume::plan_resume`); this is the backstop for every other caller.
fn resume_arg(spec: &RunSpec) -> Option<&str> {
    spec.resume.as_deref().filter(|id| is_safe_resume_id(id))
}

/// The native session the command really resumes: [`resume_arg`], and only
/// on an interactive spec, which is when every builder passes it. A
/// non-interactive spec with a resume id runs a fresh session, so
/// [`contextual_prompt`] holds the configured half back only when this is
/// `Some` (wave 3 review of G2).
fn resumed_id(spec: &RunSpec) -> Option<&str> {
    resume_arg(spec).filter(|_| spec.interactive)
}

/// OB23: the most characters of one Bash tool result Claude Code keeps
/// inline (its own default is 30,000). An inline result stays in the
/// conversation and is re-read on every later call: results over 8,000
/// characters carried 4.4% of all main-thread cache reads. Past the cap
/// Claude gets only the first 2 KB plus the path of a file holding the
/// whole output (probe, CLI 2.1.301): the tail, where cargo and vitest print
/// their failure summary, is dropped. Hence 16,000 rather than 8,000, so
/// most test output still arrives whole. Argv and env are never
/// fingerprinted (`chat_resume`), so this resets no session.
pub const CLAUDE_BASH_MAX_OUTPUT_CHARS: u32 = 16_000;

/// Whether a Claude spec runs tool-free: every [`crate::claude_runner::NO_TOOLS`]
/// entry disallowed, which [`build_claude`] turns into `--tools ""`.
fn claude_tool_free(spec: &RunSpec) -> bool {
    crate::claude_runner::NO_TOOLS
        .iter()
        .all(|tool| spec.disallowed_tools.iter().any(|s| s == tool))
}

/// The Bash output cap `kind`'s command for `spec` carries (OB23), as the
/// run manifest records it (`capabilities.bashOutputMaxChars`): Claude runs
/// that have Bash only. A tool-free run, one that disallows Bash (the
/// heartbeat's tool pass) or one whose `--tools` list leaves it out has no
/// Bash to cap.
pub(crate) fn bash_output_cap(kind: AgentKind, spec: &RunSpec) -> Option<u32> {
    // `NO_TOOLS` holds Bash, so a tool-free run fails the first test.
    let is_bash = |t: &String| t == "Bash" || t.starts_with("Bash(");
    let has_bash = !spec.disallowed_tools.iter().any(|t| t == "Bash")
        && (spec.allowed_tools.is_empty() || spec.allowed_tools.iter().any(is_bash));
    if kind == AgentKind::Claude && has_bash {
        Some(CLAUDE_BASH_MAX_OUTPUT_CHARS)
    } else {
        None
    }
}

/// How the cap reaches Claude Code (OB23): the `BASH_MAX_OUTPUT_LENGTH`
/// env var the installed CLI honours. The one place to change should the
/// route become `--settings '{"bashOutputMaxChars":…}'`.
fn apply_bash_output_cap(cmd: &mut Command, chars: u32) {
    cmd.env("BASH_MAX_OUTPUT_LENGTH", chars.to_string());
}

/// NV9: Claude Code (2.1.325) stops every background shell and agent as the
/// process exits, so a turn that backgrounds work loses it silently; with this
/// set the tool refuses to launch any, and the work stays in the turn.
pub const CLAUDE_DISABLE_BACKGROUND_TASKS_ENV: &str = "CLAUDE_CODE_DISABLE_BACKGROUND_TASKS";

/// NV9: with this set Claude Code (2.1.325) does not retry a refused turn on
/// another model, so the refusal reaches the run and the usage ledger names
/// the model that answered. (`CLAUDE_CODE_DISABLE_REFUSAL_RETRY` is a
/// different switch and is left alone.)
pub const CLAUDE_DISABLE_REFUSAL_FALLBACK_ENV: &str = "CLAUDE_CODE_DISABLE_REFUSAL_FALLBACK";

/// NV9: the fixed run settings of every Claude run: both on, in the
/// environment (never argv), on the local child and, since a runner run's
/// leader copies the command's environment, on the Mac mini too. Claude only;
/// probes (`--help`, the model list, the effort check) build their own
/// command and stay as they are.
fn apply_claude_run_settings(cmd: &mut Command) {
    cmd.env(CLAUDE_DISABLE_BACKGROUND_TASKS_ENV, "1")
        .env(CLAUDE_DISABLE_REFUSAL_FALLBACK_ENV, "1");
}

/// OB22: the env var Claude Code (2.1.301) reads for the context window its
/// auto-compaction works against. A run gets it through `RunExtras::env`
/// (`agent_settings::compaction_env`), so it reaches the child unchanged.
pub const CLAUDE_COMPACT_WINDOW_ENV: &str = "CLAUDE_CODE_AUTO_COMPACT_WINDOW";

/// OB22: Hollow's own carrier for Codex's compaction limit. Codex (0.163.0)
/// reads `model_auto_compact_token_limit` from its config, not from the
/// environment, so a `RunExtras::env` entry under this name becomes
/// `-c model_auto_compact_token_limit=<n>` in [`build_codex`] and is kept
/// off the child's environment. It rides the env so no `RunExtras` field
/// (and no literal of one) changes.
pub const CODEX_COMPACT_LIMIT_ENV: &str = "HOLLOW_CODEX_AUTO_COMPACT_LIMIT";

/// OB22: the compaction window `spec`'s command carries for `kind`, read
/// from its `extras.env`: Claude's [`CLAUDE_COMPACT_WINDOW_ENV`] or Codex's
/// [`CODEX_COMPACT_LIMIT_ENV`] (the last entry wins, as with `Command::env`).
/// `None` when unset or not a number, and for Antigravity and Grok. The run
/// manifest records it.
pub(crate) fn compact_window(kind: AgentKind, spec: &RunSpec) -> Option<u32> {
    // A self-hosted run's comes from its endpoint (Q3), over Settings',
    // on the harness its API style names (Claude, or Codex since Q9).
    if let Some(target) = spec.extras.self_hosted.as_ref() {
        return (target.agent() == Some(kind)).then(|| target.compact_window());
    }
    let key = match kind {
        AgentKind::Claude => CLAUDE_COMPACT_WINDOW_ENV,
        AgentKind::Codex => CODEX_COMPACT_LIMIT_ENV,
        AgentKind::Antigravity | AgentKind::Grok => return None,
    };
    spec.extras
        .env
        .iter()
        .rev()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| value.trim().parse().ok())
}

/// Whether a Claude run reads its messages from stdin (a steerable chat
/// turn): only an interactive run, where the turn owns the pipe.
pub fn live_claude_input(spec: &RunSpec) -> bool {
    spec.interactive && spec.extras.live_input
}

/// Whether a Codex run is a steerable chat turn over `codex app-server`
/// (steering PR 3): only an interactive run, where the turn owns the pipe.
pub fn live_codex_input(spec: &RunSpec) -> bool {
    spec.interactive && spec.extras.live_input
}

/// The one `--settings` value of a Claude run (CX4b): the self-hosted
/// settings (`self_hosted::settings_json`) when there are any, with
/// `autoMemoryDirectory` set to `memory_dir` (the personal clone's
/// `memory/<project>`) when there is one. `None` when neither applies, so
/// the flag is absent and the CLI keeps its default memory folder.
fn claude_settings(base: Option<String>, memory_dir: Option<&Path>) -> Option<String> {
    let Some(dir) = memory_dir else {
        return base;
    };
    let mut value = match base.as_deref() {
        None => serde_json::json!({}),
        Some(json) => {
            match serde_json::from_str::<serde_json::Value>(json) {
                Ok(v) if v.is_object() => v,
                // Never drop the base's pins: leave it as it was, no memory key.
                _ => {
                    eprintln!("[agent] the run's settings aren't a JSON object; auto memory stays default");
                    return base;
                }
            }
        }
    };
    value["autoMemoryDirectory"] = dir.to_string_lossy().into_owned().into();
    Some(value.to_string())
}

/// Whether a Claude run carries the AG4b defer hook: an interactive local
/// chat turn in ask-first mode (`default`) on Claude's own models, with an
/// answers file. Never a runner run (its hook would run on the mini, away
/// from the answers file) or a self-hosted model.
pub fn claude_defers(spec: &RunSpec) -> bool {
    spec.extras.defer_answers.is_some()
        && spec.interactive
        && spec.perm == "default"
        && spec.extras.remote.is_none()
        && spec.extras.self_hosted.is_none()
}

/// Research and read-only tools a headless accept-edits or default Claude run
/// may use without a prompt (see [`build_claude`]).
const READ_ONLY_PREAPPROVALS: &[&str] = &[
    "WebFetch",
    "WebSearch",
    "Bash(gh repo view:*)",
    "Bash(gh pr view:*)",
    "Bash(gh pr list:*)",
    "Bash(gh pr diff:*)",
    "Bash(gh issue view:*)",
    "Bash(gh issue list:*)",
    "Bash(gh search:*)",
    "Bash(printenv HOLLOW_SCRATCH_DIR)",
];

/// Claude's arguments for `spec`. Returns the appended system prompt a runner
/// run carries in its header instead (R5b).
fn build_claude(cmd: &mut Command, spec: &RunSpec, config_file: Option<&Path>) -> Option<String> {
    let perm = if spec.perm.is_empty() {
        "acceptEdits"
    } else {
        &spec.perm
    };
    // A steerable chat turn's prompt is its first stdin line (`live_input`).
    // Q-R33: any other local run reads it from stdin too ([`stdin_prompt`]),
    // so `ps` never shows it. A runner run's prompt goes in the leader's
    // header too (`Handover::stdin`), which the runner makes the agent's stdin.
    if live_claude_input(spec) {
        cmd.arg("-p").arg("--input-format").arg("stream-json");
    } else if stdin_prompt(AgentKind::Claude, spec).is_some() {
        cmd.arg("-p");
    } else {
        cmd.arg("-p").arg(&spec.prompt);
    }
    cmd.arg("--output-format")
        .arg("stream-json")
        .arg("--verbose");
    // Paperclip PR 15a: Claude in acceptEdits refuses writes outside the cwd
    // and its added directories, so the scratch folder is added. The flag is
    // variadic, so it sits right before `--permission-mode`, which ends it.
    if let Some(dir) = spec.extras.scratch.as_ref().and_then(|s| s.dir()) {
        cmd.arg("--add-dir").arg(dir);
    }
    for dir in &spec.extras.extra_dirs {
        cmd.arg("--add-dir").arg(dir);
    }
    cmd.arg("--permission-mode").arg(perm);
    if spec.interactive {
        cmd.arg("--include-partial-messages");
        if let Some(sid) = resumed_id(spec) {
            match crate::session_import::native_resume::fork_args(sid)
                .filter(|_| spec.extras.fork_resume)
            {
                Some(args) => {
                    cmd.args(args);
                }
                None => {
                    cmd.arg("--resume").arg(sid);
                }
            }
        }
    } else {
        cmd.arg("--no-session-persistence");
        if spec.usage_stream {
            cmd.arg("--include-partial-messages");
        }
        if let Some(usd) = spec
            .max_budget_usd
            .filter(|usd| usd.is_finite() && *usd > 0.0)
        {
            cmd.arg("--max-budget-usd")
                .arg(format!("{:.2}", usd.max(0.01)));
        }
    }
    // A self-hosted run names the endpoint's served model (Q1), whatever
    // the spec's model says.
    let model = match spec.extras.self_hosted.as_ref() {
        Some(target) => Some(target.served_model.as_str()),
        None => spec.model.as_deref(),
    };
    if let Some(m) = model.filter(|s| !s.is_empty()) {
        cmd.arg("--model").arg(m);
    }
    // Paperclip PR 14a. Accepted per invocation, including on `--resume`.
    if let Some(effort) = run_effort(AgentKind::Claude, spec).effort {
        cmd.arg("--effort").arg(effort);
    }
    // The permission preapprovals: the spec's own list plus the preapproved
    // Hollow-owned servers attached to this run (Paperclip PR 13), which a
    // headless `-p` run would otherwise deny. They are NOT added to the spec's
    // allowed_tools, which would force the `--tools` restriction below and
    // strip every built-in from an unrestricted run.
    let mut preapproved: Vec<&str> = spec.allowed_tools.iter().map(String::as_str).collect();
    for token in owned_preapprovals(spec.mcp_config.as_deref(), perm) {
        if !preapproved.contains(&token) {
            preapproved.push(token);
        }
    }
    let shared_tokens: Vec<String> = spec
        .extras
        .shared_connectors
        .iter()
        .filter(|c| connector_preapproved(c, perm))
        .map(|c| format!("mcp__{}", c.id))
        .collect();
    for token in &shared_tokens {
        if !preapproved.contains(&token.as_str()) {
            preapproved.push(token);
        }
    }
    // 2026-11-19: a headless default or accept-edits run can't be asked, so a
    // pure research turn (clone, read a repo, fetch a page) was refused every
    // call and did nothing. Web reads and read-only `gh` calls write nothing,
    // so they are preapproved there. `git clone` and `git -C` are not: their
    // flags can pick any destination or run configured commands (review).
    if matches!(perm, "acceptEdits" | "default") {
        for token in READ_ONLY_PREAPPROVALS {
            if !preapproved.contains(token) {
                preapproved.push(token);
            }
        }
    }
    if !preapproved.is_empty() {
        cmd.arg("--allowed-tools");
        for t in &preapproved {
            cmd.arg(t);
        }
    }
    if claude_tool_free(spec) {
        cmd.arg("--tools").arg("").arg("--disable-slash-commands");
    } else if !spec.allowed_tools.is_empty() {
        // --allowed-tools is a permission preapproval list, not a tool-set
        // boundary. --tools actually removes non-listed built-in tools.
        let mut builtins: Vec<_> = spec
            .allowed_tools
            .iter()
            .filter(|t| !t.starts_with("mcp__"))
            .map(String::as_str)
            .collect();
        // Claude Code 2.1.283 defers MCP tools behind the built-in
        // ToolSearch: the model must select an MCP tool before calling it
        // (recorded 2026-09-25). A restricted built-in list without it would
        // make every attached MCP tool unreachable.
        if has_mcp_servers(spec.mcp_config.as_deref()) && !builtins.contains(&"ToolSearch") {
            builtins.push("ToolSearch");
        }
        cmd.arg("--tools").arg(builtins.join(","));
    }
    // OB23: chat, delegate and automation runs that have Bash (the
    // heartbeat disallows it). Inherited by the agent's own shell, so a
    // nested `claude` it spawns is capped too. Env set here
    // survives `build_with_codex_servers` (it only adds PATH and the
    // spec's own env after) and the account step (it touches only the
    // token and account vars).
    if let Some(chars) = bash_output_cap(AgentKind::Claude, spec) {
        apply_bash_output_cap(cmd, chars);
    }
    // Plus the attached servers' tools no agent gets (overnight S8 review:
    // the built-in Plaud's login/logout).
    let mut disallowed: Vec<String> = spec.disallowed_tools.clone();
    for t in hidden_mcp_tools(spec.mcp_config.as_deref()) {
        if !disallowed.contains(&t) {
            disallowed.push(t);
        }
    }
    // The org context clone is read only (CX5): no file edit lands in it.
    for rule in crate::context_layers::org_write_rules() {
        if !disallowed.contains(&rule) {
            disallowed.push(rule);
        }
    }
    if !disallowed.is_empty() {
        cmd.arg("--disallowed-tools");
        for t in &disallowed {
            cmd.arg(t);
        }
    }
    // One `--settings` only (a second replaces the first): a self-hosted
    // run's pins the endpoint over any settings file (Q1 review), and a
    // local run's names its auto-memory folder (CX4b). No secret in either,
    // so it may ride argv.
    let memory_dir = if spec.extras.remote.is_none() {
        crate::context_layers::auto_memory_dir(&spec.cwd)
    } else {
        None
    };
    let settings = claude_settings(
        spec.extras
            .self_hosted
            .as_ref()
            .map(crate::self_hosted::settings_json),
        memory_dir.as_deref(),
    );
    // AG4b: the defer hook rides the same one `--settings`. Its command line
    // holds two paths and no secret.
    let hook = spec
        .extras
        .defer_answers
        .as_deref()
        .filter(|_| claude_defers(spec))
        .and_then(|answers| {
            let exe = std::env::current_exe().ok()?;
            Some(crate::claude_defer::hook_command(&exe, answers))
        });
    let settings = match hook {
        Some(command) => crate::claude_defer::settings_with_hook(settings, &command),
        None => settings,
    };
    if let Some(json) = settings {
        cmd.arg("--settings").arg(json);
    }
    // R5b: a local run's appended prompt rides a private descriptor; a runner
    // run's is returned for the leader's header, where the runner makes the
    // same hand-over itself (`remote_session`).
    isolate_claude_with(
        cmd,
        config_file,
        spec.append_system_prompt.as_deref(),
        &run_policy(AgentKind::Claude, spec),
        if spec.extras.remote.is_some() {
            AppendTo::Withheld
        } else {
            AppendTo::Descriptor
        },
    )
}

fn build_codex(cmd: &mut Command, spec: &RunSpec) {
    // `--skip-git-repo-check` is always safe (a no-op inside a repo) and lets
    // codex run against a non-git vault. The prompt is positional and must
    // come after all options. `codex exec` is non-interactive by design — it
    // has no `-a/--ask-for-approval` flag and never prompts, so the sandbox
    // policy alone governs what the run can touch.
    cmd.args([
        "--disable",
        "apps",
        "--disable",
        "plugins",
        "--disable",
        "remote_plugin",
    ]);
    // Self-hosted models Q9: Hollow's provider at the forwarder, the served
    // model and its window. Top-level, so `exec resume` gets them too.
    codex_self_hosted_overrides(cmd, spec);
    // Paperclip PR 14a: a top-level config override, so it applies to
    // `exec` and to `exec resume` alike (the resume sub-subcommand takes a
    // narrower flag set). The value is a JSON (TOML-compatible) string.
    if let Some(effort) = run_effort(AgentKind::Codex, spec).effort {
        cmd.arg("-c").arg(format!(
            "model_reasoning_effort={}",
            serde_json::to_string(&effort).expect("effort level")
        ));
    }
    // OB22: the compaction window from Settings, carried in the run's env
    // ([`CODEX_COMPACT_LIMIT_ENV`]). Top-level like the effort, so a resumed
    // turn gets it too; parsed as a number, so nothing else reaches argv.
    if let Some(limit) = compact_window(AgentKind::Codex, spec) {
        cmd.arg("-c")
            .arg(format!("model_auto_compact_token_limit={limit}"));
    }
    // `auto` is Codex's own "Approve for me" (`--approve-for-me`, codex-cli
    // 0.153.4): approval requests go to its automatic reviewer instead of
    // failing under `exec`'s default `never`. Set as config overrides so a
    // resumed turn gets them too (user decision, 2026-10-02: auto means
    // auto in every agent).
    if spec.perm == "auto" {
        cmd.arg("-c").arg("approval_policy=\"on-request\"");
        cmd.arg("-c").arg("approvals_reviewer=\"auto_review\"");
    }
    // Auto's writable roots and network (`codex_auto_sandbox_overrides`):
    // top-level too, so `exec resume` (which has no `--add-dir`) gets them.
    let auto_values = codex_auto_sandbox_overrides(spec);
    let profile = codex_auto_uses_profile(&auto_values);
    for value in auto_values {
        cmd.arg("-c").arg(value);
    }
    let resume_id = resumed_id(spec);
    // `codex exec resume` takes no -s/--sandbox, but it reads the
    // `sandbox_mode` config key, so a resumed chat runs in the mode it was
    // switched to rather than the one it started in.
    if resume_id.is_some() && spec.perm != "bypassPermissions" && !profile {
        cmd.arg("-c")
            .arg(format!("sandbox_mode=\"{}\"", codex_sandbox(&spec.perm)));
    }
    cmd.arg("exec");
    if let Some(sid) = resume_id {
        // `codex exec resume` is a sub-subcommand with a NARROWER flag set: it
        // accepts --json / --skip-git-repo-check / -m / the bypass flag, but
        // NOT -s/--sandbox (the sandbox comes from the config override above).
        cmd.arg("resume")
            .arg(sid)
            .arg("--json")
            .arg("--skip-git-repo-check");
        if spec.perm == "bypassPermissions" {
            cmd.arg("--dangerously-bypass-approvals-and-sandbox");
        }
    } else {
        cmd.arg("--json").arg("--skip-git-repo-check");
        if spec.perm == "bypassPermissions" {
            cmd.arg("--dangerously-bypass-approvals-and-sandbox");
        } else if !profile {
            cmd.arg("-s").arg(codex_sandbox(&spec.perm));
        }
    }
    if let Some(m) = spec.model.as_deref().filter(|s| !s.is_empty()) {
        cmd.arg("-m").arg(m);
    }
    if !spec.interactive {
        cmd.arg("--ephemeral");
    }
    // Q-R33: `-` makes `codex exec` (and `exec resume`) read the prompt
    // from stdin ([`stdin_prompt`]); a runner run keeps it as an argument.
    if stdin_prompt(AgentKind::Codex, spec).is_some() {
        cmd.arg("-");
    } else {
        cmd.arg(contextual_prompt(AgentKind::Codex, spec));
    }
}

/// Q-R33: the prompt a run reads from stdin instead of argv, which every
/// process on the Mac can read with `ps`. Claude (`-p` with no prompt
/// argument reads stdin) and Codex (`exec -` and `exec resume <id> -`)
/// take it for every local run that isn't a steerable turn (those already
/// write their messages to stdin). A runner run has one too (R5b): its
/// leader puts it in the header it ships to the mini, and the runner makes it
/// the agent's stdin there. Grok takes it as `--prompt-file /dev/stdin`
/// (R5b). Antigravity takes one stream-json user message (R5 residual, probed
/// 2026-11-17: `-p=` plus `--input-format stream-json`). `None` for a
/// steerable turn.
pub fn stdin_prompt(kind: AgentKind, spec: &RunSpec) -> Option<String> {
    match kind {
        AgentKind::Claude if !live_claude_input(spec) => Some(spec.prompt.clone()),
        AgentKind::Codex if !live_codex_input(spec) => {
            Some(contextual_prompt(AgentKind::Codex, spec))
        }
        // R5b: Grok reads `--prompt-file /dev/stdin` (see [`build_grok`]).
        // OC1: an ACP turn sends it in `session/prompt` instead.
        AgentKind::Grok if !grok_acp(spec) => Some(contextual_prompt(AgentKind::Grok, spec)),
        // R5 residual: Antigravity reads one stream-json user message.
        // 2026-11-20: an ACP turn sends it in `session/prompt` instead.
        AgentKind::Antigravity if !antigravity_acp(spec) => Some(antigravity_stdin_message(
            &contextual_prompt(AgentKind::Antigravity, spec),
        )),
        _ => None,
    }
}

/// The one NDJSON line `agy --input-format stream-json` reads as a prompt
/// (probed 2026-11-17: the `event` field is required, `message` is an object
/// with `content`, and end of input ends the session after its turn).
fn antigravity_stdin_message(prompt: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({"event": "user", "message": {"content": prompt}})
    )
}

#[cfg(test)]
thread_local! {
    static LAST_STDIN_PROMPT: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Test-only: the prompt the last command built on this thread put on its
/// stdin (Q-R33), taken, so a test that reads a run's prompt still can.
#[cfg(test)]
pub(crate) fn take_last_stdin_prompt() -> Option<String> {
    LAST_STDIN_PROMPT.with(|last| last.borrow_mut().take())
}

/// Q-R33: an unlinked private file holding `prompt`, rewound, for a run's
/// stdin. It has no name once this returns, so no other process can open it,
/// and as a regular file it can't block the child or the caller the way an
/// unread pipe could.
pub(crate) fn prompt_stdin_file(prompt: &str) -> std::io::Result<std::fs::File> {
    use std::io::{Seek, Write};
    let mut file = tempfile::tempfile()?;
    file.write_all(prompt.as_bytes())?;
    file.flush()?;
    file.rewind()?;
    Ok(file)
}

/// Self-hosted models Q9: a self-hosted Codex run's provider overrides
/// ([`crate::self_hosted::codex_overrides`]), one `-c` each; nothing for any
/// other run, so its argv is unchanged.
fn codex_self_hosted_overrides(cmd: &mut Command, spec: &RunSpec) {
    if let Some(target) = spec.extras.self_hosted.as_ref() {
        for value in crate::self_hosted::codex_overrides(target) {
            cmd.arg("-c").arg(value);
        }
    }
}

/// The effort `spec` runs at on `kind` ([`crate::effort::resolve`]); a
/// self-hosted run's from its endpoint's levels, `medium` by default (Q3).
pub(crate) fn run_effort(kind: AgentKind, spec: &RunSpec) -> crate::effort::Choice {
    if let Some(target) = spec.extras.self_hosted.as_ref() {
        let effort = target.effort(spec.extras.effort.as_deref());
        let ultracode = spec
            .extras
            .effort
            .as_deref()
            .and_then(crate::effort::canonical)
            == Some(crate::effort::ULTRACODE);
        let note = ultracode.then(|| {
            format!(
                "Ultracode isn't available on {}, a self-hosted model; this turn ran at {}.",
                target.name,
                effort.as_deref().unwrap_or("its default")
            )
        });
        return crate::effort::Choice {
            effort,
            note,
            ..crate::effort::Choice::default()
        };
    }
    crate::effort::resolve(kind, spec.model.as_deref(), spec.extras.effort.as_deref())
}

/// Codex's sandbox for a non-bypass mode: read-only for `plan`, a
/// writable workspace otherwise.
fn codex_sandbox(perm: &str) -> &'static str {
    if perm == "plan" {
        "read-only"
    } else {
        "workspace-write"
    }
}

/// The extra folders a remote run may write on the runner: the copy of a
/// build workspace's vault folder there (Phase 3), never a path of this Mac.
fn remote_extra_dirs(target: &crate::remote_session::RemoteTarget) -> Vec<PathBuf> {
    match (&target.folder, target.git_worktree) {
        (Some(folder), true) => vec![PathBuf::from(&folder.dir)],
        _ => Vec::new(),
    }
}

/// Codex auto's sandbox additions (2026-11-12, "Codex's Auto works for
/// builds"), one `-c` value each: the run's extra folders and its
/// worktree's git folders as writable roots
/// ([`codex_auto_writable_roots`]), and network access on (git push, `gh`,
/// package registries, the Mac mini runner). Codex's own keys under
/// `sandbox_workspace_write`, which apply only to the `workspace-write`
/// sandbox auto runs in. Every other mode gets nothing: plan stays
/// read-only without network, default and acceptEdits keep Codex's
/// defaults, bypass has no sandbox, except that default and acceptEdits
/// carry the nested-`.git` deny alone ([`codex_edit_profile_overrides`], N22).
/// Roots are JSON strings, which TOML
/// reads the same way, so a space, apostrophe or backslash in a path stays
/// inside its string.
pub(crate) fn codex_auto_sandbox_overrides(spec: &RunSpec) -> Vec<String> {
    if spec.perm != "auto" {
        return codex_edit_profile_overrides(spec).unwrap_or_default();
    }
    let roots = codex_auto_writable_roots(spec);
    if let Some(values) = codex_auto_profile_overrides(spec, &roots) {
        return values;
    }
    let mut values = Vec::new();
    if !roots.is_empty() {
        let quoted: Vec<String> = roots
            .iter()
            .map(|root| serde_json::to_string(root).expect("a path string"))
            .collect();
        values.push(format!(
            "sandbox_workspace_write.writable_roots=[{}]",
            quoted.join(",")
        ));
    }
    values.push("sandbox_workspace_write.network_access=true".to_string());
    values
}

/// Name of the Codex permission profile a local auto run in a linked
/// worktree runs under ([`codex_auto_profile_overrides`]).
const CODEX_AUTO_PROFILE: &str = "hollow_auto";

/// Whether `values` ([`codex_auto_sandbox_overrides`]) select the profile
/// instead of the `workspace-write` sandbox: then the run passes no `-s`,
/// no `sandbox_mode` and no thread `sandbox` (a thread's `sandbox` would
/// override the profile, codex-cli 0.167.2).
pub(crate) fn codex_auto_uses_profile(values: &[String]) -> bool {
    values.iter().any(|v| v.starts_with("default_permissions="))
}

/// Whether a built Codex command (`cmd`) selects the auto profile: a
/// `-c default_permissions=...` on its own argv (2026-11-13, "decide the
/// profile once"). The app-server's thread reads the decision from here,
/// the process config it really got, instead of checking the worktree a
/// second time: the two checks used to run apart, and another writer
/// changing the layout between them could leave a thread with no profile
/// (the process didn't get one) and no `sandbox` (the thread thought it
/// had). Anything else here, a wrapped or remote command included, reads
/// as no profile, so the thread names its sandbox.
pub(crate) fn codex_command_uses_profile(cmd: &Command) -> bool {
    let args: Vec<&std::ffi::OsStr> = cmd.as_std().get_args().collect();
    args.windows(2).any(|w| {
        w[0] == "-c"
            && w[1]
                .to_str()
                .is_some_and(|v| codex_auto_uses_profile(&[v.to_string()]))
    })
}

/// The thread `sandbox` an app-server run names: none only when its process
/// runs a profile (auto's or an edit mode's, [`codex_edit_profile_overrides`]) (`profile`, from [`codex_command_uses_profile`]),
/// since a thread `sandbox` would override it; otherwise its mode's
/// sandbox, so a thread is never left with neither.
pub(crate) fn codex_app_thread_sandbox(perm: &str, profile: bool) -> &'static str {
    if profile && matches!(perm, "auto" | "acceptEdits" | "default" | "") {
        ""
    } else {
        codex_app_server_policy(perm).sandbox
    }
}

/// A local auto run in a checked linked worktree runs under a permission
/// profile (2026-11-12, "close the planted-payload routes"): the same grants
/// as the `workspace-write` roots (`roots`) and network, plus the worktree
/// git folder's `commondir` and `gitdir` files read-only, which a
/// `workspace-write` root can't express. A rewritten `commondir` would point
/// the person's own terminal git at a decoy folder whose `core.fsmonitor`
/// runs; read-only there is no 200 ms window for [`codex_worktree_guard_tick`]
/// to close. The `index` stays writable on purpose: `git add` and `git commit`
/// replace it (probed: with it read-only both fail with "unable to write new
/// index file"); its planted-submodule route is closed in the repository
/// config instead ([`harden_common_submodule_ignore`]). Codex's own `:workspace`
/// base keeps the cwd, `/tmp` and `$TMPDIR` writable and each root's `.git`
/// read-only. `None` (the `workspace-write` sandbox as before) when no
/// worktree git folder is granted. Keys are JSON strings, which TOML reads
/// the same way.
fn codex_auto_profile_overrides(spec: &RunSpec, roots: &[String]) -> Option<Vec<String>> {
    if spec.extras.remote.is_some() {
        return None;
    }
    let (gitdir, _) = checked_linked_worktree(&spec.cwd, &spec.extras.extra_dirs)?;
    let gitdir = gitdir.canonicalize().unwrap_or(gitdir);
    // Only when the roots really hold the git folder (a path Codex can't be
    // given, such as one with a double quote, falls back to the sandbox
    // that also grants nothing there).
    let gitdir_text = gitdir.to_str()?;
    if !roots.iter().any(|r| r == gitdir_text) {
        return None;
    }
    // Codex reads a key with a glob character (`*?[]{}\`) as a glob, which
    // takes only `deny`, and refuses the profile: such a path keeps the
    // sandbox that grants it as a root.
    let is_glob = |text: &str| text.chars().any(|c| "*?[]{}\\".contains(c));
    if roots.iter().any(|r| is_glob(r)) || is_glob(gitdir_text) {
        return None;
    }
    let key = |text: &str| serde_json::to_string(text).expect("a path string");
    let mut entries: Vec<String> = roots
        .iter()
        .map(|root| format!("{}=\"write\"", key(root)))
        .collect();
    // `config.worktree` too, whether or not it exists (Q-R6, 2026-11-13):
    // git reads it only with `extensions.worktreeConfig`, which the layout
    // check refuses, but a turn must not be able to plant `core.fsmonitor`
    // or `core.hooksPath` there for a git that reads it anyway. Probed
    // (codex-cli 0.167.2): creating, renaming over or symlinking it is
    // refused, commits still work.
    for name in ["commondir", "gitdir", "config.worktree"] {
        let file = gitdir.join(name);
        entries.push(format!("{}=\"read\"", key(file.to_str()?)));
    }
    // The org context clone is read only (CX5b): `read` beats a `write`
    // folder that holds it (probed, codex-cli 0.169.0: a file can't be
    // made there, in the cwd or out of it). A path Codex can't carry as a
    // key (a glob character, a quote, a control character) is left out.
    if let Some(org) = crate::context_layers::org_root() {
        if let Some(text) = org
            .to_str()
            .filter(|t| !is_glob(t) && !t.chars().any(|c| c == '"' || c.is_control()))
        {
            entries.push(format!("{}=\"read\"", key(text)));
        }
    }
    entries.extend(codex_nested_git_denies(spec)?);
    Some(vec![
        format!("default_permissions=\"{CODEX_AUTO_PROFILE}\""),
        format!("permissions.{CODEX_AUTO_PROFILE}.extends=\":workspace\""),
        format!(
            "permissions.{CODEX_AUTO_PROFILE}.filesystem={{{}}}",
            entries.join(",")
        ),
        format!("permissions.{CODEX_AUTO_PROFILE}.network.enabled=true"),
    ])
}

/// A nested `.git` can't be created or renamed into place (a decoy
/// repository for a planted submodule entry, whose config would run
/// `core.fsmonitor` in the person's own `git status`/`add -A`): `*/.git` and
/// `*/**/.git` under the worktree, never the top-level `.git` the worktree
/// itself needs. Probed: `git init`, a `.git` file, a deep `.git` and
/// `--separate-git-dir` are refused (reviewer: any case of the name too),
/// commits work. **Known gap:** the deny matches only the created or
/// renamed path, so a repository built in a writable temp folder (`/tmp`,
/// `$TMPDIR`) and then moved into the worktree (`git init /tmp/d; mv /tmp/d
/// <wt>/sub`) is not caught; the index guard and
/// `harden_common_submodule_ignore` still apply to it. Pinned by the live
/// fixture's `MOVED_IN_GIT_OK` assertion. Fix idea, a later row: make the
/// temp folders read-only except the run's scratch folder.
/// `None` when the cwd has a glob character (Codex would read the key as a
/// glob and refuse the profile). The filesystem entries, shared by auto's
/// profile and the edit modes' ([`codex_edit_profile_overrides`]).
fn codex_nested_git_denies(spec: &RunSpec) -> Option<Vec<String>> {
    let top = spec.cwd.canonicalize().unwrap_or_else(|_| spec.cwd.clone());
    let top = top.to_str()?;
    if top.chars().any(|c| "*?[]{}\\".contains(c)) {
        return None;
    }
    Some(
        [format!("{top}/*/.git"), format!("{top}/*/**/.git")]
            .iter()
            .map(|pattern| {
                format!(
                    "{}=\"deny\"",
                    serde_json::to_string(pattern).expect("a path string")
                )
            })
            .collect(),
    )
}

/// The edit modes' permission profile (N22, 2026-11-15): default and
/// acceptEdits run `workspace-write` with no network and no extra roots, but
/// a turn there could still make a decoy nested repository (the index guard
/// only removes its gitlink). This is Codex's own `:workspace` base with the
/// same nested-`.git` deny as auto and nothing else: no roots, `network`
/// off, so the grant is no wider than the sandbox it replaces. Local runs
/// only (a runner's paths aren't this Mac's); `None` keeps the plain
/// sandbox (plan is read-only, bypass has none, a glob in the cwd can't be a
/// profile key). Like auto's, it names no `-s` and no thread `sandbox`.
fn codex_edit_profile_overrides(spec: &RunSpec) -> Option<Vec<String>> {
    if !matches!(spec.perm.as_str(), "acceptEdits" | "default" | "") || spec.extras.remote.is_some()
    {
        return None;
    }
    let entries = codex_nested_git_denies(spec)?;
    Some(vec![
        format!("default_permissions=\"{CODEX_AUTO_PROFILE}\""),
        format!("permissions.{CODEX_AUTO_PROFILE}.extends=\":workspace\""),
        format!(
            "permissions.{CODEX_AUTO_PROFILE}.filesystem={{{}}}",
            entries.join(",")
        ),
        format!("permissions.{CODEX_AUTO_PROFILE}.network.enabled=false"),
    ])
}

/// The folders a Codex auto run may write besides its cwd (which
/// `workspace-write` already allows), in order, once each, absolute only
/// (Codex refuses a relative root and would not start) and never one with
/// a double quote or a control character (codex-cli 0.167.2 writes a root
/// into its seatbelt profile unescaped, so a `"` breaks the profile and
/// every command of the run fails with `sandbox-exec: unbound variable`;
/// such a folder stays readable, as before): the run's
/// `extra_dirs` (a build workspace's vault folder) and, when the cwd is a
/// checked linked git worktree, its git folder and the common folder's
/// `objects`, `refs` and `logs` ([`linked_worktree_git_dirs`]). A
/// remote run's are the runner's paths ([`remote_extra_dirs`]) and no git
/// folder: nothing of this Mac reaches the runner.
pub(crate) fn codex_auto_writable_roots(spec: &RunSpec) -> Vec<String> {
    let local: Vec<PathBuf> = match spec.extras.remote.as_ref() {
        Some(target) => remote_extra_dirs(target),
        None => spec
            .extras
            .extra_dirs
            .iter()
            .cloned()
            .chain(linked_worktree_git_dirs(&spec.cwd, &spec.extras.extra_dirs))
            .map(|dir| dir.canonicalize().unwrap_or(dir))
            .collect(),
    };
    // Never the org context clone or a folder in it (CX5b: read only).
    let org = crate::context_layers::org_root();
    let mut roots: Vec<String> = Vec::new();
    for dir in local {
        if org.as_ref().is_some_and(|org| dir.starts_with(org)) {
            continue;
        }
        let Some(text) = dir.to_str() else { continue };
        let breaks_seatbelt = text.chars().any(|c| c == '"' || c.is_control());
        if dir.is_absolute() && !breaks_seatbelt && !roots.iter().any(|r| r == text) {
            roots.push(text.to_string());
        }
    }
    roots
}

/// The git folders a Codex auto run in a linked git worktree may write so
/// a commit and a push work: the worktree's own git folder
/// (`<common>/worktrees/<name>`: index, HEAD, its reflog, FETCH_HEAD) and
/// only `objects`, `refs` and `logs` of the repository's common git folder.
/// Never the common folder itself, so its `config` and `hooks` stay
/// read-only (a hook or `core.fsmonitor` written there would run outside
/// the sandbox at the next git call, Hollow's or the person's). Codex's
/// `workspace-write` refuses the worktree's git folder unless it is a root
/// itself (probed with `codex sandbox`, codex-cli 0.167.2).
///
/// Everything here is read from files the agent can write (`<cwd>/.git`
/// and the folders it names), so nothing is granted unless every check
/// holds, after resolving symlinks ([`checked_linked_worktree`]); any
/// failed check grants nothing and logs nothing.
fn linked_worktree_git_dirs(cwd: &Path, extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    // An earlier auto turn's tampering is undone before anything is read.
    restore_codex_worktree_git(cwd);
    let Some((gitdir, common)) = checked_linked_worktree(cwd, extra_dirs) else {
        return Vec::new();
    };
    let mut dirs = vec![gitdir];
    for sub in ["objects", "refs", "logs"] {
        // A subfolder that is a symlink out of the common folder is refused.
        match common.join(sub).canonicalize() {
            Ok(dir) if dir.parent() == Some(common.as_path()) && dir.is_dir() => dirs.push(dir),
            Ok(_) => return Vec::new(),
            // `logs` is missing until the first reflog entry; git makes it,
            // but under a sandbox it can't: grant what exists.
            Err(_) if sub == "logs" => {}
            Err(_) => return Vec::new(),
        }
    }
    dirs
}

/// What a Codex auto turn could change in the worktree git folder it may
/// write, as it was when the folder was granted (delta review 2026-11-12):
/// `<gitdir>/commondir` and `<gitdir>/gitdir`, which git reads on every
/// call in that worktree. Rewritten, `commondir` would point git (Hollow's
/// own calls, or the person's) at a folder whose `config` runs a program
/// (`core.fsmonitor`) outside the sandbox.
#[derive(Debug, Clone)]
struct WorktreeGitSnapshot {
    cwd: PathBuf,
    gitdir: PathBuf,
    common: PathBuf,
    commondir: Vec<u8>,
    back: Vec<u8>,
    /// The submodule (`160000`) paths in `HEAD`'s tree when the worktree was
    /// armed: the only ones the index may hold ([`scrub_planted_gitlinks`]).
    gitlinks: std::sync::Arc<Vec<Vec<u8>>>,
    /// The [`IndexStamp`] last scanned, so the guard runs git only when the
    /// index (or a shared index it may link) changed.
    index_seen: std::sync::Arc<std::sync::Mutex<Option<IndexStamp>>>,
    /// The stamp of an index whose scan or removal failed, and when to try it
    /// again: a failure is never remembered as "clean" (the turn can break git
    /// on purpose, then plant), only throttled to one retry per
    /// [`SCRUB_RETRY`] while that index doesn't change.
    index_failed: std::sync::Arc<std::sync::Mutex<Option<(IndexStamp, std::time::Instant)>>>,
    /// Set by [`forget_codex_worktree_guard`]; checked right before each
    /// write, so a pass that listed the snapshot before a `forget` never
    /// writes after it (the pass does its file work outside the lock).
    forgotten: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// What git reads as the index, for the guard's change check: length,
/// inode and change time of `index`, plus every `sharedindex.*` beside it
/// (Q-R5, 2026-11-13). A split index (`git update-index --split-index`, no
/// config needed) keeps its entries in `<gitdir>/sharedindex.<hash>`, which
/// the turn can write; git doesn't verify that file's checksum on a normal
/// read (only the trailing hash, which is copied), so a patched shared index
/// adds a `160000` entry with `index` untouched (probed, git 2.54). Each
/// shared file counts by name, its own inode and change time and, through a
/// symlink, its target's length, inode and change time.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexStamp {
    index: (u64, u64, i64, i64),
    shared: Vec<(std::ffi::OsString, [i64; 7])>,
}

/// The `sharedindex.*` part of [`IndexStamp`] for `gitdir`, sorted by name.
fn shared_index_stamps(gitdir: &Path) -> Vec<(std::ffi::OsString, [i64; 7])> {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(gitdir) else {
        return Vec::new();
    };
    let mut stamps: Vec<_> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            if !name.as_encoded_bytes().starts_with(b"sharedindex.") {
                return None;
            }
            let path = entry.path();
            let own = std::fs::symlink_metadata(&path).ok()?;
            let target = std::fs::metadata(&path).ok();
            let target = |f: fn(&std::fs::Metadata) -> i64| target.as_ref().map_or(-1, f);
            Some((
                name,
                [
                    own.ino() as i64,
                    own.ctime(),
                    own.ctime_nsec(),
                    target(|m| m.size() as i64),
                    target(|m| m.ino() as i64),
                    target(|m| m.ctime()),
                    target(|m| m.ctime_nsec()),
                ],
            ))
        })
        .collect();
    stamps.sort();
    stamps
}

/// The snapshots of the worktrees a Codex auto run was granted, by git
/// folder. Kept while Hollow runs; [`codex_worktree_guard_tick`] puts the
/// files back, so how the turn ended (done, failed, stopped, timed out,
/// its process killed) doesn't matter.
static WORKTREE_GIT_SNAPSHOTS: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<PathBuf, WorktreeGitSnapshot>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// How often the guard puts tampered files back while Hollow runs.
const WORKTREE_GUARD_EVERY: Duration = Duration::from_millis(200);

/// How long a failed index scan or removal waits before its next try.
const SCRUB_RETRY: Duration = Duration::from_secs(30);

/// Snapshot the git files of `cwd`'s worktree when a local Codex auto run
/// is about to be granted them (its validated layout, as
/// [`linked_worktree_git_dirs`] grants it), and start the guard thread
/// once. Nothing for any other run.
fn arm_codex_worktree_guard(cwd: &Path, extra_dirs: &[PathBuf]) {
    restore_codex_worktree_git(cwd);
    // Text an earlier turn left while no guard ran (Hollow quit, or
    // crashed) is put back to git's own, when the layout holds without it.
    repair_worktree_git_texts(cwd, extra_dirs);
    // Only canonical text is ever snapshotted (final review 2026-11-12: a
    // `commondir` naming a symlink the agent can repoint resolves to the
    // right folder today and to a decoy tomorrow).
    let Some((gitdir, common)) = checked_linked_worktree(cwd, extra_dirs) else {
        return;
    };
    let Some(layout) = worktree_layout(cwd, extra_dirs) else {
        return;
    };
    harden_common_submodule_ignore(&common);
    let (commondir, back) = canonical_git_texts(&layout.dot);
    let mut snapshot = WorktreeGitSnapshot {
        cwd: layout.cwd,
        gitdir: gitdir.clone(),
        common,
        commondir,
        back,
        gitlinks: Default::default(),
        index_seen: Default::default(),
        index_failed: Default::default(),
        forgotten: Default::default(),
    };
    // What a hostile turn left in the index while no guard ran is removed
    // now; later changes are the guard's.
    // The allowed set is fixed the first time Hollow arms a worktree and kept
    // across later spawns: `HEAD` is writable by the turn (review
    // 2026-11-12), so it can't be re-read as the truth.
    snapshot.gitlinks = match WORKTREE_GIT_SNAPSHOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&gitdir)
    {
        Some(old) => old.gitlinks.clone(),
        None => std::sync::Arc::new(
            snapshot_git(&snapshot, &["ls-tree", "-r", "-z", "HEAD"])
                .map(|out| gitlink_paths(&out))
                .unwrap_or_default(),
        ),
    };
    scrub_planted_gitlinks(&snapshot);
    let mut map = WORKTREE_GIT_SNAPSHOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let first = map.is_empty();
    map.insert(gitdir, snapshot);
    drop(map);
    static STARTED: std::sync::Once = std::sync::Once::new();
    if first && !cfg!(test) {
        STARTED.call_once(|| {
            let _ = std::thread::Builder::new()
                .name("codex-worktree-guard".into())
                .spawn(|| loop {
                    std::thread::sleep(WORKTREE_GUARD_EVERY);
                    codex_worktree_guard_tick();
                });
        });
    }
}

/// A planted submodule entry in the writable `index` (a `160000` entry whose
/// folder holds a repository the agent made, with `core.fsmonitor` in its
/// config) makes the person's own `git status` run that program, since git
/// enters the submodule to see whether it is dirty. Probed 2026-11-12:
/// `diff.ignoreSubmodules=dirty` in the common config stops git from
/// entering it, for `status`, `diff`, `log -p`, `add`, `commit`, `checkout`,
/// `switch`, `stash`, `reset` and the rest of a 17-command battery, and it is
/// read-only to a Codex auto turn (only `objects`, `refs` and `logs` of the
/// common folder are granted). Set before every auto spawn in a worktree,
/// through `git config --file` (no repository discovery, so nothing of the
/// worktree is read), unless it is already `all` or `dirty`. Costs the
/// person `git status` no longer listing a dirty submodule's inner changes in
/// that repository (new commits still show).
fn harden_common_submodule_ignore(common: &Path) {
    let config = common.join("config");
    let run = |args: &[&str]| {
        std::process::Command::new(&*crate::build_workspaces::GIT)
            .arg("config")
            .arg("--file")
            .arg(&config)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .ok()
    };
    let current = run(&["--get", "diff.ignoreSubmodules"])
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .to_ascii_lowercase()
        });
    if matches!(current.as_deref(), Some("all" | "dirty")) {
        return;
    }
    run(&["diff.ignoreSubmodules", "dirty"]);
}

/// One pass of the guard: every snapshot's files put back if they changed;
/// a worktree whose git folder is gone is forgotten. Returns how many
/// files were put back.
pub(crate) fn codex_worktree_guard_tick() -> usize {
    guard_pass(None)
}

/// [`codex_worktree_guard_tick`] over every snapshot, or only the one of
/// git folder `only` (tests, which run side by side).
fn guard_pass(only: Option<&Path>) -> usize {
    let snapshots: Vec<WorktreeGitSnapshot> = WORKTREE_GIT_SNAPSHOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|s| only.is_none_or(|only| s.gitdir == only))
        .cloned()
        .collect();
    let mut restored = 0;
    for snapshot in snapshots {
        // File work outside the lock (a slow volume must not stall every
        // Hollow git call); a `forget` is still honoured right before each
        // write through the snapshot's flag ([`put_back`]).
        if !snapshot.gitdir.is_dir() {
            let mut map = WORKTREE_GIT_SNAPSHOTS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if map
                .get(&snapshot.gitdir)
                .is_some_and(|s| std::sync::Arc::ptr_eq(&s.forgotten, &snapshot.forgotten))
            {
                map.remove(&snapshot.gitdir);
            }
            continue;
        }
        restored += put_back(&snapshot);
        restored += scrub_planted_gitlinks(&snapshot);
    }
    restored
}

/// Hollow's git in a guarded worktree: pinned to the snapshot's folders,
/// `GIT_*` of this process stripped, and `SUBMODULE_SAFE` so nothing in the
/// (agent-writable) index starts a child git. Stdout on success.
fn snapshot_git<S: AsRef<std::ffi::OsStr>>(
    snapshot: &WorktreeGitSnapshot,
    args: &[S],
) -> Option<Vec<u8>> {
    snapshot_git_index(snapshot, args, None)
}

/// [`snapshot_git`] against another index file (`GIT_INDEX_FILE`), which git
/// locks as `<file>.lock`, never the real `index.lock`.
fn snapshot_git_index<S: AsRef<std::ffi::OsStr>>(
    snapshot: &WorktreeGitSnapshot,
    args: &[S],
    index_file: Option<&Path>,
) -> Option<Vec<u8>> {
    let mut cmd = std::process::Command::new(&*crate::build_workspaces::GIT);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    let cmd = cmd
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_DIR", &snapshot.gitdir)
        .env("GIT_COMMON_DIR", &snapshot.common)
        .env("GIT_WORK_TREE", &snapshot.cwd);
    let cmd = match index_file {
        Some(file) => cmd.env("GIT_INDEX_FILE", file),
        None => cmd,
    };
    let child = cmd
        .current_dir(&snapshot.cwd)
        .args(crate::build_workspaces::SUBMODULE_SAFE)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // Bounded: git opening a FIFO the turn left as the index or a shared
    // index would block the one guard thread for good (review round 3). The
    // child is killed through its own handle, never by pid.
    let mut child = child;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut bytes);
        let _ = tx.send(bytes);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // Never joined: a grandchild holding the pipe can't stall the guard.
    let ok = status.filter(|s| s.success())?;
    let _ = ok;
    rx.recv_timeout(Duration::from_secs(2)).ok()
}

/// The paths of the `160000` entries in `git ls-tree -r -z` or
/// `git ls-files -s -z` output (`<mode> …\t<path>`, NUL-separated).
fn gitlink_paths(out: &[u8]) -> Vec<Vec<u8>> {
    out.split(|b| *b == 0)
        .filter_map(|entry| {
            let tab = entry.iter().position(|b| *b == b'\t')?;
            entry[..tab]
                .starts_with(b"160000 ")
                .then(|| entry[tab + 1..].to_vec())
        })
        .collect()
}

/// A submodule entry planted in the writable `index` (a `160000` entry for a
/// folder holding a repository the agent made, with `core.fsmonitor` in its
/// config) makes the person's own `git add -A` (and `commit -a`) start that
/// program in the submodule: `diff.ignoreSubmodules` doesn't reach `add`
/// (probed 2026-11-12). The index can't be read-only (git writes it on
/// `add` and `commit`), so whenever it changes the guard lists its submodule
/// entries and removes any that `HEAD` didn't have when the worktree was
/// armed (`update-index --force-remove`, Hollow's git, on the real index
/// only; a locked index is retried on the next pass). A legitimate
/// `git submodule add` during an auto turn is undone this way too: a turn
/// that needs one runs outside auto. Returns how many entries it removed.
fn scrub_planted_gitlinks(snapshot: &WorktreeGitSnapshot) -> usize {
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::MetadataExt;
    // Change time and inode of what git reads, a split index's shared files
    // included (a process can't set `ctime`; the modified time it can, review
    // 2026-11-12; [`IndexStamp`]). A symlinked `index` is
    // never trusted: it is removed (git writes a fresh one on its next call).
    let index_path = snapshot.gitdir.join("index");
    if std::fs::symlink_metadata(&index_path).is_ok_and(|m| m.file_type().is_symlink()) {
        let _ = std::fs::remove_file(&index_path);
        return 1;
    }
    let Ok(meta) = std::fs::metadata(&index_path) else {
        return 0;
    };
    // Only a regular file reaches git (a FIFO would block it, a folder
    // breaks it): anything else is removed like a symlink.
    if !meta.is_file() {
        if meta.is_dir() {
            let _ = std::fs::remove_dir_all(&index_path);
        } else {
            let _ = std::fs::remove_file(&index_path);
        }
        return 1;
    }
    let stamp = IndexStamp {
        index: (meta.size(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
        shared: shared_index_stamps(&snapshot.gitdir),
    };
    let seen = || {
        snapshot
            .index_seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    };
    if seen().as_ref() == Some(&stamp) {
        return 0;
    }
    let failed = || {
        snapshot
            .index_failed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    };
    if failed()
        .as_ref()
        .is_some_and(|(at, retry)| *at == stamp && std::time::Instant::now() < *retry)
    {
        return 0;
    }
    let fail = || {
        *failed() = Some((stamp.clone(), std::time::Instant::now() + SCRUB_RETRY));
        0
    };
    let Some(listing) = snapshot_git(snapshot, &["ls-files", "-s", "-z"]) else {
        return fail();
    };
    let planted: Vec<Vec<u8>> = gitlink_paths(&listing)
        .into_iter()
        .filter(|path| !snapshot.gitlinks.contains(path))
        .collect();
    if planted.is_empty() {
        *seen() = Some(stamp.clone());
        return 0;
    }
    // On a copy of the index, renamed over it: a turn that holds
    // `index.lock` (the git folder is writable) can't keep the entries in
    // (review round 2, 2026-11-12). A later write by the turn changes the
    // ctime and is scanned on the next pass.
    use rand::Rng;
    if snapshot.forgotten.load(std::sync::atomic::Ordering::SeqCst) {
        return 0;
    }
    let index = snapshot.gitdir.join("index");
    // The whole file (`read_no_follow` stops at 64 KB), checked against the
    // size the stamp saw.
    let Some(bytes) = read_regular_whole(&index, meta.size()) else {
        return fail();
    };
    let tmp = snapshot.gitdir.join(format!(
        ".hollow-guard-idx-{:016x}",
        rand::thread_rng().gen::<u64>()
    ));
    let mut removed = 0;
    let done = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, &bytes))
        .is_ok();
    if done {
        for path in &planted {
            let args = [
                std::ffi::OsString::from("update-index"),
                "--force-remove".into(),
                "--".into(),
                std::ffi::OsString::from_vec(path.clone()),
            ];
            // The first failure ends the pass: each try can cost its whole
            // timeout; what was removed is renamed in, and the next pass
            // (a new ctime) takes the rest.
            if snapshot_git_index(snapshot, &args, Some(&tmp)).is_none() {
                break;
            }
            removed += 1;
        }
        let renamed = removed > 0
            && !snapshot.forgotten.load(std::sync::atomic::Ordering::SeqCst)
            && std::fs::rename(&tmp, &index).is_ok();
        if !renamed {
            removed = 0;
        }
    }
    let _ = std::fs::remove_file(&tmp);
    if removed == 0 {
        return fail();
    }
    // The removal changed the index, so the next pass scans it again.
    removed
}

/// How Hollow's own git (`build_workspaces::git_command`) runs in `dir`
/// when no Codex auto snapshot covers it (review 2026-11-12, item 1).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WorktreeGitPins {
    /// Not inside a linked worktree (a plain checkout, no repository):
    /// git as before.
    NotLinked,
    /// A linked worktree: git pinned to these (git folder, common folder,
    /// worktree top), from its `.git` file alone, so git never reads the
    /// worktree git folder's `commondir` or `gitdir`, which an earlier auto
    /// turn may have rewritten while no guard ran.
    Pinned(PathBuf, PathBuf, PathBuf),
    /// A linked worktree whose layout doesn't hold: Hollow runs no git
    /// there.
    Refused,
}

/// [`WorktreeGitPins`] for `dir`: the nearest `.git` at or above it; a
/// `.git` file naming a `worktrees/<name>` folder is a linked worktree,
/// checked with [`dot_structure`] (a submodule's is git as before).
pub(crate) fn worktree_git_pins(dir: &Path) -> WorktreeGitPins {
    let Ok(dir) = dir.canonicalize() else {
        return WorktreeGitPins::NotLinked;
    };
    let Some(top) = dir
        .ancestors()
        .find(|d| std::fs::symlink_metadata(d.join(".git")).is_ok())
    else {
        return WorktreeGitPins::NotLinked;
    };
    if std::fs::symlink_metadata(top.join(".git")).is_ok_and(|m| m.is_dir()) {
        return WorktreeGitPins::NotLinked;
    }
    // A `.git` file that names no `worktrees/<name>` folder (a submodule)
    // is git as before.
    let names_worktree = read_no_follow(&top.join(".git"))
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|t| {
            let named = t
                .lines()
                .next()?
                .strip_prefix("gitdir:")?
                .trim()
                .to_string();
            let parent = top.join(named).parent()?.file_name()?.to_os_string();
            Some(parent == "worktrees")
        })
        .unwrap_or(true);
    if !names_worktree {
        return WorktreeGitPins::NotLinked;
    }
    match dot_structure(top) {
        Some(layout) => WorktreeGitPins::Pinned(layout.gitdir, layout.common, layout.cwd),
        None => WorktreeGitPins::Refused,
    }
}

/// The worktree (its top folder) a guarded `dir` is in, for
/// `GIT_WORK_TREE` beside the pinned git folders.
pub(crate) fn codex_guarded_worktree(dir: &Path) -> Option<PathBuf> {
    let dir = dir.canonicalize().ok()?;
    WORKTREE_GIT_SNAPSHOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .find(|s| dir.starts_with(&s.cwd))
        .map(|s| s.cwd.clone())
}

/// Put `snapshot`'s two files back byte for byte where they differ (or
/// were replaced by a symlink or a folder). Returns how many it rewrote.
fn put_back(snapshot: &WorktreeGitSnapshot) -> usize {
    let mut restored = 0;
    for (name, bytes) in [
        ("commondir", &snapshot.commondir),
        ("gitdir", &snapshot.back),
    ] {
        let path = snapshot.gitdir.join(name);
        if read_no_follow(&path).as_ref() == Some(bytes) {
            continue;
        }
        if snapshot.forgotten.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        if replace_no_follow(&path, bytes) {
            restored += 1;
        }
    }
    restored
}

/// A regular file's whole contents (no cap), never through a symlink, and
/// only when its length is `expected`.
fn read_regular_whole(path: &Path, expected: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() != expected {
        return None;
    }
    let mut bytes = Vec::with_capacity(expected as usize);
    file.read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 == expected).then_some(bytes)
}

/// A regular file's bytes, never through a symlink (`O_NOFOLLOW`); `None`
/// for a symlink, a folder or anything unreadable.
fn read_no_follow(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024).read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Make `path` a regular file holding `bytes` without ever following a
/// link an agent may plant there (final review 2026-11-12): a fresh temp
/// file is created beside it (`create_new`, which never follows a link)
/// and renamed over it, and a rename replaces the entry itself, a symlink
/// included, never what it points to. A folder in its place (rename can't
/// replace one) is removed first; `remove_dir_all` doesn't follow links.
fn replace_no_follow(path: &Path, bytes: &[u8]) -> bool {
    use rand::Rng;
    use std::io::Write;
    let Some(dir) = path.parent() else {
        return false;
    };
    let tmp = dir.join(format!(
        ".hollow-guard-{:016x}",
        rand::thread_rng().gen::<u64>()
    ));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut file| file.write_all(bytes));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, path).is_ok() {
        return true;
    }
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        let _ = std::fs::remove_dir_all(path);
        if std::fs::rename(&tmp, path).is_ok() {
            return true;
        }
    }
    let _ = std::fs::remove_file(&tmp);
    false
}

/// Drop the snapshots of the worktrees at or under `path` (a worktree, or
/// a repository whose worktree folders are gone), before Hollow's own
/// `git worktree remove` or `prune` there: the guard must not recreate a
/// git folder's files while git deletes them.
pub(crate) fn forget_codex_worktree_guard(path: &Path) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    WORKTREE_GIT_SNAPSHOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, s| {
            let ours = s.cwd.starts_with(&path)
                || (s.common.parent() == Some(path.as_path()) && !s.cwd.is_dir());
            if ours {
                s.forgotten.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            !ours
        });
}

/// Before Hollow runs git in `dir` (`build_workspaces::git_command`,
/// `repo_diff`): when `dir` is in a worktree a Codex auto run was granted,
/// its `commondir` and `gitdir` are put back as they were, and the worktree
/// git folder and common folder are returned for the caller to pin
/// (`GIT_DIR`, `GIT_COMMON_DIR`), so git never reads the writable
/// `commondir` at all. `None` (nothing to pin) for every other folder.
pub(crate) fn restore_codex_worktree_git(dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let dir = dir.canonicalize().ok()?;
    let snapshot = WORKTREE_GIT_SNAPSHOTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .find(|s| dir.starts_with(&s.cwd))
        .cloned()?;
    put_back(&snapshot);
    Some((snapshot.gitdir, snapshot.common))
}

/// `cwd`'s linked-worktree git folder and its common git folder, both
/// canonical, or `None` unless all of this holds:
/// - `<cwd>/.git` is a regular file (not a symlink) reading
///   `gitdir: <path>`;
/// - that path is a folder named `<common>/worktrees/<name>`, outside
///   `cwd`, whose `commondir` reads exactly `../..` or `<common>` and whose
///   `gitdir` reads exactly `<cwd>/.git` (canonical paths, one trailing
///   newline allowed; both read without following a symlink);
/// - `<common>` is a real git folder (`HEAD`, `objects`, `refs`), outside
///   `cwd`, without `extensions.worktreeConfig` (which would make a file in
///   the writable worktree folder a git config);
/// - the folder holding `<common>` is not `/`, the home folder or a folder
///   above it, and holds none of `extra_dirs` (the vault folder Hollow
///   passes: a repository around the vault is not a build's).
fn checked_linked_worktree(cwd: &Path, extra_dirs: &[PathBuf]) -> Option<(PathBuf, PathBuf)> {
    let layout = worktree_layout(cwd, extra_dirs)?;
    // The text itself, not only what it resolves to: a path through a
    // symlink the agent can repoint resolves right today and wrong later.
    let (_, back) = canonical_git_texts(&layout.dot);
    let commondir = read_no_follow(&layout.gitdir.join("commondir"))?;
    let commondir = String::from_utf8(commondir).ok()?;
    let commondir = commondir.strip_suffix('\n').unwrap_or(&commondir);
    let common_text = layout.common.to_str()?;
    if commondir != "../.." && commondir != common_text {
        return None;
    }
    let named_back = read_no_follow(&layout.gitdir.join("gitdir"))?;
    if named_back != back && named_back != back[..back.len() - 1] {
        return None;
    }
    Some((layout.gitdir, layout.common))
}

/// A linked worktree's layout, checked by resolution and without reading
/// `<gitdir>/commondir` (which [`checked_linked_worktree`] checks as text).
struct WorktreeLayout {
    cwd: PathBuf,
    dot: PathBuf,
    gitdir: PathBuf,
    common: PathBuf,
}

/// `cwd`'s layout when everything but the `commondir` text holds: see
/// [`checked_linked_worktree`] (the back-pointer is checked by what it
/// resolves to here). The common folder is the layout's own
/// (`<common>/worktrees/<name>`), never what `commondir` names.
fn worktree_layout(cwd: &Path, extra_dirs: &[PathBuf]) -> Option<WorktreeLayout> {
    let layout = dot_layout(cwd, extra_dirs)?;
    let back = String::from_utf8(read_no_follow(&layout.gitdir.join("gitdir"))?).ok()?;
    let back = back.trim();
    if back.is_empty() || layout.gitdir.join(back).canonicalize().ok()? != layout.dot {
        return None;
    }
    Some(layout)
}

/// [`worktree_layout`] from `<cwd>/.git` alone (review 2026-11-12, item 1):
/// every check but the back-pointer, and no file inside the worktree git
/// folder is read, so a broken `commondir` and `gitdir` both can't hide
/// where the repository is. The common folder is the parent of the
/// `worktrees` folder `.git` names.
fn dot_layout(cwd: &Path, extra_dirs: &[PathBuf]) -> Option<WorktreeLayout> {
    let layout = dot_structure(cwd)?;
    let holder = layout.common.parent()?;
    let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    if holder.parent().is_none() || home.is_some_and(|home| home.starts_with(holder)) {
        return None;
    }
    let holds_extra = extra_dirs.iter().any(|dir| {
        let dir = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        dir.starts_with(holder)
    });
    if holds_extra {
        return None;
    }
    Some(layout)
}

/// [`dot_layout`]'s structure alone, what pinning git needs (no grant, so
/// no policy on where the repository is): `.git` a regular file naming
/// `<common>/worktrees/<name>` outside `cwd`, `<common>` a real git folder
/// outside `cwd` without `extensions.worktreeConfig` (which would make the
/// writable `<gitdir>/config.worktree` a config git reads).
fn dot_structure(cwd: &Path) -> Option<WorktreeLayout> {
    let cwd = cwd.canonicalize().ok()?;
    let dot = cwd.join(".git");
    if !std::fs::symlink_metadata(&dot).ok()?.file_type().is_file() {
        return None;
    }
    let text = String::from_utf8(read_no_follow(&dot)?).ok()?;
    let named = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    if named.is_empty() {
        return None;
    }
    let gitdir = cwd.join(named).canonicalize().ok()?;
    if !gitdir.is_dir() || gitdir.starts_with(&cwd) {
        return None;
    }
    let worktrees = gitdir.parent()?;
    if worktrees.file_name()? != "worktrees" {
        return None;
    }
    let common = worktrees.parent()?.to_path_buf();
    if common.starts_with(&cwd)
        || !common.join("HEAD").is_file()
        || !common.join("objects").is_dir()
        || !common.join("refs").is_dir()
    {
        return None;
    }
    let config = std::fs::read_to_string(common.join("config")).unwrap_or_default();
    if worktree_config_on(&config) {
        return None;
    }
    Some(WorktreeLayout {
        cwd,
        dot,
        gitdir,
        common,
    })
}

/// Whether a git config text sets `extensions.worktreeConfig` true: the
/// key in an `[extensions]` section (names case-insensitive), bare or with
/// a true value (`true`, `yes`, `on`, `1`). Unsure counts as on (refuse).
fn worktree_config_on(config: &str) -> bool {
    let mut in_extensions = false;
    for line in config.lines() {
        let line = line.split(['#', ';']).next().unwrap_or("").trim();
        if let Some(header) = line.strip_prefix('[') {
            let name = header.split([']', ' ', '"']).next().unwrap_or("");
            in_extensions = name.eq_ignore_ascii_case("extensions");
            // `[extensions] worktreeConfig = true` on one line.
            let rest = header.split_once(']').map(|(_, r)| r.trim()).unwrap_or("");
            if in_extensions && !rest.is_empty() && worktree_config_line(rest) {
                return true;
            }
            continue;
        }
        if in_extensions && worktree_config_line(line) {
            return true;
        }
    }
    false
}

fn worktree_config_line(line: &str) -> bool {
    let (key, value) = match line.split_once('=') {
        Some((k, v)) => (k.trim(), Some(v.trim().trim_matches('"'))),
        None => (line.trim(), None),
    };
    if !key.eq_ignore_ascii_case("worktreeconfig") {
        return false;
    }
    match value {
        None => true,
        Some(v) => !matches!(
            v.to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0" | ""
        ),
    }
}

/// What `git worktree add` writes, from the layout: `commondir` is
/// `../..` and `gitdir` the worktree's `.git` file, each with a newline.
fn canonical_git_texts(dot: &Path) -> (Vec<u8>, Vec<u8>) {
    let mut back = dot.as_os_str().as_encoded_bytes().to_vec();
    back.push(b'\n');
    (b"../..\n".to_vec(), back)
}

/// At an auto spawn: a worktree whose layout holds but whose `commondir`
/// or `gitdir` text isn't git's own gets git's own back (written with
/// [`replace_no_follow`]). Nothing when the layout fails: no grant then.
fn repair_worktree_git_texts(cwd: &Path, extra_dirs: &[PathBuf]) {
    if checked_linked_worktree(cwd, extra_dirs).is_some() {
        return;
    }
    // From `.git` alone, so a broken back-pointer is repaired too, unless
    // it names another live worktree whose `.git` claims this git folder
    // (then neither is ours to rewrite, and nothing is granted).
    let Some(layout) = dot_layout(cwd, extra_dirs) else {
        return;
    };
    if worktree_layout(cwd, extra_dirs).is_none() {
        let named = read_no_follow(&layout.gitdir.join("gitdir"))
            .and_then(|b| String::from_utf8(b).ok())
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        // A relative back-pointer is relative to the git folder.
        let other = layout.gitdir.join(&named);
        let other = other.as_path();
        if !named.is_empty() && other != layout.dot {
            let claims = read_no_follow(other)
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|t| {
                    let dir = t
                        .lines()
                        .next()?
                        .strip_prefix("gitdir:")?
                        .trim()
                        .to_string();
                    other.parent()?.join(dir).canonicalize().ok()
                });
            if claims.as_deref() == Some(layout.gitdir.as_path()) {
                return;
            }
        }
    }
    let (commondir, back) = canonical_git_texts(&layout.dot);
    replace_no_follow(&layout.gitdir.join("commondir"), &commondir);
    replace_no_follow(&layout.gitdir.join("gitdir"), &back);
}

/// What a mode lets a Codex app-server thread do (steering PR 3,
/// [`crate::codex_app_server`]): the thread settings `codex exec` gets from
/// its flags in [`build_codex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodexPolicy {
    /// `approvalPolicy`: exec's default `never`, or auto's `on-request`.
    pub approval_policy: &'static str,
    /// `approvalsReviewer`: auto's `auto_review`, else the user's setting.
    pub approvals_reviewer: Option<&'static str>,
    /// `sandbox`: [`codex_sandbox`], or `danger-full-access` for bypass.
    pub sandbox: &'static str,
}

/// [`CodexPolicy`] for a Hollow mode, as [`build_codex`] maps it for exec:
/// plan is read-only and the other modes write the workspace, all with
/// exec's `never` (nothing ever prompts); auto routes approval requests to
/// Codex's automatic reviewer (`--approve-for-me`); bypass is
/// `--dangerously-bypass-approvals-and-sandbox`, no approvals and no
/// sandbox.
pub(crate) fn codex_app_server_policy(perm: &str) -> CodexPolicy {
    match perm {
        "bypassPermissions" => CodexPolicy {
            approval_policy: "never",
            approvals_reviewer: None,
            sandbox: "danger-full-access",
        },
        "auto" => CodexPolicy {
            approval_policy: "on-request",
            approvals_reviewer: Some("auto_review"),
            sandbox: codex_sandbox(perm),
        },
        _ => CodexPolicy {
            approval_policy: "never",
            approvals_reviewer: None,
            sandbox: codex_sandbox(perm),
        },
    }
}

/// Steering PR 3: `codex app-server` (JSON-RPC on stdio,
/// [`crate::codex_app_server`]) in place of [`build_codex`]'s `exec … <prompt>`.
/// The top-level flags are [`build_codex`]'s, word for word (a test holds
/// them equal): `-c` and `--disable` are global options, so they reach the
/// app-server from before the subcommand exactly as they reach `exec`
/// (checked with `config/read`, codex-cli 0.163.0), and
/// [`build_command`]'s `mcp_servers` override stays where it is. Model,
/// sandbox, approval policy, `--ephemeral` and the prompt go in the
/// thread and turn requests instead ([`codex_app_server_start`]), and a
/// resumed thread takes its sandbox in `thread/resume`, so the resume's
/// `sandbox_mode` override isn't needed. There is no git-repo check to
/// skip. Only a steerable chat turn runs it ([`live_codex_input`]); every
/// other Codex run is [`build_codex`].
pub(crate) fn build_codex_app_server(cmd: &mut Command, spec: &RunSpec) {
    cmd.args([
        "--disable",
        "apps",
        "--disable",
        "plugins",
        "--disable",
        "remote_plugin",
    ]);
    codex_self_hosted_overrides(cmd, spec);
    if let Some(effort) = run_effort(AgentKind::Codex, spec).effort {
        cmd.arg("-c").arg(format!(
            "model_reasoning_effort={}",
            serde_json::to_string(&effort).expect("effort level")
        ));
    }
    if let Some(limit) = compact_window(AgentKind::Codex, spec) {
        cmd.arg("-c")
            .arg(format!("model_auto_compact_token_limit={limit}"));
    }
    if spec.perm == "auto" {
        cmd.arg("-c").arg("approval_policy=\"on-request\"");
        cmd.arg("-c").arg("approvals_reviewer=\"auto_review\"");
    }
    // Process config, which a `workspace-write` thread inherits (checked
    // with `thread/start`'s answer, codex-cli 0.167.2).
    for value in codex_auto_sandbox_overrides(spec) {
        cmd.arg("-c").arg(value);
    }
    cmd.arg("app-server");
}

/// The thread and turn a Codex app-server run starts with: what
/// [`build_codex`] puts on exec's argv (the same [`contextual_prompt`],
/// the resumed id only when interactive, a non-empty model, `--ephemeral`
/// for a one-shot) and [`codex_app_server_policy`]'s settings. Effort rides
/// argv as for exec, so `config` stays empty. `profile` is the decision the
/// process already carries ([`codex_command_uses_profile`] on the built
/// command), never a second look at the worktree.
pub(crate) fn codex_app_server_start(
    spec: &RunSpec,
    profile: bool,
) -> crate::codex_app_server::StartParams {
    let policy = codex_app_server_policy(&spec.perm);
    crate::codex_app_server::StartParams {
        prompt: contextual_prompt(AgentKind::Codex, spec),
        resume: resumed_id(spec).map(str::to_string),
        // A remote run's folder on the runner (the Mac mini runner).
        cwd: match spec.extras.remote.as_ref() {
            Some(target) => target.cwd.clone(),
            None => spec.cwd.to_string_lossy().into_owned(),
        },
        model: spec.model.clone().filter(|m| !m.is_empty()),
        approval_policy: policy.approval_policy.to_string(),
        approvals_reviewer: policy.approvals_reviewer.map(str::to_string),
        // Empty under the auto profile: a thread `sandbox` overrides it.
        sandbox: codex_app_thread_sandbox(&spec.perm, profile).to_string(),
        ephemeral: !spec.interactive,
        config: serde_json::Map::new(),
    }
}

pub(crate) fn contextual_prompt(kind: AgentKind, spec: &RunSpec) -> String {
    let policy = run_policy(kind, spec);
    let held = spec.extras.configured_held && resumed_id(spec).is_some();
    match spec
        .append_system_prompt
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|context| if held { CONFIGURED_HELD } else { context })
    {
        Some(context) => format!("{policy}\n\n{context}\n\n{}", spec.prompt),
        None => format!("{policy}\n\n{}", spec.prompt),
    }
}

fn build_antigravity(cmd: &mut Command, spec: &RunSpec) {
    // `agy -p` supports model selection, stream JSON, and explicit session IDs.
    // It has no tool allowlist or owned MCP isolation, so the only
    // reliably prompt-free options are `--sandbox` (terminal-restricted) and
    // `--dangerously-skip-permissions` (full auto). See the mode mapping at
    // the end.
    // Antigravity has no per-run connector isolation flag; enforce Hollow
    // scope in the prompt while preserving this independently installed agent.
    // R5 residual (11-17 probe, agy 1.2.x): `-p=` with an EMPTY attached value
    // plus `--input-format stream-json` reads the prompt from stdin as one
    // NDJSON line (`{"event":"user","message":{"content":...}}`), so `ps`
    // never shows it. A bare `-p` takes the next flag as its prompt and a
    // non-empty `-p=` value is refused beside stream input; the output is the
    // same stream Hollow already reads.
    cmd.arg("-p=")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json");
    // Effort is the model variant on Antigravity (`…-low` / `…-high`).
    let variant = run_effort(AgentKind::Antigravity, spec).model;
    if let Some(model) = variant
        .as_deref()
        .or(spec.model.as_deref())
        .filter(|s| !s.is_empty())
    {
        cmd.arg("--model").arg(model);
    }
    if let Some(sid) = resumed_id(spec) {
        // Never use --continue: that might resume an unrelated external
        // session. Only an explicitly captured conversation id is valid.
        cmd.arg("--conversation").arg(sid);
    }
    // Paperclip PR 15a: the scratch folder, before the permission flags. A
    // live probe (agy 1.2.10) showed headless `--sandbox --mode accept-edits`
    // auto-denies every file write, this folder included, so it is writable
    // only under bypass; the environment is set either way.
    if let Some(dir) = spec.extras.scratch.as_ref().and_then(|s| s.dir()) {
        cmd.arg("--add-dir").arg(dir);
    }
    for dir in &spec.extras.extra_dirs {
        cmd.arg("--add-dir").arg(dir);
    }
    // Headless agy can't ask, so whatever its mode would prompt for is
    // denied (agy 1.2.14 probe, 2026-10-02: `--mode accept-edits` reports
    // `denied_actions: [RunCommand]` and writes nothing). `bypassPermissions`
    // approves every tool with no sandbox. There is no `auto`:
    // `--sandbox --dangerously-skip-permissions` confined only the shell, and
    // its file tools wrote outside the folder (agy 1.2.15 probe matrix,
    // [`ANTIGRAVITY_AUTO_UNAVAILABLE`]), so `validate_run_spec` refuses an
    // auto run before this builder is reached. Never emit that pair.
    match spec.perm.as_str() {
        "bypassPermissions" => {
            cmd.arg("--dangerously-skip-permissions");
        }
        _ => {
            cmd.arg("--sandbox");
            cmd.arg("--mode").arg(match spec.perm.trim() {
                "plan" => "plan",
                // agy 1.5.13's own reviewer: never the skip-permissions flag.
                "auto" => "auto",
                _ => "accept-edits",
            });
        }
    }
}

/// agy's own mode id for a Hollow mode over ACP (`session/new`'s modes):
/// `default` asks before edits and commands, `accept-edits` approves edits in
/// the folder and asks for commands, `auto` is its reviewer, `plan` reads.
/// Bypass has no mode: the server starts with `--dangerously-skip-permissions`.
pub(crate) fn antigravity_acp_mode(perm: &str) -> Option<&'static str> {
    match perm.trim() {
        "bypassPermissions" => None,
        "plan" => Some("plan"),
        "default" => Some("default"),
        "auto" => Some("auto"),
        _ => Some("accept-edits"),
    }
}

/// Set (to anything but empty, `0` or `false`) to keep every Antigravity
/// turn on headless `agy -p`, as before 2026-11-20: the way out if `agy acp`
/// misbehaves. The environment is read once per spawn.
const ANTIGRAVITY_HEADLESS_ENV: &str = "HOLLOW_AGY_HEADLESS";

/// [`ANTIGRAVITY_HEADLESS_ENV`] is set: Antigravity runs `agy -p` only.
fn antigravity_headless_forced() -> bool {
    std::env::var(ANTIGRAVITY_HEADLESS_ENV).is_ok_and(|v| !matches!(v.trim(), "" | "0" | "false"))
}

/// Whether an Antigravity turn runs over `agy acp` (2026-11-20): a chat turn
/// on this Mac with no self-hosted target. Headless `-p` can't ask, so it
/// denied every command in accept-edits; over ACP the same request becomes
/// one of Hollow's approval cards, and Auto, usage, the context window and
/// per-chat MCP servers all work. One-shot runs (automations, teammates,
/// ingest) stay on `-p`.
pub fn antigravity_acp(spec: &RunSpec) -> bool {
    spec.extras.antigravity_acp
        && !antigravity_headless_forced()
        && spec.interactive
        && spec.extras.remote.is_none()
        && spec.extras.self_hosted.is_none()
}

/// `agy acp` for a turn [`antigravity_acp`] accepts, in place of
/// [`build_antigravity`]'s `-p` argv. The prompt, model, effort, mode and
/// MCP servers go over JSON-RPC ([`antigravity_acp_start`]), never argv.
/// Bypass starts the server with `--dangerously-skip-permissions` and no
/// sandbox, as headless does; every other mode gets `--sandbox` (terminal
/// restrictions) beside its mode.
pub(crate) fn build_antigravity_acp(cmd: &mut Command, spec: &RunSpec) {
    cmd.arg("acp");
    if let Some(dir) = spec.extras.scratch.as_ref().and_then(|s| s.dir()) {
        cmd.arg("--add-dir").arg(dir);
    }
    for dir in &spec.extras.extra_dirs {
        cmd.arg("--add-dir").arg(dir);
    }
    if antigravity_acp_mode(&spec.perm).is_none() {
        cmd.arg("--dangerously-skip-permissions");
    } else {
        cmd.arg("--sandbox");
    }
}

/// The session an Antigravity ACP turn opens ([`crate::acp_client`]): the
/// contextual prompt headless reads, the resumed conversation to load, the
/// base model and its level (Hollow's one Effort control, mapped to the level
/// the model has: [`crate::effort::resolve`]), agy's mode, and the
/// permission requests routed to the person's cards.
pub(crate) fn antigravity_acp_start(spec: &RunSpec) -> crate::acp_client::StartParams {
    let model = spec
        .model
        .as_deref()
        .map(crate::effort::agy_base)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    let effort = run_effort(AgentKind::Antigravity, spec)
        .model
        .as_deref()
        .and_then(|variant| crate::effort::split_agy(variant).1.map(str::to_string));
    crate::acp_client::StartParams {
        cwd: spec.cwd.to_string_lossy().into_owned(),
        prompt: contextual_prompt(AgentKind::Antigravity, spec),
        resume: resumed_id(spec).map(str::to_string),
        model,
        effort,
        session_meta: serde_json::json!({}),
        perm: spec.perm.clone(),
        agent: AgentKind::Antigravity,
        mode: antigravity_acp_mode(&spec.perm).map(str::to_string),
        ask: antigravity_acp_mode(&spec.perm).is_some(),
        mcp_servers: crate::acp_client::mcp_servers_from_config(spec.mcp_config.as_deref()),
    }
}

fn build_grok(cmd: &mut Command, spec: &RunSpec) {
    // `grok -p` (Grok Build CLI 1.0.41) prints Claude Code's stream-json with
    // `--output-format streaming-messages-json`, takes Claude's permission
    // modes verbatim, and resumes an exact session id with `--resume`. It has
    // no per-run MCP config (the user's own `~/.grok` servers still load) and
    // no verified tool allowlist in Claude's tool names, so like Antigravity
    // the Hollow scope travels in the prompt.
    // R5b (Q-R33): the prompt comes from the run's stdin file
    // (`stdin_prompt`), so `ps` never shows it. `--prompt-file` is Grok's
    // single-turn prompt from a file and it reads before it checks the
    // sign-in (probed with an empty HOME: a missing path fails with "Failed
    // to read", `/dev/stdin` gets as far as "Not signed in").
    if stdin_prompt(AgentKind::Grok, spec).is_some() {
        cmd.arg("--prompt-file").arg("/dev/stdin");
    } else {
        cmd.arg("-p").arg(contextual_prompt(AgentKind::Grok, spec));
    }
    cmd.arg("--output-format").arg("streaming-messages-json");
    if spec.interactive {
        cmd.arg("--include-partial-messages");
    }
    if let Some(sid) = resumed_id(spec) {
        // Never `--continue`: only an explicitly captured session id.
        cmd.arg("--resume").arg(sid);
    }
    if let Some(model) = spec.model.as_deref().filter(|s| !s.is_empty()) {
        cmd.arg("--model").arg(model);
    }
    if let Some(effort) = run_effort(AgentKind::Grok, spec).effort {
        cmd.arg("--reasoning-effort").arg(effort);
    }
    // Grok 1.0.46 takes every canonical mode, `default` included.
    let perm = match spec.perm.as_str() {
        "plan" | "default" | "acceptEdits" | "auto" | "bypassPermissions" => spec.perm.as_str(),
        _ => "acceptEdits",
    };
    cmd.arg("--permission-mode").arg(perm);
    // Headless `-p` ran every command in plan, default and acceptEdits alike
    // (probed 2026-10-02: a write outside the folder, `osascript`, `open`),
    // so for those modes the OS sandbox is the limit. `read-only` writes only
    // `~/.grok` and the temp folders; `workspace` adds the working folder.
    // Both refused `osascript` to System Events and `open -a` (-54). Grok
    // fixes a session's profile when it starts and refuses a different one on
    // resume, so a resumed turn passes it too: a mode switch across profiles
    // fails as a resume failure and the chat retries fresh
    // (`result_kind::GROK_RESUME_RE`), never under the old profile. Auto and
    // bypass pass `off` explicitly rather than nothing: with no flag a resumed
    // session keeps the profile it started with, so a chat switched from plan
    // to bypass would stay silently read-only. `off` is what a session started
    // without the flag records, so chats from before the sandbox still resume.
    cmd.arg("--sandbox").arg(grok_sandbox(perm));
    for rule in spec
        .disallowed_tools
        .iter()
        .filter(|t| holds_deny(AgentKind::Grok, t))
    {
        cmd.arg("--deny").arg(rule);
    }
}

/// Grok's sandbox profile for a canonical mode ([`build_grok`]). `default`
/// asks before edits, and a headless run can't ask, so it gets read-only like
/// plan (unlike Codex's `default`, which runs `workspace-write`, see
/// [`codex_sandbox`]). Auto and bypass run with the
/// sandbox `off`; an unknown mode runs as acceptEdits, as in [`build_grok`].
pub(crate) fn grok_sandbox(perm: &str) -> &'static str {
    match perm {
        "plan" | "default" => "read-only",
        "auto" | "bypassPermissions" => "off",
        _ => "workspace",
    }
}

/// OC1: why a Grok run stays on headless `grok -p` although Settings' ACP
/// switch is on, or `Ok` with the mode it runs over ACP in
/// ([`crate::acp_client::grok_mode`] holds the per-mode table). Only a
/// chat turn on this Mac with no self-hosted target and no `--deny` rule
/// (a stdio session takes none: a top-level `--deny` left no trace) goes.
pub(crate) fn grok_acp_mode(
    spec: &RunSpec,
) -> Result<crate::acp_client::GrokAcpMode, &'static str> {
    if !spec.extras.grok_acp {
        return Err("the switch is off");
    }
    if !crate::acp_client::selectable(AgentKind::Grok) {
        return Err("Grok's dialect isn't selectable");
    }
    if !spec.interactive {
        return Err("only a chat turn runs over ACP");
    }
    if spec.extras.remote.is_some() || spec.extras.self_hosted.is_some() {
        return Err("a runner or self-hosted turn runs headless");
    }
    if spec
        .disallowed_tools
        .iter()
        .any(|t| holds_deny(AgentKind::Grok, t))
    {
        return Err("a stdio session takes no --deny rule");
    }
    crate::acp_client::grok_mode(&spec.perm)
}

/// Whether a Grok run is a turn over ACP ([`grok_acp_mode`]).
pub fn grok_acp(spec: &RunSpec) -> bool {
    grok_acp_mode(spec).is_ok()
}

/// OC1: Grok's ACP server for a turn [`grok_acp`] accepts, in place of
/// [`build_grok`]'s `-p` argv: `grok --sandbox <profile> agent
/// [--always-approve] --no-leader [--reasoning-effort <e>] stdio`. The
/// sandbox flag is the headless one (probed to reach the stdio session);
/// `--no-leader` keeps the agent in this process group even when the
/// person's config turns Grok's shared leader on. The prompt, model and
/// session go over JSON-RPC ([`grok_acp_start`]), never argv.
pub(crate) fn build_grok_acp(cmd: &mut Command, spec: &RunSpec) {
    let Ok(mode) = grok_acp_mode(spec) else {
        return build_grok(cmd, spec);
    };
    cmd.arg("--sandbox").arg(mode.sandbox);
    // The dialect's `agent stdio`, with `grok agent`'s own options between.
    cmd.arg("agent");
    if mode.always_approve {
        cmd.arg("--always-approve");
    }
    cmd.arg("--no-leader");
    if let Some(effort) = run_effort(AgentKind::Grok, spec).effort {
        cmd.arg("--reasoning-effort").arg(effort);
    }
    cmd.arg("stdio");
}

/// OC1: the session a Grok ACP turn opens ([`crate::acp_client`]): the
/// contextual prompt headless reads, the resumed session to load, the model
/// and effort as config options, and always-approve in `_meta` when the
/// mode has it.
pub(crate) fn grok_acp_start(spec: &RunSpec) -> crate::acp_client::StartParams {
    let always = grok_acp_mode(spec).is_ok_and(|m| m.always_approve);
    crate::acp_client::StartParams {
        cwd: spec.cwd.to_string_lossy().into_owned(),
        prompt: contextual_prompt(AgentKind::Grok, spec),
        resume: resumed_id(spec).map(str::to_string),
        model: spec.model.clone().filter(|m| !m.trim().is_empty()),
        effort: run_effort(AgentKind::Grok, spec)
            .effort
            .map(|e| e.to_string()),
        session_meta: if always {
            serde_json::json!({"yoloMode": true})
        } else {
            serde_json::json!({})
        },
        perm: spec.perm.clone(),
        agent: AgentKind::Grok,
        mode: None,
        ask: false,
        mcp_servers: Vec::new(),
    }
}

/// What a one-shot run ([`run_collect`], `claude_runner`, `codex_transform`)
/// observed.
pub struct Collected {
    pub result: Result<String, String>,
    /// What a run its deadline or stop ended had written so far (OF04): its
    /// non-error result text, else its streamed deltas (Antigravity, Grok; a
    /// blank line between messages), else its prose text blocks joined by
    /// blank lines (Claude, Codex). Never error text. Empty for any
    /// other ending and when it had written nothing. `result` stays the
    /// `Err` that says how it ended, so a caller that wants the partial
    /// text (team-mcp's `ask_agent`) must ask for it here.
    pub partial_text: String,
    /// The time bound elapsed; the run's processes were ended.
    pub timed_out: bool,
    /// The caller's stop signal was requested; the run's processes were ended.
    pub stopped: bool,
    /// Whether ending the run's processes was verified (`None` when the spec
    /// was not GUI-owned, so only the direct child could be killed).
    pub outcome: Option<process_group::StopOutcome>,
    /// The usage row this invocation wrote (`None` only from
    /// [`Collected::failed`], which records nothing by itself).
    #[allow(dead_code)]
    pub usage: Option<crate::usage_ledger::RunUsage>,
    /// Paperclip PR 8b: a usage budget ended the run: its budget guard
    /// crossed an enforced limit mid-run (`Ended::LimitReached`), or Claude's
    /// `--max-budget-usd` backstop stopped it. `result` is then the notice's
    /// message as an `Err`, and the usage row's outcome is `budget_stopped`.
    pub budget_stop: Option<crate::budgets::BudgetNotice>,
    /// Paperclip PR 8b: warn or limit levels the run's budget guard crossed
    /// mid-run without stopping it (a warn-only run: a delegate under chat,
    /// a soft budget), each once. team-mcp puts them in front of the
    /// delegation's result; other callers leave them to `after_run`.
    pub budget_warnings: Vec<crate::budgets::BudgetNotice>,
    /// The run's activity clock (OB08a): its spawn, first outputs, first
    /// model event and tool calls before its first text, stamped by `drive`
    /// and the stream. `None` when nothing was spawned under one: a refusal,
    /// a spawn failure in [`collect_command`], the Codex tool-free transform.
    /// The heartbeat reads its phase timings from it.
    pub activity: Option<std::sync::Arc<crate::run_activity::RunActivity>>,
}

impl Collected {
    /// A run that failed before (or without) owning a process. Records no
    /// usage row; runners that own a row set `usage` themselves.
    pub fn failed(error: String) -> Self {
        Self {
            result: Err(error),
            partial_text: String::new(),
            timed_out: false,
            stopped: false,
            outcome: None,
            usage: None,
            budget_stop: None,
            budget_warnings: Vec::new(),
            activity: None,
        }
    }
}

/// Spawn an agent non-interactively, drain its stdout via [`parse_line`], and
/// return its final assistant text. Used by one-shot helpers (`claude_runner`,
/// heartbeat) and by the team MCP server's delegations. Pass the time bound
/// as `limit` and any cancellation as `stop` rather than wrapping the call in
/// a timeout or a select: a dropped future gets only best-effort cleanup,
/// while `limit` and `stop` end every process the run started before this
/// returns. A run `limit` or `stop` ended is an `Err` that says so, with
/// what it had written kept in [`Collected::partial_text`] (OF04).
///
/// Writes exactly one usage-ledger row for `meter`: a validation refusal is a
/// `spawn_failed` row ([`crate::usage_ledger::record_refusal`]); otherwise the
/// row is begun before the spawn and finished on every ending.
///
/// A meter carrying a budget ([`crate::usage_ledger::Meter::with_budget`],
/// Paperclip PR 8b) arms a budget guard in [`collect_command`]; set
/// `spec.usage_stream` for it on a Claude run, so the guard reads final
/// per-message usage.
pub async fn run_collect(
    kind: AgentKind,
    spec: &RunSpec,
    limit: Option<Duration>,
    stop: Option<&process_group::StopSignal>,
    meter: &crate::usage_ledger::Meter,
) -> Collected {
    if let Err(error) = validate_run_spec(kind, spec) {
        let usage = crate::usage_ledger::record_refusal(meter, kind, spec.model.clone(), &error);
        return Collected {
            usage: Some(usage),
            ..Collected::failed(error)
        };
    }
    refresh_accounts(kind, spec).await;
    let cmd = build_command(kind, spec);
    let mut ledger =
        crate::usage_ledger::UsageRun::begin(meter.clone(), Some(kind), spec.model.clone());
    ledger.set_billing(crate::usage::classify(kind, &cmd, true));
    collect_command(
        kind,
        cmd,
        ParseState::for_spec(kind, spec),
        limit,
        stop,
        ledger,
    )
    .await
}

/// The body of [`run_collect`] from the spawn onward, for a command that is
/// already built and a usage row that is already begun (PR 7 places its
/// permit between the two; tests drive it with real shell processes). Every
/// ending finishes `ledger` exactly once: a spawn error as `spawn_failed`
/// (the unavailable stand-in too, via `launched`), a timeout as `timeout`, a
/// stop as `cancelled`, a failure as `error`, a budget stop as
/// `budget_stopped`, else `success`. `state` lets a caller choose the parse
/// mode (PR 8b's usage-only partial messages, [`ParseState::for_spec`]).
///
/// **Budget guard (Paperclip PR 8b).** When `ledger`'s meter carries a budget
/// with token checks, every event that grows the run's running total is
/// checked against it; an enforced limit crossed mid-run requests the limit
/// signal, which ends the run as `Ended::LimitReached` (a GUI-owned run's
/// whole process tree; an inherited run, a team-mcp delegate, loses only its
/// direct process). Claude's own backstop exit (`error_max_budget_usd`) is a
/// budget stop too. Warnings a warn-only guard raises here are returned in
/// [`Collected::budget_warnings`] (team-mcp shows them to the primary);
/// the finished run's `budgets::after_run` notifies once per window. The
/// guard is armed for Claude only and never on or after the result envelope
/// ([`crate::budgets::BudgetGuard::on_event`]), so a run that finished its
/// work is never turned into a budget stop.
pub(crate) async fn collect_command(
    kind: AgentKind,
    mut cmd: Command,
    mut state: ParseState,
    limit: Option<Duration>,
    stop: Option<&process_group::StopSignal>,
    mut ledger: crate::usage_ledger::UsageRun,
) -> Collected {
    use crate::usage::LedgerOutcome;
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Collected {
                usage: Some(ledger.finish(LedgerOutcome::SpawnFailed)),
                ..Collected::failed(format!("spawn {}: {e}", kind.as_str()))
            }
        }
    };
    ledger.launched(&cmd);
    let guard = process_group::GroupGuard::adopt_run(&cmd, &child, ledger.run_id());

    let mut last_text = String::new();
    // Every non-empty prose text block, for a run cut off before its result
    // (OF04: the partial text a timed-out consultation returns). An API
    // error's text is not prose and never lands here.
    let mut text_blocks: Vec<String> = Vec::new();
    let mut delta_buf = String::new();
    // A message boundary seen after some deltas: the next delta starts a new
    // message, so a blank line goes in first (Grok, Antigravity; no trailing
    // separator when no text follows).
    let mut delta_gap = false;
    let mut final_result: Option<String> = None;
    // The last result envelope's text that was not an error, for the partial
    // text (an error result's text must never read as the answer).
    let mut ok_result: Option<String> = None;
    let mut reported_error = None;
    let mut result_subtype: Option<String> = None;
    // Claude only: Codex and Antigravity are checked at the claim only.
    let mut budget_guard = ledger
        .budget()
        .and_then(|budget| crate::budgets::BudgetGuard::for_agent(budget, kind));
    let mut budget_warnings = Vec::new();
    let budget_limit = process_group::StopSignal::new();
    let activity = std::sync::Arc::new(crate::run_activity::RunActivity::new());
    let supervision = process_group::Supervision {
        stop,
        limit: budget_guard.as_ref().map(|_| &budget_limit),
        deadline: limit,
        // Never echo: inside `hollow team-mcp` a closed stderr must not
        // abort the server.
        stderr: process_group::StderrPolicy::Silent,
        timings: process_group::Timings::default(),
        // One-shots, the heartbeat and delegations have no watchdog (it
        // covers chat and workflows only, Paperclip PR 15b; every agent
        // since merged plan slice 3). The clock only stamps (OB08a): the
        // heartbeat's phase timings read it.
        watch: Some(process_group::Watch {
            activity: std::sync::Arc::clone(&activity),
            stall_after: None,
        }),
    };
    let driven = process_group::drive(child, guard, supervision, |line| {
        for evt in parse_line(kind, line, &mut state) {
            activity.note_event(&evt);
            let grew = ledger.observe(&evt);
            if let Some(budget_guard) = budget_guard.as_mut() {
                let running = if grew { ledger.running_total() } else { None };
                budget_guard.bind_run(ledger.key());
                let checked = budget_guard.on_event(&evt, grew, running);
                budget_warnings.extend(checked.warnings);
                if checked.stop.is_some() {
                    budget_limit.request();
                }
            }
            match evt {
                AgentEvent::AssistantText(t) => {
                    if !t.trim().is_empty() {
                        text_blocks.push(t.clone());
                        last_text = t;
                    }
                }
                // An API-error wrapper reads exactly as the text it replaced
                // in a finished run (Paperclip PR 14a), but is no part of a
                // cut-off run's partial text (OF04).
                AgentEvent::ApiError { text: t, .. } => {
                    if !t.trim().is_empty() {
                        last_text = t;
                    }
                }
                AgentEvent::TurnBoundary => delta_gap = !delta_buf.is_empty(),
                AgentEvent::AssistantDelta(t) => {
                    if delta_gap && !t.is_empty() {
                        delta_buf.push_str("\n\n");
                        delta_gap = false;
                    }
                    delta_buf.push_str(&t);
                }
                AgentEvent::Result {
                    text,
                    is_error,
                    subtype,
                    ..
                } => {
                    result_subtype = Some(subtype.clone());
                    if is_error {
                        reported_error = Some(
                            text.clone()
                                .filter(|t| !t.trim().is_empty())
                                .unwrap_or(subtype),
                        );
                    }
                    if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
                        if !is_error {
                            ok_result = Some(t.clone());
                        }
                        final_result = Some(t);
                    }
                }
                _ => {}
            }
        }
    })
    .await;

    let outcome = driven.outcome;
    // A budget stop: the guard's notice, or Claude's backstop exit, which is
    // checked before the exit status (it exits 1 with no result text).
    let budget_stop = match driven.ended {
        process_group::Ended::LimitReached => Some(
            budget_guard
                .as_ref()
                .and_then(|g| g.stop_notice().cloned())
                .unwrap_or_else(|| crate::budgets::generic_stop_notice(kind)),
        ),
        process_group::Ended::Exited
            if result_subtype.as_deref() == Some(crate::budgets::BACKSTOP_SUBTYPE) =>
        {
            Some(crate::budgets::backstop_notice(kind))
        }
        _ => None,
    };
    // A run its deadline or stop cut off keeps what it wrote (OF04).
    let partial_text = match driven.ended {
        process_group::Ended::TimedOut
        | process_group::Ended::Stopped
        | process_group::Ended::Stalled => {
            cut_off_text(ok_result.as_deref(), &delta_buf, &text_blocks)
        }
        process_group::Ended::LimitReached | process_group::Ended::Exited => String::new(),
    };
    // One ending, one ledger finish: every branch below yields exactly one
    // (result, how it ended) pair and the row is finished once, after it.
    let (result, ended_as) = match driven.ended {
        process_group::Ended::TimedOut => (
            Err(format!("{} timed out", kind.label())),
            LedgerOutcome::Timeout,
        ),
        process_group::Ended::Stopped => (
            Err(format!("{} stopped", kind.label())),
            LedgerOutcome::Cancelled,
        ),
        // Unreachable (the watch above has no watchdog), kept total so the
        // compiler checks every ending.
        process_group::Ended::Stalled => (
            Err(format!("{} stalled", kind.label())),
            LedgerOutcome::Timeout,
        ),
        process_group::Ended::LimitReached | process_group::Ended::Exited
            if budget_stop.is_some() =>
        {
            let message = budget_stop
                .as_ref()
                .map(|notice| notice.message.clone())
                .unwrap_or_default();
            (Err(message), LedgerOutcome::BudgetStopped)
        }
        // Unreachable (a limit always yields a budget stop above), kept
        // total so the compiler checks every ending.
        process_group::Ended::LimitReached => (
            Err(format!("{} stopped at a budget limit", kind.label())),
            LedgerOutcome::BudgetStopped,
        ),
        process_group::Ended::Exited => (
            exited_result(
                kind,
                &driven,
                final_result,
                delta_buf,
                last_text,
                reported_error,
            ),
            LedgerOutcome::Success,
        ),
    };
    let ended_as = match (&result, ended_as) {
        (Err(_), LedgerOutcome::Success) => LedgerOutcome::Error,
        (_, other) => other,
    };
    Collected {
        timed_out: driven.ended == process_group::Ended::TimedOut,
        stopped: driven.ended == process_group::Ended::Stopped,
        result,
        partial_text,
        outcome,
        usage: Some(ledger.finish(ended_as)),
        budget_stop,
        budget_warnings,
        activity: Some(activity),
    }
}

/// What a run cut off before it exited had written (OF04,
/// [`Collected::partial_text`]): a non-error result envelope's text when one
/// arrived, else the streamed deltas (a blank line between messages), else
/// every prose text block in order, joined by a blank line (a cut-off run's
/// last block is rarely the whole answer). The caller passes no error text:
/// an error result and an API-error block are never returned as the answer.
fn cut_off_text(ok_result: Option<&str>, delta_buf: &str, text_blocks: &[String]) -> String {
    if let Some(text) = ok_result.filter(|t| !t.trim().is_empty()) {
        return text.to_string();
    }
    if !delta_buf.trim().is_empty() {
        return delta_buf.to_string();
    }
    text_blocks.join("\n\n")
}

/// The result of a run that exited on its own: a missing or broken stdout, a
/// wait failure, a nonzero exit or a reported error is an `Err`; otherwise the
/// best text (an explicit result envelope, then accumulated plain-text deltas
/// (Antigravity), then the last full text block (Claude/Codex)).
fn exited_result(
    kind: AgentKind,
    driven: &process_group::Driven,
    final_result: Option<String>,
    delta_buf: String,
    last_text: String,
    reported_error: Option<String>,
) -> Result<String, String> {
    if driven.missing_stdout {
        return Err("no stdout".to_string());
    }
    if let Some(error) = &driven.stdout_error {
        return Err(format!("Read {} output: {error}", kind.label()));
    }
    let status = match &driven.status {
        Some(Ok(status)) => *status,
        Some(Err(e)) => return Err(format!("wait {}: {e}", kind.as_str())),
        None => return Err(format!("wait {}: exit status unavailable", kind.as_str())),
    };
    let out = final_result
        .filter(|s| !s.trim().is_empty())
        .or_else(|| (!delta_buf.trim().is_empty()).then(|| delta_buf.clone()))
        .unwrap_or(last_text);
    if !status.success() {
        return Err(format!(
            "{} exited {}: {}",
            kind.as_str(),
            status,
            crate::claude_runner::truncate(driven.stderr_tail.trim(), 400),
        ));
    }
    if let Some(error) = reported_error {
        return Err(format!("{}: {error}", kind.label()));
    }
    Ok(out)
}

/// A normalized stream event. Mirrors exactly the `json!` payloads
/// `chat.rs` emits today, so the caller's translation is mechanical.
/// Lifecycle (`done` / process-exit `error`) stays with the caller.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Completed provider-reported reasoning, separate from answer text.
    Reasoning(String),
    /// Public reported plan counts, separate from tool calls and reasoning.
    Plan { completed: u32, total: u32 },
    /// Passive Codex plan snapshot; never a terminal or a quota request.
    ProviderRateLimit(crate::provider_accounts::limits::PlanLimits),
    /// Latest per-turn token snapshot. Not a message, context measurement,
    /// or terminal outcome; a missing final report leaves it partial.
    TurnUsage(crate::usage::ReportedUsage),
    /// Session / thread id to persist for resume.
    Session(String),
    /// A fresh assistant message is starting — the UI splits the turn here
    /// so a multi-step turn renders text → tool → text in order.
    TurnBoundary,
    /// Token-level streamed text.
    AssistantDelta(String),
    /// A complete assistant text block (used when no deltas streamed it).
    AssistantText(String),
    /// The model is reasoning (OB08a): a Claude or Grok `thinking` or
    /// `redacted_thinking` block (its `content_block_start` when partial
    /// messages stream, and the `assistant` message's part), or a Codex
    /// `reasoning` item. Carries no text and is never shown or stored; it
    /// only times the run's first model event
    /// ([`crate::run_activity::RunActivity::note_event`]). Antigravity's
    /// stream has none.
    Thinking,
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
    },
    /// End-of-turn envelope. `usage` is what the agent reported for the whole
    /// invocation ([`crate::usage::ReportedUsage`]; `Default` when nothing
    /// was): Claude's `modelUsage` sum and cost, Codex's per-turn tokens,
    /// nothing from Antigravity. On a Claude `--resume` it is
    /// session-cumulative; [`crate::usage::resolve`] reduces it. The raw
    /// figure is the transcript's `costUsd`; the usage ledger is authoritative.
    ///
    /// `detail` carries the envelope's structured fields (Paperclip PR 14a),
    /// which [`crate::result_kind`] classifies. Never assistant prose.
    Result {
        text: Option<String>,
        usage: crate::usage::ReportedUsage,
        is_error: bool,
        subtype: String,
        detail: ResultDetail,
    },
    /// Claude's synthetic assistant message that wraps an API error
    /// (`is_api_error_message: true`, with `api_error_status`), emitted
    /// INSTEAD of `AssistantText` (Paperclip PR 14a). Every consumer shows it
    /// exactly as it showed that text before, and also merges it into the
    /// run's result envelope ([`crate::result_kind::ResultTracker`]), so an
    /// "API Error: 529 …" counts as an error envelope, never as prose. The
    /// fields are `@internal` in Claude Code 2.1.283's schema and may be
    /// absent from stream-json; the result and stderr then decide alone.
    ApiError { text: String, status: Option<u16> },
    /// One Claude message's usage. Repeated for the same `message_id` (an
    /// `assistant` event per content block, plus `message_start`), so
    /// consumers deduplicate by id ([`crate::usage::UsageAccumulator`]).
    /// `is_final` marks the `message_delta` figure, which only streams with
    /// `--include-partial-messages`; every other one is a start-of-message
    /// snapshot whose output count is low.
    ///
    /// OB05: `cache_write` is the message's cache-creation tokens (already
    /// folded into `tokens.input`), and `thread` its `parent_tool_use_id`:
    /// `None` on the main thread, the spawning tool call's id for a
    /// subagent's message, which never counts toward the main context.
    MessageUsage {
        message_id: String,
        model: Option<String>,
        tokens: crate::usage::TokenCounts,
        is_final: bool,
        cache_write: u64,
        thread: Option<String>,
    },
    /// The context's fill and window as the agent reports them (Antigravity
    /// over ACP, `usage_update`): the gauge's figure, like Codex's rollout.
    ContextFigure { tokens: u64, window: u64 },
    /// Claude's `rate_limit_event`: the account's five-hour and seven-day
    /// plan windows (Paperclip PR 8c). Every Claude launch point feeds its
    /// stream to the usage ledger, whose `observe` hands this to
    /// [`crate::claude_accounts::observe_stream`], credited to the account
    /// the run used (the login's also reaches the Settings, Usage card).
    /// Never persisted, never emitted to the webview.
    RateLimit(crate::claude_quota::StreamQuota),
}

/// Most entries [`ResultDetail::errors`] keeps.
pub const RESULT_ERRORS_MAX: usize = 8;
/// Characters kept per [`ResultDetail::errors`] entry.
pub const RESULT_ERROR_CHARS: usize = 2000;

/// The structured fields of a result envelope (Paperclip PR 14a). Only the
/// fields a CLI actually printed are filled; prose is never read into it.
/// - Claude: `stop_reason` (`refusal` marks a refusal, at any exit code),
///   `terminal_reason` (`max_turns`, `budget_exhausted`, `completed`, …),
///   `errors[]` and `api_error_status`, plus any merged API-error message.
/// - Codex: `errors` = the `turn.failed` / `error` message.
/// - Antigravity: `denied_actions`, the actions its headless permission
///   mode auto-denied (recorded 2026-09-25: exit 0 and `status: SUCCESS`
///   even though every write was refused).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResultDetail {
    pub stop_reason: Option<String>,
    pub terminal_reason: Option<String>,
    pub errors: Vec<String>,
    pub api_error_status: Option<u16>,
    pub denied_actions: Vec<String>,
}

impl ResultDetail {
    /// Append one error text within the caps ([`RESULT_ERRORS_MAX`] entries
    /// of [`RESULT_ERROR_CHARS`] characters). Blank text is skipped.
    pub fn push_error(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || self.errors.len() >= RESULT_ERRORS_MAX {
            return;
        }
        self.errors
            .push(text.chars().take(RESULT_ERROR_CHARS).collect());
    }
}

/// A trimmed, lowercased string field, when present and non-empty.
fn lower_field(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| v.get(*key).and_then(Value::as_str))
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

/// An HTTP status field (`api_error_status`), when it is a plausible number.
fn status_field(v: &Value, key: &str) -> Option<u16> {
    v.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u16::try_from(n).ok())
        .filter(|n| (100..=599).contains(n))
}

/// Claude's result `errors[]`: strings, or objects' `message` / `error` /
/// `code`, within the caps.
fn claude_result_detail(evt: &Value) -> ResultDetail {
    let mut detail = ResultDetail {
        stop_reason: lower_field(evt, &["stop_reason", "stopReason"]),
        terminal_reason: lower_field(evt, &["terminal_reason", "terminalReason"]),
        api_error_status: status_field(evt, "api_error_status"),
        ..ResultDetail::default()
    };
    if let Some(errors) = evt.get("errors").and_then(Value::as_array) {
        for entry in errors {
            let text = match entry {
                Value::String(s) => Some(s.clone()),
                Value::Object(_) => ["message", "error", "code"].iter().find_map(|key| {
                    entry.get(*key).and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
                }),
                _ => None,
            };
            if let Some(text) = text {
                detail.push_error(&text);
            }
        }
    }
    detail
}

/// Per-stream scratch state threaded across [`parse_line`] calls.
#[derive(Default)]
pub struct ParseState {
    /// Claude: text already streamed via deltas, so skip the aggregated
    /// `assistant` text block to avoid duplicating it.
    got_delta: bool,
    /// Claude: the message each thread is streaming (keyed by
    /// `parent_tool_use_id`, empty for the main thread), from `message_start`,
    /// because `message_delta` carries no id.
    claude_messages: std::collections::HashMap<String, (String, Option<String>)>,
    /// Antigravity: whether we've emitted the opening turn boundary yet.
    started: bool,
    /// Antigravity's explicit step index identifies separate assistant turns.
    antigravity_step: Option<u64>,
    /// Antigravity (agy 1.5.13): the base model from `init` (its level
    /// variant dropped, so the price table's rows match), the steps whose
    /// `usage` was already counted, and their sum. The `result` usage is
    /// SESSION-CUMULATIVE on `--conversation` (a resume reported 36744 input
    /// after steps of 18912 and 17832), so a turn's figure is its own steps'.
    antigravity_model: Option<String>,
    antigravity_counted: std::collections::HashSet<u64>,
    antigravity_tokens: crate::usage::TokenCounts,
    /// Claude, Paperclip PR 8b: a non-interactive run streaming partial
    /// messages only for their usage (`RunSpec::usage_stream`). Partial text
    /// deltas are ignored and `message_start` emits no `TurnBoundary`, so
    /// the text a one-shot or workflow collects is byte-for-byte what it was
    /// without partials; only the `MessageUsage` events (and OB08a's
    /// text-free `Thinking`) are added.
    usage_only_partials: bool,
    /// Codex, Paperclip PR 13: `mcp_tool_call` item ids whose normalized
    /// `ToolUse` was already emitted at `item.started`, so `item.completed`
    /// emits only the result. An id missing here (a stream that skipped the
    /// start) gets both at completion instead.
    codex_started: std::collections::HashSet<String>,
    /// Claude: the stream reported a compaction of the session's history
    /// (a `system` event, subtype `compact_boundary`), so what the session
    /// was given earlier may be summarized away (S02 review).
    pub compacted: bool,
    /// How many `compact_boundary` events the stream carried (OB22): the
    /// receipt's and the run manifest's `compactions`.
    pub compactions: u32,
}

impl ParseState {
    /// Whether this state parses Claude's partial messages for their usage
    /// only (Paperclip PR 8b). Tests read it.
    #[cfg(test)]
    pub fn usage_only(&self) -> bool {
        self.usage_only_partials
    }

    /// The parse state for a run built from `spec`: usage-only partials for
    /// a non-interactive Claude run with [`RunSpec::usage_stream`], the
    /// default otherwise (chat, being interactive, keeps its deltas).
    pub fn for_spec(kind: AgentKind, spec: &RunSpec) -> Self {
        Self {
            usage_only_partials: kind == AgentKind::Claude
                && !spec.interactive
                && spec.usage_stream,
            ..Self::default()
        }
    }
}

/// Cap on a single tool result forwarded to the UI. The on-disk transcript
/// keeps everything; the live stream stays bounded so a huge payload can't
/// stall the IPC channel.
const TOOL_RESULT_MAX: usize = 16_000;

/// Translate one raw stdout line from `kind` into zero or more normalized
/// events. Returns an empty vec for protocol lines that carry nothing the
/// UI needs (heartbeats, unparseable JSON, etc.).
pub fn parse_line(kind: AgentKind, line: &str, state: &mut ParseState) -> Vec<AgentEvent> {
    // `drive` delivers this in place of a stdout line over its cap (A·R3).
    // The line is gone, not clipped: no CLI parser sees it, and no partial
    // JSON ever reaches one. `drive` notes the drop in the stderr tail.
    if crate::process_group::dropped_line_bytes(line).is_some() {
        return Vec::new();
    }
    match kind {
        AgentKind::Claude => parse_claude(line, state),
        AgentKind::Codex => parse_codex(line, state),
        AgentKind::Antigravity => parse_antigravity(line, state),
        // Grok's `streaming-messages-json` is Claude Code's stream-json
        // (recorded in fixtures/grok), deltas included.
        AgentKind::Grok => parse_claude(line, state),
    }
}

fn parse_claude(line: &str, state: &mut ParseState) -> Vec<AgentEvent> {
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(evt): serde_json::Result<Value> = serde_json::from_str(line) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match evt.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "system" => {
            if let Some(sid) = evt.get("session_id").and_then(|v| v.as_str()) {
                out.push(AgentEvent::Session(sid.to_string()));
            }
            if evt.get("subtype").and_then(Value::as_str) == Some("compact_boundary") {
                state.compacted = true;
                state.compactions = state.compactions.saturating_add(1);
            }
            if evt.get("subtype").and_then(Value::as_str) == Some("context") {
                if let (Some(tokens), Some(window)) = (
                    evt.get("tokens").and_then(Value::as_u64),
                    evt.get("window").and_then(Value::as_u64),
                ) {
                    out.push(AgentEvent::ContextFigure { tokens, window });
                }
            }
        }
        "stream_event" => {
            if let Some(inner) = evt.get("event") {
                let thread = evt
                    .get("parent_tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                match inner.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                    "message_start" => {
                        if !state.usage_only_partials {
                            out.push(AgentEvent::TurnBoundary);
                        }
                        if let Some(message) = inner.get("message") {
                            if let Some(id) = message.get("id").and_then(Value::as_str) {
                                let model = message
                                    .get("model")
                                    .and_then(Value::as_str)
                                    .map(crate::usage::normalize_model);
                                state
                                    .claude_messages
                                    .insert(thread.clone(), (id.to_string(), model));
                            }
                            if let Some((message_id, model, tokens)) =
                                crate::usage::claude_message_usage(message)
                            {
                                out.push(AgentEvent::MessageUsage {
                                    message_id,
                                    model,
                                    tokens,
                                    is_final: false,
                                    cache_write: crate::usage::claude_cache_write(message),
                                    thread: Some(thread).filter(|t| !t.is_empty()),
                                });
                            }
                        }
                    }
                    "message_delta" => {
                        if let (Some((message_id, model)), Some(tokens)) = (
                            state.claude_messages.get(&thread),
                            crate::usage::claude_delta_usage(inner),
                        ) {
                            out.push(AgentEvent::MessageUsage {
                                message_id: message_id.clone(),
                                model: model.clone(),
                                tokens,
                                is_final: true,
                                cache_write: crate::usage::claude_cache_write(inner),
                                thread: Some(thread).filter(|t| !t.is_empty()),
                            });
                        }
                    }
                    // OB08a: a thinking block starts. In either mode: it
                    // carries no text, so a usage-only run's text is unchanged.
                    "content_block_start" => {
                        let block = inner.get("content_block").and_then(|b| b.get("type"));
                        if matches!(
                            block.and_then(Value::as_str),
                            Some("thinking" | "redacted_thinking")
                        ) {
                            out.push(AgentEvent::Thinking);
                        }
                    }
                    // Usage-only mode ignores partial text: the `assistant`
                    // event still carries each block in full.
                    "content_block_delta" if !state.usage_only_partials => {
                        if let Some(delta) = inner.get("delta") {
                            if delta.get("type").and_then(|v| v.as_str()) == Some("text_delta") {
                                if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                                    state.got_delta = true;
                                    out.push(AgentEvent::AssistantDelta(t.to_string()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "assistant" => {
            if let Some((message_id, model, tokens)) = evt
                .get("message")
                .and_then(crate::usage::claude_message_usage)
            {
                out.push(AgentEvent::MessageUsage {
                    message_id,
                    model,
                    tokens,
                    is_final: false,
                    cache_write: evt
                        .get("message")
                        .map_or(0, crate::usage::claude_cache_write),
                    thread: evt
                        .get("parent_tool_use_id")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .map(str::to_string),
                });
            }
            // Paperclip PR 14a: an assistant message that wraps an API error
            // is an error envelope, not the model's prose. Its text parts are
            // joined into one `ApiError` in place of `AssistantText`.
            if evt.get("is_api_error_message").and_then(Value::as_bool) == Some(true) {
                let text = evt
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                out.push(AgentEvent::ApiError {
                    text,
                    status: status_field(&evt, "api_error_status"),
                });
                return out;
            }
            if let Some(content) = evt
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for part in content {
                    match part.get("type").and_then(|v| v.as_str()) {
                        Some("text") => {
                            // Only emit the full block when deltas didn't.
                            if !state.got_delta {
                                if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                    out.push(AgentEvent::AssistantText(t.to_string()));
                                }
                            }
                        }
                        Some("tool_use") => {
                            out.push(AgentEvent::ToolUse {
                                id: str_field(part, "id"),
                                name: str_field(part, "name"),
                                input: part.get("input").cloned().unwrap_or(Value::Null),
                            });
                        }
                        Some("thinking") => {
                            out.push(AgentEvent::Thinking);
                            if let Some(text) = part
                                .get("thinking")
                                .and_then(Value::as_str)
                                .filter(|s| !s.trim().is_empty())
                            {
                                out.push(AgentEvent::Reasoning(
                                    text.chars().take(16_000).collect(),
                                ));
                            }
                        }
                        Some("redacted_thinking") => out.push(AgentEvent::Thinking),
                        _ => {}
                    }
                }
            }
        }
        "user" => {
            if let Some(content) = evt
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
            {
                for part in content {
                    if part.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                        out.push(AgentEvent::ToolResult {
                            id: str_field(part, "tool_use_id"),
                            content: extract_tool_result_text(part.get("content")),
                            is_error: part
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        });
                    }
                }
            }
        }
        "result" => {
            if let Some(sid) = evt.get("session_id").and_then(|v| v.as_str()) {
                out.push(AgentEvent::Session(sid.to_string()));
            }
            out.push(AgentEvent::Result {
                text: evt.get("result").and_then(|v| v.as_str()).map(String::from),
                usage: crate::usage::claude_result_usage(&evt),
                is_error: evt
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                subtype: str_field(&evt, "subtype"),
                detail: claude_result_detail(&evt),
            });
        }
        // Its own arm: no `Session` from its `session_id`, and nothing else
        // in the stream changes (Paperclip PR 8c).
        "rate_limit_event" => {
            if let Some(quota) = crate::claude_quota::parse_rate_limit_event(&evt) {
                out.push(AgentEvent::RateLimit(quota));
            }
        }
        _ => {}
    }
    out
}

/// Codex's MCP tool call, normalized to Claude's naming (Paperclip PR 13): a
/// `hollow_vault`/`search_vault` call becomes `mcp__hollow_vault__search_vault`
/// with the call's own arguments as its input, so every consumer (question
/// cards, the teamwork image path, the UI's connector rows) keys on one name
/// for both agents. `None` when the item lacks a server or tool name, which
/// then takes the legacy generic pair.
fn codex_mcp_name(item: &Value) -> Option<String> {
    let server = item
        .get("server")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let tool = item
        .get("tool")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    Some(format!("mcp__{server}__{tool}"))
}

/// The arguments of a Codex `mcp_tool_call` item. Recorded CLIs send an
/// object; a JSON string is parsed defensively, and anything else is kept as
/// is (the question recorder then rejects it visibly).
fn codex_mcp_arguments(item: &Value) -> Value {
    match item.get("arguments") {
        Some(Value::String(raw)) => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
        }
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

/// The normalized result of a completed Codex `mcp_tool_call` item. The item
/// has no `isError` field (recorded, codex-cli 0.153.4): failure is
/// `status: "failed"` or a non-null `error` (`{message}`). The text comes from
/// the MCP content blocks in `result.content`, else the structured content,
/// else the error message.
fn codex_mcp_result(item: &Value) -> (String, bool) {
    let error = item.get("error").filter(|e| !e.is_null());
    let failed = item.get("status").and_then(Value::as_str) == Some("failed") || error.is_some();
    let result = item.get("result").filter(|r| !r.is_null());
    let mut text = extract_tool_result_text(result.and_then(|r| r.get("content")));
    if text.is_empty() {
        if let Some(structured) = result
            .and_then(|r| r.get("structured_content"))
            .filter(|s| !s.is_null())
        {
            text = truncate_tool(structured.to_string());
        }
    }
    if text.is_empty() {
        if let Some(error) = error {
            text = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
        }
    }
    (text, failed)
}

/// Commands whose exit 1 means "no match" or "differs", not a failure, when
/// they end the script ([`codex_command_result`]).
const NO_MATCH_HEADS: &[&str] = &["rg", "grep", "egrep", "fgrep", "diff", "cmp", "test", "["];
/// The most output a no-match exit may carry (a `diff` of a line or two).
const NO_MATCH_BYTES: usize = 200;

/// A Codex `command_execution` result as (content, is_error) (OF18). A
/// non-zero exit starts the content with `Exit code N`, as Claude Code's Bash
/// results do (`Exit code N (no output)` when it printed nothing), so a
/// reader sees why a quiet or redirected command failed and friction signs
/// it by code and command head. An exit 1 from a script whose last command
/// is a search or comparison (`rg`, `grep`, `diff`, `test`, …) and whose
/// exit status is necessarily that command's ([`ends_in_predicate`]), with
/// at most [`NO_MATCH_BYTES`] of output, is a no-match: `Exit code 1 (no
/// match)`, not an error. A clean exit (or none reported) keeps the output
/// as it was.
pub(crate) fn codex_command_result(
    command: &str,
    output: &str,
    exit: Option<i64>,
) -> (String, bool) {
    let code = match exit {
        Some(code) if code != 0 => code,
        _ => return (output.to_string(), false),
    };
    let quiet = output.trim().is_empty();
    if code == 1 && output.len() <= NO_MATCH_BYTES && ends_in_predicate(command) {
        let content = if quiet {
            "Exit code 1 (no match)".to_string()
        } else {
            format!("Exit code 1 (no match)\n{output}")
        };
        return (content, false);
    }
    if quiet {
        (format!("Exit code {code} (no output)"), true)
    } else {
        (format!("Exit code {code}\n{output}"), true)
    }
}

/// Whether the script `command` runs (a `<shell> -lc '…'` wrapper taken off)
/// ends with a [`NO_MATCH_HEADS`] command whose exit status is necessarily
/// the script's: its last simple command, past leading assignments and
/// wrappers, by its file name (`/usr/bin/grep` is `grep`), and (OF18
/// review) only when nothing else can have produced the exit:
/// - its pipeline doesn't run after `&&` (`npx vitest run >v.log 2>&1 &&
///   grep -q passed v.log`: a failed run skips the `grep`, and the 1 is
///   vitest's). After `||`, `;`, `&` or `|` a non-zero exit is the
///   predicate's own (a `|` without pipefail: the pipeline's status is its
///   last command's);
/// - it isn't itself backgrounded (`rg foo &`), its pipeline isn't negated
///   (`! cat x | grep -q error`: the 1 is a match), and it isn't a nested
///   shell (`sh -c 'test -d x && npm run build'`, [`runs_a_shell`]);
/// - no command in the script takes the exit (`[ -d x ] || exit 1⏎rg foo`,
///   `then exit 1`, `source env.sh`, `eval`) or turns on errexit or pipefail
///   (`set -e; npm run build >b.log; grep err b.log`):
///   `evidence::changes_script_exit`, which reads wide.
///
/// An unreadable script (a subshell, `$(…)`) never does. In doubt the exit
/// stays an error.
fn ends_in_predicate(command: &str) -> bool {
    use crate::run_effects::Joint;
    let script = crate::run_effects::unwrap_shell_wrapper(command);
    let Some(segments) = crate::run_effects::split_joined(&script, true) else {
        return false;
    };
    let Some(last) = segments.last() else {
        return false;
    };
    let predicate = crate::friction::command_head(&last.text)
        .is_some_and(|head| NO_MATCH_HEADS.contains(&head.as_str()));
    if !predicate || last.joint == Joint::Background || runs_a_shell(&last.text) {
        return false;
    }
    let mut start = segments.len() - 1;
    while start > 0 && segments[start - 1].joint == Joint::Pipe {
        start -= 1;
    }
    if start > 0 && segments[start - 1].joint == Joint::And {
        return false;
    }
    // `! cat x | grep -q error`: the pipeline's status is negated, so its 1
    // is a match.
    if [start, segments.len() - 1]
        .iter()
        .any(|&i| segments[i].text.trim_start().starts_with('!'))
    {
        return false;
    }
    !segments
        .iter()
        .any(|s| crate::evidence::changes_script_exit(&s.text))
}

/// Whether a simple command runs a nested shell script (`sh -c '…'`, past
/// leading assignments and wrappers): [`ends_in_predicate`] can't read its
/// joints, though `friction::command_head` unwraps it to its first command
/// (OF18 re-review).
fn runs_a_shell(segment: &str) -> bool {
    let words = crate::run_effects::shell_words(segment);
    let Some(at) = words.iter().position(|w| {
        let assignment = w.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        !(assignment || matches!(w.as_str(), "env" | "time" | "exec" | "nohup" | "command"))
    }) else {
        return false;
    };
    let first = words[at].as_str();
    let program = first.rsplit('/').next().unwrap_or(first);
    matches!(program, "sh" | "bash" | "zsh" | "dash")
        && words[at + 1..]
            .iter()
            .any(|w| w.starts_with('-') && !w.starts_with("--") && w.contains('c'))
}

fn codex_plan(item: &Value) -> Option<AgentEvent> {
    let items = item.get("items")?.as_array()?;
    Some(AgentEvent::Plan {
        completed: items
            .iter()
            .filter(|i| i.get("completed").and_then(Value::as_bool) == Some(true))
            .count() as u32,
        total: items.len() as u32,
    })
}

fn parse_codex(line: &str, state: &mut ParseState) -> Vec<AgentEvent> {
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(evt): serde_json::Result<Value> = serde_json::from_str(line) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match evt.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "thread.started" => {
            if let Some(id) = evt.get("thread_id").and_then(|v| v.as_str()) {
                out.push(AgentEvent::Session(id.to_string()));
            }
        }
        // Tool/command items stream as started → completed pairs.
        "item.started" | "item.updated" => {
            let item = evt.get("item").unwrap_or(&Value::Null);
            let started = evt.get("type").and_then(|v| v.as_str()) == Some("item.started");
            match item.get("type").and_then(|v| v.as_str()) {
                // Emit ToolUse once, on the started event.
                Some("command_execution") if started => {
                    out.push(AgentEvent::ToolUse {
                        id: str_field(item, "id"),
                        name: "Bash".to_string(),
                        input: json!({ "command": str_field(item, "command") }),
                    });
                }
                // Paperclip PR 13: an MCP call starts as a normalized
                // `mcp__<server>__<tool>` ToolUse carrying its arguments; its
                // result follows at `item.completed`.
                Some("mcp_tool_call") if started => {
                    if let Some(name) = codex_mcp_name(item) {
                        let id = str_field(item, "id");
                        state.codex_started.insert(id.clone());
                        out.push(AgentEvent::ToolUse {
                            id,
                            name,
                            input: codex_mcp_arguments(item),
                        });
                    }
                }
                // OB08a: the model is reasoning (its summary stays private).
                Some("reasoning") if started => out.push(AgentEvent::Thinking),
                // Public progress never consumes a transcript tool slot.
                Some("todo_list") => {
                    if let Some(plan) = codex_plan(item) {
                        out.push(plan);
                    }
                }
                _ => {}
            }
        }
        "item.completed" => {
            let item = evt.get("item").cloned().unwrap_or(Value::Null);
            match item.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "agent_message" => {
                    // Each agent message is its own turn block (mirrors
                    // Claude's per-message boundary), so text after a tool
                    // call renders as a fresh assistant bubble.
                    out.push(AgentEvent::TurnBoundary);
                    out.push(AgentEvent::AssistantText(str_field(&item, "text")));
                }
                "todo_list" => {
                    if let Some(plan) = codex_plan(&item) {
                        out.push(plan);
                    }
                }
                "command_execution" => {
                    let exit = item.get("exit_code").and_then(|v| v.as_i64());
                    let (content, is_error) = codex_command_result(
                        &str_field(&item, "command"),
                        &str_field(&item, "aggregated_output"),
                        exit,
                    );
                    out.push(AgentEvent::ToolResult {
                        id: str_field(&item, "id"),
                        content: truncate_tool(content),
                        is_error,
                    });
                }
                // Keep reported summaries separate from the assistant answer.
                "reasoning" => {
                    out.push(AgentEvent::Thinking);
                    if let Some(text) = item
                        .get("text")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        out.push(AgentEvent::Reasoning(text.chars().take(16_000).collect()));
                    }
                }
                // Paperclip PR 13: the result of a normalized MCP call. A call
                // whose start was not seen gets its ToolUse here first; an item
                // with no server or tool name falls through to the legacy
                // generic pair below.
                "mcp_tool_call" if codex_mcp_name(&item).is_some() => {
                    let id = str_field(&item, "id");
                    if !state.codex_started.remove(&id) {
                        out.push(AgentEvent::ToolUse {
                            id: id.clone(),
                            name: codex_mcp_name(&item).unwrap_or_default(),
                            input: codex_mcp_arguments(&item),
                        });
                    }
                    let (content, is_error) = codex_mcp_result(&item);
                    out.push(AgentEvent::ToolResult {
                        id,
                        content,
                        is_error,
                    });
                }
                // Any other tool-ish item (file_change, web_search, an
                // mcp_tool_call without server/tool names, …): surface it as a
                // one-shot tool row so it's visible and doesn't hang in a
                // "running" state. OB08a's clock counts these as tool calls,
                // except a non-fatal `error` item (`RunActivity::note_event`).
                other if !other.is_empty() => {
                    let id = str_field(&item, "id");
                    out.push(AgentEvent::ToolUse {
                        id: id.clone(),
                        name: other.to_string(),
                        input: item.clone(),
                    });
                    out.push(AgentEvent::ToolResult {
                        id,
                        content: String::new(),
                        is_error: false,
                    });
                }
                _ => {}
            }
        }
        "account.rate_limits" => {
            if let Some(limits) =
                crate::provider_accounts::limits::parse(evt.get("limits").unwrap_or(&Value::Null))
            {
                out.push(AgentEvent::ProviderRateLimit(limits));
            }
        }
        "turn.usage" => {
            out.push(AgentEvent::TurnUsage(crate::usage::codex_turn_usage(&evt)));
        }
        "turn.completed" => {
            out.push(AgentEvent::Result {
                text: None,
                usage: crate::usage::codex_turn_usage(&evt),
                is_error: false,
                subtype: "success".to_string(),
                detail: ResultDetail::default(),
            });
        }
        "turn.failed" | "error" => {
            // `error.message`, then `error` as a string, then (Paperclip PR
            // 14a) a top-level `message`, which a bare `{type:"error",
            // message}` event carries and which used to read "codex error".
            let msg = evt
                .get("error")
                .and_then(|e| {
                    e.get("message")
                        .and_then(|m| m.as_str())
                        .or_else(|| e.as_str())
                })
                .or_else(|| evt.get("message").and_then(Value::as_str))
                .filter(|m| !m.trim().is_empty())
                .unwrap_or("codex error");
            let mut detail = ResultDetail::default();
            detail.push_error(msg);
            // Overnight 9 A3: the error's code (`server_overloaded`), so the
            // classifier reads the cause and not only its wording.
            if let Some(info) = evt
                .get("error")
                .and_then(|e| e.get("codex_error_info"))
                .and_then(Value::as_str)
            {
                detail.push_error(info);
            }
            out.push(AgentEvent::Result {
                text: Some(msg.to_string()),
                usage: Default::default(),
                is_error: true,
                subtype: "error".to_string(),
                detail,
            });
        }
        _ => {}
    }
    out
}

fn parse_antigravity(line: &str, state: &mut ParseState) -> Vec<AgentEvent> {
    // Verified against installed agy's --output-format stream-json (2026-09-07).
    // The protocol uses `event` / nested payloads, not Claude's `type` envelope.
    // Unknown protocol events are ignored, never rendered as assistant prose.
    let Ok(evt): Result<Value, _> = serde_json::from_str(line) else {
        return Vec::new();
    };
    // An ACP turn (`agy acp`, `acp_client`) speaks to the parser in the
    // stream-json shape Grok's ACP turns use: a `type` field, no `event`.
    if evt.get("type").is_some() && evt.get("event").is_none() {
        return parse_claude(line, state);
    }
    let mut out = Vec::new();
    match evt.get("event").and_then(Value::as_str) {
        Some("init") => {
            if let Some(id) = evt
                .get("conversation_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.push(AgentEvent::Session(id.into()));
            }
            state.antigravity_model = evt
                .get("init")
                .and_then(|init| init.get("model"))
                .and_then(Value::as_str)
                .filter(|m| !m.trim().is_empty())
                .map(|m| crate::effort::agy_base(m).to_string());
        }
        Some("step_update") => {
            let Some(step) = evt.get("step_update") else {
                return out;
            };
            // A finished step's token usage: one message per step, so the
            // context gauge reads the last call's prompt and the ledger sums
            // the turn.
            if step.get("state").and_then(Value::as_str) == Some("DONE") {
                if let (Some(index), Some(tokens)) = (
                    step.get("step_index").and_then(Value::as_u64),
                    step.get("usage")
                        .and_then(crate::usage::antigravity_counter),
                ) {
                    if state.antigravity_counted.insert(index) {
                        state.antigravity_tokens = state.antigravity_tokens.add(tokens);
                        let conversation = step
                            .get("conversation_id")
                            .and_then(Value::as_str)
                            .unwrap_or("agy");
                        out.push(AgentEvent::MessageUsage {
                            message_id: format!("{conversation}:{index}"),
                            model: state.antigravity_model.clone(),
                            tokens,
                            is_final: true,
                            cache_write: 0,
                            thread: None,
                        });
                    }
                }
            }
            if step.get("step_type").and_then(Value::as_str) == Some("agent_response") {
                if let Some(delta) = step
                    .get("text_delta")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                {
                    let index = step.get("step_index").and_then(Value::as_u64);
                    if !state.started || index != state.antigravity_step {
                        state.started = true;
                        state.antigravity_step = index;
                        out.push(AgentEvent::TurnBoundary);
                    }
                    state.got_delta = true;
                    out.push(AgentEvent::AssistantDelta(delta.into()));
                }
            }
        }
        Some("result") => {
            let Some(result) = evt.get("result") else {
                return out;
            };
            if let Some(id) = result
                .get("conversation_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.push(AgentEvent::Session(id.into()));
            }
            let status = result
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN");
            // Paperclip PR 14a: the actions headless mode auto-denied
            // (`denied_actions[].action`, recorded with agy 1.2.10). The run
            // still reports `SUCCESS` and exits 0.
            let denied_actions = result
                .get("denied_actions")
                .and_then(Value::as_array)
                .map(|actions| {
                    actions
                        .iter()
                        .filter_map(|a| {
                            a.get("action")
                                .or_else(|| a.get("display_name"))
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_string)
                        })
                        .take(RESULT_ERRORS_MAX)
                        .collect()
                })
                .unwrap_or_default();
            out.push(AgentEvent::Result {
                text: result
                    .get("response")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                usage: crate::usage::antigravity_turn_usage(
                    state.antigravity_tokens,
                    state.antigravity_model.as_deref(),
                ),
                is_error: status != "SUCCESS",
                subtype: status.into(),
                detail: ResultDetail {
                    denied_actions,
                    ..ResultDetail::default()
                },
            });
        }
        _ => {}
    }
    out
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn truncate_tool(mut s: String) -> String {
    if s.len() > TOOL_RESULT_MAX {
        let mut end = TOOL_RESULT_MAX;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        let omitted = s.len() - end;
        s.truncate(end);
        s.push_str(&format!("\n… (+{omitted} more bytes truncated)"));
    }
    s
}

/// Flatten a Claude `tool_result` block's `content` to display text. The
/// content is either a plain string or an array of content blocks
/// (text / image). Joins text parts, notes image parts inline, and
/// truncates on a char boundary.
pub(crate) fn extract_tool_result_text(content: Option<&Value>) -> String {
    let mut out = String::new();
    match content {
        Some(Value::String(s)) => out.push_str(s),
        Some(Value::Array(arr)) => {
            for block in arr {
                match block.get("type").and_then(|v| v.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                            out.push_str(t);
                        }
                    }
                    Some("image") => {
                        if !out.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str("[image]");
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    truncate_tool(out)
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
