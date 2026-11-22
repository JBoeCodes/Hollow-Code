//! Replay bench on real turn sequences (overnight 10-17 S07): how many
//! config-changed resets HEAD's resume plan makes on the turns the Sessions
//! DB recorded, against how many were recorded. Baseline (2026-10-09 →
//! 10-16T21:16Z): 102 config-changed resets in Claude sessions (20.2 per
//! 100 Claude turns), mode flips 14.85 per 100 of them, the Main agent 24.77
//! per 100, and 34 of 80 Main-agent wakes reset the session (every wake:
//! a finished chat, a check-in, a swarm, a `wake_me` wake-up, the weekly
//! update, the morning review; `main_agent::is_wake_message`, as OF17's host
//! goals count them).
//!
//! **Replay** ([`replay_session`]): conversation by conversation, each
//! turn's `FingerprintInput` is rebuilt from what was recorded
//! ([`TurnInput`]) and goes through HEAD's own
//! `chat_resume::config_fingerprint` and `plan_resume_with`, with a
//! `ResumeState` built the way the chat path reads it: the open stint holds
//! the native id the last new or reset turn announced, stamped with that
//! turn's fingerprint and its per-part hashes (`config_parts`, OB03), and a
//! turn of another agent closes it (`agent changed`, as
//! `session_store::start_turn` does) and starts new, with no native id
//! requested (the composer drops it on an agent switch). Every stamp is v3,
//! so no restamp applies. A reset is named as the chat path names it
//! (`chat_resume::reset_parts`): OB03's [`PARTS`] whose hash differs from
//! the stamp, [`UNKNOWN_PART`] when none does.
//!
//! **Recorded** ([`bench`]): stints closed `config changed`, each at the
//! latest turn that started by its close. Recorded stints before OB03 hold
//! no parts, so recorded resets are named in the same [`PARTS`] by diffing
//! the rebuilt inputs against the previous turn ([`parts_between`]), with
//! the mode itself compared (what v1 hashed), and [`UNKNOWN_PART`] for none
//! (v2's appended prompt bytes, a prompt contract bump, or an input not
//! rebuilt here). A close that lands on a turn before the window, on a turn
//! skipped for want of an agent, or before any turn is counted in
//! `unmatched` instead.
//!
//! **Naming** (both sides): `byPart` names a reset by its *first* differing
//! part in [`PARTS`] order, so `mode` wins over `mcp`; `recordedByPartAll`
//! and `replayedByPartAll` count every differing part. An unattended wake
//! (acceptEdits, without teammates, computer control or services) next to
//! the person's bypass turn records as `mode` and replays as `mcp`: a real
//! reset at HEAD while unattended turns alternate with the person's. A flip
//! is a reset with `mode` among its differing parts (the mode itself on the
//! recorded side, its class on the replayed one); `modeChangedResets`
//! counts resets whose turn merely ran in another mode than the turn before
//! it, whatever differed. `assumedPartResets` counts resets with a
//! differing part filled in on either turn rather than recorded (below).
//!
//! **Inputs** ([`load`]): `turns.permission_mode`, the turn's chat usage row
//! (`usage_ledger`: the requested model and the agent), `sessions.cwd` (the
//! conversation's folder now: a moved conversation's earlier folder is
//! lost, and a move is its own reset reason anyway) and, where the run still
//! has its context manifest (the newest `evidence::KEEP` runs, read in place
//! from the vault copy's `.evidence-manifests`), `capabilities.ownedServers`
//! and the Canvases choice its `compiler.visualGuide.delivery` implies. A
//! value a turn lacks comes from the same conversation: the model and mode
//! of the turn before, the servers and Canvases of the nearest turn with a
//! manifest (one of the same kind first: plan or not, unattended or not,
//! wake or not, then plan and unattended alone), else a fixed default (the
//! most common server set over the manifests read, per agent, plan and
//! unattended kind; Canvases' default). Borrowed servers are cut to what the
//! turn's kind attaches ([`servers_for`]): no team server or services in
//! plan mode, and none of those or computer control on an unattended turn
//! (`mcp_attach::for_unattended`). A turn is unattended when its request is
//! one Hollow queues so ([`is_unattended_request`]): a `wake_me` wake-up,
//! the morning review or a night-queue item; a runbook step run at night
//! isn't told apart. Other wakes (a finished chat, a check-in, a swarm, the
//! weekly update) run attended. `assumed` counts every such turn by field.
//! The attached capabilities follow from the servers ([`caps_of`],
//! `mcp_attach::server_map_for`'s rules) with every teammate CLI taken as
//! installed; the prompt kind follows from the folder ([`prompt_kind`]).
//!
//! The real run is an ignored test reading a vault copy and a Sessions DB
//! named by environment variables (see `docs/reference/build-and-ship.md`
//! for the exact command and its `trap`):
//!
//! ```text
//! TMPDIR=<per-run mktemp -d folder> HOLLOW_REPLAY_VAULT=<vault copy without restricted/> \
//! HOLLOW_REPLAY_DB=<Sessions DB> HOLLOW_REPLAY_BENCH_OUT=<bench.json> \
//! [HOLLOW_REPLAY_FROM=<RFC 3339>] [HOLLOW_REPLAY_TO=<RFC 3339>] \
//!   cargo test --lib -- --ignored replay_bench
//! ```
//!
//! The DB is snapshotted by OF17's [`Replay::setup`]: 0600 under a fresh
//! 0700 temp folder, deleted when the guard drops, never under a worktree.
//! A request's text is read only to tell a wake and an unattended turn and
//! never leaves the loader. **Output**: counts and per-100 rates keyed by
//! Hollow's own vocabulary (OB03's fingerprint parts, manifest entry and
//! session kinds), plus the window and the snapshot's `quick_check` (the only
//! strings); never a session key, a path, a model name or any text.
//! `scripts/agent-eval/replay-bench.mjs` prints it as recorded → replayed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use crate::agent::AgentKind;
use crate::chat_resume::{
    caps_bits, config_fingerprint, config_parts, mode_class, plan_resume_with, reset_parts,
    FingerprintInput, ResetReason, ResumePlan,
};
use crate::main_agent::{is_wake_message, WAKE_PREFIX};
use crate::mcp_attach::AttachedCaps;
use crate::night_queue::{MORNING_REVIEW_OPEN, NIGHT_PREFIX};
use crate::replay_probes::{env_nonempty, workspace_of, Replay, RESTRICTED};
use crate::run_manifest::RunManifest;
use crate::session_store::{parse_iso, ClosedStint, OpenStint, ResumeState};
use crate::visual_prompt::CanvasFrequency;

/// The fingerprint's parts a reset is named by, and the name for none:
/// OB03's own, so the bench, the app's reset notice and the scorecard's
/// `continuity.resetsByPart` share one vocabulary.
pub(crate) use crate::chat_resume::{PARTS, UNKNOWN_PART};

/// The report's schema.
pub(crate) const SCHEMA: u32 = 1;
/// The part a flip differs in (one of [`PARTS`]).
const MODE_PART: &str = "mode";
/// Hollow passes a turn no skills (`FingerprintInput::skills`).
const NO_SKILLS: &[String] = &[];
/// The default window: the recorded baseline's.
pub(crate) const DEFAULT_FROM: &str = "2026-10-09T00:00:00Z";
pub(crate) const DEFAULT_TO: &str = "2026-10-16T21:16:00Z";
/// `session_store::start_turn`'s close reason for a turn of another agent.
const AGENT_CHANGED: &str = "agent changed";
/// A manifest larger than this is not read (`run_manifest`'s own cap).
const MANIFEST_MAX_BYTES: u64 = 512 * 1024;
/// The folders a bare scratch chat runs under (`stakeholder_sim::root`'s
/// and the removed bake-off runner's, still in old transcripts).
const BARE_ROOTS: [&str; 2] = ["PressureTests", "Bakeoffs"];
/// The team server (`mcp_attach::attach_team`).
const TEAM_SERVER: &str = "hollow_team";
/// The computer-control server (`computer_mcp::server_entry`).
const COMPUTER_SERVER: &str = "hollow_computer";
/// The mode the chat path reads an unknown one as (`app_mcp::normalize_perm`).
const FALLBACK_PERM: &str = "acceptEdits";

// ── The pure core ─────────────────────────────────────────────────────────

/// One turn's rebuilt fingerprint input (see the module doc for where each
/// field comes from).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TurnInput {
    pub agent: AgentKind,
    /// The requested model (`None`: the agent's default).
    pub model: Option<String>,
    /// The permission mode as recorded.
    pub perm: String,
    pub cwd: PathBuf,
    /// The attached Hollow MCP servers' names.
    pub mcp_servers: Vec<String>,
    pub canvases: CanvasFrequency,
    /// `chat::PromptKind`'s name ([`prompt_kind`]).
    pub workspace: &'static str,
}

/// The [`AttachedCaps`] a chat turn had, from the servers it attached, by
/// `mcp_attach::server_map_for`'s rules: a read-only task or app server in
/// plan mode, a fenced question block for an agent without owned MCP,
/// nothing but the agent for a bare turn, and every teammate but the agent
/// itself while the team server is attached (every teammate CLI taken as
/// installed).
pub(crate) fn caps_of(turn: &TurnInput) -> AttachedCaps {
    let agent = turn.agent;
    if turn.workspace == "bare" {
        return AttachedCaps {
            agent: Some(agent),
            ..AttachedCaps::default()
        };
    }
    let has = |name: &str| turn.mcp_servers.iter().any(|server| server == name);
    let plan = turn.perm.trim() == "plan";
    let team = has(TEAM_SERVER);
    let tasks = has(crate::tasks_mcp::SERVER_NAME);
    let app = has(crate::app_mcp::SERVER_NAME);
    AttachedCaps {
        computer: has(COMPUTER_SERVER),
        vault: has("hollow_vault"),
        code_context: has(crate::code_context_mcp::SERVER_NAME),
        team_claude: team && agent != AgentKind::Claude,
        team_codex: team && agent != AgentKind::Codex,
        team_antigravity: team && agent != AgentKind::Antigravity,
        team_grok: team && agent != AgentKind::Grok,
        questions: has("hollow_questions"),
        tasks,
        tasks_read_only: tasks && plan,
        app,
        app_read_only: app && plan,
        services: has(crate::services_mcp::SERVER_NAME),
        questions_fenced: !crate::agent::capabilities(agent).owned_mcp,
        agent: Some(agent),
    }
}

/// `turn`'s fingerprint input (no skills, as the chat path).
fn input_of(turn: &TurnInput) -> FingerprintInput<'_> {
    FingerprintInput {
        agent: turn.agent,
        model: turn.model.as_deref(),
        perm: &turn.perm,
        cwd: &turn.cwd,
        mcp_servers: &turn.mcp_servers,
        caps: caps_of(turn),
        canvases: turn.canvases,
        workspace: turn.workspace,
        skills: NO_SKILLS,
    }
}

/// HEAD's `config_fingerprint` of `turn`.
pub(crate) fn fingerprint_of(turn: &TurnInput) -> String {
    config_fingerprint(&input_of(turn))
}

/// HEAD's per-part hashes of `turn` (`chat_resume::config_parts`, OB03),
/// what the chat path stamps a stint with beside its fingerprint.
pub(crate) fn parts_of(turn: &TurnInput) -> String {
    config_parts(&input_of(turn))
}

fn model_of(turn: &TurnInput) -> &str {
    turn.model.as_deref().map(str::trim).unwrap_or("")
}

/// The mode as hashed today (`mode_class`), or the mode itself when `raw`
/// (what v1 hashed; the recorded side's naming).
fn mode_of(turn: &TurnInput, raw: bool) -> &'static str {
    if raw {
        crate::app_mcp::normalize_perm(&turn.perm)
    } else {
        mode_class(turn.agent, &turn.perm)
    }
}

/// Sorted and deduplicated, as the fingerprint hashes a name list.
fn sorted(items: &[String]) -> Vec<&str> {
    let mut items: Vec<&str> = items.iter().map(String::as_str).collect();
    items.sort_unstable();
    items.dedup();
    items
}

fn caps_key(turn: &TurnInput) -> Vec<String> {
    let mut bits = caps_bits(&caps_of(turn));
    bits.sort_unstable();
    bits
}

/// The [`PARTS`] in which the rebuilt inputs `a` and `b` differ, in that
/// order, each compared as the fingerprint hashes it; the mode itself
/// instead of its class when `raw_mode`. The recorded side's naming:
/// recorded stints before OB03 hold no parts, so their resets are named
/// from the inputs instead. `contract` and `skills` are never rebuilt
/// (constant within a replay), so they never differ here.
pub(crate) fn parts_between(a: &TurnInput, b: &TurnInput, raw_mode: bool) -> Vec<&'static str> {
    PARTS
        .into_iter()
        .filter(|part| match *part {
            "agent" => a.agent != b.agent,
            "model" => model_of(a) != model_of(b),
            MODE_PART => mode_of(a, raw_mode) != mode_of(b, raw_mode),
            "cwd" => a.cwd != b.cwd,
            "mcp" => sorted(&a.mcp_servers) != sorted(&b.mcp_servers),
            "caps" => caps_key(a) != caps_key(b),
            "canvases" => a.canvases != b.canvases,
            "workspace" => a.workspace != b.workspace,
            _ => false,
        })
        .collect()
}

/// One turn's replayed plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replayed {
    pub plan: ResumePlan,
    /// For a config-changed reset, the [`PARTS`] HEAD names it by
    /// (`chat_resume::reset_parts`: the open stint's stamped parts against
    /// this turn's, [`UNKNOWN_PART`] when none differs); empty otherwise.
    pub parts: Vec<&'static str>,
    /// For a config-changed reset, the index of the turn the open stint was
    /// stamped with (what `parts` compares against).
    pub against: Option<usize>,
}

impl Replayed {
    pub(crate) fn is_config_reset(&self) -> bool {
        self.plan == ResumePlan::Reset(ResetReason::ConfigChanged)
    }
}

fn fresh_stint(agent: AgentKind) -> OpenStint {
    OpenStint {
        agent: agent.as_str().to_string(),
        harness_session_id: None,
        config_fingerprint: None,
        config_parts: None,
    }
}

/// One conversation's turns, oldest first, through HEAD's
/// `plan_resume_with` (see the module doc for the state it is given).
pub(crate) fn replay_session(turns: &[TurnInput]) -> Vec<Replayed> {
    let mut out = Vec::with_capacity(turns.len());
    let mut open: Option<OpenStint> = None;
    let mut last_closed: Option<ClosedStint> = None;
    // The turn whose input the open stint's native session was started with.
    let mut stamped: Option<usize> = None;
    let mut announced = 0usize;
    for (i, turn) in turns.iter().enumerate() {
        // `session_store::start_turn`: the first turn opens a stint, and a
        // turn of another agent closes the open one and opens a fresh one.
        match open.as_ref() {
            Some(stint) if stint.agent == turn.agent.as_str() => {}
            Some(stint) => {
                last_closed = Some(ClosedStint {
                    harness_session_id: stint.harness_session_id.clone(),
                    reason: Some(AGENT_CHANGED.to_string()),
                    config_parts: stint.config_parts.clone(),
                });
                open = Some(fresh_stint(turn.agent));
                stamped = None;
            }
            None => open = Some(fresh_stint(turn.agent)),
        }
        // The composer sends the native id the conversation last announced.
        // After an agent switch it sends none (`setChatAgentPersistent` in
        // `src/store/chatRuntime.ts` drops it: a native id isn't portable
        // across agents), which the fresh stint's empty id stands for here.
        let requested = open.as_ref().and_then(|s| s.harness_session_id.clone());
        let fingerprint = fingerprint_of(turn);
        let current_parts = parts_of(turn);
        let state = ResumeState {
            open: open.clone(),
            last_closed: last_closed.clone(),
        };
        let plan = plan_resume_with(requested.as_deref(), &state, &fingerprint, false);
        let mut parts = Vec::new();
        let mut against = None;
        if let ResumePlan::Reset(reason) = &plan {
            let reason = *reason;
            if reason == ResetReason::ConfigChanged {
                against = stamped;
                // As the chat path names it (OB03).
                parts = reset_parts(&state, &current_parts);
            }
            // `session_store::restart_stint`: the open stint closes with
            // its parts.
            last_closed = Some(ClosedStint {
                harness_session_id: requested.clone(),
                reason: Some(reason.as_str().to_string()),
                config_parts: open.as_ref().and_then(|s| s.config_parts.clone()),
            });
            open = Some(fresh_stint(turn.agent));
        }
        if !matches!(plan, ResumePlan::Resume(_)) {
            // A fresh native session announces its id, stored with this
            // turn's fingerprint and its parts (`record_harness_stamp`).
            announced += 1;
            if let Some(stint) = open.as_mut() {
                stint.harness_session_id = Some(format!("replay-{announced}"));
                stint.config_fingerprint = Some(fingerprint);
                stint.config_parts = Some(current_parts);
            }
            stamped = Some(i);
        }
        out.push(Replayed {
            plan,
            parts,
            against,
        });
    }
    out
}

/// One recorded turn: its rebuilt input and what was recorded of it.
#[derive(Debug, Clone)]
pub(crate) struct TurnRecord {
    pub input: TurnInput,
    /// Started inside the window: counted. Earlier turns only build state.
    pub counted: bool,
    /// The stints closed `config changed` at this turn.
    pub recorded_resets: usize,
    /// One of Hollow's wakes or check-ins (`main_agent::is_wake_message`).
    pub wake: bool,
    /// The [`PARTS`] whose value was filled in rather than recorded for
    /// this turn (see [`Assumed`]).
    pub assumed: Vec<&'static str>,
}

impl TurnRecord {
    /// Whether any of `parts` was filled in on this turn or on `other`.
    fn assumes_any(&self, parts: &[&'static str], other: Option<&TurnRecord>) -> bool {
        parts.iter().any(|part| {
            self.assumed.contains(part) || other.is_some_and(|o| o.assumed.contains(part))
        })
    }
}

/// One conversation's turns, oldest first.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionRecord {
    /// `sessions.agent` is Claude.
    pub claude: bool,
    /// Its folder is the Main agent's workspace.
    pub main: bool,
    pub turns: Vec<TurnRecord>,
}

/// Resets of one side (recorded or replayed) in a bucket.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Side {
    pub resets: usize,
    /// Resets with `mode` among their differing parts.
    pub flips: usize,
    pub per100: f64,
    pub flips_per100: f64,
    /// Resets whose turn ran in another permission mode than the turn
    /// before it, whatever part differed.
    pub mode_changed_resets: usize,
    /// Resets with a differing part filled in (not recorded) on either
    /// turn compared.
    pub assumed_part_resets: usize,
    /// Resets by their first differing part in [`PARTS`] order
    /// ([`UNKNOWN_PART`] for none).
    pub by_part: BTreeMap<&'static str, usize>,
}

impl Side {
    /// Count `n` resets at `turn`, differing from `other` in `parts` (never
    /// empty), `mode_changed` from the turn before.
    fn count(
        &mut self,
        n: usize,
        parts: &[&'static str],
        turn: &TurnRecord,
        other: Option<&TurnRecord>,
        mode_changed: bool,
    ) {
        self.resets += n;
        if parts.contains(&MODE_PART) {
            self.flips += n;
        }
        if mode_changed {
            self.mode_changed_resets += n;
        }
        if turn.assumes_any(parts, other) {
            self.assumed_part_resets += n;
        }
        let first = parts.first().copied().unwrap_or(UNKNOWN_PART);
        *self.by_part.entry(first).or_default() += n;
    }

    fn add(&mut self, other: &Side) {
        self.resets += other.resets;
        self.flips += other.flips;
        self.mode_changed_resets += other.mode_changed_resets;
        self.assumed_part_resets += other.assumed_part_resets;
        for (part, n) in &other.by_part {
            *self.by_part.entry(*part).or_default() += *n;
        }
    }

    fn rate(&mut self, turns: usize) {
        self.per100 = per100(self.resets, turns);
        self.flips_per100 = per100(self.flips, turns);
    }
}

/// One group of conversations: its counted turns and both sides' resets.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Bucket {
    /// Conversations with a counted turn.
    pub sessions: usize,
    pub turns: usize,
    pub recorded: Side,
    pub replayed: Side,
    /// Turns with a recorded reset that replay as no reset.
    pub recorded_only: usize,
    /// Turns that replay as a reset with none recorded.
    pub replayed_only: usize,
}

impl Bucket {
    fn add(&mut self, other: &Bucket) {
        self.sessions += other.sessions;
        self.turns += other.turns;
        self.recorded.add(&other.recorded);
        self.replayed.add(&other.replayed);
        self.recorded_only += other.recorded_only;
        self.replayed_only += other.replayed_only;
    }

    fn rate(&mut self) {
        self.recorded.rate(self.turns);
        self.replayed.rate(self.turns);
    }
}

/// The Main agent's counted wakes (every one of Hollow's wake and check-in
/// turns, `main_agent::is_wake_message`) and how many reset the session.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Wakes {
    pub turns: usize,
    pub recorded_resets: usize,
    pub replayed_resets: usize,
}

/// The bench's numbers: counts and rates only.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BenchReport {
    pub schema: u32,
    /// Every conversation.
    pub all: Bucket,
    /// Conversations whose `sessions.agent` is Claude.
    pub claude: Bucket,
    /// Conversations in the Main agent's workspace.
    pub main_agent: Bucket,
    pub main_wakes: Wakes,
    /// Recorded resets by every differing part (a reset counts once per
    /// part), over every conversation.
    pub recorded_by_part_all: BTreeMap<&'static str, usize>,
    /// Replayed resets by every differing part, the same way.
    pub replayed_by_part_all: BTreeMap<&'static str, usize>,
}

/// `part` per 100 of `whole`, to two decimals; 0 for none.
pub(crate) fn per100(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    (part as f64 * 10_000.0 / whole as f64).round() / 100.0
}

/// Replay every conversation and count both sides over its counted turns.
pub(crate) fn bench(sessions: &[SessionRecord]) -> BenchReport {
    let mut out = BenchReport {
        schema: SCHEMA,
        ..BenchReport::default()
    };
    for session in sessions {
        let inputs: Vec<TurnInput> = session.turns.iter().map(|t| t.input.clone()).collect();
        let replayed = replay_session(&inputs);
        let mut tally = Bucket::default();
        for (i, (turn, replay)) in session.turns.iter().zip(&replayed).enumerate() {
            if !turn.counted {
                continue;
            }
            tally.turns += 1;
            let previous = i.checked_sub(1).map(|j| &session.turns[j]);
            let mode_changed =
                previous.is_some_and(|p| mode_of(&p.input, true) != mode_of(&turn.input, true));
            let recorded = turn.recorded_resets;
            if recorded > 0 {
                let mut parts = previous
                    .map(|p| parts_between(&p.input, &turn.input, true))
                    .unwrap_or_default();
                if parts.is_empty() {
                    parts.push(UNKNOWN_PART);
                }
                let recorded_side = &mut tally.recorded;
                recorded_side.count(recorded, &parts, turn, previous, mode_changed);
                for part in &parts {
                    *out.recorded_by_part_all.entry(*part).or_default() += recorded;
                }
            }
            let reset = replay.is_config_reset();
            if reset {
                let against = replay.against.map(|j| &session.turns[j]);
                let replayed_side = &mut tally.replayed;
                replayed_side.count(1, &replay.parts, turn, against, mode_changed);
                for part in &replay.parts {
                    *out.replayed_by_part_all.entry(*part).or_default() += 1;
                }
            }
            tally.recorded_only += usize::from(recorded > 0 && !reset);
            tally.replayed_only += usize::from(reset && recorded == 0);
            if session.main && turn.wake {
                out.main_wakes.turns += 1;
                out.main_wakes.recorded_resets += usize::from(recorded > 0);
                out.main_wakes.replayed_resets += usize::from(reset);
            }
        }
        if tally.turns == 0 {
            continue;
        }
        tally.sessions = 1;
        out.all.add(&tally);
        if session.claude {
            out.claude.add(&tally);
        }
        if session.main {
            out.main_agent.add(&tally);
        }
    }
    for bucket in [&mut out.all, &mut out.claude, &mut out.main_agent] {
        bucket.rate();
    }
    out
}

// ── Loading the recorded turns ────────────────────────────────────────────

/// The window turns are counted in: started at or after `from`, before `to`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Window {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl Window {
    /// `HOLLOW_REPLAY_FROM` / `HOLLOW_REPLAY_TO` (RFC 3339), each defaulting
    /// to the baseline's ([`DEFAULT_FROM`], [`DEFAULT_TO`]).
    pub(crate) fn from_env() -> Result<Window, String> {
        let read = |key: &str, default: &str| {
            let text = env_nonempty(key).unwrap_or_else(|| default.to_string());
            parse_iso(text.trim()).ok_or_else(|| format!("{key} is not an RFC 3339 time"))
        };
        let window = Window {
            from: read("HOLLOW_REPLAY_FROM", DEFAULT_FROM)?,
            to: read("HOLLOW_REPLAY_TO", DEFAULT_TO)?,
        };
        if window.from >= window.to {
            return Err("the replay window is empty".into());
        }
        Ok(window)
    }

    fn holds(&self, at: Option<DateTime<Utc>>) -> bool {
        at.is_some_and(|at| at >= self.from && at < self.to)
    }
}

/// What the bench reads of one run's context manifest.
#[derive(Debug, Clone, Default)]
pub(crate) struct ManifestFacts {
    /// `capabilities.ownedServers`.
    pub servers: Vec<String>,
    /// The Canvases choice `compiler.visualGuide.delivery` implies.
    pub canvases: Option<CanvasFrequency>,
    /// `compiler.entry` and `compiler.session` in Hollow's vocabulary
    /// (`other` for anything else, `none` without a compiler record).
    pub entry: &'static str,
    pub session: &'static str,
    /// The compiler recorded a config-changed reset.
    pub config_reset: bool,
}

/// The Canvases choice a visual-guide delivery implies
/// (`visual_prompt::delivery`: always `prompt` at Most, `off` at Off, and
/// `turn`, `given` or `onDemand` only at Helps).
pub(crate) fn canvases_of(delivery: &str) -> Option<CanvasFrequency> {
    match delivery {
        "prompt" => Some(CanvasFrequency::Most),
        "off" => Some(CanvasFrequency::Off),
        "turn" | "given" | "onDemand" => Some(CanvasFrequency::Helps),
        _ => None,
    }
}

pub(crate) fn facts_of(manifest: &RunManifest) -> ManifestFacts {
    let compiler = manifest.compiler.as_ref();
    let entry = match compiler.map(|c| c.entry.as_str()) {
        Some("composer") => "composer",
        Some("appSession") => "appSession",
        Some(_) => "other",
        None => "none",
    };
    let session = match compiler.map(|c| c.session.as_str()) {
        Some("new") => "new",
        Some("resume") => "resume",
        Some("reset") => "reset",
        Some("retry") => "retry",
        Some(_) => "other",
        None => "none",
    };
    let config_reset = session == "reset"
        && compiler.and_then(|c| c.reset_reason.as_deref())
            == Some(ResetReason::ConfigChanged.wire());
    ManifestFacts {
        servers: manifest.capabilities.owned_servers.clone(),
        canvases: compiler
            .and_then(|c| c.visual_guide.as_ref())
            .and_then(|guide| canvases_of(&guide.delivery)),
        entry,
        session,
        config_reset,
    }
}

enum ManifestRead {
    Missing,
    Unreadable,
    Found(ManifestFacts),
}

/// Run `run_id`'s manifest in vault `source`, read in place: a regular
/// file (no link, the folder no link either) within `run_manifest`'s size
/// cap, of its schema and naming this run. The receipt's hash isn't
/// checked: the bench only counts.
fn read_manifest(source: &Path, run_id: &str) -> ManifestRead {
    let valid = !run_id.is_empty()
        && run_id.len() <= 128
        && !run_id.starts_with('.')
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        return ManifestRead::Missing;
    }
    let dir = source.join(crate::run_manifest::DIR_NAME);
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return ManifestRead::Missing;
    }
    let path = dir.join(format!("{run_id}.json"));
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return ManifestRead::Missing;
    };
    if !meta.is_file() || meta.len() > MANIFEST_MAX_BYTES {
        return ManifestRead::Unreadable;
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return ManifestRead::Unreadable;
    };
    match serde_json::from_slice::<RunManifest>(&bytes) {
        Ok(manifest)
            if manifest.schema_version == crate::run_manifest::SCHEMA_VERSION
                && manifest.run_id == run_id =>
        {
            ManifestRead::Found(facts_of(&manifest))
        }
        _ => ManifestRead::Unreadable,
    }
}

/// `chat::PromptKind`'s name for a conversation's folder: `bare` under a
/// pressure-test or bake-off scratch root, `main` and `quick` for the
/// built-in workspaces, `workspace` for any other workspace folder or a
/// build worktree, `vault` otherwise.
pub(crate) fn prompt_kind(cwd: &str) -> &'static str {
    let path = Path::new(cwd);
    let has = |name: &str| path.components().any(|c| c.as_os_str() == name);
    if BARE_ROOTS.iter().any(|root| has(root)) {
        return "bare";
    }
    match workspace_of(cwd) {
        Some(id) if crate::workspaces::is_main_agent(id) => "main",
        Some(id) if crate::workspaces::is_quick_chats(id) => "quick",
        Some(_) => "workspace",
        None if has("Worktrees") => "workspace",
        None => "vault",
    }
}

/// Turns whose input field was not recorded for them, by field (counted
/// turns only).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Assumed {
    /// The counted turns, for scale.
    pub turns: usize,
    /// No chat usage row: the model of the turn before.
    pub model: usize,
    /// No `permission_mode`: the mode of the turn before.
    pub perm: usize,
    /// No agent on the usage row: `sessions.agent`.
    pub agent_from_session: usize,
    /// No manifest: the servers of the conversation's nearest manifest.
    pub mcp_from_session: usize,
    /// No manifest in the conversation: the most common server set.
    pub mcp_default: usize,
    /// No visual-guide record: the Canvases of the nearest one.
    pub canvases_from_session: usize,
    /// None in the conversation: Canvases' default.
    pub canvases_default: usize,
    /// The folder is the conversation's current one (every turn).
    pub cwd_session_level: usize,
    /// The team server attached: every teammate CLI taken as installed.
    pub teammates_installed: usize,
}

/// The manifests of the counted turns.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestStats {
    /// Receipts (and so manifests) the vault keeps (`evidence::KEEP`).
    pub pool: usize,
    pub turns: usize,
    pub turns_with_manifest: usize,
    /// A manifest file that was there but refused (size, link, schema, run).
    pub unreadable: usize,
    /// With a manifest but no visual-guide record (before 2026-10-07).
    pub without_visual_guide: usize,
    pub by_entry: BTreeMap<&'static str, usize>,
    pub by_session: BTreeMap<&'static str, usize>,
    /// The compiler recorded a config-changed reset.
    pub config_changed_resets: usize,
}

/// Recorded config-changed closes the bench doesn't count.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Unmatched {
    /// Inside the window, before any turn of their conversation started.
    pub no_turn: usize,
    /// Inside the window, at a turn that started before it.
    pub pre_window_turn: usize,
    /// At a counted turn with no agent to replay it with (`skippedTurns`).
    pub skipped_turn: usize,
}

/// What [`load`] read.
#[derive(Debug, Default)]
pub(crate) struct Loaded {
    pub sessions: Vec<SessionRecord>,
    pub assumed: Assumed,
    pub manifests: ManifestStats,
    /// Counted turns with no agent to replay them with.
    pub skipped_turns: usize,
    pub unmatched: Unmatched,
}

/// One `turns` row as read, its request already reduced to flags.
struct RawTurn {
    started: Option<DateTime<Utc>>,
    perm: Option<String>,
    /// The chat usage row's requested model and agent.
    usage: Option<(Option<String>, Option<String>)>,
    manifest: Option<ManifestFacts>,
    manifest_unreadable: bool,
    wake: bool,
    unattended: bool,
    recorded_resets: usize,
}

struct RawSession {
    agent: String,
    cwd: String,
    turns: Vec<RawTurn>,
}

impl RawSession {
    fn is_plan(&self, i: usize) -> bool {
        self.turns[i].perm.as_deref().map(str::trim) == Some("plan")
    }

    /// Turn `i`'s plan-or-not and unattended-or-not kind, what its servers
    /// follow from.
    fn kind(&self, i: usize) -> (bool, bool) {
        (self.is_plan(i), self.turns[i].unattended)
    }
}

/// Whether a request is one Hollow queues as an unattended turn
/// (`ChatSendRequest::unattended`): a `wake_me` wake-up, the morning review
/// or a night-queue item (both removed 11-10, S6, but old transcripts carry them). Other
/// wakes and check-ins run attended (`main_agent::queue_wake`).
pub(crate) fn is_unattended_request(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with(crate::wake_me::WAKE_OPEN)
        || text.starts_with(NIGHT_PREFIX)
        || text
            .strip_prefix(WAKE_PREFIX)
            .is_some_and(|rest| rest.trim_start().starts_with(MORNING_REVIEW_OPEN))
}

/// The index nearest `at` other than it (the earlier one on a tie), below
/// `len`, for which `has` holds.
fn nearest(len: usize, at: usize, has: impl Fn(usize) -> bool) -> Option<usize> {
    (1..len).find_map(|d| {
        let before = at.checked_sub(d);
        let after = Some(at + d).filter(|j| *j < len);
        [before, after].into_iter().flatten().find(|j| has(*j))
    })
}

/// What a fallback server set is kept per: the agent, whether the turn ran
/// in plan mode and whether it ran unattended.
type ServerKind = (&'static str, bool, bool);

/// `servers` as a turn of this kind would have had them: no team server and
/// no services in plan mode (`mcp_attach::server_map_for`); none of those
/// and no computer control unattended (`mcp_attach::for_unattended`).
fn servers_for(servers: &[String], plan: bool, unattended: bool) -> Vec<String> {
    servers
        .iter()
        .filter(|s| {
            let team_or_services = *s == TEAM_SERVER || *s == crate::services_mcp::SERVER_NAME;
            let cut =
                ((plan || unattended) && team_or_services) || (unattended && *s == COMPUTER_SERVER);
            !cut
        })
        .cloned()
        .collect()
}

fn read_err(what: &str) -> impl Fn(rusqlite::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

/// Every conversation with a turn in `window`, every turn of it (earlier
/// ones build the replay's state), from Sessions DB `conn` and the
/// manifests of vault `source` (see the module doc).
pub(crate) fn load(conn: &Connection, source: &Path, window: &Window) -> Result<Loaded, String> {
    let sessions: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare("SELECT key, agent, cwd FROM sessions ORDER BY key")
            .map_err(read_err("sessions"))?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(read_err("sessions"))?;
        rows.collect::<Result<_, _>>()
            .map_err(read_err("sessions"))?
    };
    let mut turns_stmt = conn
        .prepare(
            "SELECT run_id, started_at, permission_mode, asked FROM turns \
             WHERE session_key = ?1 ORDER BY ord ASC",
        )
        .map_err(read_err("turns"))?;
    let mut usage_stmt = conn
        .prepare(
            "SELECT requested_model, agent FROM usage_ledger \
             WHERE run_id = ?1 AND surface = 'chat' ORDER BY id ASC LIMIT 1",
        )
        .map_err(read_err("usage"))?;
    let mut stints_stmt = conn
        .prepare(
            "SELECT ended_at FROM stints WHERE session_key = ?1 AND reason = ?2 \
             AND ended_at IS NOT NULL",
        )
        .map_err(read_err("stints"))?;
    let mut loaded = Loaded::default();
    let mut raw_sessions = Vec::new();
    for (key, agent, cwd) in sessions {
        type Row = (Option<String>, String, Option<String>, Option<String>);
        let rows: Vec<Row> = turns_stmt
            .query_map(params![key], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(read_err("turns"))?
            .collect::<Result<_, _>>()
            .map_err(read_err("turns"))?;
        let started: Vec<Option<DateTime<Utc>>> =
            rows.iter().map(|(_, at, _, _)| parse_iso(at)).collect();
        if !started.iter().any(|at| window.holds(*at)) {
            continue;
        }
        let mut turns = Vec::with_capacity(rows.len());
        for ((run_id, _, perm, asked), started) in rows.into_iter().zip(started) {
            let usage: Option<(Option<String>, Option<String>)> = match run_id.as_deref() {
                Some(run) => usage_stmt
                    .query_row(params![run], |row| Ok((row.get(0)?, row.get(1)?)))
                    .optional()
                    .map_err(read_err("usage"))?,
                None => None,
            };
            let read = match run_id.as_deref() {
                Some(run) => read_manifest(source, run),
                None => ManifestRead::Missing,
            };
            let (manifest, manifest_unreadable) = match read {
                ManifestRead::Found(facts) => (Some(facts), false),
                ManifestRead::Unreadable => (None, true),
                ManifestRead::Missing => (None, false),
            };
            // The request is reduced to these flags here and dropped.
            let asked = asked.as_deref().unwrap_or("");
            turns.push(RawTurn {
                started,
                perm,
                usage,
                manifest,
                manifest_unreadable,
                wake: is_wake_message(asked),
                unattended: is_unattended_request(asked),
                recorded_resets: 0,
            });
        }
        let closes: Vec<String> = stints_stmt
            .query_map(params![key, ResetReason::ConfigChanged.as_str()], |row| {
                row.get(0)
            })
            .map_err(read_err("stints"))?
            .collect::<Result<_, _>>()
            .map_err(read_err("stints"))?;
        for ended in closes {
            let ended = parse_iso(&ended);
            let at = ended.and_then(|ended| {
                turns
                    .iter()
                    .rposition(|t| t.started.is_some_and(|start| start <= ended))
            });
            // In the window but at no counted turn: unmatched (a close at a
            // skipped turn is counted when that turn is skipped).
            let inside = usize::from(window.holds(ended));
            match at {
                Some(i) => {
                    turns[i].recorded_resets += 1;
                    if !window.holds(turns[i].started) {
                        loaded.unmatched.pre_window_turn += inside;
                    }
                }
                None => loaded.unmatched.no_turn += inside,
            }
        }
        raw_sessions.push(RawSession { agent, cwd, turns });
    }

    // The fallback server set: the most common one per agent, plan and
    // unattended kind.
    let mut counts: BTreeMap<ServerKind, BTreeMap<Vec<String>, usize>> = BTreeMap::new();
    for session in &raw_sessions {
        let session_agent = AgentKind::from_opt(&session.agent);
        for (i, turn) in session.turns.iter().enumerate() {
            let (Some(facts), Some(agent)) =
                (turn.manifest.as_ref(), agent_of(turn, session_agent))
            else {
                continue;
            };
            let set: Vec<String> = sorted(&facts.servers)
                .into_iter()
                .map(str::to_string)
                .collect();
            let (plan, unattended) = session.kind(i);
            *counts
                .entry((agent.as_str(), plan, unattended))
                .or_default()
                .entry(set)
                .or_default() += 1;
        }
    }
    let defaults: BTreeMap<ServerKind, Vec<String>> = counts
        .into_iter()
        .filter_map(|(kind, sets)| {
            let set = sets.into_iter().max_by_key(|(_, n)| *n)?.0;
            Some((kind, set))
        })
        .collect();

    loaded.manifests.pool = crate::evidence::KEEP;
    for session in &raw_sessions {
        let record = build_session(session, window, &defaults, &mut loaded);
        loaded.sessions.push(record);
    }
    Ok(loaded)
}

/// A turn's agent: its usage row's, else the conversation's.
fn agent_of(turn: &RawTurn, session: Option<AgentKind>) -> Option<AgentKind> {
    turn.usage
        .as_ref()
        .and_then(|(_, agent)| agent.as_deref())
        .and_then(AgentKind::from_opt)
        .or(session)
}

/// One conversation's [`SessionRecord`], filling what a turn lacks (and
/// counting it in `loaded`'s `assumed` and `manifests`).
fn build_session(
    session: &RawSession,
    window: &Window,
    defaults: &BTreeMap<ServerKind, Vec<String>>,
    loaded: &mut Loaded,
) -> SessionRecord {
    let session_agent = AgentKind::from_opt(&session.agent);
    let kind = prompt_kind(&session.cwd);
    let cwd = PathBuf::from(&session.cwd);
    let len = session.turns.len();
    let mut record = SessionRecord {
        claude: session_agent == Some(AgentKind::Claude),
        main: workspace_of(&session.cwd).is_some_and(crate::workspaces::is_main_agent),
        turns: Vec::with_capacity(len),
    };
    let (mut model, mut perm): (Option<String>, Option<String>) = (None, None);
    for (i, turn) in session.turns.iter().enumerate() {
        let counted = window.holds(turn.started);
        let n = usize::from(counted);
        let Some(agent) = agent_of(turn, session_agent) else {
            loaded.skipped_turns += n;
            loaded.unmatched.skipped_turn += n * turn.recorded_resets;
            continue;
        };
        // The parts filled in for this turn (the cwd and the teammates are
        // assumed alike for every turn, so they never tell turns apart).
        let mut filled: Vec<&'static str> = Vec::new();
        let assumed = &mut loaded.assumed;
        assumed.turns += n;
        assumed.cwd_session_level += n;
        match &turn.usage {
            Some((requested, used)) => {
                model = requested.clone();
                if used.as_deref().and_then(AgentKind::from_opt).is_none() {
                    assumed.agent_from_session += n;
                    filled.push("agent");
                }
            }
            None => {
                assumed.model += n;
                assumed.agent_from_session += n;
                filled.extend(["model", "agent"]);
            }
        }
        match &turn.perm {
            Some(recorded) => perm = Some(recorded.clone()),
            None => {
                assumed.perm += n;
                filled.extend([MODE_PART, "caps"]);
            }
        }
        let turn_perm = perm.clone().unwrap_or_else(|| FALLBACK_PERM.to_string());
        let plan = turn_perm.trim() == "plan";
        let unattended = turn.unattended;
        let has_manifest = |j: usize| session.turns[j].manifest.is_some();
        let mcp_servers = match &turn.manifest {
            Some(facts) => facts.servers.clone(),
            None => {
                filled.extend(["mcp", "caps"]);
                // The nearest manifest of this turn's own kind first: plan,
                // unattended and wake alike, then plan and unattended alike.
                let same = |j: usize| has_manifest(j) && session.kind(j) == (plan, unattended);
                let found = nearest(len, i, |j| same(j) && session.turns[j].wake == turn.wake)
                    .or_else(|| nearest(len, i, same))
                    .or_else(|| nearest(len, i, has_manifest));
                match found.and_then(|j| session.turns[j].manifest.as_ref()) {
                    Some(facts) => {
                        assumed.mcp_from_session += n;
                        servers_for(&facts.servers, plan, unattended)
                    }
                    None => {
                        assumed.mcp_default += n;
                        // This kind's default, else an attended one's (cut
                        // to this kind), else none.
                        let name = agent.as_str();
                        let set = [
                            (name, plan, unattended),
                            (name, plan, false),
                            (name, false, false),
                        ]
                        .iter()
                        .find_map(|kind| defaults.get(kind));
                        set.map(|set| servers_for(set, plan, unattended))
                            .unwrap_or_default()
                    }
                }
            }
        };
        let canvases_at = |j: usize| {
            session.turns[j]
                .manifest
                .as_ref()
                .and_then(|facts| facts.canvases)
        };
        let canvases = match canvases_at(i) {
            Some(canvases) => canvases,
            None => {
                filled.push("canvases");
                match nearest(len, i, |j| canvases_at(j).is_some()).and_then(canvases_at) {
                    Some(canvases) => {
                        assumed.canvases_from_session += n;
                        canvases
                    }
                    None => {
                        assumed.canvases_default += n;
                        CanvasFrequency::default()
                    }
                }
            }
        };
        if mcp_servers.iter().any(|s| s == TEAM_SERVER) {
            assumed.teammates_installed += n;
        }
        let stats = &mut loaded.manifests;
        stats.turns += n;
        stats.unreadable += usize::from(counted && turn.manifest_unreadable);
        if let (true, Some(facts)) = (counted, turn.manifest.as_ref()) {
            stats.turns_with_manifest += 1;
            stats.without_visual_guide += usize::from(facts.canvases.is_none());
            *stats.by_entry.entry(facts.entry).or_default() += 1;
            *stats.by_session.entry(facts.session).or_default() += 1;
            stats.config_changed_resets += usize::from(facts.config_reset);
        }
        filled.sort_unstable();
        filled.dedup();
        record.turns.push(TurnRecord {
            input: TurnInput {
                agent,
                model: model.clone(),
                perm: turn_perm,
                cwd: cwd.clone(),
                mcp_servers,
                canvases,
                workspace: kind,
            },
            counted,
            recorded_resets: turn.recorded_resets,
            wake: turn.wake,
            assumed: filled,
        });
    }
    record
}

// ── Report ────────────────────────────────────────────────────────────────

/// The bench over `replay`'s snapshot and its source vault's manifests, as
/// one aggregates-only JSON object.
pub(crate) fn report(replay: &Replay, window: &Window) -> Result<Value, String> {
    let conn = replay.conn()?;
    let loaded = load(&conn, &replay.source, window)?;
    let mut out = serde_json::to_value(bench(&loaded.sessions)).map_err(|e| e.to_string())?;
    let at = |time: DateTime<Utc>| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    out["window"] = json!({ "from": at(window.from), "to": at(window.to) });
    out["snapshotQuickCheck"] = replay.quick_check.clone().into();
    out["assumed"] = serde_json::to_value(&loaded.assumed).map_err(|e| e.to_string())?;
    out["manifests"] = serde_json::to_value(&loaded.manifests).map_err(|e| e.to_string())?;
    out["skippedTurns"] = loaded.skipped_turns.into();
    out["unmatched"] = serde_json::to_value(&loaded.unmatched).map_err(|e| e.to_string())?;
    Ok(out)
}

/// The real run. Skips (passes) when either input variable is unset.
#[test]
#[ignore = "reads a real vault copy and Sessions DB: TMPDIR=<mktemp -d> HOLLOW_REPLAY_VAULT=<copy without restricted/> HOLLOW_REPLAY_DB=<db> cargo test --lib -- --ignored replay_bench"]
fn real_vault_replay_bench() {
    let (Some(vault), Some(db)) = (
        env_nonempty("HOLLOW_REPLAY_VAULT"),
        env_nonempty("HOLLOW_REPLAY_DB"),
    ) else {
        eprintln!("replay_bench: HOLLOW_REPLAY_VAULT or HOLLOW_REPLAY_DB is unset; skipped");
        return;
    };
    let vault = PathBuf::from(vault);
    assert!(
        !vault.join(RESTRICTED).exists(),
        "HOLLOW_REPLAY_VAULT has a restricted/ folder: run on a copy without it (rsync -a --exclude restricted/)"
    );
    let window = Window::from_env().expect("the replay window");
    let replay = Replay::setup(&vault, Path::new(&db)).expect("replay setup");
    let made = [replay.vault.clone(), replay.db.clone()];
    let result = report(&replay, &window);
    drop(replay);
    let deleted = made.iter().all(|path| !path.exists());
    eprintln!("replay_bench: snapshot and copied docs deleted: {deleted}");
    let mut json = result.expect("replay bench");
    json["snapshotDeleted"] = deleted.into();
    let text = serde_json::to_string_pretty(&json).unwrap();
    match env_nonempty("HOLLOW_REPLAY_BENCH_OUT") {
        Some(out) => std::fs::write(out, text).expect("write the replay bench"),
        None => println!("{text}"),
    }
    assert!(deleted, "the snapshot or the copied docs were not deleted");
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "/v/workspaces/main";

    /// A Claude turn in the Main agent's folder, Sonnet, with the servers a
    /// non-plan chat attaches.
    fn turn(perm: &str) -> TurnInput {
        TurnInput {
            agent: AgentKind::Claude,
            model: Some("sonnet".into()),
            perm: perm.into(),
            cwd: PathBuf::from(MAIN),
            mcp_servers: vec![
                "hollow_questions".into(),
                "hollow_vault".into(),
                "hollow_tasks".into(),
                TEAM_SERVER.into(),
            ],
            canvases: CanvasFrequency::Helps,
            workspace: "main",
        }
    }

    fn record(input: TurnInput, recorded_resets: usize) -> TurnRecord {
        TurnRecord {
            input,
            counted: true,
            recorded_resets,
            wake: false,
            assumed: Vec::new(),
        }
    }

    /// (index, parts) of each config-changed reset.
    fn resets(replayed: &[Replayed]) -> Vec<(usize, Vec<&'static str>)> {
        replayed
            .iter()
            .enumerate()
            .filter(|(_, r)| r.is_config_reset())
            .map(|(i, r)| (i, r.parts.clone()))
            .collect()
    }

    fn resume(n: usize) -> ResumePlan {
        ResumePlan::Resume(format!("replay-{n}"))
    }

    /// A by-part map from `(part, count)` pairs.
    fn counts(pairs: &[(&'static str, usize)]) -> BTreeMap<&'static str, usize> {
        pairs.iter().copied().collect()
    }

    /// Red-first (HB01): a Main-agent wake in a lower mode (then capped at
    /// acceptEdits) alternating with the person's bypass turns reset the
    /// session each time while the mode itself was hashed; at HEAD the four
    /// turns share one session.
    #[test]
    fn act_mode_flips_keep_one_session() {
        let turns: Vec<TurnInput> = [
            "acceptEdits",
            "bypassPermissions",
            "acceptEdits",
            "bypassPermissions",
        ]
        .into_iter()
        .map(turn)
        .collect();
        let replayed = replay_session(&turns);
        let plans: Vec<ResumePlan> = replayed.iter().map(|r| r.plan.clone()).collect();
        assert_eq!(
            plans,
            vec![ResumePlan::New, resume(1), resume(1), resume(1)]
        );
        assert!(resets(&replayed).is_empty());
        // The recorded side names such a reset by the mode itself.
        assert_eq!(parts_between(&turns[0], &turns[1], true), vec!["mode"]);
        assert!(parts_between(&turns[0], &turns[1], false).is_empty());
    }

    /// Red-first: a model change is one reset, named `model` by OB03's
    /// `changed_parts` (the stamp's parts against the turn's); the next
    /// turn on the new model resumes the fresh session.
    #[test]
    fn a_model_change_is_one_reset_named_model() {
        let mut opus = turn("acceptEdits");
        opus.model = Some("opus".into());
        let turns = vec![turn("acceptEdits"), opus.clone(), opus];
        let stamped = parts_of(&turns[0]);
        assert_eq!(
            crate::chat_resume::changed_parts(Some(&stamped), &parts_of(&turns[1])),
            vec!["model"]
        );
        let replayed = replay_session(&turns);
        assert_eq!(resets(&replayed), vec![(1, vec!["model"])]);
        assert_eq!(replayed[1].against, Some(0));
        assert_eq!(replayed[2].plan, resume(2));
        // A model only spelled with spaces around it is the same model.
        let mut spaced = turn("acceptEdits");
        spaced.model = Some(" sonnet ".into());
        assert!(resets(&replay_session(&[turn("acceptEdits"), spaced])).is_empty());
    }

    /// A flip into plan (a read-only session, without the team server) is a
    /// reset named `mode` first, and so is the flip back.
    #[test]
    fn a_flip_into_plan_resets_named_mode() {
        let mut plan = turn("plan");
        plan.mcp_servers.retain(|s| s != TEAM_SERVER);
        let turns = vec![turn("acceptEdits"), plan, turn("bypassPermissions")];
        let replayed = replay_session(&turns);
        let found = resets(&replayed);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].0, 1);
        assert_eq!(found[0].1.first(), Some(&"mode"));
        assert!(found[0].1.contains(&"mcp") && found[0].1.contains(&"caps"));
        assert_eq!(found[1].0, 2);
        assert_eq!(found[1].1.first(), Some(&"mode"));
    }

    /// Each input the bench rebuilds names its own OB03 part on the
    /// recorded side, so both sides share the app's vocabulary; `contract`
    /// and `skills` are never rebuilt, so never named.
    #[test]
    fn the_recorded_side_names_resets_in_ob03s_parts() {
        let base = turn("acceptEdits");
        let changed = |edit: fn(&mut TurnInput)| {
            let mut other = base.clone();
            edit(&mut other);
            parts_between(&base, &other, true)
        };
        assert_eq!(changed(|t| t.model = Some("opus".into())), vec!["model"]);
        assert_eq!(
            changed(|t| t.perm = "bypassPermissions".into()),
            vec!["mode"]
        );
        assert_eq!(changed(|t| t.cwd = PathBuf::from("/v/x")), vec!["cwd"]);
        assert_eq!(
            changed(|t| t.canvases = CanvasFrequency::Off),
            vec!["canvases"]
        );
        assert_eq!(changed(|t| t.workspace = "quick"), vec!["workspace"]);
        assert_eq!(
            changed(|t| t.mcp_servers.retain(|s| s != "hollow_vault")),
            vec!["mcp", "caps"]
        );
        let agent = changed(|t| t.agent = AgentKind::Codex);
        assert_eq!(agent.first(), Some(&"agent"));
        assert!(PARTS.contains(&MODE_PART));
        assert_eq!(UNKNOWN_PART, "unknown");
    }

    /// A recorded reset at a turn whose rebuilt inputs match the turn
    /// before (a stint stamped before OB03 holds no parts to say what
    /// changed: v2's prompt bytes, say) names `unknown`, never a guess; at
    /// HEAD the same turns resume. HEAD names a stint stamped without
    /// parts `unknown` too.
    #[test]
    fn a_stint_without_parts_names_unknown() {
        let session = SessionRecord {
            claude: true,
            main: false,
            turns: vec![
                record(turn("acceptEdits"), 0),
                record(turn("acceptEdits"), 1),
            ],
        };
        let out = bench(&[session]);
        assert_eq!(out.all.recorded.resets, 1);
        assert_eq!(out.all.recorded.by_part, counts(&[(UNKNOWN_PART, 1)]));
        assert_eq!(out.recorded_by_part_all, counts(&[("unknown", 1)]));
        assert_eq!(out.all.replayed.resets, 0);
        let mut opus = turn("acceptEdits");
        opus.model = Some("opus".into());
        let legacy = ResumeState {
            open: Some(OpenStint {
                agent: "claude".into(),
                harness_session_id: Some("replay-1".into()),
                config_fingerprint: Some(fingerprint_of(&turn("acceptEdits"))),
                config_parts: None,
            }),
            last_closed: None,
        };
        assert_eq!(reset_parts(&legacy, &parts_of(&opus)), vec![UNKNOWN_PART]);
    }

    /// A conversation in another folder is a reset named `cwd`.
    #[test]
    fn a_cwd_change_resets_named_cwd() {
        let mut moved = turn("acceptEdits");
        moved.cwd = PathBuf::from("/v/workspaces/elsewhere");
        let replayed = replay_session(&[turn("acceptEdits"), moved]);
        assert_eq!(resets(&replayed), vec![(1, vec!["cwd"])]);
    }

    /// A turn of another agent starts that agent's own session (`agent
    /// changed`), never a config-changed reset; switching back does too.
    /// The composer drops the native id on a switch (`setChatAgentPersistent`
    /// in `src/store/chatRuntime.ts`), so such a turn requests none and is
    /// new by construction. What can go wrong is the stamp: the new agent's
    /// session must be compared with its own first turn, never the other
    /// agent's, and the turn after a switch back must resume the session the
    /// switch announced.
    #[test]
    fn an_agent_switch_starts_new_without_a_config_reset() {
        let mut codex = turn("acceptEdits");
        codex.agent = AgentKind::Codex;
        codex.model = Some("gpt".into());
        let mut codex_mini = codex.clone();
        codex_mini.model = Some("gpt-mini".into());
        let turns = vec![
            turn("acceptEdits"),
            codex.clone(),
            codex,
            codex_mini,
            turn("acceptEdits"),
            turn("acceptEdits"),
        ];
        let replayed = replay_session(&turns);
        let plans: Vec<ResumePlan> = replayed.iter().map(|r| r.plan.clone()).collect();
        assert_eq!(
            plans,
            vec![
                ResumePlan::New,
                ResumePlan::New,
                resume(2),
                ResumePlan::Reset(ResetReason::ConfigChanged),
                ResumePlan::New,
                resume(4),
            ]
        );
        // Against the Codex session's own stamp (turn 1): the model alone,
        // never `agent`.
        assert_eq!(resets(&replayed), vec![(3, vec!["model"])]);
        assert_eq!(replayed[3].against, Some(1));
    }

    /// Capabilities follow the servers and the mode as the attach rules
    /// set them; a bare turn has only its agent's.
    #[test]
    fn caps_follow_the_servers_and_the_mode() {
        let bits = |t: &TurnInput| caps_key(t);
        let act = turn("acceptEdits");
        assert_eq!(
            bits(&act),
            vec![
                "contract:claude",
                "questions",
                "tasks",
                "teamAntigravity",
                "teamCodex",
                "teamGrok",
                "vault"
            ]
        );
        let mut plan = turn("plan");
        plan.mcp_servers.retain(|s| s != TEAM_SERVER);
        assert!(bits(&plan).contains(&"tasksReadOnly".to_string()));
        assert!(!bits(&plan).iter().any(|b| b.starts_with("team")));
        let mut bare = turn("acceptEdits");
        bare.workspace = "bare";
        assert_eq!(bits(&bare), vec!["contract:claude"]);
        let mut agy = turn("acceptEdits");
        // Antigravity chats attach owned servers over ACP; the fenced block is
        // Grok's now.
        agy.agent = AgentKind::Grok;
        agy.mcp_servers.clear();
        assert!(bits(&agy).contains(&"questionsFenced".to_string()));
    }

    /// The baseline's shape: a Main-agent conversation whose recorded
    /// resets were three mode flips and a model change replays with the
    /// model change alone; a turn before the window builds state but isn't
    /// counted; wakes are counted apart.
    #[test]
    fn bench_counts_recorded_and_replayed_by_bucket() {
        let mut before = record(turn("acceptEdits"), 0);
        before.counted = false;
        let mut wake = record(turn("acceptEdits"), 1);
        wake.wake = true;
        let mut opus = turn("bypassPermissions");
        opus.model = Some("opus".into());
        let main = SessionRecord {
            claude: true,
            main: true,
            turns: vec![
                before,
                record(turn("bypassPermissions"), 1),
                wake,
                record(turn("bypassPermissions"), 1),
                record(opus, 1),
            ],
        };
        let mut codex_turn = turn("acceptEdits");
        codex_turn.agent = AgentKind::Codex;
        codex_turn.cwd = PathBuf::from("/v/workspaces/alpha");
        codex_turn.workspace = "workspace";
        let codex = SessionRecord {
            claude: false,
            main: false,
            turns: vec![record(codex_turn.clone(), 0), record(codex_turn, 0)],
        };
        let out = bench(&[main, codex]);
        assert_eq!(out.schema, SCHEMA);
        let main = &out.main_agent;
        assert_eq!((main.sessions, main.turns), (1, 4));
        assert_eq!(main.recorded.resets, 4);
        // The last recorded reset changed the model, not the mode.
        assert_eq!(main.recorded.flips, 3);
        assert_eq!(main.recorded.per100, 100.0);
        assert_eq!(main.recorded.flips_per100, 75.0);
        assert_eq!(main.recorded.by_part.get("mode"), Some(&3));
        assert_eq!(main.recorded.by_part.get("model"), Some(&1));
        assert_eq!(main.recorded.by_part.get(UNKNOWN_PART), None);
        assert_eq!(main.recorded.mode_changed_resets, 3);
        assert_eq!(main.recorded.assumed_part_resets, 0);
        assert_eq!(main.replayed.resets, 1);
        assert_eq!(main.replayed.by_part.get("model"), Some(&1));
        assert_eq!(main.replayed.flips, 0);
        assert_eq!(main.replayed.per100, 25.0);
        assert_eq!((main.recorded_only, main.replayed_only), (3, 0));
        assert_eq!(out.claude, out.main_agent);
        assert_eq!((out.all.sessions, out.all.turns), (2, 6));
        assert_eq!(out.all.replayed.per100, 16.67);
        assert_eq!(
            out.main_wakes,
            Wakes {
                turns: 1,
                recorded_resets: 1,
                replayed_resets: 0
            }
        );
        assert_eq!(
            out.recorded_by_part_all,
            counts(&[("mode", 3), ("model", 1)])
        );
        assert_eq!(out.replayed_by_part_all, counts(&[("model", 1)]));
    }

    /// A flip is a reset with `mode` among its differing parts, on each
    /// side by its own comparison: a model change at a turn that also moved
    /// between act modes is a recorded flip (the mode itself differs) but
    /// not a replayed one (its class doesn't), though its turn changed mode;
    /// a flip into plan is a flip on both sides. Every differing part is
    /// counted apart from the first.
    #[test]
    fn a_flip_counts_only_when_mode_is_a_differing_part() {
        let mut opus = turn("bypassPermissions");
        opus.model = Some("opus".into());
        let mut plan = turn("plan");
        plan.model = Some("opus".into());
        plan.mcp_servers.retain(|s| s != TEAM_SERVER);
        let session = SessionRecord {
            claude: true,
            main: false,
            turns: vec![
                record(turn("acceptEdits"), 0),
                record(opus, 1),
                record(plan, 1),
            ],
        };
        let out = bench(&[session]);
        let (recorded, replayed) = (&out.all.recorded, &out.all.replayed);
        assert_eq!((recorded.resets, replayed.resets), (2, 2));
        assert_eq!((recorded.flips, replayed.flips), (2, 1));
        assert_eq!(
            (recorded.mode_changed_resets, replayed.mode_changed_resets),
            (2, 2)
        );
        assert_eq!(recorded.flips_per100, 66.67);
        assert_eq!(replayed.flips_per100, 33.33);
        // `model` comes before `mode` in PARTS: the first part is the model.
        assert_eq!(recorded.by_part.get("model"), Some(&1));
        assert_eq!(recorded.by_part.get("mode"), Some(&1));
        assert_eq!(
            out.recorded_by_part_all,
            counts(&[("caps", 1), ("mcp", 1), ("mode", 2), ("model", 1)])
        );
        assert_eq!(
            out.replayed_by_part_all,
            counts(&[("caps", 1), ("mcp", 1), ("mode", 1), ("model", 1)])
        );
    }

    fn leaves_are_numbers(value: &Value) -> bool {
        match value {
            Value::Number(_) | Value::Bool(_) => true,
            Value::Object(map) => map.values().all(leaves_are_numbers),
            Value::Array(items) => items.iter().all(leaves_are_numbers),
            Value::Null | Value::String(_) => false,
        }
    }

    /// The report is numbers only: no key, folder, model or mode reaches it,
    /// and its part keys are the bench's own vocabulary.
    #[test]
    fn the_report_serialises_numbers_only() {
        let mut secret = turn("acceptEdits");
        secret.cwd = PathBuf::from("/Users/someone/private-folder");
        secret.model = Some("secret-model".into());
        let mut changed = secret.clone();
        changed.model = Some("other-secret-model".into());
        let session = SessionRecord {
            claude: true,
            main: false,
            turns: vec![record(secret, 0), record(changed, 1)],
        };
        let out = bench(&[session]);
        let value = serde_json::to_value(&out).unwrap();
        assert!(leaves_are_numbers(&value), "{value}");
        for stats in [
            serde_json::to_value(Assumed::default()).unwrap(),
            serde_json::to_value(ManifestStats::default()).unwrap(),
        ] {
            assert!(leaves_are_numbers(&stats), "{stats}");
        }
        let text = value.to_string();
        for leak in ["someone", "private", "secret", "acceptEdits", "sonnet"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        for map in [&out.all.replayed.by_part, &out.all.recorded.by_part] {
            assert!(map.keys().all(|k| PARTS.contains(k) || *k == UNKNOWN_PART));
        }
    }

    /// The dotted paths of `value`'s string leaves.
    fn string_paths(value: &Value, at: &str, out: &mut Vec<String>) {
        let join = |key: &str| {
            if at.is_empty() {
                key.to_string()
            } else {
                format!("{at}.{key}")
            }
        };
        match value {
            Value::String(_) => out.push(at.to_string()),
            Value::Object(map) => {
                for (key, child) in map {
                    string_paths(child, &join(key), out);
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    string_paths(child, &join(&i.to_string()), out);
                }
            }
            _ => {}
        }
    }

    /// The whole report over a snapshot of a store the session store wrote
    /// (`Replay::setup`): only the window and the snapshot's `quick_check`
    /// are strings, and no key, model or request reaches it.
    #[test]
    fn the_report_holds_strings_only_in_window_and_quick_check() {
        let dir = store();
        let v = dir.path();
        let key = "s-s07-report";
        let (one, two, act) = ("First request here.", "Second request here.", "acceptEdits");
        store_turn(v, key, MAIN, "run-s07-r1", one, act, Some("sonnet"));
        store_turn(v, key, MAIN, "run-s07-r2", two, act, Some("opus"));
        crate::session_store::restart_stint(v, key, "claude", ResetReason::ConfigChanged.as_str())
            .unwrap();
        let db = crate::session_store::db_path_for(v).unwrap();
        let replay = Replay::setup(v, &db).unwrap();
        let value = report(&replay, &always()).unwrap();
        drop(replay);
        let mut found = Vec::new();
        string_paths(&value, "", &mut found);
        found.sort();
        assert_eq!(
            found,
            vec!["snapshotQuickCheck", "window.from", "window.to"]
        );
        assert_eq!(value["snapshotQuickCheck"], "ok");
        assert_eq!(value["all"]["recorded"]["resets"], 1);
        assert_eq!(value["all"]["replayed"]["resets"], 1);
        for field in [
            "assumed",
            "manifests",
            "unmatched",
            "skippedTurns",
            "mainWakes",
        ] {
            assert!(value.get(field).is_some(), "{field} missing");
        }
        let text = value.to_string();
        for leak in [key, "sonnet", "opus", "request here", "acceptEdits"] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        assert!(!text.contains(v.to_str().unwrap()));
    }

    /// A delivery names the Canvases choice that produced it.
    #[test]
    fn canvases_follow_the_visual_guide_delivery() {
        use crate::visual_prompt::{delivery, Delivery};
        for frequency in CanvasFrequency::ALL {
            for (cue, file, fresh, given) in [
                (Some("chart"), false, true, None),
                (None, true, false, None),
                (None, false, false, Some("sha")),
            ] {
                let d: Delivery = delivery(frequency, cue, file, fresh, given, "sha");
                assert_eq!(canvases_of(d.as_str()), Some(frequency), "{d:?}");
            }
        }
        assert_eq!(canvases_of("elsewhere"), None);
    }

    #[test]
    fn prompt_kind_reads_the_folder() {
        assert_eq!(prompt_kind("/v/workspaces/main"), "main");
        assert_eq!(prompt_kind("/v/workspaces/quick-chats/"), "quick");
        assert_eq!(prompt_kind("/v/workspaces/alpha"), "workspace");
        assert_eq!(
            prompt_kind("/Users/x/Hollow/Worktrees/app/lane-1"),
            "workspace"
        );
        assert_eq!(
            prompt_kind("/Users/x/Library/Application Support/Hollow/PressureTests/v-1/p"),
            "bare"
        );
        assert_eq!(prompt_kind("/v"), "vault");
    }

    /// The requests Hollow queues unattended: a `wake_me` wake-up, the
    /// morning review and a night-queue item; never a wake Hollow queues
    /// attended (a finished chat, a check-in) or the person's own words.
    #[test]
    fn unattended_requests_are_wake_ups_the_morning_review_and_night_items() {
        let wake_up = format!("{} (the build). Continue.", crate::wake_me::WAKE_OPEN);
        let review = format!("{WAKE_PREFIX} {MORNING_REVIEW_OPEN} (night of 10-16).");
        let night = format!("{NIGHT_PREFIX} item 3 added by you at 22:00.");
        for text in [&wake_up, &review, &night] {
            assert!(is_unattended_request(text), "{text}");
        }
        for text in [
            crate::main_agent::WAKE_MESSAGE,
            "⟲ Hollow: check-in at 10:00. Nothing new came up.",
            "Fix the build.",
        ] {
            assert!(!is_unattended_request(text), "{text}");
        }
        // The wake-up and the review are wakes; a night item isn't.
        assert!(is_wake_message(&wake_up) && is_wake_message(&review));
        assert!(!is_wake_message(&night));
    }

    /// A fresh temp vault for a session store the test writes.
    fn store() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("vault-s07-{}-", std::process::id()))
            .tempdir()
            .unwrap()
    }

    /// A Claude turn `run` of conversation `key` in folder `cwd` asking
    /// `ask` in mode `perm`, with a chat usage row for `model` when given;
    /// as the session store records it.
    fn store_turn(
        v: &Path,
        key: &str,
        cwd: &str,
        run: &str,
        ask: &str,
        perm: &str,
        model: Option<&str>,
    ) {
        use crate::session_store::{record_turn_permission, start_turn};
        let ord = start_turn(v, key, "claude", cwd, run, ask).unwrap();
        record_turn_permission(v, key, ord, perm).unwrap();
        if let Some(model) = model {
            let db = crate::session_store::db_path_for(v).unwrap();
            let conn = Connection::open(&db).unwrap();
            conn.execute(
                "INSERT INTO usage_ledger(run_key, at, started_at, surface, run_id, agent, \
                 requested_model, billing, outcome, spawned, basis, cost_status) \
                 VALUES(?1, ?2, ?2, 'chat', ?1, 'claude', ?3, 'subscription', 'success', 1, \
                 'reported', 'known')",
                params![run, Utc::now().to_rfc3339(), model],
            )
            .unwrap();
        }
    }

    /// Run `run`'s context manifest in vault `v`, attaching `servers`.
    fn write_manifest(
        v: &Path,
        run: &str,
        servers: &[&str],
        compiler: Option<crate::run_manifest::CompilerRecord>,
    ) {
        let manifest = RunManifest {
            schema_version: crate::run_manifest::SCHEMA_VERSION,
            run_id: run.into(),
            capabilities: crate::run_manifest::Capabilities {
                owned_servers: servers.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            compiler,
            ..RunManifest::default()
        };
        let manifests = v.join(crate::run_manifest::DIR_NAME);
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(
            manifests.join(format!("{run}.json")),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    /// A window every test turn falls in.
    fn always() -> Window {
        Window {
            from: parse_iso("2000-01-01T00:00:00Z").unwrap(),
            to: parse_iso("2100-01-01T00:00:00Z").unwrap(),
        }
    }

    /// The loader over a store the real session store wrote: the recorded
    /// mode and model per turn (a turn without a usage row takes the one
    /// before), a run's manifest servers and Canvases (and the nearest one's
    /// for runs without), a refused manifest, a config-changed stint close
    /// at the turn it followed, and wakes, a check-in and a finished chat's
    /// alike; nothing of it in the output.
    #[test]
    fn load_reads_recorded_fields_manifests_and_stints() {
        use crate::run_manifest::{CompilerRecord, VisualGuideRecord};
        let dir = store();
        let v = dir.path();
        let db = crate::session_store::db_path_for(v).unwrap();
        let key = "s-s07-load";
        let asks = [
            "⟲ Hollow: check-in at 10:00. Nothing new came up.",
            crate::main_agent::WAKE_MESSAGE,
            "Third request here.",
        ];
        let perms = ["acceptEdits", "bypassPermissions", "bypassPermissions"];
        let models = [Some("sonnet"), None, Some("opus")];
        for (i, ask) in asks.iter().enumerate() {
            let run = format!("run-s07-{}", i + 1);
            store_turn(v, key, MAIN, &run, ask, perms[i], models[i]);
        }
        // The third turn's plan reset the session.
        crate::session_store::restart_stint(v, key, "claude", ResetReason::ConfigChanged.as_str())
            .unwrap();
        let compiler = CompilerRecord {
            entry: "composer".into(),
            session: "reset".into(),
            reset_reason: Some(ResetReason::ConfigChanged.wire().into()),
            visual_guide: Some(VisualGuideRecord {
                delivery: "off".into(),
                ..VisualGuideRecord::default()
            }),
            ..CompilerRecord::default()
        };
        write_manifest(v, "run-s07-3", &["hollow_vault"], Some(compiler));
        let manifests = v.join(crate::run_manifest::DIR_NAME);
        std::fs::write(manifests.join("run-s07-2.json"), b"{not a manifest").unwrap();

        let conn =
            Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let loaded = load(&conn, v, &always()).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        let session = &loaded.sessions[0];
        assert!(session.claude && session.main);
        let turns = &session.turns;
        assert_eq!(turns.len(), 3);
        assert!(turns.iter().all(|t| t.counted));
        // A finished chat's wake is a wake too, not only a check-in.
        let flags: Vec<(bool, usize)> = turns.iter().map(|t| (t.wake, t.recorded_resets)).collect();
        assert_eq!(flags, vec![(true, 0), (true, 0), (false, 1)]);
        // The second turn's model, agent, servers and Canvases were filled
        // in; the third's were all recorded.
        assert_eq!(
            turns[1].assumed,
            vec!["agent", "canvases", "caps", "mcp", "model"]
        );
        assert!(turns[2].assumed.is_empty());
        let models: Vec<Option<&str>> = turns.iter().map(|t| t.input.model.as_deref()).collect();
        assert_eq!(models, vec![Some("sonnet"), Some("sonnet"), Some("opus")]);
        assert_eq!(turns[1].input.perm, "bypassPermissions");
        for t in turns {
            assert_eq!(t.input.mcp_servers, vec!["hollow_vault".to_string()]);
            assert_eq!(t.input.canvases, CanvasFrequency::Off);
            assert_eq!(t.input.workspace, "main");
            assert_eq!(t.input.agent, AgentKind::Claude);
        }
        let assumed = &loaded.assumed;
        assert_eq!(assumed.turns, 3);
        assert_eq!((assumed.model, assumed.perm), (1, 0));
        assert_eq!((assumed.mcp_from_session, assumed.mcp_default), (2, 0));
        assert_eq!(
            (assumed.canvases_from_session, assumed.canvases_default),
            (2, 0)
        );
        let stats = &loaded.manifests;
        assert_eq!(stats.pool, crate::evidence::KEEP);
        assert_eq!((stats.turns, stats.turns_with_manifest), (3, 1));
        assert_eq!(stats.unreadable, 1);
        assert_eq!(stats.config_changed_resets, 1);
        assert_eq!(stats.by_session.get("reset"), Some(&1));
        assert_eq!(stats.by_entry.get("composer"), Some(&1));
        assert_eq!(loaded.unmatched, Unmatched::default());

        let out = bench(&loaded.sessions);
        let main = &out.main_agent;
        assert_eq!((main.recorded.resets, main.replayed.resets), (1, 1));
        assert_eq!(main.recorded.by_part.get("model"), Some(&1));
        assert_eq!(main.replayed.by_part.get("model"), Some(&1));
        assert_eq!((main.recorded.flips, main.replayed.flips), (0, 0));
        // The recorded side compares with the turn before, whose model was
        // filled in; the replayed side with the first turn's, recorded.
        assert_eq!(
            (
                main.recorded.assumed_part_resets,
                main.replayed.assumed_part_resets
            ),
            (1, 0)
        );
        assert_eq!(out.main_wakes.turns, 2);

        let text = [
            serde_json::to_string(&out).unwrap(),
            serde_json::to_string(&loaded.assumed).unwrap(),
            serde_json::to_string(&loaded.manifests).unwrap(),
        ]
        .concat();
        for leak in [
            key,
            "sonnet",
            "opus",
            "/v/",
            "request here",
            "check-in at",
            "chat you started",
        ] {
            assert!(!text.contains(leak), "{leak} in {text}");
        }
        // A window the turns fall outside loads nothing.
        let past = Window {
            from: parse_iso("2000-01-01T00:00:00Z").unwrap(),
            to: parse_iso("2000-01-02T00:00:00Z").unwrap(),
        };
        assert!(load(&conn, v, &past).unwrap().sessions.is_empty());
    }

    /// A turn without a manifest borrows the servers of the nearest turn of
    /// its own kind, an unattended wake-up's from another wake-up and the
    /// person's from the person's, even past a nearer turn of the other
    /// kind; resets whose differing part was borrowed on either side are
    /// counted apart.
    #[test]
    fn a_turn_without_a_manifest_borrows_servers_of_its_own_kind() {
        let dir = store();
        let v = dir.path();
        let key = "s-s07-kinds";
        let attended = [
            "hollow_questions",
            "hollow_vault",
            "hollow_tasks",
            TEAM_SERVER,
            COMPUTER_SERVER,
        ];
        let unattended = ["hollow_questions", "hollow_vault"];
        let wake_up = format!(
            "{} (the build). Continue where you left off.",
            crate::wake_me::WAKE_OPEN
        );
        let asks = [
            "Look at the plan.",
            wake_up.as_str(),
            "Carry on.",
            wake_up.as_str(),
            "And the rest.",
        ];
        for (i, ask) in asks.iter().enumerate() {
            let run = format!("run-s07-k{}", i + 1);
            store_turn(v, key, MAIN, &run, ask, "acceptEdits", None);
        }
        write_manifest(v, "run-s07-k1", &attended, None);
        write_manifest(v, "run-s07-k4", &unattended, None);
        write_manifest(v, "run-s07-k5", &attended, None);
        let db = crate::session_store::db_path_for(v).unwrap();
        let conn =
            Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let loaded = load(&conn, v, &always()).unwrap();
        let turns = &loaded.sessions[0].turns;
        let servers: Vec<Vec<&str>> = turns
            .iter()
            .map(|t| t.input.mcp_servers.iter().map(String::as_str).collect())
            .collect();
        assert_eq!(
            servers,
            vec![
                attended.to_vec(),
                unattended.to_vec(),
                attended.to_vec(),
                unattended.to_vec(),
                attended.to_vec(),
            ]
        );
        assert_eq!(
            (loaded.assumed.mcp_from_session, loaded.assumed.mcp_default),
            (2, 0)
        );

        let out = bench(&loaded.sessions);
        let replayed = &out.main_agent.replayed;
        assert_eq!(replayed.resets, 4);
        assert_eq!(replayed.by_part.get("mcp"), Some(&4));
        // The last reset is between two recorded server sets.
        assert_eq!(replayed.assumed_part_resets, 3);
        assert_eq!(out.main_wakes.turns, 2);
    }

    /// Only an attended turn's servers are borrowed whole: an unattended
    /// one's lose the team server, computer control and services, a plan
    /// one's the team server and services.
    #[test]
    fn borrowed_servers_are_cut_to_the_turns_kind() {
        let all: Vec<String> = [
            "hollow_questions",
            TEAM_SERVER,
            COMPUTER_SERVER,
            crate::services_mcp::SERVER_NAME,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(servers_for(&all, false, false), all);
        assert_eq!(
            servers_for(&all, true, false),
            vec!["hollow_questions".to_string(), COMPUTER_SERVER.to_string()]
        );
        assert_eq!(
            servers_for(&all, false, true),
            vec!["hollow_questions".to_string()]
        );
    }

    /// Recorded config-changed closes the bench can't count are counted in
    /// `unmatched`: before any turn, at a turn before the window, at a turn
    /// with no agent to replay.
    #[test]
    fn unmatched_counts_closes_at_no_turn_an_earlier_turn_or_a_skipped_one() {
        use crate::session_store::{restart_stint, start_turn};
        let dir = store();
        let v = dir.path();
        let config = ResetReason::ConfigChanged.as_str();
        let (early, none, skip) = ("s-s07-early", "s-s07-none", "s-s07-skip");
        store_turn(v, early, MAIN, "run-s07-e1", "First.", "acceptEdits", None);
        store_turn(v, early, MAIN, "run-s07-e2", "Second.", "acceptEdits", None);
        restart_stint(v, early, "claude", config).unwrap();
        store_turn(v, none, MAIN, "run-s07-n1", "Only.", "acceptEdits", None);
        restart_stint(v, none, "claude", config).unwrap();
        start_turn(v, skip, "nobody", MAIN, "run-s07-x1", "Unknown agent.").unwrap();
        restart_stint(v, skip, "nobody", config).unwrap();
        let db = crate::session_store::db_path_for(v).unwrap();
        {
            let conn = Connection::open(&db).unwrap();
            let at = |day: &str| format!("2026-01-{day}+00:00");
            for (key, ord, started) in [
                (early, 1, at("01T00:00:00")),
                (early, 2, at("03T00:00:00")),
                (none, 1, at("04T00:00:00")),
                (skip, 1, at("03T12:00:00")),
            ] {
                conn.execute(
                    "UPDATE turns SET started_at = ?3 WHERE session_key = ?1 AND ord = ?2",
                    params![key, ord, started],
                )
                .unwrap();
            }
            for (key, ended) in [
                (early, at("02T00:00:00")),
                (none, at("02T06:00:00")),
                (skip, at("03T13:00:00")),
            ] {
                conn.execute(
                    "UPDATE stints SET ended_at = ?2 WHERE session_key = ?1 AND reason = ?3",
                    params![key, ended, config],
                )
                .unwrap();
            }
        }
        let conn =
            Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let window = Window {
            from: parse_iso("2026-01-01T12:00:00Z").unwrap(),
            to: parse_iso("2026-01-05T00:00:00Z").unwrap(),
        };
        let loaded = load(&conn, v, &window).unwrap();
        assert_eq!(
            loaded.unmatched,
            Unmatched {
                no_turn: 1,
                pre_window_turn: 1,
                skipped_turn: 1
            }
        );
        assert_eq!(loaded.skipped_turns, 1);
        let out = bench(&loaded.sessions);
        assert_eq!((out.all.turns, out.all.recorded.resets), (2, 0));
    }
}
