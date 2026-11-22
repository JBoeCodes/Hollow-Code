//! Self-hosted models (2026-11-03, brief `hollow-qwen-agent-brief.md`): a
//! model Alex runs on his own GPU, driven through the Claude harness
//! (an `anthropic` entry) or, since Q9, the Codex harness (an `openai`
//! entry). Qwen3.8 27B on Cloud Run is the first entry; nothing here is
//! Qwen-specific.
//!
//! Q1, credential isolation: a run on a self-hosted model never gets an
//! Anthropic credential. [`apply_env`] strips every `ANTHROPIC_*` variable
//! and Claude's OAuth token from the command and points Claude Code at
//! Hollow's loopback forwarder with a per-run key, so the agent never sees
//! the endpoint's own token (a Google identity token for Qwen) and the
//! person's Claude login never reaches a third-party endpoint. The account
//! routing in `agent::build_with_codex_servers` is skipped for such a run.
//!
//! Q5, the run path: the Claude model menu lists each enabled registry entry
//! as `self-hosted:<id>` ([`MODEL_PREFIX`], `agent_models`), and a chat turn
//! on one resolves it ([`turn_route`]) to a [`RunTarget`] on the entry's
//! forwarder with a key minted for that turn, revoked when the turn ends
//! ([`TurnRun`]). Anything else that is given such a model id refuses it
//! (`agent::validate_run_spec`, [`chats_only`]).
//!
//! Q7, Runs on: such a turn runs on this Mac whatever Runs on says
//! (`remote_session::turn_place`), and its first line says why
//! ([`runner_note`]).
//! Q10, background jobs: `self_hosted_jobs` routes Hollow's own model calls
//! (extraction, titles, learning, meeting recaps and prep packs, the
//! About-you scan, the heartbeat) to an entry per job kind, with a key
//! minted per job ([`run_for`]).
//!
//! Q9, Codex: an `openai` entry's turn runs Codex with a Hollow-owned
//! `CODEX_HOME` per entry ([`codex_home_for`], no `auth.json`, never the
//! person's `~/.codex`), a custom provider pointed at the same forwarder
//! ([`codex_overrides`]) and the run key in [`CODEX_KEY_ENV`]
//! ([`apply_codex_env`]), so no ChatGPT login or connector tool reaches the
//! endpoint (q9-findings: a signed-in home sent 728 connector tools).
use crate::self_hosted_forwarder::Forwarder;
use crate::self_hosted_registry::SelfHostedEntry;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

/// Claude Code's own variables a self-hosted run sets. Everything named
/// `ANTHROPIC_*` that the app inherited is removed first ([`apply_env`]).
pub const BASE_URL_ENV: &str = "ANTHROPIC_BASE_URL";
pub const AUTH_TOKEN_ENV: &str = "ANTHROPIC_AUTH_TOKEN";
pub const MODEL_ENV: &str = "ANTHROPIC_MODEL";
/// The aliases Claude Code resolves for subagents and its own small calls
/// (`haiku`, `sonnet`, `opus`): all go to the served model, never to a
/// model the endpoint doesn't have.
pub const ALIAS_MODEL_ENVS: [&str; 3] = [
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
];
/// No telemetry, error reports, update checks or other calls that would go
/// to Anthropic beside the model calls.
pub const NONESSENTIAL_ENV: &str = "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC";
/// Claude Code's request timeout. A cold start is 4 to 6 minutes (Qwen on
/// Cloud Run: 272 s in Q0), so the first call must be allowed to wait.
pub const API_TIMEOUT_ENV: &str = "API_TIMEOUT_MS";
pub const API_TIMEOUT_MS: u32 = 900_000;
/// Tool search off (Q0, 2026-11-03): with it on, Claude Code defers MCP
/// tools and answers `ToolSearch` with `tool_reference` blocks, which vLLM
/// refuses ("Unexpected item type in content"). Off, every tool goes in
/// full on the first call (about 52K tokens of a 131K window for a Hollow
/// chat turn), and no call fails.
pub const TOOL_SEARCH_ENV: &str = "ENABLE_TOOL_SEARCH";
/// Q3: the endpoint's real context window, which Claude Code reads for a
/// model id it doesn't recognise and that doesn't start with `claude-`
/// (otherwise it assumes 200K and warns; model-config doc, "Correct the
/// window for a gateway or custom model ID").
pub const MAX_CONTEXT_ENV: &str = "CLAUDE_CODE_MAX_CONTEXT_TOKENS";
/// Q3: the answer budget (`max_tokens` of every call).
pub const MAX_OUTPUT_ENV: &str = "CLAUDE_CODE_MAX_OUTPUT_TOKENS";
/// Q3: Claude Code's own floor for `CLAUDE_CODE_AUTO_COMPACT_WINDOW`: a
/// smaller value is read as 100,000 with a warning.
pub const COMPACT_FLOOR: u32 = 100_000;
/// Q3: the answer budget a run wants (Claude Code's own default for an
/// unknown model, Q0), when the window leaves room above [`COMPACT_FLOOR`].
pub const ANSWER_BUDGET: u32 = 32_000;
/// Q3: the least answer budget a run gets.
pub const MIN_ANSWER: u32 = 8_192;
/// Q3: tokens of a first call that are Claude Code's own (its system
/// prompt, the tool schemas, the CLAUDE.md files and memory it adds), not
/// Hollow's: about 42K in Q0's Hollow turn, with room.
pub const HARNESS_RESERVE: u32 = 45_000;
/// Q3: characters per token used to size a context pack: low, so the
/// estimate errs towards cutting (Q0: about 3.8 for a Hollow turn).
pub const CHARS_PER_TOKEN: usize = 3;

/// Claude's OAuth token for an added account (`claude_accounts::TOKEN_ENV`).
pub const OAUTH_TOKEN_ENV: &str = crate::claude_accounts::TOKEN_ENV;
/// Claude Code's token-descriptor variable: Hollow never sets it, but a
/// self-hosted run must not inherit or be pointed at one.
pub const OAUTH_TOKEN_FD_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR";

/// Q5 review: marks a self-hosted run's process tree with its entry id, so
/// what it starts (a teammate through `hollow team-mcp`) knows it runs
/// inside one ([`inherited_from`]).
pub const MARKER_ENV: &str = "HOLLOW_SELF_HOSTED";

/// Prefixes of variables removed from a self-hosted run's command, inherited
/// or set: Anthropic's own and Claude Code's provider switches
/// (`CLAUDE_CODE_USE_BEDROCK`, `_VERTEX`, `_FOUNDRY` and any later one).
pub const REMOVED_PREFIXES: [&str; 2] = ["ANTHROPIC_", "CLAUDE_CODE_USE_"];

/// Variables removed beside [`REMOVED_PREFIXES`]: credentials that would
/// send the run, or the person's login, somewhere else.
pub const REMOVED_ENVS: [&str; 4] = [
    OAUTH_TOKEN_ENV,
    OAUTH_TOKEN_FD_ENV,
    "ANTHROPIC_API_KEY",
    "AWS_BEARER_TOKEN_BEDROCK",
];

/// Loopback hosts added to `NO_PROXY`/`no_proxy`, so an inherited
/// `HTTP_PROXY` never carries the run key off this Mac.
pub const NO_PROXY_HOSTS: &str = "127.0.0.1,localhost,::1,[::1]";

/// Q9: the variable a self-hosted Codex run's key rides in. Codex's custom
/// provider reads it (`env_key`) and sends it as `Authorization: Bearer`.
/// The commands Codex runs don't get it: its shell environment policy keeps
/// its default excludes (any name containing `KEY`) and excludes this name
/// besides ([`codex_overrides`]; checked with Codex 0.166.0). Like Claude's
/// run key, it is readable by the person's own processes while the run lives
/// (macOS `ps -E`), and the forwarder revokes it when the turn ends.
pub const CODEX_KEY_ENV: &str = "HOLLOW_SELF_HOSTED_KEY";
/// Q9: the Codex provider id Hollow sets for a self-hosted run.
pub const CODEX_PROVIDER_ID: &str = "hollow-self-hosted";
/// Q9: the folder in Hollow's app data that holds one `CODEX_HOME` per
/// `openai` entry ([`codex_home_for`]).
pub const CODEX_HOMES_DIR: &str = "self-hosted-codex";
/// Q9: how long Codex waits on a quiet stream, the same 15 minutes Claude
/// Code gets ([`API_TIMEOUT_MS`]): a cold start is 4 to 6 minutes.
pub const CODEX_STREAM_IDLE_MS: u32 = API_TIMEOUT_MS;
/// Q9: prefixes removed from a self-hosted Codex run, inherited or set:
/// OpenAI's and Codex's own (`OPENAI_API_KEY`, `OPENAI_BASE_URL`,
/// `CODEX_API_KEY`, `CODEX_HOME`, `CODEX_ACCESS_TOKEN`, ...), Azure
/// OpenAI's, and Anthropic's and Claude Code's provider switches
/// ([`REMOVED_PREFIXES`]). `CODEX_HOME` is set again to the entry's own.
pub const CODEX_REMOVED_PREFIXES: [&str; 5] = [
    "OPENAI_",
    "CODEX_",
    "AZURE_OPENAI_",
    "ANTHROPIC_",
    "CLAUDE_CODE_USE_",
];

/// What a settings file could set to undo the env above: every value
/// [`settings_json`] pins or blanks for Claude Code.
pub const BLANKED_IN_SETTINGS: [&str; 8] = [
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_CUSTOM_HEADERS",
    OAUTH_TOKEN_ENV,
    OAUTH_TOKEN_FD_ENV,
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "AWS_BEARER_TOKEN_BEDROCK",
];

/// A self-hosted run's endpoint, as the command builder needs it. Built
/// per chat turn ([`turn_run`], Q5) from a registry entry and the
/// forwarder (Q2); carried in `agent::RunExtras::self_hosted`.
#[derive(Clone, PartialEq, Eq)]
pub struct RunTarget {
    /// The registry entry's id (`qwen`).
    pub entry: String,
    /// The entry's display name (`Qwen3.8 27B`).
    pub name: String,
    /// The forwarder's loopback base URL (`http://127.0.0.1:<port>`).
    pub base_url: String,
    /// The per-run key the forwarder checks. Never logged ([`RunKey`]'s
    /// `Debug` is redacted), never on argv, zeroed when dropped.
    pub run_key: RunKey,
    /// The model id the endpoint serves (`qwen3.8-27b`).
    pub served_model: String,
    /// The endpoint's context window in tokens (131,072 for Qwen).
    pub context_window: u32,
    /// The effort levels the endpoint takes, lowest first (`low`,
    /// `medium`, `xhigh` for Qwen).
    pub effort_levels: Vec<String>,
    /// The entry's API style (`self_hosted_registry::API_ANTHROPIC` or
    /// `API_OPENAI`), which decides the harness ([`harness`], Q9).
    pub api_style: String,
    /// Q9: the Hollow-owned `CODEX_HOME` of an `openai` entry
    /// ([`codex_home_for`]); `None` for an `anthropic` one.
    pub codex_home: Option<PathBuf>,
}

/// A per-run key: printed redacted, zeroed on drop (like
/// `claude_accounts::Secret`). `claude_runner::redact` also masks its
/// `hsr_` shape in any text.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct RunKey(Vec<u8>);

impl RunKey {
    pub fn new(key: String) -> Self {
        RunKey(key.into_bytes())
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}

impl From<&str> for RunKey {
    fn from(key: &str) -> Self {
        RunKey::new(key.to_string())
    }
}

impl fmt::Debug for RunKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RunKey(<redacted>)")
    }
}

impl Drop for RunKey {
    fn drop(&mut self) {
        crate::claude_quota::zero(&mut self.0);
    }
}

impl RunTarget {
    /// Q3: the window Claude Code compacts against: the window less the
    /// answer budget, at least [`COMPACT_FLOOR`] (Claude Code's own floor)
    /// and leaving at least [`MIN_ANSWER`]. Qwen's 131,072 gives 100,000.
    pub fn compact_window(&self) -> u32 {
        let window = self.context_window;
        window
            .saturating_sub(ANSWER_BUDGET)
            .max(COMPACT_FLOOR)
            .min(window.saturating_sub(MIN_ANSWER))
    }

    /// Q3: the answer budget: the window less [`Self::compact_window`], so
    /// a full prompt plus a full answer is exactly the window (31,072 for
    /// Qwen).
    pub fn answer_budget(&self) -> u32 {
        self.context_window.saturating_sub(self.compact_window())
    }

    /// Q3: the effort the run sends: the saved level mapped to the nearest
    /// one the endpoint takes, `medium` when none is saved, ultracode read
    /// as `max`. For Qwen (`low`, `medium`, `xhigh`): low to low, medium
    /// and high to medium (a tie goes to the lower level), xhigh and max to
    /// xhigh. A level the endpoint lacks is never sent.
    pub fn effort(&self, requested: Option<&str>) -> Option<String> {
        // Q9: Codex has no `max`, so an `openai` entry never sends it.
        let codex = self.agent() == Some(crate::agent::AgentKind::Codex);
        let levels: Vec<&str> = self
            .effort_levels
            .iter()
            .map(String::as_str)
            .filter(|l| !codex || crate::self_hosted_registry::CODEX_LEVELS.contains(l))
            .collect();
        let level = match requested.and_then(crate::effort::canonical) {
            Some(crate::effort::ULTRACODE) => "max",
            Some(level) => level,
            None => DEFAULT_EFFORT,
        };
        crate::effort::nearest(level, &levels).map(str::to_string)
    }

    /// Q3: the most characters Hollow's own prompt (the message plus the
    /// appended system prompt) may hold: the compact window less Claude
    /// Code's share ([`HARNESS_RESERVE`]), in characters.
    pub fn pack_cap_chars(&self) -> usize {
        (self.compact_window().saturating_sub(HARNESS_RESERVE) as usize) * CHARS_PER_TOKEN
    }

    /// Q9: the harness the target runs on ([`harness`] of its style).
    pub fn agent(&self) -> Option<crate::agent::AgentKind> {
        harness(&self.api_style)
    }
}

/// Q9: the harness an API style runs on: Claude for `anthropic`, Codex for
/// `openai`, none for anything else.
pub fn harness(api_style: &str) -> Option<crate::agent::AgentKind> {
    match api_style.trim() {
        crate::self_hosted_registry::API_ANTHROPIC => Some(crate::agent::AgentKind::Claude),
        crate::self_hosted_registry::API_OPENAI => Some(crate::agent::AgentKind::Codex),
        _ => None,
    }
}

/// Q9: the API style's name in a sentence.
fn style_name(api_style: &str) -> &'static str {
    match harness(api_style) {
        Some(crate::agent::AgentKind::Codex) => "OpenAI's Responses API",
        _ => "Anthropic's Messages API",
    }
}

/// Q3: the effort a self-hosted run sends when none is saved.
pub const DEFAULT_EFFORT: &str = "medium";

/// Q3: the least of a prompt's end that a cut always keeps (the person's
/// message), even when the cap is smaller.
pub const MIN_TAIL: usize = 2_000;
/// Q3: how far a cut moves to fall on a line end.
const SNAP: usize = 2_000;

/// Q3: `prompt` cut to fit `target` beside an appended prompt of
/// `append_chars`: the head and the tail (where the person's message is)
/// stay, the middle goes, and a note in its place says how much and why.
/// Unchanged (and `None`) when it fits. The tail is at least [`MIN_TAIL`]
/// bytes, so the request survives even a cap too small for it; a cut moves
/// to a line end when one is near. The result, note included, is within the
/// cap whenever the cap holds [`MIN_TAIL`] and the note.
pub fn fit_pack(prompt: &str, append_chars: usize, target: &RunTarget) -> (String, Option<String>) {
    let cap = target.pack_cap_chars().saturating_sub(append_chars);
    if prompt.len() <= cap {
        return (prompt.to_string(), None);
    }
    let note_for = |cut: usize| {
        format!(
            "[Hollow: {cut} characters of this turn's context were left out here so the turn fits {}'s {}-token window.]",
            target.name, target.context_window
        )
    };
    // The longest the note can be, plus its line end.
    let note_room = note_for(prompt.len()).len() + 1;
    let keep = cap.saturating_sub(note_room);
    let tail_len = (keep * 2 / 5).max(MIN_TAIL).min(prompt.len());
    let head_len = keep.saturating_sub(tail_len);
    let mut head_end = floor_char(prompt, head_len);
    if let Some(nl) = prompt[..head_end].rfind('\n') {
        if head_end - nl <= SNAP {
            head_end = nl + 1;
        }
    }
    let mut tail_start = ceil_char(prompt, prompt.len() - tail_len);
    if let Some(nl) = prompt[tail_start..].find('\n') {
        if nl < SNAP && tail_start + nl + 1 < prompt.len() {
            tail_start += nl + 1;
        }
    }
    let tail_start = tail_start.max(head_end);
    let cut = prompt[head_end..tail_start].chars().count();
    let note = note_for(cut);
    let fitted = format!("{}{note}\n{}", &prompt[..head_end], &prompt[tail_start..]);
    (fitted, Some(note))
}

fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

// ── What Hollow knows about each endpoint (Q3) ───────────────────────────

/// What Hollow knows about a served id: its window.
#[derive(Debug, Clone, Copy, Default)]
struct Known {
    window: u64,
}

fn known() -> &'static std::sync::Mutex<std::collections::HashMap<String, Known>> {
    static KNOWN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Known>>> =
        std::sync::OnceLock::new();
    KNOWN.get_or_init(Default::default)
}

/// Q9 delta review: the served ids of the `openai` entries saved in the
/// app's registry now (switched on or not, each passing the checks), which
/// a Codex run refuses without its target ([`needs_target_on`]). Replaced
/// whole from the registry at startup and on every change the app's
/// registry makes ([`registry_changed`]), so an edited, restyled or removed
/// entry's old id stops being refused at once.
fn codex_guard() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static IDS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    IDS.get_or_init(Default::default)
}

/// Q3: remember an endpoint's served model id, window and effort levels,
/// so the context gauge measures against its window
/// (`context_gauge::context_window`) and effort resolution for that id
/// never sends a level it lacks (`effort::remember`, under the id as
/// given and lowercased). A chat turn on a self-hosted target calls it
/// before it registers with the gauge (Q3); Q5 also calls it for each
/// enabled registry entry. Q4 refuses served ids that are Claude's own.
/// Q9: runs call [`remember_for`] with their harness; this Claude form
/// stays for Q3's tests (hence the allow).
#[cfg_attr(not(test), allow(dead_code))]
pub fn remember(served_model: &str, context_window: u32, effort_levels: &[String]) {
    remember_for(
        crate::agent::AgentKind::Claude,
        served_model,
        context_window,
        effort_levels,
    );
}

/// Q9: [`remember`] for the harness `kind` an entry runs on: the window
/// (any harness) and the effort levels for Claude (`effort::resolve` reads
/// them; a Codex run's effort comes from its target, `RunTarget::effort`).
/// The Codex guard follows the saved registry instead ([`codex_guard`]).
pub fn remember_for(
    kind: crate::agent::AgentKind,
    served_model: &str,
    context_window: u32,
    effort_levels: &[String],
) {
    let id = served_model.trim().to_ascii_lowercase();
    if id.is_empty() {
        return;
    }
    if let Ok(mut map) = known().lock() {
        map.entry(id).or_default().window = u64::from(context_window);
    }
    if kind != crate::agent::AgentKind::Claude {
        return;
    }
    for key in [
        served_model.trim().to_string(),
        served_model.trim().to_ascii_lowercase(),
    ] {
        crate::effort::remember(kind, &key, effort_levels.to_vec());
    }
}

/// Q9 review: remember every saved entry that passes the registry's checks
/// under the harness its style runs on, switched on or not, so the
/// served-id guard ([`needs_target_on`]) knows them before a model menu or
/// a turn has read the registry. Reads nothing. Windows and Claude's ids are
/// only ever added (an old one can only be a self-hosted model's id, never
/// one of Claude's own); Codex's set is replaced ([`registry_changed`]).
pub fn remember_entries(entries: &[SelfHostedEntry]) {
    for entry in entries {
        let Ok(entry) = crate::self_hosted_registry::normalize(entry.clone()) else {
            continue;
        };
        if let Some(kind) = harness(&entry.api_style) {
            remember_for(
                kind,
                &entry.served_model,
                entry.context_window,
                &entry.effort_levels,
            );
        }
    }
}

/// Q3: the window of a self-hosted model id Hollow knows, if any.
pub fn known_window(model: &str) -> Option<u64> {
    let id = model.trim().to_ascii_lowercase();
    known().lock().ok()?.get(&id).map(|k| k.window)
}

/// Q9: whether an `openai` entry saved in the app's registry serves
/// `model` ([`codex_guard`]).
fn known_codex(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    codex_guard()
        .lock()
        .map(|ids| ids.contains(&id))
        .unwrap_or(false)
}

/// Q9 delta review: the app's registry now holds `entries` (startup, and
/// each save, removal or switch, called under the registry's lock so two
/// changes can't land out of order): remember them ([`remember_entries`])
/// and replace the Codex guard with the served ids of the `openai` ones.
pub fn registry_changed(entries: &[SelfHostedEntry]) {
    remember_entries(entries);
    let ids = codex_guard_ids(entries);
    if let Ok(mut guard) = codex_guard().lock() {
        *guard = ids;
    }
}

/// The served ids (lowercased) of the `openai` entries in `entries` that
/// pass the registry's checks, switched on or not.
pub(crate) fn codex_guard_ids(entries: &[SelfHostedEntry]) -> std::collections::HashSet<String> {
    entries
        .iter()
        .filter_map(|e| crate::self_hosted_registry::normalize(e.clone()).ok())
        .filter(|e| harness(&e.api_style) == Some(crate::agent::AgentKind::Codex))
        .map(|e| e.served_model.trim().to_ascii_lowercase())
        .collect()
}

impl fmt::Debug for RunTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunTarget")
            .field("entry", &self.entry)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("run_key", &self.run_key)
            .field("served_model", &self.served_model)
            .field("context_window", &self.context_window)
            .field("effort_levels", &self.effort_levels)
            .field("api_style", &self.api_style)
            .field("codex_home", &self.codex_home)
            .finish()
    }
}

/// Whether `url` is `http://` to a literal loopback address (`127.0.0.1`
/// or `[::1]`): the only place a run's base URL may point (the forwarder,
/// which binds `127.0.0.1`). Anything else, `localhost` included (the hosts
/// file decides where it goes), is refused by the command builder, so a run
/// key can't leave the machine.
pub fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host_port = rest.split('/').next().unwrap_or("");
    let host = match host_port.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        Some(_) => return false,
        None => host_port,
    };
    matches!(host, "127.0.0.1" | "[::1]")
}

/// A run key's shape: `hsr_` and at least 32 lowercase hex digits (the
/// forwarder mints 64), which `claude_runner::redact` masks wherever it
/// appears.
pub fn is_run_key(key: &str) -> bool {
    key.strip_prefix("hsr_").is_some_and(|hex| {
        hex.len() >= 32 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// Why `target` can't drive a run, or `None` when it can.
pub fn invalid(target: &RunTarget) -> Option<String> {
    if !is_loopback_url(&target.base_url) {
        return Some(format!(
            "The self-hosted model {} has no local forwarder address, so the run didn't start.",
            target.name
        ));
    }
    if !is_run_key(target.run_key.as_str()) || target.served_model.trim().is_empty() {
        return Some(format!(
            "The self-hosted model {} isn't ready (no run key or model id), so the run didn't start.",
            target.name
        ));
    }
    // Q9: a Codex run needs Hollow's own Codex home for the entry.
    if target.agent() == Some(crate::agent::AgentKind::Codex)
        && !target.codex_home.as_deref().is_some_and(Path::is_absolute)
    {
        return Some(format!(
            "The self-hosted model {} has no Codex home in Hollow's app data, so the run didn't start.",
            target.name
        ));
    }
    None
}

/// Q9: why `target` can't run on `kind`, or `None` when its API style runs
/// there (an `anthropic` entry on Claude, an `openai` one on Codex).
pub fn wrong_harness(target: &RunTarget, kind: crate::agent::AgentKind) -> Option<String> {
    match target.agent() {
        Some(agent) if agent == kind => None,
        Some(agent) => Some(format!(
            "{} is a self-hosted model that speaks {}, which runs on {} only, so the run didn't start.",
            target.name,
            style_name(&target.api_style),
            agent.label()
        )),
        None => Some(format!(
            "{} uses the {} API style, which Hollow can't run, so the run didn't start.",
            target.name, target.api_style
        )),
    }
}

/// Q1: put `cmd` on the self-hosted endpoint and off every Anthropic
/// credential. Removes every inherited `ANTHROPIC_*` variable (as the app
/// sees them now) and [`REMOVED_ENVS`], then sets the forwarder's base
/// URL, the per-run key, the served model for the main model and every
/// alias, and the timeout and tool-search settings above. Call it after
/// every other env step, so nothing set later can put a credential back.
pub fn apply_env(cmd: &mut Command, target: &RunTarget) {
    apply_env_from(cmd, target, std::env::vars_os().map(|(k, _)| k));
}

/// [`apply_env`] with the inherited variable names given, for tests.
pub(crate) fn apply_env_from(
    cmd: &mut Command,
    target: &RunTarget,
    inherited: impl IntoIterator<Item = std::ffi::OsString>,
) {
    let removed = |name: &std::ffi::OsStr| {
        let name = name.to_string_lossy();
        REMOVED_PREFIXES.iter().any(|p| name.starts_with(p))
    };
    for name in inherited {
        if removed(&name) {
            cmd.env_remove(&name);
        }
    }
    // Anything the command itself was given (a run's extra env).
    let set: Vec<std::ffi::OsString> = cmd
        .as_std()
        .get_envs()
        .filter(|(name, value)| value.is_some() && removed(name))
        .map(|(name, _)| name.to_os_string())
        .collect();
    for name in set {
        cmd.env_remove(name);
    }
    for name in REMOVED_ENVS {
        cmd.env_remove(name);
    }
    let no_proxy = no_proxy_for(cmd);
    cmd.env_remove(crate::claude_accounts::ACCOUNT_ENV);
    cmd.env(MARKER_ENV, &target.entry);
    cmd.env(BASE_URL_ENV, &target.base_url)
        .env(AUTH_TOKEN_ENV, target.run_key.as_str())
        .env(MODEL_ENV, &target.served_model)
        .env("NO_PROXY", &no_proxy)
        .env("no_proxy", &no_proxy);
    for alias in ALIAS_MODEL_ENVS {
        cmd.env(alias, &target.served_model);
    }
    cmd.env(NONESSENTIAL_ENV, "1")
        .env(API_TIMEOUT_ENV, API_TIMEOUT_MS.to_string())
        .env(TOOL_SEARCH_ENV, "false");
    // Q3: the window, the compact window (over Settings' own, OB22) and
    // the answer budget, from the entry.
    cmd.env(MAX_CONTEXT_ENV, target.context_window.to_string())
        .env(
            crate::agent::CLAUDE_COMPACT_WINDOW_ENV,
            target.compact_window().to_string(),
        )
        .env(MAX_OUTPUT_ENV, target.answer_budget().to_string());
}

/// `NO_PROXY` for a run's command: the command's own value first (a run's
/// extra env), else the app's, with the loopback hosts appended once each.
fn no_proxy_for(cmd: &Command) -> String {
    let set_here = crate::usage::command_env(cmd);
    let mut no_proxy: Vec<String> = Vec::new();
    for name in ["NO_PROXY", "no_proxy"] {
        if let Some(value) = set_here(name).or_else(|| std::env::var(name).ok()) {
            no_proxy.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(String::from),
            );
        }
    }
    no_proxy.extend(NO_PROXY_HOSTS.split(',').map(String::from));
    let mut hosts: Vec<String> = Vec::new();
    for host in no_proxy {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts.join(",")
}

// ── Codex on a self-hosted model (Q9) ─────────────────────────────────────

/// Q9: the `CODEX_HOME` Hollow owns for `openai` entry `entry`: a stable
/// folder in app data (`self-hosted-codex/<id>`), so its rollouts persist
/// and a chat's Codex session resumes there. `None` when app data can't be
/// found (the run then fails closed, [`invalid`]).
pub fn codex_home_for(entry: &str) -> Option<PathBuf> {
    let root = crate::agent_settings::app_data_dir().ok()?;
    Some(root.join(CODEX_HOMES_DIR).join(entry))
}

/// Q9: whether `path` is one of Hollow's self-hosted Codex homes (its
/// parent folder is [`CODEX_HOMES_DIR`]).
pub fn is_self_hosted_codex_home(path: &Path) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == CODEX_HOMES_DIR)
}

/// Q9 review: the files in a Codex home Hollow never writes and Codex
/// doesn't write in a run (probed with 0.166.0, `exec` and `app-server`):
/// a sign-in, and config that could change where the run goes (a user
/// config's provider, a managed config that outranks `-c`, requirements).
pub const CODEX_HOME_REFUSED: [&str; 4] = [
    "auth.json",
    "config.toml",
    "managed_config.toml",
    "requirements.toml",
];

/// Q9 review: files outside the home that Codex reads and that can outrank
/// what Hollow sets on the command line: a system config (whose legacy
/// `profile` an older Codex would follow over `-c`), a managed config and
/// the managed preferences an organisation's MDM installs. None of them is
/// on a personal Mac (none here, 2026-11-03).
pub fn codex_outranking_files() -> Vec<PathBuf> {
    let managed = Path::new("/Library/Managed Preferences");
    let mut files = vec![
        PathBuf::from("/etc/codex/config.toml"),
        PathBuf::from("/etc/codex/managed_config.toml"),
        managed.join("com.openai.codex.plist"),
    ];
    if let Some(user) = std::env::var_os("USER").filter(|u| !u.is_empty()) {
        files.push(managed.join(user).join("com.openai.codex.plist"));
    }
    files
}

/// Q9 delta review: the outranking files a run checks. Tests check none: a
/// test host with one of them (a gate host with `/etc/codex`) must not fail
/// every self-hosted Codex test; `codex_home_problem` is tested with its
/// list given.
fn checked_outranking_files() -> Vec<PathBuf> {
    #[cfg(test)]
    {
        Vec::new()
    }
    #[cfg(not(test))]
    {
        codex_outranking_files()
    }
}

/// Q9 review: why a self-hosted Codex run can't use `home`, given the
/// outranking files to check (`system`), or `None`. A file Hollow never
/// writes in the home ([`CODEX_HOME_REFUSED`]: a turn with full access
/// could have written one to send later turns elsewhere) or one of
/// `system` refuses the run, naming the path; Hollow never deletes either.
pub fn codex_home_problem(home: &Path, system: &[PathBuf]) -> Option<String> {
    for name in CODEX_HOME_REFUSED {
        let file = home.join(name);
        if file.exists() {
            let what = if name == "auth.json" {
                "a sign-in, which a self-hosted run never uses"
            } else {
                "settings Hollow never writes there, which could send this model's conversation elsewhere"
            };
            return Some(format!(
                "Hollow's Codex home for this self-hosted model holds {what} ({}), so the run didn't start. Remove that file and send it again.",
                file.display()
            ));
        }
    }
    system.iter().find(|file| file.exists()).map(|file| {
        format!(
            "Codex on this Mac reads settings from {} that can outrank what Hollow sets for a self-hosted model, so the run didn't start. Set this model's API style to Anthropic's Messages API to run it on Claude, or remove that file (an organisation's managed settings can only be removed by its admin).",
            file.display()
        )
    })
}

/// Q9: make `home` ready for a run: created (only its owner may read it),
/// then checked ([`codex_home_problem`] with [`codex_outranking_files`]).
pub fn ensure_codex_home(home: &Path) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| {
        format!(
            "Hollow couldn't make the Codex home for a self-hosted model ({}): {e}",
            home.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700));
    }
    match codex_home_problem(home, &checked_outranking_files()) {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// Q9: the `-c` overrides that put a Codex run on the self-hosted endpoint:
/// Hollow's provider ([`CODEX_PROVIDER_ID`]) at the forwarder's `/v1` with
/// the Responses wire API, the run key read from [`CODEX_KEY_ENV`] (only
/// the variable's name is here: argv holds no secret), no OpenAI login, a
/// 15-minute quiet-stream limit for a cold start; the served model; the
/// entry's window; Codex's own telemetry off (Q9 review: 0.166.0 otherwise
/// opens `ab.chatgpt.com:443` for its metrics, so `otel` exporters `none`,
/// `analytics` and `feedback` off); and its shell policy keeping the run
/// key out of the commands it runs. Effort and the compaction limit come
/// from the run's own steps (`agent::run_effort`, `agent::compact_window`).
/// A `-c` value outranks the user and system config and Codex 0.166.0
/// ignores a project's provider keys, so no file Hollow allows can move the
/// run ([`codex_home_problem`] refuses the ones that could).
pub fn codex_overrides(target: &RunTarget) -> Vec<String> {
    let quote = |v: &str| serde_json::to_string(v).expect("a string serializes");
    vec![
        format!("model_provider={}", quote(CODEX_PROVIDER_ID)),
        format!(
            "model_providers.{CODEX_PROVIDER_ID}={{name={},base_url={},wire_api=\"responses\",env_key={},requires_openai_auth=false,stream_idle_timeout_ms={CODEX_STREAM_IDLE_MS}}}",
            quote(&format!("{} (self-hosted)", target.name)),
            quote(&format!("{}/v1", target.base_url.trim_end_matches('/'))),
            quote(CODEX_KEY_ENV),
        ),
        format!("model={}", quote(&target.served_model)),
        format!("model_context_window={}", target.context_window),
        "otel.exporter=\"none\"".into(),
        "otel.trace_exporter=\"none\"".into(),
        "otel.metrics_exporter=\"none\"".into(),
        "analytics.enabled=false".into(),
        "feedback.enabled=false".into(),
        "shell_environment_policy.ignore_default_excludes=false".into(),
        format!("shell_environment_policy.exclude=[{}]", quote(CODEX_KEY_ENV)),
    ]
}

/// Q9: put a Codex `cmd` on the self-hosted endpoint and off every OpenAI,
/// Codex and Anthropic credential: removes every inherited or set variable
/// with a [`CODEX_REMOVED_PREFIXES`] prefix (`OPENAI_API_KEY`,
/// `OPENAI_BASE_URL`, `CODEX_API_KEY`, an inherited `CODEX_HOME`, ...) and
/// [`REMOVED_ENVS`], then sets `CODEX_HOME` to the entry's own home, the run
/// key in [`CODEX_KEY_ENV`], the marker, and the loopback hosts in
/// `NO_PROXY`. Call it after every other env step.
pub fn apply_codex_env(cmd: &mut Command, target: &RunTarget) {
    apply_codex_env_from(cmd, target, std::env::vars_os().map(|(k, _)| k));
}

/// [`apply_codex_env`] with the inherited variable names given, for tests.
pub(crate) fn apply_codex_env_from(
    cmd: &mut Command,
    target: &RunTarget,
    inherited: impl IntoIterator<Item = std::ffi::OsString>,
) {
    let removed = |name: &std::ffi::OsStr| {
        let name = name.to_string_lossy();
        CODEX_REMOVED_PREFIXES.iter().any(|p| name.starts_with(p))
    };
    for name in inherited {
        if removed(&name) {
            cmd.env_remove(&name);
        }
    }
    let set: Vec<std::ffi::OsString> = cmd
        .as_std()
        .get_envs()
        .filter(|(name, value)| value.is_some() && removed(name))
        .map(|(name, _)| name.to_os_string())
        .collect();
    for name in set {
        cmd.env_remove(name);
    }
    for name in REMOVED_ENVS {
        cmd.env_remove(name);
    }
    let no_proxy = no_proxy_for(cmd);
    cmd.env_remove(crate::claude_accounts::ACCOUNT_ENV);
    cmd.env(MARKER_ENV, &target.entry)
        .env(CODEX_KEY_ENV, target.run_key.as_str())
        .env("NO_PROXY", &no_proxy)
        .env("no_proxy", &no_proxy);
    if let Some(home) = target.codex_home.as_deref() {
        cmd.env("CODEX_HOME", home);
    }
}

/// The `--settings` a self-hosted run's Claude Code gets (Q1 review): a
/// settings file's `env` (the person's `~/.claude/settings.json`, or a
/// project's `.claude/settings*.json`) is applied over the process env, so
/// one setting `ANTHROPIC_BASE_URL` or `CLAUDE_CODE_USE_BEDROCK` would send
/// the run elsewhere. Command-line settings outrank user, project and local
/// ones, so this pins the base URL and the models and blanks every
/// credential and provider switch. No secret is in it: it rides argv, and
/// the run key stays in the env (a settings file that set
/// `ANTHROPIC_AUTH_TOKEN` only makes the forwarder refuse the run).
pub fn settings_json(target: &RunTarget) -> String {
    let mut env = serde_json::Map::new();
    env.insert(BASE_URL_ENV.into(), target.base_url.clone().into());
    env.insert(MODEL_ENV.into(), target.served_model.clone().into());
    for alias in ALIAS_MODEL_ENVS {
        env.insert(alias.into(), target.served_model.clone().into());
    }
    env.insert(NONESSENTIAL_ENV.into(), "1".into());
    env.insert(API_TIMEOUT_ENV.into(), API_TIMEOUT_MS.to_string().into());
    env.insert(TOOL_SEARCH_ENV.into(), "false".into());
    env.insert(
        MAX_CONTEXT_ENV.into(),
        target.context_window.to_string().into(),
    );
    env.insert(
        crate::agent::CLAUDE_COMPACT_WINDOW_ENV.into(),
        target.compact_window().to_string().into(),
    );
    env.insert(
        MAX_OUTPUT_ENV.into(),
        target.answer_budget().to_string().into(),
    );
    for name in BLANKED_IN_SETTINGS {
        env.insert(name.into(), "".into());
    }
    // A settings file's proxy would otherwise carry the key off this Mac.
    for name in ["NO_PROXY", "no_proxy"] {
        env.insert(name.into(), NO_PROXY_HOSTS.into());
    }
    serde_json::json!({ "env": env }).to_string()
}

// ── A chat turn on a self-hosted model (Q5) ──────────────────────────────

/// Q5: how a chat's model names a registry entry (`self-hosted:qwen`). The
/// Claude model menu lists each enabled entry under it.
pub const MODEL_PREFIX: &str = "self-hosted:";

/// Q5: the longest a turn's run key lives. The turn revokes it as soon as
/// it ends ([`TurnRun`]); this only bounds a key a crash left behind.
pub const TURN_KEY_TTL: Duration = Duration::from_secs(12 * 60 * 60);

/// Where a person manages these models, named in the refusals below.
pub const SETTINGS_PLACE: &str = "Settings → Models → Self-hosted models";

/// Q5: the chat model id of registry entry `entry`.
pub fn model_id(entry: &str) -> String {
    format!("{MODEL_PREFIX}{entry}")
}

/// Q5: the registry entry a chat model id names (`qwen` for
/// `self-hosted:qwen`), or `None` for any other model. An empty id is
/// `Some("")`, which no entry has.
pub fn entry_id(model: &str) -> Option<&str> {
    model.trim().strip_prefix(MODEL_PREFIX).map(str::trim)
}

/// Q5: an entry's row in its harness's model menu (Claude's for an
/// `anthropic` entry, Codex's for an `openai` one, Q9): "Qwen3.8 27B ·
/// self-hosted".
pub fn menu_label(name: &str) -> String {
    format!("{name} · self-hosted")
}

/// Q5: why a run that isn't a chat turn refuses a self-hosted model id:
/// only a chat turn resolves one to its endpoint, and anywhere else the id
/// would go to the agent's own provider with the person's login.
pub fn chats_only(model: &str) -> String {
    format!(
        "{} is a self-hosted model, which only a chat can use for now (choose it in a chat's model menu), so this run didn't start.",
        model.trim()
    )
}

/// Q5 review: why a setter refuses a self-hosted model id for something
/// that isn't a chat (the default model, the extraction model, an
/// automation's model): only a chat turn runs one.
pub fn not_for_this(model: &str) -> String {
    format!(
        "{} is a self-hosted model, which only a chat can use for now (choose it in a chat's model menu). Nothing changed.",
        model.trim()
    )
}

/// Q5 review: whether `model` is a model id a Claude run must not send
/// without its self-hosted target: a `self-hosted:` id, or the served id
/// of an endpoint Hollow knows ([`known_window`]).
pub fn needs_target(model: &str) -> bool {
    entry_id(model).is_some() || known_window(model).is_some()
}

/// Q9: [`needs_target`] for a run of `kind`, on purpose not symmetric.
/// Claude refuses every served id Hollow knows, of either style (as Q5 did):
/// it would go to Anthropic with the person's login, and no served id may be
/// one of Claude's own (`self_hosted_registry::normalize`), so no ordinary
/// Claude run is refused. Codex refuses an `openai` entry's served id only:
/// an `anthropic` entry may serve an OpenAI-shaped id (`gpt-oss-120b`),
/// which an ordinary Codex run must still reach, while an `openai` entry may
/// not (`normalize`). Every agent refuses a `self-hosted:` id.
pub fn needs_target_on(kind: crate::agent::AgentKind, model: &str) -> bool {
    match kind {
        crate::agent::AgentKind::Claude => needs_target(model),
        crate::agent::AgentKind::Codex => entry_id(model).is_some() || known_codex(model),
        _ => entry_id(model).is_some(),
    }
}

/// Q5 review: a chat turn's spec model and its self-hosted target, decided
/// in one place so the served id never goes without its target: the served
/// id and the target for a turn that resolved one ([`turn_route`]), else
/// the chat's own model and none.
pub fn turn_spec_model(
    model: Option<&str>,
    run: Option<&TurnRun>,
) -> (Option<String>, Option<RunTarget>) {
    match run {
        Some(run) => (
            Some(run.target.served_model.clone()),
            Some(run.target.clone()),
        ),
        None => (model.map(str::to_string), None),
    }
}

/// Q5 review: whether `model` names a self-hosted entry for `agent`'s
/// chat turn (Claude, or Codex since Q9): its connectors, retitle and
/// learning follow from it before the route is resolved.
pub fn chat_on_self_hosted(agent: crate::agent::AgentKind, model: Option<&str>) -> bool {
    matches!(
        agent,
        crate::agent::AgentKind::Claude | crate::agent::AgentKind::Codex
    ) && model.and_then(entry_id).is_some()
}

/// Q5 review: what a delegate of a self-hosted run must not inherit: the
/// endpoint, the run key, the served model and its aliases, the window and
/// answer budget, the other settings [`apply_env`] sets, and the marker.
/// Q9: and a Codex run's key ([`CODEX_KEY_ENV`]); its `CODEX_HOME` goes too
/// when it is a self-hosted one ([`strip_inherited_from`]).
pub const INHERITED_ENVS: [&str; 14] = [
    MARKER_ENV,
    CODEX_KEY_ENV,
    BASE_URL_ENV,
    AUTH_TOKEN_ENV,
    MODEL_ENV,
    ALIAS_MODEL_ENVS[0],
    ALIAS_MODEL_ENVS[1],
    ALIAS_MODEL_ENVS[2],
    MAX_CONTEXT_ENV,
    crate::agent::CLAUDE_COMPACT_WINDOW_ENV,
    MAX_OUTPUT_ENV,
    NONESSENTIAL_ENV,
    API_TIMEOUT_ENV,
    TOOL_SEARCH_ENV,
];

/// Q5 review: whether the environment `env` reads is a self-hosted run's
/// (its marker, or a run key as its Anthropic token).
pub fn inherited_from(env: &dyn Fn(&str) -> Option<String>) -> bool {
    env(MARKER_ENV).is_some_and(|v| !v.trim().is_empty())
        || env(AUTH_TOKEN_ENV).is_some_and(|key| is_run_key(key.trim()))
        || env(CODEX_KEY_ENV).is_some_and(|key| is_run_key(key.trim()))
}

/// Q5 review: `mcp_config` with [`MARKER_ENV`] set to `entry` in the `env`
/// of each Hollow-owned server (`hollow_*`), the way
/// `process_group::with_run_tag` adds the run tag, so `hollow team-mcp`
/// knows it serves a self-hosted run even when the CLI passes its servers
/// no inherited environment. Library connectors get nothing. Invalid input
/// is returned unchanged; the adapter's own validation reports it.
pub fn with_marker(mcp_config: &str, entry: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(mcp_config) else {
        return mcp_config.to_string();
    };
    let Some(servers) = value
        .get_mut("mcpServers")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return mcp_config.to_string();
    };
    for (name, server) in servers.iter_mut() {
        if !name.starts_with("hollow_") {
            continue;
        }
        let Some(object) = server.as_object_mut() else {
            continue;
        };
        let env = object
            .entry("env")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(env) = env.as_object_mut() {
            env.insert(
                MARKER_ENV.to_string(),
                serde_json::Value::String(entry.to_string()),
            );
        }
    }
    value.to_string()
}

/// Q5 review: a teammate refused inside a self-hosted run.
pub const NO_CLAUDE_TEAMMATE: &str = "This chat runs on a self-hosted model, so it can't ask a Claude teammate: a Claude teammate would run on your Claude login, outside the self-hosted model. Switch the chat to a model that isn't self-hosted to ask one.";

/// Q5 review: remove the self-hosted run's variables ([`INHERITED_ENVS`])
/// from `cmd` when this process runs inside one (a delegate started by
/// `hollow team-mcp` under a self-hosted chat), so its run key never
/// reaches another process. A no-op anywhere else (the app itself).
pub fn strip_inherited(cmd: &mut Command) -> bool {
    strip_inherited_from(cmd, &|name: &str| std::env::var(name).ok())
}

/// [`strip_inherited`] with the process environment given, for tests.
pub(crate) fn strip_inherited_from(
    cmd: &mut Command,
    env: &dyn Fn(&str) -> Option<String>,
) -> bool {
    if !inherited_from(env) {
        return false;
    }
    for name in INHERITED_ENVS {
        cmd.env_remove(name);
    }
    // Q9: a self-hosted Codex home is that run's alone.
    if env("CODEX_HOME").is_some_and(|home| is_self_hosted_codex_home(Path::new(home.trim()))) {
        cmd.env_remove("CODEX_HOME");
    }
    true
}

/// Q5 review: whether `spec` carries a claude.ai connector (a library
/// connector on Claude Code's login, or a `claudeai-proxy` server in its
/// MCP config), which a self-hosted run never may: that proxy runs on the
/// person's Anthropic login.
pub fn claude_login_servers(spec: &crate::agent::RunSpec) -> bool {
    if spec.extras.shared_connectors.iter().any(|c| c.claude_login) {
        return true;
    }
    let Some(config) = spec.mcp_config.as_deref().filter(|c| !c.trim().is_empty()) else {
        return false;
    };
    match serde_json::from_str::<serde_json::Value>(config) {
        Ok(value) => value["mcpServers"].as_object().is_some_and(|servers| {
            servers
                .values()
                .any(|server| server["type"].as_str() == Some(CLAUDEAI_PROXY))
        }),
        // A config Hollow can't read is refused rather than trusted.
        Err(_) => config.contains(CLAUDEAI_PROXY),
    }
}

/// The MCP server type of a claude.ai connector (`connectors::entry`).
pub const CLAUDEAI_PROXY: &str = "claudeai-proxy";

/// Q5: what a chat turn on a model runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Not a self-hosted model: the turn runs as it always has.
    Ordinary,
    /// This registry entry (switched on, its API style the turn's agent's:
    /// `anthropic` on Claude, `openai` on Codex since Q9).
    Entry(SelfHostedEntry),
    /// Why the turn can't run. It ends before any agent starts and never
    /// runs on another model instead.
    Refused(String),
}

/// Q5: the route of a turn of `agent` on `model`. `entries` reads the
/// registry (`self_hosted_registry::entries`), and only for a self-hosted
/// model, so an ordinary turn never touches the file. A missing,
/// switched-off or unreadable entry, an API style Hollow can't run, an
/// agent other than Claude or Codex, or (Q9) an entry whose API style runs
/// on the other one is refused with a reason naming [`SETTINGS_PLACE`] or
/// the model menu.
pub fn route(
    agent: crate::agent::AgentKind,
    model: Option<&str>,
    entries: impl FnOnce() -> Result<Vec<SelfHostedEntry>, String>,
) -> Route {
    let Some(model) = model.filter(|m| entry_id(m).is_some()) else {
        return Route::Ordinary;
    };
    let model = model.trim();
    let id = entry_id(model).unwrap_or_default();
    if !matches!(
        agent,
        crate::agent::AgentKind::Claude | crate::agent::AgentKind::Codex
    ) {
        return Route::Refused(format!(
            "{model} is a self-hosted model, which runs on Claude or Codex only, so the turn didn't start. Choose it in the model menu, or choose a {} model.",
            agent.label()
        ));
    }
    match find_entry(id, agent, entries()) {
        Found::Ready(entry) => Route::Entry(entry),
        other => Route::Refused(other.refusal(id)),
    }
}

/// Q5 review: what the registry holds for an entry id, as a route needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Found {
    /// The file can't be read: no entry is known, none is stopped.
    Unreadable(String),
    Missing,
    Off(SelfHostedEntry),
    /// Saved with a style Hollow can't run.
    OtherStyle(SelfHostedEntry),
    /// Q9: saved with a style that runs on the other harness (an
    /// `anthropic` entry on Codex, an `openai` one on Claude): the agent
    /// the turn asked for.
    Mismatch(SelfHostedEntry, crate::agent::AgentKind),
    /// Saved, but it no longer passes the registry's checks
    /// (`self_hosted_registry::normalize`, a hand-edited file): the reason.
    Invalid(SelfHostedEntry, String),
    Ready(SelfHostedEntry),
}

impl Found {
    /// Whether the entry is switched off or gone: only then does a turn that
    /// started its forwarder stop it (other turns on it end too, as when
    /// Settings switches it off). An unreadable file or an invalid entry
    /// refuses this turn alone.
    fn off_or_gone(&self) -> bool {
        matches!(self, Found::Missing | Found::Off(_))
    }

    /// Why a turn on this entry can't run, naming where to fix it.
    fn refusal(&self, id: &str) -> String {
        match self {
            Found::Unreadable(why) => format!(
                "The self-hosted models list can't be read, so the turn didn't start ({}). See {SETTINGS_PLACE}, or choose another model.",
                why.trim().trim_end_matches('.')
            ),
            Found::Missing => format!(
                "There is no self-hosted model \"{id}\" in {SETTINGS_PLACE}, so the turn didn't start. Add it there, or choose another model."
            ),
            Found::Off(entry) => format!(
                "{} is switched off in {SETTINGS_PLACE}, so the turn didn't start. Switch it on there, or choose another model.",
                entry.name
            ),
            Found::OtherStyle(entry) => format!(
                "{} uses the {} API style, which Hollow can't run yet, so the turn didn't start. Choose another model.",
                entry.name, entry.api_style
            ),
            Found::Mismatch(entry, asked) => {
                let runs_on = harness(&entry.api_style)
                    .map(|a| a.label())
                    .unwrap_or("another agent");
                format!(
                    "{} speaks {}, which runs on {runs_on}, not {}, so the turn didn't start. Choose it under {runs_on} in the model menu, or choose a {} model.",
                    entry.name,
                    style_name(&entry.api_style),
                    asked.label(),
                    asked.label()
                )
            }
            Found::Invalid(entry, why) => format!(
                "{} in {SETTINGS_PLACE} can't be used as saved, so the turn didn't start: {} Edit it there, or choose another model.",
                entry.name,
                why.trim()
            ),
            Found::Ready(entry) => format!("{} is ready.", entry.name),
        }
    }
}

/// Q5 review: entry `id` in a registry read, checked like a save
/// (`self_hosted_registry::normalize`), so an entry a hand edit broke is
/// refused with the reason Settings shows for it rather than looking absent.
/// Q9: its API style must run on `agent` (`anthropic` on Claude, `openai`
/// on Codex).
fn find_entry(
    id: &str,
    agent: crate::agent::AgentKind,
    entries: Result<Vec<SelfHostedEntry>, String>,
) -> Found {
    let entries = match entries {
        Ok(entries) => entries,
        Err(why) => return Found::Unreadable(why),
    };
    let Some(entry) = entries.into_iter().find(|e| e.id == id) else {
        return Found::Missing;
    };
    if !entry.enabled {
        return Found::Off(entry);
    }
    match harness(&entry.api_style) {
        None => return Found::OtherStyle(entry),
        Some(runs_on) if runs_on != agent => return Found::Mismatch(entry, agent),
        Some(_) => {}
    }
    match crate::self_hosted_registry::normalize(entry.clone()) {
        Ok(entry) => Found::Ready(entry),
        Err(why) => Found::Invalid(entry, why),
    }
}

/// Q5: a chat turn's endpoint and the run key minted for it. Dropping it
/// revokes the key, so a turn that holds it until it returns ends its key
/// however it ends (an answer, an error, Stop). It also keeps the
/// forwarder alive while the turn runs.
pub struct TurnRun {
    pub target: RunTarget,
    forwarder: Arc<Forwarder>,
    /// How the forwarder signs in to the endpoint ([`sign_in_kind`]), for
    /// the note a turn kept on this Mac shows (Q7).
    sign_in: &'static str,
}

impl TurnRun {
    /// The forwarder the key was minted on (tests).
    #[cfg(test)]
    pub fn forwarder(&self) -> &Arc<Forwarder> {
        &self.forwarder
    }
}

impl Drop for TurnRun {
    fn drop(&mut self) {
        self.forwarder.revoke(self.target.run_key.as_str());
    }
}

impl fmt::Debug for TurnRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnRun")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Q5: `entry`'s run target on `forwarder`, with a key minted for one turn
/// ([`TURN_KEY_TTL`] at most).
pub fn turn_run(entry: &SelfHostedEntry, forwarder: Arc<Forwarder>) -> TurnRun {
    run_for(entry, forwarder, TURN_KEY_TTL)
}

/// `entry`'s run target on `forwarder`, with a key that lives `ttl` at
/// most and is revoked when the [`TurnRun`] drops. Q10's background jobs
/// mint theirs for `self_hosted_jobs::JOB_KEY_TTL`.
pub fn run_for(entry: &SelfHostedEntry, forwarder: Arc<Forwarder>, ttl: Duration) -> TurnRun {
    let codex = harness(&entry.api_style) == Some(crate::agent::AgentKind::Codex);
    let target = RunTarget {
        entry: entry.id.clone(),
        name: entry.name.clone(),
        base_url: forwarder.base_url(),
        run_key: forwarder.mint(ttl),
        served_model: entry.served_model.clone(),
        context_window: entry.context_window,
        effort_levels: entry.effort_levels.clone(),
        api_style: entry.api_style.clone(),
        codex_home: codex.then(|| codex_home_for(&entry.id)).flatten(),
    };
    TurnRun {
        target,
        forwarder,
        sign_in: sign_in_kind(&entry.auth),
    }
}

// ── Runs on (Q7) ─────────────────────────────────────────────────────────

/// Q7: how Hollow signs in to an entry's endpoint, as the note of a turn
/// kept on this Mac names it: `gcloud` (a token command running `gcloud`,
/// as the Cloud Run preset's does), `command` (another token command) or
/// `none` (an open endpoint).
pub fn sign_in_kind(auth: &crate::self_hosted_registry::SelfHostedAuth) -> &'static str {
    use crate::self_hosted_registry::SelfHostedAuth;
    match auth {
        SelfHostedAuth::TokenCommand { program, .. }
            if std::path::Path::new(program.trim())
                .file_name()
                .is_some_and(|name| name == "gcloud") =>
        {
            "gcloud"
        }
        SelfHostedAuth::TokenCommand { .. } => "command",
        SelfHostedAuth::None => "none",
    }
}

/// Q7: the first line of a self-hosted turn whose Runs on is the runner
/// (`host`; empty when the runner is chosen but none is set up, which the
/// chat words as its own sentence). The turn runs on this Mac, because
/// Hollow serves the model through this Mac's loopback forwarder, which the
/// runner can't reach, and signs in to its endpoint here (Q7 review: not
/// "no Google Cloud login", which a Google Cloud runner's own service
/// account could make false). A `remoteRun` notice of phase `stayed`, as
/// unattended work that stays here records (overnight N6), with
/// `selfHosted` naming the model and its sign-in ([`sign_in_kind`]) and,
/// Q7 review, `leftOnRunner` (`left`): `pending` when an earlier turn
/// stopped by a quit wasn't brought back (the runner didn't answer, or
/// wasn't asked because it didn't answer minutes ago) and a later message
/// tries again,
/// `runner` when work an earlier turn left there stays until a turn there
/// brings it back, else null (`chat::settled_notices`). The chat words it
/// (`remoteRunNotice.ts`). Holds no key, token or address.
pub fn runner_note(
    host: &str,
    run: &TurnRun,
    left: Option<&str>,
    session_key: &str,
) -> serde_json::Value {
    serde_json::json!({
        "type": "remoteRun",
        "phase": "stayed",
        "host": host,
        "selfHosted": {
            "model": run.target.name,
            "signIn": run.sign_in,
            "leftOnRunner": left,
        },
        "sessionKey": session_key,
    })
}

/// Q5: what a chat turn of `agent` on `model` runs on: `None` for an
/// ordinary model, else the entry's [`TurnRun`] on its forwarder (started
/// if it isn't running), or why the turn can't run. Reads the registry and
/// starts a loopback listener; the endpoint itself is never contacted here.
pub async fn turn_route(
    agent: crate::agent::AgentKind,
    model: Option<&str>,
) -> Result<Option<TurnRun>, String> {
    turn_route_with(agent, model, crate::self_hosted_registry::entries).await
}

/// [`turn_route`] with the registry read given (tests). Q5 review: a
/// switch-off can't slip between the read and the start. The entry's stop
/// count is read before the registry, and the forwarder starts only if no
/// stop came since (`self_hosted_forwarder::ensure_since`); the registry is
/// then read again, and an entry now off or gone stops its forwarder and
/// refuses the turn.
pub(crate) async fn turn_route_with(
    agent: crate::agent::AgentKind,
    model: Option<&str>,
    entries: impl Fn() -> Result<Vec<SelfHostedEntry>, String>,
) -> Result<Option<TurnRun>, String> {
    let since = model
        .and_then(entry_id)
        .map(crate::self_hosted_forwarder::stops)
        .unwrap_or_default();
    let entry = match route(agent, model, &entries) {
        Route::Ordinary => return Ok(None),
        Route::Refused(why) => return Err(why),
        Route::Entry(entry) => entry,
    };
    let forwarder = crate::self_hosted_forwarder::ensure_since(entry.upstream(), since)
        .await
        .map_err(|e| {
            format!(
                "Hollow's local forwarder for {} didn't start, so the turn didn't start: {e}",
                entry.name
            )
        })?;
    // The second read: the entry as it is now that the forwarder runs.
    let id = entry.id.clone();
    match find_entry(&id, agent, entries()) {
        Found::Ready(now) if now.upstream().normalized() == entry.upstream().normalized() => {
            Ok(Some(turn_run(&now, forwarder)))
        }
        Found::Ready(now) => Err(format!(
            "{} changed in {SETTINGS_PLACE} while the turn was starting, so the turn didn't start. Send it again.",
            now.name
        )),
        found => {
            // Off or gone: its forwarder goes, as when Settings switches it
            // off. Unreadable or invalid: this turn alone is refused.
            if found.off_or_gone() {
                crate::self_hosted_forwarder::stop(&id);
            }
            Err(found.refusal(&id))
        }
    }
}

// ── The cold start and a turn's limits (Q11) ─────────────────────────────

/// Q11: the least time a self-hosted turn's no-output watchdog and run
/// deadline give it: one request's whole timeout ([`API_TIMEOUT_MS`]; a cold
/// start is 4 to 6 minutes with nothing printed) and a minute more.
pub const COLD_START_FLOOR: Duration = Duration::from_millis(API_TIMEOUT_MS as u64 + 60_000);

/// Q11: a chat turn's run limits, raised to at least [`COLD_START_FLOOR`]
/// for a turn with a self-hosted `target` (a bake-off variant's may be one),
/// so a watchdog or deadline set shorter in Settings never ends it while the
/// model starts. A switched-off watchdog or deadline stays off, a longer one
/// stays, and any other turn's limits are unchanged.
pub(crate) fn cold_start_limits(
    limits: crate::agent_settings::RunLimits,
    target: Option<&RunTarget>,
) -> crate::agent_settings::RunLimits {
    if target.is_none() {
        return limits;
    }
    let floor = |limit: Option<Duration>| limit.map(|d| d.max(COLD_START_FLOOR));
    crate::agent_settings::RunLimits {
        deadline: floor(limits.deadline),
        stall_after: floor(limits.stall_after),
    }
}

#[cfg(test)]
#[path = "self_hosted_tests.rs"]
mod tests;
