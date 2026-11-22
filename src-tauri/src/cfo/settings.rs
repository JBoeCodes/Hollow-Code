//! The CFO's policy (COO rebuild L7): which models it may use, how effort is
//! chosen, the reserve kept for the person's own prompts and any extra usage
//! the person approved. One JSON row (key `policy`) in the vault's
//! `cfo_settings` table. The person changes it in Settings, Models and
//! Settings, Usage; the COO changes it only through confirm actions whose
//! card the person allows (`settings.cfo_save_settings`,
//! `settings.cfo_approve_extra_usage`, L13).
//!
//! Migration: a vault with no `policy` row reads the removed company code's
//! `company_settings` rows once (`routing`'s allowlist; `cfo`'s reserve plus
//! buffer as the reserve, capped at [`MAX_RESERVE`], and its extra-usage
//! approval), writes the `policy` row, and never reads them again. The
//! company rows are left as they are.

use crate::agent::AgentKind;
use crate::session_store;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS cfo_settings(
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS cfo_subagent_budgets(
  session_key TEXT PRIMARY KEY,
  tokens INTEGER,
  minutes INTEGER,
  parent_key TEXT,
  paused_at TEXT,
  paused_reason TEXT,
  set_at TEXT NOT NULL
);";

const POLICY_KEY: &str = "policy";
const MAX_ENTRIES: usize = 32;
/// Room kept for the person's own prompts by default: the same 80% long-work
/// line `claude_accounts` used before the CFO, so behaviour does not jump.
pub const DEFAULT_RESERVE: u8 = 20;
pub const MAX_RESERVE: u8 = 90;
/// Extra usage is approved for at most this many hours at a time.
pub const MAX_EXTRA_HOURS: u32 = 72;
/// The reserve choices Settings, Usage offers.
pub const RESERVE_CHOICES: [u8; 6] = [0, 10, 20, 30, 40, 50];
/// At most this many project budgets (L9).
const MAX_PROJECT_BUDGETS: usize = 64;
/// A token cap is at most this (10 billion): anything larger is a typo.
pub const MAX_BUDGET_TOKENS: u64 = 10_000_000_000;
/// A time cap is at most a week of working time, in minutes.
pub const MAX_BUDGET_MINUTES: u32 = 7 * 24 * 60;

/// One project's budget (L9): tokens over the last 5 hours and the last 7
/// days, rolling like the accounts' own windows. A missing figure is no cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoProjectBudget"))]
#[serde(rename_all = "camelCase")]
pub struct ProjectBudget {
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub five_hour_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub week_tokens: Option<u64>,
}

/// A subagent's budget (L9): its whole chain's tokens and working time. The
/// policy's is every subagent's default; `cfo_subagent_budgets` overrides it
/// for one. A missing figure is no cap.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoSubagentBudget"))]
#[serde(rename_all = "camelCase")]
pub struct SubagentBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub minutes: Option<u32>,
}

impl SubagentBudget {
    pub fn is_empty(&self) -> bool {
        self.tokens.is_none() && self.minutes.is_none()
    }
}

/// One model the CFO may use: an agent, optionally narrowed to one model.
/// `model: None` allows every model of that agent (its saved model unless a
/// task names one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoAllowEntry"))]
#[serde(rename_all = "camelCase")]
pub struct AllowEntry {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub model: Option<String>,
}

/// How each turn's effort is chosen when nobody named one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoEffortMode"))]
#[serde(rename_all = "camelCase")]
pub enum EffortMode {
    /// The CFO picks a level per job kind (K2).
    #[default]
    Automatic,
    /// Each agent's saved level (`agent_settings.efforts`), as before.
    Fixed,
}

/// The person's CFO policy for this vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase", default)]
pub struct CfoSettings {
    /// Empty: every installed agent, each on its saved model.
    pub allow: Vec<AllowEntry>,
    pub effort_mode: EffortMode,
    /// Percent of every account's 5-hour and weekly window kept for the
    /// person's own prompts.
    pub reserve_percent: u8,
    /// Unattended work may use extra usage until then (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub extra_usage_until: Option<String>,
    /// Per-project budgets (L9). Empty: no project is capped.
    pub project_budgets: Vec<ProjectBudget>,
    /// Every subagent's default budget (L9). Empty: no subagent is capped.
    pub subagent_budget: SubagentBudget,
}

impl Default for CfoSettings {
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            effort_mode: EffortMode::Automatic,
            reserve_percent: DEFAULT_RESERVE,
            extra_usage_until: None,
            project_budgets: Vec::new(),
            subagent_budget: SubagentBudget::default(),
        }
    }
}

impl CfoSettings {
    /// The highest usage percent at which unattended work may still start.
    pub fn pool_ceiling(&self) -> f64 {
        100.0 - f64::from(self.reserve_percent.min(MAX_RESERVE))
    }

    pub fn extra_usage_open(&self, now: DateTime<Utc>) -> bool {
        self.extra_usage_until
            .as_deref()
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|until| until.with_timezone(&Utc) > now)
    }
}

/// Check and tidy a policy before it is saved (the model lists are checked
/// separately, against each agent's real catalog: `super::save`).
pub(crate) fn normalize(mut settings: CfoSettings) -> Result<CfoSettings, String> {
    if settings.allow.len() > MAX_ENTRIES {
        return Err(format!(
            "CFO_SETTINGS: at most {MAX_ENTRIES} allowed models"
        ));
    }
    if settings.reserve_percent > MAX_RESERVE {
        return Err(format!(
            "CFO_SETTINGS: the reserve is at most {MAX_RESERVE}%"
        ));
    }
    let mut seen = Vec::new();
    for entry in &mut settings.allow {
        entry.agent = entry.agent.trim().to_ascii_lowercase();
        if AgentKind::from_opt(&entry.agent).is_none() {
            return Err(format!("CFO_SETTINGS: unknown agent `{}`", entry.agent));
        }
        entry.model = entry
            .model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string);
        if entry
            .model
            .as_deref()
            .is_some_and(|m| m.len() > 128 || !m.bytes().all(|b| b.is_ascii_graphic()))
        {
            return Err("CFO_SETTINGS: model names are short plain identifiers".into());
        }
        let key = (entry.agent.clone(), entry.model.clone());
        if seen.contains(&key) {
            return Err("CFO_SETTINGS: a model is listed twice".into());
        }
        seen.push(key);
    }
    if let Some(until) = &settings.extra_usage_until {
        DateTime::parse_from_rfc3339(until)
            .map_err(|_| "CFO_SETTINGS: the extra usage expiry is not a time".to_string())?;
    }
    if settings.project_budgets.len() > MAX_PROJECT_BUDGETS {
        return Err(format!(
            "CFO_SETTINGS: at most {MAX_PROJECT_BUDGETS} project budgets"
        ));
    }
    let mut projects: Vec<String> = Vec::new();
    for budget in &mut settings.project_budgets {
        budget.workspace_id = budget.workspace_id.trim().to_string();
        let id = &budget.workspace_id;
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("CFO_SETTINGS: a project budget names no project".into());
        }
        if projects.contains(id) {
            return Err("CFO_SETTINGS: a project has two budgets".into());
        }
        projects.push(id.clone());
        check_tokens(budget.five_hour_tokens)?;
        check_tokens(budget.week_tokens)?;
    }
    // A budget with neither figure caps nothing: dropped.
    settings
        .project_budgets
        .retain(|b| b.five_hour_tokens.is_some() || b.week_tokens.is_some());
    check_subagent_budget(&settings.subagent_budget)?;
    Ok(settings)
}

fn check_tokens(tokens: Option<u64>) -> Result<(), String> {
    match tokens {
        Some(0) => Err("CFO_SETTINGS: a token budget is more than 0".into()),
        Some(t) if t > MAX_BUDGET_TOKENS => {
            Err("CFO_SETTINGS: a token budget is at most 10 billion".into())
        }
        _ => Ok(()),
    }
}

/// A subagent budget's figures are in range (also the agent action's check).
pub(crate) fn check_subagent_budget(budget: &SubagentBudget) -> Result<(), String> {
    check_tokens(budget.tokens)?;
    match budget.minutes {
        Some(0) => Err("CFO_SETTINGS: a time budget is at least 1 minute".into()),
        Some(m) if m > MAX_BUDGET_MINUTES => {
            Err("CFO_SETTINGS: a time budget is at most a week of working time".into())
        }
        _ => Ok(()),
    }
}

fn read_row(conn: &Connection) -> Result<Option<CfoSettings>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM cfo_settings WHERE key=?1",
            [POLICY_KEY],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("cfo settings read: {e}"))?;
    match raw {
        None => Ok(None),
        // A damaged row fails closed for unattended work (its callers hold
        // it with this message); the person's prompts still run.
        Some(raw) => serde_json::from_str(&raw).map(Some).map_err(|_| {
            "CFO_CORRUPT: the CFO settings are unreadable; save them again in Settings → Models"
                .to_string()
        }),
    }
}

fn write_in(tx: &Transaction<'_>, settings: &CfoSettings) -> Result<(), String> {
    let value = serde_json::to_string(settings).map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO cfo_settings(key,value,updated_at) VALUES(?1,?2,?3)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        params![POLICY_KEY, value, session_store::db_time(Utc::now())],
    )
    .map_err(|e| format!("cfo settings write: {e}"))?;
    Ok(())
}

/// One `company_settings` row as JSON, when that table and row exist and
/// parse. Read only by the one-time migration, with plain SQL (the company
/// module is paused and is deleted later).
fn company_row(conn: &Connection, key: &str) -> Option<serde_json::Value> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='company_settings'",
            [],
            |_| Ok(true),
        )
        .optional()
        .ok()
        .flatten()
        .unwrap_or(false);
    if !exists {
        return None;
    }
    let raw: String = conn
        .query_row(
            "SELECT value FROM company_settings WHERE key=?1",
            [key],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    serde_json::from_str(&raw).ok()
}

/// The policy a vault with no `policy` row starts from: the company rows'
/// allowlist, reserve plus buffer and extra usage, else the defaults. A
/// damaged or invalid company row is ignored (its part keeps the default).
pub(crate) fn migrated_in(conn: &Connection) -> CfoSettings {
    let mut settings = CfoSettings::default();
    if let Some(routing) = company_row(conn, "routing") {
        let allow: Vec<AllowEntry> = routing
            .get("allow")
            .and_then(|a| serde_json::from_value(a.clone()).ok())
            .unwrap_or_default();
        let candidate = CfoSettings {
            allow,
            ..CfoSettings::default()
        };
        if let Ok(valid) = normalize(candidate) {
            settings.allow = valid.allow;
        }
    }
    if let Some(policy) = company_row(conn, "cfo") {
        let percent = |key: &str| policy.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        if policy.get("reservePercent").is_some() {
            let total = percent("reservePercent") + percent("bufferPercent");
            settings.reserve_percent = total.min(u64::from(MAX_RESERVE)) as u8;
        }
        settings.extra_usage_until = policy
            .get("extraUsageUntil")
            .and_then(|v| v.as_str())
            .filter(|t| DateTime::parse_from_rfc3339(t).is_ok())
            .map(str::to_string);
    }
    settings
}

/// The vault's policy, migrating once when it has none yet.
pub fn load(vault: &Path) -> Result<CfoSettings, String> {
    let settings = match session_store::with_read(vault, read_row)? {
        Some(settings) => settings,
        None => session_store::with_write(vault, "cfo settings", |tx| {
            if let Some(settings) = read_row(tx)? {
                return Ok(settings);
            }
            let settings = migrated_in(tx);
            write_in(tx, &settings)?;
            Ok(settings)
        })?,
    };
    publish_reserve(&settings);
    Ok(settings)
}

/// Read without writing (an MCP helper process, a read that must not take
/// the write lock): the row, else what the migration would write.
pub fn peek(vault: &Path) -> Result<CfoSettings, String> {
    session_store::with_read(vault, |conn| {
        Ok(match read_row(conn)? {
            Some(settings) => settings,
            None => migrated_in(conn),
        })
    })
}

/// Save the whole policy (tests; the app saves through [`update`]).
#[cfg(test)]
pub fn save(vault: &Path, settings: CfoSettings) -> Result<CfoSettings, String> {
    let settings = normalize(settings)?;
    session_store::with_write(vault, "cfo settings", |tx| write_in(tx, &settings))?;
    publish_reserve(&settings);
    Ok(settings)
}

/// What one Settings panel changes: only the fields it owns, so the Models
/// rows and the Usage rows never overwrite each other's (review L4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoPatch"))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Patch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub allow: Option<Vec<AllowEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub effort_mode: Option<EffortMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reserve_percent: Option<u8>,
    /// Usage (L9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub project_budgets: Option<Vec<ProjectBudget>>,
    /// Usage (L9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub subagent_budget: Option<SubagentBudget>,
}

impl Patch {
    pub(crate) fn apply(self, settings: &mut CfoSettings) {
        if let Some(allow) = self.allow {
            settings.allow = allow;
        }
        if let Some(mode) = self.effort_mode {
            settings.effort_mode = mode;
        }
        if let Some(reserve) = self.reserve_percent {
            settings.reserve_percent = reserve;
        }
        if let Some(budgets) = self.project_budgets {
            settings.project_budgets = budgets;
        }
        if let Some(budget) = self.subagent_budget {
            settings.subagent_budget = budget;
        }
    }
}

/// Read, change and write the policy in ONE transaction (review L4): two
/// saves at once never lose each other's change.
pub fn update(vault: &Path, change: impl FnOnce(&mut CfoSettings)) -> Result<CfoSettings, String> {
    let settings = session_store::with_write(vault, "cfo settings", |tx| {
        let mut settings = match read_row(tx) {
            Ok(Some(settings)) => settings,
            // No row yet, or a damaged one the person is saving over.
            Ok(None) | Err(_) => migrated_in(tx),
        };
        change(&mut settings);
        let settings = normalize(settings)?;
        write_in(tx, &settings)?;
        Ok(settings)
    })?;
    publish_reserve(&settings);
    Ok(settings)
}

/// Apply one panel's patch (already checked against the model
/// catalogs). An extra-usage approval stays as it is.
pub fn save_patch(vault: &Path, patch: Patch) -> Result<CfoSettings, String> {
    update(vault, |settings| patch.apply(settings))
}

/// Allow extra usage for unattended work for `hours` (1 to
/// [`MAX_EXTRA_HOURS`]), or withdraw it with `hours = 0`.
pub fn approve_extra_usage(vault: &Path, hours: u32) -> Result<CfoSettings, String> {
    if hours > MAX_EXTRA_HOURS {
        return Err(format!(
            "CFO_SETTINGS: approve extra usage for at most {MAX_EXTRA_HOURS} hours at a time"
        ));
    }
    update(vault, |settings| {
        settings.extra_usage_until =
            (hours > 0).then(|| (Utc::now() + Duration::hours(i64::from(hours))).to_rfc3339());
    })
}

/// App start (review L5): the long-work line follows the saved reserve
/// before any turn reads the policy.
pub fn publish_at_startup(vault: &Path) {
    if let Ok(settings) = peek(vault) {
        publish_reserve(&settings);
    }
}

/// The long-work ceiling `claude_accounts` routes unattended Claude runs by
/// follows the reserve. Not in tests: they run in parallel, and a global
/// set here would move other tests' routing.
fn publish_reserve(settings: &CfoSettings) {
    #[cfg(not(test))]
    crate::claude_accounts::set_long_work_reserve(settings.reserve_percent);
    #[cfg(test)]
    let _ = settings;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn company(vault: &Path, key: &str, value: &str) {
        session_store::with_write(vault, "test", |tx| {
            tx.execute_batch(
                "CREATE TABLE IF NOT EXISTS company_settings(key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at TEXT NOT NULL);",
            )
            .map_err(|e| e.to_string())?;
            tx.execute(
                "INSERT INTO company_settings(key,value,updated_at) VALUES(?1,?2,'x')
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn defaults_are_automatic_with_a_twenty_percent_reserve() {
        let dir = vault();
        let settings = load(dir.path()).unwrap();
        assert_eq!(settings, CfoSettings::default());
        assert_eq!(settings.effort_mode, EffortMode::Automatic);
        assert_eq!(settings.pool_ceiling(), 80.0);
    }

    #[test]
    fn migration_from_company_settings_happens_once() {
        let dir = vault();
        company(
            dir.path(),
            "routing",
            r#"{"openRoute":true,"allow":[{"agent":"codex","model":null},{"agent":"claude","model":"claude-sonnet-5-5"}]}"#,
        );
        let until = (Utc::now() + Duration::hours(2)).to_rfc3339();
        company(
            dir.path(),
            "cfo",
            &format!(r#"{{"reservePercent":40,"bufferPercent":10,"extraUsageUntil":"{until}"}}"#),
        );
        let first = load(dir.path()).unwrap();
        assert_eq!(first.reserve_percent, 50, "reserve plus buffer");
        assert_eq!(first.allow.len(), 2);
        assert_eq!(first.allow[1].model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(first.extra_usage_until.as_deref(), Some(until.as_str()));
        // A later company change is never read again.
        company(
            dir.path(),
            "cfo",
            r#"{"reservePercent":80,"bufferPercent":10}"#,
        );
        company(dir.path(), "routing", r#"{"openRoute":false,"allow":[]}"#);
        assert_eq!(load(dir.path()).unwrap(), first);
        assert_eq!(peek(dir.path()).unwrap(), first);
    }

    #[test]
    fn migration_caps_the_reserve_and_ignores_damaged_rows() {
        let dir = vault();
        company(
            dir.path(),
            "cfo",
            r#"{"reservePercent":85,"bufferPercent":10}"#,
        );
        company(dir.path(), "routing", "not json");
        let settings = load(dir.path()).unwrap();
        assert_eq!(settings.reserve_percent, MAX_RESERVE);
        assert!(settings.allow.is_empty());
    }

    #[test]
    fn a_patch_changes_only_its_own_fields() {
        let dir = vault();
        approve_extra_usage(dir.path(), 4).unwrap();
        save_patch(
            dir.path(),
            Patch {
                reserve_percent: Some(40),
                ..Patch::default()
            },
        )
        .unwrap();
        let models = save_patch(
            dir.path(),
            Patch {
                allow: Some(vec![AllowEntry {
                    agent: "codex".into(),
                    model: None,
                }]),
                effort_mode: Some(EffortMode::Fixed),
                ..Patch::default()
            },
        )
        .unwrap();
        assert_eq!(models.reserve_percent, 40, "Usage's field kept");
        assert!(models.extra_usage_open(Utc::now()), "the approval kept");
        assert_eq!(models.effort_mode, EffortMode::Fixed);
        assert!(save_patch(
            dir.path(),
            Patch {
                reserve_percent: Some(95),
                ..Patch::default()
            }
        )
        .is_err());
        assert_eq!(
            load(dir.path()).unwrap(),
            models,
            "a refused patch writes nothing"
        );
    }

    #[test]
    fn save_validates_and_extra_usage_expires() {
        let dir = vault();
        let mut bad = CfoSettings {
            reserve_percent: 95,
            ..CfoSettings::default()
        };
        assert!(save(dir.path(), bad.clone()).is_err());
        bad.reserve_percent = 30;
        bad.allow = vec![
            AllowEntry {
                agent: "Claude ".into(),
                model: Some(" ".into()),
            },
            AllowEntry {
                agent: "claude".into(),
                model: None,
            },
        ];
        assert!(save(dir.path(), bad.clone()).is_err(), "listed twice");
        bad.allow.truncate(1);
        let saved = save(dir.path(), bad).unwrap();
        assert_eq!(saved.allow[0].agent, "claude");
        assert_eq!(saved.pool_ceiling(), 70.0);
        assert!(approve_extra_usage(dir.path(), 100).is_err());
        let approved = approve_extra_usage(dir.path(), 4).unwrap();
        assert!(approved.extra_usage_open(Utc::now()));
        assert!(!approved.extra_usage_open(Utc::now() + Duration::hours(5)));
        assert_eq!(approved.reserve_percent, 30);
        let withdrawn = approve_extra_usage(dir.path(), 0).unwrap();
        assert!(!withdrawn.extra_usage_open(Utc::now()));
    }
}
