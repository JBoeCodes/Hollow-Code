//! Paperclip PR 14 (14b): resume hardening for chat turns.
//!
//! Three pieces, all AppHandle-free so tests drive them. What a fresh
//! native session is sent (the conversation bundle and the operator profile)
//! is the host context compiler's (`context_compiler.rs`, agent
//! effectiveness F4), for every entry point.
//!
//! - **The configuration fingerprint.** [`config_fingerprint`] hashes what a
//!   native session was started with: agent, model, whether the mode is
//!   plan (HB01: every adapter takes the mode itself per process, resumes
//!   included, so only plan's read-only session counts), working
//!   folder, the names of the attached Hollow MCP servers, what selects
//!   the appended system prompt's text (the attached capabilities, the
//!   Canvases choice, the workspace kind) with [`PROMPT_CONTRACT`] (OB04,
//!   v3: never the text itself), and skills (none yet). Effort, the MCP
//!   JSON body (it carries per-run tags and sockets), disallowed tools and
//!   any per-run directory are left out. [`plan_resume_with`] turns the
//!   fingerprint and the stored stints into [`ResumePlan`], resuming a
//!   stint an older version stamped when [`may_restamp`] allows it. Each
//!   part's own hash is stored beside it ([`config_parts`], OB03), so a
//!   config-changed reset names what changed ([`reset_parts`], [`PARTS`]).
//! - **Probation.** A resume attempt that can still be retried holds back
//!   what it would announce (the native id, its trace events, an API-error
//!   message, the result) until real content arrives ([`Probation`]), so a
//!   failed resume never reaches the UI or the store.
//! - **Quota failover (R1).** [`decide_failover`] decides, from facts
//!   alone, whether a Claude turn whose account ran out resumes the same
//!   native session on each eligible untried account ([`FAILOVER_PROMPT`]),
//!   and [`failover_stop_message`] words the
//!   stop when it cannot.
//! - **Mid-turn guards (2026-10-09).** R1 acts only after an attempt fails,
//!   but an account past 100% on overage is still served, so no error ever
//!   arrives. The CLI re-emits `rate_limit_event` whenever the rate-limit
//!   state it reads from each API response changes (and at most every 30 s
//!   while `rejected`), so a running attempt knows. [`decide_ceiling`] moves
//!   it at a safe point ([`SafePoints`]: its next model request starting
//!   with no tool call open) when its account is at or
//!   over the Spread ceiling and another has room: the same native session,
//!   [`CEILING_PROMPT`], with no further proactive ceiling move. The night
//!   guard (`night_queue::NightSpend`) ends a night turn at a safe point
//!   once the overnight stop is reached.

use crate::agent::AgentKind;
use crate::mcp_attach::AttachedCaps;
use crate::session_store::{OpenStint, PreviousTurn, ResumeState};
use crate::visual_prompt::CanvasFrequency;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

/// A JSON string literal (serde_json's escaping matches `JSON.stringify` for
/// every well-formed string).
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

// ── The configuration fingerprint ──────────────────────────────────────────

/// What a native session was started with (see the module doc for what is
/// left out and why).
#[derive(Debug, Clone)]
pub(crate) struct FingerprintInput<'a> {
    pub agent: AgentKind,
    pub model: Option<&'a str>,
    /// The resolved permission mode; only [`mode_class`] of it is hashed.
    pub perm: &'a str,
    /// The working folder the agent is spawned in.
    pub cwd: &'a Path,
    /// The attached Hollow MCP servers' names ([`mcp_server_names`]).
    pub mcp_servers: &'a [String],
    /// The attached capabilities, whose bits select the appended prompt's
    /// text ([`caps_bits`]). OB04 (v3): the text itself is not hashed, so a
    /// guidance edit reaches a resumed session instead of restarting it;
    /// [`PROMPT_CONTRACT`] restarts sessions on purpose.
    pub caps: AttachedCaps,
    /// The turn's Canvases choice (it selects "Visual answers in Hollow").
    pub canvases: CanvasFrequency,
    /// Which brief the appended prompt carries: `main`, `quick`,
    /// `workspace`, `vault`, or `bare` for a bare scratch turn
    /// (`chat::PromptKind`).
    pub workspace: &'a str,
    /// Per-run skills. Hollow passes none today; the slot is kept for M5.
    /// Not the installed skills, and not the operator profile either
    /// (agent effectiveness F4, K2): each turn's CLI process loads its own
    /// skills, and a profile edit reaches a resumed session as the context
    /// compiler's `<operator-update>` delta, without a restart.
    pub skills: &'a [String],
}

/// What of the permission mode a native session is bound to (HB01, evening
/// 10-16): `plan`, which reads only, or `act` for every other mode. Claude
/// takes `--permission-mode`, Codex its sandbox override and Antigravity
/// `--sandbox` or its bypass flag on every process, resumes included, and a
/// chat's configured half says nothing mode-specific outside plan
/// (`mode_contract::MODE_POINTER`; the mode is the turn's `<hollow-mode>`
/// line). Grok is the exception: a Grok session keeps the sandbox profile
/// it started with and refuses to resume under another
/// (`result_kind::GROK_RESUME_RE`), so its class is that profile
/// ([`crate::agent::grok_sandbox`]). Hashing the mode itself reset the
/// session each time a Main-agent wake (then capped at acceptEdits) and the
/// person's bypass turn alternated: 74 of 99 config-changed resets from
/// 10-09 to 10-16.
pub(crate) fn mode_class(agent: AgentKind, perm: &str) -> &'static str {
    let perm = crate::app_mcp::normalize_perm(perm);
    if perm == "plan" {
        return "plan";
    }
    match (agent, crate::agent::grok_sandbox(perm)) {
        (AgentKind::Grok, "read-only") => "grok:read-only",
        (AgentKind::Grok, "off") => "grok:off",
        (AgentKind::Grok, _) => "grok:workspace",
        _ => "act",
    }
}

/// The version of what the appended system prompt tells a session, hashed
/// into every fingerprint instead of the prompt's bytes (OB04). Bump it on
/// purpose, in the change itself, when that change should reset every
/// native session: guidance a resumed session would act against (a tool
/// renamed or removed, a rule reversed), not wording, a new sentence or a
/// trimmed one. A bump restarts every conversation once, at install.
pub(crate) const PROMPT_CONTRACT: u32 = 1;

/// The fingerprint version prefixes before v3 (OB04): v1 hashed the mode
/// itself, v2 the appended prompt's bytes.
const OLDER_VERSIONS: [&str; 2] = ["v1:", "v2:"];

/// Whether `stored` was stamped by a fingerprint version before v3.
pub(crate) fn is_older_version(stored: &str) -> bool {
    OLDER_VERSIONS
        .iter()
        .any(|prefix| stored.starts_with(prefix))
}

/// The [`AttachedCaps`] bits that select the appended prompt's text (the
/// capability index, the environment brief, the guidance blocks), by
/// name, with the agent whose provider-contract line ends the index.
pub(crate) fn caps_bits(caps: &AttachedCaps) -> Vec<String> {
    let mut bits: Vec<String> = [
        (caps.computer, "computer"),
        (caps.vault, "vault"),
        (caps.code_context, "codeContext"),
        (caps.team_claude, "teamClaude"),
        (caps.team_codex, "teamCodex"),
        (caps.team_antigravity, "teamAntigravity"),
        (caps.team_grok, "teamGrok"),
        (caps.questions, "questions"),
        (caps.tasks, "tasks"),
        (caps.tasks_read_only, "tasksReadOnly"),
        (caps.app, "app"),
        (caps.app_read_only, "appReadOnly"),
        (caps.services, "services"),
        (caps.questions_fenced, "questionsFenced"),
    ]
    .into_iter()
    .filter(|&(on, _)| on)
    .map(|(_, name)| name.to_string())
    .collect();
    if let Some(agent) = caps.agent {
        bits.push(format!("contract:{}", agent.as_str()));
    }
    bits
}

/// The fingerprint's parts (OB03), in its canonical order: the names a
/// config-changed reset gives for what changed (the `sessionReset` event's
/// `changed`, the run manifest's `compiler.resetParts`). Stable, short
/// strings that the reset notice and the scorecard key on. They are also
/// the canonical text's keys, so renaming one changes the v3 hash and
/// resets every native session once: don't, short of a fingerprint
/// version bump. `contract` is [`PROMPT_CONTRACT`], `mode` the
/// [`mode_class`], `mcp` the attached Hollow servers' names, `caps` the
/// [`caps_bits`], `canvases` the Canvases choice, `workspace` the prompt
/// kind; the rest are the [`FingerprintInput`] fields of the same name.
pub(crate) const PARTS: [&str; 10] = [
    "contract",
    "agent",
    "model",
    "mode",
    "cwd",
    "mcp",
    "caps",
    "canvases",
    "workspace",
    "skills",
];

/// What a reset names when the stint stored no parts (stamped before
/// OB03, or a legacy NULL fingerprint): never a guess.
pub(crate) const UNKNOWN_PART: &str = "unknown";

/// `"v3:"` + the sha256 hex of a canonical JSON object whose keys are
/// written in a fixed order (never through a map, whose order depends on
/// serde_json's features): `v`, then [`PARTS`]: `contract`
/// ([`PROMPT_CONTRACT`]), `agent`, `model` (`""` when unset), `mode`
/// ([`mode_class`]), `cwd`, `mcp` (sorted names), `caps` ([`caps_bits`],
/// sorted), `canvases`, `workspace`, `skills` (sorted). v1 hashed the mode
/// itself as `perm`, v2 the exact appended prompt; a stint either stamped
/// resumes once [`may_restamp`] says nothing else changed, and is
/// restamped v3 after that turn succeeds.
pub(crate) fn config_fingerprint(input: &FingerprintInput<'_>) -> String {
    fingerprint_at(input, PROMPT_CONTRACT)
}

/// [`config_fingerprint`] under `contract` (a test bumps it).
fn fingerprint_at(input: &FingerprintInput<'_>, contract: u32) -> String {
    let fields = PARTS
        .iter()
        .zip(part_values(input, contract))
        .map(|(name, value)| format!("\"{name}\":{value}"))
        .collect::<Vec<_>>()
        .join(",");
    let canonical = format!("{{\"v\":3,{fields}}}");
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("v3:{:x}", hasher.finalize())
}

/// Each of [`PARTS`]' canonical JSON value under `contract`, in order.
fn part_values(input: &FingerprintInput<'_>, contract: u32) -> [String; 10] {
    let sorted = |items: &[String]| {
        let mut items: Vec<&str> = items.iter().map(String::as_str).collect();
        items.sort_unstable();
        items.dedup();
        format!(
            "[{}]",
            items
                .iter()
                .map(|item| json_string(item))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let model = input.model.map(str::trim).unwrap_or("");
    [
        contract.to_string(),
        json_string(input.agent.as_str()),
        json_string(model),
        json_string(mode_class(input.agent, input.perm)),
        json_string(&input.cwd.to_string_lossy()),
        sorted(input.mcp_servers),
        sorted(&caps_bits(&input.caps)),
        json_string(input.canvases.as_str()),
        json_string(input.workspace),
        sorted(input.skills),
    ]
}

/// The fingerprint's per-part hashes (OB03), stored beside it as
/// `stints.config_parts`: a JSON object from each of [`PARTS`] to the
/// first 16 hex digits of the sha256 of its canonical value (never the
/// value: the folder and model stay out of the store's text).
pub(crate) fn config_parts(input: &FingerprintInput<'_>) -> String {
    parts_at(input, PROMPT_CONTRACT)
}

/// [`config_parts`] under `contract` (a test bumps it).
fn parts_at(input: &FingerprintInput<'_>, contract: u32) -> String {
    let fields = PARTS
        .iter()
        .zip(part_values(input, contract))
        .map(|(name, value)| {
            let mut hasher = Sha256::new();
            hasher.update(value.as_bytes());
            let hex = format!("{:x}", hasher.finalize());
            format!("\"{name}\":\"{}\"", &hex[..16])
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{fields}}}")
}

/// A stored `config_parts` object, or `None` when it doesn't parse.
fn parse_parts(text: &str) -> Option<serde_json::Map<String, Value>> {
    serde_json::from_str(text).ok()
}

/// The parts that differ between `stored` (a stint's `config_parts`) and
/// `current` ([`config_parts`]), in [`PARTS`] order (OB03). No stored
/// parts, parts that don't parse, or parts that all match name
/// [`UNKNOWN_PART`] alone.
pub(crate) fn changed_parts(stored: Option<&str>, current: &str) -> Vec<&'static str> {
    let (Some(stored), Some(current)) = (stored.and_then(parse_parts), parse_parts(current)) else {
        return vec![UNKNOWN_PART];
    };
    let changed: Vec<&'static str> = PARTS
        .into_iter()
        .filter(|name| stored.get(*name) != current.get(*name))
        .collect();
    if changed.is_empty() {
        vec![UNKNOWN_PART]
    } else {
        changed
    }
}

/// What a [`ResetReason::ConfigChanged`] plan names (OB03), against the
/// stint [`plan_resume_with`] compared: the open stint once it holds a
/// native id, else the last closed one (a pending reset, its rule 2).
pub(crate) fn reset_parts(state: &ResumeState, current: &str) -> Vec<&'static str> {
    let stored = match &state.open {
        Some(open) if open.harness_session_id.is_some() => open.config_parts.as_deref(),
        _ => state
            .last_closed
            .as_ref()
            .and_then(|closed| closed.config_parts.as_deref()),
    };
    changed_parts(stored, current)
}

/// The open stint, when an older fingerprint version stamped it: the only
/// case [`may_restamp`] needs the previous turn read for.
pub(crate) fn older_stamp(state: &ResumeState) -> Option<&OpenStint> {
    state.open.as_ref().and_then(older_stamp_of)
}

fn older_stamp_of(open: &OpenStint) -> Option<&OpenStint> {
    let stored = open.config_fingerprint.as_deref()?;
    is_older_version(stored).then_some(open)
}

/// Whether a stint an older fingerprint version stamped ([`is_older_version`])
/// may resume under v3 and be restamped (OB04): only when every check
/// holds against what the stint can't hash any more:
/// - not Grok (a Grok session refuses to resume under another sandbox
///   profile, so it keeps v2's reset-once);
/// - the open stint is this turn's agent;
/// - the last turn that ran on the stint's native session
///   (`session_store::previous_turn`: its usage row names that session, so
///   a turn that failed before its resume plan never counts) ran in this
///   turn's mode class (`turns.permission_mode`);
/// - that turn's chat usage row asked for this turn's model (its agent, when
///   recorded, this one), as `session_store::last_requested_model` reads it;
/// - `sessions.cwd` (the conversation's folder, which changes only on a
///   move) is this turn's folder.
///
/// A missing value (no such turn, no recorded mode, no usage row) fails the
/// check, so the turn resets as before. v1 and v2 hashed the model, and
/// the composer never clears the native id on a model change, so the
/// stored model is what keeps a model switch a reset.
pub(crate) fn may_restamp(
    open: &OpenStint,
    agent: AgentKind,
    perm: &str,
    model: Option<&str>,
    cwd: &Path,
    previous: Option<&PreviousTurn>,
) -> bool {
    if agent == AgentKind::Grok || open.agent != agent.as_str() {
        return false;
    }
    if older_stamp_of(open).is_none() {
        return false;
    }
    let Some(previous) = previous else {
        return false;
    };
    let Some(previous_perm) = previous.permission_mode.as_deref() else {
        return false;
    };
    if mode_class(agent, previous_perm) != mode_class(agent, perm) {
        return false;
    }
    let Some((previous_model, used)) = previous.requested.as_ref() else {
        return false;
    };
    if used.as_deref().is_some_and(|used| used != agent.as_str()) {
        return false;
    }
    let previous_model = previous_model.as_deref().map(str::trim).unwrap_or("");
    if previous_model != model.map(str::trim).unwrap_or("") {
        return false;
    }
    previous
        .session_cwd
        .as_deref()
        .is_some_and(|stored| stored == cwd.to_string_lossy().as_ref())
}

/// The sorted server names of an attached-server map (`mcp_attach`'s
/// `server_map_for`), never their bodies: those carry per-run values
/// (`HOLLOW_RUN_TAG`, `HOLLOW_RUN_ID`, the approval socket).
pub(crate) fn mcp_server_names(servers: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut names: Vec<String> = servers.keys().cloned().collect();
    names.sort_unstable();
    names
}

// ── The resume plan ────────────────────────────────────────────────────────

/// Why a conversation moved to a fresh native session. `as_str` is the
/// stint's close reason; `wire` is the `sessionReset` event's `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResetReason {
    ConfigChanged,
    ResumeFailed,
    /// The conversation moved to another workspace
    /// (`session_store::move_agent_session`): its native session belongs to
    /// the old folder.
    Moved,
    /// The requested id is not a shape any agent hands out
    /// ([`crate::agent::is_safe_resume_id`]): passed on, it could parse as a
    /// flag, so the turn starts fresh instead (review finding A·R10).
    InvalidId,
    /// Overnight 9 A9: a large conversation came back after more than an
    /// hour away ([`rests`]). Its prompt cache has expired, so a resume
    /// would re-read the whole context uncached; it starts fresh with the
    /// conversation bundle and task state instead. `away_minutes` and
    /// `tokens` word the notice; both are 0 when re-derived from a closed
    /// stint (a fresh attempt that never announced its id).
    Rested {
        away_minutes: u32,
        tokens: u64,
    },
}

impl ResetReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ResetReason::ConfigChanged => "config changed",
            ResetReason::ResumeFailed => "resume failed",
            ResetReason::Moved => crate::session_store::MOVED_REASON,
            ResetReason::InvalidId => "invalid session id",
            ResetReason::Rested { .. } => "rested",
        }
    }

    pub(crate) fn wire(self) -> &'static str {
        match self {
            ResetReason::ConfigChanged => "configChanged",
            ResetReason::ResumeFailed => "resumeFailed",
            ResetReason::Moved => "moved",
            ResetReason::InvalidId => "invalidId",
            ResetReason::Rested { .. } => "rested",
        }
    }

    fn from_stint_reason(reason: &str) -> Option<Self> {
        match reason {
            "config changed" => Some(ResetReason::ConfigChanged),
            "resume failed" => Some(ResetReason::ResumeFailed),
            crate::session_store::MOVED_REASON => Some(ResetReason::Moved),
            "invalid session id" => Some(ResetReason::InvalidId),
            "rested" => Some(ResetReason::Rested {
                away_minutes: 0,
                tokens: 0,
            }),
            _ => None,
        }
    }
}

/// Overnight 9 A9: a conversation away longer than this since its last
/// turn ended may start fresh instead of resuming ([`rests`]). Claude's
/// prompt cache lasts an hour at most, so after it every resume re-reads
/// the whole context at the uncached price.
pub(crate) const REST_AFTER_MINUTES: i64 = 60;

/// Overnight 9 A9: whether a turn may rest at all: a Claude turn (the only
/// agent whose resume cost was measured), not plan mode, not a bare scratch
/// turn, in the Main agent's workspace, with Settings, Background work,
/// COO → Start fresh after an hour away on, and never an unattended
/// turn (the night queue, a wake or check-in): a fresh stint expires the
/// conversation's open question cards, and only the person's own return
/// should do that. For the same reason never a turn another agent sent
/// (`app_depth` above 0: `send_to_session`, `start_session`; RF-810-F1).
pub(crate) fn may_rest(
    agent: AgentKind,
    perm: &str,
    bare: bool,
    unattended: bool,
    app_depth: u32,
    main_workspace: bool,
    setting_on: bool,
) -> bool {
    agent == AgentKind::Claude
        && perm != "plan"
        && !bare
        && !unattended
        && app_depth == 0
        && main_workspace
        && setting_on
}

/// Overnight 9 A9: the reset a turn that `may` rest gets instead of a
/// resume: away more than [`REST_AFTER_MINUTES`] since the conversation's
/// last turn ended, holding at least
/// [`crate::context_gauge::ADVISORY_TOKENS`] by its stored figure. `None`
/// (resume as planned) otherwise, or when either is unknown.
pub(crate) fn rests(
    may: bool,
    away_minutes: Option<i64>,
    tokens: Option<u64>,
) -> Option<ResetReason> {
    let away = away_minutes.filter(|minutes| *minutes > REST_AFTER_MINUTES)?;
    let tokens = tokens.filter(|tokens| *tokens >= crate::context_gauge::ADVISORY_TOKENS)?;
    may.then(|| ResetReason::Rested {
        away_minutes: u32::try_from(away).unwrap_or(u32::MAX),
        tokens,
    })
}

/// `plan` with A9's rest applied: only a resume becomes a reset.
pub(crate) fn with_rest(plan: ResumePlan, rest: Option<ResetReason>) -> ResumePlan {
    match (plan, rest) {
        (ResumePlan::Resume(_), Some(reason)) => ResumePlan::Reset(reason),
        (plan, _) => plan,
    }
}

/// "away 3 h at 714K" (A9's receipt row and notice); "away 75 min",
/// "away 2 days". `None` for a reason re-derived without figures.
pub(crate) fn rested_text(away_minutes: u32, tokens: u64) -> Option<String> {
    if away_minutes == 0 || tokens == 0 {
        return None;
    }
    let away = match away_minutes {
        0..=119 => format!("{away_minutes} min"),
        120..=2_879 => format!("{} h", away_minutes / 60),
        _ => format!("{} days", away_minutes / 1_440),
    };
    Some(format!("away {away} at {}K", (tokens + 500) / 1_000))
}

/// How a chat turn starts its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResumePlan {
    /// Resume this exact native session.
    Resume(String),
    /// No native session was requested: a new one.
    New,
    /// The requested session must not be resumed: a fresh one, with the
    /// conversation bundle.
    Reset(ResetReason),
}

/// Decide how a turn that requested `requested` starts, in order:
/// 1. nothing requested → [`ResumePlan::New`]; an id that is not a safe
///    provider id ([`crate::agent::is_safe_resume_id`]) → reset, invalid id,
///    so it is never passed where it could parse as a flag;
/// 2. a pending reset: the open stint has no native id yet and the last
///    closed stint closed for a config change, a failed resume or a move to
///    another workspace while holding `requested` (a fresh attempt that
///    never announced its id, or a moved conversation, with the old id still
///    persisted in the composer) → reset again, for that reason; also a
///    rest (A9): its reason is re-derived without figures and reapplied
///    whatever the setting or mode now say, since the bundle already went
///    to the abandoned attempt and the old session must not be resumed;
/// 3. no open stint, an open stint holding another id, or a NULL (legacy,
///    unknown) fingerprint → resume;
/// 4. the same fingerprint → resume;
/// 5. a stored fingerprint an older version stamped ([`is_older_version`])
///    when `restamp` ([`may_restamp`] for the open stint, OB04) → resume;
///    the caller overwrites the stamp with v3 once the turn succeeds;
/// 6. otherwise (a different fingerprint on the requested session) → reset,
///    config changed.
pub(crate) fn plan_resume_with(
    requested: Option<&str>,
    state: &ResumeState,
    fingerprint: &str,
    restamp: bool,
) -> ResumePlan {
    let Some(requested) = requested.map(str::trim).filter(|id| !id.is_empty()) else {
        return ResumePlan::New;
    };
    if !crate::agent::is_safe_resume_id(requested) {
        return ResumePlan::Reset(ResetReason::InvalidId);
    }
    let resume = || ResumePlan::Resume(requested.to_string());
    let Some(open) = state.open.as_ref() else {
        return resume();
    };
    if open.harness_session_id.is_none() {
        if let Some(closed) = state.last_closed.as_ref() {
            let reason = closed
                .reason
                .as_deref()
                .and_then(ResetReason::from_stint_reason);
            if let Some(reason) =
                reason.filter(|_| closed.harness_session_id.as_deref() == Some(requested))
            {
                return ResumePlan::Reset(reason);
            }
        }
    }
    if open.harness_session_id.as_deref() != Some(requested) {
        return resume();
    }
    match open.config_fingerprint.as_deref() {
        None => resume(),
        Some(stored) if stored == fingerprint => resume(),
        Some(stored) if restamp && is_older_version(stored) => resume(),
        Some(_) => ResumePlan::Reset(ResetReason::ConfigChanged),
    }
}

/// [`plan_resume_with`] with no restamp: a stint an older version stamped
/// resets.
#[cfg(test)]
pub(crate) fn plan_resume(
    requested: Option<&str>,
    state: &ResumeState,
    fingerprint: &str,
) -> ResumePlan {
    plan_resume_with(requested, state, fingerprint, false)
}

// ── Probation ──────────────────────────────────────────────────────────────

/// Something a resume attempt would announce, held until it commits.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Held {
    /// A `chat-event` payload.
    Emit(Value),
    /// A session-store trace event of the turn.
    Store { kind: &'static str, payload: String },
    /// The native id to record on the open stint (with the turn's
    /// fingerprint).
    HarnessId(String),
}

/// Holds a resume attempt's announcements until the first real content
/// (a turn boundary, assistant text, a tool event), so a resume that fails
/// before any content can be discarded without trace and retried fresh. A
/// probation that does not hold passes everything straight through.
#[derive(Debug, Default)]
pub(crate) struct Probation {
    holding: bool,
    held: Vec<Held>,
}

impl Probation {
    pub(crate) fn new(holding: bool) -> Self {
        Probation {
            holding,
            held: Vec::new(),
        }
    }

    /// Still holding: no content has committed the attempt.
    pub(crate) fn holding(&self) -> bool {
        self.holding
    }

    /// Hold `item`, or hand it back to be performed now.
    pub(crate) fn push(&mut self, item: Held) -> Option<Held> {
        if self.holding {
            self.held.push(item);
            None
        } else {
            Some(item)
        }
    }

    /// Stop holding and hand back everything held, in order.
    pub(crate) fn commit(&mut self) -> Vec<Held> {
        self.holding = false;
        std::mem::take(&mut self.held)
    }

    /// Drop everything held (a failed resume that will be retried).
    pub(crate) fn discard(&mut self) {
        self.held.clear();
    }

    #[cfg(test)]
    pub(crate) fn held(&self) -> &[Held] {
        &self.held
    }
}

// ── Quota failover (R1) ────────────────────────────────────────────────────

/// The whole prompt of a turn resumed on another account. The native
/// session already holds the user's message, so it is never sent again.
pub(crate) const FAILOVER_PROMPT: &str =
    "Continue where you left off; the previous account ran out of usage.";

/// What an attempt resumed after a provider outage is told (overnight 9
/// A3, `chat::Next::Backoff`, not a failover): the session already holds
/// the user's message.
pub(crate) const BACKOFF_PROMPT: &str =
    "Continue where you left off; the provider was overloaded and the last attempt failed.";

/// Proactive ceiling switches per chat turn, at most. Terminal quota
/// recovery instead visits each eligible account at most once.
pub(crate) const MAX_FAILOVERS: u32 = 1;

/// What one source says about whether an account is out of room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exhaustion {
    /// Out of allowance, or at or over the routing ceiling.
    Exhausted,
    NotExhausted,
    /// Nothing to go on (no event in the run; a probe that failed).
    Unknown,
}

impl From<Option<bool>> for Exhaustion {
    fn from(value: Option<bool>) -> Self {
        match value {
            Some(true) => Exhaustion::Exhausted,
            Some(false) => Exhaustion::NotExhausted,
            None => Exhaustion::Unknown,
        }
    }
}

/// Everything [`decide_failover`] reads about one failed Claude attempt.
#[derive(Debug, Clone)]
pub(crate) struct FailoverFacts<'a> {
    /// Settings, Usage, "Resume on another account when one runs out".
    pub enabled: bool,
    /// The added account the attempt ran on; `None` for Claude Code's own
    /// login or with no account added (nothing to switch between).
    pub account: Option<&'a str>,
    /// How the attempt ended (`result_kind::classify`).
    pub kind: crate::result_kind::ResultKind,
    /// An overload, a 5xx or a network error (`result_kind::is_outage`).
    pub outage: bool,
    /// The native session to resume; `None` when it never announced one.
    pub session: Option<&'a str>,
    /// The attempt's own `rate_limit_event`s (the last one).
    pub stream: Exhaustion,
    /// One fresh probe, once [`Failover::Probe`] asked for it.
    pub probe: Option<Exhaustion>,
    /// Accounts already attempted this turn, including the current one.
    pub tried: &'a [String],
}

/// What a failed attempt leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failover {
    /// Not an exhausted account (or failover is off): the turn ends as it
    /// always did.
    No,
    /// Only a fresh probe of the account can tell: probe, then decide again.
    Probe,
    /// Resume the same native session on `to`.
    Switch { to: String },
    /// The account is exhausted and the turn cannot move.
    Stop(FailoverStop),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailoverStop {
    /// No other account is eligible.
    NoneEligible,
}

/// Decide a failed Claude attempt's failover, in order:
/// 1. off, no added account, no native session, a kind other than
///    `Transient`/`ProviderQuota`, or an outage (a 529, an overload, a
///    network error) → [`Failover::No`]: a real outage never walks the
///    accounts;
/// 2. neither a quota rejection, the attempt's events nor a probe says the
///    account is exhausted → [`Failover::Probe`] once, then [`Failover::No`];
/// 3. `next()` (the spread policy over eligible accounts, excluding the
///    exhausted ones) → switch, or stop when none is eligible. `next` runs
///    only here.
pub(crate) fn decide_failover(
    facts: &FailoverFacts<'_>,
    next: impl FnOnce() -> Option<String>,
) -> Failover {
    use crate::result_kind::ResultKind;
    let candidate = facts.enabled
        && facts.account.is_some()
        && facts.session.is_some_and(|id| !id.trim().is_empty())
        && matches!(
            facts.kind,
            ResultKind::Transient | ResultKind::ProviderQuota
        )
        && !facts.outage;
    if !candidate {
        return Failover::No;
    }
    // A rejected quota envelope is direct evidence. A stale stream or an
    // unavailable usage endpoint must not veto that rejection. Generic 429s
    // still require stream/probe evidence; auth/outages were excluded above.
    let exhausted = facts.kind == ResultKind::ProviderQuota
        || facts.stream == Exhaustion::Exhausted
        || facts.probe == Some(Exhaustion::Exhausted);
    if !exhausted {
        return if facts.probe.is_none() {
            Failover::Probe
        } else {
            Failover::No
        };
    }
    match next() {
        Some(to)
            if !to.trim().is_empty()
                && Some(to.as_str()) != facts.account
                && !facts.tried.contains(&to) =>
        {
            Failover::Switch { to }
        }
        _ => Failover::Stop(FailoverStop::NoneEligible),
    }
}

/// A routing ceiling cannot terminate jobs the current attempt started.
/// Explain that once without stopping work or changing any provider setting.
pub(crate) fn quota_background_notice(
    enabled: bool,
    added_account: bool,
    over: bool,
    background: bool,
) -> Option<&'static str> {
    (enabled && added_account && over && background).then_some(
        "Claude reached its routing ceiling. This attempt started background jobs, so Hollow will not switch its account while those jobs may still be running. You can let the turn finish or stop it yourself. Your chat stays here.",
    )
}

// ── Mid-turn guards (2026-10-09) ───────────────────────────────────────────

/// The whole prompt of a turn moved at the ceiling (it had not run out).
pub(crate) const CEILING_PROMPT: &str =
    "Continue where you left off; the turn moved to another account.";

/// Where a running attempt may be ended without cutting a tool call in two
/// (review fixes 2026-10-09): the line that starts the agent's next MAIN
/// model request (a `message_start` with no `parent_tool_use_id`), when no
/// tool call is open and at least one tool result arrived since the last
/// request. By then the CLI has finished the previous message (every
/// `tool_use` block of it, parallel ones included) and taken its results
/// into the request it builds from the session, so the resumed session holds
/// whole calls; ending here aborts only that fresh request. A teammate's or
/// subagent's own messages (non-null `parent_tool_use_id`) never count, and
/// a call stays open until its own result, so nothing inside a call counts.
/// Evaluated once per stdout line, after all of its events.
#[derive(Debug, Default)]
pub(crate) struct SafePoints {
    open: std::collections::HashSet<String>,
    results_since_request: bool,
    background: bool,
}

impl SafePoints {
    /// Fold one stdout line's events in: true when the line is a safe point.
    pub(crate) fn observe_line(&mut self, line: &str, events: &[crate::agent::AgentEvent]) -> bool {
        use crate::agent::AgentEvent;
        let mut starts = false;
        for evt in events {
            match evt {
                AgentEvent::ToolUse { id, input, .. } => {
                    self.open.insert(id.clone());
                    if input.get("run_in_background").and_then(Value::as_bool) == Some(true) {
                        self.background = true;
                    }
                }
                AgentEvent::ToolResult { id, .. } => {
                    self.open.remove(id);
                    self.results_since_request = true;
                }
                AgentEvent::TurnBoundary => starts = true,
                _ => {}
            }
        }
        if !starts {
            return false;
        }
        let main = serde_json::from_str::<Value>(line)
            .ok()
            .is_some_and(|v| v.get("parent_tool_use_id").is_none_or(Value::is_null));
        if !main {
            return false;
        }
        let safe = self.open.is_empty() && self.results_since_request;
        self.results_since_request = false;
        safe
    }

    /// The attempt started work that outlives its own tool result (a
    /// `run_in_background` shell or agent): ending the process would kill
    /// it, so the attempt is never moved to another account.
    pub(crate) fn background(&self) -> bool {
        self.background
    }
}

/// Everything [`decide_ceiling`] reads about a running Claude attempt.
#[derive(Debug, Clone)]
pub(crate) struct CeilingFacts<'a> {
    /// Spread usage AND "Resume on another account when one runs out", with
    /// added accounts. Single mode and failover off never move a turn.
    pub enabled: bool,
    /// The added account the attempt runs on (`None`: the login).
    pub account: Option<&'a str>,
    /// The native session to resume; `None` until the stream announced one.
    pub session: Option<&'a str>,
    /// The attempt's last `rate_limit_event` puts its account at or over the
    /// ceiling. Overage counts: utilization past 100% (`1.01`) or a
    /// `rejected` status is over any ceiling.
    pub over: bool,
    /// Switches this turn already made, for either reason.
    pub failovers: u32,
    /// The attempt posted a question card: the agent is ending its turn to
    /// wait for the person, so it must not be told to continue.
    pub posted_card: bool,
    /// The attempt started background work ([`SafePoints::background`]).
    pub background: bool,
}

/// Whether a running attempt moves to another account at a safe point
/// ([`SafePoints`]), in order: off, no added account, no session, under the
/// ceiling, a question card posted, background work running, or the turn already
/// switched ([`MAX_FAILOVERS`]: no further proactive ceiling moves;
/// terminal quota recovery can still continue) → `None`, and the turn goes on
/// where it is. Otherwise `next()` (the spread policy over the eligible
/// accounts under the ceiling, never this one); `None` when none is
/// eligible, so a turn with nowhere better to go just continues.
pub(crate) fn decide_ceiling(
    facts: &CeilingFacts<'_>,
    next: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let candidate = facts.enabled
        && facts.account.is_some()
        && facts.session.is_some_and(|id| !id.trim().is_empty())
        && facts.over
        && !facts.posted_card
        && !facts.background
        && facts.failovers < MAX_FAILOVERS;
    if !candidate {
        return None;
    }
    next().filter(|to| Some(to.as_str()) != facts.account)
}

/// Why an account switch happened, on the receipt and the transcript row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchReason {
    /// The account ran out: the attempt failed (R1).
    Limit,
    /// The account passed the Spread ceiling mid-turn (turn guards).
    Ceiling,
}

impl SwitchReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SwitchReason::Limit => "limit",
            SwitchReason::Ceiling => "ceiling",
        }
    }

    /// The receipt's reason line.
    pub(crate) fn receipt(self, from: &str) -> String {
        match self {
            SwitchReason::Limit => format!("{from} hit its limit"),
            SwitchReason::Ceiling => format!("{from} passed its ceiling"),
        }
    }

    /// What the resumed attempt is told.
    pub(crate) fn prompt(self) -> &'static str {
        match self {
            SwitchReason::Limit => FAILOVER_PROMPT,
            SwitchReason::Ceiling => CEILING_PROMPT,
        }
    }
}

/// A reset time as the stop message shows it, in `now`'s time zone: `01:20`
/// today, else with the weekday (`Fri 01:20`).
pub(crate) fn reset_label<Tz: chrono::TimeZone>(
    at: i64,
    now: &chrono::DateTime<Tz>,
) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let at = chrono::DateTime::<chrono::Utc>::from_timestamp(at, 0)?.with_timezone(&now.timezone());
    Some(if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%a %H:%M").to_string()
    })
}

/// The error a turn that could not move ends with. `resets` is the
/// soonest reset ([`reset_label`]). One line, note-safe. With no account
/// left it says when Claude has room again (repo ideas 10-27 #8:
/// "Waiting for Claude until 16:40", `agent_out::waiting`).
pub(crate) fn failover_stop_message(stop: FailoverStop, resets: Option<&str>) -> String {
    match stop {
        FailoverStop::NoneEligible => format!(
            "{}: no other eligible Claude account has room. Check Accounts before retrying.",
            crate::agent_out::waiting("Claude", resets)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::{ClosedStint, OpenStint};
    use serde_json::json;

    /// Turn guards: a safe point is the line starting the next MAIN model
    /// request with no call open and a result since the last request. A
    /// result inside an open call, a subagent's own request, the attempt's
    /// first request and a request with a call still open never are.
    #[test]
    fn safe_points_never_fall_inside_a_tool_call() {
        use crate::agent::AgentEvent;
        let tool = |id: &str, input: Value| AgentEvent::ToolUse {
            id: id.into(),
            name: "Bash".into(),
            input,
        };
        let result = |id: &str| AgentEvent::ToolResult {
            id: id.into(),
            content: "ok".into(),
            is_error: false,
        };
        let main =
            r#"{"type":"stream_event","event":{"type":"message_start"},"parent_tool_use_id":null}"#;
        let sub =
            r#"{"type":"stream_event","event":{"type":"message_start"},"parent_tool_use_id":"a"}"#;
        let start = [AgentEvent::TurnBoundary];
        let mut safe = SafePoints::default();
        assert!(
            !safe.observe_line(main, &start),
            "the attempt's first request"
        );
        assert!(!safe.observe_line("", &[tool("a", json!({}))]));
        assert!(!safe.observe_line(sub, &start), "a subagent's request");
        assert!(!safe.observe_line("", &[tool("b", json!({})), result("b")]));
        assert!(!safe.observe_line(main, &start), "a is still open");
        assert!(!safe.observe_line("", &[result("a")]));
        // Parallel calls: a's result arrived, but c was opened in the same
        // message before the next request.
        assert!(!safe.observe_line("", &[tool("c", json!({}))]));
        assert!(!safe.observe_line(main, &start), "c is open");
        assert!(!safe.observe_line("", &[result("c")]));
        assert!(!safe.observe_line(sub, &start), "only the main thread");
        assert!(safe.observe_line(main, &start), "nothing open, results in");
        assert!(!safe.observe_line(main, &start), "no result since");
        assert!(!safe.background());
        safe.observe_line(
            "",
            &[tool("d", json!({"run_in_background": true})), result("d")],
        );
        assert!(safe.background());
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive_to_each_input() {
        let cwd = Path::new("/Users/x/vault");
        let other_cwd = Path::new("/Users/x/vault/workspaces/w1");
        let servers = vec!["hollow_vault".to_string(), "hollow_questions".to_string()];
        let base = FingerprintInput {
            agent: AgentKind::Claude,
            model: Some("claude-opus-5-5"),
            perm: "acceptEdits",
            cwd,
            mcp_servers: &servers,
            caps: test_caps(),
            canvases: CanvasFrequency::Helps,
            workspace: "vault",
            skills: &[],
        };
        let fp = config_fingerprint(&base);
        assert!(fp.starts_with("v3:"));
        assert!(!is_older_version(&fp));
        assert_eq!(fp.len(), 3 + 64);
        assert!(fp[3..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(fp, config_fingerprint(&base.clone()), "stable");

        let reordered = vec!["hollow_questions".to_string(), "hollow_vault".to_string()];
        assert_eq!(
            fp,
            config_fingerprint(&FingerprintInput {
                mcp_servers: &reordered,
                ..base.clone()
            }),
            "server order does not matter"
        );
        assert_eq!(
            config_fingerprint(&FingerprintInput {
                model: None,
                ..base.clone()
            }),
            config_fingerprint(&FingerprintInput {
                model: Some(""),
                ..base.clone()
            }),
            "an unset model is the empty string"
        );

        let fewer = vec!["hollow_vault".to_string()];
        let skills = vec!["release-notes".to_string()];
        for changed in [
            FingerprintInput {
                agent: AgentKind::Codex,
                ..base.clone()
            },
            FingerprintInput {
                model: Some("claude-sonnet-5"),
                ..base.clone()
            },
            FingerprintInput {
                perm: "plan",
                ..base.clone()
            },
            FingerprintInput {
                cwd: other_cwd,
                ..base.clone()
            },
            FingerprintInput {
                mcp_servers: &fewer,
                ..base.clone()
            },
            FingerprintInput {
                caps: AttachedCaps {
                    app_read_only: true,
                    ..test_caps()
                },
                ..base.clone()
            },
            FingerprintInput {
                caps: AttachedCaps {
                    agent: None,
                    ..test_caps()
                },
                ..base.clone()
            },
            FingerprintInput {
                canvases: CanvasFrequency::Most,
                ..base.clone()
            },
            FingerprintInput {
                workspace: "main",
                ..base.clone()
            },
            FingerprintInput {
                skills: &skills,
                ..base.clone()
            },
        ] {
            assert_ne!(config_fingerprint(&changed), fp, "{changed:?}");
        }
        assert_ne!(
            fingerprint_at(&base, PROMPT_CONTRACT + 1),
            fp,
            "a contract bump"
        );

        // Per-run MCP values never reach the hash: only the names do.
        let mut one = serde_json::Map::new();
        one.insert(
            "hollow_team".into(),
            json!({"env": {"HOLLOW_RUN_TAG": "aaaa", "HOLLOW_RUN_ID": "run-1"}}),
        );
        let mut two = serde_json::Map::new();
        two.insert(
            "hollow_team".into(),
            json!({"env": {"HOLLOW_RUN_TAG": "bbbb", "HOLLOW_RUN_ID": "run-2"}}),
        );
        assert_eq!(mcp_server_names(&one), mcp_server_names(&two));
        assert_eq!(mcp_server_names(&one), vec!["hollow_team".to_string()]);
        // Effort is not an input: FingerprintInput has no such field, and
        // the canonical text names only the listed keys.
    }

    /// OB03: the stored parts name exactly the input that changed, one at
    /// a time; a stint with no parts (or parts that don't parse) names
    /// `unknown`, never a guess; and the v3 hash is still the one OB04
    /// shipped (its canonical text, written out by hand).
    #[test]
    fn a_reset_names_exactly_the_changed_part() {
        let cwd = Path::new("/Users/x/vault/workspaces/main");
        let servers = vec!["hollow_app".to_string(), "hollow_vault".to_string()];
        let base = FingerprintInput {
            agent: AgentKind::Claude,
            model: Some("claude-opus-5-5"),
            perm: "bypassPermissions",
            cwd,
            mcp_servers: &servers,
            caps: test_caps(),
            canvases: CanvasFrequency::Helps,
            workspace: "main",
            skills: &[],
        };
        let parts = config_parts(&base);
        let stored = Some(parts.as_str());
        let object: serde_json::Map<String, Value> = serde_json::from_str(&parts).unwrap();
        assert_eq!(object.len(), PARTS.len());
        for name in PARTS {
            let hash = object[name].as_str().unwrap();
            assert_eq!(hash.len(), 16, "{name}");
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{name}");
        }
        assert!(!parts.contains("/Users/x") && !parts.contains("opus"));
        assert_eq!(changed_parts(stored, &parts), vec![UNKNOWN_PART]);
        // An acting mode flip changes nothing (HB01): same parts.
        let accept = FingerprintInput {
            perm: "acceptEdits",
            ..base.clone()
        };
        assert_eq!(config_parts(&accept), parts);

        let fewer = vec!["hollow_vault".to_string()];
        let skills = vec!["release-notes".to_string()];
        let other_cwd = Path::new("/Users/x/vault/workspaces/other");
        let one_at_a_time = [
            (
                FingerprintInput {
                    agent: AgentKind::Codex,
                    ..base.clone()
                },
                "agent",
            ),
            (
                FingerprintInput {
                    model: Some("claude-sonnet-5"),
                    ..base.clone()
                },
                "model",
            ),
            (
                FingerprintInput {
                    perm: "plan",
                    ..base.clone()
                },
                "mode",
            ),
            (
                FingerprintInput {
                    cwd: other_cwd,
                    ..base.clone()
                },
                "cwd",
            ),
            (
                FingerprintInput {
                    mcp_servers: &fewer,
                    ..base.clone()
                },
                "mcp",
            ),
            (
                FingerprintInput {
                    caps: AttachedCaps {
                        app_read_only: true,
                        ..test_caps()
                    },
                    ..base.clone()
                },
                "caps",
            ),
            (
                FingerprintInput {
                    canvases: CanvasFrequency::Off,
                    ..base.clone()
                },
                "canvases",
            ),
            (
                FingerprintInput {
                    workspace: "quick",
                    ..base.clone()
                },
                "workspace",
            ),
            (
                FingerprintInput {
                    skills: &skills,
                    ..base.clone()
                },
                "skills",
            ),
        ];
        for (changed, part) in &one_at_a_time {
            let now = config_parts(changed);
            assert_eq!(changed_parts(stored, &now), vec![*part], "{part}");
            assert_eq!(changed_parts(Some(now.as_str()), &parts), vec![*part]);
        }
        let bumped = parts_at(&base, PROMPT_CONTRACT + 1);
        assert_eq!(changed_parts(stored, &bumped), vec!["contract"]);
        // Two changes at once name both, in PARTS order.
        let two = config_parts(&FingerprintInput {
            canvases: CanvasFrequency::Off,
            model: None,
            ..base.clone()
        });
        assert_eq!(changed_parts(stored, &two), vec!["model", "canvases"]);
        // No parts stored (a stint stamped before OB03, or legacy NULL), or
        // parts that don't parse: unknown.
        for stored in [None, Some(""), Some("not json"), Some("[1]")] {
            assert_eq!(
                changed_parts(stored, &parts),
                vec![UNKNOWN_PART],
                "{stored:?}"
            );
        }
        // The reset reads the stint the plan compared: the open one once it
        // holds an id, else the last closed one (a pending reset).
        let with_parts = |harness: Option<&str>, stored: Option<&str>| OpenStint {
            config_parts: stored.map(str::to_string),
            ..open(harness, Some("v3:old")).unwrap()
        };
        let model = config_parts(&one_at_a_time[1].0);
        let held = ResumeState {
            open: Some(with_parts(Some("sess-1"), Some(model.as_str()))),
            last_closed: None,
        };
        assert_eq!(reset_parts(&held, &parts), vec!["model"]);
        let legacy = ResumeState {
            open: Some(with_parts(Some("sess-1"), None)),
            last_closed: None,
        };
        assert_eq!(reset_parts(&legacy, &parts), vec![UNKNOWN_PART]);
        let pending = ResumeState {
            open: Some(with_parts(None, None)),
            last_closed: Some(ClosedStint {
                config_parts: Some(config_parts(&one_at_a_time[5].0)),
                ..closed("sess-1", "config changed").unwrap()
            }),
        };
        assert_eq!(reset_parts(&pending, &parts), vec!["caps"]);

        // The refactor kept v3 byte-identical: OB04's canonical text.
        let canonical = format!(
            "{{\"v\":3,\"contract\":{PROMPT_CONTRACT},\"agent\":\"claude\",\"model\":\"claude-opus-5-5\",\"mode\":\"act\",\"cwd\":\"/Users/x/vault/workspaces/main\",\"mcp\":[\"hollow_app\",\"hollow_vault\"],\"caps\":[\"app\",\"contract:claude\",\"questions\",\"tasks\",\"vault\"],\"canvases\":{},\"workspace\":\"main\",\"skills\":[]}}",
            json_string(CanvasFrequency::Helps.as_str()),
        );
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        assert_eq!(
            config_fingerprint(&base),
            format!("v3:{:x}", hasher.finalize())
        );
    }

    /// HB01 (evening 10-16): a Main-agent wake runs at acceptEdits and the
    /// person's next turn at bypass. Every adapter takes the mode per
    /// process, resumes included, so a flip between two modes that both
    /// act keeps the native session; plan, which only reads, still resets.
    #[test]
    fn a_mode_flip_outside_plan_resumes_the_same_session() {
        let cwd = Path::new("/Users/x/vault/workspaces/main");
        let servers = vec!["hollow_app".to_string(), "hollow_vault".to_string()];
        let at = |perm| {
            config_fingerprint(&FingerprintInput {
                agent: AgentKind::Claude,
                model: Some("claude-opus-5-5"),
                perm,
                cwd,
                mcp_servers: &servers,
                caps: test_caps(),
                canvases: CanvasFrequency::Helps,
                workspace: "main",
                skills: &[],
            })
        };
        let bypass = at("bypassPermissions");
        for perm in ["acceptEdits", "auto", "default", ""] {
            assert_eq!(at(perm), bypass, "{perm:?}");
        }
        assert_ne!(at("plan"), bypass, "plan changes what the session may do");
        // Grok refuses to resume across its sandbox profiles, so they stay
        // apart; modes that share a profile resume.
        let grok = |perm| {
            config_fingerprint(&FingerprintInput {
                agent: AgentKind::Grok,
                model: None,
                perm,
                cwd,
                mcp_servers: &[],
                caps: AttachedCaps::default(),
                canvases: CanvasFrequency::Helps,
                workspace: "main",
                skills: &[],
            })
        };
        assert_eq!(grok("auto"), grok("bypassPermissions"));
        assert_ne!(grok("acceptEdits"), grok("bypassPermissions"));
        assert_ne!(grok("default"), grok("acceptEdits"));
        assert_ne!(grok("plan"), grok("default"));

        let stamped = |fp: &str| ResumeState {
            open: open(Some("sess-main"), Some(fp)),
            last_closed: None,
        };
        assert_eq!(
            plan_resume(Some("sess-main"), &stamped(&bypass), &at("acceptEdits")),
            ResumePlan::Resume("sess-main".into()),
            "a wake after a person's bypass turn"
        );
        assert_eq!(
            plan_resume(Some("sess-main"), &stamped(&at("acceptEdits")), &bypass),
            ResumePlan::Resume("sess-main".into()),
            "the person's turn after a wake"
        );
        assert_eq!(
            plan_resume(Some("sess-main"), &stamped(&bypass), &at("plan")),
            ResumePlan::Reset(ResetReason::ConfigChanged)
        );
    }

    /// A Main-agent-like chat's attached capabilities.
    fn test_caps() -> AttachedCaps {
        AttachedCaps {
            questions: true,
            vault: true,
            app: true,
            tasks: true,
            agent: Some(AgentKind::Claude),
            ..Default::default()
        }
    }

    /// A Claude chat in the Main agent's folder at `perm`, its fingerprint
    /// under [`PROMPT_CONTRACT`] bumped `bump` times.
    fn main_chat(perm: &str, canvases: CanvasFrequency, bump: u32) -> String {
        let servers = vec!["hollow_app".to_string(), "hollow_vault".to_string()];
        fingerprint_at(
            &FingerprintInput {
                agent: AgentKind::Claude,
                model: Some("claude-opus-5-5"),
                perm,
                cwd: Path::new("/Users/x/vault/workspaces/main"),
                mcp_servers: &servers,
                caps: test_caps(),
                canvases,
                workspace: "main",
                skills: &[],
            },
            PROMPT_CONTRACT + bump,
        )
    }

    /// OB04 (fingerprint v3): the appended prompt's text is not an input
    /// (FingerprintInput has no such field), so a stint stamped before a
    /// guidance edit resumes under the same contract; a contract bump, an
    /// agent change, a mode-class change or a Canvases change still resets.
    #[test]
    fn a_guidance_edit_resumes_and_semantic_changes_reset() {
        let stamped = |fp: &str| ResumeState {
            open: open(Some("sess-1"), Some(fp)),
            last_closed: None,
        };
        let helps = CanvasFrequency::Helps;
        let before = main_chat("bypassPermissions", helps, 0);
        // The install that changed the guidance text: same inputs, same
        // contract, same fingerprint.
        let after = main_chat("bypassPermissions", helps, 0);
        assert_eq!(
            plan_resume_with(Some("sess-1"), &stamped(&before), &after, false),
            ResumePlan::Resume("sess-1".into()),
            "different appended text, same contract"
        );
        let codex = fingerprint_at(
            &FingerprintInput {
                agent: AgentKind::Codex,
                model: Some("claude-opus-5-5"),
                perm: "bypassPermissions",
                cwd: Path::new("/Users/x/vault/workspaces/main"),
                mcp_servers: &["hollow_app".to_string(), "hollow_vault".to_string()],
                caps: test_caps(),
                canvases: CanvasFrequency::Helps,
                workspace: "main",
                skills: &[],
            },
            PROMPT_CONTRACT,
        );
        let bumped = main_chat("bypassPermissions", helps, 1);
        let off = main_chat("bypassPermissions", CanvasFrequency::Off, 0);
        for (changed, what) in [
            (bumped, "a contract bump"),
            (codex, "an agent change"),
            (main_chat("plan", helps, 0), "a mode-class change"),
            (off, "a Canvases change"),
        ] {
            // The restamp never rescues a v3 stamp.
            for restamp in [false, true] {
                assert_eq!(
                    plan_resume_with(Some("sess-1"), &stamped(&before), &changed, restamp),
                    ResumePlan::Reset(ResetReason::ConfigChanged),
                    "{what}"
                );
            }
        }
    }

    /// The previous turn of a Claude chat in the Main agent's folder, at
    /// `perm` on `model`.
    fn previous(perm: Option<&str>, model: Option<&str>) -> PreviousTurn {
        PreviousTurn {
            permission_mode: perm.map(str::to_string),
            requested: Some((model.map(str::to_string), Some("claude".to_string()))),
            session_cwd: Some("/Users/x/vault/workspaces/main".to_string()),
        }
    }

    /// OB04's version restamp: a stint v1 or v2 stamped resumes (and the
    /// caller restamps it v3) only when the agent, the previous turn's mode
    /// class, the requested model and the folder are all unchanged; Grok,
    /// a changed value or a missing one resets as before.
    #[test]
    fn an_older_stamp_resumes_only_when_nothing_else_changed() {
        let main = "/Users/x/vault/workspaces/main";
        let opus = Some("claude-opus-5-5");
        let resume = ResumePlan::Resume("sess-1".into());
        let reset = ResumePlan::Reset(ResetReason::ConfigChanged);
        // A same-class previous turn: the person's bypass, now a wake at
        // acceptEdits.
        let same = previous(Some("bypassPermissions"), opus);
        for stored in ["v1:aaaa", "v2:bbbb"] {
            let claude = open(Some("sess-1"), Some(stored)).unwrap();
            let state = ResumeState {
                open: Some(claude.clone()),
                last_closed: None,
            };
            assert_eq!(older_stamp(&state), Some(&claude));
            let plan = restamp_plan(&claude, AgentKind::Claude, opus, main, Some(&same));
            assert_eq!(plan, resume, "{stored}");
            // Without the restamp an older stamp resets, as before.
            let now = main_chat("acceptEdits", CanvasFrequency::Helps, 0);
            let plan = plan_resume_with(Some("sess-1"), &state, &now, false);
            assert_eq!(plan, reset, "{stored}");
        }
        let v2 = open(Some("sess-1"), Some("v2:bbbb")).unwrap();
        let planned = previous(Some("plan"), opus);
        let other_agent = PreviousTurn {
            requested: Some((opus.map(str::to_string), Some("codex".to_string()))),
            ..same.clone()
        };
        let no_mode = previous(None, opus);
        let no_usage = PreviousTurn {
            requested: None,
            ..same.clone()
        };
        let no_cwd = PreviousTurn {
            session_cwd: None,
            ..same.clone()
        };
        let other = "/Users/x/vault/workspaces/other";
        let sonnet = Some("claude-sonnet-5");
        for (model, cwd, prior, what) in [
            (opus, main, Some(&planned), "plan, now act"),
            (sonnet, main, Some(&same), "changed model"),
            (None, main, Some(&same), "now the default model"),
            (opus, other, Some(&same), "changed cwd"),
            (opus, main, Some(&other_agent), "another agent's turn"),
            (opus, main, None, "no previous turn"),
            (opus, main, Some(&no_mode), "no recorded mode"),
            (opus, main, Some(&no_usage), "no usage row"),
            (opus, main, Some(&no_cwd), "no folder"),
        ] {
            let plan = restamp_plan(&v2, AgentKind::Claude, model, cwd, prior);
            assert_eq!(plan, reset, "{what}");
        }
        // Another agent's stint.
        let plan = restamp_plan(&v2, AgentKind::Codex, opus, main, Some(&same));
        assert_eq!(plan, reset, "another agent's stint");
        // Grok keeps its reset-once, everything else unchanged.
        let grok = OpenStint {
            agent: "grok".into(),
            harness_session_id: Some("sess-1".into()),
            config_fingerprint: Some("v1:aaaa".into()),
            config_parts: None,
        };
        let grok_previous = PreviousTurn {
            requested: Some((None, Some("grok".to_string()))),
            ..previous(Some("acceptEdits"), None)
        };
        let plan = restamp_plan(&grok, AgentKind::Grok, None, main, Some(&grok_previous));
        assert_eq!(plan, reset, "Grok");
        // Only an older stamp: a NULL or a v3 one is never restamped.
        let now = main_chat("acceptEdits", CanvasFrequency::Helps, 0);
        let cwd = Path::new(main);
        for stored in [None, Some(now.as_str())] {
            let stint = open(Some("sess-1"), stored).unwrap();
            let claude = AgentKind::Claude;
            let ok = may_restamp(&stint, claude, "acceptEdits", opus, cwd, Some(&same));
            assert!(!ok, "{stored:?}");
            let state = ResumeState {
                open: Some(stint),
                last_closed: None,
            };
            assert_eq!(older_stamp(&state), None);
        }
    }

    /// The plan of a turn at acceptEdits on `model` in `cwd` (fingerprinted
    /// as the Main agent's Claude chat), whose open stint is `stint` and
    /// whose previous turn is `previous`, with OB04's restamp check.
    fn restamp_plan(
        stint: &OpenStint,
        agent: AgentKind,
        model: Option<&str>,
        cwd: &str,
        previous: Option<&PreviousTurn>,
    ) -> ResumePlan {
        let now = main_chat("acceptEdits", CanvasFrequency::Helps, 0);
        let restamp = may_restamp(stint, agent, "acceptEdits", model, Path::new(cwd), previous);
        let state = ResumeState {
            open: Some(stint.clone()),
            last_closed: None,
        };
        plan_resume_with(Some("sess-1"), &state, &now, restamp)
    }

    fn open(harness: Option<&str>, fp: Option<&str>) -> Option<OpenStint> {
        Some(OpenStint {
            agent: "claude".into(),
            harness_session_id: harness.map(str::to_string),
            config_fingerprint: fp.map(str::to_string),
            config_parts: None,
        })
    }

    fn closed(harness: &str, reason: &str) -> Option<ClosedStint> {
        Some(ClosedStint {
            harness_session_id: Some(harness.to_string()),
            reason: Some(reason.to_string()),
            config_parts: None,
        })
    }

    #[test]
    fn plan_resume_rules() {
        let resume = ResumePlan::Resume("sess-1".into());
        let state = |open, last_closed| ResumeState { open, last_closed };
        assert_eq!(
            plan_resume(None, &state(open(Some("sess-1"), Some("fp")), None), "fp"),
            ResumePlan::New
        );
        assert_eq!(
            plan_resume(Some("  "), &ResumeState::default(), "fp"),
            ResumePlan::New
        );
        assert_eq!(
            plan_resume(Some("sess-1"), &ResumeState::default(), "fp"),
            resume,
            "no open stint"
        );
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(Some("sess-1"), None), None),
                "fp"
            ),
            resume,
            "NULL is unknown"
        );
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(Some("sess-1"), Some("fp")), None),
                "fp"
            ),
            resume,
            "same config"
        );
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(Some("other"), Some("old")), None),
                "fp"
            ),
            resume,
            "another id"
        );
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(Some("sess-1"), Some("old")), None),
                "fp"
            ),
            ResumePlan::Reset(ResetReason::ConfigChanged)
        );
        // A pending reset: the fresh attempt never announced its id.
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(None, None), closed("sess-1", "config changed")),
                "fp"
            ),
            ResumePlan::Reset(ResetReason::ConfigChanged)
        );
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(None, None), closed("sess-1", "resume failed")),
                "fp"
            ),
            ResumePlan::Reset(ResetReason::ResumeFailed)
        );
        // Not pending: another closed id, another reason, or an announced id.
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(None, None), closed("sess-0", "resume failed")),
                "fp"
            ),
            resume
        );
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(None, None), closed("sess-1", "agent changed")),
                "fp"
            ),
            resume
        );
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(
                    open(Some("sess-2"), Some("fp")),
                    closed("sess-1", "resume failed")
                ),
                "fp"
            ),
            resume
        );
        // A move closed the stint holding the requested id: a fresh session.
        assert_eq!(
            plan_resume(
                Some("sess-1"),
                &state(open(None, None), closed("sess-1", "moved")),
                "fp"
            ),
            ResumePlan::Reset(ResetReason::Moved)
        );
        assert_eq!(ResetReason::ConfigChanged.wire(), "configChanged");
        assert_eq!(ResetReason::ResumeFailed.as_str(), "resume failed");
        assert_eq!(
            (ResetReason::Moved.as_str(), ResetReason::Moved.wire()),
            ("moved", "moved")
        );
    }

    /// Review finding A·R10: a requested id that could parse as a flag is
    /// never resumed, even when the open stint holds it with the same
    /// fingerprint; the turn starts fresh with a visible `invalidId` note.
    #[test]
    fn a_flag_shaped_resume_id_starts_fresh() {
        let state = |open, last_closed| ResumeState { open, last_closed };
        for bad in ["-x", "--dangerously-skip-permissions", " -x ", "sess 1"] {
            for resume_state in [
                ResumeState::default(),
                state(open(Some(bad.trim()), Some("fp")), None),
            ] {
                assert_eq!(
                    plan_resume(Some(bad), &resume_state, "fp"),
                    ResumePlan::Reset(ResetReason::InvalidId),
                    "{bad:?}"
                );
            }
        }
        let reason = ResetReason::InvalidId;
        assert_eq!(
            (reason.as_str(), reason.wire()),
            ("invalid session id", "invalidId")
        );
        assert_eq!(
            ResetReason::from_stint_reason(reason.as_str()),
            Some(reason)
        );
    }

    #[test]
    fn probation_holds_until_content_and_discard_drops() {
        let mut open = Probation::new(false);
        assert!(!open.holding());
        assert_eq!(
            open.push(Held::HarnessId("a".into())),
            Some(Held::HarnessId("a".into())),
            "passes through"
        );

        let mut probation = Probation::new(true);
        assert!(probation.holding());
        assert_eq!(probation.push(Held::HarnessId("id-1".into())), None);
        assert_eq!(
            probation.push(Held::Store {
                kind: "session",
                payload: "id-1".into()
            }),
            None
        );
        assert_eq!(probation.push(Held::Emit(json!({"type": "session"}))), None);
        assert_eq!(probation.held().len(), 3);
        let released = probation.commit();
        assert_eq!(
            released,
            vec![
                Held::HarnessId("id-1".into()),
                Held::Store {
                    kind: "session",
                    payload: "id-1".into()
                },
                Held::Emit(json!({"type": "session"})),
            ],
            "in order"
        );
        assert!(!probation.holding());
        assert_eq!(
            probation.push(Held::Emit(json!({"type": "later"}))),
            Some(Held::Emit(json!({"type": "later"})))
        );

        let mut failed = Probation::new(true);
        let _ = failed.push(Held::Emit(json!({"type": "result"})));
        failed.discard();
        assert!(failed.held().is_empty());
        assert!(failed.commit().is_empty());
    }

    // ── Quota failover (R1) ──

    fn facts(kind: crate::result_kind::ResultKind) -> FailoverFacts<'static> {
        FailoverFacts {
            enabled: true,
            account: Some("acct-admin"),
            kind,
            outage: false,
            session: Some("sess-1"),
            stream: Exhaustion::Exhausted,
            probe: None,
            tried: &[],
        }
    }

    fn tech() -> Option<String> {
        Some("acct-tech".into())
    }

    /// A Transient or quota exit whose own events put the account over the
    /// ceiling can continue through every untried eligible account.
    #[test]
    fn an_exhausted_account_switches_to_an_untried_account() {
        use crate::result_kind::ResultKind;
        for kind in [ResultKind::Transient, ResultKind::ProviderQuota] {
            assert_eq!(
                decide_failover(&facts(kind), tech),
                Failover::Switch {
                    to: "acct-tech".into()
                },
                "{kind:?}"
            );
        }
        let tried = vec!["acct-admin".into(), "acct-tech".into()];
        let second = FailoverFacts {
            account: Some("acct-tech"),
            tried: &tried,
            ..facts(ResultKind::Transient)
        };
        assert_eq!(
            decide_failover(&second, || Some("acct-third".into())),
            Failover::Switch {
                to: "acct-third".into()
            }
        );
        for previous in &tried {
            assert_eq!(
                decide_failover(&second, || Some(previous.clone())),
                Failover::Stop(FailoverStop::NoneEligible)
            );
        }
    }

    /// A 529, an overload or a network failure never switches, whatever the
    /// account's state; nor does any other kind, a login run, a run with no
    /// native session, or the setting switched off.
    #[test]
    fn outages_and_other_failures_never_switch() {
        use crate::result_kind::ResultKind;
        let never = || -> Option<String> { panic!("never picks") };
        let outage = FailoverFacts {
            outage: true,
            ..facts(ResultKind::Transient)
        };
        assert_eq!(decide_failover(&outage, never), Failover::No);
        for kind in [
            ResultKind::Error,
            ResultKind::Auth,
            ResultKind::ResumeFailed,
            ResultKind::Refusal,
            ResultKind::Success,
        ] {
            assert_eq!(
                decide_failover(&facts(kind), never),
                Failover::No,
                "{kind:?}"
            );
        }
        let login = FailoverFacts {
            account: None,
            ..facts(ResultKind::ProviderQuota)
        };
        assert_eq!(decide_failover(&login, never), Failover::No);
        let sessionless = FailoverFacts {
            session: None,
            ..facts(ResultKind::ProviderQuota)
        };
        assert_eq!(decide_failover(&sessionless, never), Failover::No);
        let off = FailoverFacts {
            enabled: false,
            ..facts(ResultKind::ProviderQuota)
        };
        assert_eq!(decide_failover(&off, never), Failover::No);
    }

    /// A direct quota rejection outranks a stale meter or failed probe.
    #[test]
    fn explicit_quota_rejection_does_not_depend_on_a_usage_probe() {
        use crate::result_kind::ResultKind;
        for stream in [Exhaustion::Unknown, Exhaustion::NotExhausted] {
            for probe in [
                None,
                Some(Exhaustion::Unknown),
                Some(Exhaustion::NotExhausted),
            ] {
                let rejected = FailoverFacts {
                    stream,
                    probe,
                    ..facts(ResultKind::ProviderQuota)
                };
                assert_eq!(
                    decide_failover(&rejected, tech),
                    Failover::Switch {
                        to: "acct-tech".into()
                    }
                );
            }
        }
    }

    #[test]
    fn quota_rejection_keeps_eligibility_and_per_turn_loop_guards() {
        use crate::result_kind::ResultKind;
        let rejected = FailoverFacts {
            stream: Exhaustion::Unknown,
            ..facts(ResultKind::ProviderQuota)
        };
        assert_eq!(
            decide_failover(&rejected, || None),
            Failover::Stop(FailoverStop::NoneEligible)
        );
        assert_eq!(
            decide_failover(&rejected, || Some("acct-admin".into())),
            Failover::Stop(FailoverStop::NoneEligible)
        );
        // Another job's own facts do not inherit tried accounts.
        assert!(matches!(
            decide_failover(&rejected, tech),
            Failover::Switch { .. }
        ));
        for denied in [
            FailoverFacts {
                enabled: false,
                ..rejected.clone()
            },
            FailoverFacts {
                account: None,
                ..rejected.clone()
            },
            FailoverFacts {
                session: None,
                ..rejected.clone()
            },
            FailoverFacts {
                outage: true,
                ..rejected.clone()
            },
            FailoverFacts {
                kind: ResultKind::Auth,
                ..rejected.clone()
            },
        ] {
            assert_eq!(
                decide_failover(&denied, || panic!("not eligible")),
                Failover::No
            );
        }
    }

    #[test]
    fn background_limit_explanation_never_authorizes_a_switch() {
        assert!(quota_background_notice(true, true, true, true).is_some());
        for facts in [
            (false, true, true, true),
            (true, false, true, true),
            (true, true, false, true),
            (true, true, true, false),
        ] {
            assert!(quota_background_notice(facts.0, facts.1, facts.2, facts.3).is_none());
        }
    }

    /// A generic rate error still needs stream evidence or one fresh probe.
    #[test]
    fn one_probe_decides_when_the_stream_did_not() {
        use crate::result_kind::ResultKind;
        for stream in [Exhaustion::NotExhausted, Exhaustion::Unknown] {
            let unsure = FailoverFacts {
                stream,
                ..facts(ResultKind::Transient)
            };
            assert_eq!(decide_failover(&unsure, tech), Failover::Probe);
            let full = FailoverFacts {
                probe: Some(Exhaustion::Exhausted),
                ..unsure.clone()
            };
            assert_eq!(
                decide_failover(&full, tech),
                Failover::Switch {
                    to: "acct-tech".into()
                }
            );
            for probe in [Exhaustion::NotExhausted, Exhaustion::Unknown] {
                let fine = FailoverFacts {
                    probe: Some(probe),
                    ..unsure.clone()
                };
                assert_eq!(decide_failover(&fine, tech), Failover::No, "{probe:?}");
            }
        }
        assert_eq!(Exhaustion::from(Some(true)), Exhaustion::Exhausted);
        assert_eq!(Exhaustion::from(None), Exhaustion::Unknown);
    }

    /// No eligible account (or only the exhausted one) is a clear stop,
    /// with the soonest reset when it is known.
    #[test]
    fn no_eligible_account_stops_clearly() {
        use crate::result_kind::ResultKind;
        assert_eq!(
            decide_failover(&facts(ResultKind::ProviderQuota), || None),
            Failover::Stop(FailoverStop::NoneEligible)
        );
        assert_eq!(
            decide_failover(&facts(ResultKind::ProviderQuota), || Some(
                "acct-admin".into()
            )),
            Failover::Stop(FailoverStop::NoneEligible),
            "never back onto the exhausted account"
        );
        assert_eq!(
            failover_stop_message(FailoverStop::NoneEligible, Some("01:20")),
            "Waiting for Claude until 01:20: no other eligible Claude account has room. Check Accounts before retrying."
        );
        assert_eq!(
            failover_stop_message(FailoverStop::NoneEligible, None),
            "Waiting for Claude until its usage resets: no other eligible Claude account has room. Check Accounts before retrying."
        );
    }

    #[test]
    fn a_reset_reads_as_a_clock_time_or_a_weekday() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-29T19:55:00-04:00").unwrap();
        let tonight = chrono::DateTime::parse_from_rfc3339("2026-09-29T23:20:00-04:00")
            .unwrap()
            .timestamp();
        let friday = chrono::DateTime::parse_from_rfc3339("2026-10-02T09:00:00-04:00")
            .unwrap()
            .timestamp();
        assert_eq!(reset_label(tonight, &now).as_deref(), Some("23:20"));
        assert_eq!(reset_label(friday, &now).as_deref(), Some("Fri 09:00"));
    }

    /// Overnight 9 A9: what a Main-agent turn that may rest plans after
    /// `away` minutes at `tokens`, its open stint holding the session.
    fn rest_plan(may: bool, away: Option<i64>, tokens: Option<u64>) -> ResumePlan {
        let state = ResumeState {
            open: open(Some("sess-1"), Some("fp")),
            last_closed: None,
        };
        let plan = plan_resume(Some("sess-1"), &state, "fp");
        with_rest(plan, rests(may, away, tokens))
    }

    #[test]
    fn a_cold_large_session_rests_instead_of_resuming() {
        let may = may_rest(
            AgentKind::Claude,
            "acceptEdits",
            false,
            false,
            0,
            true,
            true,
        );
        assert!(may);
        assert_eq!(
            rest_plan(may, Some(180), Some(714_000)),
            ResumePlan::Reset(ResetReason::Rested {
                away_minutes: 180,
                tokens: 714_000
            })
        );
        let reason = ResetReason::Rested {
            away_minutes: 180,
            tokens: 714_000,
        };
        assert_eq!((reason.as_str(), reason.wire()), ("rested", "rested"));
        assert_eq!(
            rested_text(180, 714_000).as_deref(),
            Some("away 3 h at 714K")
        );
        assert_eq!(
            rested_text(75, 350_000).as_deref(),
            Some("away 75 min at 350K")
        );
        assert_eq!(
            rested_text(4_000, 600_400).as_deref(),
            Some("away 2 days at 600K")
        );
        // A fresh attempt that never announced its id rests again, without figures.
        let pending = ResumeState {
            open: open(None, None),
            last_closed: closed("sess-1", "rested"),
        };
        let again = plan_resume(Some("sess-1"), &pending, "fp");
        assert_eq!(
            again,
            ResumePlan::Reset(ResetReason::Rested {
                away_minutes: 0,
                tokens: 0
            })
        );
        assert_eq!(rested_text(0, 0), None);
        // Only a resume rests: a new session or another reset stays as planned.
        let rest = rests(true, Some(180), Some(714_000));
        assert_eq!(with_rest(ResumePlan::New, rest), ResumePlan::New);
        assert_eq!(
            with_rest(ResumePlan::Reset(ResetReason::ConfigChanged), rest),
            ResumePlan::Reset(ResetReason::ConfigChanged)
        );
    }

    #[test]
    fn a_warm_or_small_session_still_resumes() {
        let resume = ResumePlan::Resume("sess-1".into());
        assert_eq!(
            rest_plan(true, Some(60), Some(714_000)),
            resume,
            "an hour exactly"
        );
        assert_eq!(rest_plan(true, Some(30), Some(714_000)), resume, "warm");
        assert_eq!(rest_plan(true, Some(180), Some(349_999)), resume, "small");
        assert_eq!(rest_plan(true, None, Some(714_000)), resume, "no end time");
        assert_eq!(rest_plan(true, Some(180), None), resume, "no figure");
    }

    #[test]
    fn plan_mode_never_rests() {
        assert!(!may_rest(
            AgentKind::Claude,
            "plan",
            false,
            false,
            0,
            true,
            true
        ));
        assert!(
            !may_rest(AgentKind::Claude, "acceptEdits", true, false, 0, true, true),
            "bare"
        );
        assert!(
            !may_rest(AgentKind::Claude, "acceptEdits", false, true, 0, true, true),
            "unattended: a wake or check-in never expires open cards"
        );
        assert!(
            !may_rest(AgentKind::Codex, "acceptEdits", false, false, 0, true, true),
            "Claude only"
        );
        assert!(
            !may_rest(
                AgentKind::Claude,
                "acceptEdits",
                false,
                false,
                0,
                false,
                true
            ),
            "COO only"
        );
        assert_eq!(
            rest_plan(false, Some(180), Some(714_000)),
            ResumePlan::Resume("sess-1".into())
        );
    }

    #[test]
    fn the_setting_off_never_rests() {
        let may = may_rest(
            AgentKind::Claude,
            "bypassPermissions",
            false,
            false,
            0,
            true,
            false,
        );
        assert!(!may);
        assert_eq!(
            rest_plan(may, Some(600), Some(738_000)),
            ResumePlan::Resume("sess-1".into())
        );
    }

    /// RF-810-F1 (second-pass review R2): a turn another agent sent
    /// (`send_to_session`, depth above 0) never rests, whatever else holds:
    /// a rest expires the conversation's open cards, and only the person's
    /// own return should do that.
    #[test]
    fn an_agent_sent_turn_never_rests() {
        for depth in [1, 2] {
            let may = may_rest(
                AgentKind::Claude,
                "acceptEdits",
                false,
                false,
                depth,
                true,
                true,
            );
            assert!(!may, "depth {depth}");
            assert_eq!(
                rest_plan(may, Some(180), Some(714_000)),
                ResumePlan::Resume("sess-1".into())
            );
        }
        assert!(
            may_rest(
                AgentKind::Claude,
                "acceptEdits",
                false,
                false,
                0,
                true,
                true
            ),
            "the person's own turn still may"
        );
    }
}
