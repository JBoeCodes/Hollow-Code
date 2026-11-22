//! Self-hosted models, Q4 (2026-11-03): the registry of self-hosted model
//! endpoints and Settings' Test. Settings, Models, Self-hosted models
//! lists, adds, edits, switches off, removes and tests them.
//!
//! - **File.** `self-hosted-models.json` in Hollow's app data, beside
//!   `agent-settings.json` ([`crate::agent_settings::app_data_dir`]), written
//!   atomically. Schema v1: `{"version":1,"entries":[…]}`, each entry a
//!   [`SelfHostedEntry`] (camelCase). A missing file is an empty list.
//!   Another version, or a file that can't be read or parsed, is reported in
//!   the list's `error` (never a crash) and refuses every change until it is
//!   fixed or moved aside, so Hollow never overwrites it.
//! - **Validation** ([`normalize`]) on every save, one plain message per
//!   rule; effort levels are stored lowest first.
//! - **Preset** ([`preset`]): Qwen3.8 27B on Cloud Run. Settings offers it
//!   to fill the form; nothing is saved until the person adds it.
//! - **Test** ([`test_at`], then [`test_entry`]): through the entry's
//!   forwarder (`self_hosted_forwarder::ensure_since`), the way a run goes:
//!   the health path (a cold start can take minutes), the model list
//!   (`/v1/models`: is the served id there, and its `max_model_len`), then
//!   one 1-token completion (`/v1/messages` for an `anthropic` entry,
//!   `/v1/responses` for an `openai` one, Q9)
//!   with a run key that lives 2 minutes and is revoked after. The endpoint
//!   token and the run key are never logged or returned.
//! - **Forwarders.** Removing or switching off an entry stops its forwarder
//!   (`self_hosted_forwarder::stop`), which also ends a Test in flight on it
//!   (the forwarder's closed flag). A Test of a switched-off entry reads the
//!   entry again when it ends and stops the forwarder only if the entry is
//!   still off or gone, so switching it on meanwhile keeps it (Q4 review).
use crate::self_hosted_forwarder::{self as forwarder, Auth, Upstream};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The registry's file name inside the app-data folder.
pub const FILE: &str = "self-hosted-models.json";
/// The schema version this Hollow reads and writes.
pub const VERSION: u64 = 1;
/// Anthropic's Messages API: the entry rides the Claude harness and shows
/// in the Claude model menu.
pub const API_ANTHROPIC: &str = "anthropic";
/// Q9: OpenAI's Responses API: the entry rides the Codex harness and shows
/// in the Codex model menu.
pub const API_OPENAI: &str = "openai";
/// Q9: the effort levels an `openai` entry may offer: Codex's own
/// (`model_reasoning_effort` has no `max`; Codex refuses its config with it).
pub const CODEX_LEVELS: [&str; 4] = ["low", "medium", "high", "xhigh"];
/// The smallest context window an entry may claim: Claude Code compacts at
/// 100,000 tokens at the earliest, and a Hollow chat turn's first call is
/// about 52,000 (Q0), so a smaller window would overflow before compacting.
pub const MIN_CONTEXT_WINDOW: u32 = 110_000;
pub const MAX_CONTEXT_WINDOW: u32 = 10_000_000;
/// Claude Code's own model aliases, which an entry's served id may not be,
/// with or without a trailing `[1m]` (nor any id starting `claude`).
pub const CLAUDE_ALIASES: [&str; 7] = [
    "default", "best", "opus", "sonnet", "haiku", "fable", "opusplan",
];
/// A token command's arguments, at most.
pub const MAX_TOKEN_ARGS: usize = 16;
pub const DEFAULT_HEALTH_PATH: &str = "/health";
/// How long the Test's run key lives (it is revoked as soon as the call ends).
const TEST_KEY_TTL: Duration = Duration::from_secs(120);
/// The Test's wait for the completion through the forwarder: the
/// forwarder's own upstream limit (15 minutes) plus a margin.
const TEST_CALL_TIMEOUT: Duration = Duration::from_secs(16 * 60);
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Most of a health or model-list answer Hollow reads.
const HEALTH_BYTES: usize = 4 * 1024;
const MODELS_BYTES: usize = 256 * 1024;

/// One self-hosted model endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedEntry {
    /// `^[a-z0-9][a-z0-9-]{0,31}$`, unique; it names the entry's forwarder.
    pub id: String,
    /// What Settings and the model menu show, 1 to 80 characters.
    pub name: String,
    /// `https://…`, or `http://` to this Mac; no trailing slash once saved.
    pub base_url: String,
    /// `anthropic` (Anthropic's Messages API, run on Claude) or `openai`
    /// (OpenAI's Responses API, run on Codex, Q9).
    #[serde(default = "default_api_style")]
    pub api_style: String,
    /// The model id the server serves (vLLM's `--served-model-name`).
    pub served_model: String,
    /// The served context window, in tokens.
    pub context_window: u32,
    /// The effort levels offered for it, a subset of `effort::LEVELS`,
    /// lowest first.
    pub effort_levels: Vec<String>,
    pub auth: SelfHostedAuth,
    /// The GET that says the server is up (and wakes a cold one).
    #[serde(default = "default_health_path")]
    pub health_path: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// How Hollow's forwarder signs in to the endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SelfHostedAuth {
    /// A command whose stdout is a bearer token, run without a shell
    /// (Qwen: `gcloud auth print-identity-token`).
    TokenCommand {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// An open endpoint (a model on this Mac).
    None,
}

fn default_api_style() -> String {
    API_ANTHROPIC.to_string()
}

fn default_health_path() -> String {
    DEFAULT_HEALTH_PATH.to_string()
}

fn default_enabled() -> bool {
    true
}

impl SelfHostedEntry {
    /// The forwarder's view of this entry.
    pub fn upstream(&self) -> Upstream {
        Upstream {
            entry: self.id.clone(),
            base_url: self.base_url.clone(),
            auth: match &self.auth {
                SelfHostedAuth::TokenCommand { program, args } => Auth::TokenCommand {
                    program: program.clone(),
                    args: args.clone(),
                },
                SelfHostedAuth::None => Auth::None,
            },
        }
    }
}

/// What Settings lists: the saved entries, the preset and the limits its
/// form shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedModels {
    pub entries: Vec<SelfHostedEntry>,
    /// The built-in preset, not saved until the person adds it.
    pub preset: SelfHostedEntry,
    /// The effort levels an entry may offer (`effort::LEVELS`), lowest first.
    pub levels: Vec<String>,
    pub min_context_window: u32,
    pub max_context_window: u32,
    /// Why the file couldn't be read. Every change is refused until it can.
    pub error: Option<String>,
    /// Q5 review: each saved entry that no longer passes [`normalize`] (a
    /// hand-edited file), by id, with the reason. Chats can't use it, its
    /// row says why, and an Edit that fixes it saves it again.
    pub invalid: std::collections::BTreeMap<String, String>,
}

/// What Test found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedTest {
    /// The health check and the completion both answered 2xx.
    pub ok: bool,
    pub health_status: Option<u16>,
    /// How long the health check took (a cold start shows here).
    pub health_ms: Option<u32>,
    /// `/v1/models`' status, when it answered.
    pub models_status: Option<u16>,
    /// `/v1/models` lists the served model id; `None` when the list couldn't
    /// be read (no answer, not 2xx, or not a list of models).
    pub model_listed: Option<bool>,
    /// That model's `max_model_len`, when the list gives one.
    pub reported_window: Option<u32>,
    /// The reported window differs from the entry's context window.
    pub window_mismatch: bool,
    /// From sending the completion to its response headers.
    pub first_byte_ms: Option<u32>,
    /// The completion's status.
    pub status: Option<u16>,
    /// What went wrong, in plain text.
    pub error: Option<String>,
}

/// The built-in preset: Qwen3.8 27B on Cloud Run (Q0's deployment).
pub fn preset() -> SelfHostedEntry {
    SelfHostedEntry {
        id: "qwen".into(),
        name: "Qwen3.8 27B".into(),
        base_url: "https://qwen-27b-x6jmsblvea-uc.a.run.app".into(),
        api_style: API_ANTHROPIC.into(),
        served_model: "qwen3.8-27b".into(),
        context_window: 131_072,
        effort_levels: vec!["low".into(), "medium".into(), "xhigh".into()],
        auth: SelfHostedAuth::TokenCommand {
            program: "gcloud".into(),
            args: vec!["auth".into(), "print-identity-token".into()],
        },
        health_path: DEFAULT_HEALTH_PATH.into(),
        enabled: true,
    }
}

// ── Validation ─────────────────────────────────────────────────────────

fn id_ok(id: &str) -> bool {
    let bytes = id.as_bytes();
    let first = bytes.first().copied().unwrap_or(b'-');
    bytes.len() <= 32
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && bytes
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The entry as it is saved, or why it can't be: trimmed, the base URL
/// without a trailing slash, an empty health path read as `/health`, the
/// effort levels lowest first, and every rule checked.
pub fn normalize(mut entry: SelfHostedEntry) -> Result<SelfHostedEntry, String> {
    entry.id = entry.id.trim().to_string();
    entry.name = entry.name.trim().to_string();
    entry.base_url = entry.base_url.trim().trim_end_matches('/').to_string();
    entry.api_style = entry.api_style.trim().to_string();
    entry.served_model = entry.served_model.trim().to_string();
    entry.health_path = entry.health_path.trim().to_string();
    if entry.health_path.is_empty() {
        entry.health_path = DEFAULT_HEALTH_PATH.into();
    }
    if let SelfHostedAuth::TokenCommand { program, .. } = &mut entry.auth {
        *program = program.trim().to_string();
    }
    if !id_ok(&entry.id) {
        return Err("The id must be 1 to 32 characters: lowercase letters, digits and hyphens, starting with a letter or digit.".into());
    }
    if entry.name.is_empty() || entry.name.chars().count() > 80 {
        return Err("The name must be 1 to 80 characters.".into());
    }
    if entry.name.contains(char::is_control) {
        return Err("The name can't contain control characters.".into());
    }
    if let Some(problem) = forwarder::upstream_url_problem(&entry.base_url) {
        return Err(problem);
    }
    if entry.api_style != API_ANTHROPIC && entry.api_style != API_OPENAI {
        return Err(format!(
            "The API style must be \"{API_ANTHROPIC}\" (Anthropic's Messages API, run on Claude) or \"{API_OPENAI}\" (OpenAI's Responses API, run on Codex)."
        ));
    }
    if entry.served_model.is_empty()
        || entry.served_model.chars().count() > 128
        || entry.served_model.contains(char::is_whitespace)
    {
        return Err(
            "The served model must be 1 to 128 characters with no spaces (the id the server serves)."
                .into(),
        );
    }
    if entry.served_model.contains(char::is_control) {
        return Err("The served model can't contain control characters.".into());
    }
    if entry.served_model.starts_with('-') {
        return Err(
            "The served model can't start with a hyphen (Claude Code would read it as an option)."
                .into(),
        );
    }
    if claude_model_id(&entry.served_model) {
        return Err(
            "This model id is one Claude Code uses for its own models; use the endpoint's own id."
                .into(),
        );
    }
    if entry.api_style == API_OPENAI && openai_model_id(&entry.served_model) {
        return Err(
            "This model id is shaped like one OpenAI uses for its own models (gpt-, codex- or chatgpt- at the start, or o and a digit), which Codex runs on your OpenAI sign-in; serve the model under another name (vLLM's --served-model-name)."
                .into(),
        );
    }
    if entry.served_model.contains(['[', ']']) {
        return Err(
            "The served model can't contain [ or ]: Claude Code reads a trailing [1m] as its own window suffix."
                .into(),
        );
    }
    if !(MIN_CONTEXT_WINDOW..=MAX_CONTEXT_WINDOW).contains(&entry.context_window) {
        return Err("The context window must be between 110,000 and 10,000,000 tokens: Claude Code compacts at 100,000 tokens at the earliest, and a Hollow chat turn's first call is about 52,000.".into());
    }
    entry.effort_levels = effort_levels(&entry.effort_levels)?;
    if entry.api_style == API_OPENAI {
        if let Some(level) = entry
            .effort_levels
            .iter()
            .find(|l| !CODEX_LEVELS.contains(&l.as_str()))
        {
            return Err(format!(
                "\"{level}\" isn't a level Codex sends: an OpenAI-style model offers {} only.",
                CODEX_LEVELS.join(", ")
            ));
        }
    }
    if let SelfHostedAuth::TokenCommand { program, args } = &entry.auth {
        if program.is_empty() {
            return Err("The token command needs a program.".into());
        }
        if args.len() > MAX_TOKEN_ARGS {
            return Err(format!(
                "The token command takes at most {MAX_TOKEN_ARGS} arguments."
            ));
        }
        if program.contains('\0') || args.iter().any(|a| a.contains('\0')) {
            return Err("The token command can't contain a NUL character.".into());
        }
    }
    if !entry.health_path.starts_with('/')
        || entry
            .health_path
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("The health path must start with / and contain no spaces.".into());
    }
    Ok(entry)
}

/// Whether `model` is an id Claude Code uses for its own models: anything
/// starting `claude`, or one of its aliases (any case), each with or
/// without trailing `[…]` suffixes (`opus[1m]`, Q4 review). Hollow remembers
/// each entry's window and effort levels by served id, so an entry under
/// such an id would change ordinary Claude runs (Q3 review).
fn claude_model_id(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    let mut base = model.as_str();
    while let Some(open) = base.strip_suffix(']').and_then(|b| b.rfind('[')) {
        base = &base[..open];
    }
    base.starts_with("claude") || CLAUDE_ALIASES.contains(&base)
}

/// Q9 review: whether `model` is shaped like an id OpenAI uses for its own
/// models (`gpt-…`, `codex-…`, `chatgpt-…`, or `o` and a digit: `o3`,
/// `o4-mini`), any case. An `openai` entry may not serve one: Hollow
/// remembers each entry's served id, and a plain Codex run on it is then
/// refused (`self_hosted::needs_target_on`), so an entry serving `gpt-5.5`
/// would refuse every ordinary Codex run on `gpt-5.5`.
pub(crate) fn openai_model_id(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    let bytes = model.as_bytes();
    ["gpt-", "codex-", "chatgpt-"]
        .iter()
        .any(|p| model.starts_with(p))
        || (bytes.first() == Some(&b'o') && bytes.get(1).is_some_and(u8::is_ascii_digit))
}

/// Q9 review: the model ids in Codex's own catalog (`$CODEX_HOME/models_cache.json`,
/// else `~/.codex`), read for a save of an `openai` entry. Empty when the
/// catalog is missing, too large or unreadable: the shape rule
/// ([`openai_model_id`]) still applies.
fn codex_catalog_ids() -> Vec<String> {
    let Some(path) = crate::codex_rollout::codex_home().map(|h| h.join("models_cache.json")) else {
        return Vec::new();
    };
    if std::fs::metadata(&path).map_or(true, |m| m.len() > 2_000_000) {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    catalog_ids(&bytes)
}

/// The `slug` of every model in a Codex catalog (`{"models":[{"slug":…}]}`).
pub(crate) fn catalog_ids(bytes: &[u8]) -> Vec<String> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|v| v.get("models").and_then(|m| m.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.get("slug").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

/// Q9 review: why an `openai` entry can't serve `model` because Codex's
/// own catalog (`ids`) lists it, or `None`.
pub(crate) fn catalog_problem(model: &str, ids: &[String]) -> Option<String> {
    let model = model.trim();
    ids.iter().any(|id| id.trim().eq_ignore_ascii_case(model)).then(|| {
        format!("\"{model}\" is a model in Codex's own catalog, which Codex runs on your OpenAI sign-in; serve the model under another name (vLLM's --served-model-name).")
    })
}

/// The levels, each one of `effort::LEVELS` and listed once, lowest first.
fn effort_levels(levels: &[String]) -> Result<Vec<String>, String> {
    if levels.is_empty() {
        return Err("Choose at least one effort level.".into());
    }
    let mut seen: Vec<usize> = Vec::new();
    for level in levels {
        let level = level.trim();
        let at = crate::effort::LEVELS
            .iter()
            .position(|l| *l == level)
            .ok_or_else(|| {
                format!(
                    "\"{level}\" is not an effort level (choose from {}).",
                    crate::effort::LEVELS.join(", ")
                )
            })?;
        if seen.contains(&at) {
            return Err(format!("\"{level}\" is listed twice in the effort levels."));
        }
        seen.push(at);
    }
    seen.sort_unstable();
    Ok(seen
        .into_iter()
        .map(|at| crate::effort::LEVELS[at].to_string())
        .collect())
}

// ── The file ───────────────────────────────────────────────────────────

/// Reads and writes of the file, one at a time.
static LOCK: Mutex<()> = Mutex::new(());

fn registry_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(FILE))
}

#[derive(Serialize)]
struct FileOut<'a> {
    version: u64,
    entries: &'a [SelfHostedEntry],
}

#[derive(Deserialize)]
struct FileIn {
    #[serde(default)]
    entries: Vec<SelfHostedEntry>,
}

/// The saved entries: empty when the file is missing; an error naming the
/// problem for another version or a file that can't be read.
pub(crate) fn read_at(path: &Path) -> Result<Vec<SelfHostedEntry>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{FILE} can't be read: {e}")),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{FILE} isn't valid JSON ({e}). Fix it or move it aside."))?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(VERSION) => {}
        Some(other) => {
            return Err(format!(
                "{FILE} is version {other}; this Hollow reads version {VERSION}. Update Hollow, or move the file aside to start a new list."
            ))
        }
        None => {
            return Err(format!(
                "{FILE} has no version number, so Hollow won't read or change it. Fix it or move it aside."
            ))
        }
    }
    let file: FileIn = serde_json::from_value(value).map_err(|e| {
        format!("{FILE} doesn't match version {VERSION} ({e}). Fix it or move it aside.")
    })?;
    Ok(file.entries)
}

fn write_at(path: &Path, entries: &[SelfHostedEntry]) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(&FileOut {
        version: VERSION,
        entries,
    })
    .map_err(|e| format!("Encode {FILE}: {e}"))?;
    bytes.push(b'\n');
    crate::vault::atomic_write(path, &bytes).map_err(|e| format!("Save {FILE}: {e}"))
}

fn view(entries: Vec<SelfHostedEntry>, error: Option<String>) -> SelfHostedModels {
    let invalid = entries
        .iter()
        .filter_map(|e| normalize(e.clone()).err().map(|why| (e.id.clone(), why)))
        .collect();
    SelfHostedModels {
        invalid,
        entries,
        preset: preset(),
        levels: crate::effort::LEVELS
            .iter()
            .map(|l| l.to_string())
            .collect(),
        min_context_window: MIN_CONTEXT_WINDOW,
        max_context_window: MAX_CONTEXT_WINDOW,
        error,
    }
}

fn locked() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) fn list_at(path: &Path) -> SelfHostedModels {
    let _guard = locked();
    match read_at(path) {
        Ok(entries) => view(entries, None),
        Err(why) => view(Vec::new(), Some(why)),
    }
}

/// Add `entry` (`edit` false: its id must be new) or replace the saved entry
/// with its id (`edit` true: it must exist; the id itself can't change).
/// Saving one switched off stops its forwarder.
pub(crate) fn save_at(
    path: &Path,
    entry: SelfHostedEntry,
    edit: bool,
) -> Result<SelfHostedModels, String> {
    let entry = normalize(entry)?;
    if entry.api_style == API_OPENAI {
        if let Some(why) = catalog_problem(&entry.served_model, &codex_catalog_ids()) {
            return Err(why);
        }
    }
    let _guard = locked();
    let entries = read_at(path)?;
    put_locked(path, entries, entry, edit)
}

/// Save an entry built from the saved one, all under the file's lock (L13
/// review): `build` gets the saved entry with id `id` when `edit` (`None`
/// when adding, or when there is none to edit), so the agent's save keeps
/// fields it left out from what is saved now. A file that can't be read
/// refuses the save, rather than building on nothing (which would replace
/// a saved token command with none).
pub(crate) fn save_with_at(
    path: &Path,
    id: &str,
    edit: bool,
    build: impl FnOnce(Option<&SelfHostedEntry>) -> Result<SelfHostedEntry, String>,
) -> Result<SelfHostedModels, String> {
    let _guard = locked();
    let entries = read_at(path)?;
    let saved = if edit {
        entries.iter().find(|e| e.id == id.trim()).cloned()
    } else {
        None
    };
    let entry = normalize(build(saved.as_ref())?)?;
    if entry.api_style == API_OPENAI {
        if let Some(why) = catalog_problem(&entry.served_model, &codex_catalog_ids()) {
            return Err(why);
        }
    }
    put_locked(path, entries, entry, edit)
}

/// [`save_at`]'s change, with the lock held and `entries` read under it.
fn put_locked(
    path: &Path,
    mut entries: Vec<SelfHostedEntry>,
    entry: SelfHostedEntry,
    edit: bool,
) -> Result<SelfHostedModels, String> {
    let at = entries.iter().position(|e| e.id == entry.id);
    match (edit, at) {
        (false, Some(_)) => {
            return Err(format!(
                "Another self-hosted model already uses the id \"{}\".",
                entry.id
            ))
        }
        (true, None) => {
            return Err(format!(
                "There is no self-hosted model with the id \"{}\" to edit.",
                entry.id
            ))
        }
        (true, Some(at)) => entries[at] = entry.clone(),
        (false, None) => entries.push(entry.clone()),
    }
    write_at(path, &entries)?;
    if !entry.enabled {
        forwarder::stop(&entry.id);
    }
    crate::self_hosted_wake::forget(&entry.id);
    // Q9 review: the served-id guard follows the saved list at once.
    guard_follows(path, &entries);
    Ok(view(entries, None))
}

/// Q9 delta review: after a change to the app's own registry (still under
/// its lock, so changes reach the guard in order), the served-id guard is
/// rebuilt from the list just written (`self_hosted::registry_changed`). A
/// registry elsewhere (a test's) leaves it alone.
fn guard_follows(path: &Path, entries: &[SelfHostedEntry]) {
    if registry_path().is_ok_and(|app| app == path) {
        crate::self_hosted::registry_changed(entries);
    }
}

/// Remove the entry and stop its forwarder.
pub(crate) fn remove_at(path: &Path, id: &str) -> Result<SelfHostedModels, String> {
    let _guard = locked();
    let mut entries = read_at(path)?;
    let before = entries.len();
    entries.retain(|e| e.id != id);
    if entries.len() == before {
        return Err(format!(
            "There is no self-hosted model with the id \"{id}\"."
        ));
    }
    write_at(path, &entries)?;
    forwarder::stop(id);
    crate::self_hosted_wake::forget(id);
    guard_follows(path, &entries);
    Ok(view(entries, None))
}

/// Switch an entry on or off; off stops its forwarder.
pub(crate) fn set_enabled_at(
    path: &Path,
    id: &str,
    enabled: bool,
) -> Result<SelfHostedModels, String> {
    let _guard = locked();
    let mut entries = read_at(path)?;
    let entry = entries
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("There is no self-hosted model with the id \"{id}\"."))?;
    entry.enabled = enabled;
    write_at(path, &entries)?;
    if !enabled {
        forwarder::stop(id);
    }
    crate::self_hosted_wake::forget(id);
    guard_follows(path, &entries);
    Ok(view(entries, None))
}

pub(crate) fn find_at(path: &Path, id: &str) -> Result<SelfHostedEntry, String> {
    let _guard = locked();
    read_at(path)?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("There is no self-hosted model with the id \"{id}\"."))
}

pub fn list() -> SelfHostedModels {
    match registry_path() {
        Ok(path) => list_at(&path),
        Err(why) => view(Vec::new(), Some(why)),
    }
}

pub fn save(entry: SelfHostedEntry, edit: bool) -> Result<SelfHostedModels, String> {
    save_at(&registry_path()?, entry, edit)
}

/// [`save_with_at`] on the app's registry.
pub(crate) fn save_with(
    id: &str,
    edit: bool,
    build: impl FnOnce(Option<&SelfHostedEntry>) -> Result<SelfHostedEntry, String>,
) -> Result<SelfHostedModels, String> {
    save_with_at(&registry_path()?, id, edit, build)
}

pub fn remove(id: &str) -> Result<SelfHostedModels, String> {
    remove_at(&registry_path()?, id)
}

pub fn set_enabled(id: &str, enabled: bool) -> Result<SelfHostedModels, String> {
    set_enabled_at(&registry_path()?, id, enabled)
}

/// The saved entry `id`. Test reads through [`test_at`] now; this stays for
/// the run path (Q5), which reads a run's entry by id.
#[allow(dead_code)]
pub fn find(id: &str) -> Result<SelfHostedEntry, String> {
    find_at(&registry_path()?, id)
}

/// The saved entries, or why the file can't be read (Q5: the Claude model
/// menu and a chat turn's route read them; neither writes). Each consumer
/// checks an entry with [`normalize`] before using it (Q5 review): the
/// route refuses an invalid one with the reason, the menu leaves it out,
/// and Settings marks it ([`SelfHostedModels::invalid`]).
pub fn entries() -> Result<Vec<SelfHostedEntry>, String> {
    let path = registry_path()?;
    let _guard = locked();
    read_at(&path)
}

// ── Test ───────────────────────────────────────────────────────────────

/// Every text of the token commands in `entries` (each whole command line,
/// and the program on its own when it is 3 characters or more), longest
/// first, for [`hide_token_commands`]. An argument quoted on its own isn't
/// matched (it could be the endpoint's own address), which is why what an
/// agent reads of a kept error is a fixed sentence besides.
pub(crate) fn token_command_texts(entries: &[SelfHostedEntry]) -> Vec<String> {
    let mut out = Vec::new();
    for entry in entries {
        if let SelfHostedAuth::TokenCommand { program, args } = &entry.auth {
            let mut line = program.clone();
            for arg in args {
                line.push(' ');
                line.push_str(arg);
            }
            out.push(line);
            if program.chars().count() >= 3 {
                out.push(program.clone());
            }
        }
    }
    // Longest first, so the whole line goes before its parts.
    out.sort_by_key(|t| std::cmp::Reverse(t.len()));
    out.dedup();
    out
}

/// Shown where a token command's text was.
pub(crate) const HIDDEN_TOKEN_COMMAND: &str = "[token command]";

/// `text` with every one of `secrets` replaced by [`HIDDEN_TOKEN_COMMAND`].
pub(crate) fn hide_token_commands(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets.iter().filter(|t| !t.is_empty()) {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), HIDDEN_TOKEN_COMMAND);
        }
    }
    out
}

/// `text` with the saved token commands' text hidden (an error about to be
/// kept: the saved list now, so one kept across a later auth change can't
/// name the command it ran with). An unreadable list hides nothing more.
pub(crate) fn without_saved_token_commands(text: &str) -> String {
    hide_token_commands(text, &token_command_texts(&entries().unwrap_or_default()))
}

fn ms(since: Instant) -> u32 {
    u32::try_from(since.elapsed().as_millis()).unwrap_or(u32::MAX)
}

fn success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Whether a `/v1/models` answer (OpenAI's shape, which vLLM serves) lists
/// `model`, and that model's `max_model_len`; `None` when the answer isn't
/// a list of models at all.
pub(crate) fn served_model_in(body: &str, model: &str) -> Option<(bool, Option<u32>)> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let data = value.get("data").and_then(serde_json::Value::as_array)?;
    let listed = data
        .iter()
        .find(|m| m.get("id").and_then(serde_json::Value::as_str) == Some(model));
    Some(match listed {
        Some(m) => (
            true,
            m.get("max_model_len")
                .and_then(serde_json::Value::as_u64)
                .map(|w| u32::try_from(w).unwrap_or(u32::MAX)),
        ),
        None => (false, None),
    })
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

/// The message in an error answer: Anthropic's `error.message`, vLLM's
/// top-level `message`, FastAPI's `detail`, else the body's first line.
pub(crate) fn error_message(status: u16, body: &str) -> String {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let found = value.as_ref().and_then(|v| {
        v.pointer("/error/message")
            .or_else(|| v.get("message"))
            .or_else(|| v.get("detail"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    });
    let text = found.unwrap_or_else(|| body.lines().next().unwrap_or("").to_string());
    if text.trim().is_empty() {
        format!("The model answered {status}.")
    } else {
        format!("The model answered {status}: {}", clip(&text, 300))
    }
}

/// The 1-token request a Test sends for `api_style`, as its harness sends
/// it: Claude Code's `POST /v1/messages` with `anthropic-version`, or (Q9)
/// Codex's `POST /v1/responses`. The path, the extra headers and the body.
pub(crate) fn one_token_request(
    api_style: &str,
    model: &str,
) -> (
    &'static str,
    Vec<(&'static str, &'static str)>,
    serde_json::Value,
) {
    if api_style == API_OPENAI {
        return (
            "/v1/responses",
            Vec::new(),
            serde_json::json!({
                "model": model,
                "input": "ok",
                "max_output_tokens": 1,
                "stream": false,
                "store": false,
            }),
        );
    }
    (
        "/v1/messages",
        vec![("anthropic-version", ANTHROPIC_VERSION)],
        serde_json::json!({
            "model": model,
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ok"}],
        }),
    )
}

/// One 1-token completion through the forwarder at `base`, with run key
/// `key`, as the entry's harness sends it ([`one_token_request`]): the
/// status, the time to the response headers and the error message for a
/// status other than 2xx.
async fn one_token(
    base: &str,
    key: &str,
    entry: &SelfHostedEntry,
) -> Result<(u16, u32, Option<String>), String> {
    let client = reqwest::Client::builder()
        // The key stays on this Mac: no proxy, no redirect.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TEST_CALL_TIMEOUT)
        .build()
        .map_err(|e| format!("The test couldn't start: {e}"))?;
    let (path, headers, body) = one_token_request(&entry.api_style, &entry.served_model);
    let started = Instant::now();
    let mut request = client
        .post(format!("{base}{path}"))
        .header("content-type", "application/json")
        .bearer_auth(key);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let resp = request.body(body.to_string()).send().await.map_err(|e| {
        if e.is_timeout() {
            "The model didn't answer within 16 minutes.".to_string()
        } else {
            "Hollow's local forwarder didn't answer.".to_string()
        }
    })?;
    let first_byte = ms(started);
    let status = resp.status().as_u16();
    if success(status) {
        return Ok((status, first_byte, None));
    }
    let text = resp.text().await.unwrap_or_default();
    Ok((status, first_byte, Some(error_message(status, &text))))
}

/// Test `entry` through its forwarder: health, model list, one completion.
/// Never fails: what went wrong is the result's `error`. `since` is a
/// `forwarder::stops` reading taken before the entry was read, so a switch-off
/// or remove after it ends the Test instead of starting a forwarder.
pub async fn test_entry(entry: &SelfHostedEntry, since: u64) -> SelfHostedTest {
    let out = test_steps(entry, since).await;
    eprintln!(
        "[self-hosted] {}: test {} (health {:?} in {:?} ms, models {:?} listed {:?}, window {:?}, completion {:?} after {:?} ms)",
        entry.id,
        if out.ok { "passed" } else { "failed" },
        out.health_status,
        out.health_ms,
        out.models_status,
        out.model_listed,
        out.reported_window,
        out.status,
        out.first_byte_ms,
    );
    out
}

async fn test_steps(entry: &SelfHostedEntry, since: u64) -> SelfHostedTest {
    let mut out = SelfHostedTest::default();
    let fwd = match forwarder::ensure_since(entry.upstream(), since).await {
        Ok(fwd) => fwd,
        Err(why) => {
            out.error = Some(why);
            return out;
        }
    };
    let started = Instant::now();
    let health = fwd.get(&entry.health_path, HEALTH_BYTES).await;
    out.health_ms = Some(ms(started));
    match health {
        Ok((status, _)) => {
            out.health_status = Some(status);
            if !success(status) {
                out.error = Some(format!(
                    "The health check ({}) answered {status}.",
                    entry.health_path
                ));
                return out;
            }
        }
        Err(why) => {
            out.error = Some(why);
            return out;
        }
    }
    if let Ok((status, body)) = fwd.get("/v1/models", MODELS_BYTES).await {
        out.models_status = Some(status);
        if let Some((listed, window)) = success(status)
            .then(|| served_model_in(&body, &entry.served_model))
            .flatten()
        {
            out.model_listed = Some(listed);
            out.reported_window = window;
            out.window_mismatch = window.is_some_and(|w| w != entry.context_window);
        }
    }
    // Switched off or removed meanwhile: no completion.
    if fwd.is_closed() {
        out.error = Some(forwarder::STOPPED.into());
        return out;
    }
    let key = fwd.mint(TEST_KEY_TTL);
    let sent = one_token(&fwd.base_url(), key.as_str(), entry).await;
    fwd.revoke(key.as_str());
    drop(key);
    match sent {
        Ok((status, first_byte, message)) => {
            out.status = Some(status);
            out.first_byte_ms = Some(first_byte);
            out.ok = success(status);
            out.error = message;
        }
        Err(_) if fwd.is_closed() => out.error = Some(forwarder::STOPPED.into()),
        Err(why) => out.error = Some(why),
    }
    out
}

/// After a Test of a switched-off entry: stop its forwarder unless the
/// entry is on now. Read under the file's lock, which a switch-on also
/// takes, so a switch-on during the Test keeps the forwarder (and any
/// retired one still serving a run) running. A file that can't be read
/// stops it: the entry was off when the Test began. Whether it stopped.
pub(crate) fn stop_unless_on_at(path: &Path, id: &str) -> bool {
    let _guard = locked();
    let on = read_at(path)
        .ok()
        .and_then(|entries| entries.into_iter().find(|e| e.id == id))
        .is_some_and(|e| e.enabled);
    if !on {
        forwarder::stop(id);
    }
    !on
}

/// Settings' Test of the saved entry `id` in the registry at `path`:
/// read, checked by [`normalize`] (a hand-edited entry is refused before
/// any call), then [`test_entry`]. When the entry was off, its forwarder is
/// stopped afterwards unless it was switched on meanwhile
/// ([`stop_unless_on_at`]).
pub(crate) async fn test_at(path: PathBuf, id: String) -> Result<SelfHostedTest, String> {
    let since = forwarder::stops(&id);
    let entry = {
        let (path, id) = (path.clone(), id.clone());
        tokio::task::spawn_blocking(move || find_at(&path, &id))
            .await
            .map_err(|e| e.to_string())??
    };
    let entry = normalize(entry)
        .map_err(|why| format!("This entry can't be tested until it is fixed: {why}"))?;
    let result = test_entry(&entry, since).await;
    if !entry.enabled {
        tokio::task::spawn_blocking(move || stop_unless_on_at(&path, &id))
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(result)
}

/// Settings' Test: [`test_at`] on the app's registry.
pub async fn test(id: String) -> Result<SelfHostedTest, String> {
    test_at(registry_path()?, id).await
}

#[cfg(test)]
#[path = "self_hosted_registry_tests.rs"]
mod tests;
