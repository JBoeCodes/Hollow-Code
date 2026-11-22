//! The CFO (COO rebuild L7, design section 4): not a chat but a Rust module
//! every turn passes through. For each prompt and each subagent it decides
//! the agent, model, effort and account, and records the decision with a
//! one-line reason in the run manifest (`RunManifest::cfo`) and the turn
//! receipt (`EvidenceReceipt::cfo`), which the turn footer shows ("Sonnet 5.5
//! · Medium · code change"). It makes no model call, so it adds no time.
//!
//! The person's explicit choice always wins; an agent's explicit choice wins
//! when the allowlist permits it. Layout:
//! - [`settings`]: the policy (`cfo_settings` table), migrated once from the
//!   removed company code's `company_settings` rows;
//! - [`job`]: the job kind of a prompt;
//! - [`effort`]: the effort per job kind (K2) and who set it;
//! - [`accounts`]: readings, the reserve, holds and the account choice;
//! - [`choose`]: the agent and model of a subagent, from the allowlist.
//!
//! [`budgets`] holds the budgets per project and per subagent (L9).

pub mod accounts;
pub mod budgets;
pub mod choose;
pub mod effort;
pub mod job;
pub mod settings;

use crate::agent::AgentKind;
use crate::commands::AppState;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tauri::State;

pub use effort::EffortSource;
pub use job::{JobKind, Origin};
pub use settings::CfoSettings;

/// What the CFO decided for one turn, with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoDecision"))]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub agent: String,
    /// The model asked for; none means the agent's own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub model: Option<String>,
    /// The level asked for; none means the runtime default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub effort: Option<String>,
    pub effort_source: EffortSource,
    /// The account the CFO put unattended work on (its label), when it chose
    /// one. Absent for the person's prompts: the account spread decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub account: Option<String>,
    pub kind: JobKind,
    pub reason: String,
}

/// One turn the CFO decides for.
pub(crate) struct TurnAsk<'a> {
    pub agent: AgentKind,
    pub model: Option<&'a str>,
    /// The words the job is read from (the person's own text, a task packet,
    /// an automation's prompt).
    pub prompt: &'a str,
    pub origin: Origin,
    /// A build workspace (a worktree): leans Code.
    pub build: bool,
    /// An explicit effort for this turn: the person's per-chat choice or an
    /// agent's `start_session` / `send_to_session` effort.
    pub requested_effort: Option<String>,
    /// Why `start_session` placed the subagent on its agent and model
    /// ([`take_placement`]), joined to the job's reason.
    pub placement: Option<String>,
}

/// The decision for one turn, and the account it should be forced onto
/// (an added Claude account id, for unattended work), if any. `readings` are
/// the readings of the accounts the turn may start on ([`accounts::pool`]).
pub(crate) fn decide_turn(
    settings: &CfoSettings,
    readings: &[accounts::Reading],
    ask: TurnAsk<'_>,
    saved_effort: Option<String>,
) -> (Decision, Option<String>) {
    let (kind, why) = job::classify(ask.prompt, ask.origin, ask.build);
    let (effort, effort_source) = effort::choose(
        ask.requested_effort,
        ask.origin.attended(),
        settings.effort_mode,
        kind,
        saved_effort,
    );
    let (account, account_id) = match accounts::choose_account(
        settings,
        readings,
        ask.agent.as_str(),
        ask.origin.attended(),
        Utc::now(),
    ) {
        accounts::AccountChoice::Use { account, label } => (Some(label), Some(account)),
        accounts::AccountChoice::Spread | accounts::AccountChoice::Hold(_) => (None, None),
    };
    (
        Decision {
            agent: ask.agent.as_str().to_string(),
            model: ask.model.map(str::to_string).filter(|m| !m.is_empty()),
            effort,
            effort_source,
            account,
            kind,
            reason: match ask.placement.filter(|p| !p.trim().is_empty()) {
                Some(placement) => format!("{why}, {placement}"),
                None => why.to_string(),
            },
        },
        account_id,
    )
}

/// The vault's policy for a turn: a damaged row reads as the defaults here
/// (a prompt still runs); unattended starts are held on it instead
/// ([`unattended_hold`]).
pub(crate) fn settings_for(vault: Option<&Path>) -> CfoSettings {
    vault
        .and_then(|v| settings::load(v).ok())
        .unwrap_or_default()
}

/// Read-only form of [`settings_for`] (helper processes such as team-mcp).
pub(crate) fn peek_settings(vault: Option<&Path>) -> CfoSettings {
    vault
        .and_then(|v| settings::peek(v).ok())
        .unwrap_or_default()
}

/// The effort for a run outside a chat turn (an automation, the heartbeat,
/// a teammate): Automatic picks by job kind, Fixed keeps the saved level.
pub(crate) fn run_effort(
    settings: &CfoSettings,
    prompt: &str,
    origin: Origin,
    saved: Option<String>,
) -> Option<String> {
    let (kind, _) = job::classify(prompt, origin, false);
    effort::choose(None, false, settings.effort_mode, kind, saved).0
}

/// Why unattended work on `agent` may not start now, or None. A policy that
/// can't be read holds it with that message.
pub(crate) fn unattended_hold(vault: &Path, agent: &str) -> Option<String> {
    let settings = match settings::load(vault) {
        Ok(settings) => settings,
        Err(error) => return Some(format!("{}{error}", accounts::HOLD_MARK)),
    };
    let pool = accounts::live_pool(&accounts::readings(), agent);
    accounts::hold_for(&settings, &pool, agent, Utc::now())
}

// ── Placement reasons, from `start_session` to the child's first turn ───────

static PLACEMENTS: std::sync::LazyLock<std::sync::Mutex<BTreeMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));
const MAX_PLACEMENTS: usize = 256;

/// Keep why a subagent was placed where it was, under a fresh client turn id
/// its queued turn carries (`cfo-<hex>`); the turn takes it back
/// ([`take_placement`]). In memory only: a restart loses the reason, never
/// the turn.
pub(crate) fn remember_placement(reason: &str) -> String {
    let id = format!("cfo-{:016x}", rand::random::<u64>());
    let mut map = PLACEMENTS.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_PLACEMENTS {
        map.clear();
    }
    map.insert(id.clone(), reason.to_string());
    id
}

/// The placement reason a turn's client turn id carries, once.
pub(crate) fn take_placement(client_turn_id: Option<&str>) -> Option<String> {
    let id = client_turn_id?;
    PLACEMENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id)
}

/// Chat turns queued or running per agent, from the run queue (the "least
/// busy" in [`choose::choose`]).
fn load_by_agent(vault: &Path) -> BTreeMap<String, usize> {
    let mut load = BTreeMap::new();
    if let Ok(rows) = crate::run_queue::live_rows(vault) {
        for row in rows
            .iter()
            .filter(|r| r.source == crate::run_queue::RunSource::Chat)
        {
            if let Some(agent) = row.payload.get("agent").and_then(|a| a.as_str()) {
                *load.entry(agent.to_string()).or_insert(0) += 1;
            }
        }
    }
    load
}

/// A model id in its pinned form: a Claude family alias (`sonnet`) through
/// the catalog's alias map (`agent_models::newest_claude`). Other agents'
/// catalogs expose no alias map, so their ids compare as written.
pub(crate) fn canon(agent: AgentKind, model: &str) -> String {
    let base = model.trim();
    // Antigravity's levels are one Effort control now, so an allow-list entry
    // saved as `gemini-4-neon-high` and a request for `gemini-4-neon` name
    // the same model.
    if agent == AgentKind::Antigravity {
        return crate::effort::agy_base(base).to_string();
    }
    if agent == AgentKind::Claude {
        if let Some(pinned) = crate::agent_models::newest_claude(base) {
            return pinned;
        }
    }
    base.to_string()
}

/// The agent a start with nothing named ran on before the CFO.
fn default_agent() -> Option<AgentKind> {
    crate::agent_settings::resolve(None, None)
        .ok()
        .map(|(kind, _)| kind)
}

/// Where a subagent runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubagentPlace {
    /// None: the app's default agent on its saved model, as before.
    pub agent: Option<String>,
    pub model: Option<String>,
    /// Why, for the child's first turn's decision.
    pub reason: String,
}

/// The agent and model a subagent runs on (`start_session`): a named agent
/// or model is checked against the allowlist; nothing named, or `auto`,
/// is the CFO's pick ([`choose::choose`]). `producer` gives the calling
/// chat's agent, which a review avoids (read only for a review). A policy
/// that can't be read counts as the defaults here, as for turns.
pub(crate) async fn place_subagent(
    vault: &Path,
    requested_agent: Option<&str>,
    requested_model: Option<&str>,
    prompt: &str,
    producer: impl FnOnce() -> Option<AgentKind>,
) -> Result<SubagentPlace, String> {
    let settings = settings_for(Some(vault));
    let saved = crate::agent_settings::get_settings().unwrap_or_default();
    let installed = crate::agent::available();
    let (kind, _) = job::classify(prompt, Origin::Subagent, false);
    let producer = (kind == JobKind::Review).then(producer).flatten();
    let agent_named = requested_agent
        .map(str::trim)
        .is_some_and(|a| !a.is_empty() && a != "auto");
    let model_named = requested_model
        .map(str::trim)
        .is_some_and(|m| !m.is_empty() && m != "auto");
    // The catalogs matter only for a model named with no agent (M2).
    let mut catalogs = BTreeMap::new();
    if model_named && !agent_named {
        for agent in &installed {
            let catalog = crate::agent_models::catalog(agent.as_str(), false).await;
            let ids: BTreeSet<String> = catalog
                .models
                .into_iter()
                .map(|m| m.id)
                .chain(catalog.newest_by_family.into_values())
                .collect();
            catalogs.insert(agent.as_str().to_string(), ids);
        }
    }
    let inputs = choose::Inputs {
        settings: &settings,
        installed: &installed,
        saved: &saved.models,
        default_agent: default_agent(),
        load: &load_by_agent(vault),
        catalogs: &catalogs,
        canon: &canon,
    };
    let placed = choose::choose(&inputs, requested_agent, requested_model, kind, producer)?;
    Ok(SubagentPlace {
        agent: placed.agent.map(|a| a.as_str().to_string()),
        model: placed.model,
        reason: placed.reason,
    })
}

/// M4: an AGENT naming an agent or model (`send_to_session`, `handover`,
/// `ask_agent`) stays inside a non-empty allowlist; a model left out is the
/// agent's allowlist or saved model. The person is never checked here.
pub(crate) fn check_named(
    settings: &CfoSettings,
    agent: AgentKind,
    model: Option<&str>,
) -> Result<(), String> {
    if settings.allow.is_empty() {
        return Ok(());
    }
    let saved = crate::agent_settings::get_settings()
        .map(|s| s.models)
        .unwrap_or_default();
    let model = choose::named_model(
        settings,
        &saved,
        agent,
        model.filter(|m| !m.trim().is_empty()),
    );
    if choose::allows(settings, agent, model.as_deref(), &canon) {
        Ok(())
    } else {
        Err(choose::refusal(settings, agent, model.as_deref()))
    }
}

/// The level each job kind runs at, for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CfoKindLevel {
    pub kind: JobKind,
    pub level: String,
}

/// Settings, Usage: the policy, each account against the reserve, and the
/// level per job kind.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CfoView {
    pub settings: CfoSettings,
    pub pool_ceiling_percent: f64,
    pub extra_usage_open: bool,
    pub accounts: Vec<accounts::CfoAccount>,
    pub levels: Vec<CfoKindLevel>,
    pub reserve_choices: Vec<u8>,
    /// L9: each project budget's usage now (rolling 5 hours and 7 days).
    pub project_usage: Vec<CfoProjectUsage>,
}

/// One project budget's usage, for Settings, Usage (L9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CfoProjectUsage {
    pub workspace_id: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub five_hour_tokens: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub week_tokens: u64,
    /// The budget is reached: the project's unattended work waits.
    pub reached: bool,
}

const KINDS: [JobKind; 7] = [
    JobKind::Conversation,
    JobKind::Question,
    JobKind::Report,
    JobKind::Review,
    JobKind::Research,
    JobKind::Plan,
    JobKind::Code,
];

pub fn view(vault: &Path) -> Result<CfoView, String> {
    let settings = settings::load(vault)?;
    let now = Utc::now();
    Ok(CfoView {
        pool_ceiling_percent: settings.pool_ceiling(),
        extra_usage_open: settings.extra_usage_open(now),
        accounts: accounts::evaluate(&settings, &accounts::readings(), now),
        levels: KINDS
            .iter()
            .map(|kind| CfoKindLevel {
                kind: *kind,
                level: effort::level_for(*kind).to_string(),
            })
            .collect(),
        reserve_choices: settings::RESERVE_CHOICES.to_vec(),
        project_usage: settings
            .project_budgets
            .iter()
            .filter_map(|budget| {
                let (five, week) =
                    budgets::project_windows(vault, &budget.workspace_id, now).ok()?;
                Some(CfoProjectUsage {
                    workspace_id: budget.workspace_id.clone(),
                    five_hour_tokens: five,
                    week_tokens: week,
                    reached: budgets::project_reached(budget, "", five, week).is_some(),
                })
            })
            .collect(),
        settings,
    })
}

/// Each allowlist model must be one of its agent's real models: the ids of
/// its catalog (and the pinned ids its aliases resolve to). `catalogs` maps
/// an agent to those ids; an agent whose list could not be read takes no
/// narrowed entry (an "any model" entry still saves).
pub(crate) fn check_models(
    settings: &CfoSettings,
    catalogs: &BTreeMap<String, BTreeSet<String>>,
) -> Result<(), String> {
    for entry in &settings.allow {
        let Some(model) = entry.model.as_deref() else {
            continue;
        };
        let label = AgentKind::from_opt(&entry.agent)
            .map(|k| k.label())
            .unwrap_or("This agent");
        match catalogs.get(&entry.agent) {
            Some(ids) if ids.contains(model) => {}
            Some(ids) if ids.is_empty() => {
                return Err(format!(
                    "CFO_SETTINGS: {label}'s model list could not be read, so `{model}` can't be checked; allow any model of {label} or try again once it lists its models."
                ))
            }
            _ => {
                return Err(format!(
                    "CFO_SETTINGS: `{model}` is not one of {label}'s models."
                ))
            }
        }
    }
    Ok(())
}

async fn catalogs_for(settings: &CfoSettings) -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    let agents: BTreeSet<String> = settings
        .allow
        .iter()
        .filter(|e| e.model.is_some())
        .map(|e| e.agent.clone())
        .collect();
    for agent in agents {
        let catalog = crate::agent_models::catalog(&agent, false).await;
        let ids: BTreeSet<String> = catalog
            .models
            .into_iter()
            .map(|m| m.id)
            .chain(catalog.newest_by_family.into_values())
            .collect();
        out.insert(agent, ids);
    }
    out
}

pub(crate) fn configured_vault(state: &AppState) -> Result<PathBuf, String> {
    if !state.configured || !state.config.vault_path.is_dir() {
        return Err("CFO_UNCONFIGURED: connect a vault first".into());
    }
    Ok(state.config.vault_path.clone())
}

/// The policy, for `cfo_settings` and the COO's `settings.cfo_settings`.
pub(crate) async fn settings_in(vault: PathBuf) -> Result<CfoSettings, String> {
    crate::vault::spawn_blocking_logged(move || settings::load(&vault))
        .await
        .map_err(|e| e.to_string())?
}

/// Each account against the reserve, for `cfo_view` and the COO's
/// `settings.cfo_view`. Makes no provider request.
pub(crate) async fn view_in(vault: PathBuf) -> Result<CfoView, String> {
    crate::vault::spawn_blocking_logged(move || view(&vault))
        .await
        .map_err(|e| e.to_string())?
}

/// Save one panel's patch (`cfo_save_settings`, and the COO's
/// `settings.cfo_save_settings` once the person allowed its card): each
/// allowlist model is checked against its agent's real list first.
pub(crate) async fn save_settings_in(
    vault: PathBuf,
    patch: settings::Patch,
) -> Result<CfoView, String> {
    let catalogs = match &patch.allow {
        Some(allow) => {
            let candidate = settings::normalize(CfoSettings {
                allow: allow.clone(),
                ..CfoSettings::default()
            })?;
            catalogs_for(&candidate).await
        }
        None => BTreeMap::new(),
    };
    crate::vault::spawn_blocking_logged(move || save_checked(&vault, patch, &catalogs))
        .await
        .map_err(|e| e.to_string())?
}

/// [`save_settings_in`] once the catalogs are read: refuses a model that is
/// not one of its agent's, before anything is written.
pub(crate) fn save_checked(
    vault: &Path,
    patch: settings::Patch,
    catalogs: &BTreeMap<String, BTreeSet<String>>,
) -> Result<CfoView, String> {
    if let Some(allow) = &patch.allow {
        let candidate = settings::normalize(CfoSettings {
            allow: allow.clone(),
            ..CfoSettings::default()
        })?;
        check_models(&candidate, catalogs)?;
    }
    settings::save_patch(vault, patch)?;
    view(vault)
}

/// Allow extra usage for unattended work for `hours` (1 to 72), or
/// withdraw it with 0 (`cfo_approve_extra_usage`, and the COO's
/// `settings.cfo_approve_extra_usage` once the person allowed its card).
pub(crate) async fn approve_extra_usage_in(vault: PathBuf, hours: u32) -> Result<CfoView, String> {
    crate::vault::spawn_blocking_logged(move || {
        settings::approve_extra_usage(&vault, hours)?;
        view(&vault)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The COO's card for `settings.cfo_save_settings`: each field the patch
/// names, old to new. Refuses an empty patch, one that changes nothing, and
/// one `normalize` refuses (the model lists are checked when it runs).
pub(crate) fn save_card_lines(
    old: &CfoSettings,
    patch: &settings::Patch,
) -> Result<Vec<String>, String> {
    if patch.allow.is_none() && patch.effort_mode.is_none() && patch.reserve_percent.is_none() {
        return Err("Name allow, effortMode or reservePercent to change. Nothing changed.".into());
    }
    let mut new = old.clone();
    patch.clone().apply(&mut new);
    let new = settings::normalize(new)?;
    let mut lines = Vec::new();
    if patch.allow.is_some() && new.allow != old.allow {
        lines.push(format!(
            "Models the CFO may use: {} to {}.",
            allow_text(&old.allow),
            allow_text(&new.allow)
        ));
    }
    if new.effort_mode != old.effort_mode {
        lines.push(format!(
            "Effort: {} to {}.",
            effort_text(old.effort_mode),
            effort_text(new.effort_mode)
        ));
    }
    if new.reserve_percent != old.reserve_percent {
        lines.push(format!(
            "Reserve kept for the person's own prompts: {}% to {}%.",
            old.reserve_percent, new.reserve_percent
        ));
    }
    if lines.is_empty() {
        return Err("Those are already the CFO's settings. Nothing changed.".into());
    }
    Ok(lines)
}

fn allow_text(allow: &[settings::AllowEntry]) -> String {
    if allow.is_empty() {
        return "every installed agent on its saved model".into();
    }
    allow
        .iter()
        .map(|e| {
            let label = AgentKind::from_opt(&e.agent)
                .map(|k| k.label().to_string())
                .unwrap_or_else(|| e.agent.clone());
            match &e.model {
                Some(model) => format!("{label} {model}"),
                None => format!("{label} (any model)"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn effort_text(mode: settings::EffortMode) -> &'static str {
    match mode {
        settings::EffortMode::Automatic => "Automatic (the CFO picks per job kind)",
        settings::EffortMode::Fixed => "Fixed (each agent's saved level)",
    }
}

/// The COO's card line for `settings.cfo_approve_extra_usage`. Refuses
/// more than [`settings::MAX_EXTRA_HOURS`] before any card.
pub(crate) fn approve_card_line(hours: u32) -> Result<String, String> {
    match hours {
        0 => Ok("Withdraw the extra usage approved for unattended work: from now on it stops at the reserve.".into()),
        h if h <= settings::MAX_EXTRA_HOURS => Ok(format!(
            "Allow extra usage for unattended work for the next {h} hour{}: it may run past the reserve kept for the person's own prompts, which can spend paid extra usage.",
            if h == 1 { "" } else { "s" }
        )),
        _ => Err(format!(
            "CFO_SETTINGS: approve extra usage for at most {} hours at a time. Nothing changed.",
            settings::MAX_EXTRA_HOURS
        )),
    }
}

/// Settings, Models: the CFO's policy (the COO's `settings.cfo_settings`).
#[tauri::command]
pub async fn cfo_settings(state: State<'_, AppState>) -> Result<CfoSettings, String> {
    let vault = configured_vault(&state)?;
    settings_in(vault).await
}

/// Save what one panel owns (the allowlist and effort mode from Settings,
/// Models, the reserve from Settings, Usage) in one transaction; fields
/// left out, and an extra-usage approval, stay as they are. Each allowlist
/// model is checked against its agent's real list. The COO's
/// `settings.cfo_save_settings` does the same after a card.
#[tauri::command]
pub async fn cfo_save_settings(
    patch: settings::Patch,
    state: State<'_, AppState>,
) -> Result<CfoView, String> {
    let vault = configured_vault(&state)?;
    save_settings_in(vault, patch).await
}

/// Allow extra usage for unattended work for `hours` (1 to 72), or withdraw
/// it with 0. The COO's `settings.cfo_approve_extra_usage` does the same
/// after a card.
#[tauri::command]
pub async fn cfo_approve_extra_usage(
    hours: u32,
    state: State<'_, AppState>,
) -> Result<CfoView, String> {
    let vault = configured_vault(&state)?;
    approve_extra_usage_in(vault, hours).await
}

/// Settings, Usage: each account against the reserve. Makes no provider
/// request (the COO's `settings.cfo_view`).
#[tauri::command]
pub async fn cfo_view(state: State<'_, AppState>) -> Result<CfoView, String> {
    let vault = configured_vault(&state)?;
    view_in(vault).await
}

#[cfg(test)]
mod tests {
    use super::settings::{AllowEntry, EffortMode};
    use super::*;

    fn ask<'a>(prompt: &'a str, origin: Origin, requested: Option<&str>) -> TurnAsk<'a> {
        TurnAsk {
            agent: AgentKind::Claude,
            model: Some("claude-sonnet-5-5"),
            prompt,
            origin,
            build: false,
            requested_effort: requested.map(str::to_string),
            placement: None,
        }
    }

    #[test]
    fn the_placement_reason_joins_the_jobs_and_travels_once() {
        let id = remember_placement("independent of Claude");
        assert!(id.starts_with("cfo-"));
        let placement = take_placement(Some(&id));
        assert_eq!(placement.as_deref(), Some("independent of Claude"));
        assert_eq!(take_placement(Some(&id)), None, "taken once");
        let (decision, _) = decide_turn(
            &CfoSettings::default(),
            &[],
            TurnAsk {
                placement,
                ..ask("Please review this diff", Origin::Subagent, None)
            },
            None,
        );
        assert_eq!(decision.reason, "review, independent of Claude");
    }

    #[test]
    fn an_agent_naming_a_model_stays_inside_the_list() {
        let open = CfoSettings::default();
        assert!(check_named(&open, AgentKind::Codex, Some("anything")).is_ok());
        let list = CfoSettings {
            allow: vec![AllowEntry {
                agent: "claude".into(),
                model: Some("claude-sonnet-5-5".into()),
            }],
            ..CfoSettings::default()
        };
        assert!(check_named(&list, AgentKind::Claude, Some("claude-sonnet-5-5")).is_ok());
        let err = check_named(&list, AgentKind::Claude, Some("claude-opus-5-5")).unwrap_err();
        assert!(
            err.contains("not among the models the CFO may use"),
            "{err}"
        );
        assert!(check_named(&list, AgentKind::Codex, None).is_err());
        // With no model named, the agent's allowlist model applies.
        assert!(check_named(&list, AgentKind::Claude, None).is_ok());
    }

    /// 2026-11-20: Antigravity's levels are one Effort control, so an
    /// allow-list entry saved as a variant permits its base and the reverse.
    #[test]
    fn an_antigravity_variant_in_the_list_permits_its_base_model() {
        let list = CfoSettings {
            allow: vec![AllowEntry {
                agent: "antigravity".into(),
                model: Some("gemini-4-neon-high".into()),
            }],
            ..CfoSettings::default()
        };
        for model in ["gemini-4-neon", "gemini-4-neon-xhigh", "gemini-4-neon-high"] {
            assert!(
                check_named(&list, AgentKind::Antigravity, Some(model)).is_ok(),
                "{model}"
            );
        }
        assert!(check_named(&list, AgentKind::Antigravity, Some("gemini-4-argon")).is_err());
    }

    #[test]
    fn the_persons_override_wins_over_the_cfo() {
        let settings = CfoSettings::default();
        let (mine, _) = decide_turn(
            &settings,
            &[],
            ask("What is 2 + 2?", Origin::Person, Some("max")),
            Some("high".into()),
        );
        assert_eq!(
            (mine.effort.as_deref(), mine.effort_source),
            (Some("max"), EffortSource::You)
        );
        let (cfo, _) = decide_turn(
            &settings,
            &[],
            ask("What is 2 + 2?", Origin::Person, None),
            Some("high".into()),
        );
        assert_eq!(
            (cfo.effort.as_deref(), cfo.effort_source, cfo.kind),
            (Some("low"), EffortSource::Cfo, JobKind::Question)
        );
        assert_eq!(cfo.reason, "short question");
        assert_eq!(cfo.account, None, "the person's prompt keeps its spread");
        let fixed = CfoSettings {
            effort_mode: EffortMode::Fixed,
            ..CfoSettings::default()
        };
        let (saved, _) = decide_turn(
            &fixed,
            &[],
            ask("What is 2 + 2?", Origin::Person, None),
            Some("high".into()),
        );
        assert_eq!(
            (saved.effort.as_deref(), saved.effort_source),
            (Some("high"), EffortSource::Fixed)
        );
    }

    #[test]
    fn unattended_work_is_put_on_the_soonest_reset_account() {
        let now = Utc::now();
        let reading = |id: &str, used: f64, hours: i64| accounts::Reading {
            agent: "claude".into(),
            account: id.into(),
            label: format!("Account {id}"),
            enabled: true,
            five_hour: Some(accounts::Window {
                used,
                resets_at: Some((now + chrono::Duration::hours(hours)).to_rfc3339()),
            }),
            weekly: None,
            observed_at: Some(now),
        };
        let pool = [reading("a", 10.0, 4), reading("b", 40.0, 1)];
        let (decision, id) = decide_turn(
            &CfoSettings::default(),
            &pool,
            ask("Summarise the week", Origin::Automation, None),
            None,
        );
        assert_eq!(id.as_deref(), Some("b"));
        assert_eq!(decision.account.as_deref(), Some("Account b"));
        assert_eq!(decision.kind, JobKind::Report);
        assert_eq!(decision.effort.as_deref(), Some("medium"));
    }

    #[test]
    fn allowlist_models_must_be_real_ones() {
        let settings = CfoSettings {
            allow: vec![
                AllowEntry {
                    agent: "claude".into(),
                    model: Some("claude-sonnet-5-5".into()),
                },
                AllowEntry {
                    agent: "codex".into(),
                    model: None,
                },
            ],
            ..CfoSettings::default()
        };
        let catalogs = BTreeMap::from([(
            "claude".to_string(),
            BTreeSet::from(["sonnet".to_string(), "claude-sonnet-5-5".to_string()]),
        )]);
        assert!(check_models(&settings, &catalogs).is_ok());
        let typed = CfoSettings {
            allow: vec![AllowEntry {
                agent: "claude".into(),
                model: Some("my-made-up-model".into()),
            }],
            ..CfoSettings::default()
        };
        let err = check_models(&typed, &catalogs).unwrap_err();
        assert!(err.contains("not one of Claude"), "{err}");
        let unread = BTreeMap::from([("claude".to_string(), BTreeSet::new())]);
        assert!(check_models(&settings, &unread)
            .unwrap_err()
            .contains("could not be read"));
    }

    #[test]
    fn the_view_lists_the_level_per_kind() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let view = view(dir.path()).unwrap();
        assert_eq!(view.pool_ceiling_percent, 80.0);
        assert_eq!(view.levels.len(), 7);
        assert!(view.reserve_choices.contains(&20));
        assert!(!view.extra_usage_open);
    }
}
