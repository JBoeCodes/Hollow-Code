//! Model choices for each agent's model picker. Suggestions only: a custom
//! model ID is always a valid input, and nothing here validates or rewrites
//! the ID a run uses (`agent_settings` stays provider-neutral free text).
//!
//! Every list comes from the agent itself, so a new model shows up without
//! a Hollow release:
//! - **Claude**: Claude Code's own `initialize` answer over stream-json
//!   (the handshake the Agent SDK's `supportedModels()` reads). No prompt is
//!   sent, so it costs nothing; hooks and MCP servers are off for it. Each
//!   alias comes with the model it resolves to today (`resolvedModel`),
//!   which names the alias labels and [`newest_claude`]. When Claude Code
//!   doesn't answer, the last list it gave (saved in app data), then
//!   [`builtin_claude_models`], stand in, and the picker says so.
//! - **Codex**: its local catalog (`$CODEX_HOME/models_cache.json`); when
//!   that is missing or over a day old, `codex debug models` has the CLI
//!   refresh it first.
//! - **Antigravity** / **Grok**: `agy models` / `grok models`.
//!
//! CLI calls run under process-group supervision with a bound. Results are
//! cached for [`TTL`] (a minute when they carry a warning); the picker's
//! Refresh skips the cache.
use crate::agent::{find_binary, AgentKind};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    /// The effort levels this model takes, lowest first, when they differ
    /// from the agent's (Antigravity's base models: Neon has `xhigh`, Argon
    /// stops at `high`). Empty means every level the agent offers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub models: Vec<ModelOption>,
    /// Where the list came from, shown under the picker.
    pub source: String,
    /// Why the list is empty, old or incomplete; the custom field still works.
    pub warning: Option<String>,
    /// When the agent last gave this list (RFC 3339), if known.
    #[serde(default)]
    pub checked_at: Option<String>,
    /// Claude only: each family's newest model (`sonnet` →
    /// `claude-sonnet-5-5`), so Settings can flag an older pinned ID.
    #[serde(default)]
    pub newest_by_family: BTreeMap<String, String>,
}

const MAX_MODELS: usize = 200;
const AGY_LIMIT: Duration = Duration::from_secs(15);
const AGY_STDOUT_CAP: usize = 256_000;
const CODEX_CATALOG_CAP: u64 = 2_000_000;
/// Claude Code 2.1.296 answers `initialize` in about a second.
const CLAUDE_LIMIT: Duration = Duration::from_secs(20);
const CLAUDE_STDOUT_CAP: usize = 1_000_000;
const CODEX_REFRESH_LIMIT: Duration = Duration::from_secs(30);
/// Codex's catalog older than this is refreshed before it is shown.
const CODEX_STALE: chrono::Duration = chrono::Duration::hours(24);
/// How long a list is reused before the agent is asked again.
const TTL: Duration = Duration::from_secs(10 * 60);
const WARNING_TTL: Duration = Duration::from_secs(60);
/// Claude model families whose newest member an alias names.
const FAMILIES: [&str; 4] = ["opus", "fable", "sonnet", "haiku"];

fn option(id: &str, label: &str) -> ModelOption {
    ModelOption {
        id: id.into(),
        label: label.into(),
        levels: Vec::new(),
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 200 && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The list used when Claude Code can't be asked and never answered
/// before. Labels name what each alias resolved to when this was written
/// (Claude Code 2.1.296, 2026-10-13); the live list replaces it.
fn builtin_claude_models() -> Vec<ModelOption> {
    const OPUS: &str = "Opus 5.5";
    const FABLE: &str = "Fable 5.1";
    const SONNET: &str = "Sonnet 5.5";
    const HAIKU: &str = "Haiku 5.5";
    let long = |family: &str| format!("{family} · 1M context");
    vec![
        option("opus", OPUS),
        option("opus[1m]", &long(OPUS)),
        option("fable", FABLE),
        option("fable[1m]", &long(FABLE)),
        option("sonnet", SONNET),
        option("sonnet[1m]", &long(SONNET)),
        option("haiku", HAIKU),
        option("opusplan", &format!("{OPUS} plans, {SONNET} executes")),
        option("claude-opus-5-5", "Opus 5.5 · pinned"),
        option("claude-fable-5-1", "Fable 5.1 · pinned"),
        option("claude-sonnet-5-5", "Sonnet 5.5 · pinned"),
        option("claude-sonnet-5", "Sonnet 5 · pinned"),
        option("claude-haiku-5-5", "Haiku 5.5 · pinned"),
        option("claude-haiku-4-5-20251001", "Haiku 4.5 · pinned"),
        option("claude-opus-4-8", "Opus 4.8 · pinned"),
    ]
}

/// The newest Haiku when this Hollow was written: the one fallback for every
/// default that follows the newest Haiku once Claude Code says which that is
/// ([`builtin_newest`], `main_agent::SWARM_MODEL`,
/// `extract::DEFAULT_CLAUDE_MODEL`), and the id app-action prose names. A
/// macro rather than a const so `concat!` can splice it into those
/// `&'static str` summaries (app_actions.rs).
macro_rules! haiku_fallback_id {
    () => {
        "claude-haiku-5-5"
    };
}
pub(crate) use haiku_fallback_id;

/// What [`builtin_claude_models`]' aliases resolved to.
fn builtin_newest() -> BTreeMap<String, String> {
    [
        ("opus", "claude-opus-5-5"),
        ("fable", "claude-fable-5-1"),
        ("sonnet", "claude-sonnet-5-5"),
        ("haiku", haiku_fallback_id!()),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeModelRow {
    value: String,
    #[serde(default)]
    resolved_model: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

/// A Claude list parsed from Claude Code's `initialize` answer.
#[derive(Debug, PartialEq)]
struct ClaudeList {
    models: Vec<ModelOption>,
    newest: BTreeMap<String, String>,
    version: Option<String>,
}

/// `claude-opus-4-8` → `opus`; `None` for anything else.
fn claude_family(id: &str) -> Option<&'static str> {
    let rest = id.strip_prefix("claude-")?;
    FAMILIES
        .into_iter()
        .find(|f| rest.strip_prefix(f).is_some_and(|r| r.starts_with('-')))
}

/// Parse the stream-json lines Claude Code printed for an `initialize`
/// control request: one `control_response` whose `response.response`
/// holds `models` (`value`, `resolvedModel`, `displayName`).
fn parse_claude_init(stdout: &str) -> Result<ClaudeList, String> {
    let line = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["type"] == "control_response")
        .ok_or("Claude Code didn't send its model list.")?;
    let response = &line["response"];
    if response["subtype"] == "error" {
        return Err("Claude Code refused the model-list request.".into());
    }
    let body = &response["response"];
    let rows: Vec<ClaudeModelRow> = serde_json::from_value(body["models"].clone())
        .map_err(|_| "Claude Code's model list is unreadable.")?;
    let version = body["claude_code_version"]
        .as_str()
        .filter(|v| v.len() <= 40)
        .map(str::to_string);
    let mut newest = BTreeMap::new();
    let mut display: HashMap<String, String> = HashMap::new();
    for row in &rows {
        let (Some(resolved), Some(name)) = (&row.resolved_model, &row.display_name) else {
            continue;
        };
        if FAMILIES.contains(&row.value.as_str())
            && valid_id(resolved)
            && claude_family(resolved) == Some(row.value.as_str())
        {
            newest.insert(row.value.clone(), resolved.clone());
            display.insert(row.value.clone(), name.clone());
        }
    }
    let mut models = Vec::new();
    let mut push = |m: ModelOption| {
        if !models.iter().any(|o: &ModelOption| o.id == m.id) && models.len() < MAX_MODELS {
            models.push(m);
        }
    };
    for row in &rows {
        // "default" is the CLI's own choice, which the picker's empty
        // option already means.
        if row.value == "default" || !valid_id(&row.value) {
            continue;
        }
        let name = row
            .display_name
            .as_deref()
            .filter(|n| !n.is_empty() && n.len() <= 80)
            .unwrap_or(&row.value);
        if row.value.starts_with("claude-") {
            push(option(&row.value, &format!("{name} · pinned")));
        } else {
            push(option(&row.value, name));
            // Documented alias forms Claude Code doesn't list itself.
            if matches!(row.value.as_str(), "opus" | "fable" | "sonnet") {
                push(option(
                    &format!("{}[1m]", row.value),
                    &format!("{name} · 1M context"),
                ));
            }
        }
    }
    if let (Some(opus), Some(sonnet)) = (display.get("opus"), display.get("sonnet")) {
        let plan = option("opusplan", &format!("{opus} plans, {sonnet} executes"));
        // Keep it beside the aliases, before the pinned IDs.
        let at = models
            .iter()
            .position(|m| m.id.starts_with("claude-"))
            .unwrap_or(models.len());
        if !models.iter().any(|m| m.id == "opusplan") {
            models.insert(at, plan);
        }
    }
    if models.is_empty() {
        return Err("Claude Code listed no models.".into());
    }
    Ok(ClaudeList {
        models,
        newest,
        version,
    })
}

/// Ask Claude Code (`binary`) for its model list. Sends only the
/// `initialize` control request, then closes stdin, so no model is called.
async fn ask_claude(binary: &Path) -> Result<ClaudeList, String> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--settings",
        r#"{"disableAllHooks":true}"#,
    ])
    .env("PATH", crate::agent::cli_path())
    .current_dir(std::env::temp_dir());
    let request = b"{\"type\":\"control_request\",\"request_id\":\"hollow-models\",\"request\":{\"subtype\":\"initialize\"}}\n";
    let out = crate::process_group::run_to_completion_capped(
        &mut cmd,
        Some(request),
        CLAUDE_LIMIT,
        None,
        CLAUDE_STDOUT_CAP,
    )
    .await
    .map_err(|_| "Could not start Claude Code.")?;
    if out.stdout_truncated {
        return Err("Claude Code's model list is too large.".into());
    }
    if out.timed_out {
        return Err("Claude Code didn't answer in time.".into());
    }
    parse_claude_init(&String::from_utf8_lossy(&out.stdout))
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedLists {
    #[serde(default)]
    claude: Option<ModelCatalog>,
}

fn saved_path() -> Option<PathBuf> {
    crate::agent_settings::app_data_dir()
        .ok()
        .map(|d| d.join("agent-model-cache.json"))
}

fn read_saved() -> SavedLists {
    saved_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_claude(catalog: &ModelCatalog) {
    let Some(path) = saved_path() else { return };
    let lists = SavedLists {
        claude: Some(catalog.clone()),
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_vec_pretty(&lists) {
        let _ = crate::vault::atomic_write(&path, &text);
    }
}

/// The newest model of each Claude family as Claude Code last reported it
/// (this session, then the saved list), else the built-in table.
static NEWEST: Lazy<Mutex<Option<BTreeMap<String, String>>>> = Lazy::new(|| Mutex::new(None));

fn remember_newest(newest: &BTreeMap<String, String>) {
    if !newest.is_empty() {
        *NEWEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(newest.clone());
    }
}

/// The newest model ID of a Claude family (`haiku` → `claude-haiku-5-5`)
/// as Claude Code last resolved its alias; the built-in table before it
/// ever answered. Model defaults that name a family use this, so they move
/// to a new model with Claude Code instead of with a Hollow release.
pub fn newest_claude(family: &str) -> Option<String> {
    let mut guard = NEWEST.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        let saved = read_saved()
            .claude
            .map(|c| c.newest_by_family)
            .filter(|m| !m.is_empty());
        *guard = saved;
    }
    guard
        .as_ref()
        .and_then(|m| m.get(family).cloned())
        .or_else(|| builtin_newest().get(family).cloned())
        .filter(|id| claude_family(id) == Some(family))
}

async fn claude_catalog() -> ModelCatalog {
    let found = crate::agent::installed_binary(AgentKind::Claude);
    let failure = match &found {
        None => "Claude Code isn't installed".to_string(),
        Some(binary) => match ask_claude(binary).await {
            Ok(list) => {
                remember_newest(&list.newest);
                let catalog = ModelCatalog {
                    models: list.models,
                    source: match list.version {
                        Some(v) => format!("Claude Code {v}'s model list"),
                        None => "Claude Code's model list".into(),
                    },
                    warning: None,
                    checked_at: Some(now_rfc3339()),
                    newest_by_family: list.newest,
                };
                save_claude(&catalog);
                return catalog;
            }
            Err(e) => e.trim_end_matches('.').to_string(),
        },
    };
    if let Some(saved) = read_saved().claude.filter(|c| !c.models.is_empty()) {
        remember_newest(&saved.newest_by_family);
        let when = saved
            .checked_at
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%b %-d, %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "earlier".into());
        return ModelCatalog {
            warning: Some(format!(
                "{failure}; showing its list from {when}. Refresh to ask again."
            )),
            ..saved
        };
    }
    ModelCatalog {
        models: builtin_claude_models(),
        source: "Hollow's built-in Claude list".into(),
        warning: Some(format!(
            "{failure}; showing Hollow's built-in list, which may miss newer models."
        )),
        checked_at: None,
        newest_by_family: builtin_newest(),
    }
}

/// Codex's catalog JSON (`{"models":[{slug, display_name, visibility}]}`,
/// the same shape in the cache file and `codex debug models`).
fn parse_codex(bytes: &[u8]) -> Result<(Vec<ModelOption>, Option<String>), String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "The Codex model catalog is unreadable.")?;
    let rows = value["models"]
        .as_array()
        .ok_or("The Codex model catalog has no model list.")?;
    let models: Vec<_> = rows
        .iter()
        .filter(|m| m["visibility"].as_str() != Some("hide"))
        .filter_map(|m| {
            let id = m["slug"].as_str().filter(|id| valid_id(id))?;
            Some(option(id, m["display_name"].as_str().unwrap_or(id)))
        })
        .take(MAX_MODELS)
        .collect();
    if models.is_empty() {
        return Err("The Codex model catalog lists no models.".into());
    }
    let fetched = value["fetched_at"].as_str().map(str::to_string);
    Ok((models, fetched))
}

fn codex_models(path: &Path) -> Result<(Vec<ModelOption>, Option<String>), String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Run Codex once to populate its model catalog.")?
        .take(CODEX_CATALOG_CAP + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the Codex model catalog.")?;
    if bytes.len() as u64 > CODEX_CATALOG_CAP {
        return Err("The Codex model catalog is too large.".into());
    }
    let (models, fetched) = parse_codex(&bytes)?;
    let fetched = fetched.or_else(|| {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
        Some(chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339())
    });
    Ok((models, fetched))
}

fn is_stale(fetched: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> bool {
    fetched
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_none_or(|t| now.signed_duration_since(t) > CODEX_STALE)
}

/// `codex debug models` refreshes Codex's catalog from OpenAI (its own
/// call, with its own sign-in) and prints it.
async fn refresh_codex(binary: &Path) -> Result<Vec<ModelOption>, String> {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(["debug", "models"])
        .env("PATH", crate::agent::cli_path())
        .current_dir(std::env::temp_dir());
    let out = crate::process_group::run_to_completion_capped(
        &mut cmd,
        None,
        CODEX_REFRESH_LIMIT,
        None,
        CODEX_CATALOG_CAP as usize,
    )
    .await
    .map_err(|_| "Could not start Codex")?;
    if out.stdout_truncated {
        return Err("Codex's model list is too large".into());
    }
    if out.timed_out {
        return Err("Codex didn't answer in time".into());
    }
    if !out.status.is_some_and(|s| s.success()) {
        return Err("Codex couldn't refresh its model list".into());
    }
    parse_codex(&out.stdout).map(|(models, _)| models)
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))
}

/// What `codex debug models` last printed, and when: reused for a day, so a
/// CLI that refreshes without rewriting `fetched_at` isn't asked again at
/// every cache expiry.
static CODEX_LIVE: Lazy<Mutex<Option<(Instant, String, Vec<ModelOption>)>>> =
    Lazy::new(|| Mutex::new(None));
const CODEX_LIVE_FOR: Duration = Duration::from_secs(24 * 60 * 60);

async fn codex_catalog(refresh: bool) -> ModelCatalog {
    const SOURCE: &str = "Codex's model catalog";
    if !refresh {
        let live = CODEX_LIVE.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some((_, checked, models)) = live.filter(|(at, ..)| at.elapsed() < CODEX_LIVE_FOR) {
            return ModelCatalog {
                models,
                source: SOURCE.into(),
                warning: None,
                checked_at: Some(checked),
                newest_by_family: BTreeMap::new(),
            };
        }
    }
    let cached = codex_home()
        .ok_or_else(|| "The Codex home folder is unavailable.".to_string())
        .and_then(|home| codex_models(&home.join("models_cache.json")));
    let stale = match &cached {
        Ok((_, fetched)) => refresh || is_stale(fetched.as_deref(), chrono::Utc::now()),
        Err(_) => true,
    };
    let binary = crate::agent::installed_binary(AgentKind::Codex);
    let refreshed = match (stale, &binary) {
        (true, Some(binary)) => Some(refresh_codex(binary).await),
        _ => None,
    };
    let catalog = |models, warning, checked_at| ModelCatalog {
        models,
        source: SOURCE.into(),
        warning,
        checked_at,
        newest_by_family: BTreeMap::new(),
    };
    match (refreshed, cached) {
        (Some(Ok(models)), _) => {
            let checked = now_rfc3339();
            *CODEX_LIVE.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((Instant::now(), checked.clone(), models.clone()));
            catalog(models, None, Some(checked))
        }
        (None, Ok((models, fetched))) => catalog(models, None, fetched),
        (Some(Err(why)), Ok((models, fetched))) => {
            let when = fetched
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&chrono::Local).format("%b %-d").to_string())
                .unwrap_or_else(|| "earlier".into());
            catalog(
                models,
                Some(format!("{why}; showing its list from {when}.")),
                fetched,
            )
        }
        (Some(Err(why)), Err(_)) => catalog(vec![], Some(format!("{why}.")), None),
        (None, Err(why)) => catalog(
            vec![],
            Some(if binary.is_none() {
                "Install Codex to load its model list.".into()
            } else {
                why
            }),
            None,
        ),
    }
}

/// `agy models` prints a status line, then `id\tlabel` rows.
fn parse_agy(text: &str) -> Vec<ModelOption> {
    text.lines()
        .filter_map(|line| {
            let (id, label) = line.split_once('\t')?;
            let id = id.trim();
            valid_id(id).then(|| option(id, label.trim()))
        })
        .take(MAX_MODELS)
        .collect()
}

async fn antigravity_models() -> Result<Vec<ModelOption>, String> {
    let binary =
        find_binary(AgentKind::Antigravity).ok_or("Install Antigravity to load its model list.")?;
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("models")
        .env("PATH", crate::agent::cli_path())
        .current_dir(std::env::temp_dir());
    let out = crate::process_group::run_to_completion_capped(
        &mut cmd,
        None,
        AGY_LIMIT,
        None,
        AGY_STDOUT_CAP,
    )
    .await
    .map_err(|_| "Could not start Antigravity's model list.")?;
    if out.stdout_truncated {
        return Err("Antigravity's model list is too large.".into());
    }
    if out.timed_out {
        return Err("Antigravity's model list timed out. Refresh, or enter a custom ID.".into());
    }
    if !out.status.is_some_and(|s| s.success()) {
        return Err("Antigravity could not list models. Check its sign-in.".into());
    }
    let models = parse_agy(&String::from_utf8_lossy(&out.stdout));
    if models.is_empty() {
        return Err("Antigravity returned no models.".into());
    }
    // Effort is a model variant on Antigravity (`…-low` / `…-high`): the run
    // maps Hollow's one Effort control to the variant, so the variants are
    // remembered from the raw list and the picker gets one row per base.
    crate::effort::remember_agy_catalog(models.iter().map(|m| m.id.as_str()));
    Ok(group_agy(models))
}

/// Antigravity's level words as they end a variant's label (`Gemini 4 Neon
/// (XHigh)`), to cut them from the base model's label.
const AGY_LABEL_LEVELS: [&str; 6] = ["Minimal", "Low", "Medium", "High", "XHigh", "Max"];

/// One row per base model instead of one per `…-low`/`…-high` variant: the
/// id is the base (`gemini-4-neon`), the label drops its level suffix and
/// `levels` lists what its variants take. Hollow's single Effort control
/// picks the variant at run time ([`crate::effort::resolve`]), so the model
/// picker no longer sets effort a second time. A model with no level
/// variants (`gemini-pro-orchestrator`) stays as listed.
fn group_agy(models: Vec<ModelOption>) -> Vec<ModelOption> {
    let mut grouped: Vec<ModelOption> = Vec::new();
    for m in models {
        let (base, level) = crate::effort::split_agy(&m.id);
        let Some(level) = level else {
            grouped.push(m);
            continue;
        };
        if let Some(row) = grouped.iter_mut().find(|row| row.id == base) {
            if !row.levels.iter().any(|l| l == level) {
                row.levels.push(level.to_string());
            }
            continue;
        }
        let label = AGY_LABEL_LEVELS
            .iter()
            .find_map(|word| {
                m.label
                    .strip_suffix(&format!(" ({word})"))
                    .map(str::to_string)
            })
            .unwrap_or_else(|| m.label.clone());
        grouped.push(ModelOption {
            id: base.to_string(),
            label,
            levels: vec![level.to_string()],
        });
    }
    for row in &mut grouped {
        row.levels
            .sort_by_key(|l| crate::effort::LEVELS.iter().position(|x| x == l));
    }
    grouped
}

/// `grok models` lines: `  * grok-4.7 (default)` and `  - grok-4.6`.
fn parse_grok(stdout: &str) -> Vec<ModelOption> {
    stdout
        .lines()
        .filter_map(|line| {
            let rest = line
                .trim()
                .strip_prefix("* ")
                .or_else(|| line.trim().strip_prefix("- "))?;
            let id = rest.strip_suffix("(default)").unwrap_or(rest).trim();
            let valid = !id.is_empty()
                && id.len() <= 80
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'));
            valid.then(|| {
                let label = if rest.contains("(default)") {
                    format!("{id} · default")
                } else {
                    id.to_string()
                };
                option(id, &label)
            })
        })
        .collect()
}

async fn grok_models() -> Result<Vec<ModelOption>, String> {
    let binary = find_binary(AgentKind::Grok).ok_or("Install Grok to load its model list.")?;
    let mut cmd = tokio::process::Command::new(&binary);
    cmd.arg("models")
        .env("PATH", crate::agent::cli_path())
        .current_dir(std::env::temp_dir());
    let out = crate::process_group::run_to_completion_capped(
        &mut cmd,
        None,
        AGY_LIMIT,
        None,
        AGY_STDOUT_CAP,
    )
    .await
    .map_err(|_| "Could not start Grok's model list.")?;
    if out.stdout_truncated {
        return Err("Grok's model list is too large.".into());
    }
    if out.timed_out {
        return Err("Grok's model list timed out. Refresh, or enter a custom ID.".into());
    }
    if !out.status.is_some_and(|s| s.success()) {
        return Err("Grok could not list models. Check its sign-in (`grok login`).".into());
    }
    let models = parse_grok(&String::from_utf8_lossy(&out.stdout));
    if models.is_empty() {
        return Err("Grok returned no models.".into());
    }
    for model in &models {
        learn_grok_levels(&binary, &model.id).await;
    }
    Ok(models)
}

/// Grok's levels differ per model and are undocumented, but an unknown
/// level is refused before any API call with the valid ones named (grok
/// 1.0.46: `use one of: high, medium, low`), so asking costs nothing.
async fn learn_grok_levels(binary: &Path, model: &str) {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args([
        "-p",
        "x",
        "--model",
        model,
        "--reasoning-effort",
        "hollow-probe",
    ])
    .env("PATH", crate::agent::cli_path())
    .current_dir(std::env::temp_dir());
    let Ok(out) = crate::process_group::run_to_completion_capped(
        &mut cmd,
        None,
        AGY_LIMIT,
        None,
        AGY_STDOUT_CAP,
    )
    .await
    else {
        return;
    };
    let levels = crate::effort::parse_grok_levels(&format!(
        "{}\n{}",
        out.stderr,
        String::from_utf8_lossy(&out.stdout)
    ));
    if !levels.is_empty() {
        crate::effort::remember(AgentKind::Grok, model, levels);
    }
}

/// Load the catalogs effort mapping and model defaults need (Antigravity's
/// variants, Grok's per-model levels, Claude's alias targets) once at
/// startup, so a run's effort maps and a family default resolves before
/// the user ever opens a model picker. Installed agents only; failures are
/// silent (the run then takes the safe levels and the saved defaults).
pub async fn warm_effort_catalogs() {
    // Self-hosted models Q9 review: every saved entry's served id, under the
    // harness its style runs on, for the served-id guard (a local file; no
    // endpoint is contacted).
    if let Ok(entries) = crate::self_hosted_registry::entries() {
        crate::self_hosted::registry_changed(&entries);
    }
    if crate::agent::is_available(AgentKind::Claude) {
        let _ = catalog_with(AgentKind::Claude, false).await;
    }
    if crate::agent::is_available(AgentKind::Antigravity) {
        let _ = catalog_with(AgentKind::Antigravity, false).await;
    }
    if crate::agent::is_available(AgentKind::Grok) {
        let _ = catalog_with(AgentKind::Grok, false).await;
    }
}

type Cached = (Instant, ModelCatalog);
static CACHE: Lazy<Mutex<HashMap<&'static str, Cached>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// One fetch per agent at a time; a second caller waits and reads the cache.
static FETCHING: Lazy<HashMap<&'static str, tokio::sync::Mutex<()>>> = Lazy::new(|| {
    AgentKind::all()
        .into_iter()
        .map(|k| (k.as_str(), tokio::sync::Mutex::new(())))
        .collect()
});

fn cached(kind: AgentKind) -> Option<ModelCatalog> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let (at, catalog) = cache.get(kind.as_str())?;
    let ttl = if catalog.warning.is_some() {
        WARNING_TTL
    } else {
        TTL
    };
    (at.elapsed() < ttl).then(|| catalog.clone())
}

/// Forget an agent's cached list (after its CLI updates).
pub fn forget(kind: AgentKind) {
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(kind.as_str());
}

async fn fetch(kind: AgentKind, refresh: bool) -> ModelCatalog {
    let plain = |source: &str, result: Result<Vec<ModelOption>, String>| match result {
        Ok(models) => ModelCatalog {
            models,
            source: source.into(),
            warning: None,
            checked_at: Some(now_rfc3339()),
            newest_by_family: BTreeMap::new(),
        },
        Err(warning) => ModelCatalog {
            models: vec![],
            source: source.into(),
            warning: Some(warning),
            checked_at: None,
            newest_by_family: BTreeMap::new(),
        },
    };
    match kind {
        AgentKind::Claude => claude_catalog().await,
        AgentKind::Codex => codex_catalog(refresh).await,
        AgentKind::Antigravity => {
            plain("Antigravity's available models", antigravity_models().await)
        }
        AgentKind::Grok => plain("Grok's available models", grok_models().await),
    }
}

/// The agent's list (cached), and for Claude and Codex the enabled
/// self-hosted models after it (Q5, Q9, [`self_hosted_options`]): read from
/// the registry on every call and never cached with the agent's own list,
/// so switching one on or off in Settings shows at the next read. Reading
/// the list never contacts an endpoint.
async fn catalog_with(kind: AgentKind, refresh: bool) -> ModelCatalog {
    let catalog = agent_catalog(kind, refresh).await;
    if !matches!(kind, AgentKind::Claude | AgentKind::Codex) {
        return catalog;
    }
    with_self_hosted(kind, catalog, crate::self_hosted_registry::entries())
}

/// Q5: `kind`'s `catalog` with its self-hosted rows after its own. A
/// registry that can't be read adds none: the catalog never fails for it
/// (Settings, Models, Self-hosted models says what is wrong).
pub(crate) fn with_self_hosted(
    kind: AgentKind,
    mut catalog: ModelCatalog,
    entries: Result<Vec<crate::self_hosted_registry::SelfHostedEntry>, String>,
) -> ModelCatalog {
    if let Ok(entries) = entries {
        catalog.models.extend(self_hosted_options(kind, &entries));
    }
    catalog
}

/// Q5: `kind`'s model menu rows for the self-hosted models: each enabled
/// entry whose API style runs on `kind` (`anthropic` on Claude, `openai` on
/// Codex since Q9) as `self-hosted:<id>`, labelled "<name> · self-hosted".
/// Each one's window and effort levels are remembered under its served id
/// (`self_hosted::remember_for`), so effort resolves only to a level it
/// takes. A switched-off entry isn't listed.
pub(crate) fn self_hosted_options(
    kind: AgentKind,
    entries: &[crate::self_hosted_registry::SelfHostedEntry],
) -> Vec<ModelOption> {
    entries
        .iter()
        .filter(|e| e.enabled && crate::self_hosted::harness(&e.api_style) == Some(kind))
        // Q5 review: only an entry that still passes the registry's checks,
        // so a hand-edited `sonnet` or `claude-…` served id is never
        // remembered (Settings marks it invalid).
        .filter_map(|e| crate::self_hosted_registry::normalize(e.clone()).ok())
        .map(|e| {
            crate::self_hosted::remember_for(
                kind,
                &e.served_model,
                e.context_window,
                &e.effort_levels,
            );
            ModelOption {
                id: crate::self_hosted::model_id(&e.id),
                label: crate::self_hosted::menu_label(&e.name),
                levels: Vec::new(),
            }
        })
        .collect()
}

async fn agent_catalog(kind: AgentKind, refresh: bool) -> ModelCatalog {
    if !refresh {
        if let Some(hit) = cached(kind) {
            return hit;
        }
    }
    let _one = FETCHING[kind.as_str()].lock().await;
    if !refresh {
        if let Some(hit) = cached(kind) {
            return hit;
        }
    }
    let catalog = fetch(kind, refresh).await;
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(kind.as_str(), (Instant::now(), catalog.clone()));
    catalog
}

/// An agent's model list: cached for a few minutes; `refresh` asks the
/// agent again (and has Codex refresh its catalog).
pub async fn catalog(agent: &str, refresh: bool) -> ModelCatalog {
    match AgentKind::from_opt(agent) {
        Some(kind) => catalog_with(kind, refresh).await,
        None => ModelCatalog {
            models: vec![],
            source: "Models".into(),
            warning: Some("Choose an agent to see its models.".into()),
            checked_at: None,
            newest_by_family: BTreeMap::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude Code 2.1.296's `initialize` answer, trimmed (2026-10-13).
    const CLAUDE_INIT: &str = r#"{"type":"system","subtype":"hook_started"}
{"type":"control_response","response":{"subtype":"success","request_id":"hollow-models","response":{"claude_code_version":"2.1.296","models":[
{"value":"default","resolvedModel":"claude-opus-5-5","displayName":"Default (recommended)"},
{"value":"opus","resolvedModel":"claude-opus-5-5","displayName":"Opus 5.5"},
{"value":"fable","resolvedModel":"claude-fable-5-1","displayName":"Fable 5.1"},
{"value":"sonnet","resolvedModel":"claude-sonnet-5-5","displayName":"Sonnet 5.5"},
{"value":"haiku","resolvedModel":"claude-haiku-5-5","displayName":"Haiku 5.5"},
{"value":"claude-haiku-4-5-20251001","resolvedModel":"claude-haiku-4-5-20251001","displayName":"Haiku 4.5"},
{"value":"claude-sonnet-5","resolvedModel":"claude-sonnet-5","displayName":"Sonnet 5"},
{"value":"bad id","displayName":"Bad"}
]}}}"#;

    /// [`CLAUDE_INIT`] as Claude Code prints it: one JSON object a line.
    fn init_text() -> String {
        let (first, rest) = CLAUDE_INIT.split_once('\n').unwrap();
        format!("{first}\n{}\n", rest.replace('\n', ""))
    }

    #[test]
    fn claude_init_names_aliases_by_what_they_resolve_to() {
        let list = parse_claude_init(&init_text()).unwrap();
        assert_eq!(list.version.as_deref(), Some("2.1.296"));
        assert_eq!(list.newest["sonnet"], "claude-sonnet-5-5");
        assert_eq!(list.newest["haiku"], "claude-haiku-5-5");
        let ids: Vec<_> = list.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "opus",
                "opus[1m]",
                "fable",
                "fable[1m]",
                "sonnet",
                "sonnet[1m]",
                "haiku",
                "opusplan",
                "claude-haiku-4-5-20251001",
                "claude-sonnet-5"
            ]
        );
        let label = |id: &str| {
            list.models
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .label
                .clone()
        };
        assert_eq!(label("sonnet"), "Sonnet 5.5");
        assert_eq!(label("haiku"), "Haiku 5.5");
        assert_eq!(label("opusplan"), "Opus 5.5 plans, Sonnet 5.5 executes");
        assert_eq!(label("claude-sonnet-5"), "Sonnet 5 · pinned");
        assert!(list.models.iter().all(|m| !m.label.contains("latest")));
    }

    #[test]
    fn claude_init_failures_explain_themselves() {
        assert!(parse_claude_init("").is_err());
        assert!(parse_claude_init(
            r#"{"type":"control_response","response":{"subtype":"error","error":"x"}}"#
        )
        .is_err());
        assert!(parse_claude_init(r#"{"type":"control_response","response":{"subtype":"success","response":{"models":[]}}}"#).is_err());
        // An alias that resolves outside its family is not trusted as newest.
        let odd = r#"{"type":"control_response","response":{"subtype":"success","response":{"models":[{"value":"haiku","resolvedModel":"claude-opus-9","displayName":"Haiku ?"}]}}}"#;
        assert!(parse_claude_init(odd).unwrap().newest.is_empty());
    }

    /// Self-hosted models Q5: Claude's list ends with each switched-on
    /// self-hosted model as `self-hosted:<id>`, "<name> · self-hosted", its
    /// window and levels remembered under its served id; a switched-off one
    /// isn't listed, and a missing or damaged registry adds nothing and
    /// never fails the list.
    #[test]
    fn claude_lists_switched_on_self_hosted_models() {
        use crate::self_hosted_registry::{preset, read_at, SelfHostedEntry, FILE};
        let qwen = SelfHostedEntry {
            served_model: "q5-menu-27b".into(),
            ..preset()
        };
        let spare = SelfHostedEntry {
            id: "spare".into(),
            name: "Spare".into(),
            served_model: "q5-menu-spare".into(),
            enabled: false,
            ..preset()
        };
        let base = || ModelCatalog {
            models: builtin_claude_models(),
            source: "Claude Code's model list".into(),
            warning: None,
            checked_at: None,
            newest_by_family: BTreeMap::new(),
        };
        let listed = with_self_hosted(AgentKind::Claude, base(), Ok(vec![qwen, spare]));
        assert_eq!(listed.models.len(), builtin_claude_models().len() + 1);
        assert_eq!(
            listed.models.last(),
            Some(&option("self-hosted:qwen", "Qwen3.8 27B · self-hosted"))
        );
        assert_eq!(
            crate::self_hosted::known_window("q5-menu-27b"),
            Some(131_072)
        );
        assert_eq!(crate::self_hosted::known_window("q5-menu-spare"), None);
        // Effort goes to a level the entry takes (high is between its
        // medium and xhigh; a tie goes lower).
        let effort = crate::effort::resolve(AgentKind::Claude, Some("q5-menu-27b"), Some("high"));
        assert_eq!(effort.effort.as_deref(), Some("medium"));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(
            with_self_hosted(AgentKind::Claude, base(), read_at(&path)).models,
            builtin_claude_models()
        );
        std::fs::write(&path, "{ not json").unwrap();
        let damaged = read_at(&path);
        assert!(damaged.is_err());
        let kept = with_self_hosted(AgentKind::Claude, base(), damaged);
        assert_eq!(kept.models, builtin_claude_models());
        assert_eq!(kept.warning, None);
    }

    /// Q9: an `openai` entry is in Codex's menu as "<name> · self-hosted"
    /// and not in Claude's; an `anthropic` one the other way round. Its
    /// effort levels are registered for Claude only (Codex's
    /// effort comes from the run's target), so `effort::resolve` for Codex
    /// on that id still uses Codex's own levels.
    #[test]
    fn each_harness_lists_only_its_own_api_style() {
        use crate::self_hosted_registry::{preset, SelfHostedEntry, API_OPENAI};
        let claude_side = SelfHostedEntry {
            served_model: "q9-menu-anthropic".into(),
            ..preset()
        };
        let codex_side = SelfHostedEntry {
            id: "qwen-codex".into(),
            name: "Qwen on Codex".into(),
            api_style: API_OPENAI.into(),
            served_model: "q9-menu-openai".into(),
            ..preset()
        };
        let base = || ModelCatalog {
            models: vec![option("gpt-6-luna", "GPT-6 Luna")],
            source: "Codex's model catalog".into(),
            warning: None,
            checked_at: None,
            newest_by_family: BTreeMap::new(),
        };
        let both = || Ok(vec![claude_side.clone(), codex_side.clone()]);
        let codex = with_self_hosted(AgentKind::Codex, base(), both());
        assert_eq!(
            codex.models,
            vec![
                option("gpt-6-luna", "GPT-6 Luna"),
                option("self-hosted:qwen-codex", "Qwen on Codex · self-hosted"),
            ]
        );
        let claude = with_self_hosted(AgentKind::Claude, base(), both());
        assert_eq!(
            claude.models.last(),
            Some(&option("self-hosted:qwen", "Qwen3.8 27B · self-hosted"))
        );
        assert!(!claude
            .models
            .iter()
            .any(|m| m.id == "self-hosted:qwen-codex"));
        // Antigravity and Grok list none.
        let grok = with_self_hosted(AgentKind::Grok, base(), both());
        assert_eq!(grok.models, base().models);
        // Listing doesn't feed Codex's served-id guard: the saved registry
        // does (`self_hosted_tests`), so a menu read of a list that isn't
        // saved refuses nothing.
        assert!(!crate::self_hosted::needs_target_on(
            AgentKind::Codex,
            "q9-menu-openai"
        ));
        assert!(!crate::self_hosted::needs_target_on(
            AgentKind::Codex,
            "q9-menu-anthropic"
        ));
        assert_eq!(
            crate::self_hosted::known_window("q9-menu-openai"),
            Some(131_072)
        );
        // No self-hosted levels were registered for Codex: `high` stays
        // `high` there (Codex's safe levels), while the run itself maps it
        // through its target (`RunTarget::effort`).
        let codex_effort =
            crate::effort::resolve(AgentKind::Codex, Some("q9-menu-openai"), Some("high"));
        assert_eq!(codex_effort.effort.as_deref(), Some("high"));
    }

    #[test]
    fn claude_families_come_from_pinned_ids() {
        assert_eq!(claude_family("claude-opus-4-8"), Some("opus"));
        assert_eq!(claude_family("claude-haiku-4-5-20251001"), Some("haiku"));
        assert_eq!(claude_family("claude-sonnet-5"), Some("sonnet"));
        assert_eq!(claude_family("claude-sonnetx-5"), None);
        assert_eq!(claude_family("sonnet"), None);
    }

    #[tokio::test]
    async fn claude_answers_from_a_fake_cli_without_a_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("stdin.txt");
        let bin = dir.path().join("claude");
        let body = init_text();
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\ncat > '{}'\ncat <<'EOF'\n{body}\nEOF\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let list = ask_claude(&bin).await.unwrap();
        assert_eq!(list.newest["opus"], "claude-opus-5-5");
        let sent = std::fs::read_to_string(&log).unwrap();
        assert!(sent.contains("\"subtype\":\"initialize\""));
        assert!(!sent.contains("\"type\":\"user\""), "no prompt is sent");
    }

    #[test]
    fn builtin_claude_ids_are_valid_and_unique() {
        let models = builtin_claude_models();
        let mut ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.iter().all(|id| valid_id(id)));
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), models.len());
        assert!(models.iter().all(|m| !m.label.contains("latest")));
        // Alias labels agree with what the built-in table says they resolve to.
        let newest = builtin_newest();
        for family in FAMILIES {
            let label = &models.iter().find(|m| m.id == family).unwrap().label;
            let pinned = models
                .iter()
                .find(|m| m.id == newest[family])
                .unwrap()
                .label
                .replace(" · pinned", "");
            assert_eq!(label, &pinned, "{family}");
        }
    }

    #[test]
    fn newest_claude_falls_back_to_the_builtin_table() {
        assert_eq!(newest_claude("haiku").as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(newest_claude("nope"), None);
    }

    #[test]
    fn grok_model_list_parses_default_and_others() {
        let out = "You are logged in with grok.com.\n\nDefault model: grok-4.7\n\nAvailable models:\n  * grok-4.7 (default)\n  - grok-4.7-build-fast\n  - grok-4.6\n  - bad id!\n";
        assert_eq!(
            parse_grok(out),
            vec![
                option("grok-4.7", "grok-4.7 · default"),
                option("grok-4.7-build-fast", "grok-4.7-build-fast"),
                option("grok-4.6", "grok-4.6")
            ]
        );
    }

    /// 2026-11-20: `agy models` lists every level variant; the picker gets
    /// one row per base with the levels it takes, so effort is set once.
    #[test]
    fn agy_variants_group_into_one_row_per_base_model() {
        let rows = parse_agy(
            "Fetching available models...\n\
             gemini-4-neon-high\tGemini 4 Neon (High)\n\
             gemini-4-neon-xhigh\tGemini 4 Neon (XHigh)\n\
             gemini-4-neon-low\tGemini 4 Neon (Low)\n\
             gemini-4-argon-medium\tGemini 4 Argon (Medium)\n\
             gemini-4-argon-high\tGemini 4 Argon (High)\n\
             claude-opus-5-5-max\tClaude Opus 5.5 (Max)\n\
             claude-opus-5-5-low\tClaude Opus 5.5 (Low)\n\
             gemini-pro-orchestrator\tOrchestrator\n",
        );
        let grouped = group_agy(rows);
        let shape: Vec<(&str, &str, Vec<&str>)> = grouped
            .iter()
            .map(|m| {
                (
                    m.id.as_str(),
                    m.label.as_str(),
                    m.levels.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (
                    "gemini-4-neon",
                    "Gemini 4 Neon",
                    vec!["low", "high", "xhigh"]
                ),
                ("gemini-4-argon", "Gemini 4 Argon", vec!["medium", "high"]),
                ("claude-opus-5-5", "Claude Opus 5.5", vec!["low", "max"]),
                ("gemini-pro-orchestrator", "Orchestrator", vec![]),
            ],
            "one row per base, levels lowest first, a model with no variants unchanged"
        );
    }

    #[test]
    fn agy_rows_parse_and_status_lines_are_skipped() {
        assert_eq!(
            parse_agy("Fetching available models...\ngemini-x\tGemini X\ninvalid id\tBad\n\n"),
            vec![option("gemini-x", "Gemini X")]
        );
    }
    #[test]
    fn codex_hides_internal_models_and_keeps_provider_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.json");
        std::fs::write(&path, r#"{"fetched_at":"2026-10-13T15:36:23Z","models":[{"slug":"provider/new","display_name":"New"},{"slug":"internal","visibility":"hide"},{"slug":"bad id"}]}"#).unwrap();
        let (models, fetched) = codex_models(&path).unwrap();
        assert_eq!(models, vec![option("provider/new", "New")]);
        assert_eq!(fetched.as_deref(), Some("2026-10-13T15:36:23Z"));
        std::fs::write(&path, "broken").unwrap();
        assert!(codex_models(&path).is_err());
        std::fs::write(&path, r#"{"models":[]}"#).unwrap();
        assert!(codex_models(&path).is_err());
        assert!(codex_models(&dir.path().join("missing.json"))
            .unwrap_err()
            .contains("Run Codex once"));
        // No fetched_at: the file's own time stands in.
        std::fs::write(&path, r#"{"models":[{"slug":"a"}]}"#).unwrap();
        assert!(codex_models(&path).unwrap().1.is_some());
    }

    #[test]
    fn codex_catalog_goes_stale_after_a_day() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-13T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(!is_stale(Some("2026-10-13T01:00:00Z"), now));
        assert!(is_stale(Some("2026-10-12T11:00:00Z"), now));
        assert!(is_stale(None, now));
        assert!(is_stale(Some("garbage"), now));
    }

    #[tokio::test]
    async fn codex_refresh_reads_the_cli_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("codex");
        std::fs::write(
            &bin,
            "#!/bin/sh\n[ \"$1 $2\" = 'debug models' ] || exit 3\necho '{\"models\":[{\"slug\":\"gpt-new\",\"display_name\":\"GPT New\"}]}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        assert_eq!(
            refresh_codex(&bin).await.unwrap(),
            vec![option("gpt-new", "GPT New")]
        );
        std::fs::write(&bin, "#!/bin/sh\nexit 1\n").unwrap();
        assert!(refresh_codex(&bin).await.is_err());
    }

    #[tokio::test]
    async fn an_unknown_agent_explains_itself() {
        let c = catalog("nope", false).await;
        assert!(c.models.is_empty() && c.warning.is_some());
    }
}
